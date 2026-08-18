//! Rendering — the compositor that paints panes, borders, and the dock bar.
//! Ported from TUIOS `internal/app/os_render.go` and the lipgloss rendering
//! pipeline.

use ratatui::buffer::Buffer;
use ratatui::layout::Position as TuiPosition;
use ratatui::layout::Rect as TuiRect;
use ratatui::style::{Color as TuiColor, Modifier, Style as TuiStyle};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Widget};

use crate::app::{ContextMenu, Mode, Os, Prefix, Selection};
use crate::layout::Rect;
use crate::ui::{border_type, to_tui_style};

/// Render the whole app into a ratatui buffer.
pub fn render(os: &Os, buf: &mut Buffer) {
    let area = *buf.area();
    // A zero-size terminal (e.g. headless) has nothing to paint.
    if area.width == 0 || area.height == 0 {
        return;
    }
    let dock_height = 1usize;
    let dock_area = TuiRect {
        x: 0,
        y: area.height.saturating_sub(dock_height as u16),
        width: area.width,
        height: dock_height as u16,
    };
    let content_area = TuiRect {
        x: 0,
        y: 0,
        width: area.width,
        height: area.height.saturating_sub(dock_height as u16),
    };

    // Paint the background.
    let bg = os
        .theme
        .as_ref()
        .map(|t| TuiColor::Rgb(t.background.0, t.background.1, t.background.2))
        .unwrap_or(TuiColor::Reset);
    for y in 0..content_area.height {
        for x in 0..content_area.width {
            buf[(content_area.x + x, content_area.y + y)].set_char(' ');
            buf[(content_area.x + x, content_area.y + y)].set_bg(bg);
        }
    }

    // Composite each pane.
    let layout = os.current_layout();
    let bounds = os.workspace_bounds(os.current_workspace);
    let focused = os.focused_window;
    let ws = os.current_workspace;
    let all_ids = os.workspace(ws).tree.get_all_window_ids();

    // Sort window IDs by layout order for stable focus ordering.
    let mut sorted_ids: Vec<i32> = all_ids.clone();
    sorted_ids.sort_unstable();

    for &window_id in &all_ids {
        let Some(window) = os.windows.get(window_id as usize) else {
            continue;
        };
        // A zoomed window fills the workspace.
        let rect = if window.zoomed {
            &bounds
        } else {
            match layout.get(&window_id) {
                Some(r) => r,
                None => continue,
            }
        };

        // An active minimize/restore/snap animation overrides the pane rect.
        let animated = os.animation_position(window_id);
        let tui_rect = if let Some((ax, ay, aw, ah)) = animated {
            let ar = Rect {
                x: ax,
                y: ay,
                w: aw,
                h: ah,
            };
            rect_to_tui(ar, content_area)
        } else {
            rect_to_tui(*rect, content_area)
        };
        let is_focused = focused == Some(window_id as usize);
        let selection = os
            .selection
            .as_ref()
            .filter(|s| s.window == window_id as usize);

        // Paint the pane content, selection highlight, and scrollbar.
        if let Ok(emu) = window.emulator.lock() {
            paint_emulator(buf, &emu, tui_rect, os.theme.as_ref());
            paint_selection(buf, &emu, tui_rect, selection);
            paint_scrollbar(buf, &emu, tui_rect, os, is_focused);
        }

        // Draw the border.
        let border_color = if is_focused {
            focused_border_color(os)
        } else {
            unfocused_border_color(os)
        };
        let title = window.title.clone();
        draw_pane_border(buf, tui_rect, &title, is_focused, border_color, os);
    }

    // Draw the dock bar.
    render_dock(os, buf, dock_area, &sorted_ids);

    // Modal overlays, topmost, in priority order.
    if let Some((_, text)) = &os.rename_dialog {
        let lines = vec![
            "Enter a new window title:".to_string(),
            format!("  {text}_"),
            String::new(),
            "Enter apply · Esc cancel".to_string(),
        ];
        render_overlay(buf, content_area, &lines, "Rename window");
    } else if let Some(menu) = &os.context_menu {
        render_context_menu(buf, content_area, menu);
    } else if let Some(pending) = &os.project_tape_pending {
        let mut lines = Vec::new();
        lines.push("A .tuios.tape was found in this directory.".into());
        lines.push(format!("  {}", pending.path));
        lines.push(format!(
            "  sha256 {}",
            &pending.hash[..pending.hash.len().min(16)]
        ));
        lines.push(String::new());
        lines.push("Trust and run it?  (y/n)".into());
        render_overlay(buf, content_area, &lines, "Project tape");
    } else if os.tape_manager_open {
        render_tape_manager(os, buf, content_area);
    } else if let Some((session, selected)) = &os.session_close {
        let (panes, agents) = os.session_toll(session);
        let mut toll = format!("{panes} pane(s)");
        if agents > 0 {
            toll.push_str(&format!(", {agents} agent-marked window(s)"));
        }
        let lines = vec![
            format!("Close session '{session}'? This will end its panes."),
            format!("  {toll}"),
            String::new(),
            format!("  {} Cancel", if *selected == 0 { "›" } else { " " }),
            format!("  {} Close session", if *selected == 1 { "›" } else { " " }),
        ];
        render_overlay(buf, content_area, &lines, "Close session");
    } else if let Some(menu) = &os.quit_menu {
        let mut lines: Vec<String> = Vec::new();
        for (i, item) in menu.items.iter().enumerate() {
            let marker = if i == menu.selected { "› " } else { "  " };
            let warn = if item.warn { " (loses work)" } else { "" };
            lines.push(format!("{marker}{}  [{}]{}", item.label, item.key, warn));
        }
        render_overlay(buf, content_area, &lines, "Quit");
    } else if os.show_quit_confirmation {
        render_overlay(
            buf,
            content_area,
            &["Quit TermOS?  (y/n)".to_string()],
            "Quit",
        );
    } else if os.debug_overlay_open {
        let lines = debug_stats_lines(os);
        render_overlay(buf, content_area, &lines, "Debug stats");
    } else if os.log_viewer_open {
        let lines: Vec<String> = os.event_log.iter().rev().take(20).cloned().collect();
        render_overlay(buf, content_area, &lines, "Event log");
    } else if os.theme_picker_open {
        let lines: Vec<String> = os
            .theme_list
            .iter()
            .enumerate()
            .map(|(i, name)| {
                if i == os.theme_picker_selected {
                    format!("> {}", name)
                } else {
                    format!("  {}", name)
                }
            })
            .collect();
        render_overlay(
            buf,
            content_area,
            &lines,
            "Theme  (j/k: select, Enter: apply, Esc: cancel)",
        );
    } else if os.help_open {
        render_help_modal(os, buf, content_area);
    } else if os.scrollback_mode {
        render_overlay(buf, content_area, &scrollback_help_lines(), "Scrollback");
    } else if os.palette_open {
        render_palette(os, buf, content_area);
    } else if os.switcher_open {
        render_switcher(os, buf, content_area);
    } else if os.config.appearance.which_key_enabled && os.prefix != Prefix::None {
        let lines = build_which_key_lines(os);
        render_overlay(buf, content_area, &lines, "which-key");
    }

    // Showkeys: always show the last pressed chord at the bottom.
    if !os.last_key_chord.is_empty()
        && !os.help_open
        && !os.palette_open
        && !os.switcher_open
        && !os.scrollback_mode
        && !os.theme_picker_open
    {
        render_showkeys(buf, content_area, &os.last_key_chord);
    }
}

