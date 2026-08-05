# The macro-attribution fixture (030 §1, contract 2)

**Regenerated and re-verified 2026-08-05 against gcc 13.3.0**, on the machine 030 §1 was
measured on:

```
gcc --coverage -O0 t.c -o t && ./t && gcov -b --json-format t
```

`t.gcov.json.gz` is gcov's own output, byte for byte. `t.gcov.json` is the same document
pretty-printed, so a reviewer can read it and a diff can show what changed; the tests read the
gzip, because locating `<object stem>.gcov.json.gz` is contract 3 and the stem rule is a thing
gcc gets to decide, not this repo.

## What it records, and why the whole project depends on it

```
FILE t.c   line 3 count 1      <- ADD1 is expanded *twice* here
FILE m.h   line 2 count 1      <- the `static inline`, attributed to its definition
           line 1   (absent)   <- the macro body. Not zero. No entry at all.
```

Coverage follows the **expansion site** for a macro and the **definition site** for a function,
including an inline one. So "which tests cover this line of `vec.h`" is answerable for
`hdr_fn` and unanswerable for `ADD1` — from coverage data alone, at any level of
post-processing. VPP has 754 distinct `foreach_*` X-macros and a `vec.h`/`pool.h` layer whose
hot logic *is* macros.

That gap is what chiero's own preprocessor exists to close, so contract 2 pins the fact: if a
future gcc starts recording `m.h:1`, this fixture fails loudly and the justification for the
hand-written frontend has to be re-argued rather than quietly assumed.

## The pairing and version fixtures (contracts 8 and 9)

`other.c` is a **second, unrelated compilation**, committed with its own `.gcno`/`.gcda`. Its
stamp differs from `t`'s — `9b8924d1` against `1f830cd1` — because gcc derives the stamp per
compilation, so pairing `t.gcno` with `other.gcda` is a *genuinely* stale pairing rather than a
mutated one. That is the single most common source of nonsense coverage in a build tree nobody
cleaned, and contract 8 requires it rejected with both stamps named.

`badversion.gcno` **is** a mutation, and the only one here: `other.gcno` with its version word
replaced by `*99Z`, a tag gcc has never written. Everything else is byte-identical, so a decoder
that ignored the version would read it happily — which is what contract 9 forbids. Regenerate it
with the four-line script in the commit that added it.

## The line-count rule (contract 5's prerequisite)

`loop.c` exists to answer one question the `t` fixture cannot: **when several blocks are
attributed to one line, what count does gcov report for it?**

```c
int f(int n){ int s=0; for(int i=0;i<n;i++) s+=i; return s; }   /* all of it on line 1 */
```

`f`'s seven blocks solve to counts `0 0 1 4 5 1 1`, and the five blocks carrying line 1 have
counts **[1, 4, 5, 1, 1]**. gcov reports `line 1 count 5`.

So the rule is **the maximum**, and the fixture refutes the two other readings outright: the sum
is 12 and the first block's count is 1. `t.c`'s three blocks are all 1, so it cannot tell the
three apart — which is exactly why this second fixture is here.

## Two functions with one name (030 §5's `FuncKey`)

`a.c` and `b.c` each define `static int helper(int)`, at **different lines** and with
**different counts** — a.c's runs once, b.c's twice. `m.c` calls into both.

```
prog-a   a.c   helper @1  count 1
prog-b   b.c   helper @3  count 2
```

Keying coverage by function *name* merges them, and the merge is silent: one file's tests get
attributed to another file's code. That is the misattribution `FuncKey` exists to prevent, and
this is the smallest fixture that catches it — the counts differ, so a collision cannot look like
agreement.

It also demonstrates the stem rule a second time, in the form a real build produces: compiling
and linking in one step names the artifacts after the *output*, so the stems here are `prog-a`
and `prog-b`, not `a` and `b`. Asking for `a` yields a JSON document with an empty `files` array
rather than an error — which is exactly the silent nothing contract 3 is about.

## A function that never ran (the negative counter length)

Found by pointing the decoder at a real `--coverage` build of `vppinfra`: **83 of 98 `.gcda`
files failed**, all of them claiming a record length near 2³². Read as `i32` those are negative:
−40, −168, −304.

