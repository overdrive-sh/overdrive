# Independent DESIGN Review — Remove legacy Exec workload driver (GH #293)

## Metadata

| Field | Value |
|---|---|
| Feature | `remove-legacy-exec-workload-driver` |
| Review role | `nw-solution-architect-reviewer` |
| Review scope | Application/component DESIGN in Propose mode |
| Iteration | 1–2 |
| Date | 2026-09-14 |
| Reviewed repository state | `5377fcb85681b33fb27da6025374fa5b1990fd3f` plus the uncommitted DESIGN artifacts and this review artifact |
| Requirements authority | GH #293 body and comments (no comments were present when fetched) |
| User authority | P-293-1 through P-293-6 explicitly approved on 2026-09-14 |
| Result | `APPROVED` after iteration 2 |

## Review conclusion — iteration 1

The design selects the approved bounded removal: live workload execution becomes
VM/microVM-only; the existing driver registry, allocation-driver index and
per-composed-driver exit-observer ownership remain; exactly four Exec-coupled
envelopes reset forward-only to incompatible V1; the host Exec probe surface is
deleted; the driver-neutral P-105 replacement contract is preserved; and every
shared-switch/per-tap/shared-DNS/transparent-mTLS replacement remains GH #295
scope.

The package is feasible over current components and does not invent a
compatibility, migration, persistence, recovery, retry or dataplane mechanism.
ADRs 0110–0113 each record one coherent decision with context, at least two
alternatives and consequences. The C4 addition keeps the real operator → CLI →
control-plane → intent path and labels the current VM network mechanism as
temporary. The outcome-registry additions resolve structurally and the manual
collision disposition is honest about the installed checker's zero-population
defect.

Three blocking handoff defects remain. The Reuse Analysis omits effect-shape
coverage for two retained production owners and leaves the probe subsystem
unclassified; the Lifecycle Gate Ownership matrix incorrectly says transparent
mTLS interception gates `Running`; and the exact post-cut disposition of one
public Exec-named parser accessor is not specified. These are documentation and
contract defects inside the approved architecture, not authorization to add a
new mechanism.

## Reviewed artifacts and evidence

The review read and checked:

- GH #293 directly, including its empty comment thread, as the requirements
  contract;
- GH #295 and its comments only to verify that shared switching, per-tap
  classification/interception, shared-bridge DNS, transparent-mTLS re-homing,
  density and gateway compatibility remain outside #293;
- GH #280 directly to verify that any future in-guest command probe requires a
  separately approved guest-owned API and is not a continuation of the host
  `ExecProber` contract;
- the complete feature delta, ADR-0110 through ADR-0113, the focused architecture
  brief/C4 additions and the outcome-registry changes;
- accepted/relevant architecture records for driver routing, VM network and
  interception ordering, rkyv envelopes, health probing and the P-105
  allocation-replacement contract; and
- current production parser, aggregate, codec, driver, registry, action-shim,
  VM composition, probe-runner and exit-observer paths.

The user explicitly skipped DISCUSS and journey work. This review therefore does
not require or invent a user journey, story map or KPI artifact.

## Design contract checklist