fn scrollback_help_lines() -> Vec<String> {
    vec![
        "h / l / j / k    move cursor".to_string(),
        "w / b / e        word motions".to_string(),
        "0 / ^ / $        line start / first-non-blank / end".to_string(),
        "f / F / t / T    char search (then target char)".to_string(),
        "; / ,            repeat char search".to_string(),
        "/ / ?            regex search (then Enter)".to_string(),
        "n / N            next / prev search match".to_string(),
        "v / V            visual select (char / line)".to_string(),
        "y                yank (copy)".to_string(),
        "H / M / L        top / mid / bottom of viewport".to_string(),
        "{ / }            prev / next blank line".to_string(),
        "Ctrl+U / Ctrl+D  half-page up / down".to_string(),
        "g / G            oldest / live".to_string(),
        "q / Esc          leave".to_string(),
    ]
}

/// Render the help modal with keybindings for the current mode.
fn render_help_modal(os: &Os, buf: &mut Buffer, area: TuiRect) {
    let lines = if os.mode == Mode::Terminal {
        vec![
            "Terminal Mode".to_string(),
            String::new(),
            "Esc           window management mode".to_string(),
            "Alt+N         next window".to_string(),
            "Alt+P         prev window".to_string(),
            "Alt+1-9       switch workspace".to_string(),
            "Ctrl+B        leader (then window commands)".to_string(),
            String::new(),
            "?  or Esc     close this help".to_string(),
        ]
    } else {
        vec![
            "Window Management Mode".to_string(),
            String::new(),
            "i / Enter     terminal mode".to_string(),
            "h / j / k / l focus window".to_string(),
            "n             new window".to_string(),
            "x             close window".to_string(),
            "z             toggle zoom".to_string(),
            "Space         next window".to_string(),
            "[             scrollback / copy mode".to_string(),
            "p             command palette".to_string(),
            "w             workspace switcher".to_string(),
            "t             toggle tiling".to_string(),
            "- / |         split horizontal / vertical".to_string(),
            "H / J / K / L swap window".to_string(),
            "1-9           switch workspace".to_string(),
            "q             quit".to_string(),
            String::new(),
            "?  or Esc     close this help".to_string(),
        ]
    };
    render_overlay(buf, area, &lines, "Help  (? to close)");
}

