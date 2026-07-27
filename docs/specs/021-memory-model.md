# 021 — Memory model

`chiero-mem` owns every byte the analysed program can address. It is a **flat
object/offset model** in the KLEE tradition: memory is a set of disjoint objects, and a
pointer is a pair `(object, offset)` rather than a single integer.

This choice is not stylistic. Object/offset is what makes bounds checking possible at all
(an integer address has no bounds), and it is what makes vppinfra's central data
structure representable (§2). A memory model that cannot express a **negative offset
within an object** is wrong for VPP, which disqualifies the naive
"one big symbolic array" design.

## 1. Objects

```rust
pub struct MemObject {
    pub id: ObjectId,
    pub kind: ObjKind,
    pub size: SizeVal,              // Const(u64) | Sym(Term)  — VLA, clib_mem_alloc(n)
    pub align: u64,
    pub addr: u64,                  // concrete base address (§7)
    pub contents: Contents,
    pub state: ObjState,            // Live | Freed(Span) | OutOfScope(Span)
    pub origin: ObjOrigin,          // Global(GlobalId) | Alloca{fn,AllocaId} | Heap(Span)
                                    // | LazyParam{fn, param} | LazyDeref(Span) | Extern(Symbol)
    pub readonly: bool,
}

pub enum ObjKind { Global, Stack, Heap, Extern, Lazy }
```

Reserved objects, present in every state:
`ObjectId::NULL` — size 0, address 0, any access is a null-dereference finding.
`ObjectId::UNBOUND` — the target of a pointer produced by `IntToPtr` from a value that
matches no known object; any access is a wild-pointer finding with `Fidelity::Unknown`.

## 2. Pointers and the validating use case

```rust
pub struct Pointer { pub base: ObjectId, pub off: Term }   // off is BV(pointer_width), signed
```

The offset is a **signed** bitvector and may be negative. This is required, not permitted:

```c
/* vppinfra/vec_bootstrap.h, paraphrased */
typedef struct { u32 len; u32 dlmalloc_header_offset; } vec_header_t;
#define _vec_len(v)   (vec_header(v)->len)
#define vec_header(v) ((vec_header_t *)(v) - 1)
```

A vppinfra vector is one allocation laid out `[ header | elem0 elem1 … ]`, and the value
handed to user code points at **`elem0`**. `_vec_len(v)` therefore reads at offset
`-8` from the user pointer. In chiero this is one `MemObject` spanning header and
elements, with the user pointer being `Pointer { base, off: 8 }`; `_vec_len` reads
`[0, 4)` — in bounds, correctly typed, and bounds-checkable. Every `vec_*` operation
falls out of this naturally, and an OOB write past the last element is detected because
the object's true extent is known.

`PtrAdd` ([020 §4.1](020-cir.md)) preserves `base` and adds to `off`. **Provenance is
never lost**: arithmetic that leaves an object's bounds produces a pointer that is still
anchored to that object, and is a finding when dereferenced (and, under
`ub-strict` mode, when merely formed — C makes one-past-the-end legal and anything
beyond it UB, so the two are reported at different severities).

## 3. Contents representation

```rust
pub enum Contents {
    /// Fast path: concrete bytes plus a per-byte initialization mask,
    /// plus a sparse overlay of symbolic bytes at concrete offsets.
    Bytes { data: Vec<u8>, init: BitVec, sym: BTreeMap<u64, SymByte> },
    /// Slow path: SMT array BV(ptr_width) -> BV(8), with a parallel init array.
    Array { data: Term, init: Term },
}
```

Objects start as `Bytes`. Promotion to `Array` happens on exactly one trigger: a write at
a **symbolic offset** that the solver cannot pin to a small set of concrete offsets.
Reads at a symbolic offset from a `Bytes` object are answered by an if-then-else chain
when the feasible offset set has ≤ `ite_threshold` (default 16, configurable, recorded in
the result) and force promotion otherwise. Promotion is one-way within a state.

Rationale: the overwhelming majority of VPP accesses are at concrete offsets from a
symbolic base (`p->field`, `v[i]` with `i` concretizable), and those must not pay array
theory's cost. The threshold is a documented constant so results are reproducible.

**Endianness** comes from `TargetConfig`. Multi-byte reads assemble bytes in target order;
a read of `Int(32)` over 4 bytes where 2 are concrete and 2 symbolic produces a `Concat`
term, not a promotion. This is what makes type punning and partial overwrites exact.

**No strict-aliasing assumptions.** Bytes are bytes. Reading an `Int(32)` from bytes
written as a `Float(F32)` yields the bit reinterpretation, matching what the hardware
does and what VPP's packet code relies on. `__attribute__((may_alias))` therefore needs
no special handling; it is recorded and ignored.

**Initialization tracking is mandatory**, not an optional checker feature: the `init`
mask is what makes uninitialized-read detection possible, and reading uninitialized bytes
yields a fresh symbol *plus* a finding rather than zero. Silently reading zero is the
single most common way a symbolic executor produces confidently wrong results.

## 4. Lifetime

- **Globals** are created at module load, initialized from `GlobalInit`, `Live` forever.
  `readonly` globals reject writes with a finding.
