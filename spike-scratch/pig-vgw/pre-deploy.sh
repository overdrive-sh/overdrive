#!/usr/bin/env bash
# Runs inside the env session after `serve` is ready and BEFORE the VM Service is deployed.
set -u
. "$(dirname "$0")/common.sh"
M() { echo "MARK $1 $(date +%s%3N)" >> $OUT/marks.log; }
stamp "pre-deploy start"
SP=$(pgrep -f 'overdrive serve' | head -1)
rm -rf $PIN; mkdir -p $PIN
SM=$(bpftool map show | awk '/name SERVICE_MAP/{sub(":","",$1);print $1}'); BM=$(bpftool map show | awk '/name BACKEND_MAP/{sub(":","",$1);print $1}')
echo "SERVICE_MAP id=$SM BACKEND_MAP id=$BM"
bpftool map pin id "$SM" $PIN/SERVICE_MAP; bpftool map pin id "$BM" $PIN/BACKEND_MAP
mkdir -p $NEG_CG /sys/fs/cgroup/pigvgw-sel
stamp "V-GW-2 cgroup membership of serve ($SP); production attach path /sys/fs/cgroup/overdrive.slice"
cat /proc/$SP/cgroup
echo "threads by (cgroup, comm):"
for t in /proc/$SP/task/*; do echo "$(cat $t/cgroup) $(cat $t/comm)"; done | sort | uniq -c
stamp "production programs attached under the slice"
bpftool cgroup tree /sys/fs/cgroup/overdrive.slice
stamp "production program stats (aya ProgramInfo)"
$VGW info
bpftool prog show -j | python3 -c '
import json,sys
for p in json.load(sys.stdin):
    if p.get("name","").startswith("cgroup_"): print(p.get("id"),p.get("name"),p.get("type"),"verified_insns=",p.get("verified_insns"),"xlated=",p.get("bytes_xlated"))'
stamp "load+attach vgw_cookie at overdrive.slice (alongside production hook, multi)"
nohup $VGW $ELF vgw_cookie /sys/fs/cgroup/overdrive.slice $PIN > $OUT/loader-cookie.log 2>&1 &
echo $! > $OUT/pid-cookie
sleep 2; cat $OUT/loader-cookie.log
stamp "load+attach vgw_select (V-GW-4: chained lookup) at overdrive.slice, multi"
nohup $VGW $ELF vgw_select /sys/fs/cgroup/overdrive.slice $PIN > $OUT/loader-select.log 2>&1 &
echo $! > $OUT/pid-select
sleep 2; cat $OUT/loader-select.log
bpftool cgroup tree /sys/fs/cgroup/overdrive.slice
$VGW info
stamp "V-GW-5 cookie equality, plain sockets, inside serve's cgroup"
$DIAL cookie --n 10 --cgroup $SERVE_CG
stamp "V-GW-5 cookie equality, prepared sockets (SO_MARK=1 + bind lo + NO_PORT), serve's cgroup"
$DIAL cookie --n 10 --mark-bind --cgroup $SERVE_CG
stamp "V-GW-2 controls: connect from serve's exact cgroup vs outside the slice"
$DIAL conn --port 45001 --cgroup $SERVE_CG
$DIAL conn --port 45002 --cgroup $NEG_CG
python3 - <<PY
import subprocess,json
out=json.loads(subprocess.check_output(["bpftool","map","dump","pinned","$PIN/SEEN","-j"]))
seen={int.from_bytes(bytes(int(x,16) for x in e["key"]),"little"):int.from_bytes(bytes(int(x,16) for x in e["value"]),"little") for e in out}
for p in (45001,45002): print("SEEN[port %d] = %s   (hook counted connects)"%(p,seen.get(p)))
PY
M pre_deploy_done
nohup $DIAL poll --secs 170 --cgroup $SERVE_CG > $OUT/poll.log 2>&1 &
echo $! > $OUT/pid-poll
stamp "poller started; pre-deploy end"
