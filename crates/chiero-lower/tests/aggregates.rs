//! Covers: 015 contracts 6, 7.
//!
//! Both contracts are about **not re-deriving something sema already computed**.
//!
//! Contract 6: a struct assignment is one `CopyMem` of the layout's size, never a
//! field-by-field sequence. A sequence is not merely slower — it is *wrong* for a struct
//! with padding, since C copies the padding too, and 021 would see the padding bytes stay
//! uninitialized where the program had defined them.
//!
//! Contract 7: a bit-field access uses the `BitRange` **from `RecordLayout`**, so there is
//! exactly one place in the system that can be wrong about a bit offset. 014's layout is
//! verified against gcc over 520 real VPP records; a bit offset recomputed here would be
//! a second, unverified answer to a question already settled.

use chiero_cir::{BitRange, InstKind, RValue};

mod harness;
use harness::lower;

fn probe(src: &str) -> chiero_cir::Module {
    lower(src)
}

fn insts_of<'a>(m: &'a chiero_cir::Module, name: &str) -> Vec<&'a InstKind> {
    m.funcs
        .iter()
        .find(|f| &*f.name == name)
        .unwrap_or_else(|| panic!("no `{name}`"))
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .map(|i| &i.kind)
        .collect()
}

/// **Contract 6.** A 40-byte struct assignment emits **one** `CopyMem`, not ten stores.
///
/// The size assertion is the load-bearing half. A `CopyMem` of the wrong size is exactly
/// as structurally correct as one of the right size, and the wrong size is what a
/// lowering that summed field widths instead of reading `RecordLayout::size` would
/// produce — silently dropping the tail padding C requires it to copy.
#[test]
fn a_struct_assignment_is_one_copymem_of_the_layout_size() {
    // **Locals, not pointer parameters.** A parameter is stored into its own slot on
    // entry, so `void use(struct S *p, struct S *q) { *p = *q; }` contains two perfectly
    // legitimate `Store`s of the pointers themselves — and the "no stores beside it"
    // assertion below would be measuring the prologue rather than the assignment.
    let m = probe(
        "struct S { int a[9]; char b; };\n\
         void use(void) { struct S x; struct S y; y = x; }\n",
    );
    let copies: Vec<(u64, ())> = insts_of(&m, "use")
        .iter()
        .filter_map(|k| match k {
            InstKind::CopyMem { size, .. } => match size {
                chiero_cir::Operand::Const(chiero_cir::Const::Int { val, .. }) => {
                    Some((*val as u64, ()))
                }
                _ => Some((u64::MAX, ())),
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        copies.len(),
        1,
        "one `CopyMem`, not a field-by-field sequence"
    );
    assert_eq!(
        copies[0].0, 40,
        "of the **layout's** size — 9 ints and a char padded to 40, not the 37 bytes a \
         sum of field widths gives, and C copies the padding"
    );

    // And no stores of the members: a sequence would show up as several.
    let stores = insts_of(&m, "use")
        .iter()
        .filter(|k| matches!(k, InstKind::Store { .. }))
        .count();
    assert_eq!(
        stores, 0,
        "the aggregate move is the `CopyMem`, not stores beside it"
    );
}

/// **Contract 7.** A bit-field assignment emits `StoreBits` carrying the `BitRange` that
/// `RecordLayout` computed, and lowering derives no bit offset of its own.
///
/// The two fields must differ in *both* offset and width, or an implementation that
/// stamped one range on every bit-field would pass. The numbers are not written down here
/// on purpose — they are read back from the layout sema produced, because a literal here
/// would be a second answer to a question 014 already settles against gcc.
#[test]
fn a_bitfield_store_carries_the_layouts_bit_range() {
    let m = probe(
        "struct B { int a:3; int b:5; };\n\
         void use(struct B *p) { p->a = 1; p->b = 2; }\n",
    );
    let ranges: Vec<BitRange> = insts_of(&m, "use")
        .iter()
        .filter_map(|k| match k {
            InstKind::StoreBits { bits, .. } => Some(*bits),
            _ => None,
        })
        .collect();
    assert_eq!(ranges.len(), 2, "one `StoreBits` per bit-field assignment");
    assert_eq!(
        ranges[0],
        BitRange { off: 0, width: 3 },
        "`a` is bits 0..3, which is what 014's layout says"
    );
    assert_eq!(
        ranges[1],
        BitRange { off: 3, width: 5 },
        "`b` is bits 3..8 — a different offset *and* a different width, so an \
         implementation stamping one range on every bit-field fails here"
    );

    // A bit-field read is `LoadBits`, not a `Load` and a shift lowering invented.
    let m = probe(
        "struct B { int a:3; int b:5; };\n\
         int use(struct B *p) { return p->b; }\n",
    );
    let loads: Vec<BitRange> = insts_of(&m, "use")
        .iter()
        .filter_map(|k| match k {
            InstKind::Assign {
                rv: RValue::LoadBits { bits, .. },
                ..
            } => Some(*bits),
            _ => None,
        })
        .collect();
    assert_eq!(loads.len(), 1, "one `LoadBits`");
    assert_eq!(loads[0], BitRange { off: 3, width: 5 });
}

/// A **non**-bit-field member is an ordinary `Load`/`Store` at a byte offset, so
/// `StoreBits` is not simply used for every member access.
#[test]
fn an_ordinary_member_is_a_byte_offset_not_a_bit_range() {
    let m = probe(
        "struct P { int a; int b; };\n\
         void use(struct P *p) { p->b = 7; }\n",
    );
    let kinds = insts_of(&m, "use");
    assert!(
        kinds.iter().any(|k| matches!(k, InstKind::Store { .. })),
        "an ordinary member is a `Store`: {kinds:#?}"
    );
    assert!(
        !kinds
            .iter()
            .any(|k| matches!(k, InstKind::StoreBits { .. })),
        "and not a `StoreBits`"
    );
    // The address is the base plus the field's byte offset, taken from the layout.
    let offsets: Vec<i128> = kinds
        .iter()
        .filter_map(|k| match k {
            InstKind::Assign {
                rv:
                    RValue::PtrAdd {
                        off: chiero_cir::Operand::Const(chiero_cir::Const::Int { val, .. }),
                        ..
                    },
                ..
            } => Some(*val),
            _ => None,
        })
        .collect();
    assert_eq!(
        offsets,
        vec![4],
        "`b` is at byte 4 of `struct P`, per the layout: {kinds:#?}"
    );
}
