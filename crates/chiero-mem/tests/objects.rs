//! The object/offset memory model — objects, byte contents, and the initialization mask.
//!
//! Covers **021 contracts 1, 2, 4, 5** and the bit-granular half of **3.1**.
//!
//! The load-bearing decisions being tested here, from 021 §3 and §3.1:
//!
//! - **Bytes are bytes.** No strict-aliasing assumptions: reading an `Int(32)` from bytes
//!   written as a `Float(F32)` yields the bit reinterpretation, because that is what the
//!   hardware does and what VPP's packet code relies on.
//! - **Initialization is tracked at *bit* granularity**, not per byte. `LoadBits` exists
//!   so a bitfield touches only its own bits, which is pointless unless `init` has the
//!   same resolution — `session_types.h` packs nine bitfields into one `u32`, several of
//!   them unnamed padding nobody writes.
//! - **Reading uninitialized bytes yields a fresh symbol *plus* a finding**, never zero.
//!   Silently reading zero is the single most common way a symbolic executor produces
//!   confidently wrong results.
//! - **Offsets are signed.** vppinfra's vector header lives *below* the user pointer, so
//!   a model with unsigned offsets cannot express `vec_len(v)` at all.

use chiero_mem::*;
use chiero_span::Span;

fn obj(size: u64) -> MemObject {
    MemObject::new_stack(ObjectId(1), size, 8, Span::DUMMY)
}

/// **021 contract 1 — the `vec_header` case, and the reason offsets are signed.**
///
/// vppinfra puts the vector header *below* the user pointer: `vec_len(v)` reads
/// `((vec_header_t *)v)[-1].len`. A memory model with unsigned offsets cannot express
/// that access, so it would have to treat every vector length read as out of bounds.
#[test]
fn a_negative_offset_from_the_user_pointer_reads_the_header() {
    let mut o = obj(64);
    // The user pointer is at offset 8; bytes 0..8 are the header.
    o.write_int(0, 4, 0xAABB_CCDD, Endian::Little).unwrap();

    let user = 8i64;
    let r = o.read_int(user - 8, 4, Endian::Little).expect("in bounds");
    assert_eq!(r, 0xAABB_CCDD, "the header must be readable from below");

    // Below the object entirely.
    assert!(
        matches!(
            o.read_int(user - 16, 4, Endian::Little),
            Err(AccessError::OutOfBounds { .. })
        ),
        "-16 from the user pointer is before the object"
    );
}

/// **021 contract 2 — must-OOB terminates.**
///
/// At a *concrete* offset past the end the access is out of bounds under every model, so
/// there is nothing to continue with: "continue with the in-bounds constraint" would
/// continue a state whose path condition is unsatisfiable, which 023 §3 treats as a
/// chiero bug rather than a finding.
#[test]
fn a_write_straddling_the_end_is_out_of_bounds() {
    let mut o = obj(64);
    o.write_int(60, 4, 1, Endian::Little)
        .expect("the last four bytes are in bounds");
    match o.write_int(61, 4, 1, Endian::Little) {
        Err(AccessError::OutOfBounds { off, size, .. }) => {
            assert_eq!((off, size), (61, 4));
        }
        other => panic!("61..65 straddles the end of a 64-byte object, got {other:?}"),
    }
}

/// **021 contract 4: bytes are bytes.** Writing a `Float(F32)` `1.0` and reading an
/// `Int(32)` at the same address yields `0x3F800000` on a little-endian target. This is
/// type punning working by construction rather than by special case.
#[test]
fn type_punning_a_float_through_an_int_reinterprets_the_bits() {
    let mut o = obj(16);
    o.write_bytes(0, &1.0f32.to_le_bytes()).unwrap();
    assert_eq!(
        o.read_int(0, 4, Endian::Little).unwrap(),
        0x3F80_0000,
        "reading an i32 over f32 bytes is the bit reinterpretation"
    );
}

/// Endianness comes from the target, and a model that ignored it would silently produce
/// byte-swapped answers for every multi-byte access — the kind of wrong that looks right.
#[test]
fn multibyte_access_respects_target_byte_order() {
    let mut o = obj(16);
    o.write_int(0, 4, 0x1122_3344, Endian::Big).unwrap();
    assert_eq!(o.read_bytes(0, 4).unwrap(), vec![0x11, 0x22, 0x33, 0x44]);
    assert_eq!(o.read_int(0, 4, Endian::Big).unwrap(), 0x1122_3344);
    // The same bytes read little-endian are the reverse. A model that hardcoded one
    // order would pass the round trip above and still be wrong.
    assert_eq!(o.read_int(0, 4, Endian::Little).unwrap(), 0x4433_2211);
}

