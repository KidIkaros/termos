//! OSC 9;4 agent-progress application — ported from Go TUIOS
//! `internal/session/agent_osc.go`.
//!
//! The daemon's PTY read thread scans raw output for `OSC 9;4` progress
//! sequences (see [`super::osc_scan`]) and applies each report as an
//! `AgentSource::Osc` claim. This module is the pure logic that sits between
//! the scanner and the session's agent-state machinery: it maps a progress
//! state to an agent state, runs it through the anti-flicker hold, and
//! produces the report to feed into `apply_agent_report`.
//!
//! The mapping follows the sequence's published meaning rather than any one
//! harness's habits: a determinate or indeterminate bar is the program saying
//! it is busy, clearing the bar is it saying it stopped, and the error state
//! is it saying the operation failed. The warning state is the only one that
//! carries a judgement — a program flagging its own progress as needing
//! attention is asking for a human, which is what `NeedsInput` means here.
//! Clearing maps to `Idle` rather than `Done` because the sequence says the
//! work stopped and says nothing about whether it succeeded.

use std::time::Instant;

use super::agent_hold::{agent_loudness, AgentHold};
use super::agent_state::{apply_agent_report, AgentClaim, AgentReport, AgentSource, AgentState};
use super::osc_scan::agent_state_for_progress;

use crate::vt::progress::ProgressState;

/// The outcome of applying an OSC 9;4 progress report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscApplyOutcome {
    /// The progress state mapped to no agent state (unknown code) and was
    /// ignored.
    Ignored,
    /// The report was held by the anti-flicker timer and not yet published.
    Held,
    /// The report was published. Carries the effective state.
    Published(AgentState),
}

/// Apply an OSC 9;4 progress report to a window's agent state.
///
/// This is the pure, testable counterpart of Go's `applyAgentProgressAt`. It
/// takes the current published state, the current claim, an anti-flicker
/// [`AgentHold`], the clock `now` (for the hold), and `now_nanos` (the
/// unix-nano timestamp the claim machinery stamps), and returns what happened.
///
/// A report whose progress state has no agent-state mapping is ignored. A
/// quieter state is held for the hold window before it is published. A louder
/// or equal state publishes immediately through `apply_agent_report`.
pub fn apply_osc_progress(
    current_state: AgentState,
    claim: Option<&AgentClaim>,
    hold: &mut AgentHold,
    progress: ProgressState,
    now: Instant,
    now_nanos: i64,
) -> OscApplyOutcome {
    let Some(agent) = agent_state_for_progress(progress) else {
        return OscApplyOutcome::Ignored;
    };

    // The hold timer gates publishing. A quieter state must stand unchanged
    // for the hold window; a louder or equal state goes straight through.
    if !hold.report_at(agent, now) {
        return OscApplyOutcome::Held;
    }

    let report = AgentReport {
        state: agent,
        source: AgentSource::Osc,
        ..Default::default()
    };
    let result = apply_agent_report(
        current_state,
        claim.as_ref().map(|c| c.harness.as_str()).unwrap_or(""),
        claim,
        &report,
        now_nanos,
    );
    OscApplyOutcome::Published(result.effective_state)
}

/// Whether an OSC-derived state is quieter than the current published state.
/// Exposed for callers that want to decide whether to schedule a settle tick.
pub fn osc_is_quieter(next: AgentState, current: AgentState) -> bool {
    agent_loudness(next) < agent_loudness(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn unknown_progress_is_ignored() {
        let mut hold = AgentHold::new();
        let now = Instant::now();
        let outcome = apply_osc_progress(
            AgentState::None,
            None,
            &mut hold,
            ProgressState::Clear, // 0 — valid
            now,
            0,
        );
        // Clear maps to Idle, which is louder than None, so it publishes.
        assert!(matches!(outcome, OscApplyOutcome::Published(_)));
    }

    #[test]
    fn indeterminate_publishes_working() {
        let mut hold = AgentHold::new();
        let now = Instant::now();
        let outcome = apply_osc_progress(
            AgentState::None,
            None,
            &mut hold,
            ProgressState::Indeterminate,
            now,
            0,
        );
        assert_eq!(outcome, OscApplyOutcome::Published(AgentState::Working));
    }

    #[test]
    fn warning_publishes_needs_input() {
        let mut hold = AgentHold::new();
        let now = Instant::now();
        let outcome = apply_osc_progress(
            AgentState::None,
            None,
            &mut hold,
            ProgressState::Warning,
            now,
            0,
        );
        assert_eq!(
            outcome,
            OscApplyOutcome::Published(AgentState::NeedsInput)
        );
    }

    #[test]
    fn error_publishes_errored() {
        let mut hold = AgentHold::new();
        let now = Instant::now();
        let outcome = apply_osc_progress(
            AgentState::None,
            None,
            &mut hold,
            ProgressState::Error,
            now,
            0,
        );
        assert_eq!(outcome, OscApplyOutcome::Published(AgentState::Errored));
    }

    #[test]
    fn clear_over_working_is_held() {
        let mut hold = AgentHold::with_window(Duration::from_millis(50));
        let t0 = Instant::now();
        // First bring the pane to Working.
        let outcome = apply_osc_progress(
            AgentState::None,
            None,
            &mut hold,
            ProgressState::Normal,
            t0,
            0,
        );
        assert_eq!(outcome, OscApplyOutcome::Published(AgentState::Working));

        // Clear (Idle) is quieter than Working: held.
        let outcome = apply_osc_progress(
            AgentState::Working,
            None,
            &mut hold,
            ProgressState::Clear,
            t0,
            0,
        );
        assert_eq!(outcome, OscApplyOutcome::Held);

        // After the hold window, a second clear publishes.
        let outcome = apply_osc_progress(
            AgentState::Working,
            None,
            &mut hold,
            ProgressState::Clear,
            t0 + Duration::from_millis(60),
            0,
        );
        assert_eq!(outcome, OscApplyOutcome::Published(AgentState::Idle));
    }

    #[test]
    fn osc_is_quieter_checks() {
        assert!(osc_is_quieter(AgentState::Idle, AgentState::Working));
        assert!(!osc_is_quieter(AgentState::Working, AgentState::Idle));
        assert!(!osc_is_quieter(AgentState::Working, AgentState::Working));
        assert!(osc_is_quieter(AgentState::Done, AgentState::Errored));
    }
}
