# DESIGN review — ADR-0101: ServiceLifecycle converges health against observed backend membership

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| ADR under review | `docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md` |
| Related ruling | `docs/feature/service-kind-vm-workloads/design/backend-eligibility-convergence-ruling.md` |
| Review type | Fresh isolated `nw-solution-architect-reviewer` DESIGN review |
| Iteration | 1 |
| Reviewed revision | Prior bounded-convergence proposal, pinned by the SHA-256 values below |
| Verdict | **CHANGES_REQUESTED** |

## Scope and revision boundary

This is a bounded review of the proposal that existed when the following
content hashes were recorded on 2026-09-08. It is not a review of a later
architectural comparison or a later revision of either proposal document. Any
later revision needs its own review disposition; this artifact must not be read
as implementation authorization for a replacement design.

| Reviewed artifact | SHA-256 |
|---|---|
| `design/backend-eligibility-convergence-ruling.md` | `719be4752737d8f2d3988c0bfb0773c1003e6857c369762f956544f062e0e6ef` |
| `adr-0101-service-backend-health-observed-convergence.md` | `82512cf1c502e1dbb2d78594ec2e15402795f47932622555d9f910d6418ff7e5` |
| `docs/product/architecture/brief.md` | `1006b22953715f9d9f701b8263e73a06ddacd9f9095a13f43279221883cb8a06` |
| `feature-delta.md` | `8b8efc4106a738624731cdb139808c291b08cebfa5cd2eb4fc87852bafef0a38` |
| `docs/research/backend-eligibility-convergence.md` | `b29f58feb616b47770d05820f9deb039f9a2cbf43bd68e285a384b276c4f9c53` |
| `crates/overdrive-sim/tests/e09_v2_failed_service_reachability_spike.rs` | `0219708f5772768981bd51b758409c7972c5a0b54c872ec53eaa7015a73e2732` |

The review covers the reproduced E09 v2 same-allocation-ID path, ADR-0101's
exact State/View and private-signature contract, the current production
callers and owners, ADR-0096 and its independent approved review, accepted
ADRs 0079, 0097, 0099, and 0100, the feature delta and architecture brief, the
research comparison, and the unchanged seeded witness. No implementation,
test, expectation, DES, roadmap, or native-host file was changed or executed;
no commit was created.

I read `CLAUDE.md`, all seven mandatory `.claude/rules` files, the installed
`nw-solution-architect-reviewer` role, and its required
`nw-sar-critique-dimensions` skill. No roadmap was in this review boundary, so
the roadmap-specific checks are not applicable.

## Revalidated production evidence

The claimed defect is real and reachable. The retained seed `257209` drives
the registered WorkloadLifecycle, ServiceLifecycle, and
BackendDiscoveryBridge through `run_convergence_tick` and the production
action-shim dispatch path without inserting an allocation, probe, backend, or
terminal row. Its source-level and seeded path is:

1. ServiceLifecycle observes startup exhaustion, records the allocation in
   `terminal_announced`, and constructs `healthy: false` before
   `FinalizeFailed` (`crates/overdrive-reconcilers/src/service_lifecycle.rs:657–703`).
2. The action shim serially drains the vector, while continuing after an
   individual action error (`crates/overdrive-control-plane/src/action_shim/mod.rs:905–959`).
   Terminal cleanup releases the prior supervision/network effects through the
   existing path (`:1759–1795`).
