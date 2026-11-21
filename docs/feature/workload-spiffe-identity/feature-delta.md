# Feature delta — workload-spiffe-identity

Fixes the public ingress gateway 502 after a `serve` restart at its root:
a workload's SPIFFE identity named the allocation, so a replacement allocation was a
different party to every identity consumer. Evidence:
[RCA](../../analysis/root-cause-analysis-public-ingress-gateway-502-after-serve-restart.md)
(incl. "Regression bisect": first bad `4ebc41233`, the boot resync pass exposed it),
[research](../../research/dataplane/backend-identity-on-endpoint-reuse-comprehensive-research.md).
ADRs: [0344](../../product/architecture/adr-0344-a-workload-svid-names-the-workload-not-the-allocation.md),
[0345](../../product/architecture/adr-0345-svid-audit-rows-and-restart-recovery-key-on-the-allocation-not-the-identity.md),
[0346](../../product/architecture/adr-0346-gateway-client-mtls-requires-the-selected-backends-workload-identity.md),
[0347](../../product/architecture/adr-0347-dataplane-backend-identity-association-carries-the-workload-identity.md).
Whitepaper is outdated; ADRs and code are the SSOT.

## Wave: DESIGN / [REF] Decisions

| ID | Decision | Status |
|---|---|---|
| D1 | A workload's SPIFFE ID is `spiffe://overdrive.local/workload/<w>`, shared by all its allocations; each allocation keeps its own SVID (key, serial, rotation, `AllocationId` key). ADR-0344. | Approved by the user 2026-11-21 |
| D2 | Per-allocation distinction is an explicit `AllocationId`, not encoded in identity: `issued_certificates` records it (rkyv version bump), `ever_issued` and the `alloc status` certificate listing key on it. ADR-0345. | Approved 2026-11-21 |
| D3 | The gateway requires the peer to present the selected backend's WORKLOAD identity. ADR-0346 (supersedes ADR-0234). | Approved 2026-11-21 |
| D4 | The dataplane association carries the workload identity; a same-workload replacement at a memoised endpoint reuses the `BackendId`; `BackendIdentityConflict` removed. ADR-0347 (supersedes ADR-0241). | Approved by the user 2026-11-21 |
| D5 | Supersession, not amendment; old ADR text retained. | Approved 2026-11-21 |
| DD-1 | One parse SSOT in core: `SpiffeId::for_workload` builds, `SpiffeId::workload_id` inverts. The three private parsers (`workload_of_allocation_identity`, `name_index::workload_of`, the `mtls_resolve_adapter` call) route through it. | Detail inside D1 |
| DD-2 | The audit row records `issued_for`, a sum type `{Allocation(AllocationId), Gateway}`, not an optional `alloc_id`: gateway SVIDs share the `issued_certificates` table and have no allocation (`rust.md` "Sum types over sentinels"). | Detail inside D2 |
| DD-3 | `SvidLifecycleState.ever_issued` becomes a set of `AllocationId`. The held-set filter becomes "identity equals `for_workload(target)`", attribution stays by the `AllocationId` that keys the held set. | Detail inside D2 |
| DD-4 | The gateway peer-check code is unchanged; its meaning changes through the identity it is given. | Detail inside D3 |
| DD-5 | Greenfield single cut for every persisted shape this feature touches. `ServiceBackendRow` / `Backend` shape does not change (field type is still `SpiffeId`; the PA-1 rename is layout-free), so no envelope bump there. `IssuedCertificateRow` takes a version bump (V2). Rows written by a prior binary carry per-allocation identities and are discarded with the data directory; no reader translates a `/alloc/` identity (ADR-0344, ADR-0345). | Detail inside D2, D4 |
| DR-1 | Option (b): when a different workload is programmed at an endpoint the dataplane still memoises, the existing `BackendId` is re-associated with the new workload identity (the endpoint keeps its id; the id's identity follows the workload currently programmed there). The allocator memo stays keyed by endpoint; no new allocator API. A connection whose receipt was taken for the previous workload and is verified after the re-association is checked against the new workload's identity. ADR-0347. | Approved by the user 2026-11-21 |
| DR-2 | Flow telemetry stays workload-grain (`FlowEvent.src`/`dst` carry the workload identity; per-allocation attribution, if ever wanted, is an explicit `AllocationId` field per D2). No change. | Approved by the user 2026-11-21 (no change) |
| DR-3 | Issue #242 (intended-peer pinning) pins a workload identity after this feature; the allocation dimension disappears. The orchestrator comments on #242; this design creates no issue. | Noted 2026-11-21 |
| PA-1 | Rename `Backend.alloc` to `Backend.workload_identity` (pinned in the contract below). | Approved by the user 2026-11-21 |
| QM | Growth of the Quint model for DR-1(b) (`AssociationFollowsProgrammedWorkload`, hazard `staleAssoc`, rewritten takeover run). | Approved by the user 2026-11-21 |
| TC | Two companion tests (seeded `overdrive-sim` invariant; host-vs-Sim dataplane equivalence), pinned as DISTILL obligations in § Regression test. | Approved by the user 2026-11-21 |

## Wave: DESIGN / [REF] Changed Assumptions

| Superseded contract (quoted) | Source | Replaced by |
|---|---|---|
| "While reachable by a receipt, one BackendId never denotes a different `Backend.alloc`: an application that would associate a live id with another allocation is rejected before the outer-map swap and the prior applied generation is preserved." | ADR-0241 Decision | ADR-0347: an endpoint has one `BackendId` whose association is the workload identity currently programmed there; a different workload re-associates the id, nothing is rejected for identity. The association is no longer immutable. |
| "[...] the peer certificate chains to the trust bundle and its sole SPIFFE URI SAN equals the exact workload identity resolved from the receipt-selected BackendId" where the resolved identity was the identity of one allocation (leg-C "presents the SVID of the allocation that owns the dialed guest address") | ADR-0234 Context and Decision | ADR-0346: the expected identity is the selected backend's workload identity; every allocation of that workload passes, a different workload fails closed. |
| Workload SPIFFE ID `spiffe://overdrive.local/workload/<w>/alloc/<a>` | ADR-0067 D5, ADR-0075 | ADR-0344: `spiffe://overdrive.local/workload/<w>`. |

## Wave: DESIGN / [REF] Interface contracts

**`overdrive-core` — `SpiffeId` (id.rs).** Replace `for_allocation` (single cut, no shim):
- `for_workload(&WorkloadId) -> SpiffeId`: infallible, `#[must_use]`; yields `spiffe://overdrive.local/workload/<w>`. Same `unreachable!` idiom as today.
- `workload_id(&self) -> Option<WorkloadId>`: `Some` iff the path is exactly `/workload/<label>` with a valid `WorkloadId` label; `None` for the gateway identity, for any other shape, and for a path with extra segments. Post: `for_workload(w).workload_id() == Some(w)`.
- `for_gateway` unchanged. `for_allocation` and every `/alloc/` parse are removed; no parser of the old shape survives.

**`IssuedCertificateRow` (ca/issued_certificate_row.rs), ADR-0345.**
- New sum type `IssuedFor { Allocation(AllocationId), Gateway }` (core, `Ord`, serde/rkyv as the other id types).
- `IssuedCertificateRowV2` = V1's fields plus `issued_for: IssuedFor`; `IssuedCertificateRow` and `…Latest` re-alias to V2; `IssuedCertificateRowEnvelope` gains `V2` appended after `V1` (not reordered); `known_discriminants` becomes `[0, 1]`; `discriminant_offset_from_end` re-pinned. Follow the six-step version-bump procedure in `development.md` in one commit.
- `into_latest` for V1 returns `EnvelopeError`: greenfield single cut, a V1 row is never translated (observation reads log and skip it, ADR-0048 §3). The V1 golden fixture asserts that projection. If appending the larger V2 variant changes the archive root sizing so the V1 literal no longer decodes, regenerate it as an explicit `Envelope::V1(frozen V1 payload)` (never through `latest()` or the alias, which would encode V2), per the established rkyv envelope precedent.
- `ca_issuance::issue_and_audit` gains an `issued_for: IssuedFor` parameter and writes it onto the row. `Action::IssueSvid` already carries `alloc_id`; the action-shim executor passes `Allocation(alloc_id)`, the gateway executor `Gateway`. `IssueSvid.spiffe_id` is `for_workload(workload)`.

**`SvidLifecycle` (overdrive-reconcilers).** `ever_issued: BTreeSet<AllocationId>`, filled from rows whose `issued_for` is `Allocation`. `managed_targets` derives the workload from the held identity via `SpiffeId::workload_id`. `hydrate_actual` filters the held snapshot by identity equal to `for_workload(target)`; `HeldSvidView::held_snapshot` still returns the unfiltered global set keyed by `AllocationId` (its contract text drops the `for_allocation` filter wording).

**`alloc status` (handlers.rs `issued_certificates_for_rows`).** Lists the certificates whose `issued_for` is `Allocation(row.alloc_id)`. `AllocationSpec.identity` (and every `Backend.workload_identity`) holds the workload identity.

**`Backend` field rename (PA-1, approved).** In `overdrive-core` `traits/dataplane.rs`, `Backend.alloc: SpiffeId` becomes `Backend.workload_identity: SpiffeId`; type, position and order are unchanged. `ServiceBackendRowV1.backends: Vec<Backend>` carries the renamed field, so the persisted row's rkyv layout is unchanged (positional, names are not archived) and no envelope bump or golden-fixture change follows (DD-5). Single cut, no alias, no shim. The compiler enumerates every site (the old name no longer exists); DELIVER does not grep-and-replace `.alloc`, because `AllocationSpec.alloc: AllocationId` and other `alloc` fields share the token and are not renamed.

**`Dataplane::update_service` — trait rustdoc (overdrive-core traits/dataplane.rs), additions:**
- Pre: each `Backend.workload_identity` is the WORKLOAD identity of the backend.
- Post on `Ok`: `GatewayConnectDataplane::selected_backend_identity(id)` returns the workload identity most recently programmed at that endpoint, for every `BackendId` the update programmed.
- Edge: the same workload at an endpoint the dataplane already programmed reuses that endpoint's `BackendId`, changes no association and no fingerprint (replacement of an allocation is a no-op). A different workload at an endpoint the dataplane still memoises keeps that endpoint's `BackendId` and re-associates it with the new workload identity (ADR-0347, DR-1(b)); the re-association is published under the commit guard, and a pre-commit failure leaves the prior association. A duplicate endpoint within ONE call is `LoadFailed` regardless of identity (same workload twice or two workloads), host and Sim alike. Reverse-NAT ownership by another Service remains `ReverseNatConflict`, checked on the endpoint.
- Removed: `DataplaneError::BackendIdentityConflict` (no producer remains). No identity-related error variant exists; `SimDataplane` follows the same rule (equivalence).

**`GatewayConnectDataplane::selected_backend_identity(BackendId) -> Result<SpiffeId, GatewayConnectDataplaneError>` — rustdoc.** Returns the workload identity currently applied for that id. The value follows the workload programmed at the id's endpoint, so a read taken after a takeover returns the new workload even for a receipt taken before it (the connection is verified against the new workload, ADR-0347). Not readable while Reserved (`BackendIdentityMissing`). Waits on the commit guard, so it returns the value before or after an in-flight update, never a mixed or early one. Signature unchanged.

**Gateway check (`overdrive-dataplane` mtls/gateway_client.rs; `GatewayClientMtls::authenticate`).** `expected_peer` is the workload identity from `selected_backend_identity`. Behaviour unchanged: chain to bundle, sole SPIFFE URI SAN, equality; `PeerSpiffeMismatch`/`PeerSpiffeShape`/`PeerCertificateMissing` unchanged.

**`BackendIdAllocator` (DR-1(b)).** Unchanged: the memo stays keyed by endpoint and no allocator API is added. Re-association is the dataplane's association write for an existing id (under the commit guard in the service-apply path), not an allocator operation.

**Wire/persisted.** SPIFFE URI SAN of workload leaves changes to `/workload/<w>`. `IssuedCertificateRow` V2 as above. No other persisted shape changes (DD-5).

## Wave: DESIGN / [REF] Component decomposition

No new component, crate or port. Owners that change:

- `overdrive-core`: `id.rs` (constructor/inverse), `ca/issued_certificate_row.rs` (`IssuedFor`, V2, schema-evolution fixture file), `traits/dataplane.rs` (docs, error variant removed), `traits/held_svid_view.rs` (doc), `reconcilers/mod.rs` (`IssueSvid` docs).
- `overdrive-reconcilers`: `SvidLifecycle` (`ever_issued`, filter, `managed_targets`); `WorkloadLifecycle`/`ServiceLifecycle` identity construction.
- `overdrive-control-plane`: `ca_issuance`, the SVID action-shim executors, `alloc status` handler, DNS name index, `mtls_resolve_adapter`.
- `overdrive-dataplane`: the service-apply path (drop the identity-conflict preflight; a different workload at a memoised endpoint re-associates that endpoint's id), the Quint conformance driver; the `BackendIdAllocator` is unchanged.
- `overdrive-sim`: `SimDataplane`, read-port Sim impls, and the invariants that build per-allocation identities.

`for_allocation` call sites and hand-written `/alloc/` identity literals are mostly mechanical (`for_allocation(w, a)` becomes `for_workload(w)`); the compiler enumerates them. `spike-scratch/` occurrences stay as evidence.

**Non-mechanical (reviewers check these):**
1. Any test or invariant that used two allocations of one workload as the "wrong peer" or as two distinct backends uses two WORKLOADS (the gateway wrong-peer cases at every tier, the dataplane equivalence S-PIG-67, the conformance driver's identity fixtures).
2. Tests asserting identity inequality across a recreation now assert the `AllocationId` is fresh and the identity is equal.
3. Fingerprint and backend-key generators that made backends distinct only by `/alloc/<x>` distinguish them by address or workload.
4. SVID tests that tested identity-derived `ever_issued`, held-set attribution or the `alloc status` join re-key on `AllocationId`, and add the sibling-allocation case (allocation B does not inherit allocation A's `ever_issued`).
5. Constructor tests move to a `for_workload` + `workload_id` round trip (proptest).
6. Quint conformance driver (`quint_selection_receipt.rs`): obligations are behavioural. Remove the `update_rejected` handler and the two deleted run pins (`conflicting_alloc_rejected`, `identity_conflict_precedes_reverse_nat`); pin `#[quint_test]` for `replacement_reuses_backend_id`, `serve_restart_replacement` and `different_workload_takeover`; `#[quint_run]` stays. The action-to-driving-port table and the `State` projection below are unchanged by DR-1(b). The per-step comparison of `ident[id]` must accept `update_apply` changing an existing id's value (a re-association: workload 1 becomes workload 2 on the same `BackendId`). The driver never computes a `BackendId`: it keeps no counter or memo, takes each id from a receipt (`take_receipt`, `live_receipts()`) or observes it on the owner after `update_apply`, and compares `ident[id]` for exactly those ids. `g.prog` is model-only and not projected.
7. Docs that state the old shape (ADR-0072/0079/0083/0086/0101 prose, `c4-diagrams.md`) are historical and keep the supersession pointers in ADR-0344; DELIVER updates the live doc comments only.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| Selected Backend Identity Resolved | Service Dataplane | The workload identity applied for the receipt's `BackendId` | Applied association (commit-guarded, ADR-0233) | Gateway Applied Generation, BPF Hydrated, Backend Eligibility, Allocation Running, Service Stable |
| Peer is the selected workload | Public ingress gateway (client mTLS) | The peer presenting a chain-valid certificate is the workload the receipt selected | Resolved identity + presented SAN | Allocation/ready states, backend eligibility, future requests |

No gate is added or moved; two existing gates change what identity they compare. The G-1/G-2 blocks are kept because the identity each gate compares changes meaning, which a state-ownership table alone does not show.

### Gate G-1 — Selected Backend Identity Resolved (carried from ADR-0241)
- **Existing evidence:** ADR-0233, ADR-0241; `gateway_connect.rs:456-469` reads `backend_identities` under `service_apply_commit.read()`; written by `apply_service` after the commit.
- **Owner:** Service Dataplane. **Promise:** the workload identity applied for this `BackendId`. **Affected state:** this connect's result only.
- **Failure projection:** `BackendIdentityMissing` for absent/Reserved; connect fails closed before client mTLS. Timeout/cancel/disconnect belong to the gateway connector (ADR-0242), not this read.
- **Explicitly unaffected:** allocation lifecycle, health, Stable, hydrator generation.
- **Ordering:** after the receipt is taken, before the rustls handshake; consumes no new budget.
- **Counterexample:** gating `Running` on this read would couple allocation lifecycle to the gateway's first use (design.md "Preserve the meaning of existing states").
- **Evidence lane:** Quint `different_workload_takeover` (a receipt taken before the takeover and read after it resolves the new workload), the seeded Tier-1 invariant and the host/Sim equivalence case (§ Regression test).

### Gate G-2 — Peer is the selected workload (ADR-0346)
- **Existing evidence:** ADR-0234 and `gateway_client.rs:54-66`; client mTLS armed as kTLS only after the check (ADR-0244).
- **Owner:** gateway client mTLS. **Promise:** peer SAN equals the resolved workload identity. **Affected state:** this upstream connection only.
- **Failure projection:** typed `GatewayClientMtlsError` (`PeerSpiffeMismatch`, `PeerSpiffeShape`, `PeerCertificateMissing`, handshake/deadline) projected to the existing 502/503 outcomes; no plaintext, no alternate backend.
- **Explicitly unaffected:** backend health/eligibility and the identity association; a mismatch does not retire a backend.
- **Ordering:** within the single upstream attempt and its deadline (ADR-0240); no extra budget.
- **Counterexample:** a different workload answering at a reused address must fail (wrong-peer test with two workloads); a same-workload replacement must pass (S-PIG-71).
- **Evidence lane:** gateway integration (two workloads), Tier-3/metal S-PIG-71.

Boundary scenarios (obligations for DISTILL): (1) available: same-workload replacement selects, verifies, 204; (2) different workload at the address: 502, no fallback; (3) unaffected states: alloc/readiness untouched by a failed peer check; (4) late read after a takeover: a receipt taken before a different workload takes over the endpoint resolves, when read after the re-association, the new workload (ADR-0347, accepted consequence), never a reserved, early or mixed value, and a pre-commit failure of the takeover leaves the prior workload; (5) restart/reconnect: a `serve` restart drops intents, receipts and the memo, and the boot resync followed by a replacement converges (S-PIG-71); (6) feature disabled (no public route): no change.

## Wave: DESIGN / [REF] Formal protocol model

Subsystem `specs/quint/public-ingress-gateway/` (README there states the abstraction). Model: `selection_receipt.qnt` + `hazard/selection_receipt_hazards.qnt`; `Be.alloc` became the workload identity `Be.wl`; the memo is keyed by endpoint (R-C, DR-1(b)); a different workload at a memoised endpoint re-associates the existing `BackendId` in the commit-guarded transaction. The model has no identity-refusal action: `update_service` never refuses for identity, and the rejected alternative is recorded in ADR-0347, not as a check.

**Result:** `cargo xtask quint check --subsystem public-ingress-gateway`: 36 checks, 28 hold (4 TLC safety, 24 named runs), 8 hazards fail their named invariant as expected, 0 unexpected; evidence recorded and `verify-evidence` passes. `receipt-safety-tlc` stays `ci = false` (the manual pre-merge gate).

**What the model guarantees.**
- `AssociationFollowsProgrammedWorkload` (ghost `g.prog`: `BackendId` to the workload most recently committed for it): whenever the guard is free, the applied association of every `BackendId` equals that workload, including ids that left every generation through orphan GC. No stale association survives a takeover, and no id resolves a workload never programmed at its endpoint.
- `IdentityReadableOnlyAfterCommit`: every readable id belongs to a committed generation, and no reservation outlives the guard.
- Rollback: `RollbackPreservesPriorGeneration` also covers a re-association; a pre-commit failure leaves the prior association (run `abandoned_reservation_rolled_back` asserts it).
- A read of `selected_backend_identity` waits on the guard, so it returns the value from before or after the update, never a reserved, early or mixed one.
- Receipts across a re-association: the run `different_workload_takeover` shows a receipt taken (`sel == 1`) while id 1 meant workload 1, then the takeover leaves `ident == Map(1 -> 2)` with the memo, `outer` and the id counter unchanged, and the read for that same receipt returns workload 2: the connection is verified against the new workload. The model does not distinguish a pre-takeover receipt from any other; it guarantees the value read is the committed one. This is the accepted consequence of DR-1(b), not a violation.
- `ReceiptMatchesIntent`, `NoResidueAfterCleanup`, `EmptyIsNoBackend`, `ReverseNatKeyOneVip` (id-level: an endpoint has one id) hold.
- Not modelled: a single `update_service` call carrying two different workloads at one endpoint (stays `LoadFailed`).

| Critical scenario | Named run | Conformance-replayable |
|---|---|---|
| Happy path | `commit_then_select` | yes |
| Same-workload replacement at the endpoint (D4) | `replacement_reuses_backend_id` | yes |
| S-PIG-71 serve-restart replacement | `serve_restart_replacement` | yes |
| Different workload at a memoised endpoint (DR-1) | `different_workload_takeover` | yes |
| Reverse-NAT rejection | `reverse_nat_conflict_rejected` | yes |
| Restart | `restart_destroys_receipts` | yes |
| Rollback | `abandoned_reservation_rolled_back` | no (phases not drivable) |
| Publish guard | `receipt_waits_for_publish_guard` | no |

`serve_restart_replacement` is kept as its own run because it is the S-PIG-71 critical scenario and no other run reaches its sequence: a restart empties the memo, then the boot resync apply and the replacement apply (three accepted updates in total) converge without refusal. `replacement_reuses_backend_id` covers the same-workload memo hit with no restart and stays focused on it (plus the empty-set GC tail).

**Hazards (one per decision):** `staleAssoc` (a different workload at a memoised endpoint reuses the id but the association is not updated: violates `AssociationFollowsProgrammedWorkload`; proves ADR-0347's re-association is load-bearing). Others kept: `publishEarly`, `takeAny`, `rnatShare`.

**Pinned conformance projection** (single home; the spec README points here; no new owner API):

| Model variable | Owner accessor |
|---|---|
| `ident[BackendId]` | `GatewayConnectDataplane::selected_backend_identity(BackendId)` (returns the workload `SpiffeId`) |
| `intents` | `GatewayConnectCleanup::live_intent_count()` |
| `receipts` (per cookie) | `GatewayConnectCleanup::live_receipts()` |
| model-only: `outer`, `resv`, `infl`, `guard`, `g`, `ckey`, `conf_step`, `memo`, `next_bid`, `taken` | none; compared through `ident` and the `BackendId` in each receipt |

| Action | Driving port |
|---|---|
| `update_apply` | `Dataplane::update_service` |
| `update_rejected_rnat` | `Dataplane::update_service` returning `ReverseNatConflict` |
| `open` | `GatewayConnectDataplane::open_upstream` |
| `kernel_connect_det` | the real `cgroup/connect4` hook via a real connect |
| `take` | `take_receipt` |
| `read_identity` | `selected_backend_identity` |
| `cleanup` | `GatewayConnectCleanup::cleanup` |
| `restart` | drop and reconstruct `EbpfDataplane` |

**Assumptions:** A-K1, A-K2, A-K3, A-3, SR-1, SR-2 as in the spec README. **A-WL**: every `Backend` carries the workload identity and two allocations of one workload present equal identities; discharged by V-3.

**Underspecified points the modeler raised, and rulings.** (1) Same workload at a different endpoint: fresh id (new endpoint key), the old id swept by orphan GC. (2) Same workload at a live endpoint in two Services: refused by the reverse-NAT rule on the endpoint (this feature keeps R-59). (3) Two different workloads at one endpoint in ONE call: stays `LoadFailed`, not modelled.

## Wave: DESIGN / [REF] Validation items

None is contract-shaping and none is Open; no spike was run. The executed validation is the Quint check above. A falsification of any item changes only DELIVER internals.

| ID | Item | Class | Status |
|---|---|---|---|
| V-1 | A takeover keeps the endpoint's `BackendId`, so the Service's reverse-NAT key set and `BACKEND_MAP` entry are unchanged and only the association value for the id changes | internal | Held by source reading (`prior.difference(new)` is empty, `lib.rs:1731-1737`); covered by the host/Sim equivalence case |
| V-2 | A same-workload replacement yields an identical backend set and so an equal fingerprint (`fingerprint.rs` hashes identity and address) | internal | Held by source reading; covered by a unit test |
| V-3 | A-WL: `BackendDiscoveryBridge` and `ServiceLifecycle` build `Backend.workload_identity` from the workload with the same constructor (ADR-0079 identical-constructor equality) | internal | To be discharged by the acceptance test pinning `for_workload` at both sites |

## Wave: DESIGN / [REF] Regression test

`tests/conformance/tests/public_ingress_gateway_end_to_end.rs` S-PIG-71 `…answers_503_until_the_hydrator_reprograms_after_a_serve_restart` is unchanged and must go green on Metal. Mechanism: the boot resync programs the stored row; the replacement is the same `Backend`, so the dataplane has nothing to do, the gateway's selected `BackendId` resolves the workload identity, and the guest presents that identity. Model-level companion: `serve_restart_replacement`. **DISTILL obligations (the two companion tests, approved 2026-11-21).**
1. **Seeded `overdrive-sim` invariant** (testing.md "DISTILL — prove composed system behavior", seed printed on failure) through `ServiceMapHydrator` + `SimDataplane`, covering the restart-replacement interleaving and the different-workload takeover. Contract: replacing an allocation of the same workload at the same endpoint with no empty tick between converges (the programmed fingerprint equals the desired one, no refusal, same `BackendId`, association unchanged); a different workload programmed at that endpoint converges too (no refusal), the endpoint keeps its `BackendId`, and the association equals the new workload once converged (`assert_eventually!`), with schedule variations of `ServiceLifecycle`/`WorkloadLifecycle` order. Fault stimulus: a `serve`-restart (drop and reconstruct the dataplane, memo and receipts) before the replacement. Oracle: programmed fingerprint and `selected_backend_identity` through the dataplane's own accessors; evidence tier 1. Spec replayed: `selection_receipt.qnt` runs `serve_restart_replacement`, `different_workload_takeover`; invariant `AssociationFollowsProgrammedWorkload`.
2. **Host-vs-Sim dataplane equivalence** (`tests/integration/gateway_connect_dataplane_equivalence.rs`-shaped, `.claude/rules/development.md` "Trait definitions specify behavior"): drive `EbpfDataplane` and `SimDataplane` through three sequences and assert identical observable state through `selected_backend_identity` and the backend set after each step, and no identity-related error from either adapter: (a) `update_service([A at E])` then `update_service([A' at E])` (same workload A, replacement allocation: no change, same id, same association); (b) `update_service([A at E])` then `update_service([B at E])` (different workload: same id, association becomes B); (c) one `update_service` carrying the same endpoint twice (same workload twice, and two workloads): `LoadFailed` from both adapters with no state change.

Both are part of the approved fix (RCA §9 and the 2026-11-21 ruling), not additions.

## Wave: DESIGN / [REF] FINALIZE documentation items

`website/content/docs/concepts/identity.mdx` states the per-allocation identity shape and is updated at FINALIZE to describe the workload-grain identity, the per-allocation SVID and the audit-row `AllocationId` (`.claude/rules/finalize.md`).

## Proposals the user declined

PA-2 (phased memo-coexistence run), PA-3 (operator-visible state for a repeating `dataplane.update_service.failed`), PA-4 (`ci = true` on `receipt-safety-tlc`) and PA-5 (lint rejecting `/alloc/` SPIFFE literals): declined 2026-11-21, not part of this design.

## Wave: DESIGN / [REF] Handoff

DISTILL may start on D1–D5, DR-1(b) and every named run (all conformance-replayable runs are final). No scope limitation or deferral is introduced by this design. Contract-test annotation for platform-architect: the public gateway to workload hop is an internal mTLS boundary owned by us; there is no external third-party service in this change. Enforcement: the existing compiler (removing `for_allocation` makes a return a compile error) and conformance-lint; no new gate is added.

## Review dispositions

Round 1 (`design/review-round1.md`):

1. (A-H1, A-M2, S-2) Applied: one greenfield single-cut policy in DD-5, ADR-0344, ADR-0345 and the `IssuedCertificateRow` contract: V1 `into_latest` returns `EnvelopeError`, no `/alloc/` parser, frozen-V1 fixture asserts that projection and is regenerated as an explicit `Envelope::V1` if V2's larger root breaks the literal.
2. (A-M3) Applied: § Changed Assumptions quotes ADR-0241, ADR-0234 and ADR-0067/0075 with their superseders.
3. (A-M4) Applied: a duplicate endpoint in one `update_service` call is `LoadFailed` regardless of identity (contract rustdoc, host and Sim), added as equivalence sequence (c).
4. (A-M5, S-1) Applied: `rejectTakeover`, `UpdateNeverRefusedForIdentity`, `update_refused` and `g.refused` removed from the model; Quint re-run, 36 checks, 0 unexpected.
5. (S-6) Applied by deleting ADR-0348 and folding it into ADR-0347 (one decision, ADR-0241 superseded by it alone); alternatives trimmed, model-check names out of Status, internal API detail dropped; links updated. The file still exists as a one-line stub until the orchestrator removes it with `git rm`.
6. (S-7) Applied: ADR-0345 kept and trimmed (greenfield consequence); ADR-0346 duplicated consequence and test obligation dropped.
7. (S-3) Applied: Lifecycle Gate Ownership section removed from the ADR; it lives here.
8. (S-5) Applied: `serve_restart_replacement` kept with its reason stated, its pre-restart receipt segment and duplicated middle expect removed; `replacement_reuses_backend_id` reduced to the memo hit plus the GC tail.
9. (A-L6) Applied: conformance driver takes `BackendId`s from receipts or the owner, never a driver counter (non-mechanical item 6).
10. (A-L7, S-4) Applied: rename-pending, trace-removal, no-shell, wall-time/memory, deletion-history and empty decision-request text removed; one-line declined-proposals note kept.
11. (A-L8) Applied: glued README bullets fixed.
12. (A-L9) Applied: § FINALIZE documentation items lists `website/content/docs/concepts/identity.mdx`.
13. (S-9, S-10) Applied: projection tables have one home (here; the README points to it); driver-internal prescriptions, file lists and counts trimmed to behavioural obligations; V-4 and V-5 dropped.
- Declined: S-8 (keep the G-1/G-2 gate blocks: the identity they compare changes meaning).
