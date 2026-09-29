#!/usr/bin/env bash
# Read-only end-of-probe check for Spike B3 (GH #303, increment-f): nothing from
# this probe (or the earlier increments) still running or attached; report the
# retained user-space trees. Creates nothing.
set -uo pipefail
SCRATCH="$HOME/igkm-spike-a"
W="$SCRATCH/unikraft"
echo "uname -r:                     $(uname -r)"
echo "firecracker processes:        [$(pgrep -a firecracker | tr '\n' ';')]"
echo "jailer processes:             [$(pgrep -a jailer | tr '\n' ';')]"
echo "peer/relay/inclient procs:    [$(pgrep -af 'igkm[def]-host-target' | tr '\n' ';')]"
echo "test binaries running:        [$(pgrep -af 'out/bin/(igkmf-app|igkmf-go|busybox)' | tr '\n' ';')]"
echo "tcpdump processes:            [$(pgrep -a tcpdump | tr '\n' ';')]"
echo "igk* links on host:           [$(ip -o link show 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | grep -E '^igk' | tr '\n' ' ')]"
echo "tun/tap links on host:        [$(ip -o link show type tun 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | tr '\n' ' ')]"
echo "192.168.203.x/204.x on host:  $(ip -4 -br addr | grep -cE '192\.168\.20[34]\.')"
echo "listeners 5001/6443-6451/7100: [$(ss -Htlnp 2>/dev/null | grep -E ':(5001|644[3-9]|645[01]|7100)\b' | tr '\n' ';')]"
echo "unix sockets under scratch:   $(find "$SCRATCH" -type s 2>/dev/null | wc -l)"
echo "per-run dirs left:            $(ls -d "$SCRATCH"/runs "$SCRATCH"/runs-fc "$SCRATCH"/runs-ukfc "$SCRATCH"/runs-igkmd "$SCRATCH"/runs-igkme "$SCRATCH"/runs-igkmf 2>/dev/null || echo none)"
echo "root-owned files in scratch:  $(find "$SCRATCH" -user root 2>/dev/null | wc -l)"
for inc in increment-d increment-e increment-f; do
  echo "root-owned files in $inc: $(find "$HOME/overdrive/spike-scratch/in-guest-kernel-mtls/$inc" -user root 2>/dev/null | wc -l)"
done
echo "unikraft worktrees:"
git -C "$W/unikraft" worktree list 2>&1
echo "lib-lwip worktrees:"
git -C "$W/lib-lwip" worktree list 2>&1
echo "pristine clones clean: unikraft dirty=$(git -C "$W/unikraft" status --porcelain | wc -l) lib-lwip dirty=$(git -C "$W/lib-lwip" status --porcelain | wc -l)"
echo "app-elfloader HEAD=$(git -C "$W/app-elfloader" rev-parse HEAD) dirty=$(git -C "$W/app-elfloader" status --porcelain | wc -l)"
echo "lib-libelf    HEAD=$(git -C "$W/lib-libelf" rev-parse HEAD) dirty=$(git -C "$W/lib-libelf" status --porcelain | wc -l)"
echo "Spike B  worktree unikraft-igkmd diff --stat (must be 0001+0002 only):"
git -C "$W/unikraft-igkmd" diff --stat 2>&1
echo "Spike B2 worktrees unikraft-igkme (0001+0002) and lib-lwip-igkme (0003):"
git -C "$W/unikraft-igkme" diff --stat 2>&1
git -C "$W/lib-lwip-igkme" diff --stat 2>&1
echo "Spike B3 worktrees unikraft-igkmf (0001+0002) and lib-lwip-igkmf (0003):"
git -C "$W/unikraft-igkmf" diff --stat 2>&1
git -C "$W/lib-lwip-igkmf" diff --stat 2>&1
echo "retained user-owned trees (outside ~/overdrive):"
du -sh "$SCRATCH" 2>/dev/null
du -sh "$SCRATCH"/igkmf-host-target "$SCRATCH"/stretch 2>/dev/null
du -sh "$W"/*igkmf* "$W"/app-elfloader "$W"/lib-libelf 2>/dev/null
echo "gitignored out/ of increment-f on the host:"
du -sh "$HOME/overdrive/spike-scratch/in-guest-kernel-mtls/increment-f/out" 2>/dev/null
ls -l "$HOME/overdrive/spike-scratch/in-guest-kernel-mtls/increment-f/out/bin" 2>/dev/null
