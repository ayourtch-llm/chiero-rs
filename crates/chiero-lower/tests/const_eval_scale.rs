//! **Lowering one function costs more the more *other* functions the file has.**
//!
//! `chiero_lower::Lowerer::const_of` calls `chiero_sema::const_eval` for every constant
//! expression it meets — every integer literal in every body. `const_eval` builds a
//! throwaway `Cx` and then walks `ast.items()` in full before evaluating the one
//! expression it was asked about, because an address constant is *about* a declared
//! object. So a translation unit with F functions and one literal each pays F² item
//! declarations, and the measured cost is exactly that shape: 1.2 ms per function at
//! F=250, 3.2 ms at F=1000, flat within a run.
//!
//! That is why VPP became unanalysable the moment builtins started lowering (9f7e575).
//! gcc's x86 headers hold 5973 `always_inline` wrappers; until then every one was
//! *discarded*, so nothing paid for them. One translation unit now takes **673 seconds**
//! against ~1 s before — a 1871-TU sweep is two weeks, so the corpus gate is unrunnable.
//!
//! # Why this is counted, not timed
//!
//! A wall-clock assertion in CI is a flake generator, and it would not say *what* is
//! quadratic. `SymbolText::text` is the one interface both the item walk and the
//! expression walk must go through, so counting calls to it measures work in units that
//! do not vary with machine load — and the ratio between two sizes is the asymptotics
//! directly.
//!
//! # What the fix owes
//!
//! `const_eval` must stay callable standalone: it is public API, and a `.cir` fixture or
//! any caller with no `Analysis` still needs it to resolve `sizeof(int)` and address
//! constants on its own. The defect is not that it prepares a context — it is that
//! lowering pays for that preparation once *per expression* rather than once per
//! translation unit.

mod harness;

use std::cell::Cell;

use chiero_parse::{ParsedTu, ScopedTypedefs, parse_tu};
use chiero_pp::{Config, preprocess_str};
use chiero_sema::{SymbolText, TargetConfig, analyze_with};
use chiero_span::Symbol;

/// A `SymbolText` that records how often lowering asked it anything.
struct Counting<'a> {
    inner: &'a ParsedTu,
    calls: Cell<u64>,
}

impl SymbolText for Counting<'_> {
    fn text(&self, sym: Symbol) -> Option<&str> {
        self.calls.set(self.calls.get() + 1);
        self.inner.text(sym)
    }
}

/// `n` trivial functions, each holding exactly one integer literal.
///
/// The literal is what drags `const_of` in; `static` keeps the module self-contained. The
/// bodies are deliberately identical in shape, so the only thing that differs between two
/// sizes is how many *other* functions each one is lowered alongside.
fn functions(n: usize) -> String {
    let mut src = String::new();
    for k in 0..n {
        src.push_str(&format!("static int f{k}(int x) {{ return x + {k}; }}\n"));
    }
    src
}

/// Lower `n` functions and report how many symbol lookups it took.
fn lookups_to_lower(n: usize) -> u64 {
    let src = functions(n);
    let tu = preprocess_str("t.c", &src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let names = Counting {
        inner: &parsed,
        calls: Cell::new(0),
    };
    let analysis = analyze_with(
        &parsed.ast,
        &TargetConfig::x86_64_linux(),
        &names,
        chiero_ast::Dialect::gnu(),
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    // Only lowering is under measurement: analysis is a single pass over the tree by
    // construction and is not what regressed.
    names.calls.set(0);
    let lowered = chiero_lower::lower_tu(&parsed.ast, &analysis, &names);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    assert_eq!(
        lowered.module.funcs.len(),
        n,
        "the fixture must actually lower all {n} functions, or the measurement is of nothing"
    );
    names.calls.get()
}

/// **Doubling the file must not quadruple the work.**
///
/// A linear lowering pass doubles; the quadratic one this pins goes to ~4x. The bar is set
/// at 2.6x rather than at 2.0x because a real pass has some genuinely superlinear
/// bookkeeping (name interning, the verifier's duplicate detection) — the test is a
/// guard against re-walking the translation unit, not a claim that lowering is exactly
/// linear.
#[test]
fn lowering_does_not_rewalk_the_translation_unit_per_constant() {
    let small = lookups_to_lower(40);
    let large = lookups_to_lower(80);
    let ratio = large as f64 / small as f64;
    assert!(
        ratio < 2.6,
        "40 functions took {small} symbol lookups and 80 took {large} — a ratio of \
         {ratio:.2}. Twice the file must not cost four times the work: `const_of` is \
         re-walking every declaration in the translation unit for every constant \
         expression it evaluates."
    );
}

/// **An absolute ceiling, because both ratios above measure a correlate.**
///
/// The counter is `SymbolText::text`, and an implementation that walked the translation unit by
/// `Symbol` id — fetching text only for diagnostics — would keep both ratios flat while staying
/// quadratic in time. A fixed budget per function cannot be satisfied that way: `return x + k;`
/// is a handful of names, and it measures 6.00 lookups per function at every size tried (40, 80,
/// 160, 320). Eight leaves room for an honest change and none for a walk.
#[test]
fn a_trivial_function_costs_a_handful_of_symbol_lookups() {
    for n in [40usize, 160] {
        let per = lookups_to_lower(n) as f64 / n as f64;
        assert!(
            per <= 8.0,
            "lowering `static int f(int x) {{ return x + k; }}` took {per:.2} symbol lookups \
             in a {n}-function file. That is a body of five names; anything larger is a walk \
             over something it should not be looking at."
        );
    }
}

/// **The same file, lowered function by function, must cost the same per function.**
///
/// The ratio test above can be satisfied by making the *whole* pass cheaper while leaving
/// the shape quadratic. This one pins the shape itself: cost per function at F=80 must be
/// close to cost per function at F=40, and under the defect it is twice as much.
#[test]
fn the_cost_of_lowering_a_function_does_not_depend_on_how_many_others_exist() {
    let per_40 = lookups_to_lower(40) as f64 / 40.0;
    let per_80 = lookups_to_lower(80) as f64 / 80.0;
    assert!(
        per_80 < per_40 * 1.3,
        "lowering one function costs {per_40:.0} symbol lookups in a 40-function file and \
         {per_80:.0} in an 80-function one. The cost of a body must come from the body, \
         not from how many neighbours it has."
    );
}
