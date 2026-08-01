# chiero-rs — HANDOFF

> Working state for a fresh context. Read this top to bottom, then continue at
> **§9 Next actions**. Everything below is decided, not open for re-litigation
> unless the user says so.

---

## 1. The ask (verbatim intent)

Build a **symbolic C execution environment library in Rust** — modular and reusable —
that supports **macros**, targeting **VPP** (https://gerrit.fd.io/r/vpp) as the real
codebase. It must:

1. Ingest **gcov test coverage**.
2. Given a change, **figure out which tests to run**.
3. **Spot logic errors** and **perform code optimizations**, usable **by an LLM as a tool**.

Process the user asked for, explicitly:
- Fully spec it into `docs/specs/` first.
- Then implement in **fully autonomous red-green TDD**: commit RED (failing test),
  then implement and commit GREEN.
- **Subagent adversarial reviews on both**, each producing supplemental commits.

The user also asked (mid-turn) that I write this handoff, commit it, and use
`mcp__tttt__tttt_clear_and_read_handoff_md` to voluntarily refresh context whenever
useful. **That is now standing permission — use it at natural milestone boundaries,
not mid-edit.** Always re-commit an updated HANDOFF.md before refreshing.

## 2. User's three decisions (asked and answered 2026-07-26)

| Decision | Chosen | Rejected | Why it matters |
|---|---|---|---|
| **C frontend** | **Own preprocessor + own parser** | clang/libclang-backed; own-pp + `lang-c` | See the corrected rationale below — the decision stands, the original reason did not. |
| **Build order** | **Symbolic core first** | gcov→test-selection first (my rec.); logic-errors first | User overrode my recommendation. Honor it: IR + memory + solver + engine proven on small C before the coverage verticals. |
| **Autonomy** | **Spec gate, then run free** | fully autonomous now; checkpoint every milestone | Specs get user review BEFORE implementation. After approval: no check-ins until the first vertical is green. |

**Gate discharged 2026-07-27** — see §9.

### 2.1 ⚠️ The frontend rationale was wrong — corrected 2026-07-27

The Fable reviewer tested the claim and it does not hold. **clang 18.1.3 does provide
macro provenance**, verified on this machine: a diagnostic inside a nested expansion
prints the *full* chain (`expanded from macro 'vec_add1'` → `'vec_add1_ha'`) with both
definition sites; `isMacroArgExpansion`/`isMacroBodyExpansion` give per-token argument
attribution; libclang emits `MACRO_DEFINITION`/`MACRO_INSTANTIATION` records (nested
expansions need `PPCallbacks` from the C++ API).

So "clang cannot give us macro provenance" was **false**, and 010 §1.1 now says so
explicitly rather than quietly dropping it.

**The decision still stands**, on the reason §3 already records as a hard constraint:
chiero must be a pure-Rust library that links nothing and runs with
`--no-default-features`. Depending on libclang for a *core* capability forfeits that.
Secondary reasons: diffing two revisions including non-compiling ones, `Span` as a 12-byte
`Copy` value rather than a handle into a foreign object graph, and owning lowering.

**Do not restore the old taboo.** A clang-subprocess provenance extractor is a legitimate
fallback *for the impact/selection vertical specifically*, and 010 §1.1 records it as
such. M2 is the riskiest milestone; a contingency resting on a claim that fails a
five-minute experiment is worse than no contingency.

## 3. Environment facts (verified, don't re-derive)

- Working dir `/home/ubuntu/rust/chiero-rs`. Git repo exists, branch `master`,
  **no commits yet** at time of writing.
- **VPP already checked out at `/home/ubuntu/vpp`** — HEAD `7fe9c2669396`, 2026-07-17.
  Do not clone from gerrit (the bare URL 404s to curl anyway).
  Scale: 1552 `.c`, 924 `.h`, ~1.01M lines, **754 distinct `foreach_*` X-macros**,
  340 `VLIB_REGISTER_NODE` sites.
- `gcc`/`gcov` **13.3.0** present.
- **VERIFIED 2026-07-26 (do not re-check):** `clang` 18.1.3 at `/usr/bin/clang`, working.
  `z3` **4.8.12 at `/usr/bin/z3`, working** — `z3 -in -smt2` over stdin smoke-tested
  (`sat` + model returned). `libz3.so`/`libz3-dev` also present, so the later
  off-by-default `z3-sys` feature is buildable. The user confirmed "z3 is in btw".
- **This does not change the frontend decision** — but read **§2.1**: "provenance is why
  we own the preprocessor" was tested and is false; the real reason is the pure-Rust
  no-link constraint in the last bullet here. What clang and z3 being present changes:
  - clang becomes a **differential oracle**: `clang -E` for preprocessor conformance,
    `-ast-dump` for parser cross-checking, alongside gcc.
  - z3 becomes a **first-class solver backend** and a cross-check for `solver-lite`,
    instead of a hypothetical.
  - **Hard constraint retained**: chiero never *links* clang or z3 and must build and
    run fully without them. They are oracles and optional accelerators only, behind
    subprocess boundaries or off-by-default features. If a core capability ever requires
    an external toolchain, the "modular reusable library" property is gone.
- Rust 1.97.1, cargo 1.97.1. crates.io reachable.
- 12 cores, 251 GB RAM. Parallel test/agent work is cheap.
- Scratchpad: `/tmp/claude-1000/-home-ubuntu-rust-chiero-rs/7452d602-bc54-4f42-b1e5-54f072255730/scratchpad`

## 4. Design digest

> ⚠️ **SUPERSEDED WHERE IT CONFLICTS.** All 24 specs are now written; `docs/specs/` is
> normative and this digest is a historical summary that has *already drifted* (§4.2's
> `Expansion` shape, §4.4's CIR sketch, and §4.13b's `x/0` rule are all out of date).
> Read the spec, not this, for anything you are about to implement. The instruction
> "if a spec is missing, write it from this" no longer applies — none are missing.
> §4.13b/c below remain useful as a record of *decisions and their reasons*.

### 4.1 The differentiating idea

When someone edits the **body** of `vec_add1` in `vppinfra/vec.h`, gcov attributes **no
line** to `vec.h` — coverage records the `.c` line where the macro was *used*. A
coverage-only tool cannot answer "which tests are affected". chiero can, because it
owns the preprocessor and retains every expansion site of every macro. **Headline this
everywhere. It is the justification for the entire hand-written frontend.**

### 4.2 Provenance model (rustc-style hygiene, proven shape)

```rust
struct BytePos(u32);
struct Span { lo: BytePos, hi: BytePos, ctx: ExpnCtx }
struct ExpnCtx(u32);           // 0 == ROOT == written literally in a file

struct Expansion {
    parent:   ExpnCtx,          // nesting
    macro_id: MacroId,
    call_site: Span,            // may itself live inside an expansion
    arg_spans: Vec<Span>,
    kind: ObjectLike | FunctionLike | Builtin | Pragma,
}
```
`SourceMap` owns `files: Vec<SourceFile>` and `expansions: Vec<Expansion>`.
Two required queries: `spelling_span()` (where the text literally is, maybe in a macro
body) and `expansion_span()` (walk ctx→ROOT; **this is what gcov sees**).
Plus the reverse index `MacroId -> [expansion sites]`, which powers §4.8.

### 4.3 Crate graph

`chiero-span` → `chiero-lex` → `chiero-pp` → `chiero-ast` → `chiero-parse` →
`chiero-sema` → `chiero-cir` (lowering).
Independently: `chiero-solver`, `chiero-mem`, `chiero-exec`, `chiero-model`.
Verticals: `chiero-gcov`, `chiero-diff`, `chiero-select`, `chiero-check`, `chiero-opt`.
Surfaces: `chiero-tool` (MCP/JSON-RPC), `chiero-cli`, `chiero-vpp`.
Rule: no crate depends on a vertical; verticals depend on the core. No cycles.

### 4.4 CIR — the contract boundary

**Architecturally load-bearing**: CIR is specified independent of the frontend so the
**entire symbolic core is built and TDD'd against hand-written CIR before the parser
exists.** This is what makes "symbolic core first" tractable. Do not let the engine
acquire a dependency on the parser.

Shape: unstructured CFG, three-address, **not SSA** — locals are `alloca` + load/store
(like LLVM `-O0`), so lowering is trivial and the memory model carries the weight.
Optional mem2reg later.
```
Function { params, allocas, blocks: Vec<Block>, entry: BlockId }
Block     { insts: Vec<Inst>, term: Terminator }
Inst      ::= Assign(ValueId, RValue) | Store{addr,val,ty} | Call{..} | Marker(Span)
RValue    ::= Const | Load{addr,ty} | Bin(op,a,b) | Un(op,a) | Cast{..} | AddrOf(Place)
Terminator::= Goto | Br{cond,t,f} | Switch | Return | Unreachable
```
Every `Inst` carries a `Span` (with `ExpnCtx`). Every `Block` carries its gcov line set.
C's unspecified evaluation order: pick a canonical order AND flag order-dependent
programs in a dedicated UB-detection mode.

### 4.5 Memory model — object/offset (KLEE-style)

```
MemObject { id, size: SymVal, kind: Global|Stack|Heap|Extern, bytes }
Pointer = (base: object id, offset: SymVal)
```
Concrete offset → direct byte update (fast path). Symbolic offset → SMT array theory or
fork over feasible objects.

**Validating use case, must be in the spec**: vppinfra vectors are `header | elements`
with the user pointer at element **0**, so `_vec_len(v)` reads at a **negative offset**.
Object+offset handles this naturally when the object spans header+elements. Any memory
model that can't do negative-offset-within-object is wrong for VPP.

**UCSE (under-constrained)**: start at an arbitrary function; unknown pointer params and
globals are **lazily initialized** — materialize a fresh symbolic object on first
dereference. Essential; you cannot reach VPP internals from `main`.

### 4.6 Solver

```rust
trait Solver { fn check(&mut self, assumptions:&[Term]) -> Sat(Model)|Unsat|Unknown;
               fn push(&mut self); fn pop(&mut self); fn declare_const(..); }
```
Sorts: bitvectors, bool, arrays(BV→BV). Floats deferred to v2 as a `Float` sort.
Backends, tiered — `TieredSolver` escalates on `Unknown`:
1. `solver-lite` (built-in): constant folding + interval/bitmask abstract domain +
   simple congruence closure. Fast, incomplete, resolves most real path conditions cheaply.
2. `solver-smtlib`: writes SMT-LIB2 to a **subprocess** (z3/cvc5/bitwuzla if ever
   installed). Zero link-time dependency — this is why it's a subprocess, not FFI.
3. `z3-sys` behind an off-by-default feature, later.

Caching: path-condition→result cache plus **constraint-independence slicing** (KLEE
counterexample cache). Big win, spec it properly.

### 4.7 Execution engine

Path-explosion control: depth limit, loop-unroll bound `k`, state budget.
Pluggable `Searcher` trait: DFS, BFS, coverage-guided (random-path + coverage-new).
State merging deferred.

**Fidelity annotation on every result** — `Exact | Bounded | Approximated | Unknown`.
"No bug found" from a truncated exploration must NEVER be reported as "no bug exists."
This is a hard rule, not a nicety.

### 4.8 Change-impact & test selection

1. Diff → changed **entities** (functions, macros, types, globals) via AST + provenance.
2. **Impact closure**: direct body changes; **macro closure** (functions expanding a
   changed macro — the §4.1 feature, via the reverse expansion index); type/layout
   changes; transitive callers; global readers.
3. Intersect with the coverage index (line→tests) → candidate tests.
4. **Symbolic refinement**: for a changed condition, prune tests whose path condition
   provably cannot observe the change. This is where symbolic execution earns its keep.
5. **Safety net always-run set**: tests with no coverage data, new tests, build-config
   touches.
6. Output is a **ranked list with a per-test justification**. Auditability is a
   requirement, not a feature.

### 4.9 gcov ingest

Two paths, both supported:
- **`gcov --json-format` output** — easy on-ramp, implement FIRST.
- **Native `.gcno`/`.gcda`** — precise. gcno: ANNOUNCE_FUNCTION/BLOCKS/ARCS/LINES;
  gcda: FUNCTION/COUNTER_ARCS; then reconstruct block counts from arc counts via the
  classic spanning-tree solve. Formats are gcc-version-specific — target **13.3**.

Per-test attribution: run each test with a separate `GCOV_PREFIX` → separate gcda sets.

### 4.10 Checkers & optimization

Checkers as a pluggable `Checker` trait observing execution events: null deref, OOB
(object bounds are known!), UAF, double-free, leak, signed overflow, shift UB,
div-by-zero, uninitialized read, `ASSERT`/`clib_error` failure. VPP-specific: `vec_`
bounds, `pool_` index validity, buffer-index misuse.

**Every report needs a concrete counterexample (from the model) AND a replay harness** —
`chiero-replay` emits a compilable C harness that reproduces it. This is what separates
chiero from an LLM guessing.

Optimization analysis: redundant loads/stores, dead branches (path condition proves
untaken), loop-invariant code, provably-constant expressions, redundant bounds checks,
call-site specialization. Emit **proposals with proof obligations**; never auto-patch.

### 4.11 LLM tool interface — the key insight

The valuable primitive is NOT "chiero suggests an optimization". It is
**`prove_equivalent(fn_before, fn_after)` → proof | distinguishing input**.
**An LLM proposes; chiero adjudicates.** Build every optimization/refactor feature on
that primitive. Other tools: `explain_macro_expansion(file,line)` (huge for VPP),
`symbolic_run`, `check_reachable(fn,line,assumptions)`, `select_tests(diff)`,
`find_bugs(fn)`, `get_cfg`.

### 4.12 VPP integration

`CMAKE_EXPORT_COMPILE_COMMANDS=ON` → `compile_commands.json`. Model vppinfra intrinsics
(`clib_mem_alloc`, `vec_*`, `pool_*`, `clib_bitmap_*`, `CLIB_PREFETCH`, `__builtin_*`).
Handle the `foreach_*` X-macro idiom explicitly — it's the dominant pattern (754 of them).
**Multiarch**: VPP compiles the same source multiple times under different
`CLIB_MARCH_VARIANT` → one source maps to MANY TUs. Don't assume 1:1.

### 4.12b Measured VPP extension budget (don't re-derive; use `grep -rF`, not `-rE`)

Counted by file over `/home/ubuntu/vpp/src` @ `7fe9c26`. **My first pass used `grep -rE`
with broken escaping and reported false zeros for `({` and `asm`** — always cross-check
a zero with `grep -rF`.

| Extension | Files | Verdict |
|---|---|---|
| designated initializers | 1019 | required |
| zero/flexible arrays `[0]` | 1165 | required |
| `__attribute__` | 155 | required |
| `_Static_assert` (VPP `STATIC_ASSERT`) | 140 | required |
| statement exprs `({...})` | **217** | required |
| `typeof`/`__typeof__` | 52 | required |
| `asm`/`__asm__` | **31** | parse, do NOT model — opaque effect + `Fidelity::Approximated` |
| `__builtin_*` | 30 | required |
| `__restrict` | 6 | trivial |
| case ranges | 7 | required |
| `__int128` | 1 | required |
| `__label__`/nested fns | 1 | **not supported** — diagnose + skip fn |
| `_Generic`, `__extension__`, `__auto_type` | 0 | defer |

Attributes by frequency: packed 112, unused 85, constructor/destructor 51, aligned 31,
weak 27, always_inline 15, fallthrough 6, visibility 3, vector_size 2, section 2,
may_alias 2, noinline 2. **Only `packed`, `aligned`, `may_alias`, `vector_size` change
analysis semantics**; the rest are recorded and ignored.
Top builtins: shufflevector 25, shuffle 9, prefetch 9, expect 4, clzll 4, unreachable 3,
frame_address 3, ctz 3, constant_p 3, clz 3, object_size 2, mul_overflow 2, bswap64 2.

### 4.13b Decisions locked while writing the core block (020–024) — do not re-litigate

- **CIR pointers are untyped**; the pointee type lives on `Load`/`Store`. Signedness is a
  property of the *op* (`SDiv`/`UDiv`, `SExt`/`ZExt`), not of `Int(N)`.
- **No aggregate values** in CIR; struct assignment is `CopyMem`.
- **`PtrAdd` is a distinct RValue**, never integer `Add` — that is what preserves
  provenance and lets OOB fire on the arithmetic.
- **CIR arithmetic is total and defined** (wrapping, `x/0` = all-ones). UB is *reported as
  a finding*, never encoded as IR partiality or solver "no value".
- **Textual `.cir` format is normative**; core tests are `.cir` fixtures; round-trip is a
  contract; unknown directives are a hard parse error.
- **Memory `Contents` starts as `Bytes` + init bitmask**, promotes to SMT `Array` only on
  a symbolic-offset write past `ite_threshold` (16). Symbolic ≠ uninitialized — conflating
  them makes UCSE a false-positive storm.
- **Objects get deterministic concrete addresses** with 4096-byte guard gaps (globals
  `0x1_0000_0000` up, heap `0x2_0000_0000` up, stack `0x7fff_0000_0000` down, lazy
  `0x4_0000_0000` up). No randomization, ever.
- **Lazy objects are distinct by default** (`--fork-on-alias` opt-in), and the assumption
  is recorded in every report.
- **solver-lite may only answer `Sat` with a model that passes independent concrete
  evaluation, and `Unsat` only from a real emptiness/congruence proof.** Everything else
  is `Unknown`. That is what makes incompleteness safe.
- **z3 is tier 2 via a long-lived `z3 -in -smt2` subprocess** (never linked), plus a
  `paranoid` mode that cross-checks every tier-1 answer. CI runs `--no-default-features`.
- **Fidelity rule is type-enforced**: proof-carrying results need an `ExactWitness` token
  only the engine can mint, so "no bug found" cannot be stated as "no bug exists". There
  is a `trybuild` compile-fail test for it.
- **Unmodeled externs havoc loudly** (invalidate pointees, fresh return, `Approximated`,
  named in `assumptions`). There is no quiet default. `longjmp` is diagnosed unsupported.
- **Harness intrinsics** (`chiero_make_symbolic`/`assume`/`assert`) compile to no-ops
  under gcc, so one corpus file serves both chiero and the differential oracle.

### 4.13c User challenges raised 2026-07-26/27 and how they were answered

The user reviewed 020 mid-writing and raised three gaps. All three are now spec'd; do not
re-open them, but do keep them satisfied as later specs are written.

1. **"How does CIR capture VPP's rich union / layout-dependent semantics?"**
   Answer: it falls out of three existing commitments — no aggregate values, untyped
   pointers (access type on the access), byte-level memory with no strict-aliasing
   assumption. Written up as **020 §4.5**. *Real gap found and fixed*: bitfields needed
   dedicated `LoadBits`/`StoreBits` + `BitRange` (020 §4.5.1), because byte-granular
   lowering both clobbers neighbouring bitfields' symbolic bytes and produces spurious
   uninitialized-read findings on adjacent fields. That forced **bit-granular init
   tracking** for partially-written bytes in 021 §3.1. Also added: `union-pun` checker is
   **off by default** (VPP relies on gcc-defined punning); `PathStep::UnionMember { view }`
   so reports name which union view was used; endian-conditional bitfield layouts are two
   `ConfigId`s, not a CIR concern.
2. **"Do cache behaviours need modeling?"**
   Answer: **no, not semantically** — caches are coherent, so no load's value depends on
   them; 021 §7 now states addresses are logical with no timing/physical meaning.
   **But yes as a performance analysis**: `TargetConfig::cache_line_bytes` added in 014 §1,
   consumed by **041** for cache-line straddling, hot/cold field placement, false sharing,
   prefetch distance. VPP touches `CLIB_CACHE_LINE_BYTES` in 257 files and
   `CLIB_CACHE_LINE_ALIGN_MARK` in 124 — this is a real capability, not a footnote.
   **041 must deliver it.**
3. **"Workers run multi-threaded."**
   Answer: the one-line non-goal was too glib. New spec **025-concurrency-and-threading.md**
   (spec count is now **23**, not 22). Measured VPP discipline: `thread_index` in 467
   files but `clib_spinlock_t` only 69, `clib_atomic_*` 73, barrier 54, `clib_rwlock_t` 10,
   `__thread` 20 — i.e. VPP *partitions* rather than shares, and explicit sync is rare and
   localized. So v1 ships a **path-sensitive concurrency-discipline checker** (unguarded
   shared write/RMW, inconsistent guarding, lock leak on error-return paths, lock-order
   inversion, barrier-protected state touched from a worker, per-thread key mismatch,
   missing barrier around config-time `vec_`/`pool_` mutation) plus a `ThreadCtx`
   (Main | Worker{symbolic index} | Barrier | Unspecified) — **not** an interleaving
   explorer. Hard rule: any result touching a `Shared` object is capped at `Bounded`.
   v2 hooks: IR needs no change, state is already the scheduling unit, `Searcher` already
   abstracts order, and the `Sharing` lattice is exactly DPOR's input.
4. **"We should be able to mechanically check that a function is used according to a
   prescribed ritual — e.g. the CLI parsing ritual — so maybe a module that ingests
   recipes and checks adherence."**
   Accepted as a **new capability vertical**: **042-conformance-recipes.md** + a new crate
   **`chiero-recipe`** (spec count now **24**; 001 §2/§4/§6 updated, new dep rule 7 lets
   `chiero-recipe`/`chiero-diff` use frontend crates for the typed AST).
   Measured motivation: `VLIB_CLI_COMMAND` 1187 sites/358 files, `clib_error_return` 3350,
   `unformat_line_input` 407 vs `unformat_free` 537, `pool_get`/`pool_put` 635/548,
   `vec_free` 3034. Worked example in the spec is `plugins/memif/cli.c`, where
   `vec_free(socket_filename)` is hand-repeated on **five** return paths.
   Design decisions locked: **two-tier evaluation** (tier-1 structural AST sweep over the
   whole repo → candidate set → tier-2 symbolic execution on candidates only; this is the
   only way it scales to 1552 .c files, and unescalated candidates force `Bounded`);
   C-flavoured `.recipe` DSL with `$metavars`, typestate blocks, `on_all_paths`
   obligations, and **explicit `via macro` / `expanded` matching modes**;
   **fixtures are mandatory — a recipe without a passing `good` and a failing `bad`
   fixture fails to LOAD**, which is what stops rule rot and stops an LLM shipping a rule
   that matches nothing and reports "compliant"; baseline + `chiero:allow(reason:)` with
   a *required* reason for adoption; **no severity-downgrade knob**. `chiero-recipe`
   contains zero VPP content — the catalogue is `.recipe` data in `chiero-vpp`.
   This is also the cleanest instance of §4.11's "LLM proposes, chiero adjudicates", so
   **050 owes `validate_recipe`/`apply_recipe`.** (`propose_recipe` was specified and then
   removed on review: chiero generating recipe text inverts §1's "LLM proposes, chiero
   adjudicates" and it has no language model.)

### 4.13 Testing strategy

**Primary oracle is differential against gcc** (gcc 13.3 is installed, clang is not):
compile+run a C snippet concretely, run the same under chiero with inputs concretized,
compare results. Strong oracle, cheap to run, catches real semantic bugs.
Also: `tests/corpus/` of C snippets with expected results; snapshot/golden tests.
Csmith-style fuzzing noted as later work.

## 5. Non-goals (keep saying no to these)

Soundness-as-a-verifier; full C23 or any complete compiler dialect (target = C11 + the
GNU extensions VPP actually uses); C++; being a compiler (no codegen backend);
replacing the build system; automatic patching; **interleaving exploration / weak memory /
lock-free verification** (v1 executes sequentially; see 025 and §4.13c #3 — the blind spot
is declared in every report AND caps fidelity at `Bounded`, but v1 does still ship a
concurrency-discipline checker, so "no concurrency support" is now the wrong summary);
**cache/timing semantics** (coherent, no semantic effect — but cache-*layout* analysis is
in scope for 041, see §4.13c #2).

## 6. Repo layout

```
chiero-rs/
├── crates/           one dir per crate (§4.3)
├── docs/specs/       the normative specs
├── tests/corpus/     shared C programs + expected results
├── tests/differential/  gcc-vs-chiero harness
└── xtask/            build/bench/corpus automation
```

## 7. Spec set — status

**All 25 documents written.** (025 and 042 were added in response to user review — §4.13c;
015 was added in response to the Fable review, which found that no document owned C→CIR
lowering or the computation of `Block::gcov_lines`, while M1's hand-written fixtures would
have entrenched conventions real lowering then had to match.)

- [x] `README.md` — index + reading order + status
- [x] `000-overview.md` — goals, 4 capabilities, design commitments, non-goals, glossary
- [x] `001-architecture.md` — crate graph, dependency rules, CIR-as-contract-boundary
- [x] `010-source-and-provenance.md` — **the crown jewel**; §4.2, worked `vec_add1` example
- [x] `011-lexer.md` — translation phases 1–3, pp-tokens
- [x] `012-preprocessor.md` — phase 4, expansion w/ provenance, `#if` eval, includes
- [x] `013-parser.md` — C11 + GNU extensions, **grounded in measured VPP usage** (below)
- [x] `014-semantics-and-types.md` — types, layout/ABI, name resolution, const-eval
- [x] `015-lowering.md` — **added after Fable review**; AST→CIR conventions, scope-marker
      placement, and §5 **owns `Block::gcov_lines`** (must be settled before M1 fixtures)
- [x] `020-cir.md` — §4.4; +textual format, verifier, PtrAdd-not-Add, order-sensitivity
- [x] `021-memory-model.md` — §4.5; +vec negative-offset worked example, lazy init, CoW
- [x] `022-solver.md` — §4.6; z3 4.8.12 verified as tier-2 subprocess + paranoid oracle
- [x] `023-execution-engine.md` — §4.7; fidelity enforced by an unconstructible-token type
- [x] `024-environment-models.md` — libc/builtins registry, harness intrinsics
- [x] `025-concurrency-and-threading.md` — **added mid-flight** (§4.13c #3); ThreadCtx +
      discipline checker + declared blind spot + v2 hooks
- [x] `030-coverage-gcov.md` — §4.9; formats **empirically verified** against gcc 13.3
- [x] `031-change-impact.md` — §4.8 steps 1–2; +Cosmetic-change class, completeness lattice
- [x] `032-test-selection.md` — §4.8 steps 3–6; +"drop only on Exact proof" rule, safety/reduction eval harness
- [x] `040-defect-checkers.md` — §4.10 checkers + replay; **replay is compiled and run under ASan/UBSan and the verdict recorded** (ReplayVerdict) — self-validation loop
- [x] `041-optimization-analysis.md` — `prove_equivalent` as THE primitive (032 depends on it) + cache-line/locality analysis (§4.13c #2 discharged)

- [x] `042-conformance-recipes.md` — **added mid-flight** (§4.13c #4); recipe DSL,
      two-tier evaluation, mandatory fixtures, LLM propose→adjudicate loop
- [x] `050-tool-interface.md` — §4.11 + recipe ops; **one result envelope with fidelity/proven/blind_spots** so an LLM cannot read "[]" as "safe"
- [x] `060-vpp-integration.md` — §4.12; multiarch 1:N, real vppinfra code not summaries, node tables narrow indirect calls, staged adoption
- [x] `070-testing-and-tdd-protocol.md` — §4.13 + red/green/review loop + the consolidated CI gate table
- [x] `080-roadmap.md` — M0–M8 with checkable exit gates; M1/M2 run in parallel

**Every spec must end with a `## Testable contracts` section** — numbered, checkable
assertions. Those become the RED tests. This is what makes the specs actually drive TDD
rather than decorate it.

Style: dense, decisive, concrete Rust type sketches. No filler, no "we might consider".

## 8. The TDD protocol (to formalize in 070)

Per feature, in order:
1. Write failing test → commit `red: <desc>`.
2. **Adversarial subagent review of the test**: is it tautological? does it pin the
   *right* contract? does it test behavior or implementation? → supplemental commit if needed.
3. Implement → commit `green: <desc>`.
4. **Adversarial subagent review of the implementation**: correctness, does it *cheat the
   test*, edge cases, spec conformance → supplemental commit if needed.

Commit message prefixes: `red:`, `green:`, `review:`, `spec:`, `chore:`.
The user authorized subagents specifically for these reviews. (General standing
instruction otherwise discourages unrequested subagent use — this is the carve-out.)

### 8.1 Subagent rules the user set 2026-07-27 — standing

- **Max 2–3 concurrent agents.** Not more. The user asked for this explicitly after I
  launched 5 at once. Queue the rest and run them in waves.
- **Don't kill already-running agents to enforce a new limit** — their prompt cache goes
  cold and the work is wasted. The user clarified that limits apply to *future* launches;
  let in-flight agents finish. (I killed 3 before hearing this. Don't repeat it.)
- **Use a `model: fable` subagent for adversarial reviews**, and also **for reviewing code
  and detailed architecture decisions whenever there is doubt**. Fable reasons
  differently from the main model, which is the point — it is the independent
  perspective, not a second opinion from the same mind. Give it the brief that most
  benefits from independence (architecture challenges, cross-cutting consistency),
  and tell it explicitly not to be agreeable.
- **`codex` is running in `pty-4`** (user set this up 2026-07-27) — a genuinely different
  model family, reachable via the `mcp__tttt__tttt_pty_*` tools. Use it when a second
  *independent* opinion is worth more than another Claude pass: contested architecture
  calls, a subagent finding that looks wrong, or anything where agreement between two
  Claude instances would prove little. Not a default reviewer — a tiebreaker.

## 9. Next actions

> ### ⏭️ START HERE (wave 348) — 1529 tests, 4 ignored, M1 165/165 by contract
>
> **Sema 142 of 142, `chiero-pp` 27 of 27, `chiero-parse` clean.**
>
> **Next, in descending order of what they buy:**
>   1. **Finish the audit** — roughly forty `self.error(` sites in sema still unread. Six waves,
>      six kinds of finding. Still unread and category-shaped: the **`switch`/`case` family**
>      beyond wave 319 (a `case` in a block nested inside the switch, a `default` among ranges,
>      duplicate detection across `case 1 ... 3`), and the **conversion family**'s remaining half
>      (`_Bool` against every scalar, vectors, `void *` against function pointers).
>   2. **A predicate with a documented exclusion is a checklist too.** `is_incomplete` excludes
>      `void` on purpose, and wave 347 found two callers that needed the exclusion reversed. The
>      other predicates carrying a stated exception are `not_an_lvalue` (wave 329, excludes the
>      `Cast`-of-`InitList` case), `reads_an_object` (excludes `&`), and `assignable`'s `_Bool`
>      arm. **Enumerate each predicate's callers and ask whether the exception is right for each.**
>   3. **Census C 6.8's statement constraints beyond wave 312's**, and 6.9's external definitions.
>   4. **`FloatKind` cannot tell `_Float32` from `float`** (wave 336); **qualifiers reach sema but
>      not `chiero-lower`** (wave 328).
>
> **Run a category against two members of it, not one.** Seventeen contexts asked only of
> `struct I` would have confirmed everything; asking the same seventeen of `void` found three
> defects, because `void` is the incomplete type the shared predicate deliberately omits. **When a
> predicate has a documented exception, the exception is the second member to test with.**
>
> **The census method itself is the asset**, and it has now paid four waves running. Its two
> non-obvious rules, both learned the hard way: run the **legal** half (it found three false
> positives that no other test could see), and take the verdict from **gcc**, not from judgement. — assigning to a `const`, a parameter of
> incomplete type, a variable declared `void`, and using a `void`-valued call. They are the
> remaining rows that are about *types* rather than about statements, so they sit in the same part
> of sema as wave 308's three and should cost less than rows 8–16, which need statement-level
> context (loop nesting, switch tables, label sets) that sema does not currently carry.
>
> **Before that, consider the cheaper thing wave 307 recorded:** there is still no test anywhere
> that asserts ordinary correct code produces *no* sema diagnostics. Both of the last two waves'
> false positives would have been caught on day one by one such test over the existing corpus.
>
> *The working tree is clean, every wave is committed, and all gates pass: `cargo fmt`,
> clippy, `check-deps`, `check-vpp-leak`, `check-proof-surface`. Wave 132 closed the sret
> wild pointer plus eight more defects and emptied `tests/corpus/owed/`; 133 found pointer
> null-testing did not work at all; 134 discharged the parser's speculative type-name
> rollback; 135 pinned every operator's precedence class after a sweep found `<<` could be
> moved without any test noticing; 136 fixed the bit-field read-modify-write; 137 made an
> enumeration constant its value, at its own type and in its own scope; 138 made a compound
> literal an object and let it take postfix suffixes; **139 built the generator, and it
> found a defect on its first run; 140 gave it structs and helpers and it found two more; 141 gave it
> arrays, pointers and six spellings of one access, and it found another; 142 gave it
> bit-fields and unions and it found a *wrong answer*; 143 gave it file-scope declarations
> and it found a global pointer reading as null; 144 closed the last defect on the open
> list; 145 made lowering refuse CIR the verifier rejects;
> 146 gave the generator a shrinker;
> 147 gave the refusal ledger teeth and declared floating point unimplemented;
> 148 paid the first debt the ledger recorded; 149 audited the owed list
> and found three entries stale; 150 gave prefixed string literals their element width;
> 151 replaced the second string-literal decoder with one shared one;
> 152 deleted the third; **153 built the symbolic differential oracle, and it found that
> the solver could decide only one side of every branch; 154 gave it the other half of three
> narrowings, plus `<=` and widened operands; 155 gave it a bounded candidate search and
> replaced a linear scan with an index; 156 made a symbolic divisor's zero-ness a question
> the engine asks; 157 shipped the checker that turns a UB event into a finding; 158 made the
> witness beside it one that actually faults; 159 gave each finding its own; 160 stopped
> labelling a proven path undecided; 161 turned tier 2 on by default, as 022 §4 always
> said; 162 made the result say which solver decided it; 163 gave the backend a watchdog;
> 164 made every query a dumpable artifact; 165 told the solver its own budget, closing
> 022 §4 entirely; 166 audited 021 and found nothing, and measured why the generator is
> half idle; 167 gave the engine concrete floating point; **168 made floats lower, run and
> agree with gcc; 169 finished them with comparisons and `_Bool`; 170 fixed mixed int/float
> operands; 171 closed a hole in the generator's UB filter; 172 made a float-cast overflow a
> finding; 173 censused the UB gap and found where it cannot be closed;
> 174 gave CIR the signedness C's UB rules turn on, closing a false report and two
> missing ones; 175 found a widening conversion was hiding constants from the whole
> engine, and three census rows closed at once; 176 gave float-to-integer conversions
> the destination's signedness and brought every census row to parity with UBSan;
> 177 made the generator commit memory UB and graded chiero against AddressSanitizer;
> 178 gave it a heap, and the oracle five fault classes instead of one; 179 completed
> ASan's six and found one class starving the rest**.*
>
> ### 🧭 Decided this session — do these before more one-defect waves
>
> The user observed that defects are being found one wave at a time and that this is now the
> bottleneck. A Fable meta-review agreed the *cause* but corrected the framing: the defects
> came from four channels (human fixtures, adversarial review, probing around a smaller
> reported bug, mutation), and what they share is that **each is bounded by what a human
> thought to spell**. The rules-earned list below is mostly cross-product failures — one arm
> guarded and not its sibling, every fixture putting the pointer on the left. The fix is a
> channel where constructs are enumerated **once, in one auditable grammar**, and the
> spellings × contexts × operand orders are explored mechanically.
>
> **1. Coverage recon — DONE in wave 138, and it worked.** It did not need
> `cargo-llvm-cov` in the end. Enumerating `ExprKind`'s 21 variants against the arms of
> `raw_expr` took about ten minutes and found **three** reaching the catch-all `_ => Undef`:
> `Error`, `TypeName` and `InitList`. Two behave correctly —
> `__builtin_types_compatible_p(int, int)` and `__builtin_choose_expr` push "contains a
> construct lowering cannot represent" and 015 §7 discards the function, a gap behaving
> loudly. `InitList` was the silent one and became wave 138.
>
> ~~**Do the same census for `StmtKind`, `Ty`, and `RValue`/`InstKind`.**~~ **Done in wave
> 153, and it came back clean.** All 17 `RValue`, 14 `InstKind` and 6 `Terminator` variants
> have engine arms; `cty` covers all 9 `Ty`; `stmt` reaches its catch-all only for
> `GotoIndirect` and `Asm`, and that catch-all is *loud* — a diagnostic, so 015 §7 discards
> the function. `StmtKind::Error` is silently dropped, but all four sites that build one
> push a parse diagnostic first. **The census channel is exhausted**: wave 138's `InitList`
> was the one silent gap it had to give, and a second pass found nothing. What it cannot see
> is the shape wave 153 then found by other means — a *right* arm that is wrong for an
> operand shape it was never given.
>
> **1b. Coverage recon, the tooling version (not yet run).** `cargo install cargo-llvm-cov`, run over the
> existing suite, read region coverage for `expr`, `assign`, `lvalue_addr`, `truth_of` and
> every `_ =>` fallback. Finds the "arm that returns `Undef`/32/`None` and never executed"
> class — the enum defect's shape. Structurally blind to "right arm, wrong for this operand
> type", because `width_of` answering 32 for a pointer executes the same region a passing
> `int` test does. Reconnaissance, not a channel.
>
> **2. A generative differential harness — v1 EXISTS**, in
> `crates/chiero-lower/tests/generated.rs`. First run: 200 programs, 163 compared, 32
> discarded as undefined, 0 refused, **5 mismatches — all one defect**, `_Bool b = 1; b++`
> giving 0 where C says 1. No hand-written fixture had it: `differential.rs` tests `b += 1`
> from 0 and `b -= 1` from 1, both of which fit. The boundary was the case nobody spelled.
>
> **What v1 has**: scalars including `_Bool`, the full binary operator set unparenthesised,
> casts, `?:`, compound assignment, `++`/`--`, `if`/`else`, bounded loops, an adversarial
> constant pool, a checksum return over every live variable, the five-way verdict, the
> sanitizer discard filter with gcc-O2/clang cross-checks, and fixed seeds.
>
> **What v1 still lacks, in the order the defect record says to add it:**
>   - ~~Helper functions with struct parameters and struct returns.~~ **Done in wave 140**,
>     with `struct` definitions, by-value struct parameters, struct returns, calls in the
>     expression grammar, and a checksum that reads every field. It found two defects on the
>     first run: `make(7).a` and a braced element narrower than `int`.
>   - ~~Pointers, arrays and the alternative-spelling production.~~ **Done in wave 141**:
>     fixed arrays with braced initializers, six spellings of one element access, pointers
>     walked with `p += 1`/`p++`, writes through every spelling, and a checksum over every
>     element. It found the `*(&a[i] + 0)` defect immediately.
>   - ~~Structs, bit-fields and unions.~~ **Done in waves 140 and 142.** A union writes and
>     reads only its first member — reading one not last stored is unspecified, and an
>     unspecified program teaches nothing. The generator also **runs the verifier** now and
>     reports `InvalidCir` separately from `SilentNoState`, since wave 141 showed the
>     engine's silence can be a consequence rather than the fault.
>   - ~~File-scope declarations.~~ **Done in wave 143**: global scalars, arrays, and a
>     pointer aimed at one, read and written and checksummed. It found `int *gp = &g;`
>     making `gp == 0` answer 1.
>   - **An AST shrinker** (~250 lines) emitting `agree_with("…", "…")` directly. Wave 139
>     shrank by hand in about five minutes; that will not scale.
>   - **A refusal allowlist ratchet.** The ledger is printed and currently empty, so nothing
>     forces a decision yet — the moment it is non-empty, an unlisted code should fail CI or
>     it becomes a suppression file.
>   - `xtask diff-soak --seed N` for open-ended search; CI keeps the fixed batch.
>
> **Original plan, for reference (2–4 days).** Typed AST → C, both sides of the
> existing oracle. Fable's estimate: it would have found **11–12 of the last 15 defects**.
> Design points that matter, each earned from a specific defect:
>   - **Multi-function generation is v1, not v2** — the struct-parameter defect is invisible
>     to any body-only generator.
>   - **Alternative spellings are a production**: `a[i]`, `*(a+i)`, `*(i+a)`, `p += i`, `p++`
>     for one access. Every human fixture used `a[i]`, the only working spelling.
>   - **Programs return a checksum over every live scalar and field**, which buys
>     neighbour-corruption detection (the bit-field case) for free.
>   - **Adversarial constant pool** — ±2^31, ±2^32, 2^63 — or the wide-enum class is missed.
>   - **Five-way verdict, not pass/fail**: `Agree`, `Mismatch`, `ChieroPanic`,
>     `SilentNoState`, `Refused{stage, diagnostic}`. Critically, `tests/harness/lower`
>     **panics on any diagnostic**, so a clean-lowering `None` is *always* a defect — that is
>     015 §7's "a gap is a diagnostic, not a licence" made mechanical.
>   - **`Refused` needs an allowlist ratchet**, or the ledger becomes a suppression file: a
>     refusal whose diagnostic code is not on the list fails CI.
>   - **UB by sanitizer discard, not by construction** — `-fsanitize=undefined,address`,
>     plus discard when gcc -O0 / gcc -O2 / clang -O0 disagree with each other. Building
>     provably-UB-free generation is the csmith trap.
>   - **Shrink over the AST, not the text** (~250 lines); emit the shrunk case as literal
>     `agree_with("…", "…")` so the random channel *feeds* `differential.rs` rather than
>     competing with it.
>   - Fixed-seed batch in CI; open-ended soak as `xtask diff-soak --seed N`.
>
> **Do not**: csmith, creduce/cvise (absent, and worse than AST shrinking when you own the
> AST), CIR well-formedness properties (all 15 defects produced verifier-clean CIR — that
> dimension does not vary), more corpus goldens *as detection* (six broken features left them
> byte-identical), or promoting `-O2`/clang to the primary verdict.
>
> ~~**Known limit**: this tests closed concrete programs only.~~ **Addressed in wave 153**
> by `crates/chiero-lower/tests/symbolic.rs`, though as a *hand-written* channel rather than
> a generated one. It lowers `int probe(int x)`, runs it symbolically, and for each path asks
> the solver for a model — a concrete `x` reaching that path — then evaluates the path's
> return under it and requires gcc's `probe(x)` to agree.
>
> **The witness must come from the solver.** Picking `x` from a constant pool and running
> concretely is the existing channel in a hat: nothing ever asks the path condition a
> question, so a condition that is too weak or too strong is invisible. Solving inverts it —
> too weak admits an `x` the program would have sent elsewhere and gcc computes the other
> branch's answer; too strong is unsatisfiable and the path vanishes, which a separate count
> of compared paths catches.
>
> **Generation was prototyped in wave 160 and not committed.** Three grammars, ~1300
> compared paths, two symbolic parameters, every arm UB-free by construction (unsigned
> arithmetic wraps, divisors masked away from zero, shift counts masked below the width) so
> gcc stays authoritative. **It found no value mismatches at all** — the engine agrees with
> gcc everywhere that grammar reaches. What it did surface: with a real backend ~30% of the
> paths the engine explores come back `Unsat`, explored because tier 1 cannot refute, and
> asking what a *finding* on such a path looks like is what produced wave 160. A standing
> version is still owed; the scratch harness's shape is in that wave's commit message.
>
> **A harness note worth keeping**: the oracle builds `TieredSolver::new()`, with **no
> backend**. A third of its skips were tier 1 declining, not the engine. Passing
> `SmtLib::discover()` turns most of those into `Unsat` verdicts and makes the skip counts
> mean what they say.
>
> **Still owed here**: *generating* the bodies as a committed test, and a
> witnessed *fault* from a symbolic run — which is also the fixture that would kill wave
> 153's one live surviving mutant. The wave-117 `fork_on_offset` survivor is now
> **reproduced**: `a[x & 3]` reports `Fidelity::Unknown` with "a symbolic pointer offset was
> not enumerated: 1 value(s) found and the search was cut short by the solver". That is a
> *declared* gap, 023 §7 working as specified — and it is what finally exercises the `Gap`
> verdict §9 had recorded as unexercised.
>
> **3. `chiero-cli` / `chiero-tool` — approved by the user this session** ("do as you see
> fit"). Both are genuine stubs (5 lines and 1). It is M7 work at M1 and five of the crates
> 050 §3's catalogue needs are 1-line stubs, so only one vertical can be backed: C source →
> findings, plus CIR dump/verify and span provenance. Build the 050 §2 envelope in
> `chiero-tool` first — every operation returns `fidelity`/`proven`/`blind_spots`, and a
> subcommand-per-function CLI would bypass the one design decision 050 calls the most
> important in the crate. TDD against 050 contracts 1, 2 and 4b. **Ranked after 1 and 2**:
> the user's stated pain is defects slowing progress, and the CLI does not address it.
>
> ### ✅ The intermittent solver failure — diagnosed and fixed in wave 246
>
> One full-suite run failed
> `a_subset_of_a_satisfiable_set_is_satisfiable_with_no_backend_call` (wave 185). It then
> passed **5 times in isolation and twice more under `--workspace`**, so it is not
> reproduced and not diagnosed. Written down because a flake nobody records is a flake
> rediscovered from scratch.
>
> **The hypothesis was right.** Wave 246 reproduced it deterministically — twelve spinners on a
> twelve-core box make exactly two tests fail every run — and both assert `Sat` on a cold query as
> *setup*, so 022 §4's watchdog fires and the assertion about caching never gets its chance. An
> undecided setup now **skips**, as a missing backend already did in that file. The exposure the note
> predicted is real and general: **any test asserting `Sat`/`Unsat` on a cold query is a wall-clock
> dependency**, and the `cold_sat` helper in `crates/chiero-solver/tests/cache.rs` is the shape to
> reuse.
>
> ### ✅ Closed in wave 249 — §9's oldest open item, and the framing was wrong twice
>
> **The fix is a return value.** `report_faults` now hands back the faults that survive discharge,
> and the scalar load consults *those*. Discharging a `maybe` costs up to three solver queries per
> fault; the result was computed, used for reporting, and dropped — so the caller deciding whether
> the *value* was usable had only the raw list. The engine proved the byte was written, correctly
> reported nothing, and threw the value away anyway. **One fault list decided two things and only one
> of them saw the proof.**
>
> Wave 248's inference — that the opaque init guard defeated the discharge — was wrong, and it was
> marked unverified, which is the only reason it cost twenty minutes instead of another six waves.
> The check it asked for printed `neg=Unsat`: the discharge *succeeds*.
>
> Wave 247's framing was also wrong, and is worth keeping as a lesson rather than deleting. It asked
> whether a `maybe` about definedness should discard a value about contents. It should not — but no
> `maybe` survives here to do so, so the question never arose. **Two waves of design thinking on a
> question that a single `eprintln!` dissolved.**
>
> The old text follows, because the method that corrected it is the reusable part.
>
> ### ~~🔴 a `maybe` about definedness discards a value about contents~~ — RESOLVED
>
> **Wave 247 ran the check this section asked for and the answer was no.** What follows the rule is
> the corrected chain; the old hypothesis is kept below it because the *method* that disproved it is
> the reusable part.
>
> Every link is now instrumented rather than argued:
>
> ```text
>   PROMOTE-SET obj=ObjectId(4) data=Term(192) select0_ground=Some(5)   <- the seed holds 5
>   WRITE-3287  before=Term(192) after=Term(204)   <- the symbolic store, over that array
>   READ        obj=ObjectId(4) data=Term(204)     <- the read sees that chain
>   READ-BYTE   obj=ObjectId(4) b=0 data=Term(204) <- and builds select(Term(204), 0)
>   DISCARDED   faults=["MaybeUninitialized"]      <- and the engine throws the value away
> ```
>
> The load gets a **value and a fault**. `r.value.filter(|_| !unusable(&r.faults))` discards the
> value because `yields_unknown_value` lists `MaybeUninitialized`, and the engine then mints a fresh
> unconstrained symbol and degrades to `Unknown`. "Solves to 0, not 5" was never a stale array — it
> was a *free variable* being reported as a value, and the solver picking 0.
>
> **The design question, stated so the next wave does not rediscover it.** A `maybe` about
> *definedness* and a value about *contents* are different claims. The model knows byte 0 holds 5 and
> is separately unsure whether reading it is defined, because the init chain after a symbolic store is
> 512 nodes and `init_guard` gives up past `EXPAND_LIMIT`. Reporting the `maybe` is right; replacing
> the value is what turns a program that has an answer into one that reports another.
>
> **Why it was not fixed in 247:** `unusable` gates every `maybe`-uninitialized read in the engine, so
> changing its meaning needs its own fixtures — including ones that catch it going the *other* way,
> since wave 195's invented `uninitialized-read` is what this conservatism was built against. The
> reproduction is `a_concrete_byte_written_before_promotion_survives_it` in
> `crates/chiero-lower/tests/symbolic_offset_store.rs`, `#[ignore]`d with the chain in its doc comment.
>
> #### Wave 248 read the code around it. Two facts, and one inference marked as one.
>
> **Fact.** `init_guard` (chiero-mem, just above `impl Memory`) expands the init chain with
> `select_expand(arr.init, bi, EXPAND_LIMIT)` where `EXPAND_LIMIT = 256`, and falls back to an opaque
> `a.select(arr.init, bi)` past it. Wave 214 made that fallback deliberate: refusing to expand is not
> refusing to ask, and returning `None` there once made a real finding vanish.
>
> **Fact.** A symbolic store rewrites **every init bit of the whole object** — the loop is
> `for b in 0..size { for bit in b*8..b*8+8 { … } }`, so a 64-byte object gets a **512-store** chain
> from one symbolic store. 512 > 256, so on this shape the limit is always exceeded and the guard is
> always the opaque form. The data marking itself is correct (`ite(hit, one, prev)` preserves what was
> already known); it is the *depth* that is the problem, not the content.
>
> **Inference, not yet checked.** That the opaque guard is what the discharge then fails to settle,
> giving `MaybeUninitialized` for a byte whose init bit is provably 1. It is consistent with
> everything observed, and it is exactly the kind of plausible chain wave 247 spent a wave
> disproving — so **check it before building on it.** The check: log the guard term and the
> discharge's verdict for byte 0 in this fixture.
>
> #### Three directions, in the order they look worth trying
>
>   1. **Make the limit about the work, not the length.** `select_expand` refuses at 256 *stores*
>      before folding anything; for a **concrete** index every comparison decides immediately and no
>      `ite` is ever built, so there is nothing to bound. `select`'s own walk already folds on
>      syntactic index identity. A limit that counted `ite`s *constructed* rather than stores *seen*
>      would let the concrete case through at any depth.
>   2. **Do not rewrite 512 bits to record one conditional write.** The chain's depth is what defeats
>      every downstream fold. Whether a cheaper encoding exists is a design question, not a bug fix.
>   3. **Separate "no value" from "a value I am unsure is defined"** in `unusable` — the wave 247
>      question. Note this is *last*, not first: if 1 or 2 lands, byte 0 stops being a `maybe` at all
>      and the question becomes narrower.
>
> ### ~~🔴 `arr.data` at read time lacks its seeding stores~~ — DISPROVED in wave 247
>
> The open case is unchanged in behaviour and much narrower in cause. Wave 201 instrumented
> instead of reasoning and **eliminated three explanations**:
>
> ```text
>   ca[0] = 5;  ca[(i & 31) + 32] = 7;  return ca[0];   ->  solves to 0, not 5
>
>   SEED obj=ObjectId(3) size=64 byte0=5 sym0=false init0=Yes   <- seeding is correct
>   READ obj=ObjectId(3) off=0 size=1 arr=true repr=Array       <- the read uses the array
> ```
>
> - **Promotion seeds correctly.** `byte0=5`, `init0=Yes`, observed at the seeding loop.
> - **The read goes to the array**, on the right object, with `repr == Array`.
> - **`eval` does walk store chains for `Select`** (chiero-solver:1290ff) — it follows
>   `Store`/`ArrayConst` and compares index models, so a seeded store at index 0 would be found.
>
> Those three together leave exactly one claim: **the `arr.data` the read sees does not contain
> the seeding stores** — its chain bottoms out at a fresh `array_const(_, 8, 0)`. So something
> between promotion and the read replaces `arr.data` with a store over a *fresh* array rather
> than over the seeded one.
>
> **Check it directly**: print the `Term` id of `arr.data` immediately after
> `promote_to_array` returns, and again at the read. If they differ, find the writer in
> between; the candidates are the four `e.arr = Some(arr)` write-backs, one of which may be
> writing back an `arr` captured *before* promotion. That is one `eprintln!`, and it is the
> fourth explanation rather than a fourth guess.
>
> **The design-pass question is now answered: not yet.** It was asked because three causes in
> one interaction suggested a structural problem. Two were ordinary bugs (a bypassing fast path,
> a wrong index space), and the third has narrowed to a single term-identity question. That is
> a bug, not a design flaw — so fix it, and only revisit the design question if the term ids
> turn out to match.
>
> ### 🟢 The checker sweeps are done — four targets, every survivor now killed
>
> | target | mutants | survived | acted on |
> |---|---|---|---|
> | **CIR verifier** (290) | 40 (one way) | 7 | 6 fixtured, 1 deleted as subsumed |
> | **`chiero-check`** (291) | 14 (both ways) | 5 | 3 fixtured, 2 recorded in place |
> | **`chiero-mem`** (292) | 144 (both ways) | 42 → 6 both-ways | 1 fixtured, 3 listed |
> | **`chiero-pp`** (293–294) | 138 of 166 (both ways) | 28 → 6 both-ways | 4 fixtured, 2 cosmetic |
> | **`chiero-pp` tail** (297) | 46 of 60 (split, see below) | 17 | **17 fixtured, 1 defect fixed** |
>
> **The tail was the richest slice of the four**, and the reason is worth keeping: everything past
> line 1890 is the `#if` expression evaluator, which is a *second* implementation of C expressions
> that no differential test ever reaches. The corpus compares chiero against gcc on translation
> units; nothing compares them on directives. So `#if` could compare (tested) but not calculate:
> `divide` survived mutation in **all four arms** — signed and unsigned, quotient and remainder —
> because the only committed test using `/` divided by zero, which short-circuits before `divide`
> is called. `*`, `%` and unary `+` were unfalsifiable outright.
>
> Probing the last two survivors found a **real defect**: `#if '\` at end of file panicked, because
> `parse_char_constant` sliced between the first and last `'` and for an unterminated literal those
> are the same quote. The bound that survived mutation and the crash were the same fact — nothing
> could reach the bound without crashing first, so the untested guard *was* the bug's hiding place.
>
> ~~**Next target: the `#if` evaluator deserves a differential channel of its own.**~~ **Built in
> wave 298, and it found two defects on its first run** — see below.
>
> ### 🟢 The no-diagnostics gate — `semantics.rs::the_corpus_analyses_without_a_single_diagnostic`
>
> Wave 307 recorded that nothing asserted *correct* code produces no sema diagnostics. Wave 309
> built it, in one assertion over machinery that already existed: `corpus_analyses` parses six real
> VPP headers for the layout gate and already required the preprocessor and parser to be clean —
> it simply never looked at sema.
>
> **It caught a false positive on its first run**, and that diagnostic was the *only* thing sema
> said about the entire corpus, which is what made it obviously ours rather than a finding about
> VPP: this is shipped C that gcc compiles in silence. `__func__` was reported undeclared and
> every use of it produced no state at all.
>
> **`__func__` is now the object C99 6.4.2.2 says it is** — `static const char __func__[] = "name";`
> at the top of every function body — implemented as that object rather than as a magic value, so
> `sizeof(__func__)` is the name's length plus one and two functions of the same name share one
> interned string. `__FUNCTION__` is the same object.
>
> **Widened in wave 310, and the premise above was half wrong.** The differential fixtures *are*
> already gated: `harness::lower` asserts sema is clean before lowering, so every `agree_with` case
> and the whole generated corpus already require silence. What was narrow was the corpus itself —
> six of twenty-eight headers.
>
> Asking the corpus rather than guessing gives **twenty** usable seeds, all silent. The eight
> absentees are named in the constant with gcc's own verdict beside them:
>   - `bitops.h`, `vec_bootstrap.h`, and the five `vector_*.h` are **not standalone** — each uses a
>     type an earlier header defines, and gcc rejects them alone exactly as this parser does. They
>     are still analysed through the seeds that include them.
>   - `memcpy.h` calls `clib_memcpy_fast` without including its declaration. `gcc -Wall` warns
>     "implicit declaration of function", so **sema is right and the header is not clean**.
>
> **A gate is only as good as the argument for what it leaves out**, and "gcc says the same thing"
> is the only kind of argument that does not rot. A gate with one permitted diagnostic in it will
> acquire more.
>
> **The layout gate had its own copy of the seed list**, so widening the harness moved nothing. It
> is one constant now, and the ABI gate went from 652 records / 2,909 `_Static_assert`s to
> **1,369 / 5,482**, all accepted by gcc. Second time a duplicated definition was caught only
> because the copy stopped tracking — `size_of_cty` is the other, still open below.
>
> ### 📊 Wave 326 — the ratchet reaches 61 of 63, and stops at a principled boundary
>
> Four more violations rejected: assigning to an array, a duplicate struct member, a duplicate
> parameter name, and taking the address of a `register` object. Each went where sema already
> visits the construct, and each needed one decision about where it stops:
>   - the array rule reads the left operand's **written** type, before decay — `a[0] = b[0]` is an
>     element, and a `struct` holding an array assigns whole, which is how one copies an array;
>   - a duplicate member is checked **within the record**, a duplicate parameter **within the
>     list** — `values` holds the whole file scope by then and would make every parameter
>     shadowing a global a duplicate;
>   - `register` gets its **own** scoped set beside `read_only` and `read_only_pointee`, because
>     one set for three properties gets two of them wrong.
>
> **The two below the line are the two needing machinery that does not exist**, so the queue
> emptied to a principled boundary rather than to where effort ran out:
>   - **discarding `const`** — qualified types, 436 `Ty::` sites;
>   - **a `goto` into a VLA's scope** — per-label knowledge of whether a variably-modified
>     declaration precedes it. Jumping into a block declaring a *non-VLA* is legal, so nothing
>     cheaper than that distinction is correct, and an approximation would reject legal code.
>
> ### 📊 Wave 325 — a ratchet on how much of C's constraint surface sema rejects
>
> `generated_rejection.rs` is the mirror of wave 324's channel: of the programs gcc *rejects*, how
> many does sema reject? It **cannot** be a pass/fail gate — plenty of C's constraints are
> genuinely unchecked — so it is a ratchet, and the failure names the misses rather than printing
> a percentage. A count gives the next wave a number; a list gives it a queue.
>
> **It measured 54 of 63, and the wave closed three, so the floor is 57.** Raising `FLOOR` is the
> deliberate act a wave performs when it adds a rule.
>
> **The six still below the line — this is the queue:**
>   - `discard const` — needs qualified types (436 `Ty::` sites; see the front list)
>   - assignment to an array
>   - a duplicate struct member
>   - a duplicate parameter name
>   - a `goto` into a VLA's scope
>   - taking the address of a `register` object
>
> **gcc validates the list, not the other way round.** A program the list believes illegal and gcc
> accepts is asserted on separately, so it can never be mistaken for a missing check — and that
> assertion fired on the first run: "modifying a string literal" is runtime UB, not a constraint
> violation.
>
> **A deliberate divergence was settled here.** `return v();` from a `void` function is a C 6.8.6.4
> constraint violation that gcc accepts *by default*. Wave 311 had put it in the accepted list
> having checked the default; wave 314 established that this project calibrates to
> `-pedantic-errors`. The rule now rejects it, and chiero therefore rejects a program plain `gcc`
> compiles. That is the calibration working, not an oversight.
>
> ### 🟢 Wave 324 — a generated channel for sema's diagnostics
>
> Wave 323's measurement said every diagnostic-side rule sits outside all the differential
> channels, because they compare *answers on programs gcc accepts*. `generated_silence.rs` closes
> that: **a program gcc accepts produces no sema diagnostics**, over generated programs rather
> than the twenty-header corpus.
>
> Three decisions, each of which a second attempt would get wrong:
>   - **gcc arbitrates what counts as generated.** A shape the generator thinks legal and gcc
>     rejects is a bug in the *generator*, so it is skipped and counted — otherwise every gap in
>     one's own C arrives looking like an engine finding. `-pedantic-errors`, per wave 314.
>   - **Types and values are generated in pairs.** Picking them independently spends most of the
>     output on programs gcc rejects, and a channel that skips most of what it makes measures
>     itself.
>   - **The shapes are aimed.** Twelve hundred unadventurous programs found nothing; the aimed set
>     is drawn from rejections this project actually shipped (307, 309, 311, 313, 315, 316, 321,
>     322), because rules are dense there and that is where a legal program gets caught by mistake.
>
> **No defect — and the channel was measured rather than trusted.** Five historical false positives
> re-injected; all five make it fail. One needed the generator fixed first: the shadowing shape
> *read* the inner variable where the rule guards *writes*, so it passed with the bug restored.
>
> **Tuning:** `CHIERO_SILENCE_COUNT` (default 300) and `CHIERO_SILENCE_SEED` (default 0).
>
> ### 🟡 Wave 323 — the tier method's yield fell to zero
>
> Thirty-eight more shapes on the same two rules; **none failed.** Waves 321 and 322 each found a
> defect in their first twenty, so the seam is now thin, and the next wave should know that before
> spending itself here.
>
> Both tiers are committed as `scenery_and_second_visit_shapes_agree_with_gcc`, kept for what they
> will catch when a representation moves under them. **The net was measured, not assumed:**
> re-injecting wave 321's length-zero array and wave 322's frame-slot `static` locals both make it
> fail. It cannot see two other recent defects, for reasons recorded at the site — wave 322's
> name-restore rule needs a shadowed file-scope object read afterwards, and wave 320's deref rule
> is a *diagnostic* on a program gcc rejects, which no value-comparing net can reach.
>
> **What that last point generalises to:** the differential channels compare *answers on programs
> gcc accepts*. Every diagnostic-side rule in sema — sixteen census rows, four initializer rules,
> the conversion rules — is outside all of them, and is held only by its own fixture plus the
> no-diagnostics corpus gate. That asymmetry is worth remembering when judging what the suite
> covers.
>
> ### 🔑 Wave 322 — the same method, three more defects
>
> Tier-3 shapes, chosen by wave 321's rule: constructs that appear in fixtures as *setup* rather
> than as subject. Twenty programs, one failure, and chasing it found three defects.
>
> **A `static` local had automatic storage.** C 6.2.4p3 gives it one object for the whole program,
> initialized once; this engine reinitialized it on every entry, so a counter in a loop stayed at
> 1 and one with no initializer produced no answer. **`static int c = 0;` at the top of a
> once-called function was already in the canonical net and passed** — a variable initialized once
> and read once behaves the same under either storage, and only *re-entry* separates them.
>
> **A block-scope `extern` allocated a fresh local**, so `{ extern int v; return v; }` returned
> nothing. Pre-existing; found because it reaches the same branch.
>
> **A `static` local shadowing a file-scope name was never created.** `declare_global` returns
> early when the name is already bound — the guard against a header's `extern int x;` and a later
> `int x;` becoming two objects — so the object was skipped and the name resolved outward.
>
> **The object is module-wide; the name is not.** That split is the design: the binding stays in
> the file-scope table for the body, so reads, writes and addresses all resolve it with no new
> case, and what it displaced is restored at *both* function exits — including the one lowering
> takes when it refuses a function, since the statics are declared by then.
>
> **Both name defects were invisible to the fixture as written**, because every case used a name
> appearing once in the program. Two objects with one name is the subject of those rules, and a
> fixture that names things distinctly — as fixtures do, for readability — cannot reach it.
>
> ### 🔑 Wave 321 — the channels' blind spot, found by asking what they cannot see
>
> §9 recorded a judgement rather than a task: five waves in sema, the differential channels not
> widened since wave 306. Acting on it found a defect in the first twenty programs.
>
> **`int a[] = {1,2,3}` was an array of length zero.** Sema turned an unspecified length into
> `ArrayLen::Flexible` and never completed it from the initializer (C 6.7.9p22), so `sizeof`
> returned 0 where gcc says 12, and reading any element degraded the run to `Unknown`.
> `static char buf[] = "…"` is in most C files.
>
> **Why it survived eighteen canonical programs and a hundred corpus fixtures:** every fixture
> that needs an array writes its length, because whoever writes the fixture picks the length to
> make the arithmetic obvious. It only appeared here because it was used in a *helper* line — the
> source of a copy loop — rather than as the thing under test. **A construct used incidentally is
> tested by nobody**; that is a better search than another list of constructs.
>
> The fix rebuilds the type at the *declaration*, since `ty_of` sees a declarator and no
> initializer, and re-interns rather than mutating because every consumer relies on types being
> immutable. Length comes from the **cursor**, so `int a[] = {[4] = 7}` is five.
>
> **Two other probe results were correct behaviour, not defects**, and are worth knowing: a
> 16-iteration loop reports fidelity `Bounded` — the engine declaring its limit exactly as 023 §7
> requires — and it looked like a failure only because `chiero_answer` reads return values and
> never fidelity. **A probe that ignores the fidelity field will read every declared limit as a
> defect.**
>
> ### 🟢 Wave 320 — dereferencing an incomplete pointee
>
> Closes the case wave 319 wrote down. Nothing checked a `Deref` for completeness, so `*p` on an
> opaque pointer was accepted everywhere — including `*p = *q`, which copies an object of unknown
> size.
>
> **Checked on the pointee, not the pointer.** Copying, comparing and converting an opaque handle
> never touches what it points at, and that is what the type is for. `struct I **p` falls out for
> free: its pointee is a pointer, and a pointer is complete.
>
> **`void *` is untouched, and two rules divide the work.** `(*p);` on a `void *` is legal (GNU),
> while `return *p;` is rejected by wave 311's *void-value* rule. Same expression, different
> faults. This is why `Ty::Void` sits outside `is_incomplete` — folding it in would make the deref
> rule reject a legal program and leave the void-value rule with nothing to say.
>
> **A diagnostic stopped naming the wrong thing.** `p->m` on an incomplete `p` used to report "no
> member named `m`", because the lookup searched a record with no members and failed — true and
> useless. It now reports the dereference.
>
> ### 🟢 Wave 319's `switch` census — and `_Generic` came back clean
>
> The fourth census. Seven rules unchecked: a `switch` controlled by a non-integer, a `case` label
> that is not an integer constant expression, and `case`/`default` outside any switch.
>
> **`_Generic` was censused alongside and was already correct** — a selector matching no
> association without a `default`, and two associations naming one type, are both diagnosed. **The
> first area a census has found already covered**, and the likely reason is that `_Generic` was
> implemented as a unit with its constraints, where `switch` grew its statement handling (wave 312)
> and its type rules separately. That is a useful shape to look for: a construct built in one go
> tends to carry its rules; one that grew in layers tends not to.
>
> Two rules cost two lines each because wave 312 had already built the stack of open switches — a
> `case` outside a switch is `break` outside a loop, asked of a different stack.
>
> **The controlling-type test is on the type's *category***: `char`, `unsigned` and `long` are all
> legal, so writing it against `int` rejects all three. Reading the operand's own type rather than
> the promoted one is **measured equivalent** — promotion widens narrow integers and does not turn
> a `double` into one — and is labelled as such at the site.
>
> **Recorded as not covered:** `switch(*p)` where `p` points at an incomplete struct. gcc rejects
> it; the fault is the *dereference* of an incomplete type rather than the switch, and **nothing
> checks a `Deref` for completeness today** — that is the natural home for it.
>
> ### 🟢 Wave 317 closed wave 314's two declared misses
>
> **Brace elision: count scalars, not items.** Distributing a flat list across sub-objects is the
> hard part and is not needed to answer "is there one too many" — total scalar capacity against
> list length does it. `scalar_capacity` returns `None` where the answer is not fixed (unsized
> array, incomplete record) so nothing is counted rather than guessed. Only a **fully flat** list
> is counted: `{{1,2},3,4}` is legal and a scalar count cannot see where the braced item stops.
>
> **The constant rule, asked the other way round.** "Is this a constant expression" needs a
> complete account of address constants and fails by omission — that is how wave 314 ended at
> "contains a call". "Does this **read the value of a non-`const` object**" has one answer. Four
> things that look like reads are not: an array or function name (an address), an enumerator, a
> `const` object (**gcc folds it**, even under `-pedantic-errors`), and an object of incomplete
> type (contract 20 — its declaration was reported already).
>
> **Two arms are measured unreachable and labelled at the site**, each with its own reason:
>   - the **unsized-array `None`** — an array written `[]` *with* an initializer has its length
>     inferred before this runs, so the target is always `Fixed`; the arm exists for shapes reached
>     by recursion, and a flexible array member is caught by the record rule first;
>   - the **`AddrOf` short-circuit** in `reads_an_object` — every address constant that gets that
>     far has already been answered by `addr_of`, `(long)&y` included.
>
> **Wave 317 listed a third and was wrong.** The union branch of `scalar_capacity` is falsifiable;
> the case written for it never reached the file, because an edit script raised on a later anchor
> and wrote nothing. `union U u[2] = {1,2,3};` is the shape that reaches the capacity rule rather
> than the array *range* rule, and with it present the mutant dies.
>
> ### 🟡 Pointee `const` — half done in wave 316, half still blocked
>
> **Done: writing through a pointer to `const`.** `*p = 1`, `p[i] = 1` and `p->m = 1` on a
> `const T *` are writes to a read-only object — undefined behaviour, not a matter of taste — and
> are now rejected. `read_only_pointee` is a **second** set beside wave 311's `read_only`, because
> C separates the two and one set gets one of them wrong either way: `int *const p` is in the
> first and not the second, `const int *p` the reverse. Both are in the fixture on opposite sides.
>
> The rule looks **exactly one level down**: `const int **p; *p = 0;` is legal, because `*p` is a
> perfectly writable `const int *`. And it is **syntactic** — it asks about the declared type of a
> *named* pointer, so a pointee reached through an intermediate expression is not covered.
>
> **Still blocked: `int *p = cp;` discarding the qualifier**, the last conversion-census row. It
> needs *qualified types*, and the measurement that decided wave 316's scope is worth keeping:
> **`Ty` has 436 match sites across four crates** (sema, lower, cir, exec). A `Ty::Const` wrapper
> would turn every `matches!(.., Ty::Int)` beneath it into a silently-wrong branch — the exact
> failure class waves 304 and 308 were spent on. Anyone attempting it should plan to audit those
> sites, not to add a variant and see what breaks.
>
> ### 🟢 Wave 315's conversion census — pointers and integers were interchangeable
>
> The third census. Sema converted operands but never asked whether the conversion is one C
> permits, so a pointer could be assigned to an `int`, passed as a `char *`, returned from an
> `int` function or compared with an unrelated pointer. **All nine violations are gcc *warnings*
> by default** — which is why wave 307's census tried two of them, read "gcc:ok" and moved on.
>
> `assignable` is consulted in **one** place, `coerce`, because assignment, argument passing,
> `return` and initialization all arrive there and C states one constraint for all four
> (6.5.16.1). Pointer comparison is the fifth site and does not pass through it — pointer operands
> keep their own types — so it asks the same question directly.
>
> **Only pointer mixing is judged.** Arithmetic conversions are unrestricted in C, so a rule based
> on type identity rejects `long`→`int` and every other narrowing.
>
> **Both false positives came from the corpus again, and both were about spelling:**
>   - **`_Bool` takes any scalar** (C 6.3.1.2) — `_Bool b = p;` is a test against zero.
>   - **A parameter declared `int a[2][3]` keeps its array type** while the argument passed to it
>     has decayed to a pointer, so one legal call arrives with its two sides spelled differently.
>     Compare *pointees* through a normalising accessor, not `Ptr` against `Array`.
>
> **Out of scope, deliberately:** `const int *cp; int *p = cp;` — the ninth violation gcc reports.
> Sema has no pointee qualifiers; wave 311 put `const` on *objects* only. That is a type-system
> change, not a check, and a rule written without it fires on the wrong thing.
>
> ### 🟢 Wave 314's initializer census — seven checks, and two false positives the suite found
>
> The second census in wave 307's shape, aimed where sema had never been graded: `InitList` typed
> to `Ty::Error` and was never compared against what it initializes. Twenty-six programs, nineteen
> of them legal.
>
> **Four of the seven violations are gcc *warnings* by default and errors only under
> `-pedantic-errors`** — excess elements for an array, a struct or a scalar, and an over-long
> string. Wave 307's census stopped at exactly this boundary: it tried one, read "gcc:ok", and
> moved on. **Take the verdict at both strictness levels and record which it was.**
>
> Three rules shaped the walk, each forced by a legal case:
>   - **Positions, not counts.** `{[0]=1,[2]=3}` is two items with a highest index of 2.
>   - **Brace elision defeats counting, so counting stops.** `int a[2][2] = {1,2,3,4}` is legal;
>     detecting elision is easy, distributing correctly is not, so the walk declines to answer.
>     `int a[2][2] = {1,2,3,4,5}` is a **declared miss**.
>   - **`char s[3] = "abc"` is legal** — the terminator is dropped when it is the only thing that
>     does not fit.
>
> **The two false positives came from the suite, not the census**, and that is the finding:
>   - a **vector** initialises elementwise and fell to the scalar arm, failing the whole vector
>     corpus at once;
>   - **"not constant" is not "we could not fold it"** — a function designator in a table of
>     function pointers is a constant expression that neither `eval` nor `addr_of` answers for, so
>     the complaint is narrowed to initializers containing a **call**. `int x; int g = x;` is the
>     declared miss.
>
> A census asks what gcc *rejects*. Neither of those is something gcc rejects, so no census could
> have contained them — only a corpus of real code that the checks then broke.
>
> ### 🔴 Wave 307's constraint census — one false positive fixed, sixteen gaps recorded
>
> Thirty C programs, half legal and half not, run through sema and through gcc. **The false
> positive was the find**, and only running the *legal* half could have seen it (wave 303's rule):
> contract 14's redefinition check used a whole-TU symbol set with no notion of scope, so two
> functions each saying `int a = 0;` were a redefinition. So were two `for (int i = 0; ...)` loops
> in one function, and a local shadowing a file-scope name. Fixed.
>
> **Why it survived everything:** a sema diagnostic does not stop lowering. The corpus compiled
> and ran these programs, got the right answers, and never looked at the complaint. Every test
> that checks an *answer* was green. **There is no test anywhere that reads sema's diagnostics on
> ordinary correct code** — that absence is the reusable finding, larger than the bug.
>
> **Sixteen constraints gcc rejects and this engine accepts silently**, none fixed, in rough
> descending order of how wrong the resulting execution is:
>
> | | constraint (C11) | example |
> |---|---|---|
> | ~~1~~ | ~~member that does not exist~~ | **fixed, wave 308** |
> | ~~2~~ | ~~subscripting a non-array~~ | **fixed, wave 308** — `int x = 5; x[0]` returned 5 |
> | ~~3~~ | ~~calling a non-function~~ | **fixed, wave 308** |
> | ~~4~~ | ~~assigning to a `const` object~~ | **fixed, wave 311** |
> | ~~5~~ | ~~parameter of incomplete type~~ | **fixed, wave 311** — definitions only |
> | ~~6~~ | ~~variable declared `void`~~ | **fixed, wave 311** |
> | ~~7~~ | ~~using a `void`-valued call~~ | **fixed, wave 311** |
> | ~~8~~ | ~~duplicate `case` value~~ | **fixed, wave 312** — folded values; ranges excluded |
> | ~~9~~ | ~~multiple `default` labels~~ | **fixed, wave 312** |
> | ~~10~~ | ~~`break` outside loop or switch~~ | **fixed, wave 312** |
> | ~~11~~ | ~~`continue` outside a loop~~ | **fixed, wave 312** |
> | ~~12~~ | ~~label used but never defined~~ | **fixed, wave 312** |
> | ~~13~~ | ~~function redefinition~~ | **fixed, wave 313** |
> | ~~14~~ | ~~conflicting declaration types~~ | **fixed, wave 313** — see the limit below |
> | ~~15~~ | ~~`static` after non-`static`~~ | **fixed, wave 313** |
> | ~~16~~ | ~~function returning an array~~ | **fixed, wave 313** |
>
> **✅ The census is exhausted — all sixteen rows closed, waves 308 and 311–313.**
>
> **Rows 13–16 closed in wave 313**, and two models were wrong on the first attempt, both about
> the *legal* cases:
>   - **`extern` adopts linkage, it does not defer.** The natural first model — "an `extern`
>     declaration never conflicts" — accepts both orderings. C 6.2.2p4 makes it take the prior
>     declaration's linkage, which is why `static int n; extern int n;` is legal and
>     `extern int n; static int n;` is not.
>   - **The parser cannot tell `f()` from `f(void)`**: `parameter_list` returns the same empty
>     vector for both. So parameter lists are compared only when *both* are non-empty, and return
>     types always. **Declared limit:** `int f(void); int f(int);` is a conflict this misses. It is
>     the right way round — rejecting a correct program is worse — and the alternative rejects
>     every K&R declaration in the corpus.
>
> A separate old-style guard was written, then **deleted as subsumed**: mutation could not falsify
> it because the empty-list rule already covers the K&R declaration, and a K&R definition has its
> parameter types filled in and is legitimately comparable.
>
> Rows 1–3 are the ones that matter most for this engine: they are type errors that let execution
> proceed on a value computed from nothing. The rest are diagnostics a compiler owes its user.
>
> **Rows 8–12 closed in wave 312**, and §9's prediction held: the checks are three lines each and
> the *context* was the work. Sema now carries, per function body, two depth counters and a stack:
>   - **`loop_depth` and `breakable_depth` separately.** `break` and `continue` disagree about what
>     a `switch` is — `break` leaves it, `continue` looks past it to the enclosing loop. One
>     counter accepts `continue` in a switch no loop encloses, and the distinction cannot be
>     recovered afterwards.
>   - **A stack of switch frames**, not a set per function: a nested switch starts a fresh case set
>     and a sibling may legally repeat every value of the one before it.
>   - **Label sets checked after the body is walked**, because a forward `goto` names a label
>     declared later. The *restore* is what scopes them per function.
>
> **Declared limit:** a `case` range (`case 1 ... 3`, a GNU extension) is skipped rather than
> approximated by its lower bound. `case 1 ... 3:` beside `case 2:` is a duplicate gcc rejects and
> this does not; comparing lower bounds would catch that one and miss `case 5 ... 7:` beside
> `case 6:`, trading a missed report for a wrong one.
>
> **Rows 4–7 closed in wave 311.** The `const` rule needed machinery sema did not have — it had
> never read the AST's `Quals` at all — and the shape of it is worth keeping: a scoped set of
> objects declared `const` at their **outermost** level. `const int k` and `int *const p` both make
> the *name* read-only; `const int *p` makes the *pointee* read-only and leaves `p` assignable.
> The check fires only when the assignment target is the name itself, because `*p = 1` depends on
> the pointee's qualifiers, which sema still does not model — it says nothing rather than guessing.
>
> Placement mattered more than the rules did:
>   - `void` objects are rejected in `check_complete`, **not** by adding `void` to `is_incomplete`
>     — that predicate's other callers must keep allowing `void *p` and `sizeof(void)`.
>   - A void *value* is rejected in `coerce`, which is exactly the set of places C wants a value,
>     so `v();` and `(void)v();` need no exemption. It tests the **target** type too, because
>     `return v();` from a void function is legal.
>   - An incomplete *parameter* is checked where a body is walked, since a prototype may name one.
>
> **Nothing was over-rejected: all twelve legal forms passed before the fix as well as after**,
> which is only knowable because the census runs its legal half.
>
> **Rows 1–3 closed in wave 308**, and the fix cannot key on `Ty::Error`: that means *unknown*, not
> *wrong*, and an undeclared callee types as `Error` — `__builtin_isnan` and the rest of 7.12.14
> are undeclared, since gcc knows them intrinsically. Each check asks whether the type is
> *concretely known* to be unusable, and stays quiet on anything incomplete.
>
> **The wave's two best finds came from the fixture's accepted list, not from the census.**
>   - `0[p]` was written only to stop the subscript check being too broad. `a[b]` is `*(a + b)`,
>     so the pointer may be on either side — and once sema typed it correctly it *still* produced
>     no answer, because lowering assumed the base was the aggregate at all three `Index` sites.
>   - `(void)p;` was a throwaway line silencing an unused variable in a probe. `(void)0; return 1;`
>     returned **no state at all**: `cast_kind` has no conversion *to* void, so the module it built
>     was rejected. One of the most ordinary idioms in C, and nothing had ever exercised it.
>
> ### 🔑 The technique that is paying: find the *second* implementation of a rule
>
> Wave 298's `#if` channel worked for a reason worth naming, because it generalises. `#if` is not
> a corner of the preprocessor — it is a **second implementation of C constant expressions**, and
> the oracle never watched it. Wave 300 asked where the *third* one is, and the answer was
> `chiero-sema::const_eval`: array bounds, bit-field widths, enumeration constants and static
> initializers are evaluated at layout time, so nothing they compute passes through the lowering
> the corpus compares. **Aiming the two rules wave 298 had just found at it took one probe.**
>
> | implementation | of what | watched by |
> |---|---|---|
> | `chiero-exec` | C arithmetic at runtime | the corpus, since wave 153 |
> | `chiero-pp`'s `#if` evaluator | C constant expressions | `if_differential.rs`, wave 298 |
> | `chiero-sema::const_eval` | C constant expressions | `differential.rs`, wave 300 |
> | `chiero-solver` terms vs `chiero-exec` bits | the *same* C operation, symbolically | `differential.rs`, wave 306 |

> **Wave 306 took the fourth and it came back clean** — 390 pairs, none wrong, none undecided.
> The channel pins a symbolic input back to a known value and asks the solver to prove the
> expression equals what gcc computed, with **three distinguishable outcomes**: proved, provably
> different, or undecidable-so-both-edges-taken. Keeping the third apart from the first is the
> design: a channel that merged them would report agreement for a case the solver had given up
> on, and would go quiet over time without ever failing. The "nothing is undecided" assertion is
> therefore a *coverage* guard.
>
> **Two probes in this wave reported defects that were not there**, both because the probe could
> not build its own input — the same trap as wave 305's `<stdarg.h>`. First the intrinsics were
> undeclared; then `return_value_bits` was asked for a value that is symbolic by construction, and
> `eval_ground` refuses a non-ground term, so every case read as "chiero says None". **A probe
> against a symbolic engine must return something concrete**: branch on the property and return a
> literal, which is what the committed channel does.
>
> **Where a fifth might be:** `size_of_cty` in `chiero-exec` is a second implementation of type
> layout, independent of sema's `size_of_ty` (note `CTy::Ptr => 8`, hardcoded rather than read
> from the target). It is *indirectly* watched — the corpus compares memory behaviour end to end —
> so it is a weaker candidate than the four above, but it is the next one on the list.
>
> Both new-implementation waves found the **same two defects**: the usual arithmetic conversions
> and the conditional operator's type. That is not coincidence — they are the rules where the
> right *number* comes with the wrong *type*, so every test that compares against a positive
> constant passes. **When a defect is found in one implementation, try it against the others
> before looking for a new one.**
>
> ~~**Still to check by this method:** integer *literal* spelling has three parsers.~~ **Checked in
> wave 301, and it came back clean** — every spelling, suffix and width probed agreed with gcc in
> both the runtime and the constant-expression path, including the cases where the *type* differs
> between spellings of the same value (`-0x80000000` is unsigned and `-2147483648` is not;
> `0xFFFFFFFF + 1` wraps to zero and `4294967295 + 1` does not).
>
> **The probe found something else on the way, which is the more useful lesson.** Six `sizeof`
> cases disagreed, all in constant contexts: `const_eval` had an arm for `SizeofType` and none for
> `SizeofExpr`, so `sizeof(int)` folded and `sizeof(1)` did not. And it was *silent*, because the
> enumeration walk folded initializers with `.unwrap_or(next)` — anything `const_eval` could not
> answer became the implicit next value, the same number the enumerator would have had with no
> initializer written. **A gap that returns the plausible answer is worse than one that returns
> none**, and it is why the missing arm could only ever be found by accident. Both are fixed; the
> fallback is kept (an enumeration that stopped resolving would cascade) but announced.
>
> ~~**Where else does `.unwrap_or` swallow a fold failure?**~~ **Censused in wave 302 — seven
> sites, and the pattern paid immediately.**
>
> | site | fallback | verdict |
> |---|---|---|
> | bit-field width (`lib.rs:1498`) | `0`, then `.max(0)` | **four defects, fixed in wave 302** |
> | array element size in `addr_of` (3529, 3570) | `1` | **real gap, not fixed** — see below |
> | bit-field unit size / align (1499, 1500) | `4` | unreached: a bit-field's type must be integral |
> | member size (1538), elem size (1179) | `0` | unreached by the same argument |
>
> **The bit-field width was the worst possible place for a fallback**, because `0` is not a
> neutral value there — it is C's *legal* unnamed zero-width field, handled on the very next line.
> So a width that could not be folded, and a negative width, produced a valid but different
> declaration: member deleted, next field bumped to a unit boundary, nothing reported.
> `int f : notconst;` laid out byte-for-byte identically to `int f : 0;`. Four constraints from
> C 6.7.2.1p4 were missing altogether — non-constant, negative, wider than the field's type, and
> a *named* zero-width field.
>
> ### 🟢 Wave 303 closed the incomplete-type front — and it was a false *positive* too
>
> Five rules, one of which **removes** a check: `extern struct I x;` with no initializer is valid
> C 6.9.2p3 and this engine rejected it. A rule that rejects correct programs is worse than one
> that misses incorrect ones, so that case is in the *accepted* list of the fixture and is what
> the RED failed on first.
>
> | context | rule | site |
> |---|---|---|
> | `extern` decl, no init | **exempt** — completed in another unit | `check_complete` |
> | `extern` decl *with* init, `static` | not exempt: both are definitions | same |
> | array element | must be complete — no stride, no `arr[1]` | array construction, so `extern struct I arr[];` is caught |
> | `p + n`, `p - q` | must be complete; **comparisons excluded** | binary typing |
> | `sizeof` | checked on the resolved *type*, so `sizeof(*p)` counts | both spellings |
>
> **The `unwrap_or(1)` was standing in for the pointer rule, badly**: one byte is exactly a
> `char`'s stride, so the wrong answer was indistinguishable from a right one in any code using
> byte offsets — wave 302's rule about fallbacks whose value *means* something, again.
>
> **Mutation found the exemption was the weak spot**, which generalises: a check that is added can
> only be wrong about programs it rejects, and the RED enumerates those. An exemption is wrong
> about programs it *accepts*, and nothing enumerates those unless written on purpose.
>
> ### 🟢 Wave 304 gave tags a real representation — and found a linked list never worked
>
> `RecordLayout` gained `complete`, and a reference to a named undefined tag now creates that
> record instead of returning `Ty::Error`. **The load-bearing part is *when*:** `tag()` registers
> the name *before* laying out the members, so a member mentioning the tag being defined resolves
> to the record under construction. That one reordering is what makes
> `struct Node { struct Node *next; }` work — **it never had.** Five differential cases produced
> no answer at all: `a.next->v`, `p->next->v`, `sizeof(*a.next)`, mutually recursive structs, and
> use-then-define. Nothing in 1470-odd tests or the VPP corpus covered a self-referential struct.
>
> **A sixth case was a regression from wave 303**, and is the sharper lesson. `a.next - &b`
> compiled and ran before that wave and stopped after it: the new "arithmetic on a pointer to an
> incomplete type" rule is *correct*, applied to an *incorrect fact*. `a.next` points at a
> complete type; only the representation said otherwise. **A check is exactly as right as what it
> asks** — and adding a check over a broken representation converts a silent wrong answer into a
> loud wrong rejection, which is how a latent modelling bug becomes visible.
>
> Two follow-on rules and their non-obvious parts:
>   - **A member must have a size** (new in 304). This is what makes the early reservation safe:
>     `struct S { struct S s; }` now *finds* `S`, and what stops it is that the record is still
>     marked incomplete while its own members are walked. `struct S { struct S *p; }` stays legal.
>   - **`is_incomplete` is deliberately not "has no size."** A function type and a VLA both have
>     no size and neither is an incomplete object type; that phrasing would reject every function
>     declaration and `sizeof(vla)`.
>   - An **anonymous** undefined tag stays `Ty::Error`: no name means nothing can ever complete it.
>   - The VPP layout gate now skips incomplete records — opaque glibc tags like
>     `struct __locale_data` have `RecordId`s now, and asking gcc for their size is asking it to
>     reject the program.
>
> **Closed in wave 305.** `enum_ty` returns `Ty::Error` for a tag whose enumerators have not been
> seen, so the five checks reach it through `is_incomplete`. Two things fell out:
>   - **The folding path needed its own check.** An array bound never reaches `type_expr` —
>     `ty_of` folds the length by calling `eval` directly — so `int a[sizeof(enum E)]` returned
>     `None` and the length silently became `ArrayLen::Vla`: a *file-scope VLA*, from an
>     expression that is not variable at all. Another fallback whose value means something.
>   - **One cause, one report.** With both checks live, `enum { X = sizeof(struct I) }` said the
>     `sizeof` was incomplete *and* that the enumerator was not constant. The second is suppressed
>     when the fold has already explained itself; wave 301's message survives for the case it was
>     written for.
>
> **Declared limit, deliberately:** an enum tag gets *no* completable placeholder, because an
> enum's complete form is `Ty::Int` and there is no record to fill in later. A reference written
> before the definition stays poisoned. This costs nothing in standard C, where an enum cannot be
> forward-declared at all — it is a GNU extension — but it is the one asymmetry with `tag()`.
>
> ### 🟢 The canonical-use net — `differential.rs::the_canonical_uses_of_c_agree_with_gcc`
>
> Wave 304's rule applied before wave 305's recorded front, and the result is worth knowing:
> **eighteen textbook shapes were swept and seventeen already passed.** Trees, list traversal,
> dispatch tables, a qsort-style callback, aggregates by value and by array, nested structs, union
> punning, bit-field read-modify-write, 2-D arrays, a pointer to an array, string walking,
> recursion and mutual recursion, a static local, varargs. The one failure was the linked list
> wave 304 had just fixed. **The canonical-use gap was one gap, not a class of them** — so the net
> is kept for what it will catch when a representation moves under these shapes again, not as an
> open seam to keep mining.
>
> One trap it nearly set: the varargs case first used `<stdarg.h>`, which this harness cannot
> resolve, and `catch_unwind` turned the preprocessing failure into "chiero says None". **A probe
> that cannot build its own input reports a defect in the engine.** It uses `__builtin_va_list`
> now.
>
> ### ✅ Fixed in wave 303 — was: incomplete types are never rejected
>
> `size_of_ty(...).unwrap_or(1)` in `addr_of` silently scales pointer arithmetic by one byte when
> the element type is incomplete. Two programs gcc rejects and this engine accepts without a word:
>
> ```c
> struct I; extern struct I arr[]; void *q = &arr[1];   // gcc: incomplete element type
> struct I; extern struct I *p;    void *q = p + 1;     // gcc: invalid use of undefined type
> ```
>
> This is **not** a fold-guard fix like the bit-field one. The `unwrap_or(1)` is downstream of a
> missing rule — nothing anywhere asks whether a type is complete where C requires it to be — and
> that rule has a wide blast radius: array declarations, pointer arithmetic, `sizeof`, member
> access, and assignment all need it. It was left out deliberately rather than bolted on at the
> end of a wave, and it is the obvious next front.
>
>
> ### 🟢 The `#if` differential channel — `chiero-pp/tests/if_differential.rs`
>
> The corpus compares chiero against gcc on translation units; this compares them on *directives*.
> It generates `#if` expressions from a bounded grammar and checks them against **both** gcc and
> clang, which must agree with each other before either may judge us.
>
> **It compares values, not branches.** A `#if` yields one bit, and a channel that only checks
> which branch was taken cannot see `7 / 2` give 4. Each expression is emitted as 64 directives —
> `#if ((E) >> b) & 1` for each bit — plus a `#if (E) < 0` sign probe. The bit tests recover the
> exact 64-bit pattern for signed and unsigned alike; the sign probe recovers what no bit pattern
> can show. **Both defects it found were sign-only: every one of the 64 value bits agreed.**
>
> | defect | rule |
> |---|---|
> | `conditional` returned the selected arm verbatim | C 6.5.15: `a ? b : c` takes the usual arithmetic conversions of *both* arms, so the arm not taken still decides signedness |
> | every character constant was typed signed | `u'x'` is `char16_t` and `U'x'` is `char32_t`, both unsigned; `'x'`, `L'x'`, `u8'x'` are signed |
>
> **What it deliberately does not generate**, each omission being a claim about what it can prove:
> division by zero (gcc refuses the program), signed overflow (undefined, so a disagreement would
> be a gap not a defect), the comma operator (not permitted in a constant expression), and
> out-of-range hex escapes like `'\x4142'` — a warning in gcc, a hard **error** in clang. Divisors
> are wrapped `(x | 1)`, odd hence nonzero for every value, so the operator is still exercised on
> arbitrary operands rather than constants.
>
> **Wave 299 gave it macro expansion and `#elif`.** A `#if` operand is expanded before the
> expression parser sees a token, so the evaluator's real input is a sequence nobody wrote. The
> prelude now carries object-like and function-like macros, aliases, macros expanding to operators
> and brackets rather than values, `EMPTY`, and the two that cannot terminate (`SELF`, `PING`/
> `PONG`). Half the probes run through `#elif`, alternating by index so the file stays the same
> size while covering twice the directive surface. **No product defect: 6000 expressions across
> four seed bases agree.** What it found was two rules it reached constantly and could not
> falsify — see the rule below.
>
> **Macro arguments are generated with `defined` disabled**, threaded as a flag rather than
> filtered from the finished string: C 6.10.1 leaves `defined` arising from macro expansion
> undefined, the two oracles differ there, and `defined` can sit arbitrarily deep in an argument.
>
> **Two rules are outside this channel by construction**, and belong to fixtures instead: a
> wrong-arity macro call (gcc rejects the program, so the oracle cannot answer — fixtured at
> `macro_expansion.rs:195`), and dropping the hide set from a replacement list, which makes
> `PING`/`PONG` expand forever and so shows up as a hang rather than a wrong answer.
>
> **Tuning:** `CHIERO_IF_DIFF_COUNT` (default 400) and `CHIERO_IF_DIFF_SEED` (default 0).
>

> ### 🛑 The sweep harness scored hangs as survivors — read this before trusting a survivor list
>
> Wave 297 found `#if`'s `>` and `<=` branches reported as both-ways survivors. They were not.
> Those branches sit inside the `#if` evaluator's `loop`; forcing one true means its operator
> token is never consumed, so the loop spins, `timeout` kills `cargo test`, **no
> `test result: FAILED` line is ever printed**, and a harness that greps for exactly that line
> reads the timeout as a pass. Re-running with the new fixture *removed* proved it: the mutant
> hangs either way, so committed tests were already killing it.
>
> This is wave 290's lesson — *a test that aborts looks like a test that passed* — recurring for
> **hangs** rather than aborts, and it is the more dangerous form, because a hang also costs the
> full timeout and so silently dominates the sweep's runtime. Reading the remaining sites made a
> third mode obvious before it cost anything: the `#if` evaluator's *unary* operators sit in a
> recursive descent, so forcing one true recurses until the process aborts, and an aborted test
> binary prints no result line either. Counting lines catches it — but only against the pristine
> count, because `cargo test -p CRATE` runs several binaries and the survivors still print `ok`.
>
> **The scoring rule is four-way, and every future sweep must use it.** `BASE` is the number of
> `test result:` lines a clean run prints (9 for `chiero-pp`) — measure it, do not assume it:
>
> ```sh
> out=$(timeout 90 cargo test -p CRATE 2>&1); rc=$?
> lines=$(echo "$out" | grep -cE "^test result:")
> if   [ $rc -eq 124 ];                              then v="KILLED(hang)"          # NOT survived
> elif echo "$out" | grep -qE "^test result: FAILED"; then v=KILLED
> elif [ "$lines" -ne "$BASE" ];                      then v="KILLED(crash)"         # NOT survived
> else                                                    v=SURVIVED; fi
> ```
>
> **What this retracts.** Every *unactioned* `chiero-pp` survivor below is now suspect — it may be
> a hang, not a gap. Every *actioned* one stands, because a fixture moved it from SURVIVED to
> killed-by-assertion, and a hang cannot do that. The verifier, `chiero-check` and `chiero-mem`
> sweeps are *probably* unaffected — their mutation sites are guards in straight-line checking
> code, not conditions that consume input inside a loop, so there is nothing for a mutant to spin
> on — but that is an argument, not a measurement, and those runs were not re-scored.
>
> **Still open, and small:**
>   - ~~**`chiero-pp` was swept to 85%**~~ **Closed in wave 297.** The 30 remaining sites were run
>     as 46 mutants, not 60: the 14 whose condition *consumes a token* got only the `false`
>     direction, because forcing such a condition true means the token is never consumed and the
>     parser spins or recurses until it dies — a "kill" that proves nothing (rule below). That
>     split, plus the four-way scoring, took the run from "hours, mostly timeouts" to about twenty
>     minutes. Budget by **hangs**, not rebuilds: a clean `-p chiero-pp` run is 9s, so a 90s cap is
>     a 10× margin. Wave 293's "budget by rebuild cost" note was the wrong diagnosis.
>   - **`chiero-mem` is closed** (waves 295–296). The index-width guard was a duplicate of `fit`
>     and is now a call to it, with `fit`'s narrowing arm fixtured; the havoc-uninitialize
>     promotion guard has a fixture; the fault-propagation early-out is *unreachable* — promotion
>     faults only through `state_fault` and every caller pre-checks it — and is kept deliberately
>     as forwarding rather than deleted as a rule, with the measurement recorded at the site.
>   - Two `chiero-pp` survivors are inside the `__VA_OPT__` refusal path, where how much of the
>     group gets skipped after a declared-limit diagnostic is cosmetic by construction.
>
> **What the sweeps taught, one line each.** Mutate both ways — `true` finds the false-positive
> guards. Sort by both-ways survival — it turns forty survivors into six. Expect survivors to
> cluster on **one missing input shape**: leaf callees in `chiero-check`, in-bounds writes in
> `chiero-mem`, root-level fixtures in `chiero-pp`. Expect at least one fixture to pass for the
> wrong reason first — it happened in every one of the four.
>
> ### 🟢 The corpus work is done — and the focused channel is now the place to add shapes
>
> **Wave 277's rule is discharged, its overflow is housed, and the channel has started paying for
> itself.** All six lagging constructs went through the corpus (284–287); the two the shared
> channel could not afford moved to `program_focused` (288); and wave 289 used it for something
> new — three shapes that were on record as *unreachable*, now graded.
>
> | channel | compares | carries |
> |---|---|---|
> | `program_control_flow` | 100 / 200 | vectors, `typeof`, `_Generic`, `offsetof`, classification builtins, `__label__`, alignment (present, not discriminating) |
> | `program_focused` | **200 / 200** | non-square multi-dimensional arrays; alignment specifiers at a discriminating rate, on scalars and arrays; anonymous members, chained `offsetof` designators, `typeof(<type-name>)` |
>
> The corpus now makes **twenty mutant kills** it could not three weeks ago, across waves 271,
> 275, 276, 278, 279, 280, 281, 282, 283 and 284.
>
> **Where the next construct goes.** If it needs an object, a statement, or values flowing through
> arithmetic — `program_focused`, where it is a `match` arm. If it is an *expression wrapper*
> (285) or a pure naming construct (286) — the shared channel still absorbs it for roughly
> nothing. **And if a mutant survives, ask whether it is a missing shape before calling it
> unreachable**: wave 289 is three counter-examples to that call.
>
> **No survivor is currently recorded as shape-limited.** The next mutation sweep that produces
> one has a channel waiting for it.
>
> ### 🟢 Every named census axis is now run
>
> **Wave 283 ran the last one, `TypeKind`**, and it found the biggest single item left: `typeof`
> resolved to nothing, in **37 VPP files**. Twenty-two probes over nine variants — `Builtin` at
> every width including `__int128` and `_Float16`, `Named`, `Tag` for `struct`/`union`/`enum`,
> `Ptr`, `Array`, `Func` — and only the two `typeof` variants failed. A *control* in that census
> also turned up a **source-triggerable panic** on 128-bit signed arithmetic.
>
> **The axes and what each gave:** `ExprKind` vs the generator (270, two defects), `CmpOp` vs
> lowering (271, seven builtins), every CIR enum vs lowering (272, the vector cluster), `UbKind`
> (272, clean), `Kw::` vs the parser (275–276, `_Generic` and `__label__`), `StmtKind` vs the
> generator (277, clean), the **declarator grammar** (278, five defects — the best single yield,
> closed over waves 278–282), the **preprocessor** (281, clean), `TypeKind` (283, `typeof` plus a
> panic).
>
> **What is left, with no census to point at it:**
>   - **`__attribute__((aligned(1)))` should reduce an alignment** and `_Alignas(1)` is a
>     constraint violation gcc rejects; both arrive as one `aligned` attribute, so telling them
>     apart needs the parser to record the spelling. No reach in the target.
>   - **`__VA_OPT__`**, `#elifdef`/`#elifndef` diagnostics, and `defined` produced by macro
>     expansion — all *declared* preprocessor limits, all 0 VPP files.
>   - **`BinOp::PtrDiff`** is dead in every crate with a working alternative; the spec also lists
>     a `BinOp::PtrAdd` "reserved" that does not exist in the code. Fix both or leave both.
>   - **The soak**, which is now a different search: the corpus reaches vectors, `_Generic`,
>     `__label__`, the classification builtins and `!`, none of which it did a month ago.
>     `SOAK_CF=1` was clean over seeds 0..1200 in wave 278 with the vector-carrying corpus.
>
> ### 🔴 The generator's grammar is the frontier again — ask what the AST can hold
>
> **Every census axis is now run, and wave 277 ran the last one** (`StmtKind` against the
> *generator*). It came back clean, and the clean result is the record: of the three statement
> forms the control-flow channel never emits, `Empty` already works (the pattern I censused with
> was a bad proxy), `Asm` is a declared gap, and `GotoIndirect` is declared **fixture-only** by
> 020 contract 42 — "VPP contains no computed gotos, verified, zero real uses tree-wide", which I
> re-checked: the one grep hit was the comment `/* This can only be reached via goto */`.
>
> **The axes and what each gave:** `ExprKind` vs the generator (270, two defects), `CmpOp` vs
> lowering (271, seven builtins), every CIR enum vs lowering (272, the vector cluster), `UbKind`
> (272, clean), `Kw::` vs the parser (275–276, `_Generic` and `__label__`), `StmtKind` vs the
> generator (277, clean). **The census channel is exhausted.**
>
> **What wave 277 closed off the open list, all negative results:**
>   - **`Const::FuncAddr` is a spelling question, not a gap.** Lowering uses `RValue::AddrOfFunc`
>     in bodies and `GlobalInit::FuncAddr` for file-scope initializers; the operand form is simply
>     the one it never reaches for. Struck from the list.
>   - **`__builtin_convertvector` appears in 0 VPP files**, and vector casts already agree.
>   - **`BinOp::PtrDiff`** remains dead in every crate; `bin()` gives it a *declared* lowering gap
>     rather than a silent `Undef`, so it is dead weight with an honest failure mode rather than a
>     hazard. Note that the spec also lists a `BinOp::PtrAdd` "reserved" that does not exist in
>     the code at all — **fix the spec and the enum together, or leave both.**
>
> **With the censuses exhausted, the remaining channels are the soak and mutation.** The corpus
> now reaches vectors, `_Generic`, `__label__`, the classification builtins and `!` — none of
> which it could a fortnight ago — so pushing `SOAK_CF=1` past its frontier is a different search
> than it was.
>
> **The keyword census is exhausted** (wave 276). All 59 `Kw::` variants now have a production;
> `__label__` was the last, and it is implemented by *renaming* local labels in the parser so
> lowering's per-function label map never sees a collision. The AST, sema and lowering were not
> touched.
>
> **The census axes run so far, and what each gave:** `ExprKind` against the generator (wave 270,
> two defects), `CmpOp` against lowering (271, seven builtins), every CIR enum against lowering
> (272, the vector cluster), `UbKind` (272, clean), `Kw::` against the parser (275–276, two
> features). The IR and keyword axes are done. **What has not been censused: `StmtKind` against
> the *generator*** — wave 217 did it against lowering and found `switch`/`do`-`while`, but the
> question "which statement forms does the corpus never emit" has not been asked since.
>
> **What is left from earlier censuses:**
>   - **`__builtin_convertvector`**, the only vector gap; casts and `?:` already agree.
>   - **`BinOp::PtrDiff` is dead in every crate** and `p - q` works without it. Delete or produce.
>   - **`Const::FuncAddr` has no producer** although `GlobalInit::FuncAddr` does. Confirm it is a
>     spelling question before writing it down as a gap.
>
> **`_Generic` is in** (wave 275) — parser production, AST node, sema selection recorded in
> `Analysis::generic_selections`, lowering, and the two 6.5.1.1p2 constraint violations reported.
> That was the last C11 *expression* form missing. The keyword had been in the lexer's table
> since the lexer was written with nothing consuming it, which is worth remembering as a census
> axis of its own: **a keyword with no production is the same fingerprint as an opcode with no
> producer.**
>
> **The vector census came back nearly clean** (wave 275, probed before choosing this wave's
> work). Vector→vector casts, including gcc's bit *reinterpretation* for same-size casts, a
> vector in a `?:`, and `sizeof` of a cast all already agree. Only `__builtin_convertvector` — the
> value-converting form — is absent, and it refuses loudly.
>
> **What is left on the census, in the order it looks worth taking:**
>   - **Every `Kw::` with no production**, the axis this wave stumbled into. `_Generic` was one;
>     run the whole list before assuming it was the only one.
>   - **`__builtin_convertvector`**, now the only vector gap.
>   - **`BinOp::PtrDiff` is dead in every crate** and `p - q` works without it. Delete or produce;
>     do not leave it.
>   - **`Const::FuncAddr` has no producer** although `GlobalInit::FuncAddr` does. Function
>     pointers work, so confirm it is a spelling question before writing it down as a gap.
>
> **`vector_size` is done for storage, arithmetic and comparison** — waves 272, 273 and 274. The
> extension VPP is written in now initializes, subscripts, computes elementwise, broadcasts a
> scalar, compound-assigns and compares, for `int`, `unsigned char`, `long`, `float` and `double`
> lanes, with NaN handled by the same ordered/unordered split the scalar operators use.
>
> **Still lowered through memory, still on purpose.** `Splat`, `Shuffle`, `InsertLane` and
> `ExtractLane` remain unproduced and should stay that way until someone wants a vector as an SSA
> *value*. Two representations of one type would put "which am I holding?" into every load,
> store, copy, member and cast. A comparison's result is a *different vector type* from its
> operands, which is the first place that would have bitten.
>
> **What the census still has open:**
>   - **`_Generic` is not in the parser at all.** C11, loud parse diagnostic, declared gap. The
>     cheapest remaining item and the one most likely to appear in real headers.
>   - **`BinOp::PtrDiff` is dead in every crate** and `p - q` works without it. Delete or produce;
>     do not leave it.
>   - **`Const::FuncAddr` has no producer** although `GlobalInit::FuncAddr` does. Function
>     pointers work, so confirm it is a spelling question before writing it down as a gap.
>   - **Vector conversions and casts** — `(v4si)f` between vector types, and a vector in a
>     `?:` — are not probed at all. The three waves above covered operators; conversions are the
>     obvious next census on the same type.
>
> **Wave 273 finished the arithmetic half of `vector_size`.** Every arithmetic, bitwise and shift
> operator, both operand orders of the scalar broadcast, unary `-`/`~`, and compound assignment
> now agree with gcc for `int`, `unsigned char`, `long`, `float` and `double` lanes. Combined with
> wave 272's initializer and subscript, the extension VPP is written in now works for storage and
> arithmetic.
>
> **Lowered lane by lane through memory, on purpose.** A vector is an aggregate here — `cty` says
> `CTy::Ptr`, `alloca_for` gives it storage — so `Splat`, `Shuffle`, `InsertLane` and
> `ExtractLane` are still unproduced and *should* be until someone wants an SSA vector
> representation. Two representations of one type would put "which am I holding?" into every
> load, store, copy, member and cast. That is a decision, not an omission.
>
> **What the census still has open:**
>   - **Vector comparisons.** `x == y` and `x < y` are ordinary C and still refuse, loudly and by
>     name. They need a *result* type that differs from the operand type — a signed integer vector
>     of the lane's width, which for a `v4sf` is not the operand type at all — so it is a sema
>     change of a different kind from wave 273's. This is the next vector wave.
>   - **`_Generic` is not in the parser at all.** C11, loud parse diagnostic, declared gap.
>   - **`BinOp::PtrDiff` is dead in every crate** and `p - q` works without it. Delete or produce.
>   - **`Const::FuncAddr` has no producer** although `GlobalInit::FuncAddr` does; function
>     pointers work, so confirm it is a spelling question before writing it down as a gap.
>
> **Wave 272 ran the census over every CIR enum and it found a whole half-built feature.** The
> unproduced variants were `CTy::Vector`, `RValue::Splat`/`Shuffle`/`InsertLane`/`ExtractLane`,
> `Const::Wide`, `InstKind::Opaque`/`Phi`, `RValue::Select`/`Fresh`, `Const::FuncAddr` and
> `BinOp::PtrDiff`. The vector cluster is gcc's `vector_size` — the extension **VPP is written
> in** — and it turned out to be half-supported rather than absent: `sizeof` right, subscript
> store/load right, **initializer silently dropped and every lane read as `Int(32)`**. Two
> defects, both fixed.
>
> **What the census still has open, in the order it looks worth taking:**
>   - **Vector *arithmetic*.** `x + y` on two vectors still lowers to CIR the verifier rejects
>     (`Add operand is Ptr, declared Int(32)`) — a loud refusal, so it is honest, but it is the
>     next thing anyone reading VPP hits. `Splat`, `Shuffle`, `InsertLane`, `ExtractLane` are all
>     still unproduced and this is what they are for.
>   - **`_Generic` is not in the parser at all** — a C11 feature, found while writing a fixture
>     that wanted to witness a lane's type without arithmetic in the way. Loud parse diagnostic,
>     so a declared gap.
>   - **`BinOp::PtrDiff` is dead in every crate** — enum, printer, parser, and nowhere else;
>     `bin`'s catch-all returns `None` for it and the comment says "pointer differences remain
>     unmodelled". C's `p - q` works, lowered as a subtract and a divide, so this variant has a
>     working alternative and no producer. Delete it or produce it; do not leave it.
>   - **`Const::FuncAddr` has no producer in lowering** although `GlobalInit::FuncAddr` does.
>     Function pointers work, so this is a *spelling* question rather than a gap — worth ten
>     minutes to confirm before it is written down as one.
>
> **`UbKind` came back clean** — all five kinds have producers and tests. Note the trap: the first
> census said `MaybeSignedOverflow` had none, because the tests match on the display string
> `may-signed-overflow` and not the variant name. **A name-based census produces false gaps.**
>
> **Wave 271 closed the two forms wave 270 left, and neither closed the way it looked.** Statement
> expressions probed **clean** — twenty-four hard shapes, controlled by two engine mutants — and
> mutating their lowering arm left nothing a generated one could kill, so they were deliberately
> **not** added to the corpus. `_Alignof` went in during wave 270 for its type.
>
> **The census then moved to CIR and paid immediately.** Lowering can emit twelve of `CmpOp`'s
> twenty variants; the eight it cannot are the ordered/unordered distinctions C makes in 7.12.14's
> *macros* and not in its operators — and the executor implements all twenty. Seven macros
> (`isnan`, `isunordered`, `isless`, `islessequal`, `isgreater`, `isgreaterequal`,
> `islessgreater`) were being refused whole, and they now lower to the one comparison each is.
> Still unproduced: `FUEq`, `FULt`, `FULe`, `FOrd`.
>
> **The obvious next censuses, in the order they look worth running:** `UbKind` (which kinds can
> the checker ever report?), `Fidelity` (which degradations can actually be reached?), and
> `InstKind` against the *executor* rather than against lowering. The AST axis is worked out; the
> IR axis had one finding on its first pass.
>
> **Still refusing, deliberately and by name:** `isinf`, `isfinite`, `isnormal`, `signbit`,
> `fpclassify`. None is a comparison — they need a magnitude test, the sign bit, or a
> classification tree — and 015 §7's named refusal is an honest declared limit. That is the next
> obvious capability if numeric C matters more than the census does.
>
> **Wave 270 ran the `ExprKind` version of this and it paid twice.** Of twenty-one variants, three
> appear in *no* generated program in either channel: `AlignofType`, `StmtExpr` and **`!`**. Ten
> hand probes of each — a minute's work, before touching the generator — found `_Alignof` and
> statement expressions already correct on every shape, and `!` **wrong on the first probe**:
>
> ```text
>   int probe(void) { double d = -0.0; return !d; }   chiero says 0, gcc says 1
> ```
>
> C11 6.5.3.3p5 makes `!E` mean `E == 0` and IEEE makes `-0.0 == 0.0` true. Chiero tested the
> *bits*. Widening the probe found a second, larger one: **a float on the right of `&&` or `||`
> refused to lower at all** (`Ne operand is Float(F64), declared Int(32)`), so every
> `x && <double>` in every program had been dropped whole. Both were one root cause — three copies
> of "is this scalar zero", one right — now `zero_cmp_op(ty, equal)`.
>
> **The corpus could not see either, and making it able to took three corrections.** All are the
> same mistake at different depths: `x && !x` short-circuits away the `!`; `!` of a nested
> subexpression never evaluates to a negative zero (41 sites across 2000 seeds, mutant survived all
> 41); `-0.0` was not in the float pool. And the channel itself counted `Refused` as `Discarded`,
> which is why a defect that produced *nothing* was free to hide. That bound is now zero. Six
> lowering mutants that the corpus could not kill before the wave all die to it now.
>
> **`_Alignof` is in the grammar anyway** for its type — `size_t`, unsigned, wider than `int`,
> `sizeof`'s class, where wave 217's defect lived. **Statement expressions are still absent** and
> are the obvious next addition, with the caveat this wave earned: probe first, and grade the
> addition by a mutant it kills rather than by a census that counts it.
>
> **Wave 217 found a real defect by widening the grammar, not by running more seeds.** Seeds
> 800..1600 of the existing soak came back clean — 490 comparisons, zero defects — so the *shapes*
> were exhausted rather than the seed count. Asking what `StmtKind` can hold rather than what the
> generator emits turned up two forms lowering has always supported and the generator never
> produced: `Switch` and `DoWhile`. Seed 344 then mismatched.
>
> **The defect:** `_Bool b = 1; b += -1;` gave 1 where gcc gives 0. C11 6.5.16.2p3 makes `b += e`
> mean `b = b + e` with both operands promoted, so the addition happens in `int` and only the
> *result* is converted; sema coerced the right operand to the lvalue's type first, so `-1` became
> `1` and the expression stopped depending on `b`. **Invisible for every other integer type** —
> conversion to `char` is a truncation and truncation commutes with `+`, `-`, `*`, so
> `(char)(1 + 300)` and `1 + (char)300` are both 45. `_Bool` conversion is `!= 0`, which commutes
> with nothing. That is why a hundred waves of differential testing never saw it.
>
> **The channel and its knob.** `control_flow_programs_agree_with_gcc` is the fixed batch (121
> comparisons), `SOAK_CF=1 SOAK_LO=.. SOAK_HI=..` searches open-ended, and the new arms are gated
> **before any `rng` call** so the eight existing channels are byte-for-byte unchanged. The first
> version was ungated and shifted the memory-UB corpus until `stack-buffer-overflow` appeared in
> two programs instead of enough to grade on — caught by the adequacy guard.
> **Frontier: `SOAK_CF=1` clean to seed 900 after the fix; the plain grammar clean to 1600.**
>
> ### The `Bounded` thread is closed; the three parked decisions are what is left
>
> Waves 222–223 followed wave 221's `gap: Bounded` runs to the bottom. **The machinery is right:**
> `findings()` and `reports()` are empty because a degradation is not a defect in the program, and
> the reason is on `State::assumptions()` with a kind, a span and a detail naming the bound, its
> value and the back edge. Two things came out of looking:
>
>   - **`max_depth` was the one message not naming its value**, at both its sites. Fixed, with an
>     invariant test over `Budget` as a whole rather than a seventh per-field test — including the
>     *tight* case, four instructions against a bound of three, because `> max_depth + 1` still cuts
>     a twelve-instruction program one step late and nothing noticed.
>   - **`take_edge`'s copy of the check was unreachable** and is now a `debug_assert!` on
>     `steps <= max_depth`. `steps` is zeroed at three construction sites, incremented at one, and
>     tested there immediately; mutating the two sites separately killed only the step loop's, and an
>     `eprintln!` fired zero times across the workspace. An unreachable guard always survives
>     mutation, so it protected nothing — the assertion fires the moment a second counting site
>     appears, which is what the branch was for, and mutation confirms it (three tests fail when a
>     second `steps +=` is added).
>
> **What is left is the three parked decisions and the capability list.** No open defect has a
> reproducible trigger right now: the generator channel is searched out (~3900 comparisons, one
> defect), the reporting and symbolic-memory threads are closed, and the budget messages hold to one
> standard. The honest next steps, in the order I would take them:
>
>   1. **A decision** (below) — any of the three unblocks work rather than needing investigation.
>   2. **`tests/corpus/c/pointer_fields.c`**, still owed, and **re-diagnose it**: arrow access is
>      clean on five hand-written shapes, so whatever that corpus file is about, it is not `->`.
>   3. **x87 80-bit floats** and **symbolic floats** — the two remaining capability items, both
>      milestone-sized, and §9's earlier note that a symbolic float needs an FP theory or a
>      bit-blasted encoding still stands.
>   4. **The soak frontier**, if a cheap wave is wanted: `SOAK_CF=1` is clean to 2600 and the plain
>      grammar to 1600, so pushing either is a known-cost, low-yield errand.
>
> ### 🔖 Handoff point — waves 239–248
>
> **239–246 were floats, and they are finished.** From "literals only, and some of those wrong" to a
> complete model of x87's 80-bit format and both narrowings — multiply, exact decimal literals, add
> and subtract, divide, NaNs, subnormals, narrowing to `float`. About 2.4 million cases against gcc's
> own arithmetic, no disagreement. `crates/chiero-cir/src/fp.rs` holds it: `unpack` the single decode,
> `round_to` the single rounding core, `pack` and `to_f32` its two callers. Only *symbolic* floats
> remain, and those need a solver theory.
>
> **245 audited the fall-through hazard** those waves named: of twenty-nine `size_of`/`align_of`
> fall-throughs in lowering, twenty-eight never fire. It fixed two wrong-number defects in the
> conditional operator on the way.
>
> **247–248 turned §9's oldest open item inside out.** The reproduction carried since wave 200 —
> `ca[0] = 5; … return ca[0]` solving to 0 — came with a hypothesis and the check that would settle
> it. Running the check disproved the hypothesis: the array is right at every link, and the 0 is a
> *fresh unconstrained symbol* the engine invents after discarding a correct value. 248 read the code
> around it and left two facts and one explicitly-unverified inference.
>
> ### What the next context should know
>
> **Start at the 🔴 section on `unusable`** — it now carries a verified causal chain, three ranked
> directions, and the one check still to run. Do the check first; the previous hypothesis in that slot
> survived forty-six waves because six readers took its conclusion and skipped its check.
>
> **The float soak harness is in `scratchpad/`** (`soak.c`, `nsoak.c`, `chk`) and must be built in
> **debug** — the release build hid a shift-overflow panic for 960,000 cases.
>
> **The verification lessons in the rules list are not domain-specific** and each cost real time: a
> random soak cannot reach a rounding boundary (shrink and enumerate instead), a generator that makes
> one operand special at a time never reaches the interactions, three hand-picked probes cannot
> distinguish two rules, `x != x` is no test of *which* NaN, a wrong value and an invented value look
> identical from outside, and instrumentation's silence is two facts until a control line separates
> them.

> ### 🔖 Earlier handoff — waves 239–246: floats are finished
>
> **Waves 239–244 built `long double`.** From "literals only, and some of those wrong" to a complete
> finite model of x87's 80-bit format — 239 multiply, 240 exact decimal literals, 241 add and
> subtract, 242 divide (the milestone), 243 NaNs, 244 subnormals. `crates/chiero-cir/src/fp.rs` holds
> all of it: `unpack` the single decode, `round_to` the single rounding core, `pack` and `to_f32` its
> two callers.
>
> **Wave 245 audited the fall-through hazard** those waves named, and the answer was a number: of
> twenty-nine `size_of`/`align_of` fall-throughs in lowering, twenty-eight never fire. It found and
> fixed two wrong-number defects in the conditional operator on the way — 6.3.2.1's missing decay and
> 6.5.15p6's null-pointer-constant rule.
>
> **Wave 246 closed the last float gap**, narrowing to `float`, by parameterizing the rounding core
> over significand width. **Floats are done.** About 2.4 million cases against gcc's own arithmetic,
> no disagreement.
>
> ### What the next context should know
>
> **The soak harness is in `scratchpad/`** and is worth reusing before any float work:
> `soak.c` generates random 80-bit operand pairs and runs them through gcc's x87 by `memcpy`;
> `nsoak.c` does the same for narrowing; `chk` compares against `fp`. **Build it in debug** — the
> release build hid a shift-overflow panic for 960,000 cases.
>
> **The four verification lessons in the rules list cost real time and are not float-specific**: a
> random soak cannot reach a rounding boundary (`2^-62` events — shrink and enumerate instead), a
> generator that makes one operand special at a time never reaches the interactions, three hand-picked
> probes cannot distinguish two rules, and `x != x` is no test of *which* NaN.
>
> **Nothing on the float list is left except symbolic floats**, which need an FP theory in the solver
> or a bit-blasted encoding — a different kind of wave from these seven.

> ### 🔖 Earlier handoff — waves 239–245
>
> **Waves 239–244 were one arc: `long double`.** From "literals only, and some of those wrong" to a
> complete finite model of x87's 80-bit format, verified against the hardware rather than a manual —
> 239 multiply, 240 exact decimal literals, 241 add and subtract, 242 divide (the milestone), 243
> NaNs, 244 subnormals. `crates/chiero-cir/src/fp.rs` holds all of it, with `pack` the single
> rounding site and `unpack` the single decode.
>
> **Wave 245 did the audit that arc asked for**, and its result is a number: of twenty-nine
> `size_of`/`align_of` fall-throughs in lowering, twenty-eight never fire. It found and fixed two
> wrong-number defects in the conditional operator on the way (6.3.2.1's decay, then 6.5.15p6's
> null-pointer-constant rule), and left two things recorded above rather than chased.
>
> **What the next context should know.** The float verification harness is worth reusing before any
> more float work: `scratchpad/soak.c` generates random 80-bit operand pairs, runs them through
> gcc's own x87 by `memcpy`, and `scratchpad/chk` compares that against `fp`. Nearly two million
> cases, no disagreement. Two lessons about it are in the rules list and both cost real time: a
> random soak cannot reach a rounding boundary (`2^-62` events, use shrink-and-enumerate instead),
> and a generator that makes one operand special at a time never reaches the interactions.
>
> **The one float gap left is narrowing `long double` to `float`.** Everything else on floats is
> either done or needs a solver theory.

> ### 🔖 Earlier handoff — waves 239–244 were one arc, and it is finished
>
> Six waves, one subject: **`long double` went from "literals only, and some of those wrong" to a
> complete finite model of x87's 80-bit format**, verified against the hardware rather than against a
> manual. In order — 239 multiply, 240 exact decimal literals, 241 add and subtract, 242 divide (the
> milestone), 243 NaNs, 244 subnormals. `crates/chiero-cir/src/fp.rs` is where all of it lives, with
> `pack` as the single rounding site and `unpack` as the single decode.
>
> **What the next context should know.** The verification harness is worth reusing before writing any
> more float code: `scratchpad/soak.c` generates random 80-bit operand pairs, runs them through gcc's
> own x87 by `memcpy`, and prints operands and result; `scratchpad/chk` compares that against `fp`.
> Nearly two million cases across the four operations, all four classes of special operand, and the
> subnormal boundary — no disagreement. Two lessons about it are recorded below and both cost real
> time: a random soak cannot reach a rounding boundary (`2^-62` events), and a generator that makes
> one operand special at a time never reaches the interactions.
>
> **The one float gap left is narrowing `long double` to `float`**, item 3 below. Everything else on
> floats is either done or needs a solver theory.
>
> **The fall-through audit is done — wave 245 — and the answer is reassuring.** Twenty-nine
> `size_of`/`align_of` fall-throughs were instrumented and the suite run: twenty-eight never fire.
> The one that did led to a real defect, though not the predicted kind: sema's conditional never
> decayed its operands, so `sizeof(c ? a : b)` for two `int[4]` said sixteen and `sizeof(c ? f : g)`
> refused the function. Fixing it exposed a second: `common_type` did not know C11 6.5.15p6, so
> `sizeof(c ? 0 : a)` said four. Both fixed, both soaked in fixtures that break the arms' symmetry.
>
> **Two things wave 245 saw and deliberately did not chase**, either of which is a fine next wave:
>
>   - **`store_slot` hardcodes `align: 4`** — a literal, not derived from the type — so a conditional
>     slot's `store` prints `align 4` while its `alloca` now prints `align 8`, and a `long double`
>     slot would claim 4 where the type wants 16. Visible in
>     `tests/corpus/lowered/corpus_indirect_call.cir`.
>
>     **It is not observable today, and the reason is worth knowing before anyone "fixes" it.** The
>     only consumer of a `Store`'s `align` is the `Misaligned` fault, and `report_faults` filters
>     every one of those out: 021 §5 makes misalignment a finding only in `ub-strict` mode, which
>     does not exist yet, because reporting it unconditionally fired on every `CLIB_PACKED` packet
>     header. So this is a latent wrong value waiting for that mode rather than a live defect —
>     which means the honest order is **`ub-strict` first, then this**, since otherwise the fix has
>     no test that can observe it and the sweep will say so.
>   - **The `f64` fall-through in the *float literal* path is now nearly unreachable** — every
>     `chiero_cir::fp` conversion returns a value where it used to refuse — but it is still written,
>     and a path that cannot fire is a path nobody checks. Worth deleting or proving reachable.

> ### ✅ x87 arithmetic — the milestone, closed in wave 242
>
> ~~An unmodelled store leaves the stale value.~~ **Fixed in wave 238.** A store chiero cannot
> translate now writes a *fresh symbolic value* rather than returning: the program did store, chiero
> does not know what, and "written, value unknown" is the only answer true of both. Marking it
> uninitialized would re-create wave 195's invented `uninitialized-read` — the comment ten lines above
> the fix records that failure, which is why the fix is a symbol and not a refusal.
>
> **Every conversion into and out of x87 is exact and no path produces a wrong number**, so arithmetic
> is the only thing left and it can now be graded: a fixture that computes something is either right
> or a declared gap, with no third outcome hiding a stale value.
>
> **Multiplication landed in wave 239.** `chiero_cir::fp::mul`, wired into `fbin`'s width-80 arm.
> It went first because it is the operation that needs no iteration and no alignment: two sixty-four-bit
> significands make a product that fits *exactly* in a `u128`, so the value is computed with nothing
> lost and rounded once, on the truth. Add the exponents, normalize by one bit (chosen by bit 127,
> never a loop), round the discarded half to nearest-even — and then check for a carry, because
> rounding an all-ones significand up is a *second* normalization and skipping it is the classic
> soft-float defect. Overflow returns an infinity (§7.4 specifies one); underflow, `0 × ∞` and a NaN
> operand return `None`, because the honest answers there are a denormal and two NaNs `fp` will not
> mint. Twelve mutants, all killed; the six that survived the first sweep were all missing fixtures
> and are recorded in the rules list.
>
> **Wave 240 made every decimal literal exact**, so a `long double` now arrives whole no matter how
> it was spelled — decimal, hexadecimal or integral.
>
> **Wave 241 added addition and subtraction.** `fp::add`, with `fp::sub` as `add` with the
> subtrahend's sign flipped — IEEE-754's own definition, and every case that makes subtraction
> interesting is a mixed-sign case `add` must handle anyway. The three difficulties multiplication did
> not have: alignment (the smaller operand shifts right, and what it shifts out becomes a sticky
> flag), cancellation (near-equal operands leave leading zeros needing a variable-length left shift),
> and sign (magnitude decides which operand is subtracted, and §6.3 overrides the sign entirely when
> the result is exactly zero). **The load-bearing argument is that the two hard parts cannot
> interact**: alignment loses nothing until the exponent difference passes sixty-three, and past that
> the result cannot fall more than one bit short of normalized, so a sticky flag never survives a
> long left shift.
>
> **Wave 242 added division, and the milestone is closed.** All four operations run. The loop is
> `u128`'s own division plus one bit taken by hand, because `sa << 65` overflows a `u128` where
> `sa << 64` does not. It needs neither a tie branch nor a carry branch — see the enumeration rule
> below — and division by zero is §7.3's *value*, an infinity, not the fault the integer checker
> reports for the same spelling.
>
> ### What is left on floats, in order
>
>   1. ~~**NaN production and propagation.**~~ **Done in wave 243, and exactly rather than
>      approximately.** The open question — whether a canonical quiet NaN is honest when §6.2 wants
>      the operand's payload — dissolved once the hardware was asked: x87's rule is an invalid
>      operation giving the "real indefinite" (sign 1, significand `0xC000000000000000`), a single
>      NaN operand propagating with the quiet bit set and nothing else touched, and two NaN operands
>      resolving to the larger significand with its own sign. `fp::propagate_nan` and
>      `fp::INDEFINITE` do that, so there is no approximation and nothing for 023 §7 to declare.
>      **`sub` tests for a NaN before flipping the sign**, which is the whole of its correctness:
>      `1 - -NaN` is `-NaN`, and flipping first would differ from x87 in exactly the bit a program
>      inspecting a NaN can see.
>   2. ~~**Subnormals**~~ **Done in wave 244, and it was a wrong answer rather than a gap** — see the
>      fall-through rule below. `fp::unpack` normalizes a subnormal operand into an exponent below
>      the format's floor, and `fp::pack` is now the single rounding site for all four operations
>      *and* both literal paths, doing gradual underflow where it belongs. The encoded exponent field
>      is `0` exactly when the integer bit came out clear, which is why a subnormal rounding up into
>      the integer bit becomes the smallest normal with no special case. Division's two proofs moved
>      to being claims about the quotient rather than the result, since `pack` re-rounds subnormals
>      where ties and carries are ordinary. Verified by 1,980,000 soak cases against gcc's own x87.
>
>   3. ~~**Narrowing `long double` to `float`**~~ **Done in wave 246, and the guess about how was
>      right.** `pack`'s body is now `round_to`, parameterized by significand width: the three
>      decisions — round to nearest with ties to even, shift right instead of normalizing below the
>      floor, promote a subnormal whose rounding carries into the integer bit — are the same at
>      twenty-four bits as at sixty-four, and only the cut moves (`127 - width`). `fp::to_f32` stages
>      the significand and rebases the exponent; `round_to` knows nothing about bias. Soaked at
>      480,000 narrowings against the target, in debug.
>
>      **A NaN keeps the twenty-three bits under its integer bit, with the quiet bit forced on.**
>      Forcing it is load-bearing: a payload living entirely below bit 40 truncates to nothing, and a
>      `float` with an all-ones exponent and a zero fraction is an *infinity*.
>
>      **With this, floats are done.** Every conversion and all four operations are exact across
>      x87's whole finite range and both narrowings, verified by about 2.4 million cases against
>      gcc's own arithmetic. What remains under "floats" is only item 4.
>   4. **Symbolic floats** — still needs an FP theory in the solver or a bit-blasted encoding.
>
> **The verification pattern, which all four operations used and the next float work should:** exact
> fixtures in hex for the rounding boundaries, a mutation sweep, a randomized soak against gcc's own
> x87 through `memcpy` of raw 80-bit patterns (`scratchpad/soak.c` + `scratchpad/chk`, 840,000 cases
> across the four with zero disagreements), and — when a mutant survives both — an exhaustive
> enumeration at a narrow significand width to decide whether a witness exists at all.
>
> **The verification pattern for all three, worth repeating for divide:** exact fixtures for the
> rounding boundaries, a mutation sweep, a randomized soak against gcc's own x87 through `memcpy` of
> raw 80-bit patterns (`scratchpad/soak.c` + `scratchpad/chk`), and — when a mutant survives both — an
> exhaustive enumeration of the algorithm at a narrow significand width to decide whether a witness
> exists at all.
>
> **Surviving mutant, recorded:** `width-guard-dropped`. `chiero-solver` caps a term at 128 bits and
> panics past it, so the fresh value is minted only where a term can hold it; without the guard a
> 256-bit store would mint a *128*-bit symbol and write the wrong size. Nothing observes it because
> the only wide-store fixture (`step.rs`'s `a_store_chiero_cannot_perform_forbids_a_proof`) asserts on
> fidelity and sealing rather than on memory contents. **The stale-value defect therefore survives for
> `Int(256)` and nothing narrower** — say so rather than claiming the class is closed.
>
> ### What else is left
>
>   1. ~~**The fractional decimal `long double` literal** rounds through `f64`.~~ **Fixed in wave
>      240.** `chiero_cir::fp::from_decimal` converts the digits themselves with one correct rounding,
>      and `chiero_sema::decimal_float_parts` hands them over — the same seam `hex_float_parts` uses,
>      forced by 001 §4 keeping sema below cir and right anyway. It needs a big integer, because a
>      correctly-rounded conversion at sixty-four significand bits cannot be done in fixed-width
>      arithmetic: `1e4000`'s exact value is four thousand digits and the rounding depends on all of
>      them. `fp::Big` is the minimum that admits — digits, scale by ten, shift, compare, subtract.
>      The division is bounded twice over, to sixty-six quotient bits and starting sixty-six bits from
>      the end, which is what keeps a 13,000-bit division to sixty-six iterations.
>
>      **What is still `f64`-rounded:** `float_literal`'s return value, which is now used for
>      `X87_80` only when `from_decimal` declines — a value whose nearest `f80` is subnormal. Its
>      comment no longer claims to record a narrowing it does not. `F32` and `F64` literals go through
>      `str::parse`, which is correctly rounded for those widths, so they were never affected.
>   2. **Symbolic floats** — needs an FP theory in the solver or a bit-blasted encoding.
>   3. **UBSan's slugs** — a preference with a compatibility cost, yours.
>   4. **The soak frontier** — `SOAK_CF=1` clean to 2600, plain grammar to 1600.
>
> ### The "also open" list is settled; what is left is milestone-sized or a preference
>
> Wave 227 worked that list to the bottom. **Two entries were stale, three shapes were clean, and
> one live defect came out of the last of them.**
>
> **Stale:** `is_aggregate` excluding `Ty::Vector` is long fixed — the predicate includes it, the
> comment at its definition explains the three-predicates-disagreeing bug, and `differential.rs`'s
> `a_vector_and_a_function_designator_have_no_value_form_either` covers the copy, the high half and
> the scaled index. "Floats do not execute at all" is stale for the same reason (waves 167–172).
>
> **Clean:** all three "shapes untried" agree with gcc — a union inside a struct read through
> `pool[k].u.c[0]`, a `goto` out of three nested scopes, and a `switch` on a struct member reached
> through a pointer, default arm included.
>
> **The defect was found by asking the right question about a duplicate.** The symbolic-index
> variant produced two *identical* `pointer-outside-object` lines. Cross-path duplication is
> deliberate and documented — keying across states once collapsed `buf + 64` and `buf + 128` and
> threw away a witness — so the question was not "why two?" but "why identical?". Because `witness`
> was `obj_size`, a constant, where the field's own doc says "an offset the path allows": for
> `pool + ((i & 31) + 64)` the path reaches offsets 256..380 and the report said 32. Fixed by
> binding the `Sat(m)` the probe already computes and evaluating the offset under it, which is the
> source waves 205 and 208 already use.
>
> ### What is actually left
>
>   1. **x87 80-bit floats** and **symbolic floats** — the two remaining capability items, both
>      milestone-sized. §9's earlier note stands: a symbolic float needs an FP theory or a
>      bit-blasted encoding, and x87's 80-bit format has no Rust primitive.
>   2. **UBSan's slugs** — a preference with a compatibility cost, left to you, and its motivating
>      argument retracted in wave 227: the census classifies chiero's side by the `UbKind` enum's
>      `Debug`, not by `ub_phrase`, so the rows were already comparable.
>   3. **The soak frontier** — `SOAK_CF=1` clean to 2600, plain grammar to 1600. A known-cost,
>      low-yield errand.
>
> **Surviving mutants, recorded rather than filed as tested:** `witness-zero-fallback` (a `0`
> fallback when `a.eval` fails; unreachable in the fixture set, and the choice matters for
> *coherence* — offset 0 would read "at offset 0, which is outside it", contradicting itself, where
> `obj_size` at least is outside); `unsigned-too`, `always-opaque`, `expand-forgets-shadowing`,
> `stamp-uses-call-span`, `map-mandatory`, `nest-unbounded`, `accumulate-dropped`.
>
> **This one has no precedent to follow, and that is why it is still open.** It is a rename with two
> real arguments against: `may-signed-overflow` has no UBSan counterpart, so the sets would only
> partly align; and 023 §6.1's key is the *kind*, so renaming is a compatibility break for anything
> already grouping on it. The cheap middle path, if wanted, is a mapping table in the census channel
> rather than a rename in the engine — that keeps the reports chiero's and the comparison UBSan's.
>
> ### ✅ Wave 205's init-check mutant list is closed — audited in 258, finished in 259
>
> **Three entries were stale**, killed by `solver_gave_up.rs` and never noticed:
> `unknown-is-definite`, `unknown-is-clean`, `always-base-zero` (and `always-base-one` with it).
>
> **`expand-unbounded` is a time/precision bound, not a defect.** Removing the limit gives a *more*
> precise guard at more cost. Nothing observes the precision the limit gives up; that is a fact about
> the bound.
>
> **`expand-forgets-shadowing` is equivalent, and the chains prove it.** Logging every chain
> `select_expand` builds for `symbolic_readback_init.rs`:
>
> ```text
>   16 x  len=8   distinct_values=1
>   16 x  len=16  distinct_values=1
> ```
>
> Every store in every chain carries the **same value term**, and with one value the nesting order
> cannot change the result whatever the conditions are. §9's old recipe — "two stores that may alias
> with different values" — was sound; the shape simply does not occur. It would take an init chain
> carrying two distinct values at aliasing indices, and none does.
>
> **Two beliefs this audit overturned, both plausible and both wrong.** Wave 248 concluded the
> 64-byte fixtures always exceed `EXPAND_LIMIT`; instrumenting shows expansion *succeeds* twenty-four
> times. And shrinking the object to eight bytes — the fix §9 recommended — stops `init_guard` being
> called at all, because a narrow mask enumerates and the symbolic guard is never asked.
>
> ### ✅ A guard below a dereference — the presentation half, done in wave 269
>
> The checker was never needed (wave 185), and what remained was the sentence. It now reads:
>
> ```text
>   null-dereference: access at offset 0 of NULL, where %1 is a pointer parameter assumed to be
>   possibly null; the function tests it against null at t.c:1:31
> ```
>
> **"Assumed" invites "but my callers never pass null".** The author's own `if (p)` four tokens
> later does not, and it was already sitting in the CIR.
>
> **The search follows CIR rather than matching it, twice over.** A parameter is spilled to a slot at
> entry, so the comparison is on a *load* from that slot and not on the parameter's `ValueId`. And a
> null pointer constant on the left — `if (0 == p)` — is materialised: C11 6.3.2.3p3 makes the
> conversion explicit and CIR keeps it, so it arrives as `inttoptr i32 0i32 to ptr` rather than a
> bare `null` operand. Matching the obvious shape found neither.
>
> **What the fixtures pin is *which* check is cited**, which is where the risk is: a comparison
> against another pointer is not a null test, and another parameter's check must not be attributed to
> this one. A finding that cites a line where nothing is checked is worse than one that cites none —
> vague is weak, wrong is corrosive.
>

> ### 🔴 Then: the arithmetic oracle's remaining softness
>
> `arithmetic_ub_agrees_with_gcc_site_for_site` now reads:
>
> ```text
>   programs=54 agree=85 miss=0 extra=1
>      30 / 30   DivByZero
>       7 / 7    FloatCastOverflow
>      18 / 18   Shift
>      30 / 30   SignedOverflow
> ```
>
> - ~~**`FloatCastOverflow` at 7 is the thin row.**~~ **Closed in wave 261, and the recommended fix
>   would not have worked.** The advice was "make the shape more common in the corpus"; the warning
>   attached to it — ask what the row can *observe* first — is what mattered. Mutating the range
>   check says the row is not thin in general but blind in two directions:
>
>   ```text
>     negative into unsigned destination   KILLED
>     signed high end                      KILLED
>     signedness of destination ignored    KILLED
>     signed *low* end                     SURVIVED
>     NaN treated as in range              SURVIVED
>   ```
>
>   A float too *negative* for a signed destination, and a NaN converted to an integer. Neither is
>   about how often a float cast appears, so more of them would have left both holes open. Three
>   fixtures in `signedness.rs` — beside the mirror cases that were already there and were already
>   killing the other three — close it. All five mutants die.
> - **The substring classification is sound and that was checked, not assumed.** An
>   unclassified gcc message becomes kind `"?"`, which chiero never emits, so it scores as a
>   *miss* and fails loudly. §9 predicted this front would be about the substrings; they were
>   fine and the gap was one row down.
>
> Mutation on wave 185: `div-zero-knob-never-fires`, `engine-drops-div-by-zero` and
> `float-divisor-also-zeroed` all die (the last confirms the float exclusion is
> load-bearing — C99 Annex F makes float division by zero *defined*, so emitting one would
> manufacture a site gcc never reports).
>
> ~~`truncation-not-detected` survives, and honestly so.~~ **Closed in wave 260.** The reasoning
> was right that `extra == 0` cannot be asserted — gcc's silence here is not evidence of a false
> positive — and wrong to conclude `extra` cannot be asserted *at all*. Deleting the suppression
> takes it from **1 to 12**, so a ceiling of four kills the mutant and still leaves room for drift.
>
> **Two survivors remain on that assertion and both are named classes.** Forcing the suppression
> always on passes, and the only thing that would catch it is a *floor* — which would require
> chiero to keep reporting sites gcc does not, and that is backwards. Disabling the assertion itself
> passes, which no assertion can observe about itself.
>
> ~~🔴 the substring matching / `zz_census`'s fate.~~ **Wave 185: the substrings were sound,
> and `zz_census` is deleted** — per-run where the new test is per-site, `#[ignore]`d where
> it runs, and without a per-kind floor. Keeping a duplicate that can only give a softer
> answer invites reading it when the sharp one fails.
>
> ### 🔴 Still owed: symbolic UB checking
>
> Wave 174 planned this and wave 175 did **not** do it, deliberately. The premise was that
> census row 1 was symbolic operands; probing first showed that was false — the generated
> programs are closed, run at `fidelity Exact`, and every value in them is concrete — and
> the real cause was `sext` not folding. Building the solver path would have been work done
> on a wrong diagnosis.
>
> It stays owed for the case that genuinely needs it: a program with **real inputs**, where
> `x + y` can overflow for some `x` and not others. The machinery is wave 156's
> `symbolic_div_by_zero` — ask the solver, take `Sat`/`Unsat`/`Unknown` as three outcomes,
> keep the term for 023 §9's witness. Note the open design question before starting: with
> unconstrained inputs *every* `x + y` can overflow, so `Sat` alone would report on every
> arithmetic instruction in the program. Decide what makes a report worth making — path
> *forces* overflow, versus path *admits* it — before writing the query.
>
> Everything else on the open-defect list is empty. What remains besides the above is
> tooling and one deliberate deferral:
>
> - ~~The verifier is not run at lowering time.~~ **Done in wave 145.** `refuse_unverifiable`
>   runs it over the finished module and discards any function it rejects, with a diagnostic
>   naming the instruction — so a defect of that class is now a `Refused` rather than a
>   `SilentNoState`. `crates/chiero-lower/tests/verified.rs` holds the invariant.
>   **One branch of it is untested and said so in the commit**: the `is_error()` filter that
>   keeps a *warning* from refusing a function. Nothing in the fixture set produces a verify
>   warning, so the mutation for it is equivalent. If a warning kind is ever added, that
>   filter needs a fixture.
> - ~~The AST shrinker.~~ **Done in wave 146**, and **line-based rather than AST-based** —
>   the generator emits one statement per line, so line-deletion is already a valid operator
>   over that shape, and a deletion that breaks a reference is rejected by the compile the
>   pipeline runs anyway. The prelude reduces by whole declarations, counted by brace depth.
>   On a reintroduced wave-142 defect it goes from a full program to four lines.
> - ~~The refusal-ledger ratchet.~~ **Done in wave 147.** `KNOWN_GAPS` in `generated.rs` is
>   a closed list with a reason beside each entry; an unmatched refusal fails the run. It is
>   populated because floats joined the grammar: 101 refusals, all accounted for.
>   **`refuse_floating` in lowering is a capability statement, not a workaround** — floats
>   have never worked, and 015 §7's rule is that a construct lowering cannot represent is a
>   diagnostic rather than a silent gamble. Deleting that function is what implementing
>   floats looks like, and the `KNOWN_GAPS` entry is what will fail when it happens.
> - **The `Gap` verdict is unexercised.** It reads `Fidelity` to tell a declared modelling
>   limit from a defect (023 §7), and two mutations deleting it survive — because
>   `refuse_floating` stops floats before the engine and nothing else in the grammar
>   degrades. Kept deliberately, with the reason in the code. The next thing that degrades —
>   a budget, an unmodeled extern, an engine `lowering_gap` — is what will exercise it.
> - `xtask diff-soak --seed N` for open-ended search, with CI keeping the fixed batch.
>
> ### The corpus misses every path this wave touched
>
> The review read all 13 corpus files and confirmed the byte-identical goldens are genuine:
> the only pointers in the whole corpus are plain locals, there is **no pointer-typed global,
> no pointer member, no struct copy, no local-array decay, no compound assignment on a
> pointer, no float and no vector**. All the changed sites are unreachable from it, which is
> why 1102 tests passed over six defects.
>
> One file would cross all of them: `tests/corpus/c/pointer_fields.c` with an `int *gp`
> written and dereferenced, a struct holding an `int *` written and read through the member,
> a struct copy-initialized from another and passed by value, and an aggregate `return`.
> **Write it.** The corpus is what the goldens quantify over, and right now it certifies
> nothing about any of this.
>
> ### Also open
>
> - Shapes untried: a union inside a struct under a symbolic index; `goto` out of three
>   nested scopes; a `switch` on a struct member read through a pointer.
> - **Floats do not execute at all** — `float f; f = 2.5f; return (int)f;` returns nothing,
>   for a *local*, so it is not a lowering-type problem. Untouched by wave 132 and untested.
> - `is_aggregate` excludes `Ty::Vector` while `aggregate_size` includes it, and `cty` maps
>   `Ty::Vector` to `CTy::Ptr` — so a vector lvalue read as a value may have defect 1's shape
>   still. Unverified; a GNU `vector_size` fixture would settle it.
> - Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>   untested; `Bits` path steps are not emitted.
> - ~~022 §4's watchdog.~~ **Done in wave 163.** `UnknownReason::Timeout`, a reader thread
>   per session so the pipe read is abandonable, and a 10s default overridable by
>   `$CHIERO_SMT_TIMEOUT` (0 disables). A timeout is deliberately **not** retried — the
>   existing restart-and-replay path is for a *death*, and contract 14's correctness comes
>   from `query` redeclaring variables on a fresh session rather than from the watchdog.
> - ~~`--dump-queries <dir>` is still owed.~~ **Done in wave 164** as
>   `$CHIERO_DUMP_QUERIES`, matching the two knobs beside it; the CLI flag is one line over
>   it once `chiero-cli` exists. **022 §4 is now fully implemented** — discovery (161),
>   provenance (162), watchdog (163), dump (164).
> - **The dump is a *reconstruction*, not a transcript**, and that is load-bearing: the wire
>   declares only variables the live process has not seen, so dumping the bytes yields a file
>   that replays only after every earlier query in the session. Anyone extending the dump
>   should keep it standalone — contract 17 is a round trip, and the failure is quiet.
> - ~~`(set-option :timeout N)` is still not sent.~~ **Done in wave 165**, at nine tenths of
>   the watchdog so the solver gives up first and the process survives. **022 §4 has no
>   unimplemented clauses left** — discovery (161), provenance (162), watchdog (163), dump
>   (164), solver-side timeout (165).
> - ~~Where to audit next: 021 §5/§6.~~ **Audited in wave 166 and clean.** Contract 13b
>   (an `IntToPtr` of a wholly unconstrained symbol → `Fidelity::Unknown` + an unresolvable
>   -pointer finding) is implemented and says the right thing; §5's access order, `ub-strict`
>   alignment recording and contract 26's memoization are all present; §5.1's interval tree
>   is **not built and the code says so**, with the semantics preserved by an arithmetic
>   filter ahead of a capped solver sweep. A pointer *loaded from* lazy memory correctly does
>   **not** take step 4 — §6's chained materialization gives it an object, which is what
>   `lazy_depth`/`max_depth` are for. It looks like the same case and is not.
> - **🔴 Floats are now the top capability item, and the soak says why.** Seeds 200..800: of
>   600 generated programs, **293 were refused for floating point** and 226 discarded as
>   undefined, leaving **81 compared — 13%**. The channel that has found more defects than
>   any other spends half its budget on programs chiero declines to lower. Floats are not
>   just a missing feature; they are the largest single drag on detection.
>   **The blocker was narrower than it looked.** A *symbolic* float still needs an FP theory
>   or a bit-blasted encoding, and that remains milestone-sized. A *concrete* one needs
>   nothing: wave 167 gave the engine `Const::Float`, the five arithmetic ops, FNeg's
>   siblings and the six FP casts, all folded from bit patterns, with a symbolic operand
>   staying `Fidelity::Unknown`. **Every one of the 293 refusals is a closed
>   `int probe(void)` where every float is concrete.**
> - ~~🔴 Next: remove `refuse_floating` from lowering.~~ **Done in wave 168.** Five separate
>   omissions, not one: the literal (`Const::Int` with a catch-all zero), the opcodes
>   (`cir_binop` mapped everything to the integer table), the instruction `ty`, all four
>   conversion combinations in `convert_for_store`, and `FNeg` in the engine. Generated
>   comparisons went **73 → 97** on the same seeds.
> - ~~🔴 Next: float comparisons.~~ **Done in wave 169.** Generated comparisons **97 → 109**;
>   the float refusal ledger entry is gone. Three things worth carrying forward:
>   * **CIR has no `FOGt`/`FOGe`** — `a > b` is `FOLt(b, a)`, and `cir_fcmpop` returns the
>     opcode *and* the swap together so they cannot drift apart.
>   * **`!=` on floats is `FUNe`**, the unordered one, because C's `isnan` idiom is `x != x`
>     and `FONe` is false for NaN.
>   * **`(_Bool)f` is a comparison**, and `-0.0` is the case that proves it: its bits differ
>     from `+0.0` while C says it is false, so an integer `Ne` on the patterns is wrong.
> - **Two defects found underneath, both pre-existing and neither float-specific.**
>   `chiero-mem`'s byte-splitting store extracted a byte from a *one-bit* term (a `_Bool` is
>   one byte of storage holding one bit) and **panicked** — present in wave 168's run, masked
>   by the refusal. And `Trunc` trusted the instruction's declared width over the term's.
>   Both now degrade or widen rather than crash. **A panic reachable from verifier-clean CIR
>   is the one outcome 023 forbids**, and it took a new kind of value to expose a shape the
>   integer path had all along.
> - **The float work is measured, not asserted.** Seeds 200..800 compared **81** programs
>   before floats; seeds 800..1400 compared **320** after — four times the throughput from
>   the same generator. **Soak frontier is now seed 1400.**
> - ~~Mixed int/float operands.~~ **Fixed in wave 170**, found by that soak: lowering read
>   `is_float(lhs)`/`is_signed(lhs)` — the *left* operand only — so `d + 1` worked and
>   `1 + d` emitted `Add` with `ty: Int(32)` over two doubles. Two things to carry:
>   * **014 already inserts the conversion.** The first fix converted the operands again and
>     produced a chain of three casts. What was wrong was only the instruction's *opcode and
>     declared type*, never the values.
>   * **`eval_ground` is not `as_const`.** `sitofp` of a `sext` of a loaded byte is ground
>     and is not a `Const` node, so `char c = 2; c < 2.5` produced no value until all four
>     float evaluators read ground terms. Wave 162 hit this in the solver; it recurred here.
> - ~~🔴 chiero does not detect float-to-integer overflow at all.~~ **Fixed in wave 172.**
>   `UbKind::FloatCastOverflow`, detected inside `fcast` because a `Cast` never reaches
>   `note_ub` (which is driven from `RValue::Bin`), and named by `UndefinedArithmetic`. The
>   rule is about the **integral part**, so the test is against the *truncated* value —
>   `(int)-2147483648.5` is defined and `(unsigned)(-0.5)` is too. NaN is a separate arm
>   because a range test alone lets it through.
> - **Originally recorded as:** Raised by the user during
>   wave 171 — "UB is something that should be warned about?" — and the instinct was right.
>   Two separate things were being conflated, and only one of them was fine:
>   * **Discarding a UB program from the *differential* channel is correct** and says nothing
>     about caring. gcc's answer for an undefined program is not an oracle, so there is no
>     truth to compare a *value* against. That is all wave 171's filter fix did.
>   * **Reporting the UB is a different job, and chiero does most of it.** Measured:
>     `7 / z`, `1 << 33` and `INT_MAX + 1` all produce findings (wave 157's
>     `UndefinedArithmetic`). `(unsigned short)(-4294905087.0)` produces **nothing** —
>     `note_ub` covers `DivByZero`, `Shift` and `SignedOverflow`, and C11 6.3.1.4's
>     float-to-integer conversion is not in `UbKind` at all.
>   So the fix is a new `UbKind` plus a concrete check in `note_ub` (the operand is a folded
>   float there, so it needs no solver query), and the checker picks it up for free.
> - **The UB census** — what `-fsanitize=undefined,address,float-cast-overflow` reports
>   against what chiero records, over 300 generated programs (236 compared). `zz_census`
>   in `generated.rs` (`#[ignore]`d, ~45s):
>   ```text
>     w174     w175     w176
>     18 / 0   18 / 18  18 / 18   signed integer overflow
>     10 / 5   10 / 10  10 / 10   left shift of negative
>      2 / 0    2 / 2    2 / 2    shift exponent
>     12 / 8    8 / 12  12 / 12   outside the range of representable values
>     22 / 6    7 / 22   6 / 6    left shift of N by M places cannot be represented
>      1 / 0    1 / 0    (none)   ZZ FALSE POSITIVE (gcc silent)
>   ```
>   **Every row at parity, no false positives.** The last row's `7 / 22` was the
>   *measurement*: it matched the substring `cannot be represented`, which also appears in
>   gcc's signed-overflow message ("signed integer overflow: X * 31 cannot be represented in
>   type 'long int'"), so it counted row 1's programs a second time and graded them against
>   `Shift` — a kind they never produce. Tightened to `places cannot be represented`, the
>   real count is 6 and chiero finds all 6.
>   The measure is still deliberately loose in the other direction: "gcc printed it *and*
>   chiero recorded a matching kind somewhere in the run", with no span or operand matching.
>   Parity here is not proof of agreement, it is the absence of the gaps this can see.
> - **The census has a false-positive column**, added in wave 175 and permanent. The table
>   above counts only rows where gcc said something, so a chiero report on a program gcc
>   runs clean can never appear in it — and wave 171's rule makes that the expensive kind of
>   wrong. Measuring it was a precondition for shipping a change that makes many more checks
>   actually run. **1 in 236**, and it is a real defect rather than a checker artifact — see
>   "do this first" above.
> - ~~**🔴 020 decision needed: `Shl` carries no signedness.**~~ **Decided and done in wave
>   174**, and it was broader than `Shl`. `RValue::Bin` now carries `signed: bool` and
>   `note_ub` reads it. The bit rides on the *instruction*, as LLVM's `nsw` does, rather
>   than on the opcode (splitting `Add` would name a distinction the hardware does not
>   make — unlike `SDiv`/`UDiv`, where it does) or on `CTy::Int(w)` (which would have to
>   grow a signedness everywhere to answer a question only arithmetic asks). The rejected
>   cheap option was letting bare `Add`/`Shl` mean "signed" and adding `UAdd`/`UShl`: zero
>   test churn, but an opcode whose name hides its signedness is the same implicit
>   assumption that caused the defect.
> - **The same dropped bit was also producing a *false* report**, which the census could not
>   see because it only counts what gcc flags. `note_ub` called `.signed()` on both operands
>   unconditionally, so `3000000000u + 3000000000u` reinterpreted as
>   `-1294967296 + -1294967296`, left the signed range and fired — on a program gcc runs
>   clean. Wave 171's rule makes that the more serious half of the wave.
> - **Not every `Bin` is signed, and the generated ones are the interesting ones.** Scaling
>   an index by an element size, subtracting two addresses and negating a byte offset are
>   address arithmetic *chiero* emitted; marking them signed reports the engine's own
>   pointer math as the user's overflow. They are unsigned, which is what addresses are.
>   Only the goldens pin this — `addr-arith-marked-signed` dies on
>   `every_corpus_c_file_matches_its_lowered_golden` and on no behavioural test, because
>   overflowing 64-bit address arithmetic needs an astronomical index. **A golden bless
>   would silently accept it**, so read that part of a golden diff rather than skimming it.
> - **🔴 Row 1 is explained, and is the next wave.** Not `as_const` — checked first, since
>   wave 170 made it the obvious suspect, and literal, local and global operands all fold
>   and all report. The cause is in `note_ub`'s second line: it returns early unless **both
>   operands fold to constants**, so a generated program computing on symbolic values is
>   never checked at all. That is why row 1 reads `0 / 18` while a hand-built `INT_MAX + 1`
>   reports every time, and why rows 2–4 close only partway. **The machinery already
>   exists**: `symbolic_div_by_zero` asks the solver whether a divisor can be zero and is
>   the exact shape needed — ask whether the sum can leave the range, report with a witness
>   when it can, stay quiet when it cannot, degrade fidelity on `Unknown`. Doing this for
>   `Add`/`Sub`/`Mul`/`Shl` is what closes the census.
> - ~~**🔴 The cross-check has no legal home.**~~ **Wrong premise, corrected in wave 174.**
>   It needed no new crate: the UB *events* come from `chiero-exec`, and `chiero-lower` may
>   depend on that. Only a census phrased in terms of `default_checkers` was illegal, and
>   checkers turn events into findings — the events are the layer where the gap is. The
>   sanitizer still labels ~220 of every 600 generated programs undefined, a free labelled
>   UB corpus the generator discards; `zz_census` is the first channel to read it.
> - **🔴 Next on floats**: x87 80-bit arithmetic and comparison (no Rust primitive — its
>   width works, so loads and stores do), and **symbolic** floats, which still need an FP
>   theory or a bit-blasted encoding in the solver. The concrete path is now complete. `refuse_float_compare` now declares the two operations
>   still missing, and its `KNOWN_GAPS` entry is what will fail when they land:
>   * the engine's `cmp` has **no float arms**, so `a < b` on floats produces no value;
>   * `(_Bool)f` is *worse than missing* — C11 6.3.1.2 makes it "compares unequal to 0" and
>     `FpToSi` truncates, so `(_Bool)0.5` would answer 0. It is refused for that reason
>     rather than for being unimplemented.
>   26 of 200 seeds refuse on this, so it is the next real unlock.
> - **Still owed on the float path**: x87 80-bit arithmetic (no Rust primitive; its width
>   works so loads and stores do), and `fptrunc-and-fpext-swapped` survives as an
>   **equivalent** mutant — `fcast` converts via `f64` to the target width, so the two kinds
>   carry no distinct behaviour and only the widths matter. The engine can now evaluate what
>   those programs do; lowering still discards any function mentioning a float, so nothing
>   reaches it. That is the wave the 293 refusals are waiting on, and §9 predicted its shape
>   long ago: "Deleting that function is what implementing floats looks like, and the
>   `KNOWN_GAPS` entry is what will fail when it happens." Expect the ratchet to fire, and
>   expect lowering to need float *emission* (`Const::Float`, the F-ops) to be correct —
>   the engine side is now tested but the lowering side has never run.
> - **Still unimplemented on the float path**: x87 80-bit arithmetic (no Rust primitive; its
>   width works, so loads and stores do), and the float comparisons `FOEq`/`FOLt`/… which
>   wave 167 did not reach — `cmp` still has no float arms, so `a < b` on floats is a gap.
> - **`zz_soak` is the open-ended mode** §9 owed since wave 139: `#[ignore]`d, range under
>   `SOAK_LO`/`SOAK_HI`, prints a census rather than a verdict. **Frontier reached: seed 800.**
>   The next session should start there.
> - **One mutant survives in the watchdog** (wave 163): not killing the child on timeout,
>   because `Drop for Session` also kills it and the caller drops the session immediately.
>   The line stays and documents the distinction (`Drop` tidies up; the watchdog *ends the
>   query*), but no fixture can tell them apart today.
> - **Shift and signed-overflow UB are still concrete-only** (wave 156). `note_ub` answers
>   for two constants; wave 156 added a solver query for *division* only, because a division
>   is rare and its question is one narrow feasibility check. Overflow is a question about
>   every `Add`/`Sub`/`Mul` in the program and a shift amount is nearly as common, so asking
>   there is the per-instruction cost the original decision was about. **Not an oversight —
>   a scope line**, and the mechanism to extend it is `symbolic_div_by_zero`.
> - ~~A UB event is not yet a finding.~~ **Done in wave 157**: `chiero_check::
>   UndefinedArithmetic`, in `default_checkers()`. No engine change was needed — a checker
>   already reaches `st.ub_events()` through `Event::AfterInst`, which is worth knowing
>   before designing: the gap was a missing *consumer*, not a missing mechanism.
> - ~~The witness beside a division by zero can be wrong.~~ **Fixed in wave 158.**
>   `State::witness_requires` carries the *condition* a finding depends on — the term, not
>   the model, because a model answers for the path as it was at that instruction and the
>   state runs on. The witness is solved against the path **and** those, and `pinned` counts
>   them, so `100 / (x - 42)` now names x = 42 and says the fault needs it.
> - ~~A witness is one per *state*.~~ **Done in wave 159.** `UbEvent` carries the condition
>   that makes it a fault, the checker passes it through `Action::ReportRequiring`, and
>   `StateFinding` gets its own `requires` and its own solve. The state's witness still tries
>   to satisfy everything at once, so a reader looking at the *state* is not shown a number
>   that reproduces nothing when one exists that reproduces everything.
> - **Every wave-159 mutant dies only in `chiero-check`.** The mechanism is in `chiero-exec`
>   and no fixture there has two findings needing different inputs, so the evidence for a
>   `chiero-exec` feature lives entirely in the crate above it. Not wrong — the checker is
>   what makes the behaviour reachable — but it means a `chiero-exec`-only mutation run
>   reports a clean sweep over code nothing in that crate tests.
> - ~~A symbolic divisor tier 1 cannot decide is a declared miss.~~ **STALE, and it was
>   never a defect** — checked in wave 161. Escalation already worked; what was missing is
>   that nothing escalated by *default*, which wave 161 fixed. `100 / (x + 1)` now reports.
> - **`fork-drops-reported` survives in `UndefinedArithmetic`** (wave 157). Emptying the
>   per-path `reported` at a fork changes no test. A fixture was written — fault, branch,
>   both children return to the same site — and measurement shows both children finish
>   holding the *same* pre-fork report, which `reports()` collapses by id, so the cloned
>   field is never consulted. The field is right; the fixture that would observe it is not
>   yet known.
> - **023 c17 / the wave-117 `fork_on_offset` survivor — STALE as a defect**, checked in
>   wave 161. With a backend the enumeration completes exactly: `a[x & 7]` forks eight ways,
>   reports all four out-of-bounds accesses, and leaves the four in-bounds paths `Exact`.
>   What waves 153–156 recorded as a gap was tier 1's inability to prove "there is no ninth
>   offset", and wave 161 made a backend the default. **Nothing is owed here.**
> - **`is_aggregate` excludes `Ty::Vector` — STALE, checked in wave 153.** It includes it,
>   and matches `aggregate_size`. The entry claimed a divergence that is not there; a
>   `vector_size` fixture is still owed to say whether the *behaviour* is right, but the
>   stated reason to write one was wrong. (Wave 149's rule, third time.)
> - **Two mutants survived wave 153, recorded rather than hidden.** `and-split-any-width` is
>   *equivalent* with an argument: a predicate term has width 1, so a wider `and`'s operands
>   can never be atoms and both readings end in `Unknown`. `vars-of-immediate-only` survives
>   because nothing observes the walk's **depth** — `witness.rs`'s hand-built CIR puts the
>   variable one level down, and only a C-lowered program has the four-wrapper shape. The
>   fixture that kills it is a witnessed fault from a symbolic run. ~~owed~~ **written in
>   wave 156, and it kills the mutant.**
> - **String literals: what wave 151 did *not* close.** `chiero_sema::strlit` now owns phase
>   5 for string literals, but three things sit beside it and were left alone deliberately:
>   ~~*character constants* still have their own reading~~ **done in wave 152** — and the
>   gap it recorded was understated: `parse_char_literal` had no `\x`, no octal past `\0`,
>   no UCN, no multi-character constant, and no attention to the prefix, so sema typed every
>   character constant `int`. `string_element` hardcodes a plain literal's `char` as **signed** rather than
>   asking `target.char_signed`, which is right for the one target 014 models and wrong the
>   moment a second one exists; and a value that is not a Unicode scalar (a lone surrogate,
>   anything above U+10FFFF) is passed through truncated rather than diagnosed, because gcc
>   rejects those literals outright so nothing well-formed reaches the arm. The last is a
>   *silent* fallback in a module whose entire point is not having one — if a front end ever
>   accepts such a literal, that arm should push a diagnostic instead.
>
> ### ✅ Closed in waves 256–257 — the four unobservable extension sites
>
> Wave 254 audited `is_signed`'s extension callers, found no defect, and found that its twelve
> fixtures observed **one** of the four decisions. Waves 256 and 257 closed the other three, and not
> one of them needed a better fixture.
>
> **Three were duplicates.** `convert_for_store`'s inline `SiToFp`/`UiToFp` (256), the array-index
> `SExt`/`ZExt`, and `widen_to_64`'s own choice (both 257) each spelled out a decision `cast_kind`
> already made. They agreed, so nothing was fixed — but in every case the copy nothing could observe
> *was* the duplicate, and the mutation that survived at the copy dies at the shared site. There is
> now one place in `chiero-lower` that decides a cast kind.
>
> **The fourth needed a different oracle.** `widen_to_64` forced to sign-extend survives every value
> comparison, and structurally must: an index is promoted to `int` first, so the only narrow
> unsigned source is a 32-bit `unsigned`, and its top bit being set means an index of at least
> `2^31` — never in bounds. Sign- and zero-extension cannot disagree about an index a program can
> legally form. `an_index_widens_by_its_own_signedness` in `shapes.rs` asserts the *emitted
> instruction* instead, and the mutant dies.
>
> **The transferable rule, which cost waves 254 and 255 to find:** when a mutant survives, ask in
> order — is this site a duplicate of one already watched? can the decision change an answer at all?
> — before writing another fixture. Wave 255 spent an entire wave on fixtures for a site that
> should not have existed.
>
> ### ✅ Closed in wave 253 — the generator now catches the bit-field defect
>
> **The controlled experiment passes at last**: revert `field_signed` in `chiero-lower`, run
> `generated_programs_agree_with_gcc`, and seed 49 fails. That is the same program wave 252 held up
> as the counterexample.
>
> **The fifth condition was the read's *context*, and the fix was deleting a cast.** A bit-field read
> that is the operand of an explicit cast has `top(e)` equal to the field's own type, so a wrong
> signedness answer is masked. The checksum wrote `acc = acc * 31 + (long)(x.f);` for every field of
> every struct, so every observation the generator could make went through the shape that hides the
> bug. Dropping the cast changes no value — `acc` is a `long` and `+` promotes anyway — it changes
> which conversion the *typed AST* records, which is invisible in the emitted C. A `float` member
> keeps its cast, because there `(long)` is a real conversion.
>
> **What the arc cost, recorded because the shape will recur.** Waves 250 and 252 each raised a proxy
> five-fold and caught nothing: bit-field values that span their width, then records steered into
> scope. Both were real improvements to a shape that could never discriminate. Wave 251's four-factor
> model was not imprecise, it was missing the factor that mattered — and the way out was not a better
> model but a single counterexample (seed 49) run through `judge()`.
>
> The guards left behind: `the_generator_reads_a_field_without_a_cast`,
> `the_fixed_batch_can_discriminate_an_extension_defect`, `a_bitfield_struct_reaches_the_checksum`,
> and the four-fixture boundary in `an_unsigned_bitfield_zero_extends` that pins which contexts fire.
>
> ### ~~🔴 characterise which context makes the bit-field defect fire~~ — answered
>
> **Wave 252 raised the rate five-fold and the batch still does not catch it.** That result is worth
> more than the rate, and it comes with a concrete counterexample rather than a theory. Seed 49
> satisfies every condition wave 251's four-factor model lists and chiero agrees with gcc anyway:
>
> ```text
>   struct S0 { float f0; unsigned short f1; unsigned f2:3; };
>   struct S0 v8 = (struct S0){1.0f, 1u, 4};   <- 0b100, the top bit of a 3-bit field
>   acc = acc * 31 + (long)(v8.f2);            <- read into the checksum
> ```
>
> **So the defect is context-dependent.** `return s.a` triggers it — that is the wave-249 fixture —
> and `(long)(v8.f2)` does not. The generator reads a bit-field exactly one way, through that cast,
> which is why every rate improvement misses.
>
> **The check to run, and it is small.** Revert `field_signed` in `chiero-lower` and put both
> spellings through `agree_with`:
>
> ```c
>   struct S s; s.a = 4; return s.a;              /* known to fail */
>   struct S s; s.a = 4; return (int)(long)(s.a); /* does it? */
> ```
>
> If the cast form passes, the question is what `is_signed(e)`'s `top(e)` answers in each — wave 249
> already found that promotion applies to an rvalue but not an lvalue, and a cast is a third
> context. **Characterise that before touching the generator again**, because the generator's read
> shape is the thing to change and nobody yet knows what to change it to.
>
> The measured shape count is `the_fixed_batch_can_discriminate_an_extension_defect`, now 5 of 200
> and asserted at 5. Read it as "the shape is present", never as "the defect is reachable" — wave
> 252 is the proof those differ.
>
> ### ~~🔴 raise the discriminating rate~~ — done, and it was not sufficient
>
> **Wave 251 ran the check wave 250 asked for and the hypothesis was wrong.** A struct with a
> bit-field *does* reach the body: of 3000 seeds, 180 declare one as a local and **156 checksum its
> fields**. The guard is `a_bitfield_struct_reaches_the_checksum`, kept so a later body-grammar
> change cannot silently stop routing them.
>
> **What the numbers now say, as an estimate rather than a measurement.** For a program to be able to
> observe an extension defect, four things must coincide, and each is roughly measured:
>
> ```text
>   a bit-field struct reaches the checksum     156 / 3000   ~5%
>   the field is `unsigned`                      15 / 44     ~1/3
>   the stored value sets the field's top bit     9 / 15      ~1/2   (wave 250 raised this)
>   the program is compared, not discarded       >= 100 / 200 ~1/2   (existing adequacy floor)
> ```
>
> That multiplies to roughly **one discriminating program per two hundred seeds** — so the fixed
> batch expects about one and the 700-seed soak about three. Observing zero is unlucky rather than
> impossible, which is why "run more seeds" is the wrong instinct: the rate is the problem.
>
> **Two knobs worth trying, in order.** Both are cheap and both must be gated *before* any `rng` call
> so the other channels' streams are unchanged (wave 250 learned that the hard way):
>
>   1. **Make a bit-field prefer an `unsigned` member.** The choice currently falls out of whichever
>      fields happen to be `Ty::Int`/`Ty::UInt`, giving one in three. A `signed` bit-field cannot
>      expose an extension defect at all, so this is a third of the coverage going to a case that is
>      already covered by every other integer member.
>   2. **Make the body's struct-local arm prefer a record that has a bit-field** when one exists.
>      The arm picks uniformly from all records today.
>
> Either should be verified the same way: revert the `field_signed` fix in `chiero-lower`, run the
> fixed batch, and see whether it now fails. That controlled experiment is the only thing that
> settles it — the metrics above are proxies and wave 250 showed a proxy can improve while the goal
> does not move.
>
> ### ~~🔴 the generator can construct the shape and still not observe it~~ — the routing half is answered
>
> **Wave 250 widened the generator's bit-field values and it did not catch wave 249's defect.** That
> is the finding, and it is worth more than the widening. The controlled experiment: revert the
> `field_signed` fix in `chiero-lower`, run the fixed batch plus 700 soak seeds against the widened
> generator, and nothing fails.
>
> What *is* now true, measured over 3000 seeds: bit-fields appear in about a quarter of programs, are
> read in most of those, and an `unsigned` one's stored value now sets the field's top bit about half
> the time — the half where sign- and zero-extension disagree. The guard is
> `the_generator_fills_an_unsigned_bitfield_s_top_bit`.
>
> **So the gap is in how the value reaches the comparison, not in the value.** The generated program
> has no `main`: it is a prelude of `static` functions plus a `probe()` body that accumulates a
> checksum over the scalars, arrays and struct fields *in scope in the body*. A bit-field written
> into a struct that only ever appears as a **parameter of a prelude function** is never in that
> scope, so nothing it holds reaches the checksum. Seed 6 is the shape to read: `h1()` fills a
> bit-field, `h2(p0, p1)` reads it, and nothing calls either.
>
> **The check to run before designing anything** — and this is a hypothesis, marked as one: count how
> often a struct *with a bit-field* is a local in the `probe()` body rather than only a prelude
> parameter. If that count is near zero, the fix is to make the body declare and checksum such a
> struct, not to touch values again.
>
> ### ✅ All four arithmetic-UB grids are filled — waves 261–263
>
> One technique, four kinds, four findings:
>
> ```text
>   FloatCastOverflow   too-negative-for-signed, and NaN          261
>   Shift               the count rule for `>>`                   262
>   SignedOverflow      below the range; and INT_MIN / -1 was     263
>                       missing entirely — a real defect
>   DivByZero           only `UDiv` was observed by a fast test   263
> ```
>
> **The one real defect: `INT_MIN / -1` was not reported at all.** It fell between the `DivByZero`
> clause (`y == 0`) and the overflow clause (`Add`/`Sub`/`Mul`), so the event was absent rather than
> misclassified. It reports `SignedOverflow` now, with `SRem` beside it, and the oracle's classifier
> gained an arm — UBSan words it "cannot be represented in type", which matched none of the existing
> arms and would have scored a *miss* the first time the corpus produced one.
>
> **The technique, if it is wanted for another check.** Mutate every clause; for each survivor read
> the fixture list as a grid and look for the empty cell. The axes that mattered here were operator,
> direction, signedness, and — twice — *which operand*. Wave 263 added a fifth: a clause with a
> two-part condition has four cells, and the two that demand *silence* are as reachable as the two
> that demand a report.
>
> **Two operational notes.** Keep `generated` out of a sweep's test set — it is slow enough that the
> sweep exceeds its timeout, and a sweep killed mid-run leaves the tree mutated, so every later
> result is measured against a corrupted baseline and looks ordinary. `CONTROL KILLED` is the only
> thing that catches it. And do not leave a one-line clause pinned solely by a twenty-second channel.
>
> ### ✅ The memory-fault clauses are swept — waves 264–265
>
> Every predicate that constructs a `MemFault` has now been mutated. Eighteen mutants across three
> sweeps; **two gaps, one dead function, and the rest already covered**:
>
> ```text
>   bounds `off < 0`            nothing accessed *below* an object          264
>   AddressSpace::in_bounds     no production caller at all — deleted       264
>   align `effective < want`    nothing used an under-aligned *object*      265
>   state check (5 mutants)     covered                                     265
>   too_large (2 mutants)       covered                                     265
>   align offset half           covered                                     265
> ```
>
> **Both gaps were the same shape**: a two-part condition tested in one direction. That is now four
> waves running (261–265), across arithmetic and memory alike, and it is the first thing to look for.
>
> ### Where the technique has not been pointed yet
>
> The fault *constructors* are done; the paths that decide **whether to ask** are not. Candidates, in
> rough order of how much a defect there would cost:
>
>   - ~~**`report_faults`'s discharge**~~ **Covered — wave 267.** Five mutants across its
>     `Unsat`/`Sat`/`Unknown` outcomes for both fault kinds, all killed. No gap.
>   - ~~**`unusable`**~~ **Done in wave 266.** Dropping `MaybeUninitialized` from the list survived
>     the whole suite — the definite case was pinned in wave 249 and the conditional one was not.
>     Six of nine kinds survived removal, and **only one was a gap**: logging every discharged fault
>     at the scalar load shows `uninitialized-read` (47) and `maybe-uninitialized-read` (6) are the
>     only kinds that arrive there *with a value*, so for the rest the entry cannot change an answer.
>     That difference is invisible in the mutation table and took one instrumented run. The list is
>     unchanged and now carries the measurement.
>   - **the havoc paths** (024 §2.1) — swept in 267, finished in 268. Four of six mutants die. The
>     two that survive are the uninitialized fill's reset of a *promoted* object's `init` mask, and
>     wave 268 established the whole story:
>
>     **The branch is reachable** — `promote_to_array` is public, so a unit test puts the model in
>     that state directly, and the branch then runs with `Repr::Array` where every attempt through an
>     operation printed `Bytes`. (Ruled out on the way: `havoc_range` refuses promoted objects
>     outright; a sixteen-byte object's symbolic offset enumerates; a sixty-four-byte `write_sym`
>     still leaves `Bytes`.)
>
>     **Its effect is still not observable, and that is not a fixture problem.** A read of a promoted
>     object reports `SymbolicByte` — the contents are an SMT array, there is no concrete byte —
>     *identically* whether the mask says initialized or not. Both kinds are in
>     `yields_unknown_value`, so the value is discarded either way. The reset changes which **kind**
>     is reported, never whether the value is trusted.
>
>     **So this is an API question, not a missing test.** Observing it needs the init mask exposed —
>     an accessor on `Memory`, or a fault kind that distinguishes "symbolic because promoted" from
>     "symbolic because havocked". Worth doing only if the report-quality difference is worth an API;
>     that judgement has not been made and should be made before more fixtures are attempted.

> ### Rules earned, most recent first
>
> **Run a category against two members, and make the second one the documented exception**
> (wave 347). Seventeen incomplete-type contexts asked of `struct I` confirm everything; the same
> seventeen asked of `void` found three defects, because `is_incomplete` omits `void` on purpose
> and each caller had to decide separately. **A predicate with a stated exception is itself a
> checklist** — enumerate its callers and ask whether the exception is right for each.
>
> **A shared predicate is the place to put "needs a size", not each caller** (wave 347). Two of
> three callers had reimplemented the question and one had forgotten half of it. `has_no_size`
> exists so a fourth context inherits the answer instead of repeating the omission — and the
> *return type* deliberately does not use it, which is what a mutant swapping them proves.
>
> **An operator that inverts its operand's question needs a second recursion, not a base case**
> (wave 346). `reads_an_object` stopped at `&` because the operand of `&` is not read — true, and
> the wrong question: forming an address can still require reads, and `&*E` cancels rather than
> compounding. A base case answered two shapes wrongly; descending normally would have answered
> five more wrongly the other way. **Look for the operator that changes the question** — `sizeof`
> is the other one here, and its operand is unevaluated.
>
> **When several messages describe one relation, tabulate the relation's inputs** (wave 345). Four
> linkage diagnostics describe one relation between two declarations. Read individually each is
> accurate *whenever it fires*, so comparing wording to gcc's finds nothing; the matrix of five
> first declarations against four second ones, for objects and for functions, found three false
> positives in forty cells. **Enumerate the relation, not the messages.**
>
> **One rule applied to two categories C treats differently** (wave 345). A function with no
> storage-class specifier is `extern` (C 6.2.2p5); an object is not. The function arm shared the
> object's `deferring: storage.extern_`. This is wave 344's lesson inverted — there, two paths for
> one rule; here, one path for two rules — and both are found the same way, by asking which
> paragraph governs each case rather than which code path does.
>
> **Two paths checking "the same" rule may be checking different rules** (wave 344). A `case` label
> needs an integer constant expression (C 6.6p6) and an initializer an arithmetic one (6.6p8); the
> lists overlap by fourteen entries out of fifteen, and the fifteenth is real. **Find the paragraph
> number for each before merging two checklists** — the overlap is what makes the error attractive.
>
> **A category check can be too strict as easily as too lax** (wave 344). Three waves of
> enumeration found missing rules; the fourth found a *false positive*, `case (int)1.5:` rejected.
> **Run the accepted half of a category as carefully as the rejected half** — wave 303's rule says
> the false positive is the worse defect, and only the accepted half can find it.
>
> **One rule spread across four grammar arms will be implemented in one of them** (wave 343).
> Pointer arithmetic has seven spellings and C types them in four different places; two were
> checked. The give-away is a *flag* that routes around a shared path — `pointer_displacement`
> exists precisely so a compound assignment never reaches the binary arm, which is also why the
> check there never saw it. **When a rule has a shared predicate, grep for every arm that can
> produce the shape**, not every arm that mentions the rule.
>
> **Decide which half to blame before shipping, not after** (wave 343). `p[i]` on an incomplete
> pointee fails as arithmetic *and* as a dereference; only the missing stride is the reason.
> Waves 339–341 found three such messages after the fact — this one was chosen deliberately and
> pinned with a mutant that swaps the two sentences.
>
> **Enumerate the cases a message claims to cover, and try each one** (wave 342). Comparing
> `bit-field width exceeds the width of its type` to gcc's phrasing found nothing; writing a
> bit-field of *every* type it can take found `_Bool b : 2` accepted, because storage size and type
> width agree for every type but one. **A message naming a category is a checklist** — walk it, and
> test the boundary of each entry rather than one entry six ways.
>
> **A mechanism that names things will name the wrong thing** (wave 342). The `declaring` side
> channel was added to name an array in its diagnostic and immediately reported
> `struct S { int bad[-1]; } x;` against `x`. **Any nested walk that builds a type for a
> differently-named thing must set and restore it**, and the nested case belongs in the same edit —
> here a mutant on the restore caught what no fixture had.
>
> **A message that enumerates what is allowed is a claim about the implementation** (wave 341).
> `subscripted value is not an array or pointer` denied vectors, which this engine has subscripted
> for hundreds of waves — the sentence and the code had drifted apart with nothing checking the
> pair. **When a diagnostic lists the accepted cases, test the list against the engine**: assert
> the message *and* that each listed case is silent, or the two will separate again.
>
> **A cursor is not a fact about the program** (wave 341). The initializer walk reports `at`, and
> when it ran past the end it said "initializer index is outside the array" for
> `int a[2] = {1,2,3};` — a program with no index in it. **Before naming a quantity in a
> diagnostic, ask whether the source contains it**; internal positions, invented types and default
> values are the three that keep escaping into messages.
>
> **Assert the phrase, not the words in it** (wave 340). A fixture checking that a message mentions
> `"pointer"` and `"integer"` passed with the two arms swapped, because both words appear whichever
> direction the error runs. **When testing a diagnostic's text, assert the span that distinguishes
> it from its neighbour** — the words a wrong message shares with the right one are exactly the
> ones a fixture must not rely on.
>
> **The context of a conversion was already carried and never said** (wave 340). `coerce` has had a
> `Conversion` — assignment, argument, return — since it was written, and the diagnostic ignored
> it, so three bad arguments produced three identical sentences. **Before enriching a message, look
> for what the call site already knows**; it is usually cheaper than deriving it again.
>
> **A ratchet measures rejection, not explanation** (wave 339). `*x` on an `int` was rejected —
> green row — with "dereference of a pointer to an incomplete type", which is false about the
> program and sends the reader after a missing `struct`. **When a census reports a row as already
> caught, read the message before crediting it**; three of wave 339's findings were wrong
> sentences rather than missing rules, and no ratchet can see those.
>
> **A poisoned operand must not be described** (wave 339). Giving `*nope` an `Error` pointee and
> then reporting it as *incomplete* produced two diagnostics for one mistake. Contract 20's escape
> has to be taken **before** the code invents a type to talk about, not after. Wave 331 learned the
> same lesson for records; this is the pointer form of it.
>
> **A reduction is where a constraint goes missing** (wave 338). `builtin_of` folds a base, a sign,
> a long count and a short flag into one builtin and answers for every tuple — so `int int`,
> `signed unsigned` and `long float` all named a type. The parser's *recovery* paths were fine;
> what had no check was the place several tokens became one value. **Look for gaps where code
> reduces many inputs to one.**
>
> **A keyword with its own arm needs its own case** (wave 338). `two_signs` set on the `signed` arm
> survived, because the fixture's `signed unsigned` sets it from the `unsigned` arm instead — and
> `char`'s length check survived because every `char` case written by hand was a *two data types*
> error that returned earlier. **When a rule is spread across per-token arms, write one case per
> arm, not one per rule.**
>
> **A type is not a width** (wave 337). A plain character constant has type `int` and elements of
> one byte; `char_element` answers 32 and is right about the type, while `'\400'` is still a
> violation. **When a rule is about how much fits, ask what the thing is made of.** The same
> distinction makes `"\x1FF"` illegal and `L"\x1FF"` legal, and a rule written against
> `unsigned char` would reject every correct wide string.
>
> **A blocker in §9 is a design constraint, not just a delay** (wave 337). "`strlit.rs` has no
> diagnostic channel" is what produced the right shape: the defect information exists only
> *during* the decode — once `\q` is a `StrUnit::Char('q')` it is a literal `q` — so the channel
> had to go into the existing walk rather than beside it. **Read the recorded blocker before
> designing; it usually names the constraint the design has to satisfy.**
>
> **When a rule maps spellings onto representations, only a value separates equal-width kinds**
> (wave 336). Three mutants survived `sizeof`, `_Alignof` and `_Generic`-against-the-standard-types
> because `__bf16` and `_Float16` are both two bytes, and `_Float64x` and IEEE quad are both
> sixteen. Two needed associations naming the exact type; the third needed **arithmetic**, because
> no type test can see a format. **Ask what two candidate mappings would share, and test the thing
> they do not.**
>
> **A second implementation of a grammar is where the defect will be** (wave 336). `number_defect`
> knew the suffix grammar and `float_literal` guessed at it with `ends_with('f')` — false for
> `bf16`, `f32x`, `q` and `w` — and the same duplication left `trim_end_matches` parsing `0.0f16`
> as `0.016`. One shared scan answers both questions. This is the fourth time this project has
> found one rule implemented twice and the copies disagreeing.
>
> **A new constraint reports on whatever was already wrong in the shape it inspects** (wave 335,
> and 331 and 328 before it). Suffix validation found `0.0f16` typed as `double`; the
> declares-something rule found an aliased `__gnuc_va_list`; qualified types found `_Generic`
> ignoring lvalue conversion. **Expect the first run of a new rule to fail on the corpus for
> reasons that are not the rule**, and separate the two: accept the shape, record the defect,
> do not half-fix it inside the wave.
>
> **The corpus decides which extensions are optional** (wave 335). C11 has two floating suffixes;
> gcc has a dozen, and every VPP header reaches a `0.0f16`, so the C11-only rule was a false
> positive on all twenty corpus seeds. **Before writing a rule from the standard, grep the corpus
> for what the standard leaves out.**
>
> **"Active" is three states, not two, wherever conditional text is involved** (wave 334). A
> directive can sit in a live region with a live branch, in a live region with a *dead* branch, or
> inside skipped text — and `#if 0 / #endif junk` is an error while the same line nested inside
> another `#if 0` is not. Every hand-written case happened to have a live branch, so keying on the
> branch instead of the region survived mutation. **When a rule is guarded on activity, write the
> dead-branch case explicitly**; it is the one the obvious fixture misses.
>
> **A `python` edit script that asserts before writing loses every edit in it** (wave 334, and
> waves 330 and 317). It happened again — three edits, the second anchor stale after `cargo fmt`,
> and the ratchet then ran green at the *old* count, which reads like success. **One anchor per
> script, and `grep` that the edit landed.** A line-index edit is safer than a text anchor after
> `fmt`, but it silently eats neighbouring lines: the same wave deleted a `const` declaration that
> way and only the compiler noticed.
>
> **A component with only one kind of channel is where the findings are** (wave 333). `chiero-pp`
> had a differential channel and no constraint list, and the first census against it found eleven
> missing rules — more than any run against sema, which has had both for ten waves. A differential
> channel grades what a program *computes* and is structurally blind to a program that should have
> been *refused*. **Check which crates have which channel, and go to the gap.** `chiero-lex` and
> `chiero-parse` are the two left.
>
> **Two rules from one paragraph can need opposite guards** (wave 333). `#` and `##` are both
> C 6.10.3.2–3 operator constraints and read as a pair, but `#` applies only in a function-like
> macro and `##` applies in both. Mutation kills the mistake in each direction. **When two rules
> arrive together, test each one's guard separately** — a shared paragraph number is not a shared
> condition.
>
> **A guard standing in for a missing fact names the fact it is missing** (wave 332). Two rules were
> keyed on `params.is_empty()`, and both comments said outright that "unspecified" was the only
> safe reading because the parser could not distinguish `f()` from `f(void)`. Adding the flag
> turned both guards into what they meant. **When a guard's comment explains what it cannot know,
> that is a costed feature request** — go and look at whether the missing fact is cheap.
>
> **Two branches that look like one case are not one case** (wave 332). `static int g(){...}` and
> `int g(a) int a; {...}` are both "old-style", and the fixture had the first. It takes the
> *empty-list* branch; only a named identifier list reaches the K&R branch, which stayed unobserved
> until mutation said so. **Check which branch a fixture actually executes**, not which rule it is
> about.
>
> **A constraint check pays for itself on code nobody suspected** (wave 331). "A declaration
> declares something" is a rule against `int;`, which nobody writes. It reported
> `typedef __builtin_va_list __gnuc_va_list;` in gcc's own `stdarg.h`, where the lexer had aliased
> the typedef name to the builtin keyword — a defect that made the *type* come out right, so no
> test of a value could have found it. **Add the constraint even when its own examples look
> pointless.**
>
> **A fixture can assert the bug** (wave 331). `the_gnuc_spelling_is_the_same_type` declared
> `__gnuc_va_list ap;` with no typedef and passed — gcc rejects that outright. When a change breaks
> a test, ask gcc what the fixture claims *before* assuming the change is wrong; this is the second
> time (wave 325 was the first) that a passing fixture turned out to encode a wrong answer.
>
> **A new rule needs the poison list checked, not just the happy path** (wave 331). The record rule
> turned one diagnostic into eight because an incomplete record is a well-formed `Ty::Record`, not
> `Ty::Error`, so contract 20's escape did not cover it. **When adding a type rule, ask which
> already-reported states reach it** — `Ty::Error` is not the only one.
>
> **The shadowing case that tests a scoped set is inner-then-*outer*, not sibling-then-sibling**
> (wave 330). A sibling scope's mark already starts past the previous scope's leftovers, so it
> reads none of them and passes whether or not removal works. Only a declaration in the *enclosing*
> scope, after an inner one has closed, touches the stale entries. This is wave 326's rule with the
> missing half: **write the enclosing-scope reuse, not the sibling one.**
>
> **A `python` edit script must write before it asserts, or check that it wrote** (wave 330). Two
> multi-edit scripts raised on their second anchor and wrote nothing, and the second time the
> ratchet ran green at the *old* count — which reads exactly like success. This is the wave 317
> trap recurring. **Prefer one anchor per script**, and when a count is the result, compare it to
> the number expected rather than to zero failures.
>
> **An equivalent mutant is a claim to retract, not a gap to fill** (wave 329). The unary-operand
> check survived swapping the promoted type for the raw one, and the honest reading was not "add a
> test" — no test can exist, because promotion maps `Int` to `Int` and decay maps `Array` and
> `Func` to `Ptr`, so nothing changes side. The comment claiming the promotion was what made `~c`
> legal on a `char` was simply wrong. **When a mutant survives, decide first whether the property
> is observable at all**; if it is not, fix what the code *says* about itself.
>
> **A census pays best when its rows are left visible rather than all closed** (wave 329). Eight
> rows found, five closed, three written into the ratchet unclosed and printed by name. The channel
> is built for exactly that, and it turns "we know about this" from a claim in a commit message
> into a failing list the next wave reads.
>
> **Audit the thing that changes meaning, not the thing that mentions the type** (wave 328). The
> qualified-types effort was costed at 436 match sites and came in at four, because the sites that
> *mention* a type are not the sites that *depend on its identity*. Before paying for a
> representation change, ask which code would give a different answer — usually a far smaller set
> than the code that pattern-matches on it, and the gap between the two is where a wrong estimate
> hides. A side table beside the interning key changed nothing for 274 readers.
>
> **A rule stated only in the negative is half a rule** (wave 328). "May not discard a qualifier"
> landed and nine corpus headers went red inside a minute: C also says where qualifiers *combine*
> (6.5.15p6, the conditional operator), and without that half the new check fires on correct code.
> **When adding a constraint about a property, find the paragraph that says how that property
> propagates**, and expect the corpus to be the thing that asks for it.
>
> **A representation change needs the channels that measure *silence*, not just the ones that
> measure answers** (wave 328). Three regressions came from three different places — the
> differential oracle (`_Generic` selecting `default`), a golden (a vacuous `bitcast i32 to i32`),
> and contract 11's corpus walk (twenty `u64` against `u64`) — and none of them from the fixture
> the wave was written against. A change to what a type *is* touches everything downstream of
> typing, so the gate is the whole suite, run early and often, not the new test.
>
> **`error: test failed` matches a grep for a compile error** (wave 327). Three mutants were scored
> NO-COMPILE that were in fact killed by a failing assertion, because cargo prints `error: test
> failed, to rerun pass ...` on any test failure and the scorer checked `^error:` before checking
> `^test result: FAILED`. **Detect a build failure by `could not compile`**, never by the word
> `error`. A mutation scorer that misfiles a kill as a no-compile hides exactly the mutants that
> prove a test works.
>
> **A guard can be live and still unobservable, and the comment should say which** (wave 327). The
> `scope == Scope::Block` guard on VLA-scope tracking survived mutation, so I instrumented the
> branch instead of arguing: it *is* reached — an array parameter takes it — but a parameter's
> scope is open at every label and every `goto` in the body, so the containment test holds either
> way. Kept, with the comment stating it is unobserved rather than claiming it prevents a bug.
> **When mutation spares a guard, measure whether the branch is reached before deciding whether to
> delete it or defend it** — "unreachable" and "reachable but inconsequential" call for different
> code.
>
> **A rule's name can misdescribe its own shape** (wave 327). "A `goto` may not jump into a VLA's
> scope" sounds like a rule about entering blocks; it is a rule about crossing declarations, and
> the three gcc-verified cases that show it — a non-VLA block is fine, a flat body with no block is
> not, a jump from after the declaration is fine — each defeat a different cheap approximation.
> **Probe the boundary before designing to the name.**
>
> **A scoped set's removal is unfalsifiable until something reuses the name** (wave 326). Three
> scoped sets now exist — `read_only` (311), `read_only_pointee` (316), `register_objects` (326) —
> and every one of them had its `swap_remove` survive mutation until a shadowing case was written
> by hand. The cause is wave 322's rule seen from the far side: fixture authors give things
> distinct names, so the *removal* half of a scoped set is the last thing anything exercises.
> **When adding a scoped set, write its shadowing case in the same edit as its insertion.**
>
> **A coverage number nobody can act on is not a report** (wave 325). The rejection ratchet prints
> the *names* of what it missed, not the count, because "57 of 63" tells the next wave nothing to
> do. 023 §9's rule about reports applies to a test's own output, and a work queue is the form
> that makes a measurement worth keeping.
>
> **Two fixtures can contradict each other, and the older one can be the wrong one** (wave 325).
> Wave 311 accepted `return v();` from a `void` function after checking gcc's default; wave 314
> then established `-pedantic-errors` as this project's calibration, which makes the same program
> a violation. The conflict surfaced only when a *new* rule made both fixtures run against the
> same behaviour. **When calibration changes, the fixtures written before it do not update
> themselves.**
>
> **Aim a generator at where the rules are dense** (wave 324). Twelve hundred randomly-assembled
> legal programs produced no complaint; the same channel aimed at shapes drawn from rejections the
> project had actually shipped caught all five re-injected defects. **A false positive lives where
> rules crowd together**, not in the middle of the language — so a generator for one should be
> built from the bug list, not from the grammar.
>
> **Let the oracle decide what your generator meant to say** (wave 324). A shape the generator
> believes legal and gcc rejects is a bug in the generator, and counting those separately is what
> stops one's own misunderstanding of C from arriving as an engine finding. One shape cost a fifth
> of the output before it was noticed — `return v();` from a `void` function, which gcc accepts by
> default and rejects under `-pedantic-errors`.
>
> **A net that caught nothing must be shown able to catch something** (wave 323). Thirty-eight
> shapes passed, which is either good news or a broken net, and the two are indistinguishable
> without a test. Re-injecting the last three waves' defects settled it: two are caught, two are
> not, and **the two it cannot see are a better finding than the two it can** — one is covered by
> its own fixture, and the other is a diagnostic, which no channel comparing values on
> gcc-accepted programs will ever reach.
>
> **A fixture that names things distinctly cannot test name resolution** (wave 322). Two defects —
> a `static` local shadowing a file-scope object, and the binding not being restored afterwards —
> were invisible because every case in the fixture used a name that appeared once in the program.
> Fixtures name things distinctly for readability, which is right; it also means **the whole
> subject of a scoping rule is a shape a fixture author will not write by habit.** When a rule is
> about *which* object a name means, reuse the name on purpose.
>
> **Re-entry is what a fixture avoids** (wave 322). `static int c = 0;` at the top of a
> once-called function passed for waves, because a variable initialized once and read once
> behaves identically under either storage. Only a second entry separates them, and a test that
> runs its subject twice has to explain why — so it does not get written. **For anything with
> state, ask what the second visit sees.**
>
> **A construct used *incidentally* is tested by nobody** (wave 321). `int a[] = {1,2,3}` had
> length zero and survived eighteen canonical programs, a hundred corpus fixtures and four
> censuses — because every fixture that needs an array writes its length, since the author picks
> the length to make the arithmetic obvious. It surfaced only when it appeared in a *helper* line
> rather than as the subject. **Look at what your fixtures use to set the scene, not at what they
> assert**; the scenery is what nothing checks.
>
> **A probe that ignores fidelity reads every declared limit as a defect** (wave 321). A
> 16-iteration loop returns no value and reports `Bounded`, which is the engine doing exactly what
> 023 §7 requires. `chiero_answer` reads return values only, so the probe called it a failure.
> **Before reporting "no answer", read the fidelity** — the difference between a bug and a
> declared limit is a field this project spent waves building.
>
> **Report the fault, not its consequence** (wave 320). `p->m` on a pointer to an incomplete type
> used to say "no member named `m`" — the lookup searched a record with no members and truthfully
> failed. The message was accurate and useless, and it pointed a reader at the member rather than
> at the missing definition. **When a check fires because an earlier one is absent, the message
> names the wrong thing**; wave 319 spotted the same shape from `switch(*p)` and declined to
> report it there for exactly this reason.
>
> **Two rules that overlap on an expression are not redundant if they name different faults**
> (wave 320). `(*p)` on a `void *` is legal and `return *p` is not — the same expression, caught
> by the void-*value* rule rather than the deref rule. That is why `Ty::Void` stays outside
> `is_incomplete`: folding it in would have made one rule reject a legal program and left the
> other with nothing to say. **Before widening a predicate, check which of its callers wanted the
> narrow meaning.**
>
> **A construct built in one go carries its rules; one that grew in layers does not** (wave 319).
> The fourth census found `switch` missing all seven of its rules and `_Generic` missing none —
> and `_Generic` was written as a unit with its constraints, while `switch` acquired statement
> handling in one wave and never acquired type rules at all. **When choosing where to census next,
> prefer the construct whose implementation arrived in pieces.**
>
> **Verify the fixture edit landed before believing the mutant that survives it** (wave 318). Wave
> 317 recorded three arms as unfalsifiable; one of them was not, and the difference was an edit
> script that asserted on its third anchor and wrote *nothing* — losing two cases while a separate
> edit landed, so the file looked half-updated and the conclusion looked measured. **A survivor is
> evidence about the test that ran, and the test that ran is not always the test you wrote.**
> `grep` for the case before drawing the conclusion; it costs one command.
>
> **When a mutant survives, ask which check answered first** (wave 317). Three arms could not be
> falsified, and in every case the fixture reached a *different* rule that produced a diagnostic
> of its own — so the assertion passed, by the wrong road. The diagnosing check is not always the
> one under test. **Assert the message, or route the case past the earlier rule**; "it was
> diagnosed" is the weakest evidence a fixture can offer, and wave 315 learned the same thing
> about reaching a guard at all.
>
> **The idiom you are copying decides which cases you write** (wave 316). Every rejected case in
> the pointer-to-const fixture used a *parameter*, because `const T *` is a parameter idiom — it
> is how `memcpy` is spelled — so the declaration path for a *local* went untested and its mutant
> survived, as did the shadowing rule beside it. **When a feature has a canonical spelling, write
> one case that is deliberately not in that spelling**; the examples that come to mind are all
> drawn from the same place.
>
> **Measure the blast radius before choosing the scope, and record the number** (wave 316). The
> qualified-types half of this front was left alone after counting 436 `Ty::` match sites across
> four crates. That is a decision a later reader can check and overturn, which "it seemed too big"
> is not.
>
> **A census cannot enumerate the legal shapes nobody thought to write** (wave 315, confirming
> 314). Two censuses running, the over-rejections were found by the *corpus* rather than by the
> census: a vector initialiser and a function designator in wave 314, `_Bool b = p` and an array
> parameter in wave 315. A census asks what gcc refuses, and gcc refusing nothing is not a list.
> **After adding a check, the full suite is the other half of the census** — and both times it
> failed within one run, so the cost of finding out is small.
>
> **A fixture aimed at a guard must reach the guard** (wave 315). The first three cases written for
> the poison exemption used an incomplete struct, which has been a *record* rather than `Ty::Error`
> since wave 304 — and the check only runs when one side is pointer-like, so they left through the
> arithmetic door before the exemption was consulted. All three produced the expected diagnostic
> count and proved nothing. **"It reported what I expected" is not evidence the code under test
> ran.**
>
> **A fix that arrives as a false-positive repair lands only in the accepted list** (wave 314).
> The vector arm was written because the vector corpus broke, so `v4 v = {1,2,3,4}` went into the
> accepted list and `{1,2,3,4,5}` was never written — leaving the lane bound unfalsifiable. This
> is wave 312's lesson from the other side: there an *exception* was missing from the accepted
> list, here a *rule* was missing from the rejected one. **When a check is added to stop something
> being rejected, write the case that must still be rejected in the same edit.**
>
> **Four waves running, the surviving mutant has been an exemption** (wave 312, confirming 303,
> 308 and 311). The arithmetic is structural, not luck: a RED enumerates the programs a check must
> *reject*, so every rejection is falsifiable the moment the check exists. What a check must
> **accept** is enumerated by nobody unless someone writes it down, and that is where the
> unfalsifiable claims collect. In wave 311 both survivors were legal programs — `return v();`
> from a void function, and a block shadowing a `const` — and both were described as load-bearing
> in prose before anything loaded them. Wave 312 made it four: `break` in a bare `switch` — the
> most ordinary use of `break` there is — was in no fixture, so the counter that exists to allow
> it was unfalsifiable. **Budget mutation effort on the accepted list**, and when a rule has an
> exception, write the *plainest* example of it, not only the tricky one: every `break` case in
> that fixture sat inside a loop because loops are what the rule is about.
>
> **Ask the corpus which inputs it can take; do not curate the list by hand** (wave 310). The gate
> named six headers because six were once tried. Enumerating the directory and running all
> twenty-eight found twenty usable, and the eight failures each had a reason gcc confirms. **A
> hand-written list of inputs records what somebody got round to, not what the code can do** — and
> it never grows on its own.
>
> **Justify every exclusion with the oracle, not with judgement** (wave 310). One header really is
> unclean — `memcpy.h` calls a function it never declares, which `gcc -Wall` reports too — and
> seven are not standalone, which gcc also refuses. Every absence is therefore checkable by
> re-running gcc, rather than resting on a note somebody wrote. **A gate with one permitted
> diagnostic will acquire more**, so the right move was to exclude the header rather than
> whitelist the message.
>
> **Assert the absence, not just the presence** (wave 309). Three waves running produced false
> positives that no test could see, because every test asked "does the engine get the right
> *answer*" and none asked "does it also say something untrue on the way". One assertion over a
> corpus that was already being parsed closed the whole class, and caught a defect on its first
> run. **For every output an engine produces, there should be one test that it stays quiet when
> it should** — and that test is usually far cheaper than the ones checking it speaks correctly.
>
> **When the oracle rejects the input, saying so beats skipping the case** (wave 309). gcc reserves
> `__func__` in its parser, so no differential fixture could pin what a *declared* `__func__`
> means — and that is exactly why the guard for it survived eight mutants. Using chiero as its own
> oracle is right here only because the alternative is no test at all, and the site says so, so a
> reader does not have to wonder why one assertion in a differential file compares against nothing.
>
> **The discriminators find more than the rule does** (wave 308). Both of this wave's deeper
> defects came from the *accepted* list — cases written only to stop a new check being too broad.
> `0[p]`, written because `a[b]` is `*(a + b)`, exposed that lowering assumed the base was the
> aggregate at all three of its `Index` sites; `(void)p;`, written to silence an unused variable
> in a probe, exposed that a cast to void produced no state at all. **Writing down what must keep
> working is a better search than writing down what must break**, because the first list is drawn
> from the language and the second only from what you already suspect.
>
> **A new representation does not announce itself to code that tested the old one** (wave 308).
> The "one diagnostic per bad declaration" exemption tested `matches!(.., Ty::Error)`, correct
> when written and quietly narrower than its comment from wave 304 onward, once an undefined tag
> became an incomplete *record*. It kept compiling and kept passing. **Grep for the old shape when
> a representation changes** — and note that only an assertion on the diagnostic *count* could
> falsify it, since every affected case was still "diagnosed", just twice.
>
> **A diagnostic nothing consumes is a diagnostic nothing tests** (wave 307). Sema's complaint that
> `int a = 0;` in two different functions was a redefinition survived every test in the project,
> because a sema diagnostic does not stop lowering: the corpus compiled those programs, ran them,
> got the right answers, and never read the complaint. **Wherever an output is advisory, the suite
> will drift on it silently** — and the fix is not more fixtures for that one rule but *one* test
> that asserts ordinary correct code produces no diagnostics at all.
>
> **Census the legal half, or the whole class of false positives is invisible** (wave 307,
> confirming wave 303). Thirty programs, half of them valid. Every one of the sixteen *missing*
> checks costs the user a diagnostic they should have had; the single *spurious* check told them
> their correct program was broken. Both waves that ran a legal half found a false positive in the
> existing code, and neither would have found it otherwise.
>
> **A channel must distinguish "agrees" from "could not tell"** (wave 306). The symbolic channel
> has three outcomes, not two, because the engine can take *both* branch edges when the solver
> cannot decide — and a two-outcome channel would have scored that as agreement. It would then
> degrade silently: every case the solver stopped proving would quietly stop testing anything,
> and the suite would report the same green it always had. **When the thing under test may
> abstain, abstention needs its own verdict and its own assertion.**
>
> **A probe that cannot build its own input reports a defect in the engine** (waves 305–306, three
> times now). `<stdarg.h>` with no include loader; undeclared intrinsics; and asking
> `return_value_bits` for a value that is symbolic by construction, where `eval_ground` correctly
> refuses a non-ground term. Each produced a confident "chiero says None" for every case in the
> sweep. **A sweep that fails uniformly is evidence about the sweep, not about the engine** —
> check one case by hand before believing the column.
>
> **Delete the fix that was aimed at the wrong problem, do not leave it in** (wave 305). Two
> diagnostics for one cause looked like duplicates, so `error()` gained exact-duplicate
> suppression. Printing them showed they differ in both span and text — the suppression could
> never have fired. It was removed rather than kept as code that looks like it does something,
> and the real fix (suppress the *consequence* when the cause has spoken) went in instead.
> **When a hypothesis about a symptom turns out wrong, the code written for it is not neutral.**
>
> **Two true sentences about one cause is one report too many** (wave 305). 023 §9 asks for reports
> a person can act on, and "the enumerator is not constant" adds nothing beside "`sizeof` was
> applied to an incomplete type". The rule that keeps this honest is to suppress the *consequence*
> only when something was actually said — measured by the diagnostic count moving — so the
> consequence still speaks when it is the only witness.
>
> **A check is exactly as right as what it asks** (wave 304). Wave 303's pointer-arithmetic rule
> is correct C and it broke a working program, because the fact it consulted — "is this pointee
> incomplete?" — was answered by a representation that called a self-referential struct incomplete
> forever. **Before adding a check, ask what would have to be true for its input to be wrong**;
> a new check over a broken model does not create the bug, it converts a silent wrong answer into
> a loud wrong rejection, which is worse for the user and better for the maintainer.
>
> **The suite's blind spots are shaped like the fixtures nobody wrote** (wave 304). A linked list
> — the first data structure in every C book — produced no answer at all, through 1470 tests, a
> differential corpus, a generated channel and a VPP header gate. It survived because every
> fixture that needed a struct wrote a *flat* one. **When a change touches a construct, ask which
> canonical use of it has no test at all**, rather than which of its rules is untested.
>
> **An exemption is the dangerous half of a rule, and no RED enumerates its failures** (wave 303).
> Five rules went in; mutation killed seven of eight mutants immediately, and the survivor was the
> one that *removes* a check — widening "`extern` with no initializer" to any `extern` passed
> everything else in the fixture. The asymmetry is structural: a check can only be wrong about
> programs it rejects, and those are exactly what the RED lists. An exemption is wrong about
> programs it *accepts*, and nothing lists those unless someone writes them deliberately. **For
> every exemption, write the nearest case that must still be rejected.**
>
> **Look for the false positive before the false negatives** (wave 303). The front was recorded as
> "incomplete types are never rejected", and the probe that mapped it found the engine also
> rejected `extern struct I x;`, which is valid C. Rejecting a correct program is the worse defect
> and it was in the *existing* check, not the missing ones. **When surveying a rule, run the legal
> cases through it too** — a survey of only the illegal ones cannot see this class at all.
>
> **Ask what the fallback value *means* in the domain, not just whether it is wrong** (wave 302).
> `unwrap_or(0)` for a bit-field width is not "a wrong number"; it is a *different legal
> declaration*, handled by the branch immediately below it. That is why it hid four missing
> constraint checks at once. The census that found it sorted candidates by exactly this question,
> and the sites whose fallback is merely wrong (`unwrap_or(4)` for a unit size) turned out to be
> unreachable, while the one whose fallback is meaningful was four defects deep.
>
> **A recovery value needs its own fixture, per arm** (wave 302). Counting diagnostics proves a
> violation is reported; it says nothing about what the engine then does. The claim that the
> fallback keeps the member alive survived ten mutants and died only when the fixture asserted the
> struct's *size*. And the oversized arm needed its own case — a fallback is a per-arm decision,
> so one arm's mutant says nothing about another's, however similar the code reads.
>
> **A fallback that equals the ordinary answer makes its own gap invisible** (wave 301). The
> enumeration walk used `self.eval(e).map(|v| v.v).unwrap_or(next)` — so an initializer the engine
> could not fold produced *exactly* the value an absent initializer would have. No test can catch
> that, because there is nothing to see; the missing `sizeof` arm behind it was found by a
> differential probe aimed at something else entirely. **When choosing a fallback, prefer one that
> cannot be mistaken for success, or announce it.** Announcing is usually right: the fallback
> itself was correct here — an enumeration that stopped resolving would cascade into every use of
> its type — only the silence was wrong.
>
> **Write the claim the mutation supports, not the one that motivated the code** (wave 301). The
> fix's commit message argued that reusing an existing typing prevented a node being built where
> locals are invisible. Mutation said otherwise: forcing the reuse off leaves the whole suite
> green, as does keeping the diagnostics the fix discards. Both were kept — the two behaviours are
> equivalent today and the retained one is cheaper — but the comment now says *equivalent today*
> rather than *required*, and the test written to distinguish them was renamed for what it
> actually pins. **A rationale that mutation cannot support is a hypothesis; label it as one.**
>
> **A defect found in one implementation is a question to ask of every other** (wave 300). The
> usual arithmetic conversions and the conditional operator's type were wrong in `#if` (wave 298)
> and, independently, in `const_eval`. Two teams, two files, same two rules. They share a shape:
> the value is right and only the *type* is wrong, so every test comparing against a positive
> constant passes, in any implementation. **Before hunting a new defect, spend one probe re-asking
> the last one somewhere else** — it cost a single test here and found a third-implementation bug
> that had been reachable from `enum`, array bounds and bit-field widths the whole time.
>
> **A sweep's disagreements are not all defects, and the difference must be checked, not assumed**
> (wave 300). 260 generated constant expressions produced two disagreements with gcc, both signed
> multiplication overflow — undefined behaviour, where chiero already emits a declared diagnostic
> and wraps (`semantics.rs:107`). The right move was to *verify* that the existing behaviour was
> the declared one rather than to file two bugs or to widen the exclusion silently.
>
> **Unfalsifiable-but-reached is a generator's *normal* failure mode, not an accident** (wave 299).
> It happened three times in two waves, and each time the missing ingredient was one prelude entry
> or one extra directive, never more seeds. The pattern: a rule about **how two things combine**
> stays invisible while every generated case supplies only one of them. The rescan rule needs a
> macro whose body *names* another macro — every other macro expands to a complete value, and a
> complete value never notices what follows it. `#elif` exclusivity needs an earlier group that
> was *taken* — every probe opened with `#if 0`, so the guard was never asked. **After adding a
> generator arm, mutate the rule it was added for**; the arm running is not evidence.
>
> **Reaching a rule is not falsifying it, and corpus size does not fix that** (wave 298). The new
> `#if` channel generated `'\101'` freely, so it *reached* the octal escape decoder constantly —
> and mutating that decoder's three-digit bound to four survived 8000 expressions, because a
> fourth digit is never available before the closing quote. Only `'\1011'` can see it. **Mutate
> the code a new generator claims to cover**; a channel that has never been falsified is a channel
> whose power is unmeasured, and more seeds buy none of it.
>
> **When the oracles disagree about whether a program is a program, there is nobody to ask**
> (wave 298). `'\x4142'` is a warning in gcc and a hard error in clang. A differential channel that
> asserts its two references agree first cannot include such a construct — not because it is
> uninteresting, but because a disagreement there says nothing about us. **Removing it is a
> statement of what the channel proves**, and belongs in its docstring next to the other
> exclusions, not in a silent filter.
>
> **A defect can be invisible in every value bit** (wave 298). Both defects the `#if` channel found
> were signedness, with all 64 bits of the value agreeing — a wrong *type* on a right *number*.
> Every existing test missed them because they all compared a `#if` expression against a positive
> constant, where signed and unsigned answer alike. **When a value carries a type, probe the type
> separately**; here that is one extra directive, `#if (E) < 0`, and it is what found both.
>
> **An unfalsifiable guard can be the bug's hiding place** (wave 297). The escape decoder's
> `index >= bytes.len()` bound survived mutation because no test had ever handed it a malformed
> character literal — and the reason none had is that doing so *panicked* two functions earlier.
> The dead guard and the crash were one fact seen from two sides. **When a guard cannot be
> falsified, ask what input would reach it and then actually try that input**; if the attempt
> fails before arriving, the failure is the finding.
>
> **A line splice can silently defuse a fixture** (wave 297). `#if '\` followed by a newline does
> *not* end in a backslash — the backslash is a line continuation, removed before lexing. The test
> passed, and still did not reach the bound it named; only `#if '\` at end of file with no newline
> does. **In preprocessor fixtures, remember the input is transformed before the code under test
> sees it**, and verify by mutation rather than by reading.
>
> **Never run a mutation sweep in the tree you commit from** (wave 297). A sweep edits *tracked
> source* continuously. A `git add -A` landed mid-mutation and committed a preprocessor whose
> `#if` `>=` branch was `if true`, which would have evaluated `>=` for every operator it did not
> recognise. Nothing caught it — the suite had been run before the sweep started, and the commit
> was a documentation commit nobody would think to check for source changes. It surfaced only
> because a later `diff` against the backup disagreed in the direction that meant *HEAD* was
> wrong. **Run sweeps in a `git worktree`**, which makes the mistake unrepresentable, and never
> trust `git status` as a guard against a process that is still writing.
>
> **A backup is not a reference; the last good commit is** (wave 297). When the backup file and
> the working tree disagreed, the first assumption was that the backup had been clobbered. It had
> not — HEAD had. A scratchpad copy has no provenance and a racing script can overwrite it, so
> **diff against `git show <commit>:<path>`**, and read the direction of the diff before deciding
> which side is wrong.
>
> **The `true` direction is uninformative for a condition that consumes input** (wave 297). For
> `if self.take("<<")`, forcing false asks the useful question — does anything notice `<<` going
> unrecognised? Forcing true means the token is never consumed, so the parser spins or recurses
> until it dies. That is a "kill", but it proves only that some test reaches the loop, never that
> any test pins the operator's *meaning*. **Mutate both ways only where both answers are
> semantic**; on the 30 remaining `chiero-pp` sites this dropped 60 mutants to 46 and removed
> almost every timeout.
>
> **A mutation harness must score three outcomes, not two** (wave 297). Grepping for
> `test result: FAILED` answers "did a test fail?", but the interesting third answer is "did the
> runner finish at all?". A mutant that hangs prints nothing and reads as a survivor. Worse, it is
> self-concealing: the same hang that fakes the survival also burns the whole timeout, so the
> sweep looks slow for an unrelated reason and the real cause never gets diagnosed. **Score the
> exit code, not the output**, and treat 124 as a kill.
>
> **A survivor you never acted on is weaker evidence than one you did** (wave 297). A fixture that
> moves a mutant from SURVIVED to killed-by-assertion proves the survival was real, because no
> harness bug can produce that transition. A survivor left on a list proves only that the harness
> printed the word. **When a harness defect is found, the actioned findings survive it and the
> unactioned ones do not** — retract by that line, not by wave number.
>
> **The cheapest way to test a fixture's worth is to delete it** (wave 297). Before crediting the
> `#if` relational fixture with a kill, removing it and re-running the mutant showed the mutant
> died anyway. The fixture is still worth committing as coverage — nothing exercised `>` or `<=`
> in a `#if` — but it kills nothing, and claiming otherwise would have hidden the harness bug that
> was the actual finding. **Ask what the suite does *without* the new test, not just with it.**
>
> **A subsumed *rule* is dead weight; a subsumed *propagation* is a bet** (wave 296). Wave 290
> deleted a verifier check that a neighbouring rule already made — a claim about the input that
> could never be the only thing wrong. This wave kept an unreachable early-out that forwards
> another function's faults, because the day that function grows a fault its callers do not
> pre-check, deleting it would swallow the fault silently. **Ask whether the unreachable code
> asserts something or forwards something.**
>
> **When a fixture misses its target, say so in the fixture** (wave 296). The one written for the
> promotion early-out lands on a state check thirty lines earlier. It is kept — the property it
> *does* pin was untested too — and its docstring now records what it reaches rather than what it
> was aimed at. A test whose comment describes a line it never executes is worse than no comment.
>
> **An unfalsifiable branch is sometimes a duplicated one** (wave 295). The index-width guard the
> sweep could not kill turned out to be a hand-inlined copy of `fit`, which lives in the same file
> and is called from two other places. The right fix was a deletion, not a fixture — and then
> `fit`'s own arms had to be swept, where one *was* uncovered. **Before writing a test for a
> survivor, check whether the code it guards already exists elsewhere.**
>
> **Two arms of one adjustment are not equally likely to arise** (wave 295). `fit`'s widening arm
> is exercised by ordinary fixtures — a narrow index comes from an `unsigned char` subscript. Its
> narrowing arm needs 128-bit arithmetic and had never run. **Symmetric code does not get
> symmetric coverage**, and the rarer side is the one to write down.
>
> **Not every survivor is a gap: check for the equivalent mutant** (wave 295). `Ordering::Equal`
> returning `zext(t, w)` rather than `t` survives because zero-extending a term to its own width
> *is* that term. Recording that costs a sentence and stops the next person hunting a fixture
> that cannot exist.
>
> **Every fixture at the root is a fixture that cannot see a path rule** (wave 294). `"..."`
> versus `<...>` was unfalsifiable in both directions because every existing test put the
> including file at the top level, where its parent directory is `""` and the two searches
> coincide. Moving the includer into `src/` was the entire fix. **When a rule is about
> *location*, at least one fixture has to be somewhere.**
>
> **Two guards that look like the same guard may protect different things** (wave 294).
> `probe_include` falls back when its *directories* are empty; `include()` falls back when its
> *candidates* are. The first fixture I wrote for "no paths configured" killed one and left the
> other alive, and the shape that reaches the second is `#include_next` from the last search
> directory — nothing like the first. **Check which of a pair a fixture actually reached.**
>
> **A choice nothing records is a choice nobody made** (wave 294). Both include fallbacks resolve
> against the working directory where gcc would find nothing, because chiero's default config has
> no system paths. That is defensible and undocumented — pinned now, so the next person changing
> it knows they are changing something.
>
> **Two representations of one fact, and only one of them live** (wave 293). A macro's variadic
> kind is recorded in a public enum *and* in a `std_variadic`/`variadic_name` pair; expansion
> reads only the pair, so the enum could hold anything. The test that looked like it covered this
> — `#define V(...) [__VA_ARGS__]` expanding correctly — consults the other representation
> entirely. **A test can name the right construct, assert the right answer, and still never reach
> the code you think it does.**
>
> **Budget a sweep by rebuild cost, not by suite runtime** (wave 293) — ***superseded by wave
> 297***. `chiero-pp`'s suite runs in 8s, which suggested twenty minutes for 166 mutants; the real
> figure was hours, and this was blamed on rebuilding the test binaries. It was measured wrong:
> an isolated cycle is ~10s end to end, rebuild included. The hours come from **mutants that
> hang** — see the harness rule above. The surviving half of this rule is the procedural half:
> **time one full mutant cycle before sizing the sweep**, run it detached, and log every mutant
> rather than only the survivors (wave 294 read progress off a survivors-only file and misjudged
> it). Machine load matters too: a concurrent workspace build in another checkout tripled the
> per-mutant time, so check `/proc/loadavg` before concluding the sweep itself is slow.
>
> **A both-ways survivor is worth ten one-way survivors** (wave 292). 's sweep left 42
> survivors after escalation — too many to chase — but only **six** where *neither* direction was
> observed. A condition whose truth value nothing notices is doing no work for anybody, and that
> set is small enough to read. Sorting by "survived both" turned an unusable list into a
> six-item one in a single `uniq -c`.
>
> **Mutate a condition both ways, not just off** (wave 291). Forcing a rule to `false` asks "does
> anything need this to fire?"; forcing it to `true` asks "does anything need it *not* to?" Three
> of `chiero-check`'s five survivors were only visible in the `true` direction — they are
> false-positive guards, and a checker's false-positive guards are exactly what no happy-path
> fixture exercises.
>
> **When several guards survive together, look for the shape they share** (wave 291). All three
> `depth == 0` guards in `OrderDependence` were unfalsifiable for one reason: every fixture in the
> file calls **leaf** functions with no nested call and no sequence point. One missing shape, not
> three missing tests — and finding that made three fixtures obvious instead of three puzzles.
>
> **A comment that states a rule is a fixture that has not been written** (wave 291). Each of the
> three guards carried a paragraph explaining precisely what would break without it. Every one of
> those paragraphs was true, unfalsified, and turned directly into a test.
>
> **Sweep a checker's own rules — it is the code most likely to be untested** (wave 290). Thirty-
> three of the CIR verifier's forty rule sites were falsifiable; **seven were not**, and could
> have been deleted silently. They shared a cause: each rejects a *malformed module* that lowering
> never produces, so only a hand-built one reaches them. **Code that only fires on inputs your
> front end cannot emit has no natural test**, and a checker is made almost entirely of it.
>
> **A rule that cannot be the only thing wrong with an input is a rule no fixture can isolate**
> (wave 290). The verifier checked for a duplicate function id while a neighbouring rule required
> `funcs[i].id == FuncId(i)` — so a duplicate always came with an index error, and a
> one-defect-one-kind assertion could never be satisfied. Unkillable, not untested. Deleted.
>
> **A fixture that passes is not evidence until the thing it names can fail** (wave 290). Two
> versions of one fixture passed while the rule under test was *disabled*: a bare `Const::Null`
> has no resolved type so the rule never saw it, and a pointer shift count is caught by a
> different rule with the same error kind. Both were found by disabling the rule and re-running —
> never by reading.
>
> **"Out of reach" is a statement about the corpus, not about the defect** (wave 289). Three
> mutants were recorded as surviving because no channel emitted their shape — a chained `offsetof`
> designator (280), `typeof` of a type name (284), an anonymous member at a nonzero offset (279).
> All three died the moment a channel could afford the shape. **Keep the list of survivors and
> their reasons**; when the constraint that produced a reason goes away, the list says what to do
> next.
>
> **A surviving mutant is a shape request, and shapes are cheap once a channel is cheap** (wave
> 289). The cost of these three was one `match` arm. Before wave 288 it would have been a share of
> a comparison budget that was already spent.
>
> **When coverage costs comparisons, build a channel instead of tuning a rate** (wave 288). Waves
> 284 and 287 each spent a wave tuning — a better fold, restricted element types, four different
> firing rates — and both concluded the same thing from opposite directions. A channel of
> single-construct programs compares **200 of 200** where the shared one manages 100, and kills
> six mutants that survived there. **Two waves of tuning is the signal that the container is
> wrong, not the setting.**
>
> **A new channel needs a floor near the top, not near the bottom** (wave 288). `compared >= 180`
> of 200 is what makes "these programs are dull and therefore comparable" a claim that can fail.
> A floor of 50 would have described the design instead of testing it.
>
> **A shared budget runs out, and the last addition is where you find out** (wave 287). The
> control-flow channel compared 131 programs of 200 before wave 285; the expression wrappers took
> it to 102, `__label__` to 103, the alignment specifier to 100. Every construct since has been
> paid for out of the same 200 seeds. The alignment form needs a firing rate of 4 to discriminate
> and that costs 98 — two below a wave-270 floor — so it shipped at 7, present but not
> discriminating. **When each addition costs the next one's headroom, the answer is a channel
> with its own budget, not a bigger share of this one.**
>
> **Some constructs are only observable in pairs** (wave 287). `_Alignas` changes no value;
> `_Alignof` of a *type* asks nothing about a declaration. Either alone is decoration that
> parses. **Assert the pairing** — the test requires that the `_Alignof` names the object the
> specifier is on — because the version that counts tokens passes on decoration.
>
> **A guard behind other guards is untestable** (wave 286). The backward-`goto` check sat inline
> after half a dozen assertions, every one of which fires first on a corpus that jumps backward —
> the comparison floor, then the jump-over-nothing guard, then the floor again. Nothing could show
> the check itself still worked after I changed it. **Extract a guard you have just weakened and
> test it on hand-written inputs**; four fixtures took minutes and cover the case its own channel
> cannot reach.
>
> **Relaxing a check to admit a construct is a change to a safety property** (wave 286). Reusing
> one label name broke a guard that matched names across the whole body. The obvious repair —
> search forward from the jump — is *worse* in the direction that matters: it finds a later
> block's label and calls a genuinely backward jump forward. **Ask which way the guard is allowed
> to be wrong** before loosening it; here the answer bounded the search to the jump's own block.
>
> **What makes a corpus addition cheap is what it adds, not what it is** (wave 286). `__label__`
> is a *statement* form, so wave 285's expression-wrapper lever did not apply — and it cost one
> comparison, 102 → 103, because it declares no object, computes no value and adds nothing to the
> checksum or the UB surface. **The cost follows the storage and the arithmetic**, not the
> grammatical category.
>
> **Add nothing to the stream: wrap what the grammar already built** (wave 285). Three shapes
> failed before this one. Gating on a private stream *before* an existing gate skips that gate's
> draw; taking only the turns it declines still calls `expr` for a fresh operand. Both shift every
> draw after them, and wave 270's `!`-of-a-negative-zero — three programs in six hundred — went to
> zero and then one. **Lowering the rate did not help and was not monotonic**: a displaced stream
> scrambles *which* programs contain a rare shape, so a shape with no margin is lost at any rate.
> Wrapping the finished expression adds and skips nothing.
>
> **A corpus form should select, not combine** (wave 285). Adding a builtin's value to the operand
> is itself undefined once a narrow signed type is near its range: discards went 69 → 100 and the
> channel fell to exactly its floor. `cond ? operand : 0` keeps both in play and adds no
> arithmetic the program did not already have.
>
> **Merge a costly corpus addition only if it buys kills** (wave 285). This one costs 131
> comparisons → 102 and buys four mutant kills across three waves' features, so it is in. Wave
> 284's multi-dimensional array cost more and bought none, so it is out. **The comparison count
> is the price and the mutation sweep is the receipt** — quote both.
>
> **A separate stream is not enough; do not change state that *gates* another arm's draw**
> (wave 284). `leaf` reads `if usable.is_empty() || self.rng.chance(3)`, so pushing one more
> variable makes the left operand false and the `chance(3)` draw *happen* where `||` had
> short-circuited it — every downstream draw moves. Wave 277's lesson was that a new arm needs
> its own stream; this is the same lesson one level down. **New names go into a sink the grammar
> never reads.**
>
> **A new arm must not consume the statement slot either** (wave 284). The statement budget is
> fixed, so a slot spent on a declaration is a slot not spent on a `switch` or a `continue`:
> wave 270's `!`-of-a-negative-zero fell to one program, then `for`-`continue` to nine. Emit and
> **fall through**.
>
> **The checksum's own arithmetic can be the undefined thing** (wave 284). Folding extra values
> the way the existing loops do — `acc = acc * 31 + …` on a `long`, per element — cut the channel
> from 131 comparisons to 40, because signed overflow is undefined and an undefined program is
> *discarded* rather than compared. The existing folds carry the same hazard and get away with it
> by being fewer.
>
> **Decline the addition when it costs a guarantee** (wave 284). A multi-dimensional array is the
> shape that hid wave 278's worst defect and it costs the channel a third of its comparisons, at
> any rate, with the fold rewritten twice to rule that out. Merging it would have bought coverage
> of one defect class by quietly weakening the floor wave 270 put on `compared`. **Record the
> measurement and leave it out** — a corpus addition that lowers the corpus's yield is not an
> addition.
>
> **The control fixture is a probe too** (wave 283). `__int128 x = 1; x = x << 70;` was written as
> a *control* in the `TypeKind` census — an ordinary type form that was supposed to already work —
> and it **panicked the engine**. The eleven real failures around it hid the crash until each was
> probed on its own. **When a batch fails, separate the rows before reading the result**; a
> control that fails is the most interesting row in the table.
>
> **The arithmetic that computes a boundary has the same boundary problem** (wave 283).
> `(1i128 << (w - 1)) - 1` is the maximum of a `w`-bit signed value and underflows at `w = 128`,
> where the shift *is* `i128::MIN`. Every narrower width leaves headroom, so all of them were
> fine and only the widest was not. **A range check written in the widest type cannot check the
> widest type.**
>
> **A crash fixture proves the crash is gone, not that the answer is right** (wave 283). Seventeen
> differential fixtures for the panic all passed with the range mutated to `(0, 0)`, because a
> spurious or missing UB report does not change a program's *value* and the value oracle is all
> they have. The range needed asserting where a **report** is the observable. **Match the oracle
> to the property**: value oracles cannot see diagnostics.
>
> **Making one thing work exposes what was deferring to it** (wave 283). `sizeof(T)` never
> resolved `T` — it interned `size_t` and left the answer to `const_eval`, whose throwaway context
> sees only file-scope declarations. Nothing noticed until `sizeof(__typeof__(x))` named a local.
>
> **A specifier is attached where the *parser* put it, not where the feature lives** (wave 282).
> `_Alignas(32) int a[4]` puts the attribute on the `int` node while the declaration's type is the
> array wrapper `declarator_suffixes` built around it, so reading the outermost node found
> nothing. **Before reading an attribute off a type node, ask which node the declarator left it
> on** — and whether a typedef can be carrying it instead, which is a second place entirely.
>
> **A fixture that checks an address cannot see an alignment the runtime already gives you** (wave
> 282). The file-scope mutant survived because the engine places globals generously: `&g & 63` is
> 0 whether or not the request was honoured. `_Alignof(g)` reads the number the object actually
> carries. **When testing that a request was recorded, read the record, not a consequence that
> something else also produces.**
>
> **Two spellings of one feature can disagree in one direction only** (wave 282). `_Alignas` and
> `__attribute__((aligned))` are identical for raising an alignment — the parser rewrites the
> first into the second — and differ for lowering it: gcc *rejects* `_Alignas(1) int x` and
> accepts `aligned(1)`, which really does reduce. A RED asserted the wrong thing there until gcc
> refused to compile it. **Check both spellings at the boundary, not only in the middle.**
>
> **A clean census is a result, and it hands the wave back** (wave 281). The preprocessor was §9's
> last unrun axis: forty-four probes found **no silent wrong answer**, and all three gaps are
> declared with their own diagnostics and in 0 VPP files. That is worth recording rather than
> padding into a change — and it sent the wave to the last known defect, where probing found a
> different, worse one underneath.
>
> **Probe the reported defect before believing its description** (wave 281). §9 said "`_Alignas`
> is ignored on a variable", which is what the numbers looked like. `_Alignas(16) int x` reported
> **4 because 4 is `sizeof(int)`** — the parser recorded `_Alignof(expr)` as a `SizeofExpr`, so
> `_Alignof` computed a size for every expression, specifier or not. The described defect was a
> symptom of a bigger one that needs no specifier at all.
>
> **Measure the spelling the target uses, not the one the note names** (wave 281). `_Alignas` is
> in 0 VPP files; `__attribute__((aligned(N)))` — same path, same defect — is in **16 directly and
> `aligned(` in 266**, because VPP's cache-line macros expand to it. §9 had recorded the item
> under the spelling with no reach, understating it by two orders of magnitude.
>
> **`X - 1 > 0` cannot see signedness** (wave 281). The mutant typing a `size_t` result as signed
> survived it: four minus one is positive either way. It takes a subtraction that *wraps* —
> `_Alignof(a) - 5`. **An unsigned-ness fixture has to underflow.**
>
> **An early `return` from a function whose tail does bookkeeping skips the bookkeeping** (wave
> 280). `type_expr` ends with `set_top`, which every arm reaches by falling out of the match. A new
> arm that `return`ed pushed its node and never registered it, so `type_of` could not see it.
> Fifteen fixtures passed and **one** failed — `sizeof(x)`, the only shape that reads the
> operand's type from exactly there. **When adding an early return, read what the function does
> after the match.**
>
> **The same lookup in two arms needs a fixture that reaches the second one** (wave 280). The
> designator walk resolves a field at the root and again at each `.` step. Every fixture put the
> anonymous member at the root, so a mutant that made the chain step scan directly survived. It
> takes a designator that walks into a named struct *and then* through an anonymous one. Third
> wave running that the fixtures all landed on the same side of a two-sided rule — the square
> array (278), the zero offset (279), and now the root step.
>
> **A construct can be missing a *reader*, not a grammar** (wave 280). `__builtin_offsetof`'s
> member designator already parsed into exactly the right tree — `n.y` a `Member`, `v[2]` an
> `Index` — and the defect was sema typing that tree as an expression. **Before adding syntax,
> check what the parser already produces for it.**
>
> **The zero case is the symmetric case wearing a different hat** (wave 279). Two mutants — the
> offset rebase and the bit-offset rebase — survived fifteen fixtures because *every* one declared
> the anonymous member first, and rebasing onto offset 0 adds nothing. Wave 278's square array,
> one wave later, in a new costume. **When a fix adds an adjustment, make sure a fixture has
> something to adjust**; the natural way to write the example puts the interesting member first,
> which is exactly where the adjustment vanishes.
>
> **Severity ranks kinds of failure; the target ranks features** (wave 279). `_Alignas` is a wrong
> answer and anonymous members were only a refusal, so severity said take `_Alignas`. Two greps
> said anonymous members are in **34 VPP files including `vnet/buffer.h`** and `_Alignas` is in
> three. Take the one that unblocks the target. **Use severity to order defects of comparable
> reach, not to override reach.**
>
> **Re-probe an open item before working it** (wave 279). Of wave 278's four declarator defects,
> one — pointer-to-array indexing — was already fixed: it had been a *symptom* of the dimension
> reversal, not a defect of its own. Three minutes of probing struck an item off the list.
>
> **The symmetric case is its own reverse, and hides the bug** (wave 278). `int a[2][3]` was typed
> `int a[3][2]` for the whole life of the project. Every two-dimensional fixture in the suite used
> a **square** array, which is identical under the reversal; `sizeof(a)` is 2·3·4 either way; and
> `a[1][0]` reads the same element under both layouts. One corner read of four agreed by accident.
> **When testing a shape with two parameters, make them differ** — and prefer the test that shows
> the *type* (`sizeof(a[0])`) over the one that shows a *value*.
>
> **Fixing a type exposes the code that was never asked the right question** (wave 278). With the
> dimensions corrected, `init_list` met array-typed slots and its scalar store rejected them: C11
> 6.7.9p20 brace elision had never been implemented. The hole predated the fix — the reversed type
> also had array slots — but no fixture wrote a flat initializer for a nested array, so nothing
> rejected it. **A GREEN that uncovers a second RED is the normal case for a type-level fix.**
>
> **Census the shape of a declaration, not only of an expression** (wave 278). The expression,
> statement, IR and keyword axes had all been run; the *declarator* grammar never had. Twenty-seven
> shapes against gcc found five wrong answers in one pass — the worst defect found in twenty waves
> was sitting behind the axis nobody had asked about.
>
> **Ship a construct, then check the corpus can reach it — in the same wave** (wave 277). Waves
> 272–274 built all of `vector_size` and the generator emitted **zero** vectors, so three waves of
> surface stayed graded by whatever a person thought to spell. Wave 270's rule cuts both ways:
> "adding a construct to the corpus buys nothing until the context can discriminate" has a
> converse, and the converse is the more expensive one to miss. Five mutants across the three
> waves now die to the corpus alone.
>
> **A new generator arm on the shared RNG stream silently re-rolls every arm after it** (wave
> 277). Drawing from `rng` like the other extended arms dropped wave 270's
> `!`-of-a-negative-zero shape from three programs to **zero** — caught by the channel's own
> adequacy guard, which is exactly why those guards are counts and not booleans. Wave 217 gated
> new arms *before* any `rng` call to protect the other channels; **within** one channel the same
> protection needs a **separate stream**. Swap the streams around a shared helper rather than
> duplicating its pool.
>
> **A test that measures the wrong thing reads exactly like the feature being absent** (wave 277).
> The presence test counted zero lane reads because it collected only *braced-initialized*
> vectors while the read is of the **result** vector — indistinguishable from a generator emitting
> no reads at all. It also keyed on a `_v0` naming convention `fresh()` does not use. **Before
> believing a zero, check what the counter counts.**
>
> **A census axis is worth running even when it returns one row** (wave 276). All 59 `Kw::`
> variants against the parser's productions left exactly one unconsumed: `Kw::Label`. One row —
> and it was `__label__`, which `vppinfra/hash.h` uses and `hash_foreach_pair` is built on, while
> `_Generic` (wave 275, the same axis) appears in **no** VPP file. **The census found the more
> important gap second.** Ordering by how obscure a construct *sounds* would have got both wrong.
>
> **Ask which pass owns the problem before choosing where to fix it** (wave 276). `__label__` is
> a *naming* construct, so renaming in the parser left the AST, sema and lowering untouched. The
> alternative — teaching lowering's per-function label map about block scopes — would have
> touched three crates to express the same thing. **A construct that only affects which name a
> reference resolves to belongs where names are resolved.**
>
> **Check the target codebase, not intuition, for what matters** (wave 276). Two greps over
> `/home/ubuntu/vpp` settled the priority of two features in ten seconds, and reversed the order
> that reading the C standard would have suggested.
>
> **A test that aborts looks like a test that passed** (wave 275). A mutant that replaced the
> lowered arm with the `_Generic` node itself read as a *survivor*: it stack-overflows, and a
> crashed test binary prints no `failures:` section for a grep to find. Wave 254's rule met from
> a third direction — check the runner's exit status, not only its output.
>
> **Some behaviour can only be told apart by a program the reference rejects** (wave 275).
> "First matching association wins" and "last wins" are the same function for every *valid*
> `_Generic`, because C11 6.5.1.1p2 forbids two matches. The mutant survived the whole
> differential suite and always would have. **When a survivor is only distinguishable by an
> invalid program, the question is not which behaviour to pick — it is whether the program is
> reported at all.** Two constraint violations became diagnostics, with a sema fixture and a
> well-formed control, and the remaining choice is documented as untestable rather than implied
> to be a rule.
>
> **A construct that is unevaluated has to be unevaluated everywhere, not just in lowering**
> (wave 275). `_Generic` does not evaluate its controlling expression or its losing arms. Getting
> that right in `expr` was free — lowering emits only the recorded arm — and `OrderScan`, the
> unsequenced-access checker, would still have scanned all of them. **Ask which other passes walk
> the AST**; the one that does not know about a new node reports conflicts between expressions
> that never both run.
>
> **Four survivors, four different kinds — sort them before fixing any** (wave 274). One sweep
> left a missing fixture for an operator pair the tests never used (`>`/`>=` on floats), a missing
> fixture for a property no test could observe (the mask element's signedness, hidden because
> every fixture assigned the result to a declared type first), a redundant guard that could not
> change an answer, and a pair of expressions that are provably equal. **Only two wanted a test;
> one wanted deleting and one wanted a comment.** Treating a survivor list as a to-write-tests
> list produces two tests that assert nothing.
>
> **A value assigned to a declared type is read at that type, not its own** (wave 274). Every
> comparison fixture wrote `v4si e = (x == y);` and then read `e`, so the *comparison's* element
> type was never consulted and its signedness was unobservable. `(x == y)[0] < 0` reads the lane
> through the expression's own type. **When testing a property of an expression's type, do not let
> a declaration stand between the expression and the assertion.**
>
> **Pin the reference by running it, not by recalling it** (wave 274). gcc's vector comparison
> rules — same total size, lane width preserved, element becomes *signed* whatever the operand's
> signedness, true is all-bits-set, unsigned lanes compare unsigned, NaN follows the
> ordered/unordered split — were established with one twenty-line program before a line of the
> RED was written. Four of them would have been guessed wrong.
>
> **A well-typed instruction can mean the wrong thing, and the verifier will pass it** (wave 273).
> `x += y` on a vector stored a `CTy::Ptr` value into a `CTy::Ptr` slot. Types agree, verifier
> happy, one lane written and three left stale — a wrong answer where every neighbouring bug in
> the same wave was a loud refusal. **The verifier catches contradictions, not mistakes**, so the
> shapes it cannot see are exactly where a differential oracle earns its keep.
>
> **When two spellings of one operation disagree, the difference is the diagnosis** (wave 273).
> `x << 1` lowered correctly and `x + 1` did not, on the same operand. Shifts return from
> `type_binary` before it ever asks for a common type — so the bug was the common type, and the
> asymmetry named it in one step. Same shape as wave 270's `d && 1` versus `1 && d`.
>
> **The same mistake in a function that already documents it twice** (wave 273). `type_binary`
> and `ExprKind::Assign` each carry a comment explaining why a *pointer* operand must not be
> coerced to the lvalue's type, and `Assign` carries a second for `_Bool`. A vector is the third,
> and it broke the same way. **When a fix's comment could have been written by copying a comment
> already there, check whether the list has more members** — the tell is an lvalue whose type the
> other operand must not be dragged to.
>
> **A surviving mutant is usually a missing fixture, and the fixture is usually specific** (wave
> 273). Two of ten survived. The lvalue was evaluated twice and every fixture had a bare
> identifier on the left, where that cannot be seen — only `a[i++] += y` can. The scalar's
> conversion to the element type was unobservable because every broadcast fixture used a scalar
> already at the lane's type — only `10 - f` on a `v4sf` needs a real `SiToFp`. **Both were real
> defects the suite would have shipped**, and neither was found by reading.
>
> **A half-supported type is worse than a missing one** (wave 272). `sizeof(v4si)` was 16,
> `sizeof x` was 16, and `x[0] = 7; return x[0]` gave 7 — every smoke test a person would write.
> Meanwhile `v4si x = {1,2,3,4}` dropped its initializer entirely and every lane read zero. **The
> parts that work are what stop anyone looking at the parts that do not.**
>
> **One defect can make another invisible, so re-probe after every fix** (wave 272). The dropped
> initializer meant every lane read zero, which hid a second, independent defect: a vector
> subscript typed `Ty::Error`, so lanes were read as `Int(32)`. Fixing the first turned
> `chiero says 0` into `chiero says 1075838976` — the bit pattern of `2.5f`. The RED named one
> defect and there were two.
>
> **Accidental agreement is the normal case, not the exception** (wave 272). For that second
> defect: `int` lanes were correct by accident, `long` lanes read their low four bytes so `{7,8}`
> gives 8 either way, and a store-then-load of one lane round-trips because the value converts on
> the way in. Separating them needs arithmetic *on the loaded lane*, a value above 2^32, or a
> `char` lane. **When a fixture agrees, ask which of the two answers it would have distinguished.**
>
> **A gcc rejection is not a chiero disagreement** (wave 272). Three probe rows read as defects and
> were gcc refusing to compile the fixture: `{[2] = 5}` on a vector is *array index in non-array
> initializer*, and `&g[2]` at file scope is *initializer element is not constant*. Both went into
> a RED before the harness caught them. **Read the failure text, not the failure count** — wave
> 270's rule arriving from the other side.
>
> **Symmetry is not a reason** (wave 272). A vector arm in sema's constant-address walk mirrors the
> one in its `Index` typing exactly, and is unreachable: C forbids a vector subscript in a constant
> initializer. Adding it changed no answer either way, so it was reverted and the reason left in
> its place. Wave 271's rule, applied to a change that *looked* obviously right.
>
> **Census the IR, not just the AST — a dead opcode is the fingerprint of a missing feature**
> (wave 271). Wave 270 asked what `ExprKind` can hold against what the generator emits. Asking the
> same of **CIR** found that lowering can emit only twelve of `CmpOp`'s twenty variants, and the
> eight it cannot are not junk: `FUno`, `FONe` and the ordered relationals are exactly the
> distinctions C draws in its 7.12.14 *macros* and not in its *operators*. `chiero-exec` had
> implemented all twenty. **Six comparison semantics were sitting in the engine with no producer,
> waiting for a feature nobody had noticed was absent.** Four are still unproduced: `FUEq`, `FULt`,
> `FULe`, `FOrd`.
>
> **A refusal that is entirely correct still marks a missing capability** (wave 271). This is the
> other half of wave 270's rule. 015 §7 refused every `__builtin_isnan` loudly and by name, which
> is the honest outcome and is precisely why no oracle ever flagged it — the differential channel
> cannot grade what the engine declines to run. **Loud refusals are invisible to the oracle, so
> only a census finds them.**
>
> **Probe before extending the grammar, and be willing to not extend it** (wave 271). Statement
> expressions were wave 270's other absent form. Twenty-four hard shapes agree with gcc, two engine
> mutants confirm the probes reach the engine, and mutating the lowering arm left nothing a
> generated statement expression could kill — so they were **not** added. Coverage that kills no
> mutant is the thing wave 270 warned against, and declining to add it is the rule working.
>
> **Some survivors are the same function twice** (wave 271). `isnan(x)` as `FUNe(x, x)` survives
> every fixture, and no fixture can kill it: unordered-not-equal on two copies of one value is true
> exactly when they are unordered. Likewise a `last_stmt_value` restore that is reachable but
> provably overwritten before any reader. **Record the measurement in the code.** The alternative is
> the next reader spending a wave writing a fixture that cannot exist.
>
> **Mutation justifies the parts of a fix you would not have questioned** (wave 271). Typing the
> builtins' result `int` in sema looked obviously necessary and survived all 34 fixtures — an
> `Error` type still lowers to a 32-bit value and `isnan(a) + 2` comes out right. Only `sizeof`,
> which reads the type directly, and mixing with a `double`, which needs the usual arithmetic
> conversions, discriminate. **A change nothing can observe is a change you cannot claim.**
>
> **A refusal is a defect that costs nothing to hide behind** (wave 270). The control-flow batch
> lumped `Verdict::Refused` in with `Discarded`, so lowering that produced *nothing at all* read
> exactly like a program the oracle declined to compare. Every `x && <float>` in the corpus was
> refused — the verifier rejected the CIR and the function was dropped whole — and the channel
> counted it as ordinary for as long as the arm has existed. **A wrong answer is caught on the
> first seed; producing no answer is free.** Grade `Refused` on its own with a bound of zero, count
> `Gap` separately (023 §7: a declared limit is honest, an undeclared refusal never is), and floor
> `compared` so the channel cannot pass on nothing.
>
> **Generating the shape is not generating the value** (wave 270, three times in one wave). The `!`
> arm first emitted `x && !x` — and `&&` short-circuits away the right operand for exactly the zero
> that discriminates, so the shape was built such that the one case it existed to reach could not
> be. Corrected to an unconditional `!`, **2000 seeds put it on a float 41 times and the mutant
> survived all 41**: the operand was a sum of small constants, which lands on a *negative* zero
> essentially never. Then `-0.0` turned out not to be in the float pool at all. A census that counts
> occurrences of a construct answers none of this. **The test is a surviving mutant, and only that.**
>
> **Ask what the AST can hold, then probe before you extend the grammar** (wave 270). Censusing
> `ExprKind`'s twenty-one variants against what the generator emits found three absent from both
> channels: `_Alignof`, a statement expression, and `!`. Ten hand probes each — a minute's work —
> found `_Alignof` and statement expressions already correct and `!` **wrong on the first one**.
> That is wave 217's technique finding two real defects *before the generator was touched*, and
> waves 250–253's lesson applied in the other direction: adding a construct to the corpus buys
> nothing until you know the context can discriminate.
>
> **One decision in three places is one right answer and two wrong ones** (wave 270). "Is this
> scalar zero" was decided by `truth_of` (correct: `FUNe` for a float), by `!` (a hardcoded integer
> `Eq`, so `!(-0.0)` tested the *bits*) and by the **rhs** of `&&` (a hardcoded `Ne` at the type of
> the *result*, which has nothing to do with the operand's). The lhs of the very same short circuit
> called `truth_of`. **When two sides of one construct are asymmetric, the asymmetry is the bug** —
> and the fix is the shared function, not a third copy that happens to be right.
>
> **When a report cites something, test *which* thing it cites** (wave 269). Two fixtures asked
> whether the clause appears; three mutants survived them, and all three were about what it points
> at — any comparison counting as a null test, one operand order, one parameter's check attributed
> to another. **A finding that names a wrong line is worse than one that names none**, because a
> reader who checks it learns not to trust the next finding either.
>
> **One C construct can have two CIR spellings, and matching one silently misses the other** (wave
> 269). `if (p)` leaves a bare `null` operand; `if (0 == p)` materialises `inttoptr i32 0i32 to ptr`,
> because C11 6.3.2.3p3 makes the null pointer constant an explicit conversion. Both are the same
> thing to the person reading the report. **Dump the CIR for each spelling before writing a matcher**
> — the second one cost a red fixture to discover.
>
> **Reaching a branch is not observing it** (wave 268). Two waves went into getting an object into
> the state a havoc branch needs; the branch then ran, and the mutants survived anyway — because two
> different fault kinds lead to the same outcome, and no read can tell them apart. **A mutant can
> survive at a reachable, executing site**, and the next question is not "what fixture" but "what
> could possibly differ downstream".
>
> **A control that fails may be asserting the wrong thing** (wave 268). The control here asserted
> silence after promotion and went red; a concrete read of a promoted object reports `SymbolicByte`,
> which is 021 contract 6 *holding* — the value and its initialization status survive, the
> representation does not. **Check whether a failing control is a defect or a wrong expectation
> before treating the code as broken.**
>
> **Survivors that look alike can differ in kind, and one measurement separates them** (wave 266).
> Six entries in `unusable`'s list survived removal and read identically in the table. Instrumenting
> what actually reaches the site showed five of them can never change an answer — the faults arrive
> with no value to discard — and one was a real gap. **Before writing six fixtures, ask what the
> site receives**; the mutation table says a mutant was not observed, never why.
>
> **A sweep's verdict is only as good as the test set it runs** (wave 265). `null-not-reported`
> survived until four files covering null dereferences were added to the sweep — none of which I had
> included. **"SURVIVED" from an incomplete set reads exactly like a coverage gap**, and the fix is
> to grep for the fault's name across `tests/` before believing a survivor. §9 warned about this and
> I still did it.
>
> **A surviving mutant on a `pub fn` may mean nothing calls it** (wave 264). Three mutants against
> `in_bounds` survived every fixture, and the cause was not a missing test — the function had no
> production caller anywhere in the workspace. **Before writing a fixture for a stubborn predicate,
> grep for its callers**; testing it would have pinned a function no answer depends on, and a passing
> test implies something is watched.
>
> **Dead code hides dead code** (wave 264). `size_of` did not look dead while `in_bounds` still
> called it, and `in_bounds` did not look dead until someone grepped. One deletion exposed the next
> immediately — clippy caught it in the same run — so treat a removal as the first step rather than
> the whole of it.
>
> **A clause with a two-part condition has four cells, and half of them demand silence** (wave 263).
> The `INT_MIN / -1` check fires on one pair of operands. Dropping *either* half of the condition —
> so any `INT_MIN / x` reports, or any `x / -1` does — survived every fixture, because none asked
> chiero to stay *quiet* about those. **Reading a fixture list as a grid finds the missing reports;
> the missing non-reports need asking for separately.**
>
> **Reading a grid's *operator* axis found a real defect where its value axes found only gaps**
> (wave 263). Three waves of this technique produced three coverage holes and one genuine missed
> event — and the missed event was on the axis that lists what the clause *matches*, not what the
> operands hold. `Add`, `Sub`, `Mul` were there; `SDiv` was not, and C gives signed overflow four
> ways.
>
> **A sweep's control detects its own previous run's wreckage** (wave 262). A mutation sweep that
> exceeds its timeout is killed between applying a mutation and restoring the file. The next sweep
> then measures everything against a mutated baseline and the results look ordinary — three plausible
> kills and one plausible survivor. `CONTROL KILLED` is impossible unless the baseline is wrong, and
> that is the only reason it was caught. **Assume a timed-out sweep left the tree dirty, and never
> run one without a control.**
>
> **A thin row is a question, not a diagnosis** (wave 261). `FloatCastOverflow` at 7 sites looked
> like "not enough of this shape", and §9 recommended making more. Mutating the code the row is
> supposed to grade showed three of five checks were already covered and named the two that were
> not — a negative float into a signed destination, and a NaN. **Neither has anything to do with
> frequency**, so the recommended fix would have added sites and closed nothing. **Ask what a
> coverage number can observe before treating its size as the problem.**
>
> **Look for the mirror of a fixture you already have** (wave 261). The two gaps were the mirrors of
> two cases already written down: "too large for signed" existed and "too negative for signed" did
> not; "negative into unsigned" existed and "NaN into anything" did not. A fixture list is worth
> reading as a grid, and the empty cells are cheap to find and cheap to fill.
>
> **"Cannot be asserted to zero" is not "cannot be asserted"** (wave 260). `extra` counts sites
> chiero reports and gcc does not, and it genuinely cannot be required to be zero — gcc elides
> arithmetic used only as a condition, so its silence is not evidence. That correct observation was
> carried as a reason to assert *nothing*, and a mutant lived there for seventy-five waves. A
> ceiling caught it: the mutant moves the number from 1 to 12. **When a quantity resists an exact
> assertion, try a bound before concluding it is unassertable.**
>
> **Measure the data the code operates on, not just whether the code ran** (wave 259). Four fixture
> attempts failed to kill a mutant on store-chain ordering. Logging the chains answered it in one
> run: every chain carries a single distinct value, so ordering cannot matter and the mutant is
> equivalent. **"Which input would reach this?" is a slower question than "what do the inputs that
> reach it look like?"** — and the second one also tells you when the answer is "none can".
>
> **Shrinking a fixture can move it out of the path you are testing** (wave 259). §9 suggested an
> eight-byte object to keep the init chain inside `EXPAND_LIMIT`; at eight bytes the offset
> enumerates and `init_guard` is never called at all. **A smaller input is a different input**, and
> whether it still exercises the path has to be checked rather than assumed.
>
> **A note that records a gap does not learn when the gap closes** (wave 258). Three of the five
> surviving mutants §9 has carried since wave 205 were killed by a test written in wave 204's
> aftermath. The file said "`Unknown` is still untested" three sections above the file that tests it.
> **Re-running a recorded mutant list is cheap and should happen before the work it recommends** —
> this one took twenty minutes and removed three items.
>
> **"Survives" is not always "is a bug"** (wave 258). `expand-unbounded` removes a limit and produces
> a *better* guard at more cost; nothing observes the precision the limit gives up. A surviving
> mutant on a performance bound is a fact about the bound. **Classify a survivor before scheduling
> work for it**: missing fixture, real defect, dead line, unfalsifiable assertion, or — this — a
> trade-off with no wrong side.
>
> **When a decision cannot change an answer, assert the artifact instead of the answer** (wave 257).
> `widen_to_64`'s signedness is real and reachable and provably cannot alter any value a legal
> program produces — the discriminating index is always out of bounds. No differential fixture can
> ever see it; a `shapes.rs` assertion on the emitted `sext`/`zext` sees it immediately. **A
> surviving mutant is not always a missing fixture; sometimes it is the wrong oracle.**
>
> **Ask "should this site exist" before "what fixture would reach it"** (waves 256–257). Four
> extension decisions were unobservable. Three were duplicates and went away; one needed a different
> oracle. **Zero needed a better fixture** — and wave 255 spent itself writing them.
>
> **Deleting an untestable site is a legitimate way to close a coverage gap** (wave 256). Wave 255
> spent itself trying to build a fixture that could observe `convert_for_store`'s inline
> `SiToFp`/`UiToFp` arm and failed every time. The arm was a *duplicate* of a decision `cast_kind`
> already made, and the same mutation dies there. **Before writing another fixture for a stubborn
> site, ask whether the site should exist** — a second place computing one decision is both a drift
> risk and, usually, the one nothing watches.
>
> **A dedup can be justified by a mutation table rather than a failing test** (wave 256). Nothing was
> broken and the two copies agreed. What changed is that the surviving decision is observable and the
> deleted one was not, which is a result worth stating in those terms rather than as "tidying".
>
> **Instrumentation that logs nothing needs a control every single time** (wave 255). Wave 254
> concluded a conversion site was unreachable from an empty log. The rule was already in this list —
> earned in wave 222 and again in 247 — and it was applied in the same wave to a *different* scan
> while this one went unchecked. **"I know this rule" is not the same as "I ran the control", and the
> control costs one line.**
>
> **Instrument the value, not only the reach** (wave 255). Three fixtures reached the int-to-float
> arm and none could discriminate, because all three carried non-negative sources. Printing the
> *kind chosen* took one run and answered in seconds what four fixture attempts had not. When a
> mutant survives at a site you know is reached, ask what value arrives, not which spelling to try
> next.
>
> **The blind-half error has now recurred three waves running** (250, 254, 255), each time after the
> rule was written down. Remembering it does not work; measuring does. **Treat "pick a
> discriminating value" as a step that must be *verified*, like any other.**
>
> **An audit that finds nothing still has to prove it looked** (wave 254). Twelve fixtures agreeing
> with gcc says the code is right *if* the fixtures can see the decisions they are named after.
> Mutation said they see one of four. **The audit's result is "no defect at one site, and no
> evidence at three"** — which is a different sentence from the one twelve green fixtures suggest,
> and the only honest one.
>
> **The blind-half error recurs in your own fixtures, not just the generator's** (wave 254). I wrote
> `unsigned char i = 2` as an index to test sign- versus zero-extension, three waves after
> establishing that a value with its top bit clear cannot tell them apart. Knowing the rule did not
> stop me applying it; the mutant did.
>
> **A fixture that exceeds the engine's budget tests nothing and looks like it passed** (wave 254).
> Seeding a 256-element array with a loop made chiero return no value, so `agree` had nothing to
> compare — caught only because the *control* mutant also failed, which is the signal that a test is
> inert rather than passing.
>
> **A construct's *context* is part of its coverage, not a detail of it** (wave 253). The generator
> emitted bit-fields, gave them discriminating values, routed them into scope — and read every one
> through `(long)(x.f)`, which is exactly the context where the defect cannot be seen. **Ask what the
> read looks like, not only whether the construct appears.** Three waves of frequency work missed it
> because frequency was never the variable.
>
> **A count tells you a classifier ran, never that it was right** (wave 253). Inverting the scan's
> bare/cast arms survived every threshold, because success had made both counts large; before the
> fix one of them was zero and the error would have shown. The kill needed an assertion on the
> *direction* — bare must outnumber cast, since only floats keep one — plus the classifier asserted
> on literals. **A metric gets harder to test as the thing it measures gets better.**
>
> **A proxy metric earns its keep by predicting the goal, and must be checked against it** (wave
> 252). The discriminating-shape count went from 1 to 5 and the defect stayed invisible. The proxy
> was not wrong about what it measured — the shape really is five times commoner — it was wrong that
> the shape implies reachability. **Run the goal experiment at every step, not at the end**: one
> `judge()` call on a single qualifying seed said more than three rate improvements.
>
> **A counterexample beats a corrected model** (wave 252). Seed 49 satisfies all four conditions and
> agrees anyway. That single program is what turned "the rate is too low" into "the defect is
> context-dependent, and the generator only ever writes the context where it hides" — a question
> small enough to answer, from a number that was going nowhere.
>
> **Lowering a threshold to meet the code is usually wrong, and saying why is the price of doing it**
> (wave 252). This wave's RED asked for ten and settled at five. That is defensible only because the
> controlled experiment shows the number was never the goal, and the test now says so where a reader
> will see it rather than in a commit message they will not.
>
> **A broken measurement confirms whatever you expected** (wave 251). The scan written to test §9's
> routing hypothesis reported zero for every count — which reads exactly like the hypothesis being
> right. It was reading the prelude; a generated program is two strings and the body is the second.
> **What caught it was counting things the scan obviously should find** — any struct local, any
> checksummed field — and seeing those come back zero too. Those controls are assertions in the test
> now, not a step someone happened to take. Third wave running where a measurement overturned the
> standing story, and the second where the measurement itself was broken in the direction of the
> expected answer.
>
> **When a conjunction of four conditions is the coverage, multiply before adding seeds** (wave 251).
> Observing an extension defect needs the struct routed, the field unsigned, the value in the
> discriminating half, and the program compared — about `5% × 1/3 × 1/2 × 1/2`, or one program per
> two hundred seeds. More seeds buys that rate linearly; fixing the weakest factor buys it in one
> change. **Estimate the product before running the long job.**
>
> **A mutant that disables an assertion cannot be killed by that assertion** (wave 251). Two
> survivors in this sweep were `true || X` on the guard's own asserts — a degenerate class worth
> recognising rather than chasing, since no test can observe its own removal.
>
> **A generator can emit a construct, read it, and still be unable to observe a defect in it** (wave
> 250). Bit-fields were generated in a quarter of programs and read in most of those, and 400 soak
> seeds against a known wrong answer found nothing. Emitting a construct, giving it a value that
> discriminates, and routing that value to the comparison are **three** conditions, and coverage
> talk usually counts the first.
>
> **An adequacy guard that measures one of two necessary conditions reports the coverage you hoped
> for** (wave 250). The first version counted bit-fields whose top bit was set — 15 of 44, healthy —
> and passed. The number that mattered was `unsigned` *and* top-bit-set: 5. I wrote the guard, it
> went green, and it was measuring the wrong thing.
>
> **A ratio over seven samples decides nothing** (wave 250). With the misattribution fixed, the
> six-hundred-seed range yielded seven unambiguous unsigned initializers, and mutants that removed
> the rule entirely still passed. Widening to three thousand seeds — string work, a fifth of a
> second — killed them. **Check the sample size before trusting an adequacy ratio.**
>
> **A scan keyed on a name will attribute one record's field to another** (wave 250). Every generated
> struct has an `f0`. The guard's own lookup was misattributing five of fifteen, and it surfaced only
> because forcing the top bit *on* failed to make the count reach the total — an experiment run to
> check a mutant, not to check the scan.
>
> **Consuming randomness to add a feature moves every corpus** (wave 250). The first attempt drew the
> bit-field value from `rng`, shifting the stream and every downstream decision; the metric got
> *worse*, because the programs being measured were no longer the same programs. §9's rule about
> gating new arms before any `rng` call is the same rule from the other side.
>
> **One value consumed twice will be discharged once** (wave 249). `report_faults` filtered a fault
> list, reported from the filtered copy, and returned `()`. A second consumer needed the same list to
> decide whether the *value* was usable, had only the raw one, and threw away values the engine had
> just proved fine. **When an expensive refinement is computed for one consumer, check who else reads
> the unrefined input** — the cost of the proof is the tell that it is worth sharing.
>
> **A wrong answer can hide behind an operation that commutes with truncation** (wave 249, and 217
> before it). An `unsigned` bit-field was sign-extended on read for a hundred waves. `s.a += 1` could
> not see it — reading 5 or -3, adding one and truncating to three bits gives 6 either way — and
> neither could any value whose top bit was clear. Division, shift and the value a *postfix* form
> yields are what see it. **Pick the operator that does not commute with the narrowing, and pick
> operands in the half of the range where the two rules differ.**
>
> **`is_signed(expr)` and "the declared type is signed" are different questions** (wave 249). C11
> 6.3.1.1 promotes a narrow bit-field to `int`, so the expression is signed where the field is not —
> and the promotion applies to an rvalue but not to an lvalue, so the two answers agree at some sites
> and not others. That is exactly the shape that makes a helper look right everywhere it is tested.
>
> **A commit message is a claim and mutation checks it** (wave 249). The GREEN said the
> compound-assignment path was "the same defect one layer in". The mutant would not die; four
> fixtures later, instrumenting both expressions showed they agree at that site and the path was
> never broken. **The sweep audits the prose as well as the code**, and the correction belongs in the
> tree, not only in memory.
>
> **A hypothesis in §9 is a liability until someone runs its own check** (wave 247). "The `arr.data`
> the read sees does not contain the seeding stores" sat at the top of the open list from wave 201 to
> 247, complete with the one-`eprintln!` check that would settle it. Running that check took twenty
> minutes and disproved it. **Six waves of readers took the conclusion and skipped the check** — the
> note even said "only revisit the design question if the term ids turn out to match", and they did.
> Write the hypothesis down, but mark plainly that it is unverified, and run the check *before*
> building on it.
>
> **A wrong value and an invented value look identical from the outside** (wave 247). "Solves to 0,
> not 5" reads as a stale array. It was a *free variable*: the engine discarded a correct term,
> minted an unconstrained symbol, declared `Unknown`, and the solver picked 0. **Before chasing where
> a wrong value came from, check whether it is a value at all** — a symbol the solver was free to
> choose will impersonate any wrong answer you expect.
>
> **`return_value_bits` means "no *ground* value", not "no value"** (wave 247). After promotion a
> returned byte is a `select` over a symbolic array, so it answers `None`, and the first version of
> the RED read that as "no value at all". Solving the path and evaluating under the model is what
> reproduces the symptom. A helper that answers `None` for two different reasons will be misread.
>
> **Verify instrumentation before believing its silence** (wave 247, and 222 before it). Four tagged
> write-back sites logged nothing, which reads as "these never run". A control line at the function's
> entry proved the logging worked *and* that those sites genuinely did not fire — so the real writer
> was one I had not tagged, and the silence was two facts, not one.
>
> **A release-built soak cannot see an arithmetic panic** (wave 246). Factoring `pack`'s body into a
> width-parameterized `round_to` introduced `sig >> width`, which at width sixty-four is a shift of
> sixty-four on a `u64` — an overflow in its own right. 960,000 soak cases passed while the debug
> test suite panicked, because `--release` turns off exactly the checks that catch it. **Run the soak
> in debug too**, and remember that a passing soak is evidence about answers, not about panics.
>
> **Three samples cannot distinguish two rules** (wave 246). Probing the target with three hand-picked
> NaNs said narrowing drops the payload; all three happened to carry their bits *below* the
> twenty-three that survive, so truncation and a canonical answer were indistinguishable. A
> 480,000-case soak disagreed on one in ten. Wave 243's "ask the hardware" is right and incomplete —
> **ask it with inputs that could tell the answers apart**, which means picking probes against the
> hypotheses rather than for readability.
>
> **`x != x` is a test of NaN-ness and no test of which NaN** (wave 246, and 243 before it). Every C
> fixture for a narrowed NaN asked exactly that, so three payload mutants survived a suite that
> looked thorough. The lesson belongs to the shape of the assertion rather than to the format, since
> it had to be learned twice — at eighty bits and again at thirty-two.
>
> **A flake is worth reproducing before it is worth explaining** (wave 246). Two solver-cache tests
> failed in a sweep and passed four times in isolation. Twelve spinners on a twelve-core box made
> them fail every run: both assert `Sat` on a cold query as *setup*, and 022 §4's watchdog fires on a
> loaded machine. The first hypothesis — "my change broke the solver" — was wrong, and the
> reproduction is what said so rather than the reasoning.
>
> **An audit's best outcome is a small number** (wave 245). Twenty-nine `size_of`/`align_of`
> fall-throughs in `chiero-lower` were instrumented and the whole suite run against them; **twenty-
> eight never fire at all**. The hazard wave 244 named is, in this crate, almost entirely theoretical
> — and knowing that is worth more than the one defect the audit found, because it says where *not*
> to spend the next wave. Instrumenting and counting beat reading and worrying.
>
> **Symmetric fixtures test one side** (wave 245). Every conditional fixture had an array in both
> arms or a function in both arms, so the *then* arm's decay alone produced the right common type and
> the else arm's could be deleted unnoticed — and nothing had ever put an array in the *condition*.
> Mutation found both. **When an operation has interchangeable positions, at least one fixture must
> make them differ.**
>
> **A fixture written to isolate a mutant is a good place to find a second defect** (wave 245).
> `c ? 0 : a` was written only because it breaks the arms' symmetry; it then failed for a reason of
> its own — `common_type` did not know 6.5.15p6's null-pointer-constant rule and answered `sizeof`
> four instead of eight. Two defects, one of them found by the tooling for the other.
>
> **`sizeof` is blind to which pointer a rule picked** (wave 245). Both `void *` mutants survived a
> fixture set built entirely on `sizeof`, because every pointer is eight bytes. Pointer *arithmetic*
> distinguishes them — `void *` advances one byte, `int *` four. **Pick the observable that the rule
> actually changes**, not the one that is easiest to write.
>
> **A refusal upstream can become an invented value downstream** (wave 244). `fp` declining a
> subnormal was honest. But lowering falls through to the `f64` that `float_literal` computes when
> `fp` declines, and `0x1p-16400` is a *zero* there — so `0x1p-16400L != 0.0L` answered false. **The
> same fall-through produced wave 240's decimal defect**, which makes it a structural hazard rather
> than two coincidences: a declared limit is only declared if nothing downstream substitutes for it.
> Worth an audit of every `?` and `unwrap_or` on the lowering path.
>
> **Errors that cancel hide a whole class of fixture** (wave 244). Removing `unpack`'s normalization
> entirely passed every subnormal fixture there was, because `pack`'s denormal shift undoes exactly
> what the normalization did — so whenever the result is *also* subnormal, two wrongs make a right.
> Only an input whose result **escapes** the special range can tell. **When two stages are inverses,
> a fixture that stays inside their domain proves nothing about either.**
>
> **The shrink-and-enumerate trick is now the standard answer to a surviving rounding mutant** (wave
> 244, after 241 and 242). Three million random products could not produce the pattern the denormal
> sticky needs — sixty-two consecutive zeros in a product's low half — and enumerating at five- to
> eight-bit significands found it in seconds, in a shape that scaled straight back to sixty-four. Used
> three times now: to find a witness (241), to prove two witnesses do not exist (242), and to find one
> again here. **Reach for it before reaching for a bigger random sample.**
>
> **A test that names a constant instead of a value is testing the name** (wave 243). Every unit test
> about an invalid operation asserted `Some(fp::INDEFINITE)`, which compares the implementation
> against itself — flipping the constant's sign bit changed both sides of every assertion at once and
> the suite passed. The C fixtures were no help either, because `n != n` is true of every NaN whatever
> its sign: a good test of NaN-*ness* and no test of *which* NaN. **Spell the expected bits out, at
> least once, somewhere.**
>
> **Ask the hardware before deciding an approximation is necessary** (wave 243). §9 had carried an
> open design question — is a canonical quiet NaN honest when §6.2 wants the operand's payload? — and
> the answer was that the question does not arise. Nine `printf`s against `volatile` operands showed
> x87's rule is three lines long and exactly implementable, so there is no approximation and nothing
> for 023 §7 to declare. **The decision looked like a trade-off and was actually a missing
> measurement.**
>
> **A generator that makes one thing special at a time never reaches the interactions** (wave 243).
> The soak drew one operand per case as a NaN, infinity or zero — so `∞ - ∞`, `0 × ∞`, `0/0` and
> `∞/∞`, every invalid operation there is, were reached zero times in 360,000 cases. The tell was wave
> 222's: three operations reporting *identical* NaN counts, which is impossible if the invalid
> operations contribute anything. Drawing each operand's class independently fixed it. **Check that a
> generator's special cases can co-occur, and use a count that must differ to prove it.**
>
> **Enumeration can prove a case does not exist, and that is a design input** (wave 242). Division
> needs neither a tie branch nor a carry branch — enumerating every normalized operand pair at
> significand widths six through twelve says so, and the structural reasons hold at every width. So
> the branches were never written, and their absence is stated as `debug_assert!` rather than as a
> comment, which puts the proof under `cargo test` instead of beside code nobody re-checks. **Prove
> the case away before writing the branch, not after.**
>
> **A fourth reason a mutant survives: an assertion about an absent case** (wave 242). Forcing `div`'s
> `sticky` to `true` survives everything, because `sticky` feeds only the assertion that records why
> there is no tie. Killing it would need the very input the proof rules out. That is not a missing
> fixture, not a defect, and not dead code — it is unfalsifiable by construction, and it stays,
> because a vacuous assertion is cheaper than an unchecked proof.
>
> **When a fixture and the code disagree, the fixture is a suspect too** (wave 242). `1e-4000L /
> 1e4000L` was written into the RED and then failed the GREEN, and the code was right: the quotient
> is far below the format's floor, so a declared gap is the honest answer and `agree` compares
> *values*. **A RED fixture is a hypothesis about the answer, not the answer.**
>
> **A disjunctive test can lose half of itself silently** (wave 242). "Agrees with gcc *or* declares a
> gap" was reached only on the first branch once every operation worked, so it had become a plain
> `agree` with a misleading name. Instrumenting which branch each body took found it — and the fix is
> a body that still takes the second one. **When a capability lands, ask which existing assertions it
> just made vacuous**, which is wave 237's rule pointed at tests instead of code.
>
> **A random soak and a constructed fixture cover disjoint things, and neither substitutes for the
> other** (wave 241). 520,000 random operand pairs against gcc's own x87 found zero disagreements in
> add, subtract and multiply — and then mutation killed a sticky-bit mutant that the soak had passed
> 240,000 times. Two random sixty-four-bit significands land on an exact rounding tie with probability
> about `2^-62`, so **a random search never reaches the boundaries and a fixture never reaches the
> bulk**. Run both, and expect each to find what the other cannot.
>
> **When a witness cannot be found by search, shrink the problem until it can be enumerated** (wave
> 241). The `d - 1` that accounts for discarded bits needed an operand pair no soak would produce, and
> deriving one by hand had already failed once. Re-implementing the algorithm at six-, seven- and
> eight-bit significands made the input space small enough to enumerate *exhaustively* — every
> significand pair, every exponent difference, both signs — which produced the minimal witness in
> seconds and scaled straight back up to sixty-four bits. **The structure of a float algorithm is
> width-independent**, which is what makes the trick work here; look for the same property elsewhere.
>
> **The same enumeration proves a mutant equivalent, which is a result and not a failure** (wave 241).
> `renorm-right-no-sticky` differs in zero cases at every width, because a sum reaches bit 127 only
> when the exponents differ by sixty-three or less, and at those differences the aligned operand still
> has a zero in bit 0. The line was removed, as wave 231 removed a dead `strip_prefix('+')` for the
> same reason. **"Survived" has three causes — a missing fixture, a real defect, or a line that cannot
> fire — and only enumeration or proof tells them apart.**
>
> **Some behaviour is unreachable from the language and belongs in a unit test** (wave 241, and 237
> before it). The sign of an exact zero cannot be observed in C without division, because `-0 == 0`.
> `∞ - ∞` cannot be reached without producing an infinity first. Both are one-line assertions on the
> pure function. **Ask what the fixture language can express before assuming a gap in coverage is a
> gap in the code.**
>
> **A fixture that proves the old behaviour wrong is not a fixture that proves the new behaviour
> right** (wave 240). Nine of thirteen mutants survived the first sweep, and the reason was uniform:
> every RED fixture was written to fail *before* the fix, so each one asserted a direction rather than
> a value. `0.1L < 0.1` is satisfied by any conversion landing below the `f64` value — truncation
> included. `1e309L < 0x1p1030L` is satisfied by anything within two orders of magnitude. **The gap
> between "not the old wrong answer" and "the right answer" is exactly where a rounding defect lives**,
> and closing it means naming the exact expected result, which for a float means writing it in hex.
>
> **Two waves running, the first mutation sweep found only missing fixtures** (waves 239, 240). Six of
> twelve, then nine of thirteen — no defects, all fixture gaps. That is now the expected outcome of a
> RED written to observe a failure, and the sweep is what converts it into a test of the fix. Budget
> for it: the sweep is not a formality after the GREEN, it is the second half of the GREEN.
>
> **A conversion has more than one sticky source, and a fixture usually reaches only one** (wave 240).
> `from_decimal` discards bits in two places — the quotient's low end when it comes out sixty-six bits
> wide, and the division's remainder — and a mutant that forgets either one survives everything that
> only exercises the other. `0.1L` carries sticky in the remainder and `36893488147419103235.0L`
> carries it in the quotient, with no remainder at all. **When rounding depends on "was anything
> discarded", enumerate the places something can be discarded.**
>
> **A fixture whose reasoning you trusted is a fixture you have not tested** (wave 239). The RED's
> rounding case squared `1 + 2^-63`; the exact product does not fit in sixty-four bits, so I recorded
> that the low half must decide it. It does not fit, and the discarded half is *two* — nowhere near
> the halfway point, so truncation and round-to-nearest agree and the `truncate` mutant survived. The
> argument was true and the conclusion did not follow. **When a fixture is supposed to exercise a
> boundary, compute where it actually lands** — the replacements here were found by simulating the
> algorithm over candidate operands and keeping the ones that hit `> ½ ulp`, `= ½ ulp` and the carry.
>
> **The barely-reachable case is the one worth searching for** (wave 239). Rounding an all-ones
> significand up carries into a new power of two — a second normalization after the one the product's
> width forces, and the classic soft-float defect. It needs the exact product within `2^-63` of a
> power of two: unreachable outright in the branch where the product already fills 128 bits, and one
> part in `2^64` in the other. **No fixture written by picking round-looking numbers lands there.**
> Six of twelve mutants survived the first sweep and every one was a hole in the fixtures.
>
> **Overflow and underflow are not symmetric, and the asymmetry is 023 §7** (wave 239). An overflowed
> product *is* an infinity — fully specified by IEEE-754 §7.4 — so refusing it would declare a limit
> chiero does not have. An underflowed one is a denormal, and the plausible substitute is a zero: a
> confident claim that a very small number is nothing. **Return the value where one exists and the gap
> where the only alternative is a guess**; which end of a range you are at does not settle it.
>
> **A gap in one place makes a wrong answer safe somewhere else, until it isn't** (wave 239). Decimal
> `long double` literals have rounded through `f64` for six waves, harmlessly, because arithmetic
> could not observe it. The moment multiplication worked, `1e300L * 1e10L > 1e309L` answered *no* —
> `1e309` had overflowed `f64` to infinity. **A known gap's blast radius is a function of what else
> works**, so re-price the open list when a capability lands, not only when the gap itself moves.
>
> **Two failures can share a site and point opposite ways** (wave 238). Refusing to write left a stale
> value; writing an *uninitialized* marker would have re-created wave 195's invented
> `uninitialized-read`. The comment recording the second failure was ten lines above the line causing
> the first, and reading it is what produced the answer that avoids both — a fresh symbol, meaning
> written-but-unknown. **When a fix has an obvious form, check whether the opposite fix has already
> been tried here.**
>
> **A panic is a fixture telling you the shape you missed** (wave 238). Minting a symbol for the
> stored value crashed on a 256-bit store, because the arena caps a term at 128 bits — and the test
> that found it was written years-of-waves ago for a different purpose entirely. The guard that
> followed is narrow and its cost is recorded: the defect survives for `Int(256)`.
>
> **A declared degradation does not make the value honest** (wave 237). The stale-value defect ran at
> `Fidelity::Unknown` with the unmodelled operation named — every honesty mechanism firing — and still
> returned 2 for `2.0 * 3.0`. **Degrading the run and poisoning the value are two obligations**, and
> only one of them was met. 020 contract 43 says `Undef` is a value precisely so the second can be.
>
> **Implementing one operation can expose another's defect** (wave 237). Comparison had nothing to do
> with the stale multiply; it merely gave the disjunction test a second way to observe `x` after an
> unmodelled write. **When a new capability lands, re-read what the existing tests now reach** — the
> failure was in a test I had not touched.
>
> **A pure function's edge cases belong in a unit test, not in a fixture waiting on a feature**
> (wave 237). NaN comparison could not be written in C without `f80` division, so it went to
> `chiero-cir`'s own tests where `partial_cmp` is directly callable. The alternative was leaving
> IEEE-754 §5.11 untested until arithmetic lands, for no reason but habit about where tests live.
>
> **Fixing the layer beneath can retroactively arm the tests above it** (wave 236). Wave 235's
> tie fixtures passed for the wrong reason and a truncating implementation survived them; making hex
> literals exact turned the same fixtures into real tests, and `rounds-by-truncation` now dies. **A
> test that cannot discriminate is sometimes waiting on a different layer rather than badly written**
> — worth checking before rewriting it.
>
> **The widest legal input is a fixture, not an edge case** (wave 236). `0x1.fffffffffffffffep0` —
> every significand bit set — was a *refused function* before this wave and no test had noticed,
> because seventeen hex digits overflow a `u64` accumulator while the value fits x87 exactly. **When a
> format has a maximum, write it down as a fixture**; the trailing-zero shift that makes it fit was
> only found by trying.
>
> **A padded dump is not an expectation** (wave 236). I copied gcc's `0x017f8…` into a fixture and the
> printer does not zero-pad, so a correct value failed. The comment says so now, because the next
> person reading gcc's output into a test will do the same thing.
>
> **A fixture can measure the wrong layer and still pass** (wave 235). The tie fixtures for
> round-to-nearest-even were rounded by the *parser* before the conversion saw them, so they agreed
> with gcc for a reason that had nothing to do with the code under test — and a truncating
> implementation passed all of them. **Mutation is what distinguishes "passes" from "tests"**, and
> four survivors on a five-mutant sweep is the signal to stop and ask what the fixtures actually
> reach.
>
> **The same seam appears three times before you name it** (wave 235). `integral_float_literal`
> handed out an `f64` (wave 232, fixed 233), `hex_float` hands out an `f64`, and `float_literal` hands
> out an `f64`. Each was found separately as "this one loses precision"; they are one design decision
> — the front end answers in `f64` — and the fix is the same each time. **When a defect recurs in a
> third place, describe the shape rather than the instance.**
>
> **One refusal can keep two behaviours apart** (wave 234). `FpExt` and `FpTrunc` share an arm, and
> `as_f64` refusing width 80 as a *source* is the whole reason widening became exact while narrowing
> stayed a declared gap. Adding the widening was one line because that refusal already separated
> them. **When two operations differ only in direction, find the one guard that distinguishes them
> rather than splitting the code.**
>
> **A forward contract becomes a regression guard, and the label should follow** (wave 234). Wave 228
> recorded the `long double` disjunction as untestable-by-mutation and said so rather than claiming
> coverage. This wave's sweep found the mutant it could not see and one fixture closed it. The honest
> label changed, and §9 now says which kind of test it is — **a claim about a test's strength has a
> date on it.**
>
> **If a shared home refuses to compile, the split is in the wrong place** (wave 233). Moving the
> integral-literal encoder from `chiero-sema` to `chiero-cir` failed, because sema runs before CIR
> exists and 001 §4 puts CIR below it. The fix was not a dependency but a smaller sema function: the
> syntactic question stays, the target format leaves. **A layering gate refusing a move is telling
> you what the function is.**
>
> **The conversion that is harmless everywhere else is the one to check** (wave 233). Every
> integer-to-float target routes through `f64` because `f64` is at least as wide — and x87 is
> *wider*, so the same code rounds a 64-bit integer before it arrives. **When a width is added to a
> family, ask which existing shortcut assumed it was the widest.**
>
> **Move a duplicated pair before the third copy, not after** (wave 233). An encoder in lowering and
> a decoder in the engine looked tolerable until `SiToFp` needed the encoder in the engine too. The
> third call site is what forces the question, and it is cheaper to answer it in the wave that
> discovers it than to add the copy and plan a cleanup.
>
> **A surviving mutant may mean the sweep is missing a file** (wave 232). Two survivors died the
> moment `shapes.rs` joined the test set, and one of them was caught by a fixture that had been
> there for two waves. **Before writing a fixture for a survivor, check that the sweep runs the file
> that would already catch it** — I nearly wrote a second zero test.
>
> **A property truncation cannot see needs a test that is not a value comparison** (wave 232). An
> integral-literal path that accepted `2.5L` and dropped the fraction passed every differential
> fixture, because `(int)` of 2.5 and 2.0 are both 2 and the arithmetic that separates them is a
> gap. The bit pattern is where the difference lives, so the assertion belongs in the encoding test.
> **Ask what the observable projection discards.**
>
> **Two halves of one format in two crates is a third copy waiting to happen** (wave 232). `x87_bits`
> encodes in `chiero-lower`, `x87_trunc_to_int` decodes in `chiero-exec`, and `SiToFp` now wants the
> encoder in `chiero-exec` too. 001 §4 forbids the obvious import, so the answer is a shared home
> rather than a duplicate — and the moment to move it is before the third copy, not after.
>
> **A comment saying "this order matters" is a testable claim** (wave 231). I recorded two hazards I
> had reasoned about and avoided — stripping `0x` before the suffix, and stripping a `+` before
> parsing an exponent — and mutation killed neither, because neither hazard exists: a hex float's
> mandatory `p` puts a digit before the suffix, and `i32::from_str` accepts `+` already. One line was
> dead code. **A surviving mutant on a line you called load-bearing means the line is not**, and the
> claim in the comment is what mutation was testing.
>
> **Reasoned-about hazards are the ones to distrust** (wave 231). Both wrong claims were about traps
> avoided by thinking rather than observed by running. The parts I had actually measured — the
> significand width, the fraction scaling — were right. **Write down what you observed; mark what you
> inferred as inferred.**
>
> **A comment claiming a record is a claim about behaviour** (wave 231). `float_literal` says the
> `long double` narrowing is "a narrowing this records rather than hides", and nothing records it
> anywhere. That is the same defect class as wave 214's silent skip, hiding in prose instead of in
> code — and §9 now carries it as the next step rather than the reassurance it reads as.
>
> **A recorded plan's steps may already be done — measure before starting one** (wave 230). §9's step
> 1 was "a value that survives a store and a load", and wave 229's encoding fix had already
> delivered it: re-probing took one command and showed `long double y = x;` at `Exact` with no
> assumptions. The frontier had moved further than the wave that moved it realised.
>
> **The one-line version of a conversion is the wrong one** (wave 230). Teaching `as_f64` width 80
> would have made every `f80` cast work at once, and silently rounded a 64-bit significand into 53 —
> a wrong answer where a gap was declared. **When a decode makes several things work at once, ask
> what precision it spends**, and read the value out of the representation instead.
>
> **A fixture that fails for a reason upstream of the change belongs in a comment, not in the
> test** (wave 230). `2^62 + 1` would prove the conversion is exact and fails because the *literal*
> is rounded first. Deleting it silently would have lost the finding; leaving it failing would have
> blamed the wrong layer. The comment where it would go carries both bit patterns and §9 carries the
> step.
>
> **`cargo test | grep FAILED` reports nothing when the build is broken** (wave 229). Widening
> `Const::Float` broke five test files, and my sweep for `^test .* FAILED` came back clean because
> nothing ran. **A build failure is indistinguishable from zero failures to that grep** — run
> `--no-run` first, or count `^error`, before believing a green sweep. Third instance of trusting a
> grep's silence, after wave 222's `grep -c` counting compiler output.
>
> **Read the reference, do not derive it** (wave 229). The expected `f80` patterns came out of gcc's
> object bytes via `memcpy` and a hex dump. Deriving them from the format description is exactly the
> step that produced the bug being fixed, so re-deriving them for the test would have risked
> agreeing with the defect.
>
> **A surviving mutant on a general function may mean the callers are narrow** (wave 229). Dropping
> `x87_bits`'s sign term changed nothing, because C has no negative literals. Unlike wave 223's
> duplicated bound check — a second copy of a decision made elsewhere, which became an assertion —
> this is the only place that would get it right, so it stays with a test recording *why* it looks
> dead. **Distinguish "unreachable and redundant" from "unreachable and sole".**
>
> **Read the CIR, not only the verdict** (wave 228). `long double` degrades honestly and names the
> operation, which is the right *behaviour* — and printing the module showed
> `fconst:f80:0x3ff0000000000000`, an `f64` bit pattern in an `f80` slot. The verdict was correct
> and the artifact was wrong, and only one of those was visible from the outside.
>
> **A gap enforced upstream cannot be defeated downstream** (wave 228). Two mutants that implemented
> 80-bit arithmetic as `f64` both survived, because the value is `Undef` before arithmetic ever runs.
> That is a real property of `Undef` propagation (020 contract 43) rather than a hole in the tests —
> and it says where the milestone starts: a value that survives a store and a load, before any
> arithmetic.
>
> **Say when a test is a forward contract rather than a regression guard** (wave 228). The
> `long double` disjunction has no mutant that defeats it today. That does not make it decoration —
> it will bind the milestone — but claiming mutation coverage for it would have been false, and the
> honest label is what tells the next reader which kind of test they are looking at.
>
> **Ask why a duplicate is *identical*, not why there are two** (wave 227). Two
> `pointer-outside-object` lines from two paths is documented, deliberate behaviour — the engine
> keeps both so a second witness is not lost. What was wrong was that the two sentences were the
> same, because the witness was a constant instead of the path's own offset. The interesting
> question was one step past the one the output invited.
>
> **A witness that is always true is not always a witness** (wave 227). `obj_size` is outside the
> object by construction, so the sentence never looked wrong — and on a path that cannot reach it,
> it names an input that does not exist. **A false specific claim is worse than a vague one**,
> because a reader acts on it. Check that a witness is reachable *on this path*, not merely
> consistent with the fault.
>
> **A stale "also open" entry costs more than an empty one** (wave 227). Two of four had been fixed
> waves earlier, one with a comment at the fix site describing the exact bug the entry named.
> Reading them cost a wave's opening, and leaving them would have cost the next session the same.
> **When an item is checked and found done, strike it in the same wave.**
>
> **Grep the artifact's vocabulary, not the code's** (wave 226). I checked a freshly blessed golden
> for `CopyMem` and found none, and nearly recorded that the aggregate copies were missing — the
> printer writes `copymem`. The Rust variant name and the text format are two vocabularies, and a
> golden is written in the second. Same family as wave 222's `grep -c` counting compiler output.
>
> **When an item says "re-diagnose", suspect the note rather than the item** (wave 226).
> `pointer_fields.c` was owed for aggregate value semantics; I had recorded "arrow access is clean, so
> it must be about something else", which was true and irrelevant — `->` was never the subject. **A
> clean probe of the wrong thing is not evidence about the right thing.**
>
> **A corpus is an instrument, and its coverage is assertable** (wave 226). The goldens quantify over
> `tests/corpus/c/`, so a construct absent from it is held fixed by nothing — six defects once passed
> 1102 tests that way. Asserting the *inventory* (which shapes the corpus contains) is what stops the
> instrument silently narrowing, and it is a different test from any golden's bytes.
>
> **The precedent is usually in a comment explaining why some existing code is the way it is**
> (wave 225). Both parked decisions were answered by prose written for another purpose — `UnionPun`'s
> "off by default … for the projects that want the stricter reading" and
> `one_faulting_site_in_a_loop_is_one_finding`'s explanation of why `UbState` needs memory. **Search
> the rationales, not just the code.**
>
> **Resolving a decision in favour of current behaviour still changes something** (wave 225). The
> checker-dedup answer was "what the code already does", and the wave was not empty: the contract
> moved to where a checker author looks, and two tests now make it a contract instead of an accident.
> Choosing neither option was the only outcome that left a later change free to pick one silently.
>
> **State a contract at the call site, not at the discovery site** (wave 225). The rule about checker
> dedup lived in a test's doc comment about loops, because that is where someone hit it. It belongs on
> `Action::report`, which is what a checker author is looking at when they need it.
>
> **A decision parked for ten waves is a decision to take** (wave 224). Three were recorded as
> "yours to make" and the instruction stayed "continue autonomously"; blocking indefinitely was the
> wrong reading of it. What made the call defensible was not confidence but *precedent*: the repo had
> already resolved the same tension for `union-pun` — a stricter reading, off by default — so the
> shape was chosen for me. **Look for where the codebase has already answered the same question
> before treating one as open.**
>
> **The easy direction of a solver query is not the hard one** (wave 224). A test of mine asserted
> tier 1 would report nothing for a forced overflow and failed on correct code: satisfying
> `overflows` needs one model and tier 1 finds it, while refuting `safe` needs a proof over every
> value and it cannot. So a run without a backend earns the *weaker* kind — one program, two
> truthful answers. **When a query has two directions, ask which one your fixture actually
> exercises.**
>
> **If the kind carries the certainty, one site must yield one kind** (wave 224). Deleting the
> `return` after the forced report let one operation earn both kinds, and the control passed because
> it asserted the strong kind was *present* and not that the weak one was *absent*. A distinction
> worth encoding in the dedup key is worth asserting from both sides.
>
> **An unreachable guard protects nothing and reports nothing — make it an assertion** (wave 223).
> `take_edge` repeated a bound check that could not fire, so it always survived mutation. A
> `debug_assert!` on the invariant it was guarding *can* fail, is executed by every test, and names
> the reason: three tests now fail if a second counting site appears. **When a branch cannot be
> reached, the invariant behind it can still be stated.**
>
> **Two independent grounds before deleting defensive code** (wave 223). The static argument (one
> increment site, checked immediately) and the empirical one (zero firings across 1349 tests, and
> the removal changed no outcome) were both required — waves 198–203 were a run of confident
> readings that were wrong, and one argument would have been another.
>
> **A bound needs its tight case** (wave 223). `> max_depth + 1` cuts a twelve-instruction program
> one step late and every assertion passed. The smallest program the wrong bound lets *complete* is
> what discriminates: four instructions against a bound of three. Same rule as wave 215's
> `INT_MAX + 1` and `INT_MIN`, at a different bound — **fixtures well past a boundary cannot see an
> off-by-one at it.**
>
> **Mutate duplicated conditions separately** (wave 222). One `.replace()` covering both copies of
> the `max_depth` check reported KILLED and hid that only one copy is tested. Anchoring by line
> number and running each site on its own found the second unreached — the same "one defect, two
> homes" shape as wave 207, arriving through the sweep rather than through the fix.
>
> **`grep -c` on a build-and-run log counts the compiler too** (wave 222). An instrumentation marker
> appeared four times in four unrelated test files, which is what a diagnostic quoting the inserted
> line looks like, not what runtime firings look like. Identical counts across unrelated inputs are
> the tell. **Print the matching lines before believing the number.**
>
> **A question answered "no defect" is still worth the wave** (wave 222). §9 asked whether a
> `Bounded` run says enough to act on; it does, and finding that out located the one message that
> did not and a duplicated check nothing reaches. **The investigation's value was in the map, not
> the verdict.**
>
> **A structural bound in a generator is invisible until you look for it** (wave 221). Every
> construct count was satisfied and every construct agreed with gcc, and a loop body still could
> not contain a second statement — so the compositions §9 had named were unreachable by
> construction rather than by chance. **Ask what the grammar's *shapes* cannot contain, not just
> which constructs it lacks**, and assert the composition directly, because counting constructs
> cannot see it.
>
> **A hypothesis that finds nothing has still been tested** (wave 221). "The defects are in the
> interactions" motivated composing the arms; 1639 comparisons found none. The interval belongs in
> the record next to the hypothesis, or the next session re-runs the same experiment believing it
> is new.
>
> **When a fuzzer's output changes character, read the new category** (wave 221). Composition
> produced no defects and six `gap: Bounded` runs that no earlier grammar had. That is the channel
> saying it has moved from testing semantics to testing the engine's exploration budget — a
> different question, with a reproducible trigger, and it only shows up if the refusal ledger is
> read rather than the defect count.
>
> **Separate "does the grammar emit this often enough" from "how big was this batch"** (wave 220).
> Adding expression arms consumed randomness, reshuffled the stream and dropped `for`-continue to
> nine lines in the two hundred compared programs — under a threshold of ten. The guard was right
> and the threshold was not the problem: the shape counts now sample six hundred seeds, because
> they are a question about the grammar's propensity and not about this batch's size. **A coverage
> guard that fires after an unrelated change wants a bigger sample, not a lower bar.**
>
> **Pick the unemitted construct by the question it asks, not by the gap it fills** (wave 220).
> Seven expression forms were missing; four were generated, chosen because `~` promotes a narrow
> operand and `sizeof` drags unsigned conversions into the arithmetic — the class the one real
> defect of these three waves came from. `_Alignof` fills a gap and asks nothing.
>
> **An exhausted channel is a finding** (wave 220). Two censuses, ~2250 comparisons, one defect,
> and that defect came from an interaction rather than a construct. That redirects the next wave
> from "add a construct" to "compose the arms that exist" — which is a cheaper experiment and a
> different hypothesis, and it only becomes visible if the clean intervals are written down.
>
> **A coverage assertion can match the text of a statement instead of a statement** (wave 219). The
> check that a `goto` skips something looked for `" += "` in the span between jump and label, and a
> mutant that commented the line out passed it — `//` removes the effect and leaves the characters.
> **When asserting that generated code does something, exclude the forms that only look like it.**
>
> **One keyword can be two constructs** (wave 219). `continue` in a `for` still runs the increment;
> in a `do`-`while` it jumps to the condition. Counting the keyword found them together and turning
> one off changed nothing any assertion could see. Count what differs, not what is spelled the same.
>
> **A bound on the corpus is not a bound on the construct, and the difference belongs in an
> assertion** (wave 219). Generated `goto`s are forward-only because a backward jump can hang the
> comparison — not because chiero gets backward jumps wrong, which was checked by hand first. The
> assertion that every emitted jump goes forward keeps the bound from eroding into a belief that
> backward jumps are untestable.
>
> **Census the generator's *output*, not its source** (wave 218). Grepping `generated.rs` for
> `continue` counts the generator's own Rust control flow; generating two hundred programs and
> counting constructs in them found six C constructs neither grammar had ever emitted. The
> measurement has to be taken on the artifact, not on the thing that makes it.
>
> **A clean soak after a grammar extension is a result, not a null result** (wave 218). 900
> comparisons over the new `&&`/`||` shapes found nothing, which says short-circuit lowering is
> sound — worth recording as an interval, because the next person to suspect it should know it was
> looked at and how hard.
>
> **A coverage assertion may have to be a count rather than a universal** (wave 218). "Every
> short-circuit's witness is checksummed" failed on the *correct* code: a short-circuit inside a
> nested block has its variables truncated at the closing brace, because out of scope is out of the
> checksum. The count version still kills the mutant that stops observing the shape entirely.
>
> **When a fuzzer comes back clean, widen the grammar rather than the seed range** (wave 217). 490
> comparisons found nothing; two new statement forms found a defect in 700 seeds. The generator can
> only find what it can *say*, so the productive question is what the AST can hold that the grammar
> never emits — `StmtKind::Switch` and `DoWhile` had been supported by lowering and unreachable
> from the generator since it was written.
>
> **A defect invisible under one type may be invisible because the operation commutes** (wave 217).
> Coercing a compound assignment's right operand before the operation is wrong for every type and
> observable for exactly one: truncation commutes with `+`, `-` and `*`, so every wrapping integer
> hides it, and `_Bool`'s `!= 0` does not. **When a wrong order of operations passes, ask which
> types make the two orders agree** — that set is where the fixtures were.
>
> **Gate a new generator arm before it consumes randomness** (wave 217). An ungated `chance()`
> reshuffles every seed's program, and the memory-UB corpus lost enough `stack-buffer-overflow`
> programs to stop being gradeable. The adequacy guard caught it. `if self.knob && self.rng...` —
> short-circuit order is the whole mechanism.
>
> **The verifier caught what the fixtures did not** (wave 217). Dropping sema's conversion instead
> of promoting refused seven of two hundred generated programs with `Add operand is Int(1),
> declared Int(32)`. Nine hand-written cases had all passed. A structural check over the whole
> corpus sees what a fixture set aimed at semantics cannot.
>
> **Grade a channel on the shapes it claims, not on emitting the construct** (wave 217). Removing
> the fallthrough case and demoting the `default` both left the new channel passing — the programs
> still agreed with gcc, and the channel had quietly stopped exploring the two shapes it exists for.
> Count the shapes across the corpus, the way the `>= 20 of each form` guard already did.
>
> **An assertion of silence is worth nothing in a crate that cannot speak** (wave 216). I put the
> undecided-overflow test in `chiero-lower`, where it passed — and the mutant that turns `Unknown`
> into a finding survived it, because that harness registers no checkers, so *no* arithmetic report
> is possible there whatever the engine does. Wave 212 had already established this and I walked
> into it anyway. **Before asserting an absence, produce the thing whose absence you are claiming.**
>
> **A seam you are about to build may already be a public method** (wave 216). §9 asked for a way to
> inject a solver `Unknown` for three waves. `SolverTier::LiteOnly` has done it since wave 161 and
> four tests already used it — tier 1's incompleteness *is* the injection point, and it needs no
> test-only code. Look for the capability before designing the hook.
>
> **Two queries need two undecided fixtures** (wave 216). The discharge asks "is the guard implied"
> and "is it refuted" and the arms take their answers independently, so relaxing the *first* `Unsat`
> survived every fixture whose first query happened to come back `Sat`. One `Unknown` fixture does
> not exercise every `Unknown` arm; count the queries, not the arms.
>
> **A control that guesses wrong is worth more than one that guesses right** (wave 216). I asserted
> tier 1 would still refute a guard it cannot, and the failure produced the best test in the file:
> one program, `maybe` without a backend and definite with one. `Unknown` weakens a verdict rather
> than fabricating or dropping it — which is the property the whole tri-state exists for, and I
> would not have written it if the guess had held.
>
> **When a precedent does not transfer, say why in the code** (wave 215). Wave 156 reports a
> symbolic divisor on `Sat`; this query asks for `Unsat` of the negation instead, and the difference
> is one arm of one `match`. Division by zero needs the divisor to be one specific value where
> overflow needs only a large operand, so the same rule would turn every addition into a finding.
> A reader who finds the two queries side by side deserves that sentence next to the arm.
>
> **A monotone change is easier to justify than a better one** (wave 215). `Sat` and `Unknown` both
> stay silent, so the query adds findings and removes none, and no existing behaviour had to be
> re-argued. When a design fork is genuinely open, implementing the half that cannot be wrong keeps
> the other half a decision instead of a default.
>
> **Assert both ends of an asymmetric range** (wave 215). `INT_MAX + 1` must report and `INT_MIN`
> must not, because the representable extremes are `2^(w-1) - 1` and `-2^(w-1)`. Two mutants
> survived every fixture in the file for the same reason: they were all off by more than one, and
> only the exact boundary can see a bound that is off by one.
>
> **A surviving mutant sometimes means the guard is defence in depth** (wave 215). Dropping the
> `signed` check reported nothing anyway, because the query's own `sext` sends large unsigned values
> back inside the signed range. Recording *why* it survives is what stops the next reader deleting a
> correct guard on the evidence that nothing failed.
>
> **A bound may change how hard chiero tries, never what it finds** (wave 214). `EXPAND_LIMIT`
> silently switched the initialization check off at a size, so one more array element made a real
> finding vanish. The fix was not a bigger limit — any limit has a cliff — but downgrading the
> *form* of the question past it: an opaque `select` the solver may fail to decide, which wave 204's
> discharge already reports as a `maybe`. **When a bound is reached, degrade the answer, do not
> withdraw the question.**
>
> **A fix can retire a mutant** (wave 214). `always-base-zero` was tracked for nine waves as the one
> survivor that could lose a finding, and it is now equivalent by construction — the fallback means
> the seeding choice cannot change a verdict. That is a better outcome than a test for it, and it is
> worth saying which of the two happened rather than just striking it off the list.
>
> **Pin a bound where it is decided, not through a symptom** (wave 214). `expand-unbounded` survived
> two sweeps because an unbounded expansion is still *correct* — only bigger. The API is public, so
> the test is four lines in the solver's own suite: 300 stores refused at 256, the same chain
> accepted at 512. Both directions, or a mutant that always refuses passes.
>
> **Measure the cost you claimed was a risk** (wave 214). Sending array-theory questions to the
> solver was the obvious hazard of this fix, so it got timed both ways — and the first attempt was
> invalid, because the pre-fix run aborted early on the failing test and ran 14 binaries against 36.
> `--no-fail-fast` on both sides, then compare.
>
> **A filename match is not a dependency** (wave 213). The RED commit predicted the differential
> oracle would need updating because `generated.rs` contained the string "division by zero". It did
> not: that line reads *gcc's* UBSan stderr, and chiero's side of the comparison uses
> `format!("{:?}", u.kind)` — the enum's `Debug`, not the message. **Read the site before writing
> down what depends on it**, especially when the claim is going into a commit message as fact.
>
> **Measure the blast radius by what asserts, not by what matches** (wave 213). Thirteen files
> contained "division by zero"; two contained an assertion on it, and one of those was a
> preprocessor diagnostic for `#if 1/0` — a different message class that correctly kept its
> wording. The grep that mattered was `contains("…")`, not `"…"`.
>
> **A tempting improvement that nobody asked for goes in §9, not in the commit** (wave 213).
> Aligning these slugs to UBSan's own names would be genuinely useful and is a rename, not a
> spelling fix. Recording it as a question keeps the wave honest and keeps the decision with the
> person whose project it is.
>
> **Establish whether a hazard is reachable before proposing work on it** (wave 213). The missing
> dedup key on checker reports looked like the next defect; five minutes of reading showed forks
> are handled by id, one checker's repeats by its own state, and the only real gap needs a third
> arithmetic checker that does not exist. It is a latent hazard with a design decision attached,
> which is a different thing from a bug, and §9 now says so.
>
> **"I cannot write that fixture" is a claim to re-examine, not a fact to record** (wave 212).
> Wave 209 wrote down that a checker report could not be tested from `chiero-lower` because the
> checkers are not registered there, and left it. The obstacle was real and the conclusion was
> wrong: 001 §4 rule 7 forbids `chiero-check` a *frontend*, not a `SourceMap`, and
> `SourceMap::add_file` is public — so a hand-built map over the spans these fixtures already build
> tests the whole thing. Three waves of "covered by argument" for want of one more minute.
>
> **Enumerate the variants the code can take, not the ones the fixture happens to use** (wave 212).
> `Action` has two report variants; five fixtures exercised one, because a constant overflow
> carries no condition and only a symbolic operand produces `requires`. The untested route was the
> one with the *best* evidence behind it. The tell was in the enum, visible before mutation, and I
> did not look. Sixth consecutive wave of this shape.
>
> **A span that is convenient is usually the wrong one** (wave 212). A `Function`'s span and a
> `Block`'s both point at their own start, so stamping either passes any test that asks for "a
> location" while reporting every finding in a function at its opening brace. Assert *which* line,
> from several candidates that differ.
>
> **Tests written for the defects are what make the refactor cheap** (wave 211). Four waves of
> report fixtures pinned every sentence before `describe` rewrote how they are built, so "1322
> passing" *is* the proof that nothing changed. A refactor with no characterization tests is a
> rewrite; with them it is a mechanical edit. **Notice when the tests you already have make a
> deferred change safe, and take that moment.**
>
> **Resolve shared values before the match, not in each arm** (wave 211). Nineteen arms could each
> have called `name_of(*obj)` and bound `obj`; instead `object()` and `secondary()` are consulted
> once and the arms interpolate. Fewer bindings, no per-arm blocks, and — the real gain — a new
> variant cannot render an object that `object()` does not report, which is exactly the drift
> mutation found in `secondary()` one wave earlier.
>
> **Scripted refactors of a large `match`: transform head, tail and the irregular arms by exact
> text first, then sweep the regular ones** (wave 211). My first attempt rewrote the whole impl in
> one pass and produced unbalanced braces; the pattern that worked was three verified replacements
> followed by a mechanical sweep over what was left. Assert a count of 1 on every anchor.
>
> **A dead layer left behind is worse than the layer** (wave 211). The re-describe on the model
> route made the object substitution above it unreachable, and it compiled and passed. Grep for
> the thing you set out to remove and confirm the only matches are comments about its removal.
>
> **Assert the line *and* the column** (wave 210). The off-by-one this wave existed to fix
> survived its own test: a scope's closing brace has `hi` and `hi - 1` on the same line, one
> column apart. Fourth consecutive wave where the assertion was aimed at what the fixture
> happens to produce rather than at the property claimed — line but not column, one route but not
> the other, `ca` inside "unnamed local", a filename instead of the stamp. **When an assertion
> passes, ask which nearby wrong answer would also pass it.**
>
> **A rule without a fixture is a note** (wave 210). Wave 207 earned "one defect, two homes" and
> this wave wired both routes from it — then wrote six fixtures that all exercised the first one,
> and mutation caught the second being dead. The rule was applied to the *code* and not to the
> *test*.
>
> **A pre-existing test passing unchanged is evidence the rule is general** (wave 210). Collapsing
> a scope span to `hi` broke a synthetic point-span test; `hi - 1` fixed the real case and left
> that test untouched. When a fix makes an old test pass without editing it, the rule fits more
> than the fixture that prompted it — and when it needs the old test edited, suspect the opposite.
>
> **Script an edit across many literals by brace-matching, not by pattern** (wave 210). Adding one
> field to fifteen `StateFinding` literals by matching the line after `span:` put seven of ten
> insertions into function signatures, and three of those compiled. Match the construction, walk
> to its closing brace, insert there.
>
> **An assertion written against what the fixture produces is not an assertion about the
> property** (wave 209). The no-map control checked for the absence of `t.c:` and let a mutant
> through that stamped `(at ?:85:1)` — an invented location that simply did not name this
> fixture's file. Assert the absence of the *mechanism*, not of one of its outputs. Third
> instance of this shape in three waves (`ca` inside "unnamed local", the `\`-continuation
> mechanism, this).
>
> **When a report grows a second location, the test that pinned the first one has to be
> re-derived, not deleted** (wave 209). `the_line_named_is_the_free_and_not_the_access` asserted
> line 6 was absent, which was correct for a one-location report and wrong the moment the access
> was named on purpose. The requirement never moved; the *form* it checks did. Rewriting it kept
> the mutant that swaps the two locations dead — deleting it would have lost that.
>
> **Only the full suite catches a cross-file contradiction** (wave 209). The new file's five
> tests passed while the wave-208 file's control failed on the same rendering. Run the workspace
> before believing a green file, especially when the change is to text other tests read.
>
> **A fixture that cannot be written is worth a sentence in the commit** (wave 209). The checker
> report has no direct test because the checkers are not registered in this engine
> configuration. Writing that down turned an untested claim into a §9 item; skipping it quietly
> would have left "both routes are covered" looking stronger than it is.
>
> **A mutant that is equivalent today is a trap for the next arm** (wave 208). `secondary()`
> returning the access span for every other fault changed nothing, because only three messages
> render a location. The fourth would have been silently wrong. **Assert the contract where it is
> stated, not where it currently happens to matter** — the test was three lines in a file that
> already built all eighteen variants.
>
> **When the implementation is better than the test's wording, say so and change the test**
> (wave 208). The RED asserted the words "line 5"; what shipped is `t.c:5:1`, the form editors
> parse. The requirement was "a reader can find the place" and the wording was mine — but the
> matchers had to be re-checked for force afterwards, not just made to pass: the access-line
> control became `t.c:6` and the no-map control the absence of `t.c:`.
>
> **An optional capability keeps every existing caller working, and needs a test that says so**
> (wave 208). Making the `SourceMap` mandatory was the shorter fix and would have broken every
> `Engine::new` in the repo. The no-map control is what makes "optional" a fact rather than an
> intention, and it also pins the honest degradation: say less, never guess a line.
>
> **A substring can collide with the word for its own absence** (wave 207). The control asserting
> `ca` is still named passed with the naming deleted, because the fallback description is "the
> 8-byte unnamed lo**ca**l" — and `g` hides in "unnamed **g**lobal". Match names as words, and
> assert the absence of the fallback's own vocabulary. Wave 184's rule from a direction no one
> would guess.
>
> **Look at what the product renders, not at what the source says** (wave 207). §9's front was
> "message literals are unguarded"; scanning the literals came back clean twice. Running a
> fixture and reading the finding took one command and found that every heap report named an
> allocation counter. The literal is a template; the defect was in what filled it.
>
> **One defect, two homes** (wave 207). `report_faults` substituted the object name and
> `ModelRegistry::lift` did not, so `free`'s report kept the counter after the other three were
> fixed. It surfaced only because the test named all four heap classes rather than the one that
> motivated the wave. **When a fix is a substitution, ask what else builds the same string.**
>
> **Deleting the bad rendering can cost more than the defect** (wave 207). Replacing
> `freed at bytes 85..92` with prose that named no location at all broke a test that asserts the
> reader is told where the object died — 024 contract 10, and the code comment said as much. The
> answer was to keep the number and stop mislabelling it. **Check what the ugly thing is carrying
> before removing it.**
>
> **The output nobody tests is the output the user reads** (wave 206). Every fault test in the
> tree asserted a kind or a field; none rendered the message. Seven were malformed and one had
> lost its predicate to an old edit, and the suite was green throughout. Where a component's
> product is text, assert on the text.
>
> **An invariant finds the class; a fixture finds the instance** (wave 206). The test was written
> for one bad string and immediately failed on six more in four crates. Pin what any correct
> version must satisfy — no double space, content beyond the prefix — rather than the wording,
> and the test constrains the defect instead of the design.
>
> **An assertion with no mutant is not protection** (wave 206). `!s.trim().is_empty()` could not
> fail, because the kind prefix is written unconditionally; the sweep revealed it by having
> nothing to mutate. Rewritten as "content beyond the prefix", an empty arm fails it. **A mutant
> that will not build is a fact about the test, not a dud** — worth a second look before
> discarding.
>
> **Check the mechanism before writing it down** (wave 206). The RED commit blamed a `\` line
> continuation keeping its indentation. Rust's `\`-newline strips it, the two real continuations
> in the same `impl` render correctly, and `rustc` confirms it in three lines. The cause was an
> over-long single-line literal. Same failure as waves 198–203: a plausible mechanism written
> down as fact.
>
> **`git checkout --` after a mutation run destroyed uncommitted work again** (wave 206). Wave
> 205's message fix was wiped between the edit and `git add`, so `bd6075b` claims a change that
> was not in the tree. Wave 154's rule, twice in one session: **commit before sweeping**, and
> when a commit claims a fix, `git show` it.
>
> **A rejected fix is a fix waiting for its premise to change** (wave 205). Wave 202 declined
> the symbolic-read init check because the guard had to fold past seven non-matching stores.
> Two waves later the guard did not have to fold at all — `select_expand` eliminates the array
> and the solver answers arithmetic. **The check was re-attempted only because the refusal had
> recorded its reason**, and the reason had expired. Sixteen candidates and thirty-two had
> disagreed about whether a fault exists for four waves.
>
> **Put the question where the facts are** (wave 205). `chiero-mem` knows the whole init mask
> and cannot call a solver; the engine can call a solver and knows nothing about bits. The
> shape that worked is mem emitting a *term* and the engine discharging it — and the same split
> gives the report its witness, since only the engine has a model to read an offset out of.
> Neither half could have produced the finding alone.
>
> **An encoding that nothing can ask a question about is not a representation, it is a wall**
> (wave 205). Seeding 512 stores and seeding one constant array mean the same thing; only one
> of them lets a symbolic read reach its own answer. The fix that mattered most in this wave
> was not the check — it was making the data the check reads shallow enough to eliminate.
>
> **"Equivalent mutant" is a hypothesis, and it needs the same disproof as any other** (wave
> 205). Wave 203 called `back-to-byte-index` and `only-first-bit` equivalent because every
> conditional-init verdict was a `maybe`; wave 204 landed the discharge that removes that
> reason, and both survived unchanged. The real cause — no reader for what the write marks —
> was written in a comment in the function all along. **Prefer reading the code path the
> mutant sits on to arguing about what the mutant cannot express**; the argument was
> self-consistent and wrong twice.
>
> **A survivor that three new fixtures cannot kill is telling you about the architecture,
> not the fixtures** (wave 205). Write-then-read at the same symbolic index, offset past
> byte 0, and a path-pinned index read concretely: all silent under both mutants. The fourth
> fixture was not the answer; `read_term_at` returning `faults: vec![]` was. When targeted
> fixtures keep missing, stop writing fixtures.
>
> **A rejection recorded with its reason can expire; one recorded as a verdict cannot** (wave
> 205). Wave 202 declined the symbolic-read init check *because* proving the byte written
> needed a `select` to fold past seven non-matching stores. Wave 204 made the guard a solver
> question rather than a folding problem, and the premise was gone — visible only because the
> refusal named its own reason. Write down why, not just what.
>
> **`Unknown` needs its own fixture, or the tri-state is a two-state** (wave 205). Both
> mutants that collapse `Unknown` — into a definite report and into silence — survive, because
> the "undecided" test reaches a `Sat`. A third state nothing exercises is a third state
> nobody has checked; a *decidable* undecided case is not the same test.
>
> **Re-run the sweep instead of trusting the diagnosis** (wave 203). Wave 202 concluded the
> fixtures were why no init mutant died, and it was half right: rewriting them as locals killed
> *nothing*. What was also missing was an assertion that looks at the init finding at all — every
> test in the file asked about a value or a refusal. Two waves of hypotheses, and the check that
> settled it was re-running the mutants after each change rather than after the last one.
>
> **A mutant can be equivalent because the engine cannot yet tell the difference** (wave 203).
> `back-to-byte-index` and `only-first-bit` alter which init bits a symbolic write marks, and the
> verdict is `maybe` either way because nothing discharges the guard. That is not a missing test —
> it is a missing *capability*, and the honest record says which one rather than leaving two
> survivors unexplained for a fourth wave.
>
> **When a test cannot observe a change, suspect the fixture before the code** (wave 202). Four
> waves treated "no mutant dies on the init marking" as evidence about the engine, and two of
> them wrote a hypothesis about `arr.init` into §9. The cause was that every fixture used a
> *file-scope* array, which C zero-initializes — so the init mask was all-`Yes` and no init code
> could matter. One `char ca[64]` moved inside the function makes the whole area testable.
>
> **A fix that turns silence into a false positive is worse than the silence** (wave 202). The
> symbolic read's missing init check was added, made its target test pass, and reported
> `maybe-uninitialized-read` on memory the program had definitely written. Reverted with its
> control, because 023 §9's argument is that a report a reader must dismiss costs more than one
> that never came.
>
> **Eliminating explanations is progress worth committing** (wave 201). The open case behaves
> exactly as it did, and the wave still moved: instrumentation showed promotion seeds correctly,
> the read does use the array, and `eval` does walk store chains — which leaves one claim
> standing instead of four. A wave that removes three candidates has done real work even with no
> behavioural change to show.
>
> **When a fix lands in one of two functions sharing a structure, grep the other** (wave 201).
> Wave 200 fixed the byte-versus-bit index in `write_term`; the identical line sat in
> `write_at_symbolic_offset` for a wave. Same array, same mistake, one function apart.
>
> **An unobservable fix must say so in its own commit** (wave 201). Mutation killed nothing on
> the init-index change — deleting it entirely passes the suite — because the promoted read
> ignores `arr.init`. The commit message and the code comment both record that it is correctness
> by argument, and §9 now orders the read fix *first* so the next two become testable.
>
> **One observation beats two waves of argument** (wave 200). Waves 198 and 199 reasoned about
> which of `promoted_fault`'s four callers fired, eliminated three, and were wrong — the answer
> was the fourth, whose enclosing function neither wave had looked up. A `#[track_caller]` plus
> one `eprintln!` named it in a single run. The cost of observing is fixed and small; the cost
> of reasoning about code you have not read is unbounded.
>
> **An early return can bypass the branch you added to the same function** (wave 200). Wave 198
> put a promoted-object branch in `write_term` and it never executed, because a
> ground-constant fast path twelve lines above returns first. Read the function from the top
> before adding to the middle of it.
>
> **Two index spaces for one array is a silent corruption** (wave 200). `arr.data` is indexed
> per byte and `arr.init` per bit. A byte-indexed init store writes one bit of the wrong byte
> and leaves eight unset, and the only symptom is a `maybe-uninitialized-read` on a byte the
> program just wrote — which reads, from outside, exactly like the write not happening.
>
> **A handed-over diagnosis is a hypothesis, not a finding** (wave 199). Wave 198 stopped
> responsibly and wrote down what it believed: `promoted_fault` keys on `repr`, the read keys on
> `arr`, so the two must disagree. Wave 199 checked and they do not — all four sites that set
> `arr` without `repr` are write-backs *after* `promote_to_array` has run. The next reader would
> have spent a wave on it. Mark handover notes as suspicion or as verified, and verify before
> building on one — including your own from last wave.
>
> **After two waves of reasoning, instrument** (wave 199). Two waves have now argued about which
> `promoted_fault` caller fires, from four candidates, when a `println!` in one function answers
> it. Reasoning is cheap to start and unbounded; observation has a fixed cost. When the second
> wave's argument fails the same way as the first's, stop arguing.
>
> **Stop and hand over the diagnosis rather than guess at a fix** (wave 198). The attempt at
> the promoted-object plumbing reached the point of *guessing* — the object reports promoted to
> `promoted_fault` while the branch keyed on `entry.arr` does not fire, and the next step would
> have been trying things. The wave was reverted to green with the diagnosis written into §9,
> which is worth more than a half-fix and far more than a red suite. A reverted RED
> (`13470a5` reverts `ec70860`) is a normal outcome, not a failure to hide.
>
> **Two fields for one concept is the bug behind the bug** (wave 198, suspected). `promoted_fault`
> keys on `repr == Repr::Array`; the read path keys on `entry.arr`. If promotion can set one
> without the other, every fix written against either will miss half the time — and that is the
> first thing to check, before any plumbing.
>
> **A write that does not record having happened is indistinguishable from no write** (wave
> 197). `write_at_symbolic_offset` iterated its candidate list and wrote an if-then-else at
> each, so an *empty* list wrote nothing and **returned success** — the caller got a clean
> result and stale bytes. The sibling read had promoted on an empty list all along and called
> it "no pinning available". When two APIs are counterparts, diff their edge cases: the one
> nobody exercised is the one that lies.
>
> **Turn a warning into a test before acting on it** (wave 197). §9 said a promoted object
> refuses the arena-free byte APIs. Writing that as a fixture *first* meant the cost showed up
> as a named failing test rather than as a mystery three waves later — and the test now pins
> the current, worse-than-ideal behaviour so the next wave sees it change.
>
> **A new variant is safer than a wider field** (wave 196). `Pointer::off` could have grown a
> symbolic form; that would have made all 59 `Value::Ptr` sites silently claim to handle an
> unknown offset. A separate `Value::SymPtr` leaves them refusing — which for them is the
> honest answer — and the compiler named the five *exhaustive* matches that genuinely had to
> decide. That is the difference between a one-wave change and a 59-site sweep.
>
> **When a value's shape changes, the old assertions may be asking the wrong question**
> (wave 196). The RED used `return_value_bits`, which wants *ground* bits — and this wave's
> whole product is a value that is symbolic. Asserting concreteness would have asserted the
> opposite of what was built. `State::returned_a_value` was added for the property actually
> wanted, and a separate fixture *solves* for the bytes where they matter.
>
> **One implementation for one question** (wave 196). The first draft answered "what address
> does this value denote" in `cmp_operand` and refused it in the `Store` handler, which
> regressed two of wave 195's properties at once. `address_of_value` is now the single answer
> both share.
>
> **One sentinel for two meanings will report each as the other** (wave 195). `Value::Undef`
> stood for both C's indeterminate value — where a later read *is* an uninitialized read — and
> chiero's "I cannot represent this", where the program did write something. `fork_on_offset`
> returned it for the second and got the first's behaviour, so `int *p = ga + i;` accused the
> statement above it of never writing `p`. Look for this shape wherever one value means "the
> program left this unknown" and "the analyser gave up".
>
> **When a fix collides with a deliberate invariant, the invariant is usually right** (wave
> 195). Two fixes were tried before the one that landed: concretizing the offset (forbidden by
> `a_symbolic_ptr_add_offset_is_a_gap`, because a fabricated address makes every later report a
> confident claim about one arbitrary case) and letting a stored `Undef` keep the destination
> initialized (the `Store` handler's stated intent, correct for `int x; int y = x;`). Both were
> worse than the bug. Read the comment on the thing you are about to change.
>
> **Narrow an over-reaching RED rather than forcing it** (wave 195). Two of its assertions —
> "the path continues with a real value" — were false and had to be, because `Pointer::off` is
> a concrete `i64`. Making them pass would have meant the concretization already rejected. The
> honest move is to narrow the claim and record the milestone.
>
> **Give the type no way to express the wrong report** (wave 194). The access fault carries a
> width because an access has one; wave 193 raised it for a pointer *computation* and had to
> invent `1`, so the report said "1-byte access" of memory nothing touched. A flag on the
> existing variant would have left that field there to be filled in again. A separate variant
> with no size field cannot be misused the same way.
>
> **A rule recorded two waves ago is not a rule you have absorbed** (wave 194). Wave 184
> established that a substring is not a kind, on the arithmetic census. The tests wave 193
> wrote nine days later matched `contains("bounds")`, and when wave 194 renamed the fault they
> reported that a *correct* finding had vanished. Reading the rules list is not the same as
> applying it; grep for the mistake, not for the lesson.
>
> **A control that passes without reaching the code controls nothing** (wave 193). Every
> guarded fixture — `if (i<0||i>1)`, `ga[i & 1]` — stayed silent because the offset
> *enumerated successfully* and the new solver query never ran. Four mutants on that query
> survived, including one that reported without asking the solver at all. When a control
> passes, check it fails for the reason you think: the fixture that reaches the code is the
> one with a constrained index too large to enumerate.
>
> **An equivalent mutant can correct the commit that named it** (wave 193). The claim was
> that signed comparisons were load-bearing against an unsigned one. They are not — with a
> lower bound of zero the two are the same predicate — and the code comment now says so
> rather than asserting a distinction that does not exist.
>
> **Fixing one class exposes the next, so measure after each** (wave 192). A null *call*
> reported nothing for as long as chiero has existed, and it stayed hidden because the shapes
> that reach it could not arise: a table of function pointers read as null before wave 191,
> so `tab[i]()` never got as far as calling anything. The measurement §9 asked for was
> supposed to produce a number and produced a defect instead.
>
> **A degraded run is the more misleading way to be wrong** (wave 192). "chiero could not
> follow this" is 023 §7's honest answer for a modelling *limit*; used for a definite fault
> at a definite place, it shows a reader scanning for findings a clean run. When adding a
> `degrade`, ask whether the thing being degraded about is unknown or simply unreported.
>
> **Assert behaviour, not representation, when the representation is the thing you are
> choosing** (wave 191). The RED ran each program and read what it computed, so
> `GlobalInit::Relocated` was free to be a relocation list, bytes with provenance, or
> anything else. A test pinning the variant would have made the design harder to change and
> proved nothing extra — 020's contracts are about what the engine *does*.
>
> **Reserve the slot before computing what goes in it** (wave 191). A global's `GlobalId` was
> taken from `globals.len()` and the entry pushed afterwards, which is correct until
> computing the initializer *pushes more globals* — `char *tab[2] = { "ab", "cd" }` interns a
> literal per element. The verifier caught it as `IdNotIndex`, the rule it exists for. Any
> "allocate an index, build the value, then push" shape has this bug latent in it the moment
> building the value can allocate.
>
> **Ask the invariant, not the cases** (wave 190). §9 called for "a pointer-typed global is
> non-null unless the program wrote `0`" instead of a list of initializer forms, and the
> survey it produced found *five* broken forms where enumerating would have found the one
> already suspected. The commonest — `char *s = "hi"` — was not on anybody's list, and it is
> in every C program there is.
>
> **A fix that widens a rule must be told where the rule stops** (wave 190). Teaching
> `global_addr_init` about string literals immediately broke `char s[4] = "hi"`, which
> *copies* the bytes rather than taking the address (C11 6.7.9p14). The distinction is the
> declared type and nothing in the initializer, so the flag had to be threaded from the
> declaration. An old test caught it in one run — the value of having asserted the ordinary
> case years of waves ago.
>
> **Write the RED against the route you expect to be hardest** (wave 189). The front was
> "an escaping function address should restore the assumption", and the obvious fixture is a
> local function pointer — which lowering already represents as `AddrOfFunc`, so the fix
> would have been the escape scan alone. Asserting the *global* route instead surfaced a
> second, worse defect: `int (*table)(int *) = helper;` lowered to `GlobalInit::Zero`, so a
> global function pointer compared **equal to null**. A silent wrong answer, sitting behind
> a missing feature.
>
> **A fall-through whose default is sometimes correct hides its own bugs** (wave 189).
> `GlobalInit::Zero` is right for an uninitialized object (C11 6.7.9p10) and wrong for an
> initializer chiero failed to encode, and the two are the same value. That is why nothing
> noticed. Where a default doubles as an error path, the test to write is the *invariant*
> — "a pointer-typed global is non-null unless the program wrote `0`" — not a case list.
>
> **An assumption is only worth making where the fact is unavailable** (wave 188). "The
> caller is outside the analysis" is true of an exported function and false of a `static`
> one, and the fix was to make the engine tell them apart rather than to report less. The
> corpus went from 3 findings to 0 with nothing weakened — the assumption simply stopped
> being applied where the answer is knowable.
>
> **A field the engine reads must survive the text format** (wave 188, second instance after
> wave 174's `signed`). CIR's printer is not a debug aid: a `.cir` file that loses `static`
> round-trips a program whose *findings differ*. Whenever a new field changes behaviour,
> print it, parse it, and make the round-trip generator vary it — the last part is what
> makes the first two testable, and without it the printer can drop the field unobserved.
>
> **A true finding and an actionable one are different things** (wave 187). All three null
> dereferences the corpus produces are correct — the functions really do crash on null — and
> all three are about `static` helpers whose callers all pass `&table[i]`. The fix was not to
> report less but to make each report state its premise, so a reader can tell chiero's
> assumption about an unseen caller from a null the program itself produced. Measure the
> *rate* before deciding a policy is too noisy, and separate "wrong" from "unactionable"
> before choosing what to change.
>
> **Mutation asks for programs, not assertions** (wave 187, third instance). Three survivors
> across two waves — a fork that replaced rather than added, a fork that covered only the
> first parameter, an annotation that matched every fault kind — and not one could have been
> killed by strengthening an existing fixture. Each needed a program with a different
> *shape*. When a mutant survives, ask what program the file does not contain.
>
> **When a default changes, fix the tests by naming their subject — never by relaxing an
> assertion** (wave 186). Six tests broke on the nullable-pointer default. Every one counted
> states or asserted "no findings" while being about aliasing, materialization depth,
> provenance, format strings or string models, and every one was fixed by turning the *new*
> policy off with a note saying why. Loosening `states().len() == 1` to `>= 1` would have
> silently retired the property each test existed to hold.
>
> **A finding that breaks a test can be the correct answer** (wave 186). `printf("%s", p)`
> started reporting a null dereference, and it is right — passing null there is undefined and
> glibc's `(null)` is an extension. The test was failing for a real reason on the wrong
> subject. Ask whether the new finding is *true* before deciding the change is at fault.
>
> **"By construction" in a generator is a decision about what later channels can see**
> (wave 185, third instance). Wave 177: `Gen::arrays` kept every index in range, so ASan
> never fired. Wave 178: no `malloc`, so the heap classes were absent. Wave 185: divisors
> non-zero, so `DivByZero` was graded by nothing. Each constraint was right for the
> value-comparing channel that motivated it, and each silently bounded a channel written
> later. When adding a corpus constraint, write down which oracle it serves — the next one
> will need it lifted.
>
> **Record a flake even when it does not reproduce** (wave 185). One full-suite failure,
> then 5 clean isolated runs and 2 clean full runs. Written into §9 with the hypothesis and
> what would confirm it, because the alternative is that the next occurrence starts from
> zero — and an intermittent failure that nobody has a prior on is the one that gets
> re-run until it passes.
>
> **Assert the direction only one side can be wrong in; report the other** (wave 184). gcc
> reporting UB that chiero missed can only be a chiero defect — gcc ran the operation and the
> standard calls it undefined. The reverse is not symmetric: gcc may be silent because it
> never checked. So `miss` is an assertion and `extra` is a printed note, and the two are not
> interchangeable even though both are "a disagreement".
>
> **Two oracles over the same engine can need opposite defaults** (wave 184). ASan's silence
> means it checked and found the access in bounds; gcc's silence may mean the operation was
> folded away before UBSan could instrument it. Copying the memory oracle's `invented`
> assertion into the arithmetic one would have filed a correct finding as a defect. The
> difference belongs in the code beside each, not in a reader's memory.
>
> **An oracle's assumptions belong in its assertions** (wave 183). `invented` had always
> depended on ASan's one concrete run visiting every path chiero explores. That was true,
> undocumented, and unenforced — a property of the grammar that happened to hold. The cost of
> it silently ceasing to hold is not a failing test but a *misdiagnosis*: a correct finding
> on an untaken path, reported as a chiero false positive. When a channel's soundness rests
> on a property of its corpus, assert the property, not just the result.
>
> **A failure message should name the decision, not the symptom** (wave 183). The tripwire
> says to constrain the grammar or downgrade `invented`, and says explicitly *not* to raise
> the bound — which is the one change that makes the problem invisible again and the one a
> hurried reader would reach for first.
>
> **When the oracle and the engine disagree, ask which one the *language* backs** (wave 182).
> ASan in recover mode reports every fault; chiero reports the first and stops. Neither is a
> bug — but C gives an execution no defined continuation past undefined behaviour, so the
> reports after the first describe a simulation rather than the program. The oracle was
> changed to grade the first fault, not the engine to report more. A disagreement is a
> question about *whose model is right*, and the standard is the tie-breaker.
>
> **A rule with no test is a rule someone will "fix"** (wave 182). "The path ends at a
> definite crash" was stated in a comment, obeyed by the code, and asserted nowhere. It
> reads exactly like a missing feature. `first_fault.rs` pins it with fixtures chosen so the
> pin cannot be satisfied by accident: two overflows on *different objects* (so dedup is not
> what is observed), both fault orderings (so it is not a preference for one kind), and a
> single-fault control (so reporting nothing fails).
>
> **A check that cannot read its input looks exactly like a check that passes** (wave 181).
> The location check was written, ran green, and was grading nothing twice over: without
> `-g` ASan prints module offsets rather than lines, and frame `#0` of a double-free is
> ASan's `free` interceptor rather than the program. Both produced "no line to compare",
> which the code treated as *nothing to disagree about*. Before believing a new comparison,
> make it print what it extracted — not just its verdict.
>
> **A wave whose predicted defect is absent still ends in a commit** (wave 181). §9 called
> the location gap the cheapest of three and worth the most; chiero was right on all 36
> programs. The assertion landed as a ratchet, proven observable by mutating the engine's
> span rather than by a natural failure — which is what the protocol's mutation clause
> exists for. Reporting it as a fix would have been the dishonest option; skipping the
> commit would have thrown away the ratchet.
>
> **A disagreement column grades both participants, not one** (wave 180). `invented` was
> added in wave 177 to catch chiero inventing faults. Its first hit was chiero being *more
> precise than the oracle*: seed 1's `a[6]` on a `malloc(24)` block reads `b[0]` of the next
> allocation, 48 bytes away, and ASan cannot distinguish that from a valid access to `b`.
> Widening the redzone made ASan report it too, and the flagged count rose. A channel that
> assumes the oracle is right would have recorded a correct finding as a defect.
>
> **Measure the tool's limits, not just its output** (wave 180). ASan's detection is a
> poisoned band of *bounded width* after each allocation; overflow far enough and it is
> silent. That is a property of the instrument, invisible in its output, and the generator
> was reaching past it — indexing three elements beyond an eight-byte-element array is 32
> bytes, past the default band. `redzone=64` is a correctness setting for this corpus, not
> a preference.
>
> **`> 0` is not a floor** (wave 180). Third time this session: wave 177 on the oracle's
> total, wave 177 again on the value differential's compared count, and now per class. A
> count that can drift needs a number that fails when it drifts; `> 0` only fails when the
> thing disappears entirely, by which point it has been useless for a while.
>
> **An oracle that stops at the first fault makes its classes compete** (wave 179). ASan
> halts on the first report, so a generated shape that fires early is the only fault its
> program ever shows. Adding the scope shape at one in four silently drove
> `stack-buffer-overflow` out of the corpus — the total was unchanged and the test passed.
> Emission rates in a corpus graded by a halting oracle are not tuning constants; they
> decide which classes get graded at all.
>
> **Record a prediction that turns out wrong** (wave 179). §9 said the oracle's compile line
> would need extending before the grammar could reach `stack-use-after-scope`. It did not —
> gcc 13 reports it under a plain `-fsanitize=address`. Checking first was still the right
> order, and writing down that the blocker was not there costs a line and saves the next
> reader the same detour.
>
> **A hardcoded failure message outlives the case it was written for** (wave 179). The
> missing-class assertion told every future class that "the grammar has no malloc or free in
> it at all", which was true when written and false for the next one. A diagnostic that
> misnames the cause is worse than none; it now prints what the corpus actually produced.
>
> **A total hides which rows are empty** (wave 178). The memory oracle reported `15 flagged,
> 15 caught` and read as parity; broken out by the class ASan itself named, it was two
> classes, both buffer overflows, with the entire heap missing. Any oracle over a
> *generated* corpus needs its score split by the thing the corpus is supposed to vary,
> because the corpus is the part most likely to be silently wrong.
>
> **When the engine sees further than the oracle, that is not a false positive** (wave 178).
> Every allocating program forks: malloc can fail, the generated code does not check, and
> the failure path really does dereference NULL. ASan's malloc succeeds so its one run never
> goes there. Wave 177's `invented` column assumed "closed programs, one path" — true then,
> false the moment malloc entered the grammar. Before recording a disagreement as a defect,
> ask whether the oracle *executed* the thing it stayed silent about.
>
> **Sanitizers report things that are not undefined behaviour** (wave 178). LeakSanitizer
> ships inside ASan and flags an un-freed block at exit, but a leak is defined — every
> operation does what C promises. Left on, it would have scored every allocating program as
> a chiero miss. Read what the oracle is actually reporting before treating its output as a
> UB verdict.
>
> **A constraint that serves one channel becomes a blind spot for the next** (wave 177).
> `Gen::arrays` kept every index in range *by construction*, with a comment explaining that
> an out-of-bounds read would be discarded rather than compared. Correct for the
> value-differential, and it silently decided that the census — built two waves ago, whose
> whole subject is programs gcc calls undefined — could never see a memory fault. When a
> second consumer appears, re-read the first one's invariants as choices rather than facts.
>
> **Probe the capability before building the oracle** (wave 177). The wave could have been
> "implement out-of-bounds detection". Five minutes of probing showed chiero already
> reports every shape — past the end, before the start, through a walked pointer, through
> NULL — with the object, offset and size named. So the wave was a *grading* gap, and the
> work was the corpus rather than the engine. The RED's failure message says which.
>
> **A generated corpus needs a floor, not a `> 0`** (wave 177). `flagged > 0` passes on one
> lucky program, and a change that quietly stopped emitting the interesting case would
> leave the channel green and empty. The oracle asserts `>= 5` against 15 observed, and
> prints its census either way. **This rule was then found violated in the oldest channel
> of all**: `generated_programs_agree_with_gcc` asserted `compared > 0`, and a mutant that
> switched the new out-of-bounds knob on for *that* channel survived the whole suite —
> every program would have been discarded as undefined and the test would have gone on
> passing. Now `compared >= 100` against 130 observed.
>
> **Sweep mutants past the code you touched** (wave 177). The knob was added for the memory
> oracle; mutating it found a missing floor in a five-wave-old value channel that had
> nothing to do with this wave. A mutation whose blast radius surprises you is the useful
> kind.
>
> **A surviving mutant is a missing test until proven equivalent** (wave 176).
> `store-always-signed` survived the whole suite, which meant one of the wave's three fixed
> sites had nothing observing it — the braced-initializer path, which the deleted helper's
> comment had explicitly reasoned would never arrive unsigned. The fix was to write the
> fixture, not to argue the mutant equivalent. `syntactic_signed-defaults-unsigned` *is*
> recorded as equivalent, and the difference is that its branch needs sema to fail to
> resolve a cast's type, which is a diagnostic elsewhere.
>
> **Grade the instrument before believing its verdict** (wave 176). One census row sat at
> `7 / 22` for three waves and read as a real gap in §9. It was a substring match:
> `cannot be represented` also appears in gcc's signed-overflow message, so the row counted
> row 1's programs again and graded them against a kind they never produce. A measurement
> nobody has tried to falsify is not evidence.
>
> **A control that fails is worth more than the assertion it was guarding** (wave 176).
> `(unsigned char)(-1.0)` went into the RED as a *control* — a case that should already
> report, proving the fix could not be "delete the check". It failed, and it was a second
> defect: the same dropped bit that reported `(unsigned char)200.0` falsely also *missed*
> the negative, because -1 fits the signed range. Write controls on both sides of the
> boundary and read their failures as findings rather than as test-authoring mistakes.
>
> **Agreeing on every value is not agreeing** (wave 176). Seed 117's differential verdict
> was `Agree` — chiero and gcc returned the same `int` — and the program was still being
> mis-judged, because the disagreement was about whether it is *undefined*, which no
> value-comparing oracle asks. The differential channel is structurally blind to this and
> the census is the only thing that sees it; that is what justifies keeping a second,
> slower channel over the same corpus.
>
> **A helper that cannot see the answer is the wrong shape** (wave 176). `target_signed`
> took a `&CTy`, which carries no signedness, and so had no choice but to return a constant
> — with a comment reasoning about why the constant was usually right. Every caller already
> had the `TyId`. When a function's parameters cannot express its question, the fix is at
> the call site, not inside it.
>
> **Probe the diagnosis, not just the owed list** (wave 175). Wave 174 wrote down a cause
> for census row 1 — symbolic operands — and it was wrong; the generated programs are
> closed and run at `fidelity Exact` with every value concrete. Reproducing before building
> cost one probe and replaced a milestone of solver work with a four-line fix. §9's existing
> rule was "probe the owed list before picking from it"; this extends it to the *explanation*
> written beside an item, which reads as settled fact one wave later and is not.
>
> **When a constructor family folds constants, the one that doesn't is a silent hole**
> (wave 175). `bin`, `not` and `extract` all folded; `sext` and `zext` did not, and nothing
> looked wrong because folding is an optimisation everywhere except where `as_const` is
> load-bearing. It is load-bearing in `note_ub`, which asks "do I know both operands?" and
> quietly does nothing when the answer is no. Look for the asymmetric member when a check
> fires on one spelling of a computation and not an equivalent one.
>
> **A difference that is only a literal's suffix is the reduction you want** (wave 175).
> `acc * 31L` reported and `acc * 31` did not. Two programs identical but for a suffix
> cannot differ in their undefined behaviour, so the gap had to be in the machinery between
> them — which is one `sext`. Reduce to the smallest *pair* that disagrees, not the smallest
> program that fails.
>
> **A pin written to fail is worth more than the assertion it replaces** (wave 174). Wave
> 173 could not implement two of C11 6.5.7p4's clauses, so it asserted they do *not* fire
> and said in the doc comment that the test would speak if `Shl` ever grew a signedness. One
> wave later it spoke — it was the first thing to fail, and it named its own successor. A
> limitation recorded as a passing assertion is a tripwire; the same limitation recorded as
> a comment is a thing the next reader re-derives.
>
> **Ask what the missing bit is a property *of*, before choosing where to put it** (wave
> 174). Signedness is a property of the C operands, so the candidates were the opcode, the
> type, and the instruction. The opcode was wrong because splitting `Add` would name a
> distinction the hardware does not make — the test is whether the *machine* operation
> differs, which is why `SDiv`/`UDiv` is right and `SAdd`/`UAdd` would not be. The type was
> wrong because `CTy::Int(w)` would have to grow a signedness at every construction and
> every match to answer a question only arithmetic asks. The instruction was right, which is
> also where LLVM puts it. The cheap fourth option — let bare `Add` mean "signed", add
> `UAdd` beside it, churn no tests — was the trap: an opcode whose name hides its
> signedness is the same implicit assumption that caused the defect.
>
> **A census that only counts what the oracle flags cannot see false positives** (wave 174).
> `zz_census` compares chiero against UBSan and can only ever report rows where gcc said
> something. The worse half of this wave's defect — unsigned wraparound reported as signed
> overflow, on a program gcc runs clean — is invisible to it by construction, and was found
> by asking what the *code* assumed rather than by reading the table. Measure both
> directions or the measurement flatters you.
>
> **When a mutant dies only on a golden, say so** (wave 174). `addr-arith-marked-signed`
> is killed by `every_corpus_c_file_matches_its_lowered_golden` and by no behavioural test,
> because overflowing 64-bit address arithmetic needs an astronomical index. That is an
> honest pin, not a bad one — but it means a routine re-bless would accept the defect
> silently, so it belongs in the notes rather than in the killed column without comment.
>
> **When the fix is unsound, the wave's product is the reason** (wave 173). The RED asked
> for three UB rules; two turned out to need signedness CIR does not carry, and implementing
> them from the bits would report every `unsigned x << 31` as undefined. The wave delivered
> the one checkable rule, a *test that pins the other two as not-checkable*, and the 020
> decision that would unblock them. **A test asserting that something is not detected is
> worth writing when the reason is structural** — a mutation adding the rule back dies on it,
> so it speaks the day the structure changes.
> **An existing test contradicting your new one is evidence, not an obstacle** (wave 173).
> `Shl by width-1 is ordinary code` failed the moment 6.5.7p4's overflow rule went in. It was
> right — for `unsigned` — and the conflict was the discovery. **Check who is right before
> updating either.**
> **An architecture gate can tell you a plan is impossible** (wave 173). `check-deps` refused
> `chiero-lower` → `chiero-check`, and rule 7 refuses the mirror image, so the discard-pile
> cross-check cannot live in either crate. That is not a formality to route around; it says
> the channel needs a home neither has.
> **A rule about a derived value must be tested on the derivation** (wave 172). C11 6.3.1.4
> is undefined when the *integral part* does not fit, so the range test belongs on the
> truncated value — `(int)-2147483648.5` is legal and `(unsigned)(-0.5)` is too. Three
> mutations survived until fixtures existed whose truncation *changed the verdict*; a value
> that is out of range both before and after truncation cannot tell the two readings apart.
> **When a spec says "the X of Y", write a fixture where X and Y disagree.**
> **A second variant of one operation is a second implementation** (wave 172). `FpToUi`
> shares `fcast` with `FpToSi` and shares none of its bounds — `2^bits` not `2^(bits-1)`, and
> a floor of zero rather than a negative. There was **no `FpToUi` fixture at all**, so two
> mutations to the unsigned rule survived. Same shape as wave 167's `f32`/`f64` split.
> **The endpoint past the endpoint** (wave 172). Fixtures at `INT_MAX` and `INT_MIN` do not
> catch an inclusive/exclusive slip; `2^31` does, and it is exactly representable as a double
> so there is no excuse for omitting it. **Test one step beyond each bound, not the bound.**
> **A false defect costs more than a missed one** (wave 171). The generator reported a
> mismatch that was two implementations of undefined behaviour disagreeing. Its own doc says
> a test that fails one run in ten "gets muted within a month" — a fixed-seed one that reports
> a non-defect spends the attention the channel exists to focus, *every run*, and trains its
> reader to skim. **When a detection channel reports something, its credibility is the asset;
> protect that before adding reach.**
> **Tightening a filter has an obvious cheat, so assert the negative** (wave 171). Any UB
> filter can be made to stop reporting mismatches by discarding more, and "no defects" is
> then achievable by deleting coverage. The test pairs the out-of-range conversion with an
> **in-range** one that must still be compared, and a mutation discarding everything dies on
> it. **A test that a filter catches X needs a companion that it does not catch Y.**
> **Discarding is about the oracle, not about caring** (wave 171). A program with undefined
> behaviour has no defined *value*, so gcc cannot arbitrate one — that is why it leaves the
> differential channel. It says nothing about whether chiero should *report* the UB, which is
> a separate job it mostly does. **Keep "cannot compare" and "do not care" apart**; conflating
> them is how a discard pile becomes an excuse.
> **Ask which layer already did the work** (wave 170). Lowering emitted `Add` over an int
> and a double, and the obvious repair — convert the operands — produced *three* chained
> casts, because 014 inserts the conversion itself under contract 11. The values were always
> right; only the instruction's opcode and declared type were read off one operand. **When an
> operation looks under-converted, check whether an earlier pass converted it** before adding
> a conversion of your own.
> **A binary operator has two operands and most bugs live in the other one** (wave 170).
> `is_float(lhs)`, `is_signed(lhs)`, `compare_ty(lhs)` — three separate reads of the left
> side, all wrong for `1 + d`, `1 < d`, `0 == d`. Fixtures written as `d OP 1` pass against
> code that never looks right. **Write asymmetric fixtures in both orders**, and for a
> *pair* rule (the wider type wins) make the two sides genuinely different.
> **Fixtures that agree under both readings test neither** (wave 169). Every float
> comparison fixture used 2.5 against 1.5, where `<` and `<=` give the same answer and so do
> `>` and `>=` — a mutation turning `FOLt` into `FOLe` survived the lot. **Equal operands are
> what separate a strict comparison from a non-strict one**, and the same omission covered
> four operators at once.
> **A new kind of value exposes shapes the old ones had all along** (wave 169). The panic in
> `chiero-mem` was a byte-splitting store extracting from a one-bit term — a `_Bool` is one
> byte holding one bit, and nothing about that is float-specific. The float path merely
> reached it first, because `(_Bool)f` is a comparison. **When a new type crashes old code,
> check whether the old types could have reached it too**; here the answer was yes and the
> fix belongs in the shared path, not the new one.
> **A catch-all that becomes unreachable is a hiding place** (wave 169). `cmp`'s
> `_ => return None` was load-bearing while floats were missing and became dead the moment
> the arms landed. Removing it makes a future `CmpOp` a compile error instead of a silent
> `None` the caller reads as "symbolic operand". **When completing a match, delete the
> catch-all the incompleteness needed.**
> **A `None` that means "wrong kind" meets a caller that reads it as "nothing"** (wave 168).
> `const_of` answers about *integer* constant expressions and returned `None` for
> `double g = 2.0;`; the caller turns `None` into `GlobalInit::Zero`, so the initializer was
> silently dropped and 37 of 200 seeds disagreed with gcc. Wave 149 recorded this exact shape
> — "follow the `None` to its handler" — and it recurred the moment a *new kind of value*
> existed for the old `None` to swallow. **When adding a kind, re-read every `Option` that
> was total before.**
> **Two channels, two halves of one feature** (wave 168). Three mutants die only in
> `differential.rs` and three only in `generated.rs`: the hand-written channel covers the
> shapes a person thinks to write (a literal, one operator, a cast) and the generator covers
> `v3++` on a float and a float global. Neither would have been enough, and the split is not
> a coverage gap — it is what having two channels is *for*.
> **Refusing can be the correct answer to a wrong answer** (wave 168). `(_Bool)f` lowered
> through `FpToSi` gives 0 for 0.5. That is not a missing feature, it is an incorrect one,
> and 015 §7's refusal is the right response until the engine has float comparisons.
> **Prefer a declared gap to a plausible number**, and say which of the two you are choosing.
> **Two precisions are two implementations** (wave 167). `f32` and `f64` compute at
> different widths and round differently, so a suite written entirely in `double` leaves half
> the arithmetic unchecked — a mutation making `FAdd` subtract in the 32-bit arm survived
> everything until single-precision fixtures existed. **When a type comes in widths, test
> each width**, with values that make the narrow one visibly narrow (`0.1f + 0.2f` is not
> the double).
> **A round trip can hide the bug it was written to catch** (wave 167). `SiToFp(-7)` read as
> *unsigned* is 4294967289, and truncating that back through `int` gives -7 again — so
> int→float→int passes either way. The signedness had to be asserted on the float, where the
> two readings differ by four billion. **A conversion pair can be wrong in both directions
> and right in composition.**
> **The blocker is often narrower than the summary says** (wave 167). §9 recorded floats as
> milestone-sized because the solver has no float sort — true, and true only of *symbolic*
> floats. The generator's 293 refusals are all closed programs where every float is concrete,
> and concrete evaluation is `from_bits`, an operator, `to_bits`. **Before deferring a
> capability as too large, ask which part of it the blocked work actually needs.**
> **A soak that reports only its verdict hides its census** (wave 166). Six hundred fresh
> seeds found zero defects, and the useful output was the breakdown: 293 refused for floats,
> 226 discarded as undefined, 81 compared. "0 defects" alone would have read as reassurance
> about a channel running at 13% duty. **When a search comes back empty, report what it
> searched** — the shape of the discards is the finding.
> **An audit that finds nothing is a result, and says so** (wave 166). 021 §5/§6 were read
> claim by claim against the code and came back clean, including the one the spec calls its
> highest-value failure mode. That is worth committing and recording: the next reader should
> not spend the wave re-deriving it, and "no defect found" is the honest report when it is
> the true one. **Do not manufacture a red to keep a streak.**
> **Do not build a test on the machine being slow** (wave 165). The obvious test for
> `:timeout` is to hand z3 something hard and watch it give up — and it measures the box and
> the z3 build, not chiero. (The 64-bit semiprime factorisation it would have used answers in
> 241ms here.) Such a test passes today and fails on faster hardware for a reason nobody can
> act on. What was actually contracted is that chiero *tells the solver its budget*, and a
> fake that records its input observes that on any machine in milliseconds. **A recording
> stub beats a timing assertion whenever the contract is about what was said.**
> **A relationship between two numbers belongs in a unit test** (wave 165). "The solver's
> budget expires before the watchdog" is arithmetic, and the integration fixture could only
> observe it at the one configuration the environment happened to have — the mutation that
> broke the *other* edge (an unbounded watchdog) survived, because no fixture could set
> `$CHIERO_SMT_TIMEOUT=0` without racing every other test on a process-global variable.
> **When a rule is arithmetic, test the arithmetic**, at both edges and several scales.
> **The test *count* catches what the failure list misses** (wave 165). A full-suite run
> straight after a mutation sweep reported 1196 passing and 2 failing with no failure output
> captured; three runs since report 1198 and 0. 1198 is the arithmetic of what the wave added,
> so the count is what identified the run as reading a binary the sweep had just invalidated —
> waves 135 and 141's hazard again. **Know what the number should be.**
> **An artifact for a third party is not the bytes you sent** (wave 164). Dumping the wire
> traffic looks like the honest implementation of "writes every query" and produces files
> that only replay in session order, because a long-lived process is told each declaration
> once. What the contract wants is a *standalone* script, which has to be rebuilt. **Ask who
> reads the artifact and what they will have**; here they have z3 and nothing else.
> **"Matched nothing" is a verdict the sweep has to be able to give** (wave 164). Three
> mutations aimed at Rust string literals containing `\n` failed to match through two rounds
> of perl escaping. Because the harness compares an md5 before and after, they were reported
> as MATCHED NOTHING rather than as survivors — which would have read as three tests that
> do not check anything. **A sweep that cannot tell "no change" from "no effect" reports the
> opposite of the truth.**
> **A derived `Default` can switch a subsystem off** (wave 163). Adding a
> `timeout: Duration` field to a struct that derives `Default` compiles, reviews clean, and
> makes every backend query time out instantly, because `Duration::default()` is zero. The
> fix is a newtype whose `Default` is the real constant. **When adding a field to a derived
> `Default`, ask what the zero value *means* in that position** — for a duration, a
> capacity, or a limit, zero is usually "off".
> **Put the blocking somewhere you can abandon** (wave 163). There is no portable deadline
> on a blocking pipe read, so no amount of care in the reader adds one. Moving the read to a
> thread and waiting on a channel turns an unbounded wait into a bounded one without the
> reader knowing anything about time. **A deadline is a property of the waiter, not of the
> read.**
> **Retry the failure that is transient, not the one that is expensive** (wave 163). The
> backend path already restarted and re-asked on a dead process, which is right for a crash.
> Applying it to a timeout would spend the budget twice for the same answer. **Before reusing
> a recovery path, ask what it assumes about the failure.**
> **Read the rest of the sentence** (wave 162). 022 §4 says "Backend selection order:
> `$CHIERO_SMT_SOLVER`, then `z3`, `cvc5`, `bitwuzla` on `PATH`. **Recorded in the result so
> a finding says which solver decided it.**" The first half was implemented and the second
> greps to nothing. Wave 161 found the same shape one clause earlier in the same section.
> **A spec sentence with two clauses is two contracts**, and the implemented half is what
> makes the missing half hard to notice — the code looks like it is about that sentence.
> **A test that compares two derived values proves only that they derive alike** (wave 162).
> `assert_eq!(result.solver(), b.name())` passes whatever `name()` returns, because both
> sides move together; a mutation making it the full path survived. The fix is to assert the
> *property* — that a name has no path separator — not just the agreement. Same lesson as
> waves 154 and 159 in a third guise: **a self-consistent assertion is not a check.**
> **A missing interface reds as a compile error, and that is allowed** (wave 162). This
> project's harness rules say a mutant that does not compile is inconclusive — because there
> the compiler is refusing a hypothesis about code that exists. A test naming an interface
> the spec requires and the code lacks is the ordinary first red of TDD. **Distinguish "the
> compiler rejected my experiment" from "the compiler stated the absence".**
> **When three owed entries all say the same thing, the entry is not the bug** (wave 161).
> §9 carried three items — the `fork_on_offset` survivor, an undecidable divisor, 023 c17 —
> each describing a real limitation, each written as though the limitation were the defect.
> Probing them one at a time showed all three behaved correctly the moment a backend was
> configured, and the question they had been hiding for eight waves was *why a backend was
> never configured*. 022 §4 had said tier 2 is on by default since before any of them was
> written. **A cluster of owed items with one shape is evidence about their common cause,
> not three things to fix.**
> **A default applied in three places is a default applied in two** (wave 161). Three
> solver-construction sites each read `self.backend` independently; adding discovery to the
> obvious one left the test still failing. The fix was one `backend_for_run` that all three
> go through. **When adding a policy, find every site that implements the old one before
> writing the new one.**
> **An accidental default is a silent premise in every test that relied on it** (wave 161).
> Five tests were about tier 1's incompleteness and none of them said so — they simply got
> tier 1 because nothing else was available. Making discovery the default broke exactly
> those five, which is the right blast radius: each now asks for `LiteOnly` and thereby
> states its own subject. **A test that depends on a default depends on it silently.**
> **A degradation can be *answered*, and then it should go** (wave 160). Fidelity is a
> running worst-of and that is right for almost everything — an unmodeled call stays
> unmodeled. But "the solver could not decide this branch" is a claim the run can later
> refute: a validated model of the path condition proves the path exists, and 022 §3.1 makes
> `Sat` self-certifying, so the proof is in hand. Leaving the caveat labels a reproducible
> fault `Unknown`, which is the label a reader uses to decide what to ignore. **Ask of each
> caveat whether anything later in the run could settle it** — most cannot, and the one that
> can was carrying every witnessed finding down with it.
> **A fixture with two reasons to fail tests neither** (wave 160). The RED used a null
> dereference, which degrades for the branch *and* for `IntToPtr ... no provenance`; it
> could never reach `Exact` however right the fix was. Splitting it — an out-of-bounds write
> for the discharge, the null store for "what the proof did *not* answer" — turned one
> muddled assertion into two sharp ones, and the second killed two mutants that had survived
> the whole suite.
> **A channel that finds nothing has still measured something** (wave 160). ~1300 generated
> symbolic paths produced zero mismatches. That is not a wasted wave: it is the first
> evidence that the value semantics agree with gcc under symbolic execution, and the skip
> *reasons* — not the failures — are what pointed at the defect this wave fixed. **When a
> soak comes back clean, read the discards.**
> **Two parties each hold half a fact; the join has to be explicit** (wave 159). The engine
> proves a divisor can be zero and knows the condition; the checker decides the event is
> worth reporting. Neither can attach one to the other, and there is no clever way around
> that — the condition has to *travel*, on the event and then on the report. The temptation
> is to have the engine guess which report belongs to which event by timing, which works
> until two checkers report on one instruction. **When the knowledge is split, move the
> data, not the inference.**
> **A new variant beats a new field when most callers have nothing to say** (wave 159).
> `Action::ReportRequiring` is separate from `Report` because a null dereference needs
> nothing beyond the path, and folding them would make every existing checker declare an
> empty requirement. The additive variant left `UnionPun` and `OrderDependence` untouched.
> **Narrowing what is *reported* is not narrowing what is *run*** (wave 158). The witness
> for `100 / (x - 42)` needs `x == 42`, and the one-line way to get it is to push that onto
> the path condition — which then refutes every later branch the value contradicts, so a
> finding would delete the execution it was reporting on. 023 contract 19 already draws this
> line for a checker's `Assume`, which *does* join the path and degrades fidelity for it.
> **Before adding a constraint, ask whether it is about the program or about the report**;
> the two want different fields and only one of them is a lie if you pick wrong.
> **Keep the term, not the model** (wave 158). The solver call that proves a fault feasible
> hands back a model, and storing it is the obvious fix. It answers for the path *as it was
> at that instruction*, and the state runs on — a later branch can refute it. The condition
> stays true; the assignment satisfying it may not. **When a fact and a witness to it are
> both available, keep the fact.**
> **Establish that the mechanism is missing before building one** (wave 157). The obvious
> reading of "no checker reports UB" is that the engine needs a new `Event` variant to
> announce it. It does not: `Event::AfterInst` already carries `&State`, and `ub_events()`
> is public, so a checker could always have seen them. Ten minutes with a scratch checker
> settled it and saved an enum variant, an emit site and a migration. **Probe the seam
> before widening it.**
> **A comment that credits the wrong mechanism is a defect the tests cannot catch** (wave
> 157). `UndefinedArithmetic`'s doc said the cursor prevented re-reporting on every
> instruction; mutation froze the cursor and nothing failed, because the key-based
> deduplication was doing that work all along. The code was right and the explanation was
> wrong — and the explanation is what the next reader will change the code from. **When a
> mutation survives, re-read what the surviving line was documented to do.**
> **A recorded decision is still worth re-reading against the case in front of you**
> (wave 156). `note_ub` skipped every symbolic operand with a stated reason: the query costs
> "one per arithmetic instruction", which is 040's business. The reason is sound — for
> `Add`/`Sub`/`Mul`. It was written once, for the general case, and then applied to division,
> where a query is *one per division* and divisions are rare. **A comment explaining why
> something is not done is evidence, not a verdict**; check whether its argument covers the
> instance.
> **A mutant that changes nothing is a question, not a result** (wave 156). Removing the
> early return on a constant divisor survived every channel — and the reason was not that
> the tests were weak but that the guard was *hiding a defect*: the concrete path needs both
> operands constant, so `x / 0` with a symbolic numerator matched neither, and the guard
> sent it home. **When a mutation is invisible, ask what the code would do without the line
> before concluding the tests are at fault.**
> **Build the mutant before believing its verdict** (wave 156). One mutation this wave did
> not compile, and the sweep read the failure as a *survivor* rather than as nothing. The
> sweep now builds first and reports INCONCLUSIVE — which the harness rules have said since
> wave 112 and the script did not implement.
> **A capability that widens breaks the tests that depended on the limit — expect it, and
> ask what the fixture was really asserting** (waves 153, 154, 155). Eight instances across
> three waves now: a slicing test that needed the backend, a `pinned` flag that meant "the
> model assigned it", a "full binary tree" built from conditions that entail each other, and
> four fixtures whose subject is *an undecidable query* and which had all chosen a product to
> get one. The repair is never to re-record the golden. It is to ask what property the
> fixture existed to state and rebuild the input so it states it again — `x*y < 7` became
> `x*y == 7`, which no diagonal reaches; `i*j` as an offset became `i%j`, which no diagonal
> can push out of bounds. **When a fix makes a test fail, the first question is whether the
> test's premise was ever true.**
> **A default that is invisible with one attempt can be fatal with many** (wave 155). Model
> completeness (022 §2) filled unnarrowed variables with zero. Harmless while there was a
> single candidate; with sixty-four it meant sixty-four attempts all proposing the same zero
> — and that was exactly the case the search was built for, since a `switch` default narrows
> nothing at all. **When turning a single step into a loop, check every constant the single
> step was allowed to have.**
> **A new capability can make an old inefficiency reachable** (wave 155). `least` had always
> been a linear scan over `lo..=hi`; wave 154's single-bit mask pinning made a *high* bit
> reachable and turned it into twenty-two seconds. The scan was not new, was not wrong, and
> had never mattered. **After adding a way to produce a value, ask what already consumes it.**
> **A narrowing that is too strong is invisible until it refutes something** (wave 154).
> Four mutants survived the first sweep and all four had one shape: an over-strong domain.
> It cannot produce a wrong `Sat` — the model validator catches that and the answer degrades
> to `Unknown` — so **no test built from satisfiable inputs can see it**. It is fatal only
> where nothing validates: the domain empties and a satisfiable set is *refuted*. The tests
> that killed them are all "this set has a model, so `Unsat` is the wrong answer", which is
> 022 §3.1's asymmetry used as a test-design rule instead of quoted. **When testing a
> pruning step, write the case where pruning too much is the failure.**
> **A satisfiability check cannot pin a polarity** (wave 154, and 153 before it). Both `p`
> and `!p` have models, and a validated model is accepted either way — so a test that asks
> "is there a model" cannot tell a relation from its complement. Twice now the discriminator
> has been **which side of the bound the model lands on**: assert the value, not the verdict.
> **`git checkout` restores a file, not an edit** (wave 154). Reverting a one-line debug
> `eprintln` that way discarded the wave's uncommitted implementation work in the same file.
> Wave 153 learned this about mutation restores; it is the same rule for debugging. **Edit a
> debug line back out, or commit before adding one.**
> **A fixture can be wrong in a way only a better tool reveals** (wave 154). `strategy.rs`
> built a "binary tree of 2^depth leaves" whose nodes branched on `x <s i` — conditions that
> *entail each other*, so the tree was never full. It looked full only because the solver
> explored the infeasible paths. The recorded leaf order was therefore a golden of an
> exploration that could not happen. **When a capability improves and a golden changes, ask
> whether the golden was ever right**, rather than re-recording it.
> **A channel finds what it is shaped to find, and every channel had one shape** (wave 153).
> Four detection channels, thousands of programs, and all of them ran `int probe(void)` —
> closed and concrete. So no path condition ever contained a negated comparison, and the
> solver could decide exactly one side of every branch in every program for as long as
> anyone had been looking. The defect was not subtle and not deep; it was **outside every
> channel's shape**. When a channel stops finding things, ask what it cannot express before
> concluding there is nothing left.
> **A fix that widens a capability breaks the tests that depended on the limit** (wave 153).
> Two, and neither was a stale fixture. The slicing test asserted the engine slices, which
> only happens en route to the *backend* — a stronger in-process solver meant it never got
> there. And `pinned` meant "the model assigned it", which was only ever equivalent to "the
> path constrains it" because backend models are partial; a complete model made every input
> report as pinned, including ones nothing on the path names. **After widening what a
> component can do, re-read what its callers were inferring from what it could not.**
> **An oracle that skips honestly can skip everything** (wave 153). The symbolic oracle's
> first version skipped every path whose `Fidelity` was not `Exact`, correctly applying
> 023 §7's rule that a declared limit must not be read as a defect — and compared *nothing*,
> because the engine declares `Unknown` on any branch the solver cannot decide. "Both sides
> explored" is a conservative over-approximation, not a wrong value: an unreachable path
> comes back `Unsat` and an unmodelled value returns no scalar, so neither needed `Fidelity`
> to catch it. **Check what a skip rule excludes on real input before trusting it**, and
> prefer a rule that names the specific failure to one that keys off a summary.
> **An interaction between two rules cannot be fixed by copying arms** (wave 152). `'\u00E9'`
> is 50089 because a UCN in a *plain* constant becomes two UTF-8 bytes **and** two bytes make
> a multi-character constant — two paragraphs of the standard meeting only in the byte
> sequence. The third decoder could have grown a `\u` arm and still have been wrong, because
> it turned an escape straight into a value and had nowhere to hold the intermediate state.
> **When two rules compose through a representation, share the representation, not the
> arms.** The tell that this was one rule and not three: a single mutant
> (`plain-takes-first-unit`) changed three fixtures at once.
> **A "third copy" is worth looking for the moment you find the second** (wave 152). Wave
> 151's own note said character constants still had their own reading, and probing it took
> ten minutes and found five defects. The note is why — an owed entry written *while the
> context is loaded* is worth more than the same discovery made cold two waves later.
> **A mutation sweep needs a control run** (wave 151). A sweep whose restore step silently
> failed reported all six mutants KILLED — the mirror of wave 146's false survivor, and far
> harder to see, because all-killed is exactly what a *good* sweep reports. The restore was
> `git checkout -- <paths>` with one path an untracked new file; git rejects the whole
> pathspec when any element is unknown, so nothing was restored and the mutants stacked.
> **Every sweep gets an unmutated control that must SURVIVE**, plus an md5 check that the
> mutation changed the file and a `git diff --quiet` that the restore worked. This is the
> existing harness rule ("never `git checkout`") earned a second time, the expensive way.
> **Two channels that kill the same defect are not redundant** (wave 151). The first sweep
> had three mutants dying only in the unit tests and surviving the gcc oracle — gcc had
> never been asked about a greedy hex escape, a three-digit octal, or an astral code point.
> Fixtures were added and they die in both now. Keeping both is deliberate: the unit test
> names the property in its own units, so its failure says *which rule* broke, while the
> differential says only which integer differed — but only the differential is checked
> against something that does not share the implementation's assumptions.
> **An unkillable mutant names a missing fixture, so record it rather than dropping it**
> (wave 151). Wave 150 could not kill `char16_t`'s signedness: it is observable only above
> 32767 and no fixture could reach one. Wave 151's `u"\uFFFF"` is 65535 and killed it for
> free. An equivalent mutant is a statement about the *suite*, not only about the code.
> **Two readings of one input will disagree; the fix is one reading** (wave 151). sema sized
> a string literal by counting source characters while lowering filled it by re-scanning the
> escapes, so `"a\nb"` was five elements and four bytes at once. Adding a `\u` case to the
> decoder would have fixed one of them. **When two passes must agree about a derived value,
> derive it once and have both consume it** — here one function returns the element list,
> sema takes its length and lowering takes its values, and there is no invariant left to
> maintain. The sharper form of wave 150's rule: not "keep the copies in step", but "have
> one".
> **A fix that lands in one of two copies is worse than no fix** (wave 150). `string_bytes`
> and `raw_expr`'s `Str` arm both built a literal's bytes; teaching one about element widths
> made `sizeof(L"AB")` 12 while the object behind it stayed 3 bytes — the size and the
> storage now *disagreeing*, which is worse than both being wrong together. **Grep for the
> second copy before changing the first**, and prefer routing one through the other to
> leaving them parallel.
> **Probe the owed list before picking from it** (wave 149). Three of its entries were
> stale and one was wrong in both directions — it claimed designated initializers were
> *refused* when they work for locals and were silently *zeroed* for globals. An owed list
> is a map for the next reader, and a stale entry sends them somewhere that no longer
> exists. Ten minutes of probing found a live wrong-answer defect that the entry's own
> wording had been hiding.
> **"Refused, nothing invented" is only true if something actually refuses** (wave 149).
> `encode_into` returned `None` under a comment reading "refused whole rather than silently
> written in positional order" — and the caller maps `None` to `GlobalInit::Zero`. The
> comment described the intent; the code delivered the opposite. **Follow the `None` to its
> handler** before believing a refusal is one.
> **A branch added in front of a conversion inherits nothing** (wave 148). Wave 140 put
> `convert_for_store` before the store in `init_list`; wave 142 added the bit-field branch
> *above* it and the new path silently skipped it. Two correct changes, in order, produced
> a gap. It is §9's "a guard added to one arm is not added to its sibling" with the sibling
> added later — so **when adding an early-return branch, check what the code below it does
> that you are now skipping**, and prefer hoisting the shared step above both.
> **The ledger is a debt register, and entries are meant to be deleted** (wave 148). The
> bit-field entry said "owed rather than tolerated", survived two waves, and removing it in
> the same commit as the fix is how the ratchet records that the debt was paid. An entry
> that never changes is either a decision or a lie; either way it should be re-read.
> **`cargo fmt` moves anchors — and a missed anchor can fake a *green*** (waves 146, 147).
> Wave 146's missed anchor produced a false *survivor*; wave 147's produced a false *pass*:
> the ratchet assertion was never inserted, so the run printed "refused 101" and went green
> with nothing checking them. Clippy's `never used` on the helper is what caught it. **After
> inserting a check, confirm it can fail** — a new assertion that has never been red is
> indistinguishable from no assertion.
> **A test written for a guard can be equivalent under the guard's own mutation** (wave 146).
> `the_shrinker_refuses_to_reduce_what_does_not_fail` used an always-false predicate — under
> which the reducer keeps nothing anyway, so deleting the guard changed nothing and the
> mutation survived. The fixing shape is a predicate **false for the original and true for a
> reduction**. Ask what the code would do *without* the guard, not just whether the test
> passes with it.
> **Point a new tool at a real failure before trusting it** (wave 146). Both defects in the
> shrinker — deleting the trailing `return`, and printing the pre-shrink verdict beside the
> post-shrink source — were found in the first live run and neither by reading the code.
> The second is the sharper one: the numbers are what a reader trusts most, and they were
> the part that was wrong.
> **Two tests failing for two different reasons is not confirmation** (wave 145). Under the
> reintroduced wave-141 defect, both tests in `verified.rs` went red — one because the CIR
> was invalid, the other because `harness::lower` panics on the *diagnostic* the new guard
> correctly produces. Only the second discriminates. **Check which test failed and why**
> before reading a red suite as evidence for the thing you were testing.
> **A mutation that changes the wrong thing is not evidence** (wave 144). The first attempt
> at "does the copy happen before the scope exit" deleted `exit_scope` instead of moving the
> copy past it, and survived — which says only that an unbalanced scope here is uncaught.
> Rewritten to actually reorder, it dies. **Re-read a survivor before believing it**: the
> question is whether the mutant expresses the property you meant.
> **A surviving mutation can be telling you the code is dead** (wave 143). A
> local-shadowing guard in `global_addr_of` survived deletion, and the reason was not a
> missing fixture: the function has one caller, on a path where no local is in scope, and
> the case it guarded is a program gcc rejects outright. The guard went, not a test in.
> **Ask first whether a survivor is unreachable** — a guard against a case that cannot
> arise is dead code wearing a confident comment, which is waves 107/112/118/124/132's
> hazard in its cheapest form.
> **A wrong write can be repaired by the next one** (wave 142). Widening a bit-field's
> written range by one bit survived `{15, 2}`, because the stray bit lands in the neighbour
> and the neighbour is stored immediately afterwards over the same bit; it survived `{7, 2}`
> because 7 in four bits has that bit clear. Only the **partial** initializer `{15}` shows
> it, where the neighbour gets nothing but the zero-fill. Two conditions had to hold at once
> — the extra bit set *and* nothing written after it — and neither alone was enough. When a
> mutation survives, ask what downstream write is undoing it.
> **A grammar production that never fires is a silent gap in the generator** (wave 141).
> Three new statement arms were added *after* the `0..=3` range arm and were unreachable.
> The only symptom was a suspiciously unchanged number — same programs compared, same zero
> mismatches as the run before. **When a generator is extended and the counts do not move,
> the extension is not running.** Print what the grammar actually emitted before believing
> a clean result.
> **Do not run a mutation sweep and a suite in the same tree** (waves 135 and 141 — recorded
> after 135 and then not followed). The sweep rewrites source while `cargo test --workspace`
> is building, and the suite compiles against a mutant. Run them serially; when a result
> surprises you, check `git diff` before believing it.
> **A defect that hides behind a working sibling is the last one anyone finds** (wave 140).
> `struct S { signed char a; int b; } s = {3, 5};` produced nothing, while `s.a = 3;` on the
> same struct worked perfectly — because sema converts an assignment *expression* and not a
> braced element. Every struct with a member narrower than `int` was affected, and the suite
> used `{ int a; int b; }` throughout. When a construct works one way and not another, the
> working spelling is what keeps the broken one invisible.
> **A name is not a scope** (wave 137). The first fix recorded enumerators in a
> `Symbol -> value` table; a mutation swapping `insert` for `or_insert` survived, because a
> function-local `enum { K = 2 }` and a file-scope `enum { K = 1 }` are both legal and both
> called `K` — by name the table keeps whichever came last. Keyed by `ExprId`, resolved
> where the scope is known, the question does not arise. When a lookup can be ambiguous,
> key it on the *use*, not the *name*.
> **A discriminator must differ under *both* readings — check it against gcc first**
> (waves 135, 136). Four precedence fixtures and two signedness fixtures looked like proofs
> and were not: two operators sharing a class parse left-associatively into the *same tree*
> whenever the tighter one comes second, and `+= 1` followed by truncation to the same width
> absorbs a wrong sign-extension entirely. `7 % 4 + 1` is 4 either way; `1 + 7 % 4` is not.
> `u.a += 1` cannot see the load's signedness; `u.a /= 2` can. Compute both readings by hand,
> run them through the compiler, and only then write the case down.
> **Mutate each *part* of a statement, not the statement** (wave 134, and wave 133 from the
> other side). `self.diags.truncate(before)` has two halves — that it fires, and where it
> stops. Deleting it was caught; changing `before` to `0`, which erases diagnostics from
> *other declarations*, survived the first draft of the very test written to pin it.
> **A test for code that is already right is worth committing when the mutation is the RED**
> (wave 134, wave 125). The parser's rollback was correct and had been for its whole life;
> deleting it left 1114 tests green. Correct-and-unpinned is one careless edit from
> wrong-and-silent.
> **Mutation finds what review cannot** (wave 133). Two adversarial reviews and a green
> 1113-test suite all passed over a comparison rule keyed on the left operand only. The
> mutation that dropped the right-hand test survived — and that survival was the only signal
> anything was missing, because `0 == p` is the same C as `p == 0` and no reader thought to
> write both. **Mutate every branch of a predicate you just wrote**, not the predicate as a
> whole: `A || B` needs a fixture that fails when B alone is deleted.
> **Chase the reported defect one level down before believing its scope** (wave 133). §9 had
> "`_Bool` truncates" as one wrong answer. Probing the shapes around it found that pointer
> null-testing — the most common defensive idiom in C — did not work at all. The recorded
> defect was true and was the small half of what was there.
> **A commit message that names four shapes and a test that covers two is a lie you will
> believe** (wave 132). 998d999 listed "a by-value argument" among the shapes its guard
> fixed. No fixture passed a struct by value, and it was *wrong* there — and worse than
> before, because the pre-change failure was a dead state and the post-change one was a
> plausible number. Write the fixture for every shape the message claims, or drop the claim.
> **The predicates that answer the same question must be one function** (wave 132). Four
> places asked "is this an aggregate": `is_aggregate`, `cty`, `aggregate_size` and
> `aggregate_size_of_ty`, and three of the four included `Ty::Vector`. The odd one out is
> the one every read site called. Two questions were hiding in one name — "moves by copy"
> and "names its address" — and a function designator is the second but not the first.
> **Instrument the value, not the theory** (wave 132). Six waves argued about *where* the
> wild pointer came from. One `eprintln!` of what `dst` and `src` actually evaluated to in
> the engine's `CopyMem` handler ended it in a single step — `src` was a wild pointer whose
> offset was the struct's own bytes, which named the defect immediately. Printing the CIR
> next showed `load ptr` in black and white. Neither probe took five minutes.
> **A guard added to one arm of a match is not added to the other** (wave 132, and wave 121
> from the other side). The global ident arm had the rule *and the comment explaining it*;
> the local arm had neither. Grep for the sibling every time.
> **The predicate a comment describes is usually narrower than the one it is spelled with**
> (wave 132). "An array names its own address" was implemented as `matches!(ty, CTy::Ptr)`,
> which is also every pointer. Fixing one arm forced the narrower predicate, which then
> exposed the other arm as wrong — the fix found the bug.
> **An empty log means "not reached" — then ask *why* not** (waves 130–131).
> **A real defect found while hunting another is still not the cause** (wave 129).
> **Eliminating halves is progress worth committing** (wave 128).
> **Build the hand-built equivalent** to decide whether a bug is in the IR or in what
> consumes it (waves 109, 128).
> **An ABI change has to reach the declaration pass** (wave 127).
> **The reporting you built pays off on defects you did not anticipate** (wave 126).
> **Mutation is what makes "already correct" worth committing** (wave 125).
> **A comment claiming a property is not the property** — waves 107, 112, 118, 124, 132.
> **A defect can hide behind another of the same shape** (wave 123).
> **A corpus fixture that runs is coverage; a mutation needs something sharper** (wave 122).
> **A fix does not generalise to a second code path on its own** (wave 121).
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120) — discharged in wave 132: the
> directory is empty and every defect it motivated has a gcc-differential test.
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause** — print diagnostics before 015 §7 truncates.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113) — defect 4 above is the sharpest
> instance yet: it returned 28663 with no finding at all.
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; an **ambiguous anchor is inconclusive too**; two guards only
> ever true together are equivalent mutants; a no-op mutant is neither; **`cargo fmt` moves
> anchors** and reflows `#[ignore = "…"]` onto one line; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok`**. An oracle that can silently not run is not
> an oracle — **announce every skip**, and wave 132 made `differential.rs` enforce that
> rather than intend it.
>
> **Also owed, found in wave 141**: lowering does not run the CIR verifier, so a function
> it emits with invalid CIR is *not* refused. `*(&a[i] + 0)` produced `zext i32 %v to i64`
> on a `Ptr`, which the verifier rejects — and the module was emitted anyway and the engine
> produced no state. 015 §7 refuses what lowering knows it cannot represent; this is the
> class it does not know about. Running `verify` at the end of `function()` and refusing on
> an error would turn every such defect from a silent nothing into a diagnostic.
>
> Owed and written down — **no open defects**, these are gaps and deferrals. Each was
> re-probed in wave 149 and three were stale, so treat this list as checked-on-2026-07-29
> rather than as accumulated folklore:
>
> - **floats do not execute** — now *declared*: `refuse_floating` refuses any function
>   mentioning a floating type (wave 147), and `KNOWN_GAPS` in `generated.rs` is what fails
>   when that is implemented.
> - ~~`L`/`u`/`U` string literals lose their element width in `unquote`~~ — **fixed in wave
>   150**, and `unquote` was never the culprit: it strips the prefix correctly and the
>   *type* was what nothing asked for. Third owed entry in two waves whose wording pointed
>   away from the defect.
> - **Universal character names are not implemented.** `u"\uFFFF"` is read as the five
>   literal characters `u F F F F`, so the array has 5 elements where C has 1 and the first
>   reads 117 instead of 65535 — a *silent wrong answer*, found while mutation-testing wave
>   150. `unescape` returns `Vec<u8>` and would need to return characters for this. Fixing
>   it also buys the one mutation wave 150 could not kill: `char16_t`'s **signedness** is
>   unpinned because no fixture can reach a value above 32767 without it.
> - the wave-117 `fork_on_offset` survivor; a fault in a non-entry frame is untested;
>   `Bits` path steps are not emitted; `tests/corpus/c/pointer_fields.c` is not written;
>   010's 18, 011's 12 and 012's 17 are deliberately uncovered.
> - ~~designated and bit-field initializers refused~~ — **wrong in both directions.** They
>   work for *locals* and always have; at file scope they were not refused but silently
>   zeroed. Fixed in wave 149.
> - ~~`typeof` types to `Ty::Error` in sema~~ — **stale.** `typeof(x) y = x + 1;` computes
>   the right answer today.
>
> **013 is clear** — 20/20 contracts covered, no owed items.
>
> ### Earlier (wave 128, `fb966c2`) — 1100 tests, 5 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: a `return` unwinds its scopes twice
>
> Wave 128 found no fix and narrowed the sret bug to one suspect. **Start here, not at the
> engine and not at the call ABI** — both were eliminated:
>
> - **Not the engine.** A hand-built module with the same shape (`mk(sret, a)` writing
>   through the pointer, copying into it, returning it; caller allocates, calls, copies out)
>   runs correctly and reports nothing. sret binding, `CopyMem` through a parameter, and
>   returning a parameter all work.
> - **Not the visible half of lowering.** The lowered caller is instruction-for-instruction
>   the working hand-built one, and the callee emits `copymem %3 -> %16, 8i64` then `ret %3`.
>
> **What is left**: the lowered callee ends with **four** scope exits —
> `.scope exit 1`, `.scope exit 0`, `.scope exit 1`, `.scope exit 0`. The hand-built module
> has none, and the duplication is real. Wave 109 added an `exit_scope` after the body so a
> function falling off the end closes its parameter scope; a body ending in `return` has
> already unwound via `unwind_to(0)`, so that trailing exit runs a **second time**. 021
> retires stack objects on `Scope(Exit)`, and retiring twice is the live suspect for an
> object that later resolves to "no known object".
>
> Reproduce with `cargo test -p chiero-recipe --test no_spurious_findings -- --ignored`.
> Check `unwind_to` against the trailing `exit_scope` in `function()`, and confirm whether
> `exit_scope` on an already-terminated block emits into a live one.
>
> ### Blocked on it
>
> `a_struct_returned_by_value_carries_its_fields`, `two_aggregate_returns_are_distinct`,
> and `tests/corpus/owed/header_inline.c`.
>
> ### Shapes still untried
>
> - **A union inside a struct** under a symbolic index.
> - **`goto` out of three nested scopes**, where 021 must retire objects the jump skipped —
>   related to the suspect above, and worth writing *after* it is understood.
> - **A `switch` whose scrutinee is a struct member** read through a pointer.
>
> ### Also open
>
> - Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>   untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **Eliminating halves is progress worth committing** (wave 128). A wave that produces no fix
> but rules out the engine and the call ABI has narrowed the next attempt from three
> subsystems to one suspect line. Write down what was *excluded* and how, or the next attempt
> re-walks it.
> **Build the hand-built equivalent** — it is the fastest way to decide whether a bug is in
> the IR or in what consumes it (waves 109, 128).
> **An ABI change has to reach the declaration pass** (wave 127).
> **The reporting you built pays off on defects you did not anticipate** (wave 126).
> **Mutation is what makes "already correct" worth committing** (wave 125).
> **A comment claiming a property is not the property** — waves 107, 112, 118, 124.
> **A defect can hide behind another of the same shape** (wave 123).
> **A corpus fixture that runs is coverage; a mutation needs something sharper** (wave 122).
> **A fix does not generalise to a second code path on its own** (wave 121).
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120).
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause** — print diagnostics before 015 §7 truncates.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — twenty waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors** — it also reflows `#[ignore = "…"]`
> strings onto one line, which breaks an anchor written as the wrapped form; **a mutation one
> other site compensates for is partial**; **some changes are not expressible as a one-line
> mutant** — say so; **check a patch script printed `ok` and that its anchor was unique**. An
> oracle that can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: a `return` unwinds its scopes twice (the sret suspect, blocking two
> tests and `tests/corpus/owed/header_inline.c`); the wave-117 `fork_on_offset` survivor;
> designated, bit-field and address initializers refused; a fault in a non-entry frame is
> untested; `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in sema; the
> parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U` string literals
> lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately
> uncovered.
>
> ### Earlier (wave 127, `b626d22`) — 1100 tests, 5 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: the engine does not run an sret return
>
> Aggregate returns **lower** correctly as of wave 127 — hidden first parameter, one
> caller-owned slot per call site, callee copies through it. The golden shows
> `copymem %2 -> %15` in the callee and `call @mk(%16, 3i32, 7i32)` in the caller. The run
> still reports:
>
>     wild-pointer: access through a pointer at address 30064771075
>
> `30064771075` is `0x700000003` — the struct's own bytes `{3, 7}` used as an **address**.
> Something reads the aggregate's contents where it should read the pointer. Start by
> tracing what `%16` is bound to in the callee's frame versus what the caller's `CopyMem`
> takes as its source.
>
> A strong hint sits in wave 127's own mutation log: **appending** the sret slot instead of
> prepending it survived the shape test, and appending binds an `i32` where a `ptr` is
> expected — the same shape as this bug. Check the engine's parameter binding for an
> off-by-one against the hidden parameter.
>
> Blocked on it: `a_struct_returned_by_value_carries_its_fields`,
> `two_aggregate_returns_are_distinct` (both `#[ignore]`d in
> `chiero-recipe/tests/no_spurious_findings.rs`), and `tests/corpus/owed/header_inline.c`.
>
> ### Shapes still untried
>
> - **A union inside a struct** under a symbolic index.
> - **`goto` out of three nested scopes**, where 021 must retire objects the jump skipped.
> - **A `switch` whose scrutinee is a struct member** read through a pointer.
>
> ### Also open
>
> - Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>   untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **An ABI change has to reach the declaration pass** (wave 127). Three sites had to agree —
> definition, call, and *declaration* — and only the verifier's `CallArity` caught the third.
> When a signature changes, grep for every place a signature is built.
> **The reporting you built pays off on defects you did not anticipate** (wave 126).
> **Mutation is what makes "already correct" worth committing** (wave 125).
> **A comment claiming a property is not the property** — waves 107, 112, 118, 124.
> **A defect can hide behind another of the same shape** (wave 123).
> **A corpus fixture that runs is coverage; a mutation needs something sharper** (wave 122).
> **A fix does not generalise to a second code path on its own** (wave 121).
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120).
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause** — print diagnostics before 015 §7 truncates.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — twenty waves, and
> wave 127 again: the sret shape test checked the argument *count* and the parameter *type*
> but never which argument was the slot.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok` and that its anchor was unique**. An oracle
> that can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: the engine does not run an sret return (blocks two tests and
> `tests/corpus/owed/header_inline.c`); the wave-117 `fork_on_offset` survivor; designated,
> bit-field and address initializers refused; a fault in a non-entry frame is untested;
> `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in sema; the parser's
> speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose
> their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately
> uncovered.
>
> ### Earlier (wave 126, `190017c`) — 1097 tests, 3 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: an aggregate return has nowhere to live
>
> `return p;` where `p` is a `struct` yields `addrlocal` of the **callee's** stack slot,
> whose scope exits on return — so the caller copies from bytes that are already dead:
>
>     uninitialized-read: read at offset 0 of p touches bit 0, which was never written
>     through p.lo
>
> 015 §2 says an aggregate return is memory; it must be memory the **caller** owns. The usual
> shape is an **sret slot**: the caller allocates, passes its address as a hidden first
> argument, the callee writes through it and returns nothing. That touches lowering's call
> path, its return path, and the engine's frame setup together — a wave of its own.
>
> Every VPP accessor in a header returns a struct by value, so this is not a corner.
> `tests/corpus/owed/header_inline.c` (+ `pair.h`) is written and waiting.
>
> ### What wave 126 fixed
>
> `struct pair p = f();` **stored the returned pointer** into `p`'s slot instead of copying
> the struct, so `p.lo` read the low half of an address as an `int`. The program ran and
> every field was wrong. Now a `CopyMem` of the layout's size (015 c6's rule, applied to
> initialization).
>
> ### Shapes still untried
>
> - **A union inside a struct** under a symbolic index (020 c19–c23 test unions, c28 tests
>   `container_of`, neither tests the combination).
> - **`goto` out of three nested scopes**, where 021 must retire objects the jump skipped.
> - **A `switch` whose scrutinee is a struct member** read through a pointer.
>
> ### Also open
>
> - Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>   untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **The reporting you built pays off on defects you did not anticipate** (wave 126). The
> aggregate-return diagnosis was immediate because the finding said `through p.lo` — wave
> 110's `AccessPath`s and wave 111's naming, on a bug neither wave had in mind. Reporting
> quality compounds; treat it as infrastructure, not decoration.
> **Mutation is what makes "already correct" worth committing** (wave 125).
> **A comment claiming a property is not the property** — waves 107, 112, 118, 124.
> **A defect can hide behind another of the same shape** (wave 123).
> **A corpus fixture that runs is coverage; a mutation needs something sharper** (wave 122).
> **A fix does not generalise to a second code path on its own** (wave 121).
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120).
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause** — print diagnostics before 015 §7 truncates.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — nineteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok` and that its anchor was unique**. An oracle
> that can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: aggregate returns have no sret slot (blocks
> `tests/corpus/owed/header_inline.c`); the wave-117 `fork_on_offset` survivor; designated,
> bit-field and address initializers refused; a fault in a non-entry frame is untested;
> `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in sema; the parser's
> speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose
> their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately
> uncovered.
>
> ### Earlier (wave 125, `c096cea`) — 1095 tests, 3 ignored, M1 165/165 by contract
>
> **Ten corpus files, all executing clean. Wave 125 found no defect** — the first wave since
> 113 that did not. 021 c21 and `__attribute__((packed))` were both already correct; the
> wave landed *coverage* for them and said so rather than manufacturing a failure.
>
> Read that as a signal about where to look next, not as the corpus being exhausted. The
> shapes that found defects were ones the engine had never *executed*; the two here were
> paths 014's layout differential and 020's checkers already covered from another angle.
>
> ### Shapes still untried — prefer ones nothing else exercises
>
> - **`static inline` across a real header**, end to end. `gcov_lines.rs` tests header
>   attribution in isolation; nothing lowers *and runs* a program whose helper lives in a
>   header, which is every VPP file.
> - **A union inside a struct** under a symbolic index — 020 c19–c23 test unions and c28
>   tests `container_of`, but not the combination.
> - **`longjmp`-free early exit from three nested scopes with `goto`**, where 021 must
>   retire objects from scopes the jump skipped.
> - **A function returning a struct by value** — 015 §2 makes aggregate returns memory, and
>   no fixture has one.
>
> ### Also open
>
> - Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>   untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **Mutation is what makes "already correct" worth committing** (wave 125). A passing test
> proves nothing about behaviour that was already right — the question is whether it *would
> have noticed*, and only a mutant answers that.
> **A comment claiming a property is not the property** — waves 107, 112, 118, 124, each
> time the actual defect. When a comment states numbers, check the code produces them.
> **A defect can hide behind another of the same shape** (wave 123).
> **A corpus fixture that runs is coverage; a mutation needs something sharper** (wave 122).
> **A fix does not generalise to a second code path on its own** (wave 121).
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120).
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause** — print diagnostics before 015 §7 truncates.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — nineteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok` and that its anchor was unique** — wave 125's
> `packed` anchor matched twice and the assert caught it. An oracle that can silently not
> run is not an oracle — **announce every skip**.
>
> Owed and written down: the wave-117 `fork_on_offset` survivor; designated and bit-field initializers refused; a fault in a non-entry frame is untested; `Bits` path steps
> are not emitted; `typeof` types to `Ty::Error` in sema; the parser's speculative type-name
> diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose their element width in
> `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately uncovered.
>
> ### Earlier (wave 124, `06c4b74`) — 1093 tests, 3 ignored, M1 165/165 by contract
>
> **Nine corpus files, all executing clean, and `tests/corpus/owed/` is empty.** Varargs
> work end to end: `__builtin_va_start/_arg/_end` are compiler-declared, lower to 020
> §4.4.1's instructions, and `__builtin_va_list` has the ABI's 24 bytes aligned 8.
>
> ### Keep adding corpus fixtures — the yield is not falling
>
> Waves 114–124 found **fourteen** defects through the corpus. Shapes it still lacks:
>
> - **`static inline` across a real header**, end to end (030 attributes its lines to the
>   header; `gcov_lines.rs` tests that in isolation and nothing does it whole)
> - **bit-fields** read and written through a symbolic index
> - a **union inside a struct** under a symbolic index
> - **`const` violation**: `const int g = 1; *(int *)&g = 2;` — 021 c21 says exactly one
>   finding that does not alter the bytes, and it has been reachable since wave 114 fixed
>   `is_const` with nothing testing it
> - **`__attribute__((packed))`** on a struct read through a pointer, which VPP uses for
>   every wire header
>
> ### Also open
>
> - Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>   untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **A comment claiming a property is not the property** — wave 107's rule, and waves 112,
> 118 and 124 each found it *was* the defect. When a comment states numbers, check the code
> produces them; `B::VaList` said "24 bytes aligned 8" and built a zero-length array.
> **A defect can hide behind another of the same shape** (wave 123). Varargs took three,
> each revealed only by fixing the one above it.
> **A corpus fixture that runs is coverage; a mutation needs something sharper** (wave 122).
> **A fix does not generalise to a second code path on its own** (wave 121).
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120).
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause** — print diagnostics before 015 §7 truncates.
> The standard first move when a corpus file is "skipped"; needed three times.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — nineteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok`**, and that its anchor was unique. An oracle
> that can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: the wave-117 `fork_on_offset` survivor; designated, bit-field and
> address initializers refused; 021 c21 untested on a real global; a fault in a non-entry
> frame is untested; `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in
> sema; the parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U`
> string literals lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered.
>
> ### Earlier (wave 123, `5ce2093`) — 1090 tests, 3 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: sema has no type for `__builtin_va_list`
>
> `va_list ap;` gets a **one-byte** object, so any read of it is out of bounds:
>
>     out-of-bounds: 8-byte access at offset 0 of ap, which is 1 bytes
>
> On x86-64 `__builtin_va_list` is `__va_list_tag[1]` — 24 bytes: two `unsigned int`
> offsets, then `overflow_arg_area` and `reg_save_area` pointers. Sema needs a builtin type
> with the target's layout, which is also what lets `va_list *` cross a function boundary the
> way 020 §4.4.1 requires.
>
> It is the only thing keeping `tests/corpus/owed/varargs.c` out of the corpus. Wave 123
> already fixed the two defects underneath it (sema reporting `__builtin_va_*` undeclared;
> lowering treating them as calls) and the file lowers with `vastart` / `vaarg` / `vaend` in
> its golden.
>
> ### Eight corpus files, and the yield is holding
>
> Waves 114–123 found **thirteen** defects through the corpus. Shapes it still lacks:
>
> - **`static inline` across a real header** end to end (030 attributes its lines to the
>   header; `gcov_lines.rs` tests that in isolation and nothing does it whole)
> - **bit-fields** read and written through a symbolic index
> - a **union inside a struct** under a symbolic index
> - **`setjmp`-free early exit**: `return` from inside three nested scopes with a `goto`
>
> ### Also open
>
> - **021 c21 untested on a real global** since wave 114 fixed `is_const`.
> - Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>   untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **A defect can hide behind another of the same shape** (wave 123). Sema and lowering both
> reported `__builtin_va_start` undeclared; fixing sema only moved the message one crate
> down. When a fix reveals the identical complaint from elsewhere, expect a *layered* gap,
> not a regression.
> **A corpus fixture that runs is coverage; a mutation needs something sharper** (wave 122).
> **A fix does not generalise to a second code path on its own** (wave 121).
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120).
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause** — print diagnostics before 015 §7 truncates
> them. The standard first move when a corpus file is "skipped"; needed three times now.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — nineteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok`**, and that its anchor was unique. An oracle
> that can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: `__builtin_va_list` has no type (blocks
> `tests/corpus/owed/varargs.c`); the wave-117 `fork_on_offset` survivor; designated,
> bit-field and address initializers refused; 021 c21 untested on a real global; a fault in a
> non-entry frame is untested; `Bits` path steps are not emitted; `typeof` types to
> `Ty::Error` in sema; the parser's speculative type-name diagnostic rollback is unpinned;
> `L`/`u`/`U` string literals lose their element width in `unquote`; 010's 18, 011's 12 and
> 012's 17 are deliberately uncovered.
>
> ### Earlier (wave 122, `dd6f799`) — 1088 tests, 3 ignored, M1 165/165 by contract
>
> **Seven corpus files, all executing clean.** Wave 122 added a symbolic loop with a `goto`
> out of a nested one (clean on arrival) and a symbolic `switch` — which found that **every
> C switch with two labels on one arm was unlowerable**. `case 1: case 2:` parses as a
> `Case` whose *body* is another `Case`, and the switch's statement loop only walked
> top-level statements.
>
> ### Keep going: the corpus is the highest-yield thing in this project
>
> Waves 114–122 found **eleven** defects through it, one per fixture or better. Shapes it
> still lacks:
>
> - **varargs** (`printf`-shaped; 020 §4.4.1's `VaArg` is implemented and no corpus file
>   exercises it)
> - **`static inline` across a real header** (030 attributes its lines to the header, and
>   `gcov_lines.rs` tests that in isolation — nothing does it end to end)
> - a **`do`/`while`** and a `continue` in a nested loop
> - **bit-fields** read and written through a symbolic index
> - a **recursive** function against `max_recursion_depth`
>
> ### Also open
>
> - **021 c21 untested on a real global** since wave 114 fixed `is_const`.
> - Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>   untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **A corpus fixture that runs is coverage; a mutation needs something sharper** (wave 122).
> "The file lowers" does not die when one branch of a walk is deleted — the focused tests in
> `globals.rs` do.
> **A fix does not generalise to a second code path on its own** (wave 121).
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120).
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause** — print diagnostics before 015 §7 truncates
> them. Needed twice now (waves 119, 122); it is the standard first move when a corpus file
> is "skipped".
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — nineteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok`**, and that its anchor was unique. An oracle
> that can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: the wave-117 `fork_on_offset` survivor; designated, bit-field and
> address initializers refused; 021 c21 untested on a real global; a fault in a non-entry
> frame is untested; `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in
> sema; the parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U`
> string literals lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered.
>
> ### Earlier (wave 121, `67a03a4`) — 1086 tests, 3 ignored, M1 165/165 by contract
>
> **Six corpus files, all executing clean**, and `tests/corpus/owed/` is empty.
> `indirect_call.c` graduated after four defects — three in lowering, one in the engine
> (`direct_into` never bound the callee's parameters, so every call through a function
> pointer arrived with none).
>
> ### Next, in rough order of value
>
> 1. **More corpus fixtures.** Waves 114–121 found ten defects through the corpus. Nothing
>    else in this project has that rate. Shapes it still lacks: loops with symbolic bounds,
>    varargs, a `switch` on a symbolic value, `static inline` across a real header, a
>    `goto` out of a nested loop.
> 2. **021 c21 untested on a real global** since wave 114 fixed `is_const`:
>    `const int g = 1; *(int *)&g = 2;` should be exactly one finding that does not alter
>    the bytes.
> 3. Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>    untested; `Bits` path steps are not emitted.
> 4. **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **A fix does not generalise to a second code path on its own** (wave 121). The direct call
> path carried a comment recording "arguments were accepted and discarded" being fixed;
> `direct_into` was written afterwards without it. When you fix a calling convention, a
> lifetime, a binding — grep for the other path.
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120).
> **A fixture parked in `owed/` covers nothing** (wave 120) — the suite does not run it.
> **A fixture that will not lower is still evidence** (wave 119).
> **An aggregate diagnostic hides the cause**; print diagnostics before 015 §7 truncates.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — nineteen waves, and
> wave 121 again: reversing the parameter binding survived a one-parameter callee.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok`**. An oracle that can silently not run is not
> an oracle — **announce every skip**.
>
> Owed and written down: the wave-117 `fork_on_offset` survivor; designated, bit-field and
> address initializers refused; 021 c21 untested on a real global; a fault in a non-entry
> frame is untested; `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in
> sema; the parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U`
> string literals lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered.
>
> ### Earlier (wave 120, `6d1eb9d`) — 1083 tests, 3 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: an indirect call does not bind the callee's parameters
>
> `tests/corpus/owed/indirect_call.c` now **lowers and verifies** (waves 119–120 fixed three
> lowering defects). What is left is in the engine: both callees read their parameter `v` as
> uninitialized —
>
>     uninitialized-read: read at offset 0 of v touches bit 0, which was never written
>
> Two findings, one per resolved callee, so the indirect *dispatch* works and the *argument*
> does not arrive. Start at `Callee::Indirect` in the engine's call handling and compare
> what it binds against the `Callee::Direct` path.
>
> Then move the fixture into `tests/corpus/c/`, bless, and read the golden.
>
> ## 🔴 And: a fixture parked in `owed/` covers nothing
>
> Wave 120's mutation found that **two of the three lowering fixes from waves 119–120 had no
> test at all** — the only thing exercising them was the fixture in `owed/`, which the suite
> does not run. `globals.rs` now has direct tests for each. **When a fixture goes to `owed/`,
> the fixes it motivated need tests where the suite can see them.** Check the rest of
> `owed/` against this as it grows.
>
> ### Next
>
> 1. The indirect-call argument binding (above).
> 2. **More corpus fixtures** — waves 114–120 found nine defects through the corpus, and two
>    of them came from a file that would not lower. Loops with symbolic bounds, varargs, a
>    `switch` on a symbolic value, `static inline` across a real header.
> 3. 021 c21 untested on a real global; designated/bit-field/address initializers refused;
>    a fault in a non-entry frame is untested; `Bits` path steps are not emitted.
> 4. **023 c17** — a milestone, not a wave. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **A wrong diagnosis is expensive; disprove it with tests you keep** (wave 120). Wave 119
> blamed sema and was wrong. The six tests written to prove it now pin behaviour nothing else
> did — a disproof is worth keeping, not deleting.
> **A fixture parked in `owed/` covers nothing** (wave 120).
> **A fixture that will not lower is still evidence** (wave 119) — it found three defects
> without ever producing a golden.
> **An aggregate diagnostic hides the cause**; print diagnostics before 015 §7 truncates them.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — eighteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so; **check a patch script printed `ok`**. An oracle that can silently not run is not
> an oracle — **announce every skip**.
>
> Owed and written down: indirect calls do not bind parameters (blocks
> `tests/corpus/owed/indirect_call.c`); the wave-117 `fork_on_offset` survivor; designated,
> bit-field and address initializers refused; 021 c21 untested on a real global; a fault in a
> non-entry frame is untested; `Bits` path steps are not emitted; `typeof` types to
> `Ty::Error` in sema; the parser's speculative type-name diagnostic rollback is unpinned;
> `L`/`u`/`U` string literals lose their element width in `unquote`; 010's 18, 011's 12 and
> 012's 17 are deliberately uncovered.
>
> ### Earlier (wave 119, `8d170d3`) — 1074 tests, 3 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: sema does not type function-pointer declarators
>
> `int (*fn)(int) = twice;` types `fn` as an **integer**, so its slot is declared `Int(32)`
> and storing a `Ptr` into it fails verification. `cty` maps `Ty::Ptr(_)` to `CTy::Ptr`
> correctly, so the wrong answer is upstream — sema builds no pointer type for the
> declarator.
>
> Calling through a function pointer is how VPP dispatches every graph node, so this blocks
> a whole class of real code. Wave 119 already fixed the two *lowering* defects underneath
> it (a call through a declared variable was "undeclared"; a bare function name was `Undef`
> rather than `AddrOfFunc`), so sema is the only thing left.
>
> **The fixture is written and waiting**: `tests/corpus/owed/indirect_call.c`. Move it back
> into `tests/corpus/c/`, bless, read the golden.
>
> ### `tests/corpus/owed/` is new and worth using
>
> Real C, written to the corpus standard, that chiero refuses. Out of the corpus so the
> suite stays green; **not deleted**, so the gap stays visible and the fixture is ready the
> day it closes. Its README records the *diagnosis*, not the symptom. Put the next
> unlowerable shape there rather than dropping it.
>
> ### Next, in rough order of value
>
> 1. Sema's function-pointer declarators (above).
> 2. **More corpus fixtures.** Waves 114–119 found seven defects through the corpus, and
>    wave 119's two came from a file that *would not even lower*. Loops with symbolic
>    bounds, `static inline` across a real header, varargs, a `switch` on a symbolic value.
> 3. **021 c21 untested on a real global** since wave 114 fixed `is_const`.
> 4. Designated, bit-field and address initializers refused; a fault in a non-entry frame is
>    untested; `Bits` path steps are not emitted.
> 5. **023 c17** — a milestone, not a wave.
> 6. The wave-117 `fork_on_offset` survivor.
>
> ### Rules earned, most recent first
>
> **A fixture that will not lower is still evidence** (wave 119). `indirect_call.c` never
> produced a golden and found two defects anyway. Write the fixture for the shape you want
> to support, not the shape you think is supported.
> **An aggregate diagnostic hides the cause.** 015 §7 refuses a function whole and replaces
> its diagnostics; print them *before* the truncation when you need to know why.
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence** (wave 118).
> **State that forking clones must not be cached where forking cannot reach** (wave 118).
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **The fixture never reached the comparison the design exists for** — seventeen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so rather than claiming coverage; **check a patch script printed `ok`**. An oracle that
> can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: sema's function-pointer declarators (blocks
> `tests/corpus/owed/indirect_call.c`); the wave-117 `fork_on_offset` survivor; designated,
> bit-field and address initializers refused; 021 c21 untested on a real global; a fault in a
> non-entry frame is untested; `Bits` path steps are not emitted; `typeof` types to
> `Ty::Error` in sema; the parser's speculative type-name diagnostic rollback is unpinned;
> `L`/`u`/`U` string literals lose their element width in `unquote`; 010's 18, 011's 12 and
> 012's 17 are deliberately uncovered.
>
> ### Earlier (wave 118, `f6cb7ec`) — 1074 tests, 3 ignored, M1 165/165 by contract
>
> **All five corpus files execute clean**, and the only `#[ignore]`s left are 010's 18,
> 011's 12 and 012's 17 — the three deliberately-uncovered metrics. Every corpus file is
> lowered, verified, golden-compared *and* run, with every path terminating by returning and
> nothing invented.
>
> ### Next, in rough order of value
>
> 1. **The corpus is five files.** It is now a real oracle — waves 114–118 found five defects
>    through it — so the cheapest way to find the sixth is to add fixtures. Loops with
>    symbolic bounds, pointer arithmetic across struct members, `static inline` from a
>    header, a function pointer call.
> 2. **021 c21 is reachable and untested on a real global** (since wave 114 fixed
>    `is_const`): `const int g = 1; *(int *)&g = 2;` should be exactly one finding that does
>    not alter the bytes.
> 3. Initializer forms refused: designated, bit-field, address (`int *p = &g;` — CIR cannot
>    express a relocation).
> 4. A fault in a non-entry frame is untested; `Bits` path steps are not emitted.
> 5. **023 c17** — a milestone, not a wave; measurement in wave 110's entry.
> 6. The unresolved `fork_on_offset` survivor from wave 117 (siblings given the base offset
>    still pass) — find out whether `pending_dst` is `None` there.
>
> ### Rules earned, most recent first
>
> **When a hypothesis is wrong, the fixtures that disprove it are the evidence, not a dead
> end** (wave 118). Three passing fixtures narrowed a wild pointer to "only under forking",
> which no amount of reading `AddrOfGlobal` would have.
> **State that forking clones must not be cached where forking cannot reach.** An
> `Engine`-level map naming a `State`-level object is a hit for something that is not there.
> **A failing test is not automatically a failing engine** (wave 117).
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **A comment claiming a property is not the property** (waves 107, 112).
> **The fixture never reached the comparison the design exists for** — sixteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **some changes are not expressible as a one-line mutant** —
> say so rather than claiming coverage; **check a patch script printed `ok`**. An oracle that
> can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: the wave-117 `fork_on_offset` survivor; designated, bit-field and
> address initializers refused; 021 c21 untested on a real global; a fault in a non-entry
> frame is untested; `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in
> sema; the parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U`
> string literals lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered.
>
> ### Earlier (wave 117, `07524e1`) — 1070 tests, 4 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: a symbolic index into a *global* array is a wild pointer
>
> `table[i]` with symbolic `i` on a file-scope array reports
> `wild-pointer: access through a pointer at address 4 matching no known object`. **The same
> shape on a local array works** — `array_bounds.c` passes — so the fork and the memory model
> are fine and the global's object is not being found from the concretized offset. Start by
> comparing what `AddrOfGlobal` puts in the state against what `AddrOfLocal` does.
>
> It is the only thing keeping `globals.c` out of the corpus sweep, which excludes it **by
> name and announces it** so the other four files stay checked.
>
> ### What wave 117 delivered
>
> Symbolic indexing works. `SmtLib::discover()` opts in a backend when one is on `PATH`
> (022 c2: discovery is a runtime fact, the suite runs without one, every skip announced).
> **Four of five corpus files now execute clean** — every path terminating by returning,
> nothing invented. That was the front for three waves.
>
> ### An unresolved mutation survivor — do not assume it is covered
>
> In `fork_on_offset`, giving every sibling the base offset (`off: p.off` instead of
> `p.off.wrapping_add(v)`) leaves all four `symbolic_index` tests passing. Either the
> sibling's destination local is written somewhere else too, or `pending_dst` is `None` here
> and the siblings' values come from re-evaluation. **Find out which before trusting that
> path.** The other two fork mutants die, so the forking itself is covered.
>
> ### Also open
>
> - Initializer forms refused: designated, bit-field, address (`int *p = &g;` — CIR cannot
>   express a relocation).
> - 021 c21 reachable and untested on a real global since wave 114.
> - A fault in a non-entry frame is untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave; measurement in wave 110's entry.
>
> ### Rules earned, most recent first
>
> **A failing test is not automatically a failing engine** (wave 117). The symbolic-index
> fixture left its index unconstrained, so the offset really did have unbounded values and
> bounding was correct — the test was measuring the bound rather than the enumeration.
> Check the fixture states what you meant before changing the code.
> **Exhaustion and "the solver gave up" are different answers** (wave 116).
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one** (wave 113).
> **A survivor is not automatically a fixture gap** — remove the code and re-run (112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **A renderer that disagrees with the spec's own example is a defect** (wave 110).
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **Bisect a hand-built module against the lowered one** (wave 109).
> **A comment claiming a property is not the property** (waves 107, 112).
> **The fixture never reached the comparison the design exists for** — fifteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**; **a patch script that asserts must write nothing on failure —
> and check it said `ok`**, wave 117 lost two edits to a script that aborted mid-way. An
> oracle that can silently not run is not an oracle — **announce every skip**.
>
> Owed and written down: symbolic index into a global array is a wild pointer (excludes
> `globals.c` from the sweep); the unresolved `fork_on_offset` survivor above; designated,
> bit-field and address initializers refused; 021 c21 untested on a real global; a fault in a
> non-entry frame is untested; `Bits` path steps are not emitted; `typeof` types to
> `Ty::Error` in sema; the parser's speculative type-name diagnostic rollback is unpinned;
> `L`/`u`/`U` string literals lose their element width in `unquote`; 010's 18, 011's 12 and
> 012's 17 are deliberately uncovered.
>
> ### Earlier (wave 116, `da79c94`) — 1066 tests, 8 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: the default solver cannot enumerate
>
> Wave 116 implemented forking on a symbolic `PtrAdd` offset — enumerate feasible values,
> one state each, bounded at 16. **The mechanism is right and the tier-1 solver cannot drive
> it**: it answers the first query and returns `Unknown` on the second, so every symbolic
> index takes the bounded path instead of being explored.
>
> So `buf[i]` still does not work, and five tests are `#[ignore]`d on it:
> `symbolic_index.rs`'s two, and the three corpus sweeps from waves 114–115.
>
> **The decision to settle**: 022 §3.2 defines the tier-1 fragment; enumeration needs tier 2,
> which means an SMT backend, which HANDOFF §2 says chiero must not *link*. `TieredSolver::
> with_backend(SmtLib)` already exists and spawns a process. So either (a) the corpus tests
> opt into a backend when one is on `PATH` and announce the skip when it is not — the shape
> the gcov oracle already uses — or (b) tier 1 grows enough arithmetic to enumerate a bounded
> index. (a) is a day; (b) is a milestone. **Neither is started.**
>
> ### What wave 116 delivered
>
> A symbolic index degrades **honestly**: `Fidelity::Bounded` + `BudgetHit`, naming the
> offset and whether the search exceeded the bound or was cut short by the solver. Before it
> was `NoInformation` — a value chiero made up, everything downstream unsound. A bound a
> reader can act on is not the feature, but it is not nothing.
>
> ### Also open
>
> - Initializer forms refused: designated, bit-field, address (`int *p = &g;` — CIR cannot
>   express a relocation).
> - 021 c21 reachable and untested on a real global since wave 114.
> - A fault in a non-entry frame is untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave; measurement in wave 110's entry.
>
> ### Rules earned, most recent first
>
> **Exhaustion and "the solver gave up" are different answers** (wave 116). Any loop that
> stops on `Unsat` must not stop the same way on `Unknown` — the first is a fact about the
> program, the second about the prover, and conflating them turns incompleteness into a
> confident wrong answer.
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** (wave 114).
> **A wrong answer is worse than a missing one**; refuse whole rather than encode partially
> (wave 113).
> **A survivor is not automatically a fixture gap** — remove the code and re-run (112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **A renderer that disagrees with the spec's own example is a defect** (wave 110).
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **Bisect a hand-built module against the lowered one** (wave 109).
> **A comment claiming a property is not the property** (waves 107, 112).
> **The fixture never reached the comparison the design exists for** — fourteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**. An oracle that can silently not run is not an oracle —
> **announce every skip**.
>
> Owed and written down: the tier-1 solver cannot enumerate (blocks five tests); designated,
> bit-field and address initializers refused; 021 c21 untested on a real global; a fault in a
> non-entry frame is untested; `Bits` path steps are not emitted; `typeof` types to
> `Ty::Error` in sema; the parser's speculative type-name diagnostic rollback is unpinned;
> `L`/`u`/`U` string literals lose their element width in `unquote`; 010's 18, 011's 12 and
> 012's 17 are deliberately uncovered.
>
> ### Earlier (wave 115, `870b145`) — 1064 tests, 6 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: `PtrAdd` with a symbolic offset is not modeled
>
> `buf[i]` with symbolic `i` on a local array falls through to an invented value, and three
> more "not modeled" reports cascade from it. **This is core to what a symbolic executor
> does**, and `array_bounds.c` was written specifically to exercise it. It blocks both
> corpus-execution sweeps in `chiero-recipe/tests/no_spurious_findings.rs`
> (`every_corpus_file_runs_clean`, `no_corpus_file_invents_a_value` — both `#[ignore]`d
> naming it, both correct as written).
>
> Reproduce: `cargo test -p chiero-recipe --test no_spurious_findings -- --ignored`.
>
> ## What wave 115 found, and why nothing had
>
> **The engine had never executed a corpus file.** `run()` entered `funcs.first()`, which in
> every real TU is the first *declaration* — `chiero_make_symbolic`, with no blocks — so
> every run ended `Errored("no such block BlockId(0)")` before one instruction. Eighteen
> waves of green suite over it, because goldens compare lowered *text* and an errored state
> reports no findings. **Wave 114's own "executes clean" test passed for that reason.**
>
> The lesson is sharper than "read the artifact": **an assertion of absence needs a
> companion assertion that the run got there.** `findings().is_empty()` over files written so
> that absence is the property is close to asserting nothing. Requiring every path to
> terminate by *returning* is what broke it open — three separate defects in one sweep.
>
> ### Also open
>
> - With the default tier-1 solver, `abs_branch.c`'s `my_abs(x) >= 0` cannot be discharged.
>   Honest incompleteness ("could not rule it out"), but the file does not demonstrate what
>   it was written to demonstrate.
> - Initializer forms still refused: designated, bit-field, address (`int *p = &g;` — a CIR
>   question, `GlobalInit::Bytes` cannot express a relocation).
> - **021 c21 is reachable and untested on a real global** since wave 114 fixed `is_const`.
> - A fault in a non-entry frame is untested; `Bits` path steps are not emitted.
> - **023 c17** — a milestone, not a wave; measurement in wave 110's entry.
>
> ### Rules earned, most recent first
>
> **An assertion of absence needs a companion assertion that the run got there** (wave 115).
> **Read the golden, not just the test result** — the artifact says what happened, the suite
> says what you asked (wave 114).
> **A wrong answer is worse than a missing one**; refuse whole rather than encode partially
> (wave 113).
> **A survivor is not automatically a fixture gap** — remove the code and re-run (112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **A renderer that disagrees with the spec's own example is a defect** (wave 110).
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **Bisect a hand-built module against the lowered one** (wave 109).
> **A comment claiming a property is not the property** (waves 107, 112).
> **The fixture never reached the comparison the design exists for** — thirteen waves.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial**. An oracle that can silently not run is not an oracle —
> **announce every skip**.
>
> Owed and written down: symbolic `PtrAdd` unmodeled (blocks both corpus sweeps); tier-1
> cannot discharge `abs_branch.c`; designated, bit-field and address initializers refused;
> 021 c21 untested on a real global; a fault in a non-entry frame is untested; `Bits` path
> steps are not emitted; `typeof` types to `Ty::Error` in sema; the parser's speculative
> type-name diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose their element
> width in `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately uncovered.
>
> ### Earlier (wave 114, `7db8fc5`) — 1060 tests, 3 ignored, M1 165/165 by contract
>
> **The corpus has globals.** `tests/corpus/c/globals.c` — a `static const` initialized
> table, a counter with no initializer, a file-scope struct with padding — lowers, matches
> a golden, and **executes clean**. That closes the front waves 112 and 113 both left open.
>
> Reading its golden immediately found `is_const` hardcoded `false`, which had made 021 c21
> unreachable for every global. **That is the third wave running where reading a golden or a
> control found more than the test did.**
>
> ### Next, in rough order of value
>
> 1. **The other corpus files still have no globals**, and nothing runs *them* through the
>    engine — `goldens.rs` only compares text. `the_globals_corpus_file_runs_clean` in
>    `chiero-recipe` is the pattern; generalise it over `tests/corpus/c/*.c` so every corpus
>    file's assertions are checked, not just its shape.
> 2. **Initializer forms still refused** (each falls back to `GlobalInit::Zero`):
>    designated (`{[2] = 5}`, `{.b = 5}` — VPP uses these heavily for node registration),
>    bit-field members, and **an address** (`int *p = &g;`), which `GlobalInit::Bytes` has no
>    way to express — a CIR question before it is a lowering one.
> 3. **021 c21 is now reachable and untested on a real global.** `const int g = 1;
>    *(int *)&g = 2;` lowers and verifies (wave 112); check it produces exactly one finding
>    and leaves the bytes alone.
> 4. **A fault in a non-entry frame is untested**; **`Bits` path steps are not emitted**;
>    **023 c17** remains a milestone, not a wave (measurement in wave 110's entry).
>
> ### Rules earned, most recent first
>
> **Read the golden, not just the test result.** Wave 114's real finding was in the blessed
> output, not in any assertion. Waves 112 and 113 found theirs by dumping a *passing*
> control. The suite tells you what you asked; the artifact tells you what happened.
> **A wrong answer is worse than a missing one** — when you cannot compute something, refuse
> it whole rather than encoding the part you understood (wave 113).
> **A survivor is not automatically a fixture gap** — remove the code and re-run to tell dead
> code from missing coverage (waves 112, 113).
> **A workaround marks a defect; go back and delete it** (wave 111).
> **A renderer that disagrees with the spec's own example is a defect** (wave 110).
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **Bisect a hand-built module against the lowered one** (wave 109).
> **A comment claiming a property is not the property** (waves 107, 112).
> **The fixture never reached the comparison the design exists for** — twelve waves running.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial, not a surviving gap**. An oracle that can silently not run is
> not an oracle — **announce every skip**.
>
> Owed and written down: only one corpus file is executed; designated, bit-field and address
> initializers are refused; 021 c21 untested on a real global; a fault in a non-entry frame
> is untested; `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in sema; the
> parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U` string literals
> lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately
> uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 113, `738a6ee`) — 1058 tests, 3 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: put globals in the corpus
>
> **Still true, and now the last thing standing between two waves of global work and any
> confidence in it**: not one C file in `tests/corpus/c/` has a file-scope variable. Waves
> 112 and 113 fixed a missing feature (globals lowered to `Undef`) and a wrong answer
> (initializers parsed and discarded); both were found by hand-written probes, and neither
> would have been caught by the corpus.
>
> Add corpus C files with file-scope arrays, structs, `static` counters and initializers,
> re-bless, and **read the goldens**. Two waves of new encoding have never been seen by a
> golden.
>
> ### Known gaps in what wave 113 built
>
> Chiero refuses what it cannot encode and falls back to `GlobalInit::Zero` — correct, and
> less information than the program has. Each of these is a refusal today:
>
> - **Designated initializers** (`{[2] = 5}`, `{.b = 5}`). VPP uses these heavily for node
>   registration.
> - **Bit-field members** in a struct initializer.
> - **An address as an initializer** (`int *p = &g;`) — `const_of` cannot fold it, so it
>   refuses. `GlobalInit::Bytes` has no way to express a relocation, which is a CIR question
>   before it is a lowering one.
>
> ### Also open
>
> - **A fault in a non-entry frame is untested** (`object_name` uses `stack.last()`; no C
>   fixture distinguishes it from `first()`). Needs a hand-built `.cir` module.
> - **`Bits` path steps are not emitted** for bit-field accesses (020 §4.4).
> - **023 c17** — the last contract, a milestone not a wave; measurement in wave 110's entry.
>
> ### Rules earned, most recent first
>
> **A wrong answer is worse than a missing one.** Wave 112 fixed reads of globals returning
> `Undef`; wave 113 fixed them returning *zero*. The first suppresses findings and degrades
> fidelity; the second is asserted with `Fidelity::Exact`. When you cannot compute something,
> refuse it whole — a partial encoding is the confidently-wrong direction.
> **Dump the control when it passes** (wave 112).
> **A survivor is not automatically a fixture gap** — remove the code and re-run to tell dead
> code from missing coverage. Waves 112 and 113 each had one of each.
> **A workaround marks a defect; go back and delete it** (wave 111).
> **A renderer that disagrees with the spec's own example is a defect** (wave 110).
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **Bisect a hand-built module against the lowered one** (wave 109).
> **A comment claiming a property is not the property** (wave 107, and wave 112 again).
> **The fixture never reached the comparison the design exists for** — eleven waves running.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial, not a surviving gap**.
>
> Owed and written down: the corpus has no globals; designated, bit-field and address
> initializers are refused; a fault in a non-entry frame is untested; `Bits` path steps are
> not emitted; `typeof` types to `Ty::Error` in sema; the parser's speculative type-name
> diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose their element width in
> `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 112, `775c406`) — 1048 tests, 3 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: put globals in the corpus
>
> **Not one C file in `tests/corpus/c/` has a file-scope variable.** That is how wave 112's
> defect — lowering had *no notion* of a global, so every read of one became `Undef` —
> survived 111 waves with a green suite. An unknown value suppresses findings rather than
> producing wrong ones, so the engine was a false negative for all of VPP and said `Exact`
> while doing it.
>
> Globals now work (registered in the declaration pass, read and written through
> `AddrOfGlobal`, `Extern` distinguished from `Zero`). What is missing is *coverage*: add
> corpus C files with file-scope arrays, structs and `static` counters, re-bless, and read
> the goldens. Expect more gaps — an initializer on a global is still lowered as `Zero`
> regardless of what it says, which is the next obvious hole.
>
> **Known and unfixed**: `int g[4] = {1,2,3,4};` records `GlobalInit::Zero`. The initializer
> is parsed and ignored. That is a wrong answer rather than a missing one, which makes it
> worse than what wave 112 fixed.
>
> ### Also open
>
> - **A fault in a non-entry frame is untested** (`object_name` uses `stack.last()`; no C
>   fixture distinguishes it from `first()` because the engine enters `funcs.first()` and C
>   declares callees first). Needs a hand-built `.cir` module in `chiero-exec`'s suite.
> - **`Bits` path steps are not emitted** for bit-field accesses (020 §4.4).
> - **023 c17** — the last contract, a milestone not a wave; measurement in wave 110's entry.
>
> ### Rules earned, most recent first
>
> **Dump the control when it passes.** Wave 112's RED found two verifier errors; the *real*
> defect was in the one fixture that passed, and `verify` was too weak to see it. When a
> control passes, print what it produced before believing it.
> **A survivor is not automatically a fixture gap** — check. Wave 112 had two: one was a
> fixture gap (three fixtures added, mutant dies), the other was genuinely dead code
> (deleted). Removing the code and re-running the suite is how you tell them apart.
> **A workaround marks a defect; go back and delete it** (wave 111).
> **A renderer that disagrees with the spec's own example is a defect** (wave 110).
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **Bisect a hand-built module against the lowered one** (wave 109).
> **A comment claiming a property is not the property** (wave 107) — wave 112 again: the
> `Undef` return had a comment calling it "honest", and it was the bug.
> **The fixture never reached the comparison the design exists for** — ten waves running.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial, not a surviving gap**.
>
> Owed and written down: global initializers are ignored; the corpus has no globals; a fault
> in a non-entry frame is untested; `Bits` path steps are not emitted; `typeof` types to
> `Ty::Error` in sema; the parser's speculative type-name diagnostic rollback is unpinned;
> `L`/`u`/`U` string literals lose their element width in `unquote`; 010's 18, 011's 12 and
> 012's 17 are deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 111, `6db3cac`) — 1036 tests, 4 ignored, M1 165/165 by contract
>
> ## 🔴 Do this first: two lowering defects on file-scope variables
>
> Both found in five minutes of trying to make a *global* produce a finding. Both make
> lowering emit **invalid CIR** — `verify` rejects it, so nothing downstream runs at all:
>
> - **`int g[4]; … g[1]`** → `PtrAdd base must be pointer-typed, got Int(32)`. Indexing a
>   global array. `lvalue_addr`'s `Ident` arm looks only in `fs().locals`, so a file-scope
>   name falls through to whatever the value path produces.
> - **`const int g = 1; *(int *)&g = 2`** → `WidthMismatch`.
>
> File-scope arrays are everywhere in C and in VPP. That two independent defects surfaced
> that fast says how little of the global path is exercised — the corpus fixtures are
> function-local almost throughout. **Start by adding globals to
> `no_spurious_findings.rs`**, which is where both were caught.
>
> `chiero-recipe/tests/no_spurious_findings.rs::a_global_is_named_in_a_finding` is
> `#[ignore]`d on exactly this; un-ignore it when a global can fault.
>
> ### Also open
>
> - **A fault in a non-entry frame is untested.** `object_name` uses `stack.last()`, and no
>   C fixture can distinguish that from `first()`: the engine enters `funcs.first()` and
>   valid C declares a callee before use, so the callee is always first. Needs a hand-built
>   `.cir` module with the entry first, in `chiero-exec`'s own suite.
> - **`Bits` path steps are not emitted** for bit-field accesses (020 §4.4 specifies them).
> - **023 c17** — the last contract, and a milestone rather than a wave. Measurement and the
>   design decision are in wave 110's entry below; do not accept an ignored `workers`
>   parameter.
>
> ### What wave 111 changed
>
> Findings name the variable, not `ObjectId(N)`. The engine substitutes the one token
> `f.object()` identifies, because `chiero-mem` has no module and cannot know names.
> **`chiero-opt`'s eight-wave-old `ObjectId` normalization is deleted** — the transparency
> sweep compares finding text verbatim again, and is stronger for it.
>
> ### Rules earned, most recent first
>
> **A workaround marks a defect; go back and delete it.** Wave 102 normalized `ObjectId` out
> of a comparison to keep a sweep meaningful and recorded why. Nine waves later that note was
> the map to a real fix, and removing the workaround was the strongest verification the fix
> had.
> **A renderer that disagrees with the spec's own example is a defect** (wave 110).
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **Bisect a hand-built module against the lowered one** to decide which crate owns a bug
> (wave 109).
> **A comment claiming a property is not the property** (wave 107).
> **Comparing two configurations cannot see a leak that affects both** (wave 107).
> **Anything claiming to be stable forever needs a pinned literal** (wave 106).
> **The fixture never reached the comparison the design exists for** — nine waves running.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial, not a surviving gap**.
>
> Owed and written down: the two global defects above; a fault in a non-entry frame is
> untested; `Bits` path steps are not emitted; `typeof` types to `Ty::Error` in sema; the
> parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U` string literals
> lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately
> uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 110, `4fe60b2`) — 1031 tests, 3 ignored, M1 165/165 by contract
>
> **`AccessPath` is end-to-end.** Lowering builds paths at the member-access site (the only
> place that knows both the member name and the layout offset), 021 c19 consumes them, and
> the renderer now matches 020 §4.4's own spelling — `p->adj[3].counter`, and
> `a as ip4.as_u32` for a union member per §4.5. That closes the last owed item from wave
> 104.
>
> ## The one remaining contract: 023 c17, and why it is not a wave
>
> 1, 2 and 8 worker threads must produce **identical** `RunResult`s. Measured this wave:
>
> - **38 `&mut TermArena` parameters and 170 arena calls in `chiero-exec` alone.**
> - `TieredSolver::check_path` also takes `&mut TermArena`, and 023 §1.1's "one solver for
>   the run" exists *because* sibling states hit its caches — per-worker solvers throw away
>   the thing that makes it fast.
> - Every `State` holds `Term`s that are indices into one arena, so per-worker arenas need
>   cross-arena term migration for every state, memory, witness and finding.
> - **`completion_order` must be identical too**, and wave 106's contract-7 test reads it as
>   the observable exploration order — so it cannot be canonicalised by sorting. The
>   schedule must stay sequentially determined while only the *work* parallelises.
>
> So the design decision is: **shared arena behind interior mutability (and a shared solver)
> with speculative execution committed in schedule order**, or nothing. Either is a
> milestone, not a wave. Do not accept a `workers` parameter that is ignored — the contract's
> test would pass while testing nothing, which is the trap 020 c16's own parenthesis warns
> about.
>
> ### Worth doing before or instead
>
> - **Extend `no_spurious_findings.rs` as constructs land.** It is the shape that caught
>   wave 109's storm and the suite had nothing like it for 108 waves.
> - `AccessPath`s are built for members and indices only; a `Bits` step for a bit-field
>   access is specified in §4.4 and not emitted.
> - The engine's own findings still cite `ObjectId(N)` (wave 102's owed item). Now that
>   paths reach the engine from real C, a finding could name the variable instead.
>
> ### Rules earned, most recent first
>
> **A renderer that disagrees with the spec's own example is a defect** — wave 110's
> `(*p).adj` vs §4.4's `p->adj`. When a test expectation and the code disagree, check which
> one the spec backs before assuming the test is wrong.
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **Bisect a hand-built module against the lowered one** to decide which crate owns a bug
> (wave 109 — three probes to one instruction).
> **A comment claiming a property is not the property** (wave 107).
> **Comparing two configurations cannot see a leak that affects both** (wave 107).
> **Anything claiming to be stable forever needs a pinned literal** (wave 106).
> **The fixture never reached the comparison the design exists for** — eight waves running.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; **a mutation one other site
> compensates for is partial, not a surviving gap**.
>
> Owed and written down: `Bits` path steps are not emitted; the engine's findings cite
> `ObjectId(N)`; `typeof` types to `Ty::Error` in sema; the parser's speculative type-name
> diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose their element width in
> `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 109, `eb11102`) — 1025 tests, 3 ignored, M1 165/165 by contract
>
> **The false-positive storm is fixed and 023 c21 is closed with it.** A parameter's slot
> took `ScopeId(0)` (allocated while `open_scopes` was empty) and the body's compound then
> opened `ScopeId(0)` too (`next_scope` also starts at 0), so entering the body replaced
> every parameter slot *after* the prologue stored into it. Parameters now live in a scope
> **enclosing** the body — C11 6.2.1p4, and the same shape 015 c12 already fixes for
> `for (int i = 0; …)`.
>
> **One contract remains: 023 c17** — 1, 2 and 8 worker threads produce identical
> `RunResult`s with `wall_clock: None`. Real work, not a test: the engine is
> single-threaded, `TermArena` is not shared, and every `State` holds `Term`s from one
> arena. Per-state `CheckerState` (023 §4 names it as what makes this achievable) and wave
> 106's `pick` (which takes the whole queue) are the two pieces already in place. **Expect
> an arena-sharing decision, not a scheduling tweak** — that is the design question to
> settle before writing any test.
>
> ### The test shape this project was missing
>
> **Assert that correct code produces no findings.** 1017 tests passed while every function
> that read its own scalar parameter reported a spurious `uninitialized-read`, because every
> differential probe compares the *value* chiero computes against gcc and none asked whether
> it also reported something. `chiero-recipe/tests/no_spurious_findings.rs` is that shape;
> extend it whenever a new construct lands. Its controls are the load-bearing half — a
> genuinely uninitialized read must still be reported **and still degrade**, or the whole
> file is satisfied by an engine that went quiet.
>
> ### Rules earned, most recent first
>
> **When a result surprises you, read what the engine already recorded** — `fidelity()`,
> `assumptions()`, `findings()` — before theorising (wave 108; it is how 109's cause was
> found).
> **Bisect a hand-built module against the lowered one** to decide which crate owns a bug:
> wave 109 took three probes to go from "somewhere in exec or lowering" to one instruction.
> **A comment claiming a property is not the property** (wave 107).
> **Comparing two configurations cannot see a leak that affects both** — pair every
> "A differs from B" with an "A equals A'" (wave 107).
> **Anything claiming to be stable forever needs a pinned literal** (wave 106).
> **The fixture never reached the comparison the design exists for** — seven waves running.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**; two guards only ever true together are equivalent mutants; a
> no-op mutant is neither; **`cargo fmt` moves anchors**; and **a mutation that one other
> site compensates for is a partial mutation, not a surviving gap** — mutate every source of
> the effect (wave 109's `NoInformation` has two).
>
> Owed and written down: lowering still builds no `AccessPath`s (wave 107 is the first
> consumer and its fixture supplies them by hand); the engine's own findings cite
> `ObjectId(N)`; `typeof` types to `Ty::Error` in sema; the parser's speculative type-name
> diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose their element width in
> `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 108, `72fa18d`) — 1017 tests, 4 ignored, M1 164/165
>
> ## 🔴 Do this first: a scalar parameter reads as uninitialized from its own slot
>
> Wave 108 found this while working on 023 c21, and it is more serious than any remaining
> contract. For
>
> ```c
> int f(int n) { int t = 0; if (n > 10) { … } }
> ```
>
> lowering emits `store i32 %param -> %n_slot` and then `%5 = load i32, %n_slot`, and the
> **load reports `uninitialized-read`**. The value is therefore invented, the branch
> condition is unknown, both arms are explored, and the path degrades to `Unknown`.
>
> This is a **false-positive storm on every function that reads its own scalar parameter**,
> which is nearly all of them — exactly the failure 021 §3.1 says makes a symbolic engine
> unusable. Reproduce with `cargo test -p chiero-recipe --test replay_coverage -- --ignored`,
> or in three lines against `Engine::new(m).run(&mut a)` with no replay at all.
>
> Start by finding out **which side is wrong**: whether the parameter store writes no
> initialization bits, or the load reads the wrong object. `ObjectId(3)` in the finding is
> the clue; dump `s.mem` object ids against the two allocas. Note that lowering prints the
> parameter as `%1` and the second alloca as `%1` too — `AllocaId` and `ValueId` are
> different namespaces that the textual format spells identically, which made this hard to
> read and may be hiding the confusion somewhere in the engine.
>
> **How it was found is the lesson**: not by any assertion, but by printing the forked
> states' `assumptions()` after a state count came out wrong. The finding had been there all
> along and nothing looked at it. The engine records a great deal about *why* it did
> something and almost no test reads it — when a result surprises you, print
> `fidelity()`, `assumptions()` and `findings()` before theorising.
>
> ### After that
>
> 1. **023 c21** — the two tests are written, correct, and `#[ignore]`d with the blocker
>    named in `chiero-recipe/tests/replay_coverage.rs`. Un-ignore them once the above is
>    fixed; entry-parameter replay binding already landed in wave 108.
> 2. **023 c17** — 1, 2 and 8 worker threads produce identical `RunResult`s. The last M1
>    contract and real work: the engine is single-threaded, `TermArena` is not shared, and
>    every `State` holds `Term`s from one arena. Per-state `CheckerState` and wave 106's
>    `pick` (which takes the whole queue) are the two pieces already in place. Expect an
>    arena-sharing decision, not a scheduling tweak.
>
> ### Rules earned, most recent first
>
> **When a result surprises you, read what the engine already recorded** (wave 108).
> **A comment claiming a property is not the property** — mutate the line it describes;
> if nothing fails, the comment is a wish (wave 107).
> **Comparing two configurations cannot see a leak that affects both** — pair every
> "A differs from B" with an "A equals A'" (wave 107).
> **Anything claiming to be stable forever needs a pinned literal**, not a self-consistency
> check: two runs of a changed algorithm agree with each other (wave 106).
> **The fixture never reached the comparison the design exists for** — six waves running.
> Before running a mutation, ask what the fixture would have to look like for it to survive;
> for every `&&` in a predicate there should be a fixture where that conjunct alone is false.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**, not a survivor; two guards only ever true together are
> equivalent mutants; a genuine no-op mutant is neither; **`cargo fmt` moves anchors**.
>
> Owed and written down: **the uninitialized-parameter defect above**; lowering still builds
> no `AccessPath`s (wave 107 is the first consumer and its fixture supplies them by hand);
> the engine's own findings cite `ObjectId(N)`; `typeof` types to `Ty::Error` in sema; the
> parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U` string
> literals lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 107, `bffd5af`) — 1017 tests, M1 164/165, frontend 114/117
>
> **021 is complete.** Contract 19 closes it: the engine now materializes lazy objects
> *recursively* (it did not before — `p->next->next` ended the path as unresolvable), bounded
> by `LazyPolicy::max_depth`, with every link past the bound resolving to **one shared
> object** so the walk still reaches the return. `AccessPath` finally has a consumer: the
> `Bounded` note names `next` rather than `+8`.
>
> **One contract remains in M1: 023 c17** — 1, 2 and 8 worker threads produce identical
> `RunResult`s with `wall_clock: None`. The engine is single-threaded, so this is real work,
> not a test. Two pieces are already in place: per-state `CheckerState` (023 §4 names it as
> what makes this achievable) and wave 106's `pick`, which takes the whole queue and is the
> single place a parallel scheduler changes. **023 c21** (per-witness replay at line
> granularity against gcov) is also open and is the last of 023.
>
> ### Two rules earned this wave
>
> **A comment claiming a property is not the property.** Wave 107's worst bug was denied by
> its own comment: the cut object was left out of `lazy_depth` *because* "giving it a depth
> would let a walk through it start counting again" — and leaving it absent made
> `unwrap_or(0)` do exactly that. When a comment asserts something, mutate the line it
> describes; if nothing fails, the comment is a wish.
>
> **Comparing two configurations cannot see a leak that affects both.**
> `raising_the_bound_materializes_more_links` compares bounds 2 and 6; a leak past the cut
> makes bound 2 allocate more while still leaving it below bound 6, so it passes. Only two
> *depths at the same bound* — three links and six must cost the same — can see a floor that
> is not a floor. Pair every "A differs from B" assertion with an "A equals A'" one.
>
> ### The survivor pattern (six waves running)
>
> **The fixture never reached the comparison the design exists for.** Before running a
> mutation, ask what the fixture would have to look like for the mutant to survive; for every
> `&&` in a predicate there should be a fixture where that conjunct alone is false. Wave 107
> also hit the *coincidence* form: at four links two different bounds give the same object
> count, so the assertion compared two equal numbers.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**, not a survivor; two guards only ever true together are
> equivalent mutants; a genuine no-op mutant is neither; **`cargo fmt` moves anchors** —
> re-grep after formatting (hit twice now); and anything claiming to be stable forever needs
> a **pinned literal**, not a self-consistency check.
>
> Owed and written down: **lowering still builds no `AccessPath`s** — wave 107 is the first
> consumer and its fixture supplies them by hand, so the naming half is untested against
> real C; the engine's own findings cite `ObjectId(N)`; `typeof` types to `Ty::Error` in
> sema; the parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U`
> string literals lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 106, `7a9ebb3`) — 1011 tests, M1 163/165, frontend 114/117
>
> **010 is complete but for its deliberately-uncovered metric, and 023 c7 is closed.**
> `CookedSite` carries a `config` and the deduplication key includes it; the engine has a
> `Strategy` (`Dfs` / `RandomPath { seed }`) with the seed recorded in every `RunResult`.
>
> **Two contracts remain in M1:**
>
> 1. **021 c19** — `LazyPolicy::max_depth = 2` on a linked list stops materializing at the
>    third `next`, and the result carries `Fidelity::Bounded` **naming `next`**. Check
>    whether `LazyPolicy` exists at all before planning; the naming half is the part a
>    plausible implementation drops.
> 2. **023 c17** — 1, 2 and 8 worker threads produce identical `RunResult`s with
>    `wall_clock: None`. The engine is single-threaded today, so this is a real piece of
>    work, not a test. 023 §4 says per-state `CheckerState` is what makes it achievable —
>    that already holds, and wave 106's `Strategy` is the other half: `pick` takes the
>    whole queue, so a parallel scheduler has one place to change.
> 3. **023 c21** — per-witness replay at line granularity against gcov. The `gcov_lines`
>    oracle already exists in `chiero-lower/tests/gcov_lines.rs`; this needs a *replay* with
>    all inputs concretized.
>
> **A test that compares one run against another cannot see a changed algorithm.** Wave
> 106's only mutation survivor was altering the PRNG constant: every test compared two runs,
> and two runs of a *changed* generator agree with each other just as well. Anything whose
> value is "identical everywhere and forever" — a hash, a PRNG, a serialization — needs a
> **pinned literal**, not a self-consistency check. `strategy.rs` now pins the leaf order for
> seed 7.
>
> **Write the generator, do not depend on it.** SplitMix64 is twelve lines and a dependency
> could change its algorithm in a patch release, silently invalidating every seed in every
> recorded bug report.
>
> ### The survivor pattern (five waves running)
>
> **The fixture never reached the comparison the design exists for.** Before running a
> mutation, ask what the fixture would have to look like for the mutant to survive; for
> every `&&` in a predicate there should be a fixture where that conjunct alone is false.
>
> Harness rules: back up to a scratch copy, **never `git checkout`**; a mutant that does not
> compile is **inconclusive**, not a survivor — wave 106 had two, and `#[default]` cannot
> move to a variant with a field, so some claims are structurally forced and must be mutated
> somewhere else; two guards only ever true together are equivalent mutants; a genuine
> no-op mutant is neither. **`cargo fmt` moves anchors** — re-grep after formatting.
>
> Owed and written down: nothing builds `AccessPath`s (lowering has the layouts and member
> names; 015 is where it belongs, and it is not a numbered contract so it needs a decision);
> the engine's own findings cite `ObjectId(N)`; `typeof` types to `Ty::Error` in sema; the
> parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U` string
> literals lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 105, `92b9328`) — 1002 tests, M1 162/165, frontend 113/117
>
> **020 is complete.** Contracts 29 and 30 close it: `UnionPun` exists and is deliberately
> absent from `chiero_check::default_checkers()`, and `lower_tu_with_config` records the
> `ConfigId` so two endianness configurations produce two distinguishable modules.
>
> **Three contracts remain in M1:**
>
> 1. **021 c19** — the memory model.
> 2. **023's 7, 17, 21** — the execution engine.
> 3. **010 contract 19** — per-`ConfigId` expansion sites, small, unblocked since wave 84,
>    and now easier: wave 105 built the two-configuration fixture pattern in
>    `chiero-lower/tests/endianness.rs`, which is the same shape 010 c19 needs.
>
> **Read the contract for what it says is already true.** Contract 30 needed no CIR
> machinery at all — 020 §4.4 resolves endianness *before* CIR, so the two layouts were
> already correct and the only defect was a hardcoded `None`. Contract 23's memory half was
> the same. Two waves running where the spec's design meant most of the work was already
> done and the test was the deliverable; look for that before building.
>
> ### The survivor pattern, now four waves running
>
> Every mutation survivor since wave 102 has had one shape: **the fixture never reached the
> comparison the design exists for.** Wave 105 had four, all in one checker, each a rule the
> fixture happened never to exercise — same-offset-narrower-width, object identity, overlap
> as a range, and stores-are-not-reads. Each fix was a fixture one line different from an
> existing one. No amount of reading the checker would have found them.
>
> So: **before running a mutation, ask what the fixture would have to look like for the
> mutant to survive.** For every `&&` in a predicate, there should be a fixture where that
> conjunct alone is false. That is faster than running the campaign and it finds the same
> things.
>
> Mutation harness rules still in force: **back up to a scratch copy, never `git checkout`**;
> **a mutant that does not compile is inconclusive, not a survivor**; **two guards only ever
> true together are equivalent mutants**; a genuine no-op mutant is neither — discard it.
> And **`cargo fmt` moves anchors** — wave 105 lost one mutation to a reformatted anchor, so
> re-grep after formatting.
>
> Owed and written down: nothing builds `AccessPath`s (lowering has the layouts and member
> names; 015 is where it belongs, and it is not a numbered contract so it needs a decision);
> the engine's own findings cite `ObjectId(N)`; `typeof` types to `Ty::Error` in sema; the
> parser's speculative type-name diagnostic rollback is unpinned; `L`/`u`/`U` string
> literals lose their element width in `unquote`; 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 104, `72169ce`) — 991 tests, M1 160/165, frontend 113/117
>
> **020 contract 23 is closed.** `AccessPath` exists — reporting-only, in a side table on
> `Function` keyed by the address `ValueId`, with a textual form that round-trips
> structurally. An out-of-bounds finding now reads `… through opaque as
> l2_bridge_t.bd_index`. The memory half needed no code: 021 §3 already returns bytes
> written through one view when read through another.
>
> **Nothing produces an `AccessPath` yet.** Lowering has the record layouts and the member
> names to build them, and 015 is where that belongs; until it does, the table is empty on
> every real module and findings say what they said before. That is the next obvious piece
> of value in this area and it is **not** a numbered contract, so it needs a decision
> rather than a checkbox.
>
> Five contracts remain in M1:
>
> 1. **020's 29, 30** — the union-pun checker (off by default per 040 §1: gcc defines it
>    and VPP depends on it, so enabling it on VPP is tens of thousands of findings about
>    working code) and the endianness `ConfigId`. Both are `chiero-check` work; wave 103
>    left that crate with a working shape to copy.
> 2. **021 c19**, **023's 7, 17, 21**.
> 3. **010 contract 19**, small and unblocked since wave 84.
>
> **Adding a required field to a widely-constructed struct is a 30-file edit.** `Function`
> gained `access_paths` and broke 79 literals across the test suite. Patch line-based, then
> **`cargo test --workspace --no-run`** — `cargo build` does not compile tests, which is how
> a mechanical rewrite once damaged five test files invisibly.
>
> **The recurring survivor shape, now three waves running: the fixture never reached the
> comparison the design exists for.** Wave 104's two were `render()` never printing an
> offset (so a round trip that lost every offset compared equal) and a fixture using one
> `ValueId` for both the access address and the path key (so keyed-lookup and
> fixed-lookup were indistinguishable). Before writing a mutation, ask what the *fixture*
> would have to look like for the mutant to survive — that is usually faster than running it.
>
> Mutation lessons still in force: **back up to a scratch copy, never `git checkout`**; **a
> mutant that does not compile is inconclusive, not a survivor**; **two guards only ever
> true together are equivalent mutants**; and a mutant that is a genuine no-op is neither —
> discard it rather than chasing a test for it.
>
> Owed and written down: nothing builds `AccessPath`s; the engine's own findings cite
> `ObjectId(N)`; `typeof` types to `Ty::Error` in sema; the parser's speculative type-name
> diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose their element width in
> `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 103, `21f5be7`) — 980 tests, M1 159/165, frontend 113/117
>
> **`chiero-check` exists.** 020 contract 18 is closed — both halves, in the two crates
> 020 §7 assigns them to. The syntactic scan runs over the AST in `chiero-lower`; the
> interprocedural `OrderDependence` checker is the first real 040 checker, and the pattern
> for the rest: a `chiero_exec::Checker` with per-path `CheckerState`, off unless
> registered, fixtures in `.cir`.
>
> Six contracts remain in M1:
>
> 1. **020's 23** — `Opaque`: a 40-byte `opaque[10]` written through one struct view and
>    read through another returns the bytes exactly, and an OOB finding names the second
>    view's member (`UnionMember { view }`).
> 2. **020's 29, 30** — the union-pun checker (off by default per 040 §1: gcc defines it
>    and VPP depends on it) and the endianness `ConfigId`. Both are `chiero-check` work
>    now that the crate has a shape.
> 3. **021 c19**, **023's 7, 17, 21**.
> 4. **010 contract 19**, small and unblocked since wave 84.
>
> **Reading the C standard paragraph is the work.** Contract 18(a) turns entirely on
> C11 6.5.16p3 — an assignment's write is sequenced after its operands' *value
> computations* but not after their *side effects* — which is the only reason `i = i + 1`
> and `i = i++` differ. A plausible implementation that treats every write alike gets one
> of them wrong and passes a suite that only tests the other.
>
> **A checker needs the module.** `CheckerCtx::module()` was added this wave; without it a
> finding can only cite an `ObjectId`, an engine-internal counter. That also **closes wave
> 102's owed item** for new findings — but `chiero-opt`'s transparency sweep still
> normalizes `ObjectId(N)` out of the *engine's* existing findings, and those still need
> the same treatment at their source.
>
> **`value_provenance_of` is not "which object does this pointer point at".** It records
> only `PtrToInt` casts, so it is empty for an ordinary address. A checker wanting the
> object reads the local's `Value::Ptr`.
>
> Mutation lessons still in force: **back up to a scratch copy, never `git checkout`**; **a
> mutant that does not compile is inconclusive, not a survivor**; **two guards only ever
> true together are equivalent mutants**. And the recurring shape of every survivor this
> wave and last: *the fixture never reached the comparison the design exists for* — one
> call writing one object twice, a golden with an out-of-order line, a phi that is its own
> incoming.
>
> Owed and written down: the engine's own findings cite `ObjectId(N)`; `typeof` types to
> `Ty::Error` in sema; the parser's speculative type-name diagnostic rollback is unpinned;
> `L`/`u`/`U` string literals lose their element width in `unquote`; 010's 18, 011's 12 and
> 012's 17 are deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 102, `9cb2fb7`) — 968 tests, M1 158/165, frontend 113/117
>
> **020 §9 is complete.** All three optional passes exist — `simplify_cfg`, `const_fold`,
> `mem2reg` — behind `chiero_opt::PASSES`, with §9's four prohibitions swept over every
> registered pass and the whole `.cir` corpus. `Phi` is an `InstKind` with verifier rules,
> a textual form, execution, and a corpus fixture. That closes **020's 16, 17 and 44**.
>
> Seven contracts remain in M1:
>
> 1. **020's 18** — order sensitivity, both halves: *syntactic* (`i = i++` sets
>    `order_sensitive` during lowering, `i = i + 1` sets neither) **and** *interprocedural*
>    (`f(g(), h())` mutating one global yields one finding from the 040 checker, mutating
>    different globals yields none). §10 says outright that testing only (a) is passed by
>    an implementation that does no analysis.
> 2. **020's 23** (`Opaque` — a 40-byte `opaque[10]` written through one struct view and
>    read through another), **29, 30** (the union-pun checker, the endianness `ConfigId`).
> 3. **021 c19**, **023's 7, 17, 21**.
> 4. **010 contract 19**, small and unblocked since wave 84.
>
> **Adding a variant to `InstKind` is a guided tour of every place that must know.** The
> compiler found all of them because `chiero-exec`'s dispatch, `chiero-cir`'s printer and
> `verify`'s `operands_of` have **no catch-all arms**, and `text_format`'s
> `every_variant_is_accounted_for` demanded a fixture. Keep it that way — `Switch` went
> unimplemented for eight waves behind an `_` arm.
>
> **A phi is the one instruction ordinary dominance is the wrong rule for.** Its operands
> live on edges, so `operands_of` returns nothing for it and `check_phis` verifies against
> the CFG instead. Anything new that walks uses must make the same exception.
>
> Mutation lessons still in force: **back up to a scratch copy, never `git checkout`** (a
> wave-100 campaign destroyed a whole GREEN and made four later verdicts fictional); **a
> mutant that does not compile is inconclusive, not a survivor**; **two guards only ever
> true together are equivalent mutants** — collapse them and write the fixture that reaches
> the survivor.
>
> Owed and written down:
> - **A finding's text names `ObjectId(N)`, an engine-internal counter.** `mem2reg` removes
>   allocas, so the same defect in the same program prints with different ids under a
>   different pass configuration — `passes.rs` normalizes them to keep the transparency
>   sweep meaningful, which is a workaround, not a fix. Findings should name the variable:
>   a span, a declaration.
> - `typeof` types to `Ty::Error` in sema; the parser's speculative type-name diagnostic
>   rollback is unpinned; `L`/`u`/`U` string literals lose their element width in
>   `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately uncovered `#[ignore]`d
>   metrics.
>
> ### Earlier (wave 101, `6d7cda9`) — 949 tests, M1 157/165, frontend 113/117
>
> **`chiero-opt` exists.** 020 §9's `simplify_cfg` and `const_fold` are implemented behind
> a **registry**, and §9's four prohibitions — no dropped `Marker`, no merge across a
> `Volatile` access, no discarded `Span`, no widened `LoadBits`/`StoreBits` — are swept
> over every registered pass and the whole checked-in corpus. That closes 020's **17 and
> 44**. Eight contracts remain in M1:
>
> 1. **020 c16 — `mem2reg`**, the last of §9's three passes. It needs a `Phi` instruction
>    the IR does not have yet, which only this pass may emit. **Register it in
>    `chiero_opt::PASSES` when it lands**: `the_registry_holds_every_pass_the_spec_names`
>    asserts `pass("mem2reg").is_none()` today and will fail until you do, which is the
>    point — an implemented-but-unregistered pass is covered by no sweep in that file.
> 2. **020's 18** (order sensitivity, both halves — syntactic during lowering *and*
>    interprocedural via the 040 checker), **23** (`Opaque`), **29, 30** (the union-pun
>    checker, the endianness `ConfigId`).
> 3. **021 c19**, **023's 7, 17, 21**.
> 4. **010 contract 19**, small and unblocked since wave 84.
>
> **A test at the wrong layer is a design error, not a dependency problem.** Wave 101's RED
> lowered C for its fixtures, which made a vertical dev-depend on the frontend; `check-deps`
> caught it. Rewriting against `.cir` was the correction — 020's contracts are written about
> CIR and the corpus is already in that language.
>
> **A mutation harness backs up to a scratch copy, never to git** (wave 100 destroyed a
> whole GREEN with `git checkout --`, and the mutants that followed ran against reverted
> source, so four verdicts were fictional). **A mutant that fails to compile is
> inconclusive, not a survivor.** And **two guards that are only ever true together are
> equivalent mutants of each other** — neither can be killed while both stand; collapse
> them and write the fixture that reaches the one that remains.
>
> Standing instructions that keep earning themselves: **read a golden diff when it
> changes**, and **add a `probe` fixture to `differential.rs` for anything with a
> computable value**.
>
> Owed and written down: `typeof` types to `Ty::Error` in sema; the parser's speculative
> type-name diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose their element
> width in `unquote`; 010's 18, 011's 12 and 012's 17 are deliberately uncovered
> `#[ignore]`d metrics.
>
> ### Earlier (wave 100, `d1c19af`) — 938 tests, M1 155/165, frontend 113/117
>
> **Lowering no longer drops anything in ordinary C.** Wave 100 closed 020 contract 14 and
> the `GlobalInit` gap 020 §6 records: string literals are pooled read-only `.str.N`
> globals carrying their bytes, `sizeof` folds instead of yielding `Undef`, wide `case`
> ranges become guards rather than 10 000 `Switch` entries, and a returned value is
> converted to the function's return type.
>
> **Correction to wave 99's list below: 020 c29/c30 are *not* `GlobalInit`.** They are the
> union-pun checker and the endianness `ConfigId`. `GlobalInit` was an owed gap in 020 §6,
> and it is now closed. Ten contracts remain in M1:
>
> 1. **020's 16, 17, 18** — the optional passes (`mem2reg`, `simplify_cfg`) with their
>    observational-transparency requirement, and **23** (`Opaque`, so inline asm is
>    representable), **44** (`Marker::Line` at instruction position, reparsing into
>    `gcov_lines`).
> 2. **020's 29, 30** — the union-pun checker and the endianness `ConfigId`.
> 3. **021 c19**, **023's 7, 17, 21**.
> 4. **010 contract 19**, small and unblocked since wave 84.
>
> **A mutation harness backs up to a scratch copy, never to git.** Wave 100's first
> campaign reverted each mutant with `git checkout -- <file>` and destroyed every
> uncommitted GREEN edit mid-run; worse, the mutants that followed ran against
> already-reverted source, so four verdicts were fictional and read as real. And **a mutant
> that fails to compile is inconclusive, not a survivor** — one read as SURVIVED for
> exactly that reason, which would have sent me writing a test for a gap that did not
> exist. Both checks are now in the harness.
>
> Two standing instructions that keep earning themselves: **read a golden diff when it
> changes** — wave 100's `sizeof` drop was found that way, not by any test — and **add a
> `probe` fixture to `differential.rs` for anything with a computable value**.
>
> Owed and written down: `typeof` types to `Ty::Error` in sema; the parser's speculative
> type-name diagnostic rollback is unpinned; `L`/`u`/`U` string literals lose their element
> width in `unquote` (byte encodings only); 010's 18, 011's 12 and 012's 17 are
> deliberately uncovered `#[ignore]`d metrics.
>
> ### Earlier (wave 99, `03f74cc`) — 934 tests, M1 153/165, frontend 113/117
>
> **The frontend is done.** 013, 014 and 015 are all at 100%; C source becomes verified CIR
> that matches goldens, agrees with gcc on what it computes, and attributes lines the way
> gcov does. What remains in 010–012 is four contracts, every one deliberately owed:
> 010's 18 (peak memory, needs a large fixture) and 19 (per-`ConfigId` sites, unblocked
> since wave 84), 011's 12 and 012's 17 (both `#[ignore]`d throughput/corpus metrics that
> 070 §195 counts as uncovered on purpose).
>
> **The front now moves back to M1** — 153/165, twelve contracts left:
>
> 1. **020 c29/c30 — `GlobalInit` and `Linkage` in the text format.** Worth doing first:
>    string literals currently lower to `Undef` because a `Global` carries no initializer,
>    and that is the **last construct in ordinary C that lowering silently drops**.
> 2. **020's 14, 16, 17, 18** — the optional passes (`mem2reg`, `simplify_cfg`) with their
>    observational-transparency requirement, and 23 (`Opaque`, so inline asm is
>    representable), 44 (`Marker::Line` at instruction position, which reparses into
>    `gcov_lines`).
> 3. **021 c19**, **023's 7, 17, 21**.
> 4. **010 contract 19**, small and long-unblocked.
>
> Two standing instructions that have earned themselves and apply to all of the above:
> **read a golden diff when it changes** — one that cannot be explained in a sentence is a
> shape that moved for a reason nobody chose; and **add a `probe` fixture to
> `differential.rs` for anything with a computable value**, because a shape assertion says
> the instructions are arranged correctly and only gcc says the program is right.
>
> Owed and written down: string literals lower to `Undef` (needs `GlobalInit`); `typeof`
> types to `Ty::Error` in sema; the parser's speculative type-name diagnostic rollback is
> unpinned.
>
> ### Earlier (wave 98, `75a7a4a`) — 931 tests, frontend 112/117
>
> **015 is 24/25.** Only **contract 9c** remains: `goto` *into* a scope enters it exactly
> once, and a backward `goto` that re-enters creates a **new generation** of its objects.
> 015 §4 says re-entering matches the loop-body rule in §3, so the shape to copy is the one
> a loop body already has.
>
> After 9c the frontend is complete — 010 through 015, every contract cited. Then:
>
> 1. **010 contract 19** — per-`ConfigId` expansion sites, unblocked since wave 84.
> 2. **M1's remaining 13**: 020's 14/16/17/18/23/29/30/44 (the optional passes, `Opaque`,
>    `GlobalInit`/`Linkage` in the text format, `Marker::Line` at instruction position),
>    021 c19, 023's 7/17/21.
> 3. **020 c29/c30 — `GlobalInit`** is worth doing early: string literals currently lower
>    to `Undef` because a `Global` carries no initializer, and that is the last construct
>    in ordinary C that lowering silently drops.
>
> **Read the goldens when they change.** Wave 98 moved two of them and the diff was exactly
> contract 12 — `i` from scope 0 to a new scope 1, `.scope enter 1`/`exit 1` bracketing the
> loop, the body renumbering to 2. A diff that cannot be explained in one sentence is a
> shape that moved for a reason nobody chose.
>
> Owed and written down: string literals lower to `Undef` (needs 020's `GlobalInit`);
> `typeof` types to `Ty::Error` in sema; the parser's speculative type-name diagnostic
> rollback is unpinned.
>
> ### Earlier (wave 97, `d4fe3ce`) — 926 tests, frontend 108/117
>
> **015 is 20/25 and the corpus round trip closes.** All four files in
> `tests/corpus/c/` lower to CIR that verifies, matches a golden in
> `tests/corpus/lowered/`, and survives print/parse/print. `&x`, dereference and pointer
> indexing all work. Only 8, 9c, 12, 14 and 20 remain.
>
> **Read the goldens, not just the test results.** Wave 97's sharpest finding —
> declared functions' parameters were never typed, so the intrinsics printed as
> `chiero_make_symbolic(i64, i64, i64)` — was found by *looking at* a golden after
> blessing it. No test asserted anything about a declared signature; the wrong one was
> simply visible. `CHIERO_BLESS=1 cargo test -p chiero-lower --test goldens`.
>
> Next, in rough order:
>
> 1. **Contracts 8, 12, 14, 20** — statement expressions (`({ ... })`, 217 VPP files, and
>    `StmtExpr` still types to `Ty::Error` in sema so this is a two-crate change),
>    `for`-scope, VLAs (`AllocaDyn` at the declaration point with the size operand
>    dominating it), and refusing a nested function with **exactly one** diagnostic.
> 2. **Contract 9c** — `goto` *into* a scope, and a backward `goto` creating a new
>    generation of its objects.
> 3. **015 is then complete.** After that: **010 contract 19**, and **M1's remaining 13**
>    (020's 14/16/17/18/23/29/30/44, 021 c19, 023's 7/17/21).
>
> Owed and written down: string literals lower to `Undef` — a real one needs 020's
> `GlobalInit`, which is also uncited (020 c29/c30); `typeof` types to `Ty::Error` in sema;
> VLA bounds are treated as flexible; the parser's speculative type-name diagnostic
> rollback is unpinned.
>
> ### Earlier (wave 96, `a12bde3`) — 924 tests, frontend 107/117
>
> **015 is 19/25.** Goldens exist at `tests/corpus/lowered/` (7 files), `?:` and aggregate
> initializers lower, and every golden reparses and verifies. Only 8, 9c, 12, 14, 20 and
> 22 remain.
>
> **Blessing a golden:** `CHIERO_BLESS=1 cargo test -p chiero-lower --test goldens`. Read
> the diff first — a golden that changes for a reason nobody can state is a shape that was
> never fixed, and re-blessing without reading is how that gets lost.
>
> Next, in rough order:
>
> 1. **Contract 22 — the corpus round trip.** Its one remaining blocker is that
>    `tests/corpus/c/`'s four files call `chiero.h` intrinsics
>    (`chiero_make_symbolic`, `chiero_assume`, `chiero_assert`), which lowering does not
>    model. 024 owns the intrinsic registry and `chiero-exec` already implements them, so
>    the work is to lower a call to one into whatever CIR shape the engine expects — check
>    `crates/chiero-exec/tests/harness_intrinsics.rs` for that shape before designing one.
> 2. **Contracts 8, 12, 14, 20** — statement expressions (`({ ... })`, 217 VPP files),
>    `for`-scope, VLAs (`AllocaDyn`), and refusing a nested function with exactly one
>    diagnostic.
> 3. **Contract 9c** — `goto` *into* a scope, and a backward `goto` creating a new
>    generation of its objects.
> 4. **010 contract 19**; **M1's remaining 13**.
>
> Owed and written down, ordered by how likely each is to bite: `&a[i]` and pointer
> arithmetic are **not lowered**, which is why sign-vs-zero extension of an array index is
> unpinned (a mutation survives); `StmtExpr`/`typeof` type to `Ty::Error` in sema; VLA
> bounds are treated as flexible; the parser's speculative type-name diagnostic rollback is
> unpinned.
>
> ### Earlier (wave 95, `2b6e3ef`) — 920 tests, frontend 106/117
>
> **Aggregates and bit-fields lower** (015: 18/25). Struct copies are one `CopyMem` of the
> layout size, bit-field access takes its `BitRange` **from `RecordLayout`** and nowhere
> else, and member/array addressing reads field offsets from the same place. Only 2, 8, 9c,
> 12, 14, 20 and 22 remain.
>
> Next, in rough order:
>
> 1. **Golden `.cir` files — contracts 2 and 22.** This is what makes M1's hand-written
>    fixtures and M2's real lowering the same language, and it is the last structural piece.
>    015 §5 notes the `.line` directive is how a hand-written fixture populates
>    `gcov_lines`, so the golden format already has a place for what wave 94 computes.
> 2. **Contracts 8, 12, 14, 19, 20** — statement expressions, `for`-scope, VLAs, aggregate
>    initializers (`struct S x = {1, 2};` is **not lowered**, and a differential probe had
>    to be rewritten because of it), and refusing nested functions with one diagnostic.
> 3. **Contract 9c** — `goto` *into* a scope, and a backward `goto` creating a new
>    generation of its objects.
> 4. **010 contract 19**; **M1's remaining 13** (020's 14/16/17/18/23/29/30/44, 021 c19,
>    023's 7/17/21).
>
> Owed and written down, in rough order of how likely they are to bite:
> **aggregate initializers are not lowered**; `&a[i]` and pointer arithmetic are not
> lowered, which is why **sign-vs-zero extension of an array index is unpinned** (a
> mutation survives — distinguishing them needs a negative index);
> `InitList`/`StmtExpr`/`typeof` type to `Ty::Error` in sema; VLA bounds are treated as
> flexible; the parser's speculative type-name diagnostic rollback is unpinned.
>
> ### Earlier (wave 94, `ab34e1b`) — 916 tests, frontend 103/117
>
> **`gcov_lines` is computed and checked against `gcov --json-format`** (015: 15/25). That
> closes the join point of §4.1 → 030 → 031 → 032: a statement in a macro body is
> attributed to the `.c` line where the macro was *used*, and a `static inline` in a header
> keeps its header lines. `Inst` now carries a **recorded** `generated` flag (020 c15).
>
> **Two process notes from this wave, both worth not repeating.**
> `cargo build --workspace` does **not** compile tests — a scripted field addition across
> 96 sites left five test files broken and nothing surfaced until `cargo test`. Use
> `cargo test --workspace --no-run` after any mechanical edit. And an oracle helper that
> returns `Option` will be wrapped in `if let Some(...)` at the call site and then silently
> skip; `gcov` was in fact failing in two tests that reported success. Make oracles panic.
>
> Next, in rough order:
>
> 1. **Aggregates and bitfields — 015 contracts 6 and 7.** A struct assignment is one
>    `CopyMem`, and bitfield access uses the `BitRange` **from `RecordLayout`** — lowering
>    must never re-derive a bit offset, so there is exactly one place to be wrong. Sema
>    already computes the layout and it is gcc-verified over 520 records.
> 2. **Golden `.cir` files — contracts 2 and 22.** This is what makes M1's hand-written
>    fixtures and M2's real lowering the same language, and 015 §5 notes the `.line`
>    directive is how a hand-written fixture populates `gcov_lines`.
> 3. **Contracts 8, 12, 14, 19, 20** — statement expressions, `for`-scope, VLAs, aggregate
>    initializers, and refusing nested functions.
> 4. **Contract 9c** — `goto` *into* a scope, and a backward `goto` creating a new
>    generation of its objects. Not implemented.
> 5. **010 contract 19**; **M1's remaining 13**.
>
> Owed and written down: `InitList`/`StmtExpr`/`typeof` type to `Ty::Error` in sema; VLA
> bounds are treated as flexible; the parser's speculative type-name diagnostic rollback is
> unpinned.
>
> ### Earlier (wave 93, `8c38524`) — 912 tests, frontend 99/117
>
> **Lowering handles all of C's control flow** (015: 11/25) — scopes, `switch`, `goto`,
> labels, `break`, `continue` — and `chiero-exec` can now execute a `Switch`, which it
> could not before this wave. Nothing had ever produced one: M1's fixtures are hand-written
> `.cir` and nobody hand-writes a switch.
>
> **The lesson that generalizes: a catch-all match arm hides a missing feature.** The
> engine's terminator dispatch had an `_ =>` that reported "unsupported terminator" at run
> time, and `Switch` sat inside it for eight waves. The arm is gone, so the *compiler* now
> rejects an unhandled `Terminator` variant. Worth checking for the same shape elsewhere —
> `chiero-lower`'s statement and expression dispatch still has one, deliberately (015 §7's
> refuse-rather-than-lower-wrongly), but it should be a short list and it is not audited.
>
> Next, in rough order:
>
> 1. **`gcov_lines` — 015 §5 and contracts 15, 15b, 16, 17.** This is the join point of the
>    whole test-selection story and the reason §4.1's headline claim works. Read 15b before
>    starting: contract 17's subset property is **vacuously satisfied by the empty set**, so
>    17 and 15b have to be written together or an implementation that emits no lines at all
>    passes 17.
> 2. **Aggregates and bitfields — contracts 6 and 7.** A struct assignment is one `CopyMem`,
>    and bitfield access uses the `BitRange` **from `RecordLayout`** — lowering must never
>    re-derive a bit offset, so there is exactly one place to be wrong.
> 3. **Golden `.cir` files — contracts 2 and 22.**
> 4. **Contracts 8, 12, 14, 19, 20** — statement expressions, `for`-scope, VLAs,
>    aggregate initializers, and refusing nested functions.
> 5. **010 contract 19**; **M1's remaining 13**.
>
> Owed and written down: `InitList`/`StmtExpr`/`typeof` type to `Ty::Error` in sema;
> contract 9c (`goto` *into* a scope, and a backward `goto` creating a new generation) is
> not implemented; VLA bounds are treated as flexible; the parser's speculative type-name
> diagnostic rollback is unpinned.
>
> ### Earlier (wave 92, `adeb1c0`) — 906 tests, frontend 94/117
>
> **The differential oracle exists and it is the tool to reach for from now on.**
> `crates/chiero-lower/tests/differential.rs`: write `int probe(void) { … }`, and the
> harness lowers it, runs it with `chiero-exec`, compiles the same C with gcc, runs that,
> and compares one integer. No symbolic-input machinery is needed because the probe takes
> no arguments. **It found four defects in its first hour**, including one — `++x` matching
> the general `Unary` arm and evaluating to `x` — that every structural test in the project
> passes happily.
>
> **When adding a lowering contract, add a `probe` fixture for it.** A shape assertion says
> the blocks are arranged correctly; only the oracle says the program computes the right
> number. And make the fixture **consume** the value: `i++` as a `for` step discards it,
> which is exactly why the pre/post distinction went untested through two campaigns.
>
> Next, in rough order:
>
> 1. **Scopes — 015 contracts 9, 9b, 9c, 10, 11.** Read 9b first: contracts 9–11 test scope
>    *exits* only, so an implementation that never enters a scope on a `switch` case path
>    passes all of them.
> 2. **`switch`, `goto`, `break`, `continue`** — currently **refused** by lowering with a
>    diagnostic (015 §7) rather than lowered wrongly. `switch` is contract 18.
> 3. **`gcov_lines` — 015 §5 and contracts 15, 15b, 16, 17.** §5 is the join point of the
>    whole test-selection story; 15b warns that 17's subset property is *vacuously
>    satisfied by the empty set*, so the two must be tested together.
> 4. **Golden `.cir` files — contracts 2 and 22.**
> 5. **010 contract 19**; **M1's remaining 13**.
>
> Owed and written down: `InitList`/`StmtExpr`/`typeof` type to `Ty::Error`; VLA bounds are
> flexible; aggregates and bitfields are not lowered (contracts 6 and 7); the parser's
> speculative type-name diagnostic rollback is unpinned.
>
> ### Earlier (wave 91, `5e9b85a`) — 898 tests, frontend 93/117
>
> **C source reaches CIR.** `chiero-lower` exists (015: 5/25) — functions, blocks,
> statements, expressions, the §2.1 short-circuit shape, left-to-right evaluation — and
> every fixture produces CIR the 020 §8 verifier accepts. The pipeline is now
> preprocess → parse → types → layout → conversions → lower, end to end.
>
> **Next is 015 contract 5's differential oracle against gcc, and it is the priority.**
> Wave 91 ended with two mutations alive — ignoring signedness (always `SDiv`) and always
> zero-extending — and *neither can be killed by a structural test*, because the block
> shapes are identical and only the computed values differ. Every assertion in
> `tests/shapes.rs` is about shape. The answer already exists in the tree:
> **`chiero-exec` runs CIR**, and it is in the same layer band as `chiero-lower`
> (001 §4 rule 2 allows the dev-dependency, rule 8 explicitly contemplates it). Lower a
> fixture, run it, compile the same C with gcc, run that, compare. That closes contract 5
> and turns every later shape contract into a semantic one for free.
>
> Then, in rough order:
>
> 1. **Scopes — 015 contracts 9, 9b, 9c, 10, 11.** Read 9b before implementing: contracts
>    9–11 test scope *exits* only, so an implementation that never enters a scope on a
>    `switch` case path passes all of them.
> 2. **`gcov_lines` — 015 §5 and contracts 15, 15b, 16, 17.** §5 is the join point of the
>    whole test-selection story, and 15b warns that contract 17's subset property is
>    *vacuously satisfied by the empty set*, so the two must be tested together.
> 3. **Golden `.cir` files — contracts 2 and 22**, which is what makes M1's hand-written
>    fixtures and M2's real lowering the same language.
> 4. **010 contract 19**; **M1's remaining 13**.
>
> Owed and written down: `InitList`/`StmtExpr`/`typeof` type to `Ty::Error`; VLA bounds are
> flexible; the parser's speculative type-name diagnostic rollback is unpinned; `switch`,
> `goto`, `break`, `continue` and `asm` are **refused** by lowering with a diagnostic
> (015 §7) rather than lowered wrongly.
>
> ### Earlier (wave 90, `76805d0`) — 888 tests, frontend 88/117
>
> **014 is complete, 20/20, and with it the frontend.** 010–014 are done bar 010's two
> owed contracts: preprocessor, parser, types, layout, conversions, linkage. Everything
> below is verified against gcc or against real VPP, or both.
>
> **Next is 015 — AST→CIR lowering (0/25), and it is the milestone.** It is the last thing
> between the frontend and the symbolic core, and M1's engine has been waiting for it
> since wave 84. What it can now assume, none of which it has to re-derive:
>
> - every implicit conversion is already an explicit `Cast` with a recorded reason
>   (014 c11) — 014 §5's whole point is that lowering never infers one;
> - every record layout agrees with gcc over 520 real VPP records (014 c12);
> - `GlobalTable` resolves names across TUs, with `static` per-TU and `extern` shared
>   (014 c15/c16) — this is what 031's call graph spans TUs with;
> - `Block::gcov_lines` is **015 §5's own** responsibility and must be settled before
>   M1's hand-written fixtures entrench a different convention.
>
> Then: **010 contract 19** (unblocked since wave 84 — `ConfigId` lives in `chiero-pp`);
> **M1's remaining 13** (020's 14–18/23/29/30/44, 021 c19, 023's 7/17/21).
>
> Owed and written down: `ExprKind::InitList` and `StmtExpr` type to `Ty::Error`
> (a braced initializer needs the target type threaded through, a statement expression the
> block's last value — both are 015's shape); `typeof` resolves to `Ty::Error`; VLA bounds
> are treated as flexible; the parser's speculative type-name diagnostic rollback is
> unpinned.
>
> ### Earlier (wave 89, `6d50b57`) — 883 tests, frontend 83/117
>
> **014 is 15/20 and the typed AST exists.** Every implicit conversion is an explicit
> `Cast` with a recorded *reason*, verified over **2234 arithmetic operations of real VPP**
> — none is left with operands C would have had to convert. The layout gate is still green
> at 2909 assertions over 520 records.
>
> Next, in order:
>
> 1. **014's 14–18**: tentative definitions and linkage (14–16 — note that `static`
>    functions with the same name in *different* TUs must stay distinct `GlobalId`s, a real
>    hazard in VPP where short static helper names repeat), address constants (17), and
>    `__builtin_constant_p` (18). 14–16 need a **cross-TU** symbol table, which is the
>    first thing in this project that spans TUs and is what lets 031's call graph do so.
> 2. **015 — AST→CIR lowering** (0/25). The frontend is now complete enough to feed it:
>    parse → types → layout → conversions, all explicit.
> 3. **010 contract 19**; **M1's remaining 13**.
>
> Owed and written down rather than forgotten: `ExprKind::InitList` and `StmtExpr` type to
> `Ty::Error` (a braced initializer needs the target type threaded through, a statement
> expression needs the block's last value — both are more 015's shape than 014's);
> `typeof` still resolves to `Ty::Error`; VLA bounds are treated as flexible; and the
> parser's speculative type-name diagnostic rollback is still unpinned.
>
> ### Earlier (wave 88, `2b1e308`) — 875 tests, frontend 81/117
>
> **The 014 layout gate is live and green**: 2909 generated `_Static_assert`s over **520
> real VPP records**, zero rejected by gcc. `cargo test -p chiero-sema --test
> vpp_layout_gate`. The corpus now lives at the workspace `tests/corpus/vpp/` (001 §6),
> since both `chiero-parse` and `chiero-sema` read it.
>
> Next, in order:
>
> 1. **014 contract 11 — every implicit conversion becomes an explicit `Cast`.** This is
>    what 015 waits on: lowering must never infer a conversion, because one it gets wrong
>    is an invisible semantic bug, and making conversions explicit is what makes CIR
>    unambiguous about bit-widths for the solver. It needs full expression typing, which
>    also unblocks `typeof` (currently `Ty::Error`).
> 2. **014's 14–18 and 20** — tentative definitions and linkage (`GlobalId`, and *static*
>    functions in different TUs must stay distinct), address constants,
>    `__builtin_constant_p`, and `Ty::Error` not cascading.
> 3. **015 — AST→CIR lowering** (0/25).
> 4. **010 contract 19**; **M1's remaining 13**.
>
> Two things wave 88 established that are easy to get wrong later:
> **gcc has two alignments for a vector** — `_Alignof(u64x4)` is 16 but a `u64x4` member
> is *placed* at 32 — so `Ty::Vector` stores the placement value and `align_of_ty` applies
> the psABI cap. And that cap (`TargetConfig::max_vector_align`) is **target data**: 16
> baseline, 32 under `-mavx`, 64 under `-mavx512f`. That is 060's multiarch 1:N mapping
> arriving early — one source file, several layouts, distinguished by `ConfigId`.
>
> ### Earlier (wave 87, `ffc6da7`) — 874 tests, frontend 80/117
>
> **`chiero-sema` has a layout engine and gcc agrees with it** (014: 12/20). Records,
> bit-fields including straddling and zero-width, packed, `aligned` at record and member
> level, unions, flexible arrays, enum widening, target-dependent `char` signedness, and
> typed integer constant evaluation.
>
> **014 §7's differential is the technique to keep using.** The harness emits
> `_Static_assert`s for size/alignment/offsets and compiles them, *and* — because
> `__builtin_offsetof` is ill-formed on a bit-field — writes all-ones into each bit-field
> at run time and compares the object's bytes against the mask chiero predicts. Layout is
> the one place where a hand-written expected number is worthless, since the expectation
> is exactly what a layout bug corrupts.
>
> Next, in order:
>
> 1. **Finish 014.** Uncited: 11, 12, 14, 15, 16, 17, 18, 20. Contract 11 (every implicit
>    conversion becomes an explicit `Cast`) is the big one — it is what makes CIR
>    unambiguous about bit-widths, and 015 cannot be trusted without it. 14–16 are name
>    resolution and linkage; 17–18 are address constants and `__builtin_constant_p`.
>    **Contract 12 is the gate**: point the `_Static_assert` generator at every record in
>    the VPP corpus, which is already in-tree at
>    `crates/chiero-parse/tests/corpus/vpp/`.
> 2. **015 — AST→CIR lowering** (0/25), which finally connects the frontend to the
>    symbolic core.
> 3. **010 contract 19**, unblocked since wave 84.
> 4. **M1's remaining 13** — 020's 14–18/23/29/30/44, 021 c19, 023's 7/17/21.
>
> Owed and recorded, not forgotten: `typeof` resolves to `Ty::Error` until contract 11
> lands; a VLA bound is treated as flexible; `const_eval` standalone cannot size a tag;
> and the parser's speculative type-name diagnostic rollback is still unpinned.
>
> ### Earlier (wave 86, `5d50c7e`) — 860 tests, frontend 68/117
>
> **013 is complete, 20/20, and the parser eats real VPP.** Six vppinfra headers,
> **1,702,754 tokens of unmodified upstream C, zero diagnostics and zero panics**, AST at
> 1.65× the token stream against a 10× bound. `chiero-ast` and `chiero-parse` were
> one-line stubs at the start of wave 85.
>
> The corpus lives in-tree at `crates/chiero-parse/tests/corpus/vpp/` — 28 files, the
> transitive VPP-local include closure of those six headers, copied verbatim at VPP commit
> `7fe9c26` with provenance and Apache-2.0 attribution in `PROVENANCE.md`. **Do not edit
> those files**; a fixture edited until it parses proves that the parser handles the edit.
> Extend the corpus the same way — compute a closure, copy verbatim, pin the counts.
>
> Next, in order:
>
> 1. **014 — types, layout and name resolution** (0/20). The parser hands it a *syntactic*
>    tree on purpose: no resolved types, no folded constants. 014 owns `packed`/`aligned`
>    (every struct offset depends on getting them right), plain `char`'s signedness, phase
>    5's escape evaluation, and the `ExtFloat { bits, fmt }` formats the parser records but
>    does not interpret.
> 2. **015 — AST→CIR lowering** (0/25), which is what finally connects the frontend to the
>    symbolic core and makes the whole thing one pipeline rather than two halves.
> 3. **010 contract 19**, unblocked since the wave-84 merge: `ConfigId` lives in
>    `chiero-pp` (001 §180), so `CookedSite.config` finally has somewhere to come from.
> 4. **M1's remaining 13** — 020's 14–18/23/29/30/44, 021 c19, 023's 7/17/21.
>
> Owed, and small: the parser's speculative type-name parse rolls back its diagnostics as
> well as its cursor, and that rollback is **unpinned** — an abstract declarator may be
> empty, so `type_name()` essentially cannot fail once `starts_type_name()` was true, and
> no fixture yet makes the speculation fail *and* leave a diagnostic.
>
> Three rules this front keeps re-teaching:
> **a mutation no fixture can observe is not a killed mutation** (wave 85's span-splice
> mutant needed a third fixture, because a macro body's byte positions sit *below* its use
> site; wave 86's asm-label mutant was invisible to a 1.7M-token corpus because a wrong
> label still parses cleanly). **A real corpus finds what fixtures cannot** — all five
> wave-86 defects were constructs in *every* TU in existence, and none had been imagined.
> And a differential number taken without `gcc -dM -E` fed through `Config::defines` is
> about code neither tool ran (wave 84).

**You are here:** ✅ **ALL 24 SPEC DOCUMENTS ARE WRITTEN AND COMMITTED.** The spec set is
complete at **draft-3** (post-review, three review waves applied). ~7030 lines,
24 numbered documents + index, **497** numbered testable contracts.

**✅ SPEC GATE PASSED — 2026-07-27.** The user read the specs ("looks reasonable to me")
and granted **full autonomy**. §2 decision 3 is now discharged: run free, no check-ins
until the first vertical is green. Do not re-ask for approval.

Autonomy does **not** suspend the §8 TDD protocol or the §8.1 subagent rules — red before
green, both adversarial reviews, ≤3 concurrent agents. Full autonomy means no permission
requests, not fewer gates.

1. ~~Write the specs~~ — done. If the user asks for changes, amend in place with a
   `spec:` commit; the specs are normative and the README says deviations are amended in
   the same commit as the deviation.
   All cross-crate promises made by earlier specs were discharged by later ones and
   spot-checked: 041 delivers the locality analysis 014/021 point at; 040 carries the
   `union-pun` + order-dependence checkers and implements 025 §3's findings
   target-agnostically; 050 exposes the recipe ops; 060 ships the `.recipe` catalogue as
   data plus the lock/thread models 025 needs; 070 owns the `trybuild` fidelity test,
   the differential harness and the recipe-fixture gate; 080 places the recipe engine
   early (M3) since its tier-1 sweep needs only the frontend.

**Empirically verified while writing 030 — do not re-derive (§4.1's premise is now a
measured fact, not a claim):** with gcc 13.3, a macro defined in a header and expanded in
a `.c` gets **no coverage record at all** for the macro-body line — not a zero count, no
entry — while a `static inline` function in the *same header* does get its own file and
line counts. So the boundary is exactly: coverage follows the **expansion site** for
macros and the **definition site** for functions. 030 contract 2 pins this as a
regression test. Also verified: gcno magic `oncg` / gcda `adcg`, version tag `*33B`
(= gcc 13.3), a **shared stamp u32 that is the staleness key**, gcov JSON
`format_version: "1"` with the schema in 030 §3, output file is `<objstem>.gcov.json.gz`
(NOT `<src>.gcov.json.gz`), and JSON `branches[]` entries carry only
`{count, throw, fallthrough}` — **no target block**, which is precisely why the native
`.gcno` path is required for arc-level work.
2. **Adversarial spec review — WAVE 1 COMPLETE, findings applied 2026-07-27.**

   Three reviewers reported. Their empirical claims were **re-verified here before acting**
   and every one held (clang's nested backtrace; `bvsdiv -5 0 == 1` and
   `bvurem/bvsrem x 0 == x`; the wraparound case being SAT; `vlib_buffer_ptr_from_index`;
   `rdtsc` register outputs; 2552 `va_list *`; zero real computed gotos in VPP).
   Applied across commits `b86983f`, `8eed056`, `c8c978d`, `306da36`.

   **The findings that mattered most**, so a fresh context knows what nearly shipped:
   - **`bvsdiv`/`bvurem`/`bvsrem` by zero** were specified as all-ones; only `bvudiv` is.
     The independent evaluator would have shared the folder's error and *validated* models
     built on it — invisible to model validation, detectable only via z3.
   - **`solver-lite`'s `Unsat` had nothing backing it.** `Sat` self-certifies via model
     evaluation; `Unsat` doesn't, so it needed a syntactic fragment restriction plus a
     wrap-safety rule. A saturating interval domain reports false `Unsat` on
     `x>250 ∧ y=x+10 ∧ y<10`, which is satisfiable.
   - **Independence slicing rested on an unstated invariant** (all other components known
     satisfiable) that chiero breaks in three places by adding unproven constraints.
   - **The whole-tree expansion index dangled**: dropping per-TU tables while keeping
     `by_macro` left `ExpnCtx` handles into freed storage — the headline feature broken at
     exactly the scale where it matters.
   - **`Finding` was defined twice**, incompatibly, and the `chiero-check` copy was
     illegal under the layering rules.
   - **025 contract 11 said `fidelity <= Bounded`**, which given `Exact < Bounded`
     *permits* `Exact` — the sign was inverted and it forbade nothing.
   - **`Precision::Approximate` had no mechanical effect**, so a run calling `scanf` could
     finish `Exact` and report "no bugs exist" as a proof.
   - **`PtrToInt`/`IntToPtr` laundered provenance** into a *different real object* when
     the OOB distance exceeded the guard gap.
   - **Unconstrained pointers were concretized and reported `Bounded`**, which would have
     analysed the wrong memory for a whole function — and it fires on `vlib_get_buffer`,
     the most-executed function in VPP's data plane. Hence arenas (021 §5.2).

   **Wave 2 — COMPLETE, findings applied in `99a58a0`.** The recipe/tool review found
   that the DSL cannot express two of its three advertised catalogue rules (verified:
   1638 of 3350 `clib_error_return` sites are inline returns needing a result binder the
   grammar lacks; 655 of 749 pool functions do `pool_get` or `pool_put` but not both, so
   a per-function typestate is structurally wrong). 042 §4.3 now states what the DSL
   covers and routes the rest to Rust checkers. Also found: a verified tier-1 recall hole
   in `vnet/interface_cli.c`, a false-positive `double_free` in the flagship example
   (`unformat_free` memsets), and a job-response path where a 5%-complete run could
   report `proven: true` over an empty findings list.

   **Wave 3 — COMPLETE, applied in `284ef54` (code) and `479ac05` (specs).**

   *The span implementation review was the most valuable single review so far.* It
   demonstrated **six deliberate breakages that all 32 tests accepted**, including
   `expansion_sites` returning every expansion in the TU — i.e. "re-run every test for
   any change", indistinguishable from having no test selection. Also found: `origin`
   never read `body_extent` (so any span with a non-ROOT ctx was a "body token"), the
   fixture had drifted until that stopped mattering, the cooked index stored one entry
   per expansion *event* rather than per *site* (inverting 010 §6.3's entire
   justification), macro identity was reverse-engineered from `def_span.lo` so a builtin
   resolved to whichever file occupies offset 0, and `intern_file` did not normalize
   despite its doc comment.

   *Fable's spec review found a critical error in 015 — the document written to prevent
   exactly that class of error.* Its `gcov_lines` rule dropped lines from headers, which
   would have zeroed coverage correlation for every `static inline` in vppinfra, and 015's
   own contracts could not detect it because the subset property is vacuous on the empty
   set. Also: scope markers specified on every exit but only the lexical entry (any
   `switch` with a local breaks), the `&&` slot typed `i1` when `a && b` is `int`, a
   contract contradicting C11's zero-initialization rule, arena geometry conflating VPP's
   64-byte index unit with its ~2.5 KB element pitch, and M1's gate listing contract
   ranges that excluded every contract the reviews had added.

   **Wave 4 — span re-review COMPLETE, applied in `d1c782d`.** It attacked the wave-3
   fixes themselves and found that two of the eight were not actually pinned, plus four
   new defects. Method worth copying: it **re-broke each claimed fix** and checked a test
   failed; six did, two did not.

   Biggest lesson: *the definition-side fix left the expansion side doing what it
   forbade.* Macro identity stopped resolving `DUMMY` spans, but `cook_tu` still called
   `expansion_loc` on synthesized call sites — so every `##`/`_Pragma`/builtin reported
   a site at whichever file occupies offset 0. **When fixing "X must not be derived from
   a byte offset", grep for every other place that derives from a byte offset.**

   Also: the guard was added to `add_macro_at` (zero callers) while `add_macro` (every
   caller) kept the old behaviour; contract 17 was still false and its test still could
   not see it (fifth vacuity); the dedup key was unpinned in three directions; and the
   dedup fix made cooking quadratic — 305 ms at 32k sites against a budget of millions.

   **Wave 5 — `chiero-cir` review COMPLETE, applied in `22a3523`.** 95 mutations,
   **43 survived (45% escape rate)** — the most productive review yet, and the method
   (mutate, re-run, report survivors) should now be the default brief for every code
   review on this project.

   Fixed: rule 5 was **entirely unimplemented** (no width check on `Bin`/`Un`/`Cmp`/
   `Select`/`Br`/bare-`ret`); `Operand::Value` typing was one indirection deep so a
   pointer through `%1 = %0` disabled rule 6 downstream; **nothing asserted `is_error()`
   is true**, so returning `false` unconditionally passed all 42 tests; unreachable
   blocks also raised hard dominance errors, making "dead code is legal" true only for
   *empty* dead blocks; 20 malformed inputs panicked instead of erroring; contract 3 was
   half-tested.

   **Wave-5 debt DISCHARGED** in `6beec51`/`bb308cd` (round-trip) and `44bf4eb`
   (structural rules). All eight print-but-don't-parse constructs fixed, including
   `CTy::Vector` — the tokenizer split `<4 x i32>` on whitespace, so **no `.cir` fixture
   could contain a vector at all**; they now print as `<4xi32>` and there is a vector
   fixture. `print` no longer drops `variadic`, the four `FnAttrs`, or `is_const`, and
   the round-trip test compares **structurally** (`PartialEq` on `Module`) rather than
   text-to-text, which is what hid the loss. Block labels no longer alias. Structural
   identity (duplicate/dangling ids), rule 7 for declarations and rule 13's second half
   are checked. **020 §6's normative example now parses** and is a test; the parser
   gained named values/labels (canonicalized to numbers on output) and optional
   alloca scope/lifetime, and the spec was amended for `GlobalInit`, which is genuinely
   not implemented.

   **Still owed in `chiero-cir`** (updated after wave 7): `GlobalInit`/`Linkage` in the
   textual format; `Marker::Line` at instruction position (folded into `gcov_lines` on
   reparse, so `insts.len()` drops — and `ALL_MARKER_NAMES` has a hand-written exemption
   for exactly the variant that does not work); the optional passes (020 §9); and
   `cargo xtask fmt-corpus`, which `tests/corpus.rs` names but does not exist.

   **Wave 7 — third `chiero-cir` mutation review COMPLETE** (35% escape, flat vs wave 6,
   but the survivors clustered rather than scattered: earlier passes pinned each verifier
   rule at exactly *one* call site). Eight fixes applied in `acf0744` (red) / `7896cfa`
   (green), each mutation-tested individually; two further claims were probed and did not
   survive (`verify` does check every function; unknown function attributes already
   error) and became pinning tests instead.

   The critical one: **an unreachable predecessor destroyed dominance for a live block.**
   `dom[dead] = {dead}`, so meeting it into a live join emptied the set and a value
   defined in entry stopped dominating its use — a *hard error* on dead code falling into
   a live join, which is ubiquitous in real C. `chiero-lower` would have tripped on its
   first real function. The crate had gone out of its way to tolerate unreachable C by
   skipping the scan *inside* dead blocks, and never fixed the lattice underneath.

   **Spans and unordered predicates DONE** (`9c8aa62` red, `dcc3db1` green, 175 tests).
   Spans print as a trailing `; span lo:hi:ctx`; 020 §6 was amended from `; file:line`,
   which a `Module` *cannot* round-trip since it carries no `SourceMap` — rendering
   `file:line` and the macro backtrace is a diagnostic concern (010 §7), where a
   `SourceMap` is in hand, and serialization only has to be reversible. `CmpOp` gained
   `FUEq`/`FUNe`/`FULt`/`FULe`/`FOrd`/`FUno`.

   *Two mutation false-negatives worth remembering, both of which look exactly like an
   unpinned fix:* `cargo fmt` collapsed a `format!` onto one line between reading and
   patching, so the replace silently did nothing (**eighth** silent no-op this session —
   every mutation now asserts its anchor first); and dropping a field from a parser arm
   produced code that **does not compile**, which is indistinguishable from a green suite
   when counting failing tests. Mutate the printer instead when the parser mutation
   won't typecheck.

   **Judged valid but not yet applied** (in rough value order):
   - ~~**Spans do not round-trip.**~~ DONE, see above. Original finding kept for the
     reasoning: Nothing prints or parses them; every `span` becomes
     `Span::DUMMY`. Contracts 1 and 015/22 hold *only* because every fixture has dummy
     spans, and `every_corpus_module_round_trips` compares `print(m)` to
     `print(parse(print(m)))`, which is invariant under anything the printer omits.
     020 §1.5 calls provenance "the product". **This breaks the moment `chiero-lower`
     emits real CIR, and it breaks silently.** Highest-value remaining item.
   - ~~**Unordered float predicates are absent from `CmpOp`.**~~ DONE. C's `isnan` idiom
     `x != x` needs one; `FONe` is *ordered* not-equal, **false** for NaN — the opposite
     of C. Was a wrong-answer bug, not a coverage gap.
   - ~~`InstKind::Opaque` still absent~~ **DONE** (`2d6698b`, 182 tests): types, verifier
     rules, textual form `opaque [%v:ty]… writes [addr size]… reads [op]… why <reason>`
     with mandatory section keywords, and an `rdtsc`-shaped corpus fixture. The verifier
     now enforces 020 §4.3's "never silently a no-op" — an `Opaque` with no dsts, no
     writes and no reads is rejected. Contracts 31/32 are the *execution* half and wait
     on the engine. Spec amended: `UnsupportedConstruct` is `Symbol`, not `&'static str`,
     because a fixture's text is owned, not static.
   - ~~Rule 1 tested at 2 of 12 operand positions~~ **DONE** (`46dd8c9`, 184 tests):
     table-driven over eleven positions incl. contract 40's `AllocaDyn::count`, plus a
     companion test that a *dominated* use is accepted — without which a verifier stuck
     at `UseNotDominated` would pass every case. No implementation change was needed.
   - Still missing from `InstKind`/`RValue`:
     `ShuffleDyn`, `Fresh.why`, `MarkerKind::Assume`, `AccessPath`/`PtrAdd.path`,
     `CopyMem.overlap` (memmove vs memcpy is inexpressible), `Call.conv`,
     `Module.target`/`source_map`, `Body::Modeled`. Conversely `Module::config` and
     `Module::metadata` exist in code, appear nowhere in 020 §3, and are destroyed by a
     round trip.
   - **Verification is quadratic-to-cubic**, measured in release: doubling blocks
     quadruples time; `check_module_identity` is O(G²)+O(F²) with *string* comparison,
     and VPP whole-program is ~50k functions. 020 §8 claims O(size). Every containment
     test is `Vec::contains`; `successors()` heap-allocates per call inside the dominator
     fixpoint. Fix with `IndexSet`/`IndexMap` and a prebuilt predecessor map.
   - Rules 5/6/7/11 are still each pinned at **one call site**, and `va_list` operands
     are never pointer-checked at all (`vastart 0i32` verifies clean) — 020 §4.4.1 makes
     the `va_list` a real addressable `MemObject`. Cast shape rules for
     `PtrToInt`/`IntToPtr`/`FpToUi`/`FpToSi`/`UiToFp`/`SiToFp` are entirely untested.

   **`chiero-mem` STARTED** (`51e4a92` red, `7b2ce6f` green, 199 tests). The concrete
   core: `MemObject`, signed-offset bounds checking, byte contents, and the bit-indexed
   tri-state `InitMask`. 021 contracts 1, 2, 4 and the bit-granular half of §3.1.

   A mutation exposed a gap in the *tests* rather than the code: making `Cond` count as
   initialized survived, because nothing read *through* a conditionally-written byte.
   Storing the third state is only half of it — if a read accepts `Cond`, the model
   behaves exactly like the two-state mask §3.1 rejects.

   **Address space + provenance DONE** (`dc80b82` red, `ae6a38b` green, 209 tests):
   contracts 12, 12b, 12c, 13, 14, 15. Per-region deterministic bump allocation with
   guard gaps, and `IntToPtr` consulting recorded provenance **before** address-range
   search. Range search alone is wrong in both directions (§7.1) and no choice of gap
   fixes either case, since gaps only bound OOB distances smaller than the gap.

   *Two mutation escapes, both the same mistake in my tests:* **a test whose two candidate
   implementations give the same answer cannot tell them apart.** Contract 12c's
   arithmetic landed inside the object, where tag propagation and range search agree, so
   dropping the tag survived; and every one-past-the-end test used a *tagged* pointer, so
   the `<=` boundary in the fallback was never reached. Both now have a case where the two
   designs differ.

   **WAVE 8 — `chiero-mem` mutation review COMPLETE and applied.** Escape rate **39%**
   on `chiero-mem` vs **17%** on the `chiero-cir` verifier changes; the gap was real and
   the cause was mine — I wrote the memory model's tests around the happy path and around
   offset zero. Nine findings probed, confirmed and fixed across `239c823`/`09fa7b4`
   (mem) and `ecd0e0b`/`13312b1` (cir), 227 tests.

   Confirmed defects fixed:
   - **The bounds check did not bound.** `size as i64` is a wrapping cast, so a 16-exabyte
     overflow reported as an *uninitialized read* — a real buffer overflow silently
     reclassified. Trigger is `clib_memcpy(d, s, a - b)` with `a < b`. Now `i128`.
   - **A conditional write downgraded a definitely-initialized byte.** `InitMask` now
     **joins** over `No < Cond < Yes` instead of assigning. `memset` then a guarded write
     was producing the exact false-positive storm the tri-state exists to prevent.
   - **Over-wide bitfields and integers corrupted memory silently** (Rust masks shift
     amounts without overflow checks: `v >> 128` is `v >> 0`). Now `BadRange`, which is
     deliberately distinct from `OutOfBounds` — a chiero limit, not a program error.
   - **`check_bits` overflowed before comparing** → panic instead of a finding.
   - **`readonly` was inert**: a `pub` field that looked like a safety property.
   - **A `Cast`'s declared `from` was never checked against its operand** — the same hole
     closed for `va_list`, left open on the `PtrToInt`/`IntToPtr` pair that 021 §7.1 makes
     carry provenance. Fixing it immediately caught a real type error in
     `FULL_COVERAGE_FIXTURE` (`zext i32` of a comparison result, which is `i1`).
   - **Width casts ignored the int/float domain**: `trunc f64 -> i32` is `fptosi` wearing
     the wrong name and computes a different value.
   - **`vaarg %x : void`** minted a universal type-checking wildcard, disabling rules 5
     and 6 downstream. A gap in the arm added one commit earlier.

   **WAVE 8 ARCHITECTURAL ITEMS APPLIED** (`58bb7d1` red, `e87699a` green, 247 tests).
   `AccessResult { value, faults }` replaces `Result` throughout, plus the lifetime layer
   (`ObjState`, free/double-free/use-after-scope/leaks/realloc), §5's five-step ordering,
   signed byte offsets in the bit API, and non-materialization of oversized objects.
   021 contracts 2, 7, 8, 9, 10, 11, 21.

   *Ordering matters twice, and I had one wrong at first:* the state check must precede
   the contents check (or a use-after-free also reports "uninitialized"), and **bounds
   must precede alignment** (or a must-OOB access also reports the alignment of an access
   that never happens).

   *The same-answer trap caught me a third time:* the signed-bit-offset test used
   `user - 8`, which **evaluates to 0**, so it could not distinguish a signed offset from
   an ignored one. Standing rule now: **when testing that X is used, pick a case where
   using X and ignoring X give different answers.** Contract 12c, the one-past-the-end
   fallback, and this one all failed it.

   **STILL OWED from wave 8:**
   - ~~**The error type is the one 021 §5 says cannot work.**~~ DONE, see above. Kept for
     the reasoning: §5 states in bold that
     `Result<Term, MemFault>` cannot express the normal case, because an uninitialized
     read yields a value **and** a finding, and specifies `AccessResult { value, faults }`.
     The crate is `Result<Vec<u8>, AccessError>` throughout: it returns no value on a
     fault and can report only one. **Contracts 7, 26 and 2 are unreachable through this
     API**, and fixing it later is a signature change on every method. Do this before
     building more on top.
   - ~~**The bit API takes `lo_bit: u64` while the byte API takes `off: i64`**~~ DONE. — so a
     `LoadBits` of a bitfield *below* the user pointer is not representable, which is the
     crate's own founding premise. `check_bits` even reports `off: (lo_bit / 8) as i64`,
     acknowledging the signed domain it cannot accept.
   - **Alignment is stored and never checked** (021 §5 step 3: misalignment "is always
     recorded"); `MemObject::new` accepts `align = 0` and non-powers-of-two.
   - ~~**A large object size aborts the process**~~ DONE (`MAX_MATERIALIZED_BYTES`).: `new` eagerly allocates `size` bytes plus
     `8 * size` mask bytes, so an unconstrained `clib_mem_alloc(n)` kills the run instead
     of producing a finding. `catch_unwind` cannot contain it.
   - **The init mask costs 8× the object** (`Vec<InitBit>`, 1-byte tag per *bit*), paid
     even for objects never bit-accessed. With 021 §8's ~10⁴ objects and VPP's ~2.5 KB
     buffers this is the dominant cost.

   **WAVE 9 — access-layer review COMPLETE and applied** (`34f2706`, 274 tests). 33%
   escape rate, **fourteen** confirmed defects and **four** of my tests shown to prove
   nothing. Highlights: `abs_bit` overflowed (the byte API went to i128, the bit API did
   not follow); **two independent `readonly` fields**, with `write_bits` consulting the
   one nothing set; `realloc` inherited no graph position, so every `vec_resize` of a
   rooted vector was reported leaked; `leaks()` walked *through freed objects*, hiding
   free-the-head-forget-the-children; `exit_scope` overwrote `Freed`, erasing every free
   record at frame teardown; the alignment requirement was `min(object_align, size)`,
   wrong in both directions; `read_bits` returned a fault *instead of* a value and skipped
   the state check; `free(NULL)` was a false positive while `free(global)` was accepted;
   and **contract 26 — the stated reason `read` takes `&mut self` — was not implemented**,
   so two reads of one uninitialized byte gave two findings and could give two symbols.

   **The same-answer trap caught me a fourth time, and this one stings:** I wrote a
   comment warning about it and landed another instance in the same commit. The byte↔bit
   scale test wrote *and* read through the bit path, so both shared any wrong multiplier —
   `off * 8` → `off * 4` survived, and so did the extra assertion I added to disambiguate.
   **The fix is always cross-checking through an independent path**, not adding another
   assertion on the same one.

   **Wave-9 leftovers APPLIED** (`e7652af` red, `d4e331e` green, 284 tests): `copy`/`set`
   (contracts 22, 28), slot-keyed pointer edges, and derived leak roots. `copy`'s source
   read deliberately does **not** report an uninitialized read or memoize — a copy moves
   bytes without using them, `memcpy` of a partially-filled struct is ubiquitous and
   correct, and memoizing would defeat the status propagation. The finding belongs at the
   eventual *use*.

   *Two more same-answer escapes:* both overlap tests used two **different** objects,
   where the same-object guard short-circuits, so deleting the range check survived; and
   no test used exactly adjacent ranges, so the `<` vs `<=` off-by-one survived —
   `memcpy(p + 4, p, 4)` is correct and common and would have become a finding.
   *A third survivor was not a coverage gap:* liveness was checked in both the root
   predicate and the reachability walk, so the predicate's half was unreachable. Removing
   it was the fix; a test would have pinned redundancy.

   **Symbolic layer DONE** (`7c4f753` red, `3cc1683` green, 294 tests): contracts 5, 6, 6b.
   `sym` overlay, `read_term` assembling `Concat` over mixed concrete/symbolic bytes,
   `write_at_symbolic_offset` building `ite(off == k, val, old)` per candidate, and
   `ITE_THRESHOLD`-driven promotion. **`InitBit::Cond` is finally reachable** — it had been
   a state with no way to enter it.

   **A spec-driven correction to my own earlier work:** I had `Cond` reporting as a
   *definite* uninitialized read. **Contract 6b says the opposite**, and §3.1 explains why
   both obvious answers are wrong — forcing `Cond` to "yes" loses real uninitialized reads,
   forcing it to "no" is the false-positive storm on `v[i] = x; … use v[i]`. A third state
   needs a third **outcome**: `MemFault::MaybeUninitialized`, carrying the guard for the
   engine to discharge. That read returns a value and deliberately does **not** memoize —
   memoizing would silently discharge the guard in chiero's favour.

   *One survivor worth remembering:* building every candidate's guard as `off == 0` passed
   every test, because they all checked the *initialization* effect of a symbolic-offset
   write and none checked the **value**. Evaluating the term under each feasible offset —
   plus an infeasible one, which must write nothing — is what tells a correct chain from
   one that writes the same byte three times.

   **`AccessCtx` + symbolic bounds DONE** (`97d6ee0` red, `5e409da` green, 302 tests):
   021 §5 step 2's three-way decision — definitely-in (silent), may-be (one finding **with
   a concrete witness**, continue on the in-bounds branch with the constraint added), and
   definitely-out (one finding, terminate).

   *Two implementation notes worth not re-deriving:* the OOB condition must be stated
   **positively** as `limit - 1 <u off`, because `solver-lite`'s fragment (022 §3.2) is
   comparisons and conjunctions — `not(off <u limit)` falls outside it and comes back
   `Unknown`, which made the first draft report nothing at all. And a **concrete** offset
   folds the condition to a literal, which tier 1 also answers `Unknown` for; deciding it
   by `eval_ground` is exact, free, and what makes the symbolic and concrete paths agree
   about the same access.

   **`Unknown` is its own outcome.** Adding the in-bounds constraint on an answer the
   solver never gave would prune the path escalation exists to explore; reporting would
   invent a finding. Nothing exercised that branch until mutation showed it.

   **WAVE 10 — symbolic-layer review COMPLETE and applied** (39% escape; `2b7d4eb`/
   `d812223` solver, `f547b50`/`0a339d4` mem, `e9d2df5` witness; 323 tests).

   *Solver — five emissions z3 rejects outright, all from one guess.* `to_smtlib` inferred
   a term's sort from its **width**, which cannot work: predicates are stored width-1, so a
   one-bit bitvector and a `Bool` are indistinguishable. Now constants are always
   bit-vectors and **coercion happens where the context knows what it needs** (`smt_bool`
   wraps as `(= t #b1)`, `smt_bv` as `(ite t #b1 #b0)`). Two of the five were holes in my
   own earlier fix: a **mixed** conjunction fell through to `bvand` over a `Bool` because
   the guard demanded *both* operands be boolean, and `smt_is_bool` never recursed through
   nested connectives. `try_concat` bounds the payload width.

   *Memory — the unifying defect was two views of one object.* Concrete `data` and the
   `sym` overlay were never reconciled: a concrete write left a stale symbol behind (wrong
   **value**, not a missing finding) and a concrete read of a symbolic byte returned zero
   with no fault, which §3 names as the commonest way a symbolic executor is confidently
   wrong. Also fixed: a write past the threshold **promoted and discarded the write**, then
   reported those bytes as definitely uninitialized — manufacturing the false-positive
   class §3.1 exists to prevent, from inside the code meant to prevent it; `memoize_fresh`
   upgraded `Cond` to `Yes` as a side effect of reading a *neighbour*; OOB candidates were
   dropped silently when a feasible set spilling past the end **is** the overflow; the
   candidate constant was masked to `width(off)`, turning candidate 300 into `off == 44`;
   `readonly` held on one write path of three.

   *And my newest commit's own bug:* `read_sym`/`write_sym` computed a witness and then
   proceeded at hardcoded **offset 0**, so a read with `i == 4` pinned returned byte 0 —
   bounds-checked and thrown away, the wrong answer wearing the checking as credentials.

   **Array theory ADDED to the solver** (`955d3ec` red, `ccfe632` green, 330 tests):
   `Sort::Array`, `select`/`store`/`array_const`, SMT-LIB emission, evaluation by walking
   the store chain, and folding on **syntactic identity** (hash-consing makes `si == i`
   decidable at construction, and `v[i] = x; use v[i]` is the commonest shape there is).

   *The load-bearing bug was the envelope, not the terms:* every query announced
   **`QF_BV`**, which excludes arrays, so z3 rejected the whole script — surfacing as
   "backend gave no usable answer", which reads like a solver problem. Now `QF_ABV`.

   *Two of my test expectations were wrong and the code was right.* I asserted a
   satisfiable array query must return `Unknown` because `Model` cannot hold an array
   assignment; it returns **`Sat`**, correctly, because the model pins the *index* and the
   evaluator resolves the select by walking the store chain without needing the array's
   value. And the `Sat` direction cannot pin the emission at all — an unvalidatable model
   and a **malformed script** both give `Unknown`, so `(_ BitVec 0)` for the array sort
   passed it. The `Unsat` case distinguishes them. **Sixth same-answer instance**, this
   time between a real answer and a failure to answer.

   **THE ARRAY PATH IS REAL** (`9abbbf1` red, `e06be4f` green, 335 tests). `InitBit::Cond`
   carries its guard as 021 §3.1 writes it; promotion builds real SMT arrays with the
   mapping `No → 0`, `Yes → 1`, `Cond(t) → ite(t, 1, 0)`; `read_term`,
   `write_at_symbolic_offset` and `init_bit_via` consult them. The init array is
   **bit-indexed** so `LoadBits` keeps its resolution across promotion.

   **The byte API refuses a promoted object** — it has no arena, and answering from the
   frozen `Bytes` view is the drift the representation exists to prevent.

   *Evidence this mattered:* the mutation "promotion marks every never-written byte
   initialized" **used to pass**, because the old contract-6 test compared the `Bytes`
   path to itself *and* its `before` pass memoized every `No` into a `Yes` before
   measuring. It now fails, as do dropping the overlay, dropping the guards, and
   rebuilding on a second promotion.

   *Two things the tests had to learn:* guards compare **semantically**, not by term
   identity (the array path says `select(init, bit) == 1`, a different term meaning the
   same thing — identity would fail a correct implementation); and one-way promotion means
   a second promotion is a **no-op, not a rebuild** — rebuilding from the frozen view
   discards everything written since, and checking only that the flag stayed set does not
   distinguish the two.

   **STILL OWED:** the promotion test
   is also self-defeating (its `before` pass calls `read`, which memoizes, so it mutates
   what it measures); `InitBit::Cond` drops the `Term` the spec's `Cond(Term)` carries, so
   `MaybeUninitialized` has no guard to discharge and §3.1's "collapses when the guard
   folds to a constant" is unimplementable as built; `read_term` does not memoize (so one
   uninitialized byte is reported on every read, and the byte API and term API disagree
   about the same byte); contract 6b is enforced only for the byte API, not the bit API;
   `to_smtlib` has **no DAG sharing** — a 22-node shared DAG serializes to ~54 MB, which
   `--dump-queries` and every backend query pay; symbolic *base* pointers and the fork sink
   (§5.1, contracts 16, 17); lazy initialization (§6, contracts 18, 19); arenas (13c, 13d);
   §7.2's symbolic base addresses and `PointerBitInspection` (17b).

   **Still owed for 021:** `Contents::Array` promotion and the `ite_threshold`; symbolic
   offsets; lifetime plus the free/scope/leak findings (contracts 8–11); arenas (13c,
   13d); lazy initialization (18, 19); symbolic-base forking (16, 17); §7.2's symbolic
   base addresses and the `PointerBitInspection` event (17b) — note §7.2 requires object
   bases to be **symbolic**, constrained only by alignment and disjointness, with the
   concrete address kept as the witness value; the current `AddressSpace` provides the
   witness half only.

   **`chiero-cir` verifier gaps CLOSED** (`7036e35` red, `09ae8fd` green): `va_list`
   operands are pointer-checked — `vastart 0i32` used to verify clean while 020 §4.4.1
   makes the list a real addressable `MemObject` and VPP has 2552 `va_list *` uses — and
   every cast kind now has both a rejection and an acceptance test, including the
   equal-width boundary (`trunc i32 -> i32` is a `bitcast` wearing the wrong name).

   **DAG sharing DONE** (`ca8217b` red, `7f7e5b7` green, 340 tests): a 22-node shared
   chain went from **54,525,943 bytes to 620**, and a 2.5 KB buffer's init array (20k
   nested stores) now serializes linearly instead of aborting.

   *Two problems wore one symptom:* shared subterms were expanded (size) **and** the
   renderer recursed (abort). `let`-binding fixes one, an iterative post-order the other.
   **Sharing alone is not enough** — a long *unshared* chain, which is exactly what one
   `store` per bit produces, has refcount 1 everywhere, so above a small size every
   non-trivial node is bound. Bindings **nest**, because SMT-LIB `let` binds in parallel.

   **FOURTH mutation false-negative mode found:** a mutation that makes the process
   **abort** (stack overflow, SIGABRT) reports zero failing tests, exactly like one that
   fails to compile — grepping for `test result: FAILED` never sees it. Four of five
   apparent survivors were actually killed. **Check the exit code**, not the output.

   **`chiero-exec` STARTED** (`82edd1e` red, `de469a9` green, 351 tests) — the last M1
   crate but one. 023 contracts 1, 2, 3, 4, 11, 12 and §7.1's `ExactWitness`: `Value`
   (pointers keep their `ObjectId`), zero solver calls on constant conditions and
   straight-line arithmetic, deterministic true-branch-first forking, and the fidelity
   rule. An undecidable branch is taken on **both** sides with `Unknown` (not
   `Approximated`) plus an assumption naming the cause. `seal` is the single function that
   decides whether a negative result is a proof, and the witness is bound to its run.

   *Also owed here:* `read_term` memoization and bit-API contract 6b are **done**
   (`d9c9cc3`/`bde8fe9`). `chiero-exec` still lacks: tier-2 escalation (a symbolic branch
   is `Unknown` under tier 1 alone, so most forks degrade), loops and `max_loop_iters`
   (contract 5), calls and recursion (9), indirect calls (10), searchers and budgets (§4,
   §8), checkers (§6), and `chiero-model` entirely.

   **FIFTH mutation false-negative mode:** a **no-op mutation** — I wrote
   `if false { return None; }`, which changes nothing, and read its survival as a coverage
   gap. Verify the mutation actually alters behaviour, not just the text.

   **WAVE 11 — array-path review: solver half APPLIED** (`27322cc` red, `20d80ff` green,
   362 tests). **`chiero-mem` half is NOT yet applied — see below.**

   *Critical:* `array_const` — the base of **every** promoted object — was rejected by z3
   under `QF_ABV` ("unknown constant const"), so array theory was non-functional against
   the backend. Now `(set-logic ALL)`; each narrower logic excluded something in turn.
   **The omission that let it ship:** both array-backend tests used `array_var`, so
   `array_const` had never been sent to a solver.

   *Also:* `vars_of` and `eval` were still recursive — making only the serializer
   iterative **moved** the failure, since `vars_of` runs immediately before serialization
   and `eval` is on the model-validation path (a `Sat` over a deep term killed the process
   instead of validating). Both iterative now. `Sort::Bool` variables are usable. An
   `(error …)` from the backend is skipped rather than framed as the verdict.

   **WAVE 11 `chiero-mem` half APPLIED** (`b4ff632` red, `54463c1` green, 368 tests).
   Both contract-6 violations fixed: the join of two guarded writes is now the
   **disjunction** of their guards, and `Cond` **collapses** when its guard folds. The
   decision is made **per bit, not per byte** — the first attempt decided once from bit 0,
   which let a bitfield-initialized half decide the untouched half; §3.1 argues the whole
   tri-state *from* bitfields, so a mixed byte is the case, not a corner. Byte writes to a
   promoted object are refused rather than lost; `read_term` memoizes into whichever
   representation is live; promotion obeys the state check and reports when there is
   nothing to promote from.

   **Wave 11 fully applied** (`1448cbd` red, `ebdc2e2` green, 373 tests). An uninitialized
   read now yields a **fresh symbol**, never zero — §3's "single most common way a symbolic
   executor produces confidently wrong results". Minting and memoizing are one act
   (contract 26 wants the same term on a repeat, §3 wants it symbolic). Only `No` bytes
   are materialized: a `Cond` byte carries a live guard and overwriting it would discharge
   that guard in chiero's favour. The `s{i}`-shadowing and `Store`-children gaps are
   closed with solver-free structural tests.

   *Consequences worth remembering:* the byte API can no longer answer for a byte it just
   caused to be symbolized (it reports `SymbolicByte`, a different statement from "nobody
   wrote this"); and the contract-6 test compares **values only where the byte was
   written**, because two never-written bytes get different fresh symbols by construction
   and the two objects standing in for the two paths are different objects.

   *A process failure to avoid repeating:* I folded two new solver tests in as "passing on
   arrival" without checking — one was failing and I had read only the first lines of the
   output. **Read the whole result before calling a test a pinning test.**

   **Symbolic reads + unpinned stores DONE** (`f2a9b54` red, `7d46ba9` green, 377 tests).
   `ITE_THRESHOLD` now governs both directions per 021 §3; `store_at` is one `store` at a
   symbolic index with no enumeration; offsets are coerced to the array's canonical index
   width at the boundary.

   **A mutation on this work exposed a defect in my own earlier fix.** Making
   `read_answer` *skip* an `(error …)` and read the next form was wrong: z3 prints the
   error and then answers anyway, so skipping accepts that answer — turning a malformed
   script into a **confident verdict**, worse than the desync it replaced because nothing
   looks wrong. A mis-sorted array index sailed through as `sat`. An error now drains the
   following form and reports the query as failed.

   *Also:* the width test was vacuous in its first draft — reading at the *same* term as
   the store folds the select by identity, so no array reached the backend. Use a second
   symbolic index. And the ite chain's build order is genuinely **equivalent** (the
   unguarded fallback is unreachable when candidates are the feasible set), so it is
   documented rather than tested.

   **STILL OWED (021 §3):** no `SizeVal::Sym`; init-array index width should track
   `TargetConfig` per §3.1 rather than being a fixed 64.

   **Original wave-11 findings, for reference:**
   - **Every byte-level write to a promoted object is silently discarded.** `read` refuses
     a promoted object; no write path does, so `write`/`set`/`write_sym_byte`/`write_bits`
     return no faults, mutate the frozen `Bytes` view, and are invisible — a wrong *value*
     plus a spurious uninitialized-read finding.
   - **`join` drops the earlier `Cond` guard.** The correct join of two guarded writes is
     `Cond(t_old ∨ t_new)`; taking only the newer loses initialization, and the Bytes and
     Array paths then disagree — **a direct contract 6 violation**. The contract-6 test
     issues one symbolic write per object, so it cannot see it.
   - **§3.1's "`Cond` collapses when its guard folds to a constant" is unimplemented on
     the Bytes path**, so a definitely-written byte reports `MaybeUninitialized` — and
     `init_bit_via` *does* collapse, so that is a **second** contract 6 violation on a
     different input class.
   - `read_term`'s memoization is a **no-op on a promoted object** (it writes the mask;
     `init_bit_via` reads the array), breaking contract 26 there.
   - `promote_to_array` silently no-ops on an unmaterialized object and promotes a freed
     one.
   - **Coverage:** the `s{i}` → `s{i%2}` shadowing mutant **survives** — my sharing tests
     assert via the arena's own evaluator, which never reads the emitted text, and the
     only tests that parse it `z3_or_skip`. Also untested: `children()` dropping `Store`'s
     index/value, and "Yes must stay Yes" on the array init path.
   - **Not implemented, from 021 §3:** no fresh symbol is ever minted for an uninitialized
     read (`read_term` returns concrete 0 — the exact "silently reading zero" §3 names);
     no `SizeVal::Sym`; no symbolic-offset *read* API, so `ITE_THRESHOLD` is write-only;
     no unpinned symbolic store (`idx_bits` is hardcoded 64, unrelated to `width(off)`).

   **WAVE 12 — engine review APPLIED** (`47ef3f9` red, `df63974` green, 395 tests).
   **12 of 15 mutants had survived.** The central finding was the worst class there is:
   the engine was **confidently wrong at `Fidelity::Exact`**. `bin` implemented 4 of 19
   `BinOp`s and defaulted the rest to *addition* (`5 - 3 == 8`); `cmp` implemented 2 of 15
   and defaulted to `Eq`, which **inverts `Ne`**. Both now total, with **no default** — an
   unmodeled op records a `LoweringGap`.

   **One rule closed a family of holes:** no path ends at `Exact` unless everything on it
   was modeled. An unsupported terminator, a discarded `LoweringGap` reason, a dropped
   `Store`, and an unrepresentable `Const::Float` each minted a **proof for an unexecuted
   program** — what §7 rule 4 says the crate must be structurally incapable of.

   Also fixed: a **refuted** branch was explored alongside an undecided one; `Goto` to a
   missing block **spun forever** (nothing allocated, so not even the OOM killer);
   allocas were `count * 8`, making `char buf[4]` a 32-byte object — 8× too permissive
   for exactly the buffers overflows happen in; `DYNAMIC_EXTENT` overflowed that multiply;
   `max_depth` counted edges not instructions; and a fresh `TieredSolver` per query
   spawned a process per escalation and discarded the caches.

   *Three testing lessons:* **every solver query the suite made had an empty path
   condition** — nothing had two sequential symbolic branches, so the mechanism that makes
   this symbolic execution rather than enumeration was untested. `backend_spawns` cannot
   distinguish one solver from many (a fresh solver reports 1 for its own first query),
   so **count solver constructions** — the same-answer trap in the *metric*. And the
   `(refuted, undecided)` arm is unreachable under tier 1 (which yields `(No, Yes)`) but
   kept: it guards against unsoundness with a backend that gives up under an rlimit.

   **PROOF SURFACE SEALED** (`b7866a5` red, `6a34485` green, 399 tests + a third gate).
   `State::fidelity` and `RunResult`'s fields are private with read-only accessors;
   `RunResult` has a private field so it cannot be literal-built. **`seal` is now the only
   function reading fidelity** — gating in `witness()` too had made `seal`'s own check
   unreachable, which is why contract 13b could not be written. `PathTrace` and completion
   order make exploration order observable. `AssumptionKind::matches` is pinned per level.

   **`cargo xtask check-proof-surface`** is contract 13a: four forgery attempts compiled
   with `rustc`, each required to be rejected. *Getting it honest took three tries and
   every failure was the gate passing for the wrong reason:* `CARGO_MANIFEST_DIR` is baked
   in at compile time, so a copy of the tree probed the **original**; a bare `--extern`
   left several rlib candidates so every probe failed before reaching the code; and one
   probe tested two seals at once. It now resolves explicit rlib paths, **checks the
   failure was a visibility error**, and was verified by opening the hole.

   **Standing lesson:** a gate that can only pass is worse than no gate. Verify a new gate
   by *breaking* the thing it guards, before trusting a green result.

   **STILL OWED in `chiero-exec`:** ~~`ExactWitness` is forgeable downstream~~ DONE. Remaining:
   indirect calls, searchers and checkers are unimplemented.

   **`chiero-model` STARTED** (`4b5ed89` red, `81ef917` green, 407 tests) — **every M1
   crate now has code in it.** Registry, `Precision`, `HavocSpec`, `AllocPolicy`, and the
   builtin model list. 024 contracts 1, 2, 18, 19, 21, 21c and §2.1.

   *§2.1 is the load-bearing rule:* `Approximate` **degrades by being dispatched**, not by
   anyone remembering to record it — otherwise a run calling `scanf` finishes `Exact`,
   mints a witness and reports "no bugs exist" as a proof. The unmodeled path was already
   loud; the *modeled* one is worse because it looks deliberate. The reason string needs
   ≥8 non-whitespace chars because the obvious non-empty check is satisfied by `" "`.

   *Contract 19's guard matches whole tokens* — a substring grep would let the identifier
   match a comment explaining why the rule exists, which is how such a guard becomes
   decoration. The crate defines its own `Symbol` rather than borrowing the IR's, since a
   model registry is upstream of the IR.

   **Memory models EXECUTE** (`f4c98b2` red, `e15a997` green, 415 tests): `ModelOutcome`,
   `ModelCtx`, and `malloc`/`calloc`/`free`/`memcpy`/`memmove`/`memset`. Contracts 1–5
   and 10.

   *`ModelCtx` is deliberately narrow* — memory, arena, span, findings, but **not** the
   engine's state. That is what keeps 024's models independent of 023's searcher and
   threading choices, and why they test without an interpreter.

   *`memcpy` reports overlap and still performs the copy* — reporting is not refusing. My
   first test ran `memmove` on the object `memcpy` had already rewritten and compared
   against the wrong bytes; the memmove case needs a fresh object.

   *One mutation's answer was to delete code:* the model duplicated `Memory::free`'s NULL
   check. Two copies of one rule is how `readonly` came to hold on one write path of three.

   **STILL OWED in `chiero-model`:** the **string models** and `max_string_scan`
   (contracts 6–9), `realloc`, builtins (13, 14), harness intrinsics (15, 16), `longjmp`
   (20), `printf` format checking (22), and `ModelImpl::Cir`. **The engine still does not
   consult the registry** — every extern is treated as unmodeled, so contracts 11 and 12
   are unreachable.

   **WAVE 13 — registry/call review APPLIED** (`a484dba` red, `77237de` green, 430 tests,
   5 gate probes). Nine confirmed defects:
   - **`Proven` was forgeable** — all fields public, no marker, so a struct literal was a
     second route to a proof. Contract 13a was false again, in a different type. Sealed,
     plus a fifth gate probe verified by opening the hole.
   - **Allocas were per state, not per activation** — `AllocaId` is unique only *within* a
     function, so a callee's 100-byte local **was** the caller's 4-byte object. Silent, at
     `Exact`. `frame_objs` now lives on `Frame` as 023 §1 says.
   - **A `noreturn` function with a body never ran it** — §5's rule is that the *call* does
     not return, not that a body is discarded. `ret_to: None` is the right mechanism.
   - A `Return` of an unevaluable operand ended at `Exact`; loop budgets were shared across
     functions; an empty module panicked; call arguments were discarded.
   - `chiero-model`: seven names claimed `Exact` with **no implementation**; endianness was
     hardcoded under a comment saying otherwise (**same-answer trap #9**); contract 19's
     guard checked four exact tokens where the spec is a **prefix** regex over a recursive
     walk — which then caught the module doc quoting the prefixes, so the *prose* was
     reworded rather than the guard weakened.

   *Two tests could not observe anything and had to be rebuilt:* the alloca test saw one
   frame because the callee had returned and popped (its callee no longer returns); and the
   loop-budget test asserted on the budget **note**, which names the function from a
   different source than the counter key — it reads the keys now, over a back edge
   traversed exactly once, since an infinite loop terminates the state before the second
   function is reached.

   **STILL OWED (from this review, judged valid):** `Budget` has 3 of 023 §8's 6
   deterministic budgets (`max_states`, `max_forks`, `max_memory_objects` absent, so
   contract 18 is unimplementable); `State::trace` records only `BlockId` with no `FuncId`
   and only for `take_edge`, so it cannot be replayed; gate probe 3 fails with `E0594`
   rather than a visibility error and reports "unrelated reason" (safe — CI still reds —
   but the diagnosis is wrong); `chiero_model::Value` duplicates `chiero_exec::Value`
   against 023 §1.1's "one `Value` used consistently"; `HavocSpec.objects` is `Vec<u32>`
   not `Vec<ObjectId>` and `ranges` is missing; `ModelOutcome::Fork`'s guard is
   `Option<Term>` and always `None`; `register` returns `()` not `ModelId`. Plus the large
   remainder of 024 §3–§7 (string models, builtins, harness intrinsics) and the engine's
   indirect calls, searchers and checkers.

   **String models DONE** (`8c19004` red, `4211912` green, 438 tests): 024 §4 and
   contracts 6, 8, 9. `StrScan` has **three** outcomes — `Exact`, `Unterminated`,
   `CapReached` — because §4 is explicit that "the object ended" and "I stopped looking"
   are different claims. Collapsing them is how the cap assumes away the unterminated
   bug; an earlier spec draft did exactly that, so the small-object case has its own test.
   The cap **adds no constraint**. `strcpy` needs `strlen + 1`, tested at the boundary
   both ways.

   *This surfaced a `chiero-mem` defect:* **the scalar alignment rule was applied to
   byte-wise copies.** An N-byte access wanting N-byte alignment is about scalar loads and
   stores; `memcpy`/`memset`/`strcpy` move bytes and C imposes no alignment. Every
   `strcpy` into a `char` buffer was a false positive. Now scoped via `write_bytewise`,
   with a companion test that a *scalar* access at the same address still raises it.

   *One survivor was the fixture, not the code:* the "strcpy that fits" test copied into a
   freshly allocated object whose backing reads as zero, so forgetting the terminator left
   a zero there anyway and looked right. Pre-fill destinations with non-zero bytes.

   **Indirect calls + budgets + replayable trace DONE** (`0bf2d32` red, `c60f988` green,
   443 tests): 023 contract 10 and §8's full deterministic budget set. An indirect call
   forks per candidate **plus one unresolvable state** — without it the candidate list is
   implicitly exhaustive and a pointer from anywhere unseen is never explored. VPP's node
   dispatch is exactly this shape. `max_states`/`max_forks`/`max_indirect` exist, and the
   budget in force is part of the result. The trace is `(FuncId, BlockId)`, since bare
   block ids rendered a caller-plus-callee walk as one function's impossible path.

   *Candidates are currently **every defined function*** — over-approximating is the safe
   direction and `max_indirect` keeps it affordable. Resolving against the pointer's
   actual value is **owed**.

   *Three survivors, only one a coverage gap:* one was a **no-op mutation I wrote** (set
   the status before a `degrade` that still ran); the cap test asserted the cap was
   *recorded* but not *applied*; and the budget test compared against `Budget::default()`,
   which an implementation ignoring the run also returns. Both traps, both mine.

   **WAVE 14 — string-model review APPLIED** (`296da3e`/`f2e84e2` builtins, `2333636` red,
   `33db3ed` green, 460 tests). Also: 024 §6 builtins and §7 harness intrinsics.

   **THE CRITICAL ONE, and I caused it.** The engine read a name's `Precision`, saw
   `Exact`, and recorded nothing — **while never dispatching the model**. So a *registered*
   name was more trusted than an unregistered one: `strcpy` into a 4-byte buffer finished
   `Exact` and **sealed a proof** for 024 contract 9's textbook overflow. Registering
   `strlen`/`strcpy` in the previous wave **removed the degradation those calls had been
   causing** — writing a correct implementation made the engine less safe.
   **`Exact` describes the model's faithfulness *if it runs*.** `Engine::can_dispatch` is
   now the explicit short list of what the engine can actually perform.

   Also fixed: `strlen` dropped every memory fault, so a `malloc`'d buffer answered length
   0 silently (an uninitialized byte reads as `Some([0])` **plus** a fault — the model saw
   the zero and called it a terminator, *and* consumed the report-once memoization while
   discarding it); an `Unterminated` finding was emitted after **zero reads**;
   `Memory::set` materialized its fill buffer before any guard, so `calloc(1, 1<<45)`
   aborted the process; a copy's **source** side had neither the promoted-object refusal
   nor the symbolic-byte report, so `memcpy` laundered what `read` refuses; and `strlen`
   measured room from `max(0, off)`, licensing a walk before the object.

   *Two of my tests could not see what they claimed:* the cap test used a **uniform `'x'`
   fixture**, so scanning 256 or 1000 bytes gave the same answer — it pinned the *label*,
   not the bound; and every `strlen` fixture used offset 0, so reading from the object base
   rather than the pointer was invisible.

   **REAL DISPATCH DONE** (`8780466` red, `00050c3` green, 465 tests). `malloc`, `calloc`,
   `free`, `memcpy`, `memmove`, `memset`, `strlen`, `strcpy` run for real and their
   findings reach the run — **024 contract 9's `strcpy` overflow is *found*** where two
   commits ago it sealed a proof. `malloc` hands back a `Value::Ptr` (023 §1.1). Arguments
   are resolved **before** `ModelCtx` is built, since it borrows memory and the arena.
   An untranslatable argument list, a `Fork`, or a `Havoc` is a **gap**, not a silent skip.
   `DISPATCHABLE` is now one list feeding both `can_dispatch` and `is_implemented`, checked
   both ways against the registry.

   **STILL OWED from this review:** ~~`Fork` handling~~ and ~~the intrinsics' condition
   argument~~ DONE, below. `Havoc` is still a gap in dispatch; `longjmp` continues silently where contract 20
   wants `Unknown` + terminate (`ModelOutcome` has no `Terminate`); `HavocSpec` is inert
   and `fidelity_effect` ignores `self`, making contract 21c's test vacuous;
   `ModelCtx::lift` emits `{:?}` dumps rather than spanned findings; `malloc` returns
   `Value::Scalar` not `Value::Ptr` in the engine's eyes; `Fork` guards are all `None`.

   **FORKS + INTRINSIC CONDITIONS DONE** (`f1b91ce` red, `11ea5b6` green, 469 tests).
   `ModelOutcome::Fork` becomes sibling states — the first alternative continues *this*
   state so exploration order stays deterministic (023 §3). `malloc` with the default
   `AllocPolicy` now explores both the object and `NULL` at **`Exact`**: a fork is not an
   approximation. The intrinsics translate their first argument and pass `None` when it
   cannot be decided, which is the whole reason both take an `Option` — hardcoding "true"
   was safe for `assume` and **vacuous for `assert`** (every assertion in a harness
   passed, 070 §7's failure mode reached from the other side).

   ⚠️ **A sibling state never passes through `step`'s post-call increment**, so it must
   arrive already advanced. The first fork test found this loudly: the sibling still
   pointed *at* the `malloc` call, re-dispatched it, and forked again — 10001 states.
   **`indirect` had the identical bug silently**, every candidate skipping its entry block
   straight to the terminator. No test could see it because every candidate in the fixture
   had an **empty entry block** — the same-answer trap in the *fixture* rather than the
   assertion. Lesson: a fixture whose callee does nothing cannot tell "called it" from
   "skipped it".

   **THE INIT MASK IS A BITSET, AND THE CAP IS A HOLDABLE SIZE** (`02d9d08` red,
   `fd787cf` green, 473 tests) — found by the review agent probing `calloc`, and the
   failure was **SIGABRT, not a fault**: `memory allocation of 68719476736 bytes failed`.
   `InitMask` held one `InitBit` per *bit* and `InitBit` is eight bytes because it carries
   a `Term`, so the mask cost **64x the object** and an allocation exactly at the 1 GiB cap
   asked the host for 64 GiB. It is now a `Vec<u64>` bitset for `Yes` plus a sparse
   `BTreeMap` for `Cond`, with whole-word `set_range` (per-bit iteration made a large
   `memset` minutes rather than milliseconds). The lattice stays in `join`; the two fast
   paths are its identities, `join(old, Yes) == Yes` and `join(old, No) == old`.

   **`MAX_MATERIALIZED_BYTES` is 64 MiB, down from 1 GiB.** A cap has to be a size chiero
   can actually *hold*, not just one it is willing to accept — contents plus bitset plus
   `set`'s fill is a ~2.2x host multiplier. Every existing test allocated comfortably
   under the old cap, so none could see it: **the boundary is exactly where the arithmetic
   goes wrong**, and 64 MiB is cheap enough that a test can sit on it.

   *Two mutation survivors became tests:* the mask has a **canonical form** (a `Cond`
   entry shadowed by a set `Yes` bit is unreachable through `get`, so only `PartialEq`
   sees it — and `MemObject` derives `PartialEq`), and a **guardless conditional write**
   changes nothing about initialization in either direction. *Two survivors are left
   deliberately:* the cap's value is policy, and `get`'s range guard is defensive.

   **WAVE 15 — model-dispatch review APPLIED across three slices.** The reviewer ran 28
   mutants against `29e3132`, 15 survived, and it reported 13 findings. Every one was
   re-probed here before acting. Three were already red-tested; the rest are below.

   **Wave A** (`0b1b56a` red, `f4ab1a6` green, 478 tests) — five defects plus `PtrAdd`:

   ⚠️ **A `strlen` that established nothing handed back a fabricated length.** `CapReached`
   and `Unterminated` became `ModelOutcome::Value(None)`, which is *not* the untranslatable
   arm — `translated` stayed true, the `dst` fallback minted a fresh unconstrained 64-bit
   symbol, and the run stayed `Exact` and **sealed**. Worse than a missed bug:
   `n = strlen(buf); if (n == 999999) bug();` is feasible against an unconstrained `n`, so
   chiero reports a bug that cannot happen and calls the run a proof. Now a `Finding`.

   `ModelOutcome::Finding`'s payload was dropped by `dispatch`, which matched only `Value`
   — invisible because two of the three producers also call `cx.report`. `strcpy` measured
   destination room from the object base (`max(0)`, the same mistake fixed in `strlen` one
   wave earlier), so a negative-offset overflow reported *success* and stayed `Exact`. A
   copy laundered a symbolic byte into a stale constant: `read_raw` faulted on the source
   but still returned the bytes behind the overlay, so `memcpy` stopped being *silent*
   without stopping being a *constant*. `read_raw` now returns a triple and `copy`
   reinstates the overlay.

   ⚠️ **024 contract 9 had no end-to-end evidence.** The test never initialized the source,
   so `strlen` faulted on byte 0 and the destination check never ran; "not `Exact`" and
   "some finding" were both satisfied by the uninitialized read, and swapping dst and src
   gave the same answers. Fixing the fixture needed a pointer walk, which exposed that
   **`PtrAdd` was a lowering gap** — every pointer walk in any program was degrading the
   run to `Unknown`.

   **Wave B** (`2f1cca8` red, `99dc29d` green, 482 tests) — **a call chiero did not perform
   now invalidates what it was handed** (023 §5, 024 §1 step 4). `Memory::havoc` is
   breadth-first with a visited set, not a depth counter: a structure that points back at
   itself is normal for VPP's pools. Pointees are read *before* the fill, since the fill
   destroys the addresses they were found through. `Symbolic` swaps contents for an
   unconstrained array (O(1), versus one fresh variable per byte); `Uninitialized` clears
   the mask. `pointees` is `int_to_ptr`'s range search over *initialized concrete* words
   only — following an uninitialized one would invent a reference to allocator garbage.
   The engine test asserts on a **later read**, not on the assumption.

   **Wave C** (`75c1786`, 485 tests) — the vacuous tests. `is_implemented` began with
   `DISPATCHABLE.contains(&name)`, so it was true by construction and the "cannot drift"
   test could not fail. `can_dispatch`'s guard test passed through the *argument gap*
   rather than the refusal, because it called everything with `args: vec![]`. The `memset`
   fixture used `(byte 0, size 8)`, which reports the same as `(byte 8, size 0)`. And
   **one report is now one finding**: a fork copies a state's findings, so one
   `free(&stack_var)` plus one branch was two reports against contract 5's "exactly one".
   Findings carry an identity minted where the report is; deduplicating on *text* would
   collapse two genuine reports that read the same, which is the common case in a loop.

   **Wave D** (`939c5e7` red, `e6ff100` green, 488 tests) — `chiero_mark_fidelity` reads
   its reason through the new `Memory::c_string_at` (a *partly* readable string is `None`,
   never a prefix), and `__builtin_x` resolves to `x` for names chiero dispatches. Not a
   blanket strip: `__builtin_expect` has no counterpart and aliasing it would turn a clear
   "no model" into a lookup miss. This got *more* urgent from wave B, not less — an
   unmodeled call now havocs its buffer, so leaving gcc's preferred spelling unmodeled
   would make the most common calls in an optimized TU the biggest holes.

   **Wave E** (`6ce7190` red, `ba42e60` green, 490 tests) — **024 contract 20**:
   `ModelOutcome::Terminate` is a new variant rather than a `Finding`, because a finding
   reports the same words and leaves execution walking down a path the program does not
   have. `longjmp` ends the state at `TermReason::Unsupported` + `Fidelity::Unknown`, and
   is registered **`Exact`** — which reads backwards for a moment and is right: `Exact`
   describes the *model's* faithfulness, and ending the path is a faithful account. The
   dispatch fallback also uses the declared return type; hardcoding `BitVec(64)` made the
   sort *worse* by dispatching a model than by not having one. All three drift tests fired
   while `longjmp` was half-added — the first time they have caught anything.

   **Wave F** (`ceb65b9`, 491 tests) — findings render as **sentences**. `lift` emitted
   `{:?}`, so the product contained `Uninitialized { obj: ObjectId(2), off: 0, bit: 0, at:
   Span { lo: BytePos(0), … } }`. `MemFault` now has `kind()`/`at()`/`object()`, three of
   the four components of 023 §6.1's dedup key — a `{:?}` dump gave nothing to key on
   because the whole struct was the string. This is the small half of 023 §7's `Finding`;
   `witness`, `backtrace` and `PathTrace` need machinery that does not exist yet.

   **Wave G** (`ecb3435` red, `d12023f` green, 494 tests) — **`AddrOfFunc` was a lowering
   gap**, so taking a function's address degraded the run to `Unknown` *before* the call,
   and the call then forked over every defined function plus an unresolvable state — the
   safe answer to a question chiero already had the answer to. It now yields a pointer to
   a **zero-sized `Function` object** (non-zero would put it in the range search, where a
   nearby integer would resolve to it), one per `FuncId` because a fresh object each time
   makes `if (cb == handler)` false against itself. `indirect` resolves the operand before
   enumerating: three states and `Unknown` became one state and `Exact`, which for VPP's
   table-driven node dispatch is the difference between analysable and not.

   Also `cb7f3bf`: **024 contract 21c's test was written expecting RED and arrived green**
   — since the havoc commit both paths reach the same fallback, so they agree by
   construction rather than by two mechanisms coinciding. Kept, because what it *does* pin
   is that registering a model cannot remove the invalidation, which is the exact failure
   this project already hit with `strlen`/`strcpy`. `ModelOutcome::Havoc` from a model is
   still an unreachable branch; a model choosing its own `HavocSpec` (024 §2.1's `scanf`)
   is owed.

   **WAVE 16 — the havoc review, applied.** The reviewer pinned `99dc29d`, ran ~35
   mutants (18 killed, 17 survived), and reported eight defects plus fourteen coverage
   gaps. Every claim was re-probed here; all eight held, and **one correction landed on
   me**: I had recorded `c.bits()` vs `c.signed()` for a `PtrAdd` offset as a no-op
   mutation because they agree at 64 bits. Nothing makes the offset 64-bit — 020 §8 rule 6
   constrains only the *base*. At 32 bits `-4` reads as `4294967292`, and the C that
   matters is `container_of` (020 contract 28). The same-answer trap **misfiled as a
   no-op**, which is a new failure mode worth naming: a survivor dismissed on reasoning
   rather than on a probe.

   `9c440ff`: `scanf` returns its own `HavocSpec` (024 §2.1's example) naming only the
   pointers it writes — the fallback would throw away the format string too. An
   approximate model with an engine arm is now dispatched, since recording a reason and
   doing nothing was worse than having no model. **The `ModelOutcome` match has no
   catch-all**, so a new variant is a compile error — that catch-all is how `Finding`'s
   payload was dropped and how `Havoc` was swallowed, both found by review.

   `483a121`, the six defects: a call passing `can_dispatch` by *name* and then failing
   per-call translation now havocs (`strcpy`'s overflow arm reported the overflow while
   still believing the destination intact); read-only objects are skipped; `havoc` returns
   `Havocked { objects, truncated }` counting only what it invalidated; `pointees` returns
   `(found, complete)` because "nothing there" and "could not look" differ; a `PtrAdd`
   offset is signed at its own width and wider-than-pointer is a gap.

   `7055b1d` **spec**: 021 §3 now names a symbolic havoc as a *second* promotion trigger.
   The deviation shipped unamended in the havoc commit; the README requires amending in
   the same commit, so this is late. The costs are written down — a promoted object has no
   byte view, so a second havoc cannot follow its pointers.

   `5e339ba`: eight of the fourteen gap-mutants now die. The sharpest was a **same-answer
   trap in the fixture**: every `copy` test used `dst.off == 0`, so the offset arithmetic
   in both the overlay and the init reinstatement was correct and entirely unpinned.

   **WAVE 17 — the gap underneath** (`8bb168a` red, `6b4ddc7` green, 510 tests).
   ⚠️ **`Load` and `Store` were not implemented at all.** Every fixture in the suite wrote
   memory through `memset` because there was no other way, and the workaround was uniform
   enough to read as house style. It surfaced only because a mutation of the havoc's
   `reachable_depth` could not be killed without a *stored* pointer.

   Two arena folds came with it and are load-bearing: a whole-width extract is the value
   itself, and adjacent slices of one value are one slice. Without them `*p = x; y = *p;`
   rebuilds `x` as a `Concat` of `Extract`s *equivalent* to `x` but not identical — and
   the caller compares terms, not models, so every constraint relating `y` to `x` was
   lost. Also new: **a finding is not automatically a degradation.** A null dereference is
   a definite fact chiero modeled exactly; `MemFault::yields_unknown_value` draws the line
   at values chiero *invented*.

   **WAVE 18** (`4054a35` red, `8d5679e` green, 512 tests) — `CopyMem` and `SetMem` were
   missing too, found by reading what else `exec_inst` does not match on. They are CIR
   *instructions*: a frontend lowers a struct assignment and an array initializer to them
   with no `memcpy` in the source, so `s = t;` between structs was degrading to `Unknown`
   and silently keeping the destination's old bytes. `CopyMem` uses `Overlap::Forbidden`,
   because 021 contract 22's answer must not depend on which spelling the frontend chose.

   **WAVE 19 — the audit** (`b75fed1` red, `70e3ebf` green, 514 tests). Enumerating the
   CIR against `exec_inst`/`eval` found more than expected: `Un`, `Cast`, `Select` and
   `AddrOfGlobal` were *all* unhandled. A unary minus, an integer cast, a ternary and any
   function touching a file-scope variable — most of C — were degrading to `Unknown`.
   Doing this as an audit rather than one discovery at a time is what found them together;
   `Load`/`Store` had stayed hidden for waves behind a uniform `memset` workaround.

   Decisions worth not re-litigating: `Neg` is `0 - x` at the operand's width, so the wrap
   is the machine's. Casts are integer-only — `bits_of_cty` is `None` for floats and
   vectors rather than a plausible width, since reinterpreting a `double` would be silent.
   `Select` is **not** a fork: `?:` yields a value, and forking would double the state
   count for every conditional expression while exploring nothing new. A `const` global is
   `readonly`, which is also what keeps a havoc off a string literal.

   **WAVE 20** (`1a22ce9` red, `341b3d7` green, 516 tests) — `LoadBits`/`StoreBits`, so
   021 §3.1's bit-granular init mask is finally reachable from the engine. A store of bits
   4..7 leaves bits 0..3 of the same byte uninitialized and reading them is a finding,
   which is the half a byte-rounded mask cannot express. A signed bitfield sign-extends to
   its unit; a *symbolic* bitfield store is a gap, since `InitBit::Cond` does not reach
   the bit API.

   ⚠️ **The general rule it exposed:** 021 §5 hands back faults *alongside* a value, and
   for an uninitialized read that value is the **backing store's zero** — the exact answer
   021 §3.1 names as the most common way a symbolic executor is confidently wrong. `Load`
   and `LoadBits` now discard a value whose access reported a `yields_unknown_value` fault.
   `Load` was safe only by accident (`read_term` returns `None`); `read_bits` returns
   `Some(0)`, and the first version of that commit returned the zero. **Any future reader
   of an `AccessResult` has to make the same check** — the type permits the mistake.

   **WAVE 21** (`c082286` red, `8c493a0` green, 518 tests) — **021 §7.1's provenance-first
   `IntToPtr`**, the locked decision, implemented. `PtrToInt` records where the address
   came from and `IntToPtr` consults that *before* the range search. The origin is
   remembered, not recovered: the address alone cannot say which object it was once that
   object is freed, and `ObjectId` is what every later check is about. The record is keyed
   on the **term**, so arithmetic producing a different term loses the provenance rather
   than carrying it somewhere it does not belong. The fallback degrades to `Unknown` and
   names itself — a fallback answering `Exact` is indistinguishable from knowing.

   *Mutation caught the subtle half:* the **integer's value** was unpinned. Provenance
   hands the pointer back whatever the address says, so dropping the offset from the
   address computation round-tripped perfectly while a program printing or comparing the
   integer would see a lie. Same shape as the `container_of` offset trap two waves ago:
   when a value is recoverable by a second route, the first route stops being tested.

   **WAVE 22 — the `Load`/`Store` review reported.** 38 mutants, **20 survived (53%)**;
   the reviewer then killed 18 of them with new probes and showed the other 2 are no-ops
   *with a probe rather than an argument*. Nine confirmed defects. Probe files are in
   `/tmp/probes/` (`*_rev_probe.rs` pass on a clean tree; `*_rev_defect.rs` fail on one).

   **The folds are sound** — 40 000 randomized cases against an independent `u128`
   reference, plus width bookkeeping, plus a z3 round trip. Big-endian `write_term`/
   `read_term` also round-trips correctly, contrary to my prior.

   ⚠️ **The most dangerous survivor:** `extract(inner, hi + l2, lo + l2)` mutated to
   `extract(inner, hi, lo)` — a silently *wrong value* whenever the inner slice's `lo` is
   non-zero — passed all 510 tests. And the headline claim of wave 17, "a finding is not
   automatically a degradation", was **unpinned in both directions**: `yields_unknown_value`
   could return `false` for everything (an uninitialized read then seals a proof, 023 §7
   rule 4's forbidden case) or `true` for everything, and the suite passed.

   `bae2224` red / `1c154e5` green (520 tests) applied the two worst:
   **023 contract 23 was unimplementable** — `Store` took its value through `scalar`, which
   refuses a `Value::Ptr`, so `p->next = q` degraded every run containing it to `Unknown`,
   blamed the *address*, and manufactured a false uninitialized-read on the reload. That is
   the shape of essentially every VPP data structure. And **misalignment was reported
   unconditionally** where 021 §5 step 3 makes it `ub-strict`-only; the check is against the
   object's *declared* alignment, so every `CLIB_PACKED` header was a false positive — and
   my own `a_store_is_visible_to_a_later_load` manufactures two and cannot see them.

   **Wave 22 continued** (`8f60485` red, `4bd2a6f` green; `55a622d`, 524 tests):
   `MAX_ACCESS_BITS` is enforced on the term API through one shared `too_wide` — a free
   function, not a method, so the two callers cannot drift, which is exactly how the term
   path came to be the only one without the check. The `extract`-of-`extract` fold is
   pinned, and **that test took two goes**: the first version built its operand with
   `a.bv(...)`, so the constant fold fired on the inner extract and the mutation survived
   the test written specifically to kill it. Same-answer trap in the fixture, again — a
   fold test needs a *variable* and a model. `models::scanf` now takes
   `&[Option<Pointer>]` indexed by argument **position**; handing it a filtered list
   renumbered the arguments under its feet.

   **STILL OWED from wave 22, in the reviewer's priority order:**
   - ~~`Engine::run` never calls `chiero_cir::verify`~~ **DONE** (`2112d3b`, 526 tests). A
     failing module yields one `Errored` state at `Unknown`. ⚠️ **Wiring it in immediately
     red-flagged four of my own fixtures** — three passed `Const::Int { bits: 32 }` as
     `SetMem`'s fill byte where 020 rule 5 wants `Int(8)`; I had copied the shape from a
     `memset` *call*, whose argument really is a C `int`. The instruction and the call are
     not the same thing. **Owed back:** `a_dynamic_extent_does_not_overflow_the_size_computation`
     lost its `object_size_for_test` assertion, because a `DYNAMIC_EXTENT` alloca with no
     `AllocaDyn` is now correctly rejected before frame setup. Restore that half when
     `AllocaDyn` lands. The `yields_unknown_value` cluster is pinned in both directions.
   - ~~A zero-sized `Load` fabricates a 64-bit symbol~~ and ~~a faulting loop floods the
     findings list~~ **DONE** (`03d7539`, 529 tests). Findings now carry
     `FindingKey { kind, span, object }` and `RunResult::findings` dedups on it; the
     sequence id recognises the copies a **fork** makes and cannot recognise the copies a
     **loop** makes. Model reports are bare strings with nothing to key on, so they keep
     the fork identity — 023 §6.1's full key stays 040's to apply. ⚠️ Mutation added the
     load-bearing test: dropping the *object* component survived, and **merging is the
     dangerous direction** — a duplicate is noise, a dropped finding is a missed bug, and a
     hand-written fixture has `Span::DUMMY` everywhere so the key collapses without it.
   - `sort_of` still falls through to `BitVec(64)` for a *faulting* `f32` load, which is a
     narrower version of the same defect.
   - ~~Execution continues past a definite crash~~ **DONE** (`dba99e2`, 531 tests).
     `MemFault::is_fatal` ends the path; findings before it stay, fidelity is untouched
     because chiero modeled the crash exactly. ⚠️ **The boundary is the whole design**: it
     excludes `BadRange`, `AllocationTooLarge`, `SymbolicByte`, `MaybeUninitialized` and
     `OutOfBoundsMaybe`, which are chiero's limits or *possibilities* rather than facts
     about the program — ending a path on one would silently drop the analysis of code that
     runs fine, which is worse than the bug this fixes. Mutation caught the quiet half:
     dropping `OutOfBounds` from the set survived, and that is the case where continuing is
     least visible, since nothing traps and the write is simply lost.
   - `models::scanf` applies `.skip(1)` to the *filtered* pointer list, so an unresolvable
     format argument eats the first real output pointer. Not hypothetical:
     `AddrOfGlobal`-shaped `scanf("%d", &x)` has exactly this form. It must skip argument
     *positions*.
   - `Store`/`Load` ignore the CIR's `align` operand — the very thing that would separate a
     packed access from a promised-aligned one.
   - Land the reviewer's survivor probes as pinning tests, `S6` and the
     `yields_unknown_value` cluster first.

   **WAVE 23 — the verification/dedup review.** 34 mutants, **12 survived (35%)**, all 12
   classified with a probe: 9 real gaps, 3 no-ops. Nine confirmed defects. Probes in
   `/tmp/review-v2/crates/chiero-exec/tests/probe.rs`.

   Applied (`d7b1829`, 534 tests, on top of `b9aafc1`'s `sort_of` fix):
   **C1** `too_wide` ran *before* the state check, so a 32-byte load through a freed
   pointer reported "unsupported-access-width" instead of the use-after-free — the
   reviewer's mutation swapping the blocks survived the suite *and fixed the behaviour*.
   **C9** the misalignment filter compared `kind()` to `"misaligned"`, and the test
   guarding it greps the same literal, so renaming the slug kept the workspace green while
   restoring the `CLIB_PACKED` storm. ⚠️ **New failure mode: a test coupled to the
   implementation by the same string it checks.** **C6** `completion_order` was empty on
   the verify-failure path.

   **WAVE 24 — four of wave 23's defects fixed** (`e88c7a8`/`94987a3`, `e278c5c`,
   `4a7b806`; 539 tests).

   **C3 provenance laundering is closed.** `IntToPtr` consults `Frame::ptr_vals` — the
   local a `PtrToInt` wrote — so arithmetic writes a *different* local with no entry and
   loses provenance. Recorded in `exec_inst`'s `Assign` arm, **not** in `eval`, because
   `eval` does not know which local it is filling. **Per activation like `locals`**: a
   `ValueId` is unique only within a function, so a state-wide map would let a callee's
   `%2` inherit the caller's provenance — the exact bug `frame_objs` exists to avoid, one
   level up. The value-keyed `ptr_ints` stays for the narrower job it can do honestly: a
   pointer that went through **memory**, where the bytes are the carrier.

   **C4**: `ModelCtx::lift` keeps the `MemFault` beside the text it renders, so a `memcpy`
   bug in a loop is one finding like a `Store` bug already was. The earlier "one report is
   one finding" claim was narrower than it read — it held only for `report_faults`.

   **C5**: `FindingKey` gains `func`. ⚠️ **Two components were unpinned because of the
   fixtures**: dropping `span` changed nothing since nearly every fixture uses
   `Span::DUMMY` (there is now an `inst_at` helper), and dropping `kind` made an
   out-of-bounds read *disappear* behind an uninitialized read — the worst outcome a
   deduplicator can produce. All four components now fail a mutation individually.

   **WAVE 25 — the rest of wave 23's list** (`49cd4e3`, `8dd6687`; 543 tests).
   **C2**: `NULL` is address zero. `addr_of(ObjectId::NULL)` can never answer because ids
   start at 1, so `p->next = NULL` was a lowering gap and the reload invented an
   uninitialized-read about memory the program had just written. ⚠️ The lesson is narrow
   and general: **a fix verified on one constructor of a value is not verified on the
   others** — `Const::Null` takes a different path through `operand` than `Value::Ptr`,
   and the pointer-store commit had only exercised the latter.
   **C8**: an argument `scanf` cannot resolve degrades to `Unknown` and names itself,
   instead of silently meaning "no buffer" — a model that knows *less* than it claims is
   worse than no model, the same lesson `strlen`/`strcpy` taught in wave 14.
   **Mutant E**: the `MAX_ACCESS_BITS` boundary is pinned on both sides; `>` versus `>=`
   decides whether a 16-byte `__int128`/SSE access is answered or refused.
   **C7** has a test at two error sites; the remaining `Status::Errored` assignments still
   do not call `degrade` themselves.

   **STILL OWED from wave 23, highest first:**
   - ~~**C3 provenance laundering**~~ DONE (wave 24). `ptr_ints` is keyed on the term, and
     addresses are ground `bv(64, …)` constants that the arena hash-conses — so **term
     identity is value identity** and any integer expression evaluating to a recorded
     address gets that object back, bypassing the `Unknown` degrade exactly when it should
     fire. Both doc comments claim the opposite. `address_term` made the table far denser
     by adding every pointer *store* to it.

     **The fix, worked out but not landed** (next slice, start here): key provenance on
     the **`ValueId`**, not the `Term`. Dataflow is what provenance follows; a term is a
     *value*, and two ways of computing the same address are the same term by
     construction — hash-consing guarantees the bug. So:
     - `State::ptr_ints` becomes `IndexMap<ValueId, Pointer>`, per frame like `locals`
       (a `ValueId` is only meaningful inside its activation).
     - `PtrToInt` cannot record it from inside `eval`, which does not know `dst`. Record
       in `exec_inst`'s `InstKind::Assign` arm instead: after `eval`, if the `RValue` was
       `Cast { kind: PtrToInt, a }` and `a` resolved to a `Value::Ptr`, insert
       `dst -> that pointer`. A pointer-typed `Load` records the same way.
     - `IntToPtr` looks up `Operand::Value(v)` in the map. A `Bin`/`Un` result is a
       *different* `ValueId` with no entry, so arithmetic loses provenance — which is the
       behaviour both doc comments already claim.
     - The existing round-trip test still passes (the operand is the direct `PtrToInt`
       result); the reviewer's laundering probe starts failing, which is the point.
     - Keep the `Unknown` degrade on the fallback path unchanged.
     Write the laundering case as the RED test first: `(uintptr_t)&a + ((uintptr_t)&b -
     (uintptr_t)&a)` must **not** come back as `&b` at `Exact`.
   - **C7** no `Status::Errored` site calls `degrade`, so `State::fidelity()` is `Exact`
     for a state that gave up; only one untested line in `RunResult::fidelity` prevents a
     PROVEN seal. (One path is now pinned; the other six sites are not.)
   - ~~**C4**~~ DONE (wave 24). `ModelCtx::lift` stringified the `MemFault` and discards the struct, so
     `key: None` — every model-reported fault (`free`, `memcpy`, `memset`, `strcpy`,
     `calloc`) still floods a loop with N copies. `lift` has the fault in hand.
   - ~~**C5**~~ DONE (wave 24). The key merged distinct findings: `object()` is `None` for `NullDeref`/
     `WildPointer`/`BadRange`, and there is no *function* component, so two functions
     sharing `Span::DUMMY` collapse. Merging is the dangerous direction.
   - ~~**C2**~~ DONE (wave 25). `p->next = NULL` was dropped — `addr_of(ObjectId::NULL)` is `None` because
     ids start at 1 — and manufactures the same false uninitialized-read the pointer-store
     fix was meant to end. Wider class: `operand` handles only `Value`, `Const::Int` and
     `Const::Null`.
   - ~~**C8**~~ DONE (wave 25).
   - ~~Mutant **E**~~ DONE (wave 25). The `MAX_ACCESS_BITS` boundary was untested — a 16-byte `__int128`/SSE
     access is `Exact` today and `>=` would make it a fault.
   - ~~Mutant **Y**~~ DONE (wave 26), pinned.
   - `Store`/`Load` still ignore the CIR's `align`, which is what a real `ub-strict` mode
     would need.

   ⚠️ **A methodology trap the reviewer hit and §8 should carry:** restoring a `.bak` over
   a mutant gives the file an *older* mtime, so cargo considers it up to date and the next
   run silently tests the mutant. `touch` after every revert.

   **WAVE 26** (`2f68983`, `facfa16`; 546 tests). Pinned the last of wave 23's list — an
   untranslatable `Store` leaves the run unprovable, which was unpinned and let `seal`
   return PROVEN over a discarded write. Then **`AllocaDyn`**: a fresh object per
   execution (C's `alloca` in a loop accumulates, and reusing one object makes the second
   iteration alias the first), a symbolic count is a gap rather than a guess.

   ⚠️ **Two of my own assertions were same-answer traps and mutation found both.** "The
   thirteenth byte faults" passes against an *over-allocated* object, because an
   uninitialized read faults too — it needed the fault **kind**. And nothing looked the
   alloca up by id after `AllocaDyn` ran, so dropping the `frame_objs` insert changed
   nothing. General form: **an assertion that something went wrong is weaker than one that
   says what went wrong**, and a value written by one path must be *read back by another*
   or the write is unpinned.

   `a_dynamic_extent_does_not_overflow_the_size_computation`'s lost coverage is restored
   as `a_dynamic_extent_multiplies_the_element_size` — the computation now lives in
   `AllocaDyn`, so it is observable again.

   **WAVE 27 — the provenance/dedup review.** 20 mutants, **8 survived (40%)**, all
   classified with probes. Nine confirmed defects plus two out-of-scope, and it **refuted**
   my own worry: a stale `ptr_vals` from a reassigned `ValueId` is unreachable, because
   `verify` raises `ValueAssignedTwice` and `run` refuses the module — 020 §1.3's claim
   that `ValueId`s are single-assignment temporaries is accurate. Refuted *by probe*, which
   is the standard this project now holds itself to.

   Applied (`3dc9d35`, `aa1555c`, `8ae563b`; 550 tests):
   ⚠️ **Dead code made a module unexecutable, and that was my bug.** Wiring verification
   into `run` gated on `!errs.is_empty()` rather than `is_error()`; 020 rule 3 makes
   `UnreachableBlock` a warning, and wave 7 fixed the dominance lattice precisely so a live
   join after dead code would work. The gate meant to stop chiero analysing unchecked
   programs stopped it analysing correct ones — the worse failure, because silent refusal
   looks like a clean run to anything counting findings.
   **Address zero is `NULL`**, not `UNBOUND`: `*(int *)0 = 1` reported a *wild pointer*,
   which carries `Unknown`, where `NullDeref` is a definite finding.
   **Provenance crosses a return** — closing the laundering hole had taken the honest case
   with it, and index-to-pointer helpers are the dominant VPP idiom. My commit message
   recorded the laundering case degrading and said nothing about this one.
   **An entry function's parameters are bound** to a fresh `Extern` object (pointer) or a
   fresh symbol (scalar). They were absent, so `void f(int *out) { *out = 7; }` analysed on
   its own wrote nothing — a whole-program tool is used on a library exactly this way.
   `ENTRY_PARAM_BYTES` is documented as *a bound chiero chose*.

   **WAVE 28** (`75496e7`, `784a190`, and the key test; 554 tests).
   **A zeroed pointer field is a null pointer.** A `CTy::Ptr` load now falls back to the
   address when there is no recorded provenance — `calloc`/`memset`/`.bss` bytes were never
   in that table, so `n->next->x = 1` on a zeroed struct reported *nothing*. Zero resolves
   to `NULL` at no fidelity cost; any other address degrades, since 021 §7.1 calls the
   search wrong in both directions. This also removed the path-order dependence, where an
   unrelated earlier `q = NULL` decided how a zero word read back.
   **A model reports a bug once.** `strcpy`/`calloc` did `report` *and*
   `ModelOutcome::Finding`, two routes, two ids, unmergeable. `report` is now documented as
   "noticed and continued past"; giving up is the outcome's job.
   ⚠️ **Two same-answer traps of my own, both found by review or by mutation:** the null-
   store test answered from the table the store populated *and* accepted a `Scalar(0)` as
   an answer to "is this a null pointer"; and my first `func` fixture used two different
   buffers, so `object` told them apart and `func` stayed droppable — the trap one
   component over. The model key is now pinned in all four.

   **WAVE 29** (`f215789`, `ee45f58`; 556 tests).
   **`ModelCtx` keeps one list of `(Option<MemFault>, String)`**, not two parallel `Vec`s
   indexed together — those drift the moment a model reports without a fault, putting a
   fault's key on someone else's sentence. It held only because no shipped model
   interleaves, which is a trap for the next author rather than a bug in the current code;
   the fix makes the *shape* enforce it.
   **`Const::GlobalAddr`/`FuncAddr` are pointers**, offset included. They were lowering
   gaps, so `&g` failed in exactly the places it is most ordinary.

   ⚠️ **Two editing traps worth §8:** a two-replacement script asserted on a second anchor
   `cargo fmt` had reflowed, died before writing, and reported nothing — the edit silently
   never happened. And an insert anchored on `RValue::AddrOfFunc(FuncId(1))` landed in a
   *different test* containing the same line. **Anchor on something unique to the target,
   and verify the edit arrived** — `grep -c` for the new text before running anything.

   **WAVE 30** (`f988920`; 557 tests) — **varargs**. The cursor lives in the `va_list`
   object's own bytes, because 020 §4.4.1 needs a `va_list *` to cross a function boundary
   with the callee advancing the *caller's* state; engine-side state cannot express that.
   Variadic arguments are `Value`s on the frame, not bytes, since `format_function_t` takes
   `u8 *` and `va_list *` — in these paths the varargs *are* pointers and must keep their
   objects. `va_arg` past the end is a **gap, not a value**: a fresh symbol would let a
   `printf` with too few arguments look like it read something real. `va_end` is a
   deliberate no-op.

   ⚠️ **`exec_inst` has no catch-all any more.** Every `InstKind` is handled, so adding one
   is a compile error rather than a silent `LoweringGap` — which is exactly how `Load`,
   `Store`, `CopyMem`, `SetMem` and the four `Va*` all stayed missing for waves, behind a
   uniform workaround that read as house style. The same change was made to the
   `ModelOutcome` match for the same reason.

   Also: wave 27's `scanf` `.skip(1)` gap is **no longer reachable** — three tests kill
   that mutation now. Judged, not assumed: the claim was made against `673fc8d`, before the
   positional fix and its tests.

   **STILL OWED from wave 27:**
   - ~~**`strcpy` and `calloc` report the same bug twice**~~ DONE (wave 28). — `cx.report(msg.clone())` *and*
     `ModelOutcome::Finding(msg)`, two `finding_seq` ids, neither keyed.
   - ~~**`ModelCtx::faults()`'s index correspondence**~~ DONE (wave 29). `dispatch` zips
     `findings()[i]` with `faults()[i]`, but `report()` pushes only to `findings`. It holds
     because no shipped model interleaves; it is a trap for the next model author. Make
     `report` push a `None` fault, or carry pairs.
   - ~~**The *model* finding key is unpinned**~~ DONE (wave 28). — `4a7b806`'s claim
     covers `report_faults` only. Four merge probes exist in the review.
   - ~~**A zeroed pointer field reloads as a scalar**~~ DONE (wave 28)., so
     `n = calloc(...); n->next->x = 1;` reports nothing. Zero recall on a canonical bug
     class; `store NULL; load` works only because `address_term` seeds `ptr_ints`.
   - ~~**`NULL`-ness is path-order-dependent**~~ DONE (wave 28). for the same reason — an unrelated earlier
     `q = NULL` changes how a zero word reads back. Fix belongs in `Load`/`int_to_ptr`.
   - ~~`storing_a_null_pointer_lands_like_any_other` same-answer trap~~ DONE (wave 28).: its check
     answers from the table the store populated, and its fallback arm accepts a scalar as
     an answer to "is this a null pointer". Reload as `i64` and compare.
   - ~~`scanf`'s `.skip(1)` → `.skip(0)` survives~~ no longer reachable (wave 30).
   - ~~`Engine::operand` handles only `Const::Int` and `Const::Null`~~ DONE (wave 29) for
     `GlobalAddr`/`FuncAddr`; `Float`, `Wide` and `Undef` stay gaps deliberately.

   **WAVE 31** (`4990d4e`; 558 tests) — the **vector operations**. A vector is a
   bit-vector of `lanes * width` bits, little-endian by lane (021 §3 for SIMD), so every
   operation is slicing and concatenation. ⚠️ **The lane width is recorded, not derived**:
   `ExtractLane`/`InsertLane` carry no element type and a total width cannot say how it
   divides — 32 bits is four `u8` lanes or two `u16`. My first attempt inferred it from
   the lane index and read the whole vector for lane 0. It now travels with the local like
   provenance; a vector arriving any other way is a gap rather than a guess.

   **`eval` has no catch-all either now.** Both matches are exhaustive, so the next
   missing instruction or rvalue is a compile error.

   *Two more same-answer traps in my own fixture, both caught by mutation:* the shuffle's
   second operand was the same splat as the first, so lane 0 of each held the same byte;
   and the recorded lane width was unpinned until an `ExtractLane` read it back.

   **WAVE 32** (`ec5dbf3`; 559 tests) — `State::give_up` sets the status and degrades
   together. The guarantee rested on `RunResult::fidelity` special-casing `Errored`, so
   `State::fidelity()` alone answered `Exact` for a state that had stopped.

   ⚠️ **That test took three attempts to stop being vacuous**, and both failures are
   general: an `if … { continue; }` escape hatch meant the loop body never ran and the test
   asserted *nothing*; and once removed, every fixture was being rejected by `verify`
   first, so the run errored on the **verification path** — which degrades explicitly — and
   the engine's own sites were never reached. **A fixture that errors for a different
   reason than the one under test is the same-answer trap wearing a different hat.**

   Also judged stale: the review's claim that removing `RunResult::fidelity`'s special case
   survives. Four tests kill it now; it was true at the commit it was written against.

   **WAVE 33 — the varargs/vector review** (`454cb8c` red, `b669109` green; 565 tests).
   36 mutants, 12 survived; **four wrong answers at `Fidelity::Exact`** and one process
   kill. All applied.

   ⚠️ **The worst was mine to have caught.** 020 contract 37 — a callee advancing the
   caller's `va_list` through `va_list *`, which is the *stated reason* the list lives in
   memory and covers 2552 sites in VPP — did not work. The cursor crossed the boundary; the
   argument values did not, because `va_arg` asked `stack.last()`, the callee's frame, which
   is empty for a non-variadic `format_function_t`. My commit message said engine-side state
   "cannot express that at all" and then left the argument area as engine-side state. The
   fix puts the **owning frame index in the object's second word**, which 020 §4.4.1's ABI
   layout has room for. *Lesson: when a commit message names the reason a design exists,
   test that reason.*

   Also: `filter_map` **dropped** an unrepresentable vararg instead of holding a hole, so a
   `%f` shifted every later argument — `Exact`, no finding; `va_arg` ignored its declared
   type, putting a 64-bit term in an `i32` local and **panicking the solver** on comparison;
   and `Shuffle` took its lane count from `mask.len()`, which is the *result* length, so a
   widening shuffle read nibbles and never touched `b`.

   Two fixture defects of mine, both the usual shape: every value in the vector test was a
   byte, so hard-coding a lane width of 8 survived; and there was **no `va_copy` test at
   all**, leaving contract 36 unpinned in three directions.

   **Judged and not applied:** the reviewer's three no-op classifications (`Splat` concat
   order is genuinely symmetric; `if f.variadic` is unreachable because `verify` enforces
   the arity; `ExtractLane`'s bounds check is behind verifier rule 12) — each came with a
   probe, and each holds.

   **WAVE 34** (`893b6be`, `53f787a`; 567 tests). **`ModelOutcome::Finding` is keyed by
   its call site** — a model giving up has no `MemFault`, but the *call* identifies the
   bug, so one `strcpy` that cannot scan its source is one report however many times the
   loop runs. ⚠️ Deduplicating on the **text** would not have worked: two iterations give
   different messages ("Unterminated { scanned: 16 }" then "CapReached { scanned: 0 }"),
   because the first partly wrote the destination. The now-dead unkeyed `findings` path is
   removed rather than left empty.

   **020 contract 33 amended**: `ShuffleDyn` is declared in §4.1 and absent from `RValue`,
   so `eval`'s exhaustiveness cannot catch it — *a variant the enum never had is not one
   the match can be missing*. The contract now says its second half is untestable rather
   than reading as satisfied.

   *The give-up test needed two call sites*: with one, the `span` component was unpinned
   because every report shared it. Recurring rule — **a key component is untested whenever
   every fixture supplies the same value for it.**

   **STILL OWED from wave 33:** ~~`ModelOutcome::Finding` unkeyed~~ and ~~`ShuffleDyn`~~
   both handled in wave 34. Remaining: `InsertLane` derives its width from the inserted element while `ExtractLane`
   refuses to guess — an asymmetry the reviewer could not turn into a defect but which
   contradicts the commit's own rule.

   **WAVE 35 — M1's exit criterion, measured** (`bfe81b2`). `cargo xtask
   contract-coverage` walks 020–024 for numbered contracts and the sources for `NNN
   contract K` citations. **82 of 161 cited.** M1 is *not* close to its exit, and I had
   been answering that question from recollection.

   Uncited by document — 020: 33, 021: 17, 022: 12, 023: 11, 024: 6. The 020 gap is the
   largest and is mostly textual-format and verifier surface; 022's is the solver's
   independence slicing, caches and differential campaign, which 022 §6.2 records as owed.

   ⚠️ It is a **coverage** measure and labelled as one: a citation means a test *claims* to
   cover a contract, which this project has found is not the same thing roughly twenty
   times. It answers "what has nobody looked at".

   *Two gate bugs worth remembering:* matching the literal heading `## Testable contracts`
   found nothing (the heading is numbered) and the gate then reported **0/0 cited** — full
   coverage of an empty set, the most misleading answer a gate can give. And walking only
   `crates/` marked 023 contract 13a uncovered, since `check-proof-surface` enforces it.

   **WAVE 36** (`3f5040a`; 570 tests, **91/161 cited**). 020 contracts 9 (div/rem by zero,
   execution continues, **and the solver agrees with the engine about the same term** —
   that half is what stops the IR and solver conventions drifting), 19 and 20 (union punning
   both ways, no cast node, **zero findings** — a model that reported correct C here would
   be unusable on VPP), and 7's value half. Contracts 3 and 6 were already enforced and only
   needed citing.

   ⚠️ **The scanner now reads `020 contracts 19 and 20`, not just the singular.** Accepting
   only one form would push authors into writing "020 contract 19. 020 contract 20." to
   satisfy the gate — **a gate that forces unnatural prose gets worked around rather than
   followed** — and that change alone surfaced five citations already present.

   *Recorded, not claimed:* contract 7's other half ("exactly one signed-overflow event")
   and contract 8's ("one shift-UB event") need 023 §6's event surface, which does not
   exist. Contracts 12, 14–18 need the frontend and belong to M2.

   **WAVE 37** (`675c143`; 573 tests, **95/161 cited**) — 020's bitfield block, contracts
   24, 25, 26, 27. ⚠️ **Contract 26 was a real gap in code I wrote**: `LoadBits` with
   `signed: true` over `0b111` in three bits is `-1`, not `7`. I implemented the sign
   extension in wave 20 and tested only the unsigned side, so `sext`→`zext` survived until
   now. *Implementing both directions and testing one is its own failure mode* — the code
   looks covered because the feature is present.

   24/25 are the pair 020 names as **the** thing byte-granular init cannot do: both fields
   share a byte, so a per-byte mask must answer wrongly for one. 27's two halves each fail
   a mutation individually.

   **WAVE 38** (`11cc2cf`; 575 tests, **97/161 cited**) — 020 contract 11: an `Opaque`'s
   declared `writes` were **ignored entirely**, so inline asm saying it clobbers a buffer
   left chiero believing the old bytes. `Memory::havoc_range` clobbers exactly the declared
   range, filling through the symbolic overlay rather than promoting — promotion would take
   the *untouched* bytes with it. Contract 31 passed on arrival.

   ⚠️ **Both assertions in that test were the same trap and I fixed one at first.** 021 §5
   hands back a value *alongside* a fault, so `read(..).value` shows the stale bytes behind
   an overlay: "was invalidated" needs the **fault**, "was not invalidated" needs its
   **absence**. Comparing values alone passes either way. Mutation caught it by doubling the
   clobbered size. **Whenever a test asserts a range changed and its neighbour did not, both
   halves have to read the fault, not the value.**

   One survivor classified **as a no-op with a probe**: naming every clobbered byte the same
   still yields distinct terms — `TermArena::var` mints a fresh `VarId` per call and
   `smt_name` prefixes it (`v3_clobber`), so the name is cosmetic and cannot collide in a
   script either.

   **WAVE 39** (`1f3dee6`; 576 tests, **98/161 cited**) — 020 contract 21: a constant
   overwriting one byte of a symbolic word leaves the other three as *the same term*. VPP
   rewrites one byte of a packet header constantly, and a model answering "the whole word
   is unknown now" would lose every constraint on the rest.

   *Assertion style worth reusing:* the test asserts over **evaluations under a model**,
   not over term shape. `loaded ^ original` must have bytes 0, 2, 3 zero and byte 1 equal
   to `0xEE ^ original_byte_1`. Asserting a `Concat` structure would pass against any
   equivalent-but-wrong rebuild *and* break on a legitimate folding change; asserting
   values pins the meaning. The second half matters — "unchanged everywhere" alone is
   satisfied by the write having been **dropped**.

   **WAVE 40** (`922292c`; 578 tests, **100/161 cited**) — 020 contracts 35 (bitcast
   preserves total width) and 38 (`AllocaDyn` in a loop makes three *distinct* objects;
   mutating it to reuse the frame's object kills three tests).

   **Triaged and owed, not approximated:** contract 43 (`Const::Undef` propagates through
   arithmetic, and a branch on `Undef` forks both ways at `Unknown`) needs an `Undef`
   *value* the engine does not have — `operand` returns `None`, so it is a lowering gap
   rather than a propagating value. Contract 41 needs an observable-effect sequence for
   `Volatile`; 42 needs `IndirectGoto` execution; 44 needs the pass pipeline. Contracts 12
   and 14–18 are M2's.

   **WAVE 41 — the contract-tests review.** Ten confirmed defects. Two applied so far
   (`8ad0177`, `499272a`; 579 tests).

   ⚠️ **The coverage gate was lying in both directions**, which matters more than any one
   contract because everything else is measured by it. It counted `024 contract 22` —
   `printf` format checking, wholly unimplemented — as covered, because a doc comment about
   *deduplication* listed it in passing; counted `021 contract 3` because a header explains
   why it is **unimplementable**; and **cited itself**, since its own doc comments use
   `023 contract 10` as a syntax example. It also *missed* real citations (`024 §4,
   contracts 6-9`, and ranges) and its declaration parser anchored on the first mention of
   "Testable contracts" anywhere — one cross-reference sentence moved 020's denominator
   from 44 to 60 — while slicing to end of file, so an appendix would invent contracts that
   collide with real ids and be counted as cited. **Only `tests/` and `xtask/` count now.**
   The honest number is **99/161**, not 100. *A gate that overcounts is worse than no gate:
   it retires work that was never done.*

   ⚠️ **020 contract 25 was cited by me and was false.** `write_bits` touched only `data`,
   `read_bits` never called `first_symbolic` — so a `StoreBits` into a symbolic byte
   **vanished** and the neighbouring bitfield read back as a definite constant with no
   fault. `if (s.b != 0)` pruned, run seals `Exact`: a false negative wearing a proof. I had
   written that 25 was "the same fact from the other side" as 24; it is not — **24 is about
   the mask, 25 is about the value**. Ordering the new check took two attempts: after
   `check_bits` (or the byte range overflows) *and* after the init checks (or contract 6b's
   conditional bitfield reports the wrong fault).

   **Wave 41 continued** (`bb074d9`, `3b4d8fd`; 585 tests). D2/D3/D4 fixed:
   `havoc_range_reporting` returns how many bytes it managed and carries its faults, so a
   declared clobber wider than the object is a **finding** rather than a fidelity degrade —
   *degrading reads as "chiero was unsure", not "your program is wrong"* — and the
   read-only/freed/promoted refusals apply to both fills instead of only to `Symbolic` by
   accident. D8/D9/D10 fixed: contract 27 runs through **both** call sites and pins the
   accept side (a full-width bitfield is legal and was unpinned); an overflowing `BitRange`
   is rejected rather than *accepted in release*; and contract 9's drift check now uses a
   symbolic dividend, asserting `eval_ground` **fails** first so it fails loudly if folding
   ever makes it vacuous again.

   ⚠️ **Fixing the overflow moved the panic into the error *message***, which formatted
   `bits.off + bits.width` as well. When guarding an arithmetic overflow, check every use
   of the expression, not just the one that panicked.

   **WAVE 42** (`7727db7`, `6fe4547`; 587 tests). 020 contract 32's **execution** half —
   a parser test cited it and honestly said it covered "the representational half"; nothing
   ran the other. And the review's last item: every `OpaqueWrite` fixture had exactly one
   entry, so `writes.take(1)` survived. ⚠️ **A property over a collection is untested
   whenever every fixture supplies a collection of one** — same shape as the finding-key
   components and the two `check_bits` call sites.

   ⚠️ **021 contract 20 was unimplemented and is an architectural one.** `Entry` held its
   `MemObject` by value, so cloning a `Memory` copied every byte of every object — and
   forking is the engine's *most frequent* operation, so the cost was quadratic in the
   program's memory rather than in its branching. Objects now sit behind an `Arc` with
   `Arc::make_mut` at every mutation site. The test checks **pointer equality**, the only
   way to tell sharing from an identical copy.

   **WAVE 43** (`c358008`; 589 tests, **103/161 cited**) — 021 contracts 26 (two reads of
   one never-written byte give the *same term* and **one** finding — the memoization
   `&mut self` on `read` exists for; a fresh symbol per read makes `x == x` unprovable for
   memory nobody wrote) and 32 (VPP's registration-table shape end to end: a handler stored
   into a global, loaded, called indirectly, resolving to one `FuncId` with no fork —
   exercising `Const::FuncAddr`, pointer store/load, and provenance resolution together).
   Contracts 23 and 31 were already covered and only needed citing; 021 c31 is 020 c37 from
   the memory side, so one test satisfies both.

   021's remainder is now mostly **021 §5.1 machinery that does not exist**: contracts 16,
   17, 17b, 18, 19 need symbolic-base resolution, `max_resolutions`, lazy materialization
   and `--fork-on-alias`. That is the next substantial piece of 021, not a test gap.

   **WAVE 44 — the copy-on-write review** (`a8ba8f4`; 595 tests). ✅ **The `Arc`
   substitution itself is sound**, verified three ways: no path mutates without
   `make_mut`, `MemObject` has no interior mutability, and `Memory` never hands out a
   `&mut MemObject`. That was the thing I was least sure of.

   ⚠️ **But `make_mut` ran before the refusal checks**, so an operation that changes
   nothing cloned the whole object — undoing contract 20 on the refusal path, which wave 41
   had just made the *common* case for a bit write into a symbolic byte. Quadratic again,
   restored by the commit that reported it. `MemObject::check_bit_write` is the same code
   the write runs, so pre-check and write cannot drift. **General rule: copy-on-write means
   deciding every refusal before the clone, not after.**

   Also: a negative-offset declared clobber reported `WildPointer` about an object it had
   just looked up — losing the dedup key's object component and, being fatal, killing the
   path; the special case is now **deleted** rather than corrected, since the per-byte check
   already answers correctly and a duplicate branch is a second place to disagree. A
   refusal reported success at `size == 0`. A bit range reaching a symbolic byte was
   correct but unpinned (every fixture kept the symbol in the *first* byte — the
   collection-of-one trap on a byte range). And `check_bits` filtered on `bit_width()`,
   which `Float` and `Vector` both answer, so `LoadBits { unit: f32 }` verified clean.

   **WAVE 45** (`ffb63c1`; 599 tests) — wave 44's five gaps closed. One was a **decision,
   not a fix**: `shares_storage_with` conflated "not shared" with "nothing to share" for an
   object past `MAX_MATERIALIZED_BYTES`, and neither answer was pinned. There is no storage,
   so a fork cannot have copied it — it counts as shared, and the alternative would show a
   phantom copy in any accounting built on it. Also pinned: a refused bit write leaves the
   object *exactly* as it was (a partly-landed range would upgrade init bits to `Yes` and
   turn a real uninitialized-read defect into a clean read); `HavocFill::Uninitialized`'s
   **success** path; and that a refusal carries its faults, without which a clobber of
   `const` memory producing **no finding** would ship green. `SymbolicByte` now says
   "concrete *access*", since refused writes raise it too.

   *(wave 44's list, now resolved)* `shares_storage_with` answers `false` for two forks of an
   *unmaterialized* object — "not shared" conflated with "nothing to share", and neither
   answer is pinned; `write_bits`' atomicity (a refusal leaving the object unmodified) is
   correct and unpinned; `havoc_range`'s `Uninitialized` **success** path has no test, only
   its three refusals; `refuse` returning an empty fault vector survives, so a clobber of
   `const` or freed memory producing *no finding* would ship green; and `report_faults`
   turns a `SymbolicByte` from a refused **write** into a finding whose text says a
   concrete *read* cannot answer.

   **WAVE 46 — 021 §5.1 STARTED** (`0e92c9e`; 600 tests). `IntToPtr` of an unpinned term
   resolves instead of refusing. `Memory::live_ranges` exposes the ranges; the engine
   filters arithmetically and asks the solver only about survivors, because §5.1 forbids a
   per-dereference O(objects) solver sweep — the interval tree is the optimisation, the
   filter is the semantics. `Budget::max_resolutions` (default 8) joins the budget set.

   **Steps 4 and 5 are distinct and step 4 is pinned**: wholly unconstrained → `Unknown`,
   an `UnresolvablePointer` finding, path **stops**, and *no pointer produced* — the last
   part is what distinguishes it from step 5's concretization.

   ⚠️ **My first test for this had the wrong premise and the code was right.** I wrote
   contract 16's fixture with an unconstrained `Fresh` value and expected four states; that
   is step 4, not step 3. **Lesson: when a new test fails, check whether the fixture
   exercises the case the contract names** — I nearly "fixed" correct behaviour.

   **WAVE 47 — correcting wave 46** (`f29457b`; 601 tests). ⚠️ **Wave 46 claimed §5.1 step
   4 was pinned and it was not.** With `solver-lite`, `addr ∈ [base, base+size]` over a
   variable falls outside §3.2's fragment and returns `Unknown`, so *every* unresolved
   pointer took the step-4 branch and reported "the value is unconstrained" — blaming the
   **program** for what the **tier** could not see. That is the same conflation 021 records
   against an earlier draft of its own §5.1, one level up, and I reproduced it.

   `Feas::Unknown` now marks the resolution undecided and reports `SolverUnknown`. **Step
   4's own detection is unreachable with this tier and is not claimed as covered** — the
   test says so rather than asserting whatever text appeared.

   *General lesson, and the second time in two waves:* a test that passes tells you nothing
   about **why**. Both of wave 46's tests passed; one was reporting the wrong cause.

   **WAVE 48** (`0ee0aa5`, and contract 17; 603 tests, **105/161 cited**). 021 contracts
   16 and 17 are covered. The fork machinery needed a **tier-2 backend** — `solver-lite`
   cannot decide `addr ∈ [base, base+size]` over a variable — so both tests take
   `SmtLib::discover()` and **skip with a printed reason** when z3 is absent (022 contract
   2). I verified they are not skipping here: z3 is on `PATH` and no skip line appears. *A
   test that passes by skipping silently is worse than the gap it was written to close.*

   §5.1's two ends are now pinned against each other: `Approximated` means "looked, found
   more than it would explore, picked one"; `Unknown` means "knew nothing". Both mutations
   die.

   **WAVE 49** (`32d6fd4`; 604 tests, **106/161 cited**) — 023 contract 18, and it found a
   real defect: ⚠️ **`max_states` was not a bound.** The check ran *after* a step's siblings
   were pushed, so `max_states = 4` ended with six states — and `RunResult::budget` reports
   that number as if it held. It now checks before each push, counting the **running** state
   as well as `done` and `work`; forgetting `s` itself is why the first fix still overshot
   by one.

   *The assertion that made the difference* was the exact state count. Without it, removing
   the `max_states` check entirely survived — `max_forks` and `max_depth` also end the run
   `Bounded` with the finding intact. Same-answer trap, one budget over.

   **023's remainder needs machinery:** contract 7 needs `RandomPath` and a seed in
   `RunResult`; 14 a renderer; 15, 16 and 21 a `Witness` type; 17 threading; 19, 20 and 22
   the checker framework; 24 the `CallReturn` event.

   **WAVE 50** (`ef07a22`; 605 tests) — ⚠️ **the resolved states carried no constraint at
   all.** §5.1 step 3 says "fork one state per object **with the corresponding
   constraint**"; the fork produced different `Pointer`s and added nothing to the path, so
   a state resolved to `ObjectId(2)` went on to take a branch requiring the address to be
   `ObjectId(1)`'s base. *A false positive carrying a witness that looks replayable is worse
   than a missed bug — it survives review.* Each candidate now asserts
   `addr ∈ [base, base+size]`; the wild state asserts the negation of all of them, which is
   what makes the fork **exhaustive** rather than merely plural.

   I found this myself, from the spec's wording, while the review of the same code was
   running. **Reading the contract sentence by sentence catches what a passing test does
   not.**

   *Verified in one half only, and said so in the commit:* the siblings' constraints fail a
   mutation; the constraint on the state that **continues** does not, because in this
   fixture that state is the wild one. Correct by construction, unpinned, recorded.

   **WAVE 51 — the §5.1 review.** Eight confirmed defects, **65% escape rate** on
   `resolve_symbolic_base`. It independently confirmed D2 (no constraints), which wave 50
   had already fixed. Two applied (`7f1aa0d`; 606 tests):

   ⚠️ **D1 — the resolution discarded the offset.** Every pointer was
   `Pointer { base, off: 0 }`, so `d[i] = 0xAA` wrote `d[0]` for every `i` and a pinned
   `addr == &d + 4` landed at byte 0. `pinned_offset` takes a model and then asks whether
   **any other** offset is feasible — *a model alone names one of many, and using it
   fabricates a position the program never chose.*
   **D7 — siblings reported `Exact` with no assumptions**, because the degrade ran only on
   the continuing state. 023 §7 attaches fidelity to *paths*.

   *Fixed and not pinned, recorded not claimed:* the sibling offset assignment and
   `pinned_offset`'s feasibility guard both survive mutation — correct by construction,
   no fixture separates them.

   **WAVE 52** (`7f1aa0d` and follow-ups; 608 tests) — D3 and D5 fixed. Feasible objects
   are **counted past the cap** rather than the scan stopping at it, and step 4 is decided
   **before** step 5: "every object *and* nowhere" is about the *program*, the cap is about
   *chiero*, and testing the cap first let the object count decide which one a reader was
   told. A guard-gap address is a **wild pointer**, not "unconstrained".

   ⚠️ **The D3 test passed before I added a tier-2 backend** — `solver-lite` answers
   `Unknown`, so the run took the `SolverUnknown` branch, which also gives `Unknown` and no
   pointer. *Third time in this area that a test passed for a different reason than it
   named; the fix each time was to make the fixture reach the code under test.*

   **WAVE 53** (`8c8c6ec`, `fa71709`, `162a4dc`, `b8412cf`; 614 tests) — the rest of the
   wave-51 §5.1 list except M8/M9. **D6**: `resolvable_ranges` (freed and out-of-scope
   objects included) is what the search now uses; `live_ranges` was left alone rather than
   widened, because "which objects exist" and "which objects may a pointer name" are
   different questions and one accessor answering both is how this went wrong.
   **D8**: the search no longer sweeps. Each query asks for *some* value the address can
   still take, the containing object is found by arithmetic, and the next query excludes
   it — one query per answer plus one to prove there are no more. A model landing outside
   every object proves the wild case, and what gets excluded next is the whole surrounding
   *region*, because ruling out one address at a time never terminates.
   Step 4 is now read off the path syntactically — if no constraint mentions any variable
   the address depends on, every value is feasible — so it needs **no solver at all** and
   is reachable at tier 1 for the first time. Exact in that direction; an address a
   constraint mentions without narrowing falls through to the enumeration, which reaches
   step 4 too whenever it runs to unsat.
   **L2/L3**: extent decides something only at the boundaries, so both are tested — inside
   (a size of 0 would resolve to nothing) and two past the end (a size of `size+1` would
   swallow it). The earlier single test was pinned strictly *inside*, where widening the
   extent changes no answer: the same-answer trap in the fixture, again.
   **M14** fell out of D8: the wild path now requires a model as witness, where before
   `Unknown` was enough. **D4** is covered by the one-past-the-end half of L2/L3.
   **G1**: `>= GUARD_GAP` is satisfied by `GUARD_GAP = 0`; the assertions now name a page,
   and the constant is pinned once where it is defined.

   ⚠️ *Method note:* `an_unresolvable_pointer_stops_the_path_and_names_its_reason` had a
   ⚠️ saying step 4 was unreachable at tier 1 and its assertion accepted **either** cause.
   That note is now stale and the assertion names step 4's own cause. A tolerant assertion
   left in place after the tolerance is gone is a test that has stopped testing.

   **WAVE 54** (`b19ddf9`, `dbdff8b`, `cb0692e`; 625 tests, 108/161 cited) — the report
   renderer and **witnesses**, both M1 exit items.

   `render(&RunResult)` (023 contracts 12 and 14) is the first thing here whose contract
   is about *text a person reads*: rule 4 governs a sentence, and a run can carry every
   assumption correctly and still print one that overclaims. It prints the fidelity, then
   either the findings with their own text or the one sentence chiero may say about an
   absence — two forms differing by exactly what rule 4 makes them differ by — then every
   assumption's own text, then the bounds whether or not they were hit. The words "no bugs
   exist" and "safe" appear nowhere and are *asserted* absent.

   `Witness` (023 §9, contract 15) — every `a.var` in the engine now goes through
   `Engine::input`, which records the symbol with an `InputOrigin`. All six sites moved at
   once rather than only the one the test needed: a witness that omits an input looks
   complete, and a harness built from it supplies every value but that one, so the bug does
   not reproduce and reads as *refuted*. Computed when the state finishes — the path is
   append-only, so a model of the final path replays through every finding on the way.
   It refuses to fabricate an absence (no inputs → the **empty** witness, not `None`) and
   refuses to guess (an input the model leaves free is `pinned: false`, not silently zero).
   Findings are a real type now: `reports()` returns `Finding`, and `findings()` is its
   projection to text so the two cannot disagree.

   The M1 exit item "an OOB finding **with a witness**" is done: `buf[16] = 1` behind
   `if (x > 10)`, one finding on the one path that has it, witness pinning `x`.

   ⚠️ *Method note:* the step-4 test asserted `solver_calls == 0`, and the witness query
   broke it. The assertion was about the *sweep* and had been written as a total — rewritten
   to compare the cost at 4 objects and at 40. A whole-run counter standing in for a
   per-object property holds only until anything else in the run costs a query.

   **WAVE 55** (`a0dc73d`; 628 tests, 109/161 cited) — **023 contract 16: a witness
   replays.** `Engine::replaying(w)` binds every input to the witness's value through the
   *same* `input` seam every symbol goes through; a separate replay path could drift from
   the one it is meant to reproduce, which is the failure a replay exists to detect.
   Bindings are consumed in creation order and each origin is checked against the site
   asking for it — a mismatch or a short witness is a **finding**, because a run that
   quietly stops reproducing while still reporting what it finds is how a refuted bug and
   an unrelated one come to look the same. Values go in as constants, so the replay of the
   guarded null store asks the solver nothing and explores one state.

   ⚠️ *Method note:* the "never advance the cursor" mutant **survived** the first round —
   with a single-input witness it is invisible. The two-input fixture had to be lifted out
   of its own test and replayed too. *A replay test with one input tests almost nothing
   about ordering.*

   **WAVE 56** (`a26c484`, `54654e4`; 636 tests) — the **wave-53 adversarial review of
   §5.1 reported, and it was right about three soundness bugs I had just introduced.**
   Every claim was re-derived here before acting; the reviewer's fixtures reproduce each.

   - **The syntactic step-4 test was unsound.** It asked only that no path constraint
     mention the address's variables. `char buf[32]; unsigned char i; p = &buf[i]` mentions
     `i` nowhere on the path and is still confined to one object plus a guard gap — *the
     term's own structure constrains the value.* chiero reported an unresolvable pointer
     and **stopped the path**. It now requires a **bare variable**, which is the only shape
     where "the path says nothing" is "nothing is known". My commit message had claimed
     "exact in that direction"; it was exact only for a bare variable, and nothing checked
     that.
   - **A cut enumeration concretized, reverting wave 52.** At 40 objects the enumeration
     always ends cut, so step 4 was again unreachable exactly where §5.1 was written for.
     Now: cut + wild-feasible is step 4 *unless* some object is provably not nameable —
     one counterexample settles it, at most `cap` probes, walking the list from **both
     ends** because a bounding constraint rules out objects at one end of the space.
   - **An `Unknown` mid-enumeration produced a non-exhaustive fork claiming `Bounded`.**
     Only an immediate give-up was caught. Any `undecided` now ends the path at
     `SolverUnknown`.
   - Plus: the concretize branch handed back **byte 0** (D4, in a branch nobody had looked
     at); `wild_region_around` used `saturating_add` where its caller used `wrapping_add`;
     and a symbolic address pinned to **zero** resolved to `UNBOUND` rather than `NULL`, so
     a null dereference through a symbolic value reported a wild pointer with `Unknown`
     where `NullDeref` is definite.

   ⚠️ **Two method notes, both expensive:**
   1. *An assertion over the wrong collection cannot fail.* "The search must terminate"
      checked `findings()` for text that `degrade` writes to `assumptions`. It passed while
      the exact condition it named was true. **Check which collection the text goes to.**
   2. *`wild_region_around` had no direct test* — the engine's fixtures pin the address, so
      the first model lands inside an object and the region logic never runs. Three
      one-address mutations passed the whole 614-test suite, two of them swallowing the
      legal one-past-the-end pointer 021 §7.1 names by hand. **A helper reached only
      through a happy path is untested however many tests call into it.**

   **WAVE 57** (`7d5afd4`, `38a580c`; 642 tests, 110/161 cited) — **scope markers are
   semantic now** (021 contracts 30 and 39). `InstKind::Marker(_)` was a no-op, so
   `Scope(Exit)` retired nothing, the two `Lifetime`s were indistinguishable, and a
   pointer to a dead block read as live memory.

   `Scope(Exit)` retires **this activation's** objects declaring **that scope** with
   **`Lifetime::Scope`** — each qualifier earns its place, and dropping any one of them is
   killed by a test: `AllocaId` is unique only within a function, retiring by anything
   coarser than the alloca's own `ScopeId` fires on every nested block, and 020 §4.4 says
   retiring `alloca()` memory with the scope reports use-after-scope on a program that has
   none. Function **return** retires the popped frame's objects, both lifetimes — the
   classic `return &local` reported *nothing* before. Lifetime faults now name **both**
   spans (024 contracts 8 and 10 ask for it, and the faults carried the second one all
   along while `Display` dropped it).

   ⚠️ *Method note:* the use-after-return test first asserted the run must not be `Exact`.
   That was habit, not 023 §7 — a write through a pointer to a dead frame is a **definite
   fact modeled exactly**, and degrading would claim chiero was unsure when it was not.
   The engine already says so for null dereferences and bad frees. *An assertion about
   fidelity needs a sentence of the spec behind it, not an intuition that bugs are fuzzy.*

   **WAVE 58** (`7e4beac`; 650 tests, 111/161 cited) — **the wave-55 review of witnesses
   and the renderer reported, and its headline finding was a false statement, not a gap.**

   - **The witness said "no symbolic inputs on this path"** about paths whose whole
     condition came from symbols `chiero-mem` minted — havoc'd extern pointees, clobbered
     bytes, materialized uninitialized bytes. `Engine::input`'s doc claimed to be the seam
     every symbol passes through; three sites in another crate were never in it. Memory
     records them now. A whole-object havoc is an **array** — no `Binding` carries it — so
     the witness is *refused with that reason* rather than reported empty.
   - Consequently a replay cannot supply what memory re-invents, and now **says so**
     ("replay incomplete: N value(s) … re-invented rather than supplied"). This is a real
     limit, stated rather than hidden.
   - **`ModelApproximate` was missing from `is_modeling_lie`**, so contract 12 was false of
     shipped code: a `scanf` run carries exactly one assumption, the right one, and had
     none whose kind accounted for its fidelity.
   - **The replay cursor lived on the `Engine`**, so a forking replay let the first path
     eat the second's bindings. It is per-state now, and a diverged replay *stops* —
     before, the sentinel meant "past the end" and the missing-binding branch re-reported
     the same divergence at every remaining site.
   - **One degraded sentence blamed the bounds for every cause.** Each fidelity level has
     its own now; 023 §7's preamble warns against precisely that collapse.

   Ten mutations the 628-test suite accepted are now killed, including the reviewer's
   worst: widening the `Exact` branch so a **degraded run prints "the search was
   exhaustive"** — 023 §7 rule 4 verbatim — which contract 14's golden test missed because
   its only fixture was `Bounded`.

   ⚠️ *Method note:* three of those survivors lived behind fixtures that give the same
   answer either way — the contract-12 fixture forks *after* its single degradation, so
   every state carries the same list and "read only state 0" was invisible. **A fixture
   whose paths agree cannot test a claim about paths.**

   **WAVE 59** (`0b06d31`; 651 tests, 112/161 cited) — the wave-55 review's last soundness
   finding. `reports()` deduplicated **across paths** on 023 §6.1's key minus its
   `checker` component, one layer too early: two out-of-bounds writes at two offsets on
   two paths came back as one finding, and the second's witness was discarded. Contract 20
   says the engine does not deduplicate. Across paths only the finding **id** may merge —
   that is what recognises a fork's copies — while *within* a path the key still applies,
   because a loop running through one fault repeatedly is one report and dropping that
   half fails two existing tests.

   Still open from that review, deliberately: nothing outside `chiero-exec`'s own tests
   calls `render` or `reports` (`chiero-cli`, `chiero-check`, `chiero-tool` depend on the
   crate but not on these), so contract 14 is a golden test on text no user sees yet.
   Whichever crate eventually prints a run must call `render` rather than write its own
   sentence — §7 rule 4 does not follow it there by itself.

   **WAVE 60** (`4740826`, `9a89f8a`, `51998d3`; 661 tests, **120/164 cited**) — solver
   work: 022 contracts 15, 19a–19d, 10, 11 and 12.

   `spec:` first — the numbered contract 19 still said "`x / 0` evaluates to all-ones",
   which §2's own table contradicts in three of four cases (`bvsdiv -5 0` is 1;
   `bvurem`/`bvsrem` by zero are the *dividend*). §2 already promised "contracts 19a–19d
   pin each case separately"; the list was never updated. Split as promised, then tested
   folded, evaluated, and against z3 — the three-way check is §2's own argument, since the
   folder and the independent evaluator would share a wrong rule and the evaluator would
   then *validate* a model built on it.

   `SolverStats::backend_errors` and `SmtLib::at` (contract 15): a backend answering
   unparseably is otherwise invisible — every query comes back `Unknown`, every consumer
   degrades honestly, and a run that decided *nothing* reads as a run over a hard program.

   **The counterexample cache** (contracts 10–12) with §6's inverted index. The absent
   rule is the point: the subset of an `Unsat` set is where a wrong direction would be
   "silent and catastrophic", so only the superset direction is a rule. A candidate model
   is *evaluated* against the query before it answers it — returning one unchecked invents
   satisfying assignments for unsatisfiable queries.

   ⚠️ *Method notes, three from one file:*
   1. The first cache fixtures used **linear** constraints, which tier 1 decides — so
      `backend_calls` stayed at zero whether a cache existed or not and three tests passed
      against no cache at all. **A cache test needs queries the cache is the only way to
      avoid paying for.**
   2. `TermArena::var` mints a fresh `VarId` per call, so `x0 == 0 ∧ x0 == 1` built from
      two `var(…, "x0")` calls is two variables and satisfiable. The fixture failed for
      that reason rather than the one it was written for.
   3. Filling the cache with 1000 *backend* queries took 94 s; the contracts need 1000
      distinct **entries**, not 1000 hard ones.

   And one existing test had to change: the new cache legitimately answers the query that
   `a_killed_backend_is_restarted_and_the_stack_replayed` used to force a restart with, so
   it failed against a solver that restarts perfectly well. It now excludes the model
   already in hand. *A test that measures a mechanism by "did the backend get asked" is
   coupled to every future reason not to ask it.*

   **WAVE 61** (`5b351dd`, `6ab6960` + volatile; 671 tests, **122/164 cited**) — two CIR
   semantics gaps that were pure silence, both in 020.

   **UB events** (contracts 8 and 9). §4.1's separation — "defined IR semantics, UB
   reported as findings" — had only its first half: a shift past the width and a division
   by zero computed the right value and told nobody. `UbEvent { kind, span, detail }` is
   recorded per path for the three rows of §4.1's table. The value is unchanged (it is the
   SMT-LIB value, "so the IR and the solver cannot disagree") and the path **continues**,
   asserted directly because an earlier spec draft stopped the path on division alone.
   Two deliberate limits, both in the code: events fire only on **concrete** operands (a
   symbolic divisor *may* be zero, and deciding that is 040's business with a budget, not
   the interpreter's), and the engine does not decide whether a wrap was a *mistake* —
   VPP wraps on purpose all over.

   **Volatile** (§4.2, contract 41). `Volatility` appeared nowhere in the engine: a
   volatile load read back the bytes stored, so `*reg = 0; if (*reg == 0)` was a certainty
   here and is not on the device. Loads now yield a fresh symbol **each time**, recorded
   as `InputOrigin::Volatile` so a witness binds what the device returned; stores push an
   `Effect` in program order, before the write, and nothing coalesces them.

   ⚠️ *Method note:* the non-coalescing mutant survived the first test — it used two
   *different* store instructions, so a coalescer keyed on the site slipped through. A
   **loop** fixture is what catches it. Same family as the extent trap in wave 53: *if two
   fixtures differ in the dimension the mutation does not touch, the mutation lives.*

   **WAVE 62** (`9993247`; 675 tests, **124/164 cited**) — 020 contracts 42 and 43, both
   of which were `_ => give_up`: the engine reported "chiero cannot follow this" about
   constructs the IR *defines*, which in the output is indistinguishable from a program
   that really is unfollowable.

   **`Const::Undef` is a value now** (`Value::Undef`), and deliberately *not* a fresh
   symbol: a symbol is a value nobody has pinned yet and the solver may pin later, while
   `Undef` is a value that does not exist. Arithmetic and comparison propagate it —
   including `undef * 0`, which is 0 for every *value* and undefined for a non-value.
   Storing it leaves the destination uninitialized (so a later read is 021 §3.1's finding,
   not a read of something chiero invented). A branch on it forks both ways and adds **no**
   path constraint — there is no term to constrain, and inventing one would let a later
   query prove something about a value that does not exist. The degradation is `Unknown`,
   not `Approximated`: chiero approximated nothing, it propagated the program's own
   absence of a value.

   **`IndirectGoto` forks per declared target.** The address is not resolved to a block —
   a label address has no representation the memory model can match against a `BlockId` —
   so the run records `Approximated` and says the target list is the *frontend's*
   declaration rather than implying the set was computed. Contract 42 came with an explicit
   "drop the contract if you drop the terminator" escape hatch, which this makes moot.

   *Method note:* making `Value` non-exhaustive forced a decision at three match sites
   (return value, store operand, `scalar`) that would otherwise have been defaulted. **A
   new enum variant is a free audit of everywhere the old ones were assumed total.**

   **WAVE 63** (`af00414`, `9c1a7e1`-ish spec; 677 tests, 124/165 cited) — **the wave-60
   review of the counterexample cache reported, and its worst finding was a 115× slowdown
   on the case 023 §1 is built around.**

   - **Below tier 1, not above it.** The cache was answering queries *both tiers* would
     have refused — including assigning a truth value to a non-predicate term, returning
     `Sat` for something z3 rejects as ill-sorted. §6 puts the caches "below escalation";
     this is that, one level finer. It also fixed most of the performance problem: a
     tier-1-decidable query no longer reaches the cache at all (1000 shared-prefix states:
     **14.7 s → 0.085 s**).
   - **Bounded** (§6.2 asked and I had skipped it), and candidate *evaluation* capped:
     sibling states share long prefixes by design, so a shared term puts every cached set
     in the inverted index — the index that made enumeration cheap does nothing for
     evaluation. Eviction is wholesale, not LRU, and the comment says why (`CacheSlot` is
     a positional index; a real LRU needs stable ids).
   - **Arena identity checked.** §6.2 says caches are per-`TermArena`; `check` takes one
     per call and every key is a bare `Term` id, so a second arena's `Term(3)` was a
     different term with the same name — and the subset rules turned an exact-collision
     hazard into a subset one.
   - Four accepted mutants killed, including **an eval *error* counted as "satisfied"** (a
     false `Sat` for an unsatisfiable query — `eval`'s totality is what the whole sat rule
     rests on) and **`remember` storing only the assumptions** (a false `Unsat` after a
     `pop`).

   **`spec:` commit — 022 contracts 8, 8b, 11d.** Contract 8 ("byte-identical models … from
   a warm cache") and contract 12 ("a cached model answers a new query") **cannot both
   hold**: a model computed for a different query is not the model a fresh solve returns.
   The two caches now carry different promises — the exact cache is byte-identical, the
   counterexample cache guarantees the *verdict* with any satisfying assignment — and §6.2
   states why reproducibility survives (it is a property of *runs*: same query sequence →
   same answers, which is what a witness replay needs).

   ⚠️ *Method notes:* three fixtures had to be rebuilt **twice** — once because the cache
   now sits below tier 1 so a tier-1-decidable contradiction never reaches it, once because
   the *exact* cache answered before the counterexample cache could. **A test that cannot
   reach the code it names proves nothing, and moving the code under test moves that
   line.** And the ≥1000-entry fixture failed to defeat "remembers only the last query"
   purely because `fill` ran *before* the decisive query instead of between it and the
   lookup — the spec asks for 1000 entries for exactly that mutant, and the fixture handed
   it a pass.

   **OWED from this review:** contract 15's other two causes (a model that fails
   independent evaluation, and a dead process) are still unpinned — `backend_errors += 1`
   can be deleted on the validation path and everything passes. Testing it needs a backend
   that speaks the session protocol correctly and *lies*; a shell-script attempt hung,
   because a fake that does not answer every command exactly is indistinguishable from a
   slow one. Also owed: §6.1's `possibly_infeasible` remains unimplemented — not an
   unsoundness today (every stored set is a full assertion set, so the superset rule is
   monotone), and `remember` now carries the comment naming the coupling slicing will
   create.

   **PARALLEL: M2 frontend dispatched to codex** (pty-4, `--yolo`) in a **separate git
   worktree** at `/home/ubuntu/rust/chiero-m2`, branch `m2-frontend`, brief in
   `M2-BRIEF.md` there. It owns `chiero-lex` and `chiero-pp` only; `docs/specs/**` and
   `chiero-span` are read-only to it (a spec that quietly follows the code stops meaning
   anything). Its exit gate is 012's `gcc -E`/`clang -E` differential, which is a stronger
   oracle than anything M1 has. **That branch is reviewed adversarially before it merges.**

   ✅ **MERGED at wave 84** (`c02a0a4`), at the third gate; the worktree and the branch are
   gone, and the brief moved to `docs/reviews/m2-frontend-brief.md`. Rejected twice first —
   `docs/reviews/m2-frontend-round-{1,2}.md` are worth reading for the method more than the
   findings.

   **WAVE 64** (`6a16c3c` + the lying-backend test; 681 tests, **127/165 cited**).

   **021 contract 27 was violated, and it is the kind that fires everywhere.** An entry
   pointer parameter's pointee was an ordinary uninitialized object, so reading through one
   reported an uninitialized read — on *every function that takes a pointer*. §6 names this
   outcome in advance ("an uninitialized-read false-positive storm") and §3.1's whole
   bit-granular init apparatus exists to make the distinction expressible: a caller-supplied
   buffer is **unknown**, not **unwritten**. Entry pointees are `ObjKind::Lazy` and havoc'd
   symbolic now.

   Also cited, both already correct: contract 3 (out of an object and back preserves `base`
   — the `container_of` idiom, which works only because provenance rides the pointer rather
   than being recomputed from the address) and 13b (an unconstrained `IntToPtr` is never
   read as in-bounds, because step 4 stops the path — the `vlib_get_buffer` case).

   **022 contract 15's second cause is now pinned** — the item owed from wave 63. The fake
   backend speaks the session protocol correctly and lies only in its answer: a valid model
   with every variable zero, for a query saying one of them is 7. The earlier shell attempt
   hung because *a fake that does not answer every command exactly is indistinguishable
   from a slow one*. It kills both the missing counter and trusting tier 2's model
   unvalidated. Contract 15's third cause (a dead process) is still unpinned.

   **WAVE 65** (`c0bc0a3`, `499b67f`; 683 tests, **129/165 cited**).

   **The M2 agent found a defect in my gate**, and it was the right kind: `contract-
   coverage` scanned only 020–024, so it reported a comfortable number about the half of
   the work it could see while the frontend was being built in parallel. 010–015 are
   measured now and printed **separately, never folded into the M1 number** — 080 states
   M2's exit as *behaviours*, not as "all contracts of these documents", and a gate that
   invents an exit criterion the roadmap did not state is worse than no measure. Reads
   `010: 17/19` and zero for 011–015 in this tree; that zero is the useful part, and is
   what it should say until `m2-frontend` merges.

   **024 contracts 21b and 21e.** 21b is the proof surface in one case: a program that
   calls `scanf` cannot seal, because a run that read the outside world explored *one
   story about* the program. 21e pins the default that keeps it usable — an unmodeled
   extern's havoc leaves bytes **symbolic**, and flipping that default fails four tests
   (three pre-existing), which is the same storm 021 §6 names, one crate over.

   ⚠️ **Cross-agent dependency found:** 010 contract 19 (per-`ConfigId` expansion sites)
   cannot be done here. 001 §180 says **`ConfigId` is owned by `chiero-pp`**, which is the
   M2 agent's crate — and `chiero-span` is *below* it in the layering, so
   `CookedSite.config` has nowhere to come from yet. Left alone deliberately rather than
   solved by putting the type in the wrong crate. Worth settling at merge time.

   **WAVE 66** (`6adb437`, xtask fix; 683 tests) — **the M2 branch went through the merge
   gate and did not pass.** The review built a differential harness against **both** `gcc
   -E -P` and `clang -E -P` (which agreed with each other on all 20 torture cases), plus a
   25-mutation campaign: **13 of 20 cases diverge, 12 of 25 mutations survive.** Full
   report handed to the M2 agent as `REVIEW-1.md` in its worktree; it is working through
   it now.

   The lexer is sound (six real VPP headers tokenize identically to gcc *and* clang) and
   010 §3.2's provenance model reproduces exactly — the hardest part to retrofit is right.
   The preprocessor is not: expansion runs **per line**, so a macro call spanning two lines
   never expands (**2996 such sites in VPP**); rescanning does not see following tokens;
   `#if` silently picks the wrong branch on `&`, `?:`, hex, octal and char literals; and
   `#include` is a textual pre-pass, so `__FILE__`/`__LINE__` are wrong in every header —
   which poisons exactly the 030/032 keying provenance exists for.

   **Three findings were about my side, and are fixed here:**
   - `contract-coverage` excluded files by `Path::ends_with("contracts.rs")`, which matches
     the final *component* — so a crate's own `tests/contracts.rs` would have had all its
     citations silently dropped. Harmless until I extended the gate to 010–015 last wave;
     a live landmine after. Now excluded by full path.
   - `spec:` **012 §2.3** — `__VA_OPT__` is out of v1 scope *explicitly*, with a
     diagnostic rather than a silent pass-through. Measured: VPP uses `__VA_ARGS__` 230
     times and `__VA_OPT__` **zero**. Same line now also states that GNU comma-swallowing
     deletes the comma only when the variadic argument is **empty**.
   - `spec:` **070 §4** — a contract cited only by an `#[ignore]`d test counts as
     **uncovered**. The review found three such citations, one of which returns early when
     its input file is missing (it is). *A green number that says a human has looked, when
     nobody has, is worse than a blank.*

   ⚠️ *Method note, the most valuable one this wave:* the review's most damning evidence was
   not a failing assertion but a **differential harness against two independent oracles that
   agree**. Every M1 review has had to argue from the spec; this one could point at gcc.
   That asymmetry is why 080 says M1's oracles are weaker "rather than discovering" it —
   and it is worth remembering when M1 work *feels* well-tested.

   **WAVE 67** (`48890a7`, `3d3e700`; 686 tests, **132/165 cited**) — pinning the three
   contracts I amended in wave 63 and did not test. *Amending a contract and not checking
   it is the same "spec says, nobody checks" pattern this project keeps finding in its own
   work, and it is worse when the amendment was mine.*

   - **8**: same query, cold and from an exact-cache hit → byte-identical model, asserted
     both as map equality and as identical rendering (a golden test sees the second). Two
     independent runs agree, on tier 1 and on a backend query.
   - **8b**: a counterexample hit keeps the verdict and may change the assignment — *both*
     halves, because "may differ" is not a licence to be wrong: the returned model is still
     evaluated against every constraint of the query it answers. The fixture arranges for
     the cached model to be the **larger** one, so the difference is real.
   - **11d**: the consumer is `Engine::feasible`, which turns an answer into a fork
     decision *and* a degradation — so the test asks one nonlinear question twice on a path
     and asserts the run stays `Exact` with no assumptions, plus that the solver was
     reached more than once or it proves nothing about caching.

   `Model` had **no equality at all** until contract 8 was written — a contract about
   identical models, and nothing in the workspace could compare two.

   Contract 8's fifth way ("with slicing disabled") is *not* claimed: independence slicing
   does not exist, so the test would assert against itself. Said in the test's own doc
   comment rather than quietly counted — 070 §4's new rule applies to me too.

   **WAVE 68** (`b5777db`; 688 tests, **133/165 cited**) — 024 contract 7, `strlen` over
   symbolic bytes, which the code itself had marked owed ("the symbolic fork of §4 step 2
   is owed"). Without it chiero could not measure a string it had not written, which is
   most strings in a real program.

   `strlen_symbolic` reads each byte as a **term** and forks one branch per position the
   NUL could be, guarded by "every earlier byte non-zero ∧ this one zero". A concrete byte
   is a constant term the arena folds, so §4 step 1's fast path falls out of the same code
   instead of being a second copy of the walk. The tail branch is an **unterminated-string
   finding** when the object's end was reached and a **bound** when the scan cap was —
   §4's rule that steps 3 and 4 must not cancel, since an earlier draft had the cap
   "constrain a terminator to exist" and thereby assumed away the bug step 4 finds.

   **Three defects it exposed, all in my code:**
   - **Model fork guards were dropped entirely** (`Some((_, ...))`). Every sibling of a
     fork carried the same path condition — `malloc`'s two branches were indistinguishable
     to the solver, and a `strlen` fork would have made four states that all still believed
     the string could be any length. A sibling now drops *this* state's guard before adding
     its own; sharing them would make the states mutually contradictory rather than merely
     unconstrained.
   - **A value-less branch inherited the previous branch's value**, because siblings are
     cloned after this state's result is in place. The unterminated branch reported
     length 0 as its own.
   - **`ModelOutcome` could not express a bound.** A cap reporting a `Finding` accuses the
     program of chiero's limit; one saying nothing lets a truncated scan pass as complete.
     `Bounded(reason)` is neither.

   *Method note:* the fixture failed twice before it measured anything — a declared callee
   with the wrong parameter count does not verify, and the run reported "the module was
   never executed". **A fixture that does not verify reports the absence of everything.**

   **WAVE 69** (`b076d50`; 690 tests, **136/165 cited**) — 020 contracts 13 and 34, both
   determinism claims about the textual format, which is where 001 §5's hard requirement
   becomes visible: a golden test compares text, so an unstable case order or a lossy
   literal turns every downstream diff into noise and hides the change that mattered.

   Contract 13's fixture writes its cases **out of order** deliberately — a printer that
   sorted would pass a test whose cases were already sorted, and lose the program's own
   order. Contract 34 asserts the 512-bit literal's *words*, not merely round-trip
   stability, because a printer and parser that agreed on a wrong value would satisfy the
   round trip; `memcpy_x86_64.h` manipulates `u8x64` directly, so a truncating format
   yields a module that parses, verifies, and is not the program.

   Also: 020 c39 and 021 c30 state the same property from two sides and both halves were
   already tested, but only 021's number was cited — so 020's read as nobody's work.
   *When two documents state one property, citing one of them leaves the other looking
   unexamined.*

   A review of wave 68's forking change is running: it altered how sibling states get
   their path conditions, which is soundness-critical, and it is being checked against
   `strlen` compiled with gcc as an oracle.

   **WAVE 70** (`580bd3e`; 693 tests, **137/165 cited**) — 022 contract 7d, and it was RED
   for a real reason: **`eval_node` called `fold`**, so §3's "independent evaluator" was
   the constant folder under another name, and a model built on a wrong rule was validated
   by that rule. §2 predicts the consequence in as many words, and it had already happened
   here — `bvsdiv -5 0` is `1`, the spec said all-ones, the folder implemented all-ones,
   the evaluator agreed, and **z3 found it because z3 was the only oracle not sharing the
   code**.

   `independent_bin` is now a deliberate second implementation from SMT-LIB's definitions:
   subtraction as two's-complement addition, division via `checked_div` with the zero cases
   spelled out, shifts guarded by what the standard says about a count at or past the
   width. It must not be refactored to share code, and a test reads the source to check
   the call has not come back.

   ⚠️ **Two traps, both mine, both caught only by mutation:**
   1. The differential compared `as_const` with `eval_ground` over terms built from
      **constants** — which the arena folds on construction, so the evaluator was never
      reached and *the test compared the folder with itself*. Three deliberate breakages of
      the evaluator survived it. Operands must be **variables** with a model.
   2. The mechanical check sliced the source "to the next `    fn `", found none (the next
      item is `pub fn`), and scanned the rest of the file — which contains the folder. It
      failed loudly, which is the direction to be wrong in; the window is brace-balanced
      now and asserts it terminated.

   A third mutation — the shift boundary `>=` → `>` — **survives and is equivalent**: the
   value is masked to width afterwards, so a count of exactly `w` clears it either way.
   Recorded in the code, because a surviving mutant usually means a missing test and this
   one does not.

   **WAVE 71** (`437726b`; 696 tests) — **the wave-68 forking review reported: 13 of 27
   mutations survived, and it found two live defects with no mutation at all.** The three
   fixed here are the ones that make chiero wrong about real programs.

   1. **Every store through an entry pointer parameter was silently dropped.** Wave 64's
      `havoc_object(Symbolic)` sets `Repr::Array`, and every byte-level *write* path
      refuses a promoted object — so `p[1]='a'` never landed and the following
      `if (p[1]=='a')` explored **both** sides. A path the program does not have, on the
      most common idiom in C, reaching `memcpy`/`memset`/`strcpy` into any caller buffer.
      ⚠️ **My first fix was worse**: suppressing the uninitialized-read *finding* left the
      byte reading back as the backing store's **zero** — 021 §3.1's headline failure,
      introduced while fixing something else. The pointee is filled byte-wise with symbols
      now: `Repr::Bytes`, writable, every byte unclaimed. One term per byte; filling on
      first touch is the optimisation, not the correctness.
   2. **`strlen_symbolic` reported a false OOB on a *terminated* string** — no guard was
      checked for satisfiability, so `buf[1]=0` still produced lengths 2 and 3 plus an
      unterminated-string finding. A fabricated bug of exactly the class §4 step 4 exists
      to catch. gcc over all 256 inputs gives 0 and 1; so does chiero now.
   3. **A one-byte concrete prefix disabled the fork**, because dispatch gated it on the
      concrete walk having scanned *zero* bytes.

   **WAVE 72** (`c756de2`; 701 tests) — the rest of that review, applied.
   - The three **guard mutations** lived because the fixture had no branch after the call,
     so contradictory and overlapping paths looked identical to correct ones. The new
     fixture branches on the **bytes** (`if (p[0] == 0)`) and asserts no state is
     self-contradictory, each length implies its side, and no length appears twice.
   - **A value-less branch terminates its state.** §4 step 3 wants the cap to terminate
     with `Bounded`; the state carried on with `dst` unbound and the first *use* hit
     "branch condition is not a scalar" — 023 §3's marker for a bug in chiero — pinning the
     run at `Unknown`. The test now asserts `Bounded` **exactly**, where `!= Exact` had been
     satisfied by `Unknown`.
   - **This state's report is applied after its siblings are cloned** — the guard was undone
     per sibling, a finding and a degradation were not.
   - **Three edge guards pinned**, plus `-p.off` panicking at `i64::MIN`.
   ⚠️ *Method note:* the fault-guard test first used a **freed** object, where the read
   yields no value at all — so the fault check was never the deciding factor and dropping it
   changed nothing. An **uninitialized** read is the case that distinguishes it: a value
   *and* a fault. **Pick the fixture where the guard is the only thing standing between you
   and the wrong answer.**

   **WAVE 73** (703 tests) — that review is now **fully closed**. 024 c21e's middle clause
   (the fixture writes the bytes first, so the *havoc* is what un-initializes them — a
   fresh object would report anyway and the test would pass without the havoc doing
   anything), c21d (the inner object is found by reading its address out of the outer
   one's bytes, and depth 0 is asserted *not* to follow it), and the `scanf` test, which
   could not tell the modeled path from the unmodeled fallback because `contains("scanf")`
   matches both — de-registering the model left it passing. It asserts
   `AssumptionKind::ModelApproximate` and the model's own reason now.

   One survivor is left **and explained in the code**: the `HavocInit::Uninitialized` arm
   in `apply_havoc` is unreachable, because a `ModelEntry` carries only a name and a
   precision and no built-in model returns that fill. The memory side is pinned directly;
   the translation cannot be until a model can declare a havoc. *An unexplained survivor
   reads as a missing test; a named one is a fact about the design.*

   ⚠️ *Method note:* the 21e fixture needed 4-byte alignment — at align 1 the read reports
   `Misaligned`, so "the fixture starts clean" was false for a reason with nothing to do
   with initialization. **A precondition assertion can fail for the wrong reason too.**
   ⚠️ Also: **three claims in my wave-68 commit message are false** — `len` is a constant
   so a later `if (len == 2)` folds regardless of guards (the guards matter for branches
   over the *bytes*), `malloc`'s branches never changed, and the concrete fast path does
   *not* fall out of the same code.

   **WAVE 74** (`cc2a098`; 708 tests, **138/165 cited**) — 024 contract 22, format-string
   checking. `printf` was registered as approximate and never dispatched, so chiero saw
   neither of the two bugs behind a bad format string. It is dispatched for the *check*
   only: the model returns no value, because the output is still not modeled.

   The two findings are kept apart deliberately — a **format mismatch** is the program
   lying about what it is passing, a **memory** fault is the program handing `printf` bytes
   it may not read. `%s` of a null pointer is the second, not the first, and reporting one
   as the other sends a reader to the wrong line. A format string chiero cannot read
   concretely is `Bounded`, not a finding: an unreadable format is a gap in chiero, and
   blaming the program for it is what 023 §7 exists to prevent.

   *Method note:* the `%%` mutant needed its own fixture and is the one that matters for
   **noise** — every `"100%% done"` in a codebase would otherwise be a false "conversion
   with no argument", and a checker that noisy gets turned off. **The negative cases are
   what make a checker usable, and they need fixtures as specific as the positive ones.**

   Two gates moved with it: `printf` joined the dispatchable and implemented lists, and
   `everything_dispatchable_is_implemented_and_vice_versa` had used `printf` as its example
   of a *registered but unimplemented* model — `fscanf` takes that role, since the list must
   keep at least one **registered** name or it only tests that made-up names are missing.

   **WAVE 75** (`711d574`; 710 tests, **139/165 cited**) — 021 contract 25, promotion
   preserves initialization. This one had been **owed on a premise that stopped being
   true**: the note said "the byte API refuses a promoted object, so comparing the three
   init states before and after needs an array-aware accessor" — and `init_bit_via` has
   been that accessor for several waves. *An owed item's reason expires; re-read it before
   trusting it.*

   Flattening `Cond` either way decides a question the program left open — to `Yes` and a
   genuine uninitialized read stops being reported, to `No` and every guarded write becomes
   a false one. Both flattenings now fail. The fixture asserts it contains all three states
   before comparing, and `Cond` is compared by **meaning** rather than term identity: the
   array form is `ite(t,1,0) == 1`, a different expression for the same guard.

   A review of waves 70–75 is running, aimed squarely at the highest-risk thing in them:
   whether `independent_bin` and `fold` **both** get the semantics right, checked against
   z3 over random operands at four widths. A shared wrong rule is invisible to model
   validation by construction, which is why 022 §2 wanted two implementations in the first
   place — and having two is worth nothing if nobody differentially tests them.

   **WAVE 76** (`7573135`; 712 tests, **140/165 cited**) — 021 contract 18, the aliasing
   policy. Neither half existed: two pointer parameters got distinct objects because they
   were allocated separately, and **nothing said so**. That silence is the whole risk —
   assuming two pointer parameters do not alias is how an under-constrained run stays
   tractable, and equally how a real aliasing bug goes unseen.

   The run records it, prints it, and **degrades to `Approximated`** so it cannot seal as a
   proof: assuming away aliasing is 023 §7's "keeping one of several feasible values". That
   last part is what the first version of the test missed — it checked the assumption's
   *text* and not the claim, and the mutation that left the run `Exact` sailed through.
   *An assumption recorded without the fidelity to match is one `seal` steps over.*

   `--fork-on-alias` adds one state per pair, each naming what it explored. §6 describes
   `2^(pairs)`; pairwise is the subset that answers "do *these two* alias" — the question a
   checker asks — and grows quadratically rather than exponentially.

   ⚠️ *Method note, third occurrence:* the fixture failed twice before measuring anything —
   an 8-bit store of a 32-bit constant, then a return type that did not match the value
   returned. **A module that does not verify reports the absence of everything**, and it
   looks exactly like a passing negative assertion. Now written into the fixture's own
   comments rather than only into this file.

   **WAVE 84** (`d0966af`, `c02a0a4`, `44ca77c`; **835 tests**, 152/165 cited, frontend
   **48/117**) — 021 §5.2 arenas landed, and **the M2 frontend is merged; the worktree is
   gone**. Two things happened here, and the second is the milestone.

   *Arenas.* `ArenaShape { pitch, elem_size, index_scale, count }` +
   `Engine::with_arena`, resolved **before** §5.1's search — running the search first
   either concretizes the address (step 5) or ends the path (step 4) before the arena is
   consulted. `vlib_buffer_ptr_from_index` resolves now, which is where every VPP node
   analysis previously died.

   **The RED commit argued a three-way fork; implementing it found a fourth outcome.**
   §5.2 step 3 puts the gap at `d >= elem_size`, so `0 < d < elem_size` is a *legitimate*
   pointer into the middle of element `k` — and `Pointer::off` is an `i64`, so this memory
   model cannot represent it. Forcing `d == 0` on the good path would have deleted every
   one of those silently, which is exactly what §5.1 calls "a wrong answer instead of an
   honest unknown". It is its own state now, `Fidelity::Unknown` /
   `AssumptionKind::NoInformation` / `Terminated(Unsupported)`.

   Each case is created **only where it is feasible**, or every concrete buffer access
   would carry three siblings with unsatisfiable path conditions — states that cost a
   fork, report a finding, and describe nothing the program can do.

   ⚠️ *The method note that matters here.* Every structural assertion (how many states,
   which findings) **survived mutating the divisor to `elem_size`, mutating the gap test
   to compare against `pitch`, and deleting the `count` bound** — because the fork happens
   either way. The arithmetic needed ground-index tests, and the first cut of one used
   `n = 2496`, where `n/pitch` and `n/elem_size` are *both* 1, so the divisor mutation
   passed. It is written at `n = 4864` now — two `elem_size`s, one `pitch`. **Structure
   tests are satisfied by the shape of the computation, not its content.**

   Fallout: `SolverLite` decides **ground** assertions before §3.2's fragment test. A
   folded contradiction collapses to the constant `false`, which is not an atom, so it
   left the fragment, returned `Unknown`, and reached a backend that cannot assert a bare
   constant either — `Unknown(BackendError)` for a formula needing no solver, and 023 §3
   then explores a ground-refutable branch. Every concrete arena index has this shape.

   Also honest: **the RED commit did not compile** (`f.message` on `State::findings()`,
   which is `Vec<&str>`), so it was a build failure rather than the behavioural RED the
   rhythm asks for.

   *The M2 merge, at the third gate.* Re-run at the branch's HEAD: **seven real vppinfra
   headers are token-for-token identical to gcc** under gcc's **full 391-macro predefine
   set** — `clib.h` 257,310 tokens, `vec.h` 277,790, `pool.h` 291,727, `bitmap.h` 289,164,
   plus `hash.h`/`format.h`/`error.h` — **zero diagnostics**, `#pragma` counts matching
   exactly (375, then 381). 34 torture cases match. REVIEW-2's blocking findings verified
   individually: 400 sequential `M(x)` calls expand, `<foo/bar.h>` stays literal with
   `foo` defined, 20,000 nested `#if` parens and a 100,000-deep call chain diagnose and
   exit 0 instead of `SIGABRT`, `#if 0x8000000000000000 > 0` matches gcc.

   Two divergences remain and **neither is a defect**: `#pragma`/`_Pragma` is recorded
   out-of-band rather than emitted into the token stream (012 §3 and `ExpnKind::Pragma`
   ask for exactly that, and the parser wants it that way), and an arity error is
   *diagnosed and recovered* where gcc and clang hard-error and stop — recovery is
   required, because 030/032 must diff revisions that do not compile.

   ⚠️ **The one methodological lesson to carry into every frontend claim from here**, from
   REVIEW-2's addendum: under the old 5-macro predefine stub, real headers took
   *different branches* from gcc, so "zero diagnostics on real headers" was literally true
   and analytically worthless — 257,310 tokens against 224,074. **Agreement means nothing
   unless both sides get the same predefines.** Feed `gcc -dM -E` through `Config::defines`
   or the number is about code neither tool ran.

   **Carried forward, not buried:** `clib.h` preprocesses in **415 ms against gcc's
   52 ms** (8×, down from 13×, and 012 §6's budget extrapolates to ~3 min of 10 for 1552
   TUs on 12 cores — a measure to keep, not a gate that passed). **011 c12** (throughput)
   and **012 c17** (configured-corpus regression) are deliberately uncovered: their only
   tests are `#[ignore]`d and 070 §195 counts that as uncovered, so the `Covers:` lines
   were moved off rather than left reporting green. **010 c19** (per-`ConfigId` expansion
   sites) was blocked *on this merge* — 001 §180 puts `ConfigId` in `chiero-pp`, above
   `chiero-span` — and is now unblocked and owed.

   Reviews preserved at `docs/reviews/m2-frontend-round-{1,2}.md` with the brief and
   notes; they were untracked in the worktree and folding it would have deleted them.
   013/014/015 are at 0/20, 0/20, 0/25 — parser, semantics and lowering are not started,
   and they are the next milestone.

   **WAVE 83** (`8474a03`; 752 tests, **150/165 cited**) — the engine now uses
   `PathCondition`, so **022 §6 is reachable from a real run**. This closes the gap wave
   82 flagged.

   `probe` called `check`, so wave 79's independence slicing was implemented, given its
   own test suite, and never executed. A bare `check` *cannot* slice — one flat list, no
   way to tell which variables the question is about, which is the whole distinction
   `check_path` exists for. `RunResult::sliced_terms_skipped` now exposes what was
   withheld; without a number in the result there is no way to tell a run that sliced from
   one that could not. Proven load-bearing by mutation (revert `probe` to `check` → the
   new test fails).

   **Every constraint the engine adds is now classified**, because §6.1's rule is about
   *how* a constraint arrived and a `Vec<Term>` cannot say. The spec lists three sites;
   the engine has six:
   - *checked* — the three ordinary branch arms; the pointer-resolution constraints, whose
     candidates came from feasibility probes.
   - *unchecked* — 023 §3's three `Unknown` arms; 024 §4's `strlen` cap guard (§6.1's own
     third example); a checker's `Action::Assume`; `chiero_assume` on a symbolic
     condition. The last two are **new sites §6.1 does not list**, recorded per its own
     `push_unchecked` instruction.

   ⚠️ **Known limit, on the field's doc comment.** §6.1's "a single full check that returns
   `Sat` clears it" is not implemented: the engine only asks feasibility questions *with*
   assumptions, which prove something other than the path condition alone, and the one
   full check left (`pinned_offset`) holds `&State`. A state downstream of one solver
   `Unknown` stays unsliced for life — the slow direction, not the wrong one. Fixing it
   means giving `pinned_offset` a `&mut State`.

   **WAVE 82** (`db7dd6f`, `ac42db6`; 751 tests, **150/165 cited**) — 024 contract 17
   (`include/chiero.h` + the corpus) and the wave-80 review applied.

   **The corpus half.** `include/chiero.h`, four files in `tests/corpus/c/`, and the tests
   for both directions of 024 §7's dual-use property. The header's `#ifdef __CHIERO__` is
   load-bearing and is pinned by compiling with `-D__CHIERO__` and asserting the **link
   fails**: defining the intrinsics unconditionally is the natural way to write the header
   and would quietly destroy the corpus, because 023 §5 says "the module's own definition
   always wins" over a registered model — chiero would analyse a no-op
   `chiero_make_symbolic`, every corpus program would run on one concrete path, and the
   suite would report success over a symbolic execution that never happened.

   Three defects, all producing that same silent-success shape:
   - **`chiero_make_symbolic` and `chiero_is_symbolic` had no model at all.** The two
     intrinsics that introduce and inspect symbolism were missing from the registry, so a
     corpus call hit the unmodeled-extern path. Now modelled and dispatched, with the
     harness's name travelling into the witness.
   - **`IntrinsicOutcome::Constrain` was a no-op**, sharing an arm with `Continue`. Ground
     conditions are decided by the `Some(true)`/`Some(false)` arms, so `Constrain` is
     reached *only* for a symbolic condition — precisely what `chiero_assume` is for.
     Every harness assumption over a symbolic value was discarded.
   - A condition is now tested against zero rather than asserted as a bit (C's "nonzero is
     true"); a wider-than-1 term negated bitwise is not its negation.

   **The review half — eight defects in the checker interface, two of them soundness:**
   - `Action::Assume` and `Action::Kill` left the run **`Exact` with no assumption**, so
     `seal` would mint a proof over a program half of whose paths an unaudited checker had
     deleted. Both now degrade to `Approximated` and name the checker.
   - **`may` collapsed `Unknown` into `false`** — the anti-conservative direction, and the
     one `matches!(.., Sat(_))` gives for free. "May this pointer be NULL?" answered *no*,
     with no finding and no fidelity change, for every question outside §3.2's fragment.
   - **`must(t)` and `must(¬t)` were both true above width 1** (bitwise `not` on a ground
     fold of `bits() != 0`).
   - `Event::Call` never fired for an **indirect** call, and **compacted its arguments**
     with `filter_map` so `args[1]` read its neighbour — the same bug the varargs path had
     already been fixed for, with a comment naming it.
   - An **errored** state fired no `Terminated`; checker solver queries were invisible in
     `solver_calls`.
   - The contract-24 fixture **named three callee kinds and contained two**.

   ⚠️ ~~**Standing gap the review surfaced, bigger than the wave.**~~ **CLOSED in wave 83.**
   At the time: `grep -rn PathCondition crates/ | grep -v chiero-solver` returned
   **nothing**. The engine's `State::path` is a
   bare `Vec<Term>`, so wave 79's independence slicing and `possibly_infeasible` are
   **unreachable from a real run** — the engine calls `check`, never `check_path`. 022
   §6.1's three unchecked-push sites are therefore unflagged, and `Action::Assume` plus
   `IntrinsicOutcome::Constrain` (this wave) make five. Wiring `State::path` to
   `PathCondition` is the single highest-value integration task left in M1, and it is what
   makes wave 79 more than a well-tested island.

   ⚠️ **Recorded deviations from 023 §6 that are still open:** `Action::Fork(Term)` is
   absent from the enum; `Action::Report` carries a `String` rather than a `Finding`, so a
   checker cannot attach a kind, object or witness; and `CheckerCtx` has **no `witness()`**,
   despite §6 resting a normative claim on it ("only through this interface, so that every
   finding is forced to come with a counterexample or explicitly declare it has none").
   `Event::Fork` is also emitted at only one of the engine's six fork sites.

   **WAVE 81** (`78f641b`; 738 tests, **149/165 cited**) — 020 contracts 12 and 22.

   **Contract 12 was implemented and merely uncited**, which makes the test worthless
   unless it pins something. Verified by mutation: loosening the shuffle bound to
   `4 * lanes`, an off-by-one lane bound, dropping `InsertLane` from the shared lane arm,
   and deleting the `Bitcast` width check each fail exactly one test. Both directions
   asserted throughout — a verifier that rejects everything satisfies the rejection half,
   which is 020's own contract-29 note read in the mirror.

   **Contract 22 found two engine defects**, both silent, both in the register half:
   - `bits_of_cty` returned `None` for `CTy::Vector` through a `_` fallthrough, so a
     `Bitcast` between two views of one 128-bit union was a **lowering gap** — not a wrong
     answer but *no* answer, degrading the run to `Unknown` for a construct the CIR fully
     specifies. Arms are now written out so a new `CTy` is a compile error here.
   - Lane width was not carried across a vector `Bitcast`, so every `ExtractLane` after
     one could not tell how the bits divide. It comes from the **destination** type, which
     is the whole point of the instruction.

   The memory and scalar halves passed from the start; only the register half was broken,
   which is why the contract asks for both. The scalar path is pinned as the oracle
   separately — two views wrong the same way agree with each other perfectly.

   📋 **Actionable survey of what is left** (recorded so the next wave does not re-derive
   it). Of the 16 remaining uncited M1 contracts, most are **blocked, not skipped**:
   - **Need the M2 frontend** (lowering from C): 020 c14, c15, c18a, c30.
   - **Need the optimisation passes** (`chiero-opt` is a stub): 020 c16 (`mem2reg`),
     c17 (`simplify_cfg`), c44 (no pass widens a bitfield access).
   - **Need 040's checkers**: 020 c18b, c29 (`union-pun` off by default).
   - **Need arenas (021 §5.2), which do not exist**: 021 c13c, c13d. This is the
     highest-value unimplemented feature in M1 — §5.2 says that without it "every VPP node
     analysis dies at its first buffer access", since `vlib_buffer_ptr_from_index` is a
     pure `IntToPtr` over an unconstrained symbol. Design note for whoever takes it: the
     element index `k = byte_off / pitch` and the within-element offset `d = byte_off %
     pitch` are **both symbolic** in the general case, so it needs either symbolic-offset
     pointers or an honest restriction — decide which before writing the fixture.
   - **Doable now**: 020 c23 (two struct views of one `opaque[10]`, `UnionMember` in the
     finding text), 023 c7 (`RandomPath` seed in `RunResult`), 023 c21 (per-witness replay
     at line granularity vs gcov), 024 c17. 023 c17 (1/2/8 worker threads identical) needs
     §11 parallelism, which does not exist.

   **WAVE 80** (`fc9414d` RED, `f9eb9c3` GREEN; 732 tests, **147/165 cited**) — 023 §6,
   **the checker interface**: `Checker`, `Event`, `Action`, `CheckerCtx`, `CheckerState`,
   `Engine::with_checker`. Contracts 19, 20, 22, 24. `chiero-check` had been an empty stub
   since the crate graph was laid out, which is why these four had never been citable.

   Events are emitted at `BeforeInst`, `AfterInst`, `Fork`, `Call`, `CallReturn`, `Return`
   and `Terminated`. **`Event` defines only those** — §6 also lists `MemFault` and
   `ArithEvent`, and they arrive with the checkers that consume them rather than sitting
   in the enum unemitted, because a checker matching a variant that can never fire is
   indistinguishable from one whose logic is wrong. Leaving them out is a compile error
   later; leaving them in is a silent gap.

   Two defects the wave's own tests found, both worth carrying:
   - **`must` on a ground condition returned `false` for a tautology.** `must(1 == 1)`
     hands tier 1 a constant, which is not an atom and so leaves §3.2's fragment; the
     answer is `Unknown` and `must` reports `false`. With the engine's **default
     tier-1-only solver** that is every ground question a checker can ask, including "did
     this path return the value I care about" — §6.1's own lock example. Now folded before
     the solver is consulted.
   - **Contract 19's obvious fixture cannot fail.** Assuming `x != 0` and branching on
     `x == 0` looks like the test, but tier 1 has no `Ne` transfer, so the conjunction is
     `Unknown`, 023 §3 takes the branch anyway, and both sides survive whether or not the
     assume reached the solver. The fixture assumes `x == 5` instead — two conflicting
     equalities, inside tier 1's fragment — so it decides the question with the solver the
     engine actually uses.

   ⚠️ **A default that makes tests lie.** `Engine::new` is tier-1-only; `with_backend` is
   opt-in. Any engine test whose expectation needs a solver answer tier 1 cannot give
   passes for the wrong reason — it sees the `Unknown`-take-both-branches path, not the
   behaviour it names. Wave 80 hit this twice in one file. When writing an engine fixture,
   check that tier 1 can decide it, or attach a backend deliberately.

   Fork semantics live in `CheckerStates::clone` rather than at the fork sites, so a
   `State` clone cannot forget `on_fork`. Deviations recorded in source: `on_fork` is
   required rather than defaulted through `dyn_clone` (not a workspace dependency).

   ⚠️ **Suspect citation found while surveying:** 023 contract 20 was already "cited" by
   `witness.rs`, but that citation is about the engine not deduplicating *one* checker's
   findings across a fork — not about two checkers on one event, which is what the
   contract says. `tests/checkers.rs` now covers it properly. Worth assuming other
   citations are similarly approximate; the coverage tool checks that a contract is
   *named*, not that it is *tested*.

   **WAVE 79** (`16542c6` RED, `2e12723` GREEN; 727 tests, **144/165 cited**) — 022
   contracts 9 and 9b: independence slicing, `PathCondition`, and §6.1. Union-find
   partition of the assertion set into variable-disjoint components, a per-slice model
   cache, `check_path`, and a `sliced_terms_skipped` stat.

   **§6.1 was amended in the same commit.** As drafted it makes the `possibly_infeasible`
   flag the only thing preventing a wrong `Sat` from a quietly-dead component, which rests
   the soundness of slicing on all three call sites having remembered `push_unchecked`.
   023 contract 16 independently demands more — a witness must satisfy the *whole* path
   condition, and a model solved from one component assigns nothing in the others — so a
   sliced `Sat` is completed from the remaining components, and a dead one is caught on
   the way. The flag still disables slicing (performance) and the subset/superset rules
   (correctness: those are stated over full assertion sets), but is **no longer
   load-bearing for soundness**. The RED negative control asserting the wrong answer was
   reachable is now `slicing_stays_sound_when_the_flag_is_wrong`, asserting it is not,
   naming the cheaper model-completion that would undo it.

   Two test defects found going green, both **the same shape as wave 78's**:
   - `x == 1 && x == 2` is refuted by tier 1's interval domain before any backend call, so
     the poisoned path condition exercised neither slicing nor the subsumption index. The
     `sliced_terms_skipped == 0` assertions are what caught it. It is now `x * 2 == 1`,
     outside §3.2's transfer set and so genuinely opaque to tier 1.
   - The flagged superset reused the previous query's exact term set, so the **exact**
     cache answered it. §6.1 does not disable that cache and should not: an exact match is
     the same question, not a subsumed one.

   ⚠️ **Method note for the next wave — this is now three for three.** Waves 78 and 79
   both shipped tests that could not reach the code they named, and in both cases the
   thing that exposed it was an *instrumentation* assertion (`sliced_terms_skipped`, the
   `Unsat`-count floor) rather than the behavioural one. A behavioural assertion alone
   cannot distinguish "the feature works" from "a cheaper path reached the same answer".
   When testing an optimisation or a guard, assert that it **ran**, not only that the
   answer is right.

   **WAVE 78** (`8741781`; 721 tests, **142/165 cited**) — 022 contract 18, the random
   differential campaign. Written, then invalidated twice by mutation before it was
   committed; both failures are the same trap in different clothing and both are worth
   carrying forward.

   - **The differential compared tier 1 with itself.** The natural harness is
     `TieredSolver::new()` versus `TieredSolver::with_backend(z3)`. It does not work:
     `check` runs tier 1 *first* and consults z3 only when tier 1 answers `Unknown`, so on
     every case where tier 1 gives a definite answer — the only cases the contract is
     about — the "backend" side returns tier 1's answer. A tier-1 defect corrupts both
     sides identically and they always agree. Mutating the narrowing rule to force
     `v <= 3` on every `v <u k`, which answers `Unsat` for `4 <u v <u 200`, produced
     **zero disagreements over 400 formulas**. §5's `paranoid` mode is the only path that
     escalates an already-decided answer to the backend; the campaign runs through it and
     catches the same mutant with 11. *(Checked: no other test uses that shape.)*
   - **The generator could not reach the rule it was testing.** With the variable always
     the left operand of a comparison, only `hi` is ever lowered; an unsigned interval
     with a floor of zero and no ceiling cannot empty, so tier 1's sole route to `Unsat`
     was a pair of conflicting equalities and the entire `Ult` narrowing rule was off the
     path to any definite answer. Both operand orders now appear, constants come from a
     small colliding pool, and `ult` is the plurality atom (50 of 400 formulas now come
     back `Unsat`, and the test asserts that count is nonzero — a campaign that only ever
     answers `Sat` is not testing anything, since `Sat` self-certifies).

   **Sensitivity is measured, not assumed**, and the two scales are not interchangeable: a
   wrong-end or grossly-tight narrowing fails at 400; a one-off-by-one (`v <u k` narrowing
   to `v <= k-2`) **survives 400 and fails at 4000** with 3 disagreements, because it only
   changes an answer when two bounds on one variable land exactly two apart. A green
   default run is not the contract being met — `CHIERO_CAMPAIGN=10000` is.

   **WAVE 77** (`6bbc598`; 714 tests, **141/165 cited**) — 021 contract 17b, plus the
   wave-74 review, which **verified the most important thing and then found five defects**.

   *Verified sound:* `fold` vs `independent_bin` vs z3 over **675 000 cases** — 20 000
   random at four widths plus every operand pair at width 8 for the ten hardest operators.
   Zero disagreements. The semantics are right; what was wrong was the apparatus.

   - **021 c17b:** a branch on pointer bits below the object's alignment explores both
     sides and says why. §7.2's point is that chiero's answer is *stable* — the bump
     allocator decides it deterministically, so the wrong answer never looks flaky.
     Detection is per-`ValueId`, because the arena folds `address & 63` to a constant and
     the structure that revealed the question is gone by branch time. Bits *within* the
     alignment are still decided normally, or every aligned-pointer test in VPP doubles.
   - **The entry-parameter fill was not affordable.** 4096 symbols per pointee are paid on
     every state *clone*: 1.3 GB at 8192 states, process abort with four pointer
     parameters, against a default `max_states` of 10 000. §6 says "on **first
     dereference**" and my comment had called laziness "the optimisation, not the
     correctness" — inverting the spec. Lazy again, properly; k=13 went 1.45 s → 0.34 s.
   - **Contract 7d's mechanical check tested the wrong function**: replacing
     `independent_bin`'s body with `fold(k, x, y)` passed all 710 tests. **The two
     implementations could collapse into one and nothing noticed** — the one scenario the
     contract exists for.
   - **The z3 test never reached the evaluator** (constants fold at construction), so it
     passed with the evaluator gutted to return zero — the same trap the sibling test
     documents, in the test written to avoid it.
   - **My "equivalent mutant" claim was false at width 128.** `wrapping_shl(128)` masks the
     *count* to zero. *Declaring a mutant equivalent is a claim, and it needs checking at
     every width the type allows.*
   - **Contract 25's `Cond` half silently passed without z3** (`!Sat` is satisfied by
     `Unknown`) — 022 contract 2 says skipped with a reason, not silently passed.

   **WAVE 78** (`fd6c463`; 719 tests) — the `printf` false positives, all six. The review
   found them by compiling the same calls with **`gcc -Wformat`**, which is the strongest
   oracle available on this side of the project and the reason the frontend's differential
   was worth so much. *A format checker that fires on correct code is one that gets turned
   off, so the false positives are worth more than the true positives beside them.*

   `%*d`/`%.*f` consume their width and precision arguments; positional `%n$` forms are
   **declined** rather than guessed at (half-understanding a format produces findings about
   chiero's parser); `%m` takes no argument; unknown conversions really do claim nothing
   now, which the code's comment already said and the code did not do; a `%s` string chiero
   cannot read is a **bound**, not a memory finding — the most common `printf` call there
   is, since entry pointees are symbolic; and an untranslatable argument is no longer
   described as "a pointer".

   ⚠️ *Method note:* two assertions could not see their own mutants. "No mismatch" was
   satisfied by a parser that misreads `%2$d` as an unknown conversion and claims nothing —
   so the **decline** is asserted, not just the silence. And `%zu` cannot show that length
   modifiers are skipped, because `z` alone reads as an unknown conversion — so `%ld` of a
   pointer is asserted to be a mismatch. **When "nothing was reported" is the expected
   answer, check *why* nothing was reported.**

   **WAVE 79** (720 tests) — the last two small items from that review. `strlen` on a
   pointer before its object reported the **same defect twice**: the concrete walk reports,
   dispatch then runs the symbolic scan over the same bytes, and it reports again in its own
   words. `ModelCtx::report_mark`/`drop_reports_after` let the thorough pass supersede the
   cheap one. ⚠️ *The model-level fixture could not see this* — it calls `strlen_symbolic`
   **directly**, bypassing the dispatch that produces the pair. **A unit test of a component
   cannot see a defect that lives in how two components are wired together.**

   The wrong witness `why` ("clobbered by opaque code" for caller-supplied bytes) went away
   with the eager fill: that string now belongs only to `havoc_range_reporting`, which the
   entry path no longer uses.

   **Still owed:** §7.2's mitigation 1 (symbolic base addresses, which would make the
   pointer-bit fork unnecessary rather than merely honest); the format checker does not
   report *too many* arguments and does not check length modifiers against argument width —
   both false negatives rather than noise.

   **M2 MERGE GATE, ROUND 2: REJECTED** (report handed over as `REVIEW-2.md` in the codex
   worktree; remediation dispatched). The reviewer's own summary of the good news is worth
   keeping: with **gcc's real predefine set**, `clib.h` preprocesses **token-for-token
   identical to gcc across 257,310 tokens**, the 010 provenance model is correct end to end
   including through headers and through `#`/`##`, and every round-1 mutation survivor is
   closed. The engine is sound; what remains are surface defects.

   **The finding that generalises past M2 — and the reason a self-report is not a gate.**
   Codex's "real VPP headers preprocess with zero diagnostics" was *literally true and
   analytically worthless*: it ran with chiero's 5-macro predefine stub, under which
   glibc's `__GNUC_PREREQ(3,3)` is 0 and `__THROW`/`__attribute__`/`__extension__` all
   vanish, so those headers take **different branches from gcc** and 13% of the token
   stream differs (257,310 vs 224,074) with nothing reported. Agreement was never tested on
   the same code. Re-running with gcc's actual 401 predefines is what exposed all three
   blockers. **This is wave 79's instrumentation rule in another costume**: a green
   behavioural signal that cannot distinguish "it works" from "the test never reached it".

   Three blockers, each corrupting the token stream for ordinary C with **no diagnostic**:
   - the macro-expansion depth cap counts *sequential* expansions, because `expand_inner`
     is tail-recursive over the rest of the stream — the 257th macro in any directive-free
     region silently stops expanding (`ip4_forward.c` emits it 9 times), and a test
     *asserts* the bogus diagnostic;
   - `#include <…>` macro-expands its operand, which C11 §6.10.2p4 forbids when the
     directive matches a header-name form — gnu-mode predefines `linux`/`unix` as `1`, and
     VPP has 307 angle includes with those as path components;
   - substituted argument tokens keep their *call-site* `leading_space` instead of the
     parameter's, corrupting `#` — 34 of the 36 divergent hunks across 57 vppinfra headers.

   Judged before dispatch rather than forwarded: F12 and F13 are checkable directly against
   C11 §6.10.2p4 and §6.10.3.2p2 and both hold; F11 is code reasoning with a reproducer and
   measured VPP fallout. Also owed: the dead cross-TU header cache (0 hits / 4 misses; its
   test passes only because it preprocesses the same path twice), two `SIGABRT` paths
   `catch_unwind` cannot contain, no argument-count checking, no `intmax_t`→`uintmax_t`
   promotion, `#pragma` dropped entirely (375 in `clib.h`, including the 113 `GCC target`
   that 060's multiarch depends on), and **no test using `__VA_ARGS__` at all** for a spec
   citing 230 uses in VPP.

   ⚠️ **`cargo xtask contract-coverage` only measures 020–024**, so every `Covers:` line in
   011/012 is unverified prose — which is how `contracts.rs` came to cite 011 c12 while its
   only test is `#[ignore]`d, contradicting `M2-NOTES.md` in the same branch. Widening the
   gate is the mechanical fix.

   *(superseded)* Codex reports the REVIEW-1 remediation complete —
   all 20 differential cases matching gcc *and* clang, real `vppinfra` headers preprocessing
   with zero diagnostics, multi-line macro calls and rescanning and `#if` and real includes
   fixed, `__VA_OPT__` diagnosing per the new spec text, and ignored tests no longer
   claiming coverage (011 c12 and 012 c17 explicitly owed instead, because their
   environments are unavailable here). **That is a self-report**, and the gate is a second
   review: round 2 is re-running round 1's own evidence rather than accepting the fixes,
   checking that "zero diagnostics" on VPP headers also means *the same tokens as gcc*, and
   verifying that `#`/`##` tokens no longer carry a fabricated `expansion_loc` of 1:1 —
   010 §3.1 says of that field, "THIS IS WHAT GCOV SEES".

   **STILL OWED from wave 51, in the reviewer's priority order:**
   - ~~**D3**~~ DONE (wave 52). Steps 4 and 5 merged whenever live objects exceeded the cap: `over_cap` returns
     *before* step 4's test, so with `max_resolutions = 8` and ≥9 objects an unconstrained
     pointer gets `Approximated` and silently continues on object 1. **Under VPP's >10⁴
     objects, step 4 can never fire** — §5.1's highest-value guarantee unreachable exactly
     where it was written for. Verbatim the failure 021 records against its own draft.
   - ~~**D6**~~ DONE (wave 53). `live_ranges` filters to `ObjState::Live`, so a use-after-free through a
     symbolic address reports "unconstrained pointer" and terminates instead of reporting
     the UAF. 021 §4 keeps freed objects *precisely so* the site can be named.
   - ~~**D5**~~ DONE (wave 52). `candidates.is_empty()` is the *opposite* of "every object feasible": an address
     provably in a guard gap is reported as "wholly unconstrained". Third instance of the
     cause-conflation `f29457b` fixed, one branch over.
   - ~~**D8**~~ DONE (wave 53). The search **is** the per-dereference O(objects) solver sweep §5.1 forbids —
     `solver_calls` measured at exactly `n + 3`. The comment claims an arithmetic pre-filter
     that does not exist.
   - ~~**D4**~~ DONE (wave 53). One-past-the-end becomes an in-bounds write at byte 0 (same root as D1).
   - Mutation gaps: ~~**L2/L3**~~, ~~**M14**~~, ~~**G1**~~ DONE (wave 53). **M8/M9** step 4's
     disjunct remain — the disjunct itself is gone with the sweep, so what M8/M9 asked for
     has to be re-read against the model-driven search before it means anything.

   *(earlier §5.1 note, now superseded by wave 53)* step 4's own detection is reached, at
   tier 1 and tier 2 both, and asserted by name in three tests.
   Contracts 17b, 18 and 19 need `PointerBitInspection`, lazy materialization and
   `--fork-on-alias`, none of which exist. Contract 17 (`max_resolutions = 2`
   concretizes at `Approximated`) needs the same fixture with a smaller cap. Contracts 17b,
   18, 19 need `PointerBitInspection`, lazy materialization and `--fork-on-alias`, none of
   which exist.

   *(scoping note, now superseded)* Contracts
   16, 17, 17b, 18, 19 all depend on it. The spec's five steps, in order: provenance or a
   registered arena short-circuits the search; otherwise ask the solver which objects the
   value can fall in, capped at `max_resolutions` (8); one → continue; several → **fork per
   object plus one wild state**; wholly unconstrained → `Unknown` + `UnresolvablePointer`
   and the path **stops**; merely over the cap → concretize to the model + `Approximated`.
   ⚠️ **Steps 4 and 5 must stay distinct** — 021 says an earlier draft merged them, so an
   unconstrained pointer was concretized to an arbitrary object and reported `Bounded`,
   which reads as "we looked and bounded it" when nothing was known.

   Two accessors are missing before contract 25 (promotion preserves initialization) can be
   tested honestly: after promotion the byte API refuses the object, so comparing the three
   init states before and after needs an array-aware way to ask "is this bit `No`, `Cond`
   or `Yes`". I started that test and backed it out rather than assert something weaker
   than the contract.

   Contracts 30 and 39 need **scope markers**: `InstKind::Marker(_)` is a no-op, so
   `Scope(Exit)` retires nothing and `Lifetime::Function` versus `Lifetime::Scope` is
   currently indistinguishable.

   **STILL OWED from wave 41:** E5 — every `OpaqueWrite` fixture has exactly one entry, so
   "each declared write is honoured" is untested. Plus the suspicions the review left open:
   a faulting `LoadBits` invents an unconstrained `w`-bit symbol where the field's range is
   narrower (sound, over-approximate, but explores unreachable branches); 020 contract 32's
   execution half is untested while a parser test cites it; and `022 contract 17` /
   `024 contract 21` are cited only by module-header "Covers" lists.

   *(earlier list, now resolved)*
   - **D2** `havoc_range` clobbering only the **first** byte survives the suite — the test
     pins the upper bound only. Also `concrete_size → 1` survives.
   - **D3** `HavocFill::Uninitialized` in `havoc_range` is unreachable in-tree *and* wrong
     if reached: it mutates read-only, freed and promoted objects, and on a promoted one it
     reports success while changing nothing — what 020 §4.3 forbids.
   - **D4** an `Opaque` write past the end partially clobbers, discards the `OutOfBounds`
     fault (so a declared overflow is detected and **not reported**), returns `false` after
     mutating, and its message says "outside any object" for a range that started inside.
   - **D8** contract 27 is pinned at the `LoadBits` site only; deleting `check_bits` at the
     `StoreBits` site survives, and `> w` → `>= w` survives (a full-width bitfield is
     correct and unpinned).
   - **D9** `verify.rs:822` overflows on `BitRange { off: u32::MAX, width: 4 }` — panic in
     debug, *accepts* in release. Reachable only from a programmatically built module, i.e.
     `chiero-lower`.
   - **D10** contract 9's "solver agrees with the engine" assertion is **structurally
     vacuous**: both operands are constants, so `bin` const-folds and `return_value_bits`
     *is* the same `eval_ground` call. Proven by mutation — deleting the assertion loses
     nothing. Needs symbolic operands to be real.
   - **E5** every `OpaqueWrite` fixture has exactly one entry, so "each declared write is
     honoured" is untested.

   **M1's instruction set is complete**, but M1's *exit* is not — **752 tests, 150/165
   contracts cited** (`cargo xtask contract-coverage`); the remaining 55 contracts are the real M1 backlog, and 080 also requires the z3
   `paranoid` cross-check over the corpus, the fidelity `trybuild` test, and an OOB finding
   **with a witness** (`Witness` does not exist yet). Still owed on the engine: `Store`/`Load` ignore
   the CIR's `align`, which is what a real `ub-strict` mode would need — and note it has
   **no observable effect today**, because `report_faults` filters `Misaligned` out
   entirely, so the honest order is `ub-strict` first and `align` with it. Then **M2 onward
   in 080 — the frontend — which has not started.**

   *(superseded list)* the four `Va*` (010 measured
   2552 `va_list *` in VPP, so this is not exotic); `Shuffle`/`InsertLane`/`ExtractLane`/
   `Splat`; and `PtrToInt`/`IntToPtr` casts, which land in the gap because a pointer is
   not a scalar operand.

   **STILL OWED from wave 16:** the engine's use of `reachable_depth` is unpinned (needs a
   stored pointer — now possible, since `Store` exists); a store through NULL produces a
   finding and **execution continues**, where the program would have crashed; `HavocSpec`'s
   `ranges` field is still unused.

   **STILL OWED from wave 15's review:** the
   symbolic sizes degrade though 024 §3 permits them; `AllocPolicy` is engine-global where
   024 §3 wants it per allocator; `State::findings()` has no direct test; `HavocSpec`'s
   `ranges` field is unused and `ModelOutcome::Havoc` is still a gap in `dispatch`
   (contract 21c is only reachable through the *unmodeled* path today).

   **Standing note on mutation testing** (three instances this session): a mutation that
   **does not compile** reports as "no failing tests" and is indistinguishable from an
   unpinned fix. Deleting an arm from an exhaustive match is a *type error*, not a
   behaviour change — rewrite the arm as a no-op instead. Always check what the mutation
   actually did before believing a survivor.
   - ~~**The round-trip fixture supplies the identity/default value for nearly every
     scalar field**~~ DONE (`afd29da`, 178 tests): `tests/roundtrip_property.rs` generates
     random modules with a distinct non-default value in every field and asserts
     *structural* equality, plus a guard test on the generator itself. It found a missed
     `AllocaDecl::span` in the span commit on its first run. **One correction to the
     finding:** a *symmetric* inversion (printer and parser both transposing a pair)
     round-trips by construction and **no** round-trip test can see it — transposing
     `BitRange` is caught only because the verifier rejects the result. Encoding symmetry
     needs an independent oracle. Original finding text:, so printer and parser can drop or invert a field *in lockstep* and the
     byte-exact round trip still passes. `vacopy`'s src/dst can be swapped on both sides
     and nothing notices. The variant-coverage guard measures variant *reachability*, not
     field *fidelity*. A second fixture with distinct non-default values everywhere, or a
     random-module property test asserting `parse(print(m)) == m`, is the single
     highest-leverage test change available.
   - 020 §5 says the verifier *sorts* switch cases; it takes `&Module` so it structurally
     cannot. Either add `canonicalize(&mut Module)` or amend the spec. Contract 13 is
     satisfied trivially today, and `successors()` order is pinned only by a
     single-element list, where `rev()` is the identity.

   **Design judgements worth acting on eventually:** `AllocaDecl::count` uses `u64::MAX`
   as an in-band sentinel where `enum Extent { Static(u64), Dynamic }` costs nothing;
   `Block::id` duplicates its index (now that `IdNotIndex` exists for funcs/globals, the
   same argument applies); `VerifyError.detail` is formatted eagerly for every warning.

   **Wave 6 — `chiero-cir` re-review COMPLETE, criticals applied in `87a1b2a`.**
   151 mutations, **54 survived (36%**, down from 45%) — but the escapes clustered in the
   code the previous round *added*, which is precisely the value of re-reviewing fixes.
   The reviewer re-broke every earlier fix and confirmed all hold.

   Two wrong-answer bugs fixed: named allocas were unreferenceable and
   `store … -> %slot` minted an undefined value (so 020 §6's example parsed into a module
   that **did not verify**, invisible because the test never called `verify`); and
   `global const @x` was invisible to `scan_names`, desyncing id spaces so `addrglobal`
   silently resolved to the *wrong* global.

   **Wave-6 debt: 4 of 9 discharged** in `e60d05c` and `cc3d64c` — the tautological
   variant guard, `Function::entry` round-tripping (and its vacuous test), named/numeric
   id collision, and undefined-label aliasing. **Each fix was mutation-tested before
   being claimed**, which caught that two of them were initially unpinned: reverting
   `name_base` and the undefined-label check still passed the whole suite because no test
   exercised either.

   **Wave-6 debt: ALL 9 DISCHARGED** (`e60d05c`, `cc3d64c`, `1855b7a`, `ab01822`,
   `bdc7296`). The tautological variant guard; `Function::entry` round-tripping and its
   vacuous test; named/numeric id collision; undefined-label aliasing; module-level
   verification (duplicate `GlobalId`/`FuncId`, duplicate global and function *names*,
   dangling `Callee::Direct`/`AddrOfFunc`/`AddrOfGlobal`, call arity with variadic
   handling, rule 7 for globals); `successors()` including the switch default;
   constants in every operand position; and `gcov_lines` order.

   Constants are now **single-token** (`undef:i64`, `globaladdr:@g:8`,
   `wide:i256:0x…`) — a space-separated form cannot appear in operand position at all,
   because the tokenizer splits before the operand parser runs. Same constraint that
   made vectors print `<4xi32>`.

   **Method note, now standing:** *mutation-test every fix before claiming it.* Revert
   the fix on a scratch copy and confirm a test fails. Twice this round a fix was correct
   but unpinned, and six edits this session silently no-opped because `cargo fmt` had
   reformatted the anchor text between reading and patching — one of which a compile
   error did **not** catch, because the old code was still valid. Prefer line-range
   replacement with an assertion on the anchor, and always verify the edit landed.

   **Wave 7 (QUEUED):**
   - **A third `chiero-cir` pass.** Two passes ran 45% then 36% escape rates, and every
     finding is now fixed with each fix mutation-verified. A third pass is the test of
     whether the rate is actually falling or whether the reviews were only finding what
     they were pointed at.
   - **030+031+032** — the coverage→impact→selection chain. Brief: hunt *recall holes*
     (a missed test is a shipped regression). Specific leads worth chasing: can a change
     be misclassified `Cosmetic` when line position is semantically observable
     (`__LINE__`, `__FILE__`, `assert` text — grep VPP for `__LINE__`)? constructor/
     destructor attributes (51 VPP files) run before `main`. Conditional-compilation
     branches not taken in the analyzed config. Multiarch 1:N holes. Also: how many of
     032's 21 contracts does a trivial "always select every test" selector pass?
   - **040+041+042+050** — findings, equivalence, recipes, tool surface. Note the two
     leads originally listed here were *already found and fixed* by the Fable pass
     (static-function replay → 040 §3.1; the allocation-address `Differs` bug → 041 §1.1
     object bijection), so brief this agent on what remains: 042's DSL expressiveness
     (try writing 2–3 of the promised VPP rules in it), whether tier-1 candidate filtering
     can drop a function tier 2 would have flagged (a recall hole), and 050's truncation /
     cancelled-job paths as overclaim vectors.
   - **A second Fable pass over the *revised* specs.** The first pass found the most, and
     the specs changed substantially under it; the fixes themselves deserve adversarial
     review, especially 021 §3.1's tri-state `InitMask`, 021 §5.2 arenas, 042 §4.2.1's
     `(ObjectId, byte range)` entity identity, and **015, which is brand new and has
     never been reviewed at all**.
   - **`chiero-cir`** — types, verifier and the textual format are real code (37 tests)
     with no review yet. Brief it to re-break each claimed property, as wave 4 did. Ask
     specifically: does a wrong verifier pass? is `assert_rejects`'s
     one-defect-one-kind rule circumventable? does the printer/parser lose information
     on any construct not in the round-trip fixture? **`tests/corpus/cir/` is empty, so
     020 contracts 1 and 5 — which quantify over "every module in the corpus" — pass
     vacuously today.** That is the sixth vacuity and it is already known; the corpus is
     owed.
   - **`chiero-solver`** — `solver-lite` now exists (20 tests). Brief with the mutation
     method and point it at 022 §3's soundness rules: can a mutation make `Unsat`
     reachable outside the `as_atom` fragment? does contract 7b's enumeration actually
     bite? is the wrap-safety claim pinned, or only the one verified case?
   - **A third `chiero-span` pass** only if the others stop finding things; the trend is
     still strongly positive.

3. **Apply the findings as `spec:` commits** before any implementation. Judge them — a
   subagent finding is a claim, not a verdict; several will be wrong, and adopting a
   wrong one damages a spec that is currently correct. Where a finding is right, amend
   the spec; where it is wrong, say so and move on.

3.5. **M0 IS DONE** (commits `04d3b90` red, `dbc68e2` green). Workspace of 21 crates per
   001 §6, `xtask check-deps` enforcing the six *graph-decidable* 001 §4 rules (1,2,3,5,6,7)
   with synthetic violating fixtures, plus `xtask check-vpp-leak` for rule 4, which is a
   property of source text and cannot be decided from the dependency graph, CI running fmt + clippy-deny-warnings + `--no-default-features` + the
   dependency gate + tests. `clippy.toml` denies `HashMap`/`HashSet` workspace-wide since
   001 §5 makes determinism a hard requirement.
   *Process note*: I wrote the checker before its test, then backed it out to a stub to
   get a genuine RED (8 failures, all value mismatches). Don't repeat that — red first.

   **The dep-gate adversarial review (`e16cd3a`) found four real defects. Its lesson
   generalizes and should shape every test written from here on:**
   - *Assert the exact set, not membership.* A shotgun implementation tagging every
     violation with all seven rule names passed the whole original suite.
   - *A test over real data must assert the data is non-trivial.* `the_real_workspace_is_clean`
     passed on an empty graph, one `cargo metadata` schema change away from a silent
     no-op gate. **I independently hit the identical bug in `chiero-span`'s
     `expansion_loc_does_not_allocate`, which measured a counter nothing incremented.**
     Two instances in one session — when a test asserts "X does not happen", first prove
     the test can observe X happening.
   - *Test the thing the contract names.* Contract 8 is about an exit code; nothing ran
     the binary.
   - *Don't claim coverage you don't have.* Rule 4 was "enforced" in three places and
     nowhere. Gates now state which rules they cover; `check-vpp-leak` covers rule 4.

3.6. **M1 IN PROGRESS — `chiero-span` is largely done.** 43 tests green, both gates
   green, clippy clean.
   - core types (`890b042` red, `5aae0bd` green): `BytePos`, `ExpnCtx` (ROOT == 0),
     `Span` (12 bytes, `Copy`, half-open, `DUMMY`). 010 contracts 1–2.
   - `SourceFile`/`SourceMap` (`2c3520c`, `1069534`): global `BytePos` space,
     binary-search lookup verified against a linear scan at *every* position across 100
     files, 1-based line/col counting bytes, CRLF and no-trailing-newline handled.
     010 contract 12.
   - provenance (`01a49d4`, `bafb02a`): `Expansion`, `MacroInfo`, and the §3.1 queries.
     The `vec_add1` fixture from 010 §3.2 is built for real with correct line numbers.
     010 contracts 3–10. `expansion_loc` allocates zero times, measured with a real
     counting allocator.

   - cooked cross-TU index (`0e101e9`, `b184c89`): `GlobalInterner`,
     `CookedExpansionIndex`, `MacroEntity`, `GlobalFileId`. 010 contracts 13–19 — the
     ones that catch the dangling-`ExpnCtx` design error. `cook_tu` resolves eagerly so
     the index is self-contained by construction.

3.7. **`chiero-cir` STARTED** (`c0ae64b` red, `f27a3e9` green). All CIR types per 020
   (`CTy`, `Const` incl. `Wide` for >128-bit, `RValue` incl. vector lane ops, `InstKind`
   incl. `AllocaDyn`/`VaStart`/`VaArg`, `Terminator`, `Block`, `Function`, `Module`) plus
   the **verifier**: all 13 rules of 020 §8, 21 tests, 020 contracts 4–5.
   `Symbol = Arc<str>` for now — an interner belongs in `chiero-span` once a second
   crate needs one.

   **Textual `.cir` format DONE** (`ed111fd` red, `38c7b81` green): printer + parser,
   16 tests, 020 contracts 1–3. Uses **names** (`@counts`) not numeric ids, per 020 §6's
   example — which required threading the module through printing and a pre-pass over
   the source for forward references. Printing *is* canonicalization (no separate
   normalizer). Missing terminators are tracked with an explicit flag rather than
   defaulting to `Unreachable`, which would silently accept a truncated block that then
   verifies clean.

   **`.cir` corpus DONE** (`95291f7` red, `3d7c594` green). Seven fixtures under
   `tests/corpus/cir/`, five tests, 020 contracts 1/2/5 no longer vacuous. Two guards:
   a count guard and a **coverage guard naming the constructs the engine will be built
   against**, so "every module in the corpus" is a strong quantifier and not merely a
   large one. Verified the guard bites by deleting the fixtures on a scratch copy.

   **The corpus found a verifier bug on its first run**: parameters did not dominate
   instruction 0 of the entry block (both at position 0, strict `<`), so the commonest
   shape in real CIR was reported as an undominated use. It survived 21 verifier tests
   because every one was hand-built around constants. *Write fixtures the way lowering
   will emit CIR, not the way a unit test is convenient.*

   **Next in `chiero-cir`:** the optional passes (020 §9) with their
   observational-transparency requirement; `Opaque` (declared in the spec with `dsts`,
   but **not yet in the `InstKind` enum at all** — inline asm cannot be represented);
   `GlobalInit` and `Linkage` in the text format; and `Marker::Line` at instruction
   position, which reparses into `gcov_lines` and drops the instruction.
   **`chiero-solver`: `solver-lite` DONE** (`8bb010f` red, `0220059` green). The
   `Solver` trait, three-valued `CheckResult`, and tier 1's interval + known-bits
   product domain — 20 tests. 022 contracts 3, 4, 5, 6, 7, 7b, 7c, 16.

   The asymmetry is enforced structurally: `Unsat` is reachable only through `as_atom`
   (a conjunction of comparison atoms; anything else is `Unknown`), and `Sat` only after
   the model is evaluated against every assertion. Transfers are wrap-safe — saturating
   would report the verified-satisfiable `x>250 ∧ y=x+10 ∧ y<10` as `Unsat`. Signed
   comparison is deliberately unmodeled: incompleteness is fine, treating it as unsigned
   is not. **Contract 7b** (3000 random sets, exhaustive 256-assignment enumeration of
   every `Unsat`, plus a guard that ≥50 `Unsat`s were seen) is what makes the `Unsat`
   half trustworthy without z3. Three cases were also cross-checked against z3 directly.

   **SMT-LIB2 backend + `TieredSolver` DONE** (`2649a75` red, `ced2f93` green): runtime
   discovery, SMT-LIB2 serialization, model parsing, escalation on `Unknown`, `paranoid`
   cross-check (200 random queries, zero disagreements against real z3), and the exact
   cache keyed on the **pair** of assertion and assumption ids. 28 solver tests.
   **Verified both ways** — with z3 present, and under `env -i PATH=/empty`, where the
   four tier-2 tests print why they skipped rather than reporting success (contract 2).

   Two things worth carrying forward. Tier 2's model is validated by our own evaluator,
   so a bad external answer becomes `Unknown(BackendError)` rather than being trusted for
   being external. And the cache stores the **model** alongside the verdict — caching
   only the verdict meant a cached `Sat` re-derived its model via the backend, so the
   cache existed and saved nothing; the contract-20 backend-call counter caught it.

   **Long-lived session DONE** (`5ca6db4` red, `7774b6c` green): one process per run,
   incremental `push`/`pop`, output framed by paren balance (a live process never closes
   stdout), per-session variable declarations, and restart-and-replay on death —
   022 contract 14. `backend_spawns` is a tracked stat so a regression to per-query
   spawning is visible immediately.

   *Two lessons from that work, both found by mutation and not by passing tests:* the
   replay test initially used constraints **tier 1 decides**, so the backend was never
   consulted after the kill and the restart path was never exercised; and
   `kill_backend_for_test` initially dropped the *session*, which only tests
   "spawn when none exists" rather than the real failure of a process dying mid-query.

   **`Concat` and `Ite` ADDED** (`ba642f3` red, `cb8f368` green, 258 tests) — prerequisites
   for 021 §3's symbolic bytes and §3.1's conditional writes. Both fold at construction.

   *They exposed a real pre-existing defect unrelated to either:* the arena gives
   predicates width 1, but `(= x y)` is an SMT-LIB **`Bool`** and `#b1` is a one-bit
   vector. So `or` over two comparisons emitted `(bvor (bvult …) (bvult …))` — a sort
   error the backend rejects — **reachable from any query with a disjunction of
   comparisons**. Unnoticed because 022 contract 7c's disjunction is answered `Unknown` by
   tier 1 and never reaches translation. `smt_is_bool` now decides emission; `not` had the
   same bug. Both directions are tested, because declaring *everything* `Bool` also passed
   the suite.

   **STILL OWED in `chiero-solver`:** independence slicing and the counterexample cache (022 §6.2) with the
   `possibly_infeasible` guard, `--dump-queries`, and the contract-18 differential
   campaign over random terms.

   *(superseded)* the SMT-LIB2 subprocess backend (022 §4) — a long-lived
   `z3 -in -smt2` process with `push`/`pop` replay after a watchdog kill; `TieredSolver`
   with escalation on `Unknown` and the `paranoid` cross-check; and the three caches of
   §6.2 with the `possibly_infeasible` guard on slicing. Note 022 contract 2 requires the
   suite to run with z3 **absent** — tier-2 tests skip with a printed reason rather than
   silently passing.

   *(earlier)* **`chiero-solver` STARTED** (`33b9136` red, `de3f33a` green): `Sort`, `BvConst`,
   hash-consed `TermArena` with folding at construction, and the independent evaluator
   with SMT-LIB semantics — including the four non-uniform division-by-zero cases
   (022 contracts 19a–19d). 11 tests. **Next: `solver-lite`** — and the one with the
   heaviest validation burden (022 §7): `solver-lite`'s `Unsat` may only come from the
   §3.2 fragment, every `Sat` needs an independently-evaluated model, and contract 7b's
   exhaustive small-width enumeration is the check that closes the asymmetry without
   needing z3.

   **Still owed in `chiero-span`**: `Diagnostic` (010 §7) with macro-backtrace rendering,
   which 001 §5 says lives here; contract 11 (re-lex round trip, needs a lexer);
   contract 18 (peak-memory bound, needs a large fixture); contract 19 (per-`ConfigId`
   sites, needs `ConfigId`); `CookedSite`'s `func`/`config` fields from 010 §6.2; the
   `arg_spans: SmallVec<[Span; 4]>` of 010 §6.3 (`smallvec` is a declared dependency
   used nowhere); and contract 16's "checked mechanically" half — a comment claims an
   `xtask` grep for per-TU ids that does not exist.

   ⚠️ **Three times this session I wrote an instrument that could not observe what its
   assertion claimed** — the vacuous `alloc_count`, the empty-graph workspace test (found
   by review), and a process-global allocation counter under parallel tests (found only
   because unrelated tests happened to run beside it). Before writing any "X does not
   happen" assertion, **prove the instrument can see X happen** — the allocation test now
   does exactly that with a probe that fails if the allocator is not installed.

   Working rhythm that is going well: write the test file first, stub the impl with
   `todo!()` so failures are behavioural not missing-symbol, commit `red:` with the
   observed failure output pasted in, implement, commit `green:` with the non-obvious
   decisions recorded. Keep `cargo fmt --all` + `cargo clippy --all-targets` clean before
   each commit; CI denies warnings.

4. **Continue the TDD loop at **M1** in
   [080](docs/specs/080-roadmap.md) — the symbolic core against **hand-written `.cir`**,
   no C parsed. M1 and M2 are deliberately parallelizable (12 cores); that independence
   is the whole reason for the CIR contract boundary.

5. Refresh context via `mcp__tttt__tttt_clear_and_read_handoff_md` at milestone
   boundaries. **Update §7/§9 and commit this file first, every time.**

~~Re-verify clang/z3~~ — done, both verified working (§3). `070`'s oracle section can
assume gcc 13.3 + clang 18.1.3 + z3 4.8.12 are all present.

## 10. Standing reminders

- Don't re-ask the three §2 decisions.
- Don't clone VPP; it's at `/home/ubuntu/vpp`.
- Don't design anything that links clang or z3 at build time.
- ~~Don't start implementing before the user's spec-gate approval.~~ **Approved
  2026-07-27, full autonomy granted.** Build.
- Update §7 and §9 of this file before every context refresh, and commit it.
