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

**Retaken 2026-08-07 after `BadRange` left the defect list: byte-identical.** `ok=38 cut=2
findings=21 exact=0`. That is the correct answer and it took a second run to know it — see
below, because "the numbers did not move" is the one result a summary line cannot explain.

### Why the numbers did not move, and what that says about the corpus

`BadRange` — "unsupported-access-width", a 32-byte access chiero cannot carry — moved from
being reported to being a degradation on 2026-08-07. The expectation was that some of the 21
would go. None did, and the reason is not that the change did nothing:

**`KEEP` says the string `unsupported-access-width` appears nowhere in all forty envelopes** —
not in a finding, not in an assumption, zero occurrences. The pinned 40 never produce a
32-byte access at all, so the corpus is **blind to this change** rather than unaffected by it.

Why it is blind, measured rather than guessed:

| | |
|---|---|
| every 32-byte type in VPP lives in `vppinfra/vector_avx2.h` | `vector.h:197` includes it under `#if defined (__AVX2__)` |
| `__AVX2__` needs `-march=x86-64-v3` or `-mavx2` | `gcc -dM -E` defines it at `v3`, **not** at `v2` and not with no `-march` |
| VPP's baseline build is `-march=x86-64-v2` | from `ninja -t commands`; the AVX2 paths are compiled only in *multiarch variants* |
| chiero's `frontend::predefines` probes gcc with **no `-march`** | so it does not see them either |

So `BadRange` is currently **unreachable on VPP through any harness chiero has**, and the fix
is preventative — correct, and not measured on real code. ⚠️ Whoever reaches for these numbers
to argue a checker's worth should read that as the corpus's edge (HANDOFF §8.3), not as a
clean bill: 021 §5's note that "vppinfra uses `u8x32`/`u8x64` throughout" is true of code this
measurement has never once compiled.

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
intact; the harness keeps an outer `timeout` at `+30 s` so that the residue stays visible as a
different word rather than as silence.

That outer limit earned its keep on 2026-08-07 and then emptied: the residue it was catching
turned out not to be an engine step at all but a super-quadratic `verify::dominators`, running
before execution where no engine budget could reach it. **Zero `timeout` rows now.** Keep the
word and keep the outer limit — its value is that it makes "chiero did not finish" a different
answer from "chiero found nothing", and this is the second time that distinction has led
straight to a real defect.

⚠️ **"Zero" is true of *that* corpus only.** The widened sweep below, three entries per file,
has **three** — all in `plugins/nsh/` (`format_nsh_header`, `nsh_md2_decap`, `nsh_md2_encap`).
The verifier fix removed the cause the old rows had, so these are a *different* one and nobody
has looked yet. Sampling the stack under `gdb` is what found the last one; §11.2 has the
invocation.

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
| **retaken 2026-08-07** | `ok` **410**, `cut` 21, `timeout` **0**, `noinc` 35, `failed` 11 |
| findings | **18**, of which **1** is `Exact` |

**The `timeout` rows are gone, and finding out why they existed is the interesting part.** They
were `plugins/unittest/fib_test.c` and `llist_test.c` — **named here for the first time**, which
is the point: the 2026-08-06 numbers recorded a *count* of 3 and no rows, so nobody could act on
them. `KEEP` and this table are the fix for that.

023 §8 said they were "a single long solver query… for exactly this reason", and
`max_solver_rlimit` was specified as the bound for them. **Neither `--solver-rlimit` nor
`--time-budget` moved them at any value.** A stack sample said why: the time was in
`chiero_cir::verify::dominators`, which runs before a single instruction executes — no clock, no
solver, so no engine budget could ever have reached it. The verifier was super-quadratic in the
block count (11.5 s for 3001 blocks, release); fixed, and both entries now finish.

⚠️ **`ok` went 408 → 410 and `findings` did not move.** Two functions that had been measuring
nothing now measure something, and that something is *no defects*. A row that was `timeout` was
never evidence about the code.

## The widened plugin sweep, 2026-08-08 — **three** entries per file, 1320 entry points

`pick_entries.py --per-file 3` instead of `--per-file 1`. §8.3's pattern: widen one dimension,
read the residue before fixing anything.

| | |
|---|---|
| 1320 entries | `ok` 1133, `cut` 57, `timeout` 3, `noinc` 96, `failed` 31 |
| findings | **91**, of which **3** are `Exact` |

