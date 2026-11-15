#!/usr/bin/env bash
# V-3 probe runner. Run on metal, exclusive: the ordinary `metal run` grammar refuses a script/binary, and a
# timing measurement must not share the host with another worktree's ordinary runs.
#   cargo xtask metal run --exclusive -- env COMMIT=<sha> NS=10,100 SAMPLES=3 TAG=smoke \
#       bash spike-scratch/reconciler-resync-cost/run.sh
set -euo pipefail
export PATH=$HOME/.cargo/bin:$PATH
here="$(cd "$(dirname "$0")" && pwd)"
NS="${NS:-10,100,500,1000}"; SAMPLES="${SAMPLES:-5}"; TAG="${TAG:-run}"
ev="$here/evidence/${TAG}-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$ev" "$here/out/data"
trap 'chown -R "${SUDO_UID:-0}:${SUDO_GID:-0}" "$here/evidence" "$here/out" 2>/dev/null || true' EXIT
{
  echo "date_utc: $(date -u +%FT%TZ)"
  echo "commit: ${COMMIT:-unknown}"
  echo "uname -a: $(uname -a)"
  echo "uname -r: $(uname -r)"
  echo "nproc: $(nproc)"
  echo "--- lscpu"; lscpu
  echo "--- free -m"; free -m
  echo "--- data dir fs"; df -T "$here/out/data"
  echo "--- root mount"; findmnt -no SOURCE,FSTYPE,OPTIONS /
  echo "--- rotational"; for f in /sys/block/*/queue/rotational; do echo "$f $(cat "$f")"; done
  echo "--- load before"; cat /proc/loadavg
  echo "--- rustc"; rustc -V
} > "$ev/host.txt" 2>&1
cargo build --release --manifest-path "$here/Cargo.toml" > "$ev/build.log" 2>&1
tail -3 "$ev/build.log"
"$here/target/release/resync-cost" "$ev" "$here/out/data" "$NS" "$SAMPLES" 2>&1 | tee "$ev/stdout.log"
{ echo "--- load after"; cat /proc/loadavg; } >> "$ev/host.txt"
echo "EVIDENCE_DIR=$ev"
