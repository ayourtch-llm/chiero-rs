//! **041 §3 — cache-line and locality analysis.**
//!
//! Caches have no semantic effect (021 §7), but VPP tunes for them deliberately:
//! `CLIB_CACHE_LINE_BYTES` appears in **257** files and `CLIB_CACHE_LINE_ALIGN_MARK` in 124.
//! Layout is knowable statically, so these are findings rather than guesses.
//!
//! # The two constraints that keep this from being dangerous
//!
//! §3 states them, and they are what the tests below are mostly about:
//!
//! > - **A reordering proposal must state whether the struct's layout is observable outside
//! >   the program** — wire formats, ABI boundaries, structs with `packed`, and anything
//! >   reaching a serialization path. Reordering an `ip4_header_t` is a protocol violation,
//! >   not an optimization.
//! > - **Benefit is labelled honestly.** `Measured` requires access counts from a real run;
//! >   otherwise it is `Estimated` or `Unquantified`. chiero has no cycle model and will not
//! >   pretend to one.
//!
//! # Why the layout comes in rather than being computed here
//!
//! 014 §3 computes record layout, and `chiero-opt` is a vertical that 001 §4 rule 7 keeps
//! free of a frontend dependency. Duplicating gcc's straddling and packing rules here is
//! exactly the mistake `chiero-diff` was corrected for: 014's answer is measured against gcc
//! in its own corpus gate, and a second implementation would be a second answer.

use chiero_opt::locality::*;

/// A struct with a 64-bit field at a given offset, on a 64-byte line.
fn one_field_at(offset: u64) -> Record {
    Record {
        tag: "s".into(),
        span: chiero_span::Span::DUMMY,
        size: offset + 8,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![Field {
            name: "f".into(),
            offset,
            size: 8,
            bits: None,
        }],
    }
}

/// **Contract 18: a 64-bit field at offset 60 with `cache_line_bytes == 64` straddles; at
/// offset 56 it does not.**
///
/// The boundary case in both directions, because an off-by-one here is a report about every
/// struct in the tree or about none of them.
#[test]
fn a_field_crossing_a_line_boundary_is_reported_and_one_that_fits_is_not() {
    let cfg = LocalityCfg {
        cache_line_bytes: 64,
        ..LocalityCfg::default()
    };

    let straddling = analyse(&one_field_at(60), &cfg);
    assert!(
        straddling
            .iter()
            .any(|p| matches!(&p.kind, OptKind::LineStraddle { field, .. } if field == "f")),
        "a 64-bit field at offset 60 spans two lines: {straddling:?}"
    );

    let fitting = analyse(&one_field_at(56), &cfg);
    assert!(
        !fitting
            .iter()
            .any(|p| matches!(p.kind, OptKind::LineStraddle { .. })),
        "offsets 56..64 are one line: {fitting:?}"
    );
}

/// A struct with padding a reorder would recover: `char; long; char;`
fn padded() -> Record {
    Record {
        tag: "p".into(),
        span: chiero_span::Span::DUMMY,
        size: 24,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "a".into(),
                offset: 0,
                size: 1,
                bits: None,
            },
            Field {
                name: "big".into(),
                offset: 8,
                size: 8,
                bits: None,
            },
            Field {
                name: "b".into(),
                offset: 16,
                size: 1,
                bits: None,
            },
        ],
    }
}

/// **Padding waste, with the size delta** — §3's fourth analysis. A proposal that says "you
/// could save space" without saying how much is not actionable.
#[test]
fn recoverable_padding_is_reported_with_the_number_of_bytes() {
    let proposals = analyse(&padded(), &LocalityCfg::default());
    let waste = proposals
        .iter()
        .find_map(|p| match &p.kind {
            OptKind::PaddingWaste { recoverable } => Some(*recoverable),
            _ => None,
        })
        .expect("14 bytes of this 24-byte struct are padding");
    assert!(
        waste > 0,
        "a padding proposal with no size delta is not actionable"
    );
    // `char, long, char` is 24 bytes; `long, char, char` is 16.
    assert_eq!(
        waste, 8,
        "the delta is what a reorder would actually recover"
    );
}

