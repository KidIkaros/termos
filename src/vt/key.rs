//! Key encoding for guest terminal input — ported from Go TUIOS `internal/vt/key.go`.
//!
//! Encodes key presses as escape sequences for sending to PTY child processes.

/// Key modifier bitflags.
pub const MOD_NONE: u8 = 0;
pub const MOD_SHIFT: u8 = 1;
pub const MOD_ALT: u8 = 2;
pub const MOD_CONTROL: u8 = 4;
pub const MOD_SUPER: u8 = 8;
pub const MOD_META: u8 = 16;
pub const MOD_HYPER: u8 = 32;

/// Encode a key press as escape sequences.
///
/// `key` is the key name (e.g., "up", "down", "enter", "a").
/// `modifiers` is a bitmask of MOD_* constants.
/// `kitty_protocol` selects between legacy and kitty encoding.
pub fn encode_key(key: &str, modifiers: u8, kitty_protocol: bool) -> Vec<u8> {
    let key = normalize_key_name(key);

    // No modifier and printable: send raw byte.
    if modifiers == MOD_NONE {
        if let Some(byte) = simple_key_byte(key) {
            return vec![byte];
        }
    }

    // Special keys with CSI encoding.
    if let Some(code) = special_key_code(key) {
        if kitty_protocol {
            return encode_kitty(code, modifiers);
        }
        return encode_csi(code, modifiers);
    }

    // Fallback: if it's a single char with modifiers, encode it.
    if key.len() == 1 {
        let ch = key.chars().next().unwrap();
        if modifiers & MOD_ALT != 0 {
            // Alt prefix: ESC + char
            let mut result = vec![0x1b];
            if modifiers & MOD_CONTROL != 0 {
                result.push(encode_ctrl_char(ch));
            } else {
                result.push(ch as u8);
            }
            return result;
        }
        if modifiers & MOD_CONTROL != 0 {
            return vec![encode_ctrl_char(ch)];
        }
        return vec![ch as u8];
    }

    Vec::new()
}

/// Normalize a key name to canonical form.
pub fn normalize_key_name(name: &str) -> &str {
    match name.to_lowercase().as_str() {
        "return" => "enter",
        "escape" => "esc",
        "backspace" => "bspace",
        "tab" => "tab",
        "space" => "space",
        "up" => "up",
        "down" => "down",
        "left" => "left",
        "right" => "right",
        "home" => "home",
        "end" => "end",
        "insert" => "insert",
        "delete" => "delete",
        "pageup" | "page_up" | "pgup" => "pageup",
        "pagedown" | "page_down" | "pgdn" => "pagedown",
        "f1" => "f1",
        "f2" => "f2",
        "f3" => "f3",
        "f4" => "f4",
        "f5" => "f5",
        "f6" => "f6",
        "f7" => "f7",
        "f8" => "f8",
        "f9" => "f9",
        "f10" => "f10",
        "f11" => "f11",
        "f12" => "f12",
        _ => name,
    }
}

fn simple_key_byte(key: &str) -> Option<u8> {
    match key {
        "enter" => Some(0x0d),
        "tab" => Some(0x09),
        "bspace" => Some(0x7f),
        "esc" => Some(0x1b),
        "space" => Some(0x20),
        _ => None,
    }
}

fn special_key_code(key: &str) -> Option<u8> {
    match key {
        "up" => Some(0x41),
        "down" => Some(0x42),
        "right" => Some(0x43),
        "left" => Some(0x44),
        "home" => Some(0x48),
        "end" => Some(0x46),
        "insert" => Some(0x32),
        "delete" => Some(0x33),
        "pageup" => Some(0x35),
        "pagedown" => Some(0x36),
        _ => None,
    }
}

fn encode_csi(code: u8, modifiers: u8) -> Vec<u8> {
    if modifiers == MOD_NONE {
        // Simple: ESC[code~
        // For arrows: ESC[code
        match code {
            0x41..=0x44 | 0x48 | 0x46 => format!("\x1b[{}", code as char).into_bytes(),
            0x32 | 0x33 | 0x35 | 0x36 => format!("\x1b[{}~", code as char).into_bytes(),
            _ => format!("\x1b[{}~", code as char).into_bytes(),
        }
    } else {
        // With modifiers: ESC[1;{mod}code
        let mod_num = modifier_number(modifiers);
        format!("\x1b[1;{}{}", mod_num, code as char).into_bytes()
    }
}

