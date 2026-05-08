# Root Cause Analysis — post-pivot S-2.2-17 falsification failure

**Date**: 2026-05-07
**Author**: Rex (nw-troubleshooter)
**Inputs**:
- `docs/analysis/post-pivot-s-2-2-17-falsification-probe-1.md` (probe-1 result + pcap diagnosis — load-bearing)
- `docs/product/architecture/adr-0045-bpf-redirect-neigh-datapath.md` § 5, § 7
- `crates/overdrive-bpf/src/programs/xdp_service_map.rs` (forward path, post-pivot)
- `crates/overdrive-bpf/src/programs/xdp_reverse_nat.rs` (reverse path, post-pivot — NEW)
- `crates/overdrive-dataplane/src/lib.rs` (loader; attaches both XDP programs)
- `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs` (test fixture)
- `crates/overdrive-dataplane/tests/integration/helpers/netns.rs` (3-iface topology)
- `docs/analysis/e1-bpftrace-results.md`, `docs/analysis/root-cause-analysis-s-2-2-17-length-n-tcp-drop.md` (pre-pivot RCA — methodology reference; bug class is NOT the same)

**Constraint**: no live probes run during this RCA — reasoning is against probe-1 evidence + post-pivot source review.

---

## Problem statement

Per ADR-0045 § 7, the post-pivot dual-XDP datapath (commits `aa7734c..c8e2e5f`,
steps 09-01..09-03) was supposed to make S-2.2-17 GREEN naturally — the
kernel IP-forwarder is structurally removed from the path, so the
pre-pivot `pskb_expand_head` → `skb_checksum_help` →
`SKB_DROP_REASON_TC_EGRESS` chain cannot recur.

The falsification probe (step 09-04, with `#[ignore]` lifted and the
`ethtool -K $iface ... off` block removed per ADR-0045 § 7's
"production-realistic veth defaults must work" requirement) **failed**:
client `nc` exited code 1 after the standard 5-second TCP retransmit
budget. Six SYNs left the client; none received a SYN-ACK.

Per ADR-0045 § 7's contract, this means there is a **new bug** — either
in the post-pivot dataplane programs OR in the test fixture — and the
diagnosis requires a fresh probe sequence rather than recurrence
chasing.

The headline empirical signal (probe-1, `/tmp/ovd-rnat3-1913186/`):

- `client.pcap` — 6 outbound SYNs `10.0.0.10:46452 → 10.0.0.1:8080`. ARP
  resolves `10.0.0.1` to `lb_veth_a`'s MAC. Zero return packets.
- `lb_a.pcap`, `lb_b.pcap` — zero IPv4 TCP frames (consistent with XDP
  consuming SYNs at ingress before tcpdump can see them; see WHY-2
  below).
- `backend.pcap` — **6 SYNs ARRIVE post-DNAT, dst correctly rewritten to
  `10.1.0.5:8080`. Backend emits NOTHING in response — no SYN-ACK, no
  RST, no ICMP unreachable.**

The asymmetry is the load-bearing signal: forward path is delivering
the SYN to the backend; the backend's TCP stack then drops it silently.
This RCA is structured around explaining that drop.

---

## Scope

**In scope**: explain why the backend's TCP stack consumes the SYN
without responding, ranked by likelihood against the probe-1 evidence
and the post-pivot source.

