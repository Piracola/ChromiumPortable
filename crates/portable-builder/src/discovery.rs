//! Port of portable_builder/discovery.py - auto layout static analysis.
//! Owner: Wave3-A (contract: _migration/pe-golden/discovery_scoring_contract.md
//! and its addendum; goldens discovery_reference.json + discovery/*.json).
//!
//! Semantics follow the Python source exactly:
//! - the 14-item scoring table (scores are behavior, not tunables),
//! - reasons are Chinese strings compared byte-for-byte by the golden gates,
//! - sort key (-score, path part count, name.casefold()); the walker sorts
//!   entries deterministically (contract note: same-score same-name ties must
//!   be stable before the part-count key),
//! - select_browser_candidate: minimum_score=80 + chromium evidence required,
//!   ambiguity_margin=15 protection with exact Python error texts,
//! - extract_package_layers: max_depth=4, max_archives=16, brave_omaha first
//!   layer via the brave_bundle port, nested candidates sorted (priority,
//!   parts, name.casefold()) and probed with a 7z list.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};
use regex::Regex;

use crate::brave_bundle::extract_brave_metainstaller;
use crate::log_fmt::info;
use crate::pe::{read_pe_import_names, read_pe_machine, read_version_info};
use crate::tools::{extract_with_7z, remove_path};

/// discovery.py VERSION_DIR: ^\d+(?:\.\d+){1,5}$
fn version_dir_matches(name: &str) -> bool {
    static VERSION_DIR: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^\d+(?:\.\d+){1,5}$").expect("static regex"));
    VERSION_DIR.is_match(name)
}

/// discovery.py ARCHIVE_SUFFIXES (casefolded).
const ARCHIVE_SUFFIXES: [&str; 5] = [".7z", ".zip", ".rar", ".cab", ".msi"];

/// discovery.py KNOWN_BROWSER_EXES (casefolded members).
const KNOWN_BROWSER_EXES: [&str; 9] = [
    "360chromex.exe",
    "brave.exe",
    "chrome.exe",
    "chromium.exe",
    "helium.exe",
    "msedge.exe",
    "opera.exe",
    "thorium.exe",
    "vivaldi.exe",
];

/// discovery.py REJECT_NAME_TOKENS.
const REJECT_NAME_TOKENS: [&str; 20] = [
    "broker",
    "crash",
    "driver",
    "elevation",
    "helper",
    "install",
    "launcher",
    "notification",
    "ondemand",
    "proxy",
    "register",
    "report",
    "service",
    "setup",
    "shell",
    "unins",
    "update",
    "vpn",
    "wer",
    "wireguard",
];

/// discovery.py REJECT_PATH_TOKENS.
const REJECT_PATH_TOKENS: [&str; 3] = ["installer", "update", "win_clang_x86"];

/// discovery.py CHROMIUM_MARKERS.
const CHROMIUM_MARKERS: [&str; 6] = [
    "chrome.dll",
    "chrome_elf.dll",
    "icudtl.dat",
    "opera_elf.dll",
    "resources.pak",
    "vivaldi_elf.dll",
];

/// discovery.py::_is_pe - two leading MZ bytes.
fn is_pe(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic).is_ok() && &magic == b"MZ"
}

/// discovery.py::_safe_name - [^A-Za-z0-9._-]+ -> "_", truncated to 80 chars.
fn safe_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out.chars().take(80).collect()
}

/// discovery.py::_is_brave_metainstaller - PE + version-info fingerprint.
fn is_brave_metainstaller(path: &Path) -> bool {
    if !is_pe(path) {
        return false;
    }
    let Ok(info) = read_version_info(path) else {
        return false;
    };
    let product = info.get("ProductName").map(String::as_str).unwrap_or("");
    let original = info
        .get("OriginalFilename")
        .map(String::as_str)
        .unwrap_or("");
    let product = product.to_lowercase();
    let original = original.to_lowercase();
    product.contains("brave") && product.contains("update") && original.contains("updatesetup")
}

