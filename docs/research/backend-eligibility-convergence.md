# Research: backend eligibility convergence

Date: 2026-09-08. Researcher: Codex executing nw-researcher. Scope: evidence-led comparison of the demonstrated E09 v2 backend eligibility convergence failure; research only, not an approved design.

## Executive summary

The failure is a stale convergence comparison, not a missing startup verdict: ServiceLifecycle still computes ineligible, but compares it with what it last emitted instead of the row another owner has since rewritten. The retained seed 257209 demonstrates the persistent mismatch through real production owners. Current source still contains that path.

Kubernetes EndpointSlice and Consul independently support the same bounded principle: derive the decision from authoritative inputs, compare it with observed output, and respect the other owner's fields. Neither system establishes that Overdrive needs new persistence, attempt identities, restart policy, or a replacement owner. The smallest evidence-supported candidate is health-only reconciliation against the existing observed backend row. That is a DESIGN option requiring a pinned contract and executable proof—not a completed fix. Eventual repair and exclusion at every instant are different promises.

## Method

Read the attached diagnosis and seeded witness first, then inspected their production callers and accepted contracts before web research. Searches targeted official Kubernetes EndpointSlice/controller implementation, HashiCorp Nomad service/check ownership and Consul anti-entropy implementation, and Envoy discovery/health semantics. Only publisher-owned documentation and repositories support external claims. Findings were written progressively; no production, test, design, expectation, or DES file was changed.

## Local evidence and contract boundaries

Grounding: `docs/analysis/e09-v2-failed-service-reachability.md`, its seed-257209 regression witness, current production owner paths, and ADR-0096/0099. HEAD is `480501fd03df4b0e749e026246312b3f3099ae4e`; this is a dirty shared workspace. Source inspection is current; native and Sim executions are attributed to the retained diagnosis, not rerun by this researcher.

The demonstrated sequence is precise:

1. Production `ServiceLifecycle` decides startup failure, retains the allocation ID in `terminal_announced`, and emits the unhealthy backend row before `FinalizeFailed` ([source](../../crates/overdrive-reconcilers/src/service_lifecycle.rs), lines 660–704).
2. `BackendDiscoveryBridge` projects only Running allocations. Failed therefore removes membership; the subsequent Running observation for the **same allocation ID** recreates membership. The bridge preserves an existing member's observed health, but defaults a missing member to true ([source](../../crates/overdrive-reconcilers/src/backend_discovery_bridge.rs), lines 376–396, 525–541).
3. `ServiceLifecycle` still derives false for that ID. Its desired fingerprint, however, equals its last-emitted fingerprint, so it emits no repair despite the stored true bit. Its existing backend-row read discards contents and retains only the timestamp ([source](../../crates/overdrive-reconcilers/src/service_lifecycle.rs), lines 954–963, 1130–1150, 1178–1189).
4. The resolver correctly filters on the stored healthy bit; it does not own the terminal veto ([source](../../crates/overdrive-control-plane/src/mtls_resolve_adapter.rs), lines 491–545). Thus the wrong value reaches a correct consumer.

The [seeded witness](../../crates/overdrive-sim/tests/e09_v2_failed_service_reachability_spike.rs) drives existing registered reconcilers, action dispatch, and ProbeRunner. Production authors all disputed observations and view inputs. After membership reinsertion, six service ticks leave health true while the terminal set still contains the ID. The [diagnosis](../analysis/e09-v2-failed-service-reachability.md) records two failed runs with seed 257209 and independent native traffic to replacement VM attempts. This research does not claim a new execution or universal schedule proof.

Restart ownership is not missing: `restart_allocation_action` deliberately copies `row.alloc_id` ([source](../../crates/overdrive-reconcilers/src/workload_lifecycle.rs), lines 1426–1469). The shim awaits prior-driver stop/typed absence and existing network cleanup, then requires accepted replacement Running publication before its post-start effects ([source](../../crates/overdrive-control-plane/src/action_shim/mod.rs), lines 2260–2302, 2612 onward). The production convergence loop awaits complete ticks before checking shutdown ([source](../../crates/overdrive-control-plane/src/lib.rs), lines 3377–3403). No manufactured cancellation or failed store write is necessary to explain this defect.