**Out of scope** (deferred to probe-2 dispatch under GH #159):
- Live probe execution (Lima, bpftrace, pwru, ss, `bpftool prog show`).
- Patches. This RCA produces hypothesis triples for probe-2, not fixes.
- Pre-pivot mechanism analysis. Per ADR-0045 § 7 the pre-pivot chain
  cannot structurally recur.

**Distinguishing test-fixture bug from dataplane bug**: this RCA must
keep the two outcomes separable. ADR-0045 § 7 wants both possibilities
on the table. The branch table below tags each candidate root cause
explicitly as `dataplane` or `fixture` so the next dispatch knows which
artifact would carry the fix.

---

## Toyota 5 Whys — multi-causal branch tree

Six branches at WHY 1, traced to WHY 5. Each branch carries verifiable
evidence at each level (probe-1 pcap, source line citation, or
production-realistic-veth reasoning); levels marked **HYPOTHESIS** are
unverified and named for probe-2 to falsify.

### WHY 1 — observable symptom: `nc -w 5` exits non-zero, 6 SYNs unanswered

**Evidence**: probe-1 panic message
`client nc exited non-zero (code = Some(1)); stdout = ""`; client.pcap
6 SYNs at the standard kernel retransmit schedule (1s, 2s, 4s, ...).

The symptom decomposes into the asymmetry already established: SYNs
arrive at backend, no return traffic anywhere. So the symptom branches
on "what consumed the SYN at the backend without producing a response":

```
WHY 1A: Backend kernel rejects SYN at L3/L4 input pipeline      [→ A]
WHY 1B: Backend kernel rejects SYN at L2 input pipeline         [→ B]
WHY 1C: SYN reaches socket but `nc` is not listening / loses it [→ C]
WHY 1D: Backend emits SYN-ACK but it dies on the return path    [→ D]
WHY 1E: Forward-path SYN content is wrong in a way pcap misses  [→ E]
WHY 1F: Test fixture in inconsistent state at SYN arrival time  [→ F]
```

D is a candidate per probe-1 § "Three-line preliminary diagnosis" but
the asymmetry signal (`backend.pcap` empty of return traffic) makes it
weaker than A/B; it is included for completeness and as the natural
target for probe-2c (per-skb pwru). E asks whether the wire-form SYN
that backend.pcap captured is *self-consistent* in ways the simple
src/dst/port match doesn't catch.

---

### Branch A — Backend kernel rejects SYN at L3/L4 input

**Evidence (WHY 1A)**: `backend.pcap` shows the SYN arriving with
correct `(src=10.0.0.10, dst=10.1.0.5, dport=8080)`. Backend emits no
SYN-ACK, no RST, no ICMP. tcpdump on `backend_veth` is post-XDP-ingress
but **pre-`ip_rcv`** in the netfilter sense — the kernel's L3/L4
verifier-and-drop sites (`ip_rcv_core`, `tcp_v4_rcv`,
`tcp_v4_do_rcv`) are downstream of the tcpdump tap, so a drop at any
of those sites is consistent with "tcpdump saw the SYN, backend
emitted nothing." This is the strongest branch — it explains the full
asymmetry without invoking unobserved kernel state.

#### WHY 2A — competing hypotheses for the L3/L4 drop site

```
WHY 2A.i   TCP checksum invalid → silently dropped at tcp_v4_rcv      [HYPOTHESIS]
WHY 2A.ii  IPv4 header checksum invalid → dropped at ip_rcv_core      [HYPOTHESIS]
WHY 2A.iii rp_filter strict-mode rejects rewritten src=10.0.0.10      [HYPOTHESIS]
WHY 2A.iv  conntrack marks SYN as INVALID                             [HYPOTHESIS]
WHY 2A.v   Backend kernel has no socket bound on `0.0.0.0:8080` → SYN
           hits NO_SOCKET, kernel emits RST, but RST gets eaten on
           return path                                                 [partial — D-coupled]
```

**Note 2A.v vs branch C**: 2A.v is the kernel-level "socket missing"
form; branch C is the application-level "socket is bound but `nc -l`
isn't accepting the SYN before the test times out." They share the
same symptom shape but split on whether `nc` actually called `bind` +
`listen` before the first SYN arrived.

#### WHY 3A.i — TCP checksum invalid: how could it happen?

The XDP forward path
(`crates/overdrive-bpf/src/programs/xdp_service_map.rs:418-482`) does
RFC 1624 incremental checksum updates:

- IP csum: `csum_incremental_2_2(old_csum, old_ip_lo, old_ip_hi,
  new_ip_lo, new_ip_hi)` — only the dst IP changes, 2 words. (line
  442)
- L4 csum: `csum_incremental_3_3(old_l4_csum, old_ip_hi, old_ip_lo,
  old_dst_port, new_ip_hi, new_ip_lo, new_dst_port)` — pseudo-header
  dst-IP (2 words) plus L4 dst-port (1 word). (line 446)

In S-2.2-17 specifically, `BACKEND_PORT == VIP_PORT == 8080`
(`reverse_nat_e2e.rs:78`). So `new_dst_port == old_dst_port` and the
3-3 fold reduces arithmetically to a 2-2 fold (the port pair cancels
to zero contribution). The arithmetic is straightforward.

**WHY 4A.i — failure modes inside that arithmetic**:

```
WHY 5A.i.α: Initial-packet TCP csum offload assumed PARTIAL by sender
            but XDP treats it as fully-computed. The client SYN
            originates from the same Linux kernel that just had
            ethtool offloads RE-ENABLED (per probe-1 procedure
            removing the `ethtool -K ... off` block). With TX
            checksum offload on, the kernel hands a partial-csum skb
            to the veth driver; the partial csum is materialised at
            tx time. On veth, however, the receiving peer typically
            sees an `ip_summed = CHECKSUM_UNNECESSARY` (computed by
            the stack) or `CHECKSUM_PARTIAL` (passed through). XDP
            sees the bytes-in-buffer regardless — but if those bytes
            are a partial-only csum (the L4 csum field on the wire
            holds only the pseudo-header sum, not the full sum),
            then RFC 1624 incremental update on a PARTIAL csum gives
            a result that is itself partial. The receiver's
            tcp_v4_rcv calls `__skb_checksum_complete` over the
            byte-form csum; partial+incremental = wrong full csum =
            silent drop.                                                 [HYPOTHESIS — RANK 1]

WHY 5A.i.β: Endianness inversion in csum operands. fold32 + the
            host-order word splits in `xdp_service_map.rs:436-454`.
            The IP `dst_ip` is read with `read_u32_be` (line 307),
            which returns a host-order u32 numerically equal to the
            wire bytes. Splitting via `(>> 16) as u16` and `& 0xffff`
            produces hi/lo "host-order halves of host-order u32". On
            little-endian the wire byte sequence `[0a, 00, 00, 01]`
            (= 10.0.0.1) reads back as `read_u32_be → 0x0a000001`.
            Splitting: hi = `0x0a00`, lo = `0x0001`. The kernel's
            csum formulation is over network-order 16-bit words
            literally as they appear on the wire: `[0a, 00]`,
            `[00, 01]` = `0x0a00` and `0x0001` if the kernel reads
            big-endian. Because both kernel and our `read_u32_be`
            agree (read big-endian, store host-order numeric),
            this should match. The risk is a sneaky asymmetry on
            big-endian arches (none in our matrix) or a misread of
            "host-order halves of u16". On x86_64 (Lima default)
            and aarch64 LE, hi/lo as defined produce wire-equivalent
            words.                                                       [LOW — RANK 5]

WHY 5A.i.γ: The L4 csum field offset is wrong for the chosen proto.
            `TCP_CSUM_OFFSET = 16` (line 82); valid for TCP. The
            test is TCP. No proto-mismatch.                               [N/A]

WHY 5A.i.δ: The csum write happens AFTER the FIB lookup reads
            tot_len/tos. Code order at lines 468-473 is:
            write IP csum, write IP dst, write L4 csum, write L4
            port — all four writes happen INSIDE
            `rewrite_and_tx`, before `fib_resolve_and_rewrite_mac`
            is called (line 481). FIB lookup then reads the
            POST-rewrite tot_len etc. tot_len did not change
            (rewrite is not adding/removing bytes), so this is
            fine. No bug here.                                            [N/A]
```

**5A.i.α is the rank-1 candidate root cause for branch A.** Its
mechanism: the *test fixture* used to disable TX checksum offload via
`ethtool -K $iface tx-checksum-ip-generic off tx off`; removing that
re-enables it. With TX checksum offload on, the client kernel's
`__tcp_v4_send_check` puts only the **pseudo-header sum** in the TCP
csum field and sets `skb->ip_summed = CHECKSUM_PARTIAL`. The veth
transmit path may or may not finalise the csum before delivery
depending on `dev->features` of the peer iface — for many
configurations the partial csum stays in the bytes that cross to the
peer. The XDP program reads partial-csum bytes, applies RFC 1624
incremental update for a 32-bit field swap (correct math under the
RFC), and writes a *new* partial-csum back. The receiving backend
kernel doesn't know it's partial: it computes `__skb_checksum_complete`
on the linear bytes, gets a wrong checksum (because the partial form
was missing the full TCP-header + payload contribution), and drops.

Note: `ip_summed` is not in the wire bytes, so XDP cannot inspect it
and adjust. The Cilium L4LB pattern handles this either by
(a) explicitly disabling TX offloads on the participating ifaces
*in production*, (b) recomputing the full TCP csum from scratch
inside XDP (expensive — O(payload length) verifier instructions),
or (c) calling `bpf_csum_diff` + relying on the kernel's
fixup-at-egress paths. The post-pivot programs do (b)? No — they do
RFC 1624 incremental, which is csum-state-preserving but not
csum-state-correcting.

#### WHY 3A.ii — IPv4 header csum invalid

Same arithmetic as 3A.i but for the IPv4 header. The IPv4 csum is
**always** fully materialised on the wire (no IP-level offload
analog) — it is not affected by `CHECKSUM_PARTIAL`. So the
incremental update is well-defined and the failure mode reduces to
a bug in `csum_incremental_2_2`. The math has been used in this
codebase for the pre-pivot path and exercised by the
length-0 control-segment paths that DID work end-to-end in pre-pivot
S-2.2-17 runs (per `e1-bpftrace-results.md`). **Rank: low** — this
arithmetic was previously exercised and worked; a regression would
have to come from a recent edit. Source review of lines 442 and 184–
191 shows no recent change.

[HYPOTHESIS — RANK 4 within branch A]

#### WHY 3A.iii — rp_filter

`reverse_nat_e2e.rs::ThreeIfaceTopology::create` (helpers/netns.rs:317-
320) explicitly sets `net.ipv4.conf.all.rp_filter=0` and
`conf.default.rp_filter=0` in lb-ns AND backend-ns AND client-ns.
That's the disable. **Unless the backend kernel applies a
per-iface rp_filter (sysctl `net.ipv4.conf.<iface>.rp_filter`) that
the test doesn't set**, this branch is structurally dead.

Linux's effective rp_filter is `max(conf.all.rp_filter,
conf.<iface>.rp_filter)` — so setting `all=0` only forces 0 if no
per-iface override is non-zero. The `default` setting applies to
**newly created** ifaces; ifaces created BEFORE the sysctl write
are not affected. `ThreeIfaceTopology::create` creates the ifaces
before the sysctl; the sysctl writes apply to the iface only via
`default` propagation, which they get because the ifaces are
created fresh.

**Rank: low** — but a probe-2a `bpftrace kfree_skb_reason` filtering
for `IP_RPFILTER` would falsify cheaply.

[HYPOTHESIS — RANK 3 within branch A]

#### WHY 3A.iv — conntrack INVALID

The 3-iface topology runs `ip` userspace to set up netns; nothing
loads `nf_conntrack` explicitly. But conntrack auto-loads on most
modern distros once any iptables/nftables rule references it. If the
Lima image has conntrack loaded by default, the SYN takes a
NEW state and should pass; INVALID would only fire if conntrack saw
prior packets that don't match. With a fresh netns and no prior
flow this is unlikely.

[HYPOTHESIS — RANK 4 within branch A]

#### WHY 3A.v — backend has no socket / NO_SOCKET kernel response

`backend_nc` is spawned via `topo.backend_ns.command("nc", ["-l",
"-p", &BACKEND_PORT.to_string(), "-q", "1"])`
(`reverse_nat_e2e.rs:248-255`). The test then sleeps 200ms before
starting tcpdump (line 261). nc on a Linux netns should bind +
listen well within that window. But there are races:
- nc might bind to `127.0.0.1` instead of `0.0.0.0` if a flag-default
  changed.
- The 200ms sleep is followed by `tcpdump` startup (another 200ms
  sleep on line 295) — total ~400ms before client traffic. nc startup
  inside `ip netns exec` typically takes <100ms, so this should be
  enough.
- If nc DID bind successfully, the kernel would normally send a RST
  on SYN to a non-LISTEN port — but `backend.pcap` shows zero return.
  So either nc IS listening (and silently dropping for some L7
  reason — branch C), or kernel-level drop happens before the RST is
  generated.

NO_SOCKET drops at `tcp_v4_rcv` produce a RST under default sysctl
settings. The fact that backend.pcap shows ZERO return packets — not
even a kernel RST — is a stronger signal AGAINST 2A.v: a kernel-
level no-socket would produce RST. So:

[ELIMINATED for branch A — passed through to branch C as
"app-level lost-listener" or back to A as "kernel suppresses RST",
which would itself need a separate signal]

#### WHY 4A.i — α (the rank-1 candidate): why does no test or guard catch the partial-csum hazard?

```
WHY 4A.i.α: There is no Tier 2 / Tier 3 test that asserts
            production-realistic veth offload behaviour against the
            forward-path program. The existing Tier 2 PKTGEN
            constructs csums explicitly (full csum), so it never
            exercises a CHECKSUM_PARTIAL skb. The existing pre-pivot
            Tier 3 (with `ethtool -K ... off`) explicitly disabled
            offloads. ADR-0045 § 7 sanctioned the offload re-enable
            but did NOT prescribe a corresponding csum-handling
            path in the XDP programs.
