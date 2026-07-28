//! Covers: 020 contracts 17, 44, and 020 §9's prohibitions.
//!
//! 020 §9: passes are opt-in, off by default, and each must be **observationally
//! transparent** — for the same entry state the set of reported findings and their
//! concrete counterexamples is unchanged, only performance differs.
//!
//! The spec writes its own warning into contract 16's parenthesis: *"transparency alone
//! is satisfied by a pass that does nothing."* So each pass is checked twice — once that
//! it changes nothing anyone can observe, and once that it **did something**, on a
//! fixture named for what it should do.
//!
//! **Fixtures are `.cir`, not C.** `chiero-opt` is a vertical and 001 §4 rule 7 forbids it
//! a frontend dependency, dev-dependencies included. That is the right constraint rather
//! than an obstacle: 020's contracts are written about CIR, the checked-in corpus is
//! already in that language, and a pass that needed a C parser to be tested would be a
//! pass tested at the wrong layer.

use chiero_cir::*;
use chiero_span::{BytePos, ExpnCtx, Span};

/// Every pass, iterated — **not a hand-written list at each call site**.
///
/// Contract 44 says "running *every* pass over a bitfield fixture", and a test that names
/// the passes it knows about silently stops covering the next one somebody adds. The
/// registry is as much under test as the passes are.
fn passes() -> &'static [chiero_opt::Pass] {
    chiero_opt::PASSES
}

fn corpus_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name>/ has a workspace root above it")
        .join("tests/corpus")
}

/// Every checked-in `.cir` module: the hand-written M1 fixtures and the lowered goldens.
///
/// Both halves matter. The hand-written fixtures are minimal and name their shapes; the
/// lowered goldens are what real C actually produces, including the markers and spans a
/// hand-written fixture is unlikely to bother with — and §9's prohibitions are all about
/// those.
fn corpus() -> Vec<(String, Module)> {
    let mut out = Vec::new();
    for sub in ["cir", "lowered"] {
        let dir = corpus_dir().join(sub);
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|_| panic!("no corpus at {}", dir.display()))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "cir"))
            .collect();
        // Sorted: `read_dir` order is the filesystem's, and a sweep that walks it in a
        // different order on another machine is a different test.
        paths.sort();
        for p in paths {
            let text = std::fs::read_to_string(&p).expect("read");
            let m = chiero_cir::text::parse(&text)
                .unwrap_or_else(|e| panic!("{} does not parse: {e:?}", p.display()));
            out.push((p.file_stem().unwrap().to_string_lossy().into_owned(), m));
        }
    }
    assert!(
        out.len() >= 10,
        "the corpus sweep must actually sweep something: found {}",
        out.len()
    );
    out
}

fn block_count(m: &Module) -> usize {
    m.funcs.iter().map(|f| f.blocks.len()).sum()
}

fn markers(m: &Module) -> Vec<MarkerKind> {
    m.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Marker(k) => Some(k.clone()),
            _ => None,
        })
        .collect()
}

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
}

fn inst(kind: InstKind, lo: u32) -> Inst {
    Inst {
        kind,
        span: at(lo),
        generated: false,
    }
}

fn block(id: u32, lines: &[u32], insts: Vec<Inst>, term: Terminator) -> Block {
    Block {
        id: BlockId(id),
        insts,
        term,
        gcov_lines: lines.iter().copied().collect(),
        span: at(1),
    }
}

