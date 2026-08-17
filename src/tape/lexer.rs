//! Tape lexer — tokenizes `.tape` file input, ported from TUIOS
//! `internal/tape/lexer.go`.

use super::token::{lookup_keyword, Token, TokenType};

/// Tokenizes `.tape` input.
#[derive(Debug)]
pub struct Lexer {
    input: Vec<u8>,
    pos: usize,     // current position
    next_pos: usize, // next position
    ch: u8,         // current character (0 = EOF)
    line: usize,    // current line (1-based)
    column: usize,  // current column (1-based)
}

impl Lexer {
    /// Create a lexer for the given input.
    pub fn new(input: &str) -> Self {
        let mut l = Self {
            input: input.as_bytes().to_vec(),
            pos: 0,
            next_pos: 0,
            ch: 0,
            line: 1,
            column: 0,
        };
        l.read_char();
        l
    }

    fn read_char(&mut self) {
        if self.next_pos >= self.input.len() {
            self.ch = 0; // EOF
        } else {
            self.ch = self.input[self.next_pos];
        }
        if self.next_pos > 0 && self.ch == b'\n' {
            self.line += 1;
            self.column = 0;
        }
        self.pos = self.next_pos;
        self.next_pos += 1;
        self.column += 1;
    }

    fn peek_char(&self) -> u8 {
        if self.next_pos >= self.input.len() {
            0
        } else {
            self.input[self.next_pos]
        }
    }

    /// Skip spaces, tabs and `\r` (never newlines).
    fn skip_whitespace(&mut self) {
        while self.ch == b' ' || self.ch == b'\t' || self.ch == b'\r' {
            self.read_char();
        }
    }

    /// Skip a comment line (from `#` to end of line).
    fn skip_comment(&mut self) {
        while self.ch != b'\n' && self.ch != 0 {
            self.read_char();
        }
    }

    /// Read a quoted string (single, double, or backtick), unescaping.
    fn read_string(&mut self, quote: u8) -> String {
        let mut out = String::new();
        self.read_char(); // skip opening quote
        while self.ch != quote && self.ch != 0 {
            if self.ch == b'\\' {
                self.read_char();
                let c = match self.ch {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'\\' => '\\',
                    b'"' => '"',
                    b'\'' => '\'',
                    b'`' => '`',
                    other => other as char,
                };
                out.push(c);
            } else {
                out.push(self.ch as char);
            }
            self.read_char();
        }
        if self.ch == quote {
            self.read_char(); // skip closing quote
        }
        out
    }

    /// Read an identifier or keyword.
    fn read_identifier(&mut self) -> String {
        let mut out = String::new();
        while is_identifier_char(self.ch) {
            out.push(self.ch as char);
            self.read_char();
        }
        out
    }

    /// Read a number literal including a decimal point.
    fn read_number_with_decimal(&mut self) -> String {
        let mut out = String::new();
        while is_digit(self.ch) {
            out.push(self.ch as char);
            self.read_char();
        }
        if self.ch == b'.' && is_digit(self.peek_char()) {
            out.push('.');
            self.read_char();
            while is_digit(self.ch) {
                out.push(self.ch as char);
                self.read_char();
            }
        }
        out
    }

    /// Read a regex pattern `/pattern/` (the slashes are consumed; escapes
    /// are preserved verbatim so the parser/executor sees the pattern).
    fn read_regex(&mut self) -> String {
        let mut out = String::new();
        self.read_char(); // skip opening /
        while self.ch != b'/' && self.ch != 0 {
            if self.ch == b'\\' {
                out.push(self.ch as char);
                self.read_char();
                if self.ch != 0 {
                    out.push(self.ch as char);
                    self.read_char();
                }
            } else {
                out.push(self.ch as char);
                self.read_char();
            }
        }
        if self.ch == b'/' {
            self.read_char(); // skip closing /
        }
        out
    }

