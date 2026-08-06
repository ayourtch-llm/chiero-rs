# `find-bugs` on real VPP — the measurement, and how to retake it

`./measure.sh` runs `chiero find-bugs` over the 40 entry points in `entries.tsv` and prints one
line per function. It is checked in because the numbers below are the argument for what the
checkers are worth today, and a number nobody else can reproduce is an assertion.

It is **not** a test and nothing in CI runs it: it needs a VPP checkout, and 40 symbolic runs
take about four minutes. `xtask`'s gates are the things that must stay green; this is the thing
that says whether staying green is worth anything.

## The numbers, 2026-08-06

Release binary, `/home/ubuntu/vpp` at `7fe9c266`, 60-second cap per entry point.

| | findings | `Exact` | cut by the clock |
|---|---|---|---|
| `./measure.sh` | 21 | 0 | 2 |
| `./measure.sh --entry-ptr-nonnull` | **1** | 0 | 2 |

### The five statuses, and why `timeout` is no longer one of them

| | means |
|---|---|
| `ok` | chiero finished the search it set out to do |
| `cut` | **`--time-budget` stopped it and it printed what it had** — findings real, absence not |
| `timeout` | the process was killed from outside: something the engine's clock does not cover |
| `noinc` | a header this machine does not have (`xdp/xsk.h`, DPDK's tree) — not a chiero failure |
| `nofn` | the entry names no function in the module, so nothing was analysed |
| `failed` | chiero refused: the frontend error is now printed with its file and line |

`timeout` used to be the whole story and told none of it: a killed process prints nothing, so
a function chiero could not finish and a function with nothing to report produced the same
row. `--time-budget` (023 §8.1) makes the run end by its own decision with the envelope
intact; the harness keeps an outer `timeout` at `+30 s` so that the residue — a *single step*
that overruns, which is where the three remaining ones are — stays visible as a different word.

`nofn` exists because one of the pinned 40 was `VLIB_CLI_COMMAND`. `pick_entries.py` read
VPP's registration macros as function definitions, `find-bugs` answered "no function named
`VLIB_CLI_COMMAND`", and the harness counted that as a clean run. One entry in forty measured
nothing and reported `ok`; both ends are fixed, and the sample now has `show_node` in that
slot.

## The plugin sweep, 2026-08-06 — 477 entry points across 92 plugins

Not pinned, and reproducible from the checked-in scripts:

```sh
cd $VPP/src && ls plugins/*/*.c | grep -v '^plugins/acl/' > /tmp/files
python3 pick_entries.py --per-file 1 $(cat /tmp/files) > /tmp/plugins.tsv
LIST=/tmp/plugins.tsv TIMEOUT=20 ./measure.sh --entry-ptr-nonnull
```

| | |
|---|---|
| 477 entries, 92 plugins | `ok` 408, `cut` 20, `timeout` 3, `noinc` 35, `failed` 11 |
| findings | **18**, of which **1** is `Exact` |

**It found two source-triggerable panics**, which is what a sweep is for — both recorded as
`failed`, the same row a file that will not preprocess gets, so two crashes on real code looked
like two files chiero could not read:

- `perfmon_init` — an indirect call forked into a candidate of another signature, and comparing
  its one-byte result against a null pointer aborted the process. The candidate list was every
  defined function in the module, capped at 16, against a comment claiming it was every function
  *whose signature could be called here*.
- `send_vmxnet3_details` — `mp->admin_up_down = ... ? 1 : 0`, a `_Bool` field store, once a
  `strncpy` into the same struct had promoted the object. CIR types `_Bool` as `Int(1)` and the
  array-backed write path extracted bits 7..0 of a one-bit value.

**And the one `Exact` finding is true.** `vmxnet3_tx_comp_ring_advance_next` reaches
`comp_ring->gen ^= VMXNET3_TXCF_GEN`, and that macro is `(1 << 31)` — signed overflow, which
C11 6.5.7p4 leaves undefined. gcc says the same thing when asked:

```text
$ gcc -c -Wshift-overflow=2 shift.c
warning: result of '1 << 31' requires 33 bits to represent, but 'int' only has 32 bits
```

VPP writes it four times in `vmxnet3.h` alone. This is the first `Exact` on real VPP code that
survived being checked — the previous one, `_vec_update_len`, was a false proof (below).

**It started at 231, and one of them was a false `proven: true`.** The four defects that took
it to 1 were all found by looking at what was left after removing the previous class, which is
the argument for taking a measurement at all:

| | | findings after |
|---|---|---|
| the one `Exact` finding was wrong — `_vec_update_len` | fidelity capped for a bound chiero invented | 231 |
| 147 of 157 were about that same invented bound | not reported by default, **counted** in the envelope | 32 |
| an `extern` global read as uninitialized | 021 §6's "unknown *and* initialized", one object kind over | 27 |
| a bitfield through an entry pointer, then the same read as `symbolic-byte` | `read_bits_via`, then `read_bits_term` | 23 |

The false proof, in full, because it is the one worth remembering:

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

**The one that remains is the shape a finding should have.** `clib_time_init` divides by a
value the path allows to be zero, the envelope says `Unknown`, and it names what stands behind
it — inline asm, `__builtin_expect`, an opaque write, and 1496 accesses against the invented
bound that were not reported. A reader can decide what to do with that in one pass.

## What `entries.tsv` is

`file<TAB>function`, 40 entries: the first six functions defined in each of
`vppinfra/{bitmap,mem_dlmalloc,hash,vec,time}.c` and `vlib/{node_cli,counter}.c`, **sorted by
name** within each file, truncated to 40. Sorted rather than source order because upstream
moving a function would otherwise reshuffle the sample, and a sample that moves is not one.

Deliberately mechanical. Choosing the functions by hand — or worse, keeping the ones that
produced interesting findings — measures the chooser. These are simply the first ones in files
picked for being ordinary VPP infrastructure: allocation, vectors, hashing, formatting, CLI.

Regenerate with `python3 pick_entries.py > entries.tsv` if the file set ever changes. It takes
a file list too — `--per-file N` and paths under `$VPP/src` — which is how the plugin sweep
above was built. All-caps names at column 0 are skipped: in VPP those are registration macros,
never functions.
