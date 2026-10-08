//! 子进程执行与日志回流（无窗口依赖，可无头测试）。
//! 1:1 对照 scripts/gui_runner.py：顺序跑步骤，把 log/step/done 事件推进队列，
//! 界面侧按 80ms 批量取走（逐行推会在解包几千行日志时把 IPC 打满）。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

use super::plan::Step;

/// 退出码 -2 专门表示「用户取消」，其余原样透传子进程退出码。
pub const CANCELLED: i32 = -2;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// runner 推向界面的事件（gui_runner 的队列协议）。
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Log(String),
    Step {
        index: usize,
        total: usize,
        title: String,
    },
    Done {
        code: i32,
        elapsed: f64,
    },
}

/// Windows 命令行还原（subprocess.list2cmdline），只用于日志里显示 "$ ..."。
pub fn list2cmdline(args: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for arg in args {
        let needs_quote = arg.is_empty()
            || arg
                .chars()
                .any(|ch| ch == ' ' || ch == '\t' || ch == '"');
        if !needs_quote {
            parts.push(arg.clone());
            continue;
        }
        let mut quoted = String::with_capacity(arg.len() + 2);
        quoted.push('"');
        let mut backslashes = 0usize;
        for ch in arg.chars() {
            match ch {
                '\\' => {
                    backslashes += 1;
                }
                '"' => {
                    // 反斜杠在引号前要翻倍，引号自身也要转义
                    quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                    quoted.push('"');
                    backslashes = 0;
                }
                _ => {
                    quoted.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    quoted.push(ch);
                }
            }
        }
        quoted.push_str(&"\\".repeat(backslashes * 2));
        quoted.push('"');
        parts.push(quoted);
    }
    parts.join(" ")
}

fn log_file_name() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cpb-{}-{unique}",
        std::process::id()
    ))
}

/// 在后台线程里顺序执行步骤（gui_runner.JobRunner）。
pub struct JobRunner {
    events_tx: Sender<JobEvent>,
    events_rx: Receiver<JobEvent>,
    cancel: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    started_at: Arc<Mutex<Option<Instant>>>,
    pub lang: String,
    pub token: String,
}

impl JobRunner {
    pub fn new(lang: &str) -> JobRunner {
        let (events_tx, events_rx) = channel();
        JobRunner {
            events_tx,
            events_rx,
            cancel: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            started_at: Arc::new(Mutex::new(None)),
            lang: lang.to_string(),
            token: String::new(),
        }
    }

    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn started_at(&self) -> Option<Instant> {
        *self.started_at.lock().unwrap()
    }

    /// 取一条事件（控制器的 drain_once 用；没有就返回 None）。
    pub fn try_recv(&self) -> Option<JobEvent> {
        self.events_rx.try_recv().ok()
    }

    /// 已经在跑就拒绝（gui_runner.start 返回 False）。
    pub fn start(&self, steps: Vec<Step>, titles: Vec<String>) -> bool {
        if self.running() {
            return false;
        }
        self.cancel.store(false, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
        *self.started_at.lock().unwrap() = Some(Instant::now());

        let started = Instant::now();
        *self.started_at.lock().unwrap() = Some(started);

        let tx = self.events_tx.clone();
        let cancel = Arc::clone(&self.cancel);
        let running = Arc::clone(&self.running);
        let child = Arc::clone(&self.child);
        let token = self.token.clone();
        thread::spawn(move || {
            let total = steps.len();
            let mut code = 0i32;
            for (offset, step) in steps.into_iter().enumerate() {
                let index = offset + 1;
                if cancel.load(Ordering::SeqCst) {
                    code = CANCELLED;
                    break;
                }
                let title = titles.get(offset).cloned().unwrap_or_default();
                let _ = tx.send(JobEvent::Step {
                    index,
                    total,
                    title,
                });
                code = run_step(&tx, &cancel, &child, &token, &step);
                if cancel.load(Ordering::SeqCst) {
                    code = CANCELLED;
                    break;
                }
                if code != 0 {
                    break;
                }
            }
            running.store(false, Ordering::SeqCst);
            let elapsed = started.elapsed().as_secs_f64();
            let _ = tx.send(JobEvent::Done { code, elapsed });
        });
        true
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            kill(child);
        }
    }
}