    /// Return the next token.
    pub fn next_token(&mut self) -> Token {
        let mut tok = Token {
            type_: TokenType::Eof,
            literal: String::new(),
            line: self.line,
            column: self.column,
        };

        self.skip_whitespace();

        match self.ch {
            0 => {
                tok.type_ = TokenType::Eof;
            }
            b'\n' => {
                tok.type_ = TokenType::Newline;
                tok.literal = "\n".into();
                self.read_char();
            }
            b'#' => {
                self.skip_comment();
                return self.next_token(); // skip comments
            }
            b'+' => {
                tok.type_ = TokenType::Plus;
                tok.literal = "+".into();
                self.read_char();
            }
            b'@' => {
                tok.type_ = TokenType::At;
                tok.literal = "@".into();
                self.read_char();
            }
            b',' => {
                tok.type_ = TokenType::Comma;
                tok.literal = ",".into();
                self.read_char();
            }
            b'/' => {
                // A regex (for WaitUntilRegex) or a plain slash.
                if self.peek_char() == b'/' || is_identifier_char(self.peek_char()) {
                    let regex = self.read_regex();
                    tok.type_ = TokenType::Slash;
                    tok.literal = regex;
                } else {
                    tok.type_ = TokenType::Slash;
                    tok.literal = "/".into();
                    self.read_char();
                }
            }
            b'(' => {
                tok.type_ = TokenType::LParen;
                tok.literal = "(".into();
                self.read_char();
            }
            b')' => {
                tok.type_ = TokenType::RParen;
                tok.literal = ")".into();
                self.read_char();
            }
            b'"' | b'\'' | b'`' => {
                let quote = self.ch;
                tok.type_ = TokenType::String;
                tok.literal = self.read_string(quote);
            }
            _ => {
                if is_digit(self.ch) {
                    let num = self.read_number_with_decimal();
                    // A letter after the number makes it a duration; consume
                    // alternating unit/number runs so compound Go durations
                    // like 1m30s tokenize as a single token.
                    if self.ch.is_ascii_alphabetic() {
                        let mut literal = num;
                        while self.ch.is_ascii_alphabetic() {
                            while self.ch.is_ascii_alphabetic() {
                                literal.push(self.ch as char);
                                self.read_char();
                            }
                            if is_digit(self.ch) {
                                literal.push_str(&self.read_number_with_decimal());
                            }
                        }
                        tok.type_ = TokenType::Duration;
                        tok.literal = literal;
                    } else {
                        tok.type_ = TokenType::Number;
                        tok.literal = num;
                    }
                } else if is_identifier_char(self.ch) {
                    let literal = self.read_identifier();
                    tok.type_ = lookup_keyword(&literal);
                    tok.literal = literal;
                } else {
                    // Keep the raw byte so error messages point at a character
                    // that is actually in the file (see the Go comment).
                    tok.type_ = TokenType::Illegal;
                    tok.literal = (self.ch as char).to_string();
                    self.read_char();
                }
            }
        }

        tok
    }
}

fn is_digit(ch: u8) -> bool {
    ch.is_ascii_digit()
}

fn is_identifier_char(ch: u8) -> bool {
    ch.is_ascii_alphabetic() || ch.is_ascii_digit() || ch == b'_'
}

