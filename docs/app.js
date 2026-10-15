(function () {
  var root = document.documentElement;
  var STORAGE_KEY = "cp-theme";

  var toggle = document.querySelector("[data-theme-toggle]");
  if (toggle) {
    var prefersDark = window.matchMedia("(prefers-color-scheme: dark)");
    var labelLight = toggle.getAttribute("data-label-light") || "切换到浅色模式";
    var labelDark = toggle.getAttribute("data-label-dark") || "切换到深色模式";

    var currentTheme = function () {
      if (root.dataset.theme === "dark" || root.dataset.theme === "light") {
        return root.dataset.theme;
      }
      return prefersDark.matches ? "dark" : "light";
    };

    var syncLabel = function () {
      toggle.setAttribute("aria-label", currentTheme() === "dark" ? labelLight : labelDark);
    };

    toggle.addEventListener("click", function () {
      var next = currentTheme() === "dark" ? "light" : "dark";
      root.dataset.theme = next;
      try {
        localStorage.setItem(STORAGE_KEY, next);
      } catch (error) {
      }
      syncLabel();
    });

    prefersDark.addEventListener("change", syncLabel);
    syncLabel();
  }

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
