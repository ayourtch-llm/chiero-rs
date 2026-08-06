# 4. Adjudicating a rewrite

**What you get:** given two versions of a function, either a proof that they agree for every
input, or a concrete input at which they do not.

**Why this operation exists:** an LLM is good at proposing a faster or clearer version of a C
function and bad at being sure it is correct. chiero is bad at inventing rewrites and good at
deciding whether two functions agree. **The LLM proposes; chiero adjudicates.**

## The rewrite that looks right

An LLM is asked to make this safer:

```c
int f (int x) { return x < 0 ? -x : x; }              /* before */
```

It notices — correctly — that `-x` overflows when `x == INT_MIN`, and proposes:

```c
int f (int x) {                                       /* after */
  if (x < 0)
    return x == INT_MIN ? INT_MAX : -x;
  return x;
}
```

Sound reasoning, defensible change, **different function**. Ask:

```console
$ chiero prove-equivalent before.c after.c --entry f
verdict: differs
input:
  - origin: parameter 0
    width: 32
    value: 2147483648
    signed: -2147483648
    pinned: true
observation:
  kind: return_value
  before: 2147483648
  before_signed: -2147483648
  after: 2147483647
  after_signed: 2147483647
  width: 32
replay: (none)
proven — this holds for all inputs (Exact)
  blind spot: no replay harness was compiled (041 §1.3), so the divergence is chiero's semantics and has not been demonstrated against a compiler
```

Add `--json` for the same thing as an envelope a program can read. From Rust:

```rust
let cfg = chiero_opt::EquivCfg::new("f");     // "f" is the function to compare, in both
let env = chiero_tool::prove_equivalent(&before, &after, &cfg);
```

One input out of 2^32. *"Your rewrite is wrong"* is an opinion; *"it returns 2147483647 where
the original returns -2147483648 when `x == INT_MIN`"* ends the discussion.

## The agreeing direction

```c
int f (int x) { return x * 2; }        /* before */
int f (int x) { return x << 1; }       /* after  */
```

```console
$ chiero prove-equivalent double.c shift.c --entry f
verdict: equivalent
compared:
  - return value
  - termination
proven — this holds for all inputs (Exact)
  blind spot: caller-visible memory was not compared (041 §1.1); the verdict is silent about it
  blind spot: side-effect sequence was not compared (041 §1.1); the verdict is silent about it
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
void p (int);

int f (int x) { p (1); p (2); return x; }      /* before */
int f (int x) { p (2); p (1); return x; }      /* after  */
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

## Checking chiero against a real compiler

Everything above rests on chiero's C semantics being right. `--allow-replay-exec` is the one
thing that does not:

```console
$ chiero prove-equivalent before.c after.c --entry f --allow-replay-exec
...
  outcome: demonstrated
  before: -2147483648
  after: 2147483647
proven — this holds for all inputs (Exact)
```

chiero emits a self-contained C program that includes both versions with the entry renamed,
calls each at the witness, and **exits 0 only when they disagree** — so "it ran and said
nothing" cannot be mistaken for success. `gcc` built it, ran it, and produced the two numbers.
The blind spot about the divergence being unconfirmed is gone from that output, because it is
no longer true.

`--replay` alone emits the program without running it: compiling and executing code is
something a caller has to ask for (050 §6). With it, the response carries the source and a
**null** outcome, and the envelope says nobody ran it — "not run" and "ran and said nothing"
are different facts.

### The outcome that matters most is the one that disagrees

| outcome | what it means |
|---|---|
| `demonstrated` | a compiler agrees. The only one that confirms anything. |
| `not_demonstrated` | it built, it ran, and the two versions **agreed**. chiero and the compiler disagree; the finding is downgraded to `Approximated` and both claims are shown. |
| `did_not_build` | about the harness, not about chiero — most often two versions sharing a definition, since including both puts it in one translation unit twice. |
| `did_not_run` | the program faulted, which is not the same news as the versions agreeing. |

### What the harness cannot do yet

Scalar parameters only. 040 §3's construction rules want memory objects materialized as
initialized byte arrays with the engine's own pointer layout, unmodeled extern calls stubbed
to return the values the engine chose in call order, and the translation unit's own
`compile_commands.json` flags. A function taking a pointer is refused by `prove-equivalent`
before it gets this far, so the gap is a limit on what can be adjudicated at all rather than a
harness that quietly skips something.

## Next

[Reading the envelope →](05-envelope.md).

## Headers and defines

The command takes the same `-I` and `-D` a compiler does, repeatable:

```console
$ chiero prove-equivalent old.c new.c --entry ip4_lookup \
      -I src -I build-root/install/include -DCLIB_DEBUG=1
```

Everything the frontend refuses is an error rather than an empty answer: a file that does not
preprocess was never seen, one that does not parse was seen and not understood, and a function
lowering gives up on is *absent from the module* — so answering about it would be answering
about a translation unit with a hole in it.

*Reference: [spec 041](../specs/041-optimization-analysis.md). Worked example under test:
`crates/chiero-tool/tests/tutorials.rs::tutorial_04_prove_equivalent`.*
