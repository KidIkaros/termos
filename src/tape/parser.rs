//! Tape parser — recursive descent over the lexer, ported from TUIOS
//! `internal/tape/parser.go`. Errors are collected without aborting, so
//! playback continues with the valid commands.

use super::command::{parse_duration, Command, CommandType};
use super::lexer::Lexer;
use super::token::{Token, TokenType};

/// Parses `.tape` files into commands.
#[derive(Debug)]
pub struct Parser {
    lexer: Lexer,
    cur: Token,
    peek: Token,
    errors: Vec<String>,
    /// Set by a parse fn that hit the Go `return cmd, false` path, so
    /// `parse_command` drops the command (Go's `parseCommand`).
    failed: bool,
}

impl Parser {
    pub fn new(lexer: Lexer) -> Self {
        let mut p = Self {
            lexer,
            cur: Token {
                type_: TokenType::Eof,
                literal: String::new(),
                line: 0,
                column: 0,
            },
            peek: Token {
                type_: TokenType::Eof,
                literal: String::new(),
                line: 0,
                column: 0,
            },
            errors: Vec::new(),
            failed: false,
        };
        p.next_token();
        p.next_token();
        p
    }

    fn next_token(&mut self) {
        self.cur = std::mem::replace(
            &mut self.peek,
            Token {
                type_: TokenType::Eof,
                literal: String::new(),
                line: 0,
                column: 0,
            },
        );
        self.peek = self.lexer.next_token();
    }

    /// Parse the entire tape file and return all commands.
    pub fn parse(&mut self) -> Vec<Command> {
        let mut commands = Vec::new();
        while self.cur.type_ != TokenType::Eof {
            if self.cur.type_ == TokenType::Newline {
                self.next_token();
                continue;
            }
            if let Some(cmd) = self.parse_command() {
                commands.push(cmd);
            } else {
                self.next_token();
            }
        }
        commands
    }

