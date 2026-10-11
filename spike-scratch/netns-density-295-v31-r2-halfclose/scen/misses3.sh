#!/bin/bash
# same volume as misses2, with TWO independent fexit(skb_send_sock) counters (own recursion guards); owner uses max()
cd /tmp/v31-work
sha256sum out/drainprobe out/drain-bpf.o
for S in 11 12 13 14 15 16 17 18 19 20 21 22; do
  for ARM in owner kernel; do
    bash hostrun.sh m3-$ARM-$S --arm $ARM --seed $S --trials 4000 --conc 400 --flows 512 --n 2097152 --m 2097152 --srv-pause 0 --cli-pause 0 --io-timeout 15 --diag-seq --events-sample 5000 --settle-s 5 > /dev/null
    python3 - <<PY
import json
d=json.load(open("evidence/m3-$ARM-$S.json")); b=d["bpf_counters"]; g=d["guest"]; h=d["host"]
dis={k:(g.get(k,0)+h.get(k,0)) for k in ("counter_a_short_of_b","counter_b_short_of_a") }
print("m3-$ARM-$S pass=%d/%d live=%d/%d fexit_calls=%d misses_a=%s misses_b=%s counters_disagree=%s viol=%s"%(d["trials"]["pass"],d["trials"]["total"],d["guest_live_flows_at_end"],d["host_live_flows_at_end"],b["fexit_send_calls"]+b["fexit_send_untracked_target"],b["fexit_recursion_misses"],b["fexit_b_recursion_misses"],dis,[k for k in list(g)+list(h) if k.startswith("VIOL")]))
PY
  done
done
