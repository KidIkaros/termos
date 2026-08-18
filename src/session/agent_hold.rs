//! Anti-flicker hold timer — ported from Go TUIOS
//! `internal/session/agent_hold.go`.
//!
//! A sampled source can disagree with itself between two samples, and a pane
//! whose glyph changes twice for one event reads as noise however correct each
//! individual sample was. The hold timer swallows the gap: a quieter state
//! waits `hold_window` (default 700ms) before being published, so a source
//! that flips and flips back inside the window produces one transition rather
//! than two.

use std::time::{Duration, Instant};

use crate::session::agent_state::AgentState;

/// Default hold window: 700ms. Short enough that a genuine change feels
/// immediate, long enough to swallow the gap between a harness clearing its
/// progress bar and setting it again for the next step of the same task.
pub const DEFAULT_HOLD_WINDOW: Duration = Duration::from_millis(700);

/// Rank a state by how much it wants a human. The ordering is the whole
/// anti-flicker policy: a transition that does not lower it is published at
/// once, and only a transition that lowers it waits.
///
/// It is deliberately asymmetric. Being slow to say "the agent needs you"
/// costs the user the thing the feature exists to prevent, while being slow
/// to say "the agent went quiet" costs them nothing.
pub fn agent_loudness(state: AgentState) -> u8 {
    match state {
        AgentState::NeedsInput | AgentState::Errored => 3,
        AgentState::Working => 2,
        AgentState::Idle | AgentState::Done => 1,
        AgentState::None => 0,
    }
}

/// An anti-flicker hold timer for a single window's agent state.
///
/// A state at or above the current loudness goes straight through, as does
/// any state once it has stood unchanged for `hold_window`. A quieter state
/// that has only just appeared is recorded and refused, so a source that
/// flips and flips back inside the window produces one transition rather than
/// two.
#[derive(Debug, Clone)]
pub struct AgentHold {
    current_state: Option<AgentState>,
    pending_state: Option<AgentState>,
    hold_until: Option<Instant>,
    hold_window: Duration,
}

impl AgentHold {
    /// Create a new hold timer with the default 700ms window.
    pub fn new() -> Self {
        Self::with_window(DEFAULT_HOLD_WINDOW)
    }

    /// Create a new hold timer with a custom window.
    pub fn with_window(hold_window: Duration) -> Self {
        Self {
            current_state: None,
            pending_state: None,
            hold_until: None,
            hold_window,
        }
    }

    /// Report a new state observation. Returns `true` if the state should be
    /// applied now (published to the window), `false` if it is being held.
    ///
    /// - A state equal to the current state drops any hold and returns `false`
    ///   (nothing to publish).
    /// - A state at or above the current loudness is published immediately.
    /// - A quieter state that has just appeared is held and returns `false`.
    /// - A quieter state that has been held for `hold_window` is published.
    pub fn report(&mut self, state: AgentState) -> bool {
        self.report_at(state, Instant::now())
    }

    /// `report` with an explicit clock, for deterministic testing.
    pub fn report_at(&mut self, state: AgentState, now: Instant) -> bool {
        let current = self.current_state.unwrap_or(AgentState::None);

        if state == current {
            // Already there: drop any hold, nothing to publish.
            self.pending_state = None;
            self.hold_until = None;
            return false;
        }

        if agent_loudness(state) >= agent_loudness(current) {
            // Louder or equal loudness: publish immediately.
            self.pending_state = None;
            self.hold_until = None;
            self.current_state = Some(state);
            return true;
        }

        // Quieter state: check if we already have a pending hold for this state.
        match (self.pending_state, self.hold_until) {
            (Some(pending), Some(until)) if pending == state => {
                if now >= until {
                    // Hold expired: publish.
                    self.pending_state = None;
                    self.hold_until = None;
                    self.current_state = Some(state);
                    return true;
                }
                // Still within the hold window.
                false
            }
            _ => {
                // New quieter state: start a hold.
                self.pending_state = Some(state);
                self.hold_until = Some(now + self.hold_window);
                false
            }
        }
    }

    /// Called on a timer tick. Returns the pending state if the hold has
    /// expired, `None` otherwise. When a hold expires, the pending state
    /// becomes the current state.
    pub fn settle(&mut self) -> Option<AgentState> {
        self.settle_at(Instant::now())
    }

    /// `settle` with an explicit clock, for deterministic testing.
    pub fn settle_at(&mut self, now: Instant) -> Option<AgentState> {
        let (pending, until) = match (self.pending_state, self.hold_until) {
            (Some(p), Some(u)) => (p, u),
            _ => return None,
        };
        if now < until {
            return None;
        }
        self.pending_state = None;
        self.hold_until = None;
        self.current_state = Some(pending);
        Some(pending)
    }

