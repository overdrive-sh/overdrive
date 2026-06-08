<!-- markdownlint-disable MD013 MD024 -->
# Test Scenarios — workload-identity-manager (GH #35 · roadmap step 2.13)

**Wave**: DISTILL · **Density**: lean (Tier-1 `[REF]` only)  
**Authority**: ADR-0067 rev 2 + `docs/feature/workload-identity-manager/slices/`

> **SPECIFICATION ONLY — not executed.** This workspace does not use `.feature`
> files. The GIVEN/WHEN/THEN blocks below are the human-readable specification
> companion for Rust scaffold tests under `crates/*/tests/{acceptance,integration}`.
> DELIVER replaces the `RED scaffold` bodies with real assertions.

## Reading Checklist

| Artifact | Status | Notes |
|---|---|---|
| `docs/product/journeys/hold-identity-for-the-running-set.yaml` | + read | J-SEC-002 journey; patched from rev-1 restart recompute wording to ADR-0067 rev-2 bounded audited re-issue before scenario writing. |
| `docs/product/journeys/issue-workload-identity.yaml` | + read | J-SEC-001 sibling; confirms #35 holds/reads/drops what built-in-ca mints. |
| `docs/product/architecture/brief.md` | + read | Workload identity extension, driving/driven ports, quality scenarios, out-of-scope boundaries. |
| `docs/product/architecture/adr-0067-workload-identity-manager-svid-lifecycle.md` | + read | Latest accepted design authority. |
| `docs/product/kpi-contracts.yaml` | + read | Docs-platform-only KPI contract; no #35 KPI entries expected there. |
| `docs/feature/workload-identity-manager/discuss/user-stories.md` | - not found | DISCUSS lives in `feature-delta.md`; traceability derived from that file. |
| `docs/feature/workload-identity-manager/discuss/story-map.md` | - not found | Story map lives in `feature-delta.md`. |
| `docs/feature/workload-identity-manager/discuss/wave-decisions.md` | - not found | Decisions consolidated in root `wave-decisions.md`. |
| `docs/feature/workload-identity-manager/feature-delta.md` | + read | DISCUSS + DESIGN handoff; stale handoff wording patched before scenarios. |
| `docs/feature/workload-identity-manager/wave-decisions.md` | + read | DIVERGE decisions D-WIM-1..8; Option 1 locked. |
| `docs/feature/workload-identity-manager/design/review-design.md` | + read | Conditional approval; cleanup items checked and addressed where live handoff was stale. |
| `docs/feature/workload-identity-manager/design/upstream-changes.md` | + read | Records O4/K3 rev-2 back-propagation, already applied. |
| `docs/feature/workload-identity-manager/slices/slice-01-issue-hold-drop-audit-converge.md` | + read | Slice 01 acceptance source. |
| `docs/feature/workload-identity-manager/slices/slice-02-identity-read-port-and-consumer-surface.md` | + read | Slice 02 acceptance source. |
| `docs/feature/workload-identity-manager/slices/slice-03-restart-idempotence-and-gated-rotation-seam.md` | + read | Slice 03 acceptance source. |
| `docs/feature/workload-identity-manager/devops/wave-decisions.md` | - not found | Warning only; single-node default + existing integration-test/Lima policy applies. |

## Reconciliation

**Result**: passed after cleanup — 0 unresolved contradictions.

One stale live artifact contradicted ADR-0067 rev 2: the J-SEC-002 journey still
said restart recomputes held state from persisted issuance inputs without
re-issue. That is impossible because the held leaf key is non-persistable. The
journey now matches ADR-0067 rev 2: after restart, the held set starts empty and
every still-Running allocation is re-issued once during recovery, with an audit
row for each re-issue.

DIVERGE artifacts that preserve the earlier hypothesis are treated as historical
record and are superseded by ADR-0067 rev 2 plus `design/upstream-changes.md`.

## Driving Ports