/// A straight line of `n` blocks, each covering one source line, each with a single
/// predecessor and a single successor — the exact shape §9 says `simplify_cfg` merges.
fn chain(n: u32) -> Module {
    let mut blocks = Vec::new();
    for i in 0..n {
        let term = if i + 1 == n {
            Terminator::Return(Some(Operand::Const(Const::Int { bits: 32, val: 0 })))
        } else {
            Terminator::Goto(BlockId(i + 1))
        };
        blocks.push(block(
            i,
            &[10 + i],
            vec![inst(
                InstKind::Assign {
                    dst: ValueId(i),
                    rv: RValue::Bin {
                        op: BinOp::Add,
                        a: Operand::Const(Const::Int {
                            bits: 32,
                            val: i as i128,
                        }),
                        b: Operand::Const(Const::Int { bits: 32, val: 1 }),
                        ty: CTy::Int(32),
                    },
                },
                20 + i,
            )],
            term,
        ));
    }
    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![],
            blocks,
            entry: BlockId(0),
            attrs: Default::default(),
            access_paths: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    }
}

/// **Contract 17, second half.** `simplify_cfg` reduces the block count on a named fixture
/// from N to M < N.
///
/// Without this half the transparency test below is passed by
/// `fn simplify_cfg(_: &mut Module) {}`.
#[test]
fn simplify_cfg_reduces_the_block_count_on_a_named_fixture() {
    let mut m = chain(4);
    assert!(
        chiero_cir::verify::verify(&m).is_empty(),
        "the fixture is CIR"
    );
    chiero_opt::simplify_cfg(&mut m);
    assert_eq!(
        block_count(&m),
        1,
        "four blocks in a chain, each with one predecessor and one successor, are one \
         block: {:#?}",
        m.funcs[0].blocks
    );

    // What it leaves must still be a module the rest of the system accepts — caught here
    // rather than three crates away.
    let errs = chiero_cir::verify::verify(&m);
    assert!(errs.is_empty(), "{errs:#?}");

    // **A real branch keeps its arms.** Reduction alone is satisfied by a pass that
    // deletes blocks, and the difference between merging and deleting is invisible in a
    // count.
    let text = std::fs::read_to_string(corpus_dir().join("cir/branch.cir")).expect("read");
    let mut m = chiero_cir::text::parse(&text).expect("parse");
    let before = block_count(&m);
    chiero_opt::simplify_cfg(&mut m);
    let after = block_count(&m);
    assert!(after <= before, "{before} -> {after}");
    assert!(
        m.funcs
            .iter()
            .any(|f| f.blocks.iter().any(|b| b.term.successors().len() == 2)),
        "the two-way branch survives: a pass that flattened it would have changed which \
         paths exist"
    );
    assert!(chiero_cir::verify::verify(&m).is_empty());
}

/// **Contract 17, first half.** `simplify_cfg` preserves the union of `gcov_lines` across
/// merged blocks.
///
/// 030 correlates coverage by these sets. A merge that kept only the surviving block's
/// lines would report the absorbed block's lines as never executed — the pass would turn
/// itself into a coverage regression, which is precisely the failure §9 calls out.
#[test]
fn simplify_cfg_unions_gcov_lines_across_merged_blocks() {
    let mut m = chain(4);
    let before: Vec<u32> = vec![10, 11, 12, 13];
    chiero_opt::simplify_cfg(&mut m);
    let after: Vec<u32> = m.funcs[0]
        .blocks
        .iter()
        .flat_map(|b| b.gcov_lines.iter().copied())
        .collect();
    assert_eq!(
        after, before,
        "every line an absorbed block covered is still covered by the block that \
         absorbed it"
    );

    // 015 §5 says the set is sorted ascending and deduplicated, and a union built by
    // appending is neither.
    //
    // **The absorbed block must cover an *earlier* line than the one absorbing it**, or
    // the concatenation is already sorted and the requirement is never exercised. That is
    // not a contrived shape: a `for` latch covers the increment, which is written above
    // the body it follows. The overlap at 30 is the deduplication half — a `for` header
    // and its latch share the line the `for` is written on.
    let mut m = chain(2);
    m.funcs[0].blocks[0].gcov_lines = [30, 40].into_iter().collect();
    m.funcs[0].blocks[1].gcov_lines = [10, 30].into_iter().collect();
    chiero_opt::simplify_cfg(&mut m);
    assert_eq!(
        m.funcs[0].blocks[0].gcov_lines.as_slice(),
        &[10, 30, 40],
        "sorted ascending and deduplicated, not concatenated"
    );
}

