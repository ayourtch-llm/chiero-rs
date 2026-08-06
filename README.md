# chiero

A symbolic C execution environment, as a Rust library. It reads real C — macros and all —
and answers questions about it that a compiler, a coverage tool and a test runner each answer
half of.

It is built to be driven by an LLM. That shapes one thing above all others: **every answer
says how much it is worth.** An empty list of findings is not "the code is clean"; it comes
back with `proven: false` and a list of what was not looked at. This is the design decision
the rest of the library is arranged around, and [why](#the-one-rule-worth-knowing-first)
is worth two minutes before anything else.

```
                   ┌──────────────┐
   .gcno/.gcda ───►│  chiero-gcov │──► what the tests actually executed
                   └──────────────┘
   C source ──────►┌──────────────┐
                   │  chiero-diff │──► what changed, and everything it reaches
                   └──────────────┘
                   ┌──────────────┐
                   │ chiero-select│──► which tests to run for this change
                   └──────────────┘
                   ┌──────────────┐
   two versions ──►│  chiero-opt  │──► are these the same function? if not, here is an input
                   └──────────────┘        where they differ
                   ┌──────────────┐
                   │  chiero-tool │──► all of the above, as JSON with its own caveats attached
                   └──────────────┘
```

## What it does today

| You want to know | Ask | Status |
|---|---|---|
| what each test executed, line by line | `chiero_gcov::ingest_native` | ✅ exact on all of VPP, both compilers |
| what a source change actually reaches | `chiero impact` | ✅ including through macros |
| which tests are worth running | `chiero select-tests` | ✅ 100% recall on a mutation gate, 65% fewer tests |
| whether a rewrite is safe | `chiero prove-equivalent` | 🟡 return values, termination, side effects |
| where the defects are | `chiero find-bugs` | 🟡 two checkers of 040 |
| where a macro came from | `chiero explain-macro` | ✅ |
| what a macro expands into, everywhere | `chiero expansion-sites` | ✅ |

Each is a command *and* a library call; `chiero --help` lists them.

**Verified against the whole of VPP** (~1.5M lines): 1,871 translation units parse and lower;
1,895 gcc `.gcno` files and 1,872 clang ones decode; line counts for 322 objects match `gcov`
exactly — 0 differences across 156,991 lines.

## Getting started

```bash
cargo build --release          # no external toolchain needed — see "no dependencies" below
cargo test --workspace
```

Then, without writing any Rust:

```console
$ ./target/release/chiero prove-equivalent before.c after.c --entry f
verdict: differs
input:
  - origin: parameter 0
    signed: -2147483648
observation:
  kind: return_value
  before_signed: -2147483648
  after_signed: 2147483647
proven — this holds for all inputs (Exact)
```

`chiero --help` lists the five operations. Every one of them prints an
[envelope](#the-one-rule-worth-knowing-first); `--json` gives the machine-readable form.

The tutorials are the fastest way in. Each is a complete worked example you can paste and run:

1. **[Reading coverage](docs/tutorials/01-coverage.md)** — turning a build's `.gcno`/`.gcda`
   files into something you can ask questions of, and the one distinction that makes the
   answers trustworthy.
2. **[What a change reaches](docs/tutorials/02-change-impact.md)** — why editing a macro in a
   header is invisible to coverage tools, and what to do about it.
3. **[Choosing tests](docs/tutorials/03-test-selection.md)** — running a fifth of your suite
   without losing a regression, and how to check that claim rather than believe it.
4. **[Adjudicating a rewrite](docs/tutorials/04-prove-equivalent.md)** — the LLM proposes,
   chiero decides. Includes the `abs()` rewrite that looks right and is not.
5. **[Reading the envelope](docs/tutorials/05-envelope.md)** — how to tell a proof from a
   guess, which is the only tutorial that is really required reading.

## The one rule worth knowing first

An LLM shown `"findings": []` will report that the code is safe. So no operation in this
library returns a bare answer. Every one returns an **envelope**:

```json
{
  "result":      { "verdict": "equivalent", "compared": ["return value", "termination"] },
  "fidelity":    "Exact",
  "proven":      true,
  "assumptions": [],
  "blind_spots": ["caller-visible memory was not compared (041 §1.1)"]
}
```

`proven` is not a field anyone sets — it is derived from `fidelity`, and there is no way to
construct an envelope where the two disagree. `fidelity: "Exact"` means *proven for all
inputs*. Anything else means the answer holds within a stated bound, and the bound is stated.

The same rule runs through the layers underneath. Coverage keeps "no record" apart from
"recorded zero". Impact analysis widens every gap rather than narrowing it. Test selection
drops a test only on an `Exact` proof. If you take one thing from this README: **an empty
answer always carries what made it empty.**

## No build dependencies, on purpose

chiero never links clang, never links an SMT solver, and builds and runs with neither
installed.

- **The C frontend is chiero's own.** Lexer, preprocessor, parser, semantics and lowering are
  in this repository. That is what makes macro-level questions answerable at all.
- **A solver is used if one is on `PATH`** (z3, cvc5, bitwuzla), discovered at *run* time and
  spoken to over a subprocess. With none installed, the built-in incomplete solver answers
  what it can and says `Unknown` for the rest — never a wrong answer, just a less useful one.

Consequences worth knowing: symbolic operations get weaker without a solver, and they say so
in the envelope rather than silently guessing.

## Determinism

Byte-identical output for identical input is a hard requirement, not an aspiration —
`HashMap` and `HashSet` are `deny`-lint-banned from every output path, and every rendering
carries a `determinism_key` you can compare across runs.

## Layout

| Crate | What it is |
|---|---|
| `chiero-lex`, `chiero-pp`, `chiero-parse`, `chiero-sema`, `chiero-lower` | the C frontend |
| `chiero-cir`, `chiero-mem`, `chiero-solver`, `chiero-exec` | the IR, memory model, solver and symbolic engine |
| `chiero-gcov` | gcov/llvm-cov artifact decoding |
| `chiero-diff`, `chiero-select` | change impact and test selection |
| `chiero-check`, `chiero-opt`, `chiero-recipe` | defect checkers, optimization analysis, conformance rules |
| `chiero-vpp` | everything VPP-specific — and nothing VPP-specific lives anywhere else |
| `chiero-tool` | the envelope and the operation surface |

## Specifications

Every crate implements a numbered specification in [`docs/specs/`](docs/specs/), and every
specification ends with a list of **testable contracts** that the test suite is written
against by number. If you want to know why something behaves the way it does, the spec says
so — usually including what the alternative was and why it was rejected.

Start with [`000-overview.md`](docs/specs/000-overview.md), or go straight to the one for the
thing you are using:
[coverage](docs/specs/030-coverage-gcov.md) ·
[change impact](docs/specs/031-change-impact.md) ·
[test selection](docs/specs/032-test-selection.md) ·
[optimization analysis](docs/specs/041-optimization-analysis.md) ·
[tool interface](docs/specs/050-tool-interface.md).

## Licence

Apache-2.0.
