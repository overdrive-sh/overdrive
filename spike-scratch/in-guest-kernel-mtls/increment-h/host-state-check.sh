#!/usr/bin/env bash
# Spike C (GH #303, increment-h): read-only end-of-probe host-state check.
# Proves the HOST kernel was never touched (the .ko loaded only in the guest)
# and no probe object (tap, firecracker, listener, run dir, netns address)
# leaked. Creates nothing.
set -uo pipefail
SCRATCH="$HOME/igkm-spike-a"
echo "host uname -r:                $(uname -r)"
echo "--- HOST kernel untouched (our module must NOT be on the host) ---"
echo "igkmh modules on host:        [$(lsmod | grep -E 'igkmh' | tr '\n' ';')]"
echo "  (expected empty: the .ko was insmod'd ONLY inside the disposable guest)"
echo "host tainted?:                $(cat /proc/sys/kernel/tainted) (nonzero only if something loaded an OOT module ON THE HOST; our guest taint does not count here)"
echo "--- no leaked probe objects on the host ---"
echo "firecracker procs:            [$(pgrep -a firecracker | tr '\n' ';')]"
echo "peer/relay/inclient procs:    [$(pgrep -af 'igkm[fg]-host-target|/release/(peer|relay|inclient)' | tr '\n' ';')]"
echo "tcpdump procs:                [$(pgrep -a tcpdump | tr '\n' ';')]"
echo "igkh* taps on host:   [$(ip -o link show 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | grep -E '^igkh' | tr '\n' ' ')]"
echo "192.168.204.x/205.x on host:  $(ip -4 -br addr | grep -cE '192\.168\.20[45]\.')"
echo "test listeners 5001/6443-6451/7100: [$(ss -Htlnp 2>/dev/null | grep -E ':(5001|644[3-9]|645[012]|7100)\b' | tr '\n' ';')]"
echo "igkmh run dirs left:          [$(ls -d "$SCRATCH"/runs-igkmh 2>/dev/null || echo none)]"
echo "vsock unix sockets under scratch: $(find "$SCRATCH"/runs-igkmh -type s 2>/dev/null | wc -l)"
echo "--- retained (intended) throwaway artifacts in the unsynced scratch tree ---"
du -sh "$SCRATCH/igkmh" 2>/dev/null
ls -l "$SCRATCH/igkmh/out/module/"*.ko "$SCRATCH/igkmh/out/"*.cpio.gz "$SCRATCH/igkmh/out/bin/igkmh_app" 2>/dev/null
echo "--- host /boot kernel is the stock image we copied (sha) ---"
sha256sum "$SCRATCH/igkmh/kernel/vmlinuz-host" 2>/dev/null
echo "VERDICT: host clean = $( ([[ -z "$(pgrep -f firecracker)" ]] && [[ -z "$(ip -o link show 2>/dev/null | grep -E 'igkh')" ]] && [[ -z "$(lsmod | grep igkmh)" ]]) && echo YES || echo CHECK-ABOVE)"
