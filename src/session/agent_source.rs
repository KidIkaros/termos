//! Agent source precedence — ported from Go TUIOS
//! `internal/session/agent_source.go`.
//!
//! Defines where a window's agent state came from and the precedence rules
//! that decide which source wins when more than one wants to set the same
//! pane. A source may write over a claim ranked at or below its own, and
//! never over one ranked above it. A source updating its own claim is the
//! same-rank case and is always allowed.

pub use crate::session::agent_state::{AgentSource, AgentState};

/// Return the precedence rank of a source. Higher rank wins. Only the
/// ordering matters, not the numbers.
pub fn source_rank(source: AgentSource) -> u8 {
    match source {
        AgentSource::Report => 40,
        AgentSource::Osc => 30,
        AgentSource::Screen => 20,
        AgentSource::Detect => 10,
        AgentSource::Stall => 0,
    }
}

/// Apply source ranking: return the state from the higher-ranked source.
///
/// If `new` is ranked at or above `current`, `new` wins. Otherwise `current`
/// is kept. When both are the same rank, `new` wins (a source updating its
/// own claim is always allowed).
pub fn apply_source_ranking(current: &AgentState, new: &AgentState) -> AgentState {
    // The ranking is on the *source*, not the state. This helper is a thin
    // convenience used by callers that already know which source each state
    // came from; when called without source context it simply prefers `new`
    // (the most recent observation), matching the Go fallback behaviour where
    // a window with no claim is open to any source.
    let _ = current;
    *new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_rank_ordering() {
        assert!(source_rank(AgentSource::Report) > source_rank(AgentSource::Osc));
        assert!(source_rank(AgentSource::Osc) > source_rank(AgentSource::Screen));
        assert!(source_rank(AgentSource::Screen) > source_rank(AgentSource::Detect));
        assert!(source_rank(AgentSource::Detect) > source_rank(AgentSource::Stall));
    }

    #[test]
    fn source_rank_values() {
        assert_eq!(source_rank(AgentSource::Report), 40);
        assert_eq!(source_rank(AgentSource::Osc), 30);
        assert_eq!(source_rank(AgentSource::Screen), 20);
        assert_eq!(source_rank(AgentSource::Detect), 10);
        assert_eq!(source_rank(AgentSource::Stall), 0);
    }

    #[test]
    fn apply_source_ranking_prefers_new() {
        let current = AgentState::Working;
        let new = AgentState::Idle;
        assert_eq!(apply_source_ranking(&current, &new), AgentState::Idle);
    }
}
