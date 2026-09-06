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
| D4 | Probe location | HTTP/TCP remain host-originated and, when their host is omitted or wildcarded, target the allocation's guest `workload_addr`. An explicit probe host retains its existing meaning. |
| D5 | Exec boundary | A VM Service declaring an Exec probe is rejected at parse time, before intent is committed, with an actionable error naming GH #280. Host-process Exec probes remain unchanged. |
| D6 | Lifecycle ownership | Allocation `Running` retains the existing Beacon/VM-driver meaning. Startup probes gate `Stable`; readiness controls `Backend.healthy`; liveness feeds the existing threshold/restart policy. No health result redefines `Running`. |
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
  resolve to `workload_addr`.
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
- A real mesh client connection to the Service listener — proves backend
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
