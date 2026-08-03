//! Covers: 050 contract 6.
//!
//! **The fixtures preprocess real C** rather than assembling a `SourceMap` by hand. A
//! hand-built map would assert that this operation can read a structure this test just
//! wrote, which is the one thing never in doubt; the question is whether it can read what
//! `chiero-pp` actually records.

use chiero_pp::{Config, preprocess_str};

/// 050 contract 6: the full chain, **innermost first**, each frame naming the macro, its
/// definition site, and its body text.
///
/// `vec_add1` is the worked example in 010 §4.2 and the shape that matters for VPP: a
/// one-line call site whose meaning lives three headers away. This fixture is that shape in
/// miniature — an outer macro whose body invokes an inner one — because the property under
/// test is the chain, not the arithmetic.
#[test]
fn the_chain_is_innermost_first_with_each_definition_site_and_body() {
    let src = "#define _vec_resize(V, N) ((V) = _vec_grow((V), (N)))\n\
               #define vec_add1(V, E) (_vec_resize((V), 1), (V)[0] = (E))\n\
               void f(int *v) { vec_add1(v, 3); }\n";
    let tu = preprocess_str("vec.h", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);

    // Line 3, at the call site of `vec_add1`.
    let chain = chiero_tool::explain_macro_expansion(&tu.source_map, "vec.h", 3, None);

    let names: Vec<&str> = chain.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["_vec_resize", "vec_add1"],
        "innermost first: the macro whose body the token came from, then its caller"
    );

    // **Each frame carries the definition site**, which is the whole point: an LLM reading
    // `vec_add1(v, 3)` has no way to find line 1 of a header it was never shown.
    assert_eq!(chain[0].def_line, 1);
    assert_eq!(chain[1].def_line, 2);
    assert_eq!(chain[0].def_file.as_deref(), Some("vec.h"));

    // **And the body text**, so the answer is readable without a second lookup.
    assert_eq!(chain[0].body, "((V) = _vec_grow((V), (N)))");
    assert_eq!(chain[1].body, "(_vec_resize((V), 1), (V)[0] = (E))");

    // The call site of the outermost frame is the line asked about.
    assert_eq!(chain[1].call_line, 3);
}

/// **A line with no macro on it answers empty, not an error and not a guess.** 050 §1 makes
/// every operation total; an LLM that gets a refusal here learns nothing, and one that gets
/// a nearby chain learns something false.
#[test]
fn a_line_with_no_expansion_has_an_empty_chain() {
    let tu = preprocess_str("t.c", "#define A 1\nint x = A;\nint y = 2;\n", Config::default());
    assert!(chiero_tool::explain_macro_expansion(&tu.source_map, "t.c", 3, None).is_empty());
    assert!(!chiero_tool::explain_macro_expansion(&tu.source_map, "t.c", 2, None).is_empty());
    // A file that is not in the map is the same answer: nothing to say.
    assert!(chiero_tool::explain_macro_expansion(&tu.source_map, "other.c", 2, None).is_empty());
}
