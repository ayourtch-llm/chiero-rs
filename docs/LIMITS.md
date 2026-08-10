# What chiero cannot do, on one page

**Read this before pointing chiero at something that matters.** Every operation prints an
[envelope](tutorials/05-envelope.md) that qualifies its own answer, and that is the mechanism
this project trusts — but an envelope tells you about *one run*. This page is about the tool.

Everything here is either measured or a design decision on the record. Where a number appears it
came from a run somebody can repeat; the harnesses are in `tests/corpus/`.

## Supported platform

**x86-64 Linux.** That is where every differential gate is written and run.

**ARM (aarch64) runs the engine and fails the oracles.** Measured 2026-08-10 on an aarch64
machine: the workspace builds in 11 s and 1500+ tests pass, so the analysis itself is portable.
Twelve suites fail, and every one of them is a gate that compares chiero against the *local*
gcc — char signedness, 128-bit long double, predefines under `-march`, vector lanes, a
zero-width bit-field in a union, and a persona test asserting SSE4.2 unconditionally. Those are
real x86↔ARM divergences rather than defects, and nobody has done the work of teaching the
oracles which target they are on. **Do not read chiero's layout or predefine answers on ARM as
answers about ARM.**

## The tool surface

- **There is no MCP or JSON-RPC server yet.** [050](specs/050-tool-interface.md) §3 describes
  one; what exists is the `chiero` command and the `chiero_tool` library behind it, with the
  same operation set.
- **`--json` on stdout, diagnostics on stderr, and the exit status means something**: `0` the
  operation ran, `1` it could not, `2` the request was malformed (050 contracts 19–20). An
  operation that ran and found nothing exits `0` — "nothing found" is an answer.
- **Nothing is ever rewritten.** `layout` and `find-optimizations` produce proposals with
  obligations attached; applying them is yours.
- **Replay executes code, and it is off by default.** `--allow-replay-exec` compiles a witness
  and runs it, sandboxed (050 §6, contract 12). Without it you get the harness text and no
  verdict.

## What a green CI run does and does not say

CI runs the whole suite in two configurations — with a solver and without one — on every push,
and both gate. Two things it cannot cover, stated because a badge is a claim:

- **Three contracts need a VPP checkout the runner does not have**: every VPP source lexing
  without a panic, all 1967 translation units preprocessing, and the compile database parsing.
  Their tests skip there and run on a machine that has the tree.
- **The solverless leg skips 44 assertions**, by construction — they are the ones asserting what
  a *complete* solver decides, and there isn't one. The suite reports that count beside the
  passes rather than leaving both legs looking identical.

## The analysis

- **Analysis is per function, not per program.** You name an entry with `--entry`, and chiero
  enters it with unconstrained arguments. It does not know what your callers guarantee — which
  is what `--entry-ptr-nonnull` exists to tell it, and why using it is recorded as an assumption.
- **A pointer parameter's object is invented.** chiero knows neither its size nor where in it
  the pointer points, so bounds findings against it say nothing about your program. They are off
  by default (`--report-invented-bounds`); the count is always reported.
- **`prove_equivalent` compares return values, termination and side effects.** A rewrite that
  agrees on all three is called equivalent; anything outside that — timing, memory footprint,
  observable allocation behaviour — is not compared.
- **`pointer-outside-object` only fires for symbolic offsets.** A constant out-of-object pointer
  is not reported, so an empty result for that kind is not evidence of its absence.
  `crates/chiero-cli/tests/defect_vocabulary.rs` holds the full list of defect kinds with, for
  each one the corpus does not probe, the reason on the record.
- **`misaligned` is filtered.** Lowering emits `align 1` for a packed member, so reporting it
  would call ordinary legal C misaligned. It returns when there is a `ub-strict` mode.
- **The language level is C11** (`__STDC_VERSION__` `201112L`). Real builds that pass no
  `-std=` get gcc's default `gnu17`, so a header configuring itself on that macro may take a
  different branch under chiero than under your compiler. This is an open decision, not an
  oversight — see the owner-decision list in `HANDOFF.md` §9.

## Test selection

- **Coverage is historical**, so a selection is `Bounded` by construction and can never be
  `Exact`. It records what your tests did against the code as it *was*; if a source has changed
  since, the index says `Stale` and the envelope carries it.
- **Selection needs coverage attributed per test.** `--test NAME=PATH`, once per test run, or
  `--coverage-manifest`. `--coverage`/`--stem` read one object with no test name attached; an
  index built that way can select nothing, and the command refuses rather than answering
  `0 selected`.
- **The measured result is a mutation gate, not a proof**: 100% recall over 8 real mutations
  against 14.3% for a coverage-only baseline, running 65% fewer test-cases
  ([tutorial 3](tutorials/03-test-selection.md)).

## Budgets and reproducibility

- **The command line stops after 60 seconds by default** (`--time-budget`, `0` for none). A run
  that ends there is marked `nondeterministic_abort`: where it stopped depends on the machine,
  so the answer is a *measurement* rather than a fact, and it must not be cached
  (050 contract 16).
- **`--solver-rlimit` is the deterministic budget** — z3 work units, which do not move with
  machine speed. A run cut by it is an ordinary answer.
- **Without a solver on `PATH`, chiero answers what its own tier can and says `Unknown` for the
  rest** (022 contract 2). It never guesses. CI runs both configurations, so this is a supported
  way to use the tool rather than a degraded one.

## Scale

The frontend and engine were measured on a controlled size axis in
`crates/chiero-lower/tests/scale.rs`, and the growth ratios are gated. Real translation units are
another matter: a VPP TU's size is its header closure, and `find-bugs` over one can produce tens
of thousands of blocks. Use `--time-budget` and read what comes back — a run that stopped says so.
