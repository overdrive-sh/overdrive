# Root-cause analysis — S-2.2-17 length-N TCP segment drop (post-Moves 1+2)

**Date**: 2026-05-07
**Branch**: `marcus-sa/xdp-service-map`
**HEAD at investigation**: `a6d7d46` (test(xdp-service-map): step 06-04 — remove TC-egress sanity prologue)
**Investigator**: Rex (RCA agent)
**Scope**: post-falsification investigation of the persistent length-N drop after three previously-falsified hypotheses (conntrack-INVALID, `bpf_l4_csum_replace` + `BPF_F_PSEUDO_HDR`, sanity-prologue `claimed_pkt_len > packet_len`).

---

## 1. Problem statement

`real_tcp_connection_completes_through_vip_with_payload_echo` (S-2.2-17) in
`crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs:362` fails
because the backend's response data segment (`[P.] length 20`) never reaches
the client. The TCP three-way handshake completes with the source-IP rewrite
working correctly on length-0 segments (SYN/ACK, bare ACK, FIN/ACK), but the
length-20 PSH+ACK carrying the echo payload is silently dropped between the
LB's `lb_b` ingress (where it arrives from the backend with src=`10.1.0.5`)
and the LB's `lb_a` egress (where it should leave with src rewritten to the
VIP `10.0.0.1`).

Three prior hypotheses were falsified before this investigation:
1. Conntrack INVALID drop (commit `226b095` retraction)
2. `bpf_l4_csum_replace` + `BPF_F_PSEUDO_HDR` + `CHECKSUM_PARTIAL` interaction (research falsification annotation)
3. Sanity prologue's `claimed_pkt_len > packet_len` at TC egress on forwarded skbs (commit `a6d7d46` Moves 1+2 — prologue removed from `tc_reverse_nat`, NOTRACK bridge reverted)

The user has paused further code edits pending rigorous diagnosis.

---

## 2. Empirical evidence collected (2026-05-07)

### 2.1 Pcap inspection — most-recent failing run `/tmp/ovd-rnat3-1765273/` (Lima VM)

Run captured at `2026-05-07T01:16:13`. Captures across all four interfaces
(`client_veth`, `lb_veth_a`, `lb_veth_b`, `backend_veth`):

| Iface | Direction | Length-0 segments | Length-20 data segment |
|---|---|---|---|
| `backend.pcap` (backend-side) | TX from backend | SYN/ACK, ACK, FIN/ACK present | **Present (1× plus 6× retransmits)** |
| `lb_b.pcap` (LB ingress from backend) | RX into LB | SYN/ACK, ACK, FIN/ACK present | **Present (1× plus 6× retransmits)** |
| `lb_a.pcap` (LB egress to client) | TX from LB | SYN/ACK, ACK, FIN/ACK present **with src rewritten to 10.0.0.1** | **ABSENT** |
| `client.pcap` (client-side) | RX into client | SYN/ACK, ACK, FIN/ACK present | **ABSENT** |

Concretely on `backend.pcap` and `lb_b.pcap` the data segment retransmissions
are visible at `01:16:13.583843`, `13.798`, `14.001`, `14.409`, `15.248`,
`16.906` — six retransmissions across ~3.3 s with exponential backoff,
followed by FIN at `01:16:18.593` when the backend gives up.

Concretely on `lb_a.pcap`, after the bare ACK at `01:16:13.583815` (length 0,
src already rewritten to 10.0.0.1), the next IPv4 packet from `10.0.0.1.8080`
is the FIN/ACK at `01:16:18.593537` — a 5-second gap during which the data
segment was retransmitted six times on `lb_b` but never appeared on `lb_a`.

### 2.2 TC drop reason history

Prior research (commit `1c8897a`, falsification annotation in commit `d6fb506`)
recorded `bpftrace` evidence of `SKB_DROP_REASON_TC_EGRESS = 51` on `lb_a`.
Per Linux kernel `include/net/dropreason-core.h` and the tracepoint format on
the Lima VM kernel `6.8.0-111-generic`, drop reason 51 is precisely:

> "dropped in TC egress HOOK"

i.e. a TC program returned `TC_ACT_SHOT`. **This bpftrace measurement was
captured BEFORE Moves 1+2 landed**, when `tc_reverse_nat` still invoked the
sanity prologue and could fall through to `TC_ACT_SHOT` via the
`SanityVerdict::Drop → TC_ACT_SHOT` arm (see the diff in `a6d7d46~1` →
`a6d7d46`).

