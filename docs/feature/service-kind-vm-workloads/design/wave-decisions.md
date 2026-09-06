# DESIGN wave decisions — `service-kind-vm-workloads`

**Wave:** DESIGN | **Scope:** APPLICATION / components | **Mode:** guided
| **Architect:** Morgan | **Date:** 2026-09-06

**Active feature boundary:** GH #257 adds Service-kind VM workloads with
host-originated HTTP/TCP probes. VM Exec probes are rejected before intent
commit and remain deferred to GH #280. No VM Exec control protocol, codec,
persistent guest supervisor, session/reconnect protocol, or guest process
containment mechanism is part of this design.

## Accepted decisions

### ACD-1 — Resolve the effective HTTP/TCP destination once at probe registration

**Decision:** `ProbeRunner` owns effective-target projection at the existing
per-allocation registration boundary. The existing
`Driver::on_alloc_running(&AllocationSpec)` hook supplies the allocation facts
already materialized by the action shim. `ProbeRunner::start_alloc` changes its
existing input to receive that `AllocationSpec` rather than adding a second
registration method or a global allocation-address lookup:

```rust
pub fn start_alloc(&self, spec: &AllocationSpec) -> CancellationToken;
```

Before spawning any per-descriptor task, the runner projects an immutable
effective HTTP/TCP destination by this production-input rule:

| Declared target | Driver | Effective target |
|---|---|---|
| Explicit host | Any | The declared host, verbatim |
| Omitted HTTP host or `0.0.0.0` wildcard | VM | The same allocation's provisioned `workload_addr` |
| Omitted HTTP host or `0.0.0.0` wildcard | Exec/process | `127.0.0.1`, preserving current behavior |

TCP currently represents an omitted host as `0.0.0.0`; HTTP retains omission
as `None`. Both forms enter the same projection rule. The declared
`ProbeDescriptor` remains intent and is not rewritten or persisted with the
effective address.

For the VM row, `Some(workload_addr)` is a precondition of this registration
boundary, not a probe outcome to classify. The production action shim always
provisions the VM network and injects `Some(tap.guest_addr)` before
`Driver::start`; provisioning failure prevents `Running`, and
`on_alloc_running` is invoked only after the successful Running write. The
design therefore adds no `Vm + None` fallback, scheduled failure result,
allocation transition, public error/state, or test case. Although
`AllocationSpec.workload_addr` is optional because the shared type also serves
non-VM allocations, `Vm + None` has no producer in the real `serve` + `deploy`
path. If that premise is later disproved through the production owner path, it
is a separate DESIGN gap rather than ordinary workload unhealth.

The one production `Arc<ProbeRunner>` that has already passed its Earned-Trust
boot probe is shared by both production drivers. `ExecDriver` retains its
existing hooks. The production boot path retains the second value already
returned by `compose_production_driver`, passes it into the existing private
`compose_vm_driver` function, and that function passes it as a new required
constructor dependency immediately before `layout`:

```rust
impl VmDriver {
    pub fn new(
        vmm: Arc<dyn Vmm>,
        clock: Arc<dyn Clock>,
        fs: Arc<dyn CgroupFs>,
        cgroup_accounting: Arc<dyn CgroupAccounting>,
        probe_runner: Arc<ProbeRunner>,
        layout: VmHostLayout,
    ) -> Self;
}
```

This extends the existing constructor instead of adding an optional builder
that tests could forget. `VmDriver` then implements the same existing lifecycle
hooks:

- `on_alloc_running` registers from the full `AllocationSpec`;
- `on_alloc_stable` stops only Startup tasks;
- `on_alloc_terminal` stops the allocation supervisor.

No new `Driver` trait method, probe adapter trait, lookup registry, persisted
row, CLI/API endpoint, or lifecycle state is introduced.