```

#### WHY 5A.i — root-cause-class: structural ordering of "checksum mode awareness" vs "checksum incremental update"

```
WHY 5A.i: The post-pivot XDP programs assume the wire-form bytes
          they see are the FULL csum. RFC 1624 incremental update
          is csum-state-preserving; it requires the input csum to
          be in the SAME state as the output csum will be evaluated
          against. The receiver evaluates against the FULL csum.
          If the input is PARTIAL (because TX-csum-offload deferred
          materialisation), then the incremental update produces a
          wrong full csum.

ROOT CAUSE A.i (CANDIDATE — RANK 1):
  XDP forward-path csum incremental update is unsafe under
  production-realistic veth defaults (TX checksum offload enabled),
  because the wire bytes presented to XDP are partial-csum form,
  not full-csum form.

  Type tag: dataplane bug (the post-pivot program is wrong)
  OR fixture/spec issue (ADR-0045 § 7's "production-realistic veth
  defaults must work" requirement is, on close reading, in tension
  with the XDP fast-path csum semantics; production deployments may
  need to disable TX offload at the load-balancer-attached ifaces).

  The classification depends on what production looks like. If
  production deployments WILL have offloads enabled at the LB ifaces
  (the spec-implied posture), this is a dataplane bug. If production
  is expected to disable offloads at LB ifaces (the operationally
  realistic posture for L4LBs that do XDP packet rewriting; Cilium
  documents this), then ADR-0045 § 7's "must work" line was over-
  reaching and the test fixture's offload disable was correct
  guidance, not a workaround.
