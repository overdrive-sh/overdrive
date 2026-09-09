# DESIGN wave decisions — `service-kind-vm-workloads`

**Wave:** DESIGN | **Scope:** APPLICATION / components | **Mode:** guided
| **Architect:** Morgan | **Date:** 2026-09-06

**Active feature boundary:** GH #257 adds Service-kind VM workloads with
host-originated HTTP/TCP probes. VM Exec probes are rejected before intent
commit and remain deferred to GH #280. No VM Exec control protocol, codec,
persistent guest supervisor, session/reconnect protocol, or guest process
containment mechanism is part of this design.

**Focused amendment:** APPLICATION / component scope, PROPOSE mode,
user-authorized on 2026-09-07. ADR-0094 addresses only the proven private TCP
socket self-interception path; it adds no component or C4 relationship, so the
existing C4 topology remains unchanged.

## Decisions

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
| Omitted HTTP host or `0.0.0.0` wildcard | VM | The same allocation's provisioned guest `workload_addr` |
| Omitted HTTP host or `0.0.0.0` wildcard | Exec/process with a provisioned allocation network | The same allocation's provisioned transit `workload_addr` |
| Omitted HTTP host or `0.0.0.0` wildcard | Exec/process without a provisioned allocation network | `127.0.0.1`, preserving current behavior |

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

`ServiceSpec` and `ServiceSpecLatest` both alias the unboxed
`ServiceSpecV3`. Its exact fields remain `id`, `replicas`, parser-side
`DriverInput` `driver`, `ResourcesInput` `resources`, `listeners`, and the
three probe vectors (startup, readiness, liveness). The greenfield envelope has
exactly one direct arm: `V3(ServiceSpecV3)`. Its sole rkyv tag is `0`, so
`known_discriminants()` is exactly `[0]`; `latest` constructs that direct arm
and `into_latest` returns its payload directly. `V3` remains the existing
current payload type name, not a retained numeric archive tag.

### ACD-2 greenfield persistence amendment — proposed 2026-09-06

The user explicitly authorizes replacing the legacy ServiceSpec persistence
contract because nobody uses its old persisted versions. This supersedes the
temporary boxed-V3 archival-compatibility amendment and still requires
independent DESIGN review before step 01-01 resumes. It changes no operator,
parser, wire, intent, allocation, describe, probe, or lifecycle behavior.

`ServiceSpecV1`, `ServiceSpecV2`, their envelope arms and re-exports, their
upgrade conversions, V1/V2 frozen fixtures, legacy decode, re-archive, and
migration obligations are deleted. There is no `Box<ServiceSpecV3>` at any
parser or codec boundary. The only schema evidence is one current direct V3
fixture written via `latest`, decoded to the exact latest payload, and
round-tripped to the same bytes. Retired V1/V2 bytes are unsupported; no
fallback decoder, compatibility prefix, migration, second envelope, or public
accessor is permitted.

`SectionPresence::validated` applies the already-existing exactly-one-of
`[exec]`/`[vm]` rule uniformly to Service, Job, and Schedule. The
`VmNotAllowedOnServiceKind` rejection and its now-stale GH #257/#222 guidance
are removed. The Service parse branch selects the declared parser driver and
stores it on `ServiceSpecV3`. No ServiceSpec compatibility accessor is
introduced; existing call sites use the selected driver union directly.

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

### ACD-4 — Proposed bounded HTTP socket-mark connector amendment

**Status:** proposed under user authorization; independent DESIGN review must
approve ADR-0092 before the original `02-01` crafter resumes.

**Decision:** retain ADR-0090's exact target projection and the public
`HttpProber::probe(url, timeout)` / `HyperHttpProber::new()` shapes. Extend
only the production `HyperHttpProber` adapter with a private
`tower_service::Service<hyper::Uri>` connector. For every resolved,
non-loopback HTTP candidate, it creates the matching Tokio `TcpSocket`, stamps
the already-existing `MTLS_LEG_S_DIAL_MARK` with `SO_MARK` before `connect(2)`,
then returns that stream to the existing Hyper client. Loopback stays unmarked.