| Port | Mechanism | Scenarios |
|---|---|---|
| `SvidLifecycle::reconcile` | Direct pure call in Rust acceptance tests | S-WIM-01, S-WIM-03, S-WIM-08, S-WIM-09 |
| Workload lifecycle / exit observer handoff | Direct reconciler/observer action output assertions | S-WIM-10 |
| Action-shim dispatch for `IssueSvid` / `DropSvid` | Direct action-shim call with sim stores/fakes | S-WIM-02, S-WIM-07 |
| `IdentityRead` port | Direct trait calls against `IdentityMgr` and `SimIdentityRead` | S-WIM-04, S-WIM-05, S-WIM-06 |
| Integration test host | Real stores + real CA chain + `openssl verify`, gated by `integration-tests` | S-WIM-WS, S-WIM-12 |
| DST invariant harness | Seeded sim invariant | S-WIM-11 |

## Scenario Index

| ID | Title | Scaffold | Tags | Layer | Trace |
|---|---|---|---|---|---|
| S-WIM-01 | Running alloc without held SVID emits `IssueSvid` | `overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs::running_alloc_without_held_svid_emits_issue_svid` | `@in-memory @property` | L1 | US-WIM-01, O1, ADR-0067 D1/D2 |
| S-WIM-02 | `IssueSvid` executor audits before hold | `overdrive-control-plane/tests/acceptance/issue_svid_action_shim.rs::issue_svid_executor_audits_before_hold` | `@in-memory` | L1/L2 | US-WIM-01, US-WIM-03, O5, ADR-0067 D3 |
| S-WIM-03 | Stopped alloc with held SVID emits `DropSvid` | `overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs::stopped_alloc_with_held_svid_emits_drop_svid` | `@in-memory` | L1 | US-WIM-01, O2, ADR-0067 D1/D2 |
| S-WIM-04 | `IdentityRead` returns SVID + trust bundle without re-issue | `overdrive-control-plane/tests/acceptance/identity_mgr_read_contract.rs::identity_read_returns_svid_and_trust_bundle_without_reissue` | `@in-memory` | L1/L2 | US-WIM-02, O3, ADR-0067 D6/D7 |
| S-WIM-05 | `IdentityRead` returns absence after drop | `overdrive-control-plane/tests/acceptance/identity_mgr_read_contract.rs::identity_read_returns_none_after_drop` | `@in-memory @error` | L1/L2 | US-WIM-02, O2, ADR-0067 D7 |
| S-WIM-06 | `SimIdentityRead` matches `IdentityMgr` read contract | `overdrive-sim/tests/acceptance/identity_read_equivalence.rs::sim_identity_read_matches_identity_mgr_contract` | `@in-memory @property` | L2 | US-WIM-02, O3, ADR-0067 D7/D9 |
| S-WIM-07 | Audit-write failure refuses hold | `overdrive-control-plane/tests/acceptance/issue_svid_action_shim.rs::audit_write_failure_refuses_hold` | `@in-memory @error` | L1/L2 | US-WIM-03, O5, ADR-0063 D6 |
| S-WIM-08 | View is retry memory only | `overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs::svid_lifecycle_view_is_retry_memory_only` | `@in-memory @property` | L1 | US-WIM-01, O4/O6, ADR-0067 D8 |
| S-WIM-09 | Rotation seam is emit-gated until #40 | `overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs::near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered` | `@in-memory @error` | L1 | D-WIM-8, ADR-0067 D8 |
| S-WIM-10 | Lifecycle transitions enqueue `SvidLifecycle` | `overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs::workload_lifecycle_transitions_enqueue_svid_lifecycle` | `@in-memory` | L1 | ADR-0067 D5b |
| S-WIM-11 | Running-set identity invariant has teeth | `overdrive-sim/tests/acceptance/identity_read_equivalence.rs::running_set_identity_invariant_fails_on_broken_hold_or_drop` | `@in-memory @dst_invariant @property` | L2 | O1/O2, K1, ADR-0067 D9 |
| S-WIM-WS | Walking skeleton: issue, hold, audit, verify, drop | `overdrive-control-plane/tests/integration/workload_identity_manager/lifecycle.rs::walking_skeleton_running_alloc_issues_holds_audits_and_verifies_svid` | `@walking_skeleton @real-io @adapter-integration` | L3 | Slice 01 WS, O1/O2/O5 |
| S-WIM-12 | Restart re-issues each still-running alloc with audit row | `overdrive-control-plane/tests/integration/workload_identity_manager/lifecycle.rs::restart_reissues_each_still_running_alloc_with_audit_row` | `@real-io @error` | L3 | Slice 03, O4/K3, ADR-0067 D1/D8 |

