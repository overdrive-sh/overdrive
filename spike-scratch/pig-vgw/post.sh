#!/usr/bin/env bash
. "$(dirname "$0")/common.sh"
stamp "post: SEEN_COMM (comm -> connects counted by the slice-attached hook)"
python3 $V/seen.py SEEN_COMM 2>&1
for f in poll cookie select; do [ -f $OUT/pid-$f ] && kill $(cat $OUT/pid-$f) 2>/dev/null; done
sleep 1
rmdir /sys/fs/cgroup/pigvgw-sel /sys/fs/cgroup/pigvgw-neg 2>&1
rm -rf $PIN
