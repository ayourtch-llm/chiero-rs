//! **The frontend's cost as a curve in input size** — the axis no other gate has.
//!
//! Two quadratics have been found in this workspace by *sampling a real run*: the CIR verifier's
//! dominator pass, and `parse_model`. Item 5b's audit greps `.contains(&`, which finds neither
//! shape reliably — `parse_model`'s was `text.split(…)` inside a per-item loop.
//!
//! The obvious alternative, timing real VPP files, cannot work: `vppinfra/bitmap.c` is 167 lines
//! of its own source and 185 434 lines of CIR, because a translation unit's size here is its
//! *header closure*. The whole corpus spans **1.7x** (§7.26), and a quadratic needs an order of
//! magnitude to bend a curve. So the size axis has to be generated, which is what this is.
//!
//! **Two shapes, because they stress different structures.** Many small functions grows the
//! module; one large function grows a single CFG, which is where a per-function pass that walks
//! everything for every item shows up. The verifier defect was of the second kind.

use std::time::Instant;

struct Names<'a>(&'a chiero_parse::ParsedTu);
impl chiero_sema::SymbolText for Names<'_> {
    fn text(&self, s: chiero_span::Symbol) -> Option<&str> {
        self.0.text(s)
    }
}

/// Milliseconds spent in each stage, for one source.
#[derive(Debug, Clone, Copy)]
struct Stages {
    parse: f64,
    sema: f64,
    lower: f64,
    verify: f64,
}

fn stages_of(src: &str) -> Stages {
    use chiero_pp::{Config, preprocess_str};
    let tu = preprocess_str("scale.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics.first());

    let mut oracle = chiero_parse::ScopedTypedefs::new();
    let t = Instant::now();
    let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
    let parse = t.elapsed().as_secs_f64();
    assert!(
        parsed.diagnostics.is_empty(),
        "{:?}",
        parsed.diagnostics.first()
    );

    let names = Names(&parsed);
    let t = Instant::now();
    let an = chiero_sema::analyze(
        &parsed.ast,
        &chiero_sema::TargetConfig::x86_64_linux(),
        &names,
    );
    let sema = t.elapsed().as_secs_f64();
    assert!(an.diagnostics.is_empty(), "{:?}", an.diagnostics.first());

    let t = Instant::now();
    let m = chiero_lower::lower_tu(&parsed.ast, &an, &names);
    let lower = t.elapsed().as_secs_f64();
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics.first());

    let t = Instant::now();
    let errs = chiero_cir::verify::verify(&m.module);
    let verify = t.elapsed().as_secs_f64();
    assert!(errs.iter().all(|e| !e.is_error()), "{:?}", errs.first());

    Stages {
        parse,
        sema,
        lower,
        verify,
    }
}

/// `n` separate functions, each tiny. Grows the module, not any one CFG.
fn many_functions(n: usize) -> String {
    let mut s = String::from("int sink;\n");
    for i in 0..n {
        s.push_str(&format!(
            "static int f{i}(int a, int b) {{ int t = a + b; if (t > {i}) t = t - 1; return t; }}\n"
        ));
    }
    s.push_str("int main(void) { return 0; }\n");
    s
}

/// One function with `n` statements and locals, and a branch every fourth — so the CFG grows
/// with `n` rather than staying a single block, which is what a dominator pass needs to bite.
fn one_big_function(n: usize) -> String {
    let mut s = String::from("int sink;\nint big(int a) {\n  int acc = a;\n");
    for i in 0..n {
        s.push_str(&format!("  int v{i} = acc + {i};\n"));
        if i % 4 == 0 {
            s.push_str(&format!(
                "  if (v{i} > {i}) {{ acc = v{i}; }} else {{ acc = acc + 1; }}\n"
            ));
        } else {
            s.push_str(&format!("  acc = acc ^ v{i};\n"));
        }
    }
    s.push_str("  return acc;\n}\nint main(void) { return 0; }\n");
    s
}