**Why this boundary:** the action shim has already provisioned the VM network
and populated `AllocationSpec.workload_addr` before it commits `Running` and
calls `on_alloc_running`. `ProbeRunner` already owns mechanic execution and the
current loopback translation in `probe_tick`; resolving at registration keeps
the driver from rewriting probe intent and avoids repeating an immutable choice
on every tick.

**Lifecycle ownership:** this projection is not a new gate. It can affect only
the subsequent `ProbeResultRow` produced by an HTTP/TCP attempt. Allocation
`Running` remains owned by the VM driver/Beacon contract; Startup retains
ownership of `Stable`, and Readiness retains ownership of `Backend.healthy`.
`ServiceLifecycle` detects a liveness threshold and emits only the existing
liveness termination; `WorkloadLifecycle` alone decides restart versus
finalization under the unified budget. A Running VM whose guest port is closed
therefore records a failed probe but remains Running.

**Restart:** `RestartAllocation` reuses the allocation id and re-provisions the
same slot-derived VM address. Re-registration remains idempotent at the existing
supervisor boundary; it does not create a second task set or an address
registry. Terminal cancellation remains the only full-supervisor teardown.

**Alternatives rejected:**

- Resolve on every probe tick: repeats an immutable decision and broadens the
  long-lived task context without adding correctness.
- Rewrite descriptors during reconciler hydration: the VM address is not
  materialized there, and doing so would mix declared intent with an effective
  runtime target.
- Read a global allocation-address registry on every tick: introduces new
  mutable state, lookup failure modes, and termination races although the
  required value is already present at the Running handoff.
- Add a VM-specific probe runner or new driver hook: duplicates the delivered
  scheduler/lifecycle machinery and creates public surface the feature does not
  need.

**Evidence obligations carried forward:** pure target-projection matrix over
supported production inputs (explicit targets, VM default with a provisioned
address, and Exec/process default);
component tests proving the exact TCP/HTTP adapter destination and explicit-host
preservation; existing lifecycle tests repeated through `VmDriver`; a seeded
simulation invariant that terminal state beats any late probe success; and a
built-default-feature native-metal expectation through the production
TAP/netns path.

### ACD-2 — Reuse the existing driver union end to end for Service ingress

**Decision:** the parser-side Service shape stops carrying an Exec-only field
and reuses the parser's existing `DriverInput::{Exec, Vm}` union. This is a
versioned replacement of the parser payload, not a new Service kind or wire
type:

```rust
pub type ServiceSpec = ServiceSpecV3;
pub type ServiceSpecLatest = ServiceSpecV3;

pub enum ServiceSpecEnvelope {
    V1(ServiceSpecV1),
    V2(ServiceSpecV2),
    V3(ServiceSpecV3),
}

pub struct ServiceSpecV3 {
    pub id: String,
    pub replicas: u32,
    pub driver: DriverInput,       // parser-side workload_spec::DriverInput
    pub resources: ResourcesInput, // parser-side workload_spec::ResourcesInput
    pub listeners: Vec<Listener>,
    pub startup_probes: Vec<ProbeDescriptor>,
    pub readiness_probes: Vec<ProbeDescriptor>,
    pub liveness_probes: Vec<ProbeDescriptor>,
}
```

`ServiceSpecV1` and `ServiceSpecV2` stay byte-frozen. The V2-to-V3 conversion
wraps `v2.exec` as `DriverInput::Exec(v2.exec)` and carries every other
field verbatim; V1 reaches latest through the existing V1-to-V2 conversion and
then V2-to-V3. The envelope appends discriminant `2`, gains a V3 golden fixture,
and leaves the V1/V2 fixtures unchanged.

`SectionPresence::validated` applies the already-existing exactly-one-of
`[exec]`/`[vm]` rule uniformly to Service, Job, and Schedule. The
`VmNotAllowedOnServiceKind` rejection and its now-stale GH #257/#222 guidance
are removed. The Service parse branch selects the declared parser driver and
stores it on `ServiceSpecV3`; the existing `exec_command` accessor remains for
compatibility but delegates to `service.driver.command()`.

