import os, re, subprocess, sys, tempfile, shutil
B = sys.argv[1]
DUMP = "target/release/examples/dump"
objs = []
for r, _, fs in os.walk(B):
    for f in fs:
        if f.endswith(".gcno") and os.path.exists(os.path.join(r, f[:-5] + ".gcda")):
            objs.append((r, f[:-5]))
objs.sort()
agree = checked = rows = wrong = 0
examples = []
for d, stem in objs:
    tmp = tempfile.mkdtemp()
    try:
        for ext in ("gcno", "gcda"):
            shutil.copy(os.path.join(d, f"{stem}.{ext}"), os.path.join(tmp, f"{stem}.{ext}"))
        p = subprocess.run(["llvm-cov", "gcov", f"{stem}.gcda"], cwd=tmp, capture_output=True)
        want = {}
        for g in os.listdir(tmp):
            if not g.endswith(".gcov"): continue
            src = None
            for line in open(os.path.join(tmp, g), errors="replace"):
                m = re.match(r"^\s*-:\s*0:Source:(.*)$", line)
                if m: src = m.group(1).strip(); continue
                m = re.match(r"^\s*([0-9]+|#####|=====|\$\$\$\$\$|-):\s*([0-9]+):", line)
                if not m or src is None: continue
                c, ln = m.group(1), int(m.group(2))
                if c == "-" or ln == 0: continue
                want[(src, ln)] = 0 if c[0] in "#=$" else int(c)
        got = {}
        out = subprocess.run([DUMP, d, stem], capture_output=True, text=True)
        if out.returncode != 0:
            examples.append(f"INGEST {stem}: {out.stderr.strip()[:110]}"); continue
        for line in out.stdout.splitlines():
            f, ln, c = line.split("\t"); got[(f, int(ln))] = int(c)
        checked += 1
        same = True
        for k, v in want.items():
            rows += 1
            if got.get(k) != v:
                wrong += 1; same = False
                if len(examples) < 8: examples.append(f"{stem} {k[0]}:{k[1]} chiero={got.get(k)} llvm-cov={v}")
        if same: agree += 1
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
print(f"cross-validated against llvm-cov: {agree}/{checked} objects agree")
print(f"                                  {wrong} of {rows} lines differ")
for e in examples: print("  ", e)
