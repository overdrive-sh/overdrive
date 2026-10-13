#!/usr/bin/env bash
# V-LEGB-MARK S1..S7 harness. Run as root in the Lima VM. Self-cleaning (netns, procs, nft live in the ns).
set -u
BIN=${BIN:-/home/marcus.guest/.cargo-target-v33/release}
D=$(cd "$(dirname "$0")" && pwd)
OUT=${OUT:-$D/out}; mkdir -p "$OUT"
NS=v33ns
VM=100.95.0.7; GW=100.95.0.1; GP=100.95.0.0/16
nsx() { ip netns exec $NS "$@"; }
t() { echo "+ [ns] $*"; nsx "$@"; local rc=$?; echo "[exit=$rc]"; return 0; }
hdr() { echo; echo "=== $1"; echo "Hypothesis:  $2"; echo "Prediction:  $3"; echo "Falsification: $4"; }

teardown() {
  for p in $(pgrep -f "$BIN/v33 (listen|wild-tcp|wild-udp)" 2>/dev/null); do kill "$p" 2>/dev/null; done
  for p in $(pgrep -f "tcpdump -i (up0|lo) -nn -l -w $OUT" 2>/dev/null); do kill "$p" 2>/dev/null; done
  ip netns del $NS 2>/dev/null
  return 0
}
trap teardown EXIT

