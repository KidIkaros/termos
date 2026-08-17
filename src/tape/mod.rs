//! Tape scripting — the `.tape` automation language, ported from TUIOS
//! `internal/tape` (lexer, parser, executor, player, recorder, trust).

pub mod command;
pub mod executor;
pub mod header;
pub mod lexer;
pub mod parser;
pub mod player;
pub mod token;
