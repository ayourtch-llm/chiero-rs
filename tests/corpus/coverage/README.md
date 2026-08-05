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
