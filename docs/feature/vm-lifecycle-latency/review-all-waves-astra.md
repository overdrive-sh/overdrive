# VM lifecycle latency: independent cross-wave review

| Metadata | Value |
|---|---|
| Date | 2026-09-10 |
| Reviewer | GPT-6 Astra, independently dispatched at the user's request |
| Feature | VM lifecycle latency #283; shared convergence serialization #260 |
| Reviewed HEAD | `cc94b7c87e5dbefd5ff8364779c676a65edd7abd` plus the pre-existing uncommitted DISTILL/roadmap/example changes |
| Review iteration | 1 |
| Verdict | **CHANGES_REQUESTED — two bounded acceptance-evidence/handoff corrections** |
| Implementation authority | None: review only; no production, test, design, approval-status, or roadmap edits |

## Assessment

The research and RCA establish a reachable problem, and the accepted design addresses that problem at the correct boundaries. The seeded production-server witness still demonstrates independent-workload starvation behind a held start or stop. Retained native logs independently support the two-second host request window followed by the ten-second VMM grace and forced exit. Neither the literature nor those few native observations establishes the proposed latency quantiles.

The main cross-wave weakness is narrower: two preservation/shutdown promises are mapped to executable tests whose assertions do not establish those promises. These are acceptance-test and handoff findings, **not newly discovered production defects**. They require correcting the declared evidence and assigning the missing test construction within the already approved architecture. They do not justify changing ownership, adding persistence/recovery machinery, widening an API, or starting remediation without the user's instruction.

The accounting of ten behavioral RED tests, eight pending bodies, and one healthy control is accurate. The eight pending bodies are permitted DISTILL scaffolds and are explicitly disclosed. Their existence alone is not a finding. A nextest result saying “16 passed” is not a feature pass: fifteen selected cases succeed by matching expected panics. The native distributions and amended E09 execution remain pending, correctly.

There are no confirmed critical/high production findings in this review. The two medium findings below block my endorsement of the current claim that the executable handoff is fully mapped. Existing approval metadata remains untouched.

## Scope, instructions, and artifacts examined

Applied `AGENTS.md`, `CLAUDE.md`, and `.claude/rules/{bpf,debugging,design,development,rust,testing,verification}.md`. Loaded the installed solution-architect, acceptance-designer, troubleshooter, and researcher reviewer definitions and their relevant architecture, roadmap, acceptance, BDD/test-mandate, RCA, and research critique skills. The user's explicitly selected Astra model takes precedence over legacy model frontmatter. Native Markdown, exact design signatures, preserved dirty work, and the repository's production-reachability rules govern this review.

All present feature artifacts were reviewed independently of their previous verdicts:

- [Feature delta](feature-delta.md), including the approved DESIGN, DISTILL amendment, S-VLL scenarios, history, measurement profiles, coverage audit, and execution handoff.
- [DESIGN review](design/review.md), including its snapshot-versus-final-backlog correction and second iteration.
- DISTILL [acceptance review](distill/review-acceptance.md), [design-alignment review](distill/review-design-alignment.md), and [scope/history review](distill/review-scope-history.md), including recorded corrections and limitations.
- [Current roadmap](deliver/roadmap.json) and [fresh roadmap review](deliver/review-roadmap.md); retained prior roadmap, generated skeleton, and CLI evidence under `.context/vm-lifecycle-roadmap/`.
- [Comprehensive research](../../research/orchestration/vm-lifecycle-latency-283-comprehensive-research.md), its linked [RCA](../../analysis/root-cause-analysis-vm-lifecycle-latency-283.md), and the [independent RCA review](../../analysis/review-vm-lifecycle-latency-283.md). The linked RCA review supplies the companion local-premise review; it is not an independent replication of every external benchmark.
- [ADR-0102](../../product/architecture/adr-0102-bounded-convergence-evaluation-ownership.md), [ADR-0103](../../product/architecture/adr-0103-responsive-vm-stop-and-guest-supervision.md), the corresponding architecture-brief and C4 amendments, and the relevant established broker, runtime, VM/session, service-health, journey, and preservation contracts referenced by the feature.
- The changed Sim, core broker, init, worker stop-totality, and native CLI test sources; mapped runtime View-store/re-enqueue, early-exit, reclamation, stop-totality, clone-index, and native equivalence support. These were assessed as executable evidence, not solely from their names or reviews.
- E09 v2 example README, actual shell worker/coordinator, scheduler tests, expectation README/runner, expectation index, and runner harness; historical changes at `f8dad8bc`, `373b335c`, `cbcf9a6d`, `b653e1ad`, and `9e41ffdb`.
- Retained focused Rust and shell evidence in `.context/vm-lifecycle-latency-distill/`, including the unchanged-HEAD Lima timeout-fixture failure, and retained native RCA logs and observation patch. The accidental interrupted native diagnostic archive was identified as excluded evidence, not credited as a successful run.

