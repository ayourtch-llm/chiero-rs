#!/bin/bash
# Measure `chiero find-bugs` over a fixed list of VPP entry points.
#
# See README.md for the numbers this produced and what they mean. The point of checking it
# in is that the numbers are reproducible by somebody other than the person who took them —
# an unreproducible measurement is an assertion.
#
#   ./measure.sh                      the default run
#   ./measure.sh --entry-ptr-nonnull  with the caller-checks-its-pointers assumption
#
# Environment:
#   CHIERO   path to the release binary   (default: ../../../target/release/chiero)
#   VPP      a VPP checkout               (default: /home/ubuntu/vpp)
#   VPPBUILD its cmake build directory    (default: $VPP/build-root/build-vpp-native)
#   TIMEOUT  seconds per entry point      (default: 60)
#   LIST     the entry list to run        (default: entries.tsv beside this script)
#   KEEP     a directory to save each entry's envelope into, as `<file>.<fn>.json`
#
# **`KEEP` exists because the summary line cannot answer "what changed".** Moving `BadRange`
# out of the defect list on 2026-08-07 left the pinned-40 numbers **byte-identical** — 21
# findings, 0 `Exact`, 2 cut — and there was no way to tell "this corpus never produced one"
# from "the harness is not measuring what I think". §11.3: the residue of a gate is a corpus,
# and this one was being thrown away.
#
# Output is one TSV line per entry: file<TAB>fn<TAB>status<TAB>findings<TAB>exact.
# Summarise with:
#   ./measure.sh | awk -F'\t' '{s[$3]++;n+=$4;e+=$5} END{for(k in s)printf "%s=%d ",k,s[k];
#                               printf "findings=%d exact=%d\n",n,e}'
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
CHIERO=${CHIERO:-$HERE/../../../target/release/chiero}
VPP=${VPP:-/home/ubuntu/vpp}
VPPBUILD=${VPPBUILD:-$VPP/build-root/build-vpp-native}
TIMEOUT=${TIMEOUT:-60}
# Only consulted when COMPDB is set; `cargo run -p xtask --` also works.
XTASK=${XTASK:-$HERE/../../../target/debug/xtask}
# Overridable so a wider sweep does not need a second copy of this script — the checked-in
# number is `entries.tsv`, and anything else is an exploration that says which list it ran.
LIST=${LIST:-$HERE/entries.tsv}

# VPP's own flags, from `INCLUDES`/`DEFINES` in $VPPBUILD/vpp/build.ninja. Taken from the
# build rather than guessed: a header reached under different `-D`s is a different header,
# and the whole claim is about the code VPP actually compiles.
# The per-target roots too: `build.ninja` carries 1969 `INCLUDES` lines and they differ, so
# taking the first one silently excludes every `*_api.c` in the tree. VPP's API compiler
# generates `<bier/bier.api_enum.h>` into `CMakeFiles/vnet/bier/`, and a file that will not
# preprocess is a file the measurement did not cover — not a file with no defects.
INC="-I$VPP/src -I$VPPBUILD/vpp/CMakeFiles"
for d in vnet vlibmemory vpp crypto_engines plugins; do
  [ -d "$VPPBUILD/vpp/CMakeFiles/$d" ] && INC="$INC -I$VPPBUILD/vpp/CMakeFiles/$d"
done
# A plugin includes its siblings as <acl/acl.h>, so the *source* plugins root is a search
# path too — not only the generated one.
[ -d "$VPP/src/plugins" ] && INC="$INC -I$VPP/src/plugins"
DEF="-DHAVE_FCNTL64 -DHAVE_LIBUNWIND=1 -D_FORTIFY_SOURCE=2"

[ -x "$CHIERO" ] || { echo "no chiero binary at $CHIERO — cargo build --release" >&2; exit 2; }
[ -d "$VPP/src" ] || { echo "no VPP checkout at $VPP" >&2; exit 2; }

