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

## Iteration 2 — remediation commit `0bd52520a9ae15b2c58511fa599113d36c44a183`

- **Reviewer:** fresh isolated DELIVER reviewer replacement
- **Review ID:** `code_rev_20260922_012821_iteration_2`
- **Iteration:** 2
- **Reviewed cumulative range:** approved test checkpoint `4eb8aa34` through
  `0bd52520a9ae15b2c58511fa599113d36c44a183`
- **Prior iteration:** `2acfc29d471e0d811af3be62db1face521adc789`,
  **CHANGES_REQUIRED**
- **Final verdict:** **APPROVED**

### Iteration-2 summary

The remediation closes all seven iteration-1 blockers within the approved
02-03 architecture. The private D11 owner now has the exact weak sender,
observer-owned abort-on-drop listener task, consumed terminal-event slot, and
live-slot replacement refusal. Shared capability stop retains the same opaque
failed handle, its drain, and its Retiring reservation until a same-owner
retry completes; successor admission and release-last cleanup remain behind
that completion fence. The retirement race now drives the production shared
dispatch, not a direct test-owned `enforce`/`publish` sequence.

The active source-local owner tests and active worker acceptance bodies use the
real worker boundary. S-ND295-25 and S-ND295-26 have both source-local
production-owner evidence and the existing native real-enforcement bodies.
The action-shim test drives the real `StartAllocation` dispatch and proves the
`RegistrationRetired` cleanup projection. The exact seven-method public worker
surface remains unchanged, and no 03-03 clock, retry cadence, deadline,
request, or fail-stop surface was introduced.

### Updated iteration history

| Iteration | Commit | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `2acfc29d471e0d811af3be62db1face521adc789` | **CHANGES_REQUIRED** | 7 blockers | Returned to the original step-02-03 crafter. |
| 2 | `0bd52520a9ae15b2c58511fa599113d36c44a183` | **APPROVED** | 0 unresolved blockers | All D1-D7 findings resolved; step may advance. |

### Authority and review boundary

This re-review uses the exact accepted C-295-L/F-03, D-295-DISTILL-7,
D-295-DISTILL-11, D-295-DISTILL-15, GEN-295-A, and RUN-295-B contracts in
`feature-delta.md`, the 02-03 entry in `deliver/roadmap.json`, the
S-ND295-20..26 scenario table, and the approved remediation evidence table in
`distill/test-scenarios.md`. The review distinguishes the 02-03 published
worker prerequisite for S19 from the 03-03 cadence/deadline/fail-stop closure.

The two still-ignored acceptance bodies are the exact-port recovery and
occupied-recorded-port recovery bodies. They remain the later recovery lane;
they do not prevent approval of the current S19 observe-only prerequisite.
No 03-03 behavior is credited to this step.

No production, test, configuration, or execution-log file was changed by the
reviewer before this artifact append. Existing dirty `.serena/project.yml` and
`AGENTS.md` changes were preserved.

### Strengths confirmed in iteration 2

- The seven public worker methods remain exactly
  `start_shared_owner`, `wait_shared_owner_failure`, `converge_shared_owner`,
  `audit_shared_owner`, `start_alloc`, `stop_alloc`, and `shutdown_owner`.
  The remediation diff adds no `pub` declaration, generation setter, task
  control hook, clock, retry-cadence method, deadline method, request method,
  or fail-stop method.
- The D11 owner uses `mpsc::WeakSender` at
  `crates/overdrive-worker/src/mtls_intercept_worker.rs:351-375`; each
  observer captures an `AbortOnDropListenerTask` at `:454-468`, and its Drop
  aborts the underlying listener at `:330-335`.
- `wait_failure` consumes only the event's leg slot at `:379-406`, while
  `replace_terminal` refuses an occupied live slot at `:408-425`.
  `shutdown(self)` remains the exact consumed owner operation at `:428-439`.
- The private `shutdown_shared` Arc bridge is not public API. It uses the same
  abort-and-join ownership when the production `SharedOwner` still retains an
  Arc, and the enclosing owner is then dropped. The source-local consumed
  shutdown probe still proves that the sole receiver is dropped before the
  consuming operation returns.
