# DELIVER implementation review — E09-v2 cohort barrier and failure predicate

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Review ID | `e09_v2_cohort_predicate_implementation_iteration_2` |
| Roadmap step | `02-04` bounded E09-v2 correction |
| Reviewer | Codex, `nw-software-crafter-reviewer` |
| Model | GPT-5.6 Luna, maximum thinking (inherited session model) |
| Review date | 2026-09-08 |
| Reviewed baseline | `b653e1ad1758d11be33be457849b284f78141333` plus the existing dirty worktree |
| Design authority | `docs/analysis/root-cause-analysis-e09-v2-failure-case-lifecycle.md` and `docs/analysis/review-e09-v2-failure-case-lifecycle.md` |
| Native state | The supplied native rerun is failed and remains unsatisfied; no native rerun was performed by this review |
| Iteration | 2 (remediation re-review) |

## Review boundary and authority

This review covers only the user-approved two bounded E09-v2 corrections:

- each ten-worker cohort waits until every worker has completed healthy peer and
  Service stop plus allocation-scoped runtime release before any failure
  Service deploy is submitted; and
- the failure predicate accepts a same-allocation `Running` recovery when the
  restart count is positive and a prior `Failed` snapshot is retained, while
  the original typed `StartupProbeFailed` stream evidence remains required.

The preserved contract is exactly twenty pairs, two cohorts of ten, effective
concurrency ten, one persistent control plane, no pair retry/replacement/
discard, the original nonzero typed failure stream, failed guest TCP port
`18999`, no `Stable`, the negative peer assertion, allocation runtime cleanup,
the remote 600-second plus 60-second windows, and the pre-existing 180-second
stop observation. No production Rust, public API, lifecycle owner,
persistence, scheduler, recovery, or architecture change is authorized.

The implementation files reviewed were:

- `examples/service-kind-vm-workloads-v2/run-example.sh`;
- `examples/service-kind-vm-workloads-v2/test-scheduler.sh`;
- `docs/analysis/test-e09-v2-failure-lifecycle.sh`;
- `docs/analysis/test-e09-v2-failure-submit-order.sh`;
- `examples/service-kind-vm-workloads-v2/README.md`; and
- `verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/README.md`.

The seeded Sim diagnostic
`crates/overdrive-sim/tests/e09_v2_failure_stream_overlap_spike.rs` is an
intentionally red counterexample and is not a target of this review. The
native c008-c010 disposal failure is recorded as a separate investigation;
it is not reclassified or given a predicate fix here.

## Baseline and scope audit

The two implementation changes are confined to the example shell owner and
its host-safe tests, with the expectation README documenting the new contract.
The current `run-example.sh` predicate at lines 662-670 requires the current
allocation to be `Running`, a positive restart count, a retained prior
`Failed` snapshot, the original typed startup-failure stream, and the failed
`18999` observation. The healthy cleanup marker is published only after
`assert_case_resources_released` at lines 955-960. The coordinator waits for
all ten markers at lines 1114-1121 and releases the new gate before worker
termination in `abort_cohort_workers` at lines 1065-1069.

The 180-second stop observation already existed in the recorded dirty
checkpoint (`38632d20`, `run-example.sh:697`) and is not attributed to this
crafter or treated as a new finding. The 600-second remote budget and
60-second cleanup grace are unchanged.

No public API or production crate changed in the reviewed correction. No
unrelated dirty file was reset, overwritten, or included in this review.

## Contract assessment

| Contract surface | Evidence | Result |
|---|---|---|
| Worker-side cleanup order | `run-example.sh:944-960` stops the healthy peer and Service, performs `assert_case_resources_released`, then publishes one marker and waits for `failure-submit-release` before `run_service_deploy` at `:966`. | Pass |
| All-ten coordinator barrier | `run-example.sh:1114-1121` waits for exactly the cohort count (`10`) before releasing failure submission. | Pass |
| Cancellation and missing-worker handling | `wait_for_markers` checks the worker PID/marker pair and fails promptly; `abort_cohort_workers` releases all gates, marks the cohort aborted, terminates workers, and waits for them. | Pass |
| Same-allocation Running predicate | `first_service_alloc_state` and `first_service_restart_count` read the first allocation row; `has_prior_failed_snapshot` scans only that row's history before the next allocation or `Memory:`. The branch still requires the original typed stream and failed `18999` probe. | Pass |
| Independent failure oracles | The `Failed` branch remains; `Stable` stream, generic Running, other-allocation history, generic error, and timeout-only controls remain rejected. | Pass |
| Pair/concurrency/control-plane contract | Pair materialization, concurrency selection, one serve process, ordered ledger, peer checks, cleanup checks, and no retry/replacement/discard behavior remain outside the bounded correction. | Pass |
| Budget and pre-existing stop wait | The runner's 600+60 windows are unchanged; the pre-existing 180-second stop observation is preserved. | Pass |
| Operator documentation consistency | Iteration 1 found the obsolete beacon-bind wording and missing all-ten barrier text. Iteration 2 verified the README-only remediation against the implementation and expectation README. | Pass after remediation |

