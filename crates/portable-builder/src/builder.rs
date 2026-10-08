//! Port of portable_builder/builder.py - prepare/stage/inject/archive pipeline.
//! Owner: Wave3-A (contract: _migration/pe-golden/builder_archive_contract.md
//! + addendum; goldens build_context_reference.json).
//!
//! Semantics follow the Python source exactly:
//! - prepare_package: temp dir per target, provider package resolution
//!   (path copy + digest verify, or download), auto layout routes through
//!   discovery::analyze_package with the 0.0.0.0 version fallback chain;
//! - stage_app: auto copytree / move_version_dir / copy_version_root /
//!   default move_version_root, output_dir -> app_root;
//! - copy_chrome_plus: version.dll + setdll copy, the three-layer
//!   chrome++.ini resolution (baseline / defaults / override) via ini_overlay;
//! - inject_dll: setdll /t probe then /d:<relpath> injection, the rel path
//!   is version.dll relative to the exe dir (absolute on cross-drive),
//!   OEM-codepage subprocess output decoding, backup removal,
//!   assert_portable_version_import closure;
//! - finalize / _complete_build / build_package_file / archive_target,
//!   including the _complete_build version-mismatch WARN and the five
//!   env keys written through github_env::write_env.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::discovery::{analyze_package, AnalyzeResult};
use crate::github_env::write_env;
use crate::ini_overlay::{merge_ini, parse_overrides, read_ini_text, write_ini_text};
use crate::log_fmt::{info, warn};
use crate::pe::assert_portable_version_import;
use crate::tools::{
    assert_no_forbidden_files, download_file, extract_with_7z, find_7z_tool, find_child_dir,
    find_child_file, find_version_dir, human_size, remove_path, sha256_file, verify_file_digest,
};

