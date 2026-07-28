# M2 frontend brief — `chiero-lex` and `chiero-pp`

You are building the **lexer** and **preprocessor** for chiero-rs, a symbolic C execution
library in Rust. The rest of the project (M1: CIR, memory model, solver, execution engine)
is being built in parallel in a different worktree by another agent. The two are
deliberately independent — `docs/specs/080-roadmap.md` says so — and this brief exists to
keep them that way.

## Where you work

**This worktree only: `/home/ubuntu/rust/chiero-m2`, branch `m2-frontend`.**

Never touch `/home/ubuntu/rust/chiero-rs` — that is the other agent's working tree on
`main`. It is a *different directory* for the same repository; writing there will collide
with work in progress. If you find yourself typing that path, stop.

## Scope — what you may change

| Path | You may |
|---|---|
| `crates/chiero-lex/**` | own it entirely; it is a 1-line stub today |
| `crates/chiero-pp/**` | own it entirely; it is a 1-line stub today |
| `M2-NOTES.md` (create it) | your running notes, findings, and open questions |

**Do not modify anything else.** In particular:

- `docs/specs/**` is **normative and read-only**. If you believe a spec is wrong,
  write it in `M2-NOTES.md` with the evidence. Spec changes are made by the other agent as
  `spec:` commits after judging the claim; a spec that quietly follows the code is how the
  contracts stop meaning anything.
- `crates/chiero-span/**` is **shared and already complete** (851 lines, 43 tests). You
  *call* it — `SourceMap::add_file`, `add_macro`, `add_macro_at`, `add_expansion`,
  `expansion_backtrace`, `lookup_loc`, `span_text` — you do not change it. If you need a
  method it does not have, write the signature you want in `M2-NOTES.md` and work around
  it meanwhile.
- Every other crate belongs to M1. `chiero-cir` in particular is the contract boundary:
  you do not lower to CIR in this brief.
- `HANDOFF.md`, `Cargo.toml`, `clippy.toml`, `xtask/**` — leave alone. Every crate is
  already declared in the workspace.

## What to build, in order

**1. `chiero-lex` — `docs/specs/011-lexer.md`.** All 14 numbered contracts.

**2. `chiero-pp` — `docs/specs/012-preprocessor.md`.** All 19 numbered contracts.

Read `docs/specs/010-source-and-provenance.md` first: `Span`/`ExpnCtx` are load-bearing
from the first token, and 080 says provenance "is implemented first and never retrofitted
— adding them later means touching every line of the frontend". Every token carries a
`Span`. Every macro expansion calls `SourceMap::add_expansion`, so that a bug inside a
macro can name both the expansion site and the macro body. This is not decoration: it is
what the whole test-selection feature (032) is built on.

The two locked project decisions that bear on you: **chiero has its own preprocessor and
its own parser — not clang-backed.** Do not add a dependency on libclang, tree-sitter, or
any external C parser. Pure Rust, no new third-party dependencies without asking.

## How to work — this is not optional

`docs/specs/070-testing-and-tdd-protocol.md` is the protocol. Concretely:

1. **Red first.** Write the failing test and commit it, message starting `test: RED — `.
   The commit body says which contract it is and why the case matters.
2. **Then green.** Implement, commit with `feat:` or `fix:`.
3. **Every test names its contract.** A test file header says
   `//! Covers: 011 contracts 3, 4, 7` — there is a coverage gate (`cargo xtask
   contract-coverage`) that scans for exactly that, and M1's exit criterion is every
   numbered contract of the spec being cited by a test.
4. **Comments cite the spec sentence they implement**, especially where the code looks
   surprising. The house style is that a comment explains *why this and not the obvious
   alternative*, quoting the spec where it settles it.
5. **Mutation-test your own work before you call it done.** Break the code you just wrote
   in a small semantically-meaningful way and check a test fails. If none does, the test
   is decoration. This project has caught roughly a dozen tests that passed for the wrong
   reason; the standing trap is a **fixture that gives the same answer under both the
   correct and the mutated code**, so check that the fixture actually reaches the code
   under test and that the assertion would differ.

## The oracle you have and M1 does not

012's exit gate is: **the preprocessor matches `gcc -E` and `clang -E` on the corpus**
(token stream, normalized). Both are installed. Use them as a differential oracle from
early on rather than at the end — it is a far stronger oracle than fixtures, and M1 has
nothing like it. `gcc --version` here is 13.3.

`docs/specs/060-vpp-integration.md` and the VPP tree at `/home/ubuntu/vpp` are the real
target. `vppinfra` headers (`vec.h`, `pool.h`, `bitmap.h`, `clib.h`) are M2's exit gate,
and they are macro-saturated on purpose — do not tune the preprocessor to toy inputs.

## Gates that must pass before any commit

```
cargo fmt --all
cargo test --workspace          # 675 tests today; none of them may break
cargo clippy --workspace --all-targets    # zero warnings, not "few"
cargo xtask check-deps          # crate layering
cargo xtask contract-coverage   # your contracts should appear here
```

Use `ulimit -v 6291456` before cargo commands; the machine has other work on it.

**`HashMap`/`HashSet` are banned** (`clippy.toml` enforces it): iteration order is not
deterministic and 001 §5 makes determinism a hard requirement. Use `IndexMap` or
`BTreeMap`.

## Reporting back

Keep `M2-NOTES.md` current with: what is done, what each contract's status is, anything
you found wrong in a spec, and any `chiero-span` method you wished existed. Commit it with
your work. The other agent reviews this branch — adversarially, with mutation testing —
before it merges to `main`, the same way it reviews its own work.

If you get stuck on something that needs a decision outside this scope, write it in
`M2-NOTES.md` and move on to the next contract rather than guessing at a spec change.