J=$(mktemp)
E=$(mktemp)
trap 'rm -f "$J" "$E"' EXIT
while IFS=$'\t' read -r f fn; do
  case "$f" in ''|'#'*) continue ;; esac
  # Each plugin's API compiler output lives in a directory of its own, exactly as
  # `build.ninja` has it: `-I…/CMakeFiles/plugins/acl` for `plugins/acl/*.c`. Adding every
  # plugin's directory at once would work today and shadow the wrong header the first time two
  # plugins generate the same name.
  own=""
  case "$f" in
    plugins/*/*)
      d=${f#plugins/}; d=${d%%/*}
      [ -d "$VPPBUILD/vpp/CMakeFiles/plugins/$d" ] &&
        own="-I$VPPBUILD/vpp/CMakeFiles/plugins/$d"
      ;;
  esac
  # **`COMPDB=<file>` takes the flags from the build instead of from the list above.**
  #
  # The hand-kept `INC`/`DEF` is a second reader of a fact the build already states, and the
  # two have drifted: measured 2026-08-09, **198 of 935 plugin C units need include paths this
  # script never passes**, and ~16% of those fail to preprocess because of it — reported as
  # "chiero cannot read this" when the flags are the cause (HANDOFF §7.30).
  #
  # ⛔ **Opt-in, and do NOT make it the default: this is the parked `-march` item.** The
  # database's flags carry `-march=x86-64-v2 -mtune=generic`, which this script has never
  # passed, and they change the analysis rather than merely widening it — the pinned 40 run
  # this way keeps its summary line (`cut=2 ok=38 findings=21`) while **26 of 38 envelopes
  # differ**, and `-march` alone changes the CIR of `vppinfra/hash.c`.
  #
  # The *include-path* half is separable and safe (20 plugin files: 17 byte-identical CIR, 0
  # differing). If the goal is only to recover the ~30 files that fail for want of an `-I`,
  # add the missing include paths and leave the target configuration alone.
  #
  #   ninja -C $VPPBUILD/vpp -t compdb > /tmp/db.json
  #   COMPDB=/tmp/db.json ./measure.sh
  #
  # A file the build does not compile has no flags, and `compile-flags` says so by failing;
  # the hand-kept list is the fallback there, because refusing to measure a file the sweep was
  # asked about would be a silent hole rather than a reported one.
  flags="$INC $own $DEF"
  if [ -n "${COMPDB:-}" ]; then
    real=$("$XTASK" compile-flags --db "$COMPDB" "$f" 2>/dev/null | head -1)
    [ -n "$real" ] && flags="$real"
  fi
  # **chiero's own clock first, the harness's as a backstop.** `--time-budget` stops the
  # search and prints what it had; `timeout` kills the process and prints nothing, which is
  # what every `timeout` row in the old numbers was — a function about which the measurement
  # says nothing, indistinguishable from one with nothing to say. The outer limit is larger so
  # that the two are tellable apart: `cut` means chiero stopped, `timeout` means something the
  # clock does not cover did not (the frontend, or a single solver query).
  timeout "$((TIMEOUT + 30))" "$CHIERO" find-bugs "$VPP/src/$f" --entry "$fn" --json \
      --time-budget "$TIMEOUT" $flags "$@" >"$J" 2>"$E"
  rc=$?
  # Saved before any of the classification below, so a `timeout`'s empty file and a `failed`'s
  # stderr are both in the residue rather than only the rows that produced a count.
  if [ -n "${KEEP:-}" ]; then
    mkdir -p "$KEEP"
    k=$KEEP/$(printf '%s' "$f" | tr / _).$fn
    cp "$J" "$k.json" 2>/dev/null
    [ -s "$E" ] && cp "$E" "$k.err"
  fi
  # A timeout and a crash are different facts and neither is "no findings" — the whole
  # project's rule, applied to its own measurement harness.
  if [ $rc -eq 124 ]; then printf '%s\t%s\ttimeout\t0\t0\n' "$f" "$fn"; continue; fi
  # **A header this machine does not have is not a chiero failure.** `plugins/af_xdp` needs
  # `xdp/xsk.h` from libxdp, `plugins/dpdk` needs DPDK's tree: the file cannot be preprocessed
  # here at all, by chiero or by gcc. Filed under `failed` it read as seven defects in the
  # frontend; it is seven files this environment cannot present, and the number that matters —
  # what chiero does with the code it can read — is wrong either way round.
  if [ ! -s "$J" ] && grep -q "cannot include" "$E"; then
    printf '%s\t%s\tnoinc\t0\t0\n' "$f" "$fn"; continue
  fi
  if [ ! -s "$J" ]; then printf '%s\t%s\tfailed\t0\t0\n' "$f" "$fn"; continue; fi
  python3 "$HERE/count.py" "$f" "$fn" "$J"
done < "$LIST"
