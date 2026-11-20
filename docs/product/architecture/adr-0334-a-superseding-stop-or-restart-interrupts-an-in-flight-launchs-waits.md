# ADR-0334 — A stop, a restart or node shutdown interrupts an in-flight launch's waits

## Status

**Accepted** 2026-11-18 (decisions SD-1 to SD-5 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-18, option B, and
the user's approval of the exact decisions, including node shutdown as a producer). ADR-0102 is implemented, so this ADR amends it explicitly:
the amendment is stated below and is recorded in ADR-0102's own amendment section. This ADR is not implemented, so its text is stated in the
present tense. It depends on one rule it does not own: a launch abandoned by supersession leaves its generation pending until its successor
is observed ([ADR-0335](adr-0335-a-restart-generation-is-handled-when-its-successor-is-observed.md)).

## Context

ADR-0102 gives each workload one convergence lane: a stop or restart evaluated for a workload whose launch is in flight waits until the
launch returns, and a duplicate submission "never cancels an active effect". The volume launch adds a 30 s `lvm` `ensure` and a 30 s claim
poll to the existing 30 s guest-boot wait, so an operator `workload stop` can wait about 90 s, eight such launches hold every slot, and
node shutdown, which drains every admitted evaluation to its result, waits as long. The claim poll and the boot wait are waits, not effects
in flight. Kubelet cancels the in-flight sync on a termination request and fences new starts; Nomad cancels blocking prestart hooks and
checks the kill context at step boundaries; systemd replaces the pending start job
(`docs/research/orchestration/stop-during-start-comprehensive-research.md`). `.claude/rules/reconcilers.md` § "Superseded in-flight work
runs to its bound" already permits cancellation at a step boundary through a cooperative token and requires a superseded result never to
be delivered; it forbids dropping a future mid-effect. A launch leaves a row-less state (the shim writes no allocation row before
`driver.start` returns), so no stop can act on it other than through the launch itself.

## Decision

**A stop or restart commit, or the start of node shutdown, raises a signal on the convergence lane of the evaluation it concerns. A launch
running in that lane observes the signal at its step boundaries and inside its two waits (the claim poll and the guest-boot wait), abandons
them at once, tears down what it created, and returns a typed "superseded" outcome. The shim writes no allocation row for that outcome and
charges nothing. An effect already running is never interrupted: an `lvm` command in `ensure` and `Vmm::create` run to their own end.**

- The convergence owner opens the lane before hydration. A lane's token is a child of one shutdown token owned by the runtime's lane table.
  The `stop_workload` and `restart_workload` handlers raise one lane's token after their intent write commits; node shutdown cancels the
  shutdown token once, from the convergence loop's shutdown arm, which every graceful shutdown path reaches. A lane opened at any instant
  around that cancel is signalled by it or born signalled, with no "list the lanes, then signal them" step.
- A core port carries the signal to the driver as a required parameter of `Driver::start` (all drivers).
- A launch that has observed the signal returns "superseded" even when it finds a typed failure at the same point; a launch whose boot race
  already succeeded is an ordinary success.
- The volume steps run before the first host artifact.
- A launch superseded by shutdown is torn down through the same path as one superseded by a stop and writes nothing. An abandoned launch
  leaves its generation pending until its successor is observed (ADR-0335), and a first placement is placed again because no allocation is
  running.
- A superseded restart successor still runs the exact-old predecessor cleanup (ADR-0106) through the restart arm's result path; a superseded
  launch never skips it.

Exact shapes: feature delta § "Superseding a launch", § "Node shutdown raises the signal" and § "Gate G-3".

**Amends ADR-0102 § "Admission, coalescing and fairness contract"** (implemented): "it never cancels an active effect" becomes "a duplicate
submission never cancels an active effect; a stop or restart commit, or the start of node shutdown, raises the lane's supersession signal,
which a driver's `start` observes at its own step boundaries and waits and which no other submission raises". The rest of ADR-0102 is
unchanged: no future is aborted or dropped, shutdown still disables admission and drains every admitted evaluation to its result (a
launch's result is now `StartSuperseded` instead of its own bound), admission remains FIFO with no reserved stop slot, and
`run_convergence_tick` and `spawn_convergence_loop` keep their signatures. ADR-0317's consequence "a stop or restart is evaluated only after
the launch ends" is replaced by this ADR.

## Alternatives considered

- **Keep the lane serialisation (status quo).** Rejected by the user's ruling: a stop waits up to 90 s, the stop latency of a booting VM is
  its 30 s boot wait, and the slot concern grows with the volume waits.
- **Raise the signal from the broker whenever any evaluation is deferred behind an active target.** Rejected: a predecessor's late exit row,
  a probe result, a resync or an `EnqueueEvaluation` handoff for the workload would abandon a healthy launch and every re-launch the same
  way; the cause is an operator commit, which only the two handlers know.
- **Drop the start future (`select!` over the whole `driver.start`).** Rejected: it abandons effects mid-way (the rootfs clone, the VMM
  spawn, `lvm`), leaves the supervision claim `Starting` forever and the VMM alive outside the reclamation surface, and is the
  cleanup-hostile cancellation `.claude/rules/reconcilers.md` forbids.
- **Poll the intent store for the stop sentinel and generation inside the launch.** Rejected: a poll where an in-process event exists,
  with a latency floor of the interval, and a second reader of the intent keys.
- **A VM-only path: a default-provided second `Driver` method, or a task-local signal.** Rejected: a default that silently ignores the
  signal lets a future driver with waits never supersede, and a task-local is a hidden input the DST harness cannot see. A required
  parameter makes every driver take a position at compile time (cost: mechanical fallout across the `Driver` implementors and callers).
- **A new `DriverStartFailure` class `Superseded` (through `StartRejected`).** Rejected: it would be classified like a failure and need a
  `TransitionReason`, which is the representation that lets a superseded launch be written as a failed, charged allocation.
- **Shutdown: leave it draining launches to their own bound.** Rejected by the user: a node shutdown or a
  restart of the control plane waits up to about 90 s behind a launch that no operator can stop, and the launch it waits for is one the
  next boot would place again anyway.
- **Shutdown: list the open lanes and cancel each.** Rejected: a lane opened between the list and the cancel is missed, so its launch
  drains to its bound; the loop admits evaluations concurrently, so the window is reachable.
- **Shutdown: `ServerHandle::shutdown` signals the runtime directly.** Rejected: two other graceful paths (`FailedBootTasks::shutdown` and the
  `run_server` early return) cancel the same convergence token and would miss it; the loop's own shutdown arm is the one place all three reach.
- **Shutdown: abort the convergence task or its tick futures.** Rejected: `abort()` drops a tick mid-effect and skips the teardown the shim
  and the driver run, and `.claude/rules/rust.md` makes a cooperative token, never `abort()`, the shutdown mechanism.

## Consequences

- Positive: `workload stop` and `restart` return at the teardown time during a claim wait or a boot wait, for every VM launch; the
  worst case is the 30 s `ensure` deadline; eight stalled launches no longer hold a stopped workload's own slot.
- Positive: node shutdown no longer waits for a launch's claim wait or boot wait (about 90 s at worst); it waits at most for an `ensure` in
  flight, `Vmm::create` and the teardown.
- Positive: a superseded launch leaves nothing behind it. It reuses the start-failure teardown, releases the supervision claim, network
  lease and SVID, and writes no row (traced against the shim and the reclamation sweep in the feature delta).
- Negative: `Driver::start` changes signature for all drivers; one new `DriverError` variant changes exhaustive matches; one conservative
  window costs one extra evaluation per operator commit; a teardown failure while abandoning is reported as an ordinary start failure.
- Negative: a launch abandoned by shutdown is lost work (the clone and the boot are redone by the next boot); it never wrote a row, so nothing
  is repaired: the next evaluation places one allocation again from the observed rows (a pending generation, or no running allocation;
  ADR-0335).
- Accepted cost: an evaluation signalled before its launch starts still provisions the guest network and mints the SVID, then tears both
  down; no shim pre-check is added.
- Bound: shutdown raises the signal when the convergence stage begins, after the gateway stage of `ServerHandle::shutdown` (bounded by its
  drain deadline); launches run on during the gateway stage as today.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Superseding a launch, § Node shutdown raises the signal, § Gate G-3, § Formal protocol model
- Formal model: `specs/quint/node-local-volume/` (one hazard, `noInterrupt`); the restart-generation model (ADR-0335) is the separate `specs/quint/restart-generation/`
- [ADR-0102](adr-0102-bounded-convergence-evaluation-ownership.md), [ADR-0317](adr-0317-a-replacement-waits-for-the-volume-claim-not-for-the-vmm-process.md),
  [ADR-0104](adr-0104-vm-recreation-fresh-allocation-identity.md),
  [ADR-0335](adr-0335-a-restart-generation-is-handled-when-its-successor-is-observed.md), `.claude/rules/reconcilers.md`,
  `docs/research/orchestration/stop-during-start-comprehensive-research.md`
