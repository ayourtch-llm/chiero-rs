#!/bin/bash
# Whether the workspace is green — and it says how, not just yes.
#
# **This exists because the way I was checking could not fail.** A `cargo test | awk`
# pipeline summing "N passed" over `test result:` lines silently ignored crates whose test
# BINARY failed to build, and reported a total that looked fine while three xtask gates were
# red. Counting successes cannot detect a missing success. So this keys on cargo's exit
# status, which is the one signal that means what it says.
#
# ⚠️ **And then it went on being green while CI was red, for the same reason one level up.**
# Found 2026-08-07: `cargo fmt --all --check` reported **26 diffs** and
# `cargo clippy --workspace --all-targets -- -D warnings` **2 errors**, both introduced by
# commits that this script had called GREEN. CI runs three legs and this ran one, so "the
# workspace is green" was a claim about `cargo test` wearing a broader word. A local gate
# narrower than the remote one does not warn you, it reassures you.
#
# So it runs all three, and the order is deliberate: fmt and clippy are seconds, the tests are
# most of an hour, and finding out about a formatting diff after the hour is the wrong way
# round. `--skip-lints` is there for the case that motivated the split — re-running the tests
# while a fix for one of the fast legs is still being written.
#
# ⚠️ Not covered **by default**, and deliberately: CI's **second solver leg**
# (`CHIERO_SMT_SOLVER=/nonexistent cargo test --workspace --no-fail-fast`). It doubles the
# runtime, and HANDOFF §10 asks for it by name at the points where it matters. Naming the gap
# is the whole difference between a gate and a reassurance.
#
# `--both-legs` runs it. The default stays one leg — the runtime argument is unchanged and
# still right — but a gap that is only *named* has to be reconstructed from this comment by
# whoever needs it, and the invocation is exactly the kind of thing that gets mistyped or
# half-remembered. Naming it was the difference between a gate and a reassurance; making it
# reachable is the difference between a note and a tool.
set -o pipefail

lints=1
both_legs=0
case "${1:-}" in
--skip-lints) lints=0 ;;
--both-legs) both_legs=1 ;;
esac

