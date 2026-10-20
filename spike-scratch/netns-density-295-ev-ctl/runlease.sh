#!/bin/bash
# retry the canonical lease every ~2 minutes (flock -w 120 inside launch.py) until it is acquired and the job has run
cd "$(dirname "$0")"
name=$1; shift
log=out/runlease-$name.log
: > $log
while true; do
  python3 launch.py "$name" "$@" >> $log 2>&1
  rc=$?
  if [ $rc -ne 75 ]; then echo $rc > out/runlease-$name.rc; break; fi
  echo "[retry] lease busy, waiting $(date -u +%H:%M:%S)" >> $log
  sleep 60
done
