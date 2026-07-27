# 015 — Lowering (AST → CIR)

`chiero-lower` turns a typed, resolved AST ([014](014-semantics-and-types.md)) into CIR
([020](020-cir.md)). It is the last frontend stage and the one that fixes the *conventions*
the symbolic core is written against.

This document exists because of a sequencing hazard. [080](080-roadmap.md) builds the
entire symbolic core in M1 against **hand-written `.cir` fixtures**, before the frontend
exists. If lowering's conventions are not fixed first, those fixtures will encode idioms —
marker placement, block shape, line attribution — that real lowering then does not
produce, and M2 discovers a core that is subtly wrong for real C. Fixing the conventions
on paper is cheap; discovering the mismatch in M2 is not.

`chiero-lower` **contains no analyses** ([001 §2](001-architecture.md)). Everything below
is a local, syntax-directed translation.

## 1. Structure

One `Function` per definition, one `Block` per straight-line run. Blocks are created at:
a label, a branch target, immediately after a terminator, and at loop headers/latches.

**Every construct below lowers to a fixed shape.** Two lowerings of the same construct must
produce identical CIR, because golden `.cir` files are contracts
([020 §6](020-cir.md)) and the differential harness diffs them.

## 2. Expressions

**Evaluation order is left to right**, and that is normative
([020 §7](020-cir.md)): operands are emitted in source order, so side effects occur in
that order. Order-sensitivity is flagged per 020 §7.

**Every implicit conversion is already an explicit `Cast` node** in the typed AST
([014 §5](014-semantics-and-types.md)). Lowering never infers one; if it finds itself
needing to, that is a `chiero-sema` bug, not a lowering fix.

### 2.1 Short-circuit `&&` and `||`

The only expression form that creates control flow.

```
a && b   ==>   %t = alloca i32                     ; the C type of `a && b`
               <eval a>;  .seqpoint;  br a_nonzero, bb_rhs, bb_false
   bb_rhs:     <eval b>;  store i32 (b != 0) -> %t;  goto bb_join
   bb_false:   store i32 0 -> %t;                    goto bb_join
   bb_join:    %r = load i32, %t
```

**The slot is `Int(32)`, not `Int(1)`** — `a && b` has C type `int`, and the typed AST
carries no int-from-bool conversion node because the expression simply *is* `int`. A
`i1` slot would force lowering to invent a `ZExt` at every use, which §2 forbids two
paragraphs earlier and verifier rule 5 would reject. The stored value is 0 or 1.

An `alloca` rather than a phi, because CIR is not SSA ([020 §1.3](020-cir.md)). `||` is
the mirror image.

The `SeqPoint` after `a` goes at the **end of the entry block**, before the `br`. Leaving
its position free would let two conforming implementations emit different goldens, which
defeats the purpose of fixing shapes here.

`?:` uses the same shape with the slot typed as the *result* type. Three cases the
`&&` shape does not cover: a `void`-typed `?:` has no slot and both arms simply execute;
an aggregate-typed `?:` uses `CopyMem` into the slot, since CIR has no aggregate values
([020 §1.4](020-cir.md)); and GNU `a ?: b` evaluates `a` **once**, into the slot, then
branches on it.

This matters for coverage: `bb_rhs` exists precisely because `b` is conditionally
evaluated, and gcov counts it separately.

### 2.2 Compound assignment, increment, decrement

`x += e` evaluates the lvalue's address **once**, loads, operates, stores.
`x++` yields the old value; `++x` yields the new one. Both emit a `SeqPoint` after the
full expression.

### 2.3 Aggregates and bitfields

Struct/union assignment is one `CopyMem` of the layout size, never a field-by-field
sequence ([020](020-cir.md) contract 12). Bitfield access uses `LoadBits`/`StoreBits`
with the `BitRange` resolved by [014 §3](014-semantics-and-types.md) — lowering never
re-derives bit offsets, so there is exactly one place to be wrong.

### 2.4 Statement expressions `({ ... })`

