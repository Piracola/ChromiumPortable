//! GUI 后端：路径探测、语言包、目录表、计划、子进程执行、状态机。
//!
//! 放在引擎 crate 里而不是 builder-app：这一层不依赖 tauri，可以直接跑单测，
//! ubuntu CI 上也能覆盖 GUI 的全部逻辑（builder-app 只剩命令转发）。
//! 逐项语义对照 scripts/gui_core.py / gui_plan.py / gui_runner.py / gui.py。

use serde_json::Value;
use std::path::{Path, PathBuf};

pub mod controller;
pub mod plan;
pub mod runner;

pub const APP_NAME: &str = "ChromiumPortableBuilder";
/// 子进程把日志写到这个环境变量指向的文件（runner.rs 用）。
pub const LOG_ENV: &str = "CHROMIUMPORTABLE_LOG_FILE";
pub const INSTALLER_SUFFIXES: [&str; 6] = [".exe", ".msi", ".7z", ".zip", ".rar", ".cab"];
pub const ARCHITECTURES: [&str; 3] = ["x64", "x86", "arm64"];
pub const INSTALLER_DIR_NAME: &str = "installers";

/// 目录页里的在线目标顺序（与向导保持一致，源：gui_core.ONLINE_TARGETS）。
pub const ONLINE_TARGETS: [&str; 9] = [
    "brave_stable",
    "vivaldi_stable",
    "opera_stable",
    "thorium_stable",
    "cse360_stable",
    "chrome_stable",
    "chrome_beta",
    "edge_stable",
    "helium_stable",
];

/// exe 所在目录（gui_core.app_root 的 frozen 分支）。
pub fn app_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn looks_like_root(dir: &Path) -> bool {
    dir.join("catalog").is_dir() || dir.join("scripts").is_dir() || dir.join("crates").is_dir()
}