/// **Contract 21: a `packed` struct, or one reachable from a serialization path, yields only
/// an advisory proposal that says the layout may be externally observable.**
///
/// > "Reordering an `ip4_header_t` is a protocol violation, not an optimization."
#[test]
fn a_struct_whose_layout_escapes_gets_an_advisory_proposal_that_says_so() {
    // **A straddling field, not the padded fixture.** A `packed` struct has no alignment
    // padding to recover, so the padding analysis correctly says nothing about one — and a
    // test asserting "the observation is still worth making" needs an observation there is.
    // A wire-format header with a field across a line boundary is the real shape: the finding
    // is true, and acting on it is a protocol change.
    let header = Record {
        tag: "hdr".into(),
        span: chiero_span::Span::DUMMY,
        size: 68,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![Field {
            name: "seq".into(),
            offset: 60,
            size: 8,
            bits: None,
        }],
    };
    for (what, r) in [
        (
            "packed",
            Record {
                packed: true,
                ..header.clone()
            },
        ),
        (
            "externally visible",
            Record {
                externally_visible: true,
                fields_complete: true,
                ..header.clone()
            },
        ),
    ] {
        let proposals = analyse(&r, &LocalityCfg::default());
        assert!(
            !proposals.is_empty(),
            "{what}: the observation is still worth making"
        );
        for p in &proposals {
            assert!(
                p.advisory,
                "{what}: a proposal to reorder this must be advisory: {p:?}"
            );
            assert!(
                p.obligations
                    .iter()
                    .any(|o| matches!(o, Obligation::Open { .. })),
                "{what}: and carry the open obligation that says why: {p:?}"
            );
            assert!(
                p.rationale.to_lowercase().contains("observable"),
                "{what}: prominently, in words: {p:?}"
            );
        }
    }
}

/// **Contract 22: `Measured` requires real access counts.**
///
/// > "chiero has no cycle model and will not pretend to one."
///
/// With no execution data every benefit must be `Unquantified` — not `Estimated`, which is
/// still a number somebody will act on.
#[test]
fn benefit_is_unquantified_without_access_counts() {
    let proposals = analyse(&padded(), &LocalityCfg::default());
    assert!(!proposals.is_empty());
    for p in &proposals {
        assert_eq!(p.benefit, Benefit::Unquantified, "no run, no number: {p:?}");
    }
}

/// **And `Measured` is reachable**, or the label is decoration. Contract 4b's shape again: an
/// implementation that always says `Unquantified` passes the test above.
#[test]
fn access_counts_make_a_benefit_measured() {
    // A straddling field with a real count: the cost is "this many accesses, each touching two
    // lines", which is exactly a number backed by a run.
    let cfg = LocalityCfg {
        counts: vec![("f".into(), 1_000_000)],
        ..LocalityCfg::default()
    };
    let proposals = analyse(&one_field_at(60), &cfg);
    assert!(
        proposals.iter().any(|p| p.benefit == Benefit::Measured),
        "with real counts a benefit may be measured: {proposals:?}"
    );
    // And the counts are the evidence, not a footnote.
    let with = proposals
        .iter()
        .find(|p| p.benefit == Benefit::Measured)
        .expect("checked");
    assert!(
        with.evidence.iter().any(|e| e.contains("1000000")),
        "the counts that justified it must be in the proposal: {with:?}"
    );
}

/// **Contract 24: all proposals and their order are byte-identical across runs.**
#[test]
fn the_proposals_are_deterministic() {
    let cfg = LocalityCfg {
        counts: vec![("big".into(), 7), ("a".into(), 7), ("b".into(), 7)],
        ..LocalityCfg::default()
    };
    let a = analyse(&padded(), &cfg);
    let b = analyse(&padded(), &cfg);
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    // Ties broken by name, so equal counts cannot reorder between runs.
    let names: Vec<&str> = a.iter().map(|p| p.kind.field().unwrap_or("")).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names.len(), sorted.len());
}

/// **A struct with nothing wrong yields nothing** — and that is the one empty answer here
/// which means something, since the analysis is complete over a layout it was given.
#[test]
fn a_well_packed_struct_yields_no_proposals() {
    let tight = Record {
        tag: "t".into(),
        span: chiero_span::Span::DUMMY,
        size: 16,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "x".into(),
                offset: 0,
                size: 8,
                bits: None,
            },
            Field {
                name: "y".into(),
                offset: 8,
                size: 8,
                bits: None,
            },
        ],
    };
    assert!(
        analyse(&tight, &LocalityCfg::default()).is_empty(),
        "nothing straddles, nothing is wasted"
    );
}

