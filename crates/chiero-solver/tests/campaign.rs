//! The random differential campaign — 022 contract 18.
//!
//! Covers: 022 contract 18.
//!
//! "Random differential campaign of 10 000 terms: zero definite-answer disagreements with
//! z3; the tier-1 `Unknown` rate is recorded and does not regress by more than 2 points."
//!
//! Tier 1 is *deliberately incomplete* (§3), so `Unknown` is a normal answer and not a
//! failure. What must never happen is a **definite** answer that disagrees with z3: §3's
//! whole argument is that `Sat` is self-certifying and `Unsat` is constrained syntactically
//! to a fragment where the reasoning is known sound. A wrong `Unsat` is the dangerous
//! direction — it prunes a path the program has, and nothing downstream can notice.
//!
//! **Scale, and what each scale actually detects.** The contract's figure is 10 000, which
//! at ~16 ms per case is ~2½ minutes — too slow for every `cargo test`. The default here is
//! 400; set `CHIERO_CAMPAIGN=10000` for the contract's own number, which is what CI runs.
//! The campaign is *seeded and deterministic* (001 §5), so a disagreement is reproducible
//! from its index alone rather than being a story about one unlucky run.
//!
//! The two are not interchangeable, and the difference was measured rather than assumed.
//! Mutating tier 1's narrowing so that `v <u k` narrows the wrong end of the interval, or
//! so that every such atom also forces `v <= 3`, fails at 400 (10 and 11 disagreements).
//! A one-off-by-one — `v <u k` narrowing to `v <= k-2` — survives 400 and fails at 4000
//! with 3 disagreements, because it only changes an answer when two bounds on one variable
//! land exactly two apart. So the default catches a broken rule; the contract's number is
//! what catches a rule that is merely slightly wrong. Do not read a green 400-case run as
//! the contract being met.

use chiero_solver::*;

/// A tiny deterministic PRNG. `rand` is not a dependency, and a campaign whose inputs
/// change between runs cannot be re-run against a fix.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*, chosen for being four lines and having no dependencies.
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A random predicate over three 8-bit variables, deep enough to leave tier 1's fragment
/// sometimes and stay inside it others.
fn formula(a: &mut TermArena, r: &mut Rng, vars: &[Term]) -> Vec<Term> {
    let atom = |a: &mut TermArena, r: &mut Rng| -> Term {
        // Biased to the first variable, so a formula's atoms talk about the same thing —
        // which is what makes a domain narrow to nothing, and what real path conditions
        // look like.
        let pick = |r: &mut Rng| {
            if r.below(2) == 0 {
                0
            } else {
                r.below(vars.len() as u64) as usize
            }
        };
        let v = vars[pick(r)];
        let k = if r.below(4) == 0 {
            vars[pick(r)]
        } else {
            // **Constants from a small pool, not uniform over the byte.** Two bounds on
            // one variable only interact when they are *close* — `v > 7 && v < 9` pins a
            // value, `v > 7 && v < 200` says almost nothing — and uniform 8-bit constants
            // put adjacent bounds at a probability the campaign will never reach. Real
            // path conditions compare against 0, 1, small counts, and power-of-two
            // boundaries, which collide constantly.
            const POOL: [u128; 16] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 127, 255];
            a.bv(
                8,
                if r.below(4) == 0 {
                    r.below(256) as u128
                } else {
                    POOL[r.below(16) as usize]
                },
            )
        };
        // **Both operand orders.** `i < n` and `0 < i` are both everywhere in C, and they
        // narrow *opposite ends* of the interval — one lowers `hi`, the other raises `lo`.
        // A generator that only ever puts the variable on the left exercises one of the
        // two rules, and an unsigned domain with a floor of zero and no ceiling can never
        // empty, so tier 1's only route to `Unsat` is a pair of conflicting equalities.
        // Measured: with the variable always left, breaking the `Ult` narrowing rule — in
        // either direction, swapped operands or an off-by-one — survived the whole
        // campaign, because the mutated line was never on a path to a definite answer.
        let (x, y) = if r.below(2) == 0 { (v, k) } else { (k, v) };
        // A mix of the shapes real path conditions have — equalities, comparisons, and
        // the arithmetic that pushes tier 1 out of its fragment.
        // **Weighted toward the shapes §3 says real path conditions have** — "`i < n`,
        // `p != NULL`, `(flags & 4) == 0`, `len - 1 >= 0`" — with nonlinear arithmetic as
        // the minority. A generator that is mostly multiplication measures how often tier
        // 1 gives up on multiplication, which is not the question.
        // Weighted out of twelve, with **unsigned comparison the plurality**. §3 names
        // `i < n` first for a reason, and it is also the only atom shape that moves an
        // interval endpoint — the machinery whose failure would be a wrong `Unsat`. An
        // even split across shapes spends most of the campaign measuring how often tier 1
        // declines to reason about multiplication.
        match r.below(12) {
            0 | 1 => a.eq(x, y),
            2 => {
                let e = a.eq(x, y);
                a.not(e)
            }
            3..=7 => a.ult(x, y),
            8 => a.slt(x, y),
            9 => {
                let s = a.add(x, y);
                let k = a.bv(8, r.below(256) as u128);
                a.ult(s, k)
            }
            10 => {
                let m = a.bv(8, 1u128 << r.below(8));
                // `v`, not `x`: a bit test whose subject is the constant folds away.
                let masked = a.and(v, m);
                let z = a.bv(8, 0);
                a.eq(masked, z)
            }
            _ => {
                let m = a.mul(x, y);
                let k = a.bv(8, r.below(256) as u128);
                a.eq(m, k)
            }
        }
    };
    // **A list of atoms, not one `and` term.** §3.2 admits "a conjunction of atoms", and
    // `check` reads its argument as that conjunction — so a formula handed over as a
    // single `and(...)` term is *one* non-atomic assertion and leaves the fragment
    // immediately. Building it that way took tier 1's decided rate to zero, which the
    // collapse assertion caught. A path condition is a `Vec<Term>` in the engine for the
    // same reason.
    //
    // A wrong tier-1 answer can only escape as a wrong `Unsat` — §3 makes `Sat`
    // self-certifying, so a bad model is rejected by the evaluator and degrades to
    // `Unknown`. `Unsat` comes from an *empty domain*, which needs several atoms narrowing
    // the same variable from *both ends*: `c <u v && v <u c'` is the shape. Unrelated
    // atoms ORed together cannot produce one, which is why this is a conjunction over a
    // small variable set with colliding constants.
    let mut out = vec![atom(a, r)];
    for _ in 0..1 + r.below(3) {
        out.push(atom(a, r));
    }
    // One formula in eight is a disjunction, which is outside the fragment and must come
    // back `Unknown` rather than wrong.
    if r.below(8) == 0 {
        let x = atom(a, r);
        let y = atom(a, r);
        out.push(a.or(x, y));
    }
    out
}