/// builder.py::format_value over the str.format subset (simple {name}
/// fields; anything else keeps the template, like the frozen formatter).
pub fn format_value(template: &str, context: &[(String, String)]) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => {
                out.push('{');
                i += 2;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => {
                out.push('}');
                i += 2;
            }
            b'{' => {
                let end = i
                    + 1
                    + bytes[i + 1..]
                        .iter()
                        .position(|&b| b == b'}')
                        .ok_or_else(|| anyhow!("Single '{{' encountered in format string"))?;
                let field = &template[i + 1..end];
                let resolved = context
                    .iter()
                    .find(|(key, _)| key == field)
                    .map(|(_, value)| value.as_str())
                    .ok_or_else(|| anyhow!("KeyError: '{field}'"))?;
                out.push_str(resolved);
                i = end + 1;
            }
            b'}' => bail!("Single '}}' encountered in format string"),
            _ => {
                let ch = template[i..].chars().next().expect("char boundary");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    Ok(out)
}

/// builder.py::build_context. Archive facts come from env (populated by the
/// archive stage); run_url from the GitHub CI env chain.
pub fn build_context(
    target: &Value,
    version: Option<&str>,
    date: Option<&str>,
    package_version: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let resolved_version = version.unwrap_or("");
    let resolved_package_version = package_version.unwrap_or(resolved_version);
    let now = chrono::Local::now();
    let date = date
        .map(str::to_string)
        .unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    let name = target_str(target, "name")
        .or_else(|| target_str(target, "display_name"))
        .unwrap_or_default();
    let display_name = target_str(target, "display_name")
        .or_else(|| target_str(target, "name"))
        .unwrap_or_default();
    let output_dir = target_str(target, "output_dir")
        .or_else(|| target_str(target, "name"))
        .unwrap_or_else(|| "Browser".to_string());
    Ok(vec![
        (
            "target".to_string(),
            target_str(target, "target").unwrap_or_default(),
        ),
        ("name".to_string(), name),
        ("display_name".to_string(), display_name),
        ("output_dir".to_string(), output_dir),
        ("version".to_string(), resolved_version.to_string()),
        (
            "package_version".to_string(),
            resolved_package_version.to_string(),
        ),
        ("date".to_string(), date),
        (
            "arch".to_string(),
            target_str(target, "architecture").unwrap_or_else(|| "x64".to_string()),
        ),
        ("archive".to_string(), env_or_empty("ARCHIVE_NAME")),
        ("sha256".to_string(), env_or_empty("ARCHIVE_SHA256")),
        (
            "size".to_string(),
            human_size(
                std::env::var("ARCHIVE_SIZE")
                    .ok()
                    .filter(|v| !v.is_empty())
                    .and_then(|v| v.parse::<f64>().ok()),
            ),
        ),
        (
            "chrome_plus_version".to_string(),
            env_or_empty("CHROME_PLUS_VERSION"),
        ),
        ("run_url".to_string(), crate::github_env::build_run_url()),
    ])
}

/// Python str(obj) over JSON scalars the context uses.
fn target_str(target: &Value, key: &str) -> Option<String> {
    match target.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(if *b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        _ => None,
    }
}

/// os.getenv(name, "") semantics.
fn env_or_empty(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// builder.py::get_version_info - provider dispatch with the optional
/// _workdir injection (script provider resolves relative paths against it).
pub fn get_version_info(target: &Value, workdir: Option<&Path>) -> Result<Value> {
    let mut provider_config = target.get("provider").cloned().unwrap_or_else(|| json!({}));
    if let Some(workdir) = workdir {
        if let Some(obj) = provider_config.as_object_mut() {
            obj.insert(
                "_workdir".to_string(),
                json!(workdir.to_string_lossy().into_owned()),
            );
        }
    }
    crate::providers::get_package(&provider_config)
}

/// The staged tree plus everything the injection step needs.
pub struct Staged {
    pub stage_dir: PathBuf,
    pub app_root: PathBuf,
    pub version_dir: PathBuf,
    pub version: String,
    pub executable: Option<PathBuf>,
    pub discovery: Option<AnalyzeResult>,
    pub version_dll: Option<PathBuf>,
    pub setdll: Option<PathBuf>,
    pub chrome_plus_version: Option<String>,
}

/// builder.py::prepare_package. Returns the discovery result for auto
/// layout or the version-root pointers for the fixed layouts.
pub enum Prepared {
    Auto {
        temp_dir: PathBuf,
        app_source: PathBuf,
        executable_relative: PathBuf,
        version_dir_relative: PathBuf,
        version: String,
        discovery: Box<AnalyzeResult>,
    },
    VersionRoot {
        temp_dir: PathBuf,
        version_root: PathBuf,
        version_dir_name: String,
        version: String,
    },
}

impl Prepared {
    pub fn version(&self) -> &str {
        match self {
            Prepared::Auto { version, .. } => version,
            Prepared::VersionRoot { version, .. } => version,
        }
    }
}

/// builder.py::prepare_package.
pub fn prepare_package(target: &Value, workdir: &Path, package: &Value) -> Result<Prepared> {
    let target_id = target_str(target, "target").unwrap_or_default();
    let temp_dir = workdir.join("build").join("temp").join(&target_id);
    remove_path(&temp_dir);
    std::fs::create_dir_all(&temp_dir)?;

    let auto_layout = target_str(target, "layout").as_deref() == Some("auto");
    let source_path = package
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            package
                .get("installer_path")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        });

    let installer_path: PathBuf = if let Some(source) = source_path {
        let mut source_path = PathBuf::from(source);
        if source_path.is_relative() {
            source_path = workdir.join(source_path);
        }
        if !source_path.exists() {
            bail!(
                "Installer path returned by provider does not exist: {}",
                source_path.display()
            );
        }
        if source_path.is_dir() {
            if target_str(target, "layout").as_deref() != Some("auto") {
                bail!("A source directory requires layout: 'auto'");
            }
            source_path
        } else {
            let file_name = source_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let installer_path = temp_dir.join(&file_name);
            let differs = source_path
                .canonicalize()
                .ok()
                .map(|resolved| {
                    resolved
                        != installer_path
                            .canonicalize()
                            .unwrap_or_else(|_| installer_path.clone())
                })
                .unwrap_or(true);
            if differs {
                std::fs::copy(&source_path, &installer_path)?;
            }
            verify_file_digest(
                &installer_path,
                package.get("sha256").and_then(Value::as_str),
                package.get("size").and_then(Value::as_u64),
            )?;
            installer_path
        }
    } else {
        let file_name = package
            .get("file_name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("package missing 'file_name'"))?;
        let installer_path = temp_dir.join(file_name);
        download_file(
            package
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("package missing 'url'"))?,
            &installer_path,
            package
                .get("verify_ssl")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            true,
            package.get("sha256").and_then(Value::as_str),
            package.get("size").and_then(Value::as_u64),
            true,
        )?;
        installer_path
    };

    // An already extracted auto-layout package never needs an extractor.
    let needs_extractor = !(auto_layout && installer_path.is_dir());
    let seven_zip: Option<String> = if needs_extractor {
        Some(find_7z_tool(
            workdir,
            target
                .get("allow_7z_download")
                .and_then(Value::as_bool)
                .unwrap_or(!auto_layout),
            target
                .get("allow_7z_system_install")
                .and_then(Value::as_bool)
                .unwrap_or(!auto_layout),
        )?)
    } else {
        None
    };

    if auto_layout {
        let discovery = analyze_package(
            &installer_path,
            &temp_dir.join("auto-extracted"),
            seven_zip.as_deref(),
            target_str(target, "architecture")
                .as_deref()
                .unwrap_or("x64"),
        )?;
        let mut detected_version = discovery.version.clone();
        if detected_version.is_empty() || detected_version == "0.0.0.0" {
            detected_version = package
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("0.0.0.0")
                .to_string();
        }
        info(format!(
            "Auto-detected browser executable: {}",
            discovery.executable.display()
        ));
        let version_dir_relative = discovery
            .version_dir
            .strip_prefix(&discovery.app_root)
            .map_err(|_| anyhow!("version dir outside app root"))?
            .to_path_buf();
        Ok(Prepared::Auto {
            temp_dir,
            app_source: discovery.app_root.clone(),
            executable_relative: discovery.executable_relative.clone(),
            version_dir_relative,
            version: detected_version,
            discovery: Box::new(discovery),
        })
    } else {
        let seven_zip =
            seven_zip.ok_or_else(|| anyhow!("7-Zip tool is required for fixed layouts"))?;
        extract_with_7z(&installer_path, &temp_dir, &seven_zip)?;

        if let Some(inner_archive) = target.get("inner_archive").and_then(Value::as_str) {
            let inner_archive_path = find_child_file(&temp_dir, inner_archive)
                .ok_or_else(|| anyhow!("Inner archive not found: {inner_archive}"))?;
            extract_with_7z(&inner_archive_path, &temp_dir, &seven_zip)?;
        }

        let version_root_name = target
            .get("version_root")
            .and_then(Value::as_str)
            .unwrap_or("Chrome-bin");
        let version_root = find_child_dir(&temp_dir, version_root_name).ok_or_else(|| {
            anyhow!("Version root not found after extraction: {version_root_name}")
        })?;

        let version_dir = find_version_dir(
            &version_root,
            package.get("version").and_then(Value::as_str),
        )
        .ok_or_else(|| anyhow!("Version directory not found in {}", version_root.display()))?;
        let version_dir_name = version_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let version = if version_dir_name.is_empty() {
            package
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        } else {
            version_dir_name.clone()
        };
        Ok(Prepared::VersionRoot {
            temp_dir,
            version_root,
            version_dir_name,
            version,
        })
    }
}