217 VPP files use them. The block's statements lower normally into the enclosing block
sequence; the value of the last expression-statement is the result. No special CIR
construct is needed — this falls out of the unstructured CFG.

### 2.5 Calls

Arguments left to right, then the `Call`. Varargs use `VaStart`/`VaArg`/`VaEnd`
([020 §4.4.1](020-cir.md)). A call to a `noreturn` function is followed by
`Unreachable(AfterNoreturn)`.

## 3. Statements

| C | CIR |
|---|---|
| `if` | `Br` to then/else blocks, join block |
| `while`, `for` | header block (condition) → body → latch → back edge to header |
| `do…while` | body → latch (condition) → back edge |
| `switch` | `Switch` terminator; cases sorted and deduplicated by the verifier |
| `break` / `continue` | `Goto` to the enclosing loop's break/continue target |
| `goto` / labels | `Goto`; labels get their own block and a `Marker(Label)` |
| `return` | `Return`, after `Scope(Exit)` markers for every open scope |

Loops are **not** recovered as loops — the back edge is the only record, and
[023 §8](023-execution-engine.md) identifies it by dominator analysis. Lowering must
therefore emit a distinct header block even for `for(;;)`, so the back edge exists.

**Irreducible cycles** — formable with `goto` — have no dominator-identified back edge,
so `max_loop_iters` would silently never apply and only `max_depth` would stop the run,
losing the `BudgetHit` reporting entirely. The fallback is to bound each non-trivial
strongly connected component instead: entering an SCC more than `max_loop_iters` times
terminates the state with `Budget(IrreducibleCycle)`. Lowering does not detect this; it
is named here because §3's "the back edge is the only record" is what creates the gap.

`for` init runs in a scope enclosing the loop; a declaration in the init belongs to that
scope, not the body's.

## 4. Scopes and markers

Every compound statement gets a `ScopeId`. Lowering emits
`Marker(Scope { scope, Enter })` on **every edge entering the scope** and
`Scope { scope, Exit }` on **every exit path** — falling off the end, `break`,
`continue`, `goto` out, and `return`.

"Every entering edge", not "at the lexical top", because C gives automatic objects
storage on entry into the block *however entered* (C11 6.2.4p6), and two constructs
enter a scope by jumping into its middle:

- **`switch` with declarations in its body** — the `Switch` terminator jumps straight to a
  case label inside the compound, bypassing the lexical top. Since
  [021 §4](021-memory-model.md) creates stack objects on `Scope(Enter)`, the scope's
  allocas would never be materialized, the eventual `Scope(Exit)` would retire objects
  that never existed, and every access on the case path would be a wild access or a false
  use-after-scope. This is not exotic; it is any `switch` with a local.
- **`goto` into a scope**, which C permits.

Critical edges are split where needed so the marker has an edge to sit on. Re-entering an
already-entered scope (a backward `goto`) creates a *new* generation of its objects,
matching the loop-body rule in §3.

A missed marker in either direction is a false finding later, so this is the most
error-prone part of lowering and is pinned by contracts 9–11 and 9b–9c.

Allocas are declared in `Function::allocas` with their `scope` and
`Lifetime::Scope`; VLAs and `alloca()` use `AllocaDyn` at the point of declaration, with
`alloca()` carrying `Lifetime::Function` ([020 §3](020-cir.md)).

`SeqPoint` markers are emitted at each C sequence point: the end of a full expression,
after the first operand of `&&`/`||`/`?:`/`,`, and before a call's body.

## 5. `Block::gcov_lines` — who computes it, and how

Nothing else in the spec set owned this, and it is the join point of the entire
differentiating claim ([030](030-coverage-gcov.md) → [031](031-change-impact.md) →
[032](032-test-selection.md)). It is computed **here**, at lowering, because this is the
only stage that has both the AST spans and the CIR block structure.

The rule:

> For each `Inst` in the block, take its `Span`, resolve it with
> **`SourceMap::expansion_loc`** ([010 §3.1](010-source-and-provenance.md)), and collect
> the distinct `(file, line)` pairs. `gcov_lines` is that set, sorted ascending, keyed on
> the **defining file of the enclosing function** — which is where gcov attributes them.