[ADR-0096 D1/D3](../product/architecture/adr-0096-startup-failure-withdraws-backend-eligibility.md) assigns health policy to ServiceLifecycle, membership to the bridge, and restart decisions to WorkloadLifecycle. Its distinct-replacement-ID prose conflicts with both current code and [accepted ADR-0099's lifecycle matrix](../product/architecture/adr-0099-restart-running-write-acknowledgement.md). ADR-0096's header remains Proposed although its [independent review ends APPROVED](../feature/service-kind-vm-workloads/design/review-adr-0096.md). These are documented discrepancies, not permission to change restart policy. The scoped problem is failure to converge to an **already-active** health decision.

## Comparable systems

### Kubernetes EndpointSlice: observed-state diff within an explicit object owner

**Verified facts.** EndpointSlice exposes separate `serving`, `terminating`, and `ready` conditions. Multiple managers partition slices using `endpointslice.kubernetes.io/managed-by`; this is not field ownership shared by two controllers inside one slice. [Official EndpointSlice concepts](https://kubernetes.io/docs/concepts/services-networking/endpoint-slices/).

Pinned v1.34.0 `podToEndpoint` derives `serving` from Pod Ready, `terminating` from deletion timestamp, and `ready` from `publishNotReadyAddresses || (serving && !terminating)`. It reconstructs these values from inputs, including for newly constructed endpoints; presence alone does not supply readiness. [Kubernetes v1.34.0 source, lines 33–52](https://github.com/kubernetes/kubernetes/blob/v1.34.0/staging/src/k8s.io/endpointslice/utils.go#L33).

The controller lists existing slices for its Service **and manager**, rejects a known stale informer snapshot, and passes those observed objects into reconciliation. Unexpected managed-slice changes enqueue the Service. This is cached observation, not a guaranteed linearizable read on every tick. [Controller source, lines 365–389 and 418–455](https://github.com/kubernetes/kubernetes/blob/v1.34.0/pkg/controller/endpointslice/endpointslice_controller.go#L365). The reconciler compares desired endpoints against endpoints actually in those slices, updates unequal matches, and records its slice tracker after successful API writes. It does not equate unchanged desired output with unchanged external state. [Reconciler source, lines 420–426 and 450–510](https://github.com/kubernetes/kubernetes/blob/v1.34.0/staging/src/k8s.io/endpointslice/reconciler.go#L450).

**Applicability — inference.** The relevant lesson is to compare the owned value with observed state and keep the ownership boundary explicit. Kubernetes does **not** prove that two whole-row writers can independently regenerate Overdrive membership without conflict. Copying its one-projector/object-partition arrangement would change Overdrive's approved owner topology. Retaining current owners while diffing only health is a smaller adaptation, not a literal Kubernetes implementation. Costs of the Kubernetes pattern include object/list/watch state and asynchronous propagation; it supplies no zero-time exclusion guarantee for this Overdrive path.

**Confidence:** Medium under the research rubric: official docs plus pinned implementation agree, but are one independent organizational source family, not three independent confirmations.

### Nomad with Consul: registration and checks are distinct; sync markers are revalidated

**Verified facts.** Nomad owns service registration/update/deregistration with the configured provider. Group services/checks register before tasks start; task services/checks register after the task starts. Registration therefore is not itself the health verdict. [Nomad service lifecycle](https://developer.hashicorp.com/nomad/docs/job-specification/service#lifecycle). With Consul, the health API returns service instances with associated checks; `passing=true` filters to instances whose checks all pass. Plain health queries do not enable that filter by default. [Consul health API](https://developer.hashicorp.com/consul/api-docs/health#list-nodes-for-service). Restart-on-check-failure is separately configured by `check_restart`; a zero limit disables it. This does not equate discovery, health, and restart decisions. [Nomad check_restart specification](https://developer.hashicorp.com/nomad/docs/job-specification/check_restart).

Consul agents periodically reconcile local authoritative service/check state with catalog state, not only on new local transitions. [Consul consistency documentation](https://developer.hashicorp.com/consul/docs/concept/consistency). In pinned v1.21.0, `SyncFull` first calls `updateSyncState`, which queries remote services and checks, clears `InSync` for missing entries, and recomputes equality for present checks; only then does `SyncChanges` submit differences. A prior successful emission does not permanently suppress repair. The same code adopts externally owned tags when `EnableTagOverride` is enabled and preserves server-owned reserved tagged addresses before comparison. [Consul local-state source, lines 948–1148](https://github.com/hashicorp/consul/blob/v1.21.0/agent/local/state.go#L948).

**Applicability — inference.** This is the closest direct analogue for correcting the stale emit marker: a remembered sync result is an optimization that must be checked against the actual destination. Its field-preservation exceptions also demonstrate that reconciliation need not reclaim another owner's fields. However, Consul's local-agent/catalog authority and separate check records are **not** Overdrive's two writers of one `ServiceBackendRow`. Adding a Consul-shaped check store, TTL, or anti-entropy subsystem would be architectural expansion. Retaining an existing row read and comparing the owned health field does not require those mechanisms. Network synchronization/readback costs and delay exist in Consul; Overdrive already performs the relevant keyed row read.

No cited source establishes that Consul preserves a terminal startup veto forever across service deregistration/re-registration. Separate records alone do not settle identity lifetime or initialization policy. Likewise, Nomad's task restart is not a reason to change Overdrive's same-ID restart contract.

**Confidence:** Medium: official documentation and implementation corroborate behavior within the HashiCorp source family; transfer to Overdrive is explicitly inference.

### Envoy: useful separation, but a different eligibility contract

**Verified facts.** Envoy EDS supplies endpoint membership and attributes. Its eventual-discovery model combines membership with local active health checks, and can retain an absent endpoint that continues passing checks. [Envoy service discovery](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/service_discovery). Its panic mode can deliberately disregard unhealthy status and choose all hosts or no hosts, according to configuration. [Envoy panic threshold](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/panic_threshold.html).

**Applicability — inference.** This illustrates why membership and health must be named separately, but is not a drop-in model for Overdrive's terminal startup veto. The E09 negative VM deliberately serves on 18081 while its startup check fails on 18999. A new traffic-port health checker could report success without satisfying that startup contract. Retaining reachable but withdrawn endpoints or adopting fail-open panic behavior would contradict the scoped veto, not repair its publication. Moving policy into the mesh resolver would also change ownership. Active checks add probe traffic and another decision boundary; this diagnosis does not justify them.

**Confidence:** Low for transferability; both facts have authoritative documentation, but this is one source family and deliberately a limited comparison, not implementation verification.

## Alternatives and applicability

The following are research inferences, not prescribed API or approved scope:

| Pattern | Effect on the demonstrated sequence | Cost / boundary |
| --- | --- | --- |
| Existing health owner compares computed health with matching members of the observed row, preserving membership/address/weight | Can detect true after reinsertion even though desired false never changed; removes the seed's reason for permanently skipping correction | Retain contents from an existing read; scan/clone the observed row and write only on health drift. Exact missing-row/member behavior belongs in DESIGN. Keeps the existing physical full-row action and semantic owners. |
| Blindly regenerate and reassert the entire row each service tick | Could overwrite the demonstrated true value, but does not respect which owner decides membership | Repeated writes/gossip and duplicate membership derivation; unsuitable evidence for a health-only correction. |
| Revalidate a cached output marker against destination identity/version | Can make a marker safe only if invalidation includes the actual destination changes | More cache-invalidation state than comparing the health already read; success acknowledgment alone does not detect a later legitimate bridge overwrite. |
| One projector writes membership plus health, or separately stored facts are joined at consumption | Can avoid the specific last-writer interaction if identity/default rules are specified | Kubernetes-style object ownership or Consul-style separate records require ownership/data-model changes here. Neither is shown necessary by seed 257209. |
| Add faster wakes/resync without changing the diff | Does not fix it: the witness already runs six further ServiceLifecycle ticks | Scheduling work while retaining the same no-op comparison. No broker redesign follows from this failure. |

The bounded health-overlay candidate accords with the verified readback and ownership principles. It should not add members the bridge has not published or recalculate their addresses/weights. Nor should an unchanged *desired* fingerprint remain the skip condition. Its precise state/signature and behavior when the row is absent must be pinned by DESIGN; this report invents no public surface.

The candidate's honest claim is convergence once a service tick observes the reintroduced member and its corrective write succeeds. It does **not** prevent the bridge's initial true publication from being visible before that tick, or provide an atomic multi-owner transaction. The existing serial tick owner and successful-write witness are enough to motivate the bounded repair; hypothetical concurrent stale snapshots, failed writes, or process crashes are not additional proven defects. A stronger no-intermediate-eligibility requirement must be explicitly scoped and independently demonstrated before expanding the mechanism.

The meaningful verification remains the production-owner seed, with the initial nonterminal/no-readiness control retained, followed by ordinary membership and service ticks that reach a stable row without undoing another owner's fields. Native built-product E09 remains the independent traffic-outcome lane. Neither external precedent nor this report substitutes for those results, or for independent DESIGN review.

## Knowledge gaps and source analysis

### Gaps and conflicts

- **Not rerun here:** the diagnosis pins retained native and Sim failures to the same HEAD plus dirty source identity. This research read that evidence and current source; it did not acquire a Linux host or claim fresh test results. Rerun the witness against any implementation change.
- **No exact external twin:** searches found clear readback and ownership precedents, but no verified example of precisely two reconcilers sharing Overdrive's full-row LWW `ServiceBackendRow` with this same-ID terminal marker. Treat the health overlay as a locally grounded adaptation.
- **Identity lifetime:** neither Kubernetes's Pod condition projection nor Consul's registration/check lifecycle resolves the ADR-0096 distinct-ID mistake. Current WorkloadLifecycle and ADR-0099 establish same-ID attempts. Deciding when a future attempt may shed the terminal veto is a different policy question, not answered or authorized here.
- **Promise strength:** the witness proves persistent wrong eligibility after further ticks, not a latency bound. Readback repair cannot alone prove no packet traverses a transiently true row. This distinction is retained rather than silently strengthening or weakening the requirement.
- **Documentation conflicts:** ADR-0096 Proposed metadata versus approved review, and distinct-ID prose versus code/ADR-0099, remain untouched. Envoy's broad discovery matrix must be read alongside its panic-mode qualification; it is not an unconditional fail-closed guarantee.
- **Retrieval limitations:** the old Consul `/docs/architecture/anti-entropy` URL failed; the current `/docs/concept/consistency` page and pinned implementation were read instead. Two attempted Envoy health-status URLs failed; no claims rely on them. Official discovery and panic pages supplied the bounded Envoy comparison.

### Source register

All sources below were read on **2026-09-08**, are primary/publisher-owned, and rate **High (1.0)** for provenance. This is a provenance score, not a probability that a proposed Overdrive fix is correct. Source-specific claims meet the one-authoritative-source minimum. Documentation and code from one organization are not counted as independent confirmations; the cross-system readback inference has independent Kubernetes and HashiCorp support. Vendor docs have product advocacy interests, so implementation was used where the mechanism mattered.

| Source | Version/revision | Verification |
| --- | --- | --- |
| [Kubernetes EndpointSlice concepts](https://kubernetes.io/docs/concepts/services-networking/endpoint-slices/) | Unversioned live docs; no release-latest claim | Cross-checked with the following pinned code |
| [Kubernetes endpoint projection](https://github.com/kubernetes/kubernetes/blob/v1.34.0/staging/src/k8s.io/endpointslice/utils.go#L33) | v1.34.0 | Source read |
| [Kubernetes controller](https://github.com/kubernetes/kubernetes/blob/v1.34.0/pkg/controller/endpointslice/endpointslice_controller.go#L365) | v1.34.0 | Source read |
| [Kubernetes endpoint reconciliation](https://github.com/kubernetes/kubernetes/blob/v1.34.0/staging/src/k8s.io/endpointslice/reconciler.go#L450) | v1.34.0 | Source read; confirms observed-state comparison |
| [Nomad service lifecycle](https://developer.hashicorp.com/nomad/docs/job-specification/service#lifecycle) | Live docs; selector displays v2.0.x | Official contract read |
| [Nomad check_restart](https://developer.hashicorp.com/nomad/docs/job-specification/check_restart) | Live docs; selector displays v2.0.x | Official contract read |
| [Consul health query](https://developer.hashicorp.com/consul/api-docs/health#list-nodes-for-service) | Live API docs | Official contract read |
| [Consul consistency](https://developer.hashicorp.com/consul/docs/concept/consistency) | Live docs; selector displays v2.0.x | Mechanism cross-checked with pinned source |
| [Consul local state](https://github.com/hashicorp/consul/blob/v1.21.0/agent/local/state.go#L948) | v1.21.0 | Source read; historical implementation pin, not latest-release claim |
| [Envoy discovery](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/service_discovery) | Rendered 1.40.0-dev-0828fd | Official architecture contract; no source audit |
| [Envoy panic mode](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/panic_threshold.html) | Rendered 1.40.0-dev-0828fd | Qualifies the discovery/health interpretation |

Research metadata: 11 cited external sources, three organizational source families, plus current local source and retained diagnosis. Overall transfer confidence: **Medium**. No further architectural research is necessary to explain the demonstrated stale comparison; implementation-specific correctness remains an executable-evidence obligation after the bounded design is approved.
