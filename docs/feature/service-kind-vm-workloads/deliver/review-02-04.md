# DELIVER review — step 02-04 E09-v2, E10, and E13 closure

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Roadmap step | `02-04` — E09-v2 bounded acceptance and native verification |
| Reviewer | Codex, `nw-software-crafter-reviewer` |
| Model | GPT-5.6 Luna, maximum thinking |
| Review date | 2026-09-09 |
| Reviewed closure | commits `8f5d0de`, `f5c9a78`, and remediation `251ed14a` |
| Native capture identity | SHA `9f6702e431c1a68a3374445368ea8fc525f23bc4`, seed `1`, `native-metal`, dirty worktree |
| Review iterations | Iteration 1 and remediation re-review, with independent evidence audit still pending |

## Scope and authority

This review covers the accepted `02-04` criteria in
`docs/feature/service-kind-vm-workloads/deliver/roadmap.json`, the user-approved
E09-v2 `1200`-second setup/trials budget plus `60`-second cleanup grace, the
fresh E09-v2/E10/E13 runtime artifacts, and the closure commits. The reviewed
step owns the checked-in black-box expectations and their bounded example and
harness surfaces. It does not own production Rust, a public API, step `03`, or
the wave mutation gate.

The previously approved cohort barrier, failure predicate, disposal observation
history, and timeout implementation reviews are treated as existing evidence.
They are not re-opened here. The prior three capture objections are recorded
and rejected in `docs/analysis/review-02-04-capture-findings.md`; this review
does not reintroduce them.

No native command, Cargo test, source edit, DES event, staging operation, or
commit was performed by this review. The only file written by this reviewer is
this Markdown artifact. Existing dirty work, including `AGENTS.md`, was
preserved.

## Accepted criteria audit

| Criterion | Evidence reviewed | Assessment |
|---|---|---|
| E09-v2 builds/prepares once, uses one persistent control plane, runs two ten-pair cohorts at concurrency 10, and records 20 pairs | `product-run.meta:1-10`, `product-run.out` ledger and cohort sections, `verification.yaml:1-16`; read-only ledger parse | Mechanically satisfied by the fresh capture: 20 rows, two cohorts, one control-plane PID/start identity, and the native runner's `E09 v2 PASS` line. Human evidence satisfaction remains the independent audit gate. |
| Every pair is honest and no pair is retried, replaced, discarded, or substituted by a favorable writer | E09 ledger predicates, runner checks at `runner.sh:46-70`, v2 example's ordered worker/cohort path, native PASS line | Mechanically satisfied: all 20 rows are `pass`/`complete`, healthy deploy result `0`, failure deploy result `1`, typed `StartupProbeFailed`, and `zero-runtime` cleanup; the runner also checks one control-plane identity and no retries. The native artifact is not independently stamped `satisfied` here. |
| E10 covers eight cross-driver HTTP/status cells with no failure-body sentinel leakage; E13 covers inferred TCP success/failure | E10 ledger rows `product-run.out:21-29`, E10 PASS line at `:31`; E13 ledger rows `:21-23`, E13 PASS line at `:25`; current runner predicates | Mechanically satisfied: E10 has all eight Exec/VM × 204/302/404/503 rows, zero sentinel counts, and `zero-delta`; E13 has two rows with exact-reply and unreachable-verified peer outcomes and `zero-delta`. |
| E09-v2, E10, and E13 remain black-box expectation runs with pinned captures and independent audits; Rust tests stay in-process | Each `verification.yaml`, `product-run.meta`, expectation READMEs, and runner boundaries | Boundary is preserved. All three manifests pin SHA, seed, substrate, invocation, and runner exit `0`; README status remains `pending` until the independent evidence audit. No expectation runner invokes Cargo tests or an `overdrive-*` crate. |
| E09-v2 remote lifetime is 1200 seconds for setup/trials plus 60 seconds cleanup grace; only the bounded runner/scheduler correction is in scope | E09 `runner.sh:7-11`, README `:48-69`, product metadata, current roadmap criterion/notes | The runner, README, evidence, and two active step-02-04 fields use `1200+60` and derived `1320`. Two active roadmap metadata references and one stale runner comment still say `600`; this is F-01 below. |

The E09 ledger parse produced the following exact values from the saved native
output: 20 rows; outcomes only `pass`; stages only `complete`; one control-plane
PID/start-tick identity; healthy deploy results only `0`; failure deploy results
only `1`; failure reason only `StartupProbeFailed`; both cleanup columns only
`zero-runtime`; and both peer columns `Succeeded`.

