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
  "truncation":  { "truncated": false },
  "determinism_key": "..."
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

## The human rendering carries it too

```rust
println!("{}", env.render());
```

The rendering is not a bare list either: it never says "no defects found" unqualified. A
qualification that only exists in JSON is a qualification that gets dropped by the first thing
that formats it for a person.

## Determinism

```rust
env.determinism_key()   // same input → same key, on any machine
```

Byte-identical output for identical input is a hard requirement. `HashMap` and `HashSet` are
lint-banned from every output path.

## Checking an operation you have not read

```rust
env.proven                     // is this a proof?
env.fidelity                   // if not, how far off?
env.blind_spots                // what did it not look at?
env.assumptions                // what did it take on faith?
v["truncation"]["truncated"]   // am I holding all of it?
```

Five fields. If `proven` is false and the other four are empty, that is a bug in the
operation, and there is a test that says so.

*Reference: [spec 050 §2](../specs/050-tool-interface.md). Worked example under test:
`crates/chiero-tool/tests/tutorials.rs::tutorial_05_the_envelope`.*
