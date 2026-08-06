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
trap 'rm -f "$J"' EXIT
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
  timeout "$TIMEOUT" "$CHIERO" find-bugs "$VPP/src/$f" --entry "$fn" --json \
      $INC $own $DEF "$@" >"$J" 2>/dev/null
  rc=$?
  # A timeout and a crash are different facts and neither is "no findings" — the whole
  # project's rule, applied to its own measurement harness.
  if [ $rc -eq 124 ]; then printf '%s\t%s\ttimeout\t0\t0\n' "$f" "$fn"; continue; fi
  if [ ! -s "$J" ]; then printf '%s\t%s\tfailed\t0\t0\n' "$f" "$fn"; continue; fi
  python3 "$HERE/count.py" "$f" "$fn" "$J"
done < "$LIST"