/// discovery.py::_seven_zip_can_open - `7z l` must exit 0 and print "Type =".
fn seven_zip_can_open(path: &Path, seven_zip: &str) -> bool {
    let Ok(output) = Command::new(seven_zip).arg("l").arg(path).output() else {
        return false;
    };
    output.status.success() && String::from_utf8_lossy(&output.stdout).contains("Type =")
}

/// discovery.py::_nested_archive_candidates. Priority tiers: 0 chrome.7z-style
/// well-known names, 1 other archives, 2 installer/setup/package PEs whose
/// description says installer/setup. Probed in sorted order; only archives
/// 7z can open are returned.
fn nested_archive_candidates(root: &Path, seven_zip: &str) -> Vec<PathBuf> {
    let mut likely: Vec<(u8, PathBuf)> = Vec::new();
    for path in walk_files_sorted(root) {
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.len() < 64 * 1024 {
            continue;
        }
        let relative = match path.strip_prefix(root) {
            Ok(rel) => rel,
            Err(_) => continue,
        };
        if relative.components().count() > 3 {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let lowered_name = name.to_lowercase();
        let suffix: String = Path::new(&lowered_name)
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default();
        if ARCHIVE_SUFFIXES.contains(&suffix.as_str()) {
            let priority = if matches!(
                lowered_name.as_str(),
                "chrome.7z" | "vivaldi.7z" | "browser.7z"
            ) {
                0
            } else {
                1
            };
            likely.push((priority, path));
            continue;
        }
        if is_pe(&path)
            && ["installer", "setup", "package"]
                .iter()
                .any(|token| lowered_name.contains(token))
        {
            let Ok(info) = read_version_info(&path) else {
                continue;
            };
            let description = info
                .values()
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if description.contains("installer") || description.contains("setup") {
                likely.push((2, path));
            }
        }
    }

    // sorted(likely, key=(priority, len(parts), name.casefold())).
    likely.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.components().count().cmp(&b.1.components().count()))
            .then_with(|| {
                a.1.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .cmp(&b.1.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            })
    });

    let mut opened = Vec::new();
    for (_, path) in likely {
        if seven_zip_can_open(&path, seven_zip) {
            opened.push(path);
        }
    }
    opened
}

/// discovery.py::_direct_names - casefolded direct-child names.
fn direct_names(directory: &Path) -> Vec<String> {
    let Ok(read_dir) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read_dir
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .map(|name| name.to_lowercase())
        .collect();
    names.sort();
    names
}

/// discovery.py::_numeric_children - direct child dirs matching VERSION_DIR.
fn numeric_children(directory: &Path) -> Vec<PathBuf> {
    let Ok(read_dir) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut children: Vec<PathBuf> = read_dir
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .map(|n| version_dir_matches(&n.to_string_lossy()))
                    .unwrap_or(false)
        })
        .collect();
    children.sort();
    children
}

/// One scored candidate (discovery.py _score_candidate return dict).
#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: PathBuf,
    pub score: i32,
    pub reasons: Vec<String>,
    pub architecture: String,
    pub version: String,
    pub product: String,
    pub imports: Vec<String>,
    pub has_chromium_evidence: bool,
}

