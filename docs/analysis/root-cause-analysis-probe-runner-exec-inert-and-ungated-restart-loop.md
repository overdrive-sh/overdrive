# RCA — probe-runner: the "inert exec probe" and the ungated liveness restart loop

**Date:** 2026-08-02
**Investigator:** Rex (root-cause analysis)
**Measured against:** HEAD `86d6331b` + the uncommitted ADR-0080 Stage 1 working tree
**Kernel:** `7.0.0-28-generic` (Lima dev VM), disk 89 G free
**Status:** investigation only — nothing was fixed, committed, or filed
**Revision:** rev 2, after adversarial review. Rev 1's additive-latency cadence model was wrong and is corrected in § 1.1; a third defect (§ 1.4) was found while resolving a datum rev 1 reported but did not explain.

---

## Verdicts

| Anomaly | Verdict |
|---|---|
| **1 — restart rate contradicts the declared gating** | **REPRODUCES.** Three independent, compounding defects. The restart *is* streak-gated, but the streak counts **reconciler evaluations**, not probe observations (A); the `RESTART_BACKOFF_CEILING = 5` valve cannot fire on this path (B); and the probe supervisor is never torn down or re-armed across a restart (C). |
| **2 — exec probes appear completely inert** | **DOES NOT REPRODUCE.** Exec probes are functional end-to-end at this tree state. `exit 0` → 0 restarts, `exit 1` → 106 restarts, from an identical 19 executions each. The original report is best explained by zero success-path observability plus, probably, a pre-Stage-1 run. |

Both hypotheses the audit named are **falsified as stated**. Hypothesis (a) (a streak of *probe executions* gates the restart) fails because the probe interval does not set the cadence. Hypothesis (b) (presence of a liveness descriptor drives restart, ungated by any streak) is falsified three independent ways. The truth is a third shape, (a′).

---

## 1. Anomaly 1 — mechanism

### 1.0 Fixtures (reproducibility)

The fixtures were generated inside the VM's `/tmp` and deleted afterwards, so the repository was never polluted. They are reproduced verbatim here so the experiment is repeatable without them. Every fixture shares this template, with only the marked fields varying:

```toml
[service]
id = "<ID>"
replicas = 1

[[listener]]
port = <PORT>
protocol = "tcp"

[exec]
command = "/usr/bin/socat"
args    = ["TCP-LISTEN:<PORT>,fork,reuseaddr", "PIPE"]

[[health_check.startup]]
type = "tcp"
port = <PORT>
interval_seconds = 1
timeout_seconds = 1
max_attempts = <MAX_ATTEMPTS>

<LIVENESS BLOCK>

[resources]
cpu_milli = 100
memory_bytes = 67108864
```

| id | PORT | MAX_ATTEMPTS | LIVENESS BLOCK |
|---|---|---|---|
| `rca-t2i2` | 8191 | 30 | `type="tcp"`, `port=9291`, `interval_seconds=2`, `timeout_seconds=1`, `failure_threshold=2` |
| `rca-t10i2` | 8192 | 30 | as above but `failure_threshold=10` |
| `rca-t2i20` | 8193 | 30 | as `rca-t2i2` but `interval_seconds=20` |
| `rca-execf` | 8194 | 30 | `type="exec"`, `command=["/bin/sh","-c","echo tick >> /tmp/rca-exec-liveness.log; exit 1"]`, `interval_seconds=2`, `timeout_seconds=1`, `failure_threshold=2` |
| `rca-execpass` | 8291 | 300 | as `rca-execf` but `exit 0`, own log file |
| `rca-execfail` | 8292 | 300 | as `rca-execf` but `exit 1`, own log file |
| `rca-nolive` | 8293 | 300 | *(none)* |

Port 9291 is bound by nothing, so the TCP liveness target is unreachable by construction. Harness: an ephemeral `overdrive serve` (KEK via the production `$CREDENTIALS_DIRECTORY/overdrive-ca-root` systemd-creds path), `overdrive deploy <spec> --detach`, counts from `overdrive workload describe`, cadence from `serve.log`. `max_attempts=300` in the E3 set prevents the startup budget exhausting inside the window and confounding the diff.

### 1.1 The discriminating experiment

**Hypothesis (H-A′):** the restart *is* gated by the consecutive-failure streak, but the streak advances once per **reconciler evaluation** on which the latest liveness row reads `Fail` — not once per probe execution; and `RESTART_BACKOFF_CEILING` never fires because the liveness branch reads a counter its own restarts cannot increment.

**Predicted:** `t2i2` ≈ 0.4–0.6 s/restart; `t10i2` ≈ ⅓ the rate of `t2i2` (*threshold scales*); `t2i20` ≈ same cadence as `t2i2` (*interval does not scale the cadence*); all counts ≫ 5.

**Falsified if:** cadence scales with `interval_seconds` (⇒ hypothesis (a)), **or** `t10i2` ≈ `t2i2` (⇒ hypothesis (b)), **or** any fixture halts at 5 restarts (⇒ ceiling live).

