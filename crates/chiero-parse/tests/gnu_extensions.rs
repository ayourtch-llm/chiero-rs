//! Covers: 013 contracts 4, 6, 7, 8, 9, 10, 11, 14, 18.
//!
//! 013 §4 measured every one of these against `/home/ubuntu/vpp/src`, so the file counts
//! in each test's comment are the reason it exists rather than decoration: an extension
//! used by 1019 files is not optional, and one used by a single file is a diagnose-and-
//! continue case. Several of these constructs were already implemented when this file was
//! written and had simply never been exercised — which is the situation wave 85 found two
//! defects in, so each test still carries the discriminator that would catch a plausible
//! wrong implementation rather than merely asserting the happy path.

use chiero_ast::{ArrayLen, DeclKind, Designator, ExprKind, StmtKind, TypeKind};
use chiero_parse::{ParsedTu, ScopedTypedefs, parse_tu};
use chiero_pp::{Config, PreprocessedTu, preprocess_str};

fn parse(src: &str) -> (PreprocessedTu, ParsedTu) {
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(
        tu.diagnostics.is_empty(),
        "the fixture itself must preprocess cleanly, or this test proves nothing: {:?}",
        tu.diagnostics
    );
    assert!(!tu.tokens.is_empty(), "and it must produce tokens");
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    (tu, parsed)
}