if [ $lints -eq 1 ]; then
  # **Fast legs first, and they gate.** Exiting rather than warning is the point: CI will
  # refuse this tree, so reporting it as GREEN after an hour of tests would be the same lie in
  # a more expensive form.
  # 📌 **This is also `.githooks/pre-commit`**, and the reason that hook exists: on 2026-08-10
  # the CI red the owner had been seeing turned out to be one file committed unformatted, 34 of
  # 53 failed runs, caught by *this* leg in a second — except that nobody ran it. Enable the
  # hook in a clone with `git config core.hooksPath .githooks`.
  if ! fmt=$(cargo fmt --all --check 2>&1); then
    echo "$fmt" | grep "^Diff in" | sed 's/^/  /' | head -20
    echo "RED (cargo fmt): $(echo "$fmt" | grep -c '^Diff in') diffs — CI runs this; run 'cargo fmt --all'"
    exit 1
  fi
  if ! clip=$(cargo clippy --workspace --all-targets -- -D warnings 2>&1); then
    echo "$clip" | grep -E "^error" | head -20
    echo "RED (clippy -D warnings): $(echo "$clip" | grep -cE '^error(\[|:)') errors — CI runs this"
    exit 1
  fi
  # **The no-default-features build**, which nothing else here reaches: `cargo test` builds
  # with default features, so a break in 003 §3's hard constraint — chiero builds and runs
  # linking nothing — was invisible to this gate and caught only by CI. ~0.6 s warm.
  if ! nodef=$(RUSTFLAGS="-D warnings" cargo build --workspace --no-default-features 2>&1); then
    echo "$nodef" | grep -E "^error" | head -20
    echo "RED (--no-default-features build): CI runs this; the no-link constraint is §3's"
    exit 1
  fi
  # **`RUSTFLAGS: -D warnings`, which is what CI compiles everything under**, and the seventh
  # difference found *inside* a leg this script already claimed to cover. `.github/workflows/ci.yml`
  # sets it globally in `env:`, so a plain **rustc** warning — not a clippy lint — is red there
  # and was green here: a newer stable toolchain adding a lint fails CI with no code change,
  # which is the shape of the 2026-08-10 "CI has been failing for a while" report. The gap was
  # written down in this file on 2026-08-10 as a comment telling a reader to run two commands by
  # hand; a gate that has to be remembered is not one (§8.3).
  #
  # `cargo check` rather than `build`: rustc lints fire during checking, it is ~8 s cold and
  # under a second warm, and its fingerprint is its own — so this leg does not invalidate the
  # `cargo test` artifacts the long leg below is about to use.
  if ! warn=$(RUSTFLAGS="-D warnings" cargo check --workspace --all-targets 2>&1); then
    echo "$warn" | grep -E "^(error|warning)" | head -20
    echo "RED (rustc -D warnings): a *rustc* warning, which CI compiles as an error"
    exit 1
  fi
  # **023 contract 13a**, and the one CI gate with no test behind it: `check_proof_surface`
  # is called from `xtask/src/main.rs` and nowhere else, so `cargo test` cannot see a
  # regression in it. ~0.5 s. (`check-deps` and `check-vpp-leak` *are* covered — by
  # `the_real_workspace_is_clean` and `workspace_has_no_vpp_leaks` — so they stay out.)
  # **Broader than CI, deliberately, and the only leg that is.** Every other leg here exists
  # because CI runs it and a narrower local gate lies (§8.3). This one CI does not run: it
  # checks that HANDOFF.md's numbered lists have not drifted, which §9's START HERE had done
  # for thirty-five waves before anyone noticed two items numbered `3.`. The file is this
  # project's continuity, §10 asks for it to be updated before every refresh, and *before
  # pushing* is exactly when a broken one would go out. ~0.05 s, and the fix is one line.
  if ! nums=$(python3 tests/corpus/handoff/lint.py 2>&1); then
    echo "$nums" | head -10 | sed 's/^/  /'
    echo "RED (HANDOFF lint): numbering, a cited path, or a code fence — see §9.2"
    exit 1
  fi
  if ! proof=$(cargo run -q -p xtask -- check-proof-surface 2>&1); then
    echo "$proof" | head -20 | sed 's/^/  /'
    echo "RED (check-proof-surface): 023 contract 13a — a proof cannot be forged; CI runs this"
    exit 1
  fi
fi

# **`--nocapture`, so a skip is countable.** cargo hides a passing test's output, and 103
# assertions in this suite begin `if !<corpus available> { eprintln!("…skipping…"); return; }`.
# On a machine without VPP, gcc or a solver those tests print that line and report `ok` — so the
# contract they carry is asserted by nothing and the run says nothing about it. Found 2026-08-10
# via 011 c11, whose VPP corpus does not exist on the CI runner. **A skip nobody counts is a
# pass**, and this is the cheap half of the fix: not a gate, a number that moves.
out=$(cargo test --workspace -- --nocapture 2>&1)
rc=$?
echo "$out" | grep -E "^test result: FAILED|^error(\[|:)" | head -20
# **The assertion, not just the suite.** This printed the failing suite and nothing else, so a
# test that flaked once inside a full run could not be diagnosed afterwards — the detail was in
# `$out` the whole time and was thrown away. cargo groups the panic text and the `left:`/`right:`
# values under `failures:`, so that section is what a reader needs.
if [ $rc -ne 0 ]; then
  detail=$(echo "$out" | sed -n '/^failures:$/,$p')
  shown=$(echo "$detail" | head -40)
  echo "$shown" | sed 's/^/  /'
  total=$(echo "$detail" | wc -l)
  if [ "$total" -gt 40 ]; then
    echo "  … $((total - 40)) more lines of failure detail (re-run the named suite for all of it)"
  fi