/// Tokenize the whole input (useful for testing).
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut l = Lexer::new(input);
    let mut tokens = Vec::new();
    loop {
        let tok = l.next_token();
        let eof = tok.type_ == TokenType::Eof;
        tokens.push(tok);
        if eof {
            break;
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::token::TokenType as T;

    fn types(input: &str) -> Vec<TokenType> {
        tokenize(input).into_iter().map(|t| t.type_).collect()
    }

    #[test]
    fn basic_tokens() {
        assert_eq!(types(r#"Type "hello""#), vec![T::Type, T::String, T::Eof]);
        assert_eq!(types("Sleep 500ms"), vec![T::Sleep, T::Duration, T::Eof]);
        assert_eq!(types("Enter"), vec![T::Enter, T::Eof]);
        assert_eq!(types("Space"), vec![T::Space, T::Eof]);
        assert_eq!(
            types("Ctrl+B"),
            vec![T::Ctrl, T::Plus, T::Identifier, T::Eof]
        );
    }

    #[test]
    fn strings() {
        for input in [r#"Type "hello world""#, "Type 'hello world'", "Type `hello world`"] {
            let toks = tokenize(input);
            let s = toks
                .iter()
                .find(|t| t.type_ == T::String)
                .expect("string token");
            assert_eq!(s.literal, "hello world");
        }
        let toks = tokenize(r#"Type "hello \"world\"""#);
        let s = toks
            .iter()
            .find(|t| t.type_ == T::String)
            .expect("string token");
        assert_eq!(s.literal, r#"hello "world""#);
    }

    #[test]
    fn durations() {
        for (input, want) in [
            ("Sleep 500ms", "500ms"),
            ("Sleep 2s", "2s"),
            ("Sleep 1.5s", "1.5s"),
            ("Sleep 1m30s", "1m30s"),
            ("Sleep 1h2m3s", "1h2m3s"),
            ("Sleep 1.5m30s", "1.5m30s"),
        ] {
            let toks = tokenize(input);
            let d: Vec<&Token> = toks.iter().filter(|t| t.type_ == T::Duration).collect();
            assert_eq!(d.len(), 1, "expected 1 duration token for {input}");
            assert_eq!(d[0].literal, want, "duration literal for {input}");
        }
    }

    #[test]
    fn comments_are_skipped_but_newlines_kept() {
        let input = "# This is a comment\nType \"hello\"\n# Another comment\nEnter";
        assert_eq!(
            types(input),
            vec![
                T::Newline,
                T::Type,
                T::String,
                T::Newline,
                T::Newline,
                T::Enter,
                T::Eof
            ]
        );
    }

    #[test]
    fn identifiers_and_keywords() {
        let input = "NewWindow\nCloseWindow\nFocus 1\nSwitchWorkspace 2";
        assert_eq!(
            types(input),
            vec![
                T::NewWindow,
                T::Newline,
                T::CloseWindow,
                T::Newline,
                T::Focus,
                T::Number,
                T::Newline,
                T::SwitchWorkspace,
                T::Number,
                T::Eof
            ]
        );
    }

    #[test]
    fn line_numbers() {
        let input = "Type \"line1\"\nType \"line2\"\nType \"line3\"";
        let lines: Vec<usize> = tokenize(input)
            .into_iter()
            .filter(|t| t.type_ == T::Type)
            .map(|t| t.line)
            .collect();
        assert_eq!(lines, vec![1, 2, 3]);
    }

    #[test]
    fn at_modifier() {
        let input = "Type@100ms \"hello\"\nSleep@2s 500ms";
        let at_count = tokenize(input)
            .iter()
            .filter(|t| t.type_ == T::At)
            .count();
        assert_eq!(at_count, 2);
    }

    #[test]
    fn keyword_lookup_is_case_insensitive() {
        assert_eq!(lookup_keyword("Type"), T::Type);
        assert_eq!(lookup_keyword("type"), T::Type);
        assert_eq!(lookup_keyword("sleep"), T::Sleep);
        assert_eq!(lookup_keyword("NEWWINDOW"), T::NewWindow);
        assert_eq!(lookup_keyword("UnknownKeyword"), T::Identifier);
    }

    #[test]
    fn token_type_helpers() {
        assert!(T::Type.is_command());
        assert!(!T::String.is_command());
        assert!(T::Ctrl.is_modifier());
        assert!(T::Alt.is_modifier());
        assert!(!T::Type.is_modifier());
        assert!(T::Up.is_navigation_key());
        assert!(!T::Type.is_navigation_key());
    }

    #[test]
    fn illegal_characters_are_kept_verbatim() {
        // An unrecognized character must come back as itself, not re-encoded
        // (the Go fuzz finding this guards against).
        let input = "X~\nEnter";
        let toks = tokenize(input);
        let illegal = toks
            .iter()
            .find(|t| t.type_ == T::Illegal)
            .expect("illegal token");
        assert_eq!(illegal.literal, "~");
        assert_eq!(illegal.line, 1);
    }
}
