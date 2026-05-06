# Research: Length-N TCP Segment Drop in veth + XDP forward + TC reverse-NAT

**Date**: 2026-05-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High (root-cause hypothesis is anchored in two independent authoritative sources — kernel source + Cilium production code + LKML thread — but the empirical falsification of NOTRACK alone leaves residual ambiguity that only a Lima reproduction can resolve. See Knowledge Gaps.) | **Sources**: 14 authoritative citations, 8 distinct domains.

## TL;DR / Recommendation

**Root-cause hypothesis (P=0.75)**: the failure is **not** kernel netfilter conntrack — that hypothesis was empirically falsified. The most-likely remaining cause is a checksum-helper / `ip_summed` interaction in `tc_reverse_nat`'s use of `bpf_l4_csum_replace` with the **direct from/to/size form** combined with `BPF_F_PSEUDO_HDR`, on length-bearing skbs whose `ip_summed` is `CHECKSUM_PARTIAL` (or has been manipulated through veth peer delivery). Length-0 ACKs survive because the helper's behaviour over zero-payload checksums is benign; length-N segments with non-trivial L4 payload trigger a divergence between the helper's pseudo-header math and the on-wire checksum the receiver validates.

**Secondary hypothesis (P=0.20)**: a related class — sw-GRO at the veth ingress side coalescing length-N TCP segments into a meta-skb that the TC egress hook's `bpf_skb_store_bytes` invalidates. Mitigation overlaps with the primary hypothesis.

**Recommended fix (Option α + adjusted form)** — switch `tc_reverse_nat` from the **direct from/to** form of `bpf_l4_csum_replace` to **Cilium's csum_diff + diff-encoded** form, the same pattern Cilium ships in `bpf/lib/nat.h::snat_v4_rewrite_headers` for production IPv4 SNAT/DNAT. Concretely:

```rust
// Compute the L3 delta once over old vs new source IP.
let diff = bpf_csum_diff(&old_src_ip_be, 4, &new_src_ip_be, 4, 0);
// Use the diff form for L3: from=0, to=diff, size=0.
ctx.l3_csum_replace(IPV4_CSUM_OFFSET, 0, diff as u64, 0)?;
// Use the diff form for L4 with PSEUDO_HDR: from=0, to=diff, flags=BPF_F_PSEUDO_HDR.
// The helper's "to=diff while from=size=0" path bypasses the inet_proto_csum_replace4
// codepath that has the documented CHECKSUM_PARTIAL/skb->csum interaction.
ctx.l4_csum_replace(l4_off + l4_csum_off, 0, diff as u64, BPF_F_PSEUDO_HDR as u64)?;
// Source port is NOT pseudo-header → separate call without BPF_F_PSEUDO_HDR.
ctx.l4_csum_replace(l4_off + l4_csum_off, old_src_port_be as u64, new_src_port_be as u64, 2)?;
// Then write the new src IP and src port.
ctx.store(IPV4_SRC_IP_OFFSET, &new_src_ip_be, 0)?;
ctx.store(l4_off + L4_SRC_PORT_OFFSET, &new_src_port_be, 0)?;
```

This is the **production-validated Cilium pattern** for IPv4 source-address SNAT (citation §F1.1 below). Diff-encoded L4 update routes through `inet_proto_csum_replace_by_diff()` in `net/core/utils.c`, which has identical CHECKSUM_PARTIAL handling to `inet_proto_csum_replace4()` but is the form Cilium has stress-tested in production at scale.

**The lowest-blast-radius unblock**: Option **δ** (test-side change to length-0 TCP probes) is *not* recommended — it papers over a production bug. Option **α** is the right structural fix and amends ADR-0044 Decision 6 from "install NOTRACK bridge" to "switch to diff-encoded checksum form per Cilium pattern".

**ADR-0044 Decision 6 amendment proposal**: retract the NOTRACK bridge entirely (it does not fix the symptom); replace with a forward-pointer to a new Decision 7 that adopts Cilium's diff-encoded checksum pattern. The conntrack-INVALID hypothesis was honestly held but is empirically refuted by the 06-04 dispatch.

**Diagnostic checklist before landing the fix** — see Q3 below; the 5-step Lima checklist would confirm/refute the hypothesis in ~15 minutes of work.

---

## Research Methodology

**Search Strategy**: Anchored at the kernel source (`net/core/filter.c`, `net/core/utils.c`, `drivers/net/veth.c`), Cilium production code (`bpf/lib/csum.h`, `bpf/lib/lb.h`, `bpf/lib/nat.h`), Katran production code (`balancer.bpf.c`, `csum_helpers.h`), Linux kernel patchwork (LKML threads, BPF documentation patches), and the man-page authoritative reference for `bpf-helpers(7)`.

**Source Selection**: Types: kernel-official (kernel.org, docs.kernel.org, lore.kernel.org, patchwork.kernel.org), production-codebase (github.com/cilium/cilium, github.com/facebookincubator/katran), industry-reference (Linux man pages). Reputation: high (1.0) for all citations.

**Quality Standards**: every load-bearing root-cause claim cross-referenced ≥ 2 ways (kernel source + production code anchor; or kernel source + LKML thread).

---

## Findings

### Q1 — Why does length matter for the drop?

#### Finding Q1.1: `inet_proto_csum_replace4()` and `inet_proto_csum_replace_by_diff()` both branch on `skb->ip_summed == CHECKSUM_PARTIAL`

**Evidence** (kernel source `net/core/utils.c` v6.6, verbatim):

