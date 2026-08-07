# 041 — Optimization analysis

`chiero-opt` finds provable optimization opportunities and — more importantly —
**adjudicates proposed rewrites**. It never patches code.

The centre of this document is one function:

```rust
fn prove_equivalent(before: &Function, after: &Function, cfg: EquivCfg) -> Equivalence
```

That primitive, not the opportunity detectors, is chiero's most valuable output. An LLM
is good at proposing a faster or clearer version of a C function and bad at being sure it
is correct. chiero is bad at inventing rewrites and good at deciding whether two functions
agree. **The LLM proposes; chiero adjudicates** ([050](050-tool-interface.md)).

It is also load-bearing elsewhere: equivalence refinement is test selection's strongest
pruning step ([032 §3.1](032-test-selection.md)).

## 1. `prove_equivalent`

```rust
pub enum Equivalence {
    Equivalent { fidelity: Fidelity, footprint: Footprint, assumptions: Vec<Assumption> },
    Differs { input: Witness, observation: Divergence, replay: Option<Replay> },
    Unknown { reason: String },
}

pub enum Divergence {
    ReturnValue { before: BvConst, after: BvConst },
    Memory { object: ObjectOrigin, offset: u64, before: Vec<u8>, after: Vec<u8> },
    SideEffect { index: u32, before: Option<Effect>, after: Option<Effect> },
    Termination { before: TermReason, after: TermReason },
}
```

### 1.1 What equivalence means

Two functions are **observationally equivalent** when, for every input — parameters,
reachable memory, and the sequence of values returned by unmodeled externs — they agree
on:

1. the return value,
2. the final contents of the **observable footprint**: memory reachable by the caller
   (globals, objects reachable from pointer parameters or the return value) — *not*
   stack temporaries, which is what permits most real refactors;
3. the ordered sequence of observable side effects: calls to unmodeled or effectful
   externs with their arguments, volatile accesses, and abnormal termination.

Definitional choices, each of which changes the answer and so is stated rather than
assumed: local stack contents are not observable; allocation *addresses* are not
observable but allocation/free *events* are; the order of two independent extern calls
**is** observable (C fixes it, and reordering visible I/O is not a safe refactor);
divergence in resource consumption is not observable.

"Allocation addresses are not observable" needs enforcement, not just assertion.
[021 §7](021-memory-model.md) assigns deterministic addresses from a bump allocator, so
two versions that allocate a different number of objects — or the same objects in a
different order — would produce *different pointer values* for corresponding objects and
be reported `Differs` for a difference no C program can detect. That would make
`prove_equivalent` useless on exactly the refactors it is meant to bless.

