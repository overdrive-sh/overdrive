# Slice 01 — TCP VM Service walking skeleton

**Story:** US-SVM-1
**Priority:** P0
**Effort:** 6 hours maximum after spike evidence
**Dependencies:** GH #42, GH #222, delivered Service probe lifecycle

## Goal

Ana deploys one VM-backed Service with an explicit TCP startup probe, observes
a truthful Stable/Failed result, and reaches a healthy guest through the real
mesh path.

## IN

- Admit `[vm]` plus `[service]` only when the production VM Service path is wired.
- Resolve omitted/wildcard TCP probe host to the allocation `workload_addr`.
- Preserve explicit-host semantics.
- Drive the existing startup result, backend eligibility, describe render, and
  one real byte-distinct mesh response.
- Use the built default-feature `overdrive serve` and `overdrive deploy`.

## OUT

- HTTP and Exec mechanics.
- Readiness/liveness transitions after Stable.
- New network topology or mesh proxy behavior.
- Any test-installed target, route, or dataplane rule missing in production.

## Learning hypothesis

Failure disproves that GH #222's delivered guest route/intercept can carry the
existing host-originated Service probe and mesh request through the production
VM path. Success confirms the minimum VM Service loop.

## Acceptance

- `payments-vm.toml` binds guest TCP 8443; deploy reaches Stable only after the
  guest probe passes and a mesh client receives `payments-vm/ready`.
- `ledger-vm.toml` never binds 7443; deploy exits nonzero with a named TCP
  failure and the backend receives no requests.
- `workload describe payments-vm` renders the same probe identity/result.
- Native-metal run uses production addressing and leaves no test-only wiring.

## Dogfood

Ana runs both specs and the mesh request on the same native-metal host in one
session, then fixes the bad listener and redeploys.

## Reference class

Existing Exec-backed default-TCP Service walking skeleton plus GH #222's
delivered VM egress/inbound mesh route.

## Pre-slice spike

Prove H6 from the research plan: a host-originated SYN reaches the guest
`workload_addr` through the exact production TAP/netns route. Timebox: 2 hours.