    /// The current (published) state, or `None` when nothing has been
    /// published yet.
    pub fn current(&self) -> Option<AgentState> {
        self.current_state
    }

    /// The pending (held) state, if any.
    pub fn pending(&self) -> Option<AgentState> {
        self.pending_state
    }

    /// Whether a hold is currently active (a quieter state is waiting).
    pub fn is_holding(&self) -> bool {
        self.pending_state.is_some()
    }

    /// Clear any pending hold without publishing it.
    pub fn clear_hold(&mut self) {
        self.pending_state = None;
        self.hold_until = None;
    }

    /// Reset the hold timer entirely, forgetting current and pending state.
    pub fn reset(&mut self) {
        self.current_state = None;
        self.pending_state = None;
        self.hold_until = None;
    }
}

impl Default for AgentHold {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loud_state_publishes_immediately() {
        let mut hold = AgentHold::new();
        assert!(hold.report(AgentState::Working));
        assert_eq!(hold.current(), Some(AgentState::Working));
    }

    #[test]
    fn quieter_state_is_held() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Working);
        assert!(!hold.report(AgentState::Idle));
        assert_eq!(hold.current(), Some(AgentState::Working));
        assert_eq!(hold.pending(), Some(AgentState::Idle));
        assert!(hold.is_holding());
    }

    #[test]
    fn held_state_publishes_after_window() {
        let mut hold = AgentHold::with_window(Duration::from_millis(50));
        let t0 = Instant::now();
        hold.report_at(AgentState::Working, t0);
        assert!(!hold.report_at(AgentState::Idle, t0));
        // Before the window expires: nothing.
        assert!(hold.settle_at(t0 + Duration::from_millis(40)).is_none());
        // After the window: the held state publishes.
        let settled = hold.settle_at(t0 + Duration::from_millis(60));
        assert_eq!(settled, Some(AgentState::Idle));
        assert_eq!(hold.current(), Some(AgentState::Idle));
        assert!(!hold.is_holding());
    }

    #[test]
    fn same_state_drops_hold() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Working);
        hold.report(AgentState::Idle); // start a hold
        assert!(hold.is_holding());
        // Reporting the current state again drops the hold.
        assert!(!hold.report(AgentState::Working));
        assert!(!hold.is_holding());
    }

    #[test]
    fn flip_and_flip_back_cancels_hold() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Working);
        // A quieter state starts a hold.
        assert!(!hold.report(AgentState::Idle));
        // Reporting the current (louder) state again cancels the hold.
        // Nothing new is published since current is already Working.
        assert!(!hold.report(AgentState::Working));
        assert_eq!(hold.current(), Some(AgentState::Working));
        assert!(!hold.is_holding());
    }

    #[test]
    fn needs_input_publishes_over_idle() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Idle);
        // NeedsInput is louder than Idle: publishes immediately.
        assert!(hold.report(AgentState::NeedsInput));
        assert_eq!(hold.current(), Some(AgentState::NeedsInput));
    }

    #[test]
    fn errored_publishes_over_working() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Working);
        assert!(hold.report(AgentState::Errored));
        assert_eq!(hold.current(), Some(AgentState::Errored));
    }

    #[test]
    fn done_is_quieter_than_working() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Working);
        assert!(!hold.report(AgentState::Done));
        assert_eq!(hold.pending(), Some(AgentState::Done));
    }

    #[test]
    fn loudness_ordering() {
        assert!(agent_loudness(AgentState::NeedsInput) > agent_loudness(AgentState::Working));
        assert!(agent_loudness(AgentState::Errored) > agent_loudness(AgentState::Working));
        assert!(agent_loudness(AgentState::Working) > agent_loudness(AgentState::Idle));
        assert!(agent_loudness(AgentState::Idle) > agent_loudness(AgentState::None));
    }

    #[test]
    fn settle_no_hold_returns_none() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Working);
        assert!(hold.settle().is_none());
    }

    #[test]
    fn reset_clears_everything() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Working);
        hold.report(AgentState::Idle);
        hold.reset();
        assert_eq!(hold.current(), None);
        assert!(!hold.is_holding());
    }

    #[test]
    fn clear_hold_keeps_current() {
        let mut hold = AgentHold::new();
        hold.report(AgentState::Working);
        hold.report(AgentState::Idle);
        hold.clear_hold();
        assert_eq!(hold.current(), Some(AgentState::Working));
        assert!(!hold.is_holding());
    }
}
