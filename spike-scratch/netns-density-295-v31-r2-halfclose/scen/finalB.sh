#!/bin/bash
cd /tmp/v31-work
sha256sum out/drainprobe out/drain-bpf.o
bash t2.sh
