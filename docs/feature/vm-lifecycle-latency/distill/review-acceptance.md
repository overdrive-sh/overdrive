# Independent acceptance-design review — vm-lifecycle-latency DISTILL

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-lifecycle-latency` |
| Review role | `nw-acceptance-designer-reviewer` |
| Reviewer model | GPT 5.6 Luna, maximum thinking (assigned model) |
| Review date | 2026-09-10 |
| Review iteration | 1 |
| Reviewed baseline | `cc94b7c87e5dbefd5ff8364779c676a65edd7abd` (`cc94b7c8`) |
| Reviewed scope | DISTILL additions since `cc94b7c8`, the pending delivery roadmap, changed Rust test surfaces, the E09 v2 example/expectation and host-safe harness |
| Excluded work | Unrelated dirty `AGENTS.md`; no production or test edits by this reviewer; no native diagnostic archive as evidence |
| Roadmap state | `validation.status = pending`; this review does not approve or edit the roadmap |
| Verdict | **APPROVED** for acceptance-design handoff only |

## Review authority and boundaries

The acceptance contract is the user-ratified DISTILL in
`docs/feature/vm-lifecycle-latency/feature-delta.md`, together with the exact
interfaces and ownership decisions in ADR-0102 and ADR-0103. The reviewed
acceptance surface must preserve the existing public commands and wire shapes,
use the existing production composition for in-process tests, and keep the
black-box expectation boundary separate from Rust tests.

The user's latest refinement is binding: prefer seeded Sim and in-process Rust
coverage for detailed lifecycle and scheduling guarantees, reuse E09 v2 where
it expresses the needed operator outcome, add an expectation only for a
distinct stakeholder-visible outcome, and skip DEVOPS. The review therefore
does not require absent DISCUSS or DEVOPS artifacts, a new latency example, or
a new expectation ID.

This is an acceptance handoff review. The ten live RED tests and eight pending
body scaffolds are deliberately incomplete implementation obligations. A
passing `#[should_panic]` test is evidence that the intended RED assertion is
reachable; it is not evidence that the feature is implemented. Native profile
distributions and the amended E09 capture remain pending.

## Acceptance contract and scenario coverage

The authoritative scenario table at
`feature-delta.md:419-450` provides a Given/When/Then contract for S-VLL-01
through S-VLL-13. The following accounting matches the authored source at the
reviewed baseline.

