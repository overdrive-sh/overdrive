#!/bin/bash
# T1: the drain gap exists on 7.0 (loopback vsock, TCP<->vsock<->TCP, both roles on the host)
cd /tmp/v27-work
C="--trials 10000 --conc 512 --flows 1024 --io-timeout 40 --diag-seq --events-sample 1000"
export V27_TIMEOUT=600
for P in 0 300 3000; do
  bash hostrun.sh t1-drain-p$P --arm owner --n 1048576 --m 1048576 --srv-pause $P --cli-pause $P $C
  bash hostrun.sh t1-naive-p$P --arm none  --n 1048576 --m 1048576 --srv-pause $P --cli-pause $P $C
done
# not paused, small FIN-carrying and FIN-only final segments
bash hostrun.sh t1-drain-small-findata --arm owner --n 2000 --m 2000 --fin data $C
bash hostrun.sh t1-drain-small-finonly --arm owner --n 2000 --m 2000 --fin only $C
bash hostrun.sh t1-naive-small-findata --arm none  --n 2000 --m 2000 --fin data $C
bash hostrun.sh t1-naive-small-finonly --arm none  --n 2000 --m 2000 --fin only $C
