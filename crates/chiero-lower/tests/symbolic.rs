//! **A symbolic differential oracle: every path, checked at a witness the solver chose.**
//!
//! `differential.rs` and `generated.rs` between them run thousands of programs against gcc,
//! and every one of them is *closed and concrete*: `int probe(void)`, one path, one answer.
//! That is the whole engine's input space missed. Forking, path conditions, the feasibility
//! query, `Switch` over a symbolic scrutinee, a symbolic array index — none of it runs in
//! either channel, because a program with no inputs never branches on one.
//!
//! §9 has recorded that limit since wave 139 and named the shape of the fix. This is it:
//!
//! 1. lower `int probe(int x) { … }` and run it **symbolically**, so `x` is a variable and
//!    the engine forks wherever the program branches on it;
//! 2. for each path, hand the path condition to the solver and take a **model** — a
//!    concrete `x` that reaches this path and no other;
//! 3. evaluate the path's return under that model, which is what chiero claims `probe(x)`
//!    is for that `x`;
//! 4. run gcc's `probe(x)` with the same `x` and require the two to be equal.
//!
//! # Why the witness has to come from the solver
//!
//! The obvious cheap version — pick `x` from a constant pool, run concretely, compare — is
//! the channel that already exists, dressed up. It exercises the engine's *concrete*
//! evaluation and tells you nothing about whether the path condition it built is the right
//! one, because nothing ever asks the condition a question.
//!
//! Solving for the witness inverts that. A path condition that is too *weak* admits an `x`
//! the program would have sent elsewhere, and gcc then computes the other branch's answer —
//! a mismatch. A condition that is too *strong* is unsatisfiable and the path vanishes,
//! which [`every_path_is_reachable_at_its_own_witness`] catches by counting. Both failures
//! are invisible to any harness that chooses its own inputs.
//!
//! # What a disagreement means, and what it does not
//!
//! Only a **defined** program can be compared, exactly as in `differential.rs`: gcc is one
//! implementation of undefined behaviour and chiero is another, so a fixture that overflows
//! or reads an uninitialized object teaches nothing. The fixtures here are written to stay
//! inside the language, and the arithmetic is kept away from the boundaries the constant
//! pool in `generated.rs` deliberately aims at.

mod harness;

use chiero_solver::{CheckResult, Solver, TermArena, TieredSolver};

/// Where the oracle can legitimately decline to compare a path, as opposed to failing.
///
/// Named rather than folded into a `bool` because 023's whole discipline is that a run
/// producing nothing must say why. A silent `continue` here would let the file go green
/// while comparing zero paths — which is [`zz_the_symbolic_oracle_actually_ran`]'s job to
/// prevent, and this is what gives it something to count.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Skip {
    /// The path returned no scalar — `Undef`, a pointer, or no `return` at all.
    ReturnNotAScalar,
    /// The solver could not decide the path condition. `SolverLite` handles a
    /// conjunction-of-atoms fragment and says so outside it; 023's answer to "cannot
    /// decide" is to stop rather than guess, so this is the oracle declining, not failing.
    NoModel(String),
    /// The model does not bind the entry parameter, so there is no `x` to run gcc with.
    WitnessMissesParam,
}

/// One path's verdict.
#[derive(Debug)]
struct Checked {
    x: i32,
    chiero: i32,
    gcc: i32,
}

/// What one body's run produced.
#[derive(Debug, Default)]
struct Outcome {
    compared: usize,
    gaps: usize,
    skips: Vec<Skip>,
}

