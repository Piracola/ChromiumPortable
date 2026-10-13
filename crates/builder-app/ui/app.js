/*
  ChromiumPortableBuilder 渲染逻辑（2026-10 重写版）。
  无框架、无构建步骤：唯一入口 window.__TAURI__（withGlobalTauri）。

  状态流：输入变化 → app_plan（纯预览）→ 渲染计划 → 开始构建 → app_start；
  引擎输出经 Rust 泵线程 80ms 批量推 app-event，这里只做追加渲染。
  文案集中在 STR 表（zh / en），与 engine GUI 文案无耦合。
*/

"use strict";

/* ── 文案 ─────────────────────────────────────────────── */

const STR = {
  zh: {
    tagline: "解包 + Chrome++ 注入 · 不安装、不启动浏览器",
    navBuild: "构建", navTools: "调试台",
    warnEngine: "同目录没有 portable-builder.exe，构建不可用。",
    buildTitle: "构建便携版",
    buildDesc: "把安装包变成随身携带的浏览器文件夹：解包 → 注入 Chrome++ → 可选打包 7z。",
    sectSource: "1 · 来源",
    modeOne: "本机安装包", modeFolder: "整个文件夹", modeOnline: "在线目标",
    installerList: "installers 文件夹", refresh: "刷新", openInstallers: "打开 installers",
    installerEmpty: "installers\\ 里还没有安装包",
    installerLabel: "或直接指定路径",
    folderLabel: "安装包目录", browseDir: "浏览文件夹…", browseFile: "浏览文件…",
    folderHint: "每个安装包各自出一份产物，互不影响。",
    targetLabel: "目标", targetHint: "目标来自目录表，由上游提供安装包。",
    urlLabel: "自定义直链（可选）",
    urlHint: "仅当目标没有公开安装包、或要指定具体版本时填写。",
    localLabel: "本地包（可选）", localHint: "自己准备的安装包，优先于直链。",
    sectOptions: "2 · 选项",
    archLabel: "架构", archiveLabel: "打包 7z",
    archiveHint: "打包并做一次静态校验；关掉只留解包目录。",
    sectOutput: "3 · 输出",
    workdirLabel: "工作目录", changeWorkdir: "更改…", open: "打开",
    workdirHint: "installers\\、build\\ 与 7zr.exe 都放在这里。",
    outputHint: "解包目录在 build\\release\\，7z 在 build\\assets\\。",
    openRelease: "打开 build\\release", openAssets: "打开 build\\assets",
    sectRun: "4 · 计划与执行",
    planIdle: "选好来源后，这里显示将要执行的引擎命令。",
    start: "开始构建", cancel: "取消",
    toolsTitle: "调试台",
    toolsDesc: "单步跑引擎命令：排查链路、更新目录表。",
    sectToolInput: "输入", sectToolCmds: "命令",
    toolPathLabel: "安装包或文件夹",
    toolPathHint: "识别安装包用文件，批量巡检用文件夹。",
    jsonOpt: "JSON 输出", noSmokeOpt: "跳过冒烟（静态校验）",
    toolInspect: "识别安装包", toolInspectDesc: "不运行安装包，解析结构并定位主程序。",
    toolResearch: "批量巡检", toolResearchDesc: "文件夹里每个安装包各识别一次。",
    toolPrepare: "生成单目标配置", toolPrepareDesc: "按所选目标写出 build\\selected.browser.json。",
    toolResolve: "解析上游地址", toolResolveDesc: "取目标最新安装包的直链与校验值。",
    toolVerify: "校验产物压缩包", toolVerifyDesc: "按 build\\selected.browser.json 校验产物。",
    run: "运行",
    logTitle: "运行日志", autoscroll: "自动滚动",
    logSave: "保存", logCopy: "复制", logClear: "清空",
    logLines: "{count} 行",
    statusIdle: "就绪",
    statusRunning: "正在执行：{title}",
    statusDone: "完成（{seconds} 秒）",
    statusFailed: "失败（退出码 {code}）",
    statusCancelled: "已取消",
    copied: "已复制到剪贴板",
    logSaved: "日志已保存：{path}",

    step_build_one: "构建单个安装包",
    step_build_folder: "构建文件夹里的全部安装包",
    step_prepare: "准备单目标配置",
    step_build_online: "在线构建",
    step_archive: "打包 7z",
    step_verify: "静态校验产物",
    step_tool_inspect: "识别安装包",
    step_tool_research: "批量巡检",
    step_tool_prepare: "生成单目标配置",
    step_tool_resolve: "解析上游地址",
    step_tool_verify: "校验产物压缩包",

    need_installer: "请先选择一个安装包。",
    need_folder: "请先填写安装包目录。",
    need_target: "请先选择一个在线目标。",
    need_path: "这个命令需要先填写安装包或文件夹路径。",
    tool_prepare_dir: "生成单目标配置需要文件路径，不是文件夹。",
    engine_missing: "同目录没有 portable-builder.exe。",
    busy: "已有任务在运行，请等它结束或先取消。",
    workdir_invalid: "工作目录不存在。",
    unknown_kind: "未知的打开目标。",
    noRuntime: "预览模式：在 ChromiumPortableBuilder 窗口里才能执行。",
    filter_installers: "安装包",
  },
  en: {
    tagline: "Extract + Chrome++ injection · never installs or launches a browser",
    navBuild: "Build", navTools: "Debug",
    warnEngine: "portable-builder.exe not found next to this app; building is unavailable.",
    buildTitle: "Build a portable browser",
    buildDesc: "Turn an installer into a browser folder you can carry around: extract, inject Chrome++, optionally package.",
    sectSource: "1 · Source",
    modeOne: "Local installer", modeFolder: "Whole folder", modeOnline: "Online target",
    installerList: "installers folder", refresh: "Refresh", openInstallers: "Open installers",
    installerEmpty: "No installers in installers\\ yet",
    installerLabel: "Or point at a path",
    folderLabel: "Installer folder", browseDir: "Browse folder…", browseFile: "Browse file…",
    folderHint: "Every installer produces its own output, independently.",
    targetLabel: "Target", targetHint: "Targets come from the catalog; upstream supplies the installer.",
    urlLabel: "Custom URL (optional)",
    urlHint: "Only when the target has no public installer, or you need a specific build.",
    localLabel: "Local package (optional)", localHint: "A package you already have; it wins over the URL.",
    sectOptions: "2 · Options",
    archLabel: "Architecture", archiveLabel: "Create 7z",
    archiveHint: "Packages and runs one static verify; off keeps only the unpacked folder.",
    sectOutput: "3 · Output",
    workdirLabel: "Working directory", changeWorkdir: "Change…", open: "Open",
    workdirHint: "Holds installers\\, build\\ and 7zr.exe.",
    outputHint: "Unpacked builds go to build\\release\\, 7z packages to build\\assets\\.",
    openRelease: "Open build\\release", openAssets: "Open build\\assets",
    sectRun: "4 · Plan and run",
    planIdle: "Pick a source and the engine commands appear here.",
    start: "Start build", cancel: "Cancel",
    toolsTitle: "Debug console",
    toolsDesc: "Run one engine command at a time to debug a chain or refresh the catalog.",
    sectToolInput: "Input", sectToolCmds: "Commands",
    toolPathLabel: "Installer or folder",
    toolPathHint: "Inspect wants a file; batch research wants a folder.",
    jsonOpt: "JSON output", noSmokeOpt: "Skip smoke (static verify)",
    toolInspect: "Inspect an installer", toolInspectDesc: "Parse the package without running it; locate the main executable.",
    toolResearch: "Batch research", toolResearchDesc: "Identify every installer in a folder in one pass.",
    toolPrepare: "Write single-target config", toolPrepareDesc: "Writes build\\selected.browser.json for the target.",
    toolResolve: "Resolve upstream URL", toolResolveDesc: "Fetch the latest installer URL and digest.",
    toolVerify: "Verify an archive", toolVerifyDesc: "Verify the artifact described by build\\selected.browser.json.",
    run: "Run",
    logTitle: "Log", autoscroll: "Auto-scroll",
    logSave: "Save", logCopy: "Copy", logClear: "Clear",
    logLines: "{count} lines",
    statusIdle: "Ready",
    statusRunning: "Running: {title}",
    statusDone: "Done ({seconds}s)",
    statusFailed: "Failed (exit code {code})",
    statusCancelled: "Cancelled",
    copied: "Copied to the clipboard",
    logSaved: "Log saved: {path}",

    step_build_one: "Build one installer",
    step_build_folder: "Build every installer in the folder",
    step_prepare: "Prepare single-target config",
    step_build_online: "Build from upstream",
    step_archive: "Create 7z archive",
    step_verify: "Static verify",
    step_tool_inspect: "Inspect an installer",
    step_tool_research: "Batch research",
    step_tool_prepare: "Write single-target config",
    step_tool_resolve: "Resolve upstream URL",
    step_tool_verify: "Verify an archive",

    need_installer: "Pick an installer package first.",
    need_folder: "Set the installer folder first.",
    need_target: "Pick an online target first.",
    need_path: "This command needs an installer or folder path first.",
    tool_prepare_dir: "Single-target config needs a file path, not a folder.",
    engine_missing: "portable-builder.exe not found next to this app.",
    busy: "A task is already running — wait for it or cancel it first.",
    workdir_invalid: "The working directory does not exist.",
    unknown_kind: "Unknown open target.",
    noRuntime: "Preview only: run inside ChromiumPortableBuilder.",
    filter_installers: "Installers",
  },
};

