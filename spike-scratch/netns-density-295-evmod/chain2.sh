#!/bin/bash
# runs after chain1: sync the lifecycle additions, rebuild, lifecycle scenarios, host-role V-3, then perf cohorts 2 and 3
cd "$(dirname "$0")"
while ! grep -q '^done' out/chain1.log 2>/dev/null; do sleep 20; done
./sync.sh > out/chain2-sync.txt 2>&1
python3 launch.py build3 'bash /tmp/evmod-work/src/driver/build.sh' > out/build3.txt 2>&1
echo "build3 rc=$?" >> out/chain2.log
python3 launch.py vm-life "python3 /tmp/evmod-work/src/driver/vmrun.py ev-cut-A,ev-cut-T,ev-cut-G,ev-sclose-A,ev-unload-T" > out/vm-life.txt 2>&1
echo "life rc=$?" >> out/chain2.log
V3="/probe v3 v2t s3000,f 256 10 v2t-stall 40; /probe v3 v2t s10000,f 512 2 v2t-long 60; /probe v3 v2t r20 64 3 v2t-slow 60; /probe v3 v2t s1000,f:300,s1000,f:300,s1000,f 128 5 v2t-cyc 60; /probe v3 t2v s3000,f 64 10 t2v-stall 40; /probe v3 t2v s10000,f 64 2 t2v-long 60; /probe v3 t2v s1000,f:300,s1000,f:300,s1000,f 64 5 t2v-cyc 60"
python3 launch.py v3 "cd /tmp/evmod-work && python3 src/bench/run.py --bench bench-v3 --max 1 --cells evmod-tcp:0 --tag v3 --command '$V3' --timeout 2400" > out/v3.txt 2>&1
echo "v3 rc=$?" >> out/chain2.log
python3 launch.py perf-c2 "cd /tmp/evmod-work && python3 src/bench/run.py --max 16384 --cells evmod-tcp:2,aya-tcp:2,kernel-tcp:2 --tag perf-c2 --no-build --timeout 3000" > out/perf-c2.txt 2>&1
echo "perf-c2 rc=$?" >> out/chain2.log
python3 launch.py perf-c3 "cd /tmp/evmod-work && python3 src/bench/run.py --max 16384 --cells kernel-tcp:3,aya-tcp:3,evmod-tcp:3 --tag perf-c3 --no-build --timeout 3000" > out/perf-c3.txt 2>&1
echo "perf-c3 rc=$?" >> out/chain2.log
echo done >> out/chain2.log
