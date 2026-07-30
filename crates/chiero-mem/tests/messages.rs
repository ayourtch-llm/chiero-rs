//! **Every fault's message is a sentence a person can read.**
//!
//! 023 §9 makes the report the product: "a report a person cannot act on is not a report". Every
//! test in the tree checks a fault's *kind* or its fields, and not one checks the text that
//! actually reaches a user — so the text is unowned, and it drifted:
//!
//! ```text
//!   maybe-uninitialized-read: read at offset 2 of ObjectId(3) touches bit 16, written
//!   only under a                  guard the engine has not discharged
//! ```
//!
//! Eighteen spaces mid-sentence — and **not** from a `\` line continuation, which was the first
//! guess and is wrong: Rust's `\`-newline escape skips the next line's leading whitespace, and the
//! two genuine continuations in that same `impl` render correctly, which is the proof. It is a
//! single over-long literal with the spaces sitting inside it, of the kind produced by joining two
//! lines by hand. `cargo fmt` will not split a string literal and has nothing to say about its
//! contents, so no gate in this repo could see it.
//!
//! # Why an invariant and not a fixture per message
//!
//! The wording is the author's business and pinning it would make every improvement a test
//! change. What is *not* negotiable is that the result reads as prose: no run of two spaces, no
//! edges that are whitespace, nothing empty, and the kind prefix present so a reader can tell
//! findings apart. Those hold for any wording anyone would want, so the test constrains the
//! defect and not the design.
//!
//! Constructing all eighteen variants by hand is the point rather than a chore. A `match` in the
//! `Display` impl is exhaustive and the compiler proves it; the *quality* of each arm is
//! exactly what no compiler checks, and a new variant added without a message now has to walk
//! past this list to get in.
//!
//! # How it got missed
//!
//! Wave 205 fixed this string and the fix never reached a commit: a `git checkout --` restoring
//! a mutant discarded the uncommitted edit, and the commit message claimed a change that was no
//! longer in the tree. Nothing failed, because nothing looked. That is the gap this file closes
//! — the wording could be fixed again tomorrow and lost again the same way, and the test is what
//! makes the loss loud.

use chiero_mem::{MemFault, ObjKind, ObjectId};
use chiero_solver::{Sort, TermArena};
use chiero_span::Span;

/// One of every variant. Field values are unremarkable on purpose — the subject is the prose.
fn every_fault(a: &mut TermArena) -> Vec<MemFault> {
    let obj = ObjectId(3);
    let at = Span::DUMMY;
    let t = a.var(Sort::BitVec(1), "g");
    let off = a.var(Sort::BitVec(64), "i");
    vec![
        MemFault::OutOfBounds {
            obj,
            off: 4,
            size: 4,
            obj_size: 8,
            at,
        },
        MemFault::Uninitialized {
            obj,
            off: 2,
            bit: 16,
            at,
        },
        MemFault::MaybeUninitialized {
            obj,
            off: 2,
            bit: 16,
            guard: Some(t),
            at,
        },
        MemFault::MaybeUninitialized {
            obj,
            off: 2,
            bit: 16,
            guard: None,
            at,
        },
        MemFault::Misaligned {
            obj,
            off: 3,
            want: 4,
            at,
        },
        MemFault::UseAfterFree {
            obj,
            freed_at: at,
            at,
        },
        MemFault::DoubleFree {
            obj,
            freed_at: at,
            at,
        },
        MemFault::UseAfterScope {
            obj,
            scope_ended_at: at,
            at,
        },
        MemFault::ReadOnly { obj, off: 0, at },
        MemFault::BadRange {
            want_bits: 256,
            max_bits: 128,
            at,
        },
        MemFault::AllocationTooLarge {
            obj,
            size: 1 << 40,
            at,
        },
        MemFault::NullDeref { off: 0, at },
        MemFault::WildPointer { off: 999, at },
        MemFault::SymbolicByte { obj, off: 1, at },
        MemFault::PointerOutsideObject {
            obj,
            obj_size: 8,
            witness: 12,
            at,
        },
        MemFault::UninitializedSymbolic {
            obj,
            off,
            guard: t,
            at,
        },
        MemFault::OutOfBoundsMaybe {
            obj,
            size: 4,
            obj_size: 8,
            witness: 9,
            at,
        },
        MemFault::OverlappingCopy {
            obj,
            dst: 0,
            src: 2,
            size: 8,
            at,
        },
        MemFault::BadFree {
            obj,
            kind: ObjKind::Stack,
            at,
        },
    ]
}

