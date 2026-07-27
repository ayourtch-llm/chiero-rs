# 042 — Conformance recipes

`chiero-recipe` checks that code **follows a prescribed usage pattern**, from declarative
rules supplied as data. Not "is this code buggy" ([040](040-defect-checkers.md)) but
"does this code perform the ritual the way this codebase requires".

Large C codebases are held together by rituals that the compiler cannot enforce and a
grep cannot verify. VPP is an extreme case, and the rituals are load-bearing.

## 1. The motivating ritual, measured

Counted over `/home/ubuntu/vpp/src` @ `7fe9c26`:

| Idiom | Files | Occurrences |
|---|---|---|
| `VLIB_CLI_COMMAND` registrations | 358 | **1187** |
| `clib_error_return` | 430 | **3350** |
| `unformat_line_input` (acquire) | 140 | 407 |
| `unformat_free` (release) | 196 | 537 |
| `pool_get` / `pool_put` | 345 / 290 | 635 / 548 |
| `vec_free` | 614 | 3034 |
| `vlib_worker_thread_barrier_release` | 54 | 112 |

Here is the CLI ritual as actually written, from `plugins/memif/cli.c`:

```c
if (!unformat_user (input, unformat_line_input, line_input))
  return 0;
socket_id = ~0;  socket_filename = 0;
while (unformat_check_input (line_input) != UNFORMAT_END_OF_INPUT)
  {
    if      (unformat (line_input, "id %u", &socket_id))          ;
    else if (unformat (line_input, "filename %s", &socket_filename)) ;
    else {
        vec_free (socket_filename);
        unformat_free (line_input);
        return clib_error_return (0, "unknown input `%U'", format_unformat_error, input);
    }
  }
unformat_free (line_input);
if (socket_id == 0 || socket_id == ~0) { vec_free (socket_filename);
                                         return clib_error_return (0, "Invalid socket id"); }
if (!socket_filename || *socket_filename == 0) { vec_free (socket_filename);
                                         return clib_error_return (0, "Invalid socket filename"); }
err = memif_socket_filename_add_del (1, socket_id, (char *) socket_filename);
vec_free (socket_filename);
return err;
```

`vec_free (socket_filename)` is hand-written on **five** separate return paths and
`unformat_free (line_input)` on two. This pattern is repeated across 140 files by hand,
with no mechanical enforcement anywhere. Missing one path is a leak that no test will
notice and no compiler warning will catch.

That is the shape of the problem: **the correctness condition is a property of every path
through a function, the ritual is prescribed rather than derivable, and the prescription
lives in maintainers' heads.**

## 2. Why this belongs in chiero and not in a linter

A syntactic linter (clang-tidy, coccinelle, grep) can approximate some of this. Four
things it structurally cannot do, and chiero can:

1. **Path sensitivity.** "Freed on every return path" is a statement about paths. The
   memif example has eight of them. A syntactic tool sees a `unformat_free` token and a
   `return` token and cannot relate them; the symbolic engine already enumerates paths
   and already knows which are feasible.
2. **Macro awareness.** The rituals *are* macros — `pool_get`, `vec_free`,
   `VLIB_CLI_COMMAND`. A tool operating on preprocessed text sees the expansion and has
   lost the name; a tool operating on unpreprocessed text cannot resolve types. chiero's
   `SourceMap` retains macro identity through expansion
   ([010 §3.1](010-source-and-provenance.md)), so a rule can say "matched `pool_get`"
   and mean it. This is the same capability that powers test selection, reused.
3. **Type and entity resolution.** "The `unformat_input_t *` that was acquired" is not a
   token — it is a resolved entity that may be copied, passed and returned. Recipes bind
   to entities from the typed AST ([014](014-semantics-and-types.md)), not to spellings.
4. **Feasibility.** A violating path that the solver proves unreachable is not a
   violation. This is the difference between a rule that gets adopted and one that gets
   turned off after the first false-positive wave.

## 3. Two-tier evaluation — the scalability story

Symbolically executing 1552 `.c` files is not going to happen. Pattern-matching them is
cheap. So every recipe runs in two tiers:

```
        ┌── tier 1: STRUCTURAL sweep ──┐        ┌── tier 2: SEMANTIC ──┐
whole   │ typed-AST pattern match      │ cand-  │ symbolic execution   │ findings
repo ──▶│ + provenance queries         │ idates │ over those functions │ ──▶
        │ O(AST), every function       │  ~140  │ O(paths), few fns    │
        └──────────────────────────────┘        └──────────────────────┘