/// **022 contract 18.** Zero definite-answer disagreements, and the `Unknown` rate is
/// reported so a regression in tier 1's reach is visible rather than merely felt.
///
/// **The comparison has to be against the backend, not against a second tiered solver.**
/// `TieredSolver::check` runs tier 1 *first* and consults z3 only when tier 1 answers
/// `Unknown` — so `with_backend(z3)` is not "what z3 says", it is "what tier 1 says, or
/// z3 if tier 1 declined". Comparing `new()` against `with_backend(..)` therefore compares
/// tier 1 with itself on precisely the cases that matter, and any tier-1 defect corrupts
/// both sides identically. Measured: with the narrowing rule mutated to force `v <= 3` on
/// *every* `v <u k` — which answers `Unsat` for `4 <u v <u 200` — that harness reported
/// zero disagreements over 400 formulas. §5's `paranoid` mode is the one path that
/// escalates an already-*decided* answer to the backend, so the campaign runs through it.
#[test]
fn tier_one_never_disagrees_with_the_backend() {
    let Some(backend) = SmtLib::discover() else {
        eprintln!("skipping: no SMT-LIB backend found (022 contract 2)");
        return;
    };
    let n: u64 = std::env::var("CHIERO_CAMPAIGN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);

    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    let mut unknowns = 0u64;
    let mut unsats = 0u64;
    let mut disagreements = Vec::new();

    // `paranoid` reports a mismatch by panicking, which is right for a running analysis
    // and wrong for a campaign that wants every case and an index to reproduce from. The
    // hook is silenced so a real regression prints its cases rather than 400 backtraces.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    for i in 0..n {
        let mut a = TermArena::new();
        let vars: Vec<Term> = (0..3)
            .map(|k| a.var(Sort::BitVec(8), &format!("v{k}")))
            .collect();
        let f = formula(&mut a, &mut rng, &vars);

        // Tier 1 alone, for the `Unknown` rate: a solver with a backend escalates and
        // returns z3's answer, so tier 1's own rate is not visible in the result.
        let mut lite = TieredSolver::new();
        match lite.check(&mut a, &f) {
            CheckResult::Unknown(_) => unknowns += 1,
            CheckResult::Unsat => unsats += 1,
            CheckResult::Sat(_) => {}
        }

        // And the cross-check, on its own solver so no cache carries an answer across.
        let mut heavy = TieredSolver::with_backend(backend.clone());
        heavy.set_paranoid(true);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            heavy.check(&mut a, &f);
        }));
        if let Err(e) = outcome {
            let why = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "non-string panic".into());
            disagreements.push(format!("case {i}: {why}"));
        }
    }
    std::panic::set_hook(hook);

    assert!(
        disagreements.is_empty(),
        "{} definite-answer disagreement(s) — a wrong `Unsat` prunes a path the program \
         has, and nothing downstream can notice:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );

    // **Recorded, not asserted at a threshold.** The contract's "does not regress by more
    // than 2 points" compares against a *previous* run, which a single test cannot see;
    // pinning an absolute number here would fail on any improvement to tier 1 as loudly as
    // on a regression. The rate is printed so CI can diff it, and the loose bounds below
    // catch only a collapse.
    let decided = n - unknowns;
    let rate = 100.0 * unknowns as f64 / n as f64;
    eprintln!(
        "tier-1 Unknown rate: {rate:.1}% over {n} formulas ({decided} decided, {unsats} Unsat)"
    );
    assert!(
        decided > 0,
        "tier 1 decided nothing at all, which is a collapse rather than incompleteness"
    );
    // **And some of them `Unsat`.** The dangerous direction is a wrong `Unsat`, so a
    // campaign in which tier 1 only ever answers `Sat` — every one of which is
    // self-certifying and so cannot be wrong — is not exercising what the contract is
    // about, however many formulas it runs.
    assert!(
        unsats > 0,
        "no formula was refuted, so the campaign never reached the direction that can be \
         silently wrong"
    );
}