The E10 parse produced eight rows with `deploy_exit` `0`, trajectories
`operator-stop-running`, `replacement-operator-stop`, and
`replacement-start-rejected`, all sentinel columns `0`, and `zero-delta`
cleanup. The checked-in E10 helper uses `deploy_exit=0` as its bounded
case-helper result after retaining the expected startup-failure transcript for
302/404/503; it does not mean those Service deployments succeeded. The
remediated result document now says this explicitly at
`docs/feature/service-kind-vm-workloads/deliver/02-04-result.md:127-135`.

The E13 parse produced exactly the healthy and failure rows claimed by the
result document: `Stable`/`exact-reply` and
`StartupProbeFailed`/`unreachable-verified`, both with inferred targeting and
`zero-delta` cleanup.

## Pinned runtime and boundary evidence

The E09 manifest records `execution_status: "succeeded"`, native-metal, seed
1, SHA `9f6702e431c1a68a3374445368ea8fc525f23bc4`, and runner exit `0`.
`product-run.meta` records the remote `1200s` owner, `60s` cleanup grace,
`1320s` transport bound, and the run from `2026-09-08T23:34:23Z` through
`23:46:05Z`. E10 and E13 manifests pin the same product SHA and each record
native-metal, runner exit `0`, and `execution_status: "succeeded"`.

These are mechanical facts about retained artifacts. The expectation README
statuses and `verification/expectations/INDEX.md` remain `pending`, as required
until the separate evidence auditor verifies exact guest, negative-peer,
cleanup, and capture claims. This review does not turn a runner exit into a
human satisfied verdict.

The closure commits contain no `crates/` or `api/` paths. The closure source
range is documentation, expectation evidence, and DES bookkeeping only; no
public API or product behavior was added in this closure. The user-approved
barrier, predicate, and timeout behavior therefore remain the already reviewed
bounded implementation.

## DES execution-log audit

The actual `02-04` entries in
`docs/feature/service-kind-vm-workloads/deliver/execution-log.json` are:

| Phase | State/result | Interpretation |
|---|---|---|
| RED | `EXECUTED` / `FAIL` at `2026-09-08T14:04:43Z` | Historical first native/implementation attempt; retained failure. |
| GREEN | `EXECUTED` / `FAIL` at `14:40:14Z` | Historical correction attempt; retained failure. |
| RED | `EXECUTED` / `FAIL` at `15:37:17Z` | Historical re-entry after the retained failure; not silently converted to a pass. |
| RED | `SKIPPED` / `NOT_APPLICABLE` at `23:34:11Z` | Verification-only closure after implementation and approved corrections were already committed; no new RED activation was claimed. |
| GREEN | `SKIPPED` / `NOT_APPLICABLE` at `23:34:15Z` | Verification-only closure with no remaining implementation delta; no inherited GREEN coding work was claimed. |
| COMMIT | `EXECUTED` / `PASS` at `2026-09-09T00:02:51Z` | The closure documentation/evidence commit phase was recorded after the closure work. |

The skipped RED/GREEN entries are honest `NOT_APPLICABLE` dispositions rather
than inherited execution claims. The later COMMIT event corresponds to the
closure documentation/evidence commit and does not claim a new implementation
GREEN result.

## Findings — iteration 1

### F-01 — Active roadmap and runner comment retained the old 600-second budget

**Severity:** Medium, blocking contract/documentation consistency.

Before remediation, the current step criterion and implementation note were
already changed to `1200+60`, but the same active roadmap still stated `600`
in the top E09-v2 note at `roadmap.json:27` and in
`validation.current_amendment_approval_scope` at `roadmap.json:393`. The v2
example's live comment at `examples/service-kind-vm-workloads-v2/run-example.sh:758`
also said that the suite's overall `600s` run budget was unchanged, although
the runner's actual owner is `1200s` and the per-stop observation bound is
separate. A read-only `rg` reproduced all three stale active references after
the first remediation commit.

This is reachable by a maintainer or reviewer using the active roadmap or
reading the comment while diagnosing a timeout. It can cause the acceptance
owner to apply the wrong overall deadline. The `600s` build bound in the same
example is a separate per-build bound and must remain unchanged; only the
overall-budget wording is stale.

**Required smallest remediation:** update the two active `roadmap.json`
metadata strings and the overall-budget comment to `1200` while preserving the
`60` cleanup grace, `1320` transport derivation, all per-command bounds, and
all pair/concurrency/predicate behavior. This is documentation-only fallout;
it does not authorize a new timeout or production change.

**Disposition:** Accepted for bounded documentation remediation. No product,
API, or architecture change is required.

