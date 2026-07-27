# 031 — Change impact

`chiero-diff` answers: **given this change, what could behave differently?** Its output
is an `ImpactSet` of entities, each with a justification. [032](032-test-selection.md)
intersects that with coverage to pick tests.

This is the vertical where owning the preprocessor pays off. Coverage data cannot see
macro bodies ([030 §1](030-coverage-gcov.md), measured), so a coverage-only tool is blind
to any edit inside `vec.h`, `pool.h`, or any of VPP's 754 `foreach_*` X-macros. The
reverse expansion index makes those edits answerable.

## 1. Input and entity extraction

Input is a diff: a unified diff, a git revision range, or two source trees. Both sides
must be parseable configurations, because impact is computed by **comparing two parsed
programs**, not by reading diff hunks. Hunks only tell you which bytes moved; they cannot
tell you that moving a `}` changed which function a statement belongs to.

```rust
pub enum Entity {
    Function { file: FileId, name: Symbol, march: Option<Symbol> },  // static → file-scoped
    Macro    { file: FileId, name: Symbol, def_line: u32 },
    Record   { file: FileId, tag: Symbol },
    Global   { file: FileId, name: Symbol, linkage: Linkage },
    Typedef  { file: FileId, name: Symbol },
    EnumConst{ file: FileId, name: Symbol },
    BuildConfig(ConfigId),
}
```

Entity identity follows [014 §4](014-semantics-and-types.md): `static` functions are
file-scoped and never merged across TUs, and `march` distinguishes
`CLIB_MARCH_VARIANT` builds of one source ([060](060-vpp-integration.md)). Getting
either wrong produces impact sets that are confidently incomplete.

A **macro can be defined more than once** for one name across configurations, which is
why `Macro` carries `def_line`. `#undef`/redefinition is itself a change.

## 2. Change classification

Each entity present on either side is classified by comparing its **normalized token
stream plus resolved semantics**, not its text:

| Class | Trigger | Impact |
|---|---|---|
| `Unchanged` | identical tokens and identical resolution | none |
| `Cosmetic` | differs only in whitespace, comments, or line position | **none** |
| `BodyChanged` | statements/expressions differ | callers may differ |
| `SignatureChanged` | params, return type, variadicity, linkage | all callers |
| `LayoutChanged` | record size, alignment, or any field offset differs | everything touching the type |
| `MacroBodyChanged` | replacement list differs | **every expansion site** (§3.2) |
| `MacroInterfaceChanged` | parameter count/names, variadicity, object↔function | every expansion site |
| `InitializerChanged` | a global's initial value | readers |
| `Added` / `Removed` | | callers/users; removal is also a build break |
| `ConfigChanged` | a `#if` condition or `-D` flag changed | affected TUs re-derived |

**`Cosmetic` producing no impact is a real, load-bearing feature.** Reformatting a file,
adding comments, or shifting line numbers must not select every test in the repository,
and this is the most common kind of commit. It is also the easiest place to be wrong —
which is why the comparison is on normalized tokens with provenance, not on lines.

Line movement deserves emphasis: shifting a function down by 10 lines changes every
coverage line associated with it, so a naive line-based tool sees the whole file as
changed. Entity-based comparison sees `Cosmetic`.

The `LayoutChanged` test is a **computed comparison** of `RecordLayout`
([014 §3](014-semantics-and-types.md)), not a syntactic one. Reordering two same-size
fields changes offsets and is `LayoutChanged`; renaming a field is not layout-affecting
but *is* a source-compatibility change for its users; adding `__attribute__((packed))`
changes everything downstream. VPP's wire-format structs make this the highest-severity
class in the table.

## 3. Impact closure

```rust
pub struct ImpactSet {
    pub entities: IndexMap<Entity, Justification>,
    pub completeness: Completeness,     // Complete | Partial { reasons }
}

pub struct Justification { pub root: Entity, pub class: ChangeClass,
                           pub edges: Vec<ImpactEdge>, pub distance: u32 }

pub enum ImpactEdge {
    DirectlyChanged,
    ExpandsMacro { macro_: Entity, sites: Vec<Span> },
    Calls { site: Span }, CalledBy { site: Span },
    UsesType { site: Span }, ReadsGlobal { site: Span }, WritesGlobal { site: Span },
    IncludesHeader { header: FileId },
    SameConfig(ConfigId),
}
```

Every entity in the set carries the **path by which it was reached**. Auditability is a
requirement: a maintainer who is told to run 400 tests must be able to ask why, and get
"because `foo()` expands `vec_add1`, whose body you changed, at `ip4_forward.c:900`".

Closure proceeds to fixpoint over these relations:

### 3.1 Direct

Entities classified as changed in §2.

### 3.2 Macro closure — the differentiating step

For a changed macro `m`:

1. `SourceMap::expansion_sites(m)` yields every `ExpnCtx` where `m` was expanded,
   **including transitively** — expansions of macros whose bodies expand `m`
   ([010 §3.1](010-source-and-provenance.md)).
2. Each site's `expansion_loc` gives the enclosing function; those functions are impacted
   with `ExpandsMacro`.
3. The macro's *own* expansion sites in other macro bodies mark those macros changed too,
   and the closure repeats.

Worked case, and the reason this project exists: an edit to the body of `vec_add1` in
`vppinfra/vec.h` produces **no coverage delta anywhere** — gcov records only the `.c`
lines where it was used, and those lines did not change. Coverage-based selection sees
nothing to run. chiero enumerates all 1000+ expansion sites and impacts every enclosing
function.

The dual risk is honest: a change to a macro used in 900 files impacts 900 files, and
that is the correct answer. Precision comes later, from symbolic refinement in
[032 §4](032-test-selection.md), not from pretending the impact is smaller.

