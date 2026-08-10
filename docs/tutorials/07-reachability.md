# 7. What the code can and cannot reach

**What you get:** for one line, whether execution can get there — and separately, a list of
branches whose condition the surrounding code already decides.

**Why the two are one page:** they are the same question from opposite ends, and they must
agree. If a branch is dead, the line behind it is unreachable; if a line is unreachable, some
branch decided it. Reading them together is also the fastest way to see the distinction the
whole system turns on.

## The example

```c
int classify (int x)
{
  if (x > 0) {
    if (x > 0)
      return 1;
    return 2;      /* line 6 — nothing can get here */
  }
  return 3;
}
```

```console
$ chiero find-optimizations classify.c --entry classify
proposals:
  - kind:
      kind: dead_branch
      reachable_side: true
    file: classify.c
    line: 4
    rationale: the false side of this branch cannot be taken: the path condition already decides it
    advisory: false
    benefit: Unquantified
    evidence:
      - (not (= (_ bv0 32) ((_ zero_extend 31) (ite (bvslt (_ bv0 32) v0_param0) #b1 #b0))))
      - decided: (not (= (_ bv0 32) ((_ zero_extend 31) (ite (bvslt (_ bv0 32) v0_param0) #b1 #b0))))
    obligations:
      - state: discharged
        what: the search was exhaustive, so no path reaches the other side
  - kind:
      kind: redundant_load
      object: 2
      offset: 0
    file: classify.c
    line: 4
    rationale: object 2 at offset 0 is loaded twice with nothing between that could have written it, so the second load could reuse the first
    advisory: false
    benefit: Unquantified
    evidence:
      - nothing happens between them
    obligations:
      - state: discharged
        what: nothing between the two loads could have written the address
count: 2
proven — this holds for all inputs (Exact)
  blind spot: only 041 §2's dead-branch, redundant-load and dead-store detectors ran; a loop-invariant computation, a redundant bounds check or a call-site specialization is not reported
  assumed: proposals_only (chiero never patches code (041 §1))
```

```console
$ chiero check-reachable classify.c --entry classify --line 6
verdict: unreachable
line: 6
proven — this holds for all inputs (Exact)
```

The detector says the inner branch's false side cannot be taken; the reachability check says
line 6 — which *is* that side — is unreachable, and both are `proven`. They arrive from
opposite directions at the same fact, and the evidence in the first is the constraint that
makes the second true.

For contrast, line 5 is the side that can happen, and comes back with the input that gets
there:

```console
$ chiero check-reachable classify.c --entry classify --line 5
verdict: reachable
line: 5
witness:
  - origin: parameter 0
    width: 32
    value: 1
    signed: 1
    pinned: true
proven — this holds for all inputs (Exact)
```

## Four verdicts, and the two in the middle are the point

| verdict | means | `proven` |
|---|---|---|
| `reachable` | here is an input that gets there | ✅ |
| `unreachable` | the search was exhaustive and nothing arrived | ✅ |
| `not_shown_reachable` | chiero did not get there, and cannot say nothing does | ❌ |
| `no_such_line` | that line has no code; nothing was asked | ❌ |

**`unreachable` and `not_shown_reachable` are the same observation and opposite claims.** No
state arrived, either way. One of them licenses deleting the code and the other does not, so
they are separate verdicts rather than one verdict with a caveat — a consumer matching on the
string cannot conflate what it never sees.

Here is the second one, on a line past a loop chiero had to bound:

```c
int loop_then (int n)
{
  int t = 0;
  for (int i = 0; i < n; i++)
    t += i;
  if (t > 1000000)
    return 1;        /* line 7 — genuinely reachable, for large n */
  return 0;
}
```

```console
$ chiero check-reachable loopy.c --entry loop_then --line 7
verdict: not_shown_reachable
line: 7
why: max_loop_iters (8) reached on the back edge BlockId(3) -> BlockId(1) in `loop_then`
not proven — within this run's bounds (Bounded)
  blind spot: no path chiero explored reached this line, and the search was not complete — the line may still be reachable
```

Line 7 *is* reachable. chiero unrolled the loop eight times, never accumulated a million, and
says exactly that — naming the budget, so a reader knows which knob would change the answer. A
tool that reported "unreachable" here would be telling somebody to delete working code, which
is the most expensive form this project's recurring mistake takes.

## And the fourth

```console
$ chiero check-reachable classify.c --entry classify --line 7
verdict: no_such_line
line: 7
why: `classify` has no code on line 7
not proven — within this run's bounds (Unknown)
  blind spot: no block carries this line, so nothing was asked — this is not a claim that the line is dead
```

Line 7 is a closing brace. `no_such_line` exists because three verdicts would have been a trap:
a line with no code would otherwise answer `unreachable` — technically true, and read by anyone
as a statement about the code they asked about.

## What a proposal is worth

`find-optimizations` never rewrites anything (041 §1), and every proposal says whether it may
be acted on:

- **`advisory: false`** with a *discharged* obligation — the search was exhaustive, so nothing
  reaches the other side.
- **`advisory: true`** with an *open* one — the search did not finish, so the other side was not
  shown unreachable, only unvisited. Same observation, opposite claim, again.
- **`benefit: Unquantified`** — chiero has no cycle model and will not pretend to one. A dead
  branch is a real observation whose value in cycles it cannot state.

## Next

[Struct layout →](08-layout.md), or back to [the envelope](05-envelope.md).

*Reference: [050 §3](../specs/050-tool-interface.md), [041 §2](../specs/041-optimization-analysis.md).
Worked example under test: `crates/chiero-tool/tests/tutorials.rs::tutorial_07_reachability`.*
