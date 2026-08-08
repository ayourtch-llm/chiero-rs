//! Covers: 023 §9 — *"a witness is a concrete input someone can re-run"*, and what a report may
//! print of one that is too long to be.
//!
//! Measured on VPP: `find-bugs --entry nsh_md2_encap` produced **10 658 bindings, 10 657 of them
//! the same anonymous "a lazily-materialized byte"**, in 950 KB of JSON for one finding. Under
//! UCSE an entry that walks a packet buffer materialises a byte at a time, so the execution is
//! working; the rendering has stopped answering the question.

use chiero_exec::{Binding, InputOrigin, Witness};
use chiero_span::Span;

fn free_byte() -> Binding {
    Binding {
        origin: InputOrigin::Memory {
            span: Span::DUMMY,
            why: "a lazily-materialized byte",
        },
        width: 8,
        value: 0,
        pinned: false,
    }
}

fn pinned_param(index: usize) -> Binding {
    Binding {
        origin: InputOrigin::Param {
            index,
            name: format!("p{index}"),
            span: Span::DUMMY,
        },
        width: 32,
        value: index as u128,
        pinned: true,
    }
}

/// **Pinned first, and that is not a preference.**
///
/// On the fixture that reproduces the VPP shape — *n* loads then a division — the pinned bindings
/// are the *last* four, the divisor's bytes. A bound that took the first *k* in path order would
/// drop every value the finding depends on and keep *k* that it does not, then print the result
/// as the input that reproduces it.
#[test]
fn the_bindings_the_path_pinned_come_first() {
    let mut bindings: Vec<Binding> = (0..100).map(|_| free_byte()).collect();
    bindings.push(pinned_param(0));
    bindings.push(pinned_param(1));
    let w = Witness { bindings };

    let d = w.digest(8);
    assert_eq!(d.shown.len(), 8);
    assert_eq!(
        d.shown.iter().filter(|b| b.pinned).count(),
        2,
        "both pinned bindings survive a bound of 8 over 102 bindings"
    );
    assert_eq!(d.omitted, 94);
    assert_eq!(
        d.omitted_by_label,
        vec![("a lazily-materialized byte".to_string(), 94)],
        "and a reader is told what the omitted inputs were, which is what makes the omission \
         actionable: 94 materialized bytes says the finding does not turn on them"
    );
}

/// Order is preserved within each group, so the pinned inputs read in the order the path met them.
#[test]
fn order_survives_within_each_group() {
    let w = Witness {
        bindings: vec![
            pinned_param(0),
            free_byte(),
            pinned_param(1),
            free_byte(),
            pinned_param(2),
        ],
    };
    let d = w.digest(4);
    let indices: Vec<usize> = d
        .shown
        .iter()
        .filter_map(|b| match &b.origin {
            InputOrigin::Param { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(d.omitted, 1);
}

/// A witness that fits is returned whole, and reports no omission.
#[test]
fn a_witness_within_the_bound_is_untouched() {
    let w = Witness {
        bindings: vec![pinned_param(0), free_byte(), pinned_param(1)],
    };
    let d = w.digest(64);
    assert_eq!(d.shown.len(), 3);
    assert_eq!(d.omitted, 0);
    assert!(d.omitted_by_label.is_empty());
    // Unchanged order too: nothing is reordered when nothing is dropped.
    assert!(!d.shown[1].pinned, "the free byte is still in the middle");
}

/// ⚠️ **A digest is not a basis for a claim about the whole witness, and this is the trap.**
///
/// `check_reachable` licenses `proven` partly on "a solver pinned every input". Because the
/// digest puts pinned bindings first, a bounded view of a witness can be *entirely pinned* while
/// the witness is not — so computing that check from the rendering would turn an unproven arrival
/// into a proof, silently, and only on witnesses long enough to be truncated.
///
/// `chiero-tool` computes it over `Witness::bindings` for exactly this reason. This test is what
/// makes the reason checkable rather than a comment: it fails the moment `digest` stops ordering
/// pinned-first, which is the property the tool's guard is written against.
#[test]
fn a_bounded_view_can_be_all_pinned_while_the_witness_is_not() {
    let mut bindings: Vec<Binding> = (0..65).map(pinned_param).collect();
    bindings.push(free_byte());
    let w = Witness { bindings };

    assert!(
        w.digest(64).shown.iter().all(|b| b.pinned),
        "the bounded view sees only pinned bindings"
    );
    assert!(
        !w.bindings.iter().all(|b| b.pinned),
        "while the witness has an input the model left free"
    );
}
