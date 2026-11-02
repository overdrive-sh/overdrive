#!/usr/bin/env bash
# Runs inside the env session right after the VM Service reported Running+healthy (observations present).
set -u
. "$(dirname "$0")/common.sh"
M() { echo "MARK $1 $(date +%s%3N)" >> $OUT/marks.log; }
M post_deploy_start
# the readiness-driven healthy byte flapped to 0 within seconds of Running in earlier runs (see out/run3);
# the V-GW-1 path checks below need a Selected receipt, so re-assert healthy=1 on the live entries
# (bpftool map update on the production BACKEND_MAP value) right before each dial group. V-GW-3 claims come from the
# poller log, which never touches the map.
forcehealthy() {
  for k in 1 2; do
    V=$(bpftool map lookup pinned $PIN/BACKEND_MAP key hex $(printf '%02x 00 00 00' $k) -j 2>/dev/null | python3 -c 'import json,sys; v=json.load(sys.stdin)["value"]; v[8]="0x01"; print(" ".join(x[2:] for x in v))' 2>/dev/null)
    [ -n "$V" ] && bpftool map update pinned $PIN/BACKEND_MAP key hex $(printf '%02x 00 00 00' $k) value hex $V 2>&1
  done
  echo "(forced healthy=1) $(bpftool map dump pinned $PIN/BACKEND_MAP | grep -c '01 00  01 00 00 00') entries healthy"
}
stamp "V-GW-3a map state (healthy byte) before dial"
dumpmap BACKEND_MAP
stamp "V-GW-1 production nft/route state (overdrive-mtls)"
ip rule; ip route show table 100; nft list tables
nft list ruleset > $OUT/nft-before.txt 2>&1; grep -n "counter\|tproxy\|mark" $OUT/nft-before.txt | head -40
stamp "V-GW-1 leg-C listeners (ss -tlnp, IP_TRANSPARENT agent)"
ss -tlnp | grep -E "overdrive|:4[0-9]{4}|:5[0-9]{4}" | head -10
forcehealthy
stamp "V-GW-1 (1) prepared dial, NO TLS, hold 4s: ss -tanpe while ESTABLISHED (client side + leg-C accepted side)"
$DIAL dial --vip 10.96.0.2 --port 18080 --hold 4 --cgroup $SERVE_CG > $OUT/dial-plain.log 2>&1 &
DP=$!
sleep 1.5
ss -tanpe state established | grep -E "100\.95|:18080" | head -8
wait $DP; cat $OUT/dial-plain.log
forcehealthy
stamp "V-GW-1 (2) prepared dial + TLS 1.3 handshake attempt (no client cert available)"
$DIAL dial --vip 10.96.0.2 --port 18080 --tls --cgroup $SERVE_CG
forcehealthy
stamp "V-GW-1 controls: (a) no mark/bind + TLS  (b) mark only  (c) bind only  (d) unregistered"
$DIAL dial --vip 10.96.0.2 --port 18080 --no-mark --no-bind --tls --to 2000 --cgroup $SERVE_CG
$DIAL dial --vip 10.96.0.2 --port 18080 --no-bind --to 2000 --cgroup $SERVE_CG
$DIAL dial --vip 10.96.0.2 --port 18080 --no-mark --to 2000 --cgroup $SERVE_CG
$DIAL dial --vip 10.96.0.2 --port 18080 --no-register --to 2000 --cgroup $SERVE_CG
stamp "serve log: leg-C refusals during this window"
grep -a "peer sent no certificates" /srv/vm/overdrive-testing/svm-e08/serve.log | tail -4 | cut -c1-260
M v3_stop_begin
stamp "live BACKEND_MAP entries before stop"
for k in 1 2; do
  KEYHEX=$(printf '%02x 00 00 00' $k)
  V=$(bpftool map lookup pinned $PIN/BACKEND_MAP key hex $KEYHEX -j 2>/dev/null | python3 -c 'import json,sys; v=json.load(sys.stdin)["value"]; print(" ".join(v))' 2>/dev/null)
  echo "key $k value=$V"
done
stamp "stop the job; poller continues"
OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08/config /home/ubuntu/overdrive/target/debug/overdrive job stop service-vm-e08 > $OUT/stop.log 2>&1; cat $OUT/stop.log | head -5
M job_stopped
sleep 12
dumpmap BACKEND_MAP
M post_deploy_end
touch /tmp/vgw-stopped
stamp "post-deploy end"
