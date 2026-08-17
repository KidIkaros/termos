//! Input handling — the modal key dispatcher. Ported from TUIOS
//! `internal/app` (mode switch + prefix handling) and `internal/input`.
//!
//! Keys flow through three layers depending on mode and prefix state:
//! 1. A pending prefix/sub-prefix consumes the next key.
//! 2. Window-management mode routes keys to window/workspace actions.
//! 3. Terminal mode passes keys through to the focused PTY.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{Mode, Os, Prefix, SwitcherKind};
use crate::layout::{PreselectionDir, SplitType};

/// The result of handling a key: whether the event was consumed (not passed
/// through), and whether the app should quit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyResult {
    /// The key was consumed by the window manager.
    Consumed,
    /// The key should be passed through to the focused PTY.
    Passthrough,
    /// The key was not recognized in window mode.
    Ignored,
    /// The app should quit.
    Quit,
}

/// Format a key event as a human-readable chord string for the showkeys overlay.
fn format_key_chord(key: &KeyEvent) -> String {
    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl".to_string());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt".to_string());
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift".to_string());
    }
    let key_str = match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Backspace => "Bksp".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::PageUp => "PgUp".to_string(),
        KeyCode::PageDown => "PgDn".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        _ => String::new(),
    };
    if !key_str.is_empty() {
        parts.push(key_str);
    }
    parts.join("+")
}

/// Encode a key event into the byte sequence a terminal would send, for
/// passthrough to the PTY in terminal mode.
pub fn encode_key(key: &KeyEvent) -> Vec<u8> {
    use KeyCode::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        Char(c) => {
            let mut out = Vec::new();
            if alt {
                out.push(0x1b);
            }
            if ctrl {
                // Map control characters to their C0 code.
                let code = match c {
                    'a'..='z' => (c as u8) - b'a' + 1,
                    '[' => 0x1b,
                    '\\' => 0x1c,
                    ']' => 0x1d,
                    '^' => 0x1e,
                    '_' => 0x1f,
                    ' ' => 0x00,
                    _ => c as u8,
                };
                out.push(code);
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            out
        }
        Enter => b"\r".to_vec(),
        Backspace => b"\x7f".to_vec(),
        Tab => b"\t".to_vec(),
        Esc => b"\x1b".to_vec(),
        Left => {
            if alt {
                b"\x1b[1;3D".to_vec()
            } else {
                b"\x1b[D".to_vec()
            }
        }
        Right => {
            if alt {
                b"\x1b[1;3C".to_vec()
            } else {
                b"\x1b[C".to_vec()
            }
        }
        Up => {
            if alt {
                b"\x1b[1;3A".to_vec()
            } else {
                b"\x1b[A".to_vec()
            }
        }
        Down => {
            if alt {
                b"\x1b[1;3B".to_vec()
            } else {
                b"\x1b[B".to_vec()
            }
        }
        Home => b"\x1b[H".to_vec(),
        End => b"\x1b[F".to_vec(),
        PageUp => b"\x1b[5~".to_vec(),
        PageDown => b"\x1b[6~".to_vec(),
        Delete => b"\x1b[3~".to_vec(),
        BackTab => b"\x1b[Z".to_vec(),
        _ => Vec::new(),
    }
}

/// Whether a key is the configured leader key (e.g. Ctrl+B).
fn is_leader_key(key: &KeyEvent, leader: &str) -> bool {
    match leader {
        "ctrl+b" => {
            key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('b'))
        }
        "ctrl+a" => {
            key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('a'))
        }
        _ => {
            // Fall back: compare against the encoded form.
            let encoded = encode_key(key);
            !encoded.is_empty() && encoded == leader.as_bytes()
        }
    }
}

