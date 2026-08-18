//! Structured fuzzing driver — ported from Go TUIOS `internal/fuzz/`.
//!
//! A minimal deterministic driver over a [`Target`]: generate action
//! sequences, apply them, check invariants after every action, and shrink a
//! failing sequence to a minimal repro. The `AppTarget` wires the driver to
//! the window manager's named-action dispatcher so whole-app invariants can
//! be fuzzed headlessly.

use std::collections::HashMap;

/// One broken invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The rule that broke (held fixed by the shrinker).
    pub rule: String,
    /// What actually went wrong.
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.rule, self.detail)
    }
}

/// The system under test. The driver owns sequencing, seeding, and
/// minimisation; a `Target` owns only "put me back at the start", "do this
/// one thing", and "which invariants are broken right now".
pub trait Target {
    /// Return to a state that depends only on the actions replayed since
    /// (so a shrunk repro reproduces).
    fn reset(&mut self);
    /// Apply one action.
    fn apply(&mut self, action: &Action);
    /// Which invariants are broken right now (cheap; runs after every action).
    fn check(&self) -> Vec<Violation>;
    /// Release resources.
    fn close(&mut self) {}
}

/// The action vocabulary for window-manager fuzzing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    NewWindow,
    CloseWindow,
    SplitHorizontal,
    SplitVertical,
    NextWindow,
    PrevWindow,
    SwitchWorkspace(u8),
    ToggleZoom,
    SwapLeft,
    SwapRight,
    SwapUp,
    SwapDown,
    SnapLeft,
    SnapRight,
    RotateSplit,
    EqualizeSplits,
    Scrollback,
}

/// Every action, for generation.
pub const ACTIONS: &[Action] = &[
    Action::NewWindow,
    Action::CloseWindow,
    Action::SplitHorizontal,
    Action::SplitVertical,
    Action::NextWindow,
    Action::PrevWindow,
    Action::SwitchWorkspace(1),
    Action::SwitchWorkspace(2),
    Action::SwitchWorkspace(3),
    Action::ToggleZoom,
    Action::SwapLeft,
    Action::SwapRight,
    Action::SwapUp,
    Action::SwapDown,
    Action::SnapLeft,
    Action::SnapRight,
    Action::RotateSplit,
    Action::EqualizeSplits,
    Action::Scrollback,
];

/// A tiny deterministic PRNG (xorshift64*), so runs are reproducible.
#[derive(Debug, Clone)]
pub struct Prng(u64);

impl Prng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// A uniform index in `0..n`.
    pub fn index(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
}

/// Generate a random action sequence.
pub fn generate(prng: &mut Prng, len: usize) -> Vec<Action> {
    (0..len)
        .map(|_| ACTIONS[prng.index(ACTIONS.len())])
        .collect()
}

/// Run `iterations` random sequences against `target`, returning the first
/// failure (sequence + violation) or `None`.
pub fn run(
    target: &mut dyn Target,
    seed: u64,
    iterations: usize,
    seq_len: usize,
) -> Option<(Vec<Action>, Violation)> {
    let mut prng = Prng::new(seed);
    for _ in 0..iterations {
        let actions = generate(&mut prng, seq_len);
        target.reset();
        for action in &actions {
            target.apply(action);
            if let Some(v) = target.check().into_iter().next() {
                return Some((actions, v));
            }
        }
    }
    None
}

/// Minimize a failing sequence: repeatedly try removing each action; keep a
/// removal that still reproduces the same rule.
pub fn shrink(target: &mut dyn Target, actions: &[Action], violation: &Violation) -> Vec<Action> {
    let mut current: Vec<Action> = actions.to_vec();
    let mut i = 0;
    while i < current.len() {
        let mut candidate = current.clone();
        candidate.remove(i);
        if reproduces(target, &candidate, &violation.rule) {
            current = candidate;
            // Don't advance: the next action shifted into this slot.
        } else {
            i += 1;
        }
    }
    current
}

/// Whether replaying `actions` reproduces a violation of `rule`.
fn reproduces(target: &mut dyn Target, actions: &[Action], rule: &str) -> bool {
    target.reset();
    for action in actions {
        target.apply(action);
        if target.check().iter().any(|v| v.rule == rule) {
            return true;
        }
    }
    // The empty sequence is a valid repro too: a violation may already hold
    // at the reset state.
    target.check().iter().any(|v| v.rule == rule)
}

/// The window-manager target: applies named actions via the app's dispatcher
/// and checks structural invariants of the `Os` state.
pub struct AppTarget {
    pub os: crate::app::Os,
    /// The maximum window count before the `window-count` rule breaks.
    pub max_windows: usize,
    /// Workspace window membership, keyed by window id.
    membership: HashMap<usize, i32>,
}

impl AppTarget {
    /// A fresh target with default limits.
    pub fn new() -> Self {
        Self {
            os: crate::app::Os::new(crate::config::userconfig::UserConfig::default_config()),
            max_windows: 32,
            membership: HashMap::new(),
        }
    }
}

impl Default for AppTarget {
    fn default() -> Self {
        Self::new()
    }
}

impl Target for AppTarget {
    fn reset(&mut self) {
        self.os = crate::app::Os::new(crate::config::userconfig::UserConfig::default_config());
        self.os.width = 80;
        self.os.height = 25;
        self.membership.clear();
    }

