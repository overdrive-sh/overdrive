#!/bin/bash
# runs on the metal host as root: hostrun.sh <label> <loop args...>  -> evidence/<label>.json + compact accounting
L=$1; shift
cd /tmp/v27-work; mkdir -p evidence
ulimit -n 1048576
T0=$(date +%s)
nstat -az > evidence/$L.nstat0 2>/dev/null
timeout ${V27_TIMEOUT:-1500} ./out/drainprobe loop --obj out/drain-bpf.o --events /tmp/v27-work/evidence/$L "$@" > evidence/$L.json 2> evidence/$L.err
RC=$?
nstat -az > evidence/$L.nstat1 2>/dev/null
python3 /tmp/v27-work/acct.py "$L" "$RC" "$*"
