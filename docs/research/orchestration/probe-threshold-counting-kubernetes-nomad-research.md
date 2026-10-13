# Research: How probe failure/success thresholds are counted — Kubernetes, Nomad, Consul, Docker, ECS — and what Overdrive's ServiceLifecycle should do

**Date**: 2026-10-13 | **Researcher**: nw-researcher (Nova) | **Confidence**: High on kubelet, Consul, Docker; Medium-High on Nomad; Medium on the "no sample" handling | **Sources**: 13 (8 primary source files at pinned tags, 5 official docs pages)

Scope: GH #295 design round 2, finding H-A, and decision 4 of ADR-0187 (PROBE-NOSAMPLE, **proposed**). This document records evidence. It approves nothing.

---

## Executive Summary

In every system examined except Nomad's `check_restart`, a probe's consecutive-failure and consecutive-success counters move **once per probe execution**. The component that runs the probe also does the counting, at the moment the result is produced. In Kubernetes the kubelet's per-probe `worker` increments `resultRun` inside `doProbe`, which runs once per `periodSeconds` tick. Consumers downstream (the results cache, the status manager, the Pod `Ready` condition, the liveness kill path) see only the thresholded verdict. Those consumers are level readers: reading the verdict twice does not count anything twice. Consul's `StatusHandler.updateCheck`, Docker's `handleProbeResult` (`FailingStreak`) and ECS `retries` follow the same per-execution shape. Kubernetes also discards some results outright: a prober *error*, as opposed to a *failure*, is "thrown away" and counts toward neither threshold. A probe that times out is a failure in all of them.

Nomad's `check_restart` is the one exception, and it counts by **time**, not by execution. Its watcher polls check status every second. It records when the check first became unhealthy and restarts once `interval × (limit − 1)` has elapsed without a healthy poll. Re-reading a stale unhealthy status is therefore expected in Nomad: the elapsed time is what gets counted. Because `pending` also counts as unhealthy, a Nomad-native check that produces no result also moves toward a restart.

Overdrive splits the producer (the probe runner writes one row per attempt) from the counter (the `ServiceLifecycle` reconciler counts over a projection that keeps only the latest row). Today the liveness and readiness streaks advance on every reconcile pass. That is neither the per-execution model nor Nomad's time model: a stale sample is counted once per wake-up, and wake-ups are not tied to the probe period. **Advancing the streak once per distinct sample, keyed by the row's observation time, is the consumer-side equivalent of the per-execution norm, and it is the correct fix.** It matches what `update_startup_attempts` already does.

Two refinements are recommended:
- **(a) No sample stays "no verdict".** A period with no sample should change no streak. Bound that exposure by naming the conditions under which no sample is written and making each observable. Do not synthesize failures.
- **(b) Consider having the producer carry the count.** A latest-only projection cannot see samples that land between two reconcile passes, so the consumer under-counts when it is woken less often than the probe runs. The kubelet-equivalent design has the runner carry the count (a per-probe sequence number or run count) in the row.

---

## Research Methodology

**Search strategy**: primary source code at pinned release tags, fetched from `raw.githubusercontent.com`:
- kubernetes `v1.34.0`
- nomad `v1.10.0`
- consul `v1.20.0`
- moby `v26.1.0`

Official documentation came from kubernetes.io, developer.hashicorp.com and docs.aws.amazon.com. The Overdrive code and design were read in this worktree (branch `marcus-sa/spike-netns-density-295-v1`, HEAD `73e643a87`).

**Source selection**: official source and documentation only (High tier). There are no blogs and no secondary sources.

**Verification**: each external claim is backed by the source code at a tag, or by the vendor's own documentation where the code was not fetched. Where both were available they agree, as noted per finding.

**Adversarial validation**: all fetched content was scanned. No injection or directive content was found. The content was treated as data.

**Caveat on fetch fidelity**: quotes came through a summarizing fetch tool. Code quotes were requested verbatim, and they match the known shape of these files. Pages that the tool truncated are listed under Knowledge Gaps.

---

## Part 0 — Overdrive today (verified against this worktree)

### O-1. The probe runner writes one row per attempt, stamped with the clock

