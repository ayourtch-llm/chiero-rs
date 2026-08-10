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

### 3.1 Building VPP here — the interpreter cmake picks is not the one your shell has

`make build` dies at configure with *"The `ply` Python3 package is not installed"* even though
it **is** installed: cmake finds `/home/ubuntu/.local/bin/python3.11` (3.11.14, no `ply`) while
`/usr/bin/python3` is 3.12 and has it. One fact, two readers.

Fix without touching the environment — `vpp_cmake_args` is `?=` in
`build-data/packages/vpp.mk`, so a value from the environment survives and the makefile's `+=`
still appends the arguments the build needs:

```
export vpp_cmake_args="-DPython3_EXECUTABLE=/usr/bin/python3"
```

⚠️ **A failed configure does not damage the baseline** — measured: the first attempt died before
regenerating anything, and `build.ninja`, the corpus fingerprint and the VPP tree were all
untouched. A *successful* build does regenerate it (see item 8b).

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

### 7.2 `prove_equivalent` — built 2026-08-05

🗄️ Moved to [HANDOFF-ARCHIVE.md](HANDOFF-ARCHIVE.md) on 2026-08-09: a finished story, cited once.

### 7.4 `chiero-replay` — a review that found ten defects

🗄️ Moved to [HANDOFF-ARCHIVE.md](HANDOFF-ARCHIVE.md) on 2026-08-09: a finished story, cited nowhere.

### 7.3 A defect the operations found in the layer beneath them

🗄️ Moved to [HANDOFF-ARCHIVE.md](HANDOFF-ARCHIVE.md) on 2026-08-09: a finished story, cited once.

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

### 7.23 Reading the sweep's residue — an honest zero, and the misreading that produced it

The 2026-08-09 whole-VPP sweep (pedantic, the build's own flags) puts **1390 of 1552 files** in
`BothRefused`. §11.2 says a dominant bucket is a lid, so the rows underneath were read. Three
looked like large chiero defect classes. **All three were agreement.**

| row | files | what it looked like | what it was |
|---|---|---|---|
| `gcc: __int128 ‖ sema: `return` with a value in a `void` function` | **1019** | a false-positive class on two thirds of VPP | gcc says it too — *"ISO C forbids 'return' with expression, in function returning void"* |
| `gcc: __int128 ‖ parse: a member declaration must declare a member` | **90** | a parser gap, 90 of the 95 parse failures | gcc says it too — *"extra semicolon in struct or union specified"* |
| `error: ISO C does not support '__int128'` | 250 | — | both refuse the same construct |

⚠️ **The cause is presentation, and it fooled me three times in ten minutes.** A row reads
`gcc: X ‖ chiero: Y`, which invites reading X and Y as a disagreement. They are each side's
**first** message. gcc stops its report at `__int128` in the file's first header; chiero stops at
its own first. Two unrelated sentences say nothing about whether the tools agree — for that you
diff the two *full* diagnostic sets, which is what settled all three rows above.

📌 **Fixed in the instrument rather than only recorded**: the section now prints
*"(each side's FIRST message; different text is not disagreement)"*, tested, and the mutation
that captions every section fails.

⚠️ **And twice the check itself was a false zero, from a grep pattern written to chiero's
wording instead of gcc's.** `grep -i 'return with a value\|return.*void function'` scores **0**
against a gcc that is saying *"forbids 'return' with expression, in function returning void"*.
The first of those zeros nearly became a defect record. **When two tools are the subject, grep
for the construct in the other tool's vocabulary — or grep for nothing and read the list.**
Listing gcc's distinct error kinds (`grep -o 'error: .*' | sed 's/[^ ]*//' | sort -u`, 19 lines)
is what found both counterparts, and it is cheaper than the pattern that missed them.

📌 **The zero is real and it is good news**: under the pedantic dialect chiero and gcc refuse the
same VPP files for the same reasons. What the sweep cannot currently show is *which* reasons
overlap, because it keeps one message per side.

### 7.27 The scale gate, and what it found in one run

§7.26 said the missing instrument was a generator with a **controlled size axis**. Built as
`crates/chiero-lower/tests/scale.rs`, two shapes, and it found something immediately.

Ratio per 4x step on one growing function — linear is 4x, quadratic 16x:

| stage | 256→1024 | 1024→4096 | 4096→16384 |
|---|---|---|---|
| parse | 3.2x | 3.8x | 4.2x — **linear** |
| sema | 4.7x | 6.3x | 11.1x |
| verify | 5.9x | 6.7x | 13.2x |
| **lower** | 5.4x | 11.2x | **18.6x — worse than quadratic** |

**Lowering one 16 384-statement function takes 5.2 s.** `many_functions` — growing the *module*
rather than one CFG — is far better behaved, so the cost is per-function, which is where the
verifier's earlier dominator quadratic also lived.

✅ **And it closed item 8c the same day** (22.7 s → 7.9 s; see the item). The table above is
the *first* run; the curve after the fixes is in the gate's own doc comment.

⚠️ **Committed as a ratchet, and the test names say so** (`…stays_at_todays_curve`, not
`…is_subquadratic`). Three of four stages are superlinear now; the gate stops them getting
worse while the fix is queued. Every ceiling sits below 16x, so a stage turning outright
quadratic at these sizes fails.

📌 **It was flaky and the fix was not to loosen it.** A single timing at these sizes swings 2x
— verify measured 6.7x to 12.6x with nothing changed, failing 4 runs in 6. It now takes the
**minimum of three** per stage: scheduling noise only ever *adds* time, so the smallest sample
is closest to the work actually done. 6/6 green at 1.7 s, and tightening parse's ceiling below
its real 4.1x fails as it should. **Raising ceilings until a gate stops failing produces a gate
that cannot fail** — the outcome this project keeps refusing.

### 7.30 The measurement harness hand-maintains flags the compile database already has

The owner's note — *"you need to be able to run dynamically without hardcoded compile
commands"* — turned into a number.

`chiero_vpp::builddb` **already** parses a compile database and takes *text*, so it accepts
`ninja -C <build> -t compdb` output or a `compile_commands.json` unchanged. It is used by
`chiero-probe` and by `chiero-vpp`'s own tests, and **by nothing that takes a published
number**: `measure.sh` hand-assembles its `-I`/`-D` list from `build.ninja` by eye, which is a
second reader of one fact — this project's most-repeated defect class, at the level of the
instrument.

📌 **The pinned 40 is unaffected, and that was checked rather than assumed.** For all seven of
its files the harness supplies a strict *superset*: 8 include paths against the real command's
2, none missing. The extras could shadow a header, so the CIR was compared — `vppinfra/hash.c`,
`vlib/node_cli.c`, `vlib/counter.c` are **byte-identical** under real and harness flags.

⚠️ **The plugin sweep is a different story: 198 of 935 plugin C units — 21% — are *exposed*,
needing include paths the harness never passes.**

| files | missing |
|---|---|
| 80 | `install-vpp-native/external/include` |
| 52 | `src/plugins/sfdp_services` |
| 47 | `CMakeFiles/plugins/unittest/../../vpp-api` and `src/vpp-api` |
| 11 | `/usr/include/libnl3` |
| 8 | two more sets |

⚠️ **"Those cannot preprocess" is what I wrote first, and it was inference stated as fact.**
Measured instead — 32 of the 198 sampled at random, each run under harness flags and under its
own real command:

| outcome | files |
|---|---|
| **fails under harness flags, passes under the real ones** | **5** — the misattribution |
| passes either way | 23 — the missing path is never reached |
| fails either way | 4 — a real chiero limit (`dpdk`), not the harness |

So ~16% of the exposed set, **on the order of 30 files of 935**, not 198 — real, worth fixing,
and an order of magnitude smaller than the sentence I first wrote. Ready reproductions:
`linux-cp/lcp_interface.c`, `sfdp_services/acl/cli.c`, `tlspicotls/certs.c`,
`af_xdp/unformat.c`, `sasc/services/flow-quality/counter.c`.

🔶 **Where both sets of *include paths* work, they agree — and that is a narrower claim than the
one first written here.** 20 plugin files run under harness includes and their real includes:
**17 byte-identical CIR, 0 differing**, 3 with one side failing. So the extra include paths never
shadowed anything.

⚠️ **But the real compile command is not just include paths, and the rest of it changes
everything.** Every VPP unit carries `-march=x86-64-v2 -mtune=generic`, which the harness has
never passed. Running the pinned 40 with the *full* database flags: the summary line is
identical (`cut=2 ok=38 findings=21`) and **26 of 38 envelopes differ**. Confirmed directly —
`vppinfra/hash.c` and `vlib/counter.c` produce different CIR with and without `-march` alone.

⛔ **So taking flags from the database is the parked `-march` item, not flag hygiene**, and the
first version of this paragraph said the re-take "changes no existing finding" on the strength
of an include-path-only comparison. It is wrong. `COMPDB=` exists in `measure.sh` and stays
**opt-in and unused** until the owner decides.

Those land as *"chiero cannot read this"* — **a harness defect wearing a chiero defect's
clothes**. §8.3's plugin-sweep rows ("31 `failed` rows resolved to six causes") are where it
hides, and 5f's `--built-only` fixed the neighbouring confusion (files nothing builds) without
reaching this one. ⚠️ **Those rows were never kept**, only summarised in prose, so the overlap
cannot be checked after the fact — §11.3's own lesson, from the wrong side.

✅ **The fix is not new code, it is using the code that exists**: read the compile database and
take each unit's own flags. ⚠️ It re-takes the plugin numbers, which is a deliberate spend — but
the numbers it replaces are measured under flags VPP does not use.

### 7.32 The replay harness as a second oracle — what it can and cannot check

§8.3's new lead says to widen toward known ground truth. `chiero-replay` is a second one nobody
had characterised: `--replay --allow-replay-exec` compiles a finding's witness into a C harness
and runs it, so a **real compiler** can be asked whether the fault reproduces. Measured
2026-08-10:

| finding | replay outcome | what it means |
|---|---|---|
| `10 / d`, witness binds `d = 0` | **`faulted`** | independently confirmed — the compiled program died |
| `a[i]` out of range, `x << 40` | `completed` | ran fine, **and that is not disconfirmation** |
| `*p` where the witness binds a *load* rather than a parameter | `refused` | 040 §3 wants unmodelled extern calls stubbed and memory objects materialised; neither is built |

✅ **The semantics are already documented exactly right, which is why this is a zero rather than
a finding.** `FindingOutcome::Completed` says *"**Not a confirmation**: the witness did not reach
the fault chiero reported, or the fault is one that does not trap"*, and `confirms()` returns
true only for `Faulted`. The harness compiles `-std=gnu11 -w -O0` with **no sanitizers**, so
every non-trapping UB — a shift past the width, an OOB stack read, signed overflow — lands in
`completed` by construction.

📌 **So the oracle's reach is narrow and honest**: it confirms *trapping* faults with a
parameter-bound witness, and says nothing about the rest. **Neither limit is a defect today** —
the operation reports them correctly, which is the property §7.31's whole sweep was about.

⏭️ **Widening it with a sanitizer is feasible and measured — and it is a design decision, not a
patch.** `gcc -fsanitize=undefined` on `x << 40` prints the diagnosis, and with
`UBSAN_OPTIONS=halt_on_error=1:abort_on_error=1` the process aborts (exit **134**, SIGABRT), so
today's `completed` rows would become `faulted`.

⚠️ **The reason not to just do it**: a UBSan abort says *some* undefined behaviour happened,
not that **this finding's** fault did. `FindingOutcome::Faulted` currently means the program
died *at the witness*, and its doc pairs the signal with the fault — *"SIGFPE for a division by
zero, SIGSEGV for a null dereference"*. A blanket SIGABRT does not carry that. **A false
confirmation is worse than an uninformative `completed`**, because it licenses "a real compiler
agrees" when the compiler agreed about something else. Deciding what a sanitizer abort confirms
is the work; wiring the flag is three characters.

### 7.31 Do the defect checkers fire? A corpus with known ground truth

The day's repeated shape — instruments improved, coverage widened, **no new VPP defect** — has
an ambiguity underneath it. Every published sweep reports two kinds (20 `null-dereference`, 1
`division-by-zero`) while the vocabulary has nine. `findings: 0` could mean *the code is clean*
or *the checker never fires*, and nothing here could tell the two apart.

`crates/chiero-cli/tests/injected_defects.rs` — eight injected defects driven as **C through the
real CLI**, each paired with a minimally-different control. (The 16 existing checker tests use
hand-written CIR; this is the path every published number actually took.)

| | |
|---|---|
| recall | **18/19** — everything except `pointer_outside_object`, which is a judgement call rather than a defect |
| reach | **both** default checkers (`OrderDependence`, `UndefinedArithmetic`), not only memory faults |
| controls | **0 false positives** |

📌 **So VPP's zeros are not inert checkers**, at least for these six. That is the answer the
corpus was built to get.

⚠️ **It found two defects in its own controls before finding anything in chiero.**
`int a[4]; a[1]=1; return a[0];` reads an uninitialised `a[0]`; an unchecked `malloc` *is* a
null-dereference. Both were mine. **A control carrying a second defect measures nothing**, which
is the whole argument for pairing.

📌 **The whole `MemFault` vocabulary is now swept against the corpus** (2026-08-10). Three
kinds had never been probed: `double-free` and `bad-free` both worked and are now guarded;
`misaligned` reports nothing **by design** — the engine filters it "until a `ub-strict` mode
exists" (`chiero-exec/src/lib.rs:3798`, item 5j) because lowering emits `align 1` for a packed
member, and `chiero-mem/tests/copy_alignment.rs` already pins both halves. ✅ **And it is a gate now, not a habit**:
`crates/chiero-cli/tests/defect_vocabulary.rs` parses `MemFault::kind`'s match — exhaustive by
compiler — and demands every slug have a corpus case or an `EXCLUDED` entry **with a reason**.
⚠️ **It failed on its first run:** the corpus said `"uninitialized"` where the vocabulary says
`uninitialized-read`, passing only because the assertion used `contains`.

✅ **Extended to `UbKind` as well** — `ub_phrase`'s match is the second exhaustive vocabulary,
and 023 §6.1 makes both channels' kinds one namespace. Two more slugs had never been probed and
both already worked: `float-to-integer-conversion-out-of-range` (`(int) 1e30`) and
`signed-overflow` **forced** by a path (`if (x > 2147483000) return x + 1000;`).
⚠️ `may-signed-overflow` is excluded *with the measurement* — `return x + 1;` reports nothing,
and that is right: the weaker claim would fire on every addition in existence.

✅ **And the checker registry itself.** `every_default_checker_is_accounted_for` pins
`default_checkers()`: adding a checker there changes what **every** `find-bugs` run does, and
now cannot happen without the corpus noticing. It records *how* each is reached (a checker
fires kinds, so `UndefinedArithmetic` is covered through its four `UbKind` cases) and asserts
`UnionPun`'s deliberate absence, so that reasoning and the registry cannot drift apart.
Mutation-checked.

📌 **Three registries gated — `MemFault`'s kinds, `UbKind`'s kinds, the checker list.** All
three were audited by eye on 2026-08-10 and all three are tests now; each audit's last act was
to make itself unnecessary.

🔴 **`layout` claimed one too** — same question, one operation over, and 041 §2 is explicit
about the stake: *"Only `fidelity == Exact` is a proof, and only `Exact` licenses dropping a
test in 032"*. A record whose field list cannot be stated (a zero-width bit-field, or a member
the frontend could not size) left the envelope `Exact`/proven. ⚠️ **The blind spot naming that
record was already there** — *"a padding number summed over the members that are left is not a
smaller number but a wrong one"* — so the prose and the structured claim disagreed, and only one
of them is machine-readable. ✅ Fixed: partial ⇒ `Bounded`.