| Contract item | Result | Evidence and disposition |
|---|---|---|
| P-293-1: delete all live Exec routes while retaining extensible VM-family routing | PASS | The exact parser, wire, intent and runtime payload unions reduce to `Vm`; `ExecDriver`, its export and its composition helper are deleted. `DriverRegistry`, `AllocDriverIndex`, the `Driver` port and one observer per composed driver remain (`feature-delta.md:119-125,185-351,457-475`). |
| VMM absence/refusal semantics remain unchanged | PASS | Ordinary capability absence may leave the registry empty; a present/injected capability whose probe fails still refuses boot (`feature-delta.md:100-103,123-125,487-489`; current production `lib.rs:1703-1766`). |
| P-293-2/P-293-3: exactly four forward-only incompatible V1 resets | PASS | `ServiceSpecEnvelope`, `WorkloadIntentEnvelope`, `AllocStatusRowEnvelope` and `AllocLifecycleOccurrenceRowEnvelope` are the only named reset owners; old readers, conversions, variants and fixtures are forbidden, while unrelated envelopes are explicitly unchanged (`feature-delta.md:224-319,603-631`). Current source confirms the two driver-bearing envelopes and the two lifecycle envelopes contain the removed nested vocabulary. |
| No compatibility or migration bridge | PASS | The design adds no old-byte detector, retired-payload branch, conversion or migration method. The deployment contract resets affected data before the new binary writes the new V1 (`feature-delta.md:315-319,616-631`). |
| P-293-4: complete host Exec probe removal | PASS, subject to F-01 and F-04 documentation closure | The exact enum, port, adapters, runner/parser/error branches and VM rejection shim are deleted; HTTP/TCP remain and GH #280 gets no placeholder (`feature-delta.md:353-392`). |
| P-293-5: driver-neutral P-105 lifecycle preserved | PASS | `RestartAllocation.alloc_id` remains the accepted numeric-current predecessor and `spec.alloc` the distinct durably reserved successor. View reservation, handoff, successor-first outcome, exact-old cleanup and existing public actions remain unchanged; only driver payload projection is narrowed and `AllocationAttemptEvent::Exec` is renamed to `Dispatch` (`feature-delta.md:394-409`). |
| P-293-6: strict GH #295 non-implementation boundary | PASS | The design retains the current VM netns/veth/two-`/30`/TAP/`host_veth` path and expressly forbids shared switching, per-tap interception, shared-bridge DNS, transparent-mTLS redesign, listener consolidation, cross-host or density claims (`feature-delta.md:708-728`). No #295 component, port or effect appears in the proposed type/component contracts. |
| Current VM-used network/cgroup fields are not preserved solely for Exec | PASS | Every admitted allocation follows the current VM plan; `AllocationSpec::{netns,host_veth,workload_addr,guest_tap,guest_mac,guest_gateway,guest_prefix_len,guest_dns}` remain because the production VM start/interception path consumes them (`feature-delta.md:411-453`; current action-shim `mod.rs:1905-1975,1987-2008`). Shared cgroup preflight, confinement and accounting remain VM requirements. |
| One decision per ADR | PASS | ADR-0110 owns the live execution-routing disposition; ADR-0111 owns the spec/intent forward-only cut; ADR-0112 owns the lifecycle-evidence forward-only cut; ADR-0113 owns host Exec probe removal. Exact signatures stay in the feature delta rather than the ADRs. |
| ADR quality | PASS | Each ADR contains decision-specific context, three alternatives, consequences, status and links. No ADR embeds a roadmap, executable test matrix or file-by-file implementation sequence. |
| Reuse Analysis and effect isolation | FAIL | See F-01. Two retained production owners named in Component Decomposition are absent from Reuse Analysis, and the probe subsystem row has no contract-shape classification. |
| Lifecycle Gate Ownership | FAIL | See F-02. The state-ownership matrix contradicts both the same artifact's ordering and the real production path for transparent-mTLS install versus accepted `Running`. |
| Exact post-cut public API shape | FAIL | See F-03. The public `WorkloadSpecInput::exec_command` accessor is not dispositioned. |
| C4 topology | PASS | CLI, typed contracts, control plane, reconcilers, worker, adapters and stores are distinct; the control plane, not the CLI, commits intent. The current network substrate is explicitly temporary and no #295 target topology is selected (`c4-diagrams.md:1470-1538`). |
| Outcome registry | PASS | Four new outcomes are registered, the prior combined Service admission outcome is superseded, the surviving target projection is narrowed to VM, all IDs are unique and every `related`/`superseded_by` reference resolves (`registry.yaml:655-770`). |

## Findings

### F-01 — Blocking: Reuse Analysis does not cover every retained effect owner