/// **021 §3.1: reading uninitialized bytes is a finding, not a zero.**
#[test]
fn reading_a_never_written_byte_is_an_uninitialized_read() {
    let o = obj(16);
    match o.read_int(0, 4, Endian::Little) {
        Err(AccessError::Uninitialized { off, .. }) => assert_eq!(off, 0),
        other => panic!("a never-written byte must not read as zero, got {other:?}"),
    }
}

/// A partially written multi-byte read is still an uninitialized read: three of four
/// bytes initialized is not "initialized".
#[test]
fn a_partially_initialized_read_is_still_uninitialized() {
    let mut o = obj(16);
    o.write_bytes(0, &[1, 2, 3]).unwrap();
    assert!(matches!(
        o.read_int(0, 4, Endian::Little),
        Err(AccessError::Uninitialized { .. })
    ));
    o.write_bytes(3, &[4]).unwrap();
    assert_eq!(o.read_int(0, 4, Endian::Little).unwrap(), 0x0403_0201);
}

/// **021 §3.1's bit granularity, stated as the case that forces it.**
///
/// In `struct { u32 a:3; u32 b:5; }` both fields live in byte 0. Writing `a` and reading
/// `a` must produce no finding; reading `b` must produce one. A per-byte mask can only
/// answer "yes" to both (missing every real uninitialized-bitfield read) or "no" to both
/// (firing on every correct one). VPP settles it: `session_types.h` packs nine bitfields
/// into one `u32`, several of them unnamed padding that is never written.
#[test]
fn initialization_is_tracked_per_bit_not_per_byte() {
    let mut o = obj(8);
    // `a` is bits 0..3 of byte 0.
    o.write_bits(0, 3, 0b101).unwrap();
    assert_eq!(o.read_bits(0, 3).unwrap(), 0b101, "`a` reads back");
    // `b` is bits 3..8 — same byte, never written.
    assert!(
        matches!(o.read_bits(3, 5), Err(AccessError::Uninitialized { .. })),
        "`b` shares byte 0 with `a` and was never written"
    );
}

/// The tri-state's third value: a byte written under a condition is neither definitely
/// initialized nor definitely not. Forcing it to `Yes` loses real uninitialized reads;
/// forcing it to `No` produces a false-positive storm on `v[i] = x; … use v[i]`, which is
/// ubiquitous. 021 §3.1 requires the distinction to exist at all.
#[test]
fn a_conditionally_written_byte_is_neither_initialized_nor_not() {
    let mut arena = chiero_solver::TermArena::new();
    let gv = arena.var(chiero_solver::Sort::BitVec(8), "g");
    let gk = arena.bv(8, 1);
    let guard = arena.eq(gv, gk);
    let mut o = obj(8);
    o.write_bytes_cond(0, &[7], Cond::Symbolic, Some(guard))
        .unwrap();
    assert!(
        matches!(o.init_bit(0), InitBit::Cond(_)),
        "not Yes, and not No"
    );
    assert_eq!(o.init_bit(8), InitBit::No, "byte 1 is untouched");
}

/// **`Cond` is neither `Yes` nor `No`, and a read must produce a third outcome.**
///
/// An earlier version of this test asserted a *definite* `Uninitialized` for a
/// conditionally-written byte. **021 contract 6b says the opposite**: a read at a
/// conditionally-written offset does not report an uninitialized read. Both obvious
/// readings are wrong and §3.1 says so — forcing `Cond` to "yes" loses real
/// uninitialized reads, forcing it to "no" produces the false-positive storm on
/// `v[i] = x; … use v[i]` that is ubiquitous. So the read is reported *conditionally*,
/// with the guard left for the engine to discharge against the path condition.
#[test]
fn reading_through_a_conditionally_written_byte_is_conditionally_reported() {
    let mut arena = chiero_solver::TermArena::new();
    let gv = arena.var(chiero_solver::Sort::BitVec(8), "g");
    let gk = arena.bv(8, 1);
    let guard = arena.eq(gv, gk);
    let mut o = obj(8);
    o.write_bytes_cond(0, &[7], Cond::Symbolic, Some(guard))
        .unwrap();
    assert!(
        matches!(
            o.read_bytes(0, 1),
            Err(AccessError::MaybeUninitialized { .. })
        ),
        "not silently accepted, and not a definite finding either"
    );
    // And an unconditional write over it settles the question.
    o.write_bytes(0, &[9]).unwrap();
    assert_eq!(o.read_bytes(0, 1).unwrap(), vec![9]);
}