Measured (window 20.109 s between the T+10 s and T+30 s snapshots):

```
workload            n  first_s   last_s   span_s  median_gap  max_rc
rca-execf          51   101.89   129.16    27.27       0.460      51
rca-t10i2          10   103.94   129.52    25.58       2.388      10
rca-t2i2           54   102.36   129.28    26.93       0.460      54
rca-t2i20          15   120.71   129.63     8.92       0.570      15
```

| fixture | varied input | median gap | max `restart_count` |
|---|---|---|---|
| `rca-t2i2` | thr 2, int 2 (reference) | **0.460 s** | 54 |
| `rca-t10i2` | thr **10**, int 2 | **2.388 s** | 10 |
| `rca-t2i20` | thr 2, int **20** | **0.570 s** | 15 |
| `rca-execf` | exec mechanic, thr 2, int 2 | **0.460 s** | 51 |

**Scoring against the prediction:**

- **Threshold ×5 → cadence ×5.19** (0.460 → 2.388 s). The streak **is** consulted. **Hypothesis (b) falsified.**
- **Interval ×10 → cadence ×1.24** (0.460 → 0.570 s), against a prediction of ≥ `threshold × interval` = 40 s per restart under hypothesis (a). **Hypothesis (a) falsified.**
- Every `max_rc` ≫ `RESTART_BACKOFF_CEILING = 5`. **The valve never fired.**

**The cadence model (corrected in rev 2).** Rev 1 fitted `cadence = threshold × T + L` and obtained a *negative* intercept (`L = −0.022 s`), which it reported as "≈ 0". A negative fixed-overhead term is not noise — it falsifies the additive model. The correct model is single-parameter:

```
cadence ≈ threshold × T,   T ≈ 0.239 s        (t10i2: 2.388 / 10 = 0.2388)
                                              (t2i2:  0.460 /  2 = 0.2300)
```

The two independent estimates of `T` agree within 4 %, with **no additive latency term**. The reason there is none is a mechanism rev 1 missed: in `liveness_restart_action` the streak increment (`crates/overdrive-core/src/service_lifecycle.rs:751-765`) runs **before** the `triggered` gate that requires `state == Running` (`:769-773`). So the streak keeps advancing during the restart's own teardown/startup window, while the alloc is *not* `Running`. Restart round-trip latency is therefore **absorbed into** the streak accumulation rather than added to it — which is exactly why a fitted additive intercept comes out at or below zero. A strictly more accurate form is `cadence ≈ max(threshold × T, restart_round_trip)`; the two terms are near-equal at threshold 2 and the threshold term dominates at threshold 10. Distinguishing them needs a third threshold point, which was not run.

Raw restart lines, showing the cadence directly against a 2 s probe interval:

```
2026-08-02T10:01:41.885933Z  INFO allocation recovered from a terminal observation alloc=alloc-rca-execf-0 workload=rca-execf restart_count=1 prior_state=terminated
2026-08-02T10:01:42.217002Z  INFO allocation recovered from a terminal observation alloc=alloc-rca-execf-0 workload=rca-execf restart_count=2 prior_state=terminated
2026-08-02T10:01:42.357017Z  INFO allocation recovered from a terminal observation alloc=alloc-rca-t2i2-0 workload=rca-t2i2 restart_count=1 prior_state=terminated
2026-08-02T10:01:42.702021Z  INFO allocation recovered from a terminal observation alloc=alloc-rca-execf-0 workload=rca-execf restart_count=3 prior_state=terminated
2026-08-02T10:01:42.831043Z  INFO allocation recovered from a terminal observation alloc=alloc-rca-t2i2-0 workload=rca-t2i2 restart_count=2 prior_state=terminated
2026-08-02T10:01:43.150942Z  INFO allocation recovered from a terminal observation alloc=alloc-rca-execf-0 workload=rca-execf restart_count=4 prior_state=terminated
```

`restart_count` 1→4 in 1.27 s while the probe that supposedly drives it ticks every 2 s. The reference fixture's 0.460 s reproduces the reported 0.462 s to within 0.5 %.

**On the interval fixture's onset.** `rca-t2i20`'s first restart is 18.35 s later in absolute log time than `rca-t2i2`'s. Rev 1 leaned on that number; it is **weak evidence**, because the four fixtures were deployed sequentially and no per-fixture `t0` was recorded, so the comparison lacks a common origin. The claim it was used to support — that the interval delays only the *onset*, by deferring the first `Fail` row — has direct source support that rev 1 failed to cite: `supervised_probe_loop` is **tick-then-sleep**, parking on `clock.sleep(interval)` *before* each attempt, so the first row lands only after one full interval elapses (`crates/overdrive-worker/src/probe_runner/mod.rs:623-628`, contract stated at `:596-599`). That is the load-bearing evidence; the 18.35 s figure is corroborating only.

