# ADR-0342 — The API carries resource quantities as their canonical string

## Status

**Accepted** (2026-11-20). The user approved canonical strings only on the wire (decision 3a).

## Context

The submit body, the describe echo of the stored spec, and the per-allocation `resources` block of `GET /v1/allocs` carry the workload's
size as two integers (`cpu_milli`, `memory_bytes`). The operator's deploy output, `describe` and error text must show the same spelling the
operator wrote in canonical form (feature decision D-3), and `api/openapi.yaml` is the checked-in contract.

## Decision

**Every API surface that carries a workload CPU or memory carries it as the canonical string of its newtype (`"500m"`, `"128MiB"`), under the
keys `cpu` and `memory`. The OpenAPI schema for each is `type: string`. No numeric twin is carried.**

## Alternatives considered

- **The integer and the string side by side.** Rejected: two representations of one value can disagree, and the client would have to choose
  which to trust.
- **Integers on the wire; the CLI formats them.** Rejected: every client re-implements the canonical form, and the API shows a different
  spelling from the spec the operator wrote.
- **A structured object (`{"value": 128, "unit": "MiB"}`).** Rejected: a second grammar to validate for no gain over the string.

## Consequences

- Positive: the wire, `describe`, deploy output and error text show one spelling; `api/openapi.yaml` states the expected form in the field's
  example.
- Negative: an API client written against the integer shape breaks; there is no compatibility read (greenfield single cut).

## Links

- [Feature delta](../../feature/workload-resources-human-readable/feature-delta.md)
- [ADR-0341](adr-0341-workload-resources-carry-the-typed-quantities-from-spec-to-driver.md)
- GH #372
