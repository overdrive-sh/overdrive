#!/usr/bin/env bash
# Inside a private net+mount namespace. The responder (server side) sees resolv.conf = scripted upstream;
# every client runs in a nested mount namespace seeing resolv.conf = the node gateway 100.95.0.1.
set -uo pipefail
out="$1"; here="$2"; v6="${3:-v4}"
ip link set lo up
ip addr add 100.95.0.1/32 dev lo
if [ "$v6" = v6 ]; then
  # A non-loopback interface with a global IPv6 address, so AI_ADDRCONFIG (getent's getaddrinfo) permits AAAA.
  ip link add d0 type dummy; ip link set d0 up
  ip addr add 10.255.0.1/24 dev d0; ip -6 addr add 2001:db8:1::2/64 dev d0 nodad
  ip -6 route add default via 2001:db8:1::1 dev d0 metric 1024 2>/dev/null
fi
echo "address family mode: $v6"
ip -br addr
printf 'nameserver 127.0.0.2\n' > "$out/server.resolv.conf"
printf 'nameserver 100.95.0.1\n' > "$out/client.resolv.conf"
printf 'nameserver 100.95.0.1\noptions use-vc\n' > "$out/client-usevc.resolv.conf"
mount --bind "$out/server.resolv.conf" /etc/resolv.conf
echo "server-side /etc/resolv.conf: $(cat /etc/resolv.conf)"

client() { # client <resolv-file> cmd...
  local rc="$1"; shift
  unshare --mount --propagation private bash -c 'mount --bind "$1" /etc/resolv.conf; shift; exec "$@"' _ "$rc" "$@"
}
timed() {
  local s e rc; s=$(date +%s.%N); "$@"; rc=$?; e=$(date +%s.%N)
  printf '   [exit=%s wall=%.3fs]\n' "$rc" "$(echo "$e - $s" | bc -l)"
}
cap() { # cap <label> <cmd...>: tcpdump around one client run
  local label="$1"; shift
  tcpdump -i lo -nn -tt -U -w "$out/$label.pcap" 'port 53' >/dev/null 2>&1 & local tp=$!
  sleep 0.7
  echo "--- CASE $label: $*"
  "$@"
  sleep 0.5; kill $tp; wait $tp 2>/dev/null
  echo "--- wire ($label), port 53 (client<->responder and responder<->upstream), -vv:"
  tcpdump -nn -tt -vv -r "$out/$label.pcap" 2>/dev/null | sed 's/^/    /'
}
phase() {
  local mode="$1"
  rm -f "$out/ready" "$out/stop"
  "$out/harness" "$out" "$mode" 600 > "$out/harness-$mode.log" 2>&1 &
  hp=$!
  for i in $(seq 1 150); do [ -f "$out/ready" ] && break; sleep 0.2; done
  [ -f "$out/ready" ] || { echo "harness never ready"; cat "$out/harness-$mode.log"; kill $hp; return 1; }
  echo "== PHASE $mode: $(cat "$out/harness-$mode.log")"
  ss -lntup | grep ':53 ' | sed 's/^/    listen: /'
}
endphase() { # $1 = mode
  touch "$out/stop"; wait "$hp" 2>/dev/null
  cp "$out/upstream-seen.txt" "$out/upstream-seen-$1.txt"
  echo "== upstream log ($1):"; sed 's/^/    /' "$out/upstream-seen-$1.txt"; echo
}

N=pipe-test.example.com
####### Phase 1: upstream answers A, empty NOERROR for AAAA, UDP not truncated
phase scripted || exit 1
for rep in 1 2 3; do
  echo "######## rep $rep"
  cap udp-ahosts-$rep      timed client "$out/client.resolv.conf" getent ahosts $N
  cap usevc-ahosts-$rep    timed client "$out/client.resolv.conf" env RES_OPTIONS=use-vc getent ahosts $N
  cap usevc-ahostsv4-$rep  timed client "$out/client.resolv.conf" env RES_OPTIONS=use-vc getent ahostsv4 $N
  cap usevc-ahostsv6-$rep  timed client "$out/client.resolv.conf" env RES_OPTIONS=use-vc getent ahostsv6 $N
done
echo "######## strace of glibc under use-vc, AF_UNSPEC"
timed client "$out/client.resolv.conf" env RES_OPTIONS=use-vc strace -f -ttt -o "$out/strace-usevc-ahosts.txt" -e trace=network,read,write,close,poll,ppoll,writev,recvfrom,sendto getent ahosts $N
grep -v "ENOENT" "$out/strace-usevc-ahosts.txt" | sed 's/^/    /'
echo "######## 'options use-vc' in the resolv.conf file instead of RES_OPTIONS"
cap usevc-file-ahosts timed client "$out/client-usevc.resolv.conf" getent ahosts $N
echo "######## use-vc + single-request (sequential queries) control"
cap usevc-singlereq-ahosts timed client "$out/client.resolv.conf" env RES_OPTIONS="use-vc single-request" getent ahosts $N
endphase scripted

####### Phase 2: upstream truncates every UDP reply (TC); its TCP serves many queries per connection
phase tc || exit 1
for rep in 1 2 3; do
  echo "######## rep $rep (TC mode)"
  cap tc-glibc-ahosts-$rep  timed client "$out/client.resolv.conf" getent ahosts $N
  [ -x "$out/musl_lookup" ] && cap tc-musl-$rep timed client "$out/client.resolv.conf" "$out/musl_lookup" $N
done
if [ -x "$out/musl_lookup" ]; then
  echo "######## strace musl"
  client "$out/client.resolv.conf" strace -f -ttt -o "$out/strace-tc-musl.txt" -e trace=network,read,write,close,poll,ppoll,writev,recvfrom,sendto "$out/musl_lookup" $N
  sed 's/^/    /' "$out/strace-tc-musl.txt"
fi
endphase tc
echo "== done"