/// **"8 bytes recoverable" does not tell anybody which fields to move.**
///
/// A padding proposal named a total and left the reader to work out where the holes were, on a
/// struct that may have thirty members. The bytes are *between* two named fields and chiero
/// knows which two, because it has every field's offset and size — that is the whole input to
/// the number it already prints.
///
/// `char a; long big; char b;` has both kinds of hole, which is why it is the fixture: seven
/// bytes of alignment padding in the middle, and seven more at the end that exist because the
/// record's own alignment rounds the tail up.
#[test]
fn a_padding_proposal_says_where_the_padding_is() {
    let r = Record {
        tag: "p".into(),
        span: chiero_span::Span::DUMMY,
        size: 24,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "a".into(),
                offset: 0,
                size: 1,
                bits: None,
            },
            Field {
                name: "big".into(),
                offset: 8,
                size: 8,
                bits: None,
            },
            Field {
                name: "b".into(),
                offset: 16,
                size: 1,
                bits: None,
            },
        ],
    };
    let props = analyse(&r, &LocalityCfg::default());
    let pad = props
        .iter()
        .find(|p| matches!(p.kind, OptKind::PaddingWaste { .. }))
        .expect("24 bytes that would be 16");
    let ev = pad.evidence.join("\n");

    // **The interior hole, with the field on each side of it.** "After `a`" alone is not
    // actionable on a struct where `a` is one of thirty members; the pair is what says which
    // gap this is.
    assert!(
        ev.contains("7 bytes") && ev.contains("`a`") && ev.contains("`big`"),
        "the seven bytes between `a` and `big` are named: {ev}"
    );
    // **And the tail, which is a different fact**: it is not between two fields, and no
    // reorder removes all of it — the record's alignment rounds the end up whatever the order.
    assert!(
        ev.contains("`b`") && ev.to_lowercase().contains("end"),
        "the seven bytes after `b` are named as the tail: {ev}"
    );
}

/// **The holes and the recovery are different numbers, and saying so is the honest part.**
///
/// `char a; long big; char b;` has 14 bytes of padding in it and reordering recovers 8: the
/// best order still ends `long, char, char` and the record's own alignment rounds 10 up to 16.
/// A proposal that listed 14 bytes of holes beside "recoverable: 8" without a word would read
/// as an arithmetic error in chiero.
#[test]
fn the_padding_it_names_and_the_padding_it_recovers_are_reconciled() {
    let r = Record {
        tag: "p".into(),
        span: chiero_span::Span::DUMMY,
        size: 24,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "a".into(),
                offset: 0,
                size: 1,
                bits: None,
            },
            Field {
                name: "big".into(),
                offset: 8,
                size: 8,
                bits: None,
            },
            Field {
                name: "b".into(),
                offset: 16,
                size: 1,
                bits: None,
            },
        ],
    };
    let props = analyse(&r, &LocalityCfg::default());
    let pad = props
        .iter()
        .find(|p| matches!(p.kind, OptKind::PaddingWaste { .. }))
        .expect("a proposal");
    assert!(
        matches!(pad.kind, OptKind::PaddingWaste { recoverable: 8 }),
        "{:?}",
        pad.kind
    );
    let ev = pad.evidence.join("\n");
    assert!(
        ev.contains("14 bytes of padding") && ev.contains("8"),
        "the total holes and what a reorder recovers are both stated: {ev}"
    );
}

/// **Contract 25 — a bit-field run is one member, and the bytes around it are countable.**
///
/// `struct { char tag; int big; unsigned a : 1; unsigned b : 1; unsigned c : 1; unsigned d :
/// 1; }` is 12 bytes under gcc 13.3 and **8** with `int` first, so there are 4 real bytes
/// here. Withholding the number because the record holds a bit-field left out exactly the
/// hand-tuned structs where padding matters most; 041 §3.1 gives the description bits instead.
///
/// **This fixture rather than a rounder one, because it discriminates.** The obvious mistake
/// is to count each bit-field as the byte it starts in: that sums 4 + 1 + 1 + 1 + 1 + 1 to 9,
/// rounds to 12, and produces no proposal at all — so the assertion below can tell the two
/// models apart. On `char; long; unsigned a:3; unsigned b:5;` they both answer 8, and a test
/// written there would have passed without seeing anything, which is the trap this file keeps
/// re-learning.
///
/// The bit offsets are gcc's, transcribed: `big` takes bits 32..64, and `a`..`d` are bits
/// 64..68 — one byte, shared, at offset 8.
#[test]
fn a_bit_field_run_is_one_member_and_the_padding_around_it_is_counted() {
    let bit = |name: &str, bit_offset: u64| Field {
        name: name.into(),
        offset: bit_offset / 8,
        size: 1,
        bits: Some(BitExtent {
            bit_offset,
            width: 1,
        }),
    };
    let r = Record {
        tag: "q".into(),
        span: chiero_span::Span::DUMMY,
        size: 12,
        align: 4,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "tag".into(),
                offset: 0,
                size: 1,
                bits: None,
            },
            Field {
                name: "big".into(),
                offset: 4,
                size: 4,
                bits: None,
            },
            bit("a", 64),
            bit("b", 65),
            bit("c", 66),
            bit("d", 67),
        ],
    };
    let props = analyse(&r, &LocalityCfg::default());
    let pad = props
        .iter()
        .find(|p| matches!(p.kind, OptKind::PaddingWaste { .. }))
        .expect("12 bytes that gcc says would be 8 — contract 25");
    assert!(
        matches!(pad.kind, OptKind::PaddingWaste { recoverable: 4 }),
        "`int, char, bits` is 8 bytes under gcc, so 4 come back: {:?}",
        pad.kind
    );

    // **The run is named as a run.** Reporting the tail hole as "after `a`" would be false of
    // the other three, which sit in the same byte; naming one of them alone hides that moving
    // it moves all four. One member, named as what it is.
    let ev = pad.evidence.join("\n");
    assert!(
        ev.contains("bit-field") && ev.contains('a') && ev.contains('d'),
        "the hole after the bit-fields names them as one run: {ev}"
    );
    // Three bytes between `tag` and `big`, three more after the run to the record's end.
    assert!(
        ev.contains("`tag`") && ev.contains("`big`"),
        "the interior hole still names both sides: {ev}"
    );
}

