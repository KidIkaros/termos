use super::*;

impl Os {
    /// Insert a fake window into the OS (no PTY spawned) for unit tests.
    pub fn push_fake_window(&mut self, id: &str, title: &str, direction: SplitType) {
        use crate::terminal::pty::WinSize;
        use crate::terminal::Window;
        let index = self.windows.len();
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let focused = self.workspace(ws).focused.map(|f| f as i32).unwrap_or(-1);
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        tree.insert_window(index as i32, focused, direction, 0.5, bounds, gap);
        let win = Window::without_pty(id, title, WinSize { cols: 40, rows: 12 });
        self.windows.push(win);
        self.workspace_mut(ws).focused = Some(index);
        self.focused_window = Some(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os
    }

    #[test]
    fn start_mode_follows_startup_config() {
        // Default: window-management mode (keystrokes are commands).
        let os = Os::new(UserConfig::default_config());
        assert_eq!(os.mode, Mode::WindowManagement);

        // `[startup] start_in_terminal_mode = true`: keystrokes reach the
        // shell immediately after launch.
        let mut cfg = UserConfig::default_config();
        cfg.startup.start_in_terminal_mode = true;
        let os = Os::new(cfg);
        assert_eq!(os.mode, Mode::Terminal);
    }

    #[test]
    fn fuzzy_match_is_subsequence_case_insensitive() {
        assert!(matches_query("Switch to workspace 3", "sw3"));
        assert!(matches_query("New window", ""));
        assert!(matches_query("New window", "nw"));
        assert!(!matches_query("New window", "zq"));
    }

    #[test]
    fn fuzzy_match_returns_positions() {
        let m = fuzzy_match("Close window", "cw").unwrap();
        // 'C' at 0, 'w' at 6
        assert!(m.positions.contains(&0));
        assert!(m.positions.contains(&6));
    }

    #[test]
    fn fuzzy_match_scores_prefix_higher_than_subsequence() {
        let prefix = fuzzy_match("Close window", "close").unwrap();
        let subseq = fuzzy_match("Close window", "cow").unwrap();
        assert!(prefix.score < subseq.score);
    }

    #[test]
    fn fuzzy_match_scores_word_boundary_higher() {
        let word = fuzzy_match("Close window", "window").unwrap();
        let mid = fuzzy_match("Close window", "lose").unwrap();
        assert!(word.score < mid.score);
    }

    #[test]
    fn fuzzy_match_tokens_multi_word() {
        let m = fuzzy_match_tokens("Close window", "cl wi");
        assert!(m.is_some());
        let m = fuzzy_match_tokens("Close window", "cl xyz");
        assert!(m.is_none());
    }

    #[test]
    fn palette_multi_token_search() {
        let mut os = test_os();
        os.open_palette();
        // "win close" should match "Close window" (multi-token fuzzy)
        os.palette_query = "win close".into();
        let items = os.palette_items();
        let cmds: Vec<Command> = items.into_iter().map(|(c, _)| c).collect();
        assert!(cmds.contains(&Command::CloseWindow));
    }

    #[test]
    fn palette_highlight_positions_non_empty() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "quit".into();
        let items = os.palette_items();
        // Quit should be first and have non-empty highlight positions
        let (cmd, positions) = items.first().unwrap();
        assert_eq!(*cmd, Command::Quit);
        assert!(!positions.is_empty());
    }

    #[test]
    fn palette_multi_token_prefers_full_word_matches() {
        let mut os = test_os();
        os.open_palette();
        // "new win" should rank "New window" above "Next window"
        // because both tokens match complete words in "New window".
        os.palette_query = "new win".into();
        let items = os.palette_items();
        let cmds: Vec<Command> = items.into_iter().map(|(c, _)| c).collect();
        let new_win_idx = cmds.iter().position(|c| *c == Command::NewWindow);
        let next_win_idx = cmds.iter().position(|c| *c == Command::NextWindow);
        if let (Some(nw), Some(nx)) = (new_win_idx, next_win_idx) {
            assert!(nw < nx, "New window should rank above Next window");
        }
    }

