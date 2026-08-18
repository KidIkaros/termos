//! The ANSI/VT escape-sequence parser — a from-scratch state machine in the
//! spirit of Paul Williams' vt100 parser design (and the `x/ansi` parser the
//! Go code uses).
//!
//! States: Ground, Escape, EscapeIntermediate, CSI, CSIIntermediate, CSIParam,
//! OSC, DCS, DCSIntermediate, DCSParam, DCSPassthrough, APC, PM, SOS, Utf8.
//!
//! The parser consumes one byte at a time and dispatches to handler callbacks
//! for printable characters, control characters, and complete escape
//! sequences.

/// Maximum number of CSI/OSC parameters.
pub const MAX_PARAMS: usize = 32;

/// A parsed parameter value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    pub value: i64,
    pub has_value: bool,
}

impl Param {
    /// Return the parameter value or `default` if it has no value.
    pub fn or(self, default: i64) -> i64 {
        if self.has_value {
            self.value
        } else {
            default
        }
    }
}

/// The parser state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    CsiIntermediate,
    CsiParam,
    Osc,
    Dcs,
    DcsIntermediate,
    DcsParam,
    DcsPassthrough,
    Apc,
    Pm,
    Sos,
    Utf8,
    /// Inside a string sequence (OSC/DCS/APC/PM/SOS), we saw ESC and are
    /// waiting for the `\` that completes the two-byte String Terminator.
    StringTerminator {
        kind: StringKind,
    },
}

/// A parsed CSI sequence.
#[derive(Debug, Clone)]
pub struct CsiSequence {
    pub params: Vec<Param>,
    pub intermediates: Vec<u8>,
    pub final_byte: u8,
    pub private: bool,
    /// The specific private marker byte (0 if none): `<`, `=`, `>`, or `?`.
    pub private_marker: u8,
}

/// A parsed DCS sequence.
#[derive(Debug, Clone)]
pub struct DcsSequence {
    pub params: Vec<Param>,
    pub intermediates: Vec<u8>,
    pub final_byte: u8,
    pub data: Vec<u8>,
}

/// A parsed OSC sequence (raw payload, already stripped of the ST terminator).
#[derive(Debug, Clone)]
pub struct OscSequence {
    pub data: Vec<u8>,
}

/// A parsed APC/PM/SOS sequence.
#[derive(Debug, Clone)]
pub struct StringSequence {
    pub data: Vec<u8>,
}

/// Handlers invoked by the parser as it recognizes events.
pub trait Handler {
    /// A printable character (including UTF-8 continuation bytes already
    /// decoded into a char by the caller's grapheme logic).
    fn print(&mut self, c: char);
    /// A control character (C0 or C1).
    fn execute(&mut self, c: u8);
    /// A complete CSI sequence.
    fn csi(&mut self, seq: &CsiSequence);
    /// A complete ESC sequence (the byte after ESC, plus intermediates).
    fn esc(&mut self, intermediates: &[u8], final_byte: u8);
    /// A complete DCS sequence.
    fn dcs(&mut self, seq: &DcsSequence);
    /// A complete OSC sequence.
    fn osc(&mut self, seq: &OscSequence);
    /// A complete APC sequence.
    fn apc(&mut self, seq: &StringSequence);
    /// A complete PM sequence.
    fn pm(&mut self, seq: &StringSequence);
    /// A complete SOS sequence.
    fn sos(&mut self, seq: &StringSequence);
}

