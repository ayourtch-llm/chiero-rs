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
| whether chiero is right about it | `--allow-replay-exec` | 🟡 a C harness a real compiler runs, in a network namespace with a memory cap |
| where the defects are | `chiero find-bugs` | 🟡 two checkers of 040 |
| whether execution can reach a line | `chiero check-reachable` | ✅ and "nothing gets there" is a different answer from "I did not" |
| what a rewrite could gain | `chiero find-optimizations` | 🟡 dead branches, redundant loads, dead stores — proposals only |
| what a struct's padding costs | `chiero layout` | ✅ with an obligation saying whether reordering it is even allowed |
| where a macro came from | `chiero explain-macro` | ✅ |
| what a macro expands into, everywhere | `chiero expansion-sites` | ✅ |

Each is a command *and* a library call; `chiero --help` lists all nine.

**Verified against the whole of VPP** (~1.5M lines): 1,871 translation units parse and lower
with **zero** refusals — the three remaining diagnostics are VPP's own ISO C divergences, which
gcc accepts under `gnu11` and rejects under `-pedantic-errors`. 1,895 gcc `.gcno` files and
1,872 clang ones decode; line counts for 322 objects match `gcov` exactly — 0 differences
across 156,991 lines.

`find-bugs` has been run this way over hundreds of VPP entry points — most recently 477 across
92 plugins, which turned up two engine crashes and one true `Exact` finding. The harness is
checked in at `tests/corpus/vpp-findings/`, so the numbers are somebody else's to check rather
than mine to assert.

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

`chiero --help` lists all nine operations. Every one of them prints an
[envelope](#the-one-rule-worth-knowing-first); `--json` gives the machine-readable form.

Three flags are worth knowing before pointing `find-bugs` at real code, because they are the
difference between an answer and a wall of noise — measured on VPP, they took one run from 231
findings to 1:

| | |
|---|---|
| `--time-budget <secs>` | stop after that long and print what was found, rather than being killed with nothing to show. Default 60 s; `0` means none. |
| `--entry-ptr-nonnull` | the entry's pointer parameters are not null. Removes real paths, so the envelope records it. |
| `--report-invented-bounds` | show accesses that cross a bound *chiero* invented behind an entry pointer. Off by default; the count is always reported. |

The [tutorials](docs/tutorials/) are the fastest way in. Each is a complete worked example you
can paste and run, and every transcript on those pages came from a real run:

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
6. **[Finding defects](docs/tutorials/06-find-bugs.md)** — the checkers, the input that
   reaches each defect, and why the part of the answer that is not a finding matters more.
7. **[What the code can and cannot reach](docs/tutorials/07-reachability.md)** — dead branches
   and unreachable lines, and the difference between "nothing gets here" and "I did not".
8. **[Struct layout](docs/tutorials/08-layout.md)** — padding and cache lines, and why every
   proposal says whether reordering that struct is allowed at all.

## The one rule worth knowing first

An LLM shown `"findings": []` will report that the code is safe. So no operation in this
library returns a bare answer. Every one returns an **envelope**:

```json
{
  "result":      { "verdict": "equivalent", "compared": ["return value", "termination"] },
  "fidelity":    "Exact",
  "proven":      true,
  "assumptions": [],
  "blind_spots": ["caller-visible memory was not compared (041 §1.1)"],
  "nondeterministic_abort": false,
  "truncation":  { "truncated": false },
  "determinism_key": "fnv128:…"
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

**One thing in the system is a measurement rather than a computation, and it says so.** A run
stopped by `--time-budget` ends where the machine's speed put it, so its envelope carries
`nondeterministic_abort: true` and a consumer that caches answers must not cache that one.
Everything else — every verdict, every witness, every rendering — is reproducible, and the
library's own default is no clock at all.

## Layout

| Crate | What it is |
|---|---|
| `chiero-span` | spans, the source map, and the macro expansion tree everything else hangs from |
| `chiero-lex`, `chiero-pp`, `chiero-ast`, `chiero-parse`, `chiero-sema`, `chiero-lower` | the C frontend |
| `chiero-cir`, `chiero-mem`, `chiero-solver`, `chiero-exec`, `chiero-model` | the IR, memory model, solver, symbolic engine and the libc/builtin models |
| `chiero-gcov` | gcov/llvm-cov artifact decoding |
| `chiero-diff`, `chiero-select` | change impact and test selection |
| `chiero-check`, `chiero-opt`, `chiero-recipe` | defect checkers, optimization analysis, conformance rules |
| `chiero-replay` | the C harness that checks a finding against a real compiler |
| `chiero-vpp` | everything VPP-specific — and nothing VPP-specific lives anywhere else |
| `chiero-tool` | the envelope and the operation surface |
| `chiero-cli` | the `chiero` command — a thin wrapper that decides nothing of its own |

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