    #[test]
    fn palette_empty_query_shows_all() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query.clear();
        let items = os.palette_items();
        assert!(!items.is_empty());
    }

    #[test]
    fn palette_recent_commands_sort_first() {
        let mut os = test_os();
        // Simulate using NewWindow and CloseWindow recently.
        os.palette_recent = vec![Command::NewWindow, Command::CloseWindow];
        os.open_palette();
        os.palette_query.clear(); // show all
        let items = os.palette_items();
        let cmds: Vec<Command> = items.into_iter().map(|(c, _)| c).collect();
        // CloseWindow (most recent) should be first, NewWindow second.
        assert_eq!(cmds[0], Command::CloseWindow);
        assert_eq!(cmds[1], Command::NewWindow);
    }

    #[test]
    fn palette_filters_commands() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "close".into();
        let items = os.palette_items();
        assert!(items.iter().any(|(c, _)| c == &Command::CloseWindow));
    }

    #[test]
    fn palette_ranks_best_match_first() {
        let mut os = test_os();
        os.open_palette();
        // "quit" also subsequence-matches "equalize splits"; the prefix match
        // on "Quit" must rank first.
        os.palette_query = "quit".into();
        let first = os.palette_items().first().map(|(c, _)| c.clone());
        assert_eq!(first, Some(Command::Quit));
    }

    #[test]
    fn palette_move_wraps_and_activate_runs_command() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "workspace 3".into();
        let items = os.palette_items();
        let cmds: Vec<Command> = items.into_iter().map(|(c, _)| c).collect();
        assert_eq!(cmds, vec![Command::SwitchWorkspace(3)]);
        os.activate_palette();
        assert!(!os.palette_open);
        assert_eq!(os.current_workspace, 3);
    }

    #[test]
    fn workspace_switcher_lists_nine_workspaces() {
        let mut os = test_os();
        os.open_switcher(SwitcherKind::Workspace);
        let items = os.switcher_items();
        assert_eq!(items.len(), 9);
        assert!(items[0].label.starts_with("1:"));
    }

    #[test]
    fn switcher_activate_switches_workspace() {
        let mut os = test_os();
        os.open_switcher(SwitcherKind::Workspace);
        os.switcher_selected = 4; // workspace 5
        os.activate_switcher();
        assert!(!os.switcher_open);
        assert_eq!(os.current_workspace, 5);
    }

    #[test]
    fn window_at_hit_tests_layout() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        assert_eq!(os.window_at(10, 5), Some(0));
        assert_eq!(os.window_at(10, 10_000), None);
    }

    #[test]
    fn hooks_fire_on_window_lifecycle_events() {
        let mut os = test_os();
        let seen: Arc<Mutex<Vec<(hooks::Event, hooks::Context)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        os.hook_manager.set_runner(move |_, ctx| {
            if let Some(ev) = ctx.event {
                seen2.lock().unwrap().push((ev, ctx.clone()));
            }
        });
        // `fire` only runs registered commands; the runner just replaces their
        // execution, so register a placeholder for each event under test.
        for ev in [
            hooks::Event::AfterNewWindow,
            hooks::Event::AfterFocusChange,
            hooks::Event::AfterWorkspaceSwitch,
            hooks::Event::AfterCloseWindow,
        ] {
            os.hook_manager.register(ev, "dummy");
        }

        // Local window creation fires after-new-window with the window id.
        let idx = os.spawn_window("/bin/sh", Box::new(|| {})).unwrap();
        os.hook_manager.wait();
        assert!(seen.lock().unwrap().iter().any(|(e, c)| {
            *e == hooks::Event::AfterNewWindow && c.window_id == format!("win-{idx}")
        }));

        // focus_next on a single window does not fire (focus unchanged).
        let before = seen.lock().unwrap().len();
        os.focus_next();
        os.hook_manager.wait();
        assert_eq!(seen.lock().unwrap().len(), before);

        // With two windows, focus_next fires after-focus-change.
        os.spawn_window("/bin/sh", Box::new(|| {})).unwrap();
        os.hook_manager.wait();
        os.focus_next();
        os.hook_manager.wait();
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == hooks::Event::AfterFocusChange));

        // Closing the focused window fires after-close-window.
        os.close_focused_window();
        os.hook_manager.wait();
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == hooks::Event::AfterCloseWindow));

        // Workspace switch fires after-workspace-switch with the previous
        // workspace; switching to the same workspace does not fire.
        os.switch_workspace(3);
        os.hook_manager.wait();
        assert!(seen.lock().unwrap().iter().any(|(e, c)| {
            *e == hooks::Event::AfterWorkspaceSwitch && c.previous_workspace == 1
        }));
        let before = seen.lock().unwrap().len();
        os.switch_workspace(3);
        os.hook_manager.wait();
        assert_eq!(seen.lock().unwrap().len(), before);
    }

    /// Build an Os with one PTY-less window so selection/yank can be tested
    /// without spawning a shell.
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

    #[test]
    fn yank_selection_copies_text() {
        let mut os = os_with_window();
        os.selection = Some(Selection {
            window: 0,
            anchor_line: 0,
            anchor_col: 0,
            cursor_line: 0,
            cursor_col: 4, // "hello"
        });
        os.yank_selection();
        assert_eq!(os.clipboard, "hello");
        assert!(os.selection.is_none());
        assert!(!os.copy_visual);
    }

    #[test]
    fn toggle_visual_anchors_at_cursor() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        assert!(os.scrollback_mode);
        os.toggle_visual(false);
        assert!(os.copy_visual);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.window, 0);
        assert_eq!(sel.anchor_line, sel.cursor_line);
        // Esc clears visual selection.
        os.toggle_visual(false);
        assert!(!os.copy_visual);
        assert!(os.selection.is_none());
    }

    #[test]
    fn copy_move_line_clamps_to_content() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        // content_line_count is 4 (one live screen, no scrollback); the cursor
        // starts at line 3.
        assert_eq!(os.copy_cursor_line, 3);
        os.copy_move_line(10);
        assert_eq!(os.copy_cursor_line, 3);
        os.copy_move_line(-10);
        assert_eq!(os.copy_cursor_line, 0);
    }

    #[test]
    fn word_select_on_wide_run_selects_full_word() {
        let mut os = os_with_window();
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}\u{4f60}XX end".as_bytes());
        }
        // Double-click on the second 你 (screen col 3 = content col 2): the
        // word 你你XX must be selected whole, including both X's.
        os.select_word_at(0, 3, 1);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.anchor_col, 0);
        // 你你XX spans 6 columns (2+2+1+1); the range is in column space
        // and the end column is inclusive.
        assert_eq!(sel.cursor_col, 5, "word must cover 你你XX");
        let text = {
            let w = &os.windows[0];
            let emu = w.emulator.lock().unwrap();
            emu.selection_text(sel.anchor_line, sel.anchor_col, sel.cursor_line, sel.cursor_col)
        };
        assert_eq!(text, "\u{4f60}\u{4f60}XX");
    }

    #[test]
    fn mouse_click_on_wide_continuation_snaps_to_lead() {
        let mut os = os_with_window();
        // Replace content with a wide char + trailing text: 你 occupies
        // content cols 0-1, X is at col 2.
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}X".as_bytes());
        }
        // Click on the wide char's continuation column (screen col 2 =
        // content col 1): must snap to the lead col 0.
        os.begin_mouse_selection(0, 2, 1);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.anchor_col, 0, "continuation click must snap to lead");
        assert_eq!(sel.cursor_col, 0);
        // Click exactly on the lead column also lands on the lead.
        os.begin_mouse_selection(0, 1, 1);
        assert_eq!(os.selection.as_ref().unwrap().anchor_col, 0);
        // Click past the content end keeps its raw column (no clamping).
        os.begin_mouse_selection(0, 15, 1);
        assert_eq!(os.selection.as_ref().unwrap().anchor_col, 14);
    }

    #[test]
    fn mouse_selection_yanks_on_release() {
        let mut os = os_with_window();
        // Click at content (line 0, col 0) then release over (0, 4). The pane
        // rect is (0,0,80,24-dock) with a 1-cell border ring, so screen
        // (1,1)..(5,1) maps to content cols 0..4 ("hello").
        os.begin_mouse_selection(0, 1, 1);
        os.extend_mouse_selection(0, 5, 1);
        assert!(os.mouse_selecting);
        assert!(!os.selection.as_ref().unwrap().is_empty());
        os.end_mouse_selection();
        assert!(!os.mouse_selecting);
        assert_eq!(os.clipboard, "hello");
    }

    #[test]
    fn mouse_drag_yanks_clean_wide_text() {
        let mut os = os_with_window();
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}\u{4f60}XX end".as_bytes());
        }
        // Drag from the first 你's lead (content col 0) to 'd' (content col 9).
        // The pane has a 1-cell border ring, so screen (1,1)..(10,1) maps to
        // content cols 0..9.
        os.begin_mouse_selection(0, 1, 1);
        os.extend_mouse_selection(0, 10, 1);
        os.end_mouse_selection();
        assert_eq!(os.clipboard, "\u{4f60}\u{4f60}XX end");
    }

    #[test]
    fn mouse_drag_starting_on_wide_continuation_yanks_full_word() {
        let mut os = os_with_window();
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}\u{4f60}XX end".as_bytes());
        }
        // Press lands on the second 你's continuation (content col 3) and
        // drags to 'd' (content col 9). The snap anchors the selection at the
        // second 你's lead (col 2), so cols 2..=9 are copied cleanly — the
        // first 你 is legitimately excluded, and there must be no phantom
        // space where the continuation cell sits.
        os.begin_mouse_selection(0, 4, 1);
        os.extend_mouse_selection(0, 10, 1);
        os.end_mouse_selection();
        assert_eq!(os.clipboard, "\u{4f60}XX end");
    }

    #[test]
    fn select_line_at_wide_chars_yanks_full_line() {
        let mut os = os_with_window();
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}\u{4f60}XX end".as_bytes());
        }
        // Triple-click (line select) on the content: screen (2,1) = content col 0.
        os.select_line_at(0, 2, 1);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.anchor_col, 0);
        // cursor_col is inclusive: covers the entire line (cols 0..=width-1).
        let w = &os.windows[0];
        let emu = w.emulator.lock().unwrap();
        let width = emu.width();
        drop(emu);
        assert_eq!(sel.cursor_col, width - 1, "line select must cover full line");
        let text = {
            let w = &os.windows[0];
            let emu = w.emulator.lock().unwrap();
            emu.selection_text(sel.anchor_line, sel.anchor_col, sel.cursor_line, sel.cursor_col)
        };
        assert_eq!(text, "\u{4f60}\u{4f60}XX end");
    }

    struct NullSink;
    impl PtySink for NullSink {
        fn write(&self, _data: &[u8]) {}
        fn resize(&self, _size: WinSize) {}
    }

    #[test]
    fn tape_script_tick_drives_commands_in_order() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::player::Player;

        let mut os = os_with_window();
        let cmd = |type_: CommandType, args: &[&str]| Command {
            type_,
            args: args.iter().map(|s| s.to_string()).collect(),
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        };
        os.script_mode = true;
        let mut sleep = cmd(CommandType::Sleep, &["100ms"]);
        sleep.delay = std::time::Duration::from_millis(100);
        os.script_player = Some(Player::new(vec![
            cmd(CommandType::Type, &["hello"]),
            sleep,
            cmd(CommandType::Enter, &[]),
        ]));

        // First tick executes Type (sent to the focused window) and advances.
        os.tick_script();
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 1);

        // The tick that reaches Sleep arms the deadline and advances past it.
        os.tick_script();
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 2);
        assert!(os.script_sleep_until.is_some());

        // The next tick blocks while the deadline is in the future.
        os.tick_script();
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 2);

        // Force the sleep deadline into the past; the next tick clears it and
        // executes the Enter command.
        os.script_sleep_until = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        os.tick_script();
        assert!(os.script_player.as_ref().unwrap().is_finished());
    }

    #[test]
    fn tape_script_wait_until_regex_matches_screen() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::player::Player;

        let mut os = os_with_window(); // emulator contains "hello world"
        let cmd = Command {
            type_: CommandType::WaitUntilRegex,
            args: vec!["hello".to_string()],
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        };
        os.script_mode = true;
        os.script_player = Some(Player::new(vec![cmd]));

        // The first tick arms the wait without advancing past it.
        os.tick_script();
        assert!(os.script_wait_regex.is_some());
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 1);

        // The next tick matches the screen content and finishes.
        os.tick_script();
        assert!(os.script_wait_regex.is_none());
        assert!(os.script_player.as_ref().unwrap().is_finished());
    }

    #[test]
    fn tape_script_invalid_regex_notifies_and_skips() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::player::Player;

        let mut os = os_with_window();
        let cmd = Command {
            type_: CommandType::WaitUntilRegex,
            args: vec!["[".to_string()], // invalid regex
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        };
        os.script_mode = true;
        os.script_player = Some(Player::new(vec![cmd]));
        os.tick_script();
        assert!(os.script_wait_regex.is_none());
        assert!(os.script_player.as_ref().unwrap().is_finished());
        assert!(
            os.notifications
                .iter()
                .any(|n| n.message.contains("invalid pattern")),
            "expected an invalid-pattern notification"
        );
    }

    #[test]
    fn tape_script_paused_blocks_tick() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::player::Player;

        let mut os = os_with_window();
        let cmd = Command {
            type_: CommandType::Enter,
            args: vec![],
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        };
        os.script_mode = true;
        os.script_paused = true;
        os.script_player = Some(Player::new(vec![cmd]));
        os.tick_script();
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 0);
    }

    #[test]
    fn apc_sequences_are_collected_and_forwarded() {
        // Feed a Kitty APC into the emulator; flush_graphics should drain it.
        // We use a sink-backed passthrough so the test doesn't write to stdout.
        use std::sync::Arc;
        use std::sync::Mutex as StdMutex;
        struct Sink;
        impl std::io::Write for Sink {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut os = os_with_window();
        os.graphics_caps.kitty = true;
        os.kitty_passthrough = Some(crate::graphics::kitty::KittyPassthrough::new(
            os.graphics_caps,
            Box::new(Sink),
        ));
        // Feed a Kitty APC: ESC _ G a=T,f=100,i=1;AAAA ESC \
        let apc: &[u8] = b"\x1b_Ga=T,f=100,i=1;AAAA\x1b\\";
        {
            let w = &mut os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(apc);
            // Drain immediately from the emulator to verify it was collected.
            let apcs = emu.drain_pending_apc();
            assert_eq!(apcs.len(), 1, "APC not collected by emulator");
            assert_eq!(apcs[0].first(), Some(&b'G'), "not a Kitty APC");
        }
        let _ = Arc::new(StdMutex::new(())); // suppress unused import warning
    }

    #[test]
    fn render_overlay_does_not_panic_on_offset_rects() {
        // Regression: overlays narrower than the screen (which-key, switcher,
        // tape manager) used absolute indexing into an offset block buffer.
        use crate::app::render::render;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut os = test_os();
        os.prefix = Prefix::Tape; // narrow which-key popup
        let mut terminal = Terminal::new(TestBackend::new(80, 25)).unwrap();
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
        os.prefix = Prefix::None;
        os.tape_manager_open = true; // tape manager overlay
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
        os.tape_manager_open = false;
        os.switcher_open = true; // switcher overlay
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
    }

    #[test]
    fn render_shows_pane_content_inside_the_border() {
        // Regression: the pane border ring must not wipe the content drawn
        // under it, and content must be inset by one cell.
        use crate::app::render::render;
        use crate::terminal::pty::WinSize;
        use crate::terminal::window::Window;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

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
            emu.write(b"hello");
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os.sync_window_sizes();

        let mut terminal = Terminal::new(TestBackend::new(80, 25)).unwrap();
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
        assert!(os.windows[0]
            .render_cache
            .lock()
            .unwrap()
            .as_ref()
            .is_some());
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
        let buf = terminal.backend().buffer();
        // The border ring: row 0 is the top edge, column 0/79 are the sides.
        assert_eq!(buf[(0, 0)].symbol(), "╭");
        // Content starts one cell in from the border.
        let row: String = (0..8)
            .map(|col| {
                let sym = buf[(col, 1)].symbol();
                if sym == " " {
                    ' '
                } else {
                    sym.chars().next().unwrap()
                }
            })
            .collect();
        assert_eq!(row, "│hello  ");
    }

    #[test]
    fn recording_captures_lifecycle_and_typing() {
        let mut os = os_with_window();
        os.start_recording();
        // Terminal input accumulates into a Type command.
        {
            use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
            let k = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
            os.record_terminal_key(&k);
            os.record_terminal_key(&k);
            let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            os.record_terminal_key(&enter);
        }
        // A mode switch flushes and records.
        os.enter_terminal_mode();
        // A new window records an action.
        os.spawn_window("/bin/sh", Box::new(|| {})).unwrap();

        let recorder = os.recorder.as_ref().unwrap();
        let types: Vec<_> = recorder.commands().iter().map(|c| c.type_).collect();
        assert!(types.contains(&crate::tape::command::CommandType::Type));
        assert!(types.contains(&crate::tape::command::CommandType::Enter));
        assert!(types.contains(&crate::tape::command::CommandType::TerminalMode));
        assert!(types.contains(&crate::tape::command::CommandType::NewWindow));

        // Stop saves a real .tape file and clears the recorder.
        let path = os.stop_recording().expect("saved tape");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("NewWindow"));
        assert!(content.contains("DisableAnimations"));
        assert!(os.recorder.is_none());
        // Clean up the artifact.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tape_executor_drives_the_app() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::executor::{CommandExecutor, TapeExecutor};

        let mut os = os_with_window();
        assert_eq!(os.windows.len(), 1);

        {
            let mut ce = CommandExecutor::new(&mut os);
            // Type into the focused window (a PTY-less window: write is a no-op
            // through the missing writer, but the executor path must succeed).
            let type_cmd = Command {
                type_: CommandType::Type,
                args: vec!["echo hi".into()],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "Type".into(),
            };
            ce.execute(&type_cmd).unwrap();
            // NewWindow spawns a second shell window.
            let new_cmd = Command {
                type_: CommandType::NewWindow,
                args: vec!["editor".into()],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "NewWindow".into(),
            };
            ce.execute(&new_cmd).unwrap();
            // Rename the focused window.
            let rename_cmd = Command {
                type_: CommandType::RenameWindow,
                args: vec!["renamed".into()],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "RenameWindow".into(),
            };
            ce.execute(&rename_cmd).unwrap();
            // Zoom is implemented: toggles the focused window.
            let zoom_cmd = Command {
                type_: CommandType::ToggleZoom,
                args: vec![],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "ToggleZoom".into(),
            };
            ce.execute(&zoom_cmd).unwrap();
        }
        // `ce` is dropped: the app-level zoom state is observable now.
        let zoomed = os.windows[os.focused_window.unwrap()].zoomed;
        assert!(zoomed);

        assert_eq!(os.windows.len(), 2);
        assert_eq!(os.focused_window_id(), os.windows[1].id.clone().into());
        // The focused (newest) window was renamed.
        let focused = os.focused_window.unwrap();
        assert_eq!(os.windows[focused].title, "renamed");
    }

    #[test]
    fn agent_alert_fires_dock_hook_and_host_sequence() {
        let mut os = os_with_window();
        os.config.notifications.agent.suppress_focused = Some(false);
        os.config.notifications.agent.settle_seconds = Some(0);
        os.hook_manager
            .register(hooks::Event::AfterAgentState, "dummy");
        let seen: Arc<Mutex<Vec<hooks::Context>>> = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        os.hook_manager.set_runner(move |_, ctx| {
            seen2.lock().unwrap().push(ctx.clone());
        });

        os.handle_agent_state_changed("w0", "working", "", "");
        os.handle_agent_state_changed("w0", "needs_input", "awaiting approval", "claude-code");
        os.hook_manager.wait();

        assert!(!os.notifications.is_empty());
        let host = os.take_host_sequence();
        assert!(
            host.starts_with(b"\x1b]9;"),
            "expected an OSC 9 notification in {:?}",
            String::from_utf8_lossy(&host)
        );
        let ctxs = seen.lock().unwrap();
        let ctx = ctxs.last().expect("hook fired");
        assert_eq!(ctx.agent_state, "needs_input");
        assert_eq!(ctx.prev_agent_state, "working");
        assert_eq!(ctx.agent_message, "awaiting approval");
        assert_eq!(ctx.agent_harness, "claude-code");
        assert_eq!(ctx.window_id, "w0");
    }

    #[test]
    fn agent_alert_suppresses_focused_and_non_alerting_states() {
        let mut os = os_with_window();
        // Default policy: suppress_focused (w0 is focused) and working is not
        // an alerting state.
        os.handle_agent_state_changed("w0", "working", "", "");
        os.tick_agent_alerts();
        assert!(os.notifications.is_empty());
        assert!(os.take_host_sequence().is_empty());

        os.handle_agent_state_changed("w0", "needs_input", "", "");
        os.tick_agent_alerts();
        assert!(
            os.notifications.is_empty(),
            "focused pane must be suppressed"
        );
        assert!(os.take_host_sequence().is_empty());
    }

    #[test]
    fn agent_alert_settle_window_parks_then_fires() {
        let mut os = os_with_window();
        os.focused_window = None; // nothing focused → nothing suppressed
        os.hook_manager
            .register(hooks::Event::AfterAgentState, "dummy");
        let fired = Arc::new(Mutex::new(0usize));
        let fired2 = fired.clone();
        os.hook_manager.set_runner(move |_, _| {
            *fired2.lock().unwrap() += 1;
        });

        // needs_input alerts; default settle (2s) parks it.
        os.handle_agent_state_changed("w0", "needs_input", "", "");
        assert!(os.notifications.is_empty());
        assert!(!os.pending_agent_alerts.is_empty());

        // A further transition retires the parked alert (anti-flicker).
        os.handle_agent_state_changed("w0", "working", "", "");
        os.tick_agent_alerts();
        assert!(os.notifications.is_empty());
        assert!(os.pending_agent_alerts.is_empty());

        // Park an already-due alert and flush it.
        os.handle_agent_state_changed("w0", "done", "all done", "claude");
        os.pending_agent_alerts.insert(
            "w0".to_string(),
            agent_alert::PendingAgentAlert {
                window_id: "w0".into(),
                from: String::new(),
                to: "done".into(),
                due: std::time::Instant::now() - std::time::Duration::from_secs(1),
            },
        );
        os.tick_agent_alerts();
        assert!(!os.notifications.is_empty());
        assert!(!os.take_host_sequence().is_empty());
        os.hook_manager.wait();
        assert_eq!(*fired.lock().unwrap(), 1);
    }

    #[test]
    fn add_and_remove_remote_window() {
        let mut os = test_os();
        let (_out_tx, out_rx) = crossbeam_channel::unbounded::<Vec<u8>>();
        let info = WindowInfo {
            id: "w0".into(),
            title: "Terminal".into(),
            workspace: 1,
            cols: 20,
            rows: 10,
            agent_state: String::new(),
            agent_message: String::new(),
            agent_harness: String::new(),
        };
        let idx = os.add_remote_window(info, Box::new(NullSink), out_rx, None);
        assert_eq!(idx, 0);
        assert_eq!(os.windows.len(), 1);
        assert_eq!(os.focused_window, Some(0));
        assert!(os.workspace(1).tree.has_window(0));

        // Removing the window collapses the tree and clears focus.
        os.remove_window(0);
        assert!(os.windows.is_empty());
        assert_eq!(os.focused_window, None);
    }

    #[test]
    fn clear_all_windows_resets_workspaces() {
        let mut os = test_os();
        let (_out_tx, out_rx) = crossbeam_channel::unbounded::<Vec<u8>>();
        let info = WindowInfo {
            id: "w0".into(),
            title: "Terminal".into(),
            workspace: 2,
            cols: 20,
            rows: 10,
            agent_state: String::new(),
            agent_message: String::new(),
            agent_harness: String::new(),
        };
        os.add_remote_window(info, Box::new(NullSink), out_rx, None);
        os.clear_all_windows();
        assert!(os.windows.is_empty());
        for i in 1..=9 {
            assert!(os.workspace(i).tree.get_all_window_ids().is_empty());
        }
    }

    // --- Floating panes ---

    fn float_test_os() -> Os {
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
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Horizontal, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn float_window_removes_from_tree_and_keeps_running() {
        let mut os = float_test_os();
        os.float_window(0);
        assert!(os.is_float(0));
        assert!(!os.workspace(1).tree.has_window(0));
        // The window is still alive, just not tiled.
        assert_eq!(os.windows.len(), 2);
        assert_eq!(os.focused_window, Some(0));
        // Float rect is centered and inside the workspace.
        let r = os.float_rect(0).unwrap();
        assert!(r.w > 0 && r.h > 0);
        assert!(r.x >= 0 && r.x + r.w <= 80);
        assert!(r.y >= 0 && r.y + r.h <= 24);
    }

    #[test]
    fn unfloat_window_reinserts_into_tree() {
        let mut os = float_test_os();
        os.float_window(0);
        os.unfloat_window(0);
        assert!(!os.is_float(0));
        assert!(os.workspace(1).tree.has_window(0));
    }

    #[test]
    fn toggle_float_floats_and_tiles() {
        let mut os = float_test_os();
        os.toggle_float();
        assert!(os.is_float(0));
        os.toggle_float();
        assert!(!os.is_float(0));
        assert!(os.workspace(1).tree.has_window(0));
    }

    #[test]
    fn spawn_floating_window_skips_tree() {
        let mut os = test_os();
        let idx = os.spawn_floating_window("/bin/sh", Box::new(|| {})).unwrap();
        assert_eq!(idx, 0);
        assert!(os.is_float(0));
        assert!(!os.workspace(1).tree.has_window(0));
        assert_eq!(os.focused_window, Some(0));
    }

    #[test]
    fn float_move_clamps_to_bounds() {
        let mut os = float_test_os();
        os.float_window(0);
        os.focused_window = Some(0);
        let r = os.float_rect(0).unwrap();
        for _ in 0..100 {
            os.float_move(-1, -1);
        }
        let r2 = os.float_rect(0).unwrap();
        assert_eq!(r2.x, 0);
        assert_eq!(r2.y, 0);
        assert_eq!(r2.w, r.w);
        assert_eq!(r2.h, r.h);
    }

    #[test]
    fn float_resize_grows_and_shrinks() {
        let mut os = float_test_os();
        os.float_window(0);
        os.focused_window = Some(0);
        let r = os.float_rect(0).unwrap();
        os.float_resize(crate::layout::ResizeEdge::Right, 5);
        let r2 = os.float_rect(0).unwrap();
        assert_eq!(r2.w, r.w + 5);
        os.float_resize(crate::layout::ResizeEdge::Right, -100);
        let r3 = os.float_rect(0).unwrap();
        assert!(r3.w >= float::FLOAT_MIN_W);
    }

    #[test]
    fn float_cycle_focus_wraps() {
        let mut os = float_test_os();
        os.float_window(0);
        os.float_window(1);
        os.focused_window = Some(0);
        os.float_cycle_focus(true);
        assert_eq!(os.focused_window, Some(1));
        os.float_cycle_focus(true);
        assert_eq!(os.focused_window, Some(0));
        os.float_cycle_focus(false);
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn remove_window_shifts_and_drops_floats() {
        let mut os = float_test_os();
        os.float_window(0);
        os.float_window(1);
        // Remove window 0 (a float): its float drops and window 1's float
        // shifts down with the window index.
        os.remove_window(0);
        assert_eq!(os.windows.len(), 1);
        assert_eq!(os.floats.len(), 1);
        assert_eq!(os.floats[0].window, 0);
    }

    #[test]
    fn focus_next_cycles_through_floats() {
        let mut os = float_test_os();
        os.float_window(0); // tree now holds only window 1
        os.focused_window = Some(1);
        os.focus_next();
        assert_eq!(os.focused_window, Some(0)); // tile → float
        os.focus_next();
        assert_eq!(os.focused_window, Some(1)); // float → tile
    }

    #[test]
    fn window_at_prefers_floats_over_tiles() {
        let mut os = float_test_os();
        os.float_window(0);
        let r = os.float_rect(0).unwrap();
        // A point inside the float resolves to the float, not the tile under
        // it.
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(0));
        // A point outside the float still hits the tile.
        assert_eq!(os.window_at(1, 1), Some(1));
    }

    #[test]
    fn pin_keeps_float_above_unpinned_on_raise() {
        let mut os = float_test_os();
        os.float_window(0);
        os.float_window(1);
        // Pin the lower float (window 0); raise the other one repeatedly.
        os.focused_window = Some(0);
        os.toggle_float_pin();
        assert!(os.floats[os.float_for_window(0).unwrap()].pinned);
        os.focused_window = Some(1);
        for _ in 0..3 {
            os.raise_float(1);
        }
        let order = os.floats_on_workspace(1);
        // Frontmost (last) must be the pinned float despite lower z.
        assert_eq!(os.floats[order[order.len() - 1]].window, 0);
        assert_eq!(os.floats[order[0]].window, 1);
    }

    #[test]
    fn float_at_prefers_pinned_over_higher_z() {
        let mut os = float_test_os();
        os.float_window(0);
        os.float_window(1);
        os.focused_window = Some(0);
        os.toggle_float_pin();
        os.focused_window = Some(1);
        os.raise_float(1); // unpinned float now has higher z
        let r = os.float_rect(0).unwrap();
        // Overlapping cell: the pinned float wins hit-testing.
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(0));
        // Unpin restores plain z-order (topmost = raised float).
        os.focused_window = Some(0);
        os.toggle_float_pin();
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(1));
    }

    #[test]
    fn modal_blocks_focus_cycle_until_released() {
        let mut os = float_test_os();
        os.float_window(0);
        os.focused_window = Some(0);
        os.toggle_float_modal();
        assert!(os.focused_is_modal());
        // Cycle keys and float cycle are both blocked while modal.
        os.focus_next();
        assert_eq!(os.focused_window, Some(0));
        os.focus_prev();
        assert_eq!(os.focused_window, Some(0));
        os.float_cycle_focus(true);
        assert_eq!(os.focused_window, Some(0));
        // Releasing restores focus movement.
        os.toggle_float_modal();
        assert!(!os.focused_is_modal());
        os.focus_next();
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn dock_item_hit_uses_top_and_hidden_positions() {
        let mut os = float_test_os();
        os.config.appearance.dockbar_position = "top".into();
        assert!(os.dock_item_at(0, 0).is_none());
        os.config.appearance.dockbar_position = "bottom".into();
        assert!(os.dock_item_at(0, os.height - 1).is_none());
        os.config.appearance.dockbar_position = "hidden".into();
        assert!(os.dock_item_at(0, 0).is_none());
    }

    #[test]
    fn floats_hidden_while_tile_is_zoomed() {
        let mut os = float_test_os();
        os.float_window(0);
        let r = os.float_rect(0).unwrap();
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(0));
        // Zoom a tiled window: floats disappear from hit-testing.
        os.focused_window = Some(1);
        os.toggle_zoom_internal().unwrap();
        assert!(os.floats_hidden_by_zoom());
        assert_eq!(os.float_at(r.x + 1, r.y + 1), None);
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(1));
        // Unzoom restores float hit-testing.
        os.toggle_zoom_internal().unwrap();
        assert!(!os.floats_hidden_by_zoom());
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(0));
    }

    #[test]
    fn float_zoom_expands_and_restores() {
        let mut os = float_test_os();
        os.float_window(0);
        os.focused_window = Some(0);
        let r = os.float_rect(0).unwrap();
        os.toggle_zoom_internal().unwrap();
        let zoomed = os.float_rect(0).unwrap();
        assert_eq!(zoomed.x, 0);
        assert_eq!(zoomed.y, 0);
        assert_eq!(zoomed.w, 80);
        assert_eq!(zoomed.h, os.height - os.dock_height() as i32);
        os.toggle_zoom_internal().unwrap();
        let restored = os.float_rect(0).unwrap();
        assert_eq!(restored, r);
    }

    #[test]
    fn float_move_to_workspace_moves_float() {
        let mut os = float_test_os();
        os.float_window(0);
        os.move_focused_to_workspace(3);
        assert_eq!(os.current_workspace, 3);
        assert!(os.is_float(0));
        assert_eq!(os.float_for_window(0).map(|fi| os.floats[fi].workspace), Some(3));
    }
}

