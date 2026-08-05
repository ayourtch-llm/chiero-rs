//! REVIEW PROBE (uncommitted): does a shared `ConstEvaluator` answer every expression
//! the same way — value AND diagnostics — as a fresh `const_eval`, regardless of how
//! many evaluations preceded it and in what order?

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
"#;

fn parsed() -> ParsedTu {
    let tu = preprocess_str("probe.c", SRC, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    parsed
}

fn msgs(d: &[SemaDiagnostic]) -> Vec<String> {
    d.iter().map(|x| x.message.clone()).collect()
}

#[test]
fn shared_evaluator_matches_fresh_const_eval_in_any_order() {
    let parsed = parsed();
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
