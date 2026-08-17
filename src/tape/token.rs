//! Tape lexer tokens — ported from TUIOS `internal/tape/token.go`.

/// The type of a token in a `.tape` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Eof,
    Illegal,
    Comment,
    Newline,
    String,
    Number,
    Duration,
    Identifier,
    Plus,
    At,
    Comma,
    Slash,
    LParen,
    RParen,
    // Commands (keyword tokens carry the command's spelling as their type).
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
    Ctrl,
    Alt,
    Shift,
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
    Wait,
    WaitUntilRegex,
    Set,
    Output,
    Source,
    EnableAnimations,
    DisableAnimations,
    ToggleAnimations,
    RotateSplit,
    EqualizeSplits,
    ToggleZoom,
    SmartSplit,
    CommandPalette,
    SaveLayout,
    LoadLayout,
    True,
    False,
}

/// A lexical token with position tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub type_: TokenType,
    pub literal: String,
    pub line: usize,
    pub column: usize,
}

impl TokenType {
    /// True if this token type is a command keyword.
    pub fn is_command(self) -> bool {
        use TokenType::*;
        matches!(
            self,
            Type | Sleep
                | Enter
                | Space
                | Backspace
                | Delete
                | Tab
                | Escape
                | Up
                | Down
                | Left
                | Right
                | Home
                | End
                | Ctrl
                | Alt
                | Shift
                | TerminalMode
                | WindowManagementMode
                | NewWindow
                | CloseWindow
                | NextWindow
                | PrevWindow
                | FocusWindow
                | RenameWindow
                | MinimizeWindow
                | RestoreWindow
                | ToggleTiling
                | EnableTiling
                | DisableTiling
                | SnapLeft
                | SnapRight
                | SnapFullscreen
                | SwitchWorkspace
                | MoveToWorkspace
                | MoveAndFollowWorkspace
                | Split
                | Focus
                | RotateSplit
                | EqualizeSplits
                | ToggleZoom
                | SmartSplit
                | CommandPalette
                | SaveLayout
                | LoadLayout
                | Wait
                | WaitUntilRegex
                | Set
                | Output
                | Source
                | EnableAnimations
                | DisableAnimations
                | ToggleAnimations
        )
    }

    /// True if this token is a modifier key.
    pub fn is_modifier(self) -> bool {
        matches!(self, TokenType::Ctrl | TokenType::Alt | TokenType::Shift)
    }

    /// True if this token is a navigation key.
    pub fn is_navigation_key(self) -> bool {
        matches!(
            self,
            TokenType::Up
                | TokenType::Down
                | TokenType::Left
                | TokenType::Right
                | TokenType::Home
                | TokenType::End
        )
    }
}

/// Look up a keyword by (case-insensitive) spelling, or `Identifier`.
pub fn lookup_keyword(ident: &str) -> TokenType {
    KEYWORD_MAP
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(ident))
        .map(|(_, tt)| *tt)
        .unwrap_or(TokenType::Identifier)
}

/// The keyword → token table (the Rust counterpart of Go's `KeywordTokenMap`).
pub const KEYWORD_MAP: &[(&str, TokenType)] = &[
    ("Type", TokenType::Type),
    ("Sleep", TokenType::Sleep),
    ("Enter", TokenType::Enter),
    ("Space", TokenType::Space),
    ("Backspace", TokenType::Backspace),
    ("Delete", TokenType::Delete),
    ("Tab", TokenType::Tab),
    ("Escape", TokenType::Escape),
    ("Up", TokenType::Up),
    ("Down", TokenType::Down),
    ("Left", TokenType::Left),
    ("Right", TokenType::Right),
    ("Home", TokenType::Home),
    ("End", TokenType::End),
    ("Ctrl", TokenType::Ctrl),
    ("Alt", TokenType::Alt),
    ("Shift", TokenType::Shift),
    ("TerminalMode", TokenType::TerminalMode),
    ("WindowManagementMode", TokenType::WindowManagementMode),
    ("NewWindow", TokenType::NewWindow),
    ("CloseWindow", TokenType::CloseWindow),
    ("NextWindow", TokenType::NextWindow),
    ("PrevWindow", TokenType::PrevWindow),
    ("FocusWindow", TokenType::FocusWindow),
    ("RenameWindow", TokenType::RenameWindow),
    ("MinimizeWindow", TokenType::MinimizeWindow),
    ("RestoreWindow", TokenType::RestoreWindow),
    ("ToggleTiling", TokenType::ToggleTiling),
    ("EnableTiling", TokenType::EnableTiling),
    ("DisableTiling", TokenType::DisableTiling),
    ("SnapLeft", TokenType::SnapLeft),
    ("SnapRight", TokenType::SnapRight),
    ("SnapFullscreen", TokenType::SnapFullscreen),
    ("SwitchWorkspace", TokenType::SwitchWorkspace),
    ("MoveToWorkspace", TokenType::MoveToWorkspace),
    ("MoveAndFollowWorkspace", TokenType::MoveAndFollowWorkspace),
    ("Split", TokenType::Split),
    ("Focus", TokenType::Focus),
    ("RotateSplit", TokenType::RotateSplit),
    ("EqualizeSplits", TokenType::EqualizeSplits),
    ("ToggleZoom", TokenType::ToggleZoom),
    ("SmartSplit", TokenType::SmartSplit),
    ("CommandPalette", TokenType::CommandPalette),
    ("SaveLayout", TokenType::SaveLayout),
    ("LoadLayout", TokenType::LoadLayout),
    ("Wait", TokenType::Wait),
    ("WaitUntilRegex", TokenType::WaitUntilRegex),
    ("Set", TokenType::Set),
    ("Output", TokenType::Output),
    ("Source", TokenType::Source),
    ("EnableAnimations", TokenType::EnableAnimations),
    ("DisableAnimations", TokenType::DisableAnimations),
    ("ToggleAnimations", TokenType::ToggleAnimations),
    ("true", TokenType::True),
    ("false", TokenType::False),
];
