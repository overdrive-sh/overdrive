# Independent review — E09-v2 failure-case lifecycle RCA and fix proposal

## Metadata

| Field | Value |
|---|---|
| Review ID | `rca_rev_20260908_e09_v2_failure_case_lifecycle_iteration_1` |
| Reviewer | Codex, `nw-troubleshooter-reviewer` |
| Review date | 2026-09-08 |
| Reviewed artifact | `docs/analysis/root-cause-analysis-e09-v2-failure-case-lifecycle.md` |
| Review scope | E09-v2 fresh native failure, the two proposed bounded example corrections, and their reproducible diagnostics |
| Native capture | SHA `b653e1ad1758d11be33be457849b284f78141333`, native-metal, seed 1, exit 1 |
| Review iteration | 2; F-02 evidence-only re-review completed, with no broad remediation iteration |
| Final status | **APPROVED WITH CONDITIONS** for the RCA and bounded proposal; implementation and final native acceptance remain pending |

## Review mandate and authority

This review evaluates why the fresh E09-v2 expectation failed and whether the
proposed correction follows the accepted example contract. It does not mark
DELIVER step `02-04` complete, approve a production change, or authorize a
change to the stream cap, serial dispatcher, lifecycle owners, retry policy,
or public API.

The authoritative acceptance boundary is S-SVM-25 in
`docs/feature/service-kind-vm-workloads/distill/test-scenarios.md:57-107`:
twenty unique pairs, two ten-pair cohorts, one unchanged persistent
control-plane process, original typed `StartupProbeFailed` outcomes, no pair
retry/replacement/discard, negative peer observations, runtime reclamation,
and the existing 600-second setup/trials plus 60-second cleanup envelope.
ADR-0095 at `docs/product/architecture/adr-0095-service-stream-cap-startup-deadline.md:74-120`
also remains authoritative: `Timeout` is a stream-only result when its 90
second cap wins; it must not be reclassified into a lifecycle terminal.

## Baseline and preservation audit

The review read the RCA, both shell diagnostics, the seeded Sim spike, the
current v2 example and README, the accepted S-SVM-25/ADR paths, and the
retained native evidence. The tree already contained dirty changes in
`AGENTS.md`, the example, execution logs, diagnostics, Sim tests, and E09
evidence. Those files were not edited. Only this review artifact is owned by
this review.

No production process, expectation runner, terminal observation row, public
API, design artifact, roadmap file, DES record, or commit was changed.

## Executive assessment

The RCA identifies two separate, reachable example defects and does not
promote either into an invented product contract change:

1. `run_trial` submits a failure Service after its own healthy cleanup while
   peer healthy stops still occupy the production serial convergence path.
   The failure stream's independent 90-second cap can therefore close with
   `Timeout` before `ServiceLifecycle` can publish `StartupProbeFailed`.
2. The v2 shell predicate accepts a restarted `Running` allocation only when
   its human history contains the incidental
   `bind beacon listener: Address already in use` detail. Native c006 and the
   Sim recovery control show valid typed startup failure and same-ID recovery
   without that detail.

The first cause is a runner ordering defect; the second is an overly narrow
runner oracle. The evidence does not prove a product stream violation or a
need to alter production lifecycle/recovery behavior. The proposed cohort
barrier and predicate correction are therefore directionally correct and stay
within the accepted scope.

The proposal still needs a bounded passing coordinator guard before final
acceptance. The existing P4 diagnostic proves the worker reaches failure
submission before a post-cleanup gate, but it does not prove that a new
coordinator waits for all ten cleanup completions or releases/cancels workers
when one completion is missing. The RCA explicitly acknowledges this gap; it
is a condition on the eventual implementation and native rerun, not a reason
to invent a new production mechanism.

## Independent reproduction and evidence

### Native capture

The retained native evidence is internally consistent with the RCA:

| Observation | Evidence | Review result |
|---|---|---|
| c001 original failure deploy returns nonzero `Timeout` | `verification/.../evidence/run.log:464-472`; command exit code `1`, reason `workload did not converge within 90s` | Pass; this is an immutable original transcript and cannot be repaired by later polling |
| c001 failure VM guest EXEC is not released until about 87 seconds into that deploy | `run.log:311-312` at `19:14:24.014963Z`/`19:14:24.015192Z`; deploy begins at `19:12:57Z` | Pass; this leaves only about three seconds before the 90-second stream result |
| c001 later recovers under the same allocation identity | `run.log:351-353` and `:473-494`; `Running`, restart `1`, prior `Failed`, failed TCP `18999` probe | Pass; later recovery cannot change the closed c001 stream |
| c006 has typed startup failure followed by Running/restart without bind detail | `run.log:1143-1175` and the following final-describe snapshot | Pass; this falsifies the predicate's bind-error requirement |
| c008 also exhibits the bind-error trajectory | `run.log:1417` onward and its final-describe snapshot | Pass; it remains a valid control, not the universal rule |
| healthy cleanup completes but varies materially | timing sections `run.log:169-256`, including 12,591 through 130,548 ms | Pass; the captured issue is not another 60-second healthy-stop observation rejection |

The final c001 ledger label is not treated as causal evidence. The original
`failure-deploy.out` proves the deploy ran; the RCA correctly explains that
`worker_cleanup` at `run-example.sh:811-836` can be interrupted before its
result is written, after which aggregation fills a cancellation row. The
`/proc` `awk` race and later `comm`/cleanup residue are recorded as incidental
capture noise. They do not alter the immutable c001 transcript, and no fix is
required for them by this RCA.

### P1 — predicate regression

I independently ran:

```sh
bash docs/analysis/test-e09-v2-failure-lifecycle.sh
```

It exited `1` with the expected diagnostic result:

```text
checked checked-in E08 sources for E09-v2
failed_control: expected=accepted observed=accepted
bind_recovery_control: expected=accepted observed=accepted
original_c001_timeout_control: expected=rejected observed=rejected
no_bind_recovery_regression: expected=accepted observed=rejected
```

The script sources the actual `wait_for_service_failure` function from
`examples/service-kind-vm-workloads-v2/run-example.sh`; it does not grep the
function body or reimplement its branch. It substitutes only the describe
query and logical polling clock, and uses native product-derived stream and
describe excerpts. The controls establish both necessary non-regressions:
the generic c001 timeout remains rejected and the existing bind-error
trajectory remains accepted. The c006 no-bind trajectory is the reproducible
failure.

The P1 boundary intentionally tests the predicate rather than the full worker
deploy. The real worker checks the deploy return code at
`run-example.sh:940-942`; the embedded native stream also retains
`COMMAND_EXIT_CODE="1"`. P1 therefore does not claim to replace the full
native acceptance oracle.

### P2 — recovery and eligibility control

The RCA's existing `e09_v2_failed_service_reachability_spike.rs` is a valid
seeded Sim control. It exercises production WorkloadLifecycle,
ServiceLifecycle, action dispatch, observation persistence, and ProbeRunner
composition. It demonstrates a typed startup terminal, same-ID Running
recovery, retained `last_terminated.terminal`, and an unhealthy backend. No
terminal row is fabricated and no public API is added.

### P3 — stream-overlap regression

I independently ran the canonical Lima/nextest command:

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-sim \
  --features integration-tests,overdrive-control-plane/integration-tests \
  --test e09_v2_failed_service_reachability_spike \
  --test e09_v2_failure_stream_overlap_spike --no-capture