/// Render the showkeys overlay (last pressed chord) at the bottom-right.
fn render_showkeys(buf: &mut Buffer, area: TuiRect, chord: &str) {
    let y = area.y + area.height.saturating_sub(2);
    let x = area.x + area.width.saturating_sub(chord.len() as u16 + 2);
    let cell = buf.cell_mut(TuiPosition { x, y });
    if let Some(cell) = cell {
        cell.set_char(' ');
    }
    for (i, c) in chord.chars().enumerate() {
        let cell = buf.cell_mut(TuiPosition {
            x: x + 1 + i as u16,
            y,
        });
        if let Some(cell) = cell {
            cell.set_char(c);
        }
    }
}

/// Render the command palette overlay with a query line and fuzzy-filtered
/// commands, the selected one highlighted.
pub fn render_palette(os: &Os, buf: &mut Buffer, area: TuiRect) {
    let items = os.palette_items();
    let rows: Vec<(String, String)> = items.iter().map(|c| (c.label(), String::new())).collect();
    render_list_overlay(
        buf,
        area,
        "Commands",
        &os.palette_query,
        &rows,
        os.palette_selected,
    );
}

/// Render the workspace/window switcher overlay.
pub fn render_switcher(os: &Os, buf: &mut Buffer, area: TuiRect) {
    let title = match os.switcher_kind {
        super::SwitcherKind::Workspace => "Workspaces",
        super::SwitcherKind::Window => "Windows",
        super::SwitcherKind::Session => "Sessions",
        super::SwitcherKind::Layout => "Layouts",
    };
    let rows: Vec<(String, String)> = os
        .switcher_items()
        .iter()
        .map(|e| (e.label.clone(), e.detail.clone()))
        .collect();
    render_list_overlay(
        buf,
        area,
        title,
        &os.switcher_query,
        &rows,
        os.switcher_selected,
    );
}

