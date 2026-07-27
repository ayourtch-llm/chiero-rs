# 024 — Environment models

`chiero-model` decides what happens when execution reaches a function whose body is not in
the module: libc, compiler builtins, syscalls, and the extension point through which
target-specific libraries are modeled. Without it, every interesting VPP function
degrades to `Approximated` at its first `clib_mem_alloc`.

**`chiero-model` contains no VPP knowledge.** It defines the registry and the standard
models; `chiero-vpp` ([060](060-vpp-integration.md)) registers vppinfra models into it.
If `vec_`, `pool_` or `clib_` appears in this crate, the "reusable library" requirement
has been broken ([001 §7](001-architecture.md) contract 5).

## 1. Registry

```rust
pub struct ModelRegistry { by_name: IndexMap<Symbol, ModelId>, models: Vec<ModelEntry> }

pub struct ModelEntry {
    pub name: Symbol,
    pub sig: ModelSig,                 // arity, variadic, declared param/return CTys
    pub precision: Precision,          // Exact | Approximate(&'static str)
    pub imp: ModelImpl,
    pub origin: ModelOrigin,           // Builtin | Registered(&'static str) | UserCir(PathBuf)
}

pub enum ModelImpl {
    Native(Box<dyn Model>),            // Rust
    Cir(FuncId),                       // a CIR stub in a side module
}

pub trait Model {
    fn call(&self, args: &[Term], ctx: &mut ModelCtx) -> ModelOutcome;
}

pub enum ModelOutcome {
    Value(Option<Term>),               // returns, execution continues
    Fork(Vec<(Term, ModelOutcome)>),   // guarded alternatives (e.g. malloc success/failure)
    Terminate(TermReason),             // exit, abort, longjmp-as-unsupported
    Havoc(HavocSpec),                  // explicit "I don't know", never implicit
}
```

Resolution order when `Call` reaches a `Body::Declared` function:

1. A definition present in the module wins (models never shadow real code — a real body is
   always more faithful than a model, and a model that silently overrode source would make
   analysis results depend on invisible state).
