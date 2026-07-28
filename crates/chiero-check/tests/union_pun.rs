//! Covers: 020 contract 29 and 040 contract 14 — the `union-pun` checker and the default
//! checker set.
//!
//! 020 §4.5: "Reading a member other than the last one written is legal and produces no
//! finding. C89/C99 call this undefined; gcc defines it; VPP depends on it. chiero follows
//! gcc. A `union-pun` checker exists but is **off by default** — enabling it on VPP would
//! emit tens of thousands of findings about code that is working as designed."
//!
//! So this checker is unusual: it is **correct for it to be silent**, and the thing that
//! must be tested is not only that it fires but that nobody gets it by accident. 020
//! contract 29 says so in a parenthesis — "an unquantified 'produces findings' is
//! satisfied by a checker that never fires" — and asks for an exact count at named spans.
//!
//! Fixtures are `.cir`, per 001 §4 rule 7: `chiero-check` is a vertical.

use chiero_cir::*;
use chiero_span::{BytePos, ExpnCtx, Span};

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

fn addr(dst: u32, off: i128, lo: u32) -> Vec<Inst> {
    let mut v = vec![inst(
        InstKind::Assign {
            dst: ValueId(0),
            rv: RValue::AddrOfLocal {
                alloca: AllocaId(0),
            },
        },
        lo,
    )];
    if off != 0 {
        v.push(inst(
            InstKind::Assign {
                dst: ValueId(dst),
                rv: RValue::PtrAdd {
                    base: Operand::Value(ValueId(0)),
                    off: Operand::Const(Const::Int { bits: 64, val: off }),
                },
            },
            lo + 1,
        ));
    }
    v
}

fn store(addr: u32, bits: u32, val: i128, lo: u32) -> Inst {
    inst(
        InstKind::Store {
            addr: Operand::Value(ValueId(addr)),
            val: Operand::Const(Const::Int { bits, val }),
            ty: CTy::Int(bits),
            align: 4,
            vol: Volatility::Normal,
        },
        lo,
    )
}

fn load(dst: u32, addr: u32, bits: u32, lo: u32) -> Inst {
    inst(
        InstKind::Assign {
            dst: ValueId(dst),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(addr)),
                ty: CTy::Int(bits),
                align: 1,
                vol: Volatility::Normal,
            },
        },
        lo,
    )
}

/// `union { u32 as_u32; u8 as_u8[4]; } u;` — write the word, then read three bytes back
/// through the other member.
///
/// Three punned reads, at spans 40, 42 and 44. Three rather than one because contract 29
/// asks for an exact count: a checker that reported once per *object* rather than once per
/// punned access gives 1, and one that reported the store as well gives 4.
fn punning_fixture() -> Module {
    let mut insts = addr(0, 0, 10);
    insts.push(store(0, 32, 0x1122_3344, 20));
    insts.extend(addr(1, 1, 30).into_iter().skip(1));
    insts.extend(addr(2, 2, 32).into_iter().skip(1));
    insts.extend(addr(3, 3, 34).into_iter().skip(1));
    insts.push(load(4, 1, 8, 40));
    insts.push(load(5, 2, 8, 42));
    insts.push(load(6, 3, 8, 44));

    Module {
        funcs: vec![Function {
            id: FuncId(0),
            name: "pun".into(),
            params: vec![],
            ret: CTy::Void,
            variadic: false,
            allocas: vec![AllocaDecl {
                id: AllocaId(0),
                ty: CTy::Int(8),
                count: 4,
                align: 4,
                scope: ScopeId(0),
                lifetime: Lifetime::Scope,
                name: Some("u".into()),
                span: at(1),
            }],
            blocks: vec![Block {
                id: BlockId(0),
                insts,
                term: Terminator::Return(None),
                gcov_lines: Default::default(),
                span: at(1),
            }],
            entry: BlockId(0),
            attrs: Default::default(),
            body: Body::Defined,
            access_paths: Default::default(),
            span: at(1),
        }],
        ..Default::default()
    }
}

fn run_with(m: &Module, checkers: Vec<Box<dyn chiero_exec::Checker>>) -> Vec<String> {
    let errs = chiero_cir::verify::verify(m);
    assert!(errs.iter().all(|e| !e.is_error()), "{errs:#?}");
    let mut a = chiero_solver::TermArena::new();
    let mut e = chiero_exec::Engine::new(m);
    for c in checkers {
        e = e.with_checker(c);
    }
    e.run(&mut a).findings()
}