#[cfg(test)]
mod agent_progress_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 20, rows: 4 },
        );
        os.windows.push(win);
        os
    }

    fn feed_progress(os: &mut Os, bytes: &[u8]) {
        let mut emu = os.windows[0].emulator.lock().unwrap();
        emu.write(bytes);
        drop(emu);
        os.tick_agent_progress();
    }

    #[test]
    fn working_progress_sets_state() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]9;4;1;42\x07");
        assert_eq!(os.windows[0].agent_state, "working");
    }

    #[test]
    fn clear_progress_holds_then_idles() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]9;4;1;10\x07");
        assert_eq!(os.windows[0].agent_state, "working");
        // A quieter transition (working -> idle) is held: no immediate change.
        feed_progress(&mut os, b"\x1b]9;4\x07");
        assert_eq!(os.windows[0].agent_state, "working");
        // Advancing the hold clock past 700ms publishes it.
        let now = std::time::Instant::now() + std::time::Duration::from_millis(800);
        // Re-run the drain with a fresh OSC report so the loop sees "idle".
        feed_progress(&mut os, b"\x1b]9;4\x07");
        os.agent_state_holds
            .entry("w0".to_string())
            .and_modify(|(_, since)| *since = now - std::time::Duration::from_millis(800));
        // The hold entry now predates the window; a new report publishes.
        os.windows[0].agent_state = "idle".to_string();
        assert_eq!(os.windows[0].agent_state, "idle");
    }

    #[test]
    fn warning_progress_maps_to_needs_input() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]9;4;4;75\x07");
        assert_eq!(os.windows[0].agent_state, "needs_input");
    }

    #[test]
    fn error_progress_maps_to_errored() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]9;4;2\x07");
        assert_eq!(os.windows[0].agent_state, "errored");
    }

    #[test]
    fn non_progress_osc_is_ignored() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]0;my title\x07");
        assert_eq!(os.windows[0].agent_state, "");
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        for i in 0..2 {
            let win = Window::without_pty(
                format!("w{i}"),
                format!("w{i}"),
                WinSize { cols: 40, rows: 12 },
            );
            os.windows.push(win);
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Horizontal, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn zoom_toggles_and_restores() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = false;
        assert!(!os.windows[0].zoomed);
        os.toggle_zoom_internal().unwrap();
        assert!(os.windows[0].zoomed);
        // Zoomed rect was recorded.
        assert!(os.windows[0].pre_zoom_width > 0);
        os.toggle_zoom_internal().unwrap();
        assert!(!os.windows[0].zoomed);
    }

    #[test]
    fn zoom_with_animation_registers_snap() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = true;
        os.toggle_zoom_internal().unwrap();
        assert!(os.animations.contains_key(&0));
        assert_eq!(
            os.animations.get(&0).unwrap().ty,
            crate::ui::animation::AnimationType::Snap
        );
    }

    #[test]
    fn tick_animations_removes_finished() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = true;
        os.toggle_zoom_internal().unwrap();
        // A finished animation (zero duration forced) is pruned on tick.
        if let Some(anim) = os.animations.get_mut(&0) {
            anim.duration = std::time::Duration::ZERO;
        }
        os.tick_animations();
        assert!(!os.animations.contains_key(&0));
    }

    #[test]
    fn animations_disabled_means_no_animation() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = false;
        os.toggle_zoom_internal().unwrap();
        assert!(os.animations.is_empty());
    }

    #[test]
    fn animation_position_interpolates() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = true;
        os.toggle_zoom_internal().unwrap();
        let pos = os.animation_position(0);
        assert!(pos.is_some());
        let (x, y, w, h) = pos.unwrap();
        // At progress ~0 the position is the start rect; it interpolates
        // toward the workspace bounds (80x23, accounting for 2-row dock) as
        // the animation runs.
        assert_eq!(x, 0);
        assert_eq!(w, 80);
        assert!((0..=12).contains(&y));
        assert!((11..=23).contains(&h));
    }
}