/// 从 start 起向上找资源根（cargo test 的 cwd 是 crate 目录，不是仓库根，
/// 所以不能只看当前目录）。
fn find_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start.to_path_buf());
    for _ in 0..5 {
        let dir = current?;
        if looks_like_root(&dir) {
            return Some(dir);
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    None
}

/// 内嵌资源根（gui_core.engine_root）。
///
/// Python 用 sys._MEIPASS / 仓库根区分 frozen 与源码；Rust 只有一种产物，
/// 所以改成按「哪里找得到资源」判断：环境变量 → 看着像资源根的当前目录 →
/// exe 旁。开发期（cargo run）落在仓库根，分发期落在 exe 旁。
pub fn engine_root() -> PathBuf {
    if let Some(raw) = std::env::var_os("CPB_ENGINE_ROOT") {
        return PathBuf::from(raw);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(root) = find_root(&cwd) {
        return root;
    }
    find_root(&app_root()).unwrap_or_else(app_root)
}

pub fn locales_path() -> PathBuf {
    engine_root()
        .join("scripts")
        .join("locales")
        .join("wizard.json")
}

pub fn catalog_path() -> PathBuf {
    engine_root().join("catalog").join("browser_catalog.json")
}

fn catalog_json() -> Option<Value> {
    let text = std::fs::read_to_string(catalog_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn catalog_targets_map() -> serde_json::Map<String, Value> {
    catalog_json()
        .as_ref()
        .and_then(|data| data.get("targets"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// gui_core.load_catalog_targets：先按 ONLINE_TARGETS 的固定顺序，其余按名字排序。
pub fn load_catalog_targets() -> Vec<String> {
    let targets = catalog_targets_map();
    let mut ordered: Vec<String> = ONLINE_TARGETS
        .iter()
        .filter(|key| targets.contains_key(**key))
        .map(|key| (*key).to_string())
        .collect();
    let mut rest: Vec<String> = targets
        .keys()
        .filter(|key| !ordered.iter().any(|seen| seen == *key))
        .cloned()
        .collect();
    rest.sort();
    ordered.extend(rest);
    ordered
}

/// 在线目标下拉的选项：(id, display_name)；没有 display_name 就退回 id。
pub fn target_choices() -> Vec<(String, String)> {
    let targets = catalog_targets_map();
    load_catalog_targets()
        .into_iter()
        .map(|key| {
            let label = targets
                .get(&key)
                .and_then(Value::as_object)
                .and_then(|entry| entry.get("display_name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| key.clone());
            (key, label)
        })
        .collect()
}

/// gui_core.format_size：B 取整、其余一位小数、GB 收口。
pub fn format_size(size: u64) -> String {
    let mut value = size as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if value < 1024.0 || unit == "GB" {
            return if unit == "B" {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.0;
    }
    format!("{value:.1} GB")
}

fn writable(path: &Path) -> bool {
    if std::fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(".cpb-write-test");
    match std::fs::write(&probe, "ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// gui_core.default_workdir：现成的工作区 > exe 旁 > %LOCALAPPDATA%。
pub fn default_workdir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for marker in ["portable_builder", "installers", "build", ".git"] {
        if cwd.join(marker).exists() {
            return cwd;
        }
    }
    let beside = app_root();
    if writable(&beside) {
        return beside;
    }
    let fallback = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ChromiumPortable");
    if std::fs::create_dir_all(&fallback).is_ok() {
        fallback
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// 打包后自检需要的语言包键（gui_core.required_locale_keys 的清单）。
///
/// Python 还会扫 gui*.py 里的 "gui_*" 字面量补齐清单；Rust 侧这些 key 由
/// app.js 使用，扫 JS 字面量同样可行——但清单本身就是契约，测试里对着
/// 真实 wizard.json 校一遍就够（见下方测试）。
pub const REQUIRED_LOCALE_KEYS: &[&str] = &[
    "gui_title",
    "gui_subtitle",
    "gui_language",
    "gui_nav_build",
    "gui_nav_tools",
    "gui_page_build_title",
    "gui_page_build_desc",
    "gui_page_tools_title",
    "gui_page_tools_desc",
    "gui_section_source",
    "gui_mode_one",
    "gui_mode_folder",
    "gui_mode_online",
    "gui_mode_one_short",
    "gui_mode_folder_short",
    "gui_mode_online_short",
    "gui_installer_list",
    "gui_installer_empty_title",
    "gui_installer_empty_body",
    "gui_installer_path",
    "gui_installer_browse_other",
    "gui_installer_count",
    "gui_folder",
    "gui_folder_hint",
    "gui_folder_new",
    "gui_target",
    "gui_target_hint",
    "gui_target_refresh",
    "gui_url",
    "gui_url_hint",
    "gui_local_path",
    "gui_local_hint",
    "gui_section_more",
    "gui_section_more_hint",
    "gui_arch",
    "gui_label_packaging",
    "gui_archive",
    "gui_archive_hint",
    "gui_section_output",
    "gui_workdir",
    "gui_workdir_hint",
    "gui_artifact_title",
    "gui_artifact_release",
    "gui_artifact_assets",
    "gui_output_hint",
    "gui_open_workdir",
    "gui_open_release",
    "gui_open_assets",
    "gui_open",
    "gui_browse_file",
    "gui_browse_dir",
    "gui_section_flow",
    "gui_rail_hint",
    "gui_rail_steps",
    "gui_rail_pending",
    "gui_rail_running",
    "gui_start",
    "gui_cancel",
    "gui_after_build",
    "gui_elapsed",
    "gui_log",
    "gui_log_lines",
    "gui_log_autoscroll",
    "gui_log_empty",
    "gui_log_expand",
    "gui_log_collapse",
    "gui_log_clear",
    "gui_log_save",
    "gui_log_copy",
    "gui_log_saved",
    "gui_log_copied",
    "gui_status_idle",
    "gui_status_running",
    "gui_status_done",
    "gui_status_failed",
    "gui_status_cancelled",
    "gui_task",
    "gui_task_idle",
    "gui_task_step",
    "gui_toolbox_input",
    "gui_tool_path",
    "gui_tool_path_hint",
    "gui_tool_target",
    "gui_tool_arch",
    "gui_tool_json",
    "gui_token",
    "gui_token_hint",
    "gui_token_group",
    "gui_tools_commands",
    "gui_tools_commands_hint",
    "gui_run",
    "gui_tool_inspect",
    "gui_tool_inspect_hint",
    "gui_tool_research",
    "gui_tool_research_hint",
    "gui_tool_prepare",
    "gui_tool_prepare_hint",
    "gui_tool_resolve",
    "gui_tool_resolve_hint",
    "gui_tool_verify",
    "gui_tool_verify_hint",
    "gui_refresh",
    "gui_choose_installer",
    "gui_choose_folder",
    "gui_need_workdir",
    "gui_need_installer",
    "gui_need_folder",
    "gui_need_target",
    "gui_need_path",
    "gui_busy",
    "gui_confirm_cancel",
    "gui_confirm_exit",
    "gui_step_build_one",
    "gui_step_build_folder",
    "gui_step_prepare",
    "gui_step_build_online",
    "gui_step_archive",
    "gui_step_verify",
];

/// gui_core.selftest：检查这份安装的资源是否齐全（CI 对真实产物跑）。
pub fn selftest(report_path: Option<&Path>) -> i32 {
    let mut lines: Vec<String> = Vec::new();
    let mut healthy = true;

    fn check(lines: &mut Vec<String>, healthy: &mut bool, label: &str, ok: bool, detail: &str) {
        *healthy &= ok;
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!("  {detail}")
        };
        lines.push(format!(
            "[{}] {label}{suffix}",
            if ok { "OK" } else { "FAIL" }
        ));
    }

    let root = engine_root();
    check(
        &mut lines,
        &mut healthy,
        "engine",
        true,
        env!("CARGO_PKG_VERSION"),
    );
    check(
        &mut lines,
        &mut healthy,
        "engine root",
        root.is_dir(),
        &root.display().to_string(),
    );
    for relative in [
        "7zr.exe",
        "catalog/browser_catalog.json",
        "scripts/locales/wizard.json",
        "setdll/chrome++.ini",
        "setdll/chrome++.defaults.ini",
        "setdll/setdll-x64.exe",
        "setdll/version-x64.dll",
    ] {
        let item = root.join(relative);
        let detail = std::fs::metadata(&item)
            .map(|meta| format!("{} B", meta.len()))
            .unwrap_or_default();
        check(&mut lines, &mut healthy, relative, item.is_file(), &detail);
    }

    let targets = load_catalog_targets();
    check(
        &mut lines,
        &mut healthy,
        "catalog targets",
        targets.len() >= 5,
        &targets.join(", "),
    );

    let strings = crate::i18n::load_strings_default(&locales_path(), crate::i18n::DEFAULT_LANG);
    let missing: Vec<&str> = REQUIRED_LOCALE_KEYS
        .iter()
        .copied()
        .filter(|key| !strings.contains_key(*key))
        .collect();
    let locale_detail = if missing.is_empty() {
        format!("{} strings", strings.len())
    } else {
        format!("missing: {}", missing.join(", "))
    };
    check(
        &mut lines,
        &mut healthy,
        "locale keys",
        missing.is_empty(),
        &locale_detail,
    );

    let text = format!(
        "{}\nRESULT: {}\n",
        lines.join("\n"),
        if healthy { "OK" } else { "FAIL" }
    );
    if let Some(path) = report_path {
        if let Err(err) = std::fs::write(path, &text) {
            eprintln!("[WARN] 无法写自检报告: {err}");
        }
    }
    print!("{text}");
    if healthy {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gui_core.format_size 逐分支对照。
    #[test]
    fn format_size_matches_python() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(80 * 1024 * 1024), "80.0 MB");
        assert_eq!(format_size(300 * 1024 * 1024), "300.0 MB");
        assert_eq!(format_size(3 * 1024u64.pow(3)), "3.0 GB");
    }

    /// 真实 catalog 的顺序：ONLINE_TARGETS 优先，其余按名字。
    #[test]
    fn catalog_targets_follow_python_order() {
        let targets = load_catalog_targets();
        assert!(targets.len() >= 5, "catalog 解析失败: {targets:?}");
        let choices = target_choices();
        assert_eq!(choices.len(), targets.len());
        for (key, label) in &choices {
            assert!(!label.is_empty(), "display_name 为空: {key}");
        }
        let chrome = choices.iter().find(|(key, _)| key == "chrome_stable");
        assert_eq!(chrome.map(|(_, label)| label.as_str()), Some("Chrome++"));
    }

    /// 语言包必须覆盖自检清单（gui_core.required_locale_keys 的两条兜底之一）。
    #[test]
    fn locale_pack_covers_required_keys() {
        let strings = crate::i18n::load_strings_default(&locales_path(), "zh-CN");
        assert!(
            !strings.is_empty(),
            "读不到 language pack: {:?}",
            locales_path()
        );
        let missing: Vec<&str> = REQUIRED_LOCALE_KEYS
            .iter()
            .copied()
            .filter(|key| !strings.contains_key(*key))
            .collect();
        assert!(missing.is_empty(), "缺失键: {missing:?}");
        let en = crate::i18n::load_strings_default(&locales_path(), "en");
        let missing_en: Vec<&str> = REQUIRED_LOCALE_KEYS
            .iter()
            .copied()
            .filter(|key| !en.contains_key(*key))
            .collect();
        assert!(missing_en.is_empty(), "en 缺失键: {missing_en:?}");
    }
}
