# DISCUSS Wave Review - backend-instance-replacement

**Reviewer**: Codex, applying `nw-product-owner-reviewer` / DISCUSS hard-gate criteria
**Date**: 2026-06-29
**Verdict**: **CONDITIONALLY_APPROVED for DESIGN handoff**
**Scope**: `docs/feature/backend-instance-replacement/{feature-delta.md,slices/*.md}`, plus product SSOT updates in `docs/product/jobs.yaml`, `docs/product/journeys/dial-a-mesh-peer-by-name.yaml`, and `docs/product/personas/ana-platform-engineer.yaml`.

The DISCUSS artifacts are coherent enough to hand to DESIGN. The problem is grounded in the existing stop-sentinel / lifecycle-reconciler behavior, the extend-vs-mint job decision is defensible, the deferrals are tracked, and the terminal oracle correctly names all three #249-blocked Tier-3 acceptance tests.

This approval is conditional: DESIGN must close `[D1]` before any DISTILL or DELIVER work writes executable acceptance against the new operation.

---

## Findings

### 1. The user-invocable command surface is intentionally open, so it must remain a hard DESIGN gate

**Severity**: high for DISTILL/DELIVER; non-blocking for DESIGN
**Dimension**: elevator-pitch test / testability / handoff readiness

The artifacts correctly decide the important DISCUSS-level mechanism: use an explicit lifecycle verb and keep `overdrive deploy` pure-declare. They deliberately leave the exact verb name, HTTP path, response shape, and restart-vs-resume semantics to DESIGN.

Evidence:

- `feature-delta.md:150-190` records `[D1]`: explicit lifecycle verb decided; exact verb shape and semantics open.
- `feature-delta.md:192-196` explicitly says this is a hard DESIGN gate before executable acceptance.
- `feature-delta.md:223` and `feature-delta.md:278` use "replace action" with provisional examples rather than a final command.
- `slices/slice-01-replace-action-new-instance-intent-retained.md:31-37` and `slices/slice-04-unignore-tier3-oracle.md:42-45` depend on the same design-open action.

This is acceptable for DESIGN handoff because the open item is named, bounded, and marked as a gate. It would be a blocker for DISTILL/DELIVER if left unresolved: acceptance tests need a real user-invocable entry point and observable response.

**Recommendation**: DESIGN must close `[D1]` first and record the chosen CLI verb, HTTP endpoint, output/error shape, and stopped-vs-running semantics in an ADR before acceptance scenarios are made executable.

### 2. DoR summary has a stale "all three stories" statement after US-BIR-3 was removed

**Severity**: medium
**Dimension**: cross-artifact consistency / handoff hygiene

The artifact correctly refolds the old US-BIR-3 test-run story into a terminal verification gate, leaving two user stories plus one oracle gate. One DoR paragraph still says "all three stories."

Evidence:

- `feature-delta.md:321-329` says the three-test oracle is not a user story.
- `feature-delta.md:509-510` says there are two stories plus a terminal verification gate.
- `feature-delta.md:567-569` changelog records that US-BIR-3 was removed as a user story.
- `feature-delta.md:490-491` still says "DoR is met for all three stories."

This is not a substantive requirements conflict, but it should be corrected before the next wave so downstream agents do not resurrect US-BIR-3 as a story.

**Recommendation**: change the DoR verdict to "DoR is met for both stories" and, if useful, "the terminal verification gate is tracked separately."

### 3. Ana's persona SSOT is only partially amended for lifecycle replacement

**Severity**: medium
**Dimension**: persona specificity / shared artifact consistency

The feature-local persona framing is strong, and the persona file header now mentions lifecycle replacement. The structured persona body still mostly describes UDP service reachability, with role, entry points, triggers, motivations, frustrations, and success signals all UDP-shaped.

Evidence:

- `feature-delta.md:28-50` clearly frames Ana's lifecycle/ops lens for this feature.
- `docs/product/personas/ana-platform-engineer.yaml:3-26` adds a lifecycle-replacement header note.
- `docs/product/personas/ana-platform-engineer.yaml:31-72` still describes Ana primarily as running UDP-bearing services and diagnosing UDP reachability.
- `docs/product/personas/ana-platform-engineer.yaml:94-103` still lists only UDP/dataplane reachability frustrations and success signals.

