# ADR-0271: The probe descriptor no longer records whether it was inferred

## Status

**Accepted, 2026-11-09, after the user's option-R approval and explicit
greenfield compatibility waiver, and independent full DESIGN review iteration 4**
([review](../../feature/service-allocation-readiness-and-ending/design/review.md)).
The exact five-carrier D16-G1 current-format policy is in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md)
(D16). The user said: “"preserve historical bytes" -> what do um ean? we dont
need to preserve anything yet. this is greenfield. not used in production. rest
is fine”. This waives only historical persisted-byte/backward-read compatibility
for this feature's pre-D16 layouts. The in-place current-format policy is the
bounded direct consequence of that ruling; the withdrawn historical migration
proposal was never accepted. Not implemented. GH #318, GH #334.
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

**Remove `inferred` from the descriptor, the probe-result row, the `Stable`
witness, the wire schemas and the CLI render in one cut.** All five affected
current carrier layouts must remain internally coherent, including the
allocation-lifecycle occurrence row. D16-G1 keeps their existing envelope
families, tags and latest aliases, rebaselining only the current payloads in
place without a historical read promise. The exact shapes, fixture treatment
and bounded two-step landing are in the feature delta § D16. The
mechanic summary string carried by the witness identifies the
platform-supplied gate.

## Alternatives considered

- **Keep the field, set it `true` for the guest-listener descriptor.** No layout change,
  but the flag duplicates the mechanic and its name is false. Offered to the user as
  option K.
- **Keep the field, always `false`.** A dead field; the deletion discipline forbids it.
- **Rename it `platform_supplied`.** Layout-neutral for rkyv, breaking for the wire, and
  still derived from the mechanic.

## Consequences

- Five current carrier layouts change under D16: intent, spec, probe-result
  row, allocation-status row and allocation-lifecycle occurrence row. Under
  accepted D16-G1, historical nested types,
  backward-read conversion chains and new historical fixtures are not required
  under the user's feature-specific greenfield waiver. Existing fixture byte
  literals and unrelated envelopes are not rewritten; current-layout evidence
  covers all five affected carriers.
- `workload describe` no longer prints `(inferred)`; the probe line states its mechanic.
- Greenfield single cut (no dual-read period), per the project's migration policy.
