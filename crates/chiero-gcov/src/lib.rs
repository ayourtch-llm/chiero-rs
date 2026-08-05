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

#[cfg(test)]
mod memory_budget;

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

/// A test whose run produced coverage.
///
/// **An index, not a name.** 030 §5 keeps the test list dense and ordered so a line's test set is
/// a bitmap rather than a set of strings: VPP has thousands of tests and ~1M lines, and the
/// difference is whether the index fits in memory at all. The name belongs to whoever assigned
/// the id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TestId(pub u32);

/// How a test run ended (030 §6).
///
/// **The outcome is a claim about the process, not about coverage.** It is recorded separately
/// from the ingest because the two can disagree in both directions: a test can exit 0 and write
/// nothing, and a test can fail loudly having recorded everything.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TestOutcome {
    Passed,
    Failed,
    /// Killed by a signal. Writes no `.gcda` — the counters are flushed at a normal exit.
    Crashed,
    TimedOut,
    NotRun,
}

impl TestOutcome {
    /// Whether the process ended in a way that flushes counters at all.
    ///
    /// A pass or a failure both exit normally and write; a crash, a timeout and a test that never
    /// ran do not. This is necessary for complete coverage and not sufficient — the artifacts
    /// still have to arrive, which is what [`CoverageIndex::coverage_complete`] also asks.
    fn writes_coverage(self) -> bool {
        matches!(self, TestOutcome::Passed | TestOutcome::Failed)
    }
}

/// Which build of a source file coverage came from (030 §5).
///
/// VPP compiles one source many times under different `CLIB_MARCH_VARIANT`, and the copies are
/// **different code**: `#if defined(CLIB_HAVE_VEC512)` is in one and not another. Recording the
/// variant beside the line is what stops a change inside such a block being attributed to the
/// tests of a build that never contained it.
///
/// `None` is a variant rather than an absence, so a tree with no variants — everything that is
/// not VPP — has exactly one and nothing is filed under an invented name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Variant {
    None,
    Named(String),
}

impl Variant {
    pub fn named(v: &str) -> Variant {
        Variant::Named(v.to_string())
    }
}

/// The tests that executed one line.
///
/// **A sorted `Vec` today, roaring eventually.** 030 §5 specifies roaring bitmaps and contract 11
/// puts a memory budget on 1M lines × 5000 tests, which this representation will not meet — it is
/// here because the *semantics* (union, membership, ordered answers) are what everything
/// downstream depends on, and they do not change when the representation does. The type is
/// private to the crate so that swap stays an implementation detail.
type TestBitmap = Vec<TestId>;

/// What the index knows about one line.
///
/// **The per-build split is empty until a second build appears.** With one variant the union
/// *is* that variant's set, so storing both would be a second copy of every test list in the
/// index for no answer that is not already available. `promote_to_per_variant` fills it in once,
/// when the second variant arrives — the cost is a single pass, and it buys back roughly a third
/// of the index in the case every tree that is not VPP is in.
#[derive(Clone, Debug, Default)]
struct LineEntry {
    /// Aggregate execution count, saturating.
    count: u64,
    /// The tests that executed the line, unioned across builds.
    tests: TestBitmap,
    /// Per build, and empty while the index holds at most one.
    per_variant: Vec<(VariantRef, TestBitmap)>,
}

/// A file's index into [`CoverageIndex::file_names`].
///
/// **Interned rather than repeated.** Every line of the index is keyed by a path, in three maps,
/// and a build tree has a few thousand distinct paths against a million lines. See
/// `src/memory_budget.rs` for what that costs when the key owns its string.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct FileRef(u32);

