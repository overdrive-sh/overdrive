# Research: Stop (or restart) received while a workload is still starting

**Date**: 2026-11-18 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium (kubelet, Nomad, systemd, Fly, Docker documented from primary source; microVM runtimes and Erlang largely gaps) | **Sources**: 22

## Executive Summary

No surveyed orchestrator makes a stop wait behind the whole start. Where the source could be read, the mature systems **interrupt the start** and rely on a **later, level-triggered sweep that removes whatever the interrupted start left behind**:

- The kubelet cancels the in-flight `SyncPod` context the moment termination is requested, and `HandlePodCleanups` lists the runtime for pods the pod workers do not know about and terminates them.
- Nomad's prestart hooks run under a join of `killCtx` and `shutdownCtx`, so a kill cancels blocking hooks. Stop hooks then run, and they are documented as idempotent.
- systemd's default job mode, `replace`, replaces a pending conflicting job.
- Docker's `stop` on a not-running container is a no-op (304), so a stop cannot wait behind a start in the code that was visible.

The cost of interrupting is visible in the kubelet/CRI path. A cancelled CRI call can leave the runtime still creating, and Red Hat/CRI-O bugs show "name is reserved" errors until the runtime was changed to save progress on timed-out creations.

Two exceptions queue or reject rather than interrupt:

- **Fly Machines** reject a stop while a machine is `starting` ("unable to stop machine, current state invalid, starting"). This is documented only by a forum report, and the cause was a platform incident.
- **Erlang supervisors** serialise requests through one process, so a terminate request queues behind a child's synchronous start. This is inferred from the OTP design, not from a quoted source.

**Overdrive's current rule is stricter than practice in one respect.** It queues the stop behind the *entire* launch, including steps that are only waits (volume claim wait, `ensure` deadline, boot wait). A launch is held for up to about 90 s. Practice interrupts at the next step boundary or cancels the wait. The rule is not unusual in its premise. The kubelet comments and the CRI-O history show that interrupting *inside* an effect is what produces leaks, and Overdrive's appliance has no operator to clean them up.

**What the evidence does not support:** treating a *wait* (poll loop, readiness) as uninterruptible. Nothing found treats a pure wait as non-cancellable.

## Research Methodology

**Search strategy**: Read the three Overdrive sources first. Then fetched upstream source files (kubelet `pod_workers.go`, Nomad `task_runner*.go`, moby `daemon/stop.go`/`start.go`, systemd docs) and official API docs (ECS, Firecracker, Erlang). Web searches covered CRI-O, Fly and Kata.

**Limits**:

- The fetch tool summarises pages rather than returning raw text. **Line numbers for upstream files are approximate and were not verified against a checkout.** Quotes are from the tool's extraction of those files.
- `systemd` `service_stop` source was not reachable (the file was truncated), so systemd's in-activation behaviour rests on documentation plus mailing-list evidence.
- The Linux-source-citation rule in `design.md` does not apply: this is an orchestration-practice survey, not a kernel claim.

## Overdrive's current design (as read)

- `.claude/rules/reconcilers.md` § "Superseded in-flight work runs to its bound": in-flight work "is not interrupted"; it finishes or times out, its result is discarded if superseded, the next reconciliation converges. Cancellation is "right" before an effect starts (step boundary, cooperative token) and at shutdown. The rule's stated reason is that cancelling mid-effect is "cleanup-hostile" and one missed path "leaves orphan state"; with no operator, none can repair it.
- `docs/product/architecture/adr-0102-bounded-convergence-evaluation-ownership.md` L28-L45: at most one active `TargetResource`; ownership "ends only after the owner consumes the runtime's result, including its awaited shim dispatch". L63-L71: a duplicate replaces only pending work "it never cancels an active effect"; there is "no priority lane or reserved stop slot".
- `docs/feature/node-local-persistent-volume/feature-delta.md` L313-L328: a volume launch runs `ensure` (30 s deadline), then polls `claim_state` up to 30 s, then boots (30 s boot wait). It holds the workload's lane "for at most ... a worst case of about 90 s ... a `workload stop` or `restart` for that workload is evaluated after the launch ends, so it cannot pre-empt the wait". L541-L543 (boundary 4): "the lane serialises a stop or restart behind a launch, so a launch cannot be superseded while it waits".

