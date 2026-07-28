//! Arenas — 021 contracts 13c and 13d.
//!
//! Covers: 021 contracts 13c, 13d.
//!
//! §5.2 exists because §5.1 step 4 would otherwise fire on the most-executed function in
//! VPP's data plane. `vlib_buffer_ptr_from_index` is
//!
//! ```c
//! offset += ((uword) buffer_index) << CLIB_LOG2_CACHE_LINE_BYTES;
//! return uword_to_pointer (buffer_mem_start + offset, vlib_buffer_t *);
//! ```
//!
//! — a pure `IntToPtr` over an arithmetic term whose base, under UCSE, is an
//! unconstrained 64-bit symbol. Without help "every VPP node analysis dies at its first
//! buffer access".
//!
//! **Three sizes, not two.** `index_scale` is the byte width of the program's index unit
//! (64, a cache line); `pitch` is the distance between consecutive elements
//! (`vlib_buffer_alloc_size()`, ~2.5 KB); `elem_size` is the addressable extent within
//! one element. Collapsing `index_scale` into `pitch` forces a choice between an
//! `elem_size` of 64 — making every access to `b->data` out of bounds — and overlapping
//! elements, which violates §7's disjointness. Both are wrong; the layout simply has two
//! strides.
//!
//! ## The design decision this file pins
//!
//! §5.2 decomposes an address into element index `k = (n * index_scale) / pitch` and
//! within-element offset `d = (n * index_scale) % pitch`. With `n` symbolic **both are
//! symbolic**, and that is the whole difficulty: an access at `d + δ` cannot be shown
//! in bounds without knowing something about `d`.
//!
//! Three options were considered:
//!
//! 1. *Assume `d == 0`* — that the index names an element start. Cheap, matches every
//!    well-formed VPP index, and **silently discards the paths where a buffer index is
//!    wrong**, which is precisely the bug an arena would otherwise be able to find. A
//!    modelling choice that deletes the bug class it was built for is not a modelling
//!    choice, it is a blind spot.
//! 2. *Report the gap whenever it is feasible* — sound, and useless: for an unconstrained
//!    index `d >= elem_size` is always feasible, so every buffer access in VPP reports an
//!    out-of-bounds and the analysis drowns.
//! 3. **Fork.** One state where `d == 0`, carrying a well-formed element pointer, and one
//!    where `d` lands in the `[elem_size, pitch)` gap, carrying exactly one finding. Two
//!    states, not a fork over the address space — which is what §5.2 step 4 forbids when
//!    it says "one object per accessed index, materialized lazily, rather than a fork
//!    over the whole address space".
//!
//! Option 3 is what these tests require. It keeps the bug reachable, keeps the ordinary
//! path clean, and the extra state is bounded by 2 per arena access rather than by the
//! object count.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::{SmtLib, TermArena};
use chiero_span::Span;

/// VPP's real geometry (021 §5.2, and 010's measurements of it).
fn vpp_buffer_pool() -> ArenaShape {
    ArenaShape {
        // `vlib_buffer_alloc_size()` — round_pow2(ext_hdr + sizeof(vlib_buffer_t) +
        // data_size, VLIB_BUFFER_ALIGN), on the order of 2.5 KB.
        pitch: 2496,
        // The buffer's addressable extent. Smaller than the pitch, so `[elem_size, pitch)`
        // is a real gap rather than a rounding artifact.
        elem_size: 2432,
        // `CLIB_LOG2_CACHE_LINE_BYTES` = 6.
        index_scale: 64,
        count: 1024,
    }
}

fn assign(dst: u32, rv: RValue) -> Inst {
    Inst {
        kind: InstKind::Assign {
            dst: ValueId(dst),
            rv,
        },
        span: Span::DUMMY,
    }
}

/// `char *p = (char *)(base + (i << 6)); return p[delta];`
///
/// `base` is parameter 0 and `i` is parameter 1, both unconstrained — the shape UCSE
/// hands every VPP node function. `delta` is a concrete offset within the element, which
/// is what `b->data` looks like after lowering.
fn buffer_access(delta: i128) -> Module {
    let f = Function {
        id: FuncId(0),
        name: "node".into(),
        params: vec![
            Param {
                value: ValueId(0),
                ty: CTy::Int(64),
            },
            Param {
                value: ValueId(1),
                ty: CTy::Int(64),
            },
        ],
        ret: CTy::Int(8),
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![
                // %2 = i << 6
                assign(
                    2,
                    RValue::Bin {
                        op: BinOp::Shl,
                        ty: CTy::Int(64),
                        a: Operand::Value(ValueId(1)),
                        b: Operand::Const(Const::Int { bits: 64, val: 6 }),
                    },
                ),
                // %3 = base + %2
                assign(
                    3,
                    RValue::Bin {
                        op: BinOp::Add,
                        ty: CTy::Int(64),
                        a: Operand::Value(ValueId(0)),
                        b: Operand::Value(ValueId(2)),
                    },
                ),
                // %4 = (char *) %3
                assign(
                    4,
                    RValue::Cast {
                        kind: CastKind::IntToPtr,
                        a: Operand::Value(ValueId(3)),
                        from: CTy::Int(64),
                        to: CTy::Ptr,
                    },
                ),
                // %5 = %4 + delta
                assign(
                    5,
                    RValue::PtrAdd {
                        base: Operand::Value(ValueId(4)),
                        off: Operand::Const(Const::Int {
                            bits: 64,
                            val: delta,
                        }),
                    },
                ),
                assign(
                    6,
                    RValue::Load {
                        addr: Operand::Value(ValueId(5)),
                        ty: CTy::Int(8),
                        align: 1,
                        vol: Volatility::Normal,
                    },
                ),
            ],
            term: Terminator::Return(Some(Operand::Value(ValueId(6)))),
            gcov_lines: Default::default(),
            span: Span::DUMMY,
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        span: Span::DUMMY,
    };
    Module {
        funcs: vec![f],
        ..Default::default()
    }
}