Five consequences, each of which is a way to get it wrong:

- **`expansion_loc`, never `spelling_loc`.** A statement inside a macro body must be
  attributed to the `.c` line where the macro was *used*, because that is the only line
  gcov records ([030 §1](030-coverage-gcov.md), measured). Using `spelling_loc` yields
  header lines that appear in no coverage file and silently match nothing.
- **Lines in a header are kept, not dropped.** An earlier draft said to drop any line
  whose file is not "the block's own TU". That is backwards, and 030 §1's measurement is
  the proof: a `static inline` function in a header **does** get its own gcov file entry
  and line counts (`m.h:2 count 1`). Every instruction of a function defined in `vec.h`
  resolves to `vec.h`, so the drop rule would empty `gcov_lines` for all of them — and
  VPP's entire hot layer (`vec.h`, `pool.h`, `buffer_funcs.h`) is `static inline`
  functions. Coverage correlation would return ∅ for exactly the code that matters most,
  while [020](020-cir.md) contract 15 and [023](023-execution-engine.md) contract 21 both
  say otherwise. The drop rule also guarded a case that cannot arise: chiero does not
  inline ([023 §5](023-execution-engine.md)), so a block never contains another
  function's lines.
- **The file key is the enclosing function's defining file.** `gcov_lines` is a bare
  `SmallVec<u32>` with no `FileId`, so the file is implicit — and it must be the one gcov
  used, which for a header function is the header.
- **Compiler-generated instructions contribute nothing.** A block containing only
  lowering-introduced instructions (the `&&` join, an implicit `Scope(Exit)`) has an empty
  `gcov_lines`, and that is correct — gcov has no counter for it either.
  [020](020-cir.md) contract 15 requires "compiler-generated" to be a recorded property of
  the instruction, not a guess.
- **`simplify_cfg` unions the sets** when merging blocks ([020 §9](020-cir.md)), which is
  why the set is a set and not a single line.

**In hand-written `.cir` fixtures there is no `SourceMap`**, so this computation cannot
run: spans are optional and default to `Span::DUMMY` ([020 §6](020-cir.md)). The `.line`
directive in the textual format therefore **populates `Block::gcov_lines` directly**, and
that is its only meaning. Without this, M1's fixtures could not exercise
[030](030-coverage-gcov.md) contract 13 at all. `MarkerKind::Line` is the in-memory form
of the same thing.

`#line` needs no rule here. VPP contains none in-tree and `vppapigen` emits none
(verified), and gcov follows presumed locations anyway — so `expansion_loc`'s result is
already the right answer. Building machinery for it now would be speculative.

## 6. Initializers

Static initializers are evaluated by [014 §6](014-semantics-and-types.md) into
`GlobalInit`; lowering emits no code for them. Designated initializers (1019 VPP files)
are resolved to byte offsets there too.

Automatic aggregates with an initializer lower to `SetMem` zero followed by stores of the
explicitly-initialized members. The `SetMem` is not an optimization: C11 6.7.9p21
requires members with no initializer to be zero-initialized, so reading them is
well-defined and must **not** produce an uninitialized-read finding. An aggregate with
*no* initializer at all emits no `SetMem`, and its members stay uninitialized — those are
two different programs and the distinction is the whole point.

## 7. What lowering refuses

Constructs chiero parses but will not model become `Opaque`
([020 §4.3](020-cir.md)) with the reason recorded, or — where even that is not honest —
the function is skipped with a diagnostic and excluded from analysis, never silently
emitted as something else. Per [HANDOFF §4.12b] the current list is nested functions and
`__label__`. Inline `asm` is `Opaque` with declared outputs, not a skip.

## 8. Testable contracts

1. `a && b` lowers to the §2.1 shape: 4 blocks, one `alloca`, and `b`'s block is reachable
   only from the true edge of `a`'s test.
