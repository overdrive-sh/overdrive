# Phase 2.16 — XDP per-CPU LRU conntrack: feature brief

**Feature ID**: `phase-2.16-xdp-conntrack`
**GH parent**: #154 (pulled forward from its 2.16 slot)
**Status**: DISCUSS-skipped → DESIGN (see § "Why no full DISCUSS wave")
**Companion**: `phase-2-xdp-service-map` (parent feature; this feature
opens after Slice 06 closes per the design dispatch on 2026-05-06).

## Problem

The Phase 2.2 Maglev forwarder is stateless. Two structural failure
modes surface in real workloads:

1. **Half-tracked flows dropped by kernel netfilter.** The XDP
   forward path bypasses netfilter; the TC reverse path traverses
   it. With `nf_conntrack_tcp_loose=1` (the kernel default), kernel
   conntrack mid-stream-picks-up the response side, infers a
   sequence window from one direction, and silently drops
   payload-bearing TCP segments outside that window. Symptom:
   S-2.2-17 (`real_tcp_connection_completes_through_vip_with_payload_echo`)
   handshakes complete, FIN-ACK delivers, payload retransmits drop.
2. **No flow affinity across SERVICE_MAP rotations.** Maglev's
   ≤ 1 % disruption holds *per single-backend removal*. Operator
   workloads that combine long-lived TCP sessions with multiple
   successive canary rotations within a connection's lifetime
   compose the disruption probability multiplicatively. Aggressive
   canary cutover patterns weaken the §15 *Zero Downtime
   Deployments* claim accordingly.

Both classes are solved structurally by **dataplane-owned
conntrack**: a 5-tuple → backend table the XDP forward path writes
on first-packet-in-flow and reads on every subsequent packet, with
`iptables -t raw -j NOTRACK` rules ensuring kernel netfilter never
sees these flows.

## Why no full DISCUSS wave

Per the design dispatch on 2026-05-06, this feature is opened mid-
Phase-2.2 to address GH #154 as the user-ratified next chunk of
roadmap work. The DISCUSS surface is small enough to fold into the
design wave directly:

- **Stories**: one — *"As an operator deploying canary cutover, I
  want flow affinity preserved across SERVICE_MAP rotations so that
  in-flight TCP sessions are not migrated to a different backend
  mid-connection."*
- **Out of scope**: per-VIP conntrack opt-in (defer to GH #158
  POLICY_MAP); UDP conntrack with longer TTL than TCP (uniform TTL
  in Phase 2.16; per-protocol tuning is a Phase 3+ slice);
  cross-node conntrack synchronisation (each node tracks its own
  flows; cross-node state is a #156-class concern); IPv6 (defer to
  GH #155); IPv4-fragmented packets (drop with
  `DropClass::MalformedHeader` per the existing sanity prologue).
- **Constraints inherited verbatim**: all 10 from
  `phase-2-xdp-service-map/design/architecture.md` § 2 except
  Constraint 2 ("Conntrack is OUT") which this feature flips. New
  constraint: this feature MUST land **after** Phase 2.2 Slice 08
  (hydrator) so the `ServiceGeneration` increment site has a
  stable home in `Dataplane::update_service`.
- **Risks**: enumerated in ADR-0044 § Consequences; the highest
  is the Tier 4 verifier-budget delta (the conntrack lookup adds
  two map operations to the forward hot path; the gate is in
  place before this feature lands per the roadmap-impact ordering
  in ADR-0044).

## Anchors

- **Whitepaper § 7** — eBPF Dataplane / XDP fast path
- **Whitepaper § 15** — Zero Downtime Deployments
- **CLAUDE.md § Principle 12** — Earned Trust (the three NOTRACK
  / map-create / kernel-conntrack-honours-NOTRACK probes)
- **Research** — `docs/research/networking/xdp-service-load-
  balancing-research.md` § 4 (per-CPU LRU is the canonical shape)
- **ADRs** — ADR-0044 (this feature's design lockpoint); composes
  with ADR-0040, ADR-0041, ADR-0042, ADR-0043

## Acceptance flavour

Three DST invariants close the design loop:

- `ConntrackPinsBackendUnderRotation` — Established flows survive
  arbitrarily-many SERVICE_MAP rotations until LRU expiry.
- `ConntrackEventuallyConvergesAfterRotation` — stale-generation
  entries age out within the LRU expiry window.
- `NoTrackProbeRefusesStartOnFailure` — the dataplane refuses to
  start (with structured `health.startup.refused`) if any Earned
  Trust probe fails.

Plus the side-effect: S-2.2-17 GREEN once Slice 16-03 lands.

## Status row

| Field | Value |
|---|---|
| Feature | `phase-2.16-xdp-conntrack` |
| Parent | `phase-2-xdp-service-map` |
| GH | #154 |
| Wave | DESIGN (DISCUSS folded; see above) |
| Owner (DESIGN) | Morgan |
| Acceptance designer (DISTILL) | Atlas (next) |
| Roadmap state | Step IDs proposed in ADR-0044 § Roadmap impact; final structure lands via `/nw-roadmap` after DISTILL completes |
