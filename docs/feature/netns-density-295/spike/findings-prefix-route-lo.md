# V-PREFIX-ROUTE — non-local lo route for the guest prefix (Linux 7.0)

> **Kept as a spike record.** The mechanism this probe validated — a
> non-local `lo` route for the guest prefix (PREFIX-ROUTE-LO) — was withdrawn
> (DISCARD, [`wave-decisions.md`](wave-decisions.md), user ruling
> 2026-10-13). Only Case 6 is cited, as GW-LOCAL evidence (ADR-0152).

**Verdict: WORKS.** With `ip route add 100.95.0.0/16 dev lo scope link src 127.0.0.1 proto 247`:
- a host-local dial of the diverted tuple `100.95.0.7:8080` reaches the leg-C TPROXY listener;
- the listener sees `peer=127.0.0.1`, `local=100.95.0.7:8080`;
- the ADR-0172 reject counters stay at 0.

Without the route, `connect()` fails at once with `ENETUNREACH`. Removing the route restores that failure while the gateway local route keeps DNS answered.

**Caveat that shapes the design:** under this route, every other packet to the prefix is delivered locally on `lo` (cases 4a and 5). The ADR-0172 constant rules are what keeps those packets away from host sockets.

**Scope of the probe:** a Python IP_TRANSPARENT listener in a network namespace stands in for the production leg-C, and `ip`/`nft` commands mirror the production mark, table-100 and TPROXY rules (`crates/overdrive-dataplane/src/mtls/nft.rs`). The production leg-C end to end is R5-43 / V-19 (Tier 3).

## Environment

| Item | Value |
|---|---|
| Kernel (`uname -r` in Lima, run as root) | `7.0.0-38-generic` |
| Date | 2026-10-13 |
| iproute2 | 6.19.0 |
| nftables | v1.1.6 |
| Namespace | `ovd-prt-probe2`, deleted at the end |
| Scratch files | `/tmp/prt2`, removed at the end |

A routing probe needs no KVM, so it runs in Lima rather than on the metal host. Two earlier runs of the same script are not used for the verdict: run 1 failed to load the nft rules (syntax error); run 2 used a plain `reject` for TCP and its TCP clients timed out although the reject counters incremented (not investigated). Run 3 uses `reject with tcp reset` for TCP, as ADR-0172 says. Everything below is from run 3, unedited. A separate earlier probe in namespace `ovd-prt-probe` on the same kernel reached the same per-case results; its transcript was not retained.

## Setup

```
+ uname -r
7.0.0-38-generic
+ ip netns add ovd-prt-probe2
+ [ns] ip link set lo up
+ [ns] ip link add up0 type dummy
+ [ns] ip addr add 192.0.2.10/24 dev up0
+ [ns] ip link set up0 up
+ [ns] ip rule add fwmark 0x1 lookup 100
+ [ns] ip route add local 0.0.0.0/0 dev lo table 100
+ [ns] nft -f - (prt_mark)     [exit=0]
+ [ns] nft -f - (prt_const)    [exit=0]
+ [ns] nft list ruleset
table ip prt_mark {
	chain out {
		type route hook output priority mangle; policy accept;
		ip daddr 100.95.0.7 tcp dport 8080 meta mark != 0x00000002 meta mark set 0x00000001
	}

	chain pre {
		type filter hook prerouting priority mangle; policy accept;
		ip daddr 100.95.0.7 tcp dport 8080 tproxy to 127.0.0.1:15001 meta mark set 0x00000001 accept
	}
}
table ip prt_const {
	chain out {
		type filter hook output priority filter; policy accept;
		ct state established,related accept
		ip daddr 100.95.0.7 tcp dport 8080 accept
		ip daddr 100.95.0.1 accept
		ip daddr 100.95.0.0/16 meta l4proto tcp counter packets 0 bytes 0 reject with tcp reset
		ip daddr 100.95.0.0/16 counter packets 0 bytes 0 reject
	}

	chain pre {
		type filter hook prerouting priority filter; policy accept;
		ct state established,related accept
		ip daddr 100.95.0.7 tcp dport 8080 accept
		ip daddr 100.95.0.1 accept
		ip daddr 100.95.0.0/16 counter packets 0 bytes 0 drop
	}
}
+ [ns] ip rule show
0:	from all lookup local
32765:	from all fwmark 0x1 lookup 100
32766:	from all lookup main
32767:	from all lookup default
+ [ns] ip route show table all
local default dev lo table 100 scope host 
192.0.2.0/24 dev up0 proto kernel scope link src 192.0.2.10 
local 127.0.0.0/8 dev lo table local proto kernel scope host src 127.0.0.1 
local 127.0.0.1 dev lo table local proto kernel scope host src 127.0.0.1 
broadcast 127.255.255.255 dev lo table local proto kernel scope link src 127.0.0.1 
local 192.0.2.10 dev up0 table local proto kernel scope host src 192.0.2.10 
broadcast 192.0.2.255 dev up0 table local proto kernel scope link src 192.0.2.10 
fe80::/64 dev up0 proto kernel metric 256 pref medium
local ::1 dev lo table local proto kernel metric 0 pref medium
local fe80::d01f:abff:fe41:ab37 dev up0 table local proto kernel metric 0 pref medium
multicast ff00::/8 dev up0 table local proto kernel metric 256 pref medium
+ [ns] sysctl net.ipv4.conf.all.rp_filter net.ipv4.conf.lo.rp_filter net.ipv4.ip_forward net.ipv4.conf.all.route_localnet
net.ipv4.conf.all.rp_filter = 2
net.ipv4.conf.lo.rp_filter = 2
net.ipv4.ip_forward = 1
net.ipv4.conf.all.route_localnet = 0
+ cat listener.log
listener ready
```