**One residual is not explained.** `rca-t2i20`'s median gap is 0.570 s against `rca-t2i2`'s 0.460 s at the *same* threshold — a 24 % excess where root cause A alone predicts equality. A plausible mechanism is that probe-result writes themselves enqueue reconciler evaluations, so a 20 s interval yields marginally fewer evaluations and a marginally slower streak; that would mean the interval *weakly* gates the cadence as well as the onset. This was **not excluded**, and only medians were captured, so neither the 1.24× nor the 5.19× ratio carries a dispersion estimate.

### 1.2 Root cause A — a convergent level counted as an event stream

`liveness_restart_action` (`crates/overdrive-core/src/service_lifecycle.rs:751-765`) increments the streak whenever `fact.latest_liveness_probe` reads `Fail`:

```rust
let consecutive_failures = match &fact.latest_liveness_probe {
    Some(ProbeStatus::Fail { .. }) => {
        let entry = next_view.liveness_consecutive_failures.entry(key.clone()).or_insert(0);
        *entry = entry.saturating_add(1);
        *entry
    }
```

`latest_liveness_probe` is a **level**, not an edge. It is produced by `latest_probe_status` (`crates/overdrive-control-plane/src/reconciler_runtime.rs:2988-2997`):

```rust
rows.iter()
    .filter(|p| p.role == role && p.probe_idx == probe_idx)
    .max_by_key(|p| p.last_observed_at_unix_ms)
    .map(|p| p.status.clone())
```

The projection selects the LWW-latest row and then **discards `last_observed_at_unix_ms`** at `:2996`. The resulting `ServiceAllocFact` (`reconciler_runtime.rs:3134-3155`) carries the probe's *status* with no observation timestamp, so the reconciler has no input with which to distinguish "the same `Fail` I already counted" from "a new `Fail`". Every evaluation re-counts the current level.

Note this is **not** a frozen-stale-row story. § 1.4 shows the probe keeps ticking on its declared interval, so the row *is* rewritten every 2 s with a fresh timestamp — the reconciler simply reads it about eight times per write. That distinction matters for the remedy: because the timestamp does advance, a watermark discriminator (R-A) is sufficient.

This is the failure `.claude/rules/development.md` § "A convergent record cannot answer 'did it happen'" names: a convergent LWW record reports the latest *fact*, never *occurrence*. The threshold semantics the operator writes (`failure_threshold = N` = "N consecutive failed **probes**") are occurrence semantics implemented against a convergence surface.

**Consequence:** `failure_threshold` multiplies the reconciler evaluation period rather than counting probes, and `interval_seconds` does not set the restart cadence — it gates only when the first row appears (plus, possibly, the unexplained 24 % residual in § 1.1).

**The same defect exists on the other two roles**, so this is systemic, not liveness-specific:

- `update_startup_attempts` (`service_lifecycle.rs:939-954`) — `Some(Fail)` → `saturating_add(1)` per evaluation. `max_attempts` counts evaluations, not startup attempts. (The `startup_deadline` co-gate is what actually bounds the startup branch today, masking this.)
- `compute_backend_healthy` (`service_lifecycle.rs:893-921`, called from `readiness_backend_row_action` at `:832`) — `Some(Pass)` → `saturating_add(1)` per evaluation at `:904-910`. `success_threshold` is satisfied after N evaluations rather than N distinct probe passes, defeating the flap-damping and the S-SHCP-RECON-08c inverse-race guard.

### 1.3 Root cause B — the restart budget is keyed to a counter liveness cannot increment

The loop is *unbounded* for a second, independent reason. The liveness branch's budget check (`service_lifecycle.rs:778`):

```rust
if fact.restart_count >= RESTART_BACKOFF_CEILING {
    return Some(Action::FinalizeFailed { ... });
}
```

`RESTART_BACKOFF_CEILING = 5` (`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:23`). Observed `restart_count` reached **106**. `fact.restart_count` is hydrated at `reconciler_runtime.rs:3126-3128`:

```rust
let restart_count = restart_target.as_ref().map_or(0, |t| {
    state.runtime.restart_status_for_alloc(t, &row.alloc_id).0.saturating_sub(1)
});
```

`restart_status_for_alloc` (`reconciler_runtime.rs:456-466`) reads `WorkloadLifecycleView.restart_counts` — a map incremented only when **`WorkloadLifecycle`** emits a restart (`workload_lifecycle.rs:787-789`). Liveness restarts are emitted by **`ServiceLifecycle`**, which never writes it.

Crucially, `WorkloadLifecycle` *does* process Service-kind allocations, so that map is not inertly empty by construction — it is emptied by a four-step chain the liveness restart creates for itself:

1. The restart's stop-half calls `driver.stop()` (`action_shim/mod.rs:1472`), which sets `intentional_stop = true` before killing (`crates/overdrive-worker/src/driver.rs:611`).
2. The exit observer maps **any** intentional stop to `TransitionReason::Stopped { by: Operator }` (`crates/overdrive-control-plane/src/worker/exit_observer.rs:621-628`; the governing docstring at `:617-620` reads *"The `intentional_stop` flag wins: any operator stop classifies as `Terminated::{by: Operator}` regardless of the underlying kernel exit shape."*).
3. `is_intentionally_stopped` therefore returns `true` (`workload_lifecycle.rs:1096-1111`), so `is_restartable` returns `false` (`:1116-1120`).
4. `WorkloadLifecycle` skips the row → `restart_counts` is never incremented → the liveness budget reads `(0 + 1) − 1 = 0`.

**Scope of the claim.** `fact.restart_count` is 0, and the ceiling unreachable, **for allocations whose every terminal is intentional-stop-classified** — which is precisely the liveness-restart loop, since the loop's own stop-half produces that classification on every cycle. A Service allocation that genuinely *crashes* still increments `restart_counts` normally and can reach the ceiling. The ceiling is dead on this path, not globally.

**This also resolves the `stopped (by operator)` clue.** The reason string on every restarted alloc (`evidence/describe_fails.out:7`) comes from step 2, rendered via `crates/overdrive-core/src/transition_reason.rs:749`. Nothing attributes the restart to liveness because the *stop* half is attributed by the driver's `intentional_stop` flag, which cannot distinguish an operator `overdrive job stop` from a reconciler-driven restart. (Note a divergence: the `StopAllocation` arm writes `Stopped { by: Reconciler }` on the row at `action_shim/mod.rs:1730-1731`, a variant `is_intentionally_stopped` does *not* match. The observed operator render confirms the exit-observer path is the one that wins here, but the two writers do disagree.)

Meanwhile the `Restarts 106` the operator reads is a **third** counter — `AllocStatusRow.restart_count`, counting observed `terminal → Running` transitions. `crates/overdrive-core/src/traits/observation_store.rs:1218-1224` states the split explicitly:

> **An observed input, not derived state.** … It is NOT the same quantity as `WorkloadLifecycleView.restart_counts`, which counts restart *attempts* at emit time and drives the backoff budget … Do not source one from the other (ADR-0078 § D3).

So the operator sees 106, the budget sees 0, and both behave as specified.

### 1.4 Root cause C — the probe supervisor is never torn down or re-armed across a restart

Rev 1 reported, but did not explain, that `rca-execpass` and `rca-execfail` each executed **exactly 19 times** while `execfail` was destroyed and recreated 106 times in the same window. Under tick-then-sleep with a 2 s interval, a supervisor cancelled and respawned every ~0.46 s should almost never reach its first probe. 19 ≈ 40 s / 2 s is instead exactly the count for a loop that *never restarted at all*.

The source confirms it. The `RestartAllocation` arm (`action_shim/mod.rs:1596-1665`) calls `driver.on_alloc_running(&spec)` at `:1662` and **never calls `on_alloc_terminal`** — only the `FinalizeFailed` arm (`:1211`) and the `StopAllocation` arm (`:1757`) do, and those are the hooks wired to `ProbeRunner::stop_alloc` (`lib.rs:324-325`, `:1383-1384`). And `ProbeRunner::start_alloc` early-returns without respawning when the supervisor already exists (`crates/overdrive-worker/src/probe_runner/mod.rs:319-322`):

```rust
if supervisor.is_started() {
    return root_token;
}
supervisor.mark_started();
```

So across all 106 restarts the per-descriptor probe tasks are never cancelled and never re-created; they tick on their own wall-clock cadence, fully decoupled from the allocation they are probing.

**Consequences, both of which feed the loop:**

- **No grace window after a restart.** The tick-then-sleep interval that should give a restarting workload one full interval to come up is never re-armed. A workload restarted at *t* is probed at whatever point the free-running loop next fires — possibly immediately — so it is judged before it can plausibly be healthy.
- **Probe state does not reset across the restart boundary.** The liveness observation carried into the post-restart allocation is the pre-restart one. Combined with root cause A, the surviving `Fail` level is re-counted straight through the restart, which is why the comment at `service_lifecycle.rs:790-795` — which resets the streak specifically to avoid "one restart per tick" — does not achieve its stated purpose.

This also means the exec prober is repeatedly targeting `exec_scope_path(alloc_id)` for a cgroup scope that is being destroyed and recreated underneath it (`probe_runner/mod.rs:535-543`). It happened not to fail here (`probe_tick.error` count 0 across the first run), but it is a race the design does not acknowledge.

---

## 2. Anomaly 2 — exec probes are not inert

### 2.1 The run-counter probe

**Hypothesis:** the exec prober never executes its command on the production path.
**Predicted:** the side-effect log is absent or empty, and the exec fixture shows 0 restarts.
**Falsified if:** the file has ≥ 1 line.

The probe command was made self-reporting — `["/bin/sh", "-c", "echo tick >> /tmp/rca-exec-liveness.log; exit 1"]` — so the line count *is* the run counter (§ 7, "did the program run at all?"). This needs no source instrumentation: the exec child is spawned by the control-plane process and joined only to the workload **cgroup**, never a mount or network namespace (`probe_runner/mod.rs:535-543`), so it writes to the ordinary VM filesystem.

