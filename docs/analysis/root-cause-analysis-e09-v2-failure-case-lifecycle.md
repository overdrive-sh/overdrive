# E09-v2 failure-case lifecycle investigation

<!-- DES-ENFORCEMENT : exempt -->

Investigator: Codex, applying `nw-root-why`, `nw-troubleshooter`,
`nw-investigation-techniques`, and `nw-five-whys-methodology`.

Scope: the native 2026-09-08 19:12:00Z E09-v2 capture at
`b653e1ad1758d11be33be457849b284f78141333`, with the existing dirty
healthy-stop observation wait correction. Analysis and diagnostic tests only.
No production correction, expectation correction, timeout change, or design
amendment is implemented here.

## Finding and classification

The fresh failure is at **c001's failure-probe stage**, after its original
unbound Service deploy has already returned **Timeout at 90 seconds**. The
example subsequently polls a restarted `Running` allocation, but polling cannot
change that closed deploy transcript into `StartupProbeFailed`. The healthy
stop correction worked: all ten healthy Services reached a terminal state and
zero runtime resources; the slowest healthy stop took 130,548 ms.

Two distinct example problems are reproduced:

1. Each worker starts its failure deployment immediately after its own healthy
   cleanup, while other workers' healthy stops still occupy the current serial
   convergence path. A seeded production-owner composition reproduces the
   resulting 90-second stream Timeout. Moving submission after those stops
   produces the required typed startup failure with the same owners and inputs.
2. Independently, the failure predicate accepts a restarted `Running` allocation
   only when its previous failure contains a particular beacon-bind error.
   Production permits recovery without that error. Executing the actual
   predicate on native typed-failure/recovery output reproduces this rejection.

**Classification:** the native run violates S-SVM-25's example acceptance
contract; its original timeout must remain a failure. The beacon-bind clause
also incorrectly narrows valid product behavior. Neither a documented stream
Timeout nor automatic same-ID Service recovery is, by itself, a proven
production API contract violation. This investigation does not reopen serial
dispatch remediation #283 or the disproven old cleanup-handoff interpretation.

## Fresh evidence and provenance

All native references below are in
`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/`.
`verification.yaml` records native-metal, seed 1, the SHA above, dirty=true,
and runner exit 1. `product-run.meta` records 19:12:00Z–19:18:37Z and exit 1
under the existing 600-second remote owner budget plus 60-second cleanup grace.
`run.log` records the exclusive lease and canonical fail-closed preflight.
The native command was `verification/harness/run-expectation.sh E09-v2`.
This investigation reads that retained run; it does not claim a new native run.

| Native observation | Evidence in `run.log` | Meaning |
| --- | --- | --- |
| Healthy stop durations grow from 12,591 to 130,548 ms; healthy Services terminate and release resources | timing sections at lines 169–256; each healthy-stop/healthy-resources-after section | The fresh stop observation window succeeds; this is not another 60-second healthy-stop rejection. |
| c001 failure deploy starts 19:12:57Z, exits 19:14:27Z with code 1 and “workload did not converge within 90s” | lines 464–472 | Original terminal output is Timeout, not StartupProbeFailed. |
| c001's first failure guest EXEC is released at 19:14:24.015Z | lines 311–312 | Almost the entire original operator wait has elapsed before the guest is released. |
| c001 later has `Running`, restart count 1, previous `Failed` at counter 70, reason `driver started`, TCP 18999 failure | lines 473–494; another EXEC release at line 353 | Recovery occurred. Human describe does not expose the typed terminal in this history line. |
| c004 deploy returns typed startup failure in 85,809 ms and its saved failure describe is Failed | timing at lines 205–213; original c004 deploy at 879–887; Failed describe at 888–900 | Later submission can retain the required original typed result under the same server. |
| c006 original deploy returns code 1 with startup probe[0] failed after 3 attempts; later same-Service describe is Running/1 with previous Failed/Started and refused 18999 | lines 1143–1169 and 1170 onward | Authentic typed failure and recovery without a beacon-bind diagnostic coexist. |
| c008 original deploy returns typed startup failure; later describe includes previous beacon-bind failure | lines 1417 onward and 1445 onward | The current predicate's special-case recovery trajectory also occurs, but is not universal. |

**Timing identity check (review F-02):** the timing files are concatenated in
lexicographic path order (`run-example.sh`, `find ... -name timing.tsv` followed
by `sort -z`), so the case order is **1, 10, 2, 3, 4, 5, 6, 7, 8, 9**, not
numeric order. The 85,809 ms block at `run.log:205–213` therefore belongs to
**c004**, whose original transcript at 879–887 runs from 19:13:34Z to 19:15:00Z
and reports typed startup failure. **c005** instead takes **73,690 ms**
(`run.log:214–222`); its original transcript at 1011–1019 runs from 19:13:46Z
to 19:15:00Z and its Failed snapshot is at 1020–1032. The preceding 90,121 ms
block belongs to c003, whose Timeout transcript is at 743–751. F-02's proposed
c004-to-c005 relabeling is contradicted by these exact transcripts; the table
retains c004 and now makes its citations explicit.

`failure-describe.out` is a repeatedly replaced polling snapshot, not an
immutable first-attempt state. `failure-final-describe.out` is a separate
post-stop-request snapshot; a stop acknowledgement does not certify that the
allocation has stopped. The c006/c008 recovery fixtures deliberately use those
later snapshots and are labeled accordingly. They are not claimed to be the
original failing c001 predicate inputs. Each `failure-deploy.out` preserves the
original command, timestamp, terminal text and command exit code.

The final ledger's c001 `not-run-cancelled`/`not-started` label cannot establish
that the failure deployment never ran: its original transcript proves it did.
`run-example.sh:811` runs worker cleanup before `write_worker_result` at 836;
`write_worker_result` at 782 prioritizes cancellation, and aggregation at 1110
fills missing results. Supervisor interruption can therefore prevent or replace
the useful per-worker result. This is evidence provenance, not an additional
remediation requirement. The incidental `/proc` awk race and later cancelled
failure-case residue do not explain the immutable c001 Timeout; no causal claim
or fix is made for them here.

## Production entry point, owners, state and ordering

### Original deploy result and the healthy-stop overlap

The checked-in operator journey is `run-example.sh:839` (`run_trial`). It waits
at the healthy-active barrier at 895–897, deploys/checks its peer, and awaits
its own healthy stop and resource release at 918–928. It then immediately
calls failure deploy at 930–944. `run_cohort` releases healthy work at 1079 and
next waits for failure-active at 1081; there is no intervening all-healthy-
cleanup barrier. P4 executes this actual worker prefix and fails on that order.

