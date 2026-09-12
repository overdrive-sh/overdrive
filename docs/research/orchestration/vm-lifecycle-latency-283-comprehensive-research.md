# VM lifecycle latency (#283): external technical research

**Status:** research complete

**Scope:** GitHub #283 and its shared lifecycle-owner boundary with #260; external precedents and published benchmark evidence only.

**Research date:** 2026-09-09
**Method:** repository RCA/review and issue context establish the local premise; recommendations below are not implementation changes or validated measurements of Overdrive.

## Executive conclusion

The independently approved investigation establishes three reachable causes: a
single convergence owner awaits every allocation evaluation; `VmDriver::stop`
waits out a two-second request window after its writer has completed; and the
guest PID 1 waits synchronously for the workload before reading `SHUTDOWN`.
This report adds external evidence only. It does **not** change an accepted
architecture, prescribe a Rust signature, or validate a performance result for
Overdrive.

The recommended directions for DESIGN are:

1. Move independent allocation lifecycle work behind a **bounded, keyed
   scheduler**, while retaining one serial reconciliation lane for each
   allocation. The scheduler—not detached work—must own admission,
   cancellation, completion, error delivery, and draining during server
   shutdown, while preserving the existing action-shim versus exit-observer
   terminal arbitration. Kubernetes provides the closest operational precedent:
   work is serial per Pod UID, concurrent across UIDs, and teardown completion
   gates later resource release.
2. Make a healthy stop **completion-driven**. A successful host-side
   `write_all`/`flush` is request submission only; it is not guest receipt,
   workload exit, guest poweroff, VMM exit, or cleanup. Keep a bounded grace
   period only as a failure path. DESIGN must choose the authoritative
   completion condition: the current `Driver::stop` contract establishes only
   a subsequent `status` of `NotFound`, not successful VMM exit plus cleanup.
3. Make guest PID 1 supervise the workload and the control channel
   concurrently. On shutdown it must direct a defined termination signal to
   the defined workload process group, reap children, apply a bounded escalation
   only when necessary, then request guest poweroff. The exact protocol and
   process-tree policy remain DESIGN choices.

The evidence supports a sub-second candidate only for **natural finite-
workload completion**, not a live-Service stop request, and an approximately
two-second initial READY goal for the current profile as *candidates to
validate*, not as claims about the current implementation.
There is no evidence for a universal Service `Stable` budget: health and
stability depend on the workload and must remain a distinct measurement. There
is likewise no recent, directly comparable published distribution for
Cloud-Hypervisor Service shutdown, so this report deliberately does not
manufacture one.

## Evidence classification and local premise

### What this report treats as established local evidence

