#!/usr/bin/env bash
set -u
. "$(dirname "$0")/common.sh"
export OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08/config
BIN=/home/ubuntu/overdrive/target/debug/overdrive
stamp "serve alive?"; pgrep -af 'overdrive serve' | cut -c1-80
stamp "wait for BACKEND_MAP to carry the NEW backend (100.95.0.3 = 03 00 5f 64) healthy"
for i in $(seq 1 120); do
  bpftool map dump pinned $PIN/BACKEND_MAP | grep -q "03 00 5f 64 a0 46 01 00  01 00 00 00" && break; sleep 1
done
echo "waited ${i}s"
sleep 1
stamp "BACKEND_MAP now"; dumpmap BACKEND_MAP
stamp "V-GW-1 prepared dial + TLS, ss -tanpe during"
$DIAL dial --vip 10.96.0.2 --port 18080 --tls --hold 5 --cgroup $SERVE_CG > $OUT/dial-tls3.log 2>&1 &
DP=$!
sleep 1.2
ss -tanpe | grep -E "100\.95" | head -8
wait $DP; cat $OUT/dial-tls3.log
stamp "control (a) no mark/bind + TLS"
$DIAL dial --vip 10.96.0.2 --port 18080 --no-mark --no-bind --tls --hold 1 --to 2000 --cgroup $SERVE_CG
stamp "control (b) mark only"
$DIAL dial --vip 10.96.0.2 --port 18080 --no-bind --to 2000 --cgroup $SERVE_CG
stamp "control (c) bind only"
$DIAL dial --vip 10.96.0.2 --port 18080 --no-mark --to 2000 --cgroup $SERVE_CG
stamp "control (d) unregistered socket to VIP (hook passthrough; VIP is a local veth addr)"
$DIAL dial --vip 10.96.0.2 --port 18080 --no-register --to 2000 --cgroup $SERVE_CG
stamp "serve log: leg-C refusals"
grep -a "peer sent no certificates" /srv/vm/overdrive-testing/svm-e08/serve.log | tail -3 | cut -c1-260
stamp "pass3 end"
