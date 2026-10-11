#!/bin/bash
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh loop crashfinraw crashfin_raw.json
V31_PERFTP=1 bash runplan2.sh loop crashfinperf crashfin_perf.json