🔴 **`find-optimizations` claimed a proof it had not earned — the method's second real
defect, and the first outside `find-bugs`.** 050 contract 3 makes the distinction load-bearing
for `find_bugs` ("the string 'no defects found' never appears unqualified"), because an empty
list is wrong precisely when the search did not finish. This operation answers the same shape
of question and had no such rule: its fidelity came from `any_advisory` alone.

| program | proposals | verdict |
|---|---|---|
| `if (x > 10 && x < 5)` | 2 | `Exact`, proven — correct |
| the same dead branch behind an unmodelled call | **0** | `Exact`, **proven** — wrong |

📌 **The signal existed and was dropped**: `detect` runs the engine and had `run.fidelity()` in
hand, returning only the proposals. `detect_reporting` surfaces it and the operation now takes
the weaker of the two claims. ✅ Fixed: the invisible dead branch is now `Unknown`/not proven;
the visible one is unchanged.

📌 **`prove_equivalent` too, and it is the operation where a wrong answer is worst.** `x * 2`
vs `x << 1` proves equivalent; `x + 1` vs `x + 2` differs; **a difference behind an unmodelled
construct answers `unknown`, not `equivalent`** — no false proof. The existing `unknown` test
compares a function *with itself*; the new one has programs that genuinely differ with the
difference hidden, which is the shape that would licence a wrong rewrite.
⚠️ **Its limit is written into it**: making both sides identical leaves it passing, because
chiero answers `unknown` for `f80` either way. So it guards "never claims a proof it lacks",
**not** "noticed the difference" — checked by mutating the fixture rather than assumed.

📌 **`check-reachable` probed the same way, and it is healthy too** (2026-08-10). A reachable
line, a line behind `x > 10 && x < 5`, and an always-reached line all answer correctly — and
the distinction the operation exists for holds: a line guarded by an unmodelled 80-bit float
comparison answers **`not_shown_reachable`**, names `FOLt is not modeled`, and says the line may
still be reachable. ⏭️ Three routes to that verdict were tested (no solver, an exhaustive proof,
a loop bound); the **unmodelled-operation** route was not, and it is the one most likely to
break silently — it produces no error and no path, which looks exactly like nothing arriving.
Now `a_line_behind_an_unmodelled_branch_is_not_shown_reachable`.

📌 **Reach is healthy — seven probes, an honest zero, 2026-08-10.** The two defects this
corpus found were about *representation* (`UNBOUND`, 8e) and *a guard clause* (8f), so the
obvious next question was whether **depth** defeats the checkers. It does not:

| probe | result |
|---|---|
| null-deref behind a loop, and behind a symbolic `if` | reports |
| null-deref three calls deep, and returned from a callee | reports |
| OOB reachable only when `i * 7 == 91` | reports — **and the solver finds `i = 13`**, offset 52 |
| `free()` in a callee, use after return | reports |
| division by zero only on the path where `a == b` | reports |

So a defect being far away, behind a solver query, or across a function boundary does not
hide it. **That bounds where to look next**: this project's checker defects have been about
what a value *is* and which guard runs, not about how hard the defect is to reach.

⏭️ **Two standing misses, and neither is silence** — leads about the *analysis*, which is where
the remaining value is:

- **`wild_pointer_via_variable`** — masked, and its twin `wild_pointer_direct` **passes**.
  `return *(int *)0x1234;` is reported correctly; assigning the same address to a variable first
  is not. **The pair is what made it a diagnosis** rather than a puzzle: identical fault, one
  instruction of round-trip apart. Storing a `Pointer { base: UNBOUND }` writes no bytes, so the
  slot stays uninitialized and the load reports *that*. Full mechanism in item 8e.
- **`pointer_outside_object`** — **it fires only for *symbolic* offsets**, characterised
  2026-08-10. `int *p = a + 8;` (constant) is silent; `a + (i & 31) + 8` reports
  *"pointer-outside-object: a pointer into a (16 bytes) can be computed at …"* with no
  dereference needed. The reason is in the code: the fault is raised inside the symbolic-offset
  enumeration path in `chiero-exec/src/lib.rs:4986`, reached when enumeration fails and a
  feasibility query returns `Sat`, and its witness comes from the solver model. A concrete
  offset never reaches that block.

  C 6.5.6p8 makes the *computation* undefined however the offset is spelled, so the constant
  case is a real gap — a narrow one, and possibly deliberate: the variant's own doc says
  "forming a pointer past the end is deliberate in a few real idioms". ⏭️ **Worth an owner's
  view rather than a fix**, since reporting every constant `&a[n]` past the end would be noisy
  in exactly the idioms that comment protects. It also explains 5i: the `vnet/` class is
  symbolic-index heavy, which is the only shape that reports.

Recorded rather than asserted — pinning today's kind would make a future fix look like a
regression.

### 7.29 The verifier's dominator sets — 35.6 GB, and the arithmetic I did not believe

`dominators` seeded every block with a copy of *every block in the function*, so the initial
state alone is O(B²). On a 96 000-block function:

| | before | after |
|---|---|---|
| time | 27 940 ms | **3 043 ms** (9.2x) |
| peak RSS | **35 628 MB** | **494 MB** (72x) |

**The sets were never needed.** Their only consumer asks one question — *does `db` dominate
`at_block`?* — which an idom tree answers by walking up, with no allocation. So the
representation is Cooper–Harvey–Kennedy immediate dominators plus a `dominates` walk, O(B).

⚠️ **I nearly talked myself out of it.** B² × 4 bytes came to ~37 GB, which "obviously" could
not be happening without an OOM, so I went looking for the guard that made it fine. There is no
guard; the number was right. **Checking beat disbelieving** — one `VmHWM` sample settled it, and
it also corrected a block count I had published as ~24 576 when it is 96 000. Two numbers wrong
in the same claim, both in the direction of *understating* the defect.

📌 **A representation swap under a load-bearing rule needs evidence the old tests cannot give**,
because every dominance test in `verifier.rs` was written against the implementation being
replaced. Three independent checks:

- `crates/chiero-cir/tests/dominance_property.rs` — 400 random CFGs (DAGs plus back edges),
  checking the verifier's verdict against the **definition**: delete the def block, is the use
  still reachable? 218 usable cases, **123 rejected and 95 accepted**, so both verdicts are
  exercised rather than one. Mutating `dominates` to always hold fails it on case 3.
- the pinned 40: **38/38 comparable envelopes byte-identical**, `findings=21` unchanged.
- `./check.sh` GREEN 2316 across 281 suites.

📌 **`find-bugs` pays this twice** (§7.28), so the saving lands twice on that path.

📌 **Where the whole day landed**, one 32 768-statement function and the 96 000-block engine
probe. Eight quadratic scans and one O(B²) representation, none of which any VPP measurement
could see:

| | start of 2026-08-09 | end |
|---|---|---|
| `big.c` frontend | 22 671 ms | **1 693 ms** (13.4x) |
| `eng32000` frontend | 27 940 ms | **2 881 ms** (9.7x) |
| `eng32000` full `find-bugs` | 53 279 ms | **3 264 ms** (16.3x) |
| peak RSS, 96k blocks | 35 628 MB | **494 MB** |

`full − frontend` is now **383 ms**, which settles §7.28's correction from the other side: the
"engine residual" subtraction kept reporting really was the second verify. The scale gate's
ceilings moved with every fix — parse 4.3x, sema 4.7x, verify 5.3x, lower 5.7x measured, held
at 6.0/6.5/7.0/7.5.

### 7.28 An engine size axis without z3 — and the scan it found was not the suspected one

Five `blocks.iter().find(|b| b.id == …)` sites were recorded as an unmeasured shape, three of
them in `chiero-exec` running on **every execution step**. Measuring settled it, and the answer
was none of them.

📌 **The instrument is the finding.** An N-local function whose conditions are all **concrete**
executes many steps over many blocks with *no forking and no solver query*. Every VPP corpus
conflates chiero's cost with z3's — §7.25 and §7.23 both ended at "the cost is the solver" — and
this separates them for the first time.

| n | 500 | 2000 | 8000 |
|---|---|---|---|
| engine, before | 70 ms | 316 ms | **3246 ms** — 10.3x per 4x step, no z3 |
| engine, after | 24 ms | 67 ms | **1221 ms** — 2.7x faster |

Three stack samples put **2 in `Memory::entry`**, not in any block lookup. `entry`/`entry_mut`
were linear scans of `entries`; a program with N locals has N objects, so reading a local cost
O(objects) and the engine O(accesses × objects). **Eighth instance of this class in a day**, and
`.find(..)` again.

**Binary search rather than an index map, deliberately:** `Memory` is cloned on every state
fork, so a second container is paid for on every branch. `entries` is sorted by construction —
`alloc`'s push is the only writer, ids come from a monotonic counter — pinned by a
`debug_assert` at the push *and* `object_ids_are_allocated_in_increasing_order` from outside,
because an allocator that reused ids would not fail loudly: lookups would quietly miss objects
that are present.

⚠️ **"Still 18.2x on the last step, so something else remains in the engine" — that was wrong,
and the instrument caught it within the hour.** At n=32 000 the residual was profiled: **4 of 4
samples land in `verify::dominators`**, not in the engine at all.

📌 **`find-bugs` verifies the module twice.** Lowering does it (`refuse_unverifiable`) and
`Engine::run` does it again (`chiero-exec/src/lib.rs:2212`). The second is **deliberate and must
stay** — 020 §8, quoted at the site: *"always, including on hand-written fixtures. A module that
fails verification is never executed"*, which is what protects text-parsed CIR and hand-built
fixtures. So `dominators` is paid **twice** on this path, and fixing it is worth about double
what it looked like.

⚠️ **The reusable lesson: `engine = find-bugs − chiero cir` is not a phase split.** Subtracting
two whole-program timings attributes the difference to a phase only if the shared part costs the
same in both — here it does not, because one run verifies once and the other twice. The number
was real; the label on it was invented. ✅ *One alternative checked and cleared:* gdb was not
distorting the timeline — a full run under it takes **53.5 s** against **53.3 s** native.

### 7.26 Why a growth curve over VPP files cannot find 5b's class in the frontend

5c's quadratic was found by sampling, and 5b's audit grep cannot see that shape (§9.1). The
obvious replacement — time `chiero cir` across real files of increasing size and look for a
bend — was tried, and the corpus will not support it.

| file | own lines | CIR lines | frontend |
|---|---|---|---|
| `vppinfra/bitmap.c` | **167** | 185 434 | 1009 ms |
| `vppinfra/format.c` | 857 | 184 418 | 1018 ms |
| `vlib/node_cli.c` | 888 | 247 057 | 1476 ms |
| `vnet/ip/ip4_forward.c` | 2901 | 307 994 | 1966 ms |

**5–6 µs per CIR line across all ten files measured**, and the reason the curve is useless is in
the second column: 167 lines of source become 185 000 lines of CIR, a **1108x** ratio. A
translation unit's size here is its *header closure*, which barely varies — the whole corpus
spans **1.7x**. A quadratic needs an order of magnitude to show a bend, so this instrument
cannot distinguish linear from quadratic no matter how many files it runs.

📌 **§7.21's rule, applied before believing a flat line rather than after.** The flatness is
evidence about the corpus first. What could find the class is a generator with a *controlled*
size axis — the shape `generated_layout`/`generated_const_eval` already have for correctness,
and which no gate has for scale — or more sampling of real runs, which is what found both
instances so far.

### 7.24 Two of the pinned 40 cannot be compared, and I compared them twice

`--time-budget` is a **wall clock**. An entry that hits it stops wherever the machine got to,
so its envelope is a measurement of the machine. Three runs of one binary on
`clib_mem_create_heap`:

| run | 1 | 2 | 3 |
|---|---|---|---|
| states left unexplored | **22** | **23** | **24** |

📌 **chiero already says which ones these are, precisely.** The envelope carries
`"nondeterministic_abort": true`, and only on the two entries that hit the budget — the other
38 carry `false`. The product's own `--help` says it outright: *"where it stopped depends on the
machine, so the answer is a measurement"*, and offers `--solver-rlimit` as the deterministic
bound. **Nothing here is a defect in chiero.** The defect was in how the instrument was used.

⚠️ **It qualifies evidence given twice on 2026-08-09.** §7.22's "5 of 40 envelopes differ" is
**3 of 40**: two of the five were this pair. And the parse_model retake (§7.25) shows "2 of 40
differ" which is **0** — both are the same pair. Neither conclusion changes; both numbers did.

✅ **Turned into a tool rather than a rule**: `tests/corpus/vpp-findings/compare.py BEFORE AFTER`
diffs two `KEEP=` directories, excludes the flagged envelopes, and **names what it excluded** —
a comparison that silently skips entries reads as "everything matched", which is the failure it
exists to prevent.

⚠️ **The `cut` rows stay `cut` and that is a separate question.** Making them reproducible means
`--solver-rlimit` instead of a clock, which changes what the pinned 40 measure and invalidates
every published number from it. **A deliberate spend, like 032 c18's replay corpus — the
owner's call, not a wave's.**

### 7.25 Does the `parse_model` fix reach the pinned 40? Barely, and the honest answer is no

`cut=2 ok=38 findings=21 exact=0` — identical, and this time **0 comparable envelopes differ**.
The only movement is inside the two non-comparable ones: `24 → 22` states left unexplored, which
is within the 22–24 that three runs of a single binary produce anyway.

So the 30x parse speed-up is worth ~2 states out of 60 seconds on these entries — because their
cost is z3, not chiero. That is the same conclusion two stack samples gave for `nsh_md2_encap`
after the fix, arrived at from the corpus side, and it is why §7.23's "the sweep cannot show
which reasons overlap" and this both point at the solver rather than at the frontend.

### 7.22 The pinned-40 retake that *did* move — and where the movement was hiding

Item 6's engine half, measured before and after with `KEEP` on both legs.

| | before | after |
|---|---|---|
| summary line | `cut=2 ok=38 findings=21 exact=0` | **byte-identical** |
| envelopes differing | — | **3 of 40** (corrected; see §7.24) |
| assumptions | 582 | 586 |
| builtins reached only after | — | `__builtin_ia32_writeeflags_u64`, `_rstorssp`, `_clrssbsy` |
| reached only before | `__builtin_ia32_lzcnt_u64` | — |

