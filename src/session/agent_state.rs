//! Agent state infrastructure — ported from Go TUIOS `internal/session/agent_state.go`,
//! `agent_source.go`, `agent_osc.go`, `agent_hold.go`, and `agent_screen.go`.
//!
//! Tracks the semantic state of coding-agent CLIs running in window panes.
//! State is daemon-owned per-window: a pane reports its own state through
//! the verb protocol, and the daemon syncs it to attached clients.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The semantic state of an agent running in a window's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AgentState {
    /// Default: not running an agent or not reporting.
    #[default]
    None,
    /// Actively working on a task.
    Working,
    /// Blocked waiting for the user.
    NeedsInput,
    /// Not working and not blocked; output-stall heuristic.
    Idle,
    /// Finished its task.
    Done,
    /// Stopped because of an error.
    Errored,
}

impl AgentState {
    /// All accepted wire values in stable order.
    pub const NAMES: &'static [&'static str] =
        &["none", "working", "needs_input", "idle", "done", "errored"];

    /// Parse a wire value to an `AgentState`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "" | "none" => Some(Self::None),
            "working" => Some(Self::Working),
            "needs_input" => Some(Self::NeedsInput),
            "idle" => Some(Self::Idle),
            "done" => Some(Self::Done),
            "errored" => Some(Self::Errored),
            _ => None,
        }
    }

    /// Return the wire spelling, mapping `None` to `"none"`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Working => "working",
            Self::NeedsInput => "needs_input",
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Errored => "errored",
        }
    }

    /// Whether this state means a human is being waited for.
    pub fn blocks(&self) -> bool {
        matches!(self, Self::NeedsInput)
    }
}


/// Where a window's agent state came from. More than one source can want to
/// set the same pane; precedence is decided by rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AgentSource {
    /// The harness (or its hook shim) calling set-agent-state for itself.
    #[default]
    Report,
    /// An in-band escape sequence the pane emitted.
    Osc,
    /// A rule matched against the pane's rendered text.
    Screen,
    /// The foreground-process detector recognising an agent (daemon-internal).
    Detect,
    /// The output-stall heuristic, the last resort.
    Stall,
}

impl AgentSource {
    /// Accepted wire values in rank order (Detect is daemon-internal).
    pub const NAMES: &'static [&'static str] = &["report", "osc", "screen", "stall"];

    /// Parse a wire value. Empty input defaults to `Report`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "" | "report" => Some(Self::Report),
            "osc" => Some(Self::Osc),
            "screen" => Some(Self::Screen),
            "stall" => Some(Self::Stall),
            _ => None,
        }
    }

    /// Return the wire spelling.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Osc => "osc",
            Self::Screen => "screen",
            Self::Detect => "detect",
            Self::Stall => "stall",
        }
    }

    /// Rank ordering: higher rank wins. Only the ordering matters.
    pub fn rank(&self) -> i32 {
        match self {
            Self::Report => 40,
            Self::Osc => 30,
            Self::Screen => 20,
            Self::Detect => 10,
            Self::Stall => 0,
        }
    }
}


/// One source's claim on a window's agent state.
#[derive(Debug, Clone)]
pub struct AgentReport {
    pub state: AgentState,
    pub message: String,
    pub source: AgentSource,
    pub harness: String,
    /// Unix-nano time the pane last produced output, as the source read it.
    pub pane_wrote_at: i64,
}

impl Default for AgentReport {
    fn default() -> Self {
        Self {
            state: AgentState::None,
            message: String::new(),
            source: AgentSource::Report,
            harness: String::new(),
            pane_wrote_at: 0,
        }
    }
}

/// What a visible-blocker override displaced, held so it can be undone.
#[derive(Debug, Clone, Default)]
pub struct AgentPriorClaim {
    pub source: AgentSource,
    pub state: AgentState,
    pub harness: String,
}

