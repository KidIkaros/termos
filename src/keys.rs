//! Key-name encoding for the agent verbs (`send-keys`) and, later, tape
//! scripting. Maps human-readable key names to the byte sequences a terminal
//! app writes to a PTY: modifiers (`ctrl+x`, `alt+x`, `shift+x`), named keys,
//! and single characters.

/// Encode one key name (e.g. `enter`, `ctrl+b`, `alt+x`, `shift+tab`, `a`)
/// to the bytes a shell receives. Returns `None` for unknown names.
pub fn encode_key_name(name: &str) -> Option<Vec<u8>> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    // Modifier prefixes, applied from left to right.
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut rest = name;
    loop {
        let lower = rest.to_ascii_lowercase();
        if let Some(_tail) = lower.strip_prefix("ctrl+") {
            ctrl = true;
            rest = &rest[5..];
        } else if let Some(_tail) = lower.strip_prefix("alt+") {
            alt = true;
            rest = &rest[4..];
        } else if let Some(_tail) = lower.strip_prefix("shift+") {
            shift = true;
            rest = &rest[6..];
        } else {
            break;
        }
    }

    let base = match rest.to_ascii_lowercase().as_str() {
        "enter" | "return" | "cr" => Some(b"\r".to_vec()),
        "space" => Some(b" ".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "esc" | "escape" => Some(b"\x1b".to_vec()),
        "backspace" | "bs" => Some(b"\x7f".to_vec()),
        "delete" | "del" => Some(b"\x1b[3~".to_vec()),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "left" => Some(b"\x1b[D".to_vec()),
        "home" => Some(b"\x1b[H".to_vec()),
        "end" => Some(b"\x1b[F".to_vec()),
        "pageup" | "pgup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" | "pgdn" => Some(b"\x1b[6~".to_vec()),
        "backtab" => Some(b"\x1b[Z".to_vec()),
        _ => None,
    };

    // Single printable characters (possibly shifted).
    let mut bytes = if base.is_none() && rest.chars().count() == 1 {
        let c = rest.chars().next().unwrap();
        if c.is_ascii() && !c.is_ascii_control() {
            Some(vec![c as u8])
        } else {
            None
        }
    } else {
        base
    }?;

    // Shift+Tab is the back-tab sequence.
    if shift && bytes == b"\t".to_vec() {
        bytes = b"\x1b[Z".to_vec();
    } else if shift && bytes.len() == 1 && bytes[0].is_ascii_lowercase() {
        bytes[0] = bytes[0].to_ascii_uppercase();
    }
    if ctrl && bytes.len() == 1 {
        // Ctrl+letter → ASCII control char.
        let b = bytes[0].to_ascii_uppercase();
        if b.is_ascii_uppercase() {
            bytes = vec![b - b'A' + 1];
        } else {
            return None; // ctrl with a non-letter
        }
    }
    if alt {
        let mut out = vec![0x1b];
        out.extend_from_slice(&bytes);
        bytes = out;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys() {
        assert_eq!(encode_key_name("enter").unwrap(), b"\r");
        assert_eq!(encode_key_name("space").unwrap(), b" ");
        assert_eq!(encode_key_name("tab").unwrap(), b"\t");
        assert_eq!(encode_key_name("esc").unwrap(), b"\x1b");
        assert_eq!(encode_key_name("up").unwrap(), b"\x1b[A");
        assert_eq!(encode_key_name("pgdn").unwrap(), b"\x1b[6~");
        assert_eq!(encode_key_name("bogus-key"), None);
        assert_eq!(encode_key_name(""), None);
    }

    #[test]
    fn modifiers() {
        assert_eq!(encode_key_name("ctrl+b").unwrap(), b"\x02");
        assert_eq!(encode_key_name("ctrl+B").unwrap(), b"\x02");
        assert_eq!(encode_key_name("alt+x").unwrap(), b"\x1bx");
        assert_eq!(encode_key_name("shift+a").unwrap(), b"A");
        assert_eq!(encode_key_name("shift+tab").unwrap(), b"\x1b[Z");
        assert_eq!(encode_key_name("ctrl+alt+x").unwrap(), b"\x1b\x18");
        assert_eq!(encode_key_name("alt+enter").unwrap(), b"\x1b\r");
    }

    #[test]
    fn plain_characters() {
        assert_eq!(encode_key_name("a").unwrap(), b"a");
        assert_eq!(encode_key_name("Z").unwrap(), b"Z");
        assert_eq!(encode_key_name("1").unwrap(), b"1");
    }
}
