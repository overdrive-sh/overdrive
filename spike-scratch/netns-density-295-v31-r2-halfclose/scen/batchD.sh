#!/bin/bash
# D: real recursion misses at volume (test 4): cumulative quantity + backstop vs the v27 single per-call counter
cd /tmp/v31-work
export V31_TIMEOUT=300 BRIEF=1
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
sha256sum $V31_BIN $V31_OBJ
C="--arm owner --flows 512 --conc 400 --n 2097152 --m 2097152 --srv-pause 0 --cli-pause 0 --io-timeout 15 --settle-s 5 --events-sample 1000000 --trials 4000"
for S in $(seq 11 34); do bash hostrun.sh d-full-$S $C --seed $S --backstop full; done
for S in $(seq 41 52); do bash hostrun.sh d-off-$S  $C --seed $S --backstop off; done
# v27 "m2" analog: ONE per-call counter (program B always skips), no backstop
for S in $(seq 61 72); do bash hostrun.sh d-incr1-$S $C --seed $S --incr --skip-b 1000 --backstop off; done
