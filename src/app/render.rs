//! Rendering — the compositor that paints panes, borders, and the dock bar.
//! Ported from TUIOS `internal/app/os_render.go` and the lipgloss rendering
//! pipeline.

use ratatui::buffer::Buffer;
use ratatui::layout::Position as TuiPosition;
use ratatui::layout::Rect as TuiRect;
use ratatui::style::{Color as TuiColor, Modifier, Style as TuiStyle};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Widget};

use crate::app::pixel_canvas::PixelCanvas;
use crate::app::{ContextMenu, Mode, Os, Prefix, Selection};
use crate::layout::Rect;
use crate::ui::{border_type, StylePalette};

/// Render the whole app into a ratatui buffer.
pub fn render(os: &Os, buf: &mut Buffer) {
    let area = *buf.area();
    // A zero-size terminal (e.g. headless) has nothing to paint.
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Precompute the theme→ratatui color palette once per frame so per-cell
    // style conversion is an array lookup rather than re-resolving colors
    // through an `Option<&Theme>` for every cell.
    let palette = StylePalette::new(os.theme.as_ref());
    // Dock area: 3 rows — hints bar + accent bar + dock content.
    // Position depends on [appearance] dockbar_position: "bottom" (default), "top", or "hidden".
    let dock_pos = os.config.appearance.dockbar_position.as_str();
    let hints_height: u16 = if os.hints_visible && !os.show_welcome { 1 } else { 0 };
    let dock_height = if dock_pos == "hidden" {
        0usize
    } else {
        2usize + hints_height as usize
    };

    let (dock_area, hints_area, content_area) = if dock_pos == "top" {
        let dock_y = 0u16;
        let hints_y = 1u16;
        (
            TuiRect { x: 0, y: dock_y, width: area.width, height: 1 },
            TuiRect { x: 0, y: hints_y, width: area.width, height: hints_height },
            TuiRect {
                x: 0,
                y: dock_height as u16,
                width: area.width,
                height: area.height.saturating_sub(dock_height as u16),
            },
        )
    } else if dock_pos == "hidden" {
        (
            TuiRect::default(),
            TuiRect::default(),
            TuiRect { x: 0, y: 0, width: area.width, height: area.height },
        )
    } else {
        // "bottom" (default)
        (
            TuiRect {
                x: 0,
                y: area.height.saturating_sub(1),
                width: area.width,
                height: 1,
            },
            TuiRect {
                x: 0,
                y: area.height.saturating_sub(1 + hints_height),
                width: area.width,
                height: hints_height,
            },
            TuiRect {
                x: 0,
                y: 0,
                width: area.width,
                height: area.height.saturating_sub(dock_height as u16),
            },
        )
    };

    // Paint the background via the pixel canvas for gradient/shadow support.
    let bg_rgb = os
        .theme
        .as_ref()
        .map(|t| (t.background.0, t.background.1, t.background.2))
        .unwrap_or((0, 0, 0));
    let mut canvas = os.pixel_canvas.lock().unwrap_or_else(|e| e.into_inner());
    if canvas.width() != area.width as usize || canvas.height() != area.height as usize {
        *canvas = PixelCanvas::new(area.width as usize, area.height as usize);
    }

    // Accent bar: 1-row gradient strip above the dock, giving a "glass" effect.
    // Fades from content background to a dimmed version of the accent color.
    // The computed background is cached so an unchanged theme only memcpys.
    let (dock_bg, dim_accent) = os
        .theme
        .as_ref()
        .map(|theme| {
            let dock_bg = (theme.ansi[0].0, theme.ansi[0].1, theme.ansi[0].2);
            let accent = theme.ansi[4];
            // Dim the accent to 30% brightness for a subtle glass strip.
            let dim_accent = (
                (accent.0 as f64 * 0.3) as u8,
                (accent.1 as f64 * 0.3) as u8,
                (accent.2 as f64 * 0.3) as u8,
            );
            (dock_bg, dim_accent)
        })
        .unwrap_or((bg_rgb, bg_rgb));
    canvas.fill_background(bg_rgb, dim_accent, dock_bg, dock_pos);

    // Drop shadows for floating panes.
    if !os.floats_hidden_by_zoom() {
        let ws = os.current_workspace;
        for fi in os.floats_on_workspace(ws) {
            let f = &os.floats[fi];
            let fr = f.rect();
            let shadow_bg = bg_rgb;
            canvas.drop_shadow(
                fr.x as usize,
                fr.y as usize,
                fr.w as usize,
                fr.h as usize,
                2,
                1,
                3.0,
                (0, 0, 0),
                shadow_bg,
            );
        }
    }

    // Rounded corners for overlays.
    // (Applied later when overlays are rendered.)

    // Paint the RGB canvas directly into the ratatui Buffer. The previous
    // mapper-backed BGR round trip duplicated this full-frame work.
    let rgb = canvas.rgb();
    for y in 0..area.height {
        for x in 0..area.width {
            let idx = ((y as usize * area.width as usize) + x as usize) * 3;
            let cell = &mut buf[(x, y)];
            cell.set_char(' ');
            cell.set_bg(TuiColor::Rgb(rgb[idx], rgb[idx + 1], rgb[idx + 2]));
        }
    }

    // Composite each pane.
    let layout = os.current_layout();
    let bounds = os.workspace_bounds(os.current_workspace);
    let ws = os.current_workspace;
    let all_ids = os.workspace(ws).tree.get_all_window_ids();

    // Sort window IDs by layout order for stable focus ordering.
    let mut sorted_ids: Vec<i32> = all_ids.clone();
    sorted_ids.sort_unstable();

    // Content area as a layout::Rect for viewport culling.
    let content_bounds = Rect {
        x: content_area.x as i32,
        y: content_area.y as i32,
        w: content_area.width as i32,
        h: content_area.height as i32,
    };

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

        // Viewport culling: skip panes entirely outside the content area.
        if !crate::ui::perf::is_visible(rect, &content_bounds) {
            continue;
        }

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
        paint_pane(os, buf, &palette, window_id as usize, tui_rect);
    }

    // Render junction-aware border grid for shared borders.
    if !os.config.appearance.use_ascii_only && os.gap == 0 {
        let pane_rects: Vec<TuiRect> = all_ids
            .iter()
            .filter_map(|&wid| {
                let window = os.windows.get(wid as usize)?;
                if window.zoomed {
                    return Some(rect_to_tui(bounds, content_area));
                }
                layout.get(&wid).map(|r| rect_to_tui(*r, content_area))
            })
            .collect();
        if pane_rects.len() > 1 {
            crate::app::border_grid::render_border_grid(
                buf,
                &pane_rects,
                unfocused_border_color(os),
                os.config.appearance.use_ascii_only,
                os.gap,
            );
        }
    }

    // Floating panes composite above the tiled layout and the border grid,
    // sorted back-to-front by z-order (pinned floats on top). Floats are
    // hidden entirely while a tiled window is zoomed.
    if !os.floats_hidden_by_zoom() {
        for fi in os.floats_on_workspace(ws) {
            let f = &os.floats[fi];
            let window_id = f.window;
            let Some(window) = os.windows.get(window_id) else {
                continue;
            };
            let frect = f.rect();
            // A zoomed float fills the workspace.
            let rect = if window.zoomed { &bounds } else { &frect };
            if !crate::ui::perf::is_visible(rect, &content_bounds) {
                continue;
            }
            let tui_rect = rect_to_tui(*rect, content_area);
            paint_pane(os, buf, &palette, window_id, tui_rect);
        }
    }

    // Draw the key-hints bar (above the dock).
    if os.hints_visible && !os.show_welcome && hints_height > 0 {
        render_hints_bar(os, buf, hints_area);
    }

    // Draw the dock bar.
    render_dock(os, buf, dock_area, &sorted_ids);

    // Sidebar rail over the right edge.
    if os.sidebar.open {
        render_sidebar(os, buf, content_area);
    }

    // Modal overlays, topmost, in priority order.
    if let Some((_, text)) = &os.rename_dialog {
        let lines = vec![
            "Enter a new window title:".to_string(),
            format!("  {text}_"),
            String::new(),
            "Enter apply · Esc cancel".to_string(),
        ];
        render_overlay(buf, content_area, &lines, "Rename window");
    } else if let Some((text, suspended)) = &os.command_pane_dialog {
        let status = if *suspended {
            "\u{23f8} ON"
        } else {
            "OFF"
        };
        let lines = vec![
            "Run a command in a new pane; Enter re-runs it when it finishes.".to_string(),
            format!("  {text}_"),
            format!("  start_suspended: {status}"),
            "Enter run · Tab toggle suspended · Esc cancel".to_string(),
        ];
        render_overlay(buf, content_area, &lines, "Command pane");
    } else if let Some(menu) = &os.context_menu {
        render_context_menu(buf, content_area, menu);
    } else if let Some(pending) = &os.project_tape_pending {
        let mut lines = Vec::with_capacity(8);
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
    } else if os.browser_open {
        render_scrollback_browser_overlay(os, buf, content_area);
    } else if os.aggregate_open {
        let items = os.aggregate_items();
        let mut lines: Vec<String> = Vec::new();
        let mut last_ws = -1;
        for (i, (ws, _, title, preview)) in items.iter().enumerate() {
            if *ws != last_ws {
                lines.push(format!("── workspace {ws} ──"));
                last_ws = *ws;
            }
            let marker = if i == os.aggregate_selected {
                "› "
            } else {
                "  "
            };
            let preview_trim: String = preview.chars().take(50).collect();
            if preview_trim.is_empty() {
                lines.push(format!("{marker}{title}"));
            } else {
                lines.push(format!("{marker}{title}  —  {preview_trim}"));
            }
        }
        if lines.is_empty() {
            lines.push("(no windows)".into());
        }
        lines.push(String::new());
        lines.push("↑↓ select · Enter focus · Esc close".into());
        render_overlay(buf, content_area, &lines, "Aggregate view");
    } else if os.settings_open {
        render_settings_overlay(os, buf, content_area);
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
    } else if os.show_welcome {
        render_welcome_overlay(os, buf, content_area);
    } else if os.debug_overlay_open {
        let lines = debug_stats_lines(os);
        render_overlay(buf, content_area, &lines, "Debug stats");
    } else if os.log_viewer_open {
        let lines: Vec<String> = os.event_log.iter().rev().take(20).cloned().collect();
        render_overlay(buf, content_area, &lines, "Event log");
    } else if os.theme_picker_open {
        render_theme_picker_overlay(os, buf, content_area);
    } else if os.accent_picker_open {
        render_accent_picker_overlay(os, buf, content_area);
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

    // Tooltip (hovered pane title bar) above everything.
    if let Some((text, x, y)) = &os.tooltip {
        render_tooltip(buf, content_area, text, *x, *y);
    }

    // Showkeys: show the last pressed chord at the bottom, only when the
    // `[debug] show_key_events` diagnostic is enabled (default off).
    if os.config.debug.show_key_events
        && !os.last_key_chord.is_empty()
        && !os.help_open
        && !os.palette_open
        && !os.switcher_open
        && !os.scrollback_mode
        && !os.theme_picker_open
        && !os.accent_picker_open
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

/// Render the persistent key-hints bar above the dock.
fn render_hints_bar(os: &Os, buf: &mut Buffer, area: TuiRect) {
    use crate::config::theme::ThemeColors;
    let bg = os.theme.dock_bg();
    let fg = os.theme.dock_dimmed();
    let accent = os.theme.dock_accent();

    // Build contextual hints based on current mode.
    let hints = if os.prefix != Prefix::None {
        vec![
            ("c", "new"),
            ("x", "close"),
            (",", "settings"),
            ("?", "all cmds"),
        ]
    } else if os.scrollback_mode {
        vec![
            ("v", "select"),
            ("y", "yank"),
            ("/", "search"),
            ("q", "leave"),
        ]
    } else if os.mode == Mode::WindowManagement {
        vec![
            ("i", "terminal"),
            ("q", "quit"),
            ("H/J/K/L", "swap"),
            ("?", "help"),
        ]
    } else {
        // Terminal mode.
        vec![
            ("Ctrl+B", "prefix"),
            ("?", "help"),
            ("Esc", "WM mode"),
        ]
    };

    // Render the hints bar as a single row.
    let y = area.y;
    let mut x: u16 = 1;

    // Draw the mode label.
    let mode_label = match os.mode {
        Mode::Terminal => " TERM ",
        Mode::WindowManagement => "  WM  ",
    };
    for (i, ch) in mode_label.chars().enumerate() {
        if x + i as u16 >= area.width {
            break;
        }
        let cell = &mut buf[(area.x + x + i as u16, y)];
        cell.set_char(ch);
        cell.set_style(TuiStyle::default().fg(accent).bg(bg));
    }
    x += mode_label.chars().count() as u16 + 1;

    // Draw the hints.
    for (key, desc) in &hints {
        let label = format!("{key}:{desc}  ");
        for (i, ch) in label.chars().enumerate() {
            if x + i as u16 >= area.width {
                break;
            }
            let cell = &mut buf[(area.x + x + i as u16, y)];
            cell.set_char(ch);
            // Key name in accent, description in dimmed.
            let style = if i < key.len() {
                TuiStyle::default().fg(accent).bg(bg)
            } else {
                TuiStyle::default().fg(fg).bg(bg)
            };
            cell.set_style(style);
        }
        x += label.chars().count() as u16;
    }
}

/// Render the welcome overlay shown on first launch.
fn render_welcome_overlay(_os: &Os, buf: &mut Buffer, area: TuiRect) {
    let lines = vec![
        "Welcome to TermOS!".to_string(),
        String::new(),
        "You're in terminal mode — type commands as usual.".to_string(),
        String::new(),
        "Quick reference:".to_string(),
        "  Ctrl+B then C     Create a new window".to_string(),
        "  Ctrl+B then 1-9   Jump to a window".to_string(),
        "  Ctrl+B then ,     Open settings".to_string(),
        "  ?                 Show all keybindings".to_string(),
        "  Esc               Switch to window management".to_string(),
        String::new(),
        "Press any key to dismiss. Run `termos wizard` for guided setup.".to_string(),
    ];
    render_overlay(buf, area, &lines, "Getting started");
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
/// commands, the selected one highlighted, with matched characters colored.
/// Commands with default keybindings show the shortcut as a dimmed hint.
pub fn render_palette(os: &Os, buf: &mut Buffer, area: TuiRect) {
    let items = os.palette_items();
    let rows: Vec<(String, String)> = items
        .iter()
        .map(|(c, _)| {
            let mut detail = String::new();
            if let Some(kb) = c.keybinding() {
                detail.push_str(kb);
                detail.push_str("  ");
            }
            detail.push_str(c.category());
            (c.label(), detail)
        })
        .collect();
    let highlights: Vec<Vec<usize>> = items.iter().map(|(_, p)| p.clone()).collect();
    render_list_overlay(
        buf,
        area,
        "Commands",
        &os.palette_query,
        &rows,
        os.palette_selected,
        &highlights,
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
        &[],
    );
}

/// A centered, bordered list overlay: a query line at the top, rows below, and
/// the selected row drawn reverse-video. Rows are windowed so the selection
/// stays visible.  `highlights` is per-row: character indices in the label
/// that matched the fuzzy query (rendered bold+yellow).
pub fn render_list_overlay(
    buf: &mut Buffer,
    area: TuiRect,
    title: &str,
    query: &str,
    rows: &[(String, String)],
    selected: usize,
    highlights: &[Vec<usize>],
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

    // Empty state: "No matches" when the query produces zero results.
    if rows.is_empty() && !query.is_empty() {
        let msg = "No matches";
        let row_y = rect.y + 2;
        for (j, ch) in msg.chars().enumerate() {
            let x = rect.x + 2 + j as u16;
            if x < rect.x + rect.width - 2 {
                let cell = &mut buf[(x, row_y)];
                cell.set_char(ch);
                cell.set_style(TuiStyle::default().fg(TuiColor::DarkGray));
            }
        }
        return;
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
        static EMPTY_HL: Vec<usize> = Vec::new();
        let row_highlights: &Vec<usize> = highlights.get(start + i).unwrap_or(&EMPTY_HL);
        let is_selected = start + i == selected;
        for (j, ch) in text.chars().enumerate() {
            let x = rect.x + 2 + j as u16;
            if x < rect.x + rect.width - 2 {
                let cell = &mut buf[(x, row_y)];
                cell.set_char(ch);
                if is_selected {
                    let style = cell.style().add_modifier(Modifier::REVERSED);
                    cell.set_style(style);
                } else if row_highlights.contains(&j) {
                    // Highlighted match: bold + yellow foreground.
                    let style = cell
                        .style()
                        .add_modifier(Modifier::BOLD)
                        .fg(TuiColor::Yellow);
                    cell.set_style(style);
                }
            }
        }
    }
}

/// Paint one pane's content, selection highlight, scrollbar, and border at
/// the given screen rect. Shared by the tiled and floating render passes.
fn paint_pane(os: &Os, buf: &mut Buffer, palette: &StylePalette, window_id: usize, tui_rect: TuiRect) {
    let Some(window) = os.windows.get(window_id) else {
        return;
    };
    let is_focused = os.focused_window == Some(window_id);
    let selection = os
        .selection
        .as_ref()
        .filter(|s| s.window == window_id);

    // A 1-cell-high pane inside a stacked group renders as a tab bar
    // showing every pane in the stack (active highlighted).
    let ws = os.current_workspace;
    let stack_count = os.workspace(ws).tree.stack_count(window_id as i32);
    if tui_rect.height <= 1 && stack_count > 1 {
        let stack_ids = os.workspace(ws).tree.stack_windows(window_id as i32);
        let border_color = if is_focused {
            focused_border_color(os)
        } else {
            unfocused_border_color(os)
        };
        render_stack_tab_bar(buf, tui_rect, &stack_ids, window_id, &os.windows, border_color, os);
        return;
    }

    // Paint the pane content, selection highlight, and scrollbar.
    // Skip expensive emulator render if the window has no new output.
    let is_dirty = window.is_dirty();
    if let Ok(emu) = window.emulator.lock() {
        let width = emu.width();
        let height = emu.height();
        let viewport = emu.viewport();
        if let Ok(mut cache) = window.render_cache.lock() {
            let refresh = is_dirty
                || cache
                    .as_ref()
                    .map(|c| c.width != width || c.height != height || c.viewport != viewport)
                    .unwrap_or(true);
            if refresh {
                *cache = Some(crate::terminal::window::RenderCache {
                    width,
                    height,
                    viewport,
                    lines: emu.render_view_lines(),
                });
            }
            if let Some(cached) = cache.as_ref() {
                paint_emulator(buf, &emu, &cached.lines, tui_rect, palette);
            }
        }
        paint_selection(buf, &emu, tui_rect, selection);
        paint_scrollbar(buf, &emu, tui_rect, os, is_focused);
    }
    if is_dirty {
        window.clear_dirty();
    }

    // Draw the border.
    let border_color = if is_focused {
        focused_border_color(os)
    } else {
        unfocused_border_color(os)
    };
    let mut title = window.title.clone();
    // Multi-select indicator: a checkmark prefix.
    if os.selected_panes.contains(&window_id) {
        title = format!("\u{2713} {title}");
    }
    // Floating-pane badges: pin (always-on-top) and modal (blocks other
    // panes) are shown in the title so the state is discoverable.
    if let Some(fi) = os.float_for_window(window_id) {
        let f = &os.floats[fi];
        if f.modal {
            title = format!("\u{26d4} {title}");
        }
        if f.pinned {
            title = format!("\u{1f4cc} {title}");
        }
    }
    // Command-pane state is shown in the title: suspended until Enter, or
    // the exit status once the command finished.
    if window.command.is_some() {
        if window.suspended {
            title = format!("\u{23f8} {title}  [Enter to run]");
        } else if let Some(code) = window.exit_code {
            title = format!("{title}  [exit {code}]");
        }
    }
    draw_pane_border(buf, tui_rect, &title, is_focused, border_color, os);
}

/// Render a 1-cell-high stacked tab bar showing all panes in a stack.
/// The active pane's tab is highlighted; others show their title.
fn render_stack_tab_bar(
    buf: &mut Buffer,
    rect: TuiRect,
    stack_ids: &[i32],
    active_id: usize,
    windows: &[crate::terminal::window::Window],
    border_color: TuiColor,
    os: &Os,
) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    // Background.
    let bg = os
        .theme
        .as_ref()
        .map(|t| TuiColor::Rgb(t.background.0, t.background.1, t.background.2))
        .unwrap_or(TuiColor::Reset);
    let accent = os
        .theme
        .as_ref()
        .map(|t| TuiColor::Rgb(t.foreground.0, t.foreground.1, t.foreground.2))
        .unwrap_or(border_color);
    for x in 0..rect.width {
        buf[(rect.x + x, rect.y)].set_char(' ');
        buf[(rect.x + x, rect.y)].set_bg(bg);
    }
    // Tab labels separated by vertical bars: title1 | title2 | ...
    let mut x: u16 = rect.x;
    let sep = Span::styled(" | ", TuiStyle::default().fg(border_color));
    let sep_w: u16 = 3;
    let end = rect.x + rect.width;
    for (i, &sid) in stack_ids.iter().enumerate() {
        let w = windows.get(sid as usize);
        let title = w.map(|w| w.title.as_str()).unwrap_or("?");
        let is_active = sid as usize == active_id;
        let marker = if is_active { "\u{25b6} " } else { "" };
        let label = format!("{marker}{title}");
        let style = if is_active {
            TuiStyle::default().fg(accent).add_modifier(Modifier::BOLD)
        } else {
            TuiStyle::default().fg(border_color)
        };
        let span = Span::styled(&label, style);
        let span_w = label.len() as u16;
        if x + span_w > end {
            break;
        }
        buf.set_span(x, rect.y, &span, span_w);
        x += span_w;
        if i + 1 < stack_ids.len() && x + sep_w <= end {
            buf.set_span(x, rect.y, &sep, sep_w);
            x += sep_w;
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
    lines: &[Vec<crate::vt::cell::StyledChar>],
    rect: TuiRect,
    palette: &StylePalette,
) {
    // The pane border consumes the outer ring; content lives one cell in.
    let inner_x = rect.x + 1;
    let inner_y = rect.y + 1;
    let inner_w = rect.width.saturating_sub(2);
    let inner_h = rect.height.saturating_sub(2);
    if inner_w == 0 || inner_h == 0 {
        return;
    }

    let buf_h = buf.area().height;
    for (row_idx, row) in lines.iter().take(inner_h as usize).enumerate() {
        let y = inner_y + row_idx as u16;
        if y >= buf_h {
            break;
        }
        let mut col_pos = 0u16;
        for sc in row.iter() {
            let x = inner_x + col_pos;
            if x >= inner_x + inner_w {
                break;
            }
            let cell = &mut buf[(x, y)];
            if sc.has_combining() {
                // Base + zero-width marks render as one grapheme in a single
                // terminal cell (e.g. `e` + U+0301 → `é`).
                let mut symbol = String::with_capacity(1 + sc.combining_len as usize);
                symbol.push(sc.content);
                sc.for_each_combining(|m| symbol.push(m));
                cell.set_symbol(&symbol);
            } else {
                cell.set_char(sc.content);
            }
            cell.set_style(palette.style(sc.style));
            col_pos += u16::from(sc.width);
            if sc.width > 1 {
                // Mark the continuation cell skipped: ratatui's buffer diff
                // never emits skipped cells, so the terminal's wide glyph
                // keeps its right half instead of being overwritten by the
                // next column. `Cell::reset` clears `skip` every frame.
                if x + 1 < inner_x + inner_w {
                    buf[(x + 1, y)].skip = true;
                }
            }
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
    use crate::config::theme::ThemeColors;
    os.theme.border_focused_terminal()
}

fn unfocused_border_color(os: &Os) -> TuiColor {
    if let Some(c) = &os.config.appearance.border_unfocused_color {
        if let Some(rgb) = crate::config::theme::Rgb::parse(c) {
            return TuiColor::Rgb(rgb.0, rgb.1, rgb.2);
        }
    }
    use crate::config::theme::ThemeColors;
    os.theme.border_unfocused()
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
    // The scratch buffer's area is the pane rect (which can be offset from
    // the origin), and ratatui's `Buffer::cell` expects absolute coordinates.
    for y in 0..inner.height {
        for x in 0..inner.width {
            let src = block_buf
                .cell((inner.x.saturating_add(x), inner.y.saturating_add(y)))
                .unwrap();
            if src.symbol() != " " {
                buf[(inner.x.saturating_add(x), inner.y.saturating_add(y))] = src.clone();
            }
        }
    }
}

fn render_dock(os: &Os, buf: &mut Buffer, area: TuiRect, sorted_ids: &[i32]) {
    use crate::config::theme::ThemeColors;
    let bg = os.theme.dock_bg();
    let fg = os.theme.dock_fg();
    let accent = os.theme.dock_accent();
    let muted = os.theme.dock_dimmed();

    // The pixel canvas already painted the dock background (gradient).
    // We only set bg on cells that need a specific background (pills, widgets).
    // Text cells inherit the canvas gradient via their default bg.

    let ascii_only = os.config.appearance.use_ascii_only;
    let dock_width = area.width as usize;
    let y = area.y;

    // Calculate the dock layout.
    let layout = super::dock::calculate_dock_layout(os);

    // --- Left region: mode pill + workspace strip ---

    let mut x: u16;

    // Mode pill label.
    let mode_name = if os.palette_open {
        "PALETTE"
    } else if os.switcher_open {
        "SWITCH"
    } else if os.scrollback_mode {
        "SCROLL"
    } else if os.prefix != Prefix::None {
        // Show the active prefix type in the mode pill.
        match os.prefix {
            Prefix::Leader => "PREFIX",
            Prefix::Workspace => "WS",
            Prefix::Window => "WIN",
            Prefix::Minimize => "MIN",
            Prefix::Tape => "TAPE",
            Prefix::Debug => "DBG",
            Prefix::Float => "FLOAT",
            Prefix::None => unreachable!(),
        }
    } else {
        match os.mode {
            Mode::WindowManagement => "WM",
            Mode::Terminal => "TERM",
        }
    };

    // The dock count includes floating panes (which are not in the BSP
    // tree and therefore not in `sorted_ids`).
    let float_count = os.floats_on_workspace(os.current_workspace).len();
    let layout_tag = match os.layout_mode {
        crate::layout::LayoutMode::BSP => "",
        crate::layout::LayoutMode::MasterStack => " MS",
        crate::layout::LayoutMode::Scrolling => " SCR",
    };
    let mut left_text = format!(
        " {} {}:{}{} ",
        mode_name,
        os.current_workspace,
        sorted_ids.len() + float_count,
        layout_tag,
    );
    // Tape playback progress indicator.
    if os.script_active() {
        let pct = os.script_progress().unwrap_or(0);
        let state = if os.script_paused { "⏸" } else { "▶" };
        left_text.push_str(&format!(" {state} tape {pct:>3}%"));
    }
    // Recording indicator.
    if os.recording_active() {
        left_text.push_str(" ● rec");
    }
    // Agent-state indicator for the focused pane.
    let agent = os.focused_agent_state();
    if !agent.is_empty() && agent != "none" {
        left_text.push_str(&format!(" ✦{agent}"));
        let msg = os.focused_agent_message();
        if !msg.is_empty() {
            left_text.push_str(&format!(" \"{msg}\""));
        }
        left_text.push(' ');
    }

    // Draw the left text, truncating to the dock width.
    for (i, ch) in left_text.chars().enumerate().take(dock_width) {
        let cell = &mut buf[(area.x + i as u16, y)];
        cell.set_char(ch);
        cell.set_style(TuiStyle::default().fg(fg).bg(bg));
    }
    x = left_text.chars().count() as u16;

    // --- Workspace pills strip ---

    let strip = &layout.workspace_strip;
    if strip.width > 0 && x as usize + strip.width <= dock_width {
        let mut sx = x;

        // Leading column separator.
        if sx < area.width {
            let cell = &mut buf[(area.x + sx, y)];
            cell.set_char(' ');
            cell.set_style(TuiStyle::default().fg(fg).bg(bg));
            sx += 1;
        }

        // Overflow arrow (left).
        if strip.scrolls && strip.more_left {
            if sx < area.width {
                let cell = &mut buf[(area.x + sx, y)];
                cell.set_char('‹');
                cell.set_style(TuiStyle::default().fg(muted).bg(bg));
                sx += 1;
            }
            if sx < area.width {
                let cell = &mut buf[(area.x + sx, y)];
                cell.set_char(' ');
                cell.set_style(TuiStyle::default().fg(fg).bg(bg));
                sx += 1;
            }
        }

        // Pills.
        for pill in &strip.pills {
            if sx as usize + pill.width > dock_width {
                break;
            }
            let pill_style = if pill.active {
                TuiStyle::default().fg(bg).bg(accent).add_modifier(Modifier::BOLD)
            } else {
                TuiStyle::default().fg(fg).bg(bg)
            };
            let cap_l = if ascii_only { "" } else { "\u{e0b6}" };
            for ch in cap_l.chars() {
                if sx < area.width {
                    let cell = &mut buf[(area.x + sx, y)];
                    cell.set_char(ch);
                    cell.set_style(pill_style);
                    sx += 1;
                }
            }
            let label = format!(" {} ", pill.label);
            for ch in label.chars() {
                if sx < area.width {
                    let cell = &mut buf[(area.x + sx, y)];
                    cell.set_char(ch);
                    cell.set_style(pill_style);
                    sx += 1;
                }
            }
            let cap_r = if ascii_only { "" } else { "\u{e0b4}" };
            for ch in cap_r.chars() {
                if sx < area.width {
                    let cell = &mut buf[(area.x + sx, y)];
                    cell.set_char(ch);
                    cell.set_style(pill_style);
                    sx += 1;
                }
            }
            if sx < area.width {
                let cell = &mut buf[(area.x + sx, y)];
                cell.set_char(' ');
                cell.set_style(TuiStyle::default().fg(fg).bg(bg));
                sx += 1;
            }
        }

        // Overflow arrow (right).
        if strip.scrolls && strip.more_right {
            if sx < area.width {
                let cell = &mut buf[(area.x + sx, y)];
                cell.set_char(' ');
                cell.set_style(TuiStyle::default().fg(fg).bg(bg));
                sx += 1;
            }
            if sx < area.width {
                let cell = &mut buf[(area.x + sx, y)];
                cell.set_char('›');
                cell.set_style(TuiStyle::default().fg(muted).bg(bg));
                sx += 1;
            }
        }

        // Pinned "+" tab.
        if let Some(ref add) = strip.add {
            if sx as usize + add.width <= dock_width {
                let add_style = TuiStyle::default().fg(accent).bg(bg);
                let cap_l = if ascii_only { "" } else { "\u{e0b6}" };
                for ch in cap_l.chars() {
                    if sx < area.width {
                        let cell = &mut buf[(area.x + sx, y)];
                        cell.set_char(ch);
                        cell.set_style(add_style);
                        sx += 1;
                    }
                }
                let label = format!(" {} ", add.label);
                for ch in label.chars() {
                    if sx < area.width {
                        let cell = &mut buf[(area.x + sx, y)];
                        cell.set_char(ch);
                        cell.set_style(add_style);
                        sx += 1;
                    }
                }
                let cap_r = if ascii_only { "" } else { "\u{e0b4}" };
                for ch in cap_r.chars() {
                    if sx < area.width {
                        let cell = &mut buf[(area.x + sx, y)];
                        cell.set_char(ch);
                        cell.set_style(add_style);
                        sx += 1;
                    }
                }
            }
        }

        x = sx;
    }

    // --- Center region: minimized window items ---

    for (i, item) in layout.visible_items.iter().enumerate() {
        let item_x = layout.item_positions.get(i).copied().unwrap_or(0);
        let mut ix = item_x as u16;
        let item_style = TuiStyle::default().fg(fg).bg(bg);
        let cap_l = crate::config::constants::dock_pill_left(ascii_only);
        let cap_r = crate::config::constants::dock_pill_right(ascii_only);
        for ch in cap_l.chars() {
            if ix < area.width {
                let cell = &mut buf[(area.x + ix, y)];
                cell.set_char(ch);
                cell.set_style(item_style);
                ix += 1;
            }
        }
        for ch in item.label.chars() {
            if ix < area.width {
                let cell = &mut buf[(area.x + ix, y)];
                cell.set_char(ch);
                cell.set_style(item_style);
                ix += 1;
            }
        }
        for ch in cap_r.chars() {
            if ix < area.width {
                let cell = &mut buf[(area.x + ix, y)];
                cell.set_char(ch);
                cell.set_style(item_style);
                ix += 1;
            }
        }
    }

    // Truncation indicator.
    if layout.truncated_count > 0 {
        let trunc_x = x;
        let trunc_text = format!(" +{} ", layout.truncated_count);
        let trunc_style = TuiStyle::default().fg(muted).bg(bg);
        for (i, ch) in trunc_text.chars().enumerate() {
            let tx = trunc_x as usize + i;
            if tx < dock_width {
                let cell = &mut buf[(area.x + tx as u16, y)];
                cell.set_char(ch);
                cell.set_style(trunc_style);
            }
        }
    }

    // --- Status widgets: right-aligned, before session controls ---

    let widget_cache = os.widget_cache.lock().unwrap();
    if !widget_cache.is_empty() {
        let mut wx: u16 = area.width;
        // Render widgets right-to-left.
        let mut widget_texts: Vec<(&str, &str)> = os.config.status_widgets.iter()
            .filter_map(|w| widget_cache.get(&w.name).map(|v| (w.name.as_str(), v.as_str())))
            .collect();
        widget_texts.reverse(); // right-align order
        for (i, (_name, text)) in widget_texts.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let label = format!(" {text} ");
            let label_w = label.chars().count() as u16;
            // Separator before each widget (except the first).
            let sep_w: u16 = if i > 0 { 1 } else { 0 };
            let total = label_w + sep_w;
            if wx < total {
                break;
            }
            wx -= total;
            // Draw separator.
            if sep_w > 0 {
                let cell = &mut buf[(area.x + wx, y)];
                cell.set_char('│');
                cell.set_style(TuiStyle::default().fg(muted).bg(bg));
            }
            // Draw widget text.
            for (j, ch) in label.chars().enumerate() {
                let cx = area.x + wx + sep_w + j as u16;
                if cx < area.x + area.width {
                    let cell = &mut buf[(cx, y)];
                    cell.set_char(ch);
                    cell.set_style(TuiStyle::default().fg(muted).bg(bg));
                }
            }
        }
    }

    // --- Right region: session controls ---
    // Account for status widget width so widgets and buttons don't overlap.
    let mut widget_total_width: u16 = 0;
    for (i, w) in os.config.status_widgets.iter().enumerate() {
        if let Some(text) = widget_cache.get(&w.name) {
            if !text.is_empty() {
                widget_total_width += text.len() as u16 + 2; // text + spaces
                if i > 0 { widget_total_width += 1; } // separator
            }
        }
    }

    let session_fit = super::dock_session_buttons::dock_session_controls_fit(dock_width);
    if session_fit {
        let buttons = super::dock_session_buttons::dock_session_buttons(ascii_only);
        let mut rx = area.width.saturating_sub(widget_total_width);
        for (action, icon) in buttons.iter().rev() {
            let label = format!(" {} ", icon);
            let btn_width = label.chars().count() as u16;
            if rx < btn_width + 1 {
                break;
            }
            rx = rx.saturating_sub(btn_width);
            let style = match action {
                super::dock_session_buttons::DockSessionAction::Close => {
                    TuiStyle::default().fg(muted).bg(bg)
                }
                super::dock_session_buttons::DockSessionAction::Detach => {
                    TuiStyle::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
                }
                super::dock_session_buttons::DockSessionAction::Rename => {
                    TuiStyle::default().fg(fg).bg(bg)
                }
            };
            for (i, ch) in label.chars().enumerate() {
                let cx = area.x + rx + i as u16;
                if cx < area.x + area.width {
                    let cell = &mut buf[(cx, y)];
                    cell.set_char(ch);
                    cell.set_style(style);
                }
            }
            if rx > 0 {
                rx = rx.saturating_sub(1);
            }
        }
    }

    // --- Copy mode help (when in copy mode) ---

    if os.scrollback_mode {
        let state = super::dock::copy_mode_state_from_os(os);
        let tiers = super::dock::copy_mode_help_tiers(state);
        let room = dock_width
            .saturating_sub(layout.left_width)
            .saturating_sub(layout.right_width);
        let chosen = tiers.iter().rev().find(|tier| {
            let w: usize = tier.iter().map(|h| h.width() + 2).sum::<usize>() + 2;
            w <= room
        });
        if let Some(tier) = chosen {
            let mut hx = layout.left_width as u16;
            let help_style = TuiStyle::default().fg(muted).bg(bg);
            let key_style = TuiStyle::default().fg(fg).bg(bg);
            if hx < area.width {
                let cell = &mut buf[(area.x + hx, y)];
                cell.set_char(' ');
                cell.set_style(help_style);
                hx += 1;
            }
            for hint in tier {
                let key_text = format!("{} ", hint.key);
                for ch in key_text.chars() {
                    if hx < area.width {
                        let cell = &mut buf[(area.x + hx, y)];
                        cell.set_char(ch);
                        cell.set_style(key_style);
                        hx += 1;
                    }
                }
                let label_text = format!("{}  ", hint.label);
                for ch in label_text.chars() {
                    if hx < area.width {
                        let cell = &mut buf[(area.x + hx, y)];
                        cell.set_char(ch);
                        cell.set_style(help_style);
                        hx += 1;
                    }
                }
            }
        }
    }

    // --- Notification (right-aligned when no copy mode help) ---

    if let Some(notif) = os.notifications.last() {
        if !os.scrollback_mode {
            let notif_text = format!(" | {} ", notif.message);
            let notif_style = TuiStyle::default().fg(fg).bg(bg);
            let max_x = if session_fit {
                area.width.saturating_sub(
                    super::dock_session_buttons::dock_session_strip_width(ascii_only) as u16,
                )
            } else {
                area.width
            };
            let start = max_x.saturating_sub(notif_text.chars().count() as u16);
            for (i, ch) in notif_text.chars().enumerate() {
                let nx = start + i as u16;
                if nx < area.width {
                    let cell = &mut buf[(area.x + nx, y)];
                    cell.set_char(ch);
                    cell.set_style(notif_style);
                }
            }
        }
    }
}


/// Render the tape manager overlay: a filterable list of recorded tapes.
fn render_tape_manager(os: &Os, buf: &mut Buffer, area: TuiRect) {
    let items = os.tape_manager_items();
    let mut lines = Vec::with_capacity(8);

    match os.tape_manager_mode {
        super::TapeManagerMode::ConfirmDelete => {
            let name = os
                .tape_manager_delete_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            lines.push(format!("Delete '{name}'?"));
            lines.push(String::new());
            lines.push("y: confirm   n/Esc: cancel".into());
            render_overlay(buf, area, &lines, "Tape manager — confirm delete");
            return;
        }
        super::TapeManagerMode::Naming => {
            lines.push("Name for new recording:".into());
            lines.push(String::new());
            lines.push(format!("> {}", os.tape_manager_name_buffer));
            lines.push(String::new());
            lines.push("Enter: start   Esc: cancel".into());
            render_overlay(buf, area, &lines, "Tape manager — name recording");
            return;
        }
        super::TapeManagerMode::Recording => {
            lines.push("Recording… (Ctrl+B T s to stop)".into());
            lines.push(String::new());
            lines.push(format!("Commands: {}", os.recorder.as_ref().map(|r| r.command_count()).unwrap_or(0)));
            render_overlay(buf, area, &lines, "Tape manager — recording");
            return;
        }
        super::TapeManagerMode::Playing => {
            lines.push("Playing tape…".into());
            lines.push(String::new());
            if let Some(p) = &os.script_player {
                lines.push(format!("Step: {}/{}", p.current_index(), p.total_commands()));
            }
            lines.push(String::new());
            lines.push("Esc: stop".into());
            render_overlay(buf, area, &lines, "Tape manager — playing");
            return;
        }
        super::TapeManagerMode::List => {}
    }

    lines.push(format!("Filter: {}", os.tape_manager_query));
    lines.push(String::new());
    if items.is_empty() {
        lines.push("No recordings yet — r to start recording".into());
    }
    let visible = super::Os::TAPE_MANAGER_VISIBLE_ROWS;
    let scroll = os.tape_manager_scroll.min(items.len().saturating_sub(visible));
    let end = (scroll + visible).min(items.len());
    for (i, path) in items.iter().enumerate().skip(scroll).take(end - scroll) {
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
    if items.len() > visible {
        lines.push(format!(
            "  ({}-{} of {})",
            scroll + 1,
            end,
            items.len()
        ));
    }
    lines.push(String::new());
    lines.push("Enter: play   j/k: move   d: delete   r: record   type: filter   Esc: close".into());
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
    let mut lines = Vec::with_capacity(16);
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
    let mut lines = Vec::with_capacity(16);
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

/// Render the sidebar rail on the right edge.
fn render_sidebar(os: &Os, buf: &mut Buffer, area: TuiRect) {
    const WIDTH: u16 = 32;
    let width = WIDTH.min(area.width.saturating_sub(8));
    let x = area.width.saturating_sub(width);
    let bg = os
        .theme
        .as_ref()
        .map(|t| TuiColor::Rgb(t.ansi[0].0, t.ansi[0].1, t.ansi[0].2))
        .unwrap_or(TuiColor::DarkGray);
    let fg = TuiColor::White;
    for cy in 0..area.height {
        for cx in x..area.width {
            let cell = &mut buf[(cx, area.y + cy)];
            cell.set_bg(bg);
            cell.set_fg(fg);
        }
    }
    let rows = os.sidebar_rows();
    let mut y = 0u16;
    for (i, row) in rows.iter().enumerate() {
        if y >= area.height {
            break;
        }
        let buf_y = area.y + y;
        let selected = i == os.sidebar.selected;
        let indent = if row.kind == super::sidebar::RowKind::Window {
            2
        } else {
            0
        };
        let glyph = if row.kind == super::sidebar::RowKind::Window {
            super::sidebar::agent_glyph(&row.agent_state)
        } else {
            "▸"
        };
        let label: String = row
            .label
            .chars()
            .take((width as usize).saturating_sub(indent + 4))
            .collect();
        let mut text = format!("{}{} {}", " ".repeat(indent), glyph, label);
        if selected {
            text.insert(0, '›');
        }
        for (j, ch) in text.chars().enumerate() {
            let cx = x + j as u16;
            if cx >= area.width {
                break;
            }
            let cell = &mut buf[(cx, buf_y)];
            cell.set_char(ch);
            if selected {
                cell.set_bg(TuiColor::Blue);
            }
        }
        // Detail line for sessions.
        if row.kind == super::sidebar::RowKind::Session && !row.detail.is_empty() {
            y += 1;
            if y < area.height {
                for (j, ch) in row.detail.chars().enumerate() {
                    let cx = x + 1 + j as u16;
                    if cx >= area.width {
                        break;
                    }
                    let cell = &mut buf[(cx, area.y + y)];
                    cell.set_char(ch);
                    cell.set_bg(bg);
                }
            }
        }
        y += 1;
    }
}

/// Render a small tooltip box at a position, clamped to the screen.
fn render_tooltip(buf: &mut Buffer, area: TuiRect, text: &str, x: i32, y: i32) {
    let width = (text.chars().count() as u16 + 4).min(area.width.saturating_sub(2));
    let x = (x as u16).clamp(0, area.width.saturating_sub(width + 2));
    let y = (y as u16).min(area.height.saturating_sub(3));
    for cy in y..y + 3 {
        for cx in x..x + width + 2 {
            let cell = &mut buf[(cx, cy)];
            cell.set_bg(TuiColor::DarkGray);
            cell.set_fg(TuiColor::White);
        }
    }
    let mut chars = text.chars();
    for j in 0..width {
        let ch = chars.next().unwrap_or(' ');
        let cell = &mut buf[(x + 1 + j, y + 1)];
        cell.set_char(ch);
    }
}

/// Render the settings overlay using the Panel system.
fn render_settings_overlay(os: &Os, buf: &mut Buffer, area: TuiRect) {
    use crate::config::theme::ThemeColors;
    use crate::ui::overlay::{Hint, Palette, Panel};

    let rows = os.settings_rows();
    let preferred_width = 60i32;
    let screen_w = area.width as i32;
    let inner_w = crate::ui::overlay::fit_width(preferred_width, screen_w);

    // Build the body: each row is "label  value" with a selection marker.
    let mut body_lines = Vec::new();
    for (i, (label, value)) in rows.iter().enumerate() {
        let marker = if i == os.settings_selected { "› " } else { "  " };
        let label_padded = format!("{:<20}", label);
        body_lines.push(format!("{marker}{label_padded} {value}"));
    }
    body_lines.push(String::new());
    body_lines.push("↑↓ select · ←/→ or Enter adjust · Esc close".into());

    let panel = Panel {
        glyph: String::new(),
        title: "Settings".into(),
        width: inner_w,
        tabs: Vec::new(),
        active_tab: 0,
        body: body_lines.join("\n"),
        hints: vec![
            Hint::new("↑↓", "move"),
            Hint::new("←→", "change"),
            Hint::new("esc", "close"),
        ],
    };

    let pal = Palette::default();
    let (lines, geo) = panel.render(&pal);

    // Place the panel centered in the content area.
    let panel_w = geo.width as u16;
    let panel_h = geo.height as u16;
    let px = area.x + area.width.saturating_sub(panel_w) / 2;
    let py = area.y + area.height.saturating_sub(panel_h) / 2;

    // Fill the panel background.
    let bg = os.theme.overlay_bg();
    let fg = os.theme.overlay_fg();
    for y in 0..panel_h {
        for x in 0..panel_w {
            if px + x < area.x + area.width && py + y < area.y + area.height {
                let cell = &mut buf[(px + x, py + y)];
                cell.set_char(' ');
                cell.set_style(TuiStyle::default().fg(fg).bg(bg));
            }
        }
    }

    // Render the panel lines.
    for (i, line) in lines.iter().enumerate() {
        let ly = py + i as u16;
        if ly >= area.y + area.height {
            break;
        }
        for (j, ch) in line.chars().enumerate() {
            let lx = px + j as u16;
            if lx >= area.x + area.width {
                break;
            }
            let cell = &mut buf[(lx, ly)];
            cell.set_char(ch);
            cell.set_style(TuiStyle::default().fg(fg).bg(bg));
        }
    }
}

/// Render the scrollback browser overlay using the Panel system with tabs.
fn render_scrollback_browser_overlay(os: &Os, buf: &mut Buffer, area: TuiRect) {
    use crate::config::theme::ThemeColors;
    use crate::ui::overlay::{Hint, Palette, Panel};

    let preferred_width = 70i32;
    let screen_w = area.width as i32;
    let inner_w = crate::ui::overlay::fit_width(preferred_width, screen_w);

    let rows = os.browser_rows();
    let start = os.browser_scroll;
    let visible_rows = 20;

    let mut body_lines: Vec<String> = Vec::new();
    for row in rows.iter().skip(start).take(visible_rows) {
        body_lines.push(row.clone());
    }

    let active_tab = match os.browser_mode {
        crate::scrollback::BrowseMode::Commands => 0,
        crate::scrollback::BrowseMode::Output => 1,
        crate::scrollback::BrowseMode::Json => 2,
        crate::scrollback::BrowseMode::Paths => 3,
    };

    let panel = Panel {
        glyph: String::new(),
        title: "Scrollback browser".into(),
        width: inner_w,
        tabs: vec!["Commands".into(), "Output".into(), "Json".into(), "Paths".into()],
        active_tab,
        body: body_lines.join("\n"),
        hints: vec![
            Hint::new("j/k", "select"),
            Hint::new("m", "mode"),
            Hint::new("enter", "jump"),
            Hint::new("esc", "close"),
        ],
    };

    let pal = Palette::default();
    let (lines, geo) = panel.render(&pal);

    let panel_w = geo.width as u16;
    let panel_h = geo.height as u16;
    let px = area.x + area.width.saturating_sub(panel_w) / 2;
    let py = area.y + area.height.saturating_sub(panel_h) / 2;

    let bg = os.theme.overlay_bg();
    let fg = os.theme.overlay_fg();

    // Fill the panel background.
    for y in 0..panel_h {
        for x in 0..panel_w {
            if px + x < area.x + area.width && py + y < area.y + area.height {
                let cell = &mut buf[(px + x, py + y)];
                cell.set_char(' ');
                cell.set_style(TuiStyle::default().fg(fg).bg(bg));
            }
        }
    }

    // Render the panel lines.
    for (i, line) in lines.iter().enumerate() {
        let ly = py + i as u16;
        if ly >= area.y + area.height {
            break;
        }
        for (j, ch) in line.chars().enumerate() {
            let lx = px + j as u16;
            if lx >= area.x + area.width {
                break;
            }
            let cell = &mut buf[(lx, ly)];
            cell.set_char(ch);
            cell.set_style(TuiStyle::default().fg(fg).bg(bg));
        }
    }

    // Render block count info at the bottom of the body.
    let info_y = py + panel_h.saturating_sub(3);
    let info = format!("{} block(s)", os.browser_blocks.len());
    for (j, ch) in info.chars().enumerate() {
        let lx = px + 2 + j as u16;
        if lx < area.x + area.width && info_y < area.y + area.height {
            let cell = &mut buf[(lx, info_y)];
            cell.set_char(ch);
            cell.set_style(TuiStyle::default().fg(os.theme.dock_dimmed()).bg(bg));
        }
    }
}

/// Render the accent picker overlay with color swatches.
fn render_accent_picker_overlay(os: &Os, buf: &mut Buffer, area: TuiRect) {
    use crate::config::theme::ThemeColors;
    use crate::ui::overlay::{Hint, Palette, Panel};

    let preferred_width = 40i32;
    let screen_w = area.width as i32;
    let inner_w = crate::ui::overlay::fit_width(preferred_width, screen_w);

    // Build the body with accent color names.
    let mut body_lines = Vec::new();
    for (i, name) in os.accent_list.iter().enumerate() {
        let marker = if i == os.accent_picker_selected { "› " } else { "  " };
        body_lines.push(format!("{marker}{name}"));
    }

    let panel = Panel {
        glyph: String::new(),
        title: "Accent".into(),
        width: inner_w,
        tabs: Vec::new(),
        active_tab: 0,
        body: body_lines.join("\n"),
        hints: vec![
            Hint::new("j/k", "select"),
            Hint::new("enter", "apply"),
            Hint::new("esc", "cancel"),
        ],
    };

    let pal = Palette::default();
    let (lines, geo) = panel.render(&pal);

    let panel_w = geo.width as u16;
    let panel_h = geo.height as u16;
    let px = area.x + area.width.saturating_sub(panel_w) / 2;
    let py = area.y + area.height.saturating_sub(panel_h) / 2;

    let bg = os.theme.overlay_bg();
    let fg = os.theme.overlay_fg();

    // Fill the panel background.
    for y in 0..panel_h {
        for x in 0..panel_w {
            if px + x < area.x + area.width && py + y < area.y + area.height {
                let cell = &mut buf[(px + x, py + y)];
                cell.set_char(' ');
                cell.set_style(TuiStyle::default().fg(fg).bg(bg));
            }
        }
    }

    // Render the panel lines.
    for (i, line) in lines.iter().enumerate() {
        let ly = py + i as u16;
        if ly >= area.y + area.height {
            break;
        }
        for (j, ch) in line.chars().enumerate() {
            let lx = px + j as u16;
            if lx >= area.x + area.width {
                break;
            }
            let cell = &mut buf[(lx, ly)];
            cell.set_char(ch);
            cell.set_style(TuiStyle::default().fg(fg).bg(bg));
        }
    }

    // Draw color swatch next to each accent name.
    let accent_colors: [(&str, TuiColor); 8] = [
        ("blue", TuiColor::Blue),
        ("cyan", TuiColor::Cyan),
        ("green", TuiColor::Green),
        ("magenta", TuiColor::Magenta),
        ("orange", TuiColor::Rgb(0xff, 0x87, 0x00)),
        ("purple", TuiColor::Rgb(0x88, 0x39, 0xbf)),
        ("red", TuiColor::Red),
        ("yellow", TuiColor::Yellow),
    ];
    for (i, name) in os.accent_list.iter().enumerate() {
        let swatch_color = accent_colors
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| *c)
            .unwrap_or(TuiColor::White);
        let line_y = py + 3 + i as u16; // 3 = top pad + title + blank
        if line_y >= area.y + area.height {
            break;
        }
        let sx = px + 4 + name.len() as u16 + 2;
        if sx < area.x + area.width {
            let cell = &mut buf[(sx, line_y)];
            cell.set_char('\u{2588}');
            cell.set_style(TuiStyle::default().fg(swatch_color).bg(bg));
        }
    }
}

/// Render the theme picker overlay with color swatches.
fn render_theme_picker_overlay(os: &Os, buf: &mut Buffer, area: TuiRect) {
    use crate::config::theme::ThemeColors;

    // Calculate overlay dimensions: extra width for swatch column.
    let max_name = os.theme_list.iter().map(|n| n.len()).max().unwrap_or(0);
    let swatch_width = 10; // 8 color blocks + 2 spaces
    let content_width = (max_name + swatch_width + 4) as u16;
    let width = content_width.min(area.width);
    let height = (os.theme_list.len() as u16 + 4).min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let rect = TuiRect::new(x, y, width, height);

    // Draw the border + title.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(TuiStyle::default().fg(os.theme.overlay_border()))
        .title(Span::styled(
            "Theme  (j/k: select, Enter: apply, Esc: cancel)",
            TuiStyle::default().fg(os.theme.overlay_title()),
        ));
    let mut block_buf = Buffer::empty(rect);
    block.render(rect, &mut block_buf);
    for yy in 0..rect.height {
        for xx in 0..rect.width {
            buf[(rect.x + xx, rect.y + yy)] = block_buf[(rect.x + xx, rect.y + yy)].clone();
        }
    }

    // Draw theme names + swatches.
    for (i, name) in os.theme_list.iter().enumerate() {
        let line_y = rect.y + 2 + i as u16;
        if line_y >= rect.y + rect.height - 1 {
            break;
        }
        let prefix = if i == os.theme_picker_selected { "> " } else { "  " };
        let prefix_chars: Vec<char> = prefix.chars().collect();
        for (j, ch) in prefix_chars.iter().enumerate() {
            let px = rect.x + 2 + j as u16;
            if px < rect.x + rect.width - 2 {
                buf[(px, line_y)].set_char(*ch);
            }
        }
        let name_chars: Vec<char> = name.chars().collect();
        for (j, ch) in name_chars.iter().enumerate() {
            let nx = rect.x + 4 + j as u16;
            if nx < rect.x + rect.width - 2 {
                buf[(nx, line_y)].set_char(*ch);
            }
        }
        // Light/dark marker between the name and swatch.
        let light = crate::config::theme::Theme::built_in(name)
            .map(|t| t.is_light())
            .unwrap_or(false);
        let marker = if light { "\u{2600}" } else { "\u{263e}" }; // ☀ / ☾
        let mx = rect.x + 4 + max_name as u16 + 1;
        if mx < rect.x + rect.width - 2 {
            buf[(mx, line_y)].set_char(marker.chars().next().unwrap());
        }
        // Draw swatch: 8 colored block characters.
        let swatch = crate::config::theme::Theme::built_in(name)
            .map(|t| t.swatch())
            .unwrap_or([crate::config::theme::Rgb::new(0, 0, 0); 8]);
        for (j, color) in swatch.iter().enumerate() {
            let sx = rect.x + 4 + max_name as u16 + 2 + j as u16;
            if sx < rect.x + rect.width - 2 {
                let cell = &mut buf[(sx, line_y)];
                cell.set_char('\u{2588}'); // full block
                cell.set_style(TuiStyle::default().fg(TuiColor::Rgb(color.0, color.1, color.2)));
            }
        }
    }
}

pub fn render_overlay(buf: &mut Buffer, area: TuiRect, lines: &[String], title: &str) {
    use crate::app::pixel_canvas::PixelCanvas;

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

    // Paint a dark gradient background for the overlay using the pixel canvas.
    let mut overlay_canvas = PixelCanvas::new(width as usize, height as usize);
    overlay_canvas.clear(20, 20, 30);
    overlay_canvas.gradient_vertical(
        0, 0,
        width as usize, height as usize,
        (25, 25, 38),  // slightly lighter top
        (15, 15, 22),  // darker bottom
    );
    let rgb = overlay_canvas.rgb();

    // Capture the content behind the four corners before painting, so the
    // SDF blend below can fade the overlay's corners out (rounded-corner
    // look instead of square ratatui corners).
    let corners = [
        (0usize, 0usize),
        ((width - 1) as usize, 0usize),
        (0usize, (height - 1) as usize),
        ((width - 1) as usize, (height - 1) as usize),
    ];
    let mut underlying = [(0u8, 0u8, 0u8); 4];
    for (i, &(cx, cy)) in corners.iter().enumerate() {
        let cell = &buf[(rect.x + cx as u16, rect.y + cy as u16)];
        underlying[i] = match cell.bg {
            TuiColor::Rgb(r, g, b) => (r, g, b),
            _ => (20, 20, 30),
        };
    }

    for yy in 0..height {
        for xx in 0..width {
            let idx = ((yy as usize * width as usize) + xx as usize) * 3;
            let cell = &mut buf[(rect.x + xx, rect.y + yy)];
            cell.set_char(' ');
            cell.set_bg(TuiColor::Rgb(rgb[idx], rgb[idx + 1], rgb[idx + 2]));
        }
    }

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

    // SDF rounded corners: blend each corner cell toward the content behind
    // it and drop the square corner glyph, so overlays read as rounded panels.
    if width >= 3 && height >= 3 {
        let radius = 1.0f64;
        for (i, &(cx, cy)) in corners.iter().enumerate() {
            let alpha = crate::app::pixel_canvas::rounded_corner_alpha(
                cx,
                cy,
                width as usize,
                height as usize,
                radius,
            );
            if alpha >= 1.0 {
                continue;
            }
            let idx = ((cy * width as usize) + cx) * 3;
            let grad = (rgb[idx], rgb[idx + 1], rgb[idx + 2]);
            let (u, v, wcol) = underlying[i];
            let blended = (
                crate::app::pixel_canvas::lerp(u, grad.0, alpha),
                crate::app::pixel_canvas::lerp(v, grad.1, alpha),
                crate::app::pixel_canvas::lerp(wcol, grad.2, alpha),
            );
            let cell = &mut buf[(rect.x + cx as u16, rect.y + cy as u16)];
            cell.set_char(' ');
            cell.set_bg(TuiColor::Rgb(blended.0, blended.1, blended.2));
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
    fn paint_emulator_writes_combining_grapheme_into_one_cell() {
        // A decomposed `e` + U+0301 must land in a single ratatui cell as the
        // composed symbol `é`, not as two separate columns.
        use crate::vt::Emulator;
        let palette = StylePalette::new(None);
        let mut emu = Emulator::new(10, 3);
        emu.write("e\u{301}x".as_bytes());

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 14, 5));
        let rect = TuiRect::new(0, 0, 14, 5);
        let lines = emu.render_view_lines();
        paint_emulator(&mut buf, &emu, &lines, rect, &palette);

        // Content is inset one cell by the pane border: base at (1,1).
        let cell = &buf[(1, 1)];
        assert_eq!(cell.symbol(), "e\u{301}");
        let next = &buf[(2, 1)];
        assert_eq!(next.symbol(), "x");
        // The combining mark must not consume a column of its own.
        assert_eq!(buf[(3, 1)].symbol(), " ");
    }

    #[test]
    fn paint_emulator_writes_spilled_combining_run() {
        // Marks beyond the inline budget (5+ on one base) must still render
        // as a single composed symbol in one cell.
        use crate::vt::Emulator;
        let palette = StylePalette::new(None);
        let mut emu = Emulator::new(10, 3);
        let mut s = String::from("q");
        for _ in 0..crate::vt::cell::MAX_COMBINING + 2 {
            s.push('\u{301}');
        }
        emu.write(s.as_bytes());

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 14, 5));
        let lines = emu.render_view_lines();
        paint_emulator(&mut buf, &emu, &lines, TuiRect::new(0, 0, 14, 5), &palette);

        let cell = &buf[(1, 1)];
        assert_eq!(
            cell.symbol().chars().count(),
            1 + crate::vt::cell::MAX_COMBINING + 2
        );
        assert_eq!(cell.symbol().chars().next(), Some('q'));
    }

    #[test]
    fn paint_emulator_spaces_wide_glyphs_and_trailing_text() {
        // Wide CJK glyphs occupy two terminal columns each; the buffer must
        // advance by the glyph width and skip the continuation cell, so
        // trailing text lines up. Regression for the live dogfood finding
        // where `你你XX` collapsed to `你X` and shifted the pane border.
        use crate::vt::Emulator;
        let palette = StylePalette::new(None);
        let mut emu = Emulator::new(10, 3);
        emu.write("\u{4f60}\u{4f60}XX".as_bytes());

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 14, 5));
        let lines = emu.render_view_lines();
        paint_emulator(&mut buf, &emu, &lines, TuiRect::new(0, 0, 14, 5), &palette);

        // Content is inset one cell by the border ring.
        assert_eq!(buf[(1, 1)].symbol(), "\u{4f60}");
        assert!(buf[(2, 1)].skip, "wide continuation must be skipped");
        assert_eq!(buf[(3, 1)].symbol(), "\u{4f60}");
        assert!(buf[(4, 1)].skip, "wide continuation must be skipped");
        assert_eq!(buf[(5, 1)].symbol(), "X");
        assert_eq!(buf[(6, 1)].symbol(), "X");
    }

    #[test]
    fn paint_emulator_spaces_hangul_jamo_run() {
        // A Hangul jamo cluster: lead (width 2) + zero-width vowel + final
        // all pack into one wide cell; the char after must land two columns
        // past the lead.
        use crate::vt::Emulator;
        let palette = StylePalette::new(None);
        let mut emu = Emulator::new(10, 3);
        emu.write("|\u{1112}\u{1161}\u{11ab}|".as_bytes());

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 14, 5));
        let lines = emu.render_view_lines();
        paint_emulator(&mut buf, &emu, &lines, TuiRect::new(0, 0, 14, 5), &palette);

        assert_eq!(buf[(1, 1)].symbol(), "|");
        let run = &buf[(2, 1)];
        assert_eq!(run.symbol(), "\u{1112}\u{1161}\u{11ab}");
        assert!(buf[(3, 1)].skip, "wide continuation must be skipped");
        assert_eq!(buf[(4, 1)].symbol(), "|");
    }

    #[test]
    fn paint_selection_reverses_wide_lead_cells_at_correct_columns() {
        // Selection coordinates are emulator columns; buffer leads must sit
        // at those columns (continuations skipped), so the highlight lands on
        // the wide glyphs and the text after them, not drifting left.
        use crate::app::Selection;
        use crate::vt::Emulator;
        let palette = StylePalette::new(None);
        let mut emu = Emulator::new(10, 3);
        emu.write("\u{4f60}\u{4f60}XX".as_bytes()); // cols: 你 0, 你 2, X 4, X 5

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 14, 5));
        let rect = TuiRect::new(0, 0, 14, 5);
        let lines = emu.render_view_lines();
        paint_emulator(&mut buf, &emu, &lines, rect, &palette);

        let sel = Selection {
            window: 0,
            anchor_line: 0,
            anchor_col: 0,
            cursor_line: 0,
            cursor_col: 5,
        };
        paint_selection(&mut buf, &emu, rect, Some(&sel));

        let reversed = |x: u16| {
            buf[(x, 1)]
                .style()
                .add_modifier
                .intersects(ratatui::style::Modifier::REVERSED)
        };
        // The two wide leads and both X's are highlighted at their emulator
        // columns (1, 3, 5, 6) — no drift.
        assert!(reversed(1), "first wide lead not reversed");
        assert!(reversed(3), "second wide lead not reversed");
        assert!(reversed(5), "first X not reversed");
        assert!(reversed(6), "second X not reversed");
        // The continuation cells stay skipped; nothing past the selection
        // (col 8) is highlighted. Col 7 is the cursor cell, reversed by the
        // cursor drawing in paint_emulator, landing one column past the last
        // char — cursor math also tracks the wide layout.
        assert!(buf[(2, 1)].skip);
        assert!(buf[(4, 1)].skip);
        assert!(!reversed(8));
    }

    #[test]
    fn paint_scrollback_rows_keep_wide_spacing() {
        // Content scrolled into scrollback renders through the same
        // width-aware row path: leads spaced by width, continuation skipped,
        // trailing text intact.
        use crate::vt::Emulator;
        let palette = StylePalette::new(None);
        let mut emu = Emulator::new(10, 3);
        // Fill past the 3 rows so wide-char lines scroll off the top.
        for _ in 0..5 {
            emu.write("\u{4f60}\u{4f60}XX\r\n".as_bytes());
        }
        assert!(emu.scrollback_len() >= 2, "expected scrolled lines");
        emu.scroll_viewport(2);

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 14, 5));
        let rect = TuiRect::new(0, 0, 14, 5);
        let lines = emu.render_view_lines();
        // The scrolled-back rows carry width so the painter spaces them.
        for row in &lines {
            let widths: Vec<u8> = row.iter().map(|sc| sc.width).collect();
            assert_eq!(&widths[..2], &[2, 2], "wide leads must carry width 2");
        }
        paint_emulator(&mut buf, &emu, &lines, rect, &palette);
        // First scrolled-back row: 你 at col 1, continuation skipped, second
        // 你 at col 3, X at 5 and 6.
        assert_eq!(buf[(1, 1)].symbol(), "\u{4f60}");
        assert!(buf[(2, 1)].skip);
        assert_eq!(buf[(3, 1)].symbol(), "\u{4f60}");
        assert!(buf[(4, 1)].skip);
        assert_eq!(buf[(5, 1)].symbol(), "X");
        assert_eq!(buf[(6, 1)].symbol(), "X");
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
    fn render_split_panes_with_offset_rects_does_not_panic() {
        // Regression: `draw_pane_border` reads its scratch buffer back with
        // absolute coordinates; panes offset from the origin (right/top
        // halves, floating panes) used to hit a `None` unwrap.
        use crate::terminal::pty::WinSize;
        use crate::terminal::window::Window;
        let mut os = test_os();
        for i in 0..2 {
            let w = Window::without_pty(
                format!("w{i}"),
                format!("win{i}"),
                WinSize { cols: 10, rows: 3 },
            );
            os.windows.push(w);
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1).tree.insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Vertical, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
        // Both pane borders are painted (the right pane starts at x=40).
        let cell = &buf[(79, 0)];
        assert!(!cell.symbol().is_empty());
    }

    #[test]
    fn render_float_above_tiles_does_not_panic() {
        use crate::app::float::FloatPane;
        use crate::terminal::pty::WinSize;
        use crate::terminal::window::Window;
        let mut os = test_os();
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
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        // A float offsets from the origin: {16, 5, 48, 14} on an 80x24 host.
        os.floats.push(FloatPane {
            window: 1,
            workspace: 1,
            x: 16,
            y: 5,
            w: 48,
            h: 14,
            z: 1,
            pinned: false,
            modal: false,
        });

        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
        // The float's top-left border corner paints at its offset position.
        let cell = &buf[(16, 5)];
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
    fn render_overlay_has_rounded_corners() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        let area = TuiRect::new(0, 0, 80, 24);
        // Underlying content: bright red, so the SDF corner fade target is
        // unambiguous.
        for cell in buf.content.iter_mut() {
            cell.set_bg(TuiColor::Rgb(200, 0, 0));
        }
        let lines = vec!["line1".into(), "line2".into()];
        render_overlay(&mut buf, area, &lines, "Test Title");

        // Overlay is centered; width = 5 + 4 = 9, height = 2 + 4 = 6.
        let x = area.x + area.width.saturating_sub(9) / 2;
        let y = area.y + area.height.saturating_sub(6) / 2;

        // The top-left corner must not carry the square `╭` glyph.
        assert_ne!(buf[(x, y)].symbol(), "╭", "corner glyph should be dropped");

        // Its background must blend toward the underlying red (the SDF fade
        // ran) rather than the interior gradient color (~25,25,38).
        let (r, _, _) = match buf[(x, y)].bg {
            TuiColor::Rgb(r, g, b) => (r, g, b),
            _ => (0, 0, 0),
        };
        assert!(r > 80, "corner should fade toward the underlying red, got r={r}");

        // An interior cell keeps the opaque gradient background.
        let (ri, _, _) = match buf[(x + 3, y + 2)].bg {
            TuiColor::Rgb(r, g, b) => (r, g, b),
            _ => (0, 0, 0),
        };
        assert!(ri < 40, "interior should stay dark, got r={ri}");
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
    fn render_showkeys_requires_debug_flag() {
        let mut os = test_os();
        os.last_key_chord = "Ctrl+A".into();
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        // Disabled by default: the chord must not be drawn.
        render(&os, &mut buf);
        let drawn: String = buf
            .content
            .iter()
            .filter(|c| c.symbol() != " ")
            .map(|c| c.symbol())
            .collect();
        assert!(!drawn.contains('A'), "showkeys must be off by default");

        // Enabled via `[debug] show_key_events`: the chord is drawn.
        os.config.debug.show_key_events = true;
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 80, 24));
        render(&os, &mut buf);
        let drawn: String = buf
            .content
            .iter()
            .filter(|c| c.symbol() != " ")
            .map(|c| c.symbol())
            .collect();
        assert!(
            drawn.contains("Ctrl+A"),
            "showkeys should draw the chord when enabled, got: {drawn:?}"
        );
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
