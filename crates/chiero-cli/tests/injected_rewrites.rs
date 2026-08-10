//! **Does `prove-equivalent` adjudicate at all? A paired C corpus whose answer is known in
//! advance.**
//!
//! §8.3's strongest form, pointed at the neighbour that never got it. `injected_defects.rs` was
//! written on 2026-08-10 because every published `find-bugs` number came through the CLI while
//! every test drove hand-written CIR, and it found **seven defects in a morning**.
//! `prove-equivalent` is in exactly that position: `chiero-tool/tests/prove_equivalent.rs` is
//! eight cases of `Module`s built by hand, and the operation a caller actually runs starts at C.
//!
//! **Both directions, and one of them matters far more than the other.** For a pair that really
//! does differ, answering `equivalent` is a *false proof* — the failure this whole project is
//! organised against, since 041 §1 exists to let a caller act on the verdict. For a pair that
//! really is equivalent, `unknown` is an honest answer and only `differs` is a defect. So:
//!
//! | the pair | `differs` | `equivalent` | `unknown` |
//! |---|---|---|---|
//! | genuinely differs | ✅ and the witness must be real | ❌ **a false proof** | 🟡 recorded |
//! | genuinely equivalent | ❌ a false accusation | ✅ | 🟡 recorded |
//!
//! The `unknown` rates are printed rather than asserted: pinning them would be asserting the
//! strength of whatever solver happens to be on `PATH`, and 022 contract 2 makes running without
//! one a supported configuration. What is asserted is that neither wrong answer ever appears,
//! and that the corpus is not silently answering `unknown` to everything — a suite where nothing
//! is decided passes every assertion about wrong answers.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

/// **Whether a complete solver is reachable**, which decides what may be asserted here.
///
/// 022 contract 2 makes running without one a *supported* configuration — tier 1 answers what it
/// can and says `Unknown` for the rest — and CI runs it as its own leg. Measured with
/// `CHIERO_SMT_SOLVER=/nonexistent`: two of six differing rewrites and one of six equivalent
/// ones are still decided, and every other verdict becomes `unknown`. That is the right
/// behaviour and it would have turned this file red on half of CI.
///
/// So the two wrong answers are forbidden **unconditionally** — a false proof is a false proof
/// with or without z3 — and only the floor that says "something was decided" is conditional.
fn has_solver() -> bool {
    chiero_opt::EquivCfg::new("probe").backend.is_some()
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-rewrites-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

/// The verdict, and the witness bindings when there is one.
fn adjudicate(name: &str, before: &str, after: &str) -> (String, serde_json::Value) {
    let d = scratch();
    let (b, a) = (d.join(format!("{name}-b.c")), d.join(format!("{name}-a.c")));
    std::fs::write(&b, before).expect("write");
    std::fs::write(&a, after).expect("write");
    let out = Command::new(bin())
        .args([
            "prove-equivalent",
            b.to_str().unwrap(),
            a.to_str().unwrap(),
            "--entry",
            "probe",
            "--json",
            "--time-budget",
            "20",
        ])
        .output()
        .unwrap_or_else(|e| panic!("cannot run `{}`: {e}", bin()));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "`prove-equivalent` on `{name}` exited {:?}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("`{name}` did not print JSON ({e}):\n{text}"));
    let verdict = v["result"]["verdict"]
        .as_str()
        .unwrap_or_else(|| panic!("`{name}` has no verdict:\n{v:#}"))
        .to_string();
    (verdict, v)
}

/// One rewrite, and whether the two versions really agree.
struct Case {
    name: &'static str,
    /// What a C compiler would say: do these two functions agree on every input?
    same: bool,
    before: &'static str,
    after: &'static str,
    /// Why the answer is what it is — so a reader meeting a failure can judge the *fixture*
    /// before judging the tool. A corpus whose ground truth is asserted rather than argued is
    /// a corpus that will one day be wrong and unfalsifiable.
    why: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "identity-add",
        same: true,
        before: "int probe (int x) { return x + 0; }\n",
        after: "int probe (int x) { return x; }\n",
        why: "adding zero is the identity on every int, with no overflow to differ about",
    },
    Case {
        name: "double-to-shift",
        same: true,
        before: "unsigned probe (unsigned x) { return x * 2u; }\n",
        after: "unsigned probe (unsigned x) { return x << 1; }\n",
        why: "unsigned multiply wraps modulo 2^32 and so does the shift — the same function",
    },
    Case {
        name: "de-morgan",
        same: true,
        before: "int probe (int a, int b) { return !(a && b); }\n",
        after: "int probe (int a, int b) { return !a || !b; }\n",
        why: "De Morgan, and neither side has an effect for the short circuit to skip",
    },
    Case {
        name: "reassociated-sum",
        same: true,
        before: "unsigned probe (unsigned a, unsigned b) { return (a + b) + 1u; }\n",
        after: "unsigned probe (unsigned a, unsigned b) { return a + (b + 1u); }\n",
        why: "unsigned addition is associative because it is modular; the signed version is \
              the `overflow-reassociation` case below and is not",
    },
    Case {
        name: "branch-to-select",
        same: true,
        before: "int probe (int x) { if (x > 3) return 1; else return 0; }\n",
        after: "int probe (int x) { return x > 3; }\n",
        why: "`>` already yields 0 or 1, so the branch is the select",
    },
    Case {
        name: "loop-to-closed-form",
        same: true,
        before: "unsigned probe (void) { unsigned s = 0; for (int i = 0; i < 5; i++) s += i; \
                 return s; }\n",
        after: "unsigned probe (void) { return 10u; }\n",
        why: "0+1+2+3+4 = 10, and the loop is concrete, so this is a closed form rather than \
              a claim about all inputs",
    },
    Case {
        name: "abs-at-int-min",
        same: false,
        before: "int probe (int x) { return x < 0 ? -x : x; }\n",
        after: "int probe (int x) { return x & 0x7fffffff; }\n",
        why: "the classic: at INT_MIN the negation is UB and the mask gives 0, and they differ \
              for every negative input besides",
    },
    Case {
        name: "signed-divide-to-shift",
        same: false,
        before: "int probe (int x) { return x / 2; }\n",
        after: "int probe (int x) { return x >> 1; }\n",
        why: "signed division truncates toward zero and the arithmetic shift floors: -1/2 is 0 \
              and -1>>1 is -1",
    },
    Case {
        name: "unsigned-compare-flipped",
        same: false,
        before: "int probe (unsigned x) { return x > 10u; }\n",
        after: "int probe (unsigned x) { return (int) x > 10; }\n",
        why: "the cast makes a large unsigned negative, so every x above INT_MAX flips",
    },
    Case {
        name: "off-by-one-bound",
        same: false,
        before: "int probe (int x) { return x >= 10; }\n",
        after: "int probe (int x) { return x > 10; }\n",
        why: "they disagree at exactly one input, which is the case a sampler would miss and a \
              solver should not",
    },
    Case {
        name: "modulo-sign",
        same: false,
        before: "int probe (int x) { return x % 4; }\n",
        after: "int probe (int x) { return x & 3; }\n",
        why: "C's `%` keeps the sign of the dividend: -1 % 4 is -1 and -1 & 3 is 3",
    },
    Case {
        name: "shift-count-swapped",
        same: false,
        before: "unsigned probe (unsigned x, unsigned n) { return x << (n & 31u); }\n",
        after: "unsigned probe (unsigned x, unsigned n) { return (x << n) & 31u; }\n",
        why: "the parenthesis moved, and the second masks the result rather than the count",
    },
];

