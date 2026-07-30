//! `chiero-check` — the defect checkers of 040.
//!
//! Each is a [`chiero_exec::Checker`], registered on an `Engine` and off unless asked for.

use std::any::Any;

use chiero_cir::{InstKind, MarkerKind, Operand, RValue};
use chiero_exec::{Action, Checker, CheckerCtx, CheckerState, Event, UbKind};
use chiero_mem::ObjectId;
use indexmap::IndexMap;

/// The checkers 040 §1 enables unless told otherwise.
///
/// **`union-pun` is not here, and that is the contract** (040 §1, 020 §4.5): reading a
/// member other than the last written is legal, gcc defines it, and VPP is built on it.
/// Enabling it by default would bury every real finding under tens of thousands about code
/// working as designed.
pub fn default_checkers() -> Vec<Box<dyn Checker>> {
    vec![
        Box::new(OrderDependence::new()),
        Box::new(UndefinedArithmetic::new()),
    ]
}

/// **020 contract 29 / 040 §1** — reading a union member other than the one last written.
///
/// Off by default. C89/C99 call this undefined and gcc defines it; chiero follows gcc, so
/// this checker exists for the projects that want the stricter reading rather than for
/// this one.
///
/// A pun is a read whose bytes were last written **at a different offset or a different
/// width** — the two facts that say the bytes are being reinterpreted rather than read
/// back. Neither alone is enough: same offset and a narrower width is a pun (`as_u8[0]`
/// after `as_u32`), and so is the same width at a shifted offset.
#[derive(Debug, Default)]
pub struct UnionPun;

impl UnionPun {
    pub fn new() -> Self {
        Self
    }
}

/// What was last written to each object, as `(offset, byte width)`.
#[derive(Debug, Default)]
struct PunState {
    writes: IndexMap<(ObjectId, u64), u64>,
}

impl CheckerState for PunState {
    fn on_fork(&self) -> Box<dyn CheckerState> {
        Box::new(PunState {
            writes: self.writes.clone(),
        })
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Checker for UnionPun {
    fn name(&self) -> &'static str {
        "union-pun"
    }

    fn initial_state(&self) -> Box<dyn CheckerState> {
        Box::new(PunState::default())
    }

    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        let Event::BeforeInst { st: exec, inst } = ev else {
            return vec![];
        };
        let (addr, ty, is_write) = match &inst.kind {
            InstKind::Store { addr, ty, .. } => (addr, ty, true),
            InstKind::Assign {
                rv: RValue::Load { addr, ty, .. },
                ..
            } => (addr, ty, false),
            _ => return vec![],
        };
        let Operand::Value(v) = addr else {
            return vec![];
        };
        let Some(chiero_exec::Value::Ptr(p)) = exec.local(*v) else {
            return vec![];
        };
        let Some(width) = ty.bit_width().map(|b| u64::from(b).div_ceil(8)) else {
            return vec![];
        };
        let off = p.off.max(0) as u64;

        if is_write {
            cx.state_mut::<PunState>()
                .writes
                .insert((p.base, off), width);
            return vec![];
        }

        // **A read with no overlapping write is not a pun.** Nothing was reinterpreted;
        // it is an uninitialized read, which a different checker owns. A checker that
        // reported every load it could not attribute would mislabel all of them.
        let st = cx.state_mut::<PunState>();
        let overlapping: Option<(u64, u64)> = st
            .writes
            .iter()
            .find(|((obj, w_off), w_width)| {
                *obj == p.base && *w_off < off + width && off < *w_off + **w_width
            })
            .map(|((_, w_off), w_width)| (*w_off, *w_width));
        let Some((w_off, w_width)) = overlapping else {
            return vec![];
        };
        if w_off == off && w_width == width {
            // Read back exactly what was written: an ordinary read, however many members
            // the type has.
            return vec![];
        }
        vec![Action::report(format!(
            "union-pun: reading {width} byte(s) at offset {off} of bytes last written as \
             {w_width} byte(s) at offset {w_off}"
        ))]
    }
}

/// **020 §7 / contract 18(b)** — the interprocedural half of order sensitivity.
///
/// C leaves the evaluation order of subexpressions unspecified; CIR picks one and writes
/// it down. `chiero-lower` flags the cases decidable from syntax alone; this one starts
/// where that stops. `f(g(), h())` where `g` and `h` both mutate a shared global needs
/// side-effect summaries and "is this the same object?" — questions 001 §2 forbids
/// lowering to answer and the memory model can.
///
/// The input is CIR with `SeqPoint` markers. Between two of them lies one **unsequenced
/// region**; a conflict is two *different* calls in one region writing one object. Two
/// writes by the same call are not a conflict — a function's own statements are sequenced
/// — and that distinction is why writes are keyed by call site rather than merely counted.
#[derive(Debug, Default)]
pub struct OrderDependence;

impl OrderDependence {
    pub fn new() -> Self {
        Self
    }
}