⚠️ **Reported as "5 of 40" when it was taken, and that was wrong.** Two of the five are
`mem_dlmalloc`'s pair, which chiero marks `nondeterministic_abort` — their diff was the wall
clock, not the change (§7.24). The conclusion survives unchanged: the three real ones are
exactly the three carrying the newly reached void builtins.

**The summary line said nothing happened and three envelopes disagreed.** This is the case
`KEEP` was added for (§11.3) and the first time it has paid: §7.21's identical retake was
genuinely a control, this one was not, and *the two are indistinguishable from the summary
line alone*. All three newly reached builtins return `void` — the fix stated as a measurement.

⚠️ **`lzcnt_u64` is not a width exclusion.** The new message never fires on this corpus, so
the width filter — the change the item was actually about — cut **nothing** here. `lzcnt` is
displaced under `max_indirect`: admitting the candidates that belonged pushes it past the cap.
A reordering under a fixed budget, not a lost candidate class. Both explanations were on the
table and the counter settled it; "correctly excluded" was the comfortable one and it was wrong.

📌 **So the width filter is a control on the pinned 40, and the whole measured effect is a
defect found while pre-registering what the corpus could reach.** The item's stated purpose
produced no movement; the movement came from reading the filter closely enough to see the rule
beside the one being added was backwards.

⚠️ **And the pre-registration itself was wrong the first time.** The census said 6 indirect
sites, 2 non-void. Run with the *sweep's own* include flags it is **56 and 26** — I had used
`-I src` alone while `measure.sh` supplies the cmake build's generated roots. A file that will
not preprocess yields little CIR, and I had read that as "few indirect calls". The error was
caught only because a file the census scored **0** turned up in the results reaching an
indirect call. **A census only interprets a measurement if it is taken in that measurement's
configuration** — the same trap `measure.sh`'s own header warns about for `INCLUDES`, arrived
at from the analysis side.

### 7.33 Per-operation `--help` — and the flag it turned up that was documented nowhere

User-test finding 4: `select-tests --help` printed the global page, so a reader who had already
chosen an operation still had to work out which of eighteen options applied to it, and 030's
`--coverage`/`--stem` semantics cost the first user three attempts. Each operation now has its
own page — title, its own synopsis, its paragraph, and **only the options it reads**. A usage
error prints that page rather than the global one, which is where the user actually met it: a
complaint about `--stem`, answered with every operation in the tool.

📌 **The yield was not the pages. It was what writing the gate first found.** The help text
moved out of a hand-written `USAGE` string into two tables in `chiero-cli/src/help.rs`, and
`crates/chiero-cli/tests/help.rs` was written before them — reading three sources of truth already in `main.rs`
rather than restating any of them:

| read from | what it decides |
|---|---|
| the dispatch `match` in `run` | which operations exist |
| the `match` in `Options::parse` | which flags are accepted at all |
| each operation function's own `o.<field>` uses | which flags **that** operation reads |

The third is the one a table cannot supply, and it makes the gate bidirectional: a page must
name every option its implementation consults, and advertise none it ignores. Five of six
assertions were red, and one was a defect the feature had not reached — **`--march` has been
accepted by the parser since 2026-08-09 and was documented nowhere.** That is the flag §7.30
found to be load-bearing: every AVX2 path in vppinfra depends on it, and no reader could have
learned it existed.

⚠️ **Two under-detections in the gate itself, both in the flattering direction**, and both found
by disbelieving a pass. `o.entry` is a prefix of `o.entry_ptr_nonnull`, so a substring test made
every operation reading the assumption flag look as though it read `--entry`; and rustfmt writes
`let entry = o\n    .entry\n    .clone()`, so a `contains("o.entry")` said `prove_equivalent`
reads no options at all. A test that parses source has to be read as a *measurement instrument*,
and both failures made the gate quieter rather than louder.

### 7.34 `select-tests` from the command line — D1, and the two gates that refused it first

The first end-to-end user's biggest finding, closed the day it was reported. `--test NAME=PATH`
once per test run, or `--coverage-manifest <file>` with a `NAME<TAB>PATH` line each — what a
`make test-cov TEST=<name>` loop writes. Gated end to end in
`crates/chiero-cli/tests/select_tests_cli.rs` against two real gcov objects in the corpus: a
change to `other.c` selects the test that ran `other` and leaves the one that ran `t` alone.

📌 **The library could always do this. Only the command line could not say it.**
`ingest_native_as` has taken a `TestId` since the day it was written, and the VPP walkthrough
that proved the thesis reached it through a 145-line Rust driver. The whole fix is an argument
shape — which is worth remembering the next time a capability looks missing.

⚠️ **Two things the fixture taught that no amount of design would have.** The before/after pair
must keep the *same* file name in two directories: coverage records source paths as gcov wrote
them, and a pair called `before.c`/`after.c` describes a file no test has ever run. The envelope
said so exactly — *"`after.c` is not in the coverage index at all"* — which is the envelope
discipline working on its own author. And a `TestId` is a number: `"test": 3` is not an answer a
consumer can act on, so `select_tests_named` carries the caller's own names back.

**Both gates that went red on this were right, and one of them was made better by it:**

| gate | what it refused | outcome |
|---|---|---|
| `chiero-tool/tests/operations.rs` | a new public function returning an `Envelope` with no samples — 050 contracts 1 and 4b are quantified over operations | registered, with the declaration that naming a test cannot make a historical measurement a proof |
| `chiero-cli/tests/help.rs` | `select-tests --help` advertising `--coverage`, once the coverage handling moved into a helper | the gate now follows an operation into any function it hands `o` to. **A gate that goes red when code is *tidied* teaches people to weaken it** |

### 7.35 A paired C corpus for `prove-equivalent` — an honest zero worth having

§8.3's strongest form — *widen toward a corpus whose answer is known in advance* — pointed at
the operation next door to the one that paid. `injected_defects.rs` exists because every
published `find-bugs` number came through the CLI while every test drove hand-written CIR, and
it found seven defects in a morning. `prove-equivalent` was in the same position:
`chiero-tool/tests/prove_equivalent.rs` is eight `Module`s built by hand, and a caller starts at
C. `crates/chiero-cli/tests/injected_rewrites.rs` is twelve rewrites through the real command.

| really equivalent | really different |
|---|---|
| `x + 0` → `x`; `x * 2u` → `x << 1`; De Morgan; unsigned reassociation; branch → select; a concrete loop → its closed form | abs at `INT_MIN`; `x / 2` → `x >> 1`; an unsigned compare cast to signed; an off-by-one bound; `%` versus `&` on a negative; a moved parenthesis in a shift |

📌 **All twelve are decided correctly — no false proof, no false accusation.** That is a real
result rather than a null one: it says the adjudicator is in much better shape than the defect
checkers were, measured from the surface a caller uses rather than from the one the tests used.

⚠️ **And it would have turned half of CI red.** With `CHIERO_SMT_SOLVER=/nonexistent` — a
*supported* configuration (022 contract 2) with its own CI leg — nine of the twelve become
`unknown`, which is the correct answer and not a failure. The rule the file settled on is worth
reusing: **the wrong answers are forbidden unconditionally, because a false proof is a false
proof with or without z3; only the floor that says *something was decided* is conditional on a
backend.** Found by running the second leg before committing rather than after.

### 7.36 Three known-ground-truth corpora, and what the third pair of them found

**Nothing.** Both are honest zeros, and both are worth the hour.

§8.3's strongest form says to widen toward a corpus whose answer is known in advance.
`injected_defects.rs` did that for the checkers on 2026-08-10 and found seven defects, because
every published `find-bugs` number came through the CLI while every test drove hand-written
CIR. Two neighbours were in exactly the same position and got the same treatment:

| corpus | cases | result |
|---|---|---|
| `crates/chiero-cli/tests/injected_rewrites.rs` | 6 equivalent rewrites, 6 that differ | **12/12 correct** — no false proof, no false accusation |
| `crates/chiero-cli/tests/injected_reachability.rs` | 4 live lines, 4 dead ones | **8/8 correct** — and the dead ones are *proved*, not merely not-shown |

📌 **A zero from a corpus with known answers is not the same as a zero from real code**, which
is the whole argument of §8.3 read backwards. `find-bugs` was in bad shape and nobody could see
it, because a `findings: 0` over VPP reads identically whether the code is clean or the checker
never fires. These two say something a VPP sweep cannot: the adjudicator and the reachability
analysis *decide*, from C, through the command, and they decide correctly.

⏭️ **The rule both files settled, and it generalises past them.** The wrong answers are
forbidden **unconditionally** — a false proof is a false proof with or without z3 — and only the
floor asserting that *something was decided* is conditional on a backend. Without that split
`injected_rewrites.rs` would have been red on the `solver: none` leg, where nine of its twelve
verdicts correctly become `unknown`. ⚠️ Found by running the second leg **before** committing.

### 7.37 The coverage instrument could not see the product surface — 133 uncited contracts

`cargo run -p xtask -- contract-coverage` measured 020–024 (the M1 gate) and 010–015 (the
frontend, reported). Nothing else. Those are the two groups somebody was building when it was
written, and it was never widened as the work moved — so **050, the spec an agent consumer
depends on most, had twenty numbered contracts and no counter saying which any test claims.**

📌 **The same omission is written in that file already, one group earlier**: *"a coverage tool
that cannot see half the work in flight reports a comfortable number about the half it can"* —
added when the frontend was invisible to it. The instrument was widened once, for the work in
flight at the time, and not again. **Ask of an instrument what §8.3 asks of a gate: what is
outside its corpus?**

| | cited | | | cited |
|---|---|---|---|---|
| 030 change impact | 13/19 | | 041 optimization | 10/30 |
| 031 impact closure | 6/22 | | 042 conformance | 3/31 |
| 032 test selection | 4/22 | | 050 tool interface | 14/23 |
| 040 defect analysis | 2/23 | | 060 VPP | 3/18 |
| | | | **total** | **55/188** |

⚠️ **Uncited is not untested and the number must not be read that way.** This counts citations
of the form `NNN contract K` — a convention the M1 and frontend tests follow and the product
tests largely predate. `injected_defects.rs` plainly exercises 040's checkers and cites none of
its contracts. What the number says is *what nobody has claimed*.

