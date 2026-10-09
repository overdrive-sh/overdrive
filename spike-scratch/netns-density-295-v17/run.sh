#!/usr/bin/env bash
# V-17 probe runner (throwaway). Must run as root on the Linux host/VM.
# Usage: run.sh [phase ...]   phases: control design stacked cases
# Everything created is prefixed v17- ; cleanup runs on exit.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
tag="${V17_TAG:-$(uname -m)-$(date -u +%Y%m%dT%H%M%SZ)}"
log="$here/out/run-$tag.log"
mkdir -p "$here/out"
H=v17-host

cleanup() {
  for k in 1 2 3 4 5 6; do ip netns del v17-peer$k 2>/dev/null; done
  ip netns del $H 2>/dev/null
  ip link del v17-gone 2>/dev/null
}
trap cleanup EXIT
cleanup

{
  echo "== V-17 run $tag"
  echo "uname -r: $(uname -r)"
  echo "uname -m: $(uname -m)"
  echo "date: $(date -u +%FT%TZ)"
  set -e
  ip netns add $H
  ip -n $H link set lo up
  for k in 1 2 3 4 5 6; do
    ip netns add v17-peer$k
    ip -n v17-peer$k link set lo up
    # the same destination address on every peer: only the host's policy routing can pick the egress
    ip -n v17-peer$k addr add 10.17.200.1/32 dev lo
  done
  mk() { # k mtu
    ip link add v17-h$1 type veth peer name v17-p$1
    ip link set v17-p$1 netns v17-peer$1
    ip link set v17-h$1 netns $H
    ip -n $H link set v17-h$1 mtu $2 up
    ip -n v17-peer$1 link set v17-p$1 mtu $2 up
  }
  # 1: veth, mtu 1500, source-address policy rule
  mk 1 1500
  ip -n $H addr add 10.17.1.1/24 dev v17-h1; ip -n v17-peer1 addr add 10.17.1.2/24 dev v17-p1
  ip -n $H route add 10.17.200.1/32 via 10.17.1.2 dev v17-h1 table 171
  ip -n $H rule add from 10.17.1.1/32 lookup 171 priority 1001
  # 2: veth, mtu 1280, source-address policy rule
  mk 2 1280
  ip -n $H addr add 10.17.2.1/24 dev v17-h2; ip -n v17-peer2 addr add 10.17.2.2/24 dev v17-p2
  ip -n $H route add 10.17.200.1/32 via 10.17.2.2 dev v17-h2 table 172
  ip -n $H rule add from 10.17.2.1/32 lookup 172 priority 1002
  # 3: veth, mtu 9000, fwmark policy rule
  mk 3 9000
  ip -n $H addr add 10.17.3.1/24 dev v17-h3; ip -n v17-peer3 addr add 10.17.3.2/24 dev v17-p3
  ip -n $H route add 10.17.200.1/32 via 10.17.3.2 dev v17-h3 table 173
  ip -n $H rule add fwmark 0x173 lookup 173 priority 1003
  # 4: macvlan over veth (stacked)
  mk 4 1500
  ip -n $H link add v17-mv4 link v17-h4 type macvlan mode bridge
  ip -n $H link set v17-mv4 up
  ip -n $H addr add 10.17.4.1/24 dev v17-mv4; ip -n v17-peer4 addr add 10.17.4.2/24 dev v17-p4
  ip -n $H route add 10.17.200.1/32 via 10.17.4.2 dev v17-mv4 table 174
  ip -n $H rule add from 10.17.4.1/32 lookup 174 priority 1004
  # 5: 802.1Q vlan over veth (stacked)
  mk 5 1500
  ip -n $H link add link v17-h5 name v17-h5.50 type vlan id 50
  ip -n v17-peer5 link add link v17-p5 name v17-p5.50 type vlan id 50
  ip -n $H link set v17-h5.50 up; ip -n v17-peer5 link set v17-p5.50 up
  ip -n $H addr add 10.17.5.1/24 dev v17-h5.50; ip -n v17-peer5 addr add 10.17.5.2/24 dev v17-p5.50
  ip -n $H route add 10.17.200.1/32 via 10.17.5.2 dev v17-h5.50 table 175
  ip -n $H rule add from 10.17.5.1/32 lookup 175 priority 1005
  # 6: bridge with a veth port (stacked)
  mk 6 1500
  ip -n $H link add v17-br6 type bridge
  ip -n $H link set v17-h6 master v17-br6
  ip -n $H link set v17-br6 up
  ip -n $H addr add 10.17.6.1/24 dev v17-br6; ip -n v17-peer6 addr add 10.17.6.2/24 dev v17-p6
  ip -n $H route add 10.17.200.1/32 via 10.17.6.2 dev v17-br6 table 176
  ip -n $H rule add from 10.17.6.1/32 lookup 176 priority 1006
  # other interfaces present at boot: the design's own ADR-0061 pair (both ends in the root namespace), a dummy NIC
  ip link add v17-cli type veth peer name v17-bk
  ip link set v17-cli netns $H; ip link set v17-bk netns $H
  ip -n $H addr add 10.17.90.1/30 dev v17-cli; ip -n $H addr add 10.17.90.2/30 dev v17-bk
  ip -n $H link set v17-cli up; ip -n $H link set v17-bk up
  ip -n $H link add v17-dum0 type dummy; ip -n $H addr add 10.17.91.1/24 dev v17-dum0; ip -n $H link set v17-dum0 up
  set +e
  for n in $H v17-peer1 v17-peer2 v17-peer3 v17-peer4 v17-peer5 v17-peer6; do
    ip netns exec $n sysctl -qw net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.default.rp_filter=0
  done
  echo "--- topology evidence"
  echo "ip rule (v17-host):"; ip -n $H rule
  echo "main table (no route to 10.17.200.1):"; ip -n $H route
  for src in 10.17.1.1 10.17.2.1 10.17.4.1 10.17.5.1 10.17.6.1; do echo -n "route get 10.17.200.1 from $src: "; ip -n $H route get 10.17.200.1 from $src | head -1; done
  echo -n "route get 10.17.200.1 mark 0x173: "; ip -n $H route get 10.17.200.1 mark 0x173 | head -1
  echo "links:"; ip -n $H -br link
  phases="${*:-control design stacked}"
  ip netns exec $H "$here/out/v17" run "$here/out/v17-bpf.o" $(echo $phases | sed 's/\bcases\b//')
  if [[ " $phases " == *" cases "* ]]; then
    for c in ${V17_CASES:-R1 R2 R3 R4 R5 R6 R7}; do
      echo; echo "=== CASE $c"
      ip netns exec $H "$here/out/v17" case $c "$here/out/v17-bpf.o"
    done
  fi
  echo "== exit status: $?"
} 2>&1 | tee "$log"
echo "log: $log"
