//! The textual `.cir` format (020 §6).
//!
//! Normative, not a debugging convenience: every M1 core fixture is a `.cir` file, so
//! `print` must canonicalize and `parse` must reject anything it does not understand.
//! Silent tolerance here produces tests that pass by not testing anything.

use crate::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based.
    pub line: u32,
    pub message: String,
}

pub fn parse(_src: &str) -> Result<Module, ParseError> {
    todo!("green")
}

pub fn print(_m: &Module) -> String {
    todo!("green")
}