/// discovery.py::_score_candidate. read_pe_machine failure => None
/// (the candidate disappears entirely, it does not score 0).
fn score_candidate(path: &Path, desired_arch: &str) -> Option<Candidate> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lowered_name = name.to_lowercase();
    let stem = Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lowered_stem = stem.to_lowercase();

    let info = read_version_info(path).ok()?;
    let metadata: String = info
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut reasons: Vec<String> = Vec::new();
    let mut score: i32 = 0;

    let architecture = read_pe_machine(path).ok()?;
    if architecture == desired_arch {
        score += 15;
        reasons.push(format!("架构为 {architecture}"));
    } else if !desired_arch.is_empty() && architecture != desired_arch {
        score -= 100;
        reasons.push(format!("架构为 {architecture}，目标为 {desired_arch}"));
    }

    if KNOWN_BROWSER_EXES.contains(&lowered_name.as_str()) {
        score += 120;
        reasons.push("文件名是常见浏览器主程序".to_string());
    }
    if REJECT_NAME_TOKENS
        .iter()
        .any(|token| lowered_stem.contains(token))
    {
        score -= 180;
        reasons.push("文件名像安装器、更新器或辅助程序".to_string());
    }
    // path.parent.parts[-3:]: the last (up to) 3 components of the parent.
    let parent = path.parent().unwrap_or(Path::new(""));
    let parent_parts: Vec<String> = parent
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let tail_start = parent_parts.len().saturating_sub(3);
    let path_tokens: Vec<String> = parent_parts[tail_start..]
        .iter()
        .map(|part| part.to_lowercase())
        .collect();
    if path_tokens
        .iter()
        .any(|token| REJECT_PATH_TOKENS.contains(&token.as_str()))
    {
        score -= 70;
        reasons.push("位于安装器或兼容组件目录".to_string());
    }

    let product_text = ["ProductName", "FileDescription", "OriginalFilename"]
        .iter()
        .map(|field| info.get(*field).map(String::as_str).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if ["installer", "setup", "update", "helper", "crash", "service"]
        .iter()
        .any(|token| product_text.contains(token))
    {
        score -= 140;
        reasons.push("文件说明表明它不是浏览器主程序".to_string());
    }
    if [
        "browser", "chromium", "chrome", "vivaldi", "opera", "brave", "thorium",
    ]
    .iter()
    .any(|token| metadata.contains(token))
    {
        score += 25;
        reasons.push("产品信息表明它属于 Chromium 浏览器".to_string());
    }
    let original = info
        .get("OriginalFilename")
        .map(String::as_str)
        .unwrap_or("");
    let original_name = Path::new(original)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !original_name.is_empty() && original_name == lowered_name {
        score += 15;
        reasons.push("原始文件名与当前文件一致".to_string());
    }

    let direct = direct_names(parent);
    let numeric = numeric_children(parent);
    let mut child_markers: Vec<String> = Vec::new();
    for child in numeric.iter().take(4) {
        for name in direct_names(child) {
            if CHROMIUM_MARKERS.contains(&name.as_str()) {
                child_markers.push(name);
            }
        }
    }
    let direct_markers: Vec<String> = direct
        .iter()
        .filter(|name| CHROMIUM_MARKERS.contains(&name.as_str()))
        .cloned()
        .collect();
    if !direct_markers.is_empty() {
        score += 35;
        reasons.push("同目录存在 Chromium 核心文件".to_string());
    }
    if !numeric.is_empty() && !child_markers.is_empty() {
        score += 45;
        reasons.push("同目录存在版本资源目录".to_string());
    }
    if parent
        .file_name()
        .map(|n| version_dir_matches(&n.to_string_lossy()))
        .unwrap_or(false)
        && !direct_markers.is_empty()
    {
        score += 20;
        reasons.push("位于完整的版本资源目录".to_string());
    }

    let imports = read_pe_import_names(path).unwrap_or_default();
    let lowered_imports: Vec<String> = imports.iter().map(|item| item.to_lowercase()).collect();
    let elf_imports = lowered_imports
        .iter()
        .filter(|item| {
            matches!(
                item.as_str(),
                "chrome_elf.dll" | "vivaldi_elf.dll" | "opera_elf.dll"
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    if !elf_imports.is_empty() {
        score += 55;
        reasons.push("直接加载浏览器 ELF 组件".to_string());
    }

    if imports.is_empty() {
        score -= 15;
        reasons.push("没有可用的 PE 导入表".to_string());
    }

    let version = info
        .get("ProductVersion")
        .filter(|value| !value.is_empty())
        .or_else(|| info.get("FileVersion"))
        .cloned()
        .unwrap_or_default();
    let product = match info
        .get("ProductName")
        .filter(|value| !value.is_empty())
        .or_else(|| info.get("FileDescription"))
    {
        Some(value) => value.clone(),
        None => stem,
    };

    let has_chromium_evidence =
        !direct_markers.is_empty() || !child_markers.is_empty() || !elf_imports.is_empty();

    Some(Candidate {
        path: path.to_path_buf(),
        score,
        reasons,
        architecture,
        version,
        product,
        imports,
        has_chromium_evidence,
    })
}

/// Deterministic recursive file walk (every dir level sorted; the Python
/// walker relies on rglob order only for tie stability, and the final sort
/// key covers the rest).
fn walk_files_sorted(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = read_dir.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                stack.push(entry.clone());
            } else if entry.is_file() {
                out.push(entry);
            }
        }
    }
    out
}