3. WorkloadLifecycle constructs `RestartAllocation` with the existing
   `row.alloc_id` (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:1426–1465`).
   The shim awaits the prior stop/absence and existing restart cleanup before
   provisioning the replacement (`crates/overdrive-control-plane/src/action_shim/mod.rs:2244–2302`).
4. The bridge removes membership for the Failed row and later recreates the
   Running member. It carries health for an observed member but defaults a
   newly absent member to `true` (`crates/overdrive-reconcilers/src/backend_discovery_bridge.rs:372–455`).
5. ServiceLifecycle still computes the same terminal veto, but current
   hydration retains only the prior row timestamp and the equal
   `last_emitted_backend_fingerprint` suppresses the repair
   (`crates/overdrive-reconcilers/src/service_lifecycle.rs:936–974,
   :1114–1173`). The unchanged spike's final assertion fails after six normal
   ServiceLifecycle ticks.
6. The resolver consumes the stored `healthy` bit rather than reconstructing
   ServiceLifecycle's veto (`crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:491–545`).
   The diagnosis separately records native E09 v2 traffic reaching the
   replacement VM path; that evidence is not rerun in this review.

This establishes necessity for an observed-state correction. It does not by
itself establish that the proposed two-writer architecture is the correct
long-term authority model.

## Contract and API assessment

### ADR-0101 D1–D2 — accepted in substance

The exact surface is clear and does not invent public API. Replacing
`ServiceLifecycleState::prior_backend_row_at` with
`Option<ServiceBackendRow>` and deleting
`ServiceLifecycleView::last_emitted_backend_fingerprint` are explicitly named
public State/View changes. The existing private signatures remain unchanged,
the State remains transient, and the existing keyed
`ObservationStore::service_backends_rows` read is retained rather than
duplicated. The design correctly forbids a new method, type, trait, variant,
action, port, configuration, dependency, wire field, stored-row schema, or
acknowledgement cache.

The legacy CBOR obligation is feasible: the runtime uses `ciborium` through
the existing `ViewStoreExt`, and the current View has no
`deny_unknown_fields`. A legacy blob containing the removed marker can be
decoded while the marker is discarded. This remains an implementation test
obligation, not a reason to add a migration.

### ADR-0101 D3 — accepted as the bounded correction

The health-only overlay is precisely stated:

- compare only matching observed members using
  `Backend.alloc == ServiceAllocFact.backend_spiffe`;
- change only those members' `healthy` bits and the existing dominating LWW
  stamp;
- preserve `service_id`, VIP, member count/order, SPIFFE identities, address,
  weight, and unmatched members byte-for-byte;
- do not synthesize a member, remove a member, or reconstruct bridge-owned
  structure; and
- treat an absent row, empty row, or row with no matching Running member as a
  no-op for ServiceLifecycle, leaving membership creation/removal to the
  bridge.

The current terminal veto remains the deciding startup failure or persisted
`terminal_announced` membership. A same-ID replacement does not clear it; a
genuinely distinct allocation still follows the existing predicate. The
action is placed before `FinalizeFailed` when a deciding failure needs a
correction, while the existing shim error-isolation behavior is preserved.

### ADR-0101 D4 — guarantee is correctly limited, but not a final authority ruling

The proposal is honest about what its mechanism can establish. If a
ServiceLifecycle evaluation hydrates a row containing a matching Running
member whose observed health differs from the unchanged veto, it emits a
full-row health overlay. If that write wins, a subsequent bridge evaluation
carries the false bit, and later hydration equality suppresses further writes.
The existing self-re-enqueue, AllocStatus interests, and periodic relist remain
the wakeup paths.

This is eventual convergence only. It does not guarantee:

- atomic exclusion between the bridge's membership write and
  ServiceLifecycle's health write;
- that no request observes the bridge's initial `healthy: true` default before
  the corrective evaluation and an accepted write;
- a wall-clock repair bound;
- durable withdrawal before terminal publication when the health write fails;
  or
- that two stale full-row writers cannot temporarily overwrite one another.

The native negative-peer oracle is correctly retained unchanged: the design
does not weaken the failed-frontend refusal, add a favorable final-writer
assertion, or turn the Rust test into a black-box expectation. The proposal's
explicit limits are technically truthful, but they are material to whether
this is a bounded mitigation or the correct architecture for the shared row.

## Findings

| ID | Severity | Status | Disposition |
|---|---|---|---|
| R0101-1 | **High** | Open; blocking for architectural approval | The bounded correction is not a comparative authority/guarantee ruling. |
| R0101-2 | Medium | Open; documentation correction | Linked SSOT tables contradict the proposal's ServiceLifecycle extension. |

### R0101-1 — Shared-row authority and guarantee comparison is unresolved

**Reachability and evidence.** This is not a hypothetical race. The current
production path has two reconcilers writing the same `ServiceBackendRow` key:
the bridge emits a full row at
`crates/overdrive-reconcilers/src/backend_discovery_bridge.rs:442–455`, and
ServiceLifecycle emits a full row at
`crates/overdrive-reconcilers/src/service_lifecycle.rs:1157–1172`. The row is
keyed by `ServiceId` alone and the ObservationStore contract is full-row LWW
(`crates/overdrive-core/src/traits/observation_store.rs:672–698,
:2017–2029`). Seed `257209` proves the reachable consequence: membership
disappearance/reappearance changes the stored row while the policy owner’s
desired veto is unchanged, and the old emit marker suppresses repair. The
native E09 v2 capture independently demonstrates that the wrong stored health
can reach replacement-VM traffic. ADR-0079 D9 already records this two-writer
condition as a state-layer ownership violation and identifies sole ownership as
a leading candidate, while explicitly saying its carry-through implementation
does not resolve that architecture (`adr-0079-backend-discovery-bridge-converges-on-the-rows-it-manages.md:1354–1468`).

**Finding.** ADR-0101 selects health-only observed-row convergence and rejects
“single backend-policy owner or persisted eligibility protocol” as out of
scope (`ADR-0101:175–182`). That is sufficient to describe a bounded repair of
the seeded stale-comparison defect, but it does not compare the actual owner
and guarantee options after acknowledging that two full-row writers remain.
Its own D4 expressly disclaims atomic exclusion and a repair bound
(`ADR-0101:110–131`). Consequently the document cannot be treated as the
correct long-term authority design merely because the seed can become green.
The user's reopened architecture comparison makes this an approval-blocking
ADR-quality/priority gap, not a request to invent a new mechanism in this
review.

**Required bounded disposition.** Record a separate, independently reviewed
authority/guarantee comparison before implementation approval, or explicitly
reclassify ADR-0101 as a bounded mitigation whose implementation is not
authorization to accept the shared-row architecture. The comparison must
state which owner is authoritative for each row field and distinguish eventual
repair, atomic exclusion, failure-path ordering, and any latency promise. This
finding does not select an owner, add an API, mandate persistence, or authorize
an owner migration.

### R0101-2 — ServiceLifecycle is simultaneously REUSE and EXTEND in linked SSOT

The proposal changes ServiceLifecycle hydration and reconciliation: ADR-0101
D1 replaces the State field and removes the View marker, while D2–D3 change
the observed-row comparison (`ADR-0101:35–101`). The feature-delta component
decomposition correctly labels “ServiceLifecycle and backend projection”
`EXTEND` (`feature-delta.md:461–479`), but its Reuse Analysis later labels
`ServiceLifecycle` `REUSE` (`feature-delta.md:542–560`). The architecture brief
also labels `ServiceLifecycle` `REUSE` while its immediately preceding
proposal text specifies the ADR-0101 hydration and health-only observed diff
(`brief.md:10512–10537`).

This is a documentation/scope contradiction rather than a production defect:
the exact ADR and the feature-delta decomposition identify the intended
affected file, and “reuse the existing owner” could explain the shorthand.
Before any implementation handoff, the linked tables need one consistent
classification (or an explicit distinction between ownership reuse and
implementation extension), with the same Contract Shape and bounded universe.

## Architecture quality dimensions

### Dimension 1 — technology and resume bias

**Pass.** The proposal adds no technology, dependency, service, or adapter.
Kubernetes and Consul are used as bounded, explicitly qualified precedents;
the research rejects transplanting their owner topologies. Existing Rust,
CBOR, keyed observation read, full-row LWW, and action-shim paths are selected
from the demonstrated production boundary rather than technology preference.

### Dimension 2 — ADR quality

**R0101-1 is the high finding.** Context, exact API, alternatives, ownership
matrix, consequences, and evidence lanes are otherwise unusually precise.
The missing element is a comparative decision about whether the acknowledged
two-writer row is an acceptable bounded mitigation or the correct authority
architecture. ADR-0101's explicit limit prevents a false stronger claim but
does not resolve that decision.

### Dimension 3 — completeness and quality attributes

**Pass for the bounded proposal; not a final architecture claim.**

- Reliability: the seed's persistent stale health is repaired after an
  observed member and successful write; write failures and propagation limits
  are not disguised as stronger guarantees.
- Security/data-plane honesty: the existing stored health filter and native
  negative-peer refusal remain the proof boundary; no active traffic-port
  health substitute is introduced.
- Performance/scalability: the existing keyed read is reused, no broad scan or
  new wake path is added, and the in-memory pass is bounded to one observed
  Service row and its members.
- Maintainability/testability: State/View changes, private signatures,
  Contract Shape declarations, seeded composition, in-process hydration and
  pure transition evidence are named.
- Observability: the existing operator-visible resolver/native oracle remains
  the relevant outcome surface; no new metric is needed to make this bounded
  behavior testable.

The two-writer authority trade-off is covered by R0101-1, not silently counted
as resolved by the quality-attribute summary.

### Dimension 4 — implementation feasibility

**Pass for the bounded contract.** The public shape and private signatures are
exact, the full-row overlay can be implemented at the existing health action
boundary, and no new port or asynchronous interface is required. Tests can
exercise the pure transition and production-composed seed without spawning the
product binary. The native run remains a separate Tier-3 obligation.

### Dimension 5 — priority validation

| Question | Assessment | Evidence |
|---|---|---|
| Q1: largest bottleneck | **YES** | Seed 257209 and native E09 v2 establish persistent false-to-true eligibility after same-ID membership reappearance. |
| Q2: simpler alternatives | **ADEQUATE for bounded repair; incomplete for authority choice** | Marker retention, whole-row replacement, and persisted/owner alternatives are rejected; the authority/guarantee comparison remains R0101-1. |
| Q3: constraint prioritization | **UNRESOLVED** | The proposal prioritizes no new owner/storage and honestly preserves an intermediate window, but the user has reopened whether that constraint is appropriate for the correct architecture. |
| Q4: data justified | **JUSTIFIED for necessity; insufficient for final authority selection** | Seeded failure, source path, native capture, and keyed-row/LWW facts support the bounded repair; they do not measure or establish the stronger authority guarantee. |

## Verification and boundary assessment

The verification obligations are correctly layered for the bounded proposal:

1. Keep seed `257209` as a production-owner Sim invariant, including the
   no-readiness healthy control, and require the final assertion to become
   false/healthy-ineligible without injecting rows or weakening the oracle.
2. Add pure transition evidence for equality/no-write, changed health with
   unchanged policy, exact structural preservation, absent/empty/no-member
   no-op, terminal-veto/no-readiness behavior, deciding-failure ordering, late
   pass, and distinct-allocation behavior.
3. Add in-process hydration and legacy CBOR decode evidence. Rust tests must
   not run the product binary or emit black-box expectation evidence.
4. Retain independent built-default-feature native E09 v2 evidence: one
   persistent control-plane process, failed frontend refusal, healthy paired
   serving, and the existing cleanup oracle. No fixed sleep, favorable final
   writer, or internal-state assertion belongs in that expectation.

These obligations prove the bounded convergence effect and preserve the
native negative-peer oracle. They do not prove atomic exclusion, a zero-length
eligibility window, a durable write-before-terminal barrier on error, or a
new architecture; the proposal correctly must not claim those properties.

## Iteration record

| Iteration | Verdict | Findings | Remediation disposition |
|---:|---|---|---|
| 1 | **CHANGES_REQUESTED** | R0101-1 High open; R0101-2 Medium open | No remediation was performed by this independent reviewer. A later architectural comparison or document revision requires a fresh review and must not be attributed to this iteration. |

## Verdict

**CHANGES_REQUESTED.** The pinned ADR-0101 revision is a necessary and
technically feasible bounded repair for the proven seed: its exact health-only
API, same-ID semantics, ownership-preserving row overlay, convergence limits,
and unchanged native oracle are clear. It is not approval of the shared
full-row two-writer architecture or of an intermediate eligibility window as
the desired long-term guarantee. Because the user has reopened the authority
and guarantee comparison, that decision must be recorded and independently
reviewed before this proposal can receive architectural approval or authorize
implementation. The linked REUSE/EXTEND contradiction must also be
synchronized.

---

## Iteration 2 — Revision 3 independent design review

### Review metadata

| Field | Value |
|---|---|
| Reviewer | `nw-solution-architect-reviewer` (selected GPT5.6 Luna, maximum thinking) |
| Review date | 2026-09-08 |
| Reviewed design revision | Revision 3, the ServiceLifecycle sole-publisher proposal |
| Exact ruling reviewed | `docs/feature/service-kind-vm-workloads/design/backend-eligibility-convergence-ruling.md` — SHA-256 `2ff59cbd09e3ad9680c5ec9994f0bff587bb1df60a8a8a21ce07f2f39d39efc2` |
| Exact ADR reviewed | `docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md` — SHA-256 `f4809e013f12eae8b43aea9b2bd133ec0819018f0e428e196136b27db0b61f24` |
| Linked material | ADR-0096 SHA-256 `f620492be20861becdb23f604560a1c96cbb66bced261b64fc99fecb7c917a97`; brief SHA-256 `3c8ffa90b8c82fb4b1b5a910124ec90f9c301c66989351211bf4985b5135521e`; feature-delta SHA-256 `aa4dd54809adf6e9f81e8622dea565eab185d967cf679889e0300775b7658e12`; research SHA-256 `b29f58feb616b47770d05820f9deb039f9a2cbf43bd68e285a384b276c4f9c53` |
| Premise evidence | Diagnosis SHA-256 `3e2b055511de5d44bd018160c202e1ad4a210a32938419dfb5674649e2a4ba49`; retained seed-257209 spike SHA-256 `0219708f5772768981bd51b758409c7972c5a0b54c872ec53eaa7015a73e2732` |
| Prior review | Iteration 1 in this artifact, ruling SHA-256 `719be475...` and ADR-0101 SHA-256 `82512cf...` |
| Review boundary | Architecture, exact public/private contract, ownership, lifecycle ordering, and stated guarantees only |
| Actions taken | No design, production, test, harness, native run, commit, DES event, or additional-agent action |

This iteration reviews revision 3 itself. It does not treat the iteration-1
bounded two-writer proposal as the target, does not authorize implementation,
and does not decide a future architecture beyond the revision under review.
The user-selected decisions are taken as the review contract: one
authoritative publisher colocated with the existing ServiceLifecycle health
authority; publication safety under the existing terminal veto, including a
same-ID restart; asynchronous routing-consumer convergence; greenfield
operation without compatibility, migration, rollout, or dual-publisher
machinery; and preservation of normal View persistence, failure, and restart
behavior.

### Premise and production reachability

The original defect remains a proven production-owner-path failure, rather
than a hypothetical ordering concern. The retained seed `257209` registers
the production `WorkloadLifecycle`, `ServiceLifecycle`,
`BackendDiscoveryBridge`, and `VmReclamation` reconcilers, drives the
production `run_convergence_tick`, and uses the production probe/action
composition with `ProbeRunner`, `SimObservationStore`, the clock, and the
driver registry. It does not seed a backend row, terminal row, allocation, or
deduplication marker.

The reachable ordering is concrete:

1. Three failed startup probes leave the allocation Running without readiness;
   `ServiceLifecycle` constructs the existing full-row false publication and
   `FinalizeFailed` (`service_lifecycle.rs:657–703`).
2. The bridge observes the terminal membership and publishes the empty row
   (`backend_discovery_bridge.rs:372–455`).
3. The existing `WorkloadLifecycle` restart path reuses the same allocation
   identity (`workload_lifecycle.rs:1426–1465`; action-shim restart path
   `action_shim/mod.rs:2244–2302`), after which bridge membership reappears
   Running and healthy (`backend_discovery_bridge.rs:372–455`).
4. ServiceLifecycle hydration discards the observed row body and its marker
   prevents the correction (`service_lifecycle.rs:936–974, 1114–1173`). Six
   subsequent ServiceLifecycle ticks leave the row eligible despite the
   retained terminal veto.

No forced future abort, dropped write, reclamation race, or test-only row is
needed. The diagnosis and spike therefore satisfy the repository requirement
for a seeded convergence failure through the current production owner path.
The native E09 v2 capture supplies the independent real-product negative-peer
and cleanup evidence; it is not being used to claim atomic routing exclusion.

### Revision-3 architecture assessment

#### Authority and publication guarantee

Revision 3 makes the necessary authority choice explicit. `ServiceLifecycle`
is the only reconciler that constructs the complete `ServiceBackendRow` for
the current listener set. The action shim remains the existing effectful
ObservationStore writer, so “publisher” is correctly distinguished from the
store adapter. `BackendDiscoveryBridge` is retired, including its registration,
dispatch variants, constructors, helpers, and active writer path. This removes
the proven full-row last-writer-wins conflict instead of adding a field-level
merge rule.

The guarantee is stated at the right boundary. From the deciding tick on
which the existing terminal veto applies, every subsequent row published by
this owner for a still-member allocation is ineligible. A same-ID
disappearance/reappearance does not clear the retained terminal input, and
Running is not equivalent to healthy. The existing readiness predicate and
allocation-scoped counter semantics remain the policy authority; observed
backend health is not treated as an input that can override policy.

The design also correctly limits the guarantee. It promises stored-row
convergence when the existing reads, writes, and owner execution make
progress, not atomic exclusion from already-running routing consumers, a
zero-delay window, a write-before-terminal barrier on the existing
continue-on-error path, or HA fencing. Startup failure does not wait for
routing consumers, and existing consumer wakeup, retry, relist, and fault
behavior remains in force. These are user-selected scope limits, not missing
mechanisms.

#### Exact contract and state ownership

The ADR pins an implementable, non-invented contract:

- `ServiceLifecycleState` retains `pub allocs`, replaces the singular
  dataplane identity with `pub service_dataplane:
  BTreeMap<ServiceId, ServiceDataplaneIdentity>`, and replaces the timestamp
  marker with `pub observed_backend_rows:
  BTreeMap<ServiceId, ServiceBackendRow>`.
- `ServiceDataplaneIdentity` contains exactly `vip`, `port`, `protocol`, and
  `writer`; the redundant `service_id` field is removed. `ServiceAllocFact`
  changes only the backend address representation needed to combine an
  allocation IPv4 address with each listener port.
- The View loses only `last_emitted_backend_fingerprint`; terminal lifecycle
  inputs and `has_alloc_mid_startup_window` remain persisted as before. The
  observed rows are transient readback, not a new persistence schema.
- The private hydration and action shapes are pinned, including
  `service_dataplane_identities`, `hydrate_service_alloc_facts` without
  `backend_port`, and `service_backend_row_actions` returning the existing
  `Vec<Action>` row/hydrator groups. Existing public constructors, `new`,
  `Default`, `Reconciler` signatures, interests, action variants, store
  schema, and consumer APIs remain unchanged.

This is a precise API replacement and retirement, not a request for a new
port, generation, acknowledgement, persistence subsystem, broker protocol,
or owner migration. The ruling and ADR agree that all current listener
identities come from intent plus allocator facts, every listener is retained,
and each Running allocation contributes its actual IPv4 address. Missing
rows are observed as missing rather than healthy; missing hydration is a
typed error rather than an empty default.

#### Actual-state comparison and action ordering

The proposed `actual` contains the complete keyed observed row for every
current listener. The desired projection is calculated for all listeners,
including the real empty-membership projection; it does not return early just
because no allocation is currently Running. Comparison covers service ID,
VIP, and the entire deterministic backend vector while excluding only
`updated_at`. Equality is the sole no-write case, so the old emitted marker is
not an acknowledgement substitute. Changed listeners use the existing
`LogicalTimestamp::dominating` and existing `WriteServiceBackendRow` action,
followed immediately by the existing `EnqueueEvaluation` handoff to
`service-map-hydrator`.

The action group is placed before the existing startup `FinalizeFailed` action
when the deciding tick fails, and after startup/Stable otherwise, while
liveness collection remains in its existing order. Runtime persistence of
the next View precedes serial awaited dispatch (`reconciler_runtime.rs:
1617–1779`). The shim's existing per-action error isolation is preserved: a
row-write failure is surfaced and the batch continues, with no invented
durable ordering claim. This is internally consistent with the stated stored
convergence guarantee.

#### Wakeup, hydration, and consumer handoff

The revision preserves the production owner path needed to make the single
publisher observe changes: `WorkloadLifecycle` directly nudges
ServiceLifecycle for all existing allocation-mutating actions (Start,
Restart, Stop, and FinalizeFailed); ServiceLifecycle retains its self-enqueue;
the existing AllocStatus interest route and boot LIST/periodic relist remain;
and reclamation removes only the bridge submission while retaining the
ServiceLifecycle nudge and resource cleanup. The design does not create a
second health publisher to compensate for a missed wakeup.

The existing `ServiceMapHydrator` remains a read-side consumer. It resolves
listener facts, reads the row, applies the materialized healthy bit, and uses
its existing retry and supported local/remote paths. Mesh and DNS retain their
existing independent list/watch/relist behavior. The explicit row-write then
hydrator-enqueue pair, including health-only changes, gives the existing
consumer handoff without waiting for consumer processing. No consumer
predicate, acknowledgement, connection revocation, or new ServiceLifecycle
resync loop is introduced.

#### Alternatives, quality, and scope

The ruling and ADR compare the former health-only shared-row overlay, an
independent-facts join, and consumer-side joins. They explain why the first
does not satisfy strict publication safety and why the latter two add policy
or freshness authorities not required by this production path. The selected
sole publisher is therefore a user-approved authority decision grounded in
the retained defect, not an unqualified claim that the research mandates one
architecture. External research is used only as qualified support for
observed-state comparison and explicit ownership; no external mechanism is
imported.

Reliability is addressed by retained terminal state, same-ID semantics,
complete readback, equality idempotence, serial ordering, and existing error
handling. Maintainability is improved by deleting the competing writer and
its public dispatch surface. Performance remains bounded by the inherent
allocation-by-listener projection and reads only changed rows. Security and
identity remain unchanged: existing SVID identity, listener protocol, IPv4
restriction, and local/remote consumer boundaries are retained. Observability
remains the existing action-shim error and reconciliation path; no new metric
or acknowledgement is implied.

No rollback, compatibility, migration, old-View conversion, recovery,
scheduling, attempt fencing, or owner migration requirement is accepted. No
unproven timing or cancellation scenario is promoted to a defect. Detailed
acceptance scenarios and executable tests are deliberately handed to the
acceptance designer after DESIGN approval; this review neither authors tests
nor mandates a test architecture. The retained seed, native negative-peer
oracle, and implementation evidence are sufficient premise boundaries for
this architecture review.

### Finding dispositions

#### R0101-1 (Iteration 1): shared-row authority and guarantee comparison

**Disposition: Resolved by revision 3.** The prior finding was High because
revision 1 kept ServiceLifecycle and BackendDiscoveryBridge as competing
full-row writers and explicitly left authority selection unresolved. Revision
3 now selects ServiceLifecycle as the sole complete-row publisher, gives the
guarantee a precise terminal-veto boundary, compares the rejected authority
alternatives, and specifies direct bridge retirement. This resolution is
recorded as an assessment of the revision-3 proposal only; it is not approval
of the former two-writer design or authorization to implement it.

#### R0101-2 (Iteration 1): ServiceLifecycle REUSE versus EXTEND contradiction

**Disposition: Resolved in the reviewed current design set.** The current
feature-delta component table and brief identify ServiceLifecycle as
`EXTEND` with ownership reused, and identify BackendDiscoveryBridge as
`RETIRE`. The lifecycle and API sections consistently describe the
ServiceLifecycle extension and direct bridge removal. The old iteration-1
finding is not carried forward.

#### R0101-3: current feature-delta driving-port row retains the retired bridge

**Severity: High — blocking. Status: Open.**

The current `Wave: DESIGN / [REF] Driven Ports and Adapters` table still says
at `docs/feature/service-kind-vm-workloads/feature-delta.md:505` that the
“Backend-health withdrawal” effect is handled by ServiceLifecycle and that
“the bridge continues to carry observed health.” This sentence is not marked
historical. It directly contradicts the same feature-delta's current
component table (`:475–476`), lifecycle ruling (`:568–585`), and both
revision-3 design artifacts, which require ServiceLifecycle to be the sole
publisher and require BackendDiscoveryBridge's writer, registration, and
dispatch surface to be deleted.

This is a reachable implementation risk, not a stylistic disagreement:
`backend_discovery_bridge.rs:372–455` is the existing production full-row
writer, and a crafter following the current driving-port row could retain it
alongside the new ServiceLifecycle writer. That would recreate the exact
last-writer-wins ordering that seed `257209` proves can restore healthy
eligibility after a same-ID restart, defeating the selected publication-safety
guarantee. The conflict also violates the exact retirement API contract.

**Required disposition:** synchronize that one current feature-delta contract
statement with revision 3—state that ServiceLifecycle alone constructs the
complete observed-row projection and that the bridge is retired—or explicitly
mark the old wording superseded. This is documentation synchronization only;
it does not authorize a new mechanism, production change, test change, or
broader architecture. The historical bridge descriptions in ADR-0096 and
older brief material need no new architecture finding where their surrounding
text already identifies them as superseded historical topology.

### Verification and handoff boundary

The exact ADR/ruling provide enough contract for an independent implementation
review after this documentation contradiction is repaired: public/private
shapes, full-row ownership, all-listener identity, actual-state comparison,
action grouping, wakeup union, consumer handoff, policy authority, same-ID
behavior, and explicit non-guarantees are all pinned. The acceptance designer
owns the detailed scenarios and executable evidence after DESIGN approval.
That later work must retain seed `257209` and the native negative-peer oracle,
must not use a favorable final writer, and must not turn asynchronous consumer
convergence into an atomic-exclusion claim. Those are handoff boundaries, not
additional findings in this architecture review.

### Iteration record update

| Iteration | Reviewed revision | Verdict | Findings | Disposition |
|---:|---|---|---|---|
| 1 | Revision 1 bounded shared-row overlay | **CHANGES_REQUESTED** | R0101-1 High open; R0101-2 Medium open | Superseded by the revision-3 review; history retained above |
| 2 | Revision 3 ServiceLifecycle sole publisher | **CHANGES_REQUESTED** | R0101-3 High open | Synchronize the nonhistorical feature-delta driving-port contract, then obtain independent re-review |

## Current verdict

**CHANGES_REQUESTED.** Revision 3 itself resolves the prior architectural
authority gap: it selects one ServiceLifecycle publisher, pins exact state and
private API changes, preserves existing ownership and wakeups, compares
complete observed rows for every listener, retires the independent bridge
writer, and states stored convergence without claiming atomic routing
exclusion. It also correctly preserves the existing terminal veto through a
same-ID restart and leaves startup failure independent of routing-consumer
completion.

Approval is blocked by R0101-3 only: a current linked feature-delta contract
still directs the bridge to carry observed health. Until that contradictory
instruction is synchronized, the exact owner and retirement contract is not
single-source-of-truth enough to authorize implementation. This bounded
finding does not reopen the architecture comparison, impose compatibility or
rollback work, or mandate any test mechanism.

---

## Iteration 3 — Focused re-review of R0101-3

### Review metadata

| Field | Value |
|---|---|
| Reviewer | `nw-solution-architect-reviewer` (selected GPT5.6 Luna, maximum thinking) |
| Review date | 2026-09-08 |
| Review scope | R0101-3 only; no re-opening of the already resolved revision-3 findings |
| Corrected linked artifact | `docs/feature/service-kind-vm-workloads/feature-delta.md` — SHA-256 `f2778710b7bd8b3c53735220d20aaed55431a297fe6ccd20a8bd561c3d3e1feb` |
| Candidate ruling | Revision-3 ruling SHA-256 `2ff59cbd09e3ad9680c5ec9994f0bff587bb1df60a8a8a21ce07f2f39d39efc2` |
| Candidate ADR | Revision-3 ADR-0101 SHA-256 `f4809e013f12eae8b43aea9b2bd133ec0819018f0e428e196136b27db0b61f24` |
| Prior review artifact before this append | SHA-256 `e2e79c86f2f2c94e0d30db300bf230a5b995b2d5c791e66f027fa35b656ba1da` |
| Actions taken | Documentation read-only verification; no design, production, test, harness, native run, commit, DES event, or additional-agent action |

### R0101-3 verification

The corrected `feature-delta.md:505` now states the exact revision-3
contract: ServiceLifecycle alone constructs the complete backend projection,
compares it with observed rows, and BackendDiscoveryBridge is retired. It
also states the already pinned ordering and scope limits: changed rows use
the deciding allocation's `healthy: false` before the startup-terminal
action, serial dispatch may continue after a row-write error, and routing
consumers converge asynchronously.

That wording now agrees with the exact candidate documents:

- ADR-0101 D1/D4 make ServiceLifecycle the sole complete-row publisher,
  require all-listener actual-state comparison, and place changed-row
  publication before the deciding `FinalizeFailed` action.
- ADR-0101 D5 retires the bridge module, writer, registration, constructors,
  dispatch variants, and live call sites; no second publisher remains in the
  selected composition.
- ADR-0101 D6 and the ruling preserve the action-shim error-isolation path and
  explicitly accept asynchronous consumer convergence without an
  acknowledgement barrier or atomic routing exclusion.

The formerly reachable contradiction is therefore removed. A crafter reading
the current feature-delta driving-port table is no longer directed to retain
the production bridge writer at `backend_discovery_bridge.rs:372–455`; the
table now directs the same retirement required by the exact API contract. The
seed-257209 reachability evidence remains the correct reason this ownership
must be explicit, but no new reproduction or test is needed for this focused
documentation disposition.

No remaining concrete contradiction is found within R0101-3. This focused
review does not assess or expand any other design, test, migration,
compatibility, rollback, recovery, scheduling, fencing, or consumer-boundary
requirement. The acceptance designer may now take the detailed scenarios and
executable-test handoff defined by revision 3.

### R0101-3 disposition

**Disposition: Resolved.** The current linked contract no longer preserves an
independent observed-health publisher and is consistent with the selected
ServiceLifecycle authority, exact bridge retirement, terminal-veto ordering,
serial error semantics, and asynchronous consumer handoff. No remediation
finding remains in this focused scope.

### Iteration record update

| Iteration | Reviewed revision | Verdict | Findings | Disposition |
|---:|---|---|---|---|
| 1 | Revision 1 bounded shared-row overlay | **CHANGES_REQUESTED** | R0101-1 High open; R0101-2 Medium open | Superseded by later independent review |
| 2 | Revision 3 ServiceLifecycle sole publisher | **CHANGES_REQUESTED** | R0101-3 High open | Feature-delta contract correction requested |
| 3 | Revision 3 plus corrected feature-delta contract | **APPROVED** | None remaining in focused scope | R0101-3 resolved; no further design remediation required |

## Current verdict after focused re-review

**APPROVED.** R0101-3 is resolved by the single feature-delta correction.
The reviewed revision-3 candidate now has a consistent sole-publisher and
bridge-retirement instruction across the exact ADR, ruling, and linked
feature-delta contract. The previously blocking ownership contradiction is
gone. This approval is for the bounded revision-3 DESIGN review and permits
the separately owned acceptance-design handoff; it does not claim atomic
routing exclusion, add a new mechanism, or record implementation completion.
