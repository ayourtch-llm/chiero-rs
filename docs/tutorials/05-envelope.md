# 5. Reading the envelope

This is the short one, and the one that is actually required reading. Everything else in
chiero is a way of producing an answer; this is how to tell what an answer is worth.

## The failure it exists to prevent

An LLM handed `{"findings": []}` will report that the code is safe. It is the reasonable
reading. It is also wrong whenever the list is empty because nothing was checked.

So no operation returns a bare result. Every one returns:

```json
{
  "result":      { ... },
  "fidelity":    "Exact",
  "proven":      true,
  "assumptions": [],
  "blind_spots": [],
  "nondeterministic_abort": false,
  "truncation":  { "truncated": false },
  "determinism_key": "fnv128:..."
}
```

## `proven` is derived, never set

```rust
pub fn new(result: Value, fidelity: Fidelity) -> Envelope {
    Envelope { proven: fidelity == Fidelity::Exact, .. }
}
```

There is no constructor, no setter and no code path by which `proven` and `fidelity` can
disagree. A boolean that *could* be set independently would eventually be set wrongly, once,
in a hurry — and it would be set wrongly in the flattering direction.

| `fidelity` | Means |
|---|---|
| `Exact` | proven for all inputs. The only one that sets `proven`. |
| `Bounded` | proven within a stated bound — a loop unrolled to a depth, an array modelled to a size. |
| `Approximated` | something was modelled imprecisely. A deliberate simplification, not a truncated search. |
| `Unknown` | the engine does not know and cannot bound its ignorance. |

## The two empty answers

```rust
let absent  = expansion_sites_envelope(&map, "NOPE", None, 50);
// result.total == 0, proven: true   — this macro genuinely expands nowhere

let unknown = explain_macro_expansion_envelope(&map, "other.c", 1, None);
// chains: [],      proven: false  — this file is not in the translation unit
```

Both carry an empty list. Only the first means anything. Distinguishing them is the whole job
of the envelope, and the same distinction runs all the way down: coverage keeps "no record"
apart from "recorded zero", impact analysis widens every gap rather than narrowing it, test
selection drops a test only on an `Exact` proof.

## Every unproven answer says why

```rust
assert!(!env.blind_spots.is_empty() || !env.assumptions.is_empty() || truncated);
```

That is asserted over every operation in the test suite, not spot-checked. `proven: false`
with nothing to read is a bare no, which is the shape that gets misread.

- **`assumptions`** — things this specific run rested on. Every assumption kind that actually
  occurred, not a representative sample.
- **`blind_spots`** — classes of thing this answer cannot see at all.
- **`truncation`** — `{ truncated: true, shown: 50, total: 1043 }`. A caller holding 50 of
  1,043 sites does not hold the answer, and nothing about the page is approximate — which is
  exactly why this is easy to lose.
- **`nondeterministic_abort`** — the answer is a measurement rather than a computation. See
  below; it is the only field here that is about the machine instead of about your code.

## The human rendering carries it too

```rust
println!("{}", env.render());
```

— which is what every `chiero` command prints unless you pass `--json`.

The rendering is not a bare list either: it never says "no defects found" unqualified. A
qualification that only exists in JSON is a qualification that gets dropped by the first thing
that formats it for a person.

## Determinism, and the one exception

```rust
env.determinism_key()   // same input → same key, on any machine
```

Byte-identical output for identical input is a hard requirement. `HashMap` and `HashSet` are
lint-banned from every output path.

**`nondeterministic_abort: true` is the one answer that carries no such promise.** A symbolic
run can be given a wall clock — `--time-budget <secs>`, 60 by default on the command line — and
a run the clock ends stopped where this machine's speed put it. Everything it *did* find is
still real; where it stopped is not reproducible.

Twenty-four independent branches is 16 million paths, which is not a search that finishes:

```c
int f (unsigned x)
{
  int t = 0;
  if ((x >> 0) & 1u) t += 1; else t -= 1;
  if ((x >> 1) & 1u) t += 2; else t -= 2;
  /* … 22 more … */
  return t;
}
```

```console
$ chiero find-bugs busy.c --entry f --time-budget 1
findings: (empty)
budgets:
...
not proven — within this run's bounds (Bounded)
  blind spot: the search did not cover the whole program, so an absent finding is not an absent defect
...
  the clock ended this run, not the search — re-running may answer differently
```

The elided line is the interesting one, and it is elided **because it is not reproducible**:

```text
    - wall_clock (1.000s) reached, 20 state(s) left unexplored
```

Twenty on the machine that took this transcript, something else on yours — which is exactly
what the last line of the envelope is warning about, and why the page cannot pin it the way it
pins every other transcript here.

Three more things are worth noticing. The bound is **named**, like every other budget, so a
reader knows which knob to turn. It says **how much was left**, because "a budget was hit" is
true of every bound and actionable for none. And `findings: (empty)` here means *nothing was
found in one second*, not *there is nothing to find* — the distinction this whole page is
about, arriving in the one operation where getting it wrong is expensive.

The library's own default is no clock at all (`Budget::wall_clock: None`), because the
determinism contracts have to run without one. `--time-budget 0` turns it off, as `timeout(1)`
does. **The alternative to a clock is not "no clock"** — it is somebody killing the process
from outside, and a killed process prints nothing: no findings, no fidelity, no envelope. That
is the one output shape this project does not allow, arriving through the back door.

## Checking an operation you have not read

```rust
env.proven                     // is this a proof?
env.fidelity                   // if not, how far off?
env.blind_spots                // what did it not look at?
env.assumptions                // what did it take on faith?
v["truncation"]["truncated"]   // am I holding all of it?
v["nondeterministic_abort"]    // will asking again give the same answer?
```

Six fields. If `proven` is false and the other five say nothing, that is a bug in the
operation, and there is a test that says so.

*Reference: [spec 050 §2](../specs/050-tool-interface.md). Worked example under test:
`crates/chiero-tool/tests/tutorials.rs::tutorial_05_the_envelope`.*
