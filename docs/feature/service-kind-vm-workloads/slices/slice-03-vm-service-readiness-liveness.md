# Slice 03 — VM Service readiness and liveness

**Story:** US-SVM-3
**Priority:** P2
**Effort:** 5 hours maximum
**Dependencies:** Slices 01–02

## Goal

Ana relies on continuing VM probe results to withdraw, restore, or restart the
guest Service under the same role semantics used by other drivers.

## IN

- TCP and HTTP readiness transitions drive existing backend eligibility.
- Liveness failure drives the existing restart policy without a VM-specific
  policy vocabulary.
- Termination cancels roles and prevents late results from restoring eligibility.
- Prove routing with real requests and render the transition in describe.

## OUT

- New thresholds, restart policy, or observation model.
- Multi-node behavior.
- Exec mechanic.

## Learning hypothesis

Failure disproves that the existing role-aware ProbeRunner and Service
reconciler remain driver-neutral once targets are guest addresses. Success
confirms continuous, not startup-only, VM Service honesty.

## Acceptance

- `inventory-vm` readiness changes 204 → 503 → 204; requests stop and resume
  within one interval plus timeout.
- `pricing-vm` liveness reaches the existing restart threshold and describe
  reports the resulting allocation lifecycle honestly.
- A result completing after workload termination never restores eligibility.

## Dogfood

Ana toggles a health file inside the production guest fixture while sending a
steady stream of byte-distinct mesh requests and observes the withdrawal gap.

## Reference class

Delivered Service readiness/liveness slices and backend-health fingerprinting,
with the driver target changed from host-local to guest-reachable.