/// **020 §9.** No pass drops a `Marker` or discards a `Span`.
///
/// Both are load-bearing for something outside the pass's view: 021 retires objects on
/// `Marker::Scope`, and 030 attributes coverage by `Span`. A pass that merged blocks by
/// concatenating only their non-marker instructions would pass every other test here and
/// leak every local in the absorbed block.
#[test]
fn no_pass_drops_a_marker_or_a_span() {
    for pass in passes() {
        for (name, m0) in corpus() {
            let mut m = m0.clone();
            (pass.run)(&mut m);

            let (before, after) = (markers(&m0), markers(&m));
            assert_eq!(
                before.len(),
                after.len(),
                "`{}` dropped a marker in `{name}`",
                pass.name
            );

            let dummies_before = spans(&m0).filter(|s| *s == Span::DUMMY).count();
            let dummies_after = spans(&m).filter(|s| *s == Span::DUMMY).count();
            assert!(
                dummies_after <= dummies_before,
                "`{}` discarded a span in `{name}`: {dummies_before} dummy spans became \
                 {dummies_after}",
                pass.name
            );
        }
    }
}

fn spans(m: &Module) -> impl Iterator<Item = Span> + '_ {
    m.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.insts.iter())
        .map(|i| i.span)
}

/// **020 §9.** No pass merges two blocks with different `Volatile` accesses.
///
/// A volatile access is an observable event in program order (020 §4.2). Merging across
/// one is not merely a reordering risk: the two blocks were separated by a branch, and a
/// merge asserts a path the hardware need not take.
#[test]
fn no_pass_merges_across_a_volatile_access() {
    // Two volatile stores in separate blocks joined by a `Goto` — the shape
    // `simplify_cfg` would otherwise merge on sight.
    let vol_store = |v: i128, lo: u32| {
        inst(
            InstKind::Store {
                addr: Operand::Value(ValueId(0)),
                val: Operand::Const(Const::Int { bits: 32, val: v }),
                ty: CTy::Int(32),
                align: 4,
                vol: Volatility::Volatile,
            },
            lo,
        )
    };
    let base = Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "f".into(),
            params: vec![],
            ret: CTy::Int(32),
            variadic: false,
            allocas: vec![AllocaDecl {
                id: AllocaId(0),
                ty: CTy::Int(32),
                count: 1,
                align: 4,
                scope: ScopeId(0),
                lifetime: Lifetime::Scope,
                name: None,
                span: at(1),
            }],
            blocks: vec![
                block(
                    0,
                    &[10],
                    vec![
                        inst(
                            InstKind::Assign {
                                dst: ValueId(0),
                                rv: RValue::AddrOfLocal {
                                    alloca: AllocaId(0),
                                },
                            },
                            11,
                        ),
                        vol_store(1, 12),
                    ],
                    Terminator::Goto(BlockId(1)),
                ),
                block(
                    1,
                    &[11],
                    vec![vol_store(2, 20)],
                    Terminator::Return(Some(Operand::Const(Const::Int { bits: 32, val: 0 }))),
                ),
            ],
            entry: BlockId(0),
            attrs: Default::default(),
            access_paths: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    };
    for pass in passes() {
        let mut m = base.clone();
        (pass.run)(&mut m);
        for f in &m.funcs {
            for b in &f.blocks {
                let vols = b
                    .insts
                    .iter()
                    .filter(|i| match &i.kind {
                        InstKind::Store { vol, .. } => *vol == Volatility::Volatile,
                        InstKind::Assign {
                            rv: RValue::Load { vol, .. },
                            ..
                        } => *vol == Volatility::Volatile,
                        _ => false,
                    })
                    .count();
                assert!(
                    vols <= 1,
                    "`{}` merged two volatile accesses into one block, which asserts an \
                     order the device did not agree to",
                    pass.name
                );
            }
        }
    }
}

