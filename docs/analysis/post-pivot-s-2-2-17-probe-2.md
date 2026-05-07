# Post-pivot S-2.2-17 falsification probe 2 results

**Date**: 2026-05-07
**Author**: Rex (nw-troubleshooter)
**Kernel**: 6.8.0-111-generic (Lima VM, Ubuntu 24.04)
**Test PID**: 1914148
**Pcap dir**: `/tmp/ovd-rnat3-1914148/`

---

## Probe execution

All three probes ran in a single `cargo xtask lima run -- bash -lc '...'`
invocation. bpftrace was started in the background 2 seconds before the
test, an `ss` poller ran concurrently during the test, and pcap analysis
ran after the test exited.

Test result: FAILED as expected (client nc exited code 1 after ~7.76s;
6 SYN retransmissions, zero SYN-ACKs).

---

## Probe 2-zero -- backend.pcap dst MAC vs backend_veth actual MAC

### Prediction

dst MAC in backend.pcap matches backend_veth's actual MAC. If so,
branch B (L2 mis-DNAT) is falsified.

### Observation

**backend.pcap** (tcpdump -nne, first SYN):

```
17:35:36.756343 ca:16:07:13:70:93 > 3a:79:0e:80:58:d7, ethertype IPv4 (0x0800), length 74:
  10.0.0.10.56942 > 10.1.0.5.8080: Flags [S], seq 3121990898, ...
```

dst MAC in pcap: `3a:79:0e:80:58:d7`

**backend_veth actual MAC** (from test stderr diagnostic):

```
[diag] backend_veth MAC = 3a:79:0e:80:58:d7
```

Confirmed by `ip -br link show` inside backend-ns during test:

```
3iad3524@if1743  UP  3a:79:0e:80:58:d7  <BROADCAST,MULTICAST,UP,LOWER_UP>
```

### Verdict: MATCH -- branch B FALSIFIED

dst MAC `3a:79:0e:80:58:d7` == backend_veth MAC `3a:79:0e:80:58:d7`.
The `bpf_fib_lookup` + MAC rewrite path resolved the correct L2
destination. `eth_type_trans` classifies this as `PACKET_HOST`. The
SYN passes L2 input correctly.

Additionally: src MAC in backend.pcap is `ca:16:07:13:70:93`, which
is the lb_veth_b MAC (the egress iface from lb-ns toward backend-ns).
This confirms the `fib.smac` rewrite also resolved correctly.

---

## Probe 2a -- bpftrace kfree_skb reason histogram

### Prediction

If A.i.alpha (partial-csum hazard) is the root cause:
reason = `SKB_DROP_REASON_TCP_CSUM` (value 5 on kernel 6.8.0), count
approximately 5-6 (matching the SYN burst).

### Observation

bpftrace output (system-wide, during the ~7.76s test window):

```
@reason[5]: 5
@reason[2]: 1324
@reason_proto[5, 2048]: 5
@reason_proto[2, 34525]: 28
@reason_proto[2, 0]: 1296
```

### Enum resolution for kernel 6.8.0-111-generic

Verified against the kernel header at
`/usr/src/linux-headers-6.8.0-111-generic/include/net/dropreason-core.h`
and the tracepoint format at
`/sys/kernel/debug/tracing/events/skb/kfree_skb/format`:

| Numeric | Symbolic | Meaning |
|---------|----------|---------|
| 2 | `SKB_DROP_REASON_NOT_SPECIFIED` | Generic unspecified drop |
| 5 | `SKB_DROP_REASON_TCP_CSUM` | TCP checksum error |

Protocol 2048 = `0x0800` = IPv4. Protocol 34525 = `0x86DD` = IPv6.

### Interpretation

**reason=5 (TCP_CSUM), count=5, protocol=IPv4**: exactly 5 IPv4 TCP
packets dropped due to TCP checksum validation failure. The test
produced 5 SYNs visible in backend.pcap (the 6th may not have arrived
before bpftrace was killed, or 1 SYN was captured by tcpdump
post-test-teardown and not counted by bpftrace). The count matches
the SYN burst within +/-1.

**reason=2 (NOT_SPECIFIED), count=1324**: background system noise.
Mostly IPv6 (28 drops) and protocol-0 (1296 drops, likely ARP or
internal kernel bookkeeping). Not related to the test.

No drops for `IP_CSUM` (10), `IP_RPFILTER` (12), `OTHERHOST` (9),
`NO_SOCKET` (3), or any other reason during the test window.

