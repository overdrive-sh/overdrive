#!/bin/bash
cd "$(dirname "$0")"
while ! grep -q '^done' out/chain2.log 2>/dev/null; do sleep 20; done
./sync.sh > out/chain3a-sync.txt 2>&1
python3 launch.py build4 'bash /tmp/evmod-work/src/driver/build.sh' > out/build4.txt 2>&1
echo "build4 rc=$?" >> out/chain3a.log
# throughput of one flow per direction, module variants (percpu/unbound x 64KiB/256KiB peek)
for cfg in "percpu64:" "unbound64:unbound=1" "percpu256:buf_kb=256" "unbound256:unbound=1+buf_kb=256"; do
  name=${cfg%%:*}; mp=${cfg#*:}
  python3 launch.py tp-$name "python3 /tmp/evmod-work/src/driver/vmrun.py ev-tp-A,ev-tp-T,ev-tp-G modparams=$mp" > out/tp-$name.txt 2>&1
  echo "tp-$name rc=$?" >> out/chain3a.log
done
echo done >> out/chain3a.log