### F-02 — Closure result wording conflated E10 helper completion with Service success and revived the old leak interpretation

**Severity:** Medium, blocking honest closure documentation until corrected.

The pre-remediation result said all E10 rows had deploy exit `0` without
explaining the checked-in helper's expected-failure convention. A reader could
interpret that as 302/404/503 Service deployment success. Its historical
paragraph also called the replaced capture a c008-c010 allocation-resource
observation failure, while the premise validation records that the cited
inventories were captured before stop and do not establish post-terminal
residue (`docs/analysis/root-cause-analysis-e09-v2-vm-stop-cleanup-premise.md:25-35`,
`:157-187`).

The old result's wording therefore needed a precise historical boundary. The
later disposal timeout is a separate bounded runner observation issue; it does
not turn the pre-stop inventories into proof of a product cleanup-ordering
defect.

**Required smallest remediation:** explain the E10 helper `deploy_exit=0`
meaning for expected failure cells, and state that the old artifact-leak
interpretation came from pre-stop inventories and was disproven/left
unestablished by premise validation. Preserve the later disposal timeout and
its 60-to-180-second observation correction as separate history.

**Disposition:** Accepted for bounded result-document remediation. No native
rerun or behavior change is required.

## Remediation re-review — iteration 2

The original crafter committed `251ed14a` (`docs(verification): correct 02-04
budget and history`) with no source/API paths.

F-02 is **closed**. The E10 section now says that `deploy_exit=0` records the
checked-in helper completing the expected startup-failure case for 302/404/503,
not that those Service deployments succeeded
(`docs/feature/service-kind-vm-workloads/deliver/02-04-result.md:127-135`).
The history section now says the old artifact interpretation came from pre-stop
inventories, cites the premise validation, explicitly says it does not establish
post-terminal residue or a cleanup-ordering defect, and keeps the later disposal
timeout separate (`02-04-result.md:167-177`). The checked-in E10 evidence
supports the revised wording: all eight rows have helper `deploy_exit=0`,
expected trajectories, zero sentinel counts, and `zero-delta`; the E10 runner
accepts the expected-failure trajectories at
`examples/service-kind-vm-workloads/run-example.sh:650-670`.

F-01 is **still open**. The same remediation changed the active step criterion
and implementation note to `1200+60` (`roadmap.json:276,304`), but left the
active top note at `roadmap.json:27`, the current approval scope at
`roadmap.json:393`, and the live overall-budget comment at
`run-example.sh:758` at `600`. The residual references are directly reproduced
by the focused static check and must be corrected before approval.

No new reachable product or expectation defect was found in this re-review.
The previous barrier/predicate/timeout behavior remains within its approved
scope and was not re-audited beyond the exact closure/documentation delta.

## Focused verification performed

| Check | Result |
|---|---|
| `jq -e . docs/feature/service-kind-vm-workloads/deliver/roadmap.json` | PASS |
| `bash -n` on the v2 example, scheduler test, E09 runner, and runner harness | PASS |
| Read-only E09 ledger parse | PASS — 20 `pass`/`complete` rows, one control-plane identity, expected healthy/failure predicates, zero-runtime cleanup |
| Read-only E10 ledger parse | PASS — 8 cells, helper deploy exit `0`, expected trajectories, all sentinel columns `0`, zero-delta cleanup |
| Read-only E13 ledger parse | PASS — exact-reply healthy row and unreachable-verified failure row, both zero-delta |
| DES JSON parse and phase audit | PASS — historical failures retained; verification-only RED/GREEN explicitly `NOT_APPLICABLE`; COMMIT recorded separately |
| Closure path audit for commits `8f5d0de`, `f5c9a78`, `251ed14a` | PASS — no `crates/` or `api/` paths |
| Focused active-budget scan | **FAIL** — `roadmap.json:27`, `roadmap.json:393`, and `run-example.sh:758` still say overall `600` |

No native rerun, Cargo test, mutation test, DES mutation, source edit, or
commit was performed. The existing native captures were read only.

## Evidence-audit gate and limits

The fresh manifests and saved outputs mechanically report successful runners,
but independent audit remains a separate required gate. E09-v2, E10, and E13
README statuses and the expectations index remain `pending`; this reviewer does
not write `satisfied` or infer all internal transitions from aggregate output.
The independent evidence auditor owns final capture sufficiency and any README
or index status transition.

This review does not claim the full feature, step `03`, native capacity or
throughput, mutation readiness, or any unrelated historical evidence. Once
F-01 is corrected and the independent evidence audit returns an approving
verdict, the same reviewer can complete the final step verdict without a broad
re-audit.

## Verdict

