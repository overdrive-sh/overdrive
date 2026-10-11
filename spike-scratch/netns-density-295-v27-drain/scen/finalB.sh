#!/bin/bash
cd /tmp/v27-work
sha256sum out/drainprobe out/drain-bpf.o
bash t2.sh
