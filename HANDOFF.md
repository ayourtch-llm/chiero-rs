# chiero-rs — HANDOFF

> Working state for a fresh context. Read this top to bottom, then continue at
> **§9 Next actions**. Everything below is decided, not open for re-litigation
> unless the user says so.
>
> **Map, if you are short of context:** §9 is the queue and the only thing that changes often —
> start there and at **§8.3**, which is the standing job. §1–§3 are the ask, the three locked
> decisions and the verified environment. §4 is a design digest that has **drifted** — read
> `docs/specs/`, which is normative. §7 is the curated defect record, §8 the operating protocol,
> §10 the standing reminders, **§11 the method lessons** that each cost a wave to learn.
>
> 🧹 **Cut from 16169 lines to ~1670 on 2026-08-07** at the owner's request. What went was §9's
> chronological wave log — ~50 superseded "where things stand" entries whose durable content was
> already promoted into §7/§8/§10, plus §11 which was harvested from it during the cut. The full
> pre-cleanup file is **`git show c94051f:HANDOFF.md`**. Keep it this size: when a wave ends,
> *replace* §9's state rather than appending to it.

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
- Scratchpad: whatever the session's own is — the path changes per session, so take it from
  the harness rather than from this line.
- **Remote: `git@github.com:ayourtch-llm/chiero-rs.git`**, first pushed 2026-08-06. The push
  credential is a deploy key in `.deploy/` (gitignored, `700`): `./.deploy/push.sh` adds it to
  an agent scoped to the script and pushes the current branch, `--check` only authenticates.
  Nothing in `.deploy/` is committed — key, script and README are properties of this machine.
- **Licence: MIT OR Apache-2.0**, both texts at the repository root, every one of the 23
  packages inheriting the SPDX field. Left for the first publish: the texts are not inside each
  crate's tarball and no manifest has a `description`.

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

### 7.1 Implementation status — what is *built*, not what is written
>
| spec | state | evidence |
|---|---|---|
| 010–024 frontend | ✅ | **1871 VPP TUs lower, 0 not-run**; the 3 `diagnosed` are VPP's own ISO C divergences |
| **030 coverage** | ✅ 19/19 contracts | full VPP, gcc: **1895/1895 `.gcno`, 322/322 objects, 0 of 156991 lines differ**. clang: **1872/1872** |
| **031 change impact** | ✅ 20/20 contracts | incl. the headline — a header macro edit impacts every expansion site while coverage sees nothing |
| **032 test selection** | 🟡 18/20 | mutation gate: **recall 100%, coverage-only 14.3%, reduction 65%** |
| **041 `prove_equivalent`** + §2/§3 | 🟡 contracts 1–6, 9–18, 21, 22, 24 | z3 proves `x*2 == x<<1` over all 2^32; finds `INT_MIN` as the one input two `abs()`s disagree on — and **gcc confirms it**, via `chiero-replay` |
| **050 tool interface** | 🟡 10 operations + a CLI (contracts 1–3, 4b, 5–8, 11, 12 partly, 14) | envelope + `select_tests`, `expansion_sites`, `explain_macro_expansion`, `prove_equivalent`(+replay), `impact`, `find_bugs`, `check_reachable`, `find_optimizations`, `layout`; all reachable as `chiero <op>`. **No MCP/JSON-RPC server**, so contract 18 cannot run |
| 040 checkers, 042 recipes, 060 vpp | partial | `chiero-check` runs **2** checkers by default; `chiero-recipe`, `chiero-vpp` exist |
| **`layout` on real VPP** | ✅ fixed 2026-08-07 | anonymous members counted, partial field lists refuse a number, and each hole names the fields it sits between — §7.7 |
| **CI** | ✅ both solver legs gate | `solver: [none, z3]`; `check-proof-surface` moved from prose into the workflow |
| **`find-bugs` on real VPP** | 🟡 measured 2026-08-06 | pinned 40: **231 → 21 findings**, `--entry-ptr-nonnull` **1**. Plugins: **477 entries over 92 plugins → 18 findings, 1 `Exact` and it is true**; two engine panics found and fixed. §7.6 |

**The two 032 contracts left, and why neither is "just work":**

- **7, reachability refinement** — *proven* not to fire at line granularity (a test the line index
  selects has a non-zero count for the line, so some block carrying it was entered). Needs a
  line→block bridge on the change side. Both halves are built: `line_reached` and `changed_lines`.
- **18, historical replay** — §6's ground-truth oracle, "the one that would catch a real design
  flaw". **Unblocked and VERIFIED 2026-08-05.** The user corrected the premise and it holds:
  `make test TEST=test_vlib` in `/home/ubuntu/vpp` builds and runs with **no root and no
  network namespaces** — 6 scheduled, 6 executed, 1 passed, 5 skipped, "Test run was
  successful". Every note below saying otherwise is wrong and is kept only so the correction
  is legible.

  > One stumble worth recording: `make build` refuses on `$(BR)/.deps.ok` because apt reports
  > `openssl` one patch behind. It is a freshness check, not a missing dependency, and
  > `touch build-root/.deps.ok` (a gitignored artifact directory) walks past it. The toolchain
  > has built this tree many times.

  **The harness is built** — `xtask/src/replay_gate.rs`, `cargo run -p xtask -- replay-gate`,
  corpus at `tests/corpus/replay/corpus.tsv`. What remains is *populating* it.

  **First candidate probed and rejected, 2026-08-05.** `1d0e0e825 pvti: fix adjacent packet
  overwrite with very big packets` against `test_pvti`, whose
  `test_0003_pvti_send_simple_1pkt_big` exists at the parent and is exactly the right shape.
  It **passes** at the parent — the suite does not exercise the overwrite. Two builds and
  ~40 minutes to find that out, which is precisely what `observed` is for: reading the commit
  message would have produced a confident wrong entry.

  **Use the better method next time.** Hunting for a commit an existing test happens to catch
  is a poor use of 40 minutes an attempt. Instead **revert a historical fix's `src/` diff on
  top of HEAD and run the suite** — whatever fails is ground truth, observed rather than
  guessed, on the build that already exists. That is the mutation gate's methodology applied
  to a change somebody else made for their own reasons, which is the whole difference §6 draws
  between the two harnesses. The entry still records the original commit, and the diff replayed
  for selection is still `commit^..commit`, so nothing the gate measures changes.
  ~~`$SCRATCH/replay-probe.sh` does the two-checkout version~~ — **that script is lost** (§9.2);
  it restored the user's tree on every exit path, and a rebuild must do the same.
  The manifest records `observed` vs `asserted` and
  **only `observed` counts towards recall**: a ground-truth oracle computed over beliefs
  measures the beliefs, which is the exact failure the mutation gate had to be rebuilt to
  avoid.

### 7.2 `prove_equivalent` — built 2026-08-05, and what is left of it

`crates/chiero-opt/src/equiv.rs`. Relational (product) execution per 041 §1.2: both versions
run on **one shared `TermArena`**, every terminated path of `before` is paired with every
terminated path of `after`, and each pair is conjoined with an explicit equality per matched
entry parameter. `TermArena::var` mints a fresh `VarId` per call, so "the same symbolic
inputs" is *imposed* rather than assumed — which is the useful accident, because it makes the
matching visible: an input with no counterpart is a refusal (`Unknown`), never a zero.

**The witness is minimized by binary search, not taken from the first `Sat`.** Contract 13
wants the swapped argument order to give a correspondingly swapped witness, and two queries
differing only in which side minted variable 0 may legitimately return different models — so
a first-`Sat` witness makes the contract a coin flip. Minimization is canonical, reproducible
(001 §5), and "the smallest input that distinguishes them" is a better thing to hand a reader.

**Eight flattering failures found and fixed, all the project's recurring shape.** Two by
asking what the pairing loop does with nothing to iterate over; six by an adversarial `fable`
review, every one of which reproduced. `crates/chiero-opt/tests/adversarial.rs` holds them.

| what was blessed | verdict it got |
|---|---|
| `g = x; return 0` vs `return 0` | `Equivalent { Exact }` |
| a volatile store vs no store | `Equivalent { Exact }` |
| a dropped unmodeled extern call | `Equivalent { Approximated }` |
| `max_forks = 0` / `max_states = 1`, no loop, disagreeing on 2^32-1 inputs | `Equivalent { Bounded }` |
| every path budget-cut | `Equivalent { Bounded }` |
| a pair with one side cut | `Differs { Termination { Return, Budget } }` |
| termination differing at exactly `{(0,200), (3,7)}` | witness `(0, 7)`, where both return 32 |
| the same as a return difference | `Unknown`, with a real model thrown away |

**A third review found six more, and the finding that matters is not any of them: two were
earlier defects back through a different door.** Each earlier fix had been attached to the
*site* where the defect was demonstrated rather than to the level the rule lives at.

| what came back | how |
|---|---|
| a truncated search is not a proof | the screen lived in `blessable`'s `Bounded` arm; one unmodeled call degrades the run to `Approximated` and the `Bounded` `BudgetHit` sails past |
| a read of caller-visible memory | the guard named `Load`; `CopyMem`'s **source** is a read too |

Both fixes moved: the truncation screen now runs over every assumption before any fidelity is
considered, and the memory guard is written about the *role* an address plays rather than the
instruction that spells it. **When a review finds a defect, the question to ask is what rule it
violates, not what line to change.**

The third new one was worse and unrelated: `malloc` is modeled, the model forks into a success
path and a NULL path on a guard nothing links between the two runs, and it *overwrites* the
extern-return symbol linking works on — so one run's success paired with the other's failure
and **a function differed from itself**. Reflexivity is the cheapest property this operation
has and nothing was asserting it. `EffectKind::ModeledCall` now refuses a modeled call rather
than aligning it, which also stops a dead `memcpy` between two locals reading as observable I/O.

**A second review, after contract 6 landed, found five more** — three of them again false
`Equivalent`, and this time the wrong reasoning was reasoning *I had written down as the
justification*:

| what was blessed | verdict it got |
|---|---|
| a global read either side of a call that may write it | `Equivalent { Approximated }` |
| `p(x)` against `p(x + 1)`, `p` declared `pure` | `Equivalent { Approximated }` |
| returning `p(2)` against returning `p(1)` | `Equivalent { Approximated }` |
| two pure calls reordered, computing the same value | `Differs`, where both return 0 |
| `memset` against `__builtin_memset`, byte-identical | `Differs` |

**The lesson is about comments, twice.** "The ordinal is the same thing the effect sequence
orders by" was false — `ExternReturn` is minted only for a call *with a destination*, so a
discarded result shifts the numbering. "Pure, therefore declared to do nothing observable" was
false — `pure` means no side effects, not a return value independent of the arguments; `abs` is
pure. Both were written in the same commit as the code they justified, and both were
convincing enough to ship. **A plausible rationale is not evidence, and writing one down makes
it harder to check, not easier.**

**The one worth remembering: the first three were already ruled out in the module
documentation, in the same commit as the code that did not do it.** *"A comparison that
would have to reason about caller-visible memory or about a side-effect sequence answers
`Unknown` naming the claim it could not check."* Nothing implemented that sentence. A written
intention with no implementation is worse than an admitted gap — it is what a reader checks
*instead of* the code.

The two witness defects shared one cause: the minimizer fixed inputs one at a time but seeded
each from a model taken before any were fixed. Where the divergence set is not a product,
that seed is unreachable under the earlier pins. Now re-solved per input, one extra query, so
the loop's invariant is true rather than asserted.

**What the contract suite could not have caught:** its fixtures are pure, one-parameter,
branch-light arithmetic — no global, no volatile, no extern, no two-parameter `Differs`, and
its one budget test used `max_states = 0`, the single value where nothing finishes and the
guard fires.

**Left to build, in rough order of value:**

0. **✅ DONE — `chiero-cli`, 2026-08-05.** Five operations from a command line:
   `prove-equivalent`, `impact`, `select-tests`, `expansion-sites`, `explain-macro`, each
   printing an envelope (`--json` for the machine form). `Envelope::render` now renders a
   result as lines rather than as compact JSON, and `serde_json` gained `preserve_order` so
   `verdict` leads instead of sorting alphabetically under `replay`.

   **Every `$ chiero ...` block in the tutorials is a transcript under test**
   (`crates/chiero-cli/tests/tutorial_transcripts.rs`) and must match byte for byte. That test
   exists because I hand-wrote those blocks and every one was wrong — invented entity order,
   omitted fields, and, on the page about telling a proof from a guess, a "proven, Exact" with
   both blind spots missing.

   *Superseded — kept for the reasoning:*

   ~~**⭐ `chiero-cli` — the user asked for it, 2026-08-05.**~~ *"add the CLI to trigger all those
   great cases without the user having to do too much programming; and update the tutorials
   with how they are used."* `crates/chiero-cli/src/main.rs` is still a 5-line stub that prints
   a version. Every operation in `chiero-tool` is reachable only from Rust, so the tutorials
   teach a library API to someone who wants a command. Wanted, at least:
   `chiero prove-equivalent before.c after.c --entry f`, `chiero select-tests`,
   `chiero impact`, `chiero expansion-sites`, `chiero explain-macro`. 050 §1 says `chiero-cli`
   is "a thin wrapper over the identical" operation surface, so the shape is settled.

   Also from the same message, and already applied: **every tutorial must show the data it
   talks about.** Tutorial 4 described an LLM's rewrite in prose and never showed the `after`
   C, which is exactly the thing a reader stops to ask about. Audited all five.

### 7.4 `chiero-replay` — a review that found ten defects, and what is left of them

A fourth adversarial review (2026-08-06) found **ten defects**, all reproduced. The headline
verdict is the one to keep:

> "The harness is the one thing that asks a real compiler" is true only for one narrow
> observable: *the two return values, cast to `long long`, at one input, called sequentially in
> one process*. That observable is narrower than the divergences it adjudicates, and it is
> corruptible by shared state in the combined TU. So the arbiter is neither sound (it can
> fabricate `Demonstrated`) nor complete (it reports `NotDemonstrated` for real divergences),
> and contract 11's downgrade converts the incompleteness into wrong verdict changes.

**The worst is D1, and it inverts contract 11.** `prove_equivalent_with_replay` discards the
`observation` and downgrades on any `NotDemonstrated` — so a true `SideEffect`, `Termination`
or `Memory` divergence, which the harness cannot see at all, drops from `Exact/proven` to
`Approximated` with the assumption text *"chiero's semantics and this compiler do not agree
here"*. That statement is false; the compiler was never asked. **Contract 11 exists to catch
chiero being wrong and currently punishes it for being right, systematically.**

The rest, in short: `Demonstrated` can be fabricated three ways (globals merged by the
two-include trick, pointer returns whose addresses always differ, and an entry that prints
`before=… after=…` itself, since the result shares stdout with the program under test); no
wall-clock limit, so `--allow-replay-exec` on a `Termination` finding hangs the tool at the
witness chosen to show the hang; witness bindings are rendered as a positional argument list
even when they are extern returns or when a pointer parameter minted none; `literal()`
truncates above 64 bits and renders float bit-patterns as integers; the return channel
`(long long) f(...)` refuses `void`, truncates `double` and `__int128`, and 050 §6's sandbox
does not exist while the doc comments cite contract 12 as though it did.

**Seven of the ten are fixed** (2026-08-06), at the rule rather than at the sites:

| # | fix |
|---|---|
| D1 | only a `ReturnValue` divergence may be adjudicated; anything else refuses and says which kind went unchecked. Contract 11 still fires where the harness *did* measure — a test asserts it |
| D4 | `emit_equivalence` returns `Result<Replay, Refusal>`; a witness that is not an argument list (extern returns, pointer params, non-contiguous indices) is refused, not compiled |
| D5 | widths > 64 refused — gcc truncates a decimal constant silently and `-w` hides it |
| D6 | the tool layer refuses a return type the `long long` channel would convert (`double`) or truncate (`__int128`) |
| D7 | the result goes to a file the harness is compiled with, not stdout, which the included program can write |
| D3 | a ten-second wall-clock limit — a `Termination` witness *is* an input that does not terminate |
| D8 | `Outcome::NotRun` and `Outcome::NoCompiler` are distinct |
| D10 | `ReplaySources::flags` carries the TU's `-I`/`-D` (040 §3's last rule) |

**Left, and worth knowing before trusting `--allow-replay-exec` on real code:**

