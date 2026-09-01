#!/usr/bin/env bash
# PROBE increment-n — guest-in-microVM egress transparently intercepted by the
# production-shape nft-TPROXY on the host-side veth ingress.
#
# Topology (routed /30, closest to production):
#
#   [ VM guest ] eth0 10.77.0.2/30
#        | virtio-net  (NO host struct sock for this connection)
#   [ tap "tapw" ] 10.77.0.1/30   ---.
#        |                            |  netns "probens" (ip_forward=1)
#   [ veth "wveth0" ] 10.66.0.2/30 --'   default route via 10.66.0.1
#        | veth pair
#   [ veth "hveth0" ] 10.66.0.1/30       HOST netns
#        |  <-- nft: iifname "hveth0" meta l4proto tcp
#        |          tproxy to 127.0.0.1:PORT meta mark set 0x1
#        v
#   [ IP_TRANSPARENT listener 127.0.0.1:PORT ]  (ip rule fwmark 0x1 -> table 100 local)
#
# Guest dials 10.99.0.1:9000 (NOT present on the wire) -> intercepted -> byte
# distinct REQUEST/RESPONSE round-trips back into the guest.
#
# Toggles (env), so each empirical hypothesis is recorded with-vs-without:
#   RP_FILTER=0|1   (default 0)  rp_filter on host+netns forwarding ifaces
#   TX_OFF=1|0      (default 1)  ethtool -K <if> tx off rx off on tap/veths
#
# Run as root on the metal box:  sudo bash run.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-n
RUN=/run/spike-increment-n
NETNS=probens
PORT=15000
PEER=10.99.0.1
PEER_PORT=9000
HOST_VETH=hveth0
NS_VETH=wveth0
TAP=tapw
GUEST_MAC=12:34:56:78:9a:bc

RP_FILTER="${RP_FILTER:-0}"
TX_OFF="${TX_OFF:-1}"

teardown() {
  pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null
  pkill -9 -f "host-listener $PORT" 2>/dev/null
  pkill -9 -f "tcpdump -" 2>/dev/null
  ip netns del "$NETNS" 2>/dev/null
  ip link del "$HOST_VETH" 2>/dev/null           # removes the veth pair
  nft delete table ip overdrive_probe 2>/dev/null
  while ip rule del fwmark 0x1 lookup 100 2>/dev/null; do :; done
  ip route flush table 100 2>/dev/null
  sleep 0.2
}

echo "##########################################################"
echo "### PROBE increment-n  guest-tap-tproxy"
echo "### uname -r         : $(uname -r)"
echo "### uname -m         : $(uname -m)"
echo "### cloud-hypervisor : $(cloud-hypervisor --version 2>&1)"
echo "### nft version      : $(nft --version 2>&1)"
echo "### toggles          : RP_FILTER=$RP_FILTER  TX_OFF=$TX_OFF"
echo "##########################################################"
echo

echo "=== [0] teardown any prior state"
teardown
trap teardown EXIT

echo "=== [1] build (gcc static + rootfs)"
bash "$HERE/build.sh" || { echo "BUILD FAILED"; exit 1; }
rm -rf "$RUN"; mkdir -p "$RUN"
cp "$OUT/rootfs.ext4" "$RUN/rootfs.ext4"

echo
echo "=== [2] load nft_tproxy module (TPROXY is a MODULE on this kernel)"
modprobe nft_tproxy && echo "  modprobe nft_tproxy OK" || { echo "  modprobe nft_tproxy FAILED"; exit 1; }
lsmod | grep -E '^nft_tproxy' | sed 's/^/    /'

