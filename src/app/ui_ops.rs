use std::collections::HashMap;
use crate::app::copymode_ext;
use crate::app::sidebar;
use crate::config::Theme;
use crate::layout::{BSPTree, Rect, SplitType};
use crate::terminal::pty::WinSize;
use super::{fuzzy_match_tokens, fuzzy_rank, Mode, Prefix, QuitMenuKind, QuitMenuItem, Selection,  QuitMenu};
use super::Os;use super::{Command, ContextAction, ContextMenu, SwitcherEntry, SwitcherKind};

/// Geometry of the which-key overlay for click hit-testing.
pub struct WhichKeyGeo {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub row_count: usize,
}

impl WhichKeyGeo {
    /// Check if (x, y) is on a content row (not border/header).
    /// Returns the row index if so.
    pub fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        if x < self.x || x >= self.x + self.w || y < self.y || y >= self.y + self.h {
            return None;
        }
        let ry = y - self.y - 2;
        if ry >= 0 && (ry as usize) < self.row_count {
            Some(ry as usize)
        } else {
            None
        }
    }
}


impl Os {
    // -----------------------------------------------------------------------

    pub fn open_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_selected = 0;
    }

    pub fn close_palette(&mut self) {
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_selected = 0;
    }

    /// The commands matching the current query, best match first.
    /// Each entry carries the command and the character positions that matched
    /// the query (for rendering with highlights).
    pub fn palette_items(&self) -> Vec<(Command, Vec<usize>)> {
        let mut items: Vec<(i64, Command, Vec<usize>)> = Command::all()
            .into_iter()
            .filter_map(|c| {
                fuzzy_match_tokens(&c.label(), &self.palette_query)
                    .map(|m| (m.score, c, m.positions))
            })
            .collect();
        // Append custom actions from config.
        for action in &self.config.custom_actions {
            if let Some(m) = fuzzy_match_tokens(&action.name, &self.palette_query) {
                items.push((m.score, Command::CustomAction(action.name.clone()), m.positions));
            }
        }
        // Boost recently-used commands: each recency slot gives a -10 bonus
        // so the most-recent command beats any fuzzy-score difference.
        for (score, cmd, _) in &mut items {
            if let Some(pos) = self.palette_recent.iter().rev().position(|r| r == cmd) {
                *score -= ((self.palette_recent.len() - pos) as i64) * 10;
            }
        }
        items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label().cmp(&b.1.label())));
        items.into_iter().map(|(_, c, p)| (c, p)).collect()
    }

    pub fn palette_move(&mut self, delta: i32) {
        let len = self.palette_items().len();
        if len == 0 {
            return;
        }
        let sel = self.palette_selected as i32 + delta;
        self.palette_selected = sel.rem_euclid(len as i32) as usize;
    }

    /// Compute the palette overlay geometry for mouse hit-testing.
    /// Returns (panel_x, panel_y, panel_w, panel_h, row_y_starts).
    pub fn palette_geometry(&self) -> Option<(i32, i32, i32, i32, Vec<i32>)> {
        if !self.palette_open {
            return None;
        }
        let items = self.palette_items();
        let rows: Vec<(String, String)> = items
            .iter()
            .map(|(c, _)| (c.label(), c.category().to_string()))
            .collect();
        let max_row = rows
            .iter()
            .map(|(l, d)| {
                l.chars().count()
                    + if d.is_empty() { 0 } else { 2 + d.chars().count() }
            })
            .max()
            .unwrap_or(0);
        let query_w = self.palette_query.chars().count() + 2;
        let content_w = max_row.max(query_w).max("Commands".len()) + 4;
        let w = (content_w as i32).clamp(20, self.width - 2);
        let h = ((rows.len() + 4) as i32).clamp(3, self.height - 2);
        let px = (self.width - w) / 2;
        let py = (self.height - h) / 2;
        let visible = (h as usize).saturating_sub(3).max(1);
        let start = if rows.len() > visible {
            self.palette_selected.saturating_sub(visible - 1)
        } else {
            0
        };
        let mut row_ys = Vec::new();
        for i in 0..visible {
            if start + i >= rows.len() {
                break;
            }
            let ry = py + 2 + i as i32;
            if ry >= py + h - 1 {
                break;
            }
            row_ys.push(ry);
        }
        Some((px, py, w, h, row_ys))
    }

    /// Compute the which-key overlay geometry: (x, y, w, h, row_count).
    pub fn which_key_geometry(&self) -> Option<WhichKeyGeo> {
        if !self.config.appearance.which_key_enabled || self.prefix == crate::app::types::Prefix::None {
            return None;
        }
        let lines = crate::app::render::build_which_key_lines(self);
        let max_width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as i32 + 4;
        let w = max_width.min(self.width - 2);
        let h = ((lines.len() as i32) + 4).min(self.height - 2);
        let x = (self.width - w) / 2;
        let y = (self.height - h) / 2;
        let row_count = lines.len();
        Some(WhichKeyGeo { x, y, w, h, row_count })
    }

    /// Extract the key character from a which-key line (e.g. "  h          move left" -> "h").
    pub fn which_key_key_at_row(&self, row_idx: usize) -> Option<char> {
        let lines = crate::app::render::build_which_key_lines(self);
        let line = lines.get(row_idx)?;
        // Key lines start with two spaces, then the key, then spaces.
        if line.starts_with("  ") && line.len() > 4 {
            line[2..].chars().find(|c| !c.is_whitespace())
        } else {
            None
        }
    }

    /// Run the selected command and close the palette.
    pub fn activate_palette(&mut self) {
        let items = self.palette_items();
        let cmd = items.get(self.palette_selected).map(|(c, _)| c.clone());
        self.close_palette();
        if let Some(cmd) = cmd {
            // Track recency: remove if already present, push to end, cap at 8.
            self.palette_recent.retain(|c| c != &cmd);
            self.palette_recent.push(cmd.clone());
            if self.palette_recent.len() > 8 {
                self.palette_recent.remove(0);
            }
            self.run_command(cmd);
        }
    }

    pub(crate) fn run_command(&mut self, cmd: Command) {
        match cmd {
            Command::NewWindow => {
                let shell = self.default_shell();
                let _ = self.spawn_window(&shell, Box::new(|| {}));
            }
            Command::CloseWindow => self.close_focused_window(),
            Command::SplitHorizontal => {
                let shell = self.default_shell();
                let _ = self.split(SplitType::Horizontal, &shell, Box::new(|| {}));
            }
            Command::SplitVertical => {
                let shell = self.default_shell();
                let _ = self.split(SplitType::Vertical, &shell, Box::new(|| {}));
            }
            Command::NextWindow => self.focus_next(),
            Command::PrevWindow => self.focus_prev(),
            Command::ToggleTiling => {
                // BSP tiling is always on in this port; the palette entry
                // exists for parity with the Go command list.
            }
            Command::CycleLayoutMode => self.cycle_layout_mode(),
            Command::ToggleFloat => self.toggle_float(),
            Command::FloatNew => {
                let shell = self.default_shell();
                let _ = self.spawn_floating_window(&shell, Box::new(|| {}));
            }
            Command::EqualizeSplits => {
                let ws = self.current_workspace;
                self.workspace_mut(ws).tree.equalize_ratios();
            }
            Command::Scrollback => self.enter_scrollback_mode(),
            Command::SwitchWorkspace(i) => self.switch_workspace(i),
            Command::Quit => {
                self.show_quit_confirmation = true;
            }
            Command::Theme => self.open_theme_picker(),
            Command::ThemeDetect => self.redetect_theme(),
            Command::CommandPane => self.open_command_pane_dialog(),
            Command::Settings => self.open_settings(),
            Command::FocusLeft => {
                let _ = self.focus_direction("left");
            }
            Command::FocusRight => {
                let _ = self.focus_direction("right");
            }
            Command::FocusUp => {
                let _ = self.focus_direction("up");
            }
            Command::FocusDown => {
                let _ = self.focus_direction("down");
            }
            Command::SwapLeft => {
                self.swap_focused_with(crate::layout::PreselectionDir::Left);
            }
            Command::SwapRight => {
                self.swap_focused_with(crate::layout::PreselectionDir::Right);
            }
            Command::SwapUp => {
                self.swap_focused_with(crate::layout::PreselectionDir::Up);
            }
            Command::SwapDown => {
                self.swap_focused_with(crate::layout::PreselectionDir::Down);
            }
            Command::ZoomToggle | Command::Fullscreen => {
                if let Err(e) = self.toggle_zoom_internal() {
                    self.notify(e, "error");
                }
            }
            Command::RenameWindow => self.open_rename_dialog(),
            Command::CopyMode => self.enter_scrollback_mode(),
            Command::ToggleSidebar => self.sidebar.toggle(),
            Command::OpenBrowser => self.open_scrollback_browser(),
            Command::OpenAggregate => self.open_aggregate_view(),
            Command::CommandPalette => self.open_palette(),
            Command::SessionSwitcher => {
                self.open_switcher(SwitcherKind::Session);
            }
            Command::WorkspaceSwitcher => {
                self.open_switcher(SwitcherKind::Workspace);
            }
            Command::LayoutSwitcher => self.open_switcher(SwitcherKind::Layout),
            Command::TapeManager => self.open_tape_manager(),
            Command::AccentPicker => self.open_accent_picker(),
            Command::Detach => self.leave_terminal_mode(),
            Command::Help => self.toggle_help(),
            Command::StackPane => self.stack_focused(),
            Command::CycleStack => self.cycle_stack_focus(true),
            Command::MultiSelect => self.toggle_multi_select_mode(),
            Command::BulkClose => self.bulk_close_selected(),
            Command::BulkStack => self.bulk_stack_selected(),
            Command::BulkBreak => self.bulk_break_selected(),
            Command::CustomAction(name) => self.run_custom_action(&name),
        }
    }

    pub fn default_shell(&self) -> String {
        if !self.config.appearance.preferred_shell.is_empty() {
            return self.config.appearance.preferred_shell.clone();
        }
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }

    // -----------------------------------------------------------------------
    // Switcher (workspace / window)
    // -----------------------------------------------------------------------

    pub fn open_switcher(&mut self, kind: SwitcherKind) {
        self.switcher_kind = kind;
        self.switcher_query.clear();
        self.switcher_selected = 0;
        self.switcher_open = true;
    }

    pub fn close_switcher(&mut self) {
        self.switcher_open = false;
        self.switcher_query.clear();
        self.switcher_selected = 0;
    }

    /// The switcher rows matching the current query.
    pub fn switcher_items(&self) -> Vec<SwitcherEntry> {
        let items = match self.switcher_kind {
            SwitcherKind::Workspace => {
                let mut items = Vec::new();
                for i in 1..=9 {
                    let ws = self.workspace(i);
                    let ids = ws.tree.get_all_window_ids();
                    let focused_title = ws
                        .focused
                        .and_then(|f| self.windows.get(f))
                        .map(|w| w.title.clone())
                        .unwrap_or_default();
                    let detail = if ids.is_empty() {
                        "(empty)".to_string()
                    } else {
                        format!("{} window(s) — {}", ids.len(), focused_title)
                    };
                    items.push(SwitcherEntry {
                        label: format!("{i}: {}", ws.name),
                        detail,
                        workspace: i,
                        window: ws.focused,
                        session: None,
                    });
                }
                items
            }
            SwitcherKind::Window => {
                let mut items = Vec::new();
                for (idx, window) in self.windows.iter().enumerate() {
                    let mut ws_num = 0;
                    for i in 1..=9 {
                        if self.workspace(i).tree.has_window(idx as i32) {
                            ws_num = i;
                            break;
                        }
                    }
                    items.push(SwitcherEntry {
                        label: window.title.clone(),
                        detail: format!("workspace {ws_num}"),
                        workspace: ws_num,
                        window: Some(idx),
                        session: None,
                    });
                }
                items
            }
            SwitcherKind::Layout => {
                let mut items: Vec<SwitcherEntry> = self
                    .layouts
                    .keys()
                    .map(|name| SwitcherEntry {
                        label: name.clone(),
                        detail: "saved layout — Enter to apply, x to delete".into(),
                        workspace: 0,
                        window: None,
                        session: None,
                    })
                    .collect();
                items.sort_by(|a, b| a.label.cmp(&b.label));
                items
            }
            SwitcherKind::Session => {
                let mut items = Vec::new();
                for s in &self.remote_sessions {
                    let mut suffix = String::new();
                    if Some(&s.name) == self.remote_session.as_ref() {
                        suffix.push_str(" (current)");
                    } else if s.attached {
                        suffix.push_str(" (attached)");
                    }
                    items.push(SwitcherEntry {
                        label: format!("{}{}", s.name, suffix),
                        detail: format!("{} window(s)", s.windows),
                        workspace: 0,
                        window: None,
                        session: Some(s.name.clone()),
                    });
                }
                items
            }
            SwitcherKind::Widget => {
                let mut items = Vec::new();
                let meta = self.widget_registry.list_meta();
                for m in &meta {
                    let enabled = !self.enabled_widgets.contains(&m.id);
                    let status = if enabled { "ON" } else { "OFF" };
                    items.push(SwitcherEntry {
                        label: m.name.clone(),
                        detail: format!("[{}] {}", status, m.kind.label()),
                        workspace: 0,
                        window: None,
                        session: None,
                    });
                }
                items.sort_by(|a, b| a.label.cmp(&b.label));
                items
            }
        };
        let mut items: Vec<(usize, SwitcherEntry)> = items
            .into_iter()
            .filter_map(|e| fuzzy_rank(&e.label, &self.switcher_query).map(|r| (r, e)))
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        items.into_iter().map(|(_, e)| e).collect()
    }

    /// Apply a saved layout to the current workspace (`layouts` map).
    pub fn apply_saved_layout(&mut self, name: &str) -> bool {
        if let Some(serialized) = self.layouts.get(name) {
            let tree = BSPTree::deserialize(serialized);
            if let Some(ws) = self.workspaces.get_mut(&self.current_workspace) {
                ws.tree = tree;
                self.notify(format!("applied layout '{name}'"), "info");
                self.log_action(&format!("load_layout {name}"));
                self.sync_window_sizes();
                return true;
            }
        }
        false
    }

    /// Delete a saved layout by name.
    pub fn delete_saved_layout(&mut self, name: &str) {
        self.layouts.remove(name);
        self.notify(format!("deleted layout '{name}'"), "info");
        self.log_action(&format!("delete_layout {name}"));
    }

    pub fn switcher_move(&mut self, delta: i32) {
        let len = self.switcher_items().len();
        if len == 0 {
            return;
        }
        let sel = self.switcher_selected as i32 + delta;
        self.switcher_selected = sel.rem_euclid(len as i32) as usize;
    }

    /// Reorder the selected widget in the widget switcher by `delta` slots.
    /// Only works when the switcher is showing widgets.
    pub fn switcher_widget_reorder(&mut self, delta: i32) {
        if self.switcher_kind != SwitcherKind::Widget {
            return;
        }
        let items = self.switcher_items();
        let len = items.len();
        if len < 2 {
            return;
        }
        let sel = self.switcher_selected;
        let new_sel = (sel as i32 + delta).max(0).min(len as i32 - 1) as usize;
        if new_sel == sel {
            return;
        }
        // Map the displayed items (filtered/sorted by name) to widget IDs.
        let meta = self.widget_registry.list_meta();
        let name_to_id: std::collections::HashMap<&str, &str> = meta.iter().map(|m| (m.name.as_str(), m.id.as_str())).collect();
        let displayed_ids: Vec<String> = items.iter()
            .filter_map(|e| name_to_id.get(e.label.as_str()).map(|id| id.to_string()))
            .collect();
        if sel < displayed_ids.len() && new_sel < displayed_ids.len() {
            // Save current layout for undo before swapping.
            self.last_widget_layout = Some(self.widget_registry.layout().clone());
            let layout = self.widget_registry.layout().clone();
            let mut slots = layout.slots;
            let id_a = &displayed_ids[sel];
            let id_b = &displayed_ids[new_sel];
            if let (Some(pos_a), Some(pos_b)) = (
                slots.iter().position(|s| &s.widget_id == id_a),
                slots.iter().position(|s| &s.widget_id == id_b),
            ) {
                slots.swap(pos_a, pos_b);
            }
            *self.widget_registry.layout_mut() = crate::widgets::layout::WidgetLayout {
                columns: layout.columns,
                rows: layout.rows,
                gap: layout.gap,
                slots,
                visible: layout.visible,
                position: layout.position,
            };
        }
        self.switcher_selected = new_sel;
        self.sync_layout_to_config();
    }

    /// Sync the current widget layout back to the dashboard config and save.
    fn sync_layout_to_config(&mut self) {
        let layout = self.widget_registry.layout().clone();
        self.config.dashboard.widgets = layout.slots.iter().map(|s| {
            crate::config::userconfig::DashboardWidgetConfig {
                id: s.widget_id.clone(),
                col: s.col,
                row: s.row,
                width: s.width,
                height: s.height,
                refresh_ms: 0,
            }
        }).collect();        let _ = self.config.save();
    }

    /// Undo the last widget reorder by restoring the previous layout.
    pub fn undo_widget_reorder(&mut self) {
        if let Some(prev) = self.last_widget_layout.take() {
            *self.widget_registry.layout_mut() = prev;
            self.sync_layout_to_config();
            self.notify("widget order undone", "info");
        }
    }



    /// Activate the selected switcher row: switch workspace and focus window,
    /// or (for the session switcher) request a session switch.
    pub fn activate_switcher(&mut self) {
        let items = self.switcher_items();
        let entry = items.get(self.switcher_selected).cloned();
        self.close_switcher();
        if let Some(entry) = entry {
            if self.switcher_kind == SwitcherKind::Layout {
                if !entry.label.is_empty() {
                    self.apply_saved_layout(&entry.label);
                }
                return;
            }
            if self.switcher_kind == SwitcherKind::Widget {
                // Toggle widget: find the widget ID by name.
                // enabled_widgets tracks DISABLED widgets (empty = all enabled).
                let meta = self.widget_registry.list_meta();
                if let Some(m) = meta.iter().find(|m| m.name == entry.label) {
                    let id = m.id.clone();
                    if self.enabled_widgets.contains(&id) {
                        self.enabled_widgets.remove(&id);
                        self.notify(format!("widget '{}' enabled", m.name), "info");
                    } else {
                        self.enabled_widgets.insert(id);
                        self.notify(format!("widget '{}' disabled", m.name), "info");
                    }
                }
                // Re-open the switcher so the user can toggle more.
                self.switcher_open = true;
                self.switcher_kind = SwitcherKind::Widget;
                return;
            }
            if let Some(session) = entry.session {
                self.pending_switch = Some(session);
                return;
            }
            if entry.workspace >= 1 {
                self.switch_workspace(entry.workspace);
            }
            if let Some(w) = entry.window {
                self.focus_window(w);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Scrollback / copy mode
    // -----------------------------------------------------------------------

    pub fn enter_scrollback_mode(&mut self) {
        let Some(i) = self.focused_window else {
            return;
        };
        self.scrollback_mode = true;
        self.mode = Mode::WindowManagement;
        self.prefix = Prefix::None;
        self.palette_open = false;
        self.switcher_open = false;
        self.copy_visual = false;
        self.selection = None;
        // Start the cursor at the live bottom line.
        if let Some(w) = self.windows.get(i) {
            if let Ok(emu) = w.emulator.lock() {
                let count = emu.content_line_count();
                self.copy_cursor_line = count.saturating_sub(1);
                self.copy_cursor_col = 0;
            }
        }
    }

    pub fn exit_scrollback_mode(&mut self) {
        self.scrollback_mode = false;
        self.copy_visual = false;
        self.copy_visual_line = false;
        self.copy_char_search = None;
        self.copy_search_typing = false;
        self.copy_search_query.clear();
        self.copy_search_state.clear();
        self.copy_count.reset();
        self.copy_pending_g = false;
        self.copy_pending_register = None;
        self.copy_pending_mark = None;
        self.selection = None;
        self.mouse_selecting = false;
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(mut emu) = w.emulator.lock() {
                    emu.reset_viewport();
                }
            }
        }
    }

    /// Whether the focused pane is currently scrolled back into history.
    pub fn focused_in_scrollback(&self) -> bool {
        self.focused_window
            .and_then(|i| self.windows.get(i))
            .and_then(|w| w.emulator.lock().ok())
            .map(|emu| emu.in_scrollback())
            .unwrap_or(false)
    }

    /// Scroll the focused pane's viewport (positive = back in history).
    pub fn scroll_focused_viewport(&mut self, delta: i32) {
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(mut emu) = w.emulator.lock() {
                    emu.scroll_viewport(delta);
                }
            }
        }
    }

    /// Scroll an arbitrary window's viewport (mouse wheel over any pane).
    pub fn scroll_window_viewport(&self, index: usize, delta: i32) {
        if let Some(w) = self.windows.get(index) {
            if let Ok(mut emu) = w.emulator.lock() {
                emu.scroll_viewport(delta);
            }
        }
    }

    // -- Copy-mode cursor & selection --------------------------------------

    /// Move the copy-mode cursor vertically by `delta` content lines,
    /// auto-scrolling the viewport to keep it visible.
    pub fn copy_move_line(&mut self, delta: i32) {
        let Some(i) = self.focused_window else {
            return;
        };
        let count = {
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        if count == 0 {
            return;
        }
        let new = (self.copy_cursor_line as i64 + delta as i64).clamp(0, count as i64 - 1) as usize;
        self.copy_cursor_line = new;
        self.scroll_to_cursor(new);
        self.sync_selection_cursor();
    }

    /// Move the copy-mode cursor horizontally by `delta` columns.
    pub fn copy_move_col(&mut self, delta: i32) {
        self.copy_cursor_col = (self.copy_cursor_col + delta).max(0);
        self.sync_selection_cursor();
    }

    /// Move a copy-mode line motion by a vim count.
    pub fn copy_move_lines(&mut self, delta: i32, count: usize) {
        let distance = delta.saturating_mul(count.min(i32::MAX as usize) as i32);
        self.copy_move_line(distance);
    }

    /// Jump the copy-mode cursor to the oldest line.
    pub fn copy_top(&mut self) {
        self.copy_cursor_line = 0;
        self.scroll_to_cursor(0);
        self.sync_selection_cursor();
    }

    /// Jump the copy-mode cursor to the live bottom line.
    pub fn copy_bottom(&mut self) {
        let Some(i) = self.focused_window else {
            return;
        };
        if let Some(w) = self.windows.get(i) {
            if let Ok(mut emu) = w.emulator.lock() {
                let count = emu.content_line_count();
                self.copy_cursor_line = count.saturating_sub(1);
                self.copy_cursor_col = 0;
                emu.reset_viewport();
            }
        }
        self.sync_selection_cursor();
    }

    /// Adjust the viewport so `line` is visible, keeping it as close to the
    /// current view as possible.
    fn scroll_to_cursor(&mut self, line: usize) {
        let Some(i) = self.focused_window else {
            return;
        };
        let Some(w) = self.windows.get(i) else {
            return;
        };
        let Ok(mut emu) = w.emulator.lock() else {
            return;
        };
        let sb = emu.scrollback_len();
        let h = emu.height() as usize;
        if h == 0 {
            return;
        }
        let viewport = emu.viewport();
        let view_start = sb.saturating_sub(viewport);
        let new_viewport = if line < view_start {
            sb.saturating_sub(line)
        } else if line >= view_start + h {
            sb.saturating_sub(line) + h - 1
        } else {
            viewport
        };
        emu.set_viewport(new_viewport);
    }

    fn sync_selection_cursor(&mut self) {
        if self.copy_visual {
            if let Some(sel) = &mut self.selection {
                sel.cursor_line = self.copy_cursor_line;
                sel.cursor_col = self.copy_cursor_col;
            }
        }
    }

    /// Toggle vim visual selection anchored at the copy-mode cursor.
    /// `line_wise` true for `V` (line-wise), false for `v` (char-wise).
    pub fn toggle_visual(&mut self, line_wise: bool) {
        let Some(i) = self.focused_window else {
            return;
        };
        if self.copy_visual {
            self.copy_visual = false;
            self.copy_visual_line = false;
            self.selection = None;
        } else {
            self.copy_visual = true;
            self.copy_visual_line = line_wise;
            self.selection = Some(Selection {
                window: i,
                anchor_line: self.copy_cursor_line,
                anchor_col: self.copy_cursor_col,
                cursor_line: self.copy_cursor_line,
                cursor_col: self.copy_cursor_col,
            });
        }
    }

    /// Open the context menu at a cell (right-click), focusing the window
    /// under the cursor first.
    pub fn open_context_menu_at(&mut self, x: i32, y: i32) {
        if let Some(idx) = self.window_at(x, y) {
            self.focus_window(idx);
        }
        self.context_menu = Some(ContextMenu {
            x,
            y,
            selected: 0,
            items: ContextMenu::standard(),
        });
    }

    /// Open the rename dialog for the focused window.
    pub fn open_rename_dialog(&mut self) {
        let Some(index) = self.focused_window else {
            return;
        };
        let title = self
            .windows
            .get(index)
            .map(|w| w.title.clone())
            .unwrap_or_default();
        self.rename_dialog = Some((index, title));
    }

    /// Commit the rename dialog text to the window title.
    pub fn commit_rename_dialog(&mut self) {
        if let Some((index, text)) = self.rename_dialog.take() {
            let text = text.trim().to_string();
            if !text.is_empty() {
                self.rename_window(index, &text);
                self.log_action(&format!("rename_window {text}"));
            }
        }
    }

    /// Cancel the rename dialog without applying.
    pub fn cancel_rename_dialog(&mut self) {
        self.rename_dialog = None;
    }

    /// Open the "New command pane" text-input dialog.
    pub fn open_command_pane_dialog(&mut self) {
        self.command_pane_dialog = Some((String::new(), false));
    }

    /// Commit the command-pane dialog text: spawn the command as a command
    /// pane. The suspended flag comes from the dialog's current toggle state.
    pub fn commit_command_pane_dialog_inner(&mut self) {
        if let Some((text, suspended)) = self.command_pane_dialog.take() {
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            match self.spawn_command_window(&text, suspended) {
                Ok(_) => self.log_action(&format!("command_pane {text}")),
                Err(e) => self.notify(format!("command pane: {e}"), "error"),
            }
        }
    }

    /// Toggle the suspended flag in the command-pane dialog.
    pub fn toggle_command_pane_suspended(&mut self) {
        if let Some((_, ref mut suspended)) = self.command_pane_dialog {
            *suspended = !*suspended;
        }
    }

    /// Cancel the command-pane dialog without spawning.
    pub fn cancel_command_pane_dialog(&mut self) {
        self.command_pane_dialog = None;
    }

    /// Re-run the focused window if it is a finished command pane.
    /// Returns `true` when a re-run was triggered.
    pub fn rerun_focused_command_pane(&mut self) -> bool {
        let Some(i) = self.focused_window else {
            return false;
        };
        if !self.windows[i].can_rerun() {
            return false;
        }
        let size = self.windows[i]
            .last_geometry()
            .map(|g| WinSize {
                cols: g.width.max(1) as u16,
                rows: g.height.max(1) as u16,
            })
            .unwrap_or(WinSize { cols: 80, rows: 24 });
        let env = crate::util::guestenv::base_guest_env("local", &self.windows[i].id, false, false);
        let wake = Box::new(|| {}) as Box<dyn Fn() + Send + 'static>;
        match self.windows[i].restart(size, wake, &env) {
            Ok(()) => {
                let title = self.windows[i].title.clone();
                self.notify(format!("re-ran {title}"), "info");
                self.log_action("command_pane_rerun");
                true
            }
            Err(e) => {
                self.notify(format!("command pane: {e}"), "error");
                true
            }
        }
    }

    /// Resume the focused window if it is a suspended command pane.
    /// Returns `true` when a pane was resumed.
    pub fn resume_focused_suspended_pane(&mut self) -> bool {
        let Some(i) = self.focused_window else {
            return false;
        };
        self.windows[i].resume_if_suspended()
    }

    /// Poll every command pane for a finished child and record its exit
    /// status. Called from the render path each frame.
    pub fn poll_window_exits(&mut self) {
        let mut finished: Vec<(String, i32)> = Vec::new();
        for w in &mut self.windows {
            if w.poll_exit() {
                finished.push((w.title.clone(), w.exit_code.unwrap_or(-1)));
            }
        }
        for (title, code) in finished {
            let level = if code == 0 { "success" } else { "error" };
            self.notify(format!("command pane '{title}' finished (exit {code})"), level);
        }
    }

    /// Open the scrollback browser for the focused window: parse its
    /// semantic markers into command blocks (prompt fallback when no markers).
    pub fn open_scrollback_browser(&mut self) {
        let Some(i) = self.focused_window else { return };
        let Some(window) = self.windows.get(i) else {
            return;
        };
        let (markers, count) = {
            let Ok(emu) = window.emulator.lock() else {
                return;
            };
            let markers = emu.semantic_markers().markers();
            let count = emu.content_line_count();
            (markers, count)
        };
        let text = |line: usize| {
            self.windows
                .get(i)
                .and_then(|w| w.emulator.lock().ok())
                .map(|emu| emu.content_line_text(line))
                .unwrap_or_default()
        };
        self.browser_blocks = crate::scrollback::parse_blocks(&markers, count, text);
        self.browser_selected = 0;
        self.browser_scroll = 0;
        self.browser_open = true;
    }

    /// Close the scrollback browser.
    pub fn close_scrollback_browser(&mut self) {
        self.browser_open = false;
        self.browser_blocks.clear();
        self.browser_selected = 0;
        self.browser_scroll = 0;
    }

    /// Cycle the browser display mode (Commands/Output/JSON/Paths).
    pub fn cycle_browser_mode(&mut self) {
        use crate::scrollback::BrowseMode;
        self.browser_mode = match self.browser_mode {
            BrowseMode::Commands => BrowseMode::Output,
            BrowseMode::Output => BrowseMode::Json,
            BrowseMode::Json => BrowseMode::Paths,
            BrowseMode::Paths => BrowseMode::Commands,
        };
        self.browser_scroll = 0;
    }

    /// The text rows for the selected block in the current mode.
    pub fn browser_rows(&self) -> Vec<String> {
        use crate::scrollback::BrowseMode;
        match self.browser_mode {
            BrowseMode::Commands => self
                .browser_blocks
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let marker = if i == self.browser_selected {
                        "› "
                    } else {
                        "  "
                    };
                    format!("{marker}{}", b.command)
                })
                .collect(),
            BrowseMode::Output => {
                let mut rows: Vec<String> = Vec::new();
                for (i, b) in self.browser_blocks.iter().enumerate() {
                    if i == self.browser_selected {
                        rows.push(format!("── {} ──", b.command));
                        rows.extend(b.output.lines().map(|l| l.to_string()));
                    }
                }
                if rows.is_empty() {
                    rows.push("(no output)".into());
                }
                rows
            }
            BrowseMode::Json => {
                let mut rows: Vec<String> = Vec::new();
                for (i, b) in self.browser_blocks.iter().enumerate() {
                    if i == self.browser_selected {
                        rows.push(format!("── {} ──", b.command));
                        for frag in crate::scrollback::extract_json(&b.output) {
                            rows.push(frag);
                        }
                    }
                }
                if rows.is_empty() {
                    rows.push("(no JSON found)".into());
                }
                rows
            }
            BrowseMode::Paths => {
                let mut rows: Vec<String> = Vec::new();
                for (i, b) in self.browser_blocks.iter().enumerate() {
                    if i == self.browser_selected {
                        rows.push(format!("── {} ──", b.command));
                        for p in crate::scrollback::extract_paths(&b.output) {
                            rows.push(p);
                        }
                    }
                }
                if rows.is_empty() {
                    rows.push("(no paths found)".into());
                }
                rows
            }
        }
    }

    /// The sidebar rail rows for the current state.
    pub fn sidebar_rows(&self) -> Vec<sidebar::SidebarRow> {
        let ws_of = |idx: usize| {
            for i in 1..=9 {
                if self.workspace(i).tree.has_window(idx as i32) {
                    return i;
                }
            }
            0
        };
        sidebar::build_rows(
            self.remote_session.as_deref(),
            &self.remote_sessions,
            &self.windows,
            self.current_workspace,
            ws_of,
        )
    }

    /// Activate the selected sidebar row: switch to the session (daemon mode)
    /// or focus the window.
    pub fn activate_sidebar_selection(&mut self) {
        let rows = self.sidebar_rows();
        let Some(row) = rows.get(self.sidebar.selected) else {
            self.sidebar.close();
            return;
        };
        self.sidebar.close();
        if let Some(session) = &row.session {
            self.pending_switch = Some(session.clone());
            return;
        }
        if let Some(idx) = row.window {
            if row.workspace >= 1 {
                self.switch_workspace(row.workspace);
            }
            self.focus_window(idx);
        }
    }

    /// Open the aggregate view.
    pub fn open_aggregate_view(&mut self) {
        self.aggregate_open = true;
        self.aggregate_selected = 0;
    }

    /// Close the aggregate view.
    pub fn close_aggregate_view(&mut self) {
        self.aggregate_open = false;
        self.aggregate_selected = 0;
    }

    /// Every window across every workspace, grouped by workspace: returns
    /// (workspace, window index, title, first content line).
    pub fn aggregate_items(&self) -> Vec<(i32, usize, String, String)> {
        let mut items = Vec::new();
        for ws in 1..=9 {
            let ids = self.workspace(ws).tree.get_all_window_ids();
            if ids.is_empty() && self.floats_on_workspace(ws).is_empty() {
                continue;
            }
            let mut windows: Vec<usize> = ids.iter().map(|&i| i as usize).collect();
            for fi in self.floats_on_workspace(ws) {
                windows.push(self.floats[fi].window);
            }
            for idx in windows {
                let Some(window) = self.windows.get(idx) else {
                    continue;
                };
                let preview = window
                    .emulator
                    .lock()
                    .ok()
                    .and_then(|emu| {
                        (0..emu.content_line_count())
                            .map(|i| emu.content_line_text(i))
                            .find(|l| !l.trim().is_empty())
                    })
                    .unwrap_or_default();
                let cwd = window.cwd();
                let detail = if cwd.is_empty() {
                    preview
                } else {
                    format!("[{}] {}", cwd, preview)
                };
                items.push((ws, idx, window.title.clone(), detail));
            }
        }
        items
    }

    /// Activate the selected aggregate row: switch to its workspace and focus
    /// the window.
    pub fn activate_aggregate_selection(&mut self) {
        let items = self.aggregate_items();
        let Some((ws, idx, _, _)) = items.get(self.aggregate_selected) else {
            self.close_aggregate_view();
            return;
        };
        self.close_aggregate_view();
        self.switch_workspace(*ws);
        self.focus_window(*idx);
    }

    /// Open the quit menu, building rows from the session state (daemon vs
    /// standalone). The first row is always the safe default.
    pub fn open_quit_menu(&mut self) {
        let busy = self.windows.iter().any(|w| !w.exited);
        let items = if self.remote_session.is_some() {
            let has_others = self
                .remote_sessions
                .iter()
                .any(|s| Some(&s.name) != self.remote_session.as_ref());
            let mut items = vec![QuitMenuItem {
                label: "Detach — leave session running".into(),
                key: 'D',
                kind: QuitMenuKind::Detach,
                warn: false,
            }];
            if has_others {
                items.push(QuitMenuItem {
                    label: "Switch session".into(),
                    key: 'S',
                    kind: QuitMenuKind::SwitchSession,
                    warn: false,
                });
                items.push(QuitMenuItem {
                    label: "Kill this session and quit".into(),
                    key: 'K',
                    kind: QuitMenuKind::KillAndQuit,
                    warn: busy,
                });
            } else {
                items.push(QuitMenuItem {
                    label: "Kill this session and quit".into(),
                    key: 'K',
                    kind: QuitMenuKind::KillAndQuit,
                    warn: busy,
                });
            }
            items.push(QuitMenuItem {
                label: "Cancel".into(),
                key: 'C',
                kind: QuitMenuKind::Cancel,
                warn: false,
            });
            items
        } else {
            vec![
                QuitMenuItem {
                    label: "Quit".into(),
                    key: 'Q',
                    kind: QuitMenuKind::Standalone,
                    warn: false,
                },
                QuitMenuItem {
                    label: "Cancel".into(),
                    key: 'C',
                    kind: QuitMenuKind::Cancel,
                    warn: false,
                },
            ]
        };
        self.quit_menu = Some(QuitMenu { selected: 0, items });
    }

    /// Dismiss the quit menu without running anything.
    pub fn close_quit_menu(&mut self) {
        self.quit_menu = None;
    }

    /// Run the selected quit-menu row. Returns `true` when the client should
    /// quit.
    pub fn run_quit_menu_selection(&mut self) -> bool {
        let Some(menu) = self.quit_menu.take() else {
            return false;
        };
        let Some(item) = menu.items.get(menu.selected) else {
            return false;
        };
        match item.kind {
            QuitMenuKind::Detach | QuitMenuKind::Standalone => {
                self.quitting = true;
                true
            }
            QuitMenuKind::SwitchSession => {
                self.open_switcher(SwitcherKind::Session);
                false
            }
            QuitMenuKind::KillAndQuit => {
                if let Some(current) = self.remote_session.clone() {
                    self.pending_kill = Some(current);
                    self.quit_after_kill = true;
                } else {
                    self.quitting = true;
                    return true;
                }
                false
            }
            QuitMenuKind::Cancel => false,
        }
    }

    /// Open the session-close confirmation for a session. Raised every time,
    /// whatever the session holds; the toll line counts what would be lost.
    pub fn open_session_close(&mut self, session: &str) {
        self.session_close = Some((session.to_string(), 0));
    }

    /// Cancel the session-close confirmation.
    pub fn cancel_session_close(&mut self) {
        self.session_close = None;
    }

    /// The toll closing `session` would take: panes and agent-marked windows.
    pub fn session_toll(&self, session: &str) -> (usize, usize) {
        let windows = self
            .remote_sessions
            .iter()
            .find(|s| s.name == session)
            .map(|s| s.windows)
            .unwrap_or(0);
        // Agent-marked windows are only known locally for the attached
        // session; remote sessions report their count from the listing.
        let agents = if self.remote_session.as_deref() == Some(session) {
            self.windows
                .iter()
                .filter(|w| !w.agent_state.is_empty())
                .count()
        } else {
            0
        };
        (windows, agents)
    }

    /// Confirm the close: request the kill.
    pub fn confirm_session_close(&mut self) {
        if let Some((session, _)) = self.session_close.take() {
            self.pending_kill = Some(session);
        }
    }

    /// Dismiss the context menu (also called by the next click anywhere).
    pub fn dismiss_context_menu(&mut self) {
        self.context_menu = None;
    }

    /// Run the selected context-menu action.
    pub fn run_context_action(&mut self, action: ContextAction) {
        let shell = self.default_shell();
        match action {
            ContextAction::NewWindow => {
                let wake = Box::new(|| {});
                let _ = self.split(crate::layout::SplitType::None, &shell, wake);
            }
            ContextAction::SplitHorizontal => {
                let wake = Box::new(|| {});
                let _ = self.split(crate::layout::SplitType::Horizontal, &shell, wake);
            }
            ContextAction::SplitVertical => {
                let wake = Box::new(|| {});
                let _ = self.split(crate::layout::SplitType::Vertical, &shell, wake);
            }
            ContextAction::CloseWindow => self.close_focused_window(),
            ContextAction::Rename => self.open_rename_dialog(),
            ContextAction::Zoom => {
                let _ = self.toggle_zoom_internal();
            }
            ContextAction::Copy => self.yank_selection(),
            ContextAction::Paste => {
                let text = self.clipboard.clone();
                if !text.is_empty() {
                    self.write_to_focused(text.as_bytes());
                }
            }
            ContextAction::Cancel => {}
        }
    }

    /// Copy the current selection to the clipboard and the host terminal via
    /// OSC 52.
    pub fn yank_selection(&mut self) {
        let Some(sel) = self.selection.clone() else {
            return;
        };
        let Some(w) = self.windows.get(sel.window) else {
            return;
        };
        let text = {
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.selection_text(
                sel.anchor_line,
                sel.anchor_col,
                sel.cursor_line,
                sel.cursor_col,
            )
        };
        self.selection = None;
        self.copy_visual = false;
        self.mouse_selecting = false;
        self.clipboard = text.clone();
        if !text.is_empty() {
            Self::emit_osc52(&text);
        }
        self.notify(format!("yanked {} char(s)", text.chars().count()), "info");
    }

    /// Emit an OSC 52 clipboard-write to the host terminal.
    fn emit_osc52(text: &str) {
        use base64::Engine;
        use std::io::Write;
        let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let seq = format!("\x1b]52;c;{b64}\x07");
        let mut out = std::io::stdout();
        let _ = out.write_all(seq.as_bytes());
        let _ = out.flush();
    }

    // -- Copy-mode word/char/line motions -----------------------------------

    /// Get the text of the content line at `line` (or empty if out of range).
    fn copy_line_text(&self, line: usize) -> String {
        let Some(i) = self.focused_window else {
            return String::new();
        };
        let Some(w) = self.windows.get(i) else {
            return String::new();
        };
        let Ok(emu) = w.emulator.lock() else {
            return String::new();
        };
        emu.content_line_text(line)
    }

    /// Move cursor to the first non-blank column of the current line.
    pub fn copy_first_non_blank(&mut self) {
        let text = self.copy_line_text(self.copy_cursor_line);
        let col = text
            .char_indices()
            .skip_while(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i as i32)
            .next()
            .unwrap_or(0);
        self.copy_cursor_col = col;
        self.sync_selection_cursor();
    }

    /// Move cursor to the last non-blank column of the current line.
    pub fn copy_last_non_blank(&mut self) {
        let text = self.copy_line_text(self.copy_cursor_line);
        let trimmed = text.trim_end();
        let col = trimmed.chars().count() as i32;
        self.copy_cursor_col = col;
        self.sync_selection_cursor();
    }

    /// Move cursor to column 0.
    pub fn copy_col_zero(&mut self) {
        self.copy_cursor_col = 0;
        self.sync_selection_cursor();
    }

    /// Move to the next word start (`w` motion).
    /// `big` true uses whitespace-only word boundaries (`W`).
    pub fn copy_word_forward(&mut self, big: bool) {
        let text = self.copy_line_text(self.copy_cursor_line);
        if self.copy_cursor_col as usize >= text.chars().count() {
            // At end of line — move to next line.
            self.copy_move_line(1);
            self.copy_cursor_col = 0;
            self.sync_selection_cursor();
            return;
        }
        let motion = if big {
            copymode_ext::WordMotion::WordForwardBig
        } else {
            copymode_ext::WordMotion::WordForward
        };
        self.copy_cursor_col =
            copymode_ext::word_motion(&text, self.copy_cursor_col as usize, motion) as i32;
        self.sync_selection_cursor();
    }

    /// Move to the previous word start (`b` motion).
    pub fn copy_word_backward(&mut self, big: bool) {
        let text = self.copy_line_text(self.copy_cursor_line);
        if text.chars().count() == 0 {
            self.copy_move_line(-1);
            self.copy_last_non_blank();
            return;
        }
        let motion = if big {
            copymode_ext::WordMotion::WordBackwardBig
        } else {
            copymode_ext::WordMotion::WordBackward
        };
        self.copy_cursor_col =
            copymode_ext::word_motion(&text, self.copy_cursor_col as usize, motion) as i32;
        self.sync_selection_cursor();
    }

    /// Move to the next word end (`e` motion).
    pub fn copy_word_end(&mut self, big: bool) {
        let text = self.copy_line_text(self.copy_cursor_line);
        let motion = if big {
            copymode_ext::WordMotion::WordEndBig
        } else {
            copymode_ext::WordMotion::WordEnd
        };
        self.copy_cursor_col =
            copymode_ext::word_motion(&text, self.copy_cursor_col as usize, motion) as i32;
        self.sync_selection_cursor();
    }

    /// Move to the next/previous occurrence of `target` on the current line.
    /// `forward` true = `f`/`t`, false = `F`/`T`.
    /// `till` true = `t`/`T` (stop before), false = `f`/`F` (land on).
    /// Delegates to `copymode_ext::find_char_on_line` (wide-character aware).
    pub fn copy_char_search(&mut self, target: char, forward: bool, till: bool) {
        let text = self.copy_line_text(self.copy_cursor_line);
        let search = if forward {
            if till {
                copymode_ext::CharSearch::forward_till(target)
            } else {
                copymode_ext::CharSearch::forward_find(target)
            }
        } else if till {
            copymode_ext::CharSearch::backward_till(target)
        } else {
            copymode_ext::CharSearch::backward_find(target)
        };
        if let Some(col) =
            copymode_ext::find_char_on_line(&text, self.copy_cursor_col as usize, &search)
        {
            self.copy_cursor_col = col as i32;
            self.sync_selection_cursor();
            self.copy_last_char_search = Some((target, forward, till));
        }
    }

    /// Repeat the last char search (`;`), optionally reversed (`,`).
    pub fn copy_char_search_repeat(&mut self, reverse: bool) {
        if let Some((target, forward, till)) = self.copy_last_char_search {
            let fwd = if reverse { !forward } else { forward };
            self.copy_char_search(target, fwd, till);
        }
    }

    /// Jump to the matching bracket (`%` motion), if the cursor is on one.
    pub fn copy_bracket_match(&mut self) {
        let text = self.copy_line_text(self.copy_cursor_line);
        if let Some(col) = copymode_ext::find_matching_bracket(&text, self.copy_cursor_col as usize)
        {
            self.copy_cursor_col = col as i32;
            self.sync_selection_cursor();
        }
    }

    /// Execute the pending regex search.
    pub fn copy_execute_search(&mut self) {
        if self.copy_search_query.is_empty() {
            self.copy_search_typing = false;
            return;
        }
        let query = self.copy_search_query.clone();
        let forward = self.copy_search_forward;
        self.copy_search_typing = false;
        self.copy_search_next_match(&query, forward, false);
    }

    /// Find the next/previous match of `query` from the current cursor.
    pub fn copy_search_next_match(&mut self, query: &str, forward: bool, reverse: bool) {
        let fwd = if reverse { !forward } else { forward };
        let Ok(re) = regex::Regex::new(query) else {
            return;
        };
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        let start_line = self.copy_cursor_line;
        let start_col = self.copy_cursor_col as usize;
        if fwd {
            // Search forward from current position.
            for line in start_line..count {
                let text = self.copy_line_text(line);
                let start = if line == start_line { start_col + 1 } else { 0 };
                if let Some(m) = re.find_at(&text, start) {
                    self.copy_cursor_line = line;
                    self.copy_cursor_col = m.start() as i32;
                    self.scroll_to_cursor(line);
                    self.sync_selection_cursor();
                    return;
                }
            }
        } else {
            // Search backward from current position.
            for line in (0..=start_line).rev() {
                let text = self.copy_line_text(line);
                let end = if line == start_line {
                    start_col
                } else {
                    text.len()
                };
                if let Some(pos) = re.find_iter(&text[..end]).last() {
                    self.copy_cursor_line = line;
                    self.copy_cursor_col = pos.start() as i32;
                    self.scroll_to_cursor(line);
                    self.sync_selection_cursor();
                    return;
                }
            }
        }
    }

    /// Move cursor to top of viewport (`H`).
    pub fn copy_viewport_top(&mut self) {
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(emu) = w.emulator.lock() {
                    let sb = emu.scrollback_len();
                    let viewport = emu.viewport();
                    self.copy_cursor_line = sb.saturating_sub(viewport);
                }
            }
        }
        self.sync_selection_cursor();
    }

    /// Move cursor to middle of viewport (`M`).
    pub fn copy_viewport_middle(&mut self) {
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(emu) = w.emulator.lock() {
                    let sb = emu.scrollback_len();
                    let viewport = emu.viewport();
                    let h = emu.height() as usize;
                    self.copy_cursor_line = sb.saturating_sub(viewport) + h / 2;
                }
            }
        }
        self.sync_selection_cursor();
    }

    /// Move cursor to bottom of viewport (`L`).
    pub fn copy_viewport_bottom(&mut self) {
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(emu) = w.emulator.lock() {
                    let sb = emu.scrollback_len();
                    let viewport = emu.viewport();
                    let h = emu.height() as usize;
                    self.copy_cursor_line = sb.saturating_sub(viewport) + h.saturating_sub(1);
                }
            }
        }
        self.sync_selection_cursor();
    }

    /// Move to the next/previous blank line (`}`/`{`).
    pub fn copy_blank_line(&mut self, forward: bool) {
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        if count == 0 {
            return;
        }
        let delta = if forward { 1 } else { -1 };
        let mut line = self.copy_cursor_line as i64 + delta;
        while line >= 0 && (line as usize) < count {
            let text = self.copy_line_text(line as usize);
            if text.trim().is_empty() {
                self.copy_cursor_line = line as usize;
                self.copy_cursor_col = 0;
                self.scroll_to_cursor(line as usize);
                self.sync_selection_cursor();
                return;
            }
            line += delta;
        }
    }

    /// Move to the start of the next sentence (`)` motion).
    pub fn copy_sentence_next(&mut self) {
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        let start_line = self.copy_cursor_line;
        let start_col = self.copy_cursor_col as usize;
        let text = |line: usize| self.copy_line_text(line);
        let (line, col) =
            copymode_ext::next_sentence(count, start_line, start_col, text);
        if line != start_line || col != start_col {
            self.copy_cursor_line = line;
            self.copy_cursor_col = col as i32;
            self.scroll_to_cursor(line);
            self.sync_selection_cursor();
        }
    }

    /// Move to the start of the previous sentence (`(` motion).
    pub fn copy_sentence_prev(&mut self) {
        let start_line = self.copy_cursor_line;
        let start_col = self.copy_cursor_col as usize;
        let text = |line: usize| self.copy_line_text(line);
        let (line, col) =
            copymode_ext::prev_sentence(start_line, start_col, text);
        if line != start_line || col != start_col {
            self.copy_cursor_line = line;
            self.copy_cursor_col = col as i32;
            self.scroll_to_cursor(line);
            self.sync_selection_cursor();
        }
    }

    /// Move to the start of the next paragraph (`}` motion, paragraph-aware).
    /// Unlike `copy_blank_line` which stops on the blank line, this skips
    /// past blank lines to the first non-blank line of the next paragraph.
    pub fn copy_paragraph_next(&mut self) {
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        let start = self.copy_cursor_line;
        let text = |line: usize| self.copy_line_text(line);
        let target = copymode_ext::paragraph_forward(count, start, text);
        if target != start {
            self.copy_cursor_line = target;
            self.copy_cursor_col = 0;
            self.scroll_to_cursor(target);
            self.sync_selection_cursor();
        }
    }

    /// Move to the start of the previous paragraph (`{` motion, paragraph-aware).
    pub fn copy_paragraph_prev(&mut self) {
        let start = self.copy_cursor_line;
        let text = |line: usize| self.copy_line_text(line);
        let target = copymode_ext::paragraph_backward(start, text);
        if target != start {
            self.copy_cursor_line = target;
            self.copy_cursor_col = 0;
            self.scroll_to_cursor(target);
            self.sync_selection_cursor();
        }
    }

    /// Set a vim-style mark at the current cursor position (`m{letter}`).
    pub fn copy_set_mark(&mut self, letter: char) {
        self.copy_marks
            .set(letter, self.copy_cursor_line, self.copy_cursor_col as usize);
    }

    /// Jump the cursor to a vim-style mark (`'{letter}` or `` `{letter} ``).
    /// `exact_col` true for backtick (exact column), false for apostrophe
    /// (first non-blank column of the mark's line).
    pub fn copy_goto_mark(&mut self, letter: char, exact_col: bool) {
        if let Some(mark) = self.copy_marks.get(letter) {
            self.copy_cursor_line = mark.line;
            if exact_col {
                self.copy_cursor_col = mark.col as i32;
            } else {
                let text = self.copy_line_text(mark.line);
                let col = text
                    .char_indices()
                    .skip_while(|(_, c)| c.is_whitespace())
                    .map(|(i, _)| i as i32)
                    .next()
                    .unwrap_or(0);
                self.copy_cursor_col = col;
            }
            self.scroll_to_cursor(mark.line);
            self.sync_selection_cursor();
        }
    }

    /// Yank the current selection into a named register (or the unnamed
    /// register if `register` is `None`). Also copies to the clipboard and
    /// emits OSC 52.
    pub fn yank_selection_to_register(&mut self, register: Option<char>) {
        let Some(sel) = self.selection.clone() else {
            return;
        };
        let Some(w) = self.windows.get(sel.window) else {
            return;
        };
        let text = {
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.selection_text(
                sel.anchor_line,
                sel.anchor_col,
                sel.cursor_line,
                sel.cursor_col,
            )
        };
        let cleaned = copymode_ext::clean_selection_text(&text);
        let kind = if self.copy_visual_line {
            copymode_ext::RegisterKind::Line
        } else {
            copymode_ext::RegisterKind::Char
        };
        self.copy_registers.yank(register, &cleaned, kind);
        self.selection = None;
        self.copy_visual = false;
        self.mouse_selecting = false;
        self.clipboard = cleaned.clone();
        if !cleaned.is_empty() {
            Self::emit_osc52(&cleaned);
        }
        let count = cleaned.chars().count();
        self.notify(format!("yanked {count} char(s)"), "info");
    }

    /// Execute the search using the consolidated search state, storing all
    /// matches for highlighting and navigation.
    pub fn copy_execute_search_state(&mut self) {
        if self.copy_search_query.is_empty() {
            self.copy_search_typing = false;
            self.copy_search_state.clear();
            return;
        }
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        self.copy_search_state.query = self.copy_search_query.clone();
        self.copy_search_state.forward = self.copy_search_forward;
        let start_line = self.copy_cursor_line;
        let start_col = self.copy_cursor_col as usize;
        let lines: Vec<String> = {
            let mut lines = Vec::with_capacity(count);
            for i in 0..count {
                lines.push(self.copy_line_text(i));
            }
            lines
        };
        self.copy_search_state
            .execute(count, |line: usize| lines.get(line).cloned().unwrap_or_default());
        self.copy_search_typing = false;
        if let Some(m) = self.copy_search_state.jump_initial(start_line, start_col) {
            self.copy_cursor_line = m.line;
            self.copy_cursor_col = m.start as i32;
            self.scroll_to_cursor(m.line);
            self.sync_selection_cursor();
        }
    }

    /// Jump to the next match in the consolidated search state.
    pub fn copy_search_state_next(&mut self) {
        let m = self.copy_search_state.next();
        if let Some(m) = m {
            self.copy_cursor_line = m.line;
            self.copy_cursor_col = m.start as i32;
            self.scroll_to_cursor(m.line);
            self.sync_selection_cursor();
        }
    }

    /// Jump to the previous match in the consolidated search state.
    pub fn copy_search_state_prev(&mut self) {
        let m = self.copy_search_state.prev();
        if let Some(m) = m {
            self.copy_cursor_line = m.line;
            self.copy_cursor_col = m.start as i32;
            self.scroll_to_cursor(m.line);
            self.sync_selection_cursor();
        }
    }

    /// Clear search highlighting (vim's `:noh` / Ctrl+L).
    pub fn copy_clear_search(&mut self) {
        self.copy_search_state.clear();
        self.copy_search_query.clear();
    }

    /// The content position (line, column) under a screen cell coordinate for
    /// a window.
    pub fn content_position_at(
        &self,
        window: usize,
        column: i32,
        row: i32,
    ) -> Option<(usize, i32)> {
        let rect = if self.is_float(window) {
            self.float_rect(window)
        } else {
            self.current_layout().get(&(window as i32)).copied()
        }?;
        // The pane border consumes the outer ring; content starts one cell in.
        let rel_row = (row - rect.y - 1).max(0);
        let rel_col = (column - rect.x - 1).max(0);
        let w = self.windows.get(window)?;
        let emu = w.emulator.lock().ok()?;
        let line = emu.content_index_for_view_row(rel_row);
        // Terminal columns span wide glyphs, but selection coordinates are
        // emulator columns (leads only). A click on a wide char's second
        // column snaps to the glyph's lead so the char is selected whole;
        // clicks beyond the line's content keep their raw column.
        let mut snapped = rel_col;
        let mut col = 0i32;
        for cell in &emu.content_line(line) {
            let cell_w = cell.width.max(1) as i32;
            if rel_col < col + cell_w {
                snapped = col;
                break;
            }
            col += cell_w;
        }
        Some((line, snapped))
    }

    // -- Mouse selection ----------------------------------------------------

    pub fn begin_mouse_selection(&mut self, window: usize, column: i32, row: i32) {
        let Some((line, col)) = self.content_position_at(window, column, row) else {
            return;
        };
        self.mouse_selecting = true;
        self.copy_visual = false;
        self.selection = Some(Selection {
            window,
            anchor_line: line,
            anchor_col: col,
            cursor_line: line,
            cursor_col: col,
        });
    }

    pub fn extend_mouse_selection(&mut self, window: usize, column: i32, row: i32) {
        if !self.mouse_selecting {
            return;
        }
        let Some((line, col)) = self.content_position_at(window, column, row) else {
            return;
        };
        if let Some(sel) = &mut self.selection {
            if sel.window == window {
                sel.cursor_line = line;
                sel.cursor_col = col;
            }
        }
    }

    pub fn end_mouse_selection(&mut self) {
        self.mouse_selecting = false;
        let empty = self
            .selection
            .as_ref()
            .map(|s| s.is_empty())
            .unwrap_or(true);
        if empty {
            self.selection = None;
        } else {
            self.yank_selection();
        }
    }

    /// The window index under a cell coordinate (column, row), if any.
    pub fn window_at(&self, column: i32, row: i32) -> Option<usize> {
        let layout = self.current_layout();
        self.window_at_with_layout(column, row, &layout)
    }

    /// Like `window_at` but accepts a precomputed layout to avoid recomputing
    /// the BSP tree on every call.
    pub fn window_at_with_layout(
        &self,
        column: i32,
        row: i32,
        layout: &HashMap<i32, Rect>,
    ) -> Option<usize> {
        // Floating panes sit above the tiled layout: the topmost float wins.
        if let Some(idx) = self.float_at(column, row) {
            return Some(idx);
        }
        let mut best: Option<(usize, i32)> = None;
        for (&window_id, &rect) in layout {
            if column >= rect.x
                && column < rect.x + rect.w
                && row >= rect.y
                && row < rect.y + rect.h
            {
                // Topmost is approximated by largest area here (single layer).
                let area = rect.w * rect.h;
                if best.map(|(_, a)| area > a).unwrap_or(true) {
                    best = Some((window_id as usize, area));
                }
            }
        }
        best.map(|(idx, _)| idx)
    }

    /// Hit-test the dock bar: returns the window index if the click lands
    /// on a dock pill, or `None` otherwise.  Computes layout on demand.
    pub fn dock_item_at(&self, column: i32, row: i32) -> Option<usize> {
        let dock_position = self.config.appearance.dockbar_position.as_str();
        if dock_position == "hidden" {
            return None;
        }
        let dock_row = if dock_position == "top" {
            0
        } else {
            self.height - 1
        };
        if row != dock_row {
            return None;
        }
        let layout = crate::app::dock::calculate_dock_layout(self);
        for (i, item) in layout.visible_items.iter().enumerate() {
            let pill_x = layout.item_positions.get(i).copied().unwrap_or(0);
            let pill_w = item.width as i32;
            if column >= pill_x && column < pill_x + pill_w {
                return Some(item.window_index as usize);
            }
        }
        None
    }

    /// Detect if a screen coordinate is on a pane border (within 1 cell slop).
    /// Returns (window_id, edge) if the click is on a border between panes.
    /// Precomputes edge coordinate sets so neighbor checks are O(1) instead
    /// of O(n), making the overall function O(n) instead of O(n²).
    pub fn border_at_with_layout(
        &self,
        column: i32,
        row: i32,
        layout: &HashMap<i32, Rect>,
    ) -> Option<(i32, crate::layout::ResizeEdge)> {
        let slop = 1;
        // Precompute sets of all x and y coordinates that appear as rect
        // left edges or right edges, so neighbor checks are O(1).
        let mut left_edges: std::collections::HashSet<i32> = std::collections::HashSet::new();
        let mut right_edges: std::collections::HashSet<i32> = std::collections::HashSet::new();
        let mut top_edges: std::collections::HashSet<i32> = std::collections::HashSet::new();
        let mut bottom_edges: std::collections::HashSet<i32> = std::collections::HashSet::new();
        for rect in layout.values() {
            left_edges.insert(rect.x);
            right_edges.insert(rect.x + rect.w);
            top_edges.insert(rect.y);
            bottom_edges.insert(rect.y + rect.h);
        }
        for (&wid, &rect) in layout {
            // Right border: column is at rect.x + rect.w or rect.x + rect.w - 1
            if (column == rect.x + rect.w || column == rect.x + rect.w - 1)
                && row >= rect.y.saturating_sub(slop)
                && row < rect.y + rect.h + slop
                && left_edges.contains(&(rect.x + rect.w))
            {
                return Some((wid, crate::layout::ResizeEdge::Right));
            }
            // Bottom border
            if (row == rect.y + rect.h || row == rect.y + rect.h - 1)
                && column >= rect.x.saturating_sub(slop)
                && column < rect.x + rect.w + slop
                && top_edges.contains(&(rect.y + rect.h))
            {
                return Some((wid, crate::layout::ResizeEdge::Bottom));
            }
            // Left border
            if (column == rect.x || column == rect.x + 1)
                && row >= rect.y.saturating_sub(slop)
                && row < rect.y + rect.h + slop
                && right_edges.contains(&rect.x)
            {
                return Some((wid, crate::layout::ResizeEdge::Left));
            }
            // Top border
            if (row == rect.y || row == rect.y + 1)
                && column >= rect.x.saturating_sub(slop)
                && column < rect.x + rect.w + slop
                && bottom_edges.contains(&rect.y)
            {
                return Some((wid, crate::layout::ResizeEdge::Top));
            }
        }
        None
    }

    /// Begin a border-drag resize operation.
    pub fn begin_border_drag(&mut self, window_id: i32, edge: crate::layout::ResizeEdge, pos: i32) {
        self.drag_resize = Some((window_id, edge, pos));
    }

    /// Apply a border-drag resize to the current position.
    pub fn apply_border_drag(&mut self, pos: i32) {
        let Some((wid, edge, _start)) = self.drag_resize else {
            return;
        };
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        tree.resize_split(wid, edge, pos, bounds, gap);
    }

    /// End the border-drag resize.
    pub fn end_border_drag(&mut self) {
        self.drag_resize = None;
    }

    /// Select a word at the given screen position and return the selection text.
    pub fn select_word_at(&mut self, window: usize, column: i32, row: i32) {
        let Some((line, col)) = self.content_position_at(window, column, row) else {
            return;
        };
        let (text, cells) = {
            let Some(w) = self.windows.get(window) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            (emu.content_line_text(line), emu.content_line(line))
        };
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return;
        }
        let pos = (col as usize).min(chars.len().saturating_sub(1));
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !is_word(chars[pos]) {
            // If not on a word char, select the single char.
            self.selection = Some(Selection {
                window,
                anchor_line: line,
                anchor_col: col,
                cursor_line: line,
                cursor_col: col + 1,
            });
            return;
        }
        // Find word boundaries in text-char space.
        let mut start = pos;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = pos;
        while end + 1 < chars.len() && is_word(chars[end + 1]) {
            end += 1;
        }
        let end_excl = end + 1;
        // Convert the char range to column space: wide runes occupy two
        // columns per char, so the selection must span their full width.
        let mut char_idx = 0usize;
        let mut col_idx = 0i32;
        let mut start_col = 0i32;
        let mut end_col = col;
        for cell in &cells {
            if cell.width == 0 {
                continue;
            }
            if char_idx == start {
                start_col = col_idx;
            }
            if char_idx == end_excl {
                end_col = col_idx;
                break;
            }
            char_idx += 1;
            col_idx += cell.width.max(1) as i32;
        }
        if char_idx == end_excl {
            end_col = col_idx;
        }
        // end_col is exclusive; selection_text treats the end column as
        // inclusive, so back off by one (saturating for the empty case).
        let end_col_incl = end_col.saturating_sub(1);
        self.selection = Some(Selection {
            window,
            anchor_line: line,
            anchor_col: start_col,
            cursor_line: line,
            cursor_col: end_col_incl,
        });
    }

    /// Select the entire line at the given screen position.
    pub fn select_line_at(&mut self, window: usize, column: i32, row: i32) {
        let Some((line, _)) = self.content_position_at(window, column, row) else {
            return;
        };
        let width = {
            let Some(w) = self.windows.get(window) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.width()
        };
        self.selection = Some(Selection {
            window,
            anchor_line: line,
            anchor_col: 0,
            cursor_line: line,
            // Inclusive end column: width - 1 covers the entire line.
            cursor_col: (width - 1).max(0),
        });
    }

    /// Open the settings overlay.
    pub fn open_settings(&mut self) {
        self.settings_open = true;
        self.settings_selected = 0;
    }

    /// Close the settings overlay.
    pub fn close_settings(&mut self) {
        self.settings_open = false;
        self.settings_selected = 0;
    }

    /// The settings rows with their current values.
    pub fn settings_rows(&self) -> Vec<(String, String)> {
        let theme = self
            .theme
            .as_ref()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| self.config.appearance.theme.clone());
        let mut rows = vec![
            ("Theme".to_string(), theme),
            (
                "Animations".to_string(),
                if self.config.appearance.animations_enabled {
                    "on".into()
                } else {
                    "off".into()
                },
            ),
            (
                "Which-key overlay".to_string(),
                if self.config.appearance.which_key_enabled {
                    "on".into()
                } else {
                    "off".into()
                },
            ),
            (
                "Pane gap".to_string(),
                if self.gap > 0 {
                    self.gap.to_string()
                } else {
                    "off".into()
                },
            ),
        ];
        rows.push((
            "Scroll lines".to_string(),
            self.config.appearance.scroll_lines.to_string(),
        ));
        rows
    }

    /// Adjust the selected settings row: `delta` -1/0/+1 (left/enter/right).
    pub fn adjust_settings_row(&mut self, delta: i32) {
        let theme_names = crate::config::theme::Theme::built_in_names();
        let row = self.settings_selected;
        match row {
            0 => {
                // Cycle the theme.
                let current = self
                    .theme
                    .as_ref()
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| self.config.appearance.theme.clone());
                let idx = theme_names.iter().position(|n| *n == current).unwrap_or(0);
                let next = (idx as i32 + delta).rem_euclid(theme_names.len() as i32) as usize;
                let name = theme_names[next].to_string();
                self.theme = crate::config::Theme::built_in(&name);
                self.config.appearance.theme = name.clone();
                self.notify(format!("theme: {name}"), "info");
                self.log_action(&format!("set_theme {name}"));
            }
            1 => {
                if delta != 0 {
                    self.config.appearance.animations_enabled =
                        !self.config.appearance.animations_enabled;
                }
            }
            2 => {
                if delta != 0 {
                    self.config.appearance.which_key_enabled =
                        !self.config.appearance.which_key_enabled;
                }
            }
            3 => {
                self.gap = (self.gap + delta).max(0);
            }
            4 => {
                self.config.appearance.scroll_lines =
                    (self.config.appearance.scroll_lines + delta).max(1);
            }
            _ => {}
        }
    }

    pub fn open_theme_picker(&mut self) {
        self.theme_list = crate::config::theme::all_themes()
            .into_iter()
            .map(|t| t.name)
            .collect();
        self.theme_picker_open = true;
        self.theme_picker_selected = 0;
    }

    pub fn close_theme_picker(&mut self) {
        self.theme_picker_open = false;
    }

    /// Re-run host-terminal light/dark detection and swap the theme in place.
    ///
    /// Only acts when `theme = "auto"` is configured, and should be called
    /// with the terminal in raw mode so the OSC 11 reply is delivered. Leaves
    /// `config.appearance.theme` as `"auto"` so the next launch re-detects.
    pub fn redetect_theme(&mut self) {
        if !self.auto_theme {
            return;
        }
        let mode = crate::util::theme_detect::detect_terminal_mode();
        let name = crate::util::theme_detect::resolve_auto_theme_name(
            mode,
            &self.config.appearance.theme_auto_light,
            &self.config.appearance.theme_auto_dark,
        );
        let applied = Theme::built_in(&name);
        let unchanged = matches!(
            (&self.theme, &applied),
            (Some(cur), Some(next)) if cur.name == next.name
        );
        if unchanged {
            return;
        }
        let mode_name = match mode {
            Some(crate::util::theme_detect::ThemeMode::Light) => "light",
            _ => "dark",
        };
        self.theme = applied;
        self.damage_full(crate::app::damage::DamageReason::Theme);
        self.notify(format!("theme: {name} ({mode_name} detected)"), "info");
        self.log_action(&format!("theme_detect {name}"));
    }

    pub fn theme_picker_move(&mut self, delta: i32) {
        if self.theme_list.is_empty() {
            return;
        }
        let len = self.theme_list.len() as i32;
        let new = (self.theme_picker_selected as i32 + delta).rem_euclid(len) as usize;
        self.theme_picker_selected = new;
    }

    pub fn apply_selected_theme(&mut self) {
        if let Some(name) = self.theme_list.get(self.theme_picker_selected).cloned() {
            self.config.appearance.theme = name;
            self.close_theme_picker();
        }
    }

    // --- Accent picker ---

    pub fn open_accent_picker(&mut self) {
        self.accent_picker_open = true;
        self.accent_picker_selected = 0;
    }

    pub fn close_accent_picker(&mut self) {
        self.accent_picker_open = false;
    }

    pub fn accent_picker_move(&mut self, delta: i32) {
        if self.accent_list.is_empty() {
            return;
        }
        let len = self.accent_list.len() as i32;
        let new = (self.accent_picker_selected as i32 + delta).rem_euclid(len) as usize;
        self.accent_picker_selected = new;
    }

    pub fn apply_selected_accent(&mut self) {
        if let Some(name) = self.accent_list.get(self.accent_picker_selected).cloned() {
            // Store the accent color as the border_focused_color.
            self.config.appearance.border_focused_color = Some(name);
            self.close_accent_picker();
        }
    }

    /// Toggle the help modal overlay.
    pub fn toggle_help(&mut self) {
        self.help_open = !self.help_open;
    }

    /// Toggle the persistent key-hints bar.
    pub fn toggle_hints(&mut self) {
        self.hints_visible = !self.hints_visible;
    }

    /// Dismiss the welcome overlay.
    pub fn dismiss_welcome(&mut self) {
        self.show_welcome = false;

}
}
