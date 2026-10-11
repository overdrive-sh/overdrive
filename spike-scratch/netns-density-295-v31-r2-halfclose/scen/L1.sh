#!/bin/bash
# L1: guard smoke, then the loopback split-process kill/crash/control-loss matrix
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh loop smokeA smokeA.json > /tmp/v31-work/smokeA.out 2>&1
cat /tmp/v31-work/smokeA.out
if grep -qE "Connection refused|Traceback|ERROR|EXCEPTION" /tmp/v31-work/smokeA.out; then echo "GUARD: smoke failed, not running the matrix"; exit 3; fi
bash runplan2.sh loop loopkill loopkill.json