**Severity:** High

**Locations:** `feature-delta.md:457-475,529-551`

Component Decomposition names production composition and the exit observer as
separate retained/changed components, but neither has its own Reuse Analysis
row. The row for `ProbeMechanic` / `ProbeRunner` / prober ports describes a
descriptor/supervisor universe but does not declare one of the mandatory
contract shapes (`pure-function`, `bounded-change`, or
`unbounded-preservation`). This leaves the mutation universe and assertion
mechanism incomplete for stateful effects the implementation must preserve.

The omission intersects a concrete production owner path:

1. `run_server` currently composes the trusted `ProbeRunner`, inserts the Exec
   driver, conditionally discovers/probes/inserts `VmDriver`, then passes the
   registry into the server (`crates/overdrive-control-plane/src/lib.rs:1675-1766`).
2. `run_server_with_obs_and_drivers` then iterates every registry entry and
   spawns exactly one exit observer for it, cloning one shared shutdown token
   and retaining every task handle (`lib.rs:3019-3051`). Empty registry means
   zero observer tasks; one surviving VM entry means one.
3. Each observer consumes that driver's sole receiver, captures that driver's
   `DriverType`, writes lifecycle evidence and releases that driver's
   supervision claim (`worker/exit_observer.rs:84-122,161-208,289-306`). Receiver
   provenance, shared cancellation and task draining are observable bounded
   state, not documentation-only rewording.
4. The probe runner similarly owns a bounded map of per-allocation supervisors
   and writes `ProbeResultRow`s from HTTP/TCP tasks; removing the Exec adapter
   changes its dependency and dispatch set but must not loosen the HTTP/TCP
   supervisor or observation-write contract (`overdrive-worker/src/probe_runner/mod.rs:57-124,208-233`).

**Required remediation:** Add explicit Reuse Analysis coverage for production
composition and the per-composed-driver exit observer, and give the probe
subsystem an explicit contract-shape classification. For each, state the
existing effect universe, the exact #293 delta or zero-delta preservation, and
the assertion mechanism. Preserve the already-approved composition and
observer ownership; do not create a new helper, observer merger, shutdown
protocol, port or task owner.

### F-02 — Blocking: the state-ownership matrix incorrectly makes interception a `Running` gate

**Severity:** High

**Location:** `feature-delta.md:572-578`

The `Allocation Running` row lists “network/intercept gates” among inputs that
may gate `Running`. That is false for the current accepted VM lifecycle and
contradicts the feature delta's own later order (`feature-delta.md:650-651`) and
its statement that `Running` meaning does not change (`:670-671`).

The mismatch is reachable on every successful mTLS-composed VM start:

1. The action shim provisions/injects the current VM network before
   `Driver::start` (`crates/overdrive-control-plane/src/action_shim/mod.rs:1905-1975`).
2. A successful `driver.start(&spec)` selects `AllocState::Running`
   (`mod.rs:2010-2075`).
3. The shim writes and accepts that `Running` lifecycle row
   (`mod.rs:2111-2185`). At this point no allocation intercept has been
   installed; the code explicitly says the intercept guard sits after the
   Running write (`mod.rs:2175-2176`).
4. Only after the accepted row does the shim call
   `MtlsInterceptLifecycle::start_alloc` (`mod.rs:2254-2289`). Failure writes a
   later dominating `Failed` row and withholds the guest `EXEC` command; success
   precedes `release_for_exit_emission` (`mod.rs:2297-2305`). Accepted ADR-0089
   pins the same `READY ≺ intercept-live ≺ EXEC-release` order while treating
   the initial Running observation separately (`ADR-0089:130-170`).

Treating interception as an input to `Running` could cause DISTILL to assert an
unreachable transition order or DELIVER to move the gate across a state
boundary, which would change an explicitly preserved lifecycle meaning.

