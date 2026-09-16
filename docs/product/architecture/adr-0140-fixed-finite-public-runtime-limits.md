# ADR-0140 — Use one fixed finite first-slice public runtime limit policy

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The unauthenticated public edge must bound connections, requests, HTTP heads,
bodies, cleanup state and phase deadlines. The first slice has no measured
capacity basis or product requirement for operator-specific tuning.

## Decision

Use one private, fixed first-slice gateway policy with finite ceilings for
every resource and deadline category consumed by public runtime, connection
admission, cleanup and gateway-client handshake. Construct it once at the
composition root and expose no operator configuration, default override or
second limit source.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Leave parser/body/concurrency limits unbounded | Public clients could consume memory, tasks or sockets without a finite ceiling. |
| Add operator knobs in the first slice | No validated tuning contract exists, and multiple values would expand configuration/error scope. |
| Derive adaptive limits from current load | It adds policy state and feedback behavior without a first-slice requirement or selected control model. |

## Consequences

- The exact values and accessors live only in the feature implementation
  contract, not this ADR.
- Changing one ceiling does not require revisiting HTTP routing or streaming
  architecture.
- The values are safety policy, not claimed measured capacity or KPI.

## Links

- [Exact limit policy](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 public runtime](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [HTTP/1.1 ADR-0137](adr-0137-public-http11-only.md)
- [Streaming ADR-0141](adr-0141-stream-public-proxy-bodies-with-backpressure.md)
