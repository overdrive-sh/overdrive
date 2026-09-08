# E09-v2 concurrency and timing investigation

Date: 2026-09-08. Diagnosis only, against HEAD `c28a5f94da5fc2a5823aeaa2fa4cdc5173dc976b` plus the retained dirty diff. No suite, build, product command, cleanup, or lease acquisition was performed for this investigation. Existing local captures and read-only SSH inspection were used.

## Conclusion

The run really used one product build invocation, one reusable-artifact preparation, one persistent control-plane process, and concurrent workers. It did **not** use a rolling pool: it ran **two complete healthy/failure pairs per lockstep cohort**, including fresh peer VM Jobs and their resource reclamation. Successive cohorts started approximately **116–120 seconds apart**. The evidence does not support attributing that interval to VM boot: the first six healthy startup observations occurred only **2.22–3.48 seconds after worker launch**.

At the effective concurrency of two, the negative peer fixture's intentional ten-second observation window alone imposes at least **500 seconds for 100 successful pairs**, before any other work. Thus this configuration could never satisfy a 360-second whole-command cap. Increasing the cap would not address the throughput mismatch.

There is also a reporting correction: **“six completed within 360 seconds” is not established.** The local command timed out at 15:31:42 UTC, but its remote suite and control plane continued. Six completed results were inspected afterward; cohort 4 began at 15:31:54 and cohort 5 subsequently ran. Exact completion count at the deadline is unrecoverable from the retained evidence.

## Evidence provenance

The canonical [invocation metadata][meta] records 15:25:42–15:31:42 UTC, exit 124. [Product output][out] records local xtask compilation 0.18s, one bootstrap/rsync/preflight, native product compilation 0.19s, and reusable artifacts prepared. It contains no final timing report.

The raw native output root was `/srv/vm/overdrive-testing/svm-e08-v2`. Its measurement and case files were removed by the prior attempt's cleanup, not by this investigation. Selected contents survive in the external crafter's session capture:

`/Users/marcus/.codex/sessions/2026/09/08/rollout-2026-09-08T17-16-07-01a08197-3d9a-71e1-8127-249338ded23f.jsonl`

In that file, `event_msg / item_completed / CommandExecution / stdout` contains:

- [Line 457][capture-times]: measurement mtimes, six-result count, cohort markers, retained serve-log tail, and worker transcript mtimes; observation completed 15:32:55 UTC.
- [Line 464][capture-results]: actual concurrency rows, results 1–6, and case 7/8 healthy deploy/describe/client outputs; observation completed 15:33:13 UTC.
- [Lines 443 and 450][capture-process]: original suite/control-plane process identities surviving the deadline.
- [Line 478][capture-failure]: case 7/8 failure deploy transcripts, observed after the deadline.
- Lines 499, 523, 539, and 546: cohort 5 process tree; subsequent manual process-group termination; deletion of the marker-owned materialization; final absence checks.

These later inspection commands each invoked xtask again and acquired their own lease. They were **not repeated builds/preparations/control-plane starts inside the original example run**. Their observation timestamps must not be mistaken for event timestamps inside the original six-minute window. No September 7 capture is used as timing evidence for September 8.

## Wall-clock reconstruction

Times below are UTC on September 8. Local metadata is second-resolution; native artifacts provide finer timestamps. Setup estimates assume the recorded local and remote UTC clocks are comparable.

