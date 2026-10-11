#!/bin/bash
cd /tmp/v31-work
sha256sum out/drainprobe out/drain-bpf.o
bash t1.sh
bash t4.sh