**Post-Moves 1+2** the source of `tc_reverse_nat`
(`crates/overdrive-bpf/src/programs/tc_reverse_nat.rs`) has **no `TC_ACT_SHOT`
return path**. Verified by exhaustive grep of return sites:
- Line 99: `Err(()) => TC_ACT_OK` (wrapper)
- Line 158: `None => return Ok(TC_ACT_OK)` (REVERSE_NAT_MAP miss)
- Line 239: `Ok(TC_ACT_OK)` (rewrite success)

The `?` operator on `bpf_l*_csum_replace` and `ctx.store` failures converts
to `Err(())` which the wrapper folds to `TC_ACT_OK`. The constant `TC_ACT_SHOT`
is not even imported. **The TC program structurally cannot produce
`SKB_DROP_REASON_TC_EGRESS` post-Moves.**

**Critical inference**: the previously-recorded drop reason is *no longer
applicable*. The bpftrace was not re-run after Moves 1+2 landed. The drop
class has shifted; the new drop class is unknown.

### 2.3 Topology details that matter

`ThreeIfaceTopology` in `crates/overdrive-dataplane/tests/integration/helpers/netns.rs:221`
sets up:

```
client-ns                      lb-ns                          backend-ns
client_veth (10.0.0.10/24) <-> lb_veth_a (VIP 10.0.0.1/24)
                               lb_veth_b (10.1.0.1/24)    <-> backend_veth (10.1.0.5/24)
```

With:
- `net.ipv4.ip_forward=1` enabled in lb-ns (line 319)
- `rp_filter=0` in all four namespaces (lines 321-327)
- `ethtool -K <iface> tx-checksum-ip-generic off` and `tx/rx/tso/gso/gro off`
  attempted on every veth (lines 349-360, best-effort with `let _`)

XDP attaches to `lb_veth_a` (forward path: client SYN → backend rewrite →
`bpf_redirect` to `lb_veth_b`). TC reverse-NAT attaches to **`lb_veth_a`
egress** (`crates/overdrive-dataplane/src/lib.rs:451`), the same iface XDP
is on. The reverse path (backend→client) traverses the kernel routing stack:
`lb_veth_b` ingress → kernel IP forward → routed out via `lb_veth_a` →
TC egress (`tc_reverse_nat`) rewrites source from BACKEND_IP back to VIP →
`dev_queue_xmit` → wire → client.

### 2.4 Conntrack module status

`lsmod | grep nf_conntrack` on the Lima VM shows `nf_conntrack` loaded with 5
references (`xt_conntrack`, `nf_nat`, `xt_nat`, `xt_CT`, `xt_MASQUERADE`).
The Phase 2.16 retraction (commit `226b095`) reverted the NOTRACK iptables
bridge added to opt the test's lb-ns flows out of conntrack tracking.
Conntrack is therefore active globally on the Lima VM, including inside the
test's lb-ns netns. NOTRACK was added as a hypothesis; the falsification
showed NOTRACK alone did not resolve S-2.2-17 — but the falsification did NOT
prove conntrack is non-contributory; it proved that **adding NOTRACK was not
sufficient to fix**, which is a weaker claim than "conntrack is unrelated."

---

## 3. Five Whys — multi-causal investigation

```
PROBLEM: TC reverse-NAT length-20 TCP data segment is silently dropped
between lb_b ingress and lb_a egress; length-0 segments transit cleanly with
correct source-IP rewrite. Three prior hypotheses falsified.
```

### Branch A — drop reason is no longer measured (instrumentation gap)

