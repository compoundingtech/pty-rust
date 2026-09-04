//! `format_event`: the one-line text `pty events` prints.

use pty_core::events::{Event, format_event};
use pty_core::registry;
use serde_json::json;

const TS: &str = "2026-04-05T10:15:03.000Z";

fn ev(r#type: &str, payload: serde_json::Value) -> Event {
    let mut e = Event::new("test", r#type).with_ts(TS);
    if let serde_json::Value::Object(map) = payload {
        e.payload = map;
    }
    e
}

fn body(e: &Event) -> String {
    let line = format_event(e);
    let prefix = format!("] {}: ", e.session);
    let idx = line
        .find(&prefix)
        .unwrap_or_else(|| panic!("no prefix in {line}"));
    line[idx + prefix.len()..].to_string()
}

/// The prefix is `[HH:MM:SS] <session>:` in the local zone.
///
/// node: src/events.ts:549-553
#[test]
fn prefix_is_local_time_and_session() {
    let line = format_event(&ev("bell", json!({})));
    assert!(line.starts_with('['), "{line}");
    assert_eq!(&line[9..], "] test: bell", "{line}");
    let hms = &line[1..9];
    assert_eq!(hms.len(), 8);
    assert_eq!(hms.as_bytes()[2], b':');
    assert_eq!(hms.as_bytes()[5], b':');
    let expected = registry::local_hms(registry::parse_iso8601_ms(TS).unwrap());
    assert_eq!(hms, expected);
    // An unparseable timestamp renders as Node's `Invalid Date`.
    let bad = format_event(&Event::new("test", "bell").with_ts("t"));
    assert_eq!(bad, "[Invalid Date] test: bell");
}

/// node: tests/events.test.ts:216-295
#[test]
fn system_event_bodies() {
    assert_eq!(body(&ev("bell", json!({}))), "bell");
    assert_eq!(
        body(&ev("title_change", json!({"value": "Building..."}))),
        "title -> \"Building...\""
    );
    assert_eq!(
        body(&ev(
            "notification",
            json!({"title": "Done", "body": "Build succeeded", "source": "osc9"})
        )),
        "notification -- \"Done\" Build succeeded"
    );
    assert_eq!(
        body(&ev("notification", json!({"body": "only body"}))),
        "notification only body"
    );
    assert_eq!(
        body(&ev("notification", json!({"title": "", "body": ""}))),
        "notification"
    );
    assert_eq!(body(&ev("focus_request", json!({}))), "focus requested");
    assert_eq!(body(&ev("cursor_visible", json!({}))), "cursor restored");
    assert_eq!(body(&ev("session_start", json!({}))), "started");
    assert_eq!(
        body(&ev(
            "session_start",
            json!({"tags": {"role": "web", "env": "dev"}})
        )),
        "started role=web env=dev"
    );
    assert_eq!(
        body(&ev("session_exit", json!({"exitCode": 0}))),
        "exited (code 0)"
    );
    assert_eq!(
        body(&ev("session_exit", json!({"exitCode": 137, "signal": 9}))),
        "killed by signal 9 (code 137)"
    );
    assert_eq!(
        body(&ev(
            "session_exec",
            json!({"previousCommand": "cat", "command": "sh -c x"})
        )),
        "exec sh -c x (was cat)"
    );
    assert_eq!(body(&ev("session_respawn", json!({}))), "respawned");
    assert_eq!(
        body(&ev("session_abandoned", json!({"reason": "cwd-gone"}))),
        "abandoned (cwd-gone)"
    );
    assert_eq!(
        body(&ev(
            "session_abandoned",
            json!({"reason": "idle", "idleDays": 12})
        )),
        "abandoned (idle 12d)"
    );
    assert_eq!(
        body(&ev("session_abandoned", json!({"reason": "idle"}))),
        "abandoned (idle)"
    );
    assert_eq!(
        body(&ev(
            "session_flapping",
            json!({"counter": 3, "limit": 3, "window": 60})
        )),
        "session_flapping"
    );
}

/// node: tests/events.test.ts:264-295
#[test]
fn user_event_bodies() {
    assert_eq!(
        body(&ev("user.note", json!({"text": "checkpoint"}))),
        "user.note \"checkpoint\""
    );
    assert_eq!(
        body(&ev("user.progress", json!({"data": {"pct": 40}}))),
        "user.progress {\"pct\":40}"
    );
    assert_eq!(body(&ev("user.ping", json!({}))), "user.ping");
    assert!(format_event(&ev("user.ping", json!({}))).ends_with("user.ping"));
    assert_eq!(
        body(&ev(
            "user.both",
            json!({"data": {"ok": true}, "text": "done"})
        )),
        "user.both \"done\""
    );
    assert_eq!(
        body(&ev("user.null", json!({"data": null}))),
        "user.null null"
    );
    assert_eq!(body(&ev("something.else", json!({}))), "something.else");
}

/// node: tests/metadata-events.test.ts:632-692
#[test]
fn metadata_event_bodies() {
    let line = body(&ev(
        "display_name_change",
        json!({"previous": "old", "value": "new"}),
    ));
    assert_eq!(line, "display_name -> \"new\" (was \"old\")");
    assert_eq!(
        body(&ev(
            "display_name_change",
            json!({"previous": "old", "value": null})
        )),
        "display_name -> null (was \"old\")"
    );
    let line = body(&ev(
        "tags_change",
        json!({"previous": {"role": "web"}, "value": {"role": "web", "owner": "forge"}}),
    ));
    assert_eq!(line, "tags -> role=web owner=forge (was role=web)");
    assert_eq!(
        body(&ev(
            "tags_change",
            json!({"previous": {"role": "web"}, "value": {}})
        )),
        "tags -> {} (was role=web)"
    );
    let line = body(&ev(
        "metadata_change",
        json!({"previous": {"displayName": null, "tags": {"role": null}}, "value": {"displayName": "Worker", "tags": {"role": "worker"}}}),
    ));
    assert_eq!(
        line,
        "metadata -> {\"displayName\":\"Worker\",\"tags\":{\"role\":\"worker\"}} (was {\"displayName\":null,\"tags\":{\"role\":null}})"
    );
}

/// The event a daemon writes when it could not kill everything it started.
///
/// **The shape is tested here and the trigger is not**, which is worth saying
/// rather than leaving somebody to assume otherwise. Reaching it needs a
/// descendant that outlives both a TERM and a KILL, and a process that
/// survives SIGKILL cannot be manufactured in a test — a stopped process
/// still dies, and anything that genuinely resists is a wedged machine rather
/// than a fixture.
///
/// What this pins is what a reader gets: the type, and the pids under `data`,
/// which is the field the Node tool's renderer falls back to for a type it
/// does not know.
#[test]
fn survivors_are_reported_as_pids_under_data() {
    let e = Event::session_descendants_survived("s", &[4321, 8765]);
    assert_eq!(e.r#type, "session_descendants_survived");
    assert_eq!(e.session, "s");
    let json = serde_json::to_value(&e).expect("serialize");
    assert_eq!(json["data"]["pids"], serde_json::json!([4321, 8765]));
}
