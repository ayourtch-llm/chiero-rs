#!/usr/bin/env python3
"""Diff two `measure.sh KEEP=` directories, and **exclude the envelopes that cannot be
compared**.

`--time-budget` is a wall clock, so an entry that hits it stops wherever the machine got to.
chiero says so: the envelope carries `"nondeterministic_abort": true`, and only there — the 38
of the pinned 40 that finish carry `false`. Comparing the flagged ones measures the machine.

Measured 2026-08-09, three runs of one binary on `clib_mem_create_heap`: **22, 23 and 24**
states left unexplored. On 2026-08-09 an envelope diff was reported as "5 of 40 differ" when
two of the five were this pair; the conclusion survived, the number did not.

    ./compare.py BEFORE_DIR AFTER_DIR

Exit 0 always: this is a report, like `measure.sh`.
"""

import json
import pathlib
import sys


def aborted(path):
    try:
        return bool(json.loads(path.read_text()).get("nondeterministic_abort"))
    except (OSError, ValueError):
        # An unreadable envelope is not a comparable one, and saying so beats guessing.
        return True


def main(argv):
    if len(argv) != 3:
        print(__doc__)
        return 0
    before, after = pathlib.Path(argv[1]), pathlib.Path(argv[2])
    changed, same, skipped, missing = [], 0, [], []
    for a in sorted(after.glob("*.json")):
        b = before / a.name
        if not b.exists():
            missing.append(a.name)
            continue
        if aborted(a) or aborted(b):
            skipped.append(a.name)
            continue
        if a.read_bytes() == b.read_bytes():
            same += 1
        else:
            changed.append(a.name)

    for n in changed:
        print(f"DIFFERS  {n}")
    for n in missing:
        print(f"MISSING in before  {n}")
    print(f"\ncomparable: {same + len(changed)}  identical: {same}  differing: {len(changed)}")
    # **Never silent about what was dropped.** A comparison that quietly skips entries reads as
    # "everything matched", which is the failure this script exists to prevent.
    if skipped:
        print(f"excluded {len(skipped)} envelope(s) marked `nondeterministic_abort` — "
              "their stopping point depends on the machine, so a diff there is noise:")
        for n in skipped:
            print(f"  - {n}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