`crates/overdrive-worker/src/probe_runner/mod.rs`:
- **`supervised_probe_loop`**: `clock.sleep(interval)`, then `probe_tick`. The loop is sleep-then-probe; the probe's own duration is bounded by `timeout_seconds` (`tokio::time::timeout` in `tcp_prober.rs` and `http_prober.rs`).
- **`probe_tick`**: maps `ProbeOutcome::{Pass, Fail}` to `ProbeStatus`. It stamps `last_observed_at_unix_ms = unix_ms_from_clock(clock)` and calls `write_probe_result(row)`.
- **Timeouts are failures**: `Fail { reason: "timeout after <N>" }`, per the `tcp_prober.rs` and `http_prober.rs` tables.
- **Overdrive already has a no-sample path.** When the adapter returns `Err` (`ProbeRunnerError::ProbeAdapterFailed`), the `?` returns before `write_probe_result`. No row is written, and the loop logs `probe_tick.error` and continues. The same holds for `ObservationWriteFailed`, where the write itself is lost. This is independent of ADR-0187.

### O-2. ServiceLifecycle sees only the latest row per (role, probe_idx)

`crates/overdrive-reconcilers/src/service_lifecycle.rs`:
- **`latest_probe_row`**: `.max_by_key(|p| p.last_observed_at_unix_ms)`.
- **`hydrate_service_alloc_facts`**: carries the observed-at time for **startup only** (`latest_startup_probe_observed_at`). For liveness and readiness it carries the status only (`latest_liveness_probe` and `latest_readiness_probe`, each `.map(|row| row.status.clone())`).

### O-3. When ServiceLifecycle wakes

`interests()` returns `&[ObservationRowKind::AllocStatus, ObservationRowKind::ProbeResult]` (`service_lifecycle.rs:460-462`). It is therefore woken by:
- every accepted probe-row write;
- every alloc-status change;
- the interest router's unconditional 30 s relist (ADR-0084, `.claude/rules/reconcilers.md`);
- any `has_work` self-re-enqueue.

The number of wake-ups is **not tied to the probe period**.

### O-4. Streak arithmetic per role (current)

| Role | Latest = Fail | Latest = Pass | Latest = None | Dedup by sample? |
|---|---|---|---|---|
| Startup (`update_startup_attempts`, :1260-1286) | +1 **only if** `observed_at` ≠ `last_fail_seen_at` | remove counter and seen-at | untouched | **Yes** |
| Liveness (`liveness_terminate_action`, :1044-1095) | `saturating_add(1)` on **every pass** | remove (reset 0) | untouched | **No** |
| Readiness (`compute_backend_healthy`, :1198-1239) | remove (reset 0) | `saturating_add(1)` on **every pass** | **reset 0** | **No** |

Two observations follow from the code. The first is that the asymmetry is real: liveness keeps its streak on `None`, but readiness resets it. In practice, `None` means no row exists for this allocation's probe. Before the first sample that is harmless. The second observation is the defect H-A names: with liveness at `failure_threshold = 3`, one Fail row followed by two unrelated wake-ups (an alloc-status change and a relist) reaches the threshold. The reconciler then emits `StopAllocation { by: LivenessProbe }` on one sample. This happens for every workload kind (exec, process, VM), and it does not need ADR-0187's no-sample refusals. Wake-ups more frequent than the probe interval are enough. The same applies to readiness: one stale Pass can reach `success_threshold`.

ADR-0187 decision 4 and the H-A resolution in `.context/round-2026-10-11/r2-resolutions.md` propose keying the liveness and readiness streaks by the row's observed-at, "exactly as `update_startup_attempts` already dedups startup attempts". The last-counted observed-at would be persisted per (alloc, probe) in `ServiceLifecycleView` as a `UnixInstant`.

---

## Part 1 — Kubernetes (kubelet)

### Finding K-1: Counting happens in the per-probe worker, once per probe execution, on a period ticker

**Evidence (`pkg/kubelet/prober/worker.go@v1.34.0`)**:
- Worker state is `lastResult results.Result` and `resultRun int`.
- `run()` builds `probeTicker := time.NewTicker(probeTickerPeriod)`, where `probeTickerPeriod` is `time.Duration(w.spec.PeriodSeconds) * time.Second`, and calls `doProbe` on each `<-probeTicker.C`.
- `doProbe` contains:

```go
result, err := w.probeManager.prober.probe(ctx, w.probeType, w.pod, status, w.container, w.containerID)
if err != nil {
	// Prober error, throw away the result.
	return true
}
...
if w.lastResult == result {
	w.resultRun++
} else {
	w.lastResult = result
	w.resultRun = 1
}

if (result == results.Failure && w.resultRun < int(w.spec.FailureThreshold)) ||
	(result == results.Success && w.resultRun < int(w.spec.SuccessThreshold)) {
	// Success or failure is below threshold - leave the probe state unchanged.
	return true
}

w.resultsManager.Set(w.containerID, result, w.pod)

if (w.probeType == liveness && result == results.Failure) || w.probeType == startup {
	// The container fails a liveness/startup check, it will need to be restarted.
	// Stop probing until we see a new container ID. ...
	w.onHold = true
	w.resultRun = 0
}
```

