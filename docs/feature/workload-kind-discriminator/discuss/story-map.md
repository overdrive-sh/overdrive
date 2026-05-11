# Story Map — workload-kind-discriminator

## Changed Assumptions

- 2026-05-10 — folded in GH #164 (service listener spec shape) as Slice 06.
  Goal extended: operators also declare per-listener `(port, protocol, vip?)`
  on a Service spec and trust submit/`alloc status` to round-trip them. The
  runtime allocator for `vip = None` is tracked at #167; this feature ships
  only the spec shape, forward-compatible with either #167 outcome.

## User: Ana, Overdrive platform engineer
## Goal: declare a workload's lifecycle kind (and, for Services, the listener triples) in the spec, and trust the CLI's view to match that kind end-to-end

## Backbone (user activities, left-to-right)

| 1. Author spec | 2. Submit | 3. Watch convergence | 4. Inspect post-hoc | 5. Recover from misuse |
|---|---|---|---|---|
| Decide kind (Service / Job / Schedule) | `overdrive job submit <spec>` | Read streaming events | `overdrive alloc status --job <id>` | Read parser errors |
| Write `[service]` or `[job]` block | See submit echo with kind | See terminal event (Job) or running summary (Service) | See kind-aware render | Fix mixed-kind specs |
| Add `[schedule]` if recurring | Receive validated commit | Get honest verdict | See exit codes for Failed Jobs | Discover composition rules |

## Walking-skeleton-equivalent slice (NOT the same as a fresh walking skeleton)

This feature evolves a landed walking skeleton (Phase 1 `submit-a-job` + first-workload).
The "thinnest end-to-end slice that proves the kind discriminator concept" is **Slice 02**
(Job submit ends on terminal exit). It is the shortest path to:

1. A user-visible behavioral change ("the bug is fixed").
2. A working Service path (no regression).
3. A working Job path (the new structural separation).

Slices 01 (parser) and 04 (Service preservation) are prerequisites that ship first; Slice
02 is where the operator's lived experience changes. Slice 03 (alloc status exit code) is
the user's explicit framing journey. Slice 05 (Schedule parsing) is the lowest-priority
slice and could ship after Slices 01–04 land independently.

## Carpaccio slices (vertical, ≤1 day each, each ships end-to-end value)

### Slice 01 — Spec parser accepts `[service]` / `[job]` discriminator

- **Hypothesis**: introducing the `WorkloadKind` enum at the parser boundary unblocks
  every downstream change without breaking the existing walking-skeleton submit path
  (which becomes the Service code path post-rename).
- **End-to-end value**: an operator can write a Job spec that *parses* and a Service
  spec that *parses*; mixed-kind specs are rejected with named guidance.
- **Production data**: the existing `examples/coinflip.toml` is migrated in this slice
  to the new `[job]` shape (single-cut migration per
  `feedback_single_cut_greenfield_migrations.md`).
- **Effort**: ~1 day.
- **Touches**: `crates/overdrive-cli/src/spec.rs` (or wherever the spec parser lives —
  the architect will pin the path), `crates/overdrive-core/src/aggregate/`, parser
  tests, `examples/coinflip.toml`.

### Slice 02 — Job submit terminates on Succeeded/Failed (closes the bug)

- **Hypothesis**: a typed `JobSubmitEvent` enum with no `ConvergedRunning` variant makes
  the operator-facing bug structurally unreachable — the call site that emits "is
  running with N/M replicas (took live)" does not exist on the Job code path.
- **End-to-end value**: `overdrive job submit examples/coinflip.toml` returns
  `Job 'coinflip' succeeded.` (exit 0, exit code 0) on a SUCCESS run and
  `Job 'coinflip' failed.` (exit non-zero, exit code 1, attempt count) on an ERROR run.
  The bug is gone.
