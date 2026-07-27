# 060 — VPP integration

`chiero-vpp` is the **only** crate that knows VPP exists. Everything below plugs into
extension points the core already defines: models via
[024 §8](024-environment-models.md), checkers via [023 §6](023-execution-engine.md),
recipes as data via [042 §8](042-conformance-recipes.md). If any of it leaks downward, the
"modular reusable library" requirement is gone ([001 §4](001-architecture.md) rule 4).

Scale, measured at `/home/ubuntu/vpp` @ `7fe9c26`: 1552 `.c`, 924 `.h`, ~1.01M lines.

## 1. Build ingest

`cmake -DCMAKE_EXPORT_COMPILE_COMMANDS=ON` produces `compile_commands.json`, giving each
TU its exact `-I`, `-D`, `-std` and warning flags. Those flags are not optional detail:
they determine the `ConfigId` ([012 §3.3](012-preprocessor.md)), which determines which
`#if` branches exist, which determines layout, which determines every offset in the
analysis.

**The VPP tree at `/home/ubuntu/vpp` is not built yet** — no `compile_commands.json`
exists — so building it is a prerequisite of the VPP milestone, not something to discover
mid-implementation.

```rust
pub struct BuildDb { units: Vec<TranslationUnit> }
pub struct TranslationUnit { pub src: PathBuf, pub dir: PathBuf, pub args: Vec<String>,
                             pub config: ConfigId, pub march: Option<Symbol>,
                             pub object: PathBuf }
```

### 1.1 Multiarch — one source, many TUs

VPP compiles the same source repeatedly with different `CLIB_MARCH_VARIANT` (107 files
reference it; `CLIB_MARCH_FN` appears 54 times; `VLIB_NODE_FN` 568 times across 204
files) so the dispatcher can select an AVX-512 or NEON implementation at runtime.

Consequences, each of which breaks a naive assumption:

- **The source→TU mapping is 1:N, not 1:1.** Every index keyed by file path is wrong.
- Coverage entities are keyed by `(file, name, start_line, march)`
  ([030 §5](030-coverage-gcov.md)); merging variants attributes one variant's tests to
  another's code.
- Impact analysis treats variants as distinct entities
  ([031 §1](031-change-impact.md)) — one source edit impacts all of them.
- A finding must name its variant, because a bug present only under the AVX-512 variant
  is a different bug report from one in the scalar path.

## 2. vppinfra models

Registered into `ModelRegistry`. The counts below are why these specific APIs and not
others:

| API | Files / uses | Model |
|---|---|---|
| `vec_*` (`vec_add1` 1551, `vec_validate` 1382, `vec_resize` 118) | 448 / — | Real code by default (§2.1) |
| `pool_*` (`pool_elt_at_index` 1649) | 461 | Real code + index-validity checker |
| `clib_mem_alloc` | 153 / 288 | Heap object, **`alloc_may_fail = false`** — it aborts rather than returning NULL, unlike `malloc` ([024 §3](024-environment-models.md)) |
| `clib_memcpy_fast` | 266 / 1146 | `CopyMem`, exact |
| `clib_bitmap_*` | 134 / 792 | Real code |
| `vlib_get_buffer` / `vlib_buffer_t` | 388 / 540 | Buffer-index model (§4) |
| `unformat*` | 695 / **16275** | Modeled: symbolic parse results, since executing the real parser explodes paths |
| `clib_error_*` | 430 | Heap-allocated error object, tracked for leaks |
| `CLIB_PREFETCH` | — | No-op, pointer still range-checked |

### 2.1 Vectors: execute the real code

The default is to **execute vppinfra's actual implementation**, not a summary model.
Three reasons: the bugs are frequently *in* the vector code's interaction with caller
assumptions; the memory model was designed for exactly this layout
([021 §2](021-memory-model.md) — the header at a negative offset from the user pointer);
and a summary model that disagrees with the real implementation produces findings that
do not reproduce.

Summary models exist behind `--vec-summary` for scaling, and switching them on sets
`Fidelity::Approximated`. A divergence test compares findings with and without them over
the corpus, so the summary models are held to the real code's behaviour rather than
drifting.

`vec_resize` reallocating is the interesting case: the model must make every surviving
copy of the old pointer dangling ([021 §4](021-memory-model.md)), which is a real and
frequent VPP bug class — a worker holding a pointer across a resize.

## 3. The `foreach_*` X-macro idiom

`foreach_` appears **3720** times across 910 files; there are 754 distinct
`foreach_*` macros. This is VPP's dominant abstraction: a list macro is defined once and
expanded with different per-item macros to generate enums, string tables, registration
records and dispatch code.

The preprocessor handles them with no special support — they are ordinary macros, and
`chiero-pp` expands them with full provenance. What `chiero-vpp` adds is *recognition*:

- Identify the X-macro pattern (a macro whose body is a sequence of invocations of one
  parameter) and expose the item list, so `explain_macro_expansion`
  ([050 §3](050-tool-interface.md)) can say "this generated 47 entries, one per error
  code" instead of dumping expanded soup.
- Attribute a generated entity back to its list item, so editing one line of a
  `foreach_` list impacts exactly the entities that line generated — not all 47.

That last point is a precision win available to nothing that works on preprocessed text.

## 4. The graph: nodes, dispatch, and entry points

340 `VLIB_REGISTER_NODE` sites. Node functions have a fixed signature and are invoked
**indirectly** through registration tables, which is exactly the case
[031 §3.4](031-change-impact.md) handles with address-taken conservatism — every
signature-compatible function becomes a potential callee, which for VPP means all 340.

