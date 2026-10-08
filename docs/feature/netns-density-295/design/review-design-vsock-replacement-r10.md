# Independent DESIGN review — vsock Attachment Replacement (revision 10)

- Reviewer: independent `nw-solution-architect-reviewer` (Opus), 2026-10-08
- Under review: `docs/feature/netns-density-295/feature-delta.md` § "vsock Attachment Replacement DESIGN" (L16552–20867), ADR-0145 … ADR-0171, `specs/quint/guest-flow-owner/` (specs, `hazard/`, `checks.toml`, `README.md`, `evidence/summary.md` + `summary.json`), `design/quint-guest-flow-owner-findings-r7.md`
- Prior review: `deliver/review-design-vsock-replacement.md` (revision 4, CHANGES_REQUESTED)
- Recorded verbatim by the orchestrator from the reviewer's returned output.

## Verdict: CHANGES_REQUESTED

BLOCKER 0 · HIGH 4 · MEDIUM 4 · LOW 5

No blockers. The reviewer blocks on H-3 per `.claude/rules/design.md`; the other HIGH findings exceed the approval threshold.

## HIGH

**H-1 — The steering map caps open intake listeners at 65,536 with no admission control; a full map becomes a retry loop that never succeeds.**
- Location: delta L18200–18201 (`GUEST_INTAKE_STEERING_MAX = 65_536`), L18602–18604 and L19322–19324 (a steer failure is retried at the audit cadence), L18322–18330 (`activate` is all-or-nothing on steer), ADR-0171 L62–63. Previously accepted capacity contract: delta L7908–7911 ("Any valid distribution with `M > 65,536` remains supported product semantics").
- Defect: intake listeners are bounded by the map size, but neither the node admission placeholder (16,384 attachments) nor any declared-port bound accounts for that limit. T1-PORT4 (16,384 × 4 ports) fills the map exactly. Any further listener fails `MapUpdate`, deterministically until another listener closes. The design treats it as transient: close the listener, retry at the audit cadence, keep refusing the port. The model shows the exposure without bounding it: `intake-steer-forever-unreachable` (summary L95) is a violation of `ServingReachable`.
- Failure scenario: 16,384 VMs each declare 4 ports and the map is full. VM 16,385 is admitted (one lease free after a retire), boots to READY, and fails `activate` with `IntakeSteer { source: MapUpdate }`, projected to `WorkloadNetnsProvisionFailed`, indistinguishable from a bind failure. Or an Active VM's guest opens a 5th port: the port stays refused forever; the only signal is a counter.
- Rule: `design.md` § "Design the correct mechanism — never a fallback around a missing fact" (a retry compensates for a capacity fact the design does not admit) and § "Surface material design decisions" (the M > 65,536 capacity semantics changed without a user decision). Fix: admit intake capacity (count listening declared ports against the map, typed capacity refusal), or size the map from the admitted bound; surface the changed capacity contract to the user.

**H-2 — Shared-link damage detection and its gold-test injection defend only against an actor that cannot exist on the appliance.**
- Location: delta L20113–20117 (Self-application: "deletes each shared link mid-run (`lo` unframe, `sock_ops`, drain counter) and checks that `audit_shared` reports the component damaged"; "detaches a non-`lo` unframe link"), V-8 L19491 ("host link detach"), `audit_shared` contract L18379–18383.
- Defect: these links are process-owned BPF links (ADR-0159). While `serve` holds the file descriptor, only an explicit `BPF_LINK_DETACH` or an outside link deletion removes them; on the appliance nothing but Overdrive does that (A-31). Revision 9 removed the steering audit by exactly this argument ("every runtime write reports its outcome to the owner; no drift of Overdrive's own exists"). The forwarder links have the same property but keep an audit-damage → quiesce → recovery path, and the gold test injects the foreign detach.
- Failure scenario: DISTILL and DELIVER build and test a recovery path whose only trigger is a fault injected by the test itself — dead code wearing a green test, costing quiescence machinery for every flow on the node.
- Rule: `CLAUDE.md` § "Overdrive runs on its own appliance OS" ("When a proposed recovery, audit, fence or retry path defends only against an actor that cannot exist on the appliance, drop it."). Fix: name the Overdrive-own producer of each audited link fault (e.g. the netdev of a TCX link is deleted), or drop link-damage audit, recovery and injection for these links, as was done for the steering.