/// **020 contract 29 / 040 contract 14.** Enabling `union-pun` on the fixture produces
/// exactly three findings, at the three punned reads' spans; the default set produces
/// none.
#[test]
fn union_pun_fires_exactly_three_times_when_enabled() {
    let m = punning_fixture();
    let got = run_with(&m, vec![Box::new(chiero_check::UnionPun::new())]);
    let pun: Vec<&String> = got.iter().filter(|f| f.contains("pun")).collect();
    assert_eq!(
        pun.len(),
        3,
        "one per punned *read* — not one per object, and not counting the store: {got:#?}"
    );

    // The default set is silent on the same module. Both halves in one test, because a
    // checker that never fires satisfies the second alone and one that is on by default
    // satisfies the first alone.
    let quiet = run_with(&m, chiero_check::default_checkers());
    assert!(
        quiet.iter().all(|f| !f.contains("pun")),
        "gcc defines this and VPP depends on it — enabling it by default would bury every \
         real finding under tens of thousands of these: {quiet:#?}"
    );
}

/// The findings land at the **spans of the punned reads**, which is what makes them
/// actionable rather than a count.
#[test]
fn the_findings_name_the_reads_that_punned() {
    let m = punning_fixture();
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(&m)
        .with_checker(Box::new(chiero_check::UnionPun::new()))
        .run(&mut a);
    let spans: Vec<u32> = r
        .reports()
        .into_iter()
        .filter(|f| f.message.contains("pun"))
        .map(|f| f.span.lo.0)
        .collect();
    assert_eq!(
        spans,
        vec![40, 42, 44],
        "the three reads, in program order — a checker reporting at the store's span \
         would point a reader at the line that was right"
    );
}

/// **Reading the member that was written is not punning.**
///
/// The negative half, and the one a checker keyed on "is this object a union" fails: a
/// `u32` store followed by a `u32` load at the same offset is an ordinary read, however
/// many members the type has.
#[test]
fn reading_back_the_member_that_was_written_is_not_a_pun() {
    let mut insts = addr(0, 0, 10);
    insts.push(store(0, 32, 7, 20));
    insts.push(load(1, 0, 32, 30));
    let mut m = punning_fixture();
    m.funcs[0].blocks[0].insts = insts;

    let got = run_with(&m, vec![Box::new(chiero_check::UnionPun::new())]);
    assert!(
        got.iter().all(|f| !f.contains("pun")),
        "same offset, same width: nothing was reinterpreted: {got:#?}"
    );
}

/// **A read of memory nothing wrote is not punning either** — it is an uninitialized
/// read, which a different checker owns.
///
/// Without this, a checker that reported every load whose bytes it could not attribute
/// would pass every test above and then fire on every uninitialized read in the corpus,
/// mislabelled.
#[test]
fn a_read_with_no_prior_write_is_not_a_pun() {
    let mut insts = addr(0, 0, 10);
    insts.push(load(1, 0, 8, 30));
    let mut m = punning_fixture();
    m.funcs[0].blocks[0].insts = insts;

    let got = run_with(&m, vec![Box::new(chiero_check::UnionPun::new())]);
    assert!(
        got.iter().all(|f| !f.contains("pun")),
        "nothing was written, so nothing was reinterpreted: {got:#?}"
    );
}

/// The default set is **not empty** — otherwise "absent from the default set" is true of
/// everything and 040 contract 14 tests nothing.
#[test]
fn the_default_set_contains_checkers() {
    let names: Vec<&'static str> = chiero_check::default_checkers()
        .iter()
        .map(|c| c.name())
        .collect();
    assert!(
        !names.is_empty(),
        "a default set that is empty makes every absence vacuous"
    );
    assert!(
        !names.contains(&"union-pun"),
        "and `union-pun` is not in it (040 §1): {names:?}"
    );
}

