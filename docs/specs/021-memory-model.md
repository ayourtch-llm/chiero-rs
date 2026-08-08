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

pub enum ObjKind { Global, Stack, Heap, Extern, Lazy, Function, VarArgs }
```

`ObjKind::Function` gives every `FuncId` an object of size 0 in the Text region (§7), so
that `AddrOfFunc`/`Const::FuncAddr` ([020 §4](020-cir.md)) has somewhere to point. Without
it there is no `Term`→`FuncId` mapping, and the indirect-call resolution
[023 §5](023-execution-engine.md) depends on cannot be implemented. VPP needs this
constantly: `vlib_node_t::function` is assigned `&node##_fn` and later called through the
loaded pointer, and `format_function_t *`/`unformat_function_t *` are passed by value
throughout.

`ObjKind::VarArgs` backs `va_list` (§10).

```rust
pub enum ObjOrigin {
    Global(GlobalId), Alloca { func: FuncId, alloca: AllocaId, frame: FrameId },
    Heap(Span), LazyParam { func: FuncId, param: u32 }, LazyDeref(Span),
    Extern(Symbol), Function(FuncId), VarArgs { func: FuncId, frame: FrameId },
}
```

`Alloca` carries a `FrameId`: with `max_recursion_depth = 32`
([023 §5](023-execution-engine.md)) there can be 32 live objects for one `AllocaId`, and
an alloca inside a loop produces a fresh object per iteration (§4). An origin that cannot
distinguish activations cannot report which one a use-after-scope refers to.

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
    /// Fast path: concrete bytes, a bit-indexed initialization mask, and a sparse
    /// overlay of symbolic bytes at concrete offsets.
    Bytes { data: Vec<u8>, init: InitMask, sym: BTreeMap<u64, SymByte> },
    /// Slow path: SMT array BV(ptr_width) -> BV(8), with a parallel init array.
    Array { data: Term, init: Term },
}
```

Objects start as `Bytes` — **except** those with a symbolic size, which start as `Array`.
A `Bytes` object's `data: Vec<u8>` and bit-indexed `init` both need a concrete length, so
`SizeVal::Sym` has no `Bytes` representation at all. Note this is rare in practice:
[023 §10](023-execution-engine.md) concretizes symbolic allocation sizes to a solver model
and records the equality constraint, which also gives §7's bump allocator the concrete
extent it needs to place the next object. `SizeVal::Sym` survives for the window between
allocation and concretization, and for objects whose size is never queried.

Promotion to `Array` happens on **two** triggers. The first is a write at a **symbolic
offset** that the solver cannot pin to a small set of concrete offsets. Reads at a symbolic
offset from a `Bytes` object are answered by an if-then-else chain when the feasible offset
set has ≤ `ite_threshold` (default 16, configurable, recorded in the result) and force
promotion otherwise.

The second is a `HavocFill::Symbolic` havoc ([024 §2.1](024-environment-models.md)), which
replaces an object's contents with an unconstrained array. This was added while
implementing the havoc and is amended here rather than left as an undocumented deviation.
The alternative — one fresh byte term per byte — is O(size) in solver variables for
something that happens on **every** unmodeled call, and `MAX_MATERIALIZED_BYTES` is 64 MiB.
It has a real cost, and the cost is stated rather than hidden: a promoted object has no
byte view, so a later concrete read of it faults and a later havoc of it cannot follow the
pointers stored inside it. The second of those is why `Memory::pointees` reports
completeness separately from what it found — "nothing there" and "could not look" are
different answers, and only one closes the reachable set.

Promotion is one-way within a state. In particular a `HavocFill::Uninitialized` havoc of an
already-promoted object zeroes its init array; it must **not** drop back to `Bytes`, which
would discard the array contents and answer later reads from stale bytes.

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

### 3.1 The initialization mask

Two properties are required of `init`, and a naive per-byte boolean has neither.

**Bit granularity.** `LoadBits`/`StoreBits` ([020 §4.5.1](020-cir.md)) exist so a bitfield
access touches only its own bits. That is pointless unless initialization is tracked at
the same granularity: writing `a` in `struct { u32 a:3; u32 b:5; }` and then reading `a`
must produce no finding, while reading `b` must produce one — and both fields live in
byte 0. A per-byte mask can only answer "yes" (missing every real uninitialized-bitfield
read) or "no" (firing on every correct one). VPP settles the argument: `session_types.h`
packs nine bitfields into one `u32`, several of them unnamed padding that is never
written.

**A third state.** A write at a *symbolic* offset that stays in `Bytes` (§3, feasible set
≤ `ite_threshold`) writes each candidate byte conditionally: `ite(off == k, val, old)`.
Such a byte is neither definitely initialized nor definitely not. Forcing it to "yes"
silently loses real uninitialized reads; forcing it to "no" produces a false-positive
storm on `v[i] = x; … use v[i]`, which is ubiquitous. Worse, the `Array` path represents
this exactly (`Store(init, off, 1)`), so a two-state `Bytes` mask would make the two paths
disagree on the *same program* purely because the feasible set crossed 16 — which would
falsify contract 6.

```rust
pub struct InitMask { bits: Vec<InitBit> }      // length 8 * size, indexed by BIT
pub enum InitBit { No, Yes, Cond(Term) }        // Cond: initialized iff Term holds
```

`Cond` collapses to `Yes`/`No` whenever its guard folds to a constant, so the common case
costs one enum tag. The `Array` variant's `init: Term` is an SMT array
`BV(ptr_width + 3) -> BV(1)`, bit-indexed to match, and promotion maps `No`→0, `Yes`→1,
`Cond(t)`→`ite(t, 1, 0)` — so promotion preserves initialization state exactly, which is
what makes the two paths agree.

**Symbolic is not uninitialized.** A lazily-materialized object (§6) is fully `Yes` with
unknown *values*. Conflating "we don't know the value" with "nobody wrote it" turns every
UCSE run into a false-positive storm, and is the single easiest way to make this whole
subsystem useless.

**And an unknown value is *one* value.** Once a byte has been materialized, every read of that
address on that path yields the **same term** until something writes it. This was left implicit
until 2026-08-08, when it turned out not to hold: a lazy object plus an unmodeled call gave
`Term(3)` to one read and `Term(27)` to the next, with no faults, so a guard on the first
constrained nothing the second used. That produced 19 of the 44 findings in a `vnet/` sweep — a
guarded array subscript reported as escaping its object, which is the commonest idiom in C.

It is written down because **nothing in this section forbade it**: "unknown values" and contract
7's "no finding" are both satisfied by handing out a fresh symbol each time. The rule that makes
memory memory has to be stated to be testable — see contract 7b, which is now met.

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
    fn read (&mut self, cx: &mut AccessCtx, p: &Pointer, size: u64, ty: CTy) -> AccessResult;
    fn read_bits(&mut self, cx: &mut AccessCtx, p: &Pointer, unit: CTy, bits: BitRange,
                 signed: bool) -> AccessResult;
    fn write(&mut self, cx: &mut AccessCtx, p: &Pointer, val: &Term, size: u64) -> AccessResult;
    fn write_bits(&mut self, cx: &mut AccessCtx, p: &Pointer, val: &Term, unit: CTy,
                  bits: BitRange) -> AccessResult;
    fn copy (&mut self, cx: &mut AccessCtx, dst: &Pointer, src: &Pointer, size: &Term,
             overlap: Overlap) -> AccessResult;
    fn set  (&mut self, cx: &mut AccessCtx, dst: &Pointer, byte: &Term, size: &Term) -> AccessResult;
}

pub struct AccessResult { pub value: Option<Term>, pub faults: Vec<MemFault> }
```

