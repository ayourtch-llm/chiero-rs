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
**candidate filter**.

### 3.1 The filter is a closure, not a conjunction

The obvious filter — functions matching `scope` **and** containing the acquisition — has
a demonstrated recall hole. In `vnet/interface_cli.c`, `show_hw_interfaces` and
`clear_hw_interfaces` are the registered handlers and each body is a single delegating
call; the acquisition and all the frees live in `show_or_clear_hw_interfaces`, which is
not itself registered. The handlers match `scope` but contain no acquisition; the helper
contains the acquisition but is out of scope. **Neither becomes a candidate, and tier 2
would have analysed it correctly had it been escalated.** The same shape appears in
`plugins/vrrp/vrrp_cli.c`, `vnet/classify/in_out_acl.c` and several unit-test drivers.

So the candidate set is the **transitive callee closure** of the scope-matching
functions, bounded by `max_candidate_depth` (default 3) and reported.

**A function excluded by the filter is exactly as unexamined as one that was never
escalated**, and must degrade the result identically. An earlier draft only counted
unescalated candidates toward `Bounded`, so a function filtered out before escalation was
invisible — the recipe would report "conforms" over a set it never looked at. Both counts
are reported and both force `Bounded`.

For the CLI recipe the closure is roughly 140 files out of 1552. Candidates found,
candidates filtered out, and candidates escalated all appear in the run summary.

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
    freed   -> freed  on `unformat_free($li)`     // idempotent here; see below
  }

  require on_all_paths { at return: state($li) != owned }

  fixture good "fixtures/cli_ok.c"
  fixture bad  "fixtures/cli_leak.c" expect 1 at "cli_leak.c:22"
}
```

Two things this example deliberately does **not** do, both of which an earlier draft got
wrong:

- **No `double_free` report on the second `unformat_free`.** `vppinfra/format.h`'s
  `unformat_free` ends with `clib_memset (i, 0, sizeof (i[0]))`, so a second call is a
  no-op. Shipping a rule that fires on safe code is how a rule suite gets switched off.
  Whether repeated release is a defect is a per-API fact, not a DSL default.
- **No `forbid use($li) when state($li) == freed` alongside that transition.** The
  release call is itself a use, so the two clauses would both match one event and produce
  two findings that [040 §4](040-defect-checkers.md)'s dedup key would not merge, their
  kinds differing. **A call that drives a transition is not additionally a `use`** — that
  is a language rule, stated here because the interaction is not obvious.

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
typestate block, for rules the DSL cannot express (§4.3 lists which, and it is a third of
the catalogue, not a rare case). A recipe that uses it must still supply fixtures.

### 4.2.1 What a tracked entity *is*

`$e`'s identity domain must be stated, because each plausible choice breaks a different
real rule:

- An **AST declaration** breaks aliasing (`p = line_input; unformat_free(p);` becomes a
  false leak) and misfires in `vnet/ip/punt.c`, where a stack-local is named `input` and
  the caller-owned parameter is `input__`.
- An **`ObjectId`** breaks the flagship rule outright: `line_input` points at a *stack
  local* (`unformat_input_t _line_input, *line_input = &_line_input;`, 366 occurrences
  tree-wide), and `unformat_free` frees a heap vector *inside* the struct. The object's
  `ObjState` never becomes `Freed`, so a typestate keyed on memory-model liveness sees
  nothing happen. It also cannot express pool slots, which are offsets within one vector.

So an entity is **`(ObjectId, byte range)`** — the object plus the sub-range the rule
tracks — with two explicit transition kinds for the cases pointers alone cannot follow:

```
alias        $b = $a          // a second name for the same (object, range)
reinterpret  $b = $a via <pattern>   // e.g. index round-trip through pool_elt_at_index
```

`reinterpret` is what lets `pool_get(P, e); i = e - P; … e2 = pool_elt_at_index(P, i)`
keep one identity across the pointer→integer→pointer round trip.

**A tracked entity migrates across reallocation.** [021 §4](021-memory-model.md) models
`realloc` as allocate-new, copy, free-old, which mints a *new* `ObjectId` — so a naive
`(ObjectId, range)` identity detaches the moment a vector grows. That is not hypothetical:
`vec_add1(name, 0)` to NUL-terminate immediately after `unformat "%s"` is a standard VPP
idiom (`fib_api.c`, `feature.c`, `af_packet_api.c`, `crypto.c`). The engine's realloc model
therefore carries the tracked entity to the successor object at the copied range, and a
contract pins it.

### 4.2.2 `on_all_paths` and abnormal termination

`require on_all_paths { at return: <cond> }` must say what happens on paths that never
reach a `return`, or the rule is either a flood or a hole:

| Termination | Treatment |
|---|---|
| Normal `return` | The obligation applies. |
| Budget (`max_loop_iters`, depth, state cap) | **Not** a violation, but forces `Bounded` and is counted. Treating it as violating would flood every input-driven loop. |
| `noreturn` (`clib_panic`, `os_panic`) | Excluded — the path does not return. |
| Infeasible | Excluded. |
| `Action::Kill` by another checker | Excluded, recorded. |
| `longjmp` | Unsupported ([024 §5](024-environment-models.md)); recorded assumption, `Unknown`. |

`<point>` is `return` (function return), `exit` (process termination), or
`call <pattern>`.

**A violation is reported once, at the acquisition site**, listing the offending return
paths as `sites` — mirroring [040 §4](040-defect-checkers.md)'s macro grouping, where one
fix addresses all of them. This also makes the finding location agree with §6's baseline
key `(recipe, file, entity)`: five per-branch findings would collapse to one baseline row,
so baselining four and un-fixing one would keep CI green.

### 4.3 What the DSL can and cannot express

An earlier draft advertised a nine-rule VPP catalogue. Attempting to write three of them
in the grammar above showed that **only one is expressible**, and the two that are not are
the two with the largest site counts. Recording that honestly, because a DSL advertised
against a catalogue it cannot express would be discovered at implementation time.

**Expressible today** — the *single-entity, single-function, acquire/release* shape:

| Rule | Sites |
|---|---|
| CLI `unformat_free` on every path (the §4 example) | 407 |
| `barrier_sync`/`_release` pairing | 112 |
| `VLIB_CLI_COMMAND` callback signature (tier 1) | 1187 |
| `VLIB_NODE_FN` returns `frame->n_vectors` (tier 1) | 568 |
| Forbidden raw `malloc`/`strcpy` (tier 1) | — |
| `unformat` format/argument agreement (tier 1) | — |

**Not expressible — and why**, measured against the real tree:

- **`clib_error_return` returned-or-freed** (3350 sites). Five separate gaps. There is no
  binder for a *call's result* — every pattern form binds in argument position — yet
  **1638 of 3350 sites are `return clib_error_return(...)` inline**, where the entity is
  an unnamed temporary. There is no `returned($e)` predicate, so the obligation
  "freed **or** returned" is unwritable, and `at return: state($e) != owned` would flag
  the commonest correct idiom in VPP. The sinks are assignment macros, not calls
  (`#define clib_error_free(e) e = clib_error_free_vector(e)`), which neither `via macro`
  nor `expanded` has a defined meaning for. `clib_error_return(e, …)` both consumes `e`
  and produces a new error, which needs a multi-entity transition. And six producer
  variants need alternation the grammar lacks.