**Required remediation:** Correct the existing state-ownership matrix to
exclude transparent-mTLS intercept success from the inputs that gate the
accepted `Running` row. State the existing narrow order consistently:
current VM network provision and driver/guest readiness precede `Running`;
intercept install follows accepted `Running`; install failure authors the
existing dominating `Failed` result and prevents guest-command release. Do not
move a gate or change a state meaning.

### F-03 — Blocking: one public Exec-named parser accessor has no exact post-cut disposition

**Severity:** High

**Locations:** `feature-delta.md:185-223,455-476`; current
`crates/overdrive-core/src/aggregate/workload_spec.rs:759-768`

The exact live parser contract deletes both `ExecInput` types and narrows both
`DriverInput` unions to `Vm`, but it does not say what happens to
`WorkloadSpecInput::exec_command(&self) -> &str`. The method is public on the
publicly re-exported `WorkloadSpecInput` type
(`crates/overdrive-core/src/aggregate/mod.rs:51-53`). Its current body is a
driver-neutral command projection despite its Exec-specific public name; the
only in-tree call is a test, so there is no production-user compatibility
obligation that resolves the choice implicitly.

Keeping it would leave an Exec-named public workload API after the design says
the live parser surface is VM-only. Deleting it or replacing it with another
public accessor are different API shapes. Under the repository's exact-design
rule, a crafter may not choose between them simply because one compiles.

**Required remediation:** Pin the accessor's exact post-cut disposition in the
feature delta. If it is deleted, say that it is deleted without replacement and
classify its test as deletion or driver-neutral migration. If a replacement is
selected, specify the exact public signature and obtain any required material
API approval before implementation. Do not leave the choice to DELIVER.

### F-04 — Non-blocking: the historical Exec-probe ADR supersession is implicit

**Severity:** Medium

**Locations:** ADR-0113 `:9-37,76-82`; `feature-delta.md:757-766`

ADR-0054 remains `Accepted` with a three-port TCP/HTTP/Exec probe decision, and
ADR-0059 remains `Accepted` with the host-cgroup `ExecProber` placement
decision. ADR-0113 clearly deletes those Exec-specific clauses and links both
records, but it does not explicitly state which prior decision is superseded;
the Changed Assumptions table also omits ADR-0054/0059 while listing the prior
driver ADRs. The newer decision is understandable, but the authority chain is
less explicit than the other supersessions in the architecture SSOT.

**Required remediation:** Record that ADR-0113 supersedes ADR-0054's
Exec-mechanic/port clauses and ADR-0059's host Exec-probe decision, while
retaining ADR-0054's HTTP/TCP task, row and supervision contract. This is an
authority-link correction only; do not rewrite historical prose or add a new
probe design.

## Rejected hypotheses and scope checks

| Hypothesis | Disposition | Evidence |
|---|---|---|
| #293 should delete netns/veth/TAP/`NetSlot` now because Exec is gone | REJECTED — GH #295 scope | Current production VM start requires the C3 network/TAP plan before `Driver::start`, and current transparent mTLS consumes `host_veth`. GH #293 expressly makes their later replacement a #295 handoff. The feature delta preserves them temporarily and forbids selecting the replacement (`feature-delta.md:411-453,708-728`). |
| A one-driver product should collapse `DriverRegistry` and per-driver observers | REJECTED — contradicts P-293-1 and existing ownership | Start routing, stop/finalize routing without a spec, exit provenance and future capability composition already consume the registry/index/observer model. The current production observer loop is per registry entry (`lib.rs:3019-3051`). |
| Host `ExecProber` should remain for a future VM command probe | REJECTED — contradicts P-293-4 and GH #280 | The current adapter executes a host process in a host cgroup. GH #280 requires a separately designed guest protocol, correlation, cancellation, containment and session lifecycle. Retaining the host port would preserve the wrong boundary. |
| Old VM-only portions of pre-cut envelopes should remain readable | REJECTED — contradicts explicit greenfield approval | P-293-2/P-293-3 require no old readers, variants, conversions or fixtures. There are no production users or backward-compatibility obligations. |
| The design pulls forward shared switching or transparent-mTLS replacement | NOT OBSERVED | The exact contracts retain the current VM provision/intercept ports and explicitly prohibit every #295 mechanism. C4 labels the current network substrate temporary rather than presenting it as the selected target. |