/// discovery.py::find_browser_candidates.
pub fn find_browser_candidates(root: &Path, desired_arch: &str) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for path in walk_files_sorted(root) {
        let is_exe = path
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase() == "exe")
            .unwrap_or(false);
        if !is_exe || !is_pe(&path) {
            continue;
        }
        if let Some(candidate) = score_candidate(&path, desired_arch) {
            candidates.push(candidate);
        }
    }
    sort_candidates(&mut candidates);
    candidates
}

/// The shared sort key: (-score, len(path.parts), name.casefold()).
pub(crate) fn sort_candidates(candidates: &mut [Candidate]) {
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| {
                a.path
                    .components()
                    .count()
                    .cmp(&b.path.components().count())
            })
            .then_with(|| {
                a.path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .cmp(
                        &b.path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase()),
                    )
            })
    });
}

/// discovery.py::select_browser_candidate - 80-score + evidence gate, then
/// the 15-point ambiguity protection. Error texts are Python's, verbatim.
pub fn select_browser_candidate(
    candidates: &[Candidate],
    minimum_score: i32,
    ambiguity_margin: i32,
) -> Result<Candidate> {
    let Some(first) = candidates.first() else {
        bail!("没有找到可信的浏览器主程序：没有 EXE 候选");
    };
    if first.score < minimum_score || !first.has_chromium_evidence {
        let summary = candidates
            .iter()
            .take(8)
            .map(|item| format!("{} ({})", file_name_of(&item.path), item.score))
            .collect::<Vec<_>>()
            .join(", ");
        bail!("没有找到可信的浏览器主程序：{summary}");
    }
    let best = first;
    if let Some(second) = candidates.get(1) {
        if second.score >= best.score - ambiguity_margin {
            let tied = candidates
                .iter()
                .take(5)
                .filter(|item| item.score >= best.score - ambiguity_margin)
                .map(|item| format!("{} ({})", item.path.display(), item.score))
                .collect::<Vec<_>>()
                .join(", ");
            bail!("存在多个接近的主程序候选，已停止自动选择：{tied}");
        }
    }
    Ok(best.clone())
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One extraction layer (discovery.py extract_package_layers dict).
#[derive(Debug, Clone)]
pub struct Layer {
    pub source: PathBuf,
    pub root: PathBuf,
    pub depth: usize,
    pub kind: String,
}

/// discovery.py::extract_package_layers. Directory input is used as-is
/// (kind=directory); anything else is removed first, then extracted layer by
/// layer (brave_omaha or archive first layer, nested archives queued).
pub fn extract_package_layers(
    package_path: &Path,
    extraction_root: &Path,
    seven_zip: Option<&str>,
    // Python threads desired_arch through extract_package_layers but never
    // reads it there; keep the parameter for signature parity.
    _desired_arch: &str,
    max_depth: usize,
    max_archives: usize,
) -> Result<Vec<Layer>> {
    if package_path.is_dir() {
        return Ok(vec![Layer {
            source: package_path.to_path_buf(),
            root: package_path.to_path_buf(),
            depth: 0,
            kind: "directory".to_string(),
        }]);
    }
    let Some(seven_zip) = seven_zip else {
        bail!("A 7-Zip tool is required to extract packages.");
    };

    remove_path(extraction_root);
    std::fs::create_dir_all(extraction_root)?;

    let first_root = extraction_root.join("layer-00-package");
    let first_kind = if is_brave_metainstaller(package_path) {
        info(format!(
            "Statically extracting Brave/Omaha bundle: {}",
            file_name_of(package_path)
        ));
        extract_brave_metainstaller(package_path, &first_root)?;
        "brave_omaha"
    } else {
        extract_with_7z(package_path, &first_root, seven_zip)?;
        "archive"
    };

    let mut layers = vec![Layer {
        source: package_path.to_path_buf(),
        root: first_root.clone(),
        depth: 0,
        kind: first_kind.to_string(),
    }];
    let mut queue = vec![layers[0].clone()];
    let mut processed: BTreeMap<String, ()> = BTreeMap::new();
    let mut sequence = 1usize;
    while let Some(layer) = queue.first().cloned() {
        queue.remove(0);
        if layer.depth >= max_depth {
            continue;
        }
        for archive in nested_archive_candidates(&layer.root, seven_zip) {
            let key = match archive.canonicalize() {
                Ok(resolved) => resolved.to_string_lossy().to_lowercase(),
                Err(_) => archive.to_string_lossy().to_lowercase(),
            };
            if processed.contains_key(&key) {
                continue;
            }
            processed.insert(key, ());
            let child_root =
                extraction_root.join(format!("layer-{:02}-{}", sequence, safe_name(&archive)));
            sequence += 1;
            info(format!(
                "Recursively extracting nested package: {}",
                file_name_of(&archive)
            ));
            extract_with_7z(&archive, &child_root, seven_zip)?;
            let child = Layer {
                source: archive,
                root: child_root,
                depth: layer.depth + 1,
                kind: "nested_archive".to_string(),
            };
            layers.push(child.clone());
            queue.push(child);
            if sequence > max_archives {
                break;
            }
        }
        if sequence > max_archives {
            break;
        }
    }
    Ok(layers)
}

/// discovery.py::_infer_version_dir.
fn infer_version_dir(app_root: &Path, executable: &Path, version: &str) -> PathBuf {
    let parent = executable.parent().unwrap_or(app_root);
    if parent
        .file_name()
        .map(|n| version_dir_matches(&n.to_string_lossy()))
        .unwrap_or(false)
    {
        return parent.to_path_buf();
    }
    if !version.is_empty() {
        let candidate = app_root.join(version);
        if candidate.is_dir() {
            return candidate;
        }
    }
    let numeric = numeric_children(app_root);
    if !numeric.is_empty() {
        let mut sorted = numeric;
        sorted.sort_by(|a, b| {
            let a_name = file_name_of(a);
            let b_name = file_name_of(b);
            b_name.cmp(&a_name)
        });
        return sorted[0].clone();
    }
    app_root.to_path_buf()
}

/// discovery.py::analyze_package / analyze_extracted_app return shape.
#[derive(Clone)]
pub struct AnalyzeResult {
    pub package: PathBuf,
    pub layers: Vec<Layer>,
    pub candidates: Vec<Candidate>,
    pub selected: Candidate,
    pub app_root: PathBuf,
    pub executable: PathBuf,
    pub executable_relative: PathBuf,
    pub version_dir: PathBuf,
    pub version: String,
    pub product: String,
    pub architecture: String,
    /// None for analyze_extracted_app (no layer concept there).
    pub layer_root: Option<PathBuf>,
}

/// discovery.py::analyze_package.
pub fn analyze_package(
    package_path: &Path,
    extraction_root: &Path,
    seven_zip: Option<&str>,
    desired_arch: &str,
) -> Result<AnalyzeResult> {
    let layers = extract_package_layers(
        package_path,
        extraction_root,
        seven_zip,
        desired_arch,
        4,
        16,
    )?;
    let mut candidates: Vec<Candidate> = Vec::new();
    for layer in &layers {
        for candidate in find_browser_candidates(&layer.root, desired_arch) {
            candidates.push(candidate);
        }
    }
    sort_candidates(&mut candidates);
    let selected = select_browser_candidate(&candidates, 80, 15)?;
    let app_root = selected
        .path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let version = if selected.version.is_empty() {
        "0.0.0.0".to_string()
    } else {
        selected.version.clone()
    };
    let version_dir = infer_version_dir(&app_root, &selected.path, &version);
    let executable_relative = selected
        .path
        .strip_prefix(&app_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| selected.path.clone());
    let layer_root = layers
        .iter()
        .find(|layer| selected.path.starts_with(&layer.root))
        .map(|layer| layer.root.clone());
    Ok(AnalyzeResult {
        package: package_path.to_path_buf(),
        layers,
        candidates,
        selected: selected.clone(),
        app_root,
        executable: selected.path.clone(),
        executable_relative,
        version_dir,
        version,
        product: selected.product.clone(),
        architecture: selected.architecture.clone(),
        layer_root,
    })
}

/// discovery.py::analyze_extracted_app - same scorer, no layer concept.
pub fn analyze_extracted_app(app_root: &Path, desired_arch: &str) -> Result<AnalyzeResult> {
    let mut candidates = find_browser_candidates(app_root, desired_arch);
    sort_candidates(&mut candidates);
    let selected = select_browser_candidate(&candidates, 80, 15)?;
    let version = if selected.version.is_empty() {
        "0.0.0.0".to_string()
    } else {
        selected.version.clone()
    };
    let version_dir = infer_version_dir(app_root, &selected.path, &version);
    let executable_relative = selected
        .path
        .strip_prefix(app_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| selected.path.clone());
    Ok(AnalyzeResult {
        package: PathBuf::new(),
        layers: Vec::new(),
        candidates,
        selected: selected.clone(),
        app_root: app_root.to_path_buf(),
        executable: selected.path.clone(),
        executable_relative,
        version_dir,
        version,
        product: selected.product.clone(),
        architecture: selected.architecture.clone(),
        layer_root: None,
    })
}

/// discovery.py::public_report - the --json contract shape. serde_json with
/// preserve_order keeps the Python dict literal key order.
pub fn public_report(result: &AnalyzeResult) -> serde_json::Value {
    let selected_display = match &result.layer_root {
        Some(layer_root) => result
            .executable
            .strip_prefix(layer_root)
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default(),
        None => result
            .executable
            .strip_prefix(&result.app_root)
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default(),
    };
    let package = if result.package.as_os_str().is_empty() {
        None
    } else {
        Some(result.package.to_string_lossy().into_owned())
    };
    serde_json::json!({
        "package": package,
        "product": result.product,
        "version": result.version,
        "architecture": result.architecture,
        "executable": selected_display,
        "score": result.selected.score,
        "reasons": result.selected.reasons,
        "layers": result.layers
            .iter()
            .map(|layer| {
                serde_json::json!({
                    "depth": layer.depth,
                    "kind": layer.kind,
                    "source": file_name_of(&layer.source),
                })
            })
            .collect::<Vec<_>>(),
        "alternatives": result
            .candidates
            .iter()
            .skip(1)
            .take(5)
            .map(|item| {
                serde_json::json!({
                    "path": item.path.to_string_lossy(),
                    "score": item.score,
                    "product": item.product,
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// discovery.py::print_report - human mode prints the five [OK] lines; JSON
/// mode prints public_report like json.dumps(ensure_ascii=False, indent=2).
pub fn print_report(result: &AnalyzeResult, as_json: bool) {
    let report = public_report(result);
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
        return;
    }
    let reasons = report["reasons"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join("；")
        })
        .unwrap_or_default();
    println!("[OK] 浏览器：{}", report["product"].as_str().unwrap_or(""));
    println!(
        "[OK] 版本：{} ({})",
        report["version"].as_str().unwrap_or(""),
        report["architecture"].as_str().unwrap_or("")
    );
    println!(
        "[OK] 主程序：{}",
        report["executable"].as_str().unwrap_or("")
    );
    println!("[OK] 可信分：{}", report["score"]);
    println!("[OK] 判断依据：{reasons}");
}
#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> Option<PathBuf> {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let samples_ok = [
            "bin/360csex_23.1.1253.64.exe",
            "bin/Opera_135.0.5973.92_Setup_x64.exe",
            "bin/thorium_AVX2_mini_installer.exe",
            "bin/Vivaldi.8.2.4133.52.x64.exe",
            "bin/BraveBrowserStandaloneSetup.exe",
            "_migration/pe-golden/discovery",
            "7zr.exe",
        ]
        .iter()
        .all(|rel| repo.join(rel).exists());
        samples_ok.then_some(repo)
    }

    fn seven_zip_tool(repo: &Path) -> String {
        // Repo-root 7zr.exe, no download (allow_download=false semantics).
        let local = repo.join("7zr.exe");
        assert!(local.is_file(), "7zr.exe missing from repo root");
        local.to_string_lossy().into_owned()
    }

    fn temp_extract_dir(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("pb-discovery-{tag}-{}-{n}", std::process::id()));
        remove_path(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn load_golden(repo: &Path, name: &str) -> serde_json::Value {
        serde_json::from_str(
            &std::fs::read_to_string(repo.join("_migration/pe-golden/discovery").join(name))
                .unwrap(),
        )
        .unwrap()
    }

    /// Rewrite the machine-specific extraction root to the golden workdir
    /// prefix ("_migration\pe-golden\tmp_inspect\<tag>") so package +
    /// alternatives paths compare like-for-like.
    fn rewrite_paths(
        value: &serde_json::Value,
        actual_root: &str,
        golden_prefix: &str,
    ) -> serde_json::Value {
        match value {
            serde_json::Value::String(text) => {
                serde_json::Value::String(text.replace(actual_root, golden_prefix))
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(
                items
                    .iter()
                    .map(|item| rewrite_paths(item, actual_root, golden_prefix))
                    .collect(),
            ),
            serde_json::Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (key, item) in map {
                    out.insert(key.clone(), rewrite_paths(item, actual_root, golden_prefix));
                }
                serde_json::Value::Object(out)
            }
            other => other.clone(),
        }
    }

    /// The five golden samples: inspect-package --json deep-equal, Chinese
    /// reasons byte-for-byte (Gate 1).
    #[test]
    fn golden_discovery_samples_deep_equal() {
        let Some(repo) = repo_root() else { return };
        for (sample, tag, golden_name) in [
            (
                "bin/thorium_AVX2_mini_installer.exe",
                "thorium",
                "thorium.json",
            ),
            ("bin/Vivaldi.8.2.4133.52.x64.exe", "vivaldi", "vivaldi.json"),
            (
                "bin/Opera_135.0.5973.92_Setup_x64.exe",
                "opera",
                "opera.json",
            ),
            ("bin/360csex_23.1.1253.64.exe", "csex", "csex.json"),
            ("bin/BraveBrowserStandaloneSetup.exe", "brave", "brave.json"),
        ] {
            let golden = load_golden(&repo, golden_name);
            let dir = temp_extract_dir(tag);
            let seven_zip = seven_zip_tool(&repo);
            let result = analyze_package(
                &repo.join(sample),
                &dir.join("extracted"),
                Some(&seven_zip),
                "x64",
            )
            .unwrap_or_else(|err| panic!("{tag}: {err}"));
            let mut report = public_report(&result);
            // Python golden ran with a relative package argument
            // ("bin\<sample>.exe"); mirror that in the report.
            if let Some(obj) = report.as_object_mut() {
                obj.insert(
                    "package".to_string(),
                    serde_json::Value::String(sample.replace('/', "\\")),
                );
            }
            let report = report;
            let actual_root = dir.join("extracted").to_string_lossy().into_owned();
            let golden_prefix = format!("_migration\\pe-golden\\tmp_inspect\\{tag}");
            let rewritten = rewrite_paths(&report, &actual_root, &golden_prefix);
            assert_eq!(
                rewritten, golden,
                "{tag}: discovery report must deep-equal the golden"
            );
            remove_path(&dir);
        }
    }

    /// Gate 2: print_report human mode is byte-identical to
    /// print_report_golden.json for opera + vivaldi.
    #[test]
    fn golden_print_report_bytes() {
        let Some(repo) = repo_root() else { return };
        let golden: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join("_migration/pe-golden/print_report_golden.json"))
                .unwrap(),
        )
        .unwrap();
        for (tag, sample) in [
            ("opera", "bin/Opera_135.0.5973.92_Setup_x64.exe"),
            ("vivaldi", "bin/Vivaldi.8.2.4133.52.x64.exe"),
        ] {
            // Unique counter-based dirs mean no cross-test sharing even in
            // parallel runs (7zr output files collide on shared paths).
            let dir = temp_extract_dir(&format!("{tag}-pr"));
            let seven_zip = seven_zip_tool(&repo);
            let result = analyze_package(
                &repo.join(sample),
                &dir.join("extracted"),
                Some(&seven_zip),
                "x64",
            )
            .unwrap();
            let report = public_report(&result);
            let reasons = report["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>()
                .join("；");
            let human = format!(
                "[OK] 浏览器：{}\n[OK] 版本：{} ({})\n[OK] 主程序：{}\n[OK] 可信分：{}\n[OK] 判断依据：{}\n",
                report["product"].as_str().unwrap(),
                report["version"].as_str().unwrap(),
                report["architecture"].as_str().unwrap(),
                report["executable"].as_str().unwrap(),
                report["score"],
                reasons
            );
            assert_eq!(
                human,
                golden[tag].as_str().unwrap(),
                "{tag}: print_report bytes must match the golden"
            );
            remove_path(&dir);
        }
    }

    #[test]
    fn version_dir_regex_shapes() {
        assert!(version_dir_matches("153.1.95.102"));
        assert!(version_dir_matches("8.2"));
        assert!(!version_dir_matches("Chrome-bin"));
        assert!(!version_dir_matches("153"));
        assert!(!version_dir_matches("1.2.3.4.5.6.7"));
    }

    #[test]
    fn below_threshold_error_text_matches_python() {
        // No candidates at all: exact error text.
        let err = select_browser_candidate(&[], 80, 15)
            .unwrap_err()
            .to_string();
        assert_eq!(err, "没有找到可信的浏览器主程序：没有 EXE 候选");

        // A low-scoring candidate lists name + score (first 8 summary).
        let dir = std::env::temp_dir().join(format!("pb-discovery-low-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let candidate = Candidate {
            path: dir.join("random_tool.exe"),
            score: -50,
            reasons: Vec::new(),
            architecture: "x64".to_string(),
            version: String::new(),
            product: "Random".to_string(),
            imports: Vec::new(),
            has_chromium_evidence: false,
        };
        let err = select_browser_candidate(&[candidate], 80, 15)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("没有找到可信的浏览器主程序：random_tool.exe (-50)"),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ambiguity_protection_error_text_matches_python() {
        let dir = std::env::temp_dir().join(format!("pb-discovery-amb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, score: i32| Candidate {
            path: dir.join(name),
            score,
            reasons: Vec::new(),
            architecture: "x64".to_string(),
            version: String::new(),
            product: "Probe".to_string(),
            imports: Vec::new(),
            has_chromium_evidence: true,
        };
        let candidates = vec![mk("a.exe", 200), mk("b.exe", 190)];
        let err = select_browser_candidate(&candidates, 80, 15)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("存在多个接近的主程序候选，已停止自动选择："),
            "{err}"
        );
        assert!(err.contains("a.exe (200)"), "{err}");
        assert!(err.contains("b.exe (190)"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