## Scenarios

### S-WIM-01 — Running Alloc Without Held SVID Emits `IssueSvid`

```gherkin
Given a SvidLifecycle desired set containing a Running allocation
And the IdentityMgr held snapshot has no SVID for that allocation
When SvidLifecycle::reconcile is driven through the pure reconciler port
Then it emits exactly one Action::IssueSvid with alloc_id, spiffe_id, node_id, and correlation
And reconcile performs no CA I/O and no ObservationStore I/O
```

Universe: emitted action list + next View. No private runtime fields.

### S-WIM-02 — `IssueSvid` Executor Audits Before Hold

```gherkin
Given an Action::IssueSvid for a running allocation
And a CA adapter whose issue_and_audit call returns SvidMaterial and writes an audit row
When the action shim dispatches the action
Then the issued_certificates row is observable
And IdentityMgr holds the returned SvidMaterial for the allocation
And the held cert serial matches the audit row serial
```

Universe: action-shim result, `IdentityMgr` held snapshot, ObservationStore audit row.

### S-WIM-03 — Stopped Alloc With Held SVID Emits `DropSvid`

```gherkin
Given the desired running set no longer contains an allocation
And IdentityMgr actual contains a held SVID for that allocation
When SvidLifecycle::reconcile is driven
Then it emits Action::DropSvid for that allocation
And no IssueSvid is emitted for the stopped allocation
```

Universe: emitted actions and next View.

### S-WIM-04 — `IdentityRead` Returns SVID + Bundle Without Re-Issue

```gherkin
Given IdentityMgr holds SvidMaterial for an allocation
And IdentityMgr has a hydrated TrustBundle
When a consumer reads through IdentityRead::svid_for and current_bundle
Then it receives owned clones of the current SVID and bundle
And no CA issuance call happens on the read path
```

Universe: `IdentityRead` return values and a CA call counter.

### S-WIM-05 — `IdentityRead` Returns Absence After Drop

```gherkin
Given IdentityMgr held an SVID for an allocation
And DropSvid has removed that allocation
When a consumer reads through IdentityRead::svid_for
Then the result is explicit absence
And the consumer cannot observe a stale credential for the stopped allocation
```

Universe: `IdentityRead::svid_for` result for the dropped allocation.

### S-WIM-06 — `SimIdentityRead` Matches `IdentityMgr`

```gherkin
Given the same fixture SVIDs and TrustBundle are loaded into IdentityMgr and SimIdentityRead
When the test consumer reads both adapters through IdentityRead
Then every held allocation returns equivalent SvidMaterial
And absent allocations return absence in both adapters
And current_bundle returns equivalent trust-bundle material
```

Universe: trait-observable values only.

### S-WIM-07 — Audit-Write Failure Refuses Hold

```gherkin
Given an IssueSvid action
And ca_issuance::issue_and_audit fails while writing the issued_certificates row
When the action shim dispatches the action
Then dispatch reports the audit failure
And IdentityMgr does not hold SvidMaterial for the allocation
And no unaudited SVID is handed to a consumer
```

Universe: dispatch error + empty held map entry.

### S-WIM-08 — View Is Retry Memory Only