## Architecture quality assessment — iteration 1

| Dimension | Assessment |
|---|---|
| Architectural bias | PASS. The design removes a concrete adapter and narrows existing types; it adds no technology, service, daemon, store or compatibility layer. |
| ADR quality | PASS with F-04 authority-link cleanup. The four ADRs are focused, alternatives-based and consequence-complete. |
| Completeness | FAIL on the mandatory Reuse Analysis and Lifecycle Gate Ownership details in F-01/F-02; otherwise the four-envelope, probe, lifecycle and #295 boundaries are complete. |
| Implementation feasibility | PASS, subject to exact API closure in F-03. Current production owners already expose every required VM, registry, observer, codec and action-shim seam. |
| Priority validation | PASS. The design removes 100% of newly admitted/live Exec execution while assigning 0% change to P-105 semantics and 0% implementation to #295. Simpler compatibility retention and scalar-driver collapse were considered and rejected against the approved constraints. |
| Testability | PASS after F-01 classification is completed. The reduced type graph can make Exec payload/admission unrepresentable; exact post-cut V1 fixtures and existing VM/sim/native evidence lanes are available without a new test seam. |

## Validation performed — iteration 1

- `git diff --check` passed for the tracked DESIGN changes. The untracked review
  artifact was checked separately and contains no trailing whitespace.
- `nwave-ai outcomes check-delta
  docs/feature/remove-legacy-exec-workload-driver/feature-delta.md` exited 0 and
  printed `5 outcomes checked, 0 collisions found across 0 outcomes`. As the
  design itself records, zero loaded registry outcomes makes this unsuitable as
  semantic collision proof; the registry was therefore checked manually.
- `docs/product/outcomes/registry.yaml` parsed successfully: 29 unique outcome
  IDs and every `related` / `superseded_by` reference resolves.
- Every local Markdown link in the feature delta, ADR-0110 through ADR-0113,
  architecture brief and C4 document resolves to an existing file.
- No roadmap exists for this feature, so roadmap checks were not applicable.
- No tests, mutation run, native-metal run or expectation capture was executed.
  This is a documentation/design review; production code was read as current
  evidence and was not changed.

## Iteration and remediation disposition

### Iteration 1 — 2026-09-14

The user-approved architecture and scope are coherent. F-01, F-02 and F-03 are
blocking because they leave effect isolation, lifecycle ownership and public
API shape incomplete or contradictory for downstream work. F-04 is an
authority-link cleanup that should be closed in the same documentation pass.
No production defect, compatibility obligation or #295 dependency was promoted
without a reachable current path.

## Iteration-1 verdict

**CHANGES_REQUESTED**

Do not begin DISTILL, roadmap creation or DELIVER. The original solution
architect should make only the scope-bound documentation corrections in
F-01–F-04, without changing P-293-1 through P-293-6 or adding a new mechanism,
then request a fresh independent review.

## Iteration 2 — independent remediation re-review

### Conclusion

The architect's bounded documentation revision closes F-01 through F-04
without changing P-293-1 through P-293-6. The revised feature delta now makes
the retained effect universes explicit, describes the real `Running` →
intercept → guest-command order consistently, pins deletion of the only omitted
public Exec-named accessor without replacement, and records the narrow prior-ADR
supersession. No remediation introduces a component, port, action, lifecycle
state, persistence/recovery protocol, compatibility reader, migration bridge,
retry owner or GH #295 mechanism.

