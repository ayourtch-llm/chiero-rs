//! The CIR verifier (020 §8).
//!
//! A module that fails verification is never executed, so a missed rule lets malformed
//! IR reach the engine — where the symptom is a confusing wrong answer rather than a
//! clear error.

use crate::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VerifyErrorKind {
    ValueAssignedTwice,
    UseNotDominated,
    UnknownBlock,
    WidthMismatch,
    BadCast,
    BadPointerOperand,
    BadAlignment,
    DuplicateSwitchCase,
    EntryHasPredecessor,
    DeclaredWithBody,
    DefinedWithoutBody,
    BadBitRange,
    BadLane,
    AllocaExtentMismatch,
    /// Rule 3: a *warning*. Unreachable C code exists and is legal.
    UnreachableBlock,
}

impl VerifyErrorKind {
    /// Whether this blocks execution. Only `UnreachableBlock` does not.
    pub fn is_error(self) -> bool {
        self != VerifyErrorKind::UnreachableBlock
    }
}

#[derive(Clone, Debug)]
pub struct VerifyError {
    pub kind: VerifyErrorKind,
    pub func: FuncId,
    pub detail: String,
    pub span: Span,
}

impl VerifyError {
    pub fn is_error(&self) -> bool {
        self.kind.is_error()
    }
}

pub fn verify(_m: &Module) -> Vec<VerifyError> {
    todo!("green")
}