```gherkin
Given SvidLifecycleView is serialized and deserialized by the reconciler runtime
When the test inspects the public View shape
Then the View contains only retry memory for failed IssueSvid attempts
And it contains no serial, issued_at, spiffe_id, expires_at, or next_renewal_at success fact
```

Universe: public View type shape / serde round trip.

### S-WIM-09 — Rotation Seam Is Emit-Gated Until #40

```gherkin
Given a held cert is near expiry
And cert_rotation is not registered in the workflow engine
When SvidLifecycle::reconcile evaluates the near-expiry branch
Then no StartWorkflow(cert_rotation) action is emitted
And no UnknownWorkflow error can be produced per tick
```

Universe: emitted action list.

### S-WIM-10 — Lifecycle Transitions Enqueue `SvidLifecycle`

```gherkin
Given WorkloadLifecycle emits a transition that changes allocation running state
When the action list is produced for Running or Stopped transition
Then it includes Action::EnqueueEvaluation targeting SvidLifecycle for job/<workload_id>
And SvidLifecycle can tick without a manual broker poke
```

Universe: emitted action list from the lifecycle/exit-observer driving ports.

### S-WIM-11 — Running-Set Identity Invariant Has Teeth

```gherkin
Given the seeded DST harness with allocations churning Running and Stopped
When the platform reconciles repeatedly
Then eventually every Running allocation holds a valid SVID
And no stopped allocation holds an SVID
And a deliberately broken hold/drop implementation fails the invariant
```

Universe: running allocation set + held `BTreeMap` snapshot + SVID validity.

### S-WIM-WS — Walking Skeleton: Issue, Hold, Audit, Verify, Drop

```gherkin
Given a real integration-test control plane with the built-in CA booted
And an allocation reaches Running
When SvidLifecycle emits IssueSvid and the action shim dispatches it
Then IdentityMgr holds the minted SVID
And an issued_certificates row is observable for the issuance
And openssl verify accepts Root -> Intermediate -> SVID
When the allocation stops and DropSvid is dispatched
Then IdentityMgr no longer holds that allocation's SVID
```

Universe: real held map, real observation row, `openssl verify` exit code.

### S-WIM-12 — Restart Re-Issues Each Still-Running Alloc With Audit Row

```gherkin
Given a running allocation held an SVID before control-plane restart
When the control plane restarts and IdentityMgr starts empty
And SvidLifecycle is ticked with the still-Running desired set
Then the allocation is re-issued exactly once during recovery convergence
And the re-issued SVID is held
And the re-issue leaves an issued_certificates audit row
And the old leaf key was not persisted or reconstructed
```

Universe: post-restart held map + audit rows + `openssl verify` exit code.

## Adapter Coverage

| Driven adapter / port | Coverage |
|---|---|
| `Ca` / `issue_and_audit` | S-WIM-02, S-WIM-07, S-WIM-WS, S-WIM-12 |
| `ObservationStore` audit row | S-WIM-02, S-WIM-07, S-WIM-WS, S-WIM-12 |
| `IdentityMgr` | S-WIM-02, S-WIM-04, S-WIM-05, S-WIM-WS, S-WIM-12 |
| `IdentityRead` | S-WIM-04, S-WIM-05, S-WIM-06 |
| `SimIdentityRead` | S-WIM-06 |
| Reconciler runtime / ViewStore | S-WIM-08, S-WIM-12 |
| Workflow engine rotation boundary | S-WIM-09 |

## Test Placement

| Crate | Files | Rationale |
|---|---|---|
| `overdrive-core` | `tests/acceptance/svid_lifecycle_reconcile.rs` | Pure reconciler / core type-shape contracts live with core acceptance tests. |
| `overdrive-control-plane` | `tests/acceptance/{issue_svid_action_shim,identity_mgr_read_contract}.rs` | In-process control-plane action/read contracts. |
| `overdrive-sim` | `tests/acceptance/identity_read_equivalence.rs` | Sim double and DST invariant home. |
| `overdrive-control-plane` | `tests/integration/workload_identity_manager/lifecycle.rs` | Real stores + real CA + `openssl verify`, gated by `integration-tests`. |

