#!/bin/bash
# E: flows larger than 2^32 bytes per direction: the 32-bit kernel quantities (tcp_sock.write_seq, vvs->tx_cnt) wrap
cd /tmp/v31-work
export V31_TIMEOUT=900
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
sha256sum $V31_BIN $V31_OBJ
C="--arm owner --flows 64 --io-timeout 300 --settle-s 15 --events-sample 1"
bash hostrun.sh e-wrap-5GiB-x4 $C --trials 4 --conc 4 --n 5368709120 --m 5368709120 --fin mix --srv-pause 0 --cli-pause 0 | grep -E "^===|^trials|^ guest|^ host|^   exact|^ live_at|^   gap"
bash hostrun.sh e-wrap-5GiB-pause $C --trials 2 --conc 2 --n 5368709120 --m 5368709120 --fin data --srv-pause 3000 --cli-pause 3000 | grep -E "^===|^trials|^ guest|^ host|^   exact|^ live_at|^   gap"
