//! 界面一次点击 → 一串可顺序执行的子进程步骤（纯函数，无 IO、无窗口）。
//! 1:1 对照 scripts/gui_plan.py；改这里的唯一真源是那份 Python，不是界面。

use std::path::{Path, PathBuf};

/// 引擎可执行文件：环境变量 → GUI 旁的同名 sidecar → PATH。
///
/// 开发期（cargo run -p builder-app）引擎就在同一个 target 目录里，分发期由
/// Tauri 的 sidecar 机制放在 GUI 旁边，两种情形都命中「exe 旁」。
pub fn engine_exe() -> PathBuf {
    if let Some(raw) = std::env::var_os("CPB_ENGINE_EXE") {
        return PathBuf::from(raw);
    }
    let dir = super::app_root();
    for name in ["portable-builder.exe", "portable-builder"] {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(if cfg!(windows) {
        "portable-builder.exe"
    } else {
        "portable-builder"
    })
}

/// 一个子进程步骤；title 是语言包 key，显示前再翻译（gui_plan.Step）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub title: String,
    pub cmd: Vec<String>,
    pub cwd: PathBuf,
}

impl Step {
    fn new(title: &str, cmd: Vec<String>, cwd: PathBuf) -> Step {
        Step {
            title: title.to_string(),
            cmd,
            cwd,
        }
    }
}

/// 界面上的全部输入（gui_plan.BuildRequest）。
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub workdir: PathBuf,
    pub mode: String,
    pub installer: String,
    pub folder: String,
    pub target: String,
    pub architecture: String,
    pub archive: bool,
    pub url: String,
    pub local_path: String,
}

impl Default for BuildRequest {
    fn default() -> Self {
        BuildRequest {
            workdir: super::default_workdir(),
            mode: "one".to_string(),
            installer: String::new(),
            folder: String::new(),
            target: String::new(),
            architecture: "x64".to_string(),
            archive: true,
            url: String::new(),
            local_path: String::new(),
        }
    }
}

/// 步骤 + 校验结果。界面用它渲染预览，也在点「开始」之前就地报错。
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub error_key: String,
    pub error_detail: String,
}

impl Plan {
    pub fn ok(&self) -> bool {
        self.error_key.is_empty() && !self.steps.is_empty()
    }

    fn error(key: &str) -> Plan {
        Plan {
            error_key: key.to_string(),
            ..Plan::default()
        }
    }

    fn error_with(key: &str, detail: String) -> Plan {
        Plan {
            error_key: key.to_string(),
            error_detail: detail,
            ..Plan::default()
        }
    }
}

/// 引擎命令：全局参数在前，子命令在后（与 CLI 契约一致）。
pub fn engine_command(workdir: &Path, args: &[&str]) -> Vec<String> {
    let mut cmd = vec![
        engine_exe().to_string_lossy().into_owned(),
        "--workdir".to_string(),
        workdir.to_string_lossy().into_owned(),
    ];
    cmd.extend(args.iter().map(|arg| (*arg).to_string()));
    cmd
}

fn archive_flag(archive: bool) -> Vec<&'static str> {
    if archive {
        vec!["--archive"]
    } else {
        vec![]
    }
}

/// 把界面上的路径解析成绝对路径：相对路径一律相对工作目录（gui_plan.resolve_input）。
pub fn resolve_input(request: &BuildRequest, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        request.workdir.join(path)
    }
}

