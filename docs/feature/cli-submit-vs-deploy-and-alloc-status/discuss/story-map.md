# Story Map — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DISCUSS / Phase 2.5
**Owner**: Luna
**Date**: 2026-04-30

## User: Ana — Overdrive platform engineer
## Goal: Run `overdrive job submit` and either know the platform converged inline, or know exactly why it didn't, in one verb, in seconds.

---

## Backbone

| Activity 1 — Submit | Activity 2 — Observe convergence | Activity 3 — Inspect post-deploy |
|---|---|---|
| Read TOML, build request | Watch lifecycle stream | Render dense snapshot |
| Detect TTY / pipe / `--detach` | Map terminal event to exit code | Surface restart budget |
| POST to /v1/jobs (Accept-gated) | (failure path) Print structured Error | Surface verbatim driver error |
| (`--detach`) Get one JSON object | (success path) Print summary, exit 0 | Honest empty-state for Pending/no-capacity |

---

## Walking Skeleton

**N/A — explicit per the wave invocation (`walking_skeleton: no`).**
This feature is brownfield: the inner-loop submit already exists, the
lifecycle reconciler already runs, and `alloc status` is already
wired (it just renders sparsely). The feature does NOT establish a
new end-to-end flow; it reshapes an existing one. The slicing
discipline is therefore "smallest no-regret cut first" rather than
"thinnest end-to-end skeleton."

---

## Slices (carpaccio)

Three thin slices, each end-to-end and shippable in ≤1 day. Each
brief at `docs/feature/cli-submit-vs-deploy-and-alloc-status/slices/`.

### Slice 1 — `alloc status` snapshot enrichment (no-regret)

**Goal**: rewrite `alloc status` from `Allocations: 1` into the dense
snapshot specified in the journey's step 4 TUI mockup.

**Why first**: ships under any DIVERGE option (S, A, M). The
DIVERGE recommendation called this out as no-regret. Validates the
snapshot type shape against the existing `AllocStatusResponse` and
gives DESIGN a stable target before the streaming-submit ADR is
finalised. No new transport machinery required.

**Learning hypothesis**: "Operators inspecting a deliberately-broken
allocation (binary-not-found case) find the new fields actionable
enough that they do not reach for `journalctl` or `systemctl status`
on the cgroup scope to diagnose." Disproves slice value if operators
still pivot away from the CLI for the same diagnostic the snapshot
should serve.

**Effort**: ≤1 day.

### Slice 2 — NDJSON streaming submit (the load-bearing slice)

**Goal**: ship NDJSON streaming on `POST /v1/jobs` (Accept-header
gated) end-to-end through the CLI, with TTY-detection, `--detach`,
and exit-code mapping. Validate against the broken-binary regression
case the user filed.

**Why second**: depends structurally on the snapshot fields shipped
in Slice 1 (the `transition_reason` + `restart_budget` lineage that
both surfaces share). Once Slice 1 is in, the streaming events have
a concrete `reason` shape to emit; the AC asserting "snapshot reason
== streaming terminal reason" can be tested.

**Learning hypothesis**: "Streaming submit reaches Running OR
surfaces failure inline on the broken-binary case, with exit code
matching the terminal event, in a single verb." Disproves the Option
S recommendation if (a) operators actively prefer detached + polling,
(b) the 200ms first-event budget is unreachable on a healthy local
control plane, or (c) the NDJSON shape proves harder to consume than
SSE.

**Effort**: ≤1 day. The slice is bounded; both server-side
NDJSON emission and CLI-side line-delimited consumption have mature
Rust patterns (axum's `Sse` family demonstrates the streaming-body
shape; reqwest exposes `bytes_stream()` for the consumer).

### Slice 3 — `--detach` flag and pipe auto-detection (only if Slice 2 grows)

**Goal**: pull `--detach` and `isatty(stdout)`-based auto-detach into
their own slice if the implementation surface in Slice 2 starts
threatening the ≤1-day budget.