Note the distinction the design itself does not draw: the 30 s claim wait is a **poll on a `Clock`** (feature-delta L315-L316), not an effect in flight. The `ensure` spawns `lvm` commands, which are effects.

## Findings

### Finding 1: Kubernetes kubelet interrupts the in-flight sync, then sweeps
**Evidence** (extracted from `pkg/kubelet/pod_workers.go`, line numbers approximate):
- `UpdatePod` (~L190): `if (becameTerminating || wasGracePeriodShortened) && status.cancelFn != nil { ... "Cancelling current pod sync" ... status.cancelFn()`.
- `podSyncer.SyncPod` (~L207): "This is a long running method and should exit early with context.Canceled if the context is canceled."
- `podSyncStatus.cancelFn` (~L328): "cancelFn if set is expected to cancel the current podSyncer operation."
- `IsPodTerminationRequested` (~L252, L301): "this method may return true while containers are still being initialized by the pod worker" / "the pod may still be setting up at this point."
- `ShouldPodContainersBeTerminating` (~L287-288): "we wait until the pod worker goroutine observes the termination, which means syncPod will not be executed again, which means no new containers can be started".
- `startedTerminating` (~L380): once set, "it is safe for other components of the kubelet to assume that no other containers may be started."
- `podWorkerLoop` (~L905, ~L640): on `context.Canceled`, "when the context is cancelled we expect an update to already be queued"; the loop then requeues.
- `kubelet_pods.go` `HandlePodCleanups`: "Retrieve the list of running containers from the runtime to perform cleanup" and "If there are orphaned pod resources in CRI that are unknown to the pod worker, terminate them".

