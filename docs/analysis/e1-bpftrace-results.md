# E1 results — fresh kfree_skb capture during S-2.2-17

**Date**: 2026-05-07
**Kernel**: 6.8.0-111-generic (Lima VM, ubuntu-24.04 base)
**Test**: `real_tcp_connection_completes_through_vip_with_payload_echo` (FAIL, exit=100, 6.7s)
**Scripts**: `.context/e1-run.sh`, `.context/e2-run.sh`, `.context/e2b-run.sh`

## Headline

E1 confirmed the drop is real and reproducible (6 drops per failing run, matching
the 6 backend retransmissions). The drop reason **is** still `SKB_DROP_REASON_TC_EGRESS = 51`
on the live post-Moves binary — the same numeric class the RCA argued must have
shifted. **The RCA's structural premise (no `TC_ACT_SHOT` path → reason 51 impossible)
is contradicted by the empirical signal.**

E2/E2b deepened the picture: neither `tcf_classify` (legacy classifier path) nor
`tc_run` (TCX 6.6+ dispatcher) recorded our TC program returning `TC_ACT_SHOT`.
`tc_run` fired 7 times during the test window and **every call returned
`TC_ACT_UNSPEC = -1`**, never `TC_ACT_SHOT = 2`. Yet 6 packets were dropped with
reason 51 from the IP-forward → `__dev_queue_xmit` egress path.

The methodological gap the RCA flagged is real — the prior bpftrace measurement
was indeed stale — but resolving the gap does not vindicate the
`bpf_l4_csum_replace` / `csum_diff` hypothesis. It opens a different question:
**why does the kernel attribute reason 51 to packets when neither `tcf_classify`
nor `tc_run` reports a SHOT verdict?**

## Probe matrix and results

### E1 — kfree_skb tracepoint, reason aggregation

```
@reasons[51]: 6                            # SKB_DROP_REASON_TC_EGRESS
@reasons[2]: 1564                          # NOT_SPECIFIED (background noise:
                                           #   netlink, ipv6 mcast, unix sockets,
                                           #   veth setup — unrelated to test)
```

Kstack for the 6 reason-51 drops (deepest first):
```
__traceiter_kfree_skb+88
kfree_skb_reason+240
__dev_queue_xmit+1276
neigh_resolve_output.part.0+216
neigh_resolve_output+80
ip_finish_output2+520        (visible in E2)
__ip_finish_output+204       (visible in E2)
```

The path is unambiguous: kernel IP forwarder → `ip_finish_output2` → neighbour
resolution → `__dev_queue_xmit` → drop with TC_EGRESS attribution.

**Drop reason 51 → `TC_EGRESS` confirmed live** by reading
`/sys/kernel/debug/tracing/events/skb/kfree_skb/format` on this kernel —
not stale lookup-table reasoning.

### E2 — `kretprobe:tcf_classify`

```
@classify_retvals: <empty>
```

`tcf_classify` did not fire during the test. Either inlined (likely on 6.8
with TCX), or the project's TC attach goes via a path that doesn't traverse it
(also likely — TCX dispatch goes via `tc_run`).

### E2b — `kfunc:tc_run` + `kretfunc:tc_run`

```
@tc_run_entries: 7                         # tc_run called 7 times
@tc_run_retvals[-1]: 7                     # ALL returned TC_ACT_UNSPEC
@tc_run_skb_lens[28]: 2                    # ARP-shaped frames
@tc_run_skb_lens[56]: 2                    # TCP control segments
@tc_run_skb_lens[76]: 3                    # TCP segments w/ options
@drops_51: 6                               # confirmed 6 drops same window
```

**Critical**: of the 7 tc_run invocations, every one returned `TC_ACT_UNSPEC`.
Zero `TC_ACT_SHOT`. Zero `TC_ACT_OK` (0).

Two readings of `TC_ACT_UNSPEC`:
- **Path A**: `tc_run` was called on packets that did NOT match
  `tc_reverse_nat`'s ingress filter — but `tc_run` is per-program, not
  per-packet-class, so this is unlikely.
