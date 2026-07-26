/* 渐进增强脚本。
   页面内容全部由 tools/generate.py 预渲染成静态 HTML，这里只负责两件交互：
   深色模式切换、按浏览器筛选下载卡片。禁用 JavaScript 时页面依然完整可用。*/

(function () {
  var root = document.documentElement;
  var STORAGE_KEY = "cp-theme";

  /* ---- 深色模式 ---- */
  var toggle = document.querySelector("[data-theme-toggle]");
  if (toggle) {
    var prefersDark = window.matchMedia("(prefers-color-scheme: dark)");

    var currentTheme = function () {
      if (root.dataset.theme === "dark" || root.dataset.theme === "light") {
        return root.dataset.theme;
      }
      return prefersDark.matches ? "dark" : "light";
    };

    var syncLabel = function () {
      var next = currentTheme() === "dark" ? "浅色" : "深色";
      toggle.setAttribute("aria-label", "切换到" + next + "模式");
    };

    toggle.addEventListener("click", function () {
      var next = currentTheme() === "dark" ? "light" : "dark";
      root.dataset.theme = next;
      try {
        localStorage.setItem(STORAGE_KEY, next);
      } catch (error) {
        /* 隐私模式下 localStorage 可能不可写，忽略即可 */
      }
      syncLabel();
    });

    // 没有显式选择过主题时，跟随系统变化
    prefersDark.addEventListener("change", syncLabel);
    syncLabel();
  }

  /* ---- 下载卡片筛选 ---- */
  var filters = Array.prototype.slice.call(document.querySelectorAll("[data-filter]"));
  var cards = Array.prototype.slice.call(document.querySelectorAll(".build-card[data-family]"));
  if (!filters.length || !cards.length) {
    return;
  }

  var apply = function (value) {
    filters.forEach(function (button) {
      var active = button.dataset.filter === value;
      button.classList.toggle("is-active", active);
      button.setAttribute("aria-pressed", active ? "true" : "false");
    });
    cards.forEach(function (card) {
      var visible = value === "all" || card.dataset.family === value;
      card.classList.toggle("is-hidden", !visible);
    });
  };

  filters.forEach(function (button) {
    button.addEventListener("click", function () {
      apply(button.dataset.filter);
    });
  });
})();