No new blocking finding was identified. The design remains a greenfield
single-cut removal: old data and Exec vocabulary are not read, translated or
preserved; only the four approved envelopes reset; the driver-neutral P-105
contract is unchanged; and the current VM netns/veth/TAP/`host_veth` mechanism
remains temporary until GH #295 replaces it.

### Finding dispositions

| Finding | Iteration-2 disposition | Independent evidence |
|---|---|---|
| F-01 — incomplete Reuse Analysis/effect isolation | **CLOSED** | The Reuse Analysis now has explicit bounded-change rows for the production composition root, per-composed-driver exit-observer ownership and the HTTP/TCP-only probe subsystem (`feature-delta.md:543-568`). The composition row pins the one-boot universe and the only legal post-states (`∅` or `{Vm}`), preserving the existing trusted runner and VMM absence/refusal split. The observer row pins exactly one taken receiver and captured source per registry entry, one shared cancellation token, retained task handles and join-all shutdown; current production creates those tasks from `drivers.kinds()` and joins the vector after cancelling the shared token (`crates/overdrive-control-plane/src/lib.rs:3019-3051,1445-1459`). `exit_observer::spawn_with_runtime` still owns the receiver/provenance/write/release path (`worker/exit_observer.rs:161-208,289-306`). The probe row defines the per-allocation supervisor/task/result universe and limits the delta to removal of the Exec dependency/dispatch set; current `ProbeRunner` state and HTTP/TCP write path support that bounded assertion (`overdrive-worker/src/probe_runner/mod.rs:57-124,208-233`). No observer merger, token, helper or task owner is authorized. |
| F-02 — interception incorrectly listed as a `Running` gate | **CLOSED** | The lifecycle matrix now states that accepted `Running` promises current VM network provision, guest-ready `Driver::start` and accepted observation only, explicitly excluding intercept success and guest-command release (`feature-delta.md:585-596`). G-293-3 and the boundary obligations carry the same order and failure projection (`:651-697`), and the brief/C4 use the same relationship. This matches the production caller path: network provision/injection precedes start (`action_shim/mod.rs:1905-1975`); successful start selects `Running` (`:2010-2075`); the row is accepted (`:2111-2185`); only then does `MtlsInterceptLifecycle::start_alloc` run (`:2254-2289`); success precedes `release_for_exit_emission`, while failure returns through the existing dominating-`Failed` cleanup path (`:2297-2305`, `:616-705`). The correction documents the current accepted boundary and moves no gate. |
| F-03 — public `WorkloadSpecInput::exec_command` disposition absent | **CLOSED** | The exact live API section now deletes `WorkloadSpecInput::exec_command(&self) -> &str` without replacement and expressly forbids `command`, `driver_command`, `vm_command` or another substitute (`feature-delta.md:225-236`). Component Decomposition, Reuse Analysis and the test-migration contract repeat the same exact deletion (`:469-491,567,706-720`). Current source confirms the method is public on the re-exported `WorkloadSpecInput` and that `coinflip_migration` is its sole in-tree call (`workload_spec.rs:759-768`; `aggregate/mod.rs:51-53`; `tests/acceptance/coinflip_migration.rs:38-43`). Removing that assertion while retaining kind/ID checks is therefore complete and requires no invented public surface. |
| F-04 — prior Exec-probe ADR supersession implicit | **CLOSED** | ADR-0113 now explicitly supersedes only ADR-0054's Exec mechanic, port, adapter, dispatch/error and three-port-construction clauses while preserving HTTP/TCP tasks, rows, roles, thresholds, cancellation, supervision and Earned-Trust TCP probing (`ADR-0113:39-47`). It supersedes ADR-0059's host Exec-probe placement/timeout/cleanup decision in full without touching shared cgroups or VM confinement (`:49-55`). The feature delta Changed Assumptions table and brief ADR index carry the same narrow authority chain (`feature-delta.md:786-797`; `brief.md:3580`). This records history rather than rewriting it and introduces no future guest-probe placeholder. |

### Scope and decision revalidation