/// Per-path memory: where the current region is and what has been written in it.
#[derive(Debug, Default)]
struct OrderState {
    /// Nesting depth of calls. Only depth 0 → 1 opens a new call *site*: a callee's own
    /// inner calls are sequenced by its own statements and are all one side effect from
    /// the caller's point of view.
    depth: u32,
    /// The site currently executing, if any.
    site: Option<u64>,
    next_site: u64,
    /// Object → the call site that wrote it, within the current region.
    written: IndexMap<ObjectId, u64>,
    /// Objects already reported in this region, so one conflict is one finding however
    /// many times the two calls write.
    reported: Vec<ObjectId>,
    /// Object → a readable name, learned from `AddrOfGlobal`. A finding that cited an
    /// `ObjectId` would be naming an engine-internal counter.
    names: IndexMap<ObjectId, String>,
}

impl CheckerState for OrderState {
    fn on_fork(&self) -> Box<dyn CheckerState> {
        Box::new(OrderState {
            depth: self.depth,
            site: self.site,
            next_site: self.next_site,
            written: self.written.clone(),
            reported: self.reported.clone(),
            names: self.names.clone(),
        })
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Checker for OrderDependence {
    fn name(&self) -> &'static str {
        "order-dependence"
    }

    fn initial_state(&self) -> Box<dyn CheckerState> {
        Box::new(OrderState::default())
    }

    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        match ev {
            Event::Call { .. } => {
                let st = cx.state_mut::<OrderState>();
                if st.depth == 0 {
                    st.site = Some(st.next_site);
                    st.next_site += 1;
                }
                st.depth += 1;
                vec![]
            }
            Event::CallReturn { .. } => {
                let st = cx.state_mut::<OrderState>();
                st.depth = st.depth.saturating_sub(1);
                if st.depth == 0 {
                    st.site = None;
                }
                vec![]
            }
            // **The name is learned after the instruction, not before.** Provenance for a
            // `dst` is recorded by executing the instruction, so `BeforeInst` would read
            // the value the local held on a previous iteration — or nothing at all.
            Event::AfterInst { st: exec, inst } => {
                if let InstKind::Assign {
                    dst,
                    rv: RValue::AddrOfGlobal { g },
                } = &inst.kind
                    && let Some(chiero_exec::Value::Ptr(p)) = exec.local(*dst)
                {
                    let name = cx
                        .module()
                        .globals
                        .iter()
                        .find(|x| x.id == *g)
                        .map(|x| x.name.to_string())
                        .unwrap_or_else(|| format!("global {}", g.0));
                    cx.state_mut::<OrderState>().names.insert(p.base, name);
                }
                vec![]
            }
            Event::BeforeInst { st: exec, inst } => match &inst.kind {
                // A sequence point closes the region: everything before it is sequenced
                // before everything after, so nothing across one can race.
                InstKind::Marker(MarkerKind::SeqPoint) => {
                    let st = cx.state_mut::<OrderState>();
                    // Only at the top level. A `SeqPoint` *inside* a callee separates that
                    // callee's own statements and says nothing about the caller's region —
                    // clearing on it would forget the first call's writes and lose every
                    // conflict with a callee that contains a full expression of its own,
                    // which is every real callee.
                    if st.depth == 0 {
                        st.written.clear();
                        st.reported.clear();
                    }
                    vec![]
                }
                InstKind::Store {
                    addr: Operand::Value(v),
                    ..
                } => {
                    // **The local's value, not `value_provenance_of`.** That map records
                    // only `PtrToInt` casts — a pointer that has been through integer
                    // arithmetic — so it is empty for an ordinary address, which is every
                    // address a store actually uses.
                    let Some(chiero_exec::Value::Ptr(p)) = exec.local(*v) else {
                        return vec![];
                    };
                    let st = cx.state_mut::<OrderState>();
                    let Some(site) = st.site else {
                        // A write outside any call is the caller's own, and the syntactic
                        // half already owns those.
                        return vec![];
                    };
                    match st.written.get(&p.base) {
                        Some(&prev) if prev != site && !st.reported.contains(&p.base) => {
                            st.reported.push(p.base);
                            let what = st
                                .names
                                .get(&p.base)
                                .cloned()
                                .unwrap_or_else(|| format!("{:?}", p.base));
                            vec![Action::report(format!(
                                "order-dependence: two calls in one unsequenced region \
                                 both write `{what}`, and C does not say which happens \
                                 first"
                            ))]
                        }
                        Some(_) => vec![],
                        None => {
                            st.written.insert(p.base, site);
                            vec![]
                        }
                    }
                }
                _ => vec![],
            },
            _ => vec![],
        }
    }
}