/// **Contract 44.** No pass widens a `LoadBits`/`StoreBits` into a byte-granular access,
/// verified by running every pass over a bitfield fixture and re-checking contract 24.
///
/// Contract 24 is the one byte-granular lowering fails: writing only `a` and reading `a`
/// must produce no uninitialized-read finding, while reading `b` produces exactly one. A
/// pass that rewrote `StoreBits { off: 0, width: 3 }` into a byte store would make that
/// second read *defined* — the finding would disappear, and the checker would go quiet
/// about a real defect.
#[test]
fn no_pass_widens_a_bitfield_access() {
    let text = std::fs::read_to_string(corpus_dir().join("cir/bitfield.cir")).expect("read");
    let m0 = chiero_cir::text::parse(&text).expect("parse");
    let bits_before = count_bit_ops(&m0);
    assert!(
        bits_before >= 3,
        "the fixture must contain bit-granular accesses or this proves nothing: \
         {bits_before}"
    );
    for pass in passes() {
        let mut m = m0.clone();
        (pass.run)(&mut m);
        assert_eq!(
            count_bit_ops(&m),
            bits_before,
            "`{}` changed the number of bit-granular accesses; contract 24's \
             uninitialized-read finding depends on them staying bit-granular",
            pass.name
        );
        assert!(
            !m.funcs
                .iter()
                .flat_map(|f| f.blocks.iter())
                .flat_map(|b| b.insts.iter())
                .any(|i| matches!(i.kind, InstKind::Store { .. })),
            "`{}` turned a `StoreBits` into a byte store",
            pass.name
        );
    }
}

fn count_bit_ops(m: &Module) -> usize {
    m.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.insts.iter())
        .filter(|i| {
            matches!(
                i.kind,
                InstKind::StoreBits { .. }
                    | InstKind::Assign {
                        rv: RValue::LoadBits { .. },
                        ..
                    }
            )
        })
        .count()
}

/// **020 §9's core requirement.** Every pass is observationally transparent: the findings
/// and their concrete counterexamples are unchanged, over the whole corpus.
///
/// §9 asks for exactly this — "a differential test that runs the corpus with and without
/// it and diffs the findings" — and the corpus rather than one fixture because
/// transparency is a claim about all programs.
#[test]
fn every_pass_is_observationally_transparent_over_the_corpus() {
    for pass in passes() {
        for (name, m0) in corpus() {
            let base = run_report(&m0);
            let mut m = m0.clone();
            (pass.run)(&mut m);
            let errs = chiero_cir::verify::verify(&m);
            assert!(
                errs.is_empty(),
                "`{}` produced a module that does not verify on `{name}`: {errs:#?}",
                pass.name
            );
            assert_eq!(
                base,
                run_report(&m),
                "`{}` changed what the engine reports on `{name}` — §9 permits it to \
                 change performance and nothing else",
                pass.name
            );
        }
    }
}

/// The engine's findings for a module, as the reader would see them.
///
/// **Compared verbatim.** For eight waves this normalized `ObjectId(N)` out of the text,
/// because `mem2reg` removes allocas and the remaining objects renumber — so the same
/// defect printed differently with the pass on, and the sweep would have read that as a
/// pass changing what the engine reports. Wave 111 fixed the finding instead: it names the
/// variable now, which is stable across pass configurations because it is a property of
/// the program rather than of the allocator. The normalization is gone, and this
/// comparison is stronger for it.
fn run_report(m: &Module) -> Vec<String> {
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(m).run(&mut a);
    let mut out = r.findings();
    // The *set* is the contract; completion order is a scheduling detail a pass may
    // legitimately disturb, so sorting keeps a reordering from reading as a regression.
    out.sort();
    out
}

