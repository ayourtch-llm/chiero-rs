//! Covers: 013 contracts 1, 2, 3, 5, 12, 13, 15, 16, 17.
//!
//! **Every test here is written against its own mutation.** 013's contracts are mostly of
//! the form "X parses as A, and the lookalike Y parses as B", and a parser that answered
//! A unconditionally would satisfy half of each one. So the fixtures come in pairs with
//! the *same token shape* and opposite expected answers wherever the contract allows it:
//! `T * x;` against `A * B;` for the typedef problem, `int a[0]` against `int a[]` for
//! array kinds, a truncating error flood against a three-error file for the diagnostic
//! cap. Where a contract says "diagnoses", there is a companion fixture that must *not*
//! diagnose, because "diagnose everything" is otherwise a passing implementation.

use chiero_ast::{ArrayLen, BinOp, Builtin, DeclKind, ExprKind, StmtId, StmtKind, TypeKind};
use chiero_parse::{MAX_DIAGNOSTICS, ParsedTu, ScopedTypedefs, parse_tu};
use chiero_pp::{Config, PreprocessedTu, preprocess_str};

/// Preprocess and parse.
///
/// The preprocessor's own diagnostics are asserted empty first. Without that, a fixture
/// with a typo produces an empty token stream, the parser produces an empty tree, and
/// **every negative assertion in this file passes for the wrong reason** — the same trap
/// that cost three fixtures in wave 79 and one in wave 81.
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

/// The statements of `name`'s body, which must be a definition with a compound body.
fn body_of(p: &ParsedTu, name: &str) -> Vec<StmtId> {
    let found = p
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
        .unwrap_or_else(|| {
            panic!(
                "no definition of `{name}` in {:?}",
                p.ast
                    .items()
                    .iter()
                    .map(|&i| &p.ast.decl(i).kind)
                    .collect::<Vec<_>>()
            )
        });
    match &p.ast.stmt(found).kind {
        StmtKind::Compound(v) => v.clone(),
        other => panic!("`{name}`'s body is not a compound statement: {other:?}"),
    }
}

/// **Contract 1, the sharp form.** `T * x;` and `A * B;` are the same five token kinds.
/// The only thing that can tell them apart is the oracle, which is why 013 §3 exists at
/// all — so the test uses that exact shape rather than the trivially distinguishable
/// `T x;`, where a parser that ignored the oracle entirely would still be right half the
/// time.
#[test]
fn the_same_token_shape_is_a_declaration_or_a_multiplication_by_typedef_alone() {
    let (_, p) = parse("typedef int T; void f(void) { T * x; }");
    let stmts = body_of(&p, "f");
    assert_eq!(stmts.len(), 1);
    let decls = match &p.ast.stmt(stmts[0]).kind {
        StmtKind::Decl(d) => d.clone(),
        other => panic!("`T * x;` with `T` a typedef must be a declaration, got {other:?}"),
    };
    assert_eq!(decls.len(), 1);
    match &p.ast.decl(decls[0]).kind {
        DeclKind::Var { name, ty, .. } => {
            assert_eq!(p.text(name.expect("named")), Some("x"));
            assert!(
                matches!(p.ast.ty(*ty).kind, TypeKind::Ptr(_)),
                "and `x` is a pointer, not an int: {:?}",
                p.ast.ty(*ty).kind
            );
        }
        other => panic!("expected a variable, got {other:?}"),
    }

    let (_, p) = parse("int A; int B; void f(void) { A * B; }");
    let stmts = body_of(&p, "f");
    assert_eq!(stmts.len(), 1);
    let e = match &p.ast.stmt(stmts[0]).kind {
        StmtKind::Expr(e) => *e,
        other => panic!("`A * B;` with `A` an object must be an expression, got {other:?}"),
    };
    match &p.ast.expr(e).kind {
        ExprKind::Binary { op, .. } => assert_eq!(*op, BinOp::Mul),
        other => panic!("and specifically a multiplication: {other:?}"),
    }
}

/// **Contract 1, the plain form**, kept because it is what the contract literally says.
#[test]
fn a_typedef_name_introduces_a_declaration() {
    let (_, p) = parse("typedef int T; T x;");
    let items = p.ast.items();
    let last = p.ast.decl(*items.last().expect("two items"));
    match &last.kind {
        DeclKind::Var { name, ty, .. } => {
            assert_eq!(p.text(name.expect("named")), Some("x"));
            assert!(matches!(p.ast.ty(*ty).kind, TypeKind::Named(_)));
        }
        other => panic!("expected `x` declared with the typedef's name, got {other:?}"),
    }
}