**Why conditional**: the recommendation explicitly notes this can be
folded into Slice 2. The decision to split is a complexity-budget
call DESIGN/CRAFT can make at the start of Slice 2 work; DISCUSS
documents the option here so the slicing is not surprising.

**Learning hypothesis**: "Auto-detach on piped stdout removes the
need for CI scripts to remember `--detach`." Disproves if CI
operators still find themselves passing `--detach` defensively
because TTY detection is unreliable in their environment (containers
without a TTY allocated, GitHub Actions, etc.).

**Effort**: ≤0.5 day if needed.

---

## Slice taste tests (per skill)

| Test | Verdict |
|---|---|
| "If a slice ships 4+ new components → not thin." | PASS. Slice 1: one snapshot struct + one renderer change. Slice 2: one NDJSON event enum + server emitter + CLI consumer + exit-code mapping (4 components, all small). Slice 3: one flag + one isatty check. |
| "If every slice depends on a new abstraction → ship abstraction first." | PASS. Slice 1 establishes the snapshot struct; Slice 2 establishes the NDJSON event enum. Each abstraction lands inside the slice that introduces it; no slice depends on an unshipped abstraction. |
| "If no slice disproves any pre-commitment → decoration." | PASS. Slice 2 disproves the entire Option S recommendation if the broken-binary case fails to surface inline. That's a sharp learning hypothesis. |
| "If a slice uses only synthetic data → proves plumbing only." | PASS. Slice 1's AC requires a Failed allocation produced by a real ProcessDriver call against a missing binary. Slice 2's AC names the same regression-target session the user filed. Production data, not synthetic. |
| "If two slices are identical except for scale → merge." | PASS. Slice 1 and Slice 2 are structurally distinct (snapshot vs streaming). Slice 3 is conditional on Slice 2's complexity, not a duplicate. |

---

## Priority Rationale

Outcome impact ranks Slice 1 highest on "fix-the-actionable-output"
(the second half of the user's complaint) and Slice 2 highest on
"fix-the-honest-success-criterion" (the first half). Both score 5/5
on outcome impact. The dependency tie-break is technical: Slice 2's
streaming events emit the same `transition_reason` lineage Slice 1
exposes on the snapshot, so Slice 1 going first means Slice 2's
"snapshot reason == terminal reason" AC has something to assert
against immediately.

| Priority | Slice | Outcome targeted | KPI | Rationale |
|---|---|---|---|---|
| 1 | Slice 1 — `alloc status` enrichment | "Operators identify failure cause without external tools" (ODI outcome 3) | Snapshot field count ≥ 6; verbatim driver error present | No-regret under any DIVERGE option; smallest cut; type-shapes the snapshot before streaming arrives. |
| 2 | Slice 2 — NDJSON streaming submit | "Time to know if spec converged" + "Likelihood of silent-accept-while-failing" (ODI outcomes 1+2+6) | First NDJSON event ≤200 ms p95; exit code 1 on broken-binary; same `reason` in stream and snapshot | Load-bearing; closes the user's actual complaint; depends on Slice 1's `transition_reason` shape being concrete. |
| 3 (conditional) | Slice 3 — flag + auto-detach | "CI/automation friction" — not a journey ODI outcome | Pipe → JSON object; `--detach` overrides | Only carved out if Slice 2's budget is at risk. Otherwise folded into Slice 2. |

---

## Scope Assessment: PASS — 6 stories, 1 bounded context (CLI ↔ control plane), estimated 2–3 days

The feature is right-sized:

- Stories: 6 (well under the 10-story ceiling).
- Bounded contexts touched: 1 (CLI ↔ control plane API surface).
  No reconciler internals modified; no driver internals modified;
  no IntentStore/ObservationStore schema changes.
- Walking skeleton: explicitly waived (brownfield extension).
- Effort: 2–3 days end-to-end (Slice 1 ≤1d + Slice 2 ≤1d + optional
  Slice 3 ≤0.5d).
- Independence from other in-flight features: no cross-feature
  ordering dependency. Phase-1-first-workload's lifecycle
  reconciler must be in main (it is — see git log).

No oversize signals trigger.