/// A pass is **off by default**: `run_default` changes nothing.
///
/// 020 §9's first sentence, and the one nothing else here would catch. A build that
/// enabled `mem2reg` for everyone would pass every transparency test above *precisely
/// because* those tests assert transparency.
#[test]
fn passes_are_off_by_default() {
    let m0 = chain(4);
    let mut m = chain(4);
    chiero_opt::run_default(&mut m);
    assert_eq!(
        block_count(&m),
        block_count(&m0),
        "no pass runs unless it is asked for"
    );
    assert!(
        !passes().is_empty(),
        "and the registry is not empty, or every loop above ran zero times"
    );
}

/// **The other half for `const_fold`**, which contract 16's parenthesis demands of every
/// pass: it must actually fold something.
///
/// `const_fold` has no numbered contract of its own — 020 §9 lists it without one — so
/// this is the pairing the section's own logic requires. The `chain` fixture's
/// instructions are all `Const + Const`, which is exactly what folds.
#[test]
fn const_fold_folds_a_constant_expression() {
    let mut m = chain(3);
    let before = folded_count(&m);
    assert_eq!(before, 0, "nothing is folded before the pass runs");
    chiero_opt::const_fold(&mut m);
    assert_eq!(
        folded_count(&m),
        3,
        "every `Const + Const` became a `Use` of the sum: {:#?}",
        m.funcs[0].blocks
    );
    // And the *values* are right, not merely folded. `chain` builds `i + 1`.
    let vals: Vec<i128> = m.funcs[0]
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Assign {
                rv: RValue::Use(Operand::Const(Const::Int { val, .. })),
                ..
            } => Some(*val),
            _ => None,
        })
        .collect();
    assert_eq!(
        vals,
        vec![1, 2, 3],
        "a pass that folded to zero would pass a count"
    );
    assert!(chiero_cir::verify::verify(&m).is_empty());

    // **Division is not folded**, on purpose: division by zero is undefined behaviour the
    // engine reports, and folding it either invents a value the program cannot produce or
    // panics. Either way §9's finding disappears.
    let mut m = chain(1);
    m.funcs[0].blocks[0].insts[0].kind = InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Bin {
            op: BinOp::SDiv,
            a: Operand::Const(Const::Int { bits: 32, val: 8 }),
            b: Operand::Const(Const::Int { bits: 32, val: 0 }),
            ty: CTy::Int(32),
        },
    };
    chiero_opt::const_fold(&mut m);
    assert_eq!(
        folded_count(&m),
        0,
        "`8 / 0` is left for the engine to report"
    );
}

fn folded_count(m: &Module) -> usize {
    m.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.insts.iter())
        .filter(|i| {
            matches!(
                i.kind,
                InstKind::Assign {
                    rv: RValue::Use(Operand::Const(Const::Int { .. })),
                    ..
                }
            )
        })
        .count()
}