- ~~**050 §6's sandbox does not exist.**~~ **Built 2026-08-06.** A network namespace of its
  own, a 2 GiB address-space cap, a cleared environment, the scratch directory as cwd, and the
  ten-second clock. Three C fixtures attempt the forbidden things.

  **Writes are still not confined, and the code says so in those words** — without root it
  needs more than an unprivileged user namespace, since remounting the filesystem read-only
  inside one fails on the underlying device. So `Sandbox` *reports* what this machine enforces
  and a test asserts that report against what a fixture harness actually manages, in whichever
  direction. **A limit claimed and not enforced is worse than one honestly absent**, and the
  test fails on exactly that. Every confirmation carries the report as an assumption.
- ~~**D9, the two-include trick.**~~ **Fixed 2026-08-06 — three translation units.** Each
  version compiles alone (so a shared `static` helper stays file-local) with a non-static
  wrapper appended *inside* it (so a `static` entry is still reachable, which is what the
  single-TU trick existed for). The entry is renamed per unit, since two units defining a
  non-static `f` collide at link time. Verified end to end on two versions of an `abs()` that
  share two static helpers: `differs` at `INT_MIN`, `outcome: demonstrated`.

  *Still open:* a **non-static** helper the two versions share collides the same way. Renaming
  every shared symbol needs the file parsed, which is more than a harness should do.
- ~~**D2's remaining route.**~~ **Closed 2026-08-06, at the class rather than the door.** A
  fifth review fabricated `Demonstrated` from two byte-identical sources *four* ways —
  `rand()`, `clock_gettime()`, a constructor, and an `atexit` handler rewriting the result
  file. All four were one defect: `before` and `after` ran **in one process**, so everything
  outside a translation unit was shared.

  **Each version is now its own program**, built and run separately, `_exit`ing after it writes
  so no atexit handler can rewrite the answer — plus a **determinism re-check**, since
  isolation cannot fix a program that reads the clock. Running `before` twice and disagreeing
  is `Outcome::Nondeterministic`, which is *not* a downgrade: nothing was learned about chiero.

  > **The lesson, after five rounds on one file: I kept fixing the door.** Each round I closed
  > the demonstrated route and the next review walked through a neighbouring one. The fixes that
  > held were the ones that changed the *shape* — refuse what the channel cannot carry, one
  > process per version, one place that decides how to launch.

**Still open from the fifth review, and worth reading before trusting a `demonstrated`:**

- **S1/S9 — the argument types are unchecked.** `unrepresentable_return` guards the *return*
  type; nothing guards the *parameters*. A `float` parameter's witness is a **bit pattern**
  (the engine sorts floats as `BitVec`), rendered as a decimal and passed through `long long`
  — so `2.0f` goes in as `1073741824.0f`, the harness reports agreement, and contract 11
  downgrades a **true** finding with the false sentence D1 was filed for. The rule is *every
  value crossing the channel, in either direction*; only one direction is written. Arity is
  the same gap: a trailing pointer parameter leaves the indices contiguous and the call short.
- **S4 — nothing ties `cfg.entry` to `src.entry`.** `prove_equivalent` compares one function
  and the harness compiles whichever the sources name; they are independent strings.
  `unrepresentable_return` returns `None` (= representable) for an absent entry, so the type
  gate silently no-ops on exactly that case. The reviewer got a fabricated `demonstrated` at
  `proven: true`.
- ~~S1/S9, S4, S6, S8, S10~~ — **fixed 2026-08-06.** One rule for every value crossing the
  channel in either direction (`harness_signature_objection` checks the return *and* every
  parameter, and an absent entry is an objection rather than silence); `cfg.entry` must equal
  `src.entry`; the compile gets the same wall clock as the run; `-fcommon` is refused by name;
  a relative or quoted scratch path is refused with a message about the path.
- ~~S7~~ — **fixed 2026-08-06.** The timeout kills anything still running with this call's
  unique path on its command line, in *both* the run and the compile paths. Writing the test
  found the same leak where the fix would not have reached: a compiler driver spawns `cc1` as a
  child, so the compile timeout killed `cc` and left `cc1` grinding on the blocked source.

**All ten of the fifth review are closed.** The harness is narrow — return-value divergences,
integer parameters and returns of ≤64 bits, an entry that matches the module chiero analysed —
and it refuses everything else by name. Within that, `Demonstrated` now survives every
fabrication the review could construct.
- ~~Refusal whitespace~~ and ~~`sandbox()`'s per-call `unshare` spawn~~ — fixed.

Probes: ~~`$SCRATCH/rev5` (20 fixtures, `cargo run -- <name>`); `$SCRATCH/replayprobe` (13)~~ —
**both lost** (§9.2). They were never committed.

### 7.3 A defect the operations found in the layer beneath them, 2026-08-06

Pointing `chiero check-reachable` at a `return` line answered *"the function has no code on
line 4"*. **015 §5's rule is written over a block's instructions, and `return <constant>;`
lowers to a terminator with no instructions at all** — so both return blocks of
`if (v) return 1; return 2;` had an empty `gcov_lines` while gcov counted both lines.

Sixteen lines across fourteen lowered goldens were missing. §5 calls `gcov_lines` "the join
point of the entire differentiating claim (030 → 031 → 032)", so every one was a line coverage
correlation could not reach. The implementation matched the spec, which makes it a spec gap
rather than a slip — and it survived a full-VPP cross-validation because that validates the
*decoder* against gcov, not the CIR correlation.

**Worth remembering as a method, not a bug:** the defect surfaced within minutes of the
operation existing, by using it on ordinary C. Nothing in the test suite was going to find it.

1. ~~**§1.3's replay harness**~~ — **built 2026-08-06.** `chiero-replay` emits a self-contained
   C program that `#include`s both versions with the entry renamed (040 §3.1's mechanism, the
   only one that reaches a `static` target), calls each at the witness, and exits 0 only when
   they disagree. `chiero prove-equivalent ... --allow-replay-exec` reports
   `outcome: demonstrated` with the two numbers **a real compiler** produced, and the standing
   "no replay harness was compiled" blind spot is removed because it is no longer true.

   **This is the first claim in the system that does not rest on chiero's own semantics.**
   `Outcome` has four values and three are ways of having demonstrated nothing;
   `not_demonstrated` is 041 contract 11's downgrade — chiero and a compiler disagree, fidelity
   drops to `Approximated`, and the verdict stays `differs` because something *is* wrong and a
   reader needs both claims. Execution is gated behind `--allow-replay-exec` (050 contract 11).

    **040 contract 4 landed 2026-08-06**: every `find_bugs` finding carries a harness, and
   `--allow-replay-exec` reports `outcome: faulted, confirms: true` for a division by zero — a
   real compiler dying on `SIGFPE` at chiero's witness. `FindingOutcome` is a different shape
   from `Outcome` because a finding has one program: it is reproduced when it *faults*, and the
   **signal** is the answer (a process killed by `SIGFPE` has no exit code at all). It reuses
   the equivalence harness's launcher rather than copying it — every piece of that machinery
   was earned by a review finding a hole, and a second copy would start again from the first.

   *Left:* the harness takes **scalar parameters only**. 040 §3's construction rules also want
   memory objects as initialized byte arrays with the engine's own pointer layout, and extern
   stubs returning the values the engine chose in call order. Neither is built, and both are
   refused by name rather than guessed at.
   *"Your rewrite is wrong" is an opinion; "here is the program" ends the discussion.* Nothing
   in the tree emits a C replay harness yet — 040 §3 wants one too.
1a. ~~**041 §2 opportunity detection**~~ — **contract 15 built 2026-08-06.**
   `chiero_opt::opportunity::detect` proposes a branch the path condition already decides,
   with the implying constraints as SMT-LIB. It reads the engine's own `Event::Fork { feasible }`
   rather than re-asking the solver — the engine has decided that question and a second answer
   would eventually disagree. A proposal from a run that did not finish is **advisory** and
   names the budget: "no state took that edge" and "no state *can*" are the same observation
   and opposite claims, and here the difference is whether somebody deletes live code.

   **Contract 14 landed 2026-08-06** — the redundant-load detector, and the contract that makes
   the obligation machinery mean something: the *observation* is identical across a callee
   chiero can see through and across one it cannot, and only the strength of the claim differs
   (all `Discharged` vs one `Open` and `advisory`).

   **It works on real C** (2026-08-06, after two wrong diagnoses). `int a = *p; quiet(a);
   int b = *p;` as gcc hands it over comes back `redundant_load` with every obligation
   *discharged*; the same function with an `extern` between comes back **advisory**.

   > **Both wrong turns were the same mistake: matching on how the CIR spelled something
   > instead of asking what it was.** The identity criterion was "the same `ValueId` loaded
   > twice", which unoptimized C never satisfies because `p` lives in a slot and is reloaded —
   > it is now the engine's own `Pointer` (object + offset), which is 021's answer rather than
   > a second one. And "a callee with no store" cleared nothing, because lowering stores every
   > parameter into a slot — it is now "a callee whose every store is into its own confined
   > local", reusing the caller-side escape check.
   >
   > I recorded the limitation as needing "redundant-load analysis one level down". That was
   > the wrong diagnosis: **the level below already had the answer and was not being asked.**

   **Dead store landed too** (2026-08-06) — `*p = a; *p = b;` proposes the first write dead,
   discharged, and a call between makes it advisory. Keyed on the engine's `Pointer` from the
   start, because the load detector had already paid for that lesson twice. **Two tables, not
   one:** a load is redundant when nothing could have *written* between and a store is dead when
   nothing could have *read* between, which are opposite questions.

   *Left:* loop-invariant computation, redundant bounds check, call-site specialization,
   unreachable code, unnecessary zeroing.

1b. ~~**041 §3 locality**~~ — **built 2026-08-06.** `chiero_opt::locality`: line straddling
   (contract 18's boundary both ways), padding waste with the byte delta, hot/cold placement.
   Contract 21's advisory rule and contract 22's honest labelling are most of the module —
   `advisory` is *derived* from the obligations, `Benefit::Estimated` is in the enum and never
   produced (no cycle model, and §3 says not to pretend), and `Measured` is reachable only from
   real counts. The layout arrives as data: 014 §3 computes it and is measured against gcc, so
   re-deriving it here would be a second answer.

   *Left:* contracts 19, 20, 23 need the `FieldAccessProfile` §3 specifies — false sharing needs
   025's `Sharing` classification, prefetch distance needs loop stride. And nothing calls
   `analyse` yet: it wants a caller that turns 014's `RecordLayout` into a `Record`, which is
   a natural `chiero-vpp` or CLI job.

2. **§1.1's remaining claim — caller-visible memory** (with the object bijection, contracts
   13c/13d). `observable_beyond_the_return` refuses anything that could touch it: a volatile
   access, a store through an address that is not provably a stack slot, inline asm, a
   variadic list, an indirect call. Every one of those refusals is a comparison that should
   be possible later.

   *Done since:* **contract 6, the side-effect sequence.** `EffectKind::Call` carries the
   callee and its **arguments as terms** — the load-bearing half, since contract 6's rewrite
   swaps two calls to the *same* function and a name sequence is identical either way.
   `link_inputs` learned §1.2's shared extern-return symbols, keyed by (function, nth call),
   not by span: the two versions are different modules and a span key would match nothing.

   **`Approximated` can carry an `Equivalent` under one narrow condition**, arrived at after
   two wrong versions of the argument. Three channels connect a callee to the comparison: the
   effect sequence (compared position by position), memory (loads *and* stores through a
   non-local address refused, pointer arguments refused outright), and the return value —
   where both earlier attempts failed. So the condition is that neither side has an
   extern-return input at all. `proven` stays false; 032 §3.1 still refuses to drop a test.

   **§1.2's shared extern-return symbols are matched, on the third attempt** (2026-08-05).
   `InputOrigin::ExternReturn`/`ModelReturn` carry `seq`, the call's index in the effect
   sequence; **every** declared call is in that sequence, pure ones as `EffectKind::PureCall`,
   so the ordinal counts one thing. `comparable_effects` then drops pure calls whose result
   nobody bound — `pure` plus an unread return is genuinely unobservable — and the link key is
   the position in *that* list, because dropping one from one side would shift every later raw
   index. `compare_effects` runs before any return is linked, so position *n* is only used as a
   key once the two runs' *n*th calls are shown to be the same callee with the same arguments.

   A function with a value-returning callee is now answerable: `return p(x)` against
   `return p(x + 0)` is `Equivalent`, `p(a) - p(b)` against `p(b) - p(a)` is not.
3. **Pointer parameters and pointer returns**, which currently answer `Unknown` by name.
4. **032 §3.1's `Prover` seam wired to it.** The blocker is not equivalence — it is that
   `Prover::prove_equivalent(&chiero_diff::Entity)` has to turn an entity into two runnable
   modules, which needs the frontend from a crate that must not depend on it.

### 7.6 `find-bugs` measured on VPP, 2026-08-06 — 231 findings to 1, and why

The first time the defect checkers were pointed at real VPP code rather than at fixtures.
40 entry points from `vppinfra/{bitmap,mem_dlmalloc,hash,vec,time}.c` and
`vlib/{node_cli,counter}.c`. **Checked in and reproducible: `tests/corpus/vpp-findings/`,
`./measure.sh` (~4 min, needs a VPP checkout).**

| | findings | `Exact` |
|---|---|---|
| at the start of the wave | 231 | 1, and it was **wrong** |
| now | 23 | 0 |
| now, `--entry-ptr-nonnull` | **1** | 0 |

**Every one of the four fixes came from reading what was left after the previous one.** That is
the method worth keeping, not the number:

| what the output said | what was actually wrong |
|---|---|
| one `Exact` finding, on `_vec_update_len` | a bound chiero invented could produce `proven: true` |
| 147 of 157 were that same invented bound | reported at all, when chiero knows nothing about the caller's object |
| 5 said an `extern` global "was never written" | 021 §6's rule applied to entry pointers and not to `extern` |
| 4 said a bitfield was uninitialized, then `symbolic-byte` | the bit read path discharged neither laziness nor symbols |

**The false proof, because it is the one to remember.** `_vec_update_len` reported
`out-of-bounds: 4-byte access at offset -8 of the 4096-byte object` and `proven — this holds
for all inputs`. That access is `_vec_find (v)->len = n_elts`, and `_vec_find(v)` is
`((vec_header_t *) (v) - 1)`: **every VPP vector is an interior pointer by design**, and 021
has a worked example of exactly this. The engine had the example and shipped the false positive
anyway, because the rule that produced it lived somewhere else. Two chiero inventions, neither
a fact about the program — `ENTRY_PARAM_BYTES` = 4096, and the pointer placed at offset 0 of
that object. The finding's own text carries the contradiction: a pointer cannot be both
"unconstrained" and known to sit at a 4096-byte object's base.

