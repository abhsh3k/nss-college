// Behaviour tests for the `data-*` hooks in `static/js/app.js`.
//
// There is no browser here, so the script is loaded into a `vm` context with a
// small fake DOM. That is enough to exercise the delegated listeners the real
// pages rely on: each hook is fired exactly the way a click/change/submit does,
// and the effect on the fake elements is asserted.
//
// Run directly with `node --test tests/js/app_hooks.test.mjs` (it is also run by
// `cargo test --test js_hooks`).

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

const SOURCE = fs.readFileSync(new URL("../../static/js/app.js", import.meta.url), "utf8");

/// A fresh fake DOM and the listeners `app.js` registers against it.
function loadApp() {
  const listeners = {};
  const bySelector = new Map();

  const document = {
    getElementById: () => null,
    addEventListener: (type, fn) => {
      (listeners[type] ||= []).push(fn);
    },
    querySelectorAll: (sel) => bySelector.get(sel) ?? [],
    querySelector: (sel) => (bySelector.get(sel) ?? [])[0] ?? null,
    body: {
      addEventListener: (type, fn) => {
        (listeners["body:" + type] ||= []).push(fn);
      },
    },
  };

  const window = {
    location: { pathname: "/", hash: "" },
    matchMedia: () => ({ matches: false, addEventListener() {}, addListener() {} }),
    addEventListener: (type, fn) => {
      (listeners["window:" + type] ||= []).push(fn);
    },
    requestAnimationFrame: (fn) => fn(),
    setTimeout: (fn) => fn(),
    confirm: () => true,
    print: () => {
      window.__printed = true;
    },
  };

  vm.runInNewContext(SOURCE, { window, document, console });

  /// Invoke every handler registered for `type`, returning whether the event
  /// was prevented.
  function fire(type, event) {
    let prevented = false;
    const e = Object.assign({}, event, {
      preventDefault: () => {
        prevented = true;
      },
    });
    for (const fn of listeners[type] ?? []) fn(e);
    return prevented;
  }

  return { bySelector, window, fire };
}

/// A fake element, with a `closest` that matches the selectors it is told to.
function element({ attrs = {}, matches = () => false, closest = () => null, extra = {} } = {}) {
  return Object.assign(
    {
      checked: false,
      value: "",
      form: null,
      matches: (sel) => matches(sel),
      closest: (sel) => closest(sel),
      getAttribute: (name) => (name in attrs ? attrs[name] : null),
    },
    extra
  );
}

test("data-mark-all sets every attendance radio to its value", () => {
  const app = loadApp();
  const radios = [
    { type: "radio", name: "status_1", value: "present", checked: false },
    { type: "radio", name: "status_2", value: "present", checked: false },
    { type: "radio", name: "status_2", value: "absent", checked: false },
  ];
  app.bySelector.set('input[type="radio"][name^="status_"]', radios);
  const btn = element({ attrs: { "data-mark-all": "present" }, closest: (sel) => (sel === "[data-mark-all]" ? btn : null) });

  app.fire("click", { target: btn });

  assert.equal(radios[0].checked, true);
  assert.equal(radios[1].checked, true);
  assert.equal(radios[2].checked, false, "the other value must be cleared");
});

test("data-select-all mirrors the header checkbox onto every named row", () => {
  const app = loadApp();
  const rows = [{ checked: false }, { checked: false }];
  app.bySelector.set(".row-checkbox", rows);
  const header = element({
    extra: { checked: true },
    matches: (sel) => sel === "[data-select-all]",
    attrs: { "data-select-all": ".row-checkbox" },
  });

  app.fire("change", { target: header });

  assert.ok(rows.every((r) => r.checked === true));
});

test("data-select-all-toggle flips the rows and the header together", () => {
  const app = loadApp();
  const rows = [{ checked: false }, { checked: true }];
  app.bySelector.set(".row-checkbox", rows);
  const header = { checked: false };
  app.bySelector.set("[data-select-all]", [header]);
  const btn = element({
    closest: (sel) => (sel === "[data-select-all-toggle]" ? btn : null),
    attrs: { "data-select-all-toggle": ".row-checkbox" },
  });

  app.fire("click", { target: btn });

  // One row was unchecked, so the button checks them all.
  assert.ok(rows.every((r) => r.checked === true));
  assert.equal(header.checked, true);
});

test("data-autosubmit submits the select's form on change", () => {
  const app = loadApp();
  let submitted = false;
  const form = { submit: () => (submitted = true) };
  const select = element({ matches: (sel) => sel === "[data-autosubmit]", extra: { form } });

  app.fire("change", { target: select });

  assert.equal(submitted, true);
});

test("data-confirm only blocks the submit the user declines", () => {
  const app = loadApp();
  const form = element({ attrs: { "data-confirm": "Delete this?" } });

  app.window.confirm = () => false;
  assert.equal(app.fire("submit", { target: form }), true, "a declined confirm must prevent the submit");

  app.window.confirm = () => true;
  assert.equal(app.fire("submit", { target: form }), false, "an accepted confirm must let it through");
});

test("data-print opens the print dialog", () => {
  const app = loadApp();
  const btn = element({ closest: (sel) => (sel === "[data-print]" ? btn : null) });

  app.fire("click", { target: btn });

  assert.equal(app.window.__printed, true);
});
