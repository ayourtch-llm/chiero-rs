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

   **Judged valid but not yet applied** (in rough value order):
   - **Spans do not round-trip.** Nothing prints or parses them; every `span` becomes
     `Span::DUMMY`. Contracts 1 and 015/22 hold *only* because every fixture has dummy
     spans, and `every_corpus_module_round_trips` compares `print(m)` to
     `print(parse(print(m)))`, which is invariant under anything the printer omits.
     020 §1.5 calls provenance "the product". **This breaks the moment `chiero-lower`
     emits real CIR, and it breaks silently.** Highest-value remaining item.
   - **Unordered float predicates are absent from `CmpOp`.** C's `isnan` idiom `x != x`
     needs one; `FONe` is *ordered* not-equal, which is **false** for NaN — the opposite
     of C. This is a wrong-answer bug, not a coverage gap.
   - `InstKind::Opaque` still absent (015 §7 forbids the workarounds); also missing:
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
   - Rule 1 is applied at 12 operand positions but *tested* at 2. Contract 40 is written
     specifically about `AllocaDyn::count` and nothing exercises it. Same shape for rules
     5/6/7/11, each pinned at one call site. `va_list` operands are never pointer-checked
     at all. One table-driven test per operand position kills ~25 survivors.
   - **The round-trip fixture supplies the identity/default value for nearly every scalar
     field**, so printer and parser can drop or invert a field *in lockstep* and the
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