**Source**: [kubelet prober worker.go @ v1.34.0](https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/kubelet/prober/worker.go). Accessed 2026-10-13.
**Confidence**: High (primary source).

**Analysis**:
- `resultRun` counts consecutive identical results. Each execution of `doProbe` changes it exactly once, and `doProbe` runs once per ticker tick.
- **The opposite result resets the run.** A Success after Failures sets `resultRun = 1` for Success.
- **Crossing a liveness-failure or startup threshold also resets the run.** It sets `resultRun = 0` and puts the worker `onHold`.
- **A new container resets the hold.** When the container ID changes, the worker calls `resultsManager.Remove(old)`, then `Set(new, w.initialValue)`, and sets `w.onHold = false`.
- **A non-running container short-circuits the probe.** Its result is set to `Failure` directly ("Non-running container probed") and no probe runs.
- **Stale re-counting is structurally impossible.** The counter lives with the producer and moves only when a probe actually executed.

### Finding K-2: Downstream consumers read a level; repeated reads count nothing

**Evidence (`pkg/kubelet/prober/results/results_manager.go@v1.34.0`)**:
- The `Updates` channel doc: "Updates creates a channel that receives an Update whenever its result changes (but not removed)."
- `setInternal` returns true only when `!exists || prev != result`, and `Set` pushes an `Update` only in that case.

**Evidence (`pkg/kubelet/prober/prober_manager.go@v1.34.0`)**: in `UpdatePodStatus`, container `ready` is computed from `m.readinessManager.Get(containerID)` == `results.Success`. With no readiness worker, the code comment is "no readinessProbe -> always ready".

**Sources**: [results_manager.go](https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/kubelet/prober/results/results_manager.go) and [prober_manager.go](https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/kubelet/prober/prober_manager.go). Accessed 2026-10-13.
**Confidence**: High.

**Analysis**: the status manager and the Pod `Ready` condition consume the **thresholded verdict** held in a cache. Re-reading it on every pod-status sync is idempotent, because it is a level and not an event to count. In Kubernetes, the stale-result problem cannot arise at the consumer: the consumer never counts.

### Finding K-3: A timeout is a Failure; a prober error is no sample

**Evidence**:
- **`pkg/probe/http/http.go@v1.34.0`, `DoHTTPProbe`**: on a `client.Do(req)` error it does `return probe.Failure, err.Error(), nil`, with the comment "Convert errors into failures to catch timeouts."
- **`pkg/kubelet/prober/prober.go@v1.34.0`**: `timeout := time.Duration(p.TimeoutSeconds) * time.Second` is passed to the exec, HTTP, TCP and gRPC probes. `probe()` retries internally through `runProbeWithRetries(..., maxProbeRetries)` (3) and returns `results.Failure, err` on a hard error.
- **`worker.go`**: a non-nil `err` is "throw away the result" (Finding K-1).

**Sources**: [http.go](https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/probe/http/http.go) and [prober.go](https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/kubelet/prober/prober.go). Accessed 2026-10-13.
**Confidence**: High for the HTTP path. The exec path's timeout-to-failure was not fetched (see Knowledge Gaps).

**Analysis**: Kubernetes has **two non-success outcomes with different semantics**:
- A **probe failure**, including a timeout and a connection error, counts toward `failureThreshold`.
- A **prober error**, where the prober itself could not run the check, records nothing. It neither advances nor resets the run.

That is the same split ADR-0187 makes: a dial refusal caused by the node is no sample, and a dial result attributable to the workload is a failure.

### Finding K-4: API semantics: "consecutive" executions

The Probe API field descriptions read:
- `failureThreshold`: "Minimum consecutive failures for the probe to be considered failed after having succeeded. Defaults to 3. Minimum value is 1."
- `successThreshold`: "Minimum consecutive successes for the probe to be considered successful after having failed. Defaults to 1. Must be 1 for liveness and startup. Minimum value is 1."
- `periodSeconds`: "How often (in seconds) to perform the probe."

**Source**: [Kubernetes API reference — Pod v1, Probe](https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/pod-v1/). Accessed 2026-10-13.
**Confidence**: Medium. The fetched page was truncated before the Probe section, and these wordings are the well-known `types.go` doc comments, not re-quoted from this fetch. The code in K-1 independently establishes the semantics.

### What happens when a kubelet probe never returns (inference, Medium)