| Boundary | Time | Meaning |
|---|---:|---|
| Outer runner starts | 15:25:42 | Cap starts before transport and preparation |
| Measurements created after preparation | 15:25:53.108 | Bootstrap, sync, checks, build, and preparation finished within approximately the first 11s |
| Serve-start timing record last appended | 15:25:56.486 | Specs materialized and serve readiness check completed |
| Suite baseline finished | 15:25:57.633 | Baseline snapshot itself approximately 1.14s |
| Workers 1/2 launch | 15:25:58.767 | Approximately 16.8s after outer start |
| Workers 3/4 launch | 15:27:54.674 | 115.91s after preceding launch |
| Workers 5/6 launch | 15:29:54.742 | 120.07s after preceding launch |
| Local timeout returns 124 | 15:31:42 | Does not terminate the native suite |
| Cohort 3 store-growth record | 15:31:53.121 | Cohort-level cleanup/accounting continues after cap |
| Workers 7/8 launch | 15:31:54.487 | 119.75s after preceding launch; outside cap |
| Six completed rows inspected | by 15:32:55 | No preserved per-result completion timestamps |
| Workers 9/10 observed | by 15:34:40 | Same suite and control plane still active |
| Explicit termination completes | by 15:35:52 | Historical manual cleanup, not deadline enforcement |

Worker-launch times come from the redirected `transcript.out` files and the [background-launch statement][cohort]; these files receive no ordinary successful worker output. They are launch proxies, not directly instrumented cohort-start events. Each interval already includes overlapping workers plus the intervening cohort cleanup/snapshots; worker durations have not been added together.

The exact preparation duration and serve-start duration were recorded in `measurements/timing.tsv`, but only its mtime survives. The 0.19s cargo result and the approximately 17s until first workers exclude expensive recompilation or preparation as the explanation for minutes per cohort. [Suite setup][setup] calls build, preparation, and serve once, before the loop. Preparation compiles the two guest fixture executables and prepares one reusable rootfs; it does not preboot 400 workload VMs (`prepare.sh:242`).

## Actual concurrency, identity, and critical path

Recovered concurrency rows say `workers=2` for cohorts 1–4. Healthy snapshots report `active_vms_host=2`, `owned_cgroups=2`, and `owned_run_dirs=2`. Failure snapshots report two active VMs in cohort 1 and one in cohorts 2–4; a failed VM exiting does not mean the workers were serial. Result rows 1–6 all identify **control-plane PID 2964664, start ticks 216696280**. Process inspection independently shows the same `overdrive serve` process parented by suite PID 2962502. The original lease-holder PID 2962151 is not the control-plane PID.

The [scheduler][cohort] backgrounds two workers, waits for every healthy-active marker, performs a global snapshot/count, releases both, repeats that barrier for failure-active, joins every worker, snapshots cleanup, and only then starts the next cohort. There is no rolling refill. The [worker sequence][trial] is:

`healthy Service deploy/readiness → healthy cohort barrier → healthy peer VM Job deploy/success/stop/resource cleanup → healthy Service stop/resource check → failure Service deploy/failure observation → failure cohort barrier → negative peer VM Job deploy/success/stop/resource cleanup → failed Service disposal/resource check`.

That is four workload specifications per logical pair, not two bare VM launches. The peer Jobs are created during each pair, not prepared as already-running peers beforehand.

The retained serve events locate the long gaps. For the third cohort, entirely starting before the cap:

| Event / interval | Case 5 | Case 6 |
|---|---:|---:|
| Worker launched | 15:29:54.742 | 15:29:54.742 |
| Healthy guest EXEC released after intercept install | 15:29:56.001 | 15:29:57.177 |
| Healthy startup probe pass recorded | 15:29:57.002 | 15:29:58.179 |
| Healthy peer guest EXEC released | 15:30:02.790 | 15:30:04.202 |
| Failure Service guest EXEC released | 15:30:52.383 | 15:30:53.670 |
| Gap from healthy peer EXEC to failure Service EXEC | 49.59s | 49.47s |
| Negative peer guest EXEC released | 15:31:01.777 | 15:31:02.976 |
| Gap from negative peer EXEC to next cohort launch | 52.71s | 51.51s |

These are elapsed boundaries, **not measured boot durations or exact cleanup durations**. The first long gap includes peer execution/completion/stop, healthy Service stop, resource checks, and the beginning of failed-Service deployment. The second includes the intentional negative observation, peer completion/stop, failed-Service disposal, and cohort accounting. The available evidence localizes the bulk of time to these lifecycle sections but cannot divide it accurately among their sub-stages.

