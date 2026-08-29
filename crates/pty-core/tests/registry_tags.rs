//! Tag rules: reserved keys, filter matching, `--filter-tag` extraction, the
//! `keep` tag, exit-time reap precedence, and gc bookkeeping stripping.

use pty_core::registry::{self, TagMap};

fn tags(pairs: &[(&str, &str)]) -> TagMap {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// node: tests/tags-helpers.test.ts:62-94; src/tags.ts:56-79
#[test]
fn reserved_tag_keys() {
    for k in [
        "ptyfile",
        "ptyfile.session",
        "ptyfile.tags",
        "strategy",
        ":l123-abc",
        ":",
    ] {
        assert!(registry::is_reserved_tag_key(k), "{k}");
    }
    for k in [
        "parent",
        "keep",
        "strategy.status",
        "strategy.idle-days",
        "role",
        "ptyfile.other",
        "",
    ] {
        assert!(!registry::is_reserved_tag_key(k), "{k}");
    }
    assert_eq!(
        registry::EXACT_RESERVED_TAG_KEYS,
        &["ptyfile", "ptyfile.session", "ptyfile.tags", "strategy"]
    );
}

/// node: tests/tags-helpers.test.ts:4-60; src/tags.ts:17-46
#[test]
fn matches_all_tags_and_extract_filter_tags() {
    let session = tags(&[("role", "web"), ("env", "prod")]);
    assert!(registry::matches_all_tags(Some(&session), &tags(&[])));
    assert!(registry::matches_all_tags(
        Some(&session),
        &tags(&[("role", "web")])
    ));
    assert!(registry::matches_all_tags(
        Some(&session),
        &tags(&[("role", "web"), ("env", "prod")])
    ));
    assert!(!registry::matches_all_tags(
        Some(&session),
        &tags(&[("role", "web"), ("env", "dev")])
    ));
    assert!(!registry::matches_all_tags(
        Some(&session),
        &tags(&[("owner", "x")])
    ));
    assert!(registry::matches_all_tags(None, &tags(&[])));
    assert!(!registry::matches_all_tags(None, &tags(&[("role", "web")])));

    let mut args: Vec<String> = [
        "--json",
        "--filter-tag",
        "role=web",
        "x",
        "--filter-tag",
        "k=v=w",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let extracted = registry::extract_filter_tags(&mut args).unwrap();
    assert_eq!(extracted, tags(&[("role", "web"), ("k", "v=w")]));
    assert_eq!(args, vec!["--json".to_string(), "x".to_string()]);

    let mut bad: Vec<String> = vec!["--filter-tag".into()];
    assert_eq!(
        registry::extract_filter_tags(&mut bad),
        Err("--filter-tag expects \"key=value\"".into())
    );
    let mut bad: Vec<String> = vec!["--filter-tag".into(), "no-equals".into()];
    assert_eq!(
        registry::extract_filter_tags(&mut bad),
        Err("--filter-tag expects \"key=value\"".into())
    );
    let mut empty_key: Vec<String> = vec!["--filter-tag".into(), "=v".into()];
    assert_eq!(
        registry::extract_filter_tags(&mut empty_key).unwrap(),
        tags(&[("", "v")])
    );
}

/// node: src/sessions.ts:1020-1044; tests/exit-reap.test.ts:772-842
#[test]
fn keep_tag_semantics() {
    assert_eq!(registry::KEEP_TAG, "keep");
    assert!(!registry::is_keep_requested(None));
    assert!(!registry::is_keep_requested(Some(&tags(&[(
        "role", "web"
    )]))));
    for yes in ["true", "1", "yes", "TRUE", "anything", "", " on "] {
        assert!(
            registry::is_keep_requested(Some(&tags(&[("keep", yes)]))),
            "{yes:?}"
        );
    }
    for no in ["false", "0", "no", "off", "FALSE", " Off ", "No"] {
        assert!(
            !registry::is_keep_requested(Some(&tags(&[("keep", no)]))),
            "{no:?}"
        );
    }
}

/// node: src/sessions.ts:1069-1089
#[test]
fn should_reap_at_exit_precedence() {
    let none: Option<&TagMap> = None;
    // Default reap.
    assert!(registry::should_reap_at_exit(none, false, true));
    assert!(!registry::should_reap_at_exit(none, false, false));
    // Ephemeral forces reap even under preserve.
    assert!(registry::should_reap_at_exit(none, true, false));
    // Permanent preserves…
    let permanent = tags(&[("strategy", "permanent")]);
    assert!(!registry::should_reap_at_exit(
        Some(&permanent),
        false,
        true
    ));
    // …unless ephemeral.
    assert!(registry::should_reap_at_exit(Some(&permanent), true, true));
    // keep beats everything, including ephemeral.
    let keep = tags(&[("keep", "true")]);
    assert!(!registry::should_reap_at_exit(Some(&keep), true, true));
    let keep_false = tags(&[("keep", "false")]);
    assert!(registry::should_reap_at_exit(
        Some(&keep_false),
        false,
        true
    ));
    let keep_and_ephemeral = tags(&[("keep", "yes"), ("strategy", "permanent")]);
    assert!(!registry::should_reap_at_exit(
        Some(&keep_and_ephemeral),
        true,
        true
    ));
}

/// node: src/cli.ts:3667-3672, 4088-4100
#[test]
fn strip_gc_bookkeeping_keys() {
    assert_eq!(
        registry::GC_BOOKKEEPING_KEYS,
        &[
            "strategy.status",
            "strategy.consecutive-fast-fails",
            "strategy.last-respawn-at",
            "strategy.command-hash",
        ]
    );
    assert_eq!(registry::strip_gc_bookkeeping(None), None);
    assert_eq!(
        registry::strip_gc_bookkeeping(Some(&tags(&[]))),
        Some(tags(&[]))
    );
    let full = tags(&[
        ("strategy", "permanent"),
        ("strategy.status", "flapping"),
        ("role", "web"),
        ("strategy.consecutive-fast-fails", "3"),
        ("strategy.last-respawn-at", "2026-01-01T00:00:00.000Z"),
        ("strategy.command-hash", "abcd"),
        ("strategy.fast-fail-window", "60"),
    ]);
    let stripped = registry::strip_gc_bookkeeping(Some(&full)).unwrap();
    assert_eq!(
        stripped,
        tags(&[
            ("strategy", "permanent"),
            ("role", "web"),
            ("strategy.fast-fail-window", "60")
        ])
    );
    let only_bookkeeping = tags(&[("strategy.status", "flapping")]);
    assert_eq!(
        registry::strip_gc_bookkeeping(Some(&only_bookkeeping)),
        None
    );
}