/// gui_plan.plan_build。
pub fn plan_build(request: &BuildRequest) -> Plan {
    let arch = if request.architecture.is_empty() {
        "x64".to_string()
    } else {
        request.architecture.clone()
    };
    let workdir = request.workdir.clone();

    match request.mode.as_str() {
        "one" => {
            if request.installer.is_empty() {
                return Plan::error("gui_need_installer");
            }
            let path = resolve_input(request, &request.installer);
            if !path.exists() {
                return Plan::error_with("gui_need_installer", path.to_string_lossy().into_owned());
            }
            let path_text = path.to_string_lossy().into_owned();
            let mut args = vec![
                "build-package",
                path_text.as_str(),
                "--architecture",
                arch.as_str(),
            ];
            args.extend(archive_flag(request.archive));
            Plan {
                steps: vec![Step::new(
                    "gui_step_build_one",
                    engine_command(&workdir, &args),
                    workdir,
                )],
                ..Plan::default()
            }
        }
        "folder" => {
            if request.folder.is_empty() {
                return Plan::error("gui_need_folder");
            }
            let folder = resolve_input(request, &request.folder);
            if !folder.is_dir() {
                return Plan::error_with("gui_need_folder", folder.to_string_lossy().into_owned());
            }
            let folder_text = folder.to_string_lossy().into_owned();
            let mut args = vec![
                "build-packages",
                folder_text.as_str(),
                "--architecture",
                arch.as_str(),
            ];
            args.extend(archive_flag(request.archive));
            Plan {
                steps: vec![Step::new(
                    "gui_step_build_folder",
                    engine_command(&workdir, &args),
                    workdir,
                )],
                ..Plan::default()
            }
        }
        _ => {
            if request.target.is_empty() {
                return Plan::error("gui_need_target");
            }
            let config = workdir.join("build").join("selected.browser.json");
            let config_text = config.to_string_lossy().into_owned();
            let mut prepare: Vec<&str> = vec![
                "prepare-target",
                "--browser",
                request.target.as_str(),
                "--output",
                config_text.as_str(),
                "--architecture",
                arch.as_str(),
            ];
            let url = request.url.trim();
            if !url.is_empty() {
                prepare.push("--url");
                prepare.push(url);
            }
            let local = request.local_path.trim();
            if !local.is_empty() {
                prepare.push("--path");
                prepare.push(local);
            }

            let mut steps = vec![
                Step::new(
                    "gui_step_prepare",
                    engine_command(&workdir, &prepare),
                    workdir.clone(),
                ),
                Step::new(
                    "gui_step_build_online",
                    engine_command(
                        &workdir,
                        &[
                            "--config",
                            config_text.as_str(),
                            "--target",
                            request.target.as_str(),
                            "build",
                        ],
                    ),
                    workdir.clone(),
                ),
            ];
            if request.archive {
                steps.push(Step::new(
                    "gui_step_archive",
                    engine_command(
                        &workdir,
                        &[
                            "--config",
                            config_text.as_str(),
                            "--target",
                            request.target.as_str(),
                            "archive",
                        ],
                    ),
                    workdir.clone(),
                ));
                steps.push(Step::new(
                    "gui_step_verify",
                    engine_command(
                        &workdir,
                        &[
                            "--config",
                            config_text.as_str(),
                            "--target",
                            request.target.as_str(),
                            "verify",
                            "--no-smoke",
                        ],
                    ),
                    workdir.clone(),
                ));
            }
            Plan {
                steps,
                ..Plan::default()
            }
        }
    }
}

