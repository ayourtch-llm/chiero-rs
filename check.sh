#!/bin/bash
# Whether the workspace is green — and it says how, not just yes.
#
# **This exists because the way I was checking could not fail.** A `cargo test | awk`
# pipeline summing "N passed" over `test result:` lines silently ignored crates whose test
# BINARY failed to build, and reported a total that looked fine while three xtask gates were
# red. Counting successes cannot detect a missing success. So this keys on cargo's exit
# status, which is the one signal that means what it says.
set -o pipefail
out=$(cargo test --workspace 2>&1)
rc=$?
echo "$out" | grep -E "^test result: FAILED|^error(\[|:)" | head -20
passed=$(echo "$out" | grep -E "^test result: ok" | awk -F'[ ;]' '{p+=$4} END {print p+0}')
failed=$(echo "$out" | grep -E "^test result: FAILED" | awk -F'[ ;]' '{f+=$6} END {print f+0}')
suites=$(echo "$out" | grep -cE "^test result:")
if [ $rc -eq 0 ]; then
  echo "GREEN: $passed passed across $suites suites"
else
  echo "RED (cargo exit $rc): $passed passed, $failed failed across $suites suites"
fi
exit $rc