This is deliberately adapter-wide for non-loopback HTTP sockets. The unchanged
port passes a URL, not VM-origin metadata; adding metadata, a second method,
or a persisted descriptor field would be public-surface divergence. The mark
does not rewrite the explicit URL or create a route: it only selects the
existing first OUTPUT exemption when the destination matches an installed
per-workload divert. Destinations without that divert retain normal routing.
HTTP status/redirect/body policy, TCP behavior, and Exec behavior remain
unchanged.

**Manifest contract:** root `[workspace.dependencies]` gains only
`tower-service = "0.3.3"`; `overdrive-worker` uses only
`tower-service.workspace = true`. The crate is already locked transitively.
No leaf pin, other dependency, crate, feature, daemon, protocol, persisted
row, CLI/API, route, or lifecycle state is permitted.

**Why:** native-metal E08 reached the actual owner path and showed the
projected guest readiness GET fail as Hyper `SendRequest`; the worker OUTPUT
divert accepts the same pre-existing mark before its per-workload rule. A
marked direct production GET to the same guest returned 204, and the
provisional private connector made E08's readiness row pass. ADR-0092 records
the exact bounded contract; its provisional code is evidence, not an accepted
implementation.

**Evidence handoff:** preserve existing HTTP outcome coverage and add an
adapter-level pre-connect mark/loopback-nonmark proof without changing the
public trait. Re-run E08 using the built default-feature binary on qualified
native metal; it must show the production projected readiness HTTP 204 as
`last=pass` without harness-installed routing, listener, target, or mark.
E08's separate Stable wait is not evidence for, or a change to, this adapter
decision.

### ACD-5 — Proposed streaming Service Accepted-before-Stable renderer amendment

**Status:** proposed under user authorization; independent DESIGN review must
approve ADR-0093 before the original `02-02` crafter resumes.

**Decision:** extend only the existing streaming Service CLI consumer's
private summary assembly. When its existing ordered `Accepted` then `Stable`
events complete successfully, its existing `DeployStreamingOutput.summary`
contains the exact existing Accepted acknowledgement block followed by the
exact existing Stable detail, once each and in order. The existing binary
wrapper prints that one summary. It does not add a callback, output type,
event, command, flag, protocol, route, or background output task.

**Boundary:** detached/non-TTY acknowledgement behavior is unchanged. So are
pre-Accepted failures, `Failed`/`Stopped` terminal output, stream-close or
body-decode errors, cancellation, exit codes, Service lifecycle ownership,
probe behavior, persistence, and production composition. The amendment is
only the successful un-detached streaming Service visible output.

**Why:** E08's accepted product oracle requires one fresh un-detached Service
deploy transcript to order Accepted before Stable. The actual control-plane
stream already does so, but `consume_stream` currently stores the Accepted
fields and renders only the later Stable detail. Combining a detached command
with a second idempotent stream is evidence fabrication, not product behavior.

**Evidence handoff:** add one direct ordered Service-stream/render regression
that proves exact multiplicity and order, then run E08 with exactly one fresh
PTY-backed Service deploy and assert the pair within that transcript. No shell
rendering or multi-command concatenation is accepted.

### ACD-6 — Proposed bounded TCP socket-mark adapter amendment

**Status:** proposed under user authorization; independent DESIGN review must
approve ADR-0094 before the original `02-03` crafter resumes.

**Decision:** retain the exact public `TcpProber::probe(host, port, timeout)`
and `TokioTcpProber::new()` shapes, ADR-0090 target projection, and every
existing TCP timeout/error/result string. Extend only the private production
`TokioTcpProber` connection path: for each resolved candidate, create the
matching Tokio socket, stamp the already-existing `MTLS_LEG_S_DIAL_MARK` with
`SO_MARK` before `connect(2)` if it is non-loopback, and leave loopback
candidates unmarked. Candidate order and existing resolution, immediate-drop,
timeout, refusal, DNS, and generic-I/O semantics remain unchanged.

This is deliberately adapter-wide for non-loopback TCP candidates. The
unchanged port receives only host, port, and timeout; adding VM-origin metadata,
a marked probe method, a descriptor field, or a public socket abstraction
would be divergent surface. The mark neither changes the destination nor adds
a route/rule: it selects the existing first OUTPUT exemption only when an
installed per-workload divert matches. UDP is explicitly out of scope pending
its own real production-path evidence and DESIGN decision.