let lang = "zh";
let snap = null;
let plan = null;
let running = false;
let logCount = 0;
let planTimer = 0;

const $ = (id) => document.getElementById(id);

function fmt(template, vars) {
  return template.replace(/\{(\w+)\}/g, (_, k) =>
    vars && k in vars ? String(vars[k]) : `{${k}}`
  );
}

function tr(key, vars) {
  const table = STR[lang];
  const t = key in table ? table[key] : key in STR.zh ? STR.zh[key] : key;
  return vars ? fmt(String(t), vars) : t;
}

/* ── Tauri API 访问 ───────────────────────────────────── */

function tauri() { return window.__TAURI__; }

function invoke(cmd, args) {
  return tauri().core.invoke(cmd, args);
}

/* ── 渲染 ─────────────────────────────────────────────── */

function applyLang() {
  document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = tr(el.getAttribute("data-i18n"));
  });
  localStorage.setItem("cpb-lang", lang);
  refreshPlanSoon();
}

function setStatus(kind, key, vars) {
  const dot = $("status-dot");
  dot.className = kind === "run" ? "dot run" : kind === "ok" ? "dot ok" : kind === "fail" ? "dot fail" : "dot";
  $("status-text").textContent = tr(key, vars);
}

function fillTargets() {
  const options = (snap && snap.targets ? snap.targets : [])
    .map((t) => `<option value="${escapeAttr(t.id)}">${escapeHtml(t.label)}</option>`)
    .join("");
  $("target-build").innerHTML = options;
  $("target-tool").innerHTML = options;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[c]));
}
function escapeAttr(s) { return escapeHtml(s); }

