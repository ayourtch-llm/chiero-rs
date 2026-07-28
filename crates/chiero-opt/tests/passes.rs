//! Covers: 020 contracts 17, 44, and 020 §9's prohibitions.
//!
//! 020 §9: passes are opt-in, off by default, and each must be **observationally
//! transparent** — for the same entry state the set of reported findings and their
//! concrete counterexamples is unchanged, only performance differs.
//!
//! The spec's own parenthesis on contract 16 is the shape of every test here:
//! *"transparency alone is satisfied by a pass that does nothing."* So each pass is
//! checked twice — once that it changes nothing anyone can observe, and once that it
//! **did something**, on a fixture named for what it should do.
//!
//! §9's four prohibitions are separately testable and separately easy to violate: no pass
//! may drop a `Marker`, merge two blocks with different `Volatile` accesses, discard
//! `Span`s, or widen a `LoadBits`/`StoreBits` into a byte-granular access.

use chiero_cir::{InstKind, MarkerKind, Module, Volatility};

mod harness;
use harness::lower;

/// Every pass, iterated — **not a hand-written list at each call site**.
///
/// Contract 44 says "running *every* pass over a bitfield fixture", and a test that names
/// the passes it knows about silently stops covering the next one somebody adds. The
/// registry is the thing under test as much as the passes are.
fn passes() -> &'static [chiero_opt::Pass] {
    chiero_opt::PASSES
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

/// **Contract 17, second half.** `simplify_cfg` reduces the block count on a named
/// fixture from N to M < N.
///
/// The fixture is a straight line through braces: lowering gives each scope its own
/// block, and every one of them has a single predecessor and a single successor, which is
/// exactly the shape §9 says the pass merges. Without the reduction assertion the
/// transparency test below is passed by `fn simplify_cfg(_: &mut Module) {}`.
#[test]
fn simplify_cfg_reduces_the_block_count_on_a_named_fixture() {
    let mut m = lower("int f(int n) { { n = n + 1; } { n = n + 2; } { n = n + 3; } return n; }");
    let before = block_count(&m);
    chiero_opt::simplify_cfg(&mut m);
    let after = block_count(&m);
    assert!(
        after < before,
        "the pass merges single-pred/single-succ blocks: {before} -> {after}"
    );

    // And what it leaves must still be a module the rest of the system accepts. A pass
    // that produced unverifiable CIR would be caught here rather than three crates away.
    let errs = chiero_cir::verify::verify(&m);
    assert!(errs.is_empty(), "{errs:#?}");

    // **A branching function is not flattened.** Reduction alone is satisfied by a pass
    // that deletes blocks, and the difference between merging and deleting is invisible
    // in a count.
    let mut m = lower("int f(int n) { if (n > 0) { return 1; } return 0; }");
    let before = block_count(&m);
    chiero_opt::simplify_cfg(&mut m);
    assert!(
        block_count(&m) >= 3 && block_count(&m) <= before,
        "a real branch keeps its arms: {before} -> {}",
        block_count(&m)
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
    let mut m = lower("int f(int n) { { n = n + 1; } { n = n + 2; } { n = n + 3; } return n; }");
    let before: std::collections::BTreeSet<u32> = m
        .funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.gcov_lines.iter().copied())
        .collect();
    assert!(
        before.len() >= 3,
        "the fixture must span several lines or the union is trivially preserved: {before:?}"
    );
    chiero_opt::simplify_cfg(&mut m);
    let after: std::collections::BTreeSet<u32> = m
        .funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.gcov_lines.iter().copied())
        .collect();
    assert_eq!(
        before, after,
        "every line an absorbed block covered is still covered by the block that absorbed it"
    );

    // 015 §5 says the set is sorted ascending, and a union built by appending is not.
    for f in &m.funcs {
        for b in &f.blocks {
            let mut sorted = b.gcov_lines.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(
                b.gcov_lines, sorted,
                "a merged block's lines are still sorted and deduplicated"
            );
        }
    }
}

/// **020 §9.** No pass drops a `Marker` or discards a `Span`.
///
/// Both are load-bearing for something outside the pass's view: 021 retires objects on
/// `Marker::Scope`, and 030 attributes coverage by `Span`. A pass that merged blocks by
/// concatenating only their non-marker instructions would pass every other test here and
/// leak every local in the absorbed block.
#[test]
fn no_pass_drops_a_marker_or_a_span() {
    let src = "int f(int n) { { int a = n; { int b = a + 1; n = b; } } return n; }";
    for pass in passes() {
        let m0 = lower(src);
        let mut m = lower(src);
        (pass.run)(&mut m);

        let (before, after) = (markers(&m0), markers(&m));
        assert_eq!(
            before.len(),
            after.len(),
            "`{}` dropped a marker: {before:?} -> {after:?}",
            pass.name
        );

        for f in &m.funcs {
            for b in &f.blocks {
                for i in &b.insts {
                    assert!(
                        i.span != chiero_span::Span::DUMMY,
                        "`{}` left an instruction with no span: {:?}",
                        pass.name,
                        i.kind
                    );
                }
            }
        }
    }
}

