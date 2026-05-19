# Definition of Ready — service-vip-allocator

**Story under audit**: US-01 (the sole story).
**Date**: 2026-05-14.
**Auditor**: Luna (nw-product-owner).

The 9-item DoR checklist below maps to the skill's hard-gate (§ DoR
Checklist (9-Item Hard Gate)). Each item carries evidence with a
file:line citation and a verdict.

---

### Item 1 — Problem statement clear, domain language

**Required**: Problem stated in domain language; not a technical
restatement; identifies a real persona and a real pain.

**Evidence**: `user-stories.md` § US-01 / Problem (Maya Okonkwo,
Overdrive platform engineer, dev-host single-node, friction = having
to pick a dataplane address). Domain terms: "Service spec", "VIP",
"submit", "dataplane address". No technical-shape language ("alloc",
"trait", "module", "atomic counter") in the problem statement itself.

**Verdict**: **PASS**.

---

### Item 2 — User/persona with specific characteristics

**Required**: Persona named, role specified, context concrete enough
to design for.

**Evidence**: `user-stories.md` § US-01 / Who:

- Persona: Maya Okonkwo (and Diego Hernández in error-case examples).
- Type: Overdrive platform engineer (operator persona).
- Context: Local single-node control plane on dev host.
- Motivation: J-OPS-002 + J-OPS-003 from `docs/product/jobs.yaml`.

Both personas appear with concrete actions and concrete data (specific
file names, port numbers, IP addresses) in the Domain Examples.

**Verdict**: **PASS**.

---

### Item 3 — 3+ domain examples with real data

**Required**: At least 3 concrete examples. Real names (not "user123"
/ "test@test.com"). Real data (specific values, not placeholders).

**Evidence**: `user-stories.md` § US-01 / Domain Examples — 5
examples:

1. Happy path (Maya, `frontend.toml`, `10.96.42.17`, port 8080, tcp).
2. Idempotency (Maya, byte-identical resubmit, same VIP).
3. Operator-supplied VIP rejected (Diego, pinned VIP in spec).
4. Reclamation on terminal state (Maya, stop command, pool recycle).
5. Pool exhaustion (Diego, 257th submission against 256-pool).

All examples use real persona names (Maya Okonkwo, Diego Hernández),
real file names (`frontend.toml`), real IP addresses
(`10.96.42.17`), real port numbers (8080), real protocol
identifiers (`tcp`). No generic placeholders (`user1`, `test@test.com`,
`192.168.0.1`).

**Verdict**: **PASS** (exceeds minimum; 5 examples covering happy
path, idempotency, three error / boundary cases).

---

### Item 4 — UAT in Given/When/Then (3–7 scenarios)

**Required**: 3–7 scenarios in Gherkin form. Scenario titles describe
business outcomes, NOT implementation. No class names, no protocol
detail in titles.

**Evidence**: `user-stories.md` § US-01 / UAT Scenarios — 5
scenarios in Gherkin form:

1. "Operator submits a Service spec without supplying a VIP, platform
   allocates one"
2. "Resubmitting the same spec returns the same VIP idempotently"
3. "Operator-supplied `vip` is rejected with named guidance"
4. "Terminal-state transition releases the VIP for reuse"
5. "Pool exhaustion produces a typed rejection"

Scenario count: 5 (within 3–7 band). All titles describe operator-
observable outcomes; none name classes, modules, traits, or wire
formats. Each scenario uses concrete persona + concrete data in the
Given / When / Then bodies.

**Verdict**: **PASS**.

---

### Item 5 — AC derived from UAT

**Required**: Acceptance criteria traceable 1:1 (or N:1) to UAT
scenarios. Each AC is observable, testable, named.

**Evidence**: `user-stories.md` § US-01 / Acceptance Criteria — 6
ACs (AC-01 … AC-06). Traceability:

- AC-01 ← Scenario 1 + Scenario 5's first-256 baseline.
- AC-02 ← Scenario 2.
- AC-03 ← Scenario 4.
- AC-04 ← Scenario 5.
- AC-05 ← Domain Example 5's underlying constraint + #167 AC 6
  (cross-reference to GitHub SSOT).
- AC-06 ← Scenario 3.

Each AC names an observable outcome (not an implementation
mechanism). AC-06 explicitly defers the rejection layer (parser vs.
admission) to DESIGN; the AC describes the *observable* outcome
(rejection with named guidance, no state mutation), which is the
correct abstraction level for DISCUSS.

**Verdict**: **PASS**.

---

### Item 6 — Right-sized (1–3 days, 3–7 scenarios)

**Required**: Story estimated at 1–3 days; UAT count 3–7.

**Evidence**:

- UAT count: 5 (within band).
- Effort estimate: 1–3 days per `story-map.md` § Scope Assessment
  ("1–3 days, depending on DESIGN's resolution of Open Questions 1,
  2, 4"). The single-story scope refactors an existing isolated
  primitive (`BackendIdAllocator` at `crates/overdrive-dataplane/src/
  allocator.rs:31`) and adds a second consumer; the test surface is
  already in place (proptest at `allocator.rs:92-110`, collision
  witness at `:125-138`).
- No oversizing signals per `story-map.md` § Scope Assessment (1
  bounded context, 1 story, brownfield refactor, no walking skeleton
  needed).

**Verdict**: **PASS**.

---

### Item 7 — Technical notes: constraints/dependencies

**Required**: Constraints, dependencies, technical context that
DESIGN needs to know but DISCUSS does not resolve.

**Evidence**: `user-stories.md` § US-01 / Technical Notes — 5
bullets covering:

- Existing `BackendIdAllocator` precedent with file:line citation.
- Spec digest hint toward submit-time allocation, deferred to DESIGN.
- Reclamation trigger deferred to DESIGN.
- Pool config shape deferred to DESIGN.
- Upstream slice-06 field shape preserved.
- Cross-references to all related GH issues (#167, #164, #61, #163).

Plus `wave-decisions.md` § System Constraints (Phase 1 single-node;
platform-issued only; allocator in `overdrive-dataplane/`; solution-
neutral).

**Verdict**: **PASS**.

---

### Item 8 — Dependencies resolved or tracked

**Required**: Every dependency on other features / issues / decisions
is named and either resolved or tracked with explicit status.

**Evidence**: `wave-decisions.md`:

- Upstream Slice 06 of `workload-kind-discriminator` — landed; field
  shape preserved; Changed Assumption documented (back-propagation
  note).
- GH #167 — SSOT, open, this feature's umbrella.
- GH #164 — downstream wiring of `Dataplane::update_service`, out
  of scope per #167.
- GH #61 — pool / range definition, out of scope per #167.
- GH #163 — referenced in #167, out of scope.
- GH #175 — `client_iface` / `backend_iface` config, named as
  context for Open Question 3.
- Open questions 1–5 — DESIGN-wave decisions, parked in
  `wave-decisions.md` § Open questions for DESIGN.

All five open questions are explicitly DESIGN-wave concerns within
#167's umbrella; no new GitHub issues required (per task framing
and `.claude/rules/development.md` § "Deferrals require GitHub
issues — AND user approval BEFORE creation").

**Verdict**: **PASS**.

---

### Item 9 — Outcome KPIs defined with measurable targets

**Required**: KPIs follow the Outcome formula (Who / Does what / By
how much / Measured by / Baseline). Measurable targets named.

**Evidence**: `outcome-kpis.md` — 4 KPIs (K1–K4) with:

- K1 (success rate, 100% target, baseline 0%, measurement defined).
- K2 (latency p50 ≤ 5 ms / p99 ≤ 25 ms, new code path).
- K3 (reclamation lag p50 ≤ 1 s / p99 ≤ 5 s, guardrail).
- K4 (pool exhaustion 0 per 24-hour window under nominal load,
  guardrail; utilisation gauge).

North star (K1), leading indicators (K2/K3), guardrails (K3/K4) all
specified. Measurement plan, alerting thresholds, DEVOPS handoff
items all present. Smell-test verification table confirms each KPI
is a rate/distribution (not gross count), an outcome (not output),
and team-influenceable.

**Verdict**: **PASS**.

---

## Summary

| # | Item | Verdict |
|---|---|---|
| 1 | Problem statement clear, domain language | PASS |
| 2 | User/persona with specific characteristics | PASS |
| 3 | 3+ domain examples with real data | PASS |
| 4 | UAT in Given/When/Then (3–7 scenarios) | PASS |
| 5 | AC derived from UAT | PASS |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | PASS |
| 7 | Technical notes: constraints/dependencies | PASS |
| 8 | Dependencies resolved or tracked | PASS |
| 9 | Outcome KPIs defined with measurable targets | PASS |

**Overall**: **9/9 PASS**. Story US-01 is ready for DESIGN-wave
handoff to `@nw-solution-architect`.

## Anti-Pattern Sweep

Per `nw-leanux-methodology` § Anti-Pattern Detection, swept the
artifacts:

| Anti-Pattern | Status |
|---|---|
| Implement-X / "Add feature" language | Absent — problem stated from operator pain (Maya's friction). |
| Generic data (`user123`, `test@test.com`) | Absent — Maya Okonkwo, Diego Hernández, `frontend.toml`, `10.96.42.17`, port 8080, tcp. |
| Technical AC ("Use JWT tokens" shape) | Absent — every AC is observable / outcome-shaped. AC-06 explicitly defers the layer choice. |
| Technical scenario title | Absent — all 5 scenario titles describe operator-observable outcomes. |
| Oversized story (>7 scenarios, >3 days) | Absent — 5 scenarios, 1–3 days. |
| Abstract requirements (no concrete examples) | Absent — 5 concrete examples. |

No anti-patterns detected.

## Handoff readiness

DISCUSS wave artifacts are ready for:

- DESIGN wave (`@nw-solution-architect`): all 5 documents in
  `docs/feature/service-vip-allocator/discuss/` plus the SSOT issue
  #167 and the upstream slice-06 brief.
- DEVOPS wave (`@nw-platform-architect`): `outcome-kpis.md` only,
  for instrumentation planning of K1–K4.

The DESIGN-wave open questions (5 total) are parked in
`wave-decisions.md`; the architect resolves each before DELIVER.
