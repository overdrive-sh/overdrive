#!/usr/bin/env bash
# Ad-hoc evidence capture for E-3 (not a test, not part of any crate). Second attempt: the first
# attempt, in capture.sh, ran the benchmark under the same OVERDRIVE_METAL_RUN_ID as E-2 (the run
# id comes from that variable), so the runner refused to reuse the result directory (exit 70,
# receipts in e3-attempt-1-run-id-collision/). This run gives the killed benchmark its own id.
#   cargo xtask metal run --exclusive -- bash benchmarks/results/cgroup-connect-hook/receipts/capture-e3.sh
ROOT="$OVERDRIVE_METAL_SOURCE_ROOT"
FAM=cgroup-connect-hook
RES="$ROOT/benchmarks/results"
R="$RES/$FAM/receipts"
resultdir() { sed -n 's/^OVERDRIVE_BENCH_RESULT_DIR=//p' "$1" | head -1; }
report() { cargo run -q -p overdrive-bench --bin bench-report -- "$@"; }
probe() {
  echo "kernel.bpf_stats_enabled=$(cat /proc/sys/kernel/bpf_stats_enabled)"
  holders=$(for f in /proc/[0-9]*/fd/*; do l=$(readlink "$f" 2>/dev/null); [ "$l" = "anon_inode:bpf-stats" ] && echo "$f"; done)
  echo "bpf-stats descriptor holders: $(printf '%s' "$holders" | grep -c .)"
  [ -n "$holders" ] && echo "$holders"
}
echo "uname -r: $(uname -r)"
{ echo "before benchmark:"; probe; } > "$R/e3-probe-before.txt"
export OVERDRIVE_METAL_RUN_ID=$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')
echo "benchmark run id: $OVERDRIVE_METAL_RUN_ID" | tee "$R/e3.runid"
cargo bench -p overdrive-bench-families --bench benchmarks -- run --family "$FAM" --results-root "$RES" > "$R/e3.stdout" 2> "$R/e3.stderr" &
BPID=$!
for _ in $(seq 1500); do [ -n "$(resultdir "$R/e3.stdout")" ] && break; sleep 0.2; done
D3=$(resultdir "$R/e3.stdout"); echo "$D3" > "$R/e3.resultdir"
echo "E-3 dir: $D3"
for _ in $(seq 3000); do grep -q '"t":"replicate"' "$ROOT/$D3/record.jsonl" 2>/dev/null && break; sleep 0.1; done
pkill -KILL -f 'release/deps/benchmarks-'; kill -KILL "$BPID" 2>/dev/null; wait "$BPID" 2>/dev/null
sleep 1
{ echo "after SIGKILL:"; probe; echo "benchmark processes: $(pgrep -fc 'release/deps/benchmarks-')"; } > "$R/e3-probe.txt"
{ echo "files in $D3:"; ls -l "$ROOT/$D3"; echo "line kinds:"; sed -n 's/^{"t":"\([a-z_]*\)".*/\1/p' "$ROOT/$D3/record.jsonl" | sort | uniq -c; } > "$R/e3-record-shape.txt"
report render "$ROOT/$D3/record.jsonl" > "$R/e3-render.stdout" 2> "$R/e3-render.stderr"; echo $? > "$R/e3-render.status"
cat "$R/e3-record-shape.txt" "$R/e3-probe.txt" "$R/e3-render.stdout" | head -40