/// Run `int probe(int x) { body }` symbolically and check **every path chiero claims to
/// model exactly** at a witness the solver chose for it.
fn agree_symbolic(body: &str) -> Outcome {
    let src = format!("int probe(int x) {{ {body} }}");
    let m = harness::lower(&src);
    let mut arena = TermArena::new();
    let r = chiero_exec::Engine::new(&m)
        .with_entry("probe")
        .run(&mut arena);

    let bin = match gcc_probe(body) {
        Ok(b) => b,
        Err(Oracle::NoGcc) => {
            eprintln!("SKIP {body}: gcc is absent");
            return Outcome::default();
        }
        Err(Oracle::Broken(why)) => panic!("the oracle is broken, not absent: {why}"),
    };

    let mut out = Outcome::default();
    let mut checked: Vec<Checked> = Vec::new();
    for s in r.states() {
        // **A degraded fidelity is recorded, not skipped**, and the reason is specific to
        // what degrades here. The first version skipped every path whose `Fidelity` was
        // not `Exact` — 023 §7's rule that a limit must not be read as a defect — and the
        // channel then compared *nothing*, because the engine declares `Unknown` on any
        // branch `SolverLite` cannot decide, which is every branch on an input.
        //
        // The distinction the first version missed: "could not decide a branch; both sides
        // explored" is a **conservative over-approximation**, not a wrong value. The engine
        // kept both paths rather than guessing one. If the solver then hands back a witness
        // satisfying this path's condition, the path really is reachable at that `x`, and
        // what the program computes there is a fact gcc can be asked about. A path that is
        // *not* reachable comes back `Unsat` and is skipped below; a path whose value the
        // engine could not model returns no scalar and is skipped too. Neither needs
        // `Fidelity` to catch it, which is what makes reading it as a veto pure loss.
        if s.fidelity() != chiero_exec::Fidelity::Exact {
            out.gaps += 1;
        }
        // The entry parameter is the first input a path ever mints — it exists before the
        // first instruction runs. Taken by position rather than by name: the name is a
        // formatting detail of the engine, but *that it is first* is 023 §9's stated
        // creation order.
        let Some(&(param, _)) = s.inputs().first() else {
            out.skips.push(Skip::WitnessMissesParam);
            continue;
        };
        let mut solver = TieredSolver::new();
        let model = match solver.check(&mut arena, &s.path) {
            CheckResult::Sat(m) => m,
            other => {
                out.skips.push(Skip::NoModel(format!("{other:?}")));
                continue;
            }
        };
        let Some(x) = arena.eval(&model, param).ok().map(|c| c.signed() as i32) else {
            out.skips.push(Skip::WitnessMissesParam);
            continue;
        };
        let Some(bits) = s.return_value_under(&model, &arena) else {
            out.skips.push(Skip::ReturnNotAScalar);
            continue;
        };
        let chiero = bits as u32 as i32;
        let gcc = run_probe(&bin, x);
        checked.push(Checked { x, chiero, gcc });
    }
    let _ = std::fs::remove_dir_all(bin.parent().unwrap());

    for c in &checked {
        assert_eq!(
            c.chiero, c.gcc,
            "`{body}` at the solver's own witness x={}: chiero says {}, gcc says {}. \
             All paths: {checked:?}",
            c.x, c.chiero, c.gcc
        );
    }
    out.compared = checked.len();
    PATHS.fetch_add(out.compared, std::sync::atomic::Ordering::Relaxed);
    out
}

/// **Every path is compared, and the count is the one the source implies.**
///
/// A path condition that is too strong makes its path unsatisfiable: the state exists, the
/// solver says `Unsat`, and the value comparison would simply skip it and pass. So the
/// count of *compared* paths is asserted separately against what a reader can work out from
/// the body — the half of the contract the value comparison cannot see.
///
/// The count is returned per call rather than read from a process-wide counter, which the
/// first version did: `cargo test` runs the tests in this file **in parallel threads**, so
/// a shared counter attributed one fixture's paths to another and reported 2 for a
/// straight-line body. It looked exactly like the engine forking where the program does
/// not branch.
fn agree_symbolic_with_paths(body: &str, want: usize) {
    if !gcc_present() {
        return;
    }
    let out = agree_symbolic(body);
    assert_eq!(
        out.compared, want,
        "`{body}` compared {} path(s), not {want}. Too few means a path vanished — an \
         unsatisfiable condition, or a declared gap; too many means the engine forked \
         where the program does not branch. Gaps: {}, skips: {:?}",
        out.compared, out.gaps, out.skips
    );
}

// A process-wide count of compared paths, for `zz_the_symbolic_oracle_actually_ran` only.
// Monotone and order-independent, so parallel tests cannot corrupt it — unlike the
// per-fixture count, which they did.
static PATHS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn paths_so_far() -> usize {
    PATHS.load(std::sync::atomic::Ordering::Relaxed)
}

