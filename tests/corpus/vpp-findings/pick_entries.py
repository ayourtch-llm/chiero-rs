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
    files = []
    i = 0
    while i < len(args):
        if args[i] == "--per-file":
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
    for f, name in entries(files or FILES, per_file, total):
        print(f"{f}\t{name}")


if __name__ == "__main__":
    sys.exit(main())
