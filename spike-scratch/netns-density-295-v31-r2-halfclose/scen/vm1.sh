#!/bin/bash
# real microVM (CH, vhost-kernel vsock, 7.0.0-29 host + guest): guest forwarder role in the VM, host role + destination on the host
cd /tmp/v31-work
ulimit -n 1048576
C="--diag-seq --io-timeout 40 --events-sample 500"
run() { L=$1; shift; timeout 900 python3 vmrun.py drain $L "$@" 2>&1 | grep -E "^===|^trials|^ guest|^ host|^   gap|^ live_at|^ EX|FAILED|Traceback|Error" ; }
run vm-owner-mix  --arm owner  --trials 8000 --conc 64 --flows 256 --srv-pause mix --cli-pause mix $C
run vm-kernel-mix --arm kernel --trials 8000 --conc 64 --flows 256 --srv-pause mix --cli-pause mix $C
for P in 0 300 3000; do
  run vm-owner-2MiB-p$P --arm owner --trials 1500 --conc 128 --flows 256 --n 2097152 --m 2097152 --srv-pause $P --cli-pause $P $C
done
run vm-kernel-2MiB-p300 --arm kernel --trials 1500 --conc 128 --flows 256 --n 2097152 --m 2097152 --srv-pause 300 --cli-pause 300 $C
run vm-naive-p300 --arm none --trials 1500 --conc 128 --flows 256 --n 1048576 --m 1048576 --srv-pause 300 --cli-pause 300 $C
run vm-owner-race-small --arm owner --race-spin --trials 4000 --conc 64 --flows 128 --n small --m small --fin mix $C
run vm-kernel-race-small --arm kernel --trials 4000 --conc 64 --flows 128 --n small --m small --fin mix $C
run vm-owner-reuse --arm owner --trials 8000 --conc 24 --flows 32 $C
