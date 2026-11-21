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

## Wave: DISTILL / [REF] Scenario list

No `.feature` files (`.claude/rules/testing.md`); every scenario is a Rust test. Reconciliation: DISCUSS has no wave-decisions for this feature; DESIGN's D1-D5, DR-1(b), PA-1 and the approved TC pair are mutually consistent; 0 contradictions. RED state is classified in [`distill/red-classification.md`](distill/red-classification.md). "Pending" = complete body + `#[should_panic(expected = ...)]` naming the semantic RED (removed by DELIVER; the body runs unchanged). "Guard" = green today, pins the post-feature contract. "Held" = names API that does not compile until DELIVER; gated `#![cfg(any())]` with a `distill-pending-owner-api` marker (see Held tests).

| ID | Test (file) | Decision / AC | Lane | Placement | State and RED reason |
|---|---|---|---|---|---|
| S-WSI-01 | `a_workload_identity_round_trips_to_its_workload` (proptest), `the_payments_workload_has_the_documented_identity` (`overdrive-core/tests/acceptance/workload_spiffe_identity.rs`) | D1, DD-1 | unit, default | `overdrive_test_default!(host_pure)` | Pending: `for_workload` is a scaffold ("not delivered yet") |
| S-WSI-02 | `the_superseded_allocation_shaped_identity_names_no_workload` (proptest; the one `/alloc/`-shape rejection), `identities_that_are_not_exactly_a_workload_path_name_no_workload` (same file; builds every identity before asserting) | D1, DD-5 (no parser of `/alloc/`) | unit, default | host_pure | Pending: `workload_id` is a scaffold |
| S-WSI-03 | `issued_certificate_row_v1_rows_are_refused_and_never_translated` (`overdrive-core/tests/schema_evolution/issued_certificate_row.rs`) | D2, DD-5, ADR-0345 | schema evolution, default | host_pure | Pending: today the pre-V2 literal decodes as the latest row |
| S-WSI-04 | `issued_certificate_row_v2_golden_bytes_roundtrip_preserves_issued_for` (same file; `FIXTURE_V2`) | D2 | schema evolution | host_pure | Pending: today `V2` bytes are an unknown version |
| S-WSI-05 | `issued_certificate_row_envelope_declares_v1_and_v2_with_the_repinned_offset` (same file) | D2 (`known_discriminants [0, 1]`, offset 104 to 120) | schema evolution | host_pure | Pending: today `[0]` / `104` |
| S-WSI-06 | `a_replacement_allocation_presents_the_identity_of_its_workload`, `an_initial_allocation_presents_the_identity_of_its_workload` (`overdrive-reconcilers/tests/acceptance/vm_recreation_allocation_identity.rs`); the file's per-alloc identity assertions are removed (superseded) | D1; non-mechanical item 2 | unit (pure `reconcile`), default | module `placement!`/binary default as the file | Pending: `AllocationSpec.identity` is `.../workload/<w>/alloc/<a>` |
| S-WSI-07 | `two_allocations_of_one_workload_each_hold_their_own_svid_naming_the_workload`, `after_a_serve_restart_both_allocations_recover_their_own_svid_for_the_workload` (`overdrive-control-plane/tests/acceptance/workload_svid_identity_shared_by_allocations.rs`, `integration-tests` gate like its sibling) | D1, D2 (own SVID per allocation; restart recovery) | in-process composition through `run_convergence_tick`, `SimCa`/`SimObservationStore` | `placement!(host_pure)` | Pending: held SVID identity is per-allocation; the per-allocation serial assertion holds and runs first |
| S-WSI-08 | `one_call_naming_an_endpoint_twice_is_refused_and_changes_nothing` (`overdrive-sim/tests/acceptance/workload_identity_replacement.rs`) | D4, DR-1(c); equivalence (c) Sim half; the only default-lane check of a duplicate endpoint in one `update_service` call. Sequences (a) and (b) are covered by S-WSI-09 (Sim) and S-WSI-10 (host and Sim) | in-memory (`SimDataplane`) | `placement!(host_pure)` | Pending: same-workload duplicate endpoint is accepted (should be `LoadFailed`) |
| S-WSI-09 | `a_replacement_converges_without_refusal_across_restarts_and_schedules` (proptest over a seed; same file) | approved TC (i): seeded restart-replacement interleaving + different-workload takeover through the real `ServiceMapHydrator` + `SimDataplane` | seeded in-memory (tier 1) | host_pure | Pending: "the dataplane refused the update" names the seed; a refusal that also changed the programmed fingerprint would fail with its own message (the programmed fingerprint is read back after every refusal) |
| S-WSI-10 | `host_and_sim_agree_that_a_replacement_allocation_of_the_same_workload_changes_nothing`; `..._a_different_workload_at_a_memoised_endpoint_keeps_the_id`; `host_and_sim_refuse_one_call_that_names_an_endpoint_twice` (`overdrive-dataplane/tests/integration/gateway_connect_dataplane_equivalence.rs`); S-PIG-67's superseded rejection step is removed; rig `alloc(n)` is now workload `gw-<n>` | approved TC (ii) (a)(b)(c); item 1 | real `EbpfDataplane` + Sim, `integration-tests` | `#[overdrive_test(guest = "mtls-relay")]` | (a) guard; (b) Pending: host refuses the takeover; (c) Pending: same workload twice is accepted (deduplicated) instead of `LoadFailed` |
| S-WSI-11 | `selection_receipt_replacement_reuses_backend_id`, `..._serve_restart_replacement`, `..._different_workload_takeover`, `..._receipt_conf_randomized_traces` (`overdrive-dataplane/src/quint_selection_receipt.rs`); the `update_rejected` handler and the `conflicting_alloc_rejected` / `identity_conflict_precedes_reverse_nat` pins are removed | Formal protocol model: runs `replacement_reuses_backend_id`, `serve_restart_replacement`, `different_workload_takeover`; `AssociationFollowsProgrammedWorkload` | quint-connect conformance on `EbpfDataplane`, `integration-tests` | module `placement!(host_pure)` (as before) | first two guards; takeover and randomized Pending: `update_service must be accepted: backend id ... is bound to` |
| S-WSI-12 | `a_peer_presenting_another_identity_than_the_selected_workloads_is_rejected_exactly` (`overdrive-control-plane/tests/integration/public_ingress_gateway_mtls.rs`) | D3 / G-2: exact equality, no prefix | real client mTLS, `integration-tests` | `guest = "cgroup-default"` | Guard (check unchanged, DD-4) |
| S-PIG-71 | `native_metal_kvm_public_ingress_gateway_answers_503_until_the_hydrator_reprograms_after_a_serve_restart` (`tests/conformance/tests/public_ingress_gateway_end_to_end.rs`), UNCHANGED | regression anchor | metal guest | `guest = "cgroup-default", profile = "nested-launch-rustc"` | RED (fails today, not `should_panic`): 60 s of 502 |
| S-WSI-13 | `every_allocation_of_a_workload_is_issued_the_workload_identity` (proptest; `overdrive-core/tests/acceptance/workload_spiffe_identity.rs`, moved out of held H-1: it compiles today) | D1, D2: one `IssueSvid` per allocation, all naming the workload identity | unit (pure `reconcile`), default | host_pure | Pending: `IssueSvid` carries the per-allocation identity |
| S-WSI-14 | `a_running_allocation_is_published_behind_its_workloads_identity` (`overdrive-control-plane/tests/acceptance/service_backend_names_its_workload.rs`) | D1, V-3: the identity the Service publishes for a Running allocation (production shim + `ServiceLifecycle` hydrate, no authored observation) is the workload's | in-process composition, seeded `overdrive-sim` | `placement!(host_pure)` | Pending: published identity is `.../workload/<w>/alloc/<a>` |
| Held H-1 | `svid_lifecycle_allocation_keyed_recovery.rs` (core acceptance, 2 tests): restart recovery for the issued allocation and not its sibling; held attribution per allocation | D2, DD-3 | unit (pure `reconcile`) | binary default | Held: `ever_issued: BTreeSet<AllocationId>`, real `for_workload` |
| Held H-2 | `alloc_status_per_allocation_certificates.rs` (control-plane integration, 1 test): each running allocation lists its own certificate under a shared identity | D2 | in-process, `integration-tests` | `guest = "cgroup-default"` | Held: `issued_for` / `IssuedFor` on the row alias |