```

---

### Branch B — Backend kernel rejects SYN at L2 input

**Evidence (WHY 1B)**: tcpdump on `backend_veth` runs *after*
`eth_type_trans` in the kernel's RX path. If `eth_type_trans` set
`pkt_type = PACKET_OTHERHOST` (the dest MAC didn't match the iface's
MAC), the SYN would still be tcpdump-visible (tcpdump sees all
classifications) but `ip_rcv` would drop it before any L3 work. The
backend.pcap pattern is consistent with this — the SYN appears, but
the kernel does nothing further with it.

#### WHY 2B — what produces a wrong dst MAC?

The XDP forward path resolves L2 MACs via `bpf_fib_lookup`
(`xdp_service_map.rs:541-569`). On RET_SUCCESS the program writes
`fib.dmac` into `eth->h_dest` and `fib.smac` into `eth->h_source`
(lines 588-593). If `bpf_fib_lookup` returns a `dmac` other than the
backend's actual MAC, the L2 destination check at the receiver fails.

The test pre-populates ARP via `ip neigh replace 10.1.0.5 lladdr
<backend_mac> dev lb_veth_b nud permanent` (`reverse_nat_e2e.rs:304-
326`). `<backend_mac>` is read from
`/sys/class/net/<backend_veth>/address` inside backend-ns
(`read_iface_mac` at line 409). So the lb-ns ARP table has
`10.1.0.5 → backend_veth's MAC`.

`bpf_fib_lookup` queries with `ipv4_dst = 10.1.0.5.to_be()` (line
554). The expected output: `dmac = backend_veth's MAC`, `smac =
lb_veth_b's MAC`, `ifindex = lb_veth_b's ifindex`. If that resolves
correctly, the rewritten frame's dst MAC = backend_veth's MAC, which
backend_veth's `eth_type_trans` accepts as PACKET_HOST.

#### WHY 3B — competing FIB-resolution failure modes

```
WHY 3B.i:  bpf_fib_lookup returns RET_NO_NEIGH despite ARP
           pre-population — falls through to XDP_PASS. The L3+L4
           rewrite has already committed (see line 579 comment).
           XDP_PASS hands rewritten packet to lb-ns kernel stack;
           kernel ARPs for 10.1.0.5 (already in table); kernel
           routes to lb_veth_b; lb_veth_b egress sends to peer with
           kernel-resolved MACs. Should still arrive at backend
           with correct dst MAC. Symptom-eliminated.                     [N/A]

WHY 3B.ii: bpf_fib_lookup returns RET_SUCCESS but resolves the
           WRONG egress iface (e.g. resolves to lo or a phantom
           iface). The neigh-replace was on lb_veth_b; FIB
           resolution should agree if the route table picks
           lb_veth_b for 10.1.0.0/24. lb-ns has IP_FORWARD=1
           (helpers/netns.rs:312) and a /24 route on lb_veth_b
           via `assign_ip_and_up(&lb_veth_b, "10.1.0.1/24")` (line
           303). FIB should pick lb_veth_b. **HOWEVER**: FIB lookup
           in XDP runs against the lb-ns FIB (because the program
           is attached in lb-ns). Correct expected behaviour.            [LOW — RANK 4]

WHY 3B.iii: bpf_fib_lookup is invoked with src_ip_host ≠ client IP.
            Looking at `xdp_service_map.rs:495` — `src_ip_host` is
            passed through from the caller (line 393), which got it
            from `read_u32_be(ctx, ETH_HDR_LEN +
            IPV4_SRC_IP_OFFSET)?` at line 358. **But this is read
            AFTER the dst-IP rewrite has happened?** No — line 358
            reads src_ip BEFORE the rewrite block at lines 468-473
            commits. src_ip is unchanged by forward-path DNAT, so
            this is correct. FIB src input = client IP = 10.0.0.10.
            FIB resolves "from 10.0.0.10 to 10.1.0.5 in lb-ns RIB"
            → next-hop neigh = backend_veth's MAC, egress =
            lb_veth_b. Looks right.                                      [N/A]

WHY 3B.iv: The redirect target ifindex is lb_veth_b's ifindex (per
            FIB output), but `bpf_redirect` delivers into a queue
            that doesn't actually arrive at backend_veth in time
            for tcpdump to see it before the SYN gets dropped. But
            backend.pcap DOES show the SYN — so this is eliminated.     [N/A]

WHY 3B.v:  ingress_ifindex (line 531) is being read as
            `(*ctx.ctx).ingress_ifindex`. fib.ifindex is set to
            ingress_ifindex (line 547). The FIB lookup uses that as
            the *input* iface for the lookup (i.e. "given a packet
            arrived on iface X with src/dst, where does it go?").
            On lb_veth_a ingress, fib.ifindex = lb_veth_a's
            ifindex. bpf_fib_lookup honors that as the lookup
            context. After the call, fib.ifindex is OVERWRITTEN
            with the egress ifindex. Then the comparison at line
            602 (`fib.ifindex == ingress_ifindex`) decides
            XDP_TX vs bpf_redirect.

            **BUT**: between the two reads, fib.ifindex is the
            output egress index. ingress_ifindex is captured to a
            local at line 531. So the comparison is "does FIB say
            egress is the same iface I came in on?" The 3-iface
            topology forces ingress=lb_veth_a, egress=lb_veth_b;
            different ifaces → bpf_redirect is taken.

            That's the correct path. No bug.                             [N/A]
```

