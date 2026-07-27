# 025 — Concurrency and threading

VPP is a worker-thread architecture. Declaring concurrency a non-goal in one line
([000](000-overview.md)) is not good enough for a tool whose validation target runs its
hot path on N threads: it would leave users guessing which of chiero's answers survive
contact with a multi-threaded run.

This document says exactly what v1 does, what it refuses to do, what it *reports* about
the gap, and which architectural hooks keep v2 from being a rewrite.

## 1. VPP's threading discipline (measured)

Counted by file over `/home/ubuntu/vpp/src` @ `7fe9c26`:

| Primitive | Files |
|---|---|
| `thread_index` (per-thread indexing) | **467** |
| `clib_atomic_*` | 73 |
| `clib_spinlock_t` | 69 |
| `vlib_worker_thread_barrier_sync` | 54 |
| `clib_rwlock_t` | 10 |
| `__thread` (TLS) | 20 |

The shape this reveals is the whole reason a useful answer is possible: **VPP does not
share mutable state casually.** The dominant discipline is *partitioning* — per-thread
`vlib_main_t`, per-thread heaps, per-thread counters, all indexed by `thread_index` — and
explicit synchronization is rare and localized. Concretely:

1. **Worker threads** run the graph over disjoint packet sets, touching per-thread state.
2. **Configuration changes** run on the main thread with workers stopped at a barrier
   (`vlib_worker_thread_barrier_sync` / `_release`). Inside a barrier, shared state is
   effectively single-threaded.
3. **The residue** — genuinely shared, concurrently mutated state — is guarded by
   spinlocks, rwlocks, or atomics, in ~70 files.

So the analysis question worth answering is not "enumerate all interleavings of a million
lines" (hopeless). It is **"is this code obeying the discipline?"** — which is decidable
per-path and is where real VPP concurrency bugs actually live.

## 2. What v1 executes

**One thread, with an explicit thread context.** Execution is sequential and
sequentially consistent. Every run declares its context:

```rust
pub enum ThreadCtx {
    Main,                       // main thread, workers running
    Worker { index: Term },     // a worker; `index` is symbolic by default
    Barrier,                    // main thread, workers stopped — shared state is exclusive
    Unspecified,                // unknown; the most conservative for the §3 checker
}
```

`Worker { index }` with a **symbolic** thread index is the important case: per-thread
arrays like `vlib_mains[thread_index]` or `vm->thread_index`-keyed counters are then
analysed for an arbitrary worker rather than worker 0, so an off-by-one against
`vlib_num_workers()` is reachable. The context is supplied by the caller, or inferred by
`chiero-vpp` from node registration ([060](060-vpp-integration.md)).

C11 atomics and `clib_atomic_*` execute as their non-atomic equivalents. The memory
ordering argument is **preserved in the IR** ([020 §4.2](020-cir.md)) and consumed by §3
even though execution ignores it.

## 3. The concurrency-discipline checker

This is v1's actual concurrency deliverable, and it is a `Checker`
([023 §6](023-execution-engine.md)) — path-sensitive, driven by real symbolic execution,
not by grep.

Each `MemObject` is classified as execution proceeds:

```rust
pub enum Sharing {
    PerThread { key: Term },     // reached only via an index equal to the thread index
    Immutable,                   // written before workers start, read-only after
    Guarded { lock: ObjectId },  // every access on this path held `lock`
    BarrierOnly,                 // accessed only in ThreadCtx::Barrier
    Shared,                      // concurrently reachable and mutable
    Unknown,
}
```

Classification is a lattice join over accesses; an object that is `PerThread` on one path
and `Shared` on another is `Shared`. The checker maintains a **lock set** per state,
updated by lock/unlock models registered by `chiero-vpp`.

Findings it produces:

| Finding | Condition |
|---|---|
| **Unguarded shared write** | A write to a `Shared` object in `Worker` context with an empty lock set and no atomic op. |
| **Unguarded shared read-modify-write** | A `Shared` counter incremented non-atomically — the classic missed-count bug. |
| **Lock not held on a guarded object** | An object `Guarded { l }` elsewhere is accessed here without `l`. Inconsistent guarding is stronger evidence than absolute rules. |
| **Lock leak / asymmetric unlock** | A path returns with a lock still held, or unlocks one never taken. Path-sensitivity is what makes the error-return paths — where this bug always is — visible. |
| **Lock-order inversion** | Two paths acquire `{a,b}` in opposite orders. Reported from a global lock-order graph accumulated across all analysed entry points; a cycle is a deadlock candidate. |
| **Barrier-protected state touched from a worker** | An object only ever written in `Barrier` context is written from `Worker` context. |
| **Per-thread key mismatch** | A `PerThread { key }` object accessed with an index the solver says can differ from the current `thread_index` — one worker reaching into another's state. |
| **Missing barrier around config mutation** | A `Shared` structural mutation (`vec_*`/`pool_*` resize) in `Main` context without an enclosing barrier. This is a top-tier VPP crash cause: a worker traversing a vector while main reallocs it. |