2.8x the entries, **5x the findings** (91 against 18). The three `Exact` are two halves of one
VPP case (`test_builtins.c`'s `handle_get_64bytes`/`4kbytes`) and
`vmxnet3_tx_comp_ring_advance_next`, which is the known-true `1 << 31` signed overflow above.

**The yield was a reporting defect, not a checker one.** The `test_builtins` pair came back
`proven: true`, `Exact`, resting entirely on `tb_main` being at its zero-initialised value — a
premise the envelope never stated. `find_bugs` now records `globals_at_initial_value` naming the
global. See the commit; the finding is true C and the *silence* was the bug.

### What the 31 `failed` rows are, all six causes

| count | cause | |
|---|---|---|
| 16 | `clib_crc32c_with_init` not declared | the **parked** `-march` item |
| 3 | `u32x4_sum_elts` not declared | same family — a vector intrinsic behind a target macro |
| 6 | unknown type `vl_api_http_static_*_t` | generated API headers |
| 3 | `vl_msg_api_set_handlers` not declared | generated API |
| 3 | **no member named `last_heard_age`** | ⚠️ **not a chiero defect — see below** |

The 96 `noinc` rows are headers this machine does not have (cbor, libxdp, picotls, netmap's
generated enum, libnl, DPDK, quicly) and are correctly not counted as chiero failures.

### ⚠️ The VPP build directory is **stale**, and that is worth knowing before trusting any number

`src/plugins/lldp/lldp.api` declares `f64 last_heard_age;` and was modified at **23:32:08** on
2026-08-05. The generated `lldp.api_types.h` it is compiled against was produced at
**23:14:37** — seventeen minutes *earlier*. The field does not appear in **any** header under
`build-root`.

So chiero is right, and gcc says so in the same words at the same line:

```text
src/plugins/lldp/lldp_api.c:135:12: error: 'vl_api_lldp_details_t' has no member named
  'last_heard_age'; did you mean 'last_heard'?
```

📌 **The consequence is bigger than three rows.** Every measurement in this file analyses the
source tree against *those* generated headers, so wherever `src/` has moved on, chiero is
reading a slightly different program from the one VPP would build today. Regenerating the build
directory is the fix; until then, a `failed` row naming a missing struct member is an
environment fact and should not be chased as a frontend gap.

## `vnet/` — a different subsystem, 2026-08-08, 423 entry points

The find-bugs corpus had only ever seen `vnet/ip/` (§7.17, an honest zero). This is the whole
of `vnet/*/*.c` at one entry per file — §11.3's "change the kind, not the size".

| | |
|---|---|
| 423 entries | `ok` 406, `cut` 7, `nofn` 3, `failed` 7, **`timeout` 0**, `noinc` 0 |
| findings | **44**, of which **0** are `Exact` |

A far cleaner subsystem than the plugins: no exotic external headers, so nothing is `noinc`, and
nothing timed out. Findings per entry run about three times the plugin rate. The four kinds are
`pointer-outside-object` 19, `out-of-bounds` 17, `null-dereference` 4, `uninitialized-read` 4.

### ⚠️ The sweep analyses files VPP does not compile, and two of them are broken

Six causes across the 7 `failed` rows, and **two are files the build never touches**:

| | |
|---|---|
| `fib_entry_src_default.c` | defines `fib_entry_src_default_deinit` **twice** (lines 22 and 35). gcc: `error: redefinition of …` at the same line |
| `pcap2pg.c` | calls `pcap_read` with no declaration in scope. gcc: `implicit declaration` — a *warning* in its default mode, and invalid C99+, which VPP's `-Werror` would reject |

Neither appears in `ninja -t commands all`'s 2945 entries, nor in `src/vnet/CMakeLists.txt`. They
are dead source that has never compiled, which is exactly why the defects survived — and chiero
is right about both.

✅ **Fixed the same day: `pick_entries.py --built-only`.** `failed` was mixing two unrelated
things — what chiero cannot read, and what *nothing* can. The flag reads `ninja -t commands all`,
keeps only files the build passes to a compiler with `-c`, and **prints how many it dropped**:

```text
$ python3 pick_entries.py --built-only --per-file 1 $(cat /tmp/vnetfiles) > /tmp/vnet.tsv
pick_entries: --built-only kept 420 of 427 file(s); 7 are not compiled by this build
```

Both defect files above are among those seven, with `interface_types_api.c`, `mma_template.c`,
`sr_test.c`, `tcp_cc.c` and `pcap2cinit.c`. The pinned `entries.tsv` is byte-identical without
the flag, so the default sample is untouched.

⚠️ It **refuses** rather than falling back when the build directory is missing: quietly keeping
everything would turn an absent `$VPPBUILD` into a sweep measuring the wrong corpus, which is the
failure the option exists to end. And it reads `-c <source>` rather than a constructed path,
because object paths cannot be derived from source paths under CMake object libraries — the trap
`probe.sh` documents.

⚠️ **The numbers in this section were taken *without* it**, so their 7 `failed` rows include the
two uncompilable files. Re-run with `--built-only` before comparing against anything later.

The other five: two `-march`-family intrinsics (`u32x4_gather`, `clib_crc32c_u32`, the parked
item), a generated API type, and an unresolved `api_sr_localsid_add_del_v2`.

⚠️ `nofn` 3 — `pick_entries.py` is still picking macro-registered names as functions
(`clear_session_dbg_clock_cycles_fn`, `create_simulated_srp_interfaces`). Same cause as the
`VLIB_CLI_COMMAND` row that created the `nofn` status; it was fixed for that shape and not this
one.

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
