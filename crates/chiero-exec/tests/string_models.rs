//! Symbolic string scanning — 024 contract 7.
//!
//! Covers: 024 contract 7.
//!
//! §4 step 2: "At the first byte that *may* be zero, fork: one state with the byte
//! constrained to zero (length known), one with it non-zero, continuing." Step 4: running
//! off the end of the object "is an OOB finding (unterminated string), not a silent stop —
//! this is a real bug class and the most valuable thing these models catch".
//!
//! And the rule that keeps the two from cancelling: "the scan is bounded by
//! `min(max_string_scan, object size)`, reaching the *object's* end is always an OOB
//! finding, and reaching the *scan cap* first adds no constraint. Constraining a
//! terminator to exist is never correct" — an earlier draft did exactly that, and assumed
//! away the bug step 4 exists to find.

use chiero_cir::*;
use chiero_exec::*;
use chiero_model::StringPolicy;
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

/// `char buf[4]` whose bytes are symbolic (an entry parameter's pointee is materialized
/// that way, 021 §6), then `strlen(buf)`.
fn strlen_over_symbolic_bytes() -> Module {
    let strlen = Function {
        id: FuncId(1),
        name: "strlen".into(),
        // Declared with its real signature: the verifier checks a call's argument count
        // against the callee, so a no-param declaration made the module unverifiable and
        // the fixture measured a run that never happened.
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(64),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        // A pointer parameter: 021 §6 materializes its pointee as fully symbolic and
        // fully initialized, which is exactly the input this contract is about.
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(
            0,
            vec![Inst {
                kind: InstKind::Call {
                    dst: Some(ValueId(1)),
                    callee: Callee::Direct(FuncId(1)),
                    args: vec![Operand::Value(ValueId(0))],
                },
                span: Span::DUMMY,
                generated: false,
            }],
            Terminator::Return(Some(i32c(0))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    Module {
        funcs: vec![f, strlen],
        ..Default::default()
    }
}

/// **024 contract 7.** "`strlen` over 4 symbolic bytes in a 4-byte object forks into
/// states with lengths 0,1,2,3 plus one unterminated-string OOB finding."
///
/// Today the scan stops at the first non-concrete byte and reports that it did not
/// establish a length — honest, but it means chiero cannot measure a string it did not
/// write, which is most strings in a real program.
#[test]
fn strlen_over_symbolic_bytes_forks_per_length_and_reports_the_unterminated_case() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = strlen_over_symbolic_bytes();
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_entry_ptr_nullable(false)
        .with_backend(backend)
        .with_entry_param_bytes(4)
        .run(&mut a);

    // One state per possible length, each carrying that length as its result.
    let mut lengths: Vec<u128> = r
        .states()
        .iter()
        .filter_map(|s| match s.local(ValueId(1)) {
            Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits()),
            _ => None,
        })
        .collect();
    lengths.sort_unstable();
    assert_eq!(
        lengths,
        vec![0, 1, 2, 3],
        "a NUL at each of the four positions is four lengths"
    );

    // …plus the case where no byte is NUL, which runs off the end of the object.
    assert_eq!(
        r.findings()
            .iter()
            .filter(|f| f.contains("unterminated"))
            .count(),
        1,
        "exactly one unterminated-string finding: {:#?}",
        r.findings()
    );

    // **Each length is a claim about the bytes**, so each state's path must say so. A
    // fork whose branches carry no constraint is four states that all believe the string
    // could be any length — and a later branch on `len == 2` would take both sides in
    // every one of them.
    for s in r.states() {
        if s.local(ValueId(1)).is_some() {
            assert!(
                !s.path.is_empty(),
                "a length-carrying state constrains the bytes it measured"
            );
        }
    }
}

/// The other half of §4's rule: reaching the **scan cap** adds no constraint and is not a
/// finding, while reaching the **object's end** always is. An earlier spec draft let the
/// cap "constrain a terminator to exist within the bound", which assumes away the
/// unterminated-string bug whenever the object is smaller than the cap.
#[test]
fn the_scan_cap_does_not_manufacture_a_terminator() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = strlen_over_symbolic_bytes();
    let mut a = TermArena::new();
    // A cap *below* the object size: the scan stops early, and nothing is claimed about
    // the bytes past it — no unterminated finding, because chiero did not look.
    let r = Engine::new(&m)
        .with_entry_ptr_nullable(false)
        .with_backend(backend)
        .with_entry_param_bytes(8)
        .with_string_policy(StringPolicy { max_scan: 2 })
        .run(&mut a);
    assert!(
        !r.findings().iter().any(|f| f.contains("unterminated")),
        "the cap was reached first, so nothing is known about the rest: {:#?}",
        r.findings()
    );
    // **`Bounded`, exactly** — 024 §4 step 3 says the cap "terminates the state with
    // `Bounded`", and `!= Exact` was satisfied by `Approximated` and by `Unknown` alike.
    // It *was* `Unknown`, because the state continued with its destination unbound and
    // the first use of the result hit "branch condition is not a scalar". Found by review.
    assert_eq!(
        r.fidelity(),
        Fidelity::Bounded,
        "the scan was cut, and that is a bound rather than a modelling lie: {:#?}",
        r.states()
            .iter()
            .map(|s| (s.fidelity(), &s.status))
            .collect::<Vec<_>>()
    );
    assert!(
        r.states()
            .iter()
            .any(|s| s.status == Status::Terminated(TermReason::Budget)),
        "and the state that reached the cap ends there rather than carrying on with no \
         value: {:#?}",
        r.states().iter().map(|s| &s.status).collect::<Vec<_>>()
    );
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("max_string_scan")),
        "naming the cap: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
}