| Scenario | Authored surface and state | Acceptance assessment |
|---|---|---|
| S-VLL-01 | `crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs:140-243`; `slow_start_does_not_block_independent_convergence`, `slow_stop_does_not_block_independent_convergence`, and the healthy `healthy_driver_control_progresses` control. | Live RED is driven through the production server, HTTPS handlers, local intent storage, shared owner and injected Driver/Clock ports. The held effect is released only after the independent workload is observed. |
| S-VLL-02 | `:245-367`; seven held starts and seven held stops. | Live RED covers both effect classes and requires all seven real target effects to be admitted while an independent target uses the remaining slot. |
| S-VLL-03 | `:369-383`; eight held starts and eight held stops. | Live RED covers the capacity boundary, a pending ninth, one released slot and refill. The test keeps the other seven effects held. |
| S-VLL-04 | `:401-414`; `same_workload_reconcilers_share_the_complete_evaluation_lease`. | Explicit pending scaffold. Its contract names an actual Service, accepted observations, public stop, WorkloadLifecycle/ServiceLifecycle sharing, duplicate coalescing, latest intent, Views, rows, and Driver membership. |
| S-VLL-05a/b | `crates/overdrive-core/src/eval_broker.rs:204-230`; two pending property bodies. | Explicit pure-function scaffolds name the independent reference model, timestamp domains, limits, blocked targets, replacement age, counters, reaping and FIFO fairness. They carry the exact pure-function declaration. |
| S-VLL-06a | Sim tests at `:385-399`; admission-close starts and stops. | Live RED uses `ServerHandle::shutdown` and real held Driver futures. It requires the eight-way admission before asserting that shutdown leaves the ninth unexecuted. |
| S-VLL-06b | Sim scaffold at `:416-429`; source-local owner assertions are permitted beside the private convergence owner. | Explicit pending scaffold names the locked owner snapshot, later submissions outside the unchanged count, consumed observer retries, and the absence of an unread-queue guarantee. |
| S-VLL-07 | `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:1641-1677`; `completed_shutdown_write_has_no_two_second_floor`. | Live RED uses the existing `VmDriver`, `BeaconWriter`, `SimVmm`, `SimClock`, filesystem layout and supervision state. It is correctly described as adapter-ordering evidence, not a native guest SLO. |
| S-VLL-08 | Worker scaffold at `:1679-1693`; `writer_bound_overlaps_the_single_vmm_grace_and_every_writer_is_consumed`. | Explicit pending matrix names writer success/error/EOF/absence/backpressure, early/deadline VMM completion, writer consumption, one overlapping grace and cleanup completion. |
| S-VLL-09 | `crates/overdrive-init/src/main.rs:1260-1285`; `completed_command_powers_off_without_waiting_for_shutdown`. | Live RED exercises the existing generic lifecycle trace and fails on the current post-EXIT SHUTDOWN stage. The exact pure-function declaration is present. |
| S-VLL-10a/b | Native in-process scaffolds at `crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs:2227-2264`. | Two explicit pending matrices name the real init/control boundary, process group, direct-child status, signal grace, framing errors, duplicate EXEC, EINTR/ESRCH/ECHILD and deliberate group escape. They do not spawn the Overdrive binary. |
| S-VLL-11 | Native in-process scaffold at `:2266-2284`. | Explicit pending owner for all three profiles, 200 sequential plus 200 ten-worker trials each, one persistent serve, nearest-rank quantiles, stage separation, normal VMM exit and artifact absence. |
| S-VLL-12 | Updated E09 v2 example/README/runner and host-safe checks. | The existing operator journey is reused in place. Its native expectation is pending; the host-safe shell regression proves only the corrected worker/coordinator ordering and ledger mechanics. |
| S-VLL-13 | Preservation mapping at `feature-delta.md:449-450` and roadmap criteria. | Existing runtime, session/LWW, stop-totality, clone-index, re-enqueue and reclamation regressions are named as preservation gates. No new defect or ownership mechanism is invented. |

The coverage table is complete as a specification. It is also candid about
execution status: S-VLL-04, S-VLL-05a/b, S-VLL-06b, S-VLL-08, S-VLL-10a/b and
S-VLL-11 remain pending bodies, while S-VLL-01/02/03/06a/07/09 are true RED
tests. This is sufficient for a crafter to implement against the approved
contract, provided the crafter preserves the named oracles and removes the
expected-panic markers only when the complete assertions exist.

## RED preflight and scaffold classification

The recorded focused command in
`.context/vm-lifecycle-latency-distill/rust-r4vu5xqj/` selected the Sim binary,
the core broker scaffolds, the init RED, the worker stop RED/scaffold and the
writer scaffold. It exited zero with 16 hook-compatible passes and 1,418
skipped tests. The retained output distinguishes the cases as follows:

| Classification | Count | Cases | Evidence of meaning |
|---|---:|---|---|
| Behavioral RED | 10 | Two S-VLL-01 tests; four S-VLL-02/03 tests; two S-VLL-06a tests; S-VLL-07; S-VLL-09. | Current production owner admits one held target (`admitted=1` for the seven/eight cases), independent progress is false while the effect is held, and the current stop pays the two-second floor. The init trace contains post-EXIT SHUTDOWN. Each final failure includes `RED scaffold`, so setup failures cannot be mistaken for an expected RED. |
| Pending-body scaffold selected by the focused command | 5 | S-VLL-04, S-VLL-05a, S-VLL-05b, S-VLL-06b and S-VLL-08. | Each body is an explicit `panic!(\"Not yet implemented -- RED scaffold ...\")` with a named scenario and `#[should_panic(expected = \"RED scaffold\")]`. No behavior or coverage pass is claimed. |
| Pending native scaffold compiled but not executed | 3 | S-VLL-10a, S-VLL-10b and S-VLL-11. | Each is a named native in-process `#[tokio::test]` with an expected-panic marker and a detailed oracle. The author explicitly records that they were not run as native evidence. |
| Healthy control | 1 | `healthy_driver_control_progresses`. | The same production-server fixture reaches independent `Running` without a held effect and then shuts down cooperatively. |