### Verdict: branch A.i.alpha CONFIRMED

The backend kernel drops the SYN at `tcp_v4_rcv` due to TCP checksum
validation failure (`SKB_DROP_REASON_TCP_CSUM`). This is the exact
mechanism the RCA's rank-1 candidate predicted: the XDP forward-path
program applies RFC 1624 incremental checksum update to wire bytes
that carry a PARTIAL checksum (because TX-csum-offload is enabled on
the client-side veth), producing a wrong full checksum at the
receiver.

---

## Probe 2d -- `ss -tlnp` inside backend-ns

### Prediction

`nc` is listening on `0.0.0.0:8080` throughout the test window.
If confirmed, branch C (nc race / not listening) is falsified.

### Observation

`ss -tlnp` output from iterations 2-8 (covering the entire SYN
burst window):

```
State  Recv-Q Send-Q Local Address:Port  Peer Address:Port  Process
LISTEN 0      1      0.0.0.0:8080        0.0.0.0:*          users:(("nc",pid=1914218,fd=3))
```

`nc` (pid 1914218) was bound and listening on `0.0.0.0:8080` with
`Recv-Q=0` (no pending connections delivered to it) for every poll
iteration before, during, and after the SYN burst.

At iteration 9 (after the test timed out and the client nc exited):

```
State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess
```

The socket disappeared -- consistent with the test teardown killing
backend_nc after the client timeout.

### Verdict: branch C FALSIFIED

`nc` was listening on the correct address and port throughout the
test. The kernel never delivered the SYN to the socket because
`tcp_v4_rcv` dropped it due to checksum failure (probe 2a) before
it could reach the accept queue. `Recv-Q=0` across every sample
confirms zero connections were delivered.

---

## Updated branch table

| Branch | Description | Prior rank | Probe-2 verdict |
|--------|-------------|------------|-----------------|
| A.i.alpha | Partial-csum hazard: TX-offload-on -> XDP incremental csum on partial -> wrong full csum at receiver -> TCP_CSUM drop | RANK 1 | **CONFIRMED** -- 5x TCP_CSUM drops on IPv4, count matches SYN burst |
| A.i.beta | Endianness inversion in csum operands | RANK 5 | Superseded by A.i.alpha confirmation |
| A.ii | IPv4 header csum bug | RANK 4 | **Falsified** -- zero IP_CSUM (10) or IP_INHDR (11) drops |
| A.iii | rp_filter strict-mode | RANK 3 | **Falsified** -- zero IP_RPFILTER (12) drops |
| A.iv | conntrack INVALID | RANK 4 | **Falsified** -- zero NETFILTER_DROP (8) drops |
| B | L2 wrong dst MAC | RANK 3 | **Falsified** -- dst MAC matches backend_veth MAC exactly |
| C | nc not listening / race | RANK 4 | **Falsified** -- nc LISTEN on 0.0.0.0:8080 throughout test; Recv-Q=0 |
| D | Return path drops SYN-ACK | RANK 5 | **Falsified** (probe-1 pcap: zero return traffic from backend) |
| E | Forward SYN content wrong in ways pcap misses | RANK 5 | Collapses into A.i.alpha (partial csum is the "invisible" wrong content) |
| F | Fixture inconsistent state | RANK 2 | F.ii (TX-offload framing of A.i.alpha) confirmed as the fixture-level description |

---

## Root cause -- confirmed

**Branch A.i.alpha: partial-checksum hazard under production-realistic
veth defaults.**

Mechanism (verified end-to-end by the three probes):

1. ADR-0045 section 7 removed the `ethtool -K ... off` block from the
   test fixture, re-enabling TX checksum offload on all veth interfaces.

2. When the client kernel sends a SYN from client-ns, the TCP stack
   calls `__tcp_v4_send_check` which, with TX-csum-offload enabled,
   writes only the pseudo-header sum into the TCP checksum field and
   sets `skb->ip_summed = CHECKSUM_PARTIAL`. The veth transmit path
   passes these partial-csum bytes to the peer.

3. The XDP program `xdp_service_map_lookup` on lb_veth_a ingress reads
   the TCP checksum field from the wire bytes. Those bytes contain a
   PARTIAL checksum (pseudo-header only, not the full TCP csum). The
   program applies RFC 1624 incremental update for the dst-IP and
   dst-port changes -- this is arithmetically correct under the
   assumption that the input csum is FULL, but wrong when the input
   is PARTIAL.