As corroboration outside the cap, case 7/8 deploy transcripts report healthy Stable settlement in **2511ms and 2295ms**. Their failure deploy transcripts last approximately **16s and 5s**, respectively, and report three failed startup attempts. These post-cap values are not substituted for missing in-cap timing measurements.

## Costs and uncertainties, kept separate

**Existing deliberate workload costs.** The [negative client][client] uses a ten-second deadline and succeeds only after that window; its TOML supplies no shorter deadline. Healthy startup allows 30 one-second attempts but can succeed immediately; failure startup requires three attempts, with one-second intervals/timeouts. Harness describe polls sleep one second; cohort gates poll 0.2s. The 180s Service/observation, 120s Job-success, 60s terminal/cleanup, and 300s barrier values are upper bounds, not evidence that every call waited that long.

**Existing product lifecycle costs, not newly proven bugs.** [`VmDriver::stop`][stop] deliberately consumes the two-second beacon shutdown deadline, then permits up to ten seconds for VMM exit; [`HostVmm::terminate`][terminate] returns sooner if exit is observed. Terminal VM Job host-resource reclamation has a [30-second resync cadence][reclamation]. The Job stop branch emits `StopAllocation` only for Running rows (`workload_lifecycle.rs:547`); stopping an already-Terminated peer therefore does not itself invoke driver stop through that branch. Waiting for its remaining host resources can encounter the reclamation cadence. These paths are relevant explanations to measure, but no retained per-stop/sweep timing proves how much of the observed 49–53s gaps each consumed. No control-plane ordering defect is promoted here; no new seeded Sim invariant was run or existing failure proof established for a throughput defect.

**Dirty harness change.** The retained diff adds a runtime-resource wait after terminal observation in `stop_workload`: up to 60s, polling scopes/run directories/clone state every second (`run-example.sh:327`, `:698`). This affects both peer stops and the healthy Service stop. Previously the function returned upon observing Terminated, without waiting for those resources. The change also accepts Failed/Stopped terminal states and makes Job-success polling fail early on Failed/Stopped. It does not remove concurrency or repeat preparation. It does put reclamation explicitly on the pair's serial critical path; there is no equivalent before/after timing capture proving its precise regression magnitude.

**Harness overhead and measurement gaps.** Global resource snapshots and extra active-VM counts are synchronous cohort barriers. `probe_hypervisors` scans all `/proc` PIDs using external commands (`run-example.sh:115`); the retained initial snapshot took approximately 1.14s. Repeated snapshot costs are not preserved. Crucially, peer `stop_workload` calls at lines 915 and 979 are outside the per-stage timing wrappers. Gate waits and resource snapshots also lack separate stage timing. Even preserved duration-only `timing.tsv` files would not fully explain overlap or these gaps.

**Configuration, not measured host saturation.** [Capacity selection][capacity] defaults to two above four CPUs/1GiB, statically caps at four, and also caps at half the CPU count. Memory is a minimum threshold, not a per-worker sizing model. Read-only inspection now reports 16 CPUs and approximately 62.4GiB; the run's exact saved capacity row was deleted, so those present-day values are not relabeled as run-time telemetry. Actual two-worker execution is independently proven. This is conservative sizing, not evidence that two exhausts the host or is adequate for the deadline. The recorded command passes only the output-root override; a local concurrency variable is not automatically forwarded by `bootstrap.sh:339`.

At two workers, 50 cohorts × 10s negative observation gives a **500s lower bound**. At the observed approximately 116–120s/cohort, a conditional same-rate projection is approximately 97–100 minutes for 100 pairs, not a validated completion forecast. Four workers would reduce the negative-observation lower bound to 250s, but nothing here proves that all other work could fit in the remaining 110s. The synthetic `test-scheduler.sh` proves surrogate worker overlap/order, not native throughput.

