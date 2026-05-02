# Slice 1 — `alloc status` snapshot enrichment

**Feature**: `cli-submit-vs-deploy-and-alloc-status`
**Wave**: DISCUSS / Phase 2.5
**Owner**: Luna (will hand to nw-solution-architect for ADR; nw-software-crafter for delivery)

## Goal

Replace `Allocations: 1` with the dense snapshot specified in the
journey's step 4 TUI mockup, so an operator inspecting any allocation
(Running, Failed, Pending) gets actionable state without pivoting to
external tools.

## IN scope

- Extend (or version) `AllocStatusResponse` in
  `overdrive-control-plane::api` with: per-allocation
  `state`/`resources`/`started_at`/`exit_code`, a
  `last_transition` block (`from`, `to`, `reason`, `source`, `at`),
  and a `restart_budget` field (`used`, `max`, `exhausted`).
- Server-side: hydrate the new fields from the existing
  ObservationStore `AllocStatusRow` lineage and the lifecycle
  reconciler's libSQL view (which already tracks restart count per
  phase-1-first-workload step 5).
- CLI-side: rewrite the `alloc status` renderer to produce the TUI
  layout in the journey's mockup. Honest empty-state handling for
  the no-allocations case AND for the Pending-with-no-capacity case.
- Acceptance test: a real ProcessDriver-backed Failed allocation
  (binary-not-found) renders with the verbatim driver error in the
  `error:` line and `Restart budget: M / M used (backoff exhausted)`
  when applicable.

## OUT scope

- NDJSON streaming on `submit` (Slice 2).
- `alloc status --follow` / `--watch` (not in this feature).
- Multi-replica per-allocation grouping/aggregation; Phase 1 is
  `replicas=1` per the single-node constraint.
- TUI-mode rendering (out of feature scope entirely).
- New telemetry surfaces beyond what the snapshot directly exposes.

## Learning hypothesis

**Disproves**: an operator inspecting a deliberately-broken
allocation can identify the cause from the new `alloc status` output
ALONE without running `journalctl`, `systemctl status`, or peeking
at the cgroup scope.
**Confirms**: the snapshot is sufficient as the second-day inspection
surface and Slice 2's "snapshot reason == streaming terminal reason"
AC has a concrete target.

## Acceptance criteria

1. `alloc status` for a Running allocation renders: `STATE`,
   `RESOURCES`, `STARTED`, `EXIT`, `Last transition` block (`from →
   to`, `reason`, `source`), `Restart budget` line.
2. `alloc status` for a Failed allocation (broken binary) includes
   the verbatim `stat /usr/local/bin/payments: no such file or
   directory` error string AND `Restart budget: M / M used (backoff
   exhausted)` once the lifecycle reconciler has stopped retrying.
3. `alloc status` for a Pending allocation with no node capacity
   renders an explicit `Pending: no node has capacity (...)` row
   (NOT a silent zero-allocations render).
4. The wire shape (`AllocStatusSnapshot` or extended
   `AllocStatusResponse`) is a typed Rust struct in
   `overdrive-control-plane::api` per ADR-0014, derives `Serialize`,
   `Deserialize`, `ToSchema`.
5. The CLI renderer is unit-tested against fixtures for Running,
   Failed, and Pending-no-capacity cases.

## Dependencies

- Lifecycle reconciler's `restart_count`/`backoff` view (already
  exists from phase-1-first-workload).
- `AllocStatusRow` already carries `state` and reason (already
  exists; this slice reads, does not produce).
- ProcessDriver error pass-through to the action shim's
  AllocStatusRow write (already exists from phase-1-first-workload).

## Effort estimate

≤1 day.

## Reference class

`nomad alloc status`'s task-events panel — the reference shape from
the competitive-research doc. The Overdrive snapshot is denser per
field but covers the same information categories.

## Pre-slice SPIKE

None required. The data already exists; this slice is a wire-shape
addition and a renderer rewrite.
