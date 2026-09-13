# Corrective DESIGN review — driver-neutral allocation replacement

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Review role | `nw-solution-architect-reviewer` |
| Review scope | Application/components corrective DESIGN |
| Iteration | 1 |
| Date | 2026-09-13 |
| Reviewed repository state | `cb1a3026fcd38829be550845dcabd10a9a4f4a02` plus the uncommitted corrective DESIGN artifacts listed below |
| Authority | User-selected predecessor-to-fresh-successor model A; repository DESIGN authority and reachability rules |
| Result | `CHANGES_REQUESTED` |

## Reviewed artifacts and evidence

The review covered the corrective
[`feature-delta.md`](../feature-delta.md), proposed
[`ADR-0105`](../../../product/architecture/adr-0105-driver-neutral-allocation-replacement-identity.md),
the corrective amendments to ADRs 0073, 0078, 0081, 0083, 0087, 0089, 0099,
0100, 0102, 0103 and 0104, and the corresponding architecture brief and C4
updates. It also checked the current production paths in `WorkloadLifecycle`,
the convergence runtime, the action shim, the observation store, the driver
port, `ExecDriver`, `VmDriver`, the VM exit watcher and VMM host adapter.

GH #284, with comments, remains the authority for the reproduced VM
allocation-derived artifact alias. GH #293, with comments, owns later Exec
removal and explicitly does not create a same-ID compatibility boundary. PR
#292 and its review/inline comment were read as rejected implementation and
review input, not as authority for the corrective design.

The review does not mark any canonical design accepted and does not authorize
DISTILL or DELIVER. Under `.claude/rules/design.md`, a proposal and a reviewer
verdict cannot substitute for explicit conversational user approval of each
material decision.

## Authority and scope

The user explicitly selected the following contract:

- `RestartAllocation.alloc_id` names the predecessor;
- `RestartAllocation.spec.alloc` names a distinct fresh successor;
- `spec.identity` derives from the predecessor workload and successor
  allocation;
- `WorkloadLifecycle` chooses the numeric-current predecessor, applies one
  driver-neutral budget/backoff policy, reserves the successor, and emits the
  same existing action for VM and legacy Exec;
- the action shim awaits predecessor driver, mTLS and structural-network
  cleanup before successor provision, identity, driver start, fresh-row
  publication and Running-confirmed hooks;
- no `WorkloadDriver::Vm` replacement-policy branch, new action, port,
  driver-policy method, store/schema, retry, lock or network mechanism is
  permitted;
- Exec adopts the generic invariant until GH #293 removes it;
- PR #292 remains non-mergeable until accepted corrective DESIGN is followed
  by re-DISTILL and re-DELIVER.

The review treats additional identity-allocation, View-memory, publication,
history and retry/failure details as proposals unless their prior accepted ADR
already fixes them. It rejects adjacent hardening and the unproven Greptile
network-leak hypothesis as scope-expanding implementation authority.

## Design contract checklist