## Case 1 — no candidate route

- Hypothesis: with no route for the prefix and no default route, a dial of a guest-prefix address has no route.
- Prediction: `ip route get` fails and `connect()` returns `ENETUNREACH` immediately.
- Falsification: any connection, timeout, or `ECONNREFUSED`.

```
+ [ns] ip route get 100.95.0.7
RTNETLINK answers: Network is unreachable
[exit=2]
+ [ns] python3 /tmp/prt2/tcpc.py 100.95.0.7 8080
ERR OSError 101 ENETUNREACH [Errno 101] Network is unreachable
elapsed=0.000s
```

Predicted vs actual: as predicted.

## Case 2 — with the route

- Hypothesis: the lo route gives the connect a route; the mark, table 100 and TPROXY hand the connection to leg-C; the non-diverted rules never match.
- Prediction: round trip; listener sees peer `127.0.0.1`, local `100.95.0.7:8080`; reject/drop counters 0.
- Falsification: timeout or refusal, a different peer/local pair, or a non-zero reject counter.

```
+ [ns] ip route add 100.95.0.0/16 dev lo scope link src 127.0.0.1 proto 247
+ [ns] ip route show
100.95.0.0/16 dev lo proto 247 scope link src 127.0.0.1 
192.0.2.0/24 dev up0 proto kernel scope link src 192.0.2.10 
+ [ns] ip route get 100.95.0.7
100.95.0.7 dev lo src 127.0.0.1 uid 0 
    cache <local> 
+ [ns] python3 /tmp/prt2/tcpc.py 100.95.0.7 8080
connected client-local=('127.0.0.1', 43178) client-peer=('100.95.0.7', 8080)
reply= b'RESP:REQ-100.95.0.7:8080'
elapsed=0.000s
+ cat listener.log
listener ready
ACCEPT peer=('127.0.0.1', 43178) local=('100.95.0.7', 8080) req=b'REQ-100.95.0.7:8080'
+ [ns] nft list table ip prt_const
        ...
		ip daddr 100.95.0.0/16 meta l4proto tcp counter packets 0 bytes 0 reject with tcp reset
		ip daddr 100.95.0.0/16 counter packets 0 bytes 0 reject
        ...
		ip daddr 100.95.0.0/16 counter packets 0 bytes 0 drop
```

Predicted vs actual: as predicted. `cache <local>`: the kernel treats the lo route as a local-delivery route.

## Case 3 — non-diverted traffic with the route, constant rules on

- Hypothesis: the output reject refuses every non-diverted guest-prefix packet at once.
- Prediction: TCP to `100.95.0.9:8080` and `100.95.0.7:9090` → `ECONNREFUSED`; UDP to `100.95.0.9:53` fails at once; no new accept.
- Falsification: a timeout, a connect succeeding, or a new `ACCEPT`.

