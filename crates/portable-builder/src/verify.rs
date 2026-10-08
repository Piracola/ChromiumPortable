//! Port of portable_builder/verify.py - post-archive verification.
//! Owner: Wave3-A (contract: _migration/pe-golden/verify_smoke_contract.md
//! + addendum; golden verify_archive_locate_golden.json).
//!
//! Judgement chain after extraction:
//! (a) assert_portable_version_import  (b) assert_no_setdll_backup
//! (c) assert_no_forbidden_files       (d) smoke_test
//!
//! Smoke semantics (AGENTS.md lesson, codeified): the browser exit code
//! alone proves little - msedge.exe is a launcher stub - so the real
//! assertion is the Chrome++-redirected portable Data directory appearing
//! inside the archive, polled at 0.5s because detached children set the
//! profile up long after the launcher exits.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::config::get_target;
use crate::discovery::analyze_extracted_app;
use crate::github_env::write_env;
use crate::log_fmt::{info, warn};
use crate::multi::env_name;
use crate::pe::assert_portable_version_import;
use crate::tools::{
    assert_no_forbidden_files, extract_with_7z, find_7z_tool, find_version_dir, remove_path,
    sha256_file,
};

/// verify.py DEFAULT_SMOKE_ARGS.
pub const DEFAULT_SMOKE_ARGS: [&str; 6] = [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--dump-dom",
    "about:blank",
];

/// release.py::archive_name_regex equivalent: build the anchored regex for
/// the target's archive_name and fullmatch candidate names (addendum 1:
/// fullmatch, NOT search). Returns the compiled pattern.
fn archive_name_regex(target: &Value) -> Result<regex::Regex> {
    // archive_name_regex_text already returns a ^...$ anchored pattern
    // (re.IGNORECASE in Python); wrap for case-insensitive fullmatch.
    let pattern = crate::release::archive_name_regex_text(target)?;
    Ok(regex::RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .build()?)
}

/// verify.py::find_target_archive - glob *.7z, fullmatch the archive_name
/// regex, pick the newest by mtime. No match: error listing every archive
/// present.
pub fn find_target_archive(target: &Value, workdir: &Path) -> Result<PathBuf> {
    let assets_dir = workdir.join("build").join("assets");
    if !assets_dir.exists() {
        bail!("Assets directory not found: {}", assets_dir.display());
    }
    let regex = archive_name_regex(target)?;
    let mut matches: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&assets_dir)? {
        let entry = entry?;
        let path = entry.path();
        let is_7z = path
            .extension()
            .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case("7z"))
            .unwrap_or(false);
        if !is_7z {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if regex.is_match(&name) {
            matches.push(path);
        }
    }
    if matches.is_empty() {
        let mut available: Vec<String> = std::fs::read_dir(&assets_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case("7z"))
                    .unwrap_or(false)
            })
            .map(|path| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        available.sort();
        let available = if available.is_empty() {
            "(none)".to_string()
        } else {
            available.join(", ")
        };
        bail!(
            "No archive matching {} in {}. Present: {}",
            target
                .get("archive_name")
                .and_then(Value::as_str)
                .unwrap_or(""),
            assets_dir.display(),
            available
        );
    }
    let newest = matches
        .into_iter()
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .expect("non-empty matches");
    Ok(newest)
}

/// verify.py::locate_executable - auto layout re-runs the static analysis,
/// fixed layouts use find_version_dir + exe_name with normpath semantics.
pub fn locate_executable(target: &Value, app_root: &Path) -> Result<PathBuf> {
    if target.get("layout").and_then(Value::as_str) == Some("auto") {
        let result = analyze_extracted_app(
            app_root,
            target
                .get("architecture")
                .and_then(Value::as_str)
                .unwrap_or("x64"),
        )?;
        info(format!(
            "Auto-detected archived browser executable: {}",
            result.executable.display()
        ));
        return Ok(result.executable);
    }
    let version_dir = find_version_dir(app_root, None)
        .ok_or_else(|| anyhow!("No version directory found under {}", app_root.display()))?;
    let exe_name = target
        .get("exe_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Target config requires exe_name"))?;
    let executable = version_dir.join(exe_name);
    // os.path.normpath then existence check.
    let executable = normalize_path(&executable);
    if !executable.exists() {
        bail!(
            "Browser executable not found in archive: {}",
            executable.display()
        );
    }
    Ok(executable)
}

