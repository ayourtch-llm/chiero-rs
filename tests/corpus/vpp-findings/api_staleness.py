#!/usr/bin/env python3
"""Which of VPP's generated API headers are older than the `.api` they come from.

**A stale generated header makes chiero read a program VPP does not build**, and it looks
exactly like a frontend defect: three sweep rows once failed with *"no member named
`last_heard_age`"* and the answer was that `plugins/lldp/lldp.api` had been updated 17 minutes
after the build directory was generated. gcc reported the identical error on the identical line,
which is what said it was an environment fact rather than a chiero one.

Measured 2026-08-08 on this machine: **165 of 2629 sources under `src/` are newer than the whole
build**, all at one timestamp — a single checkout 22 seconds after the build finished. Of those,
4 were `.api` files, and only those 4 mattered:

  * chiero reads `src/` **directly**, so a `.c` or `.h` that moved on is read as it is today;
  * the only derived artifacts it includes are the **1049 `*.api*.h`** headers, plus four
    `config.h`/`version.h` that come from cmake options rather than from source.

So the staleness surface is exactly what this script measures, and it is not the whole tree —
which is the correction to HANDOFF §9.1's original, broader statement of the problem.

    api_staleness.py            # report; exit 1 if anything is stale
    api_staleness.py --fix      # regenerate the stale ones with vppapigen, then re-report

⚠️ **`--fix` deliberately does not run ninja.** The ninja target for a generated header depends on
a cmake re-run, which rewrites `build.ninja` — and `build.ninja` is what `chiero_vpp::builddb`
reads for all 1967 compile commands, what `probe.sh` replays, and what 012 contract 17's corpus
gate is built from. Running `vppapigen` directly is the same command ninja would run for that one
output, with nothing else moving.
"""

import os
import subprocess
import sys

SRC = "/home/ubuntu/vpp/src"
BUILD = "/home/ubuntu/vpp/build-root/build-vpp-native/vpp"
GEN = os.path.join(BUILD, "CMakeFiles")
VPPAPIGEN = os.path.join(SRC, "tools/vppapigen/vppapigen")


def find(root, *args):
    return subprocess.run(
        ["find", root, *args], capture_output=True, text=True, check=False
    ).stdout.split()


def stale():
    """(api path, generated header, seconds of drift) for every `.api` whose header is older."""
    by_name = {}
    for g in find(GEN, "-name", "*.api_types.h"):
        by_name.setdefault(os.path.basename(g)[: -len("_types.h")], []).append(g)
    out, ungenerated = [], []
    for a in find(SRC, "-name", "*.api"):
        gs = by_name.get(os.path.basename(a))
        if not gs:
            # A plugin this configuration does not build — netmap, marvell/pp2, the sample
            # plugin. Not staleness: there is nothing to be stale.
            ungenerated.append(a)
            continue
        drift = os.path.getmtime(a) - os.path.getmtime(gs[0])
        if drift > 0:
            out.append((a, gs[0], drift))
    return out, ungenerated


def regenerate(api, header):
    """The command `ninja` would run for this one output, and nothing else."""
    outdir = os.path.dirname(header)
    env = dict(os.environ, PYTHONPYCACHEPREFIX=os.path.join(GEN, "__pycache__"))
    base = os.path.basename(api)[: -len(".api")]
    r = subprocess.run(
        [
            VPPAPIGEN,
            "--includedir", SRC,
            "--input", api,
            "--outputdir", outdir,
            "--output", os.path.join(outdir, f"{base}.api.h"),
        ],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    return r.returncode, (r.stderr or "").strip()


def fingerprint():
    """A stable identity for the **generated** half of the corpus.

    VPP's `HEAD` does not pin the corpus: 147 of the 1562 sources the corpus gate reads are
    generated into the build directory and are not in git. On 2026-08-09 a published token
    count moved by 10 972 between two sessions with `HEAD` unchanged and `git status` clean,
    and the cause was 32 regenerated API headers — which looked exactly like a chiero change.

    Content, not mtime: a `touch` must not move this, and an edit must. Printed as a short
    digest so it can sit beside a published figure without dominating it.
    """
    import hashlib

    h, n = hashlib.sha256(), 0
    for root, _, files in os.walk(GEN):
        for name in sorted(files):
            if not name.endswith((".h", ".c")):
                continue
            path = os.path.join(root, name)
            # The path matters as much as the bytes: a header that disappears changes the
            # corpus even if every surviving file is identical.
            h.update(os.path.relpath(path, GEN).encode())
            try:
                with open(path, "rb") as f:
                    h.update(f.read())
            except OSError:
                h.update(b"<unreadable>")
            n += 1
    return n, h.hexdigest()[:16]


def main():
    if "--fingerprint" in sys.argv[1:]:
        n, digest = fingerprint()
        print(f"generated corpus: {n} files, sha256:{digest}")
        print("  record this beside any published VPP number — `HEAD` does not pin it")
        return 0
    fix = "--fix" in sys.argv[1:]
    bad, ungenerated = stale()
    print(f"{len(bad)} stale, {len(ungenerated)} .api with no generated header (not built here)")
    for api, header, drift in sorted(bad, key=lambda x: -x[2]):
        print(f"  +{drift / 60:7.1f} min  {os.path.relpath(api, SRC)}")
        if fix:
            code, err = regenerate(api, header)
            print(f"      regenerated -> exit {code} {err[:120]}")
    if fix and bad:
        bad, _ = stale()
        print(f"after --fix: {len(bad)} stale")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
