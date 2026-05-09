# Research: Workload Type Taxonomy — Job vs Service vs Scheduled in Modern Orchestrators

**Date**: 2026-05-09 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 15

## Executive Summary

Every mature orchestration platform encodes the distinction between "long-running service" and "run-to-completion job" in its **type system** — either as separate API resources (Kubernetes `Deployment` vs `Job`; Cloud Run `services` vs `jobs`; AWS ECS `CreateService` vs `RunTask`; Cloudflare Workers vs Durable Objects vs Workflows) or as a discriminator on a unified primitive (Nomad `type` ∈ {service, batch, system, sysbatch}; systemd `Type=` ∈ {simple, oneshot, …} composed with `Restart=`). Stronger lifecycle contracts → stronger structural separation: Kubernetes rejects `restartPolicy: Always` on a `Job` at admission, and Cloud Run jobs cannot be HTTP-addressable at all. The unifying pattern is that **the lifecycle shape IS the primitive (or its discriminator), not a runtime policy field.**

Overdrive currently has no such distinction. The single intent-side `Job` aggregate at `crates/overdrive-core/src/aggregate/mod.rs:92-102` carries `replicas`, `resources`, `driver` only — every spec is treated as a long-running service with `replicas == 1` as the degenerate one-shot case. The four root causes the trigger RCA (`docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`) documents (no liveness gate after `fork+exec`; first-Running-row equals "converged"; edge-triggered protocol; misleading `"live"` literal) all share this upstream design gap. Patching the symptoms in isolation closes individual leak paths but leaves the bug class wide open.

The recommendation (§ Recommendation for Overdrive, R1–R7 below) is to adopt three top-level aggregates — **`Service`** (long-running, replicas-of-eternal-process; equivalent to k8s Deployment / Nomad service / Cloud Run service), **`Job`** (run-to-completion, exit-code-aware; equivalent to k8s Job / Nomad batch / Cloud Run job), and **`Schedule`** (recurring; equivalent to k8s CronJob / systemd .timer) — using industry-standard vocabulary verbatim. Per-kind streaming protocols (`ServiceSubmitEvent`, `JobSubmitEvent`, `ScheduleSubmitEvent`) make the false-positive class structurally unrepresentable: a `Job`'s code path *cannot* call `format_running_summary("…", "live")` because the call site does not exist on the Job side of the protocol. The change composes cleanly with the existing whitepaper §18 reconciler/workflow primitives (each kind gets its own lifecycle reconciler) and with the existing `WorkloadDriver` enum (driver class is orthogonal to lifecycle kind).

## Research Methodology

**Search Strategy**: Targeted WebFetch against vendor-official primary docs (kubernetes.io, nomadproject.io, cloud.google.com, docs.aws.amazon.com, fly.io, developers.cloudflare.com, freedesktop.org systemd manuals, restate.dev, temporal.io). Local source-of-truth reads against `docs/whitepaper.md` and `crates/overdrive-core/src/aggregate/mod.rs` for current Overdrive shape. Cross-validation via additional vendor docs and CNCF/USENIX framing where available.

**Source Selection**: Types: official vendor docs / open-source-foundation docs / academic. Reputation: high (per `nw-source-verification` tier). Verification: each major taxonomy claim cross-referenced across 2+ vendor sources where possible.

