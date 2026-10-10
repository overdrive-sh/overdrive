# P1 — Admission, CID lease & kernel claim, intake capacity, deploy validation

> **DISTILL status (2026-10-10, second increment after DG-1..DG-16).** Every
> round-1 `UI-*` item is closed by a DG decision, and no round-2 item bears on
> P1 (round 2: `upstream-issues.md` UI-R2-1..22). DG-2 sorts each scenario into one of two authoring times.
> **Authored now** (compiled, ignored until their DELIVER step):
> `overdrive-core` `vsock_guest_transport_vocabulary` (P1-S1, P1-S6 core half,
> P1-S8 and P1-S10/S11 pure parts), `overdrive-core` `vsock_deploy_validation`
> (P1-S10, P1-S11), `overdrive-host` `vsock_cid_claim_handoff` (P1-S12
> claim-level cells, `kvm-tests`, native metal). **Attachment cut** (DG-2 rows
> 1, 2, 3, 6): the stateful pool scenarios P1-S2..S5, S7, S9, the stateful halves
> of P1-S1 / S6 / S8, the P1-S12 VMM-handoff cell, and the P1-S13 `cid_lease`
> driver, all source-local in `overdrive-control-plane` (owner of
> `GuestAddressPool`). The `SimGuestCidClaim` sim-only cells stay dropped: the
> sim is a port double, not a system under test. Classes:
> `red-classification.md`. Per-scenario status lines below supersede the
> earlier placement text.

DISTILL scenario specification (GWT spec only — no `.feature` files; the crafter
translates each to a Rust `#[test]`/`#[tokio::test]`, `.claude/rules/testing.md`
§"Testing"). Companion: [shared-test-infrastructure.md](shared-test-infrastructure.md).

Scope: node attachment admission (gate **G-V0**), the lease→CID derivation and
the atomic host-kernel CID claim (**D16**, **D16-CLAIM**), declared-TCP-port
reservation against the steering capacity (**D8a-CAP**, **INTAKE-PORTS**), and
the deploy/API refusal of reserved-range and over-limit specs (**D25** range,
**INTAKE-PORTS**). Oracle for the lease/claim protocol: Quint **`cid_lease`**
(invariant 5, feature-delta L21045-21051).

Owns decisions **D3, D11, D16, D16-CLAIM, D8a-CAP, INTAKE-PORTS, D15-R2** (reserved
range). Gate **G-V0** + its state-ownership row (feature-delta L20064, L20078).
Quint `cid_lease` invariant 5 + `cid-lease-*` checks (incl. hazards
check-then-claim / release-before-VMM / remember-InUse). Boundary obligations
**R5-19, R5-21, R5-37, R5-44** (deploy + reservation). **V-25** done (P-39); its
production `fd=` wiring is the P1-S12 VMM-handoff cell and P6-S13 (attachment
cut, DG-2 row 6).

Design pins: feature-delta.md §"[REF] Core vocabulary — overdrive-core"
(L17449-17888), §"State-ownership matrix" (L20060), §"Gate G-V0" (L20078),
§"Required boundary scenarios" (L20411), §"Validation plan" (V-25, L20607),
§"Formal protocol model" (`cid_lease`, L20958 + invariant 5 L21045 + hazards
L21120). ADRs 0155, 0156, 0170, 0176, 0178, 0179.

---

## 1. Files owned (disjoint) + placement decisions

| File | Crate / lane | Hosts | Status |
|---|---|---|---|
| `crates/overdrive-core/tests/acceptance/vsock_guest_transport_vocabulary.rs` | overdrive-core, unit+proptest (L1-2), default lane | `GuestCid`, `LaunchToken`, `ListenPort`, `InterfaceIndex`, the take-once `GuestTransportAssignment` / `ClaimedGuestCid` over the `GuestCidClaim` port (`SimGuestCidClaim` at the driven port, observed through its QPROJ-7 arbiter accessor `held_cids()`), the D25 reserved range and the INTAKE-BIND host-port block | authored; registered in `tests/acceptance.rs` |
| `crates/overdrive-core/tests/acceptance/vsock_deploy_validation.rs` | overdrive-core, unit+proptest (L1-2), default lane | deploy/API listener validation (both ingresses are overdrive-core) | authored; registered in `tests/acceptance.rs` |
| `crates/overdrive-host/tests/integration/vsock_cid_claim_handoff.rs` | overdrive-host, Tier-3 native metal (`integration-tests,kvm-tests`), real `/dev/vhost-vsock` | the real `VhostVsockCidClaim` claim, its claim-level handoff and probe (DG-6) | authored; registered in `tests/integration.rs` under `#[cfg(feature = "kvm-tests")]`; the binary is already in `host-kernel-shared` |
| `crates/overdrive-sim/src/adapters/guest_cid_claim.rs` | overdrive-sim (`adapter-sim`) | `SimGuestCidClaim` (`script_claim_error`, `script_probe_deviation`, `survivor_hold`, `held_cids`) | in tree |
| pool acceptance bodies P1-S2..S5, S7, S9, P1-S8 stateful half | `overdrive-control-plane`, source-local | `GuestAddressPool::new(/29, gateway, SimGuestCidClaim, SimEntropy)`, `assign(a, &ports)`, `retire`, `release`, surviving `SimVmm` holders | attachment cut (DG-2 rows 1, 2); starting draft `overdrive-core/tests/acceptance/vsock_guest_address_pool.rs` @91f9436ff (removed from the tree) |
| `cid_lease` driver | `overdrive-control-plane`, in-crate quint-connect module | instances `lease_conf_ok`, `lease_conf_live` | attachment cut (DG-2 rows 1, 2, 3, 6) |
| P1-S12 VMM-handoff cell | native metal | `Vmm::create(&cfg, claim)` on the real fork | attachment cut (DG-2 row 6) |