## Production and example owner paths

The reachable example path is `run-example.sh run tcp-truthfulness-20` through
one `run_suite`, one `start_serve`, two calls to `run_cohort`, and ten
background `run_trial` workers per cohort. A worker reaches the new marker
only after the public healthy peer/Service stop and the allocation-scoped
runtime release assertion. The owner then counts the ten marker files and
opens the failure-submit gate. The worker's next public boundary is the
existing failure deploy; no production lifecycle/API path is changed.

The predicate path is `wait_for_service_failure` polling public describe output
while retaining the immutable original deploy stream. The current allocation
row supplies `Running` and the positive restart count; its embedded
`last terminated: Failed` history supplies the same-allocation recovery
evidence. The original stream supplies typed `StartupProbeFailed`, and the
current describe supplies the failed guest TCP `18999` observation. This is the
exact shape authorized by the reviewed proposal.

## Findings, reachability, and reproduction

### F-01 — Operator README retains the obsolete bind-error contract

**Severity:** Medium, blocking this implementation review until the bounded
documentation correction is made.

**Location:** `examples/service-kind-vm-workloads-v2/README.md:25-35`.

The operator README says that a recovered `Running` allocation is accepted
only with the typed prior `bind beacon listener: Address already in use`
termination (`:26-29`). The reviewed implementation intentionally accepts the
same allocation's positive restart count plus prior `Failed` snapshot without
that incidental detail (`run-example.sh:657-670`), and the expectation README
now documents that approved shape
(`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/README.md:25-32`).
The operator README also describes only the old healthy-Running and
failure-active barriers (`:33-35`) and does not tell an operator that failure
submission is released only after all ten `healthy-cleanup-complete` markers.

This is directly reachable through the documented operator journey: an
operator reading the example README is instructed to reject a trajectory that
the current example accepts, and is not told about the ordering guarantee that
selects the approved E09-v2 schedule. The inconsistency is not a theoretical
production failure and requires no new architecture to reproduce. The focused
check produced:

```text
DOC_CONSISTENCY_FAIL: operator README still requires incidental beacon-bind detail while expectation accepts same-allocation prior Failed
```

**Required remediation:** update only the example README's failure predicate
wording to require positive restart count plus the same allocation's prior
`Failed` snapshot while preserving the original typed stream and `18999`
oracles. Update its scheduler paragraph to state that each worker completes
healthy peer/Service stop and runtime cleanup before the all-ten
`healthy-cleanup-complete` barrier releases failure submission. Do not alter
the production/API/architecture boundary, budgets, or pre-existing stop wait.

**Disposition:** Accepted for remediation by the original crafter of this
bounded step. No new design decision or production mechanism is needed.

**Iteration 2 remediation disposition:** Closed. The original crafter updated
the operator README only; the corrected wording now matches the expectation
README and the implementation's predicate and all-ten barrier. The focused
consistency check passes.

No other finding met the repository threshold of a concrete reachable failure
and a reproducible in-scope defect. In particular, repeated cancellation
execution did not reproduce a partial failure cohort, and the supplied native
c008-c010 disposal failure was not duplicated or converted into a new
predicate requirement.

## Test integrity and Contract Shape compliance

The new scheduler cases carry `CONTRACT_SHAPE: bounded-change` at
`test-scheduler.sh:252` and `:286`. The P1 predicate diagnostic and P4
worker boundary diagnostic carry the same declaration. The tests keep the
required boundary separation: scheduler tests source the example orchestration
but use surrogate workers and private files, while the native expectation
remains the only built-product black-box driver. No test invokes Cargo, a Rust
test binary, or an `overdrive-*` crate.

The P1 change adds negative controls for generic Running, history belonging to
another allocation, a Stable stream, and a generic error stream. It does not
weaken the original accepted/rejected controls. The P4 check observes the
actual `run_trial` prefix through external adapters and now requires cleanup
before the post-cleanup gate. The coordinator tests assert both all-ten
release ordering and cancellation without a partial failure cohort. No deleted,
skipped, tautological, or weakened assertion was found.