Both existing CLI Service deploy lanes convert the parser-side union variant
field-for-field into the already-existing wire-side `DriverInput` and place it
on the already-existing `ServiceSpecInput.driver`. No new command or request
variant is added. The already-existing `ServiceV2::from_submit` wire-to-intent
constructor accepts and validates either driver arm using the same command
rules as `JobV2::from_submit`; the downstream `ServiceV2.driver`,
`WorkloadLifecycle`, and `AllocationSpec.driver` projections already support
both arms and remain unchanged.

The describe response already carries the same wire-side `DriverInput` union.
`ServiceV2::to_describe` removes its stale VM-unreachable arm and projects both
persisted driver variants field-for-field, matching the existing Job describe
projection. No describe response version or parallel VM renderer is added.

**VM Exec-probe exclusion:** after the Service driver and all three role lists
are known, both ingress boundaries scan the role vectors in the fixed order
**Startup -> Readiness -> Liveness**, then select the lowest zero-based vector
position within that role. This is the operator-visible meaning of “first”;
`ProbeDescriptor.idx` does not establish cross-role order and the API-supplied
value is not trusted.

The selected location maps onto the existing error surfaces exactly as follows:

| Selected role | Parser `ParseError::Field.section` | Admission `AggregateError::Validation.field` |
|---|---|---|
| Startup | `"[[health_check.startup]]"` | `"startup_probes"` |
| Readiness | `"[[health_check.readiness]]"` | `"readiness_probes"` |
| Liveness | `"[[health_check.liveness]]"` | `"liveness_probes"` |

The parser message starts `entry [{position}]:`; the admission message starts
`[{position}]:`. Both then use the same diagnostic: `exec probes are not
supported for VM Service workloads; use HTTP or TCP; optional VM Exec probes
are tracked by GH #280`. This reuses `ParseError::Field { section, message }`
and `AggregateError::Validation { field, message }` verbatim—no new public
error variant or field is introduced. Parser errors required to construct a
descriptor still take precedence; the deterministic scan applies to the
successfully constructed role vectors. Both cross-field checks run before a
`ServiceV2` exists, so the submit handler cannot archive or commit invalid
intent. Exec-backed Service Exec probes remain unchanged.

**Why this boundary:** `ServiceSpecInput` already carries the wire driver union,
`ServiceV2` already persists the intent driver union, and
`WorkloadLifecycle` already projects both drivers. Only the older parser
payload and its two CLI projections are Exec-specific. Widening that stale
edge keeps one validation/conversion path and avoids parallel VM-Service
schema, deploy, or lifecycle components.

**Alternatives rejected:**

- Add `VmServiceSpec` and VM-specific deploy functions: duplicates kind,
  validation, wire dispatch, and CLI behavior around an axis the existing
  driver union already models.
- Accept VM in the parser but reject VM Exec probes only at the server: protects
  persistence, but violates the local parse-time feedback contract and sends a
  request the CLI already knows is invalid.
- Add a new persisted Service or WorkloadIntent version: unnecessary because
  the live intent's driver field is already `WorkloadDriverV2` and already has
  the VM variant.

### ACD-3 — Treat provisioned VM addressing as a registration precondition

**Decision:** no runtime recovery policy is designed for a VM reaching probe
registration without `workload_addr`. The state is representable in the shared
`AllocationSpec` type but is not reachable through production VM composition:
`provision_and_inject_netns` injects the guest address before `Driver::start`,
and the action shim calls `on_alloc_running` only after successful start and
the Running observation write.

This decision prevents an internal control-plane invariant from being
misreported as guest health. In particular, the implementation must not emit a
probe `Fail` row, fall back to host loopback, skip ticks, or drive an allocation
transition for `Vm + None`. No new public type or method is introduced. A
future change may address the state only after a bounded production-path
reproduction proves it reachable.