```
=== exec-probe run counter: /tmp/rca-exec-liveness.log ===
file EXISTS; line count = 13
```

**Hypothesis falsified.** The exec probe executed 13 times and drove 51 restarts.

### 2.2 The pass/fail control — exec is not merely running, it is scored correctly

**Hypothesis:** exec probes are functional end-to-end and a `Pass` correctly clears the streak.
**Predicted:** `execpass` → 0 restarts; `execfail` → ≫ 5; `nolive` → 0.
**Falsified if:** `execpass` restarts at all.

```
=== exec-probe run counters (did each command actually execute?) ===
execpass(exit 0) : EXISTS, 19 executions
execfail(exit 1) : EXISTS, 19 executions

=== E3 describe results at T+40s ===
FIXTURE        STATE      RESTARTS
rca-execpass   Running    0
rca-execfail   Running    106
rca-nolive     Running    0
```

```
Service 'rca-execpass' (kind: Service)
Replicas (desired/running): 1/1
Alloc                    State        Restarts   Since
alloc-rca-execpass-0     Running      0          (c=3,w=local)
    reason: driver started
```

`execpass` and `execfail` are identical in every input **including an identical TCP startup probe** and differ only in the liveness probe's exit code; from the same 19 executions they produce 0 vs 106 restarts. The exec liveness observation therefore reaches the restart decision *and* is scored correctly: `Pass` clears the streak, `Fail` drives restart.