/// Writes are tracked, so `write` then `read` at the same place never reports
/// uninitialized. Without this the test above is satisfied by a model that reports
/// `Uninitialized` for everything.
#[test]
fn a_written_byte_reads_back_initialized() {
    let mut o = obj(8);
    for i in 0..8i64 {
        o.write_bytes(i, &[i as u8 + 1]).unwrap();
    }
    assert_eq!(o.read_bytes(0, 8).unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    for bit in 0..64 {
        assert_eq!(o.init_bit(bit), InitBit::Yes, "bit {bit}");
    }
}

/// An access wholly before the object is as much out of bounds as one past the end, and
/// the finding must say which offset — a finding that cannot name the access is not
/// actionable.
#[test]
fn out_of_bounds_findings_name_the_offset_and_size() {
    let o = obj(16);
    match o.read_bytes(-4, 2) {
        Err(AccessError::OutOfBounds {
            off,
            size,
            obj_size,
        }) => {
            assert_eq!((off, size, obj_size), (-4, 2, 16));
        }
        other => panic!("expected OOB, got {other:?}"),
    }
}

/// A zero-size access is degenerate but legal C (`memcpy(p, q, 0)`), and must not be
/// reported as out of bounds at the boundary — one past the end with size 0 is exactly
/// where a loop's final `p + n` lands.
#[test]
fn a_zero_size_access_one_past_the_end_is_in_bounds() {
    let o = obj(16);
    assert_eq!(o.read_bytes(16, 0).unwrap(), Vec::<u8>::new());
    assert!(matches!(
        o.read_bytes(17, 0),
        Err(AccessError::OutOfBounds { .. })
    ));
}

// ---------------------------------------------------------------------------
// Wave 8, from the mutation review. Every one probed before being acted on.
// ---------------------------------------------------------------------------

/// **The bounds check does not bound: a `size_t` underflow reads as in-bounds.**
///
/// `size as i64` is a wrapping cast, so any `size >= 2^63` is negative, the computed end
/// lands at or below the offset, and the check passes. The trigger is the canonical
/// overflow bug — `clib_memcpy(d, s, a - b)` with `a < b` — and the observed result was
/// an *uninitialized-read* finding for a 16-exabyte overflow: a real buffer overflow
/// silently reclassified as a different, milder bug.
#[test]
fn a_size_underflow_is_out_of_bounds_not_something_milder() {
    let o = obj(16);
    match o.read_bytes(0, u64::MAX - 3) {
        Err(AccessError::OutOfBounds { .. }) => {}
        other => panic!("a wrapped size must be out of bounds, got {other:?}"),
    }
    let mut o = obj(16);
    assert!(matches!(
        o.write_bytes_cond(0, &[0], Cond::Always, None),
        Ok(())
    ));
    assert!(matches!(
        o.read_bytes(8, u64::MAX / 2),
        Err(AccessError::OutOfBounds { .. })
    ));
}

/// **A conditional write must not *downgrade* a definitely-initialized byte.**
///
/// 021 §3.1 defines the conditional write as `ite(off == k, val, old)`. If `old` is
/// already `Yes`, both branches are initialized, so the join is `Yes`. Assigning `Cond`
/// unconditionally reintroduces exactly the false-positive storm on `v[i] = x; … use
/// v[i]` that the tri-state exists to prevent — and now on code that was *definitely*
/// initialized before the loop, which is the common shape: `memset(v, 0, n)` and then a
/// guarded write.
#[test]
fn a_conditional_write_over_initialized_memory_stays_initialized() {
    let mut arena = chiero_solver::TermArena::new();
    let gv = arena.var(chiero_solver::Sort::BitVec(8), "g");
    let gk = arena.bv(8, 1);
    let guard = arena.eq(gv, gk);
    let mut o = obj(8);
    o.write_bytes(0, &[0; 8]).unwrap(); // memset
    o.write_bytes_cond(0, &[1], Cond::Symbolic, Some(guard))
        .unwrap(); // v[i] = x
    assert_eq!(
        o.init_bit(0),
        InitBit::Yes,
        "both branches of the ite are initialized, so the join is initialized"
    );
    assert!(o.read_bytes(0, 1).is_ok(), "and the read must not fire");
}

/// The join is one-directional: a conditional write over *uninitialized* memory is still
/// `Cond`. Without this the fix above is just "always Yes", which loses every real
/// uninitialized read.
#[test]
fn a_conditional_write_over_uninitialized_memory_is_still_conditional() {
    let mut arena = chiero_solver::TermArena::new();
    let gv = arena.var(chiero_solver::Sort::BitVec(8), "g");
    let gk = arena.bv(8, 1);
    let guard = arena.eq(gv, gk);
    let mut o = obj(8);
    o.write_bytes_cond(0, &[1], Cond::Symbolic, Some(guard))
        .unwrap();
    assert!(matches!(o.init_bit(0), InitBit::Cond(_)));
}

/// **Bitfields wider than the value type corrupt memory silently.** Rust masks shift
/// amounts when overflow checks are off, so `v >> 128` is `v >> 0` and bit 128 of the
/// field gets bit 0 of the value. Nothing bounded a bitfield to the payload width, and
/// `__int128` bitfields plus over-wide `LoadBits` units are reachable.
#[test]
fn a_bitfield_wider_than_the_payload_is_rejected() {
    let mut o = obj(32);
    assert!(matches!(
        o.write_bits(0, 160, 1),
        Err(AccessError::BadRange { .. })
    ));
    assert!(matches!(
        o.read_bits(0, 129),
        Err(AccessError::BadRange { .. })
    ));
    // The boundary itself is fine.
    assert!(o.write_bits(0, 128, 1).is_ok());
}

/// `check_bits` must not overflow before it compares. `lo_bit + n_bits` wraps, the
/// wrapped sum passes the check, and the indexing that follows panics — a crash rather
/// than a finding, reachable from any bit offset derived from a wrapped pointer
/// difference.
#[test]
fn a_wrapping_bit_range_is_rejected_rather_than_panicking() {
    let o = obj(16);
    assert!(matches!(
        o.read_bits(u64::MAX - 63, 64),
        Err(AccessError::OutOfBounds { .. })
    ));
}

/// **`read_int`/`write_int` above the payload width.** Writing 20 bytes duplicated the
/// value's low bytes at offset 16 and returned `Ok`; reading 20 bytes silently narrowed
/// to 16 and reported nothing. The two were not inverses and neither said so.
#[test]
fn an_integer_access_wider_than_the_payload_is_rejected() {
    let mut o = obj(32);
    assert!(matches!(
        o.write_int(0, 20, 0xDEAD, Endian::Little),
        Err(AccessError::BadRange { .. })
    ));
    o.write_bytes(0, &[0xFF; 20]).unwrap();
    assert!(matches!(
        o.read_int(0, 20, Endian::Little),
        Err(AccessError::BadRange { .. })
    ));
}

/// `read_int` and `write_int` are exact inverses at every legal size, in both orders.
/// This is the companion to the rejection above — otherwise "reject everything" passes.
#[test]
fn integer_access_round_trips_at_every_legal_size() {
    for size in 1..=16u64 {
        for e in [Endian::Little, Endian::Big] {
            let mut o = obj(32);
            let v = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210u128 >> (8 * (16 - size));
            o.write_int(0, size, v, e).unwrap();
            assert_eq!(o.read_int(0, size, e).unwrap(), v, "size {size}, {e:?}");
        }
    }
}

/// **021 §4: `readonly` globals reject writes with a finding.** The field existed and
/// nothing read it — a `pub` field that looks like a safety property but is inert is
/// worse than no field at all. Contract 21 also requires the bytes to be unchanged.
#[test]
fn a_readonly_object_rejects_writes_and_keeps_its_bytes() {
    let mut o = MemObject::new(ObjectId(2), ObjKind::Global, 8, 8, Span::DUMMY);
    o.write_bytes(0, &[1, 2, 3, 4]).unwrap();
    o.readonly = true;
    assert!(matches!(
        o.write_bytes(0, &[0xEE]),
        Err(AccessError::ReadOnly { .. })
    ));
    assert!(matches!(
        o.write_bits(0, 4, 0xF),
        Err(AccessError::ReadOnly { .. })
    ));
    assert_eq!(
        o.read_bytes(0, 4).unwrap(),
        vec![1, 2, 3, 4],
        "a rejected write must not alter the bytes"
    );
}

/// **021 contract 24: `StoreBits` to `a` leaves every bit of `b` unchanged.** The crate's
/// headline claim, and it had no test in the direction that matters — the existing one
/// writes `a` and only asserts `b` is uninitialized, never writing `b` at all.
///
/// The value is deliberately not a palindrome. `0b101` is bit-reversal-invariant in three
/// bits, so a model that reversed bit order passed the old test: the measuring apparatus
/// could not see the fault it was aimed at.
#[test]
fn writing_one_bitfield_leaves_its_neighbour_untouched() {
    let mut o = obj(8);
    o.write_bits(0, 3, 0b110).unwrap(); // `a`, not a palindrome
    o.write_bits(3, 5, 0b10011).unwrap(); // `b`, same byte
    assert_eq!(o.read_bits(0, 3).unwrap(), 0b110, "writing b disturbed a");
    assert_eq!(o.read_bits(3, 5).unwrap(), 0b10011);
    // And back the other way.
    o.write_bits(0, 3, 0b001).unwrap();
    assert_eq!(
        o.read_bits(3, 5).unwrap(),
        0b10011,
        "rewriting a disturbed b"
    );
}

/// A zero bit written over a one bit must actually clear. `|=` instead of a masked
/// assignment loses it, and no earlier test ever wrote a zero over a one.
#[test]
fn writing_a_zero_bit_clears_it() {
    let mut o = obj(8);
    o.write_bits(0, 8, 0xFF).unwrap();
    o.write_bits(0, 8, 0x00).unwrap();
    assert_eq!(o.read_bits(0, 8).unwrap(), 0);
}

/// Bit accesses away from offset zero. Every earlier bit test used `lo_bit == 0` on the
/// value path, so a model that discarded `lo_bit` entirely passed the whole suite.
#[test]
fn bit_access_works_away_from_the_first_byte() {
    let mut o = obj(8);
    o.write_bits(20, 12, 0xABC).unwrap();
    assert_eq!(o.read_bits(20, 12).unwrap(), 0xABC);
    assert_eq!(o.init_bit(20), InitBit::Yes);
    assert_eq!(o.init_bit(19), InitBit::No, "bit 19 was not written");
    assert_eq!(o.init_bit(32), InitBit::No, "bit 32 was not written");
}

/// Byte accesses away from offset zero, for the same reason: every successful read in the
/// suite was at offset 0, so "read from offset 0 always" survived. Contract 1's positive
/// case reads at `user - 8`, which *evaluates* to 0 — only its negative half had teeth.
#[test]
fn byte_access_works_away_from_the_start_of_the_object() {
    let mut o = obj(16);
    o.write_bytes(9, &[1, 2, 3]).unwrap();
    assert_eq!(o.read_bytes(9, 3).unwrap(), vec![1, 2, 3]);
    assert!(matches!(
        o.read_bytes(8, 3),
        Err(AccessError::Uninitialized { .. })
    ));
}

/// The location fields in a finding are asserted at a non-zero location, since the type's
/// own doc-comment says a finding that cannot say *where* is not actionable — and every
/// existing assertion was at offset 0, where a model reporting 0 unconditionally passes.
#[test]
fn an_uninitialized_finding_names_the_first_bad_bit() {
    let mut o = obj(16);
    o.write_bytes(0, &[1, 2, 3]).unwrap();
    match o.read_bytes(0, 4) {
        Err(AccessError::Uninitialized { off, bit }) => {
            assert_eq!(off, 0);
            assert_eq!(bit, 24, "byte 3 is the first uninitialized one");
        }
        other => panic!("expected Uninitialized, got {other:?}"),
    }
}

/// 021 §1's reserved objects. Nothing asserted them, so swapping the two constants
/// survived — and they are the difference between a null-dereference finding and a
/// wild-pointer one.
#[test]
fn the_reserved_object_ids_are_what_the_spec_says() {
    assert_eq!(ObjectId::NULL, ObjectId(0));
    assert_ne!(ObjectId::UNBOUND, ObjectId::NULL);
}

/// **The cap has to be a size chiero can actually materialize.** `MAX_MATERIALIZED_BYTES`
/// was 1 GiB and the init mask held one `InitBit` — eight bytes — *per bit*, so an object
/// at exactly the cap asked the host for 64 GiB of mask and aborted the process. The
/// guard was checking the program's number, not chiero's cost.
///
/// A test that allocates *under* the cap cannot see this; the boundary is the whole
/// point, so this allocates exactly at it and then uses the object.
#[test]
fn an_object_at_the_cap_can_be_built_and_used() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, MAX_MATERIALIZED_BYTES, 16, Span::DUMMY);
    let p = Pointer { base: o, off: 0 };
    let r = m.set(p, 7, MAX_MATERIALIZED_BYTES, Span::DUMMY);
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    // The last byte, not the first: a mask that ran short would still answer for byte 0.
    let last = Pointer {
        base: o,
        off: (MAX_MATERIALIZED_BYTES - 1) as i64,
    };
    let r = m.read(last, 1, Span::DUMMY);
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    assert_eq!(r.value, Some(vec![7]));
}

