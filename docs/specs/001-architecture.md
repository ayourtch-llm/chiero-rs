# 001 — Architecture

## 1. Crate graph

Every box is a crate under `crates/`. Arrows are `Cargo.toml` dependencies.

```
                            ┌─────────────┐
                            │ chiero-span │  spans, SourceMap, expansion tree
                            └──────┬──────┘
                                   │
        ┌──────────────────────────┼───────────────────────────┐
        │                          │                           │
 ┌──────▼──────┐            ┌──────▼──────┐             ┌──────▼──────┐
 │ chiero-lex  │───────────▶│  chiero-pp  │────────────▶│ chiero-ast  │
 └─────────────┘  pp-tokens └─────────────┘  pp-tokens  └──────┬──────┘
                                                               │
                                                        ┌──────▼──────┐
                                                        │chiero-parse │
                                                        └──────┬──────┘
                                                        ┌──────▼──────┐
                                                        │ chiero-sema │  types, layout, names
                                                        └──────┬──────┘
                                                        ┌──────▼──────┐
                                                        │chiero-lower │  AST+sema ──▶ CIR
                                                        └──────┬──────┘
 ═══════════════════════════════════════════════════════════════╪═══════ CONTRACT BOUNDARY
                                                        ┌──────▼──────┐
                                                        │ chiero-cir  │  the IR (no frontend dep)
                                                        └──────┬──────┘
                    ┌───────────────┬───────────────────┬──────┴────────┐
             ┌──────▼──────┐ ┌──────▼──────┐     ┌──────▼──────┐ ┌──────▼──────┐
             │chiero-solver│ │ chiero-mem  │     │chiero-model │ │ chiero-exec │
             └──────┬──────┘ └──────┬──────┘     └──────┬──────┘ └──────┬──────┘
                    └───────────────┴───────────────────┴────────────────┘
                                                        │
        ┌──────────┬──────────┬────────────┬────────────┼────────────┐
 ┌──────▼─────┐┌───▼──────┐┌──▼─────────┐┌─▼──────────┐┌▼──────────┐ │
 │chiero-gcov ││chiero-diff││chiero-select││chiero-check││chiero-opt│ │
 └──────┬─────┘└───┬──────┘└──┬─────────┘└─┬──────────┘└┬──────────┘ │
        └──────────┴──────────┴────────────┴────────────┴────────────┘
                                   │
                    ┌──────────────┼──────────────┐
             ┌──────▼──────┐┌──────▼──────┐┌──────▼──────┐
             │ chiero-vpp  ││ chiero-tool ││ chiero-cli  │
             └─────────────┘└─────────────┘└─────────────┘
```

## 2. Crate responsibilities

### Foundation

| Crate | Owns | Must not |
|---|---|---|
| `chiero-span` | `BytePos`, `Span`, `ExpnCtx`, `SourceMap`, `Expansion`, `Diagnostic` | Know what a token is |

`chiero-span` is depended on by literally everything and depends on nothing. It must
stay small and stable; a change here recompiles the world.

### Frontend

| Crate | Owns | Must not |
|---|---|---|
| `chiero-lex` | Translation phases 1–3, pp-token stream | Expand macros, resolve includes |
| `chiero-pp` | Phase 4: macro table, expansion, `#if` eval, include resolution, `_Pragma` | Build an AST, know C's grammar beyond `#if` expressions |
| `chiero-ast` | AST node types, arena, visitor | Parse |
| `chiero-parse` | Phases 5–7 grammar → AST | Resolve types (beyond the typedef disambiguation it is forced to do) |
| `chiero-sema` | Type system, name resolution, layout/ABI, constant evaluation, typedef feedback to the parser | Lower to CIR |
| `chiero-lower` | AST + sema → CIR | Contain analyses |

Splitting `chiero-lower` out of `chiero-sema` keeps `chiero-cir` free of any frontend
dependency, which is what makes §3 possible.

### Symbolic core

| Crate | Owns | Must not |
|---|---|---|
| `chiero-cir` | IR types, builder, verifier, text format (parse + print) | Depend on any frontend crate |
| `chiero-solver` | `Solver` trait, term language, `solver-lite`, SMT-LIB2 backend, `TieredSolver`, caches | Know about C, or about CIR |
| `chiero-mem` | `MemObject`, addressing, lazy initialization, byte-level read/write | Drive execution |
| `chiero-model` | Environment model registry: libc, `__builtin_*`, hooks for target-specific models | Hard-code VPP knowledge |
| `chiero-exec` | `State`, forking, `Searcher`, `Checker` hook points, path-explosion budgets | Implement specific checkers |

`chiero-solver` knowing nothing about C is deliberate: it should be independently
useful, and it makes its own test suite pure constraint-solving.

### Verticals

| Crate | Owns |
|---|---|
| `chiero-gcov` | `.gcno`/`.gcda` decoding, `gcov --json-format` ingest, coverage index, per-test attribution |
| `chiero-diff` | Diff parsing, byte-range → entity mapping, impact closure |
| `chiero-select` | Coverage ∩ impact, symbolic refinement, ranking, justification records |
| `chiero-check` | Concrete `Checker` implementations, report types, `chiero-replay` harness emission |
| `chiero-opt` | Optimization opportunity detection, `prove_equivalent`, proof obligations, cache-line/locality analysis |
| `chiero-recipe` | The conformance-recipe language, loader, fixture harness, and two-tier evaluator ([042](042-conformance-recipes.md)). Ships **no recipes** — VPP's live in `chiero-vpp` as data |