echo
echo "=== [3] netns + veth pair + tap"
ip netns add "$NETNS"
ip link add "$HOST_VETH" type veth peer name "$NS_VETH"
ip link set "$NS_VETH" netns "$NETNS"
# host veth end
ip addr add 10.66.0.1/30 dev "$HOST_VETH"
ip link set "$HOST_VETH" up
# netns veth end
ip netns exec "$NETNS" ip addr add 10.66.0.2/30 dev "$NS_VETH"
ip netns exec "$NETNS" ip link set "$NS_VETH" up
ip netns exec "$NETNS" ip link set lo up
# tap (guest NIC) in the netns
ip netns exec "$NETNS" ip tuntap add dev "$TAP" mode tap
ip netns exec "$NETNS" ip addr add 10.77.0.1/30 dev "$TAP"
ip netns exec "$NETNS" ip link set "$TAP" up
# netns forwarding: guest /30 is connected via tapw; everything else via hveth0
ip netns exec "$NETNS" sysctl -qw net.ipv4.ip_forward=1
ip netns exec "$NETNS" ip route add default via 10.66.0.1
# host: return route to the guest /30 so replies (and rp_filter) resolve
ip route add 10.77.0.0/30 via 10.66.0.2 dev "$HOST_VETH"

echo "  --- host $HOST_VETH:"; ip -br addr show "$HOST_VETH" | sed 's/^/    /'
echo "  --- netns links:"; ip netns exec "$NETNS" ip -br addr show | sed 's/^/    /'
echo "  --- netns routes:"; ip netns exec "$NETNS" ip route | sed 's/^/    /'
echo "  --- host route to guest:"; ip route get 10.77.0.2 | sed 's/^/    /'

echo
echo "=== [4] rp_filter=$RP_FILTER + tx-offload toggle (TX_OFF=$TX_OFF)"
sysctl -qw "net.ipv4.conf.all.rp_filter=$RP_FILTER"
sysctl -qw "net.ipv4.conf.$HOST_VETH.rp_filter=$RP_FILTER"
ip netns exec "$NETNS" sysctl -qw "net.ipv4.conf.all.rp_filter=$RP_FILTER"
ip netns exec "$NETNS" sysctl -qw "net.ipv4.conf.$TAP.rp_filter=$RP_FILTER"
ip netns exec "$NETNS" sysctl -qw "net.ipv4.conf.$NS_VETH.rp_filter=$RP_FILTER"
if [ "$TX_OFF" = "1" ]; then
  for spec in "$NETNS:$TAP" "$NETNS:$NS_VETH" ":$HOST_VETH"; do
    ns="${spec%%:*}"; ifn="${spec##*:}"
    if [ -n "$ns" ]; then pfx=(ip netns exec "$ns"); else pfx=(); fi
    "${pfx[@]}" ethtool -K "$ifn" tx off rx off 2>/dev/null \
      && echo "    tx/rx off on ${ns:-host}:$ifn" \
      || echo "    (ethtool -K ${ns:-host}:$ifn tx/rx off not supported)"
  done
fi

echo
echo "=== [5] production-shape nft-TPROXY on $HOST_VETH ingress"
nft add table ip overdrive_probe
nft add chain ip overdrive_probe prerouting '{ type filter hook prerouting priority mangle; policy accept; }'
nft add rule  ip overdrive_probe prerouting iifname "$HOST_VETH" meta l4proto tcp tproxy to 127.0.0.1:$PORT meta mark set 0x1
ip rule add fwmark 0x1 lookup 100
ip route add local 0.0.0.0/0 dev lo table 100
echo "  --- ruleset:"; nft list table ip overdrive_probe | sed 's/^/    /'
echo "  --- ip rule:"; ip rule show | grep -E 'fwmark|lookup 100' | sed 's/^/    /'
echo "  --- table 100:"; ip route show table 100 | sed 's/^/    /'

echo
echo "=== [6] host IP_TRANSPARENT listener on 127.0.0.1:$PORT (HOST netns)"
"$OUT/host-listener" "$PORT" >"$RUN/host-listener.log" 2>&1 &
LPID=$!
for _ in $(seq 1 50); do ss -ltnp 2>/dev/null | grep -q ":$PORT " && break; sleep 0.05; done
ss -ltnp 2>/dev/null | grep ":$PORT " | sed 's/^/    /' || echo "    (listener socket not visible yet)"

