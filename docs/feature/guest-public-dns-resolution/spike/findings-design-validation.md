# Findings: design validation V-2, V-3, V-5 (guest-public-dns-resolution, #382)

Status: PRE-REGISTERED. The Hypothesis / Prediction / Falsification blocks below were written before any run and are not edited afterwards. Results are appended in later sections.

Probe code: `spike-scratch/guest-public-dns-increment-b/` (increment-a, `spike-scratch/guest-public-dns-probe/`, is preserved unchanged). Nothing under `crates/` is touched.

## V-2 — `ipi_spec_dst` distinguishes gateway-addressed queries (GPD-1, ADR-0356)

Kernel reading (v7.0, `vendors/linux`, `Makefile` VERSION=7 PATCHLEVEL=0 SUBLEVEL=0, commit 028ef9c96):

- `net/ipv4/udp.c:2499` — `udp_queue_rcv_one_skb` calls `ipv4_pktinfo_prepare(sk, skb, true)` for each datagram queued to a UDP socket.
- `net/ipv4/ip_sockglue.c:1376-1408` — `ipv4_pktinfo_prepare`: when the socket has `IP_PKTINFO` set and the skb has an rtable, it sets `pktinfo->ipi_spec_dst.s_addr = fib_compute_spec_dst(skb)` (line 1400); otherwise ifindex and spec_dst are zero.
- `net/ipv4/fib_frontend.c:272-298` — `fib_compute_spec_dst`: `if ((rt->rt_flags & (RTCF_BROADCAST | RTCF_MULTICAST | RTCF_LOCAL)) == RTCF_LOCAL) return ip_hdr(skb)->daddr;` (lines 283-285). For a unicast datagram delivered locally the spec_dst IS the IP destination; the fib/`inet_select_addr` branch is only reached for non-local, broadcast or multicast input routes.
- `net/ipv4/ip_sockglue.c:54-60` — `ip_cmsg_recv_pktinfo` copies that cached info and ALSO sets `info.ipi_addr.s_addr = ip_hdr(skb)->daddr`.

What the source predicts: for every locally delivered unicast datagram, `ipi_spec_dst == ipi_addr == the datagram's destination address`. A datagram to 100.95.0.1 reports 100.95.0.1; a datagram to the host's primary address reports that address; to 127.0.0.1 reports 127.0.0.1. (Note `ipi_addr` carries the same information as `ipi_spec_dst` in the local-unicast case.)

Hypothesis: a datagram the host owner sends on a socket connected to gateway:53 arrives at a wildcard-bound `IP_PKTINFO` socket with `ipi_spec_dst == 100.95.0.1`; a datagram addressed to another node address arrives with `ipi_spec_dst` equal to that address.

Prediction: on metal, after `serve` has installed the production routes, a spare-port wildcard socket sees spec_dst = destination for every case: gateway-connected -> 100.95.0.1, host-primary-connected -> host primary, 127.0.0.1-connected -> 127.0.0.1, regardless of the sender's source address. A real guest query observed on a second wildcard `:53` socket (SO_REUSEADDR) shows spec_dst == 100.95.0.1.

Falsification: spec_dst equals the gateway for a datagram addressed to a non-gateway node address, or differs from the gateway for a gateway-addressed one.

## V-3 — AAAA pass-through is harmless to an IPv4-only guest (GPD-6, ADR-0361)

Hypothesis: in a guest with no IPv6 address or route, `getaddrinfo(AF_UNSPEC)` for a dual-stack name returns A and AAAA records, and sequential connect over the returned list connects with no stall, because an IPv6 connect fails immediately (ENETUNREACH) rather than timing out.

Prediction: for each of `www.google.com`, `cloudflare.com`, `example.com`, resolution returns both families; connect over the list in resolver order succeeds on an IPv4 address; total list-connect time exceeds the first-IPv4-alone baseline by under 100 ms; an IPv6 connect alone fails in under 10 ms with a network-unreachable error. Same for musl (static) and glibc (dynamic) if the guest runs the glibc build. `github.com` (no AAAA) is the A-only control.

Falsification: any name/library shows an extra delay of 1 s or more over the A-only baseline, or the lookup itself stalls 1 s or more waiting for an AAAA answer.