/// Who currently owns one window's agent state.
#[derive(Debug, Clone, Default)]
pub struct AgentClaim {
    pub source: AgentSource,
    pub harness: String,
    /// The foreground-process detector promoted this window.
    pub auto: bool,
    /// A claim taken through the visible-blocker exception.
    pub blocker: bool,
    /// What the blocker displaced.
    pub prior: AgentPriorClaim,
}

/// How long a higher-ranked claim must stand without refresh before a
/// visible blocker may write over it.
pub const AGENT_BLOCKER_OVERRIDE_GRACE: Duration = Duration::from_secs(2);

/// Whether a screen rule may override a higher-ranked claim.
pub fn blocker_overrides_claim(
    current_state: AgentState,
    current_state_at: i64,
    report: &AgentReport,
    now: i64,
) -> bool {
    if report.source != AgentSource::Screen
        || !report.state.blocks()
        || current_state == report.state
    {
        return false;
    }
    if report.pane_wrote_at <= current_state_at {
        return false;
    }
    now - current_state_at >= AGENT_BLOCKER_OVERRIDE_GRACE.as_nanos() as i64
}

/// Per-window agent claim tracker.
#[derive(Debug, Default)]
pub struct AgentClaims {
    claims: HashMap<String, AgentClaim>,
}

impl AgentClaims {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a claim on a window.
    pub fn set(&mut self, window_id: &str, claim: AgentClaim) {
        self.claims.insert(window_id.to_string(), claim);
    }

    /// Get the claim on a window, or the zero claim when nothing has claimed it.
    pub fn get(&self, window_id: &str) -> AgentClaim {
        self.claims.get(window_id).cloned().unwrap_or_default()
    }

    /// Whether a window has been claimed.
    pub fn held(&self, window_id: &str) -> bool {
        self.claims.contains_key(window_id)
    }

    /// Remove a claim.
    pub fn remove(&mut self, window_id: &str) {
        self.claims.remove(window_id);
    }

    /// Whether a report from the given source may write over the current claim.
    pub fn can_write(&self, window_id: &str, source: AgentSource) -> bool {
        match self.claims.get(window_id) {
            None => true,
            Some(claim) => source.rank() >= claim.source.rank(),
        }
    }

    /// Release a visible-blocker override, putting back the prior claim.
    /// Returns true if there was an override to release.
    pub fn release_blocker(&mut self, window_id: &str) -> Option<AgentPriorClaim> {
        let claim = self.claims.get(window_id)?;
        if !claim.blocker {
            return None;
        }
        let prior = claim.prior.clone();
        let next = AgentClaim {
            source: prior.source,
            harness: prior.harness.clone(),
            auto: claim.auto,
            blocker: false,
            prior: AgentPriorClaim::default(),
        };
        self.claims.insert(window_id.to_string(), next);
        Some(prior)
    }
}

/// Whether a window has been silent since cutoff.
pub fn stalled_at(state_at: i64, pty_last_output: i64, cutoff: i64) -> bool {
    let effective = state_at.max(pty_last_output);
    effective <= cutoff
}

/// Apply the output-stall heuristic: move windows that have been silently
/// working for at least `stall` into `Idle`. Returns how many were moved.
pub fn apply_stall_heuristic(
    candidates: &[(String, i64, i64)], // (window_id, state_at, pty_last_output)
    now: i64,
    stall: Duration,
) -> Vec<String> {
    if stall.is_zero() {
        return vec![];
    }
    let cutoff = now - stall.as_nanos() as i64;
    candidates
        .iter()
        .filter(|(_, state_at, pty_last)| stalled_at(*state_at, *pty_last, cutoff))
        .map(|(id, _, _)| id.clone())
        .collect()
}

