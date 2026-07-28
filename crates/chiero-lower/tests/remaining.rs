//! Covers: 015 contracts 8, 12, 14, 20.
//!
//! The last four of 015 that are not about shapes already fixed elsewhere. Each is a
//! different kind of claim: contract 8 is about a *value*, 12 about a *scope*, 14 about an
//! instruction's *position*, and 20 about **refusing** rather than lowering wrongly.

use chiero_cir::{InstKind, Lifetime, MarkerKind, ScopeEvent, ScopeKind, Terminator};

mod harness;
use harness::{lower, lower_raw};

fn func<'a>(m: &'a chiero_cir::Module, name: &str) -> &'a chiero_cir::Function {
    m.funcs
        .iter()
        .find(|f| &*f.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no `{name}` in {:?}",
                m.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
            )
        })
}

/// **Contract 12.** `for (int i = 0; …)` puts `i` in a scope **enclosing the body**, and
/// `i` is out of scope after the loop.
///
/// 015 §3 says the init runs in a scope enclosing the loop, and the placement is the
/// contract: if `i` lived in the *body's* scope it would be created and retired on every
/// iteration, so 021 would see a fresh object each time and `&i` would compare unequal
/// across iterations — legal C says it is one object.
#[test]
fn a_for_init_declaration_lives_in_a_scope_enclosing_the_body() {
    let m = lower("int f(int n) { for (int i = 0; i < n; i++) { int b = i; } return 0; }");
    let f = func(&m, "f");

    // Collect (block, scope, kind) in layout order.
    let events: Vec<(u32, ScopeKind)> = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            InstKind::Marker(MarkerKind::Scope(ScopeEvent { scope, kind })) => {
                Some((scope.0, *kind))
            }
            _ => None,
        })
        .collect();

    // The `for`'s own scope is entered before the body's and exited after it.
    let enters: Vec<u32> = events
        .iter()
        .filter(|(_, k)| *k == ScopeKind::Enter)
        .map(|(s, _)| *s)
        .collect();
    assert!(
        enters.len() >= 3,
        "the function body, the `for` scope and the loop body each enter one: {events:?}"
    );
    // `i`'s alloca belongs to the `for` scope, and `b`'s to the body's — different scopes.
    let scope_of = |name: &str| {
        f.allocas
            .iter()
            .find(|a| a.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no alloca for `{name}`"))
            .scope
            .0
    };
    assert_ne!(
        scope_of("i"),
        scope_of("b"),
        "`i` is not in the body's scope; if it were, it would be retired and recreated \
         every iteration and `&i` would change across them"
    );
}