### 3.3 Type and layout closure

`LayoutChanged` propagates to every entity that declares, allocates, accesses a field of,
or computes `sizeof`/`offsetof` on the record — and transitively to records embedding it,
since embedding changes *their* layout. Field-level precision is used where available: a
change to field `f`'s offset impacts accessors of `f` and anything after it in the record,
but a pure rename impacts only `f`'s users.

### 3.4 Call graph

Transitive callers of a changed function, over the cross-TU call graph
([014 §4](014-semantics-and-types.md)). Indirect calls are handled by
**address-taken conservatism**: if a changed function's address is taken, every indirect
call site whose type signature is compatible is treated as a potential caller. VPP's node
dispatch is table-driven indirect calls, so this matters constantly; `chiero-vpp` narrows
it with knowledge of the registration tables, and the general engine does not guess.

Recursion and cycles are handled by fixpoint, and `distance` records the shortest path
for ranking.

### 3.5 Globals and configuration

Writers of a changed global impact its readers. A `ConfigChanged` invalidates every TU
compiled under the affected `ConfigId` — and because VPP's multiarch compiles one source
under several variants, a config change can impact a superset of the obvious files.

### 3.6 Include closure

A changed header impacts every TU that includes it, *transitively*. This is the crudest
relation and would swamp everything if applied naively — `vlib/vlib.h` reaches most of
the tree. So it is used only as a **backstop for entities chiero could not parse or
resolve**, never for entities it understood. If chiero parsed a header and determined
that only macro `m` changed, the answer is `m`'s expansion sites, not every includer.

## 4. Completeness

```rust
pub enum Completeness {
    Complete,
    Partial { unparsed_files: Vec<PathBuf>, unresolved_calls: u32,
              unknown_configs: Vec<ConfigId>, address_taken_fallbacks: u32 },
}
```

Impact analysis must be **over-approximate to be useful**: missing an impacted entity
means silently skipping the test that would have caught the regression. So every gap
widens the set rather than narrowing it:

- a file chiero cannot parse → all its entities impacted, and the include closure applied;
- an unresolvable indirect call → all signature-compatible targets;
- an unknown build configuration → all configurations.

`Partial` is reported prominently and is what makes [032](032-test-selection.md)'s
always-run set non-empty. A tool that quietly narrows here is worse than no tool: it
converts an unknown into a false assurance.

## 5. Output

Deterministic ordering (by entity kind, then file, then name), stable across runs. Both a
machine format (JSON, for [050](050-tool-interface.md)) and a human rendering that leads
with the closure reason:

```
CHANGED  macro vec_add1  (vppinfra/vec.h:120)  MacroBodyChanged
  ├─ 1043 expansion sites in 287 functions
  ├─ ip4_forward.c:900  ip4_rewrite_inline           ExpandsMacro
  └─ … 286 more (--verbose to list)
CHANGED  struct ip4_header_t  (ip/ip4_packet.h:44)  LayoutChanged (size 20→24)
  └─ 412 entities access this type
PARTIAL  3 files did not parse; their tests are in the always-run set
```

## 6. Testable contracts

1. Reformatting a file (whitespace only) yields an empty `ImpactSet`.
2. Adding a comment inside a function body yields an empty `ImpactSet`.
3. Moving a function 100 lines down without editing it yields an empty `ImpactSet` —
   despite every one of its coverage lines changing.
4. Editing one statement in one function impacts that function and its transitive
   callers, and nothing else.
5. **The headline contract**: editing the body of a macro defined in a header and used in
   N functions, with no `.c` file touched, yields an `ImpactSet` containing all N
   functions with `ExpandsMacro` justifications — and the coverage-only baseline for the
   same diff yields the empty set. Both are asserted in the same test, so the difference
   is the artifact.
6. A macro whose body expands a changed macro is itself marked changed, and its expansion
   sites are included (transitive closure), to a depth of at least 3 in the fixture.
7. Renaming a macro parameter without changing behaviour is still
   `MacroInterfaceChanged` — chiero does not attempt to prove macro equivalence here.
8. Changing a function's return type impacts all callers even if no body changed.
9. Reordering two same-size struct fields is `LayoutChanged`, and impacts accessors of
   both; renaming a field is not `LayoutChanged` and impacts only that field's users.
10. Adding `__attribute__((packed))` to a struct is `LayoutChanged` with the size delta
    reported.
11. Embedding a `LayoutChanged` record inside another record makes the outer record
    `LayoutChanged` too.
12. Two `static` functions with the same name in different files: changing one impacts
    only its own callers.
13. Two `CLIB_MARCH_VARIANT` builds: changing the source impacts both variants as
    distinct entities.
14. Taking the address of a changed function makes every signature-compatible indirect
    call site an impacted caller, and `address_taken_fallbacks` is incremented.
15. A file that fails to parse puts all of its entities in the set and sets `Partial`
    naming the file.
16. Changing a `#if` condition impacts every TU under the affected `ConfigId`.
17. Changing `vlib/vlib.h`'s *macro* `m` impacts `m`'s expansion sites, **not** every
    includer of `vlib.h` — the include closure is not applied to entities that resolved.
18. Deleting a function impacts its callers and is reported as `Removed`.
19. Every entity in the set has a non-empty `Justification` with a valid edge chain back
    to a directly-changed root, verified structurally for every entity in every fixture.
20. Impact of a diff against itself (empty diff) is the empty set.
21. Impact analysis is deterministic: identical input yields byte-identical output,
    including ordering.
22. Impact over a 500-file VPP diff completes within a documented time budget, and the
    entity count is reported so a reviewer can spot an explosion.
