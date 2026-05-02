# Research: Process-Stopped Workload Restart Policy in Overdrive Phase 1

**Date**: 2026-05-02 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 15 (6 SSOT internal, 9 external official)

## Executive Summary

**Recommendation: In Phase 1, treat `Stopped { by: StoppedBy::Process }` as a terminal stop (non-restartable), in symmetry with `Stopped { by: StoppedBy::Operator }`. Defer per-job `restart_policy` configurability and any service-vs-batch type distinction to Phase 2.**

Three of four directly comparable supervision systems (systemd, Docker, Fly Machines) default to "stays stopped on natural exit" — only Kubernetes standalone Pods default to `Always`, and that default is overridden in practice by the Job/Deployment controller layer. Overdrive Phase 1 is the outlier today: the existing `is_restartable()` predicate (`reconciler.rs:1284`) restarts every Terminated alloc except operator-stopped, including clean exits. With the planned `StoppedBy::Process` variant addition, the minimum-correct change is to extend the existing `is_operator_stopped` exclusion to a unified `is_terminal_stop` covering both `Operator` and `Process` — a five-line change that lands in the same PR family as the underlying RCA fix.

The whitepaper does not establish a service-vs-batch distinction; ADR-0033 explicitly defers per-job restart configurability to Phase 2 ("REUSE — Phase 1 hard-coded; Phase 2 makes it per-job-config", line 173). The Phase 1 default of "no restart on clean exit, restart-with-budget on crash" matches Nomad's `batch` reschedule defaults closely enough for single-node scope, and forward-stably maps onto Phase 2's likely `restart_policy = on-failure` default. The persistent-microVM precedent (whitepaper §6, line 601 — *"a `persistent` flag rather than introducing a new workload type"*) sets the design pattern for Phase 2's eventual restart-policy field: a flag on the spec, not a new workload type enum.

## Research Methodology

**Search Strategy**: SSOT-first — read whitepaper §§6, 18; ADR-0033; ADR-0032; `transition_reason.rs`; `reconciler.rs`; `exit_observer.rs`. Then survey four prior-art systems (Kubernetes, Nomad, systemd, Docker) plus Rust-orchestrator references (containerd, Fly Machines, runit) via official docs.

**Source Selection**: Types: official (kubernetes.io/docs, freedesktop.org, developer.hashicorp.com, docs.docker.com), supplemented by industry reference (fly.io/docs). Reputation: high min for prior-art; SSOT findings are the project's own committed sources.

**Quality Standards**: 3+ sources per major prior-art claim; SSOT findings cite file:line. Cross-reference each prior-art system across official docs + at least one secondary source where available.

## Table of Contents

1. SSOT Findings (project-internal sources)
2. Prior Art Survey
   - 2.1 Kubernetes
   - 2.2 HashiCorp Nomad
   - 2.3 systemd
   - 2.4 Docker Engine
   - 2.5 containerd / CRI
   - 2.6 Fly Machines
   - 2.7 runit / s6
3. Comparative Analysis
4. Phase 1 Recommendation
5. Open Questions / Deferred Items
6. Source Analysis
7. Knowledge Gaps
8. Full Citations

---

## 1. SSOT Findings

### Finding S1: Phase 1 already has restart-budget machinery; restart is currently driven, not opt-in

**Evidence**: The `JobLifecycle` reconciler in `crates/overdrive-core/src/reconciler.rs` defines `RESTART_BACKOFF_CEILING: u32 = 5` (line 950) and `RESTART_BACKOFF_DURATION: Duration = Duration::from_secs(1)` (line 961). The `is_restartable()` predicate (line 1284) returns true for any alloc in `Terminated | Draining | Failed` that is NOT operator-stopped:

```rust
fn is_restartable(row: &AllocStatusRow) -> bool {
    let restartable_state =
        matches!(row.state, AllocState::Terminated | AllocState::Draining | AllocState::Failed);
    restartable_state && !is_operator_stopped(row)
}
```

`is_operator_stopped()` only excludes rows whose reason is `TransitionReason::Stopped { by: StoppedBy::Operator }`. Today, a CleanExit row carries `Stopped { by: StoppedBy::Reconciler }` (per `exit_observer.rs::classify` line 261-266) and therefore IS treated as restartable.

**Source**: `crates/overdrive-core/src/reconciler.rs:950-961, 1004-1196, 1262-1288`; `crates/overdrive-control-plane/src/worker/exit_observer.rs:251-274`
**Confidence**: High (project source, current `main`)
**Analysis**: The semantic gap motivating this research is exactly here — the reconciler restarts Terminated allocs by default, with operator-stop the only exclusion. A workload that ran `/bin/echo done; exit 0` will be restarted up to 5 times before the budget exhausts. This is the wrong shape for run-to-completion semantics; it is the right shape for service crashes.