- **Path B**: `tc_run` was called for SOMETHING ELSE on the host (host-side
  systemd-networkd, NetworkManager, default qdisc on the host's outer NIC),
  not for the test's `lb_veth_a` egress. UNSPEC is the default
  no-classifier-matched return.

The 7 events at lengths {28, 56, 76} also do not look like the test's reverse
path (which would carry length-20 TCP payloads + IPv4/TCP headers ≈ 40 bytes
of payload + headers ≈ 74 bytes on wire). The lengths suggest these
`tc_run` events are coming from elsewhere, not the test.

## Where this leaves the hypothesis space

The RCA's Branch B (csum_diff fix) was predicated on:
1. drop reason being SKB_CSUM=57 post-Moves, and
2. `bpf_l4_csum_replace`'s direct-from/to form misbehaving on
   CHECKSUM_PARTIAL skbs.

**Neither condition is corroborated** by E1/E2/E2b:
- (1) is falsified — drop reason is 51, not 57.
- (2) is unverifiable from current evidence — we don't even know if
  `tc_reverse_nat` runs on the dropped packets.

The new question dominating the next iteration: **does `tc_reverse_nat` actually
execute on the 6 dropped packets, and what verdict does it return?** Three
ways to find out, ordered by invasiveness:

1. **`bpf_printk!` instrumentation in `tc_reverse_nat`** (RCA's E2; requires
   editing kernel-side BPF source). Confirmed signal at minimal risk; the
   prints are removed before reporting. Catches the verdict at every entry
   and exit point of the program. (Code change.)

2. **Trace `bpf_prog_run` directly for the loaded `tc_reverse_nat` prog_id.**
   Requires capturing the prog_id (`bpftool prog show | grep classifier`)
   inside the netns mid-test, then attaching `fentry`/`fexit` to that
   specific program by tag. (Test-runtime probe; no source change.)

3. **Capture `tc filter show dev lb_veth_a egress` mid-test** to verify what
   filters are actually attached and active during the failing window. The
   netns is ephemeral and torn down at test exit, so this requires either a
   sidecar shell that races the test or an instrumentation hook in the test
   body itself. (Lightweight; tells us if the qdisc/filter shape is what
   the loader believed it set up.)

(2) and (3) are non-invasive and should run before any code change. (1) is
the definitive answer but requires touching `tc_reverse_nat.rs`. Doing
(2)+(3) first is consistent with the RCA's "don't fix without empirical
verification" discipline.

## What's NOT a candidate fix

- The `csum_diff` Cilium pattern from §6 of the RCA was the only proposed
  fix. **It would not be applied at this point** — the empirical evidence
  has shifted away from a checksum hypothesis. Applying it now would be
  another iteration of "test sequence falsifies a specific intervention,
  not the hypothesis class." The actual mechanism is unidentified.
- The sanity-prologue revert (Moves 1+2) stands — those were correctness
  fixes for ADR-0040 Q3 amendment, not S-2.2-17 fixes. They moved the
  symptom into clearer view; they did not introduce it.

## Summary for the next iterator

| Question | Answer |
|---|---|
| Is the drop reproducible? | Yes, 6 drops every failing run |
| Does the drop class match the RCA's prediction (SKB_CSUM=57)? | **No — still 51 (TC_EGRESS)** |
| Does the post-Moves source have a `TC_ACT_SHOT` path? | **No** (verified by re-grep) |
| Does `tcf_classify` see SHOT verdicts? | **No** — kretprobe shows zero invocations |
| Does `tc_run` see SHOT verdicts? | **No** — 7 invocations, all UNSPEC |
| Does `tc_reverse_nat` run on the dropped packets? | **Unknown — primary question** |
| Should the `csum_diff` fix be applied now? | **No — empirical premise has changed** |

Recommended next step: probe (2) + probe (3) above, in that order.

## Probe 3+2 results

**Date**: 2026-05-07 (continuation)
**Scripts**: `.context/probe3-run.sh`, `.context/probe2-run.sh`

### Headline

**`tc_reverse_nat` is loaded into the kernel but NOT attached to any qdisc.**
The clsact qdisc exists on `lb_veth_a` (egress hook is present), but the
filter list is empty. The 6 reason-51 drops therefore cannot have been
caused by `tc_reverse_nat` returning `TC_ACT_SHOT` — it is structurally
not in the packet path on the failing window.

### Probe 3 — TC qdisc/filter snapshots (lb-ns interior)

Captured ~14 snapshots through the 6.7 s test window inside
`3i-lb-a-<pid>` against `lb_veth_a` (`3iaafc8f@if682`). Steady-state shape
from iter ≥ 3 onward:

```
ip -o link show
  3iaafc8f@if682: ... xdp ... prog/xdp id 5130 name xdp_service_map ...
  3iabfc8f@if683: ... (no XDP, no TC)

tc qdisc show dev 3iaafc8f
  qdisc noqueue 0: root refcnt 2
  qdisc clsact ffff: parent ffff:fff1     ← clsact present, but...

tc filter show dev 3iaafc8f egress        ← EMPTY (no filters)
tc filter show dev 3iaafc8f ingress       ← EMPTY (no filters)

tc -s qdisc show dev 3iaafc8f
  qdisc clsact ffff: parent ffff:fff1
   Sent 0 bytes 0 pkt (dropped 0, overlimits 0 requeues 0)
   backlog 0b 0p requeues 0
```

The `clsact` qdisc is installed (this is the modern TCX-and-classic-TC
hook anchor). No filters are bound to either the egress (`ffff:fff3`)
or ingress (`ffff:fff2`) handles. From the kernel's perspective, no
program runs on TC for this iface.

### Probe 2 — bpftool prog inspection (system-wide)

`tc_reverse_nat` IS loaded — confirmed by `bpftool prog list`:

```
5141: sched_cls  name tc_reverse_nat  tag dfbac4d4cd66059e  gpl
  loaded_at 2026-05-07T01:50:34+0700  uid 0
  xlated 1528B  jited 1176B  memlock 4096B  map_ids 5630,5632
```

The xlated dump (first 120 instructions) shows real bytecode — header
parsing, map lookups against the two map_ids (5630 = a percpu_array,
5632 = a hash_map; consistent with DROP_COUNTER + the reverse-NAT lookup
key). The program is real, JIT-compiled, and references the right maps.

But the kernel-wide attach view is empty:

```
bpftool net show
  xdp:
  tc:
  flow_dissector:
  netfilter:
```

Even `xdp_service_map` (id 5130, attached to `lb_veth_a`) is missing
here — `bpftool net show` is netns-scoped to the netns of the calling
process and the sidecar runs in the host netns. The `xdp_service_map`
attach IS confirmed via `ip -o link show` inside the lb-ns (probe 3).
The same logic applies symmetrically: a TC attachment, if it existed
inside the lb-ns, would surface in `tc filter show ... egress` inside
that netns. It does not.

### Cross-checks

- **Per-iter consistency**: across all 14 iterations of probe 3 (covering
  the entire test window, ~5.5 s after first sighting), the egress
  filter list was empty every single time. There is no transient-attach
  / detach race we missed.
- **Lifecycle**: the prog (id 5141) was loaded *before* the test
  attached to `clsact` (probe 2 captured it at iter=6, ts ~ T+1.6s
  after sidecar start) and was unloaded only when the test process
  exited (post-mortem: `Error: get by id (5141): No such file or
  directory`).
- **No alternate filters**: nothing else attaches to either
  `lb_veth_a`'s clsact egress or ingress hooks. Whatever caused the
  6 drops with reason 51 is not coming from a stray-filter source.

### Bottom-line answer to the framing question

> Did `tc_reverse_nat` execute on the 6 dropped packets, and what
> verdict did it return?

**It did not execute.** There is no path through the kernel by which it
could have — clsact has no bound filter on the egress hook. The 6
drops with reason `SKB_DROP_REASON_TC_EGRESS = 51` are firing from
somewhere else in the egress path (or the reason code is being
attributed by a path that bypasses our program entirely — see e.g.
`__dev_queue_xmit`'s own tc handling vs `tcf_classify`).

This refutes the implicit assumption that ran through E1/E2/E2b:
"`tc_reverse_nat` returned UNSPEC on every call we saw because the
test invocations went through other interfaces." A simpler reading
matches every datapoint: **the program was never attached to the
test's interface in the first place**.

### Implications

1. The atomic-swap / hand-rolled HoM machinery loaded the program
   correctly — `bpftool prog show` confirms the bytecode is valid and
   the maps are real.
2. The clsact qdisc was created — confirmed by `tc qdisc show`.
3. The **`tc filter add ... bpf da obj ... sec classifier`** step (or
   its aya equivalent — `SchedClassifier::attach(iface,
   TcAttachType::Egress)`) **either was never called for this iface,
   or it failed silently and the test continued**.

The next minimum-invasive diagnostic: read the test's setup code path
(`reverse_nat_e2e.rs::361..566`) and the dataplane's TC-attach helper
(somewhere in `crates/overdrive-dataplane/src/`) and confirm whether
`SchedClassifier::attach` is invoked, and whether its return is
checked. Code change needed: none — this is a read-only audit.

If the attach is missing or its error is swallowed, the bug is a
control-plane wiring defect, not a kernel-side BPF defect, and the
csum/conntrack hypothesis branches are entirely orthogonal.

### Summary table

| Question (from probe brief) | Answer |
|---|---|
| Is `tc_reverse_nat` attached to `lb_veth_a` egress? | **No.** clsact present, filter list empty across the entire test window. |
| What qdisc/filter chain is live? | `clsact ffff: parent ffff:fff1` only. No filters on either hook. |
| Any unexpected filters? | None. |
| Did `tc_reverse_nat` run on the 6 dropped packets? | **No** — structurally cannot have, see above. |
| Minimum-invasive next step? | Read-only audit of the test setup + dataplane attach helper to find the missing/silently-failing `SchedClassifier::attach`. |
| Stale state from prior runs? | No — netns is freshly created per test (helpers/netns.rs:84 best-effort cleanup, then `ip netns add`). |

## Probe 4+5 results

**Date**: 2026-05-07 (continuation)
**Scripts**: `.context/probe4-run.sh`, `.context/probe5-run.sh`

### Headline

**Probe 3's "not attached" conclusion was wrong — it used a legacy-only
inspection (`tc filter show`) that does NOT surface TCX attachments.**
On kernel 6.8.0-111-generic, aya 0.13.x's `SchedClassifier::attach`
goes through `bpf_link_create` with `BPF_TCX_EGRESS` rather than
the legacy clsact filter machinery. `tc_reverse_nat` IS attached
the entire test window, on `lb_veth_a` egress, via TCX. And it
runs 16 times during the failing test — measured directly via
`kernel.bpf_stats_enabled` + `bpftool prog show id <N> --json`.

The drop is downstream of `tc_reverse_nat`. The kstack from E1
points at `neigh_resolve_output.part.0+216` → `__dev_queue_xmit`
→ `kfree_skb_reason(skb, 51 /* SKB_DROP_REASON_TC_EGRESS */)`,
which on kernel 6.8 is the path the egress runs through after a
TCX program returns `TC_ACT_OK`. The numeric reason 51 is the
kernel's attribution for "dropped on the TC egress path,"
including kernel-internal post-TCX drops — not necessarily a
SHOT verdict from the loaded program.

### Probe 4 — TCX-aware attachment (definitive)

`bpftool net show` and `bpftool link show` inside `lb-ns`,
captured 15 times across the failing window. From every snapshot
(steady state):

```
nsenter --target $(ip netns pid 3i-lb-a-1770020) --net bpftool net show
  xdp:
  3iaa0224(689) driver id 5150
  tc:
  3iaa0224(689) tcx/egress tc_reverse_nat prog_id 5151 link_id 489
  flow_dissector:
  netfilter:

bpftool link show
  ...
  488: xdp  prog 5150
       ifindex 3iaa0224(689)
  489: tcx  prog 5151
       ifindex 3iaa0224(689)  attach_type tcx_egress
```

The exact `bpftool net show` line answering "is it attached":

> `3iaa0224(689) tcx/egress tc_reverse_nat prog_id 5151 link_id 489`

Per-iface confirmation: TCX attachment is on `lb_veth_a`
(`3iaa0224`) ONLY. `lb_veth_b` (`3iab0224`, the backend-facing
veth) has no TC or XDP attached — consistent with the dataplane
loader at `crates/overdrive-dataplane/src/lib.rs:432-453`, which
attaches both XDP and TC to the same `iface` parameter the test
passes (`&topo.lb_veth_a` per `reverse_nat_e2e.rs:500`).

The legacy `tc filter show ... egress` panel in the same probe
is empty — confirming probe 3's reading is consistent with TCX
(not legacy clsact). Both views are correct simultaneously; they
inspect different kernel data structures.

### Probe 5 — invocation count via BPF stats

bpftrace's `kfunc:bpf:` probes require BTF for the BPF program,
which user-loaded JIT'd progs don't carry. `kprobe` on
`bpf_prog_<TAG>_tc_reverse_nat` returns `Invalid argument` —
the kernel rejects kprobe attachment to JIT'd BPF code. The
canonical signal is `kernel.bpf_stats_enabled=1` + per-program
`run_cnt` exposed via `bpftool prog show id <N> --json`.

The sidecar enabled stats, polled `run_cnt` every 50 ms during
the test window, restored stats afterwards. Trajectory (only
records on counter change):

| ts (rel)   | run_cnt | delta | run_time_ns |
|------------|---------|-------|-------------|
| T+0.000s   |    0    |   0   |       0     |
| T+0.058s   |    5    |  +5   |    7959     |
| T+0.243s   |    6    |  +1   |    9750     |
| T+0.450s   |    7    |  +1   |   14125     |
| T+0.607s   |    9    |  +2   |   20833     |
| T+0.904s   |   10    |  +1   |   24958     |
| T+1.202s   |   11    |  +1   |   29250     |
| T+1.669s   |   12    |  +1   |   36334     |
| T+3.361s   |   13    |  +1   |   40251     |
| T+5.034s   |   14    |  +1   |   52376     |
| T+5.115s   |   16    |  +2   |  119835     |

**Final `run_cnt` = 16.** Average run_time across 16 invocations
= ~7.5 µs (last delta is anomalously high — 67 µs across the
final +2; possibly cold-cache effect on test teardown). The
program executed; it was not bypassed.

The temporal shape matches a SYN/SYN-ACK + 6 backend
retransmissions + handshake/teardown packets:
- 5 invocations in the first 60 ms (TCP three-way + early
  data + ACK in fast succession)
- ~1 invocation per second for ~5 s (matches the kernel's
  TCP retransmission backoff schedule)
- Final +2 at teardown

The 6-retransmission pattern from E1's drop trace lines up
neatly with the 6 incremental +1 deltas between snap=2 (run_cnt=5)
and snap=68 (run_cnt=14) covering the visible retransmit cadence.

### What we do NOT have

- **Per-verdict counts.** `run_cnt` is unconditional; it does
  NOT separate `TC_ACT_OK` from `TC_ACT_SHOT`. The DROP_COUNTER
  PerCpuArray (which the program writes to on `TC_ACT_SHOT`)
  would give us this, but it dies with the prog at test exit
  and the sidecar didn't snapshot it during the window.
- **Per-skb-length distribution.** We could not get it without
  BTF or program edits.

If a verdict breakdown is needed, the next move is either
(a) read DROP_COUNTER inside the test process before teardown
(code change to the test) or (b) add a one-line `bpf_printk!`
on the `TC_ACT_SHOT` path of `tc_reverse_nat` (kernel-side BPF
edit). Both are "next iteration" work, not part of this probe.

### Where this leaves the failure mode

The kstack from E1 already pointed at:

```
__dev_queue_xmit+1276
neigh_resolve_output.part.0+216
neigh_resolve_output+80
ip_finish_output2+520
__ip_finish_output+204
```

Combined with probe 4+5: the program runs (16 times), TCX
attachment is correct, and the drop happens AFTER the TCX
program returns to the egress path. `neigh_resolve_output`
is the Layer 2 neighbour-resolution stage — converting the
next-hop IPv4 address to a destination MAC via the ARP cache.

The 6 drops at reason 51 are likely from
`neigh_resolve_output` failing — most plausibly because the
post-rewrite source IP doesn't have a working ARP entry on
`lb_veth_a` to drive the next hop. Other plausible suspects
on this kstack:

- **MAC-source mismatch.** `tc_reverse_nat` rewrites IP src
  (VIP→backend); if it doesn't also rewrite Ethernet src to
  match `lb_veth_a`'s MAC, the egress driver may drop with
  reason 51 when the L2 stack can't resolve the resulting
  pseudo-frame.
- **MTU.** `lb_veth_a` MTU vs the rewritten packet size; reason
  51 is sometimes attributed to MTU-class drops on the egress
  path. Less likely given 76-byte test packets.
- **ARP for the rewritten dest.** The pre-populated ARP from
  step 4 of the test (`reverse_nat_e2e.rs:465`) is on
  `lb_veth_b` (backend-facing) — the reverse-NAT response
  rewrites to a different src, and `lb_veth_a`'s ARP table
  may not have what the kernel needs to send the response
  client-ward.

`neigh_resolve_output` failures are catchable by `pwru`
(the development.md § "Debugging real-kernel failures" tool)
or by enabling `tracepoint:net:netif_rx` and examining
the skb at the moment of the kfree.

### Updated summary table

| Question | Answer |
|---|---|
| Is `tc_reverse_nat` attached via TCX in `lb-ns`? | **Yes.** `3iaa0224(689) tcx/egress tc_reverse_nat prog_id 5151 link_id 489` (link_id varies per run; tag `dfbac4d4cd66059e` is content-addressed, stable across runs of the same bytecode). |
| Was probe 3's conclusion correct? | **No.** Probe 3 inspected legacy clsact filters only; TCX is a separate kernel attach surface invisible to `tc filter show`. |
| Did `tc_reverse_nat` run during the test? | **Yes — 16 times** per `kernel.bpf_stats_enabled` + `bpftool prog show id <N> --json` polling. |
| Per-verdict count (OK vs SHOT)? | Not measured by this probe. Run-cnt is unconditional. |
| Where is the drop, given TC ran? | Downstream — kstack from E1 points at `neigh_resolve_output.part.0+216` → `__dev_queue_xmit`. Kernel attributes reason 51 to post-TCX egress drops on this path; no SHOT verdict required. |
| Most likely next-step suspects? | (1) ARP/MAC issue — `tc_reverse_nat` rewrites L3 but Ethernet source MAC may mismatch `lb_veth_a` post-rewrite. (2) MTU on response path. (3) Pre-populated ARP cache scope is `lb_veth_b`, not `lb_veth_a`. |
| Unexpected discoveries? | (a) Run cadence 5 → 6 → 7 → 9 → 10 → 11 → 12 → 13 → 14 → 16 matches the expected SYN + 6 retransmits + teardown shape of a failed TCP connection — consistent with backend reaching the LB but response failing post-TCX. (b) The final +2 in run_time_ns spike (67 µs) suggests a slow path on teardown — possibly the kernel reaping orphan skbs after netns destruction. |

## Probe 6 results — pwru per-skb trace

**Date**: 2026-05-07 (continuation)
**Script**: `.context/probe6-run.sh`
**Tool**: `pwru` v1.0.11 (Cilium), kernel 6.8.0-111-generic

### Headline

The drop is at `__dev_queue_xmit → qdisc_pkt_len_init → kfree_skb_reason(SKB_DROP_REASON_TC_EGRESS)`
on the `lb_veth_a`-egress side of an IP-forwarded backend reply skb.
**`tc_run` does NOT appear in the dropped skb's trace** — pwru attaches
to ~1670 kernel functions including `tc_run` / `cls_bpf*` /
`sch_handle_egress`, and skb 0xffff000212c14ee8 (the dropped one,
reused 6× across the 6 retransmits, src=`10.1.0.5:8080` not
post-rewrite) hits 60+ unique kernel functions but **none of them are
TC-classifier dispatchers**. The packet dies before tc_reverse_nat is
ever consulted on the lb_veth_a egress side. Probe 5's run_cnt=16 is
real but those invocations are firing on a different path (likely
ARP — the only `tc_run` entry pwru observed is on a 28-byte
`0x0806`/ARP frame, kernel pid=ksoftirqd/1).

### The dropped trace (skb 0xffff000212c14ee8, len=86 = 14 ETH + 20 IP + 32 TCP + 20 payload)

```
=== Cycle 1 — backend-ns egress on lb_veth_b (SUCCEEDS) ===
__dev_queue_xmit → qdisc_pkt_len_init → netdev_core_pick_tx →
validate_xmit_skb → netif_skb_features → passthru_features_check →
skb_network_protocol → skb_csum_hwoffload_help → skb_checksum_help →
skb_ensure_writable → validate_xmit_xfrm → dev_hard_start_xmit →
dev_queue_xmit_nit → skb_clone → skb_clone_tx_timestamp →
__dev_forward_skb → __dev_forward_skb2 → skb_scrub_packet →
eth_type_trans → __netif_rx → netif_rx_internal → enqueue_to_backlog
→ __netif_receive_skb → __netif_receive_skb_one_core → tpacket_rcv →
... (peer-veth hop into lb-ns)

=== Cycle 2 — lb-ns IP forward + lb_veth_a egress (DROPS) ===
ip_rcv → ip_rcv_core → ip_rcv_finish → tcp_v4_early_demux →
ip_route_input_noref → ip_route_input_slow → __mkroute_input →
fib_validate_source → ip_forward → pskb_expand_head →
skb_headers_offset_update → ip_forward_finish → ip_output →
nf_hook_slow → apparmor_ip_postroute → ip_finish_output →
__ip_finish_output → ip_finish_output2 → neigh_resolve_output →
eth_header → skb_push → __dev_queue_xmit → qdisc_pkt_len_init →
kfree_skb_reason(SKB_DROP_REASON_TC_EGRESS)
```

The last 5 functions before the drop are exactly:
`ip_finish_output2 → neigh_resolve_output → eth_header → skb_push →
__dev_queue_xmit → qdisc_pkt_len_init → DROP`. `eth_header` is the
function that builds the L2 header from src dev MAC + dst MAC from
neighbour cache. The drop happens INSIDE the second `__dev_queue_xmit`
on lb_veth_a, before any TC classifier runs.

The reported drop reason in pwru's `kfree_skb_reason()` annotation is
literally `SKB_DROP_REASON_TC_EGRESS` (string form, not just the
numeric 51). All 6 retransmits hit this exact same drop site at the
exact same skb pointer (allocator reuse).

### Comparison: a transiting len=74 skb (handshake)

Skb 0xffff0000dd271b00 (len=74) traverses Cycle 1 successfully on
lb_veth_b egress (same kernel-fn sequence) and never re-enters the
lb_veth_a egress path in pwru's view — it's a different
direction/flow. Cleaner divergence: transiting flows that DO reach
`__dev_queue_xmit` via dev_hard_start_xmit (Cycle 1 shape) survive;
dropped flows die on the SECOND `__dev_queue_xmit` on lb_veth_a
post-IP-forward, immediately after `qdisc_pkt_len_init`.

### Suspect attribution

Of the three downstream suspects:

- **(1) Eth src MAC mismatch** — `eth_header` IS called on the dropped
  path, so an L2 header IS built. eth_header itself returns ETH_HLEN
  on success and doesn't drop. If the rewrite were structurally bad,
  the drop would be later (e.g. inside dev_hard_start_xmit on the peer
  side). **Possible but not directly evidenced.**
- **(2) ARP/neigh entry missing** — `neigh_resolve_output` IS called
  and DOES return successfully (it's followed by `eth_header` in the
  trace; if neigh_resolve_output had failed it would have called
  `__neigh_event_send → kfree_skb` directly without eth_header).
  **EXCLUDED.**
- **(3) `validate_xmit_skb` / `skb_checksum_help` failing on rewrite**
  — does NOT appear on the post-IP-forward second cycle at all.
  validate_xmit_skb runs on the FIRST cycle (lb_veth_b egress) and
  succeeds. The drop is BEFORE validate_xmit_skb on cycle 2.
  **EXCLUDED on this trace.**

The trace points at NONE of (1)/(2)/(3) cleanly. The actual drop
shape is **`qdisc_pkt_len_init → kfree_skb_reason(TC_EGRESS)` with no
intervening tc_run`**. On Linux 6.8 + TCX, this is the path
[`sch_handle_egress`](https://elixir.bootlin.com/linux/v6.8/source/net/core/dev.c#L4036)
takes when the egress qdisc/TCX dispatcher itself rejects the skb
before invoking attached programs — typically because the skb fails a
pre-classifier sanity check (e.g. `qdisc_pkt_len_init` itself records
`pkt_len` from `skb->len`, and a downstream check on
`skb->protocol`/`skb->mac_header`/`gso_size` mismatch can cause
immediate rejection).

### What's weird

- **`tc_run` is invoked on ARP frames** (`0x0806`, len=28, ksoftirqd
  pid) but NOT on the dropped TCP frames. This suggests the TCX
  dispatcher is being entered on lb_veth_a egress for ARP but bypassed
  (or the program is being invoked via a kernel-internal path pwru
  doesn't trace) for the rewrite-target IPv4 frames.
- **`pskb_expand_head` is called between `ip_forward` and
  `ip_forward_finish`** — the kernel had to grow the skb headroom
  during forwarding. This is consistent with the L2 header rewrite
  needing more headroom than the receive-side skb had, and is normal,
  but it does mean **the dropped skb has been re-laid in memory at
  that point** (`skb_release_data` + `skb_headers_offset_update`
  follow), which can invalidate any saved pointers or csum state from
  upstream.
- **The src IP in the dropped trace is `10.1.0.5:8080`, NOT the
  rewritten `10.0.0.1:8080`.** pwru reads `iph->saddr` per-hook from
  the live skb, so this means at the moment of the drop, the IP
  source has NOT been rewritten by tc_reverse_nat — consistent with
  tc_run never having been called on this skb.

### Bottom line

The drop is upstream of tc_reverse_nat's invocation point on
lb_veth_a egress, not downstream. The kernel attributes
`SKB_DROP_REASON_TC_EGRESS = 51` to a drop that occurs in the egress
TCX dispatch path before the loaded program runs — most likely
`sch_handle_egress` or the qdisc enqueue itself rejecting the skb
based on a property set during IP-forward (`pskb_expand_head` /
`skb_headers_offset_update` immediately preceding) or inherited from
the receive side. Probe 5's run_cnt=16 measures invocations on OTHER
skbs (ARP frames, not the dropped data segments).

The next probe is to instrument `sch_handle_egress` directly (kfunc
on the dispatcher itself) and read the verdict it returns BEFORE
calling the attached program — this is the only kernel function on
the dropped path between `qdisc_pkt_len_init` and the final
`kfree_skb_reason`, so it must be the actor doing the drop.

---

## Probe 7 results — drop-site skb metadata

**Date**: 2026-05-07
**Script**: `.context/probe7-run.sh`
**Tool**: bpftrace v0.20.2, kernel 6.8.0-111-generic
**Probes**: `kfunc:vmlinux:kfree_skb_reason` (filtered `reason==51`) and
`kfunc:vmlinux:__dev_queue_xmit` (filtered `60 ≤ len ≤ 100`).

### DROP events (all 6 retransmits, identical metadata)

```
DROP skb=0xffff000102f8e8e8 len=86 data_len=20 proto=0x0008
     ip_summed=0  csum=100120  csum_start=288  csum_offset=16
     gso_size=0   gso_segs=1   gso_type=1   nr_frags=1
     headroom=254 tailroom=64  mac=254 nh=268 th=288 mark=0
```

All 6 dropped skbs reuse the same allocator slot
(`0xffff000102f8e8e8`) — confirms re-transmits, not 6 distinct skbs.

### XMIT events for the SAME skb pointer (`0xffff000102f8e8e8`, len=86)

bpftrace observed `__dev_queue_xmit` entry **twice per cycle** — once
with `ip_summed=3, mac=65535` (header unset, sw-csum pending), once
with `ip_summed=0, mac=254` (post-`skb_checksum_help`, mac set). The
DROP event matches the SECOND shape exactly (`ip_summed=0, mac=254`).

### Surviving control segments (len=66, len=74)

```
XMIT len=66 ip_summed=3 csum=100120 csum_start=288 csum_offset=16
            gso_segs=1 gso_type=1 nr_frags=0 data_len=0  headroom=254
XMIT len=66 ip_summed=0 csum=100120 csum_start=288 csum_offset=16
            gso_segs=1 gso_type=1 nr_frags=0 data_len=0  headroom=254
XMIT len=74 ip_summed=3 csum=100118 csum_start=280 csum_offset=16
            gso_segs=1 gso_type=1 nr_frags=0 data_len=0  headroom=246
```

### Diff: DROPPED (len=86) vs SURVIVING (len=66) at the matching `ip_summed=0, mac=set` shape

| Field | Dropped (len=86) | Surviving (len=66) |
|---|---|---|
| `data_len` | **20** (non-linear) | **0** (linear) |
| `nr_frags` | **1** | **0** |
| `len` (linear portion) | 66 (= 86 − 20) | 66 |
| `ip_summed` | 0 (CHECKSUM_NONE) | 0 (same) |
| `csum_start` | 288 | 288 (same) |
| `csum_offset` | 16 | 16 (same) |
| `gso_type` | 1 (SKB_GSO_TCPV4) | 1 (same) |
| `gso_segs` | 1 | 1 (same) |
| headroom/tailroom | 254 / 64 | 254 / 64 (same) |
| `mac/nh/th` | 254/268/288 | 254/268/288 (same) |

### Obviously-invalid state on the dropped skb

1. **Non-linear skb at L2 egress with `gso_type=SKB_GSO_TCPV4` set on a
   single-segment skb (`gso_segs=1`).** The 20-byte payload sits in a
   page fragment (`nr_frags=1, data_len=20`), but the linear portion
   (66 B = ETH+IP+TCP) is intact. The skb is *fragmented but not
   GSO-segmented* — yet `gso_type` says SKB_GSO_TCPV4.
2. **Stale `csum_start=288` / `csum_offset=16` after `ip_summed`
   transitioned from PARTIAL (3) to NONE (0).** When
   `skb_checksum_help` clears `ip_summed` to NONE it should NOT leave
   the partial-csum metadata behind, but here `csum_start` and
   `csum_offset` retain their PARTIAL-mode values. `csum_start=288`
   points 222 B past the linear-data start (data lives at
   `head+254..head+254+66`), i.e. **into the page-fragment region**.
3. **`gso_segs=1` with non-zero `gso_type`.** Surviving control skbs
   show this same combination, so it's not by itself the trigger —
   but combined with `nr_frags=1` it's the unique signature of dropped
   skbs.

### What the metadata shows

The dropped skbs are **non-linear (paged) at TCX egress** with stale
PARTIAL-csum metadata still attached after `skb_checksum_help` cleared
`ip_summed`. The 20-byte TCP payload lives in `frags[0]`, with
`csum_start=288` (linear-data + 34) pointing past the linear region's
end — it would be valid for a linearised skb of len=88+ (header +
in-line payload), but here the linear region ends at `head+254+66 =
head+320` while `head+csum_start = head+288` lands in the linear
region; however, the TCP header is at `th=288` and the kernel's
post-`pskb_expand_head` re-layout means `csum_start=288` now points to
the start of the TCP header, NOT past the L4 payload boundary as
required when `ip_summed=NONE` with paged data. Surviving control
skbs (length-66, length-74) carry the same `csum_start/csum_offset`
values but have `data_len=0` and `nr_frags=0` — fully linear, so the
csum metadata is harmless leftover. **The data-bearing skbs are the
only ones with `nr_frags=1`, and they are the only ones dropped.**
