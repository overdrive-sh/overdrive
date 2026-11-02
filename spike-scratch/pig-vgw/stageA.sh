#!/usr/bin/env bash
# V-GW-2 / V-GW-5 / V-GW-4(load). Serve + VM Service already up (env.sh run healthy).
set -u
. "$(dirname "$0")/common.sh"
stamp "stageA start"
pgrep -af 'overdrive serve' | cut -c1-120
SP=$(pgrep -f 'overdrive serve' | head -1)
mkdir -p $PIN
SM=$(bpftool map show | awk '/name SERVICE_MAP/{sub(":","",$1);print $1}'); BM=$(bpftool map show | awk '/name BACKEND_MAP/{sub(":","",$1);print $1}')
echo "SERVICE_MAP id=$SM BACKEND_MAP id=$BM"
rm -f $PIN/* 2>/dev/null
bpftool map pin id "$SM" $PIN/SERVICE_MAP; bpftool map pin id "$BM" $PIN/BACKEND_MAP
mkdir -p $NEG_CG
stamp "V-GW-2 cgroup membership of serve ($SP)"
echo "attach cgroup (production): /sys/fs/cgroup/overdrive.slice"
cat /proc/$SP/cgroup
echo "threads by (cgroup, comm):"
for t in /proc/$SP/task/*; do echo "$(cat $t/cgroup) $(cat $t/comm)"; done | sort | uniq -c
stamp "production programs attached under the slice"
bpftool cgroup tree /sys/fs/cgroup/overdrive.slice
stamp "production verified_insns (bpftool prog show -j)"
bpftool prog show -j | python3 -c '
import json,sys
for p in json.load(sys.stdin):
    if p.get("name","").startswith("cgroup_"): print(p.get("id"),p.get("name"),p.get("type"),"verified_insns=",p.get("verified_insns"),"xlated=",p.get("bytes_xlated"))'
stamp "load+attach vgw_cookie at overdrive.slice (alongside production, multi)"
nohup $VGW $ELF vgw_cookie /sys/fs/cgroup/overdrive.slice $PIN > $OUT/loader-cookie.log 2>&1 &
LP=$!
sleep 3; cat $OUT/loader-cookie.log
bpftool cgroup tree /sys/fs/cgroup/overdrive.slice
stamp "V-GW-5 cookie equality, plain sockets, inside serve's cgroup"
$DIAL cookie --n 10 --cgroup $SERVE_CG
stamp "V-GW-5 cookie equality, prepared sockets (SO_MARK=1 + bind lo + NO_PORT)"
$DIAL cookie --n 10 --mark-bind --cgroup $SERVE_CG
stamp "V-GW-2 positive/negative controls via SEEN counters"
dump_seen() { bpftool map dump pinned $PIN/SEEN_COMM -j 2>/dev/null | python3 -c '
import json,sys
for e in json.load(sys.stdin):
    k=bytes(int(x,16) for x in e["key"]).split(b"\0")[0].decode(errors="replace"); v=int.from_bytes(bytes(int(x,16) for x in e["value"]),"little"); print("  comm=%-18s connects=%d"%(k,v))'; }
echo "SEEN_COMM now:"; dump_seen
echo "port 45001 count before:"; bpftool map lookup pinned $PIN/SEEN key 0xc9 0xaf 0 0 2>&1 | head -2
$DIAL conn --port 45001 --cgroup $SERVE_CG        # in-slice (serve's exact cgroup)
$DIAL conn --port 45002 --cgroup $NEG_CG          # outside the slice: negative control
python3 - <<PY
import subprocess,json
out=json.loads(subprocess.check_output(["bpftool","map","dump","pinned","$PIN/SEEN","-j"]))
seen={int.from_bytes(bytes(int(x,16) for x in e["key"]),"little"):int.from_bytes(bytes(int(x,16) for x in e["value"]),"little") for e in out}
for p in (45001,45002): print("SEEN[port %d] = %s"%(p,seen.get(p)))
PY
stamp "serve's own sockets: observe 45s of SEEN_COMM growth (tgid-agnostic, by comm)"
sleep 45
dump_seen
echo "(serve thread comms present above prove serve-created sockets fire the slice-attached hook)"
kill $LP; wait $LP 2>/dev/null
stamp "stageA end"