Every one carries a witness and an expansion backtrace like any other finding
([023 §9](023-execution-engine.md)). None of them requires exploring an interleaving,
which is why they are affordable.

## 4. What v1 explicitly does not do

Stated in these words in every report touching a `Shared` object:

- **No interleaving exploration.** Chiero does not consider one thread's writes landing
  between another thread's instructions. A data race whose only symptom is a torn or
  stale read will not be found.
- **No weak memory model.** Execution is sequentially consistent; x86-TSO and aarch64's
  weaker ordering are not modeled, so missing-barrier and missing-`acquire`/`release` bugs
  are not found. Ordering arguments are recorded, not honoured.
- **No lock-free algorithm verification.** Anything whose correctness depends on the
  interleaving (RCU-style publish/retire, hazard pointers, seqlocks) is out of scope.
- **No ABA, no priority inversion, no livelock.**

Findings are still sound in the direction that matters: a reported unguarded shared write
is really unguarded on that path. Absences are not proofs — and per
[023 §7](023-execution-engine.md), a run in `Worker` or `Unspecified` context whose result
touched a `Shared` object **cannot be reported as `Exact`**. It is capped at `Bounded`
with an assumption naming the sequential-consistency and no-interleaving limits. That cap
is the mechanism that keeps this document's honesty from depending on prose.

## 5. Hooks for v2

None of the following is implemented in v1; all of it is *not precluded*, which is the
requirement:

1. **The IR needs no change.** Atomic ordering already rides on `Load`/`Store`
   ([020 §4.2](020-cir.md)), and `Marker` is extensible with a `SchedPoint` variant.
2. **State is already the unit of scheduling.** A multi-threaded state is
   `Vec<ThreadState>` sharing one `Memory`; `chiero-mem`'s copy-on-write objects
   ([021 §8](021-memory-model.md)) already support that sharing.
3. **`Searcher` already abstracts exploration order** ([023 §4](023-execution-engine.md)),
   so an interleaving explorer is a `Searcher` plus a scheduling-point set — not a change
   to the engine's contract.
4. **The tractable v2 design** is bounded interleaving with **dynamic partial-order
   reduction, scheduling only at synchronization points and `Shared`-object accesses**.
   The §1 measurements are what make this plausible: with sync in ~70 files and the rest
   partitioned, the reduced interleaving space for a single graph node is small. §3's
   `Sharing` classification is precisely the input DPOR needs, so v1's checker doubles as
   v2's groundwork.

## 6. Testable contracts

1. A run with `ThreadCtx::Worker { index: fresh }` over an access to
   `per_thread[thread_index]` classifies the object `PerThread` and produces no finding;
   an access to `per_thread[thread_index + 1]` produces exactly one per-thread-key
   mismatch with a witness.
2. `per_thread[i]` where the solver proves `i == thread_index` produces no finding — the
   check is semantic, not syntactic.
3. A global counter incremented with `g++` in `Worker` context with no lock is exactly one
   unguarded-shared-RMW finding; the same increment via `clib_atomic_add_fetch` produces
   none.
4. The same increment inside `spinlock_lock(&l) … spinlock_unlock(&l)` produces none, and
   the object is classified `Guarded { l }`.
5. An object accessed under `l` on one path and without `l` on another produces exactly
   one lock-not-held finding, naming both spans.
6. A function returning early on an error path with a lock held produces exactly one
   lock-leak finding, at the `return`.
7. Unlocking a lock never acquired on that path is exactly one asymmetric-unlock finding.
8. Two entry points acquiring `a,b` and `b,a` produce exactly one lock-order-inversion
   finding; the lock-order graph is deterministic across runs and independent of the order
   the entry points were analysed in.
9. A `vec_add1` on a `Shared` vector in `Main` context without a barrier is exactly one
   missing-barrier finding; wrapped in `barrier_sync`/`barrier_release` it produces none
   and the context is `Barrier` inside.
10. An object written only in `Barrier` context, then written from `Worker` context,
    produces exactly one finding.
11. Every result whose execution touched a `Shared` object has `fidelity <= Bounded` and
    carries an assumption naming sequential consistency and no-interleaving — enforced by
    a test that greps the rendered report text.
12. A result in `ThreadCtx::Main` touching only `PerThread` and `Immutable` objects may
    still be `Exact` (the cap is not blanket pessimism).
13. `Sharing` classification is a monotone lattice join: replaying a corpus with the paths
    explored in reverse order yields identical classifications.
14. `grep -rE 'vlib_|clib_spinlock|thread_index' crates/chiero-exec/src crates/chiero-check/src`
    yields no hits — every VPP-specific lock and thread-index model is registered from
    `chiero-vpp` through the [024 §8](024-environment-models.md) extension point, and the
    checker itself is target-agnostic.
15. The discipline checker runs on a pthread-based (non-VPP) corpus program with
    `pthread_mutex_*` models and finds the same five bug classes — proving contract 14 is
    real and not an accident of naming.