echo
echo "=== [7] tcpdump on tapw (netns) AND hveth0 (host) — population diff"
# -l line-buffers stdout so a graceful SIGINT (below) flushes captured lines;
# a SIGKILL would lose tcpdump's buffer (first-run gap).
ip netns exec "$NETNS" timeout 40 tcpdump -l -tt -ni "$TAP"  'tcp' >"$RUN/tcpdump-tapw.log"  2>&1 &
TD_TAP=$!
timeout 40 tcpdump -l -tt -ni "$HOST_VETH" 'tcp' >"$RUN/tcpdump-hveth0.log" 2>&1 &
TD_HV=$!
sleep 1

echo
echo "=== [8] boot Cloud-Hypervisor in netns, guest NIC = tap $TAP"
CMDLINE="root=/dev/vda rw console=ttyS0 init=/init panic=1 loglevel=7"
CH_ARGV=(
  cloud-hypervisor
  --cpus boot=1
  --memory size=512M
  --kernel "$OUT/kernel"
  --cmdline "$CMDLINE"
  --disk "path=$RUN/rootfs.ext4"
  --net "tap=$TAP,mac=$GUEST_MAC"
  --serial "file=$RUN/console.log"
  --console off
)
echo "  --- exact argv:"; printf '    ip netns exec %s' "$NETNS"; printf ' %q' "${CH_ARGV[@]}"; echo
ip netns exec "$NETNS" "${CH_ARGV[@]}" >"$RUN/ch-stderr.log" 2>&1 &
CH_PID=$!
( sleep 45; kill -9 $CH_PID 2>/dev/null ) & WD=$!
wait $CH_PID; CH_RC=$?
kill $WD 2>/dev/null; wait $WD 2>/dev/null

# let listener + tcpdump drain
for _ in $(seq 1 40); do kill -0 $LPID 2>/dev/null || break; sleep 0.1; done
kill -9 $LPID 2>/dev/null; wait $LPID 2>/dev/null; LRC=$?
sleep 1
# Graceful SIGINT so tcpdump prints its "N packets captured" summary and
# flushes the line-buffered capture BEFORE the EXIT-trap SIGKILL sweep.
kill -INT $TD_TAP $TD_HV 2>/dev/null; sleep 1.5
pkill -9 -f "tcpdump -l -tt" 2>/dev/null; sleep 0.3

echo
echo "=================== GUEST SERIAL CONSOLE ==================="
cat "$RUN/console.log" 2>/dev/null || echo "<no console>"
echo "=================== END SERIAL CONSOLE ===================="
echo
echo "=================== HOST IP_TRANSPARENT LISTENER ==========="
cat "$RUN/host-listener.log" 2>/dev/null || echo "<no listener output>"
echo "=================== END LISTENER =========================="
echo
echo "=================== tcpdump hveth0 (host ingress) =========="
cat "$RUN/tcpdump-hveth0.log" 2>/dev/null | grep -vE 'listening on|verbose output' || true
echo "=================== tcpdump tapw (netns) =================="
cat "$RUN/tcpdump-tapw.log" 2>/dev/null | grep -vE 'listening on|verbose output' || true
echo "=========================================================="
echo
echo "--- cloud-hypervisor stderr (tail):"
tail -5 "$RUN/ch-stderr.log" 2>/dev/null | sed 's/^/    /'
echo "--- CH exit rc=$CH_RC  listener rc=$LRC"

# ---- VERDICT ----
echo
if grep -q "ROUND-TRIP SUCCESS" "$RUN/console.log" 2>/dev/null \
   && grep -q "ORIG-DST(getsockname)=$PEER:$PEER_PORT" "$RUN/host-listener.log" 2>/dev/null; then
  echo "############## VERDICT: WORKS (RP_FILTER=$RP_FILTER TX_OFF=$TX_OFF) ##############"
else
  echo "############## VERDICT: DOES-NOT-WORK (this config) — see evidence above ##############"
fi