/// The mask's three states survive a compact representation. `Cond` is sparse and `Yes`
/// is a bitset, so the interesting case is the *interaction*: an unconditional write over
/// a conditional one wins, and a conditional write over an unconditional one does not
/// downgrade it (021 §3.1's join).
#[test]
fn the_compact_mask_keeps_the_three_states_apart() {
    let mut a = chiero_solver::TermArena::new();
    let g = a.var(chiero_solver::Sort::Bool, "g");
    let mut mask = InitMask::new(4);
    mask.set_range(0, 8, InitBit::Cond(g));
    mask.set_range(8, 8, InitBit::Yes);
    assert_eq!(mask.get(3), InitBit::Cond(g));
    assert_eq!(mask.get(9), InitBit::Yes);
    assert_eq!(mask.get(20), InitBit::No);
    // Yes over Cond: definite.
    mask.set_range(0, 8, InitBit::Yes);
    assert_eq!(mask.get(3), InitBit::Yes);
    // Cond over Yes: still definite — the false-positive storm the tri-state prevents.
    mask.set_range(8, 8, InitBit::Cond(g));
    assert_eq!(mask.get(9), InitBit::Yes);
    assert_eq!(mask.first_no(0, 16), None);
    assert_eq!(mask.first_no(0, 32), Some(16));
    assert_eq!(mask.first_cond(0, 32), None);
}

