//! **A path ends at its first definite fault, and everything after it is not reported.**
//!
//! `report_faults` states the rule — "the path ends at a definite crash; everything
//! reported before it is real; everything after it would be about a program that does not
//! exist" — and until wave 182 nothing tested it. It is easy to mistake for a limitation
//! and "fix", so it needs a test that says it is a decision.
//!
//! The decision is C's. An execution has no defined continuation past undefined behaviour:
//! once `a[5]` reads outside `a`, the standard describes nothing about what the program
//! does next, so a second fault reported afterwards is a fault of *someone's simulation*
//! of the program rather than of the program.
//!
//! AddressSanitizer can be asked to take the other position. With
//! `-fsanitize-recover=address` and `halt_on_error=0` it continues past a fault and reports
//! every one it meets:
//!
//! ```text
//!   ==48256==ERROR: AddressSanitizer: stack-buffer-overflow
//!   ==48256==ERROR: AddressSanitizer: heap-use-after-free
//! ```
//!
//! That is a useful thing for a sanitizer to offer and the wrong thing for chiero to copy.
//! It is also why the memory-UB oracle in `generated.rs` grades chiero against ASan's
//! **first** report and not its list: the two tools agree about the program up to that
//! point and are describing different things afterwards.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

const HEAP: &str = "void *malloc(unsigned long); void free(void *);\n";

/// Every memory finding, in order.
fn memory_findings(src: &str) -> Vec<String> {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    r.findings()
        .into_iter()
        .filter(|f| {
            f.contains("out-of-bounds") || f.contains("use-after") || f.contains("double-free")
        })
        .collect()
}

/// Two overflows, one report — the first.
///
/// The second access is on the same path and after the first, so it belongs to an execution
/// C does not describe. `b` is a *different object*, which is what makes this a real second
/// fault rather than a repeat of the first being deduplicated by 023 §6.1's key.
#[test]
fn only_the_first_of_two_overflows_is_reported() {
    let found = memory_findings(
        "int probe(void){ int a[2]={1,2}; int b[2]={3,4}; int x=a[5]; int y=b[7]; return x+y; }",
    );
    assert_eq!(
        found.len(),
        1,
        "the path ends at the first definite fault, so `b[7]` is never reached: {found:?}"
    );
    assert!(
        found[0].contains(" of a,"),
        "and the one report is the first fault, on `a`: {found:?}"
    );
}

/// The same across two *kinds* of fault, so the rule is not an artifact of both being
/// out-of-bounds.
///
/// Ordered both ways: whichever comes first is the one reported, which is the property.
/// A checker that simply preferred one kind over another would pass one of these and fail
/// the other.
#[test]
fn the_first_fault_wins_whichever_kind_it_is() {
    let overflow_first = memory_findings(&format!(
        "{HEAP}int probe(void){{ int a[2]={{1,2}}; int *p=(int*)malloc(8); p[0]=1; \
         int x=a[5]; free(p); int y=p[0]; return x+y; }}"
    ));
    assert!(
        overflow_first.len() == 1 && overflow_first[0].contains("out-of-bounds"),
        "the overflow comes first, so the later use-after-free is not reported: \
         {overflow_first:?}"
    );

    let use_after_free_first = memory_findings(&format!(
        "{HEAP}int probe(void){{ int a[2]={{1,2}}; int *p=(int*)malloc(8); p[0]=1; \
         free(p); int y=p[0]; int x=a[5]; return x+y; }}"
    ));
    assert!(
        use_after_free_first.len() == 1 && use_after_free_first[0].contains("use-after-free"),
        "and with the order swapped, the use-after-free is the one reported: \
         {use_after_free_first:?}"
    );
}

/// The rule is about *definite* faults, and a program with one fault still reports it.
///
/// The control: a fix that satisfied the tests above by reporting nothing at all would pass
/// both and fail here.
#[test]
fn a_single_fault_is_still_reported() {
    let found = memory_findings("int probe(void){ int a[2]={1,2}; return a[5]; }");
    assert_eq!(found.len(), 1, "one fault, one report: {found:?}");
}