fn kill(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let killed = Command::new("taskkill")
            .args(["/F", "/T", "/PID", pid.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .is_ok();
        if killed {
            return;
        }
    }
    let _ = child.kill();
}

/// 跑一个步骤：子进程 stdout/stderr 直接重定向到临时日志文件，按偏移量跟随。
///
/// 走文件而不是管道：这是 gui_runner.py 里写明的既有决定——顺序追加的文件
/// 读起来不会丢行，也不受窗口程序没有控制台的影响。
fn run_step(
    tx: &Sender<JobEvent>,
    cancel: &Arc<AtomicBool>,
    slot: &Arc<Mutex<Option<Child>>>,
    token: &str,
    step: &Step,
) -> i32 {
    if step.cmd.is_empty() {
        let _ = tx.send(JobEvent::Log("[ERROR] 空命令\n".to_string()));
        return 1;
    }
    let folder = log_file_name();
    if let Err(err) = fs::create_dir_all(&folder) {
        let _ = tx.send(JobEvent::Log(format!("[ERROR] 无法创建日志目录: {err}\n")));
        return 1;
    }
    let log_path = folder.join("job.log");
    let _ = tx.send(JobEvent::Log(format!(
        "\n$ {}\n",
        list2cmdline(&step.cmd)
    )));

    let stream = match fs::File::create(&log_path) {
        Ok(file) => file,
        Err(err) => {
            let _ = tx.send(JobEvent::Log(format!("[ERROR] 无法创建日志文件: {err}\n")));
            let _ = fs::remove_dir_all(&folder);
            return 1;
        }
    };
    let stream_err = match stream.try_clone() {
        Ok(file) => file,
        Err(err) => {
            let _ = tx.send(JobEvent::Log(format!("[ERROR] 无法重定向 stdout: {err}\n")));
            let _ = fs::remove_dir_all(&folder);
            return 1;
        }
    };

    let mut command = Command::new(&step.cmd[0]);
    command
        .args(&step.cmd[1..])
        .current_dir(&step.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stream))
        .stderr(Stdio::from(stream_err))
        .env(super::LOG_ENV, &log_path);
    if token.trim().is_empty() {
        command.env_remove("GITHUB_TOKEN");
    } else {
        command.env("GITHUB_TOKEN", token.trim());
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = tx.send(JobEvent::Log(format!("[ERROR] 无法启动子进程: {err}\n")));
            let _ = fs::remove_dir_all(&folder);
            return 1;
        }
    };
    *slot.lock().unwrap() = Some(child);

    let code = follow(tx, cancel, slot, &log_path);
    *slot.lock().unwrap() = None;
    let _ = fs::remove_dir_all(&folder);
    code
}

