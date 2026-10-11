#!/bin/bash
# does the perf-attached vsock tracepoint program record recursion_misses (bpf_prog_active skips) during crash-fin reps?
cd /tmp/v31-work
ulimit -n 1048576
sysctl -w kernel.bpf_stats_enabled=1 > evidence/cf6.sysctl 2>&1
rm -f stop-poll; ( while [ ! -e stop-poll ]; do bpftool prog show 2>/dev/null | grep -A1 -E "name vsock_recv" | tr '\n' ' ' ; echo; sleep 0.2; done ) > evidence/cf6.progstats 2>&1 &
PP=$!
V31_PERFTP=1 bash runplan2.sh loop cf6perf crashfin_perf.json
touch stop-poll; wait $PP; sysctl -w kernel.bpf_stats_enabled=0 >/dev/null 2>&1
grep -v '^$' evidence/cf6.progstats | tail -3 | cut -c1-400
grep -o "recursion_misses [0-9]*" evidence/cf6.progstats | sort -u -k2 -n | tail -3