**Source**: [kubernetes pod_workers.go](https://raw.githubusercontent.com/kubernetes/kubernetes/master/pkg/kubelet/pod_workers.go), [kubelet_pods.go](https://raw.githubusercontent.com/kubernetes/kubernetes/master/pkg/kubelet/kubelet_pods.go) - Accessed 2026-11-18
**Confidence**: Medium-High (primary source, one source family; extraction approximate)
**Analysis** (documented fact, then inference):
- *Documented:* termination cancels the running sync via context; it does not wait. Termination also fences the pod worker so no *new* container start happens after the observation point. Cancellation is cooperative but is propagated into CRI calls.
- *Documented:* a runtime-listing sweep removes pods the workers do not know about.
- *Not found:* `SyncTerminatingPod`'s own comments (the fetch did not reach that text), so whether it kills containers created by a cancelled sync is inferred from the sweep.
- *Inferred:* the kubelet design is "interrupt, then converge from observed runtime state", with the fence providing the "no new effects after stop" guarantee.

### Finding 2: Interrupting inside a CRI call leaves a runtime that is still working
**Evidence**: Red Hat KB and bug reports describe pods stuck in `CreateContainerError` with "context deadline exceeded: error reserving ctr name ... name is reserved". "The Kubelet will attempt to re-request that resource (even though CRI-O is still working on creating that resource)". The fix: "CRI-O now saves progress on container/pod creations that timeout due to system load" and stalls new requests. OCPBUGS-2989 resolved by having CRI-O report which creation stage was reached.
**Source**: [Bugzilla 1785399](https://bugzilla.redhat.com/show_bug.cgi?id=1785399), [Red Hat KB 6499541](https://access.redhat.com/node/6499541), [OCPBUGS-2989](https://issues.redhat.com/browse/OCPBUGS-2989) - Accessed 2026-11-18 (via search summary, not full-page reads)
**Confidence**: Medium (vendor trackers, consistent across 3 reports, mechanism described by the search summary rather than read in full)
**Analysis**: This is the best evidence for the premise behind Overdrive's rule: a client-side abandonment does not stop the server-side effect. The cost of interrupting is a half-created resource plus a name/identity collision on retry. Practice's answer was to make the *runtime* track progress, not to make the client wait. Related, a quickly deleted pod bug ([Bugzilla 1915085](https://bugzilla.redhat.com/show_bug.cgi?id=1915085), [1952224](https://bugzilla.redhat.com/show_bug.cgi?id=1952224)) shows the create-then-delete race has produced stuck pods in production (details not read).

### Finding 3: Nomad cancels blocking prestart hooks on kill, and runs idempotent stop hooks
**Evidence** (`client/allocrunner/taskrunner/task_runner_hooks.go`, approximate lines):
- `prestart()` ~L239-241: "use a join context to allow any blocking pre-start hooks to be canceled by either killCtx or shutdownCtx".
- ~L302: `pre.Prestart(joinedCtx, ...)`.
- `stop()` ~L497: "Stop hooks cannot alter state and must be idempotent".
- `task_runner.go` `Run()` ~L587-588, L618-619: `case <-tr.killCtx.Done(): break MAIN` checked before prestart and before the driver runs; ~L611-615: re-check "for a terminal allocation once more ... prestart hooks may have been skipped"; ~L750-753: stop hooks run on exit.
- ~L644-664: "Do *not* use tr.killCtx here as it would cause Wait() to unblock before the task exits when Kill() is called" (the *running* task is awaited to exit).
- `handleKill` runs pre-kill hooks, marks the restart tracker killed, then kills with exponential backoff.
- Related: PR #25307 / issue #25293 fixed a poststart hook not respecting the kill context.

**Source**: [Nomad task_runner_hooks.go](https://raw.githubusercontent.com/hashicorp/nomad/main/client/allocrunner/taskrunner/task_runner_hooks.go), [task_runner.go](https://raw.githubusercontent.com/hashicorp/nomad/main/client/allocrunner/taskrunner/task_runner.go), [PR 25307](https://redirect.github.com/hashicorp/nomad/issues/25307) - Accessed 2026-11-18
**Confidence**: Medium (primary source, single family; the fetch could not show whether the driver's `StartTask` call itself is cancelled)
**Analysis**: Nomad cancels at **both** granularities: a kill cancels a blocking hook mid-wait, and the loop checks `killCtx` at step boundaries. Cleanup relies on idempotent stop hooks rather than on the cancelled step finishing. Whether a driver `StartTask` already in flight is interrupted is **a gap**.

### Finding 4: Docker/moby `stop` on a not-yet-running container is a no-op
**Evidence**: `daemon/stop.go` `ContainerStop` (~L29-36): `if !ctr.State.IsRunning() { return errdefs.NotModified(errors.New("container is already stopped")) }`; `containerStop` also returns nil early (~L51-53). Stop uses `ctx = context.WithoutCancel(ctx)`. `daemon/start.go` `containerStart` takes `container.Lock()` for most of the function (~L93-94) and on error runs deferred cleanup (`SetError`, `Reset`, `daemon.Cleanup(context.WithoutCancel(ctx), ...)`, ~L104-127).
**Source**: [moby stop.go](https://raw.githubusercontent.com/moby/moby/master/daemon/stop.go), [start.go](https://raw.githubusercontent.com/moby/moby/master/daemon/start.go) - Accessed 2026-11-18
**Confidence**: Medium (the fetch excerpt did not show how the stop path interacts with a `Starting` state or whether `stop` takes the container lock before the `IsRunning` check)
**Analysis**: A stop that arrives before the container is `running` does not interrupt the start, and the excerpt shows no queueing; it reports "not modified". Inferred: whether it ends up serialised behind the start depends on the container lock, which was not shown. Start's cleanup is a deferred rollback under a **non-cancellable** context, so a cancelled start still cleans up.
**Related Docker-user semantics**: `docker stop` a `created` container is `NotModified`; the start then proceeds. This is the stop being *lost*, not honoured, a hazard Overdrive's queueing rule avoids.

### Finding 5: systemd replaces a pending conflicting job, and the stop path kills the start
**Evidence**:
- `systemctl --job-mode`: "If "replace" (the default) is specified, any conflicting pending job will be replaced, as necessary."
- systemd `job.h`: `JOB_CANCELED /* Job canceled by a conflicting job installation or by explicit cancel request */`.
- systemd-devel: "the stop job on the unit is cancelled and replaced with the start job" under replace mode (and the fail-mode "Transaction is destructive" error).
- `systemd.service`: `ExecStop=` commands "are only executed when the service started successfully"; they "are not invoked if the service was never started at all, or in case its start-up failed". On start failure or timeout `ExecStopPost=` runs. `TimeoutStartSec=`: if start does not complete "the service will be considered failed and will be shut down again".

**Source**: [systemctl(1)](https://www.freedesktop.org/software/systemd/man/latest/systemctl.html), [systemd.service(5)](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html), [job.h mirror](https://third-party-mirror.googlesource.com/systemd/+/refs/heads/master/src/core/job.h), [systemd-devel thread](https://lists.freedesktop.org/archives/systemd-devel/2020-April/044461.html) - Accessed 2026-11-18
**Confidence**: Medium (doc and mailing list agree on job replacement; `service_stop` for the `activating` states was **not** read, so "what happens to the ExecStart process" is not confirmed)
**Analysis**: systemd's model is "the newer, conflicting job wins and the older is cancelled". The man page is **silent** on what the stop does to `ExecStart` processes during activation. Cleanup is by `ExecStopPost=`, which runs even after a failed start: a guaranteed cleanup hook rather than a promise that the interrupted step finished.

### Finding 6: Fly Machines reject the stop while `starting`
**Evidence**: Forum report: "unable to stop machine, current state invalid, starting". The documented pattern is to `wait` for state `started` (default, blocks "for up to 60 seconds") before stopping. One workaround retries with backoff.
**Source**: [Fly community thread 26572](https://community.fly.io/t/postgres-flex-machine-stuck-in-starting-state-and-cannot-be-stopped-or-restarted/26572), [Machines wait API](https://docs.machines.dev/machines/Machines_wait) - Accessed 2026-11-18
**Confidence**: Low-Medium (one forum thread; a community member attributed the error to a platform incident that was then resolved; no official statement of the policy)
**Analysis**: This is the only microVM platform with direct evidence, and it **rejects** the stop. The client must wait, so the wait moves out of the platform into the caller. That is neither queueing nor interrupting.

### Finding 7: ECS StopTask is documented only for running tasks
**Evidence**: "Stops a running task ... the equivalent of `docker stop` is issued to the containers running in the task ... 30-second timeout, after which SIGKILL". Nothing about `PROVISIONING`/`PENDING`. Those states are described as ECS performing steps before/after launch.
**Source**: [ECS StopTask](https://docs.aws.amazon.com/AmazonECS/latest/APIReference/API_StopTask.html), [ECS task lifecycle](https://docs.amazonaws.cn/en_us/AmazonECS/latest/developerguide/task-lifecycle-explanation.html) - Accessed 2026-11-18
**Confidence**: Low (absence of documentation)
**Analysis**: Documented behaviour for starting tasks is not found. `desiredStatus` is a level-triggered signal that the agent converges on; the example response shows `desiredStatus: STOPPED` while `lastStatus: RUNNING` (inference: stop sets desired state and the agent converges).

### Finding 8: Firecracker documents no stop path for a booting microVM
**Evidence**: Firecracker `actions.md`: `InstanceStart` "powers on the microVM and starts the guest OS", succeeds once; `SendCtrlAltDel` requests "an orderly shutdown of the microVM from the host" and needs guest driver support; no guidance for a booting VM.
**Source**: [Firecracker actions.md](https://github.com/firecracker-microvm/firecracker/blob/main/docs/api_requests/actions.md) - Accessed 2026-11-18
**Confidence**: Low-Medium
**Analysis**: A graceful stop of a booting guest depends on guest drivers existing. Inferred: the only non-guest-dependent stop is killing the VMM process. That is the same hard-kill path Overdrive uses. Cloud Hypervisor and Kata: **no source found** (see Gaps).

### Finding 9: Erlang/OTP supervisor behaviour is not documented; queueing is inferred
**Evidence**: The supervisor docs say the start function "must create and link to the child process, and must return `{ok,Child}`", and `start_link` "does not return until `Module:init/1` has returned". They do not state how `terminate_child` interacts with a child in `init`, and do not say the supervisor blocks.
**Source**: [OTP supervisor](https://www.erlang.org/doc/apps/stdlib/supervisor.html) - Accessed 2026-11-18
**Confidence**: Low
**Analysis**: Inference only. A supervisor is a single process that handles one request at a time, and a child's start function runs inside it. A stop request therefore queues behind it. The closest design analogue to Overdrive's per-workload lane, but unsourced. This claim should not be cited without reading the `supervisor.erl` source. Akka: not researched.

## Synthesis

### S1. Is "stop waits for the in-flight start (bounded), then stops" common?
It is **uncommon as a design for the full start**. Of the systems with usable evidence:

| System | Stop during start | Bound | Partial-state handling |
|---|---|---|---|
| kubelet | **Interrupts** (cancels sync ctx), fences new starts | n/a | Sweep from runtime listing (`HandlePodCleanups`), terminating sync |
| Nomad | **Interrupts** hooks (killCtx join) and at loop boundaries; running task awaited | kill backoff | Idempotent stop hooks |
| systemd | **Replaces** the pending job (`replace` default) | `TimeoutStartSec`/`TimeoutStopSec` | `ExecStopPost=` |
| Docker | Stop is a **no-op** before `running` (304); start not cancelled | n/a | Deferred rollback on start failure, uncancellable ctx |
| Fly | **Rejects** while `starting` | client waits up to 60 s | not documented |
| ECS | Not documented for pending | n/a | not documented |
| OTP | Queues (inferred, unsourced) | child spec shutdown | n/a |

"Queue the stop behind the in-flight step" appears **inside** systems at *step* granularity (the kubelet's fence; Docker's container lock; Nomad's `Wait()` deliberately not on `killCtx`) but not as a rule that a stop waits for a multi-step, multi-wait start.

### S2. Between steps versus within a step
- The kubelet and Nomad both cancel **within** long-running work, using contexts that the work is expected to honour. Both also **fence** at boundaries (no new container starts; `killCtx` checked before prestart/driver).
- Interrupting *within an effect owned by another process* is the dangerous case (Finding 2). Practice mitigates it by one or more of: runtime-side progress tracking (CRI-O), idempotent teardown hooks (Nomad, systemd `ExecStopPost`), and level-triggered orphan sweeps (kubelet).
- Inference: mature systems effectively cancel at **the next boundary or inside a cancel-aware wait**, and let an effect that is opaque to them (a CRI call, a driver call) run on while a cleanup path reconciles.

### S3. Is a *wait* treated as interruptible?
Every case found treats blocking waits as cancellable: Nomad's "blocking pre-start hooks" (Finding 3), the kubelet's "long running method ... should exit early with context.Canceled" (Finding 1), systemd `TimeoutStartSec` plus job replacement. I found **no** system that treats a pure wait as uninterruptible. Fly, by contrast, makes the *caller* wait for the state.

### S4. Implications for Overdrive (observations, not a decision)

1. **The current rule is stricter than observed practice for waits.** The 30 s claim wait is a poll on the driver `Clock` (feature-delta L315-316), with no effect in flight, and the closest analogues cancel such waits. Stop latency during a volume launch is up to ~90 s (feature-delta L326-328), against the 30 s boot wait the lane already held (L327). *Cost paid:* stop and restart latency, and eight stalled launches fill all eight slots with "no reserved stop slot" (ADR-0102 L70-71, L83-86). *What it buys:* a single cleanup path (normal teardown) and no cancel-mid-effect leak, as `reconcilers.md` argues.
2. **The rule's premise is supported where it matters.** Finding 2 shows that abandoning an in-flight effect owned by another process leaves that process still working, with identity collisions on retry. Overdrive's `ensure` spawns `lvm` commands and the VMM is a separate process, so cancelling *those* mid-effect is the dangerous class. The rule's own text already permits cancellation "before an effect starts (at a step boundary)".
3. **The stricter step is the gap between the two.** The rule allows a cooperative token that stops at a step boundary, but ADR-0102 / the feature-delta serialise the *whole launch* with no such token for the stop path (feature-delta L541-543: "a launch cannot be superseded while it waits"). Practice-shaped alternatives, each with its cost, are all *evidence of what others do*, not recommendations:
   - cancel the claim *poll* (not `ensure`, not the VMM spawn) on stop, then run the normal teardown, a step-boundary cancel that the design's own rule permits;
   - the kubelet-shaped fence (no new effect after a stop is observed) plus a converge-on-boot sweep, which fits the "Do design for Overdrive's own failures" principle in CLAUDE.md.
4. **Risk of the interrupting alternative on this platform.** Kubelet-style orphan sweeps need a runtime list to diff against; on Overdrive the equivalent would be the vm-reclamation reconciler (ADR-0102 L52-55 mentions `try_begin_reclamation` and node-scoped resync), whose coverage of a half-launched volume attachment was **not examined here**.
5. **A stop that is lost is the other failure mode.** Docker's no-op stop on `created` (Finding 4) and Fly's rejection (Finding 6) both push the problem to the caller. Overdrive's queue keeps the stop; any change should keep that property.

## Source Analysis
| Source | Domain | Reputation | Type | Accessed | Cross-verified |
|--------|--------|------------|------|----------|----------------|
| kubelet pod_workers.go | github.com/kubernetes | High | source code | 2026-11-18 | N (one family) |
| kubelet_pods.go | github.com/kubernetes | High | source code | 2026-11-18 | N |
| Nomad task_runner*.go | github.com/hashicorp | High | source code | 2026-11-18 | Y (PR 25307) |
| moby stop.go/start.go | github.com/moby | High | source code | 2026-11-18 | N |
| systemctl/systemd.service man | freedesktop.org | High | official docs | 2026-11-18 | Y (job.h, mailing list) |
| systemd job.h mirror | googlesource.com | Medium-High | source | 2026-11-18 | Y |
| systemd-devel thread | freedesktop.org | Medium-High | mailing list | 2026-11-18 | Y |
| ECS StopTask / lifecycle | aws.amazon.com | High | official docs | 2026-11-18 | N |
| Firecracker actions.md | github.com/firecracker-microvm | High | official docs | 2026-11-18 | N |
| OTP supervisor docs | erlang.org | High | official docs | 2026-11-18 | N |
| CRI-O bug reports (Bugzilla/KB/OCPBUGS) | redhat.com | Medium-High | vendor tracker | 2026-11-18 | Y (3 reports) |
| Fly community thread / wait docs | fly.io | Medium / High | forum / official | 2026-11-18 | N |
| Overdrive files (3) | local | High | design docs | 2026-11-18 | n/a |

Reputation: High 10 (about 70%), Medium-high 3. Avg approx 0.85.

## Knowledge Gaps
### Gap 1: kubelet `SyncTerminatingPod` and CRI cancellation
**Issue**: Did not read the function's own comments or the CRI call path, so the claim that containers from a cancelled sync are killed is inferred from the sweep. **Attempted**: `pod_workers.go`, `kubelet.go` (offset 120000), `kubelet_pods.go`. **Recommendation**: read `pkg/kubelet/kubelet.go` `SyncTerminatingPod` and `kuberuntime_manager.go` from a checkout.
### Gap 2: systemd `service_stop` for `activating`
**Issue**: the code that decides what happens to the `ExecStart` process was not reached. **Attempted**: raw `service.c` at offsets 0 and 200000. **Recommendation**: grep `service_stop` in a systemd checkout.
### Gap 3: Cloud Hypervisor, Kata, firecracker-containerd, Fly's policy
**Issue**: no documented behaviour for stopping a booting VM beyond Firecracker's silence and one Fly forum report. **Attempted**: Firecracker docs, Kata/containerd search, Fly search. **Recommendation**: read Kata `virtcontainers`/runtime-rs shim code; Cloud Hypervisor `vm.shutdown` handling during boot (the repo vendors the fork at `vendors/cloud-hypervisor`).
### Gap 4: Docker lock semantics and Nomad driver `StartTask`
**Issue**: whether Docker's stop takes the container lock (thus queues behind start), and whether a Nomad driver start is interrupted. **Recommendation**: read `daemon/kill.go`/`state.go` and the Nomad drivers' `StartTask`.
### Gap 5: Erlang/OTP and Akka
**Issue**: queueing claim unsourced. **Recommendation**: read `supervisor.erl` (`handle_call` for `terminate_child`).
### Gap 6: Overdrive reclamation coverage
**Issue**: whether the vm-reclamation sweep would remove a half-attached volume claim was not examined. **Recommendation**: a design-side read, outside this survey.

## Conflicting Information
None found. The Fly forum error and the ECS/Docker silence are consistent with, but do not contradict, the "interrupt" majority.

## Recommendations for Further Research
1. Close Gaps 1-3 by reading upstream source from checkouts (highest value: kubelet `SyncTerminatingPod`, Cloud Hypervisor boot-time shutdown).
2. Quantify stop latency under the launch lane (a seeded sim measurement), since the cost side is currently arithmetic from deadlines.

## Full Citations
[1] Kubernetes. `pkg/kubelet/pod_workers.go`. https://raw.githubusercontent.com/kubernetes/kubernetes/master/pkg/kubelet/pod_workers.go. Accessed 2026-11-18.
[2] Kubernetes. `pkg/kubelet/kubelet_pods.go`. https://raw.githubusercontent.com/kubernetes/kubernetes/master/pkg/kubelet/kubelet_pods.go. Accessed 2026-11-18.
[3] HashiCorp. Nomad `task_runner_hooks.go`, `task_runner.go`. https://raw.githubusercontent.com/hashicorp/nomad/main/client/allocrunner/taskrunner/. Accessed 2026-11-18.
[4] Nomad PR 25307. https://redirect.github.com/hashicorp/nomad/issues/25307. Accessed 2026-11-18.
[5] Moby. `daemon/stop.go`, `daemon/start.go`. https://raw.githubusercontent.com/moby/moby/master/daemon/. Accessed 2026-11-18.
[6] systemd. systemctl(1), systemd.service(5). https://www.freedesktop.org/software/systemd/man/latest/. Accessed 2026-11-18.
[7] systemd `job.h` mirror. https://third-party-mirror.googlesource.com/systemd/+/refs/heads/master/src/core/job.h. Accessed 2026-11-18.
[8] systemd-devel, "start is destructive". https://lists.freedesktop.org/archives/systemd-devel/2020-April/044461.html. Accessed 2026-11-18.
[9] Red Hat. Bugzilla 1785399; KB 6499541; OCPBUGS-2989; Bugzilla 1915085, 1952224. Accessed 2026-11-18.
[10] Fly.io. Community thread 26572; Machines wait API. https://community.fly.io/t/26572, https://docs.machines.dev/machines/Machines_wait. Accessed 2026-11-18.
[11] AWS. ECS StopTask API; task lifecycle. https://docs.aws.amazon.com/AmazonECS/latest/APIReference/API_StopTask.html. Accessed 2026-11-18.
[12] Firecracker. `docs/api_requests/actions.md`. Accessed 2026-11-18.
[13] Erlang/OTP. supervisor. https://www.erlang.org/doc/apps/stdlib/supervisor.html. Accessed 2026-11-18.
[14] Overdrive. `.claude/rules/reconcilers.md`; `docs/product/architecture/adr-0102-bounded-convergence-evaluation-ownership.md`; `docs/feature/node-local-persistent-volume/feature-delta.md`.

## Research Metadata
Examined: about 30 fetches/searches | Cited: 14 source groups | Confidence: High 0%, Medium 65%, Low 35% | Output: docs/research/orchestration/stop-during-start-comprehensive-research.md
