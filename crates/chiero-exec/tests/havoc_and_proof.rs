//! Havoc initialization and the proof surface — 024 contracts 21b and 21e.
//!
//! Covers: 024 contracts 21b, 21e.
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
    }];
    insts.push(Inst {
        kind: InstKind::Call {
            dst: None,
            callee: Callee::Direct(FuncId(1)),
            args,
        },
        span: Span::DUMMY,
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
            assert!(
                np.assumptions.iter().any(|x| x.detail.contains("scanf")),
                "naming the model: {:#?}",
                np.assumptions.iter().map(|x| &x.detail).collect::<Vec<_>>()
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