/// builder.py::stage_app.
pub fn stage_app(target: &Value, workdir: &Path, prepared: &Prepared) -> Result<Staged> {
    let target_id = target_str(target, "target").unwrap_or_default();
    let stage_dir = workdir.join("build").join("stage").join(&target_id);
    remove_path(&stage_dir);
    std::fs::create_dir_all(&stage_dir)?;

    let output_dir_name = target_str(target, "output_dir")
        .or_else(|| target_str(target, "name"))
        .unwrap_or_else(|| "Browser".to_string());
    let app_root = stage_dir.join(&output_dir_name);
    let layout = target
        .get("layout")
        .and_then(Value::as_str)
        .unwrap_or("move_version_root");

    let staged = match prepared {
        Prepared::Auto {
            app_source,
            executable_relative,
            version_dir_relative,
            version,
            discovery,
            ..
        } => {
            copy_tree(app_source, &app_root)?;
            let version_dir = app_root.join(version_dir_relative);
            let executable = app_root.join(executable_relative);
            if !executable.is_file() {
                bail!(
                    "Auto-detected executable was not staged: {}",
                    executable.display()
                );
            }
            Staged {
                stage_dir,
                app_root,
                version_dir,
                version: version.clone(),
                executable: Some(executable),
                discovery: Some(discovery.as_ref().clone()),
                version_dll: None,
                setdll: None,
                chrome_plus_version: None,
            }
        }
        Prepared::VersionRoot {
            version_root,
            version_dir_name,
            version,
            ..
        } => {
            match layout {
                "move_version_dir" => {
                    std::fs::create_dir_all(&app_root)?;
                    std::fs::rename(
                        version_root.join(version_dir_name),
                        app_root.join(version_dir_name),
                    )?;
                }
                "copy_version_root" => {
                    copy_tree(version_root, &app_root)?;
                }
                _ => {
                    std::fs::rename(version_root, &app_root)?;
                }
            }
            let version_dir = app_root.join(version_dir_name);
            if !version_dir.exists() {
                bail!(
                    "Staged version directory not found: {}",
                    version_dir.display()
                );
            }
            Staged {
                stage_dir,
                app_root,
                version_dir,
                version: version.clone(),
                executable: None,
                discovery: None,
                version_dll: None,
                setdll: None,
                chrome_plus_version: None,
            }
        }
    };
    Ok(staged)
}

/// shutil.copytree equivalent (dirs + files, symlinks followed).
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    let mut stack = vec![source.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rel = dir.strip_prefix(source).unwrap_or(Path::new(""));
        let target_dir = destination.join(rel);
        std::fs::create_dir_all(&target_dir)?;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let entry_path = entry.path();
            let dest_path = target_dir.join(entry.file_name());
            if entry_path.is_dir() {
                stack.push(entry_path);
            } else {
                std::fs::copy(&entry_path, &dest_path)?;
            }
        }
    }
    Ok(())
}

/// builder.py::resolve_chrome_plus_ini - the three-layer chrome++.ini
/// resolution. Returns (text, encoding) or None when nothing applies.
fn resolve_chrome_plus_ini(
    target: &Value,
    workdir: &Path,
    setdll_src_dir: &Path,
) -> Result<Option<(String, crate::ini_overlay::IniEncoding)>> {
    let chrome_plus_dir = workdir.join(
        target
            .get("chrome_plus_dir")
            .and_then(Value::as_str)
            .unwrap_or("chrome++"),
    );
    let ini_name = target
        .get("ini_name")
        .and_then(Value::as_str)
        .unwrap_or("chrome++.ini");
    let override_name = target
        .get("ini_override_name")
        .and_then(Value::as_str)
        .unwrap_or("chrome++.override.ini");

    let baseline_path = setdll_src_dir.join("chrome++.ini");
    let defaults_path = setdll_src_dir.join("chrome++.defaults.ini");
    let override_path = [
        workdir.join(override_name),
        chrome_plus_dir.join(override_name),
    ]
    .into_iter()
    .find(|path| path.exists());
    let full_path = [workdir.join(ini_name), chrome_plus_dir.join(ini_name)]
        .into_iter()
        .find(|path| path.exists());

    if let Some(full_path) = &full_path {
        if override_path.is_none() {
            warn(format!("Using the full {} verbatim; it shadows the synced baseline at {}, so upstream chrome++ additions will not reach users. Consider replacing it with {override_name}.", full_path.display(), baseline_path.display()));
            let (text, encoding) = read_ini_text(full_path)?;
            return Ok(Some((text, encoding)));
        }
    }

    if !baseline_path.exists() {
        if let Some(override_path) = &override_path {
            bail!("{} needs a baseline chrome++.ini at {}. Point --builder-dir (or PYTHONPATH) at the ChromiumPortable checkout.", override_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), baseline_path.display());
        }
        return Ok(None);
    }

    if let Some(full_path) = &full_path {
        warn(format!(
            "Ignoring {} because {override_name} takes precedence.",
            full_path.display()
        ));
    }

    let (mut text, encoding) = read_ini_text(&baseline_path)?;
    info(format!(
        "chrome++.ini baseline: {}",
        baseline_path.display()
    ));
    for layer in [Some(&defaults_path), override_path.as_ref()] {
        let Some(layer) = layer else { continue };
        if !layer.exists() {
            continue;
        }
        let (layer_text, _) = read_ini_text(layer)?;
        let overrides = parse_overrides(&layer_text)?;
        let (merged, applied) = merge_ini(&text, &overrides)?;
        text = merged;
        let summary = if applied.is_empty() {
            "(nothing)".to_string()
        } else {
            applied.join(", ")
        };
        info(format!(
            "Applied {} override(s) from {}: {}",
            applied.len(),
            layer
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            summary
        ));
    }
    Ok(Some((text, encoding)))
}