/// **020 §9.** No pass merges two blocks with different `Volatile` accesses.
///
/// A volatile access is an observable event in program order (020 §4.2). Merging across
/// one is not merely a reordering risk: the two blocks were separated by a branch the
/// device's value decides, and a merge asserts a path the hardware need not take.
#[test]
fn no_pass_merges_across_a_volatile_access() {
    let src = "int f(volatile int *p) { int a = *p; if (a) { a = *p; } return a + *p; }";
    for pass in passes() {
        let mut m = lower(src);
        (pass.run)(&mut m);
        for f in &m.funcs {
            for b in &f.blocks {
                let vols = b
                    .insts
                    .iter()
                    .filter(|i| match &i.kind {
                        InstKind::Store { vol, .. } => *vol == Volatility::Volatile,
                        InstKind::Assign {
                            rv: chiero_cir::RValue::Load { vol, .. },
                            ..
                        } => *vol == Volatility::Volatile,
                        _ => false,
                    })
                    .count();
                assert!(
                    vols <= 1,
                    "`{}` merged two volatile accesses into one block, which asserts an \
                     order the device did not agree to: {:#?}",
                    pass.name,
                    b.insts
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
/// pass that rewrote `StoreBits { off: 0, width: 3 }` into a byte store would make the
/// second read *defined* — the finding would disappear, and the checker would go quiet
/// about a real defect.
#[test]
fn no_pass_widens_a_bitfield_access() {
    let src = "struct B { unsigned a:3; unsigned b:5; };\n\
               unsigned f(void) { struct B s; s.a = 1; return s.b; }\n";
    for pass in passes() {
        let mut m = lower(src);
        let bits_before = count_bit_ops(&m);
        assert!(
            bits_before > 0,
            "the fixture must contain bit-granular accesses or this proves nothing"
        );
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
                        rv: chiero_cir::RValue::LoadBits { .. },
                        ..
                    }
            )
        })
        .count()
}

/// **020 §9's core requirement.** Every pass is observationally transparent: the findings
/// and their counterexamples are unchanged.
///
/// Run over several fixtures rather than one, because transparency is a claim about all
/// programs and a single straight-line function exercises no pass's interesting case.
/// The engine's own report is the comparison — not a summary of it — since §9's promise
/// covers the counterexamples too.
#[test]
fn every_pass_is_observationally_transparent() {
    let fixtures = [
        "int f(int n) { int t = 0; for (int i = 0; i < n; i++) { t += i; } return t; }",
        "int f(int n) { if (n > 3) { return n * 2; } return n - 1; }",
        "int f(int n) { int a[4]; a[0] = n; a[1] = n + 1; return a[0] + a[1]; }",
        "int f(int n) { switch (n) { case 1: return 1; case 2: return 2; default: return 0; } }",
        "int f(int n) { int x = 1; { int y = n; x = y + x; } return x; }",
    ];
    for pass in passes() {
        for src in fixtures {
            let base = run_report(&lower(src));
            let mut m = lower(src);
            (pass.run)(&mut m);
            assert!(
                chiero_cir::verify::verify(&m).is_empty(),
                "`{}` produced a module that does not verify on `{src}`",
                pass.name
            );
            let after = run_report(&m);
            assert_eq!(
                base, after,
                "`{}` changed what the engine reports on `{src}` — §9 permits it to change \
                 performance and nothing else",
                pass.name
            );
        }
    }
}

/// The engine's findings for a module, as the reader would see them.
fn run_report(m: &Module) -> Vec<String> {
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(m).run(&mut a);
    let mut out = r.findings();
    // The *set* is the contract, and completion order is a scheduling detail a pass is
    // allowed to disturb — sorting here so a reordering does not read as a regression.
    out.sort();
    out
}

/// A pass is **off by default**: `run_default` is the identity.
///
/// 020 §9's first sentence, and the one nothing else here would catch. A build that
/// enabled `mem2reg` for everyone would pass every transparency test above precisely
/// because those tests assert transparency.
#[test]
fn passes_are_off_by_default() {
    let src = "int f(int n) { { n = n + 1; } { n = n + 2; } return n; }";
    let m0 = lower(src);
    let mut m = lower(src);
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
