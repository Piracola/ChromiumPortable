//! ChromiumPortableBuilder —— 图形构建器命令面（2026-10 重写版）。
//!
//! 架构一句话：GUI 逻辑全部长在这个 crate 里，引擎 crate 保持纯 CLI。
//! 前端 `ui/`（无构建步骤的静态资产）通过 Tauri invoke 调这里的命令；
//! 所有引擎工作都以子进程方式调用同目录的 `portable-builder.exe` 完成，
//! 命令序列与手敲 CLI 完全一致（`--workdir` 在前，子命令在后）。
//! 子进程的 stdout/stderr 逐行进 mpsc，由泵线程每 80ms 批量 emit 一次
//! `app-event`（逐行推会把 IPC 打满，节流是刻意的）；同一批行里出现的
//! `KEY=value` 是引擎的跨步骤事实（CI 里由 GITHUB_ENV 落盘），这里现场捕获、
//! 喂给后续步骤——archive 拿不到 BUILT_VERSION 就会压出没有版本号的包名。
//!
//! 命令面（7 个）：
//! - `app_snapshot`  工作目录 + installers 清单 + 目录表 targets + 引擎在不在
//! - `app_workdir`   设置工作目录（必须已存在）并重新快照
//! - `app_plan`      按当前选择算出将要执行的引擎命令（纯预览，无副作用）
//! - `app_start`     启动执行链；同时只允许一个任务
//! - `app_cancel`    取消当前任务（杀引擎子进程）
//! - `app_open`      资源管理器打开 工作目录 / installers / build\release / build\assets
//! - `app_save_log`  原生另存为对话框 + 写日志文本
//!
//! 计划只有一个事实来源：`build_plan()` 同时服务预览与执行，
//! 前端展示的步骤与真正会跑的命令永远一致。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::{DialogExt, FilePath, MessageDialogButtons, MessageDialogKind};

const ENGINE_EXE: &str = "portable-builder.exe";
const EVENT: &str = "app-event";
const PUMP: Duration = Duration::from_millis(80);
const INSTALLER_EXTS: [&str; 6] = ["exe", "msi", "7z", "zip", "rar", "cab"];