```c
int never_called(int x){ if (x > 3) return x * 2; return x + 1; }   /* line 1, count 0 */
int ran(int x){ return x + 1; }
int main(void){ return ran(1) == 2 ? 0 : 1; }
```

`unrun.gcda` holds three counter records: `16`, `8`, and **`-16`**. The negative one belongs to
`never_called`, and it has **no payload at all** — the next record begins immediately after the
length. Measured against the notes: `|len| / 8` equals the function's non-tree arc count exactly
(`bihash_all_vector.c` gave −40 for 5 arcs and −32 for 4).

So a negative length means *"this many counters, every one of them zero, none of them stored"* —
gcc's compression for a function that never executed. gcov agrees: line 1 has count **0**, which
is recorded-and-zero, not absent.

None of the earlier fixtures could find this: in all of them, every function ran.

## One block, two files (the `LINES` record's file groups)

Also found at scale, in the same `vppinfra` build. A `LINES` record is a block followed by a
*stream* of file groups, and one block can carry lines from **several** files:

```text
block 5  FILE mem.h  191   FILE bihash_all_vector.c  16   END
```

`inl.h` holds an `always_inline` function called twice from `inl.c`, which is enough to produce
it at `-O0`:

```text
block 2  FILE inl.c 2   FILE inl.h 3 4   END
block 4  FILE inl.c 2   FILE inl.h 3 4   END
```

A decoder that keeps one file per record attributes every line to whichever group came last —
`mem.h:191` becomes `bihash_all_vector.c:191`, a line that may not even exist. That is worse than
dropping it: a wrong file and line is a coverage answer about the wrong code.

None of `t`, `loop`, `unrun` or `prog-a`/`prog-b` has a multi-file block, which is why this
needed a real tree to surface.

## The line-rule fixtures — `cyc`, `nonmono`, `multi`, `group`, `omp`, `samelin`, `xline`

Seven fixtures added while making the native decoder agree with gcov exactly. Each exists to
refute one specific wrong reading, and each was built *after* a plausible implementation had
already passed everything else — so the list doubles as the record of how a rule that fits can
still be wrong.

| fixture | built with | refutes |
|---|---|---|
| `cyc.c` | `gcc --coverage -O0` | that a line's count is the entry arcs into its blocks. A whole `for` loop on line 5: entry-only says 1, gcov says 5. |
| `nonmono.c` | `gcc --coverage -O0` | that a block belongs to the *last* line of its group. Two force-inlined calls on line 21 make the group read `[21, 10, 11, 12, 13]`; gcov sorts first, so the block belongs to 21. |
| `multi.c`, `multi.h` | `gcc --coverage -O0` | that a source's lines are accounted per function. `bump` is inlined into both `one` and `two`; gcov reports 2, taking the maximum reports 1. |
| `group.c` | `gcc --coverage -O0` | that functions sharing a start line share a line table. `two` fits on line 1 and graphs it, `one` starts there and accumulates; one table lets the graph count erase the accumulation. |
| `omp.c` | `gcc --coverage -fopenmp -O0` | that every function in the notes is counted. The outlined parallel region is `artificial` and carries the source's own lines; gcov erases it. |
| `samelin.c`, `samelin.h` | `gcc --coverage -O0` | that an empty location group carries nothing. It carries the fact that there *was* a group, and gcov attributes the block a second time because of it. |
| `xline.c` | `gcc --coverage -O0` | that a file name is a string. `#line` gives two functions one line of `gen.c` under two spellings; gcov canonicalizes, so they are one file and a group. |

Every one is checked against `<stem>.gcov.json.gz` produced by `gcov -b --json-format <stem>.c`,
committed beside it. `cargo run --release -p chiero-gcov --example scale -- tests/corpus/coverage`
compares the whole directory in one go.

⚠️ **Four of these took two attempts.** The obvious shape passes under both readings: a group
whose members agree, an inlined block whose lines happen to be ascending, two blocks of equal
weight where the sum and the graph answer coincide. If a fixture for a rule of this kind passes
the moment it is written, suspect the fixture.
