# Research: Restart budgets for launches that never ran

**Date**: 2026-11-24 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High (direction), Medium (mechanism) | **Sources**: 14 upstream/official pages

## Executive Summary
No system studied charges a launch that never ran as a crash. Kubernetes (backoff keyed on the finish time of an exited container), Nomad (user restarts and kills return before the counting code; the tracker is in memory and rebuilt on agent restart) and Erlang/OTP (only supervisor-initiated restarts of an exited child are recorded; `terminate_child` and `restart_child` never are) all keep a never-ran event out of the budget. Docker and systemd are the counter-examples: both count start attempts without asking whether the process ran.

Every system that keeps history keeps it apart from the budget (Kubernetes `restartCount` vs the backoff entry; Nomad `TaskState.Restarts` and events vs `RestartTracker`). None keeps both in one field, and none relies on a row being skipped by a lookup. Overdrive's abandoned row is the history; the question is only how the budget is derived from the rows.

On operator-restart forgiveness the evidence is split. Docker resets the count on a manual start (`ResetRestartManager(true)` sets `RestartCount = 0`); Kubernetes forgives by replacing the pod (new backoff identity); systemd does NOT (a manual start counts, and only `reset-failed` flushes); Nomad is neutral (a user restart neither charges nor resets). No studied system makes the outcome depend on whether a never-ran launch sat in between. That supports the proposed fix as a transparency rule: deleting abandoned rows from a chain must not change any successor's count.

## Research Methodology
**Search Strategy**: upstream source and official docs for kubelet, Nomad, systemd, Erlang/OTP, Docker/moby; local ADR-0355, ADR-0335, restart_budget.qnt.
**Source Selection**: official docs and upstream GitHub source; no blogs.
**Quality Standards**: 3 sources/claim ideal, 1 authoritative minimum; claims below 3 are rated accordingly.

## Local frame (read from the repository)
- `restart_budget.qnt` (definitions only): the budget is the per-allocation count at issue. Fresh placement 0; crash restart `cnt(cur)+1`; generation replacement `cnt(cur)` unchanged, which forgives a Failed predecessor. `charged = cnt(cur) + (Failed ? 1 : 0)`; exhausted when `charged >= LIMIT` with nothing running. `cur` is the highest allocation id with a row.
- ADR-0355: every launch that ends before running ends its `Pending` row with a `Terminated` row (`LaunchSuperseded` / `LaunchLostWithControlPlane`), never `Failed`, `terminal` absent. "An abandoned launch charges nothing ... its relaunch carries the abandoned allocation's own count." The abandoned row is "a restartable row it relaunches under the existing path."
- The scenario's defect: `cur` is now the abandoned #2 (count c+1, non-Failed), so an operator replacement #3 carries c+1 where, with no abandoned row, it would read Failed #1 (count c) and carry c. ADR-0355's "charges nothing" is satisfied for #2 but not for #3, which pays for #1's failure that the operator restart should have forgiven.
- (RA9 / R-1 README rows: the grep returned only the RA9 row text, which states the budget abstraction R-1 = A and `LIMIT` = production ceiling + 1; ADR-0335 was not read in full, see Gaps.)

## Findings

### F1. Kubernetes: backoff is keyed off an EXITED container; a launch that never ran has no exit and no backoff charge
**Evidence**: kubelet `doBackOff` — "Use the finished time of the latest exited container as the start point to calculate whether to do back-off"; it loops statuses for `c.State == kubecontainer.ContainerStateExited`, and with no exited status returns "no backoff". `backOff.Next` is called only inside `doBackOff`; no `backOff.Reset` appears in `kuberuntime_manager.go`; the key comes from `GetBackoffKey(pod, container)`. Docs: "Once a container has executed for 10 minutes without any problems, the kubelet resets the restart backoff timer for that container"; delays 10s, 20s, 40s ... capped at 300s.
**Source**: [kuberuntime_manager.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/kuberuntime/kuberuntime_manager.go) (read via raw view); [Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/) - Accessed 2026-11-24
**Confidence**: Medium (upstream source + official docs agree; `GetBackoffKey` body not read, so the exact key composition is unverified)
**Analysis**: Kubernetes never charges a "launch" as such. The only budget input is the time of the last exit. Recovery from the budget is a duration of healthy running, not an operator action. A container that was killed before it ran leaves no exited status and so contributes nothing.