```

The command built successfully and ran three tests. Nextest run
`84dc67d2-4436-4360-b798-e11f0b283f41` exited `100` because exactly one
assertion failed: two tests passed and the seeded overlap test failed at
`crates/overdrive-sim/tests/e09_v2_failure_stream_overlap_spike.rs:320`.
The decisive output was:

```text
e09-v2-failure-stream seed=257210 overlap=true
authored=Some(ServiceFailed { reason: StartupProbeFailed { ... } })
recovered_state=Running
recovered_last_terminal=Some(ServiceFailed { reason: StartupProbeFailed { ... } })
stream=Failed { alloc_id: None, reason: Timeout { after_seconds: 90 }, ... }
seed=257210: E09 requires StartupProbeFailed on the original unbound deploy; got Timeout
```

The separated control with the same owners, inputs, probe outcomes, and stop
delay passed and emitted `StartupProbeFailed`. The Sim spike uses a private
driven-port delay only to model an awaited healthy stop; it calls the existing
per-tick production owner path and real Service stream. It does not claim to
measure native VM teardown or broker selection. That boundary is honest and
adequate for the ordering invariant required here: the native code already
shows the VM stop effect is awaited, while P3 proves the cap collision when
those effects are selected serially.

P3 is deliberately red under an example-only correction. It must remain a
diagnostic counterexample; changing its assertion to make the old overlap
schedule pass would require a different product contract and would violate the
accepted scope.

### P4 — actual worker ordering regression

I independently ran:

```sh
bash docs/analysis/test-e09-v2-failure-submit-order.sh
```

It exited `1` with:

```text
checked checked-in E08 sources for E09-v2
failure-submit: own_cleanup=1 cohort_gate_after_cleanup=0
FAIL: failure submit is reachable before a post-cleanup cohort gate
```

The script sources and executes the real `run_trial` prefix. Existing external
adapters provide only the healthy-success phase and stop at the actual failure
deploy call with exit `42`; no product state, terminal row, VM workload, or
test-only production loop is created. This is a behavioral order check, not a
source-shape grep. Its `# CONTRACT_SHAPE: bounded-change` declaration is
present at lines 11-15.

The P4 witness is meaningful but intentionally incomplete. Once the proposed
worker gate is implemented, the assertion should become a passing guard that
requires `own_cleanup=1` before `cohort_gate_after_cleanup=1`. It cannot by
itself prove ten-marker coordination or cancellation behavior.

### Contract Shape and test-boundary audit

The Rust P3 properties carry the exact required rustdoc declarations at
`e09_v2_failure_stream_overlap_spike.rs:380` and `:389`. The shell diagnostics
carry `CONTRACT_SHAPE: bounded-change` declarations. Rust tests use nextest and
the production composition in process; they do not spawn the Overdrive binary.
The P1/P4 diagnostics are outside the black-box expectation boundary and do
not emit expectation evidence or invoke Cargo tests. This separation is
correct.

## Production owner-path and reachability review

The causal path is reproduced through the relevant production boundaries:

1. `run-example.sh:839-944` starts each worker, waits for the existing
   healthy-active gate, performs its own public stop and runtime-release check,
   and immediately submits its failure Service.
2. `run-example.sh:1039-1091` releases all healthy workers after the active
   marker but waits directly for `failure-active`; it has no all-healthy-cleanup
   marker or failure-submit release between those points.
3. Public deploy/stop reach the existing control-plane handlers and intent
   store. The production convergence loop at
   `crates/overdrive-control-plane/src/lib.rs:3360-3389` awaits each pending
   `run_convergence_tick` serially before checking shutdown.
4. `VmDriver::stop` at `crates/overdrive-worker/src/vm_driver.rs:1697-1755`
   awaits beacon shutdown, VMM termination grace, and cleanup effects. The
   action is not detached; a selected batch can therefore consume the stream
   budget.
5. `streaming.rs:886-981` starts the existing cap and closes with
   `Timeout { after_seconds: 90 }` when no projectable lifecycle terminal has
   arrived. It cannot reopen the original stream after a later terminal.
6. `ServiceLifecycle::startup_probe_failed_action` at
   `service_lifecycle.rs:1257-1317` authors the typed startup failure only
   after its attempts/deadline/no-pass gates. WorkloadLifecycle then treats
   the standing Service intent as restartable at `workload_lifecycle.rs:827-975`
   and builds a same-ID restart at `:1337-1383`.

This establishes a complete owner/state/order path for both defects. The exact
native allocation batch that delayed c001 is not claimed: the RCA correctly
labels the observed staircase as evidence of the schedule and uses seeded P3
to isolate the sufficient serial-stop mechanism. No theoretical future,
forced test abort, fabricated row, or unreachable cancellation state was
promoted to a finding.

The history-rendering distinction is also correct. The retained
`last_terminated.terminal` is a typed product fact, while the human renderer
prints the prior allocation's `reason`/detail. Thus c006's `driver started`
history text does not prove loss of `StartupProbeFailed`; P2/P3 directly verify
retention. A human rendering enhancement would require a separate design
decision and is outside this proposal.

## Alternative hypotheses