Public deploy reaches `handlers::submit_workload`
(`crates/overdrive-control-plane/src/handlers.rs:256`), validated intent
admission/write at 454–461, and Service stream construction at 616–627.
Public stop writes the existing stop-intent key and enqueues WorkloadLifecycle
at `handlers.rs:834–891`. Production serve owns the convergence task
(`crates/overdrive-control-plane/src/lib.rs:3066`):
`spawn_convergence_loop` drains pending evaluations at 3360 and awaits each
`run_convergence_tick` serially at 3363–3381. That entry point hydrates,
reconciles, dispatches and persists the participating owner's view
(`reconciler_runtime.rs:1352–1500`).

For a healthy VM stop, action dispatch awaits `VmDriver::stop`. The driver
changes its supervision entry to `EndingInFlight` at
`crates/overdrive-worker/src/vm_driver.rs:1697`, after which status reports
NotFound at 1760. With a beacon writer it waits the existing two-second
shutdown-request deadline at 1715–1735. If the writer stalls, it aborts **that
writer task**, awaits its termination, then proceeds. A quick writer completion
still waits the same deadline. It next awaits `Vmm::terminate` with the
ten-second grace at 1741, then attempts cgroup/scope/run-directory/clone removal
at 1752–1755. The constants are at 68 and 73. This is an awaited driver effect,
not a detached cleanup handoff. The native staircase is consistent with these
serial stop effects; exact syscall durations were not traced in this capture.

The Service stream independently arms its cap when polled after Accepted
(`streaming.rs:886–918`). It projects ServiceLifecycle broadcasts at 919–959,
or publishes the existing Timeout and closes at 973–981. Its cap does not
cancel admission, driver start, probes, reconciliation, or subsequent recovery.
A later lifecycle result cannot alter that already closed stream. P3 proves
this with real stream and lifecycle owners and a selected legal 108-second
remaining-stop partition. It does not assert that the native broker selected
exactly nine stops in one batch: native c001's 87-second pre-EXEC delay and
subsequent work are the observed schedule, while the Sim isolates the sufficient
blocking mechanism. No broker implementation or full-server test seam is needed
to demonstrate that an already selected serial batch can consume the cap.

Shutdown in `spawn_convergence_loop` is checked only at its final select
(`lib.rs:3386`), after its current batch completes. There is no real-owner
task abort in the reproduction. The example's worker cancellation/trap path
requests public stops and can be interrupted by the outer bound; it does not
retroactively cancel the failed original deploy admission.

### Startup failure, successful restart, and the misleading history text

1. Driver start and action dispatch author `Running`; `VmDriver` forwards
   production ProbeRunner start/stop hooks at `vm_driver.rs:1894–1906`.
   `ServiceLifecycle` counts distinct refused observations and requires the
   attempts/deadline/no-pass predicate
   (`crates/overdrive-reconcilers/src/service_lifecycle.rs:526`, 635–648,
   1269–1315). The checked-in failure descriptor has three attempts at one
   second (`examples/service-kind-vm-workloads/tcp-startup-failure.toml:25`).
2. The deciding tick puts backend withdrawal ahead of terminal dispatch
   (`service_lifecycle.rs:652–677`) and records `terminal_announced`. The action
   shim finalizes the allocation Failed with a typed
   `ServiceFailed { StartupProbeFailed }`, while forwarding the prior driver
   reason `Started` (`action_shim/mod.rs:1559`, 1687). These are different fields.
3. Finalization awaits mTLS teardown, structural network release and probe
   retirement, then writes the lifecycle row, releases driver supervision and
   emits the occurrence (`action_shim/mod.rs:1755–1791`). Fallible teardown
   returns before the terminal write; the existing owner/replay rules apply.
   It does not synchronously run the normal VM stop/grace sequence here.
4. Standing Service intent remains. WorkloadLifecycle considers Failed
   restartable, checks its existing attempt budget/backoff and emits
   `RestartAllocation` (`workload_lifecycle.rs:827–968`, 1379). The natural Job
   terminal guard at 803 is not a standing-Service guard. The existing ceiling
   is five (`:32`); there is no new retry policy in this report.
5. Restart uses the **same allocation ID** (`workload_lifecycle.rs:1337–1372`).
   Its stop half awaits the prior driver and accepts typed NotFound as absence;
   other errors return before replacement (`action_shim/mod.rs:2254–2288`).
   Successful start yields Running/Started with no detail at 2463. Mid-budget
   restart does not author a new Service terminal (`:2515`). The production
   `CrashFacts::advance` snapshots the prior terminal and increments the restart
   count (`crates/overdrive-core/src/traits/observation_store.rs:1378`).
6. ServiceLifecycle's existing terminal veto survives same-ID recovery; it does
   not announce Stable for that allocation (`service_lifecycle.rs:503`). P2 and
   P3 verify backend ineligibility after recovery, including repeated ticks in
   P2. This is not a new eligibility defect or permission to rework ADR-0101.
7. `LastTerminated` includes the typed terminal (`observation_store.rs:1225`),
   and describe's API projection carries it verbatim (`handlers.rs:139–146`).
   The CLI's history renderer uses `lt.reason` and optional detail, not
   `lt.terminal` (`crates/overdrive-cli/src/render.rs:489–529`). Thus
   “last terminated: Failed … driver started” does **not** prove the typed
   startup fact was lost. P3 prints the retained typed fact after recovery.

VmReclamation can discard stranded terminal, unsupervised VM artifacts
(`crates/overdrive-reconcilers/src/vm_reclamation.rs:165–174`, resync at 265).
This and same-ID start make beacon-bind outcomes dependent on actual host
artifact ordering. No native syscall capture here establishes which exact
reclamation/bind order produced c001 or c006. That chronology remains a
hypothesis. It is unnecessary to require EADDRINUSE: the native no-bind snapshot
exists, and the seeded driver-independent owner composition produces the same
successful recovery/retained-terminal state without it.

## Five Whys with separate causal branches

### A — Why c001 cannot satisfy the required original failure result

| Why | Answer and evidence |
| --- | --- |
| 1. Why does failure-probe fail? | Both acceptance branches require typed startup failure in the immutable stream (`run-example.sh:636–657`); c001 contains only Timeout (`run.log:464–472`). |
| 2. Why is that stream Timeout? | Its independent 90-second cap wins before a projectable startup terminal; the production stream then closes (`streaming.rs:973`; P3 red invariant). |
| 3. Why can the lifecycle miss that cap? | The failure stream runs while the serial convergence owner awaits other allocations' stop effects (`lib.rs:3363`; P3's real dispatch plus nine delayed driven-port stops). Native guest EXEC is not released until roughly 87 seconds into c001's wait (`run.log:312`). |
| 4. Why does this example admit that overlap? | A worker advances after its own cleanup, without waiting for peers (`run-example.sh:918–944`); P4 fails on the actual call order. |
| 5. Why does that ordering break this acceptance contract? | S-SVM-25 requires every original unbound deployment to report StartupProbeFailed and explicitly rejects timeout (`test-scenarios.md:68`, 86–87), while the current stream API has a bounded independent wait. The existing cohort gates certify active overlap, not the healthy-cleanup/failure-submit boundary (`run-example.sh:1060–1081`). |

