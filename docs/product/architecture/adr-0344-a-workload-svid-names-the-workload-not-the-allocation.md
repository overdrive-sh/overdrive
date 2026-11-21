# ADR-0344 — A workload SVID names the workload, shared by all its allocations

## Status

Accepted — approved by the user 2026-11-21 (D1). Supersedes the identity-shape part of
[ADR-0067](adr-0067-workload-identity-manager-svid-lifecycle.md) D5 and
[ADR-0075](adr-0075-bucket-c-wire-identity-rename-v1-jobs-routes-and-spiffe-job-segment.md)
(the `/workload/<w>/alloc/<a>` shape). Those ADRs keep their text; the rest of
each stays in force.

## Context

The workload SPIFFE ID is `spiffe://overdrive.local/workload/<w>/alloc/<a>`.
The `/alloc/` segment was copied from a whitepaper example and was never
decided: ADR-0067 D5 consolidated two private helpers that already emitted it,
and ADR-0075 renamed `/job/` to `/workload/` mechanically. Nothing weighed
whether an identity should name an instance.

The shape has a cost. A replacement allocation of the same workload (crash
restart, liveness restart, serve-restart reclaim) gets a different SPIFFE ID, so
every consumer that compares or memoises identities treats a normal replacement
as a different party. The public ingress gateway answered 502 for the whole
recovery window after a serve restart for this reason
([RCA](../../analysis/root-cause-analysis-public-ingress-gateway-502-after-serve-restart.md)).
Every surveyed identity system expects workload-grain identity: SPIFFE defines an
ID as naming a workload that "may comprise multiple running instances", and
Istio, Linkerd, Consul and Cilium bind the expected peer identity to a service
account, service or label set, never to a pod instance
([research](../../research/dataplane/backend-identity-on-endpoint-reuse-comprehensive-research.md)).

Workloads hold no SVID material and the kernel terminates mTLS
(CLAUDE.md "Workload identity model"); `IdentityMgr` already keys every held
SVID by `AllocationId`. The SAN is therefore the only place the allocation is
encoded into identity.

## Decision

The SPIFFE ID of a workload SVID is `spiffe://overdrive.local/workload/<w>`. All
allocations of workload `<w>` share it. Each allocation still receives its own
SVID with its own key, serial and rotation, held by `IdentityMgr` under its
`AllocationId`. The gateway identity `spiffe://overdrive.local/gateway/<node>`
and the trust domain are unchanged. The allocation is never encoded in an
identity; where a consumer needs per-allocation distinction it carries an
explicit `AllocationId` ([ADR-0345](adr-0345-svid-audit-rows-and-restart-recovery-key-on-the-allocation-not-the-identity.md)).

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Keep per-allocation IDs; the gateway and dataplane compare only the workload part of the path | Couples every verifier to a path convention SPIFFE leaves to the administrator, and leaves the per-allocation ID as the root cause for the next consumer. |
| Keep per-allocation IDs; mint a fresh `BackendId` per replacement | Repairs the gateway failure only, keeps a per-instance expectation bound to an address slot, and no surveyed system does it. |
| Keep per-allocation IDs and add a workload-level identity beside them | Two identities per SVID; the verifier would still have to choose, and the choice is this decision. |

## Consequences

- A replacement allocation presents the same identity as its predecessor, so
  identity equality no longer distinguishes instances. Instance attribution uses
  the certificate serial and the `AllocationId`.
- An allocation that holds a valid SVID is indistinguishable by SAN from its
  sibling replicas. This is the SPIFFE model: authorisation is per workload.
- `SpiffeId::for_allocation` and every parse of the `/alloc/` segment go away in
  one cut (greenfield, no compatibility path). Stored rows that still carry
  per-allocation identities are discarded with the data directory.
- Intended-peer pinning for transparent mTLS (GH #242) pins a workload identity;
  this decision removes the allocation dimension it would otherwise need.

## Links

- [feature-delta](../../feature/workload-spiffe-identity/feature-delta.md)
- [ADR-0345](adr-0345-svid-audit-rows-and-restart-recovery-key-on-the-allocation-not-the-identity.md), [ADR-0346](adr-0346-gateway-client-mtls-requires-the-selected-backends-workload-identity.md), [ADR-0347](adr-0347-dataplane-backend-identity-association-carries-the-workload-identity.md)
