#!/bin/bash
# rate of fexit(skb_send_sock) recursion misses and the stuck directions they cause, owner-armed vs kernel-armed
cd /tmp/v31-work
for S in 11 12 13 14 15 16 17 18 19 20 21 22; do
  for ARM in owner kernel; do
    bash hostrun.sh m2-$ARM-$S --arm $ARM --seed $S --trials 4000 --conc 400 --flows 512 --n 2097152 --m 2097152 --srv-pause 0 --cli-pause 0 --io-timeout 15 --diag-seq --events-sample 5000 --settle-s 5 > /dev/null
    python3 - <<PY
import json
d=json.load(open("evidence/m2-$ARM-$S.json")); b=d["bpf_counters"]
print("m2-$ARM-$S pass=%d/%d live=%d/%d fexit_calls_executed=%d recursion_misses=%s"%(d["trials"]["pass"],d["trials"]["total"],d["guest_live_flows_at_end"],d["host_live_flows_at_end"],b["fexit_send_calls"]+b["fexit_send_untracked_target"],b["fexit_recursion_misses"]))
PY
  done
done
