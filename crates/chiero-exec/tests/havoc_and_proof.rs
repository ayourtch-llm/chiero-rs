//! Havoc initialization and the proof surface — 024 contracts 21b and 21e.
//!
//! Covers: 024 contracts 21b, 21d, 21e; 022 contract 11d.
//!
//! Both are about the same failure: a run that knows less than it says. 024 §2.1 puts it
//! plainly — the *modeled* imprecise path is more dangerous than the unmodeled one,
//! "because it looks deliberate".

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn block(id: u32, insts: Vec<Inst>, term: Terminator) -> Block {
    Block {
        id: BlockId(id),
        insts,
        term,
        gcov_lines: Default::default(),
        span: Span::DUMMY,
    }
}

/// A module whose entry calls a declared-but-undefined `name`, passing `argc` pointer
/// arguments taken from its own alloca.
fn calling(name: &str, args: Vec<Operand>) -> Module {
    let mut insts = vec![Inst {
        kind: InstKind::Assign {
            dst: ValueId(0),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        },
        span: Span::DUMMY,
        generated: false,
    }];
    insts.push(Inst {
        kind: InstKind::Call {
            dst: None,
            callee: Callee::Direct(FuncId(1)),
            args,
        },
        span: Span::DUMMY,
        generated: false,
    });
    let caller = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(32),
            count: 4,
            align: 4,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: Span::DUMMY,
        }],
        blocks: vec![block(0, insts, Terminator::Return(Some(i32c(0))))],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let ext = Function {
        id: FuncId(1),
        name: name.into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: true,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
    };
    Module {
        funcs: vec![caller, ext],
        ..Default::default()
    }
}

/// **024 contract 21b.** "A corpus program calling `scanf` cannot produce an `Exact`
/// result, and `seal` returns `NotProven` for it."
///
/// This is the whole proof surface in one case: `scanf` reads the outside world, so a run
/// that touched it has not explored the program — it explored one story about the
/// program. 023 §7.1 makes `seal` the single function that decides, and this is the
/// property that makes chiero safe to hand an LLM: it is *structurally* unable to say "no
/// bugs" about a run like this.
#[test]
fn a_run_that_called_scanf_cannot_be_sealed_as_a_proof() {
    let m = calling("scanf", vec![]);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_ne!(
        r.fidelity(),
        Fidelity::Exact,
        "reading external input is not exact modelling: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
    let w = r.witness();
    match seal(&r, w) {
        Err(np) => {
            assert!(
                !np.assumptions.is_empty(),
                "and it says what stopped it being a proof"
            );
            // **The *modeled* path, not the unmodeled fallback.** `contains("scanf")`
            // matches both this and "`scanf` has no body and no model", so the test could
            // not tell them apart — and 024 §2.1's whole point is that the modeled
            // imprecise path is the more dangerous one, "because it looks deliberate".
            // De-registering `scanf` entirely used to leave this test passing. Found by
            // review.
            assert!(
                np.assumptions.iter().any(|x| {
                    x.kind == AssumptionKind::ModelApproximate
                        && x.detail.contains("scanf")
                        && x.detail.contains("reads external input")
                }),
                "the model ran and said why it is approximate: {:#?}",
                np.assumptions
                    .iter()
                    .map(|x| (&x.kind, &x.detail))
                    .collect::<Vec<_>>()
            );
        }
        Ok(_) => panic!("a run that called `scanf` sealed as a proof"),
    }
    // The rendered report must not sound conclusive either — the sentence is what a
    // reader acts on, and 023 §7's rule 4 is about that sentence.
    let text = render(&r);
    assert!(
        !text.contains("exhaustive") && !text.contains("no bugs exist"),
        "{text}"
    );
}

/// **024 contract 21e.** "`HavocInit::Symbolic` produces no uninitialized-read finding on
/// the havoc'd bytes; `Uninitialized` produces one. The default for an unmodeled extern
/// is `Symbolic`."
///
/// The default is the load-bearing part. An unmodeled extern handed a pointer *wrote*
/// something there — chiero does not know what, which 021 §3.1 insists is not the same as
/// nobody having written it. Defaulting to `Uninitialized` makes every unmodeled call
/// that takes a buffer produce a false uninitialized-read.
#[test]
fn an_unmodeled_externs_havoc_leaves_bytes_symbolic_not_uninitialized() {
    let m = calling("some_unmodeled_thing", vec![Operand::Value(ValueId(0))]);
    // Read the buffer back after the call.
    let mut m = m;
    m.funcs[0].blocks[0].insts.push(Inst {
        kind: InstKind::Assign {
            dst: ValueId(1),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(0)),
                ty: CTy::Int(32),
                align: 4,
                vol: Volatility::Normal,
            },
        },
        span: Span::DUMMY,
        generated: false,
    });
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        !r.findings()
            .iter()
            .any(|f| f.contains("never written") || f.contains("uninitialized")),
        "the callee wrote these bytes; chiero does not know what with: {:#?}",
        r.findings()
    );
    assert!(
        r.states()[0].local(ValueId(1)).is_some(),
        "and the read produced a value, so the assertion above is not vacuous"
    );
    // The call is still recorded as unmodeled — "no uninitialized-read finding" must not
    // be achieved by the call having done nothing.
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("some_unmodeled_thing")),
        "the unmodeled call is named: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
}