`rca-nolive` is a *separate, two-variable* control (it differs from `execpass` by the liveness descriptor's **presence**, not by an exit code). Its role is the (b)-falsifier pairing: a declared-but-passing liveness probe (`execpass`, 0 restarts) is indistinguishable from no liveness probe at all (`nolive`, 0 restarts), so descriptor presence alone drives nothing. That is the **third independent falsification of hypothesis (b)**.

This three-way diff also excludes the obvious alternative drivers of the loop — the startup branch, `WorkloadLifecycle` crash-restart, and the `socat` workload self-exiting would each fire identically across all three fixtures, and do not.

It is additionally the negative control the O07 capture could not obtain for TCP. Because an exec probe crosses no network namespace, `exit 0` is an unambiguous, reachable `Pass` — so **the exec mechanic can express "my workload is healthy" on the current composition, and the TCP/HTTP mechanics cannot.** That is load-bearing for the remedy.

### 2.3 Why the original observation looked inert

Two compounding causes, in the § 3 "inspection-tool gaps look like negative evidence" shape:

- **The "no log output" signal was uninformative in both directions.** `probe_tick` emits **nothing** on the success path; the only event in the subsystem is `probe_tick.error`, logged on adapter/store failure (`probe_runner/mod.rs:639-646`). Across the first run, `probe_tick.error` count was **0** while 13 exec probes demonstrably executed. There is also no operator surface: `workload describe` had no `Probes` section at capture time (O07 README § "Not the Probes render", `README.md:130`). Absence of logs was therefore fully consistent with a perfectly healthy exec probe and could never have distinguished "did not run" from "ran fine".
- **Most probably the fixture predated ADR-0080 Stage 1** — *inference, not measurement.* Before Stage 1, `ProbeRunner::start_alloc` derived `probe_idx` from a flat `enumerate()` over `startup ++ readiness ++ liveness`, while the liveness hydrate filters per-role index 0 (`reconciler_runtime.rs:3069-3073`). With a startup probe declared — as every one of these fixtures declares — the liveness descriptor landed at flat index 1 and, *on that tree*, no restart decision would have seen it for **any** mechanic. Both that conclusion and the "for any mechanic" sub-claim are inference about a tree state I did not run; confirming either requires checking out the pre-Stage-1 tree, which is out of scope here.

**The malformed-fixture alternative is not fully excluded.** A *parse-invalid* fixture is weakly disfavoured — `overdrive deploy` exits non-zero with a typed `ParseError` — but a **parse-valid but semantically wrong** fixture (the liveness block under the wrong table, or a command that in fact exits 0) produces exactly the reported "no restarts, no output" and is indistinguishable from the pre-Stage-1 explanation on the evidence available. Since the original fixture was not preserved, this cannot be settled.

---

## 3. Five Whys

```
PROBLEM: A Service declaring a failing liveness probe restarts every ~0.46 s —
         far faster than interval_seconds x failure_threshold permits — is
         attributed "stopped (by operator)", and never stops; while exec probes
         were believed to be entirely inert.

WHY 1A: Restart cadence 0.460 s vs a 4 s floor.
        [Evidence: median gap over 54 restarts, rca-t2i2.]
  WHY 2A: The streak advances per reconciler evaluation (T ~ 0.239 s), not per probe.
        [Evidence: threshold x5 -> cadence x5.19; interval x10 -> cadence x1.24.]
    WHY 3A: liveness_restart_action increments on a LEVEL read, every evaluation,
            and does so BEFORE the state==Running gate.
        [Evidence: service_lifecycle.rs:751-765 vs the gate at :769-773.]
      WHY 4A: The projection discards the observation timestamp, so no edge can be
              detected. [Evidence: reconciler_runtime.rs:2996; ServiceAllocFact :3134-3155.]
        WHY 5A: Occurrence semantics ("N consecutive failed probes") implemented
              against a convergent LWW surface that reports only the latest fact.
              [Evidence: development.md § "A convergent record cannot answer
              'did it happen'".]
        -> ROOT CAUSE A: consecutive-probe thresholds count reconciler evaluations
           over a convergent level with no watermark. failure_threshold multiplies
           the tick period; interval_seconds does not set the cadence.

WHY 1B: The loop is unbounded — 106 restarts against a ceiling of 5.
        [Evidence: rca-execfail; workload_lifecycle.rs:23.]
  WHY 2B: fact.restart_count stays 0 on this path. [Evidence: service_lifecycle.rs:778.]
    WHY 3B: It is hydrated from WorkloadLifecycleView.restart_counts, which only
            WorkloadLifecycle writes. [Evidence: reconciler_runtime.rs:456-466, 3126-3128.]
      WHY 4B: The restart's own stop-half marks the exit intentional -> Stopped{Operator}
            -> is_restartable() == false -> WorkloadLifecycle skips the alloc, so the
            counter it owns never advances.
            [Evidence: action_shim:1472; driver.rs:611; exit_observer.rs:621-628;
             workload_lifecycle.rs:1096-1120 vs the increment at :787-789.]
        WHY 5B: Two reconcilers share one budget through a counter only one writes,
              and the writer is disqualified by a side effect of the other's action;
              the operator-visible count is a third counter entirely.
              [Evidence: observation_store.rs:1218-1224.]
        -> ROOT CAUSE B: the liveness restart budget is keyed to a counter liveness
           restarts cannot increment. RESTART_BACKOFF_CEILING is unreachable on this path.

WHY 1C: 19 probe executions spanned 106 restarts of the same alloc.
        [Evidence: run counters vs describe, E3.]
  WHY 2C: The probe supervisor is never cancelled or respawned by a restart.
    WHY 3C: The RestartAllocation arm never calls on_alloc_terminal, and start_alloc
            early-returns when already started.
        [Evidence: action_shim/mod.rs:1596-1665 vs :1211/:1757; probe_runner/mod.rs:319-322.]
      WHY 4C: Probe supervision is bound to the alloc's terminal hooks, but a restart
            is not modelled as a terminal for probe purposes.
        WHY 5C: The probe lifecycle and the allocation lifecycle are coupled only at
              start and terminal, with no re-arm on the restart edge.
        -> ROOT CAUSE C: probe tasks outlive the allocation generation they probe:
           no grace window is re-armed and no probe state resets across a restart.

WHY 1D: Exec probes reported to produce no restart and no log output.
  WHY 2D: They are not inert — 19 executions/40 s; exit 1 -> 106 restarts, exit 0 -> 0.
    WHY 3D: "No log output" could not have shown execution: the subsystem emits
          nothing on the success path. [Evidence: probe_runner/mod.rs:639-646;
          probe_tick.error count 0 while 13 probes ran.]
      WHY 4D: "No restart" is most consistent with a pre-Stage-1 run, where the flat
            enumerate() index made every liveness probe inert. [INFERENCE — tree not run;
            a parse-valid-but-wrong fixture produces the same symptom and is not excluded.]
        WHY 5D: A mechanic-independent defect was attributed to the exec mechanic
              because the only available observable was blind on the happy path.
        -> ROOT CAUSE D: zero success-path observability in the probe subsystem;
           absence of evidence read as evidence of absence, finding discarded unrecorded.
```

### Backwards chain validation

- **A + B + C → forward.** A `Fail` level plus an evaluation-driven streak yields one restart per `threshold × 0.239 s`; the inert budget makes it unbounded; the un-re-armed supervisor means the level is never reset by the restart. This predicts cadence **linear in threshold**, **not set by interval**, **unbounded counts**, and **probe executions decoupled from restart count**. All four observed. ✓
- **A without B** would produce a bounded 5-restart burst then `FinalizeFailed`. Not observed ⇒ B required. ✓
- **B without A** would produce unbounded restarts at *probe* cadence (≥ 4 s apart for the reference fixture). Not observed ⇒ A required. ✓
- **C alone** produces no restart at all (it is an amplifier and a grace-window defect, not a trigger); it explains the 19-vs-106 decoupling that A and B do not. ✓
- **D is independent and non-contradictory.** Exec probes traverse the identical hydrate → decision path, which is why `execfail` reproduces A + B + C byte-for-byte against the TCP reference (0.460 s median gap for both). ✓

**All reported symptoms explained:** the 0.462 s cadence (A, threshold 2); `stopped (by operator)` (B, step 2); `restart_count 83 ≫ 5` (B); `liveness-holds` restarting identically to `liveness-fails` (the established netns gap makes both `Fail`, then A + B + C); exec "inertness" (D).

---

## 4. Blast radius

**Severity: P0.** The defects compound with the already-established namespace-reachability gap.

- **Every Service declaring a TCP or HTTP liveness probe on the production mesh path enters an unbounded restart loop.** The netns gap guarantees the probe fails regardless of workload health; root cause A converts that into a restart roughly every `threshold × 0.24 s`; root cause B removes the only stop condition; root cause C removes the grace window that might otherwise let the workload recover. The workload never stabilises and never reaches a `Failed` terminal an operator could triage.
- **The crash-loop guard is absent on this path, not merely mistuned.** `RESTART_BACKOFF_CEILING = 5` reads as a bounded-blast-radius guarantee in review; a liveness-driven loop never reaches it.
- **`failure_threshold` and `interval_seconds` do not mean what the operator wrote.** `interval_seconds` does not set the restart cadence; `failure_threshold` is a multiplier on the reconciler evaluation period. An operator tuning these to damp flapping gets no damping.
- **Readiness is affected by the same root cause A**, less visibly: `success_threshold` is satisfied after N evaluations (~0.24 s each) rather than N probe passes, so a backend is marked `healthy` within a fraction of a second on a single `Pass` row.
- **Startup is affected too** (`max_attempts` counts evaluations), currently masked by the co-gating `startup_deadline`. It is a latent trap for anyone who later relaxes or removes that deadline.
- **Restart attribution is misleading platform-wide.** Because the driver's `intentional_stop` flag cannot distinguish an operator stop from a reconciler-driven restart, *every* reconciler-driven restart is recorded as `Stopped { by: Operator }`. Any audit, SLO, or forensic query filtering on operator stops is contaminated.
- **Per-restart resource churn at ~2 Hz — inferred, not measured.** Each restart is expected to tear down and recreate a cgroup scope and network namespace and to mint a fresh workload SVID, implying on the order of 100 certificate issuances per 40 s and corresponding `issued_certificates` growth. I did **not** capture the `issued_certificates` count or a cgroup/netns churn trace, so treat the magnitude as an expectation derived from the restart path, not an observation.

**Not affected:** Job- and Schedule-kind workloads (`project_probe_descriptors` returns an empty vector for both, `workload_lifecycle.rs:1256-1258`), and any Service that declares no liveness probe (`rca-nolive`: 0 restarts).

---

## 5. Recommended remedies

Investigation only — none of this was implemented, and the design decisions are not mine to take.

### R-A (root cause A) — make the streak edge-triggered

Carry `last_observed_at_unix_ms` from `ProbeResultRow` into `ServiceAllocFact` (stop discarding it at `reconciler_runtime.rs:2996`), persist a per-`(alloc, role, probe_idx)` **watermark** in `ServiceLifecycleView`, and advance the streak only when the observed timestamp is **strictly greater** than the watermark.

- *For:* fixes all three roles with one change; the watermark is an observed *input*, not derived state, so it satisfies § "Persist inputs, not derived state"; makes `failure_threshold` and `interval_seconds` mean what the operator wrote. It works precisely because the timestamp *does* advance every interval (§ 1.4).
- *Against / risks:* a `View` schema addition (additive `#[serde(default)]`). Under `SimClock` a non-advancing clock would stall the streak, so DST fixtures must tick time. Worth a Tier-1 invariant asserting *streak ≤ number of distinct observations*.
- *Rejected alternative:* maintaining the streak in the probe runner (which sees edges natively) and publishing a count on the row. That relocates **policy** into the worker, contradicting the reconciler-decides split, and persists derived state on an observation row.

### R-B (root cause B) — fix the stop attribution first, then the budget

**Evaluate this first, as it is the highest-leverage single change.** `exit_observer::classify` collapses every intentional stop to `StoppedBy::Operator`. A distinct `StoppedBy::Reconciler` — the variant already exists and is used by the `StopAllocation` arm at `action_shim/mod.rs:1730-1731` — threaded through the restart stop-half would fix the misleading operator render *and* stop `is_intentionally_stopped` disqualifying restarted allocs, which would let `WorkloadLifecycleView.restart_counts` advance and make the existing ceiling live again. That may make the budget work below unnecessary.

If a separate budget is still wanted, two shapes, and the choice is a real decision:

1. **A `ServiceLifecycle`-owned counter** incremented when *it* emits a liveness restart. Honest and local. *Against:* two independent budgets mean a workload that both crash-loops and fails liveness gets ~2× the intended blast radius unless explicitly composed.
2. **Source the budget from `AllocStatusRow.restart_count`** — the observed transition counter, which does increment and is already the operator-visible number, so budget and CLI would finally agree. *Against:* ADR-0078 § D3 explicitly forbids sourcing one from the other, because attempts and observed transitions diverge on driver-rejected restarts. This needs an ADR amendment, not a quiet edit.

Either way the `FinalizeFailed { LivenessProbeFailed }` path is currently unreachable and untested end-to-end; it needs a test that actually drives it.

### R-C (root cause C) — re-arm probe supervision on the restart edge

Decide explicitly whether a restart is a probe-lifecycle boundary. If it is (recommended — it is a new process generation), the `RestartAllocation` arm should tear the supervisor down and re-create it, so the tick-then-sleep interval is re-armed as a grace window and probe state does not leak across generations. That likely means calling `ProbeRunner::stop_alloc` on the restart's stop-half, or making `start_alloc` re-arm rather than early-return when the allocation generation has changed.

- *Trade-off:* re-arming costs one full interval of blindness after every restart, which is the intended semantics but does delay genuine failure detection. It also removes the cgroup-scope race noted in § 1.4.
- *Note:* R-C alone materially reduces the loop's severity even without R-A, because the grace window breaks the immediate re-trigger.

### R-D (root cause D) — close the observability gap

Add a per-tick success-path event to `probe_tick` (role, idx, status, mechanic, at `debug`), and land the `Probes` section on `workload describe`. Add an investigator-facing surface too — this investigation had no way to dump probe rows or inspect a reconciler `View`, and had to infer both from behaviour. The untracked `crates/overdrive-cli/tests/integration/workload_describe_probes.rs` in the working tree suggests the CLI half may already be in flight; the runner-side event and the row/View dump are still missing.

**Priority:** R-D is cheap and unblocks diagnosis of everything else. R-A and R-B are both required — either alone leaves a live defect (§ 3 backwards-chain validation). R-C is required for correct semantics and is a strong mitigation on its own.

---

## 6. Limits of this investigation

Stated so nothing here is over-read:

- **`T ≈ 0.239 s` is derived from two data points**, not measured directly. `DEFAULT_TICK_CADENCE` is 100 ms (`reconciler_runtime.rs:1274`), so this is ≈ 2.4 ticks per increment; the extra factor is **unexplained** (plausibly evaluation enqueue/drain or hydrate cost) and was not instrumented. The additive model `threshold × T + L` is falsified (negative intercept); `max(threshold × T, restart_round_trip)` also fits and was **not excluded** — a third threshold point would discriminate.
- **The 24 % cadence residual on the interval fixture (0.460 → 0.570 s at equal threshold) is unexplained**, and is consistent with the interval weakly gating cadence via write-driven evaluation enqueue. Only medians were captured; no dispersion, so no significance can be attached to either the 1.24× or the 5.19× ratio.
- **The +18.35 s onset comparison lacks a common `t0`.** Fixtures were deployed sequentially and per-fixture deploy timestamps were not recorded. The onset claim rests on the tick-then-sleep source contract, not on this number.
- **The fixtures were deleted** (per the investigation's no-pollution constraint) and are reproduced in § 1.0 rather than preserved as files. The run is repeatable from § 1.0 but was not third-party reproduced.
- **No direct observation of the emitting action.** The cadence lines are `alloc.restart.observed` (`action_shim/mod.rs:333-340`), which fires on any `terminal → Running` transition and carries **no reconciler attribution**. No `RestartAllocation { LivenessExhausted }` dispatch, no `liveness_consecutive_failures` View dump was captured. Liveness attribution rests entirely on the `execpass`/`execfail`/`nolive` population diff, which is strong but indirect.
- **The durable-key claim is behavioural, not observed.** That exec rows land under the ADR-0080 `(alloc_id, role, probe_idx)` key is inferred from correct end-to-end discrimination; no probe-row dump was captured.
- **Per-restart certificate and cgroup/netns churn (§ 4) is inferred from the restart path, not measured.**
- **The pre-Stage-1 explanation for Anomaly 2 is inference** (§ 2.3), and the parse-valid-but-wrong-fixture alternative is not excluded.
- **The HTTP mechanic was not exercised.** It shares `probe_tick` and the hydrate path, so A, B, and C should apply identically — reasoning, not evidence.
- **Startup- and readiness-role evaluation-counting is established by source reading**, not by a dedicated experiment.
- **Concurrent edits.** The working tree gained further probe-surface changes (`render.rs`, `api.rs`, `handlers.rs`, `workload_describe_probes.rs`) *during* this investigation. Measurements are pinned to HEAD `86d6331b` with the Stage-1 tree as it stood at run time. `service_lifecycle.rs`, `workload_lifecycle.rs`, and `exit_observer.rs` were verified **unmodified against HEAD**; `reconciler_runtime.rs`, `driver.rs`, and `action_shim/mod.rs` were dirty and their line numbers are tree-relative.

## 7. Housekeeping

Both runs started and ended clean, verified on the kernel surface: no `alloc-*.scope` cgroups, no `ovd-ns-*` namespaces, loopback `HEALTHY` (no leaked XDP), no stray `overdrive serve`. Every throwaway artifact — three repo-root scripts and all VM temp state — was deleted; fixtures were generated inside the VM's `/tmp` so the repository was never polluted. `git status` shows only the pre-existing ADR-0080 work.
