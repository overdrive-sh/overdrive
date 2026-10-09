# DISTILL scenario index — netns-density-295 (vsock attachment replacement)

This index replaces the prior (deleted) `test-scenarios.md`, which specified
the superseded TAP / shared-bridge / intercept-program attachment model
(S-ND295-*). The replacement DESIGN (feature-delta L16552+, ADR-0145..0181)
forwards guest traffic over virtio-vsock (kernel vhost) + Aya SK_SKB / strparser
forwarding + `sk_lookup` guest-prefix steering; there is **no** guest NIC, TAP,
bridge, per-VM netdevice or per-VM netns (D1).

Binding rules: no `.feature` files; GWT scenarios are spec-only here and in the
per-piece docs; the crafter translates each to a Rust `#[test]`/`#[tokio::test]`
(`.claude/rules/testing.md`). Shared test-support contract:
[`shared-test-infrastructure.md`](shared-test-infrastructure.md). The Quint
oracle is `specs/quint/guest-flow-owner/` (do **not** edit).

## Pieces (disjoint file ownership)

| Piece | Title | Scenario doc |
|---|---|---|
| **P1** | Admission, CID lease & kernel claim, intake capacity, deploy validation | [test-scenarios-p1-admission-cid-lease.md](test-scenarios-p1-admission-cid-lease.md) |
| **P2** | Guest-flow owner: control session, flow admission, pairing, launch attribution | [test-scenarios-p2-owner-flows.md](test-scenarios-p2-owner-flows.md) |
| **P3** | Datagram associations, UDP slots, framing, cells | [test-scenarios-p3-udp-slots-framing.md](test-scenarios-p3-udp-slots-framing.md) |
| **P4** | Guest-prefix steering, intake listeners, boot convergence & landing | [test-scenarios-p4-steering-intake-boot.md](test-scenarios-p4-steering-intake-boot.md) |
| **P5** | Egress unframe (hybrid), host-internal deny, mesh/non-mesh egress, VIP | [test-scenarios-p5-egress-unframe-hostinternal.md](test-scenarios-p5-egress-unframe-hostinternal.md) |
| **P6** | Quiescence & recovery, shared-table, VMM device & fork, guest READY, walking skeleton + crash matrix | [test-scenarios-p6-recovery-vmm-ws.md](test-scenarios-p6-recovery-vmm-ws.md) |

## Gate → piece

| Gate | Owner / promise | Piece |
|---|---|---|
| G-V0 node attachment admission | `GuestAddressPool::assign` (lease + CID claim + port reservation) | P1 |
| G-V1 shared forwarding substrate startup | owner boot: probe/converge, steering load+swap, unframe attach, route | P4 (steering/boot/route) + P5 (unframe attach) |
| G-V2 guest command release | VM driver under EXEC claim (after Active) | P6 |
| G-V3 allocation forwarding activation | owner `activate` (all-or-nothing listeners) | P2 (activate/flow) + P4 (listeners) |
| G-V4 VMM vsock device | `CloudHypervisorVmm` + fork (`backend=vhost-kernel`, `fd=`) | P6 |
| G-V5 runtime recovery reopen | supervisor, per-component quiescence holders | P6 |
| G-V6 per-flow pairing | flow acceptor (`Paired` after install) | P2 (TCP flows) + P3 (datagram) + P5 (egress legs) |
| G-V7 guest network config before READY | `overdrive-init` | P6 |
| G-V8 intake listener + steering entry | owner (`GuestPrefixSteering::steer`) | P4 |

## Quint module → piece (conformance driver)

| Module | Invariants it defends | Piece |
|---|---|---|
| `cid_lease` | inv 5 (CID claim) | P1 |
| `owner_flows` | inv 1,3,4,4+,9,13 (session/admission/pairing/attribution/half-close) | P2 |
| `udp_slots` | inv 2,14 (slot held once / released-carries-no-data / cell generation) | P3 |
| `prefix_boot` | inv 6 (route only behind pinned steering; swap) | P4 |
| `intake` | inv 7,8,12 (landing, all-or-nothing, capacity) | P4 |
| `prefix_landing` | landing-class table (stateless) | P4 |
| `quiescence` | inv 10 (holder set; repair inside hold) | P6 |
| `shared_table` | inv 11 (prefix closed whatever table state) | P6 |

## Decision → piece (every approved decision lands in exactly one piece)

