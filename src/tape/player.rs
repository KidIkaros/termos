//! Tape player — sequential playback state machine, ported from TUIOS
//! `internal/tape/player.go`.

use std::time::Duration;

use super::command::Command;

/// Manages script playback.
#[derive(Debug, Clone)]
pub struct Player {
    commands: Vec<Command>,
    /// Current command index.
    index: usize,
    /// Whether playback is paused.
    paused: bool,
    /// Whether all commands have been played.
    finished: bool,
    /// Remaining delay before the next command.
    current_delay: Duration,
}

impl Player {
    /// Create a player from a list of commands.
    pub fn new(commands: Vec<Command>) -> Self {
        Self {
            commands,
            index: 0,
            paused: false,
            finished: false,
            current_delay: Duration::ZERO,
        }
    }

    /// The next command to execute, without advancing.
    pub fn next_command(&self) -> Option<&Command> {
        self.commands.get(self.index)
    }

    /// Move to the next command.
    pub fn advance(&mut self) {
        if self.index < self.commands.len() {
            self.index += 1;
        }
        if self.index >= self.commands.len() {
            self.finished = true;
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Reset to the beginning.
    pub fn reset(&mut self) {
        self.index = 0;
        self.paused = false;
        self.finished = false;
        self.current_delay = Duration::ZERO;
    }

    pub fn current_index(&self) -> usize {
        self.index
    }

    pub fn total_commands(&self) -> usize {
        self.commands.len()
    }

    /// A value between 0 and 100 representing playback progress.
    pub fn progress(&self) -> usize {
        if self.commands.is_empty() {
            return 100;
        }
        (self.index * 100) / self.commands.len()
    }

    /// A string representation of the current command for display.
    pub fn command_str(&self) -> String {
        match self.commands.get(self.index) {
            Some(cmd) => cmd.string(),
            None => "Script finished".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::command::CommandType;

    fn cmd() -> Command {
        Command {
            type_: CommandType::Enter,
            args: Vec::new(),
            delay: Duration::ZERO,
            line: 1,
            column: 1,
            raw: "Enter".into(),
        }
    }

    #[test]
    fn advances_until_finished() {
        let mut p = Player::new(vec![cmd(), cmd(), cmd()]);
        assert!(!p.is_finished());
        assert_eq!(p.total_commands(), 3);
        for i in 0..3 {
            assert_eq!(p.current_index(), i);
            assert!(p.next_command().is_some());
            p.advance();
        }
        assert!(p.is_finished());
        assert_eq!(p.next_command(), None);
        assert_eq!(p.command_str(), "Script finished");
        assert_eq!(p.progress(), 100);
    }

    #[test]
    fn progress_and_reset() {
        let mut p = Player::new(vec![cmd(), cmd(), cmd(), cmd()]);
        assert_eq!(p.progress(), 0);
        p.advance();
        p.advance();
        assert_eq!(p.progress(), 50);
        p.reset();
        assert_eq!(p.current_index(), 0);
        assert!(!p.is_finished());
        assert!(!p.is_paused());
    }

    #[test]
    fn empty_tape_is_finished() {
        let p = Player::new(Vec::new());
        assert_eq!(p.progress(), 100);
        assert_eq!(p.command_str(), "Script finished");
    }

    #[test]
    fn pause_state() {
        let mut p = Player::new(vec![cmd()]);
        p.set_paused(true);
        assert!(p.is_paused());
        p.set_paused(false);
        assert!(!p.is_paused());
    }
}
