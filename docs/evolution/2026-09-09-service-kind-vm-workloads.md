# Evolution — service-kind-vm-workloads (GH #257)

**Finalized:** 2026-09-09. **Feature slug:** `service-kind-vm-workloads`.
**Waves:** SPIKE → DESIGN → DISTILL → DELIVER → FINALIZE.
**Completion record:** [roadmap](../feature/service-kind-vm-workloads/deliver/roadmap.json), [execution log](../feature/service-kind-vm-workloads/deliver/execution-log.json), and the nine step review artifacts under `docs/feature/service-kind-vm-workloads/deliver/`.

---

## What shipped

Overdrive Services now admit the existing VM driver union for host-originated
HTTP and TCP health probes. A VM Service's omitted or wildcard HTTP/TCP target
is resolved once, at probe registration, to that allocation's provisioned guest
address; explicit targets remain unchanged. VM Exec probes remain rejected
before intent commit and are deferred to GH #280.

The feature preserves lifecycle ownership: VM driver/beacon ownership retains
`Running`; `ServiceLifecycle` owns startup/readiness/liveness policy and
backend health; `WorkloadLifecycle` alone decides restart versus finalization.
The operator surface now faithfully presents accepted Service streams, startup
failure eligibility withdrawal, readiness traffic withdrawal and recovery, and
the current `terminal: Stable` claim on a running Service allocation.

## Permanent decisions and product artifacts

The accepted architecture is already at its permanent repository home; no ADR
was copied during finalization. The feature's decision sequence is:

- [ADR-0090](../product/architecture/adr-0090-vm-service-network-probe-target-projection.md) and [ADR-0091](../product/architecture/adr-0091-service-parser-driver-union-and-vm-exec-exclusion.md): guest-targeted HTTP/TCP probes and atomic ServiceSpecV3 driver-union ingress.
- [ADR-0092](../product/architecture/adr-0092-marked-host-http-probe-connector.md), [ADR-0093](../product/architecture/adr-0093-streaming-service-accepted-render.md), [ADR-0094](../product/architecture/adr-0094-marked-host-tcp-probe-sockets.md), and [ADR-0095](../product/architecture/adr-0095-service-stream-cap-startup-deadline.md): truthful host-originated probes and bounded streaming presentation.
- [ADR-0096](../product/architecture/adr-0096-startup-failure-withdraws-backend-eligibility.md) through [ADR-0101](../product/architecture/adr-0101-service-backend-health-observed-convergence.md): terminal eligibility withdrawal, probe-result observation/wake convergence, network cleanup, restart acknowledgements, VM exit watcher session ownership, and observed backend health.

The durable product artifacts also remain at their normal homes: checked-in
operator examples in `examples/service-kind-vm-workloads/`, executable
black-box expectations in `verification/expectations/`, and the VM probe
research in `docs/research/`.

## Delivery record

All nine roadmap steps completed their final COMMIT phase with `PASS`. Step
`02-04` was an approved verification-only closure; its RED and GREEN entries
are explicitly `NOT_APPLICABLE`, rather than omitted implementation work.

| Step | Delivered outcome | Completion |
|---|---|---|
| 01-01 | Atomic direct ServiceSpecV3 ingress and Service VM driver admission | 2026-09-06 |
| 01-02 | VM HTTP/TCP target projection and marked host-originated HTTP probing | 2026-09-06 |
| 01-03 | Shared VM probe supervision in the production driver composition | 2026-09-06 |
| 02-01 | Marked TCP probe sockets and VM probe behavior coverage | 2026-09-06 |
| 02-02 | Service walking skeleton, accepted-before-stable stream, and startup behavior | 2026-09-06 |
| 02-03 | Observed Service backend-health convergence and approved bridge retirement | 2026-09-08 |
| 02-04 | E09-v2, E10, and E13 expectation/evidence closure | 2026-09-09 |
| 03-01 | Readiness probe-result wake, traffic withdrawal/recovery, and current Stable observation | 2026-09-09 |
| 03-02 | Liveness threshold terminal/restart observation | 2026-09-09 |

Each roadmap step has a final `APPROVED` review disposition in its native
Markdown review artifact. Separate different-fox expectation audits retained
the E08, E09-v2, E11, and E12 evidence dispositions; the canonical executable
catalogue remains under `verification/expectations/` rather than being copied
here.

## Evidence and validation disposition

The approved roadmap records E08, E09-v2, E10, E11, E12, and E13 as completed
native product evidence. In particular, the final E11 native-metal capture
observed the current `terminal: Stable` claim in every before/during/after
describe response, running state with zero restarts, correct reachable /
unreachable / reachable peer behavior, and zero teardown delta.

Mutation testing was explicitly skipped by the user during FINALIZE. No
mutation run or mutation-evidence artifact is claimed by this record.

## Lessons retained

- Project probe intent and effective runtime target at the existing
  registration boundary; do not rewrite intent or introduce a lookup registry
  for allocation facts already present there.
- Preserve lifecycle gate ownership. A failed guest probe is a probe result;
  it does not make the VM driver's `Running` state a probe-owned state.
- A durable accepted probe result needs the existing observation-to-broker wake
  path when Service backend health must converge within the product's bounded
  window. The seeded simulation witness and native run established that need.
- Native product evidence and in-process tests have distinct jobs. The former
  drove checked-in examples through the built binary; the latter covered owner
  paths, ordering, and state invariants without replacing the black-box oracle.
- Spike harnesses remain evidence, not production surface. The discarded
  VM-Exec feasibility work informs the separately scoped GH #280 and grants no
  API or mechanism to this feature.

## Finalization migration and workspace disposition

The finalization destination map was evaluated. No temporary
`architecture-design.md`, `component-boundaries.md`, `technology-stack.md`,
`data-models.md`, `walking-skeleton.md`, feature-local `design/adrs/`, or
DISCUSS journey artifact exists for migration. Therefore no duplicate permanent
artifact was created.

The feature workspace is intentionally preserved as historical wave evidence.
Only session markers or temporary files may be removed after explicit user
approval; no such file was found at the time this evolution record was created.