The following are repository facts at baseline
[`d880ce9`](https://github.com/overdrive-sh/overdrive/tree/d880ce983276e7d4107450b264f6ceab64c70eff),
not new research measurements.

| Item | Established fact | Why it matters here |
|---|---|---|
| Issue scope | [#283](https://github.com/overdrive-sh/overdrive/issues/283) rejects serial lifecycle execution and unexplained seconds of healthy-path latency. [#260](https://github.com/overdrive-sh/overdrive/issues/260) identifies the shared serial allocation-dispatch path, absent lifecycle concurrency limit, and unused tick deadline. Neither issue selects a mechanism. Both issue comment sets were empty when read. | A design has to cover the whole owner-to-driver path, not merely wrap `driver.start` or increase a timeout. |
| Serial owner | The convergence loop awaits each pending evaluation in sequence in [`lib.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-control-plane/src/lib.rs#L3320-L3393), and the action shim serially awaits dispatch in [`action_shim/mod.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-control-plane/src/action_shim/mod.rs#L900-L953). | A driver-only semaphore cannot free an unrelated allocation while its outer evaluation is still awaited. |
| Same-allocation and terminal arbitration | The reconciler runtime persists its view and dispatches/re-enqueues work in [`reconciler_runtime.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-control-plane/src/reconciler_runtime.rs#L1497-L1608). The action shim's stop arm performs cleanup and writes the terminal allocation state, deliberately handling an LWW race with the independent exit observer in [`action_shim/mod.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-control-plane/src/action_shim/mod.rs#L2825-L2963). The exit observer independently publishes actual exits in [`exit_observer.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-control-plane/src/worker/exit_observer.rs#L84-L102). | Parallelism must serialize competing reconciliation chains for one allocation **and** preserve the existing stop-versus-observed-exit LWW arbitration and observer drain; it cannot assume one component is the sole terminal author. |
| Fixed healthy-path delay and current stop postcondition | `VmDriver::stop` completes writer I/O and still awaits the two-second request deadline; it then uses a ten-second VMM termination grace and discards selected termination/cleanup errors in [`vm_driver.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-worker/src/vm_driver.rs#L1651-L1757). `Driver::stop` promises only that `status` returns `NotFound` after `Ok` in [`driver.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-core/src/traits/driver.rs#L802-L817). | The two seconds are not evidence of any completion event. `Ok(())` is not current proof of normal VMM exit or fully successful cleanup. |
| Guest response gap | Guest lifecycle currently receives `EXEC`, synchronously waits for the workload, then reads shutdown and powers off in [`overdrive-init/main.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-init/src/main.rs#L200-L205). | A long-lived Service cannot act on `SHUTDOWN` before it exits. |

The approved [RCA](../../analysis/root-cause-analysis-vm-lifecycle-latency-283.md)
and [review](../../analysis/review-vm-lifecycle-latency-283.md) additionally
record the already-collected native observations used below. This research did
not run, instrument, benchmark, or otherwise measure the repository or host.

### Vocabulary and boundaries

The terms below deliberately do not collapse into one word such as “stopped”.

| Signal or observation | What it establishes | What it does **not** establish |
|---|---|---|
| Control plane admitted a stop | The allocation owner has requested a stop. | That a driver has submitted any guest request. |
| Host writer `write_all` and `flush` return `Ok` | The local writer accepted/completed its requested write and flush operation. | Wire delivery, guest read, guest parsing, workload signal delivery, guest poweroff, VMM exit, or cleanup. The current writer has no acknowledgement path. |
| Guest receipt acknowledgement, if the protocol supplies one | A guest control reader parsed/accepted the request at the defined point. | That it has stopped the workload or powered off. A new acknowledgement must not be assumed without an approved protocol design. |
| Workload child exit/reap | The supervised child has terminated and the PID 1 has collected its status. | That all descendants have exited, unless the chosen process-group/reaping policy proves it. |
| Guest poweroff transition | The guest has requested/entered its architecture-specific poweroff path. Linux documents that a successful poweroff-style `reboot()` does not return; in a PID namespace it terminates namespace init. | That the host has observed the VMM process exit or released host resources. |
| VMM reaper/process exit | The host has observed the VMM process end. | That every driver-owned cgroup, socket, directory, and durable state update has completed. |
| Explicitly verified driver cleanup | A selected cleanup criterion has established that each required driver-owned release operation reached its stipulated outcome. | A separate Service health or `Stable` property. The current `Driver::stop` postcondition alone does not establish this: it specifies a subsequent `status` of `NotFound`, while current `VmDriver::stop` discards selected termination and cleanup errors. |

The Linux `reboot(2)` semantics make the distinction concrete: a successful
poweroff stops the system and does not return, while a PID-namespace poweroff
terminates namespace init and is reported to its parent by `wait(2)`.
[Linux `reboot(2)` man page](https://man7.org/linux/man-pages/man2/reboot.2.html)

## 1. Concurrent lifecycle execution without losing allocation ownership

### What established controllers do

Kubernetes' kubelet is the most applicable mature pattern. In
[`v1.34.0` `pod_workers.go`](https://github.com/kubernetes/kubernetes/blob/v1.34.0/pkg/kubelet/pod_workers.go),
`UpdatePod` is processed FIFO by one goroutine per Pod UID, while the pod
syncer must be safe for simultaneous calls for *different* Pods. A Pod that
starts termination stays in its terminating state; `SyncTerminatingPod` is
repeated with diminishing grace periods and must succeed before other
components may tear down volumes and devices. Its worker tracks pending versus
active state, termination start, termination completion, and a post-termination
notification rather than treating task submission as task completion. This is
direct precedent for keyed ownership, explicit lifecycle state, completion
reporting, and cleanup gating.

The general controller-runtime takes the complementary, capacity-oriented
approach. Its
[`v0.22.0` controller options](https://github.com/kubernetes-sigs/controller-runtime/blob/v0.22.0/pkg/controller/controller.go#L838-L952)
make `MaxConcurrentReconciles` explicit (default one) and provide a
per-reconcile context timeout. It is a useful precedent for a bounded global
admission policy, but by itself does not specify allocation lifecycle ordering.

Cloud Hypervisor illustrates a narrower command/response ownership pattern. Its
[`v53.0` API documentation](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/docs/api.md#L419-L437)
states that the internal MPSC command carries a response sender and that the
command sender creates and waits on that response channel. The same version
exposes distinct `/vm.shutdown` and `/vmm.shutdown` commands, but its API
documentation does not define the REST command response as an end-to-end guest
or host-cleanup acknowledgement. It is therefore useful for “every accepted
command has a result”, not for collapsing request acceptance and VM completion.

### Options for #283/#260

| Option | Preserves same-allocation ordering and ownership | Bounded resource use | Completion/error and server shutdown | Trade-offs |
|---|---|---|---|---|
| **Fixed worker pool over allocation keys** | The scheduler allows at most one active reconciliation evaluation/action chain per allocation key; later state for that key is retained/coalesced until that chain reports completion. It does not replace the independent exit observer or its LWW terminal arbitration with the action shim. | A fixed global worker/permit count limits simultaneous VM provisioning, VMM processes, I/O, and memory pressure. Separate limits for start and stop are possible only if DESIGN explicitly requires different contention control. | Each worker returns a typed success/error/cancellation outcome to the established paths: the reconciler runtime can persist/re-enqueue its view, while the action shim retains its stop cleanup and terminal-write role. On server shutdown: close admission, define scheduler/observer drain ordering, then await owned workers. | Closest to controller-runtime capacity control. Requires a fairness policy, a rule for queued intermediate desired state, and explicit interaction with an independently reporting exit observer. |
| **Per-allocation lifecycle actor plus a global lifecycle permit** | One actor/lane serializes the allocation's reconciliation dispatch chain and acquires a shared permit only around scarce lifecycle work. It must preserve—not replace—the existing reconciler-runtime view, action-shim terminal update, and exit-observer roles. | Actors may be numerous, but active costly operations are capped. | The actor returns outcomes through the existing paths; a supervisor joins actors and drains/joins the observer under a defined server-shutdown order. | Closest to kubelet's keyed worker model. More lifecycle bookkeeping and a clearly defined actor retirement rule are required. |
| **Unkeyed concurrent work queue with an allocation lock** | Correct only if every competing reconciliation start/stop/retry chain consistently takes the same lock **and** the design preserves the independent observer's stop-versus-exit LWW behavior. | Global worker count is simple. | Futures can return outcomes, but lock cancellation, queued duplicates, stale work, and observer interaction become the central correctness risk. | Smallest-looking mechanical change, but weakest expression of the actual allocation owner. It is easy to reintroduce start/stop races or hold a lock across slow I/O. |
| **Spawn-and-forget driver calls** | Does not preserve the current owner/result boundary unless a separate, complete result and shutdown protocol is designed. | May create unbounded VM operations. | Errors can be lost; server shutdown can abandon in-flight cleanup; a later evaluation may observe an indeterminate lifecycle state. | Not a sufficient solution and not supported by the precedents. |

### Required invariants, independent of mechanism

These are behavioral requirements for a design, not an invented API:

1. **One allocation has one active scheduled reconciliation chain.** A new
   desired state for allocation A must not make competing reconciliation
   start/stop/retry chains run concurrently; allocation B remains independently
   eligible for a bounded slot. The scheduler must deliberately coexist with
   the independent exit observer, preserving its stop-versus-exit LWW
   arbitration rather than suppressing, blocking, or losing its publication.
2. **A scheduled operation remains owned until a real result is consumed.**
   Success, driver error, cancellation, join failure, and deadline expiry must
   reach the established consumers: reconciler-runtime persistence/re-enqueue
   and, where applicable, the action shim's cleanup/terminal path. A detached
   task is not a completion model.
3. **No stronger state is published without its evidence.** Current
   `Driver::stop` says only that a later `status` returns `NotFound` after
   `Ok`. Any claim of normal VMM exit or verified cleanup needs separately
   defined evidence; Kubernetes similarly gates downstream teardown on a
   successful terminating sync.
4. **Shutdown has an ordered drain.** Stop admitting new work; preserve the
   record of queued and active allocation lanes; define interaction with the
   exit observer; request cancellation only at an operation boundary whose
   cancellation semantics are defined; wait for owned workers and observer
   drain, or a separate server-shutdown failure deadline; then surface every
   non-completion. Do not use background work to make server shutdown appear
   complete.
5. **Capacity is observable.** Queue depth, active slots, queued duration,
   per-operation outcome, and drain duration are needed to distinguish a
   capacity limit from a driver/guest latency problem.

### Recommendation for the shared #260 scope

Use a keyed allocation scheduler with **bounded global lifecycle admission**.
The concurrency unit should include the currently serial allocation evaluation
through action completion—not merely the innermost VM driver call—because the
outer convergence loop currently awaits the whole chain. Preserve the existing
division of responsibility: reconciler runtime persists its view and
re-enqueues, the action shim performs stop cleanup and terminal LWW updates,
and the exit observer independently reports exits. The scheduler coordinates
the reconciliation lane; it must not silently become the sole terminal author.
This is the combination supported by Kubernetes’ per-key lifecycle worker and
controller-runtime’s explicit concurrency bound.

The exact limit, fairness, coalescing rule, start/stop capacity sharing,
cancellation mechanics, and use of `TickContext.deadline` are unresolved
DESIGN decisions. #260 names several candidate remedies (concurrency limit,
per-action deadline, tick-deadline enforcement); they address different
failure modes and should not be silently conflated. In particular, a deadline
does not remove head-of-line blocking, and concurrency without a capacity limit
does not bound host resource use.

## 2. Completion-driven shutdown and its evidence ladder

### The relevant external behavior

Cloud Hypervisor v53.0 exposes `/vm.shutdown` and `/vmm.shutdown` as distinct
RPC-style actions, and its internal API uses a command-response message with a
sender-owned reply channel.
[Cloud Hypervisor v53.0 API](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/docs/api.md#L239-L280)
That describes a control request being processed by the VMM. It does not
document a guest workload-exit, guest-poweroff, VMM-process-exit, or
host-cleanup completion event. Substituting a VMM request for the current guest
message may choose a different shutdown semantic, but it does not by itself
solve the evidence problem.

Kubernetes makes the same separation at a higher level. Its Pod lifecycle
documentation says that runtime stop is asynchronous, uses a graceful
termination period, and is followed by forceful termination only when the
period expires; the kubelet reports the Pod terminal only after it has ensured
termination. [Kubernetes Pod lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-termination-flow)
The kubelet source makes the cleanup gate stronger: `SyncTerminatingPod` must
return successfully before other components tear down dependent resources.
[Kubernetes v1.34.0](https://github.com/kubernetes/kubernetes/blob/v1.34.0/pkg/kubelet/pod_workers.go#L2651-L2678)

Systemd and minimal init systems provide a second useful distinction: send a
graceful signal now, wait only while the process is actually alive, then use a
bounded forced signal if it remains. `KillMode` can target a process group or
control group and the `TimeoutStopSec` expiry is the threshold for escalation,
not a mandatory healthy-path sleep.
[systemd.kill(5)](https://man7.org/linux/man-pages/man5/systemd.kill.5.html)

### Completion-driven choices

| Direction | Healthy completion condition | Grace/failure condition | Benefit | Limitation or decision required |
|---|---|---|---|---|
| **Request then wait for VMM/reaper exit and explicitly verified cleanup** | The VMM process exit is observed, then a DESIGN-defined verification establishes the required driver-owned cleanup. | If the VMM does not exit before a configured grace, force termination is a failure/escalation path. | Removes the unconditional two-second sleep without falsely asserting guest receipt, while providing a genuine completed-cleanup criterion. | This is stronger than the current `Driver::stop` postcondition, which establishes only later `status == NotFound`; the intended evidence and error model need DESIGN agreement. It cannot distinguish “guest processed SHUTDOWN” from an independent guest/VMM exit unless more protocol evidence exists. |
| **Request, guest acknowledgement, then observe exit and cleanup** | A guest-defined acknowledgement proves request receipt; workload exit/poweroff/VMM exit/cleanup remain separate later events. | No acknowledgement or no exit before the relevant bound becomes a classified timeout/escalation. | Best diagnostics and stage-level latency accounting. | An acknowledgement message, its exact meaning, duplicate/retry behavior, and shutdown race semantics would be new protocol surface and therefore require separate DESIGN approval. It must not be assumed here. |
| **Use a VMM-level shutdown request** | VMM’s documented command-response succeeds, plus separately observed lifecycle completion. | VMM/guest non-completion remains subject to a grace/escalation policy. | May be appropriate where the VMM’s guest power mechanism is the requested semantic. | Cloud Hypervisor documentation distinguishes VM and VMM shutdown, but does not promise that an accepted command equals guest workload completion. It may bypass the guest-level workload policy that #283 needs to define. |
| **Submit request then immediately return/clean up** | Request submission only. | None. | Low apparent latency. | Not a completion-driven solution. It can race live VMM resources and leave action-shim/exit-observer ownership, counters, and cleanup incorrect. |

### Recommendation

Remove the unconditional request-window wait from the healthy path. Drive
healthy completion from a DESIGN-selected authoritative condition; a
conservative candidate is VMM reaper exit plus explicitly verified required
cleanup, while preserving a grace period only to bound an uncooperative guest
or VMM. The code’s VMM helper already distinguishes waiting for a process
outcome from applying a grace and forced termination in
[`vmm.rs`](https://github.com/overdrive-sh/overdrive/blob/d880ce983276e7d4107450b264f6ceab64c70eff/crates/overdrive-host/src/vmm.rs#L534-L574);
`VmDriver::stop` currently discards that termination result and selected
cleanup failures. Thus VMM exit plus verified cleanup is a proposed stronger
criterion, not the current `Driver::stop` guarantee. DESIGN must choose the
condition and error classification before any contract change.

Do not claim guest receipt from `write_all`/`flush`, and do not turn the
two-second request window into a longer “healthy” timeout. A bounded grace is
still necessary for failure containment; it is an error/degradation deadline,
not a performance target. If exact guest receipt has operator or error-model
value, design a protocol acknowledgement separately; do not infer it from
host-side writer success.

## 3. Responsive guest init while supervising a workload

### Linux PID 1 constraints that apply to the guest

The init process in a PID namespace becomes parent to orphaned descendants. If
it exits, the kernel SIGKILLs the other processes in that namespace. Further,
only signals for which PID 1 established a handler can be sent to it (apart
from the SIGKILL/SIGSTOP exception).
[pid_namespaces(7)](https://man7.org/linux/man-pages/man7/pid_namespaces.7.html)
This means a microVM init cannot be treated as an ordinary child process or
assume default signal delivery will be enough.

Process-group targeting is material. `kill(2)` allows a signal to be sent to
the process group selected by a negative PID; it returns success for a group
when at least one target received the signal, which is still not an exit
confirmation. [kill(2)](https://man7.org/linux/man-pages/man2/kill.2.html)
After a child exits, a parent must wait to release its resources; otherwise it
remains a zombie. [wait(2)](https://man7.org/linux/man-pages/man2/waitpid.2.html)

Tini is a concise, version-pinned production precedent for the needed
supervision responsibilities. It spawns one child, waits while reaping zombies
and forwarding signals, and optionally forwards to the child process group so
that shell descendants are included.
[Tini v0.19.0](https://github.com/krallin/tini/tree/v0.19.0#understanding-tini)
The project also documents why PID 1/subreaping matters and why a direct child
alone is insufficient for a workload that creates descendants.

Cloud Hypervisor itself is not a guest init system. Its v53.0 API has distinct
VM and VMM shutdown commands and an internal command-response control loop,
but it does not supervise the guest’s workload process tree.
[Cloud Hypervisor v53.0 API](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/docs/api.md#L419-L437)
The guest must therefore remain the owner of workload signal, wait/reap, and
poweroff semantics. Firecracker documentation reaches the same boundary from
the VMM side: it documents that the guest init is expected not to exit and that
guest power management behavior depends on the virtual hardware/architecture.
[Firecracker v1.14.1 rootfs and kernel setup](https://github.com/firecracker-microvm/firecracker/blob/v1.14.1/docs/rootfs-and-kernel-setup.md)

### Guest-init options

| Option | How `SHUTDOWN` remains responsive | Workload/process-tree handling | Trade-offs |
|---|---|---|---|
| **Single supervisor event loop** | Concurrently wait for the control read and the direct child’s state change; first event drives a defined state transition. | Start the workload in an owned process group/session; on shutdown signal that group, reap children until the group is gone, then power off. | Clearest one-owner state machine and accounting. Requires an async/pollable child-wait integration rather than a blocking `status()` path. |
| **Dedicated control reader feeding the PID 1 supervisor** | A reader remains blocked on the control channel and reports `SHUTDOWN` to the single workload supervisor. | The supervisor—not the reader—signals, waits/reaps, escalates and powers off. | Can fit a blocking control interface. Requires explicit ownership/cancellation so a reader cannot outlive or race the supervisor. |
| **Signal only the direct workload child** | Reader can be concurrent, but descendants depend on workload cooperation. | Direct child may exit while children remain; PID 1 must still reap/orchestrate them. | Lowest mechanism cost; unsafe for shell wrappers and spawned descendants unless the workload contract already guarantees propagation. |
| **Terminate PID 1 or immediately power off on request** | Fast only superficially. | Linux kills the rest of a PID namespace when its init exits; reaping and graceful workload cleanup are skipped. | Not compatible with orderly Service shutdown. |

### Recommended guest lifecycle model

DESIGN should preserve exactly one guest lifecycle owner (PID 1) and make it
simultaneously responsive to control input and workload completion. On a
received shutdown request, it should:

1. Stop accepting/starting further workload work.
2. Send the agreed graceful signal to the **defined workload process group**,
   not merely whichever direct child happens to be running.
3. Reap the direct child and any adopted children, retaining the exit cause for
   the host-visible lifecycle result.
4. After a bounded workload grace, apply the separately agreed escalation only
   to still-live members of that workload group.
5. Invoke guest poweroff only after the guest’s workload/reaping completion
   rule is met. Linux documents `RB_POWER_OFF` as system poweroff; a successful
   stop/restart `reboot()` call does not return.
   [reboot(2)](https://man7.org/linux/man-pages/man2/reboot.2.html)

This is a behavior model, not a request to add a signal, enum, method, or
protocol frame. The accepted design must pin the exact signal, process-group
creation boundary, descendant policy, grace deadline, and exit-status mapping.
It must also state whether `SHUTDOWN` can arrive before `EXEC`, during an
`EXEC` transition, or after natural workload exit.

## Published latency evidence and candidate targets

### Published results and existing RCA observations

The table intentionally keeps VM creation, guest init/READY, application
reachability, Service health/`Stable`, shutdown request, and completed cleanup
separate. “Boot time” is not a portable metric without its endpoints.

| Evidence class | System, version, conditions | Boundary and repetitions | Reported result | Comparability limits |
|---|---|---|---|---|
| **Published upstream specification** | Firecracker [v1.14.1](https://github.com/firecracker-microvm/firecracker/blob/v1.14.1/SPECIFICATION.md): M5D.metal (HT disabled) and M6G.metal; 1 vCPU, 128 MiB; tuned kernel, minimal rootfs, serial console disabled, available host resources. | Receipt of `InstanceStart` API call → start of Linux `/sbin/init`; CI-enforced bound. Cold boot. No percentile reported. | **≤125 ms**. Separately, VMM process start to API availability has 8 CPU-ms target but 6–60 ms wall-time variation. | Init start only: not guest READY, vsock readiness, Service health, `Stable`, or shutdown. Hardware and tiny VM differ from the repository profile. |
| **Published peer-reviewed benchmark** | Abeni, *Journal of Systems Architecture* 154 (2024): AMD Ryzen 7 5700U 1.4 GHz, 16 GB, 1 TB NVMe, Ubuntu 23.10, PREEMPT_RT 6.2.14-rt3. Minimal busybox guest immediately shuts down; initial experiment says 1 vCPU, 1 GiB RAM, ~5 MiB initramfs. | Cold **combined boot + immediate guest shutdown**, average and maximum. The paper documents 100 runs for the initial experiment; it reports avg/max, not quantiles, for the Cloud Hypervisor rows. | Cloud Hypervisor + standard Linux: **386 ms avg, 451 ms max**. Optimized Linux: **188 ms avg, 237 ms max**. | Combined measurement cannot be split into READY versus shutdown. Empty/instant workload, real-time host, and “Intel Cloud Hypervisor” era/version are not current Service conditions. [Paper](https://www.iris.santannapisa.it/retrieve/7749e680-a0c2-44e6-b9a3-264bd7211d6c/blah.pdf) |
| **Published production benchmark** | Depot (May 2026): Cloud Hypervisor v51.1.0/KVM, i7i.metal-24xl bare metal, Debian 13; cold Ubuntu image; 8 vCPU/16 GiB VM; guest agent over vsock. No warm pool. | Helper starts VM then runs `/usr/bin/date`; by that point its network is up. This is later than init and closer to agent-ready, but is not a declared Service health/`Stable` test. | Optimized configuration: **P50 ~600 ms, P90 1.2 s**; individual shown run 789 ms. | P95/P99, shutdown, image-cache state, and service health are not reported. It is a first-party published engineering report, not an independent paper. [Method and results](https://depot.dev/blog/optimizing-microvm-boot-times) |
| **Published historical microVM benchmark** | Firebench: Firecracker v0.20.0; Dell PowerEdge R210, Xeon L3426 1.87 GHz, 8 GiB, Ubuntu 4.15; Alpine 3.10/OpenRC, Linux 5.3, 1 vCPU/512 MiB, per-VM TAP. Sequential path repeated 10 times; concurrent 10/20 also tested. | Shutdown: HTTP request to guest agent → guest powered off and host control process exits. Kernel boot ends at final dmesg marker; service-ready is VM launch → HTTP 200. Means, no percentiles. | Firecracker kernel boot mean **800 ms sequential / 1,000 ms concurrent**; **~2,000 ms average shutdown**. | Older VMM and hardware, guest uses reboot because that Firecracker version lacked a power model, no jailer, low sample count. It is useful mainly because it declares a true completed-shutdown boundary. [Method and results](https://dreadl0ck.net/papers/Firebench.pdf) |
| **Existing RCA observation — not external or a benchmark** | Current repository’s qualified native host; Cloud Hypervisor v53.0, Ubuntu kernel 7.0.0-29, default CLI, Service workload. | Stop call: host write/flush, fixed request wait, VMM grace and forced termination, cleanup. Single healthy Service observation, no percentile. | **12,020.339 ms total**: writer `Ok` at **0.119 ms**, request window **2,002.164 ms**, VMM grace **10,017.545 ms** then SIGKILL, cleanup calls 0.630 ms. | Precisely explains the current defect, but cannot set an SLO. The writer result proves no guest receipt. [RCA](../../analysis/root-cause-analysis-vm-lifecycle-latency-283.md) |
| **Existing RCA observation — not external or a benchmark** | Current repository’s qualified native host; finite Job control case. | EXEC release → normal VMM exit. A few samples, not a distribution. | **30.445 ms**, with other cited trials **112.179 ms** and **113.641 ms**. | Demonstrates the ten-second grace is not intrinsic host cleanup; it is not a Service-stop or percentile measurement. [RCA](../../analysis/root-cause-analysis-vm-lifecycle-latency-283.md) |
| **Existing RCA observation — not external or a benchmark** | Current repository’s qualified native host; Service create/READY path. | Create → current guest READY signal; one untraced observation. | **1,131.665 ms**. | Not a percentile, not stable/health, and tracing perturbed other trials. [RCA](../../analysis/root-cause-analysis-vm-lifecycle-latency-283.md) |

### Proposed initial targets — not validated implementation performance

These are candidate acceptance targets for a later approved design and an
explicit benchmark protocol. They do not modify current timeouts and must not
be reported as current performance.

| Boundary to define in DESIGN | Candidate target | Rationale and confidence |
|---|---|---|
| Current-profile healthy VM launch → existing guest `READY`, under a named cold/warm profile and within the chosen lifecycle capacity | **P95 ≤2,000 ms; P99 ≤3,000 ms** | Depot reaches an agent command at P90 1.2 s on a materially larger 8 vCPU/16 GiB cold VM; the RCA saw one 1.132 s READY. A 2 s P95 leaves headroom for the uncalibrated profile but requires local distribution validation. **Medium-low confidence.** |
| Controlled finite-Job **natural completion**: EXEC release → normal VMM reaper-observed exit (report cleanup separately) | **P95 ≤500 ms; P99 ≤1,000 ms** | The 2024 Cloud-Hypervisor study reports 188–237 ms for *combined* optimized boot+instant shutdown, and the RCA observed finite-Job natural VMM exit in 30–114 ms. This is an exploratory target for a finite workload that exits on its own, not a stop-request or Service-shutdown target. **Low confidence.** |
| Healthy long-running Service stop request → completed cleanup | **No universal millisecond target yet.** Require stage histograms first: request submission; optional guest receipt; workload-group exit; guest poweroff; VMM exit; cleanup. | Published shutdown evidence either combines boot and shutdown, uses an immediate workload, or is old Firecracker. Service shutdown depends on signal handling and the workload’s own graceful period. A made-up global number would conceal the very semantics #283 asks to expose. |
| Service `READY` → health/`Stable` | **No universal target.** | These are workload/data-plane contracts, not VM lifecycle creation. Neither Firecracker’s init target nor the cited Cloud Hypervisor benchmarks measures them. |
| Failure handling: no guest acknowledgement, live workload after grace, no VMM exit, or incomplete cleanup | **Separate configurable failure/grace deadlines; never a healthy-path SLO.** | The current 10-second VMM grace is failure containment evidence, not a latency objective. The two-second unconditional request wait has no such justification and should not remain a successful-stop floor. |

Before adopting any candidate: specify the exact clock endpoints, VM resources,
image/kernel, cached versus cold inputs, host capacity/parallel load, sample
size, percentile method, failure accounting, and whether the target applies to
Job, Service, or both. Snapshot/warm restore results must be reported in a
separate lane; they do not validate cold boot.

## Recommended directions

1. **Approve one bounded, keyed allocation-lifecycle scheduling design for the
   shared #260/#283 path.** It must make unrelated allocations concurrently
   eligible while preserving the current reconciler-runtime, action-shim, and
   exit-observer responsibilities and a real outcome path between them. Do not
   solve only the innermost driver call and do not use detached work.
2. **Define stop success from completion, not host write latency.** In the
   healthy case, wait on DESIGN-selected lifecycle evidence; reserve bounded
   grace and force termination for uncooperative cases. The current
   `Driver::stop` guarantee is only a later `status == NotFound`; explicitly
   verified VMM exit/cleanup is a stronger proposed condition. Decide
   separately whether guest receipt needs a protocol acknowledgement.
3. **Approve PID 1 supervision semantics for a running workload.** Read
   `SHUTDOWN` concurrently with child state, signal and drain a defined process
   group, reap, then power off. The direct-child-only alternative is acceptable
   only if the workload contract proves it has no independent descendants.
4. **Create a stage-separated performance contract before changing numerical
   deadlines.** It must expose the capacity queue, VM launch→READY,
   request→guest receipt when available, request→workload exit,
   request→VMM exit, and request→cleanup. Keep Service health/`Stable` outside
   the VM lifecycle target unless separately specified.

Increasing test timeouts, adding a larger unconditional delay, or reporting a
writer return as shutdown completion does not address any of these directions.

## Scope agreement and DESIGN questions

1. Is the concurrency unit the full allocation evaluation/action lifecycle
   (recommended), and what is the initial capacity/fairness rule? Are starts
   and stops one shared resource pool or separately bounded pools?
2. What is the exact disposition of an already queued or active allocation
   operation during server shutdown: admission closure, cancellation boundary,
   drain deadline, result persistence, and operator-visible error?
3. How must the keyed scheduler coexist with the independent exit observer:
   specifically, what preserves the current stop-versus-observed-exit LWW
   arbitration, and what observer drain/terminal-publication state is required
   before server shutdown is complete?
4. Does DESIGN retain the current `Driver::stop` postcondition of subsequent
   `status == NotFound`, or define a stronger VMM-exit and verified-cleanup
   success condition? In either case, how are normal exit, forced termination,
   and individual cleanup failures classified? Is explicit guest receipt
   required for diagnostics or semantics? If yes, that is a protocol-design
   change requiring an exact contract before implementation.
5. What are the guest PID 1’s exact process-group/session boundary, graceful
   signal, workload grace, escalation signal, descendant/reaping rule, and
   exit-status mapping? How are SHUTDOWN-before-EXEC and SHUTDOWN-versus-natural
   exit ordered?
6. Which named benchmark profiles establish the candidate targets: VM resource
   sizes, kernel/rootfs/image cache state, Job versus Service workload,
   lifecycle concurrency, host conditions, repetitions, and percentile method?
7. Is `TickContext.deadline` a whole-evaluation budget, an admission deadline,
   or a driver-operation deadline? #260 identifies the gap but does not choose
   among these semantically different policies.

Out of scope: persistence/recovery redesign, logical workload deletion,
Service identity removal, dataplane teardown, and any new public API. Those
must not be pulled into this design merely because lifecycle work becomes
concurrent.

## Source register

### Repository and issue context

- [Issue #283](https://github.com/overdrive-sh/overdrive/issues/283) and
  [issue #260](https://github.com/overdrive-sh/overdrive/issues/260), including
  comments, read 2026-09-09.
- [RCA](../../analysis/root-cause-analysis-vm-lifecycle-latency-283.md) and
  [independent RCA review](../../analysis/review-vm-lifecycle-latency-283.md).
- Source references are pinned to repository baseline
  [`d880ce9`](https://github.com/overdrive-sh/overdrive/tree/d880ce983276e7d4107450b264f6ceab64c70eff).

### Upstream source and official documentation

- [Kubernetes kubelet pod workers v1.34.0](https://github.com/kubernetes/kubernetes/blob/v1.34.0/pkg/kubelet/pod_workers.go)
  — keyed FIFO ownership, terminating/terminated state, cleanup gate,
  cancellation contract.
- [controller-runtime controller options v0.22.0](https://github.com/kubernetes-sigs/controller-runtime/blob/v0.22.0/pkg/controller/controller.go)
  — explicit maximum concurrent reconciles and reconciliation timeout.
- [Cloud Hypervisor API v53.0](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/docs/api.md)
  — VM/VMM commands and internal MPSC command-response ownership.
- [Linux `pid_namespaces(7)`](https://man7.org/linux/man-pages/man7/pid_namespaces.7.html),
  [`kill(2)`](https://man7.org/linux/man-pages/man2/kill.2.html),
  [`wait(2)`](https://man7.org/linux/man-pages/man2/waitpid.2.html), and
  [`reboot(2)`](https://man7.org/linux/man-pages/man2/reboot.2.html) — PID 1,
  process-group, reaping, and poweroff semantics (current published Linux
  man-pages accessed 2026-09-09).
- [Tini v0.19.0](https://github.com/krallin/tini/tree/v0.19.0) — a compact,
  production init precedent for signal forwarding, process-group targeting and
  zombie reaping.
- [Firecracker v1.14.1 specification](https://github.com/firecracker-microvm/firecracker/blob/v1.14.1/SPECIFICATION.md)
  and [design](https://github.com/firecracker-microvm/firecracker/blob/v1.14.1/docs/design.md)
  — version-pinned init boundary, hardware/resources and mutation-rate context.

### Published performance evidence

- Abeni, “Virtualized real-time workloads in containers and virtual machines,”
  *Journal of Systems Architecture* 154 (2024), article 103238:
  [publisher-hosted PDF](https://www.iris.santannapisa.it/retrieve/7749e680-a0c2-44e6-b9a3-264bd7211d6c/blah.pdf).
- Busse et al., “Firebench” (Firecracker v0.20.0 versus QEMU microVM):
  [paper PDF](https://dreadl0ck.net/papers/Firebench.pdf).
- Depot engineering, “How we got microVMs booting in under a second” (Cloud
  Hypervisor v51.1.0): [methodology and P50/P90 report](https://depot.dev/blog/optimizing-microvm-boot-times).
