//! File-scope variables lower to **valid** CIR.
//!
//! Not a numbered contract; found in wave 111 while trying to make a global produce a
//! finding. Two independent defects surfaced in five minutes, both making `verify` reject
//! the module — so nothing downstream runs at all, and every checker, pass and engine test
//! is silently inapplicable to any translation unit that indexes a file-scope array.
//!
//! That is most of C, and all of VPP: `ip4_main`, the node registration tables, every
//! `static` counter array. The corpus fixtures are function-local almost throughout, which
//! is how this survived 111 waves.
//!
//! **`verify` is the assertion.** These tests do not check shapes; they check that lowering
//! produced something the rest of the system will accept, which is the property that was
//! actually broken.

mod harness;
use harness::lower;

/// Lower `src` and return the verifier's complaints.
fn errors(src: &str) -> Vec<String> {
    let m = lower(src);
    if std::env::var("DUMP").is_ok() {
        eprintln!("{}", chiero_cir::text::print(&m));
    }
    chiero_cir::verify::verify(&m)
        .iter()
        .filter(|e| e.is_error())
        .map(|e| format!("{:?}: {}", e.kind, e.detail))
        .collect()
}

/// **The defect.** Indexing a file-scope array.
///
/// `PtrAdd base must be pointer-typed, got Int(32)` — `lvalue_addr`'s `Ident` arm looks
/// only in `fs().locals`, so a global name falls through to the value path and the index
/// arithmetic is done on the array's first *element* rather than on its address.
#[test]
fn indexing_a_global_array_verifies() {
    assert!(
        errors("int g[4]; int f(void) { return g[1]; }").is_empty(),
        "{:#?}",
        errors("int g[4]; int f(void) { return g[1]; }")
    );
}

