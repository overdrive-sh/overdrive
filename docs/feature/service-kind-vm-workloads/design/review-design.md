# DESIGN Wave Review — service-kind-vm-workloads

**Reviewer**: `nw-solution-architect-reviewer` stance

**Date**: 2026-09-06

**Iteration**: 1

**Verdict**: **CONDITIONALLY APPROVED — core architecture is sound; three handoff-contract corrections are required before DISTILL / DELIVER**

**Scope**: GH #257 and #280, `feature-delta.md`, the three active slice briefs,
`design/wave-decisions.md`, ADR-0090, ADR-0091, the GH #257 additions to
`docs/product/architecture/brief.md` and `c4-diagrams.md`, relevant accepted
ADRs, and the current production parser, aggregate, action-shim, driver,
ProbeRunner, and reconciler paths.

## Verdict

The selected mechanism is architecturally viable and appropriately small:

- `ProbeRunner` receives the existing `AllocationSpec` at registration and
  privately projects only the effective HTTP/TCP destination.
- `VmDriver` reuses the existing Running/Stable/terminal hooks and receives the
  already-trusted production runner as a required dependency.
- parser `ServiceSpec` advances through the existing rkyv-envelope discipline,
  while wire, persisted intent, allocation, and describe reuse their existing
  driver unions.
- arbitrary in-guest Exec remains outside GH #257 and is anchored to GH #280;
  no protocol, guest supervisor, containment mechanism, or new lifecycle state
  leaks into this design.
- `Vm + workload_addr = None` is correctly treated as representable but
  unreachable. The production fresh-start and restart paths both provision and
  inject the guest address before VM start and call `on_alloc_running` only
  after the Running write. No fallback, probe-Fail row, synthetic lifecycle
  transition, public error/state, or test is warranted for that state.

Three delivery-facing contract defects remain. They do not require a new
architecture mechanism, but they must be corrected before an implementer is
asked to follow the design exactly.

### Blocking-issue count

| Severity | Issues |
|---|---:|
| Critical | 0 |
| High | 3 |
| Medium | 0 |
| Low | 0 |

## Findings

### High: lifecycle tables assign the restart action to the wrong owner

**Dimension**: Lifecycle-gate ownership / Accepted-ADR consistency / Handoff correctness

**Location**: `docs/feature/service-kind-vm-workloads/design/wave-decisions.md:234-249`; `docs/feature/service-kind-vm-workloads/feature-delta.md:488-505`; `docs/product/architecture/brief.md:10443-10457`; contrast ADR-0087 D1-D4 and `crates/overdrive-reconcilers/src/service_lifecycle.rs:984-1064`

All three current lifecycle tables name a “Restart request” or “Restart action”
and assign it to the `ServiceLifecycle` liveness branch plus the existing single
restart authority. That is not the accepted architecture. ADR-0087 explicitly
demoted `ServiceLifecycle` to a liveness detector that emits
`StopAllocation { terminal: Stopped { by: LivenessProbe } }`, reads no restart
budget, and makes no restart-versus-finalize decision. `WorkloadLifecycle` is
the sole owner that observes the liveness-terminated row and emits any
`RestartAllocation` under its unified budget. The production implementation
states and enforces that split at `service_lifecycle.rs:984-992` and
`:1057-1064`.

This matters even though GH #257 changes neither reconciler: lifecycle ownership
tables are the executable handoff. Their current wording can direct a crafter to
reintroduce the exact split restart authority ADR-0087 removed.

**Required correction**: make every active lifecycle table distinguish the two
existing signals and owners: liveness threshold detection/termination belongs
to `ServiceLifecycle`; restart decision/action and budget belong solely to
`WorkloadLifecycle`. Keep the feature-level statement that GH #257 changes
neither mechanism.

### High: “first VM Exec probe” has no pinned cross-role ordering

**Dimension**: Exact operator/API contract / Cross-boundary equivalence / Testability

**Location**: `docs/product/architecture/adr-0091-service-parser-driver-union-and-vm-exec-exclusion.md:67-78`, `:115-126`; `docs/feature/service-kind-vm-workloads/design/wave-decisions.md:183-191`; `docs/feature/service-kind-vm-workloads/feature-delta.md:384-389`; `docs/product/architecture/brief.md:10437-10441`

ADR-0091 requires both the TOML parser and `ServiceV2::from_submit` to reject
the *first* offending Exec probe and produce equivalent role/index/GH #280
guidance, but it does not define which role wins when more than one of startup,
readiness, and liveness contains Exec. `ProbeDescriptor.idx` is deliberately
per-role under ADR-0080, so `(role, idx)` cannot itself establish a total order.
The current parser happens to construct startup, readiness, then liveness, and
the aggregate currently validates its three vectors in that order, but an
incidental source sequence is not an operator-visible contract.

