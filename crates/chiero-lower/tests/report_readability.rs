//! **Every heap finding names an allocation counter instead of the allocation.**
//!
//! 023 §9 makes the report the product, and `MemFault`'s `Display` impl carries a doc comment
//! saying so in as many words: it exists because a struct dump "makes a reader decode chiero's
//! internals". It then hands them two pieces of exactly that:
//!
//! ```text
//!   use-after-free: ObjectId(3) was freed at bytes 85..92 before this access
//!   double-free: ObjectId(3) was already freed at bytes 85..92
//!   uninitialized-read: read at offset 0 of ObjectId(3) touches bit 0, ...
//!   out-of-bounds: 1-byte access at offset 8 of ObjectId(3), which is 4 bytes
//! ```
//!
//! `ObjectId(3)` is an allocation counter. The engine's own comment at the substitution site
//! says what is wrong with it — "it means nothing to a reader, and it is not stable across pass
//! configurations, so the *same defect in the same program* printed differently with `mem2reg`
//! on" — and then substitutes a name only when there is one. A `malloc` has none, so every
//! finding about heap memory keeps the counter.
//!
//! That is not a corner: it is the whole ASan channel for dynamically allocated memory, which
//! waves 178 and 179 built. Four of chiero's six memory-UB classes are unreadable in the case
//! they exist for.
//!
//! `bytes 85..92` is the second piece. A `BytePos` range is chiero's index into the
//! preprocessed buffer; a reader has neither that buffer nor a reason to want offsets into it.
//!
//! # What a fix owes
//!
//! A description the reader could act on, built from what the engine knows: this is heap memory,
//! it is four bytes, and it was allocated somewhere nameable. Not a name invented for an
//! anonymous object — there isn't one — and not the counter.
//!
//! # Why the named case is asserted too
//!
//! The substitution that names `ca` and `sa` is load-bearing and easy to break while replacing
//! it. A fix that described every object generically would satisfy the assertions above and make
//! every *named* finding worse, which is the trade nobody wants.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

const HEAP: &str = "void *malloc(unsigned long); void free(void *);\n";

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
}

/// **No finding hands the reader an `ObjectId`.**
///
/// All four heap classes, because the substitution site is shared and a fix keyed on one fault
/// kind would leave the others leaking.
#[test]
fn no_finding_names_an_object_id() {
    for (what, src) in [
        (
            "use after free",
            "int probe(void){ int *p = malloc(4); free(p); return *p; }",
        ),
        (
            "double free",
            "int probe(void){ int *p = malloc(4); free(p); free(p); return 0; }",
        ),
        (
            "uninitialized heap read",
            "int probe(void){ int *p = malloc(4); return *p; }",
        ),
        (
            "out of bounds",
            "int probe(void){ char *p = malloc(4); return p[8]; }",
        ),
    ] {
        let f = findings(&format!("{HEAP}{src}"));
        assert!(
            !f.is_empty(),
            "`{what}`: the fixture must produce the finding it is named for"
        );
        assert!(
            f.iter().all(|m| !m.contains("ObjectId")),
            "`{what}`: an allocation counter is not something a reader can act on: {f:?}"
        );
    }
}

/// A heap finding says the memory came from an allocation.
///
/// Separate from the assertion above, which a fix could satisfy by deleting the object from the
/// sentence entirely. "was freed before this access" without saying *what* was freed is shorter
/// and no more useful.
#[test]
fn a_heap_finding_describes_the_allocation() {
    let f = findings(&format!(
        "{HEAP}int probe(void){{ int *p = malloc(4); free(p); return *p; }}"
    ));
    let uaf = f
        .iter()
        .find(|m| m.starts_with("use-after-free"))
        .unwrap_or_else(|| panic!("no use-after-free among {f:?}"));
    assert!(
        uaf.contains("allocation") || uaf.contains("heap"),
        "the reader has to be told which memory this is about: {uaf:?}"
    );
}

/// **No finding hands the reader a `BytePos` range.**
///
/// `freed at bytes 85..92` is an offset into the preprocessed buffer, which the reader does not
/// have. Asserted for the two faults that name a *second* location, since those are the only
/// messages that render a span at all.
#[test]
fn no_finding_names_raw_byte_positions() {
    for (what, src) in [
        (
            "use after free",
            "int probe(void){ int *p = malloc(4); free(p); return *p; }",
        ),
        (
            "double free",
            "int probe(void){ int *p = malloc(4); free(p); free(p); return 0; }",
        ),
    ] {
        let f = findings(&format!("{HEAP}{src}"));
        assert!(
            f.iter()
                .all(|m| !m.contains("bytes 8") && !m.contains("..")),
            "`{what}`: a byte range into the preprocessed buffer is not a location: {f:?}"
        );
    }
}

/// The named case still names the variable. **The control.**
///
/// The name is matched as a *word*, and mutation is why. Deleting the name lookup altogether
/// left this test passing: `ca` is a substring of "the 8-byte unnamed lo**ca**l", and `g` of
/// "unnamed **g**lobal". Both generic descriptions contain the name they were supposed to have
/// replaced — wave 184's substring rule, arriving from a direction no one would guess. Asserting
/// that "unnamed" is absent is the other half: it is the word every nameless description uses.
#[test]
fn a_named_object_is_still_named() {
    for (what, src, want) in [
        (
            "local array",
            "int probe(void){ char ca[8]; return ca[0]; }",
            "ca",
        ),
        (
            "global array",
            "int g[4]; int probe(void){ return g[9]; }",
            "g",
        ),
    ] {
        let f = findings(src);
        let names_it = f.iter().any(|m| {
            m.split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|w| w == want)
        });
        assert!(
            names_it,
            "`{what}`: the engine knows this object's name and the report must use it: {f:?}"
        );
        assert!(
            f.iter().all(|m| !m.contains("unnamed")),
            "`{what}`: this object has a name, so no report about it is of an unnamed \
             object: {f:?}"
        );
    }
}
