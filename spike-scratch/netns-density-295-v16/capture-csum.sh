#!/usr/bin/env bash
# V-16: what checksum is on the wire for the program-edited datagrams (receiver ns capture, software tx checksum, vv).
set -u
cleanup() { ip link del v16-a 2>/dev/null; ip netns del v16-rx 2>/dev/null; }
trap cleanup EXIT; cleanup
ip netns add v16-rx; ip link add v16-a type veth peer name v16-b; ip link set v16-b netns v16-rx
ip addr add 10.216.16.1/30 dev v16-a; ip link set v16-a up
ip -n v16-rx addr add 10.216.16.2/30 dev v16-b; ip -n v16-rx link set v16-b up; ip -n v16-rx link set lo up
ip netns exec v16-rx ethtool -K v16-b rx off >/dev/null; ethtool -K v16-a tx off >/dev/null
BIN=/tmp/v16-out/v16; OBJ=/tmp/v16-out/v16-bpf.o
ip netns exec v16-rx tcpdump -i v16-b -nn -vv -l -c 40 'udp and (dst port 42000 or dst port 42009 or dst port 42031 or dst port 42034 or dst port 42037)' > "$1" 2>&1 &
TP=$!
ip netns exec v16-rx "$BIN" rx 10.216.16.2 plain 6 > /dev/null 2>&1 &
sleep 1.5
"$BIN" tx "$OBJ" v16-a 10.216.16.1 10.216.16.2 plain --rounds 1 | grep -E "ATTACH|REGISTERED"
sleep 1; kill $TP 2>/dev/null; wait 2>/dev/null
