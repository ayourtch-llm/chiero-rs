//! `chiero-check` — the defect checkers of 040.
//!
//! Each is a [`chiero_exec::Checker`], registered on an `Engine` and off unless asked for.

use std::any::Any;

use chiero_cir::{InstKind, MarkerKind, Operand, RValue};
use chiero_exec::{Action, Checker, CheckerCtx, CheckerState, Event};
use chiero_mem::ObjectId;
use indexmap::IndexMap;

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