/// What a 4x step in input may cost, per stage. **A ratchet at the measured curve, not a claim
/// of linearity** — three of these four stages are superlinear today and the ceilings say so.
///
/// Measured 2026-08-09 on `one_big_function`, ratio per 4x step (linear is 4x, quadratic 16x):
///
/// | stage | 1024→4096, first measured | after the fixes |
/// |---|---|---|
/// | parse | 3.8x | 4.0x — linear throughout |
/// | sema | 6.3x | **4.4x — near linear** |
/// | verify | 6.7x | 6.9x |
/// | lower | **11.2x** | **5.7x** |
///
/// Two O(n²) scans came out of lowering the day this gate was built: `emit` scanned every
/// block to find the current one on **every instruction**, and `reachable_from` used a `Vec`
/// with `contains` plus a per-block `find`. **Six came out in all** — two in lowering, two in
/// the verifier's `check_structural_identity`, two in sema's scope lookups. A 32 768-statement
/// function went **22.7 s → 3.8 s**. That is a real limit and it is
/// §9.1's open item; this gate exists so it cannot quietly get worse first. The ceilings are
/// the measured value with room for a loaded machine, and every one is *below* 16x, so a stage
/// that becomes outright quadratic at these sizes fails here.
///
/// **Ratios, not bounds** — a wall-clock threshold stops being able to fail as the machine
/// gets faster, which is the whole reason 5a rewrote the verifier's scale test.
fn max_ratio_per_4x(stage: &str) -> f64 {
    match stage {
        // The one stage that is genuinely linear, so it is held to it.
        "parse" => 6.0,
        // 4.4x measured once the scope lookups were indexed — near linear.
        "sema" => 7.0,
        // 6.9x measured after `check_structural_identity`'s two scans became sets.
        "verify" => 9.0,
        // **5.7x measured after the 2026-08-09 fixes**, down from 11.2x. The ceiling moves with
        // it: a ratchet that keeps yesterday's slack cannot see tomorrow's regression.
        _ => 9.0,
    }
}

fn assert_subquadratic(tag: &str, sizes: &[usize], make: fn(usize) -> String) {
    // **The minimum of three runs, per stage.** A single timing at these sizes swings by 2x on
    // a loaded machine — verify's 1024→4096 ratio was measured anywhere from 6.7x to 12.6x, and
    // the gate failed 4 runs in 6 while nothing had changed. The minimum is the standard robust
    // estimator here: scheduling noise only ever *adds* time, so the smallest of k samples is
    // the closest to the work actually done. Loosening the ceilings until nothing failed would
    // have produced a gate that cannot fail, which is the thing this project keeps refusing.
    // **Five, not three, and the reason is on the record.** Three repeats were stable while
    // `verify`'s ceiling sat at 10.0 and went flaky (1 failure in 3) the moment the ceiling
    // followed the measurement down to 9.0. The choice was to raise the ceiling back or to
    // measure better; raising it would have parked permanent slack in the gate to buy quiet,
    // which is the failure mode every note here warns about. Five repeats: 5/5 green, ~2.9 s.
    const REPEATS: usize = 5;
    let mut rows: Vec<(usize, Stages)> = Vec::new();
    for &n in sizes {
        let src = make(n);
        let mut best = stages_of(&src);
        for _ in 1..REPEATS {
            let s = stages_of(&src);
            best = Stages {
                parse: best.parse.min(s.parse),
                sema: best.sema.min(s.sema),
                lower: best.lower.min(s.lower),
                verify: best.verify.min(s.verify),
            };
        }
        rows.push((n, best));
    }
    for (n, s) in &rows {
        eprintln!(
            "{tag} n={n:5}  parse {:.4}  sema {:.4}  lower {:.4}  verify {:.4}",
            s.parse, s.sema, s.lower, s.verify
        );
    }
    let of = |s: &Stages, which: &str| match which {
        "parse" => s.parse,
        "sema" => s.sema,
        "lower" => s.lower,
        _ => s.verify,
    };
    for which in ["parse", "sema", "lower", "verify"] {
        for w in rows.windows(2) {
            let (n0, s0) = w[0];
            let (n1, s1) = w[1];
            let (t0, t1) = (of(&s0, which), of(&s1, which));
            // Below a couple of milliseconds the clock is scheduler noise. Say so rather than
            // assert on it — a silently skipped check reads as a pass.
            if t0 < 0.002 {
                eprintln!("{tag}/{which} n={n0} at {t0:.4}s is too fast to ratio, skipped");
                continue;
            }
            let ratio = t1 / t0;
            let steps = (n1 as f64) / (n0 as f64);
            assert!(
                ratio < max_ratio_per_4x(which),
                "{tag}/{which}: {n0} -> {n1} ({steps:.0}x input) cost {ratio:.1}x. \
                 Linear is {steps:.0}x and quadratic is {:.0}x. Rows: {rows:?}",
                steps * steps
            );
        }
    }
}

#[test]
fn the_frontends_growth_in_function_count_stays_at_todays_curve() {
    assert_subquadratic("many-fns", &[256, 1024, 4096], many_functions);
}

#[test]
fn the_frontends_growth_in_one_functions_size_stays_at_todays_curve() {
    assert_subquadratic("one-big", &[256, 1024, 4096], one_big_function);
}