Backward validation: the required typed original result needs a lifecycle
terminal before cap; pending serial stops can prevent that; the example allows
them inside the cap. Forward validation: submit amid those stops → cap closes
with Timeout → later valid startup failure/recovery → unchanged predicate
rejects forever. P3's separated control changes only submission order and returns
StartupProbeFailed. Polling longer or accepting no-bind Running alone cannot
repair c001's original timeout.

### B — Why Running without a beacon-bind error is wrongly rejected

| Why | Answer and evidence |
| --- | --- |
| 1. Why can truthful failed startup still be rejected? | The Running branch requires `last terminated … bind beacon listener: Address already in use` (`run-example.sh:649–653`); P1 rejects native c006 recovery. |
| 2. Why is that driver error absent? | Recovery can succeed directly: native c006 is Running/1 with prior Failed/Started, and P2/P3 produce successful same-ID restart with no start-error detail. |
| 3. Why does a startup-failed Service restart? | Standing intent and WorkloadLifecycle's existing restart rules permit it (`workload_lifecycle.rs:827`, 959, 1379); the terminal Job no-op rule does not apply. |
| 4. Why does describe seem to erase the startup reason? | FinalizeFailed carries prior Started separately from the typed terminal; the history renderer prints the former (`action_shim/mod.rs:1687`; `render.rs:498`). P3 verifies the latter remains stored. |
| 5. Why does the oracle depend on an incidental driver failure? | Its comment and predicate encode a particular restart/bind trajectory (`run-example.sh:644–653`; current E09-v2 README) instead of the accepted independent startup-failure, restart and eligibility contracts (ADR-0096:54–76). No evidence establishes a deeper organizational cause. |

Backward validation: accepted typed startup failure does not require a failed
replacement bind; automatic restart and display choices explain the observed
Running/Started text without loss of the startup fact. Forward validation:
startup failure → standing-intent recovery → retained typed terminal but
Started history rendering → unnecessary bind regex fails. P1's Failed and bind
controls pass, its no-bind case fails, and its timeout control still rejects.

## Revalidated design and bounded fix proposal

S-SVM-25 is authoritative for this example, including twenty unique pairs,
concurrency ten, one unchanged server, no retries/discards, original typed
failure, negative peer observation and the existing run bound. Its text calls
this a functional sample, not a throughput guarantee. ADR-0095's reviewed
decision keeps Timeout stream-only and allows late lifecycle convergence
(`docs/product/architecture/adr-0095-service-stream-cap-startup-deadline.md:74–120`;
review in `docs/feature/service-kind-vm-workloads/design/review-adr-0095.md`).
The nominal 30-second margin is not a general promise covering arbitrary queued
stop effects. ADR-0096 explicitly preserves restart authority and same-ID veto;
ADR-0099 concerns restart write acknowledgement, not immutable Service failure.
Current production matches those distinctions. Historical Proposed labels and
obsolete bridge wording do not override the reviewed ADR-0101 sole publisher
now visible at `service_lifecycle.rs:652`.

**Proposed correction, after the reproductions above:**

1. Add one cohort phase barrier in the existing example: each worker announces
   completion only after both healthy peer/Service cleanup and its runtime
   release check; the cohort owner releases failure submission only after all
   ten workers have announced that completion. All ten failure trials then
   start concurrently. Keep the existing active-phase barriers and cancellation
   ownership. This removes the proven healthy-stop backlog from each failure
   stream's budget without changing the server's serial dispatcher, retry
   policy, API, stream cap, assertions, pair count or control-plane identity.
2. Correct the existing failure predicate and its explanatory README/comment
   to accept either current Failed or same-allocation Running with a positive
   restart count and a prior Failed snapshot, without requiring a particular
   driver-start failure. Retain **all** independent evidence: original deploy
   nonzero, its original typed StartupProbeFailed, failed guest TCP 18999 probe,
   no Stable result, and the existing negative peer outcome. This accepts an
   already-proven startup failure across permitted recovery; it does not turn
   timeout, generic deploy error or merely Running into success.

Affected implementation surfaces are the existing example's worker/cohort
coordination and failure predicate, the corresponding explanation in the
example/expectation README, and their bounded host-safe regressions. No new
public method, type, variant, persisted state or control-plane mechanism is
needed for this proposal. No such changes are made by this investigation.

The same **P1 behavioral regression** should become fully green after the
predicate correction, including the still-rejected original c001 timeout.
The same **P4 worker ordering regression** should become green once the worker
actually awaits the new post-cleanup cohort release. The coordinator additionally
needs a bounded behavior test proving it waits for all ten completions and
cancels cleanly on a missing/failed worker; P4 alone cannot certify that half.
P3's separated control is already green and shows why the phase correction
addresses the demonstrated cap collision.

**P3's deliberately overlapping diagnostic remains red with an example-only
fix.** It directly selects the old schedule inside Rust and does not execute
the example. Changing that assertion or claiming the example fix makes it green
would be dishonest. Retain it as a diagnostic counterexample to the forbidden
example schedule, not as a newly promised arbitrary-load production invariant.
Making that overlapping production schedule itself pass would require a
different product contract/remediation, outside this requested proposal and
the explicit exclusion of #283.

Final acceptance still requires the fresh canonical E09-v2 native run with
exclusive lease and preflight, all twenty original pairs, the unchanged
600+60-second envelope, and real negative-peer/runtime observations. The Sim
does not prove native boot distributions, total-run budget, socket deletion,
or wire/kernel cleanup. The proposed barrier removes a proven source of delay;
it does not prove that no other native work can exceed the cap. If that run
still times out, retain its first-attempt result and investigate the new bounded
cause rather than widening the timer or substituting success.

**Specific presentation gap, not a required implementation dependency:**
ADR-0078's exact human-rendering shape omits `last_terminated.terminal`, even
though its model/API retain it (`adr-0078-crash-and-recover-is-durably-observable-last-terminated-plus-restart-count.md:990–1040`;
`render.rs:489`). If operators must see the typed prior Service cause in human
describe after losing the original stream, a separate bounded DESIGN decision
must pin that display change. The current renderer is not proven to diverge
from its accepted shape. The proposed example correction uses the retained
original typed stream and does not depend on inventing that API/display change.

## Diagnostic probes (predictions recorded before execution)

