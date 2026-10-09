#!/usr/bin/env bash
# V-26 probe runner (throwaway). Must run as root on the Linux host/VM.
# Usage: run.sh [phase]    phases: p0 p1 p2 p3 p4 p5 (default: all)
# Everything created is prefixed v26- ; cleanup runs on exit.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
tag="${V26_TAG:-$(uname -m)-$(date -u +%Y%m%dT%H%M%SZ)}"
log="$here/out/run-$tag${1:+-$1}.log"
mkdir -p "$here/out"

cleanup() {
  rm -f /sys/fs/bpf/v26-steer/link 2>/dev/null
  rmdir /sys/fs/bpf/v26-steer 2>/dev/null
  ip netns exec v26-host nft delete table ip v26-mtls 2>/dev/null
  ip netns del v26-peer 2>/dev/null
  ip netns del v26-host 2>/dev/null
}
trap cleanup EXIT
cleanup

{
  echo "== V-26 run $tag"
  echo "uname -r: $(uname -r)"
  echo "uname -m: $(uname -m)"
  echo "date: $(date -u +%FT%TZ)"
  set -e
  ip netns add v26-host
  ip netns add v26-peer
  ip -n v26-host link set lo up
  ip -n v26-peer link set lo up
  ip link add v26-h type veth peer name v26-p
  ip link set v26-h netns v26-host
  ip link set v26-p netns v26-peer
  ip -n v26-host addr add 10.77.26.1/30 dev v26-h
  ip -n v26-host link set v26-h up
  ip -n v26-peer addr add 10.77.26.2/30 dev v26-p
  ip -n v26-peer link set v26-p up
  ip -n v26-peer route add 100.95.0.0/16 via 10.77.26.1
  # the design's shared, Overdrive-tagged `local` route on lo (here: proto 251 as the tag), and the production
  # TPROXY policy routing (ensure_shared_routing_infra: fwmark 0x1 -> table 100, local 0.0.0.0/0 dev lo)
  ip -n v26-host route add local 100.95.0.0/16 dev lo proto 251
  ip -n v26-host rule add fwmark 1 lookup 100
  ip -n v26-host route add local 0.0.0.0/0 dev lo table 100
  set +e
  ip netns exec v26-host sysctl -w net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.default.rp_filter=0 >/dev/null
  # not `ip netns exec`: it remounts /sys and hides /sys/fs/bpf; the probe setns()es itself
  "$here/out/v26" run "$here/out/v26-bpf.o" ${1:-}
  echo "== exit status: $?"
} 2>&1 | tee "$log"
echo "log: $log"