Doubles and constants moved to workload identities (compile-checked, green): the gateway integration doubles' `backend_identity` and the `other` peer (`public_ingress_gateway.rs`, `public_ingress_gateway_relay.rs`), `connector.rs` test constants, `public_ingress_gateway_mtls.rs` `SELECTED_PEER_SPIFFE`.

Counts: 23 new active tests (19 pending-RED, 4 guards) plus 3 held; 6 existing tests re-shaped; error/edge share is above 40% (refusals, takeover, duplicate endpoint, V1 refusal, non-workload shapes).

## Wave: DISTILL / [REF] Fault obligations (composed system)

Per `.claude/rules/testing.md` "DISTILL: prove composed system behavior". Each row: contract, owner path, stimulus, oracle, tier, bound.

| Contract | Production owner path | Fault stimulus | Outcome oracle | Tier | Bound |
|---|---|---|---|---|---|
| A replacement of the same workload at a memoised endpoint converges, same `BackendId`, no refusal | `ServiceMapHydrator` -> `Action::DataplaneUpdateService` -> `update_service` (S-WSI-09) | `serve` restart (dataplane, memo, receipts rebuilt) before the replacement; 0-2 idle ticks | programmed fingerprint and `selected_backend_identity` through the dataplane's accessors | 1, seeded (`seed` named on failure) | 8 hydrator ticks + idle ticks |
| A different workload at the endpoint converges; id kept; association follows | same | same, plus takeover of the endpoint by another workload | same | 1, seeded | same |
| The real guest recovers through the gateway after `serve` restarts | `overdrive serve` composition, `ServiceMapHydrator`, gateway, guest (S-PIG-71) | stop and restart `serve` on the same data directory, replacement allocation | `204`/`200` through the public route within the 60 s window | 3 metal | 60 s recovery window (existing) |
| Host and Sim agree on replacement, takeover and a duplicate endpoint | `EbpfDataplane` vs `SimDataplane` via `Dataplane` and `GatewayConnectDataplane` (S-WSI-10) | rejected duplicate call; takeover | selection identity, `BackendId`, map state through each adapter's own accessors | 3 guest `mtls-relay` | one script per adapter |
| A receipt taken before a takeover and read after it resolves the new workload; never reserved, early or mixed | quint-connect replay of `different_workload_takeover` on `EbpfDataplane` | takeover between `take` and `read_identity` | `ident[id]` per step | 3 | 1 trace (+ 100 random) |
| A pre-commit failure of a takeover leaves the prior association | run `abandoned_reservation_rolled_back` is not conformance-replayable (phases not drivable); existing rollback fault tests (`service_apply_rollback_faults.rs`) stay the evidence | - | - | n/a | recorded gap: no new test (no drivable production seam; surfaced, not invented) |

