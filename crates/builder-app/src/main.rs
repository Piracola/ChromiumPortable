//! ChromiumPortableBuilder GUI —— Tauri 2 外壳。
//!
//! 全部状态与逻辑在 portable_builder::gui（不依赖 tauri，可无头测试），
//! 这里只做两件事：把命令面接到 Controller（docs/MIGRATION_RUST_TAURI.md §6.2），
//! 以及跑那个 80ms 的事件泵（§6.3）。
//!
//! 启动冒烟判据沿用旧约定：窗口标题含 ChromiumPortable（test_gui_launch.py 的精神）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use portable_builder::gui::controller::{Controller, Emit};
use serde_json::{json, Value};
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
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 所有命令的统一返回形状：{snapshot, strings}（前端拿它整页重渲染）。
fn payload(controller: &Controller) -> Value {
    controller.snapshot_payload()
}

// --------------------------------------------------------------------------
// 命令面（§6.2 映射表）
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

#[tauri::command]
fn set_input(state: State<'_, Shared>, key: String, value: Value) -> Value {
    let mut controller = lock(&state);
    controller.set_input(&key, value);
    payload(&controller)
}

/// 两种用法都收：带 values（整份表单）就重算计划，不带就重扫目录表与安装包。
#[tauri::command]
fn refresh(state: State<'_, Shared>, values: Option<Value>) -> Value {
    let mut controller = lock(&state);
    match values.as_ref().and_then(Value::as_object) {
        Some(form) => {
            controller.refresh_plan(Some(form));
        }
        None => {
            controller.refresh();
            controller.refresh_plan(None);
        }
    }
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

/// 文件/目录对话框：替换 Python 版「问一下、JS 回调 done」的模式（§6.2）。
#[tauri::command]
fn pick(app: tauri::AppHandle, state: State<'_, Shared>, kind: String) -> Value {
    let chosen: Option<PathBuf> = match kind.as_str() {
        "save_log" => app
            .dialog()
            .file()
            .set_file_name("ChromiumPortable-build.log")
            .blocking_save_file()
            .and_then(|path| path.into_path().ok()),
        "folder" | "workdir" | "tool_dir" => app
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
    };

    let mut controller = lock(&state);
    let Some(path) = chosen else {
        return payload(&controller);
    };
    let text = path.to_string_lossy().into_owned();
    match kind.as_str() {
        "save_log" => {
            controller.save_log(&path);
        }
        "folder" => {
            controller.set_input("folder", json!(text));
        }
        "workdir" => {
            controller.set_input("workdir", json!(text));
        }
        "tool_dir" => {
            controller.set_input("tool_path", json!(text));
        }
        "local" => {
            controller.set_input("local_path", json!(text));
        }
        "tool_file" => {
            controller.set_input("tool_path", json!(text));
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
fn log(state: State<'_, Shared>, action: String, value: Option<Value>) -> Value {
    let mut controller = lock(&state);
    match action.as_str() {
        "clear" => {
            controller.clear_log();
        }
        "toggle" => {
            controller.toggle_log();
        }
        "autoscroll" => {
            controller.set_autoscroll(value.as_ref().and_then(Value::as_bool).unwrap_or(true));
        }
        "text" => return json!({"text": controller.log_text()}),
        _ => {}
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
            log
        ])
        .run(tauri::generate_context!())
        .expect("error while running ChromiumPortableBuilder");
}