`chiero-vpp` narrows it by reading the registration tables: a node's function is reached
from the graph, and the `next_nodes` arcs give the real successor set. This turns a
340-way over-approximation into the actual graph, and it is the single largest precision
improvement available for VPP.

**Entry points** — you cannot reach VPP internals from `main`, which is why UCSE exists
([021 §6](021-memory-model.md)). `chiero-vpp` enumerates the sensible starting points:
node dispatch functions, CLI command handlers (1187 `VLIB_CLI_COMMAND` registrations),
binary API handlers, and vppinfra unit tests. Each comes with a **frame model**: a node
function analysed with a symbolic `vlib_frame_t` of *k* buffer indices, each pointing at
a symbolic `vlib_buffer_t` with plausible constraints on `current_data`/`current_length`.
Getting these preconditions right is what separates useful findings from a flood of
"NULL passed to everything" noise, so the frame model is data, versioned, and reviewable.

## 5. Threading

`chiero-vpp` supplies what [025](025-concurrency-and-threading.md) needs and cannot know
itself: lock/unlock models for `clib_spinlock_t` (69 files) and `clib_rwlock_t` (10);
barrier models for `vlib_worker_thread_barrier_sync`/`_release` (54 files, 112 uses)
driving `ThreadCtx::Barrier`; `clib_atomic_*` (73 files); and the per-thread-index
convention (`thread_index` in 467 files) that lets the discipline checker classify
`vlib_mains[thread_index]`-style access as `PerThread`.

Node functions default to `ThreadCtx::Worker { index: symbolic }`; CLI and API handlers to
`ThreadCtx::Main`.

## 6. Recipe catalogue

Shipped as `.recipe` data ([042 §4.3](042-conformance-recipes.md)), each with fixtures:
CLI `unformat_free` on all paths; `vec_free` of CLI-local vectors on all paths;
`pool_get`/`pool_put` pairing; `clib_error_return` results returned or freed (3350 sites);
`VLIB_CLI_COMMAND` callback signature conformance (1187 sites); barrier pairing;
`VLIB_NODE_FN` returning `frame->n_vectors`; forbidden raw `malloc`/`strcpy`.

## 7. Staged adoption

Ordered so each stage produces a usable result and de-risks the next:

1. **vppinfra alone** — `vec.h`, `pool.h`, `bitmap.h` and their unit tests. Small, header-
   heavy, macro-saturated: the best possible exercise of the frontend and the memory
   model, with real tests as the oracle.
2. **Recipe tier-1 sweep over the whole tree** — needs only the frontend, delivers
   findings across all 1552 files, and validates parser coverage at scale early.
3. **`explain_macro_expansion` over the whole tree** — same prerequisite, immediately
   useful, and the clearest demonstration of the provenance thesis.
4. **One node end to end** — `ip4-lookup` or similar: frame model, symbolic execution,
   findings with replay harnesses.
5. **Coverage + selection** on VPP's own test suite, measured against
   [032 §6](032-test-selection.md)'s safety and reduction metrics.

Parser coverage is reported as a first-class number throughout: **what fraction of VPP's
TUs parse cleanly**, tracked per commit. A frontend that handles 95% of VPP is useful; one
that silently skips 30% is not, and only measurement distinguishes them.

## 8. Testable contracts

1. `compile_commands.json` from a real VPP build parses, and every TU yields a `ConfigId`
   and a resolved include path set.
2. A source compiled under 3 `CLIB_MARCH_VARIANT`s yields 3 `TranslationUnit`s with
   distinct `march`, and no index keyed on path alone collapses them.
3. A finding in a multiarch function names its variant.
4. `clib_mem_alloc` produces exactly one state (no NULL branch); `malloc` in the same
   program produces two.
5. `vec_add1` on a vector at capacity reallocates, and a retained copy of the old pointer
   is exactly one use-after-free finding.
6. `_vec_len(v)` reads the header at the negative offset and returns the stored length
   (the [021](021-memory-model.md) contract, exercised through real vppinfra source).
7. An OOB write one element past a vector's end is detected with a witness, and the replay
   harness is ASan-confirmed.
8. `--vec-summary` sets `Approximated`, and the divergence test reports zero
   finding-set differences against real-code execution over the corpus.
9. A `foreach_*` X-macro with 47 items is recognized, its item list exposed, and editing
   one item impacts only the entities that item generated.
10. `explain_macro_expansion` on a `foreach_`-generated line names the list macro, the
    per-item macro, and the specific item.
11. Node registration tables are read, and the callee set for a node dispatch is the
    graph's actual successors rather than all 340 signature-compatible functions;
    both counts are reported so the improvement is visible.
12. A node function analysed with the frame model produces no NULL-dereference findings on
    the frame or buffer pointers (the precondition model is doing its job).
13. `spinlock_lock`/`unlock` update the lock set, and the
    [025](025-concurrency-and-threading.md) discipline checker's contracts pass against
    real VPP locking code.
14. `barrier_sync`/`_release` set and clear `ThreadCtx::Barrier`.
15. Every shipped `.recipe` passes its fixtures (the [042](042-conformance-recipes.md)
    contract 4 gate, applied to the VPP catalogue).
16. `grep -rE 'vlib_|vnet_|clib_|vec_add1|pool_get' crates/ --include=*.rs` yields hits
    only under `crates/chiero-vpp/` (and test fixtures).
17. Parser coverage over VPP's TUs is reported as a percentage on every CI run and does
    not regress.
18. The vppinfra unit-test suite runs under chiero end to end, and every failure is either
    a real bug or a recorded, triaged chiero limitation — the stage-1 exit criterion.
