# ADR-0112 — Exec-affected lifecycle evidence resets forward-only to V1

## Status

**User-approved on 2026-09-14; iteration-1 findings remediated; awaiting
independent DESIGN re-review.** The user explicitly directed a greenfield
single cut with no row/occurrence migration or compatibility because there are
no production users. This record is not implementation authority until the
review approves the complete #293 design bundle.

## Context

Current lifecycle evidence has two Exec-coupled nested vocabularies:

- `AllocStatusRowEnvelope` V1/V2/V3 payloads contain
  `reason: Option<TransitionReason>`; current V3 crash facts can carry the same
  reason. `TransitionReason` contains Exec-only start failures and an unused
  Exec OOM promise.
- `AllocLifecycleOccurrenceRowEnvelope::V1` contains the same reason plus
  `source: TransitionSource`; `TransitionSource::Driver(DriverType)` can carry
  `DriverType::Exec`.

Current-code search finds no other persisted envelope containing
`TransitionReason` or `DriverType`. `stderr_tail`,
`WorkloadCrashedImmediately`, `DriverInternalError`, `StoppedBy::Process`, and
the current VM/cgroup fields are not Exec-only: the VM lifecycle produces or
consumes them.

## Decision

Delete the Exec-only lifecycle vocabulary:

- `DriverType::Exec`;
- `TransitionReason::{ExecBinaryNotFound, ExecPermissionDenied,
  ExecBinaryInvalid, CgroupSetupFailed, OutOfMemory}`;
- their conversion, rendering, OpenAPI, and test arms.

Then reset exactly the two affected lifecycle envelope owners:

- collapse the current `AllocStatusRowV3` field set, minus the removed nested
  variants, into a new sole `AllocStatusRowV1` and a sole
  `AllocStatusRowEnvelope::V1`;
- keep the current occurrence fields but regenerate the sole
  `AllocLifecycleOccurrenceRowV1`/envelope V1 against the reduced reason/source
  enums;
- delete all prior row payload types, conversions, discriminant history,
  fixtures, and backwards readers;
- add only new V1 golden fixtures for the post-cut shapes.

No historical label, deprecated variant, translation, negative old-byte
fixture, or observation migration remains.

## Alternatives considered

### Preserve Exec labels for forensic history

Rejected by the user. With no production users there is no evidence to
preserve, and passive labels would keep legacy wire/schema surface alive.

### Append new row/occurrence versions

Rejected by the user. Appending would require the exact backwards readers and
conversion branches the greenfield ruling forbids.

### Delete generic process/VM lifecycle fields by association

Rejected. VM exit reporting uses generic crash, process-origin, stderr-tail,
and driver-internal vocabulary. Their production reachability makes them
current VM contract, not Exec compatibility.

## Consequences

Positive:

- Current lifecycle wire and stored schemas contain no Exec driver/reason
  label.
- The row model retains only fields reachable from the surviving VM lifecycle.
- Only two lifecycle envelope owners reset; unrelated observations remain
  byte-stable.

Negative:

- Existing V1/V2/V3 schema-evolution fixtures and conversion tests for
  allocation rows are deleted and replaced by one new V1 baseline.
- OpenAPI clients see a breaking reduction in `DriverType` and
  `TransitionReason` variants.

## Links

- [GH #293](https://github.com/overdrive-sh/overdrive/issues/293)
- [Exact feature contract](../../feature/remove-legacy-exec-workload-driver/feature-delta.md)
- [ADR-0033](adr-0033-alloc-status-snapshot-enrichment.md)
- [ADR-0078](adr-0078-crash-and-recover-is-durably-observable-last-terminated-plus-restart-count.md)