This is a meaningful RED preflight. In S-VLL-01 the delay is inserted only in
the external `Driver` port (`vm_lifecycle_latency_283_spike.rs:39-82`), while
admission, hydration, storage, HTTPS, observation routing, convergence
ownership and shutdown are the real composition. The seed `283001` is printed
and retained. The capacity cases use distinct public workload IDs and the same
owner path. In S-VLL-07, the test uses the existing `SimVmm` and injected clock
and waits for the real `VmDriver::stop` task rather than manufacturing an
`EndingInFlight` map entry. The init test invokes the existing lifecycle
composition and asserts the actual trace, rather than panicking before the
production helper runs.

The healthy control and the final recovery/shutdown assertions provide a
negative-control layer. The `should_panic` markers are narrow: an unexpected
fixture/setup panic, a server failure or a cleanup panic does not contain the
marker and therefore fails the test. The recorded output shows the expected
behavioral failures before the final marker assertions.

## Driving paths and fixture-theater audit

The primary walking skeleton, S-VLL-01, is an operator-visible independent
workload reaching `Running` while another workload's real convergence effect
is held. The test follows the actual path:

HTTPS submit/stop → existing handlers and intent commit → shared
EvaluationBroker → convergence owner → reconciler runtime and action shim →
injected Driver effect → existing observation store and public query.

The current serial owner is the concrete obstruction documented at
`feature-delta.md:33-43` and in the production path around
`crates/overdrive-control-plane/src/lib.rs:3280-3394`. The test does not invoke
`EvaluationBroker` to simulate the owner, does not inject a scheduler task,
and does not fabricate terminal rows. The `DelayedDriver` semaphore controls
only external Driver latency. This satisfies the hexagonal boundary for the
production-owner acceptance lane while keeping the pure broker policy lane
separate.

The worker S-VLL-07 test drives the existing `Driver` boundary with a real
`VmDriver` and existing SimVmm/filesystem adapters. That is the appropriate
in-process native-adapter lane for a host stop-ordering guarantee. The init
S-VLL-09 test is a source-local pure lifecycle composition with injected
closures and no external process. S-VLL-10a/b and S-VLL-11 are placed in the
existing native integration module and explicitly require the existing
in-process ServeHandle/VmFixture composition. They are not black-box tests in
disguise.

The E09 v2 product path remains distinct. `run-example.sh:1357-1429` builds
the default-feature `overdrive` binary, prepares the checked-in guest bundle,
starts one `serve`, and drives public deploy/describe/`job stop` operations.
The expectation runner at
`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh:21-70`
invokes that example through `cargo xtask metal run --`, extracts its public
ledger and checks operator/kernel/wire/cleanup evidence. It does not run
Cargo tests, a Rust test binary or import an `overdrive-*` crate.

`examples/service-kind-vm-workloads-v2/test-scheduler.sh` and
`verification/harness/test-e09-v2-runner.sh` are explicitly host-safe
surrogate checks. They source or intercept the external shell boundaries and
state that they are not native product evidence. The scheduler test's
`test_independent_failure_submission` at `:248-297` uses the real
`run_trial`/`run_cohort` orchestration, substitutes only external command and
resource observations, holds worker 10's healthy stop, and proves worker 1 can
submit its failure before worker 10 cleanup. This is a valid shell-orchestration
regression and is not used as proof of VM health, kernel behavior or latency.