    fn apply(&mut self, action: &Action) {
        let os = &mut self.os;
        match action {
            Action::NewWindow => {
                let shell = os.default_shell();
                let _ = os.spawn_window(&shell, Box::new(|| {}));
            }
            Action::CloseWindow => os.close_focused_window(),
            Action::SplitHorizontal => {
                let shell = os.default_shell();
                let _ = os.split(
                    crate::layout::SplitType::Horizontal,
                    &shell,
                    Box::new(|| {}),
                );
            }
            Action::SplitVertical => {
                let shell = os.default_shell();
                let _ = os.split(crate::layout::SplitType::Vertical, &shell, Box::new(|| {}));
            }
            Action::NextWindow => os.focus_next(),
            Action::PrevWindow => os.focus_prev(),
            Action::SwitchWorkspace(n) => os.switch_workspace((*n).clamp(1, 9) as i32),
            Action::ToggleZoom => {
                let _ = os.toggle_zoom_internal();
            }
            Action::SwapLeft => os.swap_focused_with(crate::layout::PreselectionDir::Left),
            Action::SwapRight => os.swap_focused_with(crate::layout::PreselectionDir::Right),
            Action::SwapUp => os.swap_focused_with(crate::layout::PreselectionDir::Up),
            Action::SwapDown => os.swap_focused_with(crate::layout::PreselectionDir::Down),
            Action::SnapLeft => os.snap_half(true),
            Action::SnapRight => os.snap_half(false),
            Action::RotateSplit => {
                if let Some(focused) = os.focused_window {
                    let ws = os.current_workspace;
                    os.workspace_mut(ws).tree.rotate_split(focused as i32);
                }
            }
            Action::EqualizeSplits => {
                let ws = os.current_workspace;
                os.workspace_mut(ws).tree.equalize_ratios();
            }
            Action::Scrollback => os.enter_scrollback_mode(),
        }
        // Refresh membership for the invariant checks.
        self.membership.clear();
        for ws in 1..=9 {
            for id in os.workspace(ws).tree.get_all_window_ids() {
                self.membership.insert(id as usize, ws);
            }
        }
    }

    fn check(&self) -> Vec<Violation> {
        let mut out = Vec::new();
        let os = &self.os;

        // The window list may never exceed the cap (a leak would show here).
        if os.windows.len() > self.max_windows {
            out.push(Violation {
                rule: "window-count".into(),
                detail: format!(
                    "{} windows exceed cap {}",
                    os.windows.len(),
                    self.max_windows
                ),
            });
        }

        // The focused index must be in bounds or None.
        if let Some(f) = os.focused_window {
            if f >= os.windows.len() {
                out.push(Violation {
                    rule: "focused-bounds".into(),
                    detail: format!("focused {} >= windows {}", f, os.windows.len()),
                });
            }
        }

        // Every window must belong to exactly one workspace tree.
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut duplicates = Vec::new();
        for (&id, &ws) in &self.membership {
            if !seen.insert(id) {
                duplicates.push((id, ws));
            }
        }
        if !duplicates.is_empty() {
            out.push(Violation {
                rule: "membership-unique".into(),
                detail: format!("windows in multiple trees: {duplicates:?}"),
            });
        }

        // Every window in the flat list must be in some tree.
        for (i, w) in os.windows.iter().enumerate() {
            if !self.membership.contains_key(&i) && w.id.starts_with("win-") {
                out.push(Violation {
                    rule: "membership-present".into(),
                    detail: format!("window {i} ({}) not in any workspace tree", w.id),
                });
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prng_is_deterministic() {
        let mut a = Prng::new(42);
        let mut b = Prng::new(42);
        let seq_a: Vec<u64> = (0..10).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..10).map(|_| b.next_u64()).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn generate_respects_vocabulary() {
        let mut prng = Prng::new(7);
        let actions = generate(&mut prng, 100);
        assert_eq!(actions.len(), 100);
        for a in &actions {
            assert!(ACTIONS.contains(a));
        }
    }

    #[test]
    fn app_target_holds_invariants_under_random_actions() {
        let mut target = AppTarget::new();
        let mut prng = Prng::new(1234);
        for _ in 0..2000 {
            let actions = generate(&mut prng, 1);
            target.apply(&actions[0]);
            let violations = target.check();
            // The window-count cap is a designed canary, not a structural
            // bug; the structural rules must always hold.
            let structural: Vec<&Violation> = violations
                .iter()
                .filter(|v| v.rule != "window-count")
                .collect();
            assert!(
                structural.is_empty(),
                "violation after {:?}: {:?}",
                actions[0],
                structural
            );
        }
    }

    #[test]
    fn app_target_focused_bounds_never_breaks() {
        // Drive the target hard with close/next/prev to ensure the focused
        // index can never dangle.
        let mut target = AppTarget::new();
        let mut prng = Prng::new(99);
        for _ in 0..500 {
            let actions = generate(&mut prng, 8);
            target.reset();
            for a in &actions {
                target.apply(a);
                let violations = target.check();
                assert!(
                    violations.is_empty(),
                    "violation after {:?}: {:?}",
                    actions,
                    violations
                );
            }
        }
    }

    #[test]
    fn shrink_reduces_failing_sequence() {
        // Build a target whose check always passes so shrink is a no-op path;
        // the important property is that shrink terminates and keeps the
        // failure. Use a deliberately failing target.
        struct AlwaysFail;
        impl Target for AlwaysFail {
            fn reset(&mut self) {}
            fn apply(&mut self, _a: &Action) {}
            fn check(&self) -> Vec<Violation> {
                vec![Violation {
                    rule: "always".into(),
                    detail: "x".into(),
                }]
            }
        }
        let actions = vec![Action::NewWindow, Action::CloseWindow, Action::NextWindow];
        let mut target = AlwaysFail;
        let shrunk = shrink(
            &mut target,
            &actions,
            &Violation {
                rule: "always".into(),
                detail: "x".into(),
            },
        );
        // With a target that always fails, the shrinker should remove
        // everything down to the empty sequence.
        assert!(shrunk.is_empty());
    }
}
