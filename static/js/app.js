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
    var path = window.location.pathname;
    var best = null;
    document.querySelectorAll("a[data-nav]").forEach(function (a) {
      a.removeAttribute("aria-current");
      var href = a.getAttribute("href");
      var match = path === href || path.indexOf(href + "/") === 0;
      if (match && (!best || href.length > best.getAttribute("href").length)) best = a;
    });
    if (best) best.setAttribute("aria-current", "page");
  }
  markActive();
  document.body.addEventListener("htmx:pushedIntoHistory", markActive);
  window.addEventListener("popstate", markActive);
  document.body.addEventListener("htmx:afterSettle", function (e) {
    var main = document.getElementById("app-main");
    if (main && e.detail && e.detail.target && e.detail.target.id === "app-main") main.focus();
  });

  // Attendance register: "Mark everyone present/absent".
  document.addEventListener("click", function (e) {
    var btn = e.target.closest("[data-mark-all]");
    if (!btn) return;
    var value = btn.getAttribute("data-mark-all");
    document.querySelectorAll('input[type="radio"][name^="status_"]').forEach(function (r) {
      if (r.value === value) r.checked = true;
    });
  });
})();
