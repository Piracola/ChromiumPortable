(function () {
  "use strict";

  /* ========================================================================= *
   * ChromiumPortableBuilder GUI — app.js（渲染层 + 事件接线）
   * 提取自 portable_builder/web_ui.py（Wave1-E 脚手架，M4b 补齐渲染函数）。
   *
   * 架构决策（docs/MIGRATION_RUST_TAURI.md §2.2 决策 B）：
   *   前端渲染层从「Rust/Python 拼 HTML」改为「JS 拿状态渲染」。命令返回
   *   状态快照 JSON + 一次性下发的语言包，JS 消费后填充 DOM 挂载点。
   *
   * 本文件三部分：
   *   1. invoke() 垫片 —— Tauri 环境走 window.__TAURI__.core.invoke，
   *      无 __TAURI__ 时（file:// 直开 / Playwright 无头测试，迁移文档 §7）
   *      走 MOCK 层；
   *   2. window.__MOCK_API__ —— Controller 状态快照的占位数据，字段名与
   *      scripts/gui.py 的 Controller.state() 完全一致（Wave4 的契约，见文件
   *      底部「状态字段契约」）；
   *   3. 渲染函数 render*() —— web_ui.py 的 HTML 生成函数逐个平移为 DOM 构建
   *      函数（对照表见文件底部），外加 data-action / data-bind 事件接线。
   *
   * 移植模式（renderBanner / renderLogHead 是范例）：
   *   a) 渲染函数只写 host 内部，不碰挂载点以外的 DOM；
   *   b) 文案一律经 t(strings, key, fields)，缺失键回落裸 key；
   *   c) 数据只来自 snapshot，渲染阶段不做 IO、不发 invoke；
   *   d) HTML 注入内容不需要 e()——DOM 构建走 textContent，压根没有字符串拼接。
   *
   * 设计规范：docs/ui-architecture.md（唯一主角、三级表面、四级字号、五状态）。
   * ========================================================================= */

  /* ------------------------------------------------------------------ */
  /* 常量（web_ui.py 顶部平移）                                          */
  /* ------------------------------------------------------------------ */

  /* 工具箱命令卡的字形。刻意用几何符号而不是 emoji——emoji 要额外的字体文件，
   * 而且在 Windows 上会被渲染成彩色字形，跟整套中性色打不上。
   * （源：portable_builder/web_ui.py 的 TOOL_GLYPHS，字形值勿改。）
   *
   * 与原版唯一的差别：**不含 chplus**。Chrome++ 更新是维护者 CI 的事
   * （update_chrome_plus.py 走 update-chrome-plus.yml，迁移文档明确「不动」），
   * Rust 引擎里没有对应子命令，终端用户也不该在 GUI 里更新仓库内置的 Chrome++。 */
  var TOOL_GLYPHS = {
    inspect: "\u25CE",   /* ◎ */
    research: "\u25A4",  /* ▤ */
    prepare: "\u2261",   /* ≡ */
    resolve: "\u2193",   /* ↓ */
    verify: "\u2713"     /* ✓ */
  };
  var TOOL_ORDER = ["inspect", "research", "prepare", "resolve", "verify"];

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
   * （顺带记一笔：Python 版 _t() 的 default 参数是死参数——函数体从不读它，
   *   所以 _t(s,"gui_title","ChromiumPortable") 缺键时得到的是裸 key。
   *   这里照「缺键回落裸 key」实现，不补那个 default。） */

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

  /* HTML 转义（源：web_ui.py 的 _e()，escape(value, quote=True)）。
   * 本文件改用 DOM 构建，没有字符串拼接，所以 e() 只在外部调用者
   * （无头测试、Wave4 的 Rust 侧回填）需要时保留。 */
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
   *
   * 返回形状：除 snapshot 之外的每个命令同样返回 {snapshot, strings}，
   * 前端据此就地更新或整页重绘（见文件底部「命令面契约」）。
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
      workdir: "C:\\Users\\you\\ChromiumPortable"
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
   * 这里只放渲染冒烟需要的最小集合，值取自 scripts/locales/wizard.json。
   * 其余键会按「缺失回落裸 key」显示成 gui_xxx——这是刻意的：少一条语言包
   * 不该让页面开不出来，也不该把缺失悄悄藏起来。 */
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

  /* 不覆盖已存在的 MOCK：无头测试在 app.js 之前注入自己的快照 + 语言包
   * （迁移文档 §7：Playwright 用注入 mock 的方式量 DOM 几何）。 */
  window.__MOCK_API__ = window.__MOCK_API__ || {
    snapshot: MOCK_SNAPSHOT,
    strings: MOCK_STRINGS
  };

  /* 当前快照 / 语言包：渲染与事件接线共用的一份。命令返回新快照后就地替换，
   * 不额外发一次 invoke("snapshot")。 */
  var currentSnapshot = {};
  var currentStrings = {};

  /* ------------------------------------------------------------------ */
  /* 渲染模式：本文件确立的移植模式                                      */
  /* ------------------------------------------------------------------ */
  /* 规则（每一条都来自对 web_ui.py 渲染函数的平移）：
   *   1. 挂载点是固定 id 的容器（<div id="banner">），或 index.html 里的
   *      {{name}} 注释标记——后者由 mount() 就地生成容器；
   *   2. 渲染函数 render*(host, snapshot, strings) 只写 host 内部
   *      （createElement / textContent），不碰挂载点以外的 DOM；
   *   3. 文案一律经 t(strings, key, fields) 取得——缺失键显示裸 key；
   *   4. 数据来源只有 snapshot；渲染函数不做 IO、不发 invoke。
   */

  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  }

  function $(sel) { return document.querySelector(sel); }
  function $$(sel) {
    return Array.prototype.slice.call(document.querySelectorAll(sel));
  }

  /* ---- 挂载点解析（源：index.html 的「挂载点约定」）----
   * index.html 并存两种写法：固定 id 的空容器（#banner / #deck / #loghead /
   * #logbox），和 {{name}} 注释标记（容器整体由渲染函数生成）。id 优先；
   * 标记存在就地替换；两者都没有才退到 fallbackId 兜底——
   * 渲染函数因此永远只面对「一个属于它的 host」。 */
  function findMarker(text) {
    var walker = document.createTreeWalker(document.body,
      NodeFilter.SHOW_COMMENT, null, false);
    var node = walker.nextNode();
    while (node) {
      if (String(node.nodeValue).trim() === text) return node;
      node = walker.nextNode();
    }
    return null;
  }

  function mount(id, tag, cls, fallbackId) {
    var node = document.getElementById(id);
    if (node) return node;
    node = el(tag, cls);
    node.id = id;
    var marker = findMarker("{{" + id + "}}");
    if (marker && marker.parentNode) {
      marker.parentNode.replaceChild(node, marker);
      return node;
    }
    var parent = document.getElementById(fallbackId || "content");
    if (parent) parent.appendChild(node);
    return node;
  }

  /* ------------------------------------------------------------------ */
  /* 片段辅助（源：web_ui.py 的 _segmented/_field/_row/_text/_select）    */
  /* ------------------------------------------------------------------ */
  /* 原版返回 HTML 字符串再拼进页面；这里直接造节点。类名、层级、属性名
   * 逐一对齐（data-seg / data-bind / data-action / aria-selected ...）。 */

  /* _segmented(name, options, current, aria) */
  function segCtrl(name, options, current, aria) {
    var box = el("div", "seg");
    box.setAttribute("data-seg", name);
    box.setAttribute("role", "tablist");
    if (aria) box.setAttribute("aria-label", aria);
    (options || []).forEach(function (pair) {
      var btn = el("button", "", pair[1]);
      btn.type = "button";
      btn.setAttribute("data-value", pair[0]);
      btn.setAttribute("aria-selected",
        String(pair[0]) === String(current) ? "true" : "false");
      box.appendChild(btn);
    });
    return box;
  }

  /* _field(caption, inner, hint, hint_cls) */
  function fieldNode(caption, inner, hint, hintCls) {
    var box = el("div", "field");
    box.appendChild(el("span", "cap", caption));
    if (inner) box.appendChild(inner);
    if (hint) box.appendChild(el("div", ("hint " + (hintCls || "")).trim(), hint));
    return box;
  }

  /* _row(inner) */
  function rowNode(children) {
    var box = el("div", "row");
    (children || []).forEach(function (child) { if (child) box.appendChild(child); });
    return box;
  }

  /* _text(bind, value, placeholder) */
  function textInput(bind, value, placeholder) {
    var input = document.createElement("input");
    input.type = "text";
    input.setAttribute("data-bind", bind);
    input.value = value == null ? "" : String(value);
    if (placeholder) input.setAttribute("placeholder", placeholder);
    return input;
  }

  /* _select(bind, options, current) */
  function selectInput(bind, options, current) {
    var sel = document.createElement("select");
    sel.setAttribute("data-bind", bind);
    (options || []).forEach(function (pair) {
      var opt = document.createElement("option");
      opt.value = pair[0];
      opt.textContent = pair[1];
      if (String(pair[0]) === String(current)) opt.selected = true;
      sel.appendChild(opt);
    });
    return sel;
  }

  /* 复选框（源：_build_page / _tools_page 里内联的 label.check 片段） */
  function checkNode(bind, checked, label) {
    var lab = el("label", "check");
    var input = document.createElement("input");
    input.type = "checkbox";
    input.setAttribute("data-bind", bind);
    if (checked) input.checked = true;
    lab.appendChild(input);
    lab.appendChild(el("span", "lab", label));
    return lab;
  }

  /* 带 data-action 的按钮 */
  function actionBtn(cls, action, text, arg) {
    var btn = el("button", cls, text);
    btn.type = "button";
    btn.setAttribute("data-action", action);
    if (arg != null) btn.setAttribute("data-arg", arg);
    return btn;
  }

  /* 状态字段 → 渲染参数。等价于 gui.py 的 snapshot_for_render()：
   * 界面参数全部来自 snapshot.vars，名字与 web_ui.render() 的 args 对齐
   * （唯一改名：vars.local → args.local_path，即 gui.py 的 _JS_TO_VAR）。 */
  function argsFrom(snapshot) {
    var v = (snapshot && snapshot.vars) || {};
    return {
      catalog_choices: (snapshot && snapshot.catalog_choices) || [],
      mode: v.mode || "one",
      installer: v.installer || "",
      folder: v.folder || "",
      target: v.target || "",
      arch: v.arch || "x64",
      archive: !!v.archive,
      workdir: v.workdir || "",
      url: v.url || "",
      local_path: v.local || "",
      token: v.token || "",
      tool_path: v.tool_path || "",
      tool_target: v.tool_target || "",
      tool_arch: v.tool_arch || "x64",
      tool_json: !!v.tool_json
    };
  }

  /* ---- 已移植：renderBanner —— 源：web_ui.py render() 里的 banner 分支
   * <div class="banner warn" id="banner"><span class="mark">!</span>
   *     <span>{text}</span><span class="detail">{detail}</span></div>
   * plan.error_key 非空时显示（tone 固定 warn），文案经 t() 缺键回落裸 key。
   * 注：_build_page() 里那份重复的 #banner 不移植——同一个 id 在页面里出现
   * 两次本身就是非法的（原版就有这个重复），index.html 只留 #content 顶部这一份。 */
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

  /* ---- 已移植：renderLogHead —— 源：web_ui.py _deck() 的日志折叠头部。
   * data-tpl 里的 {count} 由 t() 的占位语义填充（缺失键显示裸 key）；
   * gui_log_empty 语义平移：空日志时预填一条占位行并清零计数。 */
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

  /* ---- renderTopbar —— 源：web_ui.py _topbar(s) ----
   * <header id="topbar"> 标记/标题/副标题 + spacer + 两个 Tab + 状态胶囊 + 中EN。
   * Tab 的 aria-selected 初始值是构建页 true / 工具箱 false；
   * 状态胶囊的文字与 tone 来自 snapshot.status（源：window.state 的
   * setPill / setPillText）。 */
  function renderTopbar(snapshot, strings) {
    var host = mount("topbar", "header", "", "app");
    if (!host) return;
    var status = (snapshot && snapshot.status) || {};
    host.textContent = "";

    host.appendChild(el("div", "mark", "C"));

    var titles = el("div", "titles");
    titles.appendChild(el("h1", "", t(strings, "gui_title")));
    titles.appendChild(el("div", "sub", t(strings, "gui_subtitle")));
    host.appendChild(titles);

    host.appendChild(el("div", "spacer", ""));

    var tabs = el("div", "tabs");
    tabs.setAttribute("role", "tablist");
    [["page-build", "gui_nav_build", "true"],
     ["page-tools", "gui_nav_tools", "false"]].forEach(function (spec) {
      var tab = el("button", "tab", t(strings, spec[1]));
      tab.type = "button";
      tab.setAttribute("data-page", spec[0]);
      tab.setAttribute("role", "tab");
      tab.setAttribute("aria-selected", spec[2]);
      tabs.appendChild(tab);
    });
    host.appendChild(tabs);

    var pill = el("span", "pill " + (status.tone || "idle"), status.text || "");
    pill.id = "status";
    host.appendChild(pill);

    var lang = el("button", "btn", "\u4E2D/EN");
    lang.type = "button";
    lang.id = "langbtn";
    lang.setAttribute("data-action", "toggle_lang");
    host.appendChild(lang);
  }

  /* ---- renderBuildPage —— 源：web_ui.py _build_page(s, args) + _mode_block ----
   * ① 唯一带序号的卡：装什么（主角）；② 目标与输出（一行设置条）；
   * ③ 更多选项（details.fold 默认收起，R3）。
   * 卡内控件带 data-bind，动作按钮带 data-action，与 web_ui.py 逐个对齐。 */
  function renderBuildPage(snapshot, strings) {
    var host = mount("page-build", "section", "page active", "content");
    if (!host) return;
    var args = argsFrom(snapshot);
    host.textContent = "";

    /* ① 来源卡 */
    var card = el("div", "card");
    var head = el("div", "card-head");
    head.appendChild(el("span", "stepchip", "1"));
    head.appendChild(el("h2", "", t(strings, "gui_section_source")));
    card.appendChild(head);
    card.appendChild(segCtrl("mode", [
      ["one", t(strings, "gui_mode_one_short")],
      ["folder", t(strings, "gui_mode_folder_short")],
      ["online", t(strings, "gui_mode_online_short")]
    ], args.mode));
    card.appendChild(modeBlock(strings, args));
    host.appendChild(card);

    /* ② 目标与输出：收窄成一行设置条 */
    var card2 = el("div", "card");
    var head2 = el("div", "card-head");
    head2.appendChild(el("h2", "", t(strings, "gui_section_output")));
    card2.appendChild(head2);
    var two = el("div", "two");
    two.appendChild(fieldNode(t(strings, "gui_workdir"), rowNode([
      textInput("workdir", args.workdir),
      actionBtn("btn ghost", "open_workdir", t(strings, "gui_open_workdir")),
      actionBtn("btn", "pick_workdir", t(strings, "gui_browse_dir"))
    ])));
    /* #artifact 在 web_ui.py 里就是空的（没有任何 JS 往里写）——产物落点
     * 由执行卡的结果行显示（gui.py _artifact_hint）。这里保持同样为空，
     * 不擅自发明新行为。 */
    var artifact = el("div", "mono");
    artifact.id = "artifact";
    two.appendChild(fieldNode(t(strings, "gui_artifact_title"), artifact,
      t(strings, "gui_output_hint"), "ref"));
    card2.appendChild(two);
    host.appendChild(card2);

    /* ③ 更多选项：默认收起 */
    var fold = document.createElement("details");
    fold.className = "fold";
    var summary = document.createElement("summary");
    summary.appendChild(el("span", "t", t(strings, "gui_section_more")));
    summary.appendChild(el("span", "h", t(strings, "gui_section_more_hint")));
    fold.appendChild(summary);
    var body = el("div", "body");
    var two2 = el("div", "two");
    two2.appendChild(fieldNode(t(strings, "gui_arch"), segCtrl("arch", [
      ["x64", "x64"], ["x86", "x86"], ["arm64", "arm64"]
    ], args.arch)));
    two2.appendChild(fieldNode(t(strings, "gui_label_packaging"),
      checkNode("archive", args.archive, t(strings, "gui_archive")),
      t(strings, "gui_archive_hint")));
    body.appendChild(two2);
    fold.appendChild(body);
    host.appendChild(fold);
  }

  /* ---- _mode_block(s, args)：来源卡的三个变体 + 三条模式说明 ----
   * 同一时刻只显示一个（IA §2）；顺序与原版一致：one → folder → online → hints。 */
  function modeBlock(strings, args) {
    var frag = document.createDocumentFragment();

    function browseOther() {
      return actionBtn("btn ghost", "pick_installer",
        t(strings, "gui_installer_browse_other"));
    }

    var refresh = actionBtn("btn", "refresh_installers", t(strings, "gui_refresh"));

    /* 文件清单（主角的核心） */
    var picker = el("div", "picker");
    var bar = el("div", "bar");
    var count = el("span", "count");
    count.id = "count";
    /* data-tpl 存带 {count} 占位的模板，renderInstallers 拿它填实际条数 */
    count.setAttribute("data-tpl", t(strings, "gui_installer_count"));
    bar.appendChild(count);
    bar.appendChild(el("span", "grow", ""));
    bar.appendChild(refresh);
    bar.appendChild(browseOther());
    picker.appendChild(bar);
    var rows = el("div", "rows");
    rows.id = "rows";
    picker.appendChild(rows);
    var empty = el("div", "empty");
    empty.id = "empty";
    empty.appendChild(el("div", "t", t(strings, "gui_installer_empty_title")));
    empty.appendChild(el("div", "b", t(strings, "gui_installer_empty_body")));
    empty.appendChild(actionBtn("btn ghost", "pick_installer", t(strings, "gui_browse_file")));
    picker.appendChild(empty);

    /* W1：这一条只在这里说一次——区块标题之外不再复述「把包装进 installers」 */
    var manual = el("div", "zone");
    manual.style.marginTop = "var(--s3)";
    manual.appendChild(el("div", "hint", t(strings, "gui_installer_path")));
    manual.appendChild(rowNode([
      textInput("installer", args.installer),
      actionBtn("btn ghost", "pick_installer", t(strings, "gui_browse_file"))
    ]));

    var one = el("div", "");
    one.setAttribute("data-mode", "one");
    one.hidden = args.mode !== "one";
    one.appendChild(picker);
    one.appendChild(manual);
    frag.appendChild(one);

    var folder = el("div", "");
    folder.setAttribute("data-mode", "folder");
    folder.hidden = args.mode !== "folder";
    folder.appendChild(fieldNode(t(strings, "gui_folder"), rowNode([
      textInput("folder", args.folder),
      actionBtn("btn ghost", "pick_folder", t(strings, "gui_browse_dir")),
      actionBtn("btn", "new_folder", t(strings, "gui_folder_new"))
    ]), t(strings, "gui_folder_hint"), "req"));
    frag.appendChild(folder);

    var online = el("div", "");
    online.setAttribute("data-mode", "online");
    online.hidden = args.mode !== "online";
    /* 必选：目标。说明是规则（req 级）。 */
    online.appendChild(fieldNode(t(strings, "gui_target"), rowNode([
      selectInput("target", args.catalog_choices, args.target),
      actionBtn("btn", "refresh_targets", t(strings, "gui_target_refresh"))
    ]), t(strings, "gui_target_hint"), "req"));
    /* 可选进阶：直链与本地包。缩进成一组，视觉上从属于上面的必选项。 */
    var optional = el("div", "optional");
    optional.appendChild(fieldNode(t(strings, "gui_url"), textInput("url", args.url),
      t(strings, "gui_url_hint")));
    optional.appendChild(fieldNode(t(strings, "gui_local_path"), rowNode([
      textInput("local", args.local_path),
      actionBtn("btn ghost", "pick_local", t(strings, "gui_browse_file"))
    ]), t(strings, "gui_local_hint")));
    online.appendChild(optional);
    frag.appendChild(online);

    ["one", "folder", "online"].forEach(function (key) {
      var hint = el("div", "hint req", t(strings, "gui_mode_" + key));
      hint.setAttribute("data-mode-hint", key);
      hint.hidden = key !== args.mode;
      frag.appendChild(hint);
    });

    return frag;
  }

  /* ---- renderToolsPage —— 源：web_ui.py _tools_page(s, args) ----
   * 输入卡（工作目录 / 工具路径 / 目标 / 架构 / JSON / 凭据）+ 命令卡。
   * 命令卡按 TOOL_ORDER 出卡，字形取 TOOL_GLYPHS（chplus 已按迁移决定删除）。 */
  function renderToolsPage(snapshot, strings) {
    var host = mount("page-tools", "section", "page", "content");
    if (!host) return;
    var args = argsFrom(snapshot);
    host.textContent = "";

    var card = el("div", "card");
    var head = el("div", "card-head");
    head.appendChild(el("h2", "", t(strings, "gui_toolbox_input")));
    card.appendChild(head);

    var two = el("div", "two");
    two.style.marginTop = "var(--s3)";
    two.appendChild(fieldNode(t(strings, "gui_workdir"), rowNode([
      textInput("workdir", args.workdir),
      actionBtn("btn ghost", "open_workdir", t(strings, "gui_open_workdir"))
    ])));
    two.appendChild(fieldNode(t(strings, "gui_tool_path"), rowNode([
      textInput("tool_path", args.tool_path),
      actionBtn("btn ghost", "pick_tool_file", t(strings, "gui_browse_file")),
      actionBtn("btn ghost", "pick_tool_dir", t(strings, "gui_browse_dir"))
    ]), t(strings, "gui_tool_path_hint"), "req"));
    card.appendChild(two);

    var two2 = el("div", "two");
    two2.style.marginTop = "var(--s3)";
    two2.appendChild(fieldNode(t(strings, "gui_tool_target"),
      selectInput("tool_target", args.catalog_choices, args.tool_target)));
    two2.appendChild(fieldNode(t(strings, "gui_tool_arch"), segCtrl("tool_arch", [
      ["x64", "x64"], ["x86", "x86"], ["arm64", "arm64"]
    ], args.tool_arch)));
    two2.appendChild(fieldNode("", checkNode("tool_json", args.tool_json,
      t(strings, "gui_tool_json"))));
    card.appendChild(two2);

    /* 凭据单独成组：它不是一个普通输入框 */
    var zone = el("div", "zone");
    zone.style.marginTop = "var(--s3)";
    zone.appendChild(el("div", "hint", t(strings, "gui_token_group")));
    zone.appendChild(rowNode([textInput("token", args.token)]));
    var tokenHint = el("div", "hint", t(strings, "gui_token_hint"));
    tokenHint.style.marginTop = "var(--s1)";
    zone.appendChild(tokenHint);
    card.appendChild(zone);
    host.appendChild(card);

    var card2 = el("div", "card");
    var head2 = el("div", "card-head");
    head2.appendChild(el("h2", "", t(strings, "gui_tools_commands")));
    head2.appendChild(el("span", "note", t(strings, "gui_tools_commands_hint")));
    card2.appendChild(head2);
    var tools = el("div", "tools");
    tools.style.marginTop = "var(--s3)";
    TOOL_ORDER.forEach(function (key) { tools.appendChild(toolCell(strings, key)); });
    card2.appendChild(tools);
    host.appendChild(card2);
  }

  /* _tools_page 里的一格命令卡 */
  function toolCell(strings, key) {
    var cell = el("div", "tool");
    cell.appendChild(el("div", "ico", TOOL_GLYPHS[key] || "\u2022"));
    var tx = el("div", "tx");
    tx.appendChild(el("div", "n", t(strings, "gui_tool_" + key)));
    tx.appendChild(el("div", "h", t(strings, "gui_tool_" + key + "_hint")));
    cell.appendChild(tx);
    cell.appendChild(actionBtn("btn ghost go", "run_tool",
      t(strings, "gui_run"), key));
    return cell;
  }

  /* ---- renderDeck —— 源：web_ui.py _deck(s) ----
   * 执行卡的骨架（#rail / #result / #start / #cancel / #elapsed / #logwrap）
   * 由 index.html 静态持有（Wave1-E 的拆分决定），这里只写文本与可见性：
   * 静态文案 + 解除初始 hidden；动态部分（隐藏/禁用/计时/结果）统一由
   * applyState() 填，避免两处各写一份。 */
  function renderDeck(snapshot, strings) {
    var deck = document.getElementById("deck");
    if (!deck) return;
    deck.hidden = false;

    var headTitle = deck.querySelector(".head .t");
    if (headTitle) headTitle.textContent = t(strings, "gui_section_flow");

    var start = document.getElementById("start");
    if (start) start.textContent = t(strings, "gui_start");
    var cancel = document.getElementById("cancel");
    if (cancel) cancel.textContent = t(strings, "gui_cancel");
  }

  /* ---- renderInstallers —— 源：web_ui.py JS 的 renderInstallers(items) ----
   * 后端条目形状有两种写法：Bridge._installer_item 把显示串放在 size 里，
   * 而状态契约（gui.py state()）是 size=字节数 + size_text=显示串。
   * 两种都吃，显示串优先——页面只负责摆放，不做单位换算。 */
  function renderInstallers(items) {
    var host = document.getElementById("rows");
    if (!host) return;
    host.textContent = "";
    var cur = getVal("installer");
    var list = items || [];

    if (!list.length) {
      var empty = document.getElementById("empty");
      if (empty) empty.hidden = false;
      var c0 = document.getElementById("count");
      if (c0) c0.textContent = "";
      return;
    }
    var empty2 = document.getElementById("empty");
    if (empty2) empty2.hidden = true;

    list.forEach(function (it) {
      var row = el("button", "row-item");
      row.type = "button";
      row.setAttribute("data-path", it.path);
      if (it.path === cur) row.setAttribute("aria-selected", "true");
      var nm = el("span", "nm", it.name);
      nm.title = it.name;
      row.appendChild(nm);
      row.appendChild(el("span", "sz", it.size_text || it.size || ""));
      if (it.path && it.name !== it.path) {
        var pth = el("span", "pth", it.path);
        pth.title = it.path;
        row.appendChild(pth);
      }
      host.appendChild(row);
    });

    var c2 = document.getElementById("count");
    if (c2) {
      var tpl = c2.getAttribute("data-tpl") || "{count}";
      c2.textContent = tpl.split("{count}").join(String(list.length));
    }
  }

  /* ---- renderRail —— 源：web_ui.py JS 的 window.setRail(steps) ----
   * 步骤轨是本产品的标志性元素。入参是**已翻译**的步骤标题（翻译在渲染前
   * 就做完了，JS 不做翻译——切语言整页重绘，不会两种语言混在一条轨上）。
   * 首屏全部 pending；步骤高亮由 M4c 的 step/done 事件接管（Python 侧是
   * window.step 改 railState 后重画）。 */
  function renderRail(steps) {
    var host = document.getElementById("rail");
    if (!host) return;
    host.textContent = "";
    var list = steps || [];
    if (!list.length) return;   /* 空计划就空着——画一条假轨道只会添噪 */
    list.forEach(function (label) {
      var cell = el("div", "seg-item s-pending");
      cell.appendChild(el("div", "bar"));
      var lab = el("div", "lab");
      lab.appendChild(el("span", "dot"));
      var nm = el("span", "nm", label);
      nm.title = label;
      lab.appendChild(nm);
      cell.appendChild(lab);
      host.appendChild(cell);
    });
  }

  /* ---- 日志折叠体：源：web_ui.py JS 的 openLog() + window.log() 的行数回填 ----
   * 行内容只在**行数变化**时重建：applyState 会随每次输入变化被调用，无脑
   * 重建 4000 行的 DOM 会让打字发涩（Python 侧是 appendChild 增量追加）。 */
  var shownLogLines = -1;

  function syncLog(snapshot, strings) {
    var log = (snapshot && snapshot.log) || { lines: [], open: false };
    var lines = log.lines || [];
    var box = document.getElementById("logbox");
    if (box) {
      box.hidden = !log.open;
      if (shownLogLines !== lines.length) {
        box.textContent = "";
        if (!lines.length) {
          var empty = el("div", "l-dim", t(strings, "gui_log_empty"));
          empty.setAttribute("data-log-empty", "");
          box.appendChild(empty);
        } else {
          lines.forEach(function (line) { box.appendChild(el("div", "", line)); });
        }
        shownLogLines = lines.length;
      }
    }
    /* 展开/收起共用同一个按钮，文案按当前开合状态取（源：openLog） */
    var toggle = document.getElementById("logtoggle");
    if (toggle) {
      toggle.textContent = log.open
        ? toggle.getAttribute("data-collapse")
        : toggle.getAttribute("data-expand");
    }
    var n = document.getElementById("logn");
    if (n) {
      var tpl = n.getAttribute("data-tpl") || t(strings, "gui_log_lines");
      n.textContent = tpl.split("{count}").join(String(lines.length));
    }
  }

  /* ------------------------------------------------------------------ */
  /* 状态 → 局部视图（源：web_ui.py JS 的 window.state(payload)）         */
  /* ------------------------------------------------------------------ */
  /* 整页 render() 只在启动与结构性变化时跑；命令返回的快照走这里就地更新，
   * 这样正在编辑的输入框不会被重建（焦点与光标原地保留）——这正是 Python
   * 侧 render() 与 window.state() 的分工。 */

  function setText(id, text) {
    var node = document.getElementById(id);
    if (node) node.textContent = text == null ? "" : text;
  }

  function setPill(tone, text) {
    var pill = document.getElementById("status");
    if (!pill) return;
    pill.className = "pill " + (tone || "idle");
    if (text != null) pill.textContent = text;
  }

  function setResult(text) {
    setText("result", text || "");
  }

  /* 运行时开始/取消互斥（源：setRunning）。
   * Python 在这里顺手 openLog(true)；这里不做——日志开合已经在快照里
   * （Controller._start 会 _set_log_open(True)），由 syncLog 统一落地。 */
  function setRunning(on) {
    var start = document.getElementById("start");
    var cancel = document.getElementById("cancel");
    if (start) start.hidden = !!on;
    if (cancel) cancel.hidden = !on;
  }

  function applyState() {
    var snapshot = currentSnapshot || {};
    var strings = currentStrings || {};
    var plan = snapshot.plan || {};
    var status = snapshot.status || {};
    var task = snapshot.task || {};

    setPill(status.tone, status.text || "");
    document.documentElement.lang = snapshot.lang === "en" ? "en" : "zh-CN";

    /* 任务进度并入执行卡标题行（旧的左下任务卡已删；meta 优先于 title） */
    setText("railcap", task.meta || task.title || "");
    renderRail(plan.steps || []);
    renderInstallers(snapshot.installer_choices || []);

    var start = document.getElementById("start");
    if (start) start.disabled = !(plan.ok && !snapshot.running);
    setRunning(!!snapshot.running);
    setText("elapsed", snapshot.elapsed || "");
    setResult(snapshot.result || "");
    renderBanner(snapshot, strings);
    syncLog(snapshot, strings);
  }

  /* 命令被拒时的就地横幅（IA §5：不弹窗；迁移文档 §6.4：错误参数必须
   * 浮出 UI 而不是静默进日志）。下一次成功的快照会把它清掉。 */
  function showError(err) {
    var host = document.getElementById("banner");
    if (!host) return;
    host.hidden = false;
    host.className = "banner err";
    host.textContent = "";
    host.appendChild(el("span", "mark", "!"));
    host.appendChild(el("span", "", err && err.message ? err.message : String(err)));
  }

  /* ------------------------------------------------------------------ */
  /* 表单值读写（源：web_ui.py JS 的 setVal/getVal/getChecked/collect）    */
  /* ------------------------------------------------------------------ */
  /* 同一个 data-bind 可能出现在两个页面（workdir 在构建页与工具箱页各一个），
   * 所以读取时优先取「可见页面」里的那个（原版只取第一个匹配：在工具箱页改
   * 工作目录会对不上——见交付报告的偏差清单）。 */

  function bindNodes(name) {
    return $$('[data-bind="' + name + '"]');
  }

  function bindNode(name) {
    var all = bindNodes(name);
    for (var i = 0; i < all.length; i += 1) {
      var page = all[i].closest ? all[i].closest(".page") : null;
      if (page && page.classList.contains("active")) return all[i];
    }
    return all.length ? all[0] : null;
  }

  function setVal(name, value) {
    var text = value == null ? "" : String(value);
    bindNodes(name).forEach(function (node) { node.value = text; });
  }

  function getVal(name) {
    var node = bindNode(name);
    return node ? node.value : "";
  }

  function getChecked(name) {
    var node = bindNode(name);
    return node ? !!node.checked : false;
  }

  /* 分段控件不是 input：它的当前值在 aria-selected 上（web_ui.py 的
   * collect() 用 getVal("arch") 读不到它，于是改架构对计划没有影响——
   * 这里按 DOM 实况读，见报告）。 */
  function segVal(name) {
    var on = $('[data-seg="' + name + '"] button[aria-selected="true"]');
    return on ? on.getAttribute("data-value") : "";
  }

  /* collect()：整份表单。键名与 gui.py 的 apply_inputs 一一对应
   * （唯一改名 local_path → vars.local，同 _JS_TO_VAR）。 */
  function collect() {
    return {
      mode: segVal("mode"),
      target: getVal("target"),
      arch: segVal("arch"),
      archive: getChecked("archive"),
      workdir: getVal("workdir"),
      installer: getVal("installer"),
      folder: getVal("folder"),
      url: getVal("url"),
      local_path: getVal("local"),
      tool_target: getVal("tool_target"),
      tool_arch: segVal("tool_arch"),
      tool_json: getChecked("tool_json"),
      tool_path: getVal("tool_path")
    };
  }

  /* ------------------------------------------------------------------ */
  /* 命令面（data-action → #[tauri::command]）                           */
  /* ------------------------------------------------------------------ */
  /* 参数形状就是这里，Wave4 的 Rust 侧按同一张表实现（形状变了要同步改两边）。 */

  var ACTION_COMMAND = {
    start_build: "start",
    run_tool: "run_tool",
    cancel: "cancel",
    toggle_lang: "set_lang",
    refresh_installers: "refresh",
    refresh_targets: "refresh",
    pick_installer: "pick",
    pick_local: "pick",
    pick_folder: "pick",
    pick_workdir: "pick",
    pick_tool_file: "pick",
    pick_tool_dir: "pick",
    new_folder: "pick",
    open_workdir: "open_path",
    save_log: "log_save",
    copy_log: "log_copy",
    clear_log: "log_clear"
  };

  /* pick 的 kind：与 Controller 的 pick_* 方法一一对应（去掉 pick_ 前缀） */
  var PICK_KIND = {
    pick_installer: "installer",
    pick_local: "local",
    pick_folder: "folder",
    pick_workdir: "workdir",
    pick_tool_file: "tool_file",
    pick_tool_dir: "tool_dir",
    new_folder: "new_folder"
  };

  /* 会改变页面结构的动作：语言变了要重译整页；目录/目标列表变了；对话框
   * 选中的路径要回填到输入框。这些走整页重绘，其余只走 applyState。 */
  var STRUCTURAL_ACTIONS = {
    toggle_lang: true,
    refresh_installers: true,
    refresh_targets: true,
    pick_installer: true,
    pick_local: true,
    pick_folder: true,
    pick_workdir: true,
    pick_tool_file: true,
    pick_tool_dir: true,
    new_folder: true
  };

  function actionArgs(name, act) {
    if (name === "run_tool") return { tool: act.getAttribute("data-arg") || "" };
    if (PICK_KIND[name]) return { kind: PICK_KIND[name] };
    if (name === "refresh_installers") return { kind: "installers" };
    if (name === "refresh_targets") return { kind: "targets" };
    if (name === "toggle_lang") {
      return { lang: currentSnapshot.lang === "zh-CN" ? "en" : "zh-CN" };
    }
    if (name === "open_workdir") return { path: getVal("workdir") };
    return {};
  }

  /* 命令返回 {snapshot, strings} 时：结构性动作整页重绘，其余就地更新。
   * 返回形状不对（没有 snapshot）就什么都不动——宁可保持原样，也不清空页面。 */
  function applyResult(data, structural) {
    if (!data || !data.snapshot) return data;
    currentSnapshot = data.snapshot;
    if (data.strings) currentStrings = data.strings;
    if (structural) paint(data); else applyState();
    return data;
  }

  function runAction(act) {
    var name = act.getAttribute("data-action") || "";
    var cmd = ACTION_COMMAND[name];
    if (!cmd) return;
    var args = actionArgs(name, act);
    var structural = !!STRUCTURAL_ACTIONS[name];
    var pending = flushInputs();
    var run = function () {
      invoke(cmd, args).then(function (data) {
        applyResult(data, structural);
      }).catch(showError);
    };
    /* 待发的输入改动先落库，再发命令：否则命令会跑在旧表单上
     * （web_ui.py 是 refreshPlan() 与 api[name]() 并发，存在这个竞态）。 */
    if (pending) pending.then(run); else run();
  }

  function logCommand(cmd, args) {
    /* 复制走浏览器剪贴板：Rust 侧没有剪贴板插件，而文本本来就在快照里。
     * 失败也要回一次命令，让横幅说明情况而不是静默。 */
    if (cmd === "log_copy") {
      var text = ((currentSnapshot.log || {}).lines || []).join("");
      var write = navigator.clipboard && navigator.clipboard.writeText
        ? navigator.clipboard.writeText(text).then(function () { return true; })
        : Promise.resolve(false);
      write.catch(function () { return false; }).then(function (ok) {
        logCommand("log_copy", { ok: !!ok });
      });
      return;
    }
    invoke(cmd, args || {}).then(function (data) {
      applyResult(data, false);
    }).catch(showError);
  }

  /* ------------------------------------------------------------------ */
  /* 事件接线（源：web_ui.py JS 的点击/输入/键盘三条委派链）              */
  /* ------------------------------------------------------------------ */

  var inputTimer = null;
  var eventsInstalled = false;

  /* 输入变化 → set_input（整份表单，与 gui.py apply_inputs 的语义一致）。
   * 60ms 防抖与 Python 侧一致：一个字符一次 IPC 会把输入框打顿。 */
  function pushInputs() {
    if (inputTimer != null) clearTimeout(inputTimer);
    inputTimer = setTimeout(function () {
      inputTimer = null;
      invoke("set_input", { values: collect() }).then(function (data) {
        applyResult(data, false);
      }).catch(showError);
    }, 60);
  }

  /* 命令派发前把待发的输入改动落一次（失败也不拦命令：命令自己会用
   * 服务端手里的值再算一遍）。 */
  function flushInputs() {
    if (inputTimer == null) return null;
    clearTimeout(inputTimer);
    inputTimer = null;
    return invoke("set_input", { values: collect() }).catch(function () {});
  }

  /* 选中安装包：写回输入框 + 行高亮（源：selectFile） */
  function selectInstaller(path) {
    if (!path) return;
    setVal("installer", path);
    $$(".row-item").forEach(function (row) {
      row.setAttribute("aria-selected",
        row.getAttribute("data-path") === path ? "true" : "false");
    });
    pushInputs();
  }

  /* 分段控件（源：pickSeg/showMode）。模式切换先本地生效，再推给后端；
   * 后端返回的快照不改写表单，所以选中态不会闪回去。 */
  function pickSeg(name, value) {
    if (value == null) return;
    $$('[data-seg="' + name + '"] button').forEach(function (btn) {
      btn.setAttribute("aria-selected",
        btn.getAttribute("data-value") === value ? "true" : "false");
    });
    if (name === "mode") showMode(value);
    pushInputs();
  }

  function showMode(mode) {
    $$("[data-mode]").forEach(function (node) {
      node.hidden = node.getAttribute("data-mode") !== mode;
    });
    var hint = $('[data-mode-hint="' + mode + '"]');
    $$("[data-mode-hint]").forEach(function (node) { node.hidden = true; });
    if (hint) hint.hidden = false;
  }

  /* 顶栏 Tab：纯视图切换，不发命令（源：.tabs button 的 click） */
  function selectPage(id) {
    $$(".tabs button").forEach(function (btn) {
      btn.setAttribute("aria-selected",
        btn.getAttribute("data-page") === id ? "true" : "false");
    });
    $$(".page").forEach(function (page) {
      page.classList.toggle("active", page.id === id);
    });
    var main = document.getElementById("main");
    if (main) main.scrollTop = 0;
  }

  function onClick(ev) {
    if (!ev.target || !ev.target.closest) return;

    var row = ev.target.closest(".row-item");
    if (row) { selectInstaller(row.getAttribute("data-path")); return; }

    var tab = ev.target.closest(".tabs button");
    if (tab) { selectPage(tab.getAttribute("data-page")); return; }

    var segBtn = ev.target.closest("[data-seg] button");
    if (segBtn) {
      var group = segBtn.closest("[data-seg]");
      pickSeg(group.getAttribute("data-seg"), segBtn.getAttribute("data-value"));
      return;
    }

    /* 日志按钮没有 data-action（原版按 id 直接绑），命令面是 log_* */
    var btn = ev.target.closest("button");
    if (btn && btn.id === "logtoggle") { logCommand("log_toggle", {}); return; }
    if (btn && btn.id === "autoscroll") {
      var vars = currentSnapshot.vars || {};
      logCommand("log_autoscroll", { value: !vars.autoscroll });
      return;
    }

    var act = ev.target.closest("[data-action]");
    if (act) runAction(act);
  }

  function onKeydown(ev) {
    if (!ev.target || !ev.target.closest) return;

    var row = ev.target.closest(".row-item");
    if (row && (ev.key === "Enter" || ev.key === " ")) {
      ev.preventDefault();
      selectInstaller(row.getAttribute("data-path"));
      return;
    }

    /* 分段控件：←/→ 切换（源：分组上的 keydown） */
    var group = ev.target.closest("[data-seg]");
    if (group && (ev.key === "ArrowRight" || ev.key === "ArrowLeft")) {
      var keys = Array.prototype.slice.call(group.querySelectorAll("button"));
      var at = keys.indexOf(document.activeElement);
      if (at < 0) return;
      var next = ev.key === "ArrowRight"
        ? keys[(at + 1) % keys.length]
        : keys[(at - 1 + keys.length) % keys.length];
      if (!next) return;
      ev.preventDefault();
      next.focus();
      pickSeg(group.getAttribute("data-seg"), next.getAttribute("data-value"));
      return;
    }

    /* Ctrl+Enter = 开始构建（不用离开输入框，IA §6） */
    if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) {
      ev.preventDefault();
      var start = document.getElementById("start");
      if (start && !start.disabled) start.click();
    }
  }

  function onEdit(ev) {
    if (!ev.target || !ev.target.matches) return;
    if (!ev.target.matches("input, select")) return;
    pushInputs();
  }

  function installEvents() {
    if (eventsInstalled) return;
    eventsInstalled = true;
    document.addEventListener("click", onClick);
    document.addEventListener("keydown", onKeydown);
    document.addEventListener("input", onEdit);
    document.addEventListener("change", onEdit);
  }

  /* ------------------------------------------------------------------ */
  /* 后端事件（builder-event）—— M4c 接线                                */
  /* ------------------------------------------------------------------ */
  /* Rust 侧每 80ms 批量推一条 {kind: "state"|"log"|"step"|"done", …}：
   *   state → 整份快照就地更新（走 applyState，不重建输入框）
   *   log   → 追加日志块（tag "clear" = 清空），只重画日志体
   *   step  → 轨道高亮当前步（进度串已由 state 带过，这里兜底）
   *   done  → 收尾一次重画
   * 事件是增量、快照是全集：即使两者都到，画出来的也是同一个界面。 */

  function appendLogChunk(text) {
    var log = currentSnapshot.log || { lines: [], open: false };
    var lines = (log.lines || []).slice();
    lines.push(text);
    if (lines.length > 4000) lines = lines.slice(lines.length - 4000);
    log.lines = lines;
    currentSnapshot.log = log;
  }

  /* 步骤轨道的五状态里，这里只用三个：已过=done、当前=running、其余=pending */
  function markRail(index) {
    $$("#rail .seg-item").forEach(function (cell, position) {
      var state = position + 1 < index ? "s-done"
        : position + 1 === index ? "s-running" : "s-pending";
      cell.className = "seg-item " + state;
    });
  }

  function applyEvent(payload) {
    if (!payload) return;
    if (payload.state) {
      currentSnapshot = payload.state;
      if (payload.strings) currentStrings = payload.strings;
      applyState();
    }
    if (payload.kind === "log") {
      if (payload.tag === "clear") {
        var log = currentSnapshot.log || { lines: [], open: false };
        log.lines = [];
        currentSnapshot.log = log;
      } else if (payload.text) {
        appendLogChunk(payload.text);
      }
      syncLog(currentSnapshot, currentStrings);
    } else if (payload.kind === "step") {
      markRail(payload.index || 0);
    } else if (payload.kind === "done") {
      var steps = (currentSnapshot.plan && currentSnapshot.plan.steps) || [];
      markRail(steps.length + 1);
    }
  }

  /* 没有 __TAURI__（file:// 直开 / 无头测试）时静默返回：静态页照样能打开。 */
  function listenEvents() {
    var api = window.__TAURI__;
    if (!api || !api.event || typeof api.event.listen !== "function") return false;
    api.event.listen("builder-event", function (evt) { applyEvent(evt.payload); });
    return true;
  }

  /* ------------------------------------------------------------------ */
  /* render() 入口                                                       */
  /* ------------------------------------------------------------------ */
  /* 整页渲染（等价于 Python 侧重新 render() + load_html）：切语言、刷新
   * 列表这类结构性变化走这里；命令返回的普通状态变化走 applyState。 */

  function paint(data) {
    if (!data || !data.snapshot) return;
    currentSnapshot = data.snapshot;
    currentStrings = data.strings || {};
    installEvents();
    renderTopbar(currentSnapshot, currentStrings);
    renderBuildPage(currentSnapshot, currentStrings);
    renderToolsPage(currentSnapshot, currentStrings);
    renderDeck(currentSnapshot, currentStrings);
    renderBanner(currentSnapshot, currentStrings);
    renderLogHead(currentSnapshot, currentStrings);
    applyState();   /* 日志开合、计时、按钮禁用等动态部分 */
  }

  async function render() {
    var data = await invoke("snapshot");
    paint(data);
  }

  window.__cpb = { render: render, invoke: invoke, t: t, e: e };
  window.__cpb.TOOL_GLYPHS = TOOL_GLYPHS;
  window.__cpb.TOOL_ORDER = TOOL_ORDER;
  /* M4c 的 step/done 事件接线与无头测试复用这两个（Python 侧对应
   * window.setRail / renderInstallers）。 */
  window.__cpb.renderRail = renderRail;
  window.__cpb.renderInstallers = renderInstallers;
  window.__cpb.applyEvent = applyEvent;

  function boot() {
    /* 先挂监听再取首屏快照：反过来的话，取回快照之前产生的事件会丢 */
    listenEvents();
    render().catch(showError);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }

  /* ==================================================================== *
   * 命令面契约（data-action / data-bind → #[tauri::command]）             *
   * ==================================================================== *
   * 每个命令返回 {snapshot, strings}，与 invoke("snapshot") 同形。
   *   data-action 元素点击：
   *     start_build         start        {}
   *     run_tool            run_tool     {tool: data-arg}
   *     cancel              cancel       {}
   *     toggle_lang         set_lang     {lang: "en"|"zh-CN"}
   *     refresh_installers  refresh      {kind: "installers"}
   *     refresh_targets     refresh      {kind: "targets"}
   *     pick_* / new_folder pick         {kind: "installer"|"local"|"folder|
   *                                              "workdir"|"tool_file"|
   *                                              "tool_dir"|"new_folder"}
   *     open_workdir        open_path    {path: <工作目录输入框当前值>}
   *     save_log            log_save     {}
   *     copy_log            log_copy     {}
   *     clear_log           log_clear    {}
   *   按钮 id：               命令
   *     #logtoggle          log_toggle   {}
   *     #autoscroll         log_autoscroll {value: !snapshot.vars.autoscroll}
   *   [data-bind] 的 input/change：
   *                         set_input    {values: collect()}  整份表单，键名
   *                         与 gui.py Controller.apply_inputs 一致
   *                         （local_path → vars.local）。
   *
   * web_ui.py → app.js 平移对照表：
   *   _segmented/_field/_row/_text/_select → segCtrl/fieldNode/rowNode/
   *                                          textInput/selectInput/checkNode
   *   _topbar(s)        → renderTopbar(snapshot, strings)
   *   _build_page(s,a)  → renderBuildPage()（含 _mode_block → modeBlock()）
   *   _tools_page(s,a)  → renderToolsPage()（TOOL_ORDER 出卡，chplus 已删）
   *   _deck(s)          → renderDeck()（骨架在 index.html，#loghead 见下）
   *   render() banner 分支 → renderBanner()（Wave1-E 已移植）
   *   JS renderInstallers() → renderInstallers(items)
   *   JS window.setRail()   → renderRail(steps)
   *   JS openLog()/window.log() 的计数 → syncLog()
   *   JS window.state(payload)   → applyState()
   *   JS reportError()      → showError()（落 #banner，不落日志框）
   *   JS collect()/getVal() → collect()/getVal()
   *   JS pickSeg()/showMode()/selectFile()/Tab 切换/键盘 → 同名函数
   *   （Python 的 window.step/done/running/log 事件入口属 M4c，未平移。）
   *
   * 与原版的三处有意偏差（详见交付报告）：
   *   1. TOOL_GLYPHS/TOOL_ORDER 去掉 chplus（父任务决定：CI 专用工具不进 GUI）；
   *   2. collect() 的 arch/tool_arch 从分段控件的 aria-selected 读（原版用
   *      getVal 读 input，恒为空 → 改架构对计划无效）；
   *   3. getVal/bindNode 优先取可见页面里的控件（原版取第一个匹配，工具箱页
   *      的「工作目录」与构建页对不上）。
   * ==================================================================== */

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
