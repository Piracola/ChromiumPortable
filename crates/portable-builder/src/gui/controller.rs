//! 全部状态与逻辑的持有者（scripts/gui.py 的 Controller）。
//! 与界面框架无关：命令层（builder-app）只做转发 + 对话框/剪贴板这类系统能力。

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use super::plan::{plan_build, plan_tool, BuildRequest, Plan};
use super::runner::{JobEvent, JobRunner, CANCELLED};

/// 事件出口：builder-app 侧包一层 app.emit("builder-event", payload)。
pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;

const JS_TO_VAR: [(&str, &str); 1] = [("local_path", "local")];
const BOOL_VARS: [&str; 3] = ["archive", "tool_json", "autoscroll"];
/// 改了这些输入就要重算计划（gui.py 里走 _touch 的那些）。
const PLAN_VARS: [&str; 9] = [
    "mode",
    "installer",
    "folder",
    "target",
    "arch",
    "archive",
    "url",
    "local",
    "workdir",
];

/// 内存里最多留多少行日志：跑几小时的构建不该把内存吃光，最近几千行够回看。
const LOG_LIMIT: usize = 4000;

const README_TEXT: &str = "把要构建的浏览器安装包放在这里。\n\
Put installer packages for portable builds in this folder.\n\
Supported: .exe .msi .7z .zip .rar .cab\n";

pub struct Controller {
    lang: String,
    strings: BTreeMap<String, String>,
    vars: Map<String, Value>,
    log_lines: Vec<String>,
    banner_text: String,
    banner_tone: String,
    banner_detail: String,
    status_text: String,
    status_tone: String,
    result_text: String,
    task_title: String,
    task_meta: String,
    elapsed_text: String,
    log_open: bool,
    cancel_armed: bool,
    started_at: Option<Instant>,
    plan: Plan,
    plan_titles: Vec<String>,
    runner: JobRunner,
    emit: Option<Emit>,
    tick_counter: u64,
}

impl Controller {
    pub fn new(emit: Option<Emit>) -> Controller {
        Self::build(emit, None)
    }

    /// 测试用：工作目录显式给，别去碰真实 cwd。
    pub fn with_workdir(emit: Option<Emit>, workdir: PathBuf) -> Controller {
        Self::build(emit, Some(workdir))
    }

    fn build(emit: Option<Emit>, workdir: Option<PathBuf>) -> Controller {
        let lang = crate::i18n::detect_lang().to_string();
        let strings = crate::i18n::load_strings_default(&super::locales_path(), &lang);
        let workdir = workdir.unwrap_or_else(super::default_workdir);

        let mut vars = Map::new();
        vars.insert("mode".to_string(), json!("one"));
        vars.insert("installer".to_string(), json!(""));
        vars.insert("folder".to_string(), json!(""));
        vars.insert("target".to_string(), json!("chrome_stable"));
        vars.insert("arch".to_string(), json!("x64"));
        vars.insert("archive".to_string(), json!(true));
        vars.insert("url".to_string(), json!(""));
        vars.insert("local".to_string(), json!(""));
        vars.insert("token".to_string(), json!(""));
        vars.insert("tool_path".to_string(), json!(""));
        vars.insert("tool_target".to_string(), json!("chrome_stable"));
        vars.insert("tool_arch".to_string(), json!("x64"));
        vars.insert("tool_json".to_string(), json!(false));
        vars.insert("autoscroll".to_string(), json!(true));
        vars.insert("workdir".to_string(), json!(workdir.to_string_lossy()));

        let mut controller = Controller {
            lang: lang.clone(),
            strings,
            vars,
            log_lines: Vec::new(),
            banner_text: String::new(),
            banner_tone: "info".to_string(),
            banner_detail: String::new(),
            status_text: String::new(),
            status_tone: "idle".to_string(),
            result_text: String::new(),
            task_title: String::new(),
            task_meta: String::new(),
            elapsed_text: String::new(),
            log_open: false,
            cancel_armed: false,
            started_at: None,
            plan: Plan::default(),
            plan_titles: Vec::new(),
            runner: JobRunner::new(&lang),
            emit,
            tick_counter: 0,
        };
        controller.prepare_workspace(None);
        controller
    }

    // ---------------- 语言 ----------------

    pub fn lang(&self) -> &str {
        &self.lang
    }

    pub fn strings(&self) -> &BTreeMap<String, String> {
        &self.strings
    }

    pub fn t(&self, key: &str) -> String {
        crate::i18n::translate(&self.strings, key, &[])
    }

    pub fn tf(&self, key: &str, fields: &[(&str, &str)]) -> String {
        crate::i18n::translate(&self.strings, key, fields)
    }

    /// gui.py:set_lang —— 只换字符串，然后推一次全量状态。
    pub fn set_lang(&mut self, lang: &str) -> Value {
        let lang = if lang == "en" { "en" } else { "zh-CN" };
        self.lang = lang.to_string();
        self.strings = crate::i18n::load_strings_default(&super::locales_path(), lang);
        self.runner.lang = lang.to_string();
        self.status_text = self.t("gui_status_idle");
        if self.log_lines.is_empty() {
            self.log_lines = vec![format!("{}\n", self.t("gui_log_empty"))];
        }
        self.state()
    }

    // ---------------- 状态 → 视图 ----------------

    /// gui.py:state() —— 前端渲染的唯一切口，字段名即契约（app.js 尾部注释）。
    pub fn state(&self) -> Value {
        let choices: Vec<Value> = super::target_choices()
            .into_iter()
            .map(|(id, label)| json!([id, label]))
            .collect();
        let raw_steps: Vec<Value> = self
            .plan
            .steps
            .iter()
            .map(|step| json!({"title": step.title, "cmd": step.cmd}))
            .collect();
        let artifact = self.artifact_paths();
        json!({
            "lang": self.lang,
            "strings": self.strings,
            "vars": Value::Object(self.vars.clone()),
            "catalog_choices": choices,
            "architectures": super::ARCHITECTURES,
            "installer_choices": self.installer_choices(),
            "plan": {
                "steps": self.plan_titles,
                "raw_steps": raw_steps,
                "ok": self.plan.ok(),
                "error_key": self.plan.error_key,
                "error_detail": self.plan.error_detail,
            },
            "banner": {"text": self.banner_text, "tone": self.banner_tone, "detail": self.banner_detail},
            "status": {"text": self.status_text, "tone": self.status_tone},
            "result": self.result_text,
            "task": {"title": self.task_title, "meta": self.task_meta},
            "elapsed": self.elapsed_text,
            "log": {"lines": self.log_lines, "open": self.log_open},
            "running": self.runner.running(),
            "cancel_armed": self.cancel_armed,
            "artifact": {"release": artifact.0, "assets": artifact.1},
        })
    }

    /// invoke("snapshot") 的返回形状：{snapshot, strings}。
    pub fn snapshot_payload(&self) -> Value {
        json!({"snapshot": self.state(), "strings": self.strings})
    }

    pub fn workdir(&self) -> PathBuf {
        let raw = self.vars["workdir"].as_str().unwrap_or("").trim().to_string();
        if raw.is_empty() {
            super::default_workdir()
        } else {
            expand_user(&raw)
        }
    }

    pub fn artifact_paths(&self) -> (String, String) {
        let base = self.workdir().join("build");
        (
            base.join("release").to_string_lossy().into_owned(),
            base.join("assets").to_string_lossy().into_owned(),
        )
    }

