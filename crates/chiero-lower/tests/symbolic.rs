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