/// The parser. Feed it bytes one at a time via [`Parser::advance`].
#[derive(Debug)]
pub struct Parser {
    state: State,
    /// Accumulated UTF-8 bytes for the current char.
    utf8_buf: [u8; 4],
    /// Number of bytes received so far for the current char (lead byte = 1).
    utf8_pos: usize,
    /// CSI parameter accumulation.
    params: Vec<Param>,
    current_param: i64,
    param_has_value: bool,
    /// CSI/ESC intermediate bytes.
    intermediates: Vec<u8>,
    /// Whether a CSI sequence is private (starts with `?`, `>`, `!`, `=`).
    private: bool,
    /// The specific private marker byte (0 if none).
    private_marker: u8,
    /// String-sequence (OSC/DCS/APC/PM/SOS) data.
    string_data: Vec<u8>,
    /// Raw bytes for the current string sequence.
    string_raw: Vec<u8>,
    /// DCS final byte.
    dcs_final: u8,
    /// The last state (used by callers to detect UTF-8 transitions).
    last_state: State,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            utf8_buf: [0; 4],
            utf8_pos: 0,
            params: Vec::new(),
            current_param: 0,
            param_has_value: false,
            intermediates: Vec::new(),
            private: false,
            private_marker: 0,
            string_data: Vec::new(),
            string_raw: Vec::new(),
            dcs_final: 0,
            last_state: State::Ground,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn last_state(&self) -> State {
        self.last_state
    }

