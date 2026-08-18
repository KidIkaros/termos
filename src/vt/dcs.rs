//! DCS (Device Control String) sequence handlers — ported from Go TUIOS `internal/vt/dcs.go`.
//!
//! Parses and dispatches DCS sequences for Sixel, DECRQSS, and tmux
//! passthrough.

/// The kind of DCS command identified by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DcsCommand {
    /// Sixel graphics data (`DCS q ... ST`).
    Sixel,
    /// DECRQSS — request status string (`DCS $ p ... ST`).
    Decrqss,
    /// tmux passthrough (`DCS t T ... ST`).
    TmuxPassthrough,
    /// Kitty graphics (`DCS | q ... ST` or similar).
    Kitty,
    /// Unknown/unhandled DCS command.
    Other,
}

/// The result of handling a DCS sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DcsResult {
    /// The sequence was handled; bytes may need to be sent as a response.
    Handled(Vec<u8>),
    /// The payload should be passed through to the host terminal.
    Passthrough(Vec<u8>),
    /// The sequence was not recognized.
    Unhandled,
}

/// Parse a DCS command from its intermediate and final bytes.
///
/// DCS sequences have the form: `ESC P <intermediates> <final> <data> ST`
pub fn parse_dcs_command(intermediates: &[u8], final_byte: u8) -> DcsCommand {
    match final_byte {
        b'q' => {
            // Sixel if no intermediates, Kitty if '|' intermediate
            if intermediates.contains(&b'|') {
                DcsCommand::Kitty
            } else {
                DcsCommand::Sixel
            }
        }
        b'p' => {
            if intermediates.contains(&b'$') {
                DcsCommand::Decrqss
            } else {
                DcsCommand::Other
            }
        }
        b'T' => {
            if intermediates.contains(&b't') {
                DcsCommand::TmuxPassthrough
            } else {
                DcsCommand::Other
            }
        }
        _ => DcsCommand::Other,
    }
}

/// Handle a DCS sequence, returning the result.
pub fn handle_dcs(data: &[u8], command: &DcsCommand) -> DcsResult {
    match command {
        DcsCommand::Sixel => {
            // Sixel data should be passed through to the host terminal
            // for rendering.
            DcsResult::Passthrough(data.to_vec())
        }
        DcsCommand::Kitty => {
            // Kitty graphics passthrough.
            DcsResult::Passthrough(data.to_vec())
        }
        DcsCommand::Decrqss => {
            // DECRQSS: the terminal should respond with the current setting.
            // For now, return an empty response (handled).
            DcsResult::Handled(Vec::new())
        }
        DcsCommand::TmuxPassthrough => {
            // tmux passthrough: forward the data to the host.
            DcsResult::Passthrough(data.to_vec())
        }
        DcsCommand::Other => DcsResult::Unhandled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sixel() {
        assert_eq!(parse_dcs_command(&[], b'q'), DcsCommand::Sixel);
    }

    #[test]
    fn parse_kitty() {
        assert_eq!(parse_dcs_command(b"|", b'q'), DcsCommand::Kitty);
    }

    #[test]
    fn parse_decrqss() {
        assert_eq!(parse_dcs_command(b"$", b'p'), DcsCommand::Decrqss);
    }

    #[test]
    fn parse_tmux_passthrough() {
        assert_eq!(parse_dcs_command(b"t", b'T'), DcsCommand::TmuxPassthrough);
    }

    #[test]
    fn parse_unknown() {
        assert_eq!(parse_dcs_command(&[], b'z'), DcsCommand::Other);
    }

    #[test]
    fn handle_sixel_passthrough() {
        let result = handle_dcs(b"!1!1~", &DcsCommand::Sixel);
        assert_eq!(result, DcsResult::Passthrough(b"!1!1~".to_vec()));
    }

    #[test]
    fn handle_kitty_passthrough() {
        let result = handle_dcs(b"some kitty data", &DcsCommand::Kitty);
        assert_eq!(result, DcsResult::Passthrough(b"some kitty data".to_vec()));
    }

    #[test]
    fn handle_decrqss_handled() {
        let result = handle_dcs(b"m", &DcsCommand::Decrqss);
        assert_eq!(result, DcsResult::Handled(Vec::new()));
    }

    #[test]
    fn handle_tmux_passthrough() {
        let result = handle_dcs(b"raw data", &DcsCommand::TmuxPassthrough);
        assert_eq!(result, DcsResult::Passthrough(b"raw data".to_vec()));
    }

    #[test]
    fn handle_other_unhandled() {
        let result = handle_dcs(b"data", &DcsCommand::Other);
        assert_eq!(result, DcsResult::Unhandled);
    }
}