The accidental archive
`.context/vm-lifecycle-latency-distill/native-diagnostic-20260910T162739.tar.gz`
is correctly excluded. It records an interrupted, incomplete diagnostic after
a non-executable temporary harness fell through to real Cargo; it is neither
native acceptance nor latency evidence.

## Contract Shape and API compliance

Every newly authored or transitioned Rust test has the repository-required
per-test declaration. The two source-local policy/lifecycle properties use the
exact rustdoc line `/// CONTRACT_SHAPE: pure-function.` at
`eval_broker.rs:204,221` and `main.rs:1260`. The bounded Rust tests carry
`/// CONTRACT_SHAPE: bounded-change.` at the Sim, worker and native scaffold
sites. The changed shell test functions carry their corresponding
`# CONTRACT_SHAPE: bounded-change.` declarations. No unclassified new test was
found in the reviewed diff.

The source diff adds no production method, type, enum variant, trait, wire
frame, public parameter or persistence type. The roadmap's first step names
the exact ADR-0102 signatures at `roadmap.json:39-43`:

EvaluationBroker::submit(&mut self, eval: Evaluation, now: Instant)

EvaluationBroker::drain_pending(&mut self, limit, blocked_targets, now)
  -> Vec<(Evaluation, Duration)>

InterestRouterBroker::from_runtime(runtime, clock)
InterestRouterBroker::from_shared_broker(broker, clock)
action_shim::enqueue_evaluation::dispatch(action, broker, now)

ADR-0103's unchanged public `Driver::stop`, `Vmm::terminate` and
`BeaconWriter::request_stop` interfaces, plus its private init boundary, are
represented in S-VLL-07 through S-VLL-11 and roadmap step 01-02. No test
requires a second retry method or a new public scheduler seam.

At the reviewed snapshot, the roadmap groups several scenario IDs under one
representative `scenario_name`, while the authoritative feature-delta table
names every test function and pending body. This is a mechanical locator
limitation already owned by the independent roadmap review and does not make
the acceptance specifications ambiguous to a crafter. Step 01-01 owns nine
scenario IDs, so the review records the required informational
`@sizing-review-needed` signal; this is not a correctness blocker. The roadmap
also correctly leaves `validation.status` pending and does not create DES
events before independent approval.

## Eight acceptance dimensions

Scores use the acceptance-review scale where 7 or above is ready for handoff.
The scores assess the acceptance design, not implementation completion.

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias | 8 | The live RED set includes blocked/full-capacity admission, independent-progress failure, shutdown closure, writer deadline behavior and the post-EXIT lifecycle defect. Pending S-VLL-08 and S-VLL-10 matrices add explicit error, EOF, malformed, signal and cleanup cases. The one healthy control is intentionally small and does not erase the negative coverage. |
| Given/When/Then structure | 8 | All 13 scenario rows use an explicit Given → When → Then contract. S-VLL-05 and S-VLL-10 describe bounded input matrices rather than pretending that every frame/error variant is a separate user story; the underlying boundaries and outcomes are named. |
| Business/domain language | 7 | The walking skeleton and E09 journey use operator terms such as workload, Service, Job, Running, Stable, failure and cleanup. Exact infrastructure terms such as `TargetResource`, VMM, SHUTDOWN and View are retained where they are the approved domain contract; there are no .feature step methods or technical HTTP assertions substituted for the user outcome. |
| Coverage completeness | 9 | S-VLL-01 through S-VLL-13 are mapped, including preservation, error and native evidence lanes. The feature explicitly accounts for every pending body and does not present scaffolds as completed coverage. |
| Walking-skeleton user value | 8 | S-VLL-01 demonstrates independent operator progress through the real server; S-VLL-12 retains the full public E09 journey. Internal lifecycle guarantees are kept in focused Rust/Sim/native tests as the user required. |
| Priority validation | 10 | Seed `283001` reproduces the real serial convergence owner; the healthy control passes, release recovery is checked, and the prior 12,020 ms Service stop observation grounds the stop/guest work. The E09 history analysis follows the actual global cleanup gate rather than guessing at a new requirement. |
| Observable behavior | 8 | Sim tests assert public `Running` observations, live queries and cooperative shutdown; adapter tests assert VMM/process/run-directory and supervision complements; native scaffolds name reaper, wire and artifact outcomes. In-process private state is used only where the repository explicitly requires integration guarantees for ownership and cleanup. |
| Traceability | 8 | The feature-delta table maps each scenario to its test or pending body and ties the outcomes to ADRs, the VM journey and existing US-SVM-1/K1. Missing DISCUSS/DEVOPS files are an authorized scope choice. The grouped roadmap locator is a known documentation issue, not missing acceptance coverage. |
| Fixture-theater detection | 9 | The owner path is real, the Driver delay is an external-port substitution, and the shell surrogates are labeled and bounded. No Rust test launches the production binary and no expectation invokes a test harness. |
| Roadmap sizing signal | 8 | Step 01-01 has nine scenario IDs and receives the informational `@sizing-review-needed` signal. Its criteria, verification commands and implementation scope remain executable; roadmap concision/locator metadata is handled by the separate roadmap review. |

