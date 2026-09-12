//! Cross-binary regression for the attached-child publication window from
//! pty#180. The fixed Node reference is pty#181; older Node binaries are
//! expected to fail this test with `metadata is busy` or a missing pid sidecar.

use pty_conformance::*;
use serde_json::Value;
use std::time::Duration;

const ATTEMPTS: usize = 3;

fn operation_budget() -> Duration {
    if unoptimized_binary_under_test() {
        deadline() * 2
    } else {
        deadline()
    }
}

fn remove_if_present(path: &std::path::Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("remove {}: {e}", path.display()),
    }
}

/// node: tests/attach-window.test.ts:220
#[test]
fn attached_child_can_patch_metadata_after_owner_publication() {
    let rig = Rig::new();
    let pty = pty_bin().to_string_lossy().into_owned();

    for attempt in 0..ATTEMPTS {
        let id = unique_id("aw");
        let observed_pid = rig.tmp().join(format!("{id}.observed-pid"));
        let patch_out = rig.tmp().join(format!("{id}.patch-out"));
        let patch_err = rig.tmp().join(format!("{id}.patch-err"));
        let patch_status = rig.tmp().join(format!("{id}.patch-status"));
        let tag_value = format!("attempt-{attempt}");
        let patch = format!(r#"{{"tags":{{"attach-window":"{tag_value}"}}}}"#);
        let script = r#"
if test -s "$PTY_ROOT/$PTY_SESSION.pid"; then
  cat "$PTY_ROOT/$PTY_SESSION.pid" > "$2"
else
  printf 'missing\n' > "$2"
fi
printf '%s' "$6" | "$1" metadata patch --id "$PTY_SESSION" > "$3" 2> "$4"
status=$?
printf '%s\n' "$status" > "$5"
exit "$status"
"#;
        let args = [
            "run",
            "--id",
            id.as_str(),
            "--no-display-name",
            "--tag",
            "keep=true",
            "--",
            "sh",
            "-c",
            script,
            "attach-window-child",
            pty.as_str(),
            observed_pid.to_str().unwrap(),
            patch_out.to_str().unwrap(),
            patch_err.to_str().unwrap(),
            patch_status.to_str().unwrap(),
            patch.as_str(),
        ];

        let mut attached = rig.pty_tty_raw(&[], &[], &args, 24, 80);
        let code = attached.wait_exit(operation_budget()).unwrap_or_else(|| {
            panic!(
                "[{attempt}] attached run did not exit: {:?}",
                attached.output_str()
            )
        });
        assert_eq!(
            code,
            0,
            "[{attempt}] attached output: {:?}",
            attached.output_str()
        );

        let status = std::fs::read_to_string(&patch_status)
            .unwrap_or_else(|e| panic!("[{attempt}] no child patch status: {e}"));
        assert_eq!(
            status.trim(),
            "0",
            "[{attempt}] patch stderr: {}",
            std::fs::read_to_string(&patch_err).unwrap_or_default()
        );

        let daemon_pid: i32 = std::fs::read_to_string(&observed_pid)
            .unwrap_or_else(|e| panic!("[{attempt}] child did not record the owner sidecar: {e}"))
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("[{attempt}] owner sidecar was absent or invalid: {e}"));
        assert!(
            daemon_pid > 1,
            "[{attempt}] invalid daemon pid {daemon_pid}"
        );
        wait_for_process_gone(daemon_pid);

        let response: Value = serde_json::from_slice(
            &std::fs::read(&patch_out)
                .unwrap_or_else(|e| panic!("[{attempt}] no patch response: {e}")),
        )
        .unwrap_or_else(|e| panic!("[{attempt}] patch response is not JSON: {e}"));
        assert_eq!(response["changed"], true, "[{attempt}] {response}");
        assert_eq!(
            response["metadata"]["tags"]["attach-window"], tag_value,
            "[{attempt}] {response}"
        );

        let stored = rig
            .meta(&id)
            .unwrap_or_else(|| panic!("[{attempt}] retained metadata missing"));
        assert_eq!(
            stored["tags"]["attach-window"], tag_value,
            "[{attempt}] {stored}"
        );

        let removed = rig.pty(&["rm", &id]);
        expect_status(&removed, 0);
        rig.wait_for_gone(&id);
        for path in [&observed_pid, &patch_out, &patch_err, &patch_status] {
            remove_if_present(path);
        }
    }
}