The RCA does not tunnel on the first plausible explanation. It separately
tests or bounds the principal alternatives:

| Alternative | Disposition |
|---|---|
| A stream `Timeout` is a product contract violation | Rejected. ADR-0095 explicitly permits stream-only Timeout after the cap; the stronger S-SVM-25 example contract is what fails. |
| Polling later can repair c001's original result | Rejected. The stream terminal is immutable after close; c001's transcript remains Timeout. |
| Same-ID restart losing the typed terminal requires a product fix | Rejected. P2/P3 retain the typed terminal and keep the backend ineligible. |
| Beacon bind collision is required for valid recovery | Rejected by native c006 and P2/P3 no-bind recovery. |
| The old 60-second cleanup observation window caused this run | Rejected by all healthy terminal/zero-runtime observations and the captured 130,548 ms maximum; the current dirty 180-second observation correction is distinct. |
| Native `/proc`/`comm` cleanup messages caused c001's Timeout | Unproven and not needed: they occur as incidental/rundown evidence, while the immutable c001 stream and P3/P4 failure are independent. |
| Serial dispatcher remediation #283 or a production lifecycle redesign is needed | Outside scope and not established. The bounded example barrier removes the reachable selected-stop overlap without changing the dispatcher. |

Residual uncertainty is stated rather than hidden: P3 does not prove native
VM duration distributions or that every one of the nine native peers belonged
to c001's exact convergence batch. The proposed final native rerun is the
correct evidence gate for that remaining uncertainty.

## Proposal assessment

### Cohort barrier

The proposed barrier is causally sufficient for the reproduced defect if its
implementation has this exact ownership/order:

- each worker calls the existing public healthy peer/Service stop and runtime
  release check;
- only after that check does it publish one healthy-cleanup-complete marker;
- the cohort owner waits for all ten markers, then publishes one
  failure-submit-release gate;
- each worker waits for that gate before invoking failure deploy; and
- the existing failure-active gate and failure-release gate remain in place.

This waits for the effect that matters to the serial convergence queue, not
just a stop request or a local command return. It keeps ten workers, one
control plane, the existing pair identities, and the 600+60 envelope. It does
not widen the stream cap, alter retry/replacement behavior, or require a
production API.

The cancellation path must release the new gate before terminating workers.
Otherwise a worker waiting after healthy cleanup could remain blocked until
the marker timeout, and the coordinator behavior would not be demonstrated as
bounded. The existing `abort_cohort_workers` at `run-example.sh:1033-1037`
currently touches only `healthy-release` and `failure-release`; this is a
concrete implementation obligation for the new example-only marker. It is a
bounded coordination update, not an architectural expansion.

The proposal must also add a host-safe coordinator regression with two
independent cases: all ten markers release failure submission only after the
tenth cleanup completion, and a missing/failed worker causes the coordinator
to release/cancel the cohort within the existing owner bound without allowing
a partial failure cohort to be reported as success. P4 remains the worker
boundary guard; it is not sufficient evidence for this coordinator half.

### Predicate correction

The proposed predicate shape is valid when it retains every independent oracle:

- deploy exit is nonzero;
- the immutable original stream contains typed `StartupProbeFailed`;
- the current public describe has the failed guest TCP startup observation at
  port `18999`;
- either the current allocation is `Failed`, or it is the same E09 allocation
  with a positive restart count and a prior `Failed` snapshot;
- the original stream contains no Stable result; and
- the negative peer Job still completes only as the unreachable assertion.

The Running branch should use the public `last terminated: Failed` snapshot
and the positive restart count, while relying on the original stream for the
typed terminal. The current human renderer does not expose
`last_terminated.terminal`; requiring that unavailable field would silently
recreate the defect or invent a CLI/API change. A generic `Running` row,
generic deploy error, timeout-only stream, or Stable stream must remain
rejected.

This correction accepts permitted product recovery across the existing
same-ID restart path. It does not call the restart a runner retry or replace a
trial, and it does not make the recovered allocation backend-eligible.

### Native acceptance and budget

