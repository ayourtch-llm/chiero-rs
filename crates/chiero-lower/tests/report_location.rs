//! **A report cannot say where anything happened.**
//!
//! Wave 207 got `use-after-free` to name the memory — "the 4-byte heap allocation" instead of
//! `ObjectId(3)` — and left the other half of the sentence unreadable:
//!
//! ```text
//!   use-after-free: the 4-byte heap allocation was freed earlier on this path
//!                   (source offset 85), before this access
//! ```
//!
//! `85` is a `BytePos`: an index into the concatenated preprocessed source space. It is honest
//! about what it is and useless to a reader, who has neither that buffer nor a way to make one.
//!
//! # The capability, not the wording
//!
//! `chiero-span::SourceMap::lookup_loc` turns a `BytePos` into a file, line and column, and
//! **nothing downstream of the front end holds a `SourceMap`.** The harness in this very
//! directory keeps one and its doc comment says why — "the only way to ask whether chiero found
//! the fault *where* the sanitizer did rather than merely finding one of the right kind
//! somewhere in the program" — so the differential oracle resolves locations that the report
//! itself cannot.
//!
//! That is the asymmetry this file is about. A location good enough to grade chiero against ASan
//! is good enough to put in the product, and 023 §9 asks for a report a person can act on.
//!
//! # The rendering is `path:line:col`
//!
//! These tests were first written asserting the words "line 5". The wording was mine and the
//! requirement is that a reader can find the place, so what ships is the form every compiler and
//! editor already understands — `t.c:5:1` — and the assertions match it. Every control keeps its
//! force: the access line is checked for as `t.c:6` rather than "line 6", and the no-map case
//! for the absence of `t.c:` rather than of "line ".
//!
//! # Why the second location and not the first
//!
//! `Finding::span` already carries where the *access* was, structurally, for a caller to render.
//! What has no home is the second place a memory fault names: where the object was freed, or
//! where its scope ended. 024 contract 10 wants the reader told, an existing test in
//! `chiero-exec` asserts they are, and prose is the only carrier it has.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

/// Line 5 frees, line 6 dereferences. Written across lines on purpose — a one-line fixture
/// makes every location the same one and proves nothing about which is reported.
const UAF: &str = "void *malloc(unsigned long);\n\
                   void free(void *);\n\
                   int probe(void){\n\
                   int *p = malloc(4);\n\
                   free(p);\n\
                   return *p;\n\
                   }\n";

/// Run with the `SourceMap` the front end produced.
fn findings_with_map(src: &str) -> Vec<String> {
    let (m, map) = harness::lower_maybe_with_map(src).expect("the fixture lowers");
    let mut arena = TermArena::new();
    let r = Engine::new(&m)
        .with_source_map(&map)
        .with_entry("probe")
        .run(&mut arena);
    r.findings()
}

fn find<'a>(f: &'a [String], kind: &str) -> &'a String {
    f.iter()
        .find(|m| m.starts_with(kind))
        .unwrap_or_else(|| panic!("no `{kind}` among {f:?}"))
}

/// **The line the memory was freed on.**
#[test]
fn a_use_after_free_names_the_line_where_the_memory_was_freed() {
    let f = findings_with_map(UAF);
    let uaf = find(&f, "use-after-free");
    assert!(
        uaf.contains("t.c:5:"),
        "the `free` is on line 5 and the reader has to be told so: {uaf:?}"
    );
    assert!(
        !uaf.contains("source offset"),
        "and a byte offset into the preprocessed buffer is not a location: {uaf:?}"
    );
}

/// The same for a double free, whose second location is the *first* free.
///
/// Separate because the two messages render their span independently, and a fix keyed on one
/// fault kind would leave the other printing offsets.
#[test]
fn a_double_free_names_the_line_of_the_first_free() {
    let src = "void *malloc(unsigned long);\n\
               void free(void *);\n\
               int probe(void){\n\
               int *p = malloc(4);\n\
               free(p);\n\
               free(p);\n\
               return 0;\n\
               }\n";
    let f = findings_with_map(src);
    let df = find(&f, "double-free");
    assert!(
        df.contains("t.c:5:"),
        "the first `free` is on line 5: {df:?}"
    );
}

/// **The line is the one the fault names, not the one the finding is reported at.**
///
/// The sharpest thing this file pins. A fix that rendered `Finding::span` — the *access* — would
/// satisfy "contains a line number" and name line 6 for an event on line 5, which is worse than
/// the offset it replaced: it would be confidently wrong instead of merely useless.
///
/// **Rewritten in wave 209**, which added the access location to the same sentence. This test
/// asserted line 6 was absent, and that was the right assertion in a report carrying exactly one
/// location; it is the wrong one now that the access is named on purpose. The requirement has not
/// moved — the *free* must not be rendered at the access's line — so what it checks is the form:
/// a bare `(t.c:5:1)` is the event the fault names, and the appended `(at t.c:6:1)` is where the
/// finding sits. A confusion between them shows up as `(t.c:6` with no `at`.
#[test]
fn the_line_named_is_the_free_and_not_the_access() {
    let f = findings_with_map(UAF);
    let uaf = find(&f, "use-after-free");
    assert!(
        uaf.contains("(t.c:5:"),
        "the free's own location is the bare parenthetical: {uaf:?}"
    );
    assert!(
        !uaf.contains("(t.c:6"),
        "line 6 is where the access is, and it must not be presented as where the free was: \
         {uaf:?}"
    );
}

/// Without a map, the report still says everything it can. **The control.**
///
/// Every other test in this file runs with a `SourceMap`, so a fix could reasonably make one
/// mandatory — and `Engine::new` is called without one all over this repo. A run with no map
/// must still produce the finding and must not claim a line it cannot resolve.
#[test]
fn without_a_source_map_the_report_still_names_the_fault() {
    let m = harness::lower(UAF);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let f = r.findings();
    let uaf = find(&f, "use-after-free");
    assert!(
        !uaf.contains("t.c:"),
        "with no map there is no location to name, and inventing one is worse than omitting \
         it: {uaf:?}"
    );
}
