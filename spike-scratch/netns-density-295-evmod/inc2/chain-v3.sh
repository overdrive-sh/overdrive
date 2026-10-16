#!/bin/bash
# item 1: host-role V-3 (vsock source on the vhost transport) with evmod
cd "$(dirname "$0")"
V3="/probe v3 v2t s3000,f 256 10 v2t-stall 40; /probe v3 v2t s10000,f 512 2 v2t-long 60; /probe v3 v2t r20 64 3 v2t-slow 60; /probe v3 v2t s1000,f:300,s1000,f:300,s1000,f 128 5 v2t-cyc 60; /probe v3 t2v s3000,f 64 10 t2v-stall 40; /probe v3 t2v s10000,f 64 2 t2v-long 60; /probe v3 t2v s1000,f:300,s1000,f:300,s1000,f 64 5 t2v-cyc 60"
python3 launch.py v3 "cd /var/tmp/evmod2-work && python3 src/bench/run.py --bench bench-v3 --max 1 --cells evmod-tcp:0 --tag v3 --command '$V3' --timeout 2400" > out/v3.txt 2>&1
echo "v3 rc=$?" >> out/chain.log