Three things about this signature are load-bearing:

**`&mut self`, even for `read`.** A read mutates: it materializes a `Lazy` object on first
dereference (§6), it memoizes the fresh symbol invented for an uninitialized byte, and it
may force promotion to `Array` (§3). The memoization is not an optimization — 020
contract 10 requires a non-volatile load to yield the *same* value when repeated, so two
reads of one never-written byte must return one term. An immutable `read` returning a new
symbol each time makes `x == x` satisfiably false over uninitialized memory.

**`AccessCtx`, not a bare `Memory`.** Bounds checking adds a constraint to the path
condition, symbolic-base resolution forks states (§5.1), and neither belongs to `Memory`.
`AccessCtx` carries the solver, the path condition, and a fork sink, and is supplied by
the engine.

**Faults alongside a value, not instead of one.** `Result<Term, MemFault>` cannot express
the normal case: an out-of-bounds access continues on the in-bounds branch *and* reports;
a misaligned access is recorded *and* succeeds; an uninitialized read yields a value *and*
a finding. Multiple faults per access are possible (misaligned *and* partially
uninitialized), so `faults` is a vector.

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

0. If the value carries provenance (§7.1) or came from a registered **arena** (§5.2), use
   it directly — no search.
1. Otherwise ask the solver for the set of objects whose address range the pointer value
   can fall in, capped at `max_resolutions` (default 8). The search is over an interval
   tree keyed on base address, not a linear scan: §8 concedes VPP entry points may exceed
   10⁴ objects, and a per-dereference O(objects) solver sweep is not viable.