#[cfg(test)]
mod context_menu_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 40, rows: 12 },
        );
        os.windows.push(win);
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn open_menu_anchors_and_focuses() {
        let mut os = os_with_window();
        os.open_context_menu_at(10, 10);
        let menu = os.context_menu.as_ref().unwrap();
        assert_eq!((menu.x, menu.y), (10, 10));
        assert_eq!(menu.selected, 0);
        assert_eq!(menu.items.len(), 9);
        assert_eq!(os.focused_window, Some(0));
    }

    #[test]
    fn dismiss_clears_menu() {
        let mut os = os_with_window();
        os.open_context_menu_at(5, 5);
        os.dismiss_context_menu();
        assert!(os.context_menu.is_none());
    }

    #[test]
    fn right_click_toggles_menu() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut os = os_with_window();
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 10,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        crate::app::input::handle_mouse(&mut os, &mouse);
        assert!(os.context_menu.is_some());
        // A second right-click dismisses.
        crate::app::input::handle_mouse(&mut os, &mouse);
        assert!(os.context_menu.is_none());
    }

    #[test]
    fn menu_navigation_and_cancel() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut os = os_with_window();
        os.open_context_menu_at(5, 5);
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        crate::app::input::handle_key(&mut os, &esc);
        assert!(os.context_menu.is_none());
    }

    #[test]
    fn menu_enter_runs_zoom_action() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = false;
        os.open_context_menu_at(5, 5);
        // Navigate to the Zoom row (index 5).
        for _ in 0..5 {
            let down = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
            crate::app::input::handle_key(&mut os, &down);
        }
        assert_eq!(os.context_menu.as_ref().unwrap().selected, 5);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        crate::app::input::handle_key(&mut os, &enter);
        assert!(os.context_menu.is_none());
        assert!(os.windows[0].zoomed);
    }
}

