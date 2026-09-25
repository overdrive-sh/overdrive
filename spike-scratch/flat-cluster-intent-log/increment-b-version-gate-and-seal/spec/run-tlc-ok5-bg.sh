#!/usr/bin/env bash
# Detached exhaustive TLC run of ok5 (added on resume). The first attempt died with ENOSPC on the VM's
# 7.8 GB /tmp tmpfs (evidence/tlc-ok5-AllSafety-attempt1-tmpfs-enospc.txt); this one puts Quint's
# tmpdir + TLC's -metadir on the root disk. A disk guard stops TLC (and records it) if the root disk's
# free space falls below GUARD_GB, so a sibling agent sharing the VM is never starved.
# Start:  nohup bash run-tlc-ok5-bg.sh >/dev/null 2>&1 &   (inside Lima, as root)
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVID="$HERE/../evidence"
WORK=/var/tmp/fcil-incb-tlc-ok5
GUARD_GB=${GUARD_GB:-25}
guard_log="$EVID/tlc-ok5-attempt2-diskguard.txt"
rm -f "$EVID/tlc-ok5-attempt2.done"
echo "# disk guard for ok5 TLC attempt 2 — started $(date -u +%FT%TZ) — stop if avail(/var/tmp) < ${GUARD_GB} GB" >"$guard_log"
( TLC_ENDPOINT=localhost:18932 TLC_TMPDIR="$WORK" TLC_CONFIG="$HERE/tlc-ok5.json" bash "$HERE/run-tlc.sh" ok5
  echo $? >"$EVID/tlc-ok5-attempt2.done" ) &
runner=$!
while kill -0 "$runner" 2>/dev/null; do
  sleep 120
  avail=$(df --output=avail -BG /var/tmp | tail -1 | tr -dc 0-9)
  used=$(du -s -BG "$WORK" 2>/dev/null | cut -f1)
  echo "$(date -u +%FT%TZ) avail=${avail}G work=${used}" >>"$guard_log"
  if [ "${avail:-0}" -lt "$GUARD_GB" ]; then
    echo "$(date -u +%FT%TZ) DISK GUARD TRIPPED — stopping TLC (tlc2.TLC under $WORK)" >>"$guard_log"
    pkill -f "tlc2.TLC.*$WORK" || true
  fi
done
echo "# runner exited $(date -u +%FT%TZ)" >>"$guard_log"
rm -rf "$WORK"
echo "# removed $WORK" >>"$guard_log"