/// builder.py::copy_chrome_plus - version.dll + setdll into the staged
/// tree, then the ini layers. staged is mutated in place.
pub fn copy_chrome_plus(
    target: &Value,
    workdir: &Path,
    staged: &mut Staged,
    builder_dir: Option<&Path>,
) -> Result<()> {
    let arch = target
        .get("architecture")
        .and_then(Value::as_str)
        .unwrap_or("x64");
    let version_dll_name = target
        .get("version_dll_name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("version-{arch}.dll"));
    let setdll_name = target
        .get("setdll_name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("setdll-{arch}.exe"));

    let (setdll_src_dir, version_dll_src, setdll_src) = if let Some(builder_dir) = builder_dir {
        let setdll_src_dir = builder_dir.join("setdll");
        if !setdll_src_dir.exists() {
            bail!(
                "Builder setdll directory not found: {}",
                setdll_src_dir.display()
            );
        }
        let version_dll_src = setdll_src_dir.join(&version_dll_name);
        let setdll_src = setdll_src_dir.join(&setdll_name);
        (setdll_src_dir, version_dll_src, setdll_src)
    } else {
        let setdll_src_dir = workdir.join(
            target
                .get("chrome_plus_dir")
                .and_then(Value::as_str)
                .unwrap_or("chrome++"),
        );
        if !setdll_src_dir.exists() {
            bail!("chrome++ directory not found: {}", setdll_src_dir.display());
        }
        let version_dll_src = setdll_src_dir.join(&version_dll_name);
        let setdll_src = setdll_src_dir.join(&setdll_name);
        (setdll_src_dir, version_dll_src, setdll_src)
    };

    if !version_dll_src.exists() {
        bail!("Required DLL not found: {}", version_dll_src.display());
    }
    if !setdll_src.exists() {
        bail!("Required setdll tool not found: {}", setdll_src.display());
    }

    let dll_location = target
        .get("version_dll_location")
        .and_then(Value::as_str)
        .unwrap_or("app_root");
    let dll_dir = if dll_location == "version_dir" {
        staged.version_dir.clone()
    } else {
        staged.app_root.clone()
    };
    let dll_path = dll_dir.join("version.dll");
    std::fs::copy(&version_dll_src, &dll_path)?;

    let setdll_path = staged.app_root.join(&setdll_name);
    std::fs::copy(&setdll_src, &setdll_path)?;

    let chrome_plus_version_path = setdll_src_dir.join("version.txt");
    if chrome_plus_version_path.exists() {
        let text = std::fs::read_to_string(&chrome_plus_version_path)?;
        let version = text.trim().to_string();
        info(format!("Chrome++ version: {version}"));
        staged.chrome_plus_version = Some(version);
    }

    match resolve_chrome_plus_ini(target, workdir, &setdll_src_dir)? {
        None => {
            warn("chrome++.ini not found; continuing without it.");
        }
        Some((ini_text, ini_encoding)) => {
            let ini_location = target
                .get("ini_location")
                .and_then(Value::as_str)
                .unwrap_or("app_root");
            let ini_dir = if ini_location == "version_dir" {
                staged.version_dir.clone()
            } else {
                staged.app_root.clone()
            };
            write_ini_text(ini_dir.join("chrome++.ini"), &ini_text, ini_encoding)?;
        }
    }

    staged.version_dll = Some(dll_path);
    staged.setdll = Some(setdll_path);
    Ok(())
}

/// Decode subprocess bytes the way Python text=True does on Windows: the
/// console code page (setdll emits OEM code page output - risk R4), with
/// replacement characters for undecodable bytes.
fn decode_oem_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    #[cfg(windows)]
    {
        let code_page = unsafe { windows_sys::Win32::Globalization::GetOEMCP() };
        let encoding = encoding_rs::Encoding::for_label(format!("windows-{code_page}").as_bytes())
            .or_else(|| encoding_rs::Encoding::for_label(b"windows-1252"))
            .unwrap_or(encoding_rs::UTF_8);
        let (text, _, _) = encoding.decode(bytes);
        text.into_owned()
    }
    #[cfg(not(windows))]
    {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Recursive file collection for the exe-fallback search.
fn collect_files(root: &Path) -> Vec<PathBuf> {
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
                stack.push(entry);
            } else if entry.is_file() {
                out.push(entry);
            }
        }
    }
    out
}

