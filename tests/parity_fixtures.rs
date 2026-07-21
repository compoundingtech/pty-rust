//! Shared-parity fixtures loader. The canonical fixtures file is owned by the
//! node repo (tests/fixtures/parity/screens.json); this is the byte-identical
//! vendored mirror. Both suites load the SAME data and assert the SAME expected
//! values so node and rust pass one behavioral spec.
//!
//! rust loader ≙ node's tests/parity-fixtures.test.ts: spawn the daemon, wait
//! settleMs, run `peek --plain`, assert stdout (trailing "\n" stripped) equals
//! expect.plainScreen (+ plainScreenLength / status / exitCode where present).

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

fn pty_bin() -> &'static str {
    env!("CARGO_BIN_EXE_pty")
}

fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn unique_root(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pty-fix-{}-{}-{}", tag, std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run a pty subcommand in an isolated registry, scrubbing ambient PTY_SESSION
/// and PTY_REAP_ON_EXIT (so a fixture's own env — e.g. PTY_REAP_ON_EXIT=false —
/// is the only source of reap config, deterministically).
fn pty(root: &PathBuf, args: &[&str]) -> (String, String, i32) {
    pty_env(root, args, &[])
}

fn pty_env(root: &PathBuf, args: &[&str], env: &[(String, String)]) -> (String, String, i32) {
    let mut c = Command::new(pty_bin());
    c.args(args)
        .env("PTY_ROOT", root)
        .env_remove("PTY_SESSION")
        .env_remove("PTY_REAP_ON_EXIT");
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("spawn pty");
    (
        // NB: only trailing newline stripped (node strips one trailing \n too),
        // NOT full trim — the fixtures assert exact inner + trailing content.
        strip_one_trailing_nl(&String::from_utf8_lossy(&out.stdout)),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn strip_one_trailing_nl(s: &str) -> String {
    s.strip_suffix('\n').unwrap_or(s).to_string()
}

#[test]
fn shared_parity_fixtures_pass() {
    let _serial = serial();
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/parity/screens.json"
    ))
    .expect("read fixtures");
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("valid fixtures json");
    // Schema v2: { version, note, fixtures: [...] }.
    let arr = doc["fixtures"].as_array().expect("doc.fixtures is an array");
    assert!(!arr.is_empty(), "no fixtures loaded");

    for fx in arr {
        let id = fx["id"].as_str().unwrap_or("?");
        let kind = fx["kind"].as_str().unwrap_or("");
        let command = fx["spawn"]["command"].as_str().expect("spawn.command");
        let args: Vec<String> = fx["spawn"]["args"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect())
            .unwrap_or_default();
        let rows = fx["spawn"]["rows"].as_u64().unwrap_or(24).to_string();
        let cols = fx["spawn"]["cols"].as_u64().unwrap_or(80).to_string();
        let settle = fx["settleMs"].as_u64().unwrap_or(500);
        // Fixture-level `env` overlay (daemon env), e.g. PTY_REAP_ON_EXIT=false.
        let env: Vec<(String, String)> = fx["env"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let root = unique_root(id);
        let mut run_args: Vec<&str> =
            vec!["run", "--id", id, "--rows", &rows, "--cols", &cols, "--"];
        run_args.push(command);
        for a in &args {
            run_args.push(a);
        }
        let (_o, err, code) = pty_env(&root, &run_args, &env);
        assert_eq!(code, 0, "[{id}] run failed: {err}");

        std::thread::sleep(Duration::from_millis(settle));

        match kind {
            // Live or preserved screen → peek --plain must equal the exact bytes.
            "plain-screen" | "plain-screen-after-exit" => {
                let expect_screen =
                    fx["expect"]["plainScreen"].as_str().expect("expect.plainScreen");
                let (screen, err, code) = pty(&root, &["peek", "--plain", id]);
                assert_eq!(code, 0, "[{id}] peek failed: {err}");
                assert_eq!(screen, expect_screen, "[{id}] plainScreen mismatch");

                if let Some(len) = fx["expect"]["plainScreenLength"].as_u64() {
                    assert_eq!(
                        screen.chars().count() as u64,
                        len,
                        "[{id}] plainScreenLength mismatch: {screen:?}"
                    );
                }
                if let Some(exp_status) = fx["expect"]["status"].as_str() {
                    let (ls, _e, _c) = pty(&root, &["ls", "--json"]);
                    assert!(
                        ls.contains(&format!("\"status\":\"{exp_status}\"")),
                        "[{id}] ls --json status != {exp_status}: {ls}"
                    );
                }
                if let Some(exp_code) = fx["expect"]["exitCode"].as_i64() {
                    let (ls, _e, _c) = pty(&root, &["ls", "--json"]);
                    assert!(
                        ls.contains(&format!("\"exitCode\":{exp_code}")),
                        "[{id}] ls --json exitCode != {exp_code}: {ls}"
                    );
                    // idempotent: a second peek is byte-identical.
                    let (screen2, _e, _c) = pty(&root, &["peek", "--plain", id]);
                    assert_eq!(screen2, expect_screen, "[{id}] post-exit peek not idempotent");
                }
            }
            // Default-reap: the session removed itself → peek fails + ls omits.
            "reaped-after-exit" => {
                let (_o, _e, peek_code) = pty(&root, &["peek", "--plain", id]);
                assert_ne!(peek_code, 0, "[{id}] reaped session should not be peekable");
                let (ls, _e, _c) = pty(&root, &["ls", "--json"]);
                assert!(
                    !ls.contains(&format!("\"{id}\"")),
                    "[{id}] reaped session should be omitted from ls: {ls}"
                );
            }
            other => panic!("[{id}] unknown fixture kind {other:?}"),
        }

        let _ = pty(&root, &["kill", id]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
