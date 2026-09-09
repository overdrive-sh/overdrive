# Slice 02 — HTTP VM Service probes

**Story:** US-SVM-2
**Priority:** P1
**Effort:** 4 hours maximum
**Dependencies:** Slice 01

## Goal

Ana uses a guest HTTP response, not merely an open guest port, to decide
whether a VM Service is ready for initial traffic.

## IN

- Apply guest default-target projection to existing HTTP probe semantics.
- Preserve 2xx-pass and 3xx/4xx/5xx-fail behavior.
- Render the bounded status-derived result in deploy and describe.
- Exercise the default-feature binary against a real guest HTTP server.

## OUT

- Redirect following or response-body capture.
- New HTTP client semantics.
- Readiness/liveness transitions after Stable.
- Exec probes.

## Learning hypothesis

Failure disproves that driver-aware targeting can reuse the existing HTTP
mechanic without semantic drift. Success confirms application-level guest
health parity.

## Acceptance

- `checkout-vm` returns 204 from `/ready`; deploy reaches Stable.
- `search-vm` accepts TCP but returns 503; deploy never reports Stable and names
  the HTTP failure.
- `profile-vm` returns 302; the probe fails without exposing an unbounded body.
- Describe and deploy agree on probe identity and latest result.

## Dogfood

Ana changes one real guest fixture from 503 to 204 and watches the same deploy
path change from Failed to Stable.

## Reference class

Delivered Exec-backed HTTP startup probes; Kubernetes node-to-Pod-IP probing
is semantic precedent for a host-originated guest-address target.