- `CapabilityRegistry` snapshots include generation, every record's capability
  and lifecycle/effect/handle/in-flight/pending-owner fields, allocation
  reservations, and both indexes at `:1412-1472`.
- The action-shim and native bodies are wired to existing production ports and
  adapters. No no-op replacement port, unwired fixture, or test-only public
  dispatch hook was introduced.

## Prior-finding remediation dispositions

### D1 — D11 task-owner shape and detach risk — RESOLVED

The owner field is now exactly a weak sender. `SharedListenerTaskOwner::new`
creates the bounded channel, gives strong sender clones to the two observers,
and stores only `event_tx.downgrade()` (`mtls_intercept_worker.rs:365-376`).
`observe` moves one `AbortOnDropListenerTask` into each observer future
(`:454-468`); dropping or aborting that observer therefore aborts its listener
instead of detaching it.

The actual Tokio source-local tests cover the required boundaries:

- `dropping_every_observer_aborts_its_listener_and_closes_the_real_event_channel`
  (`:639-672`) aborts both observers, observes both child Drop witnesses, and
  observes `TryRecvError::Disconnected` from the real channel.
- `one_real_join_event_removes_only_its_terminal_slot_before_replacement_and_consumed_shutdown`
  (`:674-702`) receives a real leg-F join event, proves only leg-F is removed,
  proves live leg-C retention, replaces only the consumed slot, and uses the
  strong sender probe to prove consuming `shutdown(self)` drops the receiver.
- `replacement_refuses_a_still_live_occupied_slot_without_detaching_either_listener`
  (`:704-723`) proves a live occupied slot is refused and both original child
  tasks remain live until the owner consumes shutdown.

The observer-close classifier remains source-honest, and the production
listener task created at `:1277-1331` remains the owner path that emits the
real event. The prior strong-sender/borrowed-shutdown/detach finding is closed.

### D2 — shared teardown failure loses retry ownership — RESOLVED

`AllocStop` now retains both `retry_handles` and an optional `retry_drain`
(`mtls_intercept_worker.rs:1885-1917`). The shared capability stop branch
waits for claims, takes the handles, clones each failed handle before invoking
teardown, and retains the drain/elements and Retiring reservation when any
teardown fails (`:2968-2993`). A subsequent same-owner `stop_alloc` takes the
retained drain and exact failed handles (`:2917-2942`) and uses
`start_capability_drain_retry` (`:3673-3705`). The drain completes only after
all retries succeed; only then can a successor reserve the address.

The real production-path regression
`shared_teardown_failure_retains_the_exact_handle_drain_and_reservation_until_same_owner_retry`
(`:4904-5021`) drives `start_shared_owner`, `start_alloc`,
`handle_shared_outbound`, and `spawn_shared_enforcement`. It proves the first
typed teardown source, stable `<allocation>#0` identity, failed-handle retry
ownership, Retiring conflict for a contender, exactly two teardown calls with
the same identity, successful same-owner retry, completion, and successor
admission only after completion.

The release-last complement is independently proven by the action-shim
`RegistrationRetired` body at
`crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs:1886-1929`:
the network lease is still held while structural teardown runs, mTLS element
drops precede network teardown, and successful cleanup releases the address
only after the accepted cleanup sequence. Owner shutdown remains a sealed
terminal owner path, not a new retry API; this review does not credit it with
03-03 recovery behavior.

### D3 — retirement-race test bypasses shared dispatch — RESOLVED

`enforcement_returning_after_retirement_tears_down_the_real_returned_handle_before_drain`
now starts the shared owner and a networked capability through
`start_alloc` (`mtls_intercept_worker.rs:4798-4803`). It invokes
`handle_shared_outbound` from a blocking executor task (`:4804-4809`), which is
the same production dispatch called by `shared_listener_task` at
`:1297-1302`. That path claims the immutable capability, resolves it, calls
`spawn_shared_enforcement` (`:3445-3482`), holds the claim across the awaited
`enforce` call (`:3521-3543`), and publishes or tears down the returned handle
according to the retirement fence.

