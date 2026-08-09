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
- **Licence: MIT OR Apache-2.0**, both texts at the repository root, every one of the **24**
  packages inheriting the SPDX field (`chiero-probe` was added 2026-08-08). Left for the first publish: the texts are not inside each
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
`chiero-sema` → `chiero-cir` (lowering). Beside `chiero-pp`: **`chiero-probe`**, the one crate
that runs `cc -dM -E` — a `Persona` is read from that text and something has to produce it, and
one probe rather than one per surface is the whole point (2026-08-08).
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
| 040 checkers, 042 recipes | partial | `chiero-check` runs **2** checkers by default; `chiero-recipe` exists |
| 060 vpp | **contract 1 met 2026-08-08** | `chiero_vpp::builddb` — VPP's compile database; 1967 C units → **423 configurations**; `chiero-vpp` was an empty placeholder until this |
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

### 7.19 Every tolerance list in the repo, read backwards — 2026-08-09

**Four lists, three defects, two honest zeros.** The class: *a list that excuses things is
checked in one direction only.* An item **not** on it is a violation; nothing ever asks whether
an item **on** it still excuses anything. A stale entry is not inert — it silently re-permits
the thing it names, so a decision somebody once argued about gets re-made by nobody.

| list | verdict |
|---|---|
| `chiero-lower/tests/generated.rs` **`KNOWN_GAPS`** | ❌ **entirely dead — 0 of 4 entries matched anything across 600 programs.** One provably stale: it excused a float-comparison gap closed two hundred waves earlier, and its own text read *"this entry is what will fail when they land"*. Split into `KNOWN_GAPS` (gap records, must fire) and `DECLARED_FIDELITY` (023 §7 grading policy, exempt); the float entry deleted; `KNOWN_GAPS` is **empty**, which makes the original assertion strict — *any* refusal now fails, matching what the focused and control-flow channels already assert |
| `xtask/src/deps.rs` **`ALLOWED_VERTICAL_EDGES`** + **`FRONTEND_USING_VERTICALS`** | ⚠️ **all six edges and both exemptions live today** — but the file *records this failure already happening once* (`chiero-diff -> chiero-gcov` "was declared and unused", caught by a person reading the list). `unused_exemptions()` added, wired into `main`'s `check-deps` |
| `chiero-vpp/tests/persona_gap.rs` **`DELIBERATE`** | ❌ **five of six entries excused nothing**, while the doc claimed *"every entry is a difference the gate really sees on every run"*. **The only instance whose claim was false today** — and it was measurable from the gate's own printed output (`15 compared, 1 differ` against six entries). Same split: `DELIBERATE` (must fire) / `NEVER_COMPARABLE` (exempt) |
| `generated.rs` **`ASAN_CLASSES`** | ✅ **honest zero** — already has a real per-class liveness floor (`seen >= 3`). It is the one that got this right first; the pattern was available in-repo |
| `xtask/src/pp_gate.rs` **`SIMPLECPP_SKIP`** | ✅ **honest zero, and not a suppression at all** — `Prior` is *"carried, not obeyed"*: all 141 cases run regardless and the list is only a label |

⚠️ **Three things this cost, in the order they hurt.**

1. **A false zero on the way in.** The census keyed float comparisons on a `d_`/`f_` naming
   convention the generator does not have, and read **0** where there are **57 of 200**. It read
   exactly like *"the corpus cannot reach this"*, which would have made the whole finding
   backwards. §8.3's "when a probe reports zero, check it can report non-zero", again.
2. **The fix's own assertion was unreachable logic, not merely vacuous.** With `KNOWN_GAPS` empty
   the filter iterates zero entries, so a polarity flip and a reversed `contains` both survived
   the entire suite. The mutation evidence had been gathered by mutating the **data** — adding a
   bogus entry — and then left in a commit message, where it expires the moment somebody edits
   the block. Fixed by extracting `dead_entries()` and unit-testing it on synthetic input, a
   pattern this file already used for the shrinker. **Mutate the code that will still be there,
   not the input you can revert.**
3. **An exemption list is a place to file things unless something stops it.** Both splits create
   an exempt list, and in `persona_gap` a mutant moving the *live* `__STDC_VERSION__` entry down
   into `NEVER_COMPARABLE` **survived** — the dodge is real, not theoretical. Closed with the
   structural property that is the actual reason those entries never fire: **VPP tests none of
   them in an `#if`**, checkable from the same map the gate already builds. `generated.rs` got
   the same treatment differently — `DECLARED_FIDELITY` is consulted only for the row a
   `Verdict::Gap` produced, and matched exactly, so the split is enforced by the verdict type
   rather than by which `const` somebody chose. ⚠️ **Both exempt lists are still unexercised** —
   nothing degrades, nothing diverges by construction — and that is written at each list rather
   than papered over.

📌 **Two lists reaching the same split independently is some evidence the distinction is real.**
The recurring shape is *"X is broken today"* (must still be true) versus *"X could never work"*
(policy, true whether or not it occurs). It is not evidence either list is right.

### 7.20 A generated record-layout corpus, and the third attribute position — 2026-08-09

**The defect.** An attribute written *before* the `struct`/`union` keyword was treated as part
of the record's definition. gcc and clang both ignore it there — it appertains to the declared
*object*, or to nothing when there is no declarator. Measured all three positions:

| written | gcc & clang |
|---|---|
| `__attribute__((aligned(16))) struct S { char a; };` | `sizeof` **1**, `_Alignof` **1** — ignored |
| `struct __attribute__((aligned(16))) S { char a; };` | 16 / 16 — applied |
| `struct S { char a; } __attribute__((aligned(16)));` | 16 / 16 — applied |
| `__attribute__((aligned(16))) struct S { char a; } v;` | `_Alignof(v)` **16**, `_Alignof(struct S)` **1** |