/// A variant's index into [`CoverageIndex::variants`], for the same reason.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct VariantRef(u32);

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
    /// **One entry per line, not one map per question.** The count, the union of tests and the
    /// per-build split were three maps keyed by the same `(file, line)`, so a million lines paid
    /// three hash buckets and three copies of the key. See [`LineEntry`] and
    /// `src/memory_budget.rs`.
    lines: IndexMap<(FileRef, u32), LineEntry>,
    /// Every path to its reference, for interning during ingest.
    ///
    /// A second copy of a few thousand strings, against a million lines that no longer each hold
    /// one. It is here because interning by scanning `file_names` made ingest quadratic — 170
    /// seconds for the benchmark, against 5 before it.
    file_ids: IndexMap<String, FileRef>,
    /// Every path, once. Keys hold a [`FileRef`] into this rather than a `String` of their own.
    ///
    /// **The whole of 030 contract 11's headroom is here.** A path appears in three keys per line
    /// — the count, the union, the per-variant set — and a VPP path is around 40 bytes, so a
    /// million lines carried 120 MB of the same few thousand strings. Interning is why the index
    /// fits: measured, it took 577 MiB to 178.
    file_names: Vec<String>,
    /// Every variant that contributed, in arrival order. A [`VariantRef`] indexes this.
    variants: Vec<Variant>,
    /// Every test that contributed an ingest, in the order they arrived.
    ///
    /// Kept even when a test covered nothing, so a selection can tell "ran and covered nothing"
    /// from "never ran" — 032 does different things about those.
    tests: Vec<TestId>,
    /// What the runner reported about each test, when it reported anything.
    outcomes: IndexMap<TestId, TestOutcome>,
    provenance: Vec<IngestRecord>,
}

impl CoverageIndex {
    /// The reference for a path already known, or `None` — a lookup must not create one, or
    /// every query for a file nobody ingested would grow the table.
    fn file_ref(&self, file: &str) -> Option<FileRef> {
        self.file_ids.get(file).copied()
    }

    /// The reference for a path, interning it if it is new. Ingest only.
    fn intern_file(&mut self, file: &str) -> FileRef {
        match self.file_ref(file) {
            Some(r) => r,
            None => {
                let r = FileRef(self.file_names.len() as u32);
                self.file_names.push(file.to_string());
                self.file_ids.insert(file.to_string(), r);
                r
            }
        }
    }

    fn variant_ref(&self, v: &Variant) -> Option<VariantRef> {
        self.variants
            .iter()
            .position(|x| x == v)
            .map(|i| VariantRef(i as u32))
    }

    /// The aggregate count for a line, or `None` when gcov recorded no entry for it.
    pub fn line_count(&self, file: &str, line: u32) -> Option<u64> {
        self.lines
            .get(&(self.file_ref(file)?, line))
            .map(|e| e.count)
    }

