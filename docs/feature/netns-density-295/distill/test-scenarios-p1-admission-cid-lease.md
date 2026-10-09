# P1 — Admission, CID lease & kernel claim, intake capacity, deploy validation

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
production `fd=` wiring is a P6/DELIVER fork cell.

Design pins: feature-delta.md §"[REF] Core vocabulary — overdrive-core"
(L17449-17888), §"State-ownership matrix" (L20060), §"Gate G-V0" (L20078),
§"Required boundary scenarios" (L20411), §"Validation plan" (V-25, L20607),
§"Formal protocol model" (`cid_lease`, L20958 + invariant 5 L21045 + hazards
L21120). ADRs 0155, 0156, 0170, 0176, 0178, 0179.

---

## 1. Files owned (disjoint) + placement decisions

| File | Crate / lane | Hosts | Status |
|---|---|---|---|
| `crates/overdrive-core/tests/acceptance/vsock_guest_address_pool.rs` | overdrive-core, unit+proptest (L1-2) | the `guest_transport` **vocabulary** + pure functions + the `GuestCidClaim` port contract (via `SimGuestCidClaim`, reachable through the overdrive-core↔overdrive-sim dev-dep back-edge) + `GuestTransportAssignment::take_claim` | authored |
| `crates/overdrive-core/tests/acceptance/vsock_deploy_validation.rs` | overdrive-core, unit+proptest (L1-2) | deploy/API listener validation (both ingresses are overdrive-core) | authored |
| `crates/overdrive-host/tests/integration/vsock_cid_claim_handoff.rs` | overdrive-host, Tier-3 metal (L4/L6), `kvm-tests` | the real `VhostVsockCidClaim` claim + its claim-level handoff + probe | authored |
| `crates/overdrive-sim/src/adapters/guest_cid_claim.rs` | overdrive-sim (`adapter-sim`) | `SimGuestCidClaim` | authored |
| `tests/conformance/tests/vsock_cid_lease_conformance.rs` | conformance, quint-connect (Lima) | the `cid_lease` driver over the **production pool** + `SimGuestCidClaim` | authored |