    /// gui.py:installer_choices —— 扫 installers\ 下能构建的包并附体积。
    pub fn installer_choices(&self) -> Value {
        let folder = match self.vars["folder"].as_str() {
            Some(raw) if !raw.is_empty() => PathBuf::from(raw),
            _ => self.workdir().join(super::INSTALLER_DIR_NAME),
        };
        if !folder.is_dir() {
            return json!([]);
        }
        let mut entries: Vec<PathBuf> = match std::fs::read_dir(&folder) {
            Ok(reader) => reader.filter_map(|item| item.ok().map(|e| e.path())).collect(),
            Err(_) => return json!([]),
        };
        entries.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });
        let mut found = Vec::new();
        for item in entries {
            if !item.is_file() {
                continue;
            }
            let suffix = item
                .extension()
                .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()))
                .unwrap_or_default();
            if !super::INSTALLER_SUFFIXES.contains(&suffix.as_str()) {
                continue;
            }
            let Ok(meta) = item.metadata() else { continue };
            found.push(json!({
                "name": item.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                "path": item.to_string_lossy(),
                "size": meta.len(),
                "size_text": super::format_size(meta.len()),
            }));
        }
        Value::Array(found)
    }

    pub fn current_request(&self) -> BuildRequest {
        let text = |key: &str| self.vars[key].as_str().unwrap_or("").to_string();
        BuildRequest {
            workdir: self.workdir(),
            mode: text("mode"),
            installer: text("installer").trim().to_string(),
            folder: text("folder").trim().to_string(),
            target: text("target").trim().to_string(),
            architecture: text("arch"),
            archive: self.vars["archive"].as_bool().unwrap_or(false),
            url: text("url"),
            local_path: text("local"),
        }
    }

    // ---------------- 输入 ----------------

    /// gui.py:apply_inputs —— JS 每次推回整份表单，这里负责落库 + 类型收敛。
    pub fn apply_inputs(&mut self, values: &Map<String, Value>) {
        for (key, value) in values {
            let name = JS_TO_VAR
                .iter()
                .find(|(js, _)| js == key)
                .map(|(_, var)| (*var).to_string())
                .unwrap_or_else(|| key.clone());
            if !self.vars.contains_key(&name) {
                continue;
            }
            if BOOL_VARS.contains(&name.as_str()) {
                self.vars.insert(name, json!(truthy(value)));
            } else if name == "mode" {
                if let Some(text) = value.as_str() {
                    if ["one", "folder", "online"].contains(&text) {
                        self.vars.insert(name, json!(text));
                    }
                }
            } else if name == "arch" || name == "tool_arch" {
                if let Some(text) = value.as_str() {
                    if super::ARCHITECTURES.contains(&text) {
                        self.vars.insert(name, json!(text));
                    }
                }
            } else {
                let text = match value {
                    Value::Null => String::new(),
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                self.vars.insert(name, json!(text));
            }
        }
    }

    /// invoke("set_input", {key, value})：落库后按计划相关性决定要不要重算。
    pub fn set_input(&mut self, key: &str, value: Value) -> Value {
        let mut values = Map::new();
        values.insert(key.to_string(), value);
        self.apply_inputs(&values);
        let name = JS_TO_VAR
            .iter()
            .find(|(js, _)| *js == key)
            .map(|(_, var)| (*var).to_string())
            .unwrap_or_else(|| key.to_string());
        if name == "workdir" && !self.vars["workdir"].as_str().unwrap_or("").is_empty() {
            self.prepare_workspace(None);
            self.set_banner(None, "info", "");
        }
        if PLAN_VARS.contains(&name.as_str()) {
            self.refresh_plan(None)
        } else {
            self.state()
        }
    }

    /// gui.py:refresh_plan —— 预览与真正执行共用同一次 plan_build，两者不会分叉。
    pub fn refresh_plan(&mut self, values: Option<&Map<String, Value>>) -> Value {
        if let Some(values) = values {
            self.apply_inputs(values);
        }
        self.plan = plan_build(&self.current_request());
        self.plan_titles = self
            .plan
            .steps
            .iter()
            .map(|step| self.t(&step.title))
            .collect();
        self.result_text = String::new();
        if !self.plan.error_key.is_empty() {
            let text = self.t(&self.plan.error_key);
            let detail = self.plan.error_detail.clone();
            self.set_banner(Some(&text), "warn", &detail);
        } else if !self.runner.running() {
            self.set_banner(None, "info", "");
        }
        let state = self.state();
        self.emit_event(json!({"kind": "state", "state": state.clone()}));
        state
    }

    /// invoke("refresh")：重读目录表 + 重扫安装包目录（gui.py 的两个 refresh_*）。
    pub fn refresh(&mut self) -> Value {
        let choices = super::target_choices();
        self.append_log(&format!("[INFO] catalog targets: {}\n", choices.len()), None);
        self.state()
    }

    pub fn set_banner(&mut self, text: Option<&str>, tone: &str, detail: &str) {
        self.banner_text = text.unwrap_or("").to_string();
        self.banner_tone = tone.to_string();
        self.banner_detail = detail.to_string();
    }

    fn set_status(&mut self, text: String, tone: &str) {
        self.status_text = text;
        self.status_tone = tone.to_string();
    }

    // ---------------- 工作区 ----------------

    /// gui.py:_prepare_workspace —— 目录、installers/README.txt、7zr.exe。
    pub fn prepare_workspace(&mut self, workdir: Option<PathBuf>) {
        let workdir = workdir.unwrap_or_else(|| self.workdir());
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&workdir)?;
            let installers = workdir.join(super::INSTALLER_DIR_NAME);
            if !installers.is_dir() {
                std::fs::create_dir_all(&installers)?;
            }
            let readme = installers.join("README.txt");
            if !readme.exists() {
                std::fs::write(&readme, README_TEXT)?;
            }
            let bundled = super::engine_root().join("7zr.exe");
            let local = workdir.join("7zr.exe");
            if bundled.is_file() && !local.exists() {
                std::fs::copy(&bundled, &local)?;
            }
            Ok(())
        })();
        if let Err(err) = result {
            self.append_log(&format!("[WARN] 工作目录准备失败: {err}\n"), None);
            return;
        }
        if self.vars["folder"].as_str().unwrap_or("").trim().is_empty() {
            let installers = workdir.join(super::INSTALLER_DIR_NAME);
            self.vars
                .insert("folder".to_string(), json!(installers.to_string_lossy()));
        }
    }

    /// gui.py:_require_workdir —— 失败就地提示，不弹窗。
    fn require_workdir(&mut self) -> Option<PathBuf> {
        let raw = self.vars["workdir"].as_str().unwrap_or("").trim().to_string();
        if raw.is_empty() {
            let text = self.t("gui_need_workdir");
            self.set_banner(Some(&text), "warn", "");
            return None;
        }
        let expanded = expand_user(&raw);
        let workdir = if expanded.is_absolute() {
            expanded
        } else {
            super::app_root().join(expanded)
        };
        if let Err(err) = std::fs::create_dir_all(&workdir) {
            self.set_banner(
                Some(&err.to_string()),
                "err",
                &workdir.to_string_lossy(),
            );
            return None;
        }
        self.vars
            .insert("workdir".to_string(), json!(workdir.to_string_lossy()));
        Some(workdir)
    }

    // ---------------- 任务 ----------------

    pub fn start_build(&mut self) -> Value {
        let Some(workdir) = self.require_workdir() else {
            return self.state();
        };
        self.prepare_workspace(Some(workdir));
        let plan = plan_build(&self.current_request());
        self.plan = plan.clone();
        self.plan_titles = plan
            .steps
            .iter()
            .map(|step| self.t(&step.title))
            .collect();
        if !plan.ok() {
            let text = self.t(&plan.error_key);
            let detail = plan.error_detail.clone();
            self.set_banner(Some(&text), "warn", &detail);
            return self.state();
        }
        self.start_plan(plan);
        self.state()
    }

    pub fn run_tool(&mut self, tool: &str) -> Value {
        let Some(workdir) = self.require_workdir() else {
            return self.state();
        };
        self.prepare_workspace(Some(workdir.clone()));
        let mut path = self.vars["tool_path"].as_str().unwrap_or("").trim().to_string();
        if !path.is_empty() && !Path::new(&path).is_absolute() {
            path = workdir.join(&path).to_string_lossy().into_owned();
        }
        let plan = plan_tool(
            tool,
            &workdir,
            &path,
            self.vars["tool_target"].as_str().unwrap_or("").trim(),
            self.vars["tool_arch"].as_str().unwrap_or("x64"),
            self.vars["tool_json"].as_bool().unwrap_or(false),
            true,
        );
        self.start_plan(plan);
        self.state()
    }

    fn start_plan(&mut self, plan: Plan) {
        if self.runner.running() {
            let text = self.t("gui_busy");
            self.set_banner(Some(&text), "warn", "");
            return;
        }
        if plan.steps.is_empty() {
            if !plan.error_key.is_empty() {
                let text = self.t(&plan.error_key);
                let detail = plan.error_detail.clone();
                self.set_banner(Some(&text), "warn", &detail);
            }
            return;
        }
        self.plan = plan;
        self.plan_titles = self
            .plan
            .steps
            .iter()
            .map(|step| self.t(&step.title))
            .collect();
        self.runner.token = self.vars["token"].as_str().unwrap_or("").to_string();
        self.runner.lang = self.lang.clone();
        self.started_at = Some(Instant::now());
        self.log_open = true;
        let first = self.plan_titles.first().cloned().unwrap_or_default();
        let status = self.tf("gui_status_running", &[("title", first.as_str())]);
        self.set_status(status, "run");
        self.task_title = first;
        let total = self.plan_titles.len().to_string();
        self.task_meta = self.tf("gui_task_step", &[("index", "0"), ("total", total.as_str())]);
        self.elapsed_text = String::new();
        self.result_text = String::new();
        self.cancel_armed = false;
        self.set_banner(None, "info", "");
        self.runner.start(self.plan.steps.clone(), self.plan_titles.clone());
        let state = self.state();
        self.emit_event(json!({"kind": "state", "state": state}));
        self.tick_clock();
    }

    /// gui.py:cancel —— 第一次点击只是武装，第二次才真的取消。
    pub fn cancel(&mut self) -> Value {
        if !self.runner.running() {
            return self.state();
        }
        if !self.cancel_armed {
            self.cancel_armed = true;
            let state = self.state();
            self.emit_event(json!({"kind": "state", "state": state.clone()}));
            return state;
        }
        self.cancel_armed = false;
        self.runner.cancel();
        let text = self.t("gui_status_cancelled");
        self.set_status(text.clone(), "warn");
        self.append_log(&format!("[WARN] {text}\n"), None);
        self.state()
    }

    // ---------------- 事件 ----------------

    /// gui.py:drain_once —— 抽干队列里现有的事件，返回处理条数（测试直接调）。
    pub fn drain_once(&mut self) -> usize {
        let mut handled = 0;
        while let Some(event) = self.runner.try_recv() {
            handled += 1;
            match event {
                JobEvent::Log(text) => self.append_log(&text, None),
                JobEvent::Step {
                    index,
                    total,
                    title,
                } => self.on_step(index, total, &title),
                JobEvent::Done { code, elapsed } => self.on_done(code, elapsed),
            }
        }
        handled
    }

    /// 80ms 一次的泵：批量取事件 + 每秒刷新计时。
    ///
    /// gui.py 用两个线程（事件泵 + Timer 计时）；合成一个循环就够了，
    /// 计时按约 1 秒的节拍推 state（逐行推事件会把 IPC 打满，这是既有教训）。
    pub fn pump_once(&mut self) -> usize {
        let handled = self.drain_once();
        self.tick_counter += 1;
        if self.runner.running() && self.tick_counter % 12 == 0 {
            self.tick_clock();
            let state = self.state();
            self.emit_event(json!({"kind": "state", "state": state}));
        }
        handled
    }

    fn tick_clock(&mut self) {
        if let Some(started) = self.started_at {
            let secs = format!("{:.0}s", started.elapsed().as_secs_f64());
            self.elapsed_text = self.tf("gui_elapsed", &[("time", secs.as_str())]);
        }
    }

    fn on_step(&mut self, index: usize, total: usize, title: &str) {
        self.task_title = title.to_string();
        let index_text = index.to_string();
        let total_text = total.to_string();
        self.task_meta = self.tf(
            "gui_task_step",
            &[("index", index_text.as_str()), ("total", total_text.as_str())],
        );
        self.append_log(
            &format!("\n===== [{index}/{total}] {title} =====\n"),
            Some("head"),
        );
        self.emit_event(json!({"kind": "step", "index": index, "total": total, "title": title}));
    }

    fn on_done(&mut self, code: i32, elapsed: f64) {
        let secs = format!("{elapsed:.1}s");
        self.elapsed_text = self.tf("gui_elapsed", &[("time", secs.as_str())]);
        if code == 0 {
            let text = self.tf("gui_status_done", &[("seconds", secs.as_str())]);
            self.set_status(text.clone(), "ok");
            self.append_log(&format!("\n[OK] {text}\n"), Some("ok"));
            self.task_title = text;
            self.set_banner(None, "info", "");
        } else if code == CANCELLED {
            let text = self.t("gui_status_cancelled");
            self.set_status(text.clone(), "warn");
            self.append_log(&format!("\n[WARN] {text}\n"), Some("warn"));
            self.task_title = text;
        } else {
            let code_text = code.to_string();
            let text = self.tf("gui_status_failed", &[("code", code_text.as_str())]);
            self.set_status(text.clone(), "err");
            self.append_log(&format!("\n[ERROR] {text}\n"), Some("err"));
            self.task_title = text.clone();
            self.set_banner(Some(&text), "err", "");
        }
        self.cancel_armed = false;
        self.started_at = None;
        self.refresh_plan(None);
        // gui.py 的顺序缺陷：它先算 _artifact_hint() 再 refresh_plan()，而后者把
        // result_text 清空——所以 Python 版界面上永远看不到产物落点。这里把顺序
        // 摆正（差异清单记录在 docs/MIGRATION_RUST_TAURI.md 附录）。
        if code == 0 {
            self.result_text = self.artifact_hint();
        }
        let state = self.state();
        self.emit_event(json!({
            "kind": "done",
            "code": code,
            "elapsed": elapsed,
            "result": self.result_text,
            "state": state,
        }));
    }

    /// gui.py:_artifact_hint —— 完成后给一个能直接点开的落点。
    fn artifact_hint(&self) -> String {
        let (release, assets) = self.artifact_paths();
        if self.vars["archive"].as_bool().unwrap_or(false) {
            let assets_dir = Path::new(&assets);
            if assets_dir.is_dir() {
                let mut newest: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
                if let Ok(reader) = std::fs::read_dir(assets_dir) {
                    for item in reader.flatten() {
                        let path = item.path();
                        if let Ok(meta) = path.metadata() {
                            if let Ok(modified) = meta.modified() {
                                newest.push((modified, path));
                            }
                        }
                    }
                }
                newest.sort_by(|a, b| a.0.cmp(&b.0));
                if let Some((_, path)) = newest.pop() {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    return format!("{}: {name}", self.t("gui_open_assets"));
                }
            }
        }
        if Path::new(&release).is_dir() {
            return format!("{}: {release}", self.t("gui_open_release"));
        }
        Path::new(&assets)
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or(assets)
    }

    // ---------------- 日志 ----------------

    pub fn append_log(&mut self, text: &str, tag: Option<&str>) {
        if text.is_empty() {
            return;
        }
        self.log_lines.push(text.to_string());
        if self.log_lines.len() > LOG_LIMIT {
            let cut = self.log_lines.len() - LOG_LIMIT;
            self.log_lines.drain(..cut);
        }
        self.emit_event(json!({"kind": "log", "text": text, "tag": tag.unwrap_or("")}));
    }

    pub fn log_text(&self) -> String {
        self.log_lines.concat()
    }

    pub fn clear_log(&mut self) -> Value {
        self.log_lines = vec![format!("{}\n", self.t("gui_log_empty"))];
        self.emit_event(json!({"kind": "log", "text": "", "tag": "clear"}));
        self.state()
    }

    pub fn toggle_log(&mut self) -> Value {
        self.log_open = !self.log_open;
        self.state()
    }

    pub fn set_autoscroll(&mut self, value: bool) -> Value {
        self.vars.insert("autoscroll".to_string(), json!(value));
        self.state()
    }

    /// invoke("log_save") 的落盘部分；对话框在 builder-app 侧。
    pub fn save_log(&mut self, path: &Path) -> Value {
        match std::fs::write(path, self.log_text()) {
            Ok(()) => {
                let text = self.t("gui_log_saved");
                self.set_banner(Some(&text), "ok", &path.to_string_lossy());
            }
            Err(err) => self.set_banner(Some(&err.to_string()), "err", &path.to_string_lossy()),
        }
        self.state()
    }

    pub fn log_saved_banner(&mut self) -> Value {
        let text = self.t("gui_log_saved");
        self.set_banner(Some(&text), "ok", "");
        self.state()
    }

    pub fn log_copied_banner(&mut self) -> Value {
        let text = self.t("gui_log_copied");
        self.set_banner(Some(&text), "ok", "");
        self.state()
    }

    // ---------------- 打开路径 ----------------

    /// 校验并规范化要打开的路径；真正打开由 builder-app 的 opener 插件做。
    pub fn open_target(&mut self, path: &str) -> Option<PathBuf> {
        let target = PathBuf::from(path);
        let _ = std::fs::create_dir_all(&target);
        if !target.exists() {
            let text = self.t("gui_need_workdir");
            self.set_banner(Some(&text), "warn", &target.to_string_lossy());
            return None;
        }
        Some(target)
    }

    pub fn open_failed(&mut self, path: &str, err: &str) -> Value {
        self.set_banner(Some(err), "err", path);
        self.state()
    }

    pub fn open_ok(&mut self) -> Value {
        self.state()
    }

    // ---------------- 事件出口 ----------------

    fn emit_event(&self, payload: Value) {
        if let Some(emit) = &self.emit {
            emit(payload);
        }
    }

    #[doc(hidden)]
    pub fn set_emit(&mut self, emit: Option<Emit>) {
        self.emit = emit;
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().map(|n| n != 0.0).unwrap_or(false),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

/// Python 的 Path.expanduser()：只认开头的 ~。
fn expand_user(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix('~') {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let rest = rest.trim_start_matches(['/', '\\']);
        if rest.is_empty() {
            return home;
        }
        return home.join(rest);
    }
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cpb-ctl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn controller(name: &str) -> (Controller, PathBuf) {
        let dir = temp_dir(name);
        (Controller::with_workdir(None, dir.clone()), dir)
    }

    fn shell_step(script: &str) -> super::super::plan::Step {
        #[cfg(windows)]
        let cmd = vec!["cmd".to_string(), "/C".to_string(), script.to_string()];
        #[cfg(not(windows))]
        let cmd = vec!["sh".to_string(), "-c".to_string(), script.to_string()];
        super::super::plan::Step {
            title: "gui_step_build_one".to_string(),
            cmd,
            cwd: std::env::temp_dir(),
        }
    }

    /// 前端契约：state() 的每个字段名都必须在（app.js 尾部注释就是这份清单）。
    #[test]
    fn state_exposes_frontend_contract_fields() {
        let (controller, _dir) = controller("state");
        let state = controller.state();
        for key in [
            "lang",
            "strings",
            "vars",
            "catalog_choices",
            "architectures",
            "installer_choices",
            "plan",
            "banner",
            "status",
            "result",
            "task",
            "elapsed",
            "log",
            "running",
            "cancel_armed",
            "artifact",
        ] {
            assert!(state.get(key).is_some(), "缺少顶层字段 {key}");
        }
        let vars = state["vars"].as_object().unwrap();
        let names: Vec<&str> = vars.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "mode",
                "installer",
                "folder",
                "target",
                "arch",
                "archive",
                "url",
                "local",
                "token",
                "tool_path",
                "tool_target",
                "tool_arch",
                "tool_json",
                "autoscroll",
                "workdir"
            ],
            "vars 的键顺序是契约（前端按名取值）"
        );
        assert_eq!(vars["target"], "chrome_stable");
        assert_eq!(vars["tool_target"], "chrome_stable");
        assert_eq!(vars["arch"], "x64");
        assert_eq!(vars["archive"], true);
        assert_eq!(state["plan"]["error_key"], "");
        assert_eq!(state["status"]["tone"], "idle");
        assert_eq!(state["banner"]["tone"], "info");
        assert!(state["artifact"]["release"].as_str().unwrap().ends_with("release"));
    }

    /// 工作区准备：installers/、README.txt、folder 默认值（gui.py:_prepare_workspace）。
    #[test]
    fn workspace_preparation_creates_installer_folder() {
        let (controller, dir) = controller("workspace");
        let installers = dir.join("installers");
        assert!(installers.is_dir());
        let readme = installers.join("README.txt");
        assert!(readme.is_file());
        let text = std::fs::read_to_string(&readme).unwrap();
        assert!(text.contains("把要构建的浏览器安装包放在这里。"));
        assert_eq!(
            controller.vars["folder"].as_str().unwrap(),
            installers.to_string_lossy()
        );
    }

    /// 安装包列表：按名字排序 + 体积换算 + 后缀过滤。
    #[test]
    fn installer_choices_lists_packages_with_size() {
        let (controller, dir) = controller("choices");
        let installers = dir.join("installers");
        std::fs::write(installers.join("b_setup.exe"), vec![0u8; 2048]).unwrap();
        std::fs::write(installers.join("A_setup.msi"), vec![0u8; 1024]).unwrap();
        std::fs::write(installers.join("notes.txt"), b"x").unwrap();
        let choices = controller.state()["installer_choices"].clone();
        let items = choices.as_array().unwrap();
        assert_eq!(items.len(), 2, "{choices}");
        assert_eq!(items[0]["name"], "A_setup.msi");
        assert_eq!(items[1]["name"], "b_setup.exe");
        assert_eq!(items[1]["size"], 2048);
        assert_eq!(items[1]["size_text"], "2.0 KB");
    }

    /// set_input：JS 的 local_path 落到 local；非法枚举值被忽略；未知键不动状态。
    #[test]
    fn set_input_coerces_like_python() {
        let (mut controller, _dir) = controller("inputs");
        controller.set_input("local_path", json!("C:\\pkg\\a.exe"));
        assert_eq!(controller.vars["local"], "C:\\pkg\\a.exe");
        controller.set_input("mode", json!("nonsense"));
        assert_eq!(controller.vars["mode"], "one");
        controller.set_input("arch", json!("arm64"));
        assert_eq!(controller.vars["arch"], "arm64");
        controller.set_input("arch", json!("mips"));
        assert_eq!(controller.vars["arch"], "arm64");
        controller.set_input("archive", json!(false));
        assert_eq!(controller.vars["archive"], false);
        controller.set_input("unknown_key", json!("x"));
        assert!(controller.vars.get("unknown_key").is_none());
    }

    /// 校验失败就地出横幅（不弹窗），并把路径带进 detail。
    #[test]
    fn missing_installer_sets_banner_not_dialog() {
        let (mut controller, _dir) = controller("banner");
        let state = controller.refresh_plan(None);
        assert_eq!(state["plan"]["error_key"], "gui_need_installer");
        assert_eq!(state["banner"]["tone"], "warn");
        assert_eq!(
            state["banner"]["text"],
            controller.t("gui_need_installer")
        );
    }

    /// 真跑一个子进程走完 start → step → done：状态、计时、结果提示都要落地。
    #[test]
    fn controller_runs_plan_and_reports_done() {
        let (mut controller, _dir) = controller("run");
        let plan = Plan {
            steps: vec![shell_step("echo cpb-controller-test")],
            ..Plan::default()
        };
        controller.start_plan(plan);
        assert!(controller.runner.running());

        let deadline = Instant::now() + Duration::from_secs(30);
        let mut finished = false;
        while Instant::now() < deadline {
            controller.pump_once();
            if controller.started_at.is_none() && !controller.runner.running() {
                finished = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(finished, "任务没结束");
        assert_eq!(controller.status_tone, "ok");
        let template = controller.tf("gui_status_done", &[("seconds", "0.1s")]);
        assert!(
            controller.status_text.starts_with(template.split("0.1s").next().unwrap()),
            "状态串: {}",
            controller.status_text
        );
        assert!(controller.log_text().contains("cpb-controller-test"));
        assert!(!controller.log_text().contains("gui_step_build_one"));
        assert!(controller.log_text().contains("===== [1/1]"));
        assert!(!controller.elapsed_text.is_empty());
        assert!(!controller.result_text.is_empty());
        assert!(controller.log_open);
    }

    /// 取消要两下：第一下只是武装（gui.py:cancel 的破坏性操作防误触）。
    #[test]
    fn cancel_needs_two_clicks() {
        let (mut controller, _dir) = controller("cancel");
        let state = controller.cancel();
        assert_eq!(state["cancel_armed"], false);

        let plan = Plan {
            steps: vec![shell_step(if cfg!(windows) {
                "ping -n 30 127.0.0.1 > nul"
            } else {
                "sleep 30"
            })],
            ..Plan::default()
        };
        controller.start_plan(plan);
        std::thread::sleep(Duration::from_millis(200));
        let armed = controller.cancel();
        assert_eq!(armed["cancel_armed"], true);
        assert!(controller.runner.running(), "第一下不该真的取消");
        let stopped = controller.cancel();
        assert_eq!(stopped["cancel_armed"], false);

        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            controller.pump_once();
            if !controller.runner.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(controller.status_tone, "warn");
        assert_eq!(controller.status_text, controller.t("gui_status_cancelled"));
        assert!(!controller.log_text().contains("gui_status_cancelled"));
    }

    /// 日志只留最近 4000 行；清空后留一条占位。
    #[test]
    fn log_buffer_keeps_recent_lines_only() {
        let (mut controller, _dir) = controller("log");
        for index in 0..(LOG_LIMIT + 500) {
            controller.append_log(&format!("line-{index}\n"), None);
        }
        assert_eq!(controller.log_lines.len(), LOG_LIMIT);
        assert!(controller.log_text().starts_with("line-500\n"));
        controller.clear_log();
        assert_eq!(controller.log_lines.len(), 1);
        assert!(controller.log_text().contains(&controller.t("gui_log_empty")));
    }

    /// 事件出口：状态/日志/步骤/完成四类事件都要带着 kind 发出去。
    #[test]
    fn emit_event_payloads_carry_kind() {
        let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let emit: Emit = Arc::new(move |payload: Value| {
            sink.lock().unwrap().push(payload);
        });
        let dir = temp_dir("emit");
        let mut controller = Controller::with_workdir(Some(emit), dir);
        controller.refresh_plan(None);
        controller.append_log("hello\n", Some("cmd"));
        let plan = Plan {
            steps: vec![shell_step("echo emit-test")],
            ..Plan::default()
        };
        controller.start_plan(plan);
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            controller.pump_once();
            if !controller.runner.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let events = seen.lock().unwrap().clone();
        let kinds: Vec<String> = events
            .iter()
            .filter_map(|event| event["kind"].as_str().map(str::to_string))
            .collect();
        assert!(kinds.contains(&"state".to_string()), "{kinds:?}");
        assert!(kinds.contains(&"log".to_string()), "{kinds:?}");
        assert!(kinds.contains(&"step".to_string()), "{kinds:?}");
        assert!(kinds.contains(&"done".to_string()), "{kinds:?}");
        let log_event = events
            .iter()
            .find(|event| event["kind"] == "log" && event["tag"] == "cmd")
            .expect("带 tag 的日志事件");
        assert_eq!(log_event["text"], "hello\n");
    }
}