/// **Contract 2.** `typedef int T; void f(int T, T x);` — the second `T` is a parameter
/// name, so it is no longer a type, and `T x` is not a declaration.
///
/// The companion fixture is the point: an identical declaration whose first parameter is
/// *not* named `T` must parse silently. Without it, "diagnoses" is satisfied by a parser
/// that diagnoses every prototype it sees.
#[test]
fn a_parameter_shadows_a_typedef_for_the_rest_of_the_declarator() {
    let (_, p) = parse("typedef int T; void f(int T, T x);");
    assert!(
        !p.diagnostics.is_empty(),
        "`T` is a parameter by the time `T x` is read, so this is not a valid declaration"
    );

    let (_, clean) = parse("typedef int T; void f(int a, T x);");
    assert!(
        clean.diagnostics.is_empty(),
        "but the same shape without the shadowing name is ordinary C: {:?}",
        clean.diagnostics
    );
}

/// **Contract 3.** A typedef declared in an inner scope does not survive `exit_scope`.
///
/// Same `T * b;` text in both functions, opposite answers — so an oracle that never
/// popped, or one that popped too much, fails in one direction or the other.
#[test]
fn an_inner_typedef_does_not_escape_its_scope() {
    let (_, p) = parse(
        "void f(void) { typedef int T; T * a; }\n\
         void g(void) { T * b; }\n",
    );

    let f = body_of(&p, "f");
    assert_eq!(f.len(), 2, "the typedef and the declaration");
    assert!(
        matches!(p.ast.stmt(f[1]).kind, StmtKind::Decl(_)),
        "inside `f`, `T` is a type: {:?}",
        p.ast.stmt(f[1]).kind
    );

    let g = body_of(&p, "g");
    assert_eq!(g.len(), 1);
    let e = match &p.ast.stmt(g[0]).kind {
        StmtKind::Expr(e) => *e,
        other => panic!("outside `f`, `T` is not a type, so this multiplies: {other:?}"),
    };
    assert!(matches!(
        p.ast.expr(e).kind,
        ExprKind::Binary { op: BinOp::Mul, .. }
    ));
}

