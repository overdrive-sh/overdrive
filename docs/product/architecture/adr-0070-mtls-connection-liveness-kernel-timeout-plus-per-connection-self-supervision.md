# ADR-0070: Transparent-mTLS connection liveness — kernel TCP timeouts + per-connection self-supervision (retire the central tick enumerator)

## Status

**Accepted** (2026-06-14). **Refines ADR-0069 § "Sensitivity / trade-off
points (ATAM)" → "Pump supervision policy (F6)"** and the SD-4 *supervision
shape* it instantiated. This ADR settles **how** a transparent-mTLS proxy
connection's liveness is supervised in v1: it adopts **(C) kernel TCP
timeouts on the spliced legs + (B) per-connection self-supervision** and
**rejects (A) a central tick-driven enumerator over the live-connection
set**. The central `MtlsSupervisor` (`overdrive-worker`, step 04-01) — the
sole concrete instance of (A) — is **retired**.

**This ADR does NOT touch the locked core of ADR-0069.** The universal,
undisableable agent-light L4 proxy model (D-MTLS-1), the fold of #222 into
#26, OQ-2 (the `overdrive-dataplane`/`overdrive-bpf` adapter home), SD-1(a)
(the owned-`OwnedFd` `InterceptedConnection` payload + the
worker-does-the-`accept()` composition-root coupling), SD-2 (the **port owns
the pump**; the worker observes liveness and tears down — UNCHANGED), SD-3
(`async fn probe`), the 4-method `MtlsEnforcement` contract
(`probe`/`enforce`/`liveness`/`teardown`), the bidirectional model (F3), the
agent-light asymmetry (D-MTLS-4/13), the no-psock invariant (D-MTLS-5), the
F4/F7 resource limits, the F5 intercept-exemption, and the authn-only v1
boundary (F1/F5/#178) are ALL unchanged. **What changes is one thing: the
F6 *supervision shape*.** ADR-0069's F6 text pinned "the worker point-queries
`liveness(&handle)` on its existing reconciler-tick cadence (SD-4 point-query,
not a push stream in v1)" — a central enumeration over the live-connection
set. Connection-supervision research (`docs/research/dataplane/
transparent-mtls-connection-supervision-research.md`, 2026-06-14, 22 sources)
found that shape is the *odd one out*: **no surveyed production dataplane
supervises per-connection liveness with a central loop**, and Overdrive's own
reconciler doctrine independently disqualifies it. This ADR replaces that one
clause; the rest of ADR-0069 F6 (the `Stalled` predicate, the fail-closed
teardown reaction, the telemetry) survives, re-homed to where the production
precedent puts it — the kernel and the connection's own task.

## Context

### The F6 concern (what ADR-0069 left as the supervision shape)

ADR-0069 ships an agent-light L4 proxy: per connection, the agent runs a
rustls handshake, arms kTLS, and then drives bounded byte-movement pumps for
the connection's life (a zero-copy `splice` out of a kTLS-RX leg for the
DECRYPT directions; a `read → write_all` copy into a kTLS-TX leg for the
ENCRYPT directions). The reliability sensitivity point ADR-0069 § ATAM names
is a **stranded/crashed pump**: the agent must keep the pump live for the
connection's life, and a stalled pump strands the affected direction (legs
open, fds pinned, no bytes moving).

ADR-0069's F6 amendment pinned a *supervision policy* for this: the dataplane
adapter DERIVES `PumpLiveness::Stalled` (SD-2) from the pump's bytes-moved
progress metric; the worker REACTS by point-querying `liveness(&handle)` for
every established connection on its reconciler-tick cadence and tearing down
the `Stalled` ones (SD-4). That worker-side reactor is the central
`MtlsSupervisor` (`crates/overdrive-worker/src/mtls_supervisor.rs`,
`supervise_tick(&[EnforcedConnection])`) shipped at step 04-01. **It is a
central loop that enumerates the live-connection set each tick and
point-queries each connection's liveness** — shape (A) in the research's
taxonomy.

### Three candidate supervision shapes

The forcing question the research answers:

- **(A) central reconciler / tick enumerator** — a worker loop that walks the
  live-connection set each tick and point-queries `liveness` per connection
  to reap stalled ones. *This is what ADR-0069 F6 / SD-4 pinned and 04-01
  shipped.*
- **(B) per-connection self-supervision** — each connection's own task owns
  its lifecycle and self-tears-down fail-closed on EOF / error / a no-progress
  deadline. No central registry, no tick loop, no enumeration.
- **(C) kernel TCP timeouts** — set `TCP_USER_TIMEOUT` + TCP keepalive on each
  enforced connection's legs so the kernel reaps transport-dead connections
  (peer gone, half-open, unacked-past-deadline) with no userspace loop.

### The evidence base

`docs/research/dataplane/transparent-mtls-connection-supervision-research.md`
(Nova, 2026-06-14; 22 sources, avg reputation ≈ 0.86; Q1–Q5). Surveyed
Cilium, Istio ambient/ztunnel, Linkerd/linkerd2-proxy, Envoy, and the Linux
kernel. The decisive findings:

1. **Per-connection self-supervision (B) is universal; no surveyed system
   uses a central liveness enumerator (A).** Envoy arms per-connection /
   per-stream `libevent` timers (`idle_timeout`, `stream_idle_timeout`,
   `max_connection_duration`) on the owning worker's event loop; ztunnel
   drives each connection as its own tokio future to EOF/error; linkerd2-proxy
   stacks per-connection tower `Idle`/`FailFast` middlewares. The timer *is*
   the liveness check, co-located with the connection. (Research Q1, Findings
   2.1, 3.1, 4.1–4.3.)

2. **Where a central loop DOES exist, it reconciles CONFIG, not liveness.**
   The one central live-connection loop in the survey — ztunnel's
   `ConnectionManager` + `PolicyWatcher` — exists to re-evaluate **RBAC on
   authorization-policy change and drain unauthorized connections**, and its
   own source documents it as "policy enforcement and graceful connection
   draining… **not** connection reaping." This is config reconciliation
   projected onto connections, a different concern from liveness. (Research
   Q2, Finding 2.2 — load-bearing.)

