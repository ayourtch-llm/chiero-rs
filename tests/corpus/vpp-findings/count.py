"""One TSV row per entry point, from the envelope `find-bugs` printed.

`ok` and `cut` are deliberately different words. A run the wall clock ended found what it
found and searched less than the whole function, so its `0 findings` is not the `0 findings`
of a run that finished — which is the same distinction the envelope itself is built around.
"""

import json, sys

f, fn, path = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    e = json.load(open(path))
except Exception:
    print("%s\t%s\tunparsed\t0\t0" % (f, fn))
    raise SystemExit
fs = e.get("result", {}).get("findings", [])
ex = sum(1 for x in fs if x.get("fidelity") == "Exact")
status = "cut" if e.get("nondeterministic_abort") else "ok"
print("%s\t%s\t%s\t%d\t%d" % (f, fn, status, len(fs), ex))