### P1 — Execute the existing failure predicate on native product inputs

- Hypothesis: the current predicate independently rejects a valid startup-failed
  Service that has automatically restarted without an intervening beacon-bind
  failure. Its requirement for a specific driver-start error is narrower than
  the Service startup truthfulness contract.
- Predicted outcome: the captured typed startup-failure stream plus the same
  Service's recovered `Running`, restart-count-positive, failed-probe describe
  is rejected; the captured `Failed` and beacon-bind-recovery controls pass.
  The original c001 generic-timeout stream must still be rejected.
- Falsification: the actual sourced predicate accepts the no-bind recovery,
  or the native inputs do not carry an authentic typed startup-failure stream,
  or the supposedly valid recovery lacks the failed guest-port observation.
- Boundary: behavioral test of the example's actual shell function, using
  retained native product output. No binary, VM, test-only lifecycle row, or
  production effect is created. Only the polling clock/query adapter is replaced.

### P2 — Replay the existing seeded production-owner recovery composition

- Hypothesis: `Running` after startup failure is an ordinary same-ID recovery,
  and the startup terminal remains available in the production-authored
  `last_terminated.terminal`; a beacon-bind rejection is not required.
- Predicted outcome: registered WorkloadLifecycle and ServiceLifecycle plus
  production ProbeRunner/action dispatch produce `Failed(ServiceFailed /
  StartupProbeFailed)` followed by same-ID `Running` with one restart, a
  retained terminal snapshot, and continued backend ineligibility.
- Falsification: production does not restart, loses the terminal snapshot,
  or makes the backend eligible despite the unchanged terminal veto.
- Boundary: existing `overdrive-sim` diagnostic, seed 257209. It uses a simulated
  process driver and real participating control-plane owners, so it establishes
  driver-independent recovery/eligibility semantics, not Unix socket cleanup.

### P3 — Reproduce the E09 stream result while healthy stops remain in flight

- Hypothesis: starting the failure stream while the current serial dispatch
  batch still contains healthy VM stops consumes the stream's 90-second budget
  before ServiceLifecycle can publish its startup failure. The E09 worker starts
  this stream after its own healthy stop, without waiting for its peers' stops.
- Predicted outcome: nine remaining 12-second stop effects, driven through
  WorkloadLifecycle and the action shim, yield a stream Timeout even though
  the subsequently started unbound Service reaches StartupProbeFailed. The
  same owners/stop effects with failure submission after those stops yield
  StartupProbeFailed on the original stream. Both continue to Running recovery
  without a beacon-bind error and preserve the typed failure snapshot.
- Falsification: the overlapping stream reports StartupProbeFailed within the
  existing cap, the lifecycle does not reach its startup terminal, or the
  separated control also times out.
- Boundary: a seeded Sim driven-port stop delay models the native 2s request
  window plus up-to-10s VMM grace. Registered production owners, action dispatch,
  ProbeRunner, observation store, and production Service stream participate.
  The bounded test invokes the existing per-tick entry point in the serial
  order already drained by production; it does not claim to test broker
  selection, native stop-duration distribution, or every full-loop schedule.
  The invariant is E09's stronger sample obligation, not a claim that a
  documented stream Timeout violates the streaming API contract.

### P4 — Observe the actual worker's phase boundary

- Hypothesis: after its own healthy cleanup, `run_trial` can enter failure
  deployment without awaiting a cohort gate that can certify its peers have
  finished healthy cleanup.
- Predicted outcome: a host-safe execution of the actual worker prefix records
  failure submission with no intervening cohort gate; the behavioral invariant
  fails. This isolates the example ordering that selects P3's red schedule.
- Falsification: the actual worker waits at a cohort gate after its healthy
  cleanup and before calling failure deploy.
- Boundary: shell orchestration only. External product adapters return the
  healthy-success control and the probe ends at the failure-deploy boundary.
  It does not model VM state or claim the coordinator implements a correct new
  barrier. That coordinator behavior remains a required check for a later fix.

## Executed reproductions and results

All commands below run from the repository root. Diagnostic tests are
intentionally failing against the current implementation; they are not a claim
of DELIVER RED/GREEN phases or a green suite. No production process is spawned
by these tests. No terminal row is seeded and no owner is forcibly aborted.

### P1 — Actual predicate, native product-derived inputs

Artifact: `docs/analysis/test-e09-v2-failure-lifecycle.sh`.

```sh
bash docs/analysis/test-e09-v2-failure-lifecycle.sh
```

Exit **1**; reproducible output:

```text
failed_control: expected=accepted observed=accepted
bind_recovery_control: expected=accepted observed=accepted
original_c001_timeout_control: expected=rejected observed=rejected
no_bind_recovery_regression: expected=accepted observed=rejected
```

The function under test is sourced verbatim from the current example. The
native stream/describe excerpts are embedded so deleted remote materialization
or later evidence replacement cannot silently change the reproduction. Only
query delivery and the polling clock are replaced. This deterministic shell
predicate has no scheduling seed; the control-plane ordering claim is tested
separately with Sim below.

### P2 and P3 — Seeded real-owner composition

New artifact:
`crates/overdrive-sim/tests/e09_v2_failure_stream_overlap_spike.rs`.
Existing control, executed unchanged:
`crates/overdrive-sim/tests/e09_v2_failed_service_reachability_spike.rs`.

Exact final verification command:

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-sim \
  --features integration-tests,overdrive-control-plane/integration-tests \
  --test e09_v2_failed_service_reachability_spike \
  --test e09_v2_failure_stream_overlap_spike --no-capture
```

Exit **1** from xtask; underlying nextest exit **100**. Nextest run ID
`c947d912-723a-467a-880e-2a0fdb871c1f`: **3 tests, 2 passed, 1 failed, 0 skipped**.
The fresh spike was also run alone before formatting with the same red/control
outcomes. Printed seed and decisive final output:

```text
e09-v2-backend-veto seed=257209
terminal_startup_veto_converges_after_same_id_restart ... ok

e09-v2-failure-stream seed=257210 overlap=false
stream=Failed { alloc_id: Some("alloc-e09-v2-failure-stream-0"),
  reason: StartupProbeFailed { probe_idx: 0, last_fail: "connection refused",
  attempts: 3 }, stderr_tail: None }
completed_healthy_stops_control_reports_startup_failure ... ok

e09-v2-failure-stream seed=257210 overlap=true
authored=Some(ServiceFailed { reason: StartupProbeFailed { probe_idx: 0,
  last_fail: "connection refused", attempts: 3 } })
recovered_state=Running
recovered_last_terminal=Some(ServiceFailed { reason: StartupProbeFailed {
  probe_idx: 0, last_fail: "connection refused", attempts: 3 } })