- **The probe is bounded.** `doProbe` runs synchronously in the worker goroutine. The HTTP, TCP and gRPC probes are bounded by `timeoutSeconds`, so a probe that would never return becomes a Failure at the timeout.
- **A hang beyond the timeout produces no sample, and that is not a failure.** If the prober itself hangs past its timeout (for example a stuck container-runtime exec call), the worker cannot run the next `doProbe`. Go's `time.Ticker` drops ticks for a slow receiver, so no extra executions accumulate and no failure is synthesized. Kubernetes does not treat the absence of a sample as a failure, consistent with K-3's "throw away" on error. This is inferred from the code structure. No kubelet document states it.

---

## Part 2 — Nomad `check_restart`

### Finding N-1: Documented semantics are "consecutive failures"; one pass resets

**Evidence**:
- `limit`: "Restart task when a health check has failed `limit` times. For example 1 causes a restart on the first failure. The default, `0`, disables health check based restarts. Failures must be consecutive. A single passing check will reset the count, so flapping services may not be restarted."
- `grace`: "Duration to wait after a task starts or restarts before checking its health."
- `ignore_warnings`: "Setting `ignore_warnings = true` treats a `warning` status like `passing` ... Only available in the Consul service provider."

**Source**: [Nomad check_restart block](https://developer.hashicorp.com/nomad/docs/job-specification/check_restart). Accessed 2026-10-13.
**Confidence**: High (official documentation).

### Finding N-2: The implementation counts elapsed time, polled every second, for both providers

**Evidence (`client/serviceregistration/watcher.go@v1.10.0`)**:
- The `restarter` struct holds `unhealthyState time.Time`, `graceUntil time.Time`, `timeLimit time.Duration` and `ignoreWarnings bool`, with `timeLimit: check.Interval * time.Duration(check.CheckRestart.Limit-1)`.
- The watcher loop is:

```go
// poll time; refresh check statuses
case now := <-checkTimer.C:
	w.interval(ctx, now, watched)
	checkTimer.Reset(w.pollFrequency)
```

  `pollFrequency` defaults to `1 * time.Second` in `NewCheckWatcher()`.
- `apply`:

```go
switch status {
case "critical": // consul
case string(structs.CheckFailure): // nomad
case string(structs.CheckPending): // nomad
case "warning": // consul
	if r.ignoreWarnings { healthy(); return false }
default:
	// All other statuses are ok, reset state and exit
	healthy()
	return false
}
if now.Before(r.graceUntil) { return false }
if r.unhealthyState.IsZero() {
	// First failure, set restart deadline
	r.unhealthyState = now
}
// restart timeLimit after start of this check becoming unhealthy
restartAt := r.unhealthyState.Add(r.timeLimit)
// Must test >= because if limit=1, restartAt == first failure
if now.Equal(restartAt) || now.After(restartAt) { ... asyncRestart ...; return true }
return false
```

**Source**: [nomad client/serviceregistration/watcher.go @ v1.10.0](https://raw.githubusercontent.com/hashicorp/nomad/v1.10.0/client/serviceregistration/watcher.go). Accessed 2026-10-13.
**Confidence**: Medium-High. This is primary source. The older path `command/agent/consul/check_watcher.go` returned 404 at v1.10.0. The unified watcher serves both the Consul and Nomad providers, as the `// consul` and `// nomad` case labels show.

**Analysis**:
- **Nomad does not count executions.** It polls the current check status, which is a level, once a second. It records the first poll at which the check is unhealthy, and restarts when `(limit − 1) × interval` of wall-clock time has passed with no healthy poll in between. "Limit consecutive failures" is approximated by assuming one check execution per `interval`.
- **Re-reading a stale unhealthy status is the design.** Time, not the sample, is the unit counted.
- **`CheckPending` counts as unhealthy.** For Nomad-native checks a check with no result yet moves toward a restart once `grace` has expired. A check that cannot produce results therefore restarts the task.
- **The approximation can drift.** If check executions are slower than `interval` (for example a long timeout), a restart can fire after fewer actual failing executions than `limit`. This is inferred from the arithmetic.

### Finding N-3: Nomad-native checks: a timeout or connection error is a failure

**Evidence (`client/serviceregistration/checks/client.go@v1.10.0`)**:
- Results are initialized `Status: structs.CheckPending`.
- The timeout is applied as `context.WithTimeout(ctx, q.Timeout)`.
- A TCP `DialContext` error, or an HTTP request error, gives `qr.Status = structs.CheckFailure`. An HTTP 4xx/5xx is also `CheckFailure`, and 2xx/3xx is `CheckSuccess`.

**Source**: [nomad checks/client.go @ v1.10.0](https://raw.githubusercontent.com/hashicorp/nomad/v1.10.0/client/serviceregistration/checks/client.go). Accessed 2026-10-13.
**Confidence**: Medium-High (primary source, summarized quotes).

---

## Part 3 — Other precedent

### Finding C-1: Consul counts per check execution

**Evidence (docs)**:
- `success_before_passing`: "how many consecutive times the check must pass before Consul marks the service or node as `passing`."
- `failures_before_critical`: "how many consecutive times the check must fail before Consul marks the service or node as `critical`."
- `failures_before_warning` likewise.

**Evidence (`agent/checks/check.go@v1.20.0`)**: `StatusHandler` holds `successBeforePassing`, `successCounter`, `failuresBeforeWarning`, `failuresBeforeCritical` and `failuresCounter`. `updateCheck` works as follows:
- On passing or warning, it increments `successCounter` and resets failures. The status propagates only when `successCounter >= successBeforePassing`.
- On critical, it increments `failuresCounter` and resets successes. It escalates to warning or critical at the respective thresholds.
- The monitor (script) check reports a timeout as critical ("Timed out (%s) running check").

**Sources**: [Consul health-check reference](https://developer.hashicorp.com/consul/docs/reference/service/health-check) and [consul agent/checks/check.go @ v1.20.0](https://raw.githubusercontent.com/hashicorp/consul/v1.20.0/agent/checks/check.go). Accessed 2026-10-13.
**Confidence**: High on the counting. The HTTP and TCP timeout-to-critical mapping is Medium, because the fetch summarized rather than quoted those lines.

**Analysis**: `updateCheck` is called by each check type's run loop once per execution, so the counter sits with the producer, as in kubelet. Consul's published status is a level, and Nomad's watcher reads that level (N-2).

### Finding D-1: Docker HEALTHCHECK `retries` counts per probe result

**Evidence (`daemon/health.go@v26.1.0`, `handleProbeResult`)**:
- A healthy exit sets `h.FailingStreak = 0` and the status to Healthy.
- Otherwise, `if shouldIncrementStreak { h.FailingStreak++; if h.FailingStreak >= retries { h.SetStatus(types.Unhealthy) } }`.
- During `Starting`, `shouldIncrementStreak = false` while `timeSinceStart < startPeriod`.
- A timeout yields `ExitCode: -1`, `"Health check exceeded timeout (%v)"`, which is a failing result.

**Source**: [moby daemon/health.go @ v26.1.0](https://raw.githubusercontent.com/moby/moby/v26.1.0/daemon/health.go). Accessed 2026-10-13. (The `v27.0.0` path returned 404.)
**Confidence**: High (primary source).

### Finding E-1: ECS counts per execution, and keeps the "last heard from" status when no check runs

**Evidence**:
- `retries`: "The number of times to retry a failed health check before the container is considered unhealthy."
- `timeout`: "The time period in seconds to wait for a health check to succeed before it is considered a failure."
- `startPeriod`: "grace period ... before failed health checks count towards the maximum number of retries."
- On the agent disconnecting: "If the Amazon ECS container agent becomes disconnected from the Amazon ECS service, this won't cause a container to transition to an `UNHEALTHY` status. ... The health check status is the "last heard from" response from the Amazon ECS agent, so if the container was considered `HEALTHY` prior to the disconnect, that status will remain until the agent reconnects and another health check occurs. There are no assumptions made about the status of the container health checks."

**Source**: [AWS ECS API — HealthCheck](https://docs.aws.amazon.com/AmazonECS/latest/APIReference/API_HealthCheck.html). Accessed 2026-10-13.
**Confidence**: High (official documentation).

**Analysis**: this is the closest published precedent for ADR-0187's rule that a missing sample is not itself a verdict. When no health check result arrives, ECS keeps the last verdict and assumes nothing.

---

## Part 4 — Synthesis for Overdrive

### S-1. Is "advance the streak once per distinct probe sample" the norm? Yes.

| System | Unit counted | Counter location | Timeout | No result produced |
|---|---|---|---|---|
| Kubernetes kubelet | probe execution (`doProbe` per `periodSeconds`) | producer (worker) | Failure | prober error is thrown away; nothing counted (K-3) |
| Consul | check execution (`updateCheck`) | producer (StatusHandler) | critical | — (not determined) |
| Docker | probe result (`handleProbeResult`) | producer (daemon monitor) | failure (exit −1) | — |
| ECS | health check execution (`retries`) | producer (agent) | failure | last verdict retained, "no assumptions" (E-1) |
| Nomad `check_restart` | **elapsed time** while the polled level is unhealthy | consumer (watcher, 1 s poll) | failure | `pending` counts as unhealthy (N-2) |
| Overdrive today (liveness, readiness) | **reconcile pass** | consumer (ServiceLifecycle) | Fail | adapter `Err` writes no row (O-1) |

Overdrive's current per-pass counting matches neither family:
- It is not per execution, because one sample can be counted many times (O-4).
- It is not Nomad's time-based model either, because a pass is not a unit of time. The 30 s relist, alloc-status changes and `has_work` re-enqueues are arbitrary wake-ups.

With per-pass counting, the operator's `failure_threshold` and `success_threshold` mean "N wake-ups", which no operator can reason about. Counting once per distinct sample restores what the threshold states ("N consecutive results"), the meaning used by Kubernetes, Consul, Docker and ECS. **Verified**: the norm is per execution. **Inference**: deduplicating by the row's observation time is the consumer-side way to get per-execution counting when the counter cannot live in the producer.

### S-2. Downsides and how precedent handles them

1. **A stuck runner, or a refused dial, produces no samples, so the streak never advances.**
   - Kubernetes and ECS accept this. Kubernetes discards prober errors and does not synthesize failures for missed ticks. ECS keeps the "last heard from" verdict.
   - Nomad does not accept it. `pending` is unhealthy, and time keeps counting.
   - **Recommendation: no new sample changes no streak and no verdict** (as ADR-0187 decision 4 states). Making absence count as failure would reintroduce the H-A defect by another route. It would also cross owning domains: a node-wide quiesce, or another tenant's load, would restart a healthy workload.
   - Absence must instead be **bounded and observable**. A probe that would never return is already converted to a sample by `timeout_seconds` (O-1), which is the norm (K-3, N-3, C-1, D-1, E-1). What remains is genuine no-sample: ADR-0187's capacity and quiesce refusals, plus the existing `ProbeAdapterFailed` and `ObservationWriteFailed` paths (O-1).
   - Each no-sample case should emit a structured, per-period event, as ADR-0187 does for its two refusals. The existing `probe_tick.error` warn is the analogue for the adapter path.
   - **Gap to surface to DESIGN**: nothing today bounds the *duration* of a no-sample window. For ADR-0187 the window ends when the node condition ends (its stated exposure). For a runner task that dies or wedges, the probe supervisor (ADR-0080) owns liveness of the runner itself. This research did not establish whether the supervisor restarts a dead per-role task.

2. **Under-counting when the reconciler is woken less often than the probe runs.**
   - `latest_probe_row` keeps only the newest row per (role, probe_idx) (O-2). If two samples of the same probe land between two reconcile passes, observation-time deduplication counts one.
   - Wake-ups are driven by probe-row writes (O-3), and the probe interval is at least 2 s against a 100 ms broker tick, so coalescing should be rare. Under broker backlog it is possible, and the effect is a threshold reached later than stated, or a Fail-Pass-Fail sequence read as two Fails.
   - Kubernetes avoids this by counting in the producer. The equivalent here, which the user would need to decide, is for the probe runner to carry the count in the row: a monotone per-probe sample sequence, or the producer's consecutive-same-result run length. The consumer could then compute the exact delta.
   - Carrying the run length is "persist inputs" (`.claude/rules/development.md`), and the threshold comparison stays in the reconciler against the live spec. This is an interpretation, not a precedent-mandated requirement.

3. **Same timestamp, different sample.** Two attempts stamped in the same millisecond would be deduplicated as one. With the runner's sleep-then-probe loop and an interval of at least 1 s per role, this cannot happen within one (alloc, role, probe_idx) except through clock regression. A wall-clock step backwards is an existing `UnixInstant` property: compare for inequality, not ordering, as `update_startup_attempts` does (`!= Some(observed_at)`). A per-probe sequence number (item 2) would remove the clock dependence altogether.

4. **Reset semantics need pinning per role.** Today, readiness resets its streak on `None` and liveness does not (O-4). With deduplication, `None` occurs only when no row exists for the probe, before the first sample or after the allocation's rows are reclaimed. Kubernetes sets the initial value on a new container (K-1). Overdrive gets the same effect from per-allocation keys: a restart is a fresh `AllocationId`. The contract should state the `None` behaviour once, consistently for every role.

5. **The streak resets on a terminate.** Overdrive already removes the liveness counter when it emits `StopAllocation` (ADR-0087 D2, :1078-1085). This mirrors kubelet's `resultRun = 0; onHold = true` on a liveness failure. The last-counted observation time should be cleared at the same point (or keyed per allocation so that it dies with the allocation). Otherwise, a re-read of the terminate-triggering row could start a new streak at 1 on a successor that shares a key. Per-`AllocationId` keys make this moot.

### S-3. Recommended semantics (for DESIGN; requires user approval per `.claude/rules/design.md`)

1. **Counting unit.** Every probe streak (startup attempts, the liveness failure streak, the readiness success streak) advances **at most once per distinct probe sample**. A sample is identified by its (alloc, role, probe_idx, observed-at). A reconcile pass that observes the same sample again changes no streak. This is the kubelet / Consul / Docker / ECS semantics. **Supported.**
2. **The opposite result resets.** A new Pass resets the failure streak, and a new Fail resets the success streak. A reset applies only to a *new* sample. Re-reading a stale opposite-result row has already been counted once, so it changes nothing further. **Supported** by K-1, C-1, D-1 and N-1.
3. **Timeout is a sample (a failure)**, unchanged from today. **Supported** by every source.
4. **No sample is no verdict.** An attempt that writes no row advances, resets and decides nothing. The last verdict stands. This covers ADR-0187's node refusals, adapter errors and write failures. **Supported** by Kubernetes (K-3) and ECS (E-1). Nomad is the lone counter-example (N-2), and its "pending is unhealthy" choice is what ADR-0187's alternatives already reject for node-wide conditions.
5. **No-sample periods must be observable**: one structured event per probe per period, naming the cause. **Interpretation**, extending ADR-0187's not-sampled event to all no-row paths.
6. **Persist the input.** Store the last-counted observation time per (alloc, probe) as a `UnixInstant` in `ServiceLifecycleView`, additive under `#[serde(default)]`, as H-A specifies. Compare with `!=`.
7. **Open option for the user (not a precedent requirement).** Should the probe row carry a producer-side sequence or run count, so that coalesced samples are not under-counted (S-2 item 2)? This would make Overdrive's counter exactly the kubelet's, rather than an approximation of it.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| kubelet `prober/worker.go` @ v1.34.0 | github (kubernetes) | High | primary source | 2026-10-13 | Y (results_manager, prober.go) |
| kubelet `prober/results/results_manager.go` @ v1.34.0 | github | High | primary source | 2026-10-13 | Y |
| kubelet `prober/prober_manager.go` @ v1.34.0 | github | High | primary source | 2026-10-13 | Y |
| kubelet `prober/prober.go` @ v1.34.0 | github | High | primary source | 2026-10-13 | Y (http.go) |
| `pkg/probe/http/http.go` @ v1.34.0 | github | High | primary source | 2026-10-13 | Y |
| Kubernetes API reference, Pod v1 | kubernetes.io | High | official docs (truncated) | 2026-10-13 | Partial |
| Nomad `check_restart` docs | developer.hashicorp.com | High | official docs | 2026-10-13 | Y (watcher.go) |
| Nomad `client/serviceregistration/watcher.go` @ v1.10.0 | github | High | primary source | 2026-10-13 | Y (docs) |
| Nomad `client/serviceregistration/checks/client.go` @ v1.10.0 | github | High | primary source | 2026-10-13 | N |
| Consul health-check reference | developer.hashicorp.com | High | official docs | 2026-10-13 | Y (check.go) |
| Consul `agent/checks/check.go` @ v1.20.0 | github | High | primary source | 2026-10-13 | Y (docs) |
| moby `daemon/health.go` @ v26.1.0 | github | High | primary source | 2026-10-13 | N |
| AWS ECS API HealthCheck | docs.aws.amazon.com | High | official docs | 2026-10-13 | N |

Reputation: High 13/13 (100%). Average 1.0.

## Knowledge Gaps

- **Probe API doc text not re-quoted.** **Issue**: the Kubernetes Probe field doc text was not re-quoted verbatim, because the API reference and task pages were truncated by the fetch tool. **Attempted**: kubernetes.io concept, task and API-reference pages, and `kubernetes/api` `core/v1/types.go` (truncated). **Recommendation**: quote it from `staging/src/k8s.io/api/core/v1/types.go` `type Probe struct` if a verbatim citation is needed. The code (K-1) carries the semantics independently.
- **Exec-probe timeout handling not fetched.** **Issue**: `pkg/probe/exec` and the `ExecProbeTimeout` feature gate were not fetched. **Recommendation**: read `pkg/probe/exec/exec.go` @ v1.34.0 if exec-probe semantics are needed.
- **The old Nomad file path.** **Issue**: `command/agent/consul/check_watcher.go` (the path named in the brief) returned 404 at v1.10.0. Its logic now lives in `client/serviceregistration/watcher.go`. Pre-1.3 history was not read.
- **Consul's per-check timeout mapping.** **Issue**: for HTTP and TCP checks this was summarized, not quoted.
- **Overdrive supervisor behaviour on a dead per-role runner task.** **Issue**: it is not established in this research whether ADR-0080's supervisor restarts a probe task that died or wedged, which bounds the "stuck runner" no-sample window. **Recommendation**: read `crates/overdrive-worker/src/probe_runner/supervisor.rs` during DESIGN.
- **Broker coalescing.** **Issue**: whether `EvaluationBroker` coalesces two probe-row wake-ups for the same target before a drain (S-2 item 2) was not verified.

## Conflicting Information

**Count executions versus count time.** Kubernetes, Consul, Docker and ECS count executions. Nomad counts elapsed time under a polled level and treats `pending` as unhealthy. **Assessment**: Nomad's model is a consumer-side approximation, forced by its watcher reading Consul's published level. It does not contradict the per-execution meaning that its own documentation states ("Failures must be consecutive"). For Overdrive, whose reconciler reads raw samples rather than a published level, the per-execution family is the applicable precedent.

## Full Citations

[1] Kubernetes. "pkg/kubelet/prober/worker.go". v1.34.0. https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/kubelet/prober/worker.go. Accessed 2026-10-13.
[2] Kubernetes. "pkg/kubelet/prober/results/results_manager.go". v1.34.0. https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/kubelet/prober/results/results_manager.go. Accessed 2026-10-13.
[3] Kubernetes. "pkg/kubelet/prober/prober_manager.go". v1.34.0. https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/kubelet/prober/prober_manager.go. Accessed 2026-10-13.
[4] Kubernetes. "pkg/kubelet/prober/prober.go". v1.34.0. https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/kubelet/prober/prober.go. Accessed 2026-10-13.
[5] Kubernetes. "pkg/probe/http/http.go". v1.34.0. https://raw.githubusercontent.com/kubernetes/kubernetes/v1.34.0/pkg/probe/http/http.go. Accessed 2026-10-13.
[6] Kubernetes. "Pod v1 — Probe". https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/pod-v1/. Accessed 2026-10-13.
[7] HashiCorp. "check_restart block". Nomad docs. https://developer.hashicorp.com/nomad/docs/job-specification/check_restart. Accessed 2026-10-13.
[8] HashiCorp. "client/serviceregistration/watcher.go". Nomad v1.10.0. https://raw.githubusercontent.com/hashicorp/nomad/v1.10.0/client/serviceregistration/watcher.go. Accessed 2026-10-13.
[9] HashiCorp. "client/serviceregistration/checks/client.go". Nomad v1.10.0. https://raw.githubusercontent.com/hashicorp/nomad/v1.10.0/client/serviceregistration/checks/client.go. Accessed 2026-10-13.
[10] HashiCorp. "Health check configuration reference". Consul docs. https://developer.hashicorp.com/consul/docs/reference/service/health-check. Accessed 2026-10-13.
[11] HashiCorp. "agent/checks/check.go". Consul v1.20.0. https://raw.githubusercontent.com/hashicorp/consul/v1.20.0/agent/checks/check.go. Accessed 2026-10-13.
[12] Moby. "daemon/health.go". v26.1.0. https://raw.githubusercontent.com/moby/moby/v26.1.0/daemon/health.go. Accessed 2026-10-13.
[13] AWS. "HealthCheck — Amazon ECS API Reference". https://docs.aws.amazon.com/AmazonECS/latest/APIReference/API_HealthCheck.html. Accessed 2026-10-13.

Local evidence (this worktree):
- `crates/overdrive-reconcilers/src/service_lifecycle.rs`: `liveness_terminate_action` :1044-1095, `compute_backend_healthy` :1198-1239, `update_startup_attempts` :1260-1286, `latest_probe_row` :864-872, `interests` :460-462.
- `crates/overdrive-worker/src/probe_runner/mod.rs`: `probe_tick`, `supervised_probe_loop`.
- `docs/product/architecture/adr-0187-...-records-no-sample.md` (proposed).
- `.context/round-2026-10-11/r2-resolutions.md`, finding H-A.

## Research Metadata

Duration: ~1 session. Examined: 16 external fetches (3 failed or truncated) and 6 local files. Cited: 13 external sources. Confidence: High ~70%, Medium-High ~15%, Medium ~15%. Output: `docs/research/orchestration/probe-threshold-counting-kubernetes-nomad-research.md`.
