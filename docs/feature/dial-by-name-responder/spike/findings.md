# Spike findings — dial-by-name-responder, Slice 00 (BLOCKING)

## Assumption under test

**Can ONE host-side (root-netns) in-agent listener receive and answer DNS
queries sent to N DIFFERENT per-netns gateway addresses, on a real kernel?**

- **Predicted:** WORKS — each per-netns gateway addr is the host-side veth
  peer's address (in the root netns), so a single wildcard listener receives
  queries to all of them.
- **Falsification:** queries from inside netns B never reach the listener
  (routing/binding needs per-netns sockets) → PIVOT before the walking
  skeleton.

## Binary verdict

**WORKS.**

A single in-agent process bound to `0.0.0.0:53` (ONE wildcard socket) received
and correctly answered DNS queries sent to TWO different per-netns gateway
addresses (`10.99.0.1`, `10.99.0.5`), validated through BOTH the explicit-`@gw`
`dig` path AND the real `getaddrinfo`/`getent` stub-resolver path, from BOTH
netns. Replies were source-pinned to the queried gateway via `IP_PKTINFO`, which
is what made `getent` (which rejects a reply from the wrong server) succeed.

## Kernel

```
$ uname -r
7.0.0-22-generic
```

This is the **dev Lima VM kernel**, NOT the pinned 6.18 appliance kernel
(ADR-0068). The verdict is pinned to `7.0.0-22-generic`. The mechanisms
exercised (`IP_PKTINFO` recv/send, multi-homed UDP, per-netns `resolv.conf`
bind-mount, `SO_REUSEADDR` wildcard coexistence with systemd-resolved) are all
long-stable kernel surfaces present well before 6.18, so the verdict is
expected to hold on the appliance kernel — but it is not separately confirmed
there in this probe.

## Topology (mirrors `veth_provisioner::derive_workload_netns_plan`)

```
slot 0: netns ovd-ns-0000; host veth ovd-hv-0000 @ 10.99.0.1/30 (root netns);
        workload veth ovd-wl-0000 @ 10.99.0.2/30 (in netns);
        default via 10.99.0.1; /etc/netns/ovd-ns-0000/resolv.conf = nameserver 10.99.0.1
slot 1: netns ovd-ns-0001; host veth ovd-hv-0001 @ 10.99.0.5/30 (root netns);
        workload veth ovd-wl-0001 @ 10.99.0.6/30 (in netns);
        default via 10.99.0.5; /etc/netns/ovd-ns-0001/resolv.conf = nameserver 10.99.0.5
```

These addresses are exactly what `derive_workload_netns_plan` computes for
slots 0 and 1 against `WORKLOAD_SUBNET_BASE = 10.99.0.0/16` (`base + slot*4`,
host = first usable, workload = second usable; responder = host/gateway).

## Pre-run environment (the port-53 contention question)

```
$ ss -ulnp | grep ':53 '
UNCONN 0 0       127.0.0.54:53   0.0.0.0:*  users:(("systemd-resolve",pid=803,fd=21))
UNCONN 0 0    127.0.0.53%lo:53   0.0.0.0:*  users:(("systemd-resolve",pid=803,fd=19))
$ systemctl is-active systemd-resolved
active
```

systemd-resolved owns `127.0.0.53:53` and `127.0.0.54:53` — but BOTH are
**specific-address** binds, NOT wildcard. This is the crux for the bind
strategy: a `SO_REUSEADDR` wildcard `0.0.0.0:53` bind does NOT collide with a
specific-address bind on the same port, so the clean one-socket shape is
available here.

## Evidence — the run (pasted, not narrated)

### Bind shape (the D-TME-11 sub-finding)

```
BIND_SHAPE wildcard 0.0.0.0:53 — SUCCESS (one socket serves all gateways)
RESPONDER READY (1 socket(s))
```

The clean shape worked. `ss` during the run confirms our wildcard binder
coexisting with systemd-resolved's two specific binds:

```
$ ss -ulnp | grep ':53 '
UNCONN 0 0          0.0.0.0:53   0.0.0.0:*  users:(("responder",pid=1298514,fd=3))
UNCONN 0 0       127.0.0.54:53   0.0.0.0:*  users:(("systemd-resolve",pid=803,fd=21))
UNCONN 0 0    127.0.0.53%lo:53   0.0.0.0:*  users:(("systemd-resolve",pid=803,fd=19))
```

**Sub-finding: 1 WILDCARD socket, NOT N per-addr sockets.** The per-gateway-addr
fallback path (the dnsmasq/Cilium-with-resolved shape) was NOT needed on this
kernel + this resolved configuration. The fallback is implemented in the probe
and would fire on `EADDRINUSE`, but did not.

### dig + getent, BOTH netns

```
########## DIG slot0 (explicit @10.99.0.1) ##########
10.1.2.3
dig0 rc=0
########## GETENT slot0 (getaddrinfo via injected resolv.conf) ##########
10.1.2.3        STREAM test.svc.overdrive.local
10.1.2.3        DGRAM
10.1.2.3        RAW
getent0 rc=0
########## DIG slot1 (explicit @10.99.0.5) ##########
10.1.2.3
dig1 rc=0
########## GETENT slot1 (getaddrinfo via injected resolv.conf) ##########
10.1.2.3        STREAM test.svc.overdrive.local
10.1.2.3        DGRAM
10.1.2.3        RAW
getent1 rc=0
```

All four resolutions return `10.1.2.3` with rc=0. The `getent` path is the real
signal: an unmodified workload's `getaddrinfo()` reads the per-netns
`resolv.conf` (`nameserver <gateway>`), queries `<gateway>:53`, and ACCEPTS the
reply — which it only does because the reply source was pinned to `<gateway>`.

### AAAA → NODATA (the v1 DNS contract)

```
########## DIG slot0 AAAA (expect NOERROR + ANSWER: 0 = NODATA) ##########
;; ->>HEADER<<- opcode: QUERY, status: NOERROR, id: 8723
;; flags: qr aa rd; QUERY: 1, ANSWER: 0, AUTHORITY: 0, ADDITIONAL: 0
```

AAAA is answered as `NOERROR` with `ANSWER: 0` — NODATA, not NXDOMAIN. This is
the pinned v1 DNS contract (a name that has an A record but no AAAA must return
NODATA so the stub resolver does not fall through to a slow negative path or
treat the name as nonexistent).

### Source-pinning proof (responder log)

```
QUERY  via=0.0.0.0:53 dst_gateway=10.99.0.1 src=10.99.0.2:60655 (65 bytes) -> reply 58 bytes
QUERY  via=0.0.0.0:53 dst_gateway=10.99.0.1 src=10.99.0.2:39618 (42 bytes) -> reply 58 bytes
QUERY  via=0.0.0.0:53 dst_gateway=10.99.0.5 src=10.99.0.6:53663 (65 bytes) -> reply 58 bytes
QUERY  via=0.0.0.0:53 dst_gateway=10.99.0.5 src=10.99.0.6:45747 (42 bytes) -> reply 58 bytes
QUERY  via=0.0.0.0:53 dst_gateway=10.99.0.1 src=10.99.0.2:42941 (65 bytes) -> reply 42 bytes
```

ONE socket (`via=0.0.0.0:53`) demultiplexed queries from BOTH netns. The
`dst_gateway` column is the address captured from the `IP_PKTINFO` control
message — `10.99.0.1` for queries from netns 0 (`src=10.99.0.2`) and
`10.99.0.5` for queries from netns 1 (`src=10.99.0.6`). The reply was sent back
with `ipi_spec_dst = dst_gateway`, pinning the reply source to the exact
address the workload queried.

### Clean teardown (leak discipline)

```
########## POST-TEARDOWN CHECK ##########
(no ovd netns remain)
(no stray non-resolved :53 binder)
ALL DONE
```

## The working bind / routing shape