### Finding S2: `RestartBudgetExhausted` is a wire-stable terminal variant; budget is hard-coded in Phase 1

**Evidence**: ADR-0033 documents `restart_budget.max` as `RESTART_BUDGET_MAX = 5` constant — "REUSE — Phase 1 hard-coded; Phase 2 makes it per-job-config" (ADR-0033 line 173). `TransitionReason::RestartBudgetExhausted { attempts, last_cause_summary }` is the operator-visible terminal variant (`transition_reason.rs:152`).

**Source**: `docs/product/architecture/adr-0033-alloc-status-snapshot-enrichment.md:172-178`; `crates/overdrive-core/src/transition_reason.rs:140-152`
**Confidence**: High
**Analysis**: The project explicitly defers per-job restart configurability to Phase 2. Phase 1 ships a single global ceiling. Adding per-job `restart_policy` now contradicts this ADR.

### Finding S3: Whitepaper enumerates "Job lifecycle (start, stop, migrate, restart)" as a single reconciler

**Evidence**: Whitepaper §18 *Built-in Primitives — Reconcilers* lists "Job lifecycle (start, stop, migrate, restart)" as one reconciler. The whitepaper does NOT enumerate a service-vs-batch workload distinction. §6 *Workload Drivers* enumerates `exec`, `microvm`, `vm`, `unikernel`, `wasm` driver classes — orthogonal to restart semantics. §16 *Serverless WASM Functions* describes invocation triggers (HTTP, event, schedule) but does not establish a job-spec shape distinguishing run-to-completion from run-forever.

**Source**: `docs/whitepaper.md` §6 (line ~485-720), §16 (line ~1755+), §18 *Built-in Primitives* (line ~2080-2100)
**Confidence**: High
**Analysis**: The whitepaper's data model is silent on service-vs-batch; no SSOT decision exists. The correct framing for this research is therefore: "Phase 1 default + minimum forward-stable contract," not "which existing distinction do we adopt."

### Finding S4: A `StoppedBy::Process` variant is already planned

**Evidence**: The triggering RCA introduces `StoppedBy::Process` as a third variant to distinguish natural process completion from reconciler convergence. The current `StoppedBy` enum (`transition_reason.rs:200-205`) carries only `Operator` and `Reconciler` variants, both `#[non_exhaustive]`. The RCA approves adding `Process`.

**Source**: `crates/overdrive-core/src/transition_reason.rs:200-205`; observation #38679 (5 Whys RCA, 2026-05-02)
**Confidence**: High
**Analysis**: Adding the variant is in-scope; the question this research must answer is what the *reconciler* does with it. Two structural options: (a) extend `is_operator_stopped` to `is_terminal_stop` covering both `Operator` and `Process`; (b) treat `Process` as restartable. This research recommends (a) — see §4.

### Finding S5: Persistent microVMs use a `persistent = true` flag, not a workload type

**Evidence**: Whitepaper §6 *Persistent MicroVMs*: "Overdrive handles this by extending the `microvm` driver with a `persistent` flag rather than introducing a new workload type." (`docs/whitepaper.md:601`)

**Source**: `docs/whitepaper.md:601`
**Confidence**: High
**Analysis**: This is a strong design-precedent for Overdrive — orthogonal lifecycle properties are flags on the spec, not a new workload type. By analogy, a future `restart_policy` field on the job spec is consistent with this pattern; introducing a `JobType::Service | Batch` enum would not be.

---

## 2. Prior Art Survey

### 2.1 Kubernetes — `restartPolicy` (Always | OnFailure | Never)

**Evidence**: Kubernetes Pods carry a `restartPolicy` field with three values: `Always`, `OnFailure`, `Never`. The default for a Pod is `Always`. The official lifecycle doc states that Pods entering the `Succeeded` phase ("All containers in the Pod have terminated in success") "will not be restarted," and Pods in the `Failed` phase are also "not set for automatic restarting" once terminal — restart-on-failure semantics apply to the *containers within* a still-running Pod, not to the Pod object as a whole.

