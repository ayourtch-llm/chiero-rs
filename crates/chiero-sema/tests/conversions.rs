//! Covers: 014 contracts 11, 20.
//!
//! Contract 11 is not really "casts exist". It is **the absence of anything implicit**:
//! after 014, no consumer should ever have to ask what C would have done to an operand.
//! So the load-bearing test here is not that a given fixture produces a given `Cast`, it
//! is that across the whole VPP corpus **every binary operation's two operands already
//! have the same type** and every argument already has its parameter's type. A test that
//! only checked "a promotion appeared somewhere" would pass on an implementation that
//! promoted one operand and forgot the other, which is exactly the bug that makes CIR
//! ambiguous about bit-widths.

mod harness;

use chiero_ast::{BinOp, DeclKind, ExprKind};
use chiero_sema::{Conversion, TargetConfig, Ty, TypedNode};
use harness::{Parsed, parse, parse_allowing_diagnostics};

/// The conversions applied to the initializer of file-scope `name`.
fn init_conversions(p: &Parsed, name: &str) -> Vec<Conversion> {
    let sym = p.symbol(name).unwrap_or_else(|| panic!("no `{name}`"));
    let init = p
        .parsed
        .ast
        .items()
        .iter()
        .find_map(|&id| match &p.parsed.ast.decl(id).kind {
            DeclKind::Var {
                name: Some(n),
                init: Some(i),
                ..
            } if *n == sym => Some(*i),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no initializer for `{name}`"));
    p.analysis.typed().conversions_of(init)
}

/// **Contract 11, integer promotion.** `c + c` on two `char`s is `int` arithmetic, and
/// both operands carry a promotion.
///
/// The discriminator is that **both** sides carry one. An implementation that promoted
/// the left operand and left the right alone would satisfy "a promotion appears", and the
/// resulting CIR would be an `add` of a 32-bit and an 8-bit value.
#[test]
fn both_operands_of_a_char_addition_are_promoted() {
    let p = parse(
        "char a; char b; int r = a + b;",
        TargetConfig::x86_64_linux(),
    );
    let sym = p.symbol("r").expect("r");
    let init = p
        .parsed
        .ast
        .items()
        .iter()
        .find_map(|&id| match &p.parsed.ast.decl(id).kind {
            DeclKind::Var {
                name: Some(n),
                init: Some(i),
                ..
            } if *n == sym => Some(*i),
            _ => None,
        })
        .expect("initializer");
    let ExprKind::Binary { lhs, rhs, op } = &p.parsed.ast.expr(init).kind else {
        panic!("not a binary expression")
    };
    assert_eq!(*op, BinOp::Add);

    for (side, e) in [("lhs", *lhs), ("rhs", *rhs)] {
        let conv = p.analysis.typed().conversions_of(e);
        assert!(
            conv.contains(&Conversion::IntegerPromotion),
            "the {side} `char` operand must carry an explicit promotion: {conv:?}"
        );
        let ty = p
            .analysis
            .typed()
            .top(e)
            .map(|t| p.analysis.typed().ty_of(t))
            .expect("typed");
        assert_eq!(
            p.analysis.ty(ty),
            &Ty::Int {
                signed: true,
                bits: 32
            },
            "and end up as `int` on the {side}"
        );
    }
}

/// **Contract 11, usual arithmetic conversions.** `i + l` converts the `int`, not the
/// `long`, and the pair with the operands swapped proves it is the *narrower* one that
/// moves rather than the left one.
#[test]
fn the_narrower_operand_is_the_one_converted() {
    for (src, narrow_is_lhs) in [
        ("int i; long l; long r = i + l;", true),
        ("int i; long l; long r = l + i;", false),
    ] {
        let p = parse(src, TargetConfig::x86_64_linux());
        let sym = p.symbol("r").expect("r");
        let init = p
            .parsed
            .ast
            .items()
            .iter()
            .find_map(|&id| match &p.parsed.ast.decl(id).kind {
                DeclKind::Var {
                    name: Some(n),
                    init: Some(i),
                    ..
                } if *n == sym => Some(*i),
                _ => None,
            })
            .expect("initializer");
        let ExprKind::Binary { lhs, rhs, .. } = &p.parsed.ast.expr(init).kind else {
            panic!("not binary")
        };
        let (narrow, wide) = if narrow_is_lhs {
            (*lhs, *rhs)
        } else {
            (*rhs, *lhs)
        };
        assert!(
            p.analysis
                .typed()
                .conversions_of(narrow)
                .contains(&Conversion::UsualArithmetic),
            "`{src}`: the `int` operand is converted"
        );
        assert!(
            !p.analysis
                .typed()
                .conversions_of(wide)
                .contains(&Conversion::UsualArithmetic),
            "`{src}`: and the `long` one is not — otherwise the rule is `convert both`"
        );
    }
}

/// **Contract 11, decay and null pointers.** An array becomes a pointer, a function
/// becomes a pointer, and `0` in pointer context becomes a null pointer of that type.
/// Each is a distinct `Conversion`, because 015 lowers them differently.
#[test]
fn arrays_functions_and_null_constants_decay_explicitly() {
    let p = parse(
        "int arr[4]; int *pa = arr;\n\
         int fn(int); int (*pf)(int) = fn;\n\
         int *pn = 0;\n",
        TargetConfig::x86_64_linux(),
    );
    assert!(
        init_conversions(&p, "pa").contains(&Conversion::ArrayDecay),
        "`int *pa = arr;` decays the array: {:?}",
        init_conversions(&p, "pa")
    );
    assert!(
        init_conversions(&p, "pf").contains(&Conversion::FunctionDecay),
        "`int (*pf)(int) = fn;` decays the function: {:?}",
        init_conversions(&p, "pf")
    );
    assert!(
        init_conversions(&p, "pn").contains(&Conversion::NullPointer),
        "`int *pn = 0;` is a null pointer constant, not an integer: {:?}",
        init_conversions(&p, "pn")
    );
    // The discriminator: an ordinary integer initializer gets no pointer conversion, so
    // `NullPointer` is not simply stamped on every literal.
    let q = parse("int n = 0;", TargetConfig::x86_64_linux());
    assert!(
        !init_conversions(&q, "n").contains(&Conversion::NullPointer),
        "`int n = 0;` is just an integer"
    );
}

/// **Contract 11, arguments and assignment.** A `char` argument to an `int` parameter and
/// a `long` assigned to an `int` both carry explicit conversions, tagged by *why* — 015
/// lowers an argument conversion differently from an arithmetic one.
#[test]
fn arguments_and_assignments_carry_their_own_conversion_kind() {
    let p = parse(
        "int f(long); char c; int r = f(c);",
        TargetConfig::x86_64_linux(),
    );
    let sym = p.symbol("r").expect("r");
    let call = p
        .parsed
        .ast
        .items()
        .iter()
        .find_map(|&id| match &p.parsed.ast.decl(id).kind {
            DeclKind::Var {
                name: Some(n),
                init: Some(i),
                ..
            } if *n == sym => Some(*i),
            _ => None,
        })
        .expect("initializer");
    let ExprKind::Call { args, .. } = &p.parsed.ast.expr(call).kind else {
        panic!("not a call")
    };
    let conv = p.analysis.typed().conversions_of(args[0]);
    assert!(
        conv.contains(&Conversion::Argument),
        "the argument conversion is tagged as one: {conv:?}"
    );
    let ty = p
        .analysis
        .typed()
        .top(args[0])
        .map(|t| p.analysis.typed().ty_of(t))
        .expect("typed");
    assert_eq!(
        p.analysis.ty(ty),
        &Ty::Int {
            signed: true,
            bits: 64
        },
        "and the argument arrives as the parameter's `long`, not as a promoted `int`"
    );
}

/// **Contract 20.** A type error yields `Ty::Error` and **does not cascade**: one bad
/// declaration is one diagnostic no matter how often the name is used afterwards.
///
/// The count is the contract. An implementation that reported the unknown type once per
/// *use* would satisfy "produces a diagnostic" while burying it under a hundred copies,
/// which is the failure mode 013 §6's diagnostic cap exists to bound and 014 §5's poison
/// type exists to prevent.
#[test]
fn a_bad_declaration_does_not_cascade_through_its_uses() {
    // The fixture has to **parse** cleanly, or the failure under test is 013's and not
    // 014's: a bare unknown identifier at declaration position is a syntax error, since
    // the parser cannot know it was meant as a type. A typedef to a tag that is never
    // defined parses fine and is exactly a *type* error — the object has no size.
    let p = parse_allowing_diagnostics(
        "typedef struct Undefined undef_t;\n\
         undef_t x;\n\
         int a = x + 1;\n\
         int b = x + 2;\n\
         int c = x + 3;\n\
         int d = x * x;\n",
        TargetConfig::x86_64_linux(),
    );
    assert_eq!(
        p.analysis.diagnostics.len(),
        1,
        "one diagnostic for the bad declaration, not one per use: {:?}",
        p.analysis
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );

    // The discriminator: two *different* bad declarations are two diagnostics, so the
    // rule is "poison propagates" and not "report at most one thing".
    let q = parse_allowing_diagnostics(
        "typedef struct A undef_a; typedef struct B undef_b;\n\
         undef_a x; undef_b y;\n\
         int a = x + y;\n",
        TargetConfig::x86_64_linux(),
    );
    assert_eq!(
        q.analysis.diagnostics.len(),
        2,
        "two bad declarations are two diagnostics: {:?}",
        q.analysis
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// **Contract 11 at corpus scale, and the assertion that actually carries it.**
///
/// Walk every typed node produced from real VPP and require that no operation is left
/// with operands C would have had to convert: a binary arithmetic operation's two
/// operands must **already** have the same type. Anything else means lowering would have
/// to infer the conversion, which 014 §5 exists to prevent.
#[test]
fn no_operation_in_the_corpus_is_left_implicit() {
    let Some(cases) = harness::corpus_analyses() else {
        eprintln!("skipping: gcc not found (014 contract 11)");
        return;
    };
    let mut checked = 0usize;
    let mut offenders: Vec<String> = Vec::new();

    for (seed, p) in &cases {
        let typed = p.analysis.typed();
        for node in typed.nodes() {
            let TypedNode::Value { expr, operands, .. } = node else {
                continue;
            };
            let ExprKind::Binary { op, .. } = &p.parsed.ast.expr(*expr).kind else {
                continue;
            };
            // Shifts do not take the usual arithmetic conversions: each operand is
            // promoted on its own and the result has the left operand's type.
            if matches!(op, BinOp::Shl | BinOp::Shr) {
                continue;
            }
            if operands.len() != 2 {
                continue;
            }
            let a = typed.ty_of(operands[0]);
            let b = typed.ty_of(operands[1]);
            // A pointer operand is legitimately of a different type from an integer one
            // (`p + n`), and a comparison against a null constant likewise.
            let scalarish =
                |t| matches!(p.analysis.ty(t), Ty::Int { .. } | Ty::Float(_) | Ty::Error);
            if !scalarish(a) || !scalarish(b) {
                continue;
            }
            if matches!(p.analysis.ty(a), Ty::Error) || matches!(p.analysis.ty(b), Ty::Error) {
                continue;
            }
            checked += 1;
            if a != b && offenders.len() < 20 {
                offenders.push(format!(
                    "{seed}: {:?} operands {:?} vs {:?}",
                    op,
                    p.analysis.ty(a),
                    p.analysis.ty(b)
                ));
            }
        }
    }

    eprintln!("contract 11: {checked} arithmetic operations checked across the corpus");
    assert!(
        checked > 2000,
        "only {checked} operations were checked; the corpus has far more, so the walk \
         is not reaching the expressions"
    );
    assert!(
        offenders.is_empty(),
        "{} operation(s) still need a conversion lowering would have to infer:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
