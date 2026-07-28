//! Covers: 020 contract 23.
//!
//! "A 40-byte `opaque[10]` region written through one struct view and read through a
//! different struct view of the same size returns the written bytes exactly, and the
//! finding text for an OOB access through the second view names the second view's member
//! (`UnionMember { view }`)."
//!
//! This is `vlib_buffer_t::opaque[10]` — 020 §4.5 names it directly. Every VPP graph node
//! reinterprets those 40 bytes as its own struct, and the two views share no field names,
//! no offsets and no types. An IR that modeled the region as a tagged value with an active
//! member would be wrong on the first packet.
//!
//! The two halves test different things and neither implies the other. The first is about
//! the **memory model**: bytes written through one view read back through another, which
//! 021 §3's byte-level memory with no strict-aliasing assumption gives directly. The
//! second is about **reporting**: an out-of-bounds access has to say
//! `opaque as ip4_rewrite_t.adj_index`, not `*(i64*)(%7 + 40)`, and that needs the
//! `AccessPath` 020 §4.4 defines — which no analysis may branch on, so it is carried
//! beside the instructions rather than inside them.

use chiero_cir::verify::verify;
use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
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

fn i64c(v: i128) -> Operand {
    Operand::Const(Const::Int { bits: 64, val: v })
}

/// The 40-byte region, as one alloca of 40 bytes.
fn opaque_slot() -> AllocaDecl {
    AllocaDecl {
        id: AllocaId(0),
        ty: CTy::Int(8),
        count: 40,
        align: 8,
        scope: ScopeId(0),
        lifetime: Lifetime::Scope,
        name: Some("opaque".into()),
        span: at(1),
    }
}

/// `%0 = addrlocal`, then `%1 = ptradd %0, off`.
fn address_at(off: i128, dst: u32, lo: u32) -> Vec<Inst> {
    vec![
        inst(
            InstKind::Assign {
                dst: ValueId(0),
                rv: RValue::AddrOfLocal {
                    alloca: AllocaId(0),
                },
            },
            lo,
        ),
        inst(
            InstKind::Assign {
                dst: ValueId(dst),
                rv: RValue::PtrAdd {
                    base: Operand::Value(ValueId(0)),
                    off: i64c(off),
                },
            },
            lo + 1,
        ),
    ]
}

fn run(insts: Vec<Inst>, paths: Vec<(ValueId, AccessPath)>) -> (RunResult, TermArena) {
    let f = Function {
        id: FuncId(0),
        name: "node".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![opaque_slot()],
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
        access_paths: paths.into_iter().collect(),
        span: at(1),
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    // **The module must verify.** A fixture that does not is reported as the absence of
    // everything, and every assertion below would read as a passing test of nothing.
    assert!(verify(&m).iter().all(|e| !e.is_error()), "{:?}", verify(&m));
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    (r, a)
}

/// **Contract 23, first half.** 40 bytes written through one struct view read back
/// exactly through another.
///
/// View A is `ip4_rewrite_t { u32 adj_index; u64 tx_sw_if_index; }` — a word at 0 and a
/// long at 8. View B is `l2_bridge_t { u64 bd_index; u32 flags; }` — a long at 0 and a
/// word at 8. They agree on no field: B's read at 0 spans both of A's first two bytes and
/// four bytes A never named, and B's read at 8 lands in the middle of A's long.
///
/// That overlap is the point. A model with an active member returns garbage for the
/// second view; a model that rounded reads to field boundaries returns the wrong bytes.
#[test]
fn forty_bytes_written_through_one_view_read_back_through_another() {
    let mut insts = address_at(0, 1, 10);
    // View A: `adj_index = 0x11223344` at offset 0, `tx_sw_if_index` at offset 8.
    insts.push(inst(
        InstKind::Store {
            addr: Operand::Value(ValueId(1)),
            val: Operand::Const(Const::Int {
                bits: 32,
                val: 0x1122_3344,
            }),
            ty: CTy::Int(32),
            align: 8,
            vol: Volatility::Normal,
        },
        20,
    ));
    insts.extend(address_at(8, 2, 22).into_iter().skip(1));
    insts.push(inst(
        InstKind::Store {
            addr: Operand::Value(ValueId(2)),
            val: i64c(0x0102_0304_0506_0708),
            ty: CTy::Int(64),
            align: 8,
            vol: Volatility::Normal,
        },
        24,
    ));

    // View B reads a u64 at 0 — four bytes A wrote, four A did not — and a u32 at 8,
    // which is the low half of A's long.
    insts.push(inst(
        InstKind::Assign {
            dst: ValueId(3),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(1)),
                ty: CTy::Int(32),
                align: 8,
                vol: Volatility::Normal,
            },
        },
        30,
    ));
    insts.push(inst(
        InstKind::Assign {
            dst: ValueId(4),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(2)),
                ty: CTy::Int(32),
                align: 8,
                vol: Volatility::Normal,
            },
        },
        32,
    ));

    let (r, mut a) = run(insts, vec![]);
    let s = &r.states()[0];
    let val = |v: u32, a: &mut TermArena| match s.local(ValueId(v)) {
        Some(Value::Scalar(t)) => a.eval_ground(t).expect("concrete").bits(),
        other => panic!("%{v} is {other:?}"),
    };
    assert_eq!(
        val(3, &mut a),
        0x1122_3344,
        "the word view A wrote at 0 reads back byte for byte"
    );
    assert_eq!(
        val(4, &mut a),
        0x0506_0708,
        "and the low half of A's long at 8 is what B's word at 8 sees — little-endian, \
         so the *low* four bytes, not the high ones"
    );

    // No finding: 020 §4.5 says reading a member other than the last written is legal and
    // produces none. gcc defines it and VPP depends on it.
    assert!(
        r.findings().is_empty(),
        "reinterpreting the region is what the region is for: {:#?}",
        r.findings()
    );
}

