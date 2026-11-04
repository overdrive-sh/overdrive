# ADR-0265: ServiceLifecycle announces Stable from the committed row

## Status

Accepted, 2026-11-03 (approval: feature delta § Approval record, B1; accepted as
proposed). Not implemented. GH #318, GH #334. The exact contract, the
lifecycle gate (G-6) and the model changes live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Consumed by [ADR-0267](adr-0267-a-startup-probed-allocation-is-a-backend-only-after-its-startup-has-passed.md)
(backend eligibility reads the committed `Stable` claim this decision hydrates).

## Context

`ServiceLifecycle` emits the `Stable` finalisation and, in the same pure `reconcile`
call, records the allocation in its persisted view's `stable_announced` set; the
top of its per-allocation loop skips every allocation in that set
(`service_lifecycle.rs:531`, `:555`, `:474-479`). The runtime persists the returned
view before it dispatches the actions and does not roll the view back when dispatch
fails (`reconciler_runtime.rs:1648-1652`, `:1682-1745`); a failed dispatch
re-enqueues the target, and the re-run skips the allocation. The shim's `Stable` arm
writes the row and then fires `on_alloc_stable` (`action_shim/mod.rs:2147-2150`), so a
failed row write fires no hook.

Result: a `Stable` row write that fails, or a control-plane restart between the view
persist and the dispatch, leaves the view claiming "announced" while the row carries
no `Stable`. Nothing ever announces it again. Before ADR-0261 this cost the
allocation its `Stable` event; with ADR-0261, which starts liveness from the `Stable`
hook, it also means liveness never starts for that allocation, a regression against
today's behaviour (liveness from `Running`).

The marker is the shape `.claude/rules/reconcilers.md` warns about: a record of what
the reconciler emitted, consulted as the diff, stamped before the effect is known to
have landed. ADR-0079 fixed the same defect for the backend row by reading the
managed resource back.

## Decision

**`ServiceLifecycle` decides whether an allocation still needs its `Stable`
announcement from the allocation's observed row: it announces `Stable` for a
`Running` allocation whose startup condition holds and whose row does not yet carry
a committed `Stable`.** The view's `stable_announced` marker is removed: after this
decision no code reads it (user ruling 2026-11-03, "remove it if no reader remains";
the readers are listed in the feature delta, D12 follow-up). Its two remaining
readers, the mid-startup-window predicate and the wake hook, read the hydrated
committed claim instead.

- The committed `Stable` claim is the existing terminal field of the allocation's
  status row, which the shim writes on the still-`Running` row. It is hydrated into
  the allocation's fact, so the decision stays a pure function of observed state.
- A `Stable` whose write failed is announced again on the next evaluation, which the
  runtime already schedules after a failed dispatch.
- Nothing is persisted that the row already records; no archived or wire change.

## Alternatives considered

- **Keep the marker; retry the row write in the shim.** The executor would decide
  that the reconciler's announcement stands, and a control-plane restart between the
  view persist and the dispatch would still lose it.
- **Roll the view back when dispatch fails.** Changes the runtime's fsync-then-
  dispatch ordering for every reconciler (ADR-0035) to repair one reconciler's
  marker, and does not cover the restart window.
- **The probe runner starts liveness when its own startup task passes.** Rejected in
  ADR-0261: it reimplements the `Stable` rule ahead of the committed row.
- **Accept the exposure.** The loss needs a store write error or a restart at the
  wrong instant, but its effect is permanent and silent: an allocation that never
  becomes `Stable` and never gets liveness.

## Consequences

- Positive: a lost `Stable` is re-announced; ADR-0261's promise that liveness starts
  for every allocation announced `Stable` holds across a write error and a restart.
- Positive: the `Stable` decision follows the reconciler rule: converge on the
  observed row, never on a marker of what was emitted.
- Negative: one new field on the allocation fact (an in-memory input), against one
  persisted view field removed (a CBOR `View` field; an older blob that still carries
  it decodes, serde ignores unknown fields).
- Negative: an unchanged `Stable` row is read on every evaluation instead of a set
  membership; the row is already hydrated for the allocation.