Recorded gap (G-2, ADR-0346): a same-workload replacement that presents the workload identity and passes G-2 at the gateway is covered only by the Metal S-PIG-71; no default-lane test drives a real client handshake across a replacement (S-WSI-12 pins the exact-equality check; S-WSI-09/10 pin the dataplane).

Boundary scenarios of Lifecycle Gate Ownership: (1) available: S-PIG-71, S-WSI-09; (2) different workload at the address: S-WSI-12 (exact mismatch) and S-WSI-09/S-WSI-10(b) at the dataplane; (3) unaffected states: no test changes Running/readiness, none is asserted by the new tests; (4) late read after takeover: S-WSI-10(b), run `different_workload_takeover`; (5) restart: S-WSI-09, S-PIG-71, `serve_restart_replacement`; (6) feature disabled: unchanged by construction, no new test.

## Wave: DISTILL / [REF] Adapter coverage

| Adapter | Real-I/O scenario | Covered by |
|---|---|---|
| `EbpfDataplane` (`update_service`, `selected_backend_identity`) | yes | S-WSI-10, S-WSI-11, S-PIG-71 |
| `SimDataplane`, `SimGatewayConnectDataplane` | in-memory | S-WSI-08, S-WSI-09, S-WSI-10 |
| `IdentityMgr` + `SimCa` + `SimObservationStore` (SVID issuance) | in-process | S-WSI-07 |
| `GatewayClientMtls` (real rustls/kTLS) | yes | S-WSI-12 (guard) and existing S-PIG-38/43/52/64 |
| `issued_certificates` observation row codec (rkyv envelope) | yes (bytes) | S-WSI-03/04/05 |
| redb / Corrosion | not touched | no adapter change |

## Wave: DISTILL / [REF] Driving ports

`Dataplane::update_service`, `GatewayConnectDataplane::{open_upstream, take_receipt, selected_backend_identity}`, `GatewayConnectCleanup::{live_intent_count, live_receipts, cleanup}`, `run_convergence_tick` for `svid-lifecycle`, `AnyReconciler::reconcile` (pure), `GatewayClientMtls::authenticate`, `alloc_status` handler (held), `SpiffeId::{for_workload, workload_id}`, `IssuedCertificateRow::{archive_for_store, from_store_bytes}`, and the production `serve` through the S-PIG-71 conformance test. The Quint driver's action-to-port table is the one pinned in Formal protocol model (handlers all call the owner; `State` reads only `selected_backend_identity`, `live_intent_count`, `live_receipts`; no counter or memo in the driver).

## Wave: DISTILL / [REF] Scaffolds

