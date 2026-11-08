# ADR-0281 — Metal invocation completion follows owned-resource cleanup

## Status

**Accepted — 2026-11-08.** The user explicitly stated “Approve P1–P5”. Independent technical review is APPROVED; the exact accepted contract is in the feature delta.

## Context

A supported nextest cancellation can end a wrapper without Rust Drop. A saved
command status and the main process exiting do not alone establish the end of
its launcher descendants or ephemeral files. Collection must remain read-only
and repeatable after the controller disappears.

## Decision

Let the existing transient unit finalizer record invocation completion after
its existing stop lifecycle ends descendants and removes only that run's
ephemeral roots. Keep the actual command terminal distinct; require the retained
completion receipt when collecting new-format runs.

## Alternatives

A collector performing cleanup violates read-only repeat collection. Inferring
cleanup from parent exit can publish success too early. A new recovery
controller, pending-command-result protocol or resource allocation registry is
unnecessary.

## Consequences

Command status remains retained unchanged, and cleanup failure is a distinct
runner outcome. Missing command terminals remain honest supervisor loss. Old
records retain old collection semantics. This decision creates no reboot
recovery or automatic pruning promise.

## Links

- [Exact proposed completion/wire/error contract](../../feature/metal-run-isolation/feature-delta.md#wave-design--ref-completion-and-read-only-collection--p5)
- [Actual native owner-path construction evidence](../../feature/metal-run-isolation/spike/findings.md)
- [Independent admission decision](adr-0278-metal-ordinary-runs-share-the-maintenance-barrier.md)

- [Bounded P2/P5 formal protocol and pinned owner projections](../../feature/metal-run-isolation/feature-delta.md#wave-design--ref-formal-protocol-model)