/// Handle a key event, mutating the OS state and returning the result.
pub fn handle_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    // Record the last key chord for the showkeys overlay.
    os.last_key_chord = format_key_chord(key);

    // A quit confirmation dialog has its own keys.
    if os.show_quit_confirmation {
        return handle_quit_confirmation(os, key);
    }

    // During tape playback, Ctrl+P toggles pause/resume (Go's prefix routing).
    if os.script_active()
        && key.code == KeyCode::Char('p')
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        os.script_paused = !os.script_paused;
        os.notify(
            if os.script_paused {
                "tape paused (Ctrl+P to resume)"
            } else {
                "tape resumed"
            },
            "info",
        );
        return KeyResult::Consumed;
    }

    // Modal overlays capture keys before the prefix and mode layers.
    if os.theme_picker_open {
        return handle_theme_picker(os, key);
    }
    if os.help_open {
        return handle_help_modal(os, key);
    }
    if os.scrollback_mode {
        return handle_scrollback_mode(os, key);
    }
    if os.palette_open {
        return handle_palette(os, key);
    }
    if os.switcher_open {
        return handle_switcher(os, key);
    }

    // The project-tape trust review captures keys while pending.
    if os.project_tape_pending.is_some() {
        return handle_project_tape_review(os, key);
    }

    // The tape manager overlay captures keys while open.
    if os.tape_manager_open {
        return handle_tape_manager(os, key);
    }

    // A pending prefix consumes the next key.
    match os.prefix {
        Prefix::Leader => return handle_leader_key(os, key),
        Prefix::Workspace => return handle_workspace_prefix(os, key),
        Prefix::Window => return handle_window_prefix(os, key),
        Prefix::Minimize => return handle_minimize_prefix(os, key),
        Prefix::Tape => return handle_tape_prefix(os, key),
        Prefix::None => {}
    }

    // The leader key works from both modes (Go's prefix routing).
    if is_leader_key(key, os.leader_key()) {
        os.prefix = Prefix::Leader;
        return KeyResult::Consumed;
    }

    // In terminal mode, most keys pass through; a few chords stay local.
    if os.mode == Mode::Terminal {
        return handle_terminal_mode(os, key);
    }

    // Window-management mode.
    handle_window_management(os, key)
}

fn handle_quit_confirmation(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            os.quitting = true;
            KeyResult::Quit
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            os.show_quit_confirmation = false;
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

fn handle_terminal_mode(os: &mut Os, key: &KeyEvent) -> KeyResult {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    // Alt+Esc — leave terminal mode (terminal_exit_mode).
    if alt && key.code == KeyCode::Esc {
        os.leave_terminal_mode();
        return KeyResult::Consumed;
    }
    // Alt+n / Alt+p — next/prev window without leaving terminal mode.
    if alt && !shift && key.code == KeyCode::Char('n') {
        os.focus_next();
        return KeyResult::Consumed;
    }
    if alt && !shift && key.code == KeyCode::Char('p') {
        os.focus_prev();
        return KeyResult::Consumed;
    }
    // Alt+arrows — directional focus.
    if alt {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                return KeyResult::Consumed;
            }
            _ => {}
        }
    }
    // Everything else passes through.
    let data = encode_key(key);
    if !data.is_empty() {
        os.write_to_focused(&data);
        os.record_terminal_key(key);
        KeyResult::Passthrough
    } else {
        KeyResult::Consumed
    }
}

fn handle_window_management(os: &mut Os, key: &KeyEvent) -> KeyResult {
    // Leader key starts the prefix.
    if is_leader_key(key, os.leader_key()) {
        os.prefix = Prefix::Leader;
        return KeyResult::Consumed;
    }

    match key.code {
        // Enter terminal mode (i / enter).
        KeyCode::Char('i') => {
            os.enter_terminal_mode();
            return KeyResult::Consumed;
        }
        KeyCode::Enter => {
            if let Some(_idx) = os.focused_window {
                os.enter_terminal_mode();
                return KeyResult::Consumed;
            }
        }
        // Escape leaves prefix or does nothing meaningful in window mode.
        KeyCode::Esc => {
            os.prefix = Prefix::None;
            return KeyResult::Consumed;
        }
        // Quit (q).
        KeyCode::Char('q') => {
            os.show_quit_confirmation = true;
            return KeyResult::Consumed;
        }
        // Next / previous window.
        KeyCode::Char('n') | KeyCode::Tab => {
            os.focus_next();
            return KeyResult::Consumed;
        }
        KeyCode::Char('p') | KeyCode::BackTab => {
            os.focus_prev();
            return KeyResult::Consumed;
        }
        // Workspace switching via Alt+1..9 (checked before the plain digit
        // arm so the guard is reachable).
        KeyCode::Char(c @ '1'..='9') if key.modifiers.contains(KeyModifiers::ALT) => {
            let ws = (c as u8 - b'0') as i32;
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                os.move_focused_to_workspace(ws);
            } else {
                os.switch_workspace(ws);
            }
            return KeyResult::Consumed;
        }
        // Jump to window 1-9.
        KeyCode::Char(c @ '1'..='9') => {
            let idx = (c as u8 - b'1') as usize;
            if idx < os.windows.len() && os.window_on_current_workspace(idx) {
                os.focus_window(idx);
            }
            return KeyResult::Consumed;
        }
        // Arrow navigation in window mode.
        KeyCode::Up => {
            os.focus_prev();
            return KeyResult::Consumed;
        }
        KeyCode::Down => {
            os.focus_next();
            return KeyResult::Consumed;
        }
        _ => {}
    }
    KeyResult::Ignored
}