**H-3 — ADR-0152 is a design bucket, and ADR-0171 bundles an independently reversed decision.**
- Location: ADR-0152 (title and Decision points 1–7); ADR-0171 Decision bullets "Loaded, swapped in, then the route (D8a-LOAD-SWAP)" and "Converged at boot only".
- Defect: ADR-0152's title alone joins three choices (the address on both sides; local delivery with steering; host-initiated flows presenting the real client). Its seven points include route convergence, reachability steering, intake-listener existence, establishment-time install, host-initiated flow, client identity and nft rule removal — each independently acceptable or reversible. ADR-0171 records D8a-LOOKUP (2026-10-07) together with D8a-LOAD-SWAP (2026-10-08), which was already decided independently (it replaced D8a-FENCE while D8a-LOOKUP stood). The prior review's M-3 split only ADR-0149 and ADR-0158.
- Failure scenario: a future reversal of the boot order (e.g. V-26 falsifying K-L9, which "returns D8a-LOAD-SWAP to DESIGN", delta L19703) must supersede ADR-0171's whole steering mechanism, or edit it in place.
- Rule: `design.md` § "One ADR records one decision". Fix: split ADR-0152 (at minimum: route plus boot convergence; reachability; intake and host-initiated flow; client identity), and move D8a-LOAD-SWAP into its own ADR.

**H-4 — The "Formal protocol model" section does not reflect the run that exists, and the spec README maps assumptions to a deleted validation item.**
- Location: delta L19957–19979 ("Next round (to run, revision 10…)"), L16631–16633 (status header: `prefix_boot` "is to be re-run"), L20741–20743 (review record: "Not yet re-reviewed; the next model-check round re-runs `prefix_boot`"), L19876–19880 (hazard list still names "revoke-failure, … route-verification"); `README.md` L109 (K-L9 → V-27), L110 (A-PROG → "Tier-3 steering tests in CI (V-26)"); delta L20013–20014 and L20644 (K-L9 → V-26 (viii); A-33 → R5-1/2/3/25 in CI; V-27 listed as not part of this design).
- Defect: round 7 ran (run `679777-1791451723.128`, 120/120 matched `expect`), but the delta records neither its results nor answers to r7's open points (pin sub-step and refusal outcome to compare against the module; K-L9 → V-27 vs V-26 (viii); A-PROG vs A-33; two `ci = true` checks over 2 minutes). The README maps K-L9 to a validation item the design deleted and names the A-33 assumption differently from the delta.
- Failure scenario: DISTILL, using the spec as its oracle (ADR-0168), cannot tell which item discharges K-L9 or whether the cited evidence is current; a re-review cannot confirm from the delta that the checks for the current text ran.
- Rule: `design.md` § "Concurrent protocols carry a model-checked Quint specification". Fix: record round 7 in the section, close r7's points, align README (K-L9 → V-26 (viii), A-PROG → A-33), replace the stale hazard list.

## MEDIUM

**M-1 — D5a validation and exposure are framed for non-appliance actors.** V-17 (L19500: "an interface owned by other software (a container bridge, a WireGuard or tunnel device, an interface carrying another TCX program)"), A-21 (L16789), ADR-0165 Consequences L97–100, and policy-based xfrm. On the appliance, root-namespace interfaces are the image's NICs plus those Overdrive creates; no third-party software owns interfaces or runs IPsec policies. The accepted exposure (an empty datagram reaches a remote peer 8 bytes long on an un-attached interface) is justified against absent actors instead of being removed for Overdrive-created interfaces (e.g. make the attach a precondition of Overdrive bringing an interface up). Rule: `CLAUDE.md` appliance section; `design.md` ("an accepted exposure that a different contract would remove"). Fix: re-derive V-17, A-21 and the D5a exposure under APPLIANCE (A-31); keep only Overdrive-own and kernel cases (runtime link appearance, notification loss).

