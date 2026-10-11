#!/bin/bash
cd /tmp/v27-work
export V27_TIMEOUT=600
C="--diag-seq --io-timeout 20 --events-sample 500"
# T4(d): server banner queued on the flow vsock BEFORE the guest installs it (install after Paired)
for B in 100000 5000; do
 for LATE in 0 1 2; do
  bash hostrun.sh t4-vlate$LATE-banner$B --arm owner --v-late $LATE --srv-banner $B --trials 400 --conc 32 --flows 64 --n small --m small --io-timeout 8 --settle-s 4 --diag-seq --events-sample 50
 done
done
# T6 again with the unannounced-vsock-end rule
bash hostrun.sh t6-tcp-rst-owner  --arm owner  --srv-rst-permille 500 --n 2097152 --m 1000 --srv-pause 100 --trials 4000 --conc 256 --flows 512 $C
bash hostrun.sh t6-tcp-rst-kernel --arm kernel --srv-rst-permille 500 --n 2097152 --m 1000 --srv-pause 100 --trials 4000 --conc 256 --flows 512 $C
bash hostrun.sh t6-vsock-reset-owner  --arm owner  --chaos-v-reset-permille 300 --trials 6000 --conc 128 --flows 256 --srv-pause mix --cli-pause mix $C
bash hostrun.sh t6-vsock-reset-kernel --arm kernel --chaos-v-reset-permille 300 --trials 6000 --conc 128 --flows 256 --srv-pause mix --cli-pause mix $C
# no false aborts on clean runs with the new rule
bash hostrun.sh t6-clean-control --arm owner --trials 20000 --conc 256 --flows 512 $C