/// 按偏移量读日志文件，直到子进程退出；退出后再把尾部读干。
fn follow(
    tx: &Sender<JobEvent>,
    cancel: &Arc<AtomicBool>,
    slot: &Arc<Mutex<Option<Child>>>,
    log_path: &Path,
) -> i32 {
    let mut offset = 0u64;
    let mut carry: Vec<u8> = Vec::new();
    loop {
        offset = drain_log(tx, log_path, offset, &mut carry);
        let status = {
            let mut guard = slot.lock().unwrap();
            match guard.as_mut() {
                Some(child) => child.try_wait().ok().flatten(),
                None => None,
            }
        };
        if let Some(status) = status {
            loop {
                let next = drain_log(tx, log_path, offset, &mut carry);
                if next == offset {
                    break;
                }
                offset = next;
            }
            if !carry.is_empty() {
                let _ = tx.send(JobEvent::Log(String::from_utf8_lossy(&carry).into_owned()));
            }
            if cancel.load(Ordering::SeqCst) {
                return CANCELLED;
            }
            return status.code().unwrap_or(1);
        }
        if cancel.load(Ordering::SeqCst) {
            if let Some(child) = slot.lock().unwrap().as_mut() {
                kill(child);
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn drain_log(tx: &Sender<JobEvent>, path: &Path, offset: u64, carry: &mut Vec<u8>) -> u64 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut handle) = fs::File::open(path) else {
        return offset;
    };
    if handle.seek(SeekFrom::Start(offset)).is_err() {
        return offset;
    }
    let mut chunk = Vec::new();
    if handle.read_to_end(&mut chunk).is_err() || chunk.is_empty() {
        return offset;
    }
    carry.extend_from_slice(&chunk);
    // 只切到最后一个完整字符边界：多字节字符被切成两半时先攒着
    let text = match std::str::from_utf8(carry) {
        Ok(text) => {
            let text = text.to_string();
            carry.clear();
            text
        }
        Err(err) => {
            let valid = err.valid_up_to();
            let text = String::from_utf8_lossy(&carry[..valid]).into_owned();
            carry.drain(..valid);
            text
        }
    };
    if !text.is_empty() {
        let _ = tx.send(JobEvent::Log(text));
    }
    offset + chunk.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_step(script: &str) -> Step {
        #[cfg(windows)]
        let cmd = vec!["cmd".to_string(), "/C".to_string(), script.to_string()];
        #[cfg(not(windows))]
        let cmd = vec!["sh".to_string(), "-c".to_string(), script.to_string()];
        Step {
            title: "gui_step_build_one".to_string(),
            cmd,
            cwd: std::env::temp_dir(),
        }
    }

    /// 真跑一个子进程：日志要回流，退出码要透传，最后必须有 Done。
    #[test]
    fn runner_streams_log_and_reports_exit_code() {
        let runner = JobRunner::new("zh-CN");
        assert!(runner.start(vec![shell_step("echo hello-cpb")], vec!["t".to_string()]));

        let deadline = Instant::now() + Duration::from_secs(30);
        let mut text = String::new();
        let mut code = None;
        while Instant::now() < deadline {
            match runner.try_recv() {
                Some(JobEvent::Log(chunk)) => text.push_str(&chunk),
                Some(JobEvent::Done { code: done, .. }) => {
                    code = Some(done);
                    break;
                }
                Some(JobEvent::Step { .. }) | None => thread::sleep(Duration::from_millis(20)),
            }
        }
        assert_eq!(code, Some(0), "事件流: {text}");
        assert!(text.contains("hello-cpb"), "日志: {text}");
        assert!(!runner.running());
        // 跑完还能再跑（同一 runner 复用）
        assert!(runner.start(vec![shell_step("exit 3")], vec!["t".to_string()]));
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut code = None;
        while Instant::now() < deadline {
            if let Some(JobEvent::Done { code: done, .. }) = runner.try_recv() {
                code = Some(done);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(code, Some(3));
    }

    /// 取消：码是 -2，且不会等到子进程自己结束。
    #[test]
    fn runner_cancel_reports_minus_two() {
        let runner = JobRunner::new("zh-CN");
        #[cfg(windows)]
        let script = "ping -n 30 127.0.0.1 > nul";
        #[cfg(not(windows))]
        let script = "sleep 30";
        assert!(runner.start(vec![shell_step(script)], vec!["t".to_string()]));
        thread::sleep(Duration::from_millis(300));
        runner.cancel();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut code = None;
        while Instant::now() < deadline {
            if let Some(JobEvent::Done { code: done, .. }) = runner.try_recv() {
                code = Some(done);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(code, Some(CANCELLED));
    }

    /// 命令行还原：带空格的参数要加引号（日志可读性 + 与 Python 显示一致）。
    #[test]
    fn list2cmdline_quotes_like_python() {
        let args = vec![
            "portable-builder.exe".to_string(),
            "--workdir".to_string(),
            "C:\\Program Files\\x".to_string(),
            "build".to_string(),
        ];
        assert_eq!(
            list2cmdline(&args),
            "portable-builder.exe --workdir \"C:\\Program Files\\x\" build"
        );
        assert_eq!(list2cmdline(&["".to_string()]), "\"\"");
    }
}
