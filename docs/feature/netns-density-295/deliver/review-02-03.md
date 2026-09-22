# Adversarial review — step 02-03

- **Feature:** `netns-density-295`
- **Step:** `02-03` — node-shared leg-F/leg-C listener owner and private capability registry
- **Reviewer:** isolated DELIVER `nw-software-crafter-reviewer`
- **Review ID:** `code_rev_20260921_224335_iteration_1`
- **Iteration:** 1
- **Reviewed commit:** `2acfc29d471e0d811af3be62db1face521adc789`
- **Parent:** `f1bd1fa1720768d09e4a12783e88ebbe39d22d23`
- **Final verdict:** **CHANGES_REQUIRED**

## Executive summary

The commit implements substantial portions of the accepted private D7 registry
and the seven-method worker surface, and it preserves the intended separation
from the 03-03 retry/deadline/fail-stop supervisor. It is not approvable yet.

The private task owner diverges from the exact D11 ownership contract: it holds
a strong sender instead of the required weak sender, does not use the required
`AbortOnDropListenerTask`, and exposes a borrowed shutdown operation that can
detach the listener task when the observer is dropped. The shared capability
stop path also completes and removes a retiring record even when enforcement
teardown fails, losing the handle and the existing worker retry complement.

The claimed late-handle race test does not enter the production shared-listener
dispatch path; it calls the injected enforcement port and registry claim
directly. There is no active S-ND295-25/S-ND295-26 worker test with two active
shared capabilities, unrelated live listeners/handles, or active-claim owner
shutdown, and no action-shim test drives the real `RegistrationRetired` result.
The active S20 test does not observe exact listener/task/guard cardinality, and
the live acceptance bodies have no outcome-anchor declaration or complete
bounded-change complement assertions.

## Iteration history

| Iteration | Commit | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `2acfc29d471e0d811af3be62db1face521adc789` | **CHANGES_REQUIRED** | 7 blockers | Return to the original step-02-03 crafter; re-review this step only after remediation. |

## Authority and scope

The review used the approved `02-03` roadmap entry in
`docs/feature/netns-density-295/deliver/roadmap.json`, the accepted
C-295-L/F-03, D-295-DISTILL-7, D-295-DISTILL-11, D-295-DISTILL-15, GEN-295-A,
and RUN-295-B sections of `feature-delta.md`, the DISTILL S-ND295-20..26
scenario table, the architecture brief, and the production callers in
`MtlsInterceptWorker`, the action shim, and the retained control-plane
supervisor scaffold. The review explicitly does not credit the pending 03-03
supervisor cadence/deadline/fail-stop body.

No production, test, or configuration file was changed by this reviewer. This
Markdown file is the sole tracked write for the review.

## Strengths

- `MtlsInterceptWorker` retains the approved public seven-method C-295-L
  surface at `crates/overdrive-worker/src/mtls_intercept_worker.rs:1984-2795`.
  No new public generation setter, task-control hook, guard operation, clock,
  retry API, or control-plane dependency was added.
- The registry implements checked generation allocation before reservation,
  Pending/Active/Retiring records, exact source/destination indexes, RAII
  claims, publication fencing, pending-owner handoff, scoped drain, and
  remove-before-reassign in `mtls_intercept_worker.rs:634-1071`.
- `start_alloc` projects activation loss to
  `MtlsInterceptInstallError::RegistrationRetired` at
  `mtls_intercept_worker.rs:2555-2561`, and the worker does not add the 03-03
  clock/retry/deadline/fail-stop machinery.
- Startup partial-bind and convergence failures keep the owner unpublished and
  close the acquired local sockets; the observe-only wrong-target body checks
  that no runtime bind or fresh `converge_shared` occurs.
- The commit preserves Marcus as author and contains exactly one
  `Co-Authored-By: Codex <codex@openai.com>` trailer and one `Step-Id: 02-03`
  trailer. The canonical DES replacement cycle is present and ordered.

