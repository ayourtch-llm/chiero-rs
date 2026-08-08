#!/bin/bash
# Lower VPP's `-march=x86-64-v3/v4` translation units — the AVX2/AVX-512 half of the tree.
#
# **Why this surface is separate from every other measurement.** VPP's baseline is
# `-march=x86-64-v2`, `__AVX2__` is defined only at v3 or above, and every 32-byte vector type in
# vppinfra sits behind `#if defined (__AVX2__)` (`vppinfra/vector.h:197`). The findings sweeps
# drive `chiero` with no `-march` at all, so **384 of VPP's 1967 compilations — 192 at v3, 192 at
# v4 — describe code no sweep has ever analysed**, including the code 021 §5 cites when it says
# vppinfra uses `u8x32`/`u8x64` throughout.
#
# What it reports per unit: definitions lowered without `-march`, definitions lowered with the
# unit's own `-march`, and the first diagnostic if there is one.
#
#   march_probe.sh            # every 16th v3/v4 unit (24 of 384), a few minutes
#   STRIDE=1 march_probe.sh   # all 384
#
# ⚠️ **Count definitions, not `^func` lines.** `func` also introduces a *declaration*, of which a
# VPP TU has thousands, and the totals barely move when the vector headers appear. Measured with
# `grep -c '^func'` the v3 and no-march runs came out **byte-identical at 5566**, and that would
# have been written up as "the widening measured nothing". The definition marker is `{ ; span`,
# and by it the same file goes 5560 -> 5852. **An instrument that reports a plausible number is
# not a measurement.**
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
CHIERO=${CHIERO:-$HERE/../../../target/release/chiero}
VPP=${VPP:-/home/ubuntu/vpp}
VPPBUILD=${VPPBUILD:-$VPP/build-root/build-vpp-native}
STRIDE=${STRIDE:-16}
TIMEOUT=${TIMEOUT:-120}

[ -x "$CHIERO" ] || { echo "no chiero at $CHIERO (cargo build --release -p chiero-cli)" >&2; exit 2; }

db=$(cd "$VPPBUILD/vpp" && ninja -t compdb 2>/dev/null)
[ -n "$db" ] || { echo "no compile database from $VPPBUILD/vpp" >&2; exit 2; }

echo "$db" | STRIDE=$STRIDE CHIERO=$CHIERO TIMEOUT=$TIMEOUT python3 -c '
import json, os, shlex, subprocess, sys

rows = []
for e in json.load(sys.stdin):
    cmd = e.get("command", "")
    if not cmd or not e.get("file", "").endswith(".c"):
        continue
    args = shlex.split(cmd)
    march = [a for a in args if a.startswith("-march=")]
    # The LAST -march wins, as it does for the compiler: one VPP unit names it twice.
    if not march or not any(v in march[-1] for v in ("v3", "v4")):
        continue
    rows.append((e["file"], march[-1].split("=")[1],
                 [a for a in args if a.startswith(("-I", "-D"))]))

stride = int(os.environ["STRIDE"])
chiero, timeout = os.environ["CHIERO"], os.environ["TIMEOUT"]
sample = rows[::stride]
print(f"{len(rows)} v3/v4 units, probing {len(sample)} (STRIDE={stride})")

def lower(path, flags, march):
    cmd = [chiero, "cir"] + (["--march", march] if march else []) + [path] + flags
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=int(timeout))
    except subprocess.TimeoutExpired:
        return None, "timeout"
    diag = next((l for l in (r.stdout + r.stderr).splitlines() if l.startswith("chiero:")), "")
    return sum(1 for l in r.stdout.splitlines() if "{ ; span" in l), diag

bad = empty = 0
for path, march, flags in sample:
    base, _ = lower(path, flags, None)
    with_m, diag = lower(path, flags, march)
    name = os.path.relpath(path, f"{os.environ.get('"'"'VPP'"'"', '"'"'/home/ubuntu/vpp'"'"')}/src")
    if diag:
        bad += 1
        print(f"  DIAGNOSED {march} {name}: {diag[:140]}")
    elif not with_m:
        # **A clean run that lowered nothing is not a pass, it is an empty analysis.** VPP guards
        # whole files on a variant this configuration does not select — `vppinfra/test/aes_cbc.c`
        # preprocesses to six non-blank lines under its own flags — and reporting that as `ok`
        # is how a sweep comes to claim coverage of code it never read (HANDOFF §9.1).
        empty += 1
        print(f"  EMPTY {march} {name}: lowered no definitions at all")
    else:
        delta = (with_m or 0) - (base or 0)
        print(f"  ok {march} {name}: {base} -> {with_m} ({delta:+d} definitions)")
print(f"{bad} of {len(sample)} diagnosed, {empty} lowered nothing")
sys.exit(1 if bad else 0)
'