**M-2 — The spec still carries revision-9 semantics and terms.** `quiescence.qnt` L69 and L126 (`route: local | fence`; `otherRepairsPrefix` writes `"fence"`), check `quiescence-hazard-fail-stop-fences`; `shared_table.qnt` L58, L106 and `intake.qnt` L28 ("verified, probed"); `prefix_boot.qnt` L20 (`GuestPrefixSteeringUnverified` where the delta has `GuestPrefixSteeringConverge`). r7 finding 3 acknowledges these but leaves them unfixed. Since the spec is the DISTILL oracle, quint-connect drivers would be written against those terms. Rule: `design.md` ("When the design changes, the spec changes in the same revision"). Fix: model a generic runtime route or steering write as the hazard, correct comments and the error name, re-record.

**M-3 — Two typed refusals have no named appliance producer.** `GuestSteeringError::PinnedObjectForeign` (L18253–18256: "Not reachable on the appliance (A-31)"); `ForeignGuestPrefixRoute` (L18372, L18988, L20102–20103; README L128–129 lists it as not modelled). The delta declares one variant unreachable; the other refuses an "untagged route overlapping the prefix" without naming who writes one on the appliance. If the producer is a host address configured inside the guest prefix (a real configuration conflict), say so and name it accordingly; if none, both are surface defending against absent actors. Rule: `CLAUDE.md` appliance section; `development.md` § "Ground the premise". Fix: name the producer, or delete the variants.

**M-4 — The claimed invariant and the checked invariant differ.** Delta invariant 6 (L19836–19837: `local` "only while a pinned steering link exists") vs `prefix_boot.qnt` `LocalOnlyOverLink` (L204: `link != "none"`, which also admits an attached, unpinned link). `OpenConverged` covers the open state, but the safety claim stated in the delta is not the one checked across crash states. Rule: `design.md` § "Concurrent protocols". Fix: check `link == "pinned"`, or restate invariant 6.

## LOW

- **L-1** Stale cross-references: L19706 ("re-review of revisions 5–9"; should be 5–10); R5-23 evidence lane "Quint `steering`" (L19387; module was split); header L16602 ("A-30 (kernel facts K-L1–K-L6)").
- **L-2** U-1 recorded as "pinned at the user's direction" in the approvals table (L16681) and decision index (L16857), while § *Model-check decisions* (L20355–20358) says the user approved every item on 2026-10-06. State U-1 as "approved by user 2026-10-06".
- **L-3** `checks.toml` L244: `udp-slots-release-liveness` is `ci = true` at 148.6 s; `design.md` reserves `ci = true` for checks under 2 minutes.
- **L-4** The `sk_lookup` program runs on every unmatched TCP SYN and UDP lookup in the host root namespace, not only prefix traffic. V-9 does not measure its per-lookup cost at density.
- **L-5** ADR-0152's Status block is a multi-date approval chain (L5–17). Permissible as approval provenance, but once split (H-3) each ADR should carry only its own decision's approval line.

## Prior findings