All scored dimensions are at least 7. No acceptance-design blocker is present.

## Three design mandates

| Mandate | Result | Evidence |
|---|---|---|
| CM-A — hexagonal boundary | **PASS** | S-VLL-01/02/03/06a use the production server and HTTPS driving path; S-VLL-07 uses the existing Driver adapter boundary; native tests stay in the existing in-process composition; E09 is the only black-box product expectation. The pure broker properties are explicitly a source-local policy lane. |
| CM-B — domain language and outcome assertions | **PASS** | The scenario contracts name workload, Service, Job, Running, Stable, stop, failure, cleanup and lifecycle outcomes. Technical fields remain only where ADR-0102/0103 make them the exact owned contract. The shell and Rust comments explicitly distinguish product observations from adapter evidence. |
| CM-C — complete user journey | **PASS** | S-VLL-01 is a thin end-to-end independent-progress journey through the real owner; S-VLL-12 drives the complete public E09 deployment, probe, peer and cleanup journey. Focused scheduling, host and guest matrices supply the internal guarantees that make those journeys true. |

## Roadmap handoff assessment

The three-step dependency order is executable against the approved API:

| Step | Handoff judgment |
|---|---|
| 01-01 — bounded complete evaluations | **Ready for implementation.** It starts with the real S-VLL-01 owner regression, carries the seven/eight-slot boundary, exact broker signatures, pure broker properties, shutdown snapshot and preservation regressions. No expectation is incorrectly assigned to this internal step. |
| 01-02 — responsive VM termination | **Ready for implementation.** It owns the existing Driver stop composition, private guest-init transition, host overlap, native adapter matrices and preserved narrow result contract. The pending native tests identify the exact process/wire/error cases without prescribing a new API. |
| 01-03 — native measurements and E09 | **Ready for implementation after 01-02.** It has one 1200-trial in-process measurement owner and one reused E09 black-box expectation. It explicitly fails on missing/forced/cleanup-invalid trials and keeps native evidence pending. |

The roadmap criteria at `roadmap.json:35-200` are behavioral and executable;
they do not contain implementation algorithms or invented APIs. The file lists
are guidance, and the acceptance criteria correctly allow compiler-required
fallout. Independent roadmap review has identified presentation/locator
metadata observations; those do not alter this acceptance verdict or justify
production changes. The roadmap remains pending until the root orchestrator
aggregates all independent reviews and performs the required mechanical
remediation.

## Findings and dispositions

No acceptance correctness finding was accepted. In particular:

1. The ten live RED tests are not placeholder panics: each reaches the real
   production or adapter entry point and fails on the named missing behavior.
2. The eight direct panic bodies are not coverage evidence, but they are
   sufficiently bounded authored specifications for implementation. Their
   pending status is disclosed in both the scenario table and roadmap notes.
