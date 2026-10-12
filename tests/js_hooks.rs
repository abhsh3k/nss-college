//! Runs the `app.js` `data-*` hook tests (`tests/js/app_hooks.test.mjs`) under
//! Node.
//!
//! The hooks are delegated listeners with no server side, so there is no Rust
//! test that can exercise them. Rather than leave them unverified, the small
//! Node harness in `tests/js/` loads the real script into a fake DOM and asserts
//! each hook's effect. This test just runs it, so `cargo test` covers the
//! behaviour too. If Node is not installed the test reports that and passes —
//! the hooks are extra coverage, not a build requirement.

use std::process::Command;

#[test]
fn the_app_js_data_hooks_behave() {
    let node = if cfg!(windows) { "node.exe" } else { "node" };
    let output = match Command::new(node)
        .args(["--test", "tests/js/app_hooks.test.mjs"])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            eprintln!("skipping the app.js hook tests: `{node}` is not available ({e})");
            return;
        }
    };

    assert!(
        output.status.success(),
        "node --test failed with {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