The test parks stop on the in-flight claim, releases enforcement, and proves
the late returned handle is torn down exactly once and is not published or
reattributed to a successor (`:4810-4831`). There is no direct test-owned
`enforcement.enforce` followed by `claim.publish` workflow left.

### D4 — missing S-ND295-25/S-ND295-26 shared-worker evidence — RESOLVED

The source-local acceptance bodies now use two networked allocations and the
real shared worker:

- `stopping_one_shared_allocation_preserves_the_unrelated_handle_and_complete_listener_owner`
  (`tests/acceptance/netns_density_shared_owner.rs:891-944`) starts two
  capabilities, drives two shared listener connections, stops only the first,
  asserts the second handle is byte-identical and live, checks both shared
  listener addresses/tasks through `audit_shared_owner`, proves the first
  allocation's three guards are the only guard delta, and completes another
  second-allocation exchange before shutdown.
- `owner_shutdown_waits_the_active_claim_then_drains_every_shared_capability_and_listener`
  (`:946-998`) starts two capabilities, holds a third real claim in
  enforcement, closes admissions, proves shutdown remains pending, then
  releases the claim and asserts all three handles, six allocation elements,
  both listener sockets, and the node owner lifecycle are drained. The shared
  guard is privately relinquished with zero Drop.

The native integration bodies are active and exercise the existing
`HostMtlsEnforcement`, TLS peers, shared listener dispatch, and real kernel
cleanup at `tests/integration/outbound_enforce_substrate_splice.rs:1936-2001`
and `:2003-2069`. The crafter's final report records both native S25 and S26
selectors passing. The previously reported 27 KVM/VMM and 54 passed plus 6
skipped zero-copy metal suites remain additional non-regression evidence; they
are not substituted for the S25/S26 assertions.

### D5 — `RegistrationRetired` not driven through action shim — RESOLVED

`registration_retired_from_real_start_alloc_keeps_exec_closed_and_releases_the_address_last`
is active at
`crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs:1886-1930`.
Its helper starts the real shared owner, constructs a networked
`StartAllocation`, calls the production `dispatch_with_network_provisioner`
at `:1808-1848`, and races the real worker Pending-to-Retired activation
barrier with `stop_alloc` (`:1851-1869`). It does not fabricate
`MtlsInterceptInstallError::RegistrationRetired` for the closed path.

The assertions prove Running-to-Failed replacement, exact
`registration_retired` stage on the successful-cleanup partition, zero EXEC
release and zero driver-running hook, driver stop before mTLS element drops,
mTLS cleanup before network teardown, address release last, and unchanged
primary-versus-cleanup error precedence on the failing structural-teardown
partition (`:1893-1929`). The action shim remains the real production caller
and no second cleanup generation is introduced.

### D6 — S20 lacks exact cardinality/guard evidence — RESOLVED

`shared_owner_starts_once_audits_and_shutdown_drains_the_owner_tree`
(`tests/acceptance/netns_density_shared_owner.rs:55-104`) drives the existing
recording adapter with real `TcpListener` sockets and a real Drop-counted
guard. It asserts exactly two binds, one convergence, the exact observation
cardinality, two distinct non-zero listener addresses, two retained listener
clones, zero allocation-guard drops, and zero shared-guard drops while
published (`:63-79`). The production `audit_shared_owner` invoked by the test
checks both exact socket identities and both live task slots before the
recorded shared identity is accepted.

The second `start_shared_owner` is asserted byte-equal to the first complete
recording surface (`:80-88`), so no duplicate bind/converge/task/guard tree
can be hidden behind an idempotent result. Shutdown closes both exact sockets,
retains the accepted constant program through private guard relinquishment,
and again records zero guard Drop (`:90-103`). The prior Sim-only cardinality
gap is closed without a public counter or hook.

### D7 — Contract Shape declarations and complete universes — RESOLVED

Every new or transitioned body named by the remediation carries both exact
rustdoc lines:

```text
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
```

Direct source inspection confirms these declarations on the three D11 owner
bodies, both full-registry bodies, the production retirement race, shared
teardown retry, S20, source-local S25/S26, native S25/S26, and the action-shim
body. No transitioned name matches the banned technical-result regex.

The registry's `RegistryUniverseSnapshot` captures the complete declared
registry surface: `next_generation`, all records and capability fields,
lifecycle, guard/effect presence, handles, in-flight count, pending-owner
state, allocation reservations, and source/destination indexes. The conflict
rows compare byte-equal snapshots; the isolated retirement row compares the
post-state with `without(&first_key)` while retaining the unrelated capability
complement (`:1497-1562`, `:1684-1733`). The worker owner and S20 tests
assert their complete adapter/owner complements rather than only selected
lookup results.

No assertion was weakened, deleted, skipped, or rewritten to accommodate the
implementation. The remediation removes only the exact step-02-03 pending
ignore markers from the bodies it closes; the two later recovery bodies retain
their explicit ignores.

## Public API and scope re-check

The accepted public worker surface is unchanged. Comparing the remediation
diff against `4eb8aa34` shows no added `pub` declaration. The existing
constructor, `leg_c_addr`, and test-only `#[doc(hidden)]` diagnostics remain
outside the seven-method owner surface; none was added by remediation.

The worker still uses the already-required injected `Clock` field only as
constructor wiring. The remediation adds no clock reads, retry cadence,
deadline, attempt counter, request sender, typed fail-stop request, or
control-plane supervisor call. The only retry code is the accepted
allocation-stop same-owner teardown retry required by D7; it is not the 03-03
runtime recovery loop. The active wrong-target body remains the S19
observe-only prerequisite: it asserts one structured conflict, no port-zero
rebind, no target rewrite, and no retry/deadline/fail-stop closure.

The published node guard is privately relinquished on owner shutdown with
`std::mem::forget`, while unpublished startup/refusal paths drop acquired
guards and sockets. Registry retirement remains remove-before-reassign: active
indexes are removed at `begin_retire`, reservations remain through claims,
pending-owner handoff, element/handle teardown, and `complete`, and only then
can a successor register the address.

## Contract Shape Compliance — iteration 2

**Overall: PASS.**

| Check | Status | Evidence |
|---|---|---|
| Declaration present | PASS | All new/transitioned 02-03 bodies carry the exact Outcome anchor and bounded-change declaration. |
| Banned technical names | PASS | Activated names do not match `returns_*`, `exit_code`, `calls_*_once`, `status_code`, or `http_*` result-name patterns. |
| D11 unbounded ownership | PASS | Weak sender, observer-owned abort wrapper, consumed shutdown, and exact slot ownership are source-local and production-wired. |
| Bounded-change declared delta | PASS | Registry, owner, S20, S25/S26, and action-shim bodies assert the named deltas. |
| Complement equality | PASS | Registry snapshots, owner surfaces, unrelated handles/listeners, guard counts, cleanup journals, and action-shim order cover the named complements. |
| Driving boundary | PASS | Worker acceptance enters public owner/allocation methods; retirement and action-shim bodies enter production dispatch; native bodies use real enforcement. |

## Mechanical evidence — iteration 2