The proposal does not claim that the barrier alone proves all twenty native
pairs. That is the correct boundary. The final run must still use the
canonical native wrapper, exclusive lease, exact concurrency ten, one
persistent control plane, original pair outcomes, negative peers, runtime
cleanup, and the unchanged 600-second plus 60-second windows. If another
bounded native cause remains after the barrier, the first-attempt evidence
must remain a failure; widening the cap or accepting a generic timeout would
violate S-SVM-25 and ADR-0095.

## Findings and dispositions

### F-01 — Coordinator cancellation and all-ten guard is an implementation condition

**Severity:** Medium, non-blocking for the RCA/proposal review.

**Evidence:** The RCA says P4 should become green after the worker gate but
also states that a separate coordinator test is needed. Current
`abort_cohort_workers` only releases the two existing gates
(`run-example.sh:1033-1037`), and current `run_cohort` waits only for
`healthy-active` and `failure-active` (`:1060-1091`).

**Impact:** A partial barrier implementation could let one worker wait on a
new gate forever until the generic marker timeout, or could release failure
submission before all ten healthy cleanup effects complete. P4 alone would
not detect either coordinator error.

**Disposition:** Accepted as an explicit implementation condition, not a
production defect. Add the bounded all-ten and missing/failed-worker
coordinator guard, release the new gate in cancellation, then require it and
P4 to pass before the final native E09 run. No new persistence, scheduler,
control-plane, or public API mechanism is authorized or needed.

### F-02 — Timing-label concern disproven on re-review

**Initial severity:** Low, evidence-quality correction.

**Initial disposition:** Open in iteration 1.

**Re-review evidence:** The RCA now documents the actual report-generation
  order at its timing identity check. `run-example.sh:1169-1171` finds each
  `timing.tsv` and applies NUL-safe lexicographic `sort -z`; the case paths are
  therefore ordered `1, 10, 2, 3, 4, 5, ...`, not numeric order. The block at
  `run.log:205-213` is consequently c004 and reports 85,809 ms. The immutable
  c004 transcript at `run.log:879-887` names `cases/4/failure-service.toml`,
  starts at 19:13:34Z, ends at 19:15:00Z, and reports typed startup failure
  with exit code 1. The next timing block at `run.log:214-222` is c005 and
  reports 73,690 ms; its immutable transcript at `run.log:1011-1019` names
  `cases/5/failure-service.toml`, starts at 19:13:46Z, and ends at 19:15:00Z.
  The preceding 90,121 ms block is c003, whose Timeout transcript is
  `run.log:743-751`.

**Disposition:** **CLOSED — DISPROVEN.** The iteration-1 relabeling was caused
  by incorrectly reading the concatenated timing blocks in numeric case order.
  The original c004 label and 85,809 ms citation are correct, and the RCA's
  tightened citation now explains why. Remove this from the active findings
  count; no RCA, production, example, or test change is required.

### F-03 — Native pass remains an evidence gate, not a promised result

**Severity:** Low, informational.

**Evidence:** One native failure deploy (c004) took 85,809 ms; the RCA
explicitly notes that P3 does not prove native boot distributions or total-run
budget. The barrier addresses the proven healthy-stop backlog but cannot
mathematically guarantee every native failure stream completes before 90
seconds.

**Impact:** Treating the proposal as a guaranteed native pass would turn a
bounded hypothesis into an unsupported claim and could invite an unapproved
timer widening if the rerun exposes another cause.

**Disposition:** Already correctly handled by the RCA's final-acceptance
section. Retain the caveat and preserve any new first-attempt failures for a
separate bounded investigation.

No other finding met the reachability and reproducibility threshold. In
particular, no product remediation is requested for the permitted stream
Timeout, same-ID restart, renderer detail omission, serial dispatcher, or
incidental cleanup diagnostics.

## Six-dimension RCA score