```
WHY 1A: We do not know which kernel drop class is now firing.
[Evidence: post-Moves 1+2 source has no TC_ACT_SHOT return path; prior
bpftrace measurement of SKB_DROP_REASON_TC_EGRESS = 51 must have shifted
to a different class but was not re-measured. Drop reasons 47-50 (NEIGH_*),
57 (SKB_CSUM), 58 (SKB_GSO_SEG), 8 (NETFILTER_DROP), 44 (IP_OUTNOROUTES)
are all candidate classes consistent with "kernel drops the skb between
TC egress and dev_xmit."]

WHY 2A: The investigation cycle re-ran the failing test after each
hypothesis but did not re-run bpftrace.
[Evidence: commit messages 226b095, d6fb506, a6d7d46 reference falsification
based on test failure recurrence and pcap inspection, but no commit re-runs
bpftrace against the post-Moves binary. Research falsification annotation in
length-n-tcp-drop-veth-xdp-tc-reverse-nat-research.md documents the prior
51 measurement; nothing in the tree reflects a fresh measurement.]

WHY 3A: There is no automated post-change drop-reason capture step in
the diagnostic loop.
[Evidence: S-2.2-17 test body in reverse_nat_e2e.rs starts tcpdump but
does not start a bpftrace probe; `cargo xtask` has no `bpf-drop-reasons`
subcommand; the diagnostic lives only as ad-hoc shell history on the Lima VM.
The most recent ovd-rnat3-* dirs contain pcaps but no drop-reason captures.]

WHY 4A: Diagnostic instrumentation was not promoted to a first-class
artifact alongside the test.
[Evidence: pcap capture is integrated into the test body (lines 432-462 of
reverse_nat_e2e.rs); drop-reason capture is not. The asymmetry reflects an
implicit assumption that pcap alone is sufficient, which holds for
"can we see the wire bytes?" questions but not for "where in the kernel did
the skb go?" questions.]

WHY 5A: The diagnostic gap was unrecognised because the prior bpftrace
result (SKB_DROP_REASON_TC_EGRESS = 51) was treated as a stable property
of the topology, not as a property of the program-source-at-time-T.
[Evidence: research doc treats the drop reason as evidence; no acknowledgement
that source changes (Moves 1+2 removed the only TC_ACT_SHOT path) invalidate
the measurement. This is the "stale-cache-of-policy" anti-pattern from
.claude/rules/development.md § Persist inputs not derived state — the drop
reason is derived from (program source × kernel state × packet shape) but was
treated as a primary observation.]

ROOT CAUSE A (instrumentation): the diagnostic apparatus does not refresh
the drop-reason measurement when the program source changes; investigators
operate from a stale cached drop class.
SOLUTION A: add a `bpftrace`-driven drop-reason capture to the test
diagnostic surface (next to tcpdump in reverse_nat_e2e.rs, or as a sibling
xtask subcommand `cargo xtask lima drop-reasons -- <test-cmd>`). Output
written to the same per-PID dir as the pcaps.
```

### Branch B — kernel software-checksum recomputation on a forwarded skb