/// **The mask has a canonical form**, and it has to, because `MemObject` derives
/// `PartialEq`: two objects that agree about every bit must compare equal. With a `Yes`
/// bitset and a sparse `Cond` map the same meaning has two possible encodings — a `Cond`
/// entry shadowed by a set `Yes` bit — and `get` cannot tell them apart, so only equality
/// can. Leaving the stale entries also grows the map without bound over a loop that
/// writes conditionally and then definitely.
#[test]
fn a_definite_write_erases_the_guard_it_covers() {
    let mut a = chiero_solver::TermArena::new();
    let g = a.var(chiero_solver::Sort::Bool, "g");
    let mut viaz = InitMask::new(2);
    viaz.set_range(0, 16, InitBit::Cond(g));
    viaz.set_range(0, 16, InitBit::Yes);
    let mut direct = InitMask::new(2);
    direct.set_range(0, 16, InitBit::Yes);
    assert_eq!(
        viaz, direct,
        "the guard is spent once the write is definite"
    );
}

/// **A conditional write with no guard changes nothing about initialization.** 021 §3.1's
/// join is `join(old, No) == old` in every case, which is what makes this safe in both
/// directions: it cannot erase an initialization (a false uninitialized-read report on
/// memory that was written), and it cannot create one (a missed report on memory that was
/// not). The bytes still land — this is about what chiero *claims* it knows.
#[test]
fn a_guardless_conditional_write_neither_initializes_nor_erases() {
    let mut o = MemObject::new(ObjectId(1), ObjKind::Heap, 4, 1, Span::DUMMY);
    o.write_bytes(0, &[1, 2]).unwrap();
    o.write_bytes_cond(0, &[9, 9], Cond::Symbolic, None)
        .unwrap();
    o.write_bytes_cond(2, &[9, 9], Cond::Symbolic, None)
        .unwrap();
    assert_eq!(o.read_bytes(0, 2).unwrap(), vec![9, 9], "the bytes land");
    assert_eq!(o.init_bit(0), InitBit::Yes, "still initialized");
    // The other half stays reportable — reading it is an error, not a value.
    assert_eq!(o.init_bit(16), InitBit::No, "still uninitialized");
    assert_eq!(
        o.read_bytes(2, 2),
        Err(AccessError::Uninitialized { off: 2, bit: 16 })
    );
}

/// **021 contract 20.** Forking a state with 1000 objects and writing one leaves the other
/// 999 shared — checked by pointer equality, because that is the only way to see the
/// difference between sharing and an identical copy.
///
/// Forking is the engine's core operation: a symbolic branch in a loop over
/// `VLIB_FRAME_SIZE` buffers forks hundreds of times, and deep-copying every object at
/// each fork makes the cost quadratic in a program's memory rather than in its branching.
/// 021 specifies structural sharing for exactly that reason.
#[test]
fn forking_shares_every_object_it_did_not_write() {
    let mut m = Memory::new();
    let objs: Vec<_> = (0..1000)
        .map(|i| {
            let o = m.alloc(ObjKind::Heap, 8, 8, Span::DUMMY);
            m.set(Pointer { base: o, off: 0 }, (i % 256) as u8, 8, Span::DUMMY);
            o
        })
        .collect();
    let mut forked = m.clone();
    // Write one object in the fork.
    forked.set(
        Pointer {
            base: objs[500],
            off: 0,
        },
        0xFF,
        8,
        Span::DUMMY,
    );

    let shared = objs
        .iter()
        .filter(|o| m.shares_storage_with(&forked, **o))
        .count();
    assert!(
        shared >= 999,
        "999 of 1000 objects are shared after writing one, got {shared}"
    );
    assert!(
        !m.shares_storage_with(&forked, objs[500]),
        "and the written one is not"
    );
    // The original is unchanged, which is what makes the sharing safe rather than aliasing.
    assert_eq!(
        m.read(
            Pointer {
                base: objs[500],
                off: 0
            },
            1,
            Span::DUMMY
        )
        .value,
        Some(vec![(500u32 % 256) as u8])
    );
}

