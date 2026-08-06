# `find-bugs` on real VPP — the measurement, and how to retake it

`./measure.sh` runs `chiero find-bugs` over the 40 entry points in `entries.tsv` and prints one
line per function. It is checked in because the numbers below are the argument for what the
checkers are worth today, and a number nobody else can reproduce is an assertion.

It is **not** a test and nothing in CI runs it: it needs a VPP checkout, and 40 symbolic runs
take about four minutes. `xtask`'s gates are the things that must stay green; this is the thing
that says whether staying green is worth anything.

## The numbers, 2026-08-06

Release binary, `/home/ubuntu/vpp` at `7fe9c266`, 60-second cap per entry point.

| | findings | `Exact` | timed out |
|---|---|---|---|
| `./measure.sh` | 231 | 0 | 3 |
| `./measure.sh --entry-ptr-nonnull` | 157 | 0 | 2 |

**Read the `Exact` column first, and know that it used to say 1.** That one finding was a false
positive that claimed `proven: true` — chiero's strongest claim, on real code, wrong:

```text
_vec_update_len:
  out-of-bounds: 4-byte access at offset -8 of the 4096-byte object reached through an
  unconstrained pointer
  proven — this holds for all inputs (Exact)
```

The access is `_vec_find (v)->len = n_elts`, and `_vec_find(v)` is `((vec_header_t *) (v) - 1)`.
**Every VPP vector is an interior pointer by design** — the header lives behind the data, which
021 has a worked example of. Two chiero inventions produced it: the object behind an entry
pointer parameter is `ENTRY_PARAM_BYTES` = 4096 bytes, and the pointer is placed at *offset 0*
of it. The finding's own wording carries the contradiction: a pointer cannot be both
"unconstrained" and known to sit at the base of a 4096-byte object.

Fixed at the rule rather than at the site — a bounds fault against an `ObjKind::Lazy` object
degrades to `Approximated` and names the size chiero chose.

**The 157 that remain are the open problem.** Every one is `Unknown`, and nearly every one says
"…of the 4096-byte object reached through an unconstrained pointer": a statement about the
*caller contract*, not about the function. Nobody can act on those. See HANDOFF §7.6 and §9.

## What `entries.tsv` is

`file<TAB>function`, 40 entries: the first six functions defined in each of
`vppinfra/{bitmap,mem_dlmalloc,hash,vec,time}.c` and `vlib/{node_cli,counter}.c`, **sorted by
name** within each file, truncated to 40. Sorted rather than source order because upstream
moving a function would otherwise reshuffle the sample, and a sample that moves is not one.

Deliberately mechanical. Choosing the functions by hand — or worse, keeping the ones that
produced interesting findings — measures the chooser. These are simply the first ones in files
picked for being ordinary VPP infrastructure: allocation, vectors, hashing, formatting, CLI.

Regenerate with `python3 pick_entries.py > entries.tsv` if the file set ever changes.