```
+ [ns] python3 /tmp/prt2/tcpc.py 100.95.0.9 8080
ERR ConnectionRefusedError 111 ECONNREFUSED [Errno 111] Connection refused
elapsed=0.000s
+ [ns] python3 /tmp/prt2/tcpc.py 100.95.0.7 9090
ERR ConnectionRefusedError 111 ECONNREFUSED [Errno 111] Connection refused
elapsed=0.000s
+ [ns] python3 /tmp/prt2/udpc.py 100.95.0.9 53
udp connected client-local=('127.0.0.1', 38128)
ERR PermissionError 1 EPERM [Errno 1] Operation not permitted
elapsed=0.000s
+ [ns] nft list table ip prt_const
        ...
		ip daddr 100.95.0.0/16 meta l4proto tcp counter packets 2 bytes 120 reject with tcp reset
		ip daddr 100.95.0.0/16 counter packets 1 bytes 40 reject
        ...
		ip daddr 100.95.0.0/16 counter packets 0 bytes 0 drop
+ cat listener.log (no new ACCEPT expected)
listener ready
ACCEPT peer=('127.0.0.1', 43178) local=('100.95.0.7', 8080) req=b'REQ-100.95.0.7:8080'
```

Predicted vs actual: as predicted. UDP `send()` fails `EPERM` (rejected in the OUTPUT hook). The prerouting drop counter stays 0 because the output reject fires first.

## Case 4a — gateway DNS, route present, no gateway local route

- Hypothesis: the lo route alone delivers a gateway-bound datagram to a wildcard `:53` socket, with the wrong source.
- Prediction: responder receives the query from `127.0.0.1`; a connected client gets no usable reply.
- Falsification: no receive, or a reply accepted from `100.95.0.1`.

```
+ [ns] ip route get 100.95.0.1
100.95.0.1 dev lo src 127.0.0.1 uid 0 
    cache <local> 
+ [ns] python3 /tmp/prt2/udpc.py 100.95.0.1 53
udp connected client-local=('127.0.0.1', 56719)
sent
ERR TimeoutError None None timed out
elapsed=3.024s
+ cat udpresp.log
udp 0.0.0.0:53 ready
UDP-RECV from=('127.0.0.1', 56719) data=b'Q-100.95.0.1'
```

Predicted vs actual: as predicted.

## Case 4b — add the gateway local route (GW-LOCAL)

- Hypothesis: an RTN_LOCAL /32 makes the connect pick `100.95.0.1` as source; the reply source matches.
- Prediction: answered; `ip route get 100.95.0.1` reports `local ... src 100.95.0.1`.
- Falsification: timeout or another source.

```
+ [ns] ip route add local 100.95.0.1/32 dev lo table local
+ [ns] ip route get 100.95.0.1
local 100.95.0.1 dev lo table local src 100.95.0.1 uid 0 
    cache <local> 
+ [ns] python3 /tmp/prt2/udpc.py 100.95.0.1 53
udp connected client-local=('100.95.0.1', 60665)
sent
reply=b'ANS:Q-100.95.0.1' from=('100.95.0.1', 53)
elapsed=0.000s
+ cat udpresp.log
udp 0.0.0.0:53 ready
UDP-RECV from=('127.0.0.1', 56719) data=b'Q-100.95.0.1'
UDP-RECV from=('100.95.0.1', 60665) data=b'Q-100.95.0.1'
```

Predicted vs actual: as predicted.

## Case 5 — route present, constant rules removed

`nft delete table ip prt_const`; a UDP responder on `0.0.0.0:53` and a TCP server on `0.0.0.0:8080`.

- Hypothesis: non-diverted packets to the prefix are delivered locally and reach host sockets; nothing leaves the uplink.
- Prediction: UDP reaches `:53`; TCP SYNs reach `:8080` and fail listener-side; `up0` carries no IPv4.
- Falsification: no socket delivery, IPv4 on `up0`, or a rise in `IpInAddrErrors` / `IpInNoRoutes`.

