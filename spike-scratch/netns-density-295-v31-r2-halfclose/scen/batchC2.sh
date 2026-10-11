#!/bin/bash
# C2: control-session loss at seeded random points, corrected judge, intake SO_RCVBUF = 4 MiB (the v27 amendment)
cd /tmp/v31-work
ulimit -n 1048576
export V31_TIMEOUT=300 BRIEF=1
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
sha256sum $V31_BIN $V31_OBJ
C="--arm owner --flows 256 --conc 128 --trials 2500 --n mix --m mix --srv-pause mix --cli-pause mix --fin mix --fault-judge --io-timeout 15 --settle-s 6 --events-sample 1000000 --intake-rcvbuf 4194304"
bash hostrun.sh c2-noloss-control $C --seed 900
for S in $(seq 201 240); do
  bash hostrun.sh c2-loss-$S $C --seed $S --ctl-loss-ms-min 5 --ctl-loss-ms-max 1500
done
for S in $(seq 301 306); do
  bash hostrun.sh c2-lossall-$S $C --seed $S --ctl-loss-ms-min 5 --ctl-loss-ms-max 1500 --ctl-loss-policy all
done
