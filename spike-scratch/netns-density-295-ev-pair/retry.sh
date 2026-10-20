#!/bin/bash
# Poll for the lease every 60 s (the holder waits 55 s then exits 75); any other exit code is the job's result.
cd "$(dirname "$0")"
while true; do
  python3 launch.py "$@"; rc=$?
  [ $rc -ne 75 ] && exit $rc
  echo "[retry] lease busy, polling again in 60 s"; sleep 5
done