    /// Parse a single command.
    fn parse_command(&mut self) -> Option<Command> {
        let line = self.cur.line;
        let column = self.cur.column;

        // Skip any leading newlines.
        while self.cur.type_ == TokenType::Newline {
            self.next_token();
        }
        if self.cur.type_ == TokenType::Eof {
            return None;
        }

        let cmd = match self.cur.type_ {
            TokenType::Type => self.parse_type_command(),
            TokenType::Sleep => self.parse_sleep_command(),
            TokenType::Enter => self.parse_basic_command(CommandType::Enter),
            TokenType::Space => self.parse_basic_command(CommandType::Space),
            TokenType::Backspace => self.parse_basic_command(CommandType::Backspace),
            TokenType::Delete => self.parse_basic_command(CommandType::Delete),
            TokenType::Tab => self.parse_basic_command(CommandType::Tab),
            TokenType::Escape => self.parse_basic_command(CommandType::Escape),
            TokenType::Up => self.parse_basic_command(CommandType::Up),
            TokenType::Down => self.parse_basic_command(CommandType::Down),
            TokenType::Left => self.parse_basic_command(CommandType::Left),
            TokenType::Right => self.parse_basic_command(CommandType::Right),
            TokenType::Home => self.parse_basic_command(CommandType::Home),
            TokenType::End => self.parse_basic_command(CommandType::End),
            TokenType::Ctrl | TokenType::Alt | TokenType::Shift => self.parse_key_combo_command(),
            TokenType::TerminalMode => self.parse_basic_command(CommandType::TerminalMode),
            TokenType::WindowManagementMode => {
                self.parse_basic_command(CommandType::WindowManagementMode)
            }
            TokenType::NewWindow => self.parse_basic_command(CommandType::NewWindow),
            TokenType::CloseWindow => self.parse_basic_command(CommandType::CloseWindow),
            TokenType::NextWindow => self.parse_basic_command(CommandType::NextWindow),
            TokenType::PrevWindow => self.parse_basic_command(CommandType::PrevWindow),
            TokenType::FocusWindow => self.parse_window_id_command(CommandType::FocusWindow),
            TokenType::RenameWindow => self.parse_window_rename_command(),
            TokenType::MinimizeWindow => self.parse_basic_command(CommandType::MinimizeWindow),
            TokenType::RestoreWindow => self.parse_basic_command(CommandType::RestoreWindow),
            TokenType::ToggleTiling => self.parse_basic_command(CommandType::ToggleTiling),
            TokenType::EnableTiling => self.parse_basic_command(CommandType::EnableTiling),
            TokenType::DisableTiling => self.parse_basic_command(CommandType::DisableTiling),
            TokenType::SnapLeft => self.parse_basic_command(CommandType::SnapLeft),
            TokenType::SnapRight => self.parse_basic_command(CommandType::SnapRight),
            TokenType::SnapFullscreen => self.parse_basic_command(CommandType::SnapFullscreen),
            TokenType::SwitchWorkspace => self.parse_switch_workspace_command(),
            TokenType::MoveToWorkspace => self.parse_move_to_workspace_command(),
            TokenType::MoveAndFollowWorkspace => self.parse_move_and_follow_workspace_command(),
            TokenType::Split => self.parse_basic_command(CommandType::Split),
            TokenType::RotateSplit => self.parse_basic_command(CommandType::RotateSplit),
            TokenType::EqualizeSplits => self.parse_basic_command(CommandType::EqualizeSplits),
            TokenType::ToggleZoom => self.parse_basic_command(CommandType::ToggleZoom),
            TokenType::SmartSplit => self.parse_basic_command(CommandType::SmartSplit),
            TokenType::CommandPalette => self.parse_basic_command(CommandType::CommandPalette),
            TokenType::SaveLayout => self.parse_save_layout_command(),
            TokenType::LoadLayout => self.parse_load_layout_command(),
            TokenType::Focus => self.parse_focus_command(),
            TokenType::Wait => self.parse_wait_command(),
            TokenType::WaitUntilRegex => self.parse_wait_until_regex_command(),
            TokenType::Set => self.parse_set_command(),
            TokenType::Output => self.parse_output_command(),
            TokenType::Source => self.parse_source_command(),
            TokenType::EnableAnimations => self.parse_basic_command(CommandType::EnableAnimations),
            TokenType::DisableAnimations => {
                self.parse_basic_command(CommandType::DisableAnimations)
            }
            TokenType::ToggleAnimations => self.parse_basic_command(CommandType::ToggleAnimations),
            _ => {
                self.add_error(format!("unexpected token: {:?}", self.cur.type_));
                self.skip_to_next_line();
                return None;
            }
        };

        // A parse fn that hit the Go `return cmd, false` path drops the
        // command (its error was already recorded).
        if self.failed {
            self.failed = false;
            return None;
        }
        let _ = (line, column);
        Some(cmd)
    }

