//! **A finding's second location exists only as prose.**
//!
//! Waves 208 and 209 got both places into the sentence:
//!
//! ```text
//!   use-after-free: the 4-byte heap allocation was freed earlier on this path (t.c:5:1),
//!                   before this access (at t.c:6:1)
//! ```
//!
//! `Finding::span` carries the access as data. The `free` is a substring. A consumer offering
//! "jump to where it was freed" — an IDE, or the LLM 001 §1 puts at the other end of these — has
//! to parse `(t.c:5:1)` back out of English, and the two renderings it must tell apart differ by
//! the word "at". That is a fine thing for a *reader* to rely on and no way to build anything.
//!
//! # Why the label is part of it
//!
//! A span alone does not say what is at it. Shown two file positions, a reader has to know which
//! is the bug and which is the cause — so `SecondEvent` carries a phrase beside the location. The
//! alternative is every consumer re-deriving "freed" against "scope ended" from the finding's
//! kind, which is a mapping worth stating once.
//!
//! # What must stay true
//!
//! The prose does not go away. It is what a person reads, waves 208–209 made it correct, and this
//! is the same information offered a second way — so the tests here assert both, and a fix that
//! moved the location out of the sentence would fail them.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

/// Line 5 frees, line 6 dereferences.
const UAF: &str = "void *malloc(unsigned long);\n\
                   void free(void *);\n\
                   int probe(void){\n\
                   int *p = malloc(4);\n\
                   free(p);\n\
                   return *p;\n\
                   }\n";

/// Every finding, plus the map needed to resolve what they point at.
fn run(src: &str) -> (Vec<chiero_exec::Finding>, chiero_span::SourceMap) {
    let (m, map) = harness::lower_maybe_with_map(src).expect("the fixture lowers");
    let mut arena = TermArena::new();
    let r = Engine::new(&m)
        .with_source_map(&map)
        .with_entry("probe")
        .run(&mut arena);
    (r.reports(), map)
}

fn of_kind<'a>(f: &'a [chiero_exec::Finding], kind: &str) -> &'a chiero_exec::Finding {
    f.iter()
        .find(|x| x.message.starts_with(kind))
        .unwrap_or_else(|| panic!("no `{kind}` among {f:#?}"))
}

/// Where a `Span` is, as a line number.
fn line(map: &chiero_span::SourceMap, s: chiero_span::Span) -> u32 {
    map.lookup_loc(s.lo).expect("the fixture's own map").line
}

/// **The free is reachable as data, and it is the free's line.**
#[test]
fn a_use_after_free_carries_the_free_as_a_span() {
    let (f, map) = run(UAF);
    let uaf = of_kind(&f, "use-after-free");
    let rel = uaf
        .related
        .expect("a use-after-free is about two places, and one of them is the free");
    assert_eq!(line(&map, rel.at), 5, "the `free` is on line 5");
    assert_eq!(
        line(&map, uaf.span),
        6,
        "and the finding itself is at the access, which must not have moved"
    );
}

/// The label says which event it is.
///
/// Separate from the span, because a fix that carried the position and left a consumer to guess
/// what is there would satisfy the test above and still need the kind decoded by hand.
#[test]
fn the_second_event_says_what_happened_there() {
    let (f, _) = run(UAF);
    let rel = of_kind(&f, "use-after-free").related.expect("present");
    assert!(
        rel.what.contains("free"),
        "a reader shown this position has to be told it is the free: {:?}",
        rel.what
    );
}

/// A use-after-scope names its scope's end, not a free.
///
/// The second of the three two-event faults, reached through an entirely different mechanism —
/// 020 §4.4's `scope` on the alloca rather than a `free` model — so a fix keyed on the heap would
/// leave it empty.
#[test]
fn a_use_after_scope_carries_the_scope_end() {
    let src = "int probe(void){\n\
               int *p;\n\
               {\n\
               int x = 7;\n\
               p = &x;\n\
               }\n\
               return *p;\n\
               }\n";
    let (f, map) = run(src);
    let uas = of_kind(&f, "use-after-scope");
    let rel = uas.related.expect("the scope's end is the second place");
    assert_eq!(line(&map, rel.at), 6, "the block closes on line 6");
    assert!(
        rel.what.contains("scope"),
        "and it is a scope ending, not a free: {:?}",
        rel.what
    );
}

/// A single-event fault has **no** second place. **The control.**
///
/// The one that stops the cheapest wrong fix. Filling `related` with the access span whenever
/// there is nothing else would satisfy every assertion above except this, and would tell a
/// consumer that every uninitialized read has a cause somewhere it does not.
#[test]
fn a_single_event_fault_carries_no_second_place() {
    let src = "int probe(void){\n\
               char ca[8];\n\
               return ca[0];\n\
               }\n";
    let (f, _) = run(src);
    let u = of_kind(&f, "uninitialized-read");
    assert!(
        u.related.is_none(),
        "nothing made this read uninitialized except never being written: {:?}",
        u.related
    );
}

/// The prose still says both. **The control for waves 208–209.**
#[test]
fn the_message_still_names_both_places() {
    let (f, _) = run(UAF);
    let m = &of_kind(&f, "use-after-free").message;
    assert!(
        m.contains("(t.c:5:") && m.contains("(at t.c:6:"),
        "offering the data must not remove what a person reads: {m:?}"
    );
}
