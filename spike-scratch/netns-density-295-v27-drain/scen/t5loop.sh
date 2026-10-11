#!/bin/bash
cd /tmp/v27-work
ulimit -n 1048576
modprobe vsock_loopback
mkdir -p evidence
N=${1:-300}
(./out/drainprobe t5 --mode obs --trials $N --obj out/drain-bpf.o --port 7960 --wait-s 1200 > evidence/t5-loop-$N.json 2>evidence/t5-loop-$N.err &)
sleep 3
./out/drainprobe t5 --mode runner --cid 1 --port 7960 --trials $N --pace-ms 40 2>&1 | grep -v "^note\|^$\|panicked\|child connect" | tail -3
sleep 4
python3 - <<PY
import json
d=json.loads([l for l in open("evidence/t5-loop-$N.json") if l.startswith("{")][-1])
for k in ("trials_observed","timeouts","signature_at_rdhup_wake","no_record_at_rdhup_wake","record_only_after_wake","poll_bits_at_wake","record_to_wake_lag_ns"): print(k, json.dumps(d[k]))
PY
tail -3 evidence/t5-loop-$N.err
