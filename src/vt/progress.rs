//! OSC 9;4 progress report parsing — ported from Go TUIOS `internal/vt/progress.go`.

/// Progress state reported by an application via OSC 9;4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    /// Removes the progress indicator.
    Clear = 0,
    /// Determinate progress bar with percentage.
    Normal = 1,
    /// Operation failed.
    Error = 2,
    /// Busy indicator, no known percentage.
    Indeterminate = 3,
    /// Determinate bar flagged as needing attention.
    Warning = 4,
}

impl ProgressState {
    /// Parse a state number.
    pub fn from_i32(n: i32) -> Option<Self> {
        match n {
            0 => Some(Self::Clear),
            1 => Some(Self::Normal),
            2 => Some(Self::Error),
            3 => Some(Self::Indeterminate),
            4 => Some(Self::Warning),
            _ => None,
        }
    }
}

/// Check if an OSC 9 payload is a 9;4 progress report.
/// Only bare "4" or "4;" followed by fields qualifies.
pub fn is_progress_payload(msg: &str) -> bool {
    msg == "4" || msg.starts_with("4;")
}

/// Parse a progress payload into (state, optional percentage).
/// Formats: "4", "4;<state>", "4;<state>;<percent>".
/// No state = Clear.
pub fn parse_progress(payload: &str) -> (ProgressState, Option<u8>) {
    if payload == "4" {
        return (ProgressState::Clear, None);
    }

    let rest = match payload.strip_prefix("4;") {
        Some(r) => r,
        None => return (ProgressState::Clear, None),
    };

    let mut parts = rest.splitn(2, ';');
    let state_str = parts.next().unwrap_or("");
    let pct_str = parts.next();

    let state = state_str
        .parse::<i32>()
        .ok()
        .and_then(ProgressState::from_i32)
        .unwrap_or(ProgressState::Clear);

    let pct = pct_str
        .and_then(|s| s.trim_end_matches('\x07').parse::<u8>().ok())
        .filter(|&p| p <= 100);

    (state, pct)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_progress_bare_4() {
        assert!(is_progress_payload("4"));
    }

    #[test]
    fn is_progress_4_semicolon() {
        assert!(is_progress_payload("4;1;50"));
        assert!(is_progress_payload("4;2"));
    }

    #[test]
    fn is_progress_not_other() {
        assert!(!is_progress_payload("9;4"));
        assert!(!is_progress_payload("hello"));
        assert!(!is_progress_payload(""));
    }

    #[test]
    fn parse_bare_4() {
        let (state, pct) = parse_progress("4");
        assert_eq!(state, ProgressState::Clear);
        assert_eq!(pct, None);
    }

    #[test]
    fn parse_normal_with_percent() {
        let (state, pct) = parse_progress("4;1;50");
        assert_eq!(state, ProgressState::Normal);
        assert_eq!(pct, Some(50));
    }

    #[test]
    fn parse_error() {
        let (state, pct) = parse_progress("4;2");
        assert_eq!(state, ProgressState::Error);
        assert_eq!(pct, None);
    }

    #[test]
    fn parse_indeterminate() {
        let (state, pct) = parse_progress("4;3");
        assert_eq!(state, ProgressState::Indeterminate);
        assert_eq!(pct, None);
    }

    #[test]
    fn parse_warning_with_percent() {
        let (state, pct) = parse_progress("4;4;75");
        assert_eq!(state, ProgressState::Warning);
        assert_eq!(pct, Some(75));
    }

    #[test]
    fn parse_invalid_state() {
        let (state, _) = parse_progress("4;99");
        assert_eq!(state, ProgressState::Clear);
    }

    #[test]
    fn parse_invalid_payload() {
        let (state, _) = parse_progress("hello");
        assert_eq!(state, ProgressState::Clear);
    }

    #[test]
    fn parse_percent_over_100_capped() {
        let (_, pct) = parse_progress("4;1;150");
        assert_eq!(pct, None);
    }

    #[test]
    fn parse_with_bel_terminator() {
        let (state, pct) = parse_progress("4;1;50\x07");
        assert_eq!(state, ProgressState::Normal);
        assert_eq!(pct, Some(50));
    }
}
