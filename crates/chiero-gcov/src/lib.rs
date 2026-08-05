//! `chiero-gcov` — gcc coverage artifacts as a queryable index (030).
//!
//! Two ingest paths feed one [`CoverageIndex`]: `gcov --json-format`, implemented here, and the
//! native `.gcno`/`.gcda` decode that arc-level selection needs. The JSON path is not a toy — it
//! is the fallback whenever a `.gcno` version is one chiero does not decode, which happens the
//! first time a CI moves to a new gcc.
//!
//! # What this crate can and cannot know
//!
//! Coverage follows the **expansion site** for a macro and the **definition site** for a
//! function, including an inline one. `tests/corpus/coverage/` pins that with gcov's own output:
//! a macro expanded twice at `t.c:3` puts both expansions on that one line and leaves the macro's
//! own line with *no record at all*. So this crate can answer "which tests executed this line"
//! and can never answer "which tests executed this macro body" — that join belongs to 031,
//! through the preprocessor's expansion index.

pub mod native;

use std::path::{Path, PathBuf};

use indexmap::IndexMap;

/// How much of the CFG an ingest recovered.
///
/// **A type rather than an empty answer.** JSON's `branches` entries are positional — a count, a
/// `throw` flag and a `fallthrough` flag, with no target block and no arc identity — so a line
/// with four of them cannot be mapped back to CFG edges. Downstream code asks this and finds
/// arc-level queries *unavailable*, rather than asking for arcs and getting nothing back.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CoverageDetail {
    /// Line counts and function summaries. `gcov --json-format`.
    Lines,
    /// Blocks, arcs and per-block line sets. Native `.gcno`/`.gcda`.
    LinesAndArcs,
}

/// What one ingest read, kept so a report can say where a number came from (030 §7).
#[derive(Clone, Debug)]
pub struct IngestRecord {
    pub artifact: PathBuf,
    pub gcc_version: String,
    pub format_version: String,
}

/// Coverage, queryable by (file, line).
///
/// Paths are stored **as gcov wrote them**. Resolving them against
/// `current_working_directory` belongs to the caller, which knows the build layout;
/// canonicalizing here would bake this machine's filesystem into an index 031 wants to move
/// between them.
#[derive(Clone, Debug, Default)]
pub struct CoverageIndex {
    detail: Option<CoverageDetail>,
    /// `(file, line) -> aggregate execution count`, saturating.
    ///
    /// **A missing key and a zero are different facts.** gcov omits a line it never recorded and
    /// writes `0` for one it recorded as unexecuted; collapsing the two would turn "coverage
    /// cannot see this" into "nothing ran this", which is the misreading 030 §1 exists to
    /// prevent.
    line_counts: IndexMap<(String, u32), u64>,
    provenance: Vec<IngestRecord>,
}

impl CoverageIndex {
    /// The aggregate count for a line, or `None` when gcov recorded no entry for it.
    pub fn line_count(&self, file: &str, line: u32) -> Option<u64> {
        self.line_counts.get(&(file.to_string(), line)).copied()
    }

    /// Every line gcov recorded for a file, ascending.
    pub fn lines_of(&self, file: &str) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .line_counts
            .keys()
            .filter(|(f, _)| f == file)
            .map(|(_, l)| *l)
            .collect();
        v.sort_unstable();
        v
    }

    /// How much of the CFG the ingests behind this index recovered.
    ///
    /// Empty means nothing was ingested, and `Lines` is the honest answer for that: an index with
    /// no data cannot answer an arc query either.
    pub fn detail(&self) -> CoverageDetail {
        self.detail.unwrap_or(CoverageDetail::Lines)
    }

    pub fn provenance(&self) -> &[IngestRecord] {
        &self.provenance
    }

    /// Every file the index holds coverage for, in ingest order.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        let mut seen: Vec<&str> = Vec::new();
        for (f, _) in self.line_counts.keys() {
            if !seen.contains(&f.as_str()) {
                seen.push(f);
            }
        }
        seen.into_iter()
    }

    /// Merge one line's count, taking the **maximum** where a line already has one.
    ///
    /// Max rather than sum, for the reason `line_counts` in `native` records: several blocks on
    /// one line report the largest, which is what gcov does and what `loop.c` measures.
    pub(crate) fn add_line(&mut self, file: String, line: u32, count: u64) {
        let slot = self.line_counts.entry((file, line)).or_insert(0);
        *slot = (*slot).max(count);
    }

    pub(crate) fn set_detail(&mut self, d: CoverageDetail) {
        self.detail = Some(d);
    }

    pub(crate) fn push_provenance(&mut self, r: IngestRecord) {
        self.provenance.push(r);
    }

    /// The gcc that produced the coverage, from the first ingest.
    pub fn gcc_version(&self) -> &str {
        self.provenance.first().map_or("", |p| &*p.gcc_version)
    }

    /// The `format_version` of the JSON that produced it.
    pub fn format_version(&self) -> &str {
        self.provenance.first().map_or("", |p| &*p.format_version)
    }
}