| Dimension | Score | Assessment |
|---|---:|---|
| Causality logic | 8/10 | Separates overlap/Timeout from predicate/bind coupling, traces both through production owners, and uses P3/P4 to establish reachability. Native exact batch membership remains explicitly unclaimed. |
| Evidence quality | 9/10 | Native lines/timestamps, immutable command exit codes, native-derived predicate fixtures, seeded Sim output, and the worker-order reproduction are independently verifiable; the re-review closes the timing-label concern. |
| Alternative hypotheses | 8/10 | Evaluates stream semantics, cleanup-window history, terminal retention, bind ordering, incidental capture noise, and out-of-scope #283; remaining native schedule uncertainty is disclosed. |
| Five-why depth | 9/10 | Both branches reach actionable WHY-5 causes and include backward/forward validation without turning a symptom into a product contract. |
| Completeness and coverage | 8/10 | Covers c001 Timeout, c006 no-bind recovery, c008 control, terminal retention, backend veto, peer/runtime/budget boundaries, and cancellation evidence. The coordinator guard is a stated follow-up condition. |
| Solution traceability | 8/10 | Barrier maps to the serial-stop cause and predicate change maps to the incidental bind requirement. The added marker's cancellation/all-ten guard must be implemented and proven. |
| **Overall** | **8.3/10** | Above the approval threshold; no dimension is below 5. |

## Verification record

| Command | Result |
|---|---|
| `bash docs/analysis/test-e09-v2-failure-lifecycle.sh` | Expected red P1: exit 1; three controls pass and no-bind recovery is rejected. |
| `bash docs/analysis/test-e09-v2-failure-submit-order.sh` | Expected red P4: exit 1; actual worker reports cleanup before no post-cleanup gate. |
| Canonical Lima/nextest command shown in P3 | Compile/setup passed; 3 tests ran, 2 passed, seeded overlap invariant failed, exit 100. |
| `bash -n docs/analysis/test-e09-v2-failure-lifecycle.sh docs/analysis/test-e09-v2-failure-submit-order.sh examples/service-kind-vm-workloads-v2/run-example.sh` | Pass; no shell syntax/setup failure was mistaken for the intended red assertions. |
| Native evidence inspection | Pass; `verification.yaml`, `product-run.meta`, retained transcripts, timing, and cleanup sections agree on native-metal seed 1, exit 1, 600+60 ownership. |

No mutation run, native rerun, production binary spawn, commit, or DELIVER
phase event was performed by this review.

## Iteration log and remediation disposition

### Iteration 1 — completed

- Read the mandatory project instructions and reviewer criteria.
- Read the full RCA and all supplied diagnostics/evidence boundaries.
- Reproduced P1, P3, and P4 independently.
- Audited the production deploy, stop, convergence, stream-cap, startup-failure,
  restart, terminal-retention, and backend-veto paths.
- Verified exact acceptance scope, test boundaries, Contract Shape declarations,
  and native provenance.
- Recorded F-01 as the bounded coordinator guard condition, F-02 as the
  native timing-label correction, and F-03 as an informational native-pass
  limitation.

No remediation iteration is applicable: this review owns only the Markdown
artifact, and the requested production/example/test/design files were
explicitly preserved.

### Iteration 2 — F-02 evidence-only re-review completed

The investigator challenged the iteration-1 timing-label finding and tightened
the RCA's citations. I re-read the unchanged native `run.log`, the current
report-generation sort at `run-example.sh:1169-1171`, and the immutable c004,
c005, and c003 command transcripts. The lexicographic path order proves that
the 85,809 ms block is c004, not c005. F-02 is therefore disproven and closed;
the RCA's retained c004 label is correct. No broad test rerun was needed for
this evidence-only correction.

F-01 remains an implementation obligation for the example coordinator's
all-ten/cancellation guards. It is non-blocking for architecture and does not
authorize a production mechanism or scope expansion. The user-approved
proposal implementation may proceed under that bounded condition.

## Verdict

**APPROVED WITH CONDITIONS.** The RCA is causally sound, evidence-backed, and
appropriately classifies both failures as bounded E09-v2 example defects. The
iteration-1 F-02 timing-label concern is closed as disproven; the c004 label
and 85,809 ms citation are correct under the report's lexicographic timing
order. The cohort barrier and predicate correction are traceable to the
reproduced causes and do not require a production fix or invented API. Before
implementation is accepted, P1 and P4 must become meaningful passing guards
under the corrected example, and a separate bounded coordinator guard must
prove all ten cleanup markers, new-gate release on cancellation, and no
partial-cohort success. The overlapping P3 diagnostic must remain a red
counterexample. Only after those guards pass should the canonical native E09-v2
run be used to assess the unchanged twenty-pair, concurrency-ten,
persistent-control-plane, negative peer, cleanup, and 600+60 acceptance
contract.
