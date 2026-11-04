# ADR-0271: The probe descriptor no longer records whether it was inferred

## Status

User-approved (option R), 2026-11-03; unaffected by V42 (held 2026-11-04, PR-2 accepted in
contract shape, [findings](../../feature/service-allocation-readiness-and-ending/spike/findings-workload-cgroup.md));
not yet Accepted (remaining gates: model check, design review round 4). Not implemented. GH #318, GH #334.
Contract and status: [feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md) (D16).
Depends on [ADR-0270](adr-0270-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md)
(which removes the inferred probe). Independently reversible: option K (keep the field,
set it for the new descriptor) remains the recorded alternative (feature delta § D16).

## Context

`ProbeDescriptor.inferred` marks the synthesised ADR-0058 probe and is carried to the
probe-result row, the `Stable` witness, the wire and the CLI. ADR-0058 §2 says no decision
uses it. With ADR-0270 no descriptor is inferred. The platform-supplied startup descriptor
is identified by its mechanic, so a flag would be `true` exactly when the mechanic is the
guest-listener one: derived state, and misnamed (the platform no longer guesses).

## Decision

**Remove `inferred` from the descriptor, the probe-result row, the `Stable` witness, the
wire schemas and the CLI render, in one cut, using the versioned-envelope procedure for
each archived carrier.** The carriers, the new versions and the fixtures are in the
feature delta (D16). The mechanic summary string carried by the witness identifies the
platform-supplied gate.

## Alternatives considered

- **Keep the field, set it `true` for the guest-listener descriptor.** No layout change,
  but the flag duplicates the mechanic and its name is false. Offered to the user as
  option K.
- **Keep the field, always `false`.** A dead field; the deletion discipline forbids it.
- **Rename it `platform_supplied`.** Layout-neutral for rkyv, breaking for the wire, and
  still derived from the mechanic.

## Consequences

- Five versioned carriers move together (feature delta D16); each gets a new envelope
  version and a historical fixture that contains a probe, because the existing intent and
  spec fixtures carry no probes and do not pin the layout the removal changes.
- `workload describe` no longer prints `(inferred)`; the probe line states its mechanic.
- Greenfield single cut (no dual-read period), per the project's migration policy.
