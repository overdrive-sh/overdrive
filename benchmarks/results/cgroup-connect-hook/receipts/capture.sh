#!/usr/bin/env bash
# Ad-hoc evidence capture for E-2, E-3, E-8 and the validation of E-1 (not a test, not part of
# any crate). Run on the Metal host as one exclusive command:
#   cargo xtask metal run --exclusive -- bash benchmarks/results/cgroup-connect-hook/receipts/capture.sh
# The benchmark is invoked with the literal command of the benchmark form (bootstrap.sh).
ROOT="$OVERDRIVE_METAL_SOURCE_ROOT"
FAM=cgroup-connect-hook
RES="$ROOT/benchmarks/results"
R="$RES/$FAM/receipts"
E8="$RES/$FAM/e-8-marker-refusal"
mkdir -p "$E8"
bench() { cargo bench -p overdrive-bench-families --bench benchmarks -- run --family "$FAM" --results-root "$RES"; }
resultdir() { sed -n 's/^OVERDRIVE_BENCH_RESULT_DIR=//p' "$1" | head -1; }
report() { cargo run -q -p overdrive-bench --bin bench-report -- "$@"; }
probe() {
  echo "kernel.bpf_stats_enabled=$(cat /proc/sys/kernel/bpf_stats_enabled)"
  holders=$(for f in /proc/[0-9]*/fd/*; do l=$(readlink "$f" 2>/dev/null); [ "$l" = "anon_inode:bpf-stats" ] && echo "$f"; done)
  echo "bpf-stats descriptor holders: $(printf '%s' "$holders" | grep -c .)"
  [ -n "$holders" ] && echo "$holders"
}
echo "uname -r: $(uname -r)"

echo "=== E-1 validate"
E1=benchmarks/results/$FAM/c591ccdbe61baae25a58fead
report validate "$E1/record.jsonl" > "$R/e1-validate.stdout" 2> "$R/e1-validate.stderr"; echo $? > "$R/e1-validate.status"
cat "$R/e1-validate.status"

echo "=== E-2 (holder + benchmark)"
python3 "$R/e2-holder.py" > "$R/e2-holder.stdout" 2>&1 &
HPID=$!
for _ in $(seq 50); do [ -s "$R/e2-holder.stdout" ] && break; sleep 0.2; done
{ echo "before benchmark:"; probe; } > "$R/e2-probe-before.txt"
bench > "$R/e2.stdout" 2> "$R/e2.stderr"; echo $? > "$R/e2.status"
kill "$HPID" 2>/dev/null; wait "$HPID" 2>/dev/null
{ echo "after holder kill:"; probe; echo "holder processes: $(pgrep -fc e2-holder.py)"; } > "$R/e2-probe-after.txt"
D2=$(resultdir "$R/e2.stdout"); echo "E-2 dir: $D2 status $(cat "$R/e2.status")"
echo "$D2" > "$R/e2.resultdir"
report validate "$ROOT/$D2/record.jsonl" > "$R/e2-validate.stdout" 2> "$R/e2-validate.stderr"; echo $? > "$R/e2-validate.status"

echo "=== E-3 (kill after first replicate)"
bench > "$R/e3.stdout" 2> "$R/e3.stderr" &
BPID=$!
for _ in $(seq 1500); do [ -n "$(resultdir "$R/e3.stdout")" ] && break; sleep 0.2; done
D3=$(resultdir "$R/e3.stdout"); echo "$D3" > "$R/e3.resultdir"
for _ in $(seq 3000); do grep -q '"t":"replicate"' "$ROOT/$D3/record.jsonl" 2>/dev/null && break; sleep 0.1; done
pkill -KILL -f 'release/deps/benchmarks-'; kill -KILL "$BPID" 2>/dev/null; wait "$BPID" 2>/dev/null
sleep 1
{ echo "after SIGKILL:"; probe; echo "benchmark processes: $(pgrep -fc 'release/deps/benchmarks-')"; } > "$R/e3-probe.txt"
{ echo "files in $D3:"; ls -l "$ROOT/$D3"; echo "line kinds:"; sed -n 's/^{"t":"\([a-z_]*\)".*/\1/p' "$ROOT/$D3/record.jsonl" | sort | uniq -c; } > "$R/e3-record-shape.txt"
report render "$ROOT/$D3/record.jsonl" > "$R/e3-render.stdout" 2> "$R/e3-render.stderr"; echo $? > "$R/e3-render.status"

echo "=== E-8 (checkout marker)"
BIN=$(ls -t target/release/deps/benchmarks-* | grep -v '\.d$' | head -1)
for case in missing malformed; do
  T=$(mktemp -d); mkdir -p "$T/benchmarks/results"
  [ "$case" = malformed ] && printf 'commit=not-a-commit\nworkspace=/x\n' > "$T/.overdrive-metal-source"
  OVERDRIVE_METAL_SOURCE_ROOT="$T" "$BIN" --bench run --family "$FAM" --results-root "$T/benchmarks/results" > "$E8/$case.stdout" 2> "$E8/$case.stderr"
  echo $? > "$E8/$case.status"
  { echo "marker present: $([ -e "$T/.overdrive-metal-source" ] && echo yes || echo no)"; echo "files under results root:"; find "$T/benchmarks/results" -mindepth 1 | wc -l; } > "$E8/$case.tree"
  echo "$case: status $(cat "$E8/$case.status")"; cat "$E8/$case.stderr"
  rm -rf "$T"
done
echo "=== final probe"; probe
