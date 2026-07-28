//! `chiero-check` — the defect checkers of 040.
//!
//! Each is a [`chiero_exec::Checker`], registered on an `Engine` and off unless asked for.

use std::any::Any;

use chiero_cir::{InstKind, MarkerKind, Operand, RValue};
use chiero_exec::{Action, Checker, CheckerCtx, CheckerState, Event};
use chiero_mem::ObjectId;
use indexmap::IndexMap;

/// The checkers 040 §1 enables unless told otherwise.
///
/// **`union-pun` is not here, and that is the contract** (040 §1, 020 §4.5): reading a
/// member other than the last written is legal, gcc defines it, and VPP is built on it.
/// Enabling it by default would bury every real finding under tens of thousands about code
/// working as designed.
pub fn default_checkers() -> Vec<Box<dyn Checker>> {
    vec![Box::new(OrderDependence::new())]
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
            "union-pun: reading {width} byte(s) at offset {off} of bytes last written as              {w_width} byte(s) at offset {w_off}"
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