stream=Failed { alloc_id: None, reason: Timeout { after_seconds: 90 },
  stderr_tail: None }
seed=257210: E09 requires StartupProbeFailed on the original unbound deploy;
got Failed { alloc_id: None, reason: Timeout { after_seconds: 90 },
  stderr_tail: None }
overlapping_healthy_stops_preserve_e09_startup_failure_result ... FAILED
```

Output is line-wrapped here for readability. The failing assertion is at the
new spike's line 320. The test prints each of nine completed healthy-stop
graces before inspecting the real lifecycle and stream results.

Composition: real registered WorkloadLifecycle and ServiceLifecycle, production
per-tick hydration/reconciliation/action dispatch/view persistence, validated
Service inputs and existing intent store/allocator, real ProbeRunner, production
Service stream, and existing seeded Sim observation/clock/driver/prober ports.
VmReclamation is registered as in the existing composition but is not scheduled:
the asserted consequence is driver-independent, not a Unix pathname race. The
private Driver wrapper models only the legal awaited healthy-stop delay and
forwards existing probe lifecycle hooks. The admitted failure starts and all
Failed/Running/history/backend rows are authored by production. The seed is
fixed and printed; this is a bounded seeded schedule, not a claim of randomized
schedule coverage or native VM execution in Sim.

Initial diagnostic fixture work encountered a compile error in test-only intent
construction and an overbroad delay selector that also delayed recovery. Those
fixture attempts are not failure evidence; the selector now delays only the
nine healthy allocations. The completed results above are after those fixture
corrections and targeted rustfmt. No production behavior was altered to obtain
the failure.

### P4 — Actual worker submission order

Artifact: `docs/analysis/test-e09-v2-failure-submit-order.sh`.

```sh
bash docs/analysis/test-e09-v2-failure-submit-order.sh
```

Exit **1**:

```text
failure-submit: own_cleanup=1 cohort_gate_after_cleanup=0
FAIL: failure submit is reachable before a post-cleanup cohort gate
```

The sourced `run_trial` executes its real healthy-prefix control flow. Private
external adapters supply the healthy successful phase and record the next
failure-submit call; exit 42 ends only the probe subprocess at that boundary.
This is not a forced production future abort. P4 isolates worker order, while
P3 supplies the required production-owner timing invariant; neither is an
expectation runner or native health claim.

## Handoff

Written by this investigation: this RCA and the three diagnostic test files
listed above. Production files, existing example assertions/timeouts, accepted
design, roadmap, DES records and pre-existing dirty work were preserved. No
commit, issue, native E10/E13 run or implementation fix was made. The report is
ready for the independent troubleshooter review requested by the orchestrator.

## Follow-up — post-fix disposal failure, 2026-09-08 20:14Z capture

This section concerns the later native capture with the approved healthy-cleanup
barrier and failure-predicate correction present. Earlier sections retain their
original capture identity and diagnosis; the evidence directory has since been
overwritten by this normal rerun. No archive or new native execution is created
by this follow-up. Current evidence metadata identifies start 20:14:18Z, finish
20:19:59Z, unchanged SHA `b653e1ad1758d11be33be457849b284f78141333`, dirty=true,
native seed 1 and exit 1. The new operator failure is at `failure-stop` for
c008–c010; c001–c007 complete, and c011–c020 are not run.

### P5 — Test whether the retained failed cases' disposal states are rejected

Recorded before execution:

- Hypothesis: the currently retained c008–c010 post-stop descriptions fail an
  overly narrow `dispose_failed_service` state/history predicate.
- Predicted outcome if that hypothesis is true: the actual sourced helper
  rejects those descriptions with their authentic pre-stop failure observations
  and successful AlreadyStopped responses.
- Falsification: the helper accepts all three. This would show that the retained
  final snapshots cannot be the rejecting original inputs and would require
  distinguishing the initial disposal call from its EXIT-trap retry.
- Boundary: deterministic shell helper execution using embedded excerpts from
  this fresh native run. Only the external stop/query adapters and polling clock
  are replaced; no lifecycle rows, VM behavior or control-plane schedule are
  fabricated. No Sim seed is applicable to this string-predicate falsification.

### P5 result: the retained final states are accepted, not rejected

New standalone diagnostic: `docs/analysis/test-e09-v2-disposal-evidence.sh`.
It does not edit or depend on changes to the crafter-owned P1/P4 diagnostics.

```sh
bash docs/analysis/test-e09-v2-disposal-evidence.sh --test-retained-rejection-hypothesis
```

Exit **1**, falsifying the proposed retained-state rejection:

```text
c008 retained disposal: expected=rejected observed=accepted
c009 retained disposal: expected=rejected observed=accepted
c010 retained disposal: expected=rejected observed=accepted
```

The control command without that flag exits **0**, with all three
`expected=accepted observed=accepted`. The deliberately failing hypothesis
command is **not a reproduced product defect** or a failing regression that
authorizes changing the disposal predicate. It establishes the opposite:
these retained inputs already pass the actual helper. The fixture includes
the original nonzero typed failure streams and authentic pre-stop describe
files; no terminal text is synthesized or relabeled.

### Fresh original results versus overwritten disposal evidence

All references in this subsection refer exclusively to the **20:14Z** `run.log`.

| Fact | Exact fresh evidence | Consequence |
| --- | --- | --- |
| All ten first-cohort failure deployments now produce original typed startup failure in roughly 13.5–15.3 seconds | timing blocks 98–207; corresponding immutable original deploy transcripts | The approved barrier resolves the earlier captured original-stream timeout in this cohort. |
| c008 original failure deploy exits 1 with startup probe[0] failed after three attempts | 1571–1579; 20:17:05–20:17:20 | This startup evidence was not lost. |
| c009 original failure deploy has the same typed terminal and exit 1 | 1732–1740; 20:17:05–20:17:20 | Same distinction. |
| c010 original failure deploy has the same typed terminal and exit 1 | 1893–1901; 20:17:05–20:17:21 | Same distinction. |
| Saved pre-stop failure observations are Failed/0, reason Started, failed guest TCP 18999 | c008 1580–1592; c009 1741–1753; c010 1902–1914 | These snapshots precede the negative peer window; they are not the states at the rejected disposal poll. |
| All three negative peer Jobs are Succeeded | c008 1601–1608; c009 1762–1769; c010 1923–1930 | The new failure occurs after the negative peer check. |
| All three retained failure-stop responses say AlreadyStopped/no-op | c008 1620–1622; c009 1781–1783; c010 1942–1944 | These files contain a repeated stop response, not evidence of the original stop command's outcome. |
| All three retained final descriptions are Terminated/1 at counter 99, current reason stopped, previous Failed with bind detail | c008 1623–1638; c009 1784–1799; c010 1945–1960 | The actual helper's Terminated/stopped branch accepts them, independently of the human previous-failure reason. |
| c001–c007 pass/complete; c008–c010 fail/failure-stop; c011–c020 are not run | ledger 72–91 | Earlier completed cases are not cancelled. The saved Failed state in the last three ledger rows is a stale pre-stop worker variable. |
| Final cleanup reports VM count zero and no run-directory/clone entries, with 204 host cgroups remaining | final cleanup section beginning 1995 | No evidence of a new live-VMM leak is shown. The skipped per-case post-disposal assertions for c008–c010 cannot be stamped successful from this aggregate alone. |

The production helper at `run-example.sh:729–759` first runs a bounded
45-second public stop command, returning its nonzero exit code immediately if
it fails. Otherwise it polls for at most **60 seconds**. Its Terminated branch
at 751–755 requires current `reason: stopped` and the **earlier** failed startup
observation in `failure-describe.out`; it does not require the current or
previous human reason to say StartupProbeFailed. Its separate current-Failed
branch at 746–749 does require that human text. The fresh final snapshots take
the already-correct Terminated branch, as P5 executes directly.

At `run_trial:1014–1024`, a nonzero disposal result immediately calls `die` with
the generic text “failed Service disposal lost its StartupProbeFailed evidence”.
That message does not distinguish stop-command failure, polling expiry, query
failure, or a specific predicate mismatch. It is therefore not evidence that
the startup terminal disappeared.

The worker still has `WORKER_FAILURE_DEPLOYED=1` when `die` executes. The EXIT
trap calls the real `worker_cleanup` (`run-example.sh:840–865`), which calls
`dispose_failed_service` **again** at 857 using the same output paths. This
overwrites `failure-stop.out`, its `.rc`, and `failure-final-describe.out`. The
retry may succeed, but `worker_cleanup` preserves the incoming failure exit
code. Since the original call never reached `record_timing` at 1019, the printed
`failure-stop-and-reclamation` duration for a failed worker comes from the
**trap retry** at 859. c008's 2,470 ms, c009's 13,712 ms and c010's 26,086 ms
are those retry durations (`run.log:196`, 207, 119, with lexicographic case
ordering); they cannot demonstrate that the first disposal failed in only a
few seconds. The ledger's `WORKER_FAILURE_STATE` update at 1021 is also skipped,
so its Failed value is not an observation contradicting final Terminated.

### What remains causal evidence, and what remains a hypothesis

The following owner behavior is unchanged and relevant:

- `handlers::stop_workload` records the stop intent and returns an acknowledgement
  (`handlers.rs:834–891`); AlreadyStopped means that intent key already exists.
  Neither response promises that VM shutdown and terminal publication finished.
- WorkloadLifecycle's stop-intent branch emits StopAllocation only for Running
  rows and returns without entering its ordinary restart branch
  (`workload_lifecycle.rs:527–569`). A remaining Failed row is not required to
  acquire a new operator-stop terminal merely to make human output uniform.
- The current serial convergence loop still awaits each selected tick, and a
  real VM stop still awaits its existing two-second beacon window and up-to-ten-
  second VMM grace before its cleanup completes. Those paths and their shutdown
  ownership are cited earlier in this RCA; no new product timing claim is added.
- Ordinary `stop_workload` in the example now allows 180 seconds of observation
  (`run-example.sh:707`), but this disposal helper still allows 60 (`:740`).
  A Running allocation is correctly insufficient evidence of completed disposal;
  its prior startup failure must not be used to waive the stop/cleanup boundary.

**Leading hypothesis:** the last three original disposal calls outlasted the
helper's 60-second observation window while queued stops converged, then their
EXIT-trap retries observed completion. The tail retry durations, successful
preceding disposals, current serial awaited stops and eventual Terminated states
are consistent with that hypothesis. They are not a timestamped record of the
first call. A failed/expired original stop command or unavailable describe
response is not excluded by the overwritten evidence. There is no surviving
first-call exit code, last poll state, or first-call start/end timestamp with
which to choose definitively between those branches.

The current-Failed human-text branch is statically narrow, but no fresh
post-stop Failed snapshot from the rejecting call survives. Promoting that
branch to the cause of these three failures would confuse the pre-stop failure
snapshot with a missing post-stop observation. Likewise, accepting Running in
disposal would confuse failure eligibility with stopped runtime. Neither is
justified by this capture.

Backward causal validation stops at the missing original call: the worker
error proves a nonzero disposal result, but its saved files come from the later
retry and all pass P5. Forward validation is complete for the evidence mismatch:
original nonzero result → `die` → EXIT cleanup retry using identical filenames
→ later accepted Terminated snapshot → unchanged failure exit and stale ledger
state. That explains why the printed record looks contradictory without
inventing a new lifecycle violation.

### Scope disposition and precise investigation blocker

There is **no proven necessity for a companion disposal predicate correction**
to finish the already-approved Running-recovery change. Its necessity is
specifically not established by this run: the retained disposal states are
already accepted. There is also no reproduced new product contract violation.
The example still fails its overall acceptance outcome, but its precise first
disposal failure is underdetermined by the retained evidence.

No further fix is proposed or implemented. A wait-policy change, disposal
predicate change or production scheduling correction must not be slipped into
the approved two fixes on this evidence. The precise next diagnostic need is
to retain **within one run** the first disposal invocation's command exit code,
start/end times and final polled description before the existing trap retries
it. That is a bounded diagnostic observation, not an archival policy or new
production test seam. Once the original branch is known, an actual failing
regression must reproduce it; any proposed production ordering defect still
requires the seeded real-owner Sim invariant before remediation is selected.

This follow-up is blocked from that final causal discrimination because this
capture's first-call outputs were overwritten and new native runs or edits to
the crafter-owned example are not authorized for this agent. It does not claim
a missing whole-control-plane testability boundary. Only this appended RCA
section and the new P5 evidence probe were written; the existing proposal,
crafter-owned tests/example, production, design and fresh evidence were preserved.

## Authorized diagnostic instrumentation and next native capture

The user authorized append-only observation capture and another canonical E09
run. Scope: preserve each stop/disposal invocation and polled public response
within that run, including main versus EXIT-cleanup context, command output,
exit codes and start/end/elapsed times. Poll samples are observations, not a
complete history of internal allocation transitions. Existing predicate,
scheduling, timeout and product behavior remain unchanged.

### P6 — First invocation survives an actual cleanup retry

Before execution: hypothesis is that the current helper/EXIT-cleanup composition
loses its first command and query records. A host-safe behavioral test drives
the actual disposal helper and actual worker_cleanup with bounded external
command adapters: first stop succeeds, a describe query fails, a later query
sees Running, observation expires, cleanup issues a second stop and sees
Terminated. These adapter inputs test recording, not product lifecycle
reachability. Predicted pre-instrumentation result: a preservation assertion
fails because only the retry output remains. Falsification: all first-attempt
output/query failures and timing already survive unchanged with distinct
main/cleanup identities. After instrumentation the identical test must pass.

### P7 — Observe the real first disposal failure

Before native execution: leading hypothesis is a successful original public
stop followed by the disposal helper's 60-second polling expiry with no accepted
terminal snapshot yet; a cleanup retry later sees Terminated/stopped. Competing
branches are original stop-command failure, query failure, or an actual
current-Failed human-text mismatch. Predictions are separate: each branch must
appear in the original invocation's immutable command/attempt records. Any
different captured branch falsifies the leading hypothesis. Run only
`verification/harness/run-expectation.sh E09-v2`, retaining its canonical native
preflight, exclusive lease and existing twenty-pair/concurrency-ten/600+60 bounds.

### P6 implementation and verification

`observe_command` (`run-example.sh:430`) creates a unique directory before each
public stop/describe invocation. Its `stdout-stderr` file receives the original
combined command streams directly; its metadata appends the command, workload,
main/cleanup context, stage, epoch-nanosecond start/end times, elapsed milliseconds
and exit code. Successful describe responses add a sampled-state label while
retaining the full response. Failed queries retain their output and nonzero code.
Existing predicate files are only compatibility scratch copies of those records.

`observe_stop_attempt` (`:464`) groups commands/polls under a unique invocation
directory and appends the complete invocation's timing/result. Actual
`worker_cleanup` marks its context `exit-cleanup` (`:892`). Case reporting prints
all record files (`:1244`), so they survive normal remote materialization removal
in the canonical local evidence capture. No existing record is truncated by a
later poll or retry. An interrupted command/attempt retains its begin metadata
and whatever combined output it wrote; missing end metadata is not fabricated.

Exact recording regression:

```sh
bash docs/analysis/test-e09-v2-disposal-observation-history.sh
```

Before the instrumentation it exited **1**:

```text
FAIL: first disposal command/query records do not survive cleanup retry
```

After instrumentation it exits **0**:

```text
PASS: main command, query failure, Running sample and timings survive cleanup Terminated sample unchanged
```

The test compares every first-attempt file byte-for-byte after the actual
worker_cleanup retry, then checks the actual `print_case_transcript` output for
both attempts' stdout/stderr, the failed query, sampled states, timing and exit
codes. The existing host-safe scheduler tests, corrected P1/P4, retained-state
P5 control, Bash syntax check and `git diff --check` also pass. The disposal
60-second bound, ordinary stop 180-second bound, all predicates, stop command
45-second bound, describe 10-second bound, scheduling and product code are unchanged.

### P8 — Replay the captured disposal completion window through both helpers

Before execution: the new immutable native records show successful original
stop commands and 53 successful Running polls, followed by disposal expiry at
about 60.8 seconds. c010's first retained Terminated/stopped sample arrives
about 85.2 seconds after its original attempt began, in cleanup. Hypothesis:
the disposal helper's remaining 60-second bound rejects this observed completion
window while the already-established ordinary stop helper accepts it. Predicted
result: on the same native-derived stop/pre-stop/Running/Terminated responses,
with the final captured sample available at rounded-up logical second 86,
ordinary stop passes and disposal fails. Falsification: both accept, or the
ordinary stop control also fails. This is a deterministic shell wait-policy
regression, not a claim that a VM transitions at exactly second 86 or a new Sim
model of production scheduling. Native P7 supplies the real-owner reachability.

### P7 result — the original disposal wait expires while every poll is Running

The canonical command completed with **exit 1**:

```sh
verification/harness/run-expectation.sh E09-v2
```

The retained `verification.yaml` identifies **20:53:25Z**, native-metal,
seed **1**, unchanged HEAD `b653e1ad1758d11be33be457849b284f78141333` and a dirty
tree. `product-run.meta` records **20:53:26Z–20:59:16Z**, exit 1, with unchanged
600-second setup/trial and 60-second cleanup bounds. `run.log:12` records
exclusive lease token `d6fed227627beb0a8a529aba`; line 19 records canonical
fail-closed native x86_64/KVM preflight. The record contains 1,642 command/attempt
metadata sections, with distinct files for every captured poll and retry.

The original missing evidence is now available. **Every original disposal stop
command for c008–c010 succeeded in 108 ms. Each original attempt then made 53
successful describe queries, all reporting Running, before returning 1 after
about 60.8 seconds. There were zero query failures in those original attempts.**
The worker's “lost its StartupProbeFailed evidence” message is therefore a
misdescription of polling expiry in this capture, not loss of the original
startup result or rejection of a completed operator stop.

| Case | Original disposal start UTC | Original elapsed/result | Original polled states | Cleanup retry elapsed/result | First cleanup Terminated query starts UTC |
| --- | --- | --- | --- | --- | --- |
| c008 | 20:57:34.099 | 60,786 ms / 1 | 53 Running, all query exits 0 | 279 ms / 0 | 20:58:35.057 |
| c009 | 20:57:34.256 | 60,855 ms / 1 | 53 Running, all query exits 0 | 11,788 ms / 0 | 20:58:46.787 |
| c010 | 20:57:34.252 | 60,862 ms / 1 | 53 Running, all query exits 0 | 24,446 ms / 0 | 20:58:59.452 |

These timestamps are public query/attempt boundaries. They do not assert the
precise instant of an internal allocation transition between polls. In
particular, c010's first retained Terminated response spans approximately
85.200–85.329 seconds after the original invocation began, not a claim that its
driver transitioned at exactly 85.200 seconds.

Exact `run.log` record locations, all belonging to the **20:53Z** capture:

| Case | Original attempt / stop metadata | Last original Running query | Cleanup attempt / successful Terminated query |
| --- | --- | --- | --- |
| c008 | 30928 / 30937 (`main.dispose_failed_service_attempt.Hv6qrE`) | 29458 (`describe.EeG6vE`) | 29050 / 29022 (`exit-cleanup.dispose_failed_service_attempt.PWLmTi`) |
| c009 | 37940 / 37949 (`main.dispose_failed_service_attempt.q445yy`) | 36960 (`describe.UiziEA`) | 36062 / 35754 (`exit-cleanup.dispose_failed_service_attempt.p3ivbg`) |
| c010 | 45693 / 45702 (`main.dispose_failed_service_attempt.M5vO0H`) | 44818 (`describe.NyvCgJ`) | 43815 / 43192 (`exit-cleanup.dispose_failed_service_attempt.BJ811w`) |

Within each uniquely named invocation, command and query subdirectories have
their own metadata and complete `stdout-stderr`. Case printing sorts filenames;
chronology comes from `started_epoch_ns`/`finished_epoch_ns`, not printed section
order. The cleanup records do not overwrite the original records above.

The failed cases' original deploy transcripts remain typed StartupProbeFailed
with exit 1: c008 at `run.log:27382–27390`, c009 at 34059–34067, c010 at
41445–41453. Their earlier describe snapshots remain the authentic failed
startup observations (27391 onward, 34068 onward, 41454 onward). Original stop
responses say `Stopped workload`; retry responses say `already stopped (no-op)`.
The accepted retry descriptions have current `Terminated` and `reason: stopped`.
The intervening prior Failed/bind diagnostics do not decide that acceptance.

Passing controls under the same process are materially informative: c004,
c005, c006 and c007 original disposals complete in 12,921, 24,250, 36,823 and
48,318 ms respectively (attempt metadata at 10039, 14175, 19026 and 24591).
They show Running until a successful Terminated sample, then pass the same
disposal predicate. The remaining cases reach that same acceptable state after
the shorter helper's window. The ledger records c001–c007 pass/complete,
c008–c010 failed/failure-stop and c011–c020 not-run; this run did not hit its
overall 600-second deadline. Final cleanup at 49480–49483 reports zero VMMs
before and after server shutdown. Per-case post-disposal resource assertions
for c008–c010 were still skipped on their original failure path, so E09 remains
failed and no twenty-pair or per-case cleanup success is inferred.

### Updated causal chain and classification

1. **Why did the original worker fail?** Its real disposal helper returned 1
   after approximately 60.8 seconds; it did not reach the accepted terminal
   predicate before the loop's deadline (`run-example.sh:790–809`; the original
   attempt records above).
2. **Why did it not accept the original observations?** Every original query
   succeeded and returned Running. Disposal correctly requires a terminal
   result; the previous startup-failure fact does not mean the recovered runtime
   has stopped. No original query returned Failed or Terminated here.
3. **Why could Running persist beyond that window?** The public stop is an
   intent acknowledgement (`handlers.rs:834–891`), while WorkloadLifecycle and
   action dispatch perform the existing awaited stop. The current production
   loop serially awaits selected ticks (`lib.rs:3363–3381`); VmDriver's existing
   stop awaits its two-second beacon window, VMM grace and cleanup
   (`vm_driver.rs:1715–1755`). The native public completion sequence extends
   past 60 seconds. This capture proves that sequence, not a new syscall-level
   breakdown or a newly violated product latency promise.
4. **Why does the example reject a sequence its other stop helper handles?**
   `dispose_failed_service_attempt` retains a 60-second observation bound at
   790, while the already-corrected ordinary `stop_workload_attempt` allows
   180 seconds at 759. P8 below reproduces the difference through those actual
   helpers using the same captured completion window.
5. **Why was this initially reported as lost startup evidence?** All nonzero
   disposal results share the same `die` message, and the original recorder
   replaced its first-attempt outputs during EXIT cleanup. The new append-only
   records falsify that interpretation and expose the leftover observation-window
   mismatch. The source supports this bounded explanation; no organizational
   cause or new lifecycle mechanism is inferred.

Backward validation: a completed Terminated/stopped observation is required;
the original helper stops observing at 60 seconds; the successful terminal
sample appears later; original stop/queries were successful. Forward validation:
acknowledged stop → Running polls throughout the 60-second window → original
helper failure → cleanup retry → accepted Terminated/stopped sample while the
original failure exit persists. The recorded chain rules out stop-command error,
query error, the current-Failed human-text branch and the earlier beacon-bind
requirement as causes of this captured failure.

**Classification:** a remaining example observation-window mismatch, in the
same family as the earlier ordinary stop wait correction. It is not another
Running-recovery predicate correction and not evidence of a new product
contract violation. The original startup result and the operator-stop result
remain separate, truthful facts. The existing serial production owner explains
why asynchronous completion must be observed; this report does not select a
product scheduling remedy or reopen #283. No new Sim seam or production API
is needed or proposed. The earlier seeded real-owner spike remains a separate
counterexample to overlapping stream submission, not a claimed new invariant
for this shell wait policy.

### P8 result and smallest proposed behavioral correction

The focused diagnostic now has a second, explicitly failing mode using native
c010 fixtures from this capture:

```sh
bash docs/analysis/test-e09-v2-disposal-observation-history.sh --replay-native-disposal-window
```

Exit **1**:

```text
ordinary_stop_control: final_sample_available_after_seconds=86 expected=accepted observed=accepted
disposal_window_regression: final_sample_available_after_seconds=86 expected=accepted observed=rejected
```

The existing ordinary stop helper accepts the observed completion window. The
disposal helper rejects it. Both execute the checked-in helper bodies and retain
their unchanged stop/describe bounds and predicates. The test advances only
logical polling time and delivers the captured public responses; it does not
create a VM, seed a lifecycle row, or claim to simulate production scheduling.
The default preservation mode still passes and verifies both first-attempt and
retry evidence survive unchanged.

**Smallest proposed fix, not implemented:** change only the disposal helper's
remaining observation bound from **60 to 180 seconds**, matching the existing
ordinary stop observation envelope. Preserve its terminal predicates, original
typed StartupProbeFailed requirement on the journey, prior failed-probe evidence,
negative peer checks, post-disposal resource assertion, stop/query command bounds
and the overall 600+60 run budget. Do not accept Running as disposed, relabel a
timeout as StartupProbeFailed, or require a different historical driver reason.

That single existing-bound correction makes the same P8 disposal replay green:
the required captured Terminated/stopped response is observed within the
180-second envelope. It addresses the original failing invocation directly,
instead of depending on a cleanup retry that correctly preserves the original
failure exit. It is a bounded companion **wait-policy** correction, rather than
an additional production mechanism or a reason to amend the accepted lifecycle
API. This assignment authorizes diagnosis and recording only; the behavioral
correction remains a proposal for the parent/user to approve and dispatch.

The observed first cohort plus setup occupied about 350 seconds. This does not
prove that both cohorts will fit the unchanged 600-second setup/trials budget;
the next corrected canonical run must establish that separately. Do not silently
raise that outer budget or infer twenty-pair success from this one-cohort
diagnosis. Independent evidence review is still required, and E09 has not been
marked satisfied.

### Final scope and verification for this assignment

Changed only the example's diagnostic command/attempt recording and record
emission, the focused host-safe recording/native-window regression, this RCA
append, and the normally regenerated canonical E09 evidence. All pre-existing
barrier/predicate/test/README work was preserved; no additional behavioral fix,
production/API/design change, commit, issue or E10/E13 run was made. Canonical
native execution is complete and its first-attempt evidence is retained.