**Placement (DG-2, QPROJ-1).** `GuestAddressPool` lives in
`overdrive_control_plane::guest_network`, and `overdrive-core` does not depend
on `overdrive-control-plane`. Its pinned `new` / `assign` signatures replace
live ones (DG-2 rows 1, 2), so every body that calls them is authored at the
attachment cut, source-local in `overdrive-control-plane` beside the owner,
and observes the pool through its QPROJ-2 accessor `lease_observation()`
(cursor, leases, `claim_in_slot`, `reserved_ports`) plus the
`SimGuestCidClaim::held_cids()` arbiter read. No `e18_test_support` wrapper
is used: `e18_test_support` is deleted at the cut (DG-2 row 17). No
production API or accessor is invented (CLAUDE.md §"Implement to the design").

---

## 2. RED mode & paradigm (Mandate 9 / `.claude/rules/testing.md`)

- **Layers 1-2** (vocabulary, pure functions, claim-port contract, deploy
  validation): **PBT full** (`proptest`) — newtype roundtrips, derivation,
  intake-port injectivity, reserved-range and port-count boundaries, the
  capacity predicate.
- **Layer 3+** (Tier-3 host claim/handoff; the `cid_lease` conformance is a
  trace replay, not PBT): **example-based only**; sad paths enumerated
  (Mandate 11). No PBT machinery at Tier-3.
- **State-delta / Universe (Mandate 8), Rust form.** This codebase has no
  Python `assert_state_delta` port and `.claude/rules/testing.md` does not
  introduce one; the Rust-idiomatic equivalent is an explicit before/after
  snapshot of the port-exposed observable set with a full-set assertion
  (fail-closed on any unexpected change). Each state-mutating scenario below
  names its **Universe** (port-exposed observables only — the claim-set holder
  table, the pool's observable lease offsets + cursor, the returned
  plan/assignment, the typed error, the `ParseError`/`AggregateError`), never a
  private pool field. No `tests/common/state_delta.rs` is bootstrapped (a
  Python-ism the project rules do not adopt).

---

## 3. Scenario specifications (GWT + contract + owner-path + fault + oracle + tier)

Format per `.claude/rules/testing.md` §"DISTILL — prove composed system
behavior": **Contract/invariant · Production owner path · Fault stimulus ·
Outcome oracle (Universe) · Evidence tier · Rust home · DELIVER activation**.

### Pool / CID derivation & claim

**P1-S1 — next-fit `assign` derives a collision-free CID (D16).** `@contract-shape:pure-function`
- GWT: *Given* a pool with some offsets leased, *When* `assign(alloc, ports)`
  runs, *Then* it returns the next-fit free offset and `GuestCid =
  GUEST_CID_BASE + offset`, computed once, never persisted.
- Contract: CID derivation is a pure function of the lease offset; uniqueness
  over Admitted+Retiring from ADR-0132 (D16).
- Owner path: `GuestAddressPool::assign` (control-plane); the derivation
  identity `GuestCid::new(GUEST_CID_BASE+offset).get()` is overdrive-core.
- Fault: none (happy path). · Oracle (Universe): returned plan's `cid()`,
  observable lease offset. · Tier: unit+proptest (derivation, core) +
  conformance (next-fit through the pool).
- Status, core half — authored: `overdrive-core`
  `vsock_guest_transport_vocabulary`::{`every_leased_offset_is_a_cid_that_round_trips`,
  `every_value_outside_the_span_is_out_of_range`,
  `the_kernel_reserved_cids_and_the_probe_cid_are_never_leasable`,
  `launch_token_round_trips`, `launch_token_rejects_all_zero_upper_case_and_wrong_length`,
  `listen_port_round_trips`, `interface_index_round_trips`,
  `listen_ports_order_by_value`, `non_decimal_text_is_unparseable`,
  `zero_is_refused_zero_by_the_constructors_from_str_and_serde`,
  `out_of_range_and_malformed_text_is_unparseable`} — ignored `pending DELIVER
  vsock new-step 2 (core types and wire codec)`;
  `a_launch_token_is_drawn_from_the_injected_entropy` — ignored `pending DELIVER vsock attachment cut (new-step 15, launch token drawn with the claim)`;
  `listen_port_and_interface_index_errors_render_their_pinned_messages` — not
  ignored (default-lane GUARD).
- Status, stateful next-fit half — attachment cut (DG-2 rows 1, 2): the pool
  body drives `GuestAddressPool::new(/29, gateway, SimGuestCidClaim,
  SimEntropy)` and `assign(a, &ports)` and asserts the returned plan's CID and
  the O2 `lease_observation()` cursor and leases before/after; the `cid_lease`
  driver (`AssignTry` / `CreateOk`) covers it against the oracle.

**P1-S2 — lease replay / retire / release change only the named allocation.** `@contract-shape:bounded-change`
- GWT: *Given* any sequence of `assign`/retire/`release` over a fixed alloc
  set, *When* applied, *Then* each step mutates only that allocation's lease,
  CID claim and reservation; all others unchanged.
- Contract: ADR-0132/0133 lease lifecycle; D16 (one CID per lease).
- Owner path: `GuestAddressPool::{assign, release}` (control-plane).
- Fault: arbitrary interleavings (property). · Oracle (Universe): the full
  offset→alloc lease map + `held_cids()` + reservation sum — only the named
  offset changes per op (fail-closed on adjacent mutation). · Tier: seeded sim
  through the production pool + conformance (`Release`/`Redeploy`).