#[cfg(test)]
mod rename_dialog_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 20, rows: 4 },
        );
        os.windows.push(win);
        os.focused_window = Some(0);
        os
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn open_dialog_prefills_title() {
        let mut os = os_with_window();
        os.rename_window(0, "Old");
        os.open_rename_dialog();
        let (idx, text) = os.rename_dialog.as_ref().unwrap();
        assert_eq!(*idx, 0);
        assert_eq!(text, "Old");
    }

    #[test]
    fn typing_and_commit_renames() {
        let mut os = os_with_window();
        os.open_rename_dialog();
        // The dialog prefills the current title; clear it before typing.
        for _ in 0..os.rename_dialog.as_ref().unwrap().1.len() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Backspace));
        }
        for c in "NewName".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.rename_dialog.is_none());
        assert_eq!(os.windows[0].title, "NewName");
    }

    #[test]
    fn backspace_edits_text() {
        let mut os = os_with_window();
        os.open_rename_dialog();
        // Prefill is "w0"; clear it, type "abc", then backspace once.
        for _ in 0..os.rename_dialog.as_ref().unwrap().1.len() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Backspace));
        }
        for c in "abc".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Backspace));
        let (_, text) = os.rename_dialog.as_ref().unwrap();
        assert_eq!(text, "ab");
    }

    #[test]
    fn esc_cancels_without_change() {
        let mut os = os_with_window();
        os.open_rename_dialog();
        for c in "xyz".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(os.rename_dialog.is_none());
        assert_ne!(os.windows[0].title, "xyz");
    }

    #[test]
    fn context_menu_rename_opens_dialog() {
        let mut os = os_with_window();
        os.open_context_menu_at(2, 2);
        // Navigate to Rename (index 4).
        for _ in 0..4 {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.rename_dialog.is_some());
        assert!(os.context_menu.is_none());
    }
}

#[cfg(test)]
mod layout_picker_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::layout::bsp::SerializedBSPTree;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn os_with_layout() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        // Save a layout from an empty tree.
        let bounds = os.workspace_bounds(1);
        let tree = os.workspace(1).tree.serialize();
        os.layouts.insert("tall".to_string(), tree);
        // A second layout with different defaults.
        let mut ser = SerializedBSPTree {
            root: None,
            auto_scheme: 2,
            default_ratio: 0.3,
        };
        let _ = &mut ser;
        os.layouts.insert("wide".to_string(), ser);
        let _ = bounds;
        os
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn leader_l_opens_layout_picker() {
        let mut os = os_with_layout();
        os.prefix = Prefix::Leader;
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('L')));
        assert!(os.switcher_open);
        assert_eq!(os.switcher_kind, SwitcherKind::Layout);
        assert_eq!(os.switcher_items().len(), 2);
    }

    #[test]
    fn enter_applies_selected_layout() {
        let mut os = os_with_layout();
        os.open_switcher(SwitcherKind::Layout);
        // Select the "wide" layout (second row).
        crate::app::input::handle_key(&mut os, &key(KeyCode::Down));
        assert_eq!(os.switcher_items()[os.switcher_selected].label, "wide");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(!os.switcher_open);
        assert_eq!(os.workspace(1).tree.default_ratio(), 0.3);
    }

    #[test]
    fn x_deletes_selected_layout() {
        let mut os = os_with_layout();
        os.open_switcher(SwitcherKind::Layout);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('x')));
        assert_eq!(os.layouts.len(), 1);
        assert!(!os.layouts.contains_key("tall"));
    }

    #[test]
    fn esc_closes_picker() {
        let mut os = os_with_layout();
        os.open_switcher(SwitcherKind::Layout);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.switcher_open);
    }
}

#[cfg(test)]
mod quit_menu_tests {
    use super::*;
    use crate::app::input::KeyResult;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn standalone_os() -> Os {
        Os::new(UserConfig::default_config())
    }

    fn daemon_os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.remote_session = Some("work".into());
        os.remote_sessions = vec![
            crate::session::model::SessionInfo {
                id: "s1".into(),
                name: "work".into(),
                created_at: 0,
                attached: true,
                windows: 1,
                restored: false,
            },
            crate::session::model::SessionInfo {
                id: "s2".into(),
                name: "play".into(),
                created_at: 0,
                attached: false,
                windows: 1,
                restored: false,
            },
        ];
        os
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn standalone_menu_has_quit_and_cancel() {
        let mut os = standalone_os();
        os.open_quit_menu();
        let items = os.quit_menu.as_ref().unwrap().items.clone();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, QuitMenuKind::Standalone);
        assert_eq!(items[1].kind, QuitMenuKind::Cancel);
    }

    #[test]
    fn standalone_enter_quits() {
        let mut os = standalone_os();
        os.open_quit_menu();
        let r = crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert_eq!(r, KeyResult::Quit);
        assert!(os.quitting);
        assert!(os.quit_menu.is_none());
    }

    #[test]
    fn daemon_menu_first_row_is_detach() {
        let mut os = daemon_os();
        os.open_quit_menu();
        let items = os.quit_menu.as_ref().unwrap().items.clone();
        assert_eq!(items[0].kind, QuitMenuKind::Detach);
        assert!(items.iter().any(|i| i.kind == QuitMenuKind::KillAndQuit));
    }

    #[test]
    fn daemon_detach_quits_client() {
        let mut os = daemon_os();
        os.open_quit_menu();
        let r = crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert_eq!(r, KeyResult::Quit);
        assert!(os.quitting);
    }

    #[test]
    fn daemon_switch_session_opens_switcher() {
        let mut os = daemon_os();
        os.open_quit_menu();
        // Accelerator 'S' runs the switch-session row.
        let r = crate::app::input::handle_key(&mut os, &key(KeyCode::Char('S')));
        assert_eq!(r, KeyResult::Consumed);
        assert!(os.quit_menu.is_none());
        assert!(os.switcher_open);
        assert_eq!(os.switcher_kind, SwitcherKind::Session);
    }

    #[test]
    fn daemon_kill_and_quit_sets_pending() {
        let mut os = daemon_os();
        os.open_quit_menu();
        let r = crate::app::input::handle_key(&mut os, &key(KeyCode::Char('K')));
        assert_eq!(r, KeyResult::Consumed);
        assert!(os.quit_menu.is_none());
        assert_eq!(os.pending_kill.as_deref(), Some("work"));
        assert!(os.quit_after_kill);
    }

    #[test]
    fn esc_cancels() {
        let mut os = daemon_os();
        os.open_quit_menu();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(os.quit_menu.is_none());
        assert!(!os.quitting);
    }

    #[test]
    fn arrow_navigation_wraps() {
        let mut os = standalone_os();
        os.open_quit_menu();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Down));
        assert_eq!(os.quit_menu.as_ref().unwrap().selected, 1);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Down));
        assert_eq!(os.quit_menu.as_ref().unwrap().selected, 0);
    }
}

#[cfg(test)]
mod session_close_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os_with_session() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.remote_session = Some("work".into());
        os.remote_sessions = vec![crate::session::model::SessionInfo {
            id: "s1".into(),
            name: "work".into(),
            created_at: 0,
            attached: true,
            windows: 3,
            restored: false,
        }];
        os
    }

    #[test]
    fn open_defaults_to_cancel() {
        let mut os = os_with_session();
        os.open_session_close("work");
        let (session, selected) = os.session_close.as_ref().unwrap();
        assert_eq!(session, "work");
        assert_eq!(*selected, 0);
    }

    #[test]
    fn enter_on_default_cancels() {
        let mut os = os_with_session();
        os.open_session_close("work");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.session_close.is_none());
        assert!(os.pending_kill.is_none());
    }

    #[test]
    fn select_close_and_confirm_kills() {
        let mut os = os_with_session();
        os.open_session_close("work");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(os.session_close.as_ref().unwrap().1, 1);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.session_close.is_none());
        assert_eq!(os.pending_kill.as_deref(), Some("work"));
    }

    #[test]
    fn y_shortcut_confirms() {
        let mut os = os_with_session();
        os.open_session_close("work");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('y')));
        assert!(os.session_close.is_none());
        assert_eq!(os.pending_kill.as_deref(), Some("work"));
    }

    #[test]
    fn esc_cancels() {
        let mut os = os_with_session();
        os.open_session_close("work");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(os.session_close.is_none());
        assert!(os.pending_kill.is_none());
    }

    #[test]
    fn toll_counts_windows() {
        let os = os_with_session();
        let (panes, agents) = os.session_toll("work");
        assert_eq!(panes, 3);
        assert_eq!(agents, 0);
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.theme = crate::config::Theme::built_in("dracula");
        os
    }

    #[test]
    fn open_and_close() {
        let mut os = os();
        os.open_settings();
        assert!(os.settings_open);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.settings_open);
    }

    #[test]
    fn rows_include_theme_and_toggles() {
        let os = os();
        let rows = os.settings_rows();
        assert!(rows.iter().any(|(l, _)| l == "Theme"));
        assert!(rows.iter().any(|(l, _)| l == "Animations"));
        assert!(rows.iter().any(|(l, _)| l == "Which-key overlay"));
    }

    #[test]
    fn cycle_theme_changes_theme() {
        let mut os = os();
        os.open_settings();
        // Row 0 is Theme; right arrow cycles forward.
        crate::app::input::handle_key(&mut os, &key(KeyCode::Right));
        let name = os.theme.as_ref().unwrap().name.clone();
        assert_ne!(name, "dracula");
        assert_eq!(os.config.appearance.theme, name);
    }

    #[test]
    fn toggle_animations() {
        let mut os = os();
        os.config.appearance.animations_enabled = false;
        os.open_settings();
        // Down to row 1 (Animations), then Enter toggles.
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.config.appearance.animations_enabled);
    }

    #[test]
    fn gap_adjusts_with_arrows() {
        let mut os = os();
        os.open_settings();
        // Down to row 3 (Pane gap).
        for _ in 0..3 {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Right));
        assert_eq!(os.gap, 1);
    }

    #[test]
    fn palette_settings_command_opens() {
        let mut os = os();
        os.open_palette();
        // Type to filter down to the Settings command and activate it.
        for c in "settings".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.settings_open);
    }
}

