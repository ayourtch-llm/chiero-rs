"""Numbered lists in HANDOFF.md must not repeat or go backwards.

HANDOFF.md is edited incrementally, wave after wave, and on 2026-08-09 §9's START HERE —
the first block a fresh context reads — had accumulated **two items numbered `3.`**, two
overlapping "closed" entries and a stale count. Nobody had looked, because nothing looked.

A duplicate number is the cheapest symptom of that drift and the only one a machine can
see. It is not a proof the prose is current; it is one check that costs nothing.

    python3 tests/corpus/handoff/numbering.py [FILE]     exits 1 on an anomaly

⚠️ **Verified against the broken revision, not only the fixed one.** `git show HEAD~1` of
the repair commit reports the duplicate at line 1793; the repaired file reports none. A
checker nobody has seen fail is a checker nobody should trust.
"""
import re, sys

path = sys.argv[1] if len(sys.argv) > 1 else "HANDOFF.md"
prev, issues = {}, []
for i, line in enumerate(open(path).read().split("\n"), 1):
    m = re.match(r"^(>?\s*)(\d+)([a-z]?)\.\s", line)
    if not m:
        # A blank line or a heading ends the list context, so unrelated lists in
        # different sections are not compared with each other.
        if not line.strip() or line.startswith("#") or line.startswith("> #"):
            prev.clear()
        continue
    indent, num, suffix = m.group(1), int(m.group(2)), m.group(3)
    depth = len(indent.replace(">", " "))
    last = prev.get(depth)
    # `1z.`/`4b.`/`5m.` are deliberate — this file uses them to keep a closed item's id
    # stable while inserting beside it — so only bare numbers are ordered.
    if last is not None and not suffix and num <= last:
        issues.append(f"  line {i}: `{num}.` follows `{last}.` — {line.strip()[:70]}")
    if not suffix:
        prev[depth] = num

if issues:
    print(f"{path}: {len(issues)} numbering anomal(ies)")
    print("\n".join(issues))
    sys.exit(1)
print(f"{path}: numbered lists are consistent")
