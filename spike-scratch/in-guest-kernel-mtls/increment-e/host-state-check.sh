#!/usr/bin/env bash
# Read-only end-of-probe check for Spike B2 (GH #303, increment-e): nothing from
# this probe (or the earlier increments) still running or attached; report the
# retained user-space trees. Creates nothing.
set -uo pipefail
SCRATCH="$HOME/igkm-spike-a"
echo "uname -r:                     $(uname -r)"
echo "firecracker processes:        [$(pgrep -a firecracker | tr '\n' ';')]"
echo "jailer processes:             [$(pgrep -a jailer | tr '\n' ';')]"
echo "peer/relay/inclient procs:    [$(pgrep -af 'igkm[de]-host-target' | tr '\n' ';')]"
echo "tcpdump processes:            [$(pgrep -a tcpdump | tr '\n' ';')]"
echo "listener processes (A-A3):    [$(pgrep -af host_listener.py | tr '\n' ';')]"
echo "igk* links on host:           [$(ip -o link show 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | grep -E '^igk' | tr '\n' ' ')]"
echo "tun/tap links on host:        [$(ip -o link show type tun 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | tr '\n' ' ')]"
echo "192.168.203.x/204.x on host:  $(ip -4 -br addr | grep -cE '192\.168\.20[34]\.')"
echo "listeners 5001/6443-6450/7100: [$(ss -Htlnp 2>/dev/null | grep -E ':(5001|644[3-9]|6450|7100)\b' | tr '\n' ';')]"
echo "unix sockets under scratch:   $(find "$SCRATCH" -type s 2>/dev/null | wc -l)"
echo "per-run dirs left:            $(ls -d "$SCRATCH"/runs "$SCRATCH"/runs-fc "$SCRATCH"/runs-ukfc "$SCRATCH"/runs-igkmd "$SCRATCH"/runs-igkme 2>/dev/null || echo none)"
echo "root-owned files in scratch:  $(find "$SCRATCH" -user root 2>/dev/null | wc -l)"
for inc in increment-d increment-e; do
  echo "root-owned files in $inc: $(find "$HOME/overdrive/spike-scratch/in-guest-kernel-mtls/$inc" -user root 2>/dev/null | wc -l)"
done
echo "unikraft worktrees:"
git -C "$SCRATCH/unikraft/unikraft" worktree list 2>&1
echo "lib-lwip worktrees:"
git -C "$SCRATCH/unikraft/lib-lwip" worktree list 2>&1
echo "pristine clones clean: unikraft dirty=$(git -C "$SCRATCH/unikraft/unikraft" status --porcelain | wc -l) lib-lwip dirty=$(git -C "$SCRATCH/unikraft/lib-lwip" status --porcelain | wc -l)"
echo "Spike B worktree unikraft-igkmd diff --stat (must be 0001+0002 only):"
git -C "$SCRATCH/unikraft/unikraft-igkmd" diff --stat 2>&1
echo "retained user-owned trees (outside ~/overdrive):"
du -sh "$SCRATCH" 2>/dev/null
du -sh "$SCRATCH"/* 2>/dev/null
du -sh "$SCRATCH"/unikraft/* 2>/dev/null
