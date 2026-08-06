# 6. Finding defects

**What you get:** the defects 040's checkers look for, each with a concrete input that reaches
it — and, more importantly, a statement of what the search did **not** cover.

**Why the second half is the point:** every other operation's empty answer is merely
uninformative. This one's empty answer is *"your code is fine"*, and it is wrong in exactly the
case that is hardest to notice — when the search stopped early.

## The example

```c
int average (int *a, int n)
{
  int total = 0;
  for (int i = 0; i < n; i++)
    total += a[i];
  return total / n;
}
```

```console
$ chiero find-bugs average.c --entry average
findings:
  - message: null-dereference: access at offset 0 of NULL, where %2 is a pointer parameter assumed to be possibly null
    paths: 1
    fidelity: Unknown
    solver: z3
    witness:
      - origin: parameter 1
        width: 32
        value: 1
        signed: 1
        pinned: true
      - origin: load with no value
        width: 32
        value: 0
        signed: 0
        pinned: false
    unwitnessed: (none)
  - message: division-by-zero: SDiv by a divisor the path allows to be zero
    paths: 2
    fidelity: Exact
    solver: z3
    witness:
      - origin: parameter 1
        width: 32
        value: 0
        signed: 0
        pinned: true
    unwitnessed: (none)
budgets:
  hit:
    - max_loop_iters (8) reached on the back edge BlockId(3) -> BlockId(1) in `average`
not proven — within this run's bounds (Unknown)
  blind spot: the search did not cover the whole program, so an absent finding is not an absent defect
  blind spot: only the 2 checkers of 040 ran; a defect no checker looks for is not reported
  assumed: BudgetHit (max_loop_iters (8) reached on the back edge BlockId(3) -> BlockId(1) in `average`)
  assumed: NoInformation (a load produced no value, so its result is invented)
```

`average(a, 0)` divides by zero, and the witness says so: `parameter 1` is `0`. The null
dereference above it is the other kind of answer — chiero was given a pointer parameter and
nothing constraining it, so it reports what a caller passing `NULL` would get, and marks the
finding `Unknown` because the load it depends on produced no value.

## Reading the parts that are not findings

**`paths`.** One bug, and the number of paths that reached it. 023 §6.1 keeps a loop's reports
separate — those really are separate reports — so an unrolled loop turns one division by zero
into nine near-identical lines. They are grouped here, but never silently: `paths: 2` is what
tells you the loop is involved.

**`budgets.hit`.** The search stopped at eight loop iterations because the trip count is an
input and chiero chose a bound. It names *which* budget, because the actionable difference
between `max_loop_iters` and `max_states` is which knob to turn.

**`fidelity` per finding.** `Exact` on the division by zero: that fault is real on a path the
solver decided completely. `Unknown` on the null dereference: it rests on a load chiero could
not give a value. A definite fault stays actionable in a run some other path degraded.

**The blind spots.** Two, always worth reading:

- *the search did not cover the whole program* — the loop bound again, from the other side
- *only the 2 checkers of 040 ran* — a defect no checker looks for is not reported, and never
  will be by this operation

## The clean case

```c
int clamp (int x)
{
  if (x < 0)
    return 0;
  return x;
}
```

```console
$ chiero find-bugs clamp.c --entry clamp
findings: (empty)
budgets:
  hit: (empty)
proven — this holds for all inputs (Exact)
  blind spot: only the 2 checkers of 040 ran; a defect no checker looks for is not reported
```

**This is the one place an empty list is an answer.** No loop, so nothing was cut; every path
was explored and every solver query was decided; `proven` is true. Compare it to the run above,
which also has an empty list *for the parts it did not reach* and says so.

That difference — between "nothing here" and "nothing I looked at" — is what
[tutorial 5](05-envelope.md) is about, and this is the operation where getting it wrong is
expensive.

## What it will not find

The checker list is short and honest about it. `chiero-check` implements 040's undefined
arithmetic (division by zero, over-wide shifts, signed overflow, float-cast overflow) and
order dependence; the memory faults above come from the engine itself. A defect outside that
set is not reported, and the blind spot says so on every single run rather than only when the
list is empty.

## Next

Back to [the envelope](05-envelope.md) if you have not read it. It is short, and it is what
makes every answer above safe to act on.

*Reference: [spec 040](../specs/040-defect-checkers.md), [050 §3](../specs/050-tool-interface.md).
Worked example under test: `crates/chiero-tool/tests/tutorials.rs::tutorial_06_find_bugs`.*