2. If exactly one, continue with it.
3. If several, **fork** one state per object with the corresponding constraint, plus one
   state constrained to none of them (the wild-pointer state, reported).
4. **If the value is wholly unconstrained** — the feasible set is "every object", i.e. the
   solver has no information at all — the answer is `Fidelity::Unknown` and an
   `UnresolvablePointer` finding. Execution of that path stops.
5. If the set is merely larger than `max_resolutions`, concretize to the solver's model,
   record `Fidelity::Approximated` ([023 §7](023-execution-engine.md)), and say so in
   every result derived from the path.

Steps 4 and 5 must stay distinct. An earlier draft merged them, so an unconstrained
pointer was concretized to an arbitrary object and the run reported `Bounded` — which
reads as "we explored a subset correctly" when in fact execution proceeded through the
whole function reading and writing *the wrong memory*. Every downstream finding, and every
downstream *absence* of a finding, would have been about unrelated bytes. This is the
highest-value instance of the "wrong answer instead of honest unknown" failure the whole
fidelity apparatus exists to prevent.

### 5.2 Arenas

Step 4 would fire on the most-executed function in VPP's data plane. `vlib_get_buffer` is:

```c
/* vlib/buffer_funcs.h */
vlib_buffer_ptr_from_index (uword buffer_mem_start, u32 buffer_index, uword offset)
{
  offset += ((uword) buffer_index) << CLIB_LOG2_CACHE_LINE_BYTES;
  return uword_to_pointer (buffer_mem_start + offset, vlib_buffer_t *);
}
```

Pure `IntToPtr` over an arithmetic term. Under UCSE, `buffer_mem_start` is loaded from a
lazily-materialized structure and is therefore an unconstrained 64-bit symbol — so without
help, every VPP node analysis dies at its first buffer access.

An **arena** is a registered striding region. Three sizes, not two, because in VPP they
genuinely differ:

```rust
pub struct Arena {
    pub base: Term,
    /// Bytes between consecutive elements. Elements are disjoint, so `elem_size <= pitch`.
    pub pitch: u64,
    /// Addressable bytes within an element; `[elem_size, pitch)` is a gap.
    pub elem_size: u64,
    /// Bytes per unit of the *index* the program uses, which is not the pitch.
    pub index_scale: u64,
    pub count: SizeVal,
}
```

`index_scale` exists because `vlib_buffer_ptr_from_index` computes
`base + (buffer_index << 6)` — the index unit is a 64-byte cache line — while consecutive
buffers are `vlib_buffer_alloc_size()` apart, which is
`round_pow2(ext_hdr + sizeof(vlib_buffer_t) + data_size, VLIB_BUFFER_ALIGN)`, on the order
of 2.5 KB. Collapsing the two would force a choice between an `elem_size` of 64 — making
every access to `b->data` an out-of-bounds finding — and overlapping elements, which
violates §7's disjointness. Both are wrong; the layout simply has two different strides.

Resolution of `IntToPtr(base + n)`:

1. `n` is in index units, so the byte offset from `base` is `n * index_scale`.
2. Element index `k = (n * index_scale) / pitch`, byte offset within the element
   `d = (n * index_scale) % pitch`. The decomposition is unique **because `d` is
   canonicalized to `[0, pitch)`** — without that, `base + k*pitch + d` has many readings
   for the same address.
3. `d >= elem_size` lands in the inter-element gap: an OOB finding, not a valid pointer.
4. `k` is bounds-checked against `count`, and may stay symbolic — one object per accessed
   index, materialized lazily, rather than a fork over the whole address space.

`chiero-vpp` registers the buffer pool this way ([060](060-vpp-integration.md)); the
mechanism itself is target-agnostic and is the general answer for any pool, slab or
index-addressed allocator.

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

### 7.1 Round-tripping through integers must not launder provenance

`PtrToInt` yields `addr + off`. The naive inverse — have `IntToPtr` search address ranges
— is **wrong in both directions**, and address-range search alone must never be the
primary mechanism:

- **It converts a real bug into a legitimate access.** Object A (size 100) and object B
  more than one guard gap away. `q = A + 8192` correctly keeps `q.base = A`, and
  dereferencing it is an honest OOB finding. But `r = (u8 *)(uword) q` computes
  `A.addr + 8192`, and the range search *hits B*, returning a valid in-bounds pointer into
  an unrelated object. The out-of-bounds write becomes a silent, legal-looking write to
  the wrong object. Guard gaps only bound OOB distances smaller than the gap.