- **Production data**: re-uses `examples/coinflip.toml` migrated in Slice 01. Empirically
  measured before/after: today's CLI output is `is running with 1/1 replicas (took
  live)` 100% of the time; after this slice, output matches the workload's actual
  exit code 100% of the time.
- **Effort**: 1.5 days (largest slice — touches streaming protocol).
- **Touches**: `crates/overdrive-control-plane/src/streaming.rs`,
  `crates/overdrive-control-plane/src/api.rs` (`SubmitEvent` per-kind),
  `crates/overdrive-cli/src/commands/job.rs`, `crates/overdrive-cli/src/render.rs` (new
  `format_succeeded_summary`, `format_failed_summary`).

### Slice 03 — `alloc status --job <id>` surfaces exit code for Failed Jobs

- **Hypothesis**: kind-aware `alloc status` render is the user's explicit "trust"
  surface for the bug-affected workload — they can see what really happened post-hoc,
  even if the streaming submit window closed early.
- **End-to-end value**: `overdrive alloc status --job coinflip` (after a Failed run)
  shows `Verdict: Failed (backoff exhausted)`, per-attempt `Exit` column with the real
  exit code, and the stderr tail.
- **Production data**: the same coinflip workload, post-Slice-02.
- **Effort**: 1 day.
- **Touches**: `crates/overdrive-cli/src/render.rs` (kind-aware render branch),
  `AllocStatusRow.kind` denormalisation, alloc status CLI command handler.

### Slice 04 — Service submit preserves `ConvergedRunning` semantics (no regression)

- **Hypothesis**: the existing Service-shaped tests (long-running workloads, e.g.
  `/bin/sleep 3600`) continue to pass with vocabulary changed to "Service" but
  semantics unchanged.
- **End-to-end value**: an operator's existing Service workflow is untouched — the
  feature does not break what was working.
- **Production data**: the existing `streaming_submit_happy_path.rs` test fixtures
  (long-running binaries) are migrated to the `[service]` shape and continue to pass.
- **Effort**: 0.5 days (rename + verify).
- **Touches**: `crates/overdrive-cli/tests/integration/streaming_submit_happy_path.rs`,
  `crates/overdrive-cli/src/render.rs` ("Job" → "Service" in the running summary).

### Slice 05 — `[schedule]` parses and validates composition rules (execution deferred)

- **Hypothesis**: parsing `[job] + [schedule]` and rejecting `[schedule]`-without-`[job]`
  / `[schedule]-with-[service]` ships the syntactic surface without incurring the cost
  of a Schedule reconciler in this feature. The CLI is honest about the deferral.
- **End-to-end value**: an operator can write a Scheduled Job spec that parses and gets
  an honest "registered, but execution is not yet implemented" submit echo plus a
  consistent alloc status reflection.
- **Production data**: a new `examples/nightly-backup.toml` shipped in this slice
  exercising the `[job] + [schedule]` shape.
- **Effort**: 1 day.
- **Touches**: parser (Schedule variant), CLI submit echo, alloc status render
  (Schedule branch), CLI config constant for deferral URL, `examples/nightly-backup.toml`.

### Slice 06 — Service `[[listener]]` spec shape (folded in 2026-05-10 from #164)

- **Hypothesis**: shipping the listener spec shape now (without the runtime
  allocator) lets operators declare protocol/port-aware Services and round-
  trip the triples through submit + alloc status. The `Option`-shaped `vip`
  is forward-compatible with #167's eventual runtime decision.
- **End-to-end value**: operator writes `[[listener]]` blocks under
  `[service]`, sees a `Listeners:` section in submit echo and `alloc
  status`, and can pin VIPs or leave them pending (rendered as `(vip:
  pending allocation — see #167)`).
- **Production data**: parser tests + a new integration test asserting
  byte-equality between submit echo and `alloc status` Listeners sections.
- **Effort**: 1.5 days.
- **Touches**: spec types (Listener, ServiceVip), parser (Service variant
  extension), CLI submit echo render (Service kind), `alloc status` render
  (Service kind), `AllocStatusRow` listener fields (architect confirms
  shape), OpenAPI derives (`utoipa::ToSchema`), property test.

## Priority Rationale

Priority is set by the joint product of **outcome impact** (does this slice move the
honesty KPI?) and **dependency order** (Slice 01 must ship before Slices 02/03/04/05/06;
Slice 02 must ship before Slice 03 makes sense end-to-end; Slice 04 must ship before
Slice 06 because Slice 06 extends Service-side render machinery). Slice 05 is
detachable from the rest. Slice 06 (folded in 2026-05-10 from #164) lands after Slice
04 (Service preservation) but is independent of Slices 02 and 03 (both Job-kind work).

| Order | Slice | Why this position |
|---|---|---|
| 1 | 01 — Parser kind discriminator | **Enabler.** Every other slice depends on `WorkloadKind` existing at the parser boundary. The new `WorkloadKind` enum IS the new abstraction. (This is the "every slice depends on a new abstraction" smell, but it is *correct* here — the abstraction IS the feature. Calling it out explicitly per the Elephant Carpaccio guidance.) |
| 2 | 02 — Job submit terminal | **Highest outcome impact.** This slice IS the bug fix. The `${honesty_rate}` KPI (K1) moves from 0% to 100% the moment this slice lands. Largest slice; could split if effort balloons but kept as one because the structural fix is one move. |
| 3 | 03 — alloc status Job render | **User's explicit framing.** The operator quote *"when we check the status using overdrive alloc status --job <id> then it should show that it failed during execution"* names this slice directly. Cannot ship before Slice 02 because the test data (Failed terminal Jobs) requires Slice 02's Job-lifecycle wiring. |
| 4 | 04 — Service preservation | **Regression guard.** Could be parallel with Slices 02/03 if developer capacity allows; positioned after them because the rename is mechanical and easier to do once the kind enum is settled. **Prerequisite for Slice 06** — the Service render path Slice 06 extends is Slice 04's deliverable. |
| 5 | 06 — Service listener spec shape | **Folded in from #164 (2026-05-10).** Lands after Slice 04 because it extends Service render machinery. Independent of Slices 02 and 03. The runtime allocator (#167) is a separate feature this slice is forward-compatible with. K6 (listener round-trip byte-equality) is the new KPI. Positioned ahead of Slice 05 because Slice 06's outcome KPI is more directly testable in CI than Slice 05's deferral consistency, and the listener feature is the larger user-facing surface. |
| 6 | 05 — Schedule parsing | **Lowest priority.** Detachable from the rest; ships honest deferral. Could be split into its own follow-up feature if the team prefers, but kept here so the parser work for `[schedule]` lives next to the parser work for `[service]` / `[job]`. Now lands after Slice 06 because Slice 06 extends the Service render machinery on which Slice 05's render symmetry depends (both echo + alloc status surfaces gain new conditional sections). |

> **Why Slice 06 is positioned 5th (between Slice 04 and Slice 05)**: dependency order
> places it after Slice 04 (Service render path it extends), and outcome-impact ranking
> places it ahead of Slice 05 (the listener round-trip is more directly observable in
> CI than the Schedule deferral consistency). Slice 05 remains the lowest priority but
> is no longer strictly the last to land — it depends only on Slice 01.

## Scope Assessment: PASS — 8 stories (one per slice plus one tech-task plus one migration plus one fold-in), 1 bounded context (CLI/control-plane), estimated 5.5–6.5 days

Right-sized check (per skill):

- Bounded contexts touched: 1 — the CLI/control-plane streaming-and-render surface.
  (The `overdrive-core` enum and `Listener` aggregate additions are small and live in
  the same logical context.)
- Story count: **8 stories** total — one per slice for 6 (US-01..05, US-08), plus 1
  cross-cutting "tech task" story (US-06: anti-pattern grep gate), plus 1 migration
  story (US-07: `examples/coinflip.toml`). Sits at the upper edge of "right-sized" but
  the slices remain thin and dependent in a clean order.
- Estimated effort: 5.5–6.5 days end-to-end on a single engineer (added ~1.5 days for
  Slice 06).
- Walking-skeleton fit: the slices form a coherent end-to-end progression with
  measurable outcome KPI movement after Slice 02 (K1) and after Slice 06 (K6).

No splitting required at the feature level — Slice 06 itself defends its single-slice
shape in `slice-06-service-listener-fields.md` with the architect's natural fault line
(parser+echo vs. alloc status) named explicitly if the slice grows during DESIGN.

## Anti-pattern check

- ❌ Feature-first slicing? **No** — every slice delivers end-to-end behaviour change
  for a real persona, not "all of feature X then all of feature Y".
- ❌ No walking skeleton? **N/A** — this is an evolution; the Phase 1 walking skeleton
  is the substrate. Slice 02 is the slice that proves the kind discriminator works
  end-to-end.
- ❌ Effort-based priority? **No** — outcome impact (Slice 02) drives priority, not
  ease.
- ❌ Orphan stories? **No** — every story traces to either J-OPS-002 or J-OPS-003 (or
  both); see `user-stories.md` for the trace per story.
- ❌ Activity gaps? **No** — every backbone activity is touched by at least one slice.

## Cross-references

- `prioritization.md` — release sequence with KPIs
- `slice-NN-*.md` — per-slice detail (≤100 lines each)
- `user-stories.md` — full LeanUX stories per slice
- `outcome-kpis.md` — measurable targets per story / per epic
