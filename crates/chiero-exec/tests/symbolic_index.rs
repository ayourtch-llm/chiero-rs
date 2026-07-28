//! A symbolic index into an array.
//!
//! `buf[i]` with symbolic `i` is the first thing anyone asks a symbolic executor to do, and
//! `tests/corpus/c/array_bounds.c` was written to exercise it. Until wave 116 the engine
//! gave up: `chiero_mem::Pointer` carries a **concrete** `i64` offset, so a symbolic one
//! cannot be represented, and `PtrAdd` returned `lowering_gap("PtrAdd with a symbolic
//! offset")` — an invented value, from which three further "not modeled" reports cascaded.
//!
//! The fix is the shape `Switch` already uses for a symbolic scrutinee (020 c14): ask the
//! solver which offsets are **feasible** and fork one state per answer, up to a bound. Each
//! path then has a concrete offset and the memory model is unchanged. Past the bound the
//! run says so rather than picking one.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn i(kind: InstKind) -> Inst {
    Inst {
        kind,
        span: Span::DUMMY,
        generated: false,
    }
}

/// `int buf[4] = {10,20,30,40}; return buf[k];` with `k` an unconstrained parameter
/// narrowed to `0..lanes` by `assume`s the fixture builds as branches.
fn indexed_read(len: u64) -> Module {
    let mut insts = vec![
        // %1 = &buf
        i(InstKind::Assign {
            dst: ValueId(1),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        }),
    ];
    // Initialize every element, so a read of any of them is defined.
    for e in 0..len {
        insts.push(i(InstKind::Assign {
            dst: ValueId(100 + e as u32),
            rv: RValue::PtrAdd {
                base: Operand::Value(ValueId(1)),
                off: Operand::Const(Const::Int {
                    bits: 64,
                    val: e as i128 * 4,
                }),
            },
        }));
        insts.push(i(InstKind::Store {
            addr: Operand::Value(ValueId(100 + e as u32)),
            val: i32c(10 * (e as i128 + 1)),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        }));
    }
    // **The index is masked into range**, which is what makes the offset's feasible set
    // finite. An unconstrained `k` gives `k * 4` unboundedly many values, and bounding
    // *that* is the correct answer — the first version of this fixture forgot the
    // constraint and was measuring the bound rather than the enumeration.
    //
    // `k & (len-1)` rather than a pair of guard branches: same effect on the offset's
    // feasible set, and it keeps the fixture one basic block so the state count is the
    // fork's doing and nothing else's.
    insts.push(i(InstKind::Assign {
        dst: ValueId(5),
        rv: RValue::Bin {
            op: BinOp::And,
            a: Operand::Value(ValueId(0)),
            b: Operand::Const(Const::Int {
                bits: 64,
                val: len as i128 - 1,
            }),
            ty: CTy::Int(64),
        },
    }));
    insts.push(i(InstKind::Assign {
        dst: ValueId(2),
        rv: RValue::Bin {
            op: BinOp::Mul,
            a: Operand::Value(ValueId(5)),
            b: Operand::Const(Const::Int { bits: 64, val: 4 }),
            ty: CTy::Int(64),
        },
    }));
    // %3 = &buf[k]
    insts.push(i(InstKind::Assign {
        dst: ValueId(3),
        rv: RValue::PtrAdd {
            base: Operand::Value(ValueId(1)),
            off: Operand::Value(ValueId(2)),
        },
    }));
    insts.push(i(InstKind::Assign {
        dst: ValueId(4),
        rv: RValue::Load {
            addr: Operand::Value(ValueId(3)),
            ty: CTy::Int(32),
            align: 4,
            vol: Volatility::Normal,
        },
    }));

    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![Param {
                value: ValueId(0),
                ty: CTy::Int(64),
            }],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![AllocaDecl {
                id: AllocaId(0),
                ty: CTy::Int(32),
                count: len,
                align: 4,
                scope: ScopeId(0),
                lifetime: Lifetime::Scope,
                name: Some("buf".into()),
                span: Span::DUMMY,
            }],
            blocks: vec![Block {
                id: BlockId(0),
                insts,
                term: Terminator::Return(Some(Operand::Value(ValueId(4)))),
                gcov_lines: Default::default(),
                span: Span::DUMMY,
            }],
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            access_paths: Default::default(),
            span: Span::DUMMY,
        }],
        ..Default::default()
    }
}