Production-path inspection covered the HTTPS submit/stop enqueue boundary; the convergence task and shutdown owner in `overdrive-control-plane/src/lib.rs`; complete runtime hydration, View persistence, awaited shim dispatch, and re-enqueue; action-shim Driver entry; `VmDriver::stop`; the real VMM wait/reaper; PID 1's existing lifecycle; and the exit observer's consumed-event retry loop. Supporting existing tests were checked against those boundaries rather than assumed to exercise them.

The user explicitly skipped DEVOPS. Missing feature-local DISCUSS/DEVOPS artifacts, deployment plans, rollback plans, or production-readiness work are not findings. E09 reuse is authorized. This review does not require a new expectation merely because a new feature exists.

## Findings

### F-01 — Medium: S-VLL-13 maps “fsync failure emits no effect” to a test that never dispatches an evaluation

**Class:** Definite acceptance-evidence mapping defect. **Disposition:** Open; no edits made.

The approved obligation is explicit: `feature-delta.md:305` requires “View fsync failure emits no effect,” and S-VLL-13 at `feature-delta.md:450` requires that no effect precede the durable View. That row and `deliver/roadmap.json` map the obligation to `reconciler_runtime_view_store::runtime_writes_through_before_in_memory_update` plus existing regression suites.

The named test's actual path is:

1. `crates/overdrive-control-plane/tests/integration/reconciler_runtime_view_store.rs:135` creates a runtime and seeds a View.
2. At lines 160–162 it injects the View-store failure and invokes `apply_next_view_for_test`.
3. `crates/overdrive-control-plane/src/reconciler_runtime.rs:920` implements that helper by calling **only** `persist_view`.
4. The assertions at test lines 165–179 establish unchanged hot and stored Views. No Driver/shim effect is observed, and `run_convergence_tick` is never invoked.

This is a valid existing persistence-ordering test. It cannot establish the stronger composed no-effect guarantee assigned to it. The equal-View companion likewise tests persistence elision. The mapped failed-allocation/backoff tests do not inject this View-store failure. Searching the control-plane and Sim tests for the relevant fsync injection did not identify a mapped composed no-effect test; the existing `WriteThroughOrdering` Sim contract also concerns the hot View, not dispatch.

**Why it matters:** The runtime currently persists before dispatch (`reconciler_runtime.rs:1497` onward), so there is no reproduced production violation here. The defect is that the declared regression gate would continue proving the persistence helper while missing an ordering regression in its caller. The current DISTILL self-audit and prior acceptance review treat this preservation mapping as complete.

**Bounded correction:** Identify an existing test that actually exercises the runtime/effect boundary under this failure and map it, or explicitly record the missing composed oracle as acceptance-designer-owned test construction for step 01-01. Preserve the existing helper test as its useful narrower complement. Do not add a public seam or claim a production failure to solve this mapping issue. If the existing composition cannot inject the needed failure, surface that concrete testability gap before changing an API.

**Evidence standard:** Established directly from the declared obligation, helper implementation, and assertions. No hypothetical production schedule is promoted to a defect, and no production mutation was performed.

### F-02 — Medium: S-VLL-06a's current oracle checks that shutdown waits for something, not that every admitted result drains

**Class:** Definite incomplete test oracle/handoff. **Disposition:** Open; no edits made.

ADR-0102 lines 151–158 requires every admitted evaluation to reach its real result before the owner reports and exits. `feature-delta.md:443` gives S-VLL-06a the corresponding all-active-drain and ninth-never-executes contract. The two named tests delegate to `capacity_case`.

In `crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs:305–318`, the close branch:

- samples `!shutdown.is_finished()` once while all effects are held;
- releases all `held` semaphore permits together;
- joins shutdown;
- checks whether the independent workload has a `Running` observation.