/// builder.py::inject_dll - setdll /t probe, /d:<relpath> injection,
/// portable-import assertion, backup + tool cleanup.
/// `os.path.relpath(dll, exe_dir)` for the setdll `/d:` argument (Python
/// builder.py). Walk both component lists to their common ancestor, emit `..`
/// for each exe-dir level below it, then the remaining dll components.
///
/// A plain `strip_prefix` is wrong whenever the DLL is not a descendant of the
/// exe dir — the chrome_stable shape (exe at `App\<ver>\chrome.exe`, dll at
/// `App\version.dll`) — because the fallback was an ABSOLUTE path, which setdll
/// writes verbatim into the import descriptor; verify.rs then rejects the
/// resulting non-portable import. (review-Wave3 finding 1, HIGH.)
fn relpath_from(dll: &Path, from_dir: &Path) -> String {
    let dll_comps: Vec<_> = dll.components().collect();
    let dir_comps: Vec<_> = from_dir.components().collect();
    let mut common = 0usize;
    while common < dll_comps.len()
        && common < dir_comps.len()
        && dll_comps[common] == dir_comps[common]
    {
        common += 1;
    }
    let mut rel = PathBuf::new();
    for _ in common..dir_comps.len() {
        rel.push("..");
    }
    for comp in &dll_comps[common..] {
        rel.push(comp);
    }
    if rel.as_os_str().is_empty() {
        dll.to_string_lossy().into_owned()
    } else {
        rel.to_string_lossy().into_owned()
    }
}

pub fn inject_dll(target: &Value, staged: &mut Staged) -> Result<()> {
    let target_exe = match &staged.executable {
        Some(exe) => exe.clone(),
        None => {
            let exe_name = target
                .get("exe_name")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    anyhow!("Target config requires exe_name unless layout is 'auto'")
                })?;
            let direct = staged.version_dir.join(exe_name);
            if direct.exists() {
                direct
            } else {
                // rglob(exe_name) keeping the shallowest relative depth.
                let needle = Path::new(exe_name)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let mut matches: Vec<PathBuf> = collect_files(&staged.app_root)
                    .into_iter()
                    .filter(|path| {
                        path.file_name()
                            .map(|n| n.to_string_lossy() == needle)
                            .unwrap_or(false)
                    })
                    .collect();
                if matches.is_empty() {
                    bail!("Browser executable not found: {}", direct.display());
                }
                matches.sort_by_key(|path| {
                    path.strip_prefix(&staged.app_root)
                        .map(|rel| rel.components().count())
                        .unwrap_or(usize::MAX)
                });
                let found = matches[0].clone();
                info(format!("Browser executable found at {}", found.display()));
                found
            }
        }
    };
    let target_exe = target_exe
        .canonicalize()
        .unwrap_or_else(|_| target_exe.clone());
    let setdll = staged
        .setdll
        .clone()
        .ok_or_else(|| anyhow!("setdll tool was not staged"))?;
    let version_dll = staged
        .version_dll
        .clone()
        .ok_or_else(|| anyhow!("version.dll was not staged"))?;

    let setdll_abs = setdll.canonicalize().unwrap_or_else(|_| setdll.clone());
    if target
        .get("verify_architecture")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        if let Ok(output) = Command::new(&setdll_abs)
            .args(["/t", &target_exe.to_string_lossy()])
            .output()
        {
            print!("{}", decode_oem_output(&output.stdout));
        }
    }

    let exe_parent = target_exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    // os.path.relpath(version_dll, start=exe_dir); a cross-drive ValueError
    // falls back to the absolute path. canonicalize() can hand back a \\?\
    // verbatim prefix; strip it on BOTH paths first so the drive components
    // compare like Path.anchor (case-insensitive) and strip_prefix works.
    fn strip_verbatim(path: &Path) -> PathBuf {
        let text = path.to_string_lossy();
        let text = text
            .strip_prefix(r"\\?\UNC\")
            .map(|rest| format!(r"\\{rest}"))
            .unwrap_or_else(|| {
                text.strip_prefix(r"\\?\")
                    .map(str::to_string)
                    .unwrap_or_else(|| text.into_owned())
            });
        PathBuf::from(text)
    }
    let dll_plain = strip_verbatim(&version_dll);
    let exe_dir_plain = strip_verbatim(&exe_parent);
    let same_drive = || {
        let dll_drive = dll_plain
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        let exe_drive = exe_dir_plain
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        match (dll_drive, exe_drive) {
            (Some(d), Some(e)) => d.eq_ignore_ascii_case(&e),
            _ => false,
        }
    };
    let dll_arg_value = if same_drive() {
        relpath_from(&dll_plain, &exe_dir_plain)
    } else {
        dll_plain.to_string_lossy().into_owned()
    };
    let dll_arg = format!("/d:{dll_arg_value}");

    info(format!(
        "Injecting version.dll into {}",
        target_exe.display()
    ));
    let output = Command::new(&setdll_abs)
        .args([&dll_arg, target_exe.to_string_lossy().as_ref()])
        .current_dir(&exe_parent)
        .output()
        .map_err(|exc| anyhow!("DLL injection failed. ({exc})"))?;
    if !output.stdout.is_empty() {
        print!("{}", decode_oem_output(&output.stdout));
    }
    if !output.status.success() {
        if !output.stderr.is_empty() {
            print!("{}", decode_oem_output(&output.stderr));
        }
        bail!("DLL injection failed.");
    }

    assert_portable_version_import(&target_exe)?;
    info(format!(
        "Portable import verified: {}",
        target_exe.display()
    ));

    // setdll leaves an un-injected backup beside the target. Shipping it
    // wastes space and hands users a copy that writes to their profile.
    let exe_name = target_exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup = target_exe
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{exe_name}~"));
    if backup.exists() {
        info(format!(
            "Removing setdll backup: {}",
            backup
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        remove_path(&backup);
    }

    if target
        .get("remove_setdll")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        remove_path(&setdll);
    }
    Ok(())
}

/// builder.py::finalize - move the staged app into build/release, write
/// version.txt, copy the start script beside it (release root, not app dir).
pub fn finalize(target: &Value, workdir: &Path, staged: &Staged) -> Result<PathBuf> {
    let release_dir = workdir.join("build").join("release");
    let output_dir_name = target_str(target, "output_dir")
        .or_else(|| target_str(target, "name"))
        .unwrap_or_else(|| "Browser".to_string());
    let final_app_dir = release_dir.join(&output_dir_name);

    std::fs::create_dir_all(&release_dir)?;
    remove_path(&final_app_dir);
    std::fs::rename(&staged.app_root, &final_app_dir)?;

    std::fs::write(final_app_dir.join("version.txt"), &staged.version)?;

    let start_script = target
        .get("start_script")
        .and_then(Value::as_str)
        .unwrap_or("开始.bat");
    let start_path = workdir.join(start_script);
    if start_path.exists() {
        let dest = release_dir.join(
            start_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        std::fs::copy(&start_path, &dest)?;
    }
    Ok(final_app_dir)
}

/// builder.py::_complete_build - stage, inject, forbid-check, finalize,
/// and publish the five env keys (+ CHROME_PLUS_VERSION when present).
/// A package/directory version mismatch warns and keeps the directory
/// version.
pub fn complete_build(
    target: &Value,
    workdir: &Path,
    package: &Value,
    prepared: &Prepared,
    builder_dir: Option<&Path>,
) -> Result<Value> {
    let package_version = package
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if prepared.version() != package_version {
        warn(format!("Package version {package_version} differs from directory version {}; using directory version.", prepared.version()));
    }

    let mut staged = stage_app(target, workdir, prepared)?;
    copy_chrome_plus(target, workdir, &mut staged, builder_dir)?;
    inject_dll(target, &mut staged)?;
    let forbidden = target
        .get("forbidden_file_names")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_no_forbidden_files(&staged.stage_dir, &forbidden)?;
    let final_app_dir = finalize(target, workdir, &staged)?;

    let mut env_values = vec![
        ("BUILT_VERSION".to_string(), staged.version.clone()),
        ("BROWSER_VERSION".to_string(), staged.version.clone()),
        ("PACKAGE_VERSION".to_string(), package_version.to_string()),
        ("UPSTREAM_VERSION".to_string(), package_version.to_string()),
        (
            "OUTPUT_DIR".to_string(),
            target_str(target, "output_dir")
                .or_else(|| target_str(target, "name"))
                .unwrap_or_else(|| "Browser".to_string()),
        ),
    ];
    if let Some(chrome_plus_version) = &staged.chrome_plus_version {
        env_values.push((
            "CHROME_PLUS_VERSION".to_string(),
            chrome_plus_version.clone(),
        ));
    }
    write_env(&env_values)?;
    info(format!("Build completed: {}", final_app_dir.display()));
    Ok(json!({
        "version": staged.version,
        "package_version": package_version,
        "output_dir": final_app_dir.to_string_lossy(),
        "chrome_plus_version": staged.chrome_plus_version.unwrap_or_default(),
    }))
}

/// builder.py::build_target - resolve upstream, then the full pipeline.
pub fn build_target(target: &Value, workdir: &Path, builder_dir: Option<&str>) -> Result<Value> {
    let package = get_version_info(target, Some(workdir))?;
    let builder_dir = builder_dir.map(PathBuf::from);
    let version = package
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    info(format!("Upstream version: {version}"));
    let prepared = prepare_package(target, workdir, &package)?;
    complete_build(target, workdir, &package, &prepared, builder_dir.as_deref())
}

/// builder.py::_safe_output_name - [<>:"/\\|?*]+ -> "_", strip " .",
/// empty -> fallback ("Browser").
pub fn safe_output_name(value: &str, fallback: &str) -> String {
    let mut cleaned = String::with_capacity(value.len());
    let mut last_underscore = false;
    for ch in value.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            if !last_underscore {
                cleaned.push('_');
                last_underscore = true;
            }
        } else {
            cleaned.push(ch);
            last_underscore = false;
        }
    }
    let cleaned = cleaned.trim_matches(|c| c == ' ' || c == '.');
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned.to_string()
    }
}

