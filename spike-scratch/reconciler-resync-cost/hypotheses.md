# V-3 hypotheses (written BEFORE any measurement run; debugging.md section 4)

Feature: reconciler-convergence-guarantees. Probe: `src/main.rs` (this directory). Commit under test: 97a417fcb.
N = number of workloads (each 1 replica, converged: Running alloc, SVID held, no action emitted).
Workload mix in the probe: 80% Job / 20% Service (VM driver, SimDriver). Targets per pass follow the delta's D-1a table.

## M1. Per-evaluation hydrate+reconcile time, as a function of N

- Hypothesis: `workload-lifecycle` and `svid-lifecycle` hydrate cost is dominated by `alloc_status_rows()` (a full table scan, one
  `spawn_blocking`, rkyv envelope decode per row), so it is linear in N per evaluation (quadratic per pass). `service-lifecycle` on a
  Service target reads the same table plus per-alloc probe rows, so it is also linear in N. `service-map-hydrator` and `vm-reclamation`
  are flat or linear with a small slope.
- Prediction (hydrate desired+actual, uncontended, release build, NVMe-backed redb, warm):
  N=10: 0.05 to 0.3 ms for every reconciler. N=1000: workload-lifecycle 1 to 5 ms, svid-lifecycle 2 to 8 ms (it also scans
  `issued_certificate_rows` and clones the held-SVID snapshot), service-lifecycle 1 to 6 ms on a Service target.
  Slope of workload-lifecycle hydrate: 1 to 5 microseconds per allocation row.
  The full tick adds `persist_view` (a redb write transaction with fsync): 0.2 to 2 ms, independent of N.
- Falsification: hydrate time at N=1000 is under 5x its time at N=10 (not linear), OR hydrate at N=1000 is under 0.3 ms for
  workload-lifecycle (the scan is not the dominant term).

## M2. Full pass wall time, CPU time, duty cycle against the 30 s period (G1: at most 3% at 1000 targets)

- Hypothesis: the pass is E evaluations (E = workload-lifecycle N + svid-lifecycle N + service-lifecycle 0.2N + service-map-hydrator
  0.2N + vm-reclamation 1) executed through `allocator.lock()` serialised hydration, so wall time is about E x (hydrate + persist),
  quadratic in N.
- Prediction: N=10 pass under 50 ms (duty under 0.2%). N=100 pass 0.1 to 0.6 s (duty 0.3 to 2%). N=500 pass 1 to 8 s.
  N=1000 pass 4 to 25 s, so duty cycle 13% to 80%: G1 FAILS at N=1000 by roughly 4x to 25x.
  CPU time is 50 to 120% of wall time (spawn_blocking threads add CPU the loop thread does not show).
  `EvaluationBroker::drain_pending` clones and sorts every pending key per call (O(P log P), P = pending count) and is called once
  per completion, so it adds a quadratic term of its own: measured separately, expected under 25% of the pass at N=1000.
- Falsification: N=1000 duty cycle at or under 3% (G1 holds), OR pass time grows less than 3x when N grows 2x from 500 to 1000
  (not quadratic).

## M3. Edge-wake admission delay behind a pass burst; allocator lock

- Hypothesis (a): the broker admits strictly FIFO by first-pending order, so a deploy-shaped `submit_evaluation` for a new target
  submitted just after the burst waits for every evaluation ahead of it: delay is approximately the pass wall time at that point,
  with no priority for edge wakes.
- Prediction (a): delay at N=10 under 20 ms; N=100 0.05 to 0.5 s; N=1000 within 10% of the N=1000 pass wall time (seconds).
  Baseline with no pass: under 5 ms at every N.
- Falsification (a): delay at N=1000 under 10% of the pass wall time (some other ordering lets the edge wake jump the burst).
- Hypothesis (b): every tick holds `state.allocator` (a fair tokio mutex, FIFO wait queue) across both hydrates; up to 8 ticks are
  admitted concurrently and all queue on it, so a Service deploy handler (handlers.rs:440 takes the same mutex) waits behind up to 7
  ticks' hydrate windows.
- Prediction (b): lock wait for a handler arriving during the pass: median about 3 to 4 x one hydrate hold, maximum about 8 x one hold.
  At N=1000 with 1 to 5 ms holds that is 5 to 40 ms median, 10 to 80 ms max. At N=10 under 2 ms. Idle baseline under 50 microseconds.
- Falsification (b): max wait during the pass at N=1000 is under 2 x the single-tick hydrate time (the mutex is not the queue), OR the
  wait is indistinguishable from the idle baseline.