| Review item | Iteration-2 result |
|---|---|
| GH #293 requirements and approved P-293-1 | PASS — the revised contracts still delete every live Exec admission/payload/adapter/composition route while retaining VM-only tagged unions, the registry/index and per-driver observers. |
| P-293-2/P-293-3 schema boundary | PASS — exactly the same four envelope owners reset to incompatible V1; no old reader, conversion, fixture, detector or error branch was added, and unrelated envelopes remain untouched. |
| P-293-4 probe boundary | PASS — the host Exec probe surface is deleted completely; HTTP/TCP remain and GH #280 still owns any future guest command probe from a clean design. |
| P-293-5 lifecycle boundary | PASS — the correction clarifies existing state order but changes no P-105 predecessor/successor, durable reservation, handoff, publication or exact-old cleanup rule. |
| P-293-6 / GH #295 boundary | PASS — no shared switch, per-tap classification/interception, shared-bridge DNS, transparent-mTLS replacement, listener consolidation, cross-host routing, density/throughput claim or public-ingress mechanism entered the design. |
| One-decision-per-ADR | PASS — ADR-0113's supersession section records the consequence/authority scope of the same host-probe deletion; it does not add a second decision. ADR-0110–0112 remain unchanged in decision scope. |
| Exact API handoff | PASS — all changed public/internal shapes now have a named retain/delete/reset/rename disposition, including the formerly omitted parser accessor. No crafter choice is left open. |

### Architecture quality assessment — iteration 2

| Dimension | Assessment |
|---|---|
| Architectural bias | PASS. The remediation only documents existing owners and deletes stale surface; no technology or mechanism was added. |
| ADR quality | PASS. ADR-0113's supersession boundary is explicit and remains one-decision-scoped; ADR-0110–0112 remain complete. |
| Completeness | PASS. Reuse Analysis, exact API shape, Lifecycle Gate Ownership, four-envelope scope, probe deletion, P-105 preservation and the #295 non-implementation boundary are internally consistent. |
| Implementation feasibility | PASS. Every retained owner and deletion site exists on the current production path; the design requires no unsanctioned API or testability seam. |
| Priority validation | PASS. The design continues to allocate 100% removal to live Exec routes, 0% semantic change to P-105 and 0% implementation to #295. |
| Testability | PASS. Each bounded effect now names its universe and assertion boundary; exact type absence, VM/sim composition and native VM effects remain distinct evidence layers. |

### Validation performed — iteration 2

- Re-fetched GH #293 and its comments; the body is unchanged and the comment
  list remains empty.
- Re-read the revised feature delta, ADR-0110 through ADR-0113, focused brief
  and C4 additions, outcome registry, and the complete iteration-1 review.
- Re-traced the current production composition, observer cardinality/shutdown,
  parser accessor/export, probe-runner state and action-shim
  network/Running/intercept/release order cited in the dispositions above.
- `git diff --check` passed for tracked DESIGN changes; the untracked feature
  and review artifacts were separately checked for trailing whitespace.
- `docs/product/outcomes/registry.yaml` parsed with 29 unique IDs and all
  `related`/`superseded_by` references resolving.
- All local Markdown targets in the feature delta, review artifact, ADR-0110
  through ADR-0113, brief and C4 document resolve to existing files.
- `nwave-ai outcomes check-delta` still exits 0 with `5 outcomes checked, 0
  collisions found across 0 outcomes`; as in iteration 1, the zero loaded
  registry population is not treated as semantic evidence.
- No roadmap exists, so roadmap review remains not applicable. No production
  code, tests, mutation run, native-metal run or expectation evidence was
  changed or executed for this documentation review.

## Final verdict

**APPROVED**

F-01 through F-04 are closed with scope-bound documentation corrections. There
are no unresolved critical/high findings and no #295 implementation leakage.
The independent DESIGN review gate for the user-approved P-293-1 through
P-293-6 bundle is satisfied.