/// A centered, bordered list overlay: a query line at the top, rows below, and
/// the selected row drawn reverse-video. Rows are windowed so the selection
/// stays visible.
pub fn render_list_overlay(
    buf: &mut Buffer,
    area: TuiRect,
    title: &str,
    query: &str,
    rows: &[(String, String)],
    selected: usize,
) {
    let max_row = rows
        .iter()
        .map(|(l, d)| {
            l.chars().count()
                + if d.is_empty() {
                    0
                } else {
                    2 + d.chars().count()
                }
        })
        .max()
        .unwrap_or(0);
    let query_w = query.chars().count() + 2;
    let content_w = max_row.max(query_w).max(title.chars().count()) + 4;
    let width = content_w.clamp(20, area.width.saturating_sub(2) as usize) as u16;
    let height = (rows.len() + 4).clamp(3, area.height.saturating_sub(2) as usize) as u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = TuiRect {
        x,
        y,
        width,
        height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(TuiStyle::default().fg(TuiColor::Yellow))
        .title(title);
    let mut block_buf = Buffer::empty(rect);
    block.render(rect, &mut block_buf);
    for yy in 0..height {
        for xx in 0..width {
            buf[(rect.x + xx, rect.y + yy)] = block_buf[(rect.x + xx, rect.y + yy)].clone();
        }
    }

    // Query line.
    let prompt = format!("> {query}");
    for (j, ch) in prompt.chars().enumerate() {
        let x = rect.x + 2 + j as u16;
        if x < rect.x + rect.width - 2 {
            buf[(x, rect.y + 1)].set_char(ch);
        }
    }

    // Rows, windowed to keep the selection visible.
    let visible = (height as usize).saturating_sub(3).max(1);
    let start = if rows.len() > visible {
        selected.saturating_sub(visible - 1)
    } else {
        0
    };
    for i in 0..visible {
        let Some((label, detail)) = rows.get(start + i) else {
            break;
        };
        let row_y = rect.y + 2 + i as u16;
        if row_y >= rect.y + rect.height - 1 {
            break;
        }
        let mut text = label.clone();
        if !detail.is_empty() {
            text.push_str("  ");
            text.push_str(detail);
        }
        let is_selected = start + i == selected;
        for (j, ch) in text.chars().enumerate() {
            let x = rect.x + 2 + j as u16;
            if x < rect.x + rect.width - 2 {
                let cell = &mut buf[(x, row_y)];
                cell.set_char(ch);
                if is_selected {
                    let style = cell.style().add_modifier(Modifier::REVERSED);
                    cell.set_style(style);
                }
            }
        }
    }
}

fn rect_to_tui(rect: Rect, content_area: TuiRect) -> TuiRect {
    TuiRect {
        x: content_area.x + rect.x.max(0) as u16,
        y: content_area.y + rect.y.max(0) as u16,
        width: rect.w.max(1) as u16,
        height: rect.h.max(1) as u16,
    }
}

fn paint_emulator(
    buf: &mut Buffer,
    emu: &crate::vt::Emulator,
    rect: TuiRect,
    theme: Option<&crate::config::theme::Theme>,
) {
    // The pane border consumes the outer ring; content lives one cell in.
    let inner_x = rect.x + 1;
    let inner_y = rect.y + 1;
    let inner_w = rect.width.saturating_sub(2);
    let inner_h = rect.height.saturating_sub(2);
    if inner_w == 0 || inner_h == 0 {
        return;
    }

    let lines = emu.render_view_lines();
    for (row_idx, row) in lines.iter().take(inner_h as usize).enumerate() {
        let y = inner_y + row_idx as u16;
        for (col, (content, style)) in row.iter().take(inner_w as usize).enumerate() {
            let x = inner_x + col as u16;
            if x >= inner_x + inner_w {
                break;
            }
            let cell = &mut buf[(x, y)];
            let c = content.chars().next().unwrap_or(' ');
            cell.set_char(c);
            cell.set_style(to_tui_style(*style, theme));
        }
    }

    // Draw the cursor for the focused pane as a reverse-video block. When the
    // view is scrolled back into the scrollback the live-screen cursor is not
    // on screen, so it is skipped.
    if !emu.in_scrollback() {
        let cursor = emu.cursor_position();
        if !emu.screen().cursor.hidden {
            let cx = inner_x + cursor.x.max(0) as u16;
            let cy = inner_y + cursor.y.max(0) as u16;
            if cx < inner_x + inner_w && cy < inner_y + inner_h {
                let cell = &mut buf[(cx, cy)];
                let mut style = cell.style();
                style = style.add_modifier(Modifier::REVERSED);
                cell.set_style(style);
            }
        }
    }
}

/// Highlight a text selection (reverse video) over a pane's content.
pub fn paint_selection(
    buf: &mut Buffer,
    emu: &crate::vt::Emulator,
    rect: TuiRect,
    selection: Option<&Selection>,
) {
    let Some(sel) = selection else {
        return;
    };
    let (l_lo, l_hi) = sel.line_range();
    let (c_lo, c_hi) = sel.col_range();
    // Content is inset by the border ring (one cell in from the rect edge).
    let inner_w = rect.width.saturating_sub(2);
    let inner_h = rect.height.saturating_sub(2);
    for row_idx in 0..inner_h as i32 {
        let content_line = emu.content_index_for_view_row(row_idx);
        if content_line >= l_lo && content_line <= l_hi {
            let y = rect.y + 1 + row_idx as u16;
            for col in c_lo..=c_hi {
                let x = rect.x + 1 + col as u16;
                if x < rect.x + 1 + inner_w && y < rect.y + 1 + inner_h {
                    let cell = &mut buf[(x, y)];
                    let style = cell.style().add_modifier(Modifier::REVERSED);
                    cell.set_style(style);
                }
            }
        }
    }
}

/// Draw a 1-column scrollbar thumb on the right edge of a scrolled-back pane.
pub fn paint_scrollbar(
    buf: &mut Buffer,
    emu: &crate::vt::Emulator,
    rect: TuiRect,
    os: &Os,
    focused: bool,
) {
    if os.config.appearance.hide_scrollbar {
        return;
    }
    if !emu.in_scrollback() || emu.is_alt_screen() {
        return;
    }
    let sb_len = emu.scrollback_len();
    if sb_len == 0 || rect.width < 3 || rect.height < 3 {
        return;
    }
    let content_h = (rect.height - 2) as usize;
    if content_h <= 2 {
        return;
    }

    // Thumb height is the viewport's share of the whole buffer; travel is the
    // rows the thumb can move within.
    let total = sb_len + content_h;
    let thumb_h = (content_h * content_h)
        .div_ceil(total)
        .clamp(1, content_h - 1);
    let travel = content_h - thumb_h;
    let offset = emu.viewport();
    let thumb_top = if travel > 0 {
        (travel - (offset * travel) / sb_len).clamp(0, travel)
    } else {
        0
    };

    let color = if focused {
        focused_border_color(os)
    } else {
        unfocused_border_color(os)
    };
    let x = rect.x + rect.width - 2; // last content column
    let top = rect.y + 1;
    for i in 0..thumb_h {
        let y = top + thumb_top as u16 + i as u16;
        if y < top + content_h as u16 {
            let cell = &mut buf[(x, y)];
            cell.set_char('█');
            cell.set_style(TuiStyle::default().fg(color));
        }
    }
}

fn focused_border_color(os: &Os) -> TuiColor {
    if let Some(c) = &os.config.appearance.border_focused_color {
        if let Some(rgb) = crate::config::theme::Rgb::parse(c) {
            return TuiColor::Rgb(rgb.0, rgb.1, rgb.2);
        }
    }
    if let Some(theme) = &os.theme {
        TuiColor::Rgb(theme.ansi[4].0, theme.ansi[4].1, theme.ansi[4].2)
    } else {
        TuiColor::Blue
    }
}

fn unfocused_border_color(os: &Os) -> TuiColor {
    if let Some(c) = &os.config.appearance.border_unfocused_color {
        if let Some(rgb) = crate::config::theme::Rgb::parse(c) {
            return TuiColor::Rgb(rgb.0, rgb.1, rgb.2);
        }
    }
    if let Some(theme) = &os.theme {
        TuiColor::Rgb(theme.ansi[8].0, theme.ansi[8].1, theme.ansi[8].2)
    } else {
        TuiColor::DarkGray
    }
}

fn draw_pane_border(
    buf: &mut Buffer,
    rect: TuiRect,
    title: &str,
    focused: bool,
    color: TuiColor,
    os: &Os,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type(&os.config.appearance.border_style))
        .border_style(TuiStyle::default().fg(color))
        .title(Span::styled(
            title,
            TuiStyle::default().fg(color).add_modifier(if focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ));
    let inner = rect;
    // Draw the block manually so we can reuse `rect` without moving it. Only
    // non-space cells are copied: Block::render styles the whole area, and
    // copying styled interior spaces would wipe the pane content drawn first.
    let mut block_buf = Buffer::empty(inner);
    block.render(inner, &mut block_buf);
    for y in 0..inner.height {
        for x in 0..inner.width {
            let src = block_buf.cell((x, y)).unwrap();
            if src.symbol() != " " {
                buf[(inner.x + x, inner.y + y)] = src.clone();
            }
        }
    }
}

fn render_dock(os: &Os, buf: &mut Buffer, area: TuiRect, sorted_ids: &[i32]) {
    let bg = os
        .theme
        .as_ref()
        .map(|t| TuiColor::Rgb(t.ansi[0].0, t.ansi[0].1, t.ansi[0].2))
        .unwrap_or(TuiColor::DarkGray);
    let fg = os
        .theme
        .as_ref()
        .map(|t| TuiColor::Rgb(t.foreground.0, t.foreground.1, t.foreground.2))
        .unwrap_or(TuiColor::White);

    // Fill the dock background.
    for x in 0..area.width {
        buf[(area.x + x, area.y)].set_bg(bg);
    }

    let mode_name = if os.palette_open {
        "PALETTE"
    } else if os.switcher_open {
        "SWITCH"
    } else if os.scrollback_mode {
        "SCROLL"
    } else {
        match os.mode {
            Mode::WindowManagement => "WM",
            Mode::Terminal => "TERM",
        }
    };

    let mut text = format!(
        " {} {}:{} ",
        mode_name,
        os.current_workspace,
        sorted_ids.len()
    );
    if os.prefix != Prefix::None {
        text.push_str("⌨ ");
    }
    // Tape playback progress indicator.
    if os.script_active() {
        let pct = os.script_progress().unwrap_or(0);
        let state = if os.script_paused { "⏸" } else { "▶" };
        text.push_str(&format!(" {state} tape {pct:>3}%"));
    }
    // Recording indicator.
    if os.recording_active() {
        text.push_str(" ● rec");
    }
    // Agent-state indicator for the focused pane.
    let agent = os.focused_agent_state();
    if !agent.is_empty() && agent != "none" {
        text.push_str(&format!(" ✦{agent}"));
        let msg = os.focused_agent_message();
        if !msg.is_empty() {
            text.push_str(&format!(" \"{msg}\""));
        }
        text.push(' ');
    }
    if let Some(notif) = os.notifications.last() {
        text.push_str(&format!(" | {}", notif.message));
    }

    // Draw the text, truncating to the dock width.
    for (i, ch) in text.chars().enumerate().take(area.width as usize) {
        let cell = &mut buf[(area.x + i as u16, area.y)];
        cell.set_char(ch);
        cell.set_style(TuiStyle::default().fg(fg).bg(bg));
    }
}

/// Render the tape manager overlay: a filterable list of recorded tapes.
fn render_tape_manager(os: &Os, buf: &mut Buffer, area: TuiRect) {
    let items = os.tape_manager_items();
    let mut lines = Vec::new();
    lines.push(format!("Filter: {}", os.tape_manager_query));
    lines.push(String::new());
    if items.is_empty() {
        lines.push("No recordings yet — Ctrl+B T r to start recording".into());
    }
    for (i, path) in items.iter().enumerate() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let marker = if i == os.tape_manager_selected {
            "▶ "
        } else {
            "  "
        };
        lines.push(format!("{marker}{name}"));
    }
    lines.push(String::new());
    lines.push("Enter: play   j/k: move   type: filter   Esc: close".into());
    render_overlay(buf, area, &lines, "Tape manager");
}

