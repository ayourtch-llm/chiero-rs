# 3. Choosing tests

**What you get:** a ranked list of the tests worth running for a given change, with a reason
attached to every one — and, for every test that was *dropped*, the proof that justified
dropping it.

**Measured result:** on a mutation gate over 8 real mutations, **100% recall** (every mutation
that broke a test was caught), against **14.3%** for a coverage-only baseline, while running
**65% fewer** test-cases.

## The example

The inputs are the two tutorials before this one: an index from
[tutorial 1](01-coverage.md) and an impact set from [tutorial 2](02-change-impact.md).

```rust
use chiero_select::{Suite, select_with};

// Every test in the tree, not only the ones the index has heard of — see "the always-run
// set" below.
let suite = Suite {
    tests: vec![TestId(0), TestId(1), TestId(2)],
    validity: index.validity(&repo_root),
};
let selection = select_with(&impact(&before, &after), &after, &index, &suite);

for t in selection.ranked() {
    println!("{t:?}: {:?}", selection.tests[&t]);
}
```

`ranked()` orders by how directly each test touches the change, so a caller with time for
three tests runs the three most likely to catch something.

```rust
let short = selection.budgeted(3);   // top 3, and the rest stay accounted for
```

`budgeted` does not discard the remainder silently — the tests it did not fit are still
recorded, because "we ran 3 of 40" and "there were 3" are different situations.

## Why it beats coverage alone by 7×

Coverage-only selection asks *did this test execute the changed line?* For a macro edited in a
header, the answer is no for every test — the changed line is in a header that gcov has no
entry for. See [tutorial 2](02-change-impact.md). chiero asks instead: *what does this change
reach*, and *which tests covered any of that*.

## The three ways it refuses to be clever

**1. The always-run set.** A test with no coverage record at all — new, crashed, from a stale
build — is always selected. There is no measurement saying it is irrelevant, and absence of
evidence is not evidence.

```rust
selection.tests[&t]  // SelectionReason::AlwaysRun { why: "..." }
```

**2. Stale coverage degrades the answer, visibly.**

```rust
selection.confidence   // Confidence::Full | Reduced { reasons }
```

If the index was measured against sources that have since changed, the confidence is `Reduced`
and the reasons say so. Nothing silently proceeds on stale data.

**3. A test is dropped only on an `Exact` proof.** Symbolic refinement can remove a test that
provably cannot observe the change — but only on a proof over *all* inputs. A `Bounded` proof
(one that holds up to a loop bound), an `Approximated` one, an `Unknown`, or a solver timeout
all mean **keep the test**. This is the seam that
[tutorial 4](04-prove-equivalent.md) plugs into.

```rust
selection.excluded   // ExcludedTest { test, entity, proof, fidelity }
```

Every exclusion records what proved it. An exclusion with no proof cannot be constructed.

## Checking the claim instead of believing it

A selection tool that quietly drops the one test that would have caught your bug is worse than
no tool. So the claim is measured, not asserted:

```bash
cargo run --release -p xtask -- mutation-gate
```

This introduces real mutations into a real program, runs the whole pipeline, and reports
recall beside a **coverage-only baseline computed the same way** — so the number has something
to be judged against. It fails if recall drops.

## Next

[Adjudicating a rewrite →](04-prove-equivalent.md).

*Reference: [spec 032](../specs/032-test-selection.md). Worked example under test:
`crates/chiero-tool/tests/tutorials.rs::tutorial_03_test_selection`.*