3. The current host-safe scheduler pass is not treated as native VM evidence.
   The current full runner harness can fail in the timeout fixture when its
   TERM-resistant descendant survives; the unchanged-HEAD runner/harness
   baseline reproduces the same failure in Lima, while the six transcript
   validation cases pass. This is a known fixture limitation, not a reachable
   E09 product defect introduced by the two timeout-constant changes.
4. No finding is raised from the accidental native diagnostic archive. The
   author correctly labels it incomplete and excludes it from acceptance and
   latency claims.
5. No finding is raised for absent DEVOPS or DISCUSS artifacts because the user
   explicitly authorized inherited verification and skipped DEVOPS.

No remediation was performed by this reviewer. The only file written is this
review artifact. The parent orchestrator owns any roadmap documentation
remediation and must not turn a presentation issue into a new architecture or
API requirement.

## Verification performed

| Command or record | Result and limits |
|---|---|
| Read `AGENTS.md`, `CLAUDE.md`, `.claude/rules/bpf.md`, `debugging.md`, `design.md`, `development.md`, `rust.md`, `testing.md`, and `verification.md` | Pass. Repository-specific Rust, boundary, RED, Contract Shape, evidence and no-DEVOPS rules were applied. |
| Read `nw-acceptance-designer-reviewer.md`, `nw-ad-critique-dimensions`, `nw-test-design-mandates`, `nw-bdd-methodology`, `nw-distill`, and `nw-at-completeness-check` | Pass. The Rust/in-process and black-box adaptations follow the repository rules where generic skill examples differ. |
| Read `.context/vm-lifecycle-distill-review-brief.md`, `feature-delta.md`, `roadmap.json`, ADR-0102, ADR-0103 and current owner paths | Pass. Exact API and ownership contracts were checked against source and scenario mapping. |
| `git diff --check cc94b7c8 -- . ':(exclude)AGENTS.md'` | Pass; no whitespace errors in the reviewed DISTILL diff. |
| `jq -e . docs/feature/vm-lifecycle-latency/deliver/roadmap.json` | Pass. The roadmap remains `validation.status: pending`. |
| `bash -n` on the changed E09 example, scheduler test, active runner and runner harness | Pass. |
| `shellcheck` on those four shell files | Pass. |
| Author focused Lima nextest record `.context/vm-lifecycle-latency-distill/rust-r4vu5xqj/` | Exit 0 for 16 expected-hook/control tests: 10 behavioral RED, 5 selected pending panics and 1 healthy control; 1,418 unrelated tests skipped. |
| Author scheduler record and independent `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh` | Pass; shell orchestration only. |
| Author runner record and six individual transcript cases | Pass for transcript cardinality/identity modes; host-safe only. |
| Current full `bash verification/harness/test-e09-v2-runner.sh` | Exit 1 in the timeout fixture because TERM-resistant descendant `14860` survived. The unchanged-HEAD Lima baseline records the same fixture failure; no native product claim is made. |
| Author `cargo check`, Clippy, formatting and feature/roadmap validators | Reported pass. No broad duplicate run was needed for this acceptance review. |
| Native lifecycle profiles, amended E09 expectation, mutation testing | Not run; correctly pending for DELIVER. The accidental native archive was not used. |

## Iteration and verdict

### Iteration 1 — independent acceptance review

The acceptance scenarios, live RED preflight, pending bodies, production
driving paths, Contract Shape declarations, expectation/test boundaries,
scope, exact API references and roadmap handoff were reviewed against the
approved design. No acceptance-design finding requires remediation. The
roadmap's independent validation and metadata review remains outside this
verdict and is still pending aggregation.

**Verdict: APPROVED.** The DISTILL acceptance handoff is sufficiently precise
and honest for isolated implementation work. This approval does not mark any
scenario GREEN, approve `roadmap.json`, satisfy native performance/E09 gates,
or authorize mutation testing.
