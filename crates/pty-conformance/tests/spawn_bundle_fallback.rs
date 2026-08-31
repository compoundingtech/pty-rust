//! Port of tests/spawn-bundle-fallback.test.ts, argv-shape half only: the
//! `pty run -d` invocation the library's CLI delegation issues
//! (`run -d --id <name> --no-display-name --cwd <cwd> --unset-env K --tag k=v
//! -- <command> <args…>`) yields a live session whose metadata carries the
//! `unsetEnv` list and the tags.
//!
//! Left out: the on-disk `server.js` fast path, the empty-`PATH` error and
//! the `setServerModulePath` override (lines 59, 116, 139) — spawn-strategy
//! resolution inside the Node library.

use pty_conformance::*;

/// node: tests/spawn-bundle-fallback.test.ts:81
#[test]
fn cli_delegation_argv_shape_spawns_a_live_session() {
    let rig = Rig::new();
    let id = "b1abc";
    let cwd = rig.make_dir("d-work");
    let out = rig.pty(&[
        "run",
        "-d",
        "--id",
        id,
        "--no-display-name",
        "--cwd",
        cwd.to_str().unwrap(),
        "--unset-env",
        "NO_COLOR",
        "--tag",
        "source=test",
        "--",
        "/bin/sh",
        "-c",
        "sleep 30",
    ]);
    expect_status(&out, 0);
    let stats = expect_json(&rig.pty(&["stats", "--json", id]));
    assert_eq!(stats["name"], id);
    assert_eq!(stats["process"]["alive"], true);
    let meta = rig.meta(id).unwrap();
    assert_eq!(meta["unsetEnv"], serde_json::json!(["NO_COLOR"]));
    assert_eq!(meta["tags"], serde_json::json!({"source": "test"}));
    assert_eq!(meta["cwd"], cwd.to_str().unwrap());
}
