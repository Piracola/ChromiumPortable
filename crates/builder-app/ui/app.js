(function () {
  "use strict";

  /* ========================================================================= *
   * ChromiumPortableBuilder GUI — app.js（渲染层脚手架）
   * 提取自 portable_builder/web_ui.py（Wave1-E，Python→Rust 迁移）。
   *
   * 架构决策（docs/MIGRATION_RUST_TAURI.md §2.2 决策 B）：
   *   前端渲染层从「Rust/Python 拼 HTML」改为「JS 拿状态渲染」。命令返回
   *   状态快照 JSON + 一次性下发的语言包，JS 消费后填充 DOM 挂载点。
   *
   * 本文件目前的范围（Wave1-E = 脚手架）：
   *   1. invoke() 垫片  —— Tauri 环境走 window.__TAURI__.core.invoke，
   *      无 __TAURI__ 时（Playwright 无头测试，迁移文档 §7）走 MOCK 层；
   *   2. window.__MOCK_API__ —— Controller 状态快照的占位数据，
   *      字段名与 scripts/gui.py 的 Controller.state()/snapshot_for_render()
   *      完全一致（字段名是 Wave4 的契约，见文件底部契约清单）；
   *   3. render() 入口 —— 消费 snapshot+strings 填充挂载点。
   *      Wave1-E 只移植了两个简单的渲染函数（renderBanner / renderLogHead）
   *      以确立移植模式，其余是 TODO(Wave4)。
   *
   * 设计规范：docs/ui-architecture.md（唯一主角、三级表面、四级字号、五状态）。
   * ========================================================================= */

  /* ------------------------------------------------------------------ */
  /* 常量（web_ui.py 顶部原样平移）                                      */
  /* ------------------------------------------------------------------ */

  /* 工具箱六件套的字形。刻意用几何符号而不是 emoji——emoji 要额外的字体文件，
   * 而且在 Windows 上会被渲染成彩色字形，跟整套中性色打不上。
   * （源：portable_builder/web_ui.py 的 TOOL_GLYPHS，勿改值。） */
  var TOOL_GLYPHS = {
    inspect: "\u25CE",   /* ◎ */
    research: "\u25A4",  /* ▤ */
    prepare: "\u2261",   /* ≡ */
    resolve: "\u2193",   /* ↓ */
    chplus: "\u21BB",    /* ↻ */
    verify: "\u2713"     /* ✓ */
  };
  var TOOL_ORDER = ["inspect", "research", "prepare", "resolve", "chplus", "verify"];

  /* ------------------------------------------------------------------ */
  /* {field} 占位替换 —— 语义文档（原样平移，缺失键显示裸 key）           */
  /* ------------------------------------------------------------------ */
  /* 取一条已翻译的文案。缺失时回落到 key 本身而不是抛异常。
   *
   * 界面不该因为少一条语言包就开不了窗口——所以这里宁可显示裸 key，
   * 也要让排版保持可读（否则一堆方框更难排查）。
   *
   * 占位符语义：模板里的 "{name}" 用 fields.name 逐个替换；
   * 语言包里**没有**的键原样留在输出里（谁缺谁现形，便于排查）。
   * 源：portable_builder/web_ui.py 的 _t()。行为差异只有一处：
   * JS 里传入的是普通对象而非 kwargs，其余语义 1:1。
   */

  function t(strings, key, fields) {
    var text = Object.prototype.hasOwnProperty.call(strings || {}, key)
      ? strings[key] : key;
    if (fields) {
      Object.keys(fields).forEach(function (name) {
        text = text.split("{" + name + "}").join(String(fields[name]));
      });
    }
    return text;
  }

  /* HTML 转义（源：web_ui.py 的 _e()，escape(value, quote=True)）。 */
  function e(value) {
    var s = value == null ? "" : String(value);
    return s
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/\"/g, "&quot;")
      .replace(/'/g, "&#x27;");
  }

  /* ------------------------------------------------------------------ */
  /* invoke 垫片                                                         */
  /* ------------------------------------------------------------------ */
  /* async invoke(cmd, args)：
   *   a) Tauri（window.__TAURI__ 存在，withGlobalTauri: true）：
   *      window.__TAURI__.core.invoke(cmd, args)；
   *   b) 无 __TAURI__（file:// 直开 / Playwright 无头）：
   *      走 window.__MOCK_API__，让页面脱离后端就能打开、能测
   *      （迁移文档 §7：Playwright 无头测试通过注入 mock invoke 量 DOM 几何）。
   *
   * 命令面（Wave4 的 #[tauri::command] 映射表，docs §6.2）：
   *   snapshot / set_lang / set_input / refresh / start / run_tool / cancel /
   *   pick / open_path / log_* —— MOCK 层只实现 snapshot 与 strings 读取，
   *   其余命令返回空快照并在控制台告警（脚手架阶段允许）。
   */

  function invokeTauri(cmd, args) {
    return window.__TAURI__.core.invoke(cmd, args || {});
  }

  function invokeMock(cmd, args) {
    var mock = window.__MOCK_API__ || {};
    switch (cmd) {
      case "snapshot":
      case "strings":
        return Promise.resolve({
          snapshot: mock.snapshot || {},
          strings: mock.strings || {}
        });
      default:
        /* Wave4 之前没有真的后端：显式拒绝而不是静默吞掉，
         * 错误必须浮出 UI 横幅（迁移文档 §6.4 的验收精神）。 */
        console.warn("[app.js] mock invoke: unsupported command " + cmd);
        return Promise.reject(new Error("mock invoke: unsupported command " + cmd));
    }
  }

  async function invoke(cmd, args) {
    if (window.__TAURI__ && window.__TAURI__.core
        && typeof window.__TAURI__.core.invoke === "function") {
      return invokeTauri(cmd, args);
    }
    return invokeMock(cmd, args);
  }

  /* ------------------------------------------------------------------ */
  /* MOCK 层：Controller 状态快照占位                                    */
  /* ------------------------------------------------------------------ */
  /* 字段名与 scripts/gui.py 的 Controller.state() / snapshot_for_render()
   * 完全一致——字段名是 Wave4（Rust Controller）的契约，改名即破坏契约。
   * 值是占位符：静态页测试只关心形状，不关心真值。
   *
   * snapshot 维度的字段见文件底部注释「状态字段契约」。
   */

  var MOCK_SNAPSHOT = {
    /* --- Controller.__init__ 的普通值字典（gui.py:187 vars）--- */
    vars: {
      mode: "one",            /* one | folder | online */
      installer: "",
      folder: "",
      target: "chrome_stable",
      arch: "x64",
      archive: true,
      url: "",
      local: "",
      token: "",
      tool_path: "",
      tool_target: "chrome_stable",
      tool_arch: "x64",
      tool_json: false,
      autoscroll: true,
      workdir: "C:\Users\you\ChromiumPortable"
    },

    /* --- Controller.state() 的派生字段（gui.py:271）--- */
    lang: "zh-CN",
    catalog_choices: [
      ["brave_stable", "Brave"],
      ["vivaldi_stable", "Vivaldi"],
      ["opera_stable", "Opera"],
      ["thorium_stable", "Thorium"],
      ["cse360_stable", "360\u6781\u901F\u6D4F\u89C8\u5668X"],
      ["chrome_stable", "Chrome++"],
      ["chrome_beta", "Chrome++ Beta"],
      ["edge_stable", "Microsoft Edge"],
      ["helium_stable", "Helium"]
    ],
    architectures: ["x64", "x86", "arm64"],
    installer_choices: [
      /* {name, path, size, size_text} —— size 是字节数（int），
       * size_text 是后端算好的显示串（gui_core.format_size），页面只负责摆放 */
    ],
    plan: {
      steps: [],              /* 已翻译的步骤标题列表（渲染用） */
      raw_steps: [],          /* [{title: key, cmd: [..]}] —— 调试用，不直接渲染 */
      ok: false,
      error_key: "",          /* 非空时页面就地显示横幅（IA §5 不弹窗） */
      error_detail: ""
    },
    banner: { text: "", tone: "info", detail: "" },
    status: { text: "", tone: "idle" },
    result: "",
    task: { title: "", meta: "" },
    elapsed: "",
    log: { lines: [], open: false },
    running: false,
    cancel_armed: false,
    artifact: { release: "", assets: "" }
  };

  /* 语言包占位：真实 wizard.json 由 build/dev 流程复制到 ui/locales/ 后加载
   * （Tauri 侧 include_str! 内嵌后经 invoke('strings') 下发）。
   * 这里只放渲染冒烟需要的最小集合，值取自 scripts/locales/wizard.json。 */
  var MOCK_STRINGS = {
    gui_title: "ChromiumPortable \u4FBF\u6437\u6784\u5EFA\u5668",
    gui_subtitle: "\u9759\u6001\u89E3\u5305 + Chrome++ \u6CE8\u5165\uFF0C\u4E0D\u5B89\u88C5\u3001\u4E0D\u542F\u52A8\u6D4F\u89C8\u5668",
    gui_nav_build: "\u6784\u5EFA",
    gui_nav_tools: "\u5DE5\u5177\u7BB1",
    gui_status_idle: "\u5C31\u7EEA",
    gui_section_flow: "\u6267\u884C\u8BA1\u5212",
    gui_start: "\u5F00\u59CB\u6784\u5EFA",
    gui_cancel: "\u53D6\u6D88",
    gui_log: "\u8FD0\u884C\u65E5\u5FD7",
    gui_log_lines: "{count} \u884C",
    gui_log_expand: "\u5C55\u5F00\u65E5\u5FD7",
    gui_log_collapse: "\u6536\u8D77\u65E5\u5FD7",
    gui_log_autoscroll: "\u81EA\u52A8\u6EDA\u52A8",
    gui_log_save: "\u4FDD\u5B58\u65E5\u5FD7",
    gui_log_copy: "\u590D\u5236\u65E5\u5FD7",
    gui_log_clear: "\u6E05\u7A7A\u65E5\u5FD7",
    gui_log_empty: "\u6784\u5EFA\u65E5\u5FD7\u4F1A\u5B9E\u65F6\u663E\u793A\u5728\u8FD9\u91CC\u3002",
    gui_need_installer: "\u8BF7\u9009\u62E9\u4E00\u4E2A\u5B89\u88C5\u5305\u3002"
  };

  window.__MOCK_API__ = {
    snapshot: MOCK_SNAPSHOT,
    strings: MOCK_STRINGS
  };

  /* ------------------------------------------------------------------ */
  /* 渲染模式：本文件确立的移植模式（Wave4 照此展开）                     */
  /* ------------------------------------------------------------------ */
  /* 规则（每一条都来自对 web_ui.py 渲染函数的平移）：
   *   1. 挂载点 <div id="X" data-mount="X"></div> 只在 index.html 里出现一次；
   *   2. 渲染函数 render*(host, snapshot, strings) 只写 host 内部
   *      （textContent / createElement，必要时 innerHTML + e()），
   *      不碰挂载点以外的 DOM；
   *   3. 文案一律经 t(strings, key, fields) 取得——缺失键显示裸 key；
   *   4. 数据来源只有 snapshot；渲染函数不做 IO、不发 invoke。
   */

  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  }

  /* ---- 已移植 1/2：renderBanner —— 源：web_ui.py render() 里的 banner 分支
   * <div class="banner warn" id="banner"><span class="mark">!</span>
   *     <span>{text}</span><span class="detail">{detail}</span></div>
   * plan.error_key 非空时显示（tone 固定 warn），文案经 t() 缺键回落裸 key。
   */
  function renderBanner(snapshot, strings) {
    var host = document.getElementById("banner");
    if (!host) return;
    var plan = (snapshot && snapshot.plan) || {};
    var key = plan.error_key || "";
    var detail = plan.error_detail || "";
    host.textContent = "";
    if (!key) { host.hidden = true; return; }
    host.hidden = false;
    host.className = "banner warn";
    host.appendChild(el("span", "mark", "!"));
    host.appendChild(el("span", "", t(strings, key)));
    if (detail) host.appendChild(el("span", "detail", detail));
  }

  /* ---- 已移植 2/2：renderLogHead —— 源：web_ui.py _deck() 的日志折叠头部。
   * data-tpl 里的 {count} 由 t() 的占位语义填充（缺失键显示裸 key）；
   * gui_log_empty 语义平移：空日志时预填一条占位行并清零计数。
   */
  function renderLogHead(snapshot, strings) {
    var host = document.getElementById("loghead");
    if (!host) return;
    var log = (snapshot && snapshot.log) || { lines: [], open: false };
    host.textContent = "";

    var toggle = el("button", "btn");
    toggle.type = "button";
    toggle.id = "logtoggle";
    toggle.setAttribute("data-expand", t(strings, "gui_log_expand"));
    toggle.setAttribute("data-collapse", t(strings, "gui_log_collapse"));
    toggle.textContent = t(strings, "gui_log_expand");
    host.appendChild(toggle);

    host.appendChild(el("span", "t", t(strings, "gui_log")));

    var n = el("span", "n");
    n.id = "logn";
    n.setAttribute("data-tpl", t(strings, "gui_log_lines", { count: "{count}" }));
    n.textContent = t(strings, "gui_log_lines", { count: String(log.lines.length) });
    host.appendChild(n);

    host.appendChild(el("span", "grow", ""));

    var auto = el("button", "btn", t(strings, "gui_log_autoscroll"));
    auto.type = "button";
    auto.id = "autoscroll";
    host.appendChild(auto);

    ["gui_log_save", "gui_log_copy", "gui_log_clear"].forEach(function (k) {
      var b = el("button", "btn", t(strings, k));
      b.type = "button";
      b.setAttribute("data-action", k.replace("gui_log_", ""));
      host.appendChild(b);
    });

    var box = document.getElementById("logbox");
    if (box) {
      box.hidden = !log.open;
      box.textContent = "";
      var lines = log.lines;
      if (!lines.length) {
        var empty = el("div", "l-dim", t(strings, "gui_log_empty"));
        empty.setAttribute("data-log-empty", "");
        box.appendChild(empty);
      } else {
        lines.forEach(function (line) { box.appendChild(el("div", "", line)); });
      }
    }
  }

  /* TODO(Wave4): port remaining Python HTML builders from web_ui.py, in this
   * order (they map 1:1 onto the mount points in index.html):
   *   renderTopbar(s)          <- _topbar(s)        -> [data-mount=topbar]
   *   renderBuildPage(s, args) <- _build_page()    -> [data-mount=page-build]
   *     renderSegmented(...)   <- _segmented() / _field() / _row() / _text() / _select()
   *     renderModeBlock(...)   <- _mode_block()
   *   renderToolsPage(s, args) <- _tools_page()    -> [data-mount=page-tools]
   *   renderDeck(s)            <- _deck()          -> [data-mount=deck]
   *   renderInstallers(items)  <- JS renderInstallers() in web_ui.py
   *   renderRail(steps)        <- window.setRail() in web_ui.py
   * Render helpers must keep TOOL_GLYPHS / TOOL_ORDER verbatim and route all
   * strings through t(); args come only from snapshot (see state-field contract
   * at the bottom of this file). */

  /* ------------------------------------------------------------------ */
  /* render() 入口                                                       */
  /* ------------------------------------------------------------------ */

  async function render() {
    var data = await invoke("snapshot");
    var snapshot = data.snapshot || {};
    var strings = data.strings || {};
    renderBanner(snapshot, strings);
    renderLogHead(snapshot, strings);
    /* TODO(Wave4): renderTopbar / renderBuildPage / renderToolsPage / renderDeck
     * once their render helpers are ported (see the port-order list above). */
  }

  window.__cpb = { render: render, invoke: invoke, t: t, e: e };
  window.__cpb.TOOL_GLYPHS = TOOL_GLYPHS;
  window.__cpb.TOOL_ORDER = TOOL_ORDER;

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", render);
  } else {
    render();
  }

  /* ==================================================================== *
   * 状态字段契约（Wave1-E 从 scripts/gui.py 提取，Wave4 必须逐字段保留）  *
   * ==================================================================== *
   * snapshot —— Controller.state()（gui.py:271），Wave4 的 invoke('snapshot')
   * 直接返回这份 JSON：
   *   lang: str                          "zh-CN" | "en"
   *   strings: dict[str, str]            语言包（渲染前一次性下发）
   *   vars: dict                         见下方 vars 明细
   *   catalog_choices: list[(id, display_name)]   在线目标（gui_core.target_choices）
   *   architectures: list[str]           core.ARCHITECTURES = ["x64","x86","arm64"]
   *   installer_choices: list[{name: str, path: str, size: int, size_text: str}]
   *   plan: {steps: list[str], raw_steps: list[{title: str, cmd: list[str]}],
   *          ok: bool, error_key: str, error_detail: str}
   *   banner: {text: str, tone: str, detail: str}    tone: info|warn|err|ok
   *   status: {text: str, tone: str}                 tone: idle|run|ok|warn|err
   *   result: str                        构建完成后的产物落点提示
   *   task: {title: str, meta: str}      当前任务（meta = "gui_task_step" 渲染串）
   *   elapsed: str                       计时显示串
   *   log: {lines: list[str], open: bool}          内存只留最近 4000 行
   *   running: bool
   *   cancel_armed: bool
   *   artifact: {release: str, assets: str}        workdir/build/{release,assets}
   *
   * vars —— Controller.__init__（gui.py:187）：
   *   mode: str          "one" | "folder" | "online"
   *   installer: str     安装包路径（mode=one）
   *   folder: str        安装包目录（mode=folder）
   *   target: str        在线目标 id，默认 "chrome_stable"（mode=online）
   *   arch: str          "x64" | "x86" | "arm64"
   *   archive: bool      是否生成 7z 分发包
   *   url: str           直链（mode=online 可选）
   *   local: str         本地包路径（mode=online 可选；JS 侧 collect() 键为
   *                      "local_path"，Controller._JS_TO_VAR 映射回 "local"）
   *   token: str         GitHub token（工具箱）
   *   tool_path: str     工具箱安装包/目录
   *   tool_target: str   工具箱目标 id
   *   tool_arch: str     "x64" | "x86" | "arm64"
   *   tool_json: bool    工具箱 JSON 输出
   *   autoscroll: bool   日志自动滚动
   *   workdir: str       工作目录
   *
   * 事件（runner.rs 平移 gui_runner.py 的队列协议）：
   *   ("log",  text, tag)                    tag: ""|warn|err|ok|cmd|head
   *   ("step", index, total, title)          index 从 1 计
   *   ("done", code, elapsed)                code 0=成功 / -2=取消 / 其他=失败
   * ==================================================================== */
})();