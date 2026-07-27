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
    let mut o = obj(8);
    o.write_bytes_cond(0, &[7], Cond::Symbolic).unwrap();
    assert_eq!(o.init_bit(0), InitBit::Cond, "not Yes, and not No");
    assert_eq!(o.init_bit(8), InitBit::No, "byte 1 is untouched");
}

/// **`Cond` is not `Yes`, and a read must treat it that way.**
///
/// Storing the third state is only half of it: if a read then accepts `Cond` as
/// initialized, the state is decoration and the model behaves exactly like the
/// two-state mask 021 §3.1 rejects. Conditionally-initialized is not definitely
/// initialized, so the read is not silently answered.
#[test]
fn reading_through_a_conditionally_written_byte_is_not_silently_initialized() {
    let mut o = obj(8);
    o.write_bytes_cond(0, &[7], Cond::Symbolic).unwrap();
    assert!(
        matches!(o.read_bytes(0, 1), Err(AccessError::Uninitialized { .. })),
        "a byte initialized only under a guard is not definitely initialized"
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