/// Build a help overlay (the which-key popup) as a list of lines.
pub fn build_which_key_lines(os: &Os) -> Vec<String> {
    use crate::config::keybindings;
    let prefix_type = match os.prefix {
        Prefix::Workspace => "workspace",
        Prefix::Window => "window",
        Prefix::Minimize => "minimize",
        _ => "",
    };
    let bindings = keybindings::get_prefix_keybindings(prefix_type, false);
    let mut lines = Vec::new();
    lines.push(format!("{:?} commands:", os.prefix));
    for b in bindings {
        lines.push(format!("  {:10} {}", b.key, b.description));
    }
    lines
}

/// Render a centered overlay (quit confirmation, help) over the content.
/// The rect is clamped to the area so a long help list cannot overflow the
/// screen (and indexing beyond the buffer panics).
/// Build the debug-stats overlay lines (leader D, then `c`).
fn debug_stats_lines(os: &Os) -> Vec<String> {
    let frames = os.tick_stats.frame_count();
    let avg = os.tick_stats.avg_render_time();
    let mut lines = Vec::new();
    lines.push(format!(
        "frames rendered: {frames}   avg render: {:.2}ms",
        avg.as_secs_f64() * 1000.0
    ));
    lines.push(format!(
        "windows: {}   focused: {}   workspace: {}",
        os.windows.len(),
        os.focused_window
            .map(|i| i.to_string())
            .unwrap_or_else(|| "-".into()),
        os.current_workspace,
    ));
    lines.push(format!("mode: {:?}   prefix: {:?}", os.mode, os.prefix));
    lines.push(format!(
        "scrollback: {}   copy-mode: {}",
        if os.scrollback_mode { "active" } else { "off" },
        if os.copy_visual { "visual" } else { "-" },
    ));
    if let Some(seq) = os.tick_stats.since_last_frame() {
        lines.push(format!(
            "since last frame: {:.1}ms",
            seq.as_secs_f64() * 1000.0
        ));
    }
    lines.push(String::new());
    lines.push("l log viewer · c stats · a animations · q close".into());
    lines
}