```

Tier 1 alone answers structural recipes (signatures, forbidden APIs, argument
constraints) across the entire codebase in one pass. For a semantic recipe, tier 1 is a
**candidate filter**: only functions that match the recipe's `scope` and contain its
acquisition pattern are escalated. For the CLI recipe that is 140 files out of 1552.

Both counts — candidates found and candidates escalated — appear in the run summary. A
recipe that silently examined 3 of 140 candidates because it hit a budget must say so;
per [023 §7](023-execution-engine.md), an unescalated candidate makes the result
`Bounded`, and "conforms" is only reportable at `Exact`.

## 4. The recipe language

Recipes are data, loaded from `.recipe` files. The syntax is deliberately C-flavoured
because the people who know the rituals are C developers, not Rust developers or Datalog
users.

```
recipe cli_line_input_freed {
  title     "CLI line input must be freed on every path"
  severity  error
  tier      semantic
  rationale "unformat_line_input allocates a line_input; VPP's CLI ritual requires
             unformat_free on every return path. 407 acquisition sites, 140 files."

  scope fn $f where registered_via VLIB_CLI_COMMAND

  track $li typestate {
    state unowned initial
    state owned
    state freed

    unowned -> owned  on `unformat_user($_, unformat_line_input, $li)` returning nonzero
    owned   -> freed  on `unformat_free($li)`
    freed   -> freed  on `unformat_free($li)`  report double_free
  }

  require on_all_paths { at return: state($li) != owned }
  forbid  use($li) when state($li) == freed

  fixture good "fixtures/cli_ok.c"
  fixture bad  "fixtures/cli_leak.c" expect 1 at "cli_leak.c:22"
}
```

### 4.1 Patterns

Backtick-quoted C expression or declaration patterns with metavariables:

| Form | Meaning |
|---|---|
| `$name` | binds an entity (typed expression); consistent within a recipe |
| `$_` | wildcard, binds nothing |
| `...` | variadic tail |
| `` `f($a, $b)` `` | a call to the resolved entity `f`, not to anything spelled `f` |

Patterns match the **typed AST**, so `$li` binds a resolved object with a type, and a
pattern naming `unformat_free` will not match a local shadowing it.

Macro matching is explicit, because the default is a trap either way:

```
match `pool_get($p, $e)` via macro       // the macro invocation, by MacroId
match `...`               expanded       // the post-expansion form
match `...`               anywhere       // either
```

Default is `via macro` when the pattern head resolves to a known macro, `expanded`
otherwise. The chosen mode is printed in `--explain`.

### 4.2 Clause kinds

| Clause | Tier | Use |
|---|---|---|
| `scope fn $f where …` | 1 | Which functions the recipe applies to. Selectors: `registered_via <macro>`, `name matches <regex>`, `in_file <glob>`, `has_attribute <attr>`, `signature <sig>`, `calls <pattern>`. |
| `require signature` | 1 | Registered callbacks conform to the expected prototype. |
| `require`/`forbid <pattern>` | 1 | Presence or absence of a construct. |
| `require const_string($x)`, `format_args_match($fmt, ...)` | 1 | Argument constraints, using const-eval ([014 §6](014-semantics-and-types.md)). |
| `track $e typestate { … }` | 2 | A finite automaton per tracked entity, driven by execution events. |
| `require on_all_paths { at <point>: <cond> }` | 2 | Path-universal obligations. `<point>` ∈ `return`, `exit`, `call <pattern>`. |
| `require <a> before <b>` / `dominates` | 2 | Ordering. |
| `forbid use($e) when <cond>` | 2 | Use-after-state. |

An escape hatch exists: a recipe may name a Rust `Checker` implementation instead of a
typestate block, for rules the DSL cannot express. It is expected to be rare, and a
recipe that uses it must still supply fixtures.

### 4.3 The recipe catalogue this enables for VPP

Shipped as data in `chiero-vpp`, not as code: CLI line-input freeing; `vec_free` on all
paths for CLI-local vectors; `pool_get`/`pool_put` pairing; `clib_error_return` results
either returned or freed (3350 sites); `VLIB_CLI_COMMAND` callback signature conformance
(1187 sites); barrier sync/release pairing (112 sites, overlapping
[025 §3](025-concurrency-and-threading.md)); `VLIB_NODE_FN` returning
`frame->n_vectors`; forbidden raw `malloc`/`strcpy` in favour of `clib_mem_alloc`/
`clib_memcpy`; `unformat` format-string/argument agreement.

## 5. Fixtures are mandatory

**A recipe that does not ship a passing `good` fixture and a failing `bad` fixture with
the expected finding location fails to load.** Not a warning — a load error.

This is the rule that keeps a recipe corpus alive. Rules rot: an API changes, a recipe
starts matching nothing, and the suite reports zero violations forever while everyone
believes it is working. A `good` fixture that starts failing catches over-matching; a
`bad` fixture that stops failing catches the far more dangerous under-matching. The cost
is two small C files per rule, which is the right price.

Fixtures are also the LLM-authoring contract (§7): a proposed recipe is not accepted
until its own fixtures adjudicate it.

## 6. Findings, suppression and adoption

Findings are ordinary `Finding`s ([023 §9](023-execution-engine.md)) with the recipe name
as the kind, a witness where tier 2 produced one, and the macro backtrace.

Adopting a rule on a 1M-line codebase with existing violations needs an on-ramp:

- **Baseline** — `chiero recipe baseline` records current violations by
  `(recipe, file, entity)`, deliberately *not* by line number, so unrelated edits do not
  invalidate it. CI gates on **new** violations only.
- **Suppression** — `/* chiero:allow(cli_line_input_freed) reason: <text> */`. The reason
  is **required**; an empty or missing reason is itself a violation. A suppression that
  matches nothing is reported as stale, so they get cleaned up.

Deliberately absent: a global severity downgrade knob. A rule that is not worth fixing is
not worth running, and the honest action is to delete the recipe.

## 7. LLM authoring loop

This vertical is the clearest instance of the project's core principle
([050](050-tool-interface.md)): **the LLM proposes, chiero adjudicates.**

Describing a ritual in prose is exactly what an LLM is good at, and verifying that a rule
actually holds across 1552 files is exactly what it is bad at. So the tool surface is:

```
propose_recipe(description, examples)  -> recipe text + generated fixtures
validate_recipe(recipe)                -> fixture results, load errors
apply_recipe(recipe, scope)            -> findings + candidate/escalation counts
```

`validate_recipe` runs the fixtures *first*; a recipe that fails its own fixtures is
never applied to the codebase. The failure mode this prevents — an LLM confidently
generating a rule that matches nothing and reporting "codebase is compliant" — is the
single most likely way this feature could do harm.

## 8. Placement

`chiero-recipe` is a vertical. It depends on `chiero-sema` (typed AST, for tier 1),
`chiero-span` (provenance queries), and `chiero-cir`/`chiero-exec` (tier 2). Nothing
depends on it except surfaces.

**No recipe content lives in `chiero-recipe`.** The crate is the engine and the language;
every VPP rule is a `.recipe` file shipped by `chiero-vpp`, and a user embedding chiero
ships their own directory. This is [001 §4](001-architecture.md) rule 4 applied to what
would otherwise be the most tempting place to hard-code VPP knowledge.

## 9. Testable contracts

1. A recipe with no `good` fixture fails to load with a diagnostic naming the recipe.
2. A recipe whose `good` fixture produces a finding fails to load (over-matching).
3. A recipe whose `bad` fixture produces no finding, or a finding at the wrong location,
   fails to load (under-matching).
4. The whole shipped recipe catalogue passes its own fixtures in CI; this is a gate.
5. `cli_line_input_freed` on the `plugins/memif/cli.c` ritual reports **zero** violations
   (it is correct code), and on a copy with `unformat_free` deleted from the `else` branch
   reports exactly one, at that branch, with a witness path reaching it.
6. The same recipe on a variant where the missing-free path is provably unreachable
   (guarded by an always-false condition) reports **zero** — feasibility is honoured.
7. Tier 1 sweep over all of VPP completes within a documented time budget on 12 cores and
   reports candidate counts per recipe.
8. For `cli_line_input_freed`, the candidate set is exactly the functions containing an
   `unformat_line_input` acquisition, and tier 2 runs on those and no others.
9. If tier 2 escalation is cut by budget, the result is `Bounded`, names the number of
   unescalated candidates, and cannot be rendered as "conforms".
10. A recipe matching `pool_get` `via macro` matches invocations of the macro and does
    **not** match a function coincidentally named `pool_get`; with `expanded` the
    behaviour inverts. Both are pinned by fixtures.
11. A pattern naming `unformat_free` does not match a call to a local variable of that
    name (entity resolution, not spelling).
12. `require signature` over the 1187 `VLIB_CLI_COMMAND` registrations reports every
    callback whose prototype deviates, and zero for conforming ones.
13. `format_args_match` reports a `clib_error_return` whose format string and arguments
    disagree, and is silent on agreeing ones.
14. A typestate entity that escapes the function (stored into a global, returned) exits
    tracking with a recorded assumption rather than a spurious "not freed" finding.
15. A `chiero:allow` with no reason is itself exactly one violation.
16. A `chiero:allow` matching no finding is reported as stale.
17. A baseline recorded on a violating tree makes CI green; introducing one new violation
    makes it red; moving an existing violation to a different line keeps it green.
18. `validate_recipe` on an LLM-proposed recipe that matches nothing fails on the `bad`
    fixture, and `apply_recipe` refuses to run it.
19. `grep -rE 'vlib_|clib_|unformat_|pool_get' crates/chiero-recipe/src` yields no hits;
    every VPP rule lives in `chiero-vpp`'s `.recipe` data.
20. Recipe evaluation is deterministic: findings and their order are byte-identical
    across runs and independent of file-system iteration order.