## Cap boundary and precise next step

The [dirty runner][runner] changes 14400s to 360s around the entire local `cargo xtask metal run`, covering transport, lease acquisition, rsync, preflight, build, preparation, serve startup, trial loop, and cleanup. [`bootstrap.sh`][transport] performs the remote run through SSH; its local exit trap releases its lease, not the remote suite/control-plane process groups. The retained post-deadline process tree directly demonstrates the resulting boundary mismatch. Final suite reporting happens during cleanup (`run-example.sh:1211`), and the subsequent historical manual cleanup deleted the raw output root before its timing files were preserved.

The next correction should be narrowly about this runner contract and measurement, with separate authorization before implementation: define whether the 360s requirement covers setup or only trials, ensure that its deadline bounds the actual remote owner and cleanup, and preserve partial results/capacity/identity/timings outside the deletable root before cleanup. Preserve absolute start/end timestamps for peer stops, terminal-to-host-resource release, Service stops, barriers, snapshots, and cohort boundaries. Then establish the throughput budget for the agreed concurrency model; do not assume a larger cap or a change to four workers meets the user's intent.

The genuine missing evidence is the deleted per-case stage timing, exact result completion times at 15:31:42, run-time capacity row, and stop/reclamation boundary timestamps. This prevents a precise product-versus-harness millisecond attribution, but does not prevent the conclusions above. No implementation or workflow resumption is part of this report.

[meta]: /Users/marcus/conductor/workspaces/helios/accra/verification/expectations/E09-v2-vm-service-tcp-truthfulness-100/evidence/product-run.meta:4
[out]: /Users/marcus/conductor/workspaces/helios/accra/verification/expectations/E09-v2-vm-service-tcp-truthfulness-100/evidence/product-run.out:1
[runner]: /Users/marcus/conductor/workspaces/helios/accra/verification/expectations/E09-v2-vm-service-tcp-truthfulness-100/runner.sh:25
[capture-times]: /Users/marcus/.codex/sessions/2026/09/08/rollout-2026-09-08T17-16-07-01a08197-3d9a-71e1-8127-249338ded23f.jsonl:457
[capture-results]: /Users/marcus/.codex/sessions/2026/09/08/rollout-2026-09-08T17-16-07-01a08197-3d9a-71e1-8127-249338ded23f.jsonl:464
[capture-process]: /Users/marcus/.codex/sessions/2026/09/08/rollout-2026-09-08T17-16-07-01a08197-3d9a-71e1-8127-249338ded23f.jsonl:450
[capture-failure]: /Users/marcus/.codex/sessions/2026/09/08/rollout-2026-09-08T17-16-07-01a08197-3d9a-71e1-8127-249338ded23f.jsonl:478
[cohort]: /Users/marcus/conductor/workspaces/helios/accra/examples/service-kind-vm-workloads-v2/run-example.sh:1038
[setup]: /Users/marcus/conductor/workspaces/helios/accra/examples/service-kind-vm-workloads-v2/run-example.sh:1257
[trial]: /Users/marcus/conductor/workspaces/helios/accra/examples/service-kind-vm-workloads-v2/run-example.sh:839
[capacity]: /Users/marcus/conductor/workspaces/helios/accra/examples/service-kind-vm-workloads-v2/run-example.sh:545
[client]: /Users/marcus/conductor/workspaces/helios/accra/examples/service-kind-vm-workloads/client.rs:67
[stop]: /Users/marcus/conductor/workspaces/helios/accra/crates/overdrive-worker/src/vm_driver.rs:1715
[terminate]: /Users/marcus/conductor/workspaces/helios/accra/crates/overdrive-host/src/vmm.rs:534
[reclamation]: /Users/marcus/conductor/workspaces/helios/accra/crates/overdrive-reconcilers/src/vm_reclamation.rs:232
[transport]: /Users/marcus/conductor/workspaces/helios/accra/infra/metal/bootstrap.sh:322
