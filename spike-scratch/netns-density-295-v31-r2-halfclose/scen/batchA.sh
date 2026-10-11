#!/bin/bash
# A: backstop / cumulative-counter ablation under injected recursion misses (loopback, both roles in one process)
cd /tmp/v31-work
export V31_TIMEOUT=300
SNAP=/tmp/v31-work/run/$(date +%s)-$$; mkdir -p $SNAP; cp out/drainprobe out/drain-bpf.o $SNAP/; export V31_BIN=$SNAP/drainprobe V31_OBJ=$SNAP/drain-bpf.o
sha256sum $V31_BIN $V31_OBJ
C="--arm owner --flows 512 --conc 256 --n 262144 --m 262144 --io-timeout 8 --settle-s 3 --events-sample 100000"
run() { L=$1; shift; bash hostrun.sh $L "$@" | grep -E "^===|^trials|^ guest|^ host|^   backstop|^ live_at|^      fexit|^   shutdown|VIOLATIONS" ; }
# control: v27 per-call increment counters (max of two), both programs skipping 30% of calls
run a1-incr-skip30-off      $C --trials 3000 --incr --skip-a 300 --skip-b 300 --backstop off
# cumulative quantity, same injection, no backstop
run a2-cum-skip30-off       $C --trials 3000 --skip-a 300 --skip-b 300 --backstop off
# only one program skipping 70%: the other still sees every call
run a5-cum-skipA70-off      $C --trials 3000 --skip-a 700 --skip-b 0 --backstop off
run a5b-incr-skipA70-off    $C --trials 3000 --incr --skip-a 700 --skip-b 0 --backstop off
# backstop on the rise only
run a3-cum-skip30-rise      $C --trials 3000 --skip-a 300 --skip-b 300 --backstop rise
run a8-cum-skip30-rise-c1   $C --trials 600 --conc 1 --skip-a 300 --skip-b 300 --backstop rise
# rise + arm-time level read
run a4-cum-skip30-full      $C --trials 3000 --skip-a 300 --skip-b 300 --backstop full
run a6-cum-skip70-full      $C --trials 3000 --skip-a 700 --skip-b 700 --backstop full
run a7-cum-skip90-full      $C --trials 3000 --skip-a 900 --skip-b 900 --backstop full
run a9-cum-skip90-full-big  $C --trials 3000 --n 2097152 --m 2097152 --skip-a 900 --skip-b 900 --backstop full
# no injection: backstop off vs full, same load -> no stuck either way, cost of the backstop
run a10-cum-noinj-off       $C --trials 6000 --backstop off
run a11-cum-noinj-full      $C --trials 6000 --backstop full