/// Writing through the same path, which is the half that corrupts memory rather than
/// merely reading wrongly.
#[test]
fn assigning_to_a_global_array_element_verifies() {
    let e = errors("int g[4]; void f(int n) { g[2] = n; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// A **symbolic** index, so the constant-folding path is not the only one covered.
#[test]
fn a_symbolic_index_into_a_global_verifies() {
    let e = errors("int g[4]; int f(int i) { return g[i]; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// `&g[1]` — the address is the value, so a fix that only repaired loads is visible.
#[test]
fn taking_the_address_of_a_global_element_verifies() {
    let e = errors("int g[4]; int *f(void) { return &g[1]; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// A **global struct member**, which reaches `lvalue_addr` through `Member` rather than
/// `Index` and so is a second route to the same missing case.
#[test]
fn a_global_struct_member_verifies() {
    let e = errors("struct S { int a; int b; }; struct S g; int f(void) { return g.b; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// **The second defect.** Writing through a cast that discards `const`.
///
/// `WidthMismatch`. Undefined behaviour in C and chiero should *report* it (021 c21), which
/// it cannot do while the module is rejected before it runs.
#[test]
fn writing_through_a_cast_away_from_const_verifies() {
    let e = errors("const int g = 1; void f(void) { *(int *)&g = 2; }");
    assert!(e.is_empty(), "{e:#?}");
}

/// A plain global read, as the control.
///
/// **`verify` alone was too weak here, and it mattered.** This test passed all along while
/// `int g; int f(void) { return g; }` lowered to `ret undef:i32` — the global was in no
/// table, the read produced nothing, and the module verified perfectly because `Undef` is
/// valid CIR. The real defect was never "two verifier errors"; it was that lowering had no
/// notion of a file-scope variable at all, and a read of one silently became "unknown",
/// which suppresses every finding downstream instead of producing a wrong one.
///
/// So the assertion is what the *program* says, not what the verifier tolerates.
#[test]
fn reading_a_global_scalar_is_not_undef() {
    let e = errors("int g; int f(void) { return g; }");
    assert!(e.is_empty(), "{e:#?}");

    let m = lower("int g; int f(void) { return g; }");
    assert_eq!(m.globals.len(), 1, "`g` is a global: {:?}", m.globals);
    assert_eq!(&*m.globals[0].name, "g");

    let text = chiero_cir::text::print(&m);
    assert!(
        text.contains("addrglobal"),
        "the read goes through the global's address: {text}"
    );
    assert!(
        !text.contains("ret undef"),
        "and is not silently unknown — an `Undef` return suppresses every finding \
         downstream rather than producing a wrong one: {text}"
    );
}

/// `static` and `extern` carry the linkage and initializer 020 §3 records.
///
/// `Extern` is not `Zero`: a definition in another TU has bytes chiero has never seen, and
/// saying zero would let the engine prove things about a value it does not have.
#[test]
fn storage_class_decides_linkage_and_initializer() {
    let m = lower("static int s; extern int e; int p; int f(void) { return s + e + p; }");
    let g = |n: &str| {
        m.globals
            .iter()
            .find(|x| &*x.name == n)
            .unwrap_or_else(|| panic!("no `{n}` in {:?}", m.globals))
    };
    assert_eq!(g("s").linkage, chiero_cir::Linkage::Internal);
    assert_eq!(g("e").linkage, chiero_cir::Linkage::External);
    assert_eq!(g("p").linkage, chiero_cir::Linkage::External);
    assert_eq!(g("e").init, chiero_cir::GlobalInit::Extern);
    assert_eq!(
        g("s").init,
        chiero_cir::GlobalInit::Zero,
        "C11 6.7.9p10: static storage with no initializer is zero"
    );
}

/// **Assigning to a global scalar**, and taking its address.
///
/// These are the shapes that reach `lvalue_addr` for a file-scope name — reads go through
/// the rvalue path instead. Mutation showed the `lvalue_addr` branch was unreachable from
/// every other fixture in this file, which is a fixture gap rather than dead code: without
/// these, removing that branch changes nothing observable.
#[test]
fn assigning_to_a_global_scalar_verifies() {
    let e = errors("int g; void f(int n) { g = n; }");
    assert!(e.is_empty(), "{e:#?}");
    let text = chiero_cir::text::print(&lower("int g; void f(int n) { g = n; }"));
    assert!(
        text.contains("addrglobal") && text.contains("store"),
        "the write goes to the global's address: {text}"
    );
}

/// `&g` on a scalar — the address is the value, with no load.
#[test]
fn taking_the_address_of_a_global_scalar_verifies() {
    let e = errors("int g; int *f(void) { return &g; }");
    assert!(e.is_empty(), "{e:#?}");
    let text = chiero_cir::text::print(&lower("int g; int *f(void) { return &g; }"));
    assert!(text.contains("addrglobal"), "{text}");
    assert!(
        !text.contains("ret undef"),
        "`&g` is a real address, not unknown: {text}"
    );
}

/// A **global struct member written through**, which reaches `lvalue_addr` via `Member`.
#[test]
fn assigning_to_a_global_struct_member_verifies() {
    let e = errors("struct S { int a; int b; }; struct S g; void f(int n) { g.b = n; }");
    assert!(e.is_empty(), "{e:#?}");
}

// ---------------------------------------------------------------------------------------
// Initializers (020 §3's `GlobalInit`)
// ---------------------------------------------------------------------------------------

fn init_of(src: &str, name: &str) -> chiero_cir::GlobalInit {
    let m = lower(src);
    m.globals
        .iter()
        .find(|g| &*g.name == name)
        .unwrap_or_else(|| panic!("no `{name}` in {:?}", m.globals))
        .init
        .clone()
}

/// **A scalar initializer reaches the CIR.**
///
/// `int g = 7;` recorded `GlobalInit::Zero`: the initializer was parsed and thrown away.
/// That is a **wrong answer rather than a missing one**, which is worse than what wave 112
/// fixed — an engine reading `g` gets 0 and proves things about a program that does not
/// exist, and every path predicated on `g` is explored as if the value were zero.
#[test]
fn a_scalar_global_initializer_is_recorded() {
    assert_eq!(
        init_of("int g = 7;", "g"),
        chiero_cir::GlobalInit::Bytes(vec![7, 0, 0, 0]),
        "little-endian bytes of 7 at `int` width"
    );
}

/// An **array** initializer, element by element.
#[test]
fn an_array_global_initializer_is_recorded() {
    assert_eq!(
        init_of("int g[4] = {1, 2, 3, 4};", "g"),
        chiero_cir::GlobalInit::Bytes(vec![
            1, 0, 0, 0, //
            2, 0, 0, 0, //
            3, 0, 0, 0, //
            4, 0, 0, 0,
        ])
    );
}

/// **A partial initializer zero-fills the rest** (C11 6.7.9p21), rather than shortening
/// the object — a consumer reading past the initialized part must see zeros, not the end
/// of a byte string.
#[test]
fn a_partial_array_initializer_zero_fills() {
    assert_eq!(
        init_of("int g[4] = {1, 2};", "g"),
        chiero_cir::GlobalInit::Bytes(vec![1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        "16 bytes, not 8"
    );
}

/// A **string** initializer, with its terminator.
#[test]
fn a_string_global_initializer_is_recorded() {
    assert_eq!(
        init_of("char s[4] = \"hi\";", "s"),
        chiero_cir::GlobalInit::Bytes(vec![b'h', b'i', 0, 0])
    );
}

/// **A struct initializer respects the layout's padding.**
///
/// `struct { char a; int b; }` puts `b` at offset 4, so the bytes are not the fields
/// concatenated — an implementation that appended field encodings gives 5 bytes and every
/// offset after the first is wrong.
#[test]
fn a_struct_global_initializer_respects_padding() {
    assert_eq!(
        init_of("struct S { char a; int b; }; struct S g = {1, 2};", "g"),
        chiero_cir::GlobalInit::Bytes(vec![1, 0, 0, 0, 2, 0, 0, 0]),
        "`b` sits at offset 4, so three padding bytes come first"
    );
}

/// **The controls.** No initializer is still `Zero`, and `extern` is still `Extern` — a
/// fix that recorded bytes for everything would break both.
#[test]
fn uninitialized_and_extern_globals_are_unchanged() {
    assert_eq!(init_of("int g;", "g"), chiero_cir::GlobalInit::Zero);
    assert_eq!(
        init_of("static int g;", "g"),
        chiero_cir::GlobalInit::Zero,
        "C11 6.7.9p10"
    );
    assert_eq!(
        init_of("extern int g;", "g"),
        chiero_cir::GlobalInit::Extern,
        "bytes chiero has never seen are not zero"
    );
}

/// **A designated initializer is refused, not silently reordered.**
///
/// `int g[4] = {[2] = 5};` puts 5 at index 2. Encoding the items positionally would put it
/// at index 0 — a wrong answer, and exactly the class this wave exists to remove. Chiero
/// does not encode designators yet, so the initializer is refused whole and the global
/// falls back to `Zero`: less information, but nothing invented.
#[test]
fn a_designated_initializer_is_refused_rather_than_reordered() {
    assert_eq!(
        init_of("int g[4] = {[2] = 5};", "g"),
        chiero_cir::GlobalInit::Zero,
        "refused whole — encoding it positionally would put 5 at index 0"
    );

    // The struct form, which reaches a different arm of the encoder.
    assert_eq!(
        init_of("struct S { int a; int b; }; struct S g = {.b = 5};", "g"),
        chiero_cir::GlobalInit::Zero
    );

    // And the *undesignated* forms still encode, or "refuse designators" would be
    // indistinguishable from "refuse everything".
    assert!(matches!(
        init_of("int g[4] = {9};", "g"),
        chiero_cir::GlobalInit::Bytes(_)
    ));
}

/// A **bit-field member** is refused too: its bytes are not a whole-field write, and
/// encoding it as one would overwrite the neighbours it shares a storage unit with.
#[test]
fn a_bitfield_initializer_is_refused() {
    assert_eq!(
        init_of(
            "struct B { unsigned a:3; unsigned b:5; }; struct B g = {1, 2};",
            "g"
        ),
        chiero_cir::GlobalInit::Zero
    );
}

/// **A `const` global is marked read-only.**
///
/// Found by reading wave 114's new corpus golden: `static const int table[4]` printed as
/// `global static @table`, with no `const`. `is_const` was hardcoded `false`.
///
/// It is not cosmetic. 021 contract 21 — "writing to a `readonly` global is exactly one
/// finding and does not alter the bytes" — cannot fire on any global while nothing marks
/// one read-only, so the checker is correct and unreachable. VPP's tables are `const`
/// precisely so that writing to one is a bug.
#[test]
fn a_const_global_is_read_only() {
    let m = lower("const int g = 1; int f(void) { return g; }");
    assert!(
        m.globals
            .iter()
            .find(|x| &*x.name == "g")
            .expect("g")
            .is_const,
        "{:?}",
        m.globals
    );

    // An array of `const` elements, which is where the qualifier sits on the *element*
    // type rather than on the declaration's — and is the shape the corpus file uses.
    let m = lower("static const int t[4] = {1, 2, 3, 4}; int f(void) { return t[0]; }");
    assert!(
        m.globals
            .iter()
            .find(|x| &*x.name == "t")
            .expect("t")
            .is_const,
        "{:?}",
        m.globals
    );

    // The control: a mutable global is not marked, or `is_const` is just `true`.
    let m = lower("int g = 1; int f(void) { return g; }");
    assert!(
        !m.globals
            .iter()
            .find(|x| &*x.name == "g")
            .expect("g")
            .is_const,
        "a writable global stays writable: {:?}",
        m.globals
    );
}

// ---------------------------------------------------------------------------------------
// Function pointers (wave 119–120)
// ---------------------------------------------------------------------------------------

/// **A call through a function-pointer variable lowers.**
///
/// `callee_of` looked only in `module.funcs`, so a name declared as a *variable* was
/// reported "call to undeclared function" — and 015 §7 turns any diagnostic into refusing
/// the whole enclosing function, so the file lowered to nothing.
#[test]
fn a_call_through_a_function_pointer_lowers() {
    let src = "static int twice(int v) { return v * 2; }\n\
               int main(void) { int (*fn)(int) = twice; return fn(7); }\n";
    let e = errors(src);
    assert!(e.is_empty(), "{e:#?}");
    let text = chiero_cir::text::print(&lower(src));
    assert!(
        text.contains("addrfunc"),
        "the function's address is taken (C11 6.3.2.1p4): {text}"
    );
    assert!(
        text.contains("callind") || text.contains("call %"),
        "and the call goes through it indirectly rather than naming a function: {text}"
    );
}

/// **A conditional yielding a pointer gets a pointer slot.**
///
/// `conditional`'s result slot was hardcoded `CTy::Int`, so `pick ? twice : thrice` stored
/// a `Ptr` into an `Int(32)` slot and failed verification — refusing the enclosing function
/// again. Wave 119 blamed sema for this; sema was right, and the wrong answer was one `?:`
/// away.
#[test]
fn a_pointer_valued_conditional_gets_a_pointer_slot() {
    let src = "static int twice(int v) { return v * 2; }\n\
               static int thrice(int v) { return v * 3; }\n\
               int main(int pick) { int (*fn)(int) = pick ? twice : thrice; return fn(7); }\n";
    let e = errors(src);
    assert!(e.is_empty(), "{e:#?}");

    let m = lower(src);
    let main = m.funcs.iter().find(|f| &*f.name == "main").expect("main");
    assert!(
        main.allocas
            .iter()
            .filter(|a| a.name.is_none())
            .any(|a| a.ty == chiero_cir::CTy::Ptr),
        "the conditional's temporary is a pointer: {:?}",
        main.allocas
            .iter()
            .map(|a| (a.name.clone(), a.ty.clone()))
            .collect::<Vec<_>>()
    );
}

/// **An integer-valued conditional is still an integer**, so the fix reads the type rather
/// than making everything a pointer.
#[test]
fn an_integer_conditional_keeps_its_width() {
    let m = lower("int f(int n) { return n ? 1 : 2; }");
    let f = m.funcs.iter().find(|f| &*f.name == "f").expect("f");
    assert!(
        f.allocas
            .iter()
            .filter(|a| a.name.is_none())
            .all(|a| a.ty != chiero_cir::CTy::Ptr),
        "{:?}",
        f.allocas.iter().map(|a| a.ty.clone()).collect::<Vec<_>>()
    );
}
