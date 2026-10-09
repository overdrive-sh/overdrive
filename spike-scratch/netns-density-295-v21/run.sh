#!/usr/bin/env bash
# V-21 runner (throwaway). Run as root on a Linux host/VM. Everything is prefixed v21- ; cleanup runs on exit.
set -u
here="$(cd "$(dirname "$0")" && pwd)"; mkdir -p "$here/out"
tag="${V21_TAG:-$(uname -m)-$(date -u +%Y%m%dT%H%M%SZ)}"; log="$here/out/run-$tag.log"
cleanup() {
  ip netns exec v21-host nft delete table ip v21-mtls 2>/dev/null
  [ -f /tmp/v21-peer.pid ] && kill "$(cat /tmp/v21-peer.pid)" 2>/dev/null; rm -f /tmp/v21-peer.pid /tmp/v21-peer.log /tmp/v21-rs.nft /tmp/v21-atomic.nft
  ip netns del v21-peer 2>/dev/null; ip netns del v21-host 2>/dev/null
}
trap cleanup EXIT; cleanup
{
  echo "== V-21 run $tag  $(date -u +%FT%TZ)"
  set -e
  ip netns add v21-host; ip netns add v21-peer
  ip -n v21-host link set lo up; ip -n v21-peer link set lo up
  ip link add v21-h type veth peer name v21-p
  ip link set v21-h netns v21-host; ip link set v21-p netns v21-peer
  ip -n v21-host addr add 10.77.21.1/30 dev v21-h; ip -n v21-host link set v21-h up
  ip -n v21-peer addr add 10.77.21.2/30 dev v21-p; ip -n v21-peer link set v21-p up
  # the design's gateway on the bridge + shared tagged `local` route for the guest prefix on lo (ADR-0152)
  ip -n v21-host link add v21-br0 type bridge; ip -n v21-host link set v21-br0 up
  ip -n v21-host addr add 100.95.0.1/16 dev v21-br0
  ip -n v21-host route add local 100.95.0.0/16 dev lo proto 251
  # production TPROXY policy routing (fwmark 1 -> table 100: local 0.0.0.0/0 dev lo)
  ip -n v21-host rule add fwmark 1 lookup 100; ip -n v21-host route add local 0.0.0.0/0 dev lo table 100
  # a host-local operator-added address and a local link-local one
  ip -n v21-host addr add 192.0.2.55/32 dev lo; ip -n v21-host addr add 169.254.1.1/32 dev lo
  # off-host destinations live in the peer namespace, reached over the veth
  ip -n v21-peer addr add 198.51.100.7/32 dev lo; ip -n v21-peer addr add 169.254.169.254/32 dev lo
  ip -n v21-host route add 198.51.100.7/32 via 10.77.21.2; ip -n v21-host route add 169.254.169.254/32 via 10.77.21.2
  ip -n v21-peer route add default via 10.77.21.1
  ip netns exec v21-host sysctl -qw net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.default.rp_filter=0
  set +e
  rm -f /tmp/v21-peer.log
  ip netns exec v21-peer python3 "$here/peer.py" /tmp/v21-peer.log >/dev/null 2>&1 & echo $! > /tmp/v21-peer.pid
  sleep 0.5
  ip netns exec v21-host python3 "$here/probe.py" "$@"
  echo "== exit status: $?"
} 2>&1 | tee "$log"
echo "log: $log"