// ---------------------------------------------------------------------------
// 数据形状（全部 camelCase，与前端字段一一对应）
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetEntry {
    id: String,
    label: String,
    product: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    workdir: String,
    engine_found: bool,
    has_installers_dir: bool,
    installers: Vec<String>,
    targets: Vec<TargetEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    /// 前端文案表的键（step_*），标题由前端按语言渲染。
    title_key: String,
    /// 完整参数（含 --workdir），既是执行依据也是前端展示的透明命令行。
    args: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Plan {
    ok: bool,
    error_key: Option<String>,
    steps: Vec<Step>,
}

impl Plan {
    fn steps(steps: Vec<Step>) -> Self {
        Plan {
            ok: true,
            error_key: None,
            steps,
        }
    }
    fn error(key: &'static str) -> Self {
        Plan {
            ok: false,
            error_key: Some(key.to_string()),
            steps: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PlanRequest {
    mode: String,
    tool: Option<String>,
    arch: String,
    archive: bool,
    installer: String,
    folder: String,
    target: String,
    url: String,
    local_path: String,
    json_output: bool,
    no_smoke: bool,
}

impl Default for PlanRequest {
    fn default() -> Self {
        PlanRequest {
            mode: "one".into(),
            tool: None,
            arch: "x64".into(),
            archive: false,
            installer: String::new(),
            folder: String::new(),
            target: String::new(),
            url: String::new(),
            local_path: String::new(),
            json_output: false,
            no_smoke: true,
        }
    }
}

// ---------------------------------------------------------------------------
// 共享状态
// ---------------------------------------------------------------------------

/// 一次运行中的任务句柄：cancel 标志 + 可被杀的当前子进程。
struct JobHandle {
    cancel: AtomicBool,
    child: Mutex<Option<Child>>,
}

impl JobHandle {
    fn new() -> Self {
        JobHandle {
            cancel: AtomicBool::new(false),
            child: Mutex::new(None),
        }
    }
}

struct Shared {
    workdir: Mutex<PathBuf>,
    job: Mutex<Option<Arc<JobHandle>>>,
}

impl Shared {
    fn new() -> Self {
        let base = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        Shared {
            workdir: Mutex::new(base),
            job: Mutex::new(None),
        }
    }
    fn workdir(&self) -> PathBuf {
        self.workdir.lock().unwrap().clone()
    }
    fn set_workdir(&self, path: PathBuf) {
        *self.workdir.lock().unwrap() = path;
    }
    /// 置取消标志并杀当前子进程；返回之前是否有任务。
    fn cancel_job(&self) -> bool {
        let handle = {
            let mut slot = self.job.lock().unwrap();
            slot.take()
        };
        match handle {
            Some(h) => {
                h.cancel.store(true, Ordering::SeqCst);
                if let Ok(mut guard) = h.child.lock() {
                    if let Some(child) = guard.as_mut() {
                        let _ = child.kill();
                    }
                }
                true
            }
            None => false,
        }
    }
    fn job_running(&self) -> bool {
        self.job.lock().unwrap().is_some()
    }
}

// ---------------------------------------------------------------------------
// 引擎与目录
// ---------------------------------------------------------------------------

/// 引擎可执行文件：优先 builder-app.exe 同目录，退回 PATH 上的裸名。
fn engine_exe() -> PathBuf {
    if let Ok(dir) = std::env::current_exe() {
        let candidate = dir.parent().unwrap_or(Path::new(".")).join(ENGINE_EXE);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(ENGINE_EXE)
}

fn engine_found() -> bool {
    match engine_exe() {
        p if p.is_absolute() => p.is_file(),
        // 裸名：交给 PATH。Windows 下不做昂贵探测，直接假定可用；
        // spawn 失败会以 err 行进日志，用户能看到。
        _ => true,
    }
}

fn catalog_path(workdir: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    // 开发态 exe 在 target\debug\ 下，catalog 在仓库根：沿父目录向上找几层
    // （与引擎的资源探测同一思路）；发布态 exe 旁就有 catalog，第一跳即中。
    if let Ok(dir) = std::env::current_exe() {
        let mut dir = dir.parent().map(Path::to_path_buf);
        for _ in 0..6 {
            let Some(d) = dir else { break };
            candidates.push(d.join("catalog"));
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    candidates.push(workdir.join("catalog"));
    candidates
        .into_iter()
        .map(|d| d.join("browser_catalog.json"))
        .find(|p| p.is_file())
}

fn load_targets(workdir: &Path) -> Vec<TargetEntry> {
    let Some(path) = catalog_path(workdir) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(map) = value.get("targets").and_then(|t| t.as_object()) {
        for (id, entry) in map {
            let label = entry
                .get("display_name")
                .or_else(|| entry.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or(id)
                .to_string();
            let product = entry
                .get("product")
                .and_then(|p| p.as_bool())
                .unwrap_or(false);
            out.push(TargetEntry {
                id: id.clone(),
                label,
                product,
            });
        }
    }
    // 产品线在前，其余按 label 排序。
    out.sort_by(|a, b| {
        b.product
            .cmp(&a.product)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    out
}

fn list_installers(workdir: &Path) -> (bool, Vec<String>) {
    let dir = workdir.join("installers");
    let mut names = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return (false, names);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if matches!(ext.as_deref(), Some(e) if INSTALLER_EXTS.contains(&e)) {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    (true, names)
}

fn snapshot(workdir: &Path) -> Snapshot {
    let (has_installers_dir, installers) = list_installers(workdir);
    Snapshot {
        workdir: workdir.to_string_lossy().into_owned(),
        engine_found: engine_found(),
        has_installers_dir,
        installers,
        targets: load_targets(workdir),
    }
}

/// 相对路径一律相对工作目录（与旧 gui_plan.resolve_input 一致）。
fn resolve_input(workdir: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workdir.join(path)
    }
}

/// 展示用：给含空格的参数补引号。
fn quote_arg(arg: &str) -> String {
    if arg.is_empty() || arg.chars().any(|c| c.is_whitespace()) {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    }
}

// ---------------------------------------------------------------------------
// 计划：与 CLI 契约一字不差（旧 gui/plan.rs 的行为基线）
// ---------------------------------------------------------------------------

/// 仓库根（catalog 的上一级）。引擎的资源探测按 §6.5 分五级，但开发态
/// exe 在 `target\debug`、catalog 在仓库根，它自己猜不到；GUI 已经解析出来了，
/// 就显式交给它，等价于一级探测（`--builder-dir`）。
fn builder_dir(workdir: &Path) -> Option<PathBuf> {
    let catalog = catalog_path(workdir)?;
    Some(catalog.parent()?.parent()?.to_path_buf())
}

fn engine_step(title_key: &str, workdir: &Path, rest: &[&str]) -> Step {
    let mut args = vec![
        "--workdir".to_string(),
        workdir.to_string_lossy().into_owned(),
    ];
    if let Some(dir) = builder_dir(workdir) {
        args.push("--builder-dir".to_string());
        args.push(dir.to_string_lossy().into_owned());
    }
    args.extend(rest.iter().map(|s| s.to_string()));
    Step {
        title_key: title_key.to_string(),
        args,
    }
}

fn build_plan(shared: &Shared, req: &PlanRequest) -> Result<Plan, &'static str> {
    if !engine_found() {
        return Err("engine_missing");
    }
    let arch = if req.arch.trim().is_empty() {
        "x64"
    } else {
        req.arch.trim()
    };
    let workdir = shared.workdir();

    if let Some(tool) = req.tool.as_deref() {
        return plan_tool(tool, req, &workdir, arch);
    }

    match req.mode.as_str() {
        "one" => {
            if req.installer.trim().is_empty() {
                return Err("need_installer");
            }
            let path = resolve_input(&workdir, req.installer.trim());
            if !path.exists() {
                return Err("need_installer");
            }
            let mut rest = vec![
                "build-package".to_string(),
                path.to_string_lossy().into_owned(),
                "--architecture".to_string(),
                arch.to_string(),
            ];
            if req.archive {
                rest.push("--archive".to_string());
            }
            Ok(Plan::steps(vec![engine_step(
                "step_build_one",
                &workdir,
                &rest.iter().map(String::as_str).collect::<Vec<_>>(),
            )]))
        }
        "folder" => {
            if req.folder.trim().is_empty() {
                return Err("need_folder");
            }
            let dir = resolve_input(&workdir, req.folder.trim());
            if !dir.is_dir() {
                return Err("need_folder");
            }
            let mut rest = vec![
                "build-packages".to_string(),
                dir.to_string_lossy().into_owned(),
                "--architecture".to_string(),
                arch.to_string(),
            ];
            if req.archive {
                rest.push("--archive".to_string());
            }
            Ok(Plan::steps(vec![engine_step(
                "step_build_folder",
                &workdir,
                &rest.iter().map(String::as_str).collect::<Vec<_>>(),
            )]))
        }
        _ => {
            if req.target.trim().is_empty() {
                return Err("need_target");
            }
            let target = req.target.trim();
            let config = workdir.join("build").join("selected.browser.json");
            let config_text = config.to_string_lossy().into_owned();

            let mut prepare = vec![
                "prepare-target".to_string(),
                "--browser".to_string(),
                target.to_string(),
                "--output".to_string(),
                config_text.clone(),
                "--architecture".to_string(),
                arch.to_string(),
            ];
            let url = req.url.trim();
            if !url.is_empty() {
                prepare.push("--url".to_string());
                prepare.push(url.to_string());
            }
            let local = req.local_path.trim();
            if !local.is_empty() {
                prepare.push("--path".to_string());
                prepare.push(local.to_string());
            }

            let mut steps = vec![
                engine_step(
                    "step_prepare",
                    &workdir,
                    &prepare.iter().map(String::as_str).collect::<Vec<_>>(),
                ),
                engine_step(
                    "step_build_online",
                    &workdir,
                    &["--config", &config_text, "--target", target, "build"],
                ),
            ];
            if req.archive {
                steps.push(engine_step(
                    "step_archive",
                    &workdir,
                    &["--config", &config_text, "--target", target, "archive"],
                ));
                steps.push(engine_step(
                    "step_verify",
                    &workdir,
                    &[
                        "--config",
                        &config_text,
                        "--target",
                        target,
                        "verify",
                        "--no-smoke",
                    ],
                ));
            }
            Ok(Plan::steps(steps))
        }
    }
}

fn plan_tool(
    tool: &str,
    req: &PlanRequest,
    workdir: &Path,
    arch: &str,
) -> Result<Plan, &'static str> {
    let path_raw = req.installer.trim();
    let path = if path_raw.is_empty() {
        String::new()
    } else {
        resolve_input(workdir, path_raw)
            .to_string_lossy()
            .into_owned()
    };
    let config = workdir.join("build").join("selected.browser.json");
    let config_text = config.to_string_lossy().into_owned();

    let step = match tool {
        "inspect" => {
            if path.is_empty() {
                return Err("need_path");
            }
            let mut rest = vec![
                "inspect-package".to_string(),
                path.clone(),
                "--architecture".to_string(),
                arch.to_string(),
            ];
            if req.json_output {
                rest.push("--json".to_string());
            }
            engine_step(
                "step_tool_inspect",
                workdir,
                &rest.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        }
        "research" => {
            if path.is_empty() {
                return Err("need_path");
            }
            let mut rest = vec![
                "research-packages".to_string(),
                path.clone(),
                "--architecture".to_string(),
                arch.to_string(),
            ];
            if req.json_output {
                rest.push("--json".to_string());
            }
            engine_step(
                "step_tool_research",
                workdir,
                &rest.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        }
        "prepare" => {
            if req.target.trim().is_empty() {
                return Err("need_target");
            }
            let mut rest = vec![
                "prepare-target".to_string(),
                "--browser".to_string(),
                req.target.trim().to_string(),
                "--output".to_string(),
                config_text.clone(),
                "--architecture".to_string(),
                arch.to_string(),
            ];
            if !path.is_empty() {
                if Path::new(&path).is_dir() {
                    return Err("tool_prepare_dir");
                }
                rest.push("--path".to_string());
                rest.push(path);
            }
            engine_step(
                "step_tool_prepare",
                workdir,
                &rest.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        }
        "resolve" => {
            if req.target.trim().is_empty() {
                return Err("need_target");
            }
            engine_step(
                "step_tool_resolve",
                workdir,
                &["resolve-upstream", "--browser", req.target.trim(), "--json"],
            )
        }
        "verify" => {
            if req.target.trim().is_empty() {
                return Err("need_target");
            }
            let mut rest = vec![
                "--config".to_string(),
                config_text,
                "--target".to_string(),
                req.target.trim().to_string(),
                "verify".to_string(),
            ];
            if req.no_smoke {
                rest.push("--no-smoke".to_string());
            }
            engine_step(
                "step_tool_verify",
                workdir,
                &rest.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        }
        _ => return Err("need_path"),
    };
    Ok(Plan::steps(vec![step]))
}

// ---------------------------------------------------------------------------
// 执行链：spawn → 读管道 → 泵线程 80ms 批量 emit
// ---------------------------------------------------------------------------

fn spawn_reader<R: std::io::Read + Send + 'static>(stream: Option<R>, tx: mpsc::Sender<String>) {
    let Some(stream) = stream else { return };
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    while matches!(buf.last(), Some(b'\n' | b'\r')) {
                        buf.pop();
                    }
                    if tx.send(String::from_utf8_lossy(&buf).into_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

/// 跨步骤事实（`KEY=value`）。CI 里 GITHUB_ENV 负责落盘并在下一步自动继承；
/// 本地子进程模式下没有 GITHUB_ENV，引擎就把它们打到 stdout，
/// 承接这份契约的是调用方——也就是这里。
type Facts = Arc<Mutex<HashMap<String, String>>>;

fn collect_facts(line: &str, facts: &mut HashMap<String, String>) {
    let Some((key, value)) = line.split_once('=') else {
        return;
    };
    // 只认环境变量样子的键：大写字母开头，其余是大写字母/数字/下划线。
    // 日志行（`[INFO] ...`）和 7-Zip 的输出都过不了这一关。
    let mut bytes = key.bytes();
    let starts_upper = bytes.next().is_some_and(|b| b.is_ascii_uppercase());
    let rest_env_like = bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    if starts_upper && rest_env_like {
        facts.insert(key.to_string(), value.to_string());
    }
}

fn pump_lines(app: AppHandle, rx: mpsc::Receiver<String>, facts: Facts) {
    let mut batch: Vec<String> = Vec::new();
    loop {
        match rx.recv_timeout(PUMP) {
            Ok(line) => {
                collect_facts(&line, &mut facts.lock().unwrap());
                batch.push(line);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    let _ = app.emit(EVENT, json!({ "type": "line", "lines": batch }));
                    batch = Vec::new();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !batch.is_empty() {
                    let _ = app.emit(EVENT, json!({ "type": "line", "lines": batch }));
                }
                break;
            }
        }
    }
}

/// 跑单步：返回退出码。cancel 通过杀子进程让 wait 封闭。
fn run_step(
    handle: &JobHandle,
    step: &Step,
    workdir: &Path,
    app: &AppHandle,
    facts: &Facts,
) -> i32 {
    let mut cmd = Command::new(engine_exe());
    cmd.args(&step.args).current_dir(workdir);
    {
        // 前面步骤攒下的事实在这一步生效（等价于 CI 里 GITHUB_ENV 的继承）。
        let facts = facts.lock().unwrap();
        for (key, value) in facts.iter() {
            cmd.env(key, value);
        }
    }
    let child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = app.emit(
                EVENT,
                json!({ "type": "line", "lines": [format!("无法启动引擎：{e}")], "err": true }),
            );
            return -1;
        }
    };
    let (tx, rx) = mpsc::channel::<String>();
    spawn_reader(child.stdout.take(), tx.clone());
    spawn_reader(child.stderr.take(), tx.clone());
    drop(tx);

    *handle.child.lock().unwrap() = Some(child);
    let pump = {
        let app = app.clone();
        let facts = Arc::clone(facts);
        thread::spawn(move || pump_lines(app, rx, facts))
    };

    let code = loop {
        thread::sleep(Duration::from_millis(120));
        let mut slot = handle.child.lock().unwrap();
        match slot.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    slot.take();
                    break status.code().unwrap_or(-1);
                }
                Ok(None) => {}
                Err(_) => {
                    slot.take();
                    break -1;
                }
            },
            None => break -1,
        }
    };
    let _ = pump.join();
    code
}

fn run_plan(
    app: AppHandle,
    shared: Arc<Shared>,
    handle: Arc<JobHandle>,
    plan: Plan,
    workdir: PathBuf,
) {
    let total = plan.steps.len();
    let started = Instant::now();
    let mut completed = 0usize;
    let mut last_code = 0i32;
    let mut failed = false;
    // 事实按任务清空：上一次构建的版本号绝不允许漏进这一次。
    let facts: Facts = Arc::new(Mutex::new(HashMap::new()));

    for (index, step) in plan.steps.iter().enumerate() {
        if handle.cancel.load(Ordering::SeqCst) {
            break;
        }
        let _ = app.emit(
            EVENT,
            json!({ "type": "stepStart", "index": index, "total": total, "titleKey": step.title_key }),
        );
        let display: String = step
            .args
            .iter()
            .map(|a| quote_arg(a))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = app.emit(
            EVENT,
            json!({ "type": "line", "lines": [format!("portable-builder {display}")], "cmd": true }),
        );
        let code = run_step(&handle, step, &workdir, &app, &facts);
        let _ = app.emit(
            EVENT,
            json!({ "type": "stepEnd", "index": index, "code": code }),
        );
        completed += 1;
        last_code = code;
        if code != 0 {
            failed = true;
            break;
        }
    }

    let cancelled = handle.cancel.load(Ordering::SeqCst) && (failed || completed < total);
    let elapsed = started.elapsed().as_secs();
    let _ = app.emit(
        EVENT,
        json!({
            "type": "done",
            "code": last_code,
            "failed": failed,
            "cancelled": cancelled,
            "seconds": elapsed,
        }),
    );
    let _ = shared.job.lock().unwrap().take();
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

#[tauri::command]
fn app_snapshot(shared: State<'_, Arc<Shared>>) -> Snapshot {
    let workdir = shared.workdir();
    snapshot(&workdir)
}

#[tauri::command]
fn app_workdir(shared: State<'_, Arc<Shared>>, path: String) -> Result<Snapshot, String> {
    let path = PathBuf::from(path.trim());
    if !path.is_dir() {
        return Err("workdir_invalid".to_string());
    }
    shared.set_workdir(path);
    let workdir = shared.workdir();
    Ok(snapshot(&workdir))
}

#[tauri::command]
fn app_plan(shared: State<'_, Arc<Shared>>, req: PlanRequest) -> Plan {
    build_plan(&shared, &req).unwrap_or_else(Plan::error)
}

#[tauri::command]
fn app_start(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
    req: PlanRequest,
) -> Result<(), String> {
    let plan = build_plan(&shared, &req).map_err(str::to_string)?;
    {
        let mut slot = shared.job.lock().unwrap();
        if slot.is_some() {
            return Err("busy".to_string());
        }
        *slot = Some(Arc::new(JobHandle::new()));
    }
    let handle = shared.job.lock().unwrap().as_ref().map(Arc::clone).unwrap();
    let workdir = shared.workdir();
    let shared_for_thread = Arc::clone(&shared);
    thread::spawn(move || run_plan(app, shared_for_thread, handle, plan, workdir));
    Ok(())
}

#[tauri::command]
fn app_cancel(shared: State<'_, Arc<Shared>>) -> bool {
    shared.cancel_job()
}

#[tauri::command]
fn app_open(shared: State<'_, Arc<Shared>>, kind: String) -> Result<(), String> {
    let workdir = shared.workdir();
    let target = match kind.as_str() {
        "workdir" => Some(workdir.clone()),
        // installers 目录允许顺手创建（旧 GUI 的「新建」按钮）。
        "installers" => {
            let dir = workdir.join("installers");
            let _ = fs::create_dir_all(&dir);
            Some(dir)
        }
        "release" => {
            let dir = workdir.join("build").join("release");
            Some(if dir.is_dir() { dir } else { workdir.clone() })
        }
        "assets" => {
            let dir = workdir.join("build").join("assets");
            Some(if dir.is_dir() { dir } else { workdir.clone() })
        }
        _ => None,
    };
    let Some(target) = target else {
        return Err("unknown_kind".to_string());
    };
    Command::new("explorer")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn app_save_log(app: AppHandle, text: String) -> Result<Option<String>, String> {
    let stamp = chrono_like_now();
    let picked = app
        .dialog()
        .file()
        .add_filter("文本文件", &["txt"])
        .set_file_name(format!("portable-log-{stamp}.txt"))
        .blocking_save_file();
    match picked {
        Some(FilePath::Path(path)) => {
            fs::write(&path, &text).map_err(|e| e.to_string())?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
        Some(FilePath::Url(url)) => Ok(Some(url.to_string())),
        None => Ok(None),
    }
}

/// 不引 chrono：本地时间的 YYYYMMDD-HHMMSS 用标准库足够。
/// std 没有本地时区，用 UTC 秒拼文件名——精确到秒防冲突即可。
fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // 民用日期换算（Howard Hinnant 算法，公历）。
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{y:04}{mth:02}{d:02}-{h:02}{m:02}{s:02}")
}

// ---------------------------------------------------------------------------
// 装配
// ---------------------------------------------------------------------------

fn main() {
    let shared = Arc::new(Shared::new());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(Arc::clone(&shared))
        .invoke_handler(tauri::generate_handler![
            app_snapshot,
            app_workdir,
            app_plan,
            app_start,
            app_cancel,
            app_open,
            app_save_log
        ])
        .setup(move |app| {
            // 任务运行时关窗：拦下 → 原生确认 → 确认则杀任务再关。
            let handle = app.handle().clone();
            let shared_for_close = Arc::clone(&shared);
            if let Some(win) = app.get_webview_window("main") {
                let close_win = win.clone();
                let handler_win = win.clone();
                handler_win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        if shared_for_close.job_running() {
                            api.prevent_close();
                            let handle = handle.clone();
                            let shared = Arc::clone(&shared_for_close);
                            let win = close_win.clone();
                            handle
                                .dialog()
                                .message(crate_close_message())
                                .title("ChromiumPortableBuilder")
                                .kind(MessageDialogKind::Warning)
                                .buttons(MessageDialogButtons::OkCancelCustom(
                                    "退出".into(),
                                    "继续".into(),
                                ))
                                .show(move |quit| {
                                    if quit {
                                        shared.cancel_job();
                                        let _ = win.close();
                                    }
                                });
                        }
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running ChromiumPortableBuilder");
}

fn crate_close_message() -> String {
    // 文案在前端字典之外：原生对话框不经过 WebView，直接双语并写。
    "任务还在运行，确定退出吗？\nA task is still running. Quit anyway?".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_come_from_env_lines_only() {
        let mut facts = HashMap::new();
        collect_facts("BUILT_VERSION=155.0.8059.40", &mut facts);
        collect_facts("OUTPUT_DIR=Chrome", &mut facts);
        // 日志行、7-Zip 输出、裸值行都不算事实。
        collect_facts(
            "[INFO] Applied 4 override(s): open_url_new_tab=1",
            &mut facts,
        );
        collect_facts(
            "7-Zip 26.03 (x64) : Copyright (c) 1999-2026 Igor Pavlov",
            &mut facts,
        );
        collect_facts("chrome_stable", &mut facts);
        collect_facts("Files read from disk: 269", &mut facts);

        assert_eq!(
            facts.get("BUILT_VERSION").map(String::as_str),
            Some("155.0.8059.40")
        );
        assert_eq!(facts.get("OUTPUT_DIR").map(String::as_str), Some("Chrome"));
        assert_eq!(facts.len(), 2);
    }
}
