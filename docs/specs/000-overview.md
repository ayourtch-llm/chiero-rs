# 000 — Overview

## 1. What chiero is

A **library** for symbolically executing C, plus the analyses that become possible once
you can. Not a monolithic tool with a symbolic engine buried inside it: a set of crates
with published contracts, each usable on its own.

The design pressure comes from one target: VPP. Roughly 1M lines of C, 754 distinct
`foreach_*` X-macros, 340 `VLIB_REGISTER_NODE` sites, and a data-plane idiom
(`vec_*`, `pool_*`, `vlib_buffer_t` index arithmetic) that most C analysis tooling
handles badly or not at all. If chiero works on VPP it works.

## 2. The four capabilities

1. **Symbolic execution of C, macro-aware.** Start at an arbitrary function, not `main`.
   Explore paths, produce path conditions, produce concrete inputs.
2. **Coverage ingest and change-driven test selection.** Read gcov data, map a diff to
   the tests that can observe it, justify each selection.
3. **Logic-error detection.** Find defects with a concrete, replayable counterexample —
   not a heuristic warning.
4. **Optimization analysis, LLM-usable.** Expose the engine as a tool an LLM can call to
   *check its own proposals*, rather than as a thing that emits suggestions at an LLM.

Capability 4 is the one that changes the shape of the project. The valuable primitive is
not "chiero suggests an optimization" — it is `prove_equivalent(before, after)`, which
returns either a proof or a distinguishing input. An LLM proposes; chiero adjudicates.
Every optimization and refactoring feature is built on that primitive.

## 3. Design commitments

**Provenance is first-class, not a debugging aid.** Every token, AST node, IR
instruction and constraint traces back through the full macro expansion tree to a byte
range in an original file. This is why chiero owns its preprocessor
([010](010-source-and-provenance.md), [012](012-preprocessor.md)).

The concrete payoff: when someone edits the body of `vec_add1` in `vppinfra/vec.h`,
gcov attributes no line to `vec.h` — coverage data records the `.c` file line where the
macro was *used*. A coverage-only tool cannot tell you which tests are affected. chiero
can, because it knows every expansion site of every macro.

⚠️ **This section used to end "and that is why the frontend is written rather than borrowed",
which is not true and was corrected once it was tested.** clang 18.1.3 does supply the nesting
chain and per-token argument attribution; [010 §1.1](010-source-and-provenance.md) records the
measurement and the honest reason, which is [001 §5](001-architecture.md)'s no-external-
toolchain constraint: a *core* capability behind libclang forfeits the pure-Rust,
links-nothing property the library is specified around. The macro capability is still the
differentiator; it is not the argument for owning the frontend.

**Incompleteness is declared, never hidden.** Symbolic execution of real C hits
unbounded loops, external calls, symbolic pointers, and solver timeouts. Every result
carries a fidelity annotation ([023 §7](023-execution-engine.md)). "No bug found" from a
truncated exploration must never be reported as "no bug exists."

**Under-constrained by default.** Whole-program execution from `main` is useless for a
1M-line packet processor. chiero starts at any function with unconstrained inputs and
lazily materializes memory on first dereference. This trades false positives for reach,
and the checkers ([040](040-defect-checkers.md)) are designed around that trade.

**The IR is a contract boundary.** CIR ([020](020-cir.md)) is specified independently of
the frontend, so the entire symbolic core can be built and tested against hand-written
CIR before the C parser exists. This is what makes the chosen build order — symbolic
core first — actually tractable.

**Real compilers are the oracle.** gcc 13.3.0, clang 18.1.3 and z3 4.8.12 are all
installed and verified. The primary correctness test for the engine is differential: run a C program
under gcc with concrete inputs, run it under chiero with the same inputs concretized,
compare. Layout and constant evaluation are checked by generating `_Static_assert`s and
compiling them ([014 §7](014-semantics-and-types.md)); the preprocessor is checked
against `gcc -E`/`clang -E`; solver answers are cross-checked against z3.

chiero **never links** clang or z3, and builds and runs fully without them. They are
test oracles and optional accelerators, reached through subprocess boundaries or
off-by-default features ([001 §5](001-architecture.md), [022](022-solver.md)). This is a
deliberate constraint: the moment a core capability requires an external toolchain, the
"modular and reusable library" property is gone. See
[070](070-testing-and-tdd-protocol.md).

## 4. Non-goals

- **Soundness as a verifier.** chiero is a bug-finder and a decision-support tool. It
  does not certify absence of defects.
- **Full C23, or any compiler's complete dialect.** The target is C11 plus the GNU
  extensions VPP actually uses, enumerated in [013 §4](013-parser.md).
- **C++.** Not now, and no design accommodation for it.
- **Being a compiler.** CIR is shaped for symbolic execution, not code generation. No
  backend, no register allocation.
- **Replacing the build system.** chiero consumes `compile_commands.json`; it does not
  build VPP.
- **Automatic patching.** Optimization analysis emits proposals with proof obligations.
  Applying them is a human or LLM decision.
- **Interleaving exploration, weak memory, and lock-free verification.** Execution is
  sequential and sequentially consistent. A race whose only symptom is a torn or stale
  read will not be found, and this is declared in every report.

  "No concurrency support" would be the wrong summary, though: VPP *partitions* rather
  than shares — 467 files index by `thread_index` while only ~70 use explicit
  synchronization — which makes a path-sensitive **discipline** checker tractable where
  interleaving exploration is not. v1 ships one, finding unguarded shared writes, lock
  leaks on error-return paths, lock-order inversions and missing barriers around
  config-time mutation. Any result touching shared state is held to
  `Fidelity ≥ Bounded`. See [025](025-concurrency-and-threading.md).

## 5. Glossary

| Term | Meaning |
|---|---|
| **Expansion** | One macro invocation, with its call site, arguments, and parent |
| **`ExpnCtx`** | Identifier for a position in the expansion tree; `ROOT` means "written directly in a file" |
| **Spelling location** | Where a token's text literally appears (possibly inside a macro body) |
| **Expansion location** | The outermost source position that produced a token — what gcov sees |
| **CIR** | chiero IR: unstructured CFG, three-address, alloca/load/store, fully provenanced |
| **UCSE** | Under-constrained symbolic execution: start mid-program, lazily initialize inputs |
| **Path condition** | Conjunction of branch constraints along one explored path |
| **Fidelity** | Declared trustworthiness of a result: `Exact`, `Bounded`, `Approximated`, `Unknown` |
| **Impact closure** | Set of program entities transitively affected by a change |
| **TU** | Translation unit. Note VPP compiles some sources into *several* TUs via multiarch |

## 6. Repository layout

```
chiero-rs/
├── crates/           one directory per crate, see 001
├── docs/specs/       these documents
├── tests/
│   ├── corpus/       C programs + expected results, shared across crates
│   └── differential/ gcc-vs-chiero harness
└── xtask/            build/bench/corpus automation
```
