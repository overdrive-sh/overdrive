# DISTILL wave decisions — `backend-discovery-bridge-service-reachability`

**Wave**: DISTILL | **Mode**: PROPOSE | **Designer**: Quinn (acceptance-test-designer)
**Date**: 2026-05-20 | **Inherits**: DESIGN wave (Atlas APPROVED, 2026-05-20)

**Scope**: GH #174 (backend discovery bridge) + GH #175 (wire
`EbpfDataplane` into production single-mode boot). Joint DISTILL.

---

## DWD-01 — Walking-skeleton strategy: Tier 1 DST + Tier 3 real-kernel

**Decision**: the e2e gate is a single Tier 3 test (S-BDB-01) running
in Lima against real BPF maps, real XDP attach, real TCP round-trip.
Coverage of the bridge's reconcile semantics is delivered by Tier 1
DST invariants (three named: `BridgeEventuallyWritesBackendRow`,
`BridgeIdempotentSteadyState`, `BridgeRecomputesFingerprintOnReplay`).

**Rationale**: this project's tiered testing model
(`.claude/rules/testing.md`) supersedes the skill's A/B/C/D Python
framework. The bridge is exactly the §18 reconciler shape — its
correctness IS its convergence behavior under arbitrary inputs, which
DST proves. The production boot composition (#175) is the
real-kernel concern; only Tier 3 against a real `EbpfDataplane`
proves wiring.

**No fakes in the e2e gate**. Sim adapters appear only inside the
DST harness for fault-injection coverage. The Tier 3 walking-
skeleton exercises every adapter through its production binding.

**Equivalence to skill's framework**: roughly Strategy C
("end-to-end with real adapters") at the walking-skeleton level
plus Strategy A ("pure-logic property tests") at the invariant
level — but the project's own tier vocabulary is the SSOT.

---

## DWD-02 — Wave-decision reconciliation result: 0 contradictions

**Reconciliation status**: 0 contradictions with DESIGN.

**Cross-check performed**:

- No DISCUSS wave exists for this feature (GH issue bodies #174 +
  #175 ARE the DISCUSS input per the user's pre-dispatch decision).
  Story-to-scenario traceability is therefore traced via GH-AC
  identifiers (`@bdb-#174-N` / `@bdb-#175-N` tags) instead of
  `US-NN` story IDs.
- Every Q174.* and Q175.* decision in DESIGN wave-decisions.md is
  honored verbatim in DISTILL artifacts. Reviewer can grep the
  test-scenarios.md "Production code guarded" lines against the
  DESIGN architecture.md skeleton fns to confirm.
- D2 / D3 / D4 inheritance from DESIGN (decided 2026-05-21) is
  fully pinned in DISTILL:
  - **D2** (Earned-Trust probe in-scope for #175 Slice 2):
    S-BDB-14 (failure path) + S-BDB-15 (happy path) cover both
    probe branches.
  - **D3** (TCP round-trip in-gate for walking-skeleton): S-BDB-01
    asserts the round-trip explicitly. Bind-readiness wait shape
    (K1), listener choice (K2), echo payload (K3) all PINNED in
    test-scenarios.md § "Walking-skeleton flake-mitigation knobs".
  - **D4** (`getifaddrs` for `host_ipv4` in-scope for #175 Slice 2):
    S-BDB-16 (happy path) + S-BDB-17 (failure path) cover both.
- Three Atlas non-blocking questions (Q1, Q2, Q3) are addressed —
  see DWD-04 below.

**Conclusion**: APPROVED-by-design holds; DISTILL adds executable
spec + RED scaffolds with no surface drift.

---

## DWD-03 — D2 / D3 / D4 inheritance pin

### D2 (Earned-Trust probe) — PINNED

- `EbpfDataplane::probe()` shape per `architecture.md` § 5.4:
  write sentinel `BackendId::PROBE = u32::MAX` with known body,
  read back via typed `BackendMapHandle::get`, assert byte-equal,
  delete.
- Boot path calls `ebpf_dataplane.probe().await?` after `new()`
  and before any other dataplane operation.
- Failure → `DataplaneBootError::Probe { source: DataplaneError }`
  → `ControlPlaneError::DataplaneBoot(..)`.
- Acceptance scenarios: S-BDB-14 (probe failure refuses boot),
  S-BDB-15 (probe success advances boot).

### D3 (TCP round-trip in-gate) — PINNED

- Walking-skeleton (S-BDB-01) opens real TCP connection to
  `<assigned_vip>:<port>` and asserts byte-equal echo of the
  payload `walking-skeleton-probe\n`.
- **K1 bind-readiness wait shape**: poll-connect-with-timeout loop,
  50 ms cadence, 2 s budget (40 attempts). Termination = first
  successful round-trip.
- **K2 listener choice**: baked-in echo via Python one-liner (Form A
  — Python IS provisioned in `infra/lima/overdrive-dev.yaml`);
  DELIVER may swap to a Rust echo binary (Form B) if RSS becomes a
  flake source. If neither workable, DELIVER MUST add `socat` to
  Lima provisioning AND surface to user.
- **K3 echo payload + assertion**: literal bytes `walking-
  skeleton-probe\n` (24 bytes including trailing newline); response
  bytes-equals request.

### D4 (`getifaddrs` for `host_ipv4`) — PINNED

- Boot calls `resolve_iface_ipv4(&dataplane_cfg.client_iface)` once
  at startup, caches result on `AppState.host_ipv4`.
- The bridge is constructed with `host_ipv4` injected (constructor
  parameter, not late binding).
- Failure → `DataplaneBootError::IfaceAddrResolution { iface, source }`.
- Acceptance scenarios: S-BDB-16 (happy path), S-BDB-17 (no-IPv4
  iface refuses boot).

---

## DWD-04 — Atlas non-blocking Q1 / Q2 / Q3 disposition

### Atlas Q1 — TestServer fixture isolation

**Disposition**: addressed in `test-scenarios.md` § "TestServer
fixture isolation". The fixture lives entirely under
`crates/overdrive-control-plane/tests/integration/
backend_discovery_bridge/test_server.rs` (test directory). Any
inspection accessors on `EbpfDataplane` are `#[cfg(any(test, feature
= "integration-tests"))]`-gated. No `pub` widening of production
surface for test-only use.

DELIVER MUST verify the gating at landing time:
`cargo check -p overdrive-dataplane` (no `integration-tests`
feature) must compile cleanly without `dataplane_inspect()` in the
public API.

### Atlas Q2 — ViewStore crash semantics

**Disposition**: addressed by a new DST invariant
`BridgeRecomputesFingerprintOnReplay` (S-BDB-06). The harness
injects a crash between `ViewStore::write_through` fsync and the
runtime's in-memory `BTreeMap::insert` step, restarts, runs
`bulk_load`, then ticks — and asserts the bridge re-projects from
fresh inputs and behaves correctly. The invariant trait is
scaffolded RED at
`crates/overdrive-sim/src/invariants/backend_discovery_bridge.rs`
and registered in `crates/overdrive-sim/src/invariants/mod.rs`'s
`Invariant` enum.

The fsync-then-memory ordering rule
(`.claude/rules/development.md` § "Reconciler I/O") is the
structural property this invariant exercises; the bridge's
correctness under crash-recovery is therefore proved structurally.

### Atlas Q3 — Mutation-testing scope

**Disposition**: the production-code → scenario mapping table in
`test-scenarios.md` § "Production code → scenario / invariant
mapping" enumerates every mutation-killable production code path
the bridge + boot composition adds, with at least one acceptance
scenario or DST invariant per row. Notably:

- `View.insert` is exercised by S-BDB-02, S-BDB-04, S-BDB-10
  (every write path inserts) and S-BDB-05 (the dedup branch decides
  NOT to insert when fingerprint unchanged).
- `View.retain` (GC clause) is exercised by S-BDB-07 specifically;
  a mutant that flipped `retain(|sid, _| projected_rows.contains_key
  (sid))` to `retain(|_, _| true)` or `retain(|_, _| false)` would
  fail S-BDB-07.
- The dedup `if Some(&new_fp) == prev_fp { continue }` decision is
  exercised by S-BDB-05; a mutant flipping the `==` would emit
  redundant rows and fail the invariant.

**All rows have at least one guarding scenario; zero TBDs**.
DELIVER's per-step mutation runs (`cargo xtask mutants --diff
origin/main --features integration-tests --package
overdrive-control-plane --file <files-touched>`) MUST achieve
≥ 80% kill rate per the project gate.

---

## DWD-05 — Adapter-coverage audit result

Per Mandate 6 / skill spec, every driven adapter has a `@real-io`
scenario. Audit table lives in `test-scenarios.md` § "Adapter
coverage table"; result: **PASS, zero empty rows**.

Notable cross-tier coverage:

- `EbpfDataplane::update_service` — Tier 3 only (the production
  binding); Tier 1 uses `SimDataplane` which proves the upstream
  bridge's behavior independent of the adapter.
- `LocalObservationStore::write` for `ServiceBackendRow` — Tier 3
  only (transitively via the bridge's action shim during S-BDB-01);
  Tier 1 uses `SimObservationStore`.
- `ServiceVipAllocator::get` — Tier 3 via
  `PersistentServiceVipAllocator`; Tier 1 via
  `SimServiceVipAllocator` (DST harness primes the memo as a
  precondition).

---

## DWD-06 — Error-path coverage assessment

**Skill target**: ≥ 40% error-path scenarios.

**Actual**: 8 of 20 scenarios are error/negative/failure paths =
**40%**. Threshold met.

Error-path scenarios:
- S-BDB-06 — crash-recovery (`@error_path` per replay semantics)
- S-BDB-08 — wrong workload kind (negative — no row written)
- S-BDB-09 — zero listeners (negative — no row written)
- S-BDB-12 — missing `[dataplane]` config
- S-BDB-13 — invalid iface
- S-BDB-14 — Earned-Trust probe failure
- S-BDB-17 — `getifaddrs` failure
- S-BDB-20 — attach-mode fallback (negative path — native rejected)

---

## DWD-07 — Mandate compliance summary (CM-A / CM-B / CM-C / CM-D)

| Mandate | Compliance evidence |
|---|---|
| **CM-A** Hexagonal boundary | Walking-skeleton enters through HTTPS `POST /v1/workloads:submit`; boot scenarios enter through `serve_with_config`. No scenario reaches into internal `BackendDiscoveryBridge::reconcile` directly. Reviewer can grep test bodies for `BackendDiscoveryBridge::new` calls outside DST invariants — none expected outside the harness wiring. |
| **CM-B** Business language | Gherkin uses domain terms throughout: `Service`, `listener`, `backend`, `VIP`, `intent`, `observation`, `alloc`, `reconciler`, `bridge`. Technical jargon (`HTTP 200`, `JSON`, `Redis`, etc.) absent. Reviewer can grep for the technical terms — zero matches expected. |
| **CM-C** User journey completeness | Walking-skeleton describes complete user journey: submit → allocation → backend → kernel → reachability. Focused scenarios test specific business rules at the bridge boundary (dedup, GC, multi-listener, wrong kind, zero listeners). |
| **CM-D** Pure function extraction | The bridge's `reconcile` body IS the pure function — `(desired, actual, view, tick) → (actions, next_view)` per ADR-0035. The dedup decision and the View GC are pure sub-functions; both have mutation coverage per the Q3 mapping table. Impure work (allocator lookup, intent read, obs read) lives at the runtime hydrate boundary, NOT in the bridge. |

---

## DWD-08 — Genuinely ambiguous spec items surfaced as BLOCKERS

**None.**

The DESIGN wave's APPROVED status + the user's pre-dispatch
decisions (D2 / D3 / D4 in-scope, joint #174+#175 DISTILL, no
DISCUSS, Lima as the canonical env) leave no genuine ambiguity for
DELIVER. The only open shape — K2's exact echo binary choice — is
pinned with a default (Python one-liner Form A) and a clear
fallback rule ("if Form A/B unworkable, add `socat` to Lima AND
surface to user").

If DELIVER discovers a genuine gap mid-implementation that DISTILL
did not anticipate, the correct response is to surface it as a
blocker via the user-feedback path, not to invent a new spec
element inline.

---

## DWD-09 — Files created in this DISTILL wave

| Path | Purpose |
|---|---|
| `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md` | Executable spec SSOT (GIVEN/WHEN/THEN) |
| `docs/feature/backend-discovery-bridge-service-reachability/distill/walking-skeleton.md` | Walking-skeleton spec + user-centric framing + demo script |
| `docs/feature/backend-discovery-bridge-service-reachability/distill/wave-decisions.md` | THIS FILE — DWD-01..09 |
| `docs/feature/backend-discovery-bridge-service-reachability/distill/acceptance-review.md` | Placeholder for review wave (reviewer fills in) |
| `crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs` | RED scaffold: walking-skeleton tests (3 tests, one per Tier 3 scenario class) |
| `crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/boot_composition.rs` | RED scaffold: boot-composition tests (S-BDB-11..S-BDB-17, S-BDB-20) |
| `crates/overdrive-sim/src/invariants/backend_discovery_bridge.rs` | RED scaffold: three DST invariants (`BridgeEventuallyWritesBackendRow`, `BridgeIdempotentSteadyState`, `BridgeRecomputesFingerprintOnReplay`) |

Files modified:

| Path | Edit |
|---|---|
| `crates/overdrive-control-plane/tests/integration.rs` | Add `mod backend_discovery_bridge { mod walking_skeleton; mod boot_composition; }` declaration |
| `crates/overdrive-sim/src/invariants/mod.rs` | Add `pub mod backend_discovery_bridge;` + add three new variants to `Invariant` enum (`BridgeEventuallyWritesBackendRow`, `BridgeIdempotentSteadyState`, `BridgeRecomputesFingerprintOnReplay`) |

---

## DWD-10 — Handoff to DELIVER

**Sequencing** (per DESIGN QJ.2 — Option B):

1. **Slice 1 (closes #174)**: implement `BackendDiscoveryBridge` +
   register in production boot under `NoopDataplane`. Replace the
   RED bodies in
   `crates/overdrive-sim/src/invariants/backend_discovery_bridge.rs`
   with the implementations of the three invariants. Slice 1 lands
   green when DST passes. No walking-skeleton yet (it's
   `#[should_panic]` until Slice 2).
2. **Slice 2 (closes #175)**: replace `NoopDataplane` with
   `EbpfDataplane`. Add `DataplaneBootError`. Add `[dataplane]`
   config section + `resolve_iface_ipv4`. Replace the RED bodies in
   `walking_skeleton.rs` + `boot_composition.rs` with real tests.
   Slice 2 lands green when Tier 3 walking-skeleton + boot scenarios
   pass through `cargo xtask lima run --`.

**One scenario enabled at a time** (per skill mandate): all
scenarios except S-BDB-02 (the first DST invariant) and S-BDB-12
(the simplest boot-refusal scenario, no XDP needed) are
`#[should_panic(expected = "RED scaffold")]`-gated. Slice 1 enables
S-BDB-02 first; Slice 2 enables the rest sequentially in the order
listed in test-scenarios.md § "Scenario index".

**Mutation gate** (per `.claude/rules/testing.md`): every DELIVER
step that touches files in the per-step Production-code mapping
runs `cargo xtask mutants --diff origin/main --features
integration-tests --package <pkg> --file <step-files>`. Final
per-PR gate runs the unfiltered per-package diff. ≥ 80% kill rate.

**Definition-of-Done verification** (handoff to DELIVER):

1. ✅ Acceptance scenarios written with GIVEN/WHEN/THEN bodies (20 scenarios).
2. ⏳ Test pyramid complete: 20 acceptance scenarios + planned per-step unit tests (DELIVER's responsibility).
3. ⏳ Peer review (post-DISTILL Acceptance Reviewer — Sentinel) — pending.
4. ⏳ Tests run in CI/CD pipeline (DELIVER lands the CI step in Slice 2 if not already present).
5. ⏳ Story demo-able to stakeholders — walking-skeleton (S-BDB-01) IS the demo per `walking-skeleton.md` § "Demo script". Demo-able from Slice 2 green CI.

Items 2–5 transition to ✅ across DELIVER waves; DISTILL has
discharged its responsibilities for items 1 + scaffolding.
