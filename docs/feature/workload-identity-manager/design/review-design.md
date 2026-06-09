# DESIGN Wave Review — workload-identity-manager

**Reviewer**: `nw-solution-architect-reviewer` stance  
**Date**: 2026-06-08  
**Iteration**: rev 2 follow-up  
**Verdict**: **CONDITIONALLY APPROVED — architecture accepted; stale handoff wording must be cleaned before DISTILL / DELIVER**

**Scope**: ADR-0067 rev 2, `docs/product/architecture/brief.md` workload-identity-manager extension, `docs/product/architecture/c4-diagrams.md` workload identity diagrams, `feature-delta.md` DESIGN sections, product SSOT `jobs.yaml`, and slice briefs as implementation handoff context.

## Verdict

ADR-0067 rev 2 resolves the prior blocking architecture defects:

- Restart recovery no longer claims the in-memory leaf key can be reconstructed. It uses bounded, audited re-issue on boot for every still-Running allocation.
- The held set is the reconciler `actual`; the View is retry memory, not issuance success facts.
- `SvidLifecycle` now has explicit `EnqueueEvaluation` handoff from workload lifecycle and the exit observer.
- The duplicate SPIFFE derivation is now framed as a `SpiffeId::for_allocation` extraction, with call-site migration required.
- Slice briefs now point DELIVER at ADR-0067 rev 2 instead of leaving the old "DESIGN call" decisions open.

The architecture is ready in substance, but the artifact set is not fully clean. A few handoff-facing sections still contain stale rev-1 wording around "issuance inputs", `issued_at`, and pending PO updates. Those are documentation defects, not design blockers, but they should be fixed before DISTILL / DELIVER so implementers follow one model.

### Blocking-issue count by category

| Category | Issues |
|---|---:|
| Critical | 0 |
| High | 1 |
| Medium | 2 |
| Low | 1 |

## Findings

### High: feature-delta still says the View carries `issued_at` / issuance inputs

**Dimension**: Cross-artifact consistency / Implementation feasibility  
**Location**: `docs/feature/workload-identity-manager/feature-delta.md:251`, `:1125`, `:1194`; `docs/product/architecture/c4-diagrams.md:1005`

ADR-0067 rev 2 is clear that `SvidLifecycleView` is retry memory only: `IssueRetry { attempts, last_failure_seen_at }`. But `feature-delta.md` still says the View persists "issuance inputs", and the DDD-10 bullet says "The View carries `issued_at` now for the future recompute." The C4 L2 relation also says the runtime write-throughs "issuance inputs."

That wording reintroduces the exact rev-1 model the review rejected: a View containing issuance/recompute facts. It is especially risky because these sections are implementation handoff material.

**Recommendation**: Replace every remaining "issuance inputs" phrase in #35 handoff sections with "retry memory" or "retry request inputs". Delete the `issued_at` sentence at `feature-delta.md:1125` and replace it with "near-expiry reads the held cert's real `not_after` from `actual`; the View carries no rotation/recompute timestamp."

### Medium: upstream-change notes still say product SSOT updates are pending after they were applied

**Dimension**: Process consistency / Handoff clarity  
**Location**: `docs/feature/workload-identity-manager/design/upstream-changes.md:125`, `:130-136`; `docs/feature/workload-identity-manager/feature-delta.md:1332-1336`, `:1367-1369`; `docs/product/jobs.yaml:309-312`; `docs/feature/workload-identity-manager/feature-delta.md:105`, `:675`

The product SSOT and feature KPI rows already carry the rev-2 wording: bounded, audited restart re-issue with no stale/silent credential. But `design/upstream-changes.md` and the DESIGN handoff still say those rows await PO/orchestrator update.

This does not invalidate the design, but it leaves the artifact status ambiguous: a downstream agent could spend time chasing an already-applied upstream change.

**Recommendation**: Mark `design/upstream-changes.md` as "applied" with the target file/line references, and change the feature-delta handoff from "need the PO's wording update" to "were updated; upstream-changes records the back-propagation."

### Medium: `jobs.yaml` still uses "idempotent across control-plane restarts" without the rev-2 qualifier

**Dimension**: Product/architecture consistency  
**Location**: `docs/product/jobs.yaml:324-325`; contrast `docs/product/jobs.yaml:309-312`

The ODI outcome text is corrected, but the functional dimension still says "Re-issuance is idempotent across control-plane restarts." Given the rev-1 history, "idempotent" is ambiguous: it can be read as "no redundant re-issue", which is explicitly impossible because the leaf key is not persisted.

**Recommendation**: Reword to the rev-2 model, for example: "Restart recovery is bounded and audited: after a control-plane restart, each still-Running allocation is re-issued at most once per recovery convergence, and every re-issue leaves an `issued_certificates` row."

### Low: leaf-key zeroization is labelled as a future hardening / DESIGN call without a tracking owner

**Dimension**: Deferral discipline  
**Location**: `docs/product/architecture/brief.md:4831-4836`, `:4854`; compare ADR-0067 `Open Questions` framing

ADR-0067 frames key zeroization as an accepted residual risk outside #35, not a live design question. The brief still calls it "DESIGN call / future hardening." That phrasing makes it look unresolved.

**Recommendation**: Either call it "accepted residual risk for #35; no zeroization beyond map removal" or create/link a hardening issue if the team intends to track it. Do not leave it as "DESIGN call" after DESIGN is complete.

## Approval Status

`conditionally_approved`

The core architecture is approved for DISTILL once the stale handoff wording above is corrected. No new architecture iteration is needed unless the cleanup changes the rev-2 decisions.