Deviation from production (stated): no forwarder exists yet, so INSIDE THE GUEST ONLY `/etc/resolv.conf` is rewritten at runtime to `nameserver 1.1.1.1`, a real upstream returning A and AAAA unchanged. That delivers exactly what pass-through would. The path guest -> upstream is ADR-0162 UDP egress, proven in increment-a (probe C). The added latency of the node forwarding hop is not measured here (that is V-4, tuning).

## V-5 — guest TCP/53 reaches a gateway TCP listener (GPD-5, ADR-0360)

Hypothesis: with a TCP listener bound to 100.95.0.1:53 in the host namespace, a guest TCP connect to 100.95.0.1:53 reaches it (the owner admits it per ADR-0167 and connects through ADR-0154/0162), and a length-framed DNS query gets a framed reply back.

Prediction: before any listener exists, the guest's connect (or its first write/read) fails visibly (refused / reset / EOF, not a silent hang; this is today's truncation-fallback behaviour). After the harness binds the listener, the guest completes a framed query/response and sees the byte-distinct litmus `GPD-V5-REPLY` in a TXT answer with the query id echoed. The host listener logs the peer.

Falsification: after the listener is bound, the guest connect is refused / reset / times out on every attempt, or the framed reply does not arrive.

Deviation from production (stated): the production responder has no TCP listener; the harness itself binds one after `serve` boots as the spike stand-in for the proposed GPD-5 listener. It answers a synthetic TXT, not a forwarded answer.

---

# Results (appended after the run; the blocks above are unchanged)

## Run record

- Command: `cargo xtask metal run --exclusive -- spike-scratch/guest-public-dns-increment-b/run.sh run6`
- Run id: `eefc250a5950857ddf5fe526` (`OVERDRIVE_METAL_RUN_COMPLETE ... exit_code=0`, `harness exit=0`).
- Host: `uname -r` = `7.0.0-38-generic`; source SHA `4f1afd298808d5c405529626e67a01ac7b0fd911`, dirty only with this spike's untracked files.
- Captures: `spike-scratch/guest-public-dns-increment-b/out/run6/{run.log,evidence.log}` (pulled with `cargo xtask metal pull`; the directory is gitignored build output, so the decisive lines are pasted below).
- Earlier attempts, kept for the record: run1 and run2 failed in setup (kernel module and BPF objects not yet built; fixed by `kmod-build`, `bpf-build`, `guest-kit-prepare`); run3 and run4 ended `maintenance_timeout` while another worktree's test run held the box; run5 lost its ssh route before START (`run_not_started`). None of them produced probe output.
- The production composition was booted with `serve` and one VM Service deployed with `deploy`, as in increment-a. Nothing was hand-installed on the guest path except the stand-ins named under each item.

## V-2 — HOLDS

Host-side probe (wildcard UDP socket with `IP_PKTINFO` on a spare port, after `serve` installed the production routes):

```
dst=100.95.0.1 -> src=100.95.0.1:57725 spec_dst=100.95.0.1 ipi_addr(hdr daddr)=100.95.0.1 ifindex=1 spec_dst==gateway:true
dst=151.115.99.251 -> src=151.115.99.251:43444 spec_dst=151.115.99.251 ipi_addr(hdr daddr)=151.115.99.251 ifindex=2 spec_dst==gateway:false
dst=127.0.0.1 -> src=127.0.0.1:57666 spec_dst=127.0.0.1 ipi_addr(hdr daddr)=127.0.0.1 ifindex=1 spec_dst==gateway:false
dst=100.95.0.1 -> src=151.115.99.251:47588 spec_dst=100.95.0.1 ipi_addr(hdr daddr)=100.95.0.1 ifindex=1 spec_dst==gateway:true
dst=100.95.0.1 -> src=100.95.0.1:57584 spec_dst=100.95.0.1 ipi_addr(hdr daddr)=100.95.0.1 ifindex=1 spec_dst==gateway:true
dst=151.115.99.251 -> src=151.115.99.251:33991 spec_dst=151.115.99.251 ipi_addr(hdr daddr)=151.115.99.251 ifindex=2 spec_dst==gateway:false
```

Real guest queries, observed on a second wildcard `:53` socket:

```
src=100.95.0.1:60673 spec_dst=100.95.0.1 ipi_addr(hdr daddr)=100.95.0.1 ifindex=1 len=29
src=100.95.0.1:33390 spec_dst=100.95.0.1 ipi_addr(hdr daddr)=100.95.0.1 ifindex=1 len=29
src=100.95.0.1:50246 spec_dst=100.95.0.1 ipi_addr(hdr daddr)=100.95.0.1 ifindex=1 len=29
```

The source prediction holds: `ipi_spec_dst` is the datagram's destination for every locally delivered unicast datagram. A gateway-addressed datagram reports `100.95.0.1` even when sent from the host's primary address; a datagram to any other node address reports that address. A real guest query (ADR-0154 path) arrives with `spec_dst == 100.95.0.1`.

Deviation: the observer socket on `:53` received the guest's queries in place of the production responder, so in this run the guest's mesh-name check got no reply (`D mesh-name via gateway ... NO-REPLY`, three tries). That is the observer's doing, not a finding about the responder; increment-a showed the responder answering the same query in 7 ms.

Design implication: GPD-1 (ADR-0356) can select gateway-addressed queries by `ipi_spec_dst` on the existing wildcard socket. No change to ADR-0116 binding is needed.

Gate: PROMOTE GPD-1 as designed.

## V-3 — HOLDS (one sub-prediction not measured)

Guest IPv6 state:

```
probe: if_inet6 = "fe80000000000000d480d6fffe41e48e 03 40 20 80     ovd0\n00000000000000000000000000000001 01 80 10 80       lo\n"
probe: ipv6 disable sysctl = "0\n"
```

The guest has only link-local `fe80::/64` on `ovd0` and `::1` on `lo`; no global IPv6 address and no IPv6 default route.

Round 1 per name and library (rounds 2 and 3 in the capture show the same shape):

```
probe: V3 musl-static www.google.com round=1 list=[142.251.154.119:443, 142.251.153.119:443, 142.251.152.119:443, 142.251.156.119:443, 142.251.157.119:443, 142.251.151.119:443, 142.251.150.119:443, 142.251.155.119:443, [2001:4860:4826:7700::]:443, [2001:4860:482a:7700::]:443, [2001:4860:4829:7700::]:443, [2001:4860:482b:7700::]:443, [2001:4860:4828:7700::]:443, [2001:4860:482d:7700::]:443, [2001:4860:4827:7700::]:443, [2001:4860:482c:7700::]:443] resolve_ms=7.35 baseline_firstv4=Some(Ok(())) baseline_ms=0.10 seq_total_ms=0.06 connected=Some(142.251.154.119:443) trace=["142.251.154.119:443 OK 0.04ms"]
probe: V3 musl-static cloudflare.com round=1 list=[104.16.132.229:443, 104.16.133.229:443, [2606:4700::6810:84e5]:443, [2606:4700::6810:85e5]:443] resolve_ms=6.25 baseline_firstv4=Some(Ok(())) baseline_ms=0.07 seq_total_ms=0.04 connected=Some(104.16.132.229:443) trace=["104.16.132.229:443 OK 0.03ms"]
probe: V3 musl-static example.com round=1 list=[104.20.23.154:443, 172.66.147.243:443, [2606:4700:10::6814:179a]:443, [2606:4700:10::ac42:93f3]:443] resolve_ms=7.66 baseline_firstv4=Some(Ok(())) baseline_ms=0.06 seq_total_ms=0.04 connected=Some(104.20.23.154:443) trace=["104.20.23.154:443 OK 0.03ms"]
probe: V3 musl-static github.com round=1 list=[140.82.121.4:443] resolve_ms=6.30 baseline_firstv4=Some(Ok(())) baseline_ms=0.06 seq_total_ms=0.04 connected=Some(140.82.121.4:443) trace=["140.82.121.4:443 OK 0.03ms"]
probe: V3 glibc-dynamic www.google.com round=1 list=[142.251.150.119:443, 142.251.151.119:443, 142.251.155.119:443, 142.251.156.119:443, 142.251.154.119:443, 142.251.157.119:443, 142.251.153.119:443, 142.251.152.119:443, [2001:4860:4829:7700::]:443, [2001:4860:482c:7700::]:443, [2001:4860:482b:7700::]:443, [2001:4860:482d:7700::]:443, [2001:4860:4826:7700::]:443, [2001:4860:4828:7700::]:443, [2001:4860:4827:7700::]:443, [2001:4860:482a:7700::]:443] resolve_ms=6.26 baseline_firstv4=Some(Ok(())) baseline_ms=0.06 seq_total_ms=0.04 connected=Some(142.251.150.119:443) trace=["142.251.150.119:443 OK 0.03ms"]
probe: V3 glibc-dynamic cloudflare.com round=1 list=[104.16.132.229:443, 104.16.133.229:443, [2606:4700::6810:84e5]:443, [2606:4700::6810:85e5]:443] resolve_ms=6.50 baseline_firstv4=Some(Ok(())) baseline_ms=0.05 seq_total_ms=0.04 connected=Some(104.16.132.229:443) trace=["104.16.132.229:443 OK 0.03ms"]
probe: V3 glibc-dynamic example.com round=1 list=[104.20.23.154:443, 172.66.147.243:443, [2606:4700:10::6814:179a]:443, [2606:4700:10::ac42:93f3]:443] resolve_ms=9.95 baseline_firstv4=Some(Ok(())) baseline_ms=0.06 seq_total_ms=0.04 connected=Some(104.20.23.154:443) trace=["104.20.23.154:443 OK 0.03ms"]
probe: V3 glibc-dynamic github.com round=1 list=[140.82.121.3:443] resolve_ms=7.04 baseline_firstv4=Some(Ok(())) baseline_ms=0.06 seq_total_ms=0.04 connected=Some(140.82.121.3:443) trace=["140.82.121.3:443 OK 0.03ms"]
```