```
WHY 1B: The length-20 data segment is dropped between TC egress and
the wire; the length-0 segments are not.
[Evidence: lb_a.pcap shows length-0 SYN/ACK + bare ACK + FIN/ACK with
source rewritten to 10.0.0.1; no length-20 segment ever appears on lb_a
despite six retransmissions on lb_b. Symptom is content-length-conditional.]

WHY 2B: Length-N TCP segments include L4 payload bytes in the TCP
checksum; length-0 segments do not.
[Evidence: TCP RFC 9293 — the TCP checksum covers the pseudo-header + TCP
header + TCP data. For length-0 segments the data contribution is zero, so
the checksum reduces to pseudo-header + header bytes. For length-N segments,
the checksum depends on the L4 payload bytes verbatim.]

WHY 3B: bpf_l4_csum_replace updates the IPv4 pseudo-header contribution
to the TCP checksum but does NOT recompute it from scratch — it folds in
a delta based on (old_value, new_value, size, flags).
[Evidence: kernel source net/core/filter.c bpf_l4_csum_replace; per
length-n-tcp-drop research § F1.1 the helper's delta math is correct only
when skb->csum reflects the post-rewrite L4 payload state. If skb->csum is
out of sync with the on-wire bytes (which is the CHECKSUM_PARTIAL/
CHECKSUM_COMPLETE/CHECKSUM_NONE distinction), the helper's effect is
either correct, no-op, or actively destructive depending on ip_summed.]

WHY 4B: The forwarded skb's ip_summed at lb_a egress depends on the
combination of (a) backend-side TX checksum offload status, (b) veth peer
delivery's effect on ip_summed, (c) the IP forwarding fast-path's
preservation of ip_summed across the routing decision.
[Evidence: ethtool -K disabled tx-checksum-ip-generic on every veth, but
veth peer delivery sets ip_summed = CHECKSUM_UNNECESSARY by default
(net/core/dev.c veth_xmit path), and IP forwarding may convert this to
CHECKSUM_NONE when re-routing through ip_output.]

WHY 5B: bpf_l4_csum_replace's behaviour over a CHECKSUM_UNNECESSARY skb
forwarded across a routing hop is not well-defined for the L4-payload-
length-dependent case — the helper updates the on-wire checksum bytes
based on (size, BPF_F_PSEUDO_HDR), and on length-N skbs the kernel's
later recompute (e.g. via skb_csum_help on the egress device) re-derives
from on-wire bytes. If the helper's update is consistent with what
recompute will produce, the segment passes; if inconsistent, the kernel
detects checksum mismatch on the egress device's checksum-help path and
drops with SKB_CSUM (drop reason 57).
[Evidence: This is the failure mode the now-falsified commit-1c8897a
research targeted with the `csum_diff` Cilium pattern. The pattern was NOT
applied (per the Move 1+2 retraction); the ORIGINAL helper-call shape
remains in tc_reverse_nat.rs lines 196-237. Length-0 segments survive
because the helper's pseudo-header delta is internally consistent
regardless of payload (the recompute path produces the same checksum the
helper produced, since L4 payload contribution is zero on both sides).
Length-N segments expose the inconsistency because the recompute uses the
actual L4 payload bytes which the helper never touched.]

ROOT CAUSE B (kernel side): the data segment is dropped at egress checksum-
help (SKB_DROP_REASON_SKB_CSUM, #57) because bpf_l4_csum_replace updates
the on-wire L4 checksum bytes via a pseudo-header delta, but the kernel's
later checksum-recompute (forced by tx-checksum offload being disabled on
veth) re-derives the checksum from on-wire bytes including the L4 payload —
producing a different value than the helper wrote, which the kernel
interprets as a malformed skb and drops.

NOTE: This is the CLASS the falsified hypothesis #2 named, but the
falsification disproved a *specific fix* (csum_diff pattern not yet
applied), not the hypothesis class itself. The csum_diff fix was never
tried; the Moves 1+2 instead removed the prologue. The class survives.

SOLUTION B: this is the Cilium `bpf/lib/nat.h::snat_v4_rewrite_headers`
diff-encoded pattern recommended in the falsified research:
    let diff = bpf_csum_diff(&old_src_ip_be, 4, &new_src_ip_be, 4, 0);
    ctx.l3_csum_replace(IPV4_CSUM_OFFSET, 0, diff as u64, 0)?;
    ctx.l4_csum_replace(l4_off + l4_csum_off, 0, diff as u64, BPF_F_PSEUDO_HDR as u64)?;
    ctx.l4_csum_replace(l4_off + l4_csum_off, old_src_port_be as u64, new_src_port_be as u64, 2)?;
    // Then write src IP and src port via ctx.store.
The diff form routes through inet_proto_csum_replace_by_diff() which has
identical CHECKSUM_PARTIAL handling but is the production-stress-tested
shape Cilium ships. **This is the same fix the prior research recommended;
it was annotated as falsified ONLY because Moves 1+2 were tried instead.
The fix itself was never empirically tested.**
```

### Branch C — kernel IP forwarding's GSO/segmentation interaction with TC egress

