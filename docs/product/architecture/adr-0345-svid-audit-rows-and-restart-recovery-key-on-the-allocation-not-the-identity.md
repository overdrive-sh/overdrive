# ADR-0345 — SVID audit rows and restart recovery key on the allocation, not the identity

## Status

Accepted — approved by the user 2026-11-21 (D2).

## Context

Three consumers attribute a certificate to an allocation by comparing SPIFFE IDs,
because the per-allocation ID of the time made that sound
([ADR-0344](adr-0344-a-workload-svid-names-the-workload-not-the-allocation.md)
removes the allocation from the ID):

- `SvidLifecycle` restart recovery (`ever_issued`, ADR-0067 D10) asks whether an
  allocation was issued before by testing its derived SPIFFE ID against the
  `issued_certificates` identity set.
- `alloc status` joins issued certificates to an allocation by SPIFFE ID.
- `SvidLifecycle` selects the held SVIDs of its target workload by comparing the
  held identity with the derived one.

The `issued_certificates` audit row carries no allocation. With a shared
identity, matching by identity would credit one allocation's issuance to every
sibling, so a fresh replacement would look as if it had already been issued and
skip its first issue.

## Decision

Every `issued_certificates` audit row records what the certificate was issued
for: an allocation (its `AllocationId`) or the gateway. Restart recovery asks
whether an `AllocationId` was issued before. `alloc status` lists the
certificates whose row names that allocation. `SvidLifecycle` selects held SVIDs
by the identity's workload and attributes them by the `AllocationId` that already
keys the held set. No consumer infers the allocation from an identity.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Keep matching by SPIFFE ID | Wrong once the identity is shared: one allocation's row credits all siblings. |
| Add `alloc_id` as an optional field, `None` for the gateway | A sentinel carrying meaning; the gateway case is a distinct subject, not an absent allocation (`.claude/rules/rust.md` "Sum types over sentinels"). |
| Derive the allocation from the certificate serial at read time | The serial does not encode the allocation; it would need a second index. |

## Consequences

- The audit row schema gains a field, so the persisted row takes a version bump
  under `development.md` "rkyv schema evolution". Greenfield single cut: rows written
  by a prior binary are discarded with the data directory, no reader translates a
  per-allocation identity, and a prior-version row that is still read is an
  envelope error that the observation read logs and skips.
- The restart marker is evaluated per allocation, which is what ADR-0067 D10
  always intended.

## Links

- [feature-delta](../../feature/workload-spiffe-identity/feature-delta.md)
- [ADR-0067](adr-0067-workload-identity-manager-svid-lifecycle.md) D10, [ADR-0063](adr-0063-built-in-ca-port-trait-and-root-key-protection.md)