/// **022 contract 11d.** "An exact-cache hit degrades the consuming state's fidelity
/// identically to a fresh answer."
///
/// The consumer is the engine: `Engine::feasible` turns a solver answer into a fork
/// decision *and* a degradation, so a cache that changed either would change the explored
/// state space and the reported fidelity — silently, and depending on exploration order.
/// This is why §6 puts the caches below escalation and why the counterexample cache is
/// scoped to the verdict (contract 8b).
///
/// Two identical conditions on one path: the second asks a question the first already
/// answered, so it is served from the exact cache. Both must leave the state in the same
/// condition as if nothing had been cached.
#[test]
fn a_cached_feasibility_answer_degrades_a_state_the_same_way_a_fresh_one_does() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    // `x = fresh; if (x*y == 7) { if (x*y == 7) { ... } }` — the inner test is the same
    // query as the outer one, and nonlinear so it must reach a solver the first time.
    let mk = |cond_block: u32, next: u32| {
        block(
            cond_block,
            vec![],
            Terminator::Br {
                cond: Operand::Value(ValueId(3)),
                t: BlockId(next),
                f: BlockId(9),
            },
        )
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![
            block(
                0,
                vec![
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(0),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(1),
                            rv: RValue::Fresh { ty: CTy::Int(32) },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::Bin {
                                op: BinOp::Mul,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(0)),
                                b: Operand::Value(ValueId(1)),
                                signed: true,
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(3),
                            rv: RValue::Cmp {
                                op: CmpOp::Eq,
                                ty: CTy::Int(32),
                                a: Operand::Value(ValueId(2)),
                                b: i32c(7),
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(3)),
                    t: BlockId(1),
                    f: BlockId(9),
                },
            ),
            mk(1, 2),
            block(2, vec![], Terminator::Return(Some(i32c(0)))),
            block(9, vec![], Terminator::Return(Some(i32c(1)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_backend(backend).run(&mut a);
    // The inner branch asked a question already in the cache — and the run is still
    // `Exact`: a cached answer that degraded, or one that failed to degrade where a
    // fresh answer would have, is the failure 11d names.
    assert_eq!(
        r.fidelity(),
        Fidelity::Exact,
        "nothing here is approximate: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| (&x.kind, &x.detail))
            .collect::<Vec<_>>()
    );
    assert!(
        r.states().iter().all(|s| s.assumptions().is_empty()),
        "and no state carries an assumption from a cache hit"
    );
    // The fixture must actually have asked twice, or it proves nothing about caching.
    assert!(
        r.solver_calls >= 2,
        "the fixture must reach the solver more than once: {}",
        r.solver_calls
    );
}

/// **024 contract 21e's middle clause.** "`HavocInit::Symbolic` produces no
/// uninitialized-read finding on the havoc'd bytes; **`Uninitialized` produces one**."
///
/// Only the first and third clauses were tested, so nothing distinguished the two fills at
/// the point where they differ — the very distinction 021 §3.1 exists to make. Found by
/// review.
#[test]
fn an_uninitialized_havoc_does_produce_a_finding() {
    use chiero_mem::{HavocFill, Memory, ObjKind, Pointer};
    let mut m = Memory::new();
    let mut a = TermArena::new();
    // Aligned to 4: an unaligned object reports `Misaligned` on the read, which is a
    // different fault and would make "the fixture starts clean" false for a reason that
    // has nothing to do with initialization.
    let id = m.alloc(ObjKind::Heap, 4, 4, Span::DUMMY);
    let p = Pointer { base: id, off: 0 };
    // Write the bytes first, so the object is genuinely initialized and the *havoc* is
    // what un-initializes it — otherwise a fresh object would report anyway and the
    // test would pass without the havoc doing anything.
    let v = a.bv(32, 0x1234_5678);
    m.write_term(&mut a, p, v, 4, chiero_mem::Endian::Little, Span::DUMMY);
    assert!(
        m.read_term(&mut a, p, 4, chiero_mem::Endian::Little, Span::DUMMY)
            .faults
            .is_empty(),
        "the fixture must start initialized: {:?}",
        m.read_term(&mut a, p, 4, chiero_mem::Endian::Little, Span::DUMMY)
            .faults
    );

    m.havoc_range_reporting(&mut a, p, 4, HavocFill::Uninitialized, Span::DUMMY);
    let after = m.read_term(&mut a, p, 4, chiero_mem::Endian::Little, Span::DUMMY);
    assert!(
        after
            .faults
            .iter()
            .any(|f| matches!(f, chiero_mem::MemFault::Uninitialized { .. })),
        "an `Uninitialized` havoc leaves bytes nobody has written: {:?}",
        after.faults
    );

    // And the other fill, on the same fixture, does not.
    let mut m2 = Memory::new();
    let mut a2 = TermArena::new();
    let id2 = m2.alloc(ObjKind::Heap, 4, 4, Span::DUMMY);
    let p2 = Pointer { base: id2, off: 0 };
    m2.havoc_range_reporting(&mut a2, p2, 4, HavocFill::Symbolic, Span::DUMMY);
    let sym = m2.read_term(&mut a2, p2, 4, chiero_mem::Endian::Little, Span::DUMMY);
    assert!(
        sym.faults.is_empty(),
        "a `Symbolic` havoc leaves bytes that are unknown, not unwritten: {:?}",
        sym.faults
    );
}

/// **024 contract 21d.** "Default havoc with `reachable_depth: 1` invalidates a pointer
/// stored inside the havoc'd object" — the callee was handed a pointer to a structure that
/// *contains* pointers, and it may have written through those too.
#[test]
fn the_default_havoc_follows_one_level_of_pointers() {
    use chiero_mem::{HavocFill, Memory, ObjKind, Pointer};
    let mut m = Memory::new();
    let mut a = TermArena::new();
    // An outer object holding a pointer to an inner one, both initialized.
    let inner = m.alloc(ObjKind::Heap, 4, 4, Span::DUMMY);
    let outer = m.alloc(ObjKind::Heap, 8, 8, Span::DUMMY);
    let ip = Pointer {
        base: inner,
        off: 0,
    };
    let op = Pointer {
        base: outer,
        off: 0,
    };
    let v = a.bv(32, 0xAAAA_BBBB);
    m.write_term(&mut a, ip, v, 4, chiero_mem::Endian::Little, Span::DUMMY);
    let addr = m.addr_of(inner).expect("placed");
    let at = a.bv(64, addr as u128);
    m.write_term(&mut a, op, at, 8, chiero_mem::Endian::Little, Span::DUMMY);
    // No side table needed: `pointees` finds the inner object by reading the address out
    // of the outer one's bytes, which is what a callee handed the outer pointer would do.
    let _ = ip;

    let hit = m.havoc(&mut a, &[outer], 1, HavocFill::Symbolic, Span::DUMMY);
    assert!(
        hit.objects.contains(&inner),
        "depth 1 follows the pointer the object holds: {hit:?}"
    );
    // Depth 0 does not — the distinction is the contract.
    let mut m2 = Memory::new();
    let mut a2 = TermArena::new();
    let inner2 = m2.alloc(ObjKind::Heap, 4, 4, Span::DUMMY);
    let outer2 = m2.alloc(ObjKind::Heap, 8, 8, Span::DUMMY);
    let hit0 = m2.havoc(&mut a2, &[outer2], 0, HavocFill::Symbolic, Span::DUMMY);
    assert!(
        !hit0.objects.contains(&inner2),
        "depth 0 stops here: {hit0:?}"
    );
}