/// **A refused write must not cost a copy.** `Arc::make_mut` ran *before* the bounds,
/// read-only and symbolic checks, so an operation that changes nothing cloned the whole
/// object — and the shipped test only ever asserted sharing after a *successful* write, so
/// nothing saw it.
///
/// This lands on the path the bit-API fix created: every `StoreBits` into a symbolic byte
/// is now refused, and each refusal cloned. `h->ver = 4` inside a loop over packet headers
/// is the shape 020 contract 25's own commit message named, so the cost was quadratic in
/// the program's memory again — on the operation contract 20 exists to make cheap, undone
/// by the commit that reported it. Found by review.
#[test]
fn a_refused_write_keeps_the_storage_shared() {
    let mut a = chiero_solver::TermArena::new();
    let mut m = Memory::new();
    let objs: Vec<_> = (0..64)
        .map(|_| {
            let o = m.alloc(ObjKind::Heap, 8, 8, Span::DUMMY);
            let x = a.var(chiero_solver::Sort::BitVec(8), "hdr");
            m.write_sym_byte(Pointer { base: o, off: 0 }, x, Span::DUMMY);
            o
        })
        .collect();
    let mut forked = m.clone();
    for o in &objs {
        // Refused: the byte is symbolic, so the bit write cannot be represented.
        let r = forked.write_bits(Pointer { base: *o, off: 0 }, 0, 4, 0b0100, Span::DUMMY);
        assert!(!r.faults.is_empty(), "the premise: this write is refused");
    }
    let shared = objs
        .iter()
        .filter(|o| m.shares_storage_with(&forked, **o))
        .count();
    assert_eq!(
        shared,
        objs.len(),
        "the fork changed nothing, so it still shares everything"
    );
}