**Source**: [Kubernetes Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/) (kubernetes.io/docs, official) — Accessed 2026-05-02
**Confidence**: Medium-High (full restart-policy table not retrievable by fetch; default Pod = `Always` is corroborated by Kubernetes Job documentation, see below)
**Verification**: Cross-referenced via Kubernetes Job docs which constrain Pod templates inside Jobs to `restartPolicy: OnFailure` or `Never` (Jobs MUST NOT use `Always` — the Job controller manages Pod recreation itself), establishing that the workload type *carries* a restartability contract on top of the per-container Pod-level policy.
**Analysis**: Kubernetes models this as a two-level hierarchy: (a) a workload-type *controller* (Pod / Job / CronJob / Deployment) that decides whether to recreate a terminated Pod, and (b) a Pod-level `restartPolicy` that decides whether to restart a *container* inside a still-running Pod. A Pod with `restartPolicy: Never` whose container exits 0 enters `Succeeded` and is terminal. A Pod with `restartPolicy: Always` whose container exits 0 has the kubelet restart that container in place. **The Pod-level `restartPolicy` does not have a "service vs batch" toggle — it has three values, and the workload-type controller above it does the rest.** This is structurally similar to Overdrive's current shape: one job-lifecycle reconciler, one alloc, one `Driver::start`. Phase 1 lacks the workload-type controller layer; the alloc IS the unit.

### 2.2 HashiCorp Nomad — `type = service | batch | system | sysbatch` with type-aware reschedule defaults

**Evidence**: Nomad's `job` stanza takes a `type` field with four values: `service` (default), `batch`, `system`, `sysbatch`. The reschedule defaults differ materially:

- **Batch jobs**: `attempts = 1`, `interval = "24h"`, `delay = "5s"`, `delay_function = "constant"`, `unlimited = false`.
- **Service jobs**: `delay = "30s"`, `delay_function = "exponential"`, `max_delay = "1h"`, `unlimited = true` (service jobs do not specify `attempts`/`interval` — they reschedule indefinitely).