clang says so out loud (*"attribute 'aligned' is ignored, place it after "union" to apply
attribute to type declaration"*); **gcc is silent, not even under `-Wall -Wextra`**. With
`packed` the consequence is an *offset*: `__attribute__((packed)) struct S { char a; int b; }`
came out 5 bytes with `b` at 1 against gcc's 8 and 4, so every field access into such a record
was wrong.

`Attr::from_specifier` is the third position, beside `from_declarator` — whose own doc records
the **identical defect from the postfix side** (*"`struct S` came out 112/16 where gcc says
104/8, and glibc's `__pthread_unwind_buf_t` with it"*). Two of three positions have now been
wrong, each found by a different corpus.

📌 **And `lay_out` read `packed` with no position filter at all**, while `record_is_packed`
ninety lines up had always filtered `from_declarator`. **Two sites reading one fact differently
is how a rule ends up half-applied** — the same shape as the two `Vec::contains` in
`verify.rs`, and as the three mechanisms for one predefine that 1b complained about. One
predicate now.

⚠️ **The residue was three things, and only one was a bug.** 120 = 119 + 1 + 7:
- **119** the prefix-attribute defect.
- **1** `aligned(4)` on a `void *` member — **gcc and clang disagree with each other** (12/4
  against 16/8) and chiero matches clang. The gate now asks the second compiler before calling
  anything a defect, and `MATCHED ONE` is its own row: never merged into agreement, which
  would lower the gate's standard, and never into disagreement, which would make the gate
  wrong about chiero.
- **7** refusals that were chiero being **right** — a record whose members are all unnamed is
  undefined (C11 6.7.2.1p8) and gcc warns under `-Wpedantic`. The generator avoids the shape
  by construction now.

**§7.6's rule held for the fourth time: a shared message is not a shared cause.** The 120 was
never trusted as a count of defects, and that is the only reason the last two were looked at
rather than assumed fixed.

#### 7.20b The fix blinded the corpus that found it — and the second run found a second defect

⚠️ **The most transferable thing in this section.** The generator emitted record attributes
only in the **prefix** position. The fix above made that position correctly *ignored* — so from
that commit onward chiero ignored those attributes, gcc ignored them, the two agreed, and the
gate's entire `packed`/`aligned` dimension was **testing nothing**. It went on printing 241
agreements, and the reach test went on asserting `packed >= 40` while not one of those
attributes packed anything.

**Counting a construct is not counting a test of it.** The reach assertion was true and
meaningless at the same time — the same shape as a probe that reports zero because it *cannot*
report non-zero, arrived at from the opposite direction: a probe reporting a healthy number
because it cannot report a *failure*.

All three positions are emitted now and counted apart (`139 prefix (ignored), 109 middle, 137
postfix`), the prefix among them, because the ignored case is the regression the fix installed
and it needs a guard of its own.

**With the attributes finally reaching records, the next run found a second defect:** a
zero-width bit-field in a **union**. In a struct a `:0` flushes the allocation unit; in a union
every member starts at offset 0, so there is nothing to flush and a `:0` declares no member.
chiero carried the bit cursor across union members:

| | chiero | gcc | clang |
|---|---|---|---|
| `union U { short a:14; int :0; };` | **4** | 2 | 2 |
| `union U { int :0; short a:14; };` | 2 | 2 | 2 — *leading was already right* |

so a fix repairing only the trailing case would have passed a test written one way round. Both
orders are pinned, and the discriminator is stronger than either number: with the `:0` and
without must be the *same union*.

📌 **The `MATCHED ONE` rows are one cause — and the first statement of it here was too narrow,
which is its own lesson.** It was written as *"`aligned(N)` on a member whose natural alignment
exceeds N"*, generalised from three rows that all had that shape. The next run produced five
and two of them did not. **Read the last one.** Measured:

| | gcc | clang & chiero |
|---|---|---|
| `{ void * __attribute__((aligned(8))) m; } __attribute__((packed));` | 8/**1** | 8/**8** |
| `{ void * __attribute__((aligned(4))) m; };` — no `packed` | 8/**4** | 8/**8** |
| `{ void * __attribute__((aligned(16))) m; } __attribute__((packed));` | 8/**1** | **16/16** |

**gcc lets an alignment-*lowering* context override a member's explicit `aligned`; clang never
does.** chiero is on clang's side throughout — and ⚠️ **gcc contradicts its own manual in both
directions**: it documents that `aligned` *"can only increase the alignment; to decrease it you
need packed as well"*, yet row two decreases without `packed` and row three refuses an increase
because of it. Worth knowing before anyone "fixes" chiero to match a quick gcc experiment.

### 7.21 The pinned-40 retake after 2026-08-09, and why identical was the only possible answer

`ok=38 cut=2 findings=21 exact=0` — **byte-identical** to the baseline, after a session that
changed `layout`, alignment resolution and sema's diagnostic severities. The tempting reading is
"no regression". The true one is stronger and less comfortable, and it took one grep:

| shape fixed on 2026-08-09 | occurrences in all of VPP `src/` |
|---|---|
| prefix-attributed record definitions (§7.20) | **0** |
| zero-width bit-fields — so union `:0` and packed `:0` too | **0** |
| `_Alignof`, in any spelling (5n) | **0** |

**The instrument could not have moved, and therefore could not have detected a regression in
this work either.** An identical retake is evidence about the corpus before it is evidence about
the tree — §8.3's trap, arrived at from the reassuring side rather than the alarming one.

📌 **It also closes a loop.** `generated_layout.rs` was built this session precisely because
the VPP gate and the hand fixtures cannot reach these shapes; the table above is that claim
turned into a number. The evidence for today's layout work is the 585-record generated corpus
and gcc, not the pinned 40 — and the pinned 40 is still the right instrument for what it does
cover, which is why it was run.

⚠️ **The general form, worth applying before any retake:** *ask what the corpus would have to
contain for this number to move.* If the answer is "nothing it has", the run is a control, not
a check — worth doing, worth recording, and worth not mistaking for confirmation.

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
| **a new subsystem: all of `vnet/`, 423 entries** (only `vnet/ip/` had ever been swept) | one sweep, ~25 min | **44 findings** (40 after the 7b fix), 0 `Exact`, 0 `timeout`, 0 `noinc` — and two `failed` rows that are **real defects in VPP source the build never compiles**: a function defined twice, and a call with no declaration. gcc agrees on both. The harness lesson is bigger than either: `pick_entries.py` globs the tree, so `failed` mixes "chiero cannot read this" with "nothing can" |
| **the plugin sweep, one entry per file → three** (477 → 1320) | one sweep, ~65 min | **91 findings against 18**, 3 `Exact` against 1 — and the yield was a *reporting* defect: a `proven: true` null dereference resting entirely on a global's initial value, with the premise unstated. Also 31 `failed` rows resolved to **six** causes, 19 of them the parked `-march` item, and one that is not a chiero defect at all (below) |
| the **second** sampling round: two more `cut` entries | one wave | **an honest zero.** `active_open_alloc_session_fifos` is dominated by `BvConst` arithmetic under `TermArena::eval`, reached from the counterexample cache — which is 022 §6.2's *self-certifying* rule doing exactly what it must, and `BvConst` is a `Copy` `u128` with no allocation behind it. Recorded because a table of only wins cannot say when to stop |
| *not a widening* — **sampling a real run's stack instead of reading its code** | one 90-second run under `gdb` | `TermArena::vars_of` allocated a bool per node in the *whole arena* on every call, and 022 §6.2's slicing calls it once per constraint on **every** backend query. 8.3 µs → 699 µs as the arena grew; now flat. **Nothing in the code reads as wrong** — the defect is a call pattern, and only a profile shows it |
| *not a widening* — **taking a `timeout` row seriously instead of counting it** | one sweep + one stack sample | the CIR verifier was **super-quadratic**: 11.5 s for 3001 blocks, and it is what killed VPP's last two `timeout` entries. 023 §8 had attributed them to a long *solver* query and specified a bound for that; the bound did not move them. **42x faster, 0 `timeout` rows left, one spec claim retracted** |
| *not a widening* — **re-measuring the pinned 40 and asking why nothing moved** | one retake + one `grep` | the corpus **cannot reach** a 32-byte access: `__AVX2__` is undefined in every configuration chiero compiles, so every AVX2/AVX512 path in vppinfra is invisible to every measurement this project has published. New evidence for the parked `-march` item, and it came from an *unchanged* number |
| **a new *kind* of gate: the preprocessor under VPP's own flags** (012 c17, 1967 TUs, 18 min) | one ingest + one gate | **three defects the pp-gate could never see**, because none is about preprocessing *syntax*: `__linux__` unbaked (VPP's `pmalloc.c` reached `#error "Unsupported OS"`), `__has_attribute(error)` answered 0 where gcc says 1, and a diagnostic class chiero was *right* about and that was still noise. Diagnosed 25 → 0, and the token count 731M → **792M: 8% more of the program became visible** |
| *not a widening* — **asking what chiero believes rather than what it says** (`persona_gap`, 0.1 s) | one differential instrument | the **endianness** defect: `__BYTE_ORDER__` undefined, so `#if __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__` read `0 == 0`, took the big-endian branch on x86-64, and reversed bit-field member order across `srv6-mobile`. **The 18-minute corpus gate is structurally incapable of finding this** — a wrongly-taken branch emits nothing. Output-watching and state-comparison are two different searches |
| the *same* audit's sibling class — **filters that drop faults or diagnostics** | free, one grep | **an honest zero: all four explain themselves**, including the two that had cost waves (`write_bytewise`'s `Misaligned` strip and `report_faults`'), now that they are commented. Two dead `let _ =` discards removed so the next sweep of the class finds only real ones — **an audit is only as sharp as its noise floor** |
| **auditing a *class* the previous wave named** — `let _ = <named parameter>;`, information dropped at a boundary | one wave | **a wrong `proven` layout on 22 VPP sites.** An enum's declared underlying type (`typedef enum … : u8`) was parsed and discarded, so `struct { enum e; char c; }` was 8 bytes where gcc says 2. 8 hits, 4 unexplained, 1 with a consequence — and the audit existed only because the previous wave's undocumented discard had cost a wrong RED |
| **following the vector surface into the memory model** — does an OOB 32-byte store get caught, and is a misaligned one recorded? | one wave | **the overwrite is caught, `Exact`, and now pinned end to end from C** (a path no test reached: everything else tests the *wide-load* route, which C vector code does not take). The alignment half diagnosed a real gap — `CopyMem`'s `align` operand is discarded — **via a RED that was itself wrong**: `write_bytewise` strips `Misaligned` deliberately, with no comment. *An undocumented deliberate behaviour is indistinguishable from a defect* |
| **the AVX2/AVX-512 half of vppinfra — 384 units, never parsed by anything** | one wave, minutes | **an honest zero on parsing: 24 sampled units, 0 diagnosed** — and the widening is real, **+292 to +528 definitions per TU** that no chiero measurement had lowered. `find-bugs` on 8 vector-using entries: 1 finding, a known class. **The yield was a corrected belief**: `unsupported-access-width` is zero not because the corpus cannot make a 32-byte access but because a vector access lowers to `copymem` — 7779 of them ≥32 bytes in one TU. ⚠️ **Two false zeros from ad-hoc greps in one wave**, both reading "nothing changed": `^func` counts declarations, and copymem sizes are `32i64` not "32 bytes". **When a probe reports zero, check that it can report non-zero** |
| *not a widening* — **measuring a "stale environment" before acting on it** | 10 min | the tree had moved by **165 files in one checkout**, but only **4 `.api`** could matter: chiero reads `src/` directly, so only *generated* artifacts can be stale. Fixed with the generator command rather than `ninja`, whose target would have re-run cmake and rewritten the `build.ninja` every VPP measurement reads. **A blocker described in prose was a one-second check** |
| **reproducing a defect at the layer it lives in** — the witness reporting item, after four waves of engine fixtures | one wave | **950 KB → 11.9 KB on the real VPP entry**, and two defects inside the fix: "show the first *k*" would have dropped every pinned binding, and a bounded rendering would have become a bounded *proof condition* in `check_reachable`. The four earlier dead ends all tried to *produce* a huge witness by execution; it was a reporting defect, and three of the six tests build a `Witness` directly |
| **splitting a clock instead of counting inside it** — timing `ingest_into` apart from the arc-index walk | one column in an existing gate, 4 min | **the gcov growth gate passes for the first time.** 90% of the cost was on the *other side* of the suspect §9.1 had recorded, and two throwaway experiments bisected it to `block_counts` — every block scanning every arc. ⚠️ One of the three fixes was a change **measured and honestly ruled out earlier the same day**; the null result was true while `block_counts` dominated |
| **the per-TU persona join** — the corpus gate stopped preprocessing 1967 units as one compiler | one crate + one wave, 23 min to re-measure | **+26M tokens, 3.3% more of VPP visible**, 0 diagnosed throughout — and the yield was a *number*: 8 distinct target flag-sets where I had written 5 in four places. A `-march` value is not a flag-set (`-mtune`, `-mprefer-vector-width=512`, `-maes`, four units with none, one naming `-march` twice). One `ninja -t compdb` pipe would have said so at any point |
| **a new *kind* of layout corpus: generated records instead of VPP headers** (`chiero-sema/tests/generated_layout.rs`) | one generator, ~1 min per run | **a real `layout` defect on the first run: an attribute written before `struct` was treated as the record's own.** `__attribute__((packed)) struct S { char a; int b; }` came out 5 bytes with `b` at offset 1 where gcc and clang both say 8 and 4 — a wrong *offset*, so every field access into such a record was wrong. **120 of 232 records contradicted → 0 of 242.** ⚠️ **VPP contains not one instance**, so the 10 248-assertion VPP gate was structurally incapable of finding it — §8.3's trap, paid out exactly as written. It reuses `assert_agrees_with_gcc` rather than writing a second oracle |
| *the residue of that run, read rather than counted* | free | **two causes, not one, and neither was a chiero defect.** `aligned(4)` on a `void *` member: **gcc and clang disagree with each other** (12/4 against 16/8) and chiero matches clang — so the gate asks the second compiler before calling anything a defect, and `MATCHED ONE` is its own row. The 7 refusals were chiero *correctly* diagnosing C11 6.7.2.1p8 (a record with no named member is UB; gcc warns under `-Wpedantic`). **§7.6's rule held again: a shared message is not a shared cause** — 120 was 119 + 1 + 7, and only the 119 was a bug |
| **the class the gating defect belonged to: does a gate exist for it?** — `generated_silence.rs` | one wave | **it existed and was structurally blind to the class.** The channel runs `gcc -std=c11 -pedantic-errors`, so a program pedantic gcc *rejects* is skipped — and every gnu11-only construct is exactly that. It could never see chiero out-talking the compiler VPP builds with. A second channel added: *a program `gcc -std=gnu11` compiles saying **nothing** must leave sema silent under `Dialect::gnu()`*. ⚠️ **Its first version was green over a corpus that could not reach its own subject** — reverting the previous wave's real fix left it passing. Six gnu11-legal/pedantic-illegal shapes added (the two record rules, zero-size array, `__int128`, cast-to-union, void conditional); the mutant then dies with 5 complaints. **The two channels partition the corpus rather than sharing it** — literally, a wave later: sharing it had cost the strict channel a quarter of its coverage (300 checked → 221), and ⚠️ **its anti-collapse floor still passed**, because 221 clears `checked * 2 > count`. *A cap does not have to defeat a guard to be silent; it only has to stay inside it.* |
| **finishing the mechanism the previous wave introduced** — which of sema's diagnostics are advisories? | one wave | **six ISO conformance remarks moved off `Error`.** Each site already said "support is unconditional and only the sentence follows the dialect", so `is_error()` was returning **true** for six diagnostics that invalidate nothing — and `is_error()` is what a consumer asks before discarding an analysis, with `chiero-diff` running the strict dialect today. **Seven more were named and deliberately left**; the next wave measured them and **four qualified** (both record rules, the enumerator-range rule, the null-pointer comparison), each shown to leave a complete layout beside the diagnostic. **Three stay at `Error` with the reason recorded**: one could not be made to fire, one has a mixed guard that also covers a real constraint violation under gnu11, and one is a mixed bucket. ⚠️ Also found **and then fixed**: `has no named members` sat outside its pedantic guard, so chiero reported it under gnu11 where gcc and clang are both silent — the sweep that exists to report *what a project's compiler would* carried a sentence gcc never says. **The severity audit found a gating defect**: the rule qualified as an advisory whichever dialect it fired in, so answering "what does this diagnostic mean" would have shipped without noticing that "when does it fire" was also wrong |
| **auditing the class the previous three waves kept re-deriving** — *where else does a diagnostic share a bucket with a failure?* | one grep | **a product defect, not a test one.** `chiero cir` stopped at the first sema diagnostic, and `SemaDiagnostic` had no severity, so a signed-overflowing constant expression — which chiero folds to **exactly** the value gcc and clang fold it to — made chiero refuse an entire translation unit that gcc compiles with exit 0. `Severity { Error, Advisory }` added, defaulting to `Error` at every one of 160 sites so nothing else moved. **Three instances inside test harnesses were a class, and the class had a fourth member in the shipping tool** |
| **a new surface: generated integer constant expressions** (`chiero-sema/tests/generated_const_eval.rs`) — the least differentially-tested path in the crate | one wave | **an honest zero on chiero: 300 of 300 agree with gcc.** `const_evaluator_reuse.rs` is 13 assertions that ask gcc *nothing* — its subject is that a shared evaluator matches a fresh one, a self-consistency property that would hold perfectly while both were wrong. ⚠️ **The one row that looked like a finding was a defect in the new gate**, and the third instance of one conflation this session: it checked `diags.first()` before the value, so an expression chiero folds *exactly as gcc does* while also warning was reported as "would not fold". **A diagnostic beside a correct answer is not a refusal** |
| **enum members added to the generated layout corpus** — the construct adjacent to 5k, which no hand fixture puts inside a struct | free, the gate existed | **an honest zero.** All three shapes agree with gcc: declared underlying type (`enum E : unsigned char`, the one discarded until 2026-08-08 and wrong on 22 VPP sites), plain `int`-width, and an enumerator past `int` that widens it. 56/67/57 over 400 seeds, so it is a real three-way discriminator rather than one shape three times. **The first mechanical confirmation that the 5k fix holds across shapes** rather than on the site that exposed it — recorded because a table of only wins cannot say when to stop |
| *not a widening* — **disbelieving a cost this file had just recorded** | 10 min | the previous row's "`__int128` had to leave the generator, `SemaDiagnostic` has no severity" was **wrong**: chiero has had a dialect switch since wave 314, `__int128`'s diagnostic is gated on it, and `harness::parse` has used `Dialect::gnu()` all along *with a doc comment saying why*. The gate had called the deliberately-strict sibling. **A blocker stated as "the type lacks a field" was a helper nobody read** — the `MemFault::BadRange` lesson again. ⚠️ And restoring the construct blinded a dimension **again**: the gcc/clang divergence rows went 5 → **0**, which reads as "the compilers agree about everything" and means "the corpus stopped reaching the shape". Gate range 120 → 300 seeds; **222 → 569 records**; both shapes now asserted reachable. **Three waves running, a generator change moved what the fixed seeds produce, and only the reach test caught it** |
| **the same gate a third time — widened by one scalar class** (16-byte-aligned members; no other corpus in the project has a member above 8) | one wave | **a third `layout` defect, and it has nothing to do with 16-byte alignment.** `packed` was cancelling a zero-width bit-field's unit flush: `struct { char a; int :0; } packed` was 1 byte where gcc and clang say 4. **Adding scalars changed what each `pick` returns, reshuffling which records the fixed seeds produce, and a pre-existing defect in an unrelated shape surfaced — a widening pays sideways as well as forwards.** ⚠️ Cost recorded, not tidied away: `__int128` had to leave the generator (chiero's *correct* ISO pedantry is indistinguishable from a refusal because `SemaDiagnostic` has no severity — 57 of 120 seeds refused and the gate stopped measuring layout), so the 128-bit bit-field allocation unit is now reached by nothing |
| **the same gate again, after noticing the fix had blinded it** — attributes in all three positions, not just the one | one wave | **a second `layout` defect: a zero-width bit-field in a *union*.** `union U { short a:14; int :0; }` was 4 bytes where gcc and clang both say 2 — the bit cursor carried across union members and `:0` rounded it to the `int`'s 32. ⚠️ **The previous wave's fix had made its own corpus vacuous**: the generator emitted `packed`/`aligned` only in the prefix position, which the fix correctly made *ignored*, so chiero and gcc agreed about nothing and the reach test went on asserting `packed >= 40` while none of them packed anything. **A fix can blind the corpus that found it** |
| **reading a gate's own tolerance list as a corpus** — the generated differential suite's `KNOWN_GAPS` | one census, minutes | **the ledger was entirely dead: 0 of 4 entries matched anything across 600 programs**, and one was provably stale — it excused a float-comparison gap that closed two hundred waves ago, and its own text said *"this entry is what will fail when they land"*. It did not fail, because **a one-directional ratchet only stops a list growing; nothing stops it going stale**, and a stale entry reads exactly like a live one. ⚠️ **A false zero on the way in**: the census keyed on a `d_`/`f_` naming convention the generator does not have and read 0 float comparisons where there are **57 of 200** |
| **the same class, three more instances** — every allowlist/excuse table in the repo, read backwards | one grep + one review | `xtask` **`ALLOWED_VERTICAL_EDGES`**/`FRONTEND_USING_VERTICALS` — all live today, but the file *records the failure already happening once* (an edge "declared and unused"), so the guard went in; **`persona_gap`'s `DELIBERATE` — five of six entries excused nothing** while its doc claimed *"every entry is a difference the gate really sees on every run"*, the only instance whose claim was **false today**; `ASAN_CLASSES` already had a real liveness floor (`seen >= 3`) and needed nothing; `SIMPLECPP_SKIP` is *"carried, not obeyed"* — a label, not a suppression. **4 found, 3 fixed, 2 honest zeros** |
| *not a widening* — **asking a reviewer "does this shape exist elsewhere"** rather than only "is this right" | one fable subagent | it named the `persona_gap` instance *and* found the fix's own weakness: the new liveness check was **unreachable logic**, not merely a vacuous assertion — with the list empty a polarity flip survived the whole suite, because the mutation evidence had been gathered by mutating the **data** and then left in a commit message. **Mutate the code that will still be there, not the input you can revert** |
| *not a widening* — **disbelieving a "blocked" label** (the gcov `Vec::contains` audit) | one generated `.gcno` | the block was false: gcc emits a `.gcno` of any size from generated C, so no VPP coverage build and no format writer were ever needed. Native ingest measured **quadratic**, then **~250x faster** (17.31 s → ~0.068 s at n=3200, two runs; ±20% at these times) across five fixes. ⚠️ **Six confident hypotheses were refuted by measurement**, four of them before anything worked; counters found every real site. The curve itself was blind twice — once holding blocks-per-line at 1, once measuring below its own noise floor — and a genuinely quadratic counter (327 808 014 cells) turned out to be 7% of the clock. **Counting stops you being confidently wrong; it does not by itself make you right** |

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
> ### 🆕 Suggested first moves after the 2026-08-09 session, in order
>
> 1. **Run the two fast gates** (`persona_gap` 0.1 s, `growth` ~5 s). The 23-minute corpus gate is
>    worth it only when something touched the frontend or the persona — and that rule was
>    exercised on 2026-08-09: a parser change triggered it, it came back **1967/1967, 0 panicked,
>    0 diagnosed, 8 personas**, and its token count is now known to be **deterministic** (two
>    byte-identical runs).
> 2. **Pick a widening (§8.3), or take a concrete item from §9.1.** ⚠️ **The layout and
>    diagnostic surfaces are mined out for now** — the last five waves there returned zeros or
>    findings about the gates themselves. Prefer the untouched concrete items: **032 contract
>    18's replay corpus**, still with no `observed` entry (the probe is committed; firing it
>    re-runs cmake and invalidates every published VPP number, so do it deliberately);
>    **`InstKind::Call` carrying no result type** — ⚠️ **not "135 sites"**, that was mentions; the direct half is **closed** and the indirect half is **~25 sites** on `Callee::Indirect`, not 110 — the recorded design put the field in the wrong place. See the item; and the two
>    `vnet/` finding classes, which are policy questions rather than defects. **5j is
>    diagnosed and half-closed** — both discards are explained at the site and both directions
>    of the gap are measured and pinned; what is left is the API change, and it is still gated
>    on `ub-strict` existing.
> 3. 📌 **The method that paid all day, and the one to reach for first:** *one fact, more than
>    one reader.* Three instances in one session — `packed` read by `lay_out` and
>    `record_is_packed` with different filters; alignment read by three arms of which the fix
>    reached one; and a diagnostic filed as a refusal in four separate places. **When a fix
>    lands, re-run the original reproduction, not the new test.** A green unit test beside an
>    unchanged tool is what that class looks like, and it happened here.
> 3. ⚠️ **Closed 2026-08-09, do not re-open:** the whole **stale-tolerance-list** class — the
>    generated suite's `KNOWN_GAPS`, `xtask`'s two dependency allowlists, and `persona_gap`'s
>    `DELIBERATE`. See the yield table's three new rows and §7.19.
> 4. ⚠️ **Closed 2026-08-09, do not re-open:** the stale-tolerance-list class (§7.19); the
>    prefix-attribute, union `:0` and packed-`:0` layout defects (§7.20, found by the new
>    generated layout corpus); `SemaDiagnostic::Severity` and the ten advisory sites; the second
>    silence channel in the gnu11 dialect; `chiero layout` refusing a TU sema refused (5m); and
>    `_Alignof` of a typedef (5n). **Two of those were defects in that session's own fixes**,
>    both found by adversarial review — an ambient severity flag that leaked a demotion across
>    the whole TU, and a fix that reached one reader of three.
> 5. ⚠️ **Closed in the 2026-08-08 sessions, do not re-open:** the whole persona thread
>    (`chiero-probe`, the per-TU join, 060 contract 2, the corpus gate's configuration); the gcov
>    growth gate, which passes for the first time; the witness reporting defect (5e — read its
>    three corrected claims before trusting a truncated witness); the stale VPP build directory
>    (5d, four files rather than a rebuild); the AVX2/AVX-512 half of vppinfra, parsed and
>    analysed for the first time; and an enum's declared underlying type, which had been making
>    `layout` wrong on 22 VPP sites and saying `proven` (5k). §9.1 1d (`__STDC_VERSION__`) is the
>    **owner's call on the language level** — and `persona_gap` now *asserts* that this divergence
>    is still live, so it goes red the day the decision is made and not acted on.
>
> ⚠️ **What not to do:** do not "fix" a `Vec::contains` because it looks quadratic — see §9.1,
> where two that looked it were not and one described in passing was the whole cost. Do not
> tighten `growth.rs`'s 8.0x threshold. **Nothing is parked right now**; the pause emoji appears
> nowhere in §9.1, which is what the heartbeat's instruction resolves to.
>
> 📌 **The four methods that paid in that session, each against something this file had already
> written down:**
>
> | method | what it beat |
> |---|---|
> | **instrument the boundary between two things** before counting inside either | the persona cache keyed on nothing; the gcov residual, found by splitting one clock into two halves — 90% of the cost sat on the opposite side from the recorded suspect |
> | **reproduce a defect at the layer it lives in** | four waves had tried to *produce* a 10 000-binding witness by execution; it was a reporting defect, testable with no engine run |
> | **audit a class, not a site** | `let _ = <named parameter>;` across the sources: 8 hits, 4 unexplained, 1 a wrong `proven` layout on 22 VPP sites |
> | **when a probe reports zero, check it can report non-zero** | two false zeros from ad-hoc greps in one wave, both reading "nothing changed" — `^func` counts declarations, copymem sizes render as `32i64` |
>
> ⚠️ **And the one that cost a wrong RED:** an *undocumented deliberate* behaviour is
> indistinguishable from a defect. `write_bytewise` strips `Misaligned` on purpose — a copy is
> byte-wise, as `memcpy` is — and nothing said so, so a measured anomaly became a committed RED
> asserting the wrong contract. The fix was a comment. **The audit it provoked found the enum
> defect the next wave**, so the cheapest response to being wrong was to generalise it.
>
> ✅ **The owner's close-the-gap ask is DONE — pp-gate reports 0 findings** (§7.11). Keep it as a
> two-minute standing check.
>
> ### 🆕 The standing gates, with their exact invocations
>
> **Five now**, all `#[ignore]`d — they need VPP, gcc, or both — so `./check.sh` never runs them
> and a fresh session will not discover them by accident. ⚠️ Two were added on 2026-08-09 and
> were **invisible here for a day**, which is §9.2's failure mode with the file's own warning
> pointed at itself: an instrument that is committed but not discoverable is one nobody runs.
>
> ```sh
> # what chiero SAYS: 1967 VPP TUs under VPP's own flags AND its own -march. ~23 min.
> # Metrics 2026-08-09: 1967/1967, 0 panicked, 0 diagnosed, 8 personas, 818 391 162 tokens.
> # ⚠️ The token count is DETERMINISTIC (two byte-identical runs) — see the lead below about
> # the +10 972 against the 2026-08-08 baseline.
> cargo test -p chiero-vpp --test preprocess_corpus -- --ignored --nocapture
>
> # what chiero BELIEVES vs gcc: predefine definedness AND value. ~0.1 s. Expect 0 gaps.
> cargo test -p chiero-vpp --test persona_gap -- --ignored --nocapture
>
> # whether cost SCALES: gcov native ingest, two input shapes, n up to 12800. ~5 s.
> # 2026-08-09, six runs: `line` 4.1-5.8x, `onelin` 4.1-6.4x, against 8.0x. 4x is linear.
> # ⚠️ Neither shape is reliably the worse one and the band is ±1x — anything under 8.0x is
> # noise, not a regression. A five-run band called `line` 4.1-4.7 and run six gave 5.8.
> cargo test -p chiero-gcov --test growth -- --ignored --nocapture
>
> # 🆕 whether LAYOUT is right on shapes VPP does not contain: generated records vs gcc, with
> # clang as tiebreak. ~30 s. Found three `layout` defects in three runs (§7.20).
> # Expect: 585 records over 300 seeds, 0 DISAGREE, 0 refused, a few `matched clang`.
> cargo test -p chiero-sema --test generated_layout -- --ignored --nocapture
>
> # 🆕 whether CONSTANT FOLDING is right: generated integer constant expressions, graded by
> # `_Static_assert` so there is no output to parse. ~2 s. Expect 300/300 agree, 0 WRONG.
> cargo test -p chiero-sema --test generated_const_eval -- --ignored --nocapture
> ```
>
> 🆕 **And one more that is not a test at all, and was in this file zero times until
> 2026-08-09** — it measures the **M1 exit criterion**, which makes it the least discoverable
> important thing in the repo:
>
> ```sh
> # 080's M1 exit: "all numbered contracts of 020-024 are green". Counts spec contracts and
> # test citations of the form `NNN contract K`. ~1 s. Coverage, not correctness.
> cargo run -q -p xtask -- contract-coverage
> ```
>
> Measured 2026-08-09: **M1 166/167 cited** (020 44/44, 021 40/40, 022 31/31, 023 25/26,
> 024 26/26); frontend **124/126** after six contracts that were fully tested and cited nowhere
> got their citations (012 c20/21/22/23/24, 014 c4b — each read first, since a citation is a
> claim).
>
> ⚠️ **The three that remain uncited are not "untested", and they are three *different* things.**
> Flattening them into "unbuilt instruments" hides what to do with each:
>
> | contract | what it asks | verdict |
> |---|---|---|
> | **023 c17** | 1, 2 and 8 worker threads give identical `RunResult`s | **the feature does not exist** — `chiero-exec` is single-threaded, no `workers`, no `thread::spawn`, no rayon. Nothing to test; an owner call on whether M1's exit may name it |
> | **010 c18** | peak memory bounded by *one TU + the index*, not the sum over 100 TUs | ✅ **well formed and buildable** — a *shape*, not a number, so it cannot rot the way an absolute bound does. **Scoped 2026-08-09; the design is below.** This is the one worth writing |
> | **011 c12** | ≥100 MB/s lexing over a 50 MB blob | ⚠️ **ill formed by this project's own rule.** `CIRCUIT_STARTS`' doc states it: *"A counter, not a clock: a wall-clock bound silently stops being able to fail whenever the build gets faster"* — paid for by the verifier's 5-second assertion, which `opt-level = 2` disarmed. A throughput floor is that mistake in spec form. If lexing cost matters, it wants `growth.rs`'s shape: a **ratio** per 4x input |

> 🆕 **What 011 c12 should say instead, if the owner wants it kept.** *"Lexing cost grows
> linearly in input size: the time for 4x the bytes is under Nx, measured at several sizes."*
> That is testable, machine-independent, and cannot be disarmed by a faster build — the three
> properties the current wording lacks. ⚠️ **It was deliberately not written today**, because
> writing it and citing `011 contract 12` would claim coverage of text it does not test: the
> contract says ≥100 MB/s and a ratio says nothing about absolute speed. **A citation is a
> claim** — six were added today only after reading the test that justified each. Amend the
> contract first, then the test is honest.
>
> ✅ **Swept every contract in every spec for the same weakness — an honest zero beyond this
> one, and the sweep found a precedent that settles the recommendation.** Two passes: absolute
> resource numbers (`MB/s`, `MB`, `ms`, seconds), then any timing language at all
> (`reference machine`, `wall.?clock`, `throughput`, `faster than`, `elapsed`). 14 contract
> lines mention time; the triage:
>
> - **011 c12** — the only contract naming an absolute number with no sound instrument.
> - **011 c13** — *"re-lexing with a warm cache is ≥20× faster than a cold lex"*, a timing
>   ratio. ⚠️ **But its test does not time anything**: `session_cache.rs` asserts
>   `lex_cache_stats()`, a counter, with zero uses of `Instant` or `elapsed`. **The
>   counter-over-clock move was already practised here**, against a contract worded exactly like
>   c12. That is the precedent, in this repo, for amending c12 the same way — the recommendation
>   above is not a new principle, it is one this project already applied and did not write down.
> - **023 c6 / c17 / c24a, 050 c15b / c16** — these name `wall_clock` as a *configuration*
>   (determinism when no clock is set), not as a threshold. Well formed.
> - **070 c8** — a *documented* budget, a record rather than a gate.
>
> 📌 **010 c18 shows the pattern this follows.** Its memory wording had the same weakness, and
> the fix was to assert the *mechanism* with counters rather than the symptom with a high-water
> mark — no spec change needed, because the contract named a property (bounded by one TU, not
> the sum) rather than a number. **011 c12 names a number, which is why it needs the owner and
> 010 c18 did not.**
>
> **So one is a missing feature, one is a missing test worth writing, and one is a contract the
> project has since learned not to write.** Only the middle one is work.
>
> 🆕 **010 c18's design, scoped so it can be executed rather than re-scoped.** The API is already
> the right shape: `CookedExpansionIndex::cook_tu(&mut interner, &sm)` takes the `SourceMap` by
> reference and the caller drops it, and 010 §6.2's eager resolution is exactly the property
> under test — `ExpnCtx`/`MacroId` are indices into `sm`, so *not* resolving eagerly would force
> retention.
>
> - **Two arms over the same input, not one absolute number.** Cook N TUs dropping each
>   `SourceMap`, then cook the same N *retaining* them in a `Vec`. Assert the peak-memory ratio
>   between the arms is large. A ratio is the form `growth.rs` uses and the form that survives a
>   different machine; an absolute high-water figure is 011 c12's mistake.
> - **The high-water mark needs no dependency**: `VmHWM` from `/proc/self/status`, which is
>   monotonic per process — so the two arms must run as **separate processes** (or the second
>   arm's peak subsumes the first). A `#[ignore]`d test that re-execs itself with an env var, or
>   an `xtask`, is the shape that works.
> - ⚠️ **The trap, and why this was scoped rather than rushed:** an allocator that does not
>   return freed pages makes the dropping arm look identical to the retaining one, and test
>   parallelism pollutes a process-wide counter. **Prove the gate can fail before trusting it
>   green** — retain-the-`SourceMap` is the mutant, and it must produce a visibly different
>   ratio, or the test is measuring the allocator rather than chiero.
>
> ⚠️ **The single uncited M1 contract is 023 contract 17, and it describes a feature that does
> not exist.** It reads *"with `wall_clock: None`, running with 1, 2 and 8 worker threads
> produces identical `RunResult`s"* — and the engine is single-threaded: no `workers`, no
> `thread::spawn`, no rayon anywhere in `chiero-exec`. So M1's exit criterion, as written,
> includes a contract for an unbuilt capability, and the gate has been reporting 166/167 without
> anyone asking what the 1 was. **That is an owner-level fact about whether M1 can be declared
> done**, not a missing test: either 023 c17 is explicitly deferred as future work, or the exit
> criterion names it and cannot be met. ⚠️ Do not "fix" it by writing a test — there is nothing
> to test.
>
> **Not `#[ignore]`d, so `./check.sh` covers it — but worth knowing it exists**, because it is
> the only channel that asks the question in the dialect chiero ships in:
> `every_program_gnu11_gcc_accepts_silently_is_silent` in `chiero-sema/tests/generated_silence.rs`.
> Its sibling runs `-pedantic-errors` and therefore *skips* every gnu-only construct, so the two
> partition the corpus rather than sharing it.
>
> ⚠️ **2026-08-09: the corpus gate's token count moved with nothing to move it, and that is an
> open lead.** Re-run after the `from_specifier` parser change (the standing rule: run it when
> something touched the frontend). Result **1967/1967, 0 panicked, 0 diagnosed, 8 personas,
> 1360 s** — all unchanged — but **818 380 190 → 818 391 162 tokens, +10 972**. Checked, not
> assumed:
>
> - the parser change cannot affect preprocessing, and `git diff` over `chiero-pp`, `chiero-lex`,
>   `chiero-probe` and `chiero-vpp/src` for the whole session is **empty**;
> - the corpus is **content-identical** — VPP `HEAD` is still `7fe9c2669` and `git status` is
>   clean. Five files have mtimes after the baseline (including `vppinfra/vec.h`, which nearly
>   every TU includes), but their contents are unchanged, so a `touch` or a re-checkout, not an
>   edit.
>
> ✅ **Answered: the count is deterministic.** A second run at an unchanged tree gave
> **818 391 162 again, byte-identical**, in 1459 s. So the metric *is* a measurement and the
> coverage deltas this file records mean what they say — which was the question worth settling.
> What remains is a smaller, named lead: the +10 972 against the 2026-08-08 baseline is real and
> unexplained. Excluded so far — chiero (empty diff over the whole preprocessing path), and the
> tracked VPP content (HEAD unchanged, `git status` clean). The remaining candidates are
> environmental: gcc's predefines, which the persona probes from `cc` on every run, or a
> generated file the corpus reads that git does not track.
>
> *Original reasoning, kept because the discriminator was the useful part:* **either the count is
> not deterministic, or the environment moved** (gcc's predefines are
> the obvious candidate — the persona is probed from `cc` on every run). ⚠️ **This matters more
> than 0.0013% suggests**: the token *delta* is what this file cites as evidence of coverage
> ("+26M tokens, 3.3% more of VPP visible"), and a delta is only evidence if the number is a
> measurement. The discriminator is one more run at an unchanged tree — if it reproduces
> 818 391 162 the count is deterministic and something environmental moved; if it varies again,
> the headline metric is noisy and every delta recorded against it needs a band.
>
> ✅ **All three re-run at the end of the second 2026-08-08 session** — verified numbers, not
> remembered ones: `preprocess_corpus` 1967/1967, 0 panicked, **0 diagnosed**, **818 380 190
> tokens** (was 792 404 723 before the per-TU persona join), **8 personas, 0 failed probes**;
> `persona_gap` **0 gaps**, 15 values compared, 1 deliberate difference; `growth` fails as intended.
>
> ⏱️ **`growth`'s ratio is noisy — take it as a band, not a figure.** Three runs the same afternoon
> gave 15.2/14.7, 13.9/11.7 and (before the fixes) 14.0/31.4. The `onelin` 31.4 is stale; both
> shapes now sit together, which is itself the result of the acyclic early-out. Judge it by the
> 8.0x threshold it asserts, not by matching a past number.
>
> **They ask three different questions and each found something the others structurally could
> not** — the corpus gate cannot see a wrongly-taken `#if` branch (it emits no diagnostic), and
> neither of the first two can see a cost curve. Do not treat any one of them as coverage for
> the others.
>
> ✅ **`MemFault::BadRange` is CLOSED (2026-08-07)** — it degrades now instead of reporting.
> Next: §9.1's remaining live items. The `-march` item stays parked for the owner.
>
> 🆕 **The newest entry in the yield table is the most useful one: change the *kind* of corpus,
> not its size.** 141 preprocessor torture cases found three defects in a session, two of them
> panics on the C standard's own worked example, after four consecutive widenings of VPP-shaped
> corpora had begun returning honest zeros. §11.3 carries the general form.
>
> 🆕 **And the newest surface is the sharpest so far: 012 contract 17's configured-VPP
> preprocessor gate (§9.1 item 1c).** 1967 translation units under the flags VPP actually
> compiles them with, 18 minutes, **three real defects on its first run** — a missing `__linux__`
> that had killed every Linux-only branch in VPP *and* glibc, a diagnostic class chiero was right
> about and that was still noise, and `__has_attribute(error)` answered 0 where gcc answers 1.
> **None of them was visible to the pp-gate**, which has reported 0 findings for weeks: none is
> about preprocessing *syntax*. A gate that has been green for weeks is an untested surface.
>
> **State: 2026-08-09 — `./check.sh` GREEN at 2296 across 279 suites**, fmt and clippy clean.
> Up from 2281/277 at the previous session's end. Both fast gates re-run and unchanged
> (`persona_gap` 0 gaps; `growth` `line` 6.3x / `onelin` 4.8x against the 8.0x threshold);
> the VPP layout gate re-run after the sema change and unchanged at 2238 records / 10248
> assertions.
>
> 🆕 **A fourth standing gate, and it found a `layout` defect on its first run:**
>
> ```sh
> # generated record layouts vs gcc, with clang as tiebreak. ~1 min. Expect 0 DISAGREE.
> cargo test -p chiero-sema --test generated_layout -- --ignored --nocapture
> ```
>
> `585 records over 300 seeds | 581 agree | 0 DISAGREE | 4 matched clang | 0 refused`, 29 s.
> **It has now found three `layout` defects in three runs** — the prefix attribute, the
> zero-width bit-field in a union, and `packed` cancelling the zero-width flush. The second
> came only after the first fix was noticed to have made the gate's whole attribute dimension
> vacuous; the third came from adding a scalar class, and had nothing to do with that class.
> It exists because layout was graded only by 17 hand fixtures and 22 VPP header seeds, and
> **VPP contains not one prefix-attributed record**, so the 10 248-assertion VPP gate could
> never have seen the bug. See the yield table's two new rows and §7.20.
>
> **The 2026-08-09 session in one line: every tolerance list in the repo was read backwards, and
> three of the four were wrong.** §7.19 has it. The method generalises past this repo and is the
> thing to carry: **a list that excuses things is checked in one direction only — an item not on
> it is a violation, and nothing ever asks whether an item on it still excuses anything.** A stale
> entry is not inert; it silently re-permits the thing it names, so the decision gets made once
> and never revisited.
>
> 📌 **And the sharpest lesson was about the fix, not the defect.** The first version proved its
> new assertion fires by adding a bogus entry — mutating the **data**. With the list then empty,
> the assertion's *logic* was unreachable: a polarity flip survived the whole suite, and the only
> evidence it ever worked lived in a commit message. **Mutate the code that will still be there,
> not the input you can revert.** An adversarial review found this; asking it *"does this shape
> exist elsewhere"* rather than only *"is this right"* is what also produced the third instance.
>
> **This session's closes:** the witness reporting defect (§9.1 5e); the gcov growth gate, which
> passes for the first time; the stale VPP build directory (5d, four files rather than a rebuild);
> the AVX2/AVX-512 half of vppinfra, parsed and analysed for the first time — with
> `unsupported-access-width`'s recorded explanation corrected and the vector-overwrite shape
> pinned; and the whole persona thread — `chiero-probe` (the **24th** crate: one
> place that runs `cc -dM -E`, memoized per flag-set), `TranslationUnit::pp_config(&probe)` so the
> join cannot be skipped, **060 contract 2**, and §9.1 1e (the corpus gate stopped measuring a
> configuration nobody ships). Three defects found along the way, each by measurement rather than
> by review: a cache keyed on nothing, a failed probe answering "nothing predefined", and a
> "5 probes" figure that is 8.
>
> **This session's closes:** 060 contract 1 (`chiero_vpp::builddb` — VPP's compile database;
> 1967 C units → 423 configurations); 012 contract 17 (1967 TUs preprocessed under VPP's own
> flags, 0 panics, diagnosed 25 → 0); 012 contract 25 (system-header diagnostics separated, not
> deleted); `pick_entries.py --verify-cir`. Five persona defects, each found by measurement:
> the three spellings of `__linux__`, `__has_attribute(error)`, the endianness triple,
> `__SSE__`/`__SSE2__`, and `_LP64`/`__amd64`/`__SIZEOF_POINTER__`.
>
> 🆕 **A third gate, and a third question: `chiero-gcov/tests/growth.rs`** — does the cost *scale*?
> `#[ignore]`d, ~5 s, and as of 2026-08-08 it **passes for the first time**. Eight fixes have
> landed against it in two sessions; the last three came from *splitting the clock* between the
> line half and the arc half, which put 90% of the cost on the opposite side from the suspect this
> file had recorded. ⚠️ Passing is not linear — 6-7x against a linear 4x — and the header says
> where the remainder is.
>
> **Three standing gates exist now and they ask different questions.** `preprocess_corpus`
> (18 min) watches what chiero *says*; `persona_gap` (0.1 s) compares what chiero *believes*
> against gcc, in both definedness and value. The second found the worst defect of the day and
> the first could never have. §11.3 carries the general form.
>
> **Closed:** `MemFault::BadRange`; 023 §8's `max_solver_rlimit` and `max_memory_objects` — every
> budget in that sketch is now built; `--solver-rlimit` on the three solver commands; the CIR
> verifier's super-quadratic `dominators` and eight scans beside it; `vars_of` paying for the whole
> arena on every solver query; `globals_at_initial_value`; `[profile.dev] opt-level = 2`;
> `pick_entries.py --built-only`; **`chiero cir`**; and **021 contract 7b**, written and then met.
>
> **The `vnet/` sweep, characterised end to end** (423 entries, 44 findings → 40 after 7b):
> 17 `out-of-bounds` are indirect-call candidates and architectural — CIR pointers are untyped
> (020 §4.13b), so the candidate filter cannot tell `u32 *` from `u64 *`; 10 of the 15
> `pointer-outside-object` are static tables indexed by an enum-shaped value the function does not
> check, which is **true** under UCSE and a *policy* question; 1 is bounded by a helper's
> postcondition chiero cannot model. **None is chiero claiming something false** — fidelity is
> never `Exact` and the assumptions name the causes.
>
> ⚠️ **The rules this session paid for, in order of what they cost:**
> - **Instrument the boundary; do not reason about the code.** Three mechanisms were asserted from
>   reading source and two were wrong.
> - **A shared *message* is not a shared *cause*.** 19 findings attributed to one defect were 4.
> - **Read the last one.** At eight of eleven samples the shape looked universal; the eleventh was
>   a different cause.
> - **Check the file, not the exit status, when an edit is scripted.** Twice a commit message
>   described a change the tree did not contain — and it happened **three more times on
>   2026-08-09**, always the same way: a `python3` replace keyed on a comment or a multi-line
>   expression that `cargo fmt` had since **reflowed**, so the pattern matched nothing and the
>   script exited 0. The suite passed each time, which proves nothing about an edit that did not
>   happen. **The three habits that actually settle it**, in order of how often they paid:
>   `assert old in s` before writing, so a no-op is loud rather than silent; key on a **line
>   number** or a short unique token rather than on prose, because prose is what `fmt` rewrites;
>   and **grep for the text you removed** afterwards, not for the test result.
> - ⚠️ **And the same trap breaks *searches*, which is worse because a search that finds nothing
>   looks like an answer.** Auditing the project's four measurement counters on 2026-08-09,
>   `grep 'CONSERVATION_ARC_VISITS.with'` returned only the getter and the reset, and the
>   conclusion "this counter counts nothing" was a keystroke from being written down. It counts
>   fine: rustfmt had wrapped the increment so the identifier and `.with(` sit on different
>   lines. **Grep for the bare identifier, never for an identifier plus punctuation** — and the
>   only reason this became a corrected sentence rather than a committed one is that the claim
>   was measured before it was acted on. A search returning nothing is not evidence of absence.
> - **Ask whether a green test can fail.** Three tests could not, each caught only by mutating.
>
> **Open leads, none blocking:** the remaining `pointer-outside-object` policy question; the VPP
> build directory is **stale**; `pick_entries.py` names functions the preprocessor removes.
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

1. ✅ **UNPARKED AND CLOSED 2026-08-08 — the owner said "go ahead and design + execute the persona
   work", then "feel free to tackle march".** Both shipped; the original entry is kept below
   because its reasoning was right and is worth reading.

   **What was built.** A [`chiero_pp::Persona`] is a *named* set of predefines, and its file format
   **is `cc -dM -E` output** — so there is no new parser, no new dependency, and one is captured
   with `gcc -dM -E -x c /dev/null > personas/name.h` from any real compiler on any target.
   `Persona::baked()` names the set chiero always had; `Config::persona` replaces it. `--march` and
   any `-m…` flag go to the probe verbatim, since only the compiler knows what each implies.

       chiero cir <file>                    -> always
       chiero cir --march x86-64-v2 <file>  -> has_sse42, always
       chiero cir --march x86-64-v3 <file>  -> has_sse42, has_avx2, always

   **The two mechanisms are one.** `frontend::predefines` hand-parsed `-dM` into a `Vec` while the
   library baked 23 entries; both now go through `Persona::from_defines`, and the CLI only knows
   how to run a compiler.

   ⚠️ **Three bugs the gate caught and I would not have**: `add_predefined_object` wrapped values
   in a single synthetic *number* token — fine for a baked set of numerals, fatal for a real dump
   where `__PTRDIFF_TYPE__` is `long int` (now deleted, not left as a trap); `--march` did not skip
   its own argument, so the command failed outright; and the new test deleted the shared scratch
   directory out from under its neighbours.

   ✅ **Per-TU selection landed too — `TranslationUnit::target_flags`.** Measured on VPP:

       target flags: 1963 of 1967 C units carry one
       distinct -march: haswell, silvermont, x86-64-v2, x86-64-v3, x86-64-v4

   **Five targets, and every chiero measurement to date used none of them.** That is 060 §1.1's
   multiarch as a number rather than a warning. Kept apart from `defines`/`include_paths` because
   they *select a persona* rather than configure the preprocessor — only the compiler knows what
   `-march=haswell` implies, so they go to a `cc -dM -E` probe uninterpreted.

   📊 **Measured on the real corpus after the join, 2026-08-08 (1358 s, 22.6 min):**

       012 c17: 1967/1967 C units | 0 panicked | 0 diagnosed | 0 unreadable | 818 380 190 tokens
         personas: 8 distinct target flag-sets probed from cc

   **+25 975 467 tokens against the run before it — 3.3% more of VPP visible**, and 0 diagnosed
   throughout, which is the same pair of numbers the persona work has produced every time: the
   count says *no unaddressed complaint*, the token delta says *this much more of the program is
   now being read*. Only the second can see a branch taken correctly for the first time.

   ⚠️ **8 distinct flag-sets, where I had written 5 — in the crate doc, the spec and two tests.**
   The five distinct `-march` *values* are real; a flag-*set* is not a `-march`. VPP's carry
   `-mtune=generic`, `-mprefer-vector-width=512` and `-maes` alongside it, four units carry none
   at all, and **one names `-march` twice** (`x86-64-v2 … silvermont`, last wins). Every one of
   those is a case an interpreter would get wrong, which is the argument for handing the flags to
   a compiler verbatim. Enumerated with one `ninja -t compdb | python3` pipe — **the number was
   one command away the whole time and I wrote it from memory instead.**

   📌 **And it answers the standing "AVX2 has never been compiled" note.** All eight flag-sets
   probe cleanly on gcc 13.3 (401–432 predefines each), and **388 of VPP's 1967 C units now carry
   a persona that defines `__AVX2__`** — 192 at `x86-64-v3`, 192 at `x86-64-v4`, 4 at `haswell`.
   Four sets define `__AES__`. The duplicate-`-march` unit resolves to **silvermont's** 416
   defines rather than v2's, i.e. last-flag-wins is a compiler fact chiero correctly declines to
   model. That vector half of vppinfra is inside the corpus gate now; it is still outside every
   *findings* sweep, which drives its own configuration through the CLI.

   ✅ **Both halves closed 2026-08-08 — `chiero-probe`, and the join.** The 24th crate exists for
   one reason: `chiero-cli` and `chiero-vpp` both need "what does *this* compiler predefine under
   *these* flags", and a second `cc -dM` probe in `chiero-vpp` would have been the **third**
   mechanism for one fact (1b's whole complaint). `chiero-pp` stays free of subprocesses.

   - **The cache was keyed on nothing.** `system_environment` took the target flags and memoized
     the answer in a `OnceLock`, so within one process the *first* flag-set was answered to every
     later one. Latent while one process meant one operation meant one flag-set — and a sweep is
     exactly the case where it is not. `Probe::persona_probes()` counts the **subprocess**, not the
     call, so "one run per distinct flag-set" is measured rather than asserted (5 for 1967 units).
   - **`TranslationUnit::pp_config` now takes the probe** and joins `target_flags` → `persona`. A
     parameter rather than an option, for the reason the `ConfigId` is handed over ready-made: a
     caller that skips the join gets every `#if defined(__AVX2__)` in its `#else` and **nothing in
     the output says so**. 060 contract 2's structural half was already met; this is the half that
     makes it mean something — one source, N units, N *different programs*.
   - Mutants confirm both: dropping the memoization, returning the cache's first entry whatever the
     key, and passing `&[]` in place of the unit's flags each fail a test that named them.
   - **A defect in the new code, found by asking what a bad flag does rather than by review.**
     `cc -march=nonsense` exits non-zero and prints no `#define`, and `Persona::from_defines` over
     that text is a well-formed persona with **zero** entries — the worst answer available, since
     `__GNUC__`/`__linux__`/`__x86_64__` all undefined sends every header down its `#else`. The
     code asked whether the process *started*, not whether it *answered*. Now one rule, plus
     `failed_probes()`: the baked fallback is right and being handed it unknowingly is not, so the
     corpus gate prints each substitution and fails on any. (gcc 13.3 accepts all eight of VPP's
     flag-sets, so this is a guard rather than a live failure — also measured, not assumed.)

1z. **🗄️ Original entry, kept because its reasoning held up — closed, not parked.** It read:
   PARKED at the owner's request
   2026-08-07 — `-march`. Do not start without checking in;
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

   🆕 **2026-08-08 — `probe.sh` was run, since this entry names it as a prerequisite to the design
   rather than part of it. The answer is neither "intrinsics" nor "`#pragma GCC target`".** The
   five default TUs are clean at 900–1700 ms each. The `-march`-gated ones report **`clean` in
   1 ms**:

   ```
   vppinfra/test/aes_cbc.c   [clang, 3 target(s)]  {"status":"clean","ms":1}
   vppinfra/vector.c         [clang, 1 target(s)]  {"status":"clean","ms":1}
   vlib/main.c               [clang, 1 target(s)]  {"status":"clean","ms":1670}
   ```

   ⚠️ **A 1 ms "clean" is not a pass, it is an empty analysis** — and that is measured, not
   inferred. For `aes_cbc.c` under those flags **gcc itself preprocesses to 6 non-blank lines**,
   and `chiero cir` lowers **0 functions**. The body is entirely behind a guard the configuration
   does not satisfy, so chiero is correctly reporting a clean analysis of six lines of nothing. Same class as `--verify-cir` dropping
   all nine functions of `crc32_5tuple.c`, and the same class as the 8% of VPP the persona was
   hiding before today's predefine fixes. **So the intrinsics question cannot be asked yet:** the
   TUs that would answer it are not being analysed at all, and the failure is silent. Fix the
   configuration first and the question may look different — or may answer itself.

   *(`plugins/nat/cnat/cnat_node_vip.c` reports `NO TARGET`: not built in this configuration, so
   the `clib_crc32c_with_init` failure named below cannot be reproduced from this build directory
   either. See the stale-build-directory item.)*

   🆕 **2026-08-08 — the missing ingredient now exists, which changes the shape of the design (it
   does *not* unpark it).** When this was parked there was no mechanism that knew which `-march`
   any given TU used. 060 contract 1's `BuildDb` is that mechanism:
   `TranslationUnit::args` holds the full command line for all **1967** C compilations, including
   the **1964** `-march=` occurrences, keyed per translation unit, tested, with **423** distinct
   configurations already identified.

   Combined with the owner's config-file persona idea (1b), the natural shape is: **a persona is a
   named set of predefines, and `BuildDb` selects one per TU.** That turns `-march` from a new
   subsystem into a lookup over machinery that already exists and is under test. ⚠️ Still parked —
   the owner asked to discuss the design, and having a better design is not the same as having
   permission. Raise it together with 1b, 1d and 1e.

   ✅ **PARSED FOR THE FIRST TIME 2026-08-08 (second session), and it is an honest zero.** With
   `--march` reaching the persona, 24 sampled v3/v4 units lower with **0 diagnostics**, and the
   widening is real rather than nominal: **+292 definitions per TU at v3, +524 at v4** — the
   `vector_avx2.h` / `vector_avx512.h` inline bodies, which gcc confirms are 1516 and 2104
   preprocessed lines respectively. `tests/corpus/vpp-findings/march_probe.sh` is committed so the
   surface stays measurable; the *findings* sweeps still drive `chiero` with no `-march`, so for
   them the sentence below remains true.

   ⚠️ **The first reading of this was a false zero, and the instrument was the defect.**
   `grep -c '^func'` counts declarations too — a VPP TU has ~5000 — and it reported the v3 and
   no-march runs byte-identical at 5566. That would have been written into the yield table as
   "the widening measured nothing". The definition marker is `{ ; span`, and by it the same file
   goes 5560 → 5852. **An instrument that reports a plausible number is not a measurement**, and
   this file's own rule caught it: an unchanged number is a claim that needs checking, not a
   result.

   ⚠️ **CORRECTED 2026-08-08: the mechanism below is wrong, and the correction is the useful
   part.** `unsupported-access-width` is indeed zero everywhere — but *not* because the corpus
   cannot produce a 32-byte access. It produces them in bulk: the AVX-512 lowering of one TU,
   `vlib/handoff.c`, contains **7779 `copymem` of 32 bytes or wider** (4038 of exactly 32, 3740 of
   64). The finding class is unreachable because of the **access shape**, not the corpus: a vector
   access lowers to `copymem`, never to a wide `load`/`store`, which is 020 §4.13b's "no aggregate
   values in CIR" applied to `vector_size` types. Measured with a five-line probe:

       u8x32 load32 (u8x32 *p) { return p[0]; }
       ->  copymem %6 -> %13, 32i64 align 16

   So the whole class was ruled out by a decision this project made deliberately, and the "corpus
   cannot reach it" story survived because nobody had asked what the IR actually contained. What
   *is* still true: **none of the pinned 40 entries is compiled at v3/v4 at all**, so that corpus
   sees no vector code whatever the shape.

   ✅ **And the shape that *does* exist is caught, pinned, and mutation-checked.** A 32-byte vector
   store past a 16-byte object reports `out-of-bounds: 32-byte access at offset 0 of buf, which is
   16 bytes`, fidelity **Exact**, while the 16-byte store into the same object is clean —
   `crates/chiero-cli/tests/cli.rs`, end to end from C. Nothing covered it: `chiero-exec`'s
   `a_width_limit_does_not_mask_a_use_after_free` calls `Memory::read_term(.., 32, ..)` directly,
   which is the *wide-load* path, and C vector code does not take it. Two mutants die (`CopyMem`
   reporting no faults; a bounds check flagging every access ≥16 bytes), and the second is killed
   by the assertion that the message **names the width that overran** — a bare "a finding exists"
   would have let it through. `find-bugs` over 8 vector-using VPP entries gave 1 finding, a known
   class.

   🗄️ **Measured 2026-08-07, and it makes the item bigger than those seven entries.** Retaking the
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

1b. 🆕 **Owner's idea, 2026-08-08: personas defined in a config file rather than baked.**
   Raised while watching me hardcode five more predefines into `chiero-pp`'s engine, and it is
   the right shape for three separate things that are currently three separate problems:

   - **The baked persona is an impersonation with no name.** `__GNUC__ 13` / `__GNUC_MINOR__ 3`
     / `__x86_64__` / `__linux__` … is "gcc 13.3 on x86-64 Linux" written as a Rust array, and
     nothing in the code says so or lets a caller say otherwise. Every gap in it has been found
     the same way — by a corpus falling into a `#else` — and each fix has been another literal.
   - **`chiero-cli`'s `frontend` already captures a whole `cc -dM`**, so two mechanisms exist for
     one fact, and only one of them is available with `--no-default-features`.
   - **It was the natural seam for the then-parked `-march` item** (unparked and closed
     2026-08-08), which is exactly "the persona
     must vary per translation unit": VPP compiles one source repeatedly under different
     `-march`, and `__AVX2__`/`__SSE4_2__` are just more predefines. A config-file persona turns
     that from a new subsystem into a per-TU choice of an existing one.

   ⚠️ **Do not start it as a way around the parked item.** The overlap is the reason to raise the
   design with the owner together, not the reason to begin. What is safe to do first, and is
   independent of any design: keep using 012 contract 17's corpus run to find *which* predefines
   are missing, since that is evidence either design will need.

1c. 🆕 **The configured-VPP preprocessor gate is a live widening surface — three defects on its
   first run, and the standing job is to keep running it.** `cargo test -p chiero-vpp --test
   preprocess_corpus -- --ignored --nocapture` (18 min, 1967 TUs, 730M tokens). Diagnosed count
   as the metric:

   | | diagnosed | what the run said |
   |---|---|---|
   | first run, 2026-08-08 | **25** | 3 distinct causes |
   | after the platform predefines | **22** | `#error "Unsupported OS"` gone |
   | after 012 c25 + the attribute table | **0** | all three causes addressed |
   | final, all persona fixes in | **0** | 792,404,723 tokens — see below |

   🆕 **The number that actually matters is not the 0, it is the token count: 731,159,228 →
   792,404,723.** Same 1967 translation units, same flags, **+61 million tokens — 8.4% more C**.
   That is code in `#if` branches the persona had left dead: Linux-only paths in VPP *and* glibc,
   the little-endian layouts, the `__SSE2__` tables. **The library's default persona had been
   describing a program 8% smaller than the one VPP ships.**

   ⚠️ **Correction, made minutes after first writing this entry: it does *not* invalidate the
   published VPP findings sweeps.** I wrote "every 0 findings this project published over VPP was
   silent about that 8%" and it is wrong. `chiero-cli`'s `frontend::predefines` runs
   `cc -dM -E -std=gnu11` and captures the lot, so every sweep driven through the CLI already had
   `__linux__`, `__BYTE_ORDER__` and `__SSE2__`. What the baked persona actually governs is
   **(a)** every in-workspace test built on `Config::default()`, **(b)** 012 contract 17's corpus
   run, which takes its `-D`/`-I` from `builddb` but inherits predefines from the default, and
   **(c)** any library consumer that does not populate `Config::defines`. That is a large blast
   radius and a real defect — it is simply not the sweeps.

   The general form, since I have now made this error twice in one day: **before claiming a defect
   invalidates a published number, check which code path produced that number.** Two mechanisms
   for one fact (§9.1 1b) means a fix to one of them proves nothing about the other.

   ⚠️ It is also the honest way to read a green metric. The diagnosed count went 25 → 22 → 0
   while the endianness defect was live throughout, reversing bit-field member order across a
   whole plugin. **The count says "no unaddressed complaint"; the token delta says "and this much
   more of the program is now visible".** Track both — a corpus gate whose only number is a count
   of complaints cannot tell you it is looking at less than half the tree.

   ⚠️ **"0 diagnosed" means the preprocessor emits nothing, not that it is right about VPP**, and
   this file has the proof in the very next paragraph: the endianness defect was live during the
   run that first reported 0. Read the row as *no unaddressed complaint*, never as *correct*.

   The three, each a real fix and none guessed at:
   - **`__linux__` and its two other spellings** were not baked, so `vppinfra/pmalloc.c` reached
     `#error "Unsupported OS"` and every Linux-only branch in VPP *and* glibc was dead.
   - **`redefinition of macro MFD_CLOEXEC`** ×5 and `ELF_NOTE_ABI` — **chiero was right and it
     was still noise**; gcc suppresses diagnostics sited in system headers. 012 contract 25.
   - **`__has_attribute(error)` answered 0; gcc answers 1** — the persona's own documented
     failure mode, found in 20 TUs that all build `_FORTIFY_SOURCE=2`.

1e. ✅ **CLOSED 2026-08-08 — the gate now measures the shipped configuration.** Both facts come
   from `chiero_probe::Probe`, the same one the CLI uses: system include paths *and* the persona
   the unit's own `-march` selects. The private `gcc -E -v` scrape in the test file is gone, so the
   count of mechanisms went from three-if-I-had-done-the-obvious-thing to **one**. The run prints
   `personas: N distinct target flag-sets probed from cc`, because that is the number that moves
   when the join is wrong and no other number here does.

   *Original entry, kept for the reasoning that made waiting right:* **it measures a configuration
   nobody ships — and fixing it is
   blocked on the persona design, not on effort.** The gate takes each TU's `-D`/`-I` from
   `builddb`, its system paths from `gcc -E -v`, and its **predefines from `Config::default()`'s
   baked table**. `chiero-cli` does not: `frontend::predefines` runs `cc -dM -E -std=gnu11` and
   captures all 401. So the gate preprocesses VPP under a persona the real build never uses.

   That is precisely why the gate earned its keep — it saw the 8% because it used the baked
   table — but as a *standing* gate it should model the shipped configuration, and the persona
   gaps are now covered by `persona_gap` instead.

   🗄️ **Not started, deliberately** *(the marker was ⛔ and is now historical — a blocker emoji on
   a closed item is exactly the rot §11.3 says to sweep for)*. The obvious fix is to capture
   `cc -dM` in `chiero-vpp` too, which would make **three** mechanisms for one fact. The right fix
   is one mechanism — a named persona the preprocessor owns, which is exactly the owner's
   config-file idea (1b). Building `Config::from_compiler()` now would pre-empt that design.

   **And waiting was right**: 1b landed as `Persona`, and the one mechanism it wanted is now a
   crate that both callers share. Had this been "fixed" when it was filed, the fix would have been
   the third mechanism.

1d. 🆕 **`__STDC_VERSION__` is `201112L` in the persona and `201710L` under gcc — and this is a
   decision, not a bug.** VPP's compile commands carry **no `-std=` flag at all**, so gcc's
   default gnu17 applies and every glibc header configures for C17. chiero says C11.

   I did not change it, because changing it is a claim about the *parser*: 013 makes chiero
   "C11 + GNU extensions", and a persona announcing C17 over a C11 parser would be a worse lie
   than the one the persona gate exists to catch. C17 is editorially near-identical to C11, so
   the likely right answer is to say `201710L` and note that the delta is nil — but that is the
   owner's call about the language level, not a wave's.

   It is **visible on every run** rather than filed away: `chiero-vpp/tests/persona_gap.rs` prints
   it as `deliberate __STDC_VERSION__: chiero "201112L" vs gcc "201710L"` with the reason inline.
   Related to the config-file persona idea (1b) — a named persona is where a language level would
   naturally live.

   🆕 **A second instrument, and it finds what the gate structurally cannot.** Intersect gcc's
   401 predefines with the identifiers VPP actually tests in `#if`/`#elif`, subtract the
   persona's baked set. Runs in seconds, needs no build, and reported **8 gaps** — six fixed,
   two `-march`-gated and parked. Reproduce with the snippet in commit `4dc105a`'s test.

   The one it found is the sharpest defect of the day: **the persona had no endianness, and "no
   endianness" is not neutral — it is big-endian.** With `__BYTE_ORDER__`,
   `__ORDER_LITTLE_ENDIAN__` and `__ORDER_BIG_ENDIAN__` all undefined, `#if` reads each as `0`,
   so *both* of VPP's real call sites evaluated `0 == 0` and both branches were taken:
   `vppinfra/byte_order.h:11` was right by accident, and `srv6-mobile/mobile.h:41` took the
   **big-endian** branch — which redefines `BITALIGN2(A,B)` from `B; A` to `A; B`, declaring
   every bit-field struct in that plugin in reverse member order.

   ⚠️ **The corpus gate could not have found it.** Taking the wrong `#if` branch emits no
   diagnostic; contract 17 counts diagnostics. The general form is in §11.3: *a corpus gate finds
   what the tool says, never what it silently believes.* For the silent half you need a
   differential instrument — something that compares chiero's state against the real compiler's,
   rather than watching chiero's output.

   ⚠️ **This is why the gate is worth the 18 minutes.** The pp-gate reports 0 findings on its own
   corpus and has for weeks; none of these three was visible there, because none of them is about
   preprocessing *syntax* — they are about the persona and about which headers a real build
   reaches. **A gate that has been green for weeks is not evidence; it is an untested surface.**

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

3. ✅ **CLOSED 2026-08-08 — the ingest is built and both blocked contracts are met.**
   `chiero_vpp::builddb` (060 contract 1) and `chiero-vpp/tests/preprocess_corpus.rs`
   (012 contract 17). Three things worth carrying forward:

   **a. The blocker was in the interface, not the world.** 060 §1 wanted a
   `compile_commands.json` *file*; VPP's build writes none, and still doesn't. `ninja -t compdb`
   emits the identical format on stdout in 90 ms. Taking `&str` instead of a path closed a
   months-old blocker with no re-configure and no VPP edit.

   **b. `ninja -t compdb` dumps every edge, not every compilation — and I published the wrong
   number before catching it.** 2902 of VPP's 6235 entries are phony order-only rows: empty
   `command`, `output` like `cmake_object_order_depends_target_…`, `file` naming a *generated*
   source. My first measurement said "2226 C entries" and I wrote it into the spec, the module
   docs and a test table. **Real figure: 1967 C compilations** over 1562 sources, 208 built more
   than once (max 5, not the 9 the phony rows implied).

   It was caught only because the ignored corpus test asserts a *property* — every unit has an
   include path — and 259 rows had none. **A test that had merely counted would have agreed with
   the wrong number forever.** That is the general rule and it is cheap: when a corpus test can
   assert a property instead of a total, assert the property; the total cannot contradict itself.

   **c. What a `ConfigId` is worth, quantified.** Hashing exactly `-D` and `-I` — the flags that
   decide which `#if` branches exist — collapses 1967 units to **423 configurations**, 4.6×.
   Hashing the command line would make every unit unique and buy nothing. Both directions are
   asserted; mutation-checked.

   *(Original entry: the M2 note recording that no `compile_commands.json` existed. It was true
   when written. See §11.3's rule — **re-measure a blocker before routing around it**; this one
   cost nothing to check and had stood for months.)*

3z. **⚠️ Original entry, kept for the general form.** `compile_commands.json` is one command away,
   and two contracts have been blocked on its absence since M2.** `docs/reviews/m2-frontend-notes.md` records *"`…/compile_commands.json`
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

5e. ✅ **CLOSED 2026-08-08 (second session) — 950 KB → 11.9 KB, and the fix found a second
   defect in the fix.** `nsh_md2_encap`'s envelope is now 64 bindings plus an account of the rest;
   `Witness::digest` (chiero-exec, since 023 §9 owns what a witness is) bounds it, pinned bindings
   first, and nothing is reordered when nothing is dropped.

   **The fixture four earlier attempts missed, recorded because it cost four waves.** They all
   reached for `copymem`, which forks on the aliasing check against a lazy object — and the
   finding then lands on the *mint-free* fork, so the witness came out empty. **Straight-line
   loads at distinct offsets through the entry pointer** mint one byte each, do not fork, and put
   the fault after the mints: n loads, n + 3 bindings, ~96 JSON bytes each, linear to any size.
   The general form: *when four attempts to reproduce a symptom fail, check whether the layer you
   are reproducing it at is the layer the defect is in.* This was a reporting defect the whole
   time, and reporting is testable without an engine run at all — three of the six tests here
   construct a `Witness` directly.

   **Three things the wave got wrong and measurement corrected:**

   | claim | what the measurement said |
   |---|---|
   | "show the first 64" | the pinned bindings are the **last four** in the fixture — that bound would drop every value the finding depends on |
   | "pinned-first keeps what matters" | on the real case **10 580 of 10 594 omitted bindings are pinned**; `pinned` means the *model* gave it a value, and a total model pins nearly everything |
   | truncating the report is harmless | `check_reachable` licenses `proven` on *"a solver pinned every input"* — computed from a bounded view, a truncated witness would turn an unproven arrival into a **proof** |

   The last is the one to remember: **a bounded rendering must not become a bounded check.**
   `PathWitness::all_pinned` runs over the whole witness, and a chiero-exec test pins the property
   it rests on (a bounded view *can* be all-pinned while the witness is not).

   ⚠️ **One mutant survives and is recorded rather than hidden:** computing `all_pinned` from the
   digest instead of the witness passes every test, because killing it needs a witness with ≥65
   pinned bindings *and* one unpinned, which no fixture here produces. The guard is written at its
   source and commented; a future editor moving it has no test to stop them.

   Applied at **both** render sites — `find_bugs` and `check_reachable`, which solves for its own
   bindings and had its own unbounded rendering. §7.2's rule, and the reason `check_reachable`'s
   trap was found at all.

5c. 🆕 **Three `timeout` rows in `plugins/nsh/`** — `format_nsh_header`, `nsh_md2_decap`,
   `nsh_md2_encap`, from the widened sweep (2026-08-08). The verifier fix removed the cause the
   *old* `timeout` rows had, so this is a different one and nobody has looked. Sampling the stack
   under `gdb` found the last one in about two minutes; §11.2 carries the invocation and the
   `ptrace_scope=1` workaround. ⚠️ A `timeout` row is a run that measured **nothing** — it is a
   lead, not a statistic.

5g. ✅ **CLOSED 2026-08-08 — `pick_entries.py --verify-cir`.** It keeps only names that survive
   into the lowered module, using `chiero cir` (built earlier the same day, which is what made
   this tractable — before it there was no way to ask).

   **A filter, not a replacement, and the split is the point.** The CIR for one VPP `.c` names
   ~7000 functions, nearly all inlines from headers, and nothing in it says which file a
   `func @name` came from. **The text knows "defined in this file"; the CIR knows "survives the
   preprocessor".** Each is asked the question it can answer.

   Verified on the two files that produced `nofn` rows: it drops
   `clear_session_dbg_clock_cycles_fn` (inside `#if SESSION_DEBUG > 0`) and `compute_ethernet_key`
   — and it names what it dropped, because a corpus that quietly shrinks is one nobody can check.
   ⚠️ `crc32_5tuple.c` loses **all nine** of its functions: the file is behind an `__SSE4_2__`
   guard, so chiero lowers none of it. That is correct and it is the parked `-march` item showing
   through — the corpus now reflects chiero's *actual* configuration rather than the source text's.

   Off by default: it costs a `chiero cir` run per file. `CHIERO_FLAGS` carries the include and
   define flags, since lowering a VPP file needs them and the picker has no other way to know.

   *(Original entry: three `nofn` rows in the `vnet/` sweep, none of them the known macro-name
   problem — all three were real definitions the configuration removes.)* Three `nofn` rows in the
   `vnet/` sweep, and none is the known macro-name problem: all three are real definitions in the
   source. `clear_session_dbg_clock_cycles_fn` is inside `#if SESSION_DEBUG > 0` and
   `session_debug.h` defines `SESSION_DEBUG` as `0`, so it is absent from the *configured* TU.
   chiero is right and the row is honest.

   `--built-only` does not help — the file is compiled, just not that part of it. **The fix is to
   pick entries from what chiero lowers rather than from the text**, which makes every entry a
   function that exists by construction. Until then `nofn` is a corpus artefact, not a chiero
   limitation, which is exactly what the status was invented to make visible.

5f. ✅ **CLOSED — `--built-only` shipped 2026-08-08.** The sweep analyses files VPP does not compile — and that is how it found a real VPP
   defect.** `src/vnet/fib/fib_entry_src_default.c` defines `fib_entry_src_default_deinit`
   **twice**, at lines 22 and 35, both `static void … {}`. chiero refuses it; **gcc gives the
   identical error at the identical line** (`redefinition of …`); and the file is **not in the
   build at all** — zero of `ninja -t commands all`'s 2945 entries mention it, and
   `src/vnet/CMakeLists.txt` does not list it. It is dead source that has never compiled, which
   is exactly why nobody noticed.

   Two things follow, and the second is the actionable one:

   - chiero found a genuine VPP defect by reading a file the build ignores. Small, but real, and
     the kind of thing 050's tool surface exists to report.
   - **`pick_entries.py` globs `vnet/*/*.c` and `plugins/*/*.c`, so the corpus includes source
     the build never touches.** That inflates `failed` with rows that are neither chiero's
     problem nor VPP's compiled code. `ninja -C $VPPBUILD -t commands all` is the authoritative
     list and takes **63 ms** (§9.2) — filtering the entry list through it would make every
     `failed` row a statement about code that ships. ⚠️ Do this *before* the next sweep, or the
     residue keeps mixing two different kinds of rejection.

5d. ✅ **CLOSED 2026-08-08 (second session) — and the staleness was narrower than this entry
   claimed.** Measured before touching anything: **165 of 2629 sources under `src/` are newer than
   the whole build**, every one of them at the same timestamp — a single checkout **22 seconds
   after the build finished**. Of those 165, **4 were `.api` files**, and only those 4 could
   matter.

   ⚠️ **The correction, and it bounds the problem:** chiero reads `src/` **directly**, so a `.c` or
   `.h` that has moved on is read as it is today — nothing is stale about it. The only derived
   artifacts chiero includes are the **1049 `*.api*.h`** headers and four `config.h`/`version.h`
   that come from cmake options rather than from source. So "chiero reads a slightly different
   program from the one VPP would build" was true only of the generated headers, which is a
   surface a script can check in a second rather than a reason to rebuild anything.

   **Fixed by running the exact `vppapigen` command ninja would run for each output** — *not* by
   `ninja`, whose target for a generated header depends on a **cmake re-run**, which rewrites
   `build.ninja`: the file `chiero_vpp::builddb` reads for all 1967 compile commands, that
   `probe.sh` replays, and that 012 contract 17's corpus gate is built from. Verified after:
   compdb still 6235 entries / **1967 C compilations**, byte-for-byte the same count.

   **The before and after, both ways round:** with the stale header, `chiero cir lldp_api.c` says
   *"no member named `last_heard_age`"* at 135:7 and **gcc reports the identical error at 135:12**
   — which is what made it an environment fact rather than a frontend defect. Regenerated, the
   same file lowers **6796 functions**, and `lldp_cli.c` and `lldp_test.c` with it.

   📌 **`tests/corpus/vpp-findings/api_staleness.py` is committed** (§9.2's rule: the instrument
   goes in the repo in the same wave). It reports drift and exits 1, `--fix` regenerates. Checked
   that it *can* fail by ageing one header: reports 1 stale, `--fix` clears it. A minute to run,
   and the class of failure it catches — a sweep row that looks exactly like a frontend bug — cost
   a wave to diagnose the first time.

5b. 🆕 **Audit `Vec` + `.contains()` on paths that scale — the shape, not the site.** The
   verifier fix above is the **second** time this exact defect class has been found in
   `crates/chiero-cir/src/verify.rs`. Seven hundred lines above `dominators`,
   `check_module_identity` already carries: *"Sets, not vectors. These were `Vec` with
   `contains`, which is O(n^2) — invisible while a module held dozens of entities… one measured
   673 s against ~1 s before. The scaling was the giveaway."* Methodology and all. **The fix went
   to the function where the symptom appeared and its neighbour in the same file had the same
   flaw.**

   `grep -rn "\.contains(&" --include=*.rs crates/*/src xtask/src | grep -vE "IndexSet|IndexMap|BTreeSet|HashSet|BTreeMap|HashMap"`
   returned **87** sites when the audit was written; **79 at `ee1b251` (2026-08-08)**, after the
   `chiero-cir` half closed and four `chiero-gcov` sites became `IndexSet`/`IndexMap`. By crate
   now: `chiero-cir` 10, `chiero-sema` 9, `chiero-gcov` 9, `chiero-exec` 6, `chiero-tool` 5.

   ⚠️ **The count is a rough progress marker and nothing more.** Today's 3.08x in `chiero-gcov`
   came from two of those sites — and three *other* `contains` conversions in the same file moved
   the ratio by nothing at all. **The grep finds the shape; only a curve finds the cost.** Most are ranges (`(0x300..=0x36F).contains`) or genuinely small fixed
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

   ✅ **All three below were converted 2026-08-08 — and the reading was two-thirds wrong.** The
   first two moved the growth ratio by *nothing* (14.7x → 15.4x); the third, dismissed here as
   merely "adds a factor", was the whole cost, and fixing its `bs` and `blocked`/`block_lists`
   membership gave **17.31 s → 5.61 s (3.08x)**. Kept unedited below because the misranking is
   the lesson: **the two that looked quadratic were not, and the one described in passing was.**

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

   🆕 **UNBLOCKED 2026-08-08, and the triage below was wrong about where the cost is.**
   The artifact block was false: a `.gcno` need not be found or hand-written — **gcc emits one of
   any size from generated C**. `if (x == i) r += i;` repeated *n* times gives Θ(n) blocks and
   Θ(n) arcs in one function, and running the binary writes the `.gcda`. The whole instrument is
   `crates/chiero-gcov/tests/growth.rs`, committed and `#[ignore]`d.

   **Measured — native arc ingest is quadratic** (4x per 4x arcs is linear, 16x is quadratic):

   | | 50→200 | 200→800 | 800→3200 |
   |---|---|---|---|
   | before | 6.3x | 12.2x | **14.7x** |
   | after the three `contains` fixes | 3.3x | 11.1x | **15.4x** |

   ⚠️ **The three sites named below are not the bottleneck.** They are `IndexSet`s now (commit
   `23ba416`) — strictly better, no scan and no clone-per-probe — and **the ratio did not move**.
   That is this file's own warning landing on the person who wrote it down: the CIR verifier entry
   two paragraphs up says *"the ratio had not moved, so only the constant had"*, and here not even
   the constant moved.

   🆕 **And then the curve itself turned out to be the problem.** The generator put one statement
   per source line, so every line carried one block. Adding a second shape — all statements on
   *one* line, which is what a multi-statement macro expansion produces and VPP is macro-heavy —
   changes the answer completely:

   | shape | 200→800 | 800→3200 | n=3200 |
   |---|---|---|---|
   | `line` (one statement per line) | 11.5x | 16.4x | 1.10 s |
   | `onelin` (all on one line) | 21.2x | **50.5x** | **17.1 s** |

   **A growth curve is only as good as the shape it grows.** A generator that varies one parameter
   while holding the interesting one at 1 reports a clean answer forever — and three wrong
   conclusions came out of exactly that below.

   It also **rehabilitates the hypothesis dismissed with a bad argument**: `cycles_count` was ruled
   out because the generated code has no loops, but its cost is `for &start in bs { circuit(...) }`
   — one DFS per block *on the line*, which runs whether or not a cycle exists. It is the only
   thing in the file that scales with blocks-per-line. **Chase the `onelin` curve.**

   ✅ **Diagnosed with a counter, and the first real win landed 2026-08-08.**
   `native::circuit_starts()` counts every `circuit` entry, recursion included:

   | shape | n=200 | n=800 | n=3200 | growth |
   |---|---|---|---|---|
   | `line` | 405 | 1 605 | 6 405 | 4x — linear |
   | `onelin` | 20 504 | 322 004 | **5 128 004** | 16x — **quadratic** |

   ⚠️ The counter **refuted its own first placement**: counting only the outer `for &start in bs`
   loop gave 6405 for *both* shapes while one ran 17x slower. The cost is not how many traversals
   begin, it is how far each walks.

   **Fixed so far:** `bs.contains(&w)` — a linear scan in the innermost recursion — is now a
   `Vec<bool>` indexed by block. **17.31 s → 8.36 s at n=3200 (2.07x)**, ratio 50.3x → ~38x, call
   count unchanged as it should be. *(clippy then caught that `bs` was being passed through
   `circuit` only to reach its own recursive call. Dropped.)*

   ✅ **Second fix, same day:** `blocked`/`block_lists` were index-correspondent parallel `Vec`s,
   so lookup was `iter().position(..)` and release was two O(n) `remove`s. One
   `IndexMap<u32, Vec<u32>>` with `swap_remove` replaces both. **8.36 s → 5.61 s.**

   **Cumulative: 17.31 s → 5.61 s (3.08x)**, call count untouched at 5 128 004 throughout — which
   is the check that both were cost-per-call changes and not accidental semantic ones, and the
   full suite agrees.

   ✅ **Third fix, and it is the algorithmic one: skip the enumeration when the induced subgraph
   is acyclic.** `cycles_count` started a DFS at *every* block in `bs` whether or not a circuit
   existed; Kahn's algorithm answers that in O(V+E) once. **5.61 s → 1.05 s**, circuit calls
   **5 128 004 → 0**.

   **Cumulative: 17.31 s → 1.05 s (16.5x)**, and the blocks-per-line pathology is *gone* — the two
   curve shapes differed by 17x in the morning and now sit within noise of each other at ~1.1 s.

   ⚠️ **Not "the enumeration is fixed".** The curve's input is straight-line, so the early-out
   fires everywhere and the counter reads 0. **A function with a real loop still pays the full
   O(V × (V+E))** — what changed is that straight-line code no longer funds a search that cannot
   succeed. The cycle path stays covered by the `cyc.gcno` fixture.

   **Still open:** ~14x per 4x arcs against a linear 4x, so `tests/growth.rs` still fails on
   purpose, and **the remaining cost is unlocated**.

   ⚠️ **Tested and ruled out after the early-out landed:** the `accumulate_line_info` predecessor
   hoist was re-applied on the theory that `cycles_count` had been masking it. It changed nothing
   again (1.00 s / 1.07 s against 1.05 s / 1.15 s — noise) and was reverted a second time. So the
   scan genuinely is not the cost, at either scale.

   ✅ **LOCATED 2026-08-08 — `solve_arcs`' conservation fixpoint**, by counter
   (`native::conservation_arc_visits()`), not by reading:

   | n | 50 | 200 | 800 | 3200 |
   |---|---|---|---|---|
   | arc visits | 254 992 | 3 898 192 | 61 670 992 | **983 962 192** |
   | ratio per 4x arcs | | 15.3x | 15.8x | **16.0x** |

   Quadratic, and the **wall-clock ratio is also 16.0x** — the counter tracking the clock is what
   makes this the answer rather than a sixth plausible site. It is identical for both curve
   shapes, which fits: once the acyclic early-out landed, the remaining cost stopped caring about
   blocks-per-line.

   ✅ **FIXED the same day.** Incidence lists built once instead of `(0..n).filter(..)` per block,
   per side, per iteration:

   | | before | after |
   |---|---|---|
   | conservation arc visits, n=3200 | 983 962 192 | **153 728** |
   | ratio per 4x arcs | 16.0x | **4.0x — exactly linear** |
   | `onelin` n=3200 | 1.05 s | **0.090 s** |

   **Cumulative across the four fixes: 17.31 s → 0.090 s, 192x.** Order-preserving by
   construction — `(0..n).filter(|i| arcs[i].to == b)` *is* the arcs into `b` in ascending index
   order — so the conservation arithmetic never changes, only how often the graph is re-derived.

   ⚠️ **I had marked this ⛔ "for its own wave" one commit earlier and then did it anyway.** The
   caution was about my remaining context, not about risk: fifteen lines, order-preserving, and
   the 2249-test suite is precisely the watch I said it needed. Worth noticing which of those two
   things a ⛔ is actually recording — **"I am nearly out of budget" and "this is dangerous" are
   different facts and only one of them should outlive the session.**

   ✅ **Fifth fix — and it is the change reverted twice earlier the same day.** With
   `cycles_count` and the conservation fixpoint both gone, `accumulate_line_info`'s predecessor
   hoist finally shows an effect: `line` 10.5x → **8.6x**, `onelin` 10.0x → **7.6x**.

   ⚠️ **A null result is scoped to the conditions it was taken under.** "This change does nothing"
   was measured honestly, twice, and was true both times — it stopped being true when the costs
   hiding it were removed. **Re-test reverted optimisations after the dominant cost moves.** The
   curve makes that a 25-second question rather than an argument.

   **Session total on this item: 17.31 s → ~0.068 s, ~250x across five fixes** (two runs;
   ±20% variance at these times) — four of which
   would have been got wrong by reading.

   **Still open, and the number is bigger than it first looked.** The curve now runs to
   **n=12800**, because at n=3200 the ingest had dropped to ~0.1 s — where process startup, file
   I/O and gcc's output size are a visible share of the clock, and a "ratio" is partly noise. The
   added point says the residual is real:

   | shape | 3200 → 12800 | |
   |---|---|---|
   | `line` | 0.109 s → 1.390 s | **12.8x** |
   | `onelin` | 0.082 s → 0.880 s | **10.7x** |

   Roughly n^1.8 against a linear 4x, while the conservation counter stays **exactly linear**
   across the same step (153 728 → 614 528) — so the fix that landed earlier still holds at four
   times the size, and the residual is elsewhere.

   ⚠️ **An instrument that has stopped discriminating still prints numbers**, and they look just
   as authoritative. This curve had stopped growing past the point where its subject dominated the
   clock — the same failure as the input-shape blindness in its own header, twice in one file.

   ⚠️ **Tested and ruled out: `acc.shift_remove(key)`** at `native.rs:1023`. It sits inside
   `for (key, bs) in &on_line` and `shift_remove` is O(n) on an `IndexMap`, so it reads as a
   textbook quadratic — Θ(lines) × O(|acc|). Swapping it for `swap_remove` moved the **ratio**
   not at all (`line` 12.8x → 12.6x, `onelin` 10.7x → 13.3x, i.e. noise) though it did cut ~30%
   of the constant. **Reverted un-shipped**, because a constant-factor win is not worth changing
   `accumulated`'s iteration order, which feeds downstream merges. *Measured as an experiment and
   thrown away — that is the cheap way to hold an opinion.*

   ⚠️ **Also ruled out: `cycles_count`'s per-call allocations.** It is invoked once per *line*
   — Θ(n) times — and each call sized `in_bs` and `indegree` by the **max block index**, i.e.
   Θ(n), for Θ(n²) in allocation alone. It also predicted `line` being worse than `onelin`, which
   is what the curve shows. Replacing both with |bs|-sized structures moved nothing (13.7x /
   10.9x against 12.8x / 10.7x). Reverted; the experiment was deliberately semantically wrong for
   cyclic input and existed only to answer the question.

   ⚠️ **The elimination above was wrong, and measuring it took two minutes.** `records()` and
   `read_notes()` are both public, so the curve now times them separately — no change to the
   solver to get the numbers:

   | phase | growth per 4x arcs | share of the 1.37 s at n=12800 |
   |---|---|---|
   | `records()` byte decode | 4.4x — **linear** | 0.4% |
   | `read_notes()` structure build | 3.9x–5.1x — **linear** | 1.5% |

   Conflating those two is what sent the elimination astray: "the parse" was one name for two
   different amounts of work, and both turned out innocent.

   **So every component measured is linear and the whole is still ~13.5x.** The `.gcda` decode was
   split out too (linear, 3.5–4.8x); at n=12800 **all decode plus structure build is under 3% of
   the clock**, leaving ~97% in the post-decode pipeline.

   ✅ **One genuine quadratic found there and fixed:** `cycles_count` is invoked once per *line*
   and sized `in_bs`/`indegree` by the **max block index** — 327 808 014 cells at n=12800, 16.0x
   per 4x arcs. Scratch is now allocated once per function and reset over `bs`; the counter reads
   **4.0x, linear**.

   ⚠️ **And the wall clock did not follow — 1.39 s → 1.32 s, ratio 13.5x either way.** That is the
   more useful result: **a quadratic counter is not automatically the bottleneck.** 328M bool
   writes is real, really was quadratic, and is also just a memset — perhaps 7% of the run. Kept
   for the asymptotics; it would dominate at larger inputs.

   ⚠️ **It had also been "ruled out" earlier the same day** by an experiment that stubbed
   `circuit`'s argument. Flagged as unclean at the time, re-tested, and the hypothesis was right.

   ✅ **LOCATED AND FIXED 2026-08-08 (second session) — and the suspect named below was wrong.**
   Not by another counter: by **splitting the clock**. `arc_coverage` runs the whole ordinary line
   ingest (`ingest_into`) and *then* walks the functions again for the arc index — "the post-decode
   pipeline" was one name for two amounts of work, the same conflation that sent the parse
   elimination astray one measurement earlier. Timing them apart put **90% of the clock on the line
   half**, i.e. the opposite side from the `ArcCoverage` index building this paragraph nominated.

   Two throwaway experiments then bisected it in four minutes — skip `line_counts` + `object.add`
   (ratio barely moved), then also skip `block_counts` (15.4x → 4.9x):

   | fix | what it was |
   |---|---|
   | `block_counts` | every block scanned every arc — Θ(blocks × arcs), **90% of the ingest** |
   | `acc.shift_remove(key)` per graphed line | O(\|acc\|) each; now one order-preserving `retain` |
   | `bs.contains(&from)` in the entry-arc sum | quadratic in blocks-per-line; now an indexed bool |

   **1.42 s → ~0.24 s at n=12800**, `line` 15.4x → 6.2–7.0x, `onelin` 11.7x → 4.7–5.8x, and the
   gate passes for the first time. ⚠️ **Passing is not linear** — the `line` shape's line half is
   still 8–9.5x, in three `IndexMap`s keyed by `(String, u32)` plus the per-object merge. Do not
   tighten the 8.0x threshold: the run-to-run band is ±0.8x.

   🆕 **Narrowed 2026-08-09, and the narrowing is free: the residual is a function of distinct
   source lines *at fixed arc count*.** The two curve shapes reach n=12800 with the **same**
   arcs — `conservation 665734` for both — and differ only in how many lines those arcs sit on:

   | at n=12800 | distinct lines | line-half | its growth |
   |---|---|---|---|
   | `line` | ~12800 | 0.1684 s | **8.8x** |
   | `onelin` | ~1 | 0.0320 s | 3.4x |

   ✅ **FOUND AND FIXED the same day, by the counter this entry called for.** `cycles_count`'s
   `cs` was `vec![0; f.arcs.len()]`, allocated and zeroed on **every call**, and it is called
   once per line — Θ(n²) cells for `line`, Θ(n) for `onelin`. `in_bs` and `indegree` had been
   hoisted for exactly this reason, with a comment saying so; `cs` was left behind. **One fix
   applied to two of three buffers.**

   | `line` at n=12800 | before | after |
   |---|---|---|
   | line-half | 0.1684 s | **0.0467 s** |
   | its growth | 8.8x | **4.0x — linear** |
   | share of the clock | 69% | 44% |
   | worst overall | 7.6x | 5.9x |

   📊 **Five runs after the fix, and the picture inverts — with a consequence for the
   threshold.** A single run is not a band, and this curve's band is the whole reason §9 says
   not to tighten:

   | shape | worst per 4x, **six** runs | was |
   |---|---|---|
   | `line` | 4.1 4.7 4.1 4.5 4.5 **5.8** — max **5.8** | the bad one, 8.8x |
   | `onelin` | 4.3 4.7 5.4 **6.4** 4.1 4.8 — max 6.4 | the good one, 3.4x |

   ⚠️ **The sixth run fell outside the band the first five established, and the correction is the
   point.** After five runs this entry said `line` was *"essentially linear, 4.1–4.7"*; run six
   gave **5.8x**. So the honest statement is a range with its sample size attached — `line`
   4.1–5.8 over six — not a claim of linearity. **Five samples were not enough to bound a
   quantity whose band is ±1x**, which is the same over-reading as concluding from one run, at a
   larger sample. Both shapes still pass 8.0x with room, and `line` is still transformed from
   8.8x; that part holds.

   ✅ **And the gate is measuring the right thing — checked, because the wide band suggested it
   might not be.** The worry was that "worst per 4x" might be coming from the noisy small-*n*
   steps, which would make the headline a measure of jitter rather than of growth. Every step,
   one run:

       line     3.0 -> 3.5 -> 3.9 -> 4.4      onelin   2.9 -> 3.6 -> 4.8 -> 3.9

   **The small steps are consistently the tamest**, and the worst comes from one of the two
   largest — where it should. So the band is wide because the *large-n* measurement is noisy on
   a loaded machine, not because the gate is reading the wrong end of the curve. An honest zero
   on the hypothesis, recorded so the next reader does not re-suspect it. ⚠️ So
   **"do not tighten the 8.0x threshold" still holds, for a completely different reason than
   when it was written**: it was `line`'s superlinearity, and it is now `onelin`'s run-to-run
   noise, which touches 6.4x. Tightening to 7.0x would sit 0.6x from an observed value. **A
   conclusion outliving its premise is the thing this session kept finding; here the conclusion
   survived the premise changing, and the reason had to be rewritten under it.**

   📌 And a false alarm pre-empted: a future reader seeing `onelin` at 6.4x should not chase it
   as a regression. The band above is what one machine produces with nothing changed.

   ⚠️ **The counter is why it survived two sessions.** `CYCLES_CELLS` counted the two hoisted
   buffers and not the one still sized per call, so it read *"4.0x, linear"* while the clock
   stayed superlinear — **a wrong measurement that agreed with the fix already made**, which is
   the worst kind. Adding the missing term made it 15.9x on `line` against 4.0x on `onelin` and
   located the defect in one run. *A counter that omits a term is not a smaller measurement, it
   is a wrong one.* The confirmation is stronger than the timing: the cells counter is now
   **identical for both shapes** (176138 → 704138), so the line-count dependence is gone rather
   than reduced.

   *The narrowing that led there, kept because it was free and did the work:*
   **5.3x more time for identical arc work, and the superlinearity is entirely on the `line`
   side.** That rules out everything scaling with arcs, blocks, or blocks-per-line — which is
   most of what the earlier waves chased — and leaves only what is done *per distinct line*.
   ⚠️ Note it also rules out the obvious reading of the suspect already recorded here: an
   `IndexMap<(String, u32), _>` over one filename hashes and clones a string per operation,
   which is **linear** in the line count, not 8.8x. So the recorded suspect is at best the
   constant, and something per-line is O(lines²)-ish. **Next step is a counter on the per-line
   operations, not a reading** — the curve makes each hypothesis a 25-second question, and this
   entry's own scoreboard is 6 refuted / 5 held.

   ⚠️ **The middle fix is the one to remember: it had been measured and honestly ruled out earlier
   the same day**, and that null result was *true* — while `block_counts` was 90% of the clock,
   nothing else could move the ratio. **Re-test reverted optimisations after the dominant cost
   moves**, which this curve makes a 5-second question.

   *Original text, kept because it is the seventh refuted hypothesis on this item:* **Still
   unlocated: the time residual.** The next counter must measure something whose unit
   tracks *time* — allocations, hash lookups, `IndexMap` probes — in the `ArcCoverage` index
   building (`line_blocks`, `counts`, `tests`, `order`, each keyed by a `FuncKey` holding two
   `String`s and cloned per insert). That is the largest block still measured only as part of a
   whole.

   *Method note, and it is the cheap thing to copy:* every hypothesis here cost about a minute —
   change it or time it, run the 25-second curve, revert regardless of the result. **Scoreboard:
   6 refuted, 5 held.** Nothing was shipped on a reading, and the six refutations include three
   that predicted the observed shape correctly and were still wrong about the cause.

   Scoreboard on this entry: **4 hypotheses wrong, 5 right.** Every wrong one looked obvious in
   the source; every right one came from a counter or a curve.

   ⚠️ *Kept below: three hypotheses that were tried first and moved nothing.* **Do not read.
   Profile.**

   | hypothesis | how it looked | ratio after |
   |---|---|---|
   | the three `Vec::contains` sites §9.1 named | a scan inside a loop | 15.4x (was 14.7x) |
   | Johnson's circuit enumeration, ~1250–1290 | `Vec` membership in the innermost recursion | not the path — the curve's input has **no loops at all**, so there are no cycles to enumerate |
   | `accumulate_line_info`'s arc scan, ~980 | `for (key, bs) in &on_line { for &b in bs { for a in f.arcs` — textbook lines×blocks×arcs | 15.4x, unchanged; **reverted** |

   Each looked obvious. Each was wrong. The third was hoisted into a predecessor map built once
   per function — the exact fix that took the CIR verifier from hours to 2.4 s — and it changed
   nothing measurable, so it was reverted rather than left in a numerically sensitive solver as
   an unproven edit.

   ✅ ~~**What is actually needed is a profiler**~~ — **written mid-investigation and wrong within
   the hour.** `ptrace_scope=1` really does block attaching to a running `cargo test` binary, the
   recorded gdb recipe needs the target to be gdb's own *child*, and `perf` is not installed — all
   true, and none of it mattered. **A counter settled it in one edit**
   (`native::circuit_starts()`), exactly as `verify::terminators_examined()` had settled the
   verifier, and the same sentence that reached for a profiler already said so.

   Kept as a correction rather than deleted, because the reflex is the thing to notice: **"I need
   a better tool" is usually cheaper to answer with "I need a number".** A profiler tells you
   where time went; a counter tells you *why*, survives a faster build, and goes in the repo where
   the next reader gets it for free.

   ⛔ *Original entry, kept because its reasoning is the thing that turned out to be wrong:*
   **`chiero-gcov`'s half is blocked on artifacts.** There are **no
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

5h. 🆕 **The dominant finding class on `vnet/` is a false positive, and its cause is
   architectural.** Sampling the 44 findings — nobody had — shows most of the 17 `out-of-bounds`
   are one shape: *"N-byte access at offset 0 of the M-byte unnamed local"*, M < N.

   Worked through on `vnet/crypto/node.c`'s `crypto_dequeue_frame`, which calls a **function
   pointer** — `(hdl) (vm, &n_elts, &enqueue_thread_idx)` — passing the address of a `u32`. The
   envelope names the cause itself: `max_indirect (16) reached` and `unresolvable callee`. So
   chiero dispatched the indirect call to a candidate declaring a wider pointee, and that
   candidate's 8-byte store landed in a 4-byte local.

   ⚠️ **The candidate filter cannot catch this, and that is a consequence of a locked decision,
   not an oversight.** The rule is `(CTy::Ptr, Value::Ptr(_)) => true` — any pointer parameter
   accepts any pointer argument — and it can be no sharper, because **020 §4.13b makes CIR
   pointers untyped**: the pointee type lives on `Load`/`Store`, so a call site does not record
   whether an argument was `u32 *` or `u64 *`.

   ✅ **Reproduced minimally 2026-08-08**, and the reproduction settles what to assert.
   `an_indirect_call_width_mismatch_is_reported_but_never_proven` in
   `chiero-tool/tests/find_bugs.rs`: a call through a function pointer passing `&(i32)`, and a
   candidate that stores 8 bytes through it. Same message as VPP's, exactly.

   ⚠️ **It does not assert the finding away, and that is deliberate.** chiero's claim is *true* if
   the pointer can name that candidate — and it can, because the type CIR discarded is the only
   thing that would say otherwise. Asserting its absence would decide a design question by
   fixture. What the test pins instead is the property that must hold whichever way the design
   goes: **the envelope names the premise.** `Exact` here would be a lie, and a reader who cannot
   see `max_indirect` and the unresolvable callee cannot tell this from a real bug. A mutant that
   stops emitting assumptions kills it.

   📌 **A design that fits the evidence rather than reopening §4.13b:** an indirect-call candidate
   whose *own* access faults past the extent of an object the caller passed is, by that fact,
   the wrong candidate — a real program cannot have made that call. Using the fault to **reject
   the candidate** rather than to report a finding costs nothing when the candidate is right and
   removes the class when it is wrong. Worth designing before implementing; it interacts with
   `max_indirect` and with what "the search was cut" then means.

   The honesty machinery is doing its job throughout — fidelity is `Approximated`, never `Exact`,
   and the assumptions name `max_indirect` and the unresolvable callee. **This is noise a reader
   must filter, not a claim chiero got wrong**, which is a different and much smaller failure.

5i. 🆕 **The other dominant `vnet/` class, `pointer-outside-object` (19 of 44 before the 7b fix,
   **15 of 40** after — see below), and a precise
   open question.** They cluster on a very common C idiom: a **static array indexed by a value
   from a lazily-materialised struct**, where the program *does* guard the index.

   `vnet/dev/counters.c`:

   ```c
   char *units[] = { [VNET_DEV_CTR_UNIT_BYTES] = "bytes", … };   /* 5 pointers, 40 bytes */
   if (c->unit < ARRAY_LEN (units) && units[c->unit])
   ```

   chiero: *"a pointer into units (40 bytes) can be computed at offset 48, which is outside it"*.
   Offset 48 is index 6, and `c->unit < 5` excludes it.

   ✅ **Settled by reading, and it is the opposite of the tempting answer.** The check is
   `self.probe(a, s, &[out])` where `out` is `offset < 0 || offset > size-1`, and `probe` builds
   `PathCondition::from_parts(s.path.clone(), …)` — so the query is *"given this path, can the
   offset be outside?"* **It is fully path-sensitive**, and the witness comes from the model
   rather than from `obj_size`, which a comment there records as a fix for exactly the
   naming-an-impossible-input failure.

   So `PointerOutsideObject` is **not** reporting an unconstrained range, and the design is not
   the noisy one. What follows is sharper: offset 48 is satisfiable *under chiero's path
   condition*, which means that condition is **weaker than the program's guard**. The envelope's
   own assumptions point at why — `NoInformation` twice and `UnmodeledCall` — and 023 §3 takes a
   branch the solver cannot decide *anyway*, leaving the state `path_unchecked`. An undecided
   `c->unit < ARRAY_LEN (units)` therefore never constrains the offset.

   📌 **And the envelope names what weakened the path — it is not an undecided branch.** Repeated
   through the assumptions: `ModelApproximate :: 'format': havoc: symbolic contents, reachable
   pointers to depth 1 — N object(s) invalidated`. `format` is VPP's unmodeled printf-alike, and
   the code reads:

   ```c
   s = format (s, "%s", c->name);                       /* havoc invalidates c's object */
   if (c->unit < ARRAY_LEN (units) && units[c->unit])   /* c->unit read from havoc'd memory */
   ```

   ❌ **Hypothesis raised and refuted the same hour — do not chase it again.** The story was that
   repeated reads of one address in havoc'd memory yield *different* symbols, so the guard binds
   one and the subscript another. Tested directly: an unmodeled call, then two `load i32` of the
   same address, then `br (a != b)`. **One state, returning 0** — the inequality is decided false,
   so havoc'd reads are stable. 021 §6's twin holds already: not knowing a value is not
   permission to give it two.

   ⚠️ **The cause is still open, but reading the full finding records narrows it and corrects two
   things I had asserted.** Fidelity is **`Unknown`**, not `Approximated` — that was generalised
   from the out-of-bounds class and is wrong for this one. And the duplicate pairs are not a
   deduplication gap: `units[c->unit]` appears in *both* the guard and the body, so two source
   sites give two findings, which is right.

   The `unwitnessed` text is the lead: *"this path reads the contents of an object written by code
   with no model (ObjectId(38)), **whose value is a whole array rather than a number**"*. The
   havoc'd object was promoted to an SMT **`Array`** (020 §4.13b's `ite_threshold`), and there is
   no witness because a witness binds numbers.

   ✅ **Tested through `Array` too, and this is the answer.** Same shape — promote an object past
   `ite_threshold` with a symbolic-offset store, then two `load i32` of one address, branch on
   `a != b`. `Bytes` gave **one** state; `Array` gives **two**, `fidelity: Unknown`, and the
   assumptions say it outright, once per load:

   > `a load produced no value, so its result is invented`

   ⚠️⚠️ **RETRACTED WITHIN THE HOUR — the probe did not test what I claimed.** I wrote that "a load
   from an `Array`-promoted object invents per read" and that this explains the `units` finding.
   The probe promoted an object with a symbolic-offset store and then read a byte **nothing had
   ever written**. That byte is *genuinely uninitialized*, and inventing a fresh value per read
   may well be correct there — reading indeterminate memory twice is not obliged to agree.

   The `units` case is a **different** input: `format` havocs the object, and 024 contract 21e
   makes an unmodeled extern's havoc `HavocInit::Symbolic`, **not** `Uninitialized` — precisely
   because "an unmodeled extern handed a pointer *wrote* something there". Symbolic contents
   should read back stably.

   ✅ **All three combinations are now tested, and the story is dead.**

   | object | contents | two reads of one address |
   |---|---|---|
   | `Bytes` | havoc'd (symbolic) | **stable** — one state |
   | `Array`-promoted | havoc'd (symbolic) | **stable** — one state |
   | `Array`-promoted | never written | unstable — two states, and defensible: reading indeterminate memory twice is not obliged to agree |

   So unstable reads do **not** explain the `units` finding, and the guard-versus-subscript story
   is finished. What remains true and unexplained: the offset check is path-sensitive (it probes
   `s.path`), the guard is `c->unit < 5`, and offset 48 is nonetheless satisfiable.

   ✅ **The free check is done and it sharpens the contradiction rather than resolving it.**
   `chiero-lower` short-circuits `&&` properly (`lib.rs` ~3756: a block for the right operand, a
   short-circuit block, a join), so `units[c->unit]` is lowered into a block reached **only when
   `c->unit < ARRAY_LEN (units)` is true**. The `PtrAdd` is downstream of the guard.

   ❗ **So the pieces contradict, and that is the state to hand over.** The offset check probes
   `s.path`; the `PtrAdd` sits under the guard; reads are stable in every representation tested;
   and the report only fires on `CheckResult::Sat`, meaning the solver **found a model** where the
   path holds *and* the offset is 48. With `c->unit < 5` on the path, index 6 should be
   unsatisfiable. One of those four is false and none is obviously so.

   ✅✅ **REPRODUCED 2026-08-08, minimally.** `chiero cir` (built for this) showed the guard and
   the subscript each `load i8` from `c + 34` *separately* — so the guard constrains load **A**
   and the subscript uses load **B**, and the finding exists only if `A < 5` and `B * 8 == 48`,
   i.e. `A != B`. Reducing from there:

   | fixture | result |
   |---|---|
   | two loads, same block, `Bytes` + havoc | stable |
   | two loads, same block, `Array`-promoted + havoc | stable |
   | two loads, same block, `Array`-promoted, never written | unstable — and defensible |
   | lazy object, guarded, **no** havoc | **constrained**: indices 0..4 only |
   | the same with the guard's `udiv 40/8` unfolded | constrained |
   | **lazy object + havoc + guard** | **offset 48** — the VPP message exactly |

   **The ingredient is the havoc *plus* the fork.** Two loads in one block after a havoc agree;
   two loads either side of a branch do not. A guard that binds one of them constrains nothing.

   The reproduction is committed as `probe_lazy_two_loads` in `chiero-tool/tests/find_bugs.rs`,
   **`#[ignore]`d** — it fails, and the suite stays green, so the next person gets an executable
   minimal case rather than a paragraph: `cargo test -p chiero-tool -- --ignored probe_lazy`.

   📌 This is 021 §6's family — *not knowing a value is not permission to give it two* — and
   §11.3's rule applies: **do not fix the site; ask which read path does not end in a stable
   symbol across a fork.**

   ✅ **MEASURED AT THE MEMORY BOUNDARY, 2026-08-08** — after two wrong mechanisms guessed from
   reading source:

   ```text
   READ obj=2 off=0 value=Some(Term(3))   raw=[] live=[]   <- the guard's load
   READ obj=2 off=0 value=Some(Term(27))  raw=[] live=[]   <- the subscript's load
   ```

   **Two reads of one address return different terms**, no faults, on the non-null path. The
   guard binds `Term(3)`; the subscript indexes with `Term(27)`; nothing relates them, so index 6
   is satisfiable and the pointer lands at offset 48.

   Three controls, each measured:

   | change | result |
   |---|---|
   | remove the `call` | **passes** — a lazy object alone is stable |
   | add a load *before* the call | **passes** — the object is materialised first |
   | `--entry-ptr-nonnull`, as the VPP run used | still fails — the null path is not it |

   **The ingredient is a lazy object plus an unmodeled call.** The call's havoc promotes the
   object, and reads afterwards mint a fresh symbol each time instead of returning the one that
   is there.

   ⚠️⚠️ **Two mechanisms were asserted on this entry before this and both were wrong** —
   *"havoc'd reads are unstable"* and *"the havoc's write fails and the loop breaks silently"* —
   each plausible, each taken from reading the source. The `READ` line above is the first
   statement here measured at the boundary the values actually cross. **On this entry, instrument
   the boundary; do not reason about the code.**

   ✅✅ **FIXED 2026-08-08 — 021 contract 7b, written then met.** `materialize_fresh` asked
   `o.sym_at(k)` (the `Bytes` side) and stored a promoted object's mint into `arr.data` alone, so
   the question went to one representation and the answer into the other and **every read minted
   afresh**. The symbol is now recorded on both; `probe_lazy_two_loads` loses its `#[ignore]` and
   is the contract's test.

   📌 **It is `memoize_via`'s bug one field over** — that helper exists because "the
   initialization lives in an array, so writing the mask was a no-op there", the `init` mask was
   fixed and `sym` was left. A `sym_via` twin would have been the third copy of one asymmetry
   waiting for a fourth field, so both sides are written in one place instead.

   ⚠️⚠️ **RE-MEASURED, AND THE PREDICTION WAS WRONG.** I said the sweep should lose *most* of the
   19 `pointer-outside-object` findings. Same 417 entries, same flags, only the fix different:

   | kind | before | after |
   |---|---|---|
   | `pointer-outside-object` | 19 | **15** |
   | out-of-bounds | 17 | 17 |
   | null-dereference / uninitialized-read | 4 / 4 | 4 / 4 |
   | **total** | **44** | **40** |

   **It accounts for 4, not 19.** The fix is right and contract 7b is met, but the class has more
   than one cause and I attributed all of it to the first one I found — the fourth time on this
   entry that a whole category got pinned on a single mechanism.

   🔍 **Two of the remaining 15 sampled (2026-08-08) — and they are not defects at all.** Both
   index an array with a value **nothing in the function checks**:

   - `vnet/dpo/lookup_dpo.c`: `lookup_input_names[lkd->lkd_input]`, where `lkd` comes from
     `lookup_dpo_get(index)` — a lazily-materialized object, so `lkd_input` is unconstrained.
   - `vnet/dpo/dvr_dpo.c`: `dvr_dpo_db[dproto]`, where `dproto` is an **entry parameter** of enum
     type indexing a six-element array.

   Neither has a guard. chiero is **right**: call `dvr_dpo_add_or_lock` with `dproto = 9` and you
   index out of bounds; C's enum type does not stop you. These are true statements conditional on
   UCSE's premise — the same family as `globals_at_initial_value`, and a **signal-to-noise**
   question rather than a correctness one: is *"this function does not validate its enum
   parameter"* worth a finding?

   **Eight of fifteen sampled, all the same shape**, and the remaining seven share the array
   naming:

   | site | index | guard |
   |---|---|---|
   | `lookup_input_names[lkd->lkd_input]` | field of a lazy object | none |
   | `dvr_dpo_db[dproto]` | entry parameter, enum type | none |
   | `qos_source_names[qs]` | `va_arg (*args, int)` | none |
   | `mfib_entry_src_vfts[msrc->mfes_src]` | field of a lazy object | none |
   | `fed_formatters[fed->mfd_type](fed, s)` | field of a lazy object — **then called** | none |
   | `ip_null_action_strings[ind->ind_action]` | field of a lazy object | none |
   | `ip4_main.fib_masks[len]` | prefix-length parameter | none |
   | `ip_null_dpos[indi]` | derived index | none |

   Every array in all fifteen is a small `static` dispatch or format table — `*_names`,
   `*_strings`, `*_vfts`, `*_db`, `*_cfg`. **The class is: a static table indexed by an
   enum-shaped value that the function does not check.** chiero is right about every one; C's
   enum type constrains nothing at the ABI, and `va_arg(*args, int)` least of all.

   📌 **So the open question is a policy one, and it belongs to the owner.** These are true, they
   are numerous, and they are almost certainly not what a reader wants first. Options, none free:
   accept them as findings; constrain an enum-typed entry value to its declared range (which C
   does not guarantee and VPP's `format_qos_source` visibly does not); or keep them and **rank**
   — 050's envelope already carries the machinery to say "true, and here is the premise", which
   is how `globals_at_initial_value` handles the same tension.

   ✅ **Census finished — all eleven distinct arrays read — and it found the exception.** Ten are
   the shape above. **One is not:**

   ```c
   const char *strings[sizeof (vnet_hw_if_caps_t) * 8] = { … };   /* one entry per bit */
   int bit = get_lowest_set_bit_index (caps);
   if (strings[bit]) …                                            /* vnet/interface/caps.c */
   ```

   The index **is** bounded — by `get_lowest_set_bit_index`'s postcondition, one entry per bit of
   the type — and chiero cannot see it. That is a **second cause**: not "the function does not
   check" but "the check is a helper's contract chiero does not model". It wants a different
   answer from the other ten, and grouping by message would have hidden it.

   ⚠️ **Finishing the census is what found it.** At eight samples the shape looked universal and I
   had already written it up that way; the eleventh site disproved it. Twice on this entry a class
   has looked like one cause and been more — 19 findings that were 4, and now ten-of-eleven that
   is ten-and-one. **Read the last one.** `pointer-outside-object` says the offset can leave the object; that can
   happen for as many reasons as there are ways to lose a constraint. The next one needs the same
   treatment from scratch — pick one, `chiero cir` it, instrument the boundary — and not the
   assumption that it is this bug again.

   §10 exists for exactly this: **re-measure after a fix, not only before.** The prediction was
   confident, cheap to check, and wrong.

   *(Historical, and the reason the fix landed in this order: 021 was silent, so it needed a
   sentence before it needed code.)* §3.1 says a lazily-materialized object is "fully `Yes` with unknown *values*", and
   contract 7 says reading its bytes yields no finding — **neither says that two reads of one
   address give the same value.** No written contract is violated by the behaviour above, which
   is exactly how it survived.

   So the design decision is: *state* that a byte's value is stable within a path once
   materialized, add it as a contract, and then the implementation follows and is testable. The
   committed reproduction becomes its test. ⚠️ Do not fix the code first — a rule this basic
   being absent is why 021 §6's family keeps recurring, and the eleventh instance will land the
   same way if only the tenth site is patched.

   *(Historical: the blocker before this was a missing instrument, not a missing idea.)* Settling it needs the
   *actual lowered CIR* for `format_vnet_dev_counter_name` — which term the guard constrains and
   which term the `PtrAdd` uses — and **there is no way to dump it**: no CLI operation prints a
   module, and 020's textual format is reachable only from Rust. §4.11 lists `get_cfg` among the
   tool operations and it does not exist.

   📌 **So the next move is to build that**, not to guess a fifth time: a `chiero cir <file.c>
   [--entry <fn>]` that prints the lowered module in 020's normative textual format. It is small,
   it is specified, the printer already exists and is round-trip tested — and every remaining
   question on this entry is one `grep` away once the CIR can be read.

   📌 And read `chiero-lower/tests/symbolic_offset_store.rs` first: it carries six waves of
   analysis of this exact sentence, ending at a real cause — `report_faults` discharges faults for
   *reporting* and the value decision then consults the **raw** list, so a proof that was paid for
   is ignored where the value is chosen. `a_concrete_byte_written_before_promotion_survives_it`
   passes, so that half is fixed. **Do not re-derive any of that.**

   Between this and 5h, **36 of the 44 `vnet/` findings are characterised**: one class traced to
   an architectural cause, one to a precise open question. Neither is chiero claiming something
   false — fidelity is `Approximated` throughout and the assumptions name the causes.

5k. ✅ **CLOSED 2026-08-08 — an enum's declared underlying type was parsed and thrown away, and
   `layout` reported the wrong size as `proven`.** `struct S { enum small s; char c; }` with
   `enum small : unsigned char` came out **8 bytes, align 4**; gcc says **2 and 1**. The envelope
   said *"proven — this holds for all inputs (Exact)"*.

   **VPP declares 22 of these across 6 files**, all `typedef enum name_ : u8` — `quic/quic.h`,
   `http/http_buffer.h`, `vperf/builtin/vperf_builtin.h` — so every struct holding one had the
   wrong size, silently. Verified on VPP's own form after the fix: `2`/`1`, matching gcc.

   The chain was three correct decisions and one missing link: the parser parsed the `: T` and
   discarded it *with a comment saying the representation "is what 014 owns"* — right about the
   ownership — and 014 was never given it, so sema fell back to the implied type. Sema already had
   the machinery (`enums: Symbol → TyId`, 014 contract 10). The fix is one AST field
   (`TypeKind::Tag::underlying`, 7 sites), the parser keeping what it already parsed, and
   `enum_ty` preferring it. The enumerator fitting still runs, because that is what produces the
   pedantic diagnostics and silencing those would hide a real ISO complaint.

   📌 **Found by auditing a class, not a site.** `let _ = <named parameter>;` across the
   workspace's sources: 8 hits, 4 unexplained, and this was the one with a consequence. The audit
   itself came out of 5j, where an *undocumented* deliberate discard cost a wrong RED — so the
   lesson paid for the next wave immediately. The other three unexplained discards are noise
   (`chiero-diff`'s loop index, `chiero-pp`'s matched `(` token, `chiero-cir`'s explained-below `t`).

5m. ✅ **CLOSED 2026-08-09 — `chiero layout` applies the same rule `lower` does**: errors refuse
   the TU, advisories are printed and do not. Six lines, identical in shape to the path ten
   lines above it — which was the point, since the two frontend entries had different answers
   to one question and one of them was "do not ask". The entry is kept below for the reasoning.

5m-orig. 🗄️ **`chiero layout` ignored sema diagnostics entirely** — found by the 2026-08-09 review,
   **pre-existing**, not from that session's changes. `chiero-cli/src/frontend.rs`'s second
   frontend path never looks at `analysis.diagnostics` at all, so a TU containing a hard error
   (an undeclared name) still produces a layout report stamped **`proven — this holds for all
   inputs (Exact)`** and exits 0, with the diagnostic never printed. It contradicts the module's
   own header ("Every stage's diagnostics are a refusal") and now also the severity policy
   `lower()` implements ten lines above it. The severity work did not cause this; it made the
   inconsistency untenable.

5n. ✅ **CLOSED 2026-08-09 — and the fix reached one reader of three.** `_Alignof(A_t)` is 16
   now, matching gcc, for all three typedef spellings. ⚠️ **The sema fix passed its unit test
   while `chiero cir` went on emitting the old numbers**: lowering has its own `AlignofType`
   arm that asks `align_of` on the resolved `TyId` and falls back to sema's fold only if that
   returns `None` — which it never does for a complete type, so the correct fallback was
   unreachable. Checking the original reproduction rather than the new test is what caught it,
   and an end-to-end test now pins the constant in the CIR. **Third time this session one fact
   had multiple readers disagreeing.** Original entry:

5n-orig. 🗄️ **`_Alignof` of a typedef never saw an `aligned` attribute on the typedef.** Also
   pre-existing. gcc gives `_Alignof(A_t) == 16` for
   `typedef __attribute__((aligned(16))) struct A { char a; } A_t;` and chiero says 1; the
   post-declarator spelling fails the same way. ⚠️ Member layout *through* the typedef is
   **correct** (`struct Holder { char c; A_t m; }` is 32 with `m` at 16, matching gcc), so this
   is narrow. The `from_specifier` fix moved the wrongness from "the record itself was 16",
   which was worse, to "the typedef name is 1".

5j. 🆕 **`CopyMem` discards the alignment the CIR hands it, so a memcpy and a vector move are the
   same access.**

   🆕 **2026-08-09: both discard sites are now explained *at the site*** (the account lived only
   here and in the test), **and the gap has a measured second half that points the other way.**
   Lowering emits `align 1` for a packed member — `store i32 7i32 -> %5 align 1` for
   `struct __attribute__((packed)) P { char c; int v; }` — which is the compiler saying the
   access is deliberately unaligned and handled. `Memory` re-derives `want = 4` from the access
   *size*, so ordinary legal C is misaligned as far as the model is concerned. **The recorded
   half under-reports and fails silently; this half would fail loudly, on legal code, the day
   `ub-strict` ships.** `a_scalar_access_in_an_align_1_object_is_reported_misaligned_today` pins
   today's behaviour and is written to fail when the operand is threaded through. Diagnosed 2026-08-08, not fixed, and the diagnosis cost a wrong RED that is worth
   reading before anyone starts.

   The chain, all measured: a `u8x32` access lowers to `copymem …, 32i64 **align 16**`;
   `chiero-exec` drops it (`let _ = align;`); `Memory::copy` writes through `write_bytewise`, which
   **strips `Misaligned` deliberately** because a copy is defined byte by byte as `memcpy` is; and
   `align_fault` derives its requirement from the access *size* and gives up above **16 bytes**, so
   it could not express a 32-byte requirement even if it were asked.

   ⚠️ **The strip is right and had no comment, and I nearly "fixed" it.** A RED was written and
   committed asserting a misaligned copy must record the misalignment — false: a byte write has no
   alignment requirement. **An undocumented deliberate behaviour is indistinguishable from a
   defect.** It is documented at its source now and `chiero-mem/tests/copy_alignment.rs` keeps the
   reasoning rather than the wrong assertion.

   What is actually wrong: C *does* distinguish these accesses — vppinfra has `u8x32` and `u8x32u`
   precisely because an aligned 32-byte move requires 32-byte alignment — and the CIR carries the
   distinction the model then throws away. Nothing changes in a report today, since the engine
   filters `Misaligned` until a `ub-strict` mode exists (021 §5 step 3). **It decides what that
   mode will see, and a mode built on a blind path is worse than no mode.** Fixing it means
   threading `align` through `Memory::copy` and lifting `align_fault`'s 16-byte bound — an API
   change across two crates, worth doing when `ub-strict` is.

6. **`InstKind::Call` carries no result type**, so an indirect call's result width is whatever
   candidate ran. The arity and parameter-type filters cut the wildest cases and cannot close it;
   the engine survives the rest by degrading. The real fix is a CIR change.

   ⚠️ **"135 sites construct `InstKind::Call`" was wrong, and it is the sentence that kept this
   item unstarted.** Measured 2026-08-09 — 135 is the count of *mentions*, most of them pattern
   matches, and 113 are in `chiero-exec`, which executes CIR and constructs none. The real shape:

   | | |
   |---|---|
   | production **constructions** | **3** — `chiero-lower/src/lib.rs:3484`, and the text parser's two spellings at `text.rs:1015` and `:1111` |
   | production matches to update | `text.rs:1889` (printer), `verify.rs:341/726/800`, `chiero-exec` 3, `chiero-opt` 5 |
   | test fixtures | ~110, of which **82 are in `chiero-exec/tests/step.rs`** — mechanical, and the bulk of the work |

   ✅ **HALF CLOSED 2026-08-09, with no CIR change and no fixture touched.** The item reads as
   one architectural change gated on a new field; it is two, and nobody had separated them.
   **Direct calls never needed the field** — `Callee::Direct` names a `FuncId` and `Function`
   carries `ret`, so `defined_by` returns the callee's declared type now. The only work was
   threading the module down two signatures, which `verify_function` already had. Direct call
   results are checked by `require_ptr`/`require_ty` like any other value, verified reaching
   real lowered C (`char *g(void)` → `Ptr`, `int f(void)` → `Int(32)`), and `./check.sh` stayed
   green — no latent defect surfaced and no false positive.

   📌 **And the audit that followed it is the reassuring kind.** Applying §11.0's top lesson to
   this exact fact — *who else decides a call result's type?* — `chiero-exec` was **already**
   reading `f.ret` from the module (`lib.rs:4136`), and has been. So the verifier was the one
   reader that did not ask, and the fix aligned it with the existing source of truth rather than
   inventing a second one. **An honest zero on "who else disagrees", and it is evidence the fix
   was the right shape**: the alternative — a side table, or a field the verifier maintains
   itself — would have created the divergence that was not there.

   **What is left is genuinely the indirect half**, which is also where §7.6's finding class
   lives: `Callee::Indirect` carries an operand rather than a signature.

   🆕 **And it is ~25 sites, not ~110 — because the field belongs on `Callee::Indirect`, not on
   `Call`.** Measured 2026-08-09: `Callee::Indirect` is mentioned **25 times across 11 files**,
   ~14 of them production (`chiero-lower` 4, `chiero-exec` 3, `chiero-cir` 4, `chiero-opt` 3).
   Every direct-call fixture — including the 82 in `step.rs` — is untouched by a change there.

   ⚠️ **The placement is not just cheaper, it is the correct one, and the recorded design was
   wrong about it.** A field on `Call` would make *direct* calls carry a copy of the callee's
   `ret` that is already in the module: **two sources of truth for one fact**, which is §11.0's
   top lesson with four instances behind it, introduced deliberately. On the variant, each
   `Callee` carries exactly what cannot be derived — nothing for `Direct`, the signature for
   `Indirect`.

       Callee::Indirect { target: Operand, ret: CTy }

   What it still needs, and none of it is architectural: the variant change and its ~25 sites,
   text-format syntax for the annotation (parser at `text.rs`, printer at `:1889`) with a
   round-trip test, and `defined_by`'s `Indirect` arm returning `ret` instead of `Void`. **The
   `require_ptr` Void exemption can then go**, which is the check that has been switched off for
   every call result since the verifier was written.

   📌 **And the payoff is one line.** `verify.rs:726` reads
   `InstKind::Call { dst: Some(d), .. } => vec![(*d, CTy::Void)]` — the verifier types every
   call's result as `Void` because there is nothing else to say. That line becoming the declared
   type *is* the fix; everything else is plumbing to let it.

   ⚠️ **Do not reach for a side table** (`Module::call_result_ty: IndexMap<InstId, CTy>`) to
   avoid the fixture churn. It would spare ~110 mechanical edits and create a second source of
   truth for one fact, which is §11.0's top lesson with four instances behind it. The field is
   right and the fixtures are its price.

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

   🆕 **STAGED 2026-08-08 — `tests/corpus/replay/replay_probe.sh` is rebuilt and committed.**
   The predecessor was written once, lived only in a scratch directory, and was gone by the next
   session (§9.2), so this one is in the repo and its whole first duty is not to lose the tree:
   it refuses a dirty `src/`, refuses an unknown commit, and restores on **every** exit path
   including SIGINT. `--check` exercises the revert and restore mechanics with no build at all —
   verified: 4 files reverted, tree clean afterwards, both refusals fire.

   ⛔ **The expensive half was NOT fired, and the reason is a measurement rather than caution.**
   A real run must build, and `ninja` regenerates `build.ninja` whenever a `CMakeLists.txt` is
   newer than it — **four of VPP's are** (`vnet`, `plugins/nsim`, `plugins/unittest`,
   `drivers/armada`). `build.ninja` is what `chiero_vpp::builddb` reads for all 1967 compile
   commands, what `probe.sh` replays, and what 012 contract 17's corpus gate is built from. So a
   run of the probe **invalidates the baseline every published VPP number was taken against, even
   when it succeeds**. That is a trade worth making — but knowingly, and with the numbers re-taken
   afterwards, not as a side effect of a wave that was about something else.

8b. 🆕 **The build graph is four `CMakeLists.txt` behind `src/`, and that qualifies every VPP
   number in this file.** Measured 2026-08-08: `build.ninja` was generated at 23:31:38 on
   2026-08-05 and the tree moved 22 seconds later. Checked rather than feared — `vnet/sfdp`, the
   subsystem those changes add, **is** in the database with 21 entries, so no subsystem is hidden.

   And the "1967 C compilations over 1562 sources" figure decomposes cleanly, which it had never
   been made to do:

   | | |
   |---|---|
   | 1967 compilations, 1562 distinct sources | 208 sources built more than once (multiarch) |
   | **147 of the 1562 are generated** | `*.api_test2.c` under the build dir, not under `src/` |
   | 1415 are `src/`'s own | and **137 of `src/`'s 1552 `.c` are never compiled here** — `drivers/armada` 18, `drivers/octeon` 11, `plugins/perfmon` 10, `tools/g2` 10 |

   The last row is what `pick_entries.py --built-only` exists for: a sweep that globs the tree
   reports "chiero cannot read this" for files **nothing** builds.

9. **`:0` bit-fields in `layout`, deliberately left open** (§7.9). ✅ **Priority settled
   2026-08-09 — leave it open, and now on evidence rather than on a 69-header sample.**
   Two measurements, both cheap:

   - **The cost, from `fixed_diff.py`** (which was verified working the same day and is exactly
     this gap's instrument): on its four fixed cases chiero proposes nothing for three, and for
     **two of those gcc's best permutation is 4 bytes smaller** — `Q_two_zero_width_runs` 12 vs
     8, `trailing_zero_width` 16 vs 12. The fourth, `no_zero_width`, gets a correct floor of 8.
     So the gap is real and its size is 4 bytes on the shapes that have it.
   - **The reach: VPP contains no zero-width bit-field at all.** Not "none in 69 headers" —
     `grep -rE '^[[:space:]]*[a-z_0-9 ]+:[[:space:]]*0[[:space:]]*;'` over all of `src/` returns
     **zero** files (measured while explaining why the pinned-40 retake could not move, §7.21).

   **So the only consumer that would benefit cannot reach the construct.**

   ✅ **And the grading path is already built, which was worth checking rather than asserting.**
   The first version of this entry named the generated *layout* gate as the corpus that could
   grade a fix; that was the wrong instrument — it compares sizes and offsets, not proposals.
   The right one is `floor_diff.py`, and measured directly: of 400 generated records **305
   contain a `:0`, and chiero proposes for exactly 0 of them.** They are silently skipped today
   for want of a proposal, and the moment one is emitted all 305 flow into the permutation
   oracle and get graded. **A fix would arrive with 305 checks behind it on the first run** —
   nobody has to build anything first, which is the only part of "worth doing" that was
   genuinely unknown. A record declaring a
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
| `tests/corpus/layout/floor_diff.py` | ✅ committed, and ⚠️ **absent from this table until 2026-08-09** — the second half of §9.2's own lesson: committing an instrument keeps the code, indexing it is what keeps it *used*. It is a **generated** differential for 041 §3's padding floor: random structs, chiero's proposed floor against the true minimum `sizeof` over every permutation of the record's *units* (a bit-field run plus a trailing `:0` moves as one). **Sharpened the same day**: the generator now biases toward records that *can* waste padding (3–7 units, scalars alternating wide/narrow), because an unbiased draw mostly cannot and the instrument was skipping 97% of what it made. Hit rate ~3% → ~18%: `python3 floor_diff.py <seed> 600` gives **98 / 117 / 106 proposals across three seeds, 0 over-claims**, so 041 §3's floor now has **321 checks** behind it rather than 20. The bias is on what is generated, never on what is checked — the oracle is still gcc's minimum `sizeof` over every permutation of the units. ⚠️ Before the fix a 12-case run printed `checked 0 proposals: 0 over-claims, 0 sound`, which is what a low hit rate looks like from outside: clean, and measuring nothing |
| `tests/corpus/vpp-findings/count.py` | ✅ committed — not standalone; `measure.sh` calls it once per entry to turn an envelope into a TSV row. It is where the `ok`/`cut` distinction is defined, and that distinction is the reason the sweep's zeros are readable |
| `tests/corpus/layout/fixed_diff.py` | ✅ committed — chiero's padding floor vs gcc's minimum over every run-preserving permutation |
| `tests/corpus/layout/vpp_sizes.py` | ✅ committed — contract-12's method pointed at arbitrary headers |
| `xtask/src/replay_gate.rs` | ✅ committed — `cargo run -p xtask -- replay-gate`, corpus `tests/corpus/replay/corpus.tsv` |
| `xtask/src/pp_gate.rs` | ✅ committed — `cargo run -p xtask -- pp-gate`, ~2 min. Reads `$SIMPLECPP` (default `/home/ubuntu/simplecpp`, pinned `74a5a63`); gcc and clang are the oracle. §7.11 |
| `tests/corpus/vpp-findings/march_probe.sh` | ✅ committed 2026-08-08 — lowers VPP's 384 `-march=x86-64-v3/v4` units with and without their own `-march`, reporting the definition delta, any diagnostic, and **`EMPTY` for a unit that lowered nothing** (a clean run over six lines of nothing is not a pass). `STRIDE=1` for all 384 |
| `tests/corpus/vpp-findings/api_staleness.py` | ✅ committed 2026-08-08 — which of VPP's 1049 generated API headers are older than the `.api` they come from. Exits 1 on drift; `--fix` regenerates with `vppapigen` rather than `ninja`, whose target re-runs cmake and rewrites the `build.ninja` every VPP measurement reads |
| `tests/corpus/vpp-findings/probe.sh` | ✅ **REBUILT and committed 2026-08-07.** The 7-second five-TU probe that replaces 2-hour sweeps — measured 7.3 s, all five `clean`. `REALCC=true` by default, so it asks what *chiero* makes of the build's flags without compiling. ⚠️ Its rebuild note: the object path **cannot** be constructed from the source path (CMake names an object after its position in the object library, so `src/vlib/main.c` is `…/vlib_objs.dir/main.c.o`) — match `-c <source>` in one `ninja -t commands all` dump, 63 ms for all 2945 |
| `crates/chiero-sema/tests/generated_const_eval.rs` | ✅ committed 2026-08-09 — the fifth standing gate. Generated integer constant expressions graded by `_Static_assert`, so there is no output to parse. `#[ignore]`d, ~2 s. **An honest zero on chiero (300/300)** — its value is that `const_evaluator_reuse.rs` asks gcc *nothing*, and constant folding decides array bounds, bit-field widths, enumerators and case labels |
| `crates/chiero-sema/tests/generated_layout.rs` | ✅ committed 2026-08-09 — the fourth standing gate. Generated record shapes vs gcc, **clang as tiebreak**, `#[ignore]`d, ~1 min. Found the prefix-attribute `layout` defect on its first run (§7.20). Writes no oracle: it reuses `harness::assert_agrees_with_cc` |
| `tests/corpus/replay/replay_probe.sh` | ✅ **REBUILT and committed 2026-08-08**, and to the *newer* method: reverts a fix's `src/` diff onto HEAD rather than checking out two revisions. Refuses a dirty tree, refuses an unknown commit, restores on every exit path including SIGINT; `--check` proves the mechanics with no build. ⚠️ A real run re-runs cmake — see §9.1 item 8 |
| `rev5` (20 fixtures), `replayprobe` (13) | ❌ **LOST.** |

**When the next instrument is built, commit it under `tests/corpus/` in the same wave.** The
committed ones are all still here; not one uncommitted one is. *(Two more went in on 2026-08-08 —
`api_staleness.py` and the rebuilt `replay_probe.sh` — so the only losses left are the `rev5` and
`replayprobe` fixture sets.)* `probe.sh` was rebuilt on
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

## 9.9 ⏰ THE HEARTBEAT — **running, re-armed 2026-08-08**

`mcp__tttt__tttt_cron_create`, **`*/10 * * * *`**, `if_busy=wait`, `session_id=pty-1`, currently
**`cron-6`**. Its standing job is §8.3's widening pattern.

⚠️ **Re-issued 2026-08-07 because the old prompt referred to queue items by number.** §9.1 gets
renumbered whenever an item closes, and the prompt said "item 2 is PARKED" — which, after item 1
closed and a new item 2 appeared, would have told a fresh context to skip live work and would
have said nothing about the actually-parked one. The prompt now **names** the parked item
(`-march` / per-TU target configuration) and tells the reader not to trust a number in it at all.
Any future edit to this cron must keep that property: *a number in a standing instruction is a
reference that rots silently.*

✅ **RE-ARMED 2026-08-08 as `cron-6`; `cron-5` deleted.** The prompt no longer names any item. It
says: *"Parked items are exactly those §9.1 marks with the pause emoji, and nothing else … That
file is edited when the state changes; this prompt is not, so believe the file over this
sentence."*

**The mutable fact now lives in the file that gets edited when it changes**, and the prompt says
only where to look — plus which of the two to trust when they disagree, since they will.

⚠️ **Invariant this creates, and it must be kept:** the pause emoji in §9.1 now means *actively
parked* and nothing else. Three occurrences were cleared when the cron was re-armed — a kept
historical entry, a prose cross-reference, and a note *about* parking — none of them a live park,
all of them things a reader following the prompt would have declined to work on. **A marker that
also means "was once parked" cannot be pointed at by an instruction.** Use 🗄️ for closed entries
kept as record.

⚠️ *The problem it fixed, kept because it is the general form:* The owner unparked `-march` that
day ("go ahead and design + execute the persona work", then "feel free to tackle march"), and it
is **built and closed** — but the cron prompt still says *"The one PARKED item is the `-march` /
per-TU target configuration work: do not start it without checking in with the owner."* A fresh
context reading only the prompt would decline work that is already done.

Fixing the numbers made the prompt survive renumbering; it did not make it survive a *decision*.
**A standing instruction encodes a state of the world and there is no mechanism that updates it.**
Two options, the owner's call: re-arm the cron without that clause, or replace the clause with
"the parked items are whatever §9.1 marks PARKED-emoji" — which moves the mutable fact into the file that
is edited when it changes, and leaves the prompt saying only where to look. The second is
probably right for the same reason the numbers were wrong.

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

### 11.0 Harvested from 2026-08-09, before §9 replaces it

*The session that produced these fixed seven defects, two of them in its own earlier fixes. Each
lesson below is here because it recurred — the instance count is the reason it is a lesson and
not an anecdote.*

- **One fact, more than one reader.** Four instances in one day. `packed` was read by `lay_out`
  and by `record_is_packed` with different position filters, so a rule was half-applied.
  Alignment was read by three arms, and the fix reached one — the unit test went green while
  `chiero cir` emitted the old numbers. A diagnostic was filed as a refusal in four separate
  places. And `cycles_count` had three scratch buffers, two hoisted and one not, which was a
  quadratic nobody could see. **The operational half: when a fix lands, re-run the original
  reproduction, not the new test.** A green unit test beside an unchanged tool is what this
  class looks like from outside.

- **A conclusion is most wrong immediately after you form it**, because the evidence that
  produced it is still the only evidence there is. Three times: a fix declared done that had
  reached one of three readers; a counter declared dead that was working (the grep pattern
  spanned a line `rustfmt` had wrapped); and a band called "essentially linear, 4.1–4.7x" from
  five runs, which run six broke at 5.8x. In each case the sentence just written was the thing
  worth testing, and in each case testing it cost under a minute.

- **A counter that omits a term is not a smaller measurement, it is a wrong one** — and a wrong
  one that agrees with the fix you already made is the worst kind, because it retires the
  question. `CYCLES_CELLS` counted the two scratch buffers that had been hoisted and not the one
  that had not, so it read "4.0x, linear" for two sessions while the clock stayed superlinear.
  Adding the missing term located the defect in a single run.

- **A fix can blind the corpus that found it.** The prefix-attribute fix made every record
  attribute the generator emitted *inert* — chiero ignored them, gcc ignored them, and the gate
  scored agreements about nothing while its reach test went on asserting `packed >= 40`.
  **Counting a construct is not counting a test of it.**

- **A cap does not have to defeat a guard to be silent; it only has to stay inside it.** Adding
  six shapes to a shared corpus cost a channel a quarter of its coverage — 300 checked became
  221 — and its anti-collapse floor, written precisely to notice that, passed comfortably.

- **Committing an instrument keeps the code; indexing it is what keeps it used.** §9.2 already
  said the first half, having lost scripts twice, and was itself missing two committed
  instruments — one of them a generated differential for a claim nothing else checks, which
  therefore nobody had ever run.

- **Before a retake, ask what the corpus would have to contain for the number to move.** The
  pinned-40 came back byte-identical after a session that changed layout, alignment and
  diagnostics — necessarily, since VPP contains zero instances of every shape that changed. That
  run was a control, not a check, and reading it as confirmation would have been the whole error.

- ⚠️ **Mechanical: never key a scripted edit or a grep on prose or on multi-token source text.**
  Four failures in one session, all the same cause — `cargo fmt` had reflowed the anchor. Three
  were silent no-ops; one produced a false finding. `assert old in s` before writing, key on line
  numbers or a bare identifier, and grep for the text you removed rather than reading the test
  result.

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

- 🆕 **A number written mid-investigation is stale by the end of it — cite the measurement beside
  it.** One sweep on 2026-08-08 corrected **four** figures, all written earlier the same day by
  the person correcting them: the `Vec::contains` site count (87 → 79), the growth gate's runtime
  (~25 s → **7 s**, *faster*, because the code it measures got ~250x faster), the corpus gate's
  (~18 → ~20 min), and a "three misses" tally that had reached six. Two pointed at resolved
  causes in a *committed instrument*, which is worse than no pointer: the next reader starts at a
  solved place with the file's own authority behind them.
  **Docs about settled measurements keep; docs about live work rot.** Two cheap defences: write
  `~20 min (1184 s measured)` rather than `~18 min`, and when an investigation closes, re-read
  every number it produced before trusting any of them.
- 🆕 **Counting beats reading — but a counter measures what you chose to count, and a quadratic
  count is not automatically the bottleneck.** `cycles_count`'s scratch really was quadratic
  (327 808 014 cells at n=12800, 16.0x per 4x arcs) and fixing it really did make the counter
  linear — and moved the wall clock by ~7%, because bool writes are a memset. **Pick a unit whose
  cost tracks time** (allocations, hash lookups, solver queries), or the number will be correct
  about the wrong thing. The companion rule to the six hypotheses this project refuted by
  counting: counting stops you being confidently wrong, it does not by itself make you right.
- 🆕 **A null result is scoped to the conditions it was taken under.** One change was reverted
  *twice* on 2026-08-08 for moving no number, both measurements honest and both correct at the
  time — and it landed on the third try, once the two costs hiding it were fixed. So "we tried
  that and it did nothing" is evidence about a state of the code, not a property of the change.
  **After the dominant cost moves, re-test what you reverted.** The general trap is treating a
  measurement as timeless; the cheap defence is an instrument fast enough that re-asking costs
  seconds.
- 🆕 **Recorded blockers rot, and the rate is higher than anyone plans for: three of them were
  false on 2026-08-08 alone.** (a) *"no `compile_commands.json` exists"* — true when written,
  and `ninja -t compdb` had been emitting one on stdout in 90 ms for months; the requirement was
  a *file*, the need was the *data*. (b) *"no `.gcno` artifacts, so no growth curve"* — a `.gcno`
  never had to be found or written, gcc emits one of any size from generated C. (c) *"what is
  actually needed is a profiler"* — every fact in it true, and a counter settled the question in
  one edit.
  **Each cost seconds to disprove and had blocked real work.** So: when picking up an item marked
  ⛔, re-measure the blocker *first* — before designing around it, and before believing the entry
  that says it is impossible. Sweep the whole file's ⛔ markers occasionally rather than waiting
  to trip over one; that is how (c) was caught, an hour after I wrote it.
- 🆕 **A corpus gate finds what the tool *says*; it never finds what the tool silently
  *believes*.** 012 contract 17 preprocesses 1967 VPP TUs and counts diagnostics, and it found
  three real defects that way. It was structurally incapable of finding the fourth and worst:
  the persona defined no `__BYTE_ORDER__`, so `#if __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__` read
  `0 == 0`, took the big-endian branch on x86, and reversed the member order of every bit-field
  struct in `srv6-mobile` — emitting nothing, because a taken branch is not an error.
  **For the silent half, build a differential instrument**: compare chiero's state against the
  real compiler's (here, gcc's 401 predefines ∩ the identifiers VPP tests in `#if`, minus the
  baked set — seconds to run, no build, 8 gaps found). Output-watching and state-comparison are
  two different searches and neither substitutes for the other.
- 🆕 **A cause without an address is not addressable — and the asymmetry hides in instruments
  that report two kinds of thing.** 012 contract 17's corpus run printed an example path for each
  distinct *panic* and only a count for each distinct *diagnostic*. That cost a wave: I reasoned
  about which VPP file produced `redefinition of macro MFD_HUGETLB` from the message alone, and
  **the guess turned out to be right**, which is worse than being wrong, because nothing would
  have corrected it. When an instrument groups results by kind, every kind gets an example.
- 🆕 **Do not guess a spelling that a tool will enumerate.** A first fix defined `__linux__` and
  claimed VPP's `pmalloc.c` no longer reached `#error "Unsupported OS"`. It still did — the guard
  is `#ifdef __linux`. gcc predefines all three spellings of each platform macro and
  `gcc -dM -E -x c /dev/null` prints them in 20 ms. The corpus caught it on the *next* run, which
  is the only reason the wrong claim in the commit message lived for minutes rather than months.
- - **"Did not look" must stay distinct from "found nothing", at every scale.** Selector
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