2. `a || b`, `a ? b : c`, and nested `a && b || c` each lower to a fixed shape that is
   byte-identical across two runs and matches a checked-in golden.
3. `f(g(), h())` emits the call to `g` before the call to `h` (left-to-right order).
4. `x += f()` evaluates `x`'s address once — exactly one `AddrOfLocal`/`PtrAdd` for it.
5. `x++` yields the pre-value and `++x` the post-value, verified by the differential
   oracle against gcc.
6. A 40-byte struct assignment emits one `CopyMem`, not 10 stores.
7. Bitfield assignment emits `StoreBits` with the `BitRange` from `RecordLayout`, and
   lowering computes no bit offset of its own (checked by construction: the layout is the
   only source).
8. A statement expression `({ int t = f(); t + 1; })` yields the value of the last
   expression statement and its side effects occur once.
9. **Every `Scope(Enter)` has a matching `Scope(Exit)` on every path that leaves the
   scope** — verified by a checker over the whole corpus, including exits via `break`,
   `continue`, `goto` out of the scope, and `return`.
9b. **`switch (x) { int y; case 1: y = 1; … }`** materializes the scope's objects on the
    case path: the jump to `case 1` carries a `Scope(Enter)`. Contracts 9–11 test exits
    only, so an implementation with this hole passes all of them.
9c. `goto` into a nested scope enters it exactly once, and a backward `goto` that
    re-enters creates a new generation of its objects.
10. `goto` out of two nested scopes emits two `Scope(Exit)` markers, innermost first.
11. `return` from inside three nested scopes emits three, before the `Return`.
12. `for (int i = 0; …)` puts `i` in a scope enclosing the body, and `i` is out of scope
    after the loop.
13. `for(;;)` still produces a distinct header block, so a back edge exists and
    [023 §8](023-execution-engine.md)'s dominator analysis finds it.
14. A VLA declaration emits `AllocaDyn` at the declaration point with the size operand
    dominating it; `alloca()` emits `AllocaDyn` with `Lifetime::Function`.
15. **`gcov_lines` for a block whose instructions all come from a macro body contains the
    expansion-site line in the `.c` file, and does not contain the macro's own line in the
    header.** This is §5's rule and the join point of the whole selection story; it is
    tested directly against `gcov --json-format` output for the same fixture.
16. A block containing only lowering-generated instructions has empty `gcov_lines`, and
    every such instruction is marked compiler-generated.
15b. **A `static inline` function defined in a header gets `gcov_lines` in the header**,
     and `tests_for_block` matches gcov's entry for that header. This is the case the
     earlier drop rule silently emptied, and it is most of vppinfra. Note that contract
     17's subset property is *vacuously satisfied* by the empty set, so it cannot detect
     this on its own — the two must be tested together.
17. Over the corpus, the union of all blocks' `gcov_lines` for a function is a **subset**
    of the lines gcov reports for that function, **and is non-empty for every function
    with at least one non-compiler-generated instruction** — chiero must neither claim a
    line gcov does not attribute there nor go silent on one it does.
18. `switch` with fallthrough lowers to blocks chained by `Goto`, and a `case` range of 4
    expands to 4 sorted cases.
19. An automatic aggregate with **no** initializer leaves its members detectable as
    uninitialized. An aggregate with a *partial* initializer does **not**: C11 6.7.9p21
    zero-initializes the rest, so `struct S s = {.a=1}; use(s.b);` reads a well-defined 0
    and must produce no finding. (An earlier version of this contract demanded the
    opposite, which both contradicts C and is unimplementable under §6's own
    `SetMem`-zero lowering, since [021](021-memory-model.md) contract 28 marks a `SetMem`
    range initialized.)
20. A function containing a nested function or `__label__` is skipped with exactly one
    diagnostic and is absent from the module, rather than lowered incorrectly.
21. Lowering the same TU twice produces byte-identical CIR.
22. For every corpus C file, `lower(parse(f))` printed as text equals the checked-in
    `.cir` golden — the round-trip that makes M1's hand-written fixtures and M2's real
    lowering the same language.