function renderInstallers() {
  const list = $("installer-list");
  const names = snap && snap.installers ? snap.installers : [];
  $("installer-empty").hidden = names.length > 0;
  list.innerHTML = names
    .map((n) => `<button type="button" class="pick-item" role="option" data-name="${escapeAttr(n)}">${escapeHtml(n)}</button>`)
    .join("");
  list.querySelectorAll(".pick-item").forEach((el) => {
    el.addEventListener("click", () => {
      list.querySelectorAll(".pick-item").forEach((o) => o.removeAttribute("aria-selected"));
      el.setAttribute("aria-selected", "true");
      $("in-installer").value = "installers\\" + el.dataset.name;
      refreshPlanSoon();
    });
  });
}

function quoteArg(arg) {
  return arg === "" || /\s/.test(arg) ? `"${arg}"` : arg;
}

function renderPlan() {
  const ol = $("plan");
  const empty = $("plan-empty");
  const start = $("btn-start");
  ol.innerHTML = "";
  start.disabled = true;

  if (!plan || (!plan.ok && !plan.errorKey)) {
    empty.hidden = false;
    return;
  }
  if (!plan.ok) {
    empty.hidden = false;
    empty.textContent = plan.errorKey ? tr(plan.errorKey) : "";
    return;
  }
  empty.hidden = true;
  plan.steps.forEach((step, i) => {
    const li = document.createElement("li");
    li.dataset.index = String(i);
    const num = document.createElement("span");
    num.className = "plan-num";
    num.textContent = String(i + 1);
    const main = document.createElement("span");
    main.className = "plan-main";
    const name = document.createElement("span");
    name.className = "plan-name";
    name.textContent = tr(step.titleKey);
    const cmd = document.createElement("span");
    cmd.className = "plan-cmd";
    cmd.textContent = "portable-builder " + step.args.map(quoteArg).join(" ");
    main.append(name, cmd);
    const st = document.createElement("span");
    st.className = "plan-st";
    li.append(num, main, st);
    ol.appendChild(li);
  });
  if (!running) start.disabled = false;
}

function markStep(index, state) {
  const li = $("plan").querySelector(`li[data-index="${index}"]`);
  if (!li) return;
  li.className = state;
}

function appendLog(lines, cls) {
  const el = $("log");
  for (const line of lines) {
    const span = document.createElement("span");
    if (cls) span.className = cls;
    span.textContent = line + "\n";
    el.appendChild(span);
  }
  logCount += lines.length;
  $("log-count").textContent = tr("logLines", { count: logCount });
  while (el.childNodes.length > 5000) el.removeChild(el.firstChild);
  if ($("in-autoscroll").checked) el.scrollTop = el.scrollHeight;
}

