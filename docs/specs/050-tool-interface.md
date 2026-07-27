# 050 — Tool interface

`chiero-tool` is the LLM-facing surface: an MCP server, and the same operations over
plain JSON-RPC for non-MCP callers. `chiero-cli` is a thin wrapper over the identical
operation set, so anything an LLM can do a human can reproduce on the command line — and
anything a human reports as broken, an LLM hit the same way.

## 1. The governing principle

> **The LLM proposes; chiero adjudicates.**

An LLM is good at generating candidate rewrites, candidate bug hypotheses, and prose
descriptions of coding rituals. It is bad at being certain, and worse at knowing when it
is uncertain. chiero is the opposite. So the API is shaped so that the *decidable* half
lives here:

| The LLM is good at | chiero decides |
|---|---|
| "here is a faster version of this function" | `prove_equivalent` → proof or distinguishing input |
| "this looks like it could overflow" | `find_bugs` → witness + compiled, sanitizer-checked replay |
| "the CLI ritual should free line_input" | `validate_recipe` → fixture verdict, then codebase findings |
| "this change probably doesn't affect the IP tests" | `select_tests` → ranked list with justification |

An API that returned "chiero thinks this optimization is good" would put chiero on the
wrong side of that table. Every operation below either returns evidence or returns an
explicit non-answer.

## 2. Result envelope

**Every** operation returns the same envelope. This is the single most important design
decision in the crate:

```jsonc
{
  "result":     { /* operation-specific */ },
  "fidelity":   "Exact" | "Bounded" | "Approximated" | "Unknown",
  "proven":     true,                      // ONLY ever true when fidelity == "Exact"
  "assumptions":[ {"kind":"unmodeled_extern","detail":"rte_eth_rx_burst","span":"…"} ],
  "blind_spots":[ "single-threaded execution", "floats approximated" ],
  "budgets":    { "wall_clock":"60s", "max_loop_iters":8, "hit":["max_loop_iters@ip4.c:88"] },
  "truncation": { "truncated": true, "shown": 50, "total": 1043, "cursor":"…" },
  "determinism_key": "blake3:…"
}
```

Rationale, from the failure mode this exists to prevent: an LLM reading `"findings": []`
will report "the code is safe." It must instead read
`"findings": [], "proven": false, "blind_spots": [...]` and be structurally unable to
miss the qualification. `proven` is emitted by the same `ExactWitness` token that gates
the Rust API ([023 §7](023-execution-engine.md)), so the JSON cannot disagree with the
library.

Text renderings follow the same rule: "no defects found **within** <bound>", never "no
defects found", unless `proven` is true.

## 3. Operation catalogue

### Provenance and navigation

| Operation | Returns |
|---|---|
| `explain_macro_expansion(file, line, column?)` | The full expansion chain at that point, innermost to outermost, with each macro's definition site and body. |
| `expansion_sites(macro)` | Every site where a macro expands, transitively. |
| `get_cfg(function)` | Blocks, edges, and each block's gcov lines. |
| `coverage_of(function)` | Per-line counts and the covering tests. |

`explain_macro_expansion` is the sleeper hit for VPP. Asked "what does this line actually
do", an LLM staring at `foreach_ip_interface_address(...)` is guessing; with the expansion
chain it is reading. No other tool in the ecosystem can answer it, because it requires
retaining provenance through preprocessing.

### Change analysis

| Operation | Returns |
|---|---|
| `impact_of(diff)` | `ImpactSet` with per-entity justification ([031](031-change-impact.md)). |
| `select_tests(diff, budget?)` | Ranked tests with reasons, exclusions with proofs, safety-set contents ([032](032-test-selection.md)). |

### Defect analysis

| Operation | Returns |
|---|---|
| `find_bugs(function, budget?, checkers?)` | Findings with witnesses, replay harnesses, and `ReplayVerdict` ([040](040-defect-checkers.md)). |
| `check_reachable(function, line, assumptions?)` | Reachable-with-witness, unreachable-with-proof, or unknown. |
| `symbolic_run(function, inputs?)` | Path summaries, return values, side effects. |
| `explain_finding(id)` | Full narrative, path, and macro backtrace for one finding. |

`check_reachable` is deliberately three-valued in its *shape*, not just its fidelity
field: "unreachable" and "not shown reachable within the bound" are different JSON
variants, so a caller cannot conflate them by reading one key.

### Adjudication

| Operation | Returns |
|---|---|
| `prove_equivalent(before, after, config?)` | Proof, or distinguishing input + replay ([041 §1](041-optimization-analysis.md)). |
| `find_optimizations(function)` | Proposals with obligations and benefit labels. |
| `locality_report(struct \| function)` | Cache-line findings ([041 §3](041-optimization-analysis.md)). |

### Conformance

| Operation | Returns |
|---|---|
| `propose_recipe(description, examples)` | Draft recipe text plus generated fixtures. |
| `validate_recipe(recipe)` | Load errors and fixture verdicts. **Runs first, always.** |
| `apply_recipe(recipe, scope)` | Findings, plus candidate and escalation counts. |

