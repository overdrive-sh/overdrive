# Independent DESIGN review — vsock Attachment Replacement (revision 11)

- Reviewer: independent `nw-solution-architect-reviewer` (Opus), 2026-10-08
- Under review: `docs/feature/netns-density-295/feature-delta.md` § "vsock Attachment Replacement DESIGN" (L16552–21263), ADR-0145 … ADR-0176, `specs/quint/guest-flow-owner/` (specs, `hazard/`, `checks.toml`, `README.md`, `evidence/summary.md`), `design/quint-guest-flow-owner-findings-r8.md`
- Prior review: `design/review-design-vsock-replacement-r10.md` (CHANGES_REQUESTED)
- Recorded by the orchestrator from the reviewer's returned output.

## Verdict: CHANGES_REQUESTED

BLOCKER 0 · HIGH 2 · MEDIUM 2 · LOW 4

H-1 blocks approval under `.claude/rules/design.md` § "Surface material design decisions". H-2 repeats r10 H-4 for round 8.

## HIGH

**H-1 — Three architect-derived consequences are recorded as user-approved; none is marked pending the user's confirmation.**
- Location: (1) boot refuses on any NIC unframe attach failure — delta L16739 (D5a-IFACE row), L21009–21011, L18237–18240, G-V1 L19286–19287 and L19306–19308; ADR-0165 Decision and the rejected alternative "Boot proceeds with a NIC that failed to attach"; ADR-0165 Status "approved … D5a-IFACE, 2026-10-08". (2) only kernel-memory steer failures retry — delta L16737 (D8a-CAP row), L20996–20999, L18374–18383; ADR-0176 Decision "What remains retried"; ADR-0176 Status "approved by user 2026-10-08". (3) A-MEM tied to #261 — delta L20294, README L114. L19901–19905 and L20987–20989 cover all three under the approval label.
- Defect: these consequences need the user's confirmation but sit inside the approved decision rows and ADR Decisions with no "pending confirmation" marker; a reader cannot tell what the user approved from what the architect added.
- Substance: none is wrong — (1) agrees with G-V1 and removes the boot-time exposure; (2) is the right classification under admission (capacity unreachable, so non-ENOMEM failures are defects); (3) is the right owner (#261 covers node-wide CPU and memory accounting, delta L475–476).
- Failure scenario: DISTILL writes R5-38 (boot refusal) and the R5-22 / R5-37 retry classification as approved contracts; if the user rejects (1), accepted downstream artifacts encode a decision the user never made.
- Rule: `design.md` § "Surface material design decisions to the user before they become accepted". Fix: mark each "Proposed — derived from D5a-IFACE / D8a-CAP; pending user confirmation" in the decision table, revision-11 items, ADR-0165 / ADR-0176 Status and A-MEM; surface them as numbered decisions; record the dated confirmation before DISTILL.

**H-2 — The Formal protocol model section does not record round 8, which has run (r10 H-4 again).**
- Location: delta L19923–19927 (Status: latest recorded run is round 7; "No result is claimed"), L20220 ("Round 8 (to run, revision 11)"), L21118–21119 (review record: "model round 8 runs first"), L19894–19895 (round 8 listed as a DISTILL blocker), L20118–20121 (`udp-slots-hazard-ignore-late-paired` "stays `ci = true`" while `checks.toml` L348 has `ci = false`; `intake-safety-apalache` L776 is also `ci = false`, unmentioned).
- Defect: round 8 ran (run `704554-1791465879.649`, 132/132 matched, `verify-evidence` passed; findings-r8 L10–13; `evidence/summary.md` L7–8; commit `93a656767`). The delta still presents it as future work, claims no result for D8a-CAP, invariant 12 or M-2 / M-4, and states a CI flag contradicting `checks.toml`.
- Failure scenario: DISTILL (spec as oracle, ADR-0168) cannot tell whether invariant 12 and `ServingReachable` were checked against revision 11.
- Rule: `design.md` § "Concurrent protocols carry a model-checked Quint specification". Fix: record round 8 (run id, 132/132, per-module verdicts, invariant 12 holds, `ServingReachable` holds, the `noIntakeAdmission` teeth, CI-flag changes); update Status header, review record and What blocks DISTILL; correct L20118–20121.

## MEDIUM

**M-1 — A-MEM's fairness differs between delta and spec; the delta's statement does not support the property.** Delta L20065–20068 ("under weak fairness … its `steer` fails only transiently"), L20225 ("under weak fairness a retry eventually succeeds"), L20294 (A-MEM: "weak fairness on the retry"); spec `intake.qnt` L46–48 and L354–358 (`strongFair(Steer(1), …)`), README L114 ("strong fairness"). r8 L31–35: a failed attempt closes the listener, so the steer is only intermittently enabled and weak fairness would not carry `ServingReachable`. Rule: `design.md` § "Concurrent protocols". Fix: restate A-MEM as "a `steer` retried infinitely often eventually succeeds (strong fairness)" at L20065, L20225, L20294.

**M-2 — The runtime-registered-interface exposure is called "bounded" but is unbounded, and it retries every failure, including defects.** ADR-0165 Consequences L103–108, rejected alternative L89–92; delta L18244–18253, L19147. The exposure lasts "while the kernel keeps refusing the attach" (no bound) — the same reason ADR-0165 rejects the boot alternative. The runtime path retries every `UnframeAttachError` at the audit cadence, whereas D8a-CAP classifies analogous steer failures (ENOMEM retried; defects not retried). On the appliance (A-31), a late NIC carries guest egress only after it has addresses and routes from the node network configuration (D8-PREFIX-CFG names that producer), so making the attach a precondition of configuring the interface would remove the exposure. Failure scenario: a hot-added NIC whose TCX attach fails deterministically (a defect) carries framed guest datagrams indefinitely, signalled only by a counter and a per-audit retry event. Rule: `design.md` § "Design the correct mechanism". Fix: gate configuration of a late interface on its attach, or classify attach failures like steer failures and state the real bound; correct "bounded" in ADR-0165 either way.

## LOW

- **L-1** `ci = true` handling is inconsistent: `udp-slots-witness-late-paired` (`ci = true`) recorded 138.1 s (`evidence/summary.md` L41) because attempt 1 hit the runner's 120 s start-up bound (attempt 2 took 18 s), yet round 8 set `udp-slots-hazard-ignore-late-paired` to `ci = false` for the same cause. The cause is the runner, not the check: fix the runner's start-up handling or apply one rule to both, and record it.
- **L-2** `GuestSteeringError::MapLookup` and `Query` (delta L18384–18387) name no producing method now that `verify` and `inventory` are deleted. Name the producer or delete the variants (`development.md` § "Trait definitions specify behavior").
- **L-3** A-MEM (L20294) omits that the steering map's entries and psocks are charged to the memory cgroup of the `serve` process (`overdrive.slice/control-plane.slice`), which sets no `memory.max` today; state it so a future slice limit cannot silently make `KernelMemory` a persistent self-inflicted producer.
- **L-4** § *Changed assumptions* (L20529–20585) quotes superseded drafts of this unimplemented design (revision 5 foreign-route refusal, revision 8 element set, revision 9 fence, D5a-SET refresh, forwarder link damage) as superseded contracts. Keep the section to operative contracts (CAP-295-A, ADR-0114, …); leave draft history to git and the review record.

## Prior findings (r10)

| ID | Status | Evidence |
|---|---|---|
| H-1 | Resolved | D8a-CAP / ADR-0176: map 262,144, `assign` reserves declared TCP ports, `IntakeCapacityReached`, capacity retry removed; invariant 12 and `ServingReachable` hold; `noIntakeAdmission` (+`capRetry`) teeth violate (r8 L94–108); CAP-295-A change surfaced and approved (L20560–20567). Derived consequences: new H-1 |
| H-2 | Resolved | D19-LINKS: components deleted (L19129–19139), lifetime contract L18264–18269, no link-detach injection (L20401–20402), producers named for what stays |
| H-3 | Resolved | ADR-0152 limited to the route; ADR-0172–0175 split out; ADR-0171 cites 0175 and 0176 by reference |
| H-4 | Resolved for round 7; recurred for round 8 | New H-2 |
| M-1 | Resolved in framing | Residual runtime exposure: new M-2 |
| M-2 | Resolved | `quiescence` route single form; `otherRepairsPrefix` generic; check renamed; `GuestPrefixSteeringConverge`; comments fixed |
| M-3 | Resolved | `PinnedObjectForeign` deleted; `GuestPrefixOverlapsNodeAddress` with named producer, checked in `probe_startup` |
| M-4 | Resolved | Invariant 6 states "pinned"; `LocalOnlyOverLink` checks `link == "pinned"` |
| L-1 | Resolved | "revisions 5–11"; no stale `steering` or K-L6 references |
| L-2 | Resolved | "Approved by user 2026-10-06" |
| L-3 | Resolved | `udp-slots-release-liveness` `ci = false`; new overrun in new L-1 |
| L-4 | Resolved | V-9 measures `sk_lookup` per-lookup cost |
| L-5 | Resolved | ADRs 0171–0176 each carry their own approval line |

## Passed

- Approvals: D8a-CAP, D19-LINKS, D5a-IFACE, D8-PREFIX-CFG each carry a dated user approval citing the reviewed finding; rejected alternatives recorded.
- Appliance threat model: no remaining audit, injection or refusal defends against an absent actor.
- Correct mechanism: capacity admitted at `assign` under the same lock, before the CID claim; steer-time capacity retry gone; A-33, K-L9, K-L3 each have a discharging item.
- Kernel mechanism first: `sk_lookup` over a sockhash; ADR-0171 names and rejects the nft alternatives.
- One ADR per decision: ADRs 0152 and 0171–0176 each state one decision; no signatures, test matrices or roadmap steps in ADRs 0145–0176; no amendment narration.
- Lifecycle Gate Ownership: G-V0, G-V1, G-V8 updated with all required fields; R5-37 and R5-38 added.
- Formal model (spec side): round 8 132/132; every new rule has a violating hazard; `ServingReachableNoMemRecovery` shows what A-MEM carries; not-modelled list complete.
- Vertical slice: boot refusals land in `run_server`, admission refusals in the shim; every path through `serve` + `deploy`.
- Deferrals: none new; #93 and #308–#312 cited.
