#!/bin/bash
# item 2b: core battery on the final module build (source sha256 d5710d7c...), A, T, G, one launch each
cd "$(dirname "$0")"
for d in A T G; do
  names=$(for k in stall long slow cyc burst bidir eof tiny fault1 fault2 fault3; do printf "ev-$k-$d,"; done | sed 's/,$//')
  python3 launch.py core-$d "python3 /var/tmp/evmod2-work/src/driver/vmrun.py $names" > out/core-$d.txt 2>&1
  echo "core-$d rc=$? $(date -u +%H:%M:%S)" >> out/chain.log
done