fn no_diagnostics(p: &ParsedTu, what: &str) {
    assert!(
        p.diagnostics.is_empty(),
        "{what} must parse without diagnostics: {:?}",
        p.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

fn func_body(p: &ParsedTu, name: &str) -> Vec<chiero_ast::StmtId> {
    let body = p
        .ast
        .items()
        .iter()
        .find_map(|&id| match &p.ast.decl(id).kind {
            DeclKind::Func {
                name: n,
                body: Some(b),
                ..
            } if p.text(*n) == Some(name) => Some(*b),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no definition of `{name}`"));
    match &p.ast.stmt(body).kind {
        StmtKind::Compound(v) => v.clone(),
        other => panic!("`{name}`'s body is not compound: {other:?}"),
    }
}

/// **Contract 4.** Old-style K&R definitions: the names appear in the declarator and the
/// types in declarations between the `)` and the `{`.
///
/// The discriminator is that the parameters end up **typed**. A parser that skipped the
/// intervening declarations would still produce a function with two parameters and a
/// body, and every structural assertion short of this one would pass.
#[test]
fn old_style_parameter_declarations_type_the_parameters() {
    let (_, p) = parse("int f(a, b) int a; int b; { return a + b; }");
    no_diagnostics(&p, "a K&R definition");

    let ty = p
        .ast
        .items()
        .iter()
        .find_map(|&id| match &p.ast.decl(id).kind {
            DeclKind::Func { name, ty, .. } if p.text(*name) == Some("f") => Some(*ty),
            _ => None,
        })
        .expect("no definition of `f`");
    let (params, kr) = match &p.ast.ty(ty).kind {
        TypeKind::Func { params, kr, .. } => (params.clone(), *kr),
        other => panic!("`f` is not a function: {other:?}"),
    };
    assert!(kr, "and it is recorded as an old-style list");
    assert_eq!(params.len(), 2);
    for (d, expect) in params.iter().zip(["a", "b"]) {
        match &p.ast.decl(*d).kind {
            DeclKind::Var { name, ty, .. } => {
                assert_eq!(p.text(name.expect("named")), Some(expect));
                assert!(
                    matches!(
                        p.ast.ty(*ty).kind,
                        TypeKind::Builtin(chiero_ast::Builtin::Int)
                    ),
                    "`{expect}` must be typed by the declaration after the `)`, not left \
                     as an untyped placeholder: {:?}",
                    p.ast.ty(*ty).kind
                );
            }
            other => panic!("not a parameter: {other:?}"),
        }
    }
    assert_eq!(func_body(&p, "f").len(), 1, "and the body still parses");
}

/// **Contract 6.** `__attribute__` in each of the positions 013 §4 lists — 155 VPP files,
/// and `packed`/`aligned` are two of the three attributes that change *analysis
/// semantics*, so attaching one to the wrong entity corrupts every offset in a struct.
///
/// The discriminator is the second declarator: in
/// `int x __attribute__((aligned(64))), y;` the attribute belongs to `x` **and not to
/// `y`**. Both share one specifier node, so an implementation that hangs the attribute on
/// the shared base type silently aligns everything in the declaration.
#[test]
fn attributes_attach_to_the_entity_they_were_written_on() {
    let (_, p) = parse(
        "struct __attribute__((packed)) A { int a; };\n\
         struct B { int b; } __attribute__((packed));\n\
         int x __attribute__((aligned(64))), y;\n",
    );
    no_diagnostics(&p, "attributes in each grammatical position");

    let tag_attrs = |tag: &str| -> Vec<String> {
        p.ast
            .items()
            .iter()
            .find_map(|&id| match &p.ast.decl(id).kind {
                DeclKind::TagDef { ty } => match &p.ast.ty(*ty).kind {
                    TypeKind::Tag { name: Some(n), .. } if p.text(*n) == Some(tag) => Some(
                        p.ast
                            .ty(*ty)
                            .attrs
                            .iter()
                            .filter_map(|a| p.text(a.name).map(str::to_owned))
                            .collect(),
                    ),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or_else(|| panic!("no definition of `struct {tag}`"))
    };
    assert!(
        tag_attrs("A").contains(&"packed".to_string()),
        "after the `struct` keyword: {:?}",
        tag_attrs("A")
    );
    assert!(
        tag_attrs("B").contains(&"packed".to_string()),
        "after the closing brace: {:?}",
        tag_attrs("B")
    );

    let var_attrs = |name: &str| -> Vec<String> {
        p.ast
            .items()
            .iter()
            .find_map(|&id| match &p.ast.decl(id).kind {
                DeclKind::Var {
                    name: Some(n), ty, ..
                } if p.text(*n) == Some(name) => Some(
                    p.ast
                        .ty(*ty)
                        .attrs
                        .iter()
                        .filter_map(|a| p.text(a.name).map(str::to_owned))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no declaration of `{name}`"))
    };
    assert!(
        var_attrs("x").contains(&"aligned".to_string()),
        "on the declarator: {:?}",
        var_attrs("x")
    );
    assert!(
        !var_attrs("y").contains(&"aligned".to_string()),
        "and **not** on `y`, which shares only the specifiers: {:?}",
        var_attrs("y")
    );
}

/// **Contract 7.** GNU statement expressions — 217 VPP files.
///
/// The value is the *last* expression in the block, so the test reaches into the compound
/// statement and checks its final statement is an expression rather than merely that a
/// `StmtExpr` node exists.
#[test]
fn a_statement_expression_carries_a_block_whose_last_statement_is_its_value() {
    let (_, p) = parse("int x = ({ int t = 1; t + 1; });");
    no_diagnostics(&p, "a statement expression");

    let init = p
        .ast
        .items()
        .iter()
        .find_map(|&id| match &p.ast.decl(id).kind {
            DeclKind::Var {
                name: Some(n),
                init: Some(i),
                ..
            } if p.text(*n) == Some("x") => Some(*i),
            _ => None,
        })
        .expect("no initializer for `x`");
    let block = match p.ast.expr(init).kind {
        ExprKind::StmtExpr(b) => b,
        ref other => panic!("`({{ ... }})` must be a statement expression: {other:?}"),
    };
    let stmts = match &p.ast.stmt(block).kind {
        StmtKind::Compound(v) => v.clone(),
        other => panic!("not a block: {other:?}"),
    };
    assert_eq!(stmts.len(), 2, "a declaration and the value expression");
    let last = match &p.ast.stmt(*stmts.last().unwrap()).kind {
        StmtKind::Expr(e) => *e,
        other => panic!("the value must be the last expression: {other:?}"),
    };
    assert!(
        matches!(p.ast.expr(last).kind, ExprKind::Binary { .. }),
        "which is `t + 1`: {:?}",
        p.ast.expr(last).kind
    );

    // **And the block is a scope.** A mutation that opened the statement expression's
    // block without entering a scope passed everything above — the structure is identical
    // either way, and only a *name* declared inside can tell. A typedef is the sharpest
    // probe, because whether it escaped changes how the next statement parses rather than
    // merely what some field says.
    let (_, p) = parse(
        "void g(void) {\n\
         int x = ({ typedef int T; T v = 1; v; });\n\
         T * y;\n\
         }\n",
    );
    let stmts = func_body(&p, "g");
    assert_eq!(stmts.len(), 2);
    let e = match &p.ast.stmt(stmts[1]).kind {
        StmtKind::Expr(e) => *e,
        other => panic!(
            "`T` was declared inside a statement expression, so outside it `T * y;` is a \
             multiplication, not a declaration: {other:?}"
        ),
    };
    assert!(matches!(
        p.ast.expr(e).kind,
        ExprKind::Binary {
            op: chiero_ast::BinOp::Mul,
            ..
        }
    ));
}

/// **Contract 8.** `typeof` and `__typeof__` — 52 VPP files.
///
/// Both spellings, and both operand forms: `typeof(x)` takes an **expression** and
/// `typeof(int *)` a type name. Conflating them would make `typeof(x)` mean "the type
/// named x", which is wrong wherever a variable and a typedef share a name.
#[test]
fn typeof_accepts_both_spellings_and_both_operand_forms() {
    let (_, p) = parse(
        "int v; int *pv;\n\
         typeof(v) a;\n\
         __typeof__(*pv) b;\n\
         typeof(int *) c;\n",
    );
    no_diagnostics(&p, "`typeof` in both spellings");

    let kind_of = |name: &str| -> TypeKind {
        p.ast
            .items()
            .iter()
            .find_map(|&id| match &p.ast.decl(id).kind {
                DeclKind::Var {
                    name: Some(n), ty, ..
                } if p.text(*n) == Some(name) => Some(p.ast.ty(*ty).kind.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no declaration of `{name}`"))
    };
    assert!(
        matches!(kind_of("a"), TypeKind::TypeofExpr(_)),
        "`typeof(v)` is an expression operand: {:?}",
        kind_of("a")
    );
    assert!(
        matches!(kind_of("b"), TypeKind::TypeofExpr(_)),
        "`__typeof__(*pv)` too, and the spelling must not matter: {:?}",
        kind_of("b")
    );
    assert!(
        matches!(kind_of("c"), TypeKind::TypeofType(_)),
        "`typeof(int *)` is a type operand, which is a different node: {:?}",
        kind_of("c")
    );
}

/// **Contract 9.** GNU case ranges — 7 VPP files, and the failure mode is silent: a
/// parser that dropped the `... 5` would produce a valid `case 1:` and analyse four fewer
/// values than the program handles.
#[test]
fn a_case_range_keeps_both_endpoints() {
    let (_, p) = parse("void f(int c) { switch (c) { case 1 ... 5: break; case 9: break; } }");
    no_diagnostics(&p, "a case range");

    let mut ranged = 0;
    let mut plain = 0;
    for s in p.ast.stmts() {
        if let StmtKind::Case { hi, .. } = &s.kind {
            if hi.is_some() {
                ranged += 1;
            } else {
                plain += 1;
            }
        }
    }
    assert_eq!(ranged, 1, "`case 1 ... 5:` keeps its upper endpoint");
    assert_eq!(
        plain, 1,
        "and `case 9:` does not grow one, or `hi` means nothing"
    );
}

/// **Contract 10.** `asm` is **parsed, not modeled** (013 §4) — 31 VPP files. Lowering
/// turns it into an opaque effect; silently treating it as a no-op would be unsound in
/// the direction that produces confident wrong answers.
///
/// Both forms: the basic `asm volatile ("" ::: "memory")` barrier, which is the one VPP
/// actually leans on, and extended asm with operands.
#[test]
fn asm_statements_parse_with_their_operands_and_clobbers() {
    let (_, p) = parse(
        "void f(int *dst, int src) {\n\
         __asm__ __volatile__ (\"\" ::: \"memory\");\n\
         asm volatile (\"mov %1, %0\" : \"=r\" (*dst) : \"r\" (src) : \"memory\", \"cc\");\n\
         }\n",
    );
    no_diagnostics(&p, "`asm` in both forms");

    let stmts = func_body(&p, "f");
    assert_eq!(stmts.len(), 2);

    let barrier = match &p.ast.stmt(stmts[0]).kind {
        StmtKind::Asm(a) => a.clone(),
        other => panic!("the memory barrier must be an asm statement: {other:?}"),
    };
    assert!(barrier.volatile, "`__volatile__` is recorded");
    assert!(barrier.outputs.is_empty() && barrier.inputs.is_empty());
    assert_eq!(
        barrier
            .clobbers
            .iter()
            .filter_map(|c| p.text(*c))
            .collect::<Vec<_>>(),
        ["\"memory\""],
        "and the clobber list survives the two empty operand sections"
    );

    let extended = match &p.ast.stmt(stmts[1]).kind {
        StmtKind::Asm(a) => a.clone(),
        other => panic!("extended asm must be an asm statement: {other:?}"),
    };
    assert_eq!(extended.outputs.len(), 1, "one output operand");
    assert_eq!(extended.inputs.len(), 1, "one input operand");
    assert_eq!(
        extended.clobbers.len(),
        2,
        "and two clobbers, so the sections are not being merged"
    );
    assert_eq!(
        p.text(extended.outputs[0].constraint),
        Some("\"=r\""),
        "the constraint keeps its spelling; nothing here interprets it"
    );
}

/// **Contract 11.** Designated initializers — 1019 VPP files, the single most-used
/// extension in 013 §4's table.
///
/// All four shapes in one initializer, and the assertion is on the *designator chain*:
/// `.b.c` is two designators in written order, not one, and flattening it would place the
/// value on `b` instead of on `b.c`.
#[test]
fn every_designator_shape_parses_with_its_chain_intact() {
    let (_, p) = parse(
        "struct I { int a; struct { int c; } b; int arr[8]; };\n\
         struct I i = { .a = 1, .b.c = 2, .arr = { [3] = 4, [1 ... 2] = 5 } };\n",
    );
    no_diagnostics(&p, "designated initializers");

    let init = p
        .ast
        .items()
        .iter()
        .find_map(|&id| match &p.ast.decl(id).kind {
            DeclKind::Var {
                name: Some(n),
                init: Some(x),
                ..
            } if p.text(*n) == Some("i") => Some(*x),
            _ => None,
        })
        .expect("no initializer for `i`");
    let items = match &p.ast.expr(init).kind {
        ExprKind::InitList(v) => v.clone(),
        other => panic!("not an initializer list: {other:?}"),
    };
    assert_eq!(items.len(), 3);

    assert_eq!(items[0].designators.len(), 1, "`.a` is one designator");
    assert_eq!(
        items[1].designators.len(),
        2,
        "`.b.c` is **two**, in written order — flattening it initializes the wrong field"
    );
    assert!(matches!(items[1].designators[0], Designator::Field(f) if p.text(f) == Some("b")));
    assert!(matches!(items[1].designators[1], Designator::Field(f) if p.text(f) == Some("c")));

    let inner = match &p.ast.expr(items[2].value).kind {
        ExprKind::InitList(v) => v.clone(),
        other => panic!("`.arr`'s value is a nested list: {other:?}"),
    };
    assert_eq!(inner.len(), 2);
    assert!(
        matches!(inner[0].designators[0], Designator::Index(_)),
        "`[3] =` is an index designator"
    );
    assert!(
        matches!(inner[1].designators[0], Designator::Range(..)),
        "and `[1 ... 2] =` is a *range*, which is a different designator — reading it as \
         an index would initialize one element instead of two"
    );
}

/// **Contract 14.** A nested function definition produces **exactly one** diagnostic and
/// the enclosing function still parses. 013 §4 puts `__label__`/nested functions in the
/// "no" column: one VPP file uses them, and the right answer is to diagnose and skip
/// rather than to build machinery for it.
///
/// "Exactly one" is the whole contract. A parser that fell into its body and reported a
/// cascade would satisfy "diagnoses" while making the enclosing function unreadable, which
/// is the outcome §6's whole recovery section exists to avoid.
#[test]
fn a_nested_function_is_one_diagnostic_and_does_not_take_the_enclosing_function_with_it() {
    let (_, p) = parse(
        "int outer(int n) {\n\
         int inner(int m) { return m + 1; }\n\
         return n;\n\
         }\n",
    );
    assert_eq!(
        p.diagnostics.len(),
        1,
        "exactly one diagnostic, not a cascade: {:?}",
        p.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let stmts = func_body(&p, "outer");
    assert!(
        stmts
            .iter()
            .any(|&s| matches!(p.ast.stmt(s).kind, StmtKind::Return(Some(_)))),
        "and the statement *after* the nested function still parses: {:?}",
        stmts
            .iter()
            .map(|&s| &p.ast.stmt(s).kind)
            .collect::<Vec<_>>()
    );
}

/// **Contract 18.** A literal formed from two macro-produced fragments reports two
/// distinct `ExpnCtx`s.
///
/// This is the concatenation half of §4.1's headline claim. VPP builds format strings out
/// of macro-produced pieces, and a diagnostic that cannot say which fragment came from
/// which macro is not actionable — so the fragments cannot be recovered from the joined
/// span afterwards, and must be retained as they are consumed.
#[test]
fn concatenated_string_fragments_keep_one_expansion_context_each() {
    let (_, p) = parse(
        "#define PREFIX \"pre:\"\n\
         #define SUFFIX \":post\"\n\
         const char *s = PREFIX \"mid\" SUFFIX;\n",
    );
    no_diagnostics(&p, "a concatenated literal");

    let init = p
        .ast
        .items()
        .iter()
        .find_map(|&id| match &p.ast.decl(id).kind {
            DeclKind::Var {
                name: Some(n),
                init: Some(i),
                ..
            } if p.text(*n) == Some("s") => Some(*i),
            _ => None,
        })
        .expect("no initializer for `s`");
    let fragments = match &p.ast.expr(init).kind {
        ExprKind::Str { fragments } => fragments.clone(),
        other => panic!("not a string literal: {other:?}"),
    };
    assert_eq!(
        fragments.len(),
        3,
        "three constituents, not one joined text"
    );
    assert_eq!(
        fragments
            .iter()
            .filter_map(|f| p.text(f.spelling))
            .collect::<Vec<_>>(),
        ["\"pre:\"", "\"mid\"", "\":post\""],
        "in written order, spellings retained"
    );

    let macro_ctxs: std::collections::BTreeSet<u32> = fragments
        .iter()
        .filter(|f| !f.span.ctx.is_root())
        .map(|f| f.span.ctx.0)
        .collect();
    assert_eq!(
        macro_ctxs.len(),
        2,
        "the two macro-produced fragments carry **two distinct** contexts, so a \
         diagnostic can name which macro produced which piece: {:?}",
        fragments.iter().map(|f| f.span.ctx).collect::<Vec<_>>()
    );
    assert!(
        fragments.iter().any(|f| f.span.ctx.is_root()),
        "and the literally-written `\"mid\"` is still root, or the test is not \
         distinguishing contexts at all"
    );
}

/// **GNU asm labels**, which rename a symbol: `int f (void) __asm__ ("real");`
///
/// Not one of 013's numbered contracts, and it is here because the VPP corpus could not
/// see it. Reading only the *first* string fragment still parses cleanly and still reports
/// zero diagnostics — it just gives every redirected symbol the wrong name — so the corpus
/// gate is structurally blind to it and a mutation proved that.
///
/// glibc's `__ASMNAME` is `__STRING (prefix) cname`: **two** adjacent literals, the first
/// normally empty. So the first-fragment-only bug produces the label `""` for every
/// redirected function in `<string.h>`, and the label is the name the object file will
/// carry — which 030 must match against gcov records and 060 against VPP's multiarch
/// aliases. A silently empty linker name would send both looking for a symbol that does
/// not exist.
#[test]
fn an_asm_label_is_the_concatenation_of_all_its_fragments() {
    let (_, p) = parse(
        "extern int redirected (int n) __asm__ (\"\" \"actual_symbol\");\n\
         extern int plain (int n);\n",
    );
    no_diagnostics(&p, "asm labels");

    let decl = |name: &str| {
        *p.ast
            .items()
            .iter()
            .find(|&&id| match &p.ast.decl(id).kind {
                DeclKind::Func { name: n, .. } => p.text(*n) == Some(name),
                _ => false,
            })
            .unwrap_or_else(|| panic!("no declaration of `{name}`"))
    };

    let label = p
        .ast
        .asm_label(decl("redirected"))
        .expect("the asm label was recorded");
    assert_eq!(
        p.text(label),
        Some("actual_symbol"),
        "both fragments, joined, and stored as content rather than spelling — a linker \
         name is a name, not a quoted literal"
    );
    assert_eq!(
        p.ast.asm_label(decl("plain")),
        None,
        "and a declaration without one has none, or the side table leaks the previous \
         declaration's label onto everything after it"
    );
}

/// A guard on the array-length rule that contract 5 established, checked here because
/// designated initializers are where a wrong answer would show up first.
#[test]
fn a_sized_array_member_is_not_confused_with_a_flexible_one() {
    let (_, p) = parse("struct S { int n; int a[8]; };");
    no_diagnostics(&p, "a sized array member");
    let len = p
        .ast
        .types()
        .iter()
        .find_map(|t| match t.kind {
            TypeKind::Array { len, .. } => Some(len),
            _ => None,
        })
        .expect("no array type");
    assert!(
        matches!(len, ArrayLen::Fixed(_)),
        "`int a[8]` is a fixed-length array: {len:?}"
    );
}

/// **`__attribute__ ((fallthrough));` as a statement** — 013 §4's attribute positions, in the
/// one place the C grammar has no production for it at all: where a statement belongs.
///
/// All 3 of VPP's remaining `expected a type specifier` findings are this construct, in
/// `plugins/http/http.c:633`, `http2/http2.c:817` and `http3/qpack.c:981`. Each marks a
/// deliberate `switch` fallthrough, and gcc compiles all three silently in **both** modes —
/// measured, `-std=gnu11` and `-std=c11 -pedantic-errors`.
///
/// **The attribute is kept, not discarded into `Empty`.** gcc refuses a misplaced
/// `fallthrough` — "not preceding a case label or default label", and "invalid use of
/// attribute" outside a switch entirely — and a checker for that needs to know which
/// attribute was written. Folding this to `Empty` would parse the corpus and throw away the
/// only thing the rule needs. See the backlog note in HANDOFF §9: the position rule is not
/// implemented here because gcc decides it during gimplification and accepts the attribute at
/// the end of an `if` body inside a switch, so a syntactic approximation would over-reject.
#[test]
fn an_attribute_may_stand_where_a_statement_belongs() {
    // The `qpack.c` shape: a statement, the attribute, then the next label.
    let (_, p) = parse(
        "int f(int x){ int r = 0; switch (x) { case 7: r = 1; __attribute__ ((fallthrough));\n\
         case 5: r += 2; break; } return r; }\n",
    );
    no_diagnostics(&p, "an attribute statement before a case label");

    // gcc's reserved spelling is equally valid and VPP uses both forms elsewhere.
    let (_, p) = parse(
        "int f(int x){ int r = 0; switch (x) { case 7: r = 1; __attribute__ ((__fallthrough__));\n\
         case 5: r += 2; break; } return r; }\n",
    );
    no_diagnostics(&p, "the reserved spelling");

    // **The attribute survives into the AST.** A `StmtKind::Empty` here would pass every
    // assertion above while destroying what a position checker must read.
    let (_, p) = parse(
        "int f(int x){ int r = 0; switch (x) { case 1: r = 1; __attribute__ ((fallthrough));\n\
         case 2: break; } return r; }\n",
    );
    let named: Vec<String> = p
        .ast
        .stmts()
        .iter()
        .filter_map(|s| match &s.kind {
            StmtKind::Attr(attrs) => Some(attrs.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|a| p.text(a.name).map(str::to_owned))
        .collect();
    assert_eq!(
        named,
        vec!["fallthrough".to_string()],
        "the attribute is recorded, not discarded"
    );

    // **Two attributes and an empty list**, so the arm is not written for exactly one.
    let (_, p) = parse(
        "int f(int x){ int r = 0; switch (x) { case 1: r = 1;\n\
         __attribute__ ((fallthrough)) __attribute__ ((unused));\n\
         case 2: break; } return r; }\n",
    );
    no_diagnostics(&p, "two attribute specifiers in one statement");

    // **A declaration starting with an attribute is still a declaration**, which is the case
    // this arm must not steal: `__attribute__((unused)) int y = 1;` declares `y`.
    let (_, p) = parse("int f(void){ __attribute__ ((unused)) int y = 1; return y; }\n");
    no_diagnostics(
        &p,
        "an attributed declaration is not an attribute statement",
    );
    assert!(
        p.ast
            .stmts()
            .iter()
            .any(|s| matches!(&s.kind, StmtKind::Decl(_))),
        "the declaration is still a declaration"
    );
}