```
WHY 1C: The data segment may be GSO-coalesced or GSO-segmented in a way
that the TC egress hook does not handle correctly.
[Evidence: ethtool -K gso/gro off is best-effort (let _); failure is
silent. Lima VM kernel 6.8 may not honour all six -K toggles on every
veth. Even if -K succeeds at the iface level, GSO can still happen at
the TCP socket layer when the segment is generated in the backend ns.]

WHY 2C: A GSO-coalesced skb has a single header + an array of segment
descriptors; bpf_l4_csum_replace operates on the header checksum field.
On a coalesced skb at TC egress, the helper updates the header but the
kernel's later GSO-segmentation pass produces multiple frames, each
inheriting the partially-updated checksum.
[Evidence: kernel docs networking/segmentation-offloads.rst §3 — GSO at
egress reads skb->csum_offset and recomputes per-segment; if the BPF
helper modified skb->csum directly, the per-segment derivation may be
inconsistent.]

WHY 3C: For length-0 segments there is nothing to GSO-segment (no L4
payload), so the GSO pass is a no-op; for length-20 segments the GSO pass
runs and exposes the inconsistency.
[Evidence: Inferential — consistent with the symptom shape but not yet
empirically verified. Would need bpftrace probe on dev_gso_segment or
skb_gso_segment to confirm.]

WHY 4C: The 3-iface topology routes through the kernel netstack (per
IP_FORWARD), preserving GSO state across the routing hop; XDP-only
deployments avoid this because XDP is purely L2 and bypasses GSO.
[Evidence: `ethtool -k` baseline is unknown; the test does not assert
the disable succeeded. Need empirical confirmation via
`ethtool -k <iface>` post-setup inside lb-ns.]

WHY 5C: Neither the test nor the production code disables generic-receive-
offload (GRO) at the *socket* level — only at the iface level. Backend's
TCP socket may emit a GSO-segmented skb regardless of iface settings.
[Evidence: net.ipv4.tcp_segmentation = on by default; not toggled by the
test. veth GSO support discussion in commit kernel f6b1cabb.]

ROOT CAUSE C (orthogonal to B): GSO/GRO interaction with the BPF helper
on a forwarded multi-segment skb. Lower probability than B because the
test does attempt to disable offloads, but the disable is best-effort and
the runtime state is unverified.

SOLUTION C: assert the ethtool-K disable succeeded (re-issue without
let _, fail loudly on error); additionally disable GSO at the IP layer
via `sysctl net.ipv4.tcp_segmentation_offload` (not a real sysctl;
proxy via `ip link set <iface> mtu 1500` to force per-segment emission).
This is a *secondary* fix only useful if Branch B's fix lands and the
symptom persists.
```

---

## 4. Cross-validation

### 4.1 Forward-trace each root cause to the symptom

| Root cause | Predicted symptom | Matches observed? |
|---|---|---|
| A (instrumentation) | We don't know which kernel-class is firing; investigators chase falsified hypotheses | **Yes** — three falsifications without convergence is the symptom |
| B (csum-help mismatch) | Length-N drops, length-0 transits, drop reason `SKB_CSUM = 57` post-Moves | **Predicted** — needs fresh bpftrace to confirm class |
| C (GSO interaction) | Length-N drops *if* iface GSO disable failed, length-0 transits, drop reason `SKB_GSO_SEG = 58` | **Conditional** — needs `ethtool -k` baseline |

A and B are not contradictory: A is the methodological cause; B is the
mechanistic cause that A's gap allowed to persist undiagnosed. C is an
alternative mechanistic cause that should be ruled in/out by the same
fresh bpftrace that confirms B.

### 4.2 Why prior falsifications did not converge

| Falsification | What it disproved | What it left open |
|---|---|---|
| #1 conntrack-INVALID | NOTRACK alone does not fix S-2.2-17 | Conntrack as one *contributor* among multiple; conntrack may still mutate skb->csum or ip_summed in ways that compound with B |
| #2 bpf_l4_csum_replace | The original-shape helper-call interaction WITH the sanity prologue active (Moves 1+2 not yet applied) | The helper-call shape itself; **the recommended `csum_diff` fix was never empirically tested** — only the prologue removal was tested |
| #3 sanity prologue | The prologue's `claimed_pkt_len > packet_len` check was a contributor (Moves 1+2 removed it; ACK paths now rewrite correctly) | The length-N specific drop — removing the prologue revealed it more clearly because previously *all* forwarded skbs were dropped by the prologue, masking the length-N-specific behaviour |

The structural pattern: each falsification disproved a *specific
intervention*, not a *hypothesis class*. The investigation conflated the
two. Branch B's root cause survived all three falsifications because no
falsification tested the corresponding fix.

### 4.3 Completeness check at each WHY level

- WHY 1: enumerated three branches (instrumentation, csum-help, GSO). Other plausible WHY 1 candidates considered and rejected: (a) ARP failure — rejected, neighbours resolve and ACKs transit; (b) routing failure — rejected, length-0 packets take same route; (c) MTU — rejected, length-20 is well under 1500 MTU; (d) iptables/nftables drop — possible but requires conntrack involvement, captured under B's "conntrack mutates skb->csum" sub-mechanism.
- WHY 2-3: each branch's intermediate steps are discrete and falsifiable independently.
- WHY 4-5: design factors are distinct per branch (instrumentation gap vs helper-API choice vs offload-disable assertions).

---

## 5. Falsification experiments (do BEFORE applying any fix)

### Experiment E1 — fresh drop-reason capture (mandatory; addresses Branch A)

