"""Regenerate `entries.tsv`: the first six functions defined in each of seven VPP files.

Mechanical on purpose. Picking the functions by hand measures the picker, and keeping the ones
that produced interesting findings measures nothing at all.

    python3 pick_entries.py > entries.tsv        # VPP=/home/ubuntu/vpp by default
"""

import os
import re
import sys

VPP = os.environ.get("VPP", "/home/ubuntu/vpp")

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


def main():
    out = []
    for f in FILES:
        path = os.path.join(VPP, "src", f)
        names = set()
        with open(path, errors="replace") as fh:
            for line in fh:
                m = DEFINITION.match(line)
                if m and not line.rstrip().endswith(";"):
                    names.add(m.group(1))
        # Sorted, not source order: the file's own order is not stable across VPP versions,
        # and a sample that reshuffles when upstream moves a function is not a fixed sample.
        for name in sorted(names)[:PER_FILE]:
            out.append((f, name))
    for f, name in out[:TOTAL]:
        print(f"{f}\t{name}")


if __name__ == "__main__":
    sys.exit(main())
