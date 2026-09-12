# DISTILL Review: VM Lifecycle Latency Scope, History, and Outcomes

## Review metadata

| Field | Value |
|---|---|
| Feature | `vm-lifecycle-latency` |
| Review role | Independent product-owner review of DISTILL scope, provenance, and acceptance outcomes |
| Reviewer | `nw-product-owner-reviewer` (Codex, GPT 5.6 Luna, maximum thinking) |
| Iteration | 2 (re-review complete) |
| Design authority | Approved `feature-delta.md`, ADR-0102, and ADR-0103; user-ratified scope recorded in `feature-delta.md` |
| Implementation baseline | `cc94b7c87e5dbefd5ff8364779c676a65edd7abd` (`docs: approve VM lifecycle latency design`) |
| Roadmap status | `docs/feature/vm-lifecycle-latency/deliver/roadmap.json` remains `validation.status = pending`; this review does not approve the roadmap |
| Verdict | `APPROVED` |

## Review boundary and user scope

This review covers the bounded DISTILL questions assigned to it: whether the change history is accurately represented, whether the proposed E09 v2 outcome is the right product outcome for the prior feature, whether the acceptance narrative covers that outcome without adding scope, and whether the author’s validation provenance is independently followable. Exact public API shape, detailed test honesty, and roadmap mechanical review are separate review responsibilities.

The user explicitly authorized a DESIGN-only input, skipped DEVOPS because there are no production users, and clarified that an existing expectation should be reused where it fits while a new expectation is acceptable if a distinct operator outcome needs one. The user also directed the designer to inspect Git history for the concurrent-deployment failure. Those decisions are reflected in the DISTILL. There is no finding for the absent DISCUSS or DEVOPS artifacts, and this review does not invent deployment readiness, rollback, or a new fault inventory.

The selected evidence boundary is also correct. Rust and seeded `overdrive-sim`/in-process tests carry the detailed scheduling, ownership, lifecycle, and cleanup guarantees. E09 v2 is the operator-facing black-box check of the existing VM service journey. The native E09 run is explicitly pending. The accidental native diagnostic archive is explicitly excluded from evidence and is not used for this verdict.

## Design and acceptance authority

The DISTILL keeps the approved architecture and public surface intact. The ownership paths, actual target identity (`workload/<workload_id>`), runtime view identity, bounded evaluation model, FIFO admission, shutdown drain, stop composition, guest supervision, and named profiles remain those established by ADR-0102/0103 and the approved DESIGN. The DISTILL changes the validation expression and corrects the E09 v2 scheduling sequence; it does not add an API, a persistence boundary, a recovery protocol, or a new lifecycle mechanism.

The E09 choice is a natural reuse. The existing example already drives the 20-pair VM service outcome and the existing expectation already observes the built product through one `serve`, two ten-worker cohorts, the complete ledger, resource cleanup, and control-plane identity. Changing that checked-in example and its expectation runner exercises the same stakeholder-visible journey with the corrected concurrent-deployment ordering. A second expectation ID would duplicate the same operator journey, while the detailed latency distributions belong in the Rust/in-process layers described by the DISTILL. The absence of a new expectation ID therefore follows the user’s clarified preference and does not weaken the previous product guarantees.

The resulting roadmap acceptance sentence is consistent with that scope: `docs/feature/vm-lifecycle-latency/deliver/roadmap.json:154-155` requires all 20 truthful pairs at concurrency ten through one `serve`, independent failure submission, the original 60-second observation/600-second owner/60-second cleanup windows, the complete ledger, and unchanged resource predicates. It separately keeps Rust/in-process evidence distinct from the native black-box expectation. The roadmap’s pending validation state remains an honest execution status rather than an unresolved product-scope defect.

## History and current-behavior audit

The author’s history account was checked against the commits and their parent/current files. The causal account is accurate and the current edits implement the stated disposition.

