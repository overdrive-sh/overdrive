# ADR-0357 — `overdrive.local` is a platform zone the node answers itself; one classifier over the decoded labels decides what is never forwarded

## Status

**Accepted** (GH #382, DESIGN decision GPD-2; approved by the user 2026-11-25, and reconfirmed unchanged under ruling D the same day: all of `overdrive.local` stays reserved).
Interface contract: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts* (zone and classification).

## Context

`wire::decode` folds "not a valid mesh name" and "outside the mesh namespace" into one `NotMeshName` outcome, so it cannot say which queries must stay on the node (D-6, AC-5: a malformed name under the mesh zone must never reach an upstream). `overdrive.local` is also the SPIFFE trust domain of every workload identity, so it is already Overdrive's namespace; `.local` itself is the mDNS special-use label (RFC 6762) and enterprise directories also use it.

## Decision

- The platform zone is `overdrive.local`, named by one const next to `MeshServiceName::SUFFIX`. Every name at or under it is answered by the node and never forwarded.
- One predicate classifies a decoded question into a valid mesh name (answered as today), an unknown platform name (NXDOMAIN with the zone's negative SOA), or forwardable. It compares the trailing labels case-insensitively, never a rendered string.
- Other `.local` names, and every other zone, are forwardable.
- A malformed or unknown name under the zone gets NXDOMAIN, a definite answer and not a 5 s wait.
- The same classification applies only to datagrams addressed to the gateway. A non-gateway-addressed query keeps today's behaviour (mesh names answered, anything else dropped).

## Alternatives considered

- **Reserve only `svc.overdrive.local`.** Smallest, but `overdrive.local` names outside `svc.` would go to an upstream, leaking platform vocabulary and letting an upstream answer for the trust domain. Rejected.
- **Reserve all of `.local`.** Breaks enterprise directory names that resolve through the upstream. Rejected.
- **Keep silence for malformed names.** Reproduces the 5 s symptom for a typo and contradicts the never-silent rule the feature adopts for names it answers. Rejected.
- **String suffix match on the rendered name.** Misclassifies escaped dots and labels such as `evil-overdrive.local`. Rejected.

## Consequences

- Positive: AC-5 holds for every platform-zone name from one predicate; the forwarded namespace boundary is a single reviewable place.
- Negative: a real external zone literally named `overdrive.local` would not resolve; judged not to exist (`.local` is unregistered).
- The mesh answer contract (ADR-0072 DDN-8) is unchanged. ADR-0072's amendment holds the old and new contract.