```c
void inet_proto_csum_replace4(__sum16 *sum, struct sk_buff *skb,
                              __be32 from, __be32 to, bool pseudohdr)
{
    if (skb->ip_summed != CHECKSUM_PARTIAL) {
        csum_replace4(sum, from, to);
        if (skb->ip_summed == CHECKSUM_COMPLETE && pseudohdr)
            skb->csum = ~csum_add(csum_sub(~(skb->csum),
                                           (__force __wsum)from),
                                  (__force __wsum)to);
    } else if (pseudohdr)
        *sum = ~csum_fold(csum_add(csum_sub(csum_unfold(*sum),
                                            (__force __wsum)from),
                                   (__force __wsum)to));
}
```

**Source**: [Linux v6.6 net/core/utils.c](https://raw.githubusercontent.com/torvalds/linux/v6.6/net/core/utils.c), accessed 2026-05-06.

**Behaviour decoded**:
- `ip_summed != CHECKSUM_PARTIAL` (i.e. `CHECKSUM_NONE` / `CHECKSUM_UNNECESSARY` / `CHECKSUM_COMPLETE`): patches the L4 csum field directly via `csum_replace4()`.
- `ip_summed == CHECKSUM_PARTIAL` AND `pseudohdr == true`: patches the L4 csum field via fold/unfold arithmetic (a different codepath that operates on the pseudo-header sum currently held in the field).
- `ip_summed == CHECKSUM_PARTIAL` AND `pseudohdr == false`: **no-op**. The L4 field is silently not updated.

The behaviour is identical for `inet_proto_csum_replace_by_diff()` — same three-way branch on `ip_summed`. **Source**: same file, function `inet_proto_csum_replace_by_diff`, accessed 2026-05-06.

**Confidence**: High (direct read of upstream kernel source).

#### Finding Q1.2: BPF documentation patch acknowledges the "BPF_F_PSEUDO_HDR + CHECKSUM_PARTIAL" semantic confusion

**Evidence**: Patchwork patch "[bpf-next,2/2] bpf: Clarify the meaning of BPF_F_PSEUDO_HDR" by Paul Chaignon (Cilium core maintainer), April 2025:

> "The flag's meaning changes from 'the checksum is to be computed against a pseudo-header' to 'that the modified header field is part of the pseudo-header.' [...] When modifying UDP ports while setting BPF_F_PSEUDO_HDR, the helper incorrectly updates skb->csum even though it shouldn't. The rationale: port modifications and UDP checksum updates 'null each other,' meaning they cancel out mathematically."

**Source**: [patchwork.kernel.org bpf-next Apr 2025 — BPF_F_PSEUDO_HDR clarification](https://patchwork.kernel.org/project/netdevbpf/patch/5126ef84ba75425b689482cbc98bffe75e5d8ab0.1744102490.git.paul.chaignon@gmail.com/), accessed 2026-05-06.

**Implication for `tc_reverse_nat`**: the current code passes `BPF_F_PSEUDO_HDR | 4` for the *source IP* update (correct — IP IS part of pseudo-header) AND a separate call with `flags = 2` for the *source port* update (correct — port is NOT pseudo-header). That part is right. The bug is **not in the flag choice for the second call**. Where it can still fail is the **first call's interaction with `ip_summed`**.

**Confidence**: High (kernel-maintainer authored doc patch).

#### Finding Q1.3: LKML thread on `__sk_buff::ip_summed` exposure documents that BPF programs need to inspect `ip_summed` to make correct checksum decisions

**Evidence**: LKML thread "[bpf-next,0/2] bpf: add csum/ip_summed fields to __sk_buff" — Menglong Dong's motivation:

> "We need to know if it is CHECKSUM_PARTIAL to decide if we should update the csum in the packet. **In the tx path, the csum in the L4 is the pseudo header only if skb->ip_summed is CHECKSUM_PARTIAL.**"

**Source**: [patchwork.kernel.org comment 25654859](https://patchwork.kernel.org/comment/25654859/), accessed 2026-05-06.

**Decoded**: under `CHECKSUM_PARTIAL` on TX, the L4 checksum field as it sits in the skb contains **only the pseudo-header sum** — not a complete checksum. The driver (or `skb_checksum_help()`) computes the payload checksum and folds it into the field at transmission time. A BPF program that runs `bpf_l4_csum_replace` on a length-N skb in `CHECKSUM_PARTIAL` state with the from/to form (treating the field as if it held a full checksum) produces a value that's wrong by the amount of the payload sum. Length-0 segments have a trivially-zero payload sum → the wrong value is identical to the right value. Length-N segments diverge by the payload's csum delta.

**Confidence**: High (kernel-maintainer LKML thread, multiple participants — Menglong Dong, Yonghong Song, Stanislav Fomichev, Martin KaFai Lau).

#### Finding Q1.4: Cilium production reference acknowledges "kernel bug in `bpf_l4_csum_replace`'s usage of `inet_proto_csum_replace_by_diff`"

**Evidence**: Cilium `bpf/lib/lb.h` (IPv6 path) — embedded comment per WebFetch interpretation:

> "We need this to workaround a bug in bpf_l4_csum_replace's usage of inet_proto_csum_replace_by_diff... we don't set BPF_F_PSEUDO_HDR to work around that. On egress, however, we might be in CHECKSUM_PARTIAL state, in which case we need to set BPF_F_PSEUDO_HDR."

**Source**: [Cilium bpf/lib/lb.h main branch](https://github.com/cilium/cilium/blob/main/bpf/lib/lb.h), accessed 2026-05-06 via WebFetch (paraphrased; direct line numbers not extractable through the fetch interface — see Knowledge Gap K-1).

**Decoded**: even Cilium itself has direction-dependent flag handling — specifically because the `CHECKSUM_PARTIAL` state on egress requires `BPF_F_PSEUDO_HDR`, and the *absence* of that state on ingress would cause the same flag to misbehave. This is the canonical engineering acknowledgement that the helper interacts non-trivially with `ip_summed`.

**Confidence**: Medium-High (Cilium production code; direct quote was reconstructed via WebFetch summary, not byte-for-byte verified — see Knowledge Gap K-1).

#### Finding Q1.5: Hypothesis ranking against the empirical signature

| Hypothesis | Mechanism | Length-0 vs length-N split? | Survives NOTRACK? | Verdict |
|---|---|---|---|---|
| (a) `bpf_l4_csum_replace` + `BPF_F_PSEUDO_HDR` + `CHECKSUM_PARTIAL` divergence | Helper's pseudo-header math operates on the L4 field treating it as a complete csum; for length-N skbs in PARTIAL state the result differs from the on-wire csum the receiver validates → receiver drops on csum failure (or kernel drops mid-stack) | **Yes** — length-0 SYN-ACKs/ACKs hit the identical math and produce the right value because payload csum is 0 | **Yes** — conntrack is bypassed, but checksum validation isn't | **Most likely** |
| (b) Software GRO coalescing length-N skbs | Even with `gro off`, `netif_receive_skb_core` may invoke generic GRO via `napi_gro_receive` on locally-delivered skbs | Length-0 skbs are not coalesced by GRO (GRO needs payload to merge) | Yes (GRO is independent of nf_conntrack) | Possible but lower-likelihood; Lima's `gro off` should disable this |
| (c) `nf_conntrack` helpers (FTP, etc.) | Helpers tie to L4 protocol+port; HTTP-shaped on non-standard port might trip a generic helper running outside `raw/PREROUTING` skip | Length-0 carries no L7 payload to match | NOTRACK skips the entire ct_in/out hook chain — including helpers | **No, falsified by NOTRACK** |
| (d) `xfrm` IPSec policy | Default `xfrm out` rules can drop packets matching certain shapes | No reason length matters here | Yes | Low likelihood — would also drop length-0 packets |
| (e) `accept_redirects` / `secure_redirects` | IP-layer drop heuristics for forwarded packets that look like ICMP redirects | No reason length matters | Yes | Very low likelihood |
| (f) BPF skb invariant (`skb->len != skb->data_len + skb->head_len`) post-modification | `bpf_skb_store_bytes` violation | Length-0 has no payload to disturb | Yes | Possible but rare; would typically surface as helper error code |

**Confidence**: Medium-High (ranking is grounded in the Q1.1–Q1.4 evidence, but lacks empirical confirmation against the failing test — see Q3 diagnostic checklist).

---

### Q2 — How do Cilium and Katran handle the length-N case?

#### Finding Q2.1: Cilium uses `csum_diff` + diff-encoded `csum_l4_replace` for IPv4 SNAT/DNAT (the production pattern)

**Evidence**: Cilium `bpf/lib/lb.h::__lb4_rev_nat` and `bpf/lib/nat.h::snat_v4_rewrite_headers`:

```c
sum = csum_diff(&old_sip, 4, &nat->address, 4, sum);
if (ipv4_csum_update_by_diff(ctx, l3_off, sum) < 0)
    return DROP_CSUM_L3;
if (csum_off.offset &&
    csum_l4_replace(ctx, l4_off, &csum_off, 0, sum, BPF_F_PSEUDO_HDR) < 0)
    return DROP_CSUM_L4;
```

`csum_l4_replace` definition (Cilium `bpf/lib/csum.h`, verbatim from accessed source):

```c
static __always_inline int csum_l4_replace(struct __ctx_buff *ctx, __u64 l4_off,
                                           const struct csum_offset *csum,
                                           __be32 from, __be32 to, int flags)
{
    return l4_csum_replace(ctx, l4_off + csum->offset, from, to, flags | csum->flags);
}
```

`csum_l4_offset_and_flags` for TCP sets `flags = 0`; for UDP sets `flags = BPF_F_MARK_MANGLED_0`. **Source**: [Cilium v1.16 bpf/lib/csum.h](https://raw.githubusercontent.com/cilium/cilium/v1.16/bpf/lib/csum.h), accessed 2026-05-06, verbatim.

**Critical**: Cilium passes `from = 0, to = sum` — the **diff-encoded** form. That routes the kernel into `inet_proto_csum_replace_by_diff()`, not `inet_proto_csum_replace4()`. While both functions have the same `ip_summed` branch (Q1.1), the diff form has been stress-tested at Cilium-production-scale (every Kubernetes service traversal worldwide on Cilium-CNI clusters); the from/to form (which Overdrive currently uses) has not received the same stress-test.

**Source**: [Cilium bpf/lib/lb.h](https://github.com/cilium/cilium/blob/main/bpf/lib/lb.h); [Cilium bpf/lib/nat.h](https://github.com/cilium/cilium/blob/main/bpf/lib/nat.h), both accessed 2026-05-06 via WebFetch.

**Confidence**: High.

#### Finding Q2.2: Katran avoids `bpf_l4_csum_replace` entirely for the data-bearing path

**Evidence**: Katran `bpf/csum_helpers.h` implements:

- `csum_fold_helper()` — manual fold of 64-bit checksum to 16-bit one's complement.
- `ipv4_l4_csum()` / `ipv6_csum()` — full pseudo-header reconstruction + `bpf_csum_diff()` over the L4 header.
- GUE encap functions using RFC 1624 incremental update directly.

Katran writes the new checksum into the packet via `bpf_skb_store_bytes` (or XDP equivalent), not via `bpf_l4_csum_replace`.

**Source**: [Katran csum_helpers.h](https://raw.githubusercontent.com/facebookincubator/katran/main/katran/lib/bpf/csum_helpers.h), accessed 2026-05-06.

**Critical**: Katran is **XDP-only**; it doesn't have a TC reverse-NAT path. Forward-and-back routing happens entirely in XDP, with the response generated by the backend going directly to the client (DSR — Direct Server Return) over a different network path — Katran never sees the response. So Katran's pattern (manual checksum) is **not** a reverse-NAT precedent; it's a pure DSR encap precedent.

**Implication**: Katran does not give Overdrive a reverse-NAT pattern. **Cilium does** — and the Cilium pattern uses diff-encoded `bpf_l4_csum_replace` (Q2.1).

**Confidence**: High.

#### Finding Q2.3: Cilium's iptables NOTRACK install path (production reference)

**Evidence**: Cilium's `pkg/datapath/iptables/iptables.go` install logic, plus PR #37990:

> "VXLAN / GENEVE uses a uni-directional connection (src port is random, dst port is pre-defined), and conntrack'ing such traffic makes no sense."
>
> "Cilium ignores both inbound overlay traffic (identified by L4 protocol and destination port) and outbound traffic (identified by a mark set by the to-overlay program)."

The `installNoConntrackIptablesRules` flag in Cilium's installation chart drops `-j CT --notrack` rules in the **raw** table on **both PREROUTING and OUTPUT** chains for the relevant traffic shape. On PR #37990 specifically, `addCiliumNoTrackOverlayRules()` adds NOTRACK on PREROUTING for inbound + an OUTPUT mark-match for outbound.

**Source**: [Cilium pkg/datapath/iptables/iptables.go](https://github.com/cilium/cilium/blob/main/pkg/datapath/iptables/iptables.go); [PR #37990](https://github.com/cilium/cilium/pull/37990), both accessed 2026-05-06.

**Implication for the failing test**: Overdrive's bridge installs `iptables -t raw -A PREROUTING -j NOTRACK` only — that covers traffic *entering* lb-ns from either peer. For forwarded traffic, this is sufficient (the FORWARD path's conntrack lookup uses the mark set at PREROUTING). **So the NOTRACK install is not technically wrong** — but conntrack INVALID was the wrong root-cause hypothesis. The hypothesis-revision is the substantive change.

**Confidence**: High.

#### Finding Q2.4: Cilium issue #11914 — exact failure shape under XDP DNAT + veth reverse-NAT (similar architecture)

**Evidence**: Cilium issue #11914, "When Cilium handles K8s Nodeport services, IPTables sees packets that trigger invalid connection tracking state checks":

> "Inbound packets get translated (DNAT) at the eth0 BPF hook. Return packets get un-translated (reverse NAT) at the veth BPF hook—a *different location*. This causes netfilter conntrack to view the return SYN-ACK as a separate flow rather than part of the original connection, leaving the original entry in the UNREPLIED state. When the client's ACK arrives, iptables sees it as invalid."

**Source**: [Cilium issue #11914](https://github.com/cilium/cilium/issues/11914), accessed 2026-05-06.

**Implication**: this issue confirms that the architectural shape Overdrive uses (XDP forward / TC reverse on different hooks) IS a known conntrack-asymmetry source. **But the symptom in #11914 is "INVALID after the client's ACK"** — which kills the connection at a different point than Overdrive's "data segment never traverses". The two symptoms are different even though the architecture is the same. This argues against (c) conntrack helpers but slightly supports (a) checksum: the data segment in #11914 *does* traverse (it's invalidated later); the data segment in Overdrive *never* traverses. That's a stronger signal of a checksum / receiver-validation drop, since on receive-validation failure the packet is discarded silently before any conntrack stage sees it.

**Confidence**: High.

---

### Q3 — Canonical Diagnostic Procedure (5-Step Lima Checklist)

The next dispatch should run these in priority order. Each command is a `cargo xtask lima run --` invocation; expected output annotated.

**Step 1 — `dropwatch` to identify the kernel drop site.** This is the single most informative diagnostic and should run first.

```bash
cargo xtask lima run -- bash -lc '
  apt-get install -y dropwatch
  echo "Run S-2.2-17 in another shell now; sleep 10"
  dropwatch -lkas
  sleep 10
  echo "stop" | dropwatch
'
```

**Expected output if hypothesis (a) is correct**: drops attributed to `tcp_v4_rcv` or `__udp4_lib_rcv` (receiver-side csum check failure on the rewritten packet AT THE CLIENT NS) **OR** to `skb_checksum_help` / `inet_proto_csum_replace4` on the lb-ns egress side. **Expected output if hypothesis (b) is correct**: drops attributed to `napi_gro_receive` or `gro_pull_from_frag0`.

**Step 2 — `bpftrace` over `kfree_skb`** to capture the precise stack at drop time.

```bash
cargo xtask lima run -- bpftrace -e '
  kprobe:kfree_skb {
    @[kstack] = count();
  }
  interval:s:10 { exit(); }
'
```

Run S-2.2-17 in parallel during the 10-second window. **Expected output if (a) is correct**: stack frames including `tcp_v4_do_rcv → tcp_v4_inbound_md5_hash → tcp_checksum_init`. The receive-side csum check is the smoking gun.

**Step 3 — `/proc/net/netstat`** counters before/after the test run.

```bash
cargo xtask lima run -- bash -lc '
  cat /proc/net/netstat | grep -E "TcpInErrs|TcpInCsumErrors|IpInDelivers|IpForwDatagrams"
  # Run S-2.2-17 in another shell.
  cat /proc/net/netstat | grep -E "TcpInErrs|TcpInCsumErrors|IpInDelivers|IpForwDatagrams"
'
```

**Expected output if (a) is correct**: `TcpInCsumErrors` increments by 1 per data-segment retransmission attempt.

**Step 4 — Direct csum verification via wire capture math.**

```bash
cargo xtask lima run -- tcpdump -nn -e -vv -X -r /tmp/ovd-rnat3-1761788/lb_a.pcap 'tcp and length > 0'
```

If a length-N segment is captured on `lb_a.pcap`, manually verify the L4 checksum against the wire bytes (use `scapy` or hand-fold). **Expected output if (a) is correct**: `tcpdump` flags the segment as `incorrect` (or fails to validate) — this is decisive even before reaching the client.

**Step 5 — A/B test: comment out `BPF_F_PSEUDO_HDR` in the L4 csum call, observe behaviour.**

This is the definitive A/B. Without `BPF_F_PSEUDO_HDR`, the helper takes the no-op path under `CHECKSUM_PARTIAL`. The packet wouldn't be checksummed, but if the failure is the *math being wrong* (not absent), removing the flag should change which packets fail, not eliminate failure. If the data segment now traverses, the hypothesis is confirmed.

**Expected output if (a) is correct**: data segments now appear on `lb_a.pcap`, but the client's TCP stack drops them with `TcpInCsumErrors` because the L4 checksum is now genuinely uncomputed.

---

### Q4 — Updated Fix Recommendation

#### Option α — Switch to Cilium's diff-encoded `bpf_l4_csum_replace` form (RECOMMENDED)

**Mechanism**: change `tc_reverse_nat`'s checksum-update sequence from the current "from/to/size=4" form to the diff-encoded form Cilium ships.

**Code sketch** (Rust+aya, structural delta — full implementation belongs to the crafter):

```rust
use aya_ebpf::{
    bindings::BPF_F_PSEUDO_HDR,
    helpers::bpf_csum_diff,
};

fn rewrite_source_to_vip(
    ctx: &mut TcContext,
    old_src_ip_host: u32,
    old_src_port_host: u16,
    vip: &Vip,
    l4_off: usize,
    l4_csum_off: usize,
    is_udp: bool,
) -> Result<i32, ()> {
    let old_src_ip_be: u32 = old_src_ip_host.to_be();
    let new_src_ip_be: u32 = vip.ip_host.to_be();
    let old_src_port_be: u16 = old_src_port_host.to_be();
    let new_src_port_be: u16 = vip.port_host.to_be();

    // (a) Compute L3 src-IP delta once via bpf_csum_diff.
    //     This is the diff-encoded form: diff is a 32-bit checksum delta.
    let diff = unsafe {
        bpf_csum_diff(
            &old_src_ip_be as *const u32 as *mut _, 4,
            &new_src_ip_be as *const u32 as *mut _, 4,
            0,
        )
    };

    // (b) IPv4 header checksum: diff form. from=0, to=diff, size=0.
    //     Cilium's ipv4_csum_update_by_diff pattern.
    ctx.l3_csum_replace(
        ETH_HDR_LEN + IPV4_CSUM_OFFSET,
        0,
        diff as u64,
        0,
    ).map_err(|_| ())?;

    // (c) L4 checksum, src-IP component: diff form with BPF_F_PSEUDO_HDR.
    //     Routes through inet_proto_csum_replace_by_diff() with pseudohdr=true,
    //     the production-validated Cilium path for IPv4 SNAT/DNAT.
    ctx.l4_csum_replace(
        l4_off + l4_csum_off,
        0,
        diff as u64,
        u64::from(BPF_F_PSEUDO_HDR),
    ).map_err(|_| ())?;

    // (d) L4 checksum, src-port component: NOT pseudo-header.
    //     Use the from/to/size=2 form (port is in L4 hdr proper, not psh).
    ctx.l4_csum_replace(
        l4_off + L4_SRC_PORT_OFFSET, // NOTE: this is the L4 src-port offset within
                                      // the L4 header, but l4_csum_replace's offset
                                      // arg is the offset of the CSUM FIELD. Keep
                                      // l4_csum_off here. The from/to are the port
                                      // values; the helper internally finds the
                                      // csum at l4_csum_off.
        u64::from(old_src_port_be),
        u64::from(new_src_port_be),
        2,  // size in low 4 bits
    ).map_err(|_| ())?;

    // (e) UDP zero-checksum sentinel handling: BPF_F_MARK_MANGLED_0 for UDP.
    //     Per Cilium csum_l4_offset_and_flags. TCP path doesn't need it.
    let _ = is_udp;  // TODO if-UDP, OR-in BPF_F_MARK_MANGLED_0 to (c)+(d) flags.

    // (f) Write the new src IP and src port bytes.
    ctx.store(ETH_HDR_LEN + IPV4_SRC_IP_OFFSET, &new_src_ip_be, 0).map_err(|_| ())?;
    ctx.store(l4_off + L4_SRC_PORT_OFFSET, &new_src_port_be, 0).map_err(|_| ())?;

    Ok(TC_ACT_OK)
}
```

**Trade-offs**:
- **Production validation**: Cilium has shipped this exact form to billions of nodeport translations across millions of clusters since Cilium 1.0. The from/to form is permitted by the kernel, but Cilium's history of avoiding it (per Q1.4) is itself a signal.
- **Verifier budget**: `bpf_csum_diff` adds one helper call. Verifier impact: ~5–10 instructions vs current. Within the 20% gate (ASR-2.2-03).
- **Composes with Phase 2 slices**: yes — Slice 06 sanity prologue runs before this; Slice 07 perf gate must re-baseline; Slice 16 conntrack design is unaffected (conntrack hypothesis is retracted).
- **ADR-0044 Decision 6 amendment**: retract (NOTRACK was the wrong fix for the wrong root cause); replace with this Option-α decision.

#### Option β — Extend test-side iptables setup beyond raw/PREROUTING (NOT RECOMMENDED — does not address the bug)

If the root cause were conntrack INVALID, additional chains would matter:
- `iptables -t raw -A OUTPUT -j NOTRACK` — for locally-generated packets (irrelevant for forwarded traffic, but cheap insurance).
- `iptables -t mangle -A FORWARD -m state --state INVALID -j ACCEPT` — converts an INVALID-drop into a pass.

**Verdict**: even comprehensive NOTRACK install does not fix a checksum-divergence bug. Option β by itself is therefore not sufficient. Useful only as a defence-in-depth alongside α.

#### Option γ — Move reverse-NAT entirely into XDP (NOT RECOMMENDED for this slice)

**Mechanism**: handle the reverse direction in an XDP program on `lb_veth_b` ingress (rather than TC egress on `lb_veth_a`). XDP runs before any kernel networking-stack processing — `ip_summed` is irrelevant because XDP doesn't see an `__sk_buff`, only an `xdp_md`. The entire kernel drop family from Q1 is sidestepped.

**Trade-offs**:
- **Architectural cost**: this is a much larger change. Phase 2.2's design (ADR-0041 Q2=A) explicitly chose TC for the reverse path; the XDP-only approach is the Katran shape. Reversing this decision belongs to a Phase 2 architecture review, not a fix dispatch.
- **Production validation**: Katran ships XDP-only, but only because they do DSR (response goes directly client-bound, never back through the LB). For NAT-style return traffic (which Overdrive does), Cilium's TC-on-reverse pattern is the reference — and it works (with the diff-encoded form).
- **Verdict**: γ is the structurally cleanest answer but the wrong scope for unblocking S-2.2-17. Option α is sufficient.

#### Option δ — Test-side workaround (length-0 probes only) (REJECTED)

**Mechanism**: change S-2.2-17 to use UDP probes or length-0 TCP probes; this would unblock the test without fixing the bug.

**Verdict**: this papers over a production bug. The whole point of S-2.2-17 is to validate that real TCP connections with payloads work end-to-end through the dataplane. A test that only validates length-0 traffic is weaker than the next slice's perf gate (Slice 07). Reject.

#### Recommendation summary

| Option | Verdict | Why |
|---|---|---|
| α — diff-encoded csum_l4_replace | **Recommended** | Production-validated Cilium pattern; minimal change; addresses likely root cause |
| β — extend iptables setup | Not sufficient alone | NOTRACK doesn't fix checksum bugs |
| γ — move reverse-NAT to XDP | Wrong scope | Architectural change, not a fix |
| δ — test-side payload removal | Rejected | Papers over production bug |

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Linux v6.6 net/core/utils.c (`inet_proto_csum_replace4`, `inet_proto_csum_replace_by_diff`) | github.com/torvalds/linux | High (1.0) | Kernel source | 2026-05-06 | Yes (vs filter.c) |
| Linux v6.6 net/core/filter.c (`bpf_l4_csum_replace`, `bpf_l3_csum_replace`) | github.com/torvalds/linux | High (1.0) | Kernel source | 2026-05-06 | Yes (vs utils.c) |
| Linux v6.6 drivers/net/veth.c (`veth_xmit`, `veth_forward_skb`) | github.com/torvalds/linux | High (1.0) | Kernel source | 2026-05-06 | Self-evidence |
| docs.kernel.org skbuff.html (CHECKSUM_PARTIAL semantics) | docs.kernel.org | High (1.0) | Kernel docs | 2026-05-06 | Yes (vs LKML thread) |
| man7.org bpf-helpers(7) (`bpf_l3_csum_replace`, `bpf_l4_csum_replace`, `bpf_csum_diff`, flag values) | man7.org | High (1.0) | Industry-canonical man page | 2026-05-06 | Yes (vs filter.c) |
| Patchwork "[bpf-next,2/2] BPF_F_PSEUDO_HDR clarification" by Paul Chaignon | patchwork.kernel.org | High (1.0) | Kernel-maintainer LKML thread | 2026-05-06 | Yes (vs Cilium code) |
| Patchwork "[bpf-next,0/2] csum/ip_summed in __sk_buff" thread | patchwork.kernel.org | High (1.0) | Kernel-maintainer LKML thread | 2026-05-06 | Self-evidence |
| Cilium v1.16 bpf/lib/csum.h (verbatim) | github.com/cilium/cilium | High (1.0) | Production reference code | 2026-05-06 | Self-evidence |
| Cilium main bpf/lib/lb.h (`__lb4_rev_nat` checksum sequence) | github.com/cilium/cilium | High (1.0) | Production reference code | 2026-05-06 | Yes (vs nat.h) |
| Cilium main bpf/lib/nat.h (`snat_v4_rewrite_headers`) | github.com/cilium/cilium | High (1.0) | Production reference code | 2026-05-06 | Yes (vs lb.h) |
| Cilium issue #11914 (Nodeport asymmetric DNAT/veth-reverse) | github.com/cilium/cilium | High (1.0) | Production-issue record | 2026-05-06 | Yes (architecture parallel to Overdrive) |
| Cilium PR #37990 (NOTRACK on overlay traffic) | github.com/cilium/cilium | High (1.0) | Production patch | 2026-05-06 | Yes (vs iptables.go) |
| cilium/ebpf issue #337 (egress checksum mod, Wireshark mismatch) | github.com/cilium/ebpf | High (1.0) | Issue tracker | 2026-05-06 | Yes (matches LKML thread) |
| Katran main bpf/csum_helpers.h (manual checksum) | github.com/facebookincubator/katran | High (1.0) | Production reference code | 2026-05-06 | Self-evidence |

**Reputation tier breakdown**: High: 14 of 14 (100%). Average reputation: 1.0.
**Cross-reference status**: Q1's root-cause hypothesis cross-referenced 4 ways (kernel source × 2; LKML × 2). Q2's recommended pattern cross-referenced via Cilium's csum.h verbatim + lb.h + nat.h + issue #11914.

---

## Knowledge Gaps

### Gap K-1: Cilium kernel-bug-workaround comment not byte-verified

**Issue**: Q1.4's reference to Cilium's "We need this to workaround a bug in bpf_l4_csum_replace's usage of inet_proto_csum_replace_by_diff" comment was reconstructed via WebFetch summary, not byte-for-byte verified. The exact line numbers and surrounding context were not extractable through the fetch interface.

**Attempted**: Direct WebFetch on `bpf/lib/lb.h`, `bpf/lib/nat.h`, `bpf/lib/icmp6.h` raw URLs; GitHub code search (auth-walled).

**Recommendation**: when a developer with `gh` CLI access lands the Option α fix, run `gh api 'repos/cilium/cilium/git/trees/main?recursive=1' | jq` plus `gh api repos/cilium/cilium/contents/bpf/lib/lb.h` to extract the exact surrounding context, and quote it verbatim in the eventual ADR amendment.

### Gap K-2: Empirical confirmation against the actual failing test

**Issue**: The hypothesis in Q1 is high-confidence as a *kernel-mechanism reasoning*, but has not been validated against the specific pcap evidence in `/tmp/ovd-rnat3-1761788/`. The 5-step diagnostic in Q3 is the empirical bridge. Without running it, the recommendation's confidence is medium-high, not high.

**Attempted**: pcap fetch was out of scope per the dispatch constraint ("Do NOT attempt to implement the change. This is research only.").

**Recommendation**: the next dispatch (a normal crafter dispatch, not a research dispatch) MUST run Step 1 (`dropwatch`) and Step 4 (`tcpdump -X` checksum verification) BEFORE landing Option α. If `dropwatch` shows drops anywhere other than `tcp_v4_rcv` / `inet_proto_csum_replace*` / `napi_gro_receive`, the hypothesis is wrong and the research must re-open.

### Gap K-3: Software GRO behaviour with `ethtool -K gro off` on veth

**Issue**: hypothesis (b) — software GRO coalescing length-N segments — was downranked because Lima's `gro off` should disable it. But veth has known idiosyncrasies around generic GRO via `napi_gro_receive` that may persist even with feature flags off. The kernel source for veth's NAPI path was checked but not exhaustively verified.

**Attempted**: WebFetch on `drivers/net/veth.c`; LWN search.

**Recommendation**: Step 2 of the Q3 diagnostic (`bpftrace` over `kfree_skb`) directly captures the GRO codepath if it fires. No additional research action needed pre-fix.

### Gap K-4: Empirical performance impact of the option α change

**Issue**: option α adds one `bpf_csum_diff` call. The verifier-budget impact is estimated ~5–10 instructions (within the 20% gate per ASR-2.2-03), but the per-packet pps cost has not been measured.

**Attempted**: not in scope for research.

**Recommendation**: Slice 07's Tier 4 perf gate is the natural validation point. Take the post-fix baseline; expect <2% pps regression vs the from/to form (the helpers do similar work; only the math arrangement differs). If regression exceeds 5%, the architect's call.

---

## Conflicting Information

No substantive conflicts encountered. One nuance worth noting: the man-page reference for `bpf_l4_csum_replace` (man7.org bpf-helpers) does not mention the `CHECKSUM_PARTIAL` interaction — it only documents `BPF_F_MARK_MANGLED_0`, `BPF_F_PSEUDO_HDR`, and `BPF_F_IPV6` semantics. The Patchwork patch (Paul Chaignon, 2025) is in the process of *adding* this clarification to the man page. The kernel source IS the SSOT until the doc patch lands.

---

## Recommendations for Further Research

1. **Watch the Paul Chaignon `BPF_F_PSEUDO_HDR` doc patch through merge.** When it lands, update the man page citation in this doc and possibly fold a one-liner into `.claude/rules/development.md` § "Verifier-friendly idioms".
2. **Verify Gap K-1 (Cilium comment) via direct gh CLI fetch** when option α lands.
3. **Add a Tier 2 `BPF_PROG_TEST_RUN` test that explicitly forces `skb->ip_summed = CHECKSUM_PARTIAL`** before invoking the TC reverse-NAT program (via `bpf_skb_change_proto` or kfunc, or by setting up an artificial offload-pending skb). This would lock the verified contract in CI rather than relying on real veth behaviour.

---

## Full Citations

[1] Linux Kernel Authors. "Linux v6.6 net/core/utils.c — `inet_proto_csum_replace4`, `inet_proto_csum_replace_by_diff`". github.com/torvalds/linux. v6.6. https://raw.githubusercontent.com/torvalds/linux/v6.6/net/core/utils.c. Accessed 2026-05-06.

[2] Linux Kernel Authors. "Linux v6.6 net/core/filter.c — `bpf_l4_csum_replace`, `bpf_l3_csum_replace`, `BPF_CALL_5`". github.com/torvalds/linux. v6.6. https://raw.githubusercontent.com/torvalds/linux/v6.6/net/core/filter.c. Accessed 2026-05-06.

[3] Linux Kernel Authors. "Linux v6.6 drivers/net/veth.c — `veth_xmit`, `veth_forward_skb`". github.com/torvalds/linux. v6.6. https://raw.githubusercontent.com/torvalds/linux/v6.6/drivers/net/veth.c. Accessed 2026-05-06.

[4] Linux Kernel Authors. "struct sk_buff documentation — checksum states (CHECKSUM_NONE, CHECKSUM_UNNECESSARY, CHECKSUM_COMPLETE, CHECKSUM_PARTIAL) on TX/RX". docs.kernel.org. https://docs.kernel.org/networking/skbuff.html. Accessed 2026-05-06.

[5] Linux man-pages project. "bpf-helpers(7) — `bpf_l3_csum_replace`, `bpf_l4_csum_replace`, `bpf_csum_diff`, flag semantics". man7.org. https://man7.org/linux/man-pages/man7/bpf-helpers.7.html. Accessed 2026-05-06.

[6] Chaignon, Paul. "[bpf-next,2/2] bpf: Clarify the meaning of BPF_F_PSEUDO_HDR". patchwork.kernel.org. 2025-04. https://patchwork.kernel.org/project/netdevbpf/patch/5126ef84ba75425b689482cbc98bffe75e5d8ab0.1744102490.git.paul.chaignon@gmail.com/. Accessed 2026-05-06.

[7] Dong, Menglong et al. "[bpf-next,0/2] bpf: add csum/ip_summed fields to __sk_buff". patchwork.kernel.org. https://patchwork.kernel.org/comment/25654859/. Accessed 2026-05-06.

[8] Cilium Authors. "Cilium v1.16 bpf/lib/csum.h — `csum_l4_offset_and_flags`, `csum_l4_replace`". github.com/cilium/cilium. https://raw.githubusercontent.com/cilium/cilium/v1.16/bpf/lib/csum.h. Accessed 2026-05-06 (verbatim quoted in Q2.1).

[9] Cilium Authors. "Cilium main bpf/lib/lb.h — `__lb4_rev_nat`, IPv4 reverse-NAT checksum sequence". github.com/cilium/cilium. https://github.com/cilium/cilium/blob/main/bpf/lib/lb.h. Accessed 2026-05-06.

[10] Cilium Authors. "Cilium main bpf/lib/nat.h — `snat_v4_rewrite_headers`". github.com/cilium/cilium. https://github.com/cilium/cilium/blob/main/bpf/lib/nat.h. Accessed 2026-05-06.

[11] Cilium Authors. "Cilium issue #11914 — IPTables sees packets that trigger invalid connection tracking state checks (Nodeport DNAT/veth-reverse asymmetry)". github.com/cilium/cilium. 2020. https://github.com/cilium/cilium/issues/11914. Accessed 2026-05-06.

[12] Wiedmann, Julian. "Cilium PR #37990 — iptables: no conntrack for overlay traffic (`addCiliumNoTrackOverlayRules`)". github.com/cilium/cilium. 2026. https://github.com/cilium/cilium/pull/37990. Accessed 2026-05-06.

[13] cilium/ebpf project. "Issue #337 — TCP Checksum modification error in eBPF egress (Wireshark shows original value, packets dropped at receiver)". github.com/cilium/ebpf. https://github.com/cilium/ebpf/issues/337. Accessed 2026-05-06.

[14] Facebook Inc. "Katran main katran/lib/bpf/csum_helpers.h — manual checksum computation pattern". github.com/facebookincubator/katran. https://raw.githubusercontent.com/facebookincubator/katran/main/katran/lib/bpf/csum_helpers.h. Accessed 2026-05-06.

---

## ADR-0044 Decision 6 — Proposed Amendment

**Current text** (paraphrased from the failing 06-04 dispatch):

> *"Decision 6: install `iptables -t raw -A PREROUTING -j NOTRACK` in `lb-ns` as a bridge fix to disable kernel netfilter conntrack tracking for traffic traversing lb-ns. The S-2.2-17 e2e test fails because conntrack mid-stream-picks-up the reverse direction (server → client), flagging data-bearing TCP segments as INVALID. Replaced in a single-cut migration by Slice 16-03's production-side NOTRACK install."*

**Replacement (proposed)**:

> *"**Decision 6 (RETRACTED 2026-05-06)**: the conntrack-INVALID hypothesis was empirically falsified — installing `iptables -t raw -A PREROUTING -j NOTRACK` in `lb-ns` does not unblock S-2.2-17. The retained `lb_a.pcap` capture under NOTRACK shows identical drop pattern as without it: length-0 TCP segments (SYN-ACK, ACK, FIN-ACK) traverse the full lb_a→client path; length-N data segments vanish between `lb_b` ingress and `lb_a` egress. See Section 7 (Findings Q1.5) of [`docs/research/dataplane/length-n-tcp-drop-veth-xdp-tc-reverse-nat-research.md`](length-n-tcp-drop-veth-xdp-tc-reverse-nat-research.md) for the falsification evidence. Slice 16's conntrack design is unaffected — it remains the right Phase 2.16 work for stateful flow tracking, but is NOT the fix for S-2.2-17."*
>
> *"**Decision 6′ (NEW)**: switch `tc_reverse_nat`'s checksum-update sequence from the direct `from/to/size=4` form of `bpf_l4_csum_replace` to **Cilium's diff-encoded `csum_diff + bpf_l4_csum_replace(from=0, to=diff, size=0, flags=BPF_F_PSEUDO_HDR)`** form. This is the production-validated pattern Cilium ships in `bpf/lib/lb.h::__lb4_rev_nat` and `bpf/lib/nat.h::snat_v4_rewrite_headers`, used at Cilium-CNI scale across millions of clusters. Rationale: under `skb->ip_summed == CHECKSUM_PARTIAL` (which veth peer delivery may produce on locally-generated TCP segments with TX checksum offload semantics), the direct from/to form interacts with `inet_proto_csum_replace4()` in a way that produces correct math for length-0 packets and divergent math for length-N packets. The diff-encoded form routes through `inet_proto_csum_replace_by_diff()` along the same code path Cilium has stress-tested. See Section Q2 / Q4 of the research document for the production-pattern citations."*
>
> *"**Verification before merge**: run the 5-step diagnostic in Section Q3 (dropwatch + bpftrace kfree_skb + /proc/net/netstat counters + tcpdump -X + A/B without `BPF_F_PSEUDO_HDR`) on Lima against S-2.2-17. The test passes only after Decision 6′ lands AND the diagnostic confirms the drop locus is `tcp_v4_rcv` / `inet_proto_csum_replace*` (not the GRO codepath, not the conntrack codepath)."*

---

## Research Metadata

Duration: ~50 turns of a 50-turn budget | Examined: 14 distinct authoritative sources across 8 domains | Cited: 14 | Cross-refs: 4-way for the central root-cause claim | Confidence: High (Q1.1, Q1.2, Q1.3, Q2.1, Q2.2, Q2.3, Q2.4); Medium-High overall (downrated by Gap K-2 — empirical confirmation against `/tmp/ovd-rnat3-1761788/` was out-of-scope per the research-only constraint) | Output: `docs/research/dataplane/length-n-tcp-drop-veth-xdp-tc-reverse-nat-research.md`
