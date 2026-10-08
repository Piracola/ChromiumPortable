//! ChromiumPortableBuilder GUI —— Tauri 2 外壳。
//!
//! 全部状态与逻辑在 portable_builder::gui（不依赖 tauri，可无头测试），
//! 这里只做两件事：把命令面接到 Controller（docs/MIGRATION_RUST_TAURI.md §6.2），
//! 以及跑那个 80ms 的事件泵（§6.3）。
//!
//! 命令名与参数形状由前端 app.js 尾部的「命令面契约」表定义，改一边要同步改另一边。
//! 启动冒烟判据沿用旧约定：窗口标题含 ChromiumPortable（test_gui_launch.py 的精神）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use portable_builder::gui::controller::{Controller, Emit};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

type Shared = Arc<Mutex<Controller>>;

/// 事件泵节拍：80ms 批量取一次（gui.py 的既有节拍，逐行推会把 IPC 打满）。
const PUMP_INTERVAL: Duration = Duration::from_millis(80);

fn lock<'a>(state: &'a State<'_, Shared>) -> MutexGuard<'a, Controller> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 所有命令的统一返回形状：{snapshot, strings}（前端拿它整页重渲染或就地更新）。
fn payload(controller: &Controller) -> Value {
    controller.snapshot_payload()
}

/// 打开文件/目录对话框，返回选中的路径。
fn ask_path(app: &tauri::AppHandle, kind: &str) -> Option<PathBuf> {
    match kind {
        "save_log" => app
            .dialog()
            .file()
            .set_file_name("ChromiumPortable-build.log")
            .blocking_save_file()
            .and_then(|path| path.into_path().ok()),
        "folder" | "workdir" | "tool_dir" | "new_folder" => app
            .dialog()
            .file()
            .blocking_pick_folder()
            .and_then(|path| path.into_path().ok()),
        _ => app
            .dialog()
            .file()
            .add_filter("Installer", &["exe", "msi", "7z", "zip", "rar", "cab"])
            .blocking_pick_file()
            .and_then(|path| path.into_path().ok()),
    }
}

// --------------------------------------------------------------------------
// 命令面（app.js 的 actionArgs / ACTION_COMMAND 表）
// --------------------------------------------------------------------------

#[tauri::command]
fn snapshot(state: State<'_, Shared>) -> Value {
    payload(&lock(&state))
}

#[tauri::command]
fn strings(state: State<'_, Shared>) -> Value {
    json!(lock(&state).strings())
}

#[tauri::command]
fn set_lang(state: State<'_, Shared>, lang: String) -> Value {
    let mut controller = lock(&state);
    controller.set_lang(&lang);
    payload(&controller)
}

/// 整份表单落库（前端传 {values: collect()}）；也接受单个 {key, value}。
#[tauri::command]
fn set_input(state: State<'_, Shared>, args: Value) -> Value {
    let mut controller = lock(&state);
    let form = args
        .get("values")
        .and_then(Value::as_object)
        .or_else(|| args.as_object())
        .cloned()
        .unwrap_or_else(Map::new);
    controller.refresh_plan(Some(&form));
    payload(&controller)
}

/// kind = "installers" / "targets"：重扫目录表与安装包目录后重算计划。
#[tauri::command]
fn refresh(state: State<'_, Shared>, kind: Option<String>) -> Value {
    let mut controller = lock(&state);
    let _ = kind;
    controller.refresh();
    controller.refresh_plan(None);
    payload(&controller)
}

#[tauri::command]
fn start(state: State<'_, Shared>) -> Value {
    let mut controller = lock(&state);
    controller.start_build();
    payload(&controller)
}

#[tauri::command]
fn run_tool(state: State<'_, Shared>, tool: String) -> Value {
    let mut controller = lock(&state);
    controller.run_tool(&tool);
    payload(&controller)
}

#[tauri::command]
fn cancel(state: State<'_, Shared>) -> Value {
    let mut controller = lock(&state);
    controller.cancel();
    payload(&controller)
}

