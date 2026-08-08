#!/bin/bash
# Establish one 032 contract 18 corpus entry: revert a historical fix, run the suite, see it fail.
#
# **The method, and why it is this one.** Hunting for a commit whose fix an existing test happens
# to catch cost ~40 minutes per attempt and produced one rejection: `1d0e0e825`'s test *passes* at
# the parent, so the suite never exercised the overwrite. Reverting a fix's `src/` diff on top of
# HEAD instead uses the build that already exists, and whatever fails is ground truth — observed
# rather than guessed, which is the whole difference `corpus.tsv` draws between its two evidence
# kinds. The entry still records the original commit, and the diff replayed for selection is still
# `commit^..commit`, so nothing the gate measures changes.
#
# ⚠️ **This script's first job is not to lose the user's tree.** Its predecessor was written once,
# lived only in a scratch directory, and was gone by the next session (HANDOFF §9.2) — so this one
# is committed, and it restores `src/` on **every** exit path including SIGINT and SIGTERM.
#
#   replay_probe.sh --check <commit>          # tree-safety and revert mechanics only, no build
#   replay_probe.sh <commit> <test-suite>     # the real thing: revert, build, run, restore
#
# ⚠️ **The real form re-runs cmake.** `ninja` regenerates `build.ninja` whenever a `CMakeLists.txt`
# is newer than it, and four of VPP's are (measured 2026-08-08). `build.ninja` is what
# `chiero_vpp::builddb` reads for all 1967 compile commands, what `probe.sh` replays, and what 012
# contract 17's corpus gate is built from — so a run of this script invalidates the *baseline* those
# numbers were taken against, even when it succeeds. Re-take them afterwards rather than assuming.
set -u -o pipefail

VPP=${VPP:-/home/ubuntu/vpp}
BUILD=$VPP/build-root/build-vpp-native/vpp

die() { echo "replay_probe: $*" >&2; exit 2; }

[ -d "$VPP/.git" ] || die "no VPP checkout at $VPP (set VPP=)"

check_only=0
if [ "${1:-}" = "--check" ]; then check_only=1; shift; fi
commit=${1:-} ; suite=${2:-}
[ -n "$commit" ] || die "usage: replay_probe.sh [--check] <commit> [<test-suite>]"

# **A dirty tree is a refusal, not something to work around.** The restore below is
# `git checkout -- src/`, which would throw away a real edit somebody had in progress.
if [ -n "$(git -C "$VPP" status --porcelain -- src/)" ]; then
    die "the VPP tree has uncommitted changes under src/ — refusing to touch it"
fi

git -C "$VPP" cat-file -e "$commit^{commit}" 2>/dev/null || die "no such commit: $commit"

restored=0
restore() {
    [ $restored -eq 1 ] && return
    restored=1
    echo "replay_probe: restoring $VPP/src"
    git -C "$VPP" checkout -- src/ || echo "replay_probe: RESTORE FAILED — check $VPP by hand" >&2
    if [ -n "$(git -C "$VPP" status --porcelain -- src/)" ]; then
        echo "replay_probe: src/ is STILL dirty after restore — check $VPP by hand" >&2
    fi
}
trap restore EXIT INT TERM

# The `src/` half of the commit, reversed. Only `src/`: a fix's test changes belong to the suite
# being used as the oracle, and reverting those would remove the very check being measured.
echo "replay_probe: reverting the src/ half of $commit"
if ! git -C "$VPP" diff "$commit^" "$commit" -- src/ | git -C "$VPP" apply -R --index=/dev/null - 2>/dev/null; then
    if ! git -C "$VPP" diff "$commit^" "$commit" -- src/ | git -C "$VPP" apply -R -; then
        die "the diff does not reverse-apply on HEAD — pick a commit HEAD has not moved past"
    fi
fi
changed=$(git -C "$VPP" status --porcelain -- src/ | wc -l)
[ "$changed" -gt 0 ] || die "reversing $commit changed nothing under src/ — is it already reverted?"
echo "replay_probe: $changed file(s) reverted"

if [ $check_only -eq 1 ]; then
    echo "replay_probe: --check, so nothing was built and nothing was run"
    exit 0
fi

[ -n "$suite" ] || die "a test suite is required for the real run, e.g. test_vlib"

# `make build` refuses on `$(BR)/.deps.ok` when apt reports a package one patch behind. It is a
# freshness check, not a missing dependency (HANDOFF §7.1).
touch "$VPP/build-root/.deps.ok" 2>/dev/null

echo "replay_probe: building"
if ! (cd "$VPP" && make build >/tmp/replay_probe_build.log 2>&1); then
    echo "replay_probe: BUILD FAILED — tail of /tmp/replay_probe_build.log:" >&2
    tail -20 /tmp/replay_probe_build.log >&2
    # A build failure is a real answer: the fix's absence may not even compile, which no test
    # can be said to have caught. Recorded, not silently retried.
    exit 3
fi

echo "replay_probe: running $suite"
(cd "$VPP" && make test TEST="$suite" >/tmp/replay_probe_test.log 2>&1)
rc=$?
tail -5 /tmp/replay_probe_test.log

echo
if [ $rc -ne 0 ]; then
    echo "OBSERVED: $suite FAILS with $commit's src/ change reverted."
    echo "Corpus line (append to corpus.tsv, then re-run the gate):"
    printf '%s\t%s\tobserved\t%s\n' "$commit" "$suite" \
        "$(git -C "$VPP" log -1 --format=%s "$commit")"
else
    echo "REJECTED: $suite still PASSES without the fix — the suite does not exercise it."
    echo "Record it as a probed-and-rejected candidate in corpus.tsv's comment block, so the"
    echo "next reader does not spend the same time on it."
fi
exit 0
