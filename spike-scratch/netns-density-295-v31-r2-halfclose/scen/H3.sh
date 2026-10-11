#!/bin/bash
# H3: after the TCP-source end check (FIN discriminator): destination-reset hunt, clean regression (no false aborts), loopback kill matrix
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh loop hunt3 hunt3.json
export V31_TIMEOUT=900
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
C="--arm owner --flows 1024 --conc 512 --io-timeout 40 --events-sample 50 --intake-rcvbuf 4194304"
run() { L=$1; shift; bash hostrun.sh $L "$@" | grep -E "^===|^trials|^ guest|^ host|^   shutdown|^ live_at" ; python3 analyze_goals.py evidence/$L; }
run b3-mix            $C --trials 20000 --n mix --m mix --fin mix --srv-pause mix --cli-pause mix
run b3-finonly-p300   $C --trials 10000 --n 1048576 --m 1048576 --fin only --srv-pause 300 --cli-pause 300
run b3-findata-p3000  $C --trials 4000 --n 1048576 --m 1048576 --fin data --srv-pause 3000 --cli-pause 3000
run b3-small-findata  $C --trials 10000 --n 2000 --m 2000 --fin data
run b3-small-finonly  $C --trials 10000 --n 2000 --m 2000 --fin only
bash runplan2.sh loop loopkill3 loopkill3.json