**Rank for branch B**: low overall — the SYN reaches the backend
*and is captured by tcpdump*, which means it survived
`eth_type_trans`'s PACKET_OTHERHOST check (otherwise tcpdump would
also see it but `ip_rcv` would silently drop without any further
trace). Branch B is structurally weaker than branch A.

[BRANCH B RANK: 3]

---

### Branch C — SYN reaches socket but `nc` is not listening / loses it

#### WHY 1C (re-evidenced)

`nc -l -p 8080 -q 1` is spawned (line 248-260) but never produces
ANY response — neither RST (kernel-level no-listener) nor SYN-ACK
(app-level listener accepting). Branch A.v already ruled out kernel
NO_SOCKET (would emit RST). So if the SYN reached the socket but
nothing happened, nc must be:

```
WHY 2C: nc bound to a non-0.0.0.0 address                              [HYPOTHESIS]
WHY 2C: nc crashed before SYN arrived                                   [HYPOTHESIS]
WHY 2C: nc accept-loop is stuck somewhere                               [HYPOTHESIS]
```

Why would any of these silence the kernel-level RST? The kernel only
emits RST if NO process owns the socket. If nc bound + crashed but
left the socket in TIME_WAIT or the file descriptor in some
intermediate state, the kernel might keep the socket and not RST.

**Falsification of branch C is cheap**: a probe of the form
`ip netns exec backend-ns ss -tlnp` after the test fixture spawns
nc but before the client SYNs. If nc is listening on `0.0.0.0:8080`,
the SYN must have been consumed by the kernel (branch A) or by the
listener (branch C); the listener case requires nc to have been
buggy.

[BRANCH C RANK: 4 — empirically `nc -l -p 8080` is reliable on Lima
Ubuntu 24.04; would have to be a 100%-reproducible nc bug, which is
not credible.]

---

### Branch D — Backend emits SYN-ACK; it dies on return path

#### WHY 1D — pcap evidence against this branch

`backend.pcap` shows ZERO return packets from `10.1.0.5`. tcpdump on
`backend_veth` is at the same kernel layer for both directions — if
backend's TCP stack emitted a SYN-ACK, tcpdump would see it on the
egress path before any in-kernel hook could drop it. So branch D is
structurally falsified by probe-1, modulo:

```
WHY 2D: bpf_redirect from xdp_reverse_nat_lookup runs at lb_veth_b
        ingress and somehow consumes a SYN-ACK before tcpdump on
        backend_veth (the peer) can see it. **No** — tcpdump on
        backend_veth runs in backend-ns, EGRESS-side, before the
        SYN-ACK leaves backend-ns. xdp_reverse_nat_lookup is in
        lb-ns. So a SYN-ACK leaving backend's stack is captured by
        backend.pcap regardless of what lb-ns XDP does to it
        afterward. Pcap rules this out.                                  [N/A]
```

[BRANCH D RANK: 5 — pcap-eliminated. Probe-2c (pwru) would
double-confirm. Per debugging.md § 5 "compare populations" we have
the two populations (backend has SYN; backend has no return) and the
diff IS the diagnosis: nothing originates from 10.1.0.5.]

---

### Branch E — Forward-path SYN content is wrong in a way pcap missed

`tcpdump -s 256` (line 286 in reverse_nat_e2e.rs) captures the first
256 bytes of each frame. SYN frames are typically <80 bytes, so
nothing is truncated.

#### WHY 2E — what could be wrong that pcap accepts but kernel rejects?

```
WHY 2E.i: Total-length / IHL mismatch. IPv4 tot_len rewritten?
          No — XDP doesn't change tot_len in the forward path
          (line 469-472 only writes csum, dst_ip, l4_csum,
          dst_port). Kernel parses IPv4 header self-consistently.
          tcpdump and kernel see the same bytes.                         [N/A]

WHY 2E.ii: TCP option corruption. The XDP program does not touch
          TCP options or the data offset field. SYN options
          (MSS, SACK, window scale) survive untouched.                   [N/A]

WHY 2E.iii: TCP sequence number rewritten? No.                          [N/A]

WHY 2E.iv: Frame length / MTU. Forward path doesn't grow/shrink
          the frame; tot_len unchanged.                                  [N/A]

WHY 2E.v: ip_summed metadata wrong (this is branch A.i in
          disguise — the partial-csum hypothesis). Already
          covered as the rank-1 candidate.                               [→ A.i]
```

[BRANCH E RANK: 5 — collapses into A.i.]

---

### Branch F — Test fixture in inconsistent state

#### WHY 1F (re-evidenced)

ADR-0045 § 7 sanctioned removing the `ethtool -K ... off` block. The
new fixture state is:

- IP forwarding ON in lb-ns (helpers/netns.rs:312).
- rp_filter=0 in lb-ns / client-ns / backend-ns conf.{all,default}.
- ARP cache pre-populated on lb_veth_b for `10.1.0.5`.
- TX/GSO/TSO/GRO offloads ENABLED on every veth (post-pivot default).
- xdp_pass stub on backend_veth so XDP_REDIRECT into the peer works.