- Status — attachment cut (DG-2 rows 1, 2): the pool body drives `assign`,
  `retire` and `release` over a fixed allocation set and asserts O2
  `lease_observation()` (leases, `claim_in_slot`, `reserved_ports`) and
  `held_cids` before/after each step, with only the named allocation
  changing; the `cid_lease` driver (`Release` / `Redeploy`) replays the same.

**P1-S3 — CID claim is atomic with the lease (R5-19a).** `@contract-shape:bounded-change`
- GWT: *Given* an offset whose CID another device instance holds (a
  `SimGuestCidClaim` holder OUTSIDE the pool's leases), *When* `assign` runs,
  *Then* that offset is skipped and the next claimable offset is returned; no
  launch ever fails on a CID clash.
- Contract: D16-CLAIM — `assign` takes a lease only together with an atomic
  `cid_claim.claim(cid)`; `EADDRINUSE` skips the offset for this call only.
- Owner path: `GuestAddressPool::assign` → `GuestCidClaim::claim` (sim at the
  driven port).
- Fault: a scripted holder on the next-fit offset. · Oracle (Universe):
  returned plan's offset = the first UNheld offset; `held_cids()` unchanged
  except the newly-claimed one; InUse skip counted. · Tier: seeded sim +
  conformance + Quint `cid_lease` (hazard `check-then-claim`).
- Status — attachment cut (DG-2 rows 1, 2): the pool body places a
  `SimGuestCidClaim::survivor_hold` on the next-fit offset, drives `assign`,
  and asserts the returned offset is the first unheld one with O2
  `lease_observation()` and `held_cids` changed only by the new claim; the
  `cid_lease` driver covers the skip. The claim-port exclusivity cell that
  ran against `SimGuestCidClaim` alone was dropped (the sim is not the system
  under test); kernel-level exclusivity is P1-S12
  (`a_second_claim_while_the_first_is_open_is_in_use_and_holds_nothing`).

**P1-S4 — all free offsets held elsewhere → `GuestCidsHeldElsewhere` (R5-19b).** `@contract-shape:unbounded-preservation`
- GWT: *Given* every free offset's CID is held elsewhere at the moment its claim
  is tried, *When* `assign` runs, *Then* it returns `GuestCidsHeldElsewhere {
  free }` with **no state change**; after one holder releases, the next `assign`
  succeeds on that offset; the earlier `InUse` is **not** remembered.
- Contract: D16-CLAIM; invariant 5 — refuse only when every free offset is held;
  non-terminal, same projection as `PoolExhausted`.
- Owner path: `GuestAddressPool::assign`.
- Fault: survivor holders on all free offsets, then one release. · Oracle: typed
  `GuestCidsHeldElsewhere`; lease map + cursor unchanged on refusal; success on
  the released offset next (Universe = lease map + `held_cids` + typed error).
  · Tier: seeded sim + conformance + Quint (hazard `remember-inuse`).
- Status — attachment cut (DG-2 rows 1, 2): the pool body holds every free
  offset with surviving `SimVmm` / `survivor_hold` holders, drives `assign`
  and asserts `GuestCidsHeldElsewhere` with no change to O2
  `lease_observation()`, then releases one holder and asserts the next
  `assign` takes that offset; the `cid_lease` driver (`SurvivorCreate` on all
  offsets → `OrphanExit`) covers it against the oracle.

**P1-S5 — non-`InUse` claim failure → `GuestCidClaim`, skips no offset (R5-19c).** `@contract-shape:unbounded-preservation`
- GWT: *Given* a claim fails for a cause other than `EADDRINUSE`, *When* `assign`
  tries that offset, *Then* it returns `GuestNetworkError::GuestCidClaim { cid,
  source }` with no state change and **no further offset tried**.
- Contract: D16-CLAIM / CLAIM-PINS — only `EADDRINUSE` is `InUse`; every other
  errno is `Kernel`, which does not skip.
- Owner path: `GuestAddressPool::assign` → `GuestCidClaim::claim` (scripted
  non-InUse).
- Fault: `SimGuestCidClaim::script_claim_error`. · Oracle: typed `GuestCidClaim`;
  no offset skipped; no state change (Universe = lease map + typed error). ·
  Tier: unit (claim port) + seeded sim + conformance + Quint (`AssignClaimErr`).
- Status — attachment cut (DG-2 rows 1, 2): the pool body scripts
  `SimGuestCidClaim::script_claim_error` on the next-fit CID, drives `assign`
  and asserts `GuestNetworkError::GuestCidClaim { cid, source }`, no further
  offset tried and no change to O2 `lease_observation()` or `held_cids`; the
  `cid_lease` driver covers `AssignClaimErr`. The claim-port-only cell over
  `SimGuestCidClaim` was dropped (the sim is not the system under test).

**P1-S6 — claim handed to the VMM exactly once (R5-19d).** `@contract-shape:bounded-change`
- GWT: *Given* an Admitted lease whose plan carries a take-once claim slot,
  *When* `create` returns `Ok`, *Then* `serve` holds no copy and a second
  `take_claim` returns `None`; a failed `create` and a pre-launch `release` each
  free the CID.
- Contract: D16-CLAIM — move-only claim; `GuestTransportAssignment::take_claim`
  atomic across clones.
- Owner path: `GuestTransportAssignment::take_claim` (core) →
  `Vmm::create(&cfg, claim)` (host, DG-2 row 6) → `serve` closes its copy.
- Fault: concurrent clones racing `take_claim`; a `create` error. · Oracle:
  exactly one `Some` across all clones; the claim released on failed create /
  pre-launch release (Universe = take-claim outcomes + `held_cids`). · Tier:
  unit (core take-once) + Tier-3 claim-level (host) + P6 walking skeleton (fork
  boot).
- Status, core half — authored: `overdrive-core`
  `vsock_guest_transport_vocabulary`::{`concurrent_clones_take_the_claim_exactly_once`,
  `the_assignment_reports_its_claim_and_equality_ignores_the_slot`,
  `an_untaken_claim_is_released_when_the_last_clone_drops`,
  `a_claim_handed_to_the_vmm_holds_the_cid_until_the_device_drops`} — ignored
  `pending DELIVER vsock attachment cut (new-step 15, move-only claim handle in the transport handoff)`. Host claim-level half — authored: `overdrive-host`
  `vsock_cid_claim_handoff`::`claimed_device_held_refuses_reclaim_until_released`
  (P1-S12).
- Status, failed-`create` / pre-launch-release half — attachment cut (DG-2
  row 6, with rows 1, 2): the body drives `Vmm::create(&cfg, claim)` on
  `SimVmm` with a scripted failure, and a `release` before launch, and asserts
  the CID is released (`held_cids`, O2 `lease_observation()` `claim_in_slot`).
  Fork boot is P6-S13 (attachment cut, rows 1-6).

**P1-S7 — surviving VMM across a `serve` crash (R5-19e/f).** `@contract-shape:bounded-change`
- GWT: *Given* a VMM that survived a `serve` crash still holds a CID, *When* the
  new process's `assign` runs, *Then* that CID's offset is skipped and is
  assigned again once that VMM exits; a pre-READY exit of any cause leaves later
  assignments as next-fit alone would choose, for every workload.
- Contract: D16-CLAIM — the kernel claim is the only cross-restart memory; no
  per-workload exclusion set.
- Owner path: boot `run_server` composes a fresh pool; the surviving holder is
  outside its leases.
- Fault: `Boot` + `SurvivorCreate`/`OrphanExit`; pre-READY exits. · Oracle: new
  `assign` skips the held offset, then reuses it post-exit; next-fit otherwise
  (Universe = lease map + `held_cids` across the Boot edge). · Tier: seeded sim +
  conformance (`lease_live`) + Quint + P6 crash matrix.
- Status — attachment cut (DG-2 rows 1, 2, 3, 6): the pool body composes a
  fresh `GuestAddressPool` beside a surviving `SimVmm` holder, drives `assign`
  before and after the holder exits, and asserts the skip then reuse through
  O2 `lease_observation()` and `held_cids`; the `cid_lease` driver instance
  `lease_conf_live` covers the `Boot` edge; the native crash matrix is P6-S14
  (attachment cut, rows 1-6).

**P1-S8 — intake capacity reservation (R5-37, D8a-CAP).** `@contract-shape:unbounded-preservation`
- GWT: *Given* held leases reserving declared TCP ports, *When* `assign`'s ports
  would push total reserved past `GUEST_INTAKE_STEERING_MAX`, *Then* it returns
  `IntakeCapacityReached { reserved, requested, capacity }` with no state change
  and no CID claimed; a Retiring lease's ports still count; after a `release`
  the same deploy is admitted. Property: for any admitted sequence, reserved
  ports ≤ capacity and open intake listeners ≤ reserved ports.
- Contract: D8a-CAP (ADR-0176) — ports reserved with the lease; the cap check
  precedes any CID claim (same lock acquisition).
- Owner path: `GuestAddressPool::assign(alloc, listen_ports)` (control-plane).
- Fault: a deploy at capacity+1; a Retiring lease. · Oracle: typed
  `IntakeCapacityReached`; `held_cids` unchanged (no claim); reservation sum ≤
  capacity invariant (Universe = reservation sum + lease map + `held_cids` +
  typed error). · Tier: **pure-arith** unit+proptest (core) + stateful
  unit+proptest (pool) + seeded sim + Quint `intake`.
- Status, pure half — authored: `overdrive-core`
  `vsock_guest_transport_vocabulary`::{`intake_host_ports_are_injective_inside_the_block`,
  `indices_beyond_the_block_map_to_nothing`,
  `the_intake_host_block_sits_below_the_host_ephemeral_range_and_apart_from_the_guest_reservation`}
  — ignored `pending DELIVER vsock new-step 9 (intake host ports,
  INTAKE-BIND)`. The earlier pure capacity-predicate test name
  (`capacity_predicate_refuses_exactly_on_overflow`) is not in the tree; the
  capacity check is part of `assign` and is covered by the stateful half.
- Status, stateful `IntakeCapacityReached` half — attachment cut (DG-2 rows 1,
  2): the pool body drives `assign(a, &ports)` at capacity + 1 (with a
  Retiring lease's ports still counted) and asserts `IntakeCapacityReached`
  with no change to O2 `lease_observation()` (`reserved_ports`, leases) and no
  new entry in `held_cids`, then admits the same deploy after a `release`. The
  Quint `intake` driver is P4-S18 (attachment cut).

**P1-S9 — G-V0 boundary rows.** `@contract-shape:unbounded-preservation`
- GWT: row 2 typed failures (`AdmissionCapReached`, `IntakeCapacityReached`,
  `GuestCidsHeldElsewhere`/`GuestCidClaim`); row 3 running allocations
  unaffected; row 4 a late `assign` after retire refused (`LeaseRetiring`).
- Contract: G-V0 (feature-delta L20078) — admission gates only the
  `StartAllocation` assignment; never READY/Running/flows.
- Owner path: `GuestAddressPool::assign`. · Fault: cap reached; retire-then-
  assign. · Oracle: the typed error per row; unrelated running allocations and
  other workloads' placement unchanged (Universe = lease map + the row's typed
  error; explicitly-unaffected set asserted unchanged). · Tier: seeded sim +
  conformance (refusals).
- Status — attachment cut (DG-2 rows 1, 2): the pool body drives each row's
  refusal (`AdmissionCapReached`, `IntakeCapacityReached`,
  `GuestCidsHeldElsewhere` / `GuestCidClaim`, retire-then-`assign`
  `LeaseRetiring`) and asserts the typed error with the other leases in O2
  `lease_observation()` unchanged; the `cid_lease` driver covers the
  refusals.

### Deploy / API validation

**P1-S10 — reserved-range listener refused (R5-21, D25).** `@contract-shape:pure-function`
- GWT: *Given* a VM spec whose `[[listener]]` declares 61,000 / 61,500 / 62,024
  (TCP or UDP), *When* `overdrive deploy` parses it, *Then* it is refused
  `ParseError::ListenerPortReserved { port, first, last }` naming the range;
  60,999 and 62,025 are accepted; the API ingress refuses the same with
  `AggregateError::Validation { field: "listeners[].port" }`.
- Contract: D25 / D15-R2 — reserved guest range `guest_reserved_ports()`
  (61,000..=62,024), above the ephemeral range.
- Owner path: **TOML** `WorkloadSpecInput::from_toml_str`; **API**
  `Service::from_submit(ServiceSpecInput)` — both overdrive-core, both read the
  constant, neither spells the number.
- Fault: a reserved port of either protocol. · Oracle: the typed `ParseError`
  (naming port+range) / `AggregateError::Validation`; nothing placed (Universe =
  the parse/validation result). · Tier: unit + proptest (range boundaries).
- Status — authored: `overdrive-core`
  `vsock_deploy_validation`::{`deploy_refuses_reserved_listener_port_naming_the_range`,
  `api_ingress_refuses_reserved_listener_port_with_validation`,
  `deploy_refusal_iff_port_in_reserved_range`} — ignored `pending DELIVER vsock new-step 9 (deploy-ingress validation: D25 reserved-port refusal, INTAKE-PORTS)`; the range constant itself:
  `vsock_guest_transport_vocabulary`::`the_reserved_range_holds_the_tcp_intake_and_every_udp_slot_above_the_ephemeral_range`
  — ignored `pending DELIVER vsock new-step 5 (guest intakes, D25)`. The
  60,999 / 62,025 acceptance cells are inside
  `deploy_refusal_iff_port_in_reserved_range` and the range test; the earlier
  separate name `deploy_accepts_ports_adjacent_to_the_reserved_range` is not
  in the tree.

**P1-S11 — too-many TCP listener ports (R5-44, INTAKE-PORTS).** `@contract-shape:pure-function`
- GWT: *Given* a spec with 4,096 distinct TCP listener ports it is accepted;
  4,097 is refused `ParseError::TooManyTcpListenerPorts { declared, max }`; the
  API ingress refuses with `AggregateError::Validation { field: "listeners" }`.
  Property: deploy refusal iff distinct TCP ports > the constant (UDP ports do
  not count).
- Contract: INTAKE-PORTS (ADR-0179) — `GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX =
  4,096` = the intake host-port block size.
- Owner path: `WorkloadSpecInput::from_toml_str` / `Service::from_submit`.
- Fault: 4,097 distinct TCP ports. · Oracle: typed refusal naming count+max /
  `Validation`; 4,096 accepted (Universe = parse/validation result). · Tier:
  unit + proptest.
- Status — authored: `overdrive-core`
  `vsock_deploy_validation`::{`deploy_accepts_exactly_the_block_size_and_refuses_one_more`,
  `api_ingress_refuses_too_many_tcp_listener_ports_with_validation`,
  `deploy_refusal_iff_distinct_tcp_ports_over_the_constant`} — ignored
  `pending DELIVER vsock new-step 9 (deploy-ingress validation: D25 reserved-port refusal, INTAKE-PORTS)`.

### Kernel claim handoff — Tier-3

**P1-S12 — claim + handoff (V-25 / R5-19d / CLAIM-PINS).** `@contract-shape:bounded-change`
- GWT: `VHOST_VSOCK_SET_GUEST_CID` on an unowned instance holds the CID
  exclusively while any reference is open; a second claim fails `EADDRINUSE` →
  `InUse`; the move-only device is handed to the fork by descriptor (fd=) with
  `serve` closing its copy after spawn; released at the last close (≤ 10 ms,
  reported not hard-asserted); the probe holds under the `serve` identity; a
  bad descriptor makes CH exit before READY with the CID still claimable (never
  a CID-clash cause).
- Contract: D16-CLAIM / CLAIM-PINS / A-29 (V-25 done, P-39).
- Owner path: `VhostVsockCidClaim` (host, DG-6: `VhostVsockCidClaim::new`,
  `VHOST_VSOCK_DEVICE`) + `ClaimedGuestCid::into_device()` →
  `Vmm::create(&cfg, claim)` (DG-2 row 6), which launches the vendored CH fork
  with `--vsock cid=…,backend=vhost-kernel,fd=K` (launch contract; DG-1: no
  probe identifies the installed CH build, the provisioning checksum is the
  identity check, and `/dev/vhost-vsock` access is the claim probe's —
  `GuestCidClaim::probe`, `GuestCidClaimError::{DeviceUnavailable,
  ProbeMismatch}`).
- Fault: a second concurrent claim; a dropped reference; a bad descriptor. ·
  Oracle: `InUse` on the second claim; re-claimable after the last close; probe
  `Ok`; guest-reported peer CID = claimed CID + byte round-trip (fork cell);
  pre-READY exit projected through the existing VMM-exit start failure
  (Universe = the kernel holder table observed via a second claim + the
  guest-reported CID). · Tier: **example-based Tier-3 metal** (`kvm-tests`), NO
  PBT.
- Status, claim-level cells — authored: `overdrive-host`
  `vsock_cid_claim_handoff`::{`claim_on_unowned_instance_holds_cid_exclusively`,
  `a_second_claim_while_the_first_is_open_is_in_use_and_holds_nothing`,
  `claim_released_at_last_reference_close`, `claim_probe_holds_on_the_real_device`,
  `claimed_device_held_refuses_reclaim_until_released`,
  `claims_of_distinct_cids_are_independent`} — ignored `pending DELIVER vsock attachment cut (new-step 15, VhostVsockCidClaim, D16-CLAIM)`;
  `the_claim_adapter_names_the_real_device` — not ignored (GUARD; runs
  whenever the `kvm-tests` binary runs). Lane: native x86_64 metal
  (`cargo xtask metal run -- cargo nextest run -p overdrive-host --features
  integration-tests,kvm-tests`), as root; CIDs at offsets 100..=105; the
  binary is in `host-kernel-shared`.
- Status, VMM-handoff cell — attachment cut (DG-2 row 6): the body drives
  `Vmm::create(&cfg, claim)` on the real fork and asserts the CID is held by
  the VMM until it exits (observed through a second claim, `InUse` ⇔ held).
  The fork-boot cells (guest-reported CID, byte round-trip, pre-READY exit on
  a bad descriptor with the CID still claimable) are P6-S13 (attachment cut,
  rows 1-6). P6-S8 (the former VMM backend probe) is withdrawn with the probe
  (DG-1); its `fd=` handoff obligations live in this cell and P6-S13.

### Conformance

> **Driver status (2026-10-10).** Placement, `State` projection and action
> map are pinned by QPROJ-1 to QPROJ-8 (`test-scenarios.md` § Quint
> conformance drivers). The `cid_lease` driver is an in-crate quint-connect
> module of `overdrive-control-plane`, authored at the attachment cut (DG-2
> rows 1, 2, 3, 6), because its action map calls `GuestAddressPool::new` /
> `assign` and `Vmm::create(&cfg, claim)`. It replays `#[quint_run]`
> randomized traces only: the specs define no Quint `run`, so no
> `#[quint_test]` pin exists (`UI-R2-1`). No `tests/conformance` file, shared
> `quint_driver` base, `e18_test_support` or `ConformanceNode` is used.

**P1-S13 — `cid_lease` conformance (invariant 5).** `@contract-shape:bounded-change`
- GWT: the quint-connect driver replays generated `cid_lease` traces (conformance
  instances `lease_conf_ok` / `lease_conf_live`, S-2) against the production
  `GuestAddressPool` + `SimGuestCidClaim`, comparing the observable `(kern, lease, cursor)` after
  every step; invariant 5 (`Safety` = `CidUnique`, `ClaimHeld`, `NoLaunchClash`,
  `NoSkipWhileClaimable`, `NoLeakedClaim`, `RefusalEachClaimInUse`) and
  placement liveness (`EventuallyPlaced`) hold; the three hazard variants
  (`check-then-claim`, `release-before-VMM`, `remember-InUse`) are the teeth
  (model-level checks in `checks.toml`; those entries are invariant and
  witness checks, not Quint runs).
- Contract: D16-CLAIM; invariant 5; progress ("placed whenever a free offset's
  CID stays unheld long enough for one claim").
- Owner path: the production `GuestAddressPool` constructed in-crate with
  `SimGuestCidClaim` and `SimEntropy` at the driven ports, `SimVmm` as the
  VMM; `State` projected from O2 `lease_observation()` (`lease`, `cursor`)
  and the arbiter reads `SimGuestCidClaim::held_cids()` (`kern`) and `is_live`
  (QPROJ-2, QPROJ-7).
- Fault: survivor holders, serve crash+restart, pre-READY exits, claim errors
  (all driven by the spec's actions). · Oracle: per-step state equality vs the
  spec. · Tier: conformance (Lima, spawns `quint`).
- Status — attachment cut (DG-2 rows 1, 2, 3, 6): `cid_lease` driver in
  `overdrive-control-plane`, `#[quint_run]` instances `lease_conf_ok` and
  `lease_conf_live`, QPROJ action map; asserts O2 `lease_observation()` plus S
  `held_cids` / `is_live` after every step.

---

## 4. G-V0 boundary table (feature-delta L20415-20420) → scenario

| Row | G-V0 obligation | Scenario |
|---|---|---|
| 1 Available | Lease + CID advances the `StartAllocation` assignment | P1-S1, S13 |
| 2 Unavailable/typed | `AdmissionCapReached`; `IntakeCapacityReached`; `GuestCidsHeldElsewhere`/`GuestCidClaim` | P1-S4, S5, S8, S9 |
| 3 Unrelated unchanged | running allocations unaffected | P1-S9 |
| 4 Late success | a late `assign` after retire refused (`LeaseRetiring`) | P1-S2, S9 |
| 5 Disconnect/reconnect | — (no G-V0 disconnect obligation; CID survival is S7) | S7 (crash edge) |
| 6 Feature disabled | single-cut (DG-2); substitute = `AllocationSpec.transport: None` | the planned attachment-cut cell P6-S12b (C5a / C5b) |

---

## 5. Coverage matrices

**Decisions:** D3 (pair capacity placeholder — admission does not fix it; S1/S9) ·
D11 (16,384 placeholder, no fixed limit below — S9 cap projection) · D16
(derivation — S1) · D16-CLAIM (atomic claim — S3,S4,S5,S6,S7,S12,S13) · D8a-CAP
(reservation — S8) · INTAKE-PORTS (S11) · D25/D15-R2 (reserved range — S10).

**Gate G-V0:** owner `GuestAddressPool::assign`; promise/affected-state/failure-
projection/unaffected per feature-delta L20080-20100 — S1-S9 + the boundary
table §4.

**Quint `cid_lease` (invariant 5):** `CidUnique`/`ClaimHeld`/`NoLaunchClash`
(S3,S6,S7,S13) · `NoSkipWhileClaimable` (S3,S13, hazard `remember-InUse`) ·
`NoLeakedClaim`/`RefusalEachClaimInUse` (S4,S5,S13) · `EventuallyPlaced` (S7,S13)
· hazards check-then-claim / release-before-VMM / remember-InUse (S13 teeth).

**R5-* (feature-delta L20449+):** R5-19 (a-f) → S3,S4,S5,S6,S7,S12,S13 · R5-21 →
S10 · R5-37 → S8 · R5-44 → S11.

**V-items:** V-25 done (P-39) → S12 (claim level, authored) + the S12
VMM-handoff cell and P6-S13 (attachment cut); the design records "nothing
blocked on a validation item" for P1.

---

## 6. Mandate & Pillar compliance (P1)

- **Mandate 1 (hexagonal):** scenarios enter through driving surfaces —
  `GuestAddressPool::assign` (admission driving port), `WorkloadSpecInput::
  from_toml_str` / `Service::from_submit` (deploy/API driving ports),
  `GuestCidClaim` (driven port, sim/host adapters), the conformance driver
  (production composition). No internal pool field is asserted.
- **Mandate 2 / Pillar 1 (domain language):** scenario titles name the
  user-valued outcome (admit a workload, refuse a reserved port, hand the CID to
  the VMM once), not HTTP/ioctl mechanics. `VHOST_VSOCK_SET_GUEST_CID` appears
  only inside Tier-3 step bodies, never a scenario title.
- **Mandate 3 (journeys):** S6/S7/S12 are complete lease→claim→handoff→exit
  journeys; S10/S11 are complete deploy-refusal journeys with the operator-facing
  typed error.
- **Mandate 8 (Universe):** each state-mutating scenario names a port-exposed
  Universe (claim-set holder table, observable lease offsets, returned
  plan/error) and asserts the full set (Rust before/after form; §2).
- **Mandate 9 (PBT mode):** PBT at L1-2 only; Tier-3 + conformance example/
  trace-based. Verified by file headers.
- **Mandate 10 (two-tier):** Tier A = the authored example tests and the
  attachment-cut pool bodies; the conformance driver (attachment cut) replays
  `#[quint_run]` traces over the production pool. Tier B state-machine PBT is **not added** —
  the `cid_lease` STATE MACHINE is already model-checked in Quint (the stronger
  oracle) and replayed by the conformance driver; a hand-rolled Rust
  `RuleBasedStateMachine` over the same protocol would duplicate the Quint model
  with a weaker instrument. Documented refusal.
- **Mandate 11 (sad paths example-based):** S4/S5/S9/S12 sad paths are named
  example/trace tests; no PBT at layer 3+.
- **Mandate 12 (SSOT via types):** the domain vocabulary is the `overdrive-core`
  newtypes (`GuestCid`, `LaunchToken`, `GuestTransportAssignment`, the
  `GuestCidClaim` port, the intake constants) — the Rust analogue of
  `domain_types.py`; step bodies invoke production services / the pool / the
  claim port, never inline admission logic. Step-reuse ratio is informational
  and low by design (config-/protocol-shaped feature); no forced parameterisation
  that would degrade Pillar 1.
- **Pillar 3 (production composition):** the conformance driver constructs the
  production `GuestAddressPool` in its owner crate (QPROJ-1); only the
  external/non-deterministic ports (`Entropy`, the CID claim's kernel) are faked
  (`SimEntropy`, `SimGuestCidClaim`). The Tier-3 host file uses the real kernel.

---

## 7. Placement gaps & shared-infra dependencies — closed

1. **Stateful-pool acceptance bodies.** Closed by DG-2 (rows 1, 2): authored
   at the attachment cut, source-local in `overdrive-control-plane`, starting
   from the removed draft `overdrive-core/tests/acceptance/vsock_guest_address_pool.rs`
   @91f9436ff. No `e18_test_support` wrapper (deleted at the cut, DG-2 row 17).
2. **Conformance crate deps / shared `quint_driver` base.** Withdrawn by
   QPROJ-1: the `cid_lease` driver is an in-crate module of
   `overdrive-control-plane`, which carries the `quint-connect`, `itf` and
   `anyhow` dev-dependencies; no shared base exists.
3. **Pool observable surface for `State`.** Closed by QPROJ-2:
   `GuestAddressPool::lease_observation()` → `GuestLeaseObservation`
   (`overdrive-control-plane/src/guest_network/flow_owner.rs`), plus the
   QPROJ-7 arbiter reads `SimGuestCidClaim::held_cids()` / `is_live`.
4. **Fork-boot cells of S6/S12.** Closed by DG-1 / DG-2 row 6: the VMM-handoff
   cell is P1-S12's attachment-cut cell; the guest-boot cells are P6-S13.
   P6-S8 is withdrawn with the VMM backend probe (DG-1).
5. **Registration lines.** Done for the authored files:
   `vsock_guest_transport_vocabulary` and `vsock_deploy_validation` in
   `overdrive-core/tests/acceptance.rs`; `vsock_cid_claim_handoff` in
   `overdrive-host/tests/integration.rs` under `#[cfg(feature = "kvm-tests")]`
   (binary already in `host-kernel-shared`). Attachment-cut bodies register
   with the cut.

No `SPECIFICATION_AMBIGUITY` blockers: C2 (the `cid_lease` state machine) is
specified in the Quint oracle; C5 (the reserved-range / port-count / capacity
mode behaviour) and C6 (the typed error taxonomy) are pinned in the DESIGN
feature-delta.

---

## 8. AT-completeness verdict (nw-at-completeness-check, 15-item)

Computed over the P1 candidate set (scenarios S1-S13 + the proptests):

| Item | Pass | Evidence |
|---|---|---|
| C1a empty/zero/min | ✅ | S1 offset 0; S8 reserved 0; S11 zero-over-cap boundary |
| C1b partition boundaries | ✅ | S10 (60,999/61,000/62,024/62,025); S11 (4,096/4,097); S8 (capacity±1) |
| C2a state machine documented | ✅ | the lease/claim machine is the Quint `cid_lease` module, referenced in §3 S13 + file header |
| C2b illegal-event-per-state | ✅ | S2 (retire→assign `LeaseRetiring`); S9 row 4; S5 (claim error mid-walk) |
| C3 count 0/1/N | ✅ | S4 (all offsets held); S1 (one); S2/S8 (many leases); S11 (0..4,097 ports) |
| C4a apply-twice/idempotency | ✅ | S6 take-claim twice → one Some; S2 replay of `assign` for an Admitted lease returns the same plan |
| C4b inverse without prerequisite | ✅ | S2/S6 release/`take_claim` without a live VMM; drop releases |
| C5a mode-flag combos | specified; owed to the cut | authored: S10 TCP×UDP × in/adjacent-range; at the attachment cut: S8 Admitted×Retiring reservation and the feature-level `AllocationSpec.transport` `None` vs `Some` cell (P6-S12b) |
| C5b flag orthogonality | specified; owed to the cut | authored: S11 UDP ports do not count toward the TCP limit (property); at the attachment cut: the feature-level `transport` orthogonality (P6-S12b) |
| C6a malformed input | ✅ | S1 out-of-range/unparseable CID; LaunchToken uppercase/wrong-len/zero |
| C6b each declared error triggered | ✅ | `OutOfRange`, `Zero`, `InUse`, `Kernel`, `ProbeMismatch`, `GuestCidsHeldElsewhere`, `GuestCidClaim`, `IntakeCapacityReached`, `ListenerPortReserved`, `TooManyTcpListenerPorts`, `LeaseRetiring`, `AdmissionCapReached` |
| C6c closed error set | specified; owed to the cut | S5 (an attachment-cut body) asserts a non-InUse failure is `Kernel` and skips no offset (no other error escapes the walk) |
| C7a degraded resource | ✅ | S4 (`GuestCidsHeldElsewhere` = CID-exhaustion-elsewhere); S8 (`IntakeCapacityReached`) |
| C7b interruption mid-operation | ✅ | S7 `serve` crash+restart; S6 failed `create` mid-handoff |
| C7c concurrent actors | ✅ | S3 holder racing `assign`; S6 concurrent clones racing `take_claim`; S12 second concurrent claim |

**Count: 15 / 15 over the P1 specification; C5a, C5b and C6c are owed to the
attachment cut, as in the feature-delta.** Over what is authored now, the
stateful-pool evidence for C2b, C4a/C4b (release), C5a (Admitted × Retiring),
C6b (`GuestCidsHeldElsewhere`, `GuestCidClaim`, `IntakeCapacityReached`,
`LeaseRetiring`, `AdmissionCapReached`), C6c and C7b (`serve` crash) is owed to
the attachment-cut bodies (DG-2 rows 1, 2, 3, 6), consistent with the
feature-level verdict ACCEPTABLE_WITH_DOCUMENTED_GAPS. (C7c is genuinely exercised — admission is a
concurrent claim arbiter — so it is not a documented-NA pass.) Verdict ≥
ACCEPTABLE_WITH_DOCUMENTED_GAPS; the gaps in §7 are all
`AT_GAP_IN_DELIVERY_SCOPE` (placement/shared-infra), zero `SPECIFICATION_AMBIGUITY`.

Completeness audit log (falsifier-gate telemetry, plan v3 §6.7):
`(netns-density-295/P1, {C1..C7}, finding_count=0, severity_max=none)` — the
candidate set covers every category; the only findings are placement gaps routed
within delivery scope.

---

## 9. Error/edge ratio

Sad/edge scenarios: S3, S4, S5, S8 (refusal), S9 (rows 2/4), S10, S11, S12
(InUse / bad-descriptor) = 8 of 13 named scenarios (≈ 62%), plus the sad proptest
arms (out-of-range CID, zero/upper/wrong-len token, over-cap, in-range port).
Well above the 40% floor (`.claude/rules` + Dimension 1).