/// **020 §4.1** — the checker that turns the engine's UB events into findings.
///
/// §4.1 divides the work deliberately: "CIR is not the place to encode UB as
/// unpredictability: the semantics are defined and total, and a `Checker` observes the
/// overflow event and reports it." The engine computes the value C leaves undefined,
/// records what happened, and **continues** — an earlier draft stopped the path on division
/// alone, "hiding everything downstream of it for no reason the other cases don't share".
/// Everything after the event is this checker's side of the line, and until wave 157 nobody
/// stood on it: the events were recorded and `reports()` was empty.
///
/// # Why this needs per-path memory rather than a counter
///
/// Two different duplications have to be suppressed and they are not the same problem.
///
/// **The path continues past the fault**, so on every subsequent instruction the state
/// still carries the event. A checker asking "does this state have a UB event?" reports
/// once per instruction from the division onward.
///
/// **A loop runs one site many times**, and 023 §6.1 says that is one finding. The engine
/// cannot help: `Action::Report` carries no §6.1 key, so `RunResult::reports` deduplicates
/// a *fork's* copies by report id and leaves the rest to the checker.
///
/// `reported` — keyed by kind and span — answers **both**, and that is worth being precise
/// about, because an earlier version of this comment credited the cursor with the first.
/// Mutation said otherwise: freezing the cursor changed no test, since the same event
/// re-read on the next instruction has the same key and is suppressed anyway. The cursor
/// is therefore a **scan bound and nothing more** — without it every instruction after the
/// first fault re-reads the whole log, which is quadratic in a long path and correct.
/// Keeping it is a performance decision, and it is documented as one rather than
/// re-mutated in the hope of a different answer.
///
/// Both live in [`UbState`], which is cloned at a fork, so two paths that reach the same
/// site each report it. That is deliberate and matches the memory checkers: two paths that
/// fault differently are two reports, and collapsing them throws away a witness.
#[derive(Debug, Default)]
pub struct UndefinedArithmetic;

impl UndefinedArithmetic {
    pub fn new() -> Self {
        Self
    }
}

/// Per-path memory: how much of the state's UB log has been read, and what has been said.
#[derive(Debug, Default)]
struct UbState {
    /// How many of `State::ub_events` this path has already considered. The log is
    /// append-only, so a cursor is enough to find what is new.
    cursor: usize,
    /// `(kind, span)` of everything already reported on this path, for 023 §6.1's
    /// "one site, one finding" across a loop.
    reported: Vec<(UbKind, chiero_span::Span)>,
}

impl CheckerState for UbState {
    fn on_fork(&self) -> Box<dyn CheckerState> {
        Box::new(UbState {
            cursor: self.cursor,
            reported: self.reported.clone(),
        })
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// How a kind reads in a finding.
///
/// Spelled out rather than derived from `Debug`: `Shl` is what the operator is called and
/// "shift" is what went wrong, and a reader looking for the second should not have to know
/// the first. The operation itself is in the event's own detail, which the message carries.
fn ub_phrase(kind: UbKind) -> &'static str {
    match kind {
        UbKind::DivByZero => "division by zero",
        UbKind::Shift => "shift past the operand width",
        UbKind::SignedOverflow => "signed overflow",
        // **The compiler asked for this.** Wave 169 removed the catch-all from the engine's
        // `cmp` on the argument that a new variant should be a compile error rather than a
        // silent fallthrough; the same holds here, and adding `FloatCastOverflow` in the
        // engine failed this match rather than producing a finding with no name.
        UbKind::FloatCastOverflow => "float-to-integer conversion out of range",
    }
}

impl Checker for UndefinedArithmetic {
    fn name(&self) -> &'static str {
        "undefined-arithmetic"
    }

    fn initial_state(&self) -> Box<dyn CheckerState> {
        Box::new(UbState::default())
    }

    fn on_event(&mut self, ev: &Event, cx: &mut CheckerCtx) -> Vec<Action> {
        // **`AfterInst`, not `BeforeInst`.** The event is recorded while the instruction
        // executes, so it exists only once the instruction is done.
        let Event::AfterInst { st, .. } = ev else {
            return Vec::new();
        };
        // Read the log before touching the checker's own state: `cx.state_mut` borrows the
        // context, and the events belong to the engine's state.
        #[allow(clippy::type_complexity)]
        let fresh: Vec<(UbKind, chiero_span::Span, String, Vec<chiero_solver::Term>)> = {
            let seen = cx.state_mut::<UbState>().cursor;
            let all = st.ub_events();
            if all.len() <= seen {
                return Vec::new();
            }
            all[seen..]
                .iter()
                .map(|u| (u.kind, u.span, u.detail.clone(), u.requires.clone()))
                .collect()
        };
        let total = st.ub_events().len();
        let mem = cx.state_mut::<UbState>();
        mem.cursor = total;
        let mut out = Vec::new();
        for (kind, span, detail, requires) in fresh {
            if mem.reported.contains(&(kind, span)) {
                continue;
            }
            mem.reported.push((kind, span));
            let message = format!("{}: {detail}", ub_phrase(kind));
            // **Pass the event's condition through.** The checker is the only thing that
            // knows which event a given report is about, so it is the only place the two
            // can be joined — and without the join the witness is solved against the path
            // alone and names an input under which nothing faults (023 §9).
            out.push(if requires.is_empty() {
                Action::report(message)
            } else {
                Action::report_requiring(message, requires)
            });
        }
        out
    }
}
