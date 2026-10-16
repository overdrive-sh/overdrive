#!/bin/bash
# final verification pass on the FINAL module build; waits for the metal host to be reachable and retries on connectivity loss
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
reach() { ssh -o BatchMode=yes -o ConnectTimeout=10 $OVERDRIVE_METAL_TARGET true 2>/dev/null; }
run() {  # name, remote command
  local name=$1; shift
  for attempt in 1 2 3 4; do
    until reach; do sleep 60; done
    python3 launch.py "$name" "$@" > out/$name.txt 2>&1
    rc=$?
    echo "$name attempt=$attempt rc=$rc $(date -u +%H:%M:%S)" >> out/chain4.log
    case $rc in 255|75) sleep 30; continue;; *) return $rc;; esac
  done
}
until reach; do sleep 60; done
./sync.sh > out/chain4-sync.txt 2>&1
run build5 'bash /tmp/evmod-work/src/driver/build.sh'
V3="/probe v3 v2t s3000,f 256 10 v2t-stall 40; /probe v3 v2t s10000,f 512 2 v2t-long 60; /probe v3 v2t r20 64 3 v2t-slow 60; /probe v3 v2t s1000,f:300,s1000,f:300,s1000,f 128 5 v2t-cyc 60; /probe v3 t2v s3000,f 64 10 t2v-stall 40; /probe v3 t2v s10000,f 64 2 t2v-long 60; /probe v3 t2v s1000,f:300,s1000,f:300,s1000,f 64 5 t2v-cyc 60"
run v3 "cd /tmp/evmod-work && python3 src/bench/run.py --bench bench-v3 --max 1 --cells evmod-tcp:0 --tag v3 --command '$V3' --timeout 2400"
run life2 "python3 /tmp/evmod-work/src/driver/vmrun.py ev-cut-A,ev-cut-T,ev-cut-G,ev-sclose-A,ev-tight-A,ev-tight-T"
run unload2 "python3 /tmp/evmod-work/src/driver/vmrun.py ev-unload-T"
for d in A T G; do
  names=$(for k in stall cyc bidir eof tiny fault1 fault2 fault3; do printf "ev-$k-$d,"; done | sed 's/,$//')
  run core2-$d "python3 /tmp/evmod-work/src/driver/vmrun.py $names"
done
for cfg in "percpu64:" "unbound64:unbound=1" "percpu256:buf_kb=256" "unbound256:unbound=1+buf_kb=256"; do
  name=${cfg%%:*}; mp=${cfg#*:}
  run tp2-$name "python3 /tmp/evmod-work/src/driver/vmrun.py ev-tp-A,ev-tp-T,ev-tp-G modparams=$mp"
done
run perf-final "cd /tmp/evmod-work && python3 src/bench/run.py --max 16384 --cells evmod-tcp:6,aya-tcp:6,evmod-tcp:7 --tag perf-final --timeout 3000"
run perf-unbound "cd /tmp/evmod-work && python3 src/bench/run.py --max 16384 --cells evmod-tcp:5 --evmod-params unbound=1 --tag perf-unbound --no-build --timeout 3000"
echo done >> out/chain4.log
