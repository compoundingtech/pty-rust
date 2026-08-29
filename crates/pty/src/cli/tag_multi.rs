//! `pty tag-multi <selector> [--json] [-y|--yes] [ops...]`: read or write
//! tags across several sessions at once. Selectors: explicit refs,
//! `--filter-tag k=v` (AND), or `--all`.
//!
//! node: src/cli.ts:1541-1544, 3300-3511

use pty_core::registry::{self, TagMap, matches_all_tags};
use serde_json::{Map, Value};

use super::{CliError, CliResult, help};

#[derive(Debug, PartialEq, Eq)]
enum Selector {
    All,
    Filter(TagMap),
    Names(Vec<String>),
}

#[derive(Debug)]
struct Parsed {
    selector: Selector,
    updates: TagMap,
    removals: Vec<String>,
    json: bool,
    yes: bool,
}

/// `Ok(Err(code))` when the parser printed help and wants to exit.
///
/// node: src/cli.ts:3300-3389
fn parse(argv: &[String]) -> Result<Result<Parsed, i32>, CliError> {
    let mut all = false;
    let mut filter_tags = TagMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut updates = TagMap::new();
    let mut removals: Vec<String> = Vec::new();
    let mut json = false;
    let mut yes = false;

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "--all" => all = true,
            "--json" => json = true,
            "--yes" | "-y" => yes = true,
            "-h" | "--help" => {
                print!("{}", help::tag_multi_parser_help());
                return Ok(Err(0));
            }
            "--filter-tag" => {
                let Some(next) = argv.get(i + 1) else {
                    return Err("pty tag-multi: --filter-tag requires k=v".into());
                };
                i += 1;
                let Some((k, v)) = next.split_once('=') else {
                    return Err(CliError(format!(
                        "pty tag-multi: --filter-tag value \"{next}\" must be k=v"
                    )));
                };
                if k.is_empty() {
                    return Err("pty tag-multi: --filter-tag key must be non-empty".into());
                }
                filter_tags.insert(k.to_string(), v.to_string());
            }
            "--rm" => {
                let Some(k) = argv.get(i + 1) else {
                    return Err("pty tag-multi: --rm requires a key (e.g. --rm role)".into());
                };
                i += 1;
                if k.is_empty() {
                    return Err("pty tag-multi: --rm requires a non-empty key".into());
                }
                removals.push(k.clone());
            }
            _ => {
                if let Some((k, v)) = a.split_once('=') {
                    if k.is_empty() {
                        return Err(CliError(format!(
                            "pty tag-multi: empty key in \"{a}\". Tag keys must be non-empty."
                        )));
                    }
                    updates.insert(k.to_string(), v.to_string());
                } else {
                    names.push(a.to_string());
                }
            }
        }
        i += 1;
    }

    let selector_count =
        usize::from(all) + usize::from(!filter_tags.is_empty()) + usize::from(!names.is_empty());
    if selector_count == 0 {
        return Err(
            "pty tag-multi: no selector — pass session names, --filter-tag k=v, or --all".into(),
        );
    }
    if selector_count > 1 {
        return Err(
            "pty tag-multi: selectors are mutually exclusive — pick one of <names>, --filter-tag, --all"
                .into(),
        );
    }
    let selector = if all {
        Selector::All
    } else if !filter_tags.is_empty() {
        Selector::Filter(filter_tags)
    } else {
        Selector::Names(names)
    };
    Ok(Ok(Parsed {
        selector,
        updates,
        removals,
        json,
        yes,
    }))
}

fn tags_json(tags: &TagMap) -> Value {
    Value::Object(
        tags.iter()
            .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
            .collect(),
    )
}

/// `cmdTagMulti`.
///
/// node: src/cli.ts:3391-3487
pub fn run(argv: &[String]) -> CliResult {
    let parsed = match parse(argv)? {
        Ok(p) => p,
        Err(code) => return Ok(code),
    };
    let is_write = !parsed.updates.is_empty() || !parsed.removals.is_empty();

    // Explicit names are resolved up-front so an unresolvable name aborts
    // before any write.
    let targets: Vec<String> = match &parsed.selector {
        Selector::Names(names) => {
            let mut out = Vec::with_capacity(names.len());
            for reference in names {
                match registry::get_session(reference).map_err(CliError)? {
                    Some(s) => out.push(s.name),
                    None => {
                        return Err(CliError(format!(
                            "pty tag-multi: session \"{reference}\" not found."
                        )));
                    }
                }
            }
            out
        }
        Selector::Filter(filter) => registry::list_sessions()
            .into_iter()
            .filter(|s| {
                matches_all_tags(s.metadata.as_ref().and_then(|m| m.tags.as_ref()), filter)
            })
            .map(|s| s.name)
            .collect(),
        Selector::All => {
            let all = registry::list_sessions();
            if is_write && !parsed.yes {
                return Err(CliError(format!(
                    "pty tag-multi: --all writes are destructive across {} session(s). Re-run with --yes to apply.",
                    all.len()
                )));
            }
            all.into_iter().map(|s| s.name).collect()
        }
    };

    let current_tags = |name: &str| -> TagMap {
        registry::read_metadata(name)
            .and_then(|m| m.tags)
            .unwrap_or_default()
    };

    if !is_write {
        let out: Vec<(String, TagMap)> = targets
            .iter()
            .map(|n| (n.clone(), current_tags(n)))
            .collect();
        if parsed.json {
            let obj: Map<String, Value> = out
                .iter()
                .map(|(n, t)| (n.clone(), tags_json(t)))
                .collect();
            println!("{}", Value::Object(obj));
            return Ok(0);
        }
        if targets.is_empty() {
            println!("0 sessions matched.");
            return Ok(0);
        }
        for (name, tags) in &out {
            if tags.is_empty() {
                println!("{name}: (no tags)");
                continue;
            }
            println!("{name}:");
            for (k, v) in tags {
                println!("  {k}={v}");
            }
        }
        return Ok(0);
    }

    if targets.is_empty() {
        if parsed.json {
            println!("{{}}");
        } else {
            println!("0 sessions matched. No writes performed.");
        }
        return Ok(0);
    }
    let mut results: Map<String, Value> = Map::new();
    let mut errors: Vec<(String, String)> = Vec::new();
    for name in &targets {
        match registry::update_tags(name, &parsed.updates, &parsed.removals) {
            Ok(_) => {
                results.insert(name.clone(), tags_json(&current_tags(name)));
            }
            Err(e) => errors.push((name.clone(), e)),
        }
    }
    if parsed.json {
        println!("{}", Value::Object(results));
    } else {
        println!("{} session(s) processed.", targets.len() - errors.len());
    }
    if !errors.is_empty() {
        for (name, msg) in &errors {
            eprintln!("pty tag-multi: {name}: {msg}");
        }
        return Ok(1);
    }
    Ok(0)
}