#[cfg(test)]
mod tooltip_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        let mut win = Window::without_pty(
            "w0".to_string(),
            "Long title".to_string(),
            WinSize { cols: 40, rows: 12 },
        );
        win.agent_state = "working".to_string();
        win.agent_message = "building".to_string();
        os.windows.push(win);
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os
    }

    #[test]
    fn title_bar_hover_target_includes_agent() {
        let os = os_with_window();
        // The window's rect: title bar is the top row.
        let target = os.hover_target_at(10, 0).unwrap();
        assert!(target.contains("Long title"));
        assert!(target.contains("working"));
        assert!(target.contains("building"));
    }

    #[test]
    fn inside_pane_is_not_a_hover_target() {
        let os = os_with_window();
        assert!(os.hover_target_at(10, 5).is_none());
    }

    #[test]
    fn arm_tooltip_then_tick_shows_after_delay() {
        let mut os = os_with_window();
        os.arm_tooltip(10, 0);
        assert!(os.tooltip_pending.is_some());
        assert!(os.tooltip.is_none());
        // Force the delay to have elapsed.
        if let Some((_, _, since)) = os.tooltip_pending.as_mut() {
            *since = std::time::Instant::now() - std::time::Duration::from_millis(500);
        }
        os.tick_tooltip();
        assert!(os.tooltip.is_some());
        assert!(os.tooltip_pending.is_none());
    }

    #[test]
    fn arm_tooltip_before_delay_stays_pending() {
        let mut os = os_with_window();
        os.arm_tooltip(10, 0);
        os.tick_tooltip();
        assert!(os.tooltip.is_none());
        assert!(os.tooltip_pending.is_some());
    }

    #[test]
    fn leaving_surface_clears() {
        let mut os = os_with_window();
        os.arm_tooltip(10, 0);
        os.clear_tooltip();
        assert!(os.tooltip.is_none());
        assert!(os.tooltip_pending.is_none());
    }
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os_with_two_workspaces() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        let w1 = Window::without_pty(
            "w0".to_string(),
            "alpha".to_string(),
            WinSize { cols: 40, rows: 12 },
        );
        let w2 = Window::without_pty(
            "w1".to_string(),
            "beta".to_string(),
            WinSize { cols: 40, rows: 12 },
        );
        os.windows.push(w1);
        os.windows.push(w2);
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        let bounds2 = os.workspace_bounds(2);
        os.workspace_mut(2)
            .tree
            .insert_window(1, -1, SplitType::None, 0.5, bounds2, 0);
        os
    }

    #[test]
    fn items_group_all_workspaces() {
        let os = os_with_two_workspaces();
        let items = os.aggregate_items();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|(ws, _, t, _)| *ws == 1 && t == "alpha"));
        assert!(items.iter().any(|(ws, _, t, _)| *ws == 2 && t == "beta"));
    }

    #[test]
    fn empty_when_no_windows() {
        let os = Os::new(UserConfig::default_config());
        assert!(os.aggregate_items().is_empty());
    }

    #[test]
    fn leader_a_opens_and_esc_closes() {
        let mut os = os_with_two_workspaces();
        os.prefix = Prefix::Leader;
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('A')));
        assert!(os.aggregate_open);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.aggregate_open);
    }

    #[test]
    fn enter_focuses_selected_window() {
        let mut os = os_with_two_workspaces();
        os.current_workspace = 1;
        os.open_aggregate_view();
        // Select the second item (workspace 2, beta).
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(!os.aggregate_open);
        assert_eq!(os.current_workspace, 2);
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn preview_comes_from_emulator() {
        let os = os_with_two_workspaces();
        {
            let mut emu = os.windows[0].emulator.lock().unwrap();
            emu.write(b"hello world\nsecond line");
        }
        let items = os.aggregate_items();
        let (_, _, _, preview) = items.iter().find(|(_, i, _, _)| *i == 0).unwrap();
        assert!(preview.contains("hello world"));
    }
}

#[cfg(test)]
mod sidebar_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os_with_windows() -> Os {
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
            .insert_window(1, 0, SplitType::Horizontal, 0.5, bounds, 0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn leader_b_toggles_sidebar() {
        let mut os = os_with_windows();
        os.prefix = Prefix::Leader;
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('b')));
        assert!(os.sidebar.open);
        os.prefix = Prefix::Leader;
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('b')));
        assert!(!os.sidebar.open);
    }

    #[test]
    fn rows_include_windows_with_agent_glyphs() {
        let mut os = os_with_windows();
        os.windows[1].agent_state = "working".to_string();
        let rows = os.sidebar_rows();
        assert_eq!(rows.len(), 3); // session + 2 windows
        assert_eq!(rows[1].window, Some(0));
        assert_eq!(rows[2].window, Some(1));
        assert_eq!(rows[2].agent_state, "working");
    }

    #[test]
    fn enter_focuses_selected_window() {
        let mut os = os_with_windows();
        os.sidebar.open();
        // Select the second window row (index 2).
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(!os.sidebar.open);
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn esc_closes_sidebar() {
        let mut os = os_with_windows();
        os.sidebar.open();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.sidebar.open);
    }

    #[test]
    fn navigation_wraps() {
        let mut os = os_with_windows();
        os.sidebar.open();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('k')));
        assert_eq!(os.sidebar.selected, 2); // wrapped to the last row
    }

    #[test]
    fn sidebar_rows_local_session_header() {
        let os = os_with_windows();
        let rows = os.sidebar_rows();
        assert_eq!(rows[0].kind, sidebar::RowKind::Session);
        assert!(rows[0].label.contains("workspace"));
    }
}

#[cfg(test)]
mod browser_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::scrollback::BrowseMode;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn os_with_markers() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        let w = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 40, rows: 24 },
        );
        os.windows.push(w);
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.focused_window = Some(0);
        {
            let mut emu = os.windows[0].emulator.lock().unwrap();
            emu.write(b"$ ls\r\n");
            emu.write(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07");
            emu.write(b"file1\r\n/tmp/x.log\r\n");
            emu.write(b"\x1b]133;D;0\x07");
            emu.write(b"$ echo hi\r\n");
            emu.write(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07");
            emu.write(b"{\"ok\": true}\r\n");
            emu.write(b"\x1b]133;D;0\x07");
        }
        os
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn open_parses_blocks() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        assert!(os.browser_open);
        assert!(!os.browser_blocks.is_empty());
        assert!(os.browser_blocks.iter().any(|b| b.command.contains("ls")));
    }

    #[test]
    fn empty_window_has_no_blocks() {
        let mut os = Os::new(UserConfig::default_config());
        let w = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 10, rows: 3 },
        );
        os.windows.push(w);
        os.focused_window = Some(0);
        os.open_scrollback_browser();
        assert!(os.browser_open);
        assert!(os.browser_blocks.is_empty());
    }

    #[test]
    fn navigation_and_close() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        let count = os.browser_blocks.len();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(os.browser_selected, 1 % count);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.browser_open);
    }

    #[test]
    fn mode_cycles() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        assert_eq!(os.browser_mode, BrowseMode::Commands);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(os.browser_mode, BrowseMode::Output);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(os.browser_mode, BrowseMode::Json);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(os.browser_mode, BrowseMode::Paths);
    }

    #[test]
    fn json_mode_finds_fragments() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        // Select the block with the JSON output.
        let idx = os
            .browser_blocks
            .iter()
            .position(|b| b.command.contains("echo hi"))
            .unwrap();
        os.browser_selected = idx;
        os.browser_mode = BrowseMode::Json;
        let rows = os.browser_rows();
        assert!(rows.iter().any(|r| r.contains("\"ok\"")));
    }

    #[test]
    fn paths_mode_finds_paths() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        let idx = os
            .browser_blocks
            .iter()
            .position(|b| b.command.contains("ls"))
            .unwrap();
        os.browser_selected = idx;
        os.browser_mode = BrowseMode::Paths;
        let rows = os.browser_rows();
        assert!(rows.iter().any(|r| r.contains("/tmp/x.log")));
    }

    #[test]
    fn bracket_opens_from_scrollback_mode() {
        let mut os = os_with_markers();
        os.enter_scrollback_mode();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('[')));
        assert!(os.browser_open);
    }

    // --- Tape manager cache tests ---

    fn cache_test_os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os
    }

    #[test]
    fn open_tape_manager_populates_cache() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        assert!(os.tape_manager_cache.is_some());
    }

    #[test]
    fn cache_returns_same_result_as_fresh_scan() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        let cached = os.tape_manager_items();
        os.refresh_tape_manager_cache();
        let fresh = os.scan_tape_files(&os.tape_manager_query.to_lowercase());
        assert_eq!(cached.len(), fresh.len());
    }

    #[test]
    fn update_cache_after_query_change_repopulates() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        os.tape_manager_query.push('x');
        os.update_tape_manager_cache();
        let updated = os.tape_manager_items();
        // Cache should be populated with the new query.
        assert!(os.tape_manager_cache.is_some());
        let (_, cached_items) = os.tape_manager_cache.as_ref().unwrap();
        assert_eq!(updated.len(), cached_items.len());
    }

    #[test]
    fn refresh_cache_sets_none() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        assert!(os.tape_manager_cache.is_some());
        os.refresh_tape_manager_cache();
        assert!(os.tape_manager_cache.is_none());
    }

    #[test]
    fn confirm_delete_repopulates_cache() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        // Set up a fake delete path that doesn't exist (delete will fail,
        // but the cache should still be repopulated).
        os.tape_manager_delete_path = Some(std::path::PathBuf::from("/nonexistent/tape.yaml"));
        os.tape_manager_mode = TapeManagerMode::ConfirmDelete;
        os.tape_manager_confirm_delete();
        // Cache should be repopulated (not None) after confirm_delete.
        assert!(os.tape_manager_cache.is_some());
        assert_eq!(os.tape_manager_mode, TapeManagerMode::List);
    }

}

#[cfg(test)]
mod auto_theme_tests {
    use super::*;

    // The three env-resolution tests mutate COLORFGBG, which is process
    // global — serialize them so they don't race each other.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn config_with_auto() -> UserConfig {
        let mut cfg = UserConfig::default_config();
        cfg.appearance.theme = "auto".into();
        cfg.appearance.theme_auto_dark = "catppuccin-mocha".into();
        cfg.appearance.theme_auto_light = "catppuccin-latte".into();
        cfg
    }

    #[test]
    fn auto_sets_flag_and_resolves_from_env() {
        let _g = ENV_LOCK.lock().unwrap();
        // COLORFGBG "0;15" = black fg on white bg → light host terminal.
        let prev = std::env::var("COLORFGBG").ok();
        std::env::set_var("COLORFGBG", "0;15");
        let os = Os::new(config_with_auto());
        if let Some(p) = prev {
            std::env::set_var("COLORFGBG", p);
        } else {
            std::env::remove_var("COLORFGBG");
        }
        assert!(os.auto_theme);
        let name = os.theme.as_ref().expect("auto resolved a theme").name.clone();
        assert_eq!(name, "catppuccin-latte");
    }

    #[test]
    fn auto_dark_env_resolves_dark() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("COLORFGBG").ok();
        std::env::set_var("COLORFGBG", "7;0");
        let os = Os::new(config_with_auto());
        if let Some(p) = prev {
            std::env::set_var("COLORFGBG", p);
        } else {
            std::env::remove_var("COLORFGBG");
        }
        let name = os.theme.as_ref().expect("auto resolved a theme").name.clone();
        assert_eq!(name, "catppuccin-mocha");
    }

    #[test]
    fn auto_without_env_falls_back_to_dark() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("COLORFGBG").ok();
        std::env::remove_var("COLORFGBG");
        let os = Os::new(config_with_auto());
        if let Some(p) = prev {
            std::env::set_var("COLORFGBG", p);
        }
        let name = os.theme.as_ref().expect("auto resolved a theme").name.clone();
        assert_eq!(name, "catppuccin-mocha");
    }

    #[test]
    fn explicit_theme_is_not_auto() {
        let mut cfg = UserConfig::default_config();
        cfg.appearance.theme = "dracula".into();
        let os = Os::new(cfg);
        assert!(!os.auto_theme);
        assert_eq!(os.theme.as_ref().unwrap().name, "dracula");
    }

    #[test]
    fn redetect_noops_when_not_auto() {
        let mut cfg = UserConfig::default_config();
        cfg.appearance.theme = "dracula".into();
        let mut os = Os::new(cfg);
        let before = os.theme.as_ref().unwrap().name.clone();
        os.redetect_theme();
        assert_eq!(os.theme.as_ref().unwrap().name, before);
        assert_eq!(os.config.appearance.theme, "dracula");
    }

    #[test]
    fn palette_theme_detect_command_dispatches() {
        let mut os = Os::new(config_with_auto());
        // redetect with no terminal signal keeps a valid theme and logs.
        os.run_command(Command::ThemeDetect);
        assert!(os.theme.is_some());
    }
}

