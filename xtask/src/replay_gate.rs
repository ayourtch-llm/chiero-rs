//! **032 contract 18 — the historical-replay gate.**
//!
//! > 18. **Safety gate**: on the historical-replay corpus, recall is 100%. A single miss fails
//! >     CI with the commit and test named.
//!
//! 032 §6 calls this *"the ground-truth oracle and the one that would catch a real design
//! flaw"*, and distinguishes it from the mutation gate beside it:
//!
//! > **Historical replay.** For VPP commits with a known test failure, run selection on that
//! > commit's diff against the parent and assert the failing test was selected.
//!
//! The difference that makes it the stronger of the two: a mutation is a change *this project
//! invented*, and a selector can be accidentally tuned to the shapes its own harness produces.
//! A historical commit is a change somebody else made for their own reasons, and the test that
//! caught it is a fact about the world.
//!
//! # Why the corpus is a checked-in manifest
//!
//! Establishing ground truth for one entry means building VPP at a commit and its parent and
//! running the test at both — tens of minutes each. Doing that in CI would make the gate
//! unrunnable, and a gate that does not run is not a gate (the same argument the mutation gate
//! makes about VPP's suite).
//!
//! So the expensive half is done once, by [`verify`], and its result is recorded in
//! `tests/corpus/replay/corpus.tsv`. The gate itself replays *selection* against those
//! recorded facts, which is fast and is the part that regresses.
//!
//! **The manifest records how each entry was established, and the gate refuses the ones that
//! were not.** An entry marked `asserted` is somebody's belief that a test would have caught a
//! commit; only `observed` means the test was run at both commits and changed its verdict.
//! Counting beliefs towards a recall figure is how a ground-truth oracle stops being one — and
//! it is the exact failure the mutation gate had to be rebuilt to avoid (its ground truth is
//! observed, not assumed).
//!
//! # What a miss reports
//!
//! Contract 18 says "fails CI with the commit and test named", which is a requirement about the
//! *message*: a recall figure alone tells a reader that something regressed and nothing about
//! what to look at.

use std::path::{Path, PathBuf};

/// One historical failure: a commit, a test that caught it, and how that was established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The commit that fixed the failure. The diff replayed is `commit^..commit`.
    pub commit: String,
    /// The test that failed at `commit^` and passed at `commit`.
    pub test: String,
    pub evidence: Evidence,
    /// Free text: what the commit did, for a reader looking at a miss.
    pub note: String,
}

/// **How an entry's ground truth was established.** The distinction the gate turns on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// The test was **run** at `commit^` and at `commit`, and changed its verdict. The only
    /// kind that may count towards recall.
    Observed,
    /// Somebody read the commit and concluded the test would have caught it. Recorded so the
    /// candidate is not lost, and **excluded from the figure**: a recall computed over
    /// beliefs measures the beliefs.
    Asserted,
}

impl Evidence {
    fn parse(s: &str) -> Option<Evidence> {
        match s {
            "observed" => Some(Evidence::Observed),
            "asserted" => Some(Evidence::Asserted),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Evidence::Observed => "observed",
            Evidence::Asserted => "asserted",
        }
    }
}

/// The corpus file, `commit<TAB>test<TAB>evidence<TAB>note`, `#` for comments.
///
/// A flat text format on purpose: this file is edited by hand when a candidate is found and by
/// [`verify`] when one is confirmed, and both want a diff that reads.
pub fn corpus_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask/ has a workspace root above it")
        .join("tests/corpus/replay/corpus.tsv")
}

pub fn parse_corpus(text: &str) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 3 {
            return Err(format!(
                "line {}: expected commit<TAB>test<TAB>evidence[<TAB>note], got {line:?}",
                n + 1
            ));
        }
        let evidence = Evidence::parse(f[2]).ok_or_else(|| {
            format!(
                "line {}: evidence must be `observed` or `asserted`, got {:?}",
                n + 1,
                f[2]
            )
        })?;
        out.push(Entry {
            commit: f[0].to_string(),
            test: f[1].to_string(),
            evidence,
            note: f.get(3).unwrap_or(&"").to_string(),
        });
    }
    Ok(out)
}

/// What the gate found, per entry.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub entry: Entry,
    /// `None` when the entry could not be replayed at all — the commit is not in this
    /// checkout, the diff would not parse. **Not a pass**: contract 18 is about recall, and an
    /// entry nobody could evaluate is not an entry that was caught.
    pub selected: Option<bool>,
    pub detail: String,
}

/// The gate's whole report.
#[derive(Clone, Debug)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
}

impl Report {
    /// Entries whose ground truth was observed — the only ones recall is computed over.
    pub fn counted(&self) -> Vec<&Outcome> {
        self.outcomes
            .iter()
            .filter(|o| o.entry.evidence == Evidence::Observed)
            .collect()
    }

    /// `None` when the corpus has no observed entry: a recall over an empty set is `0/0`, and
    /// printing "100%" for it is the most flattering number this project could produce.
    pub fn recall(&self) -> Option<f64> {
        let c = self.counted();
        if c.is_empty() {
            return None;
        }
        let hit = c.iter().filter(|o| o.selected == Some(true)).count();
        Some(hit as f64 / c.len() as f64)
    }

    pub fn render(&self) -> String {
        let mut s = String::from("032 contract 18 — historical replay\n");
        s.push_str("  commit    test                                  evidence  selected\n");
        for o in &self.outcomes {
            s.push_str(&format!(
                "  {:<9} {:<37} {:<9} {}\n",
                &o.entry.commit[..o.entry.commit.len().min(8)],
                o.entry.test,
                o.entry.evidence.label(),
                match o.selected {
                    Some(true) => "yes",
                    Some(false) => "NO",
                    None => "not replayed",
                }
            ));
            if !o.detail.is_empty() {
                s.push_str(&format!("            {}\n", o.detail));
            }
        }
        match self.recall() {
            Some(r) => s.push_str(&format!(
                "\n  recall {:.1}%  over {} observed entr{}\n",
                r * 100.0,
                self.counted().len(),
                if self.counted().len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            )),
            None => s.push_str(
                "\n  recall: NOT MEASURED — the corpus has no `observed` entry.\n  \
                 032 §6 calls this the ground-truth oracle; over asserted entries it is not one.\n",
            ),
        }
        s
    }
}

/// Run the gate.
pub fn replay_gate() -> Result<Report, String> {
    let path = corpus_path();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let entries = parse_corpus(&text)?;
    let outcomes = entries
        .into_iter()
        .map(|entry| Outcome {
            selected: None,
            detail: "replay not implemented".to_string(),
            entry,
        })
        .collect();
    Ok(Report { outcomes })
}
