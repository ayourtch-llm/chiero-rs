//! **Witnesses** — 023 §9.
//!
//! "`Witness` is a concrete assignment for every symbolic input on the path: parameter
//! values, lazily-materialized object contents, extern return values. It is what
//! [040](../../docs/specs/040-defect-checkers.md) turns into a compilable C replay
//! harness, and it is what distinguishes a chiero finding from a plausible-sounding
//! guess."
//!
//! Two things this deliberately does not do. It does not *guess*: an input the model
//! leaves free is marked `pinned: false` rather than quietly bound to zero and presented
//! as the solver's answer. And it does not fabricate an absence: a path with no symbolic
//! inputs is witnessed by the **empty** assignment, because that is a complete answer,
//! where `None` would claim a failure that did not happen and send a reader looking for
//! a solver problem.

use chiero_span::Span;

/// Where a symbolic input came from. Enough to write the replay harness line that
/// supplies it, and enough for a reader to find it in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputOrigin {
    /// A parameter of the entry function, which nothing in the analysis constrains.
    Param {
        index: usize,
        name: String,
        span: Span,
    },
    /// `RValue::Fresh` — a value the CIR itself declared unknown.
    Fresh { span: Span },
    /// A load that produced no value: uninitialized, or bytes chiero cannot read.
    Load { span: Span },
    /// The return value of a call with no body and no model (023 §5).
    ExternReturn { func: String, span: Span },
    /// A model that ran and produced no value (024 §2).
    ModelReturn { func: String, span: Span },
    /// An output of `InstKind::Opaque` — inline asm and friends (020 §4.3).
    Opaque { span: Span },
}

impl InputOrigin {
    /// The site that created the input. What a reader looks at first, and the only field
    /// every variant has.
    pub fn span(&self) -> Span {
        match self {
            InputOrigin::Param { span, .. }
            | InputOrigin::Fresh { span }
            | InputOrigin::Load { span }
            | InputOrigin::ExternReturn { span, .. }
            | InputOrigin::ModelReturn { span, .. }
            | InputOrigin::Opaque { span, .. } => *span,
        }
    }

    /// A short human label, for the rendered report.
    pub fn label(&self) -> String {
        match self {
            InputOrigin::Param { index, name, .. } if name.is_empty() => {
                format!("parameter {index}")
            }
            InputOrigin::Param { name, .. } => format!("parameter `{name}`"),
            InputOrigin::Fresh { .. } => "unknown value".to_string(),
            InputOrigin::Load { .. } => "load with no value".to_string(),
            InputOrigin::ExternReturn { func, .. } => format!("return of extern `{func}`"),
            InputOrigin::ModelReturn { func, .. } => format!("return of modeled `{func}`"),
            InputOrigin::Opaque { .. } => "opaque output".to_string(),
        }
    }
}

/// One input bound to one concrete value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub origin: InputOrigin,
    /// The input's own width in bits. A 32-bit input bound at 64 bits replays as a
    /// different value on a different-endian or differently-promoted target.
    pub width: u32,
    pub value: u128,
    /// Whether the *path* pinned this value. `false` means the model left the input free
    /// and any value replays — worth saying, because a reader who sees a specific number
    /// will otherwise assume the bug needs it.
    pub pinned: bool,
}

/// A concrete assignment for every symbolic input on a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Witness {
    pub bindings: Vec<Binding>,
}

impl Witness {
    /// The complete assignment for a path that has no symbolic inputs.
    pub fn empty() -> Witness {
        Witness {
            bindings: Vec::new(),
        }
    }
}
