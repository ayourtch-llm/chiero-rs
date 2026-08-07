#!/bin/bash
# **The five-TU probe** — chiero against real VPP build lines, in seconds rather than hours.
#
# A full `xtask sweep` over VPP is a two-hour measurement. Most of what a wave needs to know
# — does the frontend still get through a real translation unit, and with which diagnostic —
# is answered by five of them, and answered before you have finished reading the diff. The
# original paid for itself four sweeps over.
#
# ⚠️ **This script was lost once, in 2026-08-07's audit**, because it lived only in a
# scratchpad and a scratchpad is per-session. HANDOFF §9.2's rule is the reason it is here:
# *commit an instrument in the wave that builds it.* Everything committed survived; nothing
# uncommitted did.
#
# Usage:
#   tests/corpus/vpp-findings/probe.sh                 # the five default TUs
#   tests/corpus/vpp-findings/probe.sh vlib/main.c …   # any source paths, relative to src/
#
# Environment:
#   VPPBUILD   the ninja build directory  (default below)
#   REALCC     what to hand the arguments to after chiero has looked at them. The default,
#              `true`, does not compile at all — the probe asks what *chiero* makes of the
#              flags, and a real `-O3` build of `vlib/main.c` is most of the wall clock. Set
#              it to the build's own compiler when the differential is the point.
set -u -o pipefail

VPPBUILD=${VPPBUILD:-/home/ubuntu/vpp/build-root/build-vpp-native/vpp}
REALCC=${REALCC:-true}
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)

# The five: one from `vlib`, two from `vppinfra`, one from `vnet`, and `node_cli.c` because it
# is the CLI-registration shape 042's recipes are written against. They were chosen to span
# subsystems, not to be small.
TUS=("$@")
if [ ${#TUS[@]} -eq 0 ]; then
  TUS=(vlib/main.c vppinfra/format.c vnet/interface.c vlib/node_cli.c vppinfra/mem_dlmalloc.c)
fi

if [ ! -f "$VPPBUILD/build.ninja" ]; then
  echo "probe: no ninja build at $VPPBUILD (set VPPBUILD)" >&2
  exit 2
fi

# ⚠️ **`CCACHE_DISABLE=1` is mandatory** — VPP's cmake wraps the compiler in ccache, and a warm
# cache makes the measurement about the cache. It is exported even though the default `REALCC`
# never compiles, because the moment somebody sets `REALCC` it stops being hypothetical.
export CCACHE_DISABLE=1
export CHIERO_REAL_CC=$REALCC

SHIM="$ROOT/target/release/xtask"
if [ ! -x "$SHIM" ]; then
  # ⚠️ **Never build this while a sweep is running** — the shim execs the binary being
  # overwritten. Build it deliberately, not as a side effect of a probe.
  echo "probe: no release xtask; run  cargo build --release -p xtask  first" >&2
  exit 2
fi

LOG=$(mktemp -t chiero-probe-XXXXXX.jsonl)
CMDS=$(mktemp -t chiero-probe-cmds-XXXXXX)
trap 'rm -f "$LOG" "$CMDS"' EXIT
export CHIERO_CC_LOG=$LOG

# **Every command the build would run, dumped once.** 63 ms for VPP's 2945 of them, so the
# per-TU cost is a `grep`.
#
# ⚠️ The obvious route — construct the object path from the source path and ask
# `ninja -t commands` for it — does not work, and looks like it does. CMake names an object
# after its position in the *object library*, so `src/vlib/main.c` is
# `CMakeFiles/vlib/CMakeFiles/vlib_objs.dir/main.c.o`: the directory is gone. A first draft
# matched on `vlib/main.c.o` and reported **NO TARGET for all five**, which is the honest
# failure only because the message says "not built into this configuration" rather than
# printing nothing. Match on what the compiler is actually told: `-c <absolute source>`.
ninja -C "$VPPBUILD" -t commands all 2>/dev/null >"$CMDS"

for tu in "${TUS[@]}"; do
  # A source compiled into several object libraries — which multiarch does, 060's 1:N — has
  # several commands. Take the first and *say how many there were*, because a probe that
  # hides an N behind one number is the thing 060 warns about.
  mapfile -t hits < <(grep -E -- " -c [^ ]*/src/${tu//./\\.}( |$)" "$CMDS")
  if [ ${#hits[@]} -eq 0 ]; then
    printf '%-28s NO TARGET (not built into this configuration)\n' "$tu"
    continue
  fi
  cmd=${hits[0]}

  # Strip the launcher and the compiler off the front, leaving the argument list. Any number
  # of `*ccache` wrappers, then one compiler; whatever it is becomes `CHIERO_REAL_CC` unless
  # the caller named one, so the delegate matches the build rather than the machine's `cc`.
  read -r -a argv <<<"$cmd"
  i=0
  while [[ ${argv[$i]} == *ccache ]]; do i=$((i+1)); done
  built_with=${argv[$i]}
  i=$((i+1))
  if [ "$REALCC" = "true" ]; then :; else export CHIERO_REAL_CC=$built_with; fi

  before=$(wc -l <"$LOG")
  ( cd "$VPPBUILD" && "$SHIM" cc "${argv[@]:$i}" ) >/dev/null 2>&1
  added=$(( $(wc -l <"$LOG") - before ))
  if [ "$added" -eq 0 ]; then
    # **Not the same fact as "no findings".** The shim records nothing when it decides the
    # invocation compiles no C at all, which for a probe is a failure of the probe.
    printf '%-28s NOT ANALYSED (the shim found no source in the command line)\n' "$tu"
    continue
  fi
  printf '%-28s [%s, %d target(s)] %s\n' \
    "$tu" "$(basename "$built_with")" "${#hits[@]}" "$(tail -1 "$LOG")"
done
