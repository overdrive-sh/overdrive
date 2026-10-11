#!/bin/bash
cd /tmp/v27-work
sha256sum out/drainprobe out/drain-bpf.o
export V27_TIMEOUT=600
C="--diag-seq --io-timeout 20 --events-sample 500"
for S in 1 2 3 4 5; do
for ARM in owner kernel; do
bash hostrun.sh t6-tcp-rst-$ARM-s$S --arm $ARM --seed $S --srv-rst-permille 500 --n 2097152 --m 1000 --srv-pause 100 --trials 4000 --conc 256 --flows 512 $C
bash hostrun.sh t6-vsock-reset-$ARM-s$S --arm $ARM --seed $S --chaos-v-reset-permille 300 --trials 6000 --conc 128 --flows 256 --srv-pause mix --cli-pause mix $C
done; done
bash hostrun.sh t6-clean-control --arm owner --trials 20000 --conc 256 --flows 512 $C