- **It reports a bug on conforming code.** For a page-aligned object of size exactly 4096,
  the legal one-past-the-end pointer lands in the guard gap, misses every range, and
  becomes a wild-pointer finding with `Fidelity::Unknown`.

So `PtrToInt` **records provenance in the term**: the state carries
`int_provenance: IndexMap<Term, Pointer>`, populated on every `PtrToInt`. `IntToPtr`
consults it first — including through intervening integer arithmetic, by propagating the
tag across `Add`/`Sub` with a constant or with a provenance-free operand. Only on a miss
does it fall back to address-range search, and only on a miss there does it yield
`UNBOUND`.

This is what makes `uword_to_pointer` round-trips exact, and VPP does them constantly.

### 7.2 Pointer-bit inspection

Concrete addresses leak into arithmetic through `PtrToInt`, so a program that inspects its
own pointer bits gets a definite answer manufactured by the bump allocator. Real example,
`plugins/nsim/nsim_input.c`:

```c
if ((((uword) ep) & (CLIB_CACHE_LINE_BYTES - 1)) == 0)
  clib_prefetch_load (ep + 2);
```

Whether this holds depends on the real allocator; chiero decides it concretely from the
region base plus a bump, prunes one branch as infeasible on *every* run, and — because
addresses are deterministic (contract 15) — never looks flaky. The same applies to
`clib_mem_is_aligned` and every `round_pow2((uword) p, …)`. Note this also quietly
violates §7's own claim that no analysis may infer locality from the layout: it is
inferring it, implicitly.

Two mitigations, both required:

1. **Object base addresses are symbolic**, constrained only by `align` and pairwise
   disjointness. The deterministic concrete address of §7 remains, as the **witness value**
   the model reports — so replay and reporting are unchanged, but the solver cannot prove
   an alignment fact the allocator invented.
2. A `PointerBitInspection` event fires whenever a branch condition depends on
   `PtrToInt` bits below the object's guaranteed alignment, so a checker can flag it.
   Without the event, §7's promise to report "when a program's outcome depends on it" is
   unfalsifiable prose.

The same reasoning applies to §6's `distinct_by_default`: distinct concrete addresses
would *decide* `p == q` false for two lazily-materialized parameters that alias in
reality, pruning a real path. Symbolic bases leave it open, and `--fork-on-alias`
explores it.

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
2. Writing `Int(32)` at offset 60 of a 64-byte object succeeds. **Must-OOB and may-OOB are
   distinguished**: at a *concrete* offset 61 the access is out of bounds under every
   model, so it is one OOB finding and the state **terminates** — "continue with the
   in-bounds constraint" would continue a state whose path condition is unsatisfiable,
   which [023 §3](023-execution-engine.md) elsewhere treats as a chiero bug. For a
   *symbolic* offset that may or may not be in bounds, it is one OOB finding with a
   concrete witness and execution continues on the in-bounds branch.
3. `PtrAdd` past the end of an object then back inside yields a pointer that reads
   correctly and preserves `base`.
4. Writing `Float(F32)` `1.0` then reading `Int(32)` at the same address yields
   `0x3F800000` under the little-endian target.
5. Writing bytes 0..2 concretely and 2..4 symbolically, then reading `Int(32)`, yields a
   `Concat` term whose low half is concrete — and the object is still `Contents::Bytes`.
6. A write at a symbolic offset with 3 feasible values keeps the object as `Bytes`; with
   1000 feasible values it promotes to `Array`. For every feasible offset the two paths
   agree on the **`(value, initialization-status)` pair**, not merely the value —
   comparing values alone leaves the disagreement the tri-state `InitMask` exists to
   prevent (§3.1) untested.
6b. A conditional write at a symbolic offset leaves the candidate bytes `InitBit::Cond`,
    not `Yes` and not `No`: a subsequent read at a *definitely different* offset reports
    an uninitialized read, and one at the written offset does not.
7. Reading a never-written stack byte yields exactly one uninitialized-read finding and a
   fresh symbol; reading a lazily-initialized parameter's bytes yields **no** finding.
7b. **A materialized byte reads the same twice.** Two reads of one address on one path, with
    no intervening write, yield the same term — for a lazily-initialized parameter, for a
    havoc'd object, and across a promotion to `Array`. ✅ **Met since 2026-08-08.**
    `materialize_fresh` asked `o.sym_at(k)` — the `Bytes` side — and stored a promoted object's
    mint into `arr.data` only, so the question was put to one representation and answered into
    the other and every read minted afresh. The symbol is now recorded on both. Tested by
    `probe_lazy_two_loads` in `chiero-tool/tests/find_bugs.rs`.
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
12b. **Provenance survives the round trip in the cases address search gets wrong** (§7.1):
     a pointer out of bounds by more than a guard gap round-trips to its *original*
     object, not to the neighbour it would collide with; and a one-past-the-end pointer of
     a gap-sized object round-trips to its own object rather than becoming `UNBOUND`.
     These two are the contract — contract 12 alone only tests the easy case.