Without a pinned role precedence, two exact implementations can both satisfy
the prose while returning different errors, and parser/API equivalence tests
must invent the missing rule.

**Required correction**: pin the deterministic cross-role selection rule in
ADR-0091 and repeat it in the evidence obligation. Also pin how the chosen
`(role, idx)` maps onto the existing `ParseError::Field` and
`AggregateError::Validation` localization fields, without adding a new public
error variant.

### High: Reuse Analysis omits a component the decomposition requires DELIVER to change

**Dimension**: Effect Isolation / Reuse Analysis completeness / Handoff clarity

**Location**: `docs/feature/service-kind-vm-workloads/feature-delta.md:414-425`, `:474-486`; `docs/feature/service-kind-vm-workloads/design/wave-decisions.md:255-268`; `docs/product/architecture/brief.md:10426-10435`, `:10459-10470`

The component decomposition explicitly includes the two existing Service CLI
deploy lanes as EXTEND, because they currently collapse parser Service input to
Exec and must forward the selected driver union. That component is absent from
both Reuse Analysis tables and from the brief's component-boundary table. The
feature-delta Reuse Analysis also omits the production composition root, even
though the decomposition and ADR-0090 require it to retain and pass the runner;
the shorter wave-decisions table includes composition but still omits the CLI.

The review contract requires every changed component to carry an explicit
effect-isolation shape. “Every overlap is reused or extended” is not established
while delivery-required components have no row.

**Required correction**: make the Reuse Analysis component set match the
decomposition in every active handoff artifact. Add the missing CLI deploy-lane
and production-composition entries where absent, with each component's declared
`pure-function` or `bounded-change` contract; for bounded change, state the
effect universe, exact delta, and assertion strategy. Do not create a new
component to close this documentation gap.

## Checks Passed

- **Production reachability premise**: PASS. VM network provisioning calls
  `inject_workload_network`, whose VM arm writes `Some(tap.guest_addr)`
  (`action_shim/mod.rs:1218-1242`). Fresh start provisions at `:1825-1830` and
  registers at `:2203`; restart provisions at `:2294-2299` and registers at
  `:2670`. Provision failure exits before VM start/Running. The design correctly
  excludes `Vm + None` behavior and tests.
- **Exact API surface**: PASS for the implemented mechanism. ADR-0090 pins the
  replacement `ProbeRunner::start_alloc(&AllocationSpec)` signature and exact
  `VmDriver::new` parameter order. ADR-0091 pins the complete `ServiceSpecV3`
  public shape and reuses existing error variants and driver unions. No parallel
  method, type, variant, or optional builder is invented.
- **Scope integrity**: PASS. GH #257, the active feature delta, ADRs, brief, and
  C4 consistently keep HTTP/TCP in scope and arbitrary guest Exec under GH
  #280. Historical spike artifacts are clearly feasibility evidence, not active
  production design.
- **Lifecycle non-gating**: PASS apart from the owner-label finding above.
  Running remains the driver/Beacon result; probes begin after its write.
  Startup owns Stable, readiness owns backend eligibility, and a closed guest
  port cannot revoke Running.
- **Intent/runtime separation**: PASS. Declared `ProbeDescriptor` values remain
  persisted intent; projected targets are private task inputs and create no
  second address registry or observation schema.
- **Schema evolution**: PASS. V1/V2 stay frozen, V3 is appended, older payloads
  up-convert as Exec, and golden-byte obligations are explicit.
- **C4 fitness**: PASS. L1 and L2 show the affected system and logical-container
  boundaries; omission of a new L3 is justified because no complex subsystem or
  deployment unit is introduced.
- **Technology and reuse fit**: PASS in substance. The design stays inside the
  existing Rust modular monolith, ports/adapters, Tokio, Hyper, rkyv, redb, and
  Cloud Hypervisor stack with no new dependency, daemon, transport, or storage.
- **Roadmap review**: N/A. No roadmap exists yet; the DESIGN package is being
  reviewed before DISTILL/roadmap construction.

## Approval Status

`conditionally_approved`

The mechanism and boundaries do not need redesign. Correct the three active
handoff contracts above, then perform an iteration-2 architecture review before
advancing to DISTILL or DELIVER. In particular, remediation must preserve the
production-precondition ruling: do not add any behavior, public surface, or
synthetic test for `Vm + workload_addr = None`.

---

## Iteration 2 — Remediation Review

**Reviewer**: `nw-solution-architect-reviewer` stance

**Date**: 2026-09-06

**Verdict**: **APPROVED — all three iteration-1 High findings are resolved; no critical or high issue remains**

### Prior-finding dispositions