- **Stack** objects are created on `Marker(Scope(Enter))` for the enclosing scope's
  allocas and transition to `OutOfScope` on `Scope(Exit)`. They are *not* deleted:
  keeping them lets a dangling-pointer access report the exact scope that ended, with the
  `Span`. Use-after-scope is thus detected, not merely undetected-and-benign.
- **Heap** objects come from models ([024](024-environment-models.md)): `malloc`,
  `calloc`, `realloc`, `clib_mem_alloc*`. `free` sets `Freed(span)`; a subsequent access
  is use-after-free naming both the free site and the access site; a subsequent `free` is
  double-free. Leak detection runs at `Return` from the entry function: `Live` heap
  objects unreachable from globals, the return value, or any live stack object are leaks.
- **Extern** objects back symbols with no definition in the module.

`realloc` is modeled as allocate-new + `CopyMem(min(old,new))` + free-old, which is what
makes `vec_resize` analysable: the old pointer becomes dangling and any surviving copy of
it is reported. That is a real and frequent VPP bug class.

## 5. Access

```rust
impl Memory {
    fn read (&self, p: &Pointer, size: u64, ty: CTy) -> Result<Term, MemFault>;
    fn write(&mut self, p: &Pointer, val: &Term, size: u64) -> Result<(), MemFault>;
    fn copy (&mut self, dst: &Pointer, src: &Pointer, size: &Term, overlap: Overlap);
    fn set  (&mut self, dst: &Pointer, byte: &Term, size: &Term);
}
```

Every access performs, in this order:

1. **State check** — `Freed`/`OutOfScope`/`NULL` → fault, no read of stale bytes.
2. **Bounds check** — the solver is asked whether `off < 0 ∨ off + size > obj.size` is
   satisfiable under the path condition. If yes, an OOB finding is produced with a
   concrete witness, and execution **continues on the in-bounds branch** with the
   in-bounds constraint added. Continuing (rather than killing the state) is what keeps
   one early OOB from hiding everything downstream of it.
3. **Alignment check** — misalignment is a finding only in `ub-strict` mode (x86-64
   tolerates it and VPP relies on that in places), but is always recorded.
4. **Initialization check** — any uninitialized byte in the range yields a finding and a
   fresh symbol for those bytes.
5. The read/write itself.

`MemFault` carries the `ObjectId`, the offset term, a concrete witness offset, and the
`Span` of both the access and the object's origin. Findings are emitted by the engine's
checker hooks ([023 §6](023-execution-engine.md)), not by `chiero-mem` itself — the
memory model reports faults, it does not decide what they mean.

### 5.1 Symbolic base pointers

When the *base* is symbolic — a pointer loaded from memory whose value the solver has not
pinned — resolution is:

1. Ask the solver for the set of objects whose address range the pointer value can fall
   in, capped at `max_resolutions` (default 8).
2. If exactly one, continue with it.
3. If several, **fork** one state per object with the corresponding constraint, plus one
   state constrained to none of them (the wild-pointer state, reported).
4. If more than `max_resolutions`, concretize to the solver's model, record
   `Fidelity::Bounded`, and say so in every result derived from the path.

## 6. Lazy initialization (under-constrained symbolic execution)

You cannot reach a VPP internal function from `main`. Analysis must start at an arbitrary
function with unconstrained parameters, which means pointer parameters point at nothing
until they are used.

A pointer parameter or unresolved global read starts as a symbolic value with no object.
On **first dereference**, a `Lazy` object is materialized:

```rust
pub struct LazyPolicy {
    pub scalar_extent: u64,      // bytes for a pointer to a scalar/struct: sizeof(pointee)
    pub array_factor: u64,       // pointer used with PtrAdd → sizeof(pointee) * factor (default 8)
    pub max_depth: u32,          // lazily-initialized pointers *inside* lazy objects (default 3)
    pub distinct_by_default: bool,  // true
}
```

Contents are fully symbolic and fully initialized (a caller-supplied buffer is not
"uninitialized" — it is unknown), which is why `init` tracking must distinguish
*symbolic* from *uninitialized*. Those are different, and conflating them turns every
UCSE run into an uninitialized-read false-positive storm.

**Aliasing policy**: two lazily-materialized objects are **distinct** by default. This is
an assumption, not a fact, so it is recorded in `Result::assumptions` and printed in every
report. `--fork-on-alias` forks the alias case; it is off by default because it multiplies
states by `2^(pairs)`.

`max_depth` bounds the recursion of linked structures (`p->next->next->…`). Exceeding it
yields `Fidelity::Bounded` and a note naming the field that was cut off.

## 7. Concrete addresses

Every object gets a concrete base address at creation from a deterministic allocator:

| Region | Base | Direction |
|---|---|---|
| Globals | `0x0000_1000_0000` | up |
| Heap | `0x0000_2000_0000` | up |
| Stack | `0x7fff_0000_0000` | down |
| Lazy | `0x0000_4000_0000` | up |

