#!/usr/bin/env bash
# Read-only end-of-probe check: nothing from this probe still running or
# attached; report the retained user-space build tree. Creates nothing.
set -uo pipefail
SCRATCH="$HOME/igkm-spike-a"
echo "cloud-hypervisor processes: [$(pgrep -a cloud-hypervisor | tr '\n' ';')]"
echo "igkm listener processes:    [$(pgrep -af host_listener.py | tr '\n' ';')]"
echo "tap igkma0 present:         $(ip link show igkma0 >/dev/null 2>&1 && echo yes || echo no)"
echo "192.168.203.x on host:      $(ip -4 -br addr | grep -c '192\.168\.203\.')"
echo "unix sockets under scratch: $(find "$SCRATCH" -type s 2>/dev/null | wc -l)"
echo "per-run dirs left:          $(ls -d "$SCRATCH"/runs 2>/dev/null || echo none)"
echo "root-owned files in scratch: $(find "$SCRATCH" -user root 2>/dev/null | wc -l)"
echo "retained build tree (user-owned, outside ~/overdrive):"
du -sh "$SCRATCH" 2>/dev/null
du -sh "$SCRATCH"/* 2>/dev/null