    /// Parse simple commands with an optional `@duration` delay and repeat
    /// count.
    fn parse_basic_command(&mut self, cmd_type: CommandType) -> Command {
        let mut cmd = Command::new(cmd_type, self.cur.line, self.cur.column);
        let cmd_name = self.cur.literal.clone();
        self.next_token();

        // Optional delay modifier (@<duration>).
        if self.cur.type_ == TokenType::At {
            self.next_token();
            if self.cur.type_ == TokenType::Duration {
                if let Some(d) = parse_duration(&self.cur.literal) {
                    cmd.delay = d;
                } else {
                    self.add_error(format!("invalid duration: {}", self.cur.literal));
                }
                self.next_token();
            } else {
                self.add_error("expected duration after @".into());
            }
        }

        // Optional repeat count (number).
        if self.cur.type_ == TokenType::Number {
            cmd.args.push(self.cur.literal.clone());
            self.next_token();
        }

        cmd.raw = cmd_name;
        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse `Type "text"` (with optional `@speed`).
    fn parse_type_command(&mut self) -> Command {
        let mut cmd = Command::new(CommandType::Type, self.cur.line, self.cur.column);
        self.next_token(); // consume Type

        if self.cur.type_ == TokenType::At {
            self.next_token();
            if self.cur.type_ == TokenType::Duration {
                if let Some(d) = parse_duration(&self.cur.literal) {
                    cmd.delay = d;
                } else {
                    self.add_error(format!("invalid duration: {}", self.cur.literal));
                }
                self.next_token();
            } else {
                self.add_error("expected duration after @".into());
            }
        }

        if self.cur.type_ == TokenType::String {
            cmd.args = vec![self.cur.literal.clone()];
            cmd.raw = format!("Type {:?}", self.cur.literal);
            self.next_token();
        } else {
            self.add_error(format!(
                "Type command expects a string, got {:?}",
                self.cur.type_
            ));
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse `Sleep <duration>`.
    fn parse_sleep_command(&mut self) -> Command {
        let mut cmd = Command::new(CommandType::Sleep, self.cur.line, self.cur.column);
        self.next_token(); // consume Sleep

        if self.cur.type_ == TokenType::Duration {
            if let Some(d) = parse_duration(&self.cur.literal) {
                cmd.delay = d;
            } else {
                self.add_error(format!("invalid duration: {}", self.cur.literal));
            }
            cmd.args = vec![self.cur.literal.clone()];
            cmd.raw = format!("Sleep {}", self.cur.literal);
            self.next_token();
        } else {
            self.add_error(format!(
                "Sleep command expects a duration, got {:?}",
                self.cur.type_
            ));
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse `Ctrl+X`, `Alt+X`, `Ctrl+Alt+X`, etc.
    fn parse_key_combo_command(&mut self) -> Command {
        let mut cmd = Command::new(CommandType::KeyCombo, self.cur.line, self.cur.column);
        let mut combo_parts: Vec<String> = Vec::new();

        while self.cur.type_ == TokenType::Ctrl
            || self.cur.type_ == TokenType::Alt
            || self.cur.type_ == TokenType::Shift
        {
            combo_parts.push(self.cur.literal.clone());
            self.next_token();
            if self.cur.type_ == TokenType::Plus {
                self.next_token();
            }
        }

        // The final key. The EOF guard keeps an empty literal from panicking
        // (the Go comment calls this out explicitly).
        let valid_key = self.cur.type_ == TokenType::Identifier
            || self.cur.type_.is_navigation_key()
            || self.cur.type_ == TokenType::Enter
            || self.cur.type_ == TokenType::Space
            || self.cur.type_ == TokenType::Number
            || (!self.cur.literal.is_empty()
                && self.cur.literal.chars().next().unwrap().is_ascii_digit());
        if valid_key {
            combo_parts.push(self.cur.literal.clone());
            self.next_token();
        } else {
            self.add_error(format!(
                "expected key after modifier, got {:?}",
                self.cur.type_
            ));
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        let combo_str = combo_parts.join("+");
        cmd.args = vec![combo_str.clone()];
        cmd.raw = combo_str;

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse `Focus <target>`.
    fn parse_focus_command(&mut self) -> Command {
        let mut cmd = Command::new(CommandType::Focus, self.cur.line, self.cur.column);
        self.next_token(); // consume Focus

        if self.cur.type_ == TokenType::Identifier || self.cur.type_ == TokenType::Number {
            cmd.args = vec![self.cur.literal.clone()];
            cmd.raw = format!("Focus {}", self.cur.literal);
            self.next_token();
        } else {
            self.add_error("Focus command expects an identifier or number".into());
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    fn parse_switch_workspace_command(&mut self) -> Command {
        self.parse_number_command(CommandType::SwitchWorkspace, "SwitchWorkspace")
    }

    fn parse_move_to_workspace_command(&mut self) -> Command {
        self.parse_number_command(CommandType::MoveToWorkspace, "MoveToWorkspace")
    }

    fn parse_move_and_follow_workspace_command(&mut self) -> Command {
        self.parse_number_command(
            CommandType::MoveAndFollowWorkspace,
            "MoveAndFollowWorkspace",
        )
    }

    fn parse_number_command(&mut self, cmd_type: CommandType, name: &str) -> Command {
        let mut cmd = Command::new(cmd_type, self.cur.line, self.cur.column);
        self.next_token(); // consume the command name

        if self.cur.type_ == TokenType::Number {
            cmd.args = vec![self.cur.literal.clone()];
            cmd.raw = format!("{name} {}", self.cur.literal);
            self.next_token();
        } else {
            self.add_error(format!("{name} expects a number, got {:?}", self.cur.type_));
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse commands that take a window ID, like `FocusWindow <id>`.
    fn parse_window_id_command(&mut self, cmd_type: CommandType) -> Command {
        let mut cmd = Command::new(cmd_type, self.cur.line, self.cur.column);
        self.next_token(); // consume the command name

        if self.cur.type_ == TokenType::Identifier || self.cur.type_ == TokenType::Number {
            cmd.args = vec![self.cur.literal.clone()];
            cmd.raw = format!("{cmd_type:?} {}", self.cur.literal);
            self.next_token();
        } else {
            self.add_error(format!(
                "{cmd_type:?} expects a window ID, got {:?}",
                self.cur.type_
            ));
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse `RenameWindow <name>`.
    fn parse_window_rename_command(&mut self) -> Command {
        let mut cmd = Command::new(CommandType::RenameWindow, self.cur.line, self.cur.column);
        self.next_token(); // consume RenameWindow

        match self.cur.type_ {
            TokenType::String | TokenType::Identifier => {
                cmd.args = vec![self.cur.literal.clone()];
                cmd.raw = format!("RenameWindow {}", self.cur.literal);
                self.next_token();
            }
            _ => {
                self.add_error("RenameWindow expects a window name".into());
                self.failed = true;
                self.skip_to_next_line();
                return cmd;
            }
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse `Wait <duration>` (an alias for Sleep).
    fn parse_wait_command(&mut self) -> Command {
        let mut cmd = Command::new(CommandType::Wait, self.cur.line, self.cur.column);
        self.next_token(); // consume Wait

        if self.cur.type_ == TokenType::Duration {
            if let Some(d) = parse_duration(&self.cur.literal) {
                cmd.delay = d;
            } else {
                self.add_error(format!("invalid duration: {}", self.cur.literal));
            }
            cmd.args = vec![self.cur.literal.clone()];
            cmd.raw = format!("Wait {}", self.cur.literal);
            self.next_token();
        } else {
            self.add_error(format!(
                "Wait command expects a duration, got {:?}",
                self.cur.type_
            ));
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse `WaitUntilRegex <pattern> [timeout_ms]`.
    fn parse_wait_until_regex_command(&mut self) -> Command {
        let mut cmd = Command::new(CommandType::WaitUntilRegex, self.cur.line, self.cur.column);
        self.next_token(); // consume WaitUntilRegex

        if self.cur.type_ == TokenType::String {
            let pattern = self.cur.literal.clone();
            cmd.args = vec![pattern.clone()];
            self.next_token();

            if self.cur.type_ == TokenType::Number {
                cmd.args.push(self.cur.literal.clone());
                cmd.raw = format!("WaitUntilRegex {:?} {}", pattern, self.cur.literal);
                self.next_token();
            } else {
                cmd.raw = format!("WaitUntilRegex {:?}", pattern);
            }
        } else {
            self.add_error("WaitUntilRegex expects a regex pattern string".into());
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    /// Parse `Set <key> <value>`.
    fn parse_set_command(&mut self) -> Command {
        let mut cmd = Command::new(CommandType::Set, self.cur.line, self.cur.column);
        self.next_token(); // consume Set

        if self.cur.type_ == TokenType::Identifier {
            let key = self.cur.literal.clone();
            self.next_token();

            if matches!(
                self.cur.type_,
                TokenType::Identifier | TokenType::String | TokenType::Number | TokenType::Duration
            ) {
                let value = self.cur.literal.clone();
                cmd.args = vec![key.clone(), value.clone()];
                cmd.raw = format!("Set {key} {value}");
                self.next_token();
            } else {
                self.add_error("Set command expects a value".into());
                self.failed = true;
                self.skip_to_next_line();
                return cmd;
            }
        } else {
            self.add_error("Set command expects a key".into());
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    fn parse_file_arg_command(&mut self, cmd_type: CommandType, name: &str) -> Command {
        let mut cmd = Command::new(cmd_type, self.cur.line, self.cur.column);
        self.next_token(); // consume the command name

        if self.cur.type_ == TokenType::String || self.cur.type_ == TokenType::Identifier {
            cmd.args = vec![self.cur.literal.clone()];
            cmd.raw = format!("{name} {}", self.cur.literal);
            self.next_token();
        } else {
            self.add_error(format!("{name} command expects a filename"));
            self.failed = true;
            self.skip_to_next_line();
            return cmd;
        }

        if self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.skip_to_next_line();
        }
        cmd
    }

    fn parse_output_command(&mut self) -> Command {
        self.parse_file_arg_command(CommandType::Output, "Output")
    }

    fn parse_source_command(&mut self) -> Command {
        self.parse_file_arg_command(CommandType::Source, "Source")
    }

    fn parse_save_layout_command(&mut self) -> Command {
        self.parse_file_arg_command(CommandType::SaveLayout, "SaveLayout")
    }

    fn parse_load_layout_command(&mut self) -> Command {
        self.parse_file_arg_command(CommandType::LoadLayout, "LoadLayout")
    }

    fn skip_to_next_line(&mut self) {
        while self.cur.type_ != TokenType::Newline && self.cur.type_ != TokenType::Eof {
            self.next_token();
        }
    }

    fn add_error(&mut self, msg: String) {
        self.errors.push(format!("line {}: {msg}", self.cur.line));
    }

    /// The list of collected parse errors.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }
}

/// Parse a tape file from a string.
pub fn parse_file(content: &str) -> (Vec<Command>, Vec<String>) {
    let mut p = Parser::new(Lexer::new(content));
    let commands = p.parse();
    (commands, p.errors.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn basic_commands() {
        let input = "Type \"hello\"\nEnter\nSleep 500ms\nSpace";
        let (commands, errors) = parse_file(input);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        let types: Vec<CommandType> = commands.iter().map(|c| c.type_).collect();
        assert_eq!(
            types,
            vec![
                CommandType::Type,
                CommandType::Enter,
                CommandType::Sleep,
                CommandType::Space
            ]
        );
    }

    #[test]
    fn type_command() {
        for (input, want) in [
            (r#"Type "hello""#, "hello"),
            (r#"Type "hello world""#, "hello world"),
            (r#"Type "say \"hi\"""#, r#"say "hi""#),
        ] {
            let (commands, _) = parse_file(input);
            assert_eq!(commands[0].type_, CommandType::Type);
            assert_eq!(commands[0].args[0], want);
        }
    }

    #[test]
    fn sleep_command() {
        for (input, arg, want) in [
            ("Sleep 500ms", "500ms", Duration::from_millis(500)),
            ("Sleep 2s", "2s", Duration::from_secs(2)),
        ] {
            let (commands, _) = parse_file(input);
            assert_eq!(commands[0].type_, CommandType::Sleep);
            assert_eq!(commands[0].args[0], arg);
            assert_eq!(commands[0].delay, want);
        }
    }

    #[test]
    fn wait_aliases_sleep() {
        let (commands, errors) = parse_file("Wait 750ms");
        assert!(errors.is_empty());
        assert_eq!(commands[0].type_, CommandType::Wait);
        assert_eq!(commands[0].delay, Duration::from_millis(750));

        let (_, errors) = parse_file("Wait");
        assert!(!errors.is_empty(), "Wait without a duration must error");
    }

    #[test]
    fn wait_until_regex() {
        let (commands, errors) = parse_file(r#"WaitUntilRegex "\$" 3000"#);
        assert!(errors.is_empty());
        assert_eq!(commands[0].type_, CommandType::WaitUntilRegex);
        assert_eq!(commands[0].args, vec!["$", "3000"]);

        let (commands, errors) = parse_file(r#"WaitUntilRegex "done""#);
        assert!(errors.is_empty());
        assert_eq!(commands[0].args, vec!["done"]);
    }

    #[test]
    fn key_combos() {
        for (input, want) in [
            ("Ctrl+B", "Ctrl+B"),
            ("Alt+1", "Alt+1"),
            ("Ctrl+Alt+D", "Ctrl+Alt+D"),
        ] {
            let (commands, _) = parse_file(input);
            assert_eq!(commands[0].type_, CommandType::KeyCombo);
            assert_eq!(commands[0].args[0], want);
        }
    }

    #[test]
    fn tuios_actions() {
        for (input, want) in [
            ("NewWindow", CommandType::NewWindow),
            ("CloseWindow", CommandType::CloseWindow),
            ("ToggleTiling", CommandType::ToggleTiling),
        ] {
            let (commands, _) = parse_file(input);
            assert_eq!(commands[0].type_, want);
        }
    }

    #[test]
    fn switch_workspace() {
        let (commands, _) = parse_file("SwitchWorkspace 2");
        assert_eq!(commands[0].type_, CommandType::SwitchWorkspace);
        assert_eq!(commands[0].args[0], "2");
    }

    #[test]
    fn comments_skipped() {
        let input = "# comment\nType \"hello\"\n# another\nEnter";
        let (commands, _) = parse_file(input);
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].type_, CommandType::Type);
        assert_eq!(commands[1].type_, CommandType::Enter);
    }

    #[test]
    fn complex_script() {
        let input = "# Demo tape script\nType \"echo 'Hello World'\"\nSleep 500ms\nEnter\n\n# Switch workspace\nAlt+2\nSleep 1s\n\n# Create new window\nNewWindow\nSleep 200ms\nType \"vim\"\nEnter";
        let (commands, errors) = parse_file(input);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(commands.len(), 9);
    }

    #[test]
    fn error_handling() {
        let (_, errors) = parse_file("Type");
        assert_eq!(errors.len(), 1);
        let (_, errors) = parse_file("Sleep invalid");
        assert_eq!(errors.len(), 1);
        let (_, errors) = parse_file("Type \"hello\"\nEnter");
        assert!(errors.is_empty());
    }

    #[test]
    fn line_numbers() {
        let (commands, _) = parse_file("Type \"line1\"\nEnter\nType \"line3\"");
        let lines: Vec<usize> = commands.iter().map(|c| c.line).collect();
        assert_eq!(lines, vec![1, 2, 3]);
    }

    #[test]
    fn delay_modifier() {
        let input = "Type@100ms \"hello\"\nEnter@50ms\nBackspace@200ms 3";
        let (commands, _) = parse_file(input);
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].delay, Duration::from_millis(100));
        assert_eq!(commands[1].delay, Duration::from_millis(50));
        assert_eq!(commands[2].delay, Duration::from_millis(200));
    }

    #[test]
    fn key_combo_no_panic() {
        for input in ["Ctrl", "Ctrl+", "Alt+", "Ctrl+Alt+"] {
            let (commands, errors) = parse_file(input);
            assert!(!errors.is_empty(), "expected an error for {input:?}");
            assert!(commands.is_empty(), "expected no commands for {input:?}");
        }
    }

    #[test]
    fn compound_duration() {
        let (commands, errors) = parse_file("Sleep 1m30s");
        assert!(errors.is_empty());
        assert_eq!(commands[0].args[0], "1m30s");
        assert_eq!(commands[0].delay, Duration::from_secs(90));
    }

    #[test]
    fn multiple_repeat() {
        let input = "Backspace 5\nDown 3\nUp 10";
        let (commands, _) = parse_file(input);
        let args: Vec<&str> = commands.iter().map(|c| c.args[0].as_str()).collect();
        assert_eq!(args, vec!["5", "3", "10"]);
    }
}
