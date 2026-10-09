#!/usr/bin/env bash
# V-18 launcher. NEVER runs `cargo xtask metal run` with sync (its rsync --delete over the shared
# /home/ubuntu/overdrive would clobber concurrent spikes). Copies this directory to /tmp/v18-work on the
# metal host with scp, runs the increment there as root over ssh, and retrieves evidence.
# usage: launch.sh increment-a [args...]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
INC="${1:?increment}"; shift || true
T="$(grep '^OVERDRIVE_METAL_TARGET=' "$ROOT/.env" | cut -d= -f2-)"
ADDR="${T##*@}"
SSH=(ssh -o BatchMode=yes "$T")
redact() { sed "s#$T#<metal-target>#g; s#$ADDR#<metal-address>#g"; }
mkdir -p "$HERE/out/launch"
# honour a held shared metal lease (informational; this run touches nothing under /home/ubuntu/overdrive)
"${SSH[@]}" 'test -r /run/lock/overdrive-metal-shared.owner && { echo "SHARED LEASE OWNER:"; cat /run/lock/overdrive-metal-shared.owner; } || echo "no shared metal lease owner"' | redact
tar -C "$HERE" --exclude=out --exclude='*/evidence' --exclude=target --exclude=__pycache__ -czf "$HERE/out/launch/src.tgz" .
"${SSH[@]}" 'mkdir -p /tmp/v18-work'
scp -q -o BatchMode=yes "$HERE/out/launch/src.tgz" "$T:/tmp/v18-work/src.tgz"
"${SSH[@]}" 'cd /tmp/v18-work && tar -xzf src.tgz && sudo -n true && P=/home/ubuntu/overdrive/spike-scratch/netns-density-295-guest-vsock-capture/out && mkdir -p prior && { test -x prior/cloud-hypervisor || sudo cp -a $P/ch-target/release/cloud-hypervisor prior/cloud-hypervisor; } && { test -d prior/cargo-home || sudo cp -a $P/cargo-home prior/cargo-home; } && ls prior'
echo "{\"increment\":\"$INC\",\"sources_sha256\":\"$(shasum -a 256 "$HERE/out/launch/src.tgz" | cut -d' ' -f1)\",\"head\":\"$(git -C "$ROOT" rev-parse HEAD)\"}"
set +e
"${SSH[@]}" "cd /tmp/v18-work && { test -d $INC/evidence && sudo mv $INC/evidence $INC/evidence.prev.\$(date +%s); true; } && sudo -n env HOME=/home/ubuntu PATH=/home/ubuntu/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin python3 $INC/run.py $*" 2>&1 | redact | tee "$HERE/out/launch/$INC.log"
RC=${PIPESTATUS[0]}
set -e
mkdir -p "$HERE/$INC"
rm -rf "$HERE/$INC/evidence"
"${SSH[@]}" "sudo -n tar -C /tmp/v18-work/$INC -czf - evidence" | tar -C "$HERE/$INC" -xzf -
find "$HERE/$INC/evidence" -type f -not -name '*.pcap' -exec sed -i '' "s#$ADDR#<metal-address>#g" {} + 2>/dev/null || true
cp "$HERE/out/launch/$INC.log" "$HERE/$INC/evidence/native-launch.log"
echo "launcher rc=$RC"
exit $RC
