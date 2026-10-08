# Independent DESIGN review — vsock Attachment Replacement (revision 11, answers to r11)

- Reviewer: independent `nw-solution-architect-reviewer` (Opus), 2026-10-08
- Under review: `feature-delta.md` § "vsock Attachment Replacement DESIGN" (L16552–21326), ADR-0145 … ADR-0176, `specs/quint/guest-flow-owner/`, `design/quint-guest-flow-owner-findings-r8.md`
- Prior review: `design/review-design-vsock-replacement-r11.md` (CHANGES_REQUESTED)
- Recorded by the orchestrator from the reviewer's returned output.

## Verdict: CHANGES_REQUESTED

BLOCKER 0 · HIGH 1 · MEDIUM 2 · LOW 2. Every r11 finding is resolved; the new findings come from D5a-LATE's premise ("only the image configures root-namespace interfaces"), not from the decision itself.

## HIGH

**H-1 — The D5a-LATE part of A-31 names a discharging artifact that does not contain the requirement.** Delta L16860–16866 (A-31), L20363 ("Discharged by: The appliance image configuration (ADR-0068)"), L16755 (D5a-LATE), ADR-0165 "Attachment set". ADR-0068 says nothing about interfaces, network configuration, networkd or hotplug, and the appliance image does not exist yet, so nothing the image's builder reads carries "the image must not configure an interface registered after boot". Failure: the image ships a default networkd `[Match] Name=*` (as the dev/metal hosts do); a late NIC gets DHCP addresses and routes after boot, has no unframe link, and framed guest datagrams leave through it; no test fails because Tier-3 runs on the metal host. Rule: `design.md` § "Concurrent protocols" (assumption with no discharging item); CLAUDE.md deferrals need issues. Fix: record the requirement where the image is specified (amend ADR-0068, or attach it to the image issue after verifying scope with `--comments`), surface to the user first, then cite it in A-31, L20363 and ADR-0165.

## MEDIUM

**M-1 — `serve` itself creates and configures root-namespace interfaces; the unframe attachment set covers them only through an unstated ordering.** Delta L18284 ("This design creates no root-namespace interface"), L16861 and ADR-0165 L67–68, Composition steps 3 and 9 (L18980, L18994), G-V1 Ordering L19339–19343. ADR-0061's single-node pair `ovd-veth-cli` / `ovd-veth-bk` is created and addressed by `run_server` every boot (`lib.rs:7369–7389`) with route `<vip_range> dev ovd-veth-cli` (`veth_provisioner.rs:377–398`); guest datagrams to `vip_range` can leave through them. They get the unframe program only because the dataplane block runs before `converge_shared`, which neither the Composition nor G-V1 pins. #197 (promoting the pair to a runtime reconciler) would recreate a configured interface after boot with no unframe link. Rule: `design.md` § Lifecycle Gate Ownership (Ordering), § Preserve existing states. Fix: correct the premise; pin "every interface `serve` creates in the root namespace exists before `converge_shared`" in the Composition and G-V1 with a scenario asserting `ovd-veth-*` carry the unframe link; record the constraint #197 must honour.

**M-2 — The D8-PREFIX-CFG check runs before `serve`'s own address write and ignores the `vip_range` producer.** Composition step 1 vs step 3 (L18974–18980), `probe_startup` L18524–18529, L19131–19135, ADR-0152. Step 3 adds gateways derived from `vip_range` and the `vip_range` route after the step-1 check; nothing validates `vip_range` is disjoint from the guest prefix. Failure: `vip_range = 100.95.255.0/24` passes step 1; step 3 adds `100.95.255.1`; the pool can lease `100.95.255.x`; D24a treats guest TCP to that workload address as an unknown VIP (`MeshUnreachable`); steering drops host-local connects to the VIP gateway — not the promised refusal. Rule: `design.md` § Design the correct mechanism; `development.md` § Ground the premise. Fix: refuse an overlapping `vip_range` with a typed config-validation error before any boot effect (or re-check after every address-writing boot step), naming `vip_range` as a producer.

## LOW

- **L-1** The formal model section cites superseded run `704554-1791465879.649` (L19971, L20280, L21157; findings-r8 L10) while evidence is run `742976-1791482967.383`; "Running it" (L20175–20180) still says retry time is not separated and cites 138.1 s; L20172–20174 and L20321–20323 list `udp-slots-hazard-ignore-late-paired` as `ci = false` though `checks.toml` L348 has `ci = true`; L21166 says the model is unchanged though `checks.toml` changed.
- **L-2** `udp-slots-release-liveness` (67.4 s own time, `checks.toml` L245) and `udp-slots-hazard-late-paired-by-slot-data` (108.4 s, L337) are `ci = false` though under two minutes; flip them or record why their own time is unrepresentative under 8 parallel jobs.

## Prior findings (r11)

| ID | Status |
|---|---|
| H-1 | Resolved — D5a-IFACE-BOOT, D8a-CAP-RETRY, A-MEM-OWNER surfaced and approved 2026-10-08 (L16742–16754); ADR-0165/0176 status lines updated |
| H-2 | Resolved for round 8; smaller staleness for the re-recorded run → new L-1 |
| M-1 | Resolved — strong fairness in delta, A-MEM, README, `intake.qnt` |
| M-2 | Resolved by D5a-LATE; residual premise gaps → new H-1, M-1 |
| L-1 | Resolved in the runner; delta text stale → new L-1, L-2 |
| L-2 | Resolved — `MapLookup` from `steer`, `Query` from `load` and `steer` |
| L-3 | Resolved — memory-cgroup charge stated |
| L-4 | Resolved — Changed assumptions lists only operative contracts |

## Passed

Approvals (all revision-11 decisions "Proposed — approved by user 2026-10-08"); appliance threat model (no defence against absent actors; `UnframeAttachError` boot-only; only `KernelMemory` retried); correct mechanism (capacity admitted at `assign`; late-interface fallback deleted); one ADR per decision (ADR-0165, ADR-0176, no signatures or narration); interface contracts pinned with producers; Lifecycle Gate Ownership (G-V0, G-V1; R5-37, R5-38); formal model (132/132, violating hazards for D8a-CAP, D5a-LATE correctly not modelled); vertical slice; deferrals (#93, #261, #308–#312).