```
+ [ns] nstat -az TcpExtListenDrops IpOutNoRoutes IpInDelivers IpInAddrErrors IpInNoRoutes
IpInAddrErrors                  0                  0.0
IpInDelivers                    17                 0.0
IpOutNoRoutes                   1                  0.0
TcpExtListenDrops               0                  0.0
+ [ns] ip -s link show up0
    TX:  bytes packets errors dropped carrier collsns           
           140       2      0       0       0       0 
+ [ns] ip route get 100.95.0.9
100.95.0.9 dev lo src 127.0.0.1 uid 0 
    cache <local> 
+ [ns] ip route get 127.0.0.1 from 100.95.0.9
RTNETLINK answers: Network is unreachable
+ [ns] python3 /tmp/prt2/udpc.py 100.95.0.9 53
udp connected client-local=('127.0.0.1', 38001)
sent
ERR TimeoutError None None timed out
elapsed=3.032s
+ [ns] python3 /tmp/prt2/tcpc.py 100.95.0.9 8080
ERR TimeoutError None None timed out
elapsed=2.688s
+ cat udpresp.log
UDP-RECV from=('127.0.0.1', 56719) data=b'Q-100.95.0.1'
UDP-RECV from=('100.95.0.1', 60665) data=b'Q-100.95.0.1'
UDP-RECV from=('127.0.0.1', 38001) data=b'Q-100.95.0.9'
+ cat tcpserver.log
tcp 0.0.0.0:8080 ready
+ [ns] nstat -az TcpExtListenDrops IpOutNoRoutes IpInDelivers IpInAddrErrors IpInNoRoutes
IpInAddrErrors                  0                  0.0
IpInDelivers                    23                 0.0
IpOutNoRoutes                   4                  0.0
TcpExtListenDrops               3                  0.0
+ [ns] ip -s link show up0
    TX:  bytes packets errors dropped carrier collsns           
           140       2      0       0       0       0 
+ cat lo.pcap.txt
13:16:42.593798 IP 127.0.0.1.38001 > 100.95.0.9.53: 20781 op6+% [14645a] [12334q] [11824n] [11833au] [|domain]
13:16:45.636117 IP 127.0.0.1.47610 > 100.95.0.9.8080: Flags [S], seq 1759869917, win 65495, options [mss 65495,sackOK,TS val 578078782 ecr 0,nop,wscale 10], length 0
13:16:46.337361 IP 127.0.0.1.47610 > 100.95.0.9.8080: Flags [S], seq 1759869917, win 65495, options [mss 65495,sackOK,TS val 578079799 ecr 0,nop,wscale 10], length 0
13:16:47.359205 IP 127.0.0.1.47610 > 100.95.0.9.8080: Flags [S], seq 1759869917, win 65495, options [mss 65495,sackOK,TS val 578080821 ecr 0,nop,wscale 10], length 0

+ cat lo.err
4 packets captured
8 packets received by filter
0 packets dropped by kernel
```

Predicted vs actual: as predicted. The UDP datagram to `100.95.0.9:53` reached a host wildcard socket. Three SYNs reached the host listener (`TcpExtListenDrops` 0 → 3); the handshake could not complete only because a SYN-ACK from the non-local source `100.95.0.9` has no route (`IpOutNoRoutes` 1 → 4). Nothing left the host.

Kernel source (`vendors/linux` at `v7.0`):

```
net/ipv4/route.c
2595: 	if (dev_out->flags & IFF_LOOPBACK)
2596: 		flags |= RTCF_LOCAL;

1666: 		rt->dst.output = ip_output;
1667: 		if (flags & RTCF_LOCAL)
1668: 			rt->dst.input = ip_local_deliver;

2766: 		if (!(fl4->flowi4_flags & FLOWI_FLAG_ANYSRC)) {
2767: 			/* It is equivalent to inet_addr_type(saddr) == RTN_LOCAL */
2768: 			if (!__ip_dev_find(net, fl4->saddr, false))
2769: 				goto out;
```

The SYN-ACK path calls `ip_route_output_flow` in `inet_csk_route_req` (`net/ipv4/inet_connection_sock.c:779-781`), counting the failure at `:791`; a failed `route_req` in `tcp_conn_request` (`net/ipv4/tcp_input.c:7715-7717`) goes to `drop_and_free` → `tcp_listendrop(sk)` (`:7812-7815`).

## Case 6 — rules restored, route removed, GW-LOCAL kept

- Hypothesis: removing the prefix route restores `ENETUNREACH` for every guest-prefix address except the gateway.
- Prediction: TCP/UDP to `.7`/`.9` → `ENETUNREACH`; gateway query answered.
- Falsification: anything else.

```
+ [ns] nft -f - (prt_const)     [exit=0]
+ [ns] ip route del 100.95.0.0/16 dev lo
+ [ns] ip route show
192.0.2.0/24 dev up0 proto kernel scope link src 192.0.2.10 
+ [ns] ip route show table local
local 100.95.0.1 dev lo scope host 
local 127.0.0.0/8 dev lo proto kernel scope host src 127.0.0.1 
local 127.0.0.1 dev lo proto kernel scope host src 127.0.0.1 
broadcast 127.255.255.255 dev lo proto kernel scope link src 127.0.0.1 
local 192.0.2.10 dev up0 proto kernel scope host src 192.0.2.10 
broadcast 192.0.2.255 dev up0 proto kernel scope link src 192.0.2.10 
+ [ns] python3 /tmp/prt2/tcpc.py 100.95.0.7 8080
ERR OSError 101 ENETUNREACH [Errno 101] Network is unreachable
+ [ns] python3 /tmp/prt2/tcpc.py 100.95.0.9 8080
ERR OSError 101 ENETUNREACH [Errno 101] Network is unreachable
+ [ns] python3 /tmp/prt2/udpc.py 100.95.0.9 53
ERR OSError 101 ENETUNREACH [Errno 101] Network is unreachable
+ [ns] python3 /tmp/prt2/udpc.py 100.95.0.1 53
udp connected client-local=('100.95.0.1', 52847)
sent
reply=b'ANS:Q-100.95.0.1' from=('100.95.0.1', 53)
elapsed=0.000s
```