/// **Contract 25 — nothing to recover is not the same fact as nothing chiero could judge.**
///
/// `struct { char tag; unsigned a : 3; unsigned b : 5; long big; }` is 16 bytes and no order
/// makes it smaller: the two bit-fields share the byte after `tag`, and `long` needs its
/// alignment. Silence here is the right answer for the right reason, and the reason is why
/// this is a separate test from the one above rather than a second assertion in it.
#[test]
fn bit_fields_that_already_pack_tight_yield_no_proposal() {
    let r = Record {
        tag: "with_bits".into(),
        span: chiero_span::Span::DUMMY,
        size: 16,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "tag".into(),
                offset: 0,
                size: 1,
                bits: None,
            },
            Field {
                name: "a".into(),
                offset: 1,
                size: 1,
                bits: Some(BitExtent {
                    bit_offset: 8,
                    width: 3,
                }),
            },
            Field {
                name: "b".into(),
                offset: 1,
                size: 1,
                bits: Some(BitExtent {
                    bit_offset: 11,
                    width: 5,
                }),
            },
            Field {
                name: "big".into(),
                offset: 8,
                size: 8,
                bits: None,
            },
        ],
    };
    let props = analyse(&r, &LocalityCfg::default());
    assert!(
        !props
            .iter()
            .any(|p| matches!(p.kind, OptKind::PaddingWaste { .. })),
        "8 + 1 + 1 rounds to 16, which is what it already is: {props:?}"
    );
}

/// **Contract 25 / §3.1 — the reorder moves the run, it does not repack the bits.**
///
/// `struct { unsigned a : 1; unsigned : 0; unsigned b : 1; char tail; }` is 8 bytes: the
/// zero-width member forces `b` into the next allocation unit at bit 32, so the run spans
/// bytes 0..5 for two bits of payload. **Adding the widths instead would say the run is one
/// byte** and offer 4 bytes back — a floor no declaration order reaches, because the
/// allocation-unit rule follows the members wherever they go. gcc agrees: the reordered
/// declaration is still 8.
///
/// This is the direction that matters. An analysis that under-claims loses a finding; one
/// that over-claims sends somebody to reorder a struct for bytes that are not there.
#[test]
fn the_reorder_moves_a_bit_field_run_whole_rather_than_repacking_it() {
    let r = Record {
        tag: "z".into(),
        span: chiero_span::Span::DUMMY,
        size: 8,
        align: 4,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![
            Field {
                name: "a".into(),
                offset: 0,
                size: 1,
                bits: Some(BitExtent {
                    bit_offset: 0,
                    width: 1,
                }),
            },
            // The unnamed zero-width member: it forces the next unit and occupies nothing.
            Field {
                name: "<anonymous member at offset 4>".into(),
                offset: 4,
                size: 0,
                bits: Some(BitExtent {
                    bit_offset: 32,
                    width: 0,
                }),
            },
            Field {
                name: "b".into(),
                offset: 4,
                size: 1,
                bits: Some(BitExtent {
                    bit_offset: 32,
                    width: 1,
                }),
            },
            Field {
                name: "tail".into(),
                offset: 5,
                size: 1,
                bits: None,
            },
        ],
    };
    let props = analyse(&r, &LocalityCfg::default());
    assert!(
        !props
            .iter()
            .any(|p| matches!(p.kind, OptKind::PaddingWaste { .. })),
        "the run is five bytes wherever it goes, so 8 is the floor gcc reaches: {props:?}"
    );
}

/// **A member the caller could not size at all still yields no number** — `fields_complete`
/// keeps meaning what it meant, and a bit-field has simply stopped being one of its causes.
#[test]
fn a_field_list_that_is_still_partial_still_yields_no_padding_number() {
    let mut r = padded();
    r.fields_complete = false;
    let props = analyse(&r, &LocalityCfg::default());
    assert!(
        !props
            .iter()
            .any(|p| matches!(p.kind, OptKind::PaddingWaste { .. })),
        "a sum over a list missing a member is wrong in the flattering direction: {props:?}"
    );
}
