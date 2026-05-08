# ADR-0045 — `bpf_redirect` datapath replaces kernel IP-forward + TCX-egress reverse-NAT

## Status

Accepted. 2026-05-07. Decision-makers: Morgan (proposing); user
ratified the pivot in dispatch (2026-05-07) following the empirical
falsification trail in `docs/analysis/e1-bpftrace-results.md` probes
1–7. Tags: phase-2, dataplane, xdp, bpf-redirect, bpf-fib-lookup,
supersedes-tc-egress.

**Amendment 2026-05-07**: Decision §§ 1.6 and 2.6 corrected —
`bpf_redirect_neigh` replaced with `bpf_redirect`. Research evidence:
`docs/research/dataplane/bpf-redirect-xdp-forward-path-research.md`.
`bpf_redirect_neigh` is TC-only (kernel verifier rejects on XDP;
kernel commit `b4ab31414970`, Daniel Borkmann, Sep 2020; never
extended to XDP). The correct XDP pattern is `bpf_fib_lookup` +
manual L2 MAC rewrite + `bpf_redirect(ifindex, 0)`, matching
Cilium's `fib_do_redirect` fallback path (`bpf/lib/fib.h:122-151`)
and the kernel's `samples/bpf/xdp_fwd_kern.c`. `bpf_redirect_peer`
is also TC-only. Title updated accordingly.

**GitHub tracking issue**: #159 — *[2.x] Replace IP-forward +
TCX-egress with bpf_redirect_neigh datapath*. Every artifact in this
ADR cites that issue as the forward pointer for production work.

**Companion ADRs**: ADR-0040 (three-map split + HASH_OF_MAPS atomic
swap; Q2=A reopened by this ADR — see § "Supersession"), ADR-0041
(weighted Maglev + REVERSE_NAT shape — preserved; only the *layer*
of reverse-NAT changes), ADR-0042 (`ServiceMapHydrator` reconciler —
preserved; control-plane shape unchanged), ADR-0043 (3-iface test
topology — preserved; the topology stays valid under the new
datapath).

## Context

### What changed

ADR-0040 Q2=A locked a TC-egress reverse-NAT shape: XDP-ingress on
the client-facing veth performs DNAT, the kernel IP-forwarder routes
the rewritten skb to the backend-facing veth, the kernel calls
`__dev_queue_xmit` on the backend-facing veth, and a TCX-egress
program (`tc_reverse_nat`) — at that time meant to handle the
*response* path — was attached on the client-facing veth's egress
hook to rewrite the response.

