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
