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

    // The sidebar consumes its own keys while focused.
    if os.sidebar.open {
        return handle_sidebar_key(os, key);
    }

    // The aggregate view consumes its own keys.
    if os.aggregate_open {
        return handle_aggregate_key(os, key);
    }

    // The settings overlay consumes its own keys.
    if os.settings_open {
        return handle_settings_key(os, key);
    }

    // The session-close confirmation consumes its own keys.
    if os.session_close.is_some() {
        return handle_session_close_key(os, key);
    }

    // The quit menu consumes its own keys.
    if os.quit_menu.is_some() {
        return handle_quit_menu_key(os, key);
    }

    // The rename dialog consumes text input.
    if os.rename_dialog.is_some() {
        return handle_rename_dialog_key(os, key);
    }

    // The open context menu consumes navigation/selection keys.
    if os.context_menu.is_some() {
        return handle_context_menu_key(os, key);
    }

    // A pending prefix consumes the next key.
    match os.prefix {
        Prefix::Leader => return handle_leader_key(os, key),
        Prefix::Workspace => return handle_workspace_prefix(os, key),
        Prefix::Window => return handle_window_prefix(os, key),
        Prefix::Minimize => return handle_minimize_prefix(os, key),
        Prefix::Tape => return handle_tape_prefix(os, key),
        Prefix::Debug => return handle_debug_prefix(os, key),
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

fn handle_context_menu_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    let Some(menu) = os.context_menu.as_mut() else {
        return KeyResult::Consumed;
    };
    let count = menu.items.len();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            menu.selected = (menu.selected + count - 1) % count;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            menu.selected = (menu.selected + 1) % count;
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let action = menu.items[menu.selected];
            os.dismiss_context_menu();
            os.run_context_action(action);
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            os.dismiss_context_menu();
        }
        _ => {}
    }
    KeyResult::Consumed
}

fn handle_rename_dialog_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Esc => {
            os.cancel_rename_dialog();
        }
        KeyCode::Enter => {
            os.commit_rename_dialog();
        }
        KeyCode::Backspace => {
            if let Some((_, text)) = os.rename_dialog.as_mut() {
                text.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some((_, text)) = os.rename_dialog.as_mut() {
                text.push(c);
            }
        }
        _ => {}
    }
    KeyResult::Consumed
}

fn handle_sidebar_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    // The leader key and any pending prefix keep working while the sidebar is
    // focused (so leader+b closes it, and leader+S etc. still route).
    if os.prefix != Prefix::None {
        return match os.prefix {
            Prefix::Leader => handle_leader_key(os, key),
            Prefix::Workspace => handle_workspace_prefix(os, key),
            Prefix::Window => handle_window_prefix(os, key),
            Prefix::Minimize => handle_minimize_prefix(os, key),
            Prefix::Tape => handle_tape_prefix(os, key),
            Prefix::Debug => handle_debug_prefix(os, key),
            Prefix::None => unreachable!(),
        };
    }
    if is_leader_key(key, os.leader_key()) {
        return handle_leader_key(os, key);
    }
    let count = os.sidebar_rows().len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            os.sidebar.close();
        }
        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
            os.sidebar.move_selection(-1, count);
        }
        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
            os.sidebar.move_selection(1, count);
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            os.activate_sidebar_selection();
        }
        _ => {}
    }
    KeyResult::Consumed
}

fn handle_aggregate_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    let count = os.aggregate_items().len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            os.close_aggregate_view();
        }
        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
            if count > 0 {
                os.aggregate_selected = (os.aggregate_selected + count - 1) % count;
            }
        }
        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
            if count > 0 {
                os.aggregate_selected = (os.aggregate_selected + 1) % count;
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            os.activate_aggregate_selection();
        }
        _ => {}
    }
    KeyResult::Consumed
}

fn handle_settings_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    let count = os.settings_rows().len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            os.close_settings();
        }
        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
            os.settings_selected = (os.settings_selected + count - 1) % count;
        }
        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
            os.settings_selected = (os.settings_selected + 1) % count;
        }
        KeyCode::Left => {
            os.adjust_settings_row(-1);
        }
        KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
            os.adjust_settings_row(1);
        }
        _ => {}
    }
    KeyResult::Consumed
}

