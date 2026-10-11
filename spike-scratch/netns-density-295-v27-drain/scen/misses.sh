#!/bin/bash
cd /tmp/v27-work
sysctl kernel.bpf_stats_enabled
for S in 1 2 3 4 5 6; do
  for ARM in kernel owner; do
    bash hostrun.sh miss-$ARM-$S --arm $ARM --seed $S --trials 4000 --conc 400 --flows 512 --n 2097152 --m 2097152 --srv-pause 300 --cli-pause 300 --io-timeout 15 --diag-seq --events-sample 5000 | grep -E "^===|^trials|live_at"| sed -E 's/args:.*//; s/ bpf:.*(fexit_recursion_misses=)/ \1/'
  done
done
