//! Tape command model — ported from TUIOS `internal/tape/command.go`.

use std::time::Duration;

/// The type of a tape command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandType {
    Type,
    Sleep,
    Enter,
    Space,
    Backspace,
    Delete,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    KeyCombo,
    TerminalMode,
    WindowManagementMode,
    NewWindow,
    CloseWindow,
    NextWindow,
    PrevWindow,
    FocusWindow,
    RenameWindow,
    MinimizeWindow,
    RestoreWindow,
    ToggleTiling,
    EnableTiling,
    DisableTiling,
    SnapLeft,
    SnapRight,
    SnapFullscreen,
    SwitchWorkspace,
    MoveToWorkspace,
    MoveAndFollowWorkspace,
    Split,
    Focus,
    RotateSplit,
    EqualizeSplits,
    Preselect,
    Wait,
    WaitUntilRegex,
    Set,
    Output,
    Source,
    EnableAnimations,
    DisableAnimations,
    ToggleAnimations,
    Comment,
    SetConfig,
    SetTheme,
    SetDockbarPosition,
    SetBorderStyle,
    ShowNotification,
    FocusDirection,
    ToggleZoom,
    SmartSplit,
    CommandPalette,
    SaveLayout,
    LoadLayout,
}

impl CommandType {
    /// True if this is a valid command type.
    pub fn is_command(self) -> bool {
        // Every variant of the enum is a command by construction.
        true
    }
}

/// A parsed tape command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Command {
    pub type_: CommandType,
    /// Command arguments.
    pub args: Vec<String>,
    /// Delay after this command (e.g. `Type@100ms`, or the Sleep duration).
    pub delay: Duration,
    /// Source line number.
    pub line: usize,
    /// Source column number.
    pub column: usize,
    /// Original raw command text.
    pub raw: String,
}

impl Command {
    pub fn new(type_: CommandType, line: usize, column: usize) -> Self {
        Self {
            type_,
            args: Vec::new(),
            delay: Duration::ZERO,
            line,
            column,
            raw: String::new(),
        }
    }

    pub fn string(&self) -> String {
        match self.type_ {
            CommandType::Type => format!("Type {:?}", self.args),
            CommandType::Sleep => format!("Sleep {:?}", self.args),
            CommandType::KeyCombo => self.args.join(" "),
            CommandType::SwitchWorkspace => format!("SwitchWorkspace {:?}", self.args),
            other => format!("{other:?} {:?}", self.args),
        }
    }
}

/// Parse a Go-style duration string (`500ms`, `1.5s`, `1m30s`, `1h2m3s`).
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total = Duration::ZERO;
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut any = false;
    while i < bytes.len() {
        // Parse the numeric part (with optional decimal point).
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        if i == start {
            return None; // expected a number
        }
        let num: f64 = s[start..i].parse().ok()?;
        // Parse the unit.
        let unit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i == unit_start {
            return None; // expected a unit
        }
        let unit = &s[unit_start..i];
        let nanos = match unit {
            "ns" => num,
            "us" | "µs" => num * 1_000.0,
            "ms" => num * 1_000_000.0,
            "s" => num * 1_000_000_000.0,
            "m" => num * 60.0 * 1_000_000_000.0,
            "h" => num * 3600.0 * 1_000_000_000.0,
            _ => return None,
        };
        total = total
            .checked_add(Duration::from_nanos(nanos.round().max(0.0) as u64))?;
        any = true;
    }
    if any {
        Some(total)
    } else {
        None
    }
}

/// A key combination (e.g. Ctrl+B, Alt+1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The key itself (`b`, `1`, ...).
    pub key: String,
}

impl KeyCombo {
    pub fn string(&self) -> String {
        let mut result = String::new();
        if self.ctrl {
            result.push_str("Ctrl+");
        }
        if self.alt {
            result.push_str("Alt+");
        }
        if self.shift {
            result.push_str("Shift+");
        }
        result.push_str(&self.key);
        result
    }
}

/// Parse a key combo string like `Ctrl+B` or `Alt+Shift+1`.
pub fn parse_key_combo(s: &str) -> Result<KeyCombo, String> {
    let parts: Vec<&str> = s.split('+').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err("empty key combo".into());
    }
    let mut kc = KeyCombo {
        key: parts[parts.len() - 1].to_string(),
        ..KeyCombo::default()
    };
    for m in &parts[..parts.len() - 1] {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" => kc.ctrl = true,
            "alt" | "opt" => kc.alt = true,
            "shift" => kc.shift = true,
            other => return Err(format!("unknown modifier: {other}")),
        }
    }
    Ok(kc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration("2s"), Some(Duration::from_secs(2)));
        assert_eq!(parse_duration("1.5s"), Some(Duration::from_millis(1500)));
        assert_eq!(parse_duration("1m30s"), Some(Duration::from_secs(90)));
        assert_eq!(
            parse_duration("1h2m3s"),
            Some(Duration::from_secs(3600 + 120 + 3))
        );
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("10"), None);
        assert_eq!(parse_duration("abc"), None);
    }

    #[test]
    fn key_combos() {
        let kc = parse_key_combo("Ctrl+B").unwrap();
        assert!(kc.ctrl);
        assert_eq!(kc.key, "B");
        let kc = parse_key_combo("Alt+Shift+1").unwrap();
        assert!(kc.alt);
        assert!(kc.shift);
        assert_eq!(kc.key, "1");
        let kc = parse_key_combo("opt+x").unwrap();
        assert!(kc.alt);
        assert_eq!(kc.key, "x");
        assert!(parse_key_combo("Bogus+x").is_err());
        assert!(parse_key_combo("").is_err());
        assert_eq!(kc.string(), "Alt+x");
    }
}