## Lifecycle Gate Ownership

**Proposed gates: Not applicable.** Target projection and Service-driver
admission reuse existing boundaries; neither adds, removes, nor moves a
lifecycle or health gate.

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| Allocation `Running` | Action shim + selected `Driver`; VM success is the accepted Beacon/driver-start contract | The allocation driver started and its Running observation was committed | Existing VM provisioning, driver start/Beacon, and intercept-install ordering | Service `Stable`; `Backend.healthy`; liveness verdict |
| Service `Stable` | `ServiceLifecycle` startup branch | Every startup probe passed, or startup probing was explicitly disabled | Existing startup observations and deadline policy | Allocation `Running`; readiness eligibility; liveness policy |
| `Backend.healthy` | `ServiceLifecycle` readiness branch | The backend currently meets the declared readiness threshold | Existing readiness observations and counters | Allocation `Running`; Service `Stable`; restart authority |
| Liveness termination | `ServiceLifecycle` liveness detector | The declared liveness threshold was reached; emit only `StopAllocation { terminal: Stopped { by: LivenessProbe } }` | Existing liveness observations and threshold counter | Restart budget/decision; meanings of `Running`, `Stable`, or readiness |
| Restart decision/action | `WorkloadLifecycle` (sole restart authority) | Observe the liveness-terminated allocation row and either emit `RestartAllocation` under the unified budget or finalize failure | Existing `AllocStatusRow.terminal` plus unified restart budget | Probe threshold detection; meanings of `Running`, `Stable`, or readiness |

Boundary obligations for DISTILL/DELIVER:

1. A VM reaches `Running` before its first scheduled probe; a closed guest port
   yields a failed startup observation without revoking or delaying `Running`.
2. Startup success advances only Service `Stable`.
3. Readiness fail/pass changes only backend eligibility on the existing bound.
4. Liveness threshold satisfaction makes `ServiceLifecycle` emit only the
   existing liveness `StopAllocation`; `WorkloadLifecycle` alone decides
   restart versus finalization under the unified budget. No VM-specific policy
   is added.
5. Terminal state wins over any late probe success; no dead backend is restored.
6. An Exec-backed Service preserves its existing loopback-default and lifecycle
   behavior.
7. A VM Service with an Exec probe is rejected before intent commit and names
   GH #280; HTTP/TCP VM Services continue through the same deploy surface.

## Reuse Analysis