/// Parse an OSC 9;4 progress report into an agent state.
/// OSC 9;4;0 — indeterminate (working)
/// OSC 9;4;1 — normal with optional percentage
/// OSC 9;4;2 — warning
/// OSC 9;4;3 — error
/// OSC 9;4;4 — completed
pub fn osc_progress_to_state(payload: &str) -> Option<AgentState> {
    let state_code = payload.split(';').next()?;
    match state_code {
        "0" => Some(AgentState::Working), // indeterminate
        "1" => Some(AgentState::Working), // normal/running
        "2" => Some(AgentState::Working), // warning — still working
        "3" => Some(AgentState::Errored),
        "4" => Some(AgentState::Done),
        _ => None,
    }
}

/// Agent hold: a momentary "hold" that prevents the daemon from auto-clearing
/// an agent state when the foreground process changes.
#[derive(Debug, Clone)]
pub struct AgentHold {
    pub window_id: String,
    pub until: Instant,
}

impl AgentHold {
    /// Create a hold for the given duration.
    pub fn new(window_id: &str, duration: Duration) -> Self {
        Self {
            window_id: window_id.to_string(),
            until: Instant::now() + duration,
        }
    }

    /// Whether the hold is still active.
    pub fn active(&self) -> bool {
        Instant::now() < self.until
    }
}

/// A tracker for agent holds.
#[derive(Debug, Default)]
pub struct AgentHolds {
    holds: HashMap<String, AgentHold>,
}

impl AgentHolds {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a hold on a window.
    pub fn hold(&mut self, window_id: &str, duration: Duration) {
        self.holds
            .insert(window_id.to_string(), AgentHold::new(window_id, duration));
    }

    /// Whether a window is currently held.
    pub fn held(&self, window_id: &str) -> bool {
        self.holds
            .get(window_id)
            .map(|h| h.active())
            .unwrap_or(false)
    }

    /// Release a hold.
    pub fn release(&mut self, window_id: &str) {
        self.holds.remove(window_id);
    }