/// Render the context menu anchored at its cell, clamped to the screen.
fn render_context_menu(buf: &mut Buffer, area: TuiRect, menu: &ContextMenu) {
    let width = menu
        .items
        .iter()
        .map(|i| i.label().len())
        .max()
        .unwrap_or(0) as u16
        + 6;
    let height = menu.items.len() as u16 + 2;
    let x = (menu.x as u16).min(area.width.saturating_sub(width));
    let y = (menu.y as u16).min(area.height.saturating_sub(height));
    // Border.
    for cx in x..x + width {
        buf[(cx, y)].set_char('─');
        buf[(cx, y + height - 1)].set_char('─');
    }
    for cy in y..y + height {
        buf[(x, cy)].set_char('│');
        buf[(x + width - 1, cy)].set_char('│');
    }
    buf[(x, y)].set_char('┌');
    buf[(x + width - 1, y)].set_char('┐');
    buf[(x, y + height - 1)].set_char('└');
    buf[(x + width - 1, y + height - 1)].set_char('┘');
    for (i, action) in menu.items.iter().enumerate() {
        let row_y = y + 1 + i as u16;
        let selected = i == menu.selected;
        let label = action.label();
        for (j, ch) in label.chars().enumerate() {
            let cell = &mut buf[(x + 1 + j as u16, row_y)];
            cell.set_char(ch);
            if selected {
                cell.set_bg(TuiColor::DarkGray);
                cell.set_fg(TuiColor::White);
            }
        }
        if selected {
            // A marker at the left edge.
            buf[(x + 1, row_y)].set_char('›');
        }
    }
}

