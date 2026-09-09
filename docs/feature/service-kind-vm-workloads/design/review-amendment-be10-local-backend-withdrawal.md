# DESIGN review — BE10 local direct-VIP withdrawal, ADR-0101 revision 4

| Field | Value |
|---|---|
| Feature | service-kind-vm-workloads |
| Amendment under review | docs/feature/service-kind-vm-workloads/design/amendment-be10-local-backend-withdrawal.md |
| ADR under review | docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md, revision 4 D7 |
| Related ruling | docs/feature/service-kind-vm-workloads/design/backend-eligibility-convergence-ruling.md |
| Review type | Fresh isolated nw-solution-architect-reviewer DESIGN review |
| Reviewer | nw-solution-architect-reviewer |
| Model | User-selected GPT5.6 Luna, maximum thinking |
| Review date | 2026-09-08 |
| Iteration | 1 |
| Verdict | **APPROVED** |

## Scope and review boundary

This is a focused application/component review of the BE10 local non-mesh
direct-VIP consumer correction. It reviews only the necessity, exact contract,
ownership, ordering, and evidence boundary of revision 4 D7. ADR-0101 revision
3's independently approved sole-publisher architecture is carried as context;
it is not reopened here. This review does not amend BE02, the original E09
mesh/VM path, lifecycle policy, replica scheduling, persistence, retry or
acknowledgement architecture, or any existing-flow revocation behavior.

I read AGENTS.md, CLAUDE.md, all seven mandatory .claude/rules files, the
nw-solution-architect-reviewer role, and its required nw-sar-critique-dimensions
and nw-roadmap-review-checks skills. Roadmap-specific checks are not applicable:
the reviewed artifact is a design amendment, not a roadmap. No production,
test, expectation, design, DES, roadmap, or native-host file was edited. No
test, native experiment, mutation run, commit, or additional agent was started.

The reviewed design inputs were pinned at review time as follows:

| Artifact | SHA-256 |
|---|---|
| design/amendment-be10-local-backend-withdrawal.md | 672700a5efe728658a57b283be649418d012659f5c2eba33c9f19fd0a6b9d3e3 |
| adr-0101-service-backend-health-observed-convergence.md | f072cced4ccaea5a45a0f318ceb9af6b3bc62daad27c493020cee06df77c462a |
| design/backend-eligibility-convergence-ruling.md | 0e740b934f31f0c3b59d049b9811661ddbc69a4f56cc339e7d43fb0a86c31b02 |
| deliver/adr-0101-test-transfer.md | 29ad26379bd27ca06935b1822e82111fff061e643685fe33be8eb75d077630ff |
| crates/overdrive-sim/tests/integration/service_backend_projection.rs | a54291b1ed9f483dbc22cae6453314f631b390d95837a2e0845f3db1a5913206 |

## Necessity and revalidated production path

The supplied seed is a sufficient scoped reproduction. The recorded BE10 run
(21245211-7507-4f57-b5c1-48d450097fc5, seed 257221) drives the real
ServiceLifecycle, ProbeRunner, observation publication, broker handoff, and
queued ServiceMapHydrator through the production composition used by the Sim
fixture. It reaches the local-map assertion after a healthy registration, a
real readiness Fail, the authoritative unhealthy ServiceBackendRow, mesh and
DNS withdrawal, and the actual queued hydrator evaluation. The local map is
still Some(192.0.2.10:18081) where BE10 requires None
(docs/feature/service-kind-vm-workloads/deliver/adr-0101-test-transfer.md:145–196;
crates/overdrive-sim/tests/integration/service_backend_projection.rs:985–1019).

The current owner and caller path explains that failure without a fabricated
row or test-only state:

1. ServiceLifecycle computes the materialized health bit from the existing
   readiness policy and writes the complete backend row, then enqueues the
   hydrator (crates/overdrive-reconcilers/src/service_lifecycle.rs:1092–1167).
   The allocation remains Running; readiness withdrawal is not a lifecycle
   stop or restart decision.
2. The row executor awaits the existing ObservationStore write, and the
   runtime awaits the existing serial action-shim dispatch
   (crates/overdrive-control-plane/src/action_shim/write_service_backend_row.rs:46–58;
   crates/overdrive-control-plane/src/reconciler_runtime.rs:1458–1501,
   1528–1573).
3. ServiceMapHydrator excludes mesh-subnet addresses before partitioning
   non-mesh backends into local and remote candidates, and its local fingerprint
   includes the full Backend value, including healthy
   (crates/overdrive-reconcilers/src/service_map_hydrator.rs:336–371,
   442–480; crates/overdrive-core/src/dataplane/fingerprint.rs:43–65,
   151–159). On a health flip the existing local-fingerprint gate therefore
   runs, but the current helper emits RegisterLocalBackend without inspecting
   Backend.healthy (service_map_hydrator.rs:456–477, 580–634).
4. The existing shim exhaustively dispatches and awaits both local action
   variants (crates/overdrive-control-plane/src/action_shim/mod.rs:922–953,
   3038–3054; register_local_backend.rs:66–78;
   deregister_local_backend.rs:53–65).
