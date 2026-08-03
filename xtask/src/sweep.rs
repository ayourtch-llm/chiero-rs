//! Sweep an **external** C tree with chiero and with gcc, and say where they disagree.
//!
//! The hermetic corpus is 28 vendored files — the include closure of six `vppinfra` headers —
//! against VPP's 1552 `.c` files. This tool exists to find the disagreements that a corpus that
//! small cannot, without vendoring a tree: `001 §4 rule 4 / contract 5` keeps VPP knowledge inside
//! `chiero-vpp`, and a tree path passed at run time puts none of it in any crate.
//!
//! **It is a reporting tool, never a gate.** The suite must keep running with no external
//! dependency, so nothing here is wired into `xtask gates`. The working loop is: sweep → queue →
//! reduce a finding → vendor the *reduced* case → RED/GREEN as usual.
//!
//! # gcc is the oracle, with the tree's own flags
//!
//! A chiero diagnostic is a finding only if gcc accepts the same file. The trap is *which* gcc:
//! this project calibrates constraint violations to `-pedantic-errors` (wave 314), while VPP
//! builds under `-std=gnu11` where many of those are legal — `int a[0]` alone appears 1777 times.
//! An oracle run pedantically reports all of it. **The census asks what C forbids; the sweep asks
//! what real code does that chiero mishandles.** Those need different gcc invocations, and the
//! sweep takes the tree's.

use std::path::{Path, PathBuf};

/// What one compiler made of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Compiled or analysed with nothing to say.
    Clean,
    /// Produced diagnostics — the first is kept for the report.
    Diagnosed(String),
    /// Could not be run on this file at all: a flag the tool cannot take, a missing include.
    /// **Never silently dropped**, because a silent skip is how a sweep lies about its coverage.
    NotRun(String),
}

/// Where a file lands once both compilers have spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// gcc accepted and chiero did not — the finding, and the top of the queue.
    Finding,
    /// gcc refused and chiero was silent — a missing rule. Lower priority: gcc's reason may
    /// need flags this sweep does not pass.
    Miss,
    /// Both said the same kind of thing. Nothing to do.
    Agree,
    /// One of the two could not be run. Reported as its own bucket rather than skipped.
    ToolGap,
}

/// Classify one file from the pair of outcomes.
///
/// Pure, and separated from the walking and the running so it can be tested exhaustively — the
/// I/O half is what needs a tree, and this is the half that carries the judgement.
pub fn classify(gcc: &Outcome, chiero: &Outcome) -> Bucket {
    match (gcc, chiero) {
        (Outcome::NotRun(_), _) | (_, Outcome::NotRun(_)) => Bucket::ToolGap,
        (Outcome::Clean, Outcome::Diagnosed(_)) => Bucket::Finding,
        (Outcome::Diagnosed(_), Outcome::Clean) => Bucket::Miss,
        (Outcome::Clean, Outcome::Clean) | (Outcome::Diagnosed(_), Outcome::Diagnosed(_)) => {
            Bucket::Agree
        }
    }
}

/// Every `.c` file under `tree`, sorted, so a sweep is reproducible run to run.
///
/// Headers are not translation units and are swept only through the files that include them.
pub fn translation_units(tree: &Path) -> std::io::Result<Vec<PathBuf>> {
    let _ = tree;
    Ok(Vec::new())
}
