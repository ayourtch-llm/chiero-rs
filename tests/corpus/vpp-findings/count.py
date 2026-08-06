import json,sys
f,fn,path=sys.argv[1],sys.argv[2],sys.argv[3]
try: e=json.load(open(path))
except Exception: print("%s\t%s\tunparsed\t0\t0"%(f,fn)); raise SystemExit
fs=e.get("result",{}).get("findings",[])
ex=sum(1 for x in fs if x.get("fidelity")=="Exact")
print("%s\t%s\tok\t%d\t%d"%(f,fn,len(fs),ex))
