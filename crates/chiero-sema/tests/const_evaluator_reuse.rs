//! **One `ConstEvaluator`, reused, must answer exactly what a fresh `const_eval` would.**
//!
//! bd5fc12 made lowering prepare the translation unit once instead of once per constant, which
//! turned a 164-second translation unit into a 1-second one. The price is that a `Cx` now
//! outlives the expression it was built for, so anything it accumulates becomes a channel
//! between evaluations — and `const_eval` is public API whose callers are entitled to the same
//! answer whether they asked first or thousandth.
//!
//! # The sweep
//!
//! `shared_evaluator_matches_fresh_const_eval_in_any_order` evaluates every expression node of a
//! deliberately awkward translation unit — enum overflow, dead `1/0` arms, address constants,
//! `sizeof` of incomplete and expression and `__func__` operands, a struct defined inside a
//! `sizeof`, a statement-expression array bound, bit-field widths — twice forward and once in
//! reverse, against a fresh `const_eval` per expression as ground truth. Values *and*
//! diagnostics.
//!
//! # The one that failed
//!
//! `repeated_eval_must_not_swallow_diagnostics`. `Cx::unknown_names` deduplicates "was not
//! declared" once per name for the lifetime of the context. Rebuilt per call it reported every
//! call; shared, it reports the first and is then silent — including for a *different*
//! expression in a *different* function that happens to name the same identifier. The value was
//! never wrong; the report was, and a report that depends on how many constants the caller
//! folded earlier is not a report.
//!
//! Found by the adversarial review of bd5fc12, which is what that step of the protocol is for.

use chiero_parse::{ParsedTu, ScopedTypedefs, parse_tu};
use chiero_pp::{Config, preprocess_str};
use chiero_sema::{ConstEvaluator, ConstVal, SemaDiagnostic, SymbolText, TargetConfig, const_eval};

struct Names<'a>(&'a ParsedTu);
impl SymbolText for Names<'_> {
    fn text(&self, sym: chiero_span::Symbol) -> Option<&str> {
        self.0.text(sym)
    }
}

const SRC: &str = r#"
enum Big { HUGE = 2147483647, WRAP = 2147483647 + 1 };
struct S { int a; char b[13]; struct S *next; };
static int arr[10];
static struct S s;
enum { N = sizeof(arr) / sizeof(arr[0]) };
static int *p1 = &arr[3];
static char *p2 = (char*)&s + 4;
static int f0(int x) { switch(x) { case 1+2: return 0; case (int)1.5: return 1; } return sizeof(struct S); }
static int f1(void) { int a[N]; return sizeof(a) + HUGE; }
static int f2(void) { return 1 ? 7 : 1/0; }
static int f3(void) { return __builtin_constant_p(N) + (int)__builtin_offsetof(struct S, b); }
static int f4(void) { return sizeof arr + sizeof(enum Big); }
static int f5(int q) { return q + HUGE + 1; }
static int f6(void) { return (unsigned char)~0; }
static int f7(void) { return sizeof("hello") + 'x' + sizeof(long long); }
static int f8(int x) { switch(x) { case FOO: return 1; default: return 0; } }
static int f9(void) { return sizeof(__func__); }
static int f10(void) { return sizeof(struct Local { int q[3]; char c; }); }
struct Incomplete;
static int f11(void) { return sizeof(struct Incomplete); }
_Static_assert(N == 10, "N is ten");
static int f12(void) { struct { int w : 3 + 1; } bf; return sizeof(bf); }
static int f13(void) { int v[({ 3; })]; return sizeof(v); }
static int f14(void) { return __builtin_constant_p(arr) + __builtin_constant_p(&arr[2]); }
static int f15(void) { return (int)(long)&s.b[5] ; }
static int __attribute__((aligned(1 << 4))) f16(void) { return WRAP; }
"#;