| Existing component | Path | Overlap | Decision | Contract shape and assertion |
|---|---|---|---|---|
| `WorkloadSpecInput` + `ServiceSpecEnvelope` | `crates/overdrive-core/src/aggregate/{workload_spec,service_spec}.rs` | TOML discrimination and versioned Service parser payload | EXTEND | Pure parser projection over one TOML document; typed parse properties plus frozen V1/V2 and new V3 golden bytes |
| `ServiceV2::from_submit` / `to_describe` | `crates/overdrive-core/src/aggregate/mod.rs` | Authoritative admission and describe round-trip | EXTEND | Pure-function over one Service payload; cross-field rejection properties and driver-union round-trip |
| Existing `DriverInput` / `WorkloadDriverV2` unions | `crates/overdrive-core/src/{api/submit.rs,aggregate/mod.rs}` | VM/Exec representation already exists downstream | REUSE | Pure tagged-union projection; existing schema fixtures plus both-arm round-trip properties |
| Service CLI deploy lanes (`deploy_service`, `deploy_streaming_service`) | `crates/overdrive-cli/src/commands/deploy.rs` | Both currently collapse parser Service input to Exec | EXTEND | Bounded-change universe: one parsed Service projected to one `ServiceSpecInput` in either existing lane; exact delta: replace the Exec-only projection with the selected existing driver-union arm; assertions: both lanes preserve the same arm field-for-field while retaining their existing HTTP/output behavior |
| `ProbeRunner` | `crates/overdrive-worker/src/probe_runner/mod.rs` | Schedules all three roles and dispatches HTTP/TCP mechanics | EXTEND | Bounded-change: one allocation supervisor and its probe-result rows; destination-capture component tests |
| `VmDriver` | `crates/overdrive-worker/src/vm_driver.rs` | Owns VM lifecycle but does not yet register probes | EXTEND | Bounded-change: the addressed VM allocation's supervisor hooks only; lifecycle hook tests |
| Production composition (`run_server`, `compose_production_driver`, `compose_vm_driver`) | `crates/overdrive-control-plane/src/lib.rs` | Already owns the single trusted `ProbeRunner` and optional VM driver | EXTEND | Bounded-change universe: one server boot and its driver registry; exact delta: retain the runner returned beside `ExecDriver` and pass an `Arc` clone into the optional `VmDriver`; assertion: one Earned-Trust runner services lifecycle hooks from both registered drivers, with capability-absence/refusal behavior unchanged |
| Action-shim VM network injection | `crates/overdrive-control-plane/src/action_shim/mod.rs` | Produces the guest `workload_addr` before `Running` | REUSE | Bounded-change over the one allocation spec and owned network resources; existing provision-before-start evidence plus native-metal H6 |
| `ServiceLifecycle` | `crates/overdrive-reconcilers/src/service_lifecycle.rs` | Already owns startup/readiness plus liveness detection/termination | REUSE | Pure reconciliation over hydrated state/view/tick; existing role invariants repeated with VM-produced observations |
| `WorkloadLifecycle` | `crates/overdrive-reconcilers/src/workload_lifecycle.rs` | Already owns restart-versus-finalize decisions under the unified budget | REUSE | Pure reconciliation over hydrated allocation status/view/tick; ADR-0087 assertions retain liveness-terminated-row restart/finalization and prohibit a second restart authority |

No new component is created.

## Iteration-1 review remediation

- **R1 — lifecycle authority:** production `ServiceLifecycle` remains only the
  liveness detector/terminator; ADR-0087 `WorkloadLifecycle` is named as the
  sole restart-versus-finalize authority in every active lifecycle handoff.
- **R2 — deterministic rejection:** Startup -> Readiness -> Liveness and lowest
  vector position are now contractual, with exact localization through the
  existing parser `section` and aggregate `field` values.
- **R3 — reuse completeness:** both existing Service CLI deploy lanes and the
  production composition root now carry explicit bounded-change universes,
  exact deltas, and assertion strategies.
- **Precondition retained:** production fresh-start and restart paths still
  provision and inject `Some(workload_addr)` before VM start/Running. No
  fallback, failure row, lifecycle transition, public surface, or synthetic
  `Vm + None` test has been added.

## Technology stack

- Rust and the existing modular-monolith, ports-and-adapters boundaries.
- Existing Tokio task supervision, Hyper HTTP probing, TCP prober, rkyv
  versioned envelopes, redb intent/observation adapters, and Cloud Hypervisor
  VM path; no new dependency, daemon, transport, or protocol.

## Changed assumptions

- **Superseded contract:** ADR-0083's `[service] + [vm]` admission rejection was
  correct before GH #222 delivered the guest network/intercept path. It is now
  narrowed to reject only VM Services containing Exec probes; HTTP/TCP probes
  are admitted by GH #257.
- **Superseded default-target assumption:** ADR-0054/0058's loopback default
  remains correct for Exec/process allocations but no longer applies to
  omitted/wildcard VM HTTP/TCP targets, which use the provisioned guest
  `workload_addr`.
- **Rejected speculative assumption:** representability of
  `AllocationSpec { driver: Vm, workload_addr: None }` does not make it a
  production state. The action-shim owner path proves it is absent before probe
  registration, so no behavior is designed around it.

## Open questions

None for the active GH #257 Application/component boundary. In-guest Exec
probe mechanics are independently scoped by GH #280 and do not constrain this
design.
