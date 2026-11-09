# ADR-0270: A Service with no declared startup probe is Stable when the guest reports every declared listener open

## Status

**Accepted, 2026-11-09, after user ruling and independent full DESIGN review
iteration 4** ([review](../../feature/service-allocation-readiness-and-ending/design/review.md)).
The D13 hook scope it depends on is supported by held V42
([findings](../../feature/service-allocation-readiness-and-ending/spike/findings-workload-cgroup.md)).
Not implemented. GH #318, GH #334. Exact contract: [feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md)
(D15). This accepted design replaces [ADR-0058](adr-0058-default-tcp-startup-probe-inference.md)
when implemented; ADR-0058's inferred TCP gate remains the current runtime behavior until then.
Related:
[ADR-0268](adr-0268-the-guest-reports-each-workload-listener-it-opens-over-the-beacon-channel.md),
[ADR-0269](adr-0269-a-service-with-no-declared-startup-probe-waits-on-a-guest-listener-startup-probe.md),
[ADR-0057](adr-0057-health-check-toml-spec.md) (the TOML spec), RCA
`docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md` (RCA-A).

## Context

ADR-0058 closed RCA-A ("kernel-accepted exec is not liveness") by inferring a TCP-connect
startup probe against `listeners[0]` when the operator declares none. Three problems with
the mechanism, all grounded in the production path (`overdrive serve` + `overdrive deploy`,
parser `parse_startup_probes`, `workload_spec.rs:1197-1211`):

1. **A 2 s floor.** The probe polls every 2 s, so a workload that listens in 5 ms is
   `Stable` after up to 2 s. Under D12 (a startup-probed allocation is a backend only after
   its `Stable`), that floor is also the time to first traffic.
2. **TCP whatever the listener.** The inferred probe is always TCP against `listeners[0]`'s
   port. A UDP-first Service gets a TCP probe of a UDP port and never passes, and no probe
   mechanic can express a UDP listener.
3. **Only the first listener is gated**, so a Service is `Stable`, and a backend, while a
   declared second listener is not yet open.

Dropping the inference alone would reopen RCA-A: a probe-less Service that exits 1 right
after start would be `Stable` at its first `Running` and `overdrive deploy` would report
success. The replacement must keep a startup gate that a never-listening workload cannot
pass.

## Decision

**A Service that declares no `[[health_check.startup]]` (the key is absent) gets the
guest-listener startup gate: it is `Stable` when it is `Running` and the guest has reported
every declared listener open. The explicit empty list `[[health_check.startup]] = []`
remains the opt-out: `Stable` at the first `Running`. A declared startup probe is
unchanged.**

- **All declared listeners, not `listeners[0]`.** A declared listener is a promise: the
  Service's frontend routes to every one of them, and under D12 the allocation is a backend
  for the whole Service. With a free, event-driven check the AND costs nothing, and a
  Service that declares a listener it never opens fails at the startup deadline instead of
  being a backend that refuses connections. This reverses ADR-0058's rejected Alternative C
  because its objection was the cost of polling N listeners.
- **TCP and UDP.** TCP is open at `listen()`, UDP at an explicit `bind()` (ADR-0268).
- **Deadline.** The startup deadline is the platform's 60 s (the value the inferred probe's
  `interval x max_attempts` has today), so the D10 progress deadline (`max(90 s,
  startup_deadline + 30 s)`) is unchanged for a probe-less Service.
- **Services always declare a listener.** The parser and the API reject a Service with
  none (`ListenerMissing`; `validate_service_listeners`), so the gate always has at least
  one listener to wait for.
- **Where the default is applied.** Unchanged from ADR-0058: the TOML parser materialises
  it, and the submitted spec carries the materialised descriptor. A raw API submit with an
  empty startup list is the opt-out (existing behaviour, unchanged here).

## Alternatives considered

- **Keep ADR-0058's inference.** Rejected by the user's ruling (2 s floor, TCP-only).
- **Drop inference and make a probe-less Service `Stable` at `Running` (the Kubernetes and
  Nomad default).** Reopens RCA-A: a service that exits 1 at once is reported successful.
- **Gate on `listeners[0]` only.** Keeps ADR-0058's compatibility but gates a backend on
  one of its listeners; rejected above.
- **A minimum-healthy window after the listener opens (Nomad `min_healthy_time`,
  Kubernetes `minReadySeconds`).** Narrows the residual window of RCA-A's closure
  (below) but puts a floor back under cold start and adds a spec key. Rejected; an
  operator who needs sustained health declares a readiness probe, which D12 now gates
  traffic on.
- **Report only the first listener to open.** Not a gate on the declared set.

## Consequences

- **RCA-A stays closed** (feature delta § D15 argument): a workload that never listens is
  never `Stable` and is stopped at the deadline, restarted by the restart authority and
  named `EarlyExit`/`StartupProbeFailed` at give-up (D6/D7). A workload that opens every
  listener and then exits 1 before the exit is recorded can be reported `Stable`; the
  window is the interval between the guest report reaching the probe task and the exit
  event being consumed, plus one reconcile wake. It is not zero, it is the same shape
  ADR-0058's poll had (a connect that succeeded just before the exit), and it is orders of
  magnitude shorter. The post-`Stable` crash is then an ordinary crash handled by the
  restart authority, visible in `restart_count` and `LastTerminated` (ADR-0078).
- Behaviour change for operators: a probe-less Service with a second declared listener
  that the workload opens late (or never) no longer becomes `Stable` until it opens; a
  workload that opens a declared listener lazily must use the opt-out.
- Cold start: the `Stable` and first-traffic time is no longer floored at 2 s; what remains
  is guest-to-host delivery plus the reconciler's wake (ADR-0273 makes a submit wake the
  convergence loop at once; the tick remains the backstop).
- The `(inferred)` marker disappears from the CLI and is replaced by the mechanic's own
  summary ("guest listeners tcp:8080 udp:5353"); ADR-0271 removes the flag.
