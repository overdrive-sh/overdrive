# ADR-0251: The restart authority names a pre-startup crash loop at give-up

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D6 and the K1
restatement). GH #318, GH #334. Not implemented. The exact contract, the lifecycle
gate (G-4), the formal protocol model and the DISTILL obligations live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).

The crash classes that count as a pre-startup crash were approved 2026-11-03 (feature
delta § Approval record, B3) and are part of the Decision below. The consequence of
deleting the early-exit announcement for a crash successor's backend eligibility is
decided by [ADR-0260](adr-0260-a-startup-probed-allocation-is-a-backend-only-after-its-startup-has-passed.md)
(a startup-probed allocation is a backend only after its own `Stable`).
Companions: [ADR-0250](adr-0250-service-lifecycle-announces-a-startup-failure-only-for-a-live-allocation.md)
(`ServiceLifecycle` does not finalise a terminal row; this ADR can be adopted or
reversed without it only together with a replacement rule for the early-exit
branch), [ADR-0248](adr-0248-the-component-that-observes-an-allocations-end-fires-the-terminal-hook.md),
[ADR-0252](adr-0252-a-startup-probe-failure-stops-the-allocation-and-the-restart-authority-decides.md)
(the same give-up classification point, for a startup-probe failure).
Related: ADR-0087 (single restart authority), ADR-0055 (the service lifecycle
reconciler; this ADR replaces its early-exit clause), ADR-0078, ADR-0105.

## Context

`ServiceLifecycle` has a branch that classifies a `Failed` row as an early exit
when the allocation reached `Running`, no startup probe passed, and the
observation falls inside the startup deadline. It emits a genuine-terminal
finalisation, `ServiceFailed { EarlyExit }`, and the action shim runs the
terminal teardown for that allocation (mTLS stop, network teardown, driver-index
removal, terminal hook) and writes the terminal claim.

`WorkloadLifecycle` then still restarts the same allocation. Its restart
selection ignores the row's terminal claim, and only `BackoffExhausted` and
`LivenessProbeFailed` stop it at the ceiling. So two reconcilers author the
ending of one crashed allocation with no ordering: one finalises it, the other
places a fresh successor under the shared budget. ADR-0087 makes
`WorkloadLifecycle` the only restart authority and keeps `ServiceLifecycle` to
readiness, membership and liveness detection; the early-exit branch is the one
place where `ServiceLifecycle` still decides the ending of a crashed allocation.

The branch exists for an operator-facing reason (KPI K1): a Service that dies
before any startup probe passes must read as a failure naming that fact, never
as `Stable` and never as a bare `Running`. That reason survives; the question is
who may say it.

The restart authority already makes its give-up terminal cause-aware. For a
liveness kill it selects `ServiceFailed { LivenessProbeFailed }` from the
observed row instead of `BackoffExhausted` (ADR-0087 D4). Every input the
early-exit classification needs is durable and readable by the restart
authority: the crash row (state, start time, typed crash reason), the workload's
declared startup probes (intent, already hydrated by `WorkloadLifecycle`), and
the startup probe rows of the allocation (observation, which outlive the
allocation and are read by `ServiceLifecycle` today). The restart authority
does not read the probe rows today.

Each restart is a fresh allocation identity (ADR-0105) and the predecessor's row
stays in the store. So every crashed attempt is already a durable record of its
own occurrence, with its crash reason, start time and stderr tail; the occurrence
does not depend on a field a successor overwrites.

## Decision

**`WorkloadLifecycle`, the single restart authority, names a Service
allocation's ending when it gives up on a crash loop whose final attempt crashed
before any startup probe passed: the terminal is `ServiceFailed { EarlyExit }`,
carrying the crash's exit code. `ServiceLifecycle` has no crash-classification
branch and acts on no terminal row.**

- The classification is evaluated only where `WorkloadLifecycle` already decides
  to give up at the restart ceiling. It never delays, advances or replaces a
  restart, and it consumes no additional budget.
- The terminal at the ceiling is selected from the observed row, in this order:
  a liveness kill gives `LivenessProbeFailed` (unchanged), a final attempt that
  reached `Running`, crashed, and never passed a declared startup probe gives
  `EarlyExit`, anything else gives `BackoffExhausted` (unchanged).