The final assertions at lines 343–350 check the **entry** count, the eight-slot boundary, and that the independent workload is not Running. `DelayedDriver` at lines 40–69 reports entry events containing only `Effect`; it has no per-allocation completion/result-consumption ledger. The close branch does not inspect the completed state of all eight targets. In the stop variant, the wrapped `SimDriver::stop` has already executed before the hold, which makes returned-result ownership particularly important to observe separately.

**Consequences for the oracle:** Waiting on one remaining effect is sufficient to satisfy the single pre-release wait assertion. The test does not distinguish that observation from waiting on all eight. Similarly, absence of a Running row is weaker than proof that the ninth evaluation never executed. These are limitations of the assertions, not claims that current production drops seven results or admits the ninth during shutdown.

The source's existing RED is real, but narrower: the fresh seeded run fails first at `admitted == held` with `admitted=1`, just as the feature honestly states. It does not demonstrate a shutdown-drain failure. S-VLL-06b is a pending exit-report/snapshot test whose precondition says active results have been consumed; it does not presently close this missing behavioral observation.

**Bounded correction:** Explicitly assign completion of the all-active-results and ninth-not-executed oracle to acceptance-designer assistance in the S06 handoff. Strengthen the existing in-process fixture using its existing Driver/owner boundaries and complete target state/result observations before removing its RED marker. Keep start and stop cases and cooperative real-owner shutdown. No detached-task cancellation experiment, new owner protocol, or production API is requested.

**Evidence standard:** Static comparison of scenario obligations with actual assertions, plus a fresh seeded run confirming the current RED's narrower failure point. This finding does not require or assert a new production failure.

### N-01 — Low: future delivery model instruction is stale

`feature-delta.md:524` says to follow AGENTS.md's “Luna/max” crafter/reviewer selection. The current AGENTS.md says GPT 5.6 Terra with high thinking. Historical reviews correctly retain the models that actually performed them; those records should not be rewritten. The future-facing handoff sentence should defer to the current rule or name its current selection. This review's explicitly requested Astra selection is unaffected. This note alone would not block approval.

## Cross-wave alignment matrix

| Contract chain | Independent assessment | Evidence state / disposition |
|---|---|---|
| Research → local RCA premise | External controller/guest-supervision patterns inform the direction; retained local evidence establishes the actual problem. External boot benchmarks are not Overdrive READY/Service-stop SLOs. | Aligned; sampled upstream validation and native-log recomputation, not independent replication of every literature result. |
| RCA → full evaluation ownership | Real HTTPS → broker → serial convergence task → complete runtime → awaited shim → Driver explains the held-effect starvation. | Seed 283001 reproduced for start and stop; healthy/released controls pass. |
| Research's conceptual key → exact production key | DESIGN correctly chooses existing `TargetResource`, not invented allocation keys. Workload/Service/SVID turns sharing a workload target exclude one another; service projection/node reclamation retain their distinct targets and existing claims. | Aligned; no View repartition or new ownership subsystem required. |
| ADR-0102 → exact API | Changed submit/drain timestamps, router constructors with `Arc<dyn Clock>`, and enqueue dispatch timestamp are specified exactly. `Evaluation`, `BrokerCounters`, public owner signatures, and wire/config remain unchanged. | S05 property bodies pending; roadmap requires atomic caller/compiler fallout. No invented public API in the reviewed test-only additions. |
| ADR-0102 → scheduling policy | Eight shared slots, eligible FIFO with preserved replacement age, target exclusion through result consumption, immediate refill, cadence/resync, and advisory deadlines are stated consistently. Eight stalled effects may fill capacity; no stop priority is promised. | S01–03 real RED; S04/S05 pending. No claim that current serial production already supplies these guarantees. |
| ADR-0102 → shutdown | Admission closes, active results drain, one locked coalesced pending snapshot excludes later submissions, and shutdown ordering retains consumed-event retries. | Design is precise; S06a oracle needs F-02 correction; S06b body pending. No final-backlog/unread-queue guarantee invented. |
| Durable runtime order → S13 | Current code persists the View before awaited dispatch and preserves established self-requeue behavior. | Existing persistence tests are useful, but stronger no-effect mapping needs F-01 correction. |
| RCA → host stop | Existing writer completion does not end its fixed window; VMM grace starts afterward. Accepted design overlaps one bounded writer with the same VMM grace and consumes the writer. | S07 real RED; S08 outcome matrix pending. `stop Ok` remains a narrow driver result, not normal-exit/cleanup proof. |
| RCA → PID 1 | Existing blocking child wait prevents timely control handling; accepted single-thread supervision preserves pre-EXEC errors, process-group semantics, direct status, and typed errors. | S09 real lifecycle-trace RED; S10 native/private-File matrices pending. No generic new supervisor port sanctioned. |
| Design → stage measurement | READY starts at VMM create entry; finite Job starts at EXEC release; cooperative Service ends only after normal VMM exit plus required artifact absence. Independent clock domains, pinned profiles, nearest-rank quantiles, failed-trial retention, and instrumentation controls are explicit. | S11 is pending; 1200 planned trials are not executed measurements. Prior RCA create-completion→READY sample is not the new create-entry distribution. |
| DISTILL → test ownership | Acceptance designer owns additional body construction; crafters implement exact contracts; fresh reviewers verify complete assertions before RED removal. | Eight named pending bodies adequately specify their intended scenarios. F-01/F-02 identify additional specificity needed in this assignment, not permission for crafter API invention. |
| History → amended E09 | `f8dad8bc` really introduced the cleanup barrier and longer windows. Current changes remove those serial accommodations while retaining real healthy/failure activity barriers and truthful history handling. | Fresh host-safe scheduler/runner pass; native independent-worker outcome pending. |
| Tests ↔ expectations | Rust uses in-process product composition. E09 runner drives the checked-in example through built default-feature product, with external observations. Detailed framing, group status, exact counters, private lifecycle and cleanup complements stay in integration. | Boundary is preserved in the reviewed changes. Shell fixtures test the scheduler/runner, not native product success. |
| DISTILL → roadmap | Three dependent steps match convergence ownership, host/guest termination, and native evidence; exact ADRs and full scenario table remain authoritative. Shared S13 and final-only mutation gate are explicit. | Structure/locator existence are not semantic coverage proof; F-01/F-02 remain despite prior structural approval. |
| User scope → completion | No production users; DEVOPS skipped. Existing E09 reuse is sufficient for this operator journey; native detailed measurement has its own in-process owner. | Aligned. No deployment or adjacent hardening requirements added. |