| Contract item | Result | Evidence and disposition |
|---|---|---|
| Existing action shape is preserved | PASS | `Action::RestartAllocation { alloc_id, spec, kind }` already carries both identities (`crates/overdrive-core/src/reconcilers/mod.rs:491-498` on `origin/main`; current branch `:493-499`). ADR-0023 originally described restart as stop followed by a fresh start whose ID changes (`adr-0023-action-shim-placement.md:110-113`), and ADR-0031 added `spec` (`adr-0031-job-spec-exec-block.md:648-676`). |
| Predecessor and successor meanings are exact | PASS | Proposed ADR-0105 D2 pins predecessor `alloc_id`, distinct successor `spec.alloc`, and successor SVID derivation (`ADR-0105:104-137`). The feature delta repeats the same exact relation (`feature-delta.md:91-127`). |
| Replacement policy is driver-neutral | PASS, subject to F-01 approval | The current PR branch demonstrably contains the rejected driver policy at `workload_lifecycle.rs:325,365-371,936-944,1043-1073,1122-1143,1461-1485`; ADR-0105 D3 removes it except for payload projection. The proposed implementation surface is private and adds no public API. |
| Numeric-current predecessor and candidate-keyed retry remain one decision | PASS, subject to F-01 approval | Current `current_alloc` selects the numeric-max parsed suffix (`workload_lifecycle.rs:1217-1246`), while the current PR branches candidate/deadline behavior by driver (`:312-378`, `:936-948`). ADR-0105 unifies both paths and retains ADR-0102's candidate-keyed deadline rule. |
| Issued successor is durable before dispatch | PASS, subject to F-01 approval | The runtime persists `next_view` before awaited dispatch and requeues after dispatch errors (`reconciler_runtime.rs:1458-1500,1516-1571,1590-1607`). Existing View fields can carry the reservation without a schema change (`workload_lifecycle.rs:1898-1912`). |
| Predecessor cleanup precedes successor effects in one owner | PASS, subject to F-01 approval | The restart arm reads/fences the predecessor, awaits driver stop, then awaits mTLS and network teardown (`action_shim/mod.rs:2253-2296`) before provision/identity/start (`:2306-2406`). The batch dispatcher continues after a per-action error (`:920-953`), independently validating rejection of separate Stop+Start actions. |
| Successor operations use only successor identity | PASS as a complete proposed mutation universe | ADR-0105 D5 enumerates provision, identity, start, failure rows, unwind, publication, routing index and hooks under `spec.alloc` (`ADR-0105:231-283`). Current code uses the same-ID assumption at all those sites (`action_shim/mod.rs:2321-2390,2434-2451,2554-2586,2606-2790`), so the listed cut is necessary and bounded. |
| Fresh row and predecessor history are separated | PASS, subject to F-01 approval | `ObservationStore::write_alloc_lifecycle` keys current rows by `current.alloc_id`, accepts a fresh absent predecessor atomically, and emits the paired occurrence (`observation_backend.rs:460-529,691-707`; trait contract `observation_store.rs:2072-2086`). Passing no same-key prior to `build_alloc_status_row` yields per-key zero/None crash facts (`action_shim/mod.rs:378-447`). |
| Driver port remains an execution adapter, not a policy owner | PASS | `Driver::start` consumes `AllocationSpec`, `stop` consumes `AllocationHandle`, and no replacement-policy method exists (`driver.rs:694-793,908-984`). Exec derives its cgroup/live/watcher identity from `spec.alloc` (`overdrive-worker/src/driver.rs:466-467,638-657`); VM derives its run-dir/cgroup/rootfs and supervision from `spec.alloc` (`vm_driver.rs:1142-1187,1304,1484-1485`). |
| Old VM cleanup cannot name the fresh VM artifact family | PASS for the selected identity correction | The VM watcher and stop path carry exact allocation-derived `LiveVm` capabilities and clean run-dir/cgroup/rootfs for that ID (`vm_driver.rs:839-879,1424-1432,1680-1815,2155-2237`). The VMM creates clone/kernel/run-dir effects from the supplied VM config (`overdrive-host/src/vmm.rs:359-455`). Fresh `spec.alloc` therefore separates the #284 artifact names structurally. |
| Legacy Exec follows the generic invariant until removal | PASS | GH #293 explicitly owns later removal and forbids a permanent same-ID compatibility rule. ADR-0105 changes no driver API and specifies the same successor `AllocationSpec` for Exec (`ADR-0105:318-345`). |
| No new action/port/store/schema/retry/lock/network mechanism | PASS | Reuse analysis accounts for every affected component and creates none (`ADR-0105:363-386`; feature delta `:245-267`). All effect calls and persistence surfaces already exist. |
| Greptile network-leak claim is proven | REJECTED | See R-01. Static reachability of the PR's bypass is not the repository-required failing production-path reproduction. It cannot become a design requirement. |
| Material decisions were explicitly approved in conversation | FAIL | See F-01. Only model A was selected; the proposal silently treats several additional decisions as resolved and ends with “No unresolved architecture decision remains” (`feature-delta.md:474-475`). |
| C4 preserves the real driving-port boundary | FAIL | See F-02. Both L2 copies put the CLI inside the node and connect it directly to redb IntentStore instead of the existing CLI → control-plane HTTP handler → IntentStore path. |

## Findings