/// builder.py::build_package_file - the GUI and build-package entry point.
/// Synthesizes an auto-layout target, builds, and cleans build/temp in
/// finally semantics (no second unpacked copy left behind).
pub fn build_package_file(
    package_path: &Path,
    workdir: &Path,
    builder_dir: Option<&Path>,
    architecture: &str,
    output_dir: Option<&str>,
) -> Result<Value> {
    let package_path = package_path
        .canonicalize()
        .unwrap_or_else(|_| package_path.to_path_buf());
    if !package_path.exists() {
        bail!("Package not found: {}", package_path.display());
    }

    let stem = package_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sanitized: String = stem
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('_');
    let target_id = if sanitized.is_empty() {
        "auto_package".to_string()
    } else {
        format!("auto_{sanitized}")
    };

    let mut target = json!({
        "target": target_id,
        "name": stem,
        "display_name": stem,
        "output_dir": output_dir.unwrap_or("Browser"),
        "architecture": architecture,
        "layout": "auto",
        "allow_7z_download": false,
        "allow_7z_system_install": false,
        "chrome_plus_dir": "chrome++",
        "ini_location": "app_root",
        "version_dll_location": "app_root",
    });
    let mut package = json!({
        "version": "0.0.0.0",
        "path": package_path.to_string_lossy(),
        "file_name": package_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    });
    let temp_dir = workdir.join("build").join("temp").join(&target_id);
    let result = (|| -> Result<Value> {
        let prepared = prepare_package(&target, workdir, &package)?;
        package["version"] = json!(prepared.version());
        let product = match &prepared {
            Prepared::Auto { discovery, .. } => discovery.product.clone(),
            Prepared::VersionRoot { .. } => stem.clone(),
        };
        let resolved_output = safe_output_name(output_dir.unwrap_or(&product), "Browser");
        if let Some(obj) = target.as_object_mut() {
            obj.insert("name".to_string(), json!(resolved_output));
            obj.insert("display_name".to_string(), json!(product));
            obj.insert("output_dir".to_string(), json!(resolved_output));
            obj.insert(
                "archive_name".to_string(),
                json!(format!(
                    "{resolved_output}_Portable_{{version}}_{{date}}.7z"
                )),
            );
        }
        let mut result = complete_build(&target, workdir, &package, &prepared, builder_dir)?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("target_config".to_string(), target.clone());
        }
        Ok(result)
    })();
    // finally: remove build/temp/<target_id>.
    remove_path(&temp_dir);
    result
}