**Why:** the native-metal E13 `zero-probes-failure.toml` case deliberately left
guest port `18998` unbound, yet the projected inferred TCP probe passed and
the Service reached Stable. The actual owner path reaches the existing worker
OUTPUT TPROXY divert; without the established agent-dial mark its SYN connects
to mTLS leg-C rather than the guest. E09's unbound failure matrix is therefore
untrustworthy until re-executed after this correction. ADR-0092's HTTP decision
is supporting precedent only; it does not itself authorize TCP.

**Evidence handoff:** add direct production-adapter proofs that a non-loopback
TCP socket has exactly the existing mark before connect and a loopback socket
has mark zero. Re-run native-metal E09's 100 paired trials and E13's bound /
unbound inferred pair through only the built default-feature `serve` and
`deploy` paths. No harness-installed mark, route, address, or listener is
accepted.

### ACD-7 — Proposed default streaming-cap envelope for inferred startup failure

**Status:** proposed under user authorization; independent DESIGN review must
approve ADR-0095 before the original `02-03` crafter changes the E13 path.

**Decision:** change only the value of the existing
`DEFAULT_STREAMING_CAP` from 60s to 90s and the existing private CLI streaming
request timeout from 90s to 120s. These are shared defaults for the existing
Job and Service streaming lanes: `AppState::streaming_cap`, both existing cap
futures, the stream enum/projection, and custom injected cap semantics remain
exactly as they are. There is no Service-specific cap, startup-descriptor
lookup, public configuration, field, method, port, retry, persistence,
lifecycle state, or wire change. No operator configuration exists today;
`AppState::streaming_cap` is a construction/test override, not a config-file
or CLI option.

**Why:** after the TCP socket mark correction, native-metal E13 truthfully
reports refusal from the unbound guest port. The inferred descriptor's existing
30 × 2s startup policy derives the same 60s deadline as the standard stream
cap. `ServiceLifecycle` therefore cannot publish its existing
`StartupProbeFailed` terminal before the stream's existing `Timeout` can win.
Ninety seconds preserves a 30-second terminal-delivery envelope, and the
120-second client envelope preserves the existing 30-second client/server
margin.

**Boundary:** `StartupProbeFailed` remains exclusively `ServiceLifecycle`'s
existing terminal after the existing attempts-plus-elapsed condition;
`Timeout` remains exclusively the streaming loop's existing wait result.
`Running`, Stable, eligibility, liveness, restart, explicit/custom stream caps,
and explicit long startup policies retain their existing owners and semantics.
E13 must reject `Timeout` as a substitute for `StartupProbeFailed`.

**Evidence handoff:** add a seeded existing-seam simulation ordering invariant,
default/cap-first Service stream boundary regressions, a Job default-cap
regression that preserves explicit injected caps, and the generic shared-client
120-second envelope regression. Re-run E13 through built `serve` + `deploy`;
the unbound inferred guest listener must emit `StartupProbeFailed`, remain
ineligible, and deny its peer VM Job.

### ACD-8 — Proposed terminal-startup eligibility withdrawal

**Status:** proposed under user authorization; independent DESIGN review must
approve ADR-0096 before DELIVER remediates E09's failed-service peer-traffic
contradiction.

**Decision:** retain the existing `Running` meaning and the existing
`ServiceFailed { StartupProbeFailed { .. } }` terminal. When
`ServiceLifecycle` decides that terminal for an allocation, its existing full
`ServiceBackendRow` must carry that still-Running deciding allocation with
`healthy: false`, ahead of the existing `FinalizeFailed` action in the same
serially-dispatched batch. The existing no-readiness default remains
`healthy: true` only for a Running allocation that has not terminally failed
startup. The existing bridge carries the observed health; it does not gain a
startup classifier.

**Why:** E09's unbound VM listener receives real connection refusal and reaches
the existing terminal, but the current no-readiness shortcut re-advertises the
allocation as healthy in the same tick. The peer VM Job then receives the
guest reply, violating the accepted “failed deployment must not serve its peer”
contract. The reconciler can construct the existing health-withdrawal action
before the existing terminal action. The action shim attempts those actions
serially, but its existing continue-on-error behavior means that construction
order is not a durable terminal-before-withdrawal guarantee when the row write
fails.

