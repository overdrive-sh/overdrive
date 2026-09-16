# ADR-0124 — Reconcile failed shared-network owners for five seconds, then fail-stop

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records RUN-295-B.

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

**Application-boundary amendment, 2026-09-16 (user-authorized solution-review
F-01 remediation).** The dependency graph is control-plane → worker → core, so
the paired gate/supervisor capabilities and request/recovery vocabulary live in
dependency-neutral `overdrive-core`. One wiring constructor over the injected
clock creates both opaque capabilities: `VmDriver` receives only release-claim
authority, while the control-plane supervisor and `ServerHandle` retain only
recovery/reopen/fail-stop authority over the same private state. This closes the
cross-crate contract without changing the accepted runtime gate behavior or
`Driver` trait.
Exact types and signatures remain single-sourced in the #295 feature delta.

**Cold-boot amendment, 2026-09-16 (user-authorized S2-F01).** New wiring starts
BootClosed, not Open. Only the supervisor capability may open it, after VMM and
stale-attachment reclamation prove zero managed TAPs and fresh listener/owned-
rule recovery completes full read-back. A failed precondition, replacement,
rollback, or audit leaves it closed and refuses startup. Runtime still uses
Open/Recovering/FailStop with exact prior-port rebind and no target rewrite.

The owner retries the exact failed component every 250 milliseconds through the
same production convergence and read-back path for at most five seconds.
Admission reopens only after all invariants pass and quiesced TAPs are restored.
Existing enforced handles remain owned during this bounded interval; new
connects or DNS queries fail while their socket owner is absent.

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