| ID | Severity | Finding | Reachability and evidence | Required remediation or rejected disposition |
|---|---|---|---|---|
| F-01 | HIGH — blocking | Material identity-allocation, persistence-memory, retry/failure and observation-history decisions were not surfaced for explicit user approval. The artifact presents them as resolved, even though the user selected only the generic predecessor→fresh-successor action model. | This is an authority-gate failure, not a hypothetical production defect. If the design is approved and sent to DISTILL, the unsurfaced clauses become implementation authority through ADR-0105 D3-D6: checked max-suffix/`u32::MAX` no-action (`ADR-0105:163-186`); all-creation pre-dispatch reservations plus exact budget/time carry rules (`:188-229`); cleanup failure consuming a successor and later retrying above it (`:262-266`); fresh successor row reset, retained predecessor, and `Ok(None)` unwind-without-second-proposal (`:285-316`); and allowance for late exact-old-ID VM disposal (`:335-340`). The feature delta repeats these as final and says no unresolved decision remains (`feature-delta.md:145-187,206-233,319-353,474-475`). `.claude/rules/design.md:5-38` requires explicit conversational approval for identity/state ownership, internal action meaning, persistence/retry/ordering/recovery/compatibility and rejected alternatives. | Keep these clauses marked **proposed** and have the orchestrator surface numbered decisions to the user before acceptance: **(1)** the exact ID/reservation algorithm, including initial/generation creation, rows+View max scan, unused gaps and `u32::MAX` no-action; **(2)** the exact View budget/time carry rules across Workload Failure, Platform Reclamation, generation and initial placement; **(3)** cleanup-failure behavior—reservation stays consumed, runtime requeue chooses a higher successor, and no new cleanup-to-success retry; **(4)** fresh-key publication/history—predecessor immutable, successor per-key `restart_count=0`/`last_terminated=None`, accepted Failed rows can become numeric-current, and `Ok(None)` fully unwinds with no second proposal; **(5)** exact old-host-disposal relation—structural driver/mTLS/network cleanup is awaited as selected, while separately claimed exact-old-ID VM disposal may finish after the disjoint successor. Name affected owners and the alternatives already recorded. Remove the “no unresolved decision” claim until the user explicitly approves these decisions. Do not mark ADR-0105 accepted or dispatch re-DISTILL before that approval. |
| F-02 | HIGH — blocking | The C4 Level-2 diagram invents a direct CLI→IntentStore boundary and omits the existing `overdrive serve` HTTP/control-plane handler. | The mismatch is reachable through the real production entry point: `Command::Deploy` calls the deploy command (`crates/overdrive-cli/src/main.rs:63-92`); deploy calls `ApiClient::submit_workload[_streaming]` (`commands/deploy.rs:247-259,320-329,589-600,669-678`); the client sends `POST /v1/workloads` (`http_client.rs:190-216,219-238`); `run_server` routes that request to `handlers::submit_workload` (`overdrive-control-plane/src/lib.rs:3128-3140`); only the handler constructs the canonical intent key and writes through `state.store` (`handlers.rs:261-346` and its commit path). In contrast, the feature C4 places `cli` inside `System_Boundary(node)` and draws `Rel(cli, intent, "commits ...")` (`feature-delta.md:417-432`); the SSOT C4 repeats the false edge (`c4-diagrams.md:96-112`). | Correct both C4 copies to show the operator/CLI outside the Overdrive node, an existing `overdrive serve` control-plane/HTTP-handler container inside it, and `CLI → HTTP control plane → IntentStore`. Preserve the existing `IntentStore → WorkloadLifecycle/runtime` relationship and the reviewed replacement flow. This is documentation correction to current production topology, not a new component or API decision. |
| R-01 | REJECTED HYPOTHESIS — no severity | Greptile states that PR #292's VM `StartAllocation` recovery leaks predecessor netns/veth/slot state until exhaustion. | The current PR path does bypass the restart arm: VM failure selects `StartAllocation` at `workload_lifecycle.rs:1043-1071`, while predecessor network cleanup lives in the restart arm at `action_shim/mod.rs:2253-2296`. Exit-observer row publication does not itself tear down structural network state. That source trace establishes suspicion, not the claimed surviving-resource/exhaustion outcome. No bounded failing seeded production-owner regression or real-host reproduction was supplied or independently reproduced. Repository rules require that evidence before promotion. | Reject the leak/exhaustion statement as a finding and add no cleanup scanner, persistence, retry, lock or network mechanism. Preserve the existing compound restart cleanup because the user-selected action contract requires awaited predecessor cleanup; do not cite Greptile as proof of a separate defect. |