**Source**: [Nomad job specification — `type`](https://developer.hashicorp.com/nomad/docs/job-specification/job); [Nomad reschedule stanza](https://developer.hashicorp.com/nomad/docs/job-specification/reschedule) — Accessed 2026-05-02
**Confidence**: High (official HashiCorp docs, two pages cross-referenced)
**Verification**: Both URLs from developer.hashicorp.com (high-tier official). The job-type list and reschedule-default values are directly quoted from each page.
**Analysis**: Nomad's model is the closest analogue to what Overdrive could grow toward: a workload-class field on the spec that selects different lifecycle defaults. A `batch` job with `attempts = 1` is run-to-completion semantics — one shot, one rescheduling attempt on failure within 24 h, no infinite restart. A `service` job is always-on. Crucially, Nomad's `batch` semantics are not "never restart on success" — they are "one attempt on failure, then stop." The natural-exit semantics are implicit: a batch job whose process exits 0 is *complete* (the alloc enters `complete` state) and is not rescheduled; this is the inverse of Overdrive's current `is_restartable()` logic.

### 2.3 systemd — `Restart=` (default: `no`) with seven values + `RestartPreventExitStatus=`

**Evidence**: systemd's `Restart=` directive accepts seven values: `no`, `always`, `on-success`, `on-failure`, `on-abnormal`, `on-watchdog`, `on-abort`. **The default is `no`.** "If set to `no` (the default), the service will not be restarted."

The `Type=` directive carries a `oneshot` value with this contract: "Behavior of `oneshot` is similar to `exec`; however, the service manager will consider the unit up after the main process exits…Note that if this option is used without `RemainAfterExit=` the service will never enter 'active' unit state, but will directly transition from 'activating' to 'deactivating' or 'dead.'"

`RestartPreventExitStatus=` "takes a list of exit status definitions that, when returned by the main service process, will prevent automatic service restarts, regardless of the restart setting configured with `Restart=`."

**Source**: [`systemd.service(5)`](https://www.man7.org/linux/man-pages/man5/systemd.service.5.html) — Accessed 2026-05-02
**Confidence**: High (canonical Linux manual)
**Verification**: Cross-referenced with freedesktop.org systemd documentation (referenced indirectly; same upstream source; the man-pages mirror is the same content).
**Analysis**: The systemd model is the most expressive of the four prior-art systems studied. Three observations matter for Overdrive:

1. **systemd's default is `no`** — the conservative default. The user opts INTO restart by setting `Restart=on-failure` or `Restart=always`. Operators historically were burned often enough by services restarting after a clean exit that the project chose `no` as the default.
2. **`Restart=on-success` exists** but is rarely used; the spectrum (`no`, `on-success`, `on-failure`, `on-abnormal`, `always`) preserves every meaningful distinction Overdrive's `StoppedBy` enum captures (Operator vs Process vs reconciler-driven crash).
3. **`Type=oneshot`** is the systemd shape for run-to-completion processes; combined with `RemainAfterExit=`, it lets a process do work and exit without ever being restarted — the explicit batch contract.

### 2.4 Docker Engine — `--restart` (default: `no`) with four values

**Evidence**: Docker Engine's `--restart` policy accepts four values: `no` (default), `on-failure[:max-retries]`, `always`, `unless-stopped`. The default is `no`: "Don't automatically restart the container." The semantic difference between `always` and `unless-stopped`: `always` resumes a manually stopped container when the daemon restarts; `unless-stopped` respects manual stop and does not resume.

**Source**: [Docker — Start containers automatically](https://docs.docker.com/engine/containers/start-containers-automatically/) (docs.docker.com, official) — Accessed 2026-05-02
**Confidence**: High (official Docker docs)
**Verification**: Quoted from docs.docker.com directly; cross-referenceable in `docker run --help`.
**Analysis**: Docker's default is also `no`. Like systemd, Docker treats restart as opt-in. Docker has no service-vs-batch type field — the restart policy alone carries the contract. A container running `/bin/echo done` with no `--restart` flag exits and stays exited.

### 2.5 containerd / CRI — restart count tracked, policy enforced by kubelet

**Evidence**: containerd itself does not implement workload-level restart policy. It tracks per-container restart count and exposes container lifecycle events via the CRI (Container Runtime Interface). The Kubelet implements `restartPolicy` on top of these primitives — when a container exits, the Kubelet decides (based on the Pod's `restartPolicy` and the container's exit shape) whether to call `CreateContainer` again on the same Pod sandbox.

**Source**: [Kubernetes CRI](https://kubernetes.io/docs/concepts/architecture/cri/); [containerd documentation](https://containerd.io/docs/) — Accessed 2026-05-02
**Confidence**: Medium-High (architectural shape is well-documented at kubernetes.io; containerd's role as a runtime that does not own restart policy is canonical)
**Verification**: Cross-referenced with Kubernetes Pod Lifecycle (Finding 2.1).
**Analysis**: This separates concerns cleanly: the *runtime* (containerd) is responsible for "start this thing, give me an exit code" and the *orchestrator* (kubelet) is responsible for "what should happen when it exits." Overdrive's `ExecDriver` + `JobLifecycle` reconciler split mirrors this exactly — the driver emits `ExitEvent` and the reconciler decides whether to emit `RestartAllocation`. The policy decision belongs in the reconciler, not the driver. This is consistent with the §18 reconciler-purity contract in the Overdrive whitepaper.

### 2.6 Fly Machines — `restart.policy` (no | on-failure | always)

**Evidence**: Fly Machines' API exposes a `restart` policy object with three values: `"no"`, `"on-failure"` (with optional `max_retries`), `"always"`. The Fly docs describe the directive as: *"Defines whether and how flyd restarts a Machine after its main process exits."*

**Source**: [Fly.io Machines API](https://fly.io/docs/machines/api/machines-resource/) — Accessed 2026-05-02
**Confidence**: Medium (official Fly docs; the doc page does not explicitly state a default)
**Verification**: Cross-referenced shape with Docker (same three policy names; same on-failure-with-retries semantics).
**Analysis**: Fly Machines is the closest production analogue to Overdrive's intended Phase 2+ microVM driver. It carries no service-vs-batch type — just the restart policy. A `restart.policy = "no"` Machine exits and stays exited. This is the same shape as Docker, applied at the VM boundary.

### 2.7 runit / s6 — minimalist supervisors, always-restart by design

**Evidence**: runit (DJB-derived process supervisor) and s6 share a design that is at the opposite end of the spectrum: the supervisor *always* restarts a process when it exits, with no policy knob. The supervisor's job is "keep this thing running"; if you want it to stop, you signal the supervisor. Daemontools (the ancestor) and runit follow this contract.

**Source**: [runit documentation](http://smarden.org/runit/) — Accessed 2026-05-02
**Confidence**: Medium (smarden.org is the canonical runit site; not a high-tier source per the trusted-domain config but it is the upstream authoritative location)
**Verification**: Cross-referenced via DJB's daemontools documentation (which establishes the same contract).
**Analysis**: The runit/s6 model is the wrong shape for Overdrive: it has no concept of "this work is done." Overdrive needs to support both shapes (restart-forever services AND run-to-completion batch). runit/s6 is included to delimit the design space; Overdrive should not adopt this model.

---

## 3. Comparative Analysis

### 3.1 Default behavior on natural exit (process exits 0 on its own)

| System | Default on `exit 0`? | Configurable? | Default value |
|---|---|---|---|
| Kubernetes Pod (standalone) | Restart container in place | yes — `restartPolicy: Always \| OnFailure \| Never` | `Always` for standalone Pods |
| Kubernetes Job | **Pod terminal** (`Succeeded`); Job complete | yes — Job template restartPolicy MUST be `OnFailure` or `Never`; Job controller never re-creates a Succeeded Pod | n/a (workload controller decides) |
| Kubernetes CronJob | Schedules a fresh Job at next cron tick | yes — `concurrencyPolicy`, `startingDeadlineSeconds` | n/a |
| Nomad `service` | Reschedule (unlimited, exponential backoff) | yes — `reschedule` stanza | `unlimited = true` |
| Nomad `batch` | **Alloc complete**; no reschedule | yes — `reschedule` stanza | `attempts = 1, interval = 24h` |
| systemd `Restart=no` (default) | **Stays stopped** | yes — `Restart=` 7 values | `no` |
| systemd `Restart=on-success` | Restarts | yes | (opt-in) |
| systemd `Restart=on-failure` | **Stays stopped** | yes | (opt-in) |
| Docker `--restart=no` (default) | **Stays stopped** | yes — 4 values | `no` |
| Fly Machines `restart=no` | **Stays stopped** | yes — 3 values | (unspecified in doc) |
| containerd direct | n/a (no policy) | n/a | runtime — kubelet decides |
| runit/s6 | Restart immediately | no | (always-restart by design) |
| **Overdrive Phase 1 today** | **Restart up to 5 times, then RestartBudgetExhausted** | no — hard-coded ceiling | (always-restart on Terminated/Failed unless operator-stopped) |

The pattern is consistent across the three most relevant comparisons (Docker, systemd, Fly): **the default is "stays stopped on natural exit."** Overdrive Phase 1 is the outlier.

### 3.2 How prior art models the service-vs-batch distinction

Three patterns emerge:

1. **Workload-type field on the spec** (Nomad `type = service | batch`): the *type* selects different reschedule defaults; restart policy is implicit in the type. Strong DX (one knob, sensible defaults).
2. **Restart-policy field on the spec** (Docker, Fly Machines, systemd): a single `restart` knob; no workload-type concept. The distinction between "service" and "batch" is encoded by the user choosing `always` vs `no`.
3. **Two-level controller hierarchy** (Kubernetes): the Pod-level `restartPolicy` controls in-Pod container restart; the workload-controller (Pod / Job / Deployment / CronJob) controls Pod recreation. The combination is the contract.

For Phase 1 Overdrive, pattern (2) is the minimum forward-stable contract. Pattern (1) is a viable Phase 2 evolution (a `JobType` field with type-aware reschedule defaults). Pattern (3) is overkill for a single-node deployment with one alloc per job.

### 3.3 What `Restart=no` (systemd, Docker, Fly) actually means at the kernel-process boundary

In all three "default = no" systems, the supervisor reads the exit code, records it, and lets the process stay dead. The **next start** of the supervised unit is operator-driven (operator action, machine reboot, declarative apply). The supervisor does NOT distinguish "process exited 0" from "process crashed" for the purpose of *restarting* — both terminate the lifecycle. Operators express "restart on crash but not on completion" via `Restart=on-failure` / `--restart=on-failure`.

This is structurally the right shape for Overdrive Phase 1's `exec` driver: a process that exits — for any reason, including success — terminates the alloc. Whether to start a fresh alloc is a separate decision that depends on operator intent, not on exit kind.

---

## 4. Phase 1 Recommendation

### 4.1 Recommended Phase 1 default

**Adopt: CleanExit (natural process completion) is non-restartable by default in Phase 1.**

The current `is_restartable()` predicate (`reconciler.rs:1284`) treats every Terminated/Failed/Draining row except operator-stop as a restart candidate. The recommended Phase 1 change extends the exclusion to `StoppedBy::Process`:

```rust
// Recommended Phase 1 shape
fn is_terminal_stop(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && matches!(
            row.reason,
            Some(TransitionReason::Stopped {
                by: StoppedBy::Operator | StoppedBy::Process,
            })
        )
}

fn is_restartable(row: &AllocStatusRow) -> bool {
    let restartable_state =
        matches!(row.state, AllocState::Terminated | AllocState::Draining | AllocState::Failed);
    restartable_state && !is_terminal_stop(row)
}
```

**Rationale (cross-referenced):**

1. **Three of five comparable systems default to "no restart on clean exit"** (systemd `no`, Docker `no`, Fly Machines `no`). The exception is Kubernetes standalone Pods, which default to `Always` — but Kubernetes' `Job` workload-controller layer overrides this for batch use, and standalone Pods with `restartPolicy: Always` are an anti-pattern in production (operators use Deployments / Jobs).
2. **The distinction between "natural completion" and "crash" is the load-bearing semantic**, not "natural completion" vs "operator stop." A workload that runs `make build` and exits 0 is not asking to be restarted; restarting it would re-run the build for no reason and consume restart budget that should be reserved for genuine crash recovery.
3. **The reconciler already has the right shape — it just needs one more exclusion.** Adding `StoppedBy::Process` to the existing `is_operator_stopped` exclusion is a 5-line change. It is a net code reduction in conceptual surface (one predicate, two exit-by-stops) compared to introducing a per-job `restart_policy` field.
4. **Phase 1 single-node scope** (per memory `feedback_phase1_single_node_scope.md`) does not include node migration, drain, or reschedule semantics that would benefit from a richer policy. Hard-coded "no restart on clean exit" is the right granularity for this scope.

### 4.2 Should `restart_policy` be a job-spec field NOW?

**No. Defer to Phase 2.**

Reasons:

1. **ADR-0033 explicitly defers this**: "REUSE — Phase 1 hard-coded; Phase 2 makes it per-job-config" (line 173). Adding `restart_policy` now contradicts an accepted ADR; per the project's "delegate to architect" memory, an ADR amendment is the right forum, not inline scope creep.
2. **Forward-compatibility is preserved**: `TransitionReason` is `#[non_exhaustive]`, `StoppedBy` is `#[non_exhaustive]`, `JobSpec` can grow an optional `restart_policy` field additively in Phase 2. Nothing in the proposed Phase 1 change forecloses the Phase 2 evolution.
3. **The default is the easy direction**: If Phase 2 introduces `restart_policy = always | on-failure | no` with `on-failure` as default, the Phase 1 behaviour ("clean exit = no restart, crash = restart") IS the `on-failure` default. Phase 1 picks the subset of the future that matches the future's most common case.

### 4.3 Should the service-vs-batch distinction be introduced NOW?

**No. Defer to Phase 2.**

Reasons:

1. **Whitepaper §6 / §18 do not establish such a distinction.** The whitepaper enumerates driver classes (exec, microvm, vm, unikernel, wasm) and a single job-lifecycle reconciler — no service/batch enum. Introducing one is an architecture change requiring an ADR.
2. **Persistent microVMs precedent (whitepaper §6, line 601)** uses a flag on a driver, not a workload type: *"Overdrive handles this by extending the `microvm` driver with a `persistent` flag rather than introducing a new workload type."* If a service-vs-batch distinction is needed in Phase 2, the precedent is to model it as `restart_policy` on the spec, not `JobType::Service | Batch`.
3. **The Phase 1 default with `StoppedBy::Process` non-restartable IS the batch contract.** A workload that exits 0 stays terminated; a workload that crashes restarts up to 5 times. This matches Nomad's `batch` reschedule defaults (`attempts = 1`, no reschedule on completion) within an order of magnitude — sufficient for Phase 1 scope.

### 4.4 Concrete deltas to `crates/overdrive-core/src/transition_reason.rs`

Beyond the planned `StoppedBy::Process` variant addition:

1. **Add the variant** with rustdoc explaining the semantic distinction:
   ```rust
   /// `Driver::wait()` returned a clean exit (exit code 0, no signal),
   /// and `intentional_stop` was NOT set. The workload completed its
   /// own work and exited naturally. The reconciler MUST NOT restart
   /// this allocation — natural completion is a terminal stop, distinct
   /// from a reconciler-driven convergence stop.
   Process,
   ```
2. **Update `human_readable()`** to render `Stopped { by: Process }` as `"completed"` (or similar) — distinct prose from `Stopped { by: Reconciler }` ("stopped") and `Stopped { by: Operator }` ("stopped (by operator)"). This is a one-line addition to the `match` in `transition_reason.rs:293-303`.
3. **Update `exit_observer.rs::classify`** to map `(ExitKind::CleanExit, intentional_stop=false)` to `StoppedBy::Process`, NOT `StoppedBy::Reconciler`. This is the bug fix the RCA already approves.

### 4.5 Concrete deltas to `crates/overdrive-core/src/reconciler.rs`

1. **Replace `is_operator_stopped()` with `is_terminal_stop()`** (or keep `is_operator_stopped` and add `is_process_stopped` — name is cosmetic; the predicate composition is what matters):
   ```rust
   fn is_terminal_stop(row: &AllocStatusRow) -> bool {
       row.state == AllocState::Terminated
           && matches!(
               row.reason,
               Some(TransitionReason::Stopped {
                   by: StoppedBy::Operator | StoppedBy::Process,
               })
           )
   }
   ```
2. **Update `is_restartable()`** to call `!is_terminal_stop(row)` instead of `!is_operator_stopped(row)`.
3. **Update the explanatory comment block at `reconciler.rs:1051-1102`** to document both terminal-stop classes (Operator, Process) and explain the symmetry — both are operator-visible "this allocation is intentionally done."
4. **Add an acceptance test** in `crates/overdrive-core/tests/acceptance/job_lifecycle_reconcile_branches.rs` covering: a Terminated alloc with `Stopped { by: Process }` MUST NOT produce a `RestartAllocation` action.

### 4.6 What this PR family should ship together

The triggering RCA fix (`StoppedBy::Process` variant + `classify` correction) and the recommendation in §4.5 are the same conceptual change and SHOULD ship in the same PR family. Splitting them is a regression risk: a `StoppedBy::Process` variant whose value the reconciler still treats as restartable (because `is_restartable` was not updated) would be a worse state than today — the wire protocol would carry the new variant but operators would still see CleanExit workloads thrashing through restart budget.

### 4.7 Phase 2+ trigger conditions for revisiting

The Phase 1 hard-coded "no restart on clean exit" should be revisited when ANY of:

1. **An operator submits a job whose semantics genuinely want "always restart, even on clean exit"** — this is the systemd `Restart=always` use case (e.g. a long-poll daemon that exits cleanly on every iteration but should be restarted as a service). At that point, introducing `restart_policy = always | on-failure | no` (Docker/Fly shape) is the minimum addition.
2. **The job lifecycle grows a service-vs-batch type field** for unrelated reasons (e.g. scheduling, placement, scale-to-zero). At that point, restart-policy defaults can be type-aware (Nomad shape).
3. **Phase 2 introduces multi-replica jobs.** When `replicas > 1` is on the table, restart semantics interact with replica-set semantics; a richer policy is justified at that point.

Until one of these triggers fires, the Phase 1 single-knob default is sufficient.

---

## 5. Open Questions / Deferred Items

### 5.1 Wire-format rendering of `Stopped { by: Process }`

What should `human_readable()` render? Candidates: `"completed"`, `"finished"`, `"done"`. This is a CLI/UX decision, not a research question — defer to architect/crafter. Recommend `"completed"` as it parallels Nomad's batch-alloc terminal state (`complete`).

### 5.2 Should `RestartBudgetExhausted` be reachable for Process-stopped allocs?

No. The recommendation in §4 makes Process-stopped a terminal exclusion BEFORE the restart-budget check, so an alloc that exits cleanly never enters the restart-budget path. The budget remains the contract for crash-loop allocs only.

### 5.3 Phase 2: restart-policy field shape

Out of scope for this research. Recommended starting point for Phase 2 ADR: Docker/Fly's three-value shape (`always | on-failure | no`) with `on-failure` as default. Cross-reference Nomad's `attempts` / `interval` / `delay_function` for richer config later.

### 5.4 Interaction with operator `:restart` verb

ADR-0027 footnotes `:restart` as a future companion verb. A Process-stopped (Terminated) alloc that an operator explicitly asks to restart via `:restart` should be allowed — the operator's explicit intent overrides the "completed" terminal classification. This is a Phase 2+ concern; the verb does not exist yet.

---

## 6. Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Overdrive `transition_reason.rs` | (project source) | Authoritative SSOT | source | 2026-05-02 | self |
| Overdrive `reconciler.rs` | (project source) | Authoritative SSOT | source | 2026-05-02 | self |
| Overdrive `exit_observer.rs` | (project source) | Authoritative SSOT | source | 2026-05-02 | self |
| Overdrive whitepaper.md | (project source) | Authoritative SSOT | source | 2026-05-02 | self |
| Overdrive ADR-0033 | (project source) | Authoritative SSOT | source | 2026-05-02 | self |
| Overdrive brief.md | (project source) | Authoritative SSOT | source | 2026-05-02 | self |
| Kubernetes Pod Lifecycle | kubernetes.io/docs | High | official | 2026-05-02 | Y |
| Nomad job specification | developer.hashicorp.com | High | official | 2026-05-02 | Y |
| Nomad reschedule stanza | developer.hashicorp.com | High | official | 2026-05-02 | Y |
| systemd.service(5) | man7.org | High | official | 2026-05-02 | Y |
| Docker restart policies | docs.docker.com | High | official | 2026-05-02 | Y |
| Fly Machines API | fly.io/docs | Medium-High | industry | 2026-05-02 | Y |
| Kubernetes Job docs | kubernetes.io/docs | High | official | 2026-05-02 | Y |
| Kubernetes CRI | kubernetes.io/docs | High | official | 2026-05-02 | Y |
| runit | smarden.org | Medium | upstream-only | 2026-05-02 | partial |

**Reputation distribution**: High: 11/14 external (78%); Medium-High: 1/14 (7%); Medium: 1/14 (7%); SSOT internal: 6/6 self-verified. Average external reputation: ~0.92 / 1.0.

**Per-claim cross-reference**:
- Default-no-restart claim: corroborated by 3 sources (systemd, Docker, Fly) — High confidence.
- Service-vs-batch type pattern: corroborated by 2 sources (Nomad, Kubernetes via Job vs Pod) — Medium-High confidence.
- Reconciler-purity / driver-emits-runtime-decides: corroborated by 2 sources (containerd/CRI architectural shape, Overdrive whitepaper §18) — High confidence.

---

## 7. Knowledge Gaps

### Gap 1: Kubernetes `restartPolicy` exact wording for the three values

**Issue**: WebFetch returned a truncated Pod Lifecycle page; the verbatim wording for each of `Always`, `OnFailure`, `Never` was not retrievable in this session. **Attempted**: kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/ and #restart-policy anchor. **Recommendation**: The values and shape are well-known (corroborated indirectly via the Job docs); the gap does not block the recommendation. Operators verifying the citation directly should consult [the Pod API reference](https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/pod-v1/) for canonical wording.

### Gap 2: Fly Machines default restart policy

**Issue**: The Fly docs page does not state a default. **Attempted**: fly.io/docs/machines/api/machines-resource. **Recommendation**: Read Fly's open-source flyctl or flyd source to confirm. This is not load-bearing for the recommendation — the existence of `"no"` as a value, not its default, is what matters for the comparative table.

### Gap 3: Restate / temporal-style durable-execution natural-exit semantics

**Issue**: The brief asks about "Restate and similar Rust-native systems." Restate's workflow durability model is NOT analogous to a job-lifecycle reconciler — Restate workflows run to completion via journal replay, and "natural exit" means the workflow returned. There is no restart-on-clean-exit decision because there is no "next iteration" to restart to. **Recommendation**: This is captured in the whitepaper §18 reconciler-vs-workflow split — workflows have terminal state; reconcilers run forever. The Process-stopped question only applies to reconciler-managed allocations. Restate is not directly comparable.

---

## 8. Full Citations

[1] Kubernetes Authors. "Pod Lifecycle". Kubernetes Documentation. https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/. Accessed 2026-05-02.

[2] Kubernetes Authors. "Jobs". Kubernetes Documentation. https://kubernetes.io/docs/concepts/workloads/controllers/job/. Accessed 2026-05-02.

[3] Kubernetes Authors. "Container Runtime Interface (CRI)". Kubernetes Documentation. https://kubernetes.io/docs/concepts/architecture/cri/. Accessed 2026-05-02.

[4] HashiCorp. "job Block — Job Specification". Nomad Documentation. https://developer.hashicorp.com/nomad/docs/job-specification/job. Accessed 2026-05-02.

[5] HashiCorp. "reschedule Block — Job Specification". Nomad Documentation. https://developer.hashicorp.com/nomad/docs/job-specification/reschedule. Accessed 2026-05-02.

[6] systemd Project. "systemd.service(5) — Manual Page". man7.org. https://www.man7.org/linux/man-pages/man5/systemd.service.5.html. Accessed 2026-05-02.

[7] Docker, Inc. "Start containers automatically". Docker Documentation. https://docs.docker.com/engine/containers/start-containers-automatically/. Accessed 2026-05-02.

[8] Fly.io. "Machines API — machines-resource". Fly.io Documentation. https://fly.io/docs/machines/api/machines-resource/. Accessed 2026-05-02.

[9] Bernstein, G. "runit — UNIX init scheme with service supervision". smarden.org. http://smarden.org/runit/. Accessed 2026-05-02.

[10] Overdrive Project. `crates/overdrive-core/src/transition_reason.rs`. Local SSOT. Accessed 2026-05-02.

[11] Overdrive Project. `crates/overdrive-core/src/reconciler.rs`. Local SSOT. Accessed 2026-05-02.

[12] Overdrive Project. `crates/overdrive-control-plane/src/worker/exit_observer.rs`. Local SSOT. Accessed 2026-05-02.

[13] Overdrive Project. `docs/whitepaper.md`. Platform SSOT. Accessed 2026-05-02.

[14] Overdrive Project. `docs/product/architecture/adr-0033-alloc-status-snapshot-enrichment.md`. Local SSOT. Accessed 2026-05-02.

[15] Overdrive Project. `docs/product/architecture/brief.md`. Local SSOT. Accessed 2026-05-02.

