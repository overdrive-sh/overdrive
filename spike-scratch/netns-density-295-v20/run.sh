#!/usr/bin/env bash
# V-20 probe runner (throwaway). Must run as root on the Linux host/VM.
# Everything created is prefixed v20- ; cleanup runs on exit.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
tag="${V20_TAG:-$(uname -m)-$(date -u +%Y%m%dT%H%M%SZ)}"
log="$here/out/run-$tag.log"
mkdir -p "$here/out"
elf="${V20_ELF:-$here/out/v20-bpf.o}"
bin="${V20_BIN:-$here/out/v20}"
cg=/sys/fs/cgroup/v20-owner

cleanup() {
  ip netns exec v20-host nft delete table ip v20-mtls 2>/dev/null
  rm -rf /sys/fs/bpf/v20-pins 2>/dev/null
  ip netns del v20-peer 2>/dev/null
  ip netns del v20-host 2>/dev/null
  if [ -d "$cg" ]; then
    echo 1 > "$cg/cgroup.kill" 2>/dev/null
    for _ in 1 2 3 4 5 6 7 8 9 10; do rmdir "$cg" 2>/dev/null && break; sleep 0.3; done
  fi
}
trap cleanup EXIT
cleanup

{
  echo "== V-20 run $tag"
  echo "uname -r: $(uname -r)"
  echo "uname -m: $(uname -m)"
  echo "date: $(date -u +%FT%TZ)"
  echo "systemd-detect-virt: $(systemd-detect-virt 2>&1)"
  set -e
  ip netns add v20-host
  ip netns add v20-peer
  ip -n v20-host link set lo up
  ip -n v20-peer link set lo up
  ip link add v20-h type veth peer name v20-p
  ip link set v20-h netns v20-host
  ip link set v20-p netns v20-peer
  ip -n v20-host addr add 10.77.20.1/24 dev v20-h
  ip -n v20-host link set v20-h up
  ip -n v20-peer addr add 10.77.20.2/24 dev v20-p
  ip -n v20-peer addr add 10.77.20.3/24 dev v20-p
  ip -n v20-peer link set v20-p up
  ip -n v20-peer route add 100.96.0.0/24 via 10.77.20.1
  # shared Overdrive-style `local` route for the guest prefix + the production TPROXY policy routing
  ip -n v20-host route add local 100.96.0.0/24 dev lo proto 251
  ip -n v20-host rule add fwmark 1 lookup 100
  ip -n v20-host route add local 0.0.0.0/0 dev lo table 100
  # leg-C model: connects from the 10.77.20.3 client class are TPROXY'd to the transparent leg-C listener;
  # every other client class reaches the sk_lookup steering (intake listeners). Same dst addr:port for both.
  ip netns exec v20-host nft add table ip v20-mtls
  ip netns exec v20-host nft add chain ip v20-mtls prerouting '{ type filter hook prerouting priority mangle; }'
  ip netns exec v20-host nft add rule ip v20-mtls prerouting ip saddr 10.77.20.3 ip daddr 100.96.0.0/24 tcp dport 8080 meta mark set 1 tproxy ip to 127.0.0.1:19999
  set +e
  ip netns exec v20-host sysctl -w net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.default.rp_filter=0 net.core.somaxconn=65535 net.ipv4.tcp_max_syn_backlog=65535 >/dev/null
  ip netns exec v20-peer sysctl -w net.ipv4.tcp_tw_reuse=1 net.ipv4.ip_local_port_range="1024 65000" net.ipv4.conf.all.rp_filter=0 >/dev/null
  mkdir "$cg"
  echo "--- nft ruleset (v20-host)"; ip netns exec v20-host nft list ruleset
  # not `ip netns exec`: it remounts /sys and hides /sys/fs/bpf; the probe setns()es itself
  "$bin" run "$elf" "$cg"
  echo "== exit status: $?"
} 2>&1 | tee "$log"
echo "log: $log"
