//! **One more array element and the uninitialized-read finding disappears.**
//!
//! `struct P { char a; long b; }` is sixteen bytes: one written, seven of padding, eight written.
//! An array of them, every field assigned, read through a `char *` at a symbolic index — the read
//! can land on padding, so `maybe-uninitialized-read` is the right answer:
//!
//! ```text
//!   struct P pa[4];   224 unwritten bits   maybe-uninitialized-read at offset 55
//!   struct P pa[5];   280 unwritten bits   (nothing)
//! ```
//!
//! Nothing about the program changed except its size. `init_guard` eliminates `arr.init`'s store
//! chain with `select_expand`, which refuses past `EXPAND_LIMIT` (256) so a linear chain cannot
//! become a quadratic formula — and on refusal `read_term_at` reports no fault at all. The check
//! is skipped, and the report says nothing about having skipped it.
//!
//! # This is wave 205's defect at a different threshold
//!
//! Wave 205 fixed `ITE_THRESHOLD` deciding *whether a fault exists*: sixteen candidates reported
//! and thirty-two were silent. This is the same shape one layer in. A bound that changes how hard
//! chiero tries is 023 §7's business; a bound that changes what chiero *finds*, without saying so,
//! is what §7 exists to forbid.
//!
//! # What the fix owes, and does not owe
//!
//! It does not owe an unbounded expansion — the limit is there for a reason and the reason is
//! sound. It owes an *answer*: an undecided guard is a `maybe`, which is exactly what wave 204's
//! discharge does with `Unknown`, and what the memory model has to stop doing is dropping the
//! question on the floor.
//!
//! The run is `Bounded` at five elements, and that is not a defence. It is bounded because the
//! symbolic offset could not be enumerated (`a_symbolic_ptr_add_offset_is_a_gap`), which is a
//! different statement about a different thing. A reader told "the offset enumeration hit its
//! budget" has not been told "and the initialization check was skipped".

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

/// `n` elements of `struct P`, every field written, read at a symbolic byte offset.
///
/// Padding is the only way to get a large object that is *mostly* initialized without executing a
/// loop — and a loop is no good here, since 63 iterations end the run `Bounded` before it reaches
/// the read (wave 206 tried it).
fn padded_array(n: usize) -> String {
    let mut src = String::from("struct P { char a; long b; };\nint probe(int i){ struct P pa[");
    src.push_str(&n.to_string());
    src.push_str("];\n");
    for k in 0..n {
        src.push_str(&format!("pa[{k}].a = 1; pa[{k}].b = 2;\n"));
    }
    src.push_str("char *p = (char *)pa;\nreturn p[i & 63];\n}\n");
    src
}

fn findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    Engine::new(&m)
        .with_entry("probe")
        .run(&mut arena)
        .findings()
}

/// **The same program at two sizes must agree that the fault exists.**
///
/// Four and five, because four is the largest that fits under the limit and five the smallest that
/// does not — the cliff is between them, and a fixture on one side alone proves nothing.
#[test]
fn a_padded_array_reports_its_padding_at_every_size() {
    for n in [2usize, 4, 5, 8] {
        let f = findings(&padded_array(n));
        assert!(
            f.iter().any(|m| m.contains("uninitialized-read")),
            "`pa[{n}]`: the read can land on padding no program ever wrote, and the answer \
             cannot depend on how many elements there are: {f:?}"
        );
    }
}

/// A fully written object of the same size stays silent. **The control.**
///
/// The one that stops the cheapest wrong fix. Reporting `maybe-uninitialized-read` whenever the
/// expansion is refused would satisfy the test above and put a false report on every large
/// object — which is the answer wave 202 declined to ship, arrived at from the other direction.
#[test]
fn a_large_object_with_no_padding_stays_silent() {
    for n in [5usize, 8] {
        let src = format!(
            "int probe(int i){{ long la[{n}];\n\
             for (int k = 0; k < {n}; k++) la[k] = 1;\n\
             char *p = (char *)la; return p[i & 31]; }}\n"
        );
        let f = findings(&src);
        assert!(
            f.iter().all(|m| !m.contains("uninitialized-read")),
            "`la[{n}]` is written end to end, so nothing here is uninitialized: {f:?}"
        );
    }
}