/// **Contract 23, second half.** An out-of-bounds access through the second view is
/// reported with that view's member named.
///
/// `*(u64*)(opaque + 36)` reads four bytes past the region. Without the path the finding
/// says "out-of-bounds at offset 36 of ObjectId(1)", which tells a VPP developer nothing:
/// the region is reinterpreted by dozens of nodes and the offset alone does not say which
/// one. `opaque as l2_bridge_t.bd_index` names the struct that got it wrong.
#[test]
fn an_out_of_bounds_access_names_the_view_it_went_through() {
    let mut insts = address_at(36, 1, 10);
    insts.push(inst(
        InstKind::Assign {
            dst: ValueId(2),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(1)),
                ty: CTy::Int(64),
                align: 4,
                vol: Volatility::Normal,
            },
        },
        20,
    ));

    let path = AccessPath {
        root: PathRoot::Local {
            alloca: AllocaId(0),
            name: Some("opaque".into()),
        },
        steps: [PathStep::UnionMember {
            name: "bd_index".into(),
            off: 36,
            view: "l2_bridge_t".into(),
        }]
        .into_iter()
        .collect(),
    };
    let (r, _) = run(insts, vec![(ValueId(1), path)]);

    let oob: Vec<&str> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("out-of-bounds"))
        .map(|f| Box::leak(f.into_boxed_str()) as &str)
        .collect();
    assert_eq!(
        oob.len(),
        1,
        "eight bytes at 36 of a 40-byte region: {oob:#?}"
    );
    assert!(
        oob[0].contains("bd_index"),
        "the finding names the member the access went through: {}",
        oob[0]
    );
    assert!(
        oob[0].contains("l2_bridge_t"),
        "and the view it was viewed as, which is what says *which node* is wrong: {}",
        oob[0]
    );
}

/// **An access with no path still reports**, in the old form.
///
/// The path is reporting-only and optional; a finding must not depend on one existing.
/// This is also the control for the test above — without it, an implementation that only
/// ever reported when a path was present would look correct.
#[test]
fn an_out_of_bounds_access_without_a_path_still_reports() {
    let mut insts = address_at(36, 1, 10);
    insts.push(inst(
        InstKind::Assign {
            dst: ValueId(2),
            rv: RValue::Load {
                addr: Operand::Value(ValueId(1)),
                ty: CTy::Int(64),
                align: 4,
                vol: Volatility::Normal,
            },
        },
        20,
    ));
    let (r, _) = run(insts, vec![]);
    let oob: Vec<String> = r
        .findings()
        .into_iter()
        .filter(|f| f.contains("out-of-bounds"))
        .collect();
    assert_eq!(oob.len(), 1, "still reported: {oob:#?}");
    assert!(
        !oob[0].contains(" through "),
        "and says nothing about a view, because there is none: {}",
        oob[0]
    );
}
