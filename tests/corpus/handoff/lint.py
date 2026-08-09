"""Mechanical checks on HANDOFF.md and the specs — the drift nothing else compares.

Five record-vs-tree checks were run by hand on 2026-08-09 and **all five found something**:
a phantom `CallConv` field in 020, a stray code fence hiding 45 lines of it, two items
numbered `3.` in §9's START HERE, six contracts tested but uncited, and two cited paths that
did not exist. The record drifts wherever nothing mechanically compares it to the tree, and
every one of these was a one-command check nobody had run.

Three of the five are automatable and are here. The other two are not:
  * **contract citations** already have a tool — `cargo xtask contract-coverage`.
  * **"is the prose still true"** is not mechanisable at all, and nothing here pretends to
    check it. These catch shapes, not claims.

    python3 tests/corpus/handoff/lint.py        exits 1 on any finding

⚠️ Each check was verified against a revision that **fails** it, not only against a passing
tree — a checker nobody has seen fail is a checker nobody should trust.
"""
import os
import re
import sys

# `<root>/tests/corpus/handoff/lint.py` — four levels up, not three. The first attempt
# stopped at `tests/` and looked for `tests/HANDOFF.md`.
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
HANDOFF = os.path.join(ROOT, "HANDOFF.md")
SPECS = os.path.join(ROOT, "docs", "specs")


def numbering(path):
    """Numbered lists must not repeat or go backwards.

    §9's START HERE carried two items numbered `3.` for thirty-five waves. Suffixed ids
    (`1z.`, `4b.`, `5m.`) are deliberate — this file uses them to keep a closed item's number
    stable while inserting beside it — so only bare numbers are ordered.
    """
    prev, out = {}, []
    for i, line in enumerate(open(path).read().split("\n"), 1):
        m = re.match(r"^(>?\s*)(\d+)([a-z]?)\.\s", line)
        if not m:
            if not line.strip() or line.startswith("#") or line.startswith("> #"):
                prev.clear()
            continue
        indent, num, suffix = m.group(1), int(m.group(2)), m.group(3)
        depth = len(indent.replace(">", " "))
        last = prev.get(depth)
        if last is not None and not suffix and num <= last:
            out.append(f"line {i}: `{num}.` follows `{last}.` — {line.strip()[:66]}")
        if not suffix:
            prev[depth] = num
    return out


def paths_resolve(path):
    """Every repo-shaped path cited in the file must exist.

    `tests/growth.rs` and `xtask/sweep.rs` were abbreviations in a document that otherwise
    carries exact invocations, so a reader following either got nothing.
    """
    out = []
    text = open(path).read()
    for cited in sorted(
        set(re.findall(r"`((?:tests|crates|xtask|docs|\.deploy)/[A-Za-z0-9_./-]+)`", text))
    ):
        if not os.path.exists(os.path.join(ROOT, cited)):
            out.append(f"cited path does not exist: {cited}")
    return out


def fences(path):
    """Code fences must balance.

    An odd count is impossible for well-formed markdown. 020 had one, and the stray fence was
    rendering 45 lines of normative prose — including the `Opaque` rationale — as a code block.
    """
    n = open(path).read().count("```")
    return [f"{n} code fences — an odd count cannot balance"] if n % 2 else []


def main():
    findings = []
    for check in (numbering, paths_resolve, fences):
        for f in check(HANDOFF):
            findings.append(("HANDOFF.md", f))
    # The specs get the fence check only: they carry no repo paths in backticks and their
    # numbered lists *are* the contract ids, which `contract-coverage` already reasons about.
    for name in sorted(os.listdir(SPECS)):
        if name.endswith(".md"):
            for f in fences(os.path.join(SPECS, name)):
                findings.append((f"docs/specs/{name}", f))

    if findings:
        print(f"{len(findings)} finding(s):")
        for where, what in findings:
            print(f"  {where}: {what}")
        return 1
    print("HANDOFF.md and docs/specs: numbering, cited paths and fences all consistent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
