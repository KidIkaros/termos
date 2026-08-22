//! Screen-content agent state scanning — ported from Go TUIOS
//! `internal/session/agent_screen.go`.
//!
//! Scans the bottom of a pane's rendered text for agent state using the
//! harness manifest's screen rules. This is the last-resort tier: it reports
//! as `AgentSource::Screen`, so it does not write over a harness reporting
//! for itself or an escape sequence the pane emitted. What it can do is see
//! a state those two never mention — a harness sitting on a blocking prompt
//! paints it once and then emits nothing at all.

use crate::session::agent_state::{AgentSource, AgentState};

/// Minimal screen-rule definition (inlined from harness::manifest).
#[derive(Debug, Clone)]
pub struct ScreenRule {
    pub state: String,
    pub priority: i32,
    pub all: Vec<String>,
    pub any: Vec<String>,
    pub not: Vec<String>,
}

/// Minimal classify helper (inlined from harness::classify).
mod classify {
    use super::ScreenRule;
    pub fn check_rule(rule: &ScreenRule, hay: &str) -> bool {
        let hay_lower = hay.to_lowercase();
        if !rule.all.iter().all(|s| hay_lower.contains(&s.to_lowercase())) {
            return false;
        }
        if !rule.any.is_empty() && !rule.any.iter().any(|s| hay_lower.contains(&s.to_lowercase())) {
            return false;
        }
        if rule.not.iter().any(|s| hay_lower.contains(&s.to_lowercase())) {
            return false;
        }
        true
    }
}

/// A screen-rule match: the state, its priority, and which rule matched.
#[derive(Debug, Clone)]
pub struct AgentStateMatch {
    /// The agent state the matched rule names.
    pub state: AgentState,
    /// The priority of the matched rule (higher wins).
    pub priority: i32,
    /// The index of the matched rule in the rules slice.
    pub matched_rule: usize,
    /// The source this match came from (always `Screen`).
    pub source: AgentSource,
}

/// Scan screen lines for an agent state using the given screen rules.
///
/// Rules are checked in order; the highest-priority matching rule wins.
/// Returns `None` when no rule matches or when the lines are empty.
pub fn scan_screen_for_agent_state(lines: &[String], rules: &[ScreenRule]) -> Option<AgentStateMatch> {
    if lines.is_empty() || rules.is_empty() {
        return None;
    }

    let hay = lines.join("\n");

    let mut best: Option<AgentStateMatch> = None;
    let mut best_priority = i32::MIN;

    for (i, rule) in rules.iter().enumerate() {
        if !classify::check_rule(rule, &hay) {
            continue;
        }
        let state = AgentState::parse(&rule.state)?;
        if best.is_none() || rule.priority > best_priority {
            best = Some(AgentStateMatch {
                state,
                priority: rule.priority,
                matched_rule: i,
                source: AgentSource::Screen,
            });
            best_priority = rule.priority;
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_rule(state: &str, priority: i32, all: Vec<&str>, any: Vec<&str>, not: Vec<&str>) -> ScreenRule {
        ScreenRule {
            state: state.to_string(),
            priority,
            all: all.iter().map(|s| s.to_string()).collect(),
            any: any.iter().map(|s| s.to_string()).collect(),
            not: not.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn scan_finds_best_match() {
        let rules = vec![
            make_rule("idle", 5, vec!["$"], vec![], vec![]),
            make_rule("needs_input", 30, vec!["Do you want"], vec!["1. Yes"], vec![]),
        ];
        let lines = vec!["Do you want to proceed?".to_string(), "1. Yes".to_string()];
        let m = scan_screen_for_agent_state(&lines, &rules).unwrap();
        assert_eq!(m.state, AgentState::NeedsInput);
        assert_eq!(m.priority, 30);
        assert_eq!(m.matched_rule, 1);
        assert_eq!(m.source, AgentSource::Screen);
    }

    #[test]
    fn scan_no_match_returns_none() {
        let rules = vec![make_rule("idle", 10, vec!["nonexistent"], vec![], vec![])];
        let lines = vec!["completely different text".to_string()];
        assert!(scan_screen_for_agent_state(&lines, &rules).is_none());
    }

    #[test]
    fn scan_empty_lines_returns_none() {
        let rules = vec![make_rule("idle", 10, vec!["$"], vec![], vec![])];
        assert!(scan_screen_for_agent_state(&[], &rules).is_none());
    }

    #[test]
    fn scan_empty_rules_returns_none() {
        let lines = vec!["some text".to_string()];
        assert!(scan_screen_for_agent_state(&lines, &[]).is_none());
    }

    #[test]
    fn scan_all_condition() {
        let rules = vec![make_rule("working", 10, vec!["spinner", "loading"], vec![], vec![])];
        let lines = vec!["there is a spinner and loading text".to_string()];
        let m = scan_screen_for_agent_state(&lines, &rules).unwrap();
        assert_eq!(m.state, AgentState::Working);
    }

    #[test]
    fn scan_not_condition_excludes() {
        let rules = vec![make_rule("idle", 10, vec!["prompt"], vec![], vec!["running"])];
        let lines = vec!["the prompt is running".to_string()];
        assert!(scan_screen_for_agent_state(&lines, &rules).is_none());
    }

    #[test]
    fn scan_priority_picks_highest() {
        let rules = vec![
            make_rule("idle", 100, vec!["text"], vec![], vec![]),
            make_rule("working", 50, vec!["text"], vec![], vec![]),
        ];
        let lines = vec!["some text here".to_string()];
        let m = scan_screen_for_agent_state(&lines, &rules).unwrap();
        assert_eq!(m.state, AgentState::Idle);
        assert_eq!(m.priority, 100);
        assert_eq!(m.matched_rule, 0);
    }
}
