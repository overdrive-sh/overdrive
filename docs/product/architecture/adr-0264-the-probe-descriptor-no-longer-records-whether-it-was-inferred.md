# ADR-0264: The probe descriptor no longer records whether it was inferred

## Status

**User-approved in direction, 2026-11-03 ("add it. and proceed with recommendations"):
option R (remove `inferred` entirely, with the versioned-envelope procedure for each
carrier). The spikes of the decisions it depends on (V34 to V39) have run (V39 partial);
the corrected D13 contract shape was ACCEPTED by the user on 2026-11-03. Not yet Accepted as a
decision: "user-approved, pending the V41 boot-cost measurement and the independent
re-review"** (a material B-minus-A delta returns D13 to DESIGN). Not implemented. GH #318, GH #334. Depends on
[ADR-0263](adr-0263-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md)
(which removes the inferred probe). Independently reversible: option K (keep the field,
set it for the new descriptor) remains the recorded alternative (feature delta § D16).

## Context

`ProbeDescriptor.inferred` marks the synthesised ADR-0058 probe and is carried to the
probe-result row, the `Stable` witness, the wire and the CLI. ADR-0058 §2 says no decision
uses it. With ADR-0263 no descriptor is inferred. The platform-supplied startup descriptor
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
