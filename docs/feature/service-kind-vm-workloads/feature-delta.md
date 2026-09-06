# Feature delta — `service-kind-vm-workloads`

DISCUSS requirements for GH #257. Documentation density is `lean`; this file
contains Tier-1 `[REF]` sections only. The implementation baseline and
comparative evidence are recorded in
[`vm-exec-health-probes-comprehensive-research.md`](../../research/virtualization/vm-exec-health-probes-comprehensive-research.md).

The current rejection of `[vm]` plus `[service]` is an implemented admission
rule. It is not evidence that VM Services or in-guest Exec probes are
impossible. GH #42 delivered VM Jobs/Schedules and GH #222 delivered the routed
guest network plus guest-stack mesh intercept; this feature closes the remaining
Service-health path.

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
| D3 | Probe parity | VM Services support HTTP, TCP, and Exec mechanics in startup, readiness, and liveness roles. Exec is not rejected merely because the current host-only adapter cannot cross the guest-kernel boundary. |
| D4 | Probe location | HTTP/TCP remain host-originated and, when their host is omitted or wildcarded, target the allocation's guest `workload_addr`. An explicit probe host retains its existing meaning. Exec runs in the guest workload context. |
| D5 | Exec command semantics | The declared argv is executed directly. There is no implicit shell, stdin, invented working directory, or environment override. An operator who wants shell behavior names a shell explicitly in argv. |
| D6 | Timeout safety | A timed-out Exec probe terminates and reaps only that probe command and its descendants. The Service workload and VMM remain alive. |
| D7 | Loss and overload | Probe work is bounded. Overload fails the affected tick promptly; loss of the control connection never causes an ambiguously delivered command to be replayed. A later scheduled tick is new work. |
| D8 | Output | Exec-probe stdout/stderr are not part of the first operator contract. Results expose a bounded, actionable category such as exit zero, nonzero exit, signal, spawn failure, timeout, unavailable, protocol failure, or overload. |
| D9 | Trust model | Health is advisory against a buggy workload, not adversary-resistant against guest root. The host/parser boundary must remain safe against hostile guest input, but this feature does not claim a compromised guest kernel cannot lie about health. |
| D10 | Architecture boundary | DISCUSS requires an in-guest execution path but does not select its transport, protocol, exact process-supervision primitive, public Rust types, or owner signatures. DESIGN must pin those after the bounded metal spike. |
| D11 | Proof boundary | Acceptance is driven by the built default-feature binary through real `overdrive serve` plus `overdrive deploy`; tests may not hand-wire a target, route, listener, or control channel production omits. |

## Wave: DISCUSS / [REF] Grounding and Changed Assumptions

No feature-local DISCOVER or DIVERGE artifacts exist. The changed assumptions
come from older product SSOT and GH #257:

- `docs/product/journeys/run-a-vm-workload.yaml` said: “`[vm]` + `[service]`
  is REJECTED at deploy time — guest-stack mTLS interception (GH #222) is
  unbuilt”. GH #222 is now closed and its tap plus intercept are in the
  production path. The remaining work is guest-targeted probes and admission.
- GH #257 said: “Exec probes cannot work at all without an in-guest execution
  path”. The first clause was too broad. The accurate statement is: the current
  host-only `CgroupExecProber` cannot execute beneath a guest kernel; an
  in-guest component can, and `overdrive-init` already executes the primary
  workload command there once.
- The current Beacon lifecycle remains one-shot and is not silently treated as
  a repeated probe RPC. Selecting and proving a persistent control mechanism is
  DESIGN work.

## Wave: DISCUSS / [REF] Scope Assessment

**PASS after thin-slice decomposition** — five user stories, four bounded
contexts, at most five walking-skeleton integration points, and approximately
five one-day delivery slices. There is one user outcome: honest Service health
and reachability for the VM driver. HTTP, TCP, role behavior, guest Exec, and
failure containment are increments of that outcome rather than independent
products.

The feature is at its cross-context upper edge. A required pre-slice native-metal
spike removes the highest uncertainty before exact design; it is an evidence
gate, not a separately released infrastructure slice.

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
| VM Service plus Exec probe | Same argv declaration | Guest exit status determines result | Role consumes the result identically | Correct command and redeploy |
| Slow/disconnected/overloaded Exec | Same bounded timeout declaration | Actionable failure, no false pass | Existing healthy workload is not killed by probe timeout | Next scheduled tick is fresh, never a replay |

Priority follows learning leverage: TCP proves the delivered network can carry
the production health path; HTTP proves mechanic parity; readiness/liveness
prove continuous eligibility; Exec proves the guest-control premise; failure
containment proves the mechanism is safe enough to operate.

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

### US-SVM-4 — Run declared Exec probes inside the guest

`job_id: J-OPS-004`
`related_job_id: J-OPS-003`

The host-only Exec adapter can launch only host processes. Ana needs an Exec
health command to inspect guest-local state without replacing it with a weaker
network probe.

#### Elevator Pitch
Before: Ana must remove an Exec probe or abandon VM Service admission even though in-guest execution is technically possible.
After: run `overdrive deploy fraud-vm.toml` → sees startup Exec probe `/usr/local/bin/check-ledger` pass or fail from the guest command's actual termination result.
Decision enabled: Ana can keep the workload class that fits the service without weakening its health contract.

#### Domain Examples

1. `/usr/local/bin/check-ledger --shard eu-1` exits 0 in `fraud-vm`; startup passes.
2. `/usr/local/bin/check-ledger --shard missing` exits 7; describe names a nonzero guest exit without substituting the VMM's status.
3. `/usr/local/bin/not-installed` fails to spawn; the result is actionable and neither the Service process nor VMM exits.

#### UAT and Acceptance Criteria

- Given an argv present in the guest exits 0, when its startup/readiness/liveness
  tick runs, then that role records Pass from the guest result.
- Given the guest command exits nonzero or by signal, when the result is
  observed, then that role records Fail with the bounded termination category
  and never uses the host VMM's exit status.
- Given argv names no guest executable, when execution is attempted, then the
  probe fails with a spawn/not-found category and the workload remains alive.
- The command is direct argv execution; shell expansion occurs only if argv
  explicitly names a shell.

#### Outcome KPI

K4: the canonical exit-zero, exit-7, signal, and not-found fixtures produce the
correct guest-grounded result in 100% of native-metal runs, with zero VMM status
substitutions.

### US-SVM-5 — Contain timed-out, lost, and overloaded Exec probes

`job_id: J-OPS-004`

Ana must be able to trust that a health check cannot leak guest processes,
duplicate side effects after a connection loss, or kill the Service it measures.

#### Elevator Pitch
Before: no accepted lifecycle contract bounds an in-flight VM Exec probe.
After: run `overdrive workload describe reports-vm` after a timed-out probe → sees `Fail: timeout` while the same allocation and VMM remain Running and no probe descendant remains.
Decision enabled: Ana can retain Exec health checks in production instead of treating them as a larger availability risk than the fault they detect.

#### Domain Examples

1. `sh -c 'sleep 30 & wait'` exceeds a two-second timeout; the shell and child are gone and `reports-vm` still serves traffic.
2. The guest-control connection drops after delivery but before response; the command is not replayed and the next interval creates fresh work.
3. All allowed probe slots are active; `analytics-vm` receives an immediate overload failure instead of an unbounded queue.

#### UAT and Acceptance Criteria

- Given a probe forks descendants beyond its deadline, when timeout occurs,
  then every process in that probe tree is terminated and reaped within one
  second after the declared timeout, while workload and VMM remain alive.
- Given the control connection is lost at an ambiguous delivery point, when it
  reconnects, then the old request is never replayed and the next scheduled
  tick uses a fresh correlation identity.
- Given the bounded active-probe capacity is exhausted, when another tick
  arrives, then it fails promptly as overload, memory stays bounded, and the
  workload remains responsive.
- Given stop begins with probes active, when shutdown proceeds, then new work is
  refused and active probe processes are gone before guest shutdown completes.

#### Outcome KPI

K5: across 100 seeded timeout/disconnect/overload schedules, zero leaked
descendants, zero ambiguous replays, and zero workload/VMM deaths caused by
probe cleanup.

## Wave: DISCUSS / [REF] Outcome KPIs

| KPI | Who | Does what | Target | Baseline | Measured by |
|---|---|---|---:|---:|---|
| K1 | VM Service operators | Complete TCP VM-Service deploy and route one request | 100/100 native-metal runs | 0/100; parser rejects | Black-box `serve` + `deploy` expectation |
| K2 | VM Service operators | Receive mechanic-consistent HTTP outcomes | 100% canonical status cases | No VM-Service cases | Cross-driver black-box matrix |
| K3 | VM Service operators | Rely on readiness to govern eligibility | ≤ interval + timeout; zero requests to known-failed backend | Unavailable | Describe plus real request trace |
| K4 | VM Service operators | Use guest-local Exec without weakening the contract | 100% canonical termination cases | Unsupported | Native-metal guest/VMM observation |
| K5 | VM Service operators | Survive pathological Exec probe lifecycle | 0 leaks, replays, or probe-caused workload deaths across 100 seeds | No mechanism | Seeded invariant plus native-metal process observation |

North star: the percentage of representative VM Service deployments whose
operator-visible health matches actual guest reachability and execution state.
Guardrails: no eligible known-failed backend, no unbounded process/output/queue,
and no probe cleanup that terminates the measured workload or VMM.

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
| Native-metal spike H1–H6 | Required before DESIGN closes mechanism | Proves concurrent lifecycle/control sessions, process-tree cleanup, loss behavior, bounded overload, and host-to-guest TCP on the production topology. |

The spike may invalidate a mechanism choice, not the product requirement. A
failed process-tree or reconnect proof returns DESIGN to another in-guest
mechanism; it does not reclassify guest Exec as technically impossible.

## Wave: DISCUSS / [REF] Out-of-scope

- A general interactive guest exec API, remote shell, SSH service, or arbitrary
  operator command surface.
- QEMU Guest Agent, Kata's full container-management protocol, or a second
  general-purpose guest-management product.
- Adversary-resistant health truth against a compromised guest kernel/root.
- Capturing or streaming Exec-probe stdout/stderr.
- Custom probe environment, stdin, working directory, or implicit shell syntax.
- New TAP topology, new mTLS proxy, or intended-peer authorization; GH #222 and
  the existing mesh stack remain the substrate.
- Snapshot/restore continuity for in-flight probes. This feature requires a new
  boot/session to begin with no replayed probe work.
- Redefining Service thresholds, restart policy, observation-row schema, or
  backend-health semantics unless DESIGN proves unavoidable and returns for
  explicit scope approval.

## Wave: DISCUSS / [REF] Definition of Ready

| # | Gate | Status | Evidence |
|---:|---|---|---|
| 1 | Clear domain problem | PASS | Current blanket rejection and host-only Exec limitation are distinguished from technical possibility. |
| 2 | Specific persona | PASS | Ana's VM Service operator context is linked to `ana-platform-engineer`. |
| 3 | Three domain examples per story | PASS | Each US-SVM story has happy, boundary, and failure examples with named services and real ports/paths. |
| 4 | Three to seven UAT scenarios | PASS | Each story carries three or four Given/When/Then-equivalent criteria. |
| 5 | AC derived from UAT | PASS | Every criterion states a production-observable pass/fail outcome. |
| 6 | Right-sized | PASS | Five independently demonstrable one-day slices; high uncertainty pulled into a pre-slice spike. |
| 7 | Constraints identified | PASS | Execution, target, timeout, loss, bounds, trust, and production-boundary constraints are locked. |
| 8 | Dependencies resolved/tracked | PASS | #42/#222/#170 delivered; remaining spike is an in-feature evidence gate. |
| 9 | Outcome KPIs defined | PASS | K1–K5 have numeric targets, baselines, and collection methods. |

Requirements completeness: 0.98. Functional behavior, reliability/security
guardrails, business rules, error paths, and production proof are present. The
exact protocol/API is intentionally absent because it belongs to DESIGN.

## Wave: DISCUSS / [REF] Definition of Done

- [ ] All story UAT scenarios pass against the built default-feature binary.
- [ ] Unit, integration, DST, and real-kernel tests required by the accepted
  design pass in their proper lanes.
- [ ] Native-metal evidence proves guest targeting and Exec containment on the
  production Cloud Hypervisor path.
- [ ] `overdrive serve` plus `overdrive deploy` reaches every shipped behavior
  without test-installed production effects.
- [ ] Operator-visible errors answer what happened, why, and what to do next.
- [ ] Code and accepted API shape receive independent review and approval.
- [ ] Verification expectations capture and independently audit the TCP,
  readiness, and Exec operator journeys.
- [ ] Product SSOT, examples, and CLI vocabulary consistently use
  `overdrive deploy` and reflect delivered scope.
- [ ] The feature is merged and Ana can demonstrate stable, unhealthy,
  recovered, timed-out, and connection-loss outcomes in one session.

## Wave: DISCUSS / [REF] Risks and DESIGN Handoff

| Risk | Probability | Impact | Required response |
|---|---|---|---|
| Guest process-tree cleanup primitive is insufficient in the shipped image | Medium | High | Prove before selecting exact supervision design; do not enable Exec on a leaky mechanism. |
| Lifecycle and repeated control traffic interfere | Medium | High | Spike simultaneous sessions and preserve existing boot/exit ordering. |
| Dropped response causes duplicate execution | Medium | High | Specify non-replay semantics and test ambiguous disconnect schedules. |
| Guest input attacks host parser/resource use | Medium | Critical | Bound frames, argv, concurrency, and allocations; fail closed without panic. |
| Requirements drift into a general guest agent | Medium | Medium | Keep scope restricted to declared health-probe argv and observable results. |

DESIGN receives all decisions and all priorities in this file. It must select
the exact internal control, supervision, framing/versioning, ownership, and
error-mapping shapes without inventing an operator-facing API. Any required
public surface not named by the accepted design is a blocker, not DELIVER
latitude.

## Wave: DISCUSS / [REF] Artifact Index

- `docs/feature/service-kind-vm-workloads/feature-delta.md`
- `docs/feature/service-kind-vm-workloads/slices/slice-01-tcp-vm-service-walking-skeleton.md`
- `docs/feature/service-kind-vm-workloads/slices/slice-02-http-vm-service-probes.md`
- `docs/feature/service-kind-vm-workloads/slices/slice-03-vm-service-readiness-liveness.md`
- `docs/feature/service-kind-vm-workloads/slices/slice-04-in-guest-exec-probes.md`
- `docs/feature/service-kind-vm-workloads/slices/slice-05-exec-probe-failure-containment.md`
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
stories: 5
slices: 5
scope_assessment: pass-after-thin-slice-decomposition
architecture_decisions_left_to_design:
  - exact guest-control transport and protocol
  - exact Rust trait/type/signature surface
  - exact guest process-tree supervision primitive
  - exact concurrency limit and internal result mapping
review_triggered: false
review_reason: consolidated review remains mandatory at DISTILL; requirements contain no unresolved product red card
telemetry: not-emitted-helper-missing
handoff: DESIGN-all-priorities
```
