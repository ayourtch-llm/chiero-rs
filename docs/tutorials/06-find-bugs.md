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
    replay: (none)
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
    replay: (none)
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
  blind spot: no source was given, so no finding carries a replay harness and nothing has checked these against a compiler (040 contract 4)
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
  blind spot: no source was given, so no finding carries a replay harness and nothing has checked these against a compiler (040 contract 4)
```

**This is the one place an empty list is an answer.** No loop, so nothing was cut; every path
was explored and every solver query was decided; `proven` is true. Compare it to the run above,
which also has an empty list *for the parts it did not reach* and says so.

That difference — between "nothing here" and "nothing I looked at" — is what
[tutorial 5](05-envelope.md) is about, and this is the operation where getting it wrong is
expensive.

## Three flags, and why a real codebase needs them

The examples above are five lines long and take a millisecond. Pointed at 40 real VPP
functions, the same operation returned **231 findings** the first time — one of them a false
proof, and most of the rest one artifact repeated. Four engine fixes and the three flags below
took it to **one**, which is the finding that was worth reading all along.

None of the flags hides anything: each records itself in the envelope, and what is suppressed
is always counted there even when it is not shown.

### `--entry-ptr-nonnull` — the caller checked, and this function is not the place to say so

Start anywhere but `main` and the entry's pointer parameters are unconstrained, so `NULL` is a
value they can have:

```console
$ chiero find-bugs hdr.c --entry f
findings:
  - message: null-dereference: access at offset 0 of NULL through h->len, where %1 is a pointer parameter assumed to be possibly null
...
```

That is true, and it is a statement about *the caller contract* rather than about `f`. Measured
over 40 VPP functions, it was 178 findings and not one `Exact` — every one of them this. For a
helper whose callers do check, say so:

```console
$ chiero find-bugs hdr.c --entry f --entry-ptr-nonnull
findings: (empty)
...
proven — this holds for all inputs (Exact)
...
  assumed: entry_ptr_nonnull (the entry's pointer parameters were assumed non-null; a caller that does not check has paths this run did not explore)
```

**It removes real paths, so it is an assumption and appears as one.** The envelope now says the
answer holds for callers that check, which is a different claim from the one above and is
labelled as such.

### `--report-invented-bounds` — a bound chiero picked is not a fact about your program

`find-bugs` gives an entry pointer parameter an object of 4096 bytes and points the parameter at
offset 0 of it. Neither end is something it knows. VPP's vectors put their header *behind* the
data, so `_vec_len(v)` reads at a negative offset — by design:

```c
struct vec_header { unsigned len; };
unsigned vec_len (void *v) { return ((struct vec_header *) v)[-1].len; }
```

```console
$ chiero find-bugs vec.c --entry vec_len --entry-ptr-nonnull
findings: (empty)
...
  blind spot: 1 access crossed a bound chiero invented — the 4096 bytes it gives an object behind an entry pointer, which it also assumes the pointer points at the base of. Neither is a fact about your program, so they are not reported; `report_invented_bounds` (`--report-invented-bounds`) shows them
  assumed: OpaqueCode (the bound crossed belongs to the 4096-byte object reached through an unconstrained pointer, which chiero sized at 4096 bytes and pointed the parameter at the base of because the caller is outside the analysis; a caller passing an interior pointer, or a larger object, has no fault here)
```

Reported by default, this was **147 of 157 findings** on the VPP sample, and it buried the ten
that were about the functions. **The count is always in the envelope even when the findings are
not** — "nothing found" and "147 not shown" are different facts, and collapsing them is the one
move this project does not make. Turn it on for an entry whose callers really do pass a whole
object of a known size.

### `--time-budget <secs>` — an answer beats a killed process

Default 60 seconds; `0` means none. A symbolic run on real code can take longer than anyone
will wait, and the alternative to a clock is not "no clock" but somebody killing the process,
which prints nothing at all. See [tutorial 5](05-envelope.md#determinism-and-the-one-exception)
— a run the clock ends says what it found, what bound stopped it, how many states it left, and
that re-running may answer differently.

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