/// Lexical normalisation (no FS access), like os.path.normpath on Windows.
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// verify.py::wait_for_directory - poll at 0.5s until timeout.
pub fn wait_for_directory(path: &Path, timeout: Duration, interval: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if path.is_dir() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(interval);
    }
}

/// verify.py::describe_tree - sorted relative paths at depth <= 2 for the
/// failure diagnostics.
pub fn describe_tree(root: &Path, depth: usize) -> String {
    let mut entries: Vec<String> = Vec::new();
    let mut all: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut items: Vec<PathBuf> = read_dir.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        items.sort();
        for item in items {
            if item.is_dir() {
                stack.push(item.clone());
            }
            all.push(item);
        }
    }
    all.sort();
    for path in all {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        if relative.components().count() <= depth {
            let suffix = if path.is_dir() { "/" } else { "" };
            entries.push(format!(
                "{}{}",
                relative.to_string_lossy().replace('\\', "/"),
                suffix
            ));
        }
    }
    if entries.is_empty() {
        "(empty)".to_string()
    } else {
        entries.join(", ")
    }
}

/// verify.py::run_browser - launch and wait for the direct child. The exit
/// code alone proves little: zero is not proof, non-zero is failure.
pub fn run_browser(
    executable: &Path,
    args: &[String],
    timeout: Duration,
    cwd: &Path,
) -> Result<()> {
    let mut command_text = executable.to_string_lossy().into_owned();
    for arg in args {
        command_text.push(' ');
        command_text.push_str(arg);
    }
    info(format!("Running {command_text}"));
    let start = Instant::now();
    let status = Command::new(executable)
        .args(args)
        .current_dir(cwd)
        .status();
    let status = match status {
        Ok(status) => status,
        Err(exc) => {
            // subprocess.TimeoutExpired analog: handled by the caller
            // through the elapsed check; spawn errors surface directly.
            bail!("Browser failed to start: {exc}");
        }
    };
    if start.elapsed() > timeout {
        // The child outlived its budget: Python raises TimeoutExpired.
        bail!(
            "Browser did not exit within {}s: {command_text}",
            timeout.as_secs()
        );
    }
    let code = status.code().unwrap_or(-1);
    if code != 0 {
        bail!("Browser exited with code {code}: {command_text}");
    }
    Ok(())
}

/// verify.py::assert_no_setdll_backup - only the exact <exe>~ path, no
/// globbing (a tree-wide *~ scan would false-positive on unrelated files).
pub fn assert_no_setdll_backup(extracted_root: &Path, executable: &Path) -> Result<()> {
    let exe_name = executable
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup = executable
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{exe_name}~"));
    if backup.exists() {
        let relative = backup
            .strip_prefix(extracted_root)
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| backup.to_string_lossy().into_owned());
        bail!("Archive still contains the setdll backup {relative}; it is an un-injected copy of the browser and must not ship.");
    }
    Ok(())
}

