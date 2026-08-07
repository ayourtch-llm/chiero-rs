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
# ⚠️ Not covered here, and deliberately: CI's **second solver leg**
# (`CHIERO_SMT_SOLVER=/nonexistent cargo test --workspace --no-fail-fast`). It doubles the
# runtime, and HANDOFF §10 asks for it by name at the points where it matters. Naming the gap
# is the whole difference between a gate and a reassurance.
set -o pipefail

lints=1
case "${1:-}" in
--skip-lints) lints=0 ;;
esac

if [ $lints -eq 1 ]; then
  # **Fast legs first, and they gate.** Exiting rather than warning is the point: CI will
  # refuse this tree, so reporting it as GREEN after an hour of tests would be the same lie in
  # a more expensive form.
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
fi

out=$(cargo test --workspace 2>&1)
rc=$?
echo "$out" | grep -E "^test result: FAILED|^error(\[|:)" | head -20
passed=$(echo "$out" | grep -E "^test result: ok" | awk -F'[ ;]' '{p+=$4} END {print p+0}')
failed=$(echo "$out" | grep -E "^test result: FAILED" | awk -F'[ ;]' '{f+=$6} END {print f+0}')
suites=$(echo "$out" | grep -cE "^test result:")
if [ $rc -eq 0 ]; then
  if [ $lints -eq 1 ]; then
    echo "GREEN: $passed passed across $suites suites, fmt and clippy clean"
  else
    echo "GREEN (tests only, lints skipped): $passed passed across $suites suites"
  fi
else
  echo "RED (cargo exit $rc): $passed passed, $failed failed across $suites suites"
fi
exit $rc