## Contract Shape Compliance — iteration 1

**Overall: FAIL.**

| Check | Status | Evidence |
|---|---|---|
| Declaration present | PASS mechanically | The transitioned acceptance bodies in `tests/acceptance/netns_density_shared_owner.rs:41,59,244,259,276,304,338,578` and the live source-local bodies in `mtls_intercept_worker.rs:484,523,1258,1320,1363,1401,1436,1475,1509` carry `/// CONTRACT_SHAPE: bounded-change.`. |
| Banned technical names | PASS | The live names do not match the banned `returns_*`, `exit_code`, `calls_*_once`, `status_code`, or `http_*` result-name patterns. |
| Outcome anchor | **FAIL mechanically** | None of the transitioned acceptance functions in `netns_density_shared_owner.rs` has the required `/// Outcome anchor: DISCUSS Elevator Pitch` declaration. Existing acceptance bodies in this repository use that declaration. |
| Bounded-change declared delta | **FAIL** | The registry tests assert selected lookup results but do not snapshot the declared registry universe and its complement. For example, `generation_boundaries_and_every_lifecycle_conflict_precede_effects` at `mtls_intercept_worker.rs:1258-1317` never asserts `next_generation`, all records, allocation reservations, both indexes, or retained effects after a conflict. |
| Complement equality | **FAIL** | `publication_before_retirement_is_owned_by_only_that_generation` at `:1436-1473` observes one source and one second claim but does not prove listener/task/node-guard state or all unrelated registry/effect state byte-equal. |
| Driving boundary for the retirement race | **FAIL** | The purported production race at `:4362-4427` calls `enforcement.enforce` and `claim.publish` directly. It never drives `handle_shared_outbound`, `handle_shared_inbound`, or `spawn_shared_enforcement`. |

## Mechanical evidence

| Gate | Result |
|---|---|
| DES RED/GREEN/COMMIT | PASS — canonical events at `2026-09-21T22:16:49Z`, `22:16:55Z`, and `22:17:01Z`, all `EXECUTED/PASS`. Earlier 02-03 history remains in the committed log and is not silently rewritten. |
| Commit author/trailers/stat | PASS — Marcus author; exactly one Codex trailer and one `Step-Id`; 3 files, 1212 additions, 152 deletions. |
| `git diff --check` | PASS. |
| `cargo fmt --all -- --check` | PASS. |
| Crafter workspace check | Reported PASS in the DES GREEN evidence. |
| Crafter focused clippy | Reported PASS for `overdrive-worker --lib --features integration-tests -D warnings`. |
| Crafter all-target clippy | Baseline-limited by the pre-existing `overdrive-netlink` test warning and unrelated control-plane findings; no changed-code warning was reported. |
| Crafter native metal evidence | Reported 27 KVM/VMM tests passed and 54 dataplane zero-copy tests passed with 6 skipped. These suites do not call `start_shared_owner` or prove S-ND295-25/S-ND295-26 shared capability ownership. |
| Independent host focused test | Environment-limited — macOS cannot compile the Linux `aya`, `netlink-sys`, and `linux-keyutils` dependencies. |
| Independent Lima focused test | Environment-limited — the configured Lima target is read-only; an alternate `/tmp` target ran out of space before compiling (`/tmp` was 100% full). No independent green result is claimed. |
| Mutation testing | NOT RUN, correctly deferred to the final DELIVER-wave gate. |

The worktree already contained unrelated dirty `.serena/project.yml`,
`AGENTS.md`, execution-log, and prior-review changes. They were preserved.

## Blocking findings — iteration 1

### D1 — D11 task-owner contract is not implemented and can detach listener tasks

- **Severity:** Blocker
- **Dimension:** Design/API shape, task ownership, external validity
- **Locations:** `crates/overdrive-worker/src/mtls_intercept_worker.rs:315-429`
- **Authority:** `feature-delta.md:1502-1562`