- **ONE process, ONE `0.0.0.0:53` UDP socket**, `SO_REUSEADDR` + `IP_PKTINFO`.
- Recv: `recvmsg` with the `Ipv4PacketInfo` control message → read `ipi_addr`
  (the dst gateway the query targeted).
- Send: `sendmsg` with a `ControlMessage::Ipv4PacketInfo` whose `ipi_spec_dst`
  = the captured gateway → reply source is pinned to that gateway.
- Routing: the host `net.ipv4.ip_forward=1` is set (the root netns routes
  between the per-alloc /30s); each netns has a `default via <gateway>` so the
  workload's `:53` query egresses the veth and ingresses the host-side end in
  the root netns, where the wildcard listener receives it.

## Edge cases / design implications for D-TME-11

1. **systemd-resolved:53 coexistence is a non-issue on the wildcard path
   (this kernel + this resolved config).** resolved binds `127.0.0.53:53` /
   `127.0.0.54:53` as **specific** addresses; an `SO_REUSEADDR` `0.0.0.0:53`
   wildcard bind coexists with them. The responder receives queries to the
   `10.99.0.x` gateway addresses (which resolved does not bind) without
   contention. **Caveat for the appliance image:** if the production node image
   ever has a process holding a `0.0.0.0:53` *wildcard* bind, the clean shape
   would `EADDRINUSE` and the design must fall back to N per-gateway-addr
   sockets in one process. The probe implements that fallback; it was not
   exercised here. The walking skeleton should keep the
   "try-wildcard-then-fall-back-to-per-addr" shape so the responder is correct
   on either node configuration, OR confirm the appliance image has no wildcard
   `:53` holder and commit to the wildcard-only shape. **Recommendation: keep
   the fallback** — it is a few lines and removes a node-image coupling.

2. **`IP_PKTINFO` source-pinning is MANDATORY, not optional.** A multi-homed
   UDP socket replying without `ipi_spec_dst` lets the kernel choose the reply
   source from the route, which can differ from the queried gateway. The stub
   resolver (`getaddrinfo`/glibc) rejects a reply whose source != the queried
   server, so the `getent` path would FAIL silently (timeout → resolution
   failure) without the pin. The `dig +short @gw` path is more lenient and can
   mask this — so the walking skeleton's acceptance test MUST assert the
   **`getent`/`getaddrinfo`** path, not just `dig @gw`, or it will pass a
   broken responder.

3. **The responder IS the per-netns gateway (D-TME-12 G1 confirmed).** The
   address the workload queries (`plan.responder_addr` = `plan.host_addr` =
   `plan.gateway`) is the host-side veth-end address in the root netns. It is
   reachable by construction (the in-netns default route), collision-free (the
   slot's own /30 host address), and a single root-netns wildcard listener sees
   queries to all of them. No per-netns listener / no netns-entering is needed.

4. **`ip_forward=1` is a prerequisite for the in-netns→root-netns path.** Set
   it in the converge-on-boot pass (already modeled as
   `WorkloadVethStep::EnableIpForward`). Without it the gateway-addressed query
   from inside the netns is not routed to the root-netns listener.

5. **Negative-response shape for the walking skeleton.** AAAA-for-an-A-only-name
   returns NODATA (`NOERROR`, 0 answers) — proven here. A name with NO records
   at all should return `NXDOMAIN` with a negative TTL (SOA in authority) so the
   stub resolver caches the negative answer; that NXDOMAIN/SOA shape is NOT
   exercised by this probe (only the NODATA case is) and is a walking-skeleton
   concern, not a spike blocker.

## Gate recommendation

**PROMOTE.** The blocking assumption holds decisively on a real kernel through
the real `getaddrinfo` path from both netns, served by a single root-netns
wildcard listener with `IP_PKTINFO` source-pinning. The clean one-wildcard-
socket shape works; carry the per-addr fallback into the walking skeleton as
cheap insurance against a node image that holds a wildcard `:53`. No PIVOT
required.
