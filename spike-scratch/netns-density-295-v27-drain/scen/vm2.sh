#!/bin/bash
cd /tmp/v27-work
ulimit -n 1048576
C="--diag-seq --io-timeout 20 --events-sample 500"
run() { L=$1; shift; timeout 900 python3 vmrun.py drain $L "$@" 2>&1 | grep -E "^===|^trials|^ guest|^ host|^   gap|^ live_at|^ EX|FAILED|Traceback|Error" ; }
for ARM in owner kernel; do
run vm-t6-tcp-rst-$ARM --arm $ARM --srv-rst-permille 500 --n 2097152 --m 1000 --srv-pause 100 --trials 3000 --conc 128 --flows 256 $C
run vm-t6-vsock-reset-$ARM --arm $ARM --chaos-v-reset-permille 300 --trials 4000 --conc 64 --flows 256 --srv-pause mix --cli-pause mix $C
done
for LATE in 0 1 2; do
run vm-t4-vlate$LATE-banner100000 --arm owner --v-late $LATE --srv-banner 100000 --trials 400 --conc 32 --flows 64 --n small --m small --io-timeout 8 --settle-s 4 --diag-seq --events-sample 50
done
run vm-t4-rcvlowat64k --arm owner --v-rcvlowat 65536 --trials 300 --conc 32 --flows 64 --n small --m small --io-timeout 5 --settle-s 3 --diag-seq --events-sample 50
run vm-t4-bigrcvbuf --arm owner --trials 8000 --conc 64 --flows 256 --intake-rcvbuf 4194304 $C
run vm-t4-findata --arm owner --trials 8000 --conc 64 --flows 256 --n small --m small --fin data $C
run vm-t4-finonly --arm owner --trials 8000 --conc 64 --flows 256 --n small --m small --fin only $C
