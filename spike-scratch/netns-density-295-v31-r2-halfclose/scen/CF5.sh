#!/bin/bash
cd /tmp/v31-work
ulimit -n 1048576
rm -f stop-stress; python3 stress_bpfsys.py /tmp/v31-work/stop-stress > evidence/stress.out 2>&1 &
SP=$!
sleep 2
bash runplan2.sh loop stressraw crashfin_stress_raw.json
V31_PERFTP=1 bash runplan2.sh loop stressperf crashfin_stress_perf.json
touch stop-stress; wait $SP; cat evidence/stress.out