That decision was correct on its evidence at the time (research §
4.1 of `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
recommended Option α: `bpf_fib_lookup` + L2 MAC rewrite at XDP
ingress; reverse-NAT-via-TCX-egress was the natural pair on the
response side). It became wrong on the evidence collected by probes
1–7 in `docs/analysis/e1-bpftrace-results.md` between 2026-05-06 and
2026-05-07.

### The empirical chain that falsified the locked datapath

S-2.2-17 (`real_tcp_connection_completes_through_vip_with_payload_echo`)
showed a reproducible pattern: length-0 TCP control segments traverse
the dataplane successfully; length-N data-bearing segments drop, six
times per failing run, matching the kernel's TCP retransmission
schedule. Drop reason: `SKB_DROP_REASON_TC_EGRESS = 51` from
`__dev_queue_xmit` on the client-facing veth (probe 1, kernel kstack
unambiguous).

The chain:

1. **Probe 1 — kfree_skb tracepoint.** 6 drops with reason 51, kstack
   `ip_finish_output2 → neigh_resolve_output → eth_header → skb_push
   → __dev_queue_xmit → kfree_skb_reason`. The drop site is on the
   *forwarded-skb egress* path inside the lb-ns, not on the original
   ingress path.
2. **Probes 2 + 2b — `tcf_classify` / `tc_run` invocations.**
   `tcf_classify` never fires (legacy classifier inlined out on 6.8
   with TCX); `tc_run` fires 7 times during the test window, **every
   call returning `TC_ACT_UNSPEC`** — never `TC_ACT_SHOT`. The
   loaded program is not the actor doing the drop.
3. **Probes 3 + 4 — TC attachment audit.** Probe 3 read legacy
   `tc filter show` and reported "no filters bound." Probe 4
   corrected this with TCX-aware `bpftool net show` and confirmed
   `tc_reverse_nat` IS attached via TCX on the backend-facing veth
   egress (`prog_id 5151 link_id 489`). The earlier "not attached"
   reading was a methodology artefact: legacy `tc filter show` and
   TCX inspect different kernel data structures.
4. **Probe 5 — BPF stats / run_cnt.** `kernel.bpf_stats_enabled=1` +
   `bpftool prog show id <N> --json` polling shows `tc_reverse_nat`
   ran 16 times during the test, with run_time averaging ~7.5 µs.
   The program executes; it is not bypassed in the abstract sense.
5. **Probe 6 — `pwru` per-skb trace.** The dropped skb
   (`0xffff000212c14ee8`, len=86) hits 60+ unique kernel functions
   on its path; **none of them are `tc_run` or any TC-classifier
   dispatcher.** The drop happens at
   `qdisc_pkt_len_init → kfree_skb_reason(SKB_DROP_REASON_TC_EGRESS)`
   on the *second* `__dev_queue_xmit` (lb_veth_a egress, post-IP-
   forward). Probe 5's run_cnt=16 measures invocations on *other*
   skbs (ARP frames; not the dropped data segments). The src IP at
   the moment of drop is `10.1.0.5:8080`, NOT the rewritten
   `10.0.0.1:8080` — the skb has not been rewritten by
   `tc_reverse_nat` because the program never ran on this skb.
6. **Probe 7 — drop-site skb metadata.** All 6 dropped skbs share an
   identical "obviously-invalid" shape:
   - `data_len=20, nr_frags=1` — the 20-byte payload sits in a page
     fragment, the linear region is 66 B (= ETH+IP+TCP).
   - `gso_segs=1, gso_type=SKB_GSO_TCPV4` — the skb is paged but
     not GSO-segmented; the linear-only control segments at len=66
     and len=74 carry the same gso fields and pass.
   - `ip_summed=0` (CHECKSUM_NONE), `csum_start=288, csum_offset=16`
     — stale PARTIAL-csum metadata after `skb_checksum_help` cleared
     `ip_summed`. `csum_start` points into the linear region (TCP
     header), not past the payload boundary as required for
     CHECKSUM_NONE on non-linear skbs. **The dropped skbs are the
     ONLY ones with `nr_frags=1`, and they are the only ones
     dropped.**

The mechanism is now unambiguous. When the kernel IP-forwarder routes
an incoming skb across the lb-ns, it calls `pskb_expand_head` to
grow the headroom for the L2 rewrite (probe 6 trace, line:
`ip_forward → pskb_expand_head → skb_headers_offset_update →
ip_forward_finish`). On veth-peer-delivered paged skbs (the kernel's
natural choice when the receive path delivers a payload split across
the linear region and a page fragment), `pskb_expand_head` triggers
`skb_checksum_help` to materialise the deferred CHECKSUM_PARTIAL into
inline bytes. `skb_checksum_help` clears `ip_summed` to `NONE` but
leaves `csum_start` / `csum_offset` populated. The downstream
`qdisc_pkt_len_init` (or an adjacent pre-classifier sanity check
inside `sch_handle_egress`) sees a non-linear skb with stale
CHECKSUM_PARTIAL metadata and rejects it with
`SKB_DROP_REASON_TC_EGRESS = 51` *before* the TCX dispatcher invokes
the loaded `tc_reverse_nat`.

This is a structural defect of the *current architecture*, not a
defect of `tc_reverse_nat`'s body. The TCX program never runs on the
data segments. There is no in-program fix that can recover them; the
kernel has already dropped them by the time TCX would dispatch.

### What this means for the locked Q2=A decision

ADR-0040 Q2=A is empirically wrong for the data-bearing skb path.
Every length-N TCP segment in S-2.2-17 dies at this site. Length-0
control segments survive because they are linear (no `data_len`, no
`nr_frags`, no `pskb_expand_head` trigger), so the kernel never
materialises the stale-csum-on-paged-skb condition.

Test-fixture mitigations (`ethtool -K $iface tso off gso off gro
off`) were tried during the probe chain and falsified: the helper
already disabled offloads, but the paged-skb shape comes from
veth-peer delivery semantics, not iface offload settings. There is
no veth tuning that fixes this.

The path forward is to bypass the kernel IP-forwarder entirely. With
the IP-forwarder out of the path, `pskb_expand_head` and
`skb_checksum_help` never fire, the stale-csum-on-paged-skb
condition never arises, and the TCX-egress dispatcher's
pre-classifier sanity rejection cannot trigger. This is precisely
what `bpf_fib_lookup` + L2 MAC rewrite + `bpf_redirect` does — it is
the XDP forwarding pattern Cilium uses (`fib_do_redirect` in
`bpf/lib/fib.h:122-151`) and the foundation of their L4LB
dataplane.

## Decision

### 1. Replace the request path: XDP ingress L3+L2 rewrite + `bpf_redirect`

The `xdp_service_map_lookup` program on the client-facing veth
ingress is extended to perform the full forward-path rewrite
in-program, with no kernel IP-forwarding involvement:

1. SERVICE_MAP / MAGLEV_MAP lookup (preserved from Slice 04 — the
   chained HASH_OF_MAPS lookup logic is untouched).
2. BACKEND_MAP fetch for the resolved `BackendId` (preserved).
3. L3 rewrite — DNAT of `(VIP, vip_port)` → `(backend_ip,
   backend_port)`; TCP/IP checksum incremental update via
   `bpf_l3_csum_replace` / `bpf_l4_csum_replace` (preserved from
   ADR-0040 Q1=A).
4. **NEW**: `bpf_fib_lookup(ctx, &fib, sizeof(fib), 0)` against the
   post-rewrite `(src_ip, dst_ip)` to resolve the egress iface index
   and the next-hop destination MAC.
5. **NEW**: L2 rewrite — `eth_store_daddr` writes the FIB-resolved
   next-hop MAC into `eth_hdr->h_dest`; `eth_store_saddr` writes
   the egress iface's source MAC into `eth_hdr->h_source`. Both
   inside the same XDP program; the program never returns to the
   kernel networking stack between L3 and L2 rewrite.
6. **NEW**: Return `XDP_REDIRECT` via `bpf_redirect(fib.ifindex, 0)`.
   The FIB-resolved L2 MACs have already been written in step 5; no
   further neighbor resolution is needed. The kernel's XDP fast path
   delivers the rewritten frame directly to the resolved egress
   iface's tx queue, bypassing the IP-forwarder, bypassing
   `pskb_expand_head`, bypassing `skb_checksum_help` on the
   receive-side skb.

   **Note**: `bpf_redirect_neigh` and `bpf_redirect_peer` are TC-only
   helpers (restricted to `sched_cls`, `sched_act`, and `lwt_xmit` by
   the kernel verifier; kernel commit `b4ab31414970`, never extended
   to XDP). Cilium's abstraction layer makes both a compile-time error
   on XDP programs (`overloadable_xdp.h:44-52` throws
   `__throw_build_bug()`). When `bpf_fib_lookup` has already resolved
   L2 MACs and the caller has written them into the Ethernet header
   (step 5), `bpf_redirect` is functionally equivalent to
   `bpf_redirect_neigh` — the latter just automates the L2 resolution
   step that FIB already performed.

The kernel IP-forwarder never sees these packets. The TCX-egress
dispatcher on the backend-facing veth is uninvolved on the request
path.

This matches Cilium's `fib_do_redirect` fallback path
(`bpf/lib/fib.h:122-151`) and the kernel's `samples/bpf/
xdp_fwd_kern.c` — both use `bpf_fib_lookup` + manual L2 MAC rewrite
+ `bpf_redirect`, not `bpf_redirect_neigh`. This is the structural
pattern every production XDP L4LB deployed on stable 6.x kernels
uses. It is not novel for Overdrive; it is novel only relative to the
ADR-0040 Q2=A locked shape.

### 2. Replace the response path: XDP ingress on the backend-facing veth

`tc_reverse_nat` on the client-facing veth's TCX-egress hook is
**retired** as a TC program. Its reverse-NAT *logic* — REVERSE_NAT_MAP
lookup, L3+L4 rewrite, checksum update — is preserved verbatim, but
moves to a new XDP program attached at the **ingress** of the
backend-facing veth.

When the backend's response packet enters the lb-ns through
`lb_veth_b` (the backend-facing veth), the new
`xdp_reverse_nat_lookup` program runs as the first kernel-level
hook on that ingress:

1. Header parse + sanity prologue (per ADR-0040 Q3 amendment —
   ingress-only enforcement; this is the second ingress in the
   datapath, so the prologue applies).
2. REVERSE_NAT_MAP lookup keyed on the 5-tuple
   `(client_ip, client_port, backend_ip, backend_port, proto)`.
3. L3 rewrite — `(backend_ip, backend_port)` → `(VIP, vip_port)`;
   incremental checksum update.
4. `bpf_fib_lookup` against the post-rewrite `(src_ip, dst_ip)` to
   resolve the client-facing egress iface and next-hop MAC.
5. L2 rewrite — destination MAC = client-facing-veth-peer MAC;
   source MAC = client-facing-veth source MAC.
6. Return `XDP_REDIRECT` via `bpf_redirect(fib.ifindex, 0)` to the
   client-facing veth. As in § 1 step 6, the FIB-resolved L2 MACs
   have already been written in step 5; `bpf_redirect` is the correct
   XDP helper.

The response path is structurally symmetric to the request path:
both are XDP-ingress L3+L2 rewrite + `bpf_redirect`. The kernel
IP-forwarder is bypassed in both directions.

### 3. Single program or two? — two programs

The reverse-NAT logic moves to a second XDP program rather than
folding into `xdp_service_map_lookup` with a per-direction branch.
Rationale:

- **Locality matches attach point.** The forward path attaches to
  the client-facing veth ingress; the reverse path attaches to the
  backend-facing veth ingress. They never see the same packets;
  per-direction logic is per-iface logic.
- **Verifier budget.** ADR-0040's Tier 4 envelope (≤ 60% of 1M
  ceiling per program; ≤ 20% delta per PR) is per-program. Splitting
  by direction keeps each program well below ceiling.
- **Cilium's structural choice.** Cilium's `bpf_lxc.c` and
  `bpf_overlay.c` are split by attach point, not folded into one
  program with a per-direction branch (research § Q4, Finding 4.1).
  Folding would diverge from the published reference for no
  observable upside.
- **Single-purpose programs are more debuggable.** A bug in the
  reverse path does not require reasoning about forward-path map
  lookups; veristat baselines stay tied to one direction's logic.

The two programs are:

| Program | Attach | File | Maps consumed |
|---|---|---|---|
| `xdp_service_map_lookup` | XDP ingress, client-facing veth | `crates/overdrive-bpf/src/programs/xdp_service_map.rs` (preserved name; body extended with FIB+L2-rewrite+redirect) | SERVICE_MAP, MAGLEV_MAP, BACKEND_MAP, REVERSE_NAT_MAP (write-side: insert per new flow), DROP_COUNTER |
| `xdp_reverse_nat_lookup` | XDP ingress, backend-facing veth | `crates/overdrive-bpf/src/programs/xdp_reverse_nat.rs` (NEW; replaces `tc_reverse_nat.rs`) | REVERSE_NAT_MAP (read-side), DROP_COUNTER |

`tc_reverse_nat.rs` is **deleted** when the new program lands —
single-cut greenfield migration per CLAUDE.md's deletion discipline,
no retained stub, no `#[deprecated]`, no parallel paths.

### 4. Sanity prologue scope (post-pivot)

ADR-0040 Q3 (amended 2026-05-07) scoped the sanity prologue to XDP
ingress only after the TC-egress invocation was empirically wrong
for the same forwarded-skb-csum-staleness reason that motivates this
pivot. Under the post-pivot architecture:

- `xdp_service_map_lookup` (client-facing veth ingress) — calls the
  prologue. Unchanged from the Q3 amendment.
- `xdp_reverse_nat_lookup` (backend-facing veth ingress) — **calls
  the prologue.** This is also XDP ingress; the Q3 amendment's
  "ingress-only" scope is satisfied. The prologue's preconditions
  (linear-buffer length matches IPv4 `total_length`) hold on freshly
  ingressed packets that have not been forwarded.

There is no TC-egress program to scope the prologue against. The
"ingress-only" rule is preserved structurally because TC-egress is
no longer in the dataplane.

### 5. `bpf_fib_lookup` failure handling — `XDP_PASS` to kernel

On `bpf_fib_lookup` returning a non-zero status (any of
`BPF_FIB_LKUP_RET_*` codes — no route, no neighbour entry, blackhole,
unreachable, frag-needed, etc.), both XDP programs fall back to
`XDP_PASS`. The kernel's normal networking stack handles the packet
through its own routing + ARP machinery.

Rationale:

- **Cilium's canonical choice** for the same failure class. Their
  XDP fast path returns `XDP_PASS` on FIB miss; the kernel handles
  ARP request, learns the neighbour, and the next packet hits the
  populated cache and gets the fast path (research § Finding 4.4 —
  "after ARP delay, frame arrives via stack").
- **Safer than `XDP_DROP` on cold cache.** A genuinely unreachable
  destination produces a single `XDP_PASS` and a kernel-side ICMP
  Unreachable; an `XDP_DROP` would silently fail the connection.
- **Falls within the existing DROP_COUNTER discipline.** `XDP_PASS`
  on FIB miss does not consume a `DropClass` slot; only true
  `XDP_DROP` paths do. No new `DropClass` variant needed for this
  case. (A `BPF_FIB_LKUP_RET_BLACKHOLE` is structurally rare enough
  that a future ADR can add a `FibBlackhole` variant if it becomes
  signal-worthy.)

The first packet of any new flow may pay one slow-path round-trip
(SYN goes through the kernel's ARP machinery; SYN-ACK populates the
neighbour cache; ACK and onwards take the fast path). The S-2.2-17
test pre-populates the ARP cache via `ip neigh replace ... nud
permanent` (existing line in `reverse_nat_e2e.rs:472-489`)
specifically to eliminate this first-packet flake. That setup line
is preserved.

### 6. Verifier-budget envelope

The post-pivot programs add `bpf_fib_lookup` (one helper call, ~10
verifier instructions) plus a 12-byte L2-MAC `memcpy` (one
unrolled-loop, ~6 verifier instructions per program). The research's
empirical estimate (§ Finding 4.2) puts the combined verifier cost
at ~20–30 instructions per program — well below the existing
ADR-0040 ASR-2.2-03 envelope (≤ 20% delta per PR; ≤ 60% of 1M
ceiling absolute).

**Lock**: Slice 06-05 (veristat baseline update) records new
baselines for both `xdp_service_map_lookup` (extended body) and
`xdp_reverse_nat_lookup` (new program) under
`perf-baseline/main/verifier-budget/`. The PR-gate check stays at
≤ 20% delta vs the new baseline once it lands; the absolute ceiling
stays at ≤ 60% of 1M.

The retired `tc_reverse_nat.rs` baseline file is deleted in the same
commit that lands the deletion of the program — single-cut migration
per CLAUDE.md.

### 7. Falsification gate — S-2.2-17 must GREEN naturally

S-2.2-17 (`real_tcp_connection_completes_through_vip_with_payload_echo`)
is the load-bearing falsification probe for this pivot. The test
exercises the full request → backend → response path with real
nc-driven TCP traffic on a 3-iface real-veth topology (per
ADR-0043).

Under the current (pre-pivot) architecture, every length-N data
segment dies at the
`qdisc_pkt_len_init → kfree_skb_reason(TC_EGRESS)` site documented
above. Under the post-pivot architecture, the IP-forwarder is no
longer in the path, so `pskb_expand_head` + `skb_checksum_help`
never run on these skbs, so the stale-csum-on-paged-skb condition
never arises, so the pre-classifier sanity rejection cannot trigger.

The test should GREEN naturally when the new programs land and the
attach points move to ingress-on-both-veths. **No test-body change
is required** — the assertion shape (client `nc` exits 0; payload
echoed) is post-pivot-correct as written. The `#[ignore]` attribute
added in the dispatch (per Output 4) lifts when the new programs
land and the test is unblocked.

If S-2.2-17 still fails after the pivot lands, the diagnosis is a
*new* bug in the post-pivot programs, not a recurrence of the
pre-pivot failure mode (the kernel mechanism has been structurally
removed from the path). A failing post-pivot S-2.2-17 is grounds
for a fresh probe sequence, not an attempt to patch the pre-pivot
shape.

### 8. Phasing and slice impact

The pivot lands as a new follow-on slice (or slice cluster) under GH
#159. The existing Slice 04–07 work in
`docs/feature/phase-2-xdp-service-map/deliver/roadmap.json` carries
forward as follows:

| Slice | Status | Rationale |
|---|---|---|
| 01 (real-iface attach) | Preserved as-is | Iface resolution, native/SKB fallback, typed errors — all post-pivot-valid. |
| 02 (SERVICE_MAP forward path single-VIP) | Preserved; extended | The Slice 04 SERVICE_MAP lookup logic stays. The post-pivot program adds `bpf_fib_lookup` + L2 MAC rewrite + `bpf_redirect` as Slice-after-04 work. |
| 03 (HASH_OF_MAPS atomic swap) | Preserved as-is | The atomic-swap primitive is kernel-side-direction-agnostic; it works identically on the post-pivot SERVICE_MAP. |
| 04 (Weighted Maglev) | Preserved as-is | MAGLEV_MAP and the Eisenbud permutation are unchanged. The post-pivot program reads MAGLEV_MAP exactly as before. |
| 05 (REVERSE_NAT — TC egress) | **Partially superseded.** | The REVERSE_NAT_MAP shape, key structure, and endianness lockstep contract (ADR-0041) are preserved. The TC-egress *attach layer* is retired; reverse-NAT logic moves to the new `xdp_reverse_nat_lookup` program on the backend-facing veth ingress. The Slice 05 endianness-lockstep proptest (S-2.2-17 was its end-to-end gate) is preserved. |
| 06-01..06-04 (sanity prologue + DropClass + Tier 3 mixed-batch) | Preserved as-is | Slice 06-04 already landed at commit `a6d7d46` (Tier 3 mixed-batch + SanityChecksFireBeforeServiceMap DST invariant; sanity prologue ingress-only per ADR-0040 Q3 amendment). The pivot does not touch this work. |
| 06-05 (veristat baseline update) | **Replanned.** | Baselines are recorded against the post-pivot program shape, not the pre-pivot shape. The Tier 4 gate threshold (≤ 20% delta; ≤ 60% of 1M ceiling) is preserved. |
| 07 (Tier 4 perf gates) | Preserved as-is | xdp-bench / xdp-trafficgen on a veth pair is direction-agnostic. |
| 08 (`ServiceMapHydrator` reconciler) | Preserved as-is | Control-plane reconciler is dataplane-shape-agnostic. The hydrator writes into SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP via the typed userspace handles; the post-pivot dataplane reads the same maps. |

Concrete production-code work, queued under GH #159 for crafter
dispatch:

- New `crates/overdrive-bpf/src/programs/xdp_reverse_nat.rs` —
  reverse-NAT XDP program at backend-facing veth ingress (replaces
  retired `tc_reverse_nat.rs`).
- Extended `crates/overdrive-bpf/src/programs/xdp_service_map.rs` —
  add `bpf_fib_lookup` + L2 MAC rewrite + `bpf_redirect` after the
  existing SERVICE_MAP / MAGLEV_MAP / BACKEND_MAP lookup chain.
- Loader changes in `crates/overdrive-dataplane/src/lib.rs` /
  `loader.rs` — attach `xdp_reverse_nat_lookup` on the
  backend-facing veth ingress; retire the
  `SchedClassifier::attach(..., TcAttachType::Egress)` call.
- Delete `crates/overdrive-bpf/src/programs/tc_reverse_nat.rs` and
  any TC-link plumbing in the loader.
- Delete `perf-baseline/main/verifier-budget/tc_reverse_nat.txt`
  (when present) and add a new entry for `xdp_reverse_nat_lookup`.
- 3-iface test topology (ADR-0043) — preserve as-is. The new
  programs attach to existing test ifaces; no topology change.

The crafter dispatch for #159 is independent of this ADR. This ADR
captures the DECISION; the crafter owns the implementation.

## Alternatives Considered

### A — Stay with TCX-egress + work around the kernel mechanism

Add a TCX-ingress program on the *forwarded-skb path* to linearise
or fix CHECKSUM_PARTIAL metadata before the kernel's egress
pre-classifier runs. **Rejected**:

- Requires fighting against a kernel mechanism (`pskb_expand_head` +
  `skb_checksum_help`) that is correctly implemented for the
  general kernel-stack-egress case. We would be papering over a
  shape the kernel does not promise to preserve through forwarding.
- Adds a third TCX program to the dataplane for a workaround. The
  verifier-budget envelope would tighten.
- Cilium does not work this way. Their XDP path uses
  `bpf_fib_lookup` + L2 MAC rewrite + `bpf_redirect` (the TC path
  uses `bpf_redirect_neigh` where available, but that helper is
  TC-only). If there were a credible TCX-side workaround, Cilium
  would already be using it (they have spent more eng-years than
  this project on XDP/TC dataplane shape).
- The same kernel mechanism may shift across LTS releases — ADR-0040
  Q2=A's evidence chain *is* a kernel-version-specific signal (6.8
  paged-skb interaction with `qdisc_pkt_len_init`). A workaround
  that worked on 6.8 may break on 6.6 or 5.10. The pivot removes
  the dependency on this mechanism entirely.