| Commit | Verified historical change | Current DISTILL disposition | Assessment |
|---|---|---|---|
| `cbcf9a6d205b187735c5a080be9711ed32a2116d` | Introduced the persistent-control-plane E09 v2 shape: one build/preparation/control plane, worker-owned healthy stop followed by a failure deployment, and siblings able to continue. The original worker path did not require every worker’s healthy cleanup before another worker could submit its failure. | Preserve independent worker progress and retain later honest cleanup/error evidence. | Accurate. This is the baseline behavior needed to expose the concurrent deployment outcome. |
| `b653e1ad1758d11be33be457849b284f78141333` | Corrected the sample from 100 pairs to 20 pairs, used two ten-worker cohorts, and established the 600-second owner plus 60-second cleanup budget and related terminal handling. | Retain 20 pairs, concurrency ten, the one-server shape, cleanup, and complete results. | Accurate. The DISTILL treats this as a bounded sample/concurrency correction, not as permission to serialize the run. |
| `f8dad8bccc4f880ce526da64897af53a42c1fae4` | Added a global `healthy-cleanup-complete` join before failure submissions, added a `failure-submit-release` gate, extended observation/disposal from 60 to 180 seconds, and extended the E09 runner budget from 600 to 1200 seconds. Scheduler checks asserted the global gate. | Remove the global healthy-cleanup barrier, restore 60-second observation/disposal and 600-second owner budget, and have each worker submit its failure after its own cleanup. | Accurate. The current `run-example.sh` has no global healthy-cleanup join in `run_trial`/`run_cohort`; it retains the healthy-active and failure-active cohort barriers. |
| `9e41ffdbc87cbd7d4b54e61cf650831504cfab25` | Aligned the E09 overall-budget documentation and comments with the temporary 1200-second amendment; it did not change convergence behavior. | Restore active references to the user-authorized 600-second budget without relabeling the historical capture. | Accurate. The current README, runner, and harness describe 600 seconds; the history remains documented as history. |
| `373b335c9ab623938eaa8e07992327fcc0e52d45` | Added a 120-second streaming cap to the historical `e09_v2_failure_stream_overlap_spike.rs` diagnostic to accommodate its nine serial 12-second stops. | Preserve that cap as a historical/manual-drain diagnostic and keep seed `283001` for the production-server scheduling invariant. | Accurate and properly scoped. It is not presented as a new production lifecycle mechanism. |

The current code corroborates the account. In `examples/service-kind-vm-workloads-v2/run-example.sh:748-809`, the healthy stop and failed-disposal observation windows are 60 seconds. In `:1008-1014`, a worker records its own healthy cleanup and immediately proceeds to its failure deployment; there is no `wait_for_gate failure-submit-release`. In `:1122-1185`, the coordinator still requires the two ten-worker cohort markers, waits for all worker joins, preserves the complete 20-row ledger, and checks resource/store state. In `verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh:7` and the corresponding README/harness, the active owner budget is 600 seconds with 60 seconds of cleanup/grace. This is the intended correction to the historical global barrier and does not remove the prior cohort or cleanup guarantees.

## Outcome coverage and acceptance narrative

The DISTILL’s outcome mapping is sufficient for the requested product result once its provenance references are corrected.

* The concurrent-deployment outcome is exercised by the actual E09 v2 shell path. The host-safe `test_independent_failure_submission` check sources the production example’s `run_trial` and `run_cohort`, replacing only external adapters. It holds worker 10 inside its healthy stop, allows worker 1 to submit its failed deployment, and asserts that worker 10’s healthy cleanup has not become a global prerequisite. This is evidence about the real example owner/coordinator path, not a parallel synthetic implementation.
* The same E09 path retains both ten-worker cohort barriers, worker completion, the 20-row ledger, one control-plane identity, full resource/store checks, the healthy exact guest reply, and failure peer-absence/error evidence. The history correction therefore addresses the concurrency regression while retaining the previous truthfulness and cleanup outcome.
* S-VLL-01 through S-VLL-11 in `feature-delta.md` cover the internal scheduling, lease, shutdown, stop/VMM, guest-supervision, and distribution obligations through the shared production server, the seeded Sim fixture, and in-process tests. S-VLL-12 maps the operator journey to the updated E09 v2 example/expectation. The DISTILL does not make the shell expectation carry private lifecycle assertions.
* The current author validation reports the host-safe scheduler suite and amended E09 runner checks exiting zero. The recorded focused Rust command exits zero because it intentionally contains the named RED/pending cases under construction; it is not represented as GREEN. Native profiles, the fresh native E09 run, and performance distributions remain pending. That separation is accurate and prevents a host-safe scheduler pass from being overstated as native product evidence.

