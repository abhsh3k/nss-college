// Mobile navigation toggle. Everything else is HTMX or plain HTML/CSS.
(function () {
  var btn = document.getElementById("menu-toggle");
  var nav = document.getElementById("primary-nav");
  if (!btn || !nav) return;
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
})();