**Boundary:** no new lifecycle gate, state, action, row field, storage,
retry/restart mechanism, owner, configuration, API, wire event, route, or
component. `ServiceLifecycle` remains the terminal and health owner;
`WorkloadLifecycle` remains restart authority. Stable, readiness, liveness,
startup policy, target projection, and non-terminal no-readiness compatibility
are unchanged. A replacement allocation has its existing fresh allocation id
and normal lifecycle.

**Evidence handoff:** a reconciler acceptance test proves construction order
(`WriteServiceBackendRow { healthy: false }` before `FinalizeFailed`) and the
non-terminal no-readiness compatibility case. Native-metal E09 proves the
healthy-store product path: failed paired trials deny their VM peers while
healthy pairs still serve; E13's unbound inferred control remains ineligible
and denies its peer after `StartupProbeFailed`. These lanes do not claim a
durable ordering guarantee across an injected backend-row write failure.

### ACD-9 — Proposed E11 readiness wake through the accepted probe-result observation

**Status:** proposed under user authorization; independent DESIGN review must
approve the focused ADR-0101 revision-5 amendment before DELIVER step `03-01`
resumes.

**Decision:** retain `ServiceLifecycle` as the sole readiness-policy and
complete `ServiceBackendRow` owner, but carry an accepted probe-result LWW
winner over the existing lag-aware subscription. Append exactly
`ObservationRow::ProbeResult(ProbeResultRow)` and
`ObservationRowKind::ProbeResult` after the existing `Signal` variants;
`ObservationRow::kind()` maps the new variant and
`ObservationRowKind::as_str()` returns exactly `"probe-result"`. The pair is an
event-only in-process projection: `ProbeResultRow`'s V1 payload, composite
key, redb table and Sim index remain unchanged, and the variant is not added
to `ObservationWrite`, row history or gossip.

`ObservationStore::write_probe_result(row) -> Result<(), ObservationStoreError>`
keeps its signature and emits exactly one existing
`SubscriptionEvent::Row(ObservationRow::ProbeResult(row))` synchronously
after an accepted mutation commits. Stale/equal winners and failed commits
emit nothing; the local adapter's existing unreadable-predecessor repair
remains accepted. `ServiceLifecycleReconciler::interests()` keeps its existing
signature and returns exactly
`&[ObservationRowKind::AllocStatus, ObservationRowKind::ProbeResult]`.
The router resolves a probe event through the existing
`alloc_status_row(&AllocationId)` point read, targeting only a current Service
as `workload/<workload_id>`. Its existing allocation List, lag relist and
30-second relist remain the target-enumeration path; ServiceLifecycle
hydration already reads latest probes through `list_probe_results_for_alloc`.
No new method, cache, target, broker channel, cadence, resync schedule,
persistence or consumer protocol is added.

The complete probe-result family is selected because this one reconciler
hydrates startup, readiness and liveness from the same snapshot and the
existing interest discriminant has no role-level dimension; policy still
filters roles after hydration. Ordering is durable row → existing event →
existing broker evaluation → existing ServiceLifecycle backend action →
existing consumer effects. Probe, router and convergence cancellation stay
cooperative; read/write errors retain existing log-and-continue/relist
behavior, and no error becomes a terminal or restart action.

**Why:** native E11 records a failed readiness row while the Service remains
Running/Stable but its during-window peer Job still receives the guest reply;
seeded Sim seed `25717` reproduces the missing subscription wake. The direct
post-commit event reaches the existing 100 ms convergence loop without
waiting for the 30-second relist, making the existing two-second healthy-store
bound achievable while preserving terminal dominance and all owner/consumer
boundaries. Detailed alternatives, reachability evidence and proof
obligations are in
[`amendment-e11-readiness-wake.md`](amendment-e11-readiness-wake.md).

### ACD-10 — Proposed E11 current Stable observation on the existing snapshot

**Status:** proposed under user authorization; independent DESIGN review must
approve the focused ADR-0101 revision-6 amendment before DELIVER step `03-01`
resumes.