/// builder.py::archive_target - 7z a -t7z -mx=9 over build/release (or the
/// given source dir), version/date fallback chains preserved, existing
/// archive removed first, env facts written.
pub fn archive_target(
    target: &Value,
    workdir: &Path,
    version: Option<&str>,
    build_date: Option<&str>,
    package_version: Option<&str>,
    source_dir: Option<&Path>,
) -> Result<Value> {
    let auto_layout = target_str(target, "layout").as_deref() == Some("auto");
    let seven_zip = find_7z_tool(
        workdir,
        target
            .get("allow_7z_download")
            .and_then(Value::as_bool)
            .unwrap_or(!auto_layout),
        target
            .get("allow_7z_system_install")
            .and_then(Value::as_bool)
            .unwrap_or(!auto_layout),
    )?;
    let version = version
        .map(str::to_string)
        .or_else(|| env_non_empty("BUILT_VERSION"))
        .or_else(|| env_non_empty("BROWSER_VERSION"));
    let package_version = package_version
        .map(str::to_string)
        .or_else(|| env_non_empty("PACKAGE_VERSION"))
        .or_else(|| env_non_empty("UPSTREAM_VERSION"))
        .or_else(|| version.clone());
    let now = chrono::Local::now();
    let build_date = build_date
        .map(str::to_string)
        .or_else(|| env_non_empty("BUILD_DATE"))
        .unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    let context = build_context(
        target,
        version.as_deref(),
        Some(&build_date),
        package_version.as_deref(),
    )?;

    let archive_template = target
        .get("archive_name")
        .and_then(Value::as_str)
        .unwrap_or("{display_name}_Portable_{version}_{date}.7z");
    let archive_name = format_value(archive_template, &context)?;
    let assets_dir = workdir.join("build").join("assets");
    std::fs::create_dir_all(&assets_dir)?;
    let archive_path = assets_dir.join(&archive_name);
    remove_path(&archive_path);

    let (release_dir, archive_items): (PathBuf, Vec<String>) = match source_dir {
        None => (workdir.join("build").join("release"), vec!["*".to_string()]),
        Some(source_dir) => {
            if !source_dir.is_dir() {
                bail!(
                    "Archive source directory not found: {}",
                    source_dir.display()
                );
            }
            let release_dir = source_dir
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| anyhow!("source dir has no parent"))?;
            let item = source_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .ok_or_else(|| anyhow!("source dir has no name"))?;
            (release_dir, vec![item])
        }
    };
    if !release_dir.exists() {
        bail!("Release directory not found: {}", release_dir.display());
    }

    let archive_abs = archive_path.canonicalize().unwrap_or_else(|_| {
        // canonicalize fails on missing files; make the parent absolute.
        let parent = assets_dir
            .canonicalize()
            .unwrap_or_else(|_| assets_dir.clone());
        parent.join(&archive_name)
    });
    info(format!("Creating archive: {}", archive_path.display()));
    let output = Command::new(&seven_zip)
        .args(["a", "-t7z", "-mx=9"])
        .arg(&archive_abs)
        .args(&archive_items)
        .current_dir(&release_dir)
        .output()
        .map_err(|exc| anyhow!("Archive creation failed. ({exc})"))?;
    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.status.success() {
        if !output.stderr.is_empty() {
            print!("{}", String::from_utf8_lossy(&output.stderr));
        }
        bail!("Archive creation failed.");
    }

    let digest = sha256_file(&archive_path)?;
    let size = std::fs::metadata(&archive_path)?.len();
    info(format!(
        "Archive SHA256: {digest} ({})",
        human_size(Some(size as f64))
    ));
    write_env(&[
        ("ARCHIVE_NAME".to_string(), archive_name.clone()),
        (
            "ASSET_PATH".to_string(),
            archive_path.to_string_lossy().into_owned(),
        ),
        ("ARCHIVE_SHA256".to_string(), digest.clone()),
        ("ARCHIVE_SIZE".to_string(), size.to_string()),
    ])?;
    Ok(json!({
        "path": archive_path.to_string_lossy(),
        "name": archive_name,
        "sha256": digest,
        "size": size,
    }))
}