fi
passed=$(echo "$out" | grep -E "^test result: ok" | awk -F'[ ;]' '{p+=$4} END {print p+0}')
# **`$6` is not the failure count in every rendering**, and the summary said "0 failed"
# while a suite reported 31 passed / 1 failed inside — reported 2026-08-10 by the first
# end-to-end user. The verdict was right (it keys on cargo's exit status) and only the
# counter was wrong, which is the more insidious half: a reader trusts the number.
# `failed=N` is the field's own name, so match it rather than counting columns.
failed=$(echo "$out" | grep -oE "[0-9]+ failed" | awk '{f+=$1} END {print f+0}')
suites=$(echo "$out" | grep -cE "^test result:")
# **Distinct messages, not lines.** The vocabulary is what the tests already write, and the
# first version counted every line: `--nocapture` interleaves cargo's progress dots into the
# text, and a helper called in a loop prints its skip once per call — 56 lines for 26 distinct
# skips in one subset. `grep -o` from the keyword drops the dots, `sort -u` drops the repeats.
# Approximate on purpose: standardising 103 call sites is the expensive half, and a number that
# moves is useful before it.
skipped=$(echo "$out" | grep -oiE "(skipping|skipped:).*" | sort -u | wc -l)
if [ $rc -eq 0 ] && [ $both_legs -eq 1 ]; then
  # **The no-solver leg.** `discover()` consults `$CHIERO_SMT_SOLVER` first, so a path that
  # does not exist is what CI uses to prove the tree works with no solver at all.
  echo "second leg: no solver (CHIERO_SMT_SOLVER=/nonexistent)"
  out2=$(CHIERO_SMT_SOLVER=/nonexistent/no-solver-here cargo test --workspace --no-fail-fast -- --nocapture 2>&1)
  rc=$?
  echo "$out2" | grep -E "^test result: FAILED|^error(\[|:)" | head -20
  if [ $rc -ne 0 ]; then
    echo "$out2" | sed -n '/^failures:$/,$p' | head -40 | sed 's/^/  /'
  fi
  p2=$(echo "$out2" | grep -E "^test result: ok" | awk -F'[ ;]' '{p+=$4} END {print p+0}')
  s2=$(echo "$out2" | grep -oiE "(skipping|skipped:).*" | sort -u | wc -l)
  # **The number this leg exists to make visible.** With no backend the five `chiero-check`
  # tests that assert what a *complete* solver decides announce themselves and return; counting
  # them is the difference between "the solverless configuration passes" and "it passes and here
  # is what it did not ask".
  # **The leg where the list is the point.** The first leg has everything installed and skips
  # nothing; this one is the configuration that skips, so printing the count and withholding the
  # names would put the useful half where it cannot be read.
  if [ "$s2" -gt 0 ]; then
    echo "$out2" | grep -oiE "(skipping|skipped:).*" | sort -u | head -10 | sed 's/^/  - /'
    [ "$s2" -gt 10 ] && echo "  … $((s2 - 10)) more"
  fi
  echo "second leg: $p2 passed, $s2 distinct skips, exit $rc"
fi

# **The list, not just the count.** A number says something was skipped; the list says whether
# it mattered. Only when there is one, and capped — on a machine with everything installed this
# is empty and prints nothing, which is the common case and should stay silent.
if [ "$skipped" -gt 0 ]; then
  echo "$out" | grep -oiE "(skipping|skipped:).*" | sort -u | head -10 | sed 's/^/  - /'
  [ "$skipped" -gt 10 ] && echo "  … $((skipped - 10)) more"
fi

if [ $rc -eq 0 ]; then
  if [ $lints -eq 1 ]; then
    echo "GREEN: $passed passed across $suites suites, $skipped distinct skips, fmt and clippy clean"
    # ⚠️ **Still not identical to CI**, and the remaining differences are environmental rather
    # than commands: CI resolves `dtolnay/rust-toolchain@stable` on the runner, so a newer
    # rustc can introduce a lint this machine's toolchain does not have, and its `solver: z3`
    # leg installs z3 from apt rather than using the one here. `--both-legs` covers the
    # solverless configuration; nothing local can cover a toolchain this machine has not got.
  else
    echo "GREEN (tests only, lints skipped): $passed passed across $suites suites, $skipped distinct skips"
  fi
else
  echo "RED (cargo exit $rc): $passed passed, $failed failed, $skipped distinct skips across $suites suites"
fi
exit $rc