## Code-versus-design evidence review

The proposed core model is implementable with the existing API surface:

1. Base `origin/main` constructs same-ID restart at
   `workload_lifecycle.rs:1407-1444` (`alloc_id` and `spec.alloc` both use the
   predecessor). The current PR instead branches by VM/Exec. Both facts prove
   the precise correction point; neither is treated as future authority.
2. `RestartAllocation` already carries the complete successor
   `AllocationSpec`. The action shim already has both fields in one awaited
   match arm and has all required driven ports. No new action, port or driver
   method is needed.
3. The runtime's real ordering is pure reconcile → View persistence → awaited
   action dispatch → eligibility-aware requeue. Therefore a reservation in
   the returned View is genuinely durable before any successor effect and is
   retained after a typed dispatch error.
4. Current same-ID assumptions are localized but pervasive inside the restart
   arm. ADR-0105's predecessor/successor mutation universe accounts for the
   necessary ID changes: cleanup/fence/index reads use the predecessor;
   provision, SVID, start, row construction, unwind, index write and hooks use
   the successor. The design does not conceal required compiler fallout behind
   a file allowlist.
5. Observation acceptance is per allocation key. A new successor key produces
   an `Absent → Running|Failed` occurrence and cannot overwrite predecessor
   history. `current_alloc` remains a projection over accepted rows, not View
   reservations.
6. Driver capability ownership remains exact-ID. VM natural exit cleanup
   completes its driver-artifact calls before sending the exit event
   (`vm_driver.rs:2169-2237`), while VM stop awaits termination/writer handling
   and driver-artifact cleanup before returning (`:1728-1815`). Exec uses its
   `spec.alloc` key for cgroup/live state and removes/cleans that exact entry in
   `stop` (`driver.rs:466-467,646-669,699-760`). These paths support a generic
   successor identity without moving replacement policy into a driver.
7. PR #292 remains non-mergeable against the selected contract because its
   production action family, View meanings and VM-versus-Exec tests encode the
   rejected VM-only boundary. The proposal correctly requires re-DISTILL and
   re-DELIVER after design acceptance.

No additional reachable production defect was proven during this review. In
particular, theoretical cancellation/restart and Greptile's network-resource
claim were not converted into findings without the required failing seeded or
real-host reproduction.

## Architecture quality assessment

| Dimension | Assessment |
|---|---|
| Architectural bias | PASS. The proposal reuses the existing compound action and ports; it does not add a driver policy hook, action variant, persistence subsystem or compatibility layer. |
| ADR quality | PASS on context, alternatives, consequences and exact API shape; FAIL on decision authority because material D3-D6 clauses have not received conversational approval. |
| Completeness | PASS for component reuse, lifecycle owners, failure projections, evidence lanes and affected/unaffected states; FAIL for the false C4 driving boundary. |
| Implementation feasibility | PASS. The current code provides every required field and driven port, and the bounded dual-ID mutation universe is explicit. |
| Priority/scope | PASS. It corrects the reproduced #284 identity alias and rejects adjacent unproven hardening. Exec migration stays bounded to the explicitly selected temporary generic contract and GH #293. |

## Review iterations

### Iteration 1 — 2026-09-13

The selected driver-neutral predecessor→fresh-successor model is technically
coherent and implementable without new public surface. Review identified two
blocking corrections: explicit user approval is still required for the
material subordinate decisions in F-01, and the C4 driving boundary must be
made faithful to the production CLI→HTTP control-plane→IntentStore path in
F-02. Greptile's network-leak statement was rejected as unproven under the
repository reachability and reproduction rules.

## Final verdict

`CHANGES_REQUESTED`

Do not mark ADR-0105 accepted and do not begin re-DISTILL or re-DELIVER. The
original design agent should correct F-02, classify the F-01 clauses as pending
numbered user decisions, and return them through the orchestrator for explicit
approval. A fresh review iteration is required after both dispositions are
recorded.

## Architect remediation request — iteration 2

Re-review is requested; this request does not alter the iteration-1
`CHANGES_REQUESTED` verdict and does not self-approve any artifact.

The corrective DESIGN revision now:

- fixes F-02 in the canonical C4 by placing the CLI outside the node and
  showing `CLI → overdrive serve HTTP handler → IntentStore`;