- The crash classes are the two guest-crash reasons the exit observer writes,
  `WorkloadCrashedImmediately` and `VmOutOfMemory` (an out-of-memory crash loop gives
  up as `EarlyExit`, with no exit code because the observer carries none for it). The
  fail-closed supersessions of a `Running` row (mTLS intercept, guest-network
  activation, a failed cleanup) are not guest crashes and are not classified
  (ADR-0083 D6).
- The classification reads durable inputs at the moment of the decision. It is
  not persisted on the crash row, so no row, wire or View format changes.
- A Service that declares no startup probe (the explicit opt-out) is never
  classified as an early exit, because its first `Running` is its `Stable`.
- `ServiceLifecycle` loses the early-exit branch. A terminal row ends its
  startup-window watch without any action.

## Alternatives considered

- **The exit observer classifies the crash when it writes the row.** The
  observer does not hold the workload's startup-probe declaration (intent) and
  would need a new intent dependency. It would persist a derived classification
  on the row (a new reason variant embedded in the archived row, forcing an
  envelope bump), against "persist inputs, not derived state". It would still
  leave the give-up terminal to someone, so it only moves the classification
  input, not the ending.
- **`ServiceLifecycle` keeps classifying, as a verdict that does not finalise.**
  No such surface exists: its only requests against an allocation row are
  finalise and stop. A new annotation action writing `alloc_status` would make
  `ServiceLifecycle` the author of a row family it declares interest in, which
  `.claude/rules/reconcilers.md` forbids (the no-busy-loop rule); a new
  observation row kind adds a store family, an envelope and a gossip surface for
  one bit that the restart authority can derive.
- **Carry nothing; let the give-up stay `BackoffExhausted` and rely on the
  per-attempt rows.** Correct and the smallest change, but it drops the K1
  reason: the operator learns "attempt budget exhausted, last exit code 1" and
  has to open `workload describe` to learn that no startup probe ever passed.
  The give-up is the one terminal an operator sees on `overdrive deploy`, so the
  honesty target needs the reason there.
- **Give up at the first pre-startup crash instead of restarting.** The only
  alternative that keeps K1's time bound literally (`startup_deadline` plus one
  tick). It is a restart-policy change: a Service that crashes at boot because a
  dependency is not up yet stops being retried. It is also not today's
  behaviour, where such an allocation is finalised and then still restarted. It
  changes what the platform does, not who says what, so it is not taken without
  the user's decision.
- **Enrich the give-up with the attempt count (a new field on the early-exit
  reason).** The count is available from the attempt rows and the restart
  budget. The field changes an archived variant, so it forces the versioned
  envelope procedure, a wire change and a regenerated API spec, for information
  the operator can already read.

## Consequences

- Positive: one author for every ending of a crashed allocation. `ServiceLifecycle`
  finalises only a live allocation (ADR-0250) and never a crashed one. The
  early-exit finalisation's teardown no longer races a restart of the same
  allocation. The `Failed`-gated fabrication hazard recorded in ADR-0083 D6
  (a reclamation row written as `Failed` would have been classified as an early
  exit) is removed: the classification runs only at the ceiling, after the
  platform-reclamation exemption, and only for a `Failed` crash row.
- Positive: a service that crashes at boot and is restarted into health reads as
  `Stable` on the stream, not as `Failed` followed by a silent recovery.
- Negative, a changed target: the early-exit terminal arrives when the restart
  authority gives up, not at the first crash. With the current budget (five
  restarts, one second apart) that is about five seconds plus each attempt's time
  to crash, not `startup_deadline` plus one tick. A workload that takes long to
  crash can exhaust the stream's cap timer (90 s) before the terminal arrives; the
  stream then ends with its own timeout terminal. The user approved this
  restatement of the target (feature delta § Approval record); § Changed and
  unchanged assumptions quotes the original.
- Negative: the early-exit terminal no longer records that the platform retried.
  The operator reads the attempts in `workload describe` (one row per attempt)
  and the restart budget it reports.
- Neutral: no archived, wire or persisted format changes; the early-exit reason
  is an existing variant, now authored by a different reconciler.
- Neutral: the `Terminated` exit-0 case is unchanged. A clean exit lands as
  `Terminated`, which the current branch never classified, so a Service that
  exits 0 before startup still gives up as `BackoffExhausted`.