`apply_recipe` **refuses** a recipe that has not passed `validate_recipe`
([042 §5](042-conformance-recipes.md)). The failure this prevents is specific and likely:
an LLM writes a plausible rule that matches nothing, applies it, sees zero findings, and
reports the codebase compliant.

## 4. Designed for LLM consumption

- **Progressive disclosure.** Every list result is summarized first with a `cursor` for
  detail. `expansion_sites` on a vppinfra macro returns 1043 entries; dumping them wastes
  the context that would have been used to reason about them.
- **Truncation is explicit and counted.** `"shown": 50, "total": 1043`. Silent truncation
  reads as completeness.
- **Stable IDs.** Findings, proposals and entities have IDs stable within a session, so
  follow-up calls (`explain_finding`) do not re-transmit state.
- **Spans render as `file:line:col`** with the macro backtrace attached — the form an LLM
  can act on and a human can click.
- **Errors are structured**, with a machine `code`, a human `message`, and a `retryable`
  flag. `"could not parse ip4_forward.c:2201"` is actionable; a generic failure is not.
- **Budgets are inputs.** Callers set them; defaults are conservative; the response always
  echoes what was used and what was hit.

## 5. Long-running operations

Symbolic execution takes minutes. Blocking an MCP call for that long is not acceptable, so
operations above a threshold return a job handle:

```jsonc
{ "job": "job_7f3a", "status": "running", "progress": {"states": 4102, "findings": 3},
  "partial": { /* findings so far */ } }
```

`get_job(id)` polls, `cancel_job(id)` stops. **Cancellation returns partial results with
`fidelity: Bounded`** rather than discarding work — a cancelled 90%-complete run with
three findings is valuable, and throwing it away encourages callers to set reckless
budgets.

## 6. Safety

This surface executes code. `chiero-replay` compiles and runs a harness
([040 §3.1](040-defect-checkers.md)), and that harness is derived from the analysed
program.

- **Read-only by default.** No operation modifies the analysed repository. There is no
  patch operation, no file-write operation, and no shell passthrough.
- **Compilation and replay execution are opt-in** (`--allow-replay-exec`), run in a
  sandbox with no network, a scratch working directory, a wall-clock limit, and a memory
  cap. With the flag off, harnesses are still *emitted* — they are useful as artifacts —
  but never built or run, and `ReplayVerdict` is absent rather than fabricated.
- **The solver subprocess** is likewise bounded and killable ([022 §4](022-solver.md)).
- **Path scoping**: operations are confined to a configured project root; a request naming
  a path outside it is an error, not a warning.

## 7. Determinism and caching

Identical inputs produce identical outputs, including ordering, and `determinism_key` is
a hash of the inputs plus the configuration that produced the result. A caller can cache
on it, and a bug report that includes it is reproducible. This is
[001 §5](001-architecture.md)'s determinism requirement surfacing where it is most
visible.

## 8. Testable contracts

1. Every operation's response validates against the envelope schema, including error
   responses (checked for all operations by a schema test).
2. `proven: true` appears only when `fidelity == "Exact"` — property test over all
   operations and all corpus inputs.
3. A `find_bugs` run that hits a budget returns `proven: false`, a non-empty `budgets.hit`,
   and text containing "within"; the string "no defects found" never appears unqualified
   in any rendering (golden test over the corpus).
4. `blind_spots` is non-empty in every response from a v1 build.
5. `check_reachable` returns structurally distinct variants for unreachable-with-proof and
   not-shown-reachable; a fixture forcing each asserts they are not conflatable.
6. `explain_macro_expansion` on the `vec_add1` fixture returns the full chain with each
   macro's definition site and body text, innermost first.
7. `expansion_sites` on a macro with 1043 sites returns a summary with
   `total: 1043, shown: 50` and a working cursor; paging through yields exactly 1043
   distinct sites with no duplicates.
8. `prove_equivalent` on an LLM-style rewrite that differs at `INT_MIN` returns the
   distinguishing input and a harness that compiles.
9. `apply_recipe` on a recipe that has not passed `validate_recipe` returns a structured
   refusal, not findings.
10. `validate_recipe` on a recipe that matches nothing fails, and the failure names the
    `bad` fixture.
11. With `--allow-replay-exec` off, responses contain emitted harness text and **no**
    `ReplayVerdict`; with it on, corpus findings carry a verdict.
12. Replay execution cannot reach the network and cannot write outside the scratch
    directory (asserted by a fixture harness that attempts both).
13. An operation naming a path outside the project root returns an error.
14. No operation writes to the analysed repository — verified by hashing the tree before
    and after the full corpus run.
15. A long-running operation returns a job handle, and `cancel_job` yields partial results
    with `fidelity: Bounded` and the findings collected so far.
16. Two runs of any operation on identical input produce identical `determinism_key` and
    byte-identical results.
17. Every error response has a machine `code` and a `retryable` flag.
18. The CLI and the MCP surface expose the same operation set with the same names —
    checked mechanically, so the two cannot drift.
