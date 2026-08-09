# HANDOFF archive — closed queue items

Items closed out of [HANDOFF.md](HANDOFF.md) §9.1. **Moved, not deleted**: several of these
carry the reasoning that produced a defect, and this project keeps the reasoning even when the
work is done — §11.3, *the residue of a gate is a corpus*. What they no longer do is cost a
fresh context 800 lines of reading before it can pick up live work.

Numbers are the §9.1 tags they closed under; those tags renumber as items close, so treat them
as labels rather than as an order.

1. ✅ **UNPARKED AND CLOSED 2026-08-08 — the owner said "go ahead and design + execute the persona
   work", then "feel free to tackle march".** Both shipped; the original entry is kept below
   because its reasoning was right and is worth reading.

   **What was built.** A [`chiero_pp::Persona`] is a *named* set of predefines, and its file format
   **is `cc -dM -E` output** — so there is no new parser, no new dependency, and one is captured
   with `gcc -dM -E -x c /dev/null > personas/name.h` from any real compiler on any target.
   `Persona::baked()` names the set chiero always had; `Config::persona` replaces it. `--march` and
   any `-m…` flag go to the probe verbatim, since only the compiler knows what each implies.

       chiero cir <file>                    -> always
       chiero cir --march x86-64-v2 <file>  -> has_sse42, always
       chiero cir --march x86-64-v3 <file>  -> has_sse42, has_avx2, always

   **The two mechanisms are one.** `frontend::predefines` hand-parsed `-dM` into a `Vec` while the
   library baked 23 entries; both now go through `Persona::from_defines`, and the CLI only knows
   how to run a compiler.

   ⚠️ **Three bugs the gate caught and I would not have**: `add_predefined_object` wrapped values
   in a single synthetic *number* token — fine for a baked set of numerals, fatal for a real dump
   where `__PTRDIFF_TYPE__` is `long int` (now deleted, not left as a trap); `--march` did not skip
   its own argument, so the command failed outright; and the new test deleted the shared scratch
   directory out from under its neighbours.

   ✅ **Per-TU selection landed too — `TranslationUnit::target_flags`.** Measured on VPP:

       target flags: 1963 of 1967 C units carry one
       distinct -march: haswell, silvermont, x86-64-v2, x86-64-v3, x86-64-v4

   **Five targets, and every chiero measurement to date used none of them.** That is 060 §1.1's
   multiarch as a number rather than a warning. Kept apart from `defines`/`include_paths` because
   they *select a persona* rather than configure the preprocessor — only the compiler knows what
   `-march=haswell` implies, so they go to a `cc -dM -E` probe uninterpreted.

   📊 **Measured on the real corpus after the join, 2026-08-08 (1358 s, 22.6 min):**

       012 c17: 1967/1967 C units | 0 panicked | 0 diagnosed | 0 unreadable | 818 380 190 tokens
         personas: 8 distinct target flag-sets probed from cc

   **+25 975 467 tokens against the run before it — 3.3% more of VPP visible**, and 0 diagnosed
   throughout, which is the same pair of numbers the persona work has produced every time: the
   count says *no unaddressed complaint*, the token delta says *this much more of the program is
   now being read*. Only the second can see a branch taken correctly for the first time.

   ⚠️ **8 distinct flag-sets, where I had written 5 — in the crate doc, the spec and two tests.**
   The five distinct `-march` *values* are real; a flag-*set* is not a `-march`. VPP's carry
   `-mtune=generic`, `-mprefer-vector-width=512` and `-maes` alongside it, four units carry none
   at all, and **one names `-march` twice** (`x86-64-v2 … silvermont`, last wins). Every one of
   those is a case an interpreter would get wrong, which is the argument for handing the flags to
   a compiler verbatim. Enumerated with one `ninja -t compdb | python3` pipe — **the number was
   one command away the whole time and I wrote it from memory instead.**

   📌 **And it answers the standing "AVX2 has never been compiled" note.** All eight flag-sets
   probe cleanly on gcc 13.3 (401–432 predefines each), and **388 of VPP's 1967 C units now carry
   a persona that defines `__AVX2__`** — 192 at `x86-64-v3`, 192 at `x86-64-v4`, 4 at `haswell`.
   Four sets define `__AES__`. The duplicate-`-march` unit resolves to **silvermont's** 416
   defines rather than v2's, i.e. last-flag-wins is a compiler fact chiero correctly declines to
   model. That vector half of vppinfra is inside the corpus gate now; it is still outside every
   *findings* sweep, which drives its own configuration through the CLI.

   ✅ **Both halves closed 2026-08-08 — `chiero-probe`, and the join.** The 24th crate exists for
   one reason: `chiero-cli` and `chiero-vpp` both need "what does *this* compiler predefine under
   *these* flags", and a second `cc -dM` probe in `chiero-vpp` would have been the **third**
   mechanism for one fact (1b's whole complaint). `chiero-pp` stays free of subprocesses.

   - **The cache was keyed on nothing.** `system_environment` took the target flags and memoized
     the answer in a `OnceLock`, so within one process the *first* flag-set was answered to every
     later one. Latent while one process meant one operation meant one flag-set — and a sweep is
     exactly the case where it is not. `Probe::persona_probes()` counts the **subprocess**, not the
     call, so "one run per distinct flag-set" is measured rather than asserted (5 for 1967 units).
   - **`TranslationUnit::pp_config` now takes the probe** and joins `target_flags` → `persona`. A
     parameter rather than an option, for the reason the `ConfigId` is handed over ready-made: a
     caller that skips the join gets every `#if defined(__AVX2__)` in its `#else` and **nothing in
     the output says so**. 060 contract 2's structural half was already met; this is the half that
     makes it mean something — one source, N units, N *different programs*.
   - Mutants confirm both: dropping the memoization, returning the cache's first entry whatever the
     key, and passing `&[]` in place of the unit's flags each fail a test that named them.
   - **A defect in the new code, found by asking what a bad flag does rather than by review.**
     `cc -march=nonsense` exits non-zero and prints no `#define`, and `Persona::from_defines` over
     that text is a well-formed persona with **zero** entries — the worst answer available, since
     `__GNUC__`/`__linux__`/`__x86_64__` all undefined sends every header down its `#else`. The
     code asked whether the process *started*, not whether it *answered*. Now one rule, plus
     `failed_probes()`: the baked fallback is right and being handed it unknowingly is not, so the
     corpus gate prints each substitution and fails on any. (gcc 13.3 accepts all eight of VPP's
     flag-sets, so this is a guard rather than a live failure — also measured, not assumed.)


1z. **🗄️ Original entry, kept because its reasoning held up — closed, not parked.** It read:
   PARKED at the owner's request
   2026-08-07 — `-march`. Do not start without checking in;
   the owner asked to discuss the design first. What was agreed: the *flag propagation* half is a
   bug regardless (chiero probes the compiler with no flags while the sweep replays real ninja
   lines, so it preprocesses a different configuration than the one that ships), and the
   intrinsics half needs the 7-second `probe.sh` run before anyone designs it — the real first
   error may be `#pragma GCC target`, not intrinsics at all. And it is probably not "add a
   `--march` flag": VPP's multiarch compiles files repeatedly under different `-march`,
   per-function, so the target configuration is per-TU. Seven of the 11 `failed` plugin entries
   are one cause: `frontend::predefines` asks gcc for macros with **no `-march`** while VPP builds
   `-march=x86-64-v2`, so `__SSE4_2__` is undefined and `vppinfra/crc32.h` never defines
   `clib_crc32c_with_init`. The other four are two parser/sema gaps in generated API headers.

   🆕 **2026-08-08 — `probe.sh` was run, since this entry names it as a prerequisite to the design
   rather than part of it. The answer is neither "intrinsics" nor "`#pragma GCC target`".** The
   five default TUs are clean at 900–1700 ms each. The `-march`-gated ones report **`clean` in
   1 ms**:

   ```
   vppinfra/test/aes_cbc.c   [clang, 3 target(s)]  {"status":"clean","ms":1}
   vppinfra/vector.c         [clang, 1 target(s)]  {"status":"clean","ms":1}
   vlib/main.c               [clang, 1 target(s)]  {"status":"clean","ms":1670}
   ```

   ⚠️ **A 1 ms "clean" is not a pass, it is an empty analysis** — and that is measured, not
   inferred. For `aes_cbc.c` under those flags **gcc itself preprocesses to 6 non-blank lines**,
   and `chiero cir` lowers **0 functions**. The body is entirely behind a guard the configuration
   does not satisfy, so chiero is correctly reporting a clean analysis of six lines of nothing. Same class as `--verify-cir` dropping
   all nine functions of `crc32_5tuple.c`, and the same class as the 8% of VPP the persona was
   hiding before today's predefine fixes. **So the intrinsics question cannot be asked yet:** the
   TUs that would answer it are not being analysed at all, and the failure is silent. Fix the
   configuration first and the question may look different — or may answer itself.

   *(`plugins/nat/cnat/cnat_node_vip.c` reports `NO TARGET`: not built in this configuration, so
   the `clib_crc32c_with_init` failure named below cannot be reproduced from this build directory
   either. See the stale-build-directory item.)*

   🆕 **2026-08-08 — the missing ingredient now exists, which changes the shape of the design (it
   does *not* unpark it).** When this was parked there was no mechanism that knew which `-march`
   any given TU used. 060 contract 1's `BuildDb` is that mechanism:
   `TranslationUnit::args` holds the full command line for all **1967** C compilations, including
   the **1964** `-march=` occurrences, keyed per translation unit, tested, with **423** distinct
   configurations already identified.

   Combined with the owner's config-file persona idea (1b), the natural shape is: **a persona is a
   named set of predefines, and `BuildDb` selects one per TU.** That turns `-march` from a new
   subsystem into a lookup over machinery that already exists and is under test. ⚠️ Still parked —
   the owner asked to discuss the design, and having a better design is not the same as having
   permission. Raise it together with 1b, 1d and 1e.

   ✅ **PARSED FOR THE FIRST TIME 2026-08-08 (second session), and it is an honest zero.** With
   `--march` reaching the persona, 24 sampled v3/v4 units lower with **0 diagnostics**, and the
   widening is real rather than nominal: **+292 definitions per TU at v3, +524 at v4** — the
   `vector_avx2.h` / `vector_avx512.h` inline bodies, which gcc confirms are 1516 and 2104
   preprocessed lines respectively. `tests/corpus/vpp-findings/march_probe.sh` is committed so the
   surface stays measurable; the *findings* sweeps still drive `chiero` with no `-march`, so for
   them the sentence below remains true.

   ⚠️ **The first reading of this was a false zero, and the instrument was the defect.**
   `grep -c '^func'` counts declarations too — a VPP TU has ~5000 — and it reported the v3 and
   no-march runs byte-identical at 5566. That would have been written into the yield table as
   "the widening measured nothing". The definition marker is `{ ; span`, and by it the same file
   goes 5560 → 5852. **An instrument that reports a plausible number is not a measurement**, and
   this file's own rule caught it: an unchanged number is a claim that needs checking, not a
   result.

   ⚠️ **CORRECTED 2026-08-08: the mechanism below is wrong, and the correction is the useful
   part.** `unsupported-access-width` is indeed zero everywhere — but *not* because the corpus
   cannot produce a 32-byte access. It produces them in bulk: the AVX-512 lowering of one TU,
   `vlib/handoff.c`, contains **7779 `copymem` of 32 bytes or wider** (4038 of exactly 32, 3740 of
   64). The finding class is unreachable because of the **access shape**, not the corpus: a vector
   access lowers to `copymem`, never to a wide `load`/`store`, which is 020 §4.13b's "no aggregate
   values in CIR" applied to `vector_size` types. Measured with a five-line probe:

       u8x32 load32 (u8x32 *p) { return p[0]; }
       ->  copymem %6 -> %13, 32i64 align 16

   So the whole class was ruled out by a decision this project made deliberately, and the "corpus
   cannot reach it" story survived because nobody had asked what the IR actually contained. What
   *is* still true: **none of the pinned 40 entries is compiled at v3/v4 at all**, so that corpus
   sees no vector code whatever the shape.

   ✅ **And the shape that *does* exist is caught, pinned, and mutation-checked.** A 32-byte vector
   store past a 16-byte object reports `out-of-bounds: 32-byte access at offset 0 of buf, which is
   16 bytes`, fidelity **Exact**, while the 16-byte store into the same object is clean —
   `crates/chiero-cli/tests/cli.rs`, end to end from C. Nothing covered it: `chiero-exec`'s
   `a_width_limit_does_not_mask_a_use_after_free` calls `Memory::read_term(.., 32, ..)` directly,
   which is the *wide-load* path, and C vector code does not take it. Two mutants die (`CopyMem`
   reporting no faults; a bounds check flagging every access ≥16 bytes), and the second is killed
   by the assertion that the message **names the width that overran** — a bare "a finding exists"
   would have let it through. `find-bugs` over 8 vector-using VPP entries gave 1 finding, a known
   class.

   🗄️ **Measured 2026-08-07, and it makes the item bigger than those seven entries.** Retaking the
   pinned 40 after `BadRange` left the defect list gave byte-identical numbers, and the kept
   envelopes say why: `unsupported-access-width` occurs **zero** times in all forty — the corpus
   cannot produce a 32-byte access at all.

   | | |
   |---|---|
   | every 32-byte type in VPP is in `vppinfra/vector_avx2.h` | `vector.h:197`, under `#if defined (__AVX2__)` |
   | `__AVX2__` needs `-march=x86-64-v3` or `-mavx2` | `gcc -dM -E`: defined at `v3`, **not** at `v2`, not with no `-march` |
   | VPP's baseline is `-march=x86-64-v2` | so even VPP's own default build has none of it |
   | the AVX2/AVX512 paths are the **multiarch variants** | which is exactly this item's per-TU target configuration |

   So it is not only that seven entries fail to preprocess. **Every AVX2 and AVX512 vector path
   in vppinfra has never once been compiled by any chiero measurement** — including the code
   021 §5 cites when it says "vppinfra uses `u8x32`/`u8x64` throughout". Every "0 findings" this
   project has published over VPP is silent about that half of the tree. Worth putting to the
   owner when the item is unparked; it does not change the recommendation to design first.


1e. ✅ **CLOSED 2026-08-08 — the gate now measures the shipped configuration.** Both facts come
   from `chiero_probe::Probe`, the same one the CLI uses: system include paths *and* the persona
   the unit's own `-march` selects. The private `gcc -E -v` scrape in the test file is gone, so the
   count of mechanisms went from three-if-I-had-done-the-obvious-thing to **one**. The run prints
   `personas: N distinct target flag-sets probed from cc`, because that is the number that moves
   when the join is wrong and no other number here does.

   *Original entry, kept for the reasoning that made waiting right:* **it measures a configuration
   nobody ships — and fixing it is
   blocked on the persona design, not on effort.** The gate takes each TU's `-D`/`-I` from
   `builddb`, its system paths from `gcc -E -v`, and its **predefines from `Config::default()`'s
   baked table**. `chiero-cli` does not: `frontend::predefines` runs `cc -dM -E -std=gnu11` and
   captures all 401. So the gate preprocesses VPP under a persona the real build never uses.

   That is precisely why the gate earned its keep — it saw the 8% because it used the baked
   table — but as a *standing* gate it should model the shipped configuration, and the persona
   gaps are now covered by `persona_gap` instead.

   🗄️ **Not started, deliberately** *(the marker was ⛔ and is now historical — a blocker emoji on
   a closed item is exactly the rot §11.3 says to sweep for)*. The obvious fix is to capture
   `cc -dM` in `chiero-vpp` too, which would make **three** mechanisms for one fact. The right fix
   is one mechanism — a named persona the preprocessor owns, which is exactly the owner's
   config-file idea (1b). Building `Config::from_compiler()` now would pre-empt that design.

   **And waiting was right**: 1b landed as `Persona`, and the one mechanism it wanted is now a
   crate that both callers share. Had this been "fixed" when it was filed, the fix would have been
   the third mechanism.


3. ✅ **CLOSED 2026-08-08 — the ingest is built and both blocked contracts are met.**
   `chiero_vpp::builddb` (060 contract 1) and `chiero-vpp/tests/preprocess_corpus.rs`
   (012 contract 17). Three things worth carrying forward:

   **a. The blocker was in the interface, not the world.** 060 §1 wanted a
   `compile_commands.json` *file*; VPP's build writes none, and still doesn't. `ninja -t compdb`
   emits the identical format on stdout in 90 ms. Taking `&str` instead of a path closed a
   months-old blocker with no re-configure and no VPP edit.

   **b. `ninja -t compdb` dumps every edge, not every compilation — and I published the wrong
   number before catching it.** 2902 of VPP's 6235 entries are phony order-only rows: empty
   `command`, `output` like `cmake_object_order_depends_target_…`, `file` naming a *generated*
   source. My first measurement said "2226 C entries" and I wrote it into the spec, the module
   docs and a test table. **Real figure: 1967 C compilations** over 1562 sources, 208 built more
   than once (max 5, not the 9 the phony rows implied).

   It was caught only because the ignored corpus test asserts a *property* — every unit has an
   include path — and 259 rows had none. **A test that had merely counted would have agreed with
   the wrong number forever.** That is the general rule and it is cheap: when a corpus test can
   assert a property instead of a total, assert the property; the total cannot contradict itself.

   **c. What a `ConfigId` is worth, quantified.** Hashing exactly `-D` and `-I` — the flags that
   decide which `#if` branches exist — collapses 1967 units to **423 configurations**, 4.6×.
   Hashing the command line would make every unit unique and buy nothing. Both directions are
   asserted; mutation-checked.

   *(Original entry: the M2 note recording that no `compile_commands.json` existed. It was true
   when written. See §11.3's rule — **re-measure a blocker before routing around it**; this one
   cost nothing to check and had stood for months.)*


4. ✅ **Mostly answered 2026-08-07 by a config block, and the entry below is kept for what is
   left.** The whole item was written around `conversions` taking 53 s — which is the **default
   dev profile**, not the code. The same test takes **7.95 s** built by release, and
   `[profile.dev]` had no tuning at all. Setting `opt-level = 2` (with `debug-assertions` and
   `overflow-checks` pinned on) takes the full three-leg `./check.sh` from **6m51s to 3m56s**,
   both warm — 43%, about three minutes a run, against a one-off 7m12s rebuild.

   ⚠️ **The first comparison was contaminated**: 6m14s, taken after editing `chiero-solver`, so
   it carried a partial rebuild at the new opt-level. The tell was `user` time going *up* while
   wall time went down. Re-take a timing when anything has been rebuilt in between.

   ⚠️ **And the experiment turned the suite red for a reason that was not the experiment** — the
   slicing ratio test was load-sensitive and only a full-workspace run had ever exercised it
   under load. Fixed with an exact counter; see §11.1.

   *Still open, and now the smaller half:* each test binary still rebuilds the corpus from
   scratch, because Rust integration tests are separate processes. Serialising the analysis to
   disk is the remaining idea, and **020's CIR text format may already be most of it** — but the
   43% is banked, so the case for that work is now three minutes, not ten.


5. ✅ **CLOSED 2026-08-07 — "a step that outlives the clock" had the wrong cause, and the sweep
   now has zero `timeout` rows.** The entry said three find-bugs entries needed the outer
   `timeout` because "the clock is only checked *between* steps", and 023 §8 named
   `max_solver_rlimit` as the bound for them.

   ⚠️ **Neither `--solver-rlimit` nor `--time-budget` moved them at any value.** The two rows —
   `plugins/unittest/fib_test.c` and `llist_test.c`, named for the first time because the old
   numbers recorded only a count — were spending their time in `chiero_cir::verify::dominators`,
   which runs **before a single instruction executes**. No clock, no solver, so nothing 023 §8
   defines could ever have reached it. `chiero layout` on the same file, frontend only, finished
   in 1.3 s, which is what said the frontend was innocent too.

   The verifier was super-quadratic in the block count: **11.5 s for 3001 blocks** in a release
   build, 158 s in a debug one, each doubling costing about six times the previous. `dominators`
   rebuilt the predecessor list *inside* the fixpoint loop and met dominator sets with
   `retain(|x| dom[p].contains(x))` — linear in a set that starts as every block in the function.
   Now 270 ms at 3001 blocks; both VPP entries are `ok`; 023 §8's attribution is **retracted in
   the spec**, not quietly edited.

   **How it was found, because the method is the reusable part:** the stated cause was *tested*
   (does the proposed bound cut these rows? no), then a stack sample under `gdb` named the
   function. ⚠️ `ptrace_scope=1` on this machine blocks `gdb -p`; run the program **as gdb's
   child** and `pkill -INT` it from a background subshell —
   `gdb -batch -x cmds --args ./target/release/chiero …` with `run` then `bt 18`.

   ✅ *And the original entry's last leftover closes too:* **`max_memory_objects` is built**
   (2026-08-07), so **every field in 023 §8's budget sketch now exists** — the first time that
   has been true. It bounds one *path* where `max_states` bounds paths.

   ⚠️ **Enforced between steps, deliberately, and the enumeration is why.** Objects are minted
   from eleven sites in `chiero-exec` *and* from every model in `chiero-model` via
   `ModelCtx::mem`, which `chiero-vpp` extends — no call site sees them all, so a check at the
   allocations would be a check at some of them. The cost is that the count can pass the limit
   by one step's worth (measured: 13 against 12), which is `max_forks`'s shape and is stated in
   the spec rather than glossed.


5a. ✅ **DONE 2026-08-07 — the verifier's scale test asserts a counter, not a clock.** It had
   asserted 5 s, chosen under the unoptimised dev profile; `opt-level = 2` made every build about
   6.7x faster, the bound stayed, and a mutant restoring **one** of the eight removed scans came
   in at 4.60 s and passed. **A wall-clock assertion silently weakens whenever the build gets
   faster** — nobody edits the test, it just stops being able to fail.

   `verify::terminators_examined()` counts what actually differs: examining every block's
   terminator **once per function** is linear, **once per block** is quadratic. The same mutant
   now reports **144 108 008 against a bound of 240 020** — a factor of 600, identical on any
   machine, at any load, under any profile.

   ⚠️ **The design point, and it is the one I had already got wrong once that day.** The counter
   has to attach to the *scan*, not to the site a fix happened to hoist it to. Counting
   "predecessor maps built" would have gone **down** under the mutant — which stops building one
   — and the test would have passed. Every `successors()` call in `verify.rs` goes through one
   counted wrapper, so a per-block scan increments per block by construction.

   The duration survives as a loose 30 s smoke check, explicitly *not* the assertion, so a
   catastrophic regression fails fast instead of hanging the suite.


5e. ✅ **CLOSED 2026-08-08 (second session) — 950 KB → 11.9 KB, and the fix found a second
   defect in the fix.** `nsh_md2_encap`'s envelope is now 64 bindings plus an account of the rest;
   `Witness::digest` (chiero-exec, since 023 §9 owns what a witness is) bounds it, pinned bindings
   first, and nothing is reordered when nothing is dropped.

   **The fixture four earlier attempts missed, recorded because it cost four waves.** They all
   reached for `copymem`, which forks on the aliasing check against a lazy object — and the
   finding then lands on the *mint-free* fork, so the witness came out empty. **Straight-line
   loads at distinct offsets through the entry pointer** mint one byte each, do not fork, and put
   the fault after the mints: n loads, n + 3 bindings, ~96 JSON bytes each, linear to any size.
   The general form: *when four attempts to reproduce a symptom fail, check whether the layer you
   are reproducing it at is the layer the defect is in.* This was a reporting defect the whole
   time, and reporting is testable without an engine run at all — three of the six tests here
   construct a `Witness` directly.

   **Three things the wave got wrong and measurement corrected:**

   | claim | what the measurement said |
   |---|---|
   | "show the first 64" | the pinned bindings are the **last four** in the fixture — that bound would drop every value the finding depends on |
   | "pinned-first keeps what matters" | on the real case **10 580 of 10 594 omitted bindings are pinned**; `pinned` means the *model* gave it a value, and a total model pins nearly everything |
   | truncating the report is harmless | `check_reachable` licenses `proven` on *"a solver pinned every input"* — computed from a bounded view, a truncated witness would turn an unproven arrival into a **proof** |

   The last is the one to remember: **a bounded rendering must not become a bounded check.**
   `PathWitness::all_pinned` runs over the whole witness, and a chiero-exec test pins the property
   it rests on (a bounded view *can* be all-pinned while the witness is not).

   ⚠️ **One mutant survives and is recorded rather than hidden:** computing `all_pinned` from the
   digest instead of the witness passes every test, because killing it needs a witness with ≥65
   pinned bindings *and* one unpinned, which no fixture here produces. The guard is written at its
   source and commented; a future editor moving it has no test to stop them.

   Applied at **both** render sites — `find_bugs` and `check_reachable`, which solves for its own
   bindings and had its own unbounded rendering. §7.2's rule, and the reason `check_reachable`'s
   trap was found at all.


5c. ✅ **CLOSED 2026-08-09 — `parse_model` was quadratic, and the fix uncovered a second
   defect.** Found by *sampling*, not by reading: two stack samples 50 s apart both landed in
   `parse_model`. chiero was not waiting on z3 — it was reading z3's answer.
   `text.split(&format!("define-fun {key} "))` ran **once per variable** over a text that grows
   with the variable count. **Item 5b's shape, third instance in this workspace.**

   | variables | 500 | 1000 | 2000 | 4000 |
   |---|---|---|---|---|
   | before | 0.007 s | 0.025 s | 0.087 s | 0.355 s (~3.4–4.1x per doubling) |

   | entry | before | after |
   |---|---|---|
   | `nsh_md2_encap` | **>120 s, killed** | 64 s, exit 0, 5 findings |
   | `nsh_md2_decap` | timeout | 64 s, exit 0, 5 findings |
   | `format_nsh_header` | timeout | 62 s, exit 0 |

   ⚠️ **Still cut by the sweep's 60 s budget, and now for a different reason.** Two fresh
   samples land in `read_form` — genuinely waiting on z3, which is 023 §8's territory. The
   attribution those rows always carried is only *now* the true one.

   📌 **Bounding the scan to one entry exposed a second, older defect.** A `Bool` prints
   `(define-fun v0_b () Bool true)` with no `#x`/`#b` token, so the unbounded search ran on into
   the *following* definition and gave the bool whatever bit-vector it found there. The model
   stayed plausible, which is why nothing caught it — until the bounded scan returned nothing
   and `a_bool_variable_is_usable` went red. A test now plants `0xdeadbeef` immediately after a
   bool, because that is exactly what the old parser returned for it.


5g. ✅ **CLOSED 2026-08-08 — `pick_entries.py --verify-cir`.** It keeps only names that survive
   into the lowered module, using `chiero cir` (built earlier the same day, which is what made
   this tractable — before it there was no way to ask).

   **A filter, not a replacement, and the split is the point.** The CIR for one VPP `.c` names
   ~7000 functions, nearly all inlines from headers, and nothing in it says which file a
   `func @name` came from. **The text knows "defined in this file"; the CIR knows "survives the
   preprocessor".** Each is asked the question it can answer.

   Verified on the two files that produced `nofn` rows: it drops
   `clear_session_dbg_clock_cycles_fn` (inside `#if SESSION_DEBUG > 0`) and `compute_ethernet_key`
   — and it names what it dropped, because a corpus that quietly shrinks is one nobody can check.
   ⚠️ `crc32_5tuple.c` loses **all nine** of its functions: the file is behind an `__SSE4_2__`
   guard, so chiero lowers none of it. That is correct and it is the parked `-march` item showing
   through — the corpus now reflects chiero's *actual* configuration rather than the source text's.

   Off by default: it costs a `chiero cir` run per file. `CHIERO_FLAGS` carries the include and
   define flags, since lowering a VPP file needs them and the picker has no other way to know.

   *(Original entry: three `nofn` rows in the `vnet/` sweep, none of them the known macro-name
   problem — all three were real definitions the configuration removes.)* Three `nofn` rows in the
   `vnet/` sweep, and none is the known macro-name problem: all three are real definitions in the
   source. `clear_session_dbg_clock_cycles_fn` is inside `#if SESSION_DEBUG > 0` and
   `session_debug.h` defines `SESSION_DEBUG` as `0`, so it is absent from the *configured* TU.
   chiero is right and the row is honest.

   `--built-only` does not help — the file is compiled, just not that part of it. **The fix is to
   pick entries from what chiero lowers rather than from the text**, which makes every entry a
   function that exists by construction. Until then `nofn` is a corpus artefact, not a chiero
   limitation, which is exactly what the status was invented to make visible.


5f. ✅ **CLOSED — `--built-only` shipped 2026-08-08.** The sweep analyses files VPP does not compile — and that is how it found a real VPP
   defect.** `src/vnet/fib/fib_entry_src_default.c` defines `fib_entry_src_default_deinit`
   **twice**, at lines 22 and 35, both `static void … {}`. chiero refuses it; **gcc gives the
   identical error at the identical line** (`redefinition of …`); and the file is **not in the
   build at all** — zero of `ninja -t commands all`'s 2945 entries mention it, and
   `src/vnet/CMakeLists.txt` does not list it. It is dead source that has never compiled, which
   is exactly why nobody noticed.

   Two things follow, and the second is the actionable one:

   - chiero found a genuine VPP defect by reading a file the build ignores. Small, but real, and
     the kind of thing 050's tool surface exists to report.
   - **`pick_entries.py` globs `vnet/*/*.c` and `plugins/*/*.c`, so the corpus includes source
     the build never touches.** That inflates `failed` with rows that are neither chiero's
     problem nor VPP's compiled code. `ninja -C $VPPBUILD -t commands all` is the authoritative
     list and takes **63 ms** (§9.2) — filtering the entry list through it would make every
     `failed` row a statement about code that ships. ⚠️ Do this *before* the next sweep, or the
     residue keeps mixing two different kinds of rejection.


5d. ✅ **CLOSED 2026-08-08 (second session) — and the staleness was narrower than this entry
   claimed.** Measured before touching anything: **165 of 2629 sources under `src/` are newer than
   the whole build**, every one of them at the same timestamp — a single checkout **22 seconds
   after the build finished**. Of those 165, **4 were `.api` files**, and only those 4 could
   matter.

   ⚠️ **The correction, and it bounds the problem:** chiero reads `src/` **directly**, so a `.c` or
   `.h` that has moved on is read as it is today — nothing is stale about it. The only derived
   artifacts chiero includes are the **1049 `*.api*.h`** headers and four `config.h`/`version.h`
   that come from cmake options rather than from source. So "chiero reads a slightly different
   program from the one VPP would build" was true only of the generated headers, which is a
   surface a script can check in a second rather than a reason to rebuild anything.

   **Fixed by running the exact `vppapigen` command ninja would run for each output** — *not* by
   `ninja`, whose target for a generated header depends on a **cmake re-run**, which rewrites
   `build.ninja`: the file `chiero_vpp::builddb` reads for all 1967 compile commands, that
   `probe.sh` replays, and that 012 contract 17's corpus gate is built from. Verified after:
   compdb still 6235 entries / **1967 C compilations**, byte-for-byte the same count.

   **The before and after, both ways round:** with the stale header, `chiero cir lldp_api.c` says
   *"no member named `last_heard_age`"* at 135:7 and **gcc reports the identical error at 135:12**
   — which is what made it an environment fact rather than a frontend defect. Regenerated, the
   same file lowers **6796 functions**, and `lldp_cli.c` and `lldp_test.c` with it.

   📌 **`tests/corpus/vpp-findings/api_staleness.py` is committed** (§9.2's rule: the instrument
   goes in the repo in the same wave). It reports drift and exits 1, `--fix` regenerates. Checked
   that it *can* fail by ageing one header: reports 1 stale, `--fix` clears it. A minute to run,
   and the class of failure it catches — a sweep row that looks exactly like a frontend bug — cost
   a wave to diagnose the first time.


5k. ✅ **CLOSED 2026-08-08 — an enum's declared underlying type was parsed and thrown away, and
   `layout` reported the wrong size as `proven`.** `struct S { enum small s; char c; }` with
   `enum small : unsigned char` came out **8 bytes, align 4**; gcc says **2 and 1**. The envelope
   said *"proven — this holds for all inputs (Exact)"*.

   **VPP declares 22 of these across 6 files**, all `typedef enum name_ : u8` — `quic/quic.h`,
   `http/http_buffer.h`, `vperf/builtin/vperf_builtin.h` — so every struct holding one had the
   wrong size, silently. Verified on VPP's own form after the fix: `2`/`1`, matching gcc.

   The chain was three correct decisions and one missing link: the parser parsed the `: T` and
   discarded it *with a comment saying the representation "is what 014 owns"* — right about the
   ownership — and 014 was never given it, so sema fell back to the implied type. Sema already had
   the machinery (`enums: Symbol → TyId`, 014 contract 10). The fix is one AST field
   (`TypeKind::Tag::underlying`, 7 sites), the parser keeping what it already parsed, and
   `enum_ty` preferring it. The enumerator fitting still runs, because that is what produces the
   pedantic diagnostics and silencing those would hide a real ISO complaint.

   📌 **Found by auditing a class, not a site.** `let _ = <named parameter>;` across the
   workspace's sources: 8 hits, 4 unexplained, and this was the one with a consequence. The audit
   itself came out of 5j, where an *undocumented* deliberate discard cost a wrong RED — so the
   lesson paid for the next wave immediately. The other three unexplained discards are noise
   (`chiero-diff`'s loop index, `chiero-pp`'s matched `(` token, `chiero-cir`'s explained-below `t`).


5m. ✅ **CLOSED 2026-08-09 — `chiero layout` applies the same rule `lower` does**: errors refuse
   the TU, advisories are printed and do not. Six lines, identical in shape to the path ten
   lines above it — which was the point, since the two frontend entries had different answers
   to one question and one of them was "do not ask". The entry is kept below for the reasoning.

5m-orig. 🗄️ **`chiero layout` ignored sema diagnostics entirely** — found by the 2026-08-09 review,
   **pre-existing**, not from that session's changes. `chiero-cli/src/frontend.rs`'s second
   frontend path never looks at `analysis.diagnostics` at all, so a TU containing a hard error
   (an undeclared name) still produces a layout report stamped **`proven — this holds for all
   inputs (Exact)`** and exits 0, with the diagnostic never printed. It contradicts the module's
   own header ("Every stage's diagnostics are a refusal") and now also the severity policy
   `lower()` implements ten lines above it. The severity work did not cause this; it made the
   inconsistency untenable.


5n. ✅ **CLOSED 2026-08-09 — and the fix reached one reader of three.** `_Alignof(A_t)` is 16
   now, matching gcc, for all three typedef spellings. ⚠️ **The sema fix passed its unit test
   while `chiero cir` went on emitting the old numbers**: lowering has its own `AlignofType`
   arm that asks `align_of` on the resolved `TyId` and falls back to sema's fold only if that
   returns `None` — which it never does for a complete type, so the correct fallback was
   unreachable. Checking the original reproduction rather than the new test is what caught it,
   and an end-to-end test now pins the constant in the CIR. **Third time this session one fact
   had multiple readers disagreeing.** Original entry:

5n-orig. 🗄️ **`_Alignof` of a typedef never saw an `aligned` attribute on the typedef.** Also
   pre-existing. gcc gives `_Alignof(A_t) == 16` for
   `typedef __attribute__((aligned(16))) struct A { char a; } A_t;` and chiero says 1; the
   post-declarator spelling fails the same way. ⚠️ Member layout *through* the typedef is
   **correct** (`struct Holder { char c; A_t m; }` is 32 with `m` at 16, matching gcc), so this
   is narrow. The `from_specifier` fix moved the wrongness from "the record itself was 16",
   which was worse, to "the typedef name is 1".


5o. ✅ **CLOSED 2026-08-09 — and the re-take the item demanded is what caught the fix's own
   defect.** `Outcome::Advised` exists, `(Warned, Advised)` is `Agree`, and the two-file
   reproduction that printed `SEVERITY MISMATCH — 1 sema: signed overflow` prints `agree 1`.

   ⚠️ **The first `(Diagnosed, Advised)` arm was wrong and the numbers said so.** I filed it as
   `Miss` — *chiero produced a value where gcc would not build, so chiero is missing a rule* —
   and the VPP re-take moved **255 of 1552 files** under a heading reading "chiero is missing a
   rule". `BothRefused`'s own doc had the answer: gcc refusing on a real tree almost always
   means the flags are wrong for that file, so **gcc never judged the C**, and that holds
   whatever chiero's severity was. It is the exact inflation that bucket was split off to
   prevent, reached from the other direction. Now `BothRefused`.

   Two more the item did not predict. The sema site **returned early on any diagnostic**, so an
   advisory would have hidden a lowering `NotRun` behind the milder verdict — it looks for an
   *error* anywhere now, not merely the first diagnostic. And two labels went stale the instant
   the taxonomy moved: "agree, both clean" is no longer both clean, "both refused" is not both
   refusing. Leaving them would have been this item's own defect a second time.

   📌 **The item's scope estimate was wrong in the safe direction.** It said the over-report
   "needs a signed-overflowing *constant expression* — rare in VPP", having measured only the
   `gnu` dialect. Under the **pedantic** dialect the sweep runs by default, the ISO conformance
   remarks all fire: **255 of VPP's 1552 files have a chiero advisory as their only diagnostic.**

   **Re-taken numbers** (`xtask sweep --tree vpp/src` with the build's own `-I`/`-D`, pedantic):

   | findings | misses | agree | gcc refused | severity mismatch | tool gaps |
   |---|---|---|---|---|---|
   | 0 | 0 | 6 | 1390 | 0 | 156 |

   **No bucket count moved**, since every advisory-only file pairs with a gcc refusal — so old
   sweep numbers stay comparable. The report now prints a `chiero advised: N — gcc warned … gcc
   clean … gcc refused …` line, so a reader recovers the pre-change totals from a *new* run
   rather than trusting a commit message. ⚠️ Its test passed on the first run and the mutation
   swapping two of its three counts **survived** — one file per category cannot detect an
   ordering. Distinct 1/2/3 now, mutation verified fatal.


5p. ✅ **CLOSED 2026-08-09 — `chiero-diff`'s `parsed_cleanly` ignores sema, and that is
   correct.** Filed as a question, answered by measurement within the hour. Found by completing the consumer audit rather
   than stumbling on it: there are exactly **four** production readers of sema diagnostics
   (`chiero-cli/src/frontend.rs` twice, `chiero-diff/src/lib.rs`, `xtask/src/sweep.rs`, plus sema's
   own internals),
   and this is the only one that never looks.

   `parsed_cleanly = tu.diagnostics.is_empty() && parsed.diagnostics.is_empty()` — pp and parse
   only. The flag is **conservative by design**: 031 §4 puts every entity of an unreadable file
   into the impact set, so it widens the answer rather than refusing it. That design is right.

   The question is whether *sema* belongs in it. `chiero-diff` uses **layouts** to decide impact
   — `RecordShape { size, align, packed }` is the thing tokens cannot supply — and a TU where
   sema errored can still hand over records. A record resting on a mis-resolved typedef would
   then produce a confident wrong `size`, and the flag that exists to widen the set would not
   fire, because the parser was silent.

   ✅ **ANSWERED the same day, and the answer is no — `parsed_cleanly` is already sufficient.**
   The dangerous combination would be a *sema* error plus a still-`complete` record whose layout
   is wrong. Measured over four shapes:

   | shape | where it fails |
   |---|---|
   | `int f(void){return nope;}` + a clean `struct S` | sema errors, `S` is `complete=true size=8 align=4` — **the correct layout**, so nothing is wrong to widen for |
   | `struct S{ UnknownT m; char c; };` | **parse**: `unknown type name` |
   | `typedef UnknownT MyT; struct S{ MyT m; … };` | **parse** |
   | `struct S{ UnknownT m[4]; … };` | **parse** |

   **The errors that could corrupt a record's layout are parse errors**, because C cannot parse a
   member declaration without knowing whether the name is a type — and `parsed_cleanly` covers
   parse. A sema-only error leaves the records it did resolve correct. So the flag's scope is
   right as written, and the doc line "the preprocessor and the parser were both silent" is not
   an oversight but the whole reachable set.

   📌 Kept as a closed entry rather than deleted: *"is this conservative flag conservative
   enough"* is a reasonable question to ask again, and the next asker should get the measurement
   instead of repeating it.


6. ✅ **CLOSED 2026-08-09 — both halves.** (Was "partly closed": the CIR change landed hours
   before the engine did, and the payoff was not where the item said it would be — see §7.22.) The direct half needed no CIR change (`Callee::Direct`
   makes the type derivable); the indirect half is `Callee::Indirect { target, ret }`, text
   syntax `call %5 -> i32(args)`, 020 updated. ⚠️ **The claimed payoff was wrong and is reverted.** I deleted
   `require_ptr`'s `CTy::Void` exemption saying "nothing reaches here as `Void` now"; an
   adversarial review refuted it. `rvalue_type_in` records `Void` for any operand not yet
   *resolved*, and the type pass walks blocks in **textual order**, so a module whose dominator
   is listed later — legal, since 020 §8 rule 1 is about dominance — got a **false rejection
   that flipped when the blocks were reordered**. `require_ty` and `require_int` kept their
   exemptions all along, documented "skips unresolved values (recorded as `Void`), which is a
   known gap", three functions below the comment I wrote claiming the opposite.
   ✅ **Done 2026-08-09, in the right order this time.** `rvalue_type_in` returns `Option<CTy>`
   and unresolved values are **absent** from the type map, so `Void` means only *void*; then all
   four exemptions (`require_ptr`, `require_ty`, `require_int`, `select`'s arm check) came out
   in a second commit. Each restored check was proved by a test written to **fail first** —
   `store i32 <void>` and `add i32 <void>, 1` both verified clean before and are `WidthMismatch`
   now. Original note kept: **make "unknown" unrepresentable** — absent from the map rather
   than `Void` — after which all three exemptions are genuinely dead.
   `a_value_defined_in_a_later_listed_block_is_not_typed_void` guards it and reproduces the
   false positive if the exemption is deleted again.
   ✅ **CLOSED 2026-08-09.** The declared `ret` is the filter's rule, and wiring it up found
   the rule already there pointing the wrong way. `!(wants_value && f.ret == Void)` read as
   "a site that uses the result cannot be calling a void function", but `wants_value` is
   `dst.is_some()` and lowering assigns a `dst` to **every** call including a void one — so at
   a void-declared site it excluded exactly the void-returning candidates that belonged there
   and explored only the wrong ones. The declared type subsumes the intent (`void` is 0 bytes
   wide), so one width comparison replaces both rules.
   The three questions were answered as posed: (1) **same size**, mirroring the parameter
   filter's deliberate `Ptr` ↔ `Int(64)`; (2) the sweep was run — see §7.22; (3) the test
   changed, because the mismatch is now caught *before* the candidate runs, which is strictly
   better than degrading at the comparison — it is
   `a_candidate_whose_return_width_disagrees_with_the_site_is_excluded_and_said_so`, and it
   also pins the degradation message, which had to change: "a function chiero has not seen"
   is false about a candidate chiero saw and rejected.

   *Original note:* ⚠️ **And the engine still ignores `ret`.** Its only consumers are the verifier and the
   printer; `exec::indirect`'s candidate filter still checks arity and parameter shape only, so
   §7.6's class — a candidate of the right arity and the wrong return width — is **not** closed.
   Wiring `ret` into that filter is the remaining work and the item's original motivation.
   Verified on real input: `vppinfra/format.c` lowers to 184 418 lines, exit 0, its two indirect
   calls carrying `-> void` and `-> ptr`. The scope estimate held exactly: 22 mechanical sites, 3
   needing judgement, 1 spec edit. ⚠️ Two of the three were decided by **reading a fixture's own
   comment** — one said "the call site believes it called a pointer-returning function" while my
   blanket `Int(32)` said otherwise, and the verifier caught the contradiction. The entry below
   is kept for its reasoning.


6z. 🗄️ **Original entry — `InstKind::Call` carries no result type**, so an indirect call's result width is whatever
   candidate ran. The arity and parameter-type filters cut the wildest cases and cannot close it;
   the engine survives the rest by degrading. The real fix is a CIR change.

   ⚠️ **"135 sites construct `InstKind::Call`" was wrong, and it is the sentence that kept this
   item unstarted.** Measured 2026-08-09 — 135 is the count of *mentions*, most of them pattern
   matches, and 113 are in `chiero-exec`, which executes CIR and constructs none. The real shape:

   | | |
   |---|---|
   | production **constructions** | **3** — `chiero-lower/src/lib.rs:3484`, and the text parser's two spellings at `text.rs:1015` and `:1111` |
   | production matches to update | `text.rs:1889` (printer), `verify.rs:341/726/800`, `chiero-exec` 3, `chiero-opt` 5 |
   | test fixtures | ~110, of which **82 are in `chiero-exec/tests/step.rs`** — mechanical, and the bulk of the work |

   ✅ **HALF CLOSED 2026-08-09, with no CIR change and no fixture touched.** The item reads as
   one architectural change gated on a new field; it is two, and nobody had separated them.
   **Direct calls never needed the field** — `Callee::Direct` names a `FuncId` and `Function`
   carries `ret`, so `defined_by` returns the callee's declared type now. The only work was
   threading the module down two signatures, which `verify_function` already had. Direct call
   results are checked by `require_ptr`/`require_ty` like any other value, verified reaching
   real lowered C (`char *g(void)` → `Ptr`, `int f(void)` → `Int(32)`), and `./check.sh` stayed
   green — no latent defect surfaced and no false positive.

   📌 **And the audit that followed it is the reassuring kind.** Applying §11.0's top lesson to
   this exact fact — *who else decides a call result's type?* — `chiero-exec` was **already**
   reading `f.ret` from the module (`lib.rs:4136`), and has been. So the verifier was the one
   reader that did not ask, and the fix aligned it with the existing source of truth rather than
   inventing a second one. **An honest zero on "who else disagrees", and it is evidence the fix
   was the right shape**: the alternative — a side table, or a field the verifier maintains
   itself — would have created the divergence that was not there.

   **What is left is genuinely the indirect half**, which is also where §7.6's finding class
   lives: `Callee::Indirect` carries an operand rather than a signature.

   🆕 **And it is ~25 sites, not ~110 — because the field belongs on `Callee::Indirect`, not on
   `Call`.** Measured 2026-08-09: `Callee::Indirect` is mentioned **25 times across 11 files**,
   ~14 of them production (`chiero-lower` 4, `chiero-exec` 3, `chiero-cir` 4, `chiero-opt` 3).
   Every direct-call fixture — including the 82 in `step.rs` — is untouched by a change there.

   ⚠️ **The placement is not just cheaper, it is the correct one, and the recorded design was
   wrong about it.** A field on `Call` would make *direct* calls carry a copy of the callee's
   `ret` that is already in the module: **two sources of truth for one fact**, which is §11.0's
   top lesson with four instances behind it, introduced deliberately. On the variant, each
   `Callee` carries exactly what cannot be derived — nothing for `Direct`, the signature for
   `Indirect`.

       Callee::Indirect { target: Operand, ret: CTy }

   🆕 **Scope refined 2026-08-09 by reading every site, not just counting them** — "~25 sites"
   is right but hides which ones think:

   | | |
   |---|---|
   | **~22 mechanical** | `chiero-opt`'s three are pure pattern updates (checked: `Indirect(o) => v.push(o)`, `Indirect(_) => None`, `Indirect(_) => Some("makes an indirect call")`); likewise the verifier's other arms, the printer, and ~11 fixtures. A struct-variant change is **compiler-driven** — cargo names every one, so none can be missed silently |
   | **3 that think** | `verify::defined_by`'s `Indirect` arm (return the declared `ret` instead of `Void`); **`exec::indirect`'s candidate filter — this is the payoff**, and its comment already names the defect: *"entered a candidate returning `unsigned char`"*, which a declared `ret` would exclude; and the text format, needing syntax plus a round-trip test |
   | **1 spec edit** | 020's type sketch declares `Callee { Direct(FuncId), Indirect(Operand) }`, so the variant change is a spec change — legitimate, with precedent in the `conv: CallConv` retraction the same day |

   What it still needs, and none of it is architectural: the variant change and its ~25 sites,
   text-format syntax for the annotation (parser at `text.rs`, printer at `:1889`) with a
   round-trip test, and `defined_by`'s `Indirect` arm returning `ret` instead of `Void`. **The
   `require_ptr` Void exemption can then go**, which is the check that has been switched off for
   every call result since the verifier was written.

   📌 **And the payoff is one line.** `verify.rs:726` reads
   `InstKind::Call { dst: Some(d), .. } => vec![(*d, CTy::Void)]` — the verifier types every
   call's result as `Void` because there is nothing else to say. That line becoming the declared
   type *is* the fix; everything else is plumbing to let it.

   ⚠️ **Do not reach for a side table** (`Module::call_result_ty: IndexMap<InstId, CTy>`) to
   avoid the fixture churn. It would spare ~110 mechanical edits and create a second source of
   truth for one fact, which is §11.0's top lesson with four instances behind it. The field is
   right and the fixtures are its price.


7. ~~### **`MemFault::BadRange`**~~ — **CLOSED 2026-08-07.** See §7's entry. The two stated
   options were both wrong because the premise was: the probes did not need an *objectless*
   fault, only two findings agreeing on `object`, which a **shared** object gives just as well.
   Both mutants confirm the replacements. ⚠️ The lesson generalises past this item —
   **when a blocker is stated as "we need a thing of kind X", check whether the requirement is
   X or the property X was being used for.** Two waves were spent hunting for an objectless
   non-fatal fault that does not exist.


8c. ✅ **CLOSED 2026-08-09, the same day the gate that found it was built — 22.7 s → 7.9 s.**
   Two O(n²) scans in lowering, both item 5b's shape:

   | | |
   |---|---|
   | `emit` | ran **once per instruction** and did `blocks.iter_mut().find(\|b\| b.id == cur)` to locate the current block — O(instructions × blocks), both growing with the function. **The dominant cost: 4 of 7 samples.** Now an `IndexMap<BlockId, usize>` kept in step at the only two places `blocks` changes shape |
   | `reachable_from` | `Vec` + `contains` for `seen`, a per-block `find` for the lookup, and the caller's `retain(\|b\| keep.contains(…))` — quadratic three ways. Sets and an index now |

   | | before | after |
   |---|---|---|
   | one 32 768-statement function | 22.671 s | **7.876 s** |
   | `lower`, ratio per 4x step | 11.2x | **7.7x** |

   ⚠️ **`reachable_from` alone was 5%, and it was fixed first on one stack sample.** A single
   sample is a share of unknown size; the seven-sample profile is what named `emit`. Second time
   in one day that one sample pointed at a real but minor cost — the lesson was already written.

   📌 **5b's audit would not have found the dominant one.** `emit`'s scan is `.find(…)`, not
   `.contains(&)`, and 5b's per-crate census **omits `chiero-lower` entirely** — the crate has
   four `.contains(&` sites nobody counted. The audit is narrower than its own class *and* its
   census is incomplete; the working method remains sampling.

   ✅ **Then the verifier, profiled properly rather than sampled once.** Seven samples after the
   lowering fix put verify at **4 of 7**, `check_structural_identity` the largest single frame.
   Two more `iter().any(..)`-inside-a-loop scans there: the `AddrOfLocal` check ran over every
   alloca for **every instruction** (the hot one), and rule 13's half scanned every instruction
   per dynamic-extent alloca (latent, same shape, fixed together). **7.876 s → 5.872 s**, so
   **22.671 → 5.872 overall, 3.9x**; `verify`'s ratio 7.3x → 6.9x.

   ✅ **And then sema, the last stage the gate named.** `ScopedTypes::get` walked every name in
   scope on **every lookup**; `ScopedMeanings::declare` scanned the whole innermost scope on
   **every declaration**, which at file scope is the entire program. Both carry a name-keyed
   index now. `sema` 7.0x → **4.4x, near linear**.

   | | 32 768-statement function | `lower` | `sema` | `verify` |
   |---|---|---|---|---|
   | before | **22.671 s** | 11.2x | 6.3x | 6.7x |
   | after | **3.761 s (6.0x)** | 5.7x | **4.4x** | 6.9x |

   📌 **Seven instances of this one class in a day** — `parse_model` (5c), `emit`,
   `reachable_from`, the verifier's two, sema's two. **Five of the seven are
   `.find(..)`/`.any(..)`/`.split(..)`, which item 5b's `.contains(&` audit cannot see**, and
   two are in `chiero-lower`, a crate its census omits. The audit's number measures neither the
   class nor the codebase; **sampling under a size axis found all seven.**

   ✅ **`set_term_at` too** — 3 of 8 samples at 98 304 statements, doing the identical scan
   `emit` had just lost. Fixed `emit`, never looked at its sibling; the measurement found it and
   memory did not. **25.717 s → 20.105 s** at that size.

   ⏭️ **What is left, with the evidence for each.**

   | | |
   |---|---|
   | `verify::dominators` | **4 of 8 samples**, ~half the remaining time. Iterative dataflow with explicit dominator *sets*: `sorted_ids.clone()` per block is O(B²) in memory alone — at 98 304 statements that is ~24 576 blocks and ~600M entries. The fix is Cooper–Harvey–Kennedy idom, a real algorithmic change inside a verifier, **not** a scan-to-set swap |
   | `verify::check_ssa_and_types` | 1 of 8, unexamined |
   | five more `blocks.iter().find(\|b\| b.id == …)` | `chiero-cir:615`, `chiero-opt:157`, `chiero-exec` 3498 / 4427 / 6959. ✅ **Measured and cleared** — see §7.28: the engine *is* superlinear without z3, and none of these is why |


9. **`:0` bit-fields in `layout`, deliberately left open** (§7.9). ✅ **Priority settled
   2026-08-09 — leave it open, and now on evidence rather than on a 69-header sample.**
   Two measurements, both cheap:

   - **The cost, from `fixed_diff.py`** (which was verified working the same day and is exactly
     this gap's instrument): on its four fixed cases chiero proposes nothing for three, and for
     **two of those gcc's best permutation is 4 bytes smaller** — `Q_two_zero_width_runs` 12 vs
     8, `trailing_zero_width` 16 vs 12. The fourth, `no_zero_width`, gets a correct floor of 8.
     So the gap is real and its size is 4 bytes on the shapes that have it.
   - **The reach: VPP contains no zero-width bit-field at all.** Not "none in 69 headers" —
     `grep -rE '^[[:space:]]*[a-z_0-9 ]+:[[:space:]]*0[[:space:]]*;'` over all of `src/` returns
     **zero** files (measured while explaining why the pinned-40 retake could not move, §7.21).

   **So the only consumer that would benefit cannot reach the construct.**

   ✅ **And the grading path is already built, which was worth checking rather than asserting.**
   The first version of this entry named the generated *layout* gate as the corpus that could
   grade a fix; that was the wrong instrument — it compares sizes and offsets, not proposals.
   The right one is `floor_diff.py`, and measured directly: of 400 generated records **305
   contain a `:0`, and chiero proposes for exactly 0 of them.** They are silently skipped today
   for want of a proposal, and the moment one is emitted all 305 flow into the permutation
   oracle and get graded. **A fix would arrive with 305 checks behind it on the first run** —
   nobody has to build anything first, which is the only part of "worth doing" that was
   genuinely unknown. A record declaring a
   zero-width bit-field still gets no padding number, because a `:0`-terminated run's cost depends
   on where the run is placed and this arithmetic sums constants. Closing it needs the run's
   allocation unit in the field description — `Field` would carry `unit_bits`, and the ideal
   layout would charge the run `round_up(payload, unit)` at a unit-aligned offset. **Worth doing
   only if `:0` turns out to be common**; a sweep of 69 VPP headers found none.

## `5b` — the scan-in-a-loop audit, as it stood before 2026-08-09

Superseded: the live entry in [HANDOFF.md](HANDOFF.md) §9.1 states the corrected position.
Kept because the per-crate counts and the reasoning about *which* sites are ranges rather
than scans are still the only survey anyone has done.

5b. 🆕 **Audit `Vec` + `.contains()` on paths that scale — the shape, not the site.** The
   verifier fix above is the **second** time this exact defect class has been found in
   `crates/chiero-cir/src/verify.rs`. Seven hundred lines above `dominators`,
   `check_module_identity` already carries: *"Sets, not vectors. These were `Vec` with
   `contains`, which is O(n^2) — invisible while a module held dozens of entities… one measured
   673 s against ~1 s before. The scaling was the giveaway."* Methodology and all. **The fix went
   to the function where the symptom appeared and its neighbour in the same file had the same
   flaw.**

   `grep -rn "\.contains(&" --include=*.rs crates/*/src xtask/src | grep -vE "IndexSet|IndexMap|BTreeSet|HashSet|BTreeMap|HashMap"`
   returned **87** sites when the audit was written; **79 at `ee1b251` (2026-08-08)**, after the
   `chiero-cir` half closed and four `chiero-gcov` sites became `IndexSet`/`IndexMap`. By crate
   now: `chiero-cir` 10, `chiero-sema` 9, `chiero-gcov` 9, `chiero-exec` 6, `chiero-tool` 5.

   ⚠️ **The count is a rough progress marker and nothing more.** Today's 3.08x in `chiero-gcov`
   came from two of those sites — and three *other* `contains` conversions in the same file moved
   the ratio by nothing at all. **The grep finds the shape; only a curve finds the cost.** Most are ranges (`(0x300..=0x36F).contains`) or genuinely small fixed
   lists; the dangerous ones are where the receiver **grows with the input** and the call is in a
   loop. Spot-checked `chiero-gcov`'s four (`note_test`, `note_variant`, the per-line dedupes):
   all bounded by *test count* rather than line count, so O(T²) at worst and not obviously the
   next 673-second bug — ⚠️ but that is a reading, not a measurement, and this file's record on
   readings is poor. **Do it with a growth curve** (`/tmp/benchdom`-style: time the operation at
   10/40/160/320/640 and look at the ratio per doubling — 4x is quadratic, 6x is worse), because
   the ratio is what makes it undeniable and a single timing never is.

   **Triage done 2026-08-07; the interesting half is blocked.** By crate: `chiero-cir` 15,
   `chiero-gcov` 13, `chiero-sema` 9, `chiero-exec` 6. Most hits are ranges or fixed lists. The
   two that look genuinely quadratic are both in `chiero-gcov/src/native.rs`, and both scale with
   **arcs or block-lines per function**, not with test count:

   ✅ **All three below were converted 2026-08-08 — and the reading was two-thirds wrong.** The
   first two moved the growth ratio by *nothing* (14.7x → 15.4x); the third, dismissed here as
   merely "adds a factor", was the whole cost, and fixing its `bs` and `blocked`/`block_lists`
   membership gave **17.31 s → 5.61 s (3.08x)**. Kept unedited below because the misranking is
   the lesson: **the two that looked quadratic were not, and the one described in passing was.**

   - **`native.rs:1642`** — `slot.contains(&(key.clone(), bl.block))` inside `for bl in &f.lines
     { for line in &bl.lines { … } }`. `FuncKey` holds two `String`s, so **every probe allocates
     twice** purely to compare, and the enclosing `entry((bl.file.clone(), *line))` clones a third.
   - **`native.rs:1656`** — `order.contains(&(a.from, a.to))` while `order` is being filled from
     `f.arcs`, i.e. O(arcs²) per function.
   - Johnson's circuit enumeration (`native.rs:1258–1286`) uses `Vec::contains` for `bs`,
     `blocked` and `block_lists`, which adds a factor to something already expensive.

   ✅ **`chiero-cir`'s half is DONE (2026-08-07) and it took two passes, which is the lesson.**
   The first removed `dominators`' scan: 3001 blocks 11.5 s → 270 ms, and I called it fixed.
   **The ratio had not moved** — still ~4x per doubling — so only the constant had. Reading the
   file for the *shape* then found **seven more**, including `check_phis` rebuilding a
   predecessor map per block, which is the identical defect to `dominators`' one function away.
   `reachable_blocks` returning a set fixed three at once; a linear `Function::block` find was
   an eighth, hiding behind a method call rather than behind a `contains`. 30721 blocks: hours
   → **2.4 s**.

   ⚠️ **And it is still quadratic** — 4x blocks, ~15x time. The scans are gone; what remains is
   `dominators` holding an explicit dominator *set* per block, O(blocks²) by construction.
   Cutting it needs Lengauer-Tarjan's idom-only form or bitsets: **a design change, queued not
   claimed.** Worth doing only if a real VPP function turns out to be large enough to care.

   🆕 **UNBLOCKED 2026-08-08, and the triage below was wrong about where the cost is.**
   The artifact block was false: a `.gcno` need not be found or hand-written — **gcc emits one of
   any size from generated C**. `if (x == i) r += i;` repeated *n* times gives Θ(n) blocks and
   Θ(n) arcs in one function, and running the binary writes the `.gcda`. The whole instrument is
   `crates/chiero-gcov/tests/growth.rs`, committed and `#[ignore]`d.

   **Measured — native arc ingest is quadratic** (4x per 4x arcs is linear, 16x is quadratic):

   | | 50→200 | 200→800 | 800→3200 |
   |---|---|---|---|
   | before | 6.3x | 12.2x | **14.7x** |
   | after the three `contains` fixes | 3.3x | 11.1x | **15.4x** |

   ⚠️ **The three sites named below are not the bottleneck.** They are `IndexSet`s now (commit
   `23ba416`) — strictly better, no scan and no clone-per-probe — and **the ratio did not move**.
   That is this file's own warning landing on the person who wrote it down: the CIR verifier entry
   two paragraphs up says *"the ratio had not moved, so only the constant had"*, and here not even
   the constant moved.

   🆕 **And then the curve itself turned out to be the problem.** The generator put one statement
   per source line, so every line carried one block. Adding a second shape — all statements on
   *one* line, which is what a multi-statement macro expansion produces and VPP is macro-heavy —
   changes the answer completely:

   | shape | 200→800 | 800→3200 | n=3200 |
   |---|---|---|---|
   | `line` (one statement per line) | 11.5x | 16.4x | 1.10 s |
   | `onelin` (all on one line) | 21.2x | **50.5x** | **17.1 s** |

   **A growth curve is only as good as the shape it grows.** A generator that varies one parameter
   while holding the interesting one at 1 reports a clean answer forever — and three wrong
   conclusions came out of exactly that below.

   It also **rehabilitates the hypothesis dismissed with a bad argument**: `cycles_count` was ruled
   out because the generated code has no loops, but its cost is `for &start in bs { circuit(...) }`
   — one DFS per block *on the line*, which runs whether or not a cycle exists. It is the only
   thing in the file that scales with blocks-per-line. **Chase the `onelin` curve.**

   ✅ **Diagnosed with a counter, and the first real win landed 2026-08-08.**
   `native::circuit_starts()` counts every `circuit` entry, recursion included:

   | shape | n=200 | n=800 | n=3200 | growth |
   |---|---|---|---|---|
   | `line` | 405 | 1 605 | 6 405 | 4x — linear |
   | `onelin` | 20 504 | 322 004 | **5 128 004** | 16x — **quadratic** |

   ⚠️ The counter **refuted its own first placement**: counting only the outer `for &start in bs`
   loop gave 6405 for *both* shapes while one ran 17x slower. The cost is not how many traversals
   begin, it is how far each walks.

   **Fixed so far:** `bs.contains(&w)` — a linear scan in the innermost recursion — is now a
   `Vec<bool>` indexed by block. **17.31 s → 8.36 s at n=3200 (2.07x)**, ratio 50.3x → ~38x, call
   count unchanged as it should be. *(clippy then caught that `bs` was being passed through
   `circuit` only to reach its own recursive call. Dropped.)*

   ✅ **Second fix, same day:** `blocked`/`block_lists` were index-correspondent parallel `Vec`s,
   so lookup was `iter().position(..)` and release was two O(n) `remove`s. One
   `IndexMap<u32, Vec<u32>>` with `swap_remove` replaces both. **8.36 s → 5.61 s.**

   **Cumulative: 17.31 s → 5.61 s (3.08x)**, call count untouched at 5 128 004 throughout — which
   is the check that both were cost-per-call changes and not accidental semantic ones, and the
   full suite agrees.

   ✅ **Third fix, and it is the algorithmic one: skip the enumeration when the induced subgraph
   is acyclic.** `cycles_count` started a DFS at *every* block in `bs` whether or not a circuit
   existed; Kahn's algorithm answers that in O(V+E) once. **5.61 s → 1.05 s**, circuit calls
   **5 128 004 → 0**.

   **Cumulative: 17.31 s → 1.05 s (16.5x)**, and the blocks-per-line pathology is *gone* — the two
   curve shapes differed by 17x in the morning and now sit within noise of each other at ~1.1 s.

   ⚠️ **Not "the enumeration is fixed".** The curve's input is straight-line, so the early-out
   fires everywhere and the counter reads 0. **A function with a real loop still pays the full
   O(V × (V+E))** — what changed is that straight-line code no longer funds a search that cannot
   succeed. The cycle path stays covered by the `cyc.gcno` fixture.

   **Still open:** ~14x per 4x arcs against a linear 4x, so `crates/chiero-gcov/tests/growth.rs` still fails on
   purpose, and **the remaining cost is unlocated**.

   ⚠️ **Tested and ruled out after the early-out landed:** the `accumulate_line_info` predecessor
   hoist was re-applied on the theory that `cycles_count` had been masking it. It changed nothing
   again (1.00 s / 1.07 s against 1.05 s / 1.15 s — noise) and was reverted a second time. So the
   scan genuinely is not the cost, at either scale.

   ✅ **LOCATED 2026-08-08 — `solve_arcs`' conservation fixpoint**, by counter
   (`native::conservation_arc_visits()`), not by reading:

   | n | 50 | 200 | 800 | 3200 |
   |---|---|---|---|---|
   | arc visits | 254 992 | 3 898 192 | 61 670 992 | **983 962 192** |
   | ratio per 4x arcs | | 15.3x | 15.8x | **16.0x** |

   Quadratic, and the **wall-clock ratio is also 16.0x** — the counter tracking the clock is what
   makes this the answer rather than a sixth plausible site. It is identical for both curve
   shapes, which fits: once the acyclic early-out landed, the remaining cost stopped caring about
   blocks-per-line.

   ✅ **FIXED the same day.** Incidence lists built once instead of `(0..n).filter(..)` per block,
   per side, per iteration:

   | | before | after |
   |---|---|---|
   | conservation arc visits, n=3200 | 983 962 192 | **153 728** |
   | ratio per 4x arcs | 16.0x | **4.0x — exactly linear** |
   | `onelin` n=3200 | 1.05 s | **0.090 s** |

   **Cumulative across the four fixes: 17.31 s → 0.090 s, 192x.** Order-preserving by
   construction — `(0..n).filter(|i| arcs[i].to == b)` *is* the arcs into `b` in ascending index
   order — so the conservation arithmetic never changes, only how often the graph is re-derived.

   ⚠️ **I had marked this ⛔ "for its own wave" one commit earlier and then did it anyway.** The
   caution was about my remaining context, not about risk: fifteen lines, order-preserving, and
   the 2249-test suite is precisely the watch I said it needed. Worth noticing which of those two
   things a ⛔ is actually recording — **"I am nearly out of budget" and "this is dangerous" are
   different facts and only one of them should outlive the session.**

   ✅ **Fifth fix — and it is the change reverted twice earlier the same day.** With
   `cycles_count` and the conservation fixpoint both gone, `accumulate_line_info`'s predecessor
   hoist finally shows an effect: `line` 10.5x → **8.6x**, `onelin` 10.0x → **7.6x**.

   ⚠️ **A null result is scoped to the conditions it was taken under.** "This change does nothing"
   was measured honestly, twice, and was true both times — it stopped being true when the costs
   hiding it were removed. **Re-test reverted optimisations after the dominant cost moves.** The
   curve makes that a 25-second question rather than an argument.

   **Session total on this item: 17.31 s → ~0.068 s, ~250x across five fixes** (two runs;
   ±20% variance at these times) — four of which
   would have been got wrong by reading.

   **Still open, and the number is bigger than it first looked.** The curve now runs to
   **n=12800**, because at n=3200 the ingest had dropped to ~0.1 s — where process startup, file
   I/O and gcc's output size are a visible share of the clock, and a "ratio" is partly noise. The
   added point says the residual is real:

   | shape | 3200 → 12800 | |
   |---|---|---|
   | `line` | 0.109 s → 1.390 s | **12.8x** |
   | `onelin` | 0.082 s → 0.880 s | **10.7x** |

   Roughly n^1.8 against a linear 4x, while the conservation counter stays **exactly linear**
   across the same step (153 728 → 614 528) — so the fix that landed earlier still holds at four
   times the size, and the residual is elsewhere.

   ⚠️ **An instrument that has stopped discriminating still prints numbers**, and they look just
   as authoritative. This curve had stopped growing past the point where its subject dominated the
   clock — the same failure as the input-shape blindness in its own header, twice in one file.

   ⚠️ **Tested and ruled out: `acc.shift_remove(key)`** at `native.rs:1023`. It sits inside
   `for (key, bs) in &on_line` and `shift_remove` is O(n) on an `IndexMap`, so it reads as a
   textbook quadratic — Θ(lines) × O(|acc|). Swapping it for `swap_remove` moved the **ratio**
   not at all (`line` 12.8x → 12.6x, `onelin` 10.7x → 13.3x, i.e. noise) though it did cut ~30%
   of the constant. **Reverted un-shipped**, because a constant-factor win is not worth changing
   `accumulated`'s iteration order, which feeds downstream merges. *Measured as an experiment and
   thrown away — that is the cheap way to hold an opinion.*

   ⚠️ **Also ruled out: `cycles_count`'s per-call allocations.** It is invoked once per *line*
   — Θ(n) times — and each call sized `in_bs` and `indegree` by the **max block index**, i.e.
   Θ(n), for Θ(n²) in allocation alone. It also predicted `line` being worse than `onelin`, which
   is what the curve shows. Replacing both with |bs|-sized structures moved nothing (13.7x /
   10.9x against 12.8x / 10.7x). Reverted; the experiment was deliberately semantically wrong for
   cyclic input and existed only to answer the question.

   ⚠️ **The elimination above was wrong, and measuring it took two minutes.** `records()` and
   `read_notes()` are both public, so the curve now times them separately — no change to the
   solver to get the numbers:

   | phase | growth per 4x arcs | share of the 1.37 s at n=12800 |
   |---|---|---|
   | `records()` byte decode | 4.4x — **linear** | 0.4% |
   | `read_notes()` structure build | 3.9x–5.1x — **linear** | 1.5% |

   Conflating those two is what sent the elimination astray: "the parse" was one name for two
   different amounts of work, and both turned out innocent.

   **So every component measured is linear and the whole is still ~13.5x.** The `.gcda` decode was
   split out too (linear, 3.5–4.8x); at n=12800 **all decode plus structure build is under 3% of
   the clock**, leaving ~97% in the post-decode pipeline.

   ✅ **One genuine quadratic found there and fixed:** `cycles_count` is invoked once per *line*
   and sized `in_bs`/`indegree` by the **max block index** — 327 808 014 cells at n=12800, 16.0x
   per 4x arcs. Scratch is now allocated once per function and reset over `bs`; the counter reads
   **4.0x, linear**.

   ⚠️ **And the wall clock did not follow — 1.39 s → 1.32 s, ratio 13.5x either way.** That is the
   more useful result: **a quadratic counter is not automatically the bottleneck.** 328M bool
   writes is real, really was quadratic, and is also just a memset — perhaps 7% of the run. Kept
   for the asymptotics; it would dominate at larger inputs.

   ⚠️ **It had also been "ruled out" earlier the same day** by an experiment that stubbed
   `circuit`'s argument. Flagged as unclean at the time, re-tested, and the hypothesis was right.

   ✅ **LOCATED AND FIXED 2026-08-08 (second session) — and the suspect named below was wrong.**
   Not by another counter: by **splitting the clock**. `arc_coverage` runs the whole ordinary line
   ingest (`ingest_into`) and *then* walks the functions again for the arc index — "the post-decode
   pipeline" was one name for two amounts of work, the same conflation that sent the parse
   elimination astray one measurement earlier. Timing them apart put **90% of the clock on the line
   half**, i.e. the opposite side from the `ArcCoverage` index building this paragraph nominated.

   Two throwaway experiments then bisected it in four minutes — skip `line_counts` + `object.add`
   (ratio barely moved), then also skip `block_counts` (15.4x → 4.9x):

   | fix | what it was |
   |---|---|
   | `block_counts` | every block scanned every arc — Θ(blocks × arcs), **90% of the ingest** |
   | `acc.shift_remove(key)` per graphed line | O(\|acc\|) each; now one order-preserving `retain` |
   | `bs.contains(&from)` in the entry-arc sum | quadratic in blocks-per-line; now an indexed bool |

   **1.42 s → ~0.24 s at n=12800**, `line` 15.4x → 6.2–7.0x, `onelin` 11.7x → 4.7–5.8x, and the
   gate passes for the first time. ⚠️ **Passing is not linear** — the `line` shape's line half is
   still 8–9.5x, in three `IndexMap`s keyed by `(String, u32)` plus the per-object merge. Do not
   tighten the 8.0x threshold: the run-to-run band is ±0.8x.

   🆕 **Narrowed 2026-08-09, and the narrowing is free: the residual is a function of distinct
   source lines *at fixed arc count*.** The two curve shapes reach n=12800 with the **same**
   arcs — `conservation 665734` for both — and differ only in how many lines those arcs sit on:

   | at n=12800 | distinct lines | line-half | its growth |
   |---|---|---|---|
   | `line` | ~12800 | 0.1684 s | **8.8x** |
   | `onelin` | ~1 | 0.0320 s | 3.4x |

   ✅ **FOUND AND FIXED the same day, by the counter this entry called for.** `cycles_count`'s
   `cs` was `vec![0; f.arcs.len()]`, allocated and zeroed on **every call**, and it is called
   once per line — Θ(n²) cells for `line`, Θ(n) for `onelin`. `in_bs` and `indegree` had been
   hoisted for exactly this reason, with a comment saying so; `cs` was left behind. **One fix
   applied to two of three buffers.**

   | `line` at n=12800 | before | after |
   |---|---|---|
   | line-half | 0.1684 s | **0.0467 s** |
   | its growth | 8.8x | **4.0x — linear** |
   | share of the clock | 69% | 44% |
   | worst overall | 7.6x | 5.9x |

   📊 **Five runs after the fix, and the picture inverts — with a consequence for the
   threshold.** A single run is not a band, and this curve's band is the whole reason §9 says
   not to tighten:

   | shape | worst per 4x, **six** runs | was |
   |---|---|---|
   | `line` | 4.1 4.7 4.1 4.5 4.5 **5.8** — max **5.8** | the bad one, 8.8x |
   | `onelin` | 4.3 4.7 5.4 **6.4** 4.1 4.8 — max 6.4 | the good one, 3.4x |

   ⚠️ **The sixth run fell outside the band the first five established, and the correction is the
   point.** After five runs this entry said `line` was *"essentially linear, 4.1–4.7"*; run six
   gave **5.8x**. So the honest statement is a range with its sample size attached — `line`
   4.1–5.8 over six — not a claim of linearity. **Five samples were not enough to bound a
   quantity whose band is ±1x**, which is the same over-reading as concluding from one run, at a
   larger sample. Both shapes still pass 8.0x with room, and `line` is still transformed from
   8.8x; that part holds.

   ✅ **And the gate is measuring the right thing — checked, because the wide band suggested it
   might not be.** The worry was that "worst per 4x" might be coming from the noisy small-*n*
   steps, which would make the headline a measure of jitter rather than of growth. Every step,
   one run:

       line     3.0 -> 3.5 -> 3.9 -> 4.4      onelin   2.9 -> 3.6 -> 4.8 -> 3.9

   **The small steps are consistently the tamest**, and the worst comes from one of the two
   largest — where it should. So the band is wide because the *large-n* measurement is noisy on
   a loaded machine, not because the gate is reading the wrong end of the curve. An honest zero
   on the hypothesis, recorded so the next reader does not re-suspect it. ⚠️ So
   **"do not tighten the 8.0x threshold" still holds, for a completely different reason than
   when it was written**: it was `line`'s superlinearity, and it is now `onelin`'s run-to-run
   noise, which touches 6.4x. Tightening to 7.0x would sit 0.6x from an observed value. **A
   conclusion outliving its premise is the thing this session kept finding; here the conclusion
   survived the premise changing, and the reason had to be rewritten under it.**

   📌 And a false alarm pre-empted: a future reader seeing `onelin` at 6.4x should not chase it
   as a regression. The band above is what one machine produces with nothing changed.

   ⚠️ **The counter is why it survived two sessions.** `CYCLES_CELLS` counted the two hoisted
   buffers and not the one still sized per call, so it read *"4.0x, linear"* while the clock
   stayed superlinear — **a wrong measurement that agreed with the fix already made**, which is
   the worst kind. Adding the missing term made it 15.9x on `line` against 4.0x on `onelin` and
   located the defect in one run. *A counter that omits a term is not a smaller measurement, it
   is a wrong one.* The confirmation is stronger than the timing: the cells counter is now
   **identical for both shapes** (176138 → 704138), so the line-count dependence is gone rather
   than reduced.

   *The narrowing that led there, kept because it was free and did the work:*
   **5.3x more time for identical arc work, and the superlinearity is entirely on the `line`
   side.** That rules out everything scaling with arcs, blocks, or blocks-per-line — which is
   most of what the earlier waves chased — and leaves only what is done *per distinct line*.
   ⚠️ Note it also rules out the obvious reading of the suspect already recorded here: an
   `IndexMap<(String, u32), _>` over one filename hashes and clones a string per operation,
   which is **linear** in the line count, not 8.8x. So the recorded suspect is at best the
   constant, and something per-line is O(lines²)-ish. **Next step is a counter on the per-line
   operations, not a reading** — the curve makes each hypothesis a 25-second question, and this
   entry's own scoreboard is 6 refuted / 5 held.

   ⚠️ **The middle fix is the one to remember: it had been measured and honestly ruled out earlier
   the same day**, and that null result was *true* — while `block_counts` was 90% of the clock,
   nothing else could move the ratio. **Re-test reverted optimisations after the dominant cost
   moves**, which this curve makes a 5-second question.

   *Original text, kept because it is the seventh refuted hypothesis on this item:* **Still
   unlocated: the time residual.** The next counter must measure something whose unit
   tracks *time* — allocations, hash lookups, `IndexMap` probes — in the `ArcCoverage` index
   building (`line_blocks`, `counts`, `tests`, `order`, each keyed by a `FuncKey` holding two
   `String`s and cloned per insert). That is the largest block still measured only as part of a
   whole.

   *Method note, and it is the cheap thing to copy:* every hypothesis here cost about a minute —
   change it or time it, run the 25-second curve, revert regardless of the result. **Scoreboard:
   6 refuted, 5 held.** Nothing was shipped on a reading, and the six refutations include three
   that predicted the observed shape correctly and were still wrong about the cause.

   Scoreboard on this entry: **4 hypotheses wrong, 5 right.** Every wrong one looked obvious in
   the source; every right one came from a counter or a curve.

   ⚠️ *Kept below: three hypotheses that were tried first and moved nothing.* **Do not read.
   Profile.**

   | hypothesis | how it looked | ratio after |
   |---|---|---|
   | the three `Vec::contains` sites §9.1 named | a scan inside a loop | 15.4x (was 14.7x) |
   | Johnson's circuit enumeration, ~1250–1290 | `Vec` membership in the innermost recursion | not the path — the curve's input has **no loops at all**, so there are no cycles to enumerate |
   | `accumulate_line_info`'s arc scan, ~980 | `for (key, bs) in &on_line { for &b in bs { for a in f.arcs` — textbook lines×blocks×arcs | 15.4x, unchanged; **reverted** |

   Each looked obvious. Each was wrong. The third was hoisted into a predecessor map built once
   per function — the exact fix that took the CIR verifier from hours to 2.4 s — and it changed
   nothing measurable, so it was reverted rather than left in a numerically sensitive solver as
   an unproven edit.

   ✅ ~~**What is actually needed is a profiler**~~ — **written mid-investigation and wrong within
   the hour.** `ptrace_scope=1` really does block attaching to a running `cargo test` binary, the
   recorded gdb recipe needs the target to be gdb's own *child*, and `perf` is not installed — all
   true, and none of it mattered. **A counter settled it in one edit**
   (`native::circuit_starts()`), exactly as `verify::terminators_examined()` had settled the
   verifier, and the same sentence that reached for a profiler already said so.

   Kept as a correction rather than deleted, because the reflex is the thing to notice: **"I need
   a better tool" is usually cheaper to answer with "I need a number".** A profiler tells you
   where time went; a counter tells you *why*, survives a faster build, and goes in the repo where
   the next reader gets it for free.

   ⛔ *Original entry, kept because its reasoning is the thing that turned out to be wrong:*
   **`chiero-gcov`'s half is blocked on artifacts.** There are **no
   `.gcno` files under `/home/ubuntu/vpp`** — the 1895-file validation in §7.1 was a one-off
   against a coverage build that no longer exists. Without it there is no growth curve, and this
   entry's own rule says a reading is not a measurement. Two honest ways forward: rebuild VPP with
   coverage (long), or write a synthetic `.gcno` generator and curve it the way `dominators` was
   curved (bounded, and reusable afterwards). ⚠️ **Do not "just fix" the clones** — an unmeasured
   optimisation is the flattering change this file keeps warning about, and `chiero-gcov` is
   19/19 contracts green today.

   ✅ **`max_solver_rlimit` is BUILT, 2026-08-07.** `Budget::max_solver_rlimit` reaches the backend
   as `(set-option :rlimit N)`; a query that spends it answers `Unknown(ResourceLimit)`.
   `Engine::new_solver` is the single construction point, so it covers feasibility *and* checker
   queries — a budget that applied to one and not the other is not a budget.

   **The defect it uncovered is worth more than the feature.** `query` returned
   `Option<(bool, Model)>`, so a solver saying `unknown` and a broken pipe were the same `None`,
   and `ask_backend_raw` treats `None` as died-mid-query: it **replayed the whole query** and
   reported `BackendError`. So the hardest queries in a run — the only ones that answer `unknown`
   — were charged twice, and 022 contract 15's `backend_errors` counted a backend that was
   behaving correctly. `Answer` is a three-valued type now so the two cannot share an arm again.

   ⚠️ **And the mutation pass caught me shipping the exact confusion the tests' own header
   describes.** With the first three tests, `if true` in place of the classification guard —
   making *every* `unknown` a `ResourceLimit` — **survived all of them**. Closing it needed a
   fake solver answering `unknown` with a chosen reason; z3 cannot be made to decline a theory
   on demand, and that is a property of the z3 build rather than of chiero.

   ✅ **`--solver-rlimit` shipped 2026-08-07**, on `find-bugs`, `check-reachable` and
   `prove-equivalent` — and writing it found that the wave above had reached **one of three**
   solver construction sites.

   ⚠️ **The commit that built the budget claimed `Engine::new_solver` was "the single
   construction point" and invoked *fix the rule, not the site* — while missing two sites and a
   whole command.** A run builds a solver in three places: `Engine::new_solver` (feasibility and
   checkers), `chiero-tool::witness_for_path` (`check-reachable`'s witness, built *outside*
   `Engine` because a state that merely arrived carries no finding), and `chiero-opt::equiv`.
   Only the first was wired, and the CLI never set the equivalence budget at all. **Saying "one
   construction point" is not the same as making one** — `grep -rn "TieredSolver::" --include=*.rs`
   is the four-second check that settles it, and it was not run.

   The defect was invisible to every envelope field. What found it: a recording script as
   `$CHIERO_SMT_SOLVER`, showing `(set-option :timeout 9000)` on the wire and **no `:rlimit`**.

   **Two fixture traps worth keeping, both of which make a budget test vacuous:**
   - **`x * 2` against `x << 1` — 041's own headline example — never reaches a backend.** Tier 1
     settles it. A test built on it passes whatever the plumbing does. Count dumped queries
     (`CHIERO_DUMP_QUERIES`) before believing a solver fixture exercises a solver.
   - **At `:rlimit 1` z3 cannot even run `(push 1)`**, and emits an `(error …)` line that chiero
     reports as "backend gave no usable answer". Honest, and a different sentence from the one
     under test. Use 2000.

   **Both leftovers are closed.** `max_memory_objects` shipped the same day, and the plugin
   sweep's `timeout` rows were re-measured — they had nothing to do with the solver, which is
   the entry two above this one.

   *The measurements that shaped it, kept because they are about z3 rather than about chiero:*
   `UnknownReason::ResourceLimit` existed and was **constructed nowhere**; nothing read
   `(get-info :reason-unknown)` at all. What the real solver does:

   | asked | answered |
   |---|---|
   | `(set-option :rlimit 1000)` on a hard `bvmul` | `unknown`, `(:reason-unknown "max. resource limit exceeded")` |
   | the same at `:rlimit 100000000` | `sat` — so the bound is what cut it, not the formula |
   | a hard query, then a trivial one, **one process** | `unknown` then `sat` — **`:rlimit` is per-`check-sat`, not cumulative.** This is the property that makes it usable at all: chiero keeps one long-lived process, and a cumulative budget would poison every query after the first expensive one |

   ⚠️ **And the trap, which the obvious implementation walks straight into.** The documented
   string only appears with the assertion stack at top level. **Inside `(push)`/`(pop)` — which
   is how chiero *always* drives z3, since `Solver` has `push`/`pop` — the same exhaustion
   reports `"canceled"`.** Worse, a `:timeout` firing under `push`/`pop` reports `"canceled"`
   **too**, byte for byte. Measured both ways round.

   So `(get-info :reason-unknown)` **cannot distinguish a resource limit from a timeout in the
   mode chiero runs in**, and an implementation that matches `"max. resource limit exceeded"`
   passes a hand-written smoke test and misclassifies every real query. The design that follows:
   **infer `ResourceLimit` from which budget was armed, not from the string** — when
   `max_solver_rlimit` is set, do not also arm `:timeout`. That is what 023 §8.1 already wants
   anyway (CI runs the determinism gates with `wall_clock: None`), so the constraint and the
   spec agree. **That is what shipped**, and `Session::spawn` is where the exclusivity lives.
   One more measurement completes it: genuine incompleteness reads
   `"smt tactic failed to show goal to be sat/unsat (incomplete (theory arithmetic))"`, never
   `"canceled"` — so the string separates *a limit* from *a theory declined*, and only the
   armed budget says which limit.

## §7 records moved 2026-08-09

Referenced once or not at all from the live handoff, and each is a finished story. Moved so
§7 reads as *what is built and what is currently known to be wrong*, rather than as a
chronology. The section numbers are unchanged, so an old citation still finds its subject.

### 7.2 `prove_equivalent` — built 2026-08-05, and what is left of it

`crates/chiero-opt/src/equiv.rs`. Relational (product) execution per 041 §1.2: both versions
run on **one shared `TermArena`**, every terminated path of `before` is paired with every
terminated path of `after`, and each pair is conjoined with an explicit equality per matched
entry parameter. `TermArena::var` mints a fresh `VarId` per call, so "the same symbolic
inputs" is *imposed* rather than assumed — which is the useful accident, because it makes the
matching visible: an input with no counterpart is a refusal (`Unknown`), never a zero.

**The witness is minimized by binary search, not taken from the first `Sat`.** Contract 13
wants the swapped argument order to give a correspondingly swapped witness, and two queries
differing only in which side minted variable 0 may legitimately return different models — so
a first-`Sat` witness makes the contract a coin flip. Minimization is canonical, reproducible
(001 §5), and "the smallest input that distinguishes them" is a better thing to hand a reader.

**Eight flattering failures found and fixed, all the project's recurring shape.** Two by
asking what the pairing loop does with nothing to iterate over; six by an adversarial `fable`
review, every one of which reproduced. `crates/chiero-opt/tests/adversarial.rs` holds them.

| what was blessed | verdict it got |
|---|---|
| `g = x; return 0` vs `return 0` | `Equivalent { Exact }` |
| a volatile store vs no store | `Equivalent { Exact }` |
| a dropped unmodeled extern call | `Equivalent { Approximated }` |
| `max_forks = 0` / `max_states = 1`, no loop, disagreeing on 2^32-1 inputs | `Equivalent { Bounded }` |
| every path budget-cut | `Equivalent { Bounded }` |
| a pair with one side cut | `Differs { Termination { Return, Budget } }` |
| termination differing at exactly `{(0,200), (3,7)}` | witness `(0, 7)`, where both return 32 |
| the same as a return difference | `Unknown`, with a real model thrown away |

**A third review found six more, and the finding that matters is not any of them: two were
earlier defects back through a different door.** Each earlier fix had been attached to the
*site* where the defect was demonstrated rather than to the level the rule lives at.

| what came back | how |
|---|---|
| a truncated search is not a proof | the screen lived in `blessable`'s `Bounded` arm; one unmodeled call degrades the run to `Approximated` and the `Bounded` `BudgetHit` sails past |
| a read of caller-visible memory | the guard named `Load`; `CopyMem`'s **source** is a read too |

Both fixes moved: the truncation screen now runs over every assumption before any fidelity is
considered, and the memory guard is written about the *role* an address plays rather than the
instruction that spells it. **When a review finds a defect, the question to ask is what rule it
violates, not what line to change.**

The third new one was worse and unrelated: `malloc` is modeled, the model forks into a success
path and a NULL path on a guard nothing links between the two runs, and it *overwrites* the
extern-return symbol linking works on — so one run's success paired with the other's failure
and **a function differed from itself**. Reflexivity is the cheapest property this operation
has and nothing was asserting it. `EffectKind::ModeledCall` now refuses a modeled call rather
than aligning it, which also stops a dead `memcpy` between two locals reading as observable I/O.

**A second review, after contract 6 landed, found five more** — three of them again false
`Equivalent`, and this time the wrong reasoning was reasoning *I had written down as the
justification*:

| what was blessed | verdict it got |
|---|---|
| a global read either side of a call that may write it | `Equivalent { Approximated }` |
| `p(x)` against `p(x + 1)`, `p` declared `pure` | `Equivalent { Approximated }` |
| returning `p(2)` against returning `p(1)` | `Equivalent { Approximated }` |
| two pure calls reordered, computing the same value | `Differs`, where both return 0 |
| `memset` against `__builtin_memset`, byte-identical | `Differs` |

**The lesson is about comments, twice.** "The ordinal is the same thing the effect sequence
orders by" was false — `ExternReturn` is minted only for a call *with a destination*, so a
discarded result shifts the numbering. "Pure, therefore declared to do nothing observable" was
false — `pure` means no side effects, not a return value independent of the arguments; `abs` is
pure. Both were written in the same commit as the code they justified, and both were
convincing enough to ship. **A plausible rationale is not evidence, and writing one down makes
it harder to check, not easier.**

**The one worth remembering: the first three were already ruled out in the module
documentation, in the same commit as the code that did not do it.** *"A comparison that
would have to reason about caller-visible memory or about a side-effect sequence answers
`Unknown` naming the claim it could not check."* Nothing implemented that sentence. A written
intention with no implementation is worse than an admitted gap — it is what a reader checks
*instead of* the code.

The two witness defects shared one cause: the minimizer fixed inputs one at a time but seeded
each from a model taken before any were fixed. Where the divergence set is not a product,
that seed is unreachable under the earlier pins. Now re-solved per input, one extra query, so
the loop's invariant is true rather than asserted.

**What the contract suite could not have caught:** its fixtures are pure, one-parameter,
branch-light arithmetic — no global, no volatile, no extern, no two-parameter `Differs`, and
its one budget test used `max_states = 0`, the single value where nothing finishes and the
guard fires.

**Left to build, in rough order of value:**

0. **✅ DONE — `chiero-cli`, 2026-08-05.** Five operations from a command line:
   `prove-equivalent`, `impact`, `select-tests`, `expansion-sites`, `explain-macro`, each
   printing an envelope (`--json` for the machine form). `Envelope::render` now renders a
   result as lines rather than as compact JSON, and `serde_json` gained `preserve_order` so
   `verdict` leads instead of sorting alphabetically under `replay`.

   **Every `$ chiero ...` block in the tutorials is a transcript under test**
   (`crates/chiero-cli/tests/tutorial_transcripts.rs`) and must match byte for byte. That test
   exists because I hand-wrote those blocks and every one was wrong — invented entity order,
   omitted fields, and, on the page about telling a proof from a guess, a "proven, Exact" with
   both blind spots missing.

   *Superseded — kept for the reasoning:*

   ~~**⭐ `chiero-cli` — the user asked for it, 2026-08-05.**~~ *"add the CLI to trigger all those
   great cases without the user having to do too much programming; and update the tutorials
   with how they are used."* `crates/chiero-cli/src/main.rs` is still a 5-line stub that prints
   a version. Every operation in `chiero-tool` is reachable only from Rust, so the tutorials
   teach a library API to someone who wants a command. Wanted, at least:
   `chiero prove-equivalent before.c after.c --entry f`, `chiero select-tests`,
   `chiero impact`, `chiero expansion-sites`, `chiero explain-macro`. 050 §1 says `chiero-cli`
   is "a thin wrapper over the identical" operation surface, so the shape is settled.

   Also from the same message, and already applied: **every tutorial must show the data it
   talks about.** Tutorial 4 described an LLM's rewrite in prose and never showed the `after`
   C, which is exactly the thing a reader stops to ask about. Audited all five.

### 7.3 A defect the operations found in the layer beneath them, 2026-08-06

Pointing `chiero check-reachable` at a `return` line answered *"the function has no code on
line 4"*. **015 §5's rule is written over a block's instructions, and `return <constant>;`
lowers to a terminator with no instructions at all** — so both return blocks of
`if (v) return 1; return 2;` had an empty `gcov_lines` while gcov counted both lines.

Sixteen lines across fourteen lowered goldens were missing. §5 calls `gcov_lines` "the join
point of the entire differentiating claim (030 → 031 → 032)", so every one was a line coverage
correlation could not reach. The implementation matched the spec, which makes it a spec gap
rather than a slip — and it survived a full-VPP cross-validation because that validates the
*decoder* against gcov, not the CIR correlation.

**Worth remembering as a method, not a bug:** the defect surfaced within minutes of the
operation existing, by using it on ordinary C. Nothing in the test suite was going to find it.

1. ~~**§1.3's replay harness**~~ — **built 2026-08-06.** `chiero-replay` emits a self-contained
   C program that `#include`s both versions with the entry renamed (040 §3.1's mechanism, the
   only one that reaches a `static` target), calls each at the witness, and exits 0 only when
   they disagree. `chiero prove-equivalent ... --allow-replay-exec` reports
   `outcome: demonstrated` with the two numbers **a real compiler** produced, and the standing
   "no replay harness was compiled" blind spot is removed because it is no longer true.

   **This is the first claim in the system that does not rest on chiero's own semantics.**
   `Outcome` has four values and three are ways of having demonstrated nothing;
   `not_demonstrated` is 041 contract 11's downgrade — chiero and a compiler disagree, fidelity
   drops to `Approximated`, and the verdict stays `differs` because something *is* wrong and a
   reader needs both claims. Execution is gated behind `--allow-replay-exec` (050 contract 11).

    **040 contract 4 landed 2026-08-06**: every `find_bugs` finding carries a harness, and
   `--allow-replay-exec` reports `outcome: faulted, confirms: true` for a division by zero — a
   real compiler dying on `SIGFPE` at chiero's witness. `FindingOutcome` is a different shape
   from `Outcome` because a finding has one program: it is reproduced when it *faults*, and the
   **signal** is the answer (a process killed by `SIGFPE` has no exit code at all). It reuses
   the equivalence harness's launcher rather than copying it — every piece of that machinery
   was earned by a review finding a hole, and a second copy would start again from the first.

   *Left:* the harness takes **scalar parameters only**. 040 §3's construction rules also want
   memory objects as initialized byte arrays with the engine's own pointer layout, and extern
   stubs returning the values the engine chose in call order. Neither is built, and both are
   refused by name rather than guessed at.
   *"Your rewrite is wrong" is an opinion; "here is the program" ends the discussion.* Nothing
   in the tree emits a C replay harness yet — 040 §3 wants one too.
1a. ~~**041 §2 opportunity detection**~~ — **contract 15 built 2026-08-06.**
   `chiero_opt::opportunity::detect` proposes a branch the path condition already decides,
   with the implying constraints as SMT-LIB. It reads the engine's own `Event::Fork { feasible }`
   rather than re-asking the solver — the engine has decided that question and a second answer
   would eventually disagree. A proposal from a run that did not finish is **advisory** and
   names the budget: "no state took that edge" and "no state *can*" are the same observation
   and opposite claims, and here the difference is whether somebody deletes live code.

   **Contract 14 landed 2026-08-06** — the redundant-load detector, and the contract that makes
   the obligation machinery mean something: the *observation* is identical across a callee
   chiero can see through and across one it cannot, and only the strength of the claim differs
   (all `Discharged` vs one `Open` and `advisory`).

   **It works on real C** (2026-08-06, after two wrong diagnoses). `int a = *p; quiet(a);
   int b = *p;` as gcc hands it over comes back `redundant_load` with every obligation
   *discharged*; the same function with an `extern` between comes back **advisory**.

   > **Both wrong turns were the same mistake: matching on how the CIR spelled something
   > instead of asking what it was.** The identity criterion was "the same `ValueId` loaded
   > twice", which unoptimized C never satisfies because `p` lives in a slot and is reloaded —
   > it is now the engine's own `Pointer` (object + offset), which is 021's answer rather than
   > a second one. And "a callee with no store" cleared nothing, because lowering stores every
   > parameter into a slot — it is now "a callee whose every store is into its own confined
   > local", reusing the caller-side escape check.
   >
   > I recorded the limitation as needing "redundant-load analysis one level down". That was
   > the wrong diagnosis: **the level below already had the answer and was not being asked.**

   **Dead store landed too** (2026-08-06) — `*p = a; *p = b;` proposes the first write dead,
   discharged, and a call between makes it advisory. Keyed on the engine's `Pointer` from the
   start, because the load detector had already paid for that lesson twice. **Two tables, not
   one:** a load is redundant when nothing could have *written* between and a store is dead when
   nothing could have *read* between, which are opposite questions.

   *Left:* loop-invariant computation, redundant bounds check, call-site specialization,
   unreachable code, unnecessary zeroing.

1b. ~~**041 §3 locality**~~ — **built 2026-08-06.** `chiero_opt::locality`: line straddling
   (contract 18's boundary both ways), padding waste with the byte delta, hot/cold placement.
   Contract 21's advisory rule and contract 22's honest labelling are most of the module —
   `advisory` is *derived* from the obligations, `Benefit::Estimated` is in the enum and never
   produced (no cycle model, and §3 says not to pretend), and `Measured` is reachable only from
   real counts. The layout arrives as data: 014 §3 computes it and is measured against gcc, so
   re-deriving it here would be a second answer.

   *Left:* contracts 19, 20, 23 need the `FieldAccessProfile` §3 specifies — false sharing needs
   025's `Sharing` classification, prefetch distance needs loop stride. And nothing calls
   `analyse` yet: it wants a caller that turns 014's `RecordLayout` into a `Record`, which is
   a natural `chiero-vpp` or CLI job.

2. **§1.1's remaining claim — caller-visible memory** (with the object bijection, contracts
   13c/13d). `observable_beyond_the_return` refuses anything that could touch it: a volatile
   access, a store through an address that is not provably a stack slot, inline asm, a
   variadic list, an indirect call. Every one of those refusals is a comparison that should
   be possible later.

   *Done since:* **contract 6, the side-effect sequence.** `EffectKind::Call` carries the
   callee and its **arguments as terms** — the load-bearing half, since contract 6's rewrite
   swaps two calls to the *same* function and a name sequence is identical either way.
   `link_inputs` learned §1.2's shared extern-return symbols, keyed by (function, nth call),
   not by span: the two versions are different modules and a span key would match nothing.

   **`Approximated` can carry an `Equivalent` under one narrow condition**, arrived at after
   two wrong versions of the argument. Three channels connect a callee to the comparison: the
   effect sequence (compared position by position), memory (loads *and* stores through a
   non-local address refused, pointer arguments refused outright), and the return value —
   where both earlier attempts failed. So the condition is that neither side has an
   extern-return input at all. `proven` stays false; 032 §3.1 still refuses to drop a test.

   **§1.2's shared extern-return symbols are matched, on the third attempt** (2026-08-05).
   `InputOrigin::ExternReturn`/`ModelReturn` carry `seq`, the call's index in the effect
   sequence; **every** declared call is in that sequence, pure ones as `EffectKind::PureCall`,
   so the ordinal counts one thing. `comparable_effects` then drops pure calls whose result
   nobody bound — `pure` plus an unread return is genuinely unobservable — and the link key is
   the position in *that* list, because dropping one from one side would shift every later raw
   index. `compare_effects` runs before any return is linked, so position *n* is only used as a
   key once the two runs' *n*th calls are shown to be the same callee with the same arguments.

   A function with a value-returning callee is now answerable: `return p(x)` against
   `return p(x + 0)` is `Equivalent`, `p(a) - p(b)` against `p(b) - p(a)` is not.
3. **Pointer parameters and pointer returns**, which currently answer `Unknown` by name.
4. **032 §3.1's `Prover` seam wired to it.** The blocker is not equivalence — it is that
   `Prover::prove_equivalent(&chiero_diff::Entity)` has to turn an entity into two runnable
   modules, which needs the frontend from a crate that must not depend on it.

### 7.4 `chiero-replay` — a review that found ten defects, and what is left of them

A fourth adversarial review (2026-08-06) found **ten defects**, all reproduced. The headline
verdict is the one to keep:

> "The harness is the one thing that asks a real compiler" is true only for one narrow
> observable: *the two return values, cast to `long long`, at one input, called sequentially in
> one process*. That observable is narrower than the divergences it adjudicates, and it is
> corruptible by shared state in the combined TU. So the arbiter is neither sound (it can
> fabricate `Demonstrated`) nor complete (it reports `NotDemonstrated` for real divergences),
> and contract 11's downgrade converts the incompleteness into wrong verdict changes.

**The worst is D1, and it inverts contract 11.** `prove_equivalent_with_replay` discards the
`observation` and downgrades on any `NotDemonstrated` — so a true `SideEffect`, `Termination`
or `Memory` divergence, which the harness cannot see at all, drops from `Exact/proven` to
`Approximated` with the assumption text *"chiero's semantics and this compiler do not agree
here"*. That statement is false; the compiler was never asked. **Contract 11 exists to catch
chiero being wrong and currently punishes it for being right, systematically.**

The rest, in short: `Demonstrated` can be fabricated three ways (globals merged by the
two-include trick, pointer returns whose addresses always differ, and an entry that prints
`before=… after=…` itself, since the result shares stdout with the program under test); no
wall-clock limit, so `--allow-replay-exec` on a `Termination` finding hangs the tool at the
witness chosen to show the hang; witness bindings are rendered as a positional argument list
even when they are extern returns or when a pointer parameter minted none; `literal()`
truncates above 64 bits and renders float bit-patterns as integers; the return channel
`(long long) f(...)` refuses `void`, truncates `double` and `__int128`, and 050 §6's sandbox
does not exist while the doc comments cite contract 12 as though it did.

**Seven of the ten are fixed** (2026-08-06), at the rule rather than at the sites:

| # | fix |
|---|---|
| D1 | only a `ReturnValue` divergence may be adjudicated; anything else refuses and says which kind went unchecked. Contract 11 still fires where the harness *did* measure — a test asserts it |
| D4 | `emit_equivalence` returns `Result<Replay, Refusal>`; a witness that is not an argument list (extern returns, pointer params, non-contiguous indices) is refused, not compiled |
| D5 | widths > 64 refused — gcc truncates a decimal constant silently and `-w` hides it |
| D6 | the tool layer refuses a return type the `long long` channel would convert (`double`) or truncate (`__int128`) |
| D7 | the result goes to a file the harness is compiled with, not stdout, which the included program can write |
| D3 | a ten-second wall-clock limit — a `Termination` witness *is* an input that does not terminate |
| D8 | `Outcome::NotRun` and `Outcome::NoCompiler` are distinct |
| D10 | `ReplaySources::flags` carries the TU's `-I`/`-D` (040 §3's last rule) |

**Left, and worth knowing before trusting `--allow-replay-exec` on real code:**

- ~~**050 §6's sandbox does not exist.**~~ **Built 2026-08-06.** A network namespace of its
  own, a 2 GiB address-space cap, a cleared environment, the scratch directory as cwd, and the
  ten-second clock. Three C fixtures attempt the forbidden things.

  **Writes are still not confined, and the code says so in those words** — without root it
  needs more than an unprivileged user namespace, since remounting the filesystem read-only
  inside one fails on the underlying device. So `Sandbox` *reports* what this machine enforces
  and a test asserts that report against what a fixture harness actually manages, in whichever
  direction. **A limit claimed and not enforced is worse than one honestly absent**, and the
  test fails on exactly that. Every confirmation carries the report as an assumption.
- ~~**D9, the two-include trick.**~~ **Fixed 2026-08-06 — three translation units.** Each
  version compiles alone (so a shared `static` helper stays file-local) with a non-static
  wrapper appended *inside* it (so a `static` entry is still reachable, which is what the
  single-TU trick existed for). The entry is renamed per unit, since two units defining a
  non-static `f` collide at link time. Verified end to end on two versions of an `abs()` that
  share two static helpers: `differs` at `INT_MIN`, `outcome: demonstrated`.

  *Still open:* a **non-static** helper the two versions share collides the same way. Renaming
  every shared symbol needs the file parsed, which is more than a harness should do.
- ~~**D2's remaining route.**~~ **Closed 2026-08-06, at the class rather than the door.** A
  fifth review fabricated `Demonstrated` from two byte-identical sources *four* ways —
  `rand()`, `clock_gettime()`, a constructor, and an `atexit` handler rewriting the result
  file. All four were one defect: `before` and `after` ran **in one process**, so everything
  outside a translation unit was shared.

  **Each version is now its own program**, built and run separately, `_exit`ing after it writes
  so no atexit handler can rewrite the answer — plus a **determinism re-check**, since
  isolation cannot fix a program that reads the clock. Running `before` twice and disagreeing
  is `Outcome::Nondeterministic`, which is *not* a downgrade: nothing was learned about chiero.

  > **The lesson, after five rounds on one file: I kept fixing the door.** Each round I closed
  > the demonstrated route and the next review walked through a neighbouring one. The fixes that
  > held were the ones that changed the *shape* — refuse what the channel cannot carry, one
  > process per version, one place that decides how to launch.

**Still open from the fifth review, and worth reading before trusting a `demonstrated`:**

- **S1/S9 — the argument types are unchecked.** `unrepresentable_return` guards the *return*
  type; nothing guards the *parameters*. A `float` parameter's witness is a **bit pattern**
  (the engine sorts floats as `BitVec`), rendered as a decimal and passed through `long long`
  — so `2.0f` goes in as `1073741824.0f`, the harness reports agreement, and contract 11
  downgrades a **true** finding with the false sentence D1 was filed for. The rule is *every
  value crossing the channel, in either direction*; only one direction is written. Arity is
  the same gap: a trailing pointer parameter leaves the indices contiguous and the call short.
- **S4 — nothing ties `cfg.entry` to `src.entry`.** `prove_equivalent` compares one function
  and the harness compiles whichever the sources name; they are independent strings.
  `unrepresentable_return` returns `None` (= representable) for an absent entry, so the type
  gate silently no-ops on exactly that case. The reviewer got a fabricated `demonstrated` at
  `proven: true`.
- ~~S1/S9, S4, S6, S8, S10~~ — **fixed 2026-08-06.** One rule for every value crossing the
  channel in either direction (`harness_signature_objection` checks the return *and* every
  parameter, and an absent entry is an objection rather than silence); `cfg.entry` must equal
  `src.entry`; the compile gets the same wall clock as the run; `-fcommon` is refused by name;
  a relative or quoted scratch path is refused with a message about the path.
- ~~S7~~ — **fixed 2026-08-06.** The timeout kills anything still running with this call's
  unique path on its command line, in *both* the run and the compile paths. Writing the test
  found the same leak where the fix would not have reached: a compiler driver spawns `cc1` as a
  child, so the compile timeout killed `cc` and left `cc1` grinding on the blocked source.

**All ten of the fifth review are closed.** The harness is narrow — return-value divergences,
integer parameters and returns of ≤64 bits, an entry that matches the module chiero analysed —
and it refuses everything else by name. Within that, `Demonstrated` now survives every
fabrication the review could construct.
- ~~Refusal whitespace~~ and ~~`sandbox()`'s per-call `unshare` spawn~~ — fixed.

Probes: ~~`$SCRATCH/rev5` (20 fixtures, `cargo run -- <name>`); `$SCRATCH/replayprobe` (13)~~ —
**both lost** (§9.2). They were never committed.