**The recurring shape, for the third and fourth time in one wave: chiero not knowing a value is
not the program failing to write one.** 021 §6 settled it for entry pointers ("fully symbolic
and fully initialized", to avoid "an uninitialized-read false-positive storm"). An `extern`
global is the same storm through a header; a bitfield is the same storm through an access path
that went around the rule. If a fifth turns up, look for a read path that does not end in a
symbol.

**What is left of the 40, and it is the right shape.** One finding: `clib_time_init` divides by
a value the path allows to be zero. `Unknown`, with inline asm, `__builtin_expect`, an opaque
write and 1496 unreported invented-bound accesses all named in the envelope. A reader can act on
that or dismiss it in one pass, which is the whole objective.

#### The ACL plugin — 207 entry points, 16 findings → 10, and one wrong answer underneath

`LIST=<acl.tsv> tests/corpus/vpp-findings/measure.sh --entry-ptr-nonnull`. 196 analysed,
11 time out at 30 s.

| | findings |
|---|---|
| first run | 16 |
| after `__builtin_expect` | 3 of the first 198 (12 at the tail) |
| after `copy_via` | 10, of which 9 were one defect |
| after `memoize_fresh` | **1** — 196 analysed, 11 time out |

The one that remains, `relax_ip4_addr`, is about a **`static` helper analysed in isolation**:

```c
int shifts_per_relax[2][4] = { { 6, 5, 4, 2 }, { 3, 2, 1, 1 } };
int *shifts = shifts_per_relax[relax2];      /* relax2 unchecked */
```

Both call sites pass a literal `0` or `1`, so the program is safe as assembled. The engine
already has this rule for *null* parameters — "only an **exported** entry gets the assumption…
for a `static` function every call site is in this module" — and it is not applied to a scalar
parameter used as an index. That is the next honest improvement here, and it is the same
sentence one argument-kind over.

Two engine defects, both with wide blast radius:

1. **`__builtin_expect` was an opaque call.** `PREDICT_FALSE (v == 0)` is *the* guard idiom in
   VPP, so the branch stopped being about `v`, the null path survived into the body, and
   `vec_validate`'s guarded `_vec_len (v)` — `v[-8]` — reported a null dereference. GCC defines
   the builtin as returning its first argument; it now lowers to that argument.
2. **`Memory::copy` did not discharge 021 §6's laziness** on its source, so every by-value
   aggregate parameter — 015 lowers `f (struct id s)` to a `CopyMem` from the caller's pointer —
   read back as uninitialized. `copy_via` is the third API in that family.

⚠️ **Twice while fixing (1) I broke lowering on real VPP and `./check.sh` stayed green.**
`type_of` walks *down* sema's conversion chain while `self.expr` emits it, so `Int(8)` was
declared for a promoted `char` (18 of 40 entry points went `ok` → `failed`), and then
`CTy::Int(width_of(..))` assumed `Int(32)` for an `F64` (95 of 207 went `ok` → `failed`). Both
were caught by the checked-in measurement and by nothing else. `Lowerer::top_cty` is the fix and
"guessing `Int` at all" was the actual mistake. **Run `measure.sh` before committing anything
that touches lowering** — it is four minutes.

#### The wide sweep — 220 entry points across `vnet/`, and three more of the same

Run it with `LIST=<file> tests/corpus/vpp-findings/measure.sh` (the `LIST` variable exists for
this; `entries.tsv` stays the pinned 40). 220 entries: 4 definitions each from every `.c` under
`vnet/*/` and `vlib/`, first sweep at 30 s.

| | |
|---|---|
| 220 entries | 214 analysed, **6 time out**, 0 refused |
| findings | 42 → **33** after the promotion fix below, 0 `Exact` |
| worst function | 3 findings — no artifact dominates any more |

**It found three things the 40 could not.** Breadth beats depth here, and cheaply:

1. **8 entries produced no output at all** — `measure.sh` took the *first* of `build.ninja`'s
   1969 `INCLUDES` lines, and they differ. Every `*_api.c` in the tree needs a per-target root
   (`CMakeFiles/vnet`) for the API compiler's generated `<bier/bier.api_enum.h>`. A file that
   will not preprocess is a file the measurement did not cover, not a file with no defects —
   and this one had 21 findings behind it.
2. **`symbolic-byte` and a `{:?}` of an internal Rust enum**, reported as defects in someone's
   code — `strcpy: source scan gave CapReached { scanned: 0 }`. Fixed at both funnels; see
   `MemFault::is_chiero_limit`.
3. **Promotion to array theory discarded 021 §6's initialization** — the seventh instance of
   the same confusion.

**Two classes remain, and they look like the eighth and ninth.** Of the 42:

```
  13  unresolvable pointer: the value is unconstrained, so it could refer to any object or none
  10  a symbolic pointer could not be resolved: the solver did not decide ...
  10  uninitialized-read
   5  null-dereference
   2  pointer-outside-object
   2  maybe-uninitialized-read
```

The first two are 23 of 42 and read as statements about chiero, not about a program — but
unlike `symbolic-byte` they are *not* obviously so: a genuinely arbitrary pointer value is a
real hazard, and the line between "chiero lost track of this pointer" and "this program has a
wild pointer" is where the answer is. **Decide that before filtering**, or a real
`WildPointer` class gets suppressed with them.

~~⚠️ **`MemFault::BadRange` belongs to the `is_chiero_limit` class and deliberately is not in
it.**~~ **Closed 2026-08-07** — it is in it, and a 32-byte AVX load now degrades to
`Fidelity::Unknown` with a named assumption instead of appearing in a defect list.

The blocker was never an argument, it was two fixtures: `BadRange` was the only **objectless
non-fatal** fault there is (`NullDeref` and `WildPointer` are objectless and fatal), so it was
the only way to put two findings agreeing on `object` on one path, and two of `FindingKey`'s
component probes were written with it. **The way past it was to stop needing objectless at all**
— what `func` and `span` defend against is two findings agreeing on the other three components,
and a *shared* object reaches that just as well as an absent one. The probes are now one alloca
`memcpy`'d onto itself in a callee and again in the caller.

Two things that cost minutes and would have cost a wave each:

- **`uninitialized-read` is not repeatable.** Two reads of the same uninitialized bytes are
  **one** finding — the first read's invented value is written back, so the second has nothing
  to say. A span probe built on it silently measures nothing. `overlapping-copy` is the
  non-fatal fault that does repeat.
- **The mutants are the evidence, not the reasoning** (§11.3). `func := FuncId(0)` at the three
  `FindingKey {` construction sites kills the func probe and leaves the span one passing;
  `span := Span::DUMMY` does the converse. Without running both, neither probe is distinguishable
  from a fixture that merely passes.

`--entry-ptr-nonnull` (`BugCfg::entry_ptr_nonnull`) and `--report-invented-bounds`
(`BugCfg::report_invented_bounds`) are the two knobs this wave added; both are on `find-bugs`,
the first also on `check-reachable`, and both are recorded in the envelope when used.

#### The wall clock, and what `timeout` was hiding — 2026-08-06

023 §8.1 specified `Budget::wall_clock` and the engine never had the field; the type's comment
said the wall clock was "kept out of anything that gates output", which reads as caution about
determinism and behaves as **silence**. Nothing bounded a run in time, so the only way to stop
one was to kill the process, and a killed process prints nothing at all.

It is implemented now. On expiry the running state terminates with one `BudgetHit` naming the
bound *and how many states were left unexplored*, the worklist is cleared (which is also what
ends the loop), and the envelope carries `nondeterministic_abort`. Three decisions:

- **the library default is `None`** where 023 §8 says 60 s — §8.1 requires the determinism
  contracts to run without a clock and `Budget::default()` is what they use; the CLI sets 60 s
  and takes `--time-budget <secs>` (decimals, `0` = none, as `timeout(1)` has it);
- **the flag follows the abort, not the configuration** — a run that finished inside its clock
  is byte-for-byte the run it would have been without one;
- **`check_reachable` cannot answer `unreachable` on a cut run**, which holds by construction
  since the abort degrades the state, with a `debug_assert` so it cannot quietly stop holding.

⚠️ **The residue, and it is nameable now.** The clock is checked *between steps*, so a single
step can overrun it: three plugin entries still hit the harness's outer `timeout` at +30 s, and
one of them ran 10 s against a 5 s budget in a symbolic-offset enumeration. The frontend is not
the problem — `layout` on the same files takes 1 s. `prove-equivalent` has no clock at all.

#### The plugin sweep — 477 entry points, 92 plugins, two panics and one true `Exact`

`LIST=<list> TIMEOUT=20 measure.sh --entry-ptr-nonnull`, one function from each `.c` under
`plugins/*/` except `acl/`. **408 ok, 20 cut, 3 timeout, 35 noinc, 11 failed, 18 findings.**

**Two source-triggerable panics, both filed as `failed`** — the same row a file that will not
preprocess gets, so two crashes on real code read as two files chiero could not read:

1. `Engine::indirect` took **every defined function** as a candidate, capped at 16, against its
   own comment saying "every defined function *whose signature could be called here*". A
   pointer to `clib_error_t *(*)(void *, src_t *)` entered a candidate returning `unsigned
   char`; comparing that against a null pointer aborted the run.
2. **A `_Bool` store**: CIR types it `Int(1)`, `size_of_cty` rounds to a byte, and the
   array-backed write path extracted bits 7..0 of a one-bit term. `mp->admin_up_down = ... ? 1
   : 0` in a VPP API handler, needing only a `strncpy` into the same struct first.

**The one `Exact` is true, and gcc says so.** `comp_ring->gen ^= VMXNET3_TXCF_GEN` where the
macro is `(1 << 31)` — signed overflow, undefined by C11 6.5.7p4, and
`gcc -Wshift-overflow=2` prints "requires 33 bits to represent". The first `Exact` on real VPP
that survived being checked.

⚠️ **And the eighth instance of the recurring confusion was mine.** The first fix refused a
store whose value was wider than its type — correctly — and *dropped the write*. Re-measuring
turned 18 findings into 133, of which **108 were `uninitialized-read` on `__X`**, the parameter
name in gcc's `ia32intrin.h`: an unresolvable callee had entered `__bsfd (int __X)` with a
pointer argument. Two fixes, and the second is the one to remember: candidates are now filtered
by **parameter types**, not only arity — and a store chiero cannot represent **havocs its
destination** (021 §6, symbolic and initialized) rather than leaving it never-written.
**Re-measure after a fix, not only before it.** Nothing in 2136 tests moved; the corpus did.

#### What the remaining 11 `failed` are, now that errors carry a location

`frontend::at` prints `path:line:col`, with `expanded from` when a macro put the error there:

| | |
|---|---|
| 7 | `` `clib_crc32c_with_init` was not declared `` at `cnat_node.h:226` — `vppinfra/crc32.h` defines it only under `__SSE4_2__`, and `frontend::predefines` asks gcc for its macros with **no `-march`** while VPP builds with `-march=x86-64-v2`. Same cause as `u32x4_sum_elts` in `soft-rss`. |
| 2 | `http_static.c:142:39: expected a type specifier` — a generated `vl_api_*_t` typedef the parser did not record |
| 1 | `lldp_api.c:135:7: no member named last_heard_age`, expanded from `120:3` inside `REPLY_MACRO_DETAILS4_END` |
| 1 | `mactime_top.c`: `` `vl_msg_api_set_handlers` was not declared `` |

### 7.7 Two defects the *user* found by running the tool, 2026-08-07

Both arrived the same way — somebody pointed a command at real code and read the answer — and
neither was reachable from the test suite as it stood. That is now three waves running where
the operations found what the suite could not (§7.3, §7.6, this).

#### `layout` said a 72-byte struct would be 8

`chiero layout` on `plugins/acl/acl.h` reported `fib_route_path_t_` at 72 bytes with
`recoverable: 64` — "would be 8 with its fields ordered by size". Size and alignment were
**right** (gcc agrees, 72 and 8), which is what made the number read as an answer.

`frontend::records` built each field as `names.text(fl.name?)` inside a `filter_map`, so a
member with **no name** was skipped without a word — and that struct is mostly a 56-byte
anonymous union. The ideal layout was then computed over the 7 bytes that were left, and
"would be 8" is 7 rounded up to the struct's alignment.

Two fixes, and the second generalises: anonymous members are counted (with a synthetic name,
since the padding sum needs their extent), and a record whose field list is **knowingly
partial** — a bit-field, whose extent is bits inside a storage unit its neighbours share —
gets no padding proposal at all, with the envelope naming which records that happened to. A
number computed from part of a struct is not a smaller number, it is a wrong one.

Checked against gcc rather than arithmetic: a hand-reordered `fib_route_path_t` compiles to
**64 bytes**, which is what chiero now reports as the floor. On that one header, 17 padding
proposals became 12 and five had been wrong — `rusage` "144 → 32", `ip_adjacency_t_`
"256 → 192", `vnet_tm_level_capa_params_` "96 → 24", `fib_prefix_t_` "20 → 4".

Then the follow-up ask, which was the better half: **a total is not advice.** The proposal now
names the fields each hole sits between, reconciles the holes against what a reorder actually
recovers (9 bytes of padding, 8 recoverable, because alignment rounds the tail up whatever the
order), and caps the list at eight saying how many it did not show.

#### `check_reachable` proved a dead line live, when there was no solver

`int f (int x) { if (x != x) return 1; return 2; }` — line 3 is dead for every input. With z3:
`unreachable`, `Exact`. With tier 1 alone:

```text
verdict: reachable
witness:
  - origin: parameter 0
    value: 0
    pinned: false          ← there is no input; this number is invented
proven — this holds for all inputs (Exact)
```

**A proof that a dead line is live**, from the operation built to keep "nothing gets here"
apart from "I did not get here", with the witness confessing in the same breath. The cause was
one sentence, true in general and false in the case that matters: *"a path that arrived is a
fact about this program, whatever else the run had to approximate."* It is not a fact when the
**arrival itself** rests on a branch nobody decided — 023 §7 takes an undecided branch rather
than drop a path that may exist, which is right, and a proof does not follow from it.

`reachable` now requires the state's fidelity to be `Exact` **and** every witness binding to be
pinned. Neither implies the other. Otherwise: `not_shown_reachable`, carrying the candidate
witness, with a blind spot saying a path did reach the line and may not exist.

**This is the `_vec_update_len` shape a third time** — a proof resting on something chiero
invented — and the lesson is not about solvers. Whenever an answer is `Exact`, ask what would
have to be true for the *arrival at that answer* to be an artefact of a limit rather than a
fact about the program.

### 7.8 The two solver configurations, and why both are gates

Reported from GitHub: five `chiero-check` tests failing there and passing everywhere else.
**CI has no z3.** They asserted what a complete solver decides and never checked whether there
was one — an asymmetry, not a flake.

CI is now a matrix over `solver: [none, z3]`, and **both legs gate** as of 2026-08-07:

| leg | what it is for |
|---|---|
| `z3` | the solver-dependent half of the suite, which was absent from CI entirely |
| `none` | 022 contract 2 — a machine with no solver answers what tier 1 can and `Unknown` for the rest |

The `none` leg pins `$CHIERO_SMT_SOLVER` at a path that does not exist rather than trusting the
runner image to lack z3; a configuration resting on an unstated assumption is not one. Twenty
tests needed handling, in three kinds — **skip** (the claim is about what a complete solver
decides), **widen** (the claim survives both tiers and only the recorded cause differs), and
**fix** (§7.7's false proof, which only that configuration could reach).

⚠️ **Reproduce the solverless leg locally with `CHIERO_SMT_SOLVER=/nonexistent cargo test
--workspace --no-fail-fast`.** `--no-fail-fast` is not optional: cargo stops at the first
failing test *binary*, so the first measurement of this said three suites when the answer was
eleven, and each fix revealed the next one.

### 7.9 Bit-fields in `layout`, 2026-08-07 — and the review that broke the first fix

§9's item 1 was "`layout`'s remaining honesty gap": a record with a bit-field got *no*
padding proposal, which was right and left out exactly the packed, hand-tuned structs where
padding matters most. Five commits: `spec:` (041 §3.1 + contract 25), `red:`/`green:` for the
run model, then `red:`/`green:` for what an adversarial review found in it.

**The model.** The field description carries a bit extent, and a maximal run of consecutive
bit-fields is **one synthetic member** spanning the bytes its bits touch. The reorder moves
the run whole and never repacks bits — repacking needs gcc's allocation-unit rules, which
001 §4 rule 7 keeps out of `chiero-opt`, and would describe a struct no declaration order
reaches. Two VPP structs that were silent now get a proposal, both confirmed by handing gcc
the reordered declaration: `test_registration_` 48 → 40, `vnet_crypto_alg_data_t_` 64 → 56.

**⚠️ The fixture was the hard part, and the first one was a false pass.** On
`char tag; long big; unsigned a:3; unsigned b:5;` the right model and the obvious wrong one
(count each bit-field as the byte it starts in) *both* answer 8, because alignment rounds the
difference away — so the number assertion passed while the analysis was still dropping the
members, and only the evidence assertion saw anything. Contract 25 now pins
`struct { char tag; int big; unsigned a:1..d:1; }`, where the wrong model produces no
proposal at all. **Before trusting a fixture, compute what the defect would have said.**

**Then a fable review broke it**, and this is the third wave running where the finding is a
proof resting on something chiero invented:

- **`struct Q { unsigned a:1; unsigned :0; char c; unsigned b:1; unsigned :0; char d; }`** —
  12 bytes, and chiero said it "would be 4", `proven`, not advisory. gcc's floor over all 24
  orders that keep each run together is **8**. A `:0` declares no member, so it is in no field
  list — and *cannot* be, because C 6.7.9 has initializers skip unnamed bit-fields and that
  check indexes `fields` positionally. Its effect survives only as a gap that reads exactly
  like padding. `RecordLayout::has_zero_width_bitfield` now says so and the record's field
  list is partial: no number, envelope names it. **The run model cannot fix this by looking
  harder — a `:0`-terminated run's cost depends on where the run is placed, and the padding
  arithmetic is a sum of constants.**
- **An unnamed bit-field was aligning the record**, in sema. `struct { char c; unsigned :0;
  char d; }` was 8/4 where gcc says 5/1. Only a *named* bit-field contributes alignment
  (014 contract 4a). Every `chiero layout` number for such a record was computed from a size
  that was already wrong — the review found it through the proposal, but it lived in 014.
- **Two of my assertions could not fail.** Both new CLI tests greped the envelope for
  "bit-field" *after* the sentence stopped containing it. They now ask which records the
  envelope names, via an `unjudged()` helper that parses the list. **An assertion against
  prose is an assertion against the next rewording.**

Both legs green at `68f7924`: **2152 passed across 252 suites**, and the same 2152 with
`CHIERO_SMT_SOLVER=/nonexistent`. The two VPP findings survive the layout correction unchanged.

**The instrument, and it was proven before it was trusted** —
`tests/corpus/layout/fixed_diff.py` compares chiero's floor against gcc's minimum over every permutation
that keeps each run together. Run against the *stashed pre-fix* binary it prints the
over-claim on `Q`; against the fixed one it does not. A randomized 113-proposal sweep found
nothing either way: the shape needs **two** `:0`-terminated runs, since with one you can
always hide it last. The random generator was not the check, and would have blessed the bug.

### 7.10 The first widening under §8.3's pattern, 2026-08-07 — and a fix that made the gate worse

One seed (`vnet/session/session_types.h`, +86 corpus files) took the contract-12 gate from
5482 to **8492 assertions put to gcc** and rejected **one** layout on its first run. Fixing
that made it reject **eleven**. Both numbers were right, and the second is the interesting one.

1. **`typedef struct {…} T __attribute__((aligned(16)));` aligned the struct.** C puts a
   post-declarator attribute on the *name*: gcc says `struct S` is 104/8 and `T` is 104/16,
   and chiero said 112/16 — wrong entity, plus a size rounded up to it. glibc's
   `__pthread_unwind_buf_t` is this shape, so every TU reaching `<pthread.h>` had it. Fixed by
   marking declarator-sourced attributes (`Attr::from_declarator`) and having `lay_out` read
   only the definition's.
2. **Then eleven.** A member declared with such a typedef was not getting the name's
   alignment — `lay_out` asked `aligned_attr` (the declarator's) rather than `declared_align`
   (which walks `typedef_aligns`). **While defect 1 stood, this was masked**: the alignment
   reached the member through the record's own, and enclosing structs came out right by *two
   cancelling errors*. VPP's `clib_longjmp_t` is the same shape and `serialize_main_t` was
   among the eleven.
3. **Then two, and they were the gate's.** For a record reached through a typedef it asserted
   `_Alignof(T) == RecordLayout::align` — but `_Alignof` answers about the name and the layout
   about the struct, and this wave is precisely about when those differ. It now raises the
   expectation by `Analysis::typedef_align`.

⚠️ **A fix that makes a gate reject *more* is information, not a regression to hide.** The
instinct is to suspect the fix; the right move is to read the new failures, because a
compensating error only shows itself when its partner is removed. Had the gate not been
widened that morning, defect 1 would have been "fixed" and defect 2 would have silently
started mis-laying every struct containing a `clib_longjmp_t`.

### 7.11 The preprocessor conformance corpus — seven waves, 2026-08-07

`cargo run -p xtask -- pp-gate` reads a **simplecpp checkout** (`$SIMPLECPP`, default
`/home/ubuntu/simplecpp`, pinned `74a5a63`) rather than a copy: the corpus is 211 verbatim clang
files (Apache-2.0-with-LLVM-exception) and 26 gcc ones (GPL), and neither may be vendored into an
MIT-OR-Apache-2.0 repo. Pointing a gate at a checkout is this repo's existing precedent and it
dissolves the licence question. ⚠️ **The checkout must not live in a scratchpad** (§9.2).

**gcc and clang are the oracle directly** — the files carry no expected output — and chiero passes
if it matches either, mirroring simplecpp's own `run-tests.py`. simplecpp's skip/todo lists are
carried as *priors, not truth*; chiero now passes two files simplecpp itself still fails.

**Why it paid where four consecutive VPP-shaped widenings had begun returning zeros:** every
corpus before it was real VPP code, which exercises macros *as people write them* and never
reaches the dark corners. §11.3 carries the general form — **change the kind of corpus, not its
size**.

| run | findings | agreement |
|---|---|---|
| first, before any fix | **22** (2 panics) | 92 / 141 |
| after each of seven waves | 19, 21, 18, 17, 16, 16, 14 | → 100 / 141 |
| the owner's close-the-gap pass | 12, 11, 8, 5, 4, 2, 1, **0** | **106 / 141** |

*(21 is not a regression: splitting the "neither compiler ran it" bucket let two rows be seen for
the first time. §7.10's shape — a change that makes a gate reject more is information.)*

**The defects, each with the rule it violated:**

| # | defect | rule |
|---|---|---|
| 1 | **panic** on C11 6.10.3.3p4's own worked example; `FOO(##)` pasted `A` to `B`; `m(hh)` dropped the token | a `##` arriving *by substitution* is not the paste operator — the operator is identified in the replacement list, at definition time |
| 2 | the same panic **still reachable** after the fix, found by adversarial review | `paste`'s output is a substituted sequence, so **nothing leaves it still armed** — clearing state is a different rule from setting it |
| 3 | expansion stalled two steps early on `f(2)(9)` | C99 6.10.3.4p2: the invocation hide set **intersects at the closing paren**; chiero unioned |
| 4 | `__has_attribute` claimed the capability and answered **0 to everything, silently** | the predefine set is an *impersonation of the build compiler*, so the queries answer what **gcc** recognizes |
| 5+6 | one guard both **ate a comma that should stay** and **hid an invalid paste that should fire** | GNU comma-swallow is about `, ## <variadic parameter>` and nothing else |
| 7 | chiero **rejected valid C11** — 51 spurious diagnostics | C11 6.4.2.1: a universal character name is an identifier character |
| 8 | six table rows claimed `1` where gcc says `201904` | `__has_attribute` returns the *standard version* for a standard attribute; a `bool` table could not represent it and the test agreed with all six |
| 9 | scoped operands unanswerable | a scoped operand answers **1, never a version**; and C has no `::`, so `gnu::x` is four tokens |

Specs written: **011 §2.0** (UCNs), **012 §2.3** (where the paste operator lives), **§2.4** (the
hide-set combination rule), **§4.1** (the compiler persona). Contracts **011/15**, **012/20–24**.

⚠️ **One declared divergence, so nobody chases it:** `gcc -E` *normalizes* UCN spelling
(`\u00AA` → `\U000000aa`) and chiero preserves what was written, because 010 contract 11 wants a
token's byte range to re-lex to its own spelling. 011 §2.0 states it. Two gate rows differ **by
spelling, not identity**.

**The persona table** (`chiero-pp/src/features.rs`) is 105 attribute names × 3 queries + 67
builtins = **382 rows, every value measured from gcc**, and a contract test re-asks gcc for each.
A name it does not cover answers 0 **and says so by name** — which is why `answer` returns
`Option<u32>`, and which turned the last wave's fix from an investigation into a lookup.

### ✅ **THE PREPROCESSOR GAP IS CLOSED — pp-gate reports 0 findings** (2026-08-07)

The owner asked for it closed before other work, and it is. 141 C cases:

| outcome | count |
|---|---|
| matches **both** gcc and clang | **106** |
| matches one, where the compilers disagree with each other | 11 |
| rejected by all three — the corpus's negative tests | 21 |
| same program, **rendered** differently (declared, below) | 3 |
| **findings** | **0** |

Started the session at 92 agree / 22 findings.

**The last fourteen, and where each was actually fixed:**

| closure | note |
|---|---|
| `#pragma push_macro`/`pop_macro` | a stack of **name bindings** — a `MacroId` is never reused (012 §1), so `macros` stays append-only and only `by_name` moves. `Option<usize>` because a saved *absence* is a real state |
| `#pragma GCC error`/`warning` | **one text-based implementation** for both the directive and `_Pragma`, since the fixture puts the operator inside a macro |
| `#pragma GCC dependency` | recorded during expansion, answered in `finish` — `_Pragma` runs in `expand_inner`, which has **no `FileLoader`** |
| 8 persona-table rows | measured; `interrupt` and `volatile` differ **across the three queries**, so one answer per name would have been wrong |
| absent ≠ supplied-empty variadic | closed **three files at once**. ⚠️ The first attempt put the condition on `is_variadic_param` and broke the non-empty rows — only the *placemarker* carries it |
| `\`-space-newline splices | both compilers do it and warn; C99 6.10.3.4p6's own example contains one. `splice_len` is the single predicate for scan and rewrite |
| **`__VA_OPT__`** (C23 6.10.3.1) | a **scope change the owner authorised** — 012 §2.3 had it out of v1 by measurement. Resolved into an *effective replacement list* before substitution, so no second code path |
| 2 UCN rows, 1 unterminated-literal row | **fixed in the gate.** `Verdict::RendersDifferently` — the two render the same program differently and re-lexing a rendering is lossy. Never merged into `Agree`; the comparison behind it is deliberately coarser |
| 2 `x######x` rows | **fixed in the gate.** UB where gcc emits `xx` and clang rejects; chiero rejects too, and *agreeing to reject is agreement*. Narrow: it requires chiero to have diagnosed, because silence is not agreement |

⚠️ **Two of the last three "defects" were in my own gate, not in chiero.** The canonicalization
cast bytes to `char` — Latin-1 — so a UTF-8 `¨` became `Â¨` and never matched the decoded escape;
it reported a difference that was entirely its own. **Bytes in, bytes out.**

⚠️ **`__VA_OPT__` and GNU comma-swallowing have opposite conditions.** `P(1,)` supplies an empty
argument and `__VA_OPT__` yields **nothing**; `debug(Y, )` supplies an empty argument and the
comma **stays**. One turns on the argument's *tokens*, the other on whether it was *supplied*.
Two neighbouring rules with opposite tests is how a shared flag ends up wrong — which happened
here, to the comma rule, the same day. Both are pinned by tests.

**Keep `pp-gate` as a standing check** — two minutes, and it is the only thing watching these
corners. ⚠️ It still **exits 0 unconditionally**; making it fail on a finding is now sensible
(the count is 0) but would make CI depend on a checkout it does not have. The honest form is
"fail on a finding when `$SIMPLECPP` exists, print NOTHING WAS MEASURED when it does not".


Keep `pp-gate` as a two-minute standing regression check.

### 7.17 `vnet/ip/` swept, 2026-08-07 — an honest zero, and the class §7.6 predicted

152 entry points, four per `.c` from `vnet/ip/`, a subsystem the find-bugs harness had never
touched. `LIST=<file> TIMEOUT=20 measure.sh --entry-ptr-nonnull`.

| | |
|---|---|
| 143 ok, 7 cut, 1 no-such-function, **0 failed** | no crash, no file chiero could not read |
| **9 findings, 0 `Exact`**, every one `Unknown` | and all of them one class |

**Zero chiero defects.** Every earlier find-bugs widening found engine defects — two
source-triggerable panics, `__builtin_expect` as an opaque call, `Memory::copy` losing 021 §6's
laziness. This one found none, and `0 failed` on a subsystem of packet-processing code is the
result worth recording: the frontend and engine handle shapes quite unlike `vppinfra` containers.

**The 9 findings are all the same shape, and §7.6 named it as the next honest improvement.**
`ip_punt_redirect_add (fib_protocol_t fproto, …)` indexes
`ip_punt_redirect_cfg.redirect_by_rx_sw_if_index[fproto]`, an array of `FIB_PROTOCOL_IP_MAX` = 2
elements inside a 24-byte struct. `fib_protocol_t` admits **three** values — IP4, IP6 and
**MPLS = 2** — so `fproto == MPLS` reads at offset 24 of a 24-byte object. Both callers pass IP4
or IP6, so the program is safe as assembled.

⚠️ **This one is not fixed by the improvement §7.6 proposed, and that is why it is interesting.**
§7.6's plan was to apply the exported-vs-`static` call-site rule to scalar index parameters.
`ip_punt_redirect_add` is **exported**, so that rule would not reach it — and constraining
`fproto` to its *enumerators* would not help either, because `MPLS` is a valid enumerator and
still out of bounds. **An enum-typed parameter whose type is wider than the array it indexes is a
latent contract hazard in the program, not an artefact of chiero's ignorance**, and `Unknown`
fidelity is the honest verdict for it.

### 7.18 010 contract 11, 2026-08-07 — the contract nobody had tested, and it holds

> Round trip: for every token in a preprocessed fixture, the byte range given by `spelling_loc`
> re-lexes to the same token text.

**The only 010 contract with no test anywhere**, established by mapping `chiero-span`'s five test
files against the twenty contracts — 1–2, 3–10, 12, 13–17 and 19 covered, 18 declared as needing
a large fixture, 11 absent. ⚠️ §9 had called it a "sema corpus gate" contract, which named the
wrong spec; **reading the tests took two minutes and the note had been wrong for some time.**

`crates/chiero-pp/tests/spelling_round_trip.rs` — it belongs there because the contract asks for a
*preprocessed* fixture and only the preprocessor makes one. **Zero defects** across twelve
fixtures, including every construct with a span rule of its own: macro-body vs argument spans
(010 §2.2), line splices whose bytes are not contiguous (011 §2.2), and this session's UCN
identifiers.

Two properties worth copying into the next invariant test of this shape:

- **The exclusion is typed, not heuristic.** Pasted and stringized tokens have no contiguous
  source text, and `TokenOrigin::Synthesized` names exactly that set — so the test asks the type
  rather than guessing which tokens look odd. A second test pins the excluded set to *precisely*
  those two tokens, so it cannot quietly widen into "whatever fails".
- **The skip count is bounded, not merely reported.** A round-trip test that skipped every token
  would pass. The synthesized-set fixture initially had no ordinary tokens at all, making its
  round-trip half vacuous; `checked > 0` caught it and the **fixture** gained `int v = … ;`
  rather than the assertion being relaxed.

### 7.5 How to check the workspace is green — `./check.sh`

**Do not sum "N passed" out of `cargo test`.** I reported "0 failed" for a long stretch while
three xtask gates were red: a crate whose test *binary* fails to build emits no `test result`
line at all, so counting successes cannot detect a missing success. `./check.sh` keys on
cargo's exit status and prints the failing suites first. Current: **2191 passed, 257 suites**
(2026-08-07, after §7.11's seven waves).

⏱️ **It now takes over an hour per leg**, and that is the session's dominant cost — see §9's
note on the corpus. `conversions` and `semantics` are ~55 s each, the two VPP gates ~60 s, and
every corpus-consuming *binary* rebuilds the 22-seed analysis from scratch because Rust
integration tests are separate processes.

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

### 8.3 🔁 THE WIDENING PATTERN — the standing job, and the highest-yield loop found so far

**Every gate has a corpus, and every corpus has an edge. The defects live past the edge.**
This has now paid out three times in a row, each time on the first run after a widening:

| widened | cost | yield |
|---|---|---|
| `find-bugs`: 7 files → 56 → 92 plugins | a sweep each | 4 defects, then 3, then two panics and a true `Exact` |
| `layout`: no bit-field records → runs modelled | one wave | 2 VPP findings, and a review found a `proven` wrong answer inside the fix |
| contract-12 gate: 20 `vppinfra/` seeds → +1 `vnet/` seed | 86 files, ~5 min | **2 layout defects + 1 in the gate itself; 5482 → 8492 assertions** |
| chasing one failure message out of `vlib/` | one wave | no defect — gcc agrees with chiero — but the message now names the type, which is what turned a compile into a read |
| the *same* seed, reaching 014 contract 11's conversion census | free | the census was asking `&&`/`||` a question C does not ask; 10 false offenders |
| 013 contract 19's parse corpus: 6 `vppinfra/` seeds → +1 `vnet/` seed | free, the corpus was already there | **zero defects** — parses clean, 0 diagnostics, memory ratio 1.74x against a 10x bound. Coverage +45% in tokens and a new subsystem. An honest zero, recorded because a table of only wins cannot say when to stop |
| both corpora → `+ vlib/vlib.h` | **free** — its whole 67-file closure was already there | **zero defects.** Layout gate 8492 → **10248 assertions**, 2238 records; parse 357k tokens, 0 diagnostics. It is the seed that reaches `vlib/trace.h`, so that header is now under the gate rather than only in an error message |
| **a new *kind* of edge: simplecpp's `testsuite/`, 141 C preprocessor cases** | one wave | **the richest yield yet — see §7.11.** 22 findings on the first run including **2 panics on the C standard's own worked example**. Every corpus before it was real VPP code |
| *reading what that gate left behind*, rather than what it failed on | one wave | §7.11: the compiler-persona defect — `__has_attribute` claimed the capability and answered 0 to everything, silently. **The residue of a gate is a corpus too** |
| the same gate, one `Todo` row — but **measuring every row of the file, not the one it named** | one wave | §7.11: **two opposite defects behind one guard**. The gate had pointed at the wrong row; the reported one was already correct |
| `differential.rs`'s logical-operator rows: `int`-only → wider-than-`int`, mixed types, four-way short-circuit | one wave | **zero defects — and the wave was mis-scoped.** Most of what I "widened" was already covered, including the exact `-0.0` case I had picked as the discriminator. §8.3 step 1 says *ask what the corpus cannot contain*; I asked what I imagined it could not |
| the same gate's **remaining findings, read as a to-do list** rather than as noise | one wave | §7.11: chiero was rejecting **valid C11** — a UCN in an identifier. **51 spurious diagnostics gone.** The corpus paid a fifth time, in a fifth different way |
| **sharpening an oracle** — a feature query's value rather than its truthiness | one wave | §7.11: six rows of the table shipped two waves earlier were wrong, and the old test agreed with every one. The corpus was not widened at all; the *instrument* was |
| following the **diagnostics the tool itself emitted** — three names it said it did not know | one wave | §7.11: scoped operands + `__has_cpp_attribute`; findings 16 → **14**, and `pr63831-1/2` leave the list. The honesty mechanism turned the next fix into a mechanical one |
| find-bugs to a **new subsystem**: `vnet/ip/`, 152 entries (never swept) | one sweep, ~20 min | **zero chiero defects — 0 failed, 0 `Exact`.** 143 ok, 7 cut, 9 `Unknown` findings, all one known class. See §7.17 |
| **010 contract 11**, the one contract with no test anywhere — verified by reading, not by trusting §9's note | one wave | **zero defects.** The round trip holds over twelve fixtures incl. splices, macro-body/argument spans and the session's new UCN identifiers |
| **the plugin sweep, one entry per file → three** (477 → 1320) | one sweep, ~65 min | **91 findings against 18**, 3 `Exact` against 1 — and the yield was a *reporting* defect: a `proven: true` null dereference resting entirely on a global's initial value, with the premise unstated. Also 31 `failed` rows resolved to **six** causes, 19 of them the parked `-march` item, and one that is not a chiero defect at all (below) |
| the **second** sampling round: two more `cut` entries | one wave | **an honest zero.** `active_open_alloc_session_fifos` is dominated by `BvConst` arithmetic under `TermArena::eval`, reached from the counterexample cache — which is 022 §6.2's *self-certifying* rule doing exactly what it must, and `BvConst` is a `Copy` `u128` with no allocation behind it. Recorded because a table of only wins cannot say when to stop |
| *not a widening* — **sampling a real run's stack instead of reading its code** | one 90-second run under `gdb` | `TermArena::vars_of` allocated a bool per node in the *whole arena* on every call, and 022 §6.2's slicing calls it once per constraint on **every** backend query. 8.3 µs → 699 µs as the arena grew; now flat. **Nothing in the code reads as wrong** — the defect is a call pattern, and only a profile shows it |
| *not a widening* — **taking a `timeout` row seriously instead of counting it** | one sweep + one stack sample | the CIR verifier was **super-quadratic**: 11.5 s for 3001 blocks, and it is what killed VPP's last two `timeout` entries. 023 §8 had attributed them to a long *solver* query and specified a bound for that; the bound did not move them. **42x faster, 0 `timeout` rows left, one spec claim retracted** |
| *not a widening* — **re-measuring the pinned 40 and asking why nothing moved** | one retake + one `grep` | the corpus **cannot reach** a 32-byte access: `__AVX2__` is undefined in every configuration chiero compiles, so every AVX2/AVX512 path in vppinfra is invisible to every measurement this project has published. New evidence for the parked `-march` item, and it came from an *unchanged* number |

The loop, and it is deliberately mechanical:

1. **Find the edge — by *reading the corpus*, not by imagining it.** Ask what it *cannot*
   contain. `CORPUS_SEEDS` was twenty headers from one directory — so no unnamed bit-field, no
   typedef-attribute, nothing `vlib` does. The question is never "is the gate green", it is
   "what is the gate incapable of seeing".
   ⚠️ **Grep the corpus for the construct before claiming it is absent.** A wave on 2026-08-07
   was scoped around `-0.0 && 1` as the case nothing covered; two tests had pinned it in an
   earlier wave, under names that did not contain the word "logical". Cost: a whole wave to
   re-prove what held. `git stash` + grep the *pre-change* file is the five-second version.
2. **Widen by one.** One seed, one directory, one file class. One, because the next step is
   reading failures and a wide widening produces a pile nobody can attribute.
3. **Read what it rejects, and do not fix it yet.** The first run's failure list is the
   measurement. Record the count before touching anything — it is the number the wave is
   judged by, and a fix applied before it is recorded destroys it.
4. **Fix red-green, one defect at a time**, checking each answer against gcc rather than
   against arithmetic. Re-run the gate after each: the list shrinks, and sometimes *grows*,
   which is information — §7.10's 1 became 11 because the first fix let later assertions run.
5. **Land the widening only when the *whole suite* is green — not the gate you widened.**
   ⚠️ Learned by getting it wrong on 2026-08-07: the contract-12 gate was green, the widening
   was committed, and `./check.sh` then went red on 014 contract 11's conversion census, which
   reads the **same corpus** and had never seen an `&&` mixing signedness. A corpus is shared
   by every test that consumes it, so widening it is never local to one gate. Wait for the full
   run. A widening committed red makes every later run unreadable; keep the corpus copy out of
   the commit until then, and §9 carries the recipe so an unfinished one costs five minutes.
6. **Record the yield** in the table above, so the next reader can see whether the pattern is
   still paying and stop when it is not.

⚠️ **A third form, and the one that had been true longest: a gate narrower than the gate that
matters does not warn you, it reassures you.** `./check.sh` exists because an earlier check
"could not fail", and it then reported GREEN while `cargo fmt --all --check` had **26 diffs**
and `clippy -D warnings` **two errors** — because CI runs three legs and it ran one. The script
was honest about *what it measured*, and the word it printed was "GREEN". **When a local gate
and a remote one disagree about scope, the local one is the one that lies**, because it is the
one somebody trusts before pushing. Widened 2026-08-07; the one leg still missing is named in
the file itself. **It paid twice within the hour of being written** — an unformatted line on the
first real use, then two `clippy -D warnings` errors on the second, both in code committed
minutes earlier and both caught in seconds. The old script would have called each of those runs
GREEN after an hour of tests, and CI would have refused the push.

⚠️ **And its twin: an *unchanged* number is evidence about the corpus too.** Retaking the pinned
40 after `BadRange` left the defect list gave byte-identical numbers. The tempting readings are
"the change did nothing" and "the change was safe"; the true one is that the string
`unsupported-access-width` occurs **zero** times in all forty envelopes, because `__AVX2__` is
undefined in every configuration chiero compiles and every 32-byte type in VPP is behind it. A
number that does not move has said nothing until you know whether it *could* have moved — and
the only reason that took one command rather than a wave is that `measure.sh` now keeps its
residue (`KEEP=<dir>`). **When a re-measurement comes back identical, go and find the thing you
expected to change.**

⚠️ **The trap: a green gate is evidence about the corpus, not about the tree.** `vpp_layout_gate`
passed for months while 014 mis-aligned every unnamed bit-field, because no seed had one. Before
concluding "the tree does not do this", check whether the gate could have seen it if it did.

### 8.2 ⚠️ Never `git add -A` while another agent has the tree

It swept a reviewer's throwaway test files into a commit once and mixed a finished fix into the
*next* RED commit twice — a commit labelled `red:` that contains the green is a lie to whoever
reads this history next, and this history is the record. Stage the paths you edited:
`git add crates/chiero-lower/src/lib.rs …`. Rebuilding three commits to undo it cost more than
typing the paths ever would.

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

> ### ⏭️ START HERE — **§8.3's widening pattern is the standing job, and the heartbeat runs it.**
>
> Read **§8.3** first: it is the loop, its yield table, and the trap that let a defect survive
> for months (*a green gate is evidence about the corpus, not about the tree*). Then §9.1 for
> the next target.
>
> ✅ **The owner's close-the-gap ask is DONE — pp-gate reports 0 findings** (§7.11). Keep it as a
> two-minute standing check.
>
> ✅ **`MemFault::BadRange` is CLOSED (2026-08-07)** — it degrades now instead of reporting.
> Next: §9.1's remaining live items. The `-march` item stays parked for the owner.
>
> 🆕 **The newest entry in the yield table is the most useful one: change the *kind* of corpus,
> not its size.** 141 preprocessor torture cases found three defects in a session, two of them
> panics on the C standard's own worked example, after four consecutive widenings of VPP-shaped
> corpora had begun returning honest zeros. §11.3 carries the general form.
>
> **State: 2026-08-07 — `./check.sh` GREEN at 2228 passed across 264 suites, fmt and clippy
> clean, and the gate now takes **3m56s** rather than 6m51s.** That is the first run verified against all three CI legs; the tree had been **red in
> CI** (26 fmt diffs, 2 clippy errors) while the old one-leg script called it green. Closed this
> session: `MemFault::BadRange`; 023 §8's `max_solver_rlimit` and `max_memory_objects` — with
> which **every budget in that sketch is built**; `--solver-rlimit` on the three commands that
> run a solver; and — the biggest of them — **the CIR verifier's super-quadratic `dominators`**,
> which was what VPP's last two `timeout` rows actually were. The plugin sweep now has **zero**
> `timeout` rows.
>
> *Earlier in the session, at 2215/263, measured after the `BadRange` closure:* Up from 2154 at the previous session's start and 2193/258 before the
> last two preprocessor closures. Earlier in the session: §7.11's seven waves over the
> preprocessor conformance corpus (nine defects), honest zeros on 014 contract 11, `vnet/ip/`
> (§7.17) and 010 contract 11 (§7.18), then `push_macro`/`pop_macro` and `#pragma GCC
> error`/`warning`. The contract-12 layout gate is 22 seeds / 2238 records / **10248 assertions
> put to gcc**; pp-gate is 141 C cases, **106 agree**, +11 matching one compiler where the two
> disagree, +21 correctly rejected, 3 rendered-differently, and **0 findings**.
>
> ✅ **Both legs measured, and re-measured at the pushed HEAD: 2228 passed, 0 failed each.**
> The full three-leg `./check.sh` takes **6 m 51 s** wall clock — the first time this project has
> a number for its own gate, which is what any argument about the corpus runtime has to beat. Worth knowing for the clock budget:
> with a warm build it took **under ten minutes**, not the hour §9 warns about — that hour is
> a cold build plus the solver leg's z3 round trips, and the no-solver leg skips those.
>
> ⏱️ **Budget the clock.** A full both-legs run is over an hour and dominated the last session.
> Do not start a widening and a full run in the same breath.
>
> 🧹 **This file was cut from 16169 lines to ~1500 on 2026-08-07 at the owner's request.** What
> went was §9's chronological wave log — roughly fifty superseded "here is where things stand"
> entries whose durable content had already been promoted into §7, §8 and §10. **The full
> pre-cleanup file is in git at `c94051f`** (`git show c94051f:HANDOFF.md`) if a wave's detail is
> ever needed. §11 below is the harvest: the cross-cutting lessons that lived only in the log.

### 9.1 The queue

> ✅ **Closed 2026-08-07** — the preprocessor conformance corpus and everything it led to
> (**§7.11**, seven waves, nine defects), the compiler persona (012 §4.1 is normative), GNU
> comma-swallow, 014 contract 11's lowering half (an honest zero), `vnet/ip/` swept (**§7.17**,
> an honest zero), and 010 contract 11 (**§7.18**, an honest zero). The simplecpp corpus is
> **harvested** — see §7.11's assessment; keep `pp-gate` as a two-minute standing check.
>
> ⚠️ **Two consecutive honest zeros before this file was last written.** §8.3's yield table shows
> the pattern's returns flattening on the surfaces that remain. That is a signal to act on, not
> to rediscover: prefer the concrete items below over another widening, and if you do widen,
> **read the corpus first** (§8.3 step 1) — a wave was mis-scoped today by asserting an edge
> without checking what was already covered.

1. **⏸️ PARKED at the owner's request 2026-08-07 — `-march`.** Do not start without checking in;
   the owner asked to discuss the design first. What was agreed: the *flag propagation* half is a
   bug regardless (chiero probes the compiler with no flags while the sweep replays real ninja
   lines, so it preprocesses a different configuration than the one that ships), and the
   intrinsics half needs the 7-second `probe.sh` run before anyone designs it — the real first
   error may be `#pragma GCC target`, not intrinsics at all. And it is probably not "add a
   `--march` flag": VPP's multiarch compiles files repeatedly under different `-march`,
   per-function, so the target configuration is per-TU. Seven of the 11 `failed` plugin entries
   are one cause: `frontend::predefines` asks gcc for macros with **no `-march`** while VPP builds
   `-march=x86-64-v2`, so `__SSE4_2__` is undefined and `vppinfra/crc32.h` never defines
   `clib_crc32c_with_init`. The other four are two parser/sema gaps in generated API headers.

   🆕 **Measured 2026-08-07, and it makes the item bigger than those seven entries.** Retaking the
   pinned 40 after `BadRange` left the defect list gave byte-identical numbers, and the kept
   envelopes say why: `unsupported-access-width` occurs **zero** times in all forty — the corpus
   cannot produce a 32-byte access at all.

   | | |
   |---|---|
   | every 32-byte type in VPP is in `vppinfra/vector_avx2.h` | `vector.h:197`, under `#if defined (__AVX2__)` |
   | `__AVX2__` needs `-march=x86-64-v3` or `-mavx2` | `gcc -dM -E`: defined at `v3`, **not** at `v2`, not with no `-march` |
   | VPP's baseline is `-march=x86-64-v2` | so even VPP's own default build has none of it |
   | the AVX2/AVX512 paths are the **multiarch variants** | which is exactly this item's per-TU target configuration |

   So it is not only that seven entries fail to preprocess. **Every AVX2 and AVX512 vector path
   in vppinfra has never once been compiled by any chiero measurement** — including the code
   021 §5 cites when it says "vppinfra uses `u8x32`/`u8x64` throughout". Every "0 findings" this
   project has published over VPP is silent about that half of the tree. Worth putting to the
   owner when the item is unparked; it does not change the recommendation to design first.

2. **Unwidened surfaces, in rough order of expected yield** (§8.3 is the loop):
   - ~~The sema corpus gate's contract 11 and contract 19~~ — **both wrong, and both closed.**
     They are **010**'s contracts, not sema's. 19 was already covered by
     `chiero-span/tests/config_sites.rs`; 11 genuinely had no test and now does (§7.18), and it
     holds. ⚠️ **This entry named the wrong spec and claimed a gap that was half fictional** —
     `grep 'Covers:' crates/*/tests/*.rs` settles such a claim in one command.
   - ~~014 contract 11's `&&`/`||` half~~ — **closed 2026-08-07, an honest zero.** Lowering does
     call `truth_of` per operand and gets every case right; three genuinely-absent rows landed in
     `chiero-lower/tests/differential.rs` (an operand wider than `int`, operands of different
     types, four-way short-circuiting with a side effect). ⚠️ **The wave was mis-scoped** — most
     of what I planned to add was already covered, including the `-0.0` case I had named as the
     discriminator. Read the corpus before asserting its edge (§8.3 step 1).
   - `vnet/ip*` and `plugins/*/` beyond one function per file are untouched by the find-bugs
     sweep; `pick_entries.py --per-file N <files>` takes a list.

3. 🆕 **`compile_commands.json` is one command away, and two contracts have been blocked on its
   absence since M2.** `docs/reviews/m2-frontend-notes.md` records *"`…/compile_commands.json`
   does not exist, and `find /home/ubuntu/vpp -name compile_commands.json` returns no
   alternatives. Contract 17's full configured-TU regression metric therefore cannot run in this
   environment."* That was **true when written** — VPP was not built yet — and it is a stale
   blocker now, not an error.

   Measured 2026-08-07: `ninja -C $VPPBUILD -t compdb` emits the whole database in **90 ms**,
   **6235 entries, 2226 of them C**, with the `command`/`directory`/`file`/`output` keys the
   format specifies. Nothing needs to be re-configured and the VPP tree needs no edit — the
   generator writes to stdout.

   What it unblocks: **012 contract 17** (`every_vpp_compile_command_preprocesses_without_panicking`
   in `chiero-pp/tests/directives.rs` is `#[ignore]`d *and* returns early on the missing file —
   two ways of measuring nothing, stacked) and **060 contract 1** (every TU yields a `ConfigId`).
   ⚠️ Neither is free: the sweep already covers "1871 VPP TUs lower", so the value is the
   *configuration* half — per-TU flags, and 060's multiarch 1:N — not the parse. Decide which
   contract is actually being bought before writing the ingest.

   ⚠️ And the general form, because it is the third stale blocker this file has produced:
   **a blocker records the world at a moment. Re-measure one before spending a wave routing
   around it** — this one cost nothing to check and had been standing for months.

4. ✅ **Mostly answered 2026-08-07 by a config block, and the entry below is kept for what is
   left.** The whole item was written around `conversions` taking 53 s — which is the **default
   dev profile**, not the code. The same test takes **7.95 s** built by release, and
   `[profile.dev]` had no tuning at all. Setting `opt-level = 2` (with `debug-assertions` and
   `overflow-checks` pinned on) takes the full three-leg `./check.sh` from **6m51s to 3m56s**,
   both warm — 43%, about three minutes a run, against a one-off 7m12s rebuild.

   ⚠️ **The first comparison was contaminated**: 6m14s, taken after editing `chiero-solver`, so
   it carried a partial rebuild at the new opt-level. The tell was `user` time going *up* while
   wall time went down. Re-take a timing when anything has been rebuilt in between.

   ⚠️ **And the experiment turned the suite red for a reason that was not the experiment** — the
   slicing ratio test was load-sensitive and only a full-workspace run had ever exercised it
   under load. Fixed with an exact counter; see §11.1.

   *Still open, and now the smaller half:* each test binary still rebuilds the corpus from
   scratch, because Rust integration tests are separate processes. Serialising the analysis to
   disk is the remaining idea, and **020's CIR text format may already be most of it** — but the
   43% is banked, so the case for that work is now three minutes, not ten.

4b. **⚠️ Original entry, for the record.** Every corpus-consuming test
   preprocesses, parses and analyses all 22 seeds. Sharing one `PreprocessorSession` across seeds
   bought ~15% (`conversions` 62→53 s, `vpp_layout_gate` 62→58 s, `vpp_corpus` 16→13 s) — **less
   than hoped, and it says where the cost is**: not lexing, but parse and analyse, which no cache
   touches. The remaining structural waste is that each *test binary* rebuilds the corpus from
   scratch, because Rust integration tests are separate processes, so `corpus_analyses()` cannot
   be shared across them however it is written. Cutting it needs the analysis serialised to disk
   and reloaded — a real design question, and **020's CIR text format may already be most of the
   answer**.

5. ✅ **CLOSED 2026-08-07 — "a step that outlives the clock" had the wrong cause, and the sweep
   now has zero `timeout` rows.** The entry said three find-bugs entries needed the outer
   `timeout` because "the clock is only checked *between* steps", and 023 §8 named
   `max_solver_rlimit` as the bound for them.

   ⚠️ **Neither `--solver-rlimit` nor `--time-budget` moved them at any value.** The two rows —
   `plugins/unittest/fib_test.c` and `llist_test.c`, named for the first time because the old
   numbers recorded only a count — were spending their time in `chiero_cir::verify::dominators`,
   which runs **before a single instruction executes**. No clock, no solver, so nothing 023 §8
   defines could ever have reached it. `chiero layout` on the same file, frontend only, finished
   in 1.3 s, which is what said the frontend was innocent too.

   The verifier was super-quadratic in the block count: **11.5 s for 3001 blocks** in a release
   build, 158 s in a debug one, each doubling costing about six times the previous. `dominators`
   rebuilt the predecessor list *inside* the fixpoint loop and met dominator sets with
   `retain(|x| dom[p].contains(x))` — linear in a set that starts as every block in the function.
   Now 270 ms at 3001 blocks; both VPP entries are `ok`; 023 §8's attribution is **retracted in
   the spec**, not quietly edited.

   **How it was found, because the method is the reusable part:** the stated cause was *tested*
   (does the proposed bound cut these rows? no), then a stack sample under `gdb` named the
   function. ⚠️ `ptrace_scope=1` on this machine blocks `gdb -p`; run the program **as gdb's
   child** and `pkill -INT` it from a background subshell —
   `gdb -batch -x cmds --args ./target/release/chiero …` with `run` then `bt 18`.

   ✅ *And the original entry's last leftover closes too:* **`max_memory_objects` is built**
   (2026-08-07), so **every field in 023 §8's budget sketch now exists** — the first time that
   has been true. It bounds one *path* where `max_states` bounds paths.

   ⚠️ **Enforced between steps, deliberately, and the enumeration is why.** Objects are minted
   from eleven sites in `chiero-exec` *and* from every model in `chiero-model` via
   `ModelCtx::mem`, which `chiero-vpp` extends — no call site sees them all, so a check at the
   allocations would be a check at some of them. The cost is that the count can pass the limit
   by one step's worth (measured: 13 against 12), which is `max_forks`'s shape and is stated in
   the spec rather than glossed.

5a. ✅ **DONE 2026-08-07 — the verifier's scale test asserts a counter, not a clock.** It had
   asserted 5 s, chosen under the unoptimised dev profile; `opt-level = 2` made every build about
   6.7x faster, the bound stayed, and a mutant restoring **one** of the eight removed scans came
   in at 4.60 s and passed. **A wall-clock assertion silently weakens whenever the build gets
   faster** — nobody edits the test, it just stops being able to fail.

   `verify::terminators_examined()` counts what actually differs: examining every block's
   terminator **once per function** is linear, **once per block** is quadratic. The same mutant
   now reports **144 108 008 against a bound of 240 020** — a factor of 600, identical on any
   machine, at any load, under any profile.

   ⚠️ **The design point, and it is the one I had already got wrong once that day.** The counter
   has to attach to the *scan*, not to the site a fix happened to hoist it to. Counting
   "predecessor maps built" would have gone **down** under the mutant — which stops building one
   — and the test would have passed. Every `successors()` call in `verify.rs` goes through one
   counted wrapper, so a per-block scan increments per block by construction.

   The duration survives as a loose 30 s smoke check, explicitly *not* the assertion, so a
   catastrophic regression fails fast instead of hanging the suite.

5e. 🆕 **A witness of 10 658 bindings — `nsh_md2_encap`, and it is a *reporting* defect.**
   Chasing 5c's `timeout` rows found this. The entry does finish, in **2 m 22 s against a 120 s
   budget**, and emits **950 KB of JSON for one finding**. Its witness is **10 658 bindings,
   10 657 of them the same anonymous label "a lazily-materialized byte"**.

   023 §9 calls a witness *a concrete input someone can re-run*. Ten thousand unnamed bytes is
   not one: it cannot be read, cannot be typed into a harness, and it is most of both the runtime
   and the output. Under UCSE an entry that walks a packet buffer materialises a byte at a time,
   so the *execution* is probably fine and the reporting is not.

   The fix has to bound the rendered list **and say what it left out** — a quietly shortened
   witness reads as the whole input and is worse than a long one.

   ```sh
   ./target/release/chiero find-bugs $VPP/src/plugins/nsh/nsh_node.c --entry nsh_md2_encap \
     --time-budget 120 $INC -I$VPPBUILD/vpp/CMakeFiles/plugins/nsh $DEF --entry-ptr-nonnull --json
   ```

   ⚠️ **A RED was attempted and thrown away, which is the useful part of this entry.** A `.cir`
   fixture looping over a symbolic entry pointer produced **one** binding and a 1 170-byte
   envelope — nothing like the VPP case. Whatever mints ten thousand *distinct* lazy bindings is
   not "read bytes through a symbolic pointer in a loop", and **finding out what it is comes
   before writing the test.** Committing that fixture would have been a test passing for the
   wrong reason, which is the failure this file has recorded three times today.

5c. 🆕 **Three `timeout` rows in `plugins/nsh/`** — `format_nsh_header`, `nsh_md2_decap`,
   `nsh_md2_encap`, from the widened sweep (2026-08-08). The verifier fix removed the cause the
   *old* `timeout` rows had, so this is a different one and nobody has looked. Sampling the stack
   under `gdb` found the last one in about two minutes; §11.2 carries the invocation and the
   `ptrace_scope=1` workaround. ⚠️ A `timeout` row is a run that measured **nothing** — it is a
   lead, not a statistic.

5d. ⚠️ **The VPP build directory is STALE, and it affects every number this project publishes.**
   `src/plugins/lldp/lldp.api` declares `f64 last_heard_age;` and was modified at 23:32:08 on
   2026-08-05; the generated `lldp.api_types.h` it compiles against was produced at **23:14:37**,
   seventeen minutes earlier, and the field appears in **no** header under `build-root`. Three
   sweep rows fail on it, chiero is right, and **gcc reports the identical error at the identical
   line** — so it is an environment fact, not a frontend gap.

   The consequence is larger than three rows: every measurement against VPP analyses `src/` with
   *those* headers, so wherever the source has moved on, chiero reads a slightly different
   program from the one VPP would build today. Regenerating the build directory is the fix.
   ⚠️ Until then, a `failed` row naming a missing struct member is staleness and must not be
   chased as a defect — it looks exactly like one.

5b. 🆕 **Audit `Vec` + `.contains()` on paths that scale — the shape, not the site.** The
   verifier fix above is the **second** time this exact defect class has been found in
   `crates/chiero-cir/src/verify.rs`. Seven hundred lines above `dominators`,
   `check_module_identity` already carries: *"Sets, not vectors. These were `Vec` with
   `contains`, which is O(n^2) — invisible while a module held dozens of entities… one measured
   673 s against ~1 s before. The scaling was the giveaway."* Methodology and all. **The fix went
   to the function where the symptom appeared and its neighbour in the same file had the same
   flaw.**

   `grep -rn "\.contains(&" --include=*.rs crates/*/src xtask/src | grep -vE "IndexSet|IndexMap|BTreeSet|HashSet|BTreeMap|HashMap"`
   returns **87** sites. Most are ranges (`(0x300..=0x36F).contains`) or genuinely small fixed
   lists; the dangerous ones are where the receiver **grows with the input** and the call is in a
   loop. Spot-checked `chiero-gcov`'s four (`note_test`, `note_variant`, the per-line dedupes):
   all bounded by *test count* rather than line count, so O(T²) at worst and not obviously the
   next 673-second bug — ⚠️ but that is a reading, not a measurement, and this file's record on
   readings is poor. **Do it with a growth curve** (`/tmp/benchdom`-style: time the operation at
   10/40/160/320/640 and look at the ratio per doubling — 4x is quadratic, 6x is worse), because
   the ratio is what makes it undeniable and a single timing never is.

   **Triage done 2026-08-07; the interesting half is blocked.** By crate: `chiero-cir` 15,
   `chiero-gcov` 13, `chiero-sema` 9, `chiero-exec` 6. Most hits are ranges or fixed lists. The
   two that look genuinely quadratic are both in `chiero-gcov/src/native.rs`, and both scale with
   **arcs or block-lines per function**, not with test count:

   - **`native.rs:1642`** — `slot.contains(&(key.clone(), bl.block))` inside `for bl in &f.lines
     { for line in &bl.lines { … } }`. `FuncKey` holds two `String`s, so **every probe allocates
     twice** purely to compare, and the enclosing `entry((bl.file.clone(), *line))` clones a third.
   - **`native.rs:1656`** — `order.contains(&(a.from, a.to))` while `order` is being filled from
     `f.arcs`, i.e. O(arcs²) per function.
   - Johnson's circuit enumeration (`native.rs:1258–1286`) uses `Vec::contains` for `bs`,
     `blocked` and `block_lists`, which adds a factor to something already expensive.

   ✅ **`chiero-cir`'s half is DONE (2026-08-07) and it took two passes, which is the lesson.**
   The first removed `dominators`' scan: 3001 blocks 11.5 s → 270 ms, and I called it fixed.
   **The ratio had not moved** — still ~4x per doubling — so only the constant had. Reading the
   file for the *shape* then found **seven more**, including `check_phis` rebuilding a
   predecessor map per block, which is the identical defect to `dominators`' one function away.
   `reachable_blocks` returning a set fixed three at once; a linear `Function::block` find was
   an eighth, hiding behind a method call rather than behind a `contains`. 30721 blocks: hours
   → **2.4 s**.

   ⚠️ **And it is still quadratic** — 4x blocks, ~15x time. The scans are gone; what remains is
   `dominators` holding an explicit dominator *set* per block, O(blocks²) by construction.
   Cutting it needs Lengauer-Tarjan's idom-only form or bitsets: **a design change, queued not
   claimed.** Worth doing only if a real VPP function turns out to be large enough to care.

   ⛔ **`chiero-gcov`'s half is blocked on artifacts, and this is why it is recorded rather than
   fixed.** There are **no
   `.gcno` files under `/home/ubuntu/vpp`** — the 1895-file validation in §7.1 was a one-off
   against a coverage build that no longer exists. Without it there is no growth curve, and this
   entry's own rule says a reading is not a measurement. Two honest ways forward: rebuild VPP with
   coverage (long), or write a synthetic `.gcno` generator and curve it the way `dominators` was
   curved (bounded, and reusable afterwards). ⚠️ **Do not "just fix" the clones** — an unmeasured
   optimisation is the flattering change this file keeps warning about, and `chiero-gcov` is
   19/19 contracts green today.

   ✅ **`max_solver_rlimit` is BUILT, 2026-08-07.** `Budget::max_solver_rlimit` reaches the backend
   as `(set-option :rlimit N)`; a query that spends it answers `Unknown(ResourceLimit)`.
   `Engine::new_solver` is the single construction point, so it covers feasibility *and* checker
   queries — a budget that applied to one and not the other is not a budget.

   **The defect it uncovered is worth more than the feature.** `query` returned
   `Option<(bool, Model)>`, so a solver saying `unknown` and a broken pipe were the same `None`,
   and `ask_backend_raw` treats `None` as died-mid-query: it **replayed the whole query** and
   reported `BackendError`. So the hardest queries in a run — the only ones that answer `unknown`
   — were charged twice, and 022 contract 15's `backend_errors` counted a backend that was
   behaving correctly. `Answer` is a three-valued type now so the two cannot share an arm again.

   ⚠️ **And the mutation pass caught me shipping the exact confusion the tests' own header
   describes.** With the first three tests, `if true` in place of the classification guard —
   making *every* `unknown` a `ResourceLimit` — **survived all of them**. Closing it needed a
   fake solver answering `unknown` with a chosen reason; z3 cannot be made to decline a theory
   on demand, and that is a property of the z3 build rather than of chiero.

   ✅ **`--solver-rlimit` shipped 2026-08-07**, on `find-bugs`, `check-reachable` and
   `prove-equivalent` — and writing it found that the wave above had reached **one of three**
   solver construction sites.

   ⚠️ **The commit that built the budget claimed `Engine::new_solver` was "the single
   construction point" and invoked *fix the rule, not the site* — while missing two sites and a
   whole command.** A run builds a solver in three places: `Engine::new_solver` (feasibility and
   checkers), `chiero-tool::witness_for_path` (`check-reachable`'s witness, built *outside*
   `Engine` because a state that merely arrived carries no finding), and `chiero-opt::equiv`.
   Only the first was wired, and the CLI never set the equivalence budget at all. **Saying "one
   construction point" is not the same as making one** — `grep -rn "TieredSolver::" --include=*.rs`
   is the four-second check that settles it, and it was not run.

   The defect was invisible to every envelope field. What found it: a recording script as
   `$CHIERO_SMT_SOLVER`, showing `(set-option :timeout 9000)` on the wire and **no `:rlimit`**.

   **Two fixture traps worth keeping, both of which make a budget test vacuous:**
   - **`x * 2` against `x << 1` — 041's own headline example — never reaches a backend.** Tier 1
     settles it. A test built on it passes whatever the plumbing does. Count dumped queries
     (`CHIERO_DUMP_QUERIES`) before believing a solver fixture exercises a solver.
   - **At `:rlimit 1` z3 cannot even run `(push 1)`**, and emits an `(error …)` line that chiero
     reports as "backend gave no usable answer". Honest, and a different sentence from the one
     under test. Use 2000.

   **Both leftovers are closed.** `max_memory_objects` shipped the same day, and the plugin
   sweep's `timeout` rows were re-measured — they had nothing to do with the solver, which is
   the entry two above this one.

   *The measurements that shaped it, kept because they are about z3 rather than about chiero:*
   `UnknownReason::ResourceLimit` existed and was **constructed nowhere**; nothing read
   `(get-info :reason-unknown)` at all. What the real solver does:

   | asked | answered |
   |---|---|
   | `(set-option :rlimit 1000)` on a hard `bvmul` | `unknown`, `(:reason-unknown "max. resource limit exceeded")` |
   | the same at `:rlimit 100000000` | `sat` — so the bound is what cut it, not the formula |
   | a hard query, then a trivial one, **one process** | `unknown` then `sat` — **`:rlimit` is per-`check-sat`, not cumulative.** This is the property that makes it usable at all: chiero keeps one long-lived process, and a cumulative budget would poison every query after the first expensive one |

   ⚠️ **And the trap, which the obvious implementation walks straight into.** The documented
   string only appears with the assertion stack at top level. **Inside `(push)`/`(pop)` — which
   is how chiero *always* drives z3, since `Solver` has `push`/`pop` — the same exhaustion
   reports `"canceled"`.** Worse, a `:timeout` firing under `push`/`pop` reports `"canceled"`
   **too**, byte for byte. Measured both ways round.

   So `(get-info :reason-unknown)` **cannot distinguish a resource limit from a timeout in the
   mode chiero runs in**, and an implementation that matches `"max. resource limit exceeded"`
   passes a hand-written smoke test and misclassifies every real query. The design that follows:
   **infer `ResourceLimit` from which budget was armed, not from the string** — when
   `max_solver_rlimit` is set, do not also arm `:timeout`. That is what 023 §8.1 already wants
   anyway (CI runs the determinism gates with `wall_clock: None`), so the constraint and the
   spec agree. **That is what shipped**, and `Session::spawn` is where the exclusivity lives.
   One more measurement completes it: genuine incompleteness reads
   `"smt tactic failed to show goal to be sat/unsat (incomplete (theory arithmetic))"`, never
   `"canceled"` — so the string separates *a limit* from *a theory declined*, and only the
   armed budget says which limit.

6. **`InstKind::Call` carries no result type**, so an indirect call's result width is whatever
   candidate ran. The arity and parameter-type filters cut the wildest cases and cannot close it;
   the engine survives the rest by degrading. The real fix is a CIR change — **135 sites**
   construct `InstKind::Call`, and the text format needs syntax for it.

7. ~~### **`MemFault::BadRange`**~~ — **CLOSED 2026-08-07.** See §7's entry. The two stated
   options were both wrong because the premise was: the probes did not need an *objectless*
   fault, only two findings agreeing on `object`, which a **shared** object gives just as well.
   Both mutants confirm the replacements. ⚠️ The lesson generalises past this item —
   **when a blocker is stated as "we need a thing of kind X", check whether the requirement is
   X or the property X was being used for.** Two waves were spent hunting for an objectless
   non-fatal fault that does not exist.

8. **032 contract 18's corpus still has no `observed` entry** and the gate correctly exits 1
   saying "NOT MEASURED". Method, learned the hard way (§7.1): **revert a historical fix's `src/`
   diff onto HEAD and run the suite** rather than hunting for a commit whose parent happens to
   fail. Two builds and ~40 minutes bought one rejected candidate the other way.

9. **`:0` bit-fields in `layout`, deliberately left open** (§7.9). A record declaring a
   zero-width bit-field still gets no padding number, because a `:0`-terminated run's cost depends
   on where the run is placed and this arithmetic sums constants. Closing it needs the run's
   allocation unit in the field description — `Field` would carry `unit_bits`, and the ideal
   layout would charge the run `round_up(payload, unit)` at a unit-aligned offset. **Worth doing
   only if `:0` turns out to be common**; a sweep of 69 VPP headers found none.

### 9.2 Standing measurement instruments — and **the ones that were lost**

⚠️ **Verified 2026-08-07: everything that lived only in a scratchpad is GONE.** A scratchpad is
per-session; the previous session's is still on disk but has been pruned. Only what was
*committed* survived. This file already carried the warning ("the scripts were lost once when
they lived only in scratch") and it happened again anyway.

| instrument | state |
|---|---|
| `./check.sh` | ✅ committed — the green gate; keys on cargo's **exit status**, prints failing suites first (§7.5). ⚠️ **Widened 2026-08-07 to all three CI legs** — it ran `cargo test` alone while CI also gates `cargo fmt --all --check` and `clippy -D warnings`, and was reporting GREEN over **26 fmt diffs and 2 clippy errors**. Fast legs run first, in seconds, so a formatting diff is not found after the hour. `--skip-lints` re-runs the tests alone. **Still not covered, and named in the file:** CI's second solver leg |
| `tests/corpus/vpp-findings/measure.sh` | ✅ committed — retakes the find-bugs numbers (~4 min pinned 40, ~25 min plugins) |
| `tests/corpus/layout/fixed_diff.py` | ✅ committed — chiero's padding floor vs gcc's minimum over every run-preserving permutation |
| `tests/corpus/layout/vpp_sizes.py` | ✅ committed — contract-12's method pointed at arbitrary headers |
| `xtask/src/replay_gate.rs` | ✅ committed — `cargo run -p xtask -- replay-gate`, corpus `tests/corpus/replay/corpus.tsv` |
| `xtask/src/pp_gate.rs` | ✅ committed — `cargo run -p xtask -- pp-gate`, ~2 min. Reads `$SIMPLECPP` (default `/home/ubuntu/simplecpp`, pinned `74a5a63`); gcc and clang are the oracle. §7.11 |
| `tests/corpus/vpp-findings/probe.sh` | ✅ **REBUILT and committed 2026-08-07.** The 7-second five-TU probe that replaces 2-hour sweeps — measured 7.3 s, all five `clean`. `REALCC=true` by default, so it asks what *chiero* makes of the build's flags without compiling. ⚠️ Its rebuild note: the object path **cannot** be constructed from the source path (CMake names an object after its position in the object library, so `src/vlib/main.c` is `…/vlib_objs.dir/main.c.o`) — match `-c <source>` in one `ninja -t commands all` dump, 63 ms for all 2945 |
| `replay-probe.sh` | ❌ **LOST.** Two-checkout historical-replay probe that restored the tree on every exit path |
| `rev5` (20 fixtures), `replayprobe` (13) | ❌ **LOST.** |

**When the next instrument is built, commit it under `tests/corpus/` in the same wave.** The
committed ones are all still here; not one uncommitted one is. `probe.sh` was rebuilt on
2026-08-07 and took under half an hour including the bug above — but the version it replaces had
paid for itself four sweeps over, so the rebuild was pure repeated cost. `replay-probe.sh` and
the two fixture sets are still gone.

📌 **What the rebuilt probe says in passing, and neither fact was written down anywhere:** the
VPP build in `build-root/build-vpp-native` is **clang**, not gcc, and every command line carries
**`-march=x86-64-v2`** — the parked per-TU target-configuration item, visible on all five lines.
There is **no `compile_commands.json`** in that build dir, despite §4.12 assuming one; `ninja -t
commands` is the route that exists.

⚠️ **`CCACHE_DISABLE=1` is mandatory** for anything replaying VPP build lines — VPP's cmake wraps
the compiler in ccache and a warm cache makes the measurement about the cache.
⚠️ **Never `cargo build --release -p xtask` while a sweep is running** — the shim execs the binary
being overwritten.

## 9.9 ⏰ THE HEARTBEAT — **running, re-armed 2026-08-07 at the owner's request**

`mcp__tttt__tttt_cron_create`, **`*/10 * * * *`**, `if_busy=wait`, currently **`cron-4`**. Its
standing job is §8.3's widening pattern.

⚠️ **Re-issued 2026-08-07 because the old prompt referred to queue items by number.** §9.1 gets
renumbered whenever an item closes, and the prompt said "item 2 is PARKED" — which, after item 1
closed and a new item 2 appeared, would have told a fresh context to skip live work and would
have said nothing about the actually-parked one. The prompt now **names** the parked item
(`-march` / per-TU target configuration) and tells the reader not to trust a number in it at all.
Any future edit to this cron must keep that property: *a number in a standing instruction is a
reference that rots silently.*

**Ten minutes, not thirty**, and the earlier reasoning here was wrong rather than merely
superseded. It argued that a short tick "interrupts mid-task more often than it produces work"
— but `if_busy=wait` *defers* rather than interrupting, so a short interval costs nothing
during a task and only decides how long the session sits idle after one ends. The owner asked
for 10 minutes; the cost of a shorter tick is bounded by the deferral, and the benefit is less
dead time between waves.

Use the **`tttt`** scheduler, not `CronCreate`: §9.9b explains why (a `/clear` destroys
`CronCreate` jobs, which are session-only). ⚠️ Check `mcp__tttt__tttt_cron_list` before
creating — a second heartbeat just doubles the wake-ups.

## 9.9b ⏰ How to re-arm it if it is lost

`mcp__tttt__tttt_clear_and_read_handoff_md` does a `/clear`, which **destroys the heartbeat**:
`CronCreate` jobs are session-only, held in memory, and wiped along with the context. This
already happened once this session — the owner had to point out the heartbeat was gone.

**Re-arm it immediately after refreshing**, with `CronCreate`:

- cron: `13,43 * * * *` — every 30 minutes, off the `:00`/`:30` marks
- recurring: `true` (auto-expires after 7 days)
- prompt: *"Heartbeat. Continue autonomous work on chiero-rs per HANDOFF.md §9 'Next actions' —
  read it if you don't have it in context. Full autonomy was granted 2026-07-27; don't ask
  permission, just continue the queue in red-green TDD and commit. If you are mid-task, ignore
  this and keep going. Before any context refresh, update §7/§9 of HANDOFF.md and commit it."*

~~30 minutes was chosen deliberately~~ — superseded, see §9.9: with `if_busy=wait` the tick
cannot interrupt a task at all, so the interval only sets idle time between waves. It is 10
minutes now, and the `CronCreate` recipe above is the fallback for when the `tttt` scheduler is
unavailable — prefer `mcp__tttt__tttt_cron_create`, which survives a `/clear`.

⚠️ **Check `CronList` before creating** — if a heartbeat is already there, a second one just
doubles the wake-ups.

## 10. Standing reminders

- Don't re-ask the three §2 decisions.
- Don't clone VPP; it's at `/home/ubuntu/vpp`.
- Don't design anything that links clang or z3 at build time.
- ~~Don't start implementing before the user's spec-gate approval.~~ **Approved
  2026-07-27, full autonomy granted.** Build.
- Update §7 and §9 of this file before every context refresh, and commit it. **Replace §9's
  state; do not append to it** — appending is what grew this file to 16169 lines.
- ⚠️ **Commit an instrument in the wave that builds it.** Verified 2026-08-07: every script that
  lived only in a scratchpad is gone (`probe.sh`, `replay-probe.sh`, the probe fixtures), and
  every committed one survived. This file had already recorded that lesson once. §9.2 has the
  list and the rebuild recipe for `probe.sh`.
- ⚠️ **A discrepancy between two counts is not yet a defect in the smaller one.** A "304 vs 115"
  gap was queued as a splitter bug to fix first; the fixed splitter produced byte-identical
  output, because 304 came from a looser scan that over-counted. Reproduce the gap before
  budgeting a fix for it.
- ⚠️ **Do not write a `until ! pgrep -f "<cmd>"` waiter — it matches itself and never exits.**
  The waiting shell's own command line *contains* the pattern, so `pgrep -f` finds it, the loop
  spins forever, and every status check reports the job as still running. It cost over an hour
  this session across five stacked waiters, while the run they were waiting for **had never
  started** — the first waiter blocked the `./check.sh` chained after it. §11.2 again: the
  instrument was reporting the impossible.
  **There is no need for a waiter at all**: a `run_in_background` command notifies on
  completion. If a guard is genuinely wanted, match on something the waiter cannot contain
  (a pidfile, `pgrep -x cargo`, or the output file appearing).

  ☠️ **Postscript, 2026-08-07: thirteen of them were still alive, the oldest at 2 days 17 hours.**
  They survive a `/clear` and outlive the session that made them, so they accumulate: nine from
  2026-08-05 waiting on `cargo test --workspace`, four from 2026-08-06 waiting on `measure.sh`.
  All killed.

  ☠️☠️ **Third and fourth instances, 2026-08-07, both in tooling I wrote *for* this trap.**
  Sampling a test binary needs the run interrupted, and `pkill -INT -f "conversions-fdfe"`
  matched **gdb's own command line** (`gdb --args …/conversions-fdfe…`) and killed the debugger
  instead of the test. Then the cleanup — `pkill -f "pkill -INT -x conversions"` — matched
  *itself* and killed the shell running it (exit 144).

  **The rule, in the only form that has survived contact:** `pkill -f`/`pgrep -f` match the full
  command line of *every* process including the one asking, its parents, and anything that
  merely names the target. Use **`pkill -x <comm>`** (exact process name, 15 chars) or kill an
  explicit PID. To *list* candidates without matching yourself, filter in `awk` rather than in
  the pattern: `ps -eo pid,cmd | awk '/target/ && !/awk/ {print}'`.

  ⚠️ **And the first sweep at them reported "0 remaining" while eleven were still running** —
  which is the same defect one more time, in the *cleanup*. Two causes, both worth knowing:
  the pattern was narrower than the thing it was clearing, and the check **matched itself**, so
  a live hit and the grep's own shell were indistinguishable. **A cleanup verified with the
  pattern it cleaned with cannot report a miss.** What worked: list every process whose elapsed
  time has a `D-` in it, read the command lines, and exclude the current shell by its absurd
  `441077234-` etime rather than by a pattern.

  Cheap standing check, since they cost nothing until they cost an hour:
  `ps -eo pid,etime,cmd | awk '$2 ~ /^[0-9]+-/ && $2 !~ /^441077234/ && /until/ && /pgrep -f/'`
- ⚠️ **Never edit a source file while `./check.sh` is running.** A full run is over an hour and
  cargo compiles once at the start, so an edit part-way through poisons it: `E0460: found
  possibly newer version of crate chiero_pp`, reported as `RED (cargo exit 1)` with **0 tests
  failed**. That is a build artifact, not a failure, and the run has to be redone from scratch.
  Docs, specs and HANDOFF are safe to edit; anything under `crates/` or `xtask/` is not. Queue
  the edit, or start the run after it.
- **Run the suite both ways.** `./check.sh` covers the machine as it is; CI also runs
  `CHIERO_SMT_SOLVER=/nonexistent`, and `--no-fail-fast` is required to see past the first
  failing binary (§7.8).
- **Re-measure after a fix, not only before it.** §7.6's 108 false findings were introduced by
  a fix and caught only by re-running the corpus; nothing in 2136 tests moved.
- ⚠️ **Before trusting a fixture, compute what the defect would have said on it.** §7.9's
  first bit-field test asserted the right number on a struct where the *wrong* model gives the
  same number, so it passed while the analysis was still dropping members. A fixture that does
  not discriminate is not a weaker test, it is not a test.
- ⚠️ **An assertion against prose is an assertion against the next rewording.** Two tests
  greped an envelope for "bit-field" after the sentence stopped containing it, and could not
  fail. Assert the structured fact — which records are named — not the sentence naming them.
- ⚠️ **Writing Rust through a python heredoc mangles `\`-continued string literals** — python
  eats the continuation and the run of indentation lands *inside* the string. It has shipped
  three times this session (`single-threaded`, the padding blind spot, the hole text). Escape

## 11. Lessons that kept re-paying — harvested from the wave log before it was cut

> These lived only in §9's chronological log, each written after it had already cost a wave.
> They are here because they are **about method, not about a contract**, and every one of them
> recurred at least twice. The wave-by-wave detail is at `git show c94051f:HANDOFF.md`.

### 11.1 About tests and what they can see

- **A test can pass for the wrong reason, and it happens in a recognisable shape.** Diagnosing
  the typedef-shadowing bug, three of four probes passed — `(void)word` does not begin with an
  identifier, a *local* shadow lives in its own scope, and with no typedef there is nothing to
  shadow. Only an assignment whose left operand is the shadowed name shows it. Likewise a row
  written to kill "function name declared in the wrong scope" used `g()`, and the expression path
  never asks whether a callee names a type; `g * 1;` — C's declaration/expression ambiguity — is
  the only observable difference.
- **A message-only test cannot see a wrong value.** Two mutants survived a suite because *every*
  assertion in it checked a message or its absence: dropping `.trunc()` makes `(unsigned)(15.0/2)
  * 2` evaluate to 15 instead of 14 and says nothing. **Where a change touches arithmetic rather
  than reporting, at least one row must assert the arithmetic** — `_Static_assert` is the tool.
  This recurred *inside the same function, two commits later*, in a wave whose own RED existed to
  prevent exactly it.
- **An assertion of absence needs a companion assertion that the run got there.**
  `findings().is_empty()` over files written so that absence is the property is close to asserting
  nothing. Requiring every path to terminate by *returning* is what broke it open — three separate
  defects in one sweep.
- ⚠️ **Deterministic is not the same as discriminating, and I have now written two tests that
  were the first without being the second** (2026-08-07). Replacing a flaky timing assertion
  with a counter of nodes *visited* looked like a strict improvement — and the counter is
  **identical** on the defective implementation, which allocated a whole-arena buffer per call
  but still stamped only the nodes it walked. The test passed against the very code it was
  written to catch. The discriminating counter was scratch *initialisation*: 16 003 200 against
  0. **Ask what number the defect changes, not what number is easy to make deterministic** —
  and mutate, because that question is answered by measurement and not by staring.
- ⚠️ **A wall-clock assertion is a load measurement.** The same slicing test, written as a ratio
  of two durations, passed run alone and went red under the full workspace run. A ratio is more
  robust than an absolute and still not robust: the two measurements are taken at different
  loads. **Prefer a counter the code can expose**; an intermittently red suite is worse than no
  test, because it teaches everyone to re-run instead of to read.
- **A mutation no fixture can observe is not a killed mutation.** A span-splice mutant needed a
  third fixture because a macro body's byte positions sit *below* its use site; an asm-label mutant
  was invisible to a 1.7M-token corpus because a wrong label still parses cleanly.
- **A real corpus finds what fixtures cannot.** All five defects of one wave were constructs
  present in *every* TU in existence, and not one had been imagined.

### 11.2 About measurements and the numbers they produce

- **A dominant finding is a lid, not a summary.** The sweep report shows one diagnostic per file,
  so the largest category hides everything beneath it. Four rounds, four different dominant kinds,
  two of them real defects invisible until the category above was removed. **Re-sweep after every
  fix or gate** — which is the same rule as §10's "re-measure after a fix, not only before it".
- **Agreement between two failures is not agreement.** `BothRefused` was the biggest bucket
  (1017 of 1552 files) and means only that each side had *something* to say: chiero was reporting
  `__int128` from `/usr/include` while gcc reported a zero-size array, and the two had never agreed
  about anything. **Check which buckets moved before naming a mechanism** — `findings 1→0` with
  `agree 5→6` is one file; `both refused 1018→1017` with `misses 0→1` is a *different* file.
- **Read the sites before queuing a characterisation.** Six findings sat in the queue labelled
  "gcc *warns*; severity question", written from the *kind* of the message. Every one passed a
  literal `0` to a parameter declared as an array, which gcc compiles silently in both modes — an
  over-rejection, not a calibration question. Three defects in one wave were kept invisible by the
  same thing: an unmeasured assertion written as fact.
- **When instrumentation reports the impossible, suspect the instrumentation.** `record_of` and
  `field_of` are adjacent and their first five lines are identical, so every edit anchored on
  those lines — the fix and four rounds of debug prints — landed in the function that path never
  calls. **Anchor a patch on the signature, not on the body.**
- ⚠️ **A status that means "chiero did not finish" is a lead, not a footnote — and the cause it
  is filed under is a claim.** Two `timeout` rows sat in the plugin sweep for a wave, counted and
  never named, under a spec sentence saying they were a long solver query "for exactly this
  reason". The check that broke it open cost one command: *does the bound that sentence proposes
  actually move them?* It did not, at any value, and neither did the engine's clock — because the
  time was in the verifier, before execution. **When a document names a cause, it has made a
  testable claim; the test is usually cheaper than the reading.**
  ⚠️ Practical note for the next stack sample: `ptrace_scope=1` here blocks `gdb -p <pid>`. Run
  the program as gdb's *child* (`gdb -batch -x cmds --args ./prog …` with `run` then `bt 18`) and
  interrupt it from a background `( sleep N; pkill -INT -x prog )`.
- ⚠️ **Some defects are invisible to reading and cheap to sample.** `vars_of` looks fine: it walks
  a term and collects variables. What is wrong is the *pattern* — one `vec![false; nodes.len()]`
  per call, called once per constraint, on every query, against an arena that only grows. Eight
  stack samples of one VPP run found it in about two minutes; the `Vec::contains` grep that
  found its neighbours never would, because there is no `contains` in it.
  **Sampling is now a cheap standing move:** `gdb -batch -x cmds --args ./target/release/chiero …`
  with `run` then repeated `bt`/`continue`, interrupted from `( for i in $(seq 1 8); do sleep 6;
  pkill -INT -x chiero; done ) &`. ⚠️ `ptrace_scope=1` here forbids `gdb -p`, so the program must
  be gdb's *child*.
  ⚠️ **And read what the samples say, not what you went looking for.** Four of eight were waiting
  on z3 — the expected answer, and the one that would have ended the investigation if it were the
  only frame checked.
- ⚠️ **A speedup is not a complexity change, and only the ratio tells them apart.** The
  verifier's `dominators` went 11.5 s → 270 ms at 3001 blocks and was called fixed; the growth
  per doubling had not moved at all, so seven more instances of the same scan were still there —
  one of them in the *next function down*. **Re-run the curve after the fix, not just the
  timing**, and read the ratio rather than the number. §10's "re-measure after a fix" says the
  same thing one level less precisely.
- **A growth curve settles a performance claim; a single timing never does.** 10/40/160/320/640
  and the *ratio per doubling* is the whole argument — 4x is quadratic, 6x is worse than
  quadratic, and either is a fact nobody can wave away. The same shape appears in this file's
  earlier 673-second finding (250/500/1000 functions), which is how that one was attributed
  correctly too.
- **Do not trust profiler function names.** Under callgrind, `declare_ordinary` and `intern_tagged`
  existed only in the profile build's inlining; read the **inclusive** figure and only that
  (`const_eval`'s own body was 0.02% of a cost that was entirely its recursion).

### 11.3 About the design, and the distinction this project keeps re-deriving

- **"Did not look" must stay distinct from "found nothing", at every scale.** Selector
  (`Selection::NeedsAst`), recipe (`RecipeTally::needs_ast`), sweep (`Tier1Report::unreadable`),
  each with `is_complete()`. It earned its keep on first contact: a `recipe-sweep` with no `-I`
  reported *36 unreadable, PARTIAL* rather than `0 candidates`, which would have read as a clean
  tree. **Do not collapse any of them to a bool.**
- **Absence and zero are different facts**, and the design has turned on this three times: a line
  gcov never recorded (`None`) vs one it saw and nothing ran (`Some(0)`); `tests_for_line` `None`
  vs `Some([])`; a test that crashed with no artifacts (`always_run`) vs one that ran and covered
  nothing (skippable). Flattening any row gives a confident "nothing to run" built on no evidence.
  **When a new query is added, ask what its empty answer claims.**
- **chiero not knowing a value is not the program failing to write one.** Eight instances, each
  reaching memory a different way (an `extern` global, a bitfield read, a symbolic byte through a
  model, a promoted object, a `{:?}` bail-out, an invented bound). 021 §6 settled it in so many
  words and it was re-broken six times. **When you find the ninth, do not fix the site** — ask
  which read path does not end in a symbol.
- **"Fix the rule, not the site" is not a fix you apply once.** Setting a flag and clearing it
  are two different rules, and getting the first right says nothing about the second. The
  `paste_op` commit argued — correctly — that the fix belonged on the token rather than at the
  paste site, and shipped with the flag unbounded at its exit; a review then reproduced the exact
  panic the commit was named for. ⚠️ **When a fix introduces a piece of state, ask separately who
  clears it**, and prefer one place at a boundary ("nothing leaves this pass still armed") over a
  guard in each branch that happens to leak today.

  ⚠️ **Third instance, 2026-08-07, and this one is about the sentence rather than the code.** The
  commit building `max_solver_rlimit` wrote *"`Engine::new_solver` replaces two identical
  construction blocks… a budget that applied to feasibility queries but not to checker queries is
  not a budget"* — correct reasoning, stated as though done, over **one of three** places a run
  builds a solver. **Claiming a single construction point is not the same as making one.** The
  census that settles it is one command (`grep -rn "TieredSolver::" --include=*.rs`) and takes
  four seconds, and it was not run *because the commit message had already argued the point*.
  §7.2's rule, from a new direction: a plausible rationale is not evidence, and writing an
  emphatic one makes the check feel redundant exactly when it is not.
  **When a commit message says "every" or "the single", run the enumeration before the sentence
  ships** — the same discipline §11.3's last entry asks for on `ExprKind`'s variants.
- **A corpus of a new *kind* beats a wider slice of the same kind.** Twenty VPP headers became
  twenty-two and found real defects; 141 preprocessor torture cases found three in one wave,
  including two panics on the C standard's own worked example. Real code exercises constructs *as
  people write them*, which is a systematically biased sample — the dark corners are never in it,
  and no amount of widening within it reaches them. **When the yield table flattens, change the
  kind rather than the size.**
- ⚠️ **Two neighbouring rules can have opposite conditions, and a shared flag will get one
  wrong.** GNU comma-swallowing turns on whether the variadic argument was *supplied*
  (`debug(Y, )` keeps its comma); `__VA_OPT__` turns on whether it has *tokens* (`P(1,)` yields
  nothing). Same parameter, same function, opposite tests — and putting the first condition on
  the flag the second uses broke every non-empty row within a minute. **When two rules read the
  same input, write down what each one is asking before sharing anything between them.**
- ⚠️ **When a differential gate reports a difference, ask whether the difference is in the
  gate.** Of the last three pp-gate findings, **two were mine**: a canonicalization that cast
  bytes to `char` (Latin-1, so a UTF-8 `¨` became `Â¨` and never matched a decoded escape), and a
  scoring rule that counted "chiero rejected and a compiler rejected" as `MatchedNeither` because
  there were no tokens to compare. A gate compares *renderings*; the thing you care about is
  identity, and the two part company at UCN normalization, at unterminated literals, and at
  rejection.
- ⚠️ **A qualifier in someone's note is load-bearing until you have tested that it is not.**
  `is_chiero_limit`'s note said `NullDeref` cannot serve as the objectless probe "because neither
  can produce two findings **on one path**". I read "on one path" as incidental, recorded a route
  around it, and pushed that. Measured: a two-path fixture passes with `FindingKey`'s `func`
  component neutralised, while the fixture the note defends fails under the same mutant —
  deduplication happens *within* a path, so two findings on two paths never compete for a key.
  ⚠️ **The check that settles it is a mutant, not an argument**: neutralise the component the test
  claims to pin and confirm the candidate fails. Four minutes, and it is the only thing that
  distinguishes a probe from a fixture that merely passes.
  This was the **fourth** claim of one session that did not survive being checked — the others
  being an edge asserted without reading the corpus, a fact about gcc written from memory, and a
  queue entry naming the wrong spec. **Reasoning here is good enough to generate candidates and
  not good enough to skip verifying them.**
- ⚠️ **A blocker stated as "this needs a thing of kind X" is a claim about X, and usually the
  requirement is not X but the property X was being used for.** `BadRange` sat outside
  `is_chiero_limit` for waves because two `FindingKey` probes needed an **objectless** non-fatal
  fault and it was the only one in existence. A wave went into hunting for another; the note
  even recorded a route that turned out to be wrong. What the probes actually needed was *two
  findings agreeing on the `object` component* — and a **shared** object gives that as well as
  an absent one does. The fix was a fixture, not a new fault kind.
  **Restate a blocker as the property before accepting it as a search.** The tell is a
  requirement phrased with a type name in it.
- ⚠️ **A type coarser than the thing it models makes defects invisible to tests, not impossible
  in code.** `features::TABLE` held a `bool` for a query that returns a *version number*, so six
  rows claiming `1` where gcc says `201904` were agreed with by a test comparing `bool` to
  `bool`. No assertion written over that type could have failed. **Ask what values the thing can
  actually take before choosing how to store it** — and when an oracle and a table share a type,
  the oracle cannot check the table.
- ⚠️ **When a compiler has one message you expect, check whether it has two.** gcc distinguishes
  "is not a valid universal character" from "is not valid *in an identifier*" — a well-formed
  UCN naming a character that is not an identifier character — and a third for the initial
  position. I merged the first two, and the **fixture** failed rather than the code, which is the
  good direction. Enumerating a compiler's messages for the feature costs one command
  (`for` over the cases, read stderr) and settles the shape of the rule before any code is written.
- ⚠️ **A guard that enumerates exclusions is waiting for the next exclusion.** The GNU comma
  branch was guarded with `!right.paste_op` — true, useful, and the wrong shape: it named two
  things the extension does not apply to, when the rule is what it **does** apply to
  (`right.from_variadic`). The narrow guard shipped in the same session as the fix that replaced
  it. **State the positive rule; a list of things it is not never terminates.**
- ⚠️ **The harness is defective more often than you expect, and it fails *flatteringly*.** In one
  session: a gcc oracle that raced on a shared scratch path (**twice** — the second key was still
  a guess about which callers exist), a probe comparing whitespace-split words instead of tokens
  and reporting four false divergences, and a `pgrep -f` waiter that matched itself and reported
  a run as live for over an hour when it had never started. **Every one reported something
  impossible**, and two were read past before being checked. §11.2's rule is not a footnote about
  profilers — it is the most frequently-earned rule in this file.
  Corollaries, each paid for: **parallel tests must not share mutable filesystem state at all**
  (a per-call counter, never a "unique enough" key); **a per-crate run cannot see a race** — only
  the full-workspace run put enough load on to expose the second one; and when a measurement
  looks wrong, reproduce the *instrument* before reasoning about the subject.
- **The residue of a gate is a corpus.** Two of this session's three defects came from the
  pp-gate *failing*; the third came from reading the rows it had already written off — 21 cases
  filed as "neither compiler ran it" turned out to be the corpus's error-recovery half, and the
  `Skipped` rows named a whole capability (`__has_attribute`) that was answering 0 to everything.
  ⚠️ **When a gate goes green, read what it is not asserting**, not only what it now asserts.
- **A persona is only as good as its least-complete half.** `__GNUC__` baked without
  `__GNUC_MINOR__` makes glibc's `__GNUC_PREREQ` constant 0, which silently reconfigures every
  system header. One absent predefine, whole-tree effect, no diagnostic anywhere. **When chiero
  claims to be something, enumerate what that claim implies** rather than adding the parts that
  came up.
- **A catch-all match arm hides a missing feature.** The engine's terminator dispatch had an `_ =>`
  reporting "unsupported terminator" at run time, and `Switch` sat inside it for eight waves.
  Removing the arm makes the *compiler* reject an unhandled variant. `chiero-lower`'s statement and
  expression dispatch still has one, deliberately (015 §7's refuse-rather-than-lower-wrongly), and
  it is **not audited**.
- **Path identity is this project's recurring silent failure**, and it always produces a
  *flattering* answer with no error: three times, in three places, a lookup missed and read as
  success — once as a premise test passing for the wrong reason, once as 0% on exactly the
  mutations where the baseline works, once as a **100% reduction**. ⚠️ Matching by basename is the
  tempting fix and stays rejected.
- **A dummy span fabricates a location.** Three separate places have now needed an `is_dummy`
  guard first.
- **Enumerate the variants; do not wait to imagine them.** Walking `ExprKind`'s 21 variants against
  the arms of `raw_expr` took ten minutes and found three reaching the catch-all, one of them a
  live silent defect. The same census over `RValue`/`InstKind`/`Terminator` came back clean, which
  is also worth knowing. Every other defect channel — human fixtures, adversarial review, probing
  around a smaller bug, mutation — is **bounded by what a human thought to spell**.