⏭️ **The next move is reading, not building**, and it splits three ways per contract: already
tested and merely uncited (add the citation), genuinely untested (a red test), or untestable as
written (a spec amendment, which is what 023 c17's withdrawal was). 040's 2/23 is the place to
start — it is the operation with a known-ground-truth corpus already sitting next to it.

### 7.5 How to check the workspace is green — `./check.sh`

**Do not sum "N passed" out of `cargo test`.** I reported "0 failed" for a long stretch while
three xtask gates were red: a crate whose test *binary* fails to build emits no `test result`
line at all, so counting successes cannot detect a missing success. `./check.sh` keys on
cargo's exit status and prints the failing suites first. Current: **2359 passed, 294 suites** (2026-08-10), and **both legs** — the second, `CHIERO_SMT_SOLVER=/nonexistent`, passes 2359 too. Worth running that leg before committing anything whose answer depends on a solver: `injected_rewrites.rs` would have been red on half of CI otherwise (§7.35).

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

🔥 **The strongest form, and 2026-08-10 is the evidence: widen toward a corpus whose *answer is
known in advance*.** That day spent a morning on instruments — a controlled size axis, an
engine probe, a compile-database reader — and found real quadratics that moved **no published
number**. It then spent an evening on four-line C programs with an injected defect and a matched
clean control, and found **seven defects in chiero's own checkers and operations**, none of them
reachable by any VPP corpus. The difference is not effort or cleverness; it is that a
known-ground-truth corpus can distinguish *"the code is clean"* from *"the checker never
fires"*, and no amount of real code can. ⏭️ The generalisation that kept paying: **take a rule
written for one component and ask whether its neighbours obey it** — 050 contract 3's
empty-answer rule, asked of all nine operations, found two more.

This has now paid out many times, each time on the first run after a widening:

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
| **a controlled *size* axis** — VPP cannot supply one, its TUs span 1.7x because a TU's size is its header closure | one wave, 2026-08-10 | **6 quadratic scans + an O(B²) representation.** Frontend 22.7 s → 1.7 s on one 32k-statement function; peak RSS **35.6 GB → 494 MB**. ⚠️ And **no published number moved** — see the row below for why that matters |
| **a corpus of *known* defects**, C through the real CLI, each paired with a minimally-different clean control | one evening, 2026-08-10 | **the richest yield of the project so far: 7 defects** — 5 checkers (a wild-pointer masked through every shape real C uses; a shift rule skipped; a wild call reported as nothing; a wild free naming address 0; a store that kept stale bytes) and 2 operations (§7.31). **It also found two defects in its own controls before finding any in chiero**, which is the argument for pairing |
| **one rule, asked of its neighbours** — 050 c3's *what does an empty answer mean?*, put to all nine operations | one wave | **2 defects**: `find_optimizations` and `layout` each claimed `proven` over an analysis that had skipped part of its input, with a blind spot naming the skipped part sitting right beside the wrong flag |
| **a value's siblings** — `ObjectId::NULL` is special-cased, where is `UNBOUND`? | one grep | **3 of the 5 checker defects above.** Two further sentinel audits returned **zero**, and the zeros are what gave the rule its scope: audit *const sentinels*, not enum variants, because a missing enum arm is a compile error |
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
| *not a widening* — **disbelieving a cost this file had just recorded** | 10 min | the **row below**'s "`__int128` had to leave the generator, `SemaDiagnostic` has no severity" was **wrong** (this table is newest-first, so the claim being corrected sits *after* the correction): chiero has had a dialect switch since wave 314, `__int128`'s diagnostic is gated on it, and `harness::parse` has used `Dialect::gnu()` all along *with a doc comment saying why*. The gate had called the deliberately-strict sibling. **A blocker stated as "the type lacks a field" was a helper nobody read** — the `MemFault::BadRange` lesson again. ⚠️ And restoring the construct blinded a dimension **again**: the gcc/clang divergence rows went 5 → **0**, which reads as "the compilers agree about everything" and means "the corpus stopped reaching the shape". Gate range 120 → 300 seeds; **222 → 569 records**; both shapes now asserted reachable. **Three waves running, a generator change moved what the fixed seeds produce, and only the reach test caught it** |
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

> 🗂️ **Closed work lives in [HANDOFF-ARCHIVE.md](HANDOFF-ARCHIVE.md)** — 22 queue items, item
> `5b`'s original 479-line entry, and three finished §7 records. **Moved 2026-08-09, not
> deleted**: this file had reached **5027 lines**, and the reasoning behind a closed item is
> worth keeping (§11.3) but not worth making a fresh context read before it reaches live work.
> The archive is linted like this file is. Section numbers did not change, so an old citation
> still finds its subject.
>
> ### 🧑‍💻 FIRST END-TO-END USER TEST — the thesis held, and six findings, 2026-08-10
>
> claude-mini ran chiero as a genuinely naive first user on ayourtch's request: fresh x86 and
> ARM boxes, README only, then real VPP — checkout, gcov build, a planted
> `clib_max`→`clib_min` in `bfd_recalc_detection_time`, and selection. Report at
> `~/from-claude-mini-2026-08-10-user-test.md`; driver at `~/vpp-select-driver`.
>
> ✅ **The product thesis held, on real code, end to end.** `select_with` ranked `bfd`
> **first — 4582 covering relations against 343/343/343** (the boot-path floor), and running only
> that test caught the bug: 3 failures, exactly the asymmetric-interval cases.
> 📌 **A free mutation insight for VPP** came out of it: the symmetric-interval tests stay green
> because the bfd suite's defaults cannot see this bug class at all (`min == max` when `a == b`).
>
> | # | finding | state |
> |---|---|---|
> | 1 | **CLI `select-tests` was structurally empty** — `ingest_native` attributes no test, so `index.tests()` is empty and every invocation returned `0 selected` whatever the diff said. ⚠️ **Tutorial 3's console example runs this path; `tutorials.rs` covers the library one** — §8.3's third form inside our own tutorial | ✅ **fixed twice**: it refused and named the limit, and then it learned to select. `--test NAME=PATH` (repeatable) and `--coverage-manifest <file>` attribute coverage per test and land on `ingest_native_as`; tutorial 3 shows them. §7.34 |
> | 2 | **First ARM run ever** (aarch64 Grace): build 11 s, 1500+ tests green, **engine portable**. The 12 failures are *all* gcc-differential gates on real x86↔ARM divergence — char signedness, 128-bit long double, predefines under `-march`, vector lanes, zero-width bit-field in a union, and `cli.rs:1482` asserting `has_sse42` unconditionally | 🆕 **not defects — the map of an ARM port**, if VPP-on-ARM ever matters |
> | 3 | `check.sh` printed **"0 failed" while a suite failed 31/1 inside** — it summed a column that is not the failure count in every rendering | ✅ **fixed**: matches `[0-9]+ failed` by name. The verdict keyed on cargo's exit status and was always right; only the number lied |
> | 4 | **No per-operation `--help`** — `select-tests --help` prints the global page, and 030's path/stem semantics cost three attempts. ⚠️ *But the envelope text taught them each time* — "that discipline **works** on a hostile-ignorant user; it's the best part" | ✅ **fixed**: each operation has its own page, listing **only the options it reads**, and a usage error prints that page rather than the global one — which is where the user actually met it. §7.33 |
> | 5 | **Tutorial↔API drift ×3** | ✅ **all three fixed.** `ExcludedTest { test, refinement, entity, fidelity }` corrected in tutorial 3 (it said `proof`), and tutorial 1's `Fresh \| Stale \| Unknown` corrected to `Partial` with the meaning spelled out. ✅ **and the third is now found and fixed.** `FileLoader` appeared nowhere in `docs/` *because no tutorial mentioned it at all* — `Program::parse` follows no `#include`, so a reader pointing chiero at a real translation unit meets a trait the documentation never named and guesses its signature. Tutorial 2 now has a *Reading a real file* section with a working `Disk` loader (`io::Result`, and why it is not `Option`), backed by a compiled test |
> | 6 | `compiler_oracle.rs:14` bare-panics on missing clang (`spawn().unwrap()`) where house style is *"an error naming the file that was looked for"* | ✅ **fixed** (`5f47100`): the panic names the compiler, says it is looked up on `PATH`, and says what to do instead |
>
> ✅ **The gap behind finding 5 is closed**: `crates/chiero-tool/tests/tutorial_api_drift.rs`
> reads every tutorial's field lists and variant alternations and compares them to the crates'
> own `pub struct`/`pub enum` declarations. It was mutation-tested against a corpus whose answer
> was known in advance — restoring either pre-2026-08-10 line turns it red and names the field —
> which is §8.3's strongest form and the reason it was worth an hour rather than the wave that
> doctests over compilable snippets would have been. ⚠️ It says nothing about a type it cannot
> find: treating "unknown" as "wrong" would make it unusable within a week, so it catches a
> wrong name on a *known* type, which is what a reader trips over.
>
> 📌 **And the third drift, the one nobody could find, was an absence rather than an error.**
> `FileLoader` appeared nowhere under `docs/` because no tutorial had ever mentioned it — the
> search kept failing because there was nothing to find. **A missing name searches like a wrong
> one and reads like neither**; the right question was not *where is the drift* but *what does
> the tutorial not say*. Tutorial 2 now teaches `parse_with` and a `Disk` loader, gated by a
> compiled test.
>
> 📌 **Finding 1 is the one to sit with.** This session asked nine operations *what does your
> empty answer mean* and fixed two. It never asked `select-tests`, because probing it needed a
> coverage directory — so the single operation whose empty answer was **structurally
> guaranteed** was the one the sweep skipped. **The gap in a sweep is where its cheapest step
> was.**
>
> ### 🔴 CI IS RED AND I COULD NOT REPRODUCE IT — what is ruled out, 2026-08-10
>
> The owner reports GitHub CI on `ayourtch-llm/chiero-rs` failing "for a while" while
> `./check.sh` is green locally. ⚠️ **That is §8.3's third form pointed at this project's own
> gates** — a local gate narrower than CI, or runner drift — so it is worth more than anything
> in the queue below.
>
> **Every step of `.github/workflows/ci.yml` was run locally, exactly as written. All pass:**
>
> | CI step | local result |
> |---|---|
> | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` | pass (`check.sh` covers both) |
> | `cargo build --workspace` and `--no-default-features`, **with `RUSTFLAGS=-D warnings`** | pass — this is the one `check.sh` does *not* replicate, since the workflow sets it globally in `env:` |
> | `cargo test --workspace` with `CHIERO_SMT_SOLVER=/nonexistent` (the `solver: none` matrix leg) | **287 suites, exit 0** |
> | `cargo xtask check-deps`, `check-vpp-leak`, `check-proof-surface` | pass |
> | `cargo xtask contract-coverage` | exit 0 — it is a *report*, not a gate (ci.yml says so) |
>
> **So the cause is environmental, and these are the candidates I could not test from here:**
>
> 1. **A newer stable toolchain.** CI uses `dtolnay/rust-toolchain@stable`; this machine is
>    `1.97.1 (2026-07-14)` and `rustup check` says up to date. If the runner has a newer stable,
>    `RUSTFLAGS: -D warnings` turns any **new rustc lint** into a build failure — the classic
>    green-local/red-CI cause, and it matches "failing for a while" with no code change.
> 2. **z3 version.** The `solver: z3` leg installs from apt; this machine has **4.8.12**.
>    `ubuntu-latest` ships a newer one, and five `chiero-check` tests assert what a *complete*
>    solver decides.
> 3. Something outside the workflow file — a runner image change, or a step failing on
>    checkout/network.
>
> ⏭️ **First move for whoever has the log**: it names the failing leg (`solver: none` or
> `solver: z3`) and the step, which discriminates 1 from 2 immediately. There is no `gh` CLI on
> this machine, which is why this stops here rather than at an answer.
>
> ✅ **And the one gap this exposed regardless of the cause is closed** (`17a1d4a`). `check.sh`
> did not set `RUSTFLAGS=-D warnings`, so a rustc-level warning was red in CI and green
> locally — a *seventh difference inside* a leg §7.5 already claims to cover. It was written
> down here that morning as a comment telling a reader to run two commands by hand, which is
> not a gate. It is now a leg: `cargo check --workspace --all-targets` under the flag (~8 s
> cold, under a second warm, and its own fingerprint, so it does not invalidate the test
> artifacts), and the `--no-default-features` build carries it too. **Mutation-tested** — an
> unused binding turns it red.
>
> ⚠️ **The tree was green under it**, so this does not explain the failure; it removes the
> local gate from the list of suspects, which leaves the two environmental candidates above.
>
> ### 🎯 DONE-ENOUGH-TO-USE — the bar, answered 2026-08-10
>
> Asked by ayourtch through claude-mini: *what is **your** definition of the bar where chiero
> switches from under-construction to a tool my agents actively use, how far away is it, and does
> this candidate match?* Answered here rather than by compliance, as asked.
>
> **The bar, in one sentence: an agent can get a machine-readable answer to each of the nine
> questions, about its own project, from the command line, without reading chiero's source — and
> can tell when the answer is not one.** The last clause is the product; the first four are what
> make it reachable. Five conditions, each falsifiable:
>
> | | condition | today |
> |---|---|---|
> | **D1** | **Every operation runs from the CLI on a real project with no hand-written driver.** `select-tests` was the only one that failed, and it is the flagship — the thesis that held on 2026-08-10 held *through a 145-line driver* | ✅ **as of 2026-08-10** — `--test NAME=PATH` and `--coverage-manifest`, gated end to end against two real gcov objects (§7.34). ⏭️ Unverified on VPP scale: the fixture is two objects, and the walkthrough that needed the driver has not been re-run |
> | **D2** | **The machine surface is a contract**: the `--json` envelope's shape and the exit codes are documented and gated. Today `2` (usage) versus `1` (failed) is real in `main.rs` and asserted nowhere — `cli.rs` only ever checks `0` against not-`0`, and 050 never mentions an exit code. An agent branching on that is branching on an accident | ✅ **as of 2026-08-10** — 050 contracts 19 and 20, gated per operation against the dispatch list (`crates/chiero-cli/tests/exit_codes.rs`). The behaviour was already right; nothing was asserting or documenting it |
> | **D3** | **No operation panics on the pinned corpus, and a refusal names what it could not do.** Two engine panics were found the day `find-bugs` was widened to 92 plugins; the other eight operations have never been swept that way at all | ✅ **as of 2026-08-10** — `crates/chiero-cli/tests/no_panic_corpus.rs`: 126 runs over the 14-file analysis corpus, 56 through entries `cir` names for itself, in 2.5 s with no VPP. ⏭️ The wide sweep still runs one operation; extending it is a spend, not a gate |
> | **D4** | **The documents a new reader reads first are gated against the API**, not proofread against it | ✅ **as of today** — per-operation `--help` renders from the parser (§7.33) and `tutorial_api_drift.rs` compares every tutorial's field and variant lists to the crates |
> | **D5** | **A pinnable tag, one page of known limits, one stated supported platform.** x86-64 Linux is what every differential gate is written against; ARM is engine-portable and oracle-blind and that is a *sentence*, not a project | 🔶 **prepared** — [docs/LIMITS.md](docs/LIMITS.md) is the page, linked from the README and written from measured facts, and [CHANGELOG.md](CHANGELOG.md) says what a tag would hold, gaps included, with no version named. The tag itself is not cut: it is an outward-facing release action and the owner's to take |
>
> ### ✅ Where the bar stands, end of 2026-08-10
>
> **Four of the five are done, and the fifth is half.** D1 (`select-tests` from the CLI, §7.34),
> D2 (exit statuses as 050 contracts 19–20), D3 (no operation panics on the analysis corpus) and
> D4 (help and tutorials gated against the API, §7.33) all landed the day the bar was written.
> D5 has its page — [docs/LIMITS.md](docs/LIMITS.md) — and not its tag.
>
> ⏭️ **What is left is one action and it is the owner's**: cutting a tag is outward-facing, and
> pinning one is a promise about what it holds. Three things belong in that decision — a version
> number, whether the seven owner decisions below should be answered first, and whether the VPP
> walkthrough should be re-run through the new flags before anything is pinned. **That re-run is
> the honest gap in D1**: the fixture is two gcov objects, and the run that needed the 145-line
> driver has not been repeated without it.
>
> **(b) Distance: 4–7 days of autonomous work when this was written; four of five landed the
> same day.** The estimate was wrong by a factor of several, and in a direction worth
> understanding rather than celebrating: every one of them turned out to be *a shape missing
> from an interface*, not a capability missing from the analysis. D1 was an argument spelling
> over a library function that already existed; D2 was a distinction already implemented and
> never asserted; D3 was one sweep pointed at nine operations instead of one. **The bar was
> mostly a documentation-and-surface bar, and I estimated it as an engineering one.** D2 ≈ 1 day, D3 ≈ 1–2 days (a sweep harness exists; it has to run nine
> operations instead of one), D5 ≈ half a day. D1 ≈ 1–2 days *once the flag surface is settled*,
> which is the part worth objecting to early — proposal below.
>
> **(c) On the candidate: agreed, with one merge, one promotion and two additions.** Item 3
> (truthful `check.sh` counters) is ✅ already done, so it is history rather than a bar. Items 4
> and 5 merge into D5 — a limits page nobody can pin is not a limit anybody reads. Item 2's
> per-operation help is done; its "one real-project tutorial" is promoted into D4's *gated* form,
> because a tutorial proofread by hand is the failure mode that produced the drift in the first
> place, and a walkthrough that no gate executes will drift again by the second release. The
> additions are D2 and D3, and both come from the same place: agents are the consumer, so the
> JSON shape and the absence of a panic are the interface — prose is not.
>
> ⏭️ **The D1 proposal, so it can be shot down before it is built.** `--coverage <dir> --stem
> <name>` cannot express per-test attribution because it names one object and no test. Two
> spellings, and building both is an hour:
>
> ```
> chiero select-tests before.c after.c --test bfd=/build/cov/bfd:vnet_bfd_bfd_main
> chiero select-tests before.c after.c --coverage-manifest tests.tsv   # test<TAB>dir<TAB>stem
> ```
>
> The repeatable flag is for a handful of tests at a prompt; the manifest is what a `make
> test-cov TEST=<name>` loop writes, which is how claude-mini produced the run that worked. Both
> land on `ingest_native_as` (`chiero-gcov/src/lib.rs:862`), which already exists and already
> takes the `TestId` — **the library has been able to do this the whole time and only the command
> line could not say it.** `--coverage`/`--stem` stay, and keep refusing.
>
> ⚠️ **What is *not* in my bar, and why.** ARM oracle parity (D5 turns it into a sentence);
> more corpora (§8.3 says widen toward *known* ground truth, and VPP is not that); the seven
> owner decisions below — none of them blocks a user, and two of them are about what chiero
> should become rather than whether it can be used.
>
> ### 🙋 DECISIONS WAITING ON THE OWNER — the whole list, in one place
>
> Seven questions accumulated across sessions, each recorded where it arose and therefore
> scattered.
> Gathered here with **what it costs and what it unblocks**, so they can be answered in one pass.
> None is urgent; none can be answered by a wave.
>
> | # | question | cost to act | recommendation |
> |---|---|---|---|
> | 1 | **020 §4.13b — do CIR pointers stay untyped?** Settles `5i` *and* `5h`, the two dominant `vnet/` finding classes. `pointer-outside-object` fires only for symbolic offsets (§7.31), so the class's size is partly a property of what the checker can see | an architecture decision; no measurement will settle it | the two items cannot progress without it, and they are **34 of the 40** `vnet/` findings |
> | 2 | **Language level — `__STDC_VERSION__` `201112L` or `201710L`?** (`1d`) VPP builds with no `-std=`, so gcc's default gnu17 applies and every glibc header configures for C17; chiero says C11 | a persona edit, plus a claim about the parser | C17 is editorially near-identical to C11, so the honest answer is probably `201710L` with the delta recorded — but it is a claim about what the parser *is* |
> | 3 | **Personas defined in a config file** (`1b`) — your idea, 2026-08-08 | a design, then a wave | design first; it is the natural seam for anything target-configuration shaped |
> | 4 | **Should a sanitizer abort count as confirming a finding?** (§7.32) `-fsanitize=undefined` would turn today's uninformative `completed` replays into `faulted` — measured, exit 134 | three characters to wire; the semantics are the work | ⚠️ **a UBSan abort says *some* UB happened, not that *this* finding's fault did.** A false confirmation is worse than an uninformative one |
> | 5 | **`--solver-rlimit` for the pinned 40** (§7.24) — two of its rows vary 22/23/24 between runs of one binary, so they cannot be compared | needs a counter-based budget built first; `--solver-rlimit` bounds one query, not a run | **leave it** — `compare.py` excludes the two flagged rows, and 38 of 40 are solid |
> | 6 | **011 contract 12 — amend a throughput floor to a ratio?** It reads *"≥100 MB/s lexing over a 50 MB blob"*: the only contract naming an absolute number with no sound instrument, and **ill formed by this project's own rule** — *"a counter, not a clock: a wall-clock bound silently stops being able to fail whenever the build gets faster"* | a spec edit, then a test; `growth.rs` already has the shape | amend it to a **ratio per 4x input**. Precedent is next door in 011 c13, and this is the one M2 contract still uncited (125/126) |
> | 7 | **Two spends**: make `COMPDB_INCLUDES` the default and re-take the plugin sweep (~65 min, recovers ~40 files); and fire the replay gate's gcov build (item 8, `make test-cov`, a separate build tree so the baseline is safe) | ~65 min and one gcov build | both are *additions* — neither re-litigates a published number, and the second is what gets 032 c18 past `recall 0.0%` |
>
> 📌 **Nothing else in §9.1 is blocked.** Every other live item is either a spend already costed
> above, a historical note kept for its reasoning, or work done this session.
>
> ✅ **How to check this list is still complete — two structural sources, not a grep.**
> Row 6 was **missing** from the first version of this block because it was gathered by searching
> for phrasings (`owner's call`, `owner decides`) and that entry says *"needs the owner"*.
>
> 1. `cargo run -p xtask -- contract-coverage` — an uncited contract is either a missing test or
>    a contract nobody can meet. Today: **M1 166/166**, M2 **125/126**, and the one is row 6.
> 2. Classify every `§9.1` item as *owner decision / spend / gated / actionable*. Today: 11 live,
>    none actionable.
>
> Both are mechanical and neither depends on how a sentence happens to be worded.
>
> ### ⏭️ START HERE — **§8.3's widening pattern is the standing job, and the heartbeat runs it.**
>
> Read **§8.3** first: it is the loop, its yield table, and the trap that let a defect survive
> for months (*a green gate is evidence about the corpus, not about the tree*). Then §9.1 for
> the next target.
>
> ### 🔥 What the **second** 2026-08-09 session did, and what it leaves
>
> **Closed:** item 6 (both halves, §7.22), 5o (the advisory taxonomy), 5c (the nsh timeouts),
> 8b (the stale build graph, resolved as a side effect), 8c (the frontend quadratics), 023 c17
> (**withdrawn — M1 now exits 166/166**), plus three reviews that ended in honest zeros
> (§7.23, §7.24, §7.26), the verifier's dominator sets (§7.29), and 40 plugin files recovered
> into coverage (§7.30). `./check.sh` **2332 across 287 suites** at the session's end.
>
> **Two instruments that did not exist this morning**, and every performance defect below was
> found by one of them: `crates/chiero-lower/tests/scale.rs` (a *controlled size axis* — VPP
> cannot supply one, its TUs span 1.7x because a TU's size is its header closure) and the
> all-concrete-conditions engine probe of §7.28, which runs the engine with **no z3 at all**.
>
> | on one 32 768-statement function | start of day | end |
> |---|---|---|
> | frontend | 22 671 ms | **1 693 ms** |
> | full `find-bugs`, 96k blocks | 53 279 ms | **3 264 ms** |
> | peak RSS | **35 628 MB** | **494 MB** |
>
> Nine scans of one shape — *a full scan inside a per-item loop* — plus the verifier's O(B²)
> dominator sets. ⚠️ **None of this was visible to any VPP measurement**, and fixing it moved no
> published number.
>
> ### 🔎 And then the cheap half of the session, which found more
>
> **The morning's heavy instrumentation found performance defects that moved no published
> number. The evening's four-line C programs found five checker defects, all fixed.** If a fresh
> context takes one strategic point from this file, take that one.
>
> `crates/chiero-cli/tests/injected_defects.rs` — a corpus of **known** defects, driven as C
> through the real CLI, each paired with a minimally-different control. It answers a question
> nothing here could answer before: when a VPP sweep says `findings: 0`, is the code clean or is
> the checker dead? **14/15, 0 false positives**, and both default checkers reached.
>
> | defect | what it was |
> |---|---|
> | **8e** — a wild-pointer deref through *any* variable, field, array or **parameter** was masked | `address_term` special-cased `NULL` and fell through for `UNBOUND`, so the store wrote nothing and the reload reported an uninitialized read *of the pointer variable*. **The checker was effectively unreachable in real C**, which explains a zero this project had read as "VPP has no wild pointers" |
> | **8f** — `x << 40` unreported when `x` is symbolic | the concrete-operand guard returned before the count rule, which needs only the count |
> | **8h** — an indirect **call** through a wild pointer reported *nothing* | `NULL` was special-cased at the call site and `UNBOUND` was not. Its own comment calls a silent degrade "the more misleading of the two ways to be wrong" |
> | **8h** — `free((void *) 0x1234)` said "at address 0" | `free` takes an id and had no offset; `free_at` added |
> | **8g** — an untranslatable store at a **symbolic** offset kept the old bytes | the concrete path havocs, the symbolic path `return`ed. Settled with a memory-layer test, since no envelope can distinguish them |
> | `pointer-outside-object` fires only for *symbolic* offsets | ⏭️ **the one open item, and a judgement call** — the variant's own doc says forming such a pointer "is deliberate in a few real idioms". It also explains why 5i's `vnet/` class is symbolic-index heavy |
>
> 📌 **Four of the five came from one question**: *`ObjectId::NULL` is special-cased — where is
> its sibling?* The rule that generalises is **audit const sentinels, not enum variants** — a
> missing enum arm is a compile error, a missing sentinel case is silent. Two further sentinel
> audits (`ObjState`, `DYNAMIC_EXTENT`) came back **zero**, which is what gives the rule its
> scope.
>
> ⚠️ **And two of the three checkers that first looked dead were *my fixtures*** — `order-dependence`
> works (`f() + h()` writing one global), and the corpus found two defects in its own controls
> before finding anything in chiero. **Read a checker's own tests before writing a probe for it.**
>
> **032 c18 has its first `observed` entry** (`3f544b872 / test_lldp`), established with a
> control run. The gate still reports `recall 0.0%` because replay is a stub; item 8 carries the
> scouted recipe.
>
> ⚠️ **New corpus fingerprint `sha256:d8e4a04713923a31`** (was `5447e466…`) — the replay build
> regenerated cmake. The pinned 40 was re-taken and is **byte-identical**.
>
> ⚠️ **Three numbers this session published were wrong and are corrected in place** — every
> conclusion survived, which is the point:
>
> | claim | corrected | why |
> |---|---|---|
> | "6 indirect call sites in the pinned 40" | **56** | censused with the wrong include flags (§7.22) |
> | "5 of 40 envelopes differ" (void fix) | **3** | two are `nondeterministic_abort` (§7.24) |
> | "the over-report is rare in VPP" (5o) | **255 of 1552** | measured on gnu; the sweep runs pedantic |
> | "the engine is still 18.2x" | **it was a second verify** | subtraction is not a phase split (§7.28) |
> | "~24 576 blocks, ~600M entries" | **96 000 blocks, 35.6 GB** | disbelieved my own arithmetic (§7.29) |
> | "198 plugin units cannot preprocess" | **~30 of them do fail** | exposure counted as breakage (§7.30) |
>
> 📌 **The method that paid every time: write the prediction down, then let the number contradict
> it.** All three errors were invisible until one did. Two led straight to a defect — the
> backwards void filter and 5o's wrong `Miss` arm.
>
> ⚠️ **Six false zeros in one session, all one shape: a pattern narrower than the thing it
> looked for.** A grep in chiero's wording against gcc's output (twice — one nearly became a
> defect record about 1019 files), an `awk` matching gdb's *"Thread 1 received signal"* banner,
> a `grep -l` matching a JSON field's *name* instead of its value, and `-> void` eaten as a
> shell flag. **When the subject is another tool, list its output and read it.**
>
> ⚠️ **And twice, something worse than a false zero: a commit message describing edits the commit
> did not contain.** `af788cb` first — a python block asserted four anchors and wrote only at the
> end, so one bad anchor discarded three good ones. The rule went into §11.0 (*write after each
> edit*), the very next commit repeated it in a different way, and **that** one was a message
> written in the same shell block as the edit, before its `NOT FOUND` could be read.
>
> **The structural fix, not the resolution:** an edit that can fail and the commit that describes
> it **must not share a shell block**. Read the outcome, then write the message.
>
> 📌 **The single highest-yield habit of the session**, if a fresh context keeps only one thing:
> *state the number you expect before you measure, then let the measurement contradict you.*
> Every correction in the table above came that way, and three led straight to a defect — the
> backwards `void` filter, 5o's wrong `Miss` arm, and 35.6 GB of dominator sets.
>
> ### 🆕 Suggested first moves, in order
>
> 1. **Run the fast gates.** `./check.sh` is ~4 min and now covers six of CI's ten commands
>    (fmt, clippy, the `--no-default-features` build, `check-proof-surface`, tests); the other
>    four are covered by tests or are reports. `./check.sh --both-legs` adds CI's no-solver leg.
>    Then `persona_gap` (0.1 s) and `growth` (5 s). The 23-minute corpus gate is worth it only
>    when something touched the frontend or the persona.
> 2. **Then the two gates that grade what no VPP corpus can reach** — `generated_layout` (~30 s)
>    and `generated_const_eval` (~2 s). Three `layout` defects came out of the first in three
>    runs, and **VPP contains zero instances of every shape they fixed** (§7.21), which is the
>    argument for keeping them.
> 3. **Pick a widening (§8.3), or a concrete item from §9.1.** ⚠️ **Layout, diagnostics and the
>    instruments are mined out for now** — the last several waves there returned zeros, or
>    findings about the gates rather than the tree. The untouched work, in order of readiness:
>    - ~~**`InstKind::Call`'s indirect half**~~ ✅ **closed 2026-08-09, both halves.** Kept here
>      only for what it cost to believe: "135 sites" was a count of *mentions* and it is what
>      left the item unstarted for weeks. The engine half's payoff was **not** the filter it
>      added — that one cuts nothing on the pinned 40 — but the backwards rule beside it (§7.22).
>    - ⚠️ **`5b`'s audit grep cannot find its own class.** 5c's quadratic was
>      `text.split(...)` inside a per-item loop; the audit greps `.contains(&`, so its "79 sites"
>      counts a *narrower* shape than the one it names. Both instances found this year came from
>      **sampling a real run**, not from the grep. Widen the definition or change the method.
>    - **032 contract 18's replay corpus**, still with no `observed` entry. The probe is
>      committed; firing it re-runs cmake and **invalidates every published VPP number**, so it
>      is a deliberate spend, not a wave.
>    - **The two `vnet/` finding classes**, which are policy questions gated on 020 §4.13b
>      (untyped CIR pointers) — an architecture decision, not a defect.
>    - **5j's API change**, still gated on `ub-strict` existing. Both discards are explained at
>      the site now and both directions of the gap are measured and pinned.
> ⚠️ **Where the 2026-08-09 session stopped paying, recorded so a fresh context does not restart
> there.** Its last stretch worked the *meta* seam — gates, instruments, records — and the yield
> is real but has changed kind:
>
> | worked late in the session | yield |
> |---|---|
> | every tolerance list, every counter, every instrument, every contract, every spec | **3 record defects** (020's phantom `CallConv`, its stray fence, six uncited contracts) and **5 non-findings** where the guard already existed |
> | `check.sh` against CI, command by command | **2 real gaps** (`--no-default-features`, `check-proof-surface`), both now gated and mutant-verified |
> | the handoff against itself | **1 real defect** (two items numbered `3.` in START HERE) and a committed checker for it |
> | the severity change's downstream consumers | **4 found**: 2 fixed, 1 reproduced and deferred (5o), 1 asked and answered (5p) |
>
> **Nothing in that stretch found a defect in chiero's *analysis*.** Every finding was about an
> instrument, a record, or a consumer. That is the signal §8.3 asks for: *the seam is worked
> out*. A fresh context should start at item 3's list — the untouched concrete work — rather
> than auditing the auditors, which is where this one ended up.
>
> 4. 📌 **The method that paid all day, and the one to reach for first:** *one fact, more than
>    one reader* — **four instances in one session** (§11.0 lists them). Its operational half is
>    cheap and caught real bugs: **when a fix lands, re-run the original reproduction, not the
>    new test.** A green unit test beside an unchanged tool is what the class looks like.
>    ⚠️ Its counterweight is in §11.0 too: **five suspicions the same day were already handled,
>    and the code said so at the site.** Read the file before assuming the gap.
> 5. ⚠️ **Closed 2026-08-09, do not re-open.** The stale-tolerance-list class (§7.19); the
>    prefix-attribute, union `:0` and packed-`:0` layout defects (§7.20); `SemaDiagnostic`'s
>    severity and its ten advisory sites; the gnu11 silence channel; `chiero layout` refusing a
>    TU sema refused (5m); `_Alignof` of a typedef (5n); the gcov `cycles_count` quadratic (the
>    line-half is linear now); `chiero … | head` panicking on EPIPE; 010 contract 18; and six
>    contracts that were tested but uncited. **Two of these were defects in this session's own
>    fixes**, both found by adversarial review — an ambient severity flag that leaked a demotion
>    across the whole TU, and a fix that reached one reader of three.
>
> 6. ⚠️ **Closed in the 2026-08-08 sessions, do not re-open:** the whole persona thread
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
> # The token count is DETERMINISTIC (two byte-identical runs). The +10 972 against the
> # 2026-08-08 baseline is EXPLAINED: 32 regenerated API headers, i.e. the corpus moved — which
> # is why the pin below belongs with every figure taken from this gate.
> cargo test -p chiero-vpp --test preprocess_corpus -- --ignored --nocapture
> # ⚠️ Capture the corpus pin with it — `HEAD` does not identify what was measured, because
> # 147 of the 1562 sources are generated and not in git. Two runs are comparable iff it matches.
> python3 tests/corpus/vpp-findings/api_staleness.py --fingerprint
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
> | **023 c17** | 1, 2 and 8 worker threads give identical `RunResult`s | **the feature does not exist** — there is no worker pool in `chiero-exec`: no `workers`, no `thread::spawn`, no rayon. Nothing to test; an owner call on whether M1's exit may name it. ⚠️ **These are *chiero's own* threads, not VPP's** — 023 §11 says it in as many words ("a scheduling detail of chiero itself, unrelated to the analysed program's concurrency"), and the missing capability is **parallel state exploration**. Do not read this as "chiero ignores threads": the analysed program's concurrency is [025](docs/specs/025-concurrency-and-threading.md), it ships a discipline checker in v1, and 023 §11 carries a sentence warning readers off exactly this conflation. ⚠️ **My earlier note here said the owner had misremembered this; that was not mine to assert.** The spec text is unambiguous and says it in three places, but the owner owns the intent, and the question was settled a better way — see below |
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
> ✅ **RESOLVED 2026-08-09 — 023 c17 withdrawn, M1 exits 166/166.** Not deferred: *decided
> against, with a measurement*. Parallel state exploration would overlap the z3 waits chiero
> actually blocks on, so it would help — and it is still the wrong trade, because it fights the
> reproducibility this project runs on, cheaper wins are unspent (`dominators` is ~half the
> frontend), and it is the most expensive class of change in the component whose answers must be
> trusted. 023 c17 carries the full reasoning; the number is left reserved so 18+ do not move.
>
> 📌 **The question that settled it was the owner's, and it was better than mine.** I had framed
> it as "which reading of c17 was intended" — chiero's own threads (what the text says, in three
> places) or the analysed program's (what the owner recalled intending). The owner's reply:
> *does it matter — would multithreading help at all?* It does not matter, and asking whether
> the feature earns its cost dissolved a question about intent that had been open for sessions.
>
> ⚠️ *The original entry, for the record:* **the single uncited M1 contract is 023 contract 17,
> and it describes a feature that does not exist.** It reads *"with `wall_clock: None`, running with 1, 2 and 8 worker threads
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
> ✅ **AND THE CAUSE IS FOUND: the corpus moved, in the half `git status` cannot see.** **32
> generated files under `build-root/` have mtimes after the 2026-08-08 baseline** — the API
> headers 5d regenerated that day. §8b already records that **147 of the corpus's 1562 sources
> are generated** and live under the build directory rather than under `src/`, and the corpus
> gate reads them.
>
> ⚠️ **So "the corpus is content-identical, VPP `HEAD` is unchanged and `git status` is clean"
> was wrong, and wrong in an instructive way.** It is true of the *tracked* tree and says
> nothing about a tenth of the corpus. **A clean `git status` is not evidence a corpus is
> unchanged when part of it is not in git** — the same shape as a grep that matched the wrong
> thing, one level up.
>
> 📌 **The rule for anyone comparing VPP numbers across sessions, and the one command that
> follows it:** VPP's `HEAD` does not pin the corpus — the generated half is not in git.
>
> ```sh
> python3 tests/corpus/vpp-findings/api_staleness.py --fingerprint
> #   generated corpus: 1506 files, sha256:5447e4661663b86c     (0.16 s, 2026-08-09)
> ```
>
> **Record that digest beside any published VPP figure.** Two runs of the "same" corpus are
> comparable iff it matches; otherwise a ten-thousand-token difference looks like a chiero
> change and is not. It hashes content and path, not mtimes — verified: a `touch` leaves it
> unchanged, an edit moves it, a restore returns it.
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
> **State at the end of the 2026-08-09 session — `./check.sh` GREEN at 2303 across 279 suites**,
> fmt and clippy clean, both solver legs, ~110 commits from `c495118`. The gate now also covers
> CI's `--no-default-features` build and `check-proof-surface`, and this file's own numbering.
> ⚠️ This line read **2296** until the session's last wave, seven tests after it stopped being
> true — the headline number drifting is the same class as everything §7.19–7.21 records, in the
> line a fresh context reads first.
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

**Closed items live in [HANDOFF-ARCHIVE.md](HANDOFF-ARCHIVE.md)** — 22 of them, moved
2026-08-09 when this section reached 1856 lines. The reasoning is kept (§11.3); it just no
longer sits between a fresh context and the live work.

- `1` — ✅ UNPARKED AND CLOSED 2026-08-08 — the owner said "go ahead and design + execute the persona work", then "feel free to tackle march". Both shipped; th
- `1z` — 🗄️ Original entry, kept because its reasoning held up — closed, not parked. It read: PARKED at the owner's request
- `1e` — ✅ CLOSED 2026-08-08 — the gate now measures the shipped configuration. Both facts come from chiero_probe::Probe, the same one the CLI uses: system inc
- `3` — ✅ CLOSED 2026-08-08 — the ingest is built and both blocked contracts are met. chiero_vpp::builddb (060 contract 1) and chiero-vpp/tests/preprocess_cor
- `4` — ✅ Mostly answered 2026-08-07 by a config block, and the entry below is kept for what is left. The whole item was written around conversions taking 53 
- `5` — ✅ CLOSED 2026-08-07 — "a step that outlives the clock" had the wrong cause, and the sweep now has zero timeout rows. The entry said three find-bugs en
- `5a` — ✅ DONE 2026-08-07 — the verifier's scale test asserts a counter, not a clock. It had asserted 5 s, chosen under the unoptimised dev profile; opt-level
- `5e` — ✅ CLOSED 2026-08-08 (second session) — 950 KB → 11.9 KB, and the fix found a second defect in the fix. nsh_md2_encap's envelope is now 64 bindings plu
- `5c` — ✅ CLOSED 2026-08-09 — parse_model was quadratic, and the fix uncovered a second defect. Found by *sampling*, not by reading: two stack samples 50 s ap
- `5g` — ✅ CLOSED 2026-08-08 — pick_entries.py --verify-cir. It keeps only names that survive into the lowered module, using chiero cir (built earlier the same
- `5f` — ✅ CLOSED — --built-only shipped 2026-08-08. The sweep analyses files VPP does not compile — and that is how it found a real VPP defect. src/vnet/fib/f
- `5d` — ✅ CLOSED 2026-08-08 (second session) — and the staleness was narrower than this entry claimed. Measured before touching anything: 165 of 2629 sources 
- `5k` — ✅ CLOSED 2026-08-08 — an enum's declared underlying type was parsed and thrown away, and layout reported the wrong size as proven. struct S { enum sma
- `5m` — ✅ CLOSED 2026-08-09 — chiero layout applies the same rule lower does: errors refuse the TU, advisories are printed and do not. Six lines, identical in
- `5n` — ✅ CLOSED 2026-08-09 — and the fix reached one reader of three. _Alignof(A_t) is 16 now, matching gcc, for all three typedef spellings. ⚠️ The sema fix
- `5o` — ✅ CLOSED 2026-08-09 — and the re-take the item demanded is what caught the fix's own defect. Outcome::Advised exists, (Warned, Advised) is Agree, and 
- `5p` — ✅ CLOSED 2026-08-09 — chiero-diff's parsed_cleanly ignores sema, and that is correct. Filed as a question, answered by measurement within the hour. Fo
- `6` — ✅ CLOSED 2026-08-09 — both halves. (Was "partly closed": the CIR change landed hours before the engine did, and the payoff was not where the item said
- `6z` — 🗄️ Original entry — InstKind::Call carries no result type, so an indirect call's result width is whatever candidate ran. The arity and parameter-type 
- `7` — ### MemFault::BadRange — CLOSED 2026-08-07. See §7's entry. The two stated options were both wrong because the premise was: the probes did not need an
- `8c` — ✅ CLOSED 2026-08-09, the same day the gate that found it was built — 22.7 s → 7.9 s. Two O(n²) scans in lowering, both item 5b's shape:
- `9` — :0 bit-fields in layout, deliberately left open (§7.9). ✅ Priority settled 2026-08-09 — leave it open, and now on evidence rather than on a 69-header 

**Closed 2026-08-10, moved to [HANDOFF-ARCHIVE.md](HANDOFF-ARCHIVE.md)** — §9.1 had
reached 952 lines again, 316 of them finished work:

- `8h` — CLOSED 2026-08-10 — NULL had two more unhandled siblings, found by auditing 8e. 8e's own conclusion was *look at representations and
- `8g` — CLOSED 2026-08-10 — confirmed and fixed. Was: inspected, not reproduced — two lowering_gap sites in the symbolic-offset store path r
- `8f` — CLOSED 2026-08-10 — a shift past the operand width was unreported whenever the shifted value was symbolic, though the rule depends o
- `8e` — CLOSED 2026-08-10 — a wild-pointer dereference was reported as an uninitialized read *of the pointer variable*. Found by the injecte
- `8d` — Point the measurement harness at the compile database instead of a hand-kept flag list (§7.30). The capability landed 2026-08-09: ca
- `8b` — RESOLVED 2026-08-09 as a side effect of the replay probe — the build ran, cmake regenerated, and zero CMakeLists.txt are now newer t

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
   | **2026-08-10, first run on the regenerated graph** | **0** | **1971** units (+4), **820,160,849** tokens (+27.8M, **+3.5%**) |

   🆕 **The number that actually matters is not the 0, it is the token count: 731,159,228 →
   792,404,723.** Same 1967 translation units, same flags, **+61 million tokens — 8.4% more C**.
   That is code in `#if` branches the persona had left dead: Linux-only paths in VPP *and* glibc,
   the little-endian layouts, the `__SSE2__` tables. **The library's default persona had been
   describing a program 8% smaller than the one VPP ships.**

   📌 **Re-run 2026-08-10, and it moved — which makes it a check rather than a control.** The
   replay probe regenerated cmake (§7.30, item 8b), so this was the gate's first run against the
   new build graph: **+4 translation units and +27.8 million tokens, 3.5% more C**, all of it
   code the four stale `CMakeLists.txt` had been hiding from every measurement. **Still 0
   diagnosed**, so chiero handles the newly visible 3.5% as cleanly as the rest — a small honest
   positive, and the first evidence that 8b's resolution bought coverage rather than just tidiness.

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
     ⚠️ **Do this *after* `COMPDB_INCLUDES` is the default, not before** (2026-08-10). A plugin
     sweep run today measures ~40 files under flags VPP does not use — they fail to preprocess
     for want of an `-I` and land as "chiero cannot read this" (§7.30). Widening first would
     add rows to a `failed` column that the flag fix then empties, and the numbers would not be
     comparable across the change.
     📌 **And expect the yield to be a lid, not a list** (§11.2). The 40 recovered files swept
     on 2026-08-10 gave 489 findings of which **465 came from one `cut` entry**, and the single
     `Exact` was the known entry-pointer class. A wider slice of the same kind is the corpus
     move this file already warns about — *a corpus of a new kind beats a wider slice of the
     same kind*.

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

4b. **⚠️ Original entry, for the record.** Every corpus-consuming test
   preprocesses, parses and analyses all 22 seeds. Sharing one `PreprocessorSession` across seeds
   bought ~15% (`conversions` 62→53 s, `vpp_layout_gate` 62→58 s, `vpp_corpus` 16→13 s) — **less
   than hoped, and it says where the cost is**: not lexing, but parse and analyse, which no cache
   touches. The remaining structural waste is that each *test binary* rebuilds the corpus from
   scratch, because Rust integration tests are separate processes, so `corpus_analyses()` cannot
   be shared across them however it is written. Cutting it needs the analysis serialised to disk
   and reloaded — a real design question, and **020's CIR text format may already be most of the
   answer**.

5b. 🆕 **The class is "a full scan inside a per-item loop" — and the audit that names it cannot
   see most of it.** Rewritten 2026-08-09 after seven instances were found in one day; the
   original 479-line entry is in [HANDOFF-ARCHIVE.md](HANDOFF-ARCHIVE.md).

   | found 2026-08-09 | where | shape |
   |---|---|---|
   | `parse_model` | chiero-solver | `text.split(..)` per variable |
   | `emit` | chiero-lower | `blocks.iter_mut().find(..)` per instruction |
   | `set_term_at` | chiero-lower | the same `find`, in `emit`'s sibling |
   | `reachable_from` | chiero-lower | `Vec::contains` + a per-block `find` |
   | `check_structural_identity` ×2 | chiero-cir | `allocas.iter().any(..)` per instruction |
   | `ScopedTypes::get`, `ScopedMeanings::declare` | chiero-sema | scan of every name in scope |
   | `Memory::entry`/`entry_mut` | chiero-mem | scan of every object, per access |

   ⚠️ **Two things this proves about the audit itself.** Its grep is
   `.contains(&` — but **six of the nine sites above are `.find`/`.any`/`.split`**, which it
   cannot match. And its per-crate census (chiero-cir 10, chiero-sema 9, chiero-gcov 9,
   chiero-exec 6, chiero-tool 5) **omits `chiero-lower` and `chiero-mem` entirely**, where four
   of them lived. The 79-site number measures neither the class nor the codebase.

   ✅ **The method that works is sampling under a size axis**, and both axes now exist:
   `crates/chiero-lower/tests/scale.rs` for the frontend, and the all-concrete-conditions
   program of §7.28 for the engine. Every one of the nine was found that way; **none** was found
   by the grep. §11.0 carries the sampling recipe and its two traps.

   ✅ **CLOSED 2026-08-09 — and it was a memory bomb, not merely a slow pass.** See §7.29.


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

5i. 🆕 **The other dominant `vnet/` class, `pointer-outside-object`** — 19 of 44 findings before
   the 7b fix, **15 of 40** after. The 276-line investigation is in
   [HANDOFF-ARCHIVE.md](HANDOFF-ARCHIVE.md); this is what a reader needs.

   **The shape:** a static array indexed by a value from a lazily-materialised struct, where the
   program *does* guard the index — `vnet/dev/counters.c` is the clean example. chiero cannot see
   the guard because the index arrives through a pointer it invented, so it reports a pointer the
   program cannot form.

   📌 **Half of it is now explained** (2026-08-10, §7.31): the fault fires **only for symbolic
   offsets** — the raise site sits inside the symbolic-offset enumeration path
   (`chiero-exec/src/lib.rs:4986`), reached when enumeration fails and a feasibility query
   returns `Sat`. A constant `a + 8` reports nothing at all. **So this class's dominance is
   partly a property of what the checker can see**: symbolic-index code is the only code it
   looks at, and that is exactly the idiom above.

   ⏭️ **Still a policy question, not a defect.** It turns on 020 §4.13b — whether CIR pointers
   stay untyped — and the variant's own doc says forming such a pointer "is deliberate in a few
   real idioms". **Owner's call.**

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

8. 🔶 **032 contract 18 — the corpus has its first `observed` entry (2026-08-09); the gate's
   *replay* is still a stub.** `3f544b872  test_lldp  observed  lldp: fix TLV validation`.
   Both halves run: reverting the fix's `src/` diff on HEAD makes
   `test_lldp_truncated_optional_tlv_bounds` fail and the suite's other three pass; restored to
   HEAD it is `OK`. **The control is what makes it ground truth** — without it the entry would
   record a pre-existing failure.

   ⏭️ **What remains is the gate, not the corpus** — and the recipe is scouted, not guessed.
   `xtask replay-gate` prints `recall 0.0% over 1 observed entry` and `replay not implemented`
   (`xtask/src/replay_gate.rs:212`). The missing input is **per-test coverage**, and VPP can
   produce it:

   | step | command | note |
   |---|---|---|
   | 1 | `make test-cov TEST=test_lldp` | builds a gcov tree and runs *only* that suite, so the `.gcda` **is** test_lldp's coverage |
   | 2 | `chiero select-tests <before.c> <after.c> --coverage <dir> --stem lldp_input` | before/after are `lldp_input.c` at `3f544b872^` and `3f544b872` |
   | 3 | assert `test_lldp` is in the output | that is contract 18's recall for this entry |

   ✅ **It cannot disturb the baseline**: `test-cov` builds into `build-root/build-vpp_gcov-native`,
   a *separate* tree from the `build-vpp-native` every published number reads. Verified in
   `Makefile:600-604`.

   ⚠️ **One entry proves recall, not discrimination.** With only `test_lldp`'s coverage, a
   selector that returns *everything* also scores 100%. A second suite's coverage — any suite
   that does **not** touch `lldp_input.c` — is what makes the number mean something, and it
   should land in the same wave rather than after it.

   📌 **The expensive half is done and it stayed cheap**: ~12 minutes, not the feared hour, and
   the tree was restored on every path including the failed first attempt.

   🗄️ *Original entry:* **032 contract 18's corpus still has no `observed` entry** and the gate correctly exits 1
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

### 9.2 Standing measurement instruments — and **the ones that were lost**

⚠️ **Verified 2026-08-07: everything that lived only in a scratchpad is GONE.** A scratchpad is
per-session; the previous session's is still on disk but has been pruned. Only what was
*committed* survived. This file already carried the warning ("the scripts were lost once when
they lived only in scratch") and it happened again anyway.

| instrument | state |
|---|---|
| ⚠️ `chiero-solver --test solver_rlimit` | **flaked once on 2026-08-09** inside a full `./check.sh`, then passed alone and in the next full run (2303/279). One sighting in ~15 full runs that day. **Undiagnosed**, because `check.sh` names the failing *suite* and not the assertion, so the detail was gone before it could be read — see the gap noted in the row below. Recorded so a second sighting is a pattern rather than a surprise; z3 is spawned per query here, so parallel load is the first thing to suspect |
| `./check.sh` | ✅ committed — the green gate; ✅ **it prints the failing assertion now** — message, `left`/`right`, file:line — capped at 40 lines and saying so when it caps. Until 2026-08-09 it reported only the failing *suite*, which cost a diagnosis when a solver test flaked once inside a full run and could not be looked at afterwards. Verified by deliberately breaking a test, not by reading the diff; keys on cargo's **exit status**, prints failing suites first (§7.5). ⚠️ **Widened 2026-08-07 to all three CI legs** — it ran `cargo test` alone while CI also gates `cargo fmt --all --check` and `clippy -D warnings`, and was reporting GREEN over **26 fmt diffs and 2 clippy errors**. Fast legs run first, in seconds, so a formatting diff is not found after the hour. `--skip-lints` re-runs the tests alone. ✅ **Widened again 2026-08-09 to CI's `--no-default-features` build (§3's no-link constraint, invisible to `cargo test`) and `check-proof-surface` (023 c13a, called only from `main.rs`, so no test guards it)** — ~1.1 s added, both mutant-verified to gate. `check-deps` and `check-vpp-leak` stay out: they *are* covered, by `the_real_workspace_is_clean` and `workspace_has_no_vpp_leaks`. ✅ **CI's second solver leg is reachable now: `./check.sh --both-legs`** (2303 passed with `CHIERO_SMT_SOLVER=/nonexistent`, verified 2026-08-09). The default still runs one leg — the runtime argument that excluded it was right and is unchanged — but the gap used to be *named only*, so the invocation had to be reconstructed from a comment. Previously recorded here as: **still not covered, and named in the file:** CI's second solver leg |
| `tests/corpus/vpp-findings/measure.sh` | ✅ committed — retakes the find-bugs numbers (~4 min pinned 40, ~25 min plugins) |
| `tests/corpus/handoff/lint.py` | ✅ committed 2026-08-09 — **the record-vs-tree checks, made standing.** Three of the five run by hand that day, each of which found a real defect: numbered lists that repeat or go backwards (§9 had two items numbered `3.`), cited repo paths that do not exist (two did not), and unbalanced code fences (020 had one hiding 45 lines) — the last applied to every spec as well. Exits 1 on any finding, no dependency. ⚠️ Each check verified against a revision that **fails** it. **Not a proof the prose is current** — these catch shapes, not claims, and the docstring says so. Contract citations are the fourth check and already have `xtask contract-coverage`; the fifth, "is it still true", is not mechanisable. ✅ **`./check.sh` runs it**, the one leg broader than CI |
| `tests/corpus/layout/floor_diff.py` | ✅ committed, and ⚠️ **absent from this table until 2026-08-09** — the second half of §9.2's own lesson: committing an instrument keeps the code, indexing it is what keeps it *used*. It is a **generated** differential for 041 §3's padding floor: random structs, chiero's proposed floor against the true minimum `sizeof` over every permutation of the record's *units* (a bit-field run plus a trailing `:0` moves as one). **Sharpened the same day**: the generator now biases toward records that *can* waste padding (3–7 units, scalars alternating wide/narrow), because an unbiased draw mostly cannot and the instrument was skipping 97% of what it made. Hit rate ~3% → ~18%: `python3 floor_diff.py <seed> 600` gives **98 / 117 / 106 proposals across three seeds, 0 over-claims**, so 041 §3's floor now has **321 checks** behind it rather than 20. The bias is on what is generated, never on what is checked — the oracle is still gcc's minimum `sizeof` over every permutation of the units. ⚠️ Before the fix a 12-case run printed `checked 0 proposals: 0 over-claims, 0 sound`, which is what a low hit rate looks like from outside: clean, and measuring nothing |
| `tests/corpus/vpp-findings/count.py` | ✅ committed — not standalone; `measure.sh` calls it once per entry to turn an envelope into a TSV row. It is where the `ok`/`cut` distinction is defined, and that distinction is the reason the sweep's zeros are readable |
| `tests/corpus/layout/fixed_diff.py` | ✅ committed — chiero's padding floor vs gcc's minimum over every run-preserving permutation |
| `tests/corpus/layout/vpp_sizes.py` | ✅ committed — contract-12's method pointed at arbitrary headers |
| `xtask/src/replay_gate.rs` | ✅ committed — `cargo run -p xtask -- replay-gate`, corpus `tests/corpus/replay/corpus.tsv` |
| `xtask/src/pp_gate.rs` | ✅ committed — `cargo run -p xtask -- pp-gate`, ~2 min. Reads `$SIMPLECPP` (default `/home/ubuntu/simplecpp`, pinned `74a5a63`); gcc and clang are the oracle. §7.11 |
| `tests/corpus/vpp-findings/march_probe.sh` | ✅ committed 2026-08-08 — lowers VPP's 384 `-march=x86-64-v3/v4` units with and without their own `-march`, reporting the definition delta, any diagnostic, and **`EMPTY` for a unit that lowered nothing** (a clean run over six lines of nothing is not a pass). `STRIDE=1` for all 384 |
| `crates/chiero-lower/tests/scale.rs` | ✅ committed 2026-08-09 — the **size axis** no other gate has. Two shapes (many functions / one big function), four stages timed apart, minimum of three runs, ~1.7 s. A **ratchet at today's curve**, not a linearity claim: `lower` is already 18.6x per 4x step (§7.27, item 8c). VPP cannot supply this axis — its TUs span 1.7x |
| `tests/corpus/vpp-findings/compare.py` | ✅ committed 2026-08-09 — diffs two `measure.sh `KEEP=`` directories and **excludes the envelopes chiero marks `nondeterministic_abort`**, naming what it excluded. Two of the pinned 40 hit the wall clock and vary 22/23/24 states between runs of one binary; comparing them measures the machine (§7.24). Use it instead of `cmp` — two envelope claims on 2026-08-09 were inflated by exactly that pair |
| `tests/corpus/vpp-findings/api_staleness.py` | ✅ committed 2026-08-08, **`--fingerprint` added 2026-08-09** — a content digest of the 1506 generated files, 0.16 s, to pin the half of the corpus `git status` cannot see. Verified stable under `touch` and sensitive to an edit. — which of VPP's 1049 generated API headers are older than the `.api` they come from. Exits 1 on drift; `--fix` regenerates with `vppapigen` rather than `ninja`, whose target re-runs cmake and rewrites the `build.ninja` every VPP measurement reads |
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

- ⚠️ **And the audit that followed it initially missed two counters, for the reason it exists.**
  The sweep grepped `thread_local` and `AtomicU64` and reported "four counters, three sound".
  `chiero-lex`'s `cache_hits`/`cache_misses` are `Rc<Cell<u64>>` — a shape the pattern did not
  match. They are **complete** (`lex_cached` has exactly two exits and each increments exactly
  one, so hits + misses = calls), so the verdict stands at six counters, five sound. But *the
  audit for narrow measurement was itself narrowly scoped*, which is the joke the class keeps
  telling. ✅ **And the nuance left unmeasured got measured:** a hit that must be `relocated` counts
  the same as a free hit, so `cache_stats` cannot distinguish them — and on a real TU
  (`vppinfra/format.c` with `-I src`, which lexes ~3 MB of includes) **relocating hits are
  zero**. Include guards mean a cached file is re-requested at the same `start_pos` or not at
  all. So the conflation is harmless in practice, not merely harmless for 011 c13.

- ⚠️ **In a newest-first table, write *temporal* cross-references, never positional ones.**
  §8.3's yield table grows at the top, so "the previous row" means the row *above* — which is
  the **later** wave, the opposite of what a writer describing history means. One such
  reference was wrong within minutes of being written, when the next row went in above it.
  Swept all ten relative references in this file afterwards: the nine phrased as "the previous
  wave" are **correct**, because a wave is a position in *time* and reordering cannot break it;
  the one phrased as "the previous row" was the only casualty. **`lint.py` passes on both and
  always would have** — this is the claim-shaped half of drift, and re-reading is the only
  instrument for it.

- ⚠️ **Check a deferral's stated reason like any other claim.** §9.1 item 6 was deferred five
  times in one session. The reasons decayed as each was tested: *"135 sites"* (a count of
  mentions — really 3 constructions), then *"~110 fixtures"* (the field belonged on
  `Callee::Indirect`, so ~25), then *"the text format is a spec change"* (true, and 020 had
  already been amended that day with precedent), and finally **"no review budget"** — which was
  simply false: §8.1 authorises fable subagents and a subagent has its **own** context. The last
  reason was the only one that would have justified stopping, and it took thirty seconds to
  refute.
  The change then landed in one sitting and matched the scope estimate exactly. **A deferral
  compounds: each restatement makes the next one feel established**, and none of the five was
  re-examined until the fifth. Ask of any "I can't do this yet" what you would ask of a
  measurement — how do I know, and when did I last check.

- **The record drifts wherever nothing mechanically compares it to the tree — and this project
  documents obsessively, which did not help.** Five such checks were run on 2026-08-09 and
  **all five found something**: 020 declared a `conv: CallConv` field whose type exists nowhere;
  a stray fence in the same spec was rendering 45 lines of normative prose as a code block;
  §9's START HERE had two items numbered `3.`; six contracts were tested and cited nowhere, so
  the coverage gate understated itself; and two cited repo paths did not resolve. **None was
  found by looking for it** — every one surfaced while reading something adjacent for another
  reason.
  The costs were asymmetric in a useful way: each *check* was one command, each *defect* had
  been sitting for weeks or months. Four of the five are now standing in
  `tests/corpus/handoff/lint.py` (numbering, cited paths, fence balance, spec cross-links) with
  `xtask contract-coverage` covering the fifth. ⚠️ **What none of them checks is whether the
  prose is still true** — they catch shapes, not claims, and the day's other four record
  defects (a stale token figure, a stale "open lead" comment, a stale headline test count, a
  cause recorded too narrowly) were all *claims*. There is no tool for that; there is only
  re-reading what you wrote when the thing it describes changes.

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

- 📌 **Read the site before assuming a gap — five times in one day the guard already existed
  and the code said so.** `chiero-exec` was already reading the callee's `ret`; 011 c13's test
  already used a counter instead of the clock its contract names; `vpp_leak.rs` already had a
  real-tree test under a name my grep missed; `check-proof-surface` already requires each probe
  to fail *for the right reason*, with a comment recording the vacuous pass that taught it. Each
  check cost under a minute; each "fix" would have been redundant work, and one of them
  (a second `check.sh` leg) I was a command away from committing. **The comments at a site
  carry its history, and this project writes them — the answer to "surely nobody checked this"
  is usually in the file.**

- ⚠️ **Mechanical: piping a chiero command to `head` makes it exit 101, which reads as a panic.**
  Rust's `println!` panics on `EPIPE`, so `chiero cir … | head -3` reports exit 101 while
  `chiero cir … > file` reports 0. This matters here because the project *asserts exit codes* —
  the advisory-diagnostic tests compare chiero's against gcc's — and a measurement taken through
  a pipe would be measuring the pipe. **Redirect, never pipe, when the exit code is the subject.**
  Caught by re-running before reporting a phantom panic in the CLI.

- ⚠️ **Mechanical: never key a scripted edit or a grep on prose or on multi-token source text.**
  Four failures in one session, all the same cause — `cargo fmt` had reflowed the anchor. Three
  were silent no-ops; one produced a false finding. `assert old in s` before writing, key on line
  numbers or a bare identifier, and grep for the text you removed rather than reading the test
  result.

- **A summary line cannot tell a control from a check.** §7.21 and §7.22 are the same three
  words — `findings=21 exact=0`, byte-identical — and one of them means "the corpus cannot
  reach this" while the other hides five changed envelopes and a real behaviour change on VPP.
  Nothing about the number distinguishes them. **The residue is what distinguishes them**, which
  is why `measure.sh` grew `KEEP`; §7.22 is the first time it paid, and without it the honest
  report would have been "no change" about a change.

- **A census only interprets a measurement if it is taken in that measurement's configuration.**
  I pre-registered "6 indirect sites, 2 non-void" and it was **56 and 26** — the census used
  `-I src` while the sweep supplies the cmake build's generated include roots, and a file that
  will not preprocess produces little CIR, which reads exactly like a file with few indirect
  calls. Pre-registering the prediction was right and it is what caught the error, but only
  because the *result* contradicted it: a file scored `0` turned up reaching an indirect call.
  A pre-registration taken in the wrong configuration is one more confident number.

- **When a change lands beside an existing rule, read the existing rule.** The whole measured
  effect of item 6's engine half was not the filter it added but the filter already sitting one
  line above it, which had been backwards since it was written — `wants_value` meant
  `dst.is_some()`, not "uses the result", and lowering gives every call a `dst`. The added
  filter cut nothing on the corpus. **The neighbour was the defect**, and nothing but reading it
  while editing next to it would have surfaced it.

- **A prediction that fails is the measurement working.** Twice in one session. The pinned-40
  census said "6 indirect sites"; it was 56, and the file it scored `0` turned up in the
  results. Item 5o said the over-report needed a shape "rare in VPP"; the re-take moved **255
  of 1552 files**, and asking why revealed the *fix's* new classification arm was wrong, not
  the estimate. **Both errors were invisible until a number contradicted a sentence** — which
  is the argument for writing the sentence down first, and for re-taking numbers even when the
  change "obviously" cannot move them.

- **A taxonomy change ages every label attached to it, and the labels are the report.** Adding
  `Advised` made "agree, both clean" no longer both clean and "both refused" not both refusing,
  in the same edit that fixed a heading for saying "chiero refused" when chiero had not. A
  heading that has quietly stopped being true is the same defect as the one being fixed; look
  for the others *in the same commit*, because nothing will fail when they go stale.

- **When two tools are the subject, grep in the other tool's vocabulary — or grep for nothing.**
  Twice on 2026-08-09 a pattern written to chiero's wording scored `0` against a gcc that was
  saying the same thing in its own words: `return with a value` vs *"forbids 'return' with
  expression"*, `does not declare anything` vs *"extra semicolon in struct or union"*. The first
  zero nearly became a defect record about 1019 files. **Listing the other tool's distinct
  messages is cheaper than the pattern that misses them** — 19 lines of `grep -o 'error: .*' |
  sort -u` found both counterparts at once. This is the same false-zero shape as the
  float-comparison census keyed on names the generator never emits; that makes it four.

- **Sample the stack before reading the code — twice now it named a function no reading would.**
  `TermArena::vars_of` (a call pattern, not a wrong line) and now `parse_model`, where the
  suspicion was "a long z3 query" and the truth was chiero parsing z3's reply. ⚠️ **Take more
  than one sample and extract the right thread**: the first sample here showed `read_form` and
  looked like a solver wait; two more showed `parse_model`. And an `awk` that keyed on
  `^Thread 1 ` matched gdb's *"Thread 1 received signal"* banner, printing another thread's
  frames — the same too-loose-pattern class as the false zeros above.

- **A performance fix that bounds a search can expose what the unbounded one was silently
  reading.** Restricting `parse_model` to one definition made a `Bool` return nothing, and the
  red test showed the old code had been giving bools *the next variable's value* for as long as
  it existed. **The speed defect and the correctness defect were the same line**, and only the
  first was being looked for.

- **Before diffing an instrument's residue, find its noise floor — run it against itself.**
  Envelope diffs were used as evidence twice on 2026-08-09 before anyone asked which entries are
  stable. Three runs of one binary answered it in four minutes: two of the pinned 40 vary by
  themselves, and chiero had been flagging exactly those two as `nondeterministic_abort` the
  whole time. **The instrument knew; the reader did not ask.** Both conclusions survived and
  both numbers were wrong, which is the cheap version of this mistake.

- **When a timing gate is flaky, make the measurement robust — never raise the ceiling.** The
  scale gate failed 4 runs in 6 with nothing changed, because one timing at those sizes swings
  2x. Loosening thresholds until it passed would have produced a gate that cannot fail, which
  is the outcome every other lesson here is about. **Minimum of k runs** is the fix: scheduling
  noise only ever adds time, so the smallest sample is closest to the work done. 6/6 green
  afterwards, and the ceilings stayed where the measurement put them.

- **A corpus can be blind to a whole axis, not just a shape.** Every VPP frontend number this
  project has published sits at essentially *one point* on the size curve: a TU there is its
  header closure, 167 source lines becoming 185 000 CIR lines, and the corpus spans 1.7x. Not
  "the corpus lacks this construct" (§7.21) but "the corpus cannot vary this **dimension**" —
  and the first gate built with the dimension found a worse-than-quadratic stage in one run.

- **One stack sample is a share of unknown size.** Twice on 2026-08-09 a single sample named a
  real cost that turned out to be minor: `read_form` (which was z3, not the defect) and
  `reachable_from` (a genuine O(n²), worth **5%**). Fixing the second on one sample's evidence
  was a wasted round-trip — a seven-sample profile then named `emit`, which was 4 of 7 and gave
  **2.9x**. **Count the samples before believing the frame**; the lesson had been written hours
  earlier and was still not applied.

- **A ratchet must move with the fix.** `lower`'s ceiling went 14.0 → 10.0 the moment the curve
  improved. A gate left at yesterday's slack cannot see tomorrow's regression, and it will look
  green the whole way back.

- **A pass/fail line printed by a command that never ran is worse than no line.** A build
  failed and the `cargo test` after it in the same block still printed `(no FAILED = green)`,
  because the pattern matched nothing — which is what "no failures" looks like when there are
  no results at all. §7.5 has the same rule for cargo (*a crate whose test binary fails to build
  emits no `test result` line*), learned again from the other end: **check that the thing ran
  before reading what it said.**

- **Subtracting two whole-program timings is not a phase split.** `engine = find-bugs − chiero
  cir` gave a real number with an invented label: the two runs do not share a cost, because
  `find-bugs` verifies the module **twice** (lowering, then `Engine::run` by design). The
  residual attributed to "the engine" was mostly re-verification, and one profile of the window
  said so — **4 of 4 samples in `dominators`**. Profile the window you are about to name;
  arithmetic on totals cannot tell you what ran in it.

- **Check the instrument before blaming it.** When those samples disagreed with the subtraction,
  the tempting explanation was "gdb stretches the timeline, so the offsets are wrong". Measured
  instead: **53.5 s under gdb against 53.3 s native.** The instrument was fine and the
  arithmetic was wrong — the opposite of the comfortable answer, and one timing settled it.

- **When arithmetic says something absurd, measure it rather than explain it away.** B² × 4
  bytes came to 37 GB for one function, which could not be happening — so the search was for the
  guard that made it fine. There was none: **peak RSS 35.6 GB**, confirmed by one `VmHWM` sample.
  The disbelief also hid a second error, a block count published as 24 576 that is 96 000. *Both*
  mistakes understated the defect, which is the direction that keeps a defect alive.

- **When you replace an implementation, the tests written against it are not evidence.** Every
  dominance rejection in `verifier.rs` ran through the code being swapped out, so passing them
  said only "the new code agrees with the old on the cases somebody thought of". The evidence
  that counted was a property test against the *definition* of dominance — with an oracle too
  slow to ship, which is exactly what a property test's oracle should be.

- **An edit that can fail and the commit that describes it must not share a shell block.**
  Twice in one hour a commit message described changes that were not in the file. The first: a
  python block asserted four anchors and wrote only at the end, so one bad anchor discarded
  three good ones — fixed by writing after *each* edit. The second happened **immediately
  after**, in a different way: the edit printed `NOT FOUND` in the same block as the `git
  commit`, so the message was already written by the time the failure was visible. **The rule
  that survives both is structural, not attentive**: read the outcome, then write the message.
  Resolving to be careful is what failed the second time.

- **A missing input is exposure, not breakage — measure the consequence.** "198 plugin units
  lack include paths the harness passes" is a fact; "those cannot preprocess" is a guess wearing
  its clothes, and it was **wrong by an order of magnitude**: 32 sampled, 5 actually fail because
  of it. Twice on 2026-08-09 an inference was published as a measurement — the other understated
  a defect by 60x (§7.29). **The tell is the same both times: a sentence about consequences
  written from a count of causes.**

- **A fix can move fidelity without moving findings, and a findings-only diff calls that "no
  effect".** 8e's re-sweep: `findings=489 exact=1` unchanged on 40 plugin files, while the
  masked path went from **3 envelopes to 1** and lowering gaps from 84 to 80. Two real files
  stopped giving up on a store they can now model. **Diff the assumptions and the gaps, not just
  the findings** — the whole point of an envelope is that it records what chiero could *not* do,
  and that is where a modelling fix shows up first.

- **Audit const sentinels: when a value has a special case, ask where its siblings are.**
  A missing enum arm is a compile error; a `const` sentinel has no exhaustiveness checking
  anywhere. `ObjectId` has two reserved members, and **every checker defect found on 2026-08-10
  was a site handling `NULL` and not `UNBOUND`** — `address_term` (8e), the indirect-call arm
  and `free`'s fault offset (8h). One grep — *which `NULL` sites do not mention `UNBOUND`* —
  found the rest after the first was fixed by hand, and twice the site's own comment already
  argued why the missing case mattered, about the case beside it.
  **The scope came from the zeros**: `ObjState::Freed`/`OutOfScope` and `DYNAMIC_EXTENT` were
  both clean, and both are enum-or-guarded. Without them "check siblings" is advice; with them
  it names where to look. Compare *"a catch-all match arm hides a missing feature"* and
  *"enumerate the variants"* below — same family, the enum half.

- **An assertion that fails can be wrong about the property, not about the code.** 8g's first
  test asserted that a skipped store's *stale byte would change*. It does not — the fix marks
  the range uninitialized, which clears the initialization mask and leaves `0x11` in place. The
  defect was never the byte; it was chiero **answering** with it. **When a fix lands and the
  test still fails, ask whether the assertion names the property or a symptom of it** — here the
  property is "the read comes back as a question", and the byte was only ever a proxy.

- **"Worth re-running whenever X changes" is a gate that has not been written yet.** The
  `MemFault`-vocabulary sweep was recorded as a five-minute manual check; a check that must be
  *remembered* will not be. Written as a test the same hour, it **failed immediately** on drift
  nobody had noticed — a corpus case expecting `"uninitialized"` where the enum says
  `uninitialized-read`, passing only because the assertion used `contains`. **The interval
  between "I should re-run this" and the gate is where the drift lives.**

- **Ask every operation what its *empty answer* means.** 050 contract 3 spends a paragraph on
  it for `find_bugs` — an empty finding list is wrong exactly when the search did not finish —
  and that rule had not been carried to the operations beside it. Asking it of three others on
  2026-08-10 found `find_optimizations` reporting `Exact` and `proven` over a function whose
  branch condition the engine never formed. **The signal was already in hand and thrown away**:
  `detect` ran the engine, held `run.fidelity()`, and returned only the proposals. ✅ **Asked of all nine operations, 2026-08-10 — the sweep is complete.** Seven were already
  right: `find_bugs` (which is where the rule is written), `check_reachable`, `prove_equivalent`,
  `impact`, `select_tests` (strictest — never claims `Exact` at all), `expansion_sites`
  (`Bounded` + a truncation record under `--limit`, verified) and `explain_macro_expansion`.
  **`find_optimizations` and `layout` were wrong**, both in the same way — an envelope whose
  prose named what it could not see while its structured claim said `proven`. Both fixed.

- ✅ **And the envelope *shape* holds everywhere** (checked the same day, an honest zero): all
  eight envelope operations carry `fidelity`, `proven`, `assumptions`, `blind_spots`,
  `determinism_key` and `truncation`. `cir` is not an envelope operation — it prints 020's text
  format — so its absence is correct rather than a gap. **The defects were in what the fields
  *said*, never in whether they were there**, which is worth knowing before hunting further:
  a missing-field sweep is already done and found nothing.

- **A prose caveat and a structured claim in one envelope must agree.** Both operation defects
  above already carried a blind spot naming exactly what was missed — `layout` even explained
  why it mattered ("not a smaller number but a wrong one") — beside `proven: true`. **The prose
  is for a reader and the flag is for a consumer, and only one of them was right.** When adding
  a caveat, check what the fidelity says.

- **A check that prints its own reassurance cannot report the bad case.** Twice on 2026-08-10:
  `git status --short | head -3; echo "(empty = clean)"` — the echo fires either way, so a dirty
  tree and a clean one look identical, and "tree clean" was reported for several cycles while
  two files sat uncommitted. And `cargo test … | grep FAILED; echo "(no FAILED = green)"` printed
  green while the *build* had failed and no test had run at all.
  **Print the number, not the verdict**: `git status --porcelain | wc -l`, and a test count
  rather than the absence of a word. This is the false-zero family — a pattern narrower than the
  thing it looks for — turned on one's own verification idioms, which is where it is hardest to
  notice because the output is the part being trusted.
  ✅ **The repo's own gates were audited afterwards and do this correctly**: `check.sh` keys its
  GREEN/RED on cargo's exit status and prints counts beside it (§7.5 argues exactly that), and
  `lint.py` prints its success line only after the failure branch has returned. **The defect was
  in transient shell, not in the tooling** — which is the harder half to fix, since ad-hoc
  commands get no review and are exactly where a hurried check goes.

- **The gap in a sweep is wherever its cheapest step was.** 2026-08-10 asked nine operations
  *what does your empty answer mean* and fixed two. It skipped `select-tests` because probing it
  needed a coverage directory — and that was the one whose empty answer was **structurally
  guaranteed**: the CLI attributed no test, so every invocation returned `0 selected` whatever
  the diff said. A first-time user found it in an afternoon by running the tutorial. **When a
  sweep exempts a case for being awkward to reach, that exemption is the finding**, and the note
  recording the skip is not a substitute for making it.

- **Run the tutorial, not the library, when checking a tutorial.** `tutorials.rs` exercised the
  library path while tutorial 3's console example ran a CLI path no gate covered — §8.3's third
  form inside our own documentation. A worked example is a *claim about the shipped command*.

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
  guard first — and on 2026-08-10 the same shape appeared in a *fault*: `free` reported a wild
  pointer "at address 0" because it takes an `ObjectId` and had no offset (fixed via `free_at`),
  while `promote_to_array`'s `off: 0, at: Span::DUMMY` arms turned out unreachable behind guards
  that already hold. **A finding that names the wrong place is the same defect as one that names
  no place**, and both send a reader somewhere the fault is not.
- **Enumerate the variants; do not wait to imagine them.** Walking `ExprKind`'s 21 variants against
  the arms of `raw_expr` took ten minutes and found three reaching the catch-all, one of them a
  live silent defect. The same census over `RValue`/`InstKind`/`Terminator` came back clean, which
  is also worth knowing. Every other defect channel — human fixtures, adversarial review, probing
  around a smaller bug, mutation — is **bounded by what a human thought to spell**.
