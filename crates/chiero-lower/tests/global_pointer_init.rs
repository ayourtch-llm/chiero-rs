//! **A pointer-typed global is non-null unless the program wrote a null.**
//!
//! §9 asked for this as an *invariant* rather than a case list, and the reason is what makes
//! the class hard to see: `GlobalInit::Zero` is the fall-through for any initializer
//! `encode_init` cannot handle, and it is **also** the correct value for an uninitialized
//! object (C11 6.7.9p10). A dropped initializer and a genuine zero are the same value, so
//! nothing downstream can tell them apart and no existing test noticed.
//!
//! Wave 189 found the first instance — `int (*t)(int *) = helper;` reading as null — while
//! looking for something else. A survey then found the rest:
//!
//! ```text
//!   char *s = "hi";                 ZERO
//!   int *p = (int *)&g;             ZERO
//!   int *p = ga + 2;                ZERO
//!   struct S s = { &g };            ZERO
//!   int *arr[2] = { &g, &h };       ZERO
//! ```
//!
//! Every one of those is a pointer that compares equal to null, in a program where it
//! definitely is not. `char *s = "hi";` is the one that matters most: a file-scope string
//! pointer is in every C program there is.
//!
//! # What this file does and does not cover
//!
//! The first three are **one address**, which `GlobalInit::Addr` can already express, and
//! this wave fixes them. The last two are an *aggregate with an address inside* — a struct
//! whose other fields are bytes, an array of several addresses — and `Addr` describes the
//! whole global as a single address, so they need a representation that mixes bytes with
//! relocations. That is a design change and is recorded in §9 rather than half-done here.

mod harness;

use chiero_cir::GlobalInit;

/// The initializer a named global lowered to.
fn init_of(src: &str, name: &str) -> GlobalInit {
    let m = harness::lower(src);
    m.globals
        .iter()
        .find(|g| &*g.name == name)
        .unwrap_or_else(|| panic!("no global `{name}` in {src}"))
        .init
        .clone()
}

/// Is this an initializer that gives the pointer a real object?
fn is_address(i: &GlobalInit) -> bool {
    matches!(i, GlobalInit::Addr { .. } | GlobalInit::FuncAddr(_))
}

/// **The invariant.** Each of these initializes a pointer to something that exists.
#[test]
fn a_pointer_global_initialized_to_an_address_is_not_null() {
    for (what, src, name) in [
        ("&g", "int g; int *p = &g;", "p"),
        ("array decay", "int ga[4]; int *p = ga;", "p"),
        ("&ga[2]", "int ga[4]; int *p = &ga[2];", "p"),
        // A cast changes the type and not the address. C11 6.6p9 admits it in a constant
        // expression, and `(void *)` on a pointer initializer is ubiquitous.
        ("cast", "int g; int *p = (int *)&g;", "p"),
        ("void cast", "int g; void *p = (void *)&g;", "p"),
        // Pointer arithmetic on a file-scope array, which C11 6.6p9 also admits.
        ("ga + 2", "int ga[4]; int *p = ga + 2;", "p"),
        ("&ga[0] + 1", "int ga[4]; int *p = &ga[0] + 1;", "p"),
        // **The common one.** A string literal is an array with static storage duration,
        // so its address is a constant expression like any other.
        ("string literal", "char *s = \"hi\";", "s"),
    ] {
        let i = init_of(src, name);
        assert!(
            is_address(&i),
            "`{what}`: this pointer is not null in any execution, but lowered to {i:?}"
        );
    }
}

/// And a pointer the program *did* write null to stays null.
///
/// The control, and it is the reason the invariant has to be stated as "unless the program
/// wrote a null" rather than "is never `Zero`": a fix that made every pointer global an
/// address would pass the test above and invent an object for a genuine null.
#[test]
fn a_pointer_global_the_program_nulled_stays_null() {
    for (what, src) in [
        ("= 0", "int *p = 0;"),
        ("= (void *)0", "int *p = (void *)0;"),
        ("uninitialized", "int *p;"),
    ] {
        let i = init_of(src, "p");
        assert!(
            !is_address(&i),
            "`{what}` is a null pointer and must not gain an object: {i:?}"
        );
    }
}

/// The offsets are the ones C computes, not merely *an* address.
///
/// Separate from the invariant above because "non-null" is a much weaker claim than
/// "correct". A fix that pointed every derived pointer at offset 0 of the right object
/// would satisfy the first test and silently move `p` four bytes.
#[test]
fn a_derived_pointer_global_lands_at_the_right_offset() {
    for (what, src, want) in [
        ("&ga[2]", "int ga[4]; int *p = &ga[2];", 8),
        ("ga + 2", "int ga[4]; int *p = ga + 2;", 8),
        ("&ga[0] + 1", "int ga[4]; int *p = &ga[0] + 1;", 4),
        ("&ga[1] + 2", "int ga[4]; int *p = &ga[1] + 2;", 12),
        // **Subtraction, and both directions of it.** Mutation found no fixture used `-`
        // at all, so a version that added instead survived. `2 + ga` is the same address as
        // `ga + 2` (C makes `+` commute); `2 - ga` is not an address and not even valid C,
        // which is why only `+` may take its operands either way round.
        ("&ga[3] - 1", "int ga[4]; int *p = &ga[3] - 1;", 8),
        ("ga + 3 - 2", "int ga[4]; int *p = ga + 3 - 2;", 4),
        ("2 + ga", "int ga[4]; int *p = 2 + ga;", 8),
    ] {
        let GlobalInit::Addr { off, .. } = init_of(src, "p") else {
            panic!("`{what}` did not lower to an address");
        };
        assert_eq!(off, want, "`{what}` is {want} bytes into `ga`");
    }
}