### B — Adopt `bpf_fib_lookup` + L2 MAC rewrite + `bpf_redirect` (chosen)

The structural answer this ADR locks. See § Decision above.

### C — Fold reverse-NAT into `xdp_service_map_lookup` with per-direction branch

A single XDP program attached on both veths (or only one veth, with
internal direction detection from packet flow). **Rejected**:

- Splitting per attach point matches Cilium's structural choice
  (research § Q4, Finding 4.1). Folding diverges from the published
  reference.
- Verifier-budget envelope is per-program; folding two directions
  into one program halves the per-direction headroom against the
  ≤ 60% absolute ceiling.
- Single-purpose programs are more debuggable; a bug in the reverse
  path does not require reasoning about forward-path map lookups.

### D — Move reverse-NAT to TC-ingress on the backend-facing veth instead of XDP-ingress

A TC-ingress program (rather than XDP-ingress) on the backend-facing
veth. **Rejected**:

- TC-ingress runs *after* the kernel's networking stack has already
  classified the skb; XDP-ingress runs before. The whole point of
  the pivot is to bypass the kernel forwarder; staying in TC-* is a
  half-measure that re-introduces some of the kernel-mechanism
  exposure that motivates the pivot.
- The asymmetry — XDP-ingress for forward, TC-ingress for reverse —
  has no benefit and complicates the loader (different attach types
  per direction).
