# 1. Reading coverage

**What you get:** a queryable record of what every test actually executed, decoded straight
from the compiler's own `.gcno`/`.gcda` files — no `gcov` subprocess, no text parsing.

**Why it is not just "which lines ran":** the answers keep *"nobody measured this"* apart from
*"measured, and it never ran"*, and almost everything useful downstream depends on that
distinction holding.

## The example

You have a build compiled with `--coverage` and a test that has been run:

```
build/
  t.gcno      # structure, written at compile time
  t.gcda      # counts, written when the test ran
```

```rust
use chiero_gcov::{CoverageIndex, TestId, TestOutcome};
use std::path::Path;

let build_dir = Path::new("build");
let mut index = CoverageIndex::default();
chiero_gcov::ingest_native_as(&mut index, TestId(0), build_dir, "t")?;
index.record_outcome(TestId(0), TestOutcome::Passed);
```

`"t"` is the object's **stem**: your build wrote `t.gcno` (structure, at compile time) and
`t.gcda` (counts, at run time) next to each other, and both are read. Call it once per test,
with a different `TestId` each time, and the index accumulates.

```rust
let file = index.files().next().unwrap().to_string();
let measured: Vec<u32> = index.lines_of(&file);

let executed = measured.iter()
    .filter(|l| index.line_count(&file, **l).is_some_and(|c| c > 0))
    .count();
let never_ran = index.uncovered_lines(&file).len();

assert_eq!(executed + never_ran, measured.len());
```

## The distinction that matters

```rust
index.line_count(&file, 99_999)   // → None
```

`None` is not zero. It means **no artifact spoke about this line** — the file was not compiled
with coverage, or that build did not run, or the line has no code on it. A coverage tool that
returns `0` here will tell you a line is dead when the truth is that nobody looked, and that
is the single most common way a coverage-driven decision goes wrong.

Everything downstream inherits this. Test selection treats "no measurement" as *run the test*,
never as *this test is irrelevant* — see [tutorial 3](03-test-selection.md).

## Which tests touched a line

```rust
index.tests_for_line(&file, line)          // → Option<Vec<TestId>>
index.tests_for_span(file, lo, hi)         // a range, e.g. a function body
index.tests_for_block(&file, &cir_block)   // a basic block from the IR
```

Again `Option`: `None` means unmeasured, `Some(vec![])` means measured and touched by nothing.

## Is this index still true?

```rust
index.record_sources(&repo_root);   // hash the sources at ingest time
let validity = index.validity(&repo_root);   // Fresh | Stale { .. } | Unknown
```

Coverage is **historical** — it records what the tests did against the code as it was. If a
source file has changed since, the index is `Stale`, and a caller that has been told so can
decide what to do. A caller that has *not* been told will quietly reason about the wrong
program.

## Compilers and formats

Both gcc (12+ and ≤11, which use different on-disk layouts) and clang are decoded by the same
reader; `ingest_json` reads `gcov --json-format` output for cases where you have that instead.

Verified across the whole of VPP: **1,895 gcc `.gcno` files and 1,872 clang ones decode, and
line counts for 322 objects match `gcov` exactly — 0 differences across 156,991 lines.**

## From the command line

Coverage is an *input* to the operations rather than an operation itself, so it appears as a
flag on the one that consumes it:

```console
$ chiero select-tests before.c after.c --coverage build/ --stem t
```

See [tutorial 3](03-test-selection.md).

## Next

[What a change reaches →](02-change-impact.md), which is where coverage stops being enough.

*Reference: [spec 030](../specs/030-coverage-gcov.md). Worked example under test:
`crates/chiero-tool/tests/tutorials.rs::tutorial_01_reading_coverage`.*