- records the user's post-review reversal of cleanup-first ordering: successor
  creation does not wait for exact-old cleanup, and predecessor cleanup failure
  cannot select, consume, block or roll back the successor;
- records successor-owned identity consumption as a separate approved choice;
- keeps the seven remaining F-01 implementation-facing clauses explicitly
  proposed in the feature delta and removes the claim that no decision remains;
- identifies that current View/dispatch contracts cannot distinguish a durable
  unconsumed reservation from a consumed unpublished execution, without
  inventing an API; and
- applies the repository's new ADR-boundary rule: proposed ADR-0105 records
  only driver-neutral physical identity, ADR-0106 records only the non-gating
  cleanup decision, and ADR-0107 records only successor-owned identity
  consumption. Exact API contracts remain solely in the feature delta; C4
  remains solely in `c4-diagrams.md`; no test obligations or roadmap content
  remain in DESIGN ADRs/artifacts.

Please re-review F-02 and the ADR/content-boundary remediation now. F-01 must
remain open until the user explicitly resolves P-105-1 through P-105-7 and the
resulting exact design is re-reviewed. No DISTILL or DELIVER handoff is
requested.

## Architect ratification and re-review request — iteration 2

The same architecture reviewer is requested to perform iteration 2. This
request does not alter the iteration-1 `CHANGES_REQUESTED` verdict, record a
reviewer disposition, or self-approve the corrective DESIGN.

On 2026-09-13 the user explicitly ratified the entire surfaced P-105-1 through
P-105-7 bundle. The selected alternatives are now recorded in the canonical
feature delta:

- P-105-4A: only accepted numeric-current `Failed | Terminated` is a sufficient
  predecessor handoff; `Draining` is insufficient;
- P-105-5A: the existing durable View reservation itself consumes the
  successor ID, so unused gaps are valid; ADR-0107 is withdrawn before
  acceptance and ADR-0108 records the ratified opposite decision; and
- P-105-6A: the successor outcome completes first, then the shim makes one
  exact-old cleanup attempt; successor error has precedence if both fail,
  cleanup-only failure returns its existing typed error, and no generic retry,
  detached task or new cleanup owner is added.

P-105-1, P-105-2, P-105-3 and P-105-7 are likewise ratified with their exact
allocator, private helper, View carry and fresh-key publication/history
contracts. ADR-0105, ADR-0106 and ADR-0108 each retain one independently
reversible architectural decision; exact API contracts remain only in the
feature delta, topology only in `c4-diagrams.md`, and no DISTILL test contract
or DELIVER roadmap was added to DESIGN. F-02 remains corrected as CLI →
`overdrive serve` HTTP handler → `IntentStore`.

Please re-review F-01, F-02 and the ADR/content-boundary compliance against the
ratified canonical artifacts. No DISTILL or DELIVER handoff is requested until
the reviewer returns `APPROVED`.

## Review iteration 2 — 2026-09-13

### Metadata and authority

| Field | Value |
|---|---|
| Review role | `nw-solution-architect-reviewer` |
| Scope | Corrective application/components DESIGN re-review |
| Authority reviewed | User-ratified P-105-1 through P-105-7, selecting P-105-4A, P-105-5A and P-105-6A |
| Canonical artifacts | Feature delta; ADR-0104 corrective status; ADR-0105; ADR-0106; withdrawn ADR-0107; ADR-0108; architecture brief; C4 |
| Prior findings | F-01 and F-02 |
| New verdict | `CHANGES_REQUESTED` |

The re-review accepts the orchestrator's record that the user explicitly
ratified the complete P-105 bundle in conversation. It does not reinterpret or
replace those choices. The technical review is against the ratified
successor-first contract, not iteration 1's now-reversed cleanup-first model.

### Iteration-1 finding dispositions

| Prior finding | Disposition | Evidence |
|---|---|---|
| F-01 — material decisions lacked conversational approval | **RESOLVED** | The canonical feature delta records the dated, numbered ratification of P-105-1 through P-105-7 and the selected A alternatives (`feature-delta.md:13-18,49-62`). ADR-0105, ADR-0106 and ADR-0108 remain explicitly review-pending rather than self-accepted. ADR-0107 honestly records withdrawal before acceptance (`ADR-0107:3-9`). |
| F-02 — C4 invented CLI → IntentStore | **RESOLVED** | The canonical C4 places the CLI outside `System_Boundary(node)`, introduces the existing `overdrive serve` HTTP control-plane container, and shows `CLI → serve → IntentStore` (`c4-diagrams.md:97-127`). The feature delta now states the same real production boundary and links to the C4 instead of duplicating it (`feature-delta.md:43-46,87-90`). |