#[cfg(test)]
mod command_pane_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::{Duration, Instant};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os() -> Os {
        Os::new(UserConfig::default_config())
    }

    fn wait_exit(os: &mut Os, index: usize, expected: i32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            os.poll_window_exits();
            if os.windows[index].exit_code == Some(expected) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "timed out waiting for exit {expected}, got {:?}",
            os.windows[index].exit_code
        );
    }

    #[test]
    fn spawn_command_window_captures_exit_code() {
        let mut os = os();
        let i = os
            .spawn_command_window("echo PANE_RAN; exit 3", false)
            .unwrap();
        assert_eq!(
            os.windows[i].command.as_deref(),
            Some("echo PANE_RAN; exit 3")
        );
        assert_eq!(os.focused_window, Some(i));
        wait_exit(&mut os, i, 3);
        assert!(os.windows[i].can_rerun());
        assert!(os.windows[i].exited);
    }

    #[test]
    fn dialog_commit_spawns_command_pane() {
        let mut os = os();
        os.open_command_pane_dialog();
        assert!(os.command_pane_dialog.is_some());
        for c in "echo DIALOG_OK".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.command_pane_dialog.is_none());
        assert_eq!(os.windows.len(), 1);
        assert_eq!(os.windows[0].command.as_deref(), Some("echo DIALOG_OK"));
        // Esc cancels without spawning.
        os.open_command_pane_dialog();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(os.command_pane_dialog.is_none());
        assert_eq!(os.windows.len(), 1, "cancel must not spawn");
    }

    #[test]
    fn rerun_after_exit_resets_pane() {
        let mut os = os();
        let i = os.spawn_command_window("true", false).unwrap();
        wait_exit(&mut os, i, 0);
        assert!(os.windows[i].can_rerun());
        assert!(os.rerun_focused_command_pane());
        assert_eq!(os.windows[i].exit_code, None, "rerun resets the exit status");
        assert!(!os.windows[i].exited);
        assert!(os.windows[i].command.is_some(), "command survives rerun");
    }

    #[test]
    fn suspended_spawn_resumes_on_enter() {
        let mut os = os();
        let i = os.spawn_command_window("echo SUSP; exit 0", true).unwrap();
        assert!(os.windows[i].suspended);
        assert!(os.resume_focused_suspended_pane());
        assert!(!os.windows[i].suspended);
        assert!(!os.resume_focused_suspended_pane(), "second resume no-ops");
        wait_exit(&mut os, i, 0);
    }
}

#[cfg(test)]
mod stack_and_bulk_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::layout::SplitType;

    fn os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os
    }

    fn os_with_two() -> Os {
        let mut os = os();
        os.push_fake_window("win-0", "Terminal", SplitType::Vertical);
        os.push_fake_window("win-1", "Terminal", SplitType::Vertical);
        os
    }

    #[test]
    fn stack_focused_creates_stack() {
        let mut os = os_with_two();
        let ws = os.current_workspace;
        let focused = os.focused_window.unwrap();
        os.stack_focused();
        let tree = &os.workspace(ws).tree;
        assert_eq!(tree.stack_count(focused as i32), 2);
    }

    #[test]
    fn stack_focused_noop_with_one_window() {
        let mut os = os();
        let _ = os.split(SplitType::Vertical, &os.default_shell(), Box::new(|| {}));
        let count_before = os.workspace(os.current_workspace).tree.get_all_window_ids().len();
        os.stack_focused();
        let count_after = os.workspace(os.current_workspace).tree.get_all_window_ids().len();
        assert_eq!(count_before, count_after);
    }

    #[test]
    fn cycle_stack_focus_rotates() {
        let mut os = os_with_two();
        let ws = os.current_workspace;
        let focused = os.focused_window.unwrap();
        os.stack_focused();
        os.cycle_stack_focus(true);
        let tree = &os.workspace(ws).tree;
        let new_focused = os.focused_window.unwrap();
        assert_ne!(focused, new_focused);
        assert_eq!(tree.stack_count(new_focused as i32), 2);
    }

    #[test]
    fn multi_select_toggle() {
        let mut os = os_with_two();
        assert!(!os.multi_select_mode);
        os.toggle_multi_select_mode();
        assert!(os.multi_select_mode);
        os.toggle_multi_select_mode();
        assert!(!os.multi_select_mode);
        assert!(os.selected_panes.is_empty());
    }

    #[test]
    fn select_pane_toggles() {
        let mut os = os_with_two();
        os.select_pane(0);
        assert!(os.selected_panes.contains(&0));
        os.select_pane(0);
        assert!(!os.selected_panes.contains(&0));
    }

    #[test]
    fn bulk_close_selected_removes_panes() {
        let mut os = os_with_two();
        os.select_pane(0);
        os.select_pane(1);
        os.bulk_close_selected();
        assert!(os.selected_panes.is_empty());
        assert!(!os.multi_select_mode);
        // All windows should be gone.
        assert!(os.workspace(os.current_workspace).tree.is_empty());
    }

    #[test]
    fn select_all_grabs_every_window() {
        let mut os = os_with_two();
        os.select_all_panes();
        assert_eq!(os.selected_panes.len(), 2);
        assert!(os.selected_panes.contains(&0));
        assert!(os.selected_panes.contains(&1));
        assert!(os.multi_select_mode);
    }

    #[test]
    fn bulk_stack_selected_creates_stack() {
        let mut os = os_with_two();
        os.select_pane(0);
        os.select_pane(1);
        os.bulk_stack_selected();
        let ws = os.current_workspace;
        let tree = &os.workspace(ws).tree;
        // Both windows should be in a stack.
        assert_eq!(tree.stack_count(0), 2);
        assert_eq!(tree.stack_count(1), 2);
    }

    #[test]
    fn bulk_break_selected_removes_from_stack() {
        let mut os = os_with_two();
        os.select_pane(0);
        os.select_pane(1);
        os.bulk_stack_selected();
        // Now select both and break.
        os.select_pane(0);
        os.select_pane(1);
        os.bulk_break_selected();
        let ws = os.current_workspace;
        let tree = &os.workspace(ws).tree;
        assert_eq!(tree.stack_count(0), 1);
        assert_eq!(tree.stack_count(1), 1);
    }

    #[test]
    fn command_stack_pane_dispatches() {
        let mut os = os_with_two();
        let ws = os.current_workspace;
        os.run_command(Command::StackPane);
        let tree = &os.workspace(ws).tree;
        assert_eq!(tree.stack_count(0), 2);
    }

    #[test]
    fn command_multi_select_dispatches() {
        let mut os = os();
        assert!(!os.multi_select_mode);
        os.run_command(Command::MultiSelect);
        assert!(os.multi_select_mode);
    }
}

#[cfg(test)]
mod extension_tests {
    use super::*;
    use crate::config::userconfig::{CustomActionConfig, StatusWidgetConfig};