#[test]
fn the_corpus_says_what_it_is() {
    let (same, differ) = (
        CASES.iter().filter(|c| c.same).count(),
        CASES.iter().filter(|c| !c.same).count(),
    );
    assert!(
        same >= 5 && differ >= 5,
        "a corpus tilted one way is a corpus that measures one thing: {same} equivalent, \
         {differ} differing"
    );
    for c in CASES {
        assert!(
            c.why.len() > 30,
            "`{}` states its ground truth without arguing it",
            c.name
        );
    }
}

/// **The failure this project is organised against.** A rewrite that really differs, called
/// equivalent, is a proof a caller can act on and be wrong.
#[test]
fn no_rewrite_that_differs_is_ever_called_equivalent() {
    let mut decided = 0;
    let mut lines = Vec::new();
    for c in CASES.iter().filter(|c| !c.same) {
        let (verdict, v) = adjudicate(c.name, c.before, c.after);
        assert_ne!(
            verdict, "equivalent",
            "`{}` was called equivalent, and it is not: {}.\n{v:#}",
            c.name, c.why
        );
        if verdict == "differs" {
            decided += 1;
            let bindings = v["result"]["input"].as_array().map_or(0, Vec::len);
            assert!(
                bindings > 0 || c.before.contains("(void)"),
                "`{}` says `differs` and names no input that shows it — 041 §1 makes the \
                 witness the point of the verdict:\n{v:#}",
                c.name
            );
        }
        lines.push(format!("  {:24} {verdict}", c.name));
    }
    eprintln!("differing rewrites:\n{}", lines.join("\n"));
    if !has_solver() {
        eprintln!(
            "no solver on PATH: tier 1 decided {decided}; the floor below is not asserted \
             (022 contract 2), and the no-false-proof assertions above already ran"
        );
        return;
    }
    // **A suite that decides nothing passes every assertion above.** This is the counter that
    // tells "no false proofs" apart from "no answers".
    assert!(
        decided >= 4,
        "only {decided} of {} differing rewrites were decided — the corpus is not exercising \
         the adjudicator, so the absence of false proofs above means nothing",
        CASES.iter().filter(|c| !c.same).count()
    );
}

/// The other direction: a rewrite that really is equivalent must never be accused.
#[test]
fn no_equivalent_rewrite_is_ever_called_different() {
    let mut decided = 0;
    let mut lines = Vec::new();
    for c in CASES.iter().filter(|c| c.same) {
        let (verdict, v) = adjudicate(c.name, c.before, c.after);
        assert_ne!(
            verdict, "differs",
            "`{}` was called different, and it is not: {}.\n{v:#}",
            c.name, c.why
        );
        decided += usize::from(verdict == "equivalent");
        lines.push(format!("  {:24} {verdict}", c.name));
    }
    eprintln!("equivalent rewrites:\n{}", lines.join("\n"));
    if !has_solver() {
        eprintln!("no solver on PATH: tier 1 proved {decided}; the floor below is not asserted");
        return;
    }
    assert!(
        decided >= 4,
        "only {decided} of {} equivalent rewrites were proved equivalent — the absence of false \
         accusations above says nothing if nothing was decided",
        CASES.iter().filter(|c| c.same).count()
    );
}