/// Run with a real SMT backend when one is installed.
///
/// **Discovery is a runtime fact** (022 contract 2): the backend is compiled in and finds
/// nothing when no solver is on `PATH`, which is what lets the whole suite run without one.
/// Enumerating a symbolic offset needs more arithmetic than 022 §3.2's tier-1 fragment has
/// — tier 1 answers the first query and returns `Unknown` on the second — so these two
/// tests need a backend and say so when there is none.
fn run_solved(m: &Module) -> Option<(RunResult, TermArena)> {
    let b = chiero_solver::SmtLib::discover()?;
    let mut a = TermArena::new();
    let r = Engine::new(m).with_backend(b).run(&mut a);
    Some((r, a))
}

/// **A symbolic index is explored, not invented.**
///
/// Four elements, so the read has four in-bounds answers plus whatever the out-of-bounds
/// index produces. What must not happen is a single path carrying a value chiero made up:
/// everything computed from an invented value is unsound, and the *absence* of a finding
/// downstream then means nothing.
#[test]
fn a_symbolic_index_does_not_invent_a_value() {
    let m = indexed_read(4);
    assert!(
        verify::verify(&m).iter().all(|e| !e.is_error()),
        "{:?}",
        verify::verify(&m)
    );
    let Some((r, _a)) = run_solved(&m) else {
        eprintln!("skipping a_symbolic_index_does_not_invent_a_value: no SMT solver on PATH");
        return;
    };

    let invented: Vec<String> = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .filter(|x| x.kind == AssumptionKind::NoInformation)
        .map(|x| x.detail.clone())
        .collect();
    assert!(
        invented.is_empty(),
        "the index is symbolic, not unknowable: {invented:?}"
    );
}

/// **Each in-bounds index is reached**, and reads the element that was stored there.
///
/// A pass that forked but resolved every state to the same offset satisfies the test above
/// and is wrong about three of the four paths. The returned values are what separate them.
#[test]
fn every_in_bounds_index_reads_its_own_element() {
    let m = indexed_read(4);
    let Some((r, mut a)) = run_solved(&m) else {
        eprintln!("skipping every_in_bounds_index_reads_its_own_element: no SMT solver on PATH");
        return;
    };

    let mut seen: Vec<u128> = r
        .states()
        .iter()
        .filter_map(|s| s.return_value_bits(&mut a))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    for want in [10u128, 20, 30, 40] {
        assert!(
            seen.contains(&want),
            "no path returned {want}; got {seen:?} — a fork that resolved every state to \
             one offset gives a single value"
        );
    }
}

/// **A symbolic index degrades honestly rather than inventing an address.**
///
/// This is what wave 116 actually delivered, and it is worth separating from what it did
/// not. Before: `lowering_gap("PtrAdd with a symbolic offset")` — `AssumptionKind::
/// NoInformation`, a value chiero *made up*, from which everything downstream is unsound.
/// After: `Fidelity::Bounded` with `BudgetHit`, naming the offset and the reason the
/// enumeration stopped. Neither explores the index; only one of them is honest about it.
#[test]
fn a_symbolic_index_is_bounded_not_invented() {
    let m = indexed_read(4);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let notes: Vec<String> = r
        .states()
        .iter()
        .flat_map(|s| s.assumptions())
        .map(|x| x.detail.clone())
        .collect();
    assert!(
        notes.iter().any(|n| n.contains("symbolic pointer offset")),
        "the run names the offset it could not enumerate: {notes:?}"
    );
    // The *cause* is what changed; the fidelity ends at `Unknown` because the load of the
    // unusable address legitimately degrades further, and `Fidelity::degrade` keeps the
    // worse of the two. Asserting `== Bounded` would be asserting that no cascade happened,
    // which is not the claim.
    assert!(
        r.states().iter().all(|s| s.fidelity() != Fidelity::Exact),
        "the run does not claim to have explored this exactly: {:?}",
        r.states().iter().map(|s| s.fidelity()).collect::<Vec<_>>()
    );
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.kind == AssumptionKind::BudgetHit),
        "and the *first* cause recorded is a bound, not `NoInformation`: {:?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| x.kind)
            .collect::<Vec<_>>()
    );
}

/// **Past the bound, the run says so.**
///
/// A large object has more feasible offsets than it is worth forking on. Concretizing to
/// one of them silently would be a fabricated address on every other path — 021 §7's whole
/// objection — so the state degrades and names the cause instead.
#[test]
fn an_unbounded_index_degrades_rather_than_guessing() {
    let m = indexed_read(4096);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        r.states().iter().any(|s| s.fidelity() != Fidelity::Exact),
        "a symbolic index over 4096 elements is not something chiero explored exactly"
    );
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("index") || x.detail.contains("offset")),
        "and the reason names the offset: {:?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| x.detail.clone())
            .collect::<Vec<_>>()
    );
}
