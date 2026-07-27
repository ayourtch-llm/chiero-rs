# 040 — Defect checkers

`chiero-check` implements the concrete `Checker`s ([023 §6](023-execution-engine.md)), the
report types, triage, and `chiero-replay` — the component that turns a symbolic finding
into a **compilable C program that reproduces it**.

The replay harness is the point. An LLM can produce a plausible list of suspected bugs in
any C function; nobody can act on it, because the cost of triaging a plausible-but-wrong
report exceeds the value of a real one. chiero's claim is different in kind: *here is the
input, here is the program that demonstrates it, and here is what the sanitizer said when
we ran it.*

## 1. Checker catalogue

Memory safety (faults surfaced by [021 §5](021-memory-model.md)):
null dereference; out-of-bounds read/write; use-after-free; double free; use-after-scope;
invalid free (non-heap or interior pointer); memory leak; wild pointer via `IntToPtr`;
`memcpy` with overlapping ranges; uninitialized read.

Arithmetic and UB ([020 §4.1](020-cir.md) emits the events):
signed overflow; division by zero; `INT_MIN / -1`; shift by ≥ width or negative;
invalid pointer arithmetic (leaving an object's bounds); pointer comparison across
objects; `__builtin_clz(0)` and friends; lossy implicit conversion that changes value;
misaligned access (in `ub-strict` only).

Control and API:
assertion failure (`ASSERT`, `__assert_fail`, `clib_error` paths); reaching
`__builtin_unreachable` on a feasible path; unreachable code; infinite loop with no
side effects; unchecked error return; format string / argument mismatch;
order-dependence ([020 §7](020-cir.md)).

Off by default, opt-in: `union-pun` ([020 §4.5](020-cir.md)) — gcc defines it and VPP
depends on it, so it is noise here; strict-aliasing violation, for the same reason.

Delegated, not duplicated: the concurrency-discipline findings are specified in
[025 §3](025-concurrency-and-threading.md) and implemented here as `Checker`s, and must
stay target-agnostic (025 contracts 14–15). Conformance-recipe findings belong to
[042](042-conformance-recipes.md).

VPP-specific checkers (`vec_` bounds, `pool_` index validity, buffer-index misuse) are
**not in this crate**. They are registered from `chiero-vpp`
([001 §4](001-architecture.md) rule 4) through the same `Checker` trait.

## 2. Findings

```rust
pub struct Report {
    pub findings: Vec<Finding>,        // triaged, ranked
    pub fidelity: Fidelity,
    pub assumptions: Vec<Assumption>,
    pub blind_spots: Vec<BlindSpot>,   // always non-empty: v1 has known ones
}

/// An engine `Finding` (023 §9) plus everything triage adds. This crate does **not**
/// redefine `Finding`; it wraps it.
pub struct TriagedFinding {
    pub base: chiero_exec::Finding,    // kind, span, backtrace, trace, witness, object, fidelity
    pub severity: Severity,
    pub confidence: Confidence,
    pub replay: Option<Replay>,        // §3
    pub narrative: Vec<NarrativeStep>, // the path, in source terms
    pub sites: Vec<Loc>,               // other expansion sites of the same macro-body bug
}
```

**One `Finding` type, defined in the core.** An earlier draft declared a second,
field-incompatible `Finding` here, which was not merely duplication: `Action::Report`
([023 §6](023-execution-engine.md)) puts the type in `chiero-exec`, and
[001 §4](001-architecture.md) rule 2 forbids the core from depending on a vertical — so a
`Finding` owned by `chiero-check` could not be named by the engine that produces it. The
enrichment direction (core defines, vertical wraps) is the only one the layering permits.

`backtrace` carries the macro expansion chain, so a bug inside `vec_add1` reports both
the expansion site in the `.c` file and the offending line in `vec.h` — the same
provenance machinery as everywhere else. For VPP this is the difference between a report
a maintainer can act on and one that points at a line of macro soup.

`narrative` renders the path as source-level steps ("assumed `n > 4` at line 88; entered
the loop; on iteration 5, `i` is 5 and `v` has 5 elements") rather than a block trace.

`blind_spots` is **always populated** — single-threaded execution, no weak memory, floats
approximated, whatever budgets bound the run. An empty blind-spot list would itself be a
false claim.

## 3. Replay harness

For each finding, `chiero-replay` emits a self-contained C program:

```c
/* chiero replay: OOB write, vec.h:143 via ip4_forward.c:900
   witness: n = 5, v = <object 3, 40 bytes> */
#include <stdint.h>
#include <string.h>
#include "chiero_replay.h"
static uint8_t obj3[40] __attribute__((aligned(8))) = { 0x05,0x00, /* … */ };
extern int target_fn (void *v, int n);
int main (void) {
  void *v = obj3 + 8;                 /* user pointer at element 0 (021 §2) */
  int r = target_fn (v, 5);
  return r;
}
```

Construction rules:

- Every symbolic input is materialized from the `Witness`: parameters as literals, memory
  objects as initialized byte arrays, lazily-created objects as static buffers with the
  **same internal pointer layout** the engine assigned.
- Pointer relationships are reconstructed explicitly (offsets into the same buffer, or
  distinct buffers for objects the engine kept distinct), so the aliasing the analysis
  assumed is the aliasing the replay has.
- Unmodeled extern calls become stubs returning the values the engine chose, in call
  order.
- The harness compiles using the `compile_commands.json` flags for that TU
  ([060](060-vpp-integration.md)) so layout, `-D` flags and `march` variant match. A
  harness compiled with different flags can reproduce a different program.

### 3.1 Reaching the target — `static` is the common case, not the exception

An `extern` declaration plus separate compilation only works for externally-linked
functions, and chiero's analysis targets in VPP are overwhelmingly **not** that: `static`
helpers, `static inline` functions in headers, and `march`-suffixed variants. A harness
template that assumes external linkage would pass the corpus and fail on VPP's first real
finding, so the mechanism is specified now:

**The harness `#include`s the target `.c` file** and is compiled as a single TU with the
original flags. That makes every `static` function reachable and every `static inline`
header function instantiable, with layout and macro configuration identical by
construction.

The sharp edges, each handled explicitly rather than discovered during M5:

| Hazard | Handling |
|---|---|
| The TU has its own `main` | Harness entry is `#define main chiero_orig_main` before the include, and defines its own `main`. |
| `__attribute__((constructor))` runs first (51 VPP files) | Allowed to run; the harness records that it did, since suppressing it would change the state the finding depends on. |
| Static state persists between the constructor and the target call | The witness includes the values of any `static` the path read, restored before the call. |
| The target is `static inline` in a header | The harness includes the header and calls it directly; no `.c` include needed. |
| The finding is inside a macro body | The harness calls the *expansion site*'s enclosing function, not the macro. |

`-Dstatic=` is explicitly rejected: it changes linkage across the whole TU, which can
change behaviour (duplicate symbols, different inlining) and would make the harness a
different program from the one analysed.

### 3.1 Self-validation — the part that matters

The harness is not just emitted; it is **compiled and run under sanitizers**, and the
result is recorded on the finding:

```rust
pub enum ReplayVerdict {
    Confirmed { sanitizer: Sanitizer, message: String },   // ASan/UBSan agreed
    NotReproduced,                                          // ran clean
    BuildFailed(String),
    Inconclusive(String),                                   // timeout, crash without diagnosis
}
```

`gcc -fsanitize=address,undefined` and, independently, `clang` with the same — both are
installed and verified ([022](022-solver.md) preamble). ASan confirming an out-of-bounds
write at the predicted line is independent evidence from a completely different
implementation.

The consequences are asymmetric and deliberate:

- **`Confirmed`** raises `Confidence` to the top and the report leads with the sanitizer's
  own message. This is the strongest artifact chiero produces.
- **`NotReproduced`** does *not* delete the finding — the harness may have failed to
  recreate the environment, and for leaks or logic errors there may be no sanitizer that
  detects it at all. It lowers `Confidence`, is flagged for review, and the reason is
  recorded. Silently discarding these would hide genuine bugs behind harness bugs.
- A **rising `NotReproduced` rate is a defect in chiero**, tracked as a quality metric in
  CI, not tolerated as background noise.

This loop is also the strongest test of the engine itself: it closes the gap between
"the solver said this input reaches the bug" and "this input reaches the bug".

## 4. Triage

**Deduplication** by `(kind, expansion_loc, object origin, checker)` —
`expansion_loc`, not `spelling_loc`, or every expansion site of one buggy macro becomes a
separate finding. The macro-body location is reported *once*, listing its expansion sites,
which is the actionable grouping: one fix, not 1000 reports.

**Ranking** by severity × confidence × reachability, with `Confirmed` replays first.

**Suppression** follows [042 §6](042-conformance-recipes.md): `/* chiero:allow(kind)
reason: … */` with a mandatory reason, plus a baseline for adoption on an existing
codebase. A stale suppression is reported.

## 5. The negative-result rule

Restating [023 §7](023-execution-engine.md) because this is the crate that renders text a
human or an LLM will read as a safety claim:

> A report may say **"no defects found"** only when `Fidelity == Exact`. Otherwise it says
> "no defects found within <explicit bound>", lists the bounds and assumptions, and the
> API returns `NotProven`.

The proof-carrying type is unconstructible without the engine's `ExactWitness` token, so
this is enforced by the compiler rather than by reviewer discipline.

## 6. Testable contracts

1. Each checker in §1 has ≥ 1 positive fixture (fires, exactly once, at the right span)
   and ≥ 1 negative fixture (does not fire) in `tests/corpus/checkers/`.
2. A checker firing on its negative fixture fails CI naming the checker.
3. Every finding in the corpus has a `Witness`, or an explicit recorded reason for none.
4. **Every finding in the corpus emits a replay harness that compiles** under the TU's own
   flags. A harness that fails to build fails CI.
4b. A finding in a **`static`** function, and one in a **`static inline`** header
    function, each emit a harness that compiles and runs (§3.1). The corpus contains both,
    because a corpus of only externally-linked functions would pass contract 4 and tell us
    nothing about VPP.
4c. A harness for a TU that defines its own `main` compiles and runs the harness's `main`.
4d. A finding whose path depends on a value set by a `constructor` function reproduces
    with that constructor having run.
5. For every memory-safety finding in the corpus with a sanitizer that can detect it, the
   replay is `Confirmed` by ASan or UBSan at the predicted line.
6. A `NotReproduced` replay lowers confidence, keeps the finding, records the reason, and
   increments the tracked metric — it never silently drops it.
7. The replay for an OOB on a vppinfra-style vector reconstructs the user pointer at the
   element offset with the header intact, and ASan confirms the overflow.
8. Objects the engine kept distinct are distinct buffers in the harness; objects it
   aliased are the same buffer at the right offsets (verified by an aliasing fixture).
9. Extern stubs return the engine's chosen values in call order, verified by a fixture
   whose outcome depends on the second call returning a different value from the first.
10. A bug inside a macro body reports the macro's location, lists the expansion sites, and
    appears **once** — not once per expansion site.
11. Deduplication uses `expansion_loc`: a fixture with one buggy macro used 50 times
    yields one finding with 50 sites.
12. A signed-overflow finding names the exact operator span and the operand values from
    the witness.
13. An uninitialized-read finding distinguishes uninitialized from symbolic-but-known
    ([021 §6](021-memory-model.md)); a lazily-initialized parameter produces no finding.
14. `union-pun` and strict-aliasing checkers are absent from the default set; enabling
    them on a punning fixture produces findings and the default run produces none.
15. A report with `Fidelity != Exact` renders "no defects found within <bound>" and never
    "no defects found"; asserted on the rendered text.
16. `blind_spots` is non-empty in every report produced by v1.
17. `grep -rE 'vec_|pool_|vlib_|clib_' crates/chiero-check/src` yields no hits.
18. Findings, their order, and the emitted harness bytes are identical across runs.
19. A finding's `narrative` references only real source lines, and each step's span
    resolves through the `SourceMap` (checked structurally for every corpus finding).
20. Running the full checker set over the corpus produces zero findings on the
    known-clean subset — the false-positive gate.
