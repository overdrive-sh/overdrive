#!/usr/bin/env bash
# Spike D (GH #303, increment-i): read-only end-of-probe host-state check.
# Proves the HOST kernel was never touched (the .ko loaded only in the guest)
# and no probe object (tap, firecracker, listener, run dir, netns address)
# leaked -- from THIS increment (igkd* taps, igkmd scratch) OR a prior
# interrupted Spike D attempt. Creates nothing.
set -uo pipefail
SCRATCH="$HOME/igkm-spike-a"
echo "host uname -r:                $(uname -r)"
echo "--- HOST kernel untouched (our module must NOT be on the host) ---"
echo "igkm* modules on host:        [$(lsmod | grep -E 'igkm' | tr '\n' ';')]"
echo "  (expected empty: the .ko was insmod'd ONLY inside the disposable guest)"
echo "host tainted?:                $(cat /proc/sys/kernel/tainted) (nonzero only if something loaded an OOT module ON THE HOST; our guest taint does not count here)"
echo "--- no leaked probe objects on the host (this increment AND any interrupted prior attempt) ---"
echo "firecracker procs:            [$(pgrep -a firecracker | tr '\n' ';')]"
echo "peer/relay/inclient procs:    [$(pgrep -af 'igkm[dgh]-host-target|/release/(peer|relay|inclient)' | tr '\n' ';')]"
echo "tcpdump procs:                [$(pgrep -a tcpdump | tr '\n' ';')]"
echo "igk* taps on host (igkd/igkh): [$(ip -o link show 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | grep -E '^igk[dh]' | tr '\n' ' ')]"
echo "192.168.204.x/205.x on host:  $(ip -4 -br addr | grep -cE '192\.168\.20[45]\.')"
echo "test listeners 5001/6443-6451/7100: [$(ss -Htlnp 2>/dev/null | grep -E ':(5001|644[3-9]|645[012]|7100)\b' | tr '\n' ';')]"
echo "igkmd/igkmh run dirs left:    [$(ls -d "$SCRATCH"/runs-igkm* 2>/dev/null | tr '\n' ';' || echo none)]"
echo "vsock unix sockets under scratch: $(find "$SCRATCH"/runs-igkm* -type s 2>/dev/null | wc -l)"
echo "health-dir residue under scratch: [$(ls -d "$SCRATCH"/igkmd/out/health 2>/dev/null | tr '\n' ';' || echo none)]"
echo "--- retained (intended) throwaway artifacts in the unsynced scratch tree ---"
du -sh "$SCRATCH/igkmd" 2>/dev/null || echo "(no igkmd scratch yet)"
ls -l "$SCRATCH/igkmd/out/module/"*.ko "$SCRATCH/igkmd/out/"*.cpio.gz "$SCRATCH/igkmd/out/bin/igkmh_app" 2>/dev/null
echo "--- host /boot kernel is the stock image we copied (sha) ---"
sha256sum "$SCRATCH/igkmd/kernel/vmlinuz-host" 2>/dev/null || echo "(no staged kernel)"
echo "VERDICT: host clean = $( ([[ -z "$(pgrep -f firecracker)" ]] && [[ -z "$(ip -o link show 2>/dev/null | grep -E 'igk[dh]')" ]] && [[ -z "$(lsmod | grep igkm)" ]] && [[ -z "$(ss -Htln 2>/dev/null | grep -E ':(5001|644[3-9]|7100)\b')" ]]) && echo YES || echo CHECK-ABOVE)"
