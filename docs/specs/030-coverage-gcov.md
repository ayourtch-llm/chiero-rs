# 030 — Coverage ingest (gcov)

`chiero-gcov` turns gcc coverage artifacts into a queryable index: which tests executed
which lines, blocks and arcs. It is the factual substrate for test selection
([032](032-test-selection.md)) — and, just as importantly, the component whose *limits*
justify chiero's existence.

All format details below were verified on this machine against **gcc 13.3.0** on
2026-07-26. Nothing here is quoted from documentation without being checked.

## 1. What gcov actually attributes — measured, not assumed

The premise of the whole project is that coverage data cannot see macro bodies. That
claim was tested directly rather than assumed:

```c
/* m.h */
#define ADD1(V) do { (V) = (V) + 1; (V) = (V) * 2; } while (0)   /* line 1 */
static inline int hdr_fn(int x){ return x < 0 ? -x : x; }        /* line 2 */

/* t.c */
int main(void){ int v=1; ADD1(v); ADD1(v); printf("%d %d\n", v, hdr_fn(-3)); }  /* line 3 */
```

`gcc --coverage`, run, `gcov -b --json-format`:

```
FILE: t.c     fn main   lines 3-3   line 3 count 1
FILE: m.h     fn hdr_fn lines 2-2   line 2 count 1
```

The result is sharper than "gcov is imprecise about macros":

- **`ADD1` is expanded twice at `t.c:3`, and `m.h:1` receives no coverage record at all.**
  Not a zero count — *no entry*. Every statement in the macro body is attributed to the
  single line of the call site.
- **A `static inline` function in the same header gets its own file entry and its own
  line counts.** `m.h:2` is covered normally.

So the boundary is exact: **coverage follows the expansion site for macros, and the
definition site for functions** — including inline ones. Editing `hdr_fn`'s body is
answerable from coverage data alone; editing `ADD1`'s body is not, and no amount of
post-processing of gcov output can recover it. VPP has 754 distinct `foreach_*` X-macros
and a `vec.h`/`pool.h` layer where the hot logic is macros, so this is the common case,
not the corner case.

chiero closes that gap with `SourceMap`'s reverse expansion index
([010 §3.1](010-source-and-provenance.md)): the coverage index is keyed on
`expansion_loc` — the line gcov sees — while the *impact* analysis works in terms of
macro identity. [031](031-change-impact.md) joins them.

**Hard rule for this crate:** every correlation from a `Span` to a coverage line goes
through `SourceMap::expansion_loc`. Using `spelling_loc` produces lines that gcov never
records, silently matching nothing. Contract 12.

## 2. Two ingest paths

| Path | Fidelity | Use |
|---|---|---|
| `gcov --json-format` | line counts, function summaries, positional branch counts | On-ramp; implement **first**; works wherever `gcov` runs. |
| Native `.gcno` + `.gcda` | full CFG: blocks, arcs, arc counts, per-block line sets | Precise; required for arc-level selection and CIR correlation. |

Both populate the same `CoverageIndex` (§5), so downstream code is agnostic. The JSON
path is not a toy: it is the fallback whenever a `.gcno` version is one chiero does not
decode, which will happen the first time VPP CI moves to a new gcc.

## 3. JSON ingest

Verified schema, gcc 13.3.0, `format_version: "1"`:

```jsonc
{ "format_version": "1", "gcc_version": "13.3.0",
  "current_working_directory": "/build/dir", "data_file": "t.c",
  "files": [ { "file": "t.c",
      "functions": [ { "name": "f", "demangled_name": "f",
                       "start_line": 2, "start_column": 12,
                       "end_line": 2, "end_column": 60,
                       "blocks": 4, "blocks_executed": 4, "execution_count": 4 } ],
      "lines":     [ { "line_number": 2, "function_name": "f", "count": 4,
                       "unexecuted_block": false,
                       "branches": [ {"count": 2, "throw": false, "fallthrough": true},
                                     {"count": 2, "throw": false, "fallthrough": false} ] } ] } ] }
```

Output is gzipped (`<stem>.gcov.json.gz`) and the stem is the **object** name, not the
source name — `t.gcov.json.gz`, not `t.c.gcov.json.gz`. A trivially wrong assumption
here silently ingests nothing, so contract 3 pins it.

**Known loss, and the reason the native path exists:** the `branches` array is
*positional*. Each entry carries only `count`, `throw` and `fallthrough` — no target
block, no arc identity. A line with four branch entries cannot be mapped back to CFG
edges. Line-level selection is therefore fully supported from JSON; arc-level selection
is not, and the ingest records `CoverageDetail::Lines` so downstream code cannot
accidentally assume otherwise.

Paths are resolved against `current_working_directory`, then canonicalized. VPP builds
out-of-tree, so unresolved relative paths are the most likely first-day failure.

## 4. Native `.gcno` / `.gcda`

Verified headers on this machine:

```
t.gcno:  6f 6e 63 67 | 2a 33 33 42 | b6 26 e4 a0     "oncg"  "*33B"  stamp
t.gcda:  61 64 63 67 | 2a 33 33 42 | b6 26 e4 a0     "adcg"  "*33B"  stamp
```

Magic is `"gcno"`/`"gcda"` written little-endian; the version tag `"*33B"` reverses to
`B33*` = gcc **13.3**. The **stamp is identical in both files** and is the pairing key: a
`.gcda` whose stamp differs from the `.gcno` is stale and must be rejected, never merged
(contract 8). This is the single most common source of nonsense coverage data in a build
tree that was not cleaned.

Structure is a record stream: `tag: u32, length: u32 (in 4-byte words), payload`.
Target tags:

| Tag | Meaning |
|---|---|
| `0x01000000` | `FUNCTION` — ident, lineno/column/end checksums, name, source, line |
| `0x01410000` | `BLOCKS` (gcno) |
| `0x01430000` | `ARCS` (gcno) — per arc: destination block, flags |
| `0x01450000` | `LINES` (gcno) — per block: source file, line numbers |
| `0x01a10000` | `COUNTER_ARCS` (gcda) — counters, u64 each |
| `0xa1000000` | `OBJECT_SUMMARY` |

Arc flags: `ON_TREE = 1`, `FAKE = 2`, `FALLTHROUGH = 4`.

Exact field layouts are gcc-version-specific and are **not** transcribed from
documentation into this spec, because a transcription error is undetectable by reading.
Instead the decoder is validated behaviourally: contract 5 requires that for every file
in the corpus, chiero's decoded line counts equal `gcov --json-format`'s, which is ground
truth produced by the same gcc that wrote the file.

### 4.1 Reconstructing block counts

`.gcda` stores counters only for arcs **not** on gcc's spanning tree (`ON_TREE` arcs are
omitted — that is the space optimization the format exists for). Block and on-tree arc
counts are recovered by the classic flow solve:

1. Build the CFG from `BLOCKS`/`ARCS`; mark `ON_TREE` arcs as unknown.
2. Assign known counts from `COUNTER_ARCS` to non-tree arcs in order.
3. Iterate: for any block where all but one incident arc is known, the remaining arc is
   determined by conservation (in-flow = out-flow). Repeat to fixpoint.
4. The spanning-tree property guarantees termination with everything determined. If
   anything remains unknown, the data is corrupt — **report it, do not guess**.

`FAKE` arcs (to the exit block from `noreturn` calls) participate in conservation but are
not real control flow and must be excluded from arc-level selection.

Version support is explicit: chiero decodes the versions it has been tested against, and
an unknown version tag produces a clear diagnostic naming the version plus an automatic
fallback to the JSON path — never a best-effort decode of an unknown layout.

## 5. The coverage index

```rust
pub struct CoverageIndex {
    pub tests: Vec<TestId>,                             // dense, ordered
    pub detail: CoverageDetail,                         // Lines | LinesAndArcs
    /// (file, line) -> tests that executed it. Roaring bitmaps: VPP has ~1M lines
    /// and thousands of tests, so a dense matrix is not an option.
    line_tests: IndexMap<(FileId, u32), TestBitmap>,
    /// (file, line) -> aggregate execution count, saturating.
    line_counts: IndexMap<(FileId, u32), u64>,
    /// Arc-level, only when detail == LinesAndArcs.
    arc_tests: IndexMap<(FuncKey, ArcId), TestBitmap>,
    funcs: IndexMap<FuncKey, FuncCoverage>,
    pub provenance: Vec<IngestRecord>,                  // §7
}

pub struct FuncKey { pub file: FileId, pub name: Symbol, pub start_line: u32,
                     pub march: Option<Symbol> }
```

`FuncKey` includes `march` because VPP compiles one source many times under different
`CLIB_MARCH_VARIANT` ([060](060-vpp-integration.md)). Two variants of the same function
are **different coverage entities**; merging them by name would attribute one variant's
tests to another's code. `start_line` disambiguates `static` helpers that repeat across
files, consistent with [014 §4](014-semantics-and-types.md).

Queries:

```rust
fn tests_for_line(&self, f: FileId, line: u32) -> &TestBitmap;
fn tests_for_span(&self, sm: &SourceMap, sp: Span) -> TestBitmap;   // via expansion_loc
fn tests_for_block(&self, sm: &SourceMap, b: &Block) -> TestBitmap; // union over gcov_lines
fn uncovered_lines(&self, f: FileId) -> impl Iterator<Item = u32>;
fn count(&self, f: FileId, line: u32) -> u64;
```

`tests_for_block` is the bridge to CIR: `Block::gcov_lines` ([020 §3](020-cir.md)) was
computed with `expansion_loc` precisely so this union is a correct join and not a
coincidence.

## 6. Per-test attribution

One coverage set per test, obtained by running each test with its own output tree:

```
GCOV_PREFIX=/cov/<test-id>  GCOV_PREFIX_STRIP=<n>  ./run-test
```

`GCOV_PREFIX_STRIP` removes `n` leading directory components from the *compile-time*
absolute path before re-rooting under `GCOV_PREFIX`; getting it wrong produces deep
mirrored trees or collisions. It is computed from the build directory rather than
configured by hand.

Counters **accumulate** across runs into an existing `.gcda`, so each test's tree must
start empty. A test that forks writes from every process that exits normally; a test that
crashes or is killed writes nothing, which must be recorded as *unknown* coverage rather
than *no* coverage — the two mean opposite things for selection, and conflating them is
how a test-selection tool starts silently dropping the tests most likely to be broken.

```rust
pub enum TestOutcome { Passed, Failed, Crashed, TimedOut, NotRun }
pub struct IngestRecord { pub test: TestId, pub outcome: TestOutcome,
                          pub coverage_complete: bool, pub source_hash: Blake3,
                          pub config: ConfigId, pub march: Option<Symbol>,
                          pub gcc_version: String, pub stamp: u32 }
```

Any test with `coverage_complete == false` joins the always-run safety set
([032](032-test-selection.md)).

## 7. Staleness

Coverage is a claim about a specific source state, and acting on stale coverage produces
confidently wrong test selection. Every `IngestRecord` carries the source hash, the
`ConfigId`, the gcc version and the `.gcno` stamp. On ingest:

- stamp mismatch between `.gcno` and `.gcda` → reject that pair with a diagnostic;
- source hash differing from the working tree → the index is `Stale`, and every consumer
  must either refuse or degrade explicitly (selection falls back to always-run);
- differing gcc versions across the corpus → allowed, recorded, and surfaced.

There is no "probably fine" path. `CoverageIndex::validity()` returns
`Fresh | Stale { files } | Partial { missing_tests }`, and [032](032-test-selection.md)
is required to pattern-match on it.

## 8. Testable contracts

1. Ingesting the gcc-13.3 JSON fixture yields the exact line counts in the fixture, and
   `gcc_version` and `format_version` are recorded.
2. **The macro attribution fact is pinned as a test**: for the `m.h`/`t.c` fixture in §1,
   the index has an entry for `t.c:3` and **no entry of any kind** for `m.h:1`, while
   `m.h:2` (the `static inline`) is present with count 1. If a future gcc changes this,
   this test fails loudly — the entire justification for the hand-written frontend
   depends on it.
3. The JSON reader locates `t.gcov.json.gz` from object stem `t`, and reports a clear
   error (not zero coverage) when given the source stem `t.c`.
4. JSON ingest sets `CoverageDetail::Lines`, and `tests_for_arc` on such an index is a
   compile-time-unavailable operation, not a runtime empty answer.
5. **Cross-validation gate**: for every file in `tests/corpus/coverage/`, native
   `.gcno`/`.gcda` decoding produces line counts identical to `gcov --json-format` on the
   same files. Any mismatch fails CI with the file and line named.
6. The flow solve recovers all on-tree arc counts for the corpus; a deliberately
   truncated `.gcda` produces a corruption diagnostic and **no** partial index.
7. `FAKE` arcs are excluded from arc-level queries but included in the conservation solve.
8. A `.gcda` whose stamp differs from its `.gcno` is rejected with a diagnostic naming
   both stamps, and contributes nothing to the index.
9. An unknown `.gcno` version tag produces one diagnostic naming the version and falls
   back to JSON ingest; it never attempts a speculative decode.
10. Two tests covering overlapping lines produce a bitmap union, and
    `tests_for_line` returns exactly the covering tests for 10 000 random line queries in
    a synthetic 1M-line index.
11. Memory: an index over 1M lines × 5000 tests with realistic sparsity stays under a
    documented budget, asserted by a benchmark test.
12. **`grep -n 'spelling_loc' crates/chiero-gcov/src` yields no hits** — correlation is
    via `expansion_loc` only (§1).
13. `tests_for_block` on a CIR block whose `gcov_lines` are `{10, 11}` returns the union
    of the tests for lines 10 and 11.
14. Two `CLIB_MARCH_VARIANT` builds of one function produce two `FuncKey`s, and coverage
    from one is never returned for the other.
15. Two `static` functions with the same name in different files produce distinct
    `FuncKey`s.
16. A test that is killed mid-run yields `coverage_complete == false`, and its absence
    from the index is distinguishable from a test that ran and covered nothing.
17. `GCOV_PREFIX_STRIP` is computed such that a build in `/build/vpp/src` writes
    per-test trees with no collisions across 100 tests (verified on a fixture tree).
18. Modifying a source file after ingest makes `validity()` return `Stale` naming that
    file, and [032](032-test-selection.md)'s selector falls back to always-run on it.
19. Ingesting the same artifacts twice produces byte-identical indices.
