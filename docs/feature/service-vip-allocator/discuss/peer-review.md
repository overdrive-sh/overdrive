# Peer Review — service-vip-allocator (DISCUSS)

**Reviewer**: nw-product-owner (review mode)
**Date**: 2026-05-14
**Iteration**: 1
**Artifacts under review**:

- `docs/feature/service-vip-allocator/discuss/user-stories.md`
- `docs/feature/service-vip-allocator/discuss/wave-decisions.md`
- `docs/feature/service-vip-allocator/discuss/outcome-kpis.md`
- `docs/feature/service-vip-allocator/discuss/story-map.md`
- `docs/feature/service-vip-allocator/discuss/dor-validation.md`

```yaml
review_id: "req_rev_20260514_111500"
reviewer: "nw-product-owner (review mode)"
artifact: "docs/feature/service-vip-allocator/discuss/*"
iteration: 1

strengths:
  - "SSOT discipline: every AC traces back to GH #167 with explicit citation, and the two pinned-VIP items dropped per the user-directed scope refinement are documented under § Changed Assumptions with verbatim quotes from upstream slice-06."
  - "Solution-neutral ACs: AC-06 explicitly defers the rejection layer (parser vs. admission) to DESIGN; no AC names traits, modules, or persistence shapes. The 5 open questions for DESIGN are crisply scoped."
  - "Concrete domain examples: Maya Okonkwo + Diego Hernández + frontend.toml + 10.96.42.17 + port 8080 + tcp — no generic placeholders anywhere in the 5 examples or 5 UAT scenarios."
  - "Back-propagation discipline: the upstream slice-06 brief is NOT modified directly; the Changed Assumption is captured in this feature's wave-decisions.md with the verbatim original-assumption quote and rationale."
  - "Right-sized: 1 story, 1 bounded context, 5 UAT scenarios, 1–3 days — well within carpaccio bounds. Walking skeleton correctly skipped (brownfield)."

issues_identified:
  confirmation_bias:
    - issue: "Happy-path bias check"
      severity: "low"
      location: "user-stories.md § US-01 / UAT Scenarios"
      recommendation: "5 scenarios cover: 1 happy, 1 idempotency, 3 error/boundary (operator-supplied vip, terminal-state reclamation, pool exhaustion). Error/boundary outnumber happy path 3:2. PASS — no happy-path bias."
    - issue: "Technology bias check"
      severity: "low"
      location: "user-stories.md, wave-decisions.md"
      recommendation: "Technical Notes mentions BackendIdAllocator at file:line and the dataplane crate as the home, but only as context for DESIGN. AC list is solution-neutral. PASS — technology context is informational, not constraining."
    - issue: "Availability bias check"
      severity: "low"
      location: "wave-decisions.md § D3"
      recommendation: "Decision D3 (allocator in dataplane crate) is justified by an existing primitive's location, not by 'we always put allocators there'. The placement is grounded in BackendIdAllocator's existing residence. PASS."

  completeness_gaps:
    - issue: "Stakeholder coverage check"
      severity: "low"
      location: "user-stories.md § US-01 / Who"
      recommendation: "Single operator persona (platform engineer). No multi-stakeholder concern for a backend primitive at Phase 1 single-node scope. Compliance / security / multi-tenant stakeholders are out-of-scope per Phase 1 framing. PASS."
    - issue: "Error scenario coverage"
      severity: "low"
      location: "user-stories.md § US-01 / UAT Scenarios 3, 4, 5"
      recommendation: "Three error / boundary scenarios cover: invalid input (operator-supplied vip), lifecycle (terminal-state release), resource exhaustion (pool exhaustion). Authentication / network timeout / concurrent modification are out-of-scope for a single-node primitive. PASS for in-scope error classes."
    - issue: "NFR coverage"
      severity: "low"
      location: "outcome-kpis.md"
      recommendation: "Performance (K2 latency), reliability (K1 success rate), guardrails (K3 reclamation lag, K4 pool exhaustion). Security / compliance / accessibility are out-of-scope for a Phase 1 single-node backend primitive. PASS."

  clarity_issues:
    - issue: "Vague performance threshold check"
      severity: "low"
      location: "outcome-kpis.md K2, K3"
      recommendation: "K2 (p50 ≤ 5 ms / p99 ≤ 25 ms) and K3 (p50 ≤ 1 s / p99 ≤ 5 s) carry quantitative thresholds. No 'fast' / 'low-latency' qualitative language. PASS."
    - issue: "Ambiguity check on AC-06"
      severity: "low"
      location: "user-stories.md AC-06"
      recommendation: "AC-06 explicitly defers the rejection layer to DESIGN. This is intentional ambiguity scoped to layer choice; the *observable outcome* (rejection, named guidance, no state mutation) is fully pinned. Reviewer-acceptable per skill principle 5 (problem-first, solution-never)."

  testability_concerns:
    - issue: "Every AC observable / testable?"
      severity: "low"
      location: "user-stories.md AC-01 through AC-06"
      recommendation: "Each AC names an observable outcome with a verification method implicit in the corresponding UAT scenario. AC-05 (shared primitive lives in overdrive-dataplane) is testable by inspecting the crate's module structure; AC-06's no-state-mutation clause is testable by post-rejection inspection of allocator + IntentStore state. PASS."

  priority_validation:
    q1_largest_bottleneck: "YES"
    q2_simple_alternatives: "ADEQUATE"
    q3_constraint_prioritization: "CORRECT"
    q4_data_justified: "JUSTIFIED"
    verdict: "PASS"

  priority_validation_notes:
    q1: "The largest gap: upstream slice-06 landed the spec shape with vip = Option<ServiceVip> and explicitly deferred the runtime allocator to this feature. Without this feature, operators cannot submit Service specs without supplying a vip — slice-06's pending-allocation render is a dead end. PASS."
    q2: "Simpler alternative considered: operator-pinned VIPs with admission validation (slice-06's R6.1 original path). User explicitly chose platform-issued-only on 2026-05-14 with documented rationale (single source of authority, collapses conflict resolution surface). Alternative is rejected with evidence. PASS."
    q3: "Constraint prioritization: phase-1-single-node is the dominant constraint and it correctly drives both the open-questions parking (multi-node Raft deferred to Phase 5+) and the no-cross-node-consensus scope-out. PASS."
    q4: "Data-justified: the existing BackendIdAllocator at allocator.rs:31 is the structural precedent with measured collision-witness coverage (allocator.rs:125-138). The shared-primitive refactor is grounded in observed allocator API surface. PASS."

approval_status: "approved"
critical_issues_count: 0
high_issues_count: 0
medium_issues_count: 0
low_issues_count: 0
```

## Reviewer summary

All five DISCUSS artifacts pass review. No critical / high / medium
issues. The story is right-sized; ACs are solution-neutral; the
Changed Assumption is properly back-propagated; the open questions
are scoped to DESIGN; the outcome KPIs follow the Gothelf/Seiden
formula with measurable targets, baselines, and a smell-test
verification.

**Verdict**: APPROVED for DESIGN-wave handoff (`@nw-solution-architect`)
and DEVOPS-wave handoff (`@nw-platform-architect`, `outcome-kpis.md`
only). No iteration 2 required.

## Cross-references

- DoR validation: `dor-validation.md` — 9/9 PASS.
- SSOT: [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).
- Upstream slice: `docs/feature/workload-kind-discriminator/slices/slice-06-service-listener-fields.md`.