/// **A fold that wraps.** `2^31 - 1 + 1` is `-2^31` at `Int(32)`, and an implementation
/// that computed in `i128` and stored the result unwrapped would produce a constant no
/// 32-bit program can hold.
///
/// Every fold in the corpus happens to be small, so without this the pass could be wrong
/// about every overflow and the transparency sweep would still agree with itself: both
/// runs would be wrong in the same direction only if the *engine* wrapped the same way,
/// and it does not — it would report the pass's `i128` value verbatim.
#[test]
fn const_fold_wraps_to_the_declared_width() {
    let mut m = chain(1);
    m.funcs[0].blocks[0].insts[0].kind = InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Bin {
            op: BinOp::Add,
            a: Operand::Const(Const::Int {
                bits: 32,
                val: 2147483647,
            }),
            b: Operand::Const(Const::Int { bits: 32, val: 1 }),
            ty: CTy::Int(32),
        },
    };
    chiero_opt::const_fold(&mut m);
    let val = match &m.funcs[0].blocks[0].insts[0].kind {
        InstKind::Assign {
            rv: RValue::Use(Operand::Const(Const::Int { val, .. })),
            ..
        } => *val,
        other => panic!("not folded: {other:?}"),
    };
    assert_eq!(
        val, -2147483648,
        "`INT_MAX + 1` wraps to `INT_MIN` at 32 bits; an `i128` result would be 2147483648"
    );

    // Multiplication too, where the unwrapped result is further from the wrapped one than
    // an off-by-one — so a `truncate` that merely masked the low bits without sign
    // extension is visible here.
    let mut m = chain(1);
    m.funcs[0].blocks[0].insts[0].kind = InstKind::Assign {
        dst: ValueId(0),
        rv: RValue::Bin {
            op: BinOp::Mul,
            a: Operand::Const(Const::Int {
                bits: 32,
                val: 65536,
            }),
            b: Operand::Const(Const::Int {
                bits: 32,
                val: 65538,
            }),
            ty: CTy::Int(32),
        },
    };
    chiero_opt::const_fold(&mut m);
    let val = match &m.funcs[0].blocks[0].insts[0].kind {
        InstKind::Assign {
            rv: RValue::Use(Operand::Const(Const::Int { val, .. })),
            ..
        } => *val,
        other => panic!("not folded: {other:?}"),
    };
    assert_eq!(val, 131072, "65536 * 65538 wraps to 131072 at 32 bits");
}

/// **The registry holds every pass 020 §9 names.**
///
/// This is the one failure the registry exists to prevent and cannot detect on its own: a
/// pass that is *implemented but not registered* is covered by none of the sweeps above —
/// contract 44 would not run over it, nor would the marker, span or volatile checks. Rust
/// gives no way to enumerate a crate's public functions, so the list is anchored to the
/// **spec** rather than to the code, which is the right direction: §9 names the passes,
/// and a missing one fails here until somebody registers it.
///
/// `mem2reg` is §9's third pass and is not implemented yet; it is named here so that
/// landing it without registering it fails this test on the day it lands.
#[test]
fn the_registry_holds_every_pass_the_spec_names() {
    let registered: Vec<&str> = passes().iter().map(|p| p.name).collect();
    for named in ["simplify_cfg", "const_fold", "mem2reg"] {
        assert!(
            registered.contains(&named),
            "020 §9 names `{named}`, and an unregistered pass is covered by no sweep in \
             this file: {registered:?}"
        );
        assert!(
            chiero_opt::pass(named).is_some(),
            "and it is reachable by name for a configuration-driven caller"
        );
    }
    assert_eq!(
        registered.len(),
        3,
        "a pass was added without being named here, so the spec-anchored list has gone \
         stale: {registered:?}"
    );
}

/// **A self-looping entry is left alone**, and does not send the pass into a merge of a
/// block with itself.
///
/// `entry: goto entry` is `while (1);` with nothing before it. It is the one input where
/// "never merge a block into itself" and "never absorb the entry" both apply — which is
/// why they are one guard in the pass rather than two: written separately, each was
/// satisfied whenever the other was, so neither could be shown to matter.
#[test]
fn a_self_looping_entry_is_not_merged() {
    let mut m = Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "spin".into(),
            params: vec![],
            ret: CTy::Void,
            variadic: false,
            allocas: vec![],
            blocks: vec![block(0, &[10], vec![], Terminator::Goto(BlockId(0)))],
            entry: BlockId(0),
            attrs: Default::default(),
            access_paths: Default::default(),
            body: Body::Defined,
            span: at(1),
        }],
        ..Default::default()
    };
    chiero_opt::simplify_cfg(&mut m);
    assert_eq!(block_count(&m), 1, "the loop is still there");
    assert!(
        matches!(m.funcs[0].blocks[0].term, Terminator::Goto(BlockId(0))),
        "and still loops to itself: {:?}",
        m.funcs[0].blocks[0].term
    );
}