    /// Handle a single byte. Returns the new state.
    pub fn advance<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        self.last_state = self.state;
        let next = match self.state {
            State::Ground => self.ground(byte, handler),
            State::Escape => self.escape(byte, handler),
            State::EscapeIntermediate => self.escape_intermediate(byte, handler),
            State::Csi => self.csi(byte, handler),
            State::CsiIntermediate => self.csi_intermediate(byte, handler),
            State::CsiParam => self.csi_param(byte, handler),
            State::Osc => self.osc(byte, handler),
            State::Dcs => self.dcs(byte, handler),
            State::DcsIntermediate => self.dcs_intermediate(byte, handler),
            State::DcsParam => self.dcs_param(byte, handler),
            State::DcsPassthrough => self.dcs_passthrough(byte, handler),
            State::Apc => self.apc(byte, handler),
            State::Pm => self.pm(byte, handler),
            State::Sos => self.sos(byte, handler),
            State::Utf8 => self.utf8(byte, handler),
            State::StringTerminator { kind } => self.string_terminator(byte, handler, kind),
        };
        self.state = next;
        next
    }

    fn reset_string(&mut self) {
        self.string_data.clear();
        self.string_raw.clear();
    }

    // -----------------------------------------------------------------------
    // Ground
    // -----------------------------------------------------------------------

    fn ground<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        match byte {
            0x00..=0x17 | 0x19 | 0x1c..=0x1f => {
                handler.execute(byte);
                State::Ground
            }
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => {
                self.reset_string();
                State::Escape
            }
            0x20..=0x7f => {
                handler.print(byte as char);
                State::Ground
            }
            0x80..=0x9f => {
                // C1 controls.
                match byte {
                    0x90 => State::Dcs,
                    0x9d => State::Osc,
                    0x9e => State::Pm,
                    0x9f => State::Apc,
                    0x98 => State::Sos,
                    _ => {
                        handler.execute(byte);
                        State::Ground
                    }
                }
            }
            _ => {
                // UTF-8 lead byte.
                self.utf8_buf[0] = byte;
                self.utf8_pos = 1;
                let total = utf8_len_for_lead(byte);
                if total <= 1 {
                    // Invalid lead; treat as printable replacement.
                    handler.print('\u{FFFD}');
                    State::Ground
                } else {
                    State::Utf8
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // UTF-8
    // -----------------------------------------------------------------------

    fn utf8<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        let total = utf8_len_for_lead(self.utf8_buf[0]);
        if (byte & 0xC0) == 0x80 && self.utf8_pos < total && self.utf8_pos < 4 {
            self.utf8_buf[self.utf8_pos] = byte;
            self.utf8_pos += 1;
            if self.utf8_pos == total {
                // Complete.
                let s = std::str::from_utf8(&self.utf8_buf[..self.utf8_pos]);
                match s {
                    Ok(s) => {
                        for c in s.chars() {
                            handler.print(c);
                        }
                    }
                    Err(_) => handler.print('\u{FFFD}'),
                }
                self.utf8_pos = 0;
                return State::Ground;
            }
            return State::Utf8;
        }
        // Invalid continuation; fall back to ground handling.
        self.utf8_pos = 0;
        self.ground(byte, handler)
    }

    // -----------------------------------------------------------------------
    // Escape
    // -----------------------------------------------------------------------

    fn escape<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        self.intermediates.clear();
        self.private = false;
        self.private_marker = 0;
        match byte {
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => State::Escape,
            0x20..=0x2f => {
                self.intermediates.push(byte);
                State::EscapeIntermediate
            }
            0x30..=0x4f | 0x51..=0x57 | 0x59 | 0x5a | 0x5c | 0x60..=0x7e => {
                handler.esc(&self.intermediates, byte);
                State::Ground
            }
            0x50 => {
                // DCS.
                self.reset_string();
                self.params.clear();
                self.current_param = 0;
                self.param_has_value = false;
                self.intermediates.clear();
                self.private = false;
                self.dcs_final = 0;
                State::Dcs
            }
            0x58 => State::Sos,
            0x5b => State::Csi,
            0x5d => State::Osc,
            0x5e => State::Pm,
            0x5f => State::Apc,
            0x7f => State::Escape,
            _ => {
                // Unknown escape; ignore.
                State::Ground
            }
        }
    }

    fn escape_intermediate<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        match byte {
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => State::Escape,
            0x20..=0x2f => {
                self.intermediates.push(byte);
                State::EscapeIntermediate
            }
            0x30..=0x7e => {
                handler.esc(&self.intermediates, byte);
                State::Ground
            }
            _ => State::Ground,
        }
    }

    // -----------------------------------------------------------------------
    // CSI
    // -----------------------------------------------------------------------

    fn csi<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        self.params.clear();
        self.current_param = 0;
        self.param_has_value = false;
        self.intermediates.clear();
        self.private = false;
        self.private_marker = 0;
        match byte {
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => State::Escape,
            0x3a => {
                self.params.push(Param {
                    value: 0,
                    has_value: false,
                });
                State::CsiParam
            }
            0x3b => {
                self.params.push(Param {
                    value: self.current_param,
                    has_value: self.param_has_value,
                });
                self.current_param = 0;
                self.param_has_value = false;
                State::CsiParam
            }
            0x3c..=0x3f => {
                // Private marker.
                self.private = true;
                self.private_marker = byte;
                State::CsiParam
            }
            0x30..=0x39 => {
                self.current_param = self.current_param * 10 + (byte - 0x30) as i64;
                self.param_has_value = true;
                State::CsiParam
            }
            0x20..=0x2f => {
                self.intermediates.push(byte);
                State::CsiIntermediate
            }
            0x40..=0x7e => {
                self.params.push(Param {
                    value: self.current_param,
                    has_value: self.param_has_value,
                });
                handler.csi(&CsiSequence {
                    params: self.params.clone(),
                    intermediates: self.intermediates.clone(),
                    final_byte: byte,
                    private: self.private,
                    private_marker: self.private_marker,
                });
                State::Ground
            }
            _ => State::Ground,
        }
    }

    fn csi_intermediate<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        match byte {
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => State::Escape,
            0x20..=0x2f => {
                self.intermediates.push(byte);
                State::CsiIntermediate
            }
            0x30..=0x3f => {
                self.params.clear();
                self.current_param = 0;
                self.param_has_value = false;
                State::CsiParam
            }
            0x40..=0x7e => {
                // Push the last accumulated param before dispatching.
                if self.param_has_value || self.current_param != 0 {
                    self.params.push(Param {
                        value: self.current_param,
                        has_value: self.param_has_value,
                    });
                }
                handler.csi(&CsiSequence {
                    params: self.params.clone(),
                    intermediates: self.intermediates.clone(),
                    final_byte: byte,
                    private: self.private,
                    private_marker: self.private_marker,
                });
                State::Ground
            }
            _ => State::Ground,
        }
    }

    fn csi_param<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        match byte {
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => State::Escape,
            0x3a => {
                self.params.push(Param {
                    value: 0,
                    has_value: false,
                });
                State::CsiParam
            }
            0x3b => {
                self.params.push(Param {
                    value: self.current_param,
                    has_value: self.param_has_value,
                });
                self.current_param = 0;
                self.param_has_value = false;
                State::CsiParam
            }
            0x30..=0x39 => {
                self.current_param = self.current_param * 10 + (byte - 0x30) as i64;
                self.param_has_value = true;
                State::CsiParam
            }
            0x20..=0x2f => {
                self.intermediates.push(byte);
                State::CsiIntermediate
            }
            0x40..=0x7e => {
                self.params.push(Param {
                    value: self.current_param,
                    has_value: self.param_has_value,
                });
                handler.csi(&CsiSequence {
                    params: self.params.clone(),
                    intermediates: self.intermediates.clone(),
                    final_byte: byte,
                    private: self.private,
                    private_marker: self.private_marker,
                });
                State::Ground
            }
            _ => State::Ground,
        }
    }

    // -----------------------------------------------------------------------
    // OSC / DCS / APC / PM / SOS
    // -----------------------------------------------------------------------

    fn osc<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        match byte {
            0x1b => {
                // Might be ST (ESC \) or an escape into something else.
                self.string_raw.push(byte);
                State::StringTerminator {
                    kind: StringKind::Osc,
                }
            }
            0x07 => {
                // BEL terminates OSC.
                handler.osc(&OscSequence {
                    data: std::mem::take(&mut self.string_data),
                });
                State::Ground
            }
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            _ => {
                self.string_data.push(byte);
                self.string_raw.push(byte);
                State::Osc
            }
        }
    }

    fn dcs<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        self.params.clear();
        self.current_param = 0;
        self.param_has_value = false;
        self.intermediates.clear();
        self.private = false;
        self.private_marker = 0;
        match byte {
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => State::Escape,
            0x3a => {
                self.params.push(Param {
                    value: 0,
                    has_value: false,
                });
                State::DcsParam
            }
            0x3b => {
                self.params.push(Param {
                    value: self.current_param,
                    has_value: self.param_has_value,
                });
                self.current_param = 0;
                self.param_has_value = false;
                State::DcsParam
            }
            0x30..=0x39 => {
                self.current_param = self.current_param * 10 + (byte - 0x30) as i64;
                self.param_has_value = true;
                State::DcsParam
            }
            0x20..=0x2f => {
                self.intermediates.push(byte);
                State::DcsIntermediate
            }
            0x40..=0x7e => {
                self.dcs_final = byte;
                self.string_data.clear();
                self.string_raw.clear();
                State::DcsPassthrough
            }
            _ => State::Ground,
        }
    }

    fn dcs_intermediate<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        match byte {
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => State::Escape,
            0x20..=0x2f => {
                self.intermediates.push(byte);
                State::DcsIntermediate
            }
            0x30..=0x3f => {
                self.params.clear();
                self.current_param = 0;
                self.param_has_value = false;
                State::DcsParam
            }
            0x40..=0x7e => {
                self.dcs_final = byte;
                self.string_data.clear();
                self.string_raw.clear();
                State::DcsPassthrough
            }
            _ => State::Ground,
        }
    }

    fn dcs_param<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        match byte {
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x1b => State::Escape,
            0x3a => {
                self.params.push(Param {
                    value: 0,
                    has_value: false,
                });
                State::DcsParam
            }
            0x3b => {
                self.params.push(Param {
                    value: self.current_param,
                    has_value: self.param_has_value,
                });
                self.current_param = 0;
                self.param_has_value = false;
                State::DcsParam
            }
            0x30..=0x39 => {
                self.current_param = self.current_param * 10 + (byte - 0x30) as i64;
                self.param_has_value = true;
                State::DcsParam
            }
            0x20..=0x2f => {
                self.intermediates.push(byte);
                State::DcsIntermediate
            }
            0x40..=0x7e => {
                self.dcs_final = byte;
                self.string_data.clear();
                self.string_raw.clear();
                State::DcsPassthrough
            }
            _ => State::Ground,
        }
    }

    fn dcs_passthrough<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        match byte {
            0x1b => {
                // Possible ST (ESC \) or other escape.
                self.string_raw.push(byte);
                State::StringTerminator {
                    kind: StringKind::Dcs,
                }
            }
            0x9c => {
                // ST (C1).
                handler.dcs(&DcsSequence {
                    params: self.params.clone(),
                    intermediates: self.intermediates.clone(),
                    final_byte: self.dcs_final,
                    data: std::mem::take(&mut self.string_data),
                });
                State::Ground
            }
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            _ => {
                self.string_data.push(byte);
                self.string_raw.push(byte);
                State::DcsPassthrough
            }
        }
    }

    fn apc<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        self.string_sequence(byte, handler, StringKind::Apc)
    }

    fn pm<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        self.string_sequence(byte, handler, StringKind::Pm)
    }

    fn sos<H: Handler>(&mut self, byte: u8, handler: &mut H) -> State {
        self.string_sequence(byte, handler, StringKind::Sos)
    }

    fn string_sequence<H: Handler>(
        &mut self,
        byte: u8,
        handler: &mut H,
        kind: StringKind,
    ) -> State {
        match byte {
            0x1b => {
                // ESC might be the start of the two-byte ST (ESC \).
                // Transition to a special state that expects the backslash.
                self.string_raw.push(byte);
                State::StringTerminator { kind }
            }
            0x9c => {
                // Single-byte ST (C1).
                self.dispatch_string(handler, kind);
                State::Ground
            }
            0x18 | 0x1a => {
                handler.execute(byte);
                State::Ground
            }
            0x07 => {
                // BEL is an alternative string terminator (OSC commonly).
                self.dispatch_string(handler, kind);
                State::Ground
            }
            _ => {
                self.string_data.push(byte);
                self.string_raw.push(byte);
                self.state
            }
        }
    }

    /// Handle the byte after ESC inside a string sequence. `\` (0x5c)
    /// completes the ST and dispatches; anything else cancels the ST and
    /// the ESC is treated as the start of a new escape.
    fn string_terminator<H: Handler>(
        &mut self,
        byte: u8,
        handler: &mut H,
        kind: StringKind,
    ) -> State {
        match byte {
            0x5c => {
                // ESC \ — the two-byte String Terminator.
                self.dispatch_string(handler, kind);
                State::Ground
            }
            _ => {
                // Not ST; the string is cancelled and we re-enter the escape
                // state for this byte.
                self.dispatch_string(handler, kind);
                self.escape(byte, handler)
            }
        }
    }

    fn dispatch_string<H: Handler>(&mut self, handler: &mut H, kind: StringKind) {
        match kind {
            StringKind::Osc => {
                handler.osc(&OscSequence {
                    data: std::mem::take(&mut self.string_data),
                });
            }
            StringKind::Dcs => {
                handler.dcs(&DcsSequence {
                    params: self.params.clone(),
                    intermediates: self.intermediates.clone(),
                    final_byte: self.dcs_final,
                    data: std::mem::take(&mut self.string_data),
                });
            }
            StringKind::Apc => {
                let seq = StringSequence {
                    data: std::mem::take(&mut self.string_data),
                };
                handler.apc(&seq);
            }
            StringKind::Pm => {
                let seq = StringSequence {
                    data: std::mem::take(&mut self.string_data),
                };
                handler.pm(&seq);
            }
            StringKind::Sos => {
                let seq = StringSequence {
                    data: std::mem::take(&mut self.string_data),
                };
                handler.sos(&seq);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    Osc,
    Dcs,
    Apc,
    Pm,
    Sos,
}

/// Number of bytes in a UTF-8 sequence given its lead byte.
fn utf8_len_for_lead(byte: u8) -> usize {
    if byte < 0x80 {
        1
    } else if (byte & 0xE0) == 0xC0 {
        2
    } else if (byte & 0xF0) == 0xE0 {
        3
    } else if (byte & 0xF8) == 0xF0 {
        4
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Capture {
        printed: Vec<char>,
        executed: Vec<u8>,
        csi: Vec<CsiSequence>,
        esc: Vec<(Vec<u8>, u8)>,
        osc: Vec<Vec<u8>>,
    }

    impl Capture {
        fn new() -> Self {
            Self {
                printed: Vec::new(),
                executed: Vec::new(),
                csi: Vec::new(),
                esc: Vec::new(),
                osc: Vec::new(),
            }
        }
    }

    impl Handler for Capture {
        fn print(&mut self, c: char) {
            self.printed.push(c);
        }
        fn execute(&mut self, c: u8) {
            self.executed.push(c);
        }
        fn csi(&mut self, seq: &CsiSequence) {
            self.csi.push(seq.clone());
        }
        fn esc(&mut self, intermediates: &[u8], final_byte: u8) {
            self.esc.push((intermediates.to_vec(), final_byte));
        }
        fn dcs(&mut self, _seq: &DcsSequence) {}
        fn osc(&mut self, seq: &OscSequence) {
            self.osc.push(seq.data.clone());
        }
        fn apc(&mut self, _seq: &StringSequence) {}
        fn pm(&mut self, _seq: &StringSequence) {}
        fn sos(&mut self, _seq: &StringSequence) {}
    }

    #[test]
    fn prints_text() {
        let mut p = Parser::new();
        let mut c = Capture::new();
        for b in b"hello" {
            p.advance(*b, &mut c);
        }
        assert_eq!(c.printed, vec!['h', 'e', 'l', 'l', 'o']);
    }

    #[test]
    fn parses_csi_params() {
        let mut p = Parser::new();
        let mut c = Capture::new();
        for b in b"\x1b[5;10H" {
            p.advance(*b, &mut c);
        }
        assert_eq!(c.csi.len(), 1);
        let seq = &c.csi[0];
        assert_eq!(seq.final_byte, b'H');
        assert_eq!(seq.params.len(), 2);
        assert_eq!(seq.params[0].value, 5);
        assert_eq!(seq.params[1].value, 10);
    }

    #[test]
    fn parses_csi_default_params() {
        let mut p = Parser::new();
        let mut c = Capture::new();
        for b in b"\x1b[J" {
            p.advance(*b, &mut c);
        }
        assert_eq!(c.csi.len(), 1);
        let seq = &c.csi[0];
        assert_eq!(seq.final_byte, b'J');
        assert_eq!(seq.params.len(), 1);
        assert!(!seq.params[0].has_value);
    }

    #[test]
    fn parses_private_csi() {
        let mut p = Parser::new();
        let mut c = Capture::new();
        for b in b"\x1b[?25l" {
            p.advance(*b, &mut c);
        }
        assert_eq!(c.csi.len(), 1);
        assert!(c.csi[0].private);
        assert_eq!(c.csi[0].final_byte, b'l');
    }

    #[test]
    fn parses_osc() {
        let mut p = Parser::new();
        let mut c = Capture::new();
        for b in b"\x1b]0;title\x07" {
            p.advance(*b, &mut c);
        }
        assert_eq!(c.osc.len(), 1);
        assert_eq!(c.osc[0], b"0;title");
    }

    #[test]
    fn parses_esc() {
        let mut p = Parser::new();
        let mut c = Capture::new();
        for b in b"\x1b(0" {
            p.advance(*b, &mut c);
        }
        assert_eq!(c.esc.len(), 1);
        assert_eq!(c.esc[0], (vec![b'('], b'0'));
    }

    #[test]
    fn utf8_sequences() {
        let mut p = Parser::new();
        let mut c = Capture::new();
        // "é" in UTF-8: 0xC3 0xA9
        p.advance(0xC3, &mut c);
        p.advance(0xA9, &mut c);
        assert_eq!(c.printed, vec!['é']);
    }
}