The accepted D11 private contract requires `SharedListenerTaskOwner.event_tx`
to be a `tokio::sync::mpsc::WeakSender`, `shutdown(self)` to consume the owner,
and each observer to own an `AbortOnDropListenerTask`. The implementation uses a
strong `mpsc::Sender` at `:341-345`, implements `shutdown(&self)` at `:406`, and
`observe` moves the raw listener `JoinHandle` directly into the observer task at
`:418-429`. `AbortOnDropListenerTask` is defined at `:315` but has no
instantiation or caller in the production path.

This is not a naming preference. The strong sender prevents the owner from
observing an actually closed event channel while it is alive; the
`observer_closed` substitute at `:432-438` treats any finished observer as
`TaskObserverClosed`, including an observer that has successfully delivered a
real join event. Dropping an observer `JoinHandle` detaches the listener task
because the required abort-on-drop wrapper is not present. The production
construction path is `start_shared_owner_inner:2118-2131`; the retained
control-plane path calls `wait_shared_owner_failure` and `converge_shared_owner`,
so this is the actual owner boundary, not dead test scaffolding.

**Required disposition:** implement the exact D11 private owner shape. Keep only
the weak sender in the owner, give each observer a strong sender and an
`AbortOnDropListenerTask`, remove only the terminal slot associated with the
consumed event, and consume the owner in `shutdown`. Add source-local evidence
for real channel closure and abort-on-observer-drop without adding a public
kill hook.

### D2 — Shared capability stop completes after teardown failure and loses retry ownership

- **Severity:** Blocker
- **Dimension:** D7 retirement/drain contract, total cleanup, failure handling
- **Locations:** `mtls_intercept_worker.rs:2686-2703`, compared with the
  existing retry implementation at `:2642-2660` and `:3385-3405`
- **Authority:** `feature-delta.md:3438-3460`

The shared `capability_owned` stop branch takes the exact handles, awaits
`enforcement.teardown`, records any error, drops the elements, and calls
`drain.complete()` unconditionally. A failed handle is not retained in
`AllocStop.retry_handles`. The existing worker stop owner deliberately clones
failed handles into `retry_handles` and only removes the exact owner after the
retryable teardown contract has converged.

The production path is concrete: `start_alloc` dispatches networked specs to
`start_shared_allocation` at `:2333`; successful activation records
`capability_owned: true` at `:2563-2574`; `stop_alloc` reaches this branch at
`:2686`. `MtlsInterceptWorker::new` accepts the existing `MtlsEnforcement`
driven port, and the in-tree `GatedTeardown` fixture already demonstrates a
real `teardown` error through the worker owner path. In the shared branch that
error consumes the only handle, removes the Retiring record and reservation,
and leaves no path to complete cleanup on a later `stop_alloc`. The action-shim
fail-closed caller then proceeds to its structural network cleanup after the
mTLS error, so release-last and exact cleanup-error precedence are no longer
truthful for a failed shared teardown.

**Required disposition:** retain failed `EnforcedConnection` values and the
`CapabilityDrain`/reservation until the accepted retry or typed terminal
failure boundary is complete. Make shared stop use the same authoritative
retry ownership as the existing worker path, and add a real shared-path
regression that fails before the fix and proves retry, reservation retention,
and address release last.

### D3 — The retirement-race test bypasses the production shared dispatch

- **Severity:** Blocker
- **Dimension:** External validity and testing theater
- **Locations:** `mtls_intercept_worker.rs:4362-4427`; production dispatch at
  `:3141-3244`
- **Authority:** `feature-delta.md:3470-3479`, S-ND295-23

`enforcement_returning_after_retirement_tears_down_the_real_returned_handle_before_drain`
claims a real production race, but it obtains a registry claim directly at
`:4388`, calls `enforcement.enforce` directly at `:4395-4404`, and calls
`claim.publish` directly at `:4405`. Neither `handle_shared_outbound` nor
`handle_shared_inbound` nor `spawn_shared_enforcement` is invoked. The only
production callers of `spawn_shared_enforcement` are the two shared listener
dispatch sites at `:3174` and `:3214`, and no test in this commit reaches either
site.