### Ratified-contract verification

| Contract | Result | Evidence and assessment |
|---|---|---|
| Driver-neutral physical identity | PASS | ADR-0105 records one independently reversible identity decision only (`ADR-0105:27-31`). The feature delta pins the existing action fields and confines driver matching to `DriverPayload` projection (`feature-delta.md:92-149`). |
| Exact existing API; no policy port | PASS | Existing `RestartAllocation { alloc_id, spec, kind }`, `Driver::start(&AllocationSpec)`, `Driver::stop(&AllocationHandle)`, View fields, observation writes and network/mTLS ports already carry both exact identities/effects. The feature adds no action, field, trait method, error type, store or schema (`feature-delta.md:110-149,185-237,382-419`). |
| Numeric-current and durable attempt allocation | PASS | The ratified checked max-suffix algorithm, View-key scan, gap acceptance and `u32::MAX` stop are exact (`feature-delta.md:153-183`). Runtime View persistence still precedes awaited dispatch (`reconciler_runtime.rs:1458-1500`), so ADR-0108's consumption boundary is supported without new state. |
| Successor-first, one exact-old cleanup attempt | PASS as a feasible existing-arm reorder | The action fields are disjoint, and all required successor and predecessor calls already live in the one `RestartAllocation` arm. Capturing `successor_outcome`, then awaiting one exact-old cleanup attempt and applying the ratified precedence table requires private control-flow reordering only (`feature-delta.md:270-333`). It does not need a detached task, cleanup queue, retry owner or policy port. |
| Cleanup protection dependencies | PASS | The ratified order preserves the existing driver-stop failure short-circuit before mTLS/network and the existing mTLS-stop short-circuit before structural teardown (`feature-delta.md:313-316`; current shim `action_shim/mod.rs:2282-2296`). Cleanup runs after successor completion but remains exact-predecessor-only. |
| Successor publication and reservation consumption | PASS | Fresh-key publication, accepted-Running hook release, `Ok(None)`/`Err` successor unwind and higher later allocation are pinned without an ADR-0099 second proposal (`feature-delta.md:335-364`). Observation acceptance is already per `AllocationId` (`observation_backend.rs:460-529,691-707`). |
| Legacy Exec boundary | PASS | Exec receives the same supplied fresh successor while present, with no same-ID compatibility branch or policy method; GH #293 remains the separate removal owner (`feature-delta.md:94-108,390-392`). The production adapter already keys cgroup/live/watcher state by `spec.alloc` (`overdrive-worker/src/driver.rs:466-467,638-657`). |
| Greptile network-leak hypothesis | REJECTED, unchanged | No required seeded production-owner or bounded real-host reproduction was added. It remains static suspicion and authorizes no new network/retry/persistence/cleanup mechanism (`feature-delta.md:82-86`). |
| Scope and PR disposition | PASS | The correction remains application/components-only, creates no component or integration, and keeps PR #292 non-mergeable until reviewed re-DISTILL/re-DELIVER (`feature-delta.md:13-46,382-419,471-482`). |

### New findings

