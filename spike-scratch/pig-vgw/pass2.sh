#!/usr/bin/env bash
# Second pass inside the held session: redeploy the (stopped) VM Service, then richer V-GW-1 evidence.
set -u
. "$(dirname "$0")/common.sh"
export OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08/config
BIN=/home/ubuntu/overdrive/target/debug/overdrive
stamp "serve alive?"; pgrep -af 'overdrive serve' | cut -c1-80
stamp "V-GW-2: SEEN_COMM after ~3 min of serve activity (comm -> connects seen by slice-attached vgw_cookie)"
python3 $V/seen.py SEEN_COMM
stamp "redeploy VM Service (detach)"
timeout 60 $BIN deploy --detach /home/ubuntu/overdrive/examples/service-kind-vm-workloads/service.toml 2>&1 | head -5
for i in $(seq 1 90); do
  H=$(bpftool map dump pinned $PIN/BACKEND_MAP 2>/dev/null | grep -c "01 00  01 00 00 00")
  [ "$H" -ge 1 ] && break; sleep 1
done
stamp "BACKEND_MAP after redeploy (healthy entries=$H after ${i}s)"; dumpmap BACKEND_MAP
$BIN workload describe service-vm-e08 2>&1 | head -14
stamp "inbound_destinations set"
nft list set ip overdrive-mtls inbound_destinations 2>&1 | head -12
BE=$(bpftool map dump pinned $PIN/BACKEND_MAP -j | python3 -c '
import json,sys
for e in json.load(sys.stdin):
    v=bytes(int(x,16) for x in e["value"]); 
    if v[8]==1 and int.from_bytes(v[4:6],"little")==18080: print(".".join(str(b) for b in v[0:4][::-1]))' | head -1)
stamp "backend now $BE"
stamp "V-GW-1 prepared dial + TLS (leg-C expected), then ss -tanpe snapshot while held"
$DIAL dial --vip 10.96.0.2 --port 18080 --tls --hold 5 --cgroup $SERVE_CG > $OUT/dial-tls2.log 2>&1 &
DP=$!
sleep 1.5
ss -tanpe | grep -E "100\.95|36[0-9]{3}|35[0-9]{3}" | grep -v 7644 | head -8
sleep 1
wait $DP; cat $OUT/dial-tls2.log
stamp "V-GW-1 control (a): NO SO_MARK, NO bind, registered, TLS attempt: who answers?"
$DIAL dial --vip 10.96.0.2 --port 18080 --no-mark --no-bind --tls --to 2000 --cgroup $SERVE_CG
stamp "V-GW-1 control (a2): same but target is the backend directly with no hook registration (--no-register): plain connect"
$DIAL dial --vip 10.96.0.2 --port 18080 --no-register --to 2000 --cgroup $SERVE_CG
stamp "serve log lines for the leg-C refusals of the prepared dial"
grep -a "peer sent no certificates" /srv/vm/overdrive-testing/svm-e08/serve.log | tail -4 | cut -c1-260
stamp "V-GW-3: stop again, observe Selected -> refused with poller"
nohup $DIAL poll --secs 25 --cgroup $SERVE_CG > $OUT/poll2.log 2>&1 &
sleep 3
$BIN job stop service-vm-e08 2>&1 | head -2
sleep 14
cat $OUT/poll2.log | cut -c1-200
dumpmap BACKEND_MAP
stamp "pass2 end"