The test therefore remains green if the production task forgets the claim,
publishes to the wrong owner, or omits late-handle teardown; it proves the
registry primitive only. This is precisely the fixture/implementation-boundary
failure the accepted design rejects.

**Required disposition:** drive the accepted real shared-listener/worker path
with the existing enforcement barrier, hold the claim across the awaited
production enforcement call, begin exact allocation retirement, and observe
the returned handle torn down outside the registry lock with no publication.
Do not add a public dispatch hook or replace the production call with a test
copy.

### D4 — S-ND295-25 and S-ND295-26 have no shared-worker evidence

- **Severity:** Blocker
- **Dimension:** Acceptance completeness and external validity
- **Locations:** `tests/acceptance/netns_density_shared_owner.rs:43-628`,
  `mtls_intercept_worker.rs:1436-1473` and `:3691-3752`
- **Authority:** `distill/test-scenarios.md:650-696`, roadmap criteria
  `02-03` items 8, 10, and 12

The active tests do not start two networked allocations through the shared
owner. `publication_before_retirement_is_owned_by_only_that_generation` is a
registry-only test: it creates two records and claims, but no listeners,
worker allocation records, enforcement handles, or node guard. The active
owner-shutdown acceptance body starts only one Pending registration and has no
active claim or published handle. The library owner-shutdown body at
`:3691-3752` uses `record_intercept_full` with `capability_owned: false`, which
is the legacy per-allocation listener owner rather than C-295-L's shared owner.

There is consequently no evidence that stopping one shared allocation leaves
the other handle, both shared listener sockets/tasks, and node guard live
(S-ND295-25), nor that shared-owner shutdown retires multiple active
capabilities, waits claims, drains published handles, closes both shared
listeners, and relinquishes the node guard (S-ND295-26). The reported metal
KVM/zero-copy suites do not call `start_shared_owner` (repository-wide caller
search finds only this acceptance file).

**Required disposition:** add the accepted shared-worker integration/Tier-3
bodies for isolated two-allocation stop and active multi-capability owner
shutdown. Assert unrelated handles/listeners and the node guard complement,
late claims, element removal, socket closure, and sealed guard relinquishment.

### D5 — `RegistrationRetired` is not driven through the real action-shim caller

- **Severity:** Blocker
- **Dimension:** Production caller coverage and cleanup ordering
- **Locations:** `crates/overdrive-control-plane/src/action_shim/mod.rs:637-726`,
  `tests/integration/mtls_install_fail_closed.rs:3977-4102`
- **Authority:** `feature-delta.md:3452-3475`

The accepted D7 contract requires the real `start_alloc` activation-loss result
to travel through the action shim and prove no EXEC release, exact
`registration_retired` stage, driver/mTLS/network cleanup ordering, address
release last, and unchanged cleanup-error precedence. The existing action-shim
test only constructs `MtlsInterceptInstallError::RegistrationRetired` in the
synthetic `install_error_cases` table at `:3977-3999` and passes it directly to
the private `fail_closed_on_mtls_install` helper at `:4089-4102`. Its worker
fixture uses `network: None` (`:274-277`) and does not start the shared owner or
enter `start_shared_allocation`.

This verifies string/stage mapping for a fabricated error, not the production
caller path. No changed or existing test reaches the real Pending → Retired
activation result through `MtlsInterceptWorker::start_alloc` and the action
shim, so a caller that turns the result into success or releases EXEC before
cleanup would remain green.

**Required disposition:** add the accepted action-shim integration slice using
the real worker and a networked allocation, race Pending activation against
stop/owner shutdown, and assert the exact failed row/stage, no EXEC release,
driver stop, mTLS drain, guest-network teardown, address-last ordering, and
cleanup-error precedence.

