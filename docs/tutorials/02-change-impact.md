# 2. What a change reaches

**What you get:** given two versions of a source file, the set of everything the edit affects
— following calls, types, globals **and macro expansions**.

**Why you need it:** coverage cannot answer this, and the reason is structural rather than a
gap someone forgot to fill.

## The problem, in four lines of C

```c
#define SCALE(x) ((x) * 2)
int area   (int w) { return SCALE (w) * w; }
int volume (int w) { return area (w) * w; }
```

Change `* 2` to `* 3`. Which tests should you re-run?

gcov records the **`.c` line a macro was used on** and nothing at all about the macro itself.
So a coverage index has no entry for the header line you edited, and asking it "which tests
covered this line?" returns nothing — which reads exactly like "no test is affected". It is
the most confident possible wrong answer.

## The example

```rust
use chiero_diff::{Program, impact};

let before = Program::parse("geom.c", V1).unwrap();
let after  = Program::parse("geom.c", V2).unwrap();
let set = impact(&before, &after);

let names: Vec<&str> = set.entities.keys().map(|e| e.name()).collect();
// ["SCALE", "area", "volume"]
```

One macro body changed and nothing else in the file was edited. `area` is in the set because
it *expands* `SCALE`; `volume` is in it because it calls `area`. The closure is computed to a
fixpoint, so a chain of any depth is followed.

## Every entry says why

```rust
let j = &set.entities[&Entity::function("geom.c", "area")];
j.class          // what kind of change: BodyChanged, LayoutChanged, ...
j.root           // the entity that was actually edited
j.edges          // the path from `root` to here
j.distance       // how many hops
j.changed_lines  // which lines, for joining against coverage
```

An impact set with no justification is an assertion. With one, a caller — human or LLM — can
check the reasoning instead of trusting the conclusion.

## Whether the answer is complete

```rust
set.completeness   // Complete | Partial { .. }
```

`Partial` means something could not be analysed — a file that would not parse, a translation
unit not supplied. **The entities from a partially-parsed file are still in the set**, because
the safe direction is to over-report; the flag exists so that over-reporting is not mistaken
for a complete answer.

## What it will not miss

- **Renaming a field** changes no offset, so it is not a layout change — but it *is* a change,
  and it is reported as one.
- **`__attribute__((packed))` on an already-tight struct** changes no byte in that struct, and
  still changes the alignment, and therefore moves fields in any struct embedding it. Layout
  is compared as *computed layout keyed by field name*, not as tokens.
- **A function whose address is taken** (`table[] = { f }`, `p = &f`) escapes into something
  that cannot be followed, and is treated accordingly rather than assumed uncalled.

## Rendering it

```rust
println!("{}", set.render());   // for a human
println!("{}", set.to_json());  // for a program
```

## Next

[Choosing tests →](03-test-selection.md), which joins this against tutorial 1's coverage.

*Reference: [spec 031](../specs/031-change-impact.md). Worked example under test:
`crates/chiero-tool/tests/tutorials.rs::tutorial_02_change_impact`.*