/// **A terminated string is not an unterminated one.** Review's fixture:
/// `char buf[4]; buf[0]=x; buf[1]=0; buf[2]='b'; buf[3]='c';` — gcc over all 256 values of
/// `x` gives length 0 or 1, never 2 or 3, and never runs off the end. chiero reported four
/// lengths *and* a false out-of-bounds of exactly the class §4 step 4 exists to find,
/// because it emitted branches whose guards were provably false.
#[test]
fn a_concrete_nul_ends_the_string_and_reports_no_overrun() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let strlen = Function {
        id: FuncId(1),
        name: "strlen".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(64),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let store = |off: i128, val: i128, id: u32| {
        vec![
            Inst {
                kind: InstKind::Assign {
                    dst: ValueId(id),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: off }),
                    },
                },
                span: Span::DUMMY,
                generated: false,
            },
            Inst {
                kind: InstKind::Store {
                    addr: Operand::Value(ValueId(id)),
                    val: Operand::Const(Const::Int { bits: 8, val }),
                    ty: CTy::Int(8),
                    align: 1,
                    vol: Volatility::Normal,
                },
                span: Span::DUMMY,
                generated: false,
            },
        ]
    };
    let mut insts = Vec::new();
    // buf[1] = 0, buf[2] = 'b', buf[3] = 'c'. buf[0] is whatever the caller left.
    insts.extend(store(1, 0, 10));
    insts.extend(store(2, 98, 11));
    insts.extend(store(3, 99, 12));
    insts.push(Inst {
        kind: InstKind::Call {
            dst: Some(ValueId(20)),
            callee: Callee::Direct(FuncId(1)),
            args: vec![Operand::Value(ValueId(0))],
        },
        span: Span::DUMMY,
        generated: false,
    });
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(0, insts, Terminator::Return(Some(i32c(0))))],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![f, strlen],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_entry_ptr_nullable(false)
        .with_backend(backend)
        .with_entry_param_bytes(4)
        .run(&mut a);
    let mut lengths: Vec<u128> = r
        .states()
        .iter()
        .filter_map(|s| match s.local(ValueId(20)) {
            Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits()),
            _ => None,
        })
        .collect();
    lengths.sort_unstable();
    lengths.dedup();
    assert_eq!(
        lengths,
        vec![0, 1],
        "byte 1 is a concrete NUL, so the length is 0 or 1 and nothing else"
    );
    assert!(
        !r.findings().iter().any(|f| f.contains("unterminated")),
        "the string is terminated: {:#?}",
        r.findings()
    );
}