/// verify.py::smoke_test - the four-step judgement: no pre-shipped Data
/// dir, version.txt readout, --version probe, DEFAULT_SMOKE_ARGS run, then
/// the portable Data directory poll (0.5s x 60s).
pub fn smoke_test(
    target: &Value,
    extracted_root: &Path,
    app_root: &Path,
    executable: &Path,
) -> Result<()> {
    let data_dir_name = target
        .get("smoke_data_dir")
        .and_then(Value::as_str)
        .unwrap_or("Data");
    let data_dir = extracted_root.join(data_dir_name);
    if data_dir.exists() {
        bail!(
            "Archive already ships a '{data_dir_name}' directory: {}",
            data_dir.display()
        );
    }

    let version_file = app_root.join("version.txt");
    if version_file.exists() {
        // Not asserted against the browser's own reported version: Helium's
        // package version and its bundled Chromium version differ by design.
        let text = std::fs::read_to_string(&version_file)?;
        info(format!("version.txt records: {}", text.trim()));
    }

    let smoke_timeout = target
        .get("smoke_timeout")
        .and_then(Value::as_u64)
        .unwrap_or(120);

    // Chromium is a GUI-subsystem binary, so a captured pipe stays empty;
    // the exit code is the signal that the patched PE loaded our DLL.
    run_browser(
        executable,
        &["--version".to_string()],
        Duration::from_secs(smoke_timeout),
        app_root,
    )?;

    let smoke_args: Vec<String> = target
        .get("smoke_args")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| DEFAULT_SMOKE_ARGS.iter().map(|s| s.to_string()).collect());
    run_browser(
        executable,
        &smoke_args,
        Duration::from_secs(smoke_timeout + 60),
        app_root,
    )?;

    // Poll rather than check once: the launcher exits long before its
    // detached children have set the profile up.
    let data_timeout = target
        .get("smoke_data_timeout")
        .and_then(Value::as_u64)
        .unwrap_or(60);
    if !wait_for_directory(
        &data_dir,
        Duration::from_secs(data_timeout),
        Duration::from_millis(500),
    ) {
        bail!("Chrome++ did not create the portable data directory '{data_dir_name}', so the profile went to the user profile instead. Extracted tree: {}", describe_tree(extracted_root, 2));
    }
    info(format!(
        "Portable data directory created inside the archive: {}",
        data_dir.display()
    ));
    Ok(())
}

/// verify.py::cleanup - browser subprocesses outlive the parent briefly and
/// hold DLL handles: 4 attempts x 3s delay.
pub fn cleanup(extracted_root: &Path, attempts: u32, delay: Duration) -> bool {
    for attempt in 0..attempts {
        let result = if extracted_root.is_dir() {
            std::fs::remove_dir_all(extracted_root)
        } else if extracted_root.exists() {
            std::fs::remove_file(extracted_root)
        } else {
            return true;
        };
        match result {
            Ok(()) => return true,
            Err(exc) => {
                if attempt == attempts - 1 {
                    warn(format!(
                        "Could not clean up {}: {exc}",
                        extracted_root.display()
                    ));
                    return false;
                }
                std::thread::sleep(delay);
            }
        }
    }
    false
}

