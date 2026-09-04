//! Port of tests/parity-shapes.test.ts: the shared JSON-shape fixtures in
//! `tests/fixtures/parity/shapes.json` (Node-owned, vendored byte-identical
//! at the workspace root). Machine-readable output is asserted field by
//! field per policy: `{exact: v}` | `{type: "number"|"string"}` |
//! `{omitWhenUnset: true}`.

use pty_conformance::*;
use serde_json::Value;
use std::time::Duration;

fn shapes() -> Value {
    let raw = std::fs::read_to_string(workspace_root().join("tests/fixtures/parity/shapes.json"))
        .expect("read shapes.json");
    serde_json::from_str(&raw).expect("shapes.json is JSON")
}

fn assert_field(ctx: &str, entry: &Value, key: &str, policy: &Value) {
    if policy.get("omitWhenUnset").and_then(|v| v.as_bool()) == Some(true) {
        assert!(entry.get(key).is_none(), "{ctx}: {key} should be omitted, got {:?}", entry.get(key));
    } else if let Some(exact) = policy.get("exact") {
        assert_eq!(entry.get(key).unwrap_or(&Value::Null), exact, "{ctx}: {key} != {exact}");
    } else if let Some(t) = policy.get("type").and_then(|v| v.as_str()) {
        let val = entry.get(key).unwrap_or(&Value::Null);
        match t {
            "number" => assert!(val.is_number(), "{ctx}: {key} not a number: {val:?}"),
            "string" => assert!(val.is_string(), "{ctx}: {key} not a string: {val:?}"),
            other => panic!("{ctx}: unknown type policy {other:?}"),
        }
    } else {
        panic!("{ctx}: unrecognized policy {policy:?}");
    }
}

fn env_pairs(fx: &Value) -> Vec<(String, String)> {
    fx["env"]
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default()
}

fn run_with_env(rig: &Rig, env: &[(String, String)], args: &[String]) -> Out {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    rig.pty_env(&env_refs, &refs)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

/// node: tests/parity-shapes.test.ts:1
#[test]
fn shapes_json_matches_the_node_checkout() {
    let Some(node) = node_checkout() else {
        return;
    };
    let ours = std::fs::read(workspace_root().join("tests/fixtures/parity/shapes.json")).unwrap();
    let theirs = std::fs::read(node.join("tests/fixtures/parity/shapes.json")).unwrap();
    assert!(ours == theirs, "shapes.json drifted from the Node checkout");
}

/// node: tests/parity-shapes.test.ts:90
#[test]
fn shared_json_shape_fixtures_pass() {
    let doc = shapes();
    let arr = doc["fixtures"].as_array().expect("doc.fixtures array");
    assert!(!arr.is_empty(), "no fixtures loaded");

    for fx in arr {
        let id = fx["id"].as_str().unwrap_or("?");
        let kind = fx["kind"].as_str().unwrap_or("");
        let settle = fx["settleMs"].as_u64().unwrap_or(700);
        let env = env_pairs(fx);
        let rig = Rig::new();

        match kind {
            "ls-json-shape" => {
                let sessions = fx["sessions"].as_array().expect("sessions array");
                for s in sessions {
                    let args = strings(&s["run"]);
                    let out = run_with_env(&rig, &env, &args);
                    assert_eq!(out.status, 0, "[{id}] run {args:?} failed: {}", out.summary());
                }
                std::thread::sleep(Duration::from_millis(settle));
                let list = rig.list_json();
                for s in sessions {
                    let sid = s["id"].as_str().unwrap();
                    let entry = list
                        .iter()
                        .find(|e| e["name"] == sid)
                        .unwrap_or_else(|| panic!("[{id}] {sid} not in ls --json: {list:?}"));
                    for (key, policy) in s["expect"].as_object().expect("expect object") {
                        assert_field(&format!("[{id}/{sid}]"), entry, key, policy);
                    }
                }
            }
            "stats-clients" => {
                let args = strings(&fx["run"]);
                let sid = args
                    .iter()
                    .position(|a| a == "--id")
                    .and_then(|i| args.get(i + 1))
                    .cloned()
                    .expect("run has --id");
                let out = run_with_env(&rig, &env, &args);
                assert_eq!(out.status, 0, "[{id}] run failed: {}", out.summary());
                std::thread::sleep(Duration::from_millis(settle));
                // A transient peek first (must not count as an attached client).
                if fx["peekFirst"].as_bool() == Some(true) {
                    let _ = rig.pty(&["peek", &sid]);
                }
                let stats = expect_json(&rig.pty(&["stats", "--json", &sid]));
                if let Some(clients) = fx["expect"]["clients"].as_object() {
                    for (key, policy) in clients {
                        assert_field(&format!("[{id}] clients"), &stats["clients"], key, policy);
                    }
                }
                let _ = rig.pty(&["kill", &sid]);
            }
            other => panic!("[{id}] unknown json-shape kind {other:?}"),
        }
    }
}