/// What an ingest can fail with.
///
/// **Every variant names the artifact.** A coverage tool that answers "no tests cover this" when
/// it in fact read nothing is worse than one that fails, so nothing here degrades to an empty
/// index (030 contract 3).
#[derive(Debug)]
pub enum IngestError {
    /// The artifact is not where the stem said it would be.
    Missing { path: PathBuf },
    /// It is there and could not be read or decompressed.
    Unreadable { path: PathBuf, why: String },
    /// It decompressed and is not the JSON 030 §3 records.
    Malformed { path: PathBuf, why: String },
    /// A native artifact of a version this decoder has no fixture for (030 contract 9).
    ///
    /// **Its own variant, because the caller does something different about it**: an unknown
    /// version falls back to the JSON path, where a malformed file is a fault to report.
    UnknownVersion { path: PathBuf, tag: String },
    /// A `.gcda` from a different compilation than its `.gcno` (030 contract 8).
    StaleData {
        notes: PathBuf,
        data: PathBuf,
        notes_stamp: u32,
        data_stamp: u32,
    },
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestError::Missing { path } => write!(
                f,
                "no coverage artifact at {}: gcov names it after the *object* stem, so `t.o` \
                 gives `t.gcov.json.gz` and never `t.c.gcov.json.gz`",
                path.display()
            ),
            IngestError::Unreadable { path, why } => {
                write!(f, "cannot read {}: {why}", path.display())
            }
            IngestError::Malformed { path, why } => {
                write!(f, "{} is malformed: {why}", path.display())
            }
            IngestError::UnknownVersion { path, tag } => write!(
                f,
                "{} was written by a gcc whose format tag `{tag}` this decoder has no fixture \
                 for; the JSON path reads any version, and guessing a record layout does not",
                path.display()
            ),
            // **Both stamps, and both paths.** The number is the only thing that says which
            // build a file came from, and the answer is always "rebuild one of these two".
            IngestError::StaleData {
                notes,
                data,
                notes_stamp,
                data_stamp,
            } => write!(
                f,
                "{} (stamp {notes_stamp:08x}) and {} (stamp {data_stamp:08x}) are from different \
                 compilations; the counters do not belong to this control-flow graph",
                notes.display(),
                data.display()
            ),
        }
    }
}

impl std::error::Error for IngestError {}

/// Ingest `gcov --json-format` output for one object stem.
///
/// `stem` is the **object** name — `t` for `t.o`, giving `t.gcov.json.gz`. Passing the source
/// name is the first-day mistake 030 contract 3 pins, and it is an error naming the file that was
/// looked for rather than an empty index.
pub fn ingest_native(dir: &Path, stem: &str) -> Result<CoverageIndex, IngestError> {
    native::ingest(dir, stem)
}

/// Ingest `gcov --json-format` output for one object stem.
pub fn ingest_json(dir: &Path, stem: &str) -> Result<CoverageIndex, IngestError> {
    let mut idx = CoverageIndex::default();
    ingest_json_into(&mut idx, dir, stem)?;
    Ok(idx)
}

/// The same, merging into an index that may already hold other objects' coverage.
pub fn ingest_json_into(
    idx: &mut CoverageIndex,
    dir: &Path,
    stem: &str,
) -> Result<(), IngestError> {
    let path = dir.join(format!("{stem}.gcov.json.gz"));
    if !path.exists() {
        return Err(IngestError::Missing { path });
    }
    let bytes = std::fs::read(&path).map_err(|e| IngestError::Unreadable {
        path: path.clone(),
        why: e.to_string(),
    })?;
    let mut json = String::new();
    {
        use std::io::Read as _;
        flate2::read::GzDecoder::new(&bytes[..])
            .read_to_string(&mut json)
            .map_err(|e| IngestError::Unreadable {
                path: path.clone(),
                why: format!("gunzip: {e}"),
            })?;
    }
    let doc: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| IngestError::Malformed {
            path: path.clone(),
            why: e.to_string(),
        })?;

    let text = |v: &serde_json::Value, k: &str| -> String {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let files =
        doc.get("files")
            .and_then(|f| f.as_array())
            .ok_or_else(|| IngestError::Malformed {
                path: path.clone(),
                why: "no `files` array".into(),
            })?;

    for file in files {
        let Some(name) = file.get("file").and_then(|f| f.as_str()) else {
            return Err(IngestError::Malformed {
                path: path.clone(),
                why: "a `files` entry with no `file`".into(),
            });
        };
        let lines = file.get("lines").and_then(|l| l.as_array());
        for line in lines.into_iter().flatten() {
            let (Some(n), Some(c)) = (
                line.get("line_number").and_then(|n| n.as_u64()),
                line.get("count").and_then(|c| c.as_u64()),
            ) else {
                return Err(IngestError::Malformed {
                    path: path.clone(),
                    why: format!("a `lines` entry of {name} with no line_number/count"),
                });
            };
            // **Saturating, and merged rather than replaced.** One object's coverage may be
            // ingested beside another's, and two runs of the same line add up — a count that
            // wrapped would read as a line nothing executed.
            let slot = idx
                .line_counts
                .entry((name.to_string(), n as u32))
                .or_insert(0);
            *slot = slot.saturating_add(c);
        }
    }

    // **`Lines`, and it stays `Lines` even beside a native ingest.** Detail is the *weakest* any
    // contributing ingest offered: an index half of whose files have arcs cannot answer an arc
    // query about the other half, and saying otherwise is how a downstream selection silently
    // drops tests.
    idx.detail = Some(CoverageDetail::Lines);
    idx.provenance.push(IngestRecord {
        artifact: path,
        gcc_version: text(&doc, "gcc_version"),
        format_version: text(&doc, "format_version"),
    });
    Ok(())
}