Both musl (static) and glibc (dynamic) return the IPv4 addresses before the IPv6 ones for every dual-stack name. Resolution takes 6–10 ms. Sequential connect succeeds on the first IPv4 address every round, with no extra delay over the first-IPv4 baseline. The A-only control (`github.com`) behaves the same.

Not measured: "an IPv6 connect alone fails in under 10 ms". No connect ever reached an IPv6 address, because both libraries order IPv4 first. Not re-run: the decision does not depend on it, since neither library tries IPv6 first in this guest.

Caveat: each connect completed in about 0.03 ms, far below an internet round trip. That shows the guest reports the connection established on its own side (the guest adaptation), so these lines do not prove reaching the remote; increment-a probe E is the evidence for that. The stall question is answered regardless: no address-list or lookup stall occurred.

Deviation: the guest's `/etc/resolv.conf` was rewritten at runtime to `nameserver 1.1.1.1`, a real upstream returning A and AAAA unchanged, as a stand-in for forwarder pass-through.

Design implication: GPD-6 (ADR-0361) option A, pass AAAA through unchanged, holds. NODATA for AAAA is not needed.

Gate: PROMOTE GPD-6 option A.

## V-5 — HOLDS

```
probe: V5 nolistener connect OK elapsed_ms=3.61
probe: V5 nolistener write=Ok(()) read=Err(Os { code: 104, kind: ConnectionReset, message: "Connection reset by peer" }) elapsed_ms=3.88
probe: V5 attempt=1 connect OK elapsed_ms=0.15
probe: V5 attempt=1 FRAMED REPLY len=54 id_echo=true litmus_GPD-V5-REPLY=true hex=[51, 51, 81, 80, 00, 01, 00, 01, 00, 00, 00, 00, 07, 65, 78, 61, 6d, 70, 6c, 65, 03, 63, 6f, 6d, 00, 00, 01, 00, 01, c0, 0c, 00, 10, 00, 01, 00, 00, 00, 00, 00, 0d, 0c, 47, 50, 44, 2d, 56, 35] elapsed_ms=6.62
```

With no TCP listener on the gateway (today), the guest's TCP connect to `100.95.0.1:53` reports success and the first read gets a connection reset in under 4 ms: a truncated answer's TCP retry fails fast, not with a hang. After the harness bound a listener to `100.95.0.1:53` in the host namespace (the spike's stand-in for the proposed listener), the guest's framed query got a framed reply with the query id echoed and the byte-distinct `GPD-V5-REPLY` litmus.

Design implication: GPD-5 (ADR-0360) option A works: a gateway-bound TCP listener is reachable from the guest through the existing path.

Gate: PROMOTE GPD-5 option A.
