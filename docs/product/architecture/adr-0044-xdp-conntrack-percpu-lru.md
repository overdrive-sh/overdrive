# ADR-0044 — XDP per-CPU LRU conntrack table + flow-affinity-over-SERVICE_MAP-changes

## Status

**SUPERSEDED — empirically falsified, see § Falsification.** 2026-05-07.

Originally proposed 2026-05-06 by Morgan as the Phase 2.16 design
lockpoint. The S-2.2-17 root-cause hypothesis that motivated this
ADR's Decision 4 (NOTRACK installation) was empirically falsified by
a Lima-side bpftrace + netstat + pcap diagnostic on 2026-05-07. The
companion Phase 2.16 feature (`docs/feature/phase-2.16-xdp-conntrack/`)
is retracted. The actual fix lives in an amendment to ADR-0040 §
Decision 4 (Q3 — sanity prologue scope) and is unrelated to
conntrack. GH #154 remains open with its original scope (flow-affinity
across SERVICE_MAP rotations); the urgency story dissolves and there
is no longer a justification for pulling it into Phase 2.2's slice
sequence.

## Falsification (2026-05-07)

The S-2.2-17 acceptance test
(`real_tcp_connection_completes_through_vip_with_payload_echo` in
`crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs`)
shows a content-specific drop pattern: length-0 TCP segments pass
through the XDP forward + TC reverse-NAT path; length-N data
segments are dropped between `lb_b` ingress and `lb_a` egress.

Three hypotheses have been entertained over the course of this
investigation. Two are now empirically falsified:

1. **Conntrack INVALID drop** (the hypothesis this ADR was built on)
   — falsified by an iptables `-t raw -j NOTRACK` A/B test that left
   the symptom unchanged. Adding the NOTRACK rule did NOT close
   S-2.2-17. The kernel netfilter conntrack interference framing in
   § Context above is wrong; conntrack is not in the drop path at
   all.
2. **`bpf_l4_csum_replace` direct-form + `BPF_F_PSEUDO_HDR`
   interaction with `CHECKSUM_PARTIAL` on length-N skbs** (the
   research-recommended Decision 6′ alternative in the companion
   research doc) — falsified by a 5-step diagnostic on Lima:
   - bpftrace shows the drop reason is `SKB_DROP_REASON_TC_EGRESS =
     51` from `dev_queue_xmit` on `lb_a`. The drop is at TC egress,
     not in conntrack and not in csum validation.
   - `Tcp.InCsumErrors` = 0 → 0 across the test run (no checksum
     errors at the receiver).
   - The `[P.]` data segment never reaches `lb_a.pcap` (drop is
     upstream of any post-rewrite checksum validation).
   - A/B removing `BPF_F_PSEUDO_HDR` does not change the symptom —
     the helper-flag interaction is not load-bearing.
3. **Real cause** (verified): Slice 06-02's sanity prologue helper,
   specifically the `claimed_pkt_len > packet_len` check at
   `crates/overdrive-bpf/src/programs/sanity.rs:259`. When the
   kernel forwards an skb to TC egress, the IPv4 `total_length`
   field includes the full L4 payload but the skb's linear-buffer
   length (`data_end - data` in BPF context) may not — skb
   linearisation, GSO, and forwarded-packet metadata can leave the
   linear region shorter than what `total_length` advertises.
   Length-0 segments pass because `total_length == header_bytes`.
   Length-N segments fail check (3) because
   `claimed_pkt_len = ipv4_offset + total_len` exceeds `packet_len`
   for forwarded skbs.

The only path in `tc_reverse_nat` returning `TC_ACT_SHOT` is
`Verdict::Drop` from the sanity prologue, which is also the only
path that fires `SKB_DROP_REASON_TC_EGRESS` on the egress side. The
bpftrace drop count matches the retransmit count from netstat —
every length-N segment is dropped by the prologue at TC egress.

## Why this matters for the architecture record

ADR-0040 § Decision 4 (Q3=C) decided the sanity prologue is a shared
`#[inline(always)]` helper invoked from both `xdp_service_map_lookup`
(ingress) and `tc_reverse_nat` (egress). That decision was correct
for ingress but wrong for egress. The egress program operates on
already-trusted skbs that have been validated at ingress, and the
kernel's skb model does not preserve "linear bytes match
`total_length`" through forwarding. The fix is to scope the prologue
**ingress-only**; the egress program should not call it.

That fix is captured as a revision section in
`docs/product/architecture/adr-0040-service-map-three-map-split-and-hash-of-maps.md`
(Revision 2026-05-07 — Q3 amendment). The conntrack-shaped fix this
ADR proposed is unnecessary; deleting the prologue's egress-side
invocation closes S-2.2-17 directly.

## Original-design retention

The body below preserves the original 2026-05-06 design as a
historical record. Future architects landing in this corner of the
design space (e.g., when GH #154's flow-affinity-across-SERVICE_MAP-
rotation work is genuinely picked up) should read it for the typed-
key/value shapes, the per-CPU LRU rationale, and the Earned Trust
probe surface — those primitives stand on their own. They should NOT
read it as a recipe for fixing a specific symptom; the falsification
above is the load-bearing record.

