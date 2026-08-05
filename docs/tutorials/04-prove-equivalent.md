# 4. Adjudicating a rewrite

**What you get:** given two versions of a function, either a proof that they agree for every
input, or a concrete input at which they do not.

**Why this operation exists:** an LLM is good at proposing a faster or clearer version of a C
function and bad at being sure it is correct. chiero is bad at inventing rewrites and good at
deciding whether two functions agree. **The LLM proposes; chiero adjudicates.**

## The rewrite that looks right

```c
int f (int x) { return x < 0 ? -x : x; }        // before
```

An LLM notices that `-INT_MIN` overflows and proposes a version that saturates instead. Sound
reasoning, defensible change, different function.

```rust
let cfg = chiero_opt::EquivCfg::new("f");
let env = chiero_tool::prove_equivalent(&before, &after, &cfg);
```

```json
{
  "verdict": "differs",
  "input":   [{ "origin": "parameter 0", "width": 32,
                "value": "2147483648", "signed": "-2147483648" }],
  "observation": { "kind": "return_value",
                   "before_signed": "-2147483648",
                   "after_signed":  "2147483647" }
}
```

One input out of 2^32. *"Your rewrite is wrong"* is an opinion; *"it returns 2147483647 where
the original returns -2147483648 when `x == INT_MIN`"* ends the discussion.

## The agreeing direction

```rust
// int f (int x) { return x * 2; }   vs   { return x << 1; }
let env = chiero_tool::prove_equivalent(&double, &shift, &cfg);
env.proven    // true
env.fidelity  // Exact
```

`Exact` means proven over all 2^32 inputs — not sampled, not spot-checked. Try it with `x / 2`
against `x >> 1` and you get `Differs` with a negative witness, because C rounds division
toward zero and arithmetic shift toward negative infinity.

## Reading the verdict

| Verdict | What it licenses |
|---|---|
| `Equivalent` + `fidelity: Exact` | a proof. Safe to act on. |
| `Equivalent` + `Bounded` | holds within a loop bound chiero chose. **Not** a proof. |
| `Equivalent` + `Approximated` | the two agree, resting on a callee chiero could not model. |
| `Differs` | a concrete input. Reproducible. |
| `Unknown` | nothing was decided, and the reason says what stopped it. |

Only `Exact` sets `proven: true`, and only `Exact` licenses dropping a test in
[tutorial 3](03-test-selection.md).

## What it will not pretend to know

Refusals are the useful half of this operation, so they are specific:

- **A pointer parameter or a pointer return** — comparing what two versions left in
  caller-visible memory needs an object bijection that is not built. Answers `Unknown`, by name.
- **A store through a non-local address, a volatile access, inline asm, an indirect call** —
  same claim, same refusal.
- **A path that faults** — two crashes are not agreement; `Crashed` says a path faulted, not
  how, and a null dereference and a use-after-free are the same variant.
- **No solver installed** — every arithmetic identity comes back `Unknown`. Including `f`
  against a byte-identical copy of `f`: there is deliberately **no syntactic shortcut**,
  because the shortcut that blesses two identical functions is the mechanism by which some
  later almost-identical pair gets blessed.

## Side effects are compared too

```c
p (1); p (2);     vs     p (2); p (1);
```

`Differs`, with `SideEffect { index: 0 }`. The order of two extern calls is observable — C
fixes it, and reordering visible I/O is not a safe refactor. Note that the callee *names* are
identical in both versions; the arguments are what distinguish them, which is why they are
compared symbolically rather than by name.

## The witness is the smallest one

Two solver queries that differ only in argument order may legitimately return different
satisfying models, so the distinguishing input is minimized to the numerically smallest one.
That makes the answer canonical (`prove_equivalent(a, b)` and `prove_equivalent(b, a)` agree),
reproducible across runs, and more useful to read.

## What is missing

041 §1.3 wants a **replay harness** that compiles both versions and demonstrates the
divergence. It is not built. The response says so, in `blind_spots`, every time:

> `no replay harness was compiled (041 §1.3), so the divergence is chiero's semantics and has
> not been demonstrated against a compiler`

## Next

[Reading the envelope →](05-envelope.md).

*Reference: [spec 041](../specs/041-optimization-analysis.md). Worked example under test:
`crates/chiero-tool/tests/tutorials.rs::tutorial_04_prove_equivalent`.*
