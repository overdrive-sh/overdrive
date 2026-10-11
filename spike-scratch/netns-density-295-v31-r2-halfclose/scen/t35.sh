#!/bin/bash
cd /tmp/v31-work
ulimit -n 1048576
modprobe vsock_loopback
mkdir -p evidence
echo "=== T3 loopback 30000 trials"
./out/drainprobe t3 --mode loop --trials 30000 --conc 64 --port 7950 | tee evidence/t3-loop.json
echo "=== T5 loopback 500 trials per cell (6 cells)"
(./out/drainprobe t5 --mode obs --trials 500 --obj out/drain-bpf.o --port 7960 --wait-s 900 > evidence/t5-loop.json 2>&1 &)
sleep 3
./out/drainprobe t5 --mode runner --cid 1 --port 7960 --trials 500 --pace-ms 15
sleep 3
cat evidence/t5-loop.json
echo "=== T3 + T5 in the VM"
timeout 1200 python3 vmrun.py t3 6000 2>&1 | grep -E "^RESULT_JSON|Traceback|FAILED" | cut -c1-6000
timeout 1500 python3 vmrun.py t5 110 2>&1 | grep -E "^RESULT_JSON|Traceback|FAILED" | cut -c1-9000
