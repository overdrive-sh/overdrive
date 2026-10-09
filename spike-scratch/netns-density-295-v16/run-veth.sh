#!/usr/bin/env bash
# V-16 veth matrix (throwaway). Run as root on the Linux host. Everything created is prefixed v16- ; cleanup on exit.
# usage: run-veth.sh <outdir> <variant>...   variants: uso-on uso-off uso-on-legacy uso-on-unreg
# Sender side v16-a stays in the ROOT namespace (the design attaches in the root netns); receiver in netns v16-rx.
set -u
OUT=${1:?outdir}; shift
BIN=/tmp/v16-out/v16; OBJ=${V16_OBJ:-/tmp/v16-out/v16-bpf.o}
mkdir -p "$OUT"
cleanup() { ip link del v16-a 2>/dev/null; ip netns del v16-rx 2>/dev/null; }
trap cleanup EXIT
cleanup
echo "== V-16 veth matrix $(date -u +%FT%TZ)"
echo "uname -r: $(uname -r)  uname -m: $(uname -m)  virt: $(systemd-detect-virt)"
ip netns add v16-rx
ip link add v16-a type veth peer name v16-b
ip link set v16-b netns v16-rx
ip addr add 10.216.16.1/30 dev v16-a; ip link set v16-a up
ip -n v16-rx addr add 10.216.16.2/30 dev v16-b; ip -n v16-rx link set v16-b up; ip -n v16-rx link set lo up
echo "-- ethtool -i v16-a"; ethtool -i v16-a | head -3
for v in "$@"; do
  # reset checksum offloads to defaults each variant, then apply the variant
  ethtool -K v16-a tx on rx on >/dev/null 2>&1; ip netns exec v16-rx ethtool -K v16-b tx on rx on >/dev/null 2>&1
  case $v in
    csum-rx-validate) ip netns exec v16-rx ethtool -K v16-b rx off >/dev/null 2>&1;;
    csum-sw-tx) ethtool -K v16-a tx off >/dev/null 2>&1; ip netns exec v16-rx ethtool -K v16-b rx off >/dev/null 2>&1;;
  esac
  echo "-- v16-b rx-checksumming: $(ip netns exec v16-rx ethtool -k v16-b | grep '^rx-checksumming')  v16-a tx-checksumming: $(ethtool -k v16-a | grep '^tx-checksumming')"
  case $v in
    uso-on*) ethtool -K v16-a tx-udp-segmentation on >/dev/null 2>&1; ip netns exec v16-rx ethtool -K v16-b tx-udp-segmentation on >/dev/null 2>&1;;
    uso-off*) ethtool -K v16-a tx-udp-segmentation off >/dev/null 2>&1; ip netns exec v16-rx ethtool -K v16-b tx-udp-segmentation off >/dev/null 2>&1;;
  esac
  echo; echo "######## variant $v ########"
  echo "-- ethtool -k v16-a (segmentation/gso/gro/checksum/sg lines)"
  ethtool -k v16-a | egrep "udp|tcp-segm|generic-seg|scatter-gather:|tx-checksumming|gso|rx-gro-list|generic-receive"
  echo "-- mtu: $(cat /sys/class/net/v16-a/mtu)"
  flags=""
  case $v in *legacy) flags="--legacy";; *unreg) flags="--unregistered";; esac
  rxlog="$OUT/rx-$v.log"; : > "$rxlog"
  ip netns exec v16-rx "$BIN" rx 10.216.16.2 ${V16_SET:-all} 6 > "$rxlog" 2>&1 &
  rxpid=$!
  sleep 1
  "$BIN" tx "$OBJ" v16-a 10.216.16.1 10.216.16.2 ${V16_SET:-all} $flags
  wait $rxpid
  cat "$rxlog"
  # attachments must be gone once tx exits
  echo "-- tcx/tc attachments on v16-a after tx exit:"; bpftool net show dev v16-a 2>&1 | head -5
  tc filter show dev v16-a egress 2>&1 | head -3
  tc qdisc del dev v16-a clsact 2>/dev/null
done
echo "== done; cleaning"