This does not block DESIGN because the feature delta carries the lifecycle-specific persona context. It is a traceability weakness in the product SSOT.

**Recommendation**: add one lifecycle-replacement trigger, frustration, motivation, and success signal to the structured persona body so future readers do not have to rely on the comment header plus feature-local prose.

---

## Checks Passed

### Requirements quality

- **Job traceability**: PASS. Extending `J-OPS-003` is well justified as the same convergence job at finer granularity (`feature-delta.md:63-92`; `docs/product/jobs.yaml:176-208`).
- **Problem grounding**: PASS. The stop sentinel, `put_if_absent`, `is_operator_stopped`, and crash-restart/delete distinctions are explicitly documented (`feature-delta.md:125-146`).
- **Error coverage**: PASS. No-such-workload is covered as a not-found case (`feature-delta.md:239`, `feature-delta.md:258-262`, `docs/product/journeys/dial-a-mesh-peer-by-name.yaml:164-171`).
- **NFRs / constraints**: PASS for this stage. Single-node scope, no deletion, new allocation identity, stable `F`, TOCTOU-safe clearing, no `sock_destroy`, and production-entry-point testing are explicit (`feature-delta.md:370-380`).
- **Testability**: PASS with the `[D1]` condition. Acceptance is observable through new AllocationId, new `workload_addr`, retained `jobs/<id>`, byte-stable `F`, bounded churn failure, and three named Tier-3 ATs (`feature-delta.md:265-319`, `feature-delta.md:331-345`).
- **Shared artifacts**: PASS. Registry-grade rows exist for the stop sentinel, job intent row, allocation ID, workload address, stable `F`, and Tier-3 oracle (`feature-delta.md:436-449`).
- **Outcome KPIs**: PASS. K-BIR-1..4 have numeric targets, baselines, and measurement methods (`feature-delta.md:452-472`).
- **Deferral discipline**: PASS. Rolling/zero-downtime and multi-replica semantics are out of v1 and tracked as #253 and #254 (`feature-delta.md:393-402`, `feature-delta.md:533-536`).

### Cross-artifact consistency

- `docs/product/jobs.yaml` is consistent with the feature's J-OPS-003 extension.
- `docs/product/journeys/dial-a-mesh-peer-by-name.yaml` includes the step-4 actor handoff, lifecycle micro-arc, no-such-workload error path, related job, and related feature.
- `docs/product/personas/ana-platform-engineer.yaml` at least records Ana's lifecycle-replacement ownership in the header and `related_jobs`; see Finding 3 for the remaining body-level amendment.
- The compact `feature-delta.md` form is acceptable here because it explicitly declares itself the authoritative DISCUSS artifact and includes the usual split-file content: stories, story map/slices, registry, KPIs, DoR, and wave decisions.

### Verification performed

- Parsed touched product YAML successfully:
  - `docs/product/jobs.yaml`
  - `docs/product/journeys/dial-a-mesh-peer-by-name.yaml`
  - `docs/product/personas/ana-platform-engineer.yaml`
- Verified GitHub issue references:
  - #249 open: backend instance replacement / restart-after-stop.
  - #253 open: zero-downtime backend-instance replacement.
  - #254 open: multi-replica backend-instance replacement semantics.
- Verified all three oracle tests exist and are currently `#[ignore]`'d to #249:
  - `answered_frontend_is_byte_stable_across_alloc_cycle_next_connect_lands_new_backend`
  - `in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend`
  - `recovered_job_after_stop_resolves_to_the_same_stable_frontend`

---

## Handoff Status

**Cleared for DESIGN with conditions.**

DESIGN should first close `[D1]`: choose the command/API surface, response/error shape, stopped-vs-running semantics, and TOCTOU-safe sentinel-clearing mechanism. After that, DISTILL/DELIVER can safely convert the current outcome-shaped scenarios into executable acceptance tests and un-ignore the three-test oracle.