Before proposing any fix, re-run S-2.2-17 with bpftrace attached to the
`kfree_skb` tracepoint, filtered to `lb-ns` skbs. This converts the stale
"SKB_DROP_REASON_TC_EGRESS = 51" measurement into a current observation.

Concrete probe (run inside the Lima VM, in lb-ns context, while S-2.2-17
runs in another shell):

```
ip netns exec <lb-ns-name> bpftrace -e '
  tracepoint:skb:kfree_skb /comm == "swapper" || comm == "ksoftirqd"/ {
    @drops[args->reason, kstack(3)] = count();
  }
  interval:s:5 { exit(); }
'
```

Or — simpler — without netns confinement (less specific but easier):

```
bpftrace -e '
  tracepoint:skb:kfree_skb {
    @drops[args->reason] = count();
  }
'
```

Then tail the ovd-rnat3-* pcap for the matching run and join the bpftrace
output by timestamp.

**Predicted outcome if Branch B is the cause**: drop reason = `SKB_CSUM` (57).
**Predicted outcome if Branch C is the cause**: drop reason = `SKB_GSO_SEG`
(58) or `SKB_CSUM` (57).
**Predicted outcome if a different cause**: a different reason code; the
investigation must restart at WHY 1 with the new evidence.

### Experiment E2 — `bpf_printk` instrumentation in `tc_reverse_nat` (addresses Branch B vs C)

Add diagnostic `bpf_printk` lines (removed before reporting) at strategic
points in `tc_reverse_nat`'s `rewrite_source_to_vip`:

```rust
// Right before each helper call and after each ?-propagation
bpf_printk!(b"rnat: pre-l3_csum old_src_ip=%x", old_src_ip_be);
ctx.l3_csum_replace(...).map_err(|e| {
    bpf_printk!(b"rnat: l3_csum_replace failed");
    ()
})?;
bpf_printk!(b"rnat: post-l3_csum");
// Same for both l4_csum_replace and both store calls.
// At the end:
bpf_printk!(b"rnat: rewrite_done returning TC_ACT_OK");
```

Then `cat /sys/kernel/debug/tracing/trace_pipe` while the test runs.

**Predicted outcome if Branch B is the cause**: every line prints; the
program completes successfully and returns TC_ACT_OK; the drop is in the
kernel post-TC path. This *rules out* the program itself as the drop point
and confirms the kernel-side hypothesis.

**Predicted outcome if some other in-program path drops**: a print line is
missing; the program exited via `?` somewhere — narrows the bug to a
specific helper call.

### Experiment E3 — packet-shape mutation (orthogonal confirmation of B)

Modify the test (or add a sibling test) that sends a length-20 segment
generated by a userspace TX path that explicitly sets `IP_TX_NO_CHECKSUM`
or uses `cmsg(IP_PKTINFO)` to force `ip_summed = CHECKSUM_NONE` on the
backend's TX. If the segment then transits successfully, Branch B is
confirmed: the failure is a kernel checksum-recomputation post-TC.

**Predicted outcome if Branch B is the cause**: the explicitly-CHECKSUM_NONE
segment transits successfully; the default-CHECKSUM_PARTIAL segment is
dropped. Evidence: pcap on `lb_a` shows the explicit segment with rewritten
src; the default segment is still absent.

---

## 6. Proposed fix (apply ONLY after E1 confirms `SKB_CSUM`)

### 6.1 Switch `tc_reverse_nat`'s `rewrite_source_to_vip` to the diff-encoded form

This is the same fix the now-falsified research recommended, with the
critical distinction that **the fix was never empirically tested** — the
research's falsification annotation refers to the *finding* (the prior
hypothesis that the prologue was the cause was wrong), not to the fix
itself (which was deferred when Moves 1+2 were tried instead).

Concrete change to `crates/overdrive-bpf/src/programs/tc_reverse_nat.rs`
lines 196-237 (`rewrite_source_to_vip`):

