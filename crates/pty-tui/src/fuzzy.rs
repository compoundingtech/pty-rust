//! fzf-style fuzzy matching, ported from `src/tui/fuzzy.ts` with the same
//! scoring: escalating bonus for consecutive runs, +3 per match at a word
//! boundary (`- _ / space .` or the start), +5 for a match at position 0,
//! and `max(0, 10 - (target_len - query_len))` for shorter targets.

/// Match `query` against `target` (case-insensitive, characters in order).
/// `None` = no match; `Some(score)`, higher is better. The empty query
/// matches everything with score 1.
///
/// node: src/tui/fuzzy.ts:19-67
pub fn fuzzy_match(query: &str, target: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(1);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let t: Vec<char> = target.to_lowercase().chars().collect();
    if q.len() > t.len() {
        return None;
    }
    // Does it match at all?
    let mut qi = 0;
    for &c in &t {
        if qi < q.len() && c == q[qi] {
            qi += 1;
        }
    }
    if qi < q.len() {
        return None;
    }

    let positions = find_best_match(&q, &t);
    let mut score: i64 = 0;

    // Consecutive bonus.
    let mut consecutive: i64 = 0;
    for i in 0..positions.len() {
        if i > 0 && positions[i] == positions[i - 1] + 1 {
            consecutive += 1;
            score += consecutive * 2;
        } else {
            consecutive = 0;
        }
    }
    // Word boundary bonus.
    for &pos in &positions {
        if pos == 0 || is_boundary(&t, pos) {
            score += 3;
        }
    }
    // Prefix bonus.
    if positions.first() == Some(&0) {
        score += 5;
    }
    // Length penalty: prefer shorter targets.
    score += (10 - (t.len() as i64 - q.len() as i64)).max(0);
    Some(score)
}

/// `fuzzyMatch(...).match`.
pub fn fuzzy_matches(query: &str, target: &str) -> bool {
    fuzzy_match(query, target).is_some()
}

fn is_boundary(s: &[char], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    matches!(s[pos - 1], '-' | '_' | '/' | ' ' | '.')
}

fn find_best_match(query: &[char], target: &[char]) -> Vec<usize> {
    if let Some(p) = match_prefer_boundaries(query, target) {
        return p;
    }
    let mut positions = Vec::new();
    let mut qi = 0;
    for (ti, &c) in target.iter().enumerate() {
        if qi >= query.len() {
            break;
        }
        if c == query[qi] {
            positions.push(ti);
            qi += 1;
        }
    }
    positions
}

fn match_prefer_boundaries(query: &[char], target: &[char]) -> Option<Vec<usize>> {
    let mut positions = Vec::new();
    let mut qi = 0;
    let mut ti = 0;
    while qi < query.len() && ti < target.len() {
        let boundary = (ti..target.len()).find(|&ahead| {
            target[ahead] == query[qi]
                && is_boundary(target, ahead)
                && can_match(query, qi + 1, target, ahead + 1)
        });
        let mut found_boundary = false;
        if let Some(ahead) = boundary {
            positions.push(ahead);
            qi += 1;
            ti = ahead + 1;
            found_boundary = true;
        }
        if !found_boundary {
            while ti < target.len() && target[ti] != query[qi] {
                ti += 1;
            }
            if ti >= target.len() {
                return None;
            }
            positions.push(ti);
            qi += 1;
            ti += 1;
        }
    }
    (qi == query.len()).then_some(positions)
}

fn can_match(query: &[char], mut qi: usize, target: &[char], mut ti: usize) -> bool {
    while qi < query.len() && ti < target.len() {
        if target[ti] == query[qi] {
            qi += 1;
        }
        ti += 1;
    }
    qi >= query.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/tui-framework.test.ts:659-694
    #[test]
    fn matching() {
        assert!(fuzzy_matches("", "anything"));
        assert!(fuzzy_matches("node", "node"));
        assert!(fuzzy_matches("server", "node-server"));
        assert!(fuzzy_matches("nsr", "node-server"));
        assert!(fuzzy_matches("ns", "node-server"));
        assert!(!fuzzy_matches("sn", "node-server"));
        assert!(!fuzzy_matches("xyz", "node-server"));
        assert!(fuzzy_matches("NODE", "node-server"));
        assert!(fuzzy_matches("node", "Node-Server"));
        assert!(!fuzzy_matches("longquery", "short"));
    }

    /// node: tests/tui-framework.test.ts:697-725
    #[test]
    fn scoring() {
        assert!(fuzzy_match("node", "node").unwrap() > fuzzy_match("node", "n-o-d-e").unwrap());
        assert!(
            fuzzy_match("node", "node-server").unwrap()
                > fuzzy_match("node", "my-node-server").unwrap()
        );
        assert!(
            fuzzy_match("serve", "server").unwrap() > fuzzy_match("serve", "s_e_r_v_e").unwrap()
        );
        assert!(
            fuzzy_match("server", "node-server").unwrap()
                >= fuzzy_match("server", "nodeserver").unwrap()
        );
        assert!(
            fuzzy_match("node", "node").unwrap()
                > fuzzy_match("node", "node-server-application").unwrap()
        );
    }

    /// node: tests/tui-framework.test.ts:728-745
    #[test]
    fn edge_cases() {
        assert!(fuzzy_matches("n", "node"));
        assert!(!fuzzy_matches("z", "node"));
        assert!(fuzzy_matches("n", "n"));
        assert!(!fuzzy_matches("nn", "n"));
        assert_eq!(fuzzy_match("", "x"), Some(1));
        // Exact scoring, pinned: "node" in "node" = consecutive (2+4+6) +
        // boundary at 0 (3) + prefix (5) + length (10) = 30.
        assert_eq!(fuzzy_match("node", "node"), Some(30));
    }
}