### Surfaces

| Crate | Owns |
|---|---|
| `chiero-vpp` | `compile_commands.json` ingest, vppinfra models, multiarch handling, `foreach_*` idiom support, and the VPP `.recipe` catalogue |
| `chiero-tool` | MCP server + JSON-RPC: the LLM-facing surface |
| `chiero-cli` | The `chiero` binary |

All VPP-specific knowledge lives in exactly one crate. If VPP knowledge leaks into
`chiero-model` or `chiero-check`, the "reusable" requirement has been violated.

## 3. The CIR contract boundary

**`chiero-cir` does not depend on any frontend crate.** This is the single most
important structural rule in the project, because it produces a property the chosen
build order depends on:

> The entire symbolic core — memory model, solver, execution engine, checkers — can be
> built, tested and reviewed against **hand-written CIR** before a single line of C is
> parsed.

To make that practical, `chiero-cir` ships a **textual CIR format** with both a printer
and a parser (see [020 §6](020-cir.md)). Core tests are then `.cir` files in
`tests/corpus/`, readable and diffable, with no dependency on frontend maturity. When
the frontend lands, `chiero-lower` is validated by round-tripping: lower C → CIR, print,
compare against the checked-in `.cir` golden.

Practical consequence for the TDD loop: milestone 1 touches `chiero-cir`,
`chiero-solver`, `chiero-mem` and `chiero-exec` only. A frontend bug cannot make a core
test red, and vice versa.

## 4. Dependency rules

1. **No cycles.** Enforced in CI by `cargo-deny`-style check in `xtask`.
2. **No crate depends on a vertical.** Verticals depend on the core; the core never
   reaches upward.
3. **`chiero-cir` never depends on `chiero-ast`, `chiero-parse`, `chiero-sema`, or
   `chiero-lower`.** Checked mechanically (§7 contract 3).
4. **VPP-specific knowledge lives only in `chiero-vpp`.** Other crates expose the
   extension points it plugs into.
5. **`chiero-span` depends on nothing** outside the standard library and small utility
   crates.
6. Verticals do not depend on each other except: `chiero-select` → `chiero-gcov`,
   `chiero-diff`; `chiero-opt` → `chiero-check` (for report types); `chiero-recipe` →
   `chiero-check` (for report types).
7. `chiero-recipe` and `chiero-diff` may depend on frontend crates (they need the typed
   AST); no other vertical may. The core still may not.

## 5. Cross-cutting conventions

**Errors.** Each crate defines its own error enum; no `anyhow` below the surface crates.
`chiero-cli` and `chiero-tool` may use `anyhow`. Recoverable parse/pp errors are
*diagnostics*, not `Err` — the frontend must keep going and produce a partial AST, since
a 1M-line codebase will always have something chiero cannot yet handle.

**Diagnostics.** One type, `chiero_span::Diagnostic`, carrying `Span`, severity,
message, and optional notes. Rendering (including macro-expansion backtraces) lives in
`chiero-span`.

**Arenas and IDs.** Interned IDs (`FileId`, `MacroId`, `ExpnCtx`, `ValueId`, `BlockId`,
`ObjectId`) are newtypes over `u32`, `Copy`, and dense. No `Rc<RefCell<…>>` graphs.

**Determinism is mandatory.** Identical input must produce byte-identical output,
including diagnostic and path-exploration order. No `HashMap` iteration in any output
path — use `IndexMap`/`BTreeMap`. This is what makes golden tests and the differential
harness viable, and it is a hard requirement, not a preference.

**Feature flags.** Default features are minimal and pure-Rust. Anything requiring an
external binary or FFI (`smtlib-subprocess`, `z3-sys`) is opt-in.

**MSRV.** Rust 1.97. Edition 2024.

## 6. Workspace layout

```
chiero-rs/
├── Cargo.toml              [workspace] with a shared [workspace.dependencies]
├── crates/
│   ├── chiero-span/ chiero-lex/ chiero-pp/ chiero-ast/ chiero-parse/
│   ├── chiero-sema/ chiero-lower/ chiero-cir/ chiero-solver/ chiero-mem/
│   ├── chiero-model/ chiero-exec/ chiero-gcov/ chiero-diff/ chiero-select/
│   ├── chiero-check/ chiero-opt/ chiero-recipe/ chiero-vpp/ chiero-tool/ chiero-cli/
├── tests/corpus/           shared C and .cir fixtures
├── tests/differential/     gcc-vs-chiero harness
└── xtask/                  dependency-rule checks, corpus regen, benchmarks
```

Version numbers are unified across the workspace and bumped together.

## 7. Testable contracts

1. `cargo metadata` yields a dependency graph with no cycles.
2. No crate in `crates/` other than `chiero-{gcov,diff,select,check,opt,recipe,vpp,tool,cli}`
   depends on a vertical crate.
3. `chiero-cir`'s transitive dependency set contains none of `chiero-ast`,
   `chiero-parse`, `chiero-sema`, `chiero-lower`.
4. `chiero-span`'s transitive dependency set contains no other `chiero-*` crate.
5. Grepping `crates/` excluding `chiero-vpp/` for `vec_add1|vlib_|vnet_|clib_` yields no
   hits outside comments and test fixtures.
6. Building the workspace with `--no-default-features` succeeds and links no external
   solver.
7. Running any analysis twice on identical input produces byte-identical stdout.
8. `xtask check-deps` exits non-zero when a rule in §4 is violated (verified by a
   fixture that deliberately violates one).