| Gate | Result |
|---|---|
| DES RED/GREEN/COMMIT | PASS — fresh remediation cycle is ordered `RED` `2026-09-22T00:34:01Z`, `GREEN` `2026-09-22T01:12:57Z`, `COMMIT` `2026-09-22T01:14:11Z`; each is `EXECUTED/PASS`. |
| Commit identity | PASS — `0bd52520a9ae15b2c58511fa599113d36c44a183`; original Marcus author retained. |
| Commit trailers | PASS — exactly one `Step-Id: 02-03` and exactly one `Co-Authored-By: Codex <codex@openai.com>`; no Claude/Anthropic attribution. |
| Commit scope | PASS — five tightly related files, 172 insertions and 52 deletions relative to `4eb8aa34`; no unrelated production file. |
| `git diff --check` | PASS. |
| `cargo fmt --all -- --check` | PASS. |
| Crafter workspace check | PASS — reported `cargo check --workspace --all-targets`. |
| Crafter focused clippy | PASS — reported `overdrive-worker --lib --features integration-tests -D warnings`. |
| All-target clippy | Baseline-limited only — pre-existing `overdrive-netlink` and unrelated control-plane findings were reported; no changed-code warning was reported. |
| Worker verification | PASS — crafter report records worker suite 32 passing after remediation. |
| Active owner acceptance | PASS — eight 02-03 bodies active; two exact-port recovery bodies remain ignored for the later recovery lane. |
| Action-shim integration | PASS — real `RegistrationRetired` dispatch body passes both cleanup partitions. |
| Native S25/S26 | PASS — crafter report records both native real-enforcement selectors passing. |
| Native metal non-regression | PASS — 27 KVM/VMM tests and 54 zero-copy tests passed, with 6 zero-copy tests skipped by their existing environment policy. |
| Lima | Environment-limited — configured Lima target is read-only; the alternate writable target exhausted `/tmp`. No Lima-only 02-03 gate remains unproved because the source-local and native S25/S26 evidence is the accepted layer for these bodies. |
| Mutation testing | NOT RUN, correctly deferred to the final DELIVER-wave gate. |

## External validity — iteration 2

**PASS.** The active acceptance file calls the public worker owner/allocation
surface and real sockets. The retirement race enters the production shared
listener dispatch method and its production claim/enforcement/publish path.
The S25/S26 source-local bodies exercise the real worker owner with two active
capabilities, while their native counterparts exercise `HostMtlsEnforcement`
and real TLS peers. The action-shim body enters the real `dispatch` caller and
observes operator-visible lifecycle rows, cleanup effects, and EXEC behavior.
No expectation runner invokes a Rust test binary or replaces the production
composition root.

## Quality gates — iteration 2

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance set | PASS | The exact eight 02-03 bodies are active; the two later recovery bodies remain ignored as required. |
| G2 — valid RED | PASS | The fresh DES RED event is present and PASS; no collection/import failure is reported for the remediation cycle. |
| G3 — assertions protect production behavior | PASS | D3 uses production shared dispatch, D2 uses real retry ownership, D5 uses real action dispatch, and S20/S25/S26 observe complete owner complements. |
| G4 — no unsanctioned domain mocks | PASS | Doubles are at the approved `MtlsIntercept`, `MtlsEnforcement`, and network/driver port boundaries; native evidence uses the real enforcement adapter. |
| G5 — business language and Contract Shape | PASS | Outcome anchors, bounded-change declarations, exact lifecycle vocabulary, and complete universe/complement assertions are present. |
| G6 — required evidence green | PASS | D1-D7 source/acceptance/action-shim evidence is green; native S25/S26 is reported green; no 03-03 evidence is claimed. |
| G7 — green before commit | PASS | DES GREEN precedes COMMIT in the fresh remediation cycle. |
| G8 — test budget | PASS | The mapped 13 observable behaviors retain a 26-test budget; the active bodies remain within it and no duplicated input matrix was added. |
| G9 — no prohibited test modification | PASS | Remediation only removes named pending ignores and strengthens/activates the accepted bodies; no assertion was weakened or deleted. |

## RPP scan — iteration 2

- **Levels scanned:** L1-L4.
- **Cascade stopped at:** no RPP blocker; the private `shutdown_shared` Arc
  bridge is a bounded ownership adapter for the existing owner and not a
  speculative public abstraction.
- No unrelated cleanup, generalized lifecycle hardening, or architecture
  expansion was introduced while remediating D1-D7.

## Iteration-2 verdict

All seven iteration-1 blockers are resolved in the original step-02-03
architecture. The exact public API remains unchanged; the private D11 and D7
ownership contracts are now executable; production dispatch, action-shim,
source-local, and native evidence are all present at their required
boundaries. The remaining Lima limitation does not invalidate the metal or
source-local evidence and is not an exact Lima-only gate for this step.

# APPROVED