So comparison is **up to an object bijection**: the two runs' objects are matched by
allocation order within each origin class, and pointer values are compared as
`(matched object, offset)` pairs rather than as integers. A divergence in a pointer
*value* whose `(object, offset)` pair matches is not a divergence. A program that
*observes* raw pointer bits (via `PtrToInt`, per
[021 §7.2](021-memory-model.md)'s `PointerBitInspection`) is the exception: there the
comparison is genuinely undecidable under this definition, and the result is `Unknown`
with that reason, never `Equivalent`.

### 1.2 Method

Relational (product) execution: both functions run against the **same** symbolic inputs
and the same extern-return symbols, paths are paired by input constraint, and the
comparison is a solver query on the disjunction of the three disagreement conditions. A
satisfying model is a distinguishing input.

Loops are bounded by the same `k` in both. This is where honesty is required: for a
function with an unbounded loop, the result is `Equivalent { fidelity: Bounded }` — a
statement about inputs within the bound, not a proof. Only `fidelity == Exact` is a
proof, and only `Exact` licenses dropping a test in [032](032-test-selection.md) or
telling an LLM its rewrite is safe. Loop-invariant reasoning that would lift some cases to
`Exact` is future work, and its absence is reported rather than papered over.

### 1.3 `Differs` is the valuable answer

When the two disagree, the output is a **distinguishing input plus a replay harness**
([040 §3](040-defect-checkers.md)) that compiles both versions and demonstrates the
divergence. "Your rewrite is wrong" is an opinion; "your rewrite returns 0 where the
original returns -1 when `n == INT_MIN`, here is the program" ends the discussion.

The same self-validation applies: the harness is compiled and run, and a divergence the
harness fails to demonstrate is downgraded and flagged, never silently trusted.

## 2. Opportunity detection

Detectors propose; they never rewrite. Each proposal carries the evidence and the
obligations that must hold:

```rust
pub struct Proposal {
    pub kind: OptKind, pub span: Span, pub backtrace: Vec<ExpnFrame>,
    pub rationale: String,
    pub obligations: Vec<Obligation>,   // each Discharged{fidelity} | Open{why}
    pub suggested: Option<String>,      // illustrative C, never applied
    pub expected_benefit: Benefit,      // Measured | Estimated | Unquantified
}
```

Semantic detectors: redundant load (same address, no intervening write or barrier);
dead store; branch whose condition the path condition already decides; loop-invariant
computation; provably-constant expression; redundant bounds or null check (already
implied); call-site specialization where an argument is constant at every site;
unreachable code; unnecessary zeroing of memory immediately overwritten.

**A proposal with any `Open` obligation is advisory and labelled as such.** The honest
statement "this looks redundant but I could not prove the intervening call does not write
it" is more useful than a confident wrong claim, and it is what an LLM needs in order to
decide whether to investigate.

## 3. Cache-line and locality analysis

Caches have no semantic effect ([021 §7](021-memory-model.md)) — but VPP tunes for them
deliberately: `CLIB_CACHE_LINE_BYTES` appears in **257** files and
`CLIB_CACHE_LINE_ALIGN_MARK` in **124**. Layout is knowable statically, and access
frequency is knowable from symbolic execution and coverage, so these are real findings
rather than guesses.

Inputs: `RecordLayout` ([014 §3](014-semantics-and-types.md)),
`TargetConfig::cache_line_bytes`, coverage counts ([030](030-coverage-gcov.md)) as a
weight, the `Sharing` classification from [025 §3](025-concurrency-and-threading.md), and
per-field access counts — which **nothing produced** in an earlier draft. `RunResult`
carried only `BlockCoverage`, and [020 §4.4](020-cir.md) declares `AccessPath`
reporting-only, so the obvious source was barred too. Both are fixed here rather than left
as a dangling input:

```rust
// Added to RunResult (023 §9), populated only when `profile_fields` is enabled.
pub struct FieldAccessProfile {
    /// (record, field offset) -> (reads, writes), summed over executed paths.
    pub counts: IndexMap<(RecordId, u64), (u64, u64)>,
    pub sharing: IndexMap<(RecordId, u64), Sharing>,   // for false sharing
}
```

`AccessPath`'s reporting-only rule is narrowed rather than broken: **no analysis may branch
on an `AccessPath` to decide program semantics**, but aggregating them into a profile that
feeds *advisory* proposals is permitted, because a wrong profile yields a bad suggestion
rather than a wrong answer. Profiling is off by default; with it off, benefit is
`Unquantified` and the hot/cold and false-sharing findings are not produced at all rather
than being produced from nothing.

| Analysis | Finding |
|---|---|
| **Line straddling** | A hot field spans a cache-line boundary — two lines touched for one access. |
| **Hot/cold placement** | Frequently accessed fields sit beyond the first line while cold fields occupy it; proposes a reordering with the measured access counts as evidence. |
| **False sharing** | Two fields in one line where one is `PerThread`-written and the other is written by a different thread's context. This needs 025's classification, which is why that spec came first. |
| **Padding waste** | Alignment padding that a field reorder would recover, with the size delta. |
| **Prefetch distance** | A `CLIB_PREFETCH`/`__builtin_prefetch` whose distance does not match the loop's stride, or a strided loop over a hot structure with no prefetch. |

Two constraints keep this from being dangerous:

- **A reordering proposal must state whether the struct's layout is observable outside the
  program** — wire formats, ABI boundaries, structs with `packed`, and anything reaching
  a serialization path. Reordering an `ip4_header_t` is a protocol violation, not an
  optimization. When chiero cannot prove the layout is internal, the proposal is advisory
  and says so prominently.
- **Benefit is labelled honestly.** `Measured` requires access counts from a real run;
  otherwise it is `Estimated` or `Unquantified`. chiero has no cycle model and will not
  pretend to one.

### 3.1 Bit-fields — the field description carries bits, and the reorder never repacks them

A bit-field's extent is *bits inside a storage unit its neighbours share*, which `(offset,
size)` cannot express. Dropping such a member and summing the rest is how a 72-byte struct
was told it "would be 8" (§7.7 of the handoff), so the padding proposal was withheld for
any record holding one — right, and a hole: those are exactly the packed, hand-tuned
structs where padding matters most.

So the field description carries an optional **bit extent** — bit offset from the start of
the record, and width in bits, [014 §3](014-semantics-and-types.md)'s `BitField` verbatim
— and the analysis models it as follows:

- **A maximal run of consecutive bit-fields is one synthetic member** for the padding
  arithmetic, spanning the first byte its first bit falls in through the last byte its last
  bit falls in. Runs break at any non-bit-field member, exactly as gcc's allocation units
  do, so the run's byte extent is a fact about the declared layout rather than a model of
  the packing rules — which this crate must not re-derive.
- **The reorder never repacks bits.** Moving the run as a unit is achievable by
  declaration order alone; claiming the bits could be repacked tighter would require gcc's
  straddling rules, and an unachievable floor is a number in the flattering direction. So
  the recovered bytes are a lower bound, and the proposal is one gcc can be asked to check.
- **Straddling still sees each bit-field individually**, since that finding is a statement
  about one member and its byte extent is enough to make it.

A record with a member the frontend could not size at all is still partial and still gets
no padding number; a bit-field is no longer one of those cases.

## 4. What this crate will not do

No auto-patching, ever — proposals are text. No performance *measurement*; chiero is not a
profiler and does not model pipelines, branch prediction or memory bandwidth. No
whole-program optimization. No proposal without either a discharged obligation or an
explicit advisory label.

## 5. Testable contracts

### `prove_equivalent`

1. A function and its verbatim copy are `Equivalent` with `Exact`.
2. `return a + b;` vs `return b + a;` is `Equivalent` with `Exact`.
3. `x * 2` vs `x << 1` on a signed 32-bit input is `Equivalent` with `Exact`;
   `x / 2` vs `x >> 1` is **`Differs`**, with a negative distinguishing input.
4. `abs(x)` written two ways is `Differs` at `INT_MIN`, with that witness.
5. Renaming locals, reordering independent statements, and hoisting an invariant out of a
   bounded loop are each `Equivalent`.
6. A rewrite that changes the order of two `printf` calls is `Differs` with
   `Divergence::SideEffect`.
7. A rewrite that leaves different garbage in a stack temporary is `Equivalent` (stack
   temporaries are outside the footprint), while one that leaves different bytes in a
   caller-visible buffer is `Differs` with `Divergence::Memory`.
8. A rewrite that frees an object the original did not is `Differs`.
9. A function with an unbounded loop returns `Equivalent { fidelity: Bounded }` and the
   bound is stated; it is **not** accepted as a proof by
   [032 §3.1](032-test-selection.md), verified by a test that asserts the test is kept.
10. Every `Differs` produces a distinguishing input, and a replay harness that compiles
    and demonstrates the divergence for every case in the corpus.
11. A `Differs` whose harness fails to demonstrate the divergence is downgraded and
    flagged, and the event is counted.
12. Solver timeout yields `Unknown` with a reason — never `Equivalent`.
13. `prove_equivalent` is symmetric: swapping the arguments yields the same verdict and a
    correspondingly swapped witness.
13b. **An always-`Unknown` implementation must fail this suite.** Contracts 1, 2, 3, 5 and
     7 each require a definite `Equivalent`, and 3, 4, 6, 7 and 8 each require a definite
     `Differs` with a witness — so neither degenerate answer passes. A tracked metric
     records the `Unknown` rate over the corpus and fails CI on regression.
13c. A rewrite that allocates two objects where the original allocated one, but is
     otherwise identical and returns the same values, is `Equivalent` — pointer values are
     compared up to the object bijection (§1.1), not as integers.
13d. A rewrite that changes only the *order* of two independent allocations is
     `Equivalent`; one that changes which object a returned pointer points *into* is
     `Differs`.
13e. A function whose result depends on raw pointer bits (`(uword)p & 63`) yields
     `Unknown` naming pointer-bit observation — never `Equivalent`.

### Opportunity detection

14. A redundant load across a call to a function proven not to write the address is
    proposed with all obligations `Discharged`; across an unmodeled extern it is proposed
    with an `Open` obligation and labelled advisory.
15. A branch whose condition is implied by the path condition is proposed as dead, with
    the implying constraints listed.
16. Every proposal in the corpus has either a discharged obligation or an advisory label
    (structural check over all proposals).
17. No API in `chiero-opt` writes to a source file (checked by absence of write calls, and
    by a fixture asserting the crate exposes no patch operation).

### Locality

18. A 64-bit field at offset 60 with `cache_line_bytes == 64` is reported as straddling; at
    offset 56 it is not.
19. A struct whose hottest field (by execution count) is in the third line yields a
    reordering proposal listing the access counts as evidence.
20. Two fields in one line, one written under `PerThread` and one from another thread's
    context, produce exactly one false-sharing finding; separating them by a line
    produces none.
21. A struct reachable from a serialization path, or marked `packed`, yields only an
    advisory reordering proposal, explicitly stating the layout may be externally
    observable.
22. A benefit is labelled `Measured` only when backed by real access counts; with no
    execution data the same proposal is `Estimated` or `Unquantified`.
23. A strided loop with no prefetch over a hot structure is reported; adding a matching
    `CLIB_PREFETCH` silences it.
24. All proposals and their order are byte-identical across runs.
25. **Bit-fields (§3.1).** `struct { char tag; int big; unsigned a : 1; unsigned b : 1;
    unsigned c : 1; unsigned d : 1; }` — 12 bytes, gcc — yields a padding proposal of **4
    recoverable bytes**, and the reordered declaration gcc is handed to check it is 8. That
    fixture rather than a rounder one **because it discriminates**: counting each bit-field
    as its own byte sums to 9, rounds to 12, and produces no proposal at all, so an
    assertion on a struct where the two models agree would pass without seeing anything.
    The run `a`…`d` appears in the hole evidence under a name that says it is a bit-field
    run, not under one member's name alone. `struct { char tag; unsigned a : 3; unsigned b
    : 5; long big; }` — 16 bytes — yields **no** padding proposal, because there is nothing
    to recover, and **no** blind spot saying the record could not be judged: the two must
    stay distinguishable. A record holding a member the caller could not size at all still
    yields neither the number nor silence about it.