| ID | Severity | Finding | Reachability and evidence | Required remediation |
|---|---|---|---|---|
| F-03 | HIGH — blocking | The exact public action semantics omit the production-reachable SystemGc resubmit case and therefore contradict the declared preservation of existing stop/deletion behavior. `StartAllocation` is defined as initial or another placement with **no accepted predecessor**, while a SystemGc-resubmitted workload retains an accepted historical predecessor row and still falls through to fresh placement. `RestartAllocation` is defined as every replacement with an accepted predecessor, but the ratified handoff rejects the intentionally stopped SystemGc row. A crafter cannot choose the action without inventing what “accepted predecessor” means here. | Production entry is `overdrive deploy` after the prior intent was absent and SystemGc stopped the old allocation. `WorkloadLifecycle` retains all accepted rows, filters Operator/SystemGc rows out of `active_allocs_vec` specifically so SystemGc resubmit falls through to fresh placement (`workload_lifecycle.rs:735-747,821-833`), applies only the Operator veto (`:844-860`), then emits `StartAllocation` in the scheduler path (`:1091-1173`). The retained SystemGc row is still accepted observation history; ADR-0073 explicitly preserves SystemGc resubmit as fresh placement (`ADR-0073:82-84`). The corrective feature simultaneously says `StartAllocation` requires no accepted predecessor (`feature-delta.md:130-137`) and that existing stop/deletion semantics are unchanged (`:248-252,451-461`). | Clarify the feature delta's exact action meanings without changing public surface. To preserve the accepted behavior already declared unchanged, define `StartAllocation` for initial placement **and SystemGc resubmit/fresh placement where no eligible replacement predecessor exists**, even though historical accepted rows may remain; define `RestartAllocation` for the ratified eligible accepted predecessor handoff. If the intended action family for SystemGc resubmit is instead changing, that is a new material decision and must be surfaced to the user before review. |
| F-04 | HIGH — blocking | The `Lifecycle Gate Ownership` section is mechanically incomplete under `.claude/rules/design.md`. The table names owners and one-line rules but omits the mandatory per-gate existing-evidence path, promise, affected state/result, failure projection, explicitly unaffected states, ordering/timeout, counterexample and evidence lane. It also removes all DESIGN boundary obligations, so late completion, disconnect/reopen, unrelated-state and feature-disabled behavior are not mapped to the changed handoff/non-gating cleanup gates. | This is a DESIGN gate failure, not a speculative production defect. The feature changes both the replacement handoff gate and whether predecessor cleanup gates successor creation (`feature-delta.md:254-268,270-325,421-435`; ADR-0106:27-32). `.claude/rules/design.md` requires the full declaration for every added/removed/moved gate and requires boundary obligations for available, unavailable/timeout, unrelated state, late success, disconnect/reconnect and feature disabled. The current section contains only the three-column matrix. | Complete the feature delta's Lifecycle Gate Ownership handoff for the changed gates. At architecture level, name the real production owner path and the required evidence lane for: `Failed|Terminated` versus `Draining`; durable reservation plus View reopen/gap; successor success with cleanup failure; successor error with cleanup failure and precedence; late exact-old completion/nonmutation; unrelated Job/Service/stop/health behavior; and “feature disabled = not applicable/no flag.” Keep concrete Rust scenarios, fixtures and runner mechanics in re-DISTILL as required by the content-location rule; do not put them in an ADR. |
| F-05 | HIGH — blocking | P-105-4A is an independently decidable lifecycle/ownership-handoff choice but has no decision ADR. ADR-0106 deliberately assumes an already “approved terminal/ownership handoff” and records only the later non-gating cleanup decision, so neither ADR-0105, 0106 nor 0108 owns the `Failed|Terminated`/not-`Draining` architectural choice. | The user selected P-105-4A independently (`feature-delta.md:59`), and the feature makes it the replacement-emission state boundary (`:254-268`). ADR-0105 owns only driver-neutral physical identity (`ADR-0105:27-31`); ADR-0106 begins **after** the handoff and owns only no-wait cleanup (`ADR-0106:27-32`); ADR-0108 owns only reservation consumption (`ADR-0108:22-26`). `.claude/rules/design.md` requires one ADR for each independently adoptable/reversible architectural choice and routes only the exact implementation-facing contract to the feature delta. | Add one focused, user-ratified/review-pending ADR for the P-105-4A lifecycle-handoff decision, with its decision-specific context, alternatives and consequences. Keep exact predicates/private implementation shape in the feature delta and do not duplicate them in the ADR. Do not fold P-105-4A into ADR-0106; it can be superseded independently of cleanup ordering. |
| F-06 | MEDIUM — non-blocking alone | ADR-0106's consequence says a failed one-shot predecessor cleanup remains visible as a typed action failure without qualifying the ratified both-fail precedence. In the both-fail row, the feature returns the successor error and exposes cleanup only through structured tracing. | `ADR-0106:61-62` is absolute; the normative table at `feature-delta.md:306-311` distinguishes cleanup-only failure from both-fail. | Qualify the ADR consequence: cleanup is returned as the existing typed action error when the successor outcome succeeded; when both fail, the successor error remains returned and cleanup is secondary structured tracing. This preserves the one-decision ADR while making its consequence accurate. |