    /// Clean up expired holds.
    pub fn prune(&mut self) {
        self.holds.retain(|_, h| h.active());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_state() {
        assert_eq!(AgentState::parse("working"), Some(AgentState::Working));
        assert_eq!(
            AgentState::parse("needs_input"),
            Some(AgentState::NeedsInput)
        );
        assert_eq!(AgentState::parse("none"), Some(AgentState::None));
        assert_eq!(AgentState::parse(""), Some(AgentState::None));
        assert_eq!(AgentState::parse("invalid"), None);
    }

    #[test]
    fn agent_state_name() {
        assert_eq!(AgentState::None.name(), "none");
        assert_eq!(AgentState::Working.name(), "working");
        assert_eq!(AgentState::Errored.name(), "errored");
    }

    #[test]
    fn agent_state_blocks() {
        assert!(AgentState::NeedsInput.blocks());
        assert!(!AgentState::Working.blocks());
        assert!(!AgentState::Done.blocks());
    }

    #[test]
    fn parse_agent_source() {
        assert_eq!(AgentSource::parse("report"), Some(AgentSource::Report));
        assert_eq!(AgentSource::parse(""), Some(AgentSource::Report));
        assert_eq!(AgentSource::parse("osc"), Some(AgentSource::Osc));
        assert_eq!(AgentSource::parse("screen"), Some(AgentSource::Screen));
        assert_eq!(AgentSource::parse("stall"), Some(AgentSource::Stall));
        assert_eq!(AgentSource::parse("detect"), None); // not accepted from callers
        assert_eq!(AgentSource::parse("invalid"), None);
    }

    #[test]
    fn source_rank_ordering() {
        assert!(AgentSource::Report.rank() > AgentSource::Osc.rank());
        assert!(AgentSource::Osc.rank() > AgentSource::Screen.rank());
        assert!(AgentSource::Screen.rank() > AgentSource::Detect.rank());
        assert!(AgentSource::Detect.rank() > AgentSource::Stall.rank());
    }

    #[test]
    fn claims_can_write() {
        let mut claims = AgentClaims::new();
        assert!(claims.can_write("w1", AgentSource::Stall)); // unclaimed

        claims.set(
            "w1",
            AgentClaim {
                source: AgentSource::Report,
                ..Default::default()
            },
        );
        assert!(!claims.can_write("w1", AgentSource::Screen));
        assert!(claims.can_write("w1", AgentSource::Report)); // same rank
    }

    #[test]
    fn claims_release_blocker() {
        let mut claims = AgentClaims::new();
        claims.set(
            "w1",
            AgentClaim {
                source: AgentSource::Screen,
                blocker: true,
                prior: AgentPriorClaim {
                    source: AgentSource::Report,
                    state: AgentState::Working,
                    harness: "claude-code".into(),
                },
                ..Default::default()
            },
        );
        let prior = claims.release_blocker("w1");
        assert!(prior.is_some());
        let p = prior.unwrap();
        assert_eq!(p.source, AgentSource::Report);
        assert_eq!(p.state, AgentState::Working);

        // Second release returns None.
        assert!(claims.release_blocker("w1").is_none());
    }

    #[test]
    fn stalled_at_checks() {
        assert!(stalled_at(100, 0, 200));
        assert!(!stalled_at(300, 0, 200));
        // pty_last_output > state_at, so effective is 300
        assert!(!stalled_at(100, 300, 200));
        assert!(stalled_at(100, 150, 200));
    }

    #[test]
    fn apply_stall_finds_candidates() {
        let candidates = vec![
            ("w1".to_string(), 100i64, 0i64),
            ("w2".to_string(), 5_000_000_300, 0),
            ("w3".to_string(), 50, 60),
        ];
        let moved = apply_stall_heuristic(&candidates, 5_000_000_500, Duration::from_secs(2));
        assert!(moved.contains(&"w1".to_string()));
        assert!(moved.contains(&"w3".to_string()));
        assert!(!moved.contains(&"w2".to_string()));
    }

    #[test]
    fn apply_stall_zero_duration_disables() {
        let candidates = vec![("w1".to_string(), 0, 0)];
        let moved = apply_stall_heuristic(&candidates, 1_000_000_000, Duration::ZERO);
        assert!(moved.is_empty());
    }

    #[test]
    fn osc_progress_states() {
        assert_eq!(osc_progress_to_state("0"), Some(AgentState::Working));
        assert_eq!(osc_progress_to_state("1;50"), Some(AgentState::Working));
        assert_eq!(osc_progress_to_state("3"), Some(AgentState::Errored));
        assert_eq!(osc_progress_to_state("4"), Some(AgentState::Done));
        assert_eq!(osc_progress_to_state("9"), None);
    }

    #[test]
    fn blocker_override_checks() {
        let report = AgentReport {
            source: AgentSource::Screen,
            state: AgentState::NeedsInput,
            pane_wrote_at: 200,
            ..Default::default()
        };
        // Current state is Working, set at 100. Pane wrote at 200. Now is 500.
        // Grace is 2 seconds = 2_000_000_000 ns.
        assert!(blocker_overrides_claim(
            AgentState::Working,
            100,
            &report,
            100 + 3_000_000_000
        ));
        // Not enough time passed.
        assert!(!blocker_overrides_claim(
            AgentState::Working,
            100,
            &report,
            100 + 1_000_000_000
        ));
        // Pane hasn't written since claim.
        let report2 = AgentReport {
            source: AgentSource::Screen,
            state: AgentState::NeedsInput,
            pane_wrote_at: 50,
            ..Default::default()
        };
        assert!(!blocker_overrides_claim(
            AgentState::Working,
            100,
            &report2,
            100 + 3_000_000_000
        ));
    }

    #[test]
    fn agent_hold_active() {
        let mut holds = AgentHolds::new();
        holds.hold("w1", Duration::from_secs(10));
        assert!(holds.held("w1"));
        assert!(!holds.held("w2"));
        holds.release("w1");
        assert!(!holds.held("w1"));
    }

    #[test]
    fn agent_hold_prune() {
        let mut holds = AgentHolds::new();
        holds.hold("w1", Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(10));
        holds.prune();
        assert!(!holds.held("w1"));
    }
}