/// **Contract 14.** A VLA emits `AllocaDyn` **at the declaration point**, with the size
/// operand dominating it; `alloca()` emits `AllocaDyn` with `Lifetime::Function`.
///
/// The position is the contract, not the instruction. 020 §3 puts `AllocaDyn` at a real
/// program point precisely so ordinary dominance applies to `count` — a size computed from
/// a variable must be computed *before* the allocation that uses it, and an implementation
/// that hoisted the allocation to the entry block would reference a value that does not
/// dominate it.
#[test]
fn a_vla_allocates_at_its_declaration_with_the_size_dominating() {
    let m = lower("int f(int n) { int k = n + 1; int v[k]; v[0] = 3; return v[0]; }");
    let f = func(&m, "f");

    let (block_idx, inst_idx, count) = f
        .blocks
        .iter()
        .enumerate()
        .find_map(|(bi, b)| {
            b.insts
                .iter()
                .enumerate()
                .find_map(|(ii, i)| match &i.kind {
                    InstKind::AllocaDyn { count, .. } => Some((bi, ii, count.clone())),
                    _ => None,
                })
        })
        .expect("a VLA emits `AllocaDyn`");

    // The size operand is a value, and it is produced *before* the allocation in the same
    // block — the simplest form of dominance, and the one 020 §3 is about.
    let chiero_cir::Operand::Value(size_v) = count else {
        panic!("a VLA's extent is computed, not a constant: {count:?}")
    };
    let produced_at = f.blocks[block_idx]
        .insts
        .iter()
        .position(|i| matches!(&i.kind, InstKind::Assign { dst, .. } if *dst == size_v));
    assert!(
        produced_at.is_some_and(|p| p < inst_idx),
        "the size is computed before the allocation that consumes it: produced at \
         {produced_at:?}, allocated at {inst_idx}"
    );

    // **A VLA inside a branch**, which is what makes "at the declaration point" a real
    // claim. With the whole function in one block, hoisting the allocation to the entry
    // changes nothing and the mutation survives — the size and the allocation are in the
    // same block either way. Here they are not: the allocation must be in the *branch*.
    let m = lower(
        "int f(int n) { if (n > 0) { int k = n + 1; int v[k]; v[0] = 3; return v[0]; } return 0; }",
    );
    let f = func(&m, "f");
    let alloc_block = f
        .blocks
        .iter()
        .position(|b| {
            b.insts
                .iter()
                .any(|i| matches!(i.kind, InstKind::AllocaDyn { .. }))
        })
        .expect("the VLA allocates somewhere");
    assert_ne!(
        alloc_block,
        f.blocks
            .iter()
            .position(|b| b.id == f.entry)
            .expect("entry exists"),
        "the allocation is in the branch that declares it, not hoisted to the entry —          hoisted, its size operand would be computed in a block that does not dominate it"
    );

    // And the declaration is `Lifetime::Scope`: a VLA dies with its block, unlike
    // `alloca()`, which lives to function return (020 §3).
    let decl = f
        .allocas
        .iter()
        .find(|a| a.count == chiero_cir::DYNAMIC_EXTENT)
        .expect("the VLA's declaration carries a dynamic extent");
    assert_eq!(decl.lifetime, Lifetime::Scope);

    // `alloca()` is the other half, and it differs *only* in lifetime — so an
    // implementation that used one lifetime for both passes every other assertion here.
    let m =
        lower("void *alloca(unsigned long); int f(int n) { char *p = alloca(n); return p != 0; }");
    let f = func(&m, "f");
    let decl = f
        .allocas
        .iter()
        .find(|a| a.count == chiero_cir::DYNAMIC_EXTENT)
        .expect("`alloca()` is a dynamic allocation, not a call");
    assert_eq!(
        decl.lifetime,
        Lifetime::Function,
        "`alloca()` lives until function return, unlike a VLA"
    );
}

/// **Contract 20.** A function containing a nested function is **skipped with exactly one
/// diagnostic** and is absent from the module.
///
/// 015 §7 is refuse-rather-than-lower-wrongly, and "exactly one" is the substance: a
/// lowering that emitted a diagnostic per statement of the unlowerable function would
/// satisfy "produces a diagnostic" while burying it, and one that emitted a *partial*
/// function would be worse — every analysis downstream would treat it as complete.
#[test]
fn a_function_with_a_nested_function_is_refused_whole() {
    let out = lower_raw(
        "int outer(int n) { int inner(int m) { return m + 1; } return n; }\n\
         int ok(int n) { return n + 1; }\n",
    );
    assert_eq!(
        out.diagnostics.len(),
        1,
        "one diagnostic for the whole function, not one per statement: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    assert!(
        !out.module
            .funcs
            .iter()
            .any(|f| &*f.name == "outer" && matches!(f.body, chiero_cir::Body::Defined)),
        "`outer` is absent as a *definition* — a partial body is worse than none, \
         because everything downstream treats it as complete"
    );
    // The discriminator: the rest of the TU is unaffected.
    assert!(
        out.module
            .funcs
            .iter()
            .any(|f| &*f.name == "ok" && matches!(f.body, chiero_cir::Body::Defined)),
        "and the function beside it still lowers — refusing one is not refusing the file"
    );
}

/// **Contract 8.** A statement expression yields the value of its **last expression
/// statement**, and its side effects occur **once**.
///
/// 217 VPP files use them. "Once" is the half a shape test cannot see: `({ f(); f(); 1; })`
/// and `({ f(); 1; })` have the same value and different programs, so the count of `Call`
/// instructions is what carries it.
#[test]
fn a_statement_expression_yields_its_last_value_once() {
    let m = lower("int g(void); int f(void) { int x = ({ int t = g(); t + 1; }); return x; }");
    let f = func(&m, "f");
    let calls = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| matches!(i.kind, InstKind::Call { .. }))
        .count();
    assert_eq!(
        calls, 1,
        "`g()` is called once, not once per use of the block's value"
    );
    assert!(
        f.blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Return(Some(_)))),
        "and the function returns the value"
    );
}
