// Mobile menu toggle, plus small helpers for the dashboards. Everything else is HTMX or plain HTML/CSS.
(function () {
  var btn = document.getElementById("menu-toggle");
  var nav = document.getElementById("primary-nav");
  if (btn && nav) {
    btn.addEventListener("click", function () {
      var open = btn.getAttribute("aria-expanded") === "true";
      btn.setAttribute("aria-expanded", String(!open));
      nav.classList.toggle("hidden", open);
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && btn.getAttribute("aria-expanded") === "true") {
        btn.click();
        btn.focus();
      }
    });
  }

  // Dashboard sidebar: highlight the current page, and move focus to the content after an HTMX swap.
  function markActive() {
    document.querySelectorAll("a[data-nav]").forEach(function (a) {
      if (a.getAttribute("href") === window.location.pathname) {
        a.setAttribute("aria-current", "page");
      } else {
        a.removeAttribute("aria-current");
      }
    });
  }
  markActive();
  document.body.addEventListener("htmx:pushedIntoHistory", markActive);
  window.addEventListener("popstate", markActive);
  document.body.addEventListener("htmx:afterSettle", function (e) {
    var main = document.getElementById("app-main");
    if (main && e.detail && e.detail.target && e.detail.target.id === "app-main") main.focus();
  });
})();
