//! Guest environment derivation — ported from Go TUIOS
//! `internal/guestenv/guestenv.go`.
//!
//! Derives environment values TermOS exports to the processes it spawns.
//! Both the local terminal path and the daemon's PTY path build a guest
//! environment, and they must agree on what they advertise.

/// The `TERM_PROGRAM` value for a guest process, given the graphics
/// capabilities TermOS can actually forward to the host terminal.
///
/// Tools that draw images (chafa, yazi, kitten icat) pick their output format
/// from the environment rather than by querying the terminal, and none of them
/// know the name "TermOS", so advertising it made every guest fall back to
/// unicode block art even when TermOS was forwarding kitty graphics to a
/// capable host. Naming a terminal the tools do know makes them emit the
/// protocol TermOS passes through: ghostty for kitty graphics, WezTerm for
/// sixel. TERM is left alone so no guest needs a terminfo entry that may not
/// be installed, and TermOS remains identifiable through TUIOS_SESSION and
/// TUIOS_WINDOW_ID.
pub fn term_program(kitty_graphics: bool, sixel_graphics: bool) -> &'static str {
    match (kitty_graphics, sixel_graphics) {
        (true, _) => "ghostty",
        (false, true) => "WezTerm",
        (false, false) => "TUIOS",
    }
}

/// The base environment TermOS exports to every guest shell.
pub fn base_guest_env(
    session: &str,
    window_id: &str,
    kitty_graphics: bool,
    sixel_graphics: bool,
) -> Vec<(String, String)> {
    vec![
        ("TUIOS_ENV".to_string(), "1".to_string()),
        ("TUIOS_SESSION".to_string(), session.to_string()),
        ("TUIOS_WINDOW_ID".to_string(), window_id.to_string()),
        (
            "TERM_PROGRAM".to_string(),
            term_program(kitty_graphics, sixel_graphics).to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_program_prefers_kitty() {
        assert_eq!(term_program(true, true), "ghostty");
        assert_eq!(term_program(true, false), "ghostty");
        assert_eq!(term_program(false, true), "WezTerm");
        assert_eq!(term_program(false, false), "TUIOS");
    }

    #[test]
    fn base_env_identifies_session_and_window() {
        let env = base_guest_env("work", "w1", true, false);
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(map.get("TUIOS_SESSION").unwrap(), "work");
        assert_eq!(map.get("TUIOS_WINDOW_ID").unwrap(), "w1");
        assert_eq!(map.get("TERM_PROGRAM").unwrap(), "ghostty");
        assert_eq!(map.get("TUIOS_ENV").unwrap(), "1");
    }

    #[test]
    fn legacy_keys_kept() {
        // The daemon's window spawn previously advertised TERMOS_* keys;
        // the TUIOS_* keys are the canonical ones now.
        let env = base_guest_env("s", "w", false, true);
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert!(map.contains_key("TUIOS_SESSION"));
        assert!(map.contains_key("TERM_PROGRAM"));
    }
}
