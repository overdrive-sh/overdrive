# DISCUSS Decisions — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DISCUSS (product-owner / Luna)
**Date**: 2026-04-30
**Status**: COMPLETE — handoff-ready for DESIGN
(`nw-solution-architect`), pending peer review by
`nw-product-owner-reviewer`.

---

## Inputs honoured

- DIVERGE recommendation: Option S (Submit-streams-default), score
  4.47 vs runner-up Option A 3.77. Clear winner. Source:
  `recommendation.md`.
- DIVERGE wave-decisions key decisions 1-6 (validated job at
  strategic level, locked taste weights, Option S as recommendation,
  Option A as documented fallback, no-regret snapshot enrichment).
  Source: `wave-decisions.md` (DIVERGE).
- Validated job (do NOT re-run JTBD): "Reduce the time and
  uncertainty between declaring intent and knowing whether the
  platform converged on it." Source:
  `diverge/job-analysis.md`.
- Six ODI outcomes, five severely under-served. Source:
  `diverge/job-analysis.md` §6 / §7.
- Inherited journey: `docs/product/journeys/submit-a-job.yaml` (base)
  → `docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended.yaml`
  (intermediate). This wave produces the third extension.

---

## Key Decisions

### [D1] Streaming wire format is **NDJSON**, not SSE

**Decision**: `POST /v1/jobs` with `Accept: application/x-ndjson`
returns one JSON event per line (`Content-Type: application/x-ndjson`).
No `text/event-stream`, no `event:` / `data:` / `id:` framing.

