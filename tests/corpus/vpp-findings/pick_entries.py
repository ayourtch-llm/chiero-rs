"""Regenerate `entries.tsv`: the first six functions defined in each of seven VPP files.

Mechanical on purpose. Picking the functions by hand measures the picker, and keeping the ones
that produced interesting findings measures nothing at all.

    python3 pick_entries.py > entries.tsv        # VPP=/home/ubuntu/vpp by default

Any other file set is an *exploration* rather than the pinned sample, and says so by naming
its files:

    python3 pick_entries.py --per-file 4 plugins/nat/nat44-ed/*.c > /tmp/nat.tsv
    LIST=/tmp/nat.tsv ./measure.sh

Paths are relative to `$VPP/src`, and shell globs work because the shell expands them against
that directory only if you are standing in it — so absolute paths under `$VPP/src` are accepted
too and relativised here.
"""

import os
import re
import subprocess
import sys

VPP = os.environ.get("VPP", "/home/ubuntu/vpp")
SRC = os.path.join(VPP, "src")

FILES = [
    "vppinfra/bitmap.c",
    "vppinfra/mem_dlmalloc.c",
    "vppinfra/hash.c",
    "vppinfra/vec.c",
    "vppinfra/time.c",
    "vlib/node_cli.c",
    "vlib/counter.c",
]

PER_FILE = 6
TOTAL = 40

# VPP's style puts the return type on its own line and the function name at column 0, so a
# definition is a line starting with an identifier followed by `(`. A declaration ends in `;`.
DEFINITION = re.compile(r"^(\w+)\s*\(")

# **An all-caps name at column 0 is a registration macro, not a function.** Plugin sources are
# full of `VLIB_PLUGIN_REGISTER (...)`, `VLIB_CLI_COMMAND (...)`, `VNET_FEATURE_INIT (...)`,
# every one of which matches the shape above. Feeding one to `--entry` asks for a function that
# does not exist — and the honest envelope that comes back ("no function named X") counted as a
# clean run in the harness, so a list full of them measured nothing while reporting `ok`.
# The rule is a convention rather than a parse: VPP writes functions in lower case.
MACRO_NAME = re.compile(r"^[A-Z0-9_]+$")


def compiled_sources():
    """The files VPP's build actually compiles, relative to `$VPP/src`.

    **`failed` means two different things without this.** A sweep list built by globbing the
    tree includes source the build never touches, and some of it does not compile at all:
    `vnet/fib/fib_entry_src_default.c` defines one function twice, `vnet/unix/pcap2pg.c` calls
    another with no declaration in scope, and gcc rejects both. Rows like those are
    indistinguishable, in the output, from a construct chiero cannot read — so the residue
    blends "chiero cannot read this" with "nothing can", and neither number means anything.

    `ninja -t commands all` is the authoritative answer and takes about 63 ms for VPP's 2945
    commands. The compiler is told `-c <source>`, so that is what is read here rather than any
    path this script could construct: object paths cannot be derived from source paths under
    CMake object libraries, which is a trap `probe.sh` documents at length.
    """
    build = os.environ.get(
        "VPPBUILD", os.path.join(VPP, "build-root", "build-vpp-native")
    )
    ninja_dir = os.path.join(build, "vpp")
    try:
        out = subprocess.run(
            ["ninja", "-C", ninja_dir, "-t", "commands", "all"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError) as e:
        # **Refuse rather than silently keep everything.** Falling back to "no filter" would
        # turn a missing build directory into a sweep that quietly measures the wrong corpus,
        # which is the failure this option exists to end.
        sys.exit(f"pick_entries: --built-only needs a ninja build at {ninja_dir}: {e}")
    seen = set()
    for line in out.splitlines():
        parts = line.split()
        for i, tok in enumerate(parts):
            if tok == "-c" and i + 1 < len(parts):
                src = parts[i + 1]
                if src.startswith(SRC + os.sep):
                    seen.add(os.path.relpath(src, SRC))
    return seen


def entries(files, per_file, total):
    out = []
    for f in files:
        path = os.path.join(SRC, f)
        names = set()
        try:
            fh = open(path, errors="replace")
        except OSError as e:
            # **Named, not skipped.** A file the sample could not read is a hole in the
            # sample, and a list that quietly omits it measures fewer functions than it says.
            print("%s: %s" % (f, e), file=sys.stderr)
            continue
        with fh:
            for line in fh:
                m = DEFINITION.match(line)
                if m and not line.rstrip().endswith(";") and not MACRO_NAME.match(m.group(1)):
                    names.add(m.group(1))
        # Sorted, not source order: the file's own order is not stable across VPP versions,
        # and a sample that reshuffles when upstream moves a function is not a fixed sample.
        for name in sorted(names)[:per_file]:
            out.append((f, name))
    return out if total is None else out[:total]


def main():
    args = sys.argv[1:]
    per_file, total = PER_FILE, TOTAL
    built_only = False
    files = []
    i = 0
    while i < len(args):
        if args[i] == "--built-only":
            built_only = True
        elif args[i] == "--per-file":
            per_file, i = int(args[i + 1]), i + 1
        elif args[i] == "--total":
            total, i = int(args[i + 1]), i + 1
        else:
            p = args[i]
            if os.path.isabs(p):
                p = os.path.relpath(p, SRC)
            files.append(p)
        i += 1
    # The pinned sample keeps its cap; a named file set does not, because its size is the
    # thing the caller chose.
    if files:
        total = None if total == TOTAL else total
    chosen = files or FILES
    if built_only:
        compiled = compiled_sources()
        kept = [f for f in chosen if f in compiled]
        # **Say what was dropped.** A filter that silently shrinks the corpus turns "we swept
        # the tree" into a claim nobody can check, and the count is the whole point of the
        # option: it is the number of files in the tree that VPP does not build.
        print(
            f"pick_entries: --built-only kept {len(kept)} of {len(chosen)} file(s); "
            f"{len(chosen) - len(kept)} are not compiled by this build",
            file=sys.stderr,
        )
        chosen = kept
    for f, name in entries(chosen, per_file, total):
        print(f"{f}\t{name}")


if __name__ == "__main__":
    sys.exit(main())
