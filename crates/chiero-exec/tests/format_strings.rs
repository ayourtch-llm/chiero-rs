//! Format-string checking — 024 contract 22.
//!
//! Covers: 024 contract 22.
//!
//! "`printf("%d", p)` where `p` is a pointer produces exactly one format-mismatch finding;
//! `printf` with an invalid pointer argument produces one memory finding."
//!
//! Two different bugs behind one call, and the distinction is the point: the first is the
//! program lying to `printf` about what it is passing, the second is the program handing
//! it memory it may not read. A model that reported one for the other would send a reader
//! to the wrong line.

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

fn printf_decl() -> Function {
    Function {
        id: FuncId(1),
        name: "printf".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(32),
        variadic: true,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
    }
}

/// Builds `printf(fmt, arg)` where `fmt` is a string literal in a local buffer.
fn call_printf(fmt: &str, extra: Vec<Inst>, args: Vec<Operand>) -> Module {
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
    // Write the format string byte by byte, terminator included.
    for (i, b) in fmt.bytes().chain(std::iter::once(0)).enumerate() {
        insts.push(Inst {
            kind: InstKind::Assign {
                dst: ValueId(100 + i as u32),
                rv: RValue::PtrAdd {
                    base: Operand::Value(ValueId(0)),
                    off: Operand::Const(Const::Int {
                        bits: 64,
                        val: i as i128,
                    }),
                },
            },
            span: Span::DUMMY,
            generated: false,
        });
        insts.push(Inst {
            kind: InstKind::Store {
                addr: Operand::Value(ValueId(100 + i as u32)),
                val: Operand::Const(Const::Int {
                    bits: 8,
                    val: b as i128,
                }),
                ty: CTy::Int(8),
                align: 1,
                vol: Volatility::Normal,
            },
            span: Span::DUMMY,
            generated: false,
        });
    }
    insts.extend(extra);
    let mut all = vec![Operand::Value(ValueId(0))];
    all.extend(args);
    insts.push(Inst {
        kind: InstKind::Call {
            dst: None,
            callee: Callee::Direct(FuncId(1)),
            args: all,
        },
        span: Span::DUMMY,
        generated: false,
    });
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Int(32),
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(8),
            count: 32,
            align: 1,
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
    Module {
        funcs: vec![f, printf_decl()],
        ..Default::default()
    }
}

fn run(m: &Module) -> Vec<String> {
    let mut a = TermArena::new();
    Engine::new(m).run(&mut a).findings()
}

/// **024 contract 22, first half.** `printf("%d", p)` with a pointer argument is exactly
/// one format-mismatch finding. On a 64-bit target `%d` consumes four bytes of an eight
/// byte argument, so the output is wrong and, for `%s`, the read is wild.
#[test]
fn a_pointer_passed_to_a_d_conversion_is_one_format_mismatch() {
    let m = call_printf("%d", vec![], vec![Operand::Value(ValueId(0))]);
    let f = run(&m);
    let mismatches: Vec<_> = f.iter().filter(|x| x.contains("format")).collect();
    assert_eq!(
        mismatches.len(),
        1,
        "exactly one, naming the conversion: {f:#?}"
    );
    assert!(
        mismatches[0].contains("%d"),
        "and which conversion it was: {}",
        mismatches[0]
    );
}

/// The negative half, which is what stops this becoming noise: a conversion that *matches*
/// its argument reports nothing. An unquantified "produces findings" is satisfied by a
/// checker that fires on everything.
#[test]
fn a_matching_conversion_reports_nothing() {
    let m = call_printf("%d", vec![], vec![i32c(7)]);
    let f = run(&m);
    assert!(
        !f.iter().any(|x| x.contains("format")),
        "a matching conversion is correct C: {f:#?}"
    );
}

/// Too few arguments is the same class of bug and the more dangerous one: `printf("%d")`
/// reads whatever the calling convention left where an argument should be.
#[test]
fn a_conversion_with_no_argument_is_reported() {
    let m = call_printf("%d %d", vec![], vec![i32c(7)]);
    let f = run(&m);
    assert_eq!(
        f.iter().filter(|x| x.contains("format")).count(),
        1,
        "one finding for the conversion that has no argument: {f:#?}"
    );
}