3. **Transport-death is the kernel's job (C), evidenced directly.**
   Linkerd's production fix for stranded half-open connections was
   `TCP_USER_TIMEOUT` — *delegating to the kernel*, not adding a userspace
   watchdog ("kernel-level solutions are necessary for this class of
   problem"). ztunnel enables keepalive by default (1.24+). The kernel reaps
   the entire transport-dead class with zero userspace machinery; the pump
   task simply observes the resulting `ETIMEDOUT`/EOF/RST and resolves.
   (Research Q3, Findings 3.3, 2.3, 5.1–5.2.)

4. **The reconciler doctrine independently disqualifies (A).** Per
   `.claude/rules/reconcilers.md`, a reconciler candidate manages
   desired-vs-actual over a resource that can DRIFT, where the *desired state
   is independent of the actual*. A stalled connection is **not** config
   drift — there is no declared "desired connection set" the platform
   converges to; the connection's existence is driven by the application. The
   natural owner of a connection's death is the connection's own task, and a
   per-tick enumeration is the wrong granularity for a per-connection stall
   deadline (it reaps up to one tick late and walks every live connection each
   tick though almost none are stalled). (Research Q5; `reconcilers.md`
   § "Not a candidate" → "Stateless request handling"/"genuinely-terminal".)

### Two stall classes — and which mechanism owns each

The research splits the F6 concern into two classes the chosen design maps
cleanly onto:

| Stall class | Cause | Owner in v1 | Mechanism |
|---|---|---|---|
| **Transport-dead** | peer gone, unacked data past deadline, half-open | **kernel (C)** | `TCP_USER_TIMEOUT` + keepalive on the legs → `ETIMEDOUT`/RST; the pump task observes EOF/error and self-resolves (B) |
| **Progress-stuck (kernel-invisible)** | sockets healthy at the TCP layer but the `splice`/kTLS pump not advancing with a record pending | **deferred** (Tier-3 spike) | a per-connection progress watchdog evaluating the existing `Stalled` predicate — but the precise *progress signal* for a **kTLS-spliced** pump is undocumented upstream (research Gap 2); v1 ships (C)+(B), which covers transport-death + crashed-pump |

### Quality attributes driving the decision (ISO 25010)

| Attribute | Why it dominates here |
|---|---|
| **Reliability — fault tolerance** | The whole concern: a stranded pump must be reclaimed without leaking fds/kTLS state or leaking cleartext. (C) reaps transport-death promptly and tunably; (B) self-tears-down on EOF/error fail-closed. |
| **Performance efficiency** | (A) walks every live connection each tick regardless of stall; (C)+(B) cost is per-connection and event-driven — the kernel timer fires at the connection's own deadline, the pump task wakes on its own EOF/error. No central scan. |
| **Maintainability** | Removing the central `MtlsSupervisor` removes a bespoke per-tick enumerator and a worker→port `&[EnforcedConnection]` registry the worker would otherwise have to assemble. The connection's own enforce task is the single owner of its lifecycle. |
| **Security — confidentiality** | Both stall classes resolve **fail-closed**: the kernel RSTs / the pump task tears down and closes legs; no cleartext leaks, blast radius is the one stranded connection. (UNCHANGED from ADR-0069 F6.) |

## Decision

**Supervise transparent-mTLS connection liveness with (C) kernel TCP timeouts
on the spliced legs + (B) per-connection self-supervision in each connection's
own enforce task. Reject (A) the central tick-driven enumerator and retire the
`MtlsSupervisor`.**

### (C) Kernel TCP timeouts on the spliced legs — the primary v1 mechanism

On every enforced connection's legs (outbound: leg F + leg B; inbound: leg C +
leg S), the host adapter sets `TCP_USER_TIMEOUT` and enables TCP keepalive
during `enforce`, before handing the legs to the steady-state pumps. The
kernel then reaps the entire **transport-dead** class (peer gone, unacked
data past the deadline, half-open) with **no userspace loop**. This is the
direct, evidenced production answer (Linkerd's `TCP_USER_TIMEOUT`,
ztunnel's default-on keepalive). The exact socket-option *values* are an
adapter implementation concern (mechanism, hidden behind the port per SD-2),
bounded by the same fail-closed discipline as the F4/F7 limits; the
acceptance test asserts the observable — a connection whose peer dies is
reaped and reaches `Gone`, no fd/kTLS leak — not the literal `setsockopt`
arguments.

### (B) Per-connection self-supervision — owned by the enforce task

Each connection's own enforce task (the SD-2 port-owned pump task) owns its
full lifecycle and **self-tears-down fail-closed** on EOF / error / (when the
kernel timeout fires) `ETIMEDOUT`. There is **no central registry, no
`supervise_tick`, no tick cadence, no enumeration** of the live-connection
set in v1. When the kernel reaps a transport-dead leg (C), the pump's
`read`/`splice`/`write` observes the resulting EOF/error and the task tears
down its own legs (close both legs, stop both pumps, reclaim kTLS state) —
the same fail-closed teardown ADR-0069 F6 specified, now triggered by the
connection's own task rather than a central worker query. The per-connection
task is the natural owner of its own death (Envoy/ztunnel/linkerd2-proxy
precedent).

### (A) Central tick enumerator — REJECTED and retired

A central loop that enumerates the live-connection set each tick to
point-query liveness is **not adopted** for v1 liveness. It is the shape no
surveyed production dataplane uses for liveness, and `reconcilers.md`
independently disqualifies it (a stalled connection is not desired-vs-actual
config drift; the connection's own task is the natural owner; per-tick
enumeration is the wrong granularity). The central `MtlsSupervisor`
(`crates/overdrive-worker/src/mtls_supervisor.rs`) and its acceptance tests
(`crates/overdrive-worker/tests/acceptance/
mtls_supervisor_teardown_on_stall.rs`) are **deleted** — see § Consequences →
"Retiring the central MtlsSupervisor".

### The genuinely-hard residual — DEFERRED (Tier-3 spike)

The one stall class neither (C) nor a transport-level signal covers is the
**kernel-invisible progress-stall**: a `splice`/kTLS pump stuck while the
sockets look healthy at the TCP layer (a record pending but the pump not
advancing). The kernel cannot detect this (research Finding 5.3,
Cloudflare-confirmed: the kernel cannot see "slow drains" / "stuck buffers").
The app-level *progress* predicate that would catch it
(`tcpi_notsent_bytes` deltas vs the kTLS record sequence vs `splice` return
semantics) **for a kTLS-spliced pump is undocumented upstream** (research
Gap 2). Per the standing project rule that kernel-mediated mechanisms with no
test backstop are Tier-3-spiked before locking, **the kernel-invisible
progress-stall watchdog is DEFERRED to #232 (Tier-3 spike)** and is NOT built
in v1. v1 ships (C)+(B), which covers the transport-death and crashed-pump
cases for real (a crashed pump task closes its legs → EOF → the peer/kernel
sees the close). When the spike ([#232](https://github.com/overdrive-sh/overdrive/issues/232))
lands, the progress watchdog is a **per-connection** addition inside the
enforce task (NOT a central loop) consuming the existing
`PumpLiveness::Stalled` predicate, which this ADR deliberately **retains** on
the contract for exactly that hook (see § Consequences → "Contract
reconciliation").

### The policy-plane future home — a central registry IS correct there, NOT for v1 liveness (forward design rationale)

This subsection is **forward design rationale, not a tracked unit of v1
deferred work** — it records *why* a central-registry shape is right for a
future policy-plane concern, so that shape is not mistakenly resurrected for
v1 liveness. A central connection registry + control loop IS the right shape
for the FUTURE **revocation / policy-driven force-close** concern (Phase 5;
whitepaper §8): when a cert is revoked or an exemption/authorization changes,
the platform must walk existing connections and force-close the ones that no
longer pass — the ztunnel `ConnectionManager` + `PolicyWatcher` precedent
(graceful drain on authz/identity change). **That is config reconciliation
projected onto connections, not liveness reaping.** This ADR explicitly names
that registry/reconciler as the correct home for the *policy* plane and
explicitly **does NOT build it now**. v1 has no revocation and no
policy-driven force-close in #26's scope (authorization is #27's BPF-LSM hook;
expected-peer pinning is #178). The future home for that mechanism is the
existing [#37](https://github.com/overdrive-sh/overdrive/issues/37) (the
central per-allocation live-connection registry + drain detector) and
[#82](https://github.com/overdrive-sh/overdrive/issues/82) (gossip-propagated
certificate revocation) — cross-referenced here as the related future
mechanisms, **not** as work either issue plans for "revocation-driven mTLS
force-close" today. Do not resurrect the central loop for liveness on the
strength of "we'll need a registry for revocation later" — the two concerns
are separate, and the registry, when it lands, is named for policy.

## Alternatives Considered

### A. Central reconciler / tick enumerator over the live-connection set (the retired `MtlsSupervisor`)

The worker holds (or is handed) the set of established connections and, each
reconciler tick, point-queries `liveness(&handle)` per connection, tearing
down the `Stalled` ones. This is what ADR-0069 F6 / SD-4 pinned and step
04-01 shipped.

- **Workable and confidentiality-correct** (it does tear down fail-closed).
- **Rejected**: no surveyed production dataplane (Cilium / ztunnel /
  linkerd2-proxy / Envoy) supervises per-connection liveness with a central
  enumerator — the universal pattern is per-connection self-supervision +
  kernel timeouts (research Q1/Q5). `reconcilers.md` independently
  disqualifies it: a stalled connection is not desired-vs-actual *config*
  drift, there is no declared desired-connection-set to converge to, the
  connection's own task is the natural owner of its death, and a per-tick
  enumeration is the wrong granularity (reaps up to one tick late, walks every
  live connection each tick though almost none stall). It also forces the
  worker to assemble and hand a `&[EnforcedConnection]` registry to the port
  every tick — bookkeeping that the per-connection-task model removes
  entirely. Superseded by (C)+(B) on the reliability, performance, and
  maintainability axes.

### B. Rely on kernel TCP timeouts ALONE (no per-connection self-supervision)

Set `TCP_USER_TIMEOUT` + keepalive and trust the kernel for everything; the
pump tasks do nothing special on stall.

- **Covers the entire transport-dead class** with zero userspace machinery.
- **Rejected as the *sole* mechanism**: it does not cover the **crashed-pump**
  case where the agent's own task dies/hangs while the sockets are still
  transport-healthy — the kernel sees no transport fault, so it does not RST,
  and bytes silently stop. (B)'s per-connection self-teardown closes the legs
  on the task's own EOF/error, and a crashed task dropping its legs surfaces a
  close the peer/kernel sees. (C) alone is necessary but not sufficient; (C)+(B)
  is the adopted pair. (The narrower kernel-invisible *progress*-stall — pump
  alive but not advancing — is the deferred Tier-3-spike residual above, which
  neither (B) nor (C) fully closes in v1; that is an explicit, named gap, not
  an oversight.)

### C. (CHOSEN) Kernel TCP timeouts (C) + per-connection self-supervision (B)

Lean on the kernel for transport-death and on the connection's own task for
self-teardown; no central loop. Adopted — the unanimous production precedent
and the shape `reconcilers.md` endorses (the connection's own task owns its
terminal). The residual kernel-invisible progress-stall is deferred to a
Tier-3 spike; v1 ships (C)+(B).

## Consequences

### Positive

- **Matches unanimous production precedent.** Envoy, ztunnel, linkerd2-proxy,
  and Cilium all supervise per-connection liveness this way (per-connection
  self-supervision + kernel timeouts); none runs a central liveness
  enumerator. The design stops being the odd one out.
- **Removes a bespoke per-tick enumerator and a worker-side registry.** No
  `supervise_tick`, no `&[EnforcedConnection]` assembly, no tick cadence for
  liveness. The connection's own enforce task is the single lifecycle owner.
- **Kernel-paced, fail-closed, event-driven reaping.** The kernel fires
  `TCP_USER_TIMEOUT` at the connection's own deadline; the pump task wakes on
  its own EOF/error. Transport-death is reaped promptly and tunably with no
  userspace scan; everything resolves fail-closed (no cleartext leak, blast
  radius = one connection).
- **The contract stays a clean 4-method shape.** `liveness` is retained (it
  backs the `Gone` post-teardown observable the equivalence harness and the
  F4 cleanup tests genuinely assert, and it is the reserved hook for the
  deferred progress watchdog) — see § "Contract reconciliation". No
  destructive ripple to the trait, the adapters, or the equivalence tests.

### Negative / accepted residuals

- **The kernel-invisible progress-stall is not closed in v1.** A `splice`/kTLS
  pump stuck with a record pending but the sockets transport-healthy is not
  reaped by (C)+(B) — it needs the deferred progress watchdog (Tier-3 spike,
  research Gap 2). This is a **named, deferred gap**, not a silent one; the
  `PumpLiveness::Stalled` predicate and the `derive_liveness` pure function
  are retained on the contract precisely so the watchdog wires in later
  without a contract change. v1's honest claim: transport-death and
  crashed-pump are covered; the narrow kernel-invisible progress-stall is
  deferred.
- **Tuning the socket-option values is an adapter concern.** `TCP_USER_TIMEOUT`
  / keepalive values are not operator-tunable in v1 (consistent with the
  F4/F7 compile-time-defaults posture; operator-tunability of the mTLS knobs
  is the separate #230 concern). The acceptance test asserts the observable
  reaping, not the literal values.

### Contract reconciliation (`MtlsEnforcement` — binding on DELIVER, 05-01)

The locked 4-method contract (`probe`/`enforce`/`liveness`/`teardown`) is
**UNCHANGED in shape**. The reconciliation is a re-homing of the F6 supervision
*consumer*, not a method/variant change:

- **`teardown` — STAYS, unchanged.** Under (B) the per-connection enforce task
  calls `teardown` on its own EOF/error/`ETIMEDOUT` self-teardown; under any
  future progress-watchdog it remains the stall-recovery action. Still the
  Phase-4 close path. No change.
- **`liveness` — STAYS (all 4 methods retained).** Decision and justification:
  `liveness` has **live v1 consumers independent of the retired central
  loop**, so it is NOT dead/aspirational surface:
  1. The **post-teardown `Gone` observable** that the equivalence harness
     (`crates/overdrive-sim/tests/acceptance/mtls_enforcement_equivalence.rs`)
     and the F4 cleanup tests
     (`crates/overdrive-dataplane/tests/integration/mtls_guardrails.rs`)
     re-query to assert *no fd/kTLS leak after teardown*. This is the SD-2
     "worker observes liveness" surface and the F4 leak-free invariant —
     genuinely exercised today, with no relation to the central enumerator.
  2. The **(B) self-supervision verdict** — `PumpLiveness::Stalled` derived by
     the pure `derive_liveness` function
     (`crates/overdrive-dataplane/src/mtls/supervision.rs`) — is the predicate
     the per-connection task consumes internally to decide self-teardown, and
     the **reserved hook** for the deferred progress watchdog
     ([#232](https://github.com/overdrive-sh/overdrive/issues/232)).

  Keeping `liveness` is therefore *more* honest than dropping to a 3-method
  contract, on two project rules: (a) `development.md` § Documentation "No
  aspirational docs / no dead surface" is satisfied because `liveness`→`Gone`
  is a real, asserted observable (not aspirational), and `Stalled` is the (B)
  verdict + reserved watchdog hook (not dead); (b) the single-cut greenfield
  migration discipline favors NOT churning a contract whose `liveness`/`Gone`
  half is load-bearing today — dropping to 3 methods would rip the no-leak
  observable out of the equivalence harness and the F4 guardrail tests and
  force a re-add (and a re-ripple to `HostMtlsEnforcement`,
  `SimMtlsEnforcement`, and the 04-01 guardrail tests) the moment the Tier-3
  spike lands the watchdog. The cost of keeping 4 methods is a docstring
  reword; the cost of dropping to 3 is two contract churns and a lost
  observable.

  **What changes on `liveness` is its docstring, not its signature**: the
  "F6 supervision policy" block that said "the worker (D-MTLS-10)
  point-queries this on its reconciler-tick cadence (SD-4)" is replaced with
  "v1 supervision is (C) kernel `TCP_USER_TIMEOUT`/keepalive on the legs + (B)
  per-connection self-teardown in the enforce task; `liveness` is the SD-2
  observe surface (the equivalence harness re-queries it for the `Gone`
  no-leak assertion) and the reserved predicate for the deferred
  kernel-invisible progress-stall watchdog (ADR-0070; Tier-3 spike, #232). No
  central worker point-query, no `supervise_tick`, no tick cadence in v1." The
  SD-4 "point-query vs event-stream" sub-decision is moot for v1 liveness —
  neither variant runs; `liveness` is consulted by the post-teardown
  observable and (when the spike lands) the per-connection watchdog, not a
  central reactor.
- **`enforce` — STAYS, unchanged in signature; gains the (C) leg-setup as an
  adapter postcondition.** During `enforce` the adapter sets
  `TCP_USER_TIMEOUT` + keepalive on the legs before starting the pumps (an
  adapter HOW, hidden behind SD-2). No signature/variant change.
- **`InterceptedConnection` / `EnforcedConnection` / `Routed` / `Direction` /
  `MtlsLimits` / `MtlsEnforcementError` — all UNCHANGED.** `EnforcedConnection`
  remains the opaque handle `liveness`/`teardown` key on; `Routed`/`Direction`
  are the F3 routing facts; `MtlsLimits` keeps `pump_stall_deadline` (the
  retained `Stalled` predicate's threshold — now consumed by the (B)
  self-supervision verdict and the deferred watchdog, not a central tick).
- **`PumpLiveness` — STAYS with all three variants (`Running`/`Stalled`/
  `Gone`).** `Gone` is the post-teardown observable (live consumer). `Running`
  and `Stalled` are the (B) self-supervision verdict + the reserved
  progress-watchdog predicate. No variant is removed.

### Retiring the central `MtlsSupervisor` (step 04-01) — direct the deletion (DELIVER does it; this ADR does NOT touch `crates/**`)

`crates/overdrive-worker/src/mtls_supervisor.rs` (`MtlsSupervisor` +
`supervise_tick(&[EnforcedConnection])`) is the concrete instance of the
rejected shape (A). Per `.claude/rules/development.md` § "Deletion discipline"
(removed is removed — no gate, no salvage, no stub, no relocation), DELIVER
**deletes the production code AND its tests in the same commit**:

- **Delete** `crates/overdrive-worker/src/mtls_supervisor.rs` in full and its
  `pub mod mtls_supervisor;` declaration in `overdrive-worker`'s `lib.rs`.
- **Delete** `crates/overdrive-worker/tests/acceptance/
  mtls_supervisor_teardown_on_stall.rs` in full (both tests) and its module
  wiring in the acceptance entrypoint.
- This is a **delete, not a refactor-in-place**: the central enumerator does
  not migrate into the worker boot path. The per-connection self-supervision
  (B) lives inside the SD-2 port-owned enforce task (the host adapter), NOT in
  `overdrive-worker`. The worker's only mTLS lifecycle role is the 05-01
  intercept-install + leg-acquire + `enforce` drive (D-MTLS-14/15); it does
  NOT run a liveness loop.
- The dataplane-side `derive_liveness` pure function and the
  `PumpLiveness`/`MtlsLimits::pump_stall_deadline` surface are **retained**
  (they are the (B) self-supervision verdict + the deferred-watchdog
  predicate, NOT the central enumerator). Do not delete them. (Their telemetry
  events `mtls.pump.stalled` / `mtls.pump.teardown_on_stall` re-home from the
  retired `MtlsSupervisor` to the per-connection self-teardown path — the
  events survive; their emitter moves.)

### The 05-01 worker composition under (C)+(B) — pinned so the crafter is unblocked

With (A) gone, the registry/tick-loop architecture gap evaporates and the
05-01 composition is exactly the D-MTLS-14/15 shape plus the enforce-port
injection. Pinned:

- **Enforce-port injection seam.** The worker component that owns the `enforce`
  call holds `Arc<dyn MtlsEnforcement>` as a **mandatory constructor
  parameter** — NOT a builder. This reconciles with
  `.claude/rules/development.md` § "Port-trait dependencies" (port deps are
  required, not defaulted; builders are an anti-pattern *for port traits*).
  The `ProbeRunner` precedent uses a `.with_probe_runner(...)` builder because
  `ProbeRunner` is a *concrete* type composed inside the driver; for a `dyn`
  **port** like `MtlsEnforcement` the rule mandates a required `new()`
  parameter. Concretely: `MtlsEnforcement` is injected into the worker
  component that drives `on_alloc_running`'s intercept-and-enforce work — the
  same `ExecDriver`-owning composition the `ProbeRunner` lives in. The
  construction site is the binary composition root
  (`crates/overdrive-control-plane/src/lib.rs`, the `compose_production_driver`
  helper / `run_server` boot path, ~line 1147–1214, where `ExecDriver` +
  `ProbeRunner` are composed today): the host adapter
  `HostMtlsEnforcement` (built over `overdrive-dataplane`'s mTLS surface +
  `IdentityRead` + `MtlsLimits`) is constructed there, **probed** (wire →
  probe → use; `probe()` on `Ok` declares it usable, on failure the node
  refuses to start with `health.startup.refused`), and the probed
  `Arc<dyn MtlsEnforcement>` is threaded into the driver/worker component as a
  mandatory `new()` parameter — structurally mirroring the
  `compose_and_probe_runner_gate` → `with_probe_runner` Earned-Trust threading
  for `ProbeRunner`, but as a required port parameter rather than a builder.
  Name the seam in the dispatch: the field is the worker component's
  `Arc<dyn MtlsEnforcement>`; the construction site is `compose_production_driver`
  / the `run_server` boot path; the test composition injects
  `Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()))`.
- **Lifecycle drive (the established sync-seam → async-spawn precedent).**
  `Driver::on_alloc_running(&AllocationSpec)` (sync,
  `crates/overdrive-worker/src/driver.rs:783`) is the seam that, in v1, spawns
  the per-alloc intercept-and-enforce work — ProbeRunner-style (the same hook
  that today fires `ProbeRunner::start_alloc` after the action-shim commits
  `AllocStatusRow{state: Running}`). The per-connection enforce task: (a) the
  worker's D-MTLS-14/15 intercept-setup primitives accept the intercepted leg
  → build `InterceptedConnection` → call `enforce`; (b) the adapter's enforce
  sets the (C) `TCP_USER_TIMEOUT`/keepalive on its legs and starts the SD-2
  port-owned pumps; (c) the pump task self-tears-down (B) on EOF/error. The
  needs-intercept signal is `DriverType::Exec`-derived (D-MTLS-15; no new
  `AllocationSpec` field). `Driver::on_alloc_terminal(&AllocationId)`
  (`driver.rs:796`) tears down the alloc's connections.
- **Per-alloc teardown bookkeeping (NOT a central liveness registry).** Who
  owns the handle set for terminal teardown: a **per-alloc teardown set** — the
  worker component holds, per `AllocationId`, the `EnforcedConnection` handles
  it enforced (the same lifecycle-bookkeeping shape `ProbeRunner` holds its
  per-alloc probe tasks), so `on_alloc_terminal` can `teardown` them when the
  alloc goes terminal. This is **lifecycle bookkeeping, not a liveness loop**:
  it is keyed by alloc lifecycle (start/terminal), never enumerated each tick,
  never point-queries `liveness` for stall. It is the direct analogue of
  `ProbeRunner`'s per-alloc supervisor map, not of the retired
  `supervise_tick`. (A `BTreeMap<AllocationId, Vec<EnforcedConnection>>`-shape
  per-alloc set, drained on terminal — deterministic-collection per
  `development.md` § "Ordered-collection choice".)
- **State plainly: no central registry, no `supervise_tick`, no tick cadence
  in v1.** The worker holds per-alloc teardown bookkeeping for `on_alloc_terminal`
  and nothing else; liveness is (C) kernel + (B) per-connection task.

### Supersession relationship to ADR-0069

This ADR **refines, does not contradict, ADR-0069**. ADR-0069's locked core
(the universal/undisableable agent-light proxy model D-MTLS-1, the fold, OQ-2,
SD-1(a), SD-2 port-owns-pump, SD-3, the 4-method contract, F3, F4/F7, F5, the
authn-only boundary) is UNCHANGED. The single clause this ADR replaces is the
**F6 supervision shape** in ADR-0069 § ATAM ("the worker… point-queries
`liveness(&handle)` on its existing reconciler-tick cadence (SD-4 point-query,
not a push stream in v1)") and the SD-4 *supervision-shape* framing in the
feature-delta. The `Stalled` predicate, the fail-closed teardown reaction, the
telemetry, and the no-leak `Gone` assertion ADR-0069 F6 specified all survive
— re-homed from a central worker query to (C) the kernel + (B) the
per-connection task. ADR-0069's status line / F6 text is NOT edited inline
(immutability — supersede, never modify); this ADR is the record that the F6
supervision shape is now (C)+(B), and the feature-delta + wave-decisions point
here.

## Deferrals (named)

This ADR names one deferred unit of work (a Tier-3 spike, tracked as GH
**#232**) and one piece of forward design rationale (not a tracked unit of v1
work):

1. **Kernel-invisible progress-stall watchdog — deferred to #232 (Tier-3
   spike).** The per-connection progress predicate for a **kTLS-spliced** pump
   (`tcpi_notsent_bytes` vs kTLS record sequence vs `splice` return
   semantics) is undocumented upstream (research Gap 2) and is a kernel-
   mediated mechanism with no test backstop — Tier-3-spike before locking. v1
   ships (C)+(B); the watchdog is the deferred residual, **tracked as
   [#232](https://github.com/overdrive-sh/overdrive/issues/232)** ("Tier-3
   spike: kernel-invisible progress-stall watchdog for the kTLS-spliced mTLS
   pump (F6 residual)"). The `PumpLiveness::Stalled` predicate is retained on
   the contract as its reserved hook.
2. **Phase-5 policy-plane force-close (revocation / authz-driven drain) —
   forward design rationale, NOT a tracked unit of v1 deferred work.** A
   central connection registry + control loop IS the right shape for a
   *future* policy-plane force-close concern (revocation / authz-driven
   graceful drain — the ztunnel `ConnectionManager` precedent), but that is
   config reconciliation, NOT v1 liveness, and is out of #26's v1 scope. This
   note is forward design rationale (it records *why* the central-registry /
   reconciler shape is right for that future concern, so the central-loop
   shape is not mistakenly resurrected for v1 liveness) — it is **not** a
   tracked unit of deferred work and gets no dedicated issue. The future home
   for that mechanism is the existing
   [#37](https://github.com/overdrive-sh/overdrive/issues/37) (the central
   per-allocation live-connection registry + drain detector) and
   [#82](https://github.com/overdrive-sh/overdrive/issues/82) (gossip-
   propagated certificate revocation); neither is claimed to cover
   "revocation-driven mTLS force-close" as planned work today.

## Enforcement

- **Architectural rule (ArchUnit-style, Rust).** No central per-tick
  enumeration of the live-connection set for liveness exists in
  `overdrive-worker` after this ADR — `MtlsSupervisor` / `supervise_tick` are
  deleted, and no replacement loop is introduced. The worker's mTLS surface is
  the 05-01 intercept-install + leg-acquire + `enforce` drive + per-alloc
  teardown bookkeeping (D-MTLS-14/15); it does NOT point-query `liveness` on a
  tick. (A reviewer rejects any re-introduction of a `&[EnforcedConnection]`
  per-tick walk for liveness.)
- **Per-connection self-supervision is enforced where the pump lives.** (B) is
  inside the SD-2 port-owned enforce task in the host adapter — the same place
  the pumps run — NOT in `overdrive-worker`. The dst-lint crate-class
  boundaries (ADR-0003) keep this off any `core`-class compile path.
- **(C) leg-setup is an `enforce` adapter postcondition** (set
  `TCP_USER_TIMEOUT`/keepalive on the legs before starting the pumps); the
  acceptance test asserts the observable — a peer-dead connection is reaped to
  `Gone`, no fd/kTLS leak — not the literal `setsockopt` values.
- **The retained `liveness`/`Gone` no-leak observable stays asserted** by the
  equivalence harness (`mtls_enforcement_equivalence.rs`) and the F4 guardrail
  tests (`mtls_guardrails.rs`) — re-querying `liveness(&handle) == Gone` after
  teardown. Deleting `liveness` would break these; it is retained for exactly
  this reason (§ Contract reconciliation).

## References

- **Research (the evidence base):**
  `docs/research/dataplane/transparent-mtls-connection-supervision-research.md`
  (Nova, 2026-06-14; 22 sources; Q1–Q5 — per-connection-self-supervision-is-
  universal, the `reconcilers.md` doctrine point, the two-stall-class split,
  Gap 2 the kTLS-spliced progress predicate). The decisive findings: 2.2
  (ztunnel's central loop reconciles policy, not liveness), 3.3 (Linkerd's
  `TCP_USER_TIMEOUT` kernel-delegation), 5.3 (the kernel cannot detect
  progress-stalls), Q5 (the (C)+(B)-not-(A) recommendation).
- **Refined by this ADR:** ADR-0069 § "Sensitivity / trade-off points (ATAM)"
  → "Pump supervision policy (F6)" (the SD-4 central-point-query shape this
  ADR replaces); the feature-delta § "RE-review revisions" F6 row and the
  `MtlsEnforcement` `liveness` "F6 supervision policy" docstring.
- **Reconciler doctrine (independent disqualifier of shape A):**
  `.claude/rules/reconcilers.md` § "The decision rule" / "Not a candidate"
  (a stalled connection is not desired-vs-actual config drift).
- **Contract (read-only ground truth):**
  `crates/overdrive-core/src/traits/mtls_enforcement.rs` (the 4-method
  `MtlsEnforcement` trait, `PumpLiveness`, `MtlsLimits::pump_stall_deadline`).
- **Retired by this ADR:** `crates/overdrive-worker/src/mtls_supervisor.rs`
  (`MtlsSupervisor`/`supervise_tick`) +
  `crates/overdrive-worker/tests/acceptance/
  mtls_supervisor_teardown_on_stall.rs` (DELIVER deletes both, one commit).
- **Retained (the (B) verdict / deferred-watchdog predicate, NOT the
  enumerator):** `crates/overdrive-dataplane/src/mtls/supervision.rs`
  (`derive_liveness` pure fn).
- **05-01 composition precedents:** `crates/overdrive-control-plane/src/lib.rs`
  (`compose_production_driver` / `run_server` boot path, ~1147–1214 — the
  Earned-Trust gate → port-injection precedent); `crates/overdrive-worker/
  src/driver.rs:783/796` (`on_alloc_running` / `on_alloc_terminal` — the
  sync-seam → async-spawn lifecycle precedent); wave-decisions.md D-MTLS-14/15
  (intercept-setup primitives + input provenance).
- ADR-0069 (the universal agent-light L4 proxy, refined here in F6 only);
  ADR-0068 (pinned 6.18 LTS kernel floor — `TCP_USER_TIMEOUT`/keepalive
  in-tree); `.claude/rules/development.md` § "Deletion discipline" /
  "Port-trait dependencies" / Documentation.
- Phase-5 policy plane / revocation (the future central-registry home, NOT
  built now — forward design rationale, not tracked v1 work): whitepaper §8;
  the related future mechanisms [#37](https://github.com/overdrive-sh/overdrive/issues/37)
  (central per-alloc live-connection registry + drain detector) and
  [#82](https://github.com/overdrive-sh/overdrive/issues/82) (gossip-propagated
  revocation); the authorization boundary out of #26 scope (#27 BPF-LSM
  `socket_connect`; #178 expected-peer SAN-match).

---

## Amendment (2026-07-01) — the directional clean-close class: forward the FIN as a half-close (backend-instance-replacement / #249)

### Status of this amendment

**Accepted (2026-07-01).** This amendment **adds a third stall/close class**
to the two-class table in § Context ("Two stall classes — and which mechanism
owns each") and pins the mechanism that owns it. It **does not touch** the (C)
kernel-timeout + (B) self-teardown decision, the retirement of the central
`MtlsSupervisor`, the 4-method contract, or any locked ADR-0069 core. The one
thing it changes: on a **clean directional close** (a peer FIN, classified
`PumpExit::Graceful` on a *source* leg), the pump now **forwards the FIN to the
opposing leg as a half-close (`shutdown(dst, SHUT_WR)`)** instead of leaving
the opposing leg open with no path to EOF. The `Graceful`-non-reclaim gate
(D-MTLS-16 half-close correctness) is **retained unchanged** — a clean close is
still not a connection reclaim; it is now *propagated* rather than *dropped*.

### Context — the gap this closes (RCA-sourced)

`docs/analysis/root-cause-analysis-in-flight-churn-fail-fast-gap.md` (Rex,
2026-07-01, source-read evidence) found that the S-DBN-CHURN Tier-3 oracle AT
cannot go green because a backend that closes its connection **cleanly** (a
normal FIN — exactly what a graceful `overdrive workload restart` stop of the
backend, or any well-behaved one-response-then-close server, produces) is
**invisible to BOTH v1 liveness mechanisms**:

- **(B) self-teardown fires ONLY on `PumpExit::TransportDeath`** (a leg error /
  the (C) kernel-reaped `ETIMEDOUT`). A clean FIN is classified
  `PumpExit::Graceful` and `mark_exited` deliberately does NOT fire the
  self-teardown for `Graceful` — the D-MTLS-16 gate that stops a one-directional
  half-close from nuking a connection whose sibling direction is still live.
  (Evidence: `crates/overdrive-dataplane/src/mtls/splice.rs` —
  `run_decrypt_pump` breaks `Graceful` on `POLLHUP`-with-no-`POLLIN` / `n_in
  == 0`; `mark_exited` gates the fire on `TransportDeath` only.)
- **(C) `TCP_USER_TIMEOUT` reaps only *unacked* transport death.** A socket the
  peer FIN-closed *cleanly* has nothing unacked outstanding, so
  `TCP_USER_TIMEOUT` has nothing to time against and never fires. (Evidence:
  `arm_transport_death_timeouts` rustdoc — it reaps a peer that *vanished*
  without a FIN/RST, not one that *closed*.)

The two-class table pinned **transport-dead** (kernel (C) reaps) and
**progress-stuck / kernel-invisible** (deferred watchdog, #232). **A clean
directional FIN is neither.** It is a legitimate, benign, common close — but
today the pump absorbs it (`Graceful` non-reclaim) and **never propagates it to
the opposing leg**, so a client holding an in-flight connection through the
mesh hangs on a blocking read of the workload-facing leg (leg-F) until the
30 s test bound, because leg-F's write side is never shut down and its read
never surfaces EOF. The consequence is exactly the "in-flight churn never fails
fast" symptom the S-DBN-CHURN AC forbids.

### The third class (table addendum to § Context "Two stall classes")

| Close/stall class | Cause | Owner in v1 | Mechanism |
|---|---|---|---|
| **Transport-dead** | peer gone, unacked past deadline, half-open | kernel (C) | `TCP_USER_TIMEOUT` + keepalive → `ETIMEDOUT`/RST; pump observes error → (B) self-teardown |
| **Directional clean-close** *(this amendment)* | a leg's peer FIN-closed cleanly (graceful exit, SIGTERM'd process, one-response-then-close) | **the pump task (B)** | on `PumpExit::Graceful` from a *source* EOF, forward the FIN to the opposing leg as `shutdown(dst, SHUT_WR)` — the peer's FIN mirrors to the other side; **no full reclaim** (sibling direction may still be live); the connection reclaims naturally once *both* directions have closed |
| **Progress-stuck (kernel-invisible)** | sockets TCP-healthy but the splice/kTLS pump not advancing | **deferred (#232)** | per-connection progress watchdog on the retained `Stalled` predicate — undocumented kTLS-splice progress signal, Tier-3 spike |

### Decision

**On a `PumpExit::Graceful` that is a genuine *source* directional EOF (a peer
FIN observed on the pump's source leg — NOT a deliberate `teardown`), the pump
forwards the close to its opposing leg by `shutdown(dst_fd, SHUT_WR)` before
`mark_exited`, then exits `Graceful` (still non-reclaiming).** This is standard
transparent-proxy **half-close forwarding**. It is NOT `sock_destroy` (#61
scope, excluded), NOT a full connection reclaim, and NOT "fire self-teardown on
`Graceful`" (which would break half-close correctness — a backend that finished
reading but is still writing would be wrongly nuked).

Concretely, mapping to the pinned leg topology (ADR-0069; verified in
`outbound.rs` / `inbound.rs`), **every pump's own `dst_fd` IS the opposing leg
the FIN must reach** — no new plumbing, no new pump parameter:

| Direction | Pump | src → dst | On a source clean-close, half-close forward |
|---|---|---|---|
| OUTBOUND | forward (encrypt) | leg-F → leg-B | client(leg-F) FIN → `SHUT_WR` leg-B |
| OUTBOUND | return (decrypt) | leg-B → leg-F | **backend(leg-B) FIN → `SHUT_WR` leg-F — the S-DBN-CHURN fix** |
| INBOUND | deliver (decrypt) | leg-C → leg-S | client(leg-C) FIN → `SHUT_WR` leg-S |
| INBOUND | response (encrypt) | leg-S → leg-C | server(leg-S) FIN → `SHUT_WR` leg-C |

### Half-close correctness is PRESERVED — the load-bearing invariant

The `Graceful` gate exists so a clean EOF on one direction does not tear down a
connection whose **sibling direction is still live** (D-MTLS-16). The
half-close forward **preserves** this exactly:

- **`SHUT_WR` half-closes only the write side of the opposing leg.** The peer on
  that leg sees EOF on its read but its own write direction stays open — if it
  is still sending (a legitimate `shutdown(SHUT_WR)`-then-keep-reading peer, or a
  backend that finished reading the request but is still streaming its
  response), the sibling pump keeps moving those bytes. Nothing is nuked.
- **No full reclaim on `Graceful`.** `mark_exited` still does NOT fire the (B)
  self-teardown for `Graceful` — the connection's `ConnState` (both legs, kTLS
  state) survives until the *sibling* direction also reaches its terminal.
- **The connection reclaims cleanly once BOTH directions have closed.** When the
  sibling pump then observes *its* source EOF (or the now-half-closed leg's
  peer completes its own close), it forwards its own `SHUT_WR`; with both write
  sides shut, both legs reach full EOF and the pumps exit. A fully-closed
  connection is then reclaimed via the **shutdown/terminal-teardown path**
  (`on_alloc_terminal` → `teardown` → `reclaim_connection`), draining the
  `ConnState` entry. (See "The `ConnState`-linger residual" below — the A1 fix
  also removes the observed 120 s process-linger.)

### The kTLS-TX-dst subtlety — pinned (encrypt-pump direction)

The two *decrypt* pumps (OUTBOUND return, INBOUND deliver) forward into a
**plaintext** `dst` (leg-F / leg-S), where `shutdown(SHUT_WR)` sends an ordinary
TCP FIN — this is the S-DBN-CHURN fix and is unambiguously correct. The two
*encrypt* pumps (OUTBOUND forward, INBOUND response) forward into a **kTLS-TX**
`dst` (leg-B / leg-C). `shutdown(SHUT_WR)` on a kTLS-TX socket sends a bare TCP
FIN **without** a TLS `close_notify` alert. **This is accepted for v1:** the
peer's kTLS-RX observes the transport FIN as a clean read-EOF (which is what the
opposing pump needs to surface its own close), and Overdrive's own leg on the
other side of that FIN is a pump that treats EOF as `Graceful` — it does not
require a `close_notify` to make progress. Emitting a TLS `close_notify` on
kTLS-TX before the FIN (the fully-graceful TLS shutdown) is **out of scope for
this amendment** — it is a mechanism nicety, not a correctness requirement for
the fail-fast contract, and no v1 consumer distinguishes "FIN" from
"close_notify then FIN". The uniform `shutdown(dst, SHUT_WR)`-on-`Graceful`
rule therefore applies to **all four** pumps; the encrypt-pump direction sends a
bare FIN by design. (If a future consumer needs the TLS-graceful close — an
external non-Overdrive peer that strictly requires `close_notify` — that is a
separate, later refinement, not a fail-fast blocker.)

### The failed-`SHUT_WR` error model — Option B invariant tripwire (pins the `let _` gap)

The A1 decision above (2026-07-01) pinned the **success** model of the
half-close forward (`SHUT_WR` mirrors the FIN; do NOT reclaim on `Graceful`)
but left the **failure** model of `libc::shutdown(dst_fd, SHUT_WR)` — what a
`-1` return means and what to do about it — unpinned. The first implementation
(`forward_half_close_if_source_eof`, `splice.rs:252-269`) discarded the `c_int`
return with `unsafe { libc::shutdown(dst_fd, SHUT_WR); }`. Code review flagged
the bare discard. This addendum pins that error model. **Accepted (2026-07-01),
same amendment lineage.**

**Fact base (verified against the code, not re-derived):**

1. **`EBADF` / `ENOTSOCK` are structurally UNREACHABLE at this call unless a
   platform ownership invariant is violated.** The only site that closes a leg
   fd is `reclaim_connection` (`mod.rs:242-256`), which `stop_and_join()`s the
   primary pump AND every aux pump BEFORE `drop(state.legs)`. A pump executing
   `forward_half_close_if_source_eof` is by definition not-yet-joined, so
   reclaim is blocked at the join and cannot have closed `dst_fd`; the pumps
   `std::mem::forget(dst)` and never close leg fds themselves. `dst_fd` is
   therefore provably live at the shutdown call. `EBADF`/`ENOTSOCK` can appear
   ONLY if the join-before-close ownership invariant is broken — a **platform
   logic error**, not a runtime race.
2. **The only reachable non-zero return is `ENOTCONN`** — the "peer already
   gone / dst write side already shut" case the A1 docstring already documents
   as a **deliberate harmless no-op** (`splice.rs:243-248`, the
   `splice_pipe_to_dst` `n_out == 0` destination-clean-EOF shape grouped into
   the forward path on purpose). On Linux, `SHUT_WR` on a connected socket
   whose peer has already FIN-closed still returns `0`; `ENOTCONN` is the
   not-connected shape.
3. **A genuinely-dead `dst_fd` is already backstopped** by the sibling pump:
   the pump that also touches that leg hits a transport error →
   `PumpExit::TransportDeath` → `mark_exited` fires the (B) self-teardown →
   `reclaim_connection`. Even absent any change, a truly-dead leg is reclaimed
   via the sibling, so the failure is not a silent eternal hang.

**Decision — Option B (invariant tripwire; a pure diagnostic, no behavior
change).** Inspect the `shutdown` return. Treat `{0, ENOTCONN}` as success and
stay silent (both are the documented no-op / success shapes of a correct
half-close). On **any other errno** — the `EBADF`/`ENOTSOCK` family that fact 1
proves can only mean the join-before-close ownership invariant was violated —
emit a structured `tracing::error!` naming the violated invariant, and do
**nothing else**: the forward's job is done (the FIN either propagated or the
leg was already gone), the connection still reclaims via the sibling backstop
(fact 3) and the normal terminal-teardown path. The event is a **tripwire that
surfaces a platform bug**, not a runtime condition the datapath defends against.

- **Rejected — Option A (`let _ = shutdown(...)`).** Silently absorbing the
  return violates `development.md` § "Errors" ("never silently absorb a
  fallible boundary read into a default") and § "Check-and-act must be atomic"
  (a discarded syscall verdict). The one non-success errno that is genuinely
  benign (`ENOTCONN`) is *known and named*, so the honest shape allow-lists it
  explicitly rather than blanket-discarding — the same discipline the sibling
  `set_best_effort_tcp_opt` (`mod.rs:544-577`) already applies to
  `setsockopt`'s `EOPNOTSUPP`/`ENOPROTOOPT`. A blanket `let _` would also
  swallow the `EBADF` tripwire that is this addendum's whole point.
- **Rejected — Option C (fail-closed: escalate an unexpected failure to
  `mark_exited(TransportDeath)` / self-teardown).** Over-engineered for the
  reachable fact base. The only errno Option C would escalate on is the
  unreachable-unless-invariant-violated `EBADF`/`ENOTSOCK` family (fact 1);
  `ENOTCONN` is the documented no-op and must NOT escalate. Escalating a
  should-never-happen platform-logic error into a connection teardown adds a
  reclaim path that the sibling backstop (fact 3) + terminal-teardown already
  cover, buys no reliability the tripwire + backstop don't already give, and
  couples the half-close forward to the teardown machinery it was deliberately
  kept out of. (For the record — asked and answered so it is not re-litigated:
  escalating on the *error* path would NOT violate D-MTLS-16. D-MTLS-16 governs
  the *successful* clean half-close — "a clean `Graceful` close does not
  reclaim a connection whose sibling may be live." A failed `SHUT_WR` is not a
  clean half-close; it is a platform-invariant violation. So D-MTLS-16 does not
  forbid Option C — Option C is rejected on the "unreachable errno / redundant
  with the backstop / needless coupling" grounds above, not on a D-MTLS-16
  conflict.)

**The pinned error model (binding on the crafter — no new public API surface):**

- **Errno allow-list (silent):** `0` (success) and `libc::ENOTCONN` (the
  documented harmless no-op — peer already gone / write side already shut). No
  event, no assert. These are the two shapes of a *correct* half-close forward.
- **Errno tripwire (any other errno, e.g. `EBADF` / `ENOTSOCK`):** emit
  `tracing::error!(name: "mtls.pump.half_close_forward_failed", errno = <raw>,
  …)` — severity **`error`** (this is a should-never-happen ownership-invariant
  violation, strictly more severe than the existing `warn`-level
  `mtls.pump.stalled` / `mtls.transport_death.unsupported` events, per the
  module's `mtls.pump.*` / `mtls.transport_death.*` naming idiom). The message
  names the violated invariant: *the join-before-close leg-ownership guarantee
  (`reclaim_connection` joins every pump before `drop(state.legs)`) — a live
  pump's `dst_fd` must never be closed underneath it.* No behavior change: the
  connection still reclaims via the sibling `TransportDeath` backstop + the
  terminal-teardown path.
- **`debug_assert!` — INCLUDED.** Add `debug_assert!(false, "…")` on the
  tripwire branch (dev/test builds fault-inject-and-catch the invariant break;
  release builds emit the structured `error` event and continue). This matches
  `development.md` § "Logically unreachable … use `unreachable!()`" *intent*
  (signal a logic error, don't silently absorb) while staying non-fatal in
  production — a `SHUT_WR` failure is a diagnostic signal, not a reason to
  panic a live node handling other connections. (`unreachable!()` /
  `.expect()` are wrong here: both abort the process, and `.expect()` is banned
  in production library code — the errno is *unreachable-unless-a-bug*, which is
  exactly the `debug_assert!` + structured-error shape, not an unconditional
  panic.)
- **No new API surface.** No classifier enum, no new `PumpExit` variant, no new
  error type — a two-arm `match` on `std::io::Error::last_os_error()
  .raw_os_error()` at the existing call site, structurally identical to the
  in-module `set_best_effort_tcp_opt` errno `match` (`mod.rs:564-577`). Adding a
  type here would be the invented-surface anti-pattern CLAUDE.md § "Implement to
  the design — never invent API surface" forbids.

### The `teardown`-vs-source-EOF distinction — do NOT forward on deliberate teardown

`PumpExit::Graceful` is returned in **three** shapes in the pump loops, and only
the *source-EOF* shapes forward:

1. **Deliberate `teardown` set `stop == true`** (`run_decrypt_pump` /
   `run_encrypt_pump` top-of-loop `if state.stop { break Graceful }`) — the
   connection is **already being reclaimed** by `teardown` /
   `reclaim_connection`; forwarding a `SHUT_WR` here is redundant at best and a
   double-op at worst. **Do NOT forward on this shape.**
2. **Source clean EOF** (`POLLHUP`-with-no-`POLLIN`, `n_in == 0`, or the
   `splice_pipe_to_dst` `n_out == 0` dst-EOF) — a genuine peer FIN. **Forward.**

The implementation MUST distinguish these. The pinned shape (binding on the
crafter, no invented surface): the half-close forward is gated on **`!state.stop
&& exit == Graceful`** — the SAME `stop`-guard `fire_self_teardown_if_unexpected`
already uses to distinguish a deliberate teardown from an unexpected exit. Reuse
that guard; do not add a new `PumpExit` variant unless the crafter finds the
existing `Graceful` + `stop`-guard genuinely cannot express the distinction (in
which case: STOP and surface the gap, per CLAUDE.md "Implement to the design").
The likely-cleanest expression is a small helper `forward_half_close_if_source_eof(dst_fd, exit, state)`
called from the single terminal-exit site alongside `mark_exited` — but the
crafter owns the internal factoring (GREEN/REFACTOR); the design pins only the
observable: *a source directional clean-close forwards `SHUT_WR` to the opposing
leg; a deliberate teardown does not.*

### The `ConnState`-linger residual — same root, resolved by A1

The RCA's secondary finding (the 120 s process-linger after the panic, to
nextest's SIGKILL) is the **same** `Graceful`-non-reclaim: on `skeleton.shutdown()`,
a connection whose pumps exited `Graceful` without reclaim leaves a `ConnState`
entry the shutdown path does not force-close synchronously. Under A1, a clean
backend close now forwards `SHUT_WR` to leg-F → the client's leg-F read surfaces
EOF → the client closes → the sibling (forward) pump observes *its* source EOF →
forwards its own `SHUT_WR` → both legs fully close → the pumps exit → the
terminal-teardown path reclaims the `ConnState`. **A1 resolves the linger as a
consequence** — it is not a separate fix. (If a residual linger survives A1 —
e.g. a connection where the client never closes its half after the backend FIN
— the `on_alloc_terminal` → `teardown` path is the backstop, and `shutdown()`'s
existing per-alloc teardown drains the set; the crafter confirms no `ConnState`
survives `skeleton.shutdown()` in the S-DBN-CHURN AT as the linger-gone proof.)

### Relationship to the deferred #232 progress-stall watchdog — ORTHOGONAL

This amendment is **not** the #232 watchdog and does not close it. #232 covers
the **kernel-invisible progress-stall** — a pump *stuck* while both legs are
TCP-healthy and a record is pending (no FIN, no error, just no advance). A1
covers a **clean directional close** — a peer that *did* FIN. They are different
classes with different signals (a FIN the `poll`/`splice` sees vs. a
frozen-progress deadline the retained `Stalled` predicate would evaluate). A1
ships in v1; #232 stays deferred. The `PumpLiveness::Stalled` predicate is still
retained for #232 and is untouched by this amendment.

### Enforcement (addendum to § Enforcement)

- **The half-close forward is asserted by its observable, not its `setsockopt`
  args.** The Tier-3 S-DBN-CHURN AT asserts a client holding an in-flight mesh
  connection through leg-F receives a prompt read-EOF/reset when the backend
  closes (bounded well under `TCP_USER_TIMEOUT`), and a subsequent fresh connect
  lands the new backend. A Tier-2/unit test over the pump's terminal-exit
  decision pins: a source clean EOF forwards `SHUT_WR` to `dst`; a deliberate
  `teardown` (`stop == true`) does NOT; `mark_exited` still does NOT reclaim on
  `Graceful` (half-close correctness). This sits alongside the existing
  `graceful_eof_exit_does_not_fire_self_teardown` unit test — that test's
  invariant (no reclaim on `Graceful`) is **preserved**; the new test adds the
  *forward* on top of it.
- **The failed-`SHUT_WR` tripwire is asserted by a Tier-2/unit test over the
  errno branch, not by a live `EBADF` (which is unreachable).** A unit test
  drives `forward_half_close_if_source_eof` against (a) a live `dst` → returns
  `0`, silent; (b) an already-half-closed / peer-gone `dst` → `ENOTCONN`,
  silent (no event, no assert — the documented no-op); (c) a deliberately
  bad/closed fd → the tripwire branch fires (`mtls.pump.half_close_forward_failed`
  in release; `debug_assert!` in dev). Case (c) is a *test-only* fault
  injection of the ownership-invariant break; production never reaches it unless
  a real join-before-close bug lands, which is precisely what the tripwire
  exists to surface.
- **`sock_destroy` remains excluded (#61).** A1 is a userspace-leg
  `shutdown(SHUT_WR)`; it needs no `sock_destroy`. The AC's "NO `sock_destroy`"
  constraint holds under A1.
- **`TCP_USER_TIMEOUT` remains the upper bound for the *transport-death* class.**
  A1 makes a *clean* close propagate near-instantly (a forwarded FIN, well under
  `TCP_USER_TIMEOUT`); the *transport-death* class (RST / vanish) still uses (C)
  `TCP_USER_TIMEOUT`. The S-DBN-CHURN AC's "bounded by `TCP_USER_TIMEOUT`, never
  an indefinite hang" text stays accurate as the **upper** bound (clean close is
  faster; transport death is `TCP_USER_TIMEOUT`-bounded) — see the S-DBN-CHURN
  scenario note.

### References (addendum)

- **RCA (the evidence base):**
  `docs/analysis/root-cause-analysis-in-flight-churn-fail-fast-gap.md` (Rex,
  2026-07-01) — Root Cause A (design/AC gap: clean-close invisible to (B)+(C))
  and Root Cause C (test-model error: single-shot server). This amendment
  resolves Root Cause A (A1); the test-model fix (T1/T2) lands with it.
- **The pump surface this binds:**
  `crates/overdrive-dataplane/src/mtls/splice.rs` (`run_decrypt_pump` /
  `run_encrypt_pump` / `mark_exited` / the `PumpExit::Graceful` terminal
  exits; `forward_half_close_if_source_eof` at 252-269 — the `SHUT_WR` call
  the failed-forward addendum binds; the two call sites at 448 / 538);
  `crates/overdrive-dataplane/src/mtls/mod.rs` (`reclaim_connection` at
  242-256 — the join-before-close ownership invariant fact 1 rests on; the
  terminal-teardown path). Leg topology verified in
  `crates/overdrive-dataplane/src/mtls/outbound.rs` / `inbound.rs`.
- **Failed-`SHUT_WR` error-model addendum (2026-07-01):** motivated by a code
  review of `forward_half_close_if_source_eof` (`splice.rs:252-269`) flagging
  the discarded `libc::shutdown` `c_int` return (`unsafe { libc::shutdown(…); }`
  with no inspection). The in-module errno-classification precedent the pinned
  two-arm `match` mirrors is
  `set_best_effort_tcp_opt` (`crates/overdrive-dataplane/src/mtls/mod.rs:544-577`,
  allow-list `EOPNOTSUPP`/`ENOPROTOOPT` → warn-and-skip, propagate the rest);
  the `mtls.pump.*` / `mtls.transport_death.*` event-name idiom
  (`mod.rs:209/224/569`). Governing rules: `development.md` § "Errors" /
  § "Logically unreachable … `unreachable!()`" / § "Check-and-act must be
  atomic"; CLAUDE.md § "Implement to the design — never invent API surface".
- **Driving gate:** the S-DBN-CHURN Tier-3 oracle AT
  (`in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend`,
  `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs`);
  the backend-instance-replacement roadmap step that lands A1 + the test-model
  fix before the un-ignore can go green.
- ADR-0069 (leg vocabulary: F/B outbound, C/S inbound; the bidirectional model);
  #26/#236 (kernel-mediated / transparent mTLS); #232 (the orthogonal deferred
  progress-stall watchdog); #61 (`sock_destroy`, excluded).
