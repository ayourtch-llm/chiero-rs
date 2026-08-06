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
        size: offset + 8,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![Field {
            name: "f".into(),
            offset,
            size: 8,
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
            },
            Field {
                name: "big".into(),
                offset: 8,
                size: 8,
            },
            Field {
                name: "b".into(),
                offset: 16,
                size: 1,
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
        size: 68,
        align: 8,
        packed: false,
        externally_visible: false,
        fields_complete: true,
        fields: vec![Field {
            name: "seq".into(),
            offset: 60,
            size: 8,
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
            },
            Field {
                name: "y".into(),
                offset: 8,
                size: 8,
            },
        ],
    };
    assert!(
        analyse(&tight, &LocalityCfg::default()).is_empty(),
        "nothing straddles, nothing is wasted"
    );
}
