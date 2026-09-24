# ADR-0124 — Reconcile failed shared-network owners for five seconds, then fail-stop

## Status

**Accepted — the current #295 contract is user-approved and independently
approved through D-295-DISTILL-8 at review iteration 9 on 2026-09-17;
D-295-DISTILL-11 component/task/S37 evidence is autonomously authorized and
pending the trusted-checkpoint review.** This
records RUN-295-B.

**Amended 2026-09-24** by the accepted #295 correctness-recovery replacement
DESIGN. This decision is operative in code committed at HEAD `db3af700` on the
#295 feature branch (`lib.rs:1406`, `:1412`, `:1449`, `:1458-1459`: quiescence,
the 250 ms retry, and the 20-attempt / 5 s fail-stop; not merged to `main`), and
that implementation does not realize it (#295 `recovery/proof-findings.md`
§3.3). The amendments below keep this decision's intent; their exact contracts
live in the #295 feature delta:

- D-295-R13: a recovery attempt is the failing owners' converge, one full
  audit, and then the TAP restore only if that audit is clean;
- D-295-R14: a complete component matrix with per-TAP quiescence outcomes,
  per-allocation damage attributed separately, and bounded owner calls; the
  per-allocation damage set includes a changed TAP debug message mask
  (D-295-R22, ADR-0130's read-back set);
- D-295-R15: audit and repair of the program, the policy route, the guard
  table, and the members;
- ADR-0138: required composition ports;
- ADR-0140 (accepted conditional on its native RED): TPROXY before the
  policy-route mark. The listener-loss sentence of the Decision depends on it
  for outbound guest TCP, subject to the `TIME_WAIT` side door E14 (e): if that
  native RED reproduces, leg-F/leg-C loss becomes a TAP-quiescing kernel-path
  component and the killed-`serve` residue exposure goes to the user;
- ADR-0131: the action shim waits on the release claim before raising a TAP;
  it gains no recovery, reopen, or fail-stop authority.

Two parts of the Decision below are rewritten to user rulings:

- **Kill scope (D-295-R14, user rulings 2 and 8 of 2026-09-24).** As accepted
  on 2026-09-17 the Decision read: *"Kernel-path mismatch also quiesces managed
  TAPs; failure to confirm quiescence kills the affected VMM cgroups and takes
  the fail-stop path."* It now kills only the VMs whose TAPs could not be
  confirmed down or whose own network parts are damaged, and continues repair
  for the rest. The whole workloads slice is killed, and the process
  fail-stops, only when the failing set cannot be determined or a per-VM kill
  cannot be written.
- **SIGTERM (D-295-R17, user ruling of 2026-09-23).** The CLI selects the
  fail-stop request ahead of SIGTERM as well as SIGINT, and a normal SIGTERM
  exits status 0.

## Context

The shared bridge, leg-F/leg-C listeners, DNS loop, TCX links/maps/pins, and nft
guard/routing state affect every local microVM. Startup probes do not detect a
later task exit or kernel-state loss. Keeping new guest command release open
after such a failure would turn a node-global dependency into an undetected
partial service.

The repository assumes an appliance process supervisor, but contains no shipped
service-unit restart policy or restart proof. #295 can fail-stop `overdrive
serve`; it cannot claim that an external supervisor restarted it.

Bounded recovery followed by escalation is the Erlang/OTP supervisor and systemd
`StartLimitBurst` shape; Cilium's controllers instead retry without bound. Both
families exist in mature systems. Periodic audit-and-repair of owned rules and
set members is what Calico Felix does, but Felix's refresh intervals (90 to 180
seconds) exist for drift correction. The one-second audit here is a security
detection bound on how long a lost classifier or guard can go unnoticed, which
is why it is one to two orders of magnitude tighter. When no system confirms
enforcement, mature dataplanes keep enforcing the last state (Cilium) or scope
remediation to the affected workloads (Istio's repair controller); none kills
every workload on a node locally. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 8.1, 8.2 and 8.3.)

## Decision

Listener/DNS task completion is observed immediately. A one-second audit checks
bridge/TAP membership, TCX program/link/ifindex, endpoint maps, bpffs pins, and
normalized nft rules/sets. Detection atomically closes new EXEC release and
emits component/cause degraded health. Kernel-path mismatch also quiesces
managed TAPs. Quiescence reports, for each managed TAP, whether it was read back
administratively down. The VMM cgroup of each allocation whose TAP could not be
confirmed down is killed, and bounded repair continues for the rest. An
allocation whose own network parts are damaged (its TAP deleted or its owner,
persistence, host-side MAC, or debug message mask changed; its TCX ingress or egress link or
classifier detached; or its link pin, endpoint entry, or bridge-guard member
gone) is handled the same way: only its VM is killed, and the node is not
fail-stopped for it. A killed VM's parts are no longer audited or restored, so
recovery can reopen for the rest. When the platform cannot determine which TAPs
are down (the quiescence call fails as a whole or misses its bound), or a per-VM
kill cannot be written, the whole workloads slice is killed and the fail-stop
path is taken. Whole-node fail-stop otherwise remains only for node-level
components that fail bounded repair.

A consequence of the per-allocation classification, stated so it is not
mistaken for a repair path: a single common-cause loss that manifests as
per-allocation damage across every allocation — a flushed managed-TAP nft set
or a flushed endpoint map that removes every entry while the set or map identity
itself survives the node-level check — is classified per allocation. Every
affected VM is then killed, EXEC is left Open, and no node-level repair is
attempted, because the node-level identity check passed. This is the accepted
behaviour of the user-approved per-allocation kill scope, not a defect in it;
the ruling is not reopened.

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
owner algorithm.

The owner retries the exact failed component every 250 milliseconds through the
same production convergence and read-back path for at most five seconds.
Admission reopens only after all invariants pass and quiesced TAPs are restored.
Existing enforced handles remain owned during this bounded interval; new
connects or DNS queries fail while their socket owner is absent.

For bridge/TAP/TCX/map/pin/bridge-guard ownership, “the same production path”
means the same `SharedGuestNetworkOwner` object used at boot and inherited by
allocation provision/teardown. Its audit is non-repairing; convergence and TAP
quiescence are separate awaited effects.

The shared-owner plan, ports, facts, scratch observations, and orchestration
error live in `overdrive-control-plane::guest_network`; the private host owner
composes `overdrive-netlink` and `overdrive-dataplane::guest_tcx`. Dataplane
owns aya attach projection and raw aya source errors, netlink owns its existing
source error, and core contains neither. The control-plane supervisor and sim
therefore consume one source-bearing result without a duplicate error family.

The shared-network audit result carries both the first failed closed component
and its exact existing guest-network source. The host owner derives that pair
from real read-back. Listener return/error/panic/cancel/channel-close outcomes
are classified by the worker's private real Tokio task owner.

Leg-F and leg-C recovery may bind only the exact previously recorded address and
port. It never selects a new ephemeral port or rewrites nft targets. Exact-port
bind/read-back failure retries within the same five-second window and then
fail-stops. A pure listener failure does not quiesce TAPs or existing commands.

At five seconds the internal supervisor sends one typed fail-stop request to the
CLI-owned serve handle. The CLI selects that request ahead of SIGINT and
SIGTERM, bounds the entire graceful shutdown attempt to ten more seconds, and
exits status 1 on completion, shutdown error, or hard timeout. Timeout uses
immediate process exit so an unbounded owner teardown cannot extend the bound.
Normal SIGINT and SIGTERM keep status 0. Exact public request/method/error
shapes live only in the feature delta.

The `ServerHandle` itself retains the sole supervisor join handle, request
receiver, and supervisor half of the paired EXEC wiring. Its wait observes
supervisor normal return, typed task
error, panic, cancellation, and request-channel closure as distinct typed
fail-stop causes and closes EXEC before returning any of them. An explicit
fail-stop sender parks until normal owner shutdown, so join observation cannot
mask its request. Intentional SIGINT or SIGTERM shutdown cancels the supervisor
only after the CLI has left this wait. No second supervisor task or hierarchy is added.
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

The runtime never constructs its successor process or handler. No public gate
accessor exists; EXEC closure is observable only through the gate's behaviour
and the structured unhealthy event. The evidence obligations for this decision
(seeded, in-process, conformance, and native-metal lanes, including S-ND295-33
and S-ND295-37) live in the #295 feature delta, not in this record.

## Alternatives considered

### Immediate fail-stop

Rejected. It is simpler but discards a bounded opportunity to repair a transient
shared task or kernel-object loss through already-required idempotent
convergence.

### Remain degraded until operator restart

Rejected. It has no automatic recovery bound and leaves an appliance node
indefinitely unable to release new workloads.

### Kill every workload VMM whenever any TAP cannot be confirmed down

Rejected by the user ruling of 2026-09-24 (D-295-R14). When the platform knows
which TAPs failed to go down, only those VMs can still emit frames, so killing
the rest destroys healthy workloads for no isolation gain. The whole-slice kill
remains the fallback when the failing set cannot be determined.

### Treat one VM's damaged network parts as a node-level failure

Rejected by the user ruling of 2026-09-24 (D-295-R14). A deleted TAP, a detached
TCX link, or a missing link pin, endpoint entry, or guard member belongs to one
VM. Node-level convergence does not rebuild per-VM parts, so the failure would
persist to the deadline and fail-stop every workload on the node for one VM's
loss. Killing that VM removes its only frame source, and its lifecycle replaces
it.

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