| Iteration-1 finding | Disposition | Verification |
|---|---|---|
| Lifecycle tables assigned restart action to the wrong owner | **RESOLVED** | `design/wave-decisions.md:91-98`, `:245-268`; `feature-delta.md:390-396`, `:496-516`; `brief.md:10446-10461`; ADR-0090 `:86-101`; ADR-0091 `:96-101`; and C4 `:1286-1306` now consistently separate `ServiceLifecycle` liveness detection/`StopAllocation` from `WorkloadLifecycle` sole restart-versus-finalize authority. This matches ADR-0087 D1-D4 and production `service_lifecycle.rs:984-1064`. |
| “First VM Exec probe” lacked cross-role ordering and exact localization | **RESOLVED** | ADR-0091 `:67-94` pins Startup -> Readiness -> Liveness, then the lowest vector position; caller-supplied `ProbeDescriptor.idx` is not trusted. It maps exactly to existing parser sections `"[[health_check.startup]]"`, `"[[health_check.readiness]]"`, `"[[health_check.liveness]]"` and aggregate fields `"startup_probes"`, `"readiness_probes"`, `"liveness_probes"`, with exact existing message prefixes. Evidence obligations at `:132-147` cover multi-role permutations and disagreeing API-supplied indices. `ParseError::Field` and `AggregateError::Validation` remain unchanged; no public type, variant, or field was added. |
| Reuse Analysis omitted CLI deploy lanes and production composition | **RESOLVED** | `design/wave-decisions.md:275-290`, `feature-delta.md:479-494`, and `brief.md:10463-10475` now include both `deploy_service`/`deploy_streaming_service` and `run_server`/`compose_production_driver`/`compose_vm_driver`. Each entry declares a bounded universe, exact delta, and assertion strategy. The named functions exist at the current source boundaries. No new component was introduced. |

### Iteration-2 revalidation

- **Lifecycle gate ownership**: PASS. Allocation Running remains the
  action-shim/driver Beacon outcome and precedes probe registration. Startup
  alone gates Stable; readiness alone controls backend health;
  `ServiceLifecycle` only detects and terminates on liveness; and
  `WorkloadLifecycle` alone decides restart/finalization under its unified
  budget. GH #257 adds, removes, and moves no gate.
- **HTTP/TCP-only scope**: PASS. Every active feature artifact keeps
  host-originated HTTP/TCP probes in GH #257 and leaves guest Exec protocol,
  supervision, reconnect, and containment under GH #280. VM Exec admission is
  rejected locally and authoritatively before persistence; Exec-backed Service
  probes are unchanged.
- **Production precondition**: PASS. The fresh-start path provisions at
  `action_shim/mod.rs:1825-1830` and registers after the Running write at
  `:2203`; restart provisions at `:2294-2299` and registers at `:2670`. The VM
  injection arm writes `Some(tap.guest_addr)` at `:1237`. The remediation
  correctly retains no fallback, failed probe row, allocation transition,
  public error/state, silent skip, or synthetic test for unreachable
  `Vm + workload_addr = None`.
- **Exact API shape**: PASS. `ProbeRunner::start_alloc(&AllocationSpec)`, the
  exact mandatory `VmDriver::new` dependency order, `ServiceSpecV3`, existing
  driver unions, and existing error variants remain the complete public
  surface. No remediation invented an API.
- **Effect isolation**: PASS. Every component in the remediated decomposition
  appears in the authoritative Reuse Analysis with a declared pure or bounded
  contract and an assertion boundary.
- **Schema evolution and intent/runtime separation**: PASS. V1/V2 remain frozen,
  V3 is appended, prior values up-convert as Exec, and effective destinations
  remain private task inputs rather than rewritten intent or observations.
- **Roadmap review**: N/A. No roadmap exists yet; DESIGN review precedes
  DISTILL/roadmap construction.

### Non-blocking cleanup

#### Medium: historical service-health quality scenario still names the superseded direct-restart action

**Dimension**: Cross-artifact consistency

**Location**: `docs/product/architecture/brief.md:5892-5903`, especially ASR-SHCP-03 at `:5898`

The older Phase-1 Service Health-Check Probes scenario still expects
`Action::RestartAllocation { reason: LivenessExhausted { .. } }` directly after
the liveness threshold. ADR-0087, the brief's later sole-authority section at
`:1817-1844`, the remediated GH #257 section, and production code supersede that
shape with liveness `StopAllocation` followed by a `WorkloadLifecycle` restart.
The current GH #257 handoff is unambiguous, so this is not an approval blocker,
but the stale scenario can misdirect a future test reader.

**Recommendation**: reconcile ASR-SHCP-03 with ADR-0087's accepted two-owner
trajectory and remove the deleted direct-restart reason shape during normal
architecture-SSOT cleanup.

### Final issue count

| Severity | Issues |
|---|---:|
| Critical | 0 |
| High | 0 |
| Medium | 1 |
| Low | 0 |

## Final Approval Status

`approved`

The remediated architecture is approved for DISTILL. The non-blocking historical
scenario cleanup does not require another design iteration and must not be used
to add lifecycle behavior or expand GH #257 beyond the accepted HTTP/TCP-only
boundary.
