#!/bin/bash
# C: control-session loss injected at seeded random points (test 3), loopback, in-process control channel
cd /tmp/v31-work
export V31_TIMEOUT=300 BRIEF=1
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
sha256sum $V31_BIN $V31_OBJ
C="--arm owner --flows 256 --conc 128 --trials 2500 --n mix --m mix --srv-pause mix --cli-pause mix --fin mix --fault-judge --io-timeout 15 --settle-s 6 --events-sample 1000000"
bash hostrun.sh c-noloss-control $C --seed 900
for S in $(seq 1 40); do
  bash hostrun.sh c-loss-$S $C --seed $S --ctl-loss-ms-min 5 --ctl-loss-ms-max 1500
done
# the ADR's abort-everything policy for contrast
for S in $(seq 101 106); do
  bash hostrun.sh c-lossall-$S $C --seed $S --ctl-loss-ms-min 5 --ctl-loss-ms-max 1500 --ctl-loss-policy all
done