/// **Contract 5.** `int a[0]` and `int a[]` are different declarations, and 1165 VPP files
/// depend on one or the other. Collapsing both to `Fixed(0)`, or both to `Unspecified`,
/// is the mutation this asserts against — hence `assert_ne!` on the pair as well as the
/// two exact values.
#[test]
fn zero_length_and_flexible_array_members_are_distinguishable() {
    let (_, p) = parse("struct S { int a[0]; };\nstruct F { int n; int a[]; };\n");

    let len_of = |tag: &str| -> ArrayLen {
        let ty = p
            .ast
            .items()
            .iter()
            .find_map(|&id| match &p.ast.decl(id).kind {
                DeclKind::TagDef { ty } => match &p.ast.ty(*ty).kind {
                    TypeKind::Tag {
                        name: Some(n),
                        members: Some(m),
                        ..
                    } if p.text(*n) == Some(tag) => Some(m.clone()),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or_else(|| panic!("no definition of `struct {tag}`"));
        let last = *ty.last().expect("at least one member");
        match &p.ast.decl(last).kind {
            DeclKind::Var { ty, .. } => match p.ast.ty(*ty).kind {
                TypeKind::Array { len, .. } => len,
                ref other => panic!("`{tag}`'s last member is not an array: {other:?}"),
            },
            other => panic!("not a member declaration: {other:?}"),
        }
    };

    let zero = len_of("S");
    let flexible = len_of("F");
    assert_eq!(
        zero,
        ArrayLen::Zero,
        "`int a[0]` is the GNU zero-length array"
    );
    assert_eq!(
        flexible,
        ArrayLen::Unspecified,
        "`int a[]` is a flexible array member"
    );
    assert_ne!(
        zero, flexible,
        "and they are not the same thing, which is the whole point of the contract"
    );
}

/// **Contract 12.** `_Static_assert` — 140 VPP files reach it through `STATIC_ASSERT`.
#[test]
fn static_assert_parses_with_its_condition_and_message() {
    let (_, p) = parse("_Static_assert(sizeof(int) == 4, \"msg\");");
    assert_eq!(p.ast.items().len(), 1);
    match &p.ast.decl(p.ast.items()[0]).kind {
        DeclKind::StaticAssert { cond, msg } => {
            assert_eq!(
                p.text(msg.expect("a message was written")),
                Some("\"msg\""),
                "the message keeps its spelling, quotes included — 014 unescapes"
            );
            assert!(
                matches!(
                    p.ast.expr(*cond).kind,
                    ExprKind::Binary { op: BinOp::Eq, .. }
                ),
                "and the condition is the comparison as written, unfolded: {:?}",
                p.ast.expr(*cond).kind
            );
        }
        other => panic!("expected a static assertion, got {other:?}"),
    }
}

/// **Contract 13.** `__int128`, and its unsigned form, which is a *different* builtin —
/// a parser that mapped both to `Int128` would pass an assertion naming only the first.
#[test]
fn int128_and_unsigned_int128_are_separate_builtins() {
    let (_, p) = parse("__int128 x; unsigned __int128 y;");
    let builtin_of = |name: &str| -> Builtin {
        p.ast
            .items()
            .iter()
            .find_map(|&id| match &p.ast.decl(id).kind {
                DeclKind::Var {
                    name: Some(n), ty, ..
                } if p.text(*n) == Some(name) => match p.ast.ty(*ty).kind {
                    TypeKind::Builtin(b) => Some(b),
                    ref other => panic!("`{name}` is not a builtin type: {other:?}"),
                },
                _ => None,
            })
            .unwrap_or_else(|| panic!("no declaration of `{name}`"))
    };
    assert_eq!(builtin_of("x"), Builtin::Int128);
    assert_eq!(builtin_of("y"), Builtin::UInt128);
}

/// **Contract 15.** An unclosed brace still yields every complete top-level declaration
/// that preceded it. This is the property that decides whether chiero can analyse a
/// 1M-line codebase at all: one bad file must cost one file, not the tree.
#[test]
fn declarations_before_an_unclosed_brace_survive() {
    let (_, p) = parse("int a; int b; void f(void) { int c;\n");
    assert!(
        !p.diagnostics.is_empty(),
        "the unclosed brace is reported, not swallowed"
    );
    let names: Vec<&str> = p
        .ast
        .items()
        .iter()
        .filter_map(|&id| match &p.ast.decl(id).kind {
            DeclKind::Var { name: Some(n), .. } => p.text(*n),
            DeclKind::Func { name, .. } => p.text(*name),
            _ => None,
        })
        .collect();
    assert!(
        names.contains(&"a") && names.contains(&"b"),
        "both complete declarations before the damage are present: {names:?}"
    );
}

/// **Contract 16.** The cap bites, *and* it does not fire on an ordinary broken file.
///
/// One assertion alone is passable by a constant: "always 100 and truncated" satisfies the
/// first, "never truncated" satisfies the second. Both together need a real cap.
#[test]
fn the_diagnostic_cap_bites_only_when_there_is_a_flood() {
    let flood = ")".repeat(MAX_DIAGNOSTICS * 3);
    let (_, p) = parse(&flood);
    assert_eq!(
        p.diagnostics.len(),
        MAX_DIAGNOSTICS,
        "a cascade is cut off at the cap"
    );
    assert!(p.truncated, "and the run records that it was cut off");

    let (_, small) = parse(")))");
    assert!(
        small.diagnostics.len() >= 3 && small.diagnostics.len() < MAX_DIAGNOSTICS,
        "a file with three problems reports about three: {}",
        small.diagnostics.len()
    );
    assert!(
        !small.truncated,
        "and is not marked truncated, or the flag means nothing"
    );
}

/// **Contract 17.** Every node's span, through `expansion_loc`, lands in a real file.
///
/// The fixture is deliberately macro-heavy, and the test **proves it can observe the
/// failure it names** before asserting the absence of one: it checks that some node's
/// span is inside a macro expansion, so a parser that stamped every node with the same
/// root-context span could not pass by producing nothing interesting. That guard is the
/// wave-63 lesson — an instrument that cannot see X happening proves nothing by not
/// seeing it.
#[test]
fn every_node_span_maps_into_a_real_file() {
    let (tu, p) = parse(
        "#define DECL(n) int n;\n\
         #define PAIR(a, b) DECL(a) DECL(b)\n\
         PAIR(x, y)\n\
         void f(void) { x = y * 2; }\n",
    );

    assert!(
        p.ast.node_count() > 8,
        "a tree this small proves nothing: {} nodes",
        p.ast.node_count()
    );
    let spans: Vec<_> = p.ast.all_spans().collect();
    assert!(
        spans.iter().any(|s| !s.ctx.is_root()),
        "no node came from a macro, so this fixture does not test what it claims"
    );

    for sp in spans {
        assert_ne!(
            sp,
            chiero_span::Span::DUMMY,
            "013 §5: a synthesized node gets a zero-width span at a real position, \
             never a dummy"
        );
        let loc = tu
            .source_map
            .expansion_loc(sp)
            .unwrap_or_else(|| panic!("span {sp:?} does not resolve to any file"));
        assert!(loc.line >= 1, "1-based lines: {loc:?}");
        assert_eq!(
            tu.source_map.lookup_file(loc.pos),
            Some(loc.file),
            "and the expansion position is inside the file it names: {loc:?}"
        );
    }
}