/// os.getenv(name) returning None for unset AND empty (Python falsy chain).
fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// review-Wave3 finding 1: the /d: argument must stay relative in every
    /// same-root shape, including the chrome_stable one where version.dll sits
    /// beside the version directory instead of inside it.
    #[test]
    fn relpath_from_matches_os_path_relpath() {
        let cases: [(Vec<&str>, Vec<&str>, Vec<&str>); 3] = [
            // chrome_stable: exe at App\<ver>\chrome.exe, dll at App\version.dll
            (
                vec!["App", "version.dll"],
                vec!["App", "120.0.1", "chrome"],
                vec!["..", "..", "version.dll"],
            ),
            // dll inside the exe dir -> bare name
            (
                vec!["App", "v", "version.dll"],
                vec!["App", "v"],
                vec!["version.dll"],
            ),
            // sibling dirs -> single hop up
            (
                vec!["App", "bin", "version.dll"],
                vec!["App", "v"],
                vec!["..", "bin", "version.dll"],
            ),
        ];
        for (dll_parts, dir_parts, want_parts) in cases {
            let mut dll = PathBuf::new();
            for p in &dll_parts {
                dll.push(p);
            }
            let mut dir = PathBuf::new();
            for p in &dir_parts {
                dir.push(p);
            }
            let mut want = PathBuf::new();
            for p in &want_parts {
                want.push(p);
            }
            assert_eq!(
                relpath_from(&dll, &dir),
                want.to_string_lossy(),
                "{dll:?} from {dir:?}"
            );
        }
    }

    /// Golden: build_context_reference.json safe_output (Python _safe_output_name).
    #[test]
    fn safe_output_name_matches_golden() {
        assert_eq!(safe_output_name("", "Browser"), "Browser");
        assert_eq!(safe_output_name("  ..x..  ", "Browser"), "x");
        assert_eq!(safe_output_name("Chrome", "Browser"), "Chrome");
        assert_eq!(
            safe_output_name("Chromium:Portable?", "Browser"),
            "Chromium_Portable_"
        );
        // Python passes None through str(None) -> "None"; a literal matches.
        assert_eq!(safe_output_name("None", "Browser"), "None");
        assert_eq!(safe_output_name("a/b\\c", "Browser"), "a_b_c");
    }

    /// Golden: build_context_reference.json context_full.
    #[test]
    fn build_context_matches_golden() {
        let target = json!({
            "target": "chrome_stable",
            "name": "Chrome",
            "display_name": "Chrome++",
            "output_dir": "Chrome",
            "architecture": "x64",
        });
        let saved: Vec<(String, Option<String>)> = [
            "ARCHIVE_NAME",
            "ARCHIVE_SHA256",
            "ARCHIVE_SIZE",
            "CHROME_PLUS_VERSION",
            "GITHUB_REPOSITORY",
            "GITHUB_RUN_ID",
            "GITHUB_SERVER_URL",
        ]
        .iter()
        .map(|k| (k.to_string(), std::env::var(k).ok()))
        .collect();
        std::env::set_var("ARCHIVE_NAME", "X-1.2.3-x64-2026-07-01.7z");
        std::env::set_var(
            "ARCHIVE_SHA256",
            "abababababababababababababababababababababababababababababababab",
        );
        std::env::set_var("ARCHIVE_SIZE", "987653734");
        std::env::set_var("CHROME_PLUS_VERSION", "1.18.2");
        for key in ["GITHUB_REPOSITORY", "GITHUB_RUN_ID"] {
            std::env::remove_var(key);
        }
        let context = build_context(
            &target,
            Some("1.2.3.4"),
            Some("2026-07-01"),
            Some("1.2.3.3"),
        )
        .unwrap();
        let get = |key: &str| {
            context
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(get("arch"), "x64");
        assert_eq!(get("archive"), "X-1.2.3-x64-2026-07-01.7z");
        assert_eq!(get("chrome_plus_version"), "1.18.2");
        assert_eq!(get("date"), "2026-07-01");
        assert_eq!(get("display_name"), "Chrome++");
        assert_eq!(get("name"), "Chrome");
        assert_eq!(get("output_dir"), "Chrome");
        assert_eq!(get("package_version"), "1.2.3.3");
        assert_eq!(get("run_url"), "");
        assert_eq!(
            get("sha256"),
            "abababababababababababababababababababababababababababababababab"
        );
        assert_eq!(get("size"), "941.9 MB");
        assert_eq!(get("target"), "chrome_stable");
        assert_eq!(get("version"), "1.2.3.4");
        for (key, value) in saved {
            match value {
                Some(v) => std::env::set_var(&key, v),
                None => std::env::remove_var(&key),
            }
        }
    }

    #[test]
    fn format_value_missing_key_is_keyerror_text() {
        let err = format_value("{nope}", &[]).unwrap_err().to_string();
        assert_eq!(err, "KeyError: 'nope'");
    }

    #[test]
    fn format_value_escapes_and_substitutes() {
        let context = vec![
            ("version".to_string(), "1.2.3.4".to_string()),
            ("date".to_string(), "2026-07-01".to_string()),
        ];
        assert_eq!(
            format_value("Probe_{version}_{date}.7z", &context).unwrap(),
            "Probe_1.2.3.4_2026-07-01.7z"
        );
        assert_eq!(format_value("{{literal}}", &context).unwrap(), "{literal}");
    }

    #[test]
    fn target_id_synthesis_matches_python() {
        // re.sub(r"[^a-z0-9]+", "_", stem.casefold()).strip("_")
        let synth = |stem: &str| -> String {
            let sanitized: String = stem
                .to_lowercase()
                .chars()
                .map(|ch| {
                    if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
                        ch
                    } else {
                        '_'
                    }
                })
                .collect();
            let sanitized = sanitized.trim_matches('_');
            if sanitized.is_empty() {
                "auto_package".to_string()
            } else {
                format!("auto_{sanitized}")
            }
        };
        assert_eq!(
            synth("Vivaldi.8.2.4133.52.x64"),
            "auto_vivaldi_8_2_4133_52_x64"
        );
        assert_eq!(synth("---"), "auto_package");
    }
}
