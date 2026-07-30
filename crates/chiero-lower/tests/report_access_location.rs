//! **A finding names every location except its own.**
//!
//! Wave 208 taught the engine to render a fault's *second* location. The result is a report that
//! tells a reader where the memory was freed and not where the program touched it:
//!
//! ```text
//!   use-after-free: the 4-byte heap allocation was freed earlier on this path (t.c:5:1),
//!                   before this access
//! ```
//!
//! Line 5 is the `free`. The access is on line 6 and appears nowhere. For a fault with only one
//! event — an uninitialized read, an out-of-bounds access — the text names no location at all.
//!
//! # Why "it is in `Finding::span`" is not an answer
//!
//! It is, and `reports()` hands it over structurally for a caller that wants to render it. But
//! `findings()` is the projection this repo actually reads: thirty-seven assertions across the
//! test suite match on those strings, and 001 §1 puts an LLM at the other end of them. A
//! consumer that has to join two fields to learn where a bug is will either do it or not bother,
//! and the one location the text *does* carry is the wrong one to read alone.
//!
//! # Scope, stated up front
//!
//! This is about findings that name a defect in the program: memory faults and model reports,
//! which between them cover both routes a `Finding` can arrive by, and checker reports along with
//! the second. **Refusals and degradation notices are deliberately not included** — those
//! describe a limit of the run rather than a place in the program, and 023 §7 makes them a
//! statement about chiero. Section 9 carries them separately if they ever want one.

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

fn with_map(src: &str) -> Vec<String> {
    let (m, map) = harness::lower_maybe_with_map(src).expect("the fixture lowers");
    let mut arena = TermArena::new();
    Engine::new(&m)
        .with_source_map(&map)
        .with_entry("probe")
        .run(&mut arena)
        .findings()
}

fn find<'a>(f: &'a [String], kind: &str) -> &'a String {
    f.iter()
        .find(|m| m.starts_with(kind))
        .unwrap_or_else(|| panic!("no `{kind}` among {f:?}"))
}

/// **A two-event fault names both places, and they are different places.**
#[test]
fn a_use_after_free_names_the_access_as_well_as_the_free() {
    let f = with_map(UAF);
    let uaf = find(&f, "use-after-free");
    assert!(
        uaf.contains("t.c:5:"),
        "the free is still named — wave 208's behaviour must survive: {uaf:?}"
    );
    assert!(
        uaf.contains("t.c:6:"),
        "and the access on line 6 is where the reader has to look first: {uaf:?}"
    );
}

/// A fault with **one** event names it.
///
/// The case that shows this is not merely a missing clause in one message: an uninitialized read
/// has no second location, so before this wave its text carried no location whatsoever.
#[test]
fn a_single_event_fault_names_where_it_happened() {
    let src = "int probe(void){\n\
               char ca[8];\n\
               return ca[0];\n\
               }\n";
    let f = with_map(src);
    let u = find(&f, "uninitialized-read");
    assert!(
        u.contains("t.c:3:"),
        "the read is on line 3 and nothing in this report said so: {u:?}"
    );
}

/// **The model route as well**, which is a different path to a `Finding`.
///
/// Wave 207 found these two routes the hard way: a fix to `report_faults` left
/// `ModelRegistry::lift` printing `ObjectId(3)`. `use-after-free` above arrives by the first
/// route and `double-free` by the second, so the pair covers both — and checker reports share
/// the second one, so they come along with it.
///
/// (Asserting a *checker* fixture directly would be better and is not possible here: the
/// arithmetic checkers are not registered by an `Engine` built this way, and wiring them into a
/// `chiero-lower` test is plumbing this wave does not add. Section 9 carries it.)
#[test]
fn the_model_route_names_the_access_too() {
    let src = "void *malloc(unsigned long);\n\
               void free(void *);\n\
               int probe(void){\n\
               int *p = malloc(4);\n\
               free(p);\n\
               free(p);\n\
               return 0;\n\
               }\n";
    let f = with_map(src);
    let df = find(&f, "double-free");
    assert!(
        df.contains("t.c:5:"),
        "the first free is still named: {df:?}"
    );
    assert!(
        df.contains("t.c:6:"),
        "and the second free — the fault itself — is on line 6: {df:?}"
    );
}

/// **The kind still leads.** The control that matters most.
///
/// Thirty-seven assertions in this repo match findings with `starts_with`, and 023 §6.1 makes the
/// kind half the dedup key. A location prefixed in the compiler's usual position — `t.c:6:8:
/// use-after-free: …` — would be the conventional rendering and would break every one of them.
#[test]
fn the_kind_still_leads_the_message() {
    for src in [UAF, "int probe(void){\nchar ca[8];\nreturn ca[0];\n}\n"] {
        for m in with_map(src) {
            let head = m.split(':').next().unwrap_or_default();
            assert!(
                !head.is_empty() && head.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "every message begins with its kind, not with a path: {m:?}"
            );
        }
    }
}

/// Without a map there is no location to add. **The control.**
#[test]
fn without_a_source_map_nothing_is_stamped() {
    let m = harness::lower(UAF);
    let mut arena = TermArena::new();
    let f = Engine::new(&m)
        .with_entry("probe")
        .run(&mut arena)
        .findings();
    assert!(
        f.iter().all(|m| !m.contains("t.c:")),
        "a run with no map must not invent a location: {f:?}"
    );
}