## Production premise and ownership checks

The current convergence task at `crates/overdrive-control-plane/src/lib.rs:3320–3393` drains the broker and awaits each complete runtime invocation serially. `run_convergence_tick` hydrates desired/actual state, reads its named target View, reconciles, persists, awaits dispatch, and finally self-requeues before returning its result (`reconciler_runtime.rs:1352–1608`). Start dispatch reaches `Driver::start` at `action_shim/mod.rs:1931`; stop dispatch retains its awaited Driver boundary and terminal handling. The Sim wrapper holds those genuine Driver calls through existing ports after public HTTP requests; it does not synthesize the independent workload's Running result.

Current cooperative shutdown at `lib.rs:1409–1458` cancels and joins convergence before workflow emit-drain, interest router, HTTP drain, and exit-observer shutdown. The exit observer selects cancellation between received events (`worker/exit_observer.rs:200–208`); a consumed event executes `run_with_retry` to completion, with the existing 50/100/200 ms backoffs (`:347–384`). The accepted snapshot wording is consistent with this owner ordering. A theoretical Tokio cancellation point is not evidence that this owner aborts an effect.

`VmDriver::stop` at `crates/overdrive-worker/src/vm_driver.rs:1651–1757` moves a live session into ending ownership before awaiting work. Its fixed request wait precedes `Vmm::terminate`. The native reaper at `crates/overdrive-host/src/vmm.rs:465–575` observes actual child status, including the forced kill path; the `TerminateOutcome` label alone is not substituted for that status. Existing cleanup calls discard certain best-effort errors, consistent with the accepted narrow `stop Ok` contract. The native acceptance plan appropriately requires independent exit/artifact evidence.

The guest currently executes the operator synchronously and reads SHUTDOWN afterward (`crates/overdrive-init/src/main.rs:140–206,975–1080`). ADR-0103 names the exact private changed function shape and errors instead of leaving a public API gap for implementation. Its single five-second command-group grace, no deadline extension on duplicate SHUTDOWN, original-error retention, and immediate successful poweroff are meaningfully distinct test obligations in S10, not proved by S09's pure lifecycle trace alone.

## Verification performed and limits

### Fresh bounded execution

The focused Linux run was executed through the required Lima wrapper:

```text
cargo xtask lima run -- cargo nextest run \
  -p overdrive-core -p overdrive-init -p overdrive-worker -p overdrive-sim \
  --features integration-tests --success-output immediate \
  -E 'binary(vm_lifecycle_latency_283_spike) | test(bounded_admission_contract) | test(completed_command_powers_off_without_waiting_for_shutdown) | test(completed_shutdown_write_has_no_two_second_floor) | test(writer_bound_overlaps_the_single_vmm_grace_and_every_writer_is_consumed)'
```

Result: **16 test-harness passes, 1418 skipped**. The first run omitted successful-test output; the second retained it specifically to verify the expected-panic causes and printed seeds. Evidence: `.context/vm-lifecycle-astra-review/focused-rust.txt` and `focused-rust-oracles.txt`.

| Selected evidence | Actual result |
|---|---|
| S01 held start and stop | Seed 283001; independent Running is false while held, true after release; cooperative shutdown joined; intended liveness assertion panics. |
| S02/S03/S06a six cases | Seed 283001; all report only one admitted effect; intended seven/eight admission assertion panics. Later assertions are not independently reached on current production. |
| S07 | Intended completed-request two-second-floor assertion panics. |
| S09 | Exact lifecycle trace differs by the existing post-EXIT SHUTDOWN read; intended assertion panics. |
| S04/S05a/S05b/S06b/S08 | Five explicit pending-body panics; no exercised behavior credited. |
| Healthy control | Independent Running is true before/after release and shutdown joins; genuine non-panic pass. |

The other three of the eight pending bodies are native S10a/S10b/S11. Their source and prior compilation evidence were inspected; this review did not execute them or launch the native distribution workload.

Fresh `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh` and `bash verification/harness/test-e09-v2-runner.sh` both returned zero. Evidence is retained as `scheduler.txt` and `runner.txt` under the review evidence directory. The negative scheduler case's “worker 1 exited before cohort marker” output is expected harness exercise, not a failed native product run. These tests use controlled shell fixtures to verify orchestration/transcript rejection. They do not force the product to pass because they do not claim to test the native product in the first place.

### Retained evidence rechecked

`git apply --check docs/analysis/native-observation-283.patch` succeeded without applying it. The retained summarizer was run against the Uhhwly, B9sklj, and nwgtIF healthy native logs. Recomputed driver-stop totals were **12030.428, 12022.214, and 12020.339 ms**. The untraced nwgtIF case decomposed to **2002.164 ms** request window, **10017.545 ms** terminate-through-completion, and **0.630 ms** cleanup calls. The real reaper recorded signal 9. Writer success at **0.119 ms** establishes host write completion only. Finite Job EXEC-to-reaper controls recomputed to **112.179, 113.641, and 30.445 ms**.

These support the RCA's causal decomposition and tracing-perturbation warning. They do not prove prospective quantiles, production guest receipt, or universal successful cleanup. The 1131.665 ms READY sample starts at the historical create-completion marker; it must not be substituted for the newly specified create-entry profile.

The retained baseline shell evidence records the old scheduler timing out on the independence witness while the changed scheduler passes. Direct Git history supports that distinction. The unchanged-HEAD Lima runner/harness timeout case records a surviving descendant and exit 1 in `.context/vm-lifecycle-latency-distill/runner-baseline-jypkq8wq/0.txt`. That is honestly a pre-existing environment/fixture failure, not evidence against the new independent-worker scheduling change and not a successful cleanup check. No adjacent harness fix was attempted. The current host-safe macOS runner pass does not erase the retained Lima limitation.