- Cilium's reverse-NAT runs at XDP-ingress on the egress-toward-
  client veth, matching this ADR's choice.

### E — Skip the pivot; keep S-2.2-17 disabled

Accept S-2.2-17 as permanently `#[ignore]`'d and continue building on
the pre-pivot architecture for length-0 control-segment paths only.
**Rejected**:

- S-2.2-17 is the only end-to-end gate the dataplane has against a
  real TCP connection through the load balancer. Disabling it
  permanently leaves the dataplane structurally untestable against
  real traffic — every later phase (mTLS via sockops, content-
  inspector sidecars, gateway integration) sits on top of a layer
  whose forward+reverse path has never carried a single byte of
  actual application data.
- The bug class is structural, not edge-case. Length-N segments are
  the dominant traffic shape on any real workload. Punting forever
  is not viable.

## Consequences

### Positive

- **S-2.2-17 closes structurally**, restoring the only end-to-end
  TCP-with-payload gate the dataplane has.
- **Removes a kernel-mechanism dependency.** The pre-pivot path's
  failure mode is sensitive to kernel-version-specific
  `pskb_expand_head` + `skb_checksum_help` interactions on paged
  skbs. The post-pivot path has no such dependency; `bpf_fib_lookup`
  + `bpf_redirect` semantics are stable from kernel 4.18 / 4.8
  respectively (well below the project's 5.10 floor).
- **Aligns with the published reference.** Cilium's L4LB shape is
  the canonical XDP load-balancer shape on stable kernels. Following
  it reduces the project's exposure to "we are the only ones doing
  it this way."
- **Verifier-budget envelope improves.** Two XDP programs replace
  one XDP + one TCX program. Per-program headroom against the 1M
  ceiling grows.
- **Future kTLS / mTLS sockops attach point unchanged.** The
  whitepaper § 7 sockops layer sits above the XDP fast path; the
  dataplane shape change is invisible to it.

### Negative

- **One-time engineering cost.** A new XDP program, an extension to
  the existing one, loader changes, and the deletion of
  `tc_reverse_nat`. Bounded; estimated at one slice cluster's worth
  of work (≈ 3–5 days).
- **veristat baselines are reset.** The post-pivot program shape
  has no historical baseline. Slice 06-05 must record new ones;
  trend tracking restarts. (The Tier 4 gate is per-program-relative,
  not absolute, so this is a single-PR cost, not an ongoing one.)
- **First-packet ARP flake.** A genuinely cold neighbour cache
  produces one `XDP_PASS` per new flow and a slow-path SYN through
  the kernel. The S-2.2-17 test already pre-populates ARP to
  eliminate this in the test harness; production traffic at warm
  cache is unaffected. A future workload-spawning slice may need to
  pre-seed the ARP cache for newly-allocated backend IPs at
  allocation time — out of scope here, captured under #159.

### Operational

- **Test attachment topology changes.** The dataplane loader
  attaches XDP programs on both `lb_veth_a` (client-facing,
  request path) and `lb_veth_b` (backend-facing, response path).
  ADR-0043's 3-iface test topology accommodates this without
  change; the existing tests pass `&topo.lb_veth_a` to the
  loader, the loader internally derives `lb_veth_b` from the
  same topology handle.
- **Dataplane crate boundary unchanged.** `EbpfDataplane` is still
  the only `Dataplane` adapter-host implementation; its internal
  loader gains the second XDP attach call but exposes no new public
  surface.
- **Slice 08 (`ServiceMapHydrator`) unchanged.** The reconciler
  writes into the same SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP via
  the same typed handles. The pivot is below the
  `Dataplane::update_service` trait surface.

## References

- GH #159 — *[2.x] Replace IP-forward + TCX-egress with
  bpf_redirect_neigh datapath* (the tracking issue for production
  work under this ADR).
- `docs/analysis/e1-bpftrace-results.md` probes 1–7 — the empirical
  evidence that falsified ADR-0040 Q2=A.
- `docs/analysis/root-cause-analysis-s-2-2-17-length-n-tcp-drop.md`
  — Rex's earlier RCA, partially superseded by the empirical chain
  but useful for the framing context.
- `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
  § Q1 (`bpf_fib_lookup` mechanic), § Q2 (L2 MAC rewrite mechanic),
  § Q4 (Option α justification), § Finding 4.4 (FIB-miss `XDP_PASS`
  fallback).
- `docs/research/dataplane/cilium-snat-csum-rewrite-prior-art-research.md`
  — incremental checksum patterns; preserved (the L3+L4 csum
  rewrite logic is unchanged across the pivot).
- `docs/research/dataplane/cilium-tcx-egress-attach-loader-research.md`
  — Cilium loader / TCX attach patterns; partially superseded
  (TCX attach is no longer load-bearing, but the loader
  discipline carries over to the dual-XDP-attach shape).
- `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
  § A.1 (`bpf_fib_lookup` and `bpf_redirect` are exposed in
  aya-ebpf 0.1.x via `aya_ebpf::helpers`; no hand-rolled syscall
  surface needed for the pivot).
- `docs/research/dataplane/bpf-redirect-xdp-forward-path-research.md`
  — research confirming `bpf_redirect_neigh` is TC-only; 6
  independent sources with zero contradictions. Basis for this
  amendment.
- ADR-0040 Q2 (reopened by this ADR — see § "Supersession"); ADR-0040
  Q3 amendment (sanity prologue ingress-only — preserved).
- ADR-0041 (REVERSE_NAT_MAP shape + endianness lockstep) — preserved
  in full; only the *layer* of reverse-NAT changes (XDP-ingress
  instead of TCX-egress).
- ADR-0042 (`ServiceMapHydrator`) — preserved in full.
- ADR-0043 (3-iface test topology) — preserved in full.
- `docs/whitepaper.md` § 7 *eBPF Dataplane / XDP — Fast Path Packet
  Processing* — the structural authority that the dataplane is
  XDP-first; this ADR pulls the architecture closer to that
  authority by removing the IP-forward + TCX-egress detour.

## Supersession

This ADR partially supersedes ADR-0040:

- **ADR-0040 Q2=A (TC-egress reverse-NAT, kernel IP-forward in the
  data path) — superseded** by this ADR's § Decision §§ 1–3.
- **ADR-0040 Q1=A (kernel-helper checksum choice) — preserved.** The
  L3/L4 checksum incremental update via `bpf_l3_csum_replace` /
  `bpf_l4_csum_replace` runs on both XDP programs identically.
- **ADR-0040 Q3 (sanity prologue helper, ingress-only after the
  2026-05-07 amendment) — preserved.** Both post-pivot programs are
  XDP-ingress, so the ingress-only scope is structurally satisfied.
- **ADR-0040 Q5 (HASH_OF_MAPS inner-map size 256) — preserved.**
- **ADR-0040 Q7 (DROP_COUNTER 6 slots) — preserved.** No new
  `DropClass` variant required by this pivot.

ADR-0041 and ADR-0042 are unaffected. ADR-0043 is unaffected.

The Q2 reopen is captured as an amendment in ADR-0040 itself
(2026-05-07 revision section), with explicit forward pointer to
this ADR.