## DELIVER Notes

- Do not introduce `.feature` files or Python step definitions.
- Replace `#[should_panic(expected = "RED scaffold")]` bodies with real assertions slice by slice.
- Keep #35 as a foundation feature: operator `alloc status` rendering of issued cert rows belongs to #215, blocked on #35.
- Treat ADR-0067 rev 2 as the authority over older DIVERGE wording about restart idempotence.

## Verification Catalogue

No new `verification/expectations/` entry is created for #35. This feature has
no new operator CLI surface; its test-tier walking skeleton unblocks existing
catalogue expectations:

- `verification/expectations/E03-ca-full-chain-verifies/` — full chain verifies
  under `openssl verify`; already marked unblocked by #35.
- `verification/expectations/O05-ca-issued-certificates-audit-row/` — issuance
  observable as an `issued_certificates` row; already marked unblocked by #35
  and by the later operator render.

In-process reconciler/read-port scenarios stay in the Rust test tiers and do not
graduate into EDD.

## DISTILL Review

- **Reviewer**: nw-acceptance-designer-reviewer (Opus)
- **Date**: 2026-06-08
- **Verdict**: NEEDS_REVISION → **APPROVED** (both blocking issues resolved 2026-06-08)
- **Mandates**: CM-A pass · CM-B pass · CM-C pass
- **Blocking issues (2) — both RESOLVED**:
  1. **issue (blocking)** — DWD-WIM-06 claimed three outcome registrations
     (`SvidLifecycle`, `IdentityRead`, running-set invariant) but
     `docs/product/outcomes/registry.yaml` contained only
     `OUT-WIM-SVID-LIFECYCLE` and `OUT-WIM-RUNNING-SET-INVARIANT`. The
     `IdentityRead` contract (US-WIM-02, ADR-0067 D7, S-WIM-04/05/06) was
     unregistered. **Resolved**: added `OUT-WIM-IDENTITY-READ` to the registry
     pinning ADR-0067 D7's five clauses; DWD-WIM-06's claim is now accurate.
  2. **issue (blocking)** — DWD-WIM-05 stated the error-path ratio was
     "5 of 13 (38%)", but exactly 4 scenarios carry `@error` (S-WIM-05/07/09/12;
     4/13=31%) while the prose enumerates 6 negative scenarios (6/13=46%).
     Neither count was 5. **Resolved**: restated as 6 of 13 (46%) with the
     `@error`/`@property` split made explicit, scenario IDs cited inline.
- **Non-blocking**:
  - nitpick — DIVERGE `wave-decisions.md` D-WIM-8 retains superseded rev-1
    View-persists-issuance-input wording; covered by the Reconciliation
    "historical record" policy but worth a "superseded by ADR-0067 rev-2 D8"
    annotation.
  - question — S-WIM-WS dual-`When` is the accepted Tier-A lifecycle E2E shape;
    keep it as the single demo-able journey through DELIVER.
- **Scores**: happy_path_bias 9 · gwt_format 9 · business_language 10 ·
  coverage 9 · walking_skeleton_centricity 9 · observable_behavior 10 ·
  traceability 6 · walking_skeleton_boundary 9
- **Strengths**: exact scenario↔scaffold↔entrypoint wiring (inline-`mod`
  convention honored in all 4 touched entrypoints; L3 tests gated behind
  `integration-tests` under `tests/integration/`); correct RED convention
  (`#[should_panic(expected = "RED scaffold")]` + scenario-naming panic, no
  bare panics / neutral stubs); consistent rev-1→rev-2 restart-model
  reconciliation (S-WIM-08/09/12 + J-SEC-002 journey patch coherent);
  complete trait-observable driving-port coverage (Dimension 7 clean on all
  13); 46% negative/guardrail coverage.
