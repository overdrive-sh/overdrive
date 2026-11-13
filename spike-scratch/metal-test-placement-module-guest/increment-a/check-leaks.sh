#!/usr/bin/env bash
# Read-only inspection of the Metal box after a run: what of the probe's is
# still there? Usage: check-leaks.sh <run-id>
set -u
set -a; . "$(dirname "$0")/../../../.env"; set +a
R="${1:?run id}"
ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" bash -s "$R" <<'REMOTE'
R="$1"
echo "--- processes (broker / cloud-hypervisor, any run):"
sudo -n bash -c 'for p in /proc/[0-9]*; do c=$(cat $p/comm 2>/dev/null); case "$c" in modguest-broker|cloud-hyperviso*) echo "$(basename $p) $c $(tr "\0" " " < $p/cmdline | cut -c1-120)";; esac; done' 
echo "--- cloud-hypervisor processes naming run $R:"
sudo -n bash -c 'for p in /proc/[0-9]*; do tr "\0" " " < $p/cmdline 2>/dev/null; echo; done' | grep "cloud-hypervisor" | grep -c "$R"
echo "--- /tmp/modguest-$R:"; sudo -n ls -la /tmp/modguest-$R 2>&1
echo "--- /srv/vm/metal/$R:"; sudo -n ls /srv/vm/metal/$R 2>&1 | head -3
echo "--- /run/ovd-gk/$R:"; sudo -n ls /run/ovd-gk/$R 2>&1 | head -3
echo "--- per-run cgroups naming $R:"; sudo -n ls /sys/fs/cgroup/overdrive-metal-runs 2>&1 | grep -c "$R"
echo "--- vhost-vsock holders (fd count by pid):"; sudo -n bash -c 'for p in /proc/[0-9]*; do n=$(ls -l $p/fd 2>/dev/null | grep -c vhost-vsock); [ "$n" != 0 ] && echo "$(basename $p) $n $(tr "\0" " " < $p/cmdline | cut -c1-60)"; done' 2>&1
REMOTE