**Load-bearing placement finding (surfaced, not worked-around).** The
production `GuestAddressPool` / `GuestNetworkPlan` / `GuestNetworkError` live in
`overdrive_control_plane::guest_network` and the design keeps them there (the
new error variants are added "beside `GuestNetworkError`"; `assign` is
`pub(crate)`). `overdrive-core` does **not** depend on `overdrive-control-plane`
(a cycle), so the **stateful-pool** scenarios (P1-S2, the pool half of S3-S5,
S7-seeded, the stateful half of S8, S9) **cannot** be hosted in the two
overdrive-core files P1 owns. They are covered two ways that ARE in P1's set:
the **`cid_lease` conformance driver** (drives the production pool against the
formal oracle) and this doc's GWT spec. Their **example-based seeded-sim**
bodies belong in `crates/overdrive-control-plane/tests/acceptance/vsock_guest_address_pool.rs`
with an extended `e18_test_support` wrapper — a file NOT in P1's allowed set.
See §7 "Placement gaps". No production API or accessor is invented to dodge this
(CLAUDE.md §"Implement to the design"; `.claude/rules/testing.md` §"RED
acceptance scaffolds").

**Registration deferred (shared-infra §6).** None of the five files is
registered in a shared entrypoint (`tests/acceptance.rs`, `tests/integration.rs`,
`adapters/mod.rs`) or the nextest config. The crafter adds the `mod …;` line /
the `host-kernel-shared` group entry when each compiling body co-arrives with
the production surface it calls, during the DELIVER roadmap reconciliation.
Adding them now would break the whole-workspace compile before the surface
exists (the premature-churn hazard the planner avoided).

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

**P1-S1 — next-fit `assign` derives a collision-free CID (D16).**
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
- Rust home: **overdrive-core part** `vsock_guest_address_pool.rs::
  gcid_roundtrips_and_derivation_is_base_plus_offset` (+ the newtype
  roundtrip/range `gcid_rejects_out_of_range_and_reserved`). **Stateful
  next-fit part** → conformance driver (`AssignTry`/`CreateOk`) + control-plane
  acceptance (gap §7).
- Activation: `#[ignore]` → DELIVER vsock admission step (`guest_transport`).

**P1-S2 — lease replay / retire / release change only the named allocation.**
- GWT: *Given* any sequence of `assign`/retire/`release` over a fixed alloc
  set, *When* applied, *Then* each step mutates only that allocation's lease,
  CID claim and reservation; all others unchanged.
- Contract: ADR-0132/0133 lease lifecycle; D16 (one CID per lease).
- Owner path: `GuestAddressPool::{assign, release}` (control-plane).
- Fault: arbitrary interleavings (property). · Oracle (Universe): the full
  offset→alloc lease map + `held_cids()` + reservation sum — only the named
  offset changes per op (fail-closed on adjacent mutation). · Tier: seeded sim
  through the production pool + conformance (`Release`/`Redeploy`).
- Rust home: conformance driver + **control-plane acceptance (gap §7)**.
- Activation: DELIVER admission step.

**P1-S3 — CID claim is atomic with the lease (R5-19a).**
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
- Rust home: **claim-port level** `vsock_guest_address_pool.rs::
  claim_is_exclusive_and_drop_releases`. **Pool skip** → conformance +
  control-plane acceptance (gap §7).
- Activation: DELIVER admission step.

**P1-S4 — all free offsets held elsewhere → `GuestCidsHeldElsewhere` (R5-19b).**
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
- Rust home: conformance (`SurvivorCreate` on all offsets → `OrphanExit`) +
  control-plane acceptance (gap §7).
- Activation: DELIVER admission step.

**P1-S5 — non-`InUse` claim failure → `GuestCidClaim`, skips no offset (R5-19c).**
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
- Rust home: **claim-port level** `vsock_guest_address_pool.rs::
  non_inuse_claim_failure_is_kernel_not_inuse`. **Pool no-skip** → conformance +
  control-plane acceptance (gap §7).
- Activation: DELIVER admission step.

**P1-S6 — claim handed to the VMM exactly once (R5-19d).**
- GWT: *Given* an Admitted lease whose plan carries a take-once claim slot,
  *When* `create` returns `Ok`, *Then* `serve` holds no copy and a second
  `take_claim` returns `None`; a failed `create` and a pre-launch `release` each
  free the CID.
- Contract: D16-CLAIM — move-only claim; `GuestTransportAssignment::take_claim`
  atomic across clones.
- Owner path: `GuestTransportAssignment::take_claim` (core) →
  `Vmm::create(config, vsock)` (host) → `serve` closes its copy.
- Fault: concurrent clones racing `take_claim`; a `create` error. · Oracle:
  exactly one `Some` across all clones; the claim released on failed create /
  pre-launch release (Universe = take-claim outcomes + `held_cids`). · Tier:
  unit (core take-once) + Tier-3 claim-level (host) + P6 walking skeleton (fork
  boot).
- Rust home: **core** `vsock_guest_address_pool.rs::
  assignment_take_claim_yields_exactly_one_some_across_clones`; **host**
  `vsock_cid_claim_handoff.rs::claimed_device_held_refuses_reclaim_until_released`;
  **fork boot** → P6 `vsock_walking_skeleton.rs` (gap §7).
- Activation: core/host `#[ignore]` → DELIVER admission step; fork cell →
  DELIVER step 01.

**P1-S7 — surviving VMM across a `serve` crash (R5-19e/f).**
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
- Rust home: conformance (`vsock_cid_lease_conformance.rs::cid_lease_live_conformance`)
  + control-plane seeded sim (gap §7) + P6 crash matrix.
- Activation: DELIVER admission step.

**P1-S8 — intake capacity reservation (R5-37, D8a-CAP).**
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
- Rust home: **pure predicate** `vsock_guest_address_pool.rs::
  capacity_predicate_refuses_exactly_on_overflow`. **Stateful pool + Retiring**
  → control-plane acceptance (gap §7); the Quint `intake` module is P4.
- Activation: DELIVER admission step.

**P1-S9 — G-V0 boundary rows.**
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
- Rust home: conformance + control-plane acceptance (gap §7).
- Activation: DELIVER admission step.

### Deploy / API validation

**P1-S10 — reserved-range listener refused (R5-21, D25).**
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
- Rust home: `vsock_deploy_validation.rs::{deploy_refuses_reserved_listener_port_naming_the_range,
  deploy_accepts_ports_adjacent_to_the_reserved_range,
  api_ingress_refuses_reserved_listener_port_with_validation,
  deploy_refusal_iff_port_in_reserved_range}`.
- Activation: DELIVER admission/validation step.

**P1-S11 — too-many TCP listener ports (R5-44, INTAKE-PORTS).**
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
- Rust home: `vsock_deploy_validation.rs::{deploy_accepts_exactly_the_block_size_and_refuses_one_more,
  api_ingress_refuses_too_many_tcp_listener_ports_with_validation,
  deploy_refusal_iff_distinct_tcp_ports_over_the_constant}`.
- Activation: DELIVER admission/validation step.

### Kernel claim handoff — Tier-3

**P1-S12 — claim + handoff (V-25 / R5-19d / CLAIM-PINS).**
- GWT: `VHOST_VSOCK_SET_GUEST_CID` on an unowned instance holds the CID
  exclusively while any reference is open; a second claim fails `EADDRINUSE` →
  `InUse`; the move-only device is handed to the fork by descriptor (fd=) with
  `serve` closing its copy after spawn; released at the last close (≤ 10 ms,
  reported not hard-asserted); the probe holds under the `serve` identity; a
  bad descriptor makes CH exit before READY with the CID still claimable (never
  a CID-clash cause).
- Contract: D16-CLAIM / CLAIM-PINS / A-29 (V-25 done, P-39).
- Owner path: `VhostVsockCidClaim` (host) + `ClaimedGuestCid::into_device()` →
  CH fork `--vsock cid=…,backend=vhost-kernel,fd=K`.
- Fault: a second concurrent claim; a dropped reference; a bad descriptor. ·
  Oracle: `InUse` on the second claim; re-claimable after the last close; probe
  `Ok`; guest-reported peer CID = claimed CID + byte round-trip (fork cell);
  pre-READY exit projected through the existing VMM-exit start failure
  (Universe = the kernel holder table observed via a second claim + the
  guest-reported CID). · Tier: **example-based Tier-3 metal** (`kvm-tests`), NO
  PBT.
- Rust home: **claim-level** `vsock_cid_claim_handoff.rs::{claim_on_unowned_instance_holds_cid_exclusively,
  claim_released_at_last_reference_close, claim_probe_holds_under_serve_identity,
  claimed_device_held_refuses_reclaim_until_released}`. **Fork-boot cells**
  (guest-reported CID, byte round-trip, pre-READY exit on bad descriptor) → P6
  `vsock_walking_skeleton.rs` (needs the fork `fd=` mode = DELIVER step 01; gap
  §7).
- Activation: `#[ignore]` → DELIVER step 01 (`VhostVsockCidClaim`; fork cells
  additionally gated on the fork `fd=` mode).

### Conformance

**P1-S13 — `cid_lease` conformance (invariant 5).**
- GWT: the quint-connect driver replays generated `cid_lease` traces (instances
  `lease_ok` / `lease_live`) against the production `GuestAddressPool` +
  `SimGuestCidClaim`, comparing the observable `(kern, lease, cursor)` after
  every step; invariant 5 (`Safety` = `CidUnique`, `ClaimHeld`, `NoLaunchClash`,
  `NoSkipWhileClaimable`, `NoLeakedClaim`, `RefusalEachClaimInUse`) and
  placement liveness (`EventuallyPlaced`) hold; the three hazard variants
  (`check-then-claim`, `release-before-VMM`, `remember-InUse`) are the teeth
  (model-level, in `checks.toml`).
- Contract: D16-CLAIM; invariant 5; progress ("placed whenever a free offset's
  CID stays unheld long enough for one claim").
- Owner path: production pool via the DELIVER-extended `e18_test_support` +
  `SimGuestCidClaim` at the driven port; State projected from the pool's
  observable lease/claim surface + `held_cids()`.
- Fault: survivor holders, serve crash+restart, pre-READY exits, claim errors
  (all driven by the spec's actions). · Oracle: per-step state equality vs the
  spec. · Tier: conformance (Lima, spawns `quint`).
- Rust home: `vsock_cid_lease_conformance.rs::{cid_lease_ok_conformance,
  cid_lease_live_conformance}`.
- Activation: gated on the shared quint-connect base + the conformance crate's
  `quint-connect` dev-dep + the pool's observable surface (gaps §7).

---

## 4. G-V0 boundary table (feature-delta L20415-20420) → scenario

| Row | G-V0 obligation | Scenario |
|---|---|---|
| 1 Available | Lease + CID advances the `StartAllocation` assignment | P1-S1, S13 |
| 2 Unavailable/typed | `AdmissionCapReached`; `IntakeCapacityReached`; `GuestCidsHeldElsewhere`/`GuestCidClaim` | P1-S4, S5, S8, S9 |
| 3 Unrelated unchanged | running allocations unaffected | P1-S9 |
| 4 Late success | a late `assign` after retire refused (`LeaseRetiring`) | P1-S2, S9 |
| 5 Disconnect/reconnect | — (no G-V0 disconnect obligation; CID survival is S7) | S7 (crash edge) |
| 6 Feature disabled | single-cut; substitute = `transport: None` sim fixture | noted; no switch |

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

**V-items:** V-25 done (P-39) → S12 (claim level) + P6 fork cell; the design
records "nothing blocked on a validation item" for P1.

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
- **Mandate 10 (two-tier):** Tier A = the conformance driver over the production
  composition (example/trace). Tier B state-machine PBT is **not added** —
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
- **Pillar 3 (production composition):** the conformance driver boots the
  production pool via `run_server`/`e18_test_support`; only the external/non-
  deterministic ports (`Entropy`, the CID claim's kernel) are faked
  (`SimEntropy`, `SimGuestCidClaim`). The Tier-3 host file uses the real kernel.

---

## 7. Placement gaps & shared-infra dependencies (report-only; NOT worked around)

1. **Stateful-pool acceptance bodies need a control-plane file + an extended
   `e18_test_support`.** P1-S2, the pool half of S3-S5, S7-seeded, the stateful
   half of S8, and S9 exercise `GuestAddressPool::assign` (control-plane,
   `pub(crate)`). Their example-based seeded-sim bodies belong in
   `crates/overdrive-control-plane/tests/acceptance/vsock_guest_address_pool.rs`
   (NOT in P1's allowed set), reached via an `e18_test_support` wrapper extended
   for the new `assign(alloc, &[ListenPort])` signature + `SimGuestCidClaim` /
   `SimEntropy` injection + an **observable lease/claim projection**
   (`observe_leases`/`observe_cursor` — a testability boundary the design implies
   but pins no accessor for). Covered meanwhile by the `cid_lease` conformance
   driver + this doc. Routing: this is `AT_GAP_IN_DELIVERY_SCOPE` (a Quinn/ DELIVER
   placement task), not `SPECIFICATION_AMBIGUITY`.
2. **Conformance crate deps.** `tests/conformance/Cargo.toml` must gain
   `quint-connect.workspace = true` under `[dev-dependencies]`; `src/lib.rs` must
   gain the shared `pub mod quint_driver;` base (`tests/conformance/src/
   quint_driver.rs`, shared-infra §4). Both shared, created by the first quint
   piece / DELIVER — not P1's to edit.
3. **Pool observable surface for conformance `State::from_driver`.** The design
   says State is "projected from the pool's observable lease/claim surface" but
   pins no accessor; DELIVER must expose one (shared-infra §2 observable-only
   rule). Surfaced per `.claude/rules/testing.md` §"Quint conformance" ("a
   testability gap to surface for design approval, not licence to add API").
4. **Fork-boot cells of S6/S12 live in P6's walking skeleton.** The guest-
   reported-CID / byte-round-trip / pre-READY-exit-on-bad-descriptor cells need a
   real CH-fork `fd=` boot (the fork's `fd=` mode is DELIVER step 01) through
   `serve` + `deploy`; they belong in `tests/conformance/tests/
   vsock_walking_skeleton.rs` (P6), cross-referenced from the P1 host file.
5. **Registration lines** (`mod …;` in the three shared entrypoints + the
   `host-kernel-shared` nextest group entry) co-arrive in DELIVER (shared-infra
   §6); not pre-created by DISTILL.

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
| C5a mode-flag combos | ✅ | S10 TCP×UDP × in/adjacent-range; S8 Admitted×Retiring reservation |
| C5b flag orthogonality | ✅ | S11 UDP ports do not count toward the TCP limit (property) |
| C6a malformed input | ✅ | S1 out-of-range/unparseable CID; LaunchToken uppercase/wrong-len/zero |
| C6b each declared error triggered | ✅ | `OutOfRange`, `Zero`, `InUse`, `Kernel`, `ProbeMismatch`, `GuestCidsHeldElsewhere`, `GuestCidClaim`, `IntakeCapacityReached`, `ListenerPortReserved`, `TooManyTcpListenerPorts`, `LeaseRetiring`, `AdmissionCapReached` |
| C6c closed error set | ✅ | S5 asserts a non-InUse failure is `Kernel` and skips no offset (no other error escapes the walk) |
| C7a degraded resource | ✅ | S4 (`GuestCidsHeldElsewhere` = CID-exhaustion-elsewhere); S8 (`IntakeCapacityReached`) |
| C7b interruption mid-operation | ✅ | S7 `serve` crash+restart; S6 failed `create` mid-handoff |
| C7c concurrent actors | ✅ | S3 holder racing `assign`; S6 concurrent clones racing `take_claim`; S12 second concurrent claim |

**Count: 15 / 15 → COMPLETE.** (C7c is genuinely exercised — admission is a
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
