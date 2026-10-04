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

  // Dashboard sidebar: highlight the current page or section, and move focus to the content after an HTMX swap.
  // Links may carry a fragment (/hub#timetable), so the section you are reading gets the highlight,
  // and Overview stays highlighted above the first section.
  function markActive() {
    var links = Array.prototype.slice.call(document.querySelectorAll("a[data-nav]"));
    var path = window.location.pathname;
    var best = null;
    var bestLen = -1;
    var fragBest = null;

    links.forEach(function (a) {
      a.removeAttribute("aria-current");
      var href = a.getAttribute("href") || "";
      var hashAt = href.indexOf("#");
      var base = hashAt === -1 ? href : href.slice(0, hashAt);
      var frag = hashAt === -1 ? "" : href.slice(hashAt + 1);
      if (path !== base && path.indexOf(base + "/") !== 0) return;

      if (frag) {
        // A section link wins only once its section has been scrolled to.
        var target = document.getElementById(frag);
        if (target && target.getBoundingClientRect().top <= 120) fragBest = a;
      } else if (base.length > bestLen) {
        // Plain links: the most specific path wins, e.g. /admin/people over /admin.
        bestLen = base.length;
        best = a;
      }
    });

    var current = fragBest || best;
    if (current) current.setAttribute("aria-current", "page");
  }

  var scrollQueued = false;
  function markActiveOnScroll() {
    if (scrollQueued) return;
    scrollQueued = true;
    window.requestAnimationFrame(function () {
      scrollQueued = false;
      markActive();
    });
  }

  markActive();
  window.addEventListener("scroll", markActiveOnScroll, { passive: true });
  window.addEventListener("resize", markActiveOnScroll);
  document.body.addEventListener("htmx:pushedIntoHistory", markActive);
  document.body.addEventListener("htmx:afterSettle", markActive);
  window.addEventListener("popstate", markActive);
  // The sidebar uses hx-boost, which swaps #app-main without the browser's
  // native jump to the fragment. Focusing #app-main then scrolls back to the
  // top, so re-apply the fragment afterwards.
  function scrollToHash() {
    var hash = window.location.hash;
    if (!hash || hash.length < 2) return;
    var target = document.getElementById(decodeURIComponent(hash.slice(1)));
    if (target) target.scrollIntoView({ block: "start" });
  }

  document.body.addEventListener("htmx:afterSettle", function (e) {
    var main = document.getElementById("app-main");
    if (main && e.detail && e.detail.target && e.detail.target.id === "app-main") {
      main.focus();
      scrollToHash();
    }
  });

  // A fragment in the URL on first paint (back button, shared link).
  if (window.location.hash) {
    window.addEventListener("load", scrollToHash);
    scrollToHash();
  }

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
