//! **An address *inside* an aggregate initializer.**
//!
//! Waves 189 and 190 fixed the initializers that are *one* address, which `GlobalInit::Addr`
//! can express. This is the shape it cannot: a struct with a pointer field among scalars, an
//! array of several pointers. `Addr { g, off }` describes the whole global as a single
//! address, and `Bytes` cannot carry provenance — 020 §3 says so, and it is why lowering
//! falls through to `Zero` here.
//!
//! The observable result is that every such pointer reads as null:
//!
//! ```text
//!   struct S { int *p; int n; }; struct S s = { &g, 3 };   s.p != 0  ->  0
//!   int *arr[2] = { &g, &h };                              arr[1]    ->  null deref
//!   char *tab[2] = { "ab", "cd" };                         tab[1][0] ->  null deref
//! ```
//!
//! The third is the one that decides this is worth a representation change rather than
//! another special case. A table of string literals is how every error-message array and
//! every VPP node-name table is written, and a table of *function* pointers is what
//! `VLIB_REGISTER_NODE` builds.
//!
//! # Asserted as behaviour, not as a representation
//!
//! Every test here runs the program and looks at what it computed. The representation is
//! free to be a relocation list, a byte string with provenance, or something else — pinning
//! `GlobalInit::Relocated { .. }` in a test would make the design harder to change for no
//! gain, and 020's contracts are about what the engine *does*.

mod harness;

use chiero_exec::Engine;
use chiero_solver::TermArena;

/// Run `probe` and return what it computed, plus how many findings it raised.
fn run(src: &str) -> (Option<i32>, usize) {
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = Engine::new(&m).with_entry("probe").run(&mut arena);
    let v = r
        .states()
        .iter()
        .find_map(|s| s.return_value_bits(&mut arena))
        .map(|b| b as u32 as i32);
    (v, r.findings().len())
}

/// A pointer field beside a scalar one: both must survive.
///
/// The scalar is in the fixture on purpose. A fix that represented the whole aggregate as
/// one address would make `s.p` right and `s.n` garbage, and the two must come out of the
/// same initializer.
#[test]
fn a_pointer_field_in_a_struct_initializer_is_an_address() {
    let decl = "int g = 7; struct S { int *p; int n; }; struct S s = { &g, 3 };\n";
    assert_eq!(
        run(&format!("{decl}int probe(void){{ return s.p != 0; }}")).0,
        Some(1),
        "`s.p` points at `g`, so it is not null"
    );
    assert_eq!(
        run(&format!("{decl}int probe(void){{ return *s.p; }}")),
        (Some(7), 0),
        "and it points at `g` specifically, whose value is 7"
    );
    assert_eq!(
        run(&format!("{decl}int probe(void){{ return s.n; }}")).0,
        Some(3),
        "the scalar field beside it still holds its own initializer"
    );
}

/// An array of pointers, indexed past the first element.
///
/// `arr[1]` rather than `arr[0]`, because a representation that carried only the first
/// address would satisfy the obvious fixture.
#[test]
fn an_array_of_pointers_holds_every_address() {
    let decl = "int g = 7; int h = 9; int *arr[2] = { &g, &h };\n";
    assert_eq!(
        run(&format!("{decl}int probe(void){{ return *arr[0]; }}")),
        (Some(7), 0),
        "the first element points at `g`"
    );
    assert_eq!(
        run(&format!("{decl}int probe(void){{ return *arr[1]; }}")),
        (Some(9), 0),
        "and the second at `h`, at its own offset"
    );
}

/// **A table of string literals**, which is how error-message arrays are written.
#[test]
fn a_table_of_string_literals_is_readable() {
    let decl = "char *tab[2] = { \"ab\", \"cd\" };\n";
    assert_eq!(
        run(&format!("{decl}int probe(void){{ return tab[0][0]; }}")),
        (Some(i32::from(b'a')), 0),
        "the first string's first byte"
    );
    assert_eq!(
        run(&format!("{decl}int probe(void){{ return tab[1][1]; }}")),
        (Some(i32::from(b'd')), 0),
        "and into the second string, which needs both the table entry and its target"
    );
}

/// A null written *into* an aggregate stays null.
///
/// The control, and the same one wave 190 needed: a fix that gave every pointer-typed slot
/// an object would pass everything above and invent one here.
#[test]
fn a_null_in_an_aggregate_initializer_stays_null() {
    let decl = "int g = 7; struct S { int *p; int n; }; struct S s = { 0, 3 };\n";
    assert_eq!(
        run(&format!("{decl}int probe(void){{ return s.p != 0; }}")).0,
        Some(0),
        "the program wrote a null and must get one"
    );
}