### F2. Kubernetes: `restartCount` is lifetime history, not budget; it is derived from the previous status, and falls back to the log directory
**Evidence**: `startContainer`: `restartCount = containerStatus.RestartCount + 1` when a previous status exists, otherwise `calcRestartCountByLogDir(logDir)` (highest `N.log` + 1; comment: some runtimes clear state on reboot, which would reset the count to 0). Toolchain note: `toKubeContainerStatus` sets `StartedAt` only when state != `CONTAINER_CREATED`.
**Source**: [kuberuntime_container.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/kuberuntime/kuberuntime_container.go) - Accessed 2026-11-24
**Confidence**: Medium (single upstream file; whether a created-never-started container increments the count was NOT confirmed — see Gaps)
**Analysis**: The count is a history field. The decision input (the backoff entry) is separate from it. This is the same split Overdrive wants between a history row and a budget count.

### F3. Kubernetes: `kubectl rollout restart` creates new pods, so it does not "forgive" within a pod; it replaces the unit that owns the backoff
**Evidence**: `ObjectRestarterFn` sets `obj.Spec.Template.ObjectMeta.Annotations["kubectl.kubernetes.io/restartedAt"] = time.Now().Format(time.RFC3339)`, applied as a strategic merge patch. A pod-template change makes the Deployment create a new ReplicaSet and new Pod objects (standard Deployment behaviour; not read from source in this run).
**Source**: [objectrestarter.go](https://github.com/kubernetes/kubectl/blob/master/pkg/polymorphichelpers/objectrestarter.go), [rollout_restart.go](https://github.com/kubernetes/kubectl/blob/master/pkg/cmd/rollout/rollout_restart.go) - Accessed 2026-11-24
**Confidence**: Medium (annotation confirmed in source; new-pod consequence inferred, and no source states "backoff resets on rollout restart")
**Analysis**: Kubernetes has no "operator restart forgives budget" rule. Forgiveness is a side effect of the identity change (new pod UID, new backoff key). The old pod's backoff entry is not consulted by the new pod. Result equals "the replacement starts at 0", independent of any intermediate pod that was never run. Also: a spec change inside a pod restarts containers "regardless of the restart policy" but `doBackOff` "never looks at the hash", i.e. an operator-driven restart in place is still subject to the existing backoff.

### F4. Nomad: a user-triggered restart neither charges nor resets the restart budget; it is skipped before the counting code
**Evidence**: `RestartTracker.SetRestartTriggered(failure bool)`: "When failure is false the task is restarted without considering the restart policy." In `GetState`, the `restartTriggered` branch returns `structs.TaskRestarting, 0` BEFORE the interval block that does `r.count = 0` (new interval) and `r.count++`. `SetKilled` likewise returns `TaskKilled` before the interval block. So user restarts and kills leave `count` and `startTime` untouched.
**Source**: [restarts.go](https://github.com/hashicorp/nomad/blob/main/client/allocrunner/taskrunner/restarts/restarts.go) - Accessed 2026-11-24
**Confidence**: Medium (one upstream source file; consistent with docs, which say `attempts` limits restarts "in the configured interval" and mention no manual-restart exception)
**Analysis**: This is the "transparent" model: a non-failure event is invisible to the count. Nomad does NOT forgive prior failures on user restart either; only the sliding interval does ("begins when the first task starts"; count zeroed lazily when `now > startTime + interval`).

### F5. Nomad: the restart tracker is in-memory only and is rebuilt on client restart; a launch lost with the agent is therefore never charged
**Evidence**: `tr.restartTracker = restarts.NewRestartTracker(rp, tr.alloc.Job.Type, config.Task.Lifecycle)` in `NewTaskRunner`; `Restore()` loads `TaskState` and handle but "never touches the tracker". Persisted: `TaskState` (`Restarts`, `LastRestart`, events). A restored handle: "do *not* append a new task event as restoring is a client event". If `RecoverTask` fails, the task is destroyed and a `TaskRestoreFailed` event is emitted.
**Source**: [task_runner.go](https://github.com/hashicorp/nomad/blob/main/client/allocrunner/taskrunner/task_runner.go) - Accessed 2026-11-24
**Confidence**: Medium
**Analysis**: Nomad keeps two things apart: history (`TaskState.Restarts`, events, persisted) and budget (tracker, volatile, reset by agent restart). A client-lost launch costs nothing, and the history stays honest (a `TaskRestoreFailed` event). The cost: an agent crash loop silently forgives the crash budget (a known weakness, not documented as intended).

### F6. Nomad reschedule: node loss and drain do not use the reschedule budget
**Evidence**: "Rescheduling happens when the tasks of an allocation fail more than their restart block allows." "Node drains and lost nodes don't use the reschedule block."
**Source**: [reschedule](https://developer.hashicorp.com/nomad/docs/job-specification/reschedule), [restart](https://developer.hashicorp.com/nomad/docs/job-specification/restart) - Accessed 2026-11-24
**Confidence**: Medium (two official pages; the "lost nodes" statement was relayed by the fetch tool, not re-quoted)
**Analysis**: Platform-caused loss is a different cause class from task failure, charged to nothing.

### F7. systemd: manual starts count against the start limit; only `reset-failed` forgives; condition-failed starts do not count
**Evidence**: "Units which are started more than burst times within an interval time span are not permitted to start any more." "they apply to all kinds of starts (including manual), not just those triggered by the `Restart=` logic." "unit activations with failing conditions do not count towards the rate limit." "`systemctl reset-failed` will cause the restart rate counter for a service to be flushed."
**Source**: [systemd.unit(5)](https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html) - Accessed 2026-11-24. Source (`unit.c`: `unit_test_start_limit` uses `ratelimit_below`) partly read; the call sites were not.
**Confidence**: Medium-High (official doc, one source family; the doc is explicit)
**Analysis**: systemd's forgiveness is an explicit, separate operator verb. A manual start is charged like any other. A start that is refused before it launches (a failed condition) is not charged. `service.c` also states that `n_restarts` "is not updated until a restart job is enqueued", i.e. the restart history counter and the start rate limiter are different counters.

### F8. Erlang/OTP: only a supervisor-initiated restart of an exited child enters the intensity window; manual `terminate_child` / `restart_child` do not
**Evidence**: `add_restart/1` "is called only from `restart/2`", reached only from the permanent and transient clauses of `do_restart` and from the `try_again_restart` cast. `terminate_child` calls `do_terminate` and `del_child`; `restart_child` calls `do_start_child` directly. Neither touches the `restarts` list. Docs: "if more than MaxR restarts occur within MaxT seconds, the supervisor terminates all child processes and then itself." Defaults intensity 1, period 5. Normal and `shutdown` exits, and all temporary-child exits, add nothing. A retry after a failed restart (`try_again_restart`) does add an entry per retry.
**Source**: [supervisor.erl](https://github.com/erlang/otp/blob/master/lib/stdlib/src/supervisor.erl); [supervisor docs](https://www.erlang.org/doc/apps/stdlib/supervisor.html) - Accessed 2026-11-24
**Confidence**: Medium-High (source plus docs; docs are silent on the manual-operation question, the source answers it)
**Analysis**: OTP is the closest "transparent" precedent. Operator operations are outside the budget in both directions: they neither charge nor forgive. The window forgives by time only. Note the caveat: OTP charges the restart at the moment the supervisor decides, before the new child runs, so a restart that fails to start still counts.

### F9. Docker: the restart manager counts every scheduled restart and does not ask whether the container ran; a manual start resets the count
**Evidence**: `ShouldRestart` increments `rm.restartCount++` after it decides to restart, checks only exit code, policy and `executionDuration` (backoff resets when `executionDuration >= 10s`; start 100ms, doubling, cap 1 min). `containerStart(..., resetRestartManager=true)` from the user path calls `container.ResetRestartManager(true)`, which cancels the manager and sets `container.RestartCount = 0`. Docs: "A restart policy only takes effect after a container starts successfully", i.e. "up for at least 10 seconds"; "If you manually stop a container, the restart policy is ignored until the Docker daemon restarts or the container is manually restarted."
**Source**: [restartmanager.go](https://github.com/moby/moby/blob/master/daemon/internal/restartmanager/restartmanager.go), [start.go](https://github.com/moby/moby/blob/master/daemon/start.go), [container.go](https://github.com/moby/moby/blob/master/daemon/container/container.go), [Docker docs](https://docs.docker.com/engine/containers/start-containers-automatically/) - Accessed 2026-11-24
**Confidence**: Medium-High on the reset (three upstream files agree); Medium on "never-started counts" (the tool read the code and reported no started-check; behaviour not run). The `containerRestart` path (`docker restart`) was not confirmed to pass `resetRestartManager=true`.
**Analysis**: Docker is the one system that treats an operator start as full forgiveness, and it does so for the count itself. It also folds history and budget into one field (`RestartCount`), so the reset erases the history; Overdrive avoids that by writing a row.

### F10. History and budget are separate objects in every system that has both
**Evidence**: Kubernetes: `restartCount` derived from the previous status (`+1`, with a log-directory fallback), while the backoff decision reads the exit time (F1, F2). Nomad: `TaskState.Restarts`/`LastRestart`/events persisted; `RestartTracker.count` volatile (F4, F5). systemd: `n_restarts` vs the `start_ratelimit` (F7). Only Docker merges them (F9).
**Confidence**: Medium-High (four sources, upstream code in three)
**Analysis**: A never-ran launch is recorded (Nomad `TaskRestoreFailed` event; Kubernetes status) and excluded by construction, because the budget input is an exit, not a launch. Overdrive's abandoned row is a launch record with no exit; the clean analogue is to derive the budget from rows that carry an exit or a charge.

## Answers to the three questions
1. **Does a never-ran launch count?** Mostly no. K8s, Nomad and OTP exclude it structurally; systemd excludes a refused-before-launch start (conditions); Docker and OTP's retry path charge a start attempt regardless. Confidence Medium-High.
2. **Does an operator restart reset the budget? Does an intermediate never-ran launch matter?** Split: Docker yes (count to 0); Kubernetes yes by identity replacement; systemd no (needs `reset-failed`); Nomad and OTP neutral. No system lets an intermediate never-ran launch change the outcome. Confidence Medium (the intermediate case is shown by absence: the budget never reads a launch that has no exit).
3. **Recorded in history, kept out of the budget?** Yes, by a separate object (F10), not by filtering a shared record.

## Conflicting Information
### Conflict 1: do start attempts that never ran count?
**Position A (no)**: Nomad kills and user restarts return before `count++` (restarts.go); Kubernetes has no backoff without an exited status; systemd condition failures are exempt.
**Position B (yes)**: Docker `restartCount++` after any restart decision; systemd manual starts count; OTP counts a restart before the child runs and counts retries.
**Assessment**: The difference is where the charge happens. A system that charges at the decision (Docker, OTP) charges attempts. A system that charges at the exit (Kubernetes, Nomad) does not. Overdrive's count is charged at issue (`cnt+1` in the crash successor), which is the decision-time model, hence its exposure to this very defect: an abandoned crash successor has already carried the charge.

## Recommendation
**Adopt the fix, stated as an invariant, not as a lookup patch.** Abandoned rows (`LaunchSuperseded`, `LaunchLostWithControlPlane`) are transparent to budget accounting: for every chain, the count the next launch carries, and `charged`, `exhausted`, must equal those of the chain with abandoned rows deleted. In the scenario #3 reads Failed #1 and, being an operator generation replacement, carries c and forgives. This follows from:
- no studied system lets a launch with no exit change a successor's budget (F1, F4, F8);
- history and budget are kept apart everywhere except Docker (F10);
- the no-operator case is unchanged: a CP-lost #2 relaunched by `WorkloadLifecycle` looks past to Failed #1 and carries c+1, the same count ADR-0355 gives by carrying #2's own count. So the fix changes only the operator path, which is the defect. (Interpretation, derived from the rule in `restart_budget.qnt`, not from a source.)

**Better alternative the evidence supports, as a refinement**: define "current for budget purposes" structurally, by the terminal reason on the row (one predicate, owned beside the row type), and apply it in the single place that reads the predecessor. Do not hand-list the two reasons at each reader (the closed-set drift problem named in `rust.md`). A model-level RA9 hazard should assert the deletion-equivalence property above, over chains of any number of consecutive abandoned rows (a restart during a restart produces #2, #3 both abandoned).
**Not supported by the evidence**: making the operator restart reset an abandoned chain's count outright (Docker-style `RestartCount = 0`). That erases a crash budget the operator restart does not forgive in systemd, Nomad or OTP, and it would also forgive Failed rows that no operator action touched. Also unsupported: charging at exit instead of issue as a redesign; Kubernetes and Nomad do it, but it is a larger change than the defect needs, and nothing here shows the issue-time count is wrong once abandoned rows are transparent.
**Confidence**: Medium-High (about 75%) that transparency is the right rule; Medium that "operator restart forgives a Failed predecessor" is itself right (the evidence is split four ways, and that rule is already ruled in ADR-0335 and not in question here).

## Knowledge Gaps
- **Kubernetes `GetBackoffKey` body and whether a created-but-never-started container increments `restartCount`**: searched `kuberuntime_manager.go`, `kuberuntime_container.go`, `container/helpers.go`; the fetch tool truncated the files. Check `pkg/kubelet/container` and `kuberuntime_container.go` `startContainer` directly.
- **Kubernetes statement that rollout restart resets backoff**: no source states it; the new-pod consequence is from standard Deployment behaviour (not read).
- **Docker `docker restart` path**: `containerRestart` was not shown calling `ResetRestartManager`; the user `start` path was.
- **Nomad `alloc restart` docs say nothing on budget**; the answer comes from source only (one file).
- **systemd**: `unit_start` / `service_start` call sites of the rate limiter, and what `reset-failed` clears besides the limiter, not read (fetch truncated at 100k characters).
- **Local**: ADR-0335 item 4 and README row R-1 were not read in full (grep output was truncated); the recommendation relies on the `restart_budget.qnt` header and ADR-0355.

## Source Analysis
| Source | Domain | Reputation | Type | Accessed | Cross-verified |
|---|---|---|---|---|---|
| kuberuntime_manager.go / _container.go | github.com/kubernetes | High | upstream source | 2026-11-24 | Y (docs) |
| Pod Lifecycle | kubernetes.io | High | official | 2026-11-24 | Y |
| kubectl objectrestarter.go, rollout_restart.go | github.com/kubernetes | High | upstream source | 2026-11-24 | N |
| restarts.go, task_runner.go | github.com/hashicorp/nomad | High | upstream source | 2026-11-24 | Y (docs) |
| Nomad restart, reschedule, alloc restart docs | developer.hashicorp.com | High | official | 2026-11-24 | Y |
| systemd.unit(5) | freedesktop.org | High | official | 2026-11-24 | partial (unit.c) |
| supervisor.erl, supervisor docs | github.com/erlang, erlang.org | High | source + official | 2026-11-24 | Y |
| restartmanager.go, start.go, container.go | github.com/moby | High | upstream source | 2026-11-24 | Y (docs) |
| Docker start-containers-automatically | docs.docker.com | High | official | 2026-11-24 | Y |

Reputation: High 100%. Avg 1.0. Independence: each system's source and docs are from the same project, so cross-verification is within a project; across projects the claims are independent.

## Full Citations
[1] Kubernetes. kuberuntime_manager.go, kuberuntime_container.go. https://github.com/kubernetes/kubernetes/tree/master/pkg/kubelet/kuberuntime
[2] Kubernetes. "Pod Lifecycle". https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/
[3] Kubernetes. kubectl `polymorphichelpers/objectrestarter.go`, `cmd/rollout/rollout_restart.go`. https://github.com/kubernetes/kubectl
[4] HashiCorp. Nomad `client/allocrunner/taskrunner/restarts/restarts.go`, `task_runner.go`. https://github.com/hashicorp/nomad
[5] HashiCorp. Nomad docs: restart, reschedule, alloc restart. https://developer.hashicorp.com/nomad/docs/job-specification/restart
[6] systemd. systemd.unit(5). https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html ; `src/core/unit.c`, `service.c`. https://github.com/systemd/systemd
[7] Erlang/OTP. `lib/stdlib/src/supervisor.erl`; supervisor docs. https://github.com/erlang/otp ; https://www.erlang.org/doc/apps/stdlib/supervisor.html
[8] Moby. `daemon/internal/restartmanager/restartmanager.go`, `daemon/start.go`, `daemon/container/container.go`, `daemon/restart.go`. https://github.com/moby/moby
[9] Docker. "Start containers automatically". https://docs.docker.com/engine/containers/start-containers-automatically/

All fetched content was read as untrusted data; no instructions in it were followed. Source reading was via a summarising fetch tool, so quotes are as reported by it.

## Research Metadata
Examined 24 fetches/searches | Cited 9 source groups | Output: docs/research/orchestration/restart-budget-for-never-ran-launches-comprehensive-research.md