base() {   # ns, lo, uplink dummy
  teardown; rm -f "$OUT"/*.log "$OUT"/*.pcap
  ip netns add $NS
  nsx ip link set lo up
  nsx ip link add up0 type dummy
  nsx ip addr add 192.0.2.10/24 dev up0
  nsx ip link set up0 up
  nsx ip neigh replace 192.0.2.1 lladdr 02:00:00:00:00:01 dev up0 nud permanent
  # Pin the appliance-faithful value: production never sets route_localnet (grep of crates/ xtask/ infra/ finds
  # none), so it is the kernel default 0. A new netns INHERITS init_net's values (devconf_inherit_init_net=0), and
  # the metal host's root netns carries all.route_localnet=1 from earlier work, which would mask K10.
  nsx sysctl -qw net.ipv4.conf.all.route_localnet=0 net.ipv4.conf.default.route_localnet=0 net.ipv4.conf.lo.route_localnet=0
  echo "+ [ns] sysctl (netns v33ns; uname -r=$(uname -r) $(uname -m))"
  nsx sysctl net.ipv4.conf.all.route_localnet net.ipv4.conf.lo.route_localnet net.ipv4.conf.all.rp_filter net.ipv4.conf.lo.rp_filter net.ipv4.ip_forward
}
default_on()  { t ip route add default via 192.0.2.1 dev up0; }
default_off() { t ip route del default via 192.0.2.1 dev up0; }

prod_rules() {   # production ip rule + table-100 route (real netlink client), gateway /32, nft shaped as the design end-state
  if [ "${RULES:-prod}" = cli ]; then
    t ip rule add fwmark 0x1 lookup 100
    t ip route add local 0.0.0.0/0 dev lo table 100
  else
    echo "+ [ns] v33-rules   (overdrive-netlink Client::add_fib_rule_fwmark(0x1,100) + add_local_route(100,\"lo\"))"
    nsx "$BIN/v33-rules"
  fi
  t ip route add local $GW/32 dev lo proto 247
  nsx nft -f - <<NFT
table ip overdrive-mtls {
  set inbound_destinations { type ipv4_addr . inet_service; }
  chain prerouting {
    type filter hook prerouting priority -150; policy accept;
    meta mark 0x2 accept
    meta l4proto tcp ip daddr . tcp dport @inbound_destinations tproxy to 127.0.0.1:15001 meta mark set 0x1 accept
    ip daddr $GP ip daddr != $GW counter drop
  }
  chain output {
    type route hook output priority -150; policy accept;
    meta mark 0x2 accept
    meta l4proto tcp ip daddr . tcp dport @inbound_destinations meta mark != 0x2 meta mark set 0x1 accept
    ip daddr $GP ip daddr != $GW meta l4proto tcp counter reject with tcp reset
    ip daddr $GP ip daddr != $GW counter reject
  }
}
NFT
  echo "[nft load exit=$?]"
  t nft add element ip overdrive-mtls inbound_destinations "{ $VM . 8080 }"
}
show_state() { t ip rule show; t ip route show table all; t nft list ruleset; }
reset_counters() { nsx nft reset rules table ip overdrive-mtls >/dev/null 2>&1; }
counters() { echo "--- reject/drop counters:"; nsx nft list table ip overdrive-mtls | grep -E "counter packets" | sed 's/^\s*/  /'; }

leg_c() {  ip netns exec $NS "$BIN/v33" listen --port 15001 --idle-ms 60000 >> "$OUT/legc.log" 2>&1 & LEGC=$!
           for i in $(seq 60); do grep -q "ready" "$OUT/legc.log" && break; sleep 0.05; done; }
wild()  {  ip netns exec $NS "$BIN/v33" wild-tcp --port 8080 --idle-ms 60000 >> "$OUT/wtcp.log" 2>&1 & WT=$!
           ip netns exec $NS "$BIN/v33" wild-udp --port 53   --idle-ms 60000 >> "$OUT/wudp.log" 2>&1 & WU=$!
           for i in $(seq 60); do grep -q ready "$OUT/wtcp.log" && grep -q ready "$OUT/wudp.log" && break; sleep 0.05; done; }
stop_listeners() { kill ${LEGC:-} ${WT:-} ${WU:-} 2>/dev/null; wait 2>/dev/null; LEGC=; WT=; WU=; }
logs() { echo "--- leg-C listener log:"; sed 's/^/  /' "$OUT/legc.log" 2>/dev/null; echo "--- host decoy logs:"; cat "$OUT/wtcp.log" "$OUT/wudp.log" 2>/dev/null | sed 's/^/  /'; : > "$OUT/legc.log"; : > "$OUT/wtcp.log"; : > "$OUT/wudp.log"; }
dial() { echo "+ [ns] v33 dial $*"; nsx "$BIN/v33" dial "$@"; }
cap_start() { # cap_start <iface> <file> <filter>
  ip netns exec $NS tcpdump -i "$1" -nn --immediate-mode -U -w "$OUT/$2" $3 > "$OUT/$2.log" 2>&1 & CAP=$!
  for i in $(seq 60); do grep -q listening "$OUT/$2.log" && break; sleep 0.05; done; }
cap_stop() { kill -INT $CAP 2>/dev/null; wait $CAP 2>/dev/null
  echo "--- capture $1 (IPv4 packets seen):"; nsx tcpdump -nn -r "$OUT/$1" 2>/dev/null | sed 's/^/  /'
  echo "  count=$(nsx tcpdump -nn -r "$OUT/$1" 2>/dev/null | wc -l)"; }

phase_S1() {
  hdr "S1: marked+bound dial, NO default route, NO prefix route" \
      "leg-B mark=0x1 + bind(127.0.0.1) gives connect() a route via fwmark->table 100; divert/TPROXY hand it to leg-C" \
      "round trip; leg-C sees peer=127.0.0.1:<eph> local=$VM:8080; counters 0; 'ip route get $VM mark 1 from 127.0.0.1' = local dev lo" \
      "ENETUNREACH/EINVAL, other peer/local pair, non-zero reject counter"
  base; prod_rules; show_state
  t ip route get $VM mark 1 from 127.0.0.1
  t ip route get $VM
  leg_c; reset_counters
  cap_start lo lo_s1.pcap "host $VM"
  dial --ip $VM --port 8080 --mark 1 --bind 127.0.0.1
  dial --ip $VM --port 8080 --leg-b
  cap_stop lo_s1.pcap
  logs; counters; stop_listeners
}

phase_S2() {
  hdr "S2: same, default route via up0 present" \
      "mark selects table 100 ahead of main's default; nothing is sent on the uplink" \
      "same round trip; zero IPv4 packets on up0 for the marked dials" \
      "any IPv4 on up0; route via uplink"
  base; default_on; prod_rules
  t ip route show
  t ip route get $VM mark 1 from 127.0.0.1
  t ip route get $VM
  leg_c; reset_counters
  cap_start up0 up0_s2.pcap "ip"
  dial --ip $VM --port 8080 --mark 1 --bind 127.0.0.1
  dial --ip $VM --port 8080 --leg-b
  cap_stop up0_s2.pcap
  logs; counters; stop_listeners
}

phase_S3() {
  hdr "S3: marked+bound dial to prefix tuples NOT in the divert set (default route absent, decoys present)" \
      "constant output reject refuses them; no host socket gets them" \
      "TCP ECONNREFUSED (reject with tcp reset); UDP send/recv fails (EPERM/ECONNREFUSED); wild :8080/:53 decoys log nothing; counters >0" \
      "delivery to a decoy, or timeout"
  base; prod_rules; leg_c; wild; reset_counters
  dial --ip $VM --port 9999 --mark 1 --bind 127.0.0.1
  dial --ip 100.95.0.99 --port 8080 --mark 1 --bind 127.0.0.1
  dial --ip $VM --port 8080 --udp --mark 1 --bind 127.0.0.1
  dial --ip 100.95.0.99 --port 53 --udp --mark 1 --bind 127.0.0.1
  dial --ip $GW --port 53 --udp --mark 1 --bind 127.0.0.1
  logs; counters; stop_listeners
}

phase_S4() {
  hdr "S4: controls (which half of mark+bind is load-bearing)" \
      "unmarked: no route (ENETUNREACH) or, with a default route, no host-socket delivery; mark-only gives src=dst; bind-only fails" \
      "(a) ENETUNREACH; (b) not-in-set: ECONNREFUSED, in-set: reaches leg-C via today's divert from the uplink source; (c) mark-only: src=$VM, reply into prefix refused => no round trip; (d) bind-only: ENETUNREACH w/o default, EINVAL with default" \
      "any unmarked connect delivering to a host decoy socket"
  base; prod_rules; leg_c; wild; reset_counters
  echo "##### (a) unmarked, no default route"
  dial --ip $VM --port 8080
  dial --ip 100.95.0.99 --port 8080
  echo "##### (d1) bound only (127.0.0.1), no default route"
  dial --ip $VM --port 8080 --bind 127.0.0.1
  echo "##### (c) marked only (unbound), no default route"
  t ip route get $VM mark 1
  cap_start lo lo_s4c.pcap "host $VM"
  dial --ip $VM --port 8080 --mark 1
  cap_stop lo_s4c.pcap
  logs; counters
  echo "##### (c') marked only, table-100 route carrying 'src 127.0.0.1' (K8 / open question 2)"
  t ip route replace local 0.0.0.0/0 dev lo table 100 src 127.0.0.1
  t ip route show table 100
  t ip route get $VM mark 1
  reset_counters
  dial --ip $VM --port 8080 --mark 1
  logs; counters
  t ip route replace local 0.0.0.0/0 dev lo table 100
  t ip route show table 100
  echo "##### with a default route present"
  default_on
  echo "##### (b1) unmarked, tuple NOT in set (decoys present)"
  reset_counters
  dial --ip 100.95.0.99 --port 8080
  dial --ip 100.95.0.99 --port 53 --udp
  echo "##### (b2) unmarked, tuple in set (today's divert path, test hosts with a default route)"
  dial --ip $VM --port 8080
  echo "##### (d2) bound only (127.0.0.1)"
  dial --ip $VM --port 8080 --bind 127.0.0.1
  dial --ip 100.95.0.99 --port 8080 --bind 127.0.0.1
  logs; counters; stop_listeners
}

phase_S5() {
  hdr "S5: non-prefix dial through the leg-B code path (--leg-b) stays unmarked and uses the normal route" \
      "destination outside 100.95.0.0/16: no mark, no bind; SYN leaves via the uplink default route" \
      "SYN to 198.51.100.7 and 192.0.2.50 seen on up0 with src 192.0.2.10; v33 reports setup=[] (no mark, no bind); ETIMEDOUT (nobody answers on the dummy)" \
      "setup shows mark/bind, or the packet lands on lo"
  base; default_on; prod_rules; leg_c
  t ip route get 198.51.100.7
  cap_start up0 up0_s5.pcap "ip"
  dial --leg-b --ip 198.51.100.7 --port 8080 --timeout-ms 300
  dial --leg-b --ip 192.0.2.50 --port 8080 --timeout-ms 300
  cap_stop up0_s5.pcap
  echo "##### same dial with NO default route: off-link non-prefix dial keeps failing exactly as before (no dependency introduced)"
  default_off
  dial --leg-b --ip 198.51.100.7 --port 8080 --timeout-ms 300
  logs; stop_listeners
}

phase_S6() {
  hdr "S6: fallback A2 - SO_BINDTODEVICE=lo (+ bind 127.0.0.1), no mark; with and without default route" \
      "on-link fallback (route.c:2814-2843) builds a lo route; the divert re-routes via table 100 (oif lo matches nhc_oif lo)" \
      "connect succeeds, leg-C sees peer=127.0.0.1 local=$VM:8080, both with and without default route; zero IPv4 on up0" \
      "ENETUNREACH or a packet on up0"
  base; prod_rules; leg_c; reset_counters
  echo "##### no default route"
  dial --ip $VM --port 8080 --dev lo --bind 127.0.0.1
  dial --ip $VM --port 8080 --dev lo
  default_on
  echo "##### default route present"
  cap_start up0 up0_s6.pcap "ip"
  dial --ip $VM --port 8080 --dev lo --bind 127.0.0.1
  dial --ip $VM --port 8080 --dev lo
  echo "##### A2 not-in-set tuple (decoys): still refused?"
  wild
  dial --ip 100.95.0.99 --port 8080 --dev lo --bind 127.0.0.1
  cap_stop up0_s6.pcap
  logs; counters; stop_listeners
}

phase_S6x() {
  hdr "S6x: why does SO_BINDTODEVICE=lo WITHOUT bind() time out?" \
      "the on-link fallback picks the source via inet_select_addr; the SYN leaves lo with an unexpected source and the handshake dies" \
      "capture on lo shows the SYN source; if it is 127.0.0.1 the loss is after the divert" \
      "SYN source is 127.0.0.1 and the connect still fails for a reason not visible on lo"
  base; prod_rules; leg_c; reset_counters
  t ip route get $VM oif lo
  cap_start lo lo_s6x.pcap "host $VM"
  dial --ip $VM --port 8080 --dev lo --timeout-ms 600
  cap_stop lo_s6x.pcap
  logs; counters; stop_listeners
}

phase_S7() {
  hdr "S7: production ip rule order (installed by the production netlink client, no priority argument)" \
      "kernel picks pref = (second rule's pref) - 1 = 32765, ahead of main 32766" \
      "ip rule show: 0 local, 32765 fwmark 0x1 lookup 100, 32766 main, 32767 default; mark-1 lookup beats main's default route" \
      "main (or a default route) wins for mark 1"
  base; default_on; prod_rules
  t ip rule show
  t ip route get $VM mark 1 from 127.0.0.1
  t ip route get $VM mark 0
  t ip route get 198.51.100.7 mark 1
  t ip route get 198.51.100.7 mark 0
  echo "##### idempotence of the production guard (dump-then-add) - second run must not stack a rule"
  nsx "$BIN/v33-rules" 2>&1 | tail -3
  t ip rule show
}

phase_Q3() {
  hdr "Q3: does the pre-marked connect take the same route as the diverted packet?" \
      "both packets are delivered over lo through table 100 to leg-C; they differ only in source (127.0.0.1 vs uplink 192.0.2.10)" \
      "lo capture: SYN 127.0.0.1.x > $VM.8080 (pre-marked) and SYN 192.0.2.10.y > $VM.8080 (unmarked via default route + divert); leg-C accepts both" \
      "pre-marked SYN appears on up0 or is not on lo"
  base; default_on; prod_rules; leg_c
  t ip route get $VM mark 1 from 127.0.0.1
  t ip route get $VM mark 1 from 192.0.2.10
  cap_start lo lo_q3.pcap "host $VM"
  dial --ip $VM --port 8080 --mark 1 --bind 127.0.0.1
  dial --ip $VM --port 8080
  cap_stop lo_q3.pcap
  logs; stop_listeners
}

phase_S8() {
  hdr "S8-equivalent: marked+bound leg-B dial -> TPROXY leg-C stand-in -> vsock -> REAL microVM, node with NO default route" \
      "the dial reaches the guest app inside the real microVM with no default route and no prefix route anywhere" \
      "client gets FROM-MICROVM reply; leg-C logs peer=127.0.0.1 local=$VM:8080; ip route shows no default/prefix route" \
      "ENETUNREACH, a refusal, or any route covering $GP"
  base; prod_rules
  t ip route show
  t ip route show table all
  t ip rule show
  ip netns exec $NS "$BIN/v33" vm-legc --port 15001 --cid "$VM_CID" --vport 5000 --idle-ms 60000 >> "$OUT/legc.log" 2>&1 & LEGC=$!
  for i in $(seq 60); do grep -q "ready" "$OUT/legc.log" && break; sleep 0.05; done
  wild; reset_counters
  echo "##### (1) leg-B dial (proposed contract) to the VM tuple"
  dial --leg-b --ip $VM --port 8080 --timeout-ms 3000
  echo "##### (2) unmarked dial to the VM tuple (control)"
  dial --ip $VM --port 8080
  echo "##### (3) leg-B dial to a prefix tuple not in the set (control; decoys present)"
  dial --leg-b --ip $VM --port 9999
  logs; counters; stop_listeners
}

for p in "$@"; do "phase_$p"; done