Objects are page-aligned with a **4096-byte guard gap** between them, so an OOB pointer
never lands in another object by accident and `PtrToInt` comparisons behave like a real
program. Allocation is a simple bump per region, seeded identically every run — no
randomization, because determinism is a hard requirement
([001 §5](001-architecture.md)).

These addresses are **logical**. They are not physical addresses, they carry no timing
meaning, and nothing about caches, TLBs, NUMA or DMA is modeled here. Cache hierarchies
are coherent on every target chiero supports, so no cached value can differ from the
value this model returns — modeling them would cost work and change no answer. Cache-line
*layout* effects (straddling, false sharing, prefetch distance) are real and VPP tunes
them explicitly; they are a **performance** property, analysed from `RecordLayout` and
`TargetConfig::cache_line_bytes` in [041](041-optimization-analysis.md), not a semantic
one. The guard gaps above are chosen for OOB detection, not to mimic any real allocator's
placement, and no analysis may infer locality from them.

`PtrToInt` yields `addr + off`. `IntToPtr` searches the address ranges; a hit yields the
corresponding `Pointer`, a miss yields `UNBOUND`. Pointer comparison between objects is
therefore decidable and consistent with the arithmetic, which C strictly speaking leaves
unspecified — the deviation is intentional, documented, and reported when a program's
outcome depends on it.

## 8. Forking and sharing

```rust
pub struct Memory { objects: Vec<Arc<MemObject>>, next_addr: RegionCursor }
```

Forking a state clones the `Vec<Arc<_>>` — shallow, O(objects) pointer copies — and
writes use `Arc::make_mut` for copy-on-write per object. Object identity (`ObjectId`) is
stable across forks, so a finding in one state can be compared with one in another.

Measured object counts decide whether this is enough; if VPP entry points routinely
exceed ~10⁴ objects, the `Vec<Arc<_>>` is replaced by a persistent HAMT behind the same
API. The API is specified so that swap does not touch callers.

## 9. Testable contracts

1. A 64-byte object with the user pointer at offset 8: reading `Int(32)` at offset `-8`
   from the user pointer succeeds and returns the header bytes; reading at `-16` is an
   OOB finding. (The `vec_header` contract.)
2. Writing `Int(32)` at offset 60 of a 64-byte object succeeds; at offset 61 it is one
   OOB finding with concrete witness `61`, and execution continues with the in-bounds
   constraint.
3. `PtrAdd` past the end of an object then back inside yields a pointer that reads
   correctly and preserves `base`.
4. Writing `Float(F32)` `1.0` then reading `Int(32)` at the same address yields
   `0x3F800000` under the little-endian target.
5. Writing bytes 0..2 concretely and 2..4 symbolically, then reading `Int(32)`, yields a
   `Concat` term whose low half is concrete — and the object is still `Contents::Bytes`.
6. A write at a symbolic offset with 3 feasible values keeps the object as `Bytes`; with
   1000 feasible values it promotes to `Array`, and both produce identical read results
   for every feasible offset.
7. Reading a never-written stack byte yields exactly one uninitialized-read finding and a
   fresh symbol; reading a lazily-initialized parameter's bytes yields **no** finding.
8. `free(p)` then `*p` is exactly one use-after-free finding naming both spans;
   `free(p); free(p)` is exactly one double-free.
9. `realloc` shrinking an object preserves the retained prefix bytes exactly, and a
   surviving copy of the old pointer is a use-after-free on access.
10. A stack object accessed after its `Scope(Exit)` marker is exactly one use-after-scope
    finding naming the scope's span.
11. Returning from the entry function with one unreferenced `malloc`ed object produces
    exactly one leak finding; storing that pointer into a global produces none.
12. `p = (int*)(uintptr_t)q` for a valid `q` round-trips: `p` resolves to `q`'s object
    with the same offset.
13. An `IntToPtr` of `0xDEAD` resolves to `UNBOUND` and any access is one wild-pointer
    finding with `Fidelity::Unknown`.
14. Two distinct objects never overlap in the concrete address space, and every pair is
    separated by ≥ 4096 bytes (property test over 10 000 random allocation sequences).
15. Two runs of the same program assign identical addresses to identical objects.
16. A symbolic base pointer that can refer to 3 objects forks into 4 states (3 resolved +
    1 wild) with mutually exclusive constraints whose disjunction is implied by the path
    condition.
17. With `max_resolutions = 2` the same case concretizes, and the result carries
    `Fidelity::Bounded` with a note naming the cap.
18. Two lazily-materialized pointer parameters are distinct objects by default; under
    `--fork-on-alias` the alias state exists and the result's `assumptions` differ
    between the two modes.
19. `LazyPolicy::max_depth = 2` on a linked list stops materializing at the third `next`
    and the result carries `Fidelity::Bounded` naming `next`.
20. Forking a state with 1000 objects and writing one object leaves the other 999
    `Arc`s shared (pointer-equality check).
21. Writing to a `readonly` global is exactly one finding and does not alter the bytes.
22. `CopyMem` with overlapping ranges and `Overlap::Forbidden` is one finding (the
    `memcpy` contract); with `Overlap::Allowed` (`memmove`) it is none and the result is
    correct.