/// **A read that does not overlap any write is not a pun.**
///
/// Write the first four bytes of an eight-byte object, then read the *last* four. Those
/// bytes were never written by anyone, so nothing was reinterpreted — it is an
/// uninitialized read, and a different checker owns it.
///
/// Every other fixture here writes and reads the same bytes, so a checker that asked only
/// "has anything been written to this object?" passes all of them and fires here. That is
/// the difference between per-object bookkeeping and per-range bookkeeping, and it is the
/// whole reason the writes are keyed by offset.
#[test]
fn a_read_that_misses_every_write_is_not_a_pun() {
    let mut m = punning_fixture();
    m.funcs[0].allocas[0].count = 8;

    let mut insts = addr(0, 0, 10);
    insts.push(store(0, 32, 0x1122_3344, 20));
    insts.extend(addr(1, 4, 30).into_iter().skip(1));
    insts.push(load(2, 1, 32, 40));
    m.funcs[0].blocks[0].insts = insts;

    let got = run_with(&m, vec![Box::new(chiero_check::UnionPun::new())]);
    assert!(
        got.iter().all(|f| !f.contains("pun")),
        "bytes 4..8 were never written, so they were never reinterpreted: {got:#?}"
    );

    // The control: the same object, the same widths, a read that *does* overlap. Without
    // this the test above passes against a checker that went silent entirely.
    let mut insts = addr(0, 0, 10);
    insts.push(store(0, 32, 0x1122_3344, 20));
    insts.extend(addr(1, 2, 30).into_iter().skip(1));
    insts.push(load(2, 1, 32, 40));
    m.funcs[0].blocks[0].insts = insts;
    let got = run_with(&m, vec![Box::new(chiero_check::UnionPun::new())]);
    assert_eq!(
        got.iter().filter(|f| f.contains("pun")).count(),
        1,
        "a read straddling the write is a pun: {got:#?}"
    );
}

/// **A read at the same offset but a narrower width is a pun.**
///
/// `u.as_u32 = x; u.as_u8[0]` — offset 0 in both, one byte against four. Every punned read
/// in the main fixture is at a *shifted* offset (1, 2, 3), so a checker comparing only
/// offsets passes all of them and misses this one, which is the single most common punning
/// idiom in C: read the first byte of a word.
#[test]
fn a_narrower_read_at_the_same_offset_is_a_pun() {
    let mut insts = addr(0, 0, 10);
    insts.push(store(0, 32, 0x1122_3344, 20));
    insts.push(load(1, 0, 8, 30));
    let mut m = punning_fixture();
    m.funcs[0].blocks[0].insts = insts;

    let got = run_with(&m, vec![Box::new(chiero_check::UnionPun::new())]);
    assert_eq!(
        got.iter().filter(|f| f.contains("pun")).count(),
        1,
        "same offset, four bytes written and one read: {got:#?}"
    );
}

/// **A store is never a pun**, even when it overlaps an earlier store at a different
/// offset.
///
/// Writing `as_u32` and then `as_u8[2]` is a partial overwrite — 020 §4.5 calls it exact
/// and expected, and there is nothing to reinterpret because nobody has read anything. A
/// checker that ran its comparison on writes as well as reads reports here, and every
/// other fixture in this file has at most one store.
#[test]
fn overlapping_stores_are_not_puns() {
    let mut insts = addr(0, 0, 10);
    insts.push(store(0, 32, 0x1122_3344, 20));
    insts.extend(addr(1, 2, 30).into_iter().skip(1));
    insts.push(store(1, 8, 0xFF, 40));
    let mut m = punning_fixture();
    m.funcs[0].blocks[0].insts = insts;

    let got = run_with(&m, vec![Box::new(chiero_check::UnionPun::new())]);
    assert!(
        got.iter().all(|f| !f.contains("pun")),
        "a partial overwrite is exact and expected; nothing was read: {got:#?}"
    );
}

/// **Two different objects do not pun each other.**
///
/// A write to one object and a read at an overlapping *offset range* of another is not a
/// pun — they are different bytes. Every other fixture here uses a single object, so a
/// checker that compared only offsets and widths, ignoring which object they belong to,
/// passes all of them and then fires across every unrelated pair of locals in the corpus.
#[test]
fn a_read_of_a_different_object_is_not_a_pun() {
    let mut m = punning_fixture();
    m.funcs[0].allocas.push(AllocaDecl {
        id: AllocaId(1),
        ty: CTy::Int(8),
        count: 4,
        align: 4,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: Some("other".into()),
        span: at(1),
    });
    let insts = vec![
        inst(
            InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(0),
                },
            },
            10,
        ),
        store(0, 32, 0x1122_3344, 20),
        inst(
            InstKind::Assign {
                dst: ValueId(1),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(1),
                },
            },
            30,
        ),
        // Offset 0, width 1 — overlapping the *first* object's write range exactly, if
        // one forgets to ask which object.
        load(2, 1, 8, 40),
    ];
    m.funcs[0].blocks[0].insts = insts;

    let got = run_with(&m, vec![Box::new(chiero_check::UnionPun::new())]);
    assert!(
        got.iter().all(|f| !f.contains("pun")),
        "`other` is a different object; nothing written to `u` is visible in it: {got:#?}"
    );
}