#### WHY 2F — what's missing or inconsistent?

```
WHY 2F.i: Per-iface rp_filter not set to 0 — the `default`
          propagation may not have applied to ifaces created BEFORE
          the sysctl write. Check via `cat /proc/sys/net/ipv4/conf/
          <iface>/rp_filter` would falsify.                              [→ A.iii]

WHY 2F.ii: TX-offload-on at the backend's egress (server-side) is
          fine; the asymmetry says backend isn't egressing anything
          anyway. The CLIENT side TX-offload-on is what feeds
          partial-csum SYNs into the LB. **This is A.i in fixture
          framing.**                                                     [→ A.i]

WHY 2F.iii: lb_veth_a / lb_veth_b RX-offload behaviour. veth peers
          can negotiate `NETIF_F_HW_CSUM` etc., which affects what
          `ip_summed` value the receiving veth sets. Doesn't change
          the wire bytes, but interacts with whether kernel-stack
          fallback paths (when XDP returns XDP_PASS) compute the
          csum correctly.                                                [→ A.i / 5A.i]

WHY 2F.iv: backend nc race window not papered over. 200ms+200ms
          should be enough for nc startup; if not, branch C.              [→ C]
```

[BRANCH F RANK: 2 — F is the framing of A.i / A.iii / C as fixture
issues rather than dataplane issues.]

---

## Cross-validation — do the candidate root causes contradict?

