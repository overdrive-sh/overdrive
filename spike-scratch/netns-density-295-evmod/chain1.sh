#!/bin/bash
cd "$(dirname "$0")"
for d in A T G; do
  names=$(for k in stall long slow cyc bidir eof tiny fault1 fault2 fault3 tp burst; do printf "ev-$k-$d,"; done | sed 's/,$//')
  python3 launch.py vm-$d "python3 /tmp/evmod-work/src/driver/vmrun.py $names" > out/vm-$d.txt 2>&1
  echo "$d rc=$?" >> out/chain1.log
done
python3 launch.py vm-ctl "python3 /tmp/evmod-work/src/driver/vmrun.py ctl-A,ctl-T,tp-sockmap-A,tp-pump-A,tp-sockmap-T,tp-pump-T" > out/vm-ctl.txt 2>&1
echo "ctl rc=$?" >> out/chain1.log
echo done >> out/chain1.log