/// **A concrete prefix does not disable the fork.** §4 step 1 walks while the byte is
/// provably non-zero and step 2 forks at the first byte that *may* be zero — the prefix is
/// not a reason to stop. Dispatch gated the symbolic fork on the concrete walk having
/// scanned **zero** bytes, so one assigned byte before the caller's data turned the whole
/// analysis off: chiero found neither the length nor the overrun. Found by review.
#[test]
fn a_concrete_prefix_still_forks_at_the_first_symbolic_byte() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let strlen = Function {
        id: FuncId(1),
        name: "strlen".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(64),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let store = |off: i128, val: i128, id: u32| {
        vec![
            Inst {
                kind: InstKind::Assign {
                    dst: ValueId(id),
                    rv: RValue::PtrAdd {
                        base: Operand::Value(ValueId(0)),
                        off: Operand::Const(Const::Int { bits: 64, val: off }),
                    },
                },
                span: Span::DUMMY,
                generated: false,
            },
            Inst {
                kind: InstKind::Store {
                    addr: Operand::Value(ValueId(id)),
                    val: Operand::Const(Const::Int { bits: 8, val }),
                    ty: CTy::Int(8),
                    align: 1,
                    vol: Volatility::Normal,
                },
                span: Span::DUMMY,
                generated: false,
            },
        ]
    };
    // buf[0]='a' (concrete, non-zero), buf[1] from the caller, buf[2]='b', buf[3]='c'.
    let mut insts = Vec::new();
    insts.extend(store(0, 97, 10));
    insts.extend(store(2, 98, 11));
    insts.extend(store(3, 99, 12));
    insts.push(Inst {
        kind: InstKind::Call {
            dst: Some(ValueId(20)),
            callee: Callee::Direct(FuncId(1)),
            args: vec![Operand::Value(ValueId(0))],
        },
        span: Span::DUMMY,
        generated: false,
    });
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![block(0, insts, Terminator::Return(Some(i32c(0))))],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![f, strlen],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_entry_ptr_nullable(false)
        .with_backend(backend)
        .with_entry_param_bytes(4)
        .run(&mut a);
    let lengths: Vec<u128> = r
        .states()
        .iter()
        .filter_map(|s| match s.local(ValueId(20)) {
            Some(Value::Scalar(t)) => a.eval_ground(t).ok().map(|c| c.bits()),
            _ => None,
        })
        .collect();
    assert!(
        lengths.contains(&1),
        "if the caller's byte is NUL the length is 1: {lengths:?}"
    );
    // …and if it is not, the string runs off the end of a 4-byte object.
    assert_eq!(
        r.findings()
            .iter()
            .filter(|f| f.contains("unterminated"))
            .count(),
        1,
        "the overrun is found: {:#?}",
        r.findings()
    );
}