Sampled upstream checks used primary sources: [controller-runtime v0.22.0 options](https://github.com/kubernetes-sigs/controller-runtime/blob/v0.22.0/pkg/controller/controller.go), [Firecracker v1.14.1 specification](https://github.com/firecracker-microvm/firecracker/blob/v1.14.1/SPECIFICATION.md), and [Depot's reported methodology](https://depot.dev/blog/optimizing-microvm-boot-times). They support treating concurrency as explicit policy and boot figures as boundary/environment-specific evidence. I did not rerun external benchmarks or convert their statistics into Overdrive measurements.

No broad native expectation run, deployment, mutation test, implementation change, or new production-failure reproduction was performed. No fresh failure model was introduced. The precise public/private design shapes remain the implementation contract. The evidence findings above can be resolved at the existing acceptance handoff without expanding that contract.

The final preservation check matched HEAD and all 19 pre-existing dirty/untracked file hashes in the supplied baseline. Its result is retained in `.context/vm-lifecycle-astra-review/preservation-check.json`.

## Verdict and required disposition

**CHANGES_REQUESTED.** Correct F-01's stronger-than-actual regression mapping and explicitly close F-02's missing shutdown-result oracle obligation before relying on the current acceptance handoff as complete. These are bounded test-specification/construction assignments within S13 and S06; they do not authorize production changes or design expansion. N-01 is a minor future-facing instruction correction.

The rest of the reviewed chain is coherent within its stated limitations. In particular, the established RCA is reproducible, the exact design contract is sufficiently pinned for its named mechanisms, the pending-body accounting is honest, E09 reuse respects the user's scope, and native performance remains unverified. Previous approvals were read as history and were not changed. This artifact is the only feature file created by this review.

## Iteration 2 — bounded remediation re-review

| Metadata | Value |
|---|---|
| Date | 2026-09-10 |
| Reviewer | Same explicitly selected GPT-6 Astra reviewer |
| Reviewed HEAD | `cc94b7c87e5dbefd5ff8364779c676a65edd7abd` plus the authorized uncommitted remediation |
| Authority | Re-review of F-01, F-02, and N-01; no implementation, API, architecture, or scope expansion |
| Current final verdict | **APPROVED** |
| Previous iteration | Preserved above verbatim as historical evidence; its open dispositions are superseded by this iteration |

### Scope and necessity

Read `.context/vm-lifecycle-astra-remediation/review-scope.md` and `remediation.md`, the changed Sim test, current feature delta and roadmap, retained successful and unsuccessful validation records, and the relevant unchanged production runtime, generation-input, and convergence/shutdown paths. Re-ran the focused Sim binary through Lima. This review owns only this appended review section and temporary review evidence.

The corrections remain necessary and bounded: F-01 needed evidence at the runtime caller rather than its persistence helper; F-02 needed stronger observations and an explicit assignment of the remaining owner oracle. Neither finding alleged a new production failure. The author has not responded by inventing such a failure, adding public testability surface, or altering the accepted architecture. N-01 follows the user's clarification that future agent selection must refer only to AGENTS.md.

### Finding dispositions

| Finding | Disposition | Basis |
|---|---|---|
| F-01 — composed View-failure/no-effect mapping | **CLOSED** | A real composed preservation regression now exercises unchanged `run_convergence_tick`; S13 maps it explicitly, while retaining the old persistence tests under their narrower guarantees. |
| F-02 — all-result drain/non-admission oracle handoff | **CLOSED as a DISTILL handoff correction** | Per-allocation Driver return and shim-row observations strengthen the executable control. The remaining eight-way owner-consumption/non-admission assertions are precisely specified and assigned to acceptance-designer assistance before S06 RED removal. They are not represented as implemented or already proven. |
| N-01 — stale future agent selection | **CLOSED** | `feature-delta.md:527` refers only to AGENTS.md; no replacement model or thinking-level name appears in that future-facing instruction. Historical reviewer records remain intact. |

#### F-01 evidence

`crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs:541` now defines `view_fsync_failure_prevents_dispatch_and_recovers`, with the required bounded-change Contract Shape declaration immediately above it. It composes the real runtime, WorkloadLifecycle reconciler, action shim, and existing storage/Driver/observation ports. It calls `run_convergence_tick`, not `apply_next_view_for_test` and not a reproduced implementation of dispatch.

The sequence is discriminating:

1. Valid workload/generation inputs are written through the existing IntentStore. The helper does not seed a next View or allocation row. A generation increment is an existing production input (`handlers.rs:969–976`), and this is an in-process runtime-boundary regression, not a claim to exercise the HTTPS handler.
2. A healthy control reaches Running and establishes that this composition actually dispatches.
3. With the existing SimViewStore failure injected, the subject runtime call returns `ConvergenceError::ViewPersist`. Complete hot/stored View maps and allocation rows remain unchanged; Driver starts/live membership remain unchanged; no lifecycle event is published.
4. Clearing the fault allows the same subject input to reach Running and acquire its durable generated View. A subsequent evaluation does not place it again.

The current production code still awaits `persist_view` before dispatch (`reconciler_runtime.rs:1494–1501`), so this test correctly passes as preservation evidence. It does not mislabel an import failure, fixture error, or new defect as RED. The healthy/recovery controls rule out a vacuous “the subject never had actionable work” interpretation. Existing ports and helpers suffice; no public API changed.

`feature-delta.md:450` now names this composed oracle and expressly says the old helper tests do not dispatch an evaluation. `roadmap.json:231` adds its exact locator to S13, and the step-01-01 whole-binary verification command includes it. This satisfies the original bounded correction without expanding the guarantee to unrelated adapters or requiring a new failure model.

#### F-02 evidence and remaining implementation obligation

The shared fixture now records allocation identities at Driver entry and successful return (`vm_lifecycle_latency_283_spike.rs:42–79`). The close branch at lines 349–447 releases one currently entered call at a time, inspects matching returned-target and shim-authored Running/Terminated sets, checks shutdown remains pending before remaining holds, and rejects ninth-workload Driver entries and allocation rows. These observations are stronger than the original one-time wait/absence-of-Running check.

The fresh run confirms exactly the limitation documented by the author: each start/stop close case has **one pre-close Driver entry, one sequential release stage, and eight returned calls with matching rows by join**. The seven later returns belong to the unchanged owner's previously drained serial batch (`lib.rs:3355–3384`). They are not eight concurrently active Driver calls, and they do not prove eight owner-consumed evaluation results. Each case reaches its intended `1 != 8` admission RED only after the stated bounded observations. The entry-set equality after that RED remains unexecuted on current production.

The new `feature-delta.md:537` section, “Remaining S06 owner oracle,” explicitly distinguishes Driver return/shim publication from complete evaluation-result consumption, and distinguishes no Driver effect from no evaluation admission/hydration. It assigns the missing construction to acceptance-designer assistance in **01-01 before either S06a RED marker is removed**. The roadmap's `acceptance_construction` entry links that assignment directly.

The remaining specification requires matching eight admitted target/tick identities, sequentially releasing seven while the last keeps shutdown pending, matching each real result and owner completion, then releasing the eighth before the sole drain event. It requires the approved `admitted_at_close = 8` and `completed_during_drain = 8` fields and no ninth admission/completion/effect. It uses ADR-0102's already approved tracing events and existing composition, with an explicit stop-and-surface rule if that boundary cannot establish the facts. S06b retains its separate pending snapshot and consumed-observer retry contract.

This is sufficient for **this review's actual required disposition**: a complete, honest acceptance-construction assignment within the accepted design. Requiring the eight-way production implementation to exist during this bounded DISTILL remediation would change the scope. Approval therefore closes the handoff defect; it does not certify the pending shutdown oracle or authorize RED removal based solely on the now stronger Driver-level checks.

### Verification and preservation

Fresh command:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-sim \
  --features integration-tests --test vm_lifecycle_latency_283_spike \
  --no-fail-fast --success-output immediate
```

Exit **0**; **12 harness passes**: eight behavioral expected panics, two explicit pending-body panics, and two genuine non-panic passes (healthy control and composed S13 preservation). Seed **283001** and both S06 diagnostic records are present in `.context/vm-lifecycle-astra-review/iteration2-rust.txt`. The test count does not imply feature GREEN. Across the feature, the original ten behavioral REDs and eight pending bodies remain, with the additional S06 owner-oracle construction explicitly disclosed.

Inspected the author's retained failed attempts: the missing import and fixture/oracle corrections are identified as authoring issues, not production findings. Reviewed the recorded targeted Clippy, formatting, whitespace, feature-delta, and roadmap validations. The current roadmap remains pending for the orchestrator to synchronize after this verdict; this reviewer has not changed its status. No native expectations, distributions, mutation test, or broader suite was rerun because this remediation did not change those boundaries.

Re-review preservation is checked against a fresh 20-path snapshot in `.context/vm-lifecycle-astra-review/iteration2-baseline.json`. The concurrent user edit to AGENTS.md is preserved as the current policy. The original iteration-1 review remains the exact prefix of this file; only this iteration is appended. No author-owned or unrelated file was changed by the reviewer.

### Final verdict

**APPROVED.** F-01, F-02, and N-01 are closed within their original bounded scope. The corrected DISTILL/roadmap handoff is adequate for subsequent authorized delivery, including its explicit acceptance-designer-owned pending construction. This verdict is not implementation completion, measured latency acceptance, an amended E09 pass, or an instruction to start DELIVER.
