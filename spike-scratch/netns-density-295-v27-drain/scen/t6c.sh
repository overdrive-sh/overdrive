#!/bin/bash
cd /tmp/v27-work
export V27_TIMEOUT=600
C="--diag-seq --io-timeout 20 --events-sample 500"
bash hostrun.sh t6-clean-control2 --arm owner --trials 20000 --conc 256 --flows 512 $C
bash hostrun.sh t6-vsock-reset-owner  --arm owner  --chaos-v-reset-permille 300 --trials 6000 --conc 128 --flows 256 --srv-pause mix --cli-pause mix $C
bash hostrun.sh t6-vsock-reset-kernel --arm kernel --chaos-v-reset-permille 300 --trials 6000 --conc 128 --flows 256 --srv-pause mix --cli-pause mix $C
bash hostrun.sh t6-tcp-rst-owner  --arm owner  --srv-rst-permille 500 --n 2097152 --m 1000 --srv-pause 100 --trials 4000 --conc 256 --flows 512 $C
bash hostrun.sh t6-tcp-rst-kernel --arm kernel --srv-rst-permille 500 --n 2097152 --m 1000 --srv-pause 100 --trials 4000 --conc 256 --flows 512 $C