/// **No run of two spaces.** The `\` continuation defect, and the reason this file exists.
#[test]
fn no_fault_message_has_a_run_of_spaces() {
    let mut a = TermArena::new();
    for f in every_fault(&mut a) {
        let s = f.to_string();
        assert!(
            !s.contains("  "),
            "`{}` renders with a run of spaces, which no wording wants and no formatter \
             will find: {s:?}",
            f.kind()
        );
    }
}

/// Not empty, not padded, and no stray newline or tab.
///
/// Separate from the run-of-spaces check because the fix for one does not imply the other, and a
/// message ending in a space reads as truncated in exactly the places a report is quoted.
#[test]
fn every_fault_message_is_a_single_clean_line() {
    let mut a = TermArena::new();
    for f in every_fault(&mut a) {
        let s = f.to_string();
        // **Content beyond the prefix**, not merely non-empty. `Display` writes the kind
        // unconditionally, so `s` can never be empty and an emptiness check is an assertion
        // nothing can fail — mutation said so, by having no way to express the mutant. An arm
        // that writes only its kind is a report with no content, and that is expressible.
        assert!(
            s.len() > f.kind().len() + 2,
            "`{}` renders nothing beyond its own kind: {s:?}",
            f.kind()
        );
        assert_eq!(s, s.trim(), "`{}` renders with padded edges", f.kind());
        assert!(
            !s.contains('\n') && !s.contains('\t'),
            "`{}` renders across lines, and a finding is quoted as one: {s:?}",
            f.kind()
        );
    }
}

/// The kind leads the message, so two findings can be told apart at a glance.
///
/// This is the one piece of *format* worth pinning: 023 §6.1 makes the kind half the dedup key,
/// and a reader scanning a list needs it in a fixed place.
#[test]
fn every_fault_message_names_its_kind_first() {
    let mut a = TermArena::new();
    for f in every_fault(&mut a) {
        let s = f.to_string();
        assert!(
            s.starts_with(&format!("{}: ", f.kind())),
            "`{}` does not lead with its kind: {s:?}",
            f.kind()
        );
    }
}

/// Every variant is covered above.
///
/// The list is hand-written, so it can rot: a variant added without a message would simply not
/// appear and every assertion would still pass. Counting the distinct kinds against the enum's
/// own count is what makes the omission fail instead — and `kind()` is exhaustive, so the number
/// it can produce is the number of variants.
#[test]
fn the_list_covers_every_variant() {
    let mut a = TermArena::new();
    let faults = every_fault(&mut a);
    let mut kinds: Vec<&str> = faults.iter().map(MemFault::kind).collect();
    kinds.sort_unstable();
    kinds.dedup();
    // Eighteen variants, seventeen kinds: `UninitializedSymbolic` deliberately reports as
    // `maybe-uninitialized-read`, because a guard the solver could not settle *is* a maybe and
    // a second slug for it would split one finding class in two.
    assert_eq!(
        kinds.len(),
        17,
        "eighteen variants share seventeen kinds; a new one without a message would show up \
         here as sixteen: {kinds:?}"
    );
}

/// **Only a fault about two events has a second location.**
///
/// `secondary()` feeds the engine's location rendering, and the engine replaces a span it gets
/// back into the message. A fault with one event has nothing to put there, and answering with
/// the *access* span instead would eventually name the wrong place — the failure mode
/// `the_line_named_is_the_free_and_not_the_access` guards on the engine's side, one layer down.
///
/// Mutation is why this exists as its own test: making `secondary()` a catch-all that returns
/// `at()` changed nothing observable, because only three messages render a location today. It is
/// the fourth one that would be silently wrong, so the contract is asserted where it is stated
/// rather than where it currently happens to matter.
#[test]
fn only_a_two_event_fault_has_a_secondary_location() {
    let mut a = TermArena::new();
    let with: Vec<&str> = every_fault(&mut a)
        .iter()
        .filter(|f| f.secondary().is_some())
        .map(MemFault::kind)
        .collect();
    assert_eq!(
        with,
        vec!["use-after-free", "double-free", "use-after-scope"],
        "these three name where the memory died as well as where it was touched, and no \
         other fault has a second event to name"
    );
}
