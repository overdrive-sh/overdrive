# ADR-0327 — Adding a substrate does not bump the methodology version

## Status

**Accepted (2026-11-19).** Approved by the user as PD-3. Refines the version rule of [ADR-0300](adr-0300-the-runner-owns-the-statistical-protocol-and-versions-it-as-the-methodology.md) and `benchmarks/methodology.md` (operative). DDD-3 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md).

## Context

`methodology.md:3-5` says "a change to any rule below bumps the version", and the validator requires `methodology == 1`. The guest substrate adds rules (guest isolation, eligibility as applied to a guest, a guest clock, vCPU marking). The committed native records state `methodology: 1`. The feature delta (PD-3) lists every sentence of `methodology.md` and shows that exactly two are reworded, each by making an existing sentence substrate-conditional without changing what it requires of a native run; the rest are unchanged or gain an addition for the new substrate.

## Decision

The methodology version stays 1. The version identifies the rules that produced a record's numbers; it bumps when a rule that already governed some record's production changes or is removed. A section for a new substrate, and a sentence reworded to name the substrate it applies to without changing its requirement for any substrate already registered, bump nothing: no record produced before can have been governed by the new text, because the previous runner refused to produce one. `methodology.md` states this policy in one sentence beside the version line. Every record, native or guest, states `methodology: 1`.

## Alternatives

- **Bump to 2 for all new records**: the validator accepts 1 and 2, and a native family re-run stamps 2 although none of its rules changed, which makes two identical native methods look different when records are compared. Rejected.
- **Bump to 2 for guest records only**: a version that depends on the substrate is a second version axis the schema has no field for. Rejected.

## Consequences

The sentence "a change to any rule below bumps the version" becomes "a change to or removal of a rule below bumps the version; an addition for a new substrate does not". A change to an existing rule (for example the replicate minimum) bumps the version under this sentence, and the validator's accepted set follows that change.

## Links

- [feature delta, DDD-3, PD-3](../../feature/guest-benchmark-substrate/feature-delta.md)