| Pairing | Consistent? | Notes |
|---|---|---|
| A.i (partial-csum) + B (L2 wrong) | Independent — both could fire | Probe-2a (drop-reason) distinguishes: TCP_CSUM = A.i, OTHERHOST/INHDR = B |
| A.i + A.iii (rp_filter) | Independent | Probe-2a distinguishes via reason |
| A.i + C (nc not listening) | Mutually exclusive at fail-time | Each leaves different evidence on `ss -tlnp` |
| A.i + D (return path) | Mutually exclusive | D is pcap-eliminated regardless |
| A.i + F.ii (fixture-offload framing) | A.i AND F.ii are the same root with different blame | Distinguishing IS the open spec question (ADR-0045 § 7's "must work" reading vs production posture) |

The candidates do not produce contradictory predictions for probe-2.
The decision matrix in probe-1 § "Decision matrix from probe-2 results"
already distinguishes them by `(2a reason, 2b run_cnt, 2c pwru-trace)`
triple. This RCA inherits that matrix and validates it.

---

## Backwards-chain validation — does each candidate fully explain the symptoms?

For each candidate root cause, walk back from WHY 5 to WHY 1 and check
that fixing it resolves the full chain — not just the deepest level.

### A.i (partial-csum hazard) — RANK 1

```
WHY 5A.i: post-pivot XDP assumes full-csum bytes; receives partial
          → 4A.i: no test guards production-realistic offload state
          → 3A.i: csum incremental update produces wrong full-csum
          → 2A.i: backend tcp_v4_rcv silently drops (TCP_CSUM)
          → 1A:   no SYN-ACK; client retransmits; nc times out
          → top:  test fails

Forward-chain: if partial-csum hazard removed (e.g. by disabling TX
offload on client-side veth), the wire SYN arrives full-csum, RFC
1624 incremental update preserves full-csum semantics, backend's
csum check passes, SYN-ACK fires.

Validation check: per debugging.md § 7 ("probe at the right
altitude"), the predicted falsification triple is:
- Probe 2a (`bpftrace tracepoint:skb:kfree_skb` for reason): if
  reason=`TCP_CSUM` count > 0 and matches the SYN burst (6),
  CONFIRMED.
- If reason=`TCP_CSUM` count = 0 but count of any other reason
  > 0, A.i FALSIFIED — branch elsewhere (A.iii rp_filter,
  A.ii ip_csum).
- If all reasons count = 0, branch C (app-level) or
  branch B (PACKET_OTHERHOST is also reasonless from skb_kfree
  perspective — it's a netif_rx-side path, not skb_kfree).

Single-level-fix check: if we only fixed WHY 5 (e.g., wrote
production code that handles partial csum), would WHY 4 still
hold? WHY 4 was "no test guards production-realistic offload
state" — fixing the program also requires adding the test, or
the next code drift re-introduces the same bug. So the COMPLETE
fix is dataplane code + Tier 3 test that intentionally enables
offloads. Single-level fix at WHY 5 alone is incomplete.
```

[A.i validation: full chain explained; predict TCP_CSUM under
probe-2a; complete fix requires both production change AND test
guard.]

### A.iii (rp_filter)

```
WHY 5A.iii: per-iface rp_filter sysctl not zeroed (only conf.all
            + conf.default were)
          → 4A.iii: helper sets sysctl AFTER iface creation; default
            propagation only applies to ifaces created after.
          → 3A.iii: kernel fpath uses
            max(conf.all.rp_filter, conf.<iface>.rp_filter); if
            <iface>.rp_filter=2 (strict), packet rejected.
          → 2A.iii: backend's lb_veth_b → backend_veth path may
            see a packet whose src=10.0.0.10 with no return route
            on backend_veth — strict rp_filter rejects.
          → 1A:   no SYN-ACK
          → top:  test fails

But wait — backend-ns's `add_route("default", ...,
LB_BACKEND_IP)` (helpers/netns.rs:326) sets the default route to
go via 10.1.0.1 (lb_veth_b's address as seen from backend). So
return route for src=10.0.0.10 IS valid (via 10.1.0.1). rp_filter
in strict mode would still ACCEPT this because the reverse-path
route exists.

So A.iii has a structural objection: rp_filter accepts a packet
when the reverse-path route resolves, even if the path is not
the same iface. The default route in backend-ns covers this.

Falsification: probe-2a reason=`IP_RPFILTER` count would have
to be 0 for A.iii to be wrong. It probably IS 0.
```

[A.iii validation: weak chain — structural objection at WHY 3.
Probe-2a IP_RPFILTER count is the falsification gate; expected to
be 0.]

### A.ii (IPv4 hdr csum bug)

```
WHY 5A.ii: csum_incremental_2_2 has a bug
          → 4A.ii: insufficient proptest coverage of edge inputs
          → 3A.ii: arithmetic produces wrong fold
          → 2A.ii: backend ip_rcv_core drops with reason=IP_CSUM
          → 1A:   no SYN-ACK; nc times out

Falsification: probe-2a reason=`IP_INHDR` (= IP_CSUM in some
kernels) count > 0 confirms; otherwise A.ii ruled out.

Single-level-fix: a fix at WHY 5 (correct the arithmetic) alone
suffices because the unit-test suite catches the regression
once a correct test exists. WHY 4 is a corollary not a parallel
branch.
```

[A.ii validation: full chain works in principle, but rank low —
the same arithmetic worked in pre-pivot length-0 segments.]

### B (L2 wrong dst MAC)

```
WHY 5B: bpf_fib_lookup misresolves egress, OR ARP pre-pop wrong
       MAC, OR the eth->h_dest write at xdp_service_map.rs:589
       writes the wrong array slot.
       → 4B: no Tier 2/3 test asserts post-rewrite dst MAC
       → 3B: dst MAC at backend_veth ≠ backend_veth's MAC
       → 2B: eth_type_trans sets PACKET_OTHERHOST; ip_rcv drops
       → 1B: no L3 processing; no return packet
       → top: test fails

Falsification: pcap on backend_veth shows the dst MAC. Probe-1
captured backend.pcap with `-s 256` — first 256 bytes including
the L2 header. **Reading the dst MAC byte sequence from the
captured pcap is a falsification probe that does NOT require a
new live run.** This RCA does not have the pcap bytes available
(only the textual summary), so the falsification is deferred to
probe-2 — but it could be done on the existing pcap file.
```

[B validation: full chain works; falsifiable from EXISTING pcap
without live probe — see Probe-2 priority below.]

### C (nc not listening)

```
WHY 5C: nc startup race or failure mode
       → 4C: 200ms+200ms sleep budget too tight on Lima
       → 3C: nc never reaches LISTEN by SYN arrival time
       → 2C: SYN reaches kernel; no listening socket
       → 2C continued: kernel emits RST? no — backend.pcap empty.
       → ELIMINATION: kernel WOULD emit RST in this case unless
         conntrack or iptables suppressed it. Default backend-ns
         has no iptables rules and conntrack is permissive.
         → contradicts backend.pcap evidence
```

[C validation: chain CONTRADICTS evidence — the chain would
produce a kernel RST visible in backend.pcap. Pcap shows zero
return. Branch C is structurally falsified by probe-1, modulo
exotic conntrack/iptables state.]

---

## Probe-2 ranking

Per `.claude/rules/debugging.md` § 10 (probe / hypothesis /
falsification triple) and § 7 (right altitude).

### Re-confirming the probe-1 ranking

The probe-1 doc proposed: 2a (`bpftrace kfree_skb_reason`), 2b
(`bpftool prog show id <reverse-nat-id>` for run_cnt), 2c (`pwru`
per-skb trace). The decision matrix at the bottom of the probe-1
doc was sound: the (2a, 2b, 2c) triple disambiguates branches A,
B, D, and the absence of all three points to C/F.

This RCA confirms the ranking but adds a **Probe-2 priority 0**
that costs nothing and could falsify branch B before any live
work begins:

### Probe 2-zero (priority 0): RE-READ the existing backend.pcap dst MAC

**Hypothesis**: the SYN at backend.pcap has dst MAC ≠
backend_veth's MAC (branch B confirmed).

**Prediction**: dst MAC in backend.pcap == backend_veth's MAC →
B falsified, A leading. Else B confirmed.

**Falsification path**: read the L2 header from the existing
`/tmp/ovd-rnat3-1913186/backend.pcap` (still on disk pending
test rerun) and compare the dst MAC bytes to the
`backend_mac` value the test logged via
`eprintln!("[diag] backend_veth MAC = {backend_mac}")`. The
falsification is a 1-line `tcpdump -r backend.pcap -e`.

**No new live probe**. Eliminating B costs zero kernel time.

### Probe 2a (priority 1) — `bpftrace kfree_skb_reason` filtered to backend-ns

Same as probe-1's 2a. Distinguishes A.i (TCP_CSUM), A.ii
(IP_INHDR), A.iii (IP_RPFILTER) by drop reason. **Highest
expected information gain** — single probe collapses three
branches into one verdict.

**Hypothesis**: backend kernel drops SYN at L3/L4 input.

**Prediction** (under A.i): reason=`TCP_CSUM`, count = 6
(matching SYN burst).

**Falsification**: count = 0 across all kfree_skb_reason values
that match the SYN window → branch A is wrong, the SYN reached
the socket and was lost there (branch C re-opens despite the
RST-elimination above; conntrack state would need a separate
probe). This shifts probability mass to branch C and forces a
follow-up.

### Probe 2b (priority 2) — `bpftool prog show id <reverse_nat_id>` run_cnt

Confirms the asymmetry probe-1 identified: zero return traffic
at lb_veth_b ingress means xdp_reverse_nat_lookup never runs.

**Hypothesis**: backend never emitted a SYN-ACK.

**Prediction**: run_cnt = 0 across the SYN burst window.

**Falsification**: run_cnt > 0 → backend DID emit a return packet
that hit lb_veth_b ingress → xdp_reverse_nat_lookup is the
guilty party, not the forward path. Branch D re-opens. Probe-2c
(pwru) would then trace the death site inside lb-ns post-XDP-
reverse.

### Probe 2c (priority 3) — `pwru --filter-track-skb 'host 10.1.0.5'`

Same as probe-1's 2c. Lowest priority — only material if 2a comes
back zero AND 2b comes back > 0. Otherwise pwru duplicates
information.

### NEW Probe 2d (priority 1 alternative) — `ss -tlnp` inside backend-ns post-fixture-setup

**Hypothesis**: nc is bound and listening on 0.0.0.0:8080.

**Prediction**: `ss -tlnp` inside backend-ns at `t = +400ms`
shows `0.0.0.0:8080 LISTEN <pid>/nc`.

**Falsification**: not present → branch C confirmed; the SYN
reached a non-listening kernel and the missing RST is the second
puzzle (separate dispatch).

This costs the same as 2a (single shell) and falsifies branch C
directly. **Probe-2 should run 2-zero + 2a + 2d in parallel.**

---

## Highest-rank candidate root cause

**RANK 1 — A.i.α (partial-csum hazard)**: the post-pivot XDP
forward path's RFC 1624 incremental checksum update assumes the
wire bytes carry a *full* TCP checksum. When TX-csum-offload is
enabled on the client-side veth (the production-realistic state
ADR-0045 § 7 sanctioned), the SYN's wire-form TCP csum is
*partial* (pseudo-header only), so the incremental update produces
a wrong full csum at the receiver. Backend's `tcp_v4_rcv` drops
with `SKB_DROP_REASON_TCP_CSUM` and emits nothing.

**Type tag**: ambiguous between dataplane bug and spec bug — see
WHY 5A.i. The classification depends on whether production-veth
deployments are expected to disable TX-csum-offload at LB ifaces
(operationally normal for L4LBs that rewrite packets) or expected
to keep them on (ADR-0045 § 7's "must work" line, taken
literally).

**Backwards-chain validation**: full chain explained. Solution
must address WHY 5 (program correctness) AND WHY 4 (test guard);
single-level fix at either alone is incomplete.

**Falsification cost**: probe-2a is a single bpftrace one-liner
during a single test run. If the rank-1 candidate is wrong, the
drop-reason output redirects the investigation in one round.

---

## Recommended probe-2 dispatch (single-shot)

Run probes 2-zero, 2a, and 2d **in parallel** during a single
re-run of S-2.2-17 in Lima:

| Probe | Priority | Effort | Falsifies |
|---|---|---|---|
| 2-zero | 0 (offline) | seconds | branch B (L2 wrong) |
| 2a | 1 | minutes | branches A.i / A.ii / A.iii distinguished by drop reason |
| 2d | 1 | minutes | branch C (nc not listening) |
| 2b | 2 | minutes | re-confirms branch D elimination |
| 2c | 3 | 10+ min | only if 2a returns reason=0 AND 2b returns run_cnt>0 |

Execution shape (per debugging.md § 10): the dispatch carries
hypotheses + predictions explicitly; results are scored against
predictions, not the original symptom. The decision matrix from
probe-1 § "Decision matrix from probe-2 results" stands and is
inherited by this RCA.

---

## Solution mapping (preview only — patch development is out of
scope per task constraint)