fn encode_kitty(code: u8, modifiers: u8) -> Vec<u8> {
    let mod_num = modifier_number(modifiers);
    format!("\x1b[{};{}u", code, mod_num).into_bytes()
}

fn modifier_number(modifiers: u8) -> u8 {
    // Terminal modifier encoding: 1 + sum of modifier bits
    1 + modifiers
}

fn encode_ctrl_char(ch: char) -> u8 {
    let c = ch as u8;
    if c.is_ascii_uppercase() {
        c - b'A' + 1
    } else if c.is_ascii_lowercase() {
        c - b'a' + 1
    } else if c == b'@' || c == b'`' {
        0
    } else if (b'['..=b'_').contains(&c) {
        c - b'[' + 27
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_no_modifiers() {
        assert_eq!(encode_key("enter", MOD_NONE, false), vec![0x0d]);
    }

    #[test]
    fn tab_no_modifiers() {
        assert_eq!(encode_key("tab", MOD_NONE, false), vec![0x09]);
    }

    #[test]
    fn escape_no_modifiers() {
        assert_eq!(encode_key("esc", MOD_NONE, false), vec![0x1b]);
    }

    #[test]
    fn backspace_no_modifiers() {
        assert_eq!(encode_key("backspace", MOD_NONE, false), vec![0x7f]);
    }

    #[test]
    fn up_no_modifiers() {
        assert_eq!(encode_key("up", MOD_NONE, false), b"\x1b[A");
    }

    #[test]
    fn down_no_modifiers() {
        assert_eq!(encode_key("down", MOD_NONE, false), b"\x1b[B");
    }

    #[test]
    fn up_with_shift() {
        assert_eq!(encode_key("up", MOD_SHIFT, false), b"\x1b[1;2A");
    }

    #[test]
    fn up_with_alt() {
        assert_eq!(encode_key("up", MOD_ALT, false), b"\x1b[1;3A");
    }

    #[test]
    fn up_with_ctrl() {
        assert_eq!(encode_key("up", MOD_CONTROL, false), b"\x1b[1;5A");
    }

    #[test]
    fn up_kitty_protocol() {
        assert_eq!(encode_key("up", MOD_NONE, true), b"\x1b[65;1u");
    }

    #[test]
    fn up_kitty_with_shift() {
        assert_eq!(encode_key("up", MOD_SHIFT, true), b"\x1b[65;2u");
    }

    #[test]
    fn alt_plus_a() {
        assert_eq!(encode_key("a", MOD_ALT, false), vec![0x1b, b'a']);
    }

    #[test]
    fn ctrl_plus_a() {
        assert_eq!(encode_key("a", MOD_CONTROL, false), vec![0x01]);
    }

    #[test]
    fn ctrl_plus_uppercase_a() {
        assert_eq!(encode_key("A", MOD_CONTROL, false), vec![0x01]);
    }

    #[test]
    fn normalize_return() {
        assert_eq!(normalize_key_name("return"), "enter");
    }

    #[test]
    fn normalize_escape() {
        assert_eq!(normalize_key_name("escape"), "esc");
    }

    #[test]
    fn normalize_backspace() {
        assert_eq!(normalize_key_name("backspace"), "bspace");
    }

    #[test]
    fn normalize_page_up() {
        assert_eq!(normalize_key_name("pageup"), "pageup");
        assert_eq!(normalize_key_name("page_up"), "pageup");
        assert_eq!(normalize_key_name("pgup"), "pageup");
    }

    #[test]
    fn insert_no_modifiers() {
        assert_eq!(encode_key("insert", MOD_NONE, false), b"\x1b[2~");
    }

    #[test]
    fn delete_no_modifiers() {
        assert_eq!(encode_key("delete", MOD_NONE, false), b"\x1b[3~");
    }

    #[test]
    fn pageup_no_modifiers() {
        assert_eq!(encode_key("pageup", MOD_NONE, false), b"\x1b[5~");
    }

    #[test]
    fn pagedown_no_modifiers() {
        assert_eq!(encode_key("pagedown", MOD_NONE, false), b"\x1b[6~");
    }

    #[test]
    fn home_no_modifiers() {
        assert_eq!(encode_key("home", MOD_NONE, false), b"\x1b[H");
    }

    #[test]
    fn end_no_modifiers() {
        assert_eq!(encode_key("end", MOD_NONE, false), b"\x1b[F");
    }
}