The existing `e09_v2_failure_stream_overlap_spike` remains intentionally red;
the approved workflow explicitly treats it as the seeded diagnostic
counterexample and does not authorize making it green by changing product
behavior.

## Independent verification

| Command | Result |
|---|---|
| `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh all` | **PASS** — capacity, manifest, ledger, two ten-worker cohorts, all-ten cleanup barrier, and cancellation guard |
| `bash verification/harness/test-e09-v2-runner.sh all` | **PASS** — valid twenty-row transcript, malformed counts/identity, failed row, timeout, TERM, descendant cleanup, and partial output |
| `bash docs/analysis/test-e09-v2-failure-lifecycle.sh` | **PASS** — failed row, bind recovery, no-bind same-allocation recovery, timeout rejection, generic Running rejection, other-allocation history rejection, Stable rejection, and generic-error rejection |
| `bash docs/analysis/test-e09-v2-failure-submit-order.sh` | **PASS** — `own_cleanup=1 cohort_gate_after_cleanup=1` |
| `bash -n` on the four reviewed shell files | **PASS** |
| `shellcheck` on the four reviewed shell files | **PASS** |
| Fifteen repeated `cleanup-cancellation` runs under a 25-second bound | **PASS** — no partial failure submission reproduced |
| Documentation consistency check against both READMEs (iteration 1) | **FAIL** — F-01 reproduced |

### Iteration 2 focused verification

The original crafter changed only `examples/service-kind-vm-workloads-v2/README.md`
for F-01. The corrected operator text now states the positive restart count,
same-allocation prior `Failed` snapshot, original immutable typed stream, and
rejection of generic Running/Stable outcomes. It also states the worker cleanup
marker and all-ten cohort gate before failure submission. The expectation README
and implementation use the same terms.

| Command | Result |
|---|---|
| Documentation consistency check against both READMEs | **PASS** — `DOC_CONSISTENCY_PASS` |
| `bash docs/analysis/test-e09-v2-failure-lifecycle.sh` | **PASS** — all eight predicate controls |
| `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh all` | **PASS** — all-ten barrier and cancellation guards remain green |
| `bash verification/harness/test-e09-v2-runner.sh all` | **PASS** |
| `bash -n` and `shellcheck` on the reviewed shell files | **PASS** |
| `git diff --check` on the corrected README and review artifact | **PASS** |

The correction is documentation-only and does not change the production/API/
architecture boundary. No native command was run during re-review.

The retained native evidence records `exit: 1` in
`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/product-run.meta:9`,
with the later c008-c010 disposal message in `product-run.out:1490`. This
review did not rerun native metal and does not mark the expectation satisfied.
The independent disposal investigation remains the authority for that later
failure; no cause or remediation is asserted here.

No mutation testing, commit, issue, native rerun, or production/API change was
performed by this review.

## Iteration log and remediation disposition

### Iteration 1 — completed

- Read the mandatory repository instructions, reviewer role, and applicable
  review/TDD skills.
- Read the accepted E09-v2 RCA and proposal, including the all-ten and
  cancellation conditions.
- Audited the changed worker, coordinator, predicate, test, and expectation
  README paths against the exact approved contract.
- Reproduced the bounded host-safe checks and retained native failure state.
- Accepted F-01 as a documentation consistency finding; no production or
  architectural finding was promoted.

The original crafter must remediate F-01 and the same reviewer must re-review
this step. The native expectation remains pending/failed independently of this
documentation remediation, and the next roadmap step must not start from this
review alone.

### Iteration 2 — completed

- Re-read the original F-01 finding and the accepted proposal conditions.
- Verified the crafter's README-only remediation against the expectation README
  and the live example predicate/coordinator paths.
- Re-ran the focused documentation, predicate, scheduler, runner-harness,
  syntax, and shellcheck checks; all passed.
- Confirmed that the existing native evidence is still `exit: 1` and was not
  rerun or relabeled.

F-01 is closed. No new reachable in-scope defect was found, and no production,
API, architecture, native expectation, or mutation work is authorized by this
re-review.

## Verdict

**APPROVED for the bounded E09-v2 expectation corrections.** The cohort
barrier, cancellation guard, same-allocation failure predicate, preserved
failure oracles, and host-safe evidence satisfy the accepted proposal. F-01 is
closed after the README-only remediation and focused re-review. This verdict
does not mark the native expectation satisfied: the retained c008-c010 native
failure remains `exit: 1`, and E09-v2/step `02-04` completion still requires a
separate valid native result.