Cross-references for the falsification trail:

- ADR-0040 amendment (Revision 2026-05-07) — the actual fix.
- `docs/research/dataplane/length-n-tcp-drop-veth-xdp-tc-reverse-nat-research.md`
  § Update 2026-05-07 — RECOMMENDATION FALSIFIED — the
  research recommendation that motivated Decision 6′ is recorded as
  empirically falsified.
- `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
  § Update 2026-05-07 — the conntrack hypothesis was downstream of
  this research's veth-peer-XDP_TX delivery prediction; the
  research's primary findings (Option α `bpf_fib_lookup` + L2 MAC
  rewrite, landed in commit `c9f80c7`) remain valid. The conntrack
  inference downstream of those findings does not.

---

## Original ADR (2026-05-06) — retained as historical record

> The text below is the original 2026-05-06 proposed ADR. It is
> SUPERSEDED in its entirety per § Falsification above. Do NOT treat
> any decision below as binding.

### Proposed Status (original, 2026-05-06)

Proposed. 2026-05-06. Decision-makers: Morgan (proposing). Tags:
phase-2, dataplane, conntrack, kernel-maps, xdp, l4lb, lru, percpu,
flow-affinity.

**Companion ADRs**: ADR-0040 (three-map split + HASH_OF_MAPS atomic-
swap primitive), ADR-0041 (weighted Maglev + REVERSE_NAT shape +
endianness lockstep), ADR-0042 (`ServiceMapHydrator` reconciler +
`Action::DataplaneUpdateService`), ADR-0043 (three-iface transit
test topology).

**Related issues**: GH #154 (Phase 2.16 conntrack — pulled forward
from its 2.16 slot to immediately follow Phase 2.2 Slice 06; see
§ Decision 1 below).

### Original Context

Phase 2.2 ships a *stateless* Maglev forwarder. The flow-affinity
guarantee for the stateless forwarder is the M ≥ 100·N rule:
≤ 1 % of 5-tuples remap on a single-backend removal (research
§ 5.2; ASR-2.2-02). This is sufficient for *most* canary cutover
shapes — until two structural failure modes meet a real workload:

1. **Stateless XDP bypass meets kernel netfilter conntrack.** The
   forward path runs in XDP and `bpf_redirect()`s the rewritten frame
   to a peer iface; netfilter's `nf_conntrack_in()` never sees the
   forward direction. The reverse path (TC egress reverse-NAT)
   *does* traverse netfilter's hooks. Conntrack's TCP tracker, with
   `nf_conntrack_tcp_loose=1`, mid-stream-picks-up a flow it only
   ever saw in one direction and cannot validate the sequence
   window. With `nf_conntrack_tcp_be_liberal=0` (the kernel
   default), payload-bearing segments outside the inferred window
   are flagged INVALID and silently dropped in `nf_conntrack_in()`.
   Length-0 segments (SYN-ACK, ACK, FIN-ACK) trivially pass the
   window check and survive; length-N segments with `seq` outside
   the inferred window do not. Symptom observed in S-2.2-17
   (`real_tcp_connection_completes_through_vip_with_payload_echo`):
   handshake completes, FIN-ACK delivered, every retransmit of the
   payload segment dropped.

   **FALSIFICATION 2026-05-07**: this framing is wrong. See § Falsification at the top of this ADR.

2. **No flow affinity across SERVICE_MAP backend-set changes.**
   Maglev's ≤ 1 % disruption holds *per single-backend removal*.
   Operator workloads that combine long-lived TCP sessions with
   aggressive canary cutover (multiple successive backend-set
   rotations within a connection's lifetime) compose the disruption
   probabilities multiplicatively. A 1 %-per-rotation × 10
   rotations within the lifetime of a connection = ~10 % cumulative
   misroute probability. The §15 *Zero Downtime Deployments* claim
   weakens accordingly.

   **NOTE 2026-05-07**: this concern is real and remains GH #154's
   scope, but it is independent of the falsified S-2.2-17 framing
   above. There is no urgency story attaching it to Phase 2.2; it
   belongs in its original 2.16 slot.

The original Decisions 1–8, Alternatives, Consequences, Roadmap
impact, Cross-references, and Changelog sections from the 2026-05-06
draft are preserved verbatim in the git history at this file's last
pre-supersession commit. They are not re-presented here because
every decision is structurally tied to the falsified hypothesis in
§ Original Context #1, which the § Falsification section at the top
of this ADR replaces.

## Changelog

| Date | Change |
|---|---|
| 2026-05-06 | Initial draft. Six decisions; four alternatives considered; six-step roadmap-impact statement. — Morgan. |
| 2026-05-07 | SUPERSEDED — empirically falsified per § Falsification. Status changed to SUPERSEDED. Original-design body collapsed to a single retained-as-historical-record reference. — Morgan. |