fn parsed(src: &str) -> ParsedTu {
    let tu = preprocess_str("probe.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    parsed
}

fn msgs(d: &[SemaDiagnostic]) -> Vec<String> {
    d.iter().map(|x| x.message.clone()).collect()
}

/// The decisive experiment over ordinary constant expressions.
#[test]
fn shared_evaluator_matches_fresh_const_eval_in_any_order() {
    let parsed = parsed(SRC);
    let ast = &parsed.ast;
    let target = TargetConfig::x86_64_linux();
    let n = ast.exprs().len();

    // Ground truth: a fresh const_eval per expression, exactly the old code path.
    let mut fresh: Vec<(Option<ConstVal>, Vec<String>)> = Vec::with_capacity(n);
    for i in 0..n {
        let e = chiero_ast::ExprId(i as u32);
        let mut d = Vec::new();
        let v = const_eval(ast, e, &Names(&parsed), &target, &mut d);
        fresh.push((v, msgs(&d)));
    }

    // One shared evaluator, forward order, each expression evaluated twice in a row.
    let names = Names(&parsed);
    let mut ev = ConstEvaluator::new(ast, &names, &target);
    // The index is the `ExprId` as well as the position in `fresh`, so iterating the slice
    // would still need it.
    #[allow(clippy::needless_range_loop)]
    for i in 0..n {
        let e = chiero_ast::ExprId(i as u32);
        for pass in 0..2 {
            let mut d = Vec::new();
            let v = ev.eval(e, &mut d);
            assert_eq!(
                (v, msgs(&d)),
                fresh[i].clone(),
                "expr {i} ({:?}) diverged from fresh const_eval on shared-evaluator \
                 forward pass {pass}",
                ast.expr(e).kind
            );
        }
    }

    // A second shared evaluator, reverse order.
    let names = Names(&parsed);
    let mut ev = ConstEvaluator::new(ast, &names, &target);
    for i in (0..n).rev() {
        let e = chiero_ast::ExprId(i as u32);
        let mut d = Vec::new();
        let v = ev.eval(e, &mut d);
        assert_eq!(
            (v, msgs(&d)),
            fresh[i].clone(),
            "expr {i} ({:?}) diverged from fresh const_eval on shared-evaluator \
             reverse pass",
            ast.expr(e).kind
        );
    }
}

/// **The second `eval` of one expression must report what the first did.**
///
/// `sizeof(__typeof__(loc))` over a function local: after preparation the local is out of
/// scope, so typing it reports "`loc` was not declared" — once per name per context, via
/// `Cx::unknown_names`. Expected on every call, which is what a fresh `const_eval` gives:
/// ["`loc` was not declared", "`sizeof` applied to an incomplete type"]. Observed on the
/// second call before the fix: the second message alone.
#[test]
fn repeated_eval_must_not_swallow_diagnostics() {
    let src = r#"
static int t1(void) { int loc[4]; return sizeof(__typeof__(loc)); }
static int t2(void) { int loc[8]; return sizeof(__typeof__(loc)); }
"#;
    let parsed = parsed(src);
    let ast = &parsed.ast;
    let target = TargetConfig::x86_64_linux();

    // Find the two sizeof(__typeof__(loc)) expressions.
    let sizeofs: Vec<chiero_ast::ExprId> = (0..ast.exprs().len() as u32)
        .map(chiero_ast::ExprId)
        .filter(|&e| matches!(ast.expr(e).kind, chiero_ast::ExprKind::SizeofType(_)))
        .collect();
    assert_eq!(sizeofs.len(), 2);

    // Ground truth: fresh const_eval, once per expression — deterministic per call.
    let fresh: Vec<Vec<String>> = sizeofs
        .iter()
        .map(|&e| {
            let mut d = Vec::new();
            let _ = const_eval(ast, e, &Names(&parsed), &target, &mut d);
            msgs(&d)
        })
        .collect();

    let names = Names(&parsed);
    let mut ev = ConstEvaluator::new(ast, &names, &target);

    // First call agrees with fresh...
    let mut d = Vec::new();
    let _ = ev.eval(sizeofs[0], &mut d);
    assert_eq!(msgs(&d), fresh[0], "first eval already diverges");

    // ...the second call on the SAME expression must too, and does not.
    let mut d = Vec::new();
    let _ = ev.eval(sizeofs[0], &mut d);
    assert_eq!(
        msgs(&d),
        fresh[0],
        "second eval of the same expression dropped a diagnostic the first one reported"
    );

    // And a different expression in a different function is silenced by the first one's
    // history as well.
    let mut d = Vec::new();
    let _ = ev.eval(sizeofs[1], &mut d);
    assert_eq!(
        msgs(&d),
        fresh[1],
        "an eval of one expression silenced a diagnostic belonging to another"
    );
}