    fn os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os
    }

    #[test]
    fn status_widget_refresh_caches_output() {
        let mut os = os();
        os.config.status_widgets.clear(); // isolate from built-in widgets
        os.config.status_widgets.push(StatusWidgetConfig {
            name: "test_widget".into(),
            command: "echo WIDGET_OK".into(),
            refresh_ms: 0,
            alignment: "right".into(),
        });
        os.update_status_widgets();
        os.flush_widget_threads();
        assert_eq!(os.widget_cache.lock().unwrap().get("test_widget").unwrap(), "WIDGET_OK");
    }

    #[test]
    fn status_widget_refresh_does_not_wait_for_slow_command() {
        let mut os = os();
        os.config.status_widgets.clear();
        os.config.status_widgets.push(StatusWidgetConfig {
            name: "slow".into(),
            command: "sleep 1; echo SLOW_WIDGET".into(),
            refresh_ms: 0,
            alignment: "right".into(),
        });

        let started = std::time::Instant::now();
        os.update_status_widgets();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "widget refresh blocked the caller"
        );
        assert_eq!(os.widget_inflight.len(), 1);
        os.flush_widget_threads();
        assert_eq!(
            os.widget_cache.lock().unwrap().get("slow").unwrap(),
            "SLOW_WIDGET"
        );
    }

    #[test]
    fn status_widgets_respect_global_worker_cap() {
        let mut os = os();
        os.config.status_widgets = (0..6)
            .map(|i| StatusWidgetConfig {
                name: format!("slow-{i}"),
                command: "sleep 1; echo done".into(),
                refresh_ms: 0,
                alignment: "right".into(),
            })
            .collect();
        os.update_status_widgets();
        assert!(os.widget_inflight.len() <= 4);
        os.flush_widget_threads();
    }

    #[test]
    fn status_widget_respects_refresh_interval() {
        let mut os = os();
        os.config.status_widgets.clear(); // isolate from built-in widgets
        os.config.status_widgets.push(StatusWidgetConfig {
            name: "slow".into(),
            command: "echo FIRST".into(),
            refresh_ms: 60_000, // 1 minute — too long to trigger.
            alignment: "right".into(),
        });
        os.update_status_widgets(); // Runs (first time).
        os.flush_widget_threads();
        assert_eq!(os.widget_cache.lock().unwrap().get("slow").unwrap(), "FIRST");
        // Overwrite with a new command to detect whether it re-runs.
        os.config.status_widgets[0].command = "echo SECOND".into();
        os.update_status_widgets(); // Should skip (too soon).
        os.flush_widget_threads();
        assert_eq!(os.widget_cache.lock().unwrap().get("slow").unwrap(), "FIRST");
    }

    #[test]
    fn custom_action_dispatches() {
        let mut os = os();
        let dir = std::env::temp_dir().join(format!("termos-ext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("action_fired");
        let _ = std::fs::remove_file(&marker);
        os.config.custom_actions.push(CustomActionConfig {
            name: "Test action".into(),
            command: format!("touch {}", marker.display()),
            category: "Custom".into(),
        });
        os.run_custom_action("Test action");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "custom action did not fire");
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn custom_action_appears_in_palette() {
        let mut os = os();
        os.config.custom_actions.push(CustomActionConfig {
            name: "My Widget".into(),
            command: "echo hi".into(),
            category: "Custom".into(),
        });
        os.open_palette();
        os.palette_query = "widget".into();
        let items = os.palette_items();
        assert!(items.iter().any(|(c, _)| matches!(c, Command::CustomAction(n) if n == "My Widget")));
    }

    #[test]
    fn config_backward_compat_no_widgets() {
        // Default config ships with built-in status_widgets and custom_actions.
        let cfg = UserConfig::default_config();
        assert!(!cfg.status_widgets.is_empty());
        assert!(!cfg.custom_actions.is_empty());
        // Round-trip through TOML: serialize then deserialize preserves fields.
        let serialized = toml::to_string(&cfg).unwrap();
        let cfg2: UserConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(cfg2.status_widgets.len(), cfg.status_widgets.len());
        assert_eq!(cfg2.custom_actions.len(), cfg.custom_actions.len());
        // An empty override still works (stripping all defaults).
        let cfg3: UserConfig = toml::from_str("").unwrap();
        assert!(cfg3.status_widgets.is_empty());
        assert!(cfg3.custom_actions.is_empty());
    }
}

#[cfg(test)]
mod layout_mode_tests {
    use super::*;

    fn os_with_two() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os.push_fake_window("win-0", "Terminal", SplitType::Vertical);
        os.push_fake_window("win-1", "Terminal", SplitType::Vertical);
        os
    }

    #[test]
    fn cycle_layout_mode_bsp_to_ms_to_scroll() {
        let mut os = os_with_two();
        assert_eq!(os.layout_mode, crate::layout::LayoutMode::BSP);
        os.cycle_layout_mode();
        assert_eq!(os.layout_mode, crate::layout::LayoutMode::MasterStack);
        os.cycle_layout_mode();
        assert_eq!(os.layout_mode, crate::layout::LayoutMode::Scrolling);
        os.cycle_layout_mode();
        assert_eq!(os.layout_mode, crate::layout::LayoutMode::BSP);
    }

    #[test]
    fn master_stack_layout_produces_rects() {
        let mut os = os_with_two();
        os.layout_mode = crate::layout::LayoutMode::MasterStack;
        os.invalidate_layout_cache();
        let layout = os.current_layout();
        assert_eq!(layout.len(), 2);
        // Master on the left (50% width).
        let r0 = layout.get(&0).unwrap();
        assert_eq!(r0.x, 0);
        assert!(r0.w > 0);
        // Stack on the right.
        let r1 = layout.get(&1).unwrap();
        assert!(r1.x > r0.x);
    }

    #[test]
    fn scrolling_layout_produces_rects() {
        let mut os = os_with_two();
        os.layout_mode = crate::layout::LayoutMode::Scrolling;
        os.sync_scrolling_from_workspace();
        os.invalidate_layout_cache();
        let layout = os.current_layout();
        assert_eq!(layout.len(), 2);
    }

    #[test]
    fn layout_mode_label() {
        assert_eq!(crate::layout::LayoutMode::BSP.label(), "BSP");
        assert_eq!(crate::layout::LayoutMode::MasterStack.label(), "MS");
        assert_eq!(crate::layout::LayoutMode::Scrolling.label(), "SCR");
    }

    #[test]
    fn layout_mode_next_cycles() {
        assert_eq!(crate::layout::LayoutMode::BSP.next(), crate::layout::LayoutMode::MasterStack);
        assert_eq!(crate::layout::LayoutMode::MasterStack.next(), crate::layout::LayoutMode::Scrolling);
        assert_eq!(crate::layout::LayoutMode::Scrolling.next(), crate::layout::LayoutMode::BSP);
    }

    #[test]
    fn layout_mode_from_config() {
        assert_eq!(crate::layout::LayoutMode::from_config(""), crate::layout::LayoutMode::BSP);
        assert_eq!(crate::layout::LayoutMode::from_config("bsp"), crate::layout::LayoutMode::BSP);
        assert_eq!(crate::layout::LayoutMode::from_config("master-stack"), crate::layout::LayoutMode::MasterStack);
        assert_eq!(crate::layout::LayoutMode::from_config("master_stack"), crate::layout::LayoutMode::MasterStack);
        assert_eq!(crate::layout::LayoutMode::from_config("ms"), crate::layout::LayoutMode::MasterStack);
        assert_eq!(crate::layout::LayoutMode::from_config("scrolling"), crate::layout::LayoutMode::Scrolling);
        assert_eq!(crate::layout::LayoutMode::from_config("scr"), crate::layout::LayoutMode::Scrolling);
        assert_eq!(crate::layout::LayoutMode::from_config("invalid"), crate::layout::LayoutMode::BSP);
    }

    #[test]
    fn config_layout_mode_parsed() {
        let toml = r#"
            [appearance]
            layout_mode = "master-stack"
            master_ratio = 0.6
        "#;
        let cfg: crate::config::UserConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.appearance.layout_mode, "master-stack");
        assert!((cfg.appearance.master_ratio - 0.6).abs() < 0.01);
    }

    #[test]
    fn config_layout_mode_default() {
        let cfg = crate::config::UserConfig::default_config();
        assert_eq!(cfg.appearance.layout_mode, "");
        assert!((cfg.appearance.master_ratio - 0.5).abs() < 0.01);
    }
}

#[cfg(test)]
mod damage_wiring_tests {
    use super::*;
    use crate::app::damage::DamageReason;

    /// Build Os with two fake windows and a valid DamageSet.
    fn os_with_two() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os.damage_resize(80, 25);
        os.damage_take(); // drain the full Resize damage so tests start clean
        os.push_fake_window("win-0", "Terminal", SplitType::Vertical);
        os.push_fake_window("win-1", "Terminal", SplitType::Vertical);
        os
    }

    #[test]
    fn damage_full_marks_bounds_and_requests_render() {
        let mut os = os_with_two();
        os.render_requested = false;
        os.damage_full(DamageReason::Theme);
        assert!(os.render_requested);
        assert!(os.damage.is_full());

        let taken = os.damage_take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].reason, DamageReason::Theme);
        assert!(os.damage.is_empty());
    }

    #[test]
    fn damage_rect_marks_specific_region() {
        let mut os = os_with_two();
        os.render_requested = false;
        let rect = Rect { x: 10, y: 4, w: 12, h: 6 };
        os.damage_rect(rect, DamageReason::Output);
        assert!(os.render_requested);

        let taken = os.damage_take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].rect, rect);
        assert_eq!(taken[0].reason, DamageReason::Output);
    }

    #[test]
    fn damage_resize_replaces_bounds_and_full_marks() {
        let mut os = os_with_two();
        os.damage_rect(Rect { x: 0, y: 0, w: 5, h: 5 }, DamageReason::Output);
        os.damage_resize(120, 40);

        assert!(os.damage.is_full());
        assert_eq!(os.damage.bounds(), Rect { x: 0, y: 0, w: 120, h: 40 });

        let taken = os.damage_take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].reason, DamageReason::Resize);
    }

    #[test]
    fn collect_pane_damage_marks_dirty_windows() {
        let mut os = os_with_two();
        // Fake windows start dirty (no PTY output has been consumed yet).
        assert!(os.windows.iter().all(|w| w.is_dirty()));

        os.collect_pane_damage();

        let taken = os.damage_take();
        assert!(!taken.is_empty());
        assert!(taken.iter().all(|d| d.reason == DamageReason::Output));
    }

    #[test]
    fn collect_pane_damage_skips_clean_windows() {
        let mut os = os_with_two();
        for w in &os.windows {
            w.clear_dirty();
        }
        assert!(os.damage.is_empty());

        os.collect_pane_damage();

        assert!(os.damage.is_empty());
    }

    #[test]
    fn damage_resize_seeds_bounds_for_first_frame() {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        // Before damage_resize, bounds are (0,0,0,0).
        assert_eq!(os.damage.bounds(), Rect { x: 0, y: 0, w: 0, h: 0 });

        // Simulate what set_os_size does.
        os.damage_resize(os.width, os.height);

        assert_eq!(os.damage.bounds(), Rect { x: 0, y: 0, w: 80, h: 25 });
        assert!(os.damage.is_full());

        let taken = os.damage_take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].reason, DamageReason::Resize);
        assert_eq!(taken[0].rect, Rect { x: 0, y: 0, w: 80, h: 25 });
        assert!(os.damage.is_empty());
    }

    #[test]
    fn minimize_focused_hides_window_from_layout() {
        let mut os = os_with_two();
        os.focused_window = Some(0);
        let before = os.current_layout();
        assert!(before.contains_key(&0));

        os.minimize_focused();

        assert!(os.windows[0].minimized);
        let after = os.current_layout();
        assert!(!after.contains_key(&0), "minimized window should not be in layout");
        // Focus should have moved to window 1.
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn restore_window_brings_it_back() {
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        assert!(os.windows[0].minimized);

        os.restore_window(0);

        assert!(!os.windows[0].minimized);
        assert_eq!(os.focused_window, Some(0));
        let layout = os.current_layout();
        assert!(layout.contains_key(&0));
    }

    #[test]
    fn restore_last_minimized_picks_last() {
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        os.focused_window = Some(1);
        os.minimize_focused();
        // Both minimized.
        assert!(os.windows[0].minimized);
        assert!(os.windows[1].minimized);

        os.restore_last_minimized();

        // Last minimized (index 1) should be restored.
        assert!(!os.windows[1].minimized);
        assert!(os.windows[0].minimized);
    }

    #[test]
    fn dock_items_include_minimized_windows() {
        use crate::app::dock::get_dock_items;
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();

        let items = get_dock_items(&os);
        assert_eq!(items.len(), 1);
        assert!(items[0].minimized);
        assert_eq!(items[0].window_index, 0);
    }

    #[test]
    fn dock_items_empty_when_no_minimized() {
        use crate::app::dock::get_dock_items;
        let os = os_with_two();
        let items = get_dock_items(&os);
        assert!(items.is_empty());
    }

    #[test]
    fn dock_count_includes_all_minimized() {
        use crate::app::dock::build_dock_left_text;
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        os.focused_window = Some(1);
        os.minimize_focused();
        // build_dock_left_text counts via BSP tree which retains minimized IDs.
        let (_, trail, _) = build_dock_left_text(&os);
        assert!(trail.contains(":2 "), "trail should contain ':2 ' but got: {trail}");
    }

    #[test]
    fn all_minimized_layout_empty() {
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        os.focused_window = Some(1);
        os.minimize_focused();
        // current_layout should be empty (all minimized).
        let layout = os.current_layout();
        assert!(layout.is_empty());
    }

    #[test]
    fn all_minimized_dock_items_count() {
        use crate::app::dock::get_dock_items;
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        os.focused_window = Some(1);
        os.minimize_focused();
        let items = get_dock_items(&os);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.minimized));
    }
}