| Piece | Decisions owned |
|---|---|
| P1 | D3, D11, D16, D16-CLAIM, D8a-CAP, INTAKE-PORTS, D15-R2 (reserved-range deploy refusal) |
| P2 | D7, D9, D14, D17, D18, D18a, D15-R1, D15-R3, ATTR-TOKEN, ATTR-LIVE-CLAIM (U-1, U-5) |
| P3 | D4, D5, D10, D25, D25-BIND, D15-R2 (guest bind/pool/idle), CELL-RECREATE (U-2) |
| P4 | D8, D8-INTAKE, D8-PEER, D8-PREFIX-CFG, D8-PREFIX-VIP, D8a, D8a-LOOKUP, D8a-LOAD-SWAP, D23, INTAKE-BIND, PREFIX-REPLY, D5a-SERVE-IFACE (veth carries unframe), U-3, U-4, U-6, M-7 |
| P5 | D5a, D5a-IFACE, D5a-IFACE-BOOT, D5a-LATE, IMG-IFACE-BOOT, USO-NO-SEGMENT, IFACE-NO-STACK, FRAG-MAP-LIFECYCLE, D24, D24a, D26, D26-PINS |
| P6 | D2, D13, D21, D8a-HOLD, D19, D19-LINKS, HALFCLOSE-SIGNAL (crash matrix), G-V7 guest config, QUINT process |

## R5-* boundary obligation → piece

| Piece | R5-* owned |
|---|---|
| P1 | R5-19, R5-21, R5-37, R5-44 (deploy + intake-block boot refusal) |
| P2 | R5-9, R5-16, R5-17, R5-18, R5-20, R5-26, R5-28, R5-31, R5-41, R5-45 (owner-seam + codec half-close) |
| P3 | R5-14, R5-15, R5-27 |
| P4 | R5-1, R5-2, R5-3, R5-22, R5-23, R5-24, R5-25, R5-29, R5-34, R5-39, R5-40, R5-42, R5-43 |
| P5 | R5-6, R5-7, R5-8, R5-38, R5-46, R5-47, R5-48 |
| P6 | R5-4, R5-5, R5-10, R5-13, R5-30, R5-32, R5-45 (Tier-3 inside R5-10) |

(R5-5 state-interleaving is split: `ListenState` interleaving in P4 (`intake`),
quiesce/restore interleaving in P6; each doc restates its half. R5-13 intake-child
identity lives in P4's `intake` steering scope with the V-20 INTAKE-TAG-MAP map.
R5-45 half-close: codec + owner-seam in P2; the Tier-3 crash-matrix cell in P6.)

## Validation-item → piece (DELIVER-step items; scenarios RED `#[ignore]`)

| V-item | Status | Piece | Note |
|---|---|---|---|
| V-1(c) fork `fd=` under production launch | open (DELIVER step 3) | P6 | handoff proven root (P-39) |
| V-6 walking skeleton serve+deploy | open (DELIVER 10-01) | P6 | includes V-15 |
| V-8 fail-closed in production composition | open (DELIVER 10-02) | P6 | |
| V-9 density & costs | open (DELIVER ≈08-04/10-05) | P6 | surfaced-not-relaxed |
| V-12 host-local connect source address | open (DELIVER step 9) | P4 | one D15 row |
| V-19 guest-prefix exposure | open (DELIVER step 9/12) | P4 | |
| V-28 6.18 re-run | open, not a gate (KVER) | P6 | runs with DELIVER Tier-3 |
| contract-shaping (V-10/22/24/25/26/5c/13/16/17/18/20/21) | **all DONE** (P-37..P-48) | — | nothing blocked on them |

## 6-row boundary-scenario table (feature-delta §"Required boundary scenarios")

Rows 1 (available), 2 (typed failure in owner domain), 3 (unrelated unchanged),
4 (late success can't resurrect), 5 (disconnect/reconnect), 6 (feature disabled
= sim `transport: None`) are enumerated per gate inside the owning piece doc.
Rows 2/4/5 for G-V2/G-V3/G-V5/G-V6/G-V8 need a seeded `overdrive-sim` invariant
through the production owner path; kernel effects (G-V1/G-V4/G-V6/G-V7/G-V8) need
native Tier-3 through `serve` + `deploy`.

## Retired / not-deleted

- **Deleted by the planner**: `distill/test-scenarios.md` (old content, now this
  index), `distill/red-classification.md`.
- **Retired in DELIVER roadmap reconciliation** (coupled to old production-code
  removal, NOT deleted now): the OLD-model GREEN test modules cataloged in
  [`shared-test-infrastructure.md`](shared-test-infrastructure.md) §7.
