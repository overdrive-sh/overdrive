# ADR-0124 — Reconcile failed shared-network owners for five seconds, then fail-stop

## Status

**Accepted — the current #295 contract is user-approved and independently
approved through D-295-DISTILL-8 at review iteration 9 on 2026-09-17;
D-295-DISTILL-11 component/task/S37 evidence is autonomously authorized and
pending the trusted-checkpoint review; amended 2026-09-23 by explicit user
direction with no review cycle for deferred TAP activation/quiescence
serialization.** This
records RUN-295-B.

## Context

The shared bridge, leg-F/leg-C listeners, DNS loop, TCX links/maps/pins, and nft
guard/routing state affect every local microVM. Startup probes do not detect a
later task exit or kernel-state loss. Keeping new guest command release open
after such a failure would turn a node-global dependency into an undetected
partial service.

The repository assumes an appliance process supervisor, but contains no shipped
service-unit restart policy or restart proof. #295 can fail-stop `overdrive
serve`; it cannot claim that an external supervisor restarted it.

## Decision

Listener/DNS task completion is observed immediately. A one-second audit checks
bridge/TAP membership, TCX program/link/ifindex, endpoint maps, bpffs pins, and
normalized nft rules/sets. Detection atomically closes new EXEC release and
emits component/cause degraded health. Kernel-path mismatch also quiesces
managed TAPs; failure to confirm quiescence kills the affected VMM cgroups and
takes the fail-stop path.

EXEC closure and VM release share one lock-based gate. `VmDriver` obtains a
release claim before taking the deferred EXEC values and holds it through writer
acknowledgement. Detection changes Open to Recovering under that lock; later
release attempts wait, then proceed only after recovery or refuse on FailStop.
Already-written commands are never paused or frozen, and the public `Driver`
trait is unchanged.

The dependency graph is control-plane → worker → core, so
the paired gate/supervisor capabilities and request/recovery vocabulary live in
dependency-neutral `overdrive-core`. One wiring constructor over the injected
clock creates both opaque capabilities: `VmDriver` receives only release-claim
authority, while the control-plane supervisor and `ServerHandle` retain only
recovery/reopen/fail-stop authority over the same private state. This closes the
cross-crate contract without changing the accepted runtime gate behavior or
`Driver` trait.
Exact types and signatures remain single-sourced in the #295 feature delta.

`ServerHandle` retains exactly one private supervisor owner containing the
capacity-one receiver, sole supervisor join, paired EXEC supervisor capability,
and intentional-shutdown token. Its public wait delegates. Every abnormal
return/error/panic/cancellation/channel-close class writes FailStop before the
typed request returns; intentional shutdown is signalled and joined separately.
The same supervisor owns the private DNS task owner, so DNS task loss and
recovered replacement cannot create a second observer or recovery authority.

The DNS task owner has explicit Running, Exited, Replacing, ShuttingDown, and
Stopped states. Recovery publishes a replacement only after the prior task is
terminal and the replacement responder has passed exact bind/probe/read-back.
Cooperative stop/join is primary; bounded abort-and-await is only the recorded
backstop. A live join handle is never overwritten, dropped, or detached, and
intentional shutdown prevents subsequent replacement publication.

New wiring starts BootClosed, not Open. Only the supervisor capability may
open it after VMM and
stale-attachment reclamation prove zero managed TAPs and fresh listener/owned-
rule recovery completes full read-back. A failed precondition, replacement,
rollback, or audit leaves it closed and refuses startup. Runtime still uses
Open/Recovering/FailStop with exact prior-port rebind and no target rewrite.

The preceding startup scratch proof is produced by the actual private host-
owner algorithm. Source-local tests drive its module-private raw-effect seam;
composed tests may script the public owner result only to prove that BootClosed,
startup refusal, and non-publication follow. Native-metal tests retain actual
kernel-effect and inventory authority.

The owner retries the exact failed component every 250 milliseconds through the
same production convergence and read-back path for at most five seconds.
Admission reopens only after all invariants pass and quiesced TAPs are restored.
Existing enforced handles remain owned during this bounded interval; new
connects or DNS queries fail while their socket owner is absent.

Allocation activation and runtime quiescence are serialized inside this same
owner. Quiescence latches before its first down mutation. An activation that
linearizes first may complete but is then included in the quiesce/read-back;
an activation that observes the latch returns without raising the TAP. Recovery
restores only attachments whose activation completed before quiescence;
provisioned-down attachments awaiting post-Running intercept installation stay
down. `converge_shared` performs those restores and full read-back before the
latch clears and before the EXEC supervisor reopens. This is private
allocation/owner state, not a new gate method, persisted phase, or recovery
owner. The same private awaited sequencer makes runtime audit phase-aware:
ProvisionedDown and QuiescedActive expect down; Active expects up. The
intentional pre-activation interval is therefore not reported as drift.

For bridge/TAP/TCX/map/pin/bridge-guard ownership, “the same production path”
means the same `SharedGuestNetworkOwner` object used at boot and inherited by
allocation provision/activation/teardown. Its audit is non-repairing; convergence and TAP
quiescence are separate awaited effects. The sim adapter may return the same
typed outcomes for ordering/convergence evidence but cannot pretend to create
kernel state, which remains Tier-3 evidence.

The shared-owner plan, ports, facts, scratch observations, and orchestration
error live in `overdrive-control-plane::guest_network`; the private host owner
composes `overdrive-netlink` and `overdrive-dataplane::guest_tcx`. Dataplane
owns aya attach projection and raw aya source errors, netlink owns its existing
source error, and core contains neither. The control-plane supervisor and sim
therefore consume one source-bearing result without a duplicate error family.

The shared-network audit result carries both the first failed closed component
and its exact existing guest-network source. The host owner derives that pair
from real read-back; the public sim owner provides independent standing slots
for all twelve components plus one exact next component/source pair for seeded
owner-port schedules. This is deterministic port-output scripting, not a task
kill or kernel-state claim. Listener return/error/panic/cancel/channel-close
coverage instead runs through the worker's private real Tokio task owner; no
test constructs those consequences.

Leg-F and leg-C recovery may bind only the exact previously recorded address and
port. It never selects a new ephemeral port or rewrites nft targets. Exact-port
bind/read-back failure retries within the same five-second window and then
fail-stops. A pure listener failure does not quiesce TAPs or existing commands.

At five seconds the internal supervisor sends one typed fail-stop request to the
CLI-owned serve handle. The CLI selects that request ahead of SIGINT, bounds the
entire graceful shutdown attempt to ten more seconds, and exits status 1 on
completion, shutdown error, or hard timeout. Timeout uses immediate process
exit so an unbounded owner teardown cannot extend the bound. Normal SIGINT keeps
status 0. Exact public request/method/error shapes live only in the feature
delta.

The `ServerHandle` itself retains the sole supervisor join handle, request
receiver, and supervisor half of the paired EXEC wiring. Its wait observes
supervisor normal return, typed task
error, panic, cancellation, and request-channel closure as distinct typed
fail-stop causes and closes EXEC before returning any of them. An explicit
fail-stop sender parks until normal owner shutdown, so join observation cannot
mask its request. Intentional SIGINT shutdown cancels the supervisor only after
the CLI has left this wait. No second supervisor task or hierarchy is added.
The public request carries recovery elapsed time as `std::time::Duration`
measured monotonically from detection, never as a raw millisecond integer.
The existing EXEC-gate state also owns the latest recovery snapshot under the
same lock: first detection records component/start time/zero completed
attempts; each complete converge-plus-full-read-back attempt advances the count
and records the first invariant still failing in a fixed audit order. Partial
repair never reopens admission. An abnormal supervisor exit copies that
snapshot into the request before changing the gate to FailStop. If recovery
never began, it deterministically reports component `Supervisor`, zero
attempts, and `std::time::Duration::ZERO`. No second synchronization or
recovery-state owner is added; the dependency-neutral capability is the single
cross-crate projection of this state.

Fail-stop records an abandoned ownership inventory; graceful drain records
completion or hard-timeout abandonment. The next boot records recovered residue
counts and empty complements after reclamation/sweep. External restart is an
explicit deployment precondition, not a #295 mechanism. Production readiness
must prove a real supervisor starts a new process and that boot probes and
reclamation complete before admission reopens.

S-ND295-33 is a narrower recurring conformance lane: it drives the exported
server handler through the public HTTPS API, drains that handler after typed
fail-stop, and has its harness construct a fresh handler over retained roots.
The runtime never constructs its successor. That scenario makes no subprocess,
CLI, PID, or new-process claim; the external deployment-supervisor readiness
obligation above remains separate and unchanged.

S-ND295-37 proves EXEC closure by evidence composition rather than a public gate
accessor. Native metal observes the existing structured `TcxLink` unhealthy
event after typed link+guard deletion and before TAP-down, while retaining the
one-second frame/counter/quiescence oracle. A source-local supervisor test proves
the production branch wins `begin_recovery(TcxLink)` before emitting that event;
the core gate test proves Open→Recovering blocks every new claim. Exact event
name/component/order is the join key. A TAP-down observation alone is not
misrepresented as direct gate-state evidence.

## Alternatives considered

### Immediate fail-stop

Rejected. It is simpler but discards a bounded opportunity to repair a transient
shared task or kernel-object loss through already-required idempotent
convergence.

### Remain degraded until operator restart

Rejected. It has no automatic recovery bound and leaves an appliance node
indefinitely unable to release new workloads.

## Consequences

Positive: every shared owner has a detection signal, fail-closed admission
response, bounded repair window, and terminal outcome. Negative: the control
plane gains a one-second audit and shared-owner supervisor, and recovery still
depends on an external process supervisor after fail-stop. No HA, new daemon, or
durable recovery state is introduced. The control-plane/CLI boundary gains one
typed shutdown request and wait method so the current CLI handle owner can apply
the hard outer bound and non-zero exit. The confidentiality guarantee covers
healthy state and any single owned-component loss. Arbitrary near-simultaneous
external deletion of both a TAP's TCX entrypoint and the independent bridge
guard can restore ordinary forwarding for at most the one-second audit interval
before managed TAP quiescence; this bounded double-loss exposure is an accepted
RUN-295-B downside, not described as fail-closed.