pub fn render_overlay(buf: &mut Buffer, area: TuiRect, lines: &[String], title: &str) {
    let width =
        (lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16 + 4).min(area.width);
    let height = (lines.len() as u16 + 4).min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let rect = TuiRect {
        x,
        y,
        width,
        height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(TuiStyle::default().fg(TuiColor::Yellow))
        .title(title);
    let mut block_buf = Buffer::empty(rect);
    block.render(rect, &mut block_buf);
    for yy in 0..height {
        for xx in 0..width {
            buf[(rect.x + xx, rect.y + yy)] = block_buf[(rect.x + xx, rect.y + yy)].clone();
        }
    }

    for (i, line) in lines.iter().enumerate() {
        let y = rect.y + 2 + i as u16;
        for (j, ch) in line.chars().enumerate() {
            let x = rect.x + 2 + j as u16;
            if x < rect.x + rect.width - 2 && y < rect.y + rect.height - 1 {
                buf[(x, y)].set_char(ch);
            }
        }
    }
}

/// Render a list of text lines into the buffer (used for overlays).
pub fn render_text_lines(buf: &mut Buffer, area: TuiRect, lines: &[String]) {
    for (i, line) in lines.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }
        for (j, ch) in line.chars().enumerate() {
            let x = area.x + j as u16;
            if x >= area.x + area.width {
                break;
            }
            buf[(x, y)].set_char(ch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::layout::SplitType;

    fn test_os() -> Os {
        Os::new(UserConfig::default_config())
    }

    #[test]
    fn render_zero_size_buffer_does_not_panic() {
        let os = test_os();
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 0, 0));
        render(&os, &mut buf);
    }

    #[test]
    fn render_single_window_fills_content() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1).tree.insert_window(
            0,
            -1,
            crate::layout::SplitType::None,
            0.5,
            bounds,
            0,
        );
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
        // Should not panic and should paint something.
        // Check that the dock row (last row) has content.
        let dock_row = 23u16;
        let cell = &buf[(0, dock_row)];
        assert!(!cell.symbol().is_empty());
    }

    #[test]
    fn render_text_lines_basic() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 40, 10));
        let area = TuiRect::new(0, 0, 40, 10);
        let lines = vec!["hello".into(), "world".into()];
        render_text_lines(&mut buf, area, &lines);
        assert_eq!(buf[(0, 0)].symbol(), "h");
        assert_eq!(buf[(0, 1)].symbol(), "w");
    }

    #[test]
    fn render_text_lines_truncates_at_width() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 3, 10));
        let area = TuiRect::new(0, 0, 3, 10);
        let lines = vec!["abcdef".into()];
        render_text_lines(&mut buf, area, &lines);
        assert_eq!(buf[(0, 0)].symbol(), "a");
        assert_eq!(buf[(1, 0)].symbol(), "b");
        assert_eq!(buf[(2, 0)].symbol(), "c");
    }

    #[test]
    fn render_text_lines_truncates_at_height() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 40, 2));
        let area = TuiRect::new(0, 0, 40, 2);
        let lines = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        render_text_lines(&mut buf, area, &lines);
        assert_eq!(buf[(0, 0)].symbol(), "a");
        assert_eq!(buf[(0, 1)].symbol(), "b");
    }

    #[test]
    fn render_overlay_basic() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        let area = TuiRect::new(0, 0, 80, 24);
        let lines = vec!["line1".into(), "line2".into()];
        render_overlay(&mut buf, area, &lines, "Test Title");
        // Should not panic.
    }

    #[test]
    fn render_overlay_small_area() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 10, 5));
        let area = TuiRect::new(0, 0, 10, 5);
        let lines = vec!["long line that exceeds width".into()];
        render_overlay(&mut buf, area, &lines, "Title");
        // Should not panic even with small area.
    }

    #[test]
    fn build_which_key_lines_for_leader_prefix() {
        let mut os = test_os();
        os.prefix = Prefix::Leader;
        let lines = build_which_key_lines(&os);
        assert!(!lines.is_empty());
        assert!(lines[0].contains("Leader"));
    }

    #[test]
    fn build_which_key_lines_for_workspace_prefix() {
        let mut os = test_os();
        os.prefix = Prefix::Workspace;
        let lines = build_which_key_lines(&os);
        assert!(!lines.is_empty());
    }

    #[test]
    fn focused_border_color_default() {
        let os = test_os();
        let color = focused_border_color(&os);
        // Default theme should have a color.
        match color {
            TuiColor::Rgb(_, _, _) => {}
            TuiColor::Reset => {}
            _ => {}
        }
    }

    #[test]
    fn unfocused_border_color_default() {
        let os = test_os();
        let color = unfocused_border_color(&os);
        match color {
            TuiColor::Rgb(_, _, _) => {}
            TuiColor::Reset => {}
            _ => {}
        }
    }

    #[test]
    fn render_dock_single_window() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        let dock_area = TuiRect::new(0, 23, 80, 1);
        let sorted_ids = vec![0];
        render_dock(&os, &mut buf, dock_area, &sorted_ids);
        assert!(!buf[(0, 23)].symbol().is_empty());
    }

    #[test]
    fn render_dock_multiple_windows() {
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
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        let dock_area = TuiRect::new(0, 23, 80, 1);
        let sorted_ids = vec![0, 1];
        render_dock(&os, &mut buf, dock_area, &sorted_ids);
        assert!(!buf[(0, 23)].symbol().is_empty());
    }

    #[test]
    fn render_quit_confirmation() {
        let mut os = test_os();
        os.show_quit_confirmation = true;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
        // Should not panic.
    }

    #[test]
    fn render_theme_picker() {
        let mut os = test_os();
        os.theme_picker_open = true;
        os.theme_list = vec!["default".into(), "monokai".into()];
        os.theme_picker_selected = 0;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_help_modal() {
        let mut os = test_os();
        os.help_open = true;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_scrollback_mode() {
        let mut os = test_os();
        os.scrollback_mode = true;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_switcher() {
        let mut os = test_os();
        os.switcher_open = true;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_palette() {
        let mut os = test_os();
        os.palette_open = true;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_showkeys() {
        let mut os = test_os();
        os.last_key_chord = "Ctrl+A".into();
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_tape_manager() {
        let mut os = test_os();
        os.tape_manager_open = true;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_project_tape_pending() {
        let mut os = test_os();
        os.project_tape_pending = Some(crate::app::ProjectTapePending {
            path: "/tmp/test.tape".into(),
            hash: "abc123def456".into(),
            content: b"some tape content".to_vec(),
        });
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_which_key_overlay() {
        let mut os = test_os();
        os.config.appearance.which_key_enabled = true;
        os.prefix = Prefix::Leader;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn render_two_windows() {
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
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
    }

    #[test]
    fn build_which_key_lines_for_window_prefix() {
        let mut os = test_os();
        os.prefix = Prefix::Window;
        let lines = build_which_key_lines(&os);
        assert!(!lines.is_empty());
    }

    #[test]
    fn build_which_key_lines_for_minimize_prefix() {
        let mut os = test_os();
        os.prefix = Prefix::Minimize;
        let lines = build_which_key_lines(&os);
        assert!(!lines.is_empty());
    }

    #[test]
    fn build_which_key_lines_for_tape_prefix() {
        let mut os = test_os();
        os.prefix = Prefix::Tape;
        let lines = build_which_key_lines(&os);
        assert!(!lines.is_empty());
    }
}
