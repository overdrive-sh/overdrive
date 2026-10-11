#!/bin/bash
cd /tmp/v27-work
sha256sum out/drainprobe out/drain-bpf.o
C="--trials 10000 --conc 512 --flows 1024 --io-timeout 40 --diag-seq --events-sample 1000"
export V27_TIMEOUT=600
for P in 0 300 3000; do
  bash hostrun.sh t1-naive-p$P --arm none  --n 1048576 --m 1048576 --srv-pause $P --cli-pause $P $C > /dev/null
done
bash hostrun.sh t1-naive-small-findata --arm none  --n 2000 --m 2000 --fin data $C > /dev/null
bash hostrun.sh t1-naive-small-finonly --arm none  --n 2000 --m 2000 --fin only $C > /dev/null