/// **Without an arena this is §5.1 step 4**, and the test says so first — otherwise a
/// later assertion about arenas working could be satisfied by anything at all happening.
#[test]
fn without_an_arena_a_buffer_access_is_an_unresolvable_pointer() {
    let m = buffer_access(128);
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let msgs: Vec<String> = r.reports().iter().map(|f| f.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("unresolvable pointer")),
        "an `IntToPtr` over an unconstrained symbol is step 4: {msgs:?}"
    );
    assert_eq!(
        r.fidelity(),
        Fidelity::Unknown,
        "and the run knows it learned nothing about that memory"
    );
}

/// **021 contract 13c.** With the arena registered, `base + (i << 6)` resolves to element
/// `(i*64)/pitch` at offset `(i*64)%pitch`, `i` stays symbolic, `k` is bounds-checked
/// against `count`, and there is **no fork over unrelated objects**.
///
/// The access is at offset 128 — well past the 64-byte index unit, which is the whole
/// reason `elem_size` is a third number. If `index_scale` and `pitch` were collapsed this
/// would be out of bounds.
#[test]
fn a_symbolic_buffer_index_resolves_into_one_element() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = buffer_access(128);
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_backend(backend)
        .with_arena(0, vpp_buffer_pool())
        .run(&mut a);

    // Two states: the well-formed one and the gap one (see the header note). Not more —
    // "one object per accessed index", never a sweep over the address space.
    assert_eq!(
        r.states().len(),
        2,
        "an arena access forks once, into the element and the inter-element gap"
    );

    let good = r
        .states()
        .iter()
        .find(|s| s.findings().is_empty())
        .expect("one state reaches the element cleanly");
    assert!(
        good.assumptions()
            .iter()
            .any(|x| x.detail.contains("arena") || x.detail.contains("element")),
        "and it says the resolution rested on the arena's declared geometry: {:#?}",
        good.assumptions()
            .iter()
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
    // 128 is inside a 2432-byte element and far outside a 64-byte one. This is the
    // assertion that fails if `index_scale` and `pitch` are ever collapsed.
    assert!(
        good.findings().is_empty(),
        "`b->data` at +128 is in bounds for a 2432-byte element: {:#?}",
        good.findings().iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}

/// **021 contract 13d.** Elements are disjoint and separated per §7; an offset landing in
/// the `[elem_size, pitch)` gap is **exactly one** OOB finding, and not a valid pointer
/// into the next element.
///
/// The "not the next element" half is the one that matters. A model that lets the gap
/// address resolve into element `k+1` reports nothing and analyses the wrong buffer —
/// silently, and for the rest of the function.
#[test]
fn an_offset_in_the_inter_element_gap_is_one_finding_and_not_the_next_element() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let m = buffer_access(128);
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_backend(backend)
        .with_arena(0, vpp_buffer_pool())
        .run(&mut a);

    let gap: Vec<&_> = r
        .states()
        .iter()
        .filter(|s| !s.findings().is_empty())
        .collect();
    assert_eq!(gap.len(), 1, "one state explores the gap");
    assert_eq!(
        gap[0].findings().len(),
        1,
        "and reports it exactly once: {:#?}",
        gap[0]
            .findings()
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    let msg = &gap[0].findings()[0].message;
    assert!(
        msg.contains("gap") || msg.contains("out of bounds"),
        "the finding names what happened: {msg}"
    );
}

/// **The bounds check against `count` is real.** An index the arena cannot contain is not
/// silently wrapped into a valid element — §5.2 step 4 says `k` "is bounds-checked
/// against `count`", and an arena that resolves every index is an arena that has stopped
/// being a bound.
#[test]
fn an_index_beyond_count_does_not_resolve_to_an_element() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    // One element, so almost every index is out of range.
    let shape = ArenaShape {
        count: 1,
        ..vpp_buffer_pool()
    };
    let m = buffer_access(0);
    let mut a = TermArena::new();
    let r = Engine::new(&m)
        .with_backend(backend)
        .with_arena(0, shape)
        .run(&mut a);
    let msgs: Vec<String> = r
        .states()
        .iter()
        .flat_map(|s| s.findings())
        .map(|f| f.message.clone())
        .collect();
    assert!(
        msgs.iter()
            .any(|m| m.contains("count") || m.contains("out of bounds")),
        "an index past the arena's element count is reported: {msgs:?}"
    );
}