5. The host adapter's registration writes the reverse local map before the
   forward map, while its existing deregistration removes the forward entry
   and then the caller-supplied reverse key
   (crates/overdrive-dataplane/src/lib.rs:2158–2183, 2207–2235).
   The local cgroup hook uses only the forward (VIP, port, protocol) lookup
   and rewrites a new connect on a hit; it has no health field or policy read
   (crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:34–101).

This proves the amendment's necessity for the demonstrated still-present,
single-Running local candidate: absence of a new registration does not remove
an already installed address-only rewrite. It does not claim that native
unhealthy routing has been reproduced, and it does not claim that this local
map path caused the original mesh-selected VM E09 failure. Those evidence
boundaries are correctly retained in the amendment.

## Contract and API assessment

**Approved.** The correction is the smallest existing-surface action change
that closes the reproduced successful-effect trajectory.

The exact contract is implementable and does not invent public API:

- No public type, field, action variant, method, parameter, error, row,
  adapter, persistence, or consumer interface changes.
- The only named reconciler implementation change is the private helper rename
  from push_register_local_backend_actions to push_local_backend_actions with
  the exact existing arguments (&mut Vec<Action>, &[&Backend],
  &LocalBackendEmit<'_>). Its context fields remain service_id, vip_v4, vip_port,
  proto, target_str, and spec_hash with their existing types. In-module test
  references and documentation are mechanical fallout from that private
  rename, not new surface.
- On the existing local-fingerprint change, each classifier-accepted local
  candidate produces exactly one existing action: healthy true selects
  Action::RegisterLocalBackend with purpose register-local-backend; healthy
  false selects Action::DeregisterLocalBackend with purpose
  deregister-local-backend.
- Both variants retain exactly service_id, vip, vip_port, proto,
  backend: SocketAddrV4, and correlation. The VIP listener port and protocol
  come from the existing keyed listener fact; the backend tuple retains its
  own address and port; correlation remains CorrelationKey::derive(target,
  spec_hash, purpose) with target service-map-hydrator/{service_id} and the
  full desired-set content hash.
- The existing async port signatures and typed error chain are reused exactly.
  The action shim awaits the operation, DeregisterLocalBackend retains its
  existing forward-then-caller-keyed-reverse dual removal, and absent keys
  remain idempotent success.

The amendment correctly retains the complete local candidate vector, including
unhealthy candidates, through fingerprinting and action construction. That is
necessary because the still-present unhealthy candidate supplies the reverse
key for the existing removal port. It also correctly preserves remote action
selection, remote retry memory, mesh exclusion, IPv6/address classification,
listener lookup, action ordering, and the existing View field's limited
meaning: last_applied_local_fingerprint records emission input, not local
effect acknowledgement.

## Ownership, lifecycle, and effect boundaries

**Approved.** The lifecycle matrix is internally consistent and uses the
existing owner chain:

| Concern | Exact owner and effect | Unchanged boundary |
|---|---|---|
| Materialized backend eligibility | ServiceLifecycle alone constructs the authoritative Backend.healthy bit in ServiceBackendRow | Running, Stable, readiness policy, liveness, restart authority, and terminal reporting |
| Local action plan | Pure ServiceMapHydrator::reconcile selects an existing action from the materialized bit and current non-mesh candidate | Remote plan, local fingerprint/View shape, mesh/DNS readers, and no-change gate |
| Local map effect | Serial action shim awaits the existing Dataplane port; EbpfDataplane or SimDataplane applies the existing forward/reverse post-state | No map readback, new acknowledgement, new persistence, or new policy owner |
| New direct-VIP selection | Existing cgroup hook rewrites only when the forward entry is present | A map miss allows the operator-supplied destination unchanged; no firewall denial or established-flow revocation |

The amendment does not add, move, or remove a lifecycle gate. It closes a
consumer-side bypass of the already-published health bit at the local map
effect boundary; the consumer does not decide health. The normal production
loop awaits each convergence tick before checking shutdown, and the existing
action shim drains later actions after an individual error
(crates/overdrive-control-plane/src/lib.rs:3363–3389;
action_shim/mod.rs:922–953). No cancellation, reconnect, or forced-abort
behavior is introduced or used as the premise.

The stated boundary is appropriately narrow: true registers, false removes,
and a later true uses the same existing registration path. It applies to the
still-present single-Running local candidate demonstrated by BE10. Membership
disappearance, changed-address reverse-key cleanup, map-drift repair,
restart replay, generic absent-entry cleanup, and multi-replica arbitration
remain explicitly outside this amendment. BE02 remains a separate unresolved
fixture and is not silently amended here.

## Architecture-quality dimensions

| Dimension | Result | Review basis |
|---|---|---|
| Architectural bias | **PASS** | The choice reuses an existing awaited action/port already required to remove a local map entry. No new technology, dependency, service, adapter, policy store, or consumer protocol is selected. Filtering to no action cannot remove a prior registration; readback/ack/retry machinery is generalized hardening outside the reproduced trajectory. |
| ADR quality | **PASS** | Context, production reachability, decision, exact private/public contract, alternatives, consequences, lifecycle ownership, error semantics, and explicit non-guarantees are stated. Revision 4 is clearly marked pending and does not overwrite revision-3 provenance. |
| Completeness and quality attributes | **PASS for this bounded amendment** | Reliability is stated as successful effect completion, without disguising failed-effect recovery as a guarantee. Performance remains one existing fingerprint pass and one existing action per accepted local candidate. Security/data-plane behavior is limited to the existing address-only cgroup rewrite and exact key removal. Maintainability and testability use the existing pure reconcile/action and port boundaries. Existing typed errors and action draining supply the stated observability boundary. |
| Implementation feasibility | **PASS** | Existing Action variants, validator, async Dataplane methods, both adapters, and serial shims already provide the required shape. No runtime discovery, detached future, new port, or compiler-invented public API is necessary. |
| Priority validation | **PASS** | Seed 257221 identifies the largest defect within this consumer: a materialized unhealthy local candidate changes the fingerprint but still re-registers. The proposed action selection directly addresses that failure, and the rejected options and out-of-scope recovery/replica choices are explicit. |
| Effect isolation | **PASS** | The linked reuse analyses classify the hydrator delta as a pure action-plan change over one service's local candidate universe, and the existing action/Dataplane paths as bounded forward/reverse key effects. Read-only hydration remains in existing read ports; capability injection remains through HydrationContext and &dyn Dataplane. No side effect is moved into reconcile. |

## Evidence and verification boundary

### Performed in this review

- Read the amendment, ADR-0101 revision 4 D7, the backend-eligibility ruling,
  the narrow architecture brief and feature-delta sections, the test-transfer
  evidence, and the approved BE10 composition source.
- Inspected the current production caller/owner chain through ServiceLifecycle,
  runtime persistence/dispatch, ServiceMapHydrator, both action shims, the
  Dataplane trait, EbpfDataplane, SimDataplane, and the cgroup hook.
- Verified the exact field, parameter, correlation, key, and ordering claims
  against current source. In particular, the existing fingerprint archives
  Backend.healthy, the existing deregistration accepts a caller-supplied
  backend tuple, and the cgroup lookup has no health check.
- Accepted the recorded seed-257221 run as supplied evidence and revalidated
  its claimed path against source. It was not rerun, consistent with the
  instruction not to execute tests or native suites during this review.

### Unexecuted and not claimed

- The BE10 recovery suffix in service_backend_projection.rs:1021–1037 is
  still unexecuted in the supplied run and remains a downstream acceptance /
  implementation obligation.
- Native unhealthy direct-VIP routing after withdrawal was not reproduced.
  The amendment makes no native claim; a later authorized Tier-3 check must
  use the existing production dispatch, attached cgroup, healthy control, and
  new connection boundary.
- Production implementation, native/full suites, mutation testing, and
  black-box expectations were not run or inferred from the Sim result.

These limits do not block this design review because the amendment explicitly
hands them to the acceptance/implementation workflow and does not claim the
unexecuted outcomes. They must not be converted into a new API, retry protocol,
native synthetic hook, or BE02 expansion.

## Findings and dispositions

No critical, high, medium, or low design finding remains within the stated
BE10 amendment boundary. In particular, the following are verified design
limits rather than defects to remediate:

| Observed concern | Disposition |
|---|---|
| The local View marker is persisted before effect dispatch and does not guarantee retry after a failed unchanged-fingerprint effect. | Explicitly disclosed by the amendment; no retry/readback architecture is authorized by this focused task. |
| Native unhealthy routing is not yet reproduced. | Correctly retained as an unexecuted downstream evidence boundary; no native claim is made. |
| BE02 may expose multiple local candidates and existing same-slot action constraints. | Explicitly separate; no replica scheduling or arbitration rule is added here. |
| Existing connections are not revoked by a new map miss. | Correctly stated; the amendment concerns new direct-VIP selection and existing dual-map teardown only. |

No finding authorizes changing a lifecycle owner, adding persistence or
fencing, strengthening consumer acknowledgement, repairing absent membership,
or broadening the local effect universe.

## Iteration history

| Iteration | Reviewed revision | Verdict | Findings | Disposition |
|---:|---|---|---|---|
| 1 | ADR-0101 revision 4 D7 focused BE10 local direct-VIP amendment | **APPROVED** | None remaining in scope | Exact existing action selection is ready for the separately owned implementation and acceptance workflow. |

## Final verdict

**APPROVED.** The amendment is necessary for the reproduced BE10 mismatch and
is technically sufficient within its declared scope: a materialized healthy
local candidate registers through the existing path, a materialized unhealthy
still-present candidate deregisters through the existing awaited dual-removal
path, and recovery registers again. The exact public API remains unchanged,
the private helper rename is pinned, ServiceLifecycle remains the sole health
publisher, and all remote/mesh/lifecycle/replica/retry boundaries are
preserved.

This approval is for the focused revision-4 DESIGN amendment only. It does not
change ADR-0101's authoritative pending metadata, claim native unhealthy
routing evidence, mark the recovery suffix complete, or claim implementation
GREEN. Those next-gate obligations remain with the designated owners.