The expectation reuse decision is therefore accepted on scope grounds. The existing E09 ID is the right black-box boundary for this changed outcome, and the user expressly allowed reuse when it makes sense. No new expectation is needed merely to restate the same one-server/20-pair journey. The native pending state does mean implementation and execution remain incomplete, but that is already disclosed in the DISTILL and roadmap and belongs to execution/reviewer gates rather than an invented scope change.

## Finding F-01 — Validation record paths in DISTILL do not exist as cited

**Severity:** Medium; blocking for independent DISTILL provenance and handoff.

**Evidence:** The validation section of `docs/feature/vm-lifecycle-latency/feature-delta.md:502-507` cites the shell records under `.context/vm-lifecycle-distill/shell-eqtv8rcs/`; `:514-522` cites the focused Rust records under `.context/vm-lifecycle-distill/rust-r4vu5xqj/`; and `:533-537` cites the runner baseline under `.context/vm-lifecycle-distill/runner-baseline-jypkq8wq/`. The filesystem contains these records under `.context/vm-lifecycle-latency-distill/` instead:

| Evidence claimed by the author | Current cited prefix | Actual inspected path | Observed result |
|---|---|---|---|
| Host-safe shell scheduler/runner checks | `.context/vm-lifecycle-distill/shell-eqtv8rcs/` | `.context/vm-lifecycle-latency-distill/shell-eqtv8rcs/{baseline.txt,runner.txt,scheduler.txt}` | Baseline old path exits 124; amended runner and scheduler checks exit 0. |
| Focused Rust/Sim diagnostic | `.context/vm-lifecycle-distill/rust-r4vu5xqj/` | `.context/vm-lifecycle-latency-distill/rust-r4vu5xqj/{command.txt,exit.txt,output.txt}` | Recorded command exits 0 and reports the intentional RED/pending/control split described by the author. |
| Unchanged-HEAD runner baseline | `.context/vm-lifecycle-distill/runner-baseline-jypkq8wq/` | `.context/vm-lifecycle-latency-distill/runner-baseline-jypkq8wq/0.txt` | Records the bounded old runner failure and remote descendant surviving the deadline. |

The path prefix is a concrete, reproducible documentation error: a reviewer following the feature-delta citations cannot find the records at the cited locations, even though the corresponding local records exist at the feature-specific directory. The `feature-delta.md:553-555` archive citation has the same prefix error. That archive is explicitly declared incomplete and excluded from expectation/performance evidence, so it must remain excluded; correcting its exclusion pointer is metadata cleanup and does not turn it into evidence.

**Required disposition:** Correct the validation record references to `.context/vm-lifecycle-latency-distill/...` (or another exact path that exists in the author’s validation environment), retain the explicit exclusion of the accidental native archive, and leave the reported exit codes and pending native claims unchanged. No production code, public API, expectation ID, test boundary, or design artifact needs to change for this finding.

**Reachability/reproduction:** This finding is about the author’s recorded documentation, not a hypothetical production state. It reproduces directly by resolving each cited path and observing that it is absent, then resolving the feature-specific path and observing the corresponding files. No scheduler timing, cancellation, or test-only state is required to establish it.

## Non-findings and scope protections

The following were checked and do not require a finding or remediation in this review:

* The history claims about the global healthy-cleanup barrier, the temporary 180/1200-second amendment, and the restored independent worker sequence are supported by the named commits and current source.
* Reusing E09 v2 is consistent with the user’s clarified preference and the existing operator journey. A new expectation would be warranted only for a distinct stakeholder-visible outcome; the DISTILL does not identify one.
* No absent DISCUSS or DEVOPS artifact is a defect here because the user explicitly authorized DESIGN-only input and skipped DEVOPS due to the lack of production users.
* The host-safe scheduler fixture does not claim to be native kernel or VM evidence. The native E09 status is pending, and the accidental archive is not accepted as evidence.
* The review does not reopen ADR-0102/0103, alter ownership terminology, prescribe a persistence/recovery subsystem, or add adjacent hardening. The only requested correction is the exact provenance path.