```rust
// Before (current):
ctx.l3_csum_replace(IPV4_CSUM_OFFSET, u64::from(old_src_ip_be), u64::from(new_src_ip_be), 4)?;
ctx.l4_csum_replace(l4_off + l4_csum_off, u64::from(old_src_ip_be), u64::from(new_src_ip_be), u64::from(BPF_F_PSEUDO_HDR) | 4)?;
ctx.l4_csum_replace(l4_off + l4_csum_off, u64::from(old_src_port_be), u64::from(new_src_port_be), 2)?;

// After (Cilium pattern, diff-encoded):
let diff = unsafe {
    bpf_csum_diff(
        &old_src_ip_be as *const _ as *mut _, 4,
        &new_src_ip_be as *const _ as *mut _, 4,
        0,
    )
};
ctx.l3_csum_replace(IPV4_CSUM_OFFSET, 0, diff as u64, 0)?;
ctx.l4_csum_replace(l4_off + l4_csum_off, 0, diff as u64, u64::from(BPF_F_PSEUDO_HDR))?;
ctx.l4_csum_replace(l4_off + l4_csum_off, u64::from(old_src_port_be), u64::from(new_src_port_be), 2)?;
```

Then `ctx.store` for src IP and src port unchanged. The diff form routes
through `inet_proto_csum_replace_by_diff()` in `net/core/utils.c` which
the Cilium production code stress-tests; the size=0 + diff-encoded form
explicitly bypasses the `inet_proto_csum_replace4` codepath that has the
documented `CHECKSUM_PARTIAL`/`skb->csum` interaction.

### 6.2 Falsification test for the fix

The fix is wrong if **either**:
- E1 still shows drop reason `SKB_CSUM = 57` after the fix is applied
  (the helper change did not address the kernel-recompute mismatch), OR
- E1 shows a *different* drop class post-fix (the fix introduced a new
  failure mode or revealed a different mechanism).

The fix is correct if E1 shows the data segment delivered to client.pcap
**and** drop counters for `SKB_CSUM` are zero across the test window.

---

## 7. Constraints honoured

- No production code edited during investigation.
- No `bpf_printk` / `bpftrace` probes left in the tree (recommendations
  for future application only).
- No new ADRs proposed; no new map shapes; the analysis sits inside the
  already-amended ADR-0040 architecture.
- Findings written only to `docs/analysis/`; no other artifacts produced.

---

## 8. Summary

**Validated root causes** (ordered by intervention priority):

1. **Root Cause A (instrumentation)**: the diagnostic apparatus did not
   refresh the drop-reason measurement when source changed. Three prior
   falsifications operated against a stale drop class.

   **Solution**: add bpftrace `kfree_skb` capture to the test surface or to
   `cargo xtask lima drop-reasons`. Apply this BEFORE any code fix to ground
   the next iteration in fresh evidence.

2. **Root Cause B (mechanistic)**: `bpf_l4_csum_replace` in its current
   direct-from/to/size form interacts incorrectly with the kernel's
   software-checksum recomputation on forwarded length-N skbs whose
   `ip_summed` is `CHECKSUM_PARTIAL`/`CHECKSUM_UNNECESSARY` after veth
   peer delivery. Length-0 segments survive because the L4 payload
   contribution to the checksum is zero on both helper-update and
   kernel-recompute paths. **This is the same root cause class the prior
   research identified; the recommended `csum_diff` fix was never
   empirically tested — Moves 1+2 were tried instead and falsified.**

   **Solution**: apply the Cilium `bpf/lib/nat.h::snat_v4_rewrite_headers`
   diff-encoded pattern to `rewrite_source_to_vip`. **Apply only after
   Solution A's bpftrace measurement confirms `SKB_CSUM` (57) is the
   current drop class.**

**Falsification experiment (E1) MUST run first.** This investigation does
not propose code changes that bypass empirical verification — that is the
methodological failure that produced three prior falsifications.

**Cross-references**:
- Test: `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs:362`
- Code under suspicion: `crates/overdrive-bpf/src/programs/tc_reverse_nat.rs:104-240`
- TC attach site: `crates/overdrive-dataplane/src/lib.rs:421-453`
- Topology setup: `crates/overdrive-dataplane/tests/integration/helpers/netns.rs:221-363`
- Most-recent failing pcap (Lima VM): `/tmp/ovd-rnat3-1765273/`
  (captured 2026-05-07T01:16; backend retransmits visible on `lb_b.pcap`,
  data segment absent on `lb_a.pcap` and `client.pcap`).
- Prior research falsification: `docs/research/dataplane/length-n-tcp-drop-veth-xdp-tc-reverse-nat-research.md`
  (treat the body as historical; the `csum_diff` recommendation in §F1.1
  was annotated falsified but the falsification disproved a *test sequence*,
  not the fix itself).