    /// Every line gcov recorded for a file, ascending.
    pub fn lines_of(&self, file: &str) -> Vec<u32> {
        let Some(r) = self.file_ref(file) else {
            return Vec::new();
        };
        let mut v: Vec<u32> = self
            .lines
            .keys()
            .filter(|(f, _)| *f == r)
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

    /// The tests that executed a line, in test order, or `None` when nothing recorded the line.
    ///
    /// **`None` rather than an empty set.** An empty set is the claim "no test covers this", which
    /// 032 acts on by running nothing; a line gcov never recorded supports no such claim (030 §1).
    pub fn tests_for_line(&self, file: &str, line: u32) -> Option<Vec<TestId>> {
        self.lines
            .get(&(self.file_ref(file)?, line))
            .map(|e| e.tests.clone())
    }

    /// The union of the tests for a set of lines of one file, or `None` when the index recorded
    /// none of them.
    ///
    /// **`None` for "no line was recorded", not for "the recorded lines have no tests".** A line
    /// recorded with no tests is evidence — gcov saw it and nothing ran it — and stays an empty
    /// contribution to a union that other lines may fill. Absence contributes nothing at all.
    /// Every query that unions lines goes through here so the distinction is made once.
    fn union_over_lines(
        &self,
        file: &str,
        lines: impl IntoIterator<Item = u32>,
    ) -> Option<Vec<TestId>> {
        let mut out: Vec<TestId> = Vec::new();
        let mut any = false;
        for line in lines {
            let Some(tests) = self.tests_for_line(file, line) else {
                continue;
            };
            any = true;
            for t in tests {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
        any.then(|| {
            out.sort_unstable_by_key(|t| t.0);
            out
        })
    }

    /// The tests that executed the code a [`Span`] stands for (030 §5).
    ///
    /// **Through `expansion_loc`, never the spelling location** (030 §1, contract 12). A span
    /// inside a macro expansion was *written* in the macro body and *happened* at the call site,
    /// and gcov records only the call site — `tests/corpus/coverage/t.c` pins that, and `m.h:1`
    /// has no record at all. Asking the index about the macro body therefore returns `None`,
    /// which reads as "no test covers this" and is the answer 032 acts on by running nothing.
    /// The failure is silent, survives every unit test, and is why this method exists rather than
    /// callers doing their own lookup.
    ///
    /// A span covering several lines contributes all of them: it is one construct, and a test
    /// that reached any part of it reached the construct. Inside an expansion that collapses to
    /// the call site's own line, which is all gcov ever wrote.
    ///
    /// `None` when the index recorded none of those lines — not an empty set, for the reason
    /// [`tests_for_line`](Self::tests_for_line) gives.
    pub fn tests_for_span(
        &self,
        sm: &chiero_span::SourceMap,
        sp: chiero_span::Span,
    ) -> Option<Vec<TestId>> {
        let lo = sm.expansion_loc(sp)?;
        // The end resolves through the same chain; a span inside one expansion collapses to a
        // single call-site line, and one in ordinary code spans the lines it covers.
        let hi = sm
            .expansion_loc(chiero_span::Span::new(sp.hi, sp.hi, sp.ctx))
            .filter(|h| h.file == lo.file)
            .map_or(lo.line, |h| h.line);
        let file = sm.file(lo.file).path().to_string_lossy().into_owned();
        self.union_over_lines(&file, lo.line.min(hi)..=lo.line.max(hi))
    }

    /// The tests that executed a CIR block: the union over its `gcov_lines` (030 contract 13).
    ///
    /// **The bridge to the IR**, and the join 030 → 031 → 032 runs through. `gcov_lines` was
    /// computed at lowering through `expansion_loc` (015 §5) for exactly this reason: both sides
    /// name the line gcov actually recorded, so the match is an equality rather than a heuristic.
    ///
    /// **`file` is a parameter because a `Block` does not carry one.** 015 §5 keys `gcov_lines`
    /// on "the defining file of the enclosing function" and stores it as a bare `SmallVec<u32>`,
    /// so the file is implicit and belongs to the function. Hand-written `.cir` fixtures settle
    /// it: their spans are `Span::DUMMY` and the `.line` directive fills `gcov_lines` directly,
    /// which 015 §5 introduces so that fixtures can exercise this contract at all — so it must be
    /// answerable for a block whose span resolves to nothing.
    ///
    /// `None` when the index recorded none of those lines, **and when the block has none**: 015
    /// §5 says a block of only compiler-generated instructions has an empty `gcov_lines` and that
    /// gcov has no counter for it either. That is an absence of evidence, not evidence that
    /// nothing ran it, and an empty set here would let 032 skip every test for a change to such a
    /// block.
    pub fn tests_for_block(&self, file: &str, b: &chiero_cir::Block) -> Option<Vec<TestId>> {
        self.union_over_lines(file, b.gcov_lines.iter().copied())
    }

    /// The lines of a file gcov **recorded as zero** — seen by the compiler, executed by nothing.
    ///
    /// **Not the lines with no record.** A blank line, a comment or a declaration is absent from
    /// the index, and absence is not evidence that nothing ran it; reporting those would call
    /// most of every file untested. This is the crate's absence-versus-zero rule (030 §1) pointed
    /// at a whole file, and it is the direction that matters: an uncovered line is a claim, and a
    /// claim needs a record behind it.
    ///
    /// Ascending and without repeats, so two of them can be diffed. A `Vec` rather than the
    /// spec's `impl Iterator` for the reason [`lines_of`](Self::lines_of) is one — the order is
    /// part of the answer, and materialising it is what makes that checkable.
    pub fn uncovered_lines(&self, file: &str) -> Vec<u32> {
        let Some(r) = self.file_ref(file) else {
            return Vec::new();
        };
        let mut out: Vec<u32> = self
            .lines
            .iter()
            .filter(|((f, _), e)| *f == r && e.count == 0)
            .map(|((_, l), _)| *l)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The tests that executed a line **in one build**, or `None` when that build recorded
    /// nothing for it.
    ///
    /// `None` and an empty set differ here as everywhere in this crate: "no coverage recorded for
    /// the AVX-512 build" is not "the AVX-512 build ran nothing", and only the second lets a test
    /// be skipped.
    pub fn tests_for_line_in(&self, file: &str, line: u32, v: &Variant) -> Option<Vec<TestId>> {
        let vr = self.variant_ref(v)?;
        let e = self.lines.get(&(self.file_ref(file)?, line))?;
        if e.per_variant.is_empty() {
            // At most one build has been ingested, so the union is that build's set — and a
            // variant that is not it recorded nothing, which is `None` rather than empty.
            return (self.variants.first() == Some(v)).then(|| e.tests.clone());
        }
        e.per_variant
            .iter()
            .find(|(x, _)| *x == vr)
            .map(|(_, t)| t.clone())
    }

    /// Every build that contributed coverage, in arrival order.
    pub fn variants(&self) -> Vec<Variant> {
        self.variants.clone()
    }

    /// Every test that contributed an ingest, in arrival order.
    pub fn tests(&self) -> Vec<TestId> {
        self.tests.clone()
    }

    /// Record how a test run ended (030 §6).
    pub fn record_outcome(&mut self, test: TestId, outcome: TestOutcome) {
        self.outcomes.insert(test, outcome);
    }

    /// Whether this index holds the whole of a test's coverage.
    ///
    /// **Both halves are required**: the process must have ended in a way that flushes counters,
    /// *and* its artifacts must have been ingested. A test that exits 0 while `GCOV_PREFIX` points
    /// somewhere nothing is written satisfies the first and fails the second, and it is the case a
    /// runner gets wrong most easily.
    ///
    /// A test nobody recorded an outcome for is incomplete: this answers about what it was told,
    /// and silence is not a pass.
    pub fn coverage_complete(&self, test: TestId) -> bool {
        self.outcomes
            .get(&test)
            .is_some_and(|o| o.writes_coverage())
            && self.tests.contains(&test)
    }

    /// The tests that must run whatever the change is (030 §6, 032's safety set).
    ///
    /// Every test this index cannot speak for: it crashed, it timed out, it never ran, or its
    /// coverage never arrived. **Not the same as a test that covers nothing** — that one is
    /// skippable on the evidence, and this one is skippable only by pretending the absence of
    /// evidence is evidence.
    pub fn always_run(&self) -> Vec<TestId> {
        let mut out: Vec<TestId> = self
            .outcomes
            .keys()
            .copied()
            .filter(|&t| !self.coverage_complete(t))
            .collect();
        out.sort();
        out
    }

    /// Every file the index holds coverage for, in ingest order.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        // Interning already made this list: `file_names` is every path once, in ingest order.
        self.file_names.iter().map(|f| f.as_str())
    }

    /// Merge one line's count across *objects*, taking the **maximum** where a line already has
    /// one.
    ///
    /// ⚠️ This doc used to justify the max by "which is what gcov does", citing the line rule.
    /// That rule was wrong (see `tests/line_rule.rs`) and this was never what it did within an
    /// object anyway: gcov accumulates a source's lines across every function and then overwrites
    /// them with a graph count. `native::ObjectLines` now does that merge, and what reaches here
    /// is one already-correct number per object.
    ///
    /// What is left for this method is the question the merge does *not* answer: the same header
    /// line reported by two different builds — VPP compiles many, one per `CLIB_MARCH_VARIANT`.
    /// Summing those would report a line as executed more often than any build executed it. The
    /// maximum is a claim about the busiest build, which is the conservative direction for 032:
    /// too-high a count never causes a test to be skipped. Per-variant answers, which is what a
    /// caller should ask when the distinction matters, come from `tests_for_line_in`.
    pub(crate) fn add_line(&mut self, file: String, line: u32, count: u64) {
        let f = self.intern_file(&file);
        let e = self.lines.entry((f, line)).or_default();
        e.count = e.count.max(count);
    }

    /// The same, attributed to a test and to the build it came from.
    ///
    /// **A line is recorded for the test even when its count is 0.** gcov writing `0` means it
    /// *saw* the line and the test did not execute it, which is a different fact from the line
    /// being absent — and it is the fact 032 needs to not re-run a test for a line it demonstrably
    /// never reached.
    pub(crate) fn add_line_for_variant(
        &mut self,
        test: TestId,
        v: &Variant,
        file: String,
        line: u32,
        count: u64,
    ) {
        self.note_variant(v);
        self.add_line(file.clone(), line, count);
        let f = self.intern_file(&file);
        let vr = self.variant_ref(v).expect("just noted");
        let split = self.variants.len() > 1;
        let e = self.lines.entry((f, line)).or_default();
        if !e.tests.contains(&test) {
            e.tests.push(test);
        }
        if split {
            let per = match e.per_variant.iter_mut().find(|(x, _)| *x == vr) {
                Some((_, t)) => t,
                None => {
                    e.per_variant.push((vr, TestBitmap::new()));
                    &mut e.per_variant.last_mut().expect("just pushed").1
                }
            };
            if !per.contains(&test) {
                per.push(test);
            }
        }
    }

    pub(crate) fn note_variant(&mut self, v: &Variant) {
        if self.variants.contains(v) {
            return;
        }
        self.variants.push(v.clone());
        // **The second variant is where the split starts costing anything.** Until now every
        // line's union was the first build's set; move it into the split so the two stay
        // distinguishable from here on.
        if self.variants.len() == 2 {
            for e in self.lines.values_mut() {
                if !e.tests.is_empty() {
                    e.per_variant.push((VariantRef(0), e.tests.clone()));
                }
            }
        }
    }

    pub(crate) fn note_test(&mut self, test: TestId) {
        if !self.tests.contains(&test) {
            self.tests.push(test);
        }
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
    let mut idx = CoverageIndex::default();
    native::ingest_into(&mut idx, None, dir, stem)?;
    Ok(idx)
}

/// The same, attributing every line it reads to `test` and merging into an existing index.
///
/// One object's coverage from one test run. A test that touches several objects calls this once
/// per object with the same [`TestId`]; the union is what makes that correct.
pub fn ingest_native_as(
    idx: &mut CoverageIndex,
    test: TestId,
    dir: &Path,
    stem: &str,
) -> Result<(), IngestError> {
    native::ingest_into(idx, Some((test, Variant::None)), dir, stem)
}

/// The same, recording which build the object came from (030 §5).
pub fn ingest_native_as_variant(
    idx: &mut CoverageIndex,
    test: TestId,
    variant: Variant,
    dir: &Path,
    stem: &str,
) -> Result<(), IngestError> {
    native::ingest_into(idx, Some((test, variant)), dir, stem)
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
            let f = idx.intern_file(name);
            let e = idx.lines.entry((f, n as u32)).or_default();
            e.count = e.count.saturating_add(c);
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
