#!/bin/bash
# runplan2.sh <env> <label> <plan.json>
cd /tmp/v31-work
ulimit -n 1048576
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
sha256sum $V31_BIN $V31_OBJ
python3 vmrun.py "$@" 2>&1 | grep --line-buffered -vE "^\[[0-9:]+\]"