fn handle_leader_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Esc => {
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Create window.
        KeyCode::Char('c') => {
            do_spawn_window(os);
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Close window.
        KeyCode::Char('x') => {
            do_close_window(os);
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Next / previous window.
        KeyCode::Char('n') | KeyCode::Tab => {
            os.focus_next();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('p') | KeyCode::BackTab => {
            os.focus_prev();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Jump to window 0-9.
        KeyCode::Char(c @ '0'..='9') => {
            let idx = (c as u8 - b'0') as usize;
            if idx < os.windows.len() && os.window_on_current_workspace(idx) {
                os.focus_window(idx);
            }
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Toggle tiling (space) — no-op here; BSP is always on in this port.
        KeyCode::Char(' ') => {
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Sub-prefixes.
        KeyCode::Char('w') => {
            os.prefix = Prefix::Workspace;
            KeyResult::Consumed
        }
        KeyCode::Char('t') => {
            os.prefix = Prefix::Window;
            KeyResult::Consumed
        }
        KeyCode::Char('m') => {
            os.prefix = Prefix::Minimize;
            KeyResult::Consumed
        }
        KeyCode::Char('T') => {
            os.prefix = Prefix::Tape;
            KeyResult::Consumed
        }
        // Split horizontal / vertical.
        KeyCode::Char('-') => {
            do_split_window(os, SplitType::Horizontal);
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('|') | KeyCode::Char('\\') => {
            do_split_window(os, SplitType::Vertical);
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Rotate split.
        KeyCode::Char('R') => {
            if let Some(focused) = os.focused_window {
                let ws = os.current_workspace;
                os.workspace_mut(ws).tree.rotate_split(focused as i32);
            }
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Equalize splits.
        KeyCode::Char('=') => {
            let ws = os.current_workspace;
            os.workspace_mut(ws).tree.equalize_ratios();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Fullscreen/zoom.
        KeyCode::Char('z') => {
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Command palette.
        KeyCode::Char('P') => {
            os.open_palette();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Workspace switcher (W) and window switcher (S).
        KeyCode::Char('W') => {
            os.open_switcher(SwitcherKind::Workspace);
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('S') => {
            // In daemon mode `S` opens the session switcher; locally it lists
            // windows.
            if os.remote_session.is_some() {
                os.open_switcher(SwitcherKind::Session);
            } else {
                os.open_switcher(SwitcherKind::Window);
            }
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Scrollback mode (vim-like navigation).
        KeyCode::Char('[') => {
            os.enter_scrollback_mode();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Quit.
        KeyCode::Char('q') => {
            os.show_quit_confirmation = true;
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Help modal.
        KeyCode::Char('?') => {
            os.toggle_help();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Detach / exit mode.
        KeyCode::Char('d') => {
            os.prefix = Prefix::None;
            os.leave_terminal_mode();
            KeyResult::Consumed
        }
        // Preselection keys (h/j/k/l after prefix are selection/focus).
        KeyCode::Char('h') => {
            os.preselection = PreselectionDir::Left;
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('l') => {
            os.preselection = PreselectionDir::Right;
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('k') => {
            os.preselection = PreselectionDir::Up;
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('j') => {
            os.preselection = PreselectionDir::Down;
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        _ => {
            os.prefix = Prefix::None;
            KeyResult::Ignored
        }
    }
}

fn handle_workspace_prefix(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Esc => {
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char(c @ '1'..='9') => {
            let ws = (c as u8 - b'0') as i32;
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                os.move_focused_to_workspace(ws);
            } else {
                os.switch_workspace(ws);
            }
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        _ => {
            os.prefix = Prefix::None;
            KeyResult::Ignored
        }
    }
}

fn handle_window_prefix(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Esc => {
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('n') => {
            do_spawn_window(os);
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('x') => {
            do_close_window(os);
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Tab => {
            os.focus_next();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::BackTab => {
            os.focus_prev();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        _ => {
            os.prefix = Prefix::None;
            KeyResult::Ignored
        }
    }
}

fn handle_minimize_prefix(os: &mut Os, key: &KeyEvent) -> KeyResult {
    // Minimize is a no-op in this port (windows have no minimized state yet);
    // the keys are consumed to avoid leaking to the shell.
    let _ = key;
    os.prefix = Prefix::None;
    KeyResult::Consumed
}

// ---------------------------------------------------------------------------
// Tape prefix + manager
// ---------------------------------------------------------------------------

/// `Ctrl+B T` — the tape prefix: `r` record, `s` stop, `m` manager, Esc cancel.
fn handle_tape_prefix(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Esc => {
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('r') => {
            os.prefix = Prefix::None;
            os.start_recording();
            KeyResult::Consumed
        }
        KeyCode::Char('s') => {
            os.prefix = Prefix::None;
            os.stop_recording();
            KeyResult::Consumed
        }
        KeyCode::Char('m') => {
            os.open_tape_manager();
            KeyResult::Consumed
        }
        KeyCode::Char('t') => {
            os.prefix = Prefix::None;
            os.review_project_tape();
            KeyResult::Consumed
        }
        _ => {
            os.prefix = Prefix::None;
            KeyResult::Ignored
        }
    }
}

/// The project-tape trust review dialog: `y` trusts and plays, `n`/Esc skips.
fn handle_project_tape_review(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            os.resolve_project_tape(true);
            KeyResult::Consumed
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            os.resolve_project_tape(false);
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

/// The tape manager overlay: filter, navigate, Enter to play, Esc to close.
fn handle_tape_manager(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Esc => {
            os.tape_manager_open = false;
            KeyResult::Consumed
        }
        KeyCode::Enter => {
            os.play_selected_tape();
            KeyResult::Consumed
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let items = os.tape_manager_items().len();
            if items > 0 {
                os.tape_manager_selected = (os.tape_manager_selected + items - 1) % items;
            }
            KeyResult::Consumed
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let items = os.tape_manager_items().len();
            if items > 0 {
                os.tape_manager_selected = (os.tape_manager_selected + 1) % items;
            }
            KeyResult::Consumed
        }
        KeyCode::Backspace => {
            os.tape_manager_query.pop();
            os.tape_manager_selected = 0;
            KeyResult::Consumed
        }
        KeyCode::Char(c) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                os.tape_manager_query.push(c);
                os.tape_manager_selected = 0;
            }
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

// ---------------------------------------------------------------------------
// Scrollback mode
// ---------------------------------------------------------------------------

fn handle_scrollback_mode(os: &mut Os, key: &KeyEvent) -> KeyResult {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // If we're typing a search query, handle differently.
    if os.copy_search_typing {
        return handle_search_typing(os, key);
    }

    // If a char search is pending (f/F/t/T was pressed), the next key is the target.
    if let Some((_, forward, till)) = os.copy_char_search {
        os.copy_char_search = None;
        if let KeyCode::Char(c) = key.code {
            os.copy_char_search(c, forward, till);
        }
        return KeyResult::Consumed;
    }

    match key.code {
        // q always leaves scrollback mode.
        KeyCode::Char('q') => {
            os.exit_scrollback_mode();
            KeyResult::Consumed
        }
        // Esc clears visual selection first, then leaves scrollback mode.
        KeyCode::Esc => {
            if os.copy_visual {
                os.toggle_visual(false);
            } else {
                os.exit_scrollback_mode();
            }
            KeyResult::Consumed
        }
        // Visual modes: v = char-wise, V = line-wise.
        KeyCode::Char('v') => {
            os.toggle_visual(false);
            KeyResult::Consumed
        }
        KeyCode::Char('V') => {
            os.toggle_visual(true);
            KeyResult::Consumed
        }
        // Yank selection.
        KeyCode::Char('y') | KeyCode::Char('c') => {
            os.yank_selection();
            KeyResult::Consumed
        }
        // Basic cursor movement (h/j/k/l, arrows).
        KeyCode::Up | KeyCode::Char('k') => {
            os.copy_move_line(-1);
            KeyResult::Consumed
        }
        KeyCode::Down | KeyCode::Char('j') => {
            os.copy_move_line(1);
            KeyResult::Consumed
        }
        KeyCode::Left | KeyCode::Char('h') => {
            os.copy_move_col(-1);
            KeyResult::Consumed
        }
        KeyCode::Right | KeyCode::Char('l') => {
            os.copy_move_col(1);
            KeyResult::Consumed
        }
        // Word motions.
        KeyCode::Char('w') => {
            os.copy_word_forward(false);
            KeyResult::Consumed
        }
        KeyCode::Char('W') => {
            os.copy_word_forward(true);
            KeyResult::Consumed
        }
        KeyCode::Char('b') if !ctrl => {
            os.copy_word_backward(false);
            KeyResult::Consumed
        }
        KeyCode::Char('B') => {
            os.copy_word_backward(true);
            KeyResult::Consumed
        }
        KeyCode::Char('e') => {
            os.copy_word_end(false);
            KeyResult::Consumed
        }
        KeyCode::Char('E') => {
            os.copy_word_end(true);
            KeyResult::Consumed
        }
        // Line motions.
        KeyCode::Char('0') => {
            os.copy_col_zero();
            KeyResult::Consumed
        }
        KeyCode::Char('^') => {
            os.copy_first_non_blank();
            KeyResult::Consumed
        }
        KeyCode::Char('$') => {
            os.copy_last_non_blank();
            KeyResult::Consumed
        }
        // Paging.
        KeyCode::Char('u') if ctrl => {
            os.copy_move_line(-10);
            KeyResult::Consumed
        }
        KeyCode::Char('d') if ctrl => {
            os.copy_move_line(10);
            KeyResult::Consumed
        }
        KeyCode::Char('b') if ctrl => {
            os.copy_move_line(-20);
            KeyResult::Consumed
        }
        KeyCode::Char('f') if ctrl => {
            os.copy_move_line(20);
            KeyResult::Consumed
        }
        KeyCode::PageUp => {
            os.copy_move_line(-10);
            KeyResult::Consumed
        }
        KeyCode::PageDown => {
            os.copy_move_line(10);
            KeyResult::Consumed
        }
        // Jumps.
        KeyCode::Home | KeyCode::Char('g') => {
            os.copy_top();
            KeyResult::Consumed
        }
        KeyCode::End | KeyCode::Char('G') => {
            os.copy_bottom();
            KeyResult::Consumed
        }
        // Viewport positioning.
        KeyCode::Char('H') => {
            os.copy_viewport_top();
            KeyResult::Consumed
        }
        KeyCode::Char('M') => {
            os.copy_viewport_middle();
            KeyResult::Consumed
        }
        KeyCode::Char('L') => {
            os.copy_viewport_bottom();
            KeyResult::Consumed
        }
        // Blank line navigation.
        KeyCode::Char('{') => {
            os.copy_blank_line(false);
            KeyResult::Consumed
        }
        KeyCode::Char('}') => {
            os.copy_blank_line(true);
            KeyResult::Consumed
        }
        // Char search: f/F/t/T + target char.
        KeyCode::Char('f') => {
            os.copy_char_search = Some(('\0', true, false));
            KeyResult::Consumed
        }
        KeyCode::Char('F') => {
            os.copy_char_search = Some(('\0', false, false));
            KeyResult::Consumed
        }
        KeyCode::Char('t') => {
            os.copy_char_search = Some(('\0', true, true));
            KeyResult::Consumed
        }
        KeyCode::Char('T') => {
            os.copy_char_search = Some(('\0', false, true));
            KeyResult::Consumed
        }
        // Repeat char search.
        KeyCode::Char(';') => {
            os.copy_char_search_repeat(false);
            KeyResult::Consumed
        }
        KeyCode::Char(',') => {
            os.copy_char_search_repeat(true);
            KeyResult::Consumed
        }
        // Regex search.
        KeyCode::Char('/') => {
            os.copy_search_typing = true;
            os.copy_search_forward = true;
            os.copy_search_query.clear();
            KeyResult::Consumed
        }
        KeyCode::Char('?') => {
            os.copy_search_typing = true;
            os.copy_search_forward = false;
            os.copy_search_query.clear();
            KeyResult::Consumed
        }
        KeyCode::Char('n') => {
            os.copy_search_next_match(&os.copy_search_query.clone(), os.copy_search_forward, false);
            KeyResult::Consumed
        }
        KeyCode::Char('N') => {
            os.copy_search_next_match(&os.copy_search_query.clone(), os.copy_search_forward, true);
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

/// Handle key input while typing a search query (`/` or `?` was pressed).
fn handle_search_typing(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Enter => {
            os.copy_execute_search();
            KeyResult::Consumed
        }
        KeyCode::Esc => {
            os.copy_search_typing = false;
            os.copy_search_query.clear();
            KeyResult::Consumed
        }
        KeyCode::Backspace => {
            os.copy_search_query.pop();
            KeyResult::Consumed
        }
        KeyCode::Char(c) => {
            os.copy_search_query.push(c);
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

/// Handle key input while the theme picker is open.
fn handle_theme_picker(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            os.close_theme_picker();
            KeyResult::Consumed
        }
        KeyCode::Enter => {
            os.apply_selected_theme();
            KeyResult::Consumed
        }
        KeyCode::Up | KeyCode::Char('k') => {
            os.theme_picker_move(-1);
            KeyResult::Consumed
        }
        KeyCode::Down | KeyCode::Char('j') => {
            os.theme_picker_move(1);
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

/// Handle key input while the help modal is open.
fn handle_help_modal(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
            os.toggle_help();
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

fn handle_palette(os: &mut Os, key: &KeyEvent) -> KeyResult {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            os.close_palette();
            KeyResult::Consumed
        }
        KeyCode::Enter => {
            os.activate_palette();
            KeyResult::Consumed
        }
        KeyCode::Up | KeyCode::BackTab => {
            os.palette_move(-1);
            KeyResult::Consumed
        }
        KeyCode::Down | KeyCode::Tab => {
            os.palette_move(1);
            KeyResult::Consumed
        }
        KeyCode::Char('p') if ctrl => {
            os.palette_move(-1);
            KeyResult::Consumed
        }
        KeyCode::Char('n') if ctrl => {
            os.palette_move(1);
            KeyResult::Consumed
        }
        KeyCode::Backspace => {
            os.palette_query.pop();
            os.palette_selected = 0;
            KeyResult::Consumed
        }
        KeyCode::Char('u') if ctrl => {
            os.palette_query.clear();
            os.palette_selected = 0;
            KeyResult::Consumed
        }
        KeyCode::Char(c) => {
            os.palette_query.push(c);
            os.palette_selected = 0;
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

// ---------------------------------------------------------------------------
// Switcher (workspace / window)
// ---------------------------------------------------------------------------

fn handle_switcher(os: &mut Os, key: &KeyEvent) -> KeyResult {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            os.close_switcher();
            KeyResult::Consumed
        }
        KeyCode::Enter => {
            os.activate_switcher();
            KeyResult::Consumed
        }
        KeyCode::Up | KeyCode::BackTab => {
            os.switcher_move(-1);
            KeyResult::Consumed
        }
        KeyCode::Down | KeyCode::Tab => {
            os.switcher_move(1);
            KeyResult::Consumed
        }
        KeyCode::Char('p') if ctrl => {
            os.switcher_move(-1);
            KeyResult::Consumed
        }
        KeyCode::Char('n') if ctrl => {
            os.switcher_move(1);
            KeyResult::Consumed
        }
        KeyCode::Backspace => {
            os.switcher_query.pop();
            os.switcher_selected = 0;
            KeyResult::Consumed
        }
        KeyCode::Char('u') if ctrl => {
            os.switcher_query.clear();
            os.switcher_selected = 0;
            KeyResult::Consumed
        }
        KeyCode::Char('d') if ctrl => {
            // In the session switcher, Ctrl+D requests a kill.
            if os.switcher_kind == SwitcherKind::Session {
                let items = os.switcher_items();
                if let Some(e) = items.get(os.switcher_selected) {
                    os.pending_kill = e.session.clone();
                }
            }
            KeyResult::Consumed
        }
        KeyCode::Char(c) => {
            os.switcher_query.push(c);
            os.switcher_selected = 0;
            KeyResult::Consumed
        }
        _ => KeyResult::Consumed,
    }
}

// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

/// Handle a mouse event. Returns true if the event was consumed. Wheel events
/// scroll the hovered pane's scrollback (or forward to a mouse-tracking app in
/// terminal mode); a left click focuses the pane under the cursor.
pub fn handle_mouse(os: &mut Os, mouse: &MouseEvent) -> bool {
    let column = mouse.column as i32;
    let row = mouse.row as i32;
    // The dock bar occupies the bottom row; it never scrolls or focuses.
    if row >= os.height - 1 {
        return false;
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            let step = os.config.appearance.scroll_lines.max(1);
            if let Some(idx) = os.window_at(column, row) {
                if forward_wheel_to_app(os, idx, mouse, -step) {
                    return true;
                }
                os.scroll_window_viewport(idx, step);
            }
            true
        }
        MouseEventKind::ScrollDown => {
            let step = os.config.appearance.scroll_lines.max(1);
            if let Some(idx) = os.window_at(column, row) {
                if forward_wheel_to_app(os, idx, mouse, step) {
                    return true;
                }
                os.scroll_window_viewport(idx, -step);
            }
            true
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Check for border-drag resize first.
            if let Some((wid, edge)) = os.border_at(column, row) {
                let pos = if edge.vertical() { column } else { row };
                os.begin_border_drag(wid, edge, pos);
                return true;
            }
            if let Some(idx) = os.window_at(column, row) {
                os.focus_window(idx);
                os.prefix = Prefix::None;
                // Multi-click detection.
                let now = std::time::Instant::now();
                let pos = (mouse.column, mouse.row);
                let count = if let Some((last_time, last_pos, n)) = os.last_click {
                    if last_pos == pos && now.duration_since(last_time).as_millis() < 500 {
                        n + 1
                    } else {
                        1
                    }
                } else {
                    1
                };
                os.last_click = Some((now, pos, count));
                match count {
                    2 => {
                        os.select_word_at(idx, column, row);
                    }
                    3 => {
                        os.select_line_at(idx, column, row);
                    }
                    _ => {
                        os.begin_mouse_selection(idx, column, row);
                    }
                }
            }
            true
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // Handle border-drag resize.
            if os.drag_resize.is_some() {
                let pos = if matches!(
                    os.drag_resize.map(|(_, e, _)| e),
                    Some(crate::layout::ResizeEdge::Right | crate::layout::ResizeEdge::Left)
                ) {
                    column
                } else {
                    row
                };
                os.apply_border_drag(pos);
                return true;
            }
            if let Some(idx) = os.window_at(column, row) {
                os.extend_mouse_selection(idx, column, row);
            }
            true
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // End border-drag resize.
            if os.drag_resize.is_some() {
                os.end_border_drag();
                return true;
            }
            // Auto-copy on mouse release if there's a selection.
            let has_selection = os.selection.is_some();
            os.end_mouse_selection();
            if has_selection {
                os.yank_selection();
            }
            true
        }
        _ => false,
    }
}

/// In terminal mode, if the hovered pane's application has mouse tracking
/// enabled, encode and forward the wheel event to it. Returns true when the
/// event was forwarded (so the caller does not also scroll scrollback).
fn forward_wheel_to_app(os: &Os, index: usize, mouse: &MouseEvent, direction: i32) -> bool {
    if os.mode != Mode::Terminal {
        return false;
    }
    let Some(window) = os.windows.get(index) else {
        return false;
    };
    let Ok(emu) = window.emulator.lock() else {
        return false;
    };
    if !emu.has_mouse_mode() || emu.is_alt_screen() {
        return false;
    }
    let layout = os.current_layout();
    let Some(rect) = layout.get(&(index as i32)) else {
        return false;
    };
    let x = (mouse.column as i32 - rect.x).max(0) as u16 + 1;
    let y = (mouse.row as i32 - rect.y).max(0) as u16 + 1;
    let button = if direction < 0 { 64u8 } else { 65u8 };
    let seq = format!("\x1b[<{button};{x};{y}M\x1b[<{button};{x};{y}m");
    window.write(seq.as_bytes());
    true
}

/// The shell path: `$SHELL` or `/bin/sh`.
pub fn shell_path(os: &Os) -> String {
    if !os.config.appearance.preferred_shell.is_empty() {
        return os.config.appearance.preferred_shell.clone();
    }
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// Spawn a window: locally, or by asking the daemon (remote mode).
fn do_spawn_window(os: &mut Os) {
    let shell = shell_path(os);
    if os.remote_session.is_some() {
        os.request_new_window(os.current_workspace, &shell);
    } else {
        let wake = Box::new(|| {});
        let _ = os.spawn_window(&shell, wake);
    }
}

/// Close the focused window: locally, or by asking the daemon (remote mode).
fn do_close_window(os: &mut Os) {
    if os.remote_session.is_some() {
        if let Some(idx) = os.focused_window {
            if let Some(w) = os.windows.get(idx) {
                os.request_close_window(&w.id);
            }
        }
    } else {
        os.close_focused_window();
    }
}

/// Split the focused window (creating a new shell): locally, or by asking the
/// daemon and recording the split direction for when the window arrives.
fn do_split_window(os: &mut Os, direction: SplitType) {
    let shell = shell_path(os);
    if os.remote_session.is_some() {
        os.pending_split = Some(direction);
        os.request_new_window(os.current_workspace, &shell);
    } else {
        let wake = Box::new(|| {});
        let _ = os.split(direction, &shell, wake);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::userconfig::UserConfig;

    fn test_os() -> Os {
        Os::new(UserConfig::default_config())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn leader_key_enters_prefix() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Char('i')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.mode, Mode::Terminal);
    }

    #[test]
    fn esc_leaves_terminal_mode() {
        let mut os = test_os();
        os.enter_terminal_mode();
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        // In terminal mode, Esc passes through (alt+esc is the exit chord).
        assert_eq!(result, KeyResult::Passthrough);
    }

    #[test]
    fn alt_esc_leaves_terminal_mode() {
        let mut os = test_os();
        os.enter_terminal_mode();
        let result = handle_key(&mut os, &KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.mode, Mode::WindowManagement);
    }

    #[test]
    fn quit_opens_confirmation() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Char('q')));
        assert_eq!(result, KeyResult::Consumed);
        assert!(os.show_quit_confirmation);
    }

    fn leader() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
    }

    #[test]
    fn prefix_p_opens_palette() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        assert_eq!(os.prefix, Prefix::Leader);
        handle_key(&mut os, &key(KeyCode::Char('P')));
        assert!(os.palette_open);
    }

    #[test]
    fn palette_esc_closes() {
        let mut os = test_os();
        os.open_palette();
        assert!(os.palette_open);
        handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.palette_open);
    }

    #[test]
    fn prefix_bracket_enters_scrollback_mode() {
        let mut os = test_os();
        // A focused window is required to enter scrollback mode.
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('[')));
        assert!(os.scrollback_mode);
        handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.scrollback_mode);
    }

    #[test]
    fn prefix_w_opens_workspace_switcher() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('W')));
        assert!(os.switcher_open);
        assert_eq!(os.switcher_kind, SwitcherKind::Workspace);
    }

    #[test]
    fn mouse_click_focuses_window() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        assert!(handle_mouse(&mut os, &mouse));
        assert_eq!(os.focused_window, Some(0));
    }

    #[test]
    fn mouse_scroll_over_dock_is_ignored() {
        let mut os = test_os();
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: (os.height - 1) as u16, // dock row
            modifiers: KeyModifiers::NONE,
        };
        assert!(!handle_mouse(&mut os, &mouse));
    }
}
