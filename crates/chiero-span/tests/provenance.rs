//! Macro provenance: the queries that make chiero different from a coverage-only tool.
//!
//! Covers **010 contracts 3–10**. The fixture is the worked `vec_add1` example from
//! 010 §3.2, because it is the case the entire project is justified by: a change to a
//! macro body in a header, whose generated code gcov attributes only to the `.c` line
//! where the macro was used.

use chiero_span::{BytePos, ExpnCtx, ExpnKind, MacroId, SourceMap, Span, TokenOrigin};
use std::alloc::{GlobalAlloc, Layout, System};

/// A counting allocator, so 010 contract 9 ("`expansion_loc` never allocates") is a
/// real measurement rather than a comment. Installed for this test binary only.
///
/// The counter is **thread-local**. `cargo test` runs tests in parallel on one process,
/// so a process-global counter measures every other test's allocations too and the
/// assertion becomes flaky nonsense — which is exactly what a first, global version of
/// this did.
struct Counting;

thread_local! {
    static ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn bump() {
    // `try_with`: the TLS may be destroyed during thread teardown, and allocating then
    // must not panic.
    let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        bump();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn alloc_count() -> usize {
    ALLOCS.try_with(|c| c.get()).unwrap_or(0)
}

/// Builds the 010 §3.2 fixture:
///
/// ```text
/// vec.h:118  #define vec_add1_ha(V,E,H,A) (vec_resize_ha(V,1,H,A), ...)
/// vec.h:120  #define vec_add1(V,E) vec_add1_ha (V, E, 0, 0)
/// ip4_forward.c:900   vec_add1 (adj_list, ai);
/// ```
///
/// Returns the map plus the ids needed by the assertions.
struct Fixture {
    sm: SourceMap,
    vec_add1: MacroId,
    vec_add1_ha: MacroId,
    /// Span of the `vec_resize_ha` token, which is copied from `vec_add1_ha`'s body
    /// and reached only through two nested expansions.
    resize_tok: Span,
    /// Span of the `ai` token, substituted from the caller's argument.
    ai_tok: Span,
    /// The outer expansion, i.e. `vec_add1(...)` at ip4_forward.c:900.
    outer: ExpnCtx,
    /// The inner expansion, i.e. `vec_add1_ha(...)` from inside vec_add1's body.
    inner: ExpnCtx,
}

fn fixture() -> Fixture {
    // Line-exact so the assertions can name real line numbers.
    let vec_h: String = {
        let mut s = String::new();
        for _ in 1..118 {
            s.push('\n');
        }
        s.push_str("#define vec_add1_ha(V,E,H,A) (vec_resize_ha(V,1,H,A))\n"); // 118
        s.push('\n'); // 119
        s.push_str("#define vec_add1(V,E) vec_add1_ha (V, E, 0, 0)\n"); // 120
        s
    };
    let c_src: String = {
        let mut s = String::new();
        for _ in 1..900 {
            s.push('\n');
        }
        s.push_str("  vec_add1 (adj_list, ai);\n"); // 900
        s
    };

    let mut sm = SourceMap::new();
    let hf = sm.add_file("vppinfra/vec.h", vec_h.clone());
    let cf = sm.add_file("ip4_forward.c", c_src.clone());
    let hbase = sm.file(hf).start_pos.0;
    let cbase = sm.file(cf).start_pos.0;

    let at = |base: u32, hay: &str, needle: &str, nth: usize| -> Span {
        let off = hay.match_indices(needle).nth(nth).expect("needle").0 as u32;
        Span::new(
            BytePos(base + off),
            BytePos(base + off + needle.len() as u32),
            ExpnCtx::ROOT,
        )
    };

    // Macro definitions. `body_extent` is what discriminates a token copied from the
    // body from one substituted from an argument (010 §2.2).
    let ha_body = at(hbase, &vec_h, "(vec_resize_ha(V,1,H,A))", 0);
    let vec_add1_ha = sm.add_macro("vec_add1_ha", at(hbase, &vec_h, "vec_add1_ha", 0), ha_body);
    let a1_body = at(hbase, &vec_h, "vec_add1_ha (V, E, 0, 0)", 0);
    let vec_add1 = sm.add_macro(
        "vec_add1",
        at(hbase, &vec_h, "#define vec_add1(", 0),
        a1_body,
    );

    // The outer expansion: `vec_add1 (adj_list, ai)` written at ip4_forward.c:900.
    let call_site = at(cbase, &c_src, "vec_add1", 0);
    let outer = sm.add_expansion(
        ExpnCtx::ROOT,
        Some(vec_add1),
        call_site,
        at(cbase, &c_src, "vec_add1 (adj_list, ai)", 0),
        vec![at(cbase, &c_src, "adj_list", 0), at(cbase, &c_src, "ai", 0)],
        ExpnKind::FunctionLike,
    );

    // The inner expansion: `vec_add1_ha (...)` written inside vec_add1's body, so its
    // call site carries the *outer* ctx — that is the nesting.
    let inner_site = Span::new(
        at(hbase, &vec_h, "vec_add1_ha (V, E, 0, 0)", 0).lo,
        BytePos(at(hbase, &vec_h, "vec_add1_ha (V, E, 0, 0)", 0).lo.0 + 11),
        outer,
    );
    let inner = sm.add_expansion(
        outer,
        Some(vec_add1_ha),
        inner_site,
        inner_site,
        vec![],
        ExpnKind::FunctionLike,
    );

    // A token copied from vec_add1_ha's *body*, produced by the inner expansion.
    let rt = at(hbase, &vec_h, "vec_resize_ha", 0);
    let resize_tok = Span::new(rt.lo, rt.hi, inner);

    // A token substituted from the *caller's* argument, produced by the outer
    // expansion: its bytes point at ip4_forward.c, not at the macro body.
    let a = at(cbase, &c_src, "ai", 0);
    let ai_tok = Span::new(a.lo, a.hi, outer);

    Fixture {
        sm,
        vec_add1,
        vec_add1_ha,
        resize_tok,
        ai_tok,
        outer,
        inner,
    }
}

/// 010 contract 3: `spelling_loc` of the `vec_resize_ha` token is vec.h:118;
/// `expansion_loc` is ip4_forward.c:900.
///
/// This single assertion is the project's thesis. gcov records only the second.
#[test]
fn spelling_and_expansion_locations_differ() {
    let f = fixture();
    let sp = f.sm.spelling_loc(f.resize_tok).unwrap();
    assert_eq!(
        f.sm.file(sp.file).path().to_str().unwrap(),
        "vppinfra/vec.h"
    );
    assert_eq!(sp.line, 118);

    let ex = f.sm.expansion_loc(f.resize_tok).unwrap();
    assert_eq!(f.sm.file(ex.file).path().to_str().unwrap(), "ip4_forward.c");
    assert_eq!(ex.line, 900);
}

/// A ROOT span's two locations coincide — 010 contract 2, second half.
#[test]
fn root_spans_have_identical_locations() {
    let f = fixture();
    let root = Span::new(f.resize_tok.lo, f.resize_tok.hi, ExpnCtx::ROOT);
    assert_eq!(f.sm.spelling_loc(root), f.sm.expansion_loc(root));
}

/// 010 contract 4: the backtrace has length 2 and is ordered outermost-first.
#[test]
fn backtrace_is_outermost_first() {
    let f = fixture();
    let bt = f.sm.expansion_backtrace(f.resize_tok);
    assert_eq!(bt.len(), 2, "two nested expansions");
    assert_eq!(bt[0].macro_id, Some(f.vec_add1), "outermost first");
    assert_eq!(bt[1].macro_id, Some(f.vec_add1_ha));
    // The outermost frame's call site is the .c line a human would look at.
    assert_eq!(f.sm.lookup_loc(bt[0].call_site.lo).unwrap().line, 900);
}

/// 010 contract 5: origin distinguishes a token copied from a macro body from one
/// substituted from an argument. Editing a body affects every call site; an argument
/// token was written by the caller.
#[test]
fn origin_distinguishes_body_from_argument() {
    let f = fixture();
    assert_eq!(
        f.sm.origin(f.resize_tok),
        TokenOrigin::MacroBody(f.vec_add1_ha)
    );
    assert_eq!(
        f.sm.origin(f.ai_tok),
        TokenOrigin::MacroArg {
            expn: f.outer,
            arg_index: 1
        }
    );

    let verbatim = Span::new(f.ai_tok.lo, f.ai_tok.hi, ExpnCtx::ROOT);
    assert!(matches!(f.sm.origin(verbatim), TokenOrigin::Verbatim(_)));
    assert_eq!(f.sm.origin(Span::DUMMY), TokenOrigin::Synthesized);
}

/// 010 contract 6: `involves_macro` is true for a macro the source never names.
/// `ip4_forward.c` writes only `vec_add1`, yet the token came through `vec_add1_ha`.
#[test]
fn involves_macro_sees_through_nesting() {
    let f = fixture();
    assert!(f.sm.involves_macro(f.resize_tok, f.vec_add1_ha));
    assert!(f.sm.involves_macro(f.resize_tok, f.vec_add1));
    let other = MacroId(999);
    assert!(!f.sm.involves_macro(f.resize_tok, other));
    assert!(!f.sm.involves_macro(Span::DUMMY, f.vec_add1));
}

/// 010 contract 7: `expansion_sites` includes the site transitively — the reverse
/// index is what change-impact analysis is built on (031 §3.2).
#[test]
fn expansion_sites_are_transitive() {
    let f = fixture();
    let direct: Vec<_> = f.sm.expansion_sites(f.vec_add1).collect();
    assert!(direct.contains(&f.outer));

    let transitive: Vec<_> = f.sm.expansion_sites(f.vec_add1_ha).collect();
    assert!(
        transitive.contains(&f.inner),
        "vec_add1_ha is expanded from inside vec_add1's body, and ip4_forward.c \
         never names it — this is the case a coverage-only tool cannot see"
    );
}

/// 010 contract 8: `#define A B` / `#define B C` / `A` gives a depth-2 chain.
#[test]
fn object_like_chain_has_depth_two() {
    let mut sm = SourceMap::new();
    let src = "#define A B\n#define B C\nA\n";
    let fid = sm.add_file("t.c", src);
    let base = sm.file(fid).start_pos.0;
    let at = |needle: &str, nth: usize| {
        let off = src.match_indices(needle).nth(nth).unwrap().0 as u32;
        Span::new(
            BytePos(base + off),
            BytePos(base + off + needle.len() as u32),
            ExpnCtx::ROOT,
        )
    };
    let a = sm.add_macro("A", at("A", 0), at("B", 0));
    let b = sm.add_macro("B", at("B", 1), at("C", 0));

    let use_a = at("A", 1);
    let e1 = sm.add_expansion(
        ExpnCtx::ROOT,
        Some(a),
        use_a,
        use_a,
        vec![],
        ExpnKind::ObjectLike,
    );
    let b_in_a = Span::new(at("B", 0).lo, at("B", 0).hi, e1);
    let e2 = sm.add_expansion(e1, Some(b), b_in_a, b_in_a, vec![], ExpnKind::ObjectLike);

    let c_tok = Span::new(at("C", 0).lo, at("C", 0).hi, e2);
    assert_eq!(sm.expansion_backtrace(c_tok).len(), 2);
    assert_eq!(
        sm.expansion_loc(c_tok).unwrap().line,
        3,
        "attributed to the use of A"
    );
}

/// 010 contract 9: `expansion_loc` never allocates. It is the most-called query in the
/// coverage vertical, so this is a real performance contract, not a style preference.
#[test]
fn expansion_loc_does_not_allocate() {
    let f = fixture();

    // Sanity-check that the counting allocator is actually installed; otherwise the
    // real assertion below would pass vacuously.
    let probe_before = alloc_count();
    let probe: Vec<u8> = Vec::with_capacity(64);
    assert!(
        alloc_count() > probe_before,
        "counting allocator is not installed; this test would be vacuous"
    );
    drop(probe);

    let before = alloc_count();
    let loc = f.sm.expansion_loc(f.resize_tok);
    let after = alloc_count();
    assert!(loc.is_some());
    assert_eq!(
        after, before,
        "expansion_loc must not allocate (010 contract 9)"
    );
}

/// Provenance queries must terminate even on a malformed cycle rather than hanging.
/// A cycle cannot arise from correct preprocessing, but a bug upstream must surface
/// as a wrong answer, not as an infinite loop inside the most-called query.
#[test]
fn cyclic_parent_chain_terminates() {
    let mut sm = SourceMap::new();
    let fid = sm.add_file("t.c", "x");
    let base = sm.file(fid).start_pos;
    let sp = Span::new(base, BytePos(base.0 + 1), ExpnCtx::ROOT);
    let m = sm.add_macro("M", sp, sp);
    let e = sm.add_expansion(ExpnCtx::ROOT, Some(m), sp, sp, vec![], ExpnKind::ObjectLike);
    sm.force_parent_for_test(e, e); // self-parent: malformed

    let tok = Span::new(sp.lo, sp.hi, e);
    let _ = sm.expansion_backtrace(tok);
    let _ = sm.expansion_loc(tok);
}