**Rationale** (carried from the user's framing during DISCUSS):

1. **Single consumer.** The CLI is the only consumer in Phase 1.
   SSE's value (browsers, polyglot consumers, automatic reconnect)
   doesn't apply.
2. **One-shot, not long-lived.** Submit's stream closes on terminal
   convergence. SSE optimises for indefinite feeds; this is
   convergence-bounded.
3. **Mature Rust ecosystem.** `serde_json::Deserializer::from_reader`
   + `reqwest::Response::bytes_stream()` driven line-by-line is
   well-precedented (observability vendors do this in production at
   scale).
4. **OpenAPI describes NDJSON natively** via `application/x-ndjson`
   media type; the schema continues to be a byproduct of the typed
   surface per ADR-0014.

**Consequence**: DESIGN's NDJSON ADR (see "ADR follow-on" below)
documents the typed event enum and the line-delimited consumption
contract. SSE is explicitly considered-and-rejected for this
endpoint; a future cross-language consumer can re-evaluate.

**Source**: user instruction during DISCUSS handoff
("NDJSON over SSE confirmed by user").

### [D2] Submit-streams-default is the chosen direction (Option S ratified)

**Decision**: ratify the DIVERGE recommendation. `overdrive job
submit` streams convergence by default on TTY; auto-detaches on
piped stdout; explicit `--detach` overrides. Existing
`Accept: application/json` shape retained for back-compat.

**Rationale**: the streaming-submit assumption that DIVERGE
identified ("operators want a single-verb inner-loop experience and
accept that the verb's success criterion is 'converged,' not just
'committed'") is supported by the user's own framing of the
question. Both fallback triggers are explicitly NOT met:

1. The team WILL ship streaming machinery in Phase 1 (NDJSON; cost
   bounded by mature axum + reqwest patterns).
2. The API contract evolution (sync-JSON → polymorphic-by-Accept
   NDJSON) is judged tractable for the Phase 1 deadline. Slice 2
   is ≤1 day; the optional pre-slice spike de-risks the first
   afternoon.

**Source**: user invocation explicitly identifies Option S as
locked.

### [D3] Exit-code contract: 0 / 1 / 2

**Decision**: CLI exit codes are constrained to:

| Exit | Meaning |
|---|---|
| 0 | Convergence reached Running (or terminal-success for one-shot drivers, not yet shipped). |
| 1 | Convergence reached terminal-failure (driver error, restart budget exhausted, server wall-clock cap exceeded). |
| 2 | Client-side error (bad TOML, transport failure to control plane, server validation rejection per ADR-0015). |

**Rationale**: `64–78` (sysexits.h range) deliberately reserved to
avoid premature commitment. The 0 / 1 / 2 contract maps cleanly to
ADR-0015's HTTP error shape (4xx / 5xx → exit 2) and to the
streaming-protocol terminal events (Running → 0; Failed → 1).

**Source**: user instruction during DISCUSS handoff. ADR-0015 is
the basis for the HTTP-to-exit-code mapping.

### [D4] `alloc status` stays a snapshot surface — `--follow` is OUT of scope

**Decision**: `alloc status` does NOT gain a `--follow` /
`--watch` flag in this feature. It remains a snapshot surface,
enriched per the journey's TUI mockup. A future feature may add
streaming on `alloc status`, but it does not bundle here.

**Rationale**: the streaming submit covers the "live observation
during convergence" use case. After convergence completes, snapshot
inspection is the dominant case. Bundling `--follow` would expand
the slice surface without addressing a named ODI outcome.

**Source**: user instruction.

### [D5] Server-side wall-clock cap exists; value is a DESIGN call

**Decision**: the streaming endpoint MUST close the stream with
`ConvergedFailed { terminal_reason: timeout, ... }` after a
configurable wall-clock budget. DISCUSS does not pick the value;
60 s is suggested as a starting point.

**Rationale**: without a cap the stream can hang indefinitely if the
reconciler enters a pathological backoff state. The terminal event
gives the CLI a clean exit and exit code 1.

**Source**: emerges from the journey's emotional-arc analysis (the
operator must not be left staring at a stuck terminal). Value is a
DESIGN-wave concern.

### [D6] Snapshot wire shape extends `AllocStatusResponse` (or versions it)

**Decision**: the snapshot adds per-allocation fields (`state`,
`resources`, `started_at`, `exit_code`) and top-level fields
(`last_transition`, `restart_budget`) to the existing
`AllocStatusResponse` (or a versioned successor). DESIGN owns the
choice between extending in place vs versioning.

**Rationale**: the existing shape is sparse; extending is additive.
The shared-types pattern from ADR-0014 means CLI and server move
together regardless.

**Source**: journey YAML step 4 + US-05.

### [D7] Single source of truth for `transition_reason`

**Decision**: the same `String` (or typed `Reason` enum if DESIGN
prefers) flows through the lifecycle reconciler view + ProcessDriver
pass-through into BOTH the streaming `LifecycleTransition.reason` /
`ConvergedFailed.error` AND the snapshot
`last_transition.reason` / per-row `error`. Drift is a defect.

**Rationale**: the journey's "told the truth" emotional promise
spans both consumption surfaces; an integration test asserts
byte-for-byte equality (US-06).

**Source**: shared-artifacts-registry analysis.

### [D8] Walking skeleton is explicitly waived

**Decision**: this feature is a brownfield extension; the
inner-loop submit already exists and the lifecycle reconciler
already runs. There is no "thinnest end-to-end skeleton" to ship
because the end-to-end already exists; the slicing is "smallest
no-regret cut first" instead.

**Source**: per the wave invocation (`walking_skeleton: no`).

---

## Requirements Summary

- **Primary job** (validated, do not re-run): "Reduce the time and
  uncertainty between declaring intent and knowing whether the
  platform converged on it."
- **Walking skeleton scope**: N/A (waived; brownfield extension).
- **Feature type**: cross-cutting (CLI + control-plane HTTP API +
  shared-types module).
- **Slicing**: 2 slices (+ 1 conditional). Slice 1 (no-regret
  snapshot enrichment) ships first; Slice 2 (NDJSON streaming
  submit) builds on Slice 1's `transition_reason` surface; Slice 3
  (`--detach` + auto-detach) is conditional on Slice 2's complexity
  budget.

---

## Constraints Established

- **NDJSON over SSE** ([D1]).
- **Reconciler purity preserved** (§18, ADR-0023). Streaming
  endpoint is a CONSUMER of ObservationStore rows, not a producer
  that blocks the reconciler tick.
- **Intent / Observation split preserved** (whitepaper §4). Submit
  writes intent through IntentStore::put_if_absent (ADR-0020);
  convergence is observed via ObservationStore subscription.
- **Shared types** (ADR-0014). All new types live in
  `overdrive-control-plane::api`.
- **Error shape** (ADR-0015). Validation / not-found / conflict /
  internal errors flow through the existing `ErrorBody`. Streaming-
  protocol failures become NDJSON `ConvergedFailed` events, not
  `ErrorBody` payloads.
- **Exit-code contract** ([D3]).
- **Phase 1 single-node** (inherited).
- **Greenfield migration discipline**. Existing `Accept:
  application/json` shape is RETAINED unchanged. The new NDJSON
  shape is opted-into via Accept header. No deprecation period; no
  feature flag; the back-compat is structural (Accept-header
  gating).

---

## Upstream Changes

None. The DIVERGE wave's recommendation, fallback, and assumptions
all stand. DISCUSS adds five new constraints ([D1] NDJSON;
[D3] exit codes; [D4] `--follow` out of scope; [D5] server cap
exists; [D7] single source of truth for `transition_reason`) that
DIVERGE did not commit to but did not contradict. Two of the new
constraints came from explicit user instruction during the DISCUSS
invocation ([D1], [D3]).

---

## ADR follow-on (DESIGN wave's responsibility)

Two ADRs expected on the DESIGN side; both follow ADR-0014 (shared
types) and ADR-0015 (error shape) precedents.

1. **NDJSON streaming submit shape** —
   - `Accept: application/x-ndjson` gating on `POST /v1/jobs`.
   - Typed event enum (`SubmitEvent` or named) with at minimum
     `Accepted`, `LifecycleTransition`, `ConvergedRunning`,
     `ConvergedFailed` variants.
   - Server wall-clock cap value (60 s suggested) and surface
     (config? CLI override?).
   - Error mapping for HTTP-level failures vs NDJSON terminal
     events.
   - OpenAPI declaration of the NDJSON media type.

2. **`alloc status` snapshot enrichment** —
   - Shape choice: extend `AllocStatusResponse` in place vs version.
   - Per-allocation fields: `state`, `resources`, `started_at`,
     `exit_code`.
   - Top-level fields: `last_transition` (TransitionRecord shape),
     `restart_budget` (RestartBudget shape).
   - CLI render contract for Running, Failed, Pending-no-capacity
     cases.

DEVOPS receives `outcome-kpis.md` only; KPI-01 (200 ms first-event
budget) is the only KPI that may want a future telemetry surface,
and it's explicitly Phase 1 acceptance-test-only.

---

## Hand-off shape

- → DESIGN (`nw-solution-architect` / Sage): produce two ADRs above.
  DISCUSS provides the journey YAML, user-stories.md, slice briefs,
  outcome-kpis.md, and shared-artifacts-registry.md as the artifact
  set.
- → DEVOPS (`nw-platform-architect`): receive `outcome-kpis.md`
  only. No streaming-specific telemetry infrastructure required for
  Phase 1.
- → DISTILL (`nw-acceptance-designer`): receives the journey YAML
  with embedded Gherkin (no standalone `.feature` file produced
  here per `.claude/rules/testing.md`), integration points, and
  outcome KPIs.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-30 | Initial DISCUSS wave artifacts. Ratified Option S; locked NDJSON over SSE; locked exit-code contract; locked `alloc status --follow` out of scope; documented server-side wall-clock cap as DESIGN call; locked single-source-of-truth for `transition_reason`; waived walking skeleton (brownfield). Six user stories, three slice briefs (one conditional), nine-item DoR PASS. Ready for peer review. |