/// gui_plan.plan_tool：工具箱按钮 → 步骤。
///
/// 与 Python 的差异只有一处：chplus 按钮已删除。update_chrome_plus.py 是维护者
/// CI 的活（migration doc §0 明确保留 Python），Rust 引擎没有对应子命令，
/// 终端用户也不该在 GUI 里更新仓库内置的 Chrome++。
pub fn plan_tool(
    tool: &str,
    workdir: &Path,
    path: &str,
    target: &str,
    architecture: &str,
    json_output: bool,
    no_smoke: bool,
) -> Plan {
    let workdir = workdir.to_path_buf();
    let config = workdir.join("build").join("selected.browser.json");
    let config_text = config.to_string_lossy().into_owned();

    match tool {
        "inspect" => {
            let mut args: Vec<&str> =
                vec!["inspect-package", path, "--architecture", architecture];
            if json_output {
                args.push("--json");
            }
            Plan {
                steps: vec![Step::new(
                    "gui_tool_inspect",
                    engine_command(&workdir, &args),
                    workdir,
                )],
                ..Plan::default()
            }
        }
        "research" => {
            let mut args: Vec<&str> =
                vec!["research-packages", path, "--architecture", architecture];
            if json_output {
                args.push("--json");
            }
            Plan {
                steps: vec![Step::new(
                    "gui_tool_research",
                    engine_command(&workdir, &args),
                    workdir,
                )],
                ..Plan::default()
            }
        }
        "prepare" => {
            let mut args: Vec<&str> = vec![
                "prepare-target",
                "--browser",
                target,
                "--output",
                config_text.as_str(),
                "--architecture",
                architecture,
            ];
            if !path.is_empty() {
                if Path::new(path).is_dir() {
                    return Plan::error("gui_tool_prepare_hint");
                }
                args.push("--path");
                args.push(path);
            }
            Plan {
                steps: vec![Step::new(
                    "gui_tool_prepare",
                    engine_command(&workdir, &args),
                    workdir,
                )],
                ..Plan::default()
            }
        }
        "resolve" => Plan {
            steps: vec![Step::new(
                "gui_tool_resolve",
                engine_command(&workdir, &["resolve-upstream", "--browser", target, "--json"]),
                workdir,
            )],
            ..Plan::default()
        },
        "verify" => {
            let mut args: Vec<&str> =
                vec!["--config", config_text.as_str(), "--target", target, "verify"];
            if no_smoke {
                args.push("--no-smoke");
            }
            Plan {
                steps: vec![Step::new(
                    "gui_tool_verify",
                    engine_command(&workdir, &args),
                    workdir,
                )],
                ..Plan::default()
            }
        }
        _ => Plan::error("gui_need_path"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cpb-plan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// mode=one：安装包不存在时报 gui_need_installer 并把路径带上（就地提示）。
    #[test]
    fn plan_build_one_mode_requires_existing_installer() {
        let workdir = temp_workdir("one");
        let mut request = BuildRequest {
            workdir: workdir.clone(),
            mode: "one".to_string(),
            ..BuildRequest::default()
        };
        assert_eq!(plan_build(&request).error_key, "gui_need_installer");

        let missing = workdir.join("nope.exe");
        request.installer = missing.to_string_lossy().into_owned();
        let plan = plan_build(&request);
        assert_eq!(plan.error_key, "gui_need_installer");
        assert_eq!(plan.error_detail, missing.to_string_lossy());

        std::fs::write(&missing, b"x").unwrap();
        let plan = plan_build(&request);
        assert!(plan.ok(), "{plan:?}");
        assert_eq!(plan.steps[0].title, "gui_step_build_one");
        let cmd = &plan.steps[0].cmd;
        assert_eq!(cmd[1], "--workdir");
        assert_eq!(cmd[2], workdir.to_string_lossy());
        assert_eq!(cmd[3], "build-package");
        assert_eq!(cmd[4], missing.to_string_lossy());
        assert_eq!(cmd[5], "--architecture");
        assert_eq!(cmd[6], "x64");
        assert_eq!(cmd[7], "--archive");
    }

    /// mode=folder：目录用途校验 + --archive 随开关。
    #[test]
    fn plan_build_folder_mode_checks_directory() {
        let workdir = temp_workdir("folder");
        let mut request = BuildRequest {
            workdir: workdir.clone(),
            mode: "folder".to_string(),
            installer: String::new(),
            archive: false,
            ..BuildRequest::default()
        };
        assert_eq!(plan_build(&request).error_key, "gui_need_folder");

        request.folder = workdir.join("missing").to_string_lossy().into_owned();
        assert_eq!(plan_build(&request).error_key, "gui_need_folder");

        request.folder = workdir.to_string_lossy().into_owned();
        let plan = plan_build(&request);
        assert!(plan.ok());
        assert_eq!(plan.steps[0].cmd[3], "build-packages");
        assert!(!plan.steps[0].cmd.iter().any(|arg| arg == "--archive"));
    }

    /// mode=online：prepare → build → archive → verify 四步，config 落在 build/ 下。
    #[test]
    fn plan_build_online_mode_chains_four_steps() {
        let workdir = temp_workdir("online");
        let request = BuildRequest {
            workdir: workdir.clone(),
            mode: "online".to_string(),
            target: "chrome_stable".to_string(),
            url: " https://example.invalid/setup.exe ".to_string(),
            local_path: "C:\\pkg\\setup.exe".to_string(),
            ..BuildRequest::default()
        };
        let plan = plan_build(&request);
        assert!(plan.ok(), "{plan:?}");
        let titles: Vec<&str> = plan.steps.iter().map(|step| step.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "gui_step_prepare",
                "gui_step_build_online",
                "gui_step_archive",
                "gui_step_verify"
            ]
        );

        let prepare = &plan.steps[0].cmd;
        let config = workdir
            .join("build")
            .join("selected.browser.json")
            .to_string_lossy()
            .into_owned();
        assert_eq!(prepare[3], "prepare-target");
        assert_eq!(prepare[4], "--browser");
        assert_eq!(prepare[5], "chrome_stable");
        assert_eq!(prepare[6], "--output");
        assert_eq!(prepare[7], config);
        assert_eq!(prepare[8], "--architecture");
        assert_eq!(prepare[9], "x64");
        // url 两边去空白后再传（gui_plan 的 request.url.strip()）
        assert_eq!(prepare[10], "--url");
        assert_eq!(prepare[11], "https://example.invalid/setup.exe");
        assert_eq!(prepare[12], "--path");
        assert_eq!(prepare[13], "C:\\pkg\\setup.exe");

        // build/archive/verify 共用 --config + --target，verify 固定 --no-smoke
        for (index, subcommand) in [(1, "build"), (2, "archive"), (3, "verify")] {
            let cmd = &plan.steps[index].cmd;
            assert_eq!(cmd[3], "--config");
            assert_eq!(cmd[4], config);
            assert_eq!(cmd[5], "--target");
            assert_eq!(cmd[6], "chrome_stable");
            assert_eq!(cmd[7], subcommand);
        }
        assert_eq!(plan.steps[3].cmd[8], "--no-smoke");

        // 不开 archive 时只剩 prepare + build
        let lean = plan_build(&BuildRequest {
            archive: false,
            ..request
        });
        assert_eq!(lean.steps.len(), 2);
    }

    /// 工具箱：按钮到子命令的映射（chplus 已按设计删除）。
    #[test]
    fn plan_tool_maps_buttons() {
        let workdir = temp_workdir("tool");
        let config = workdir
            .join("build")
            .join("selected.browser.json")
            .to_string_lossy()
            .into_owned();

        let inspect = plan_tool("inspect", &workdir, "C:\\pkg\\a.exe", "", "x64", true, true);
        assert_eq!(inspect.steps[0].cmd[3], "inspect-package");
        assert_eq!(inspect.steps[0].cmd[4], "C:\\pkg\\a.exe");
        assert_eq!(inspect.steps[0].cmd.last().unwrap(), "--json");

        let verify = plan_tool("verify", &workdir, "", "chrome_stable", "x64", false, true);
        assert_eq!(verify.steps[0].cmd[3], "--config");
        assert_eq!(verify.steps[0].cmd[4], config);
        assert_eq!(verify.steps[0].cmd.last().unwrap(), "--no-smoke");

        let resolve = plan_tool("resolve", &workdir, "", "chrome_stable", "x64", false, true);
        assert_eq!(resolve.steps[0].cmd[3], "resolve-upstream");
        assert!(resolve.steps[0].cmd.iter().any(|arg| arg == "--json"));

        // prepare：给目录是非法输入（gui_plan 的 gui_tool_prepare_hint）
        let prepare_dir = plan_tool(
            "prepare",
            &workdir,
            &workdir.to_string_lossy(),
            "chrome_stable",
            "x64",
            false,
            true,
        );
        assert_eq!(prepare_dir.error_key, "gui_tool_prepare_hint");
        let prepare_file = plan_tool(
            "prepare",
            &workdir,
            "C:\\pkg\\a.exe",
            "chrome_stable",
            "x64",
            false,
            true,
        );
        assert!(prepare_file.ok());
        assert!(prepare_file.steps[0].cmd.iter().any(|arg| arg == "--path"));

        // chplus 已删除：走 default 分支报 gui_need_path
        assert_eq!(
            plan_tool("chplus", &workdir, "", "chrome_stable", "x64", false, true).error_key,
            "gui_need_path"
        );
        assert_eq!(
            plan_tool("nope", &workdir, "", "", "x64", false, true).error_key,
            "gui_need_path"
        );
    }
}
