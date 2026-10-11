#!/bin/bash
# T2: R1 event correctness (set-goal-then-check + BPF ringbuf completion record); R1-alt = kernel-armed TCP goals
cd /tmp/v31-work
export V27_TIMEOUT=900
for ARM in kernel; do
  bash hostrun.sh t2-$ARM-mix-25k --arm $ARM --trials 25000 --conc 256 --flows 512 --srv-pause mix --cli-pause mix --io-timeout 60 --diag-seq --events-sample 2000
  for P in 0 300 3000; do
    bash hostrun.sh t2-$ARM-2MiB-p$P --arm $ARM --trials 4000 --conc 400 --flows 512 --n 2097152 --m 2097152 --srv-pause $P --cli-pause $P --io-timeout 60 --diag-seq --events-sample 500
  done
  # forced races: small payloads so sent == goal at arm time is common, arm spun into the window of the final
  # skb_send_sock calls, owners pinned to separate CPUs
  bash hostrun.sh t2-$ARM-race-pin --arm $ARM --race-spin --pin --trials 25000 --conc 64 --flows 128 --n small --m small --fin mix --io-timeout 60 --diag-seq --events-sample 2000
  # cell reuse across flows: 32 cell pairs, ~800 reuses each
  bash hostrun.sh t2-$ARM-reuse --arm $ARM --trials 25000 --conc 24 --flows 32 --io-timeout 60 --diag-seq --events-sample 2000
  # server speaks first (banner before the request is read), stresses the reply direction
  bash hostrun.sh t2-$ARM-srvfirst --arm $ARM --trials 10000 --conc 128 --flows 256 --srv-first yes --n small --m small --io-timeout 60 --diag-seq --events-sample 2000
done
