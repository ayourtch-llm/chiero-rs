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
    ///
    /// `seq` is the call's **index in the state's effect sequence**, which is what makes two
    /// runs' extern returns matchable (041 §1.2). It has to be the sequence position and not
    /// "the nth call to this function": the return input exists only for a call that has a
    /// destination, so a discarded result would shift a by-name numbering and equate one
    /// run's `p(2)` with the other's `p(1)`. Every declared call is in the sequence,
    /// including `pure` ones, precisely so this ordinal counts one thing.
    ExternReturn {
        func: String,
        span: Span,
        seq: usize,
    },
    /// A model that ran and produced no value (024 §2). `seq` as for `ExternReturn`.
    ModelReturn {
        func: String,
        span: Span,
        seq: usize,
    },
    /// An output of `InstKind::Opaque` — inline asm and friends (020 §4.3).
    Opaque { span: Span },
    /// A `Volatility::Volatile` load — a device register, whose value the program did
    /// not compute and the replay harness must supply (020 §4.2).
    Volatile { span: Span },
    /// A byte `chiero-mem` invented: lazily-materialized contents, or memory clobbered by
    /// code with no model. 023 §9 lists these among what a witness must bind, and the
    /// engine cannot see them being created — memory reports them.
    Memory { why: &'static str, span: Span },
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
            | InputOrigin::Opaque { span, .. }
            | InputOrigin::Volatile { span }
            | InputOrigin::Memory { span, .. } => *span,
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
            InputOrigin::Volatile { .. } => "volatile read".to_string(),
            InputOrigin::Memory { why, .. } => (*why).to_string(),
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

    /// **What a report may print, and what it must say about the rest** — 023 §9.
    ///
    /// A witness is *a concrete input someone can re-run*, and past some length it stops being
    /// one: `find-bugs` on VPP's `nsh_md2_encap` produced 10 658 bindings, 10 657 of them the
    /// same anonymous "a lazily-materialized byte", for 950 KB of JSON describing one finding.
    /// Under UCSE an entry that walks a packet buffer materialises a byte at a time, so that is
    /// the execution working; it is the rendering that has stopped answering the question.
    ///
    /// **Pinned bindings first, and that is not a preference.** Measured on a fixture of *n*
    /// loads followed by a division: at n = 8, 40 and 200 the pinned bindings are always the
    /// final four — the divisor's bytes, constrained to zero. Everything before them is what the
    /// walk happened to touch, and the model left it free. A bound that took the first *k* would
    /// therefore drop every value the finding depends on and keep *k* that it does not.
    ///
    /// Order is preserved within each group, so a reader sees the pinned inputs in the order the
    /// path met them.
    ///
    /// ⚠️ **This is for rendering only.** `chiero-replay` consumes the `Witness` itself, and a
    /// caller that treats bindings positionally — an argument list — must not be handed a digest;
    /// see `harness_signature_objection`, which refuses a witness that is not an argument list
    /// rather than reordering one.
    pub fn digest(&self, limit: usize) -> WitnessDigest<'_> {
        // **Nothing is reordered when nothing is dropped.** The order bindings arrive in is the
        // order the path met them, which is what a reader follows; rearranging it to put pinned
        // inputs first is a cost worth paying only when the alternative is dropping them.
        if self.bindings.len() <= limit {
            return WitnessDigest {
                shown: self.bindings.iter().collect(),
                omitted: 0,
                omitted_by_label: Vec::new(),
            };
        }
        let (pinned, free): (Vec<&Binding>, Vec<&Binding>) =
            self.bindings.iter().partition(|b| b.pinned);
        let shown: Vec<&Binding> = pinned.into_iter().chain(free).take(limit).collect();
        // **Counted by identity, not by value**: two bindings may be equal and still be two
        // inputs, and a reader asking "how many were left out" is asking about inputs.
        let mut omitted_by_label: Vec<(String, usize)> = Vec::new();
        let mut omitted = 0usize;
        for b in &self.bindings {
            if shown.iter().any(|s| std::ptr::eq(*s, b)) {
                continue;
            }
            omitted += 1;
            let label = b.origin.label();
            match omitted_by_label.iter_mut().find(|(l, _)| *l == label) {
                Some((_, n)) => *n += 1,
                None => omitted_by_label.push((label, 1)),
            }
        }
        omitted_by_label.sort_by_key(|(l, n)| (std::cmp::Reverse(*n), l.clone()));
        WitnessDigest {
            shown,
            omitted,
            omitted_by_label,
        }
    }

    /// A witness that pins the entry function's parameters to concrete values, in order.
    ///
    /// 023 contract 21 replays "with all inputs concretized", and this is what that means
    /// for the common case. The bindings are consumed **positionally** by the engine, so
    /// the origins here are descriptive rather than load-bearing — which is why the span
    /// is `DUMMY` rather than a fabricated location (010 §4 forbids inventing one).
    pub fn concrete(values: Vec<(u32, u128)>) -> Witness {
        Witness {
            bindings: values
                .into_iter()
                .enumerate()
                .map(|(index, (width, value))| Binding {
                    origin: InputOrigin::Param {
                        index,
                        name: format!("arg{index}"),
                        span: Span::DUMMY,
                    },
                    width,
                    value,
                    pinned: true,
                })
                .collect(),
        }
    }
}

/// As much of a witness as a report prints, plus an account of the rest — see [`Witness::digest`].
///
/// **The omission is part of the report, not a detail of it.** A quietly shortened witness reads
/// as the whole input, which is worse than a long one: a reader who cannot see that 10 000 inputs
/// were dropped has no way to know the thing they are looking at will not reproduce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WitnessDigest<'a> {
    /// Pinned bindings first, then free ones, each in the order the path met them.
    pub shown: Vec<&'a Binding>,
    pub omitted: usize,
    /// What was left out, by [`InputOrigin::label`], most numerous first.
    ///
    /// The label is what makes the omission actionable: "10 593 lazily-materialized bytes" tells
    /// a reader the finding does not turn on them, and "10 593 parameters" would tell them the
    /// opposite.
    pub omitted_by_label: Vec<(String, usize)>,
}