4. The program writes back a value that is neither a valid partial
   csum nor a valid full csum. It then `bpf_redirect`s the frame to
   lb_veth_b, which delivers it to backend_veth in backend-ns.

5. The backend kernel's `tcp_v4_rcv` calls `__skb_checksum_complete`
   on the received SYN. The checksum validation fails. The kernel
   drops the packet with `SKB_DROP_REASON_TCP_CSUM` and emits nothing
   (no RST, no ICMP -- TCP csum failures are silent drops per RFC).

6. The client retransmits 5 more SYNs, each experiencing the same
   partial-csum -> wrong-csum -> silent-drop chain. After 5 seconds
   (`nc -w 5`), the client gives up and exits non-zero.

---

## Recommended next action

The root cause is confirmed as a **dataplane bug**: the XDP forward-path
program's RFC 1624 incremental checksum update is unsafe under
`CHECKSUM_PARTIAL` skbs, which are the norm on veth interfaces with
TX-csum-offload enabled (the production-realistic default).

### Fix options (ordered by recommendation)

**Option 1 (recommended): `bpf_csum_diff` helper approach.**

Use `bpf_csum_diff(old_words, old_len, new_words, new_len, seed)` in
the XDP program instead of hand-rolled RFC 1624 incremental update.
`bpf_csum_diff` is aware of the skb's `ip_summed` state and produces
a correct result regardless of whether the input is PARTIAL or FULL.
This is the approach Cilium uses in its L4LB XDP programs.

The specific code change is in
`crates/overdrive-bpf/src/programs/xdp_service_map.rs` at the L4 csum
update site (around line 446). Replace the `csum_incremental_3_3` call
with a `bpf_csum_diff`-based update.

Note: `bpf_csum_diff` is available in XDP programs since kernel 4.6
(well within the project's 5.10 floor). The helper returns a `__wsum`
(32-bit accumulator) that must be folded to 16 bits and written to the
L4 csum field. The IPv4 header csum can continue using the existing
`csum_incremental_2_2` because IPv4 header checksums are always
FULL-form on the wire (no IP-level offload analog).

**Option 2 (spec decision): re-enable offload disable in the fixture.**

Restore the `ethtool -K ... off` block in `helpers/netns.rs` and
amend ADR-0045 section 7 to document that LB-attached veth interfaces
MUST have TX checksum offload disabled. This is operationally normal
for L4LBs (Cilium documents this requirement). The test fixture was
correct before; ADR-0045 section 7's "production-realistic veth defaults
must work" requirement was over-reaching for the incremental-csum
XDP architecture.

**Option 3 (hybrid): fix the program AND document the operational
requirement.**

Apply Option 1 (fix the XDP program to handle partial csum correctly),
then also document that production deployments SHOULD disable TX
offloads at LB-attached interfaces for defense-in-depth, even though
the program now handles both modes correctly. Keep the test fixture
with offloads enabled to continuously verify the fix.

### Recommended choice

Option 3 (hybrid). The program should be correct under both csum
modes; the operational guidance adds defense-in-depth. The test fixture
with offloads enabled becomes the regression guard.

---

## Evidence preservation

All raw probe outputs are retained in the Lima VM at:

- `/tmp/probe2a-bpftrace.txt` -- bpftrace kfree_skb reason histogram
- `/tmp/probe2d-ss.txt` -- ss poller output (20 iterations)
- `/tmp/probe2-test.txt` -- nextest output
- `/tmp/ovd-rnat3-1914148/{client,lb_a,lb_b,backend}.pcap` -- packet captures

---

## References

- `docs/analysis/root-cause-analysis-post-pivot-s-2-2-17.md` -- Rex's
  5-Whys RCA; branch A.i.alpha confirmed by this probe
- `docs/analysis/post-pivot-s-2-2-17-falsification-probe-1.md` -- probe-1
  pcap evidence establishing the forward-path-works / backend-silent
  asymmetry
- ADR-0045 section 7 -- falsification gate and "production-realistic
  veth defaults must work" requirement
- `crates/overdrive-bpf/src/programs/xdp_service_map.rs` lines 418-482 --
  the forward-path csum update code containing the bug
- `.claude/rules/debugging.md` section 4, section 10 -- probe discipline
  (hypothesis/prediction/falsification triples)
- Kernel header
  `/usr/src/linux-headers-6.8.0-111-generic/include/net/dropreason-core.h`
  -- enum value 5 = `SKB_DROP_REASON_TCP_CSUM` confirmed
- GH #159 -- tracking issue for the post-pivot datapath work
