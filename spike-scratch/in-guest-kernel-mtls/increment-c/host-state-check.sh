#!/usr/bin/env bash
# Read-only end-of-probe check for Spike A3: nothing from this probe (or the two
# Nanos probes) still running or attached; report the retained user-space build
# tree. Creates nothing.
set -uo pipefail
SCRATCH="$HOME/igkm-spike-a"
echo "firecracker processes:       [$(pgrep -a firecracker | tr '\n' ';')]"
echo "jailer processes:            [$(pgrep -a jailer | tr '\n' ';')]"
echo "igkm listener processes:     [$(pgrep -af host_listener.py | tr '\n' ';')]"
for t in igkmc0 igkmb0 igkma0; do
  echo "tap $t present:          $(ip link show "$t" >/dev/null 2>&1 && echo yes || echo no)"
done
echo "tun/tap links on host:       [$(ip -o link show type tun 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | tr '\n' ' ')]"
echo "192.168.203.x on host:       $(ip -4 -br addr | grep -c '192\.168\.203\.')"
echo "unix sockets under scratch:  $(find "$SCRATCH" -type s 2>/dev/null | wc -l)"
echo "per-run dirs left:           $(ls -d "$SCRATCH"/runs "$SCRATCH"/runs-fc "$SCRATCH"/runs-ukfc 2>/dev/null || echo none)"
echo "root-owned files in scratch: $(find "$SCRATCH" -user root 2>/dev/null | wc -l)"
echo "retained build tree (user-owned, outside ~/overdrive):"
du -sh "$SCRATCH" 2>/dev/null
du -sh "$SCRATCH"/* 2>/dev/null
du -sh "$SCRATCH"/unikraft/* 2>/dev/null
