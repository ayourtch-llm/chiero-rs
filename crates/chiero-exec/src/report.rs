//! The rendered report — 023 contracts 12 and 14.
//!
//! §7 rule 4: "a negative result may only be reported as a proof when fidelity is
//! `Exact`. At any other level … the user-facing text says 'not found within <bound>',
//! never 'does not exist'." That rule is about *this file*: [`seal`](crate::seal) decides
//! whether a result **is** a proof, and this decides what a reader is told. Both can be
//! right about the struct and wrong about the sentence, which is why contract 14 is a
//! golden test on the text.
//!
//! What this deliberately never prints: "no bugs exist", "safe", or any word about the
//! program rather than about the search. chiero reports what it looked at and what it
//! did not — the verdict is the reader's, and 023 §7 exists because an LLM reading "no
//! bugs" as "safe" is the failure mode the whole fidelity apparatus is built against.

use crate::{Assumption, Fidelity, Finding, RunResult};
use std::fmt::Write;

/// Render a run as the text a person (or a tool speaking to one) reads.
pub fn render(r: &RunResult) -> String {
    let mut out = String::new();
    let f = r.fidelity();
    let _ = writeln!(out, "chiero run {} — fidelity: {f:?}", r.id());
    let _ = writeln!(out);

    let findings = r.reports();
    if findings.is_empty() {
        // **The only sentence in chiero that a reader may act on as an absence.** Its two
        // forms differ by exactly what §7 rule 4 makes them differ by, and neither claims
        // the program is correct.
        // **Each level gets its own reason.** 023 §7's preamble warns against collapsing
        // them — "a cap that was hit is `Bounded`; discarding values is `Approximated`" —
        // and one sentence for all three pointed a reader at bounds that were not the
        // reason, on runs where nothing was cut short at all. Found by review.
        let _ = match f {
            Fidelity::Exact => writeln!(
                out,
                "no bugs found: the search was exhaustive within the bounds below, and \
                 none of them was reached."
            ),
            Fidelity::Bounded => writeln!(
                out,
                "no bugs found within the bounds below. This is not a proof that none \
                 exist — a bound was reached, and everything past it is unexamined."
            ),
            Fidelity::Approximated => writeln!(
                out,
                "no bugs found, but not within an exact model of the program. This is not \
                 a proof that none exist — values were discarded or code was modeled \
                 approximately, so parts of what ran were not the program. See the \
                 assumptions below."
            ),
            Fidelity::Unknown => writeln!(
                out,
                "no bugs found, and this says nothing about the program. Something on the \
                 way was not understood at all, so an absence of findings here is an \
                 absence of analysis. See the assumptions below."
            ),
        };
    } else {
        let n = findings.len();
        let _ = writeln!(out, "{n} finding{}:", if n == 1 { "" } else { "s" });
        for (i, f) in findings.iter().enumerate() {
            let _ = writeln!(out, "  {}. {} [{:?}]", i + 1, f.message, f.fidelity);
            write_witness(&mut out, f);
        }
    }

    // **Contract 12's second half.** Every degradation's own text, not a count: an
    // assumption recorded and never printed leaves the fidelity a number nobody can act
    // on, and "a dummy assumption must not satisfy this".
    let asms = distinct_assumptions(r);
    let _ = writeln!(out);
    if asms.is_empty() {
        let _ = writeln!(out, "assumptions: none.");
    } else {
        let _ = writeln!(out, "assumptions ({}):", asms.len());
        for a in &asms {
            let _ = writeln!(out, "  - [{:?}] {}", a.kind, a.detail);
        }
    }

    // 020 §4.2's observable effects — what the outside world saw, in the order it saw
    // it. A device register written twice was written twice.
    let effects: Vec<_> = r.states().iter().flat_map(|s| s.effects()).collect();
    if !effects.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "observable effects ({}):", effects.len());
        for e in &effects {
            let _ = writeln!(out, "  - [{:?}] {} at {:?}", e.kind, e.detail, e.span);
        }
    }

    // 020 §4.1's UB events. Not findings — the engine does not decide whether wrapping
    // was a mistake, and VPP wraps deliberately all over — but a reader who cannot see
    // them has to take a checker's word for what the program did.
    let ub = r.ub_events();
    if !ub.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "undefined behaviour ({}):", ub.len());
        for e in &ub {
            let _ = writeln!(out, "  - [{:?}] {} at {:?}", e.kind, e.detail, e.span);
        }
    }

    // 023 §8: the bounds are reported whether or not they were hit, so a reader can tell
    // an `Exact` run under generous bounds from one under trivial ones.
    let b = r.budget();
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "bounds: max_depth {}, max_loop_iters {}, max_recursion_depth {}, max_states {}, \
         max_forks {}, max_indirect {}, max_resolutions {}",
        b.max_depth,
        b.max_loop_iters,
        b.max_recursion_depth,
        b.max_states,
        b.max_forks,
        b.max_indirect,
        b.max_resolutions
    );
    out
}

/// Each distinct `(kind, detail)` once, in the order the run first recorded it.
///
/// A fork copies its parent's assumptions into every descendant, so a flat concatenation
/// prints one budget hit as many times as the run happened to branch afterwards — which
/// reads as a run in far worse shape than it is. This is the same argument
/// [`RunResult::findings`](crate::RunResult::findings) makes for reports.
fn distinct_assumptions(r: &RunResult) -> Vec<Assumption> {
    let mut out: Vec<Assumption> = Vec::new();
    for a in r.states().iter().flat_map(|s| s.assumptions()) {
        if out.iter().any(|b| b.kind == a.kind && b.detail == a.detail) {
            continue;
        }
        out.push(a.clone());
    }
    out
}

/// 023 §9: the witness is what distinguishes a finding from a plausible-sounding guess,
/// so a report that keeps it to itself has not shown its work. An input the model left
/// free says so — a reader who sees a specific number assumes the bug needs it.
fn write_witness(out: &mut String, f: &Finding) {
    match (&f.witness, &f.unwitnessed) {
        (Some(w), _) if w.bindings.is_empty() => {
            let _ = writeln!(out, "     witness: no symbolic inputs on this path");
        }
        (Some(w), _) => {
            let _ = writeln!(out, "     witness:");
            for b in &w.bindings {
                let _ = writeln!(
                    out,
                    "       {} at {:?} = {} ({} bits{})",
                    b.origin.label(),
                    b.origin.span(),
                    b.value,
                    b.width,
                    if b.pinned {
                        ""
                    } else {
                        ", unconstrained — any value replays"
                    }
                );
            }
        }
        (None, Some(why)) => {
            let _ = writeln!(out, "     no witness: {why}");
        }
        (None, None) => {
            let _ = writeln!(
                out,
                "     no witness, and no reason recorded — a bug in chiero"
            );
        }
    }
}
