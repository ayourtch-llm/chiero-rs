//! Covers: 060 contract 10.
//!
//! VPP's dominant abstraction (060 §3): a list macro defined once, expanded with different
//! per-item macros to generate enums, string tables and dispatch code. Contract 10 requires
//! that an answer names **the list macro, the per-item macro, and the specific item**.

use chiero_pp::{Config, preprocess_str};

/// The shape `vnet/tunnel` writes, reduced to two items.
fn x_macro_tu() -> chiero_pp::PreprocessedTu {
    let src = "#define foreach_flag \\\n  _(NONE, \"none\", 0x0) \\\n  _(SET_DF, \"set-df\", 0x2)\n\
               int v = (\n#define _(a, b, c) FLAG_##a |\n  foreach_flag\n#undef _\n  0);\n";
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    assert_eq!(
        tu.token_texts().collect::<Vec<_>>().join(" "),
        "int v = ( FLAG_NONE | FLAG_SET_DF | 0 ) ;"
    );
    tu
}

/// **One chain per item, each naming its own item.** Every item resolves to the *same*
/// written position — the one `foreach_flag` token — so returning the single deepest chain
/// answers "what does this line do" with one arbitrary item out of 47 and no way to ask for
/// the rest. That is the "dumping expanded soup" 060 §3 exists to replace.
#[test]
fn every_item_of_a_list_macro_gets_its_own_chain() {
    let tu = x_macro_tu();
    let chains = chiero_tool::explain_macro_expansion(&tu.source_map, "t.c", 6, None);
    assert_eq!(chains.len(), 2, "one chain per generated item");

    for c in &chains {
        // The per-item macro is innermost; the list macro is outermost.
        assert_eq!(c.first().map(|f| f.name.as_str()), Some("_"));
        assert_eq!(c.last().map(|f| f.name.as_str()), Some("foreach_flag"));
    }

    // **The specific item**: the arguments the per-item macro was invoked with.
    let items: Vec<Vec<String>> = chains.iter().map(|c| c[0].args.clone()).collect();
    assert_eq!(
        items,
        vec![
            vec!["NONE".to_string(), "\"none\"".to_string(), "0x0".to_string()],
            vec![
                "SET_DF".to_string(),
                "\"set-df\"".to_string(),
                "0x2".to_string()
            ],
        ]
    );

    // The list macro takes no arguments, and an empty list is not the same as "unknown".
    assert!(chains[0].last().expect("outermost").args.is_empty());
}

/// **Items are distinct sites even though they share a written position.** 060 §3 requires
/// that editing one line of a `foreach_` list impacts exactly what that line generated. A
/// site list that collapses 47 items into 1 cannot support that at all.
#[test]
fn each_item_is_its_own_expansion_site() {
    let tu = x_macro_tu();
    let s = chiero_tool::expansion_sites(&tu.source_map, "_", None, 50);
    assert_eq!(s.total, 2, "two invocations of the per-item macro");
    // Both are written at the same place in the user's file...
    assert!(s.sites.iter().all(|x| x.line == 6));
    // ...and are told apart by where they sit in the list macro's body.
    assert_eq!(
        s.sites.iter().map(|x| x.item_line).collect::<Vec<_>>(),
        [2, 3]
    );
}
