//! Which function the engine analyses.
//!
//! 023 does not specify entry selection, so it is an implementation choice — and the one
//! in place until wave 115 was `module.funcs.first()`, whatever the translation unit
//! happened to declare first. For every corpus file that is `chiero_make_symbolic` from
//! `chiero.h`: a `Body::Declared` function with **no blocks**. The engine set `pc` to its
//! nonexistent entry block and every run ended
//! `Errored("no such block BlockId(0)")` before executing one instruction.
//!
//! Nothing caught it for eighteen waves. The goldens compare lowered *text*; an errored
//! state reports no findings, so every "runs clean" assertion was true of a run that never
//! ran. That is the shape this file exists to prevent.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

fn i32c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 32, val: v })
}

fn defined(id: u32, name: &str, ret: i128) -> Function {
    Function {
        id: FuncId(id),
        name: name.into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![],
            term: Terminator::Return(Some(i32c(ret))),
            gcov_lines: Default::default(),
            span: Span::DUMMY,
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        access_paths: Default::default(),
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    }
}

/// A declaration: no blocks, and `funcs.first()` in every real translation unit.
fn declared(id: u32, name: &str) -> Function {
    Function {
        id: FuncId(id),
        name: name.into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Declared,
        access_paths: Default::default(),
        span: Span::DUMMY,
        linkage: chiero_cir::Linkage::External,
    }
}

fn run(m: &Module) -> (Status, Option<u128>) {
    let mut a = TermArena::new();
    let r = Engine::new(m).run(&mut a);
    let s = &r.states()[0];
    (s.status.clone(), s.return_value_bits(&mut a))
}

/// **A leading declaration is not the entry.**
#[test]
fn a_declaration_is_never_the_entry() {
    let m = Module {
        funcs: vec![declared(0, "chiero_make_symbolic"), defined(1, "main", 7)],
        ..Default::default()
    };
    let (status, ret) = run(&m);
    assert!(
        matches!(status, Status::Terminated(TermReason::Return)),
        "the run reached a return rather than erroring on a block that does not exist: \
         {status:?}"
    );
    assert_eq!(ret, Some(7), "and it was `main` that ran");
}

/// **`main` is preferred over an earlier defined function.**
///
/// Without this, "skip declarations" alone would enter `helper` — right by accident for a
/// module whose only defined function is the entry, and wrong for every real program.
#[test]
fn main_is_preferred_over_an_earlier_definition() {
    let m = Module {
        funcs: vec![
            declared(0, "chiero_assume"),
            defined(1, "helper", 1),
            defined(2, "main", 2),
        ],
        ..Default::default()
    };
    assert_eq!(run(&m).1, Some(2), "`main`, not `helper`");
}

/// **With no `main`, the first *defined* function runs** — a library translation unit has
/// no entry point of its own, and refusing to run one would make 021 §6's
/// under-constrained execution impossible.
#[test]
fn without_main_the_first_defined_function_runs() {
    let m = Module {
        funcs: vec![
            declared(0, "extern_thing"),
            defined(1, "first", 5),
            defined(2, "second", 6),
        ],
        ..Default::default()
    };
    assert_eq!(run(&m).1, Some(5));
}

/// **A caller can name the entry**, which is what 021 §6 needs: under-constrained execution
/// starts at each exported function in turn, and only the caller knows which.
#[test]
fn a_named_entry_overrides_the_default() {
    let m = Module {
        funcs: vec![defined(0, "helper", 1), defined(1, "main", 2)],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_entry("helper").run(&mut a);
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(1));

    // A name that is not there falls back rather than erroring: the default is still a
    // valid answer, and a typo should not look like an empty program.
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_entry("nope").run(&mut a);
    assert_eq!(r.states()[0].return_value_bits(&mut a), Some(2));
}

/// A module of **only** declarations still errors cleanly rather than panicking.
#[test]
fn a_module_with_no_definition_errors() {
    let m = Module {
        funcs: vec![declared(0, "a"), declared(1, "b")],
        ..Default::default()
    };
    assert!(matches!(run(&m).0, Status::Errored(_)));
}
