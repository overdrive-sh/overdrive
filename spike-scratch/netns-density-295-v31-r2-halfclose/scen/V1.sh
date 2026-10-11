#!/bin/bash
# V1: microVM guard smoke, then the VM kill/crash/control-loss matrix
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh vm vmsmoke vmsmoke.json > /tmp/v31-work/vmsmoke.out 2>&1
cat /tmp/v31-work/vmsmoke.out
if grep -qE "Traceback|ERROR|EXCEPTION|FAILED|not ready" /tmp/v31-work/vmsmoke.out; then echo "GUARD: vm smoke failed, not running the matrix"; exit 3; fi
bash runplan2.sh vm vmkill vmkill.json