- **`pool_get`/`pool_put` pairing** (635/548 sites). Measured: 94 functions contain both,
  320 only `pool_get`, 335 only `pool_put` — **77% of allocating functions never free**,
  because the canonical shape is `x_create_if`/`x_delete_if` in different functions.
  Every clause in §4.2 is function-scoped, so the rule as specified would emit ~320 false
  positives and find nothing. It also needs cross-representation identity: `pool_put`
  expands to pointer subtraction with no call to match, and the tracked entity round-trips
  through an integer index via `pool_elt_at_index`.
- **`vec_free` on CLI-local vectors.** The acquisition is the address of a *variadic*
  argument whose ownership depends on the format string's conversion specifier
  (`unformat (li, "filename %s", &f)`), and the grammar has no way to bind a metavariable
  in a variadic position conditioned on a `%` conversion.

**These three ship as Rust `Checker`s through §4.2's escape hatch**, not as recipes. That
is the honest allocation of work, and it is why contract 21 requires the escape hatch to
be exercised: it is not a rare fallback, it is where the hardest third of the catalogue
lives.

**Growing the DSL to cover them** needs, in rough order of value: return-value binding
and a `returned($e)` predicate; alternation over patterns; a `scope program` with
cross-function summaries; assignment patterns; multi-entity transitions. That is a
significant language design effort and is explicitly v2
([080](080-roadmap.md) records the sequencing).

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
actually holds across 1552 files is exactly what it is bad at. So the tool surface is —
note that chiero does **not** generate recipes, which would put it on the proposing side
of its own principle:

```
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
5. `cli_line_input_freed` on **`memif_socket_filename_create_command_fn`** reports zero
   violations, and on a copy with `unformat_free` deleted reports exactly one, at the
   acquisition, listing the offending return path.

   Named by *function*, not by file: `plugins/memif/cli.c` also contains
   `memif_create_command_fn`, which leaks `args.secret` on five error paths (lines 169,
   176, 179, 184, 186 — `memif_create_if` does `vec_dup`, so the caller retains
   ownership). Anchoring "correct code" to the file would bake a false assumption into
   the gate under the `vec_free` rule.
5b. That leak is a **positive fixture** for the `vec_free`-on-all-paths rule: five real
    findings in shipped code, which is the strongest available demonstration that the
    vertical works on something nobody hand-planted.
5c. **Each shipped rule is pinned to a golden set of real-VPP findings.** For
    `cli_line_input_freed` that set includes the 29 functions that acquire a `line_input`
    and never call `unformat_free` (`plugins/tracenode/cli.c`, four in `vnet/bfd/bfd_cli.c`,
    four in `vlib/log.c`, two in `plugins/quic/quic_cli.c`, and others). A rule that finds
    none of them passes every fixture-based contract while covering nothing.
5d. **Match-count baseline**: the number of tier-1 matches per recipe over the VPP corpus
    is recorded, and a drop beyond a threshold fails CI. This, not fixtures, is the
    anti-rot mechanism — fixtures pin the fixture. A new `pool_get_aligned_zero` variant
    or a renamed sink adds sites the fixture never contained, so the `bad` fixture keeps
    failing while the rule silently stops covering a third of the tree.
6. The same recipe on a variant where the missing-free path is provably unreachable
   (guarded by an always-false condition) reports **zero** — feasibility is honoured.
7. Tier 1 sweep over all of VPP completes within a documented time budget on 12 cores and
   reports candidate counts per recipe.
8. The candidate set is the **transitive callee closure** of the scope-matching functions
   (§3.1). Specifically, `show_hw_interfaces` in `vnet/interface_cli.c` — a registered
   handler whose body is one delegating call — yields `show_or_clear_hw_interfaces` as a
   candidate, and a leak planted there is found. Under the earlier
   scope-AND-acquisition filter neither function was a candidate and the leak was missed
   silently.
9. Both **unescalated** and **filtered-out** candidates force `Bounded`, are counted
   separately in the result, and prevent rendering as "conforms". A function excluded
   before escalation is exactly as unexamined as one excluded after it.
9b. `require on_all_paths` treats a budget-terminated path as non-violating but
    `Bounded`-forcing, and a `noreturn`-terminated path as excluded (§4.2.2) — verified
    with a fixture whose loop exceeds `max_loop_iters` and one that ends in `clib_panic`.
9c. **The realistic fidelity of a semantic recipe is recorded, not assumed.** Every CLI
    handler loops on `unformat_check_input` until symbolic input is exhausted, hitting
    `max_loop_iters`, and the acquisition itself is an indirect call into another TU —
    so `Exact` is not the normal outcome. The `Exact`/`Bounded`/`Approximated`
    distribution over the VPP candidate set is a tracked metric, like
    [041](041-optimization-analysis.md) contract 13b.
10. A recipe matching `pool_get` `via macro` matches invocations of the macro and does
    **not** match a function coincidentally named `pool_get`; with `expanded` the
    behaviour inverts. Both are pinned by fixtures.
11. A pattern naming `unformat_free` does not match a call to a local variable of that
    name (entity resolution, not spelling).
12. `require signature` over the 1187 `VLIB_CLI_COMMAND` registrations reports every
    callback whose prototype deviates, and zero for conforming ones.
13. `format_args_match` reports a `clib_error_return` whose format string and arguments
    disagree, and is silent on agreeing ones.
14. **Escape is split into discharge and unknown, and the recipe declares which is
    which.** An earlier single rule — "escaping exits tracking with an assumption" —
    made the `clib_error_return` rule impossible, because *being returned is the correct
    discharge there*, not an escape; treating it as one leaves `return err;` and a
    dropped `err` indistinguishable. So:
    (a) an escape the recipe names as a discharge (`returned($e)`,
    `escapes_to($e, <sink>)`) satisfies the obligation;
    (b) any other escape exits tracking with a recorded assumption.
    Pinned with the `plugins/tap` shape, where the same struct field is discharged by one
    caller (`cli.c` returns `args.error`) and leaked by another (`tapv2_api.c`).
14b. Storing into a **struct field** is an escape and is neither "global" nor "returned";
     it is the dominant transfer in VPP (56 sites). It is covered by (b) unless the
     recipe declares the field a sink.
15. A `chiero:allow` with no reason is itself exactly one violation.
15b. **Suppression scope**: `/* chiero:allow(rule) reason: … */` applies to the next
     *statement*; `chiero:allow-function(rule)` applies to the enclosing function. A
     suppression whose scope does not contain the finding does not suppress it and is
     reported stale — scope determines both false negatives and staleness, and "the next
     line" is not even well-defined for a finding located at the acquisition.
21. **The Rust escape hatch is exercised by the shipped catalogue**, not merely
    available: the three rules §4.3 lists as inexpressible ship as `Checker`s and pass
    the same fixture gate as recipes. A design where the escape hatch is where a third of
    the work lives must have that path tested.
22. Every construct in the grammar round-trips parse→print, and every syntax error
    carries a machine code and a span ([050 §4](050-tool-interface.md) promises
    structured errors, and `validate_recipe` is specified against this grammar).
23. A tracked entity survives reallocation: acquiring a vector, appending to it until it
    grows, and freeing it produces no "not freed" finding, and the identity reported is
    the successor object's.
24. A recipe finding names its `ConfigId` and march variant. `VLIB_CLI_COMMAND` is
    `#ifndef CLIB_MARCH_VARIANT`, so `registered_via VLIB_CLI_COMMAND` matches nothing
    under a march-variant configuration — the sweep must state which configurations it
    covers rather than silently sweeping one.
16. A `chiero:allow` matching no finding is reported as stale.
17. A baseline recorded on a violating tree makes CI green; introducing one new violation
    makes it red; moving an existing violation to a different line keeps it green.
18. `validate_recipe` on an LLM-proposed recipe that matches nothing fails on the `bad`
    fixture, and `apply_recipe` refuses to run it.
19. `grep -rE 'vlib_|clib_|unformat_|pool_get' crates/chiero-recipe/src` yields no hits;
    every VPP rule lives in `chiero-vpp`'s `.recipe` data.
20. Recipe evaluation is deterministic: findings and their order are byte-identical
    across runs and independent of file-system iteration order.
