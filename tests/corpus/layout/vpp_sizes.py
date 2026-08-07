"""Does chiero's layout agree with gcc on real VPP headers the gate does not cover?

014 contract 12's method — generate `_Static_assert`s and let the compiler that defines
the ABI reject them — pointed at headers outside `CORPUS_SEEDS`. Written to measure the
blast radius of the unnamed-bit-field alignment fix; kept because the answer is a number
somebody else can check.
"""
import json, subprocess, sys, os, tempfile

VPP = "/home/ubuntu/vpp"
CH = os.environ.get("CHIERO", "/home/ubuntu/rust/chiero-rs/target/release/chiero")
INC = ["-I", "src", "-I", "build-root/install-vpp-native/vpp/include"]
S = tempfile.mkdtemp(prefix="chiero-vpp-sizes-")

for header in sys.argv[1:]:
    o = subprocess.run([CH, "layout", header, "--json"] + INC,
                       capture_output=True, text=True, cwd=VPP)
    try:
        d = json.loads(o.stdout)
    except Exception:
        print(f"{header}: chiero could not answer: {o.stderr.strip()[:120]}")
        continue
    recs = d["result"]["records"]
    # `_Static_assert` on a tag the header does not actually define would be a compile
    # error about the wrong thing, so the assertions are generated per record and the
    # failures are read one at a time.
    src = f'#include <{header[4:] if header.startswith("src/") else header}>\n'
    for i, r in enumerate(recs):
        src += (f'_Static_assert(sizeof(struct {r["tag"]}) == {r["size"]},'
                f' "size {r["tag"]}");\n'
                f'_Static_assert(_Alignof(struct {r["tag"]}) == {r["align"]},'
                f' "align {r["tag"]}");\n')
    p = os.path.join(S, "check.c")
    open(p, "w").write(src)
    g = subprocess.run(["gcc", "-fsyntax-only", "-w", "-std=gnu11",
                        "-march=x86-64-v2", "-I", "src",
                        "-I", "build-root/install-vpp-native/vpp/include", p],
                       capture_output=True, text=True, cwd=VPP)
    bad = [l for l in g.stderr.splitlines() if "static assertion failed" in l]
    print(f"{header}: {len(recs)} records, {len(bad)} disagree with gcc")
    for l in bad[:10]:
        print("   ", l.strip()[:150])