fn gcc_present() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .is_ok()
}

enum Oracle {
    NoGcc,
    Broken(String),
}

/// Compile `int probe(int x) { body }` once, with a `main` that takes `x` from the command
/// line — so one compilation serves every witness the solver produces for that body.
fn gcc_probe(body: &str) -> Result<std::path::PathBuf, Oracle> {
    let dir =
        std::env::temp_dir().join(format!("chiero-sym-{}-{}", std::process::id(), next_seq()));
    std::fs::create_dir_all(&dir).map_err(|e| Oracle::Broken(format!("mkdir {dir:?}: {e}")))?;
    let c = dir.join("p.c");
    let bin = dir.join("p");
    std::fs::write(
        &c,
        format!(
            "#include <stdio.h>\n#include <stdlib.h>\n\
             int probe(int x) {{ {body} }}\n\
             int main(int argc, char **argv) {{ printf(\"%d\\n\", probe(atoi(argv[1]))); return 0; }}\n"
        ),
    )
    .map_err(|e| Oracle::Broken(format!("write {c:?}: {e}")))?;
    let out = std::process::Command::new("gcc")
        .args(["-std=gnu11", "-w", "-O0", "-o"])
        .arg(&bin)
        .arg(&c)
        .output()
        .map_err(|_| Oracle::NoGcc)?;
    if !out.status.success() {
        panic!(
            "gcc rejected the fixture `{body}`:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(bin)
}

fn run_probe(bin: &std::path::Path, x: i32) -> i32 {
    let run = std::process::Command::new(bin)
        .arg(x.to_string())
        .output()
        .unwrap_or_else(|e| panic!("running {bin:?} with x={x}: {e}"));
    let text = String::from_utf8_lossy(&run.stdout);
    text.trim()
        .parse::<i32>()
        .unwrap_or_else(|e| panic!("the fixture printed {text:?} for x={x}, not an integer: {e}"))
}

fn next_seq() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------

/// **A branch on an input forks, and each side is right at its own witness.**
///
/// The simplest thing this channel can say that no existing test says at all. Both paths
/// are compared, so a condition that sent every `x` down one arm fails on the other's
/// witness rather than merely producing one fewer state.
#[test]
fn a_branch_on_an_input_is_right_on_both_sides() {
    agree_symbolic_with_paths("if (x > 10) { return 1; } return 2;", 2);
    agree_symbolic_with_paths("if (x == 0) { return 100; } return 200;", 2);
    agree_symbolic_with_paths("return x > 5 ? 7 : 9;", 2);
}

/// **The returned value depends on the input**, so the comparison is of an expression and
/// not of a constant.
///
/// The fixtures above would pass against an engine that got the *arithmetic* wrong on every
/// path, because both answers are literals. Here the witness is substituted into the
/// returned expression, which is what makes `return_value_under` do real work.
#[test]
fn a_returned_expression_agrees_at_the_witness() {
    agree_symbolic_with_paths("return x + 1;", 1);
    agree_symbolic_with_paths("if (x > 0) { return x * 2; } return x - 1;", 2);
    agree_symbolic_with_paths("return (x & 7) + ((x >> 3) & 1);", 1);
}

/// **A `switch` on an input**, which is 020 c14's symbolic scrutinee — a code path the
/// concrete channels cannot reach at all, since a `switch` on a constant folds.
#[test]
fn a_switch_on_an_input_reaches_every_arm() {
    agree_symbolic_with_paths(
        "switch (x) { case 1: return 11; case 2: return 22; default: return 33; }",
        3,
    );
}

/// **A symbolic array index**, the construct wave 116 built `fork_on_offset` for and which
/// nothing has since compared against a compiler.
///
/// The index is masked into range, so every path is defined — an out-of-range read is UB
/// and gcc's answer for it is not an oracle.
#[test]
fn a_symbolic_array_index_agrees_at_the_witness() {
    agree_symbolic("int a[4] = {10, 20, 30, 40}; return a[x & 3];");
}

/// **A loop whose trip count is symbolic**, bounded so the engine's unrolling terminates.
#[test]
fn a_bounded_loop_over_an_input_agrees() {
    agree_symbolic("int n = x & 3; int s = 0; for (int i = 0; i < n; i++) { s += i; } return s;");
}

/// **A comparison against a negative bound, and a test of a single bit.**
///
/// Wave 153 taught the domain two of the four signed narrowings, on the argument that
/// `v >=s k` confines `v` to the non-negative half — one unsigned interval — while `v <s k`
/// admits every negative value *plus* a low range, which is two. That argument is right for
/// a **non-negative** `k` and backwards for a negative one:
///
/// ```text
///   x <s 0     is exactly the negative half        — one interval, [2^31, 2^32-1]
///   x <s -5    is [2^31, 0xFFFFFFFA]               — one interval
///   x >=s -5   is [0, 2^31-1] and [0xFFFFFFFB, …]  — two, and rightly declined
/// ```
///
/// So the sign of the bound decides which polarity is expressible, and wave 153 implemented
/// exactly one half of that. `if (x < 0)` — as ordinary a line as C contains — has a false
/// side the solver can answer and a true side it cannot.
///
/// The mask case is the same shape in a different domain. `x & 1 == 0` is a known-bits fact
/// and pins the bit; `x & 1 != 0` was declined wholesale on the grounds that "one of the
/// masked bits differs" does not say which. True for a multi-bit mask — and vacuous for a
/// **single-bit** one, where there is only one bit it can be. `if (x & FLAG)` is the reason
/// that matters.
#[test]
fn a_negative_bound_and_a_single_bit_test_are_decidable() {
    agree_symbolic_with_paths("if (x < 0) { return 1; } return 2;", 2);
    agree_symbolic_with_paths("if (x < -5) { return 1; } return 2;", 2);
    agree_symbolic_with_paths("if (x <= -1) { return 1; } return 2;", 2);
    agree_symbolic_with_paths("if (x < 0 || x > 100) { return 1; } return 2;", 3);
    // A single-bit mask, in both the `== 0` and the truth-value spellings.
    agree_symbolic_with_paths("if ((x & 1) == 0) { return 100; } return 200;", 2);
    agree_symbolic_with_paths("if (x & 4) { return 1; } return 2;", 2);
}

/// **A value widened before it is compared** — `long l = x; if (l > 5)`.
///
/// The atom is `5 <s sext(x, 64)`, whose operand is not a variable, so no domain narrows and
/// the candidate fails validation on one side. Widening is exact and invertible for a bound
/// that fits the narrow width, which makes this a narrowing the domain can do and simply was
/// never taught.
///
/// It matters beyond `long`: every `char` and `short` in a comparison is promoted to `int`
/// first (C11 6.3.1.1), so the widened-operand shape is what integer promotion produces.
#[test]
fn a_widened_operand_still_narrows_its_source() {
    agree_symbolic_with_paths("long l = x; if (l > 5) { return 1; } return 2;", 2);
    agree_symbolic_with_paths("long l = x; if (l < 0) { return 1; } return 2;", 2);
}

/// **A `switch` whose default is reached only by the largest masked value.**
///
/// `switch (x & 3)` with cases 0, 1 and 2 gives the default arm the path condition
/// `x&3 != 0 && x&3 != 1 && x&3 != 2`, satisfied by `x = 3` and nothing smaller. Each
/// conjunct is a multi-bit negated mask, which cannot be pinned soundly — so the domain
/// stays full and the only model is three steps above its least value.
///
/// The C-level statement of `lite.rs`'s "one candidate is not a search": every arm here is
/// reachable, so a fourth compared path is exactly what a search that does not stop at its
/// first guess buys.
#[test]
fn a_switch_over_a_mask_reaches_its_default() {
    agree_symbolic_with_paths(
        "switch (x & 3) { case 0: return 10; case 1: return 11; case 2: return 12;          default: return 13; }",
        4,
    );
}

/// **A fault found symbolically comes with a witness, and the witness reproduces it.**
///
/// 023 contract 21 says a finding replays "with all inputs concretized". Every test of that
/// until now built its module by hand; this drives it from C, which is the only way the
/// *whole* chain is exercised — lowering, the entry parameter becoming a symbol, the branch
/// forking, the solver choosing a value, the witness recording it, and a second run pinned
/// to that value arriving at the same fault.
///
/// The replay assertion is the load-bearing one. A witness is a claim about the program,
/// and a claim nobody re-runs is a number in a report. Requiring the replay to collapse to a
/// **single state** is what says the inputs really were concretized rather than merely
/// recorded: a replay that still forks has not pinned what it claimed to.
///
/// This is also the fixture §9 has owed since wave 153, where a mutation reducing the
/// witness's variable walk to immediate children survived every channel — the hand-built
/// CIR in `witness.rs` puts its variable one level down, and only a C-lowered comparison has
/// the four-wrapper shape that walk exists for.
#[test]
fn a_symbolic_fault_carries_a_witness_that_reproduces_it() {
    for (body, want) in [
        ("int *p = 0; if (x == 7) { *p = 1; } return 0;", 7i128),
        ("int *p = 0; if (x > 100) { *p = 1; } return 0;", 101),
        ("int *p = 0; if ((x & 3) == 2) { *p = 1; } return 0;", 2),
    ] {
        let src = format!("int probe(int x) {{ {body} }}");
        let m = harness::lower(&src);
        let mut arena = TermArena::new();
        let r = chiero_exec::Engine::new(&m)
            .with_entry("probe")
            .run(&mut arena);

        let f = r
            .reports()
            .into_iter()
            .find(|f| f.message.contains("null"))
            .unwrap_or_else(|| panic!("`{body}`: the guarded null store was not reported at all"));
        let w = f
            .witness
            .clone()
            .unwrap_or_else(|| panic!("`{body}`: reported without a witness"));

        // **The entry parameter is pinned, and to the value the guard demands.** An
        // unpinned binding would say the fault needs no particular input, which is exactly
        // what the guard contradicts.
        let p = w
            .bindings
            .first()
            .unwrap_or_else(|| panic!("`{body}`: the witness binds nothing"));
        assert!(
            p.pinned,
            "`{body}`: the parameter the guard tests is reported as free: {p:?}"
        );
        assert_eq!(
            p.value as i128, want,
            "`{body}`: the witness names a value the guard does not admit"
        );

        // **And replaying it reproduces the fault, on one path.**
        let mut replay_arena = TermArena::new();
        let r2 = chiero_exec::Engine::new(&m)
            .with_entry("probe")
            .replaying(w)
            .run(&mut replay_arena);
        assert_eq!(
            r2.states().len(),
            1,
            "`{body}`: a replay with every input concretized still forked, so something \
             the witness claimed to pin was re-invented"
        );
        assert!(
            r2.reports().iter().any(|f| f.message.contains("null")),
            "`{body}`: the witness did not reproduce the fault it was produced for: {:?}",
            r2.reports()
                .iter()
                .map(|f| f.message.clone())
                .collect::<Vec<_>>()
        );
    }
}

/// **A symbolic divisor that can be zero is a division by zero** — from C, where the
/// operand shape that broke it actually occurs.
///
/// `crates/chiero-exec/tests/ub_events.rs` states this against hand-built CIR. This is the
/// end-to-end half: `100 / x` is the shape a person writes, and until wave 156 it produced
/// no event and left the run claiming `Fidelity::Exact`.
#[test]
fn a_division_by_a_symbolic_zero_is_an_event() {
    for (body, want) in [
        ("return 100 / x;", true),
        ("return 100 % x;", true),
        ("if (x == 0) { return 100 / x; } return 1;", true),
        // **A literal zero divisor with a symbolic numerator.** The concrete check needs
        // *both* operands constant, so this shape reached neither path and was the most
        // obvious division by zero there is.
        ("int z = 0; return x / z;", true),
        ("int z = 0; return x % z;", true),
        // The divisor cannot be zero on the path that divides.
        ("if (x == 0) { return 1; } return 100 / x;", false),
        ("int z = 5; return x / z;", false),
        // **Not a division.** Both operands symbolic, so a check that asked its question
        // of every binary operation would fire here — `x + x` is zero when `x` is.
        ("return x + x;", false),
        ("return x * x;", false),
        ("return x + 1;", false),
    ] {
        let src = format!("int probe(int x) {{ {body} }}");
        let m = harness::lower(&src);
        let mut arena = TermArena::new();
        let r = chiero_exec::Engine::new(&m)
            .with_entry("probe")
            .run(&mut arena);
        let saw = r
            .states()
            .iter()
            .flat_map(|s| s.ub_events())
            .any(|u| u.kind == chiero_exec::UbKind::DivByZero);
        assert_eq!(
            saw,
            want,
            "`{body}`: expected a DivByZero event to be {}",
            if want { "recorded" } else { "absent" }
        );
    }
}

/// **A divisor whose zero-ness the solver cannot decide degrades the run.**
///
/// The third answer, and the one with no natural fixture: `y == 0` for a plain variable is
/// always decidable, so every division written so far comes back `Sat` or `Unsat`. It takes
/// a divisor outside the solver's fragment to reach the middle case — `x * x - 7` is zero
/// exactly when `x * x == 7`, which is nonlinear and which tier 1's diagonal candidate
/// search cannot exhibit.
///
/// Silence here would be the original defect in miniature: an unanswered question reported
/// as a modelled path. 020 §5 — a gap is a diagnostic, not a licence — so the run says
/// `Fidelity::Unknown` and names what it could not decide.
#[test]
fn an_undecidable_divisor_is_a_declared_gap_not_a_silent_one() {
    let src = "int probe(int x) { int d = x * x - 7; return 100 / d; }";
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = chiero_exec::Engine::new(&m)
        .with_entry("probe")
        .run(&mut arena);
    let degraded = r.states().iter().any(|s| {
        s.fidelity() != chiero_exec::Fidelity::Exact
            && s.assumptions().iter().any(|a| a.detail.contains("divisor"))
    });
    assert!(
        degraded,
        "the solver cannot decide whether `x*x - 7` is zero, and the run neither reported \
         a division by zero nor said it could not tell. States: {:?}",
        r.states()
            .iter()
            .map(|s| (
                s.fidelity(),
                s.assumptions()
                    .iter()
                    .map(|a| a.detail.clone())
                    .collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>()
    );
}

/// **A fault with a witness is not an undecided path.**
///
/// When tier 1 cannot decide a branch the engine explores *both* sides and degrades the
/// state to `Fidelity::Unknown`, recording `solver could not decide a branch; both sides
/// explored`. That is the right call at the branch: the path may not be real, and saying so
/// beats guessing.
///
/// It is the wrong call at the end. `attach_witness` then solves the path condition and
/// gets back a **validated model** — 022 §3.1's `Sat` is self-certifying, so a model that
/// satisfies the path condition is a *proof that the path is reachable*. The reason for the
/// degradation has been discharged, and nothing discharges it.
///
/// So a real, reproducible out-of-bounds write at `x = 11` is presented as a run that could not
/// decide anything, which is exactly the label a reader uses to decide what to ignore. The
/// mirror of wave 158: there the finding was right and the number beside it wrong; here the
/// finding and the number are both right and the *confidence* beside them is wrong.
///
/// The pair is the whole test. The first body's inner block is unreachable (`x > 10` and
/// `x < 5`), and that finding **must** stay `Unknown` with no witness — it is the case the
/// degradation exists for. A fix that simply stopped degrading would satisfy the second
/// assertion and break the first.
#[test]
fn a_witnessed_fault_is_not_reported_as_undecided() {
    // **An out-of-bounds write, not a null dereference.** A null store also degrades for
    // `IntToPtr of an integer with no provenance` — a real second caveat, about finding an
    // object by address — so such a path can never be `Exact` and the fixture would be
    // testing two things at once. This write's only caveat is the branch, which is the
    // subject.
    for (body, reachable) in [
        (
            "int a[4] = {0,0,0,0}; if (x > 10) { if (x > 3) { a[7] = 1; } } return 0;",
            true,
        ),
        (
            "int a[4] = {0,0,0,0}; if (x > 10) { if (x < 5) { a[7] = 1; } } return 0;",
            false,
        ),
    ] {
        let src = format!("int probe(int x) {{ {body} }}");
        let m = harness::lower(&src);
        let mut arena = TermArena::new();
        let r = chiero_exec::Engine::new(&m)
            .with_entry("probe")
            .run(&mut arena);
        let f = r
            .reports()
            .into_iter()
            .find(|f| f.message.contains("out-of-bounds"))
            .unwrap_or_else(|| panic!("`{body}`: the out-of-bounds store was not reported"));

        if reachable {
            let w = f
                .witness
                .as_ref()
                .unwrap_or_else(|| panic!("`{body}`: reachable at x = 11 and unwitnessed"));
            assert_eq!(
                w.bindings[0].value as u32 as i32, 11,
                "`{body}`: the witness should name a value that reaches the store"
            );
            assert_eq!(
                f.fidelity,
                chiero_exec::Fidelity::Exact,
                "`{body}`: the witness is a validated model of this path's condition, which \
                 proves the path reachable — so `solver could not decide a branch` has been \
                 answered and must not still be the finding's confidence"
            );
        } else {
            // The unreachable twin. Nothing proved this path, so it keeps its caveat.
            assert!(
                f.witness.is_none(),
                "`{body}`: the inner block needs x > 10 and x < 5 at once"
            );
            assert_eq!(
                f.fidelity,
                chiero_exec::Fidelity::Unknown,
                "`{body}`: nothing discharged the branch the solver could not decide"
            );
        }
    }
}

/// **Discharging the branch does not discharge everything else.**
///
/// A null store on the same undecided path degrades twice: once for the branch, and once
/// for `IntToPtr of an integer with no provenance` — the object was found by address, which
/// is wrong if a different object now occupies it. Proving the path reachable answers the
/// first and says nothing about the second.
///
/// So this finding keeps `Unknown`, and it is the case that makes the fix a *recomputation*
/// rather than an assignment. Two mutations survived the whole suite without it: setting
/// the fidelity to `Exact` outright, and recording every assumption's severity as `Exact`
/// so the fold could only ever produce one. Both are invisible while every fixture has at
/// most one reason to degrade.
#[test]
fn a_proven_path_keeps_the_caveats_the_proof_did_not_answer() {
    let src = "int probe(int x) { if (x > 10) { if (x > 3) { int *p = 0; *p = 1; } } return 0; }";
    let m = harness::lower(src);
    let mut arena = TermArena::new();
    let r = chiero_exec::Engine::new(&m)
        .with_entry("probe")
        .run(&mut arena);
    let f = r
        .reports()
        .into_iter()
        .find(|f| f.message.contains("null"))
        .expect("the null store is reported");
    assert!(
        f.witness.is_some(),
        "reachable at x = 11, so the path is proven and witnessed"
    );
    assert_eq!(
        f.fidelity,
        chiero_exec::Fidelity::Unknown,
        "the branch was answered; `IntToPtr ... no provenance` was not, and it is the \
         reason this path is still not exact"
    );
    let s = r
        .states()
        .iter()
        .find(|s| !s.findings().is_empty())
        .expect("the faulting state");
    assert!(
        s.assumptions()
            .iter()
            .all(|a| a.detail != "solver could not decide a branch; both sides explored"),
        "the branch reason should be gone, not merely outvoted: {:?}",
        s.assumptions()
            .iter()
            .map(|a| &a.detail)
            .collect::<Vec<_>>()
    );
}

/// The companion to `differential.rs`'s `zz_the_oracle_actually_ran`: **a channel that can
/// silently compare nothing is not a channel.**
///
/// Every fixture above can pass by skipping — no model, no scalar return, gcc absent. This
/// runs its own straight-line fixture and requires it to have been compared, so the
/// assertion holds whatever the other tests did. Reading a process-wide counter instead
/// would make this depend on **test ordering**, and `cargo test` runs a file's tests in
/// parallel threads: the first version failed here while four other tests were still
/// running, reporting "compared zero paths" about a run that had compared several.
#[test]
fn zz_the_symbolic_oracle_actually_ran() {
    if !gcc_present() {
        eprintln!("SKIP: gcc is absent, so this file compared nothing");
        return;
    }
    let out = agree_symbolic("return x;");
    assert_eq!(
        out.compared, 1,
        "the symbolic oracle could not compare even `return x;`, so every assertion in \
         this file would report success without asking gcc anything. Skips: {:?}",
        out.skips
    );
    eprintln!(
        "symbolic oracle: {} path(s) compared against gcc at solver-chosen witnesses",
        paths_so_far()
    );
}