/// **Each forked state's guard is its own, and the guards partition the inputs.**
///
/// The contract-7 fixture has **no branch after the call**, so nothing ever queries the
/// path condition and three mutations lived through it: deleting the `pop_if` that stops a
/// sibling inheriting this state's guard (four of five paths become
/// `both branches infeasible`), dropping the non-zero prefix from each guard (the same
/// input then yields *every* length at once), and building the prefix from `is_zero`
/// instead of its negation (every path with a non-NUL first byte is silently lost). Found
/// by review — the "fixture whose paths agree cannot test a claim about paths" trap.
///
/// So this one branches on the **bytes** afterwards: `if (p[0] == 0)`. Each state must
/// take exactly one side, and exactly the side its length implies.
#[test]
fn each_length_state_agrees_with_a_later_branch_on_the_bytes() {
    let Some(backend) = chiero_solver::SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let strlen = Function {
        id: FuncId(1),
        name: "strlen".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(64),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![
            block(
                0,
                vec![
                    Inst {
                        kind: InstKind::Call {
                            dst: Some(ValueId(1)),
                            callee: Callee::Direct(FuncId(1)),
                            args: vec![Operand::Value(ValueId(0))],
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                    // Re-read the first byte and ask the question the guards answered.
                    Inst {
                        kind: InstKind::Assign {
                            dst: ValueId(2),
                            rv: RValue::Load {
                                addr: Operand::Value(ValueId(0)),
                                ty: CTy::Int(8),
                                align: 1,
                                vol: Volatility::Normal,
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
                                ty: CTy::Int(8),
                                a: Operand::Value(ValueId(2)),
                                b: Operand::Const(Const::Int { bits: 8, val: 0 }),
                            },
                        },
                        span: Span::DUMMY,
                        generated: false,
                    },
                ],
                Terminator::Br {
                    cond: Operand::Value(ValueId(3)),
                    t: BlockId(1),
                    f: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Return(Some(i32c(100)))),
            block(2, vec![], Terminator::Return(Some(i32c(200)))),
        ],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![f, strlen],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_entry_ptr_nullable(false)
        .with_backend(backend)
        .with_entry_param_bytes(4)
        .run(&mut a);

    // **No state is contradictory.** A sibling carrying another branch's guard has an
    // unsatisfiable path, and the engine reports that as "both branches infeasible".
    for s in r.states() {
        assert!(
            !matches!(&s.status, Status::Errored(why) if why.contains("infeasible")),
            "a state's path contradicts itself: {:?}",
            s.status
        );
    }

    // **Each length implies its side of the branch**, and takes only that side.
    let mut seen: Vec<(u128, i128)> = Vec::new();
    for s in r.states() {
        let Some(Value::Scalar(len)) = s.local(ValueId(1)) else {
            continue;
        };
        let (Ok(len), Some(ret)) = (a.eval_ground(len), s.return_value_bits(&mut a)) else {
            continue;
        };
        seen.push((len.bits(), ret as i128));
    }
    assert!(!seen.is_empty(), "the fixture must produce lengths");
    for (len, ret) in &seen {
        let want = if *len == 0 { 100 } else { 200 };
        assert_eq!(
            *ret,
            want,
            "length {len} implies p[0] {} 0, so the branch goes one way",
            if *len == 0 { "==" } else { "!=" }
        );
    }
    // And each length appears once: overlapping guards would give one input two answers.
    let mut lens: Vec<u128> = seen.iter().map(|(l, _)| *l).collect();
    lens.sort_unstable();
    let mut uniq = lens.clone();
    uniq.dedup();
    assert_eq!(lens, uniq, "one state per length: {seen:?}");
}

/// **One defect, one finding.** `strlen` on a pointer before its object was reported
/// twice: the concrete walk reports what it could not do, dispatch then runs the symbolic
/// scan over the same bytes, and that reports the same thing in its own words. 021 c26's
/// "exactly one … finding, not two" is the governing precedent, and the model-level
/// fixture could not see this because it calls `strlen_symbolic` directly, bypassing the
/// dispatch that produces the pair. Found by review.
#[test]
fn a_pointer_before_its_object_is_one_finding_not_two() {
    let strlen = Function {
        id: FuncId(1),
        name: "strlen".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(64),
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(8),
            count: 16,
            align: 1,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: None,
            span: Span::DUMMY,
        }],
        blocks: vec![block(
            0,
            vec![
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(0),
                        rv: RValue::AddrOfLocal {
                            alloca: AllocaId(0),
                        },
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
                // One byte *before* the object.
                Inst {
                    kind: InstKind::Assign {
                        dst: ValueId(1),
                        rv: RValue::PtrAdd {
                            base: Operand::Value(ValueId(0)),
                            off: Operand::Const(Const::Int { bits: 64, val: -1 }),
                        },
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
                Inst {
                    kind: InstKind::Call {
                        dst: Some(ValueId(2)),
                        callee: Callee::Direct(FuncId(1)),
                        args: vec![Operand::Value(ValueId(1))],
                    },
                    span: Span::DUMMY,
                    generated: false,
                },
            ],
            Terminator::Return(Some(i32c(0))),
        )],
        entry: BlockId(0),
        attrs: Default::default(),
        access_paths: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    };
    let m = Module {
        funcs: vec![f, strlen],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert_eq!(
        r.findings()
            .iter()
            .filter(|x| x.contains("before the object"))
            .count(),
        1,
        "one pointer, one defect, one finding: {:#?}",
        r.findings()
    );
}
