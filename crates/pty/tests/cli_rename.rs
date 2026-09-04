//! `pty rename`: set, `--show`, `--clear`, the inside-a-session forms, the
//! validation and usage errors, and the `display_name_change` event.
//!
//! node: tests/display-name.test.ts:215-311, tests/metadata-events.test.ts:476-487

mod cli_common;

use cli_common::Rig;
use serde_json::json;

fn help() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/help/rename.txt"
    ))
    .unwrap()
}

/// node: tests/display-name.test.ts:215-259, 268-287
#[test]
fn set_show_clear() {
    let rig = Rig::new();
    rig.write_meta("webapp", json!({}));
    rig.write_meta("api", json!({"displayName": "friendly-api"}));
    let out = rig.ok(&["rename", "webapp", "my-label"]);
    assert_eq!(out.stdout, "Set displayName on \"webapp\" → \"my-label\".\n");
    assert_eq!(rig.read_meta("webapp").unwrap()["displayName"], "my-label");
    let ev = rig.events("webapp");
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["type"], "display_name_change");
    assert_eq!(ev[0]["previous"], serde_json::Value::Null);
    assert_eq!(ev[0]["value"], "my-label");
    rig.ok(&["rename", "webapp", "my-label"]);
    assert_eq!(rig.events("webapp").len(), 1, "no-op emits nothing");

    assert_eq!(rig.ok(&["rename", "--show", "api"]).stdout, "friendly-api\n");
    assert_eq!(rig.ok(&["rename", "--show", "friendly-api"]).stdout, "friendly-api\n");
    assert_eq!(rig.ok(&["rename", "--clear", "api"]).stdout, "Cleared displayName on \"api\".\n");
    assert!(rig.read_meta("api").unwrap().get("displayName").is_none());
    assert_eq!(
        rig.ok(&["rename", "--show", "api"]).stdout,
        "(no displayName; session is referenced by its id: api)\n"
    );
    // A display name may equal another session's id, or the session's own.
    rig.ok(&["rename", "webapp", "api"]);
    rig.ok(&["rename", "api", "api"]);
    assert_eq!(rig.ok(&["rename", "--show", "webapp"]).stdout, "api\n");
}

/// node: tests/display-name.test.ts:261-266, 291-311
#[test]
fn inside_a_session_forms() {
    let rig = Rig::new();
    rig.write_meta("insider", json!({"displayName": "before"}));
    let out = rig.run(&["rename", "only-one"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        format!(
            "pty rename with a single arg is only allowed inside a pty session.\nOutside, use: pty rename <ref> <new-display-name>\n{}",
            help()
        )
    );
    let out = rig.run_env(&["rename", "from-inside"], &[("PTY_SESSION", "insider")]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "Set displayName on \"insider\" → \"from-inside\".\n");
    let out = rig.run_env(&["rename", "--clear"], &[("PTY_SESSION", "insider")]);
    assert_eq!(out.stdout, "Cleared displayName on \"insider\".\n");
    assert!(rig.read_meta("insider").unwrap().get("displayName").is_none());
    let out = rig.run(&["rename", "--clear"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        format!(
            "pty rename --clear with no ref requires being inside a pty session (PTY_SESSION not set).\n{}",
            help()
        )
    );
}

/// node: src/cli.ts:2941-2947, 2973-2977, 3013-3016, 3022-3025
#[test]
fn errors() {
    let rig = Rig::new();
    rig.write_meta("s", json!({}));
    let out = rig.run(&["rename", "--show"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, format!("pty rename --show requires exactly one ref.\n{}", help()));
    let out = rig.run(&["rename", "--clear", "a", "b"]);
    assert_eq!(out.stderr, format!("pty rename --clear takes at most one ref.\n{}", help()));
    let out = rig.run(&["rename", "a", "b", "c"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, help());
    let out = rig.run(&["rename", "--show", "ghost"]);
    assert_eq!(out.stderr, "Session \"ghost\" not found.\n");
    let out = rig.run(&["rename", "s", " Worker"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "Invalid displayName: Display name must be trimmed.\n");
    let out = rig.run(&["rename", "s", "Worker\u{2028}Next"]);
    assert_eq!(
        out.stderr,
        "Invalid displayName: Display name must be single-line and contain no control characters.\n"
    );
    let out = rig.run(&["rename", "s", &"😀".repeat(161)]);
    assert_eq!(out.stderr, "Invalid displayName: Display name too long (max 160 Unicode scalars).\n");
    assert!(rig.read_meta("s").unwrap().get("displayName").is_none());
    // -h after other tokens reaches the parser: `renameUsage()` prints the
    // help to STDERR and exits 0 (cli.ts:2809-2813, 2936).
    let out = rig.ok(&["rename", "s", "-h"]);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, help());
    // Ambiguity fails closed for every form.
    rig.write_meta("alpha", json!({"displayName": "shared"}));
    rig.write_meta("beta", json!({"displayName": "shared"}));
    for args in [&["rename", "--show", "shared"][..], &["rename", "--clear", "shared"][..], &["rename", "shared", "renamed"][..]] {
        let out = rig.run(args);
        assert_eq!(out.code, 1, "{args:?}");
        assert!(out.stderr.starts_with("Session reference \"shared\" is ambiguous."), "{args:?}: {}", out.stderr);
    }
}