**Decision:** retain the existing `workload describe` command and
`GET /v1/allocs?job=<id>` snapshot, and project the current typed terminal
claim already carried by the durable allocation row. Append exactly this
field, after `last_terminated`, to `AllocStatusRowBody`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub terminal: Option<overdrive_core::transition_reason::TerminalCondition>,
```

The existing handler copies it exactly as `terminal: row.terminal`; no
readiness, state, probe-result or history derivation is permitted. In the
existing `WorkloadKind::Service` renderer, preserve the
`Alloc / State / Restarts / Since` table and emit exactly
`    terminal: Stable` immediately after a row when its current claim is
`Some(TerminalCondition::Stable { .. })`. Emit no line for `None` or any other
claim; Job and Schedule rendering remains unchanged. `rows[i].terminal` is the
current-row claim, while nested `rows[i].last_terminated.terminal` remains the
prior terminal observation the allocation survived.

The field reuses the existing `TerminalCondition` serde/ToSchema type and the
existing row, route, client method and renderer. Stable remains the existing
non-lifecycle-terminal success claim paired with `AllocState::Running`; this
projection does not add a gate, owner, state, route, CLI argument, replay,
aggregate, persistence field, retry, cadence, broker operation, consumer
acknowledgement or recovery mechanism. The seeded production-owner-path
evidence proves the current row retains Stable through readiness Fail/Pass;
the fresh native E11 attempt proves the traffic/readiness predicates while
its public snapshot omits the Stable claim. Detailed source proof,
alternatives, privacy and recapture obligations are in
[`amendment-e11-stable-observation.md`](amendment-e11-stable-observation.md).

## Lifecycle Gate Ownership

**Changed health gate — ADR-0096.** Target projection and Service-driver
admission reuse existing boundaries and add no gate. ADR-0096 changes the
existing `Backend.healthy` gate only: terminal startup failure vetoes
eligibility for its deciding allocation.

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| Allocation `Running` | Action shim + selected `Driver`; VM success is the accepted Beacon/driver-start contract | The allocation driver started and its Running observation was committed | Existing VM provisioning, driver start/Beacon, and intercept-install ordering | Service `Stable`; `Backend.healthy`; liveness verdict |
| Service `Stable` | `ServiceLifecycle` startup branch | Every startup probe passed, or startup probing was explicitly disabled | Existing startup observations and deadline policy | Allocation `Running`; readiness eligibility; liveness policy |
| `Backend.healthy` | `ServiceLifecycle` readiness branch, vetoed by its existing terminal startup failure | A non-terminal backend meets the declared readiness threshold; a terminally startup-failed backend is ineligible | Existing startup terminal decision, then existing readiness observations and counters | Allocation `Running`; Service `Stable`; restart authority |
| Liveness termination | `ServiceLifecycle` liveness detector | The declared liveness threshold was reached; emit only `StopAllocation { terminal: Stopped { by: LivenessProbe } }` | Existing liveness observations and threshold counter | Restart budget/decision; meanings of `Running`, `Stable`, or readiness |
| Restart decision/action | `WorkloadLifecycle` (sole restart authority) | Observe the liveness-terminated allocation row and either emit `RestartAllocation` under the unified budget or finalize failure | Existing `AllocStatusRow.terminal` plus unified restart budget | Probe threshold detection; meanings of `Running`, `Stable`, or readiness |

The changed gate's complete contract is:

| Contract field | ADR-0096 declaration |
|---|---|
| Owner | `ServiceLifecycle` remains the sole owner of the terminal-startup failure decision and the backend-row health projection. |
| Failure projection | Its existing `StartupProbeFailed` decision projects its deciding allocation into the existing full `ServiceBackendRow` with `healthy: false`. |
| Ordering | The reconciler constructs that row action before `FinalizeFailed`; the shim attempts them serially in that order but continues after a row-write error. Thus a successful row write withdraws eligibility before the following terminal publication, while a failed write has no new durable ordering guarantee. |
| Unaffected owners/states | `Running`, `Stable`, startup and liveness policy, and `WorkloadLifecycle` restart authority remain unchanged. |
| Retained counterexamples | A non-terminal no-readiness allocation remains healthy; a late probe cannot restore a terminally startup-failed allocation; liveness/restart follows its existing owner path; and a fresh replacement uses its new allocation id and normal lifecycle. |
| Evidence | Reconciler tests cover false-row action construction/order and the retained cases. E09/E13 prove the healthy-store production path, not a row-write-failure guarantee. |

Boundary obligations for DISTILL/DELIVER:

1. A VM reaches `Running` before its first scheduled probe; a closed guest port
   yields a failed startup observation without revoking or delaying `Running`.
2. Startup success advances only Service `Stable`; startup terminal failure
   withdraws backend eligibility without redefining `Running`.
3. Readiness fail/pass changes only backend eligibility for an allocation that
   has not terminally failed startup.
4. Liveness threshold satisfaction makes `ServiceLifecycle` emit only the
   existing liveness `StopAllocation`; `WorkloadLifecycle` alone decides
   restart versus finalization under the unified budget. No VM-specific policy
   is added.
5. Terminal state wins over any late probe success; no dead backend is restored.
6. An allocation-network Exec-backed Service's default/wildcard network probe
   reaches its own provisioned transit address; an unnetworked Exec default and
   every explicit host retain their prior semantics and lifecycle behavior.
7. A VM Service with an Exec probe is rejected before intent commit and names
   GH #280; HTTP/TCP VM Services continue through the same deploy surface.

## Reuse Analysis

| Existing component | Path | Overlap | Decision | Contract shape and assertion |
|---|---|---|---|---|
| `WorkloadSpecInput` + `ServiceSpecEnvelope` | `crates/overdrive-core/src/aggregate/{workload_spec,service_spec}.rs` | TOML discrimination and greenfield Service parser payload | EXTEND | Pure parser projection over one TOML document; one direct `V3(ServiceSpecV3)` envelope arm with tag `[0]`; one current fixture proves direct round-trip, with no legacy compatibility surface |
| `ServiceV2::from_submit` / `to_describe` | `crates/overdrive-core/src/aggregate/mod.rs` | Authoritative admission and describe round-trip | EXTEND | Pure-function over one Service payload; cross-field rejection properties and driver-union round-trip |
| Existing `DriverInput` / `WorkloadDriverV2` unions | `crates/overdrive-core/src/{api/submit.rs,aggregate/mod.rs}` | VM/Exec representation already exists downstream | REUSE | Pure tagged-union projection; existing schema fixtures plus both-arm round-trip properties |
| Service CLI deploy lanes (`deploy_service`, `deploy_streaming_service`) | `crates/overdrive-cli/src/commands/deploy.rs` | Both currently collapse parser Service input to Exec | EXTEND | Bounded-change universe: one parsed Service projected to one `ServiceSpecInput` in either existing lane; exact delta: replace the Exec-only projection with the selected existing driver-union arm; assertions: both lanes preserve the same arm field-for-field while retaining their existing HTTP/output behavior |
| Service streaming renderer (`consume_stream` + existing render functions) | `crates/overdrive-cli/src/{commands/deploy.rs,render.rs}` | Existing consumer retains Accepted fields but returns only terminal Stable detail | EXTEND | Bounded-change universe: one successful `Accepted -> Stable` Service stream and its existing `DeployStreamingOutput.summary`; reuse exact acknowledgement and Stable renderers once each, asserting order/multiplicity without a new public output surface |
| `ProbeRunner` | `crates/overdrive-worker/src/probe_runner/mod.rs` | Schedules all three roles and dispatches HTTP/TCP mechanics | EXTEND | Bounded-change: one allocation supervisor and its probe-result rows; ADR-0097 projects allocation-network Exec defaults/wildcards to the existing transit address, while explicit/unnetworked behavior remains pinned |
| `HyperHttpProber` | `crates/overdrive-worker/src/probe_runner/http_prober.rs` | Owns host-side HTTP socket construction beneath the unchanged `HttpProber` port | EXTEND | Bounded-change: one transient socket per resolved HTTP candidate; `SO_MARK` before non-loopback connect, no loopback mark, no retained state; adapter and native-metal E08 evidence |
| `TokioTcpProber` | `crates/overdrive-worker/src/probe_runner/tcp_prober.rs` | Owns host-side TCP socket construction beneath the unchanged `TcpProber` port | EXTEND | Bounded-change: one transient socket per resolved TCP candidate; existing `SO_MARK` before non-loopback connect, loopback mark `0`, candidate/result semantics unchanged; direct adapter tests plus native-metal E09/E13 evidence |
| Shared Job/Service streaming cap + CLI request envelope | `crates/overdrive-control-plane/src/{lib.rs,streaming.rs}`; `crates/overdrive-cli/src/http_client.rs` | Existing shared default wait and client envelope conceal the Service lifecycle terminal when both expire at 60s | EXTEND | Change only shared existing default values to 90s and 120s; existing cap futures, wire terminals, lifecycle ownership, construction/test overrides, and custom caps remain unchanged; no operator configuration exists |
| Existing mTLS OUTPUT exemption + mark SSOT | `crates/overdrive-worker/src/mtls_intercept.rs`; `crates/overdrive-core/src/dataplane/mtls_mark.rs` | Already excludes trusted marked dials from self-interception | REUSE | Existing kernel rule and constant are read-only dependencies; verify E08 drives the installed production path, with no new rule/route/mark |
| `VmDriver` | `crates/overdrive-worker/src/vm_driver.rs` | Owns VM lifecycle but does not yet register probes | EXTEND | Bounded-change: the addressed VM allocation's supervisor hooks only; lifecycle hook tests |
| Production composition (`run_server`, `compose_production_driver`, `compose_vm_driver`) | `crates/overdrive-control-plane/src/lib.rs` | Already owns the single trusted `ProbeRunner` and optional VM driver | EXTEND | Bounded-change universe: one server boot and its driver registry; exact delta: retain the runner returned beside `ExecDriver` and pass an `Arc` clone into the optional `VmDriver`; assertion: one Earned-Trust runner services lifecycle hooks from both registered drivers, with capability-absence/refusal behavior unchanged |
| Action-shim VM network injection | `crates/overdrive-control-plane/src/action_shim/mod.rs` | Produces the guest `workload_addr` before `Running` | REUSE | Bounded-change over the one allocation spec and owned network resources; existing provision-before-start evidence plus native-metal H6 |
| `ServiceLifecycle` | `crates/overdrive-reconcilers/src/service_lifecycle.rs` | Already owns startup/readiness plus liveness detection/termination | EXTEND | Reuse existing startup attempt and last-failure timestamp view inputs so one LWW Startup result increments once; the existing terminal predicate and all owners remain unchanged |
| Existing allocation snapshot + Service renderer | `crates/overdrive-control-plane/src/{api,handlers}.rs`; `crates/overdrive-cli/src/render.rs` | Current durable `TerminalCondition::Stable` survives readiness transitions but is absent from `workload describe` | EXTEND | Proposed revision 6 appends `AllocStatusRowBody.terminal` as a mechanical current-row projection and renders only `terminal: Stable` for Service rows; no route, aggregate, history substitution or lifecycle effect |
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
  VM path. ADR-0092 proposes only the already-locked `tower-service` 0.3.3
  connector-contract dependency at the workspace boundary; no daemon,
  transport, protocol, or other dependency is selected. ADR-0094 uses only
  existing worker/Tokio/Linux socket capability and adds no dependency.

## Changed assumptions

- **Superseded contract:** ADR-0083's `[service] + [vm]` admission rejection was
  correct before GH #222 delivered the guest network/intercept path. It is now
  narrowed to reject only VM Services containing Exec probes; HTTP/TCP probes
  are admitted by GH #257.
- **Superseded default-target assumption:** ADR-0054/0058's loopback default
  remains correct only for unnetworked Exec/process allocations. Under
  ADR-0097, omitted/wildcard VM targets use the provisioned guest address and
  allocation-network Exec targets use the existing provisioned transit
  `workload_addr`; explicit hosts remain unchanged.
- **Startup-observation accounting correction:** the existing startup counter
  was incremented per reconcile while a single latest failed result remained
  unchanged. ADR-0097 reuses the existing persisted last-counted timestamp
  input so one LWW Startup result contributes once; it changes no deadline,
  retry, terminal, store, or lifecycle policy.
- **Rejected speculative assumption:** representability of
  `AllocationSpec { driver: Vm, workload_addr: None }` does not make it a
  production state. The action-shim owner path proves it is absent before probe
  registration, so no behavior is designed around it.
- **Superseded archival-compatibility amendment:** the temporary boxed V3 arm
  protected V1/V2 archived bytes after a direct V3 arm repadded their common
  rkyv root. The user-authorized greenfield assumption retires those bytes, so
  the accepted replacement is one direct `V3(ServiceSpecV3)` arm at tag `0`
  and no V1/V2 compatibility surface.
- **Superseded adapter assumption:** “no new dependency, daemon, transport, or
  protocol” in this file's pre-amendment technology stack excluded a connector
  contract required by the real production HTTP path. The replacement is
  narrowly `tower-service` 0.3.3 at the workspace boundary for ADR-0092's
  private connector; all public, persisted, lifecycle, and dataplane
  boundaries remain unchanged.
- **Superseded streaming-render assumption:** the existing streaming Service
  consumer's terminal-only human summary hid its already-consumed Accepted
  acknowledgement. For successful un-detached Service streams, ADR-0093
  replaces that with the exact existing Accepted block followed by the exact
  existing Stable detail. This is not a new wire event or CLI surface.
- **Superseded TCP adapter assumption:** resolving a VM TCP target correctly
  was treated as sufficient for a truthful connection result. Native-metal E13
  disproved that: a non-loopback host SYN still follows the existing worker
  OUTPUT divert unless it bears the existing agent-dial exemption mark before
  connect. ADR-0094 narrowly applies that private socket effect; it does not
  change target projection, TCP result semantics, HTTP, UDP, or dataplane
  ownership.
- **Current Stable observation boundary — proposed ADR-0101 revision 6:** the
  existing allocation row already carries the typed `TerminalCondition::Stable`
  claim after startup, and seeded E11 composition asserts that readiness
  Fail/Pass preserves it while the same allocation remains Running with zero
  restarts. The public snapshot and Service renderer omit that current claim;
  the amendment appends only `AllocStatusRowBody.terminal` copied from the row
  and the exact `terminal: Stable` line. No readiness-derived boolean,
  `last_terminated` substitution, route, replay, lifecycle state, owner,
  retry, cadence or consumer protocol is introduced. Independent DESIGN review
  is required before step 03-01 resumes.
- **Streaming-cap boundary:** E13's now-truthful unbound TCP path established
  that the shared existing 60s Job/Service streaming cap collides with the
  existing inferred 60s Service startup deadline. ADR-0095 changes only the
  shared default server/client durations to 90s/120s; it does not authorize a
  new waiting policy, configuration surface, terminal mapping, or lifecycle
  effect. `AppState::streaming_cap` remains a construction/test override; it
  is not an operator setting.
- **Startup-terminal eligibility boundary:** E09 proves that the existing
  no-readiness `healthy: true` default cannot override a decided
  `StartupProbeFailed`. ADR-0096 makes that existing terminal an eligibility
  veto and constructs its existing unhealthy backend-row action before the
  terminal action. The current shim may continue after a row-write error, so
  this does not promise durable ordering on that error path; Running,
  non-terminal no-readiness behavior, lifecycle owners, persistence, and
  restart policy remain unchanged.

## Open questions

Proposed ADR-0101 revision 6 (the E11 current Stable-observation amendment)
requires an independent DESIGN review at
`design/review-amendment-e11-stable-observation.md` before step `03-01`
implementation resumes. Its implementation obligation is a fresh native E11
recapture whose existing three `workload describe` snapshots and ledger carry
the direct current `terminal: Stable` observation, followed by an independent
evidence audit. This amendment does not alter the roadmap or execution log.

ADR-0092, ADR-0093, ADR-0094, ADR-0095, and ADR-0096 are delivery blockers pending
independent DESIGN review; none is an open product or API question. In-guest Exec probe
mechanics and any future UDP host probe are independently scoped and do not
constrain this design. The ACD-2 greenfield persistence amendment remains
subject to its stated review gate.