| Root cause candidate | Type | Mitigation (per `nw-investigation-techniques`) | Permanent fix |
|---|---|---|---|
| A.i.α (partial-csum) | dataplane OR spec | Re-add `ethtool -K $iface tx-checksum-ip-generic off ...` to the test fixture; document operator guidance to disable TX offloads at LB ifaces in production | Either: rewrite XDP to compute full TCP csum from scratch (verifier-budget impact; bpf_csum_diff helper exists) OR document that LB-attached ifaces must have TX offload disabled (production guidance ADR amendment) |
| A.iii (rp_filter) | fixture | Add per-iface rp_filter=0 sysctl to ThreeIfaceTopology after iface creation | Move sysctl writes to AFTER iface creation OR audit kernel default per matrix kernel |
| A.ii (IP csum bug) | dataplane | None | Fix `csum_incremental_2_2` arithmetic if the unit test surfaces the bug; add proptest coverage |
| B (L2 wrong) | dataplane OR fixture | Audit `bpf_fib_lookup` output via bpf_printk during dispatch | Match resolved dmac against ARP-cached MAC at runtime; assert in test fixture |
| C (nc race) | fixture | Increase startup sleep budget; use `nc -q 0` + explicit accept-loop healthcheck | Replace `nc` with a deterministic listener |

---

## References

- ADR-0045 § 5 (FIB miss → XDP_PASS), § 7 (S-2.2-17 falsification gate)
- `docs/analysis/post-pivot-s-2-2-17-falsification-probe-1.md` (probe-1
  evidence, decision matrix)
- `docs/analysis/e1-bpftrace-results.md` (pre-pivot probe chain;
  methodology reference, NOT bug-class precedent)
- `crates/overdrive-bpf/src/programs/xdp_service_map.rs` (lines 261-
  267 entry, 269-404 try-body, 418-482 rewrite, 493-612 FIB+L2+TX)
- `crates/overdrive-bpf/src/programs/xdp_reverse_nat.rs` (NEW; not
  on the SYN's forward path — eliminated as drop site by symmetry
  with branch D)
- `crates/overdrive-dataplane/src/lib.rs:460-503` (loader attaches
  reverse program on backend_iface ingress)
- `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs:
  78` (`BACKEND_PORT == VIP_PORT == 8080`; collapses 3-3 csum fold
  to 2-2 effectively)
- `crates/overdrive-dataplane/tests/integration/helpers/netns.rs:
  312-326` (post-pivot fixture; IP_FORWARD on, rp_filter all/default
  zero, ARP pre-population, NO offload disable)
- `.claude/rules/debugging.md` § 1 (falsifications disprove
  interventions), § 2 (error codes are taxonomy not mechanism), § 5
  (compare populations: SYN-at-backend ≠ SYN-ACK-from-backend), § 7
  (probe altitude), § 10 (hypothesis/prediction/falsification triple)
- GH #159 — *[2.x] Replace IP-forward + TCX-egress with
  bpf_redirect_neigh datapath* (probe-2 dispatch attaches here)
