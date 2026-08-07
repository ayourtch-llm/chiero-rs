import itertools, json, subprocess, sys, os, tempfile
CH = os.environ.get("CHIERO", os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../target/release/chiero"))
S = tempfile.mkdtemp(prefix="chiero-floor-")   # scratch; the repo is not a build directory
CASES = {
 "Q_two_zero_width_runs": ["unsigned a:1; unsigned :0;", "char c;", "unsigned b:1; unsigned :0;", "char d;"],
 "one_zero_width_run":    ["unsigned a:1; unsigned :0;", "char c;", "long L;"],
 "trailing_zero_width":   ["int i;", "char c;", "unsigned b:1; unsigned :0;", "char d;", "char e;", "char f;", "char g;", "char h;"],
 "no_zero_width":         ["char tag;", "int big;", "unsigned a:1; unsigned b:1; unsigned c:1; unsigned d:1;"],
}
for name, units in CASES.items():
    src = "struct G { %s };\nstruct G g;\n" % " ".join(units)
    open(f"{S}/fx.c","w").write(src)
    d = json.loads(subprocess.run([CH,"layout",f"{S}/fx.c","--json"],capture_output=True,text=True).stdout)
    rec = next(r for r in d["result"]["records"] if r["tag"]=="G")
    pad = next((p for p in rec["proposals"] if p["kind"]=="padding_waste"), None)
    perms = list(itertools.permutations(units))
    prog = "#include <stdio.h>\n" + "".join("struct P%d { %s };\n"%(k," ".join(p)) for k,p in enumerate(perms))
    prog += "int main(void){size_t m=(size_t)-1;\n" + "".join("if(sizeof(struct P%d)<m)m=sizeof(struct P%d);\n"%(k,k) for k in range(len(perms)))
    prog += 'printf("%zu\\n",m);}\n'
    open(f"{S}/fx_perm.c","w").write(prog)
    subprocess.run(["gcc","-w","-o",f"{S}/fx_perm",f"{S}/fx_perm.c"],check=True)
    best = int(subprocess.run([f"{S}/fx_perm"],capture_output=True,text=True).stdout)
    if pad is None:
        print(f"{name:24} size {rec['size']:3}  no proposal              gcc best {best}")
    else:
        floor = rec["size"] - pad["recoverable"]
        flag = "  <== OVER-CLAIM" if floor < best else ""
        print(f"{name:24} size {rec['size']:3}  floor {floor:3}  gcc best {best}{flag}")