**CHANGES REQUESTED.** F-02 is closed by `251ed14a`. F-01 remains a reachable,
bounded documentation inconsistency: active roadmap metadata and the v2 example
comment still describe the overall E09 owner as `600` seconds while the
authorized and executed contract is `1200+60` with `1320` transport framing.
Correct those three references without changing behavior, then complete the
separate independent evidence audit before returning `APPROVED` for step
`02-04`.

## Final re-review — iteration 3

The original crafter committed `9e41ffdb` (`docs(verification): align E09
overall budget references`). This remediation changes the active E09-v2
roadmap note to `1200s`, changes the current amendment scope to `1200s + 60s`
while preserving the prior `600s` approval under the explicitly historical
`prior_e09_v2_functional_acceptance_approval_scope`, and corrects the v2
example's overall-budget comment at `run-example.sh:758`. The unchanged
`bounded 600s cargo build` at `run-example.sh:1386` is a per-build bound, not
the remote owner budget, and correctly remains `600s`.

F-01 is **closed**. A focused scan finds no stale active overall-budget
reference in the reviewed roadmap or v2 example; the remaining `600s` strings
are the preserved prior approval scope and the separate build bound. The
active contract now consistently states `1200s` setup/trials, `60s` cleanup
grace, and `1320s` transport framing.

The independent different-fox evidence audit is now complete and approving in
`docs/analysis/review-02-04-final-evidence.md`. It independently marks E09-v2,
E10, and E13 **SATISFIED**, verifies the pinned native manifests and exact
fixture oracles, rejects the former EA-01/EA-02/EA-03 objections within scope,
and records no remediation. The three expectation READMEs and their INDEX rows
are now `satisfied`.

### Post-verdict completion-record synchronization

The following active roadmap/result prose still describes the pre-review state
and should be mechanically synchronized by the root orchestrator after this
review's verdict. They are completion-record updates, not implementation
findings and do not reopen historical evidence:

- `docs/feature/service-kind-vm-workloads/deliver/roadmap.json:271` says
  `native capture remains pending`.
- `roadmap.json:276` and `:304` say `native verification remains pending`.
- `roadmap.json:395` says native verification remains pending and step `02-04`
  is not complete.
- `docs/feature/service-kind-vm-workloads/deliver/02-04-result.md:7-8` says
  the independent audit remains pending, and `:181` leaves completion to a
  future reviewer.
- The active feature-coverage paragraph in
  `verification/expectations/INDEX.md:205-207` still says `600s` and native
  verification remains pending, although its E09-v2/E10/E13 rows are satisfied.

The initial roadmap baseline, the `prior_e09_v2_functional_acceptance_approval_scope`,
the earlier budget amendment, and prior implementation-review prose retain
their historical pending/600-second wording intentionally. They are not
rewritten as if the earlier approval or failed attempt had used the later
user-directed timeout.

Step `03` remains untouched by the closure commits and is outside this verdict.

### Final focused verification

| Check | Result |
|---|---|
| `jq -e . docs/feature/service-kind-vm-workloads/deliver/roadmap.json` | PASS |
| Active E09 budget scan | PASS — active roadmap/example references use `1200+60`; historical prior approval and per-build `600s` are distinguished |
| E09-v2, E10, and E13 README status scan | PASS — all three are `satisfied` and link the independent final audit |
| Expectations INDEX row scan | PASS — E09-v2, E10, and E13 are `satisfied` |
| Independent evidence audit disposition | PASS — E09-v2, E10, and E13 each `SATISFIED`; no remediation |
| Closure scope audit for `9e41ffdb` | PASS — only roadmap metadata and one source comment changed; no production/API behavior |
| Step-03 scope check | PASS — no step-03 file changed by the closure remediation |

No native rerun, Cargo test, mutation test, DES event, source edit, or commit
was performed by this review. The native captures and independent audit were
read-only inputs.

## Final verdict

**APPROVED.** Step `02-04` satisfies its bounded contract: E09-v2 has 20
truthful pairs across two ten-pair cohorts at concurrency 10 through one
persistent control plane; E10 has all eight cross-driver status cells with no
failure-body sentinel leakage; E13 has the inferred TCP success/failure pair;
all three are black-box native expectations with pinned captures and an
independent SATISFIED audit; and the authorized `1200+60` E09 lifetime is
consistent across active implementation and roadmap references. The closure
contains no production/API behavior change, and no step-03 work is approved.

The root orchestrator may now synchronize the listed stale completion-record
prose and advance the workflow according to the repository's DELIVER ordering;
that bookkeeping does not change this approval or rewrite historical records.
