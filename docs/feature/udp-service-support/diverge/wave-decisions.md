# DIVERGE Decisions — udp-service-support

> **Location note.** DISCUSS-wave decisions live inside
> `../feature-delta.md` § "Wave decisions" (D1–D8). This file holds the
> DIVERGE-wave decisions for the SCOPED option study that closes review
> finding **H3** ("simpler alternative unweighed"). It does NOT supersede
> feature-delta.md; it provides the scored evidence that feature-delta.md's
> D1 lock asserted without. The DISCUSS-revision author updates D1 + C2 +
> US-01 in feature-delta.md per `../recommendation.md` § "What the DISCUSS
> revision should say."

## Scope

ONE architectural decision: how to thread per-service L4 protocol
(`Proto::Tcp`/`Proto::Udp`) through `Dataplane::update_service` so
production `EbpfDataplane` installs `REVERSE_NAT_MAP` entries matching the
declared proto (GH #163). NOT a full new-product divergence.

## Key Decisions

- **[DV1]** *From-state is shipped option C* (`update_service(vip:
  Ipv4Addr, backends)`, `dataplane.rs:101`), not locked-A. Every option is
  a transition from C; locked-A is a paper decision never landed. (closes
  review B1) — verified against source.
- **[DV2]** *Recommendation: Option 6 (`ServiceFrontend` newtype) at 4.17,*
  co-leader Option 1 (positional proto) at 4.13, both ahead of the user's
  preferred Option 2 (typed aggregate) at 3.57 on a locked developer-tool
  taste matrix. The simpler thread-proto family wins; the aggregate is the
  documented dissent. (closes H3)
- **[DV3]** *service_id reconciliation:* the frontend/descriptor carries
  `(ServiceVip, port, Proto)`; `service_id` + `correlation` stay on the
  `Action::DataplaneUpdateService` envelope by design (it is an
  action-routing concern, not a dataplane-key concern — SERVICE_MAP key is
  `(VIP,port)`, REVERSE_NAT key is `BackendKey{ip,port,proto}`). This gives
  C2 a precise grep-checkable pass condition. (closes B2)
- **[DV4]** *Lockstep pinning:* in-process both-adapter Tier 1 retarget is
  INFEASIBLE (real `EbpfDataplane` needs a kernel — review H1). Resolved at
  DIVERGE: pin via **Tier 1 (Sim set-equality over `BackendKey`) + Tier 3
  acceptance (real Ebpf `bpftool` dump) + Tier 2 `BPF_PROG_TEST_RUN`
  triptych**. US-03's "OR Tier 3" collapses to "Tier 1 Sim AND Tier 3 Ebpf
  acceptance." (resolves H1 — no SPIKE inside the slice it gates)
- **[DV5]** *Sim hardcode narrowing (review H2):* US-01 narrows
  `reverse_nat_keys_for`'s `[Tcp,Udp]` (`sim/dataplane.rs:277`) to the
  threaded `proto`; the existing two-proto Sim assertions in
  `reverse_nat_lockstep.rs` (lines 123, 161) are updated in the same PR.
  This is "zero *production* behavior change," not "zero behavior change."

## Job Summary

- Validated jobs (NO new job): **J-OPS-004** (operator wire-trust) +
  **J-PLAT-004** (dataplane-correctness / lockstep). Rides existing jobs.
- ODI outcomes: 5 (O1–O5 in `job-analysis.md`). O1–O3 (wire correctness)
  are served identically by all three top options; **O4 (scattered args)
  and O5 (extension cost) are the discriminators** the taste matrix scores.

## Options Evaluated

- **6 options generated** (SCAMPER 7 lenses + 2 Crazy-8s, curated from 7;
  option 7 "two methods" merged out as a variation). All 6 passed the
  3-point diversity test.
- **6 survived the DVF filter** (none < 6; option 4 weakest at DVF 8).
- **Recommended: Option 6 (`ServiceFrontend` newtype) — 4.17** — proto
  threaded as the forward-path twin of the existing `BackendKey`; trivial
  lockstep; smallest blast radius compatible with newtype-STRICT.
- **Dissent: Option 2 (typed aggregate) — 3.57** — the user's standing
  preference; wins only if multi-listener becomes a trait-surface concern
  OR the team commits to `update_service`-as-typed-SSOT (an explicit
  weight-profile change, documented in `recommendation.md`).

## SSOT Updates

- **jobs.yaml: UNCHANGED.** J-OPS-004 + J-PLAT-004 already exist and are
  `active`; this DIVERGE rides them. No new job minted (would fragment
  J-OPS-004 per protocol). Optional non-blocking changelog note (review M2)
  deferred to the DISCUSS-revision author.
- **No ADR edits.** The phase-2 architecture.md §5 Q-Sig amendment (C →
  the chosen thread-proto family, superseding paper locked-A) is the
  architect's job in DESIGN — forward-pointed only.
- **No GitHub issues created.**

## Hand-off

- **To:** DISCUSS-revision author (update feature-delta.md D1/C2/US-01 per
  `recommendation.md`), then **product-owner** for the revised DISCUSS, then
  **solution-architect** (DESIGN) for the ADR amendment + the
  newtype-vs-positional secondary choice + H1's Tier-1/Tier-3 split.
- **Deliverables:** `../recommendation.md` + the five diverge artifacts +
  `review.yaml`.
