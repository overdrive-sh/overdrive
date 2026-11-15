# V-3b hypotheses: TODAY's baseline burst (written BEFORE any measurement run; debugging.md section 4)

Commit under test: 97a417fcb. Same N (10, 100, 500, 1000), same 80% Job / 20% Service mix, same settled converged state
as `hypotheses.md` / `evidence/full-20261115T180650Z/`.

## What the baseline wakes (read from source, lib.rs:5429 `list_and_route`, reconciler `interests()`)

`list_and_route` reads `alloc_status_rows()` ONLY (not `probe_result`), and for each row calls `route_observation_row`, which
submits `workload/<workload_id>` once for EVERY reconciler whose `interests()` contains `AllocStatus`:
workload-lifecycle, svid-lifecycle, service-lifecycle (service-lifecycle declares AllocStatus + ProbeResult). There is no
Service-kind filter on the AllocStatus path, so service-lifecycle is woken for Job workloads too.
Plus `vm-reclamation` `node/<id>` from its own `resync_schedule` (30 s). `gateway-svid-lifecycle` also has one but is not
registered in the probe fixture (gateway disabled, same as the prior run) so it adds 1 cheap eval in a gateway deploy only.
`service-map-hydrator` has no interest and no schedule: NOT woken by the baseline.
One alloc row per workload (replicas = 1), so baseline E = 3 x workloads + 1.

## B1. Evaluations per burst (E)
- Prediction: E = 3W + 1 with W = workloads at the pass (N + 6 idle-edge + earlier edge samples). N=10 -> about 49..55 (W 16..22);
  N=100 -> about 319..337; N=500 -> about 1519..1537; N=1000 -> about 3019..3037. Versus the proposed pass 2.4W + 1.
- Falsification: observed E differs from 3W + 1 by more than 1 (the reproduced target set is wrong, or the router submits something else).

## B2. Wall / CPU / duty
- Model (from the prior run): pass wall ~= sum of UNCONTENDED per-eval tick cost (the allocator mutex serialises hydration; the prior
  N=1000 pass was 4.55 s against an uncontended sum of 4.44 s), times a fixed per-eval overhead that is large at small N.
  Unknown input: service-lifecycle tick cost on a Job target; assume it equals its cost on a Service target (reads the same tables).
- Prediction (wall): N=10: 11..14 ms; N=100: 90..140 ms; N=500: 1.4..1.7 s; N=1000: 5.0..6.3 s (central 5.7 s, about 20% above the
  proposed pass because the baseline runs service-lifecycle on 5x as many targets and skips service-map-hydrator, which is ~free).
  CPU 105..125% of wall. Duty cycle at N=1000: 17..21% (wall). So G1 (3% at 1000 targets) fails for the baseline as well.
- Falsification: baseline N=1000 wall within +-10% of the same-run proposed pass (the +20% model is wrong, service-lifecycle on a Job is
  much cheaper than on a Service), OR baseline duty at N=1000 at or under 3%.

## B3. Edge-wake admission delay and allocator lock
- Hypothesis: the same strict-FIFO broker. A deploy submitted 3 ms after the burst's first submission waits for the whole burst.
- Prediction: admission delay within 10% of the baseline burst wall, minus 3 ms: N=10 about 10 ms, N=100 about 100 ms, N=500 about 1.5 s,
  N=1000 about 5.2 s (4.5..6.5 s). Service handler latency during the burst equals the proposed pass's within +-25%
  (mean hold per lock acquisition is the same, ~1.85 ms at N=1000 in both): N=1000 10..20 ms, N=500 6..11 ms, N=100 2..4 ms, N=10 1..3 ms.
  Lock wait p50 at N=1000 about 10..15 ms.
- Falsification: baseline edge admission at N=1000 is under 80% of the baseline burst wall, OR baseline Service handler latency at N=1000 is
  outside 0.6x..1.6x of the proposed pass's.

## Method differences (stated up front)
- The probe drives the REAL `spawn_interest_router` (pub): its boot LIST IS `list_and_route`; `relist_period` is set to 1 h so exactly one
  list runs per sample. `route_observation_row` and `list_and_route` are private but are exercised through that entry point, so the target
  set is not reproduced, it is the production code's. An independent reproduction of the expected keys (from the alloc rows) is checked against it.
- The existing `resync_schedule` submissions use the real `build_cadence_table` / `arm_next_wake` / `due_resync_evaluations`.
- The router's boot LIST reads `alloc_status_rows()` inside the timed window (the proposed run's enumeration was outside it, 4 ms at N=1000).
- The edge deploy is submitted 3 ms after the FIRST router submission is observed in the broker (not after the spawn), so a slow list read
  cannot let the edge win the FIFO race. The spawn-to-first-submit gap is recorded.
- The loop still mirrors `spawn_convergence_loop` (not callable), as before. A same-run proposed control is interleaved sample by sample.