Production scaffolds, exact shapes from Interface contracts, no other surface: `SpiffeId::for_workload`, `SpiffeId::workload_id` (`overdrive-core/src/id.rs`, panic bodies naming their absence, `#[expect(clippy::todo)]`); `IssuedFor` and `IssuedCertificateRowV2` (`ca/issued_certificate_row.rs`, type definitions only, NOT wired: the alias, the envelope variant and `issue_and_audit` are DELIVER's one version-bump commit). DELIVER replaces the bodies, removes the matched `should_panic` markers, and removes the scaffold markers.

## Wave: DISTILL / [REF] Held tests (cannot compile until DELIVER's API change)

| Held test | Names | Activated by |
|---|---|---|
| `svid_lifecycle_allocation_keyed_recovery.rs` (2 tests) | `SvidLifecycleState.ever_issued: BTreeSet<AllocationId>`, real `for_workload` | the `SvidLifecycle` re-key step; DELIVER removes `#![cfg(any())]` and deletes the identity-keyed `ever_issued` fixtures of `svid_lifecycle_reconcile.rs` and `svid_lifecycle_rotation.rs` (S-OC-05), whose type disappears |
| `alloc_status_per_allocation_certificates.rs` (1 test) | `IssuedCertificateRow.issued_for`, `IssuedFor` on the alias | the V2 row step |

Mechanical edits DELIVER makes in existing tests (compiler-enumerated, not new assertions): `Backend.alloc` to `Backend.workload_identity` (one helper in each of `quint_selection_receipt.rs::backend`, `gateway_connect_rig::backend`, and the sim tests' `backend_at`, plus the fingerprint/hydrator fixtures); `IssuedCertificateRow { .. }` literals gain `issued_for`; `for_allocation(..)` call sites in tests become `for_workload(..)` (about 50 files); the two harness calls in `issued_certificate_row.rs` (`discriminant_offset` triangulation, unknown-version probe) re-point to the V2 payload with offset `120` and `supported_max = 1` (values measured here), `FIXTURE_V1` stays untouched (S-WSI-03 asserts the pre-V2 literal is refused, `is_err()`), and `FIXTURE_V1_UNDER_V2_ROOT` sits beside it (asserted to be `ObservationStoreError::Envelope`, the design pins no finer `EnvelopeError` variant); `canonical_v1_payload()` already returns `IssuedCertificateRowV1`; the ignored `print_fixture_v1_bytes` regeneration aid encodes `Envelope::latest(<V1 payload>)`, which stops compiling when V2 is the latest payload: DELIVER re-points it to a V2 payload (it is `#[ignore]`d and not a test).

Held-file compile proof (throwaway worktree, never committed): against a local patch that re-keys `ever_issued` to `BTreeSet<AllocationId>` and wires V2 (alias, envelope variant, `into_latest`, offset 120, `known_discriminants [0, 1]`), the gate-removed bodies of H-1 and H-2 compile (`cargo check --all-targets --features integration-tests -p overdrive-core -p overdrive-reconcilers -p overdrive-control-plane`); the recovery test passes there; `held_attribution` fails only on the `for_workload` scaffold (the patch did not implement it). The V2 round trip was measured on the same patch: `FIXTURE_V2` equals the canonical V2 archive byte for byte (the V2 payload plus `Allocation("alloc-dns-resolver-0")`, encoded through the envelope), the V1 payload under the V2 root is refused with an envelope error, and the offset-120 / `[0, 1]` / tag-1 triangulation pass. The remote checkout the proof used is listed in the hand-off for cleanup.

Non-mechanical list (feature delta item 1-5), audited: (1) wrong-peer tests built from two allocations of one workload: none existed at the real client mTLS tier or the Sim tier (`ns/default/sa/*`, `workload/other` identities); the tests that derived one identity per backend from one workload (`backend_identity` doubles in the two gateway integration files, `gateway_connect_rig::alloc`, the Quint driver's `alloc_identity`, `connector.rs` constants) were moved to workload identities, the equivalence test's re-lease rejection (S-PIG-67 step 3) was removed as superseded; (2) recreation inequality: `vm_recreation_allocation_identity.rs` re-shaped (S-WSI-06); (3) fingerprint/backend-key generators: `sim_dataplane_programmed_fingerprint_contract.rs` and `fingerprint.rs` distinguish backends by address as well as identity and stay valid; (4) SVID tests: S-WSI-07 plus held H-1/H-2; (5) constructor tests: S-WSI-01/02.

## Wave: DISTILL / [REF] EDD expectation decision

No EDD expectation is authored. S-PIG-71 is a deterministic, rerunnable conformance test in the metal lane (the contract is "`serve` restart then replacement converges"), which `.claude/rules/verification.md` puts in the test tiers, not the catalogue; an expectation would duplicate it. The operator-visible `alloc status` / certificate-listing claim has an in-process test (held H-2). If FINALIZE wants a point-in-time record of the operator journey, the claim to write is "after `overdrive serve` restarts, a deployed service with a public route answers again without operator action".

## Wave: DISTILL / [REF] Mandate compliance

CM-A: tests import driving ports and the pure-function public signatures only. CM-B: scenario names use domain terms; step bodies call production services. CM-C: the walking skeleton is S-PIG-71 (existing, a user goal: the public route answers after `serve` restarts). Mandate 8 universes: S-WSI-09/10 compare selected identity, `BackendId`, map state, programmed fingerprint (port-exposed names only). Mandate 9/11: PBT only at layers 1-2 (core, sim); layers 3+ example-only. Mandate-12 is a Python-pilot mandate; this Rust repo uses the project helpers (`backend_at`, `backend`, `workload_identity`) so the pinned field name appears once per test module. Tier B: not applicable (no journey of three chained scenarios). Completeness audit (C1-C7): passes C1a/b (grammar edges), C3 (0/1/N backends: empty-set tail in `replacement_reuses_backend_id`), C4a (apply-twice: S-WSI-10(a)), C5 (flags: same/different workload x restart x idle ticks), C6 (typed `EnvelopeError`, `LoadFailed`), C7a/b (restart interruption); documented gaps: C7c concurrent updates (the commit guard is Quint-checked, not driven), the rollback fault above, and the G-2 gap above.

## Wave: DISTILL / [REF] Review dispositions

Round 1 (`distill/review-round1.md`): applied unless stated.

1. SEN-1: `identities_that_are_not_exactly_a_workload_path_name_no_workload` builds every identity first, then asserts; `SpiffeId::new` rejects none of the nine inputs (it checks scheme, trust domain and a non-empty path).
2. SEN-2: S-WSI-09 expects "the dataplane refused the update" and reads the programmed fingerprint back after a refusal (a changed one fails with its own message); the claim about `ServiceLifecycle` / `WorkloadLifecycle` order is dropped (only idle ticks vary).
3. SEN-3: S-WSI-14 discharges V-3 at the `ServiceLifecycle` publishing site through the production shim path; the expected identity is the wire shape S-WSI-01 pins, so the RED is the per-allocation identity and not the constructor scaffold. The workload-lifecycle site of the derivation is S-WSI-06; `BackendDiscoveryBridge` no longer exists in the tree.
4. SEN-5 / SUB-10: held files stay gated; compile proof recorded under Held tests; `every_allocation_of_a_workload_is_issued_the_workload_identity` is S-WSI-13 (active, pending-RED).
5. SEN-6: S-WSI-03 asserts the envelope error kind for `FIXTURE_V1_UNDER_V2_ROOT`; the delta text now says `FIXTURE_V1` stays untouched; `FIXTURE_V2` provenance and round trip recorded under Held tests.
6. SEN-7: the randomized Quint test pins `seed = "1"` and `max_samples = 100`; its matched RED reproduced in the gate run with the pinned seed, and it stays `should_panic` because the takeover is refused today.
7. SEN-8: driver header states the `PROBED_BACKEND_IDS` bound and the `held` cookie translation, and no longer says the tests are active; `read_identity` fails the step on any owner error other than `BackendIdentityMissing`.
8. SEN-9: recorded in Fault obligations (G-2 gap); no new test (prefer recording).
9. SEN-10: the unchanged S-PIG-71 does not print tracing events (the conformance harness records them in-process and prints only the test's own assertion), so no `BackendIdentityConflict` line exists in its output and the test is not changed. The refusal text is attached in `red-classification.md` from the Sim run. S-PIG-71 is not `should_panic` because it is a guest-placed conformance test whose RED is the run itself; the Metal lane stays red until DELIVER.
10. SUB-1: S-WSI-08 (a) and (b) removed; (c) kept.
11. SUB-4, SUB-5, SUB-6: dead `_workload` parameter (and `assert_restart_identity` renamed `assert_restart_allocation`, its identity assertion having been removed), `issue_for_both` wrapper and the fresh-constructor `held_snapshot().is_empty()` assertion removed.
12. SUB-7: one `/alloc/`-shape rejection remains (the S-WSI-02 property); the example list carries no `/alloc/` entry.
13. SUB-11: the H-1 proptest exists once (S-WSI-13). SUB-13: counts fixed. SUB-14: new and touched files rustfmt-clean.
- Declined (per the review): SUB-3, SUB-8 pre-V2 half, SUB-9.
