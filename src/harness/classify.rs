//! Harness classification — ported from Go TUIOS `internal/harness/classify.go`.
//!
//! Matches screen rules against the bottom of a pane and returns the state
//! the best matching rule names.

use super::manifest::{Manifest, ScreenRule};

/// Check if a screen rule matches against the joined text.
///
/// Every string in `all` must be present, at least one in `any` must be
/// present, and none in `not` may be. An empty `any` with a non-empty
/// `all` is satisfied by `all` alone. An empty rule (no `all`, no `any`)
/// matches nothing.
pub fn check_rule(rule: &ScreenRule, hay: &str) -> bool {
    // All: every string must be present.
    for s in &rule.all {
        if !hay.contains(s) {
            return false;
        }
    }

    // Not: none may be present.
    for s in &rule.not {
        if hay.contains(s) {
            return false;
        }
    }

    // Any: at least one must be present (if any is non-empty).
    if rule.any.is_empty() {
        // A rule with no Any and no All says nothing.
        if rule.all.is_empty() {
            return false;
        }
        return true;
    }

    for s in &rule.any {
        if hay.contains(s) {
            return true;
        }
    }

    false
}

/// Classify a harness's screen rules against the bottom of a pane.
/// Returns (state, rule_index) if matched, None otherwise.
pub fn classify(manifest: &Manifest, tail: &[String]) -> Option<(String, usize)> {
    if !manifest.screen.enabled || manifest.screen.rules.is_empty() || tail.is_empty() {
        return None;
    }

    let hay = tail.join("\n");

    let mut best: Option<(String, usize)> = None;
    let mut best_priority = 0;

    for (i, rule) in manifest.screen.rules.iter().enumerate() {
        if !check_rule(rule, &hay) {
            continue;
        }
        if best.is_none() || rule.priority > best_priority {
            best = Some((rule.state.clone(), i));
            best_priority = rule.priority;
        }
    }

    best
}

/// How many lines from the bottom this harness's rules see.
pub fn screen_lines(manifest: &Manifest) -> i32 {
    if !manifest.screen.enabled {
        return 0;
    }
    if manifest.screen.lines > 0 {
        return manifest.screen.lines;
    }
    super::manifest::DEFAULT_SCREEN_LINES
}

#[cfg(test)]
mod tests {
    use super::super::manifest::{DetectSpec, ScreenSpec};
    use super::*;

    fn make_rule(
        state: &str,
        priority: i32,
        all: Vec<&str>,
        any: Vec<&str>,
        not: Vec<&str>,
    ) -> ScreenRule {
        ScreenRule {
            state: state.to_string(),
            priority,
            all: all.iter().map(|s| s.to_string()).collect(),
            any: any.iter().map(|s| s.to_string()).collect(),
            not: not.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn make_manifest(rules: Vec<ScreenRule>, enabled: bool) -> Manifest {
        Manifest {
            schema_version: 1,
            id: "test".to_string(),
            display_name: "Test".to_string(),
            priority: 50,
            detect: DetectSpec {
                comm: vec!["test".to_string()],
                ..Default::default()
            },
            screen: ScreenSpec {
                enabled,
                lines: 6,
                rules,
            },
        }
    }

    #[test]
    fn check_rule_all_present() {
        let rule = make_rule("working", 10, vec!["spinner", "loading"], vec![], vec![]);
        assert!(check_rule(&rule, "there is a spinner and loading text"));
        assert!(!check_rule(&rule, "only spinner here"));
    }

    #[test]
    fn check_rule_any_present() {
        let rule = make_rule("working", 10, vec![], vec!["yes", "no"], vec![]);
        assert!(check_rule(&rule, "I say yes"));
        assert!(check_rule(&rule, "I say no"));
        assert!(!check_rule(&rule, "I say maybe"));
    }

    #[test]
    fn check_rule_not_present() {
        let rule = make_rule("idle", 10, vec!["prompt"], vec![], vec!["running"]);
        assert!(check_rule(&rule, "the prompt is here"));
        assert!(!check_rule(&rule, "the prompt is running"));
    }

    #[test]
    fn check_rule_empty_matches_nothing() {
        let rule = make_rule("idle", 10, vec![], vec![], vec![]);
        assert!(!check_rule(&rule, "anything"));
    }

    #[test]
    fn check_rule_all_only_no_any() {
        let rule = make_rule("working", 10, vec!["spinner"], vec![], vec![]);
        assert!(check_rule(&rule, "spinner is here"));
        assert!(!check_rule(&rule, "nothing here"));
    }

    #[test]
    fn classify_finds_best_match() {
        let manifest = make_manifest(
            vec![
                make_rule("idle", 5, vec!["$"], vec![], vec![]),
                make_rule(
                    "needs_input",
                    30,
                    vec!["Do you want"],
                    vec!["1. Yes"],
                    vec![],
                ),
            ],
            true,
        );
        let tail = vec!["Do you want to proceed?".to_string(), "1. Yes".to_string()];
        let (state, idx) = classify(&manifest, &tail).unwrap();
        assert_eq!(state, "needs_input");
        assert_eq!(idx, 1);
    }

    #[test]
    fn classify_disabled_returns_none() {
        let manifest = make_manifest(vec![], false);
        let tail = vec!["text".to_string()];
        assert!(classify(&manifest, &tail).is_none());
    }

    #[test]
    fn classify_empty_tail_returns_none() {
        let manifest = make_manifest(vec![make_rule("idle", 10, vec!["$"], vec![], vec![])], true);
        assert!(classify(&manifest, &[]).is_none());
    }

    #[test]
    fn classify_no_match_returns_none() {
        let manifest = make_manifest(
            vec![make_rule("idle", 10, vec!["nonexistent"], vec![], vec![])],
            true,
        );
        let tail = vec!["completely different text".to_string()];
        assert!(classify(&manifest, &tail).is_none());
    }

    #[test]
    fn screen_lines_enabled() {
        let manifest = make_manifest(vec![], true);
        assert_eq!(screen_lines(&manifest), 6);
    }

    #[test]
    fn screen_lines_disabled() {
        let manifest = make_manifest(vec![], false);
        assert_eq!(screen_lines(&manifest), 0);
    }
}