### D6 — S20 owner publication test does not prove exact cardinality or guard complement

- **Severity:** Blocker
- **Dimension:** Testing theater and acceptance contract coverage
- **Locations:** `tests/acceptance/netns_density_shared_owner.rs:41-57`,
  `RecordingSharedIntercept::call_counts` at `:131-136`
- **Authority:** C-295-L and S-ND295-20

`shared_owner_starts_once_audits_and_shutdown_drains_the_owner_tree` uses
`SimMtlsIntercept`, whose shared port exposes no bind/converge/task/guard
cardinality observation. It calls `start_shared_owner` twice and asserts only
that the second call returns `Ok(())` and that a later audit succeeds. The
test never asserts two binds, one shared convergence, two task slots, one
published guard, or that shutdown leaves the accepted retained rule state.
`RecordingSharedIntercept` already has exact call counters, but this test does
not use it.

A mutant that reruns startup on every idempotent call, publishes a replacement
owner over the first owner, or drops the published guard could pass this body.
The result is not a bounded-change proof of the required S20 universe.

**Required disposition:** drive the accepted recording owner and assert exact
F/C bind, converge, observe, task, socket, lifecycle, guard-retention, and
shutdown-relinquish complements. Keep the test at the worker owner boundary;
do not add a public counter or hook.

### D7 — Live tests lack complete Contract Shape evidence

- **Severity:** Blocker
- **Dimension:** Contract Shape compliance and test integrity
- **Locations:** all transitioned acceptance functions in
  `tests/acceptance/netns_density_shared_owner.rs:41-628`; representative
  registry body `mtls_intercept_worker.rs:1258-1317`

The required declaration line is present, but the transitioned acceptance file
has no `Outcome anchor: DISCUSS Elevator Pitch` declaration, and the bounded-
change tests do not assert the full declared delta plus complement. The
generation/conflict body checks only the returned error and a few later claims;
it does not prove unchanged generation, reservations, both indexes, retained
effects, unrelated records, or listener/owner state. Similar selective
assertions appear in the two-capability publication body. These tests can pass
while an implementation mutates an unasserted part of the declared universe.

**Required disposition:** add the mechanical outcome anchor to each transitioned
acceptance body and make each bounded-change state table snapshot the exact
declared registry/owner universe before and after, asserting the allowed delta
and equality of every complement. Re-run the repository Contract Shape check
before the next review.

## External validity

**FAIL.** The worker's public shared-owner methods are callable from the
acceptance binary, and the runtime wrong-target body correctly remains a
non-closing 02-03 prerequisite. However, the only live shared-owner caller is
the acceptance file; the claimed production retirement race bypasses the
worker dispatch, S-ND295-25/S-ND295-26 do not have shared-worker evidence, and
`RegistrationRetired` is never driven from real `start_alloc` through the
action shim. The green source-local registry and legacy-owner tests therefore
do not establish the accepted stakeholder-visible lifecycle outcome.

## Verification — iteration 1

| Verification | Result |
|---|---|
| Canonical DES RED/GREEN/COMMIT | PASS mechanically and chronological. |
| Author, trailer, and scope | PASS; changed production/acceptance/log scope matches the step, with no unrelated tracked files in the commit. |
| Formatting and whitespace | PASS (`cargo fmt --all -- --check`, `git diff --check`). |
| Source-local registry/task-owner tests | Reported 7 registry + 2 task-owner bodies passed by the crafter. Independent rerun was blocked by the Linux target environment. |
| Shared-owner acceptance selector | Reported 6 selected bodies passed and 2 exact-port recovery bodies remained ignored for 03-03. The pass does not cover active multi-capability S25/S26. |
| Real enforcement retirement race | Reported 1 passed, but the body is direct registry/enforcement invocation rather than production shared dispatch (D3). |
| Workspace check / focused clippy | Reported PASS by the crafter; no changed-code warning identified. |
| Metal KVM/VMM and zero-copy suites | Reported PASS by the crafter; they do not provide shared capability/listener S25/S26 evidence (D4). |
| Lima owner selector | Not independently run — configured target is read-only, and an alternate target ran out of space. |
| Mutation testing | Not run, correctly deferred. |

## Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance body | **FAIL substantively** | S20 is weakly asserted; S23 uses a non-production path; S25/S26 and action-shim activation are absent. |
| G2 — valid RED | PASS mechanically | Canonical RED event is present and PASS. |
| G3 — assertions protect production behavior | **FAIL** | The retirement test can pass without `spawn_shared_enforcement`, and the S20 test has no exact cardinality/guard oracle. |
| G4 — no unsanctioned domain mocks | PASS | The injected enforcement/intercept ports are accepted driven boundaries; the issue is that the race test does not enter the worker boundary. |
| G5 — business language and Contract Shape | **FAIL** | Missing outcome anchors and incomplete bounded-change complements. |
| G6 — all required evidence green | **FAIL** | S25/S26 and action-shim evidence are missing; shared teardown-error behavior is not protected. |
| G7 — green before commit | PASS mechanically | DES GREEN precedes COMMIT and crafter reports terminating checks. |
| G8 — test budget | PASS provisionally | The active new methods are below the mapped 13-behavior budget of 26, but missing scenario slices make the count non-substitutive for coverage. |
| G9 — no prohibited test modification | PASS | The commit removes only the six pending `#[ignore]` markers in the acceptance file; existing assertions were not weakened. |

## RPP scan

- **Levels scanned:** L1-L4.
- **Cascade stopped at:** contract/test-integrity blockers.
- The unused `AbortOnDropListenerTask` is reported under D1 as an ownership
  contract defect, not as a style-only dead-code finding.

## Remediation disposition — iteration 1

| Finding | Status | Owner/action |
|---|---|---|
| **D1 — exact D11 task owner/abort ownership** | OPEN | Original step-02-03 crafter: implement the weak-sender, abort-on-drop, consume-on-shutdown owner and real channel-close evidence. |
| **D2 — shared teardown failure loses drain/retry ownership** | OPEN | Original step-02-03 crafter: preserve failed handles and retiring reservations until retry/typed terminal cleanup; add a failing shared-path regression. |
| **D3 — direct retirement-race test** | OPEN | Original step-02-03 crafter: drive the production shared listener/claim/enforcement path, not direct registry calls. |
| **D4 — S25/S26 absent** | OPEN | Original step-02-03 crafter: add two-capability isolated-stop and active owner-shutdown evidence at the accepted worker/Tier-3 boundary. |
| **D5 — action-shim result path absent** | OPEN | Original step-02-03 crafter: drive real `RegistrationRetired` from `start_alloc` through fail-closed action-shim cleanup. |
| **D6 — S20 cardinality/complement theater** | OPEN | Original step-02-03 crafter: use the existing recording adapter and assert exact owner cardinality/relinquish. |
| **D7 — Contract Shape evidence** | OPEN | Original step-02-03 crafter: add outcome anchors and complete delta/complement assertions, then rerun the mechanical checker. |

No finding is waived or deferred. Per repository DELIVER rules, remediation
returns to the original step-02-03 crafter and this step's reviewer re-reviews
until `APPROVED`; no later roadmap step may begin.

## Iteration-1 final verdict

**CHANGES_REQUIRED**

The commit is a meaningful implementation advance, but the accepted step is not
yet evidenced or contract-complete. The exact D11 owner shape is divergent, the
shared drain can lose failed teardown ownership, the key race test bypasses the
production path, the active-capability/listener scenarios and action-shim
caller are absent, and the remaining active tests do not prove the declared
cardinality/complement universes. Step 02-03 must remain blocked pending
remediation and re-review.
