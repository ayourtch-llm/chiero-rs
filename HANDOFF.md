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

   **Next piece of 021, scoped but not started: §5.1 symbolic base pointers.** Contracts
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

   **M1's instruction set is complete**, but M1's *exit* is not — see the coverage numbers
   above; the remaining 79 contracts are the real M1 backlog, and 080 also requires the z3
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
