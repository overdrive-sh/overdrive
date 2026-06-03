# backend-discovery-bridge-service-reachability — Feature Evolution

**Feature ID**: `backend-discovery-bridge-service-reachability`
**Branch**: `marcus-sa/backend-discovery-bridge`
**Duration**: 2026-05-20 (roadmap landed) — 2026-05-23 (DELIVER + finalize close)
**Status**: Delivered — 9/9 DELIVER roadmap steps complete (`01-01..01-05`, `02-01..02-04`),
plus 3 upstream-issue resolutions (UI-04 / UI-05 / F1+UI-06) and ADR-0053
walking-skeleton TCP path landing. DES integrity 9/9 steps with 5/5 TDD
phases logged each. Workspace clippy + dst-lint clean; Tier 3
walking-skeleton (S-BDB-01) green via real `connect(10.96.0.2:8080)`
through `cgroup_connect4_service`.
**ADRs**: [ADR-0052](../product/architecture/adr-0052-backend-discovery-bridge.md)
(pre-feature; sets the bridge / hydrator split that this feature implements),
[ADR-0053](../product/architecture/adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md)
(mid-feature; same-host backend delivery via `cgroup_sock_addr`).
**Closes**: GH #174 (BackendDiscoveryBridge reconciler), GH #175
(`EbpfDataplane` production boot composition).

---

## What shipped

The convergence-loop wiring + dataplane integration that makes a
`Service` workload reachable end-to-end through its allocator-issued
VIP on a single-node Phase 1 deployment. Operator submits a Service
spec → control-plane issues a VIP and persists intent → reconcilers
converge → kernel-side BPF maps populate → operator `connect(VIP,
port)` from the same host reaches the workload's TCP listener.

Three layers shipped together:

1. **`BackendDiscoveryBridge` reconciler** (closes #174) — the
   intent-derived `(VIP, listener.port)` × observed-Running-allocs →
   `ServiceBackendRow` translation step. Pure `reconcile`, View-carried
   dedup fingerprint, runtime-owned CBOR persistence per ADR-0035.
2. **`EbpfDataplane` production boot** (closes #175) — operator
   `[dataplane]` config section, `host_ipv4` resolution via
   `getifaddrs`, XDP attach with generic-mode fallback, Earned-Trust
   probe, RAII detach. Replaces the prior `NoopDataplane` placeholder
   in a single-cut migration.
3. **Same-host backend delivery via `cgroup_connect4_service`** (ADR-0053,
   landed mid-feature once the walking-skeleton TCP round-trip RCA
   identified that XDP alone is structurally insufficient for
   loopback-style same-host LB on a Phase 1 shared-netns deployment).
   Adds a `cgroup/connect4` BPF program + `LOCAL_BACKEND_MAP` infrastructure
   + `Dataplane::register_local_backend` trait method.

### Production code

**Bridge reconciler** (`crates/overdrive-control-plane/src/reconcilers/backend_discovery_bridge/`):

- `BackendDiscoveryBridge::reconcile` pure body — projects desired
  endpoints from `ServiceV1.listeners + assigned_vip`, observes Running
  allocs from `alloc_status_rows`, computes a deterministic
  `BackendSetFingerprint`, dedups against the View, emits
  `Action::WriteServiceBackendRow` on drift + dual-emit
  `Action::EnqueueEvaluation { reconciler: "service-map-hydrator", … }`
  (UI-05 pattern).
- Action shim arms: `write_service_backend_row::dispatch` (obs write) +
  `enqueue_evaluation::dispatch` (broker submit).
- Runtime hydration arms (`hydrate_desired` + `hydrate_actual`) for
  bridge state, threaded through `AnyReconciler` / `AnyState`.
- Boot registration immediately followed by `service_map_hydrator()`
  registration so cross-reconciler handoffs resolve on first drain.

**Dataplane boot** (`crates/overdrive-control-plane/src/{config,app_state,lib}.rs` +
`crates/overdrive-dataplane/src/lib.rs`):

- `[dataplane]` config section (operator-facing `client_iface`,
  `service_vip_range` already covered by ADR-0049).
- `resolve_iface_ipv4(&iface_name) -> Result<Ipv4Addr,
  DataplaneBootError::IfaceAddrResolution>` via `getifaddrs`.
  Cached on `AppState.host_ipv4`; bridge constructor takes it as
  a mandatory parameter.
- `EbpfDataplane::new` — embedded BPF object bytes (with
  `OVERDRIVE_BPF_OBJECT` dev override), XDP attach with
  `XdpFlags::DRV_MODE` first + `SKB_MODE` fallback on `EOPNOTSUPP /
  ENOTSUP` (single structured `xdp.attach.fallback_generic` warn),
  RAII `Drop` detach.
- `EbpfDataplane::probe` — write `BackendId::PROBE` sentinel, read
  via typed `BackendMapHandle::get`, assert byte-equal, delete.
  Failure → `DataplaneBootError::Probe`. Boot calls it once before
  any dataplane operation.
- `DataplaneBootError` variants: `IfaceAddrResolution`, `Probe`,
  `BpfLoad`, `XdpAttach`, etc. — typed pass-through to
  `ControlPlaneError::DataplaneBoot(#[from] …)` per development.md
  § "Distinct failure modes get distinct error variants."

**Same-host delivery** (ADR-0053; `crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs`
+ `crates/overdrive-dataplane/src/maps/local_backend_map_handle.rs` +
`crates/overdrive-control-plane/src/reconciler/service_map_hydrator.rs`):

- `cgroup_connect4_service` BPF program — runs on `cgroup/connect4`
  hook; reads `(user_ip4, user_port)` from `bpf_sock_addr` context,
  looks up the VIP in `LOCAL_BACKEND_MAP`, rewrites destination on
  hit, lets the kernel TCP stack handle the rest. No userspace path
  involved; no XDP ingress traversal required for same-host paths.
- `LOCAL_BACKEND_MAP` — typed `(ServiceKey) → LocalBackend` map plus
  `register_local_backend / deregister_local_backend` accessors.
  Backend-discovery → hydrator pipeline writes here in parallel with
  the existing `SERVICE_MAP` write for remote-eligible backends.
- `ServiceMapHydrator` classifies backends as Local-vs-Remote based on
  observed-IP-vs-host-IPs; routes Local to `register_local_backend`
  (cgroup path) and Remote to `update_service` (XDP path). XDP
  programs unchanged.
- `cc67038a` (post-walking-skeleton): hydrator rejects loopback,
  link-local, and multicast backend addresses with a typed
  `BackendAddressRejection` variant — the structural defense against
  ever writing `127.0.0.1` into `LOCAL_BACKEND_MAP` (which would route
  every connect-rewrite through loopback regardless of source).

**Cross-reconciler enqueue plumbing** (UI-05 + F1/UI-06):

- `Action::EnqueueEvaluation { reconciler: ReconcilerName, target:
  TargetResource }` — generic transport action; submitted to the
  per-runtime `EvaluationBroker` via a brief sync lock grab. Used by:
  - bridge → hydrator handoff (UI-05; commit `f3a3f4ad`)
  - `WorkloadLifecycle` → bridge handoff for `StartAllocation` /
    `RestartAllocation` / `StopAllocation` / `FinalizeFailed`
    transitions (F1/UI-06; commit `3b87b653`) — closes the
    Pending→Running cold-start gap that made long-lived workloads
    structurally invisible to the bridge.

**Production code crate addition** (`crates/overdrive-testing`):

- New `adapter-host`-class crate (`bd10bf18`). Owns shared real-infra
  test fixtures: `NetNs`, `ThreeIfaceTopology`, route + sysctl
  plumbing. Consumers add `overdrive-testing.workspace = true` to
  `[dev-dependencies]`. Today: `overdrive-dataplane` integration
  tests (`reverse_nat_e2e`, `sanity_mixed_batch`, etc.) and the
  walking-skeleton in `overdrive-control-plane`. See
  `development.md` § "Shared real-infra test fixtures —
  overdrive-testing" for promotion criteria.

**ExecDriver opt-in netns** (`51512d7c`):

- `ExecDriver::start` takes an optional `netns_path: Option<PathBuf>`;
  child process enters the netns via `setns(CLONE_NEWNET)` before
  `execve`. Mirrors CNI spec; unused in production today (Phase 1
  shared-netns), exercised by the walking-skeleton's
  three-netns topology. The shape is the natural Phase 2+ extension
  point for per-workload netns isolation.

### Test coverage

Acceptance + DST + Tier 3 spread across crates by ownership:

- **`crates/overdrive-core/tests/acceptance/`** — `Action::EnqueueEvaluation`
  unit + `workload_lifecycle_terminal_decision.rs` (extended for the
  new variant per single-cut migration).
- **`crates/overdrive-control-plane/tests/acceptance/`** —
  S-BDB-02..S-BDB-10 (bridge reconcile semantics: dedup, GC,
  multi-listener, wrong-kind, zero-listeners),
  S-BDB-12..S-BDB-17, S-BDB-20 (boot composition: config missing,
  invalid iface, probe fail/pass, getifaddrs fail/pass, attach-mode
  fallback), plus the UI-05 / UI-04 / F1 follow-ups
  (`service_workload_emits_start_allocation.rs`,
  `bridge_emits_enqueue_evaluation_for_hydrator.rs`,
  `service_map_hydrator_registered_at_boot.rs`).
- **`crates/overdrive-sim/src/invariants/backend_discovery_bridge.rs`** —
  three named Tier 1 DST invariants:
  `BridgeEventuallyWritesBackendRow` (S-BDB-03),
  `BridgeIdempotentSteadyState` (S-BDB-05),
  `BridgeRecomputesFingerprintOnReplay` (S-BDB-06 / Atlas Q2 crash
  semantics). Plus `evaluate_bridge_to_hydrator_handoff` (S-BDB-19,
  ADR-0042 fingerprint identity across the reconciler boundary).
- **`crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs`** —
  Tier 3 S-BDB-01: real submit through HTTPS POST `/v1/jobs` → real
  ExecDriver Python listener → real `cgroup_connect4_service`
  rewrite → real TCP echo. Three netns topology via the new
  `overdrive-testing` fixtures + ExecDriver `netns_path` opt-in.

### Mutation testing

Per-step kill-rate gate met on every touched file. Three structural
exclusions in `.cargo/mutants.toml` (each justified inline):

- `verifier_regress` Tier 4 binary (`a9bcd288`) — verifier-budget
  gate, no per-call-path semantics to mutate.
- `EbpfDataplane` / `LocalBackendMapHandle` Tier-3-only methods
  (`f8118fac`) — the structural defense lives in Tier 3 walking-skeleton
  + the `register_local_backend` precondition docstring (per
  development.md § "Trait definitions specify behavior, not just
  signature"), not the unit-suite which cannot exercise the kernel
  surface.

---

## Steps completed (9)

Commits map per step; recovered from `git log fdbee68d..HEAD` and
`execution-log.json`.

| Step  | Commit       | Title                                                                                       |
|-------|--------------|---------------------------------------------------------------------------------------------|
| 01-01 | `3acab52c`   | Core types: State, View, Action variant, AnyReconciler/AnyState arms                        |
| 01-02 | `04ba6ca1`   | `BackendDiscoveryBridge` struct + reconcile body + fingerprint pure decision fn             |
| 01-03 | `64799c40`   | Runtime hydration arms (`hydrate_desired` + `hydrate_actual`)                               |
| 01-04 | `8ece1f59`   | Action shim `write_service_backend_row` + boot-time bridge registration                     |
| 01-05 | `154fc9ae` + `f8dccf38` | DST RED scaffolds GREEN: 3 invariants + Slice-1 quality gate (closes #174)       |
| 02-01 | `00f785fa`   | `[dataplane]` config + `DataplaneBootError` + `resolve_iface_ipv4` + `AppState.host_ipv4`   |
| 02-02 | `265ae3ac`   | Boot composition: `EbpfDataplane::new` + XDP attach + Drop RAII + attach-mode fallback emit |
| 02-03 | `18e4c671`   | `EbpfDataplane::probe` Earned-Trust + boot-time invocation + `Probe` wiring                 |
| 02-04 | `fc68beef` + `66935193` + `f3a3f4ad` + … + `6af32443` | Walking-skeleton (S-BDB-01) Tier 3 e2e + S-BDB-19 DST handoff + UI-04/UI-05/F1+UI-06/ADR-0053 cascade (closes #175) |

The roadmap's 9-step plan held shape through 01-* and 02-01..02-03.
Step 02-04 was the integration point — its DES log records a
single step ID but the work cascaded across three upstream-issue
resolutions (UI-04, UI-05, F1/UI-06), ADR-0053 (new BPF program
+ trait method + hydrator extension), one shared-testing crate
introduction, and one ExecDriver API extension. Per the
"checkpoint-vs-split" lesson from `service-vip-allocator`'s evolution
doc, the work landed under one step rather than re-splitting because
the cascading discoveries shared one root context (walking-skeleton
A4 failure) and one quality gate.

---

## Implementation arc (commits, grouped thematically)

**Foundations (steps 01-*, GH #174)** — `3acab52c` → `04ba6ca1`
→ `64799c40` → `8ece1f59` → `154fc9ae` + `f8dccf38`:
core types → reconcile body → hydration arms → action shim + boot
registration → DST RED scaffolds GREEN.

**Production wiring (steps 02-01..02-03, GH #175)** — `00f785fa`
→ `265ae3ac` → `18e4c671`: config + iface resolution → XDP attach
boot composition → Earned-Trust probe.

**Walking-skeleton convergence-chain repairs (step 02-04 cascade)** —
`fc68beef` → `66935193` → `626be7f3` → `f3a3f4ad` → `b8b64f6a` → `3b87b653`:

- `fc68beef` — S-BDB-19 DST handoff invariant.
- `66935193` — UI-04 fix: Service-arm hydrate projects driver +
  resources into Job-shape (closes the "Service workload never reaches
  Running" structural gap that made the entire chain invisible).
- `f3a3f4ad` — UI-05 fix: `Action::EnqueueEvaluation` introduced;
  bridge dual-emits hydrator enqueue; `ServiceMapHydrator` wired at
  production boot (the three concurrent defects from the audit
  collapsed into one commit per single-cut migration).
- `3b87b653` — F1/UI-06 fix: WorkloadLifecycle dual-emits bridge
  enqueue alongside `StartAllocation` / `RestartAllocation` /
  `StopAllocation` / `FinalizeFailed`. Closes the long-lived-workload
  cold-start gap (Pending → Running transition the exit-observer
  enqueue cannot see).

**Shared-fixture extraction + ADR-0053 landing** —
`bd10bf18` → `51512d7c` → `edacb3de` → `cd5b1644` → `ef5caf56` →
`6af32443` → `79253f89`:

- `bd10bf18` — `overdrive-testing` crate. Three-netns topology lifted
  from `overdrive-dataplane` integration tests; consumed by both
  `overdrive-dataplane` (existing tests) and `overdrive-control-plane`
  (walking-skeleton).
- `51512d7c` — `ExecDriver` `netns_path` opt-in (mirrors CNI spec).
- `edacb3de` — ADR-0053 drafted.
- `cd5b1644` — `cgroup_connect4_service` program + `LOCAL_BACKEND_MAP`
  infrastructure.
- `ef5caf56` — Hydrator wired to the cgroup path; closes ADR-0053.
- `6af32443` — Byte-order bug fix (see "Lessons learned" below).
- `79253f89` — `development.md` rule added: `bpf_sock_addr.user_port`
  low-16-NBO layout. Structural defense against the byte-order class.

**Phase 16 review actions + Phase 17 mutation closures + D11 runtime
validator** — `9b1aafe4` → `369e0ab7` → `9a68216d` → `cc67038a` →
`f8118fac` → `af5243c5` → `92bbe057` → `a9bcd288` → `6d62afe6`:
trait contract docstrings, explicit match arms, detach symmetry docs,
backend-address rejection, two mutation exclusions, three
mutation-gap-closing test additions, and the D11 runtime invariant at
the action_shim dispatch boundary (see "Key architectural decisions"
→ "Reconcile-output invariant at the action_shim boundary" below).

---

## Key architectural decisions

### ADR-0052 — `BackendDiscoveryBridge` reconciler

Sets the bridge / hydrator split this feature implements: bridge
projects desired endpoints into `ServiceBackendRow`s (intent →
observation); hydrator translates those rows into kernel-side BPF
map state (observation → side effect). The shape pre-existed; this
feature implemented it.

### ADR-0053 — Same-host backend delivery via `cgroup_sock_addr`

Drafted mid-feature once the walking-skeleton TCP RCA established
that XDP alone is structurally insufficient for same-host LB on a
Phase 1 shared-netns deployment. Cilium parity (Cilium runs the same
program on the same hook for the same reason). Adds:

- `cgroup_connect4_service` program on the `cgroup/connect4` hook.
- `LOCAL_BACKEND_MAP` typed map + `register_local_backend` trait
  method.
- Hydrator Local-vs-Remote classification.

XDP programs unchanged — same-host paths short-circuit at connect time;
remote paths continue through ingress XDP unchanged. The path lights
up Phase 1 single-node single-host LB without paying the per-workload
netns cost (Phase 2+ scope per `feedback_phase1_single_node_scope.md`).

Status flipped to **Accepted** as part of this finalize (the production
code shipped under it).

### Action::EnqueueEvaluation cross-reconciler handoff pattern (UI-05 + F1/UI-06)

Reconcilers emit `Action::EnqueueEvaluation { reconciler, target }`
alongside any side-effecting action whose downstream consumer must
tick on the new state. Pattern surfaced first in UI-05 (bridge →
hydrator) and immediately generalised in F1/UI-06 (WorkloadLifecycle →
bridge). The cross-reconciler dependency lives at the **producer's
emission site**, not in the action-shim — the same rationale documented
at `reconciler.rs:760-772`: action-shim-implicit triggers would couple
the shim to reconciler-pair-specific knowledge.

Production rule that fell out: every reconciler that mutates a row
another reconciler reads must dual-emit the EnqueueEvaluation. The
audit table in `audit-reconciler-handoff-topology.md` (now promoted
to `docs/architecture/.../`) is the systematic discharge of this
rule across every Action variant.

### Service-arm hydrate projects into Job-shape (UI-04)

`read_job`'s `WorkloadIntent::Service(svc)` arm now constructs a
kind-agnostic `Job { id, replicas, resources, driver }` from
`ServiceV1`'s field-for-field-equivalent envelope. The reconciler's
`Some(job)` arm consumes this projection unchanged; `desired.workload_kind`
flows separately to mark emitted actions as `kind: Service`. Lossless
projection from the reconciler's perspective; keeps the kind-agnostic
reconciler invariant intact.

### Reconcile-output invariant at the action_shim boundary (D11, `6d62afe6`)

Phase 16 review surfaced D11: a reconciler could in principle return
two `WriteServiceBackendRow` Actions targeting the same VIP in one
tick, with conflicting backend sets. The reviewer's first proposal —
encode the constraint inside the Action enum (sum-type-interior) —
was structurally insufficient. A sum-type guard prevents *intra-Action*
conflict (unlikely bug shape: one Action variant violating itself
internally); it does not prevent *inter-Action* conflict (the real
bug class: two well-formed Actions whose joint effect is contradictory).

The correct defense lives one layer up. `validate_reconcile_output` at
`crates/overdrive-control-plane/src/action_shim/validate.rs` runs on
every reconcile return at the action_shim dispatch boundary
(`reconciler_runtime.rs:993-1033`), asserting "no two write-Actions in
this tick's return target the same service VIP." Fail-safe semantics
on violation: the view still persists (per the runtime fsync-then-memory
ordering), dispatch is skipped this tick, a structured
`reconciler.output.invariant_violation` tracing event fires for operator
paging, and the next tick re-runs the reconciler against fresh
desired/actual state. The validator generalises — it is the home for
future "this set of actions must satisfy property X" invariants the
reconciler trait surface cannot express in types alone.

> **Scope clarification (added 2026-06-03).** D11 is about
> **same-class** write conflicts only: two `WriteServiceBackendRow`
> Actions (observation-row writes) targeting one VIP with conflicting
> backend sets — a genuine last-writer-wins overwrite of one
> observation slot. The invariant D11 establishes is "no two writes to
> the **same map slot** in one tick." Phrased here as "the same service
> VIP" because at the time of writing the only write surface in play was
> the single-keyed observation row.
>
> D11 does **NOT** authorise a *cross-route* conflict rule. A later
> artifact — `validate_reconcile_output` in
> `crates/overdrive-control-plane/src/action_shim/validate.rs` —
> generalised D11 into a "cross-route on the same VIP" rejection
> (its "Conflict class 2"): it rejects a tick that emits BOTH an XDP
> `DataplaneUpdateService` (SERVICE_MAP write) AND a cgroup
> `RegisterLocalBackend` (LOCAL_BACKEND_MAP write) for one VIP. **That
> generalisation is wrong** — it contradicts ADR-0053 Decisions 2/4/5,
> where the XDP-for-remote + cgroup-for-local dual-path on one VIP is
> the *intended* shape (two disjoint kernel maps, two hooks, disjoint
> local-XOR-remote backend sets, `cgroup_connect4` rewriting before the
> kernel routes the SYN so there is no precedence race). The correct
> conflict granularity is `(route, key-tuple)` — XDP keyed on
> `(vip, port, proto)`, cgroup keyed on `(vip, vip_port, proto)` —
> never the shared parent VIP. See ADR-0053 revision 2026-06-03
> ("dispatch-boundary conflict granularity is `(route, key-tuple)`")
> and the evidence base at
> `docs/research/reconcilers/dispatch-boundary-validation-and-attempt-budget-backoff.md`
> (Kubernetes Server-Side Apply field-manager granularity + Cilium
> socket-LB ⊥ XDP datapath). The next reader must not re-derive the
> over-broad invariant from D11. The validator's code + citation fix is
> a separate `/nw-deliver`.

### Phase 1 boundary clarifications (load-bearing for ADR-0053)

The TCP RCA forced the Phase 1 boundaries to be re-stated as
load-bearing constraints rather than incidental defaults:

- **Workloads share the host netns** (per
  `feedback_phase1_single_node_scope.md`); per-workload netns is Phase 2+.
- **XDP is for remote-only delivery**; same-host delivery requires
  `cgroup_sock_addr` (the kernel does not deliver loopback packets to
  XDP ingress regardless of route plumbing).
- **The walking-skeleton's three-netns topology is a test fixture**,
  not a production shape. Real Phase 1 deployments run the test
  process, control-plane, and workloads in one netns; the test creates
  separate netns *only* to exercise the XDP path under a controlled
  isolation that production does not give it.

### `bpf_sock_addr.user_port` low-16-NBO layout rule (`79253f89`)

Added to `development.md` after a real bug (commit `6af32443` fix)
where the crafter wrote `u32::from_be(ctx.user_port) as u16` — the
inverse cast that silently returns 0 because `from_be` byte-swaps
all four bytes and the `as u16` truncation then takes what was
formerly the high half. No Tier 2 backstop exists for this:
`BPF_PROG_TEST_RUN` returns `ENOTSUPP` for `cgroup_sock_addr` on
kernel ≤ 6.8 (`cg_sock_addr_verifier_ops.test_run` is null in
mainline). The rule IS the structural defense.

---

## Issues encountered during DELIVER

Five upstream issues surfaced and resolved during DELIVER. Sources:
`docs/feature/backend-discovery-bridge-service-reachability/deliver/upstream-issues.md`.

- **UI-01** (ACCEPTED) — `Backend` field shape in architecture.md
  § 4.2 was a pre-typed draft. Production reality: typed `SpiffeId` +
  `SocketAddr`. Implementation used production shape; architecture.md
  noted as needing post-feature amendment (documentation hygiene).
- **UI-02** (ACCEPTED) — `fingerprint` pure fn already exists in
  `overdrive_core::dataplane::fingerprint`. The architecture-mandated
  module placement was honored as a thin re-export; no algorithm
  duplication.
- **UI-03** (RESOLVED, commit `516eee0d`) — `Instant::now()` in test
  helper tripped dst-lint. Fixed by extending the dst-lint scanner's
  existing `cfg_test_depth` tracking to the banned-API scanner.
- **UI-04** (RESOLVED, commit `66935193`) — Service-arm convergence
  gap: `read_job` discarded Service driver/resources, blocking the
  entire downstream chain. See "Key architectural decisions" above.
- **UI-05** (RESOLVED, commit `f3a3f4ad`) — Bridge → hydrator handoff
  missing in production (three concurrent defects: hydrator never
  registered, no re-enqueue mechanism, DST passed spuriously).
  `Action::EnqueueEvaluation` introduced; landed in a single commit
  per single-cut migration discipline.
- **F1 / UI-06** (RESOLVED, commit `3b87b653`) — surfaced during the
  post-UI-04+UI-05 reconciler-handoff audit. WorkloadLifecycle →
  bridge handoff missing for Pending→Running transitions. Fixed by
  dual-emit on `StartAllocation` / `RestartAllocation` /
  `StopAllocation` / `FinalizeFailed`.

---

## Lessons learned

**Architectural research saved a wrong-direction implementation arc.**
Before ADR-0053 landed, an early hypothesis was a "single shared
netns + correct route plumbing" topology. Cilium's documentation deep-
dive proved this structurally impossible: the kernel does not deliver
loopback packets to XDP ingress, period. The reading detour cost
hours; building toward the impossible shape would have cost days. The
RCA's "compare populations" probe (working `reverse_nat_e2e` tests vs
failing walking-skeleton, only fixture-topology different) was the
move that anchored the investigation — but the eventual ADR-0053 fix
required understanding *why* a topology fix alone could not work, and
that understanding lived in Cilium's prior art.

**Tier 2 mutation testing is not achievable for `cgroup_sock_addr`
programs (kernel limitation).** `BPF_PROG_TEST_RUN` for
`cgroup_sock_addr` returns `ENOTSUPP` on kernel ≤ 6.8 —
`cg_sock_addr_verifier_ops.test_run` is null in mainline. The
PKTGEN/SETUP/CHECK triptych cannot exercise a synthetic `bpf_sock_addr`
ctx. Tier 3 walking-skeleton + `development.md` rule are the structural
defense; regressions surface only at Tier 3 as "connection hangs" or
"lookup miss" with no kernel-side drop trace. The rule lives in
`development.md` § "`bpf_sock_addr.user_port` — low-16-NBO in a u32".

**Crafter byte-order bug caught at Tier 3 only.** Commit `6af32443`
fixed `u32::from_be(ctx.user_port) as u16` (which silently returns 0
on every call). The Tier 2 gap above means the bug landed in `cd5b1644`
and reached Tier 3 before surfacing as "TCP connections hang" with no
kernel signal. The fix shipped with the `development.md` rule in the
same arc (`79253f89`) so the next contributor finds the prevention
language before writing the same expression.

**Sequential reconciler-handoff topology gaps require a topology audit,
not a per-symptom fix.** UI-04 surfaced (Service-arm hydrate gap); the
fix unblocked UI-05 (bridge → hydrator gap); the fix for UI-05 unblocked
F1/UI-06 (WorkloadLifecycle → bridge gap). Each fix revealed the next.
Rather than continuing the symptom-driven cycle, the
`audit-reconciler-handoff-topology.md` document enumerated *every*
producer → consumer edge in the broker-dispatch graph and pinned which
ones had wake mechanisms. The audit predicted F2 (no fourth gap exists)
correctly; the systematic discharge let F1 land in a single combined
commit with confidence. Promoted to permanent reference under
`docs/architecture/.../`.

**Step 02-04 deliberately did not split.** Per the `service-vip-allocator`
evolution doc, mid-step splits are sometimes cheaper than checkpoint+resume.
For this feature the reverse held: the UI-04, UI-05, F1/UI-06, and
ADR-0053 work all shared one root context (the walking-skeleton A4
failure) and one quality gate (TCP echo through the VIP). Splitting
would have produced four separate green-bar commits for work whose
correctness is only observable jointly. The DES log records one step
ID; the commit history records the actual arc.

---

## Migrated artifacts

| Artifact                                                                                                | Permanent location                                                                                                                                              |
|---------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------|
| ADR-0053 — Same-host backend delivery via `cgroup_sock_addr` (status flipped Proposed → Accepted)       | `docs/product/architecture/adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md` (already permanent; not migrated)                                       |
| Architecture document (design surface, C4 L2/L3, public surfaces, hydration arms)                       | `docs/architecture/backend-discovery-bridge-service-reachability/architecture.md`                                                                               |
| Test-scenarios catalogue (S-BDB-01..S-BDB-20)                                                           | `docs/scenarios/backend-discovery-bridge-service-reachability/test-scenarios.md`                                                                                |
| Walking-skeleton spec + demo script                                                                     | `docs/scenarios/backend-discovery-bridge-service-reachability/walking-skeleton.md`                                                                              |
| Reconciler-handoff topology audit (load-bearing reference for future cross-reconciler handoff design)   | `docs/architecture/backend-discovery-bridge-service-reachability/audit-reconciler-handoff-topology.md`                                                          |
| RCA — Service-arm convergence gap (failure-mode reference)                                              | `docs/architecture/backend-discovery-bridge-service-reachability/rca-service-arm-convergence.md`                                                                |
| RCA — Walking-skeleton TCP round-trip (failure-mode reference; explains why ADR-0053 was needed)        | `docs/architecture/backend-discovery-bridge-service-reachability/rca-walking-skeleton-tcp-roundtrip.md`                                                         |
| Wave artifacts (preserved as wave-matrix SSOT)                                                          | `docs/feature/backend-discovery-bridge-service-reachability/`                                                                                                   |

---

## Links

- ADR-0052 — `BackendDiscoveryBridge` reconciler (pre-feature):
  `docs/product/architecture/adr-0052-backend-discovery-bridge.md`
- ADR-0053 — Same-host backend delivery (mid-feature):
  `docs/product/architecture/adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md`
- ADR-0035 / ADR-0036 — Reconciler trait shape + runtime ViewStore
  ownership (load-bearing for bridge persistence model).
- ADR-0040 / ADR-0041 / ADR-0042 — three-map split + weighted Maglev
  + `ServiceMapHydrator` (the kernel-side LB stack this feature plumbs
  intent into).
- ADR-0045 — `bpf_redirect_neigh` (remote-delivery datapath).
- ADR-0049 — `ServiceVipAllocator` (the VIP this bridge reads).
- GH #174 — BackendDiscoveryBridge reconciler (closed).
- GH #175 — `EbpfDataplane` production boot composition (closed).