/// verify.py::verify_target - extract the archive, run the assertion chain,
/// optionally smoke, then clean up.
pub fn verify_target(
    target: &Value,
    workdir: &Path,
    archive: Option<&Path>,
    smoke: bool,
) -> Result<Value> {
    let target_id = target.get("target").and_then(Value::as_str).unwrap_or("");
    let archive = match archive {
        Some(path) => path.to_path_buf(),
        None => find_target_archive(target, workdir)?,
    };
    info(format!(
        "Verifying archive: {} ({} bytes)",
        archive
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::fs::metadata(&archive)?.len()
    ));
    info(format!("Archive SHA256: {}", sha256_file(&archive)?));

    let extracted_root = workdir.join("build").join("verify").join(target_id);
    remove_path(&extracted_root);
    std::fs::create_dir_all(&extracted_root)?;
    let auto_layout = target.get("layout").and_then(Value::as_str) == Some("auto");
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
    extract_with_7z(&archive, &extracted_root, &seven_zip)?;

    let output_dir_name = target
        .get("output_dir")
        .or_else(|| target.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Browser");
    let app_root = extracted_root.join(output_dir_name);
    if !app_root.is_dir() {
        let mut present: Vec<String> = std::fs::read_dir(&extracted_root)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        present.sort();
        let present = if present.is_empty() {
            "(empty)".to_string()
        } else {
            present.join(", ")
        };
        bail!("Archive does not contain '{output_dir_name}'. Root contains: {present}");
    }

    let executable = locate_executable(target, &app_root)?;
    assert_portable_version_import(&executable)?;
    assert_no_setdll_backup(&extracted_root, &executable)?;
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
    assert_no_forbidden_files(&extracted_root, &forbidden)?;

    if smoke {
        smoke_test(target, &extracted_root, &app_root, &executable)?;
    } else {
        info("Smoke launch disabled; import table check only.");
    }

    cleanup(&extracted_root, 4, Duration::from_secs(3));

    info(format!(
        "Verification passed: {}",
        archive
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    Ok(json!({
        "archive": archive.to_string_lossy(),
        "executable": executable.to_string_lossy(),
    }))
}

/// verify.py::verify_targets - per-target verification with the env-prefix
/// skip logic, then VERIFIED_TARGETS written through write_env.
pub fn verify_targets(
    config: &Value,
    target_names: &[String],
    workdir: &Path,
    smoke: bool,
) -> Result<Vec<String>> {
    let mut verified: Vec<String> = Vec::new();
    for target_name in target_names {
        let target = get_target(config, target_name)?;
        let prefix = target
            .get("env_prefix")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| env_name(target_name));
        let update_flag = format!("{prefix}_UPDATE");
        let updated = std::env::var(&update_flag)
            .map(|value| value.to_lowercase() == "true")
            .unwrap_or(false);
        let forced = std::env::var("GITHUB_EVENT_NAME").unwrap_or_default() == "workflow_dispatch";
        if std::env::var(&update_flag).is_ok() && !(updated || forced) {
            info(format!(
                "Skipping {target_name}; it was not rebuilt in this run."
            ));
            continue;
        }
        verify_target(&target, workdir, None, smoke)?;
        verified.push(target_name.clone());
    }

    if !verified.is_empty() {
        let mut sorted = verified.clone();
        sorted.sort();
        write_env(&[("VERIFIED_TARGETS".to_string(), sorted.join(","))])?;
    }
    Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden: verify_archive_locate_golden.json - the regex is a fullmatch
    /// over ProbeBrowser_{version}_{date}.7z, the newest mtime wins, and the
    /// no-match error lists every archive present ("(none)" when empty).
    #[test]
    fn find_target_archive_fullmatch_and_mtime() {
        let dir = std::env::temp_dir().join(format!("pb-verify-locate-{}", std::process::id()));
        let assets = dir.join("build").join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let target = serde_json::json!({
            "name": "ProbeBrowser",
            "display_name": "ProbeBrowser",
            "archive_name": "ProbeBrowser_{version}_{date}.7z",
        });
        // Older match + newer match + non-matching file: newest mtime wins.
        let a = assets.join("ProbeBrowser_1.0.0.0_2026-07-01.7z");
        let b = assets.join("ProbeBrowser_2.0.0.0_2026-07-02.7z");
        let c = assets.join("Other_1.0.0.0_2026-07-01.7z");
        for (path, age) in [(&a, 3), (&b, 1), (&c, 0)] {
            std::fs::write(path, b"x").unwrap();
            let stamp = std::time::SystemTime::now() - Duration::from_secs(age * 10);
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.set_modified(stamp).unwrap();
        }
        let selected = find_target_archive(&target, &dir).unwrap();
        assert_eq!(selected.file_name().unwrap(), b.file_name().unwrap());

        // No match: exact error listing present archives.
        let mut no_match_target = target.clone();
        no_match_target["archive_name"] = serde_json::json!("ProbeBrowser_{version}_{date}_9x.7z");
        let err = find_target_archive(&no_match_target, &dir)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("No archive matching ProbeBrowser_{version}_{date}_9x.7z in "),
            "{err}"
        );
        assert!(err.contains("ProbeBrowser_1.0.0.0_2026-07-01.7z"), "{err}");
        assert!(err.contains("Other_1.0.0.0_2026-07-01.7z"), "{err}");

        // Empty assets dir: the error lists "Present: (none)".
        let empty_dir =
            std::env::temp_dir().join(format!("pb-verify-locate-empty-{}", std::process::id()));
        let empty_assets = empty_dir.join("build").join("assets");
        std::fs::create_dir_all(&empty_assets).unwrap();
        let err = find_target_archive(&target, &empty_dir)
            .unwrap_err()
            .to_string();
        assert!(err.ends_with("Present: (none)"), "{err}");
        remove_path(&dir);
        remove_path(&empty_dir);
    }

    /// The golden regex itself (verify_archive_locate_golden.json).
    #[test]
    fn archive_regex_matches_golden_pattern() {
        let dir = std::env::temp_dir().join(format!("pb-verify-regex-{}", std::process::id()));
        let assets = dir.join("build").join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let target = serde_json::json!({
            "name": "ProbeBrowser",
            "archive_name": "ProbeBrowser_{version}_{date}.7z",
        });
        let pattern = crate::release::archive_name_regex_text(&target).unwrap();
        // The Python golden compiled: ^ProbeBrowser_\d+(?:\.\d+)+_\d{4}-\d{2}-\d{2}\.7z$
        assert!(pattern.contains(r"\d+(?:\.\d+)+"), "{pattern}");
        assert!(pattern.contains(r"\d{4}-\d{2}-\d{2}"), "{pattern}");
        remove_path(&dir);
    }

    #[test]
    fn setdll_backup_checks_exact_path_only() {
        let dir = std::env::temp_dir().join(format!("pb-verify-backup-{}", std::process::id()));
        let root = dir.join("extracted");
        let app = root.join("Probe");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("chrome.exe~"), b"x").unwrap();
        std::fs::write(root.join("unrelated~"), b"x").unwrap();
        let exe = app.join("chrome.exe");
        // The exact <exe>~ path exists -> error.
        assert!(assert_no_setdll_backup(&root, &exe).is_err());
        // Removing it, the unrelated *~ elsewhere must NOT trip the guard.
        std::fs::remove_file(app.join("chrome.exe~")).unwrap();
        assert!(assert_no_setdll_backup(&root, &exe).is_ok());
        remove_path(&dir);
    }

    #[test]
    fn describe_tree_shapes_like_python() {
        let dir = std::env::temp_dir().join(format!("pb-verify-tree-{}", std::process::id()));
        let root = dir.join("extracted");
        std::fs::create_dir_all(root.join("Probe").join("1.0.0.0")).unwrap();
        std::fs::write(root.join("Probe").join("chrome.exe"), b"x").unwrap();
        std::fs::write(root.join("Probe").join("1.0.0.0").join("deep.txt"), b"x").unwrap();
        let tree = describe_tree(&root, 2);
        assert!(tree.contains("Probe/"), "{tree}");
        assert!(tree.contains("Probe/chrome.exe"), "{tree}");
        assert!(tree.contains("Probe/1.0.0.0/"), "{tree}");
        assert!(!tree.contains("deep.txt"), "{tree}");
        remove_path(&dir);
    }

    #[test]
    fn wait_for_directory_polls_and_times_out() {
        let dir = std::env::temp_dir().join(format!("pb-verify-wait-{}", std::process::id()));
        let missing = dir.join("Data");
        remove_path(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!wait_for_directory(
            &missing,
            Duration::from_millis(300),
            Duration::from_millis(50)
        ));
        std::fs::create_dir_all(&missing).unwrap();
        assert!(wait_for_directory(
            &missing,
            Duration::from_secs(2),
            Duration::from_millis(50)
        ));
        remove_path(&dir);
    }

    #[test]
    fn cleanup_retries_then_reports() {
        let dir = std::env::temp_dir().join(format!("pb-verify-clean-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("f.txt"), b"x").unwrap();
        assert!(cleanup(&dir, 4, Duration::from_millis(10)));
        assert!(!dir.exists());
        // Missing path: immediate success.
        assert!(cleanup(&dir, 4, Duration::from_millis(10)));
    }
}
