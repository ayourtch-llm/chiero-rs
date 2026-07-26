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
| **C frontend** | **Own preprocessor + own parser** | clang/libclang-backed; own-pp + `lang-c` | Full macro-expansion provenance must be first-class. Do NOT retreat to clang when the parser gets hard — provenance is the whole point. |
| **Build order** | **Symbolic core first** | gcov→test-selection first (my rec.); logic-errors first | User overrode my recommendation. Honor it: IR + memory + solver + engine proven on small C before the coverage verticals. |
| **Autonomy** | **Spec gate, then run free** | fully autonomous now; checkpoint every milestone | Specs get user review BEFORE implementation. After approval: no check-ins until the first vertical is green. |

**There is a pending user gate**: specs must be reviewed before any implementation
commit. Do not start the TDD loop until the user approves. Writing/committing the
specs themselves is fine and expected.

## 3. Environment facts (verified, don't re-derive)

- Working dir `/home/ubuntu/rust/chiero-rs`. Git repo exists, branch `master`,
  **no commits yet** at time of writing.
- **VPP already checked out at `/home/ubuntu/vpp`** — HEAD `7fe9c2669396`, 2026-07-17.
  Do not clone from gerrit (the bare URL 404s to curl anyway).
  Scale: 1552 `.c`, 924 `.h`, ~1.01M lines, **754 distinct `foreach_*` X-macros**,
  340 `VLIB_REGISTER_NODE` sites.
- `gcc`/`gcov` **13.3.0** present.
- **clang 18.1.3 + libz3 4.8.12 were being installed by the user at 2026-07-26** (debs
  were in `/var/cache/apt/archives`, `dpkg -l` still empty). **VERIFY before relying on
  them.** Note the z3 debs were `libz3-4`/`libz3-dev` — the *library*, not the `z3` CLI
  binary; the SMT-LIB2 subprocess backend needs `apt install z3` as well.
- **This does NOT change the frontend decision.** Provenance is still why we own the
  preprocessor; do not retreat to clang. What it changes:
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

This is the condensed form of the full design. The specs in `docs/specs/` are the
normative expansion of it. If a spec is missing, write it from this.

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

### 4.13 Testing strategy

**Primary oracle is differential against gcc** (gcc 13.3 is installed, clang is not):
compile+run a C snippet concretely, run the same under chiero with inputs concretized,
compare results. Strong oracle, cheap to run, catches real semantic bugs.
Also: `tests/corpus/` of C snippets with expected results; snapshot/golden tests.
Csmith-style fuzzing noted as later work.

## 5. Non-goals (keep saying no to these)

Soundness-as-a-verifier; full C23 or any complete compiler dialect (target = C11 + the
GNU extensions VPP actually uses); C++; being a compiler (no codegen backend);
replacing the build system; automatic patching; **concurrency/data races** (VPP is
heavily threaded — v1 is single-threaded execution, and this must be declared as a known
blind spot in every report).

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

Planned 22 documents in `docs/specs/`. **Written so far: 8 (frontend block complete).**

- [x] `README.md` — index + reading order + status
- [x] `000-overview.md` — goals, 4 capabilities, design commitments, non-goals, glossary
- [x] `001-architecture.md` — crate graph, dependency rules, CIR-as-contract-boundary
- [x] `010-source-and-provenance.md` — **the crown jewel**; §4.2, worked `vec_add1` example
- [x] `011-lexer.md` — translation phases 1–3, pp-tokens
- [x] `012-preprocessor.md` — phase 4, expansion w/ provenance, `#if` eval, includes
- [x] `013-parser.md` — C11 + GNU extensions, **grounded in measured VPP usage** (below)
- [x] `014-semantics-and-types.md` — types, layout/ABI, name resolution, const-eval
- [ ] `020-cir.md` — §4.4
- [ ] `021-memory-model.md` — §4.5
- [ ] `022-solver.md` — §4.6
- [ ] `023-execution-engine.md` — §4.7, incl. fidelity
- [ ] `024-environment-models.md` — libc/builtins model registry
- [ ] `030-coverage-gcov.md` — §4.9
- [ ] `031-change-impact.md` — §4.8 steps 1–2
- [ ] `032-test-selection.md` — §4.8 steps 3–6
- [ ] `040-defect-checkers.md` — §4.10 checkers + replay
- [ ] `041-optimization-analysis.md` — §4.10 optimization
- [ ] `050-tool-interface.md` — §4.11
- [ ] `060-vpp-integration.md` — §4.12
- [ ] `070-testing-and-tdd-protocol.md` — §4.13 + the red/green/review loop (§8)
- [ ] `080-roadmap.md` — milestones + exit criteria

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

## 9. Next actions

**You are here:** frontend spec block done and committed. 14 specs remain.

1. **Write the remaining 14 specs** in §7 order, from the §4 digest. Next up is the
   symbolic-core block: `020-cir`, `021-memory-model`, `022-solver`,
   `023-execution-engine`, `024-environment-models`. Then the verticals
   (`030`–`041`), then `050`/`060`, then `070`/`080`.
2. Every spec ends with `## Testable contracts` — numbered, checkable. These become the
   RED tests. Style: dense, decisive, concrete Rust sketches, no filler.
3. Refresh context via `mcp__tttt__tttt_clear_and_read_handoff_md` at block boundaries
   (after the core block, after the verticals). **Update §7 checkboxes and commit this
   file first, every time.**
4. When all 22 are written: commit with `spec:` prefix, then **STOP and present the spec
   set to the user for the review gate.** Offer an adversarial subagent review of the
   specs at that point.
5. Only after approval: begin the TDD loop at Milestone 1 (symbolic core — CIR, memory,
   solver-lite, engine, all tested against **hand-written CIR**; frontend comes after).
6. Re-verify clang/z3 availability (§3) before writing `022`'s backend section or `070`'s
   oracle section — the install was in flight.

## 10. Standing reminders

- Don't re-ask the three §2 decisions.
- Don't clone VPP; it's at `/home/ubuntu/vpp`.
- Don't design anything that links clang or z3 at build time.
- Don't start implementing before the user's spec-gate approval.
- Update §7 and §9 of this file before every context refresh, and commit it.