### ADR and content-location audit

- ADR-0105: PASS — one driver-neutral allocation-identity choice; no signatures,
  test matrix, C4 or roadmap content.
- ADR-0106: PASS on one-decision scope; F-06 corrects one consequence sentence.
- ADR-0107: PASS — clearly withdrawn before acceptance and points to the
  ratified opposite decision without masquerading as authority.
- ADR-0108: PASS — one reservation-consumption boundary; no interface/test/C4
  duplication.
- Feature delta: PASS as the home for exact public/private action, View and
  failure contracts, subject to F-03 and the mandatory gate handoff in F-04.
- Architecture brief: PASS as high-level status and decision links only.
- C4: PASS as the sole detailed topology and relationship artifact.
- P-105-4A: FAIL under one-decision-per-ADR; see F-05.

No additional ADR is requested for the exact private helper signature, outcome
table or field-by-field View manipulation: those correctly belong in the
feature delta. This review does not expand the design into a new cleanup,
reservation, retry or driver-policy mechanism.

### Code-versus-design conclusion

The ratified successor-first mechanism is feasible over the current component
surface. The existing action already carries predecessor and successor IDs;
the runtime already durably consumes View changes before dispatch; and the
action shim already owns every successor and predecessor effect needed to
capture two outcomes and apply the chosen precedence. Distinct allocation keys
prevent old driver capabilities from naming successor VM/Exec artifacts.

The review found no evidence requiring a new action, port, driver-policy
method, store/schema, generic retry, detached task, lock or network mechanism.
It also did not promote Greptile's unproven leak assertion. The remaining
blockers are exact-contract and DESIGN-governance defects: one reachable action
case is under-specified, the mandatory gate handoff is incomplete, and one
independently reversible lifecycle decision lacks its ADR.

### Iteration-2 verdict

`CHANGES_REQUESTED`

F-01 and F-02 are closed. Do not mark the corrective DESIGN accepted or begin
re-DISTILL/DELIVER until F-03, F-04 and F-05 are remediated and independently
re-reviewed. F-06 should be corrected in the same documentation pass. Preserve
ADR-0107 as withdrawn and preserve the user-ratified P-105-5A/P-105-6A choices;
none of these findings authorizes a different mechanism.

## Architect final remediation record — no iteration 3 requested

The user capped corrective DESIGN review/remediation at two cycles after the
iteration-2 verdict. This is the architect's bounded documentation disposition
of F-03 through F-06, not a reviewer verdict, re-review request, or
self-approval:

- **F-03 addressed:** the feature delta now defines `StartAllocation` for
  initial placement and fresh placement with no *eligible replacement
  predecessor*. It names the production-reachable SystemGc resubmit explicitly:
  its retained accepted row remains historical but is excluded by the existing
  intentional-stop/cause gate, so existing `StartAllocation` behavior is
  preserved. `RestartAllocation` applies only after the ratified eligible
  predecessor gates pass.
- **F-04 addressed:** the feature delta now includes the required existing
  state-ownership matrix and full declarations for G-1 terminal predecessor
  handoff, G-2 durable identity consumption and G-3 successor-first/post-
  successor cleanup. It maps available, unavailable/timeout, unrelated-state,
  late-completion, disconnect/reopen, duplicate/re-drive and no-feature-flag
  obligations to architecture-level evidence lanes while leaving concrete
  scenarios, fixtures, seeds and runners to DISTILL.
- **F-05 addressed:** ADR-0109 now records only the independently reversible
  P-105-4A terminal predecessor-handoff decision. Exact predicates remain only
  in the feature delta; it duplicates no API, C4, test or roadmap content.
- **F-06 addressed:** ADR-0106's consequence now says cleanup-only failure is
  returned as the existing typed action error, while both-fail returns the
  successor error and reports cleanup through secondary structured tracing.

No ratified mechanism, public/private API, component, owner, retry,
persistence, cleanup or networking contract was added. No iteration 3 is
requested or authorized. The on-disk reviewer verdict remains
`CHANGES_REQUESTED`; explicit user disposition is required before the corrected
DESIGN can become implementation authority or proceed to re-DISTILL.