/// **024 contract 22, second half.** "`printf` with an invalid pointer argument produces
/// one **memory** finding" — a different bug from a mismatch, and it must not be reported
/// as one: `%s` of a null pointer is a dereference, not a type error.
#[test]
fn a_null_pointer_to_a_string_conversion_is_a_memory_finding() {
    let m = call_printf("%s", vec![], vec![Operand::Const(Const::Null)]);
    let f = run(&m);
    assert!(
        f.iter()
            .any(|x| x.contains("null") || x.contains("NULL") || x.contains("memory")),
        "reading a string through a null pointer is a memory bug: {f:#?}"
    );
    assert!(
        !f.iter().any(|x| x.contains("format")),
        "and not a format mismatch — `%s` did want a pointer: {f:#?}"
    );
}

/// `%%` is an escape, not a conversion: it consumes no argument. Treating it as one makes
/// every progress message in a codebase — `"100%% done"` — a false "conversion with no
/// argument", which is the kind of noise that gets a checker turned off.
#[test]
fn a_literal_percent_consumes_no_argument() {
    let m = call_printf("100%% done", vec![], vec![]);
    let f = run(&m);
    assert!(
        !f.iter().any(|x| x.contains("format")),
        "`%%` is a literal percent sign: {f:#?}"
    );
    // And one real conversion after it still lines up with its argument.
    let m2 = call_printf("%d%%", vec![], vec![i32c(50)]);
    assert!(
        !run(&m2).iter().any(|x| x.contains("format")),
        "the argument belongs to `%d`, not to `%%`"
    );
}

/// **The false positives review found by compiling the same programs with
/// `gcc -Wformat`.** Every call here is correct C that gcc accepts silently, and each one
/// was reported as a mismatch. A format checker that fires on correct code is a format
/// checker that gets turned off.
#[test]
fn correct_calls_gcc_accepts_are_not_reported() {
    // `%*s` — the width is an argument. Skipping `*` as punctuation left every later
    // argument off by one, so the `%s` saw the width and called it a mismatch.
    let m = call_printf("%*s", vec![], vec![i32c(5), Operand::Value(ValueId(0))]);
    assert!(
        !run(&m).iter().any(|x| x.contains("format")),
        "`printf(\"%*s\", w, s)` is ubiquitous column formatting: {:#?}",
        run(&m)
    );

    // `%.*f` — precision too.
    let m = call_printf("%.*d", vec![], vec![i32c(2), i32c(7)]);
    assert!(
        !run(&m).iter().any(|x| x.contains("format")),
        "{:#?}",
        run(&m)
    );

    // glibc's `%m` takes no argument at all.
    let m = call_printf("failed: %m", vec![], vec![]);
    assert!(
        !run(&m).iter().any(|x| x.contains("format")),
        "`%m` is `strerror(errno)`: {:#?}",
        run(&m)
    );

    // A conversion this checker does not know claims nothing. `%U` is VPP's own
    // `format()` extension and its argument really is a function pointer.
    let m = call_printf("%U", vec![], vec![Operand::Value(ValueId(0))]);
    assert!(
        !run(&m).iter().any(|x| x.contains("format")),
        "an unknown conversion makes no claim: {:#?}",
        run(&m)
    );

    // Length modifiers change width, not kind.
    let m = call_printf("%zu %ld", vec![], vec![i32c(1), i32c(2)]);
    assert!(
        !run(&m).iter().any(|x| x.contains("format")),
        "{:#?}",
        run(&m)
    );
}

/// Positional conversions are **declined**, not guessed at: `%2$d %1$s` reorders the
/// arguments, and consuming them in order reported two mismatches on a call gcc accepts.
/// Half-understanding a format produces findings about chiero's parser.
#[test]
fn positional_conversions_are_declined_rather_than_misread() {
    let m = call_printf(
        "%2$d %1$s",
        vec![],
        vec![Operand::Value(ValueId(0)), i32c(7)],
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    assert!(
        !r.findings().iter().any(|x| x.contains("format-mismatch")),
        "no mismatch is claimed: {:#?}",
        r.findings()
    );
    // **And it says it declined.** "No mismatch" alone is satisfied by a parser that
    // misreads `%2$d` as an unknown conversion and claims nothing — which is the wrong
    // answer arrived at by luck. The decline has to be recorded.
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("positional")),
        "the check declined and said so: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
}

