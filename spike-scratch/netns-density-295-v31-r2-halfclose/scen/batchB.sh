#!/bin/bash
# B: clean half-close volume (test 1), loopback: owners in one process, vsock over vsock_loopback, no control-session half-close
cd /tmp/v31-work
export V31_TIMEOUT=900
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
sha256sum $V31_BIN $V31_OBJ
C="--arm owner --flows 1024 --conc 512 --io-timeout 40 --events-sample 50"
run() { L=$1; shift; bash hostrun.sh $L "$@" | grep -E "^===|^trials|^ guest|^ host|^   shutdown|^   gap|^   backstop|^   exact|^ live_at|^      fexit" ; python3 analyze_goals.py evidence/$L; }
for P in 0 300 3000; do
  run b-findata-p$P  $C --trials 10000 --n 1048576 --m 1048576 --fin data --srv-pause $P --cli-pause $P
  run b-finonly-p$P  $C --trials 10000 --n 1048576 --m 1048576 --fin only --srv-pause $P --cli-pause $P
done
run b-small-findata  $C --trials 10000 --n 2000 --m 2000 --fin data
run b-small-finonly  $C --trials 10000 --n 2000 --m 2000 --fin only
# one direction only
run b-reqonly-data   $C --trials 5000 --n 1048576 --m 0 --fin data --srv-pause 300
run b-reqonly-only   $C --trials 5000 --n 1048576 --m 0 --fin only --srv-pause 300
run b-reponly-data   $C --trials 5000 --n 0 --m 1048576 --fin data --cli-pause 300
run b-reponly-only   $C --trials 5000 --n 0 --m 1048576 --fin only --cli-pause 300
run b-mix            $C --trials 20000 --n mix --m mix --fin mix --srv-pause mix --cli-pause mix
