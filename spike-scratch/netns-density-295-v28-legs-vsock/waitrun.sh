#!/bin/bash
# retry the launcher while the lease queue wait is dropped (ssh idle timeout / lease wait timeout); never retries once the lease was acquired
cd "$(dirname "$0")"
name=$1; shift
for try in $(seq 1 30); do
  V28_LEASE_WAIT=${V28_LEASE_WAIT:-3000} python3 launch.py "$name" "$@" > out/$name.try$try.log 2>&1
  rc=$?
  if [ $rc -ne 75 ]; then cp out/$name.try$try.log out/$name.log; echo $rc > out/$name.sentinel; exit $rc; fi
  if grep -q "acquired after" out/$name.try$try.log; then cp out/$name.try$try.log out/$name.log; echo $rc > out/$name.sentinel; exit $rc; fi
  # drop orphaned waiters of this scenario before retrying
  sleep 5
done
echo 75 > out/$name.sentinel
