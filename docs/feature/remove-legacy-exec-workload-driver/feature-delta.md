# Feature Delta — `remove-legacy-exec-workload-driver`

**Feature ID:** `remove-legacy-exec-workload-driver`

**Requirements contract:** [GH #293](https://github.com/overdrive-sh/overdrive/issues/293)

**Scope:** Application / components

**Interaction mode:** Propose

**Status:** **The corrected DESIGN is independently `APPROVED` after overall
review iteration 4. DISTILL has been reconciled to its minimum-evidence
boundary and is awaiting the consolidated review gate. Not DELIVER authority.**
P-293-1 through P-293-6 remain binding.

**Documentation density:** lean, from `~/.nwave/global-config.json`.
The named resolver `scripts/shared/density_config.py` and telemetry helper
`scripts/shared/telemetry.py` were not found in the repository or installed
nWave paths searched for this run. This artifact therefore follows the declared
lean mode without inventing a resolver result or telemetry event.

## Wave: DESIGN / [REF] Prior-wave Consultation

- ✓ `docs/product/architecture/brief.md`
- ✓ relevant architecture decisions: ADR-0022, ADR-0023, ADR-0026,
  ADR-0029, ADR-0030, ADR-0031, ADR-0047, ADR-0054 (both records),
  ADR-0059, ADR-0069, ADR-0071, ADR-0072, ADR-0076, ADR-0081 through
  ADR-0083, ADR-0087 through ADR-0091, ADR-0097 through ADR-0100, and
  the driver-neutral replacement contract in ADR-0105/0106/0108/0109
- ✓ `docs/product/journeys/run-a-vm-workload.yaml`
- ✓ `docs/product/journeys/submit-a-job.yaml`
- ✓ `docs/product/journeys/submit-a-service.yaml`
- ✓ `docs/feature/microvm-driver-cloud-hypervisor/feature-delta.md`
- ✓ `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md`
- ✓ `docs/feature/service-kind-vm-workloads/feature-delta.md`
- ✓ `docs/feature/vm-recreation-allocation-id-reuse/feature-delta.md`
- ⊘ `docs/feature/remove-legacy-exec-workload-driver/discuss/wave-decisions.md`
  (not found; the user explicitly skipped DISCUSS)
- ⊘ `docs/feature/remove-legacy-exec-workload-driver/discuss/user-stories.md`
  (not found; GH #293 is the requirements contract)
- ⊘ `docs/feature/remove-legacy-exec-workload-driver/discuss/story-map.md`
  (not found; no journey may be invented)
- ⊘ `docs/feature/remove-legacy-exec-workload-driver/discuss/outcome-kpis.md`
  (not found; quality priorities derive from #293 and accepted SSOT)
- ⊘ `docs/feature/remove-legacy-exec-workload-driver/spike/findings.md`
  (not found; #293 requires no new mechanism spike)

Contradiction check: no unresolved contradiction. GH #293 removes the temporary
Exec adapter while the accepted #284 correction already makes physical
allocation identity driver-neutral. The apparent tension with the current
per-workload netns/veth/TAP path is resolved by the issue's explicit sequencing:
#293 removes only Exec-specific branches; #295, later, owns the shared-switch
replacement and the deletion of the current VM network mechanism.

## Wave: DESIGN / [REF] Revalidated Premise and Scope

Observed production facts:

| Boundary | Current production fact | Consequence for #293 |
|---|---|---|
| Operator admission | `WorkloadSpecInput` and the HTTP wire type each admit `[exec]` or `[vm]`; the CLI projects both into persisted intent. | Remove the live `[exec]` arm at both ingress layers. A disabled or deprecated alias is not permitted. |
| Intent persistence | `WorkloadIntentEnvelope` V1 contains only Exec; V2 contains Exec and VM. Public aliases point at V2. | User-approved greenfield cut: delete both historical shapes and reset the VM-only envelope to V1. No reader, bridge, or old fixture survives. |
| Production composition | `run_server` always composes `ExecDriver`, then conditionally inserts `VmDriver` after the existing VMM discovery/probe. | Delete `ExecDriver` composition. Preserve the existing VMM capability absence/probe semantics rather than inventing a new boot policy. |
| Allocation lifecycle | `WorkloadLifecycle` already emits driver-neutral predecessor-to-fresh-successor `RestartAllocation`; `alloc_id != spec.alloc` is mandatory. | Preserve this contract exactly. Removing Exec deletes one payload projection, not an identity-policy branch. |
| Action shim | Network assignment and interception contain explicit `Exec | Vm` branches and optional VM TAP parameters because the two drivers shared the path. | Collapse those branches onto the current VM path, while retaining the current VM mechanism until #295. |
| Driver routing | `DriverRegistry` and `AllocDriverIndex` serve start, stop/finalize, exit observation, lifecycle hooks, and future driver capability composition. | The registry is not an Exec compatibility shim. Keep it as the execution-family routing boundary; remove only the Exec entry and payload route. |
| Worker cgroups and probes | `CgroupManager`, cgroup preflight, `CgroupFs`, and cgroup accounting are used by `VmDriver`; the host Exec health probe is separate code but has no successful admitted workload after driver removal. | Keep shared VM cgroup infrastructure. Delete the unreachable host Exec probe surface under approved P-293-4; HTTP/TCP and VM confinement remain. |
| Current VM networking | VM start requires the existing netns/veth/two-`/30`/TAP channel, and current transparent mTLS matches `host_veth`. | Retain it only as the current VM path. Do not implement its #295 replacement or describe it as permanent driver-neutral architecture. |

Static inventory at design time found 108 files naming `ExecDriver`, 32 naming
`WorkloadDriver::Exec`, 83 naming `DriverInput::Exec`, 43 naming
`DriverPayload::Exec`, and 98 naming `DriverType::Exec` across source, tests,
examples, and current documentation. This is broad migration fallout, not
evidence for a new subsystem. The implementation must classify each occurrence
as active execution surface, generic-fixture migration, or an Exec-coupled
schema that must reset. The user confirmed there are no production users, so
old persisted/wire evidence creates no compatibility obligation.

All priorities in GH #293 are in scope. No #295 implementation is in scope.

## Wave: DESIGN / [REF] Quality Attributes and Constraints

| Rank | Attribute | Design response |
|---:|---|---|
| 1 | Functional suitability / honest support boundary | `[exec]` cannot enter live parser, wire, intent, payload, registry-composition, or worker execution paths. |
| 2 | Reliability / recoverability | The affected schemas reset together in one forward-only cut; no old bytes can be mistaken for supported current state. |
| 3 | Maintainability | Retain the existing ports-and-adapters registry and allocation lifecycle; delete the concrete Exec adapter and its branches rather than add a compatibility layer. |
| 4 | Security / isolation honesty | Documentation names the microVM family and its current KVM/VMM boundary; no host-kernel process workload remains supported. |
| 5 | Testability | The live type graph makes Exec admission unrepresentable; existing simulation adapters exercise the VM kind; only new V1 fixtures remain. |
| 6 | Compatibility / migration | User-approved greenfield single cut: no old workload-intent, parser-Service, allocation-row, or occurrence bytes are supported or migrated. |
| 7 | Performance efficiency | No new runtime hop, task, store, retry, or network mechanism; active dispatch loses one branch/adapter. |
| 8 | Usability | `[vm]` is the sole supported driver table; unsupported, unknown, invalid, or missing-driver input follows the parser's existing ordinary diagnostics before any remote effect. No retired-Exec diagnostic is added. |

Constraints:

- OOP remains the repository paradigm; this design does not reopen it.
- Existing modular-monolith and ports-and-adapters boundaries remain.
- No new crate, daemon, external service, persistence engine, lifecycle state,
  action variant, identity type, retry owner, or network mechanism is added.
- VMM capability absence remains soft at node boot; a successfully discovered
  but lying VMM substrate retains its existing hard startup refusal.
- The `Driver` trait and public `Action::{StartAllocation,RestartAllocation}`
  signatures do not change.
- #295 is the only authorized implementation owner for shared-switch,
  per-tap classification/interception, shared-bridge DNS, and the associated
  deletion/replacement of netns/veth/`NetSlot`/subnet-carve mechanics.
- No team-structure change was supplied. Conway alignment is preserved by
  leaving code ownership and deployment boundaries unchanged.

Constraint/priority allocation is explicit: **100%** of newly admitted Exec
execution paths and concrete Exec runtime composition are removed; **0%** of
the accepted P-105 allocation-lifecycle semantics may change; and **0%** of
#295's replacement mechanism may be implemented. Shared VM-used cgroup/network
components are outside the first percentage because they are not live Exec
capability.

## Wave: DESIGN / [REF] DDD List

- **P-293-1 — live execution contract becomes microVM-only
  (USER-APPROVED 2026-09-14):**
  remove active Exec variants and `ExecDriver`, while retaining the existing
  tagged driver unions and `DriverRegistry` for the supported microVM execution
  family. Preserve ADR-0083's capability-absence result: ordinary VMM absence
  may leave an empty registry while `overdrive serve` still boots; a VM start
  then follows the existing typed no-capability path. See ADR-0110.
- **P-293-2 — affected spec/intent envelopes reset forward-only
  (USER-APPROVED 2026-09-14):** replace the current parser-Service V3 and
  workload-intent V1/V2 histories with one incompatible VM-only V1 each;
  delete old payload types, readers, conversions, fixtures, and error branches.
  See ADR-0111.
- **P-293-3 — affected lifecycle evidence resets forward-only
  (USER-APPROVED 2026-09-14):** delete Exec-only `TransitionReason` variants and
  `DriverType::Exec`, then reset `AllocStatusRowEnvelope` and
  `AllocLifecycleOccurrenceRowEnvelope` to new incompatible V1 shapes. No old
  row/occurrence reader, label, or fixture survives. See ADR-0112.
- **P-293-4 — remove the now-unreachable host Exec health-probe path
  (USER-APPROVED 2026-09-14):** delete `ProbeMechanic::Exec`, the `ExecProber`
  port/adapters, `CgroupExecProber`, and VM-Exec rejection plumbing. GH #280
  remains the independent owner of any future in-guest command probe and may
  not reuse host-cgroup semantics as a compatibility shortcut. See ADR-0113.
- **P-293-5 — allocation lifecycle remains driver-neutral
  (USER-APPROVED 2026-09-14):**
  preserve the accepted P-105 predecessor/fresh-successor semantics, View
  reservation, handoff, publication, and cleanup ordering exactly; remove only
  the Exec payload projection and rename the ambiguous private
  `AllocationAttemptEvent::Exec` to `::Dispatch`.
- **P-293-6 — #295 handoff is a constraint, not implementation
  (USER-APPROVED 2026-09-14):**
  #293 removes Exec-only network branches but temporarily retains the current
  VM netns/veth/TAP/`host_veth` mechanism because the production VM path still
  uses it. #293 must not implement any replacement named under “Required
  handoff to #295”.

No new bounded context or aggregate is created. `WorkloadId` remains the
stable intent/policy owner; `AllocationId` remains one physical execution;
drivers remain effect adapters that do not choose allocation identity.

## Wave: DESIGN / [REF] Material Options

### Choice A — execution-family routing

| Option | Shape | Trade-off |
|---|---|---|
| **A1 — retain tagged unions + `DriverRegistry` (selected; user-approved 2026-09-14)** | Live unions have only `Vm` today; registry may add later microVM-family adapters. `AllocDriverIndex` and per-driver exit observers remain. Ordinary VMM absence may leave an empty registry without refusing `serve`, preserving ADR-0083. | Smallest semantic change; preserves capability composition and driver-neutral lifecycle. Carries a one-arm union/registry today, and a node can be control-plane-live while unable to execute a workload. |
| A2 — collapse to one `Arc<VmDriver>` | Delete registry and route every action directly to the VM driver. | Less indirection now, but reintroduces the multi-driver refactor ADR-0083 already paid for when unikernel/sandboxed microVM adapters arrive. |
| A3 — hard-code a `match` in the action shim | Keep a type discriminator but replace registry lookup with concrete VM dispatch. | Duplicates the composed-capability fact and couples the control plane to the worker adapter; rejected by existing dependency direction. |

The persisted-data and historical-evidence alternatives are closed by the
user's 2026-09-14 greenfield ruling. No
backwards-reader, V2/V3 append, migration bridge, legacy variant, legacy-
specific payload error branch, or old-data consequence may be reintroduced.

### Choice B — host Exec health-probe residue

| Option | Shape | Trade-off |
|---|---|---|
| **B1 — delete the host Exec probe path (selected; user-approved 2026-09-14)** | Remove `ProbeMechanic::Exec`, `ExecProber`, `CgroupExecProber`, runner branches, VM rejection diagnostics, docs, and tests. GH #280 later designs an in-guest port from a clean boundary if a concrete workload justifies it. | Completes deletion discipline and removes code with no admissible successful workload. A future #280 feature re-adds its own typed/wire surface. |
| B2 — retain rejected syntax and host port | Keep `ProbeMechanic::Exec` and host `ExecProber`, but every live VM admission still rejects it before intent. | Preserves today's actionable GH #280 error but leaves a production port/adapter with no supported success path and keeps host-cgroup semantics adjacent to a future guest feature. |
| B3 — reuse host `ExecProber` for VM guests | Route VM Exec probes through the current host subprocess/cgroup adapter. | Rejected: it would probe the host, not the guest, and would implement #280 with the wrong trust/lifecycle boundary. |

P-293-5 and P-293-6 have no genuine alternative within #293: changing
allocation identity or implementing the #295 dataplane would be scope expansion
rather than an implementation option.

## Wave: DESIGN / [REF] Exact Live Type and Port Contracts

Exact API contracts belong here, not in ADRs.

### Operator/parser/wire surface

The two live driver unions retain their current names and contain exactly one
live arm:

```rust
// aggregate::workload_spec (TOML parser)
pub enum DriverInput {
    Vm(VmInput),
}

// aggregate (HTTP/JSON submit + describe)
pub enum DriverInput {
    Vm(VmInput),
}
```

The live `ExecInput` types in both modules are deleted. The parser grammar has
no Exec field, table, presence flag, variant, type, or special-case branch; it
recognizes only `[vm]` as a driver table. No dedicated retired-driver
`ParseError` variant or message exists. A document whose only former driver table
is `[exec]` therefore has no recognized supported driver and follows the
existing `ParseError::MissingDriverSection` path. That existing variant uses
the parser's ordinary missing-required-section wording and names only `[vm]`;
#293 defines no new exact display-string contract. Other unsupported, unknown,
or malformed input follows the ordinary generic parser error applicable to its
shape; none receives an Exec-specific classification or compatibility promise.

`MultipleDriverSections` is deleted because the live grammar has one driver
section. Raw JSON carrying an unsupported driver-union spelling follows the
existing generic serde/HTTP decode error and never reaches an intent
constructor. The design adds or renames no public error, API, alias, or
diagnostic surface.

The public convenience accessor below is **deleted without replacement**:

```text
WorkloadSpecInput::exec_command(&self) -> &str
```

It has no production caller and its Exec-specific public name is incompatible
with the approved single-cut surface. No `command()`, `driver_command()`,
`vm_command()`, or other replacement accessor is added. Its sole in-tree
consumer is the `coinflip_migration` acceptance test: that one command-value
assertion is deleted, while the test migrates the example to `[vm]` and retains
its driver-neutral Job-kind and workload-ID assertions.

After the cut, active production, public API, wire/schema, parser, runtime,
configuration, and generated OpenAPI surfaces contain zero dedicated Exec
vocabulary. Historical ADR/evolution context may name the deleted symbols, and
a generic parser may echo an operator-supplied token in an ordinary diagnostic;
neither is a supported compatibility contract.

### Exact affected-envelope inventory

Current-code evidence limits the reset to four owners:

| Envelope | Current embedded Exec-coupled surface | Forward-only result |
|---|---|---|
| `ServiceSpecEnvelope` (`aggregate/service_spec.rs`) | sole `V3(ServiceSpecV3)`; `ServiceSpecV3.driver` is the parser `DriverInput::{Exec,Vm}` | replace with sole incompatible `V1(ServiceSpecV1)` carrying VM-only `DriverInput` |
| `WorkloadIntentEnvelope` (`aggregate/mod.rs`) | `V1` embeds Exec-only `WorkloadDriverV1`; `V2` embeds `WorkloadDriverV2::{Exec,Vm}` across Job/Service/Schedule | replace the entire family with sole incompatible VM-only V1 |
| `AllocStatusRowEnvelope` (`traits/observation_store.rs`) | V1/V2/V3 each embed `reason: Option<TransitionReason>`; V3 crash facts can carry the same reason | collapse current V3 fields into one new incompatible `AllocStatusRowV1`; delete prior versions/conversions |
| `AllocLifecycleOccurrenceRowEnvelope` (`traits/observation_store.rs`) | V1 embeds `reason: Option<TransitionReason>` and `source: TransitionSource::Driver(DriverType)` | retain one version name but regenerate an incompatible V1 after Exec reason/source removal |

No other envelope contains the removed driver, driver-input union,
`TransitionReason`, or `DriverType`. `ProbeResultRowEnvelope`,
`NodeHealthRowEnvelope`, `ServiceHydrationResultRowEnvelope`,
`ServiceBackendRowEnvelope`, `ReconcileConflictRowEnvelope`, CA envelopes, and
`WorkflowStartEnvelope` are unchanged and must not be reset.

Approved P-293-4/B1 removes `ProbeMechanic::Exec`, which also changes the
`ProbeDescriptor` nested inside the two already-affected Service-spec and
workload-intent envelopes; it does not add a fifth envelope owner.

The exact Exec-only lifecycle vocabulary deleted before the two row envelopes
are reset is:

```text
DriverType::Exec
TransitionReason::ExecBinaryNotFound
TransitionReason::ExecPermissionDenied
TransitionReason::ExecBinaryInvalid
TransitionReason::CgroupSetupFailed
TransitionReason::OutOfMemory
```

`TransitionReason::WorkloadCrashedImmediately`, `DriverInternalError`,
`StoppedBy::Process`, `stderr_tail`, and cgroup/VM failure fields stay because
the current VM path produces or consumes them. No other allocation-row field
is Exec-specific in current code.

### Parser-side Service envelope

`ServiceSpecEnvelope::V3` embeds the current two-arm parser `DriverInput` and
therefore changes when Exec is removed. Per the user-approved greenfield cut,
it does not append V4 and does not retain V3. It resets to one VM-only V1:

```rust
pub enum DriverInput {
    Vm(VmInput),
}

pub type ServiceSpec = ServiceSpecV1;
pub type ServiceSpecLatest = ServiceSpecV1;

pub enum ServiceSpecEnvelope {
    V1(ServiceSpecV1),
}
```

`ServiceSpecV3`, its fixture, and every V3 reader/conversion are deleted. A new
V1 fixture pins the sole current shape. `JobSpec`, `ScheduleSpec`,
`WorkloadSpec`, and `WorkloadSpecInput` use the same live VM-only
`DriverInput`; they need no distinct reset because they own no separate
versioned storage envelope.

### Live intent and rkyv envelope

The current V1/V2 family is deleted and the public aliases reset together to a
new, incompatible VM-only V1:

```rust
pub type WorkloadDriver = WorkloadDriverV1;
pub type Job = JobV1;
pub type Service = ServiceV1;
pub type Schedule = ScheduleV1;
pub type WorkloadIntent = WorkloadIntentV1;
pub type WorkloadIntentLatest = WorkloadIntentV1;

pub enum WorkloadDriverV1 {
    Vm(Vm),
}

pub enum WorkloadIntentEnvelope {
    V1(WorkloadIntentV1),
}
```

The new `JobV1`, `ServiceV1`, and `ScheduleV1` preserve the current V2 fields
except that their driver is the VM-only `WorkloadDriverV1`. The three
`WorkloadIntentV1::{Job,Service,Schedule}` variants retain their current
meanings. `WorkloadIntentEnvelope::latest` writes V1 and `into_latest` has only
the V1 identity arm.

Old `WorkloadDriverV1/V2`, `Exec`, `JobV1/V2`, `ServiceV1/V2`,
`ScheduleV1/V2`, `WorkloadIntentV1/V2`, conversion impls, discriminant history,
and golden fixtures are deleted rather than retained or renamed. The new V1
names are the only source types after the cut. No `EnvelopeError` variant,
old-byte detector, refusal translation, or migration method is added.

### Driver and registry surface

The runtime payload becomes:

```rust
pub enum DriverPayload {
    Vm(VmPayload),
}
```

`ExecPayload`, `ExecStartFailure`, and `DriverStartClass::Exec` are deleted.
`DriverError::NetnsEntry` is also deleted: current-code search finds its only
producers in `ExecDriver` and its only behavioral tests in the Exec-driver
integration suite. VM netns entry is owned by the existing `Vmm`/host adapter
path and does not use this error.
The `Driver` trait signatures, `DriverRegistry` methods, `AllocDriverIndex`,
and `AllocationSpec` fields remain exact. The live route is structurally
`DriverPayload::Vm -> DriverType::Vm -> DriverRegistry::get(Vm)`.

`DriverType::Exec` and every Exec-only `TransitionReason` variant are deleted,
not preserved for old rows or wire clients. The affected row/occurrence
envelopes reset under P-293-3. `DriverStartFailure -> TransitionReason` becomes
VM/unclassified-only; OpenAPI and wire schemas regenerate from the reduced
enums with no alias or deprecated spelling.

The concrete `overdrive_worker::ExecDriver`, its module/export, its production
composition helper `compose_production_driver`, and its worker integration and
compile-fail suites are deleted. `run_server` calls the existing
`probe_runner_boot::compose_and_probe_runner_gate` directly, then passes the
trusted runner to the existing `compose_vm_driver`. No replacement helper or
new port is introduced.

### Host Exec health-probe surface (approved P-293-4/B1)

The live probe algebra and port module become HTTP/TCP-only:

```text
pub enum ProbeMechanic {
    Tcp { host: String, port: u16 },
    Http { path: String, port: u16, host: Option<String> },
}

pub enum ProbeFailure {
    InvalidTarget { reason: String },
}

ProbeRunner::new(
    tcp_prober: Arc<dyn TcpProber>,
    http_prober: Arc<dyn HttpProber>,
    clock: Arc<dyn Clock>,
    observation_store: Arc<dyn ObservationStore>,
) -> ProbeRunner

async fn compose_and_probe_runner_gate(
    tcp_prober: Arc<dyn TcpProber>,
    http_prober: Arc<dyn HttpProber>,
    clock: Arc<dyn Clock>,
    observation_store: Arc<dyn ObservationStore>,
) -> Result<Arc<ProbeRunner>, ControlPlaneError>
```

`ExecProber`, `CgroupExecProber`, `SimExecProber`,
`ProbeFailure::ExecSpawnFailed`, `ParseError::ExecProbeMissingCommand`,
`parse_exec_mechanic`, VM-Exec cross-field validators/diagnostics, runner
fields/parameters/match arms, and their tests/docs are deleted. A
currently unsupported probe-mechanic spelling follows the existing generic
`ParseError::UnknownProbeType` result; no exact deleted spelling is a test or
compatibility contract. HTTP/TCP behavior, target projection, roles,
thresholds, rows, supervision, and Earned-Trust TCP probe remain unchanged.

GH #280 remains an unimplemented future feature. It must establish a new
in-guest execution port and protocol through its own approved DESIGN; #293
adds no placeholder or compatibility seam for it.

### Workload lifecycle

The public action shapes remain unchanged. The accepted contract remains:

```text
RestartAllocation.alloc_id = accepted numeric-current predecessor
RestartAllocation.spec.alloc = distinct durably-reserved fresh successor
```

`WorkloadLifecycle` removes the Exec projection arms from initial and
replacement spec construction. `next_allocation_attempt`,
`restart_allocation_action`, View meanings, `Failed|Terminated` handoff,
successor-first action-shim ordering, publication, and exact-old cleanup remain
byte/behavior equivalent to P-105. The generic preflight event is renamed from
`AllocationAttemptEvent::Exec` to `AllocationAttemptEvent::Dispatch`; it means
“apply a start/restart action”, not a workload driver.

### Current VM network/action-shim surface

The current VM network path is narrowed without replacing it:

```rust
pub trait WorkloadNetworkProvisioner {
    fn provision(
        &self,
        workload: &WorkloadNetnsPlan,
        vm_tap: &VmTapPlan,
    ) -> Result<(), VethProvisionError>;

    fn teardown(
        &self,
        workload: &WorkloadNetnsPlan,
    ) -> Result<(), VethProvisionError>;
}

fn provision_and_inject_netns(
    spec: &mut AllocationSpec,
    net_slot_allocator: &NetSlotAllocator,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
) -> Result<(), ShimError>;

fn inject_workload_network(
    spec: &mut AllocationSpec,
    workload: &WorkloadNetnsPlan,
    vm_tap: &VmTapPlan,
);
```

`network_assignment_required`, the optional `VmTapPlan`, the Exec transit-
address injection branch, and `DriverType::Exec | DriverType::Vm` interception
matches are deleted. Every admitted allocation follows the current VM plan;
interception requirement is determined only by whether the existing mTLS
lifecycle is composed. The private `exec_release_permitted` helper is renamed
`guest_command_release_permitted` because it gates the VM beacon `EXEC` command,
not the removed Exec workload driver.

`AllocationSpec::{netns,host_veth,workload_addr,guest_tap,guest_mac,
guest_gateway,guest_prefix_len,guest_dns}` remain unchanged for #293. They are
currently populated for VM and are therefore not being retained solely for
Exec compatibility. Their replacement/deletion is #295 work.

## Wave: DESIGN / [REF] Component Decomposition

| Component | Path | Change | Responsibility after #293 |
|---|---|---|---|
| TOML workload parser | `overdrive-core/src/aggregate/workload_spec.rs` | EXTEND/DELETE | Make `[vm]` the sole recognized driver table; delete all Exec parser shapes/presence checks and public `WorkloadSpecInput::exec_command` without replacement. Unsupported or absent driver input follows only existing generic parser errors. |
| HTTP submit/describe types | `overdrive-core/src/aggregate/mod.rs`, `src/api/{submit,describe}.rs` | EXTEND/DELETE | Carry only live VM driver input; preserve workload-kind union. |
| Parser Service codec | `overdrive-core/src/aggregate/service_spec.rs`, `workload_spec.rs`, schema fixtures | RESET (approved) | Delete V3/history; create sole incompatible VM-only V1. |
| Workload intent codec | `overdrive-core/src/aggregate/mod.rs`, schema fixtures | RESET (approved) | Delete old V1/V2/history; create sole incompatible VM-only V1. |
| Allocation row codec | `overdrive-core/src/traits/observation_store.rs`, schema fixtures | RESET (approved) | Keep current row fields minus removed nested Exec vocabulary; collapse envelope to sole new V1. |
| Lifecycle occurrence codec | `overdrive-core/src/traits/observation_store.rs`, schema fixtures | RESET (approved) | Regenerate sole V1 after Exec reason/source deletion; no historical reader. |
| Runtime driver contracts | `overdrive-core/src/traits/driver.rs` | EXTEND/DELETE | Keep `Driver`, registry, and VM payload; remove active Exec payload/start classes. |
| `ExecDriver` | `overdrive-worker/src/driver.rs`, `overdrive-worker/src/lib.rs` | DELETE | No replacement component. |
| `VmDriver` / `Vmm` | `overdrive-worker/src/vm_driver.rs`, `overdrive-core::traits::vmm`, host/sim adapters | REUSE | Sole current production execution adapter; lifecycle/guest honesty unchanged. |
| Production composition | `overdrive-control-plane/src/lib.rs`, CLI `serve` | EXTEND/DELETE | Probe the existing runner, compose optional VM capability, never insert Exec. |
| `DriverRegistry` / `AllocDriverIndex` | core + control-plane | REUSE | Preserve microVM-family capability/routing and stop/finalize ownership. |
| `WorkloadLifecycle` | `overdrive-reconcilers/src/workload_lifecycle.rs` | EXTEND | Remove Exec payload projections; retain driver-neutral identity and replacement policy. |
| Action shim network/intercept path | `overdrive-control-plane/src/action_shim/mod.rs` | EXTEND | Remove Exec/no-network branches; retain current VM topology until #295. |
| Exit observer | `overdrive-control-plane/src/worker/exit_observer.rs` | REUSE/REWORD | Continue one observer per registry entry; active entries are VM-family only. |
| `SimDriver` and generic fixtures | `overdrive-sim`, cross-crate tests | EXTEND | Exercise `DriverType::Vm`; no Exec compatibility fixture remains. |
| `ProbeRunner` / probe ports | worker/core/sim/control-plane | EXTEND/DELETE (approved P-293-4) | Retain HTTP/TCP; delete the unreachable host Exec mechanic, port, adapters, runner fields/arms, and VM rejection shim. |
| Cgroup infrastructure | worker/core/host | REUSE | Keep VM cgroup confinement and accounting; delete only ExecDriver/ExecProber-specific consumers. |
| Operator examples/current docs | `README.md`, `examples/`, whitepaper, product journeys/jobs/outcomes | MIGRATE/DELETE after approval | Make VM the supported appliance execution model; remove active Exec examples and promises. |
| Historical ADR/evolution prose | accepted `adr-*`, `docs/evolution/` | REUSE AS HISTORY | Historical narrative may say Exec existed; it is not runtime/wire compatibility and is not rewritten as current support. |

## Wave: DESIGN / [REF] Driving Ports

- `overdrive deploy <SPEC>` remains the only workload-deploy verb. A `[vm]`
  Job/Service follows the existing parse → HTTP → intent path. Input without
  the supported `[vm]` table follows the existing generic
  `MissingDriverSection` path; unsupported raw JSON driver spellings follow the
  existing generic decode error. Neither writes intent. No special retired-
  driver case is added.
- `overdrive workload describe <ID>` retains its route and workload-kind
  output. Live specs and lifecycle reason/source schemas contain no Exec arm.
- `overdrive serve` keeps the existing VMM discover/probe semantics. A node
  without an available VMM may still boot with an empty driver registry and
  reject a later VM start through the existing typed capability path.
- Convergence continues through `IntentStore` → `WorkloadLifecycle` → existing
  actions → action shim. No CLI or driver selects allocation identity.

## Wave: DESIGN / [REF] Driven Ports and Adapters

| Port/effect | Adapter after #293 | Contract |
|---|---|---|
| `Driver` | `VmDriver`; `SimDriver(DriverType::Vm)` | Existing start/stop/status/resize, exit, supervision, and lifecycle-hook contract. |
| `Vmm` | `CloudHypervisorVmm`; `SimVmm` | Existing wire-then-probe-then-use gate and guest-honest lifecycle. |
| `ObservationStore` | existing local/sim adapters | New forward-only V1 allocation rows/occurrences; no old reader or Exec reason/source. |
| `IntentStore` | existing local store | New forward-only VM-only WorkloadIntent V1; no old reader, translation, or migration. |
| `WorkloadNetworkProvisioner` | existing host/test adapters | Current VM netns/veth/TAP convergence only; no replacement mechanism. |
| `MtlsInterceptLifecycle` | existing worker/sim adapters | Current VM allocation install/teardown and fail-closed ordering only. |
| `CgroupFs` / accounting | existing host/sim adapters | VM confinement/accounting; not owned by the removed driver or probe path. |
| `TcpProber` / `HttpProber` | existing adapters | Unchanged HTTP/TCP probe contracts; `ExecProber` and its adapters are deleted by approved P-293-4/B1. |

No external API or third-party service is added, so no new consumer-driven
contract test is required.

## Wave: DESIGN / [REF] Technology Choices

- Rust and the repository's OOP, modular-monolith, ports-and-adapters style:
  unchanged.
- Tokio: unchanged runtime for VM supervision and probe tasks.
- Cloud Hypervisor: unchanged sole current VMM adapter; existing discovery and
  probe remain the capability gate.
- rkyv + redb: unchanged persistence technologies; exactly four affected
  envelope owners reset to forward-only V1 under the user-approved greenfield
  cut, while all unrelated envelopes remain unchanged.
- Linux cgroup v2 and the current netns/veth/TAP/nft path: unchanged for the
  current VM implementation. The latter is explicitly temporary pending #295.
- New dependencies and proprietary technology: none.

Architecture enforcement remains type-first: parser, wire, intent, runtime
payload, failure-reason, and driver-source enums have no Exec arm; no concrete
Exec driver is exported; dependency direction stays core ← adapters ←
composition. New V1 golden-byte fixtures pin only the post-cut schemas, and the
existing crate-class/dst-lint gates remain. The removed-name statements are
implementation/reviewer deletion-audit criteria, not executable absence tests,
source scans, or compile fixtures.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing component | Overlap | Decision | Contract shape / universe / assertion mechanism |
|---|---|---|---|
| `WorkloadSpecInput` | Driver-table admission | EXTEND/DELETE | **pure-function** over one TOML document. Delete the Exec table/type/presence walk and retain VM output plus the parser's ordinary generic unknown/invalid/missing-supported-driver errors. Evidence covers successful `[vm]` parsing and ordinary generic failure/no-write behavior using non-legacy-specific invalid input; the deleted spelling, error, and types receive no executable assertion. |
| Live submit/describe `DriverInput` | Driver wire union | EXTEND | pure tagged projection; universe one workload DTO; only VM round-trips. |
| `ServiceSpecEnvelope` | Parser-side direct archive contract | RESET V1 | pure codec; universe one current Service spec; one new VM-only V1 roundtrip, no old fixture/read path. |
| `WorkloadIntentEnvelope` | Durable driver intent | RESET V1 | pure codec; universe one current workload intent; one new VM-only V1 family, no old fixture/read path. |
| `AllocStatusRowEnvelope` | Durable current allocation row | RESET V1 | bounded persisted row universe; current fields retained, Exec-only nested variants deleted, one new V1 fixture. |
| `AllocLifecycleOccurrenceRowEnvelope` | Durable lifecycle occurrence | RESET V1 | bounded occurrence universe; Exec-only nested reason/source deleted, one regenerated V1 fixture. |
| `DriverPayload` | Action-to-driver routing | EXTEND/DELETE | pure-function over one surviving VM payload; existing VM action-to-driver projection and routing behavior remain the evidence. Deleted payload names receive no fixture or source-shape assertion. |
| `DriverRegistry` | Capability/routing | REUSE | bounded-change over composed entry set; production composition inserts VM only; `kinds()` remains deterministic. |
| Production composition root (`run_server`, `probe_runner_boot`, `compose_vm_driver`) | Trusted probe-runner ownership and driver capability insertion | EXTEND/DELETE | **bounded-change** over exactly one server boot's trusted `ProbeRunner` binding and `DriverRegistry` entry set. Delete Exec construction plus `compose_production_driver`; retain the existing probe gate and conditional VM discovery/probe/insert. Allowed post-state is registry `∅` on ordinary VMM absence or `{Vm}` on success; present-but-failing VMM still refuses boot. Evidence at the existing composition boundary asserts that the same trusted runner reaches `VmDriver` and that registry kinds equal the VMM outcome; deletion of the old entry/helper is review audit, not a test subject. |
| Per-composed-driver exit-observer ownership (`run_server_with_obs_and_drivers`, `exit_observer`) | One sole receiver/provenance owner per registry entry and coordinated shutdown | REUSE WITH ENTRY-SET NARROWING | **bounded-change** over one shared `exit_observer_shutdown` token, `ServerHandle.exit_observer_tasks`, each composed driver's one taken receiver/captured `DriverType`, resulting lifecycle writes, and supervision release. #293 changes only the registry entry set: zero entries spawn zero observers; one VM entry spawns one VM observer. Assert task count/provenance equals `drivers.kinds()`, all tasks share cancellation and are joined, and a VM exit retains VM source plus supervision-release behavior. No merger, helper, token, or shutdown owner is added. |
| `AllocDriverIndex` | Stop/finalize routing without a spec | REUSE | bounded-change over exact predecessor/successor IDs; existing P-105 ordering assertions. |
| `Driver` trait | Execution effects | REUSE AS-IS | bounded-change inside exact allocation capability; existing host/sim equivalence and VM acceptance evidence. |
| `VmDriver` / `Vmm` | Supported execution | REUSE AS-IS | bounded-change over one VM allocation; existing native-metal and sim evidence. |
| `ExecDriver` | Removed host-process execution | DELETE | Production code and its tests delete together; no replacement. |
| `WorkloadLifecycle` | Identity/replacement policy | EXTEND projection only | pure-function over desired/actual/View/tick; all non-driver policy outputs remain complement-equal. |
| Action shim | Runtime effect sequencing | EXTEND | bounded-change over one action's successor/predecessor owners; remove Exec branches, preserve P-105 and current VM effect order. |
| Current network/mTLS ports | VM datapath | REUSE/NARROW | bounded-change over one allocation's current netns/veth/TAP/rules; #293 adds no shared-switch or per-tap effect. |
| `SimDriver` | Deterministic execution adapter | EXTEND fixtures | bounded-change over one simulated VM allocation; no Exec-kind fixtures. |
| `TransitionReason` / `TransitionSource` / `DriverType` | Current lifecycle evidence | EXTEND/DELETE | pure wire/persisted vocabulary; delete exact Exec-only variants, retain VM/generic variants, regenerate only affected V1 fixtures/OpenAPI. |
| `ProbeMechanic` / `ProbeRunner` / prober ports | Service health | EXTEND/DELETE (approved P-293-4/B1) | **bounded-change** over exactly one allocation's `ProbeRunner.supervisors` entry, its HTTP/TCP task set, cancellation token, and emitted `ProbeResultRow`s. Delete only the Exec dependency/dispatch set; HTTP/TCP registration, scheduling, cancellation, result writes, thresholds, and Earned-Trust TCP probe are complement-equal. Evidence asserts surviving HTTP/TCP registration, task state deltas, cancellation, and observation writes through the existing runner boundary. Deleted variants, ports, adapters, and constructor parameters receive no compile/source-shape fixture. |
| `WorkloadSpecInput::exec_command` | Public parser convenience accessor | DELETE WITHOUT REPLACEMENT | **pure-function** with universe ∅. Its name is Exec-specific and its sole in-tree consumer is the `coinflip_migration` test. Delete the public method and that command assertion; retain the migrated example's driver-neutral kind/id assertions. Do not add or rename a public accessor. |
| Cgroup infrastructure | Shared VM/worker substrate | REUSE AS-IS | bounded-change over declared cgroup paths; existing Earned-Trust probe and Tier-3 kernel evidence. |

New component count: **zero**. The four affected envelope owners receive new
forward-only V1 source shapes; no compatibility type, error, reader, bridge, or
migration component is created.

## Wave: DESIGN / [REF] Decisions Table

| ID | User-approved decision | Architectural record |
|---|---|---|
| P-293-1 | VM-only live driver unions and concrete adapters; retain registry/routing | ADR-0110 — user-approved 2026-09-14 |
| P-293-2 | Reset affected Service-spec and workload-intent envelopes to VM-only V1; delete all history | ADR-0111 — user-approved 2026-09-14 |
| P-293-3 | Delete Exec row/source vocabulary and reset allocation/occurrence envelopes to V1 | ADR-0112 — user-approved 2026-09-14 |
| P-293-4 | Delete unreachable host Exec probe surface; leave future in-guest command probing entirely to GH #280 | ADR-0113 — user-approved 2026-09-14 |
| P-293-5 | Preserve driver-neutral P-105 allocation lifecycle exactly | User-approved scope; ADR-0105/0106/0108/0109; no new ADR |
| P-293-6 | #295 handoff bullets constrain #293 but are not #293 implementation | User-approved scope; GH #293/#295; no new ADR |

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

### Existing state-ownership matrix

| Signal/state | Owner | Promise | Inputs that may gate it | Must not gate |
|---|---|---|---|---|
| Workload intent accepted | Existing CLI/parser + control-plane handler + `IntentStore` | A live supported workload declaration was validated and committed. | Kind validation, VM driver table, resources/listeners/probes, store result. | VMM boot completion, allocation Running, predecessor cleanup. |
| VM capability composed | Production composition root + `Vmm::probe` | This node has a probed VM driver entry, or explicitly lacks it. | Existing VMM discovery/probe only. | Control-plane boot on ordinary capability absence, workload identity policy. |
| Physical replacement eligibility | `WorkloadLifecycle` | Numeric-current accepted `Failed|Terminated` predecessor may receive one fresh reserved successor. | Existing P-105 cause/budget/backoff/handoff rules. | Driver kind, Service Stable/health, old cleanup completion. |
| Allocation `Running` | Action shim after `Driver::start` and accepted observation | The current VM network was provisioned, the guest-backed driver reached its existing guest-ready Running promise, and the Running row was accepted. It does **not** promise transparent-mTLS interception is live. | Current VM network provision/injection, VM guest-ready `Driver::start` result, and `ObservationStore::write_alloc_lifecycle` acceptance. | Transparent-mTLS intercept success, guest-command release, Exec compatibility, Service Stable/readiness/liveness. |
| Transparent-mTLS intercept live / VM guest-command release | Action shim sequencing `MtlsInterceptLifecycle` then `Driver::release_for_exit_emission` | After an accepted Running row, the allocation intercept installed successfully before the VM beacon `EXEC` command and exit-event gate were released. | Accepted Running row followed by `MtlsInterceptLifecycle::start_alloc == Ok`. | The already-accepted initial Running row, Service Stable/readiness/liveness, or allocation identity. |
| Exact allocation cleanup | Existing driver/mTLS/network owners sequenced by the shim | Effects for the supplied physical allocation completed or returned their existing typed error. | Exact capability/index/slot and port result. | Successor identity selection or accepted successor rollback. |

### Gate G-293-1 — supported workload-driver admission

- **Existing evidence:** `overdrive deploy` parses `[exec] | [vm]`, projects
  either into HTTP `DriverInput`, and the server constructs live
  `WorkloadDriverV2`; this is the production entry path to intent.
- **Owner:** parser handles TOML; HTTP decode and the new `JobV1`/`ServiceV1`
  constructors protect direct JSON ingress; `IntentStore` persists only the
  new forward-only WorkloadIntent V1.
- **Promise:** any newly accepted intent names the supported VM/microVM path;
  no newly accepted intent can name Exec.
- **Affected state:** only whether workload intent is accepted/committed.
- **Failure projection:** input with no recognized `[vm]` table follows the
  existing generic `MissingDriverSection` path; unsupported raw JSON driver
  spellings follow the existing generic decode path. Neither reaches
  `IntentStore` or allocation lifecycle. There is no retired-Exec error,
  message, branch, or exact legacy-token contract.
- **Explicitly unaffected:** workload-kind selection, VM validation, VMM
  capability absence, replacement identity, Service health, mTLS/networking.
- **Ordering:** parse/decode → validate → intent commit. No timeout is added.
- **Counterexample:** retaining a disabled `DriverInput::Exec` arm and rejecting
  later would allow Exec-shaped intent to leak into live DTOs and keep every
  downstream compatibility match alive.
- **Evidence lane:** positive VM-only parser/constructor coverage plus the
  existing generic `MissingDriverSection`/serde failure and no-intent-write
  behavior, driven with non-legacy-specific invalid input. No deleted type arm,
  exact legacy spelling, special rejection matrix, or retired-driver message is
  an executable acceptance obligation.

### Gate G-293-2 — forward-only affected-schema cut

- **Existing evidence:** current workload intent, parser Service spec,
  allocation row, and lifecycle occurrence envelopes embed Exec-coupled enums;
  the exact four owners are enumerated above. Other envelope payloads do not.
- **Owner:** each affected envelope remains the sole codec for its own current
  payload; the user-approved greenfield deployment resets affected stored data
  rather than asking a new binary to read it.
- **Promise:** every affected write/read after the cut uses the new VM-only V1
  schema; no code can identify, translate, render, or execute a legacy Exec
  payload or row.
- **Affected state:** only the four named envelope schemas and their generated
  wire/OpenAPI vocabulary.
- **Failure projection:** none is added for old bytes. Backwards reads are
  outside the approved contract; existing generic malformed/unknown handling
  remains whatever the new V1 codec already supplies, without an Exec-specific
  branch or diagnostic.
- **Explicitly unaffected:** all other envelope schemas, runtime View state,
  P-105 allocation identity, VMM discovery/probe, Service health, and current
  VM networking.
- **Ordering:** delete old types/conversions/fixtures and reset affected data as
  one deployment cut before the new binary writes V1. No driver/network effect
  participates in the reset.
- **Counterexample:** retaining V2 VM conversion “because it is harmless” is
  still a backwards reader and keeps the old two-driver schema as an
  implementation dependency; resetting unrelated CA/probe/backend envelopes
  would exceed the user-approved bound.
- **Evidence lane:** current V1 roundtrip/schema fixtures for exactly the four
  named owners, exercising their surviving VM/current payloads. Old types,
  conversions, discriminants, bytes, and fixtures are deletion-review scope,
  not executable negative evidence.

### Gate G-293-3 — removal of the Exec-specific network branch

- **Existing evidence:** current action-shim C3 logic gates network assignment
  on `mtls_composed || driver == Vm`, optionally provisions a VM TAP, and
  injects either Exec transit or VM guest addressing before driver start.
- **Owner:** the existing action shim remains the sole sequencer; the current
  `WorkloadNetworkProvisioner` remains the host-effect port.
- **Promise:** every newly admitted allocation takes the complete current VM
  network plan before VM start; there is no “Exec without network” or
  Exec-transit branch. After the driver reports guest readiness, the shim may
  accept `Running` before transparent-mTLS interception is installed.
- **Affected state:** current successor/start network provisioning, the
  accepted initial Running row, and the existing post-Running
  intercept-install result; no state meaning changes.
- **Failure projection:** existing slot exhaustion or provision failure occurs
  before driver start and keeps the existing fail-closed allocation result and
  cleanup. An intercept-install failure occurs **after** accepted Running,
  authors the existing dominating `Failed` row, performs existing cleanup, and
  withholds `Driver::release_for_exit_emission` so the VM guest command cannot
  start. No new error, timeout, retry, or pre-Running gate is added.
- **Explicitly unaffected:** Running meaning, Service health, P-105 identity
  and cleanup ordering, mTLS contracts, and every #295 replacement decision.
- **Ordering:** assign/derive/provision/inject current VM plan → driver start
  reaches guest-ready → Running publication is accepted → current intercept
  install → guest command/exit-event release. Intercept success does not gate
  the already-accepted Running row; it gates only the later release, and its
  failure supersedes Running with the existing dominating Failed result.
- **Counterexample:** deleting netns/`host_veth` here would leave the current VM
  production path unable to attach its TAP or enforce the current transparent
  mTLS contract; replacing them here would implement #295 without its design.
- **Evidence lane:** pure plan injection + action-shim composition; existing
  native-metal evidence remains required for real TAP/netns effects.

### Boundary scenarios for DISTILL

| Boundary | Required observable obligation |
|---|---|
| Available | `[vm]` is accepted; current VM network and guest readiness precede accepted Running, then intercept success precedes VM guest-command release. The P-105 lifecycle remains unchanged. |
| Unavailable | Input without the supported `[vm]` table follows the existing generic parser failure and cannot reach intent; absent VMM capability retains its existing typed no-driver result; network provision failure remains pre-Running; post-Running intercept failure authors the existing dominating Failed result and withholds VM guest-command release. Old affected storage is reset, not read. |
| Unrelated state | Job/Service kind, Stable/readiness/liveness, WorkloadId/AllocationId meanings, and exact predecessor/successor ordering remain unchanged. |
| Late success | A later VMM availability change cannot turn previously invalid input into committed intent; normal reboot re-runs the existing VMM probe against current V1 state only. A late/stale intercept result cannot reinterpret or rewrite a newer terminal allocation row through a new path; existing action-shim LWW/cleanup behavior remains. |
| Disconnect/reconnect | Current post-cut V1 VM intent/rows reopen through their normal codecs; no pre-cut payload participates in the contract. |
| Duplicate/re-drive | Repeated invalid submissions retain the existing generic parser/no-write behavior; repeated VM evaluations retain P-105 durable-ID and exact-action behavior. No repeated legacy spelling becomes a dedicated compatibility contract. |
| Feature disabled | Not applicable: #293 is a single-cut removal with no feature flag. The pre-removal Exec path is not retained behind configuration. |

No gate changes the meaning of `Running`, `Stable`, readiness, liveness,
`Failed`, `Terminated`, or the accepted P-105 replacement handoff.

## Wave: DESIGN / [REF] C4 Diagram Index

- C4 System Context and Container diagrams ready for independent review:
  `docs/product/architecture/c4-diagrams.md#microvm-only-workload-execution-after-exec-removal-gh-293`
- Component-level C4 is omitted: the feature creates no complex subsystem and
  the L2 diagram shows every changed owner/boundary.

## Wave: DESIGN / [REF] Documentation and Test Migration Contract

- Delete `overdrive-worker` Exec-driver production code and tests in the same
  implementation commit(s); do not move helpers under `#[cfg(test)]`, retain
  compile-fail fixtures, or repurpose tests to defend a removed adapter.
- Delete the host Exec-probe production/sim adapters under approved P-293-4/B1,
  parser/error branches, and tests in the same discipline; do not preserve a
  rejected-only `type = "exec"` compatibility fixture.
- Migrate generic lifecycle, store, parser, renderer, and simulation fixtures
  from Exec to the existing VM/`SimDriver(DriverType::Vm)` shape when the
  behavior under test is driver-neutral. Do not delete cross-cutting evidence
  merely because Exec was its cheap fixture.
- Delete `WorkloadSpecInput::exec_command` without replacement. In its sole
  `coinflip_migration` test consumer, delete only the command assertion and
  retain the migrated VM example's driver-neutral kind/ID assertions.
- Do not add a dedicated retired-driver `ParseError` variant or message, a
  special `[exec]` presence walk, or an acceptance matrix that makes the
  deleted spelling a permanent compatibility rule. Dedicated Exec parser,
  driver, probe, codec, adapter, accessor, and source-shape tests delete with
  their production symbols. No dedicated acceptance test, trybuild fixture,
  compile-pass fixture, source-token scan, or rejection case may exist solely
  to mention or detect a deleted symbol, syntax, helper, parameter, alias,
  message, variant, adapter, accessor, historical payload, or semantic rename
  of any of them. Deletion is checked in implementation/reviewer diff audit,
  not preserved as executable vocabulary.
- Narrow surviving evidence covers only successful `[vm]` parsing, ordinary
  generic missing/invalid/unknown-driver behavior with non-legacy-specific
  input, no intent write after that ordinary failure, registry `∅ | {Vm}`,
  HTTP/TCP probe-runner behavior, the four current incompatible V1 roundtrips,
  surviving VM/generic lifecycle vocabulary, P-105 behavior, and the existing
  VM production composition/effects already warranted by those contracts.
- The private cleanup renames to `AllocationAttemptEvent::Dispatch` and
  `guest_command_release_permitted` receive no dedicated compile/source-shape
  test. Their surviving semantics are covered only through the existing P-105
  preflight behavior and VM Running → intercept → guest-command ordering.
- Delete old fixtures for the four affected envelopes and replace each owner
  with only its new post-cut V1 fixture. Do not keep old bytes as negative
  compatibility tests. Unaffected envelope fixtures remain byte-identical.
- Delete operator examples whose outcome is specifically host-process Exec.
  Migrate general workload journeys/examples to existing VM artifact
  preparation and `[vm]` without recreating inline specs in verification
  expectations.
- Update current `README.md`, whitepaper, architecture brief, product jobs,
  product journeys, outcome registry, OpenAPI, crate/module rustdoc, and current
  examples to name the microVM family as supported. Historical ADRs and
  evolution records remain historical and are not rewritten to pretend Exec
  never existed.
- A documentation or test reference to the VM beacon message `EXEC` must name
  that precise guest command concept; it is not evidence of the removed
  workload driver or health-probe mechanic.

## Wave: DESIGN / [REF] #295 Handoff and Non-goals

Everything in GH #293 under **Required handoff to #295** is a constraint on
the state #293 leaves behind, not implementation scope for #293.

#293 must **not**:

- create or select a shared L2 switch/vswitch;
- delete the current VM netns, veth, TAP-in-netns, transit/guest `/30`,
  `NetSlot`, setns, route, or resolver mechanisms as #295's replacement cut;
- add per-tap/per-port classification or interception;
- move dial-by-name DNS to a shared bridge;
- redesign transparent mTLS, consolidate listeners, or claim cross-host,
  density, or throughput results;
- alter guest IP/MAC ownership on the future shared subnet; or
- absorb public-ingress gateway compatibility analysis owned by #295.

#293 must leave no Exec branch that #295 would need to preserve. The current VM
network fields and components remain explicitly temporary, VM-used inputs and
are named as #295 deletion/replacement targets, not permanent driver-neutral
abstractions.

## Wave: DESIGN / [REF] Outcome Collision Candidates

- **OUT-EXEC-REMOVAL-ADMISSION:** newly accepted workload intent is VM/microVM
  driver-only; unsupported or missing driver input follows the existing generic
  parse/decode failure before intent commit. This supersedes the
  Exec-driver half of `OUT-SVM-SERVICE-ADMISSION` and also supersedes that
  outcome's VM-Exec-probe rejection clause under approved P-293-4/B1.
- **OUT-EXEC-REMOVAL-FORWARD-SCHEMA:** exactly four Exec-coupled envelope owners
  reset to incompatible VM-only/current V1 shapes; no older payload is read,
  translated, rendered, or executed, and unrelated envelopes do not reset.
- **OUT-EXEC-REMOVAL-PROBES:** current Service
  health admits only HTTP/TCP mechanics; host Exec probing is absent and any
  future in-guest command probe belongs to GH #280's separately designed
  boundary.
- **OUT-EXEC-REMOVAL-LIFECYCLE-PRESERVATION:** removing Exec does not alter the
  driver-neutral predecessor/fresh-successor P-105 contract.

`nwave-ai outcomes check-delta` exited 0 and printed
`5 outcomes checked, 0 collisions found across 0 outcomes`. The zero registry
population remains the installed-CLI/schema-resolution defect and is not a
meaningful collision signal. Manual resolution is recorded in
`docs/product/outcomes/registry.yaml`: `OUT-SVM-SERVICE-ADMISSION` is
superseded by `OUT-EXEC-REMOVAL-ADMISSION`, the surviving VM-only target
projection drops its Exec clause and relates to the new probe outcome, and all
four approved #293 outcomes are registered. The independent reviewer must
validate that manual resolution rather than treating the broken checker as
proof.

## Wave: DESIGN / [REF] Changed Assumptions

| Accepted/current source | Superseded assumption | User-approved replacement |
|---|---|---|
| `docs/product/architecture/adr-0030-exec-driver-and-allocation-spec-args.md` | “Exec” is an active workload driver and operator vocabulary. | Historical record only; no active Exec parser, payload, adapter, or composition. |
| `docs/product/architecture/adr-0031-job-spec-exec-block.md` | `[exec]` is an admitted workload-driver table. | `[vm]` is the sole live table; tagged unions remain for future microVM-family adapters. |
| `docs/product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md` | Production always inserts Exec and may also insert VM. | Production inserts only available/probed VM-family drivers; registry and routing ownership remain. |
| `docs/product/architecture/adr-0054-probe-runner-subsystem.md` | ProbeRunner has TCP, HTTP, and host Exec mechanics/ports/adapters. | ADR-0113 supersedes only the Exec-specific clauses; HTTP/TCP tasks, rows, supervision, cancellation, roles/thresholds, and Earned-Trust TCP probing remain. |
| `docs/product/architecture/adr-0059-exec-probe-cgroup-placement.md` | Host Exec probes spawn and join the workload cgroup. | ADR-0113 supersedes this host Exec-probe decision; no replacement probe or cgroup-placement compatibility path remains. |
| `docs/product/journeys/run-a-vm-workload.yaml` | “the same file with one table swapped” from `[exec]` to `[vm]`. | `[vm]` is the normal supported appliance model, not an alternative to an active Exec baseline. |
| `docs/product/journeys/submit-a-service.yaml` | Service may be backed by `[exec]` or `[vm]`, and the host Exec driver owns successful Exec probes. | Service execution is VM/microVM-backed and current probes are HTTP/TCP. GH #280 owns any future in-guest command-probe contract from a clean boundary. |
| `docs/feature/vm-recreation-allocation-id-reuse/feature-delta.md` | Generic replacement applies to legacy Exec until GH #293 removes it. | That temporary clause is discharged; the generic identity contract remains unchanged for the surviving family. |

These are downstream/upstream documentation changes, not changes to the GH
#293 requirements contract. They become SSOT edits only after user approval.

## Wave: DESIGN / [REF] Open Questions

None. The user approved P-293-1 through P-293-6 on 2026-09-14, and the
corrected bundle's overall iteration-4 review recorded `APPROVED` in
`design/review-design.md`. The corrected contract adds no compatibility error,
message, branch, reader, fixture, or deletion/absence test obligation.

## Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDR | Impact |
|---|---|---|---|
| GH #293 + P-293-1 | Make VM/microVM the sole live workload execution family while retaining registry/index/observer routing. | ADR-0110 | Existing VM capability and production composition remain the only successful execution path. |
| P-293-2 | Reset only Service-spec and workload-intent envelopes to incompatible current V1. | ADR-0111 | Each owner keeps one current VM-only roundtrip fixture and no historical reader. |
| P-293-3 | Reset only allocation-status and lifecycle-occurrence envelopes to incompatible current V1. | ADR-0112 | Surviving VM/generic lifecycle evidence remains; unrelated envelopes stay unchanged. |
| P-293-4 | Retain only HTTP/TCP health probing and leave any future guest command probe to GH #280. | ADR-0113 | Existing HTTP/TCP registration, task, cancellation, target, and observation behavior remains. |
| P-293-5 | Preserve accepted-predecessor/distinct-durably-reserved-successor P-105 semantics exactly. | ADR-0105/0106/0108/0109 | Driver removal changes no identity, publication, retry, precedence, or exact-old cleanup rule. |
| P-293-6 | Treat GH #295 as constraint-only. | n/a | No shared switch, per-tap interception, shared DNS, or replacement mTLS mechanism enters #293. |

## Wave: DISTILL / [REF] Consultation and reconciliation

- Corrected DESIGN final verdict: `APPROVED` after overall iteration 4.
- GH #293 and the approved feature delta/ADR-0110..0113 contract were read.
- Relevant existing journeys were consulted only to locate VM behavior locks;
  no #293 journey was invented.
- Feature DISCUSS, SPIKE, and DEVOPS artifacts remain missing warnings.
- Deliverable type resolves to `application`; language is Rust; project ATDD
  policy is inherited and forbids `.feature`/Python test machinery.
- `docs/product/kpi-contracts.yaml` is docs-platform-only; no #293 KPI test is
  added.
- Reconciliation result: **0 contradictions**.
- Documentation density remains lean. The named resolver/telemetry helper is
  absent, so no density telemetry event is invented.

## Wave: DISTILL / [REF] Minimum executable contracts

The authoritative concise specification and complete evidence map are in
`distill/test-scenarios.md`. Five surviving contracts remain:

1. Valid VM Job/Service/Schedule parser and wire projections round-trip and
   production composition routes VM allocations through the probed capability.
2. No-supported-driver and arbitrary unsupported-driver input follows existing
   generic parser/serde/validation behavior with no intent write.
3. Exactly four named current payloads establish new incompatible V1 roundtrip
   fixtures; no historical negative fixture and no unrelated reset exists.
4. Registry `∅ | {Vm}`, HTTP/TCP probe lifecycle, per-driver VM exit
   observation, and current Running → intercept → guest-command ordering remain.
5. P-105 predecessor/fresh-successor, durable reservation, publication,
   precedence, and exact-old cleanup semantics remain unchanged.

No executable test may exist solely to mention or detect a deleted name,
syntax, helper, parameter, alias, message, variant, adapter, accessor,
historical payload, or semantic rename. Deletion and the GH #295 boundary are
implementation/reviewer diff audits.

## Wave: DISTILL / [REF] Existing-test reuse and transition

| Contract | Existing evidence/disposition |
|---|---|
| VM parser/wire | Retain/transition `service_kind_vm_workloads.rs` positive VM parse, `api_type_shapes.rs` VM request/describe roundtrips, `coinflip_migration.rs` kind/ID, and the existing VM Job/Schedule CLI integration tests. |
| Generic failure/no write | Transitioned `workload_spec_parser.rs::s_01_07_missing_supported_driver_rejected`; new generic `api_type_shapes.rs::submit_request_rejects_unknown_driver_keys`; transitioned production `submit_round_trip.rs` invalid VM request now asserts the canonical intent key is absent. |
| Registry/composition | Retain `vm_walking_skeleton.rs` capability-present/absent/probe-refusal tests and VM production paths; narrow the existing trusted-runner composition test to its single VM assertion. |
| HTTP/TCP probes | Retain the existing worker HTTP/TCP outcome, supervision, VM target, lifecycle-hook, and idempotent re-registration suites; delete only dedicated disappearing-mechanic tests. |
| Four V1 codecs | Rewrite existing `schema_evolution/{service_spec,workload_intent,alloc_status_row,alloc_lifecycle_occurrence_row}.rs` modules in place to one current V1 fixture each; delete historical fixtures/tests. |
| VM/generic lifecycle | Transition existing row, reason, renderer, and observation generators to surviving variants. |
| P-105 | Narrow existing reconciler and composed action-shim suites to VM without weakening their universes or assertions. |
| VM effect order | Transition existing `mtls_install_fail_closed.rs` fixtures to VM; retain Running, dominating Failed, release withholding, and cleanup assertions. |
| #295 | No executable test; reviewer audit only. |

## Wave: DISTILL / [REF] Test delta and placement

- New feature-specific test files: **0**.
- New test functions: **1** generic unsupported-driver-key property inside the
  existing `api_type_shapes.rs` module.
- Modified existing test functions: **4** (generic missing-driver, two VM wire
  roundtrips, and production invalid-submit/no-write).
- Rejected new `remove_*.rs` files deleted: **8**, including all compile-pass
  fixtures; module registrations removed.
- Pending/ignored feature scaffolds: **0**.

All tests remain in existing Rust acceptance/integration modules at their
owning ports. The property samples a bounded generated representative domain:
lowercase `driver_<suffix>` keys whose suffix length is 1..12. Every case first
proves the unchanged VM request deserializes, then inserts one unsupported
sibling key and asserts ordinary generic serde rejection. Finite VM, VMM,
probe, codec, and lifecycle cases reuse the existing example suites.

## Wave: DISTILL / [REF] Driving and adapter coverage

| Boundary | Evidence |
|---|---|
| Parser / wire | VM positive roundtrips, generic missing-driver test, generic unsupported-key property. |
| Production HTTP submit | Existing real `run_server`/HTTP/redb integration test proves ordinary invalid VM input writes no intent. |
| Production VM composition | Existing `vm_walking_skeleton` and `vm_boot_failure_vocabulary` tests drive real in-process serve/deploy/describe on the qualified native-metal lane. |
| Driver registry / VMM | Existing present, absent, and probe-refusal production cases. |
| HTTP/TCP probers | Existing worker/controller suites; no new adapter exists. |
| Intent/observation codecs | Existing four schema-evolution modules are the transition sites. |
| Action shim / mTLS / network | Existing P-105 and fail-closed suites; real host effects remain Tier 3. |

Exact internal exit-observer task-vector cardinality and shutdown-token identity
are not port-exposed. Existing VM completion, supervision-release, and clean
shutdown tests prove the behavioral boundary; exact one-task-per-entry,
clone-one-token, and join-all ownership remain reviewer diff checks. DISTILL
adds no test-only accessor or composition seam.

## Wave: DISTILL / [REF] Post-cut black-box event

E06 and E08 are historical SHA-pinned receipts only. The post-cut event is
named `E14-vm-service-post-greenfield-cut`. DELIVER/DEVOPS captures it at the
new SHA by running the checked-in
`examples/service-kind-vm-workloads/run-example.sh run healthy` through the
built default-feature binary on native metal. It records only the example's
stakeholder-visible success and owned cleanup outcomes, does not overwrite E08,
and does not duplicate integration assertions. Generic invalid-input behavior
stays in Rust tests and is not duplicated into E14.

## Wave: DISTILL / [REF] Outcome traceability and completeness

| Outcome | Evidence |
|---|---|
| `OUT-EXEC-REMOVAL-ADMISSION` | VM positive parser/wire/production evidence plus generic missing/unknown/no-write behavior. |
| `OUT-EXEC-REMOVAL-FORWARD-SCHEMA` | Four rewritten current V1 roundtrip modules. |
| `OUT-EXEC-REMOVAL-PROBES` | Existing HTTP/TCP parser, runner, supervision, target, and result-row evidence. |
| `OUT-EXEC-REMOVAL-LIFECYCLE-PRESERVATION` | Existing deterministic P-105 pure properties and composed precedence cases. The pre-existing schedule-racy oracle below is excluded from #293 evidence. |

Canonical completeness: **15/15 — COMPLETE**. C7b is N/A because this pure
deletion adds no interruption behavior. Details and run IDs are in
`distill/red-classification.md`.

Pre-existing non-blocking fixture: mandatory review ran
`driver_neutral_allocation_replacement::successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver`
50 times and observed 39 passes / 11 failures. Its printed seed controls model
data, not the unbiased Tokio `select!` between already-ready notifications, so
it is not reproducible seeded production behavior. DISTILL classifies it as an
out-of-scope pre-existing flaky fixture, leaves it untouched, and does not use
it as a #293 RED or completeness blocker. The deterministic P-105 suite remains
green.

Mutation testing was not run in DISTILL.