| ID | Status | Evidence |
|---|---|---|
| B-1 | Resolved | D26 / ADR-0167: deny set covers loopback, link-local and locally delivered addresses, atomic marked-socket output rule (V-21) |
| B-2 | Resolved (in design; V-26 open, blocks DISTILL) | D8a-LOOKUP / ADR-0171; `NoWildcardReached` checked in `prefix_boot`, `intake`, `shared_table` |
| H-1 | Resolved | Read-then-install order; ADR-0158; probe stage 4 (L20050–20051) |
| H-2 | Resolved | D25: reserved ports 61,000 and 61,001–62,024; D25-BIND; `ParseError::ListenerPortReserved` |
| H-3 | Resolved | D24a: VIP branch in mesh resolution; `MeshUnreachable` |
| H-4 | Resolved | `converge_shared` route convergence (L18362–18378); `sweep_stale` limited to process-scoped objects |
| H-5 | Resolved | D20 correction; V-13 extended to remote peers |
| H-6 | Resolved | G-V0 and G-V1 carry every required field (sampled L19073–19139) |
| H-7 | Resolved | V-7 withdrawn (KVER); V-10, V-12, V-15 justified (L19493–19498) |
| M-1 | Resolved | Listener-tag identity; V-20; probe stage 5 |
| M-2 | Resolved as requested, now conflicts with APPLIANCE | See new M-1 |
| M-3 | Resolved for 0149 and 0158 | New H-3: ADR-0152 and ADR-0171 bundle decisions |
| M-4 | Resolved (claimed; roadmap not inspected in depth) | Amendment-pointer table; roadmap steps 12–15 |
| M-5 | Resolved | `InterfaceIndex`, `ListenPort` (L20147–20148) |
| M-6 | Resolved | Lag load profile; V-9 |
| M-7 | Resolved | Quiescence closes listeners (K-L3); per-allocation order; `QuiescedClosesListeners` checked |
| M-8 | Resolved | UDP is TCP-resolve-free, as today |
| M-9 | Resolved | D16-CLAIM / ADR-0170; `cid_lease` invariants and three hazards with violations |
| M-10 | Resolved | V-9 measures pool boot cost |
| L-1 | Resolved (per the delta; not re-verifiable) | #93 and #308–#312 verified 2026-10-06 with `--comments` |
| L-2 | Resolved | — |
| L-3 | Resolved | `ClaimSet<GuestCid>` |
| L-4 | Resolved | `Refused(Malformed)`, `DatagramRequestMismatch` |
| L-5 | Resolved | D15-R3 = (b); D15 table rows |
| L-6 | Resolved | `drained()` covers the parking hop |

## Passed

- Approvals: every decision in the index carries a user-approval date (2026-10-05 to 2026-10-08); APPLIANCE and D8a-LOAD-SWAP explicitly approved; rejected alternatives recorded. L-2 is wording only.
- Kernel mechanism first: steering moved from an nft element set to Aya `sk_lookup` over a listener-keyed socket map; ADR-0171 names the nft `socket`-expression and element-set alternatives and why each lost.
- Over-engineering removed: runtime fence, `verify`, probe, `repair_guest_prefix`, steering audit, K-L7, V-23 / A-27, foreign CID holders. A-31 recorded once and discharged by the image (ADR-0068). No stale mechanism survives in an ADR Decision.
- Fallbacks removed: CID exclusion set and preference order gone (D16-CLAIM is an atomic kernel claim); stuck-listener revoke loop gone (K-L3).
- Formal model: section present with 8 modules; spec models the design on sampled points (`prefix_boot` load → swap (→ pin) → route with crash at every step; `intake` steer-after-listen and close removes entry; `quiescence` named holders with `anyRestore` hazard; `cid_lease` stepwise atomic claim with `checkThenClaim`, `releaseBeforeVmm`, `rememberInUse` hazards). Every listed rule has a violating hazard; recorded evidence 120/120 matched; 16 input hashes match 15 `.qnt` files plus `checks.toml`; hashes not recomputable by the reviewer, r7 reports `verify-evidence` passed.
- ADR hygiene: no Rust signatures, test matrices or roadmap steps in ADRs 0145–0171; contracts pinned in the delta; present tense; no amendment narration.
- Deferrals: every deferral cites an issue (#93, #308–#312).
- Architecture enforcement: typestates and `trybuild` fixtures (`ClaimedGuestCid`, `QuiescenceHold`, `GuestSteeringCandidate` → `steer`); every acceptance runs through `serve` + `deploy` (V-6).