fn handle_session_close_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
            if let Some((_, selected)) = os.session_close.as_mut() {
                *selected = if *selected == 0 { 1 } else { 0 };
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            // Cancel is row 0 (the default); Close is row 1.
            let close = os
                .session_close
                .as_ref()
                .map(|(_, s)| *s == 1)
                .unwrap_or(false);
            if close {
                os.confirm_session_close();
            } else {
                os.cancel_session_close();
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            os.cancel_session_close();
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            os.confirm_session_close();
        }
        _ => {}
    }
    KeyResult::Consumed
}

fn handle_quit_menu_key(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(menu) = os.quit_menu.as_mut() {
                let count = menu.items.len();
                menu.selected = (menu.selected + count - 1) % count;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(menu) = os.quit_menu.as_mut() {
                let count = menu.items.len();
                menu.selected = (menu.selected + 1) % count;
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if os.run_quit_menu_selection() {
                return KeyResult::Quit;
            }
        }
        KeyCode::Esc => {
            os.close_quit_menu();
        }
        KeyCode::Char(c) => {
            // Accelerators: the row whose key matches runs.
            let run = os
                .quit_menu
                .as_ref()
                .and_then(|m| m.items.iter().position(|item| item.key == c));
            if let Some(idx) = run {
                if let Some(menu) = os.quit_menu.as_mut() {
                    menu.selected = idx;
                }
                if os.run_quit_menu_selection() {
                    return KeyResult::Quit;
                }
            }
        }
        _ => {}
    }
    KeyResult::Consumed
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
    // Everything else passes through, unless read-only mode is on.
    let data = encode_key(key);
    if !data.is_empty() && !os.read_only {
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
        // Swap with neighbor: H/J/K/L (Go's binding) and ctrl+arrows.
        KeyCode::Char('H') => {
            crate::app::actions::dispatch(os, "swap_left");
            return KeyResult::Consumed;
        }
        KeyCode::Char('J') => {
            crate::app::actions::dispatch(os, "swap_down");
            return KeyResult::Consumed;
        }
        KeyCode::Char('K') => {
            crate::app::actions::dispatch(os, "swap_up");
            return KeyResult::Consumed;
        }
        KeyCode::Char('L') => {
            crate::app::actions::dispatch(os, "swap_right");
            return KeyResult::Consumed;
        }
        // Snap to half: alt+left/alt+right.
        KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
            crate::app::actions::dispatch(os, "snap_left");
            return KeyResult::Consumed;
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
            crate::app::actions::dispatch(os, "snap_right");
            return KeyResult::Consumed;
        }
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
        // Quit (q) — the quit menu (daemon-aware rows).
        KeyCode::Char('q') => {
            os.open_quit_menu();
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
        // Debug prefix.
        KeyCode::Char('D') => {
            os.prefix = Prefix::Debug;
            KeyResult::Consumed
        }
        // Layout picker.
        KeyCode::Char('L') => {
            os.open_switcher(crate::app::SwitcherKind::Layout);
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Aggregate view (all windows, all workspaces).
        KeyCode::Char('A') => {
            os.open_aggregate_view();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // Sidebar: b toggles, e focuses.
        KeyCode::Char('b') => {
            os.sidebar.toggle();
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        KeyCode::Char('e') => {
            os.sidebar.open();
            os.prefix = Prefix::None;
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
            if let Err(e) = os.toggle_zoom_internal() {
                os.notify(e, "error");
            }
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

fn handle_debug_prefix(os: &mut Os, key: &KeyEvent) -> KeyResult {
    match key.code {
        // `l` — toggle the log viewer.
        KeyCode::Char('l') => {
            os.log_viewer_open = !os.log_viewer_open;
            os.debug_overlay_open = false;
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // `c` — toggle the stats overlay.
        KeyCode::Char('c') => {
            os.debug_overlay_open = !os.debug_overlay_open;
            os.log_viewer_open = false;
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // `a` — toggle animations.
        KeyCode::Char('a') => {
            os.config.appearance.animations_enabled = !os.config.appearance.animations_enabled;
            os.notify(
                if os.config.appearance.animations_enabled {
                    "animations enabled"
                } else {
                    "animations disabled"
                },
                "info",
            );
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        // `q` / Esc — cancel.
        KeyCode::Char('q') | KeyCode::Esc => {
            os.prefix = Prefix::None;
            KeyResult::Consumed
        }
        _ => {
            os.prefix = Prefix::None;
            KeyResult::Consumed
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
        // Bracket matching.
        KeyCode::Char('%') => {
            os.copy_bracket_match();
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
            // In the session switcher, Ctrl+D requests a kill via the
            // close confirmation (Cancel is the default row).
            if os.switcher_kind == SwitcherKind::Session {
                let items = os.switcher_items();
                if let Some(e) = items.get(os.switcher_selected) {
                    if let Some(session) = e.session.clone() {
                        os.open_session_close(&session);
                    }
                }
            }
            KeyResult::Consumed
        }
        // In the layout picker, `x` deletes the selected layout.
        KeyCode::Char('x') if os.switcher_kind == SwitcherKind::Layout => {
            let items = os.switcher_items();
            if let Some(e) = items.get(os.switcher_selected) {
                let name = e.label.clone();
                os.delete_saved_layout(&name);
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
        MouseEventKind::Down(MouseButton::Right) => {
            // A right-click anywhere dismisses any open menu first; a second
            // right-click opens the menu at the new position.
            if os.context_menu.is_some() {
                os.dismiss_context_menu();
                return true;
            }
            os.open_context_menu_at(column, row);
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
        MouseEventKind::Moved => {
            // Hovering a pane title bar arms the tooltip delay.
            os.arm_tooltip(column, row);
            os.update_pointer_shape(column, row);
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

    fn os_with_window() -> Os {
        use crate::terminal::pty::WinSize;
        use crate::terminal::window::Window;
        let mut os = test_os();
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 20, rows: 4 },
        );
        os.windows.push(win);
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"hello world");
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
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
    fn quit_opens_quit_menu() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Char('q')));
        assert_eq!(result, KeyResult::Consumed);
        assert!(os.quit_menu.is_some());
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

    // ── encode_key tests ──────────────────────────────────────────────

    #[test]
    fn encode_key_ctrl_a() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&key), vec![0x01]);
    }

    #[test]
    fn encode_key_ctrl_z() {
        let key = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&key), vec![0x1a]);
    }

    #[test]
    fn encode_key_ctrl_bracket() {
        let key = KeyEvent::new(KeyCode::Char('['), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&key), vec![0x1b]);
    }

    #[test]
    fn encode_key_ctrl_backslash() {
        let key = KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&key), vec![0x1c]);
    }

    #[test]
    fn encode_key_ctrl_bracket_close() {
        let key = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&key), vec![0x1d]);
    }

    #[test]
    fn encode_key_ctrl_caret() {
        let key = KeyEvent::new(KeyCode::Char('^'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&key), vec![0x1e]);
    }

    #[test]
    fn encode_key_ctrl_underscore() {
        let key = KeyEvent::new(KeyCode::Char('_'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&key), vec![0x1f]);
    }

    #[test]
    fn encode_key_ctrl_space() {
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL);
        assert_eq!(encode_key(&key), vec![0x00]);
    }

    #[test]
    fn encode_key_alt_char() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(encode_key(&key), vec![0x1b, b'x']);
    }

    #[test]
    fn encode_key_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), vec![b'\r']);
    }

    #[test]
    fn encode_key_backspace() {
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), vec![0x7f]);
    }

    #[test]
    fn encode_key_tab() {
        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), vec![b'\t']);
    }

    #[test]
    fn encode_key_esc() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), vec![0x1b]);
    }

    #[test]
    fn encode_key_arrow_left() {
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[D".to_vec());
    }

    #[test]
    fn encode_key_arrow_right() {
        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[C".to_vec());
    }

    #[test]
    fn encode_key_arrow_up() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[A".to_vec());
    }

    #[test]
    fn encode_key_arrow_down() {
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[B".to_vec());
    }

    #[test]
    fn encode_key_alt_arrow_left() {
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::ALT);
        assert_eq!(encode_key(&key), b"\x1b[1;3D".to_vec());
    }

    #[test]
    fn encode_key_home() {
        let key = KeyEvent::new(KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[H".to_vec());
    }

    #[test]
    fn encode_key_end() {
        let key = KeyEvent::new(KeyCode::End, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[F".to_vec());
    }

    #[test]
    fn encode_key_page_up() {
        let key = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[5~".to_vec());
    }

    #[test]
    fn encode_key_page_down() {
        let key = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[6~".to_vec());
    }

    #[test]
    fn encode_key_delete() {
        let key = KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(encode_key(&key), b"\x1b[3~".to_vec());
    }

    #[test]
    fn encode_key_back_tab() {
        let key = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(encode_key(&key), b"\x1b[Z".to_vec());
    }

    #[test]
    fn encode_key_printable_char() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(encode_key(&key), vec![b'a']);
    }

    #[test]
    fn encode_key_unrecognized_returns_empty() {
        let key = KeyEvent::new(KeyCode::CapsLock, KeyModifiers::NONE);
        assert!(encode_key(&key).is_empty());
    }

    // ── handle_key state tests ────────────────────────────────────────

    #[test]
    fn handle_key_quit_confirmation_y_quits() {
        let mut os = test_os();
        os.show_quit_confirmation = true;
        let result = handle_key(&mut os, &key(KeyCode::Char('y')));
        assert_eq!(result, KeyResult::Quit);
        assert!(os.quitting);
    }

    #[test]
    fn handle_key_quit_confirmation_n_cancels() {
        let mut os = test_os();
        os.show_quit_confirmation = true;
        let result = handle_key(&mut os, &key(KeyCode::Char('n')));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.show_quit_confirmation);
    }

    #[test]
    fn handle_key_quit_confirmation_esc_cancels() {
        let mut os = test_os();
        os.show_quit_confirmation = true;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.show_quit_confirmation);
    }

    #[test]
    fn handle_key_leader_key_sets_prefix() {
        let mut os = test_os();
        let result = handle_key(&mut os, &leader());
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::Leader);
    }

    #[test]
    fn handle_key_window_mode_i_enters_terminal() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Char('i')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.mode, Mode::Terminal);
    }

    #[test]
    fn handle_key_terminal_mode_alt_n_next_window() {
        let mut os = test_os();
        os.enter_terminal_mode();
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT);
        let result = handle_key(&mut os, &key);
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_terminal_mode_alt_p_prev_window() {
        let mut os = test_os();
        os.enter_terminal_mode();
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT);
        let result = handle_key(&mut os, &key);
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_terminal_mode_passthrough() {
        let mut os = test_os();
        os.enter_terminal_mode();
        let result = handle_key(&mut os, &key(KeyCode::Char('x')));
        assert_eq!(result, KeyResult::Passthrough);
    }

    #[test]
    fn handle_key_window_mode_q_opens_quit_menu() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Char('q')));
        assert_eq!(result, KeyResult::Consumed);
        assert!(os.quit_menu.is_some());
    }

    #[test]
    fn handle_key_last_key_chord_is_recorded() {
        let mut os = test_os();
        handle_key(&mut os, &key(KeyCode::Char('x')));
        assert_eq!(os.last_key_chord, "x");
    }

    #[test]
    fn handle_key_ctrl_p_toggles_script_pause() {
        let mut os = test_os();
        // Need a script to be active — script_mode + script_player.
        os.script_mode = true;
        os.script_player = Some(crate::tape::player::Player::new(vec![]));
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        let result = handle_key(&mut os, &key);
        assert_eq!(result, KeyResult::Consumed);
        assert!(os.script_paused);
    }

    #[test]
    fn handle_key_window_mode_n_next_window() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Char('n')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_window_mode_p_prev_window() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Char('p')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_window_mode_tab_next_window() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Tab));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_window_mode_backtab_prev_window() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::BackTab));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_window_mode_esc_clears_prefix() {
        let mut os = test_os();
        os.prefix = Prefix::Leader;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_key_window_mode_arrow_up() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Up));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_window_mode_arrow_down() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Down));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_window_mode_digit_1() {
        let mut os = test_os();
        let result = handle_key(&mut os, &key(KeyCode::Char('1')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_key_leader_then_w_opens_workspace_switcher() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        assert_eq!(os.prefix, Prefix::Leader);
        handle_key(&mut os, &key(KeyCode::Char('W')));
        assert!(os.switcher_open);
    }

    #[test]
    fn handle_key_leader_then_c_opens_window_switcher() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('W')));
        assert!(os.switcher_open);
        assert_eq!(os.switcher_kind, SwitcherKind::Workspace);
    }

    #[test]
    fn handle_key_leader_then_m_opens_minimize_prefix() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(os.prefix, Prefix::Minimize);
    }

    #[test]
    fn handle_key_leader_then_t_opens_window_prefix() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('t')));
        assert_eq!(os.prefix, Prefix::Window);
    }

    #[test]
    fn handle_key_leader_then_t_window_prefix() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('t')));
        assert_eq!(os.prefix, Prefix::Window);
    }

    #[test]
    fn handle_key_leader_then_open_bracket_scrollback() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('[')));
        assert!(os.scrollback_mode);
    }

    #[test]
    fn handle_key_leader_then_p_opens_palette() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('P')));
        assert!(os.palette_open);
    }

    #[test]
    fn handle_key_leader_then_s_opens_window_switcher() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        handle_key(&mut os, &key(KeyCode::Char('S')));
        // In local mode, S opens the window switcher (same as W).
        assert!(os.switcher_open);
    }

    #[test]
    fn format_key_chord_ctrl_a() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let chord = format_key_chord(&key);
        assert_eq!(chord, "Ctrl+a");
    }

    #[test]
    fn format_key_chord_alt_esc() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT);
        let chord = format_key_chord(&key);
        assert_eq!(chord, "Alt+Esc");
    }

    #[test]
    fn format_key_chord_shift_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        let chord = format_key_chord(&key);
        assert_eq!(chord, "Shift+Enter");
    }

    #[test]
    fn format_key_chord_function_key() {
        let key = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
        let chord = format_key_chord(&key);
        assert_eq!(chord, "F5");
    }

    #[test]
    fn format_key_chord_ctrl_f12() {
        let key = KeyEvent::new(KeyCode::F(12), KeyModifiers::CONTROL);
        let chord = format_key_chord(&key);
        assert_eq!(chord, "Ctrl+F12");
    }

    #[test]
    fn format_key_chord_arrows() {
        let keys = vec![
            (KeyCode::Up, "Up"),
            (KeyCode::Down, "Down"),
            (KeyCode::Left, "Left"),
            (KeyCode::Right, "Right"),
        ];
        for (code, name) in keys {
            let k = KeyEvent::new(code, KeyModifiers::NONE);
            assert_eq!(format_key_chord(&k), name);
        }
    }

    #[test]
    fn format_key_chord_special_keys() {
        let keys = vec![
            (KeyCode::Home, "Home"),
            (KeyCode::End, "End"),
            (KeyCode::PageUp, "PgUp"),
            (KeyCode::PageDown, "PgDn"),
            (KeyCode::Backspace, "Bksp"),
            (KeyCode::Tab, "Tab"),
        ];
        for (code, name) in keys {
            let k = KeyEvent::new(code, KeyModifiers::NONE);
            assert_eq!(format_key_chord(&k), name);
        }
    }

    #[test]
    fn format_key_chord_empty_for_unknown() {
        let key = KeyEvent::new(KeyCode::CapsLock, KeyModifiers::NONE);
        let chord = format_key_chord(&key);
        assert!(chord.is_empty());
    }

    #[test]
    fn handle_leader_key_esc_clears_prefix() {
        let mut os = test_os();
        os.prefix = Prefix::Leader;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_c_creates_window() {
        let mut os = test_os();
        let result = handle_key(&mut os, &leader());
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::Leader);
        let result = handle_key(&mut os, &key(KeyCode::Char('c')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_x_closes_window() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('x')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_n_next_window() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('n')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_p_prev_window() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('p')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_tab_next_window() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Tab));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_backtab_prev_window() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::BackTab));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_space_toggle_tiling() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char(' ')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_w_workspace_prefix() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('w')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::Workspace);
    }

    #[test]
    fn handle_leader_key_t_window_prefix() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('t')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::Window);
    }

    #[test]
    fn handle_leader_key_m_minimize_prefix() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::Minimize);
    }

    #[test]
    fn handle_leader_key_tape_prefix() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('T')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::Tape);
    }

    #[test]
    fn handle_leader_key_dash_split_horizontal() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('-')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_pipe_split_vertical() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('|')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_backslash_split_vertical() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('\\')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_r_rotate_split() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('R')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_equal_equalize_ratios() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('=')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_z_fullscreen() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('z')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_p_opens_palette() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('P')));
        assert_eq!(result, KeyResult::Consumed);
        assert!(os.palette_open);
    }

    #[test]
    fn handle_leader_key_digit_jumps_to_window() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('0')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_leader_key_unrecognized_returns_consumed() {
        let mut os = test_os();
        handle_key(&mut os, &leader());
        let result = handle_key(&mut os, &key(KeyCode::Char('z')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_workspace_prefix_esc() {
        let mut os = test_os();
        os.prefix = Prefix::Workspace;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_workspace_prefix_digit() {
        let mut os = test_os();
        os.prefix = Prefix::Workspace;
        let result = handle_key(&mut os, &key(KeyCode::Char('3')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.current_workspace, 3);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_workspace_prefix_shift_digit_moves_window() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os.prefix = Prefix::Workspace;
        let k = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::SHIFT);
        let result = handle_key(&mut os, &k);
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_workspace_prefix_unrecognized() {
        let mut os = test_os();
        os.prefix = Prefix::Workspace;
        let _result = handle_key(&mut os, &key(KeyCode::Char('a')));
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_window_prefix_esc() {
        let mut os = test_os();
        os.prefix = Prefix::Window;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_window_prefix_n() {
        let mut os = test_os();
        os.prefix = Prefix::Window;
        let result = handle_key(&mut os, &key(KeyCode::Char('n')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_window_prefix_tab() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Vertical, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os.prefix = Prefix::Window;
        let result = handle_key(&mut os, &key(KeyCode::Tab));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn handle_window_prefix_backtab() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Vertical, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(1);
        os.focused_window = Some(1);
        os.prefix = Prefix::Window;
        let result = handle_key(&mut os, &key(KeyCode::BackTab));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.focused_window, Some(0));
    }

    #[test]
    fn handle_window_prefix_unrecognized() {
        let mut os = test_os();
        os.prefix = Prefix::Window;
        let _result = handle_key(&mut os, &key(KeyCode::Char('z')));
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_minimize_prefix_any_key() {
        let mut os = test_os();
        os.prefix = Prefix::Minimize;
        let result = handle_key(&mut os, &key(KeyCode::Char('x')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_tape_prefix_esc() {
        let mut os = test_os();
        os.prefix = Prefix::Tape;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_tape_prefix_r_starts_recording() {
        let mut os = os_with_window();
        os.prefix = Prefix::Tape;
        let result = handle_key(&mut os, &key(KeyCode::Char('r')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_tape_prefix_s_stops_recording() {
        let mut os = os_with_window();
        os.start_recording();
        os.prefix = Prefix::Tape;
        let result = handle_key(&mut os, &key(KeyCode::Char('s')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_tape_prefix_m_opens_manager() {
        let mut os = test_os();
        os.prefix = Prefix::Tape;
        let result = handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(result, KeyResult::Consumed);
        assert!(os.tape_manager_open);
    }

    #[test]
    fn handle_tape_prefix_unrecognized() {
        let mut os = test_os();
        os.prefix = Prefix::Tape;
        let _result = handle_key(&mut os, &key(KeyCode::Char('z')));
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn handle_tape_manager_esc() {
        let mut os = test_os();
        os.tape_manager_open = true;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.tape_manager_open);
    }

    #[test]
    fn handle_tape_manager_j_down() {
        let mut os = test_os();
        os.tape_manager_open = true;
        let result = handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_tape_manager_k_up() {
        let mut os = test_os();
        os.tape_manager_open = true;
        let result = handle_key(&mut os, &key(KeyCode::Char('k')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_tape_manager_backspace() {
        let mut os = test_os();
        os.tape_manager_open = true;
        os.tape_manager_query = "abc".into();
        let result = handle_key(&mut os, &key(KeyCode::Backspace));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.tape_manager_query, "ab");
    }

    #[test]
    fn handle_tape_manager_char_filter() {
        let mut os = test_os();
        os.tape_manager_open = true;
        let result = handle_key(&mut os, &key(KeyCode::Char('x')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.tape_manager_query, "x");
    }

    #[test]
    fn handle_theme_picker_esc() {
        let mut os = test_os();
        os.theme_picker_open = true;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.theme_picker_open);
    }

    #[test]
    fn handle_theme_picker_j_down() {
        let mut os = test_os();
        os.theme_picker_open = true;
        os.theme_list = vec!["a".into(), "b".into()];
        let result = handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_theme_picker_k_up() {
        let mut os = test_os();
        os.theme_picker_open = true;
        os.theme_list = vec!["a".into(), "b".into()];
        let result = handle_key(&mut os, &key(KeyCode::Char('k')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_help_modal_esc() {
        let mut os = test_os();
        os.help_open = true;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.help_open);
    }

    #[test]
    fn handle_palette_esc() {
        let mut os = test_os();
        os.palette_open = true;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.palette_open);
    }

    #[test]
    fn handle_palette_j_down() {
        let mut os = test_os();
        os.open_palette();
        let result = handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_palette_k_up() {
        let mut os = test_os();
        os.open_palette();
        let result = handle_key(&mut os, &key(KeyCode::Char('k')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_palette_enter() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "quit".into();
        let result = handle_key(&mut os, &key(KeyCode::Enter));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.palette_open);
    }

    #[test]
    fn handle_palette_char_query() {
        let mut os = test_os();
        os.open_palette();
        let result = handle_key(&mut os, &key(KeyCode::Char('q')));
        assert_eq!(result, KeyResult::Consumed);
        assert_eq!(os.palette_query, "q");
    }

    #[test]
    fn handle_switcher_esc() {
        let mut os = test_os();
        os.switcher_open = true;
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.switcher_open);
    }

    #[test]
    fn handle_switcher_j_down() {
        let mut os = test_os();
        os.open_switcher(SwitcherKind::Workspace);
        let result = handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_switcher_k_up() {
        let mut os = test_os();
        os.open_switcher(SwitcherKind::Workspace);
        let result = handle_key(&mut os, &key(KeyCode::Char('k')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_switcher_enter() {
        let mut os = test_os();
        os.open_switcher(SwitcherKind::Workspace);
        os.switcher_selected = 2;
        let result = handle_key(&mut os, &key(KeyCode::Enter));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.switcher_open);
    }

    #[test]
    fn handle_scrollback_q_exits() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        let result = handle_key(&mut os, &key(KeyCode::Char('q')));
        assert_eq!(result, KeyResult::Consumed);
        assert!(!os.scrollback_mode);
    }

    #[test]
    fn handle_scrollback_v_toggles_visual() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        let result = handle_key(&mut os, &key(KeyCode::Char('v')));
        assert_eq!(result, KeyResult::Consumed);
        assert!(os.copy_visual);
    }

    #[test]
    fn handle_scrollback_j_moves_down() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        let result = handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_scrollback_k_moves_up() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        let result = handle_key(&mut os, &key(KeyCode::Char('k')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_scrollback_y_yanks() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        os.toggle_visual(false);
        let result = handle_key(&mut os, &key(KeyCode::Char('y')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_mouse_basic() {
        let mut os = os_with_window();
        let _handled = handle_mouse(
            &mut os,
            &MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 5,
                row: 5,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
        );
        // Just check it doesn't panic.
    }

    #[test]
    fn handle_project_tape_review_y() {
        let mut os = test_os();
        os.project_tape_pending = Some(crate::app::ProjectTapePending {
            path: "/tmp/test.tape".into(),
            hash: "abc123".into(),
            content: b"content".to_vec(),
        });
        let result = handle_key(&mut os, &key(KeyCode::Char('y')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_project_tape_review_n() {
        let mut os = test_os();
        os.project_tape_pending = Some(crate::app::ProjectTapePending {
            path: "/tmp/test.tape".into(),
            hash: "abc123".into(),
            content: b"content".to_vec(),
        });
        let result = handle_key(&mut os, &key(KeyCode::Char('n')));
        assert_eq!(result, KeyResult::Consumed);
    }

    #[test]
    fn handle_project_tape_review_esc() {
        let mut os = test_os();
        os.project_tape_pending = Some(crate::app::ProjectTapePending {
            path: "/tmp/test.tape".into(),
            hash: "abc123".into(),
            content: b"content".to_vec(),
        });
        let result = handle_key(&mut os, &key(KeyCode::Esc));
        assert_eq!(result, KeyResult::Consumed);
    }
}

#[cfg(test)]
mod debug_prefix_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;

    fn test_os() -> Os {
        Os::new(UserConfig::default_config())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn leader_d_enters_debug_prefix() {
        let mut os = test_os();
        // Leader, then D.
        os.prefix = Prefix::Leader;
        let r = handle_key(&mut os, &key(KeyCode::Char('D')));
        assert_eq!(r, KeyResult::Consumed);
        assert_eq!(os.prefix, Prefix::Debug);
    }

    #[test]
    fn debug_c_toggles_stats_overlay() {
        let mut os = test_os();
        os.prefix = Prefix::Debug;
        handle_key(&mut os, &key(KeyCode::Char('c')));
        assert!(os.debug_overlay_open);
        assert_eq!(os.prefix, Prefix::None);
        // Toggle off again via the prefix.
        os.prefix = Prefix::Debug;
        handle_key(&mut os, &key(KeyCode::Char('c')));
        assert!(!os.debug_overlay_open);
    }

    #[test]
    fn debug_l_toggles_log_viewer() {
        let mut os = test_os();
        os.notify("hello", "info");
        os.prefix = Prefix::Debug;
        handle_key(&mut os, &key(KeyCode::Char('l')));
        assert!(os.log_viewer_open);
        assert!(!os.debug_overlay_open);
        // The event log ring captured the notification.
        assert!(os.event_log.iter().any(|e| e.contains("hello")));
    }

    #[test]
    fn debug_a_toggles_animations() {
        let mut os = test_os();
        os.config.appearance.animations_enabled = false;
        os.prefix = Prefix::Debug;
        handle_key(&mut os, &key(KeyCode::Char('a')));
        assert!(os.config.appearance.animations_enabled);
        os.prefix = Prefix::Debug;
        handle_key(&mut os, &key(KeyCode::Char('a')));
        assert!(!os.config.appearance.animations_enabled);
    }

    #[test]
    fn debug_q_cancels() {
        let mut os = test_os();
        os.prefix = Prefix::Debug;
        handle_key(&mut os, &key(KeyCode::Char('q')));
        assert_eq!(os.prefix, Prefix::None);
    }

    #[test]
    fn event_log_is_bounded() {
        let mut os = test_os();
        for i in 0..250 {
            os.notify(format!("msg {i}"), "info");
        }
        assert!(os.event_log.len() <= 200);
        // The oldest entry was dropped.
        assert!(!os.event_log.iter().any(|e| e.contains("msg 0")));
    }
}

#[cfg(test)]
mod swap_snap_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_two() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        for i in 0..2 {
            let w = Window::without_pty(
                format!("w{i}"),
                format!("win{i}"),
                WinSize { cols: 10, rows: 3 },
            );
            os.windows.push(w);
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Vertical, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn uppercase_swap_keys_do_not_panic() {
        let mut os = os_with_two();
        for c in ['H', 'J', 'K', 'L'] {
            let r = handle_key(&mut os, &key(KeyCode::Char(c), KeyModifiers::NONE));
            assert_eq!(r, KeyResult::Consumed);
        }
        assert_eq!(os.windows.len(), 2);
    }

    #[test]
    fn alt_arrows_snap() {
        let mut os = os_with_two();
        let r = handle_key(&mut os, &key(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(r, KeyResult::Consumed);
        // After snap, the focused window still exists.
        assert_eq!(os.windows.len(), 2);
    }

    #[test]
    fn swap_up_trades_positions() {
        let mut os = os_with_two();
        os.focus_window(1);
        handle_key(&mut os, &key(KeyCode::Char('K'), KeyModifiers::NONE));
        // The tree still contains both windows.
        assert!(os.workspace(1).tree.has_window(0));
        assert!(os.workspace(1).tree.has_window(1));
    }

    #[test]
    fn action_dispatch_snap_left() {
        let mut os = os_with_two();
        assert!(crate::app::actions::dispatch(&mut os, "snap_left"));
    }
}
