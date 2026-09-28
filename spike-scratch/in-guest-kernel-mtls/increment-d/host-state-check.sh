#!/usr/bin/env bash
# Read-only end-of-probe check for Spike B: nothing from this probe (or the
# earlier increments) still running or attached; report the retained
# user-space trees. Creates nothing.
set -uo pipefail
SCRATCH="$HOME/igkm-spike-a"
echo "firecracker processes:        [$(pgrep -a firecracker | tr '\n' ';')]"
echo "jailer processes:             [$(pgrep -a jailer | tr '\n' ';')]"
echo "igkmd peer/relay processes:   [$(pgrep -af 'igkmd-host-target' | tr '\n' ';')]"
echo "tcpdump processes:            [$(pgrep -a tcpdump | tr '\n' ';')]"
echo "listener processes (A-A3):    [$(pgrep -af host_listener.py | tr '\n' ';')]"
for t in igkmd0 igkmc0 igkmb0 igkma0; do
  echo "tap $t present:           $(ip link show "$t" >/dev/null 2>&1 && echo yes || echo no)"
done
echo "tun/tap links on host:        [$(ip -o link show type tun 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | tr '\n' ' ')]"
echo "192.168.203.x on host:        $(ip -4 -br addr | grep -c '192\.168\.203\.')"
echo "listeners 5001/6443-6446:     [$(ss -Htlnp 2>/dev/null | grep -E ':(5001|6443|6444|6445|6446)\b' | tr '\n' ';')]"
echo "unix sockets under scratch:   $(find "$SCRATCH" -type s 2>/dev/null | wc -l)"
echo "per-run dirs left:            $(ls -d "$SCRATCH"/runs "$SCRATCH"/runs-fc "$SCRATCH"/runs-ukfc "$SCRATCH"/runs-igkmd 2>/dev/null || echo none)"
echo "root-owned files in scratch:  $(find "$SCRATCH" -user root 2>/dev/null | wc -l)"
echo "root-owned files in increment-d: $(find "$HOME/overdrive/spike-scratch/in-guest-kernel-mtls/increment-d" -user root 2>/dev/null | wc -l)"
echo "unikraft worktrees:"
git -C "$SCRATCH/unikraft/unikraft" worktree list 2>&1
echo "retained user-owned trees (outside ~/overdrive):"
du -sh "$SCRATCH" 2>/dev/null
du -sh "$SCRATCH"/* 2>/dev/null
du -sh "$SCRATCH"/unikraft/* 2>/dev/null