/* ── 状态装配 ─────────────────────────────────────────── */

function mode() {
  return document.querySelector('input[name="mode"]:checked').value;
}

function planRequest() {
  return {
    mode: mode(),
    tool: null,
    arch: $("in-arch").value,
    archive: $("in-archive").checked,
    installer: $("in-installer").value.trim(),
    folder: $("in-folder").value.trim(),
    target: $("target-build").value,
    url: $("in-url").value.trim(),
    localPath: $("in-local").value.trim(),
    jsonOutput: false,
    noSmoke: true,
  };
}

function toolRequest(tool) {
  return {
    mode: "one",
    tool,
    arch: $("in-arch").value,
    archive: false,
    installer: $("in-tool-path").value.trim(),
    folder: "",
    target: $("target-tool").value,
    url: "",
    localPath: "",
    jsonOutput: $("in-tool-json").checked,
    noSmoke: $("in-tool-nosmoke").checked,
  };
}

function refreshPlanSoon() {
  clearTimeout(planTimer);
  planTimer = setTimeout(refreshPlan, 120);
}

async function refreshPlan() {
  if (running) return;
  if (!tauri()) {
    plan = { ok: false, errorKey: "noRuntime", steps: [] };
    renderPlan();
    return;
  }
  try {
    plan = await invoke("app_plan", { req: planRequest() });
  } catch (e) {
    plan = { ok: false, errorKey: String(e), steps: [] };
  }
  renderPlan();
}

async function loadSnapshot() {
  if (!tauri()) return;
  snap = await invoke("app_snapshot", {});
  $("in-workdir").value = snap.workdir;
  $("warn-engine").hidden = !!snap.engineFound;
  fillTargets();
  renderInstallers();
}

function setRunning(on) {
  running = on;
  $("btn-start").disabled = on || !plan || !plan.ok;
  $("btn-cancel").hidden = !on;
  document.querySelectorAll(".btn-run").forEach((b) => (b.disabled = on));
  if (!on) refreshPlanSoon();
}

/* ── 事件绑定 ─────────────────────────────────────────── */