/// **A read keeps the storage shared**, except where it genuinely changes state.
/// Memoizing an uninitialized read is a real mutation — 021 contract 26 requires the
/// second read to agree with the first — so that one is expected to break sharing. A read
/// of *written* bytes must not.
#[test]
fn a_plain_read_does_not_break_sharing() {
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 8, 8, Span::DUMMY);
    m.set(Pointer { base: o, off: 0 }, 7, 8, Span::DUMMY);
    let mut forked = m.clone();
    let r = forked.read(Pointer { base: o, off: 0 }, 8, Span::DUMMY);
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    assert!(
        m.shares_storage_with(&forked, o),
        "reading written bytes changes nothing"
    );
}

/// **"Nothing to share" is shared.** An object past `MAX_MATERIALIZED_BYTES` has no
/// storage in either memory, so a fork cannot have copied it — answering "not shared"
/// conflated the two and would report a phantom copy in any accounting built on this.
/// Neither answer was pinned, so this decides it rather than leaving it to default. Found
/// by review.
#[test]
fn an_object_with_no_storage_counts_as_shared_after_a_fork() {
    let mut m = Memory::new();
    let huge = m.alloc(ObjKind::Heap, MAX_MATERIALIZED_BYTES + 1, 16, Span::DUMMY);
    let ordinary = m.alloc(ObjKind::Heap, 8, 8, Span::DUMMY);
    let forked = m.clone();
    assert!(
        m.shares_storage_with(&forked, huge),
        "there was nothing to copy"
    );
    assert!(m.shares_storage_with(&forked, ordinary));
    // An object one memory does not have at all is still not shared.
    let mut other = Memory::new();
    let elsewhere = other.alloc(ObjKind::Heap, 8, 8, Span::DUMMY);
    assert!(!m.shares_storage_with(&other, elsewhere));
}

/// **A refused bit write leaves the object exactly as it was.** `check_bit_write`
/// validates the whole range before `write_bits` touches anything, so a range that is
/// partly writable does not land its writable half — which would upgrade those init bits
/// to `Yes` and turn a genuine uninitialized-read defect into a clean read. Correct, and
/// unpinned until now. Found by review.
#[test]
fn a_refused_bit_write_changes_nothing() {
    let mut a = chiero_solver::TermArena::new();
    let mut m = Memory::new();
    let o = m.alloc(ObjKind::Heap, 4, 4, Span::DUMMY);
    let x = a.var(chiero_solver::Sort::BitVec(8), "x");
    m.write_sym_byte(Pointer { base: o, off: 1 }, x, Span::DUMMY);

    // Bits 0..12 span byte 0 (concrete, uninitialized) and byte 1 (symbolic).
    let w = m.write_bits(Pointer { base: o, off: 0 }, 0, 12, 0xFFF, Span::DUMMY);
    assert!(!w.faults.is_empty(), "the premise: refused");

    // Byte 0 must still be uninitialized. If the writable half had landed, its bits would
    // read as `Yes` and a real defect would have become a clean read.
    let r = m.read(Pointer { base: o, off: 0 }, 1, Span::DUMMY);
    assert!(
        r.faults.iter().any(|f| f.kind() == "uninitialized-read"),
        "nothing was written: {:#?}",
        r.faults
    );
}

/// **`Memory::entry` binary-searches `entries`, so the ids must stay sorted.**
///
/// The lookup runs on every memory access and used to be a linear scan; with one object per
/// local, reading a local cost O(objects). It is a binary search since 2026-08-09, which is
/// correct only while `alloc` hands out increasing ids and nothing removes an entry. That is
/// true by construction today — `entries.push` in `alloc` is the only writer — and this pins
/// it from outside, because a future allocator that reused or reordered ids would not fail
/// loudly: lookups would just start missing objects that are present.
#[test]
fn object_ids_are_allocated_in_increasing_order() {
    let mut m = chiero_mem::Memory::new();
    let mut ids = Vec::new();
    for i in 0..64u64 {
        ids.push(m.alloc(
            chiero_mem::ObjKind::Stack,
            8 + i,
            8,
            chiero_span::Span::DUMMY,
        ));
    }
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "ids must increase for the binary search to be valid: {ids:?}"
    );

    // And every object is still findable — the property the ordering exists to serve.
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(
            m.size_of_pub(*id),
            Some(8 + i as u64),
            "object {i} ({id:?}) was not found by lookup"
        );
    }
}
