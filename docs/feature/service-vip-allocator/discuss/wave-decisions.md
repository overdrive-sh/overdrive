# Wave Decisions — service-vip-allocator (DISCUSS)

**Wave**: DISCUSS
**Feature**: service-vip-allocator
**Date**: 2026-05-14
**Author**: Luna (nw-product-owner)

## Configuration captured at dispatch

| Field | Value |
|---|---|
| feature_id | `service-vip-allocator` |
| feature_type | backend (control-plane/dataplane primitive) |
| walking_skeleton | No (brownfield refactor of existing `BackendIdAllocator`) |
| ux_research | lightweight (no end-user UX surface) |
| jtbd_analysis | SKIPPED — covered by J-OPS-002 / J-OPS-003 in `docs/product/jobs.yaml`; backend primitive |
| journey_design | SKIPPED — no end-user journey for a backend primitive |
| output_directory | `docs/feature/service-vip-allocator/discuss/` |

## Entry context (no DISCOVER / DIVERGE waves)

This feature originates from a tracked follow-up registered during the
`workload-kind-discriminator` DISCUSS wave on 2026-05-09 and recorded
in [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).

The SSOT for this feature is **GitHub issue #167**. Its body enumerates:

- Context (originated in `workload-kind-discriminator` DISCUSS, 2026-05-09)
- Scope (in / out)
- Acceptance criteria (six bullets — two of which are dropped per the
  user-directed scope refinement below)