function bindEvents() {
  $("lang").addEventListener("change", () => {
    lang = $("lang").value;
    applyLang();
  });

  document.querySelectorAll('input[name="page"]').forEach((r) => {
    r.addEventListener("change", () => {
      const page = r.value;
      $("page-build").hidden = page !== "build";
      $("page-tools").hidden = page !== "tools";
      if (page === "build") refreshPlanSoon();
    });
  });

  document.querySelectorAll('input[name="mode"]').forEach((r) => {
    r.addEventListener("change", () => {
      const m = mode();
      $("source-one").hidden = m !== "one";
      $("source-folder").hidden = m !== "folder";
      $("source-online").hidden = m !== "online";
      if (m === "folder" && $("in-folder").value.trim() === "") {
        $("in-folder").value = "installers";
      }
      refreshPlanSoon();
    });
  });

  ["in-installer", "in-folder", "in-url", "in-local"].forEach((id) => {
    $(id).addEventListener("input", refreshPlanSoon);
  });
  ["in-arch", "target-build"].forEach((id) => {
    $(id).addEventListener("change", refreshPlanSoon);
  });
  $("in-archive").addEventListener("change", refreshPlanSoon);

  $("btn-refresh").addEventListener("click", async () => {
    await loadSnapshot();
    refreshPlanSoon();
  });

  $("btn-open-installers").addEventListener("click", () => invoke("app_open", { kind: "installers" }).catch(() => {}));
  $("btn-open-workdir").addEventListener("click", () => invoke("app_open", { kind: "workdir" }).catch(() => {}));
  $("btn-open-release").addEventListener("click", () => invoke("app_open", { kind: "release" }).catch(() => {}));
  $("btn-open-assets").addEventListener("click", () => invoke("app_open", { kind: "assets" }).catch(() => {}));

  $("btn-browse-folder").addEventListener("click", async () => {
    const picked = await tauri().dialog.open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) {
      $("in-folder").value = picked;
      refreshPlanSoon();
    }
  });

  $("btn-change-workdir").addEventListener("click", async () => {
    const picked = await tauri().dialog.open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) {
      try {
        snap = await invoke("app_workdir", { path: picked });
        $("in-workdir").value = snap.workdir;
        fillTargets();
        renderInstallers();
        refreshPlanSoon();
      } catch (e) {
        setStatus("fail", String(e).replace(/^"|"$/g, ""));
      }
    }
  });

  $("btn-start").addEventListener("click", async () => {
    if (running) return;
    try {
      await invoke("app_start", { req: planRequest() });
      setRunning(true);
      unfoldLog();
      setStatus("run", "statusRunning", { title: tr("start") });
    } catch (e) {
      setStatus("fail", String(e).replace(/^"|"$/g, ""));
    }
  });

  $("btn-cancel").addEventListener("click", async () => {
    await invoke("app_cancel", {}).catch(() => {});
  });

  document.querySelectorAll(".btn-run").forEach((b) => {
    b.addEventListener("click", async () => {
      if (running) return;
      try {
        await invoke("app_start", { req: toolRequest(b.dataset.tool) });
        setRunning(true);
        unfoldLog();
        setStatus("run", "statusRunning", { title: tr("step_" + b.dataset.tool) });
      } catch (e) {
        setStatus("fail", String(e).replace(/^"|"$/g, ""));
      }
    });
  });

  $("btn-tool-file").addEventListener("click", async () => {
    const picked = await tauri().dialog.open({
      multiple: false,
      filters: [{ name: tr("filter_installers"), extensions: ["exe", "msi", "7z", "zip", "rar", "cab"] }],
    });
    if (typeof picked === "string" && picked) $("in-tool-path").value = picked;
  });

  $("btn-tool-dir").addEventListener("click", async () => {
    const picked = await tauri().dialog.open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) $("in-tool-path").value = picked;
  });

  $("btn-log-save").addEventListener("click", async () => {
    try {
      const path = await invoke("app_save_log", { text: $("log").textContent });
      if (path) setStatus("ok", "logSaved", { path });
    } catch (e) {
      setStatus("fail", String(e).replace(/^"|"$/g, ""));
    }
  });

  $("btn-log-copy").addEventListener("click", async () => {
    await tauri().clipboardManager.writeText($("log").textContent);
    setStatus("ok", "copied");
  });

  $("btn-log-clear").addEventListener("click", () => {
    $("log").textContent = "";
    logCount = 0;
    $("log-count").textContent = tr("logLines", { count: 0 });
  });

  // 日志折叠：折叠时把空间还给上面的表单；开始构建时自动展开
  $("btn-log-fold").addEventListener("click", () => {
    const box = $("logbox");
    const folded = box.classList.toggle("folded");
    $("btn-log-fold").setAttribute("aria-expanded", String(!folded));
  });

  const unfoldLog = () => {
    $("logbox").classList.remove("folded");
    $("btn-log-fold").setAttribute("aria-expanded", "true");
    // 日志在全部选项的下方，跑起来时把它滚进视野（真在窗外才动，别乱跳）
    $("logbox").scrollIntoView({ block: "nearest" });
  };

  // 自绘窗口钮（原生标题栏已关）
  const win = () => tauri().window.getCurrentWindow();
  $("win-min").addEventListener("click", () => win().minimize());
  $("win-max").addEventListener("click", () => win().toggleMaximize());
  $("win-close").addEventListener("click", () => win().close());
}

/* ── 事件泵消费 ───────────────────────────────────────── */

function bindAppEvents() {
  tauri().event.listen("app-event", (ev) => {
    const p = ev.payload;
    if (p.type === "line") {
      appendLog(p.lines, p.err ? "ln-err" : p.cmd ? "ln-cmd" : "");
    } else if (p.type === "stepStart") {
      markStep(p.index, "running");
      setStatus("run", "statusRunning", { title: tr(p.titleKey) });
    } else if (p.type === "stepEnd") {
      markStep(p.index, p.code === 0 ? "done" : "failed");
    } else if (p.type === "done") {
      setRunning(false);
      if (p.cancelled) setStatus("", "statusCancelled");
      else if (p.failed) setStatus("fail", "statusFailed", { code: p.code });
      else setStatus("ok", "statusDone", { seconds: p.seconds });
      loadSnapshot().catch(() => {});
    }
  });
}

/* ── 启动 ─────────────────────────────────────────────── */

window.addEventListener("DOMContentLoaded", async () => {
  $("in-autoscroll").checked = true;
  lang = localStorage.getItem("cpb-lang")
    || (navigator.language && navigator.language.startsWith("zh") ? "zh" : "en");
  $("lang").value = lang;
  applyLang();
  bindEvents();
  bindAppEvents();
  try {
    await loadSnapshot();
  } catch (e) {
    appendLog([String(e)], "ln-err");
  }
  await refreshPlan();
  setStatus("", "statusIdle");
});