/// A length modifier changes an argument's width, not its kind — so the conversion letter
/// after it is what decides, and `%ld` of a pointer is still a mismatch. Not skipping the
/// modifier makes `l` itself look like an unknown conversion, which claims nothing and
/// hides every mismatch behind a modifier.
#[test]
fn a_length_modifier_does_not_hide_the_conversion() {
    let m = call_printf("%ld", vec![], vec![Operand::Value(ValueId(0))]);
    let f = run(&m);
    assert!(
        f.iter().any(|x| x.contains("format-mismatch")),
        "`%ld` of a pointer is a mismatch, modifier or not: {f:#?}"
    );
}

/// **`%p` and `%n` want pointers**, and nothing pinned that — both classifications could
/// be flipped with the suite still green.
#[test]
fn p_and_n_want_pointers() {
    for conv in ["%p", "%n"] {
        let m = call_printf(conv, vec![], vec![i32c(7)]);
        let f = run(&m);
        assert!(
            f.iter().any(|x| x.contains("format-mismatch")),
            "`{conv}` of an integer is a mismatch: {f:#?}"
        );
    }
}

/// **A `%s` string chiero cannot read is chiero's gap, not the program's bug.**
///
/// Since 021 §6 fills every entry pointee with symbols, `printf("%s", p)` on a pointer
/// parameter — the most common `printf` call there is — has symbolic bytes. Reporting
/// that as a memory finding is the confusion 023 §7 exists to prevent, and it is the same
/// rule this model already applies to an unreadable *format* string. Nothing pinned it:
/// the review found the `Bounded`-not-a-finding rule, the checker's headline design
/// decision, surviving deletion.
#[test]
fn a_symbolic_string_argument_is_a_bound_not_a_finding() {
    let printf = Function {
        id: FuncId(1),
        name: "printf".into(),
        params: vec![Param {
            value: ValueId(0),
            ty: CTy::Ptr,
        }],
        ret: CTy::Int(32),
        variadic: true,
        allocas: vec![],
        blocks: vec![],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Declared,
        span: Span::DUMMY,
    };
    // `void f(char *p) { printf("%s", p); }` — the format is a literal in a local, the
    // argument is the caller's buffer.
    let mut insts = vec![Inst {
        kind: InstKind::Assign {
            dst: ValueId(1),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        },
        span: Span::DUMMY,
        generated: false,
    }];
    for (i, b) in "%s".bytes().chain(std::iter::once(0)).enumerate() {
        insts.push(Inst {
            kind: InstKind::Assign {
                dst: ValueId(100 + i as u32),
                rv: RValue::PtrAdd {
                    base: Operand::Value(ValueId(1)),
                    off: Operand::Const(Const::Int {
                        bits: 64,
                        val: i as i128,
                    }),
                },
            },
            span: Span::DUMMY,
            generated: false,
        });
        insts.push(Inst {
            kind: InstKind::Store {
                addr: Operand::Value(ValueId(100 + i as u32)),
                val: Operand::Const(Const::Int {
                    bits: 8,
                    val: b as i128,
                }),
                ty: CTy::Int(8),
                align: 1,
                vol: Volatility::Normal,
            },
            span: Span::DUMMY,
            generated: false,
        });
    }
    insts.push(Inst {
        kind: InstKind::Call {
            dst: None,
            callee: Callee::Direct(FuncId(1)),
            args: vec![Operand::Value(ValueId(1)), Operand::Value(ValueId(0))],
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
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(8),
            count: 8,
            align: 1,
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
    let m = Module {
        funcs: vec![f, printf],
        ..Default::default()
    };
    let mut a = TermArena::new();
    let r = Engine::new(&m).with_entry_param_bytes(8).run(&mut a);
    assert!(
        r.findings().is_empty(),
        "the caller's string is unknown, not wrong: {:#?}",
        r.findings()
    );
    assert!(
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .any(|x| x.detail.contains("not concretely readable")),
        "and the run says it did not check it: {:#?}",
        r.states()
            .iter()
            .flat_map(|s| s.assumptions())
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
}