12c. Provenance propagates through integer arithmetic: `(T*)((uword)p + 8 - 4)` resolves to
     `p`'s object at offset 4.
13. An `IntToPtr` of the *constant* `0xDEAD` resolves to `UNBOUND` and any access is one
    wild-pointer finding with `Fidelity::Unknown`.
13b. An `IntToPtr` of a **wholly unconstrained symbol** yields `Fidelity::Unknown` and one
     `UnresolvablePointer` finding, and no read through it is ever reported as in-bounds
     (§5.1 step 4). This is the `vlib_get_buffer` case; getting it wrong analyses the
     wrong memory for an entire function.
13c. With an arena registered (§5.2) using VPP's real geometry — `index_scale` 64,
     `pitch` = `vlib_buffer_alloc_size()` (~2.5 KB), `elem_size` the buffer's addressable
     extent — `base + (i << 6)` resolves to element `(i*64)/pitch` at offset
     `(i*64)%pitch`, with `i` symbolic and bounds-checked against `count`, and no fork
     over unrelated objects. Accessing `b->data` well past 64 bytes is **in** bounds.
13d. Elements are disjoint and separated per §7; an offset landing in the
     `[elem_size, pitch)` gap is exactly one OOB finding, not a valid pointer into the
     next element.
14. Two distinct objects never overlap and every pair is separated by ≥ 4096 bytes
    (property test, 10 000 random allocation sequences) — **and** an OOB pointer at
    distance `gap + 1` does not resolve to the neighbouring object (the property that
    actually matters; separation alone is satisfied by spacing objects a megabyte apart).
15. Two runs assign identical witness addresses to identical objects, **and** two
    different path-exploration orders assign identical addresses to the same object.
16. A symbolic base pointer that can refer to 3 objects forks into 4 states (3 resolved +
    1 wild) with mutually exclusive constraints whose disjunction is implied by the path
    condition.
17. With `max_resolutions = 2` the same case concretizes, and the result carries
    `Fidelity::Approximated` (per the [023 §7](023-execution-engine.md) table — discarding
    feasible objects is a modeling lie, not a truncated search) with a note naming the cap.
17b. A branch whose condition depends on `PtrToInt` bits below the object's guaranteed
     alignment emits a `PointerBitInspection` event and does **not** have one side pruned
     as infeasible (§7.2).
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
23. **Bit-granular initialization** (§3.1): for `struct { u32 a:3; u32 b:5; }`, writing `a`
    then reading `a` produces no finding and reading `b` produces exactly one — the
    mirror of [020](020-cir.md) contract 24, pinned on the owning side. A per-byte mask
    fails one half or the other.
24. `StoreBits` to `a` leaves every bit of `b` unchanged including when symbolic, and
    leaves `b`'s init bits untouched.
25. Promotion `Bytes` → `Array` preserves initialization exactly: `No`→0, `Yes`→1,
    `Cond(t)`→`ite(t,1,0)`, verified by comparing reads before and after promotion.
26. **Two reads of the same never-written byte return the same term** and produce exactly
    one uninitialized-read finding, not two (the memoization requirement behind `&mut
    self` in §5).
27. A lazily-materialized object's bytes are `Yes` with unknown values: reading them
    produces no uninitialized-read finding (§3.1's symbolic-is-not-uninitialized rule).
28. `SetMem` marks the written range initialized and readable as the set byte; a
    symbolic-size `SetMem` marks the definitely-covered prefix `Yes` and the rest `Cond`.
29. An alloca in a loop body executed 3 times yields 3 distinct `ObjectId`s with distinct
    `ObjOrigin::Alloca{ frame }`, and a recursive function at depth 32 has 32 live objects
    for the same `AllocaId`.
30. `Lifetime::Function` (`alloca()`) memory is not retired at inner `Scope(Exit)`;
    `Lifetime::Scope` memory is.
31. A `va_list` object round-trips through `VaStart`/`VaArg`/`VaEnd` and is addressable:
    taking its address and passing it to another function that advances it is visible to
    the caller ([020 §4.4.1](020-cir.md)).
32. A function-pointer value stored to a global, loaded, and called indirectly resolves to
    exactly one `FuncId` with no fork (the `ObjKind::Function` contract).