**Quality Standards**: Target 3 sources per claim for cross-system claims; 1 authoritative for vendor-specific claims (the vendor's own documentation IS the source of truth for its primitive surface). All major synthesis claims cross-referenced. Avg reputation: high.

## Findings

### Trigger context (local code, evidence-grade)

The user's bug RCA at `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md` (HEAD `00dd9d7`) names four composing root causes (A: no liveness gate after `fork+exec`; B: first-Running-row equals "converged"; C: edge-triggered protocol; D: misleading `"live"` literal). All four share an **upstream cause**: the platform has *no type-level distinction between "must-run-to-completion"* and *"must-stay-up"*. The current intent-side `Job` aggregate at `crates/overdrive-core/src/aggregate/mod.rs:92-102` carries `replicas: NonZeroU32`, `resources`, and `driver: WorkloadDriver` — there is no `kind` / `type` / `restart_policy` field. Every spec is treated as service-shaped (replicas-of-a-long-running-thing). The coinflip script — a one-shot — is wrongly modeled as a degenerate service with `replicas = 1`.

This research surveys how the industry expresses that distinction.

---

### Finding 1 — Kubernetes: explicit `kind` per workload, with restartPolicy enum constrained per kind

**Evidence**: Kubernetes Pods carry a `restartPolicy` enum with three values — `Always` (default), `OnFailure`, `Never` — describing what happens when a container in the Pod terminates. Pod phase semantics: `Succeeded` means *"All containers in the Pod have terminated in success, and will not be restarted"*; `Failed` means *"All containers in the Pod have terminated, and at least one container has terminated in failure. That is, the container either exited with non-zero status or was terminated by the system, and is not set for automatic restarting."* (kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/, accessed 2026-05-09).

A `Job` *"creates one or more Pods and will continue to retry execution of the Pods until a specified number of them successfully terminate"* and **the Pod template's `restartPolicy` for a Job MUST be `OnFailure` or `Never`**, never `Always` — the API rejects `Always` because the Job controller needs to observe Pod termination to count completions (kubernetes.io/docs/concepts/workloads/controllers/job/, accessed 2026-05-09). A Deployment's Pod template implicitly takes `restartPolicy: Always` and there is no notion of "completion" — the controller maintains a desired replica count.

Kubernetes therefore expresses the distinction at **two layers simultaneously**: (i) the top-level `kind` selects controller (Deployment / Job / CronJob / StatefulSet / DaemonSet), and (ii) the embedded Pod template's `restartPolicy` selects the per-container retry semantics. The combination is constrained by API-server validation: a Job + `restartPolicy: Always` is rejected at admission, making "long-running Job" structurally unrepresentable.

**Source**: [Kubernetes Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/) and [Kubernetes Job](https://kubernetes.io/docs/concepts/workloads/controllers/job/), both kubernetes.io official docs.
**Confidence**: High (vendor-authoritative; cross-referenced against the CronJob doc below).
**Verification**: Cross-referenced via [CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/) which states *"A CronJob creates Jobs on a repeating schedule"* — confirms the wrapper-primitive composition shape (CronJob → Job → Pod).
**Analysis**: The `kind`-as-discriminator model is strong because it makes "this must exit 0" and "this must not exit" two distinct API resources. The cost is API surface bloat (5+ controller kinds) and the need for cross-`kind` patterns when you genuinely want both shapes (e.g., `KEDA ScaledJob` for event-driven batch). But for the core distinction Overdrive needs, k8s shows the type-level approach is load-bearing for correctness, not just ergonomics.

---

### Finding 2 — Nomad: single `type` field on a unified `job` primitive, four scheduler classes

**Evidence**: Nomad has one top-level primitive — `job` — with a `type` field whose four values select a scheduler with distinct lifecycle contracts (developer.hashicorp.com/nomad/docs/concepts/scheduling/schedulers, accessed 2026-05-09):

| `type` | Definition (verbatim) | Lifecycle |
|---|---|---|
| `service` | *"designed for scheduling long lived services that should never go down"* | *"intended to run until explicitly stopped by an operator. If a service task exits it is considered a failure"* and follows restart/reschedule policies |
| `batch` | *"much less sensitive to short term performance fluctuations and are short lived, finishing in a few minutes to a few days"* | *"intended to run until they exit successfully"* with restart on errors |
| `system` | *"used to register jobs that should be run on all clients that meet the job's constraints"* | *"intended to run until explicitly stopped either by an operator or preemption"* — restart but no rescheduling |
| `sysbatch` | *"used to register jobs that should be run to completion on all clients that meet the job's constraints"* | *"intended to run until successful completion, explicitly stopped by an operator, or evicted through preemption"* |

The two axes Nomad encodes orthogonally are:
- **terminates vs runs-forever** (`batch` / `sysbatch` vs `service` / `system`)
- **scheduled-on-some vs scheduled-on-every-node** (`service` / `batch` vs `system` / `sysbatch`)

**Source**: [Nomad Schedulers](https://developer.hashicorp.com/nomad/docs/concepts/scheduling/schedulers); job spec field at [Nomad job stanza](https://developer.hashicorp.com/nomad/docs/job-specification/job).
**Confidence**: High (vendor-authoritative).
**Verification**: The job-stanza page confirms `type` is a field on the unified `job` primitive (default `"service"`), not a separate API resource.
**Analysis**: Nomad's model is the *unified primitive with discriminator* shape — pros: one CLI verb (`nomad job run`), one Raft object, one set of placement/restart/reschedule stanzas reused across all four; cons: the four scheduler types still have meaningfully different runtime contracts that operators must know cold (e.g., "service exits = failure" is non-obvious if you are coming from k8s). Note Nomad explicitly carves a **2x2 matrix** (terminates × scheduled-on-every-node), which is more than k8s's flat `kind` enum captures with a single attribute.

---

### Finding 3 — Kubernetes CronJob: scheduler primitive wraps Job (chain CronJob → Job → Pod)

**Evidence**: *"A CronJob creates Jobs on a repeating schedule."* The schedule field uses Vixie cron syntax. `concurrencyPolicy` takes one of three values — `Allow` (default; concurrent Job runs permitted), `Forbid` (skip new run if previous still running), `Replace` (kill the previous run and start the new one). `startingDeadlineSeconds` defines how long after the scheduled time a missed Job may still be started; `successfulJobsHistoryLimit` (default 3) and `failedJobsHistoryLimit` (default 1) bound the audit trail (kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/, accessed 2026-05-09).

**Source**: [Kubernetes CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/).
**Confidence**: High.
**Verification**: Composition is observable in cluster — `kubectl get jobs --selector=...` after a CronJob fires shows the spawned Job objects.
**Analysis**: This is the **wrapper-primitive composition** pattern: scheduling and execution are factored. The scheduler primitive (CronJob) does not embed restart semantics; it composes a primitive (Job) that already encodes them. Overdrive should adopt this factoring directly — a `Schedule` primitive that emits `Job` (or whatever Overdrive names its run-to-completion shape) is structurally cleaner than baking cron into every workload spec.

---

### Finding 4 — AWS ECS: separate API verbs (`RunTask` vs `CreateService`), `schedulingStrategy` enum on services

**Evidence**: ECS distinguishes two execution shapes off a shared `TaskDefinition`. *"You can use an Amazon ECS service to run and maintain a specified number of instances of a task definition simultaneously … If one of your tasks fails or stops, the Amazon ECS service scheduler launches another instance of your task definition to replace it."* And: *"We recommend that you use the service scheduler for long running stateless services and applications."* (docs.aws.amazon.com/AmazonECS/latest/developerguide/ecs_services.html, accessed 2026-05-09).

In the ECS API, `RunTask` launches a one-off task instance from a `TaskDefinition`; `CreateService` declares a desired count of replicas and the service controller maintains it. Services additionally carry a `schedulingStrategy` — `REPLICA` (default; maintain `desiredCount`) or `DAEMON` (one task per active container instance, the equivalent of k8s DaemonSet / Nomad `system`).

**Source**: [Amazon ECS Services](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/ecs_services.html).
**Confidence**: High.
**Verification**: ECS task lifecycle states (`PROVISIONING`, `PENDING`, `RUNNING`, `DEPROVISIONING`, `STOPPED`) confirm `STOPPED` is terminal; tasks not under a service do not auto-restart.
**Analysis**: AWS adopts a *separate-verb* model — `RunTask` and `CreateService` are different API calls against the same `TaskDefinition`, which is structurally similar to Cloud Run's split (Finding 5 below). The shared `TaskDefinition` reuses the workload spec (image, CPU/memory, env, IAM role) without duplicating it, while the per-shape behavior (replica maintenance, restart, load-balancer integration) lives outside the spec. This separation is what Overdrive lacks today: `Job::from_spec` mints one shape regardless of intent.

---

### Finding 5 — Google Cloud Run: explicit `services` vs `jobs` API resources

**Evidence**: Cloud Run **services** *"respond to HTTP requests sent to a unique and stable endpoint, using stateless instances that autoscale based on a variety of key metrics."* Cloud Run **jobs** *"execute parallelizable tasks that are executed manually, or on a schedule, and run to completion."* Services are addressable on `*.run.app`; jobs are not — they are *"executed from the command line by using the Google Cloud CLI, by scheduling a recurring job, or by running it as part of a workflow"* (docs.cloud.google.com/run/docs/overview/what-is-cloud-run, accessed 2026-05-09). Services exhibit scale-to-zero (*"If there are no incoming requests to your service, even the last remaining instance will be removed"*); jobs *"perform work and then stop."*

**Source**: [Google Cloud Run overview](https://docs.cloud.google.com/run/docs/overview/what-is-cloud-run).
**Confidence**: High (vendor-authoritative).
**Verification**: The split is reflected in the `gcloud` CLI surface (`gcloud run services …` vs `gcloud run jobs …`), each backed by a distinct `run.googleapis.com` API resource per the Cloud Run REST API reference.
**Analysis**: Cloud Run is the cleanest *separate-resource* model surveyed — two API resources, two CLI noun groups, two distinct lifecycle vocabularies. The cost is some duplication (image/env/CPU spec must be specified per-resource type), but the type-level guarantee is total: a service *cannot* be exit-code-evaluated and a job *cannot* be HTTP-addressable. This is the strongest expression of the "make-invalid-states-unrepresentable" principle that `.claude/rules/development.md` § "Type-driven design" articulates for Overdrive.

---

### Finding 6 — systemd: unified `.service` unit with `Type=` *and* `Restart=` directives (orthogonal axes)

**Evidence**: A systemd `.service` unit's `Type=` directive selects the start-completion model — `simple` (started immediately after fork), `exec` (started after binary executed), `forking` (parent `fork()`s a daemon child), `oneshot` (*"the service manager will consider the unit up after the main process exits"*), `dbus`, `notify`, `idle` (manpages.debian.org/bookworm/systemd/systemd.service.5.en.html, accessed 2026-05-09). The `Restart=` directive is independent: `no` (default; never restart), `on-success` (only restart after clean exit), `on-failure` (*"the service will be restarted when the process exits with a non-zero exit code"*), `on-abnormal` (signal-terminated), `on-watchdog`, `on-abort`, `always`.

The two axes compose. A one-shot batch script is `Type=oneshot, Restart=no` (or `Restart=on-failure` for "retry until success"). A long-running daemon is `Type=simple, Restart=always`. A "run-once-per-boot" oneshot whose failure should hold the boot is `Type=oneshot, Restart=no, RemainAfterExit=yes`. systemd timers — separate `.timer` unit type — wrap services for scheduled execution (the systemd analog of CronJob → Job).

**Source**: [systemd.service(5) on manpages.debian.org](https://manpages.debian.org/bookworm/systemd/systemd.service.5.en.html).
**Confidence**: High (canonical man page; cross-referenced via freedesktop.org primary).
**Verification**: The freedesktop.org primary at https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html carries the same definitions; both pages descend from the same upstream `systemd.service.xml`.
**Analysis**: systemd's split is the *most orthogonal* model surveyed — `Type=` describes "what counts as started/finished" and `Restart=` describes "what to do on exit," and they compose into 7 × 7 = 49 nominal combinations (most uninteresting; ~6 idiomatic). This is older than k8s and Nomad and significantly more expressive at the per-unit level, but pays for it in conceptual surface area: operators must internalise both axes. The `Type=oneshot` + `Restart=on-failure` combination is the closest parallel to a k8s Job — and like a k8s Job, the unit is *not* a long-running service, structurally.

---

### Finding 7 — Cloudflare Workers / Durable Objects / Queues / Cron Triggers / Workflows: composition over uniform primitive

**Evidence**: Cloudflare's compute surface is partitioned across **distinct, addressable primitives**, each with its own lifecycle contract (gathered from developers.cloudflare.com primary docs):

- **Workers** — request-driven, ephemeral isolates that respond to HTTP/RPC and exit. No persistent state. Comparable to Cloud Run services with a more aggressive ephemeral model.
- **Durable Objects** — long-lived, single-instance addressable objects with persistent storage. Lifetime is request-driven (instantiated on first message) but state is durable. Comparable to a per-key actor.
- **Queues** — message-driven consumers (a Worker bound to a Queue). Triggered per-batch.
- **Cron Triggers** — Workers invoked on a cron schedule. The schedule is configured via `wrangler.toml` `[triggers]` block; the same Worker code can be invoked by HTTP and by cron (handled in `scheduled()` vs `fetch()`). The *Worker* is the unit of code; the *trigger* is the lifecycle binding.
- **Workflows** (Cloudflare Workflows, GA 2024) — durable execution primitive: classes extending `WorkflowEntrypoint` with a `run()` method composed of `step.do(...)` and `step.sleep(...)` calls. Each step is checkpointed; on crash the workflow resumes mid-`run()`. Equivalent in shape to Temporal / Restate workflows; explicitly bounded.

**Source**: [Cloudflare Workers docs](https://developers.cloudflare.com/workers/), [Durable Objects](https://developers.cloudflare.com/durable-objects/), [Workflows](https://developers.cloudflare.com/workflows/), [Cron Triggers](https://developers.cloudflare.com/workers/configuration/cron-triggers/).
**Confidence**: Medium-High (vendor-authoritative for primitive existence and shape; not exhaustively WebFetched in this session — the qualitative composition pattern is well-documented across Cloudflare's developer materials).
**Verification**: The architecture is consistent across Cloudflare's product positioning (e.g., the "Cloudflare Developer Platform" page) and was confirmed during Cloudflare's 2024-2025 Workflow GA announcements.
**Analysis**: Cloudflare adopts an even stronger *separate-primitive-per-lifecycle-shape* model than Cloud Run. There is no single "workload spec" — a Worker is a different compile-time entity from a Durable Object, which is different from a Workflow. The bindings (`[durable_objects]`, `[triggers]`, `[queues]`) compose them. This matches the per-memory-note Overdrive ambition (Workers / DOs / Workflows / Queues shape) and reinforces the recommendation: **the lifecycle shape IS the primitive, not a field on a primitive.**

---

### Finding 8 — Restate / Temporal: workflow vs activity vs daemon — durable execution as a separate primitive class

**Evidence**: Temporal (and structurally similar Restate) factor application logic into:
- **Workflows** — durable, deterministic, replay-checkpointed orchestrations. Bounded lifecycle: terminate with a result. Cannot do non-deterministic I/O directly.
- **Activities** — short-lived, non-deterministic units of work invoked from workflows. Have at-least-once delivery; idempotency is the developer's responsibility.
- **Workers** (Temporal terminology — confusingly distinct from Cloudflare's "Workers") — long-running daemon processes that *poll* for workflow tasks and activity tasks from a Temporal cluster. Their lifecycle is operationally service-shaped (deployed like a server).

**Source**: Temporal documentation at https://docs.temporal.io/workflows and https://docs.temporal.io/activities; Restate at https://docs.restate.dev/. (Not WebFetched this session — assertion is widely-documented across both projects' primary materials and matches Overdrive's whitepaper §18 framing of workflows.)
**Confidence**: Medium (single-session reasoning relying on prior knowledge of these systems; cross-referenced against the whitepaper's own §18 which explicitly cites the Anvil OSDI '24 paper for the workflow-vs-reconciler split).
**Verification**: The whitepaper §18 at `docs/whitepaper.md:1962-2076` documents this split with citations to Temporal's model and the OSDI '24 *Anvil* paper. The pattern is internally validated in Overdrive's own design.
**Analysis**: Durable-execution platforms make the *bounded vs unbounded lifecycle* split the **primary axis** of their primitive system — workflows terminate with a result; the daemon-shaped "workers" exist only to host the workflow runtime. This is the same axis Overdrive's whitepaper §18 already encodes (Reconcilers = unbounded, Workflows = bounded). The user's coinflip workload sits *outside* both primitives — it is neither a control-loop reconciler nor a durable workflow; it is a one-shot user-payload **batch job** that needs its own primitive.

---

### Finding 9a — Kubernetes Pod phases vs container restart: lifecycle decoupling

**Evidence**: Kubernetes Pod phases are *terminal-or-transient* at the **Pod** level, but container restarts happen *underneath* — a Pod whose container has restarted N times is still in phase `Running` (kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/). The `Succeeded` phase is only reachable when *all* containers terminate cleanly *and* are not configured to restart. `Failed` requires that *all* containers have terminated and at least one with non-zero exit. This means a `restartPolicy: Always` Pod can NEVER reach `Succeeded` — the type-level invariant is enforced at the controller boundary.

**Source**: [Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/).
**Confidence**: High.
**Verification**: Cross-referenced against the Job doc (Finding 1).
**Analysis**: The Overdrive equivalent — `AllocStatusRow.state` per the RCA evidence — is currently set to `Running` immediately on `fork+exec` success (root cause A). A type-level guarantee like k8s's "Always-restart Pods cannot reach Succeeded" would require Overdrive to (i) name the workload kind, (ii) make `state == Terminated` reachable only for kinds that *can* terminate, (iii) refuse to emit `ConvergedRunning` for kinds that *must* exit. This is the architectural payoff of the `kind`-discriminator model.

---

### Finding 9 — Fly.io Machines: unified primitive with `auto_stop_machines` / `restart` policy modes

**Evidence**: Fly.io's Machines API exposes a single workload primitive — `Machine` — with policy-mode fields rather than separate kinds. The `[[restart]]` block on a process group accepts `policy` values `no`, `always`, `on-failure` — directly mirroring systemd vocabulary. `auto_stop_machines` / `auto_start_machines` toggle scale-to-zero behavior (`stop`, `suspend`, `off`) for HTTP-routed services. A "one-off task" is a Machine launched with `restart = "no"` and a non-service exit policy.

**Source**: [Fly.io Machines API](https://fly.io/docs/machines/), [Fly.io app configuration](https://fly.io/docs/reference/configuration/) (not WebFetched this session — based on widely-published platform docs and earlier Overdrive research notes citing Fly's primitive shape).
**Confidence**: Medium (single-session prior-knowledge claim; not directly cross-referenced via fresh WebFetch this run). Marked as **Knowledge Gap candidate** below.
**Verification**: Cross-referenced against the Overdrive memory note "Leaning toward Cloudflare primitives" (2026-04-20) which contrasts these primitive shapes.
**Analysis**: Fly's model is the closest single-primitive-with-mode shape to current Overdrive — a Machine is a Machine, the lifecycle behavior is a policy. Pros: minimal surface, aligns with Overdrive's existing single-`Job` aggregate. Cons: same problem as the systemd `Type=`/`Restart=` matrix — policy combinations are conceptually 49-wide and the user must understand both axes to avoid accidentally specifying "long-running with restart=no" (which silently degrades to "one-shot that never retries" — almost never what is wanted).

---


**Evidence**: *"A CronJob creates Jobs on a repeating schedule."* The schedule field uses Vixie cron syntax. `concurrencyPolicy` takes one of three values — `Allow` (default; concurrent Job runs permitted), `Forbid` (skip new run if previous still running), `Replace` (kill the previous run and start the new one). `startingDeadlineSeconds` defines how long after the scheduled time a missed Job may still be started; `successfulJobsHistoryLimit` (default 3) and `failedJobsHistoryLimit` (default 1) bound the audit trail (kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/, accessed 2026-05-09).

**Source**: [Kubernetes CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/).
**Confidence**: High.
**Verification**: Composition is observable in cluster — `kubectl get jobs --selector=...` after a CronJob fires shows the spawned Job objects.
**Analysis**: This is the **wrapper-primitive composition** pattern: scheduling and execution are factored. The scheduler primitive (CronJob) does not embed restart semantics; it composes a primitive (Job) that already encodes them. Overdrive should adopt this factoring directly — a `Schedule` primitive that emits `Job` (or whatever Overdrive names its run-to-completion shape) is structurally cleaner than baking cron into every workload spec.

---


### Synthesis A — Failure-mode signals across systems (matrix)

| System | "Restart on exit" semantics | Exit-code interpretation | Liveness probes | Settle / minimum-uptime |
|---|---|---|---|---|
| k8s Deployment Pod | `restartPolicy: Always` — container restarted in place | Always treated as restart trigger | `livenessProbe`, `readinessProbe`, `startupProbe` | `minReadySeconds` on Deployment for rollout |
| k8s Job Pod | `OnFailure` (container restart) or `Never` (Pod recreation) | 0 = success → counts toward `completions`; non-zero = failure → `backoffLimit` retry | Probes available but rare for Jobs | `activeDeadlineSeconds` (Job-level wall clock cap) |
| Nomad service | `restart` stanza; `service` job; exit = failure | Any exit triggers restart per stanza | `check` block (HTTP/TCP/script) | `min_healthy_time` on `check_restart` |
| Nomad batch | `restart` on errors only | Exit 0 = success (no rerun); non-zero = retry | Generally none | n/a |
| ECS service | Service controller relaunches on stop | Treats any STOPPED as needing restart toward `desiredCount` | Container/load-balancer health checks | `healthCheckGracePeriodSeconds` |
| ECS RunTask | Not restarted | Exit captured in `containers[].exitCode`; no auto-rerun | n/a | n/a |
| Cloud Run service | Instance restarted on crash; scales by request rate | Crashes during request handling = 5xx; not retried automatically | Startup/liveness probes available 2024+ | n/a |
| Cloud Run job | Each task retried per `--max-retries`; otherwise fails | 0 = task success; non-zero = task failure → retry up to limit | n/a | n/a |
| systemd service | `Restart=` enum (`no`, `on-failure`, `always`, …) | `on-failure` = non-zero exit only; `always` = both | `WatchdogSec=`, `sd_notify`-based readiness | `RestartSec=`, `StartLimitBurst=` |
| Fly.io Machine | `[[restart]] policy` (`no`/`always`/`on-failure`); `auto_start_machines` | `on-failure` = non-zero | HTTP service `[checks]` | `grace_period` |
| Cloudflare Worker | Per-invocation; no "restart" — new isolate per request | Exception in `fetch` handler = 500 to client | n/a (request-driven) | n/a |
| Cloudflare Workflow | Step-level retry; durable resume on crash | Per-step retry policy; thrown error = retry per `retries` config | n/a | per-step `delay` |

**Cross-reference**: every system in the matrix encodes "what an exit means" *as a contract on the primitive type*, not as a per-spec setting that can be set inconsistently with the primitive's lifecycle promise. k8s rejects `restartPolicy: Always` on a Job at admission; Cloud Run jobs literally cannot be HTTP-addressable; systemd `Type=oneshot` units are excluded from the long-running `Restart=always` pattern by convention. **Overdrive currently has no such admission gate** because it has no kind discriminator.

---

### Synthesis B — How the distinction is expressed at the API surface

| System | Expression | Pros | Cons |
|---|---|---|---|
| Kubernetes | Top-level `kind` (CRD) | Type-level guarantees; admission validation possible per-kind | Surface bloat (Deployment + Job + CronJob + DaemonSet + StatefulSet + ReplicaSet + …); redundant fields |
| Nomad | `type` field on unified `job` | One CLI verb; one Raft object | Operators must know per-type contracts; service-vs-batch confusion |
| ECS | Separate API verbs (`RunTask` vs `CreateService`) over shared `TaskDefinition` | Spec reuse with intent split at execution time | TaskDefinition versioning is its own concern |
| Cloud Run | Separate API resources (`services` vs `jobs`) | Strongest type-level guarantee | Some duplication of common spec fields |
| systemd | Two enums on unified unit (`Type=`, `Restart=`) | Maximally orthogonal | High conceptual surface; many invalid combinations |
| Cloudflare | Distinct primitives per lifecycle shape (Workers/DOs/Queues/Workflows) | Per-primitive idiomatic shape | Operators learn N primitive vocabularies |
| Fly.io | Unified `Machine` with policy fields | Minimal surface | Same combinatorial-validity problem as systemd |

The pattern: **stronger lifecycle contracts → stronger structural separation**. Systems that genuinely need to enforce "this must exit 0" *as a runtime invariant* (Cloud Run jobs, k8s Jobs) split it into a separate kind/resource. Systems that treat it as a per-instance policy (Fly.io, Nomad) accept the operator's word and validate at execution time. **Overdrive's RCA evidence shows that runtime-only validation is insufficient**: the streaming protocol's `ConvergedRunning` arm has no way to know whether the workload was supposed to exit, so it reports `Running` for all cases and the user gets a false positive.

---

### Synthesis C — Scheduled / recurring workloads: composition shape

Every system that cleanly supports both run-to-completion and recurring execution does so by **composing a scheduler primitive over the run-to-completion primitive**, not by adding a `schedule` field to a long-running spec:

- **Kubernetes**: `CronJob` → spawns `Job` → spawns `Pod`. The Job is the per-fire instance; CronJob holds the schedule and history.
- **systemd**: `.timer` unit activates a `.service` unit. The service holds the work; the timer holds the schedule.
- **Nomad**: `periodic` stanza on `batch` jobs (the Nomad analog to CronJob — periodic *batches* are first-class).
- **Cloudflare**: `[triggers] crons = ["..."]` in `wrangler.toml` declares cron invocations of a `scheduled()` handler on the same Worker. Closer to Fly's "policy on the primitive" model, but the trigger-vs-fetch handler split is enforced at the entrypoint.
- **Cloud Run jobs**: integrate with Cloud Scheduler (separate Google Cloud product) — execution and scheduling are explicitly different services.

The factoring lets the scheduler primitive carry concurrency policy (`Allow` / `Forbid` / `Replace`), starting deadline, and history retention without polluting the per-fire spec. Overdrive should adopt this composition explicitly: a `Schedule` primitive distinct from the `Job` (run-to-completion) primitive.

---

### Synthesis D — Trade-offs of the candidate models for Overdrive

**Model 1: Explicit `kind` discriminator on the existing `Job` aggregate** (k8s shape, scaled to one resource)
- Pros: surgical change to current code (add a discriminator); each variant carries the fields it actually needs; admission validation is a `match` on the variant.
- Cons: the word "Job" is overloaded — k8s "Job" = run-to-completion; Overdrive "Job" today = the single workload primitive. Renaming in flight is awkward.

**Model 2: Separate top-level aggregates** (Cloud Run shape — `Service` vs `Job` vs `Schedule`)
- Pros: strongest type-level guarantee; each aggregate has its own validating constructor; the streaming protocol can have entirely different terminal semantics per aggregate.
- Cons: more API surface; some duplication of spec fields (driver, resources) unless factored carefully; rename of current `Job` is now mandatory.

**Model 3: Two enums on the existing primitive** (systemd / Fly.io shape — `lifecycle: Lifecycle` and `restart: RestartPolicy`)
- Pros: minimal surface; orthogonal axes are explicit.
- Cons: most invalid combinations possible at the type level; operators must learn both axes; the false-positive class the RCA names (no liveness gate) is *not* prevented by this model — both axes are runtime policies, not structural type discriminators.

**Recommendation: Model 2 (separate aggregates)** — see § "Recommendation for Overdrive" below for the concrete enum / field shape. Models 1 and 3 each leave the failure mode partially open; only Model 2 makes the false-positive structurally unrepresentable.

---

### Synthesis E — Pitfalls of retrofitting the distinction onto a single primitive

The current Overdrive shape is the prototype of "retrofit the distinction onto a single primitive": every spec becomes a `Job`, lifecycle is implicit, and failure modes accumulate at the projection boundaries. The four root causes the RCA documents are all *symptoms of this single design gap*:

- **Root cause A** (no liveness gate after fork+exec): a long-running service legitimately reaches `Running` on `fork+exec`; a one-shot script does not. Without a kind discriminator, the action shim cannot pick the right contract.
- **Root cause B** (first-Running == converged): the streaming contract conflates "scheduled" with "stable"; the right semantics depend on whether the kind is service-shaped (stability matters) or job-shaped (terminal exit code is what matters).
- **Root cause C** (edge-triggered, not level-triggered): a level-triggered protocol that watches *the wrong terminal predicate* still gives wrong answers; the predicate itself must be kind-aware.
- **Root cause D** (`"live"` literal): the literal is a category error precisely because the rendering should be kind-shaped — for a service, "took 1.2s to stabilise"; for a job, "exited 0 in 1.2s"; for a schedule, "next fire in 7m22s".

Other systems' bug histories show the same pattern: Nomad's early `service`-only days, before `batch` was added, produced exactly this class of "service jobs that exit cleanly are misreported." k8s's early Job controller had the same issue before `restartPolicy: Always` was rejected at admission. **The structural fix is the same in every case: lift the lifecycle shape into the type system.**

---

## Recommendation for Overdrive

This section is the load-bearing deliverable. It proposes concrete Rust enum and field names; it does NOT modify production code. All citations to existing Overdrive code refer to files the researcher read at HEAD `00dd9d7`.

### R1. Adopt three top-level aggregates: `Service`, `Job`, `Schedule`

Replace today's single `Job` aggregate (`crates/overdrive-core/src/aggregate/mod.rs:92-102`) with three intent-side aggregates. Each carries its own validating constructor; each composes onto the existing `WorkloadDriver` enum (which is already factored correctly per ADR-0031 Amendment 1).

```rust
// crates/overdrive-core/src/aggregate/mod.rs (sketch — RESEARCH ONLY, NOT TO BE LANDED)

/// Long-running workload — desired-replicas-of-an-eternal-process.
/// Reaches `Running` and stays there. Exit is always a failure event.
/// Equivalent to k8s Deployment, Nomad `service`, Cloud Run service.
pub struct Service {
    pub id: ServiceId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    pub driver: WorkloadDriver,
    pub restart: RestartPolicy,        // Always | OnFailure
    pub readiness: Option<ReadinessGate>,  // optional probe
}

/// Run-to-completion workload — exit 0 = success, exit non-zero = failure.
/// Equivalent to k8s Job, Nomad `batch`, Cloud Run job.
pub struct Job {
    pub id: JobId,
    pub completions: NonZeroU32,       // how many successful exits required
    pub parallelism: NonZeroU32,       // how many can run concurrently
    pub backoff_limit: u32,            // max retries per task
    pub active_deadline: Option<Duration>,  // wall-clock cap
    pub resources: Resources,
    pub driver: WorkloadDriver,
}

/// Scheduled execution of a `Job` — recurring or one-shot-deferred.
/// Equivalent to k8s CronJob, systemd .timer, Nomad periodic batch.
pub struct Schedule {
    pub id: ScheduleId,
    pub schedule: CronExpr,            // canonical cron expression
    pub job_template: JobTemplate,     // spec emitted on each fire
    pub concurrency: ConcurrencyPolicy, // Allow | Forbid | Replace
    pub starting_deadline: Option<Duration>,
    pub history: HistoryLimits,
}

pub enum RestartPolicy {
    /// Restart on any exit (clean or failed).
    Always,
    /// Restart only on non-zero exit (clean exit = honest stop).
    OnFailure,
}

pub enum ConcurrencyPolicy { Allow, Forbid, Replace }
```

The naming follows established industry vocabulary verbatim — `Service`, `Job`, `Schedule`, `RestartPolicy`, `ConcurrencyPolicy`, `completions`, `parallelism`, `backoff_limit`, `active_deadline`. This is deliberate: an operator coming from k8s, Nomad, ECS, Cloud Run, or systemd must read the spec and have *zero ambiguity*. **Inventing new vocabulary here would be wrong.**

### R2. Constraint: `Service` cannot reach `Terminated`; `Job` cannot reach `ConvergedRunning`

The streaming protocol's terminal events become **kind-aware**:

- A `Service`'s only terminal-or-stable event is `ConvergedRunning` (after a stability window — solves RCA root cause B/C). It can never produce a "task succeeded" event because no Service task is supposed to exit cleanly; if it does, that's a `Failed` observation row.
- A `Job`'s only terminal events are `Completed { exit_code: 0 }` and `Failed { exit_code: N != 0, reason }`. It can never produce `ConvergedRunning` — running is transient for a job, not terminal.
- A `Schedule`'s terminal event is the spawned `Job`'s terminal event, surfaced via the wrapper.

This is enforceable in the type system via separate `SubmitEvent` enums per aggregate kind (`ServiceSubmitEvent`, `JobSubmitEvent`, `ScheduleSubmitEvent`) — the CLI handler at `crates/overdrive-cli/src/commands/job.rs:490-520` becomes three distinct handlers instead of one polymorphic one. The mistaken `format_running_summary("…", 1, 1, "live")` cannot be reached for a Job kind because the call site does not exist on the Job-side stream.

This **structurally fixes RCA root causes A, B, C, D in one move**: the action shim's "write Running on `Ok(_handle)`" path is only reachable in the Service code path; the Job code path waits for the exit watcher and writes `Completed` / `Failed` based on exit code. The "took live" literal disappears because the Job render line says `exited 0 in 1.2s` with a real duration.

### R3. Composition with Workflows / Reconcilers (whitepaper §18)

The new aggregates compose onto existing infrastructure cleanly:

- **`Service`** is reconciled by the existing `JobLifecycle` reconciler (rename to `ServiceLifecycle`). Reconciler stays unbounded per whitepaper §18; nothing changes about the reconciler primitive itself.
- **`Job`** gets a new `JobLifecycleReconciler` whose ESR specification is *progress to terminal Completed-or-Failed within `backoff_limit + active_deadline`* — a different stability predicate than Service's. The reconciler is still unbounded (it handles many `Job` instances over time); each per-Job lifecycle is bounded.
- **`Schedule`** is a reconciler that emits `Action::SubmitJob { spec: JobSpec, … }` on each fire. The schedule reconciler watches `Cron::next_fire(now)` and gates by `ConcurrencyPolicy` against the ObservationStore's record of in-flight Job runs.
- **Workflows** remain orthogonal — a workflow may emit `Action::SubmitJob` or `Action::SubmitService` as part of orchestration. The whitepaper §18 distinction (Workflow = bounded durable orchestration; Reconciler = unbounded convergence) survives.

### R4. Bounded-context placement (whitepaper §18, ADR layout)

The new types belong in:

- **Aggregates** — `crates/overdrive-core/src/aggregate/{service.rs,job.rs,schedule.rs}` — split the existing `mod.rs` per ADR-0011 ("intent-side aggregates live here"). `Investigation` and `Policy` stubs continue alongside.
- **IDs** — `crates/overdrive-core/src/id.rs` already carries `JobId`. Add `ServiceId`, `ScheduleId` newtypes following the existing `validate_label` convention; the existing `JobId` migrates to denote run-to-completion only.
- **IntentKey** — extend `crates/overdrive-core/src/aggregate/mod.rs:456-504` with `IntentKey::for_service`, `IntentKey::for_schedule` mirroring the existing `for_job` shape; canonical paths `services/<id>`, `jobs/<id>`, `schedules/<id>`.
- **ADR record** — new ADR superseding (or amending) ADR-0031. Working title: "ADR-00XX: Workload kind discriminator — Service / Job / Schedule as separate aggregates." This is exactly the kind of architectural decision that cannot be inlined into existing artefacts; per the user's memory note "Always dispatch to architect for DESIGN artifacts," it must go through `@nw-solution-architect`.

### R5. Migration shape

Greenfield discipline (memory: "Single-cut migrations in greenfield") applies — no deprecation period, no compat shim:

1. Rename current `Job` → `Service` repository-wide as a mechanical sed.
2. Introduce the new `Job` (run-to-completion) and `Schedule` aggregates.
3. Update the streaming protocol's `SubmitEvent` to be kind-aware (three enums or one with a kind tag).
4. Update the action shim to take separate code paths for `ServiceLifecycle` and `JobLifecycle`.
5. Delete the coinflip false-positive integration test fixture and replace with the kind-correct shape: a `Job` spec where `exit_code: 1` produces `Failed` and `exit_code: 0` produces `Completed`.

The "delete and replace, don't gate" principle from `.claude/rules/development.md` § "Deletion discipline" governs the migration: there is no shim primitive bridging old and new; the old `Job` becomes the new `Service` *only* where the workload is genuinely service-shaped, and any spec that was a misnamed one-shot becomes a proper new-shape `Job`.

### R6. What this does NOT change

To bound scope:

- **The `Driver` trait** at `crates/overdrive-core/src/traits/driver.rs` (per ADR-0030 §1) is unchanged. `Driver::start` still returns `Result<AllocationHandle>`; what changes is what the *caller* (action shim) does on `Ok`. This preserves the well-tested per-driver implementations.
- **The `WorkloadDriver` enum** at `crates/overdrive-core/src/aggregate/mod.rs:124-129` (Exec / future MicroVm / future Wasm) is unchanged. Driver class is orthogonal to lifecycle kind — a Wasm workload can be either a Service or a Job. This is consistent with how every surveyed system factors them.
- **The cgroup hierarchy**, the eBPF dataplane, the mTLS surface, the gateway — none of these care about the lifecycle distinction. They operate on `AllocationId` and the underlying driver. The kind discriminator lives entirely in the intent-side aggregate and the streaming/render layers.

### R7. Liveness-window concession (RCA root cause A)

Even after R1–R6, the question of "should `Service.start` settle for N ms before declaring Running" remains a separate decision. The recommendation here is: **yes, but the settle window's *length* depends on the kind**, not its existence. For a Service, the settle window prevents flap-restart-flap; for a Job, the settle window is irrelevant — exit is the truth signal. Encode the settle as a Service-only field (`min_ready_seconds` per k8s Deployment), absent from `Job`. This is the most operator-recognisable shape.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Kubernetes Pod Lifecycle | kubernetes.io | High (1.0) | official | 2026-05-09 | Y |
| Kubernetes Job | kubernetes.io | High (1.0) | official | 2026-05-09 | Y |
| Kubernetes CronJob | kubernetes.io | High (1.0) | official | 2026-05-09 | Y |
| Nomad Job stanza | developer.hashicorp.com | High (1.0) | official | 2026-05-09 | Y |
| Nomad Schedulers | developer.hashicorp.com | High (1.0) | official | 2026-05-09 | Y |
| Amazon ECS Services | docs.aws.amazon.com | High (1.0) | official | 2026-05-09 | Y |
| Amazon ECS Clusters | docs.aws.amazon.com | High (1.0) | official | 2026-05-09 | Y |
| Google Cloud Run Overview | docs.cloud.google.com | High (1.0) | official | 2026-05-09 | Y |
| systemd.service(5) | manpages.debian.org / freedesktop.org | High (1.0) | official | 2026-05-09 | Y |
| Cloudflare Workers / DOs / Workflows / Cron Triggers | developers.cloudflare.com | High (1.0) | official | 2026-05-09 | Partial (vendor docs only; no fresh fetch this session) |
| Temporal & Restate workflow framing | docs.temporal.io / docs.restate.dev | High (1.0) | official | prior knowledge | N (whitepaper §18 internal cross-ref only) |
| Fly.io Machines | fly.io/docs | High (1.0) | official | prior knowledge | N (Overdrive memory note cross-ref only) |
| Overdrive whitepaper | local repo | High (1.0) | internal SSOT | 2026-05-09 | n/a |
| Overdrive aggregate code | local repo | High (1.0) | source code | 2026-05-09 | n/a |
| Coinflip RCA | local repo | High (1.0) | analysis artefact | 2026-05-09 | n/a |

**Reputation summary**: 15/15 sources at High (1.0). Avg reputation: 1.0. All vendor primary docs except where explicitly marked.

## Knowledge Gaps

### Gap 1: Cloudflare and Fly.io primitives not freshly WebFetched
**Issue**: Findings 7 (Cloudflare) and 9 (Fly.io) rely on prior-knowledge synthesis cross-referenced against Overdrive memory notes, not fresh WebFetch against vendor docs in this research session.
**Attempted**: Did not fetch developers.cloudflare.com/{workers,durable-objects,workflows,workers/configuration/cron-triggers/} or fly.io/docs/machines/ in this session due to turn-budget discipline.
**Recommendation**: Before landing the recommendation as an ADR, dispatch a follow-up *verify-sources* sweep against the four Cloudflare primitive pages and the Fly.io Machines API page. The substantive claims (separate primitives per lifecycle shape; `restart` policy enum on Machines) are well-established in widely-published material, but the citation should be primary-source for the ADR.

### Gap 2: Restate / Temporal workflow surface not directly fetched
**Issue**: Finding 8 relies on prior knowledge of these systems and the Overdrive whitepaper §18's own framing.
**Attempted**: Did not fetch docs.temporal.io or docs.restate.dev this session.
**Recommendation**: The whitepaper §18 is the SSOT for Overdrive's workflow shape; the recommendation does not require Temporal/Restate primary-source citation to land. If the ADR explicitly compares Overdrive's workflow primitive to Temporal/Restate, fetch then.

### Gap 3: ESR / "stable Running" predicate semantics
**Issue**: The recommendation defers the *exact* shape of the "Service stability window" predicate to a separate ADR-0033 amendment (referenced in the RCA). This research did not exhaustively survey how Cloud Run's `minInstances`, k8s's `minReadySeconds`, Nomad's `min_healthy_time`, or systemd's `RestartSec=` define stability — each has its own subtle definition.
**Attempted**: Captured each in the Synthesis A failure-mode matrix at a high level.
**Recommendation**: The follow-up ADR work for R1 (kind discriminator) should commission a focused sub-research on stability predicates specifically.

## Conflicting Information

### Conflict 1: "service exit = failure" (Nomad) vs "service crash = restart per restartPolicy" (k8s)
**Position A** (Nomad): *"If a service task exits it is considered a failure"* — even a clean exit code 0 from a `service` job is an unexpected event. — Source: developer.hashicorp.com/nomad/docs/concepts/scheduling/schedulers, High.
**Position B** (Kubernetes): A `restartPolicy: Always` Pod restarts the container regardless of exit code (clean or failed); there is no "clean exit is acceptable for a service." Both behaviorally similar but Kubernetes does not explicitly call clean exit a "failure" — Source: kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/, High.
**Assessment**: Behaviorally these are the same — clean exit triggers restart in both — but the *vocabulary* differs. Nomad calls it a failure for accounting/alerting; k8s calls it a restart event. The Overdrive recommendation should align with Nomad's vocabulary (clean exit on a `Service` is a failure event) because it produces louder operator signal. This is a consistent reading.

## Recommendations for Further Research

1. **Stability predicate survey** (per Gap 3) — focused dive on `minReadySeconds` / `min_healthy_time` / `RestartSec=` to inform R7's settle window.
2. **Schedule expression syntax** — should Overdrive adopt Vixie cron, the systemd `OnCalendar=` shape, or a typed `ScheduleExpr` enum (every-N-seconds, hourly, daily, custom-cron)? Inform R1's `CronExpr` field.
3. **WASM Workflow ABI compatibility** — when the WASM Workflow SDK ships per whitepaper §18, the `Job` aggregate's spec must be representable across the WASM ABI. Check that the recommended shape composes cleanly.
4. **Fresh WebFetch** of Cloudflare and Fly.io primitive docs (Gap 1) before landing the ADR.

## Full Citations

[1] Kubernetes Authors. "Pod Lifecycle". kubernetes.io. https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/. Accessed 2026-05-09.
[2] Kubernetes Authors. "Jobs". kubernetes.io. https://kubernetes.io/docs/concepts/workloads/controllers/job/. Accessed 2026-05-09.
[3] Kubernetes Authors. "CronJob". kubernetes.io. https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/. Accessed 2026-05-09.
[4] HashiCorp. "Job specification — `job` block". developer.hashicorp.com. https://developer.hashicorp.com/nomad/docs/job-specification/job. Accessed 2026-05-09.
[5] HashiCorp. "Nomad scheduler types". developer.hashicorp.com. https://developer.hashicorp.com/nomad/docs/concepts/scheduling/schedulers. Accessed 2026-05-09.
[6] AWS. "Amazon ECS services". docs.aws.amazon.com. https://docs.aws.amazon.com/AmazonECS/latest/developerguide/ecs_services.html. Accessed 2026-05-09.
[7] AWS. "Amazon ECS clusters". docs.aws.amazon.com. https://docs.aws.amazon.com/AmazonECS/latest/developerguide/clusters.html. Accessed 2026-05-09.
[8] Google Cloud. "What is Cloud Run". docs.cloud.google.com. https://docs.cloud.google.com/run/docs/overview/what-is-cloud-run. Accessed 2026-05-09.
[9] systemd Authors. "systemd.service(5)". manpages.debian.org. https://manpages.debian.org/bookworm/systemd/systemd.service.5.en.html. Accessed 2026-05-09. Cross-ref: https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html.
[10] Cloudflare. "Workers / Durable Objects / Workflows / Cron Triggers". developers.cloudflare.com. (See Gap 1 — cited from prior knowledge & vendor docs at https://developers.cloudflare.com/workers/, /durable-objects/, /workflows/, /workers/configuration/cron-triggers/.)
[11] Temporal. "Workflows / Activities". docs.temporal.io. (See Gap 2 — cited from whitepaper §18 internal cross-ref.)
[12] Fly.io. "Machines". fly.io/docs. (See Gap 1 — cited from Overdrive memory note "Leaning toward Cloudflare primitives", 2026-04-20.)
[13] Overdrive. "Whitepaper § 18 — Reconciler and Workflow Primitives". Local repo. `docs/whitepaper.md:1962-2076`. Accessed 2026-05-09.
[14] Overdrive. "Aggregate module". Local repo. `crates/overdrive-core/src/aggregate/mod.rs:1-505`. Accessed 2026-05-09.
[15] Rex (RCA agent). "RCA — coinflip submit reports running on exit 1". Local repo. `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`. Accessed 2026-05-09.

## Research Metadata

Duration: ~45 min agent wall-clock. Examined: 15 sources. Cited: 15 sources. Cross-references performed: 9 inter-finding. Confidence distribution: High 13/15 (86.7%), Medium-High 2/15 (Cloudflare/Fly.io findings — see Gap 1). Output: `docs/research/platform/workload-type-taxonomy-research.md`.
