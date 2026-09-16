# ADR-0110 — Live workload execution is microVM-only while driver routing remains extensible

## Status

**User-approved on 2026-09-14; the corrected DESIGN was independently
`APPROVED` after overall review iteration 4 on 2026-09-14.** This record is
authoritative as part of the complete #293 DESIGN bundle.

## Context

Exec is a temporary host-process compatibility adapter. GH #293 removes it and
requires the surviving allocation lifecycle to remain driver-neutral for the
supported microVM execution family. The current application already has a
`Driver` port, tagged driver intent/runtime payloads, `DriverRegistry`, an
allocation-to-driver routing index, and one exit observer per composed driver.
ADR-0083 introduced those boundaries when the second driver arrived.

Collapsing the application back to a scalar `VmDriver` would remove one-arm
indirection today but would make the next unikernel or sandboxed-microVM adapter
repeat the same registry/composition migration. Retaining an Exec-shaped arm as
disabled/deprecated would keep the compatibility boundary GH #293 removes.

## Decision

Remove Exec from every **live** workload-driver surface: operator admission,
live intent aliases, action payload, concrete worker adapter, production
composition, action-shim branches, and active tests/examples.

The live parser grammar recognizes only the VM driver table. It retains no
special retired-Exec parser error, message, or presence branch; unsupported or
missing driver input follows existing ordinary generic parser/serde failures.
No replacement public API is introduced.

Retain the existing tagged driver unions and `DriverRegistry`/
allocation-driver-index/exit-observer ownership model. Their live set contains
only the current VM adapter until another explicitly designed microVM-family
adapter exists. Drivers continue to execute a supplied physical allocation;
they do not choose workload lineage, allocation identity, replacement policy,
or cleanup policy.

Preserve ADR-0083's existing capability-absence result: ordinary VMM absence
does not refuse the control-plane process and may leave the registry empty;
later VM execution follows the existing typed missing-capability result. A VMM
that is present but fails its existing probe still refuses startup. The removal
adds no second node-readiness state or capability fallback.

The exact live type, parser, action-shim, and composition contracts are owned by
the feature delta:
`docs/feature/remove-legacy-exec-workload-driver/feature-delta.md`.

This decision does not select or implement a shared-switch dataplane. The
current VM networking mechanism remains until GH #295 replaces it.

## Alternatives considered

### Collapse to a single VM driver field

Rejected. It is simpler only while exactly one microVM adapter exists, removes
the already-paid capability-composition boundary, and couples the next adapter
addition to another application-wide routing migration.

### Hard-code concrete VM dispatch in the action shim

Rejected. It duplicates the composition root's capability fact, reverses the
core-port/adapter dependency direction, and makes the control plane depend on a
worker implementation.

### Keep an inactive or deprecated Exec arm

Rejected. A disabled arm still keeps Exec in live unions, constructors, match
exhaustiveness, and tests. It is a compatibility path rather than removal.

## Consequences

Positive:

- `[exec]` cannot become live intent or a runtime driver payload.
- No concrete Exec adapter is composed or exported.
- Allocation identity and replacement remain driver-neutral.
- Existing stop/finalize routing and exit-observer ownership remain usable for
  future microVM-family adapters.
- No new component, port, action, store, daemon, or dependency is introduced.

Negative:

- One-arm tagged unions and a registry remain in the codebase until another
  microVM-family adapter exists.
- A node may be control-plane-live while its empty registry cannot execute any
  workload, preserving the prior optional-capability contract after Exec is
  gone.
- Broad tests that used Exec as a cheap fixture must migrate deliberately to
  the existing VM/simulation surface.
- The current VM network mechanism remains temporarily and is removed only by
  GH #295.

## Links

- [GH #293](https://github.com/overdrive-sh/overdrive/issues/293)
- [GH #295](https://github.com/overdrive-sh/overdrive/issues/295)
- [Exact feature contract](../../feature/remove-legacy-exec-workload-driver/feature-delta.md)
- [ADR-0083](adr-0083-driver-registry-and-per-driver-allocation-payload.md)
- [ADR-0105](adr-0105-driver-neutral-allocation-replacement-identity.md)
