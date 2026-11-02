import json,subprocess,sys
PIN="/sys/fs/bpf/pigvgw"
def dump(name,kf,vf):
    for e in json.loads(subprocess.check_output(["bpftool","map","dump","pinned",f"{PIN}/{name}","-j"])):
        f=e.get("formatted",e)
        print(kf(f["key"]), vf(f["value"]))
def b(x): return bytes(int(h,16) for h in x)
raw=json.loads(subprocess.check_output(["bpftool","map","dump","pinned",f"{PIN}/{sys.argv[1]}","-j"]))
for e in raw:
    k=e["key"]; v=e["value"]
    if sys.argv[1]=="SEEN_COMM":
        print("comm=%-18s connects=%d"%(b(k).split(b"\0")[0].decode(errors="replace"), int.from_bytes(b(v),"little")))
    else:
        print(int.from_bytes(b(k),"little"), int.from_bytes(b(v),"little"))