## Verification performed

The review used the approved feature delta and its DISTILL sections, ADR-0102, ADR-0103, the pending delivery roadmap, the E09 v2 example/README/runner/harness/INDEX, the prior VM service test-scenario anchor, and the five history commits listed above. The commits were inspected with `git show` against their parent/current files; the current E09 source and shell harness were inspected at the cited paths; and the author’s local records were inspected under `.context/vm-lifecycle-latency-distill/`.

The recorded host-safe command results were checked without rerunning a native benchmark or expectation runner. The scheduler record reports exit 0, the amended shell runner record reports exit 0, and the unchanged-HEAD baseline records the expected timeout/failure. The focused Rust record reports exit 0 while retaining the author’s explicitly named RED and pending cases. No mutation testing was run, and no accidental diagnostic archive was used as validation evidence.

The worktree contained pre-existing unrelated dirty work, including `AGENTS.md`; it was preserved. This review writes only `docs/feature/vm-lifecycle-latency/distill/review-scope-history.md`.

## Iteration and verdict

### Iteration 1

Finding F-01 remains open. No remediation was performed by this reviewer because the bounded assignment owns the review artifact only. The history, user scope, expectation reuse decision, acceptance outcome, evidence-layer boundaries, and pending-native disclosure are otherwise accepted.

**Verdict: CHANGES_REQUESTED.** Correct the four validation record prefixes, keep the accidental archive explicitly excluded, and return this same scope/history review for re-review. This verdict is limited to DISTILL provenance; it does not approve the pending roadmap or claim that native E09 execution is complete.

### Iteration 2 — remediation re-review

The author completed the requested documentation-only remediation. The
validation section of `docs/feature/vm-lifecycle-latency/feature-delta.md`
now names `.context/vm-lifecycle-latency-distill/` as its evidence root and
uses that prefix for all eight referenced records: the three shell records,
the three focused Rust records, the runner baseline, and the explicitly
excluded native diagnostic archive. A search of the current feature delta
finds no remaining `.context/vm-lifecycle-distill/` prefix.

Each pointer resolves in the current workspace. The shell records are present
at `shell-eqtv8rcs/{baseline.txt,scheduler.txt,runner.txt}`; the Rust records
are present at `rust-r4vu5xqj/{command.txt,output.txt,exit.txt}`; the runner
baseline is present at `runner-baseline-jypkq8wq/0.txt`; and the archive is
present at `native-diagnostic-20260910T162739.tar.gz`. The contents remain
consistent with the original author report: the unchanged-HEAD shell baseline
exits 124, the amended host-safe runner and scheduler records exit 0, the
focused Rust command records exit 0 while intentionally retaining ten RED
behavioral failures and five pending-body panics alongside one healthy
control, and the unchanged Lima runner baseline exits 1 with the surviving
fixture descendant. The archive remains expressly excluded from acceptance
and latency evidence.

The remediation did not alter the history account, acceptance scope, evidence
boundaries, native-pending status, expectation ID, production code, tests,
runners, or roadmap semantics. The separate roadmap compression and locator
updates remain under the architect/roadmap review; they do not change this
scope/history finding. The pre-existing `AGENTS.md` dirty work and the
author’s implementation changes remain preserved.

#### Finding disposition

| Finding | Iteration 2 evidence | Disposition |
|---|---|---|
| F-01 — validation record paths did not exist as cited | All eight feature-delta pointers now resolve under `.context/vm-lifecycle-latency-distill/`; the obsolete prefix is absent; recorded exit codes and the archive exclusion are unchanged. | **Closed.** Documentation-only remediation satisfies the required provenance correction. |

No new scope, history, acceptance, or provenance finding was produced. Native
acceptance remains pending exactly as disclosed; the local host-safe and
focused Rust records do not establish native product completion. No native
benchmark or expectation runner was run for this re-review.

**Verdict: APPROVED.** The DISTILL scope/history review is approved after F-01
closure. This approval covers provenance, prior-feature behavior, user-scoped
E09 reuse, acceptance narrative, and evidence boundaries. It does not approve
`deliver/roadmap.json`, waive its pending validation status, or claim that the
remaining RED/scaffold tests or native E09 acceptance are complete.