- References (GH #164, #61, #163; upstream slice-06)
- Definition of done

DISCOVER and DIVERGE are intentionally absent — this is a closed,
already-validated primitive with a single direction (refactor an
existing dataplane allocator into a generalised module that grows a
second consumer for Service VIPs).

## JTBD traceability

| Job ID | Title | Relevance |
|---|---|---|
| **J-OPS-002** | Submit a job to the walking-skeleton control plane and trust what the CLI tells me | **PRIMARY** — when an operator submits a Service spec without supplying a VIP, the platform must allocate one transparently and persist the assignment idempotently. "Trust what the CLI tells me" requires that the VIP is materialised before the operator's submit echo references it. |
| **J-OPS-003** | Run my actual workload on the walking-skeleton control plane and trust the platform to converge to the declared replica count | **SECONDARY** — VIP reclamation on terminal-state transition is a convergence concern (allocator pool must not silently leak under steady-state workload churn). |

No new job statement is added. Per the user's framing ("backend
primitive; motivations obvious from #167"), JTBD analysis is skipped.

## Changed Assumptions vs. upstream Slice 06 of `workload-kind-discriminator`

**Upstream artifact**:
`docs/feature/workload-kind-discriminator/slices/slice-06-service-listener-fields.md`,
lines 32–34.

**Quoted original assumption** (verbatim):

> `vip: Option<ServiceVip>` — `ServiceVip` is a thin newtype over
> `Ipv4Addr`; absent value is `None`. Validation is IPv4 syntactic only at
> this layer.

The downstream implication recorded at `slice-06-service-listener-fields.md`
line 132–137 was that `Some(vip) = operator-pinned valid`, with `None`
representing pending allocation:

> If #167 (VIP allocator) lands with a different field shape than
> `Option<ServiceVip>`, downstream rework is needed. **Mitigation**: the
> `Option`-shaped field is forward-compatible with both decisions ("reject at
> admission" → `None` is a parser error in a future ADR; "allocate at
> runtime" → `None` is the trigger for the allocator).

**New assumption (locked by this DISCUSS wave per user direction
2026-05-14)**:

VIPs are **platform-issued only**. Operators cannot supply `vip = Some(...)`
in a Service `[[listener]]` block. The parser or admission layer (DESIGN
decides which) rejects operator-supplied `vip` with named guidance.
Internally the spec field stays `Option<ServiceVip>` for forward
compatibility with the streaming/render layer (where the literal "pending
allocation" still renders before the allocator runs), but on the input
boundary `Some(addr)` is structurally rejected.

**Rationale**:

1. Single source of authority — the platform owns the VIP pool, so
   allowing operator-pinned VIPs introduces a conflict-resolution surface
   ("operator pinned X, allocator wants to assign X to another service")
   that this feature would otherwise have to support. Dropping
   operator-pinned VIPs collapses the design space.
2. Eliminates two ACs from #167 ("Conflict with operator-pinned VIPs"
   scope-in; "Operator-pinned VIPs in the allocator's pool are reserved
   at startup so they cannot be doubly allocated"). The remaining ACs
   are coherent under platform-issued-only.
3. Aligns with the broader "operators describe intent, platform owns
   mechanism" pattern already established in Phase 1 (operators do not
   pin allocation IDs, BackendIds, or node IDs).

**Back-propagation handling**: this change is **NOT** applied directly
to `slice-06-service-listener-fields.md` per the task framing and the
project's deferral discipline. The upstream slice's `Option`-shape
field stays as-is; the new admission-rejection rule for operator-
supplied `vip = Some(...)` is a runtime/admission concern that lands
with the service-vip-allocator feature. The slice's R6.1 mitigation
language ("'reject at admission' → `None` is a parser error in a future
ADR") still holds — this DISCUSS wave is the future ADR's predecessor.

**Amendment 2026-05-14 (post-DESIGN)**: DESIGN resolved Q5 to
**parser-level removal of the `Listener.vip` field** rather than
admission-level rejection. The framing above is intentionally
preserved as the audit trail of DISCUSS's original layer-agnostic
deferral. The authoritative back-propagation record is
`docs/feature/service-vip-allocator/design/upstream-changes.md`,
which documents the real spec-shape change to slice-06 (field
removal + uniqueness rule simplification + render shape + test
deletions + R6.1 mitigation moot). Readers consulting DISCUSS in
isolation should treat this section's mechanism details as
superseded by `design/upstream-changes.md`; the intent (platform-
issued only; operators cannot supply VIPs) is unchanged.

## Outcome-defining decisions captured

| # | Decision | Rationale |
|---|---|---|
| D1 | Single user story (operator persona, platform-issued VIP) | #167 is a thin backend primitive with one operator-visible outcome; multiple stories would manufacture optionality. |
| D2 | ACs lift verbatim from #167 minus two pinned-VIP items, plus one new AC covering operator-supplied `vip = Some(...)` rejection | Maintains traceability to the SSOT issue while honoring the user's scope refinement. |
| D3 | Allocator primitive lives in `crates/overdrive-dataplane/`, NOT control-plane | Per user direction 2026-05-14 ("if it lives in dataplane, then we just put it into dataplane and not control plane"). `BackendIdAllocator` already lives there at `crates/overdrive-dataplane/src/allocator.rs:31`. |
| D4 | Exact shared-primitive shape (trait vs. concrete generic) deferred to DESIGN | DISCUSS commits only to "matching allocator shape (memo + monotonic counter); no shared trait" — softened 2026-05-14 from "the underlying allocator logic is shared between BackendId and ServiceVip allocators" after the DELIVER step 01-01 rejection of the generic factoring (see ADR-0049 § Considered alternatives → Alt-0). The shape-similarity is enforced via parallel tests on each concrete type, not via a shared trait. |
| D5 | Skip journey YAML / Gherkin / JTBD artifacts | Backend primitive with no end-user journey; per `nw-leanux-methodology` § Example 4 (`--phase=requirements` shape). |
| D6 | Skip shared-artifacts-registry, story-map slicing, prioritization | Single story, single artifact (the VIP), single slice — these documents are no-ops at this scope. |
| D7 | Skip walking skeleton | Brownfield refactor of an isolated existing primitive; no new bounded context is introduced. |
| D8 | Phase 1 single-node scope | No cross-node consensus on allocations; multi-node coordination via IntentStore Raft is Phase 5+. Aligned with `feedback_phase1_single_node_scope.md`. |

## Out of scope (per #167)

- Opinionated default VIP ranges. The allocator is pool-agnostic and
  consumes whatever range operator config supplies. Phase 1 ships IPv4
  Service VIPs functional with an operator-configured range; no
  tracking issue is needed for "the IPv4 range" because there is no
  canonical platform-provided range to track.
- IPv6 ULA range (`fdc2::/16`), DNS naming
  (`<job>.svc.overdrive.local`), and auto-wake / scale-to-zero — all
  on GH #61 (Phase 4.11). #61 is **not a hard dependency** for Phase 1
  Service VIPs to be usable; it is a usability layer that adds IPv6
  addressing + name-based discovery + Phase 6.9 scale-to-zero on top
  of the functional IPv4 path #167 + #164 + #175 deliver.
- Dataplane wiring of the allocated VIP into `Dataplane::update_service`
  (#164 downstream chain).
- Cross-node allocation consensus (Phase 5+).

## Open questions for DESIGN

These are deliberately unresolved. DESIGN wave decides each.

1. **Reclamation trigger** — which reconciler / action shim emits the
   VIP release on terminal-state transition? Most likely `WorkloadLifecycle`,
   but DESIGN owns the call. The contract is "release fires exactly-once
   per allocated VIP on terminal-state entry, before the spec digest
   becomes eligible for reuse"; mechanism is open.

2. **When admission allocates** — submit-time (before IntentStore
   admission; the allocator owns the spec-digest → VIP mapping
   atomically with admission) vs. reconciler-tick (when a Service
   workload is first observed without an assigned VIP). Different
   failure surfaces. #167's AC names "spec digest" as the idempotency
   key, hinting submit-time. DESIGN owns the call.

3. **Pool config shape** — fold into the existing/forthcoming
   `[dataplane]` config block (alongside `client_iface` + `backend_iface`
   from GH #175) or a new `[vip_allocator]` section. Operator config
   format precedent is ADR-0019 (TOML).

4. **Shared allocator trait shape** — how to factor `BackendIdAllocator`
   (current, at `crates/overdrive-dataplane/src/allocator.rs:31`) and
   `ServiceVipAllocator` (new) into a reusable primitive under
   `crates/overdrive-dataplane/src/allocators/`. Trait-with-associated-
   types vs. concrete generic struct vs. macro-generated; persistence
   boundary; what stays per-consumer. DESIGN owns the call.

5. **Upstream slice-06 admission policy** — given platform-issued-only,
   does the parser reject `vip = Some(...)` outright (early failure at
   TOML parse) or does admission reject after parser-level structural
   validation (later failure at submit time)? Interaction with slice-06's
   already-landed `Option`-shaped field. DESIGN owns the call, including
   any necessary ADR-0031 amendment.

## Risks surfaced for DESIGN handoff

| # | Risk | Severity | Owner | Mitigation framing |
|---|---|---|---|---|
| R1 | The shared-primitive refactor of `BackendIdAllocator` touches dataplane hot-path code | Medium | architect | The existing primitive has good test coverage (proptest at `allocator.rs:92-110`, collision-witness at `:125-138`); refactor preserves test surface. |
| R2 | Submit-time vs. reconciler-tick allocation choice has different idempotency surfaces | Medium | architect | Open question 2 above; either is defensible but the failure modes differ (transient submit failure vs. transient reconciler tick). |
| R3 | Pool exhaustion is a real operator-visible failure mode | Medium | architect | DoR item 9 (Outcome KPIs) tracks "pool exhaustion rejection rate" as a guardrail metric. |
| R4 | DIVERGE artifacts absent | Low | PO | Not blocking: SSOT issue #167 with locked scope + a single technical direction is the same epistemic content as DIVERGE would produce. Captured here for traceability. |
| R5 | DISCOVER artifacts absent | Low | PO | Not blocking: backend primitive with no end-user opportunity space; #167's tracked-follow-up status IS the closed opportunity statement. |

## Deferrals (none)

No deferrals to GitHub issues are introduced by this DISCUSS wave. All
open questions (above) are scoped within #167's umbrella and are
explicitly DESIGN-wave concerns — per the project rule "Deferrals
require GitHub issues — AND user approval BEFORE creation"
(`.claude/rules/development.md`), the open questions are NOT separate
deferrals; they are the design surface of the parent feature.

Per the task framing: **do not create additional GitHub issues for the
open questions**.

## Handoff readiness

See `dor-validation.md` for the 9-item DoR pass. See `user-stories.md`
for the single user story. See `outcome-kpis.md` for measurable targets.
See `story-map.md` for the one-row map (slicing deferred to DESIGN).

The DESIGN wave (`@nw-solution-architect`) should expect:

- One new ADR (working title: "Platform-issued Service VIP allocator —
  shared primitive in `crates/overdrive-dataplane/src/allocators/`") OR
  an amendment to whichever ADR owns the dataplane allocator surface
  (architect's call).
- Possible ADR-0031 amendment if the admission rejection of operator-
  supplied `vip = Some(...)` lands at the parser layer (open question
  5).
- Possible `[dataplane]` config block extension (open question 3).
- Resolution of all five open questions before DELIVER.

The DEVOPS wave (`@nw-platform-architect`) receives `outcome-kpis.md`
only — KPI K1 (successful-allocation rate), K2 (allocator-induced
admission latency p50/p99), K3 (VIP reclamation lag) need
instrumentation planning.

All priorities (P1, P2, …) are in scope by default per CLAUDE.md.

## Skipped artifacts (with rationale)

The skill template lists several artifacts a full DISCUSS wave
produces. The following are explicitly skipped:

| Artifact | Skip rationale |
|---|---|
| `journey-*-visual.md` / `journey-*.yaml` | Backend primitive; no end-user journey. |
| `shared-artifacts-registry.md` | Only one shared artifact (the VIP itself); the registry would be a one-row table with no integration risk. |
| `acceptance-criteria.md` standalone | ACs are embedded in `user-stories.md` per the skill template's standard. |
| JTBD analysis artifacts | Covered by J-OPS-002 / J-OPS-003; no new job statement needed. |
| Carpaccio slicing within `story-map.md` | Single story; DESIGN/DELIVER produces the roadmap with slices. |
| Prioritization document | Single story; no prioritization needed. |

## Changelog

- 2026-05-14 — Initial DISCUSS wave decisions captured. Single story,
  ACs derived from #167 minus pinned-VIP items, five open questions
  parked for DESIGN, one Changed Assumption back-propagated against
  upstream Slice 06 of `workload-kind-discriminator`.
