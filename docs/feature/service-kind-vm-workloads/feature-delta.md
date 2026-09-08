# Feature delta — `service-kind-vm-workloads`

DISCUSS requirements for GH #257. Documentation density is `lean`; this file
contains Tier-1 `[REF]` sections only. The implementation baseline and
comparative evidence for the deferred VM Exec capability are preserved in
[`vm-exec-health-probes-comprehensive-research.md`](../../research/virtualization/vm-exec-health-probes-comprehensive-research.md)
and GH #280.

The current rejection of `[vm]` plus `[service]` is an implemented admission
rule. GH #42 delivered VM Jobs/Schedules and GH #222 delivered the routed
guest network plus guest-stack mesh intercept; this feature closes the remaining
HTTP/TCP Service-health path. In-guest Exec probes are a separate optional
capability tracked by GH #280.

## Wave: DISCUSS / [REF] Persona ID

`ana-platform-engineer` — Ana Moreno operates Service workloads and treats
`overdrive deploy` plus `overdrive workload describe` as promises about the
workload that serves traffic, including when that workload runs behind a guest
kernel.

## Wave: DISCUSS / [REF] JTBD One-liner

When Ana needs a long-running service to run with its own kernel, she wants to
deploy the same Service specification and receive health and backend-eligibility
signals grounded in the guest workload, so she can route traffic or remediate
with the same confidence she has for an Exec-backed Service.

**Primary job:** `J-OPS-004`
**Related job:** `J-OPS-003`
**Job decision:** extend the existing jobs along their workload-driver
dimension; do not mint a duplicate VM-Service job.

## Wave: DISCUSS / [REF] Locked Decisions

| ID | Decision | Verdict |
|---|---|---|
| D1 | Feature type | Cross-cutting: admission, VM lifecycle, Service health, and backend eligibility. |
| D2 | User contract | `[vm]` plus `[service]` is supported through the existing `overdrive deploy <spec>` and `overdrive workload describe <id>` surfaces. No VM-specific verb. |
| D3 | Probe mechanics | VM Services support HTTP and TCP mechanics in startup, readiness, and liveness roles. |
| D4 | Probe location | HTTP/TCP remain host-originated and, when their host is omitted or wildcarded, target the allocation's provisioned address: a VM guest address or an allocation-network Exec transit address. An explicit probe host retains its existing meaning. |
| D5 | Exec boundary | A VM Service declaring an Exec probe is rejected at parse time, before intent is committed, with an actionable error naming GH #280. Host-process Exec probes remain unchanged. |
| D6 | Lifecycle ownership | Allocation `Running` retains the existing Beacon/VM-driver meaning. Startup probes gate `Stable`; readiness controls `Backend.healthy`; `ServiceLifecycle` detects liveness termination while `WorkloadLifecycle` alone decides restart/finalization. No health result redefines `Running`. |
| D7 | Proof boundary | Acceptance is driven by the built default-feature binary through real `overdrive serve` plus `overdrive deploy`; tests may not hand-wire a target, route, or listener production omits. |

## Wave: DISCUSS / [REF] Grounding and Changed Assumptions

No feature-local DISCOVER or DIVERGE artifacts exist. The changed assumptions
come from older product SSOT and GH #257:

- `docs/product/journeys/run-a-vm-workload.yaml` said: “`[vm]` + `[service]`
  is REJECTED at deploy time — guest-stack mTLS interception (GH #222) is
  unbuilt”. GH #222 is now closed and its tap plus intercept are in the
  production path. The remaining work is guest-targeted probes and admission.
- GH #257 permits either in-guest Exec or parse-time rejection. This feature
  chooses rejection and defers the optional capability to GH #280. The
  research and spikes prove technical feasibility but do not make Exec a
  prerequisite for network Service health.

## Wave: DISCUSS / [REF] Scope Assessment

**PASS after thin-slice decomposition** — three user stories and three bounded
delivery slices. There is one user outcome: honest HTTP/TCP Service health and
reachability for the VM driver. Arbitrary in-guest execution is not part of
that outcome.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A — real-component walking skeleton.** Reuse the delivered VM driver,
routed TAP/intercept, Service probe scheduler, observation rows, reconciler, and
dataplane. The first slice enables one explicit TCP startup probe end to end:

`payments-vm.toml` → `overdrive deploy` → guest listener reached at
`workload_addr` → truthful `Stable`/`Failed` → healthy backend serves one mesh
request → `workload describe` shows the same probe result.

The skeleton is not a hand-built integration harness and has no environment
switch between fake and real paths. It runs on native x86_64 metal because a
red result from nested Lima cannot render a reliable Cloud Hypervisor verdict.

## Wave: DISCUSS / [REF] Story Map

| Declare | Deploy | Observe health | Route or withhold traffic | Recover |
|---|---|---|---|---|
| VM Service plus TCP startup probe | Existing deploy verb accepts it | Stable/Failed names guest result | Healthy guest serves mesh request | Fix listener and redeploy |
| VM Service plus HTTP probe | Same verb and schema | Status/path determine result | Readiness controls eligibility | Inspect last failure |
| Readiness/liveness roles | Same role declarations | Ongoing transitions remain visible | Unhealthy backend is withdrawn | Liveness follows existing restart policy |

Priority follows learning leverage: TCP proves the delivered network can carry
the production health path; HTTP proves mechanic parity; readiness/liveness
prove continuous eligibility.

## Wave: DISCUSS / [REF] User Stories with Elevator Pitches and Acceptance Criteria

### US-SVM-1 — Deploy a TCP-probed VM Service

`job_id: J-OPS-004`
`related_job_id: J-OPS-003`

Ana cannot currently submit a Service backed by `[vm]`; admission rejects it
before the delivered guest network or Service-health machinery can run.

#### Elevator Pitch
Before: Ana can run the VM as a Job, but cannot expose it as a health-gated Service.
After: run `overdrive deploy payments-vm.toml` → sees `Service 'payments-vm' is stable` with startup probe `tcp :8443`, then a real mesh request returns the guest response.
Decision enabled: Ana can admit the VM deployment to receive traffic or fix its guest listener.

#### Domain Examples

1. Ana deploys `payments-vm` listening on guest port 8443; the TCP probe passes and a request returns `payments-vm/ready`.
2. Ana deploys `ledger-vm` whose guest never binds port 7443; deploy ends Failed with the last connection failure and no backend receives traffic.
3. Ana deploys `catalog-vm` with an explicit probe host; the existing explicit-host meaning is preserved instead of silently rewriting it to the guest address.

#### UAT and Acceptance Criteria

- Given the real server and a VM Service whose guest binds TCP 8443, when Ana
  runs `overdrive deploy payments-vm.toml`, then the stream reaches Stable only
  after the host-originated probe reaches the guest and one mesh request returns
  the guest's byte-distinct response.
- Given the guest never binds its declared port, when the startup deadline
  expires, then deploy exits nonzero with `StartupProbeFailed`, identifies TCP
  8443 and its last failure, and the VM is absent from eligible backends.
- Given an explicit probe host is declared, when the probe runs, then its target
  retains the pre-feature explicit-host semantics; only omitted/wildcard hosts
  resolve to the allocation's provisioned address.
- The acceptance runner builds and drives the default-feature `overdrive` binary;
  it does not supply guest addressing or dataplane rules the product did not.

#### Outcome KPI

K1: VM Services complete the canonical TCP success/failure journeys truthfully
in 100% of 100 native-metal acceptance runs; baseline is 0% because admission
currently rejects every VM Service.

### US-SVM-2 — Use an HTTP contract inside the VM

`job_id: J-OPS-004`

Ana needs application-level health, not merely an open port, for guest services
whose listener can accept before the application is ready.

#### Elevator Pitch
Before: Ana cannot make an HTTP response from a VM determine Service health.
After: run `overdrive deploy checkout-vm.toml` → sees Stable for `http GET /ready :8080` only after the guest returns 2xx, or a named HTTP failure otherwise.
Decision enabled: Ana can promote the guest application only when its own readiness contract passes.

#### Domain Examples

1. `checkout-vm` returns 204 from `/ready`; its startup probe passes.
2. `search-vm` accepts TCP but returns 503 from `/ready`; it does not become Stable.
3. `profile-vm` returns 302 from `/ready`; existing HTTP semantics treat the redirect as failure rather than following it silently.

#### UAT and Acceptance Criteria

- Given a VM Service returns 204 at its declared guest path, when the startup
  probe runs, then deploy reaches Stable and `workload describe` shows the same
  passing HTTP probe.
- Given the port accepts but `/ready` returns 503, when the deadline expires,
  then deploy fails with the status-derived reason and never reports Stable.
- Given `/ready` returns 302, when probed, then it fails under the existing
  non-2xx rule and the response body is not exposed as unbounded probe output.

#### Outcome KPI

K2: all canonical HTTP status classes (2xx pass; 3xx/4xx/5xx fail) produce the
same result for Exec-backed and VM-backed Services in the black-box suite.

### US-SVM-3 — Keep VM backend eligibility honest over time

`job_id: J-OPS-004`

Startup success is insufficient for a long-running VM Service. Ana needs later
readiness and liveness results to preserve the existing Service contract across
the guest boundary.

#### Elevator Pitch
Before: a VM that becomes unhealthy after startup cannot participate in the Service health lifecycle.
After: run `overdrive workload describe inventory-vm` → sees readiness change `Pass → Fail → Pass`; real mesh requests avoid the backend while failed and return after recovery.
Decision enabled: Ana can leave convergence to the platform or intervene when liveness initiates the existing restart policy.

#### Domain Examples

1. `inventory-vm` flips readiness to 503; it is withdrawn within one configured probe cycle and restored after 204.
2. `pricing-vm` keeps readiness healthy while liveness fails; the existing liveness policy restarts it without redefining policy for VMs.
3. `orders-vm` terminates while a role probe is in flight; no later probe result resurrects backend eligibility.

#### UAT and Acceptance Criteria

- Given a Stable VM Service, when readiness changes from pass to fail, then its
  backend becomes ineligible within one configured interval plus timeout and
  real requests do not reach it; pass restores eligibility on the same bound.
- Given liveness fails according to the existing declared thresholds, when the
  role reaches its failure condition, then the existing restart lifecycle is
  invoked and describe renders the resulting allocation truthfully.
- Given the workload terminates while a probe is active, when late work
  completes, then termination wins: no result makes the dead backend eligible.

#### Outcome KPI

K3: 100% of readiness transitions change observed backend eligibility within
one declared interval plus timeout, with zero requests reaching a known-failed
backend in the acceptance window.

## Wave: DISCUSS / [REF] Outcome KPIs

| KPI | Who | Does what | Target | Baseline | Measured by |
|---|---|---|---:|---:|---|
| K1 | VM Service operators | Complete TCP VM-Service deploy and route one request | 100/100 native-metal runs | 0/100; parser rejects | Black-box `serve` + `deploy` expectation |
| K2 | VM Service operators | Receive mechanic-consistent HTTP outcomes | 100% canonical status cases | No VM-Service cases | Cross-driver black-box matrix |
| K3 | VM Service operators | Rely on readiness to govern eligibility | ≤ interval + timeout; zero requests to known-failed backend | Unavailable | Describe plus real request trace |

North star: the percentage of representative VM Service deployments whose
operator-visible health matches actual guest network reachability and response.
Guardrails: no eligible known-failed backend and no VM-specific redefinition of
the existing lifecycle states.

## Wave: DISCUSS / [REF] Driving Ports

- `overdrive deploy <SPEC>` — declares and streams admission/startup outcome.
- `overdrive workload describe <ID>` — renders current per-role probe state and
  allocation lifecycle.
- A real VM Job mesh client connection to the Service listener — proves backend
  eligibility and response bytes, not merely internal state.
- `overdrive serve` — the only production composition root accepted by the
  black-box proof.

No new CLI verb, HTTP route, interactive Exec endpoint, or test-only entry point
is introduced by these requirements.

## Wave: DISCUSS / [REF] Pre-requisites

| Dependency | Status | Why it matters |
|---|---|---|
| GH #42 — Cloud Hypervisor VM driver | Closed/delivered | Boots and supervises VM workloads and reports guest lifecycle through `overdrive-init`. |
| GH #222 — guest-stack tap plus mesh intercept | Closed/delivered | Supplies routed guest `workload_addr`, receive path, and mesh reachability. |
| Service health-check probes (GH #170) | Delivered | Supplies roles, scheduler, results, Stable/Failed decisions, describe rendering, and backend eligibility. |
| Native-metal H6 evidence | Delivered | Proves host-to-guest TCP on the production topology; preserved in commit `7035afb7`. |

## Wave: DISCUSS / [REF] Out-of-scope

- A general interactive guest exec API, remote shell, SSH service, or arbitrary
  operator command surface.
- In-guest Exec health probes, a persistent Exec-control session, VM-Exec wire
  protocol, guest command supervision/containment, and reconnect semantics;
  tracked separately by GH #280.
- QEMU Guest Agent, Kata's full container-management protocol, or a second
  general-purpose guest-management product.
- Adversary-resistant health truth against a compromised guest kernel/root.
- New TAP topology, new mTLS proxy, or intended-peer authorization; GH #222 and
  the existing mesh stack remain the substrate.
- Redefining Service thresholds, restart policy, observation-row schema, or
  backend-health semantics unless DESIGN proves unavoidable and returns for
  explicit scope approval.

## Wave: DISCUSS / [REF] Definition of Ready

| # | Gate | Status | Evidence |
|---:|---|---|---|
| 1 | Clear domain problem | PASS | The remaining blanket VM-Service rejection blocks HTTP/TCP health even though guest network reachability is delivered. |
| 2 | Specific persona | PASS | Ana's VM Service operator context is linked to `ana-platform-engineer`. |
| 3 | Three domain examples per story | PASS | Each of the three stories has happy, boundary, and failure examples with named services and real ports/paths. |
| 4 | Three to seven UAT scenarios | PASS | Each story carries three or four Given/When/Then-equivalent criteria. |
| 5 | AC derived from UAT | PASS | Every criterion states a production-observable pass/fail outcome. |
| 6 | Right-sized | PASS | Three independently demonstrable one-day slices; optional in-guest Exec is split to GH #280. |
| 7 | Constraints identified | PASS | Target resolution, lifecycle ownership, parse-time Exec rejection, and production proof are locked. |
| 8 | Dependencies resolved/tracked | PASS | #42/#222/#170 are delivered and the required H6 network evidence passed. |
| 9 | Outcome KPIs defined | PASS | K1–K3 have numeric targets, baselines, and collection methods. |

Requirements completeness: 0.98. Functional behavior, reliability/security
guardrails, business rules, error paths, and production proof are present. The
exact protocol/API is intentionally absent because it belongs to DESIGN.

## Wave: DISCUSS / [REF] Definition of Done

- [ ] All story UAT scenarios pass against the built default-feature binary.
- [ ] Unit, integration, DST, and real-kernel tests required by the accepted
  design pass in their proper lanes.
- [ ] Native-metal evidence proves HTTP/TCP guest targeting on the production
  Cloud Hypervisor path.
- [ ] `overdrive serve` plus `overdrive deploy` reaches every shipped behavior
  without test-installed production effects.
- [ ] Operator-visible errors answer what happened, why, and what to do next.
- [ ] Code and accepted API shape receive independent review and approval.
- [ ] Verification expectations capture and independently audit the TCP, HTTP,
  and readiness/liveness operator journeys.
- [ ] Product SSOT, examples, and CLI vocabulary consistently use
  `overdrive deploy` and reflect delivered scope.
- [ ] The feature is merged and Ana can demonstrate stable, unhealthy, and
  recovered VM-Service outcomes in one session.

## Wave: DISCUSS / [REF] Risks and DESIGN Handoff

| Risk | Probability | Impact | Required response |
|---|---|---|---|
| Omitted/wildcard host resolves somewhere other than `workload_addr` | Medium | High | Keep one driver-aware target projection and prove it through the real network path. |
| Probe success is conflated with allocation `Running` | Medium | High | Preserve the lifecycle ownership table and exercise the boundary scenarios required by `.claude/rules/design.md`. |
| Network probe passes through test-only wiring | Medium | Critical | Black-box expectations must drive the built binary and observe the production TAP/intercept path. |
| Exec is accidentally accepted for VM workloads | Low | High | Parse-time rejection names GH #280 and occurs before intent commit. |

DESIGN receives all decisions and priorities in this file. It must select the
smallest driver-aware HTTP/TCP target projection and preserve existing probe
role ownership without inventing an operator-facing API. It must not introduce
an Exec-control protocol, persistent guest supervisor, or guest process
containment mechanism under this feature.

## Wave: DISCUSS / [REF] Artifact Index

- `docs/feature/service-kind-vm-workloads/feature-delta.md`
- `docs/feature/service-kind-vm-workloads/slices/slice-01-tcp-vm-service-walking-skeleton.md`
- `docs/feature/service-kind-vm-workloads/slices/slice-02-http-vm-service-probes.md`
- `docs/feature/service-kind-vm-workloads/slices/slice-03-vm-service-readiness-liveness.md`
- `docs/product/jobs.yaml` (extends J-OPS-003/J-OPS-004; no new job)
- `docs/product/journeys/run-a-vm-workload.yaml`
- `docs/product/journeys/submit-a-service.yaml`
- `docs/product/personas/ana-platform-engineer.yaml`

## Wave: DISCUSS / [REF] Prior-wave Consultation

- ✓ `docs/product/jobs.yaml`
- ✓ `docs/product/vision.md`
- ✓ `docs/product/journeys/run-a-vm-workload.yaml`
- ✓ `docs/product/journeys/submit-a-service.yaml`
- ✓ `docs/product/personas/ana-platform-engineer.yaml`
- ✓ `docs/research/virtualization/vm-exec-health-probes-comprehensive-research.md`
- ⊘ `docs/project-brief.md` (not found)
- ⊘ `docs/stakeholders.yaml` (not found)
- ⊘ feature-local DISCOVER artifacts (not found)
- ⊘ feature-local DIVERGE artifacts (not found)

## Wave: DISCUSS / [REF] Wave Decisions Summary

```yaml
wave: DISCUSS
feature: service-kind-vm-workloads
density: lean
expansion_prompt: ask-intelligent
feature_type: cross-cutting
walking_skeleton: brownfield-strategy-a
jtbd: enabled
job_decision: extend-J-OPS-003-and-J-OPS-004
stories: 3
slices: 3
scope_assessment: pass-after-thin-slice-decomposition
architecture_decisions_left_to_design:
  - exact Rust trait/type/signature surface
  - exact driver-aware default network-target projection
deferred:
  - "GH #280: optional in-guest Exec health probes"
review_triggered: false
review_reason: consolidated review remains mandatory at DISTILL; requirements contain no unresolved product red card
telemetry: not-emitted-helper-missing
handoff: DESIGN-all-priorities
```

## Wave: DESIGN / [REF] Prior-wave Consultation

- ✓ `docs/product/architecture/brief.md`
- ✓ relevant accepted ADRs: ADR-0048, ADR-0051, ADR-0054, ADR-0055,
  ADR-0057, ADR-0058, ADR-0064, ADR-0080, ADR-0082, ADR-0083, ADR-0087,
  ADR-0088, ADR-0089
- ✓ `docs/product/journeys/run-a-vm-workload.yaml`
- ✓ `docs/product/journeys/submit-a-service.yaml`
- ✓ this file's DISCUSS sections and all three active slices
- ✓ `docs/feature/service-kind-vm-workloads/spike/findings.md`
- ✓ `docs/feature/service-kind-vm-workloads/spike/wave-decisions.md`
- ⊘ feature-local `discuss/wave-decisions.md`, `user-stories.md`,
  `story-map.md`, and `outcome-kpis.md` (not found; their contracts are the
  DISCUSS sections above)

Contradiction check: resolved. The active design contains only host-originated
HTTP/TCP VM probes. Guest Exec control evidence remains attached to GH #280
and supplies no component, protocol, lifecycle gate, or dependency to GH #257.

## Wave: DESIGN / [REF] DDD List

- **DDD-1 — Registration-time target projection:** `ProbeRunner` resolves the
  immutable effective HTTP/TCP destination once from the full
  `AllocationSpec`. This reuses the existing per-allocation supervisor and
  keeps runtime addressing out of persisted probe intent. See ADR-0090.
- **DDD-2 — VM address is a production precondition:** the action shim injects
  `Some(guest_addr)` before VM start and invokes `on_alloc_running` only after
  the Running write. No behavior is added for unreachable `Vm + None`. See
  ADR-0090.
- **DDD-3 — One Service driver union:** parser `ServiceSpecEnvelope` has the
  sole direct `V3(ServiceSpecV3)` arm at tag `0`, with `driver: DriverInput`.
  The greenfield cut deletes V1/V2 compatibility; existing wire, intent,
  allocation, and describe unions are reused. See ADR-0091.
- **DDD-4 — VM Exec exclusion before persistence:** parser and authoritative
  submit admission scan Startup -> Readiness -> Liveness and then lowest vector
  position, rejecting that first VM Exec probe through their existing
  role-localized error fields with GH #280 guidance. Host-process Exec probes
  are unchanged. See ADR-0091.
- **DDD-5 — Lifecycle meanings remain fixed:** Running is the existing
  Beacon/driver boundary; startup owns Stable, readiness owns backend health,
  `ServiceLifecycle` owns liveness threshold detection/termination, and
  `WorkloadLifecycle` alone owns restart/finalization. The feature moves no
  gate.
- **DDD-6 — Production proof:** the built default-feature binary must drive
  real `serve` + `deploy` through the production VM network. H6 supplies the
  already-completed host-to-guest TCP feasibility witness.
- **DDD-7 — Proposed marked HTTP connector:** retain the exact public
  `HttpProber` contract and project targets unchanged; the private production
  Hyper connector stamps the existing mTLS dial-exemption mark before each
  non-loopback connect so a host probe bypasses the already-installed OUTPUT
  divert. Loopback remains unmarked. See proposed ADR-0092.
- **DDD-8 — Proposed streaming Service acknowledgement rendering:** retain the
  existing ordered Service stream and public output shape; on a successful
  un-detached Service stream, compose the existing Accepted acknowledgement
  before the existing Stable detail in the one existing summary. See proposed
  ADR-0093.
- **DDD-9 — Proposed marked TCP adapter:** retain the exact public `TcpProber`
  contract and projected host/port; the private production TCP socket carries
  the existing mTLS dial-exemption mark before every non-loopback connect, so
  the worker OUTPUT divert cannot fabricate a TCP pass. Loopback remains
  unmarked. See proposed ADR-0094.
- **DDD-10 — Proposed shared default stream-cap envelope:** retain the existing
  Job and Service streaming loops, terminal taxonomy, and
  `AppState::streaming_cap` field; set the shared existing standard cap to 90s
  and its generic existing private CLI request envelope to 120s so the
  existing 60s inferred-startup failure can project `StartupProbeFailed` before
  the streaming-only `Timeout`. No operator cap configuration exists:
  `AppState::streaming_cap` remains a construction/test override, not a
  config-file or CLI option. See proposed ADR-0095.
- **DDD-11 — Proposed terminal-startup eligibility withdrawal:** retain the
  existing `Running` meaning and `StartupProbeFailed` terminal. On that
  terminal decision, the reconciler constructs the existing full backend row
  for the deciding allocation with `healthy: false` before the existing
  terminal action. The current shim continues after a row-write error, so this
  does not promise a durable terminal barrier on that error path. The
  no-readiness default remains healthy only for a non-terminal Running
  allocation. No state, action, row, owner, restart, storage, or public-surface
  addition is authorized. See proposed ADR-0096.
- **DDD-12 — Proposed Exec target/accounting correction:** an allocation-network
  Exec default/wildcard network probe uses its already-provisioned transit
  `workload_addr`; explicit and unnetworked targets retain their existing
  meanings. `ServiceLifecycle` reuses its existing last-counted LWW timestamp
  map so each Startup result counts once and normalizes a legacy unpaired
  counter to its current one observation. No port, store operation, schema,
  retry policy, terminal, or lifecycle owner changes. See proposed ADR-0097.

## Wave: DESIGN / [REF] Quality Attributes and Constraints

| Rank | Attribute | Design response |
|---:|---|---|
| 1 | Reliability / operator honesty | Guest network health is observed at the provisioned guest address; probe failure cannot masquerade as allocation lifecycle failure |
| 2 | Compatibility | Explicit targets and unnetworked Exec/process defaults stay unchanged; allocation-network Exec defaults reach the allocation address; prior ServiceSpec persisted bytes are deliberately unsupported in this greenfield project |
| 3 | Maintainability | Extend existing unions, runner, driver hooks, and reconcilers; create no parallel VM-Service subsystem |
| 4 | Testability | Pure projection/admission properties complement production-composition and native-metal evidence |
| 5 | Performance efficiency | Resolve the immutable target once per registration rather than once per tick |

Constraints: one existing Rust binary and crate boundary map; Cloud Hypervisor
capability remains optional at node composition; no new external service,
daemon, storage, or protocol; proposed ADR-0092 permits only the existing,
workspace-pinned `tower-service` 0.3.3 connector-contract dependency. The
production VM path requires native-metal evidence. No organization change was
supplied, so Conway alignment is preserved by leaving current component
ownership and deployment boundaries intact.

## Wave: DESIGN / [REF] Component Decomposition

| Component | Path | Change | Responsibility |
|---|---|---|---|
| Service TOML parser and parser envelope | `overdrive-core::aggregate::{workload_spec,service_spec}` | EXTEND | Admit either existing driver; use one direct V3 envelope arm at tag 0 and reject VM Exec locally |
| Service aggregate admission/describe | `overdrive-core::aggregate::ServiceV2` | EXTEND | Authoritative cross-field rejection; project both drivers through existing describe union |
| Service CLI deploy lanes | `overdrive-cli::commands::deploy` | EXTEND | Forward the selected parser driver unchanged through existing request shapes |
| Service streaming renderer | `overdrive-cli::{commands::deploy,render}` | EXTEND | Reuse the existing Accepted and Stable renderers in the existing successful Service summary, preserving one-stream order |
| `ProbeRunner` | `overdrive-worker::probe_runner` | EXTEND | Resolve effective HTTP/TCP target at allocation registration, then reuse existing scheduling and result writes |
| `HyperHttpProber` | `overdrive-worker::probe_runner::http_prober` | EXTEND | Private marked connector supplies the existing HTTP port without target or policy changes |
| `TokioTcpProber` | `overdrive-worker::probe_runner::tcp_prober` | EXTEND | Private marked socket construction preserves host/port and TCP outcomes while bypassing an existing matching OUTPUT divert |
| Service stream wait | `overdrive-control-plane::streaming` + existing CLI HTTP client | EXTEND | Change only the existing default server/client duration values; no Service-specific cap, descriptor lookup, or new configuration surface |
| Terminal-startup eligibility projection | `overdrive-reconcilers::service_lifecycle` + existing action-shim batch | EXTEND | Existing `StartupProbeFailed` constructs the deciding allocation's existing full backend row with `healthy: false` before existing `FinalizeFailed`; the shim's continue-on-error path supplies no durable terminal barrier on a row-write error; no new action, persistence, or owner |
| `VmDriver` | `overdrive-worker::vm_driver` | EXTEND | Receive the trusted shared runner and implement existing Running/Stable/terminal probe hooks |
| Production composition root | `overdrive-control-plane::run_server` helpers | EXTEND | Retain and pass the already-probed runner into the optional VM driver |
| Action-shim networking | `overdrive-control-plane::action_shim` | REUSE | Preserve provision-and-inject before start; supply the VM registration precondition |
| `ServiceLifecycle` and backend projection | `overdrive-reconcilers::service_lifecycle` | EXTEND; ownership reused | ADR-0101 revision 3 pins sole all-listener membership/eligibility publication with observed-row diff; existing terminal predicate and lifecycle action ordering retained |
| `BackendDiscoveryBridge` | `overdrive-reconcilers::backend_discovery_bridge` | RETIRE (Accepted ADR-0101 revision 3; implementation pending) | Reuse membership computation at ServiceLifecycle; directly remove publisher, registration and dispatch surface |
| `WorkloadLifecycle` | `overdrive-reconcilers::workload_lifecycle` | EXTEND wiring; authority reused | Preserve sole restart authority and unified budget; existing membership-mutating actions now wake ServiceLifecycle directly |

No new component, crate, daemon, protocol, persisted observation row, CLI verb,
HTTP route, or lifecycle state is created.

## Wave: DESIGN / [REF] Driving Ports

- `overdrive deploy <SPEC>` — existing parser and submit/streaming path; now
  admits `[service] + [vm]` for HTTP/TCP probes and rejects VM Exec locally.
  For an un-detached successful Service stream, it renders the existing
  Accepted acknowledgement before the existing Stable detail in that command's
  stdout; detached/non-TTY acknowledgement behavior is unchanged.
- `overdrive workload describe <ID>` — existing describe response and renderer;
  preserves the VM driver instead of collapsing it to Exec.
- `overdrive serve` — existing production composition root; shares the one
  trusted `ProbeRunner` with both production drivers when VM capability exists.
- Real Service traffic — an existing VM Job mesh client path observes backend eligibility
  and byte-distinct guest responses.

## Wave: DESIGN / [REF] Driven Ports and Adapters

| Port/effect | Existing adapter | Feature use |
|---|---|---|
| `Driver` | `VmDriver`, `ExecDriver` | Reuse existing lifecycle hooks; VM gains the same runner delegation |
| `TcpProber` | `TokioTcpProber` | Connect to the registration-projected host and declared port; proposed private socket construction marks only non-loopback candidates before connect |
| `HttpProber` | `HyperHttpProber` | Request the registration-projected URL with unchanged HTTP semantics; proposed private connector marks only non-loopback sockets before connect |
| Service streaming presentation | existing `consume_stream` + CLI render functions | On successful Service `Accepted -> Stable`, compose the existing acknowledgement before existing Stable detail; no new wire or CLI surface |
| Shared Job/Service stream wait | existing `AppState::streaming_cap`, both stream constructors, and CLI request timeout | Preserve existing cap futures and typed terminal projections while changing only shared standard 90s/120s durations; `AppState::streaming_cap` remains a construction/test override, with no operator configuration |
| Backend-health withdrawal | existing `ServiceLifecycle` plus serial action shim dispatch | ADR-0101: ServiceLifecycle alone constructs the complete backend projection and compares it with observed rows; BackendDiscoveryBridge is retired. Construct changed rows with the deciding allocation `healthy: false` before the startup-terminal action; serial dispatch attempts them first but may continue after an error; consumers converge asynchronously |
| `ObservationStore` | production local observation adapter | Persist unchanged `ProbeResultRow` outcomes |
| VM networking | action-shim provisioner + Cloud Hypervisor TAP attach | Supply the already-provisioned guest address and production route |

No new external integration is introduced; existing adapter Earned-Trust gates
remain unchanged.

## Wave: DESIGN / [REF] Technology Choices

- **Rust:** existing language and typed union/envelope implementation.
- **Tokio:** existing cooperative per-probe task supervision.
- **Hyper:** existing HTTP probing behavior.
- **tower-service 0.3.3:** proposed, already-locked connector contract used
  only by HyperHttpProber's private connector under ADR-0092.
- **Tokio/Linux socket capability:** existing `TokioTcpProber` uses the
  already-existing trusted-dial mark under ADR-0094; no dependency is added.
- **rkyv:** existing versioned-envelope discipline for the greenfield parser
  `ServiceSpecEnvelope::V3` direct arm; retired V1/V2 bytes are unsupported.
- **Cloud Hypervisor + Linux routed TAP/netns:** delivered VM substrate reused
  as-is. No new dependency or platform service.

## Wave: DESIGN / [REF] Decisions Table

| ID | Decision | ADR |
|---|---|---|
| DDD-1 | Resolve effective HTTP/TCP destination once at allocation registration | ADR-0090 |
| DDD-2 | Treat VM `workload_addr` presence as a production precondition, not health behavior | ADR-0090 |
| DDD-3 | Advance parser Service payload to V3 and reuse the existing driver union | ADR-0091 |
| DDD-4 | Reject VM Exec probes locally and authoritatively before intent commit | ADR-0091 |
| DDD-5 | Add, remove, and move no lifecycle gate | ADR-0090/0091 |
| DDD-7 | Mark private non-loopback HTTP sockets before connect | Proposed ADR-0092 |
| DDD-8 | Render existing Accepted before existing Stable for successful streaming Service deploy | Proposed ADR-0093 |
| DDD-9 | Mark private non-loopback TCP sockets before connect | Proposed ADR-0094 |
| DDD-10 | Make the shared Job/Service default cap 90s with a 120s generic existing client envelope | Proposed ADR-0095 |
| DDD-11 | A terminal startup failure constructs existing backend-health withdrawal before the terminal action, without redefining Running | Proposed ADR-0096 |
| DDD-12 | Project allocation-network Exec defaults/wildcards to the existing transit address and count each LWW Startup result once | Proposed ADR-0097 |
| DDD-13 | Require accepted restart Running publication before release; one fresh-predecessor re-proposal, then existing unwind on rejection | Proposed ADR-0099 |

## Wave: DESIGN / [REF] Reuse Analysis

| Existing component | Overlap | Decision | Contract shape / universe / assertion |
|---|---|---|---|
| `WorkloadSpecInput` + `ServiceSpecEnvelope` | Parser discrimination and greenfield Service payload | EXTEND | Pure function over one TOML document; one direct V3 fixture and tag `[0]`, with no V1/V2 decode, re-archive, or migration surface |
| `DriverInput` + `WorkloadDriverV2` | Exec/VM representation | REUSE | Pure tagged-union projection; both-arm round-trip and schema fixtures |
| `ServiceV2::from_submit` / `to_describe` | Admission and output projection | EXTEND | Pure function over one Service payload; cross-field properties and driver-preserving round-trip |
| Service CLI deploy lanes (`deploy_service`, `deploy_streaming_service`) | Parser-to-wire projection for both Service submission modes | EXTEND | Bounded-change universe is one parsed Service and one existing lane's `ServiceSpecInput`; exact delta is the driver projection only; lane-parity assertions preserve the selected union arm and existing HTTP/output behavior |
| Service streaming renderer (`consume_stream` + existing render functions) | Existing Accepted/terminal event projection | EXTEND | Bounded-change universe is one successful Service `Accepted -> Stable` stream; reuse the existing Accepted block and Stable detail exactly once, order asserted by a direct stream/render test and one-command E08 capture |
| `ProbeRunner` | HTTP/TCP scheduling and result ownership | EXTEND | Bounded change to one allocation supervisor and its result rows; destination-capture tests |
| `HyperHttpProber` | Host-side HTTP connection adapter below unchanged `HttpProber` | EXTEND | Bounded change to one transient socket per resolved candidate; set existing mark before non-loopback connect; no loopback mark or retained state; adapter test + built-product E08 |
| `TokioTcpProber` | Host-side TCP connection adapter below unchanged `TcpProber` | EXTEND | Bounded change to one transient socket per resolved candidate; set existing mark before non-loopback connect; loopback remains mark zero; preserve candidate order and all current TCP result semantics; direct adapter tests + built-product E09/E13 |
| mTLS OUTPUT exemption + mark constant | Existing self-interception exemption consumed by HTTP sockets | REUSE | Existing rule and shared constant are not changed; E08 proves the normal production path reaches guest HTTP 204 |
| `VmDriver` | VM lifecycle ownership | EXTEND | Bounded change to one driver's existing hook delegation; lifecycle component tests |
| Production composition (`run_server`, `compose_production_driver`, `compose_vm_driver`) | Trusted-runner ownership and optional VM registry wiring | EXTEND | Bounded-change universe is one server boot/driver registry; exact delta retains the returned runner and passes one `Arc` clone to `VmDriver`; production-composition assertions preserve one Earned-Trust gate and existing capability outcomes |
| Action-shim VM provision path | Guest address producer | REUSE | Bounded change universe is one allocation and its owned network resources; existing ordering evidence + H6 |
| Action-shim successful restart publication | Existing compound acceptance and failed-publication unwind | EXTEND (Proposed ADR-0099) | Bounded change to one authorized restart: at most two proposals; only accepted Running releases hooks; seed 257203 and no-contender control |
| `ServiceLifecycle` | Startup/readiness, terminal eligibility and liveness detection/termination | EXTEND; ownership reused | Bounded-change universe is one Service's allocation/listener rows and existing lifecycle actions; composed publication safety and convergence evidence belongs to DISTILL. ADR-0101 revision 3 pins exact API |
| `WorkloadLifecycle` | Sole restart-versus-finalize authority | EXTEND wiring; authority reused | Pure reconcile, existing allocation-action universe; consolidate membership/lifecycle enqueue at ServiceLifecycle, preserving restart and budget semantics |

Every retained overlap is reused or extended; ADR-0101 retires the competing
bridge publisher. There are zero CREATE NEW component decisions.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

**Sole projection — ADR-0101 revision 3 (2026-09-08; DESIGN and consolidated
DESIGN+DISTILL approved):** retained E09 v2 native evidence and seed `257209` prove that bridge
membership disappearance/reappearance can permanently defeat the unchanged
ServiceLifecycle terminal-health veto. The
[ruling](design/backend-eligibility-convergence-ruling.md) and
[exact ADR](../../product/architecture/adr-0101-service-backend-health-observed-convergence.md)
pin the user-selected ServiceLifecycle sole projection: combine all-listener
Running membership and existing eligibility, compare complete observed output,
and retire BackendDiscoveryBridge directly. No subsequent publication grants
eligibility while the existing terminal veto applies, including same-ID
restart. The user resolved consumer completion: lifecycle failure is not an
acknowledgement from routing consumers; they converge asynchronously.
Greenfield means no compatibility, migration, dual publishers or rollout;
normal runtime View persistence remains. No restart-identity or health-policy
change. ADR-0096's approved predicate remains input despite historical Proposed
metadata and its corrected same-ID premise. Prior overlays are not executable.
Independent [DESIGN iteration 3](design/review-adr-0101.md#iteration-3--focused-re-review-of-r0101-3)
and [consolidated DESIGN+DISTILL iteration 2](distill/review-adr-0101-design-distill.md#iteration-2--focused-re-review-of-cd-0101-0102)
are APPROVED on 2026-09-08. These approvals do not claim production GREEN or
implementation completion; recorded behavioral REDs and unexecuted suffixes
remain implementation obligations. The acceptance designer owns test design
and executable tests.

**Changed health gate — ADR-0096.** Target projection and admission add no
gate. ADR-0096 changes the existing `Backend.healthy` gate only: terminal
startup failure vetoes eligibility for its deciding allocation.

| Signal/state | Owner | Promise | Existing inputs | Explicitly unaffected |
|---|---|---|---|---|
| Allocation `Running` | Action shim + selected driver; VM start uses Beacon | Driver start succeeded and Running observation committed | Existing provisioning, VM start/Beacon, intercept ordering | Stable, readiness health, liveness verdict |
| Service `Stable` | `ServiceLifecycle` startup branch | Startup contract passed or was explicitly disabled | Startup observations + existing deadline policy | Running, readiness, liveness |
| `Backend.healthy` | `ServiceLifecycle` readiness branch, with its existing terminal startup decision as a veto | A non-terminal allocation meets the declared readiness threshold; a terminally startup-failed allocation is ineligible | Existing startup terminal decision, then readiness observations + counters | Running, Stable, restart authority |
| Liveness termination | `ServiceLifecycle` liveness detector | Threshold reached; emit only `StopAllocation { terminal: Stopped { by: LivenessProbe } }` | Liveness observations + counter | Restart budget/decision; meanings of Running, Stable, readiness |
| Restart decision/action | `WorkloadLifecycle` (sole restart authority) | Restart the liveness-terminated allocation under the unified budget or finalize failure | `AllocStatusRow.terminal` + unified restart budget | Probe detection; meanings of Running, Stable, readiness |

ADR-0096's changed gate contract: `ServiceLifecycle` owns both the terminal
startup decision and the existing backend-row projection. Its
`StartupProbeFailed` projects the deciding allocation as `healthy: false` in
the existing full `ServiceBackendRow`. The reconciler constructs that action
before `FinalizeFailed`; serial dispatch attempts it first but continues after
its error, so only a successful row write withdraws eligibility before the
following terminal publication. No durable ordering claim is made for the
existing error path. `Running`, `Stable`, liveness policy, and
`WorkloadLifecycle` restart authority are unaffected. A non-terminal
no-readiness allocation stays healthy; a late probe cannot restore a terminal
startup failure; liveness/restart retains its existing path. WorkloadLifecycle's
automatic restart creates a replacement allocation attempt under the same
allocation id; this is not a fresh allocation identity. Reconciler tests cover the constructed action
order and these retained cases; E09/E13 prove the healthy-store production
path, not a write-failure guarantee.

Executable handoff obligations: a closed guest port leaves the allocation
Running while startup fails; startup pass changes only Stable; readiness
fail/pass changes only eligibility; liveness detection emits the existing
termination and `WorkloadLifecycle` alone decides restart versus finalization;
terminal wins over late success; explicit and unnetworked Exec/process targets
are unchanged while allocation-network defaults use the existing transit address; VM Exec
is rejected before intent commit. No `Vm + None` test is warranted because no
production path produces that state.

## Wave: DESIGN / [REF] Changed Assumptions

- **VM watcher ownership — Accepted ADR-0100 (independently approved):**
  seed 257205 proves the old VM watcher can consume a same-ID replacement's
  claim and author another natural ending. Restrict the existing watcher gate
  and its Drop to the originating accepted session, reusing weak BeaconWriter
  identity with no public API or persisted shape changes. This does not move
  finalization/reclamation ownership or imply a failed allocation becomes
  Terminated after operator stop. See
  `design/vm-restart-ending-authorship-ruling.md` for the distinct startup
  rejection and subsequent user-approved E10 disposition (2026-09-07): require
  Terminated when stopping a Running allocation; an already startup-failed
  allocation may remain Failed with its failure preserved and cleanup verified.
  The user authorized the narrow example/expectation correction, not arbitrary
  crashes or a product lifecycle change. Its implementation review and full
  verification remain pending. Authorship-design approval is recorded
  in [independent DESIGN review, iteration 1](design/review-adr-0100.md); it does
  not claim implementation or E10 completion.

- **Restart publication boundary — Proposed ADR-0099:** seed `257203` through
  the existing production convergence entry point reproduces the old Exec
  allocation attempt's Terminated(36) winning against replacement Running(36).
  The proposed correction reuses the compound write's existing acceptance
  verdict: one fresh-predecessor re-proposal, then existing awaited unwind on
  exhausted rejection; Running-confirmed hooks require acceptance. No public
  API, schema, restart authority, or recovery subsystem changes. ADR-0098 is
  not a dependency and is not removed. See
  `design/allocation-restart-write-ruling.md`; independent review is pending.

- **ADR-0083 D4:** the blanket `[service] + [vm]` rejection is superseded now
  that GH #222's guest network/intercept is delivered. The replacement admits
  VM HTTP/TCP Services and rejects only VM Exec probes, which belong to GH #280.
- **ADR-0054/0058 default target:** loopback remains the unnetworked
  Exec/process default. ADR-0097 makes omitted/wildcard VM HTTP/TCP targets
  use the guest address and allocation-network Exec targets use their existing
  transit `workload_addr`; non-wildcard explicit hosts remain unchanged.
- **Spike constraints:** H1-H5 and the codec A/B remain evidence for GH #280,
  not active GH #257 requirements. H6 alone informs this feature's network
  target and proof lane.
- **Representable versus reachable:** `Vm + None` is representable in the
  shared struct but unreachable through production VM provisioning. No
  speculative failure machinery is introduced.
- **Adapter/dependency boundary:** the original DESIGN statement that no
  dependency could change is superseded only by ADR-0092. Real E08 proved the
  required socket effect; the permitted replacement is a private connector and
  workspace-pinned, already-locked `tower-service` 0.3.3. No public port,
  descriptor metadata, dataplane rule, route, daemon, protocol, persistence,
  or lifecycle change is authorized.
- **Streaming presentation boundary:** E08's one-command Accepted-before-Stable
  oracle exposed that the CLI consumed Accepted without displaying it. ADR-0093
  authorizes only a successful Service stream's existing summary composition:
  existing Accepted block then existing Stable detail. Detached/non-TTY,
  failure, cancellation, protocol, lifecycle, persistence, and probe behavior
  remain unchanged.
- **TCP transport boundary:** E13's deliberately unbound inferred-TCP control
  reached Stable because the host probe's unmarked SYN was accepted by the
  worker mTLS listener. ADR-0094 authorizes only the existing trusted-dial mark
  on private non-loopback TCP sockets before connect. Existing target
  projection, socket-result classification, public port, HTTP, UDP, rules,
  routes, lifecycle, protocol, daemon, and dependencies remain unchanged.
- **Streaming wait boundary:** after ADR-0094 made E13's refusal truthful, the
  equal 60s default Service startup deadline and shared Job/Service stream cap
  made the Service stream synthesize `Timeout` before the existing
  `StartupProbeFailed` projection. ADR-0095 changes only the shared existing
  default server cap to 90s and generic private CLI envelope to 120s. The
  streams keep their existing terminal owners, projection, `AppState` field,
  custom-cap behavior, and wire shape; no operator cap configuration exists.
- **Startup-terminal eligibility boundary:** E09 proves that “no readiness
  probe means healthy” cannot override the existing terminal
  `StartupProbeFailed` decision. ADR-0096 narrows the rule: a deciding
  startup-terminal allocation is unhealthy and the reconciler constructs its
  existing backend-row action before the terminal action. The current shim may
  continue after a row-write error, so no durable ordering is claimed for that
  path; Running truth, non-terminal no-readiness compatibility, all other
  lifecycle owners, and persistence stay unchanged.

No DISCUSS story or acceptance criterion changes are required; therefore no
`design/upstream-changes.md` is produced.

## Wave: DESIGN / [REF] Open Questions

Proposed ADR-0099 requires independent DESIGN review before step `02-03`
implementation resumes. Its seed proves the rejection defect; E10 is not yet
green. ADR-0098's removal/withdrawal remains a user decision, not an implied
part of this correction.

ADR-0092, ADR-0093, ADR-0094, ADR-0095, ADR-0096, and ADR-0097 require independent DESIGN review
before their original 02-01, 02-02, and 02-03 crafters may remediate them.
GH #280 independently owns any future in-guest Exec probe transport,
protocol, supervisor, session, or containment design; a future UDP host probe
likewise needs its own evidence and DESIGN decision.

## Wave: DESIGN / [REF] Architecture Summary

- Pattern: existing Rust modular monolith with ports-and-adapters.
- Paradigm: object-oriented, as pinned in `CLAUDE.md`.
- Key components: Service parser/admission, existing driver unions,
  `ProbeRunner`, `VmDriver`, production composition, action-shim VM network,
  and `ServiceLifecycle`.
- ADRs: ADR-0090 and ADR-0091 accepted; ADR-0092, ADR-0093, ADR-0094, ADR-0095, ADR-0096, and ADR-0097
  proposed bounded amendments pending independent DESIGN review.
- C4: `docs/product/architecture/c4-diagrams.md` section “Service-kind VM
  workload health”.
- Review state: base design approved by independent solution-architecture
  review iteration 2; ADR-0092, ADR-0093, ADR-0094, ADR-0095, ADR-0096, and ADR-0097 each require their own
  independent DESIGN review before their original DELIVER step resumes.

## Wave: DISTILL

### [REF] ADR-0101 revision 3 — authoritative backend projection amendment

The independent DESIGN review's third iteration approved the sole complete-row
author at ServiceLifecycle and direct BackendDiscoveryBridge retirement. The
acceptance designer owns this amendment's detailed scenarios and executable
tests; the original-wave tables below remain historical, not the status of
this new regression set.

`distill/adr-0101-acceptance.md` records BE-01–BE-12 plus the paired source-local
BE-P1 property, composition, Given/When/Then, taxonomy audit, bounds, and
limitations. The executable SSOT is
`crates/overdrive-sim/tests/integration/service_backend_projection.rs` and the
`#[cfg(test)]` addition in
`crates/overdrive-reconcilers/src/service_lifecycle.rs`. Tests exercise real
production-owner hydration/action dispatch and ProbeRunner using existing Sim
ports, real redb policy memory, real List/Watch consumers, and the supported
local Exec hydrator path. No production API, test seam, dependency, lifecycle
state, migration mechanism, harness, or example is added.

Consolidated-review findings CD-0101-01/02 strengthen only the test oracles:
BE-07 checks exact writer/tick/prior stamp construction before and after normal
runtime registration with the existing process-local tick reset; all published
rows, including empty/withdrawal, are checked against the retained allocator
VIP. Correlation and local-map expectations use that same independent VIP.
No production-authored prior row is replaced or fabricated for the reset case.

The unchanged original seed-257209 diagnostic remains a separate behavioral
RED witness. The new 12-test composition currently has 2 green controls and
10 behavioral RED regressions; the 128-case policy property is green. These
are full assertions, not `should_panic` success markers and not a claim that
unexecuted suffixes behind the first RED assertion have passed. Commands,
exit status, run IDs, and failure classification are appended to
`distill/red-classification.md`. The tests are left uncommitted for the approved
implementation/review sequence; no failing regression is weakened merely to
make hooks green.

Authoritative withdrawal is checked separately from later consumer convergence.
A failure report is not a durable-write or all-consumer acknowledgement.
Existing connections and historical-listener cleanup are excluded. Current
same-ID terminal veto, readiness counter behavior, normal stop/restart budget,
and current-program View reload remain explicit preservation contracts.
Native E09-v2 remains independent built-default-binary VM Service-traffic
evidence; no native/full-100 or mutation run occurs in this DISTILL amendment.
The scoped completeness audit addresses all 15 items (one input-validation
N/A); the fresh consolidated DESIGN+DISTILL reviewer still owns approval.

### [REF] Inherited commitments

| Origin | Commitment | DDR | Impact |
|---|---|---|---|
| DISCUSS#US-SVM-1 | VM-backed Services use the existing deploy/describe journey and HTTP/TCP probes reach the guest workload | n/a | One native-metal built-binary walking skeleton plus parser/TCP target scaffolds |
| DISCUSS#US-SVM-2 | HTTP keeps existing status/timeout/body semantics at the guest destination | n/a | Component scaffold covers 204/302/503 and bounded response handling |
| DISCUSS#US-SVM-3 | Startup, readiness, and liveness keep their existing distinct lifecycle meanings | n/a | Reconciler scaffolds pin Stable, backend eligibility, liveness stop, and sole WorkloadLifecycle restart authority |
| DESIGN#DDD-1/ADR-0090 | Resolve effective HTTP/TCP destination once from the full AllocationSpec at registration | n/a | Target matrix covers VM omitted/wildcard, explicit preservation, and unnetworked Exec loopback |
| DESIGN#DDD-2/ADR-0090 | VM workload address is a production precondition | n/a | No fallback behavior and no synthetic `Vm + None` test |
| DESIGN#DDD-3/ADR-0091 | ServiceSpec uses the direct V3 driver-union payload; greenfield persistence retires V1/V2 | n/a | Both-arm round-trip plus one-current-format schema round-trip obligation |
| DESIGN#DDD-4/ADR-0091 | Reject VM Exec probes locally and authoritatively before persistence in role/index order | n/a | Separate parser/direct-server scaffolds pin localization and exact GH #280 guidance |
| DESIGN#DDD-5 | Add, remove, and move no lifecycle gate | n/a | Tests treat the reconcilers as reused owners, not a VM-specific state machine |
| DESIGN#DDD-8/ADR-0093 | A successful streaming Service summary contains existing Accepted before existing Stable from the same operation | n/a | One direct CLI stream/render regression and E08 one-command PTY stdout order/multiplicity oracle; no shell composition |
| DESIGN#DDD-9/ADR-0094 | A private non-loopback TCP socket has the existing dial-exemption mark before connect; loopback stays unmarked | n/a | Direct socket-mark/loopback-preservation tests plus E09 100-pair and E13 complementary native-metal recapture; no harness-installed mark/routing |
| DESIGN#DDD-10/ADR-0095 | The shared default Job/Service cap is 90s and the generic private client envelope is 120s; explicitly injected caps are unchanged and no operator configuration exists | n/a | Seeded Service ordering proof, Service default/cap-first closure regressions, a Job default-cap/injected-cap regression, one generic client-envelope regression covering both streaming deploy lanes, and E13 native-metal recapture |
| DESIGN#DDD-11/ADR-0096 | Existing terminal `StartupProbeFailed` constructs peer-eligibility withdrawal before the terminal action, while Running and non-terminal no-readiness compatibility remain unchanged | n/a | Reconciler proof covers false-row action construction/order and retained cases; E09/E13 prove the healthy-store path and do not claim a row-write-failure guarantee |
| DESIGN#DDD-12/ADR-0097 | Allocation-network Exec defaults/wildcards target the existing transit address, and each LWW Startup result counts once; a legacy unpaired counter normalizes to one current observation | n/a | Projection coverage preserves explicit/unnetworked cases; reconciler/hydration/runtime-recovery coverage pins timestamp dedup, legacy-view recovery, and truthful three-attempt E10 failures; E10 reruns all eight built-product cells |
| SPIKE#H6 | Host-to-guest TCP feasibility passed, but spike disposition is DISCARD | n/a | Native-metal substrate retained; no spike test/code/API is promoted |

### [REF] Reconciliation result

After the ADR-0097 SSOT correction, reconciliation passed — 0 documented
target-projection or startup-accounting contradictions. The accepted design,
all three slices, product journeys, architecture brief/C4, design
review/decisions, and spike findings/decisions agree on the HTTP/TCP-only
boundary and lifecycle owners. C4 is unchanged because ADR-0097 adds no
component, owner, or persistence boundary. `docs/feature/service-kind-vm-
workloads/devops/` is absent. The generic fallback variants are inapplicable
here: hooks do not change built-product behavior, every run uses isolated
config/data rather than stale local state, and the qualified native-metal
preparation contract subsumes a clean runtime. The actual environment matrix
is native non-virtualized x86_64 Linux + KVM + canonical kernel/rootfs + leased
metal runner + isolated config/data + exact cleanup complement.

The DES runtime's exact `DESConfig(cwd=repo).deliverable_type` resolved to the
`None` sentinel (no project/global declaration and no positive plugin/skill
marker), which is the application enforcement path. Documentation density is
`lean`; only Tier-1 `[REF]` sections are emitted.

### [REF] Scenario list with tags

| ID | Scenario | Tags |
|---|---|---|
| S-SVM-01 | A healthy VM Service becomes Stable and returns its guest reply to a peer VM Job | `@walking_skeleton @driving_port @driving_adapter @real-io @adapter-integration @requires-kvm @native-metal @US-SVM-1 @US-SVM-2 @kpi:K1 @kpi:K2` |
| S-SVM-02 | A VM Service accepts supported network health without changing its declared intent | `@in-memory @property @US-SVM-1` |
| S-SVM-03..06 | Unsupported in-guest health-command ordering, localization, exact diagnostic, and pre-persistence rejection | `@in-memory @property @error @US-SVM-1` |
| S-SVM-07..09 | The one current saved format, both workload forms, and existing process-health behavior remain correct | `@in-memory @property @US-SVM-1` |
| S-SVM-10..12 | Default VM destinations, explicit destination preservation, and existing process defaults | `@in-memory @property @US-SVM-1` |
| S-SVM-13 | Guest listener refusal is health failure without rewriting Running | `@in-memory @error @US-SVM-1` |
| S-SVM-14 | Healthy, redirect, and unavailable application responses retain bounded outcomes | `@in-memory @property @US-SVM-2 @kpi:K2` |
| S-SVM-15..16 | VM health-lifecycle parity and singular restart registration | `@in-memory @US-SVM-3` |
| S-SVM-17 | Seeded terminal-authority and dead-backend invariant | `@in-memory @concurrency @seed:25717 @US-SVM-3` |
| S-SVM-18..21B | Running/startup/readiness/liveness/restart ownership and recovery | `@in-memory @error @US-SVM-3 @kpi:K3` |
| S-SVM-22 | One server boot shares one Earned-Trust-approved runner | `@in-memory @driving_port` |
| S-SVM-23..24 | Detached and streaming Service lanes preserve the same selected VM arm | `@in-memory @driving_port` |
| S-SVM-25 | 100 paired bound/unbound guest-listener trials observed by VM client Jobs | `@real-io @native-metal @US-SVM-1 @kpi:K1` |
| S-SVM-26 | Application-health outcomes agree across supported workload forms | `@real-io @native-metal @US-SVM-2 @kpi:K2` |
| S-SVM-27A | Ready baseline serves a peer VM client Job | `@real-io @native-metal @US-SVM-3 @kpi:K3` |
| S-SVM-27B | Unavailable window withdraws traffic from a peer VM client Job | `@real-io @native-metal @US-SVM-3 @kpi:K3` |
| S-SVM-27C | Recovered window restores traffic to a peer VM client Job | `@real-io @native-metal @US-SVM-3 @kpi:K3` |
| S-SVM-28 | Liveness restart visible through describe | `@real-io @native-metal @US-SVM-3` |
| S-SVM-29 | Zero-declared-health inferred listener pair observed by VM client Jobs | `@real-io @native-metal @US-SVM-1 @compatibility` |

Canonical GIVEN/WHEN/THEN prose and the AT-completeness audit live in
`docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.

### [REF] Walking-skeleton strategy

S-SVM-01/E08 is the **only** walking skeleton. The spike produced no promoted
skeleton. E09–E13 are bounded non-walking-skeleton failure, KPI, lifecycle,
and compatibility expectations over modes of the same checked-in example
bundle. Every mode runs the built default-feature binary through real
`overdrive serve` and `overdrive deploy <SPEC>`, pins public
arguments/exits/stdout/stderr/describe/peer-VM-Job outcomes/cleanup, and uses
no test-only composition seam. The bundle's one `prepare.sh` follows E07's
static-binary, private-rootfs, ownership-token, bounded mount/loop trap, and
marker-cleanup lifecycle; it installs both server and client binaries in the
guest image. Rust tests continue to exercise production crates in-process and
never spawn that binary.

### [REF] Adapter coverage

| Driven adapter/effect | Real-I/O scenario | Covered by |
|---|---|---|
| `TokioTcpProber` | YES | S-SVM-01 guest TCP startup on native metal; peer traffic uses the Service frontend rather than the probe target |
| `HyperHttpProber` | YES | S-SVM-01 guest HTTP readiness 204 on native metal |
| production local `ObservationStore` | YES | S-SVM-01 Stable/describe observations and terminal cleanup |
| action-shim VM network provisioner + Cloud Hypervisor TAP attach | YES | S-SVM-01 production serve/deploy guest route |
| Service frontend + VM client Job path | YES | S-SVM-01/S-SVM-25/S-SVM-27A/B/C/S-SVM-29 deploy a second plaintext `[job] + [vm]` caller and require its public Job result to depend on the VM guest reply |
| `VmDriver` / production Cloud Hypervisor VMM | YES | S-SVM-01 real guest boot; S-SVM-15 is the component complement |

There is no new external/non-deterministic adapter. Sim adapters are used only
for deterministic component and reconciler assertions and cannot model real
guest boot, host routes/TAP, TCP/HTTP wire behavior, process/cgroup ownership,
or kernel cleanup; S-SVM-01 owns those facts.

### [REF] Scaffolds

| Artifact | RED/pending shape | DELIVER activation |
|---|---|---|
| `crates/overdrive-core/tests/acceptance/service_kind_vm_workloads.rs` | 8 exact `should_panic("RED scaffold")` tests | parser/admission/V3/both-arm steps |
| `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs` | 7 exact `should_panic("RED scaffold")` tests | target/HTTP/TCP/hooks/re-registration steps |
| `crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs` | 1 exact seeded RED scaffold | terminal state wins; no dead-backend re-eligibility |
| `crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs` | 5 exact RED scaffolds | reused gate-ownership steps, with liveness stop and restart split |
| `crates/overdrive-control-plane/tests/acceptance/service_kind_vm_workloads.rs` | 1 exact RED scaffold | production composition step |
| `crates/overdrive-cli/tests/acceptance/service_kind_vm_workloads.rs` | 2 exact RED scaffolds | detached/streaming lane steps |
| `verification/expectations/E08`–`E13` | fail-closed exit 75 via six checked-in modes; E08 alone is the walking skeleton | native-metal built-product journeys using VM client Jobs for E08/E09/E11/E13 |
| `examples/service-kind-vm-workloads/prepare.sh` | source-safe check plus pending native-metal materialization | one E07-style private rootfs installs the static server and VM client binaries; token-bound cleanup owns the fixed tree |

No production scaffold is required: every component/import boundary already
exists, and the accepted public signature changes are DELIVER work. The
existing schema-evolution file receives its one current direct V3 fixture only
in the same commit that lands the greenfield envelope cut; pre-generating an
unbound fixture would be false evidence.

### [REF] Test placement

The generic `nw-distill` output contract's prohibition on a separate
`distill/test-scenarios.md` is superseded here by the repository's explicit
Rust override: `.claude/rules/testing.md` lines 17–24 names that exact path as
the specification-only home for Gherkin prose, forbids every `.feature` file,
and names Rust `#[test]`/`#[tokio::test]` functions as the executable SSOT.
`docs/architecture/atdd-infrastructure-policy.md` lines 5–13 repeats the same
project-wide mapping. The committed guest-stack, unconnected-UDP, and
backend-instance-replacement DISTILL artifacts are established precedents.
Accordingly this wave retains `distill/test-scenarios.md` only as
non-executable stakeholder prose; the 24 Rust functions and E08–E13 runners
are the machine-executable scenario artifacts, while this feature delta
carries every required Tier-1 summary.

- Parser, envelope, aggregate admission, and pure driver projection live in
  `overdrive-core/tests/acceptance/`, beside existing Service parser/aggregate
  tests.
- Target resolution and VM hook delegation live in
  `overdrive-worker/tests/acceptance/`, beside existing ProbeRunner/VmDriver
  tests.
- The terminal-authority outcome lives as a fixed-seed `overdrive-sim`
  invariant over existing lifecycle/backend boundaries. It does not prescribe
  ProbeRunner drain, join, tombstone, or late-write suppression machinery.
- Gate-owner assertions live in `overdrive-reconcilers/tests/acceptance/`, the
  extracted reconciler crate's direct boundary.
- One-boot ownership lives in `overdrive-control-plane/tests/acceptance/`.
- The two pure CLI projection lanes live in `overdrive-cli/tests/acceptance/`;
  no CLI Rust test spawns the production binary.
- The operator-visible binary journeys live only in the verification catalogue
  and drive modes of the one root example bundle, preserving
  examples/expectations/integration-test separation.

### [REF] Driving Adapter coverage

| DESIGN entry point | Protocol-level scenario | Pinned observable |
|---|---|---|
| `overdrive serve` | S-SVM-01/E08 through S-SVM-29/E13 | exact built default-feature argv, ready/refusal exit and stderr, cleanup |
| `overdrive deploy <SPEC>` | E08–E13 | checked-in Service and VM-client specs, Accepted/Stable/Failed and VM client Job terminal outcomes |
| `overdrive workload describe <ID>` | E08–E13 | existing VM driver, per-role probe rows, eligibility, restart, and peer VM Job outcome |
| real Service traffic | S-SVM-01/E08, S-SVM-25/E09, S-SVM-27A/B/C/E11, S-SVM-29/E13 | a checked-in plaintext VM client Job resolves the Service name, traverses the Service frontend, and succeeds only on the byte-exact VM guest reply; direct `workload_addr` calls are not the traffic oracle |

S-SVM-23/24 additionally prove the bounded internal projection parity of the
detached JSON and streaming NDJSON deploy lanes; they do not replace E08.

### [REF] Pre-requisites

- Accepted DESIGN commit `8c89c71eee2`, ADR-0090, ADR-0091, and the existing
  production `AllocationSpec`, driver union, ProbeRunner, VmDriver, action-shim,
  ServiceLifecycle, and WorkloadLifecycle boundaries.
- Native, non-virtualized x86_64 Linux with `/dev/kvm`, Cloud Hypervisor, the
  production kernel/rootfs fixtures, and the canonical `cargo xtask metal run
  --` lease for S-SVM-01. Lima is compile-only for this KVM path.
- Isolated config and data directories plus the qualified native-metal
  preparation/lease/cleanup contract. The one checked-in `prepare.sh` installs
  the static server and client into its private rootfs and refuses cleanup of
  unmarked or foreign-token output. Generic pre-commit and stale-config
  variants do not affect this built-product boundary and are not claimed as
  executable scenario variants.
- H6 is feasibility evidence only (`DISCARD`); no H1-H5 guest Exec control or
  codec artifact is a GH #257 prerequisite.

### [REF] Outcome registry and KPI disposition

Outcome collision checks returned `NO COLLISIONS` for both new typed contract
grains. `OUT-SVM-SERVICE-TARGET-PROJECTION` and
`OUT-SVM-SERVICE-ADMISSION` are registered in
`docs/product/outcomes/registry.yaml`.

K1 is executed by S-SVM-25/E09 as exactly 100 isolated paired healthy/unbound
TCP journeys, with no retry or discarded trial. K2 is executed by
S-SVM-26/E10 as the eight-cell Exec/VM × 204/302/404/503 built-product matrix;
both 503 fixtures carry the nonempty
`SVM-E10-FAILURE-BODY-MUST-NOT-LEAK` sentinel, and the ledger requires zero
sentinel occurrences across operator-visible stdout, stderr, describe, and
rendered probe results. K3 is executed by S-SVM-27A/B/C/E11 as three distinct
VM-client-Job observations around two timed readiness transitions: both
transitions must converge within `interval + timeout`, and the known-failed
window permits zero VM guest replies. S-SVM-01/E08 remains
the single happy-path walking skeleton; component scenarios S-SVM-14 and
S-SVM-20 are independent regression complements. The repository-wide
`docs/product/kpi-contracts.yaml` is
explicitly scoped to `docs-platform` and forbids injection of other feature
KPIs, so it is not modified; these feature KPI contracts remain in this delta
and their eventual evolution record.

### [REF] RED handoff

`docs/feature/service-kind-vm-workloads/distill/red-classification.md` records
the repository-sanctioned hook-compatible RED evidence: every Rust scaffold
passes only by matching its exact `RED scaffold` panic. E08–E13 remain
explicitly pending with exit 75. A compile, import, fixture, unexpected panic,
or missing marker blocks DELIVER. There is no `Vm + None` test and no
production API invention.