Predicted vs actual: as predicted.

## Uplink capture (whole run)

```
+ cat up0.pcap.txt
13:16:40.897361 IP6 fe80::d01f:abff:fe41:ab37 > ff02::2: ICMP6, router solicitation, length 16
13:16:49.799187 IP6 fe80::d01f:abff:fe41:ab37 > ff02::2: ICMP6, router solicitation, length 16
```

No IPv4 packet reached the uplink in any case.

## Cleanup

```
ns del exit=0
+ ip netns list
(empty)
+ ls /tmp/prt2
ls: cannot access '/tmp/prt2': No such file or directory
+ ip rule (host)
0:	from all lookup local
32766:	from all lookup main
32767:	from all lookup default
```

## Design implications

1. The route alone serves the diverted flow: the dial gets a route at `connect()`, and the existing mark, table 100 and TPROXY hand it to leg-C, which sees the workload address as local and `127.0.0.1` as the peer. The node needs no default route.
2. The route makes every prefix address a local-delivery route for host-originated packets (`route.c:2595-2596`, `:1667-1668`): a non-diverted packet reaches any matching host socket.
3. The ADR-0172 constant rules are therefore the barrier that keeps non-diverted guest-prefix traffic from host sockets. With them, such packets are refused at once: TCP `ECONNREFUSED`, UDP `EPERM` at `send()`.
4. GW-LOCAL is required for a correct gateway source on the DNS path (cases 4a, 4b).
5. Case 5's TCP failure (SYN-ACK source validation, `route.c:2766-2769`) is incidental, not a guarantee.

**Recommendation:** PROMOTE the non-local lo route together with the ADR-0172 constant rules and GW-LOCAL.

## Probe script (as run)

```bash
set -u
NS=ovd-prt-probe2
D=/tmp/prt2
rm -rf $D; mkdir -p $D
ip netns del $NS 2>/dev/null
rx() { echo "+ [ns] $*"; ip netns exec $NS "$@" 2>&1; echo "[exit=$?]"; }

# helper python files written to $D:
#  listener.py : IP_TRANSPARENT (SOL_IP,19) listener 127.0.0.1:15001; prints
#                "ACCEPT peer=%s local=%s req=%r"; replies b"RESP:"+req
#  tcpserver.py: 0.0.0.0:8080 listener
#  udpresp.py  : UDP 0.0.0.0:53; prints "UDP-RECV from=%s data=%r"; replies b"ANS:"+data
#  tcpc.py     : connect(host,port) timeout 3s, send REQ-host:port, print local/peer/reply or ERR errno
#  udpc.py     : connected UDP socket, send b"Q-"+host, recvfrom, print reply or ERR errno

MARK='table ip prt_mark {
  chain out {
    type route hook output priority mangle; policy accept;
    ip daddr 100.95.0.7 tcp dport 8080 meta mark != 0x2 meta mark set 0x1
  }
  chain pre {
    type filter hook prerouting priority mangle; policy accept;
    ip daddr 100.95.0.7 tcp dport 8080 tproxy to 127.0.0.1:15001 meta mark set 0x1 accept
  }
}'
CONST='table ip prt_const {
  chain out {
    type filter hook output priority filter; policy accept;
    ct state established,related accept
    ip daddr 100.95.0.7 tcp dport 8080 accept
    ip daddr 100.95.0.1 accept
    ip daddr 100.95.0.0/16 meta l4proto tcp counter reject with tcp reset
    ip daddr 100.95.0.0/16 counter reject with icmp type port-unreachable
  }
  chain pre {
    type filter hook prerouting priority filter; policy accept;
    ct state established,related accept
    ip daddr 100.95.0.7 tcp dport 8080 accept
    ip daddr 100.95.0.1 accept
    ip daddr 100.95.0.0/16 counter drop
  }
}'
# Cases 1-6 run the commands shown in each section above, in that order.
```