2. Registered model by exact symbol name.
3. `__builtin_x` falls back to the model for `x` when one exists (gcc's own aliasing).
4. Otherwise: **default havoc** — every pointer argument's pointee object is invalidated,
   a fresh return value is produced, `Fidelity::Approximated` is set, and the symbol is
   recorded in `assumptions`. This is the loud default; there is no quiet one.

## 2. Two ways to write a model

**Native Rust** for anything needing memory-model operations or forking:
`malloc`, `free`, `realloc`, `memcpy`, `strlen`.

**CIR stubs** (`.cir` files loaded into a side module) for anything expressible as plain
C-level code: `abs`, `toupper`, `strcmp` written as an explicit loop. Preferred where
possible, because a stub is data — reviewable, diffable, testable with the same harness as
any other CIR, and unable to violate the memory model's invariants.

Both forms declare a `Precision`. `Approximate` requires a static string explaining what
is lost, which is what appears in the report. A model author cannot mark something
approximate without saying why.

### 2.1 Precision has a mechanical fidelity effect

Declaring `Precision::Approximate(r)` is **not** editorial. Dispatching such a model:

1. sets `Fidelity ≥ Approximated` per the [023 §7](023-execution-engine.md) table, and
2. pushes `Assumption { kind: Model, detail: r, span }`.

`ModelOutcome::Havoc` does the same, whether it came from the default fallback (§1 step 4)
or from a registered model that chose to havoc.

Without this rule there is a hole straight through the project's central guarantee: a run
calling `scanf` (`Approximate("input")`), any `<math.h>` function (`Approximate("float")`),
`read`/`ioctl` (`Approximate("syscall")`), or `__builtin_frame_address` could finish
`Exact`, mint an `ExactWitness`, and report **"no bugs exist"** as a proof. The unmodeled
path was already handled loudly; this closes the *modeled* path, which is worse because it
looks deliberate.

`HavocSpec` must therefore say exactly what it invalidates:

```rust
pub struct HavocSpec {
    pub objects: Vec<ObjectId>,     // fully invalidated
    pub ranges: Vec<(Pointer, Term)>,
    pub reachable_depth: u32,       // follow pointers stored *inside* those objects, N deep
    pub init: HavocInit,            // Symbolic (known-unknown) | Uninitialized
    pub may_free: bool,
}
```

`init` matters and has no safe default: `Symbolic` marks bytes initialized-with-unknown-
value, which can mask a genuine uninitialized-read bug; `Uninitialized` produces a
false-positive storm on any buffer the callee legitimately filled. The default for an
**unmodeled** extern is `Symbolic` with `reachable_depth: 1` — an unknown function is
assumed to have written something meaningful rather than left garbage — and the choice is
recorded in the assumption so it is visible rather than folkloric.

## 3. Memory models

| Symbol | Semantics |
|---|---|
| `malloc(n)` | Fresh `Heap` object of size `n` (symbolic sizes allowed, see [021 §7](021-memory-model.md)), contents **uninitialized**. Forks: success and `NULL`. |
| `calloc(n,m)` | As `malloc(n*m)` with contents zeroed and marked initialized; the `n*m` overflow is checked and reported. |
| `realloc(p,n)` | New object + `CopyMem(min(old,new))` + free old; `p == NULL` behaves as `malloc`; `n == 0` is reported as implementation-defined. |
| `free(p)` | `Freed`; `free(NULL)` is a no-op; freeing a non-heap object is a finding. |
| `memcpy/memmove/memset/memcmp` | Direct memory-model operations; `memcpy` with overlap is a finding. |
| `alloca(n)` | Stack object in the current frame's scope. |
| `posix_memalign`, `aligned_alloc` | As `malloc` with alignment recorded. |

**`malloc` forks on failure by default** (`alloc_may_fail = true`). Most real allocation
failure bugs are unreachable otherwise, and the failure branch is cheap. Allocators that
abort instead of returning `NULL` register with `alloc_may_fail = false` — which is
exactly how `chiero-vpp` will model `clib_mem_alloc`.

## 4. String models

`strlen`, `strcpy`, `strncpy`, `strcmp`, `strncmp`, `strcat`, `strchr`, `strstr`,
`snprintf`.

The hard case is symbolic contents. `strlen(p)` where the bytes are symbolic:

1. Scan forward from the pointer while the byte is *provably* non-zero (fast path,
   concrete bytes).
2. At the first byte that *may* be zero, fork: one state with the byte constrained to zero
   (length known), one with it non-zero, continuing.
3. Cap at `max_string_scan` (default 256), set `Bounded`, and record.
4. Running off the end of the object is an OOB finding (unterminated string), not a
   silent stop — this is a real bug class and the most valuable thing these models catch.

Steps 3 and 4 must not be allowed to cancel each other. An earlier draft had the cap
"constrain a terminator to exist within the bound", which **assumes away exactly the
unterminated-string bug step 4 exists to find** whenever the object is smaller than the
cap. The rule is therefore: the scan is bounded by `min(max_string_scan, object size)`,
reaching the *object's* end is always an OOB finding, and reaching the *scan cap* first
adds no constraint — it terminates the state with `Bounded`. Constraining a terminator to
exist is never correct.

`strcpy` and `strcat` are the same walk plus a bounds check on the destination, which is
where classic overflows are found.

## 5. Process, I/O and diagnostics

| Symbol | Semantics |
|---|---|
| `exit`, `_exit`, `abort` | `Terminate`; `abort` additionally emits an abort finding. |
| `__assert_fail`, `__assert_rtn` | Assertion-failure finding with the asserted text and a witness, then terminate. |
| `printf`/`fprintf`/`puts`/`fwrite` | No memory effect; arguments are *read* (so an invalid pointer argument is caught) and the formatted output is recorded on the path when the format string is concrete. Format-string/argument mismatches are a finding. |
| `scanf` family | Havoc destinations, `Approximate("input")`. |
| `open/read/write/close`, `socket`, `ioctl` | Symbolic return honoring the documented error convention (e.g. `read` returns `-1..=n`), destinations havoc'd where written, `Approximate("syscall")`. |
| `getenv` | `NULL` or a symbolic string, forked. |
| `longjmp`/`setjmp` | **Unsupported**: diagnosed, state terminated with `Unknown`. Pretending is worse than declining. |

### 5.1 Threading primitives

`pthread_mutex_lock`/`_unlock`/`_trylock`, `pthread_rwlock_*`, `pthread_create`,
`pthread_self`, and C11 `mtx_*` are modeled **here**, not in `chiero-vpp`. They are
standard-library surface, not VPP knowledge, and
[025 §3](025-concurrency-and-threading.md)'s discipline checker needs a target-agnostic
lock vocabulary — 025 contract 15 requires the checker to find the same bug classes on a
pthread corpus as on a VPP one, which is what proves the checker is not secretly
VPP-shaped. `chiero-vpp` registers `clib_spinlock_*`/`clib_rwlock_*` against the same
`LockOp` vocabulary.

`pthread_create` is **not** executed as a thread in v1: it records a thread-entry point
for the discipline checker and returns success, with the no-interleaving blind spot
already declared by 025 §4.

**Thread-local storage** (`__thread`, `_Thread_local`; 20 VPP files) creates an object with
`ObjOrigin::Global` but `Sharing::PerThread { key: current thread index }` pinned at
creation. Without this a TLS variable is indistinguishable from a shared global, and every
correct per-thread access becomes a false `Shared` finding.

## 6. Compiler builtins

Modeled exactly (all measured in VPP, [060](060-vpp-integration.md)):

| Builtin | Model |
|---|---|
| `__builtin_expect(e, c)` | Returns `e`; the hint is discarded. |
| `__builtin_unreachable()` | `Terminate`; reaching it on a feasible path is a finding, not a licence. |
| `__builtin_constant_p(e)` | Resolved in the frontend ([014 §6](014-semantics-and-types.md)); reaching it here is an error. |
| `__builtin_clz/clzll/ctz/ctzll/popcount` | Exact bitvector formulations; the `x == 0` UB case is a finding. |
| `__builtin_bswap16/32/64` | Byte permutation on the term. |
| `__builtin_{add,sub,mul}_overflow` | Exact: wide computation, compare, store, return the overflow bit. |
| `__builtin_object_size(p, t)` | Object's real size from the memory model when known, else `-1`/`0` per `t`. |
| `__builtin_prefetch`, `CLIB_PREFETCH` | No-op (validated: the pointer argument is still range-checked). |
| `__builtin_frame_address` | Symbolic pointer to an `Extern` object; any dereference is `Approximate`. |
| `__builtin_shufflevector`, `__builtin_shuffle` | Exact lane permutation when indices are constant (they always are in VPP); symbolic indices are `Approximate`. |
| `__builtin_alloca` | As `alloca`. |
| `__builtin_memcpy` etc. | Alias to the libc model. |
| `__builtin_va_start/arg/end/copy` | Backed by the `VaArg` RValue ([020 §4.1](020-cir.md)) over a frame-local argument object. |

Math (`abs`, `labs`, `llabs`, `min`/`max` idioms) is exact. `<math.h>` floating-point
functions are `Approximate("float")` until the float sort lands ([022 §1](022-solver.md)).

## 7. Harness intrinsics

The corpus and any user harness need to introduce symbolism explicitly:

```c
void  chiero_make_symbolic(void *addr, size_t n, const char *name);
void  chiero_assume(int cond);          /* constrain; contradiction kills the state */
void  chiero_assert(int cond);          /* finding if violable */
int   chiero_is_symbolic(long v);       /* introspection, for tests only */
void  chiero_mark_fidelity(const char *why);
```

These are models like any other, declared in `include/chiero.h`, and compile to no-ops
under gcc so the same corpus file is both a chiero input and a runnable C program. That
dual use is what makes the differential oracle ([070](070-testing-and-tdd-protocol.md))
work without maintaining two copies of every test.

`chiero_assume(0)` kills the state silently; `chiero_assert` produces a finding with a
witness. They are not interchangeable, and confusing them is the classic way to make a
test suite vacuous — contract 15 pins the difference.

## 8. Extension point

```rust
impl ModelRegistry {
    pub fn register(&mut self, entry: ModelEntry) -> Result<ModelId, ModelError>;
    pub fn register_cir_module(&mut self, m: &Module) -> Result<Vec<ModelId>, ModelError>;
    pub fn with_defaults(target: &TargetConfig) -> Self;
}
```

Re-registering an existing symbol is an **error**, not a silent override; replacement is
explicit (`replace`). `chiero-vpp` calls `register` for vppinfra; a user embedding chiero
calls it for their own library. The registry is part of the public API and its contents
are printed in `--explain` output, so a reader can always see which models were in force.

## 9. Testable contracts

1. `malloc(16)` produces one `Heap` object of size 16 with all bytes uninitialized, and
   forks into a success state and a `NULL` state.
2. With `alloc_may_fail = false`, the same call produces exactly one state.
3. `calloc(4, 8)` yields 32 zeroed, *initialized* bytes; reading them produces no
   uninitialized-read finding.
4. `calloc(SIZE_MAX, 2)` produces exactly one overflow finding.
5. `free(NULL)` is a no-op producing no findings; `free(&stack_var)` is exactly one
   finding.
6. `strlen` over the concrete bytes `"abc\0"` returns 3 with no forking.
7. `strlen` over 4 symbolic bytes in a 4-byte object forks into states with lengths
   0,1,2,3 plus one unterminated-string OOB finding.
8. `strlen` on a 1000-byte symbolic object with `max_string_scan = 256` yields `Bounded`
   and records the cap.
9. `strcpy` into a 4-byte destination from a 10-byte concrete source is exactly one OOB
   finding, at the destination's span.
10. `memcpy` with overlapping ranges is one finding; `memmove` with the same ranges is
    none and produces the correct bytes.
11. An unmodeled extern `foo(int*, int)` invalidates the pointee object, returns a fresh
    value, sets `Approximated`, and records `"foo"` in `assumptions` exactly once.
12. A module that *defines* `memcpy` uses its own definition, not the model, and no
    assumption is recorded.
13. `__builtin_add_overflow(INT_MAX, 1, &r)` returns 1 and stores `INT_MIN`;
    `__builtin_add_overflow(1, 1, &r)` returns 0 and stores 2.
14. `__builtin_clz(0)` produces exactly one UB finding; `__builtin_clz(1)` returns 31 on
    a 32-bit input with none.
15. `chiero_assume(x > 5)` followed by a branch on `x > 3` produces one state;
    `chiero_assert(x > 5)` with unconstrained `x` produces exactly one finding with a
    witness where `x <= 5`.
16. `chiero_assume(0)` terminates the state with no finding.
17. Every corpus file including `chiero.h` compiles and runs under gcc with the
    intrinsics as no-ops.
18. `register` on an already-registered symbol returns `ModelError::Duplicate`; `replace`
    succeeds and `--explain` shows the replacement.
19. `grep -rE 'vec_|pool_|clib_|vlib_' crates/chiero-model/src` yields no hits.
20. `longjmp` in a corpus program yields exactly one "unsupported" diagnostic and a state
    terminated with `Fidelity::Unknown` — never a silently-continued path.
21. Every `ModelEntry` with `Precision::Approximate` carries a reason string of ≥ 8
    non-whitespace characters, **and** for each such entry in the default registry a
    single-call program yields `Fidelity::Approximated`, exactly one `Assumption`, and
    that reason text present in the rendered report. (A non-empty check alone is satisfied
    by `" "` and says nothing about the fidelity effect.)
21b. A corpus program calling `scanf` cannot produce an `Exact` result, and `seal`
     ([023 §7.1](023-execution-engine.md)) returns `NotProven` for it.
21c. `ModelOutcome::Havoc` from a *registered* model degrades fidelity identically to the
     default unmodeled-extern havoc.
21d. Default havoc with `reachable_depth: 1` invalidates a pointer stored inside the
     havoc'd object's bytes; with `0` it does not. Both are recorded in the assumption.
21e. `HavocInit::Symbolic` produces no uninitialized-read finding on the havoc'd bytes;
     `Uninitialized` produces one. The default for an unmodeled extern is `Symbolic`.
22. `printf("%d", p)` where `p` is a pointer produces exactly one format-mismatch finding;
    `printf` with an invalid pointer argument produces one memory finding.