#[tauri::command]
fn pick(app: tauri::AppHandle, state: State<'_, Shared>, kind: String) -> Value {
    let mut controller = lock(&state);
    let Some(path) = ask_path(&app, &kind) else {
        return payload(&controller);
    };
    let text = path.to_string_lossy().into_owned();
    match kind.as_str() {
        "new_folder" => {
            if let Err(err) = std::fs::create_dir_all(&path) {
                controller.set_banner(Some(&err.to_string()), "err", &text);
                return payload(&controller);
            }
            controller.set_input("folder", json!(text));
        }
        "folder" => {
            controller.set_input("folder", json!(text));
        }
        "workdir" => {
            controller.set_input("workdir", json!(text));
        }
        "tool_dir" | "tool_file" => {
            controller.set_input("tool_path", json!(text));
        }
        "local" => {
            controller.set_input("local_path", json!(text));
        }
        _ => {
            controller.set_input("installer", json!(text));
        }
    }
    payload(&controller)
}

#[tauri::command]
fn open_path(state: State<'_, Shared>, path: String) -> Value {
    let mut controller = lock(&state);
    match controller.open_target(&path) {
        Some(target) => {
            if let Err(err) = open_with_shell(&target) {
                return controller.open_failed(&path, &err.to_string());
            }
            controller.open_ok()
        }
        None => payload(&controller),
    }
}

#[tauri::command]
fn log_toggle(state: State<'_, Shared>) -> Value {
    let mut controller = lock(&state);
    controller.toggle_log();
    payload(&controller)
}

#[tauri::command]
fn log_autoscroll(state: State<'_, Shared>, value: bool) -> Value {
    let mut controller = lock(&state);
    controller.set_autoscroll(value);
    payload(&controller)
}

#[tauri::command]
fn log_clear(state: State<'_, Shared>) -> Value {
    let mut controller = lock(&state);
    controller.clear_log();
    payload(&controller)
}

#[tauri::command]
fn log_text(state: State<'_, Shared>) -> Value {
    json!({ "text": lock(&state).log_text() })
}

/// 保存日志：对话框在这里开，落盘在 Controller 里（与 Python 版同一分工）。
#[tauri::command]
fn log_save(app: tauri::AppHandle, state: State<'_, Shared>) -> Value {
    let mut controller = lock(&state);
    match ask_path(&app, "save_log") {
        Some(path) => {
            controller.save_log(&path);
        }
        None => return payload(&controller),
    }
    payload(&controller)
}

/// 剪贴板由前端 navigator.clipboard 负责（见 app.js），这里只给成功横幅。
#[tauri::command]
fn log_copy(state: State<'_, Shared>, ok: Option<bool>) -> Value {
    let mut controller = lock(&state);
    if ok.unwrap_or(true) {
        controller.log_copied_banner();
    } else {
        controller.set_banner(Some("clipboard unavailable"), "err", "");
    }
    payload(&controller)
}

/// 打开目录/文件：Windows 用 start，macOS 用 open，其余 xdg-open。
/// 刻意不引 opener 插件——这就是 Python 版 os.startfile 的一行等价物。
fn open_with_shell(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }
}

fn main() {
    // --selftest 在 tauri::Builder 之前处理：不建窗口，只查资源（§6.6）。
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--selftest") {
        let report = args
            .windows(2)
            .find(|pair| pair[0] == "--out")
            .map(|pair| PathBuf::from(&pair[1]));
        std::process::exit(portable_builder::gui::selftest(report.as_deref()));
    }

    let stop = Arc::new(AtomicBool::new(false));
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup({
            let stop = Arc::clone(&stop);
            move |app| {
                let handle = app.handle().clone();
                let emit: Emit = Arc::new(move |event: Value| {
                    let _ = handle.emit("builder-event", event);
                });
                let controller = Arc::new(Mutex::new(Controller::new(Some(emit))));
                let shared = Arc::clone(&controller);
                app.manage(shared);
                std::thread::spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        {
                            let mut guard = controller
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            guard.pump_once();
                        }
                        std::thread::sleep(PUMP_INTERVAL);
                    }
                });
                Ok(())
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            strings,
            set_lang,
            set_input,
            refresh,
            start,
            run_tool,
            cancel,
            pick,
            open_path,
            log_toggle,
            log_autoscroll,
            log_clear,
            log_text,
            log_save,
            log_copy
        ])
        .run(tauri::generate_context!())
        .expect("error while running ChromiumPortableBuilder");
}
