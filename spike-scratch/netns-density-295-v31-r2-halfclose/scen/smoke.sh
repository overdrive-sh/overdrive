#!/bin/bash
cd /tmp/v31-work
sha256sum out/drainprobe out/drain-bpf.o
./out/drainprobe selftest --obj out/drain-bpf.o $(cat offs.args 2>/dev/null) 2>&1 | head -20
bash hostrun.sh smoke-p0 --arm owner --trials 2000 --conc 256 --flows 512 --n 1048576 --m 1048576 --io-timeout 40 --events-sample 100
