#!/bin/bash
# H: hunt (1) the clean EOF a client saw after a destination RST, (2) client/server payload corruption under control-session loss.
cd /tmp/v31-work
ulimit -n 1048576
export V31_TIMEOUT=600
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
sha256sum $V31_BIN $V31_OBJ
C="--arm owner --flows 256 --conc 128 --fault-judge --io-timeout 15 --settle-s 6 --diag-seq --events-sample 1"
BRIEF=1 bash hostrun.sh h-destrst-1 $C --trials 20000 --conc 256 --n 2097152 --m 1000 --srv-pause 100 --fin mix --srv-rst-permille 500 --seed 5
for S in 1 2 3 4 5 6 7 8; do
  BRIEF=1 bash hostrun.sh h-loss-$S $C --trials 2500 --n mix --m mix --srv-pause mix --cli-pause mix --fin mix --seed $S --ctl-loss-ms-min 5 --ctl-loss-ms-max 1500
done
