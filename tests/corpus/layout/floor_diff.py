"""Is chiero's proposed floor ever below what gcc can actually reach?

For each random struct: chiero's `would be M`, against the minimum sizeof over every
permutation of its *units* (a bit-field run plus any trailing `:0` moves as one unit,
because that is the reorder 041 section 3.1 proposes).
"""
import itertools, json, random, subprocess, sys, os, tempfile
CH = os.environ.get("CHIERO", os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../target/release/chiero"))
S = tempfile.mkdtemp(prefix="chiero-floor-")   # scratch; the repo is not a build directory
random.seed(int(sys.argv[1]) if len(sys.argv) > 1 else 7)
TYPES = [("char", 8), ("short", 16), ("unsigned", 32), ("long long", 64)]
over = same = 0
for case in range(int(sys.argv[2]) if len(sys.argv) > 2 else 40):
    units = []
    for i in range(random.randint(2, 5)):
        kind = random.choice(["scalar", "bits", "bits0"])
        if kind == "scalar":
            t = random.choice(["char", "short", "int", "long", "double"])
            units.append(f"{t} s{i};")
        else:
            t, w = random.choice(TYPES)
            n = random.randint(1, 3)
            decl = " ".join(f"{t} b{i}_{j} : {random.randint(1, max(1, w // n))};" for j in range(n))
            if kind == "bits0":
                decl += f" {t} : 0;"
            units.append(decl)
    body = " ".join(units)
    src = f"struct G {{ {body} }};\nstruct G g;\n"
    open(f"{S}/fd.c", "w").write(src)
    o = subprocess.run([CH, "layout", f"{S}/fd.c", "--json"], capture_output=True, text=True)
    try:
        d = json.loads(o.stdout)
    except Exception:
        continue
    rec = next((r for r in d["result"]["records"] if r["tag"] == "G"), None)
    if rec is None:
        continue
    pad = next((p for p in rec["proposals"] if p["kind"] == "padding_waste"), None)
    if pad is None:
        continue
    floor = rec["size"] - pad["recoverable"]
    perms = list(itertools.permutations(units))
    random.shuffle(perms)
    prog = "#include <stdio.h>\n"
    for k, perm in enumerate(perms[:120]):
        prog += "struct P%d { %s };\n" % (k, " ".join(perm))
    prog += "int main(void){size_t m=(size_t)-1;\n"
    for k in range(len(perms[:120])):
        prog += "if(sizeof(struct P%d)<m)m=sizeof(struct P%d);\n" % (k, k)
    prog += 'printf("%zu\\n",m);}\n'
    open(f"{S}/fd_perm.c", "w").write(prog)
    if subprocess.run(["gcc", "-w", "-o", f"{S}/fd_perm", f"{S}/fd_perm.c"]).returncode:
        continue
    best = int(subprocess.run([f"{S}/fd_perm"], capture_output=True, text=True).stdout)
    if floor < best:
        over += 1
        print(f"OVER-CLAIM  chiero floor {floor}, gcc best {best}  size {rec['size']}\n  {src.strip()}")
    else:
        same += 1
print(f"checked {over + same} proposals: {over} over-claims, {same} sound")
