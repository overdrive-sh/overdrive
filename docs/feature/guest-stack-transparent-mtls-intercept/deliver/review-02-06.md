# Step 02-06 Review

## Iteration 1 metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Step | `02-06` |
| Reviewed commit | `9e5c629dafbac56f0a947f34e6a1e6cbb58830fd` |
| Parent | `6691bb677c7e56241ff39547363ea9709dccbf01` |
| Subject | `test(guest-stack-mtls): prove restart and stop cleanup` |
| Trailer | `Step-Id: 02-06` |
| Review iteration | 1 |
| Verdict | **NEEDS_REVISION** |

## Summary

The four qualified native tests pass, and substantial parts of their kernel
oracles are strong. S-GTI-06b drives the real INPUT-hook failure and restores
the exact packet-path baseline; S-GTI-12a compares the full ordered rule
sequence before and after removal of the exact target handle; S-GTI-12b
observes `Stopped` followed by `AlreadyStopped`; and the restarted-flow helper
checks the same allocation id, Platform Reclamation history, pre-EXEC guard,
exact D7 accounting, guest plaintext, peer-wire non-plaintext, and bidirectional
kTLS. The tests remain in-process Rust production-composition tests and do not
spawn the built product, emit expectation evidence, introduce E08/E09, exercise
a legacy/no-token path, or create a Service-plus-VM category.

Those green results do not establish the mapped restart contract. Both 06
scenarios call the explicitly graceful `ServeHandle::shutdown()` while naming
it unclean. The success scenario additionally rewrites the target rootfs
executable, replaces the peer executable, kills the peer manually, and authors
a replacement peer observation between boots. It therefore does not preserve
the exact data and standing workload behavior whose production reclamation
route is under test.

The supporting idempotency evidence also contains a live correctness gap.
Replaying `Action::FinalizeFailed` currently writes a new timestamped row,
re-runs terminal hooks and destructive cleanup, and emits another lifecycle
event. The new test passes because it does not observe any of those effects.
The illegal-reopening property is similarly vacuous, and the reclamation-once
test proves only a repeated boot-reclamation row write, not the mapped
same-allocation lifecycle re-drive count. Contract Shape declarations and two
required executable-map names are also incorrect, and the DES RED event covers
only one of the seven mapped contracts.

## Scope and boundary audit

The commit changes five files: four Rust test files and
`execution-log.json`, for 742 insertions and 33 deletions. There is no
production-code, public API, persisted-schema, wire, REST/OpenAPI, Cargo,
example, expectation, or BPF diff. The pre-existing untracked files remain
untouched.

| Boundary | Result |
|---|---|
| Production composition | PASS structurally — CLI command libraries, production control-plane boot/reconcile/action paths, the real VM driver, real nftables, real cgroup/KVM, and production mTLS worker are composed |
| Built-product boundary | PASS — no Rust test spawns the built Overdrive binary or invokes a Rust test runner |
| Example/expectation/integration separation | PASS — no example or expectation file changed and no expectation evidence is emitted |
| E08/E09 | PASS — neither path exists in the target diff |
| Legacy/no-token path | PASS — none introduced |
| Service-plus-VM | PASS — the VM workload is Job-kind; the independent peer is an Exec-backed Service |
| 02-05 cleanup authority | No direct source regression, but duplicate finalization re-runs cleanup and violates the retained no-double-clean obligation; see D3 |
| Mutation discipline | PASS — no per-step mutation run or exclusion change |
| Test budget | PASS mechanically — seven mapped executable contracts are within the ceiling of fourteen, although several do not prove their named behavior |

## Criterion disposition

| Criterion | Result | Evidence |
|---|---|---|
| S-GTI-06a | **FAIL** | Same-id Platform Reclamation and D7 assertions are present, but the server shutdown is graceful and the target executable/data are replaced between boots (D1, D2). |
| S-GTI-06b | **FAIL** | Real INPUT-hook failure, no EXEC/frame, exact same id, typed failure, and restoration are strong; the boot boundary is nevertheless graceful, not the required unclean restart (D1). |
| S-GTI-12a | PASS behaviorally | The test removes the exact target handle and proves ordered full `after == before` filtered only by that handle, while preserving the sibling row/rule. Its declared Contract Shape is wrong (D5). |
| S-GTI-12b | **PARTIAL** | `Stopped` then `AlreadyStopped`, absent-target preservation, and sibling preservation pass. A fixed sleep plus an unchanged empty-target baseline cannot prove that later finalization/reclamation invoked destructive cleanup exactly once (D3), and the function name is unmapped (D5). |
| P-GTI-ILLEGAL-07 | **FAIL** | The property constructs terminal rows but never drives a later READY, EXEC, or duplicate-finalization event through the lifecycle (D4). |
| C-GTI-RECLAMATION-ONCE | **FAIL** | Repeated boot convergence leaves the reclamation row unchanged, but no lifecycle re-drive or action count is observed, and the pure-function declaration is absent (D4, D5). |
| C-GTI-FINALIZE-TWICE | **FAIL** | The test deliberately dispatches twice but omits the timestamp, write/event, driver-hook, mTLS-stop, and teardown surfaces on which the duplicate is observable; production duplicates those effects (D3). |

## Findings

### D1 — both “unclean restart” scenarios execute the graceful shutdown path

- **Severity:** Critical
- **Dimensions:** External validity, production-path accuracy, acceptance honesty
- **Affected contracts:** S-GTI-06a, S-GTI-06b
- **Evidence:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4056-4059`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4193-4197`
  - `crates/overdrive-cli/src/commands/serve.rs:87-98`
  - `crates/overdrive-control-plane/src/lib.rs:1264-1338`

Both tests call `boot_one.shutdown().await` and label the call “unclean.” The
method's public contract says the opposite: it triggers graceful shutdown,
awaits the convergence/workflow/router tasks, gracefully drains the server,
then cancels and awaits the exit observer. That is an orderly lifecycle path,
not an operator process death. It can complete in-flight writes and
coordination that the required crash/restart boundary must leave incomplete.
Passing boot reclamation after this call therefore does not prove that an
uncleanly ended `serve` process with standing intent follows the same-id route.

**Required remediation:** drive a real abrupt ownership loss after the durable
Running observation without calling the graceful shutdown/stop path, retain a
safe fixture owner for eventual process/resource cleanup, and start boot two
against the exact same data/config directories. Both the success and failure
scenarios must demonstrate that this unclean boundary—not a graceful drain—causes
boot-epoch Platform Reclamation and the same-id `RestartAllocation`.

### D2 — S-GTI-06a substitutes the workload and manually authors lifecycle state between boots

- **Severity:** Critical
- **Dimensions:** Fixture truthfulness, same-data invariant, production lifecycle composition
- **Affected contract:** S-GTI-06a
- **Evidence:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4001-4018`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4060-4096`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4098-4109`

Boot one runs a spin binary at `/sbin/gti-restart-guest`; between boots the
test mounts the target rootfs and overwrites that same path with a different
mesh guest. It also overwrites the standing peer's executable, sends SIGKILL
to its fixture-authored PID, opens `observation.redb` directly, changes the
peer row to `Failed`, increments its logical timestamp, and writes the row
back. The comment that the command and data are unchanged is true only of the
path string, not the executable or durable state.

The observed success is consequently conditional on harness substitution:
boot one is deliberately non-terminating, boot two is deliberately finite,
and the peer becomes restartable because the test writes the conclusion the
production lifecycle was supposed to observe. This masks exactly the
unchanged-data/standing-intent route the scenario owns and makes the later
natural Job exit an artifact of a different executable.

**Required remediation:** use one byte-identical target rootfs/executable and
one unchanged standing intent across both boots. Arrange the guest's behavior
so the same program can remain live before the crash and complete naturally
after the real reclaimed restart, without rewriting its image. Do not write
allocation observations or timestamps manually. If the peer must change
state, drive that through its real process-observation/lifecycle path, or use a
separate externally owned peer fixture that is not part of the durable
workload history under test.

### D3 — duplicate finalization is a real second transition and destructive cleanup, while the new oracle cannot see it

- **Severity:** Critical
- **Dimensions:** Idempotency, cleanup ownership, state-transition correctness, test effectiveness
- **Affected contracts:** C-GTI-FINALIZE-TWICE, S-GTI-12b, retained 02-05 no-double-clean guarantees
- **Evidence:**
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:181-260`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:491-518`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1422-1431`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1519-1568`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1593-1624`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4349-4364`

`same_job_finalization_is_terminal_and_count_preserving` invokes the action
twice, but compares only state, terminal, restart count, history, reason,
detail, stderr, kind, and workload address. On the second invocation the live
action arm has no already-finalized fence: it computes another dominating
`updated_at`, writes another observation, calls `on_alloc_terminal` again,
removes the driver index again, calls `mtls_worker.stop_alloc` again, invokes
the netns teardown/release helper again, and emits another lifecycle event.
Because the first finalization removed the allocation-driver index, the second
driver resolution also falls back to every composed driver instead of the
original exact kind. A successfully released network slot makes the second
network-provisioner call a no-op, but that incidental guard does not prevent
the duplicate row, event, driver hooks, or mTLS-stop call.

The helper structurally hides every one of those effects. Each invocation
uses a fresh observation store; the driver has no terminal-hook counter; the
lifecycle receiver is discarded; `mtls_worker` is `None`; and the net-slot
allocator is empty. The omitted `updated_at` comparison alone would expose the
second transition. S-GTI-12b has the same oracle weakness at the native edge:
after the two stop commands it sleeps for 250 ms and compares an already-empty
packet-path baseline. Repeating an idempotent kernel deletion or terminal hook
would leave that baseline equal, so the assertion cannot support its
“repeated reclamation/finalization ... cannot duplicate work” message.

**Required remediation:** make same-claim finalization replay a total no-op at
the production action boundary, before durable write, terminal hooks, mTLS
stop, netns teardown, event emission, or counter changes. Add a production-
composition test that reuses one store and counting driven ports and asserts
the complete complement: unchanged row including `updated_at`, one durable
transition, one lifecycle event, one driver terminal hook, one mTLS stop, one
network teardown/release, unchanged exact exit code, and unchanged restart
count. Drive repeated lifecycle/reclamation evaluation explicitly rather than
using a sleep, and retain the 02-05 rule that cleanup ownership cannot be
bypassed or cleaned twice.

### D4 — the illegal-event and reclamation-once support contracts do not drive their named behavior

- **Severity:** Major
- **Dimensions:** Property strength, behavioral traceability, testing theater
- **Affected contracts:** P-GTI-ILLEGAL-07, C-GTI-RECLAMATION-ONCE
- **Evidence:**
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1603-1649`
  - `crates/overdrive-control-plane/src/vm_reclamation_boot.rs:280-372`
  - `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md:482`
  - `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md:498-499`

`terminal_vm_job_rejects_every_reopening_event` does not generate reopening
events. It chooses six reason/terminal pairs, writes the terminal directly
onto a fabricated row, and asserts either `(is_natural_exit && terminal is
Some)` or `!is_restartable`. For the three natural-exit cases the left side is
true by construction; for the intentional-stop cases the old classifier is
all that is tested. No READY, EXEC, duplicate finalization, action trace, or
state/view delta is driven, so the property can stay green if a later event
actually reopens the attempt.

`same_boot_epoch_claims_each_unsupervised_allocation_once` repeats only
`vm_reclamation_boot::converge` and compares the already-terminal row. It does
not run the standing-intent workload lifecycle, count the same-id
`RestartAllocation`, or show that repeated executor/lifecycle evaluation emits
at most one re-drive. Its new name promises the joined contract while its
assertions remain the older boot-row idempotency test.

**Required remediation:** drive the canonical lifecycle/reconcile boundary
over the full terminal plus late READY/EXEC/duplicate-finalization event set
and assert the exact empty action/unchanged state complement, with Platform
Reclaimed as the sole reopening class. For reclamation-once, compose boot
reclamation with standing-intent lifecycle evaluation and capture the exact
action trace, proving one Platform Reclamation claim and at most one same-id
re-drive for the boot epoch under repeated evaluation.

### D5 — executable-map names and Contract Shape declarations do not match the approved roadmap

- **Severity:** Major
- **Dimensions:** Mechanical traceability, repository test policy
- **Evidence:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:3987-3993`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4153-4159`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4603-4609`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4695-4702`
  - `crates/overdrive-control-plane/src/vm_reclamation_boot.rs:278-281`
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/roadmap.json:147-160`
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/roadmap.json:212-225`
  - `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md:216-225`

S-GTI-06a, S-GTI-06b, and S-GTI-12a are mapped as `bounded-change` but
declare `unbounded-preservation`. C-GTI-RECLAMATION-ONCE is mapped as a
pure-function property but has no exact
`/// CONTRACT_SHAPE: pure-function.` declaration. S-GTI-06b is implemented as
`a_same_id_restart_that_cannot_reinstall_the_guard_fails_before_exec` instead
of mapped function
`failed_re_enrolment_after_platform_reclamation_stays_closed`; S-GTI-12b is
implemented as
`stopping_a_terminal_pre_ready_vm_is_idempotent_and_never_recreates_its_guard`
instead of `job_stop_without_a_guest_egress_guard_is_idempotent`.

**Required remediation:** restore the exact executable-map function names and
the mapped per-test Contract Shape declarations. Every live pure-function
property must carry the repository's exact rustdoc line.

### D6 — the DES RED evidence covers one scaffold, not the seven mapped contracts delivered by the commit

- **Severity:** Major
- **Dimensions:** TDD phase honesty, evidence completeness
- **Evidence:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/execution-log.json` (`02-06` RED/GREEN/COMMIT events)

The single RED event says exactly one qualified native S-GTI-06 scaffold was
selected and failed. The final commit implements four stakeholder examples
and three support contracts. There is no recorded failing RED for S-GTI-06b,
S-GTI-12a, S-GTI-12b, P-GTI-ILLEGAL-07,
C-GTI-RECLAMATION-ONCE, or C-GTI-FINALIZE-TWICE. In particular, the two
support tests whose assertions are ineffective appear only as green code.
This is not evidence of one-test-at-a-time RED→GREEN for the behavior shipped
by the step.

**Required remediation:** preserve the existing audit history, add tests that
fail on each corrected production/fixture defect for the intended reason, and
append a fresh chronological RED→GREEN→COMMIT cycle describing the actual
selected failures and verification. Do not rewrite the prior events.

## Strong evidence retained

The findings above do not invalidate every assertion in the step:

- S-GTI-06b's `ProductInputHookFixture` failure is the real production
  install hook, fails with the typed `MtlsInterceptInstallFailed` cause, emits
  no EXEC or guest frame, performs VM cleanup, and restores the exact captured
  packet-path baseline.
- S-GTI-06a's post-restart observation helper ties the same allocation id and
  Platform Reclamation history to exact VM network identity, arms tap and
  host-veth capture before release, proves no pre-ready frames, validates the
  full D7 counter/capture equality, observes guest plaintext, rejects peer-wire
  plaintext, and requires TLS application records in both directions.
- S-GTI-12a selects exact typed target and sibling rules, removes the target by
  handle, and compares the entire ordered full-chain complement rather than a
  selected-rule projection.
- S-GTI-12b does directly establish the API outcomes `Stopped` then
  `AlreadyStopped`, plus the absent target and sibling-preservation outcome.
- The final commit contains no product-boundary or verification-layer leakage,
  and no mutation exclusion or per-step mutation run.

These are useful components of the remediation, but they cannot compensate
for the wrong restart boundary, changed workload data, or invisible duplicate
effects.

## DES and commit chronology

RED (`07:20:56Z`), GREEN (`08:32:19Z`), and COMMIT (`08:32:51Z`) are ordered.
The COMMIT event names `5aa8005bebf5f0c5a4a0fe975c902d4cf30a4eba`,
which is retained in reflog as the initial implementation commit at
`08:32:37Z`. That object has the same parent, tree, subject, and trailer as the
reviewed work except that its execution log does not yet contain the COMMIT
event. The event was then appended and the commit amended to
`9e5c629dafbac56f0a947f34e6a1e6cbb58830fd` at `08:33:04Z`.

That hash difference is honest, recoverable log-bearing amend chronology and
is not a finding. The remaining DES defect is D6's incomplete RED coverage,
not the COMMIT hash.

## Independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 6691bb67..9e5c629d` | PASS |
| `execution-log.json` parse | PASS |
| Focused Lima support-contract selection | PASS — 3/3; 589 skipped |
| Qualified native S-GTI-06a/06b/12a/12b selection | PASS — 4/4; 263 skipped; 87.925s |
| Affected-package all-target/all-feature clippy with `-D warnings` | PASS for `overdrive-cli`, `overdrive-control-plane`, and `overdrive-reconcilers` |
| Workspace-wide all-target/all-feature clippy | FAIL before target linting in the no-std `overdrive-bpf` binary because unwinding panics are unsupported; the step has zero BPF/Cargo diff and the affected-package gate above passes |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

The first reviewer metal invocation failed closed because the guest artifact
selectors were omitted. The reported native result is the
canonical qualified rerun using
`/srv/vm/overdrive-testing/{kernel,rootfs.ext4}` under one metal lease.

## Iteration 1 verdict

**NEEDS_REVISION.** Do not complete step 02-06 or advance the DELIVER wave.
Return D1-D6 to the original 02-06 crafter. The next review must see a genuine
unclean same-data restart for both 06 scenarios, no workload/observation
substitution, an idempotent production finalization boundary with exact
side-effect counts, non-vacuous illegal-event and reclamation-once properties,
the exact mapped names and Contract Shapes, and an honest new RED→GREEN→COMMIT
cycle. Continue the uncapped remediation/re-review loop until the reviewer
returns **APPROVED**.

---

## Iteration 2 metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Step | `02-06` |
| Reviewed commit | `6000b2e9e31faea2f57ba4ea63251d0cfc7e2e93` |
| Parent | `9e5c629dafbac56f0a947f34e6a1e6cbb58830fd` |
| Subject | `fix(mtls): close restart lifecycle remediation` |
| Trailer | `Step-Id: 02-06` |
| Review iteration | 2 |
| Verdict | **NEEDS_REVISION** |

## Iteration 2 summary

The remediation closes several literal findings. Both restart scenarios now
abort the top-level server tasks instead of calling graceful `shutdown`; the
target VM uses one staged rootfs, command, intent, allocation id, and data
directory across both boots; no test authors a replacement target observation;
the mapped 06b/12b names and stakeholder Contract Shape declarations are exact;
and the seven-contract RED event is explicit. The happy replay of one exact
`FinalizeFailed` claim is now a no-op before a second write, hook, mTLS stop,
network teardown, or event. The qualified native selection remains green 4/4.

Those green tests still do not prove the approved contracts. The durable
finalization row is written *before* its fallible network cleanup, but that same
row is now the early-return replay fence. A cleanup error or process loss after
the row write therefore makes the unfinished cleanup permanently
non-retryable. The new happy-only counting test cannot expose that crash/error
window.

The restart fixture also does not model loss of the old process owner. It keeps
a strong `Arc<MtlsInterceptWorker>` alive, so the old blocking accept/enforce
loops and listeners survive; swapping its resolver to a frozen snapshot makes
that old userspace dataplane deliberately functional. S-GTI-06a then extracts
the production allocation spec of a peer that is itself in the durable history
and directly calls identity issuance plus `MtlsInterceptWorker::start_alloc`
on boot two. This manually installs/replaces the peer intercept that the module
contract says only production boot/allocation may create.

The two source-contract remediations also remain dishonest. The illegal-event
property mutates a terminal row to `Pending`/`Running` before calling the
reconciler and then asserts that the reopened row remains unchanged; it never
applies or rejects a READY/EXEC event. The reclamation-once function is declared
`pure-function` while writing stores, mutating host state, and executing boot
reclamation. Finally, a default-feature affected-package clippy run fails on
the newly added test-only state compiled into production; the all-features gate
masked it.

## Iteration 2 remediation disposition

| Iteration 1 finding | Disposition | Evidence |
|---|---|---|
| D1 — graceful shutdown mislabeled unclean | **PARTIAL / still blocking** | Top-level server tasks are aborted, but the old mTLS worker and its accept/enforce tasks remain live behind `AbruptServerResidue` (D8). |
| D2 — target/workload substitution | **PARTIAL / still blocking** | The target rootfs, executable, intent, data, and observation are no longer rewritten. S-GTI-06a instead manually re-homes a durable peer through a test-only identity/intercept installation path (D9). |
| D3 — duplicate finalization effects | **PARTIAL / still blocking** | Immediate successful replay is fenced and its happy-path counts are strong, but the durable fence precedes fallible cleanup and suppresses recovery after partial completion (D7). |
| D4 — vacuous illegal/reclamation contracts | **PARTIAL / still blocking** | Reclamation now observes one same-id action and no second action under the returned View. The illegal property still starts from already-reopened state and preserves it (D10). |
| D5 — names and Contract Shapes | **PARTIAL / still blocking** | Executable names and stakeholder declarations are exact. C-GTI-RECLAMATION-ONCE's `pure-function` declaration is factually false (D11). |
| D6 — one-of-seven RED | **CLOSED** | The appended RED event explicitly selects all seven contracts and distinguishes the actually failing corrections from scenarios whose behavior remained green. Chronology is recoverable and ordered. |

## Iteration 2 criterion disposition

| Criterion | Result | Evidence |
|---|---|---|
| S-GTI-06a | **FAIL** | Same target id, Platform Reclamation history, natural exit, and D7 assertions are strong, but the old worker survives and the mesh peer is manually installed/replaced through test-only production ports (D8, D9). |
| S-GTI-06b | **FAIL** | Same data/id, real INPUT-hook failure, no EXEC/frame, and restoration are strong; the crash boundary still retains the old live mTLS worker rather than ending all old userspace ownership (D8). |
| S-GTI-12a | PASS | Exact target-handle deletion and ordered full sibling complement remain directly observed. |
| S-GTI-12b | **PARTIAL** | `Stopped` then `AlreadyStopped`, absent-target preservation, and sibling preservation are direct. Its broader repeated-finalization/no-double-clean claim is contradicted by D7's stranded partial-cleanup window. |
| P-GTI-ILLEGAL-07 | **FAIL** | READY/EXEC are represented by pre-mutating the row to `Pending`/`Running`; the property then accepts and preserves that hybrid reopened row (D10). |
| C-GTI-RECLAMATION-ONCE | **PARTIAL** | The function observes one same-id `RestartAllocation` and no second same-id action with the returned View, but violates its mapped pure-function contract (D11). |
| C-GTI-FINALIZE-TWICE | **FAIL** | Immediate successful replay has an excellent complete-effect complement. A first dispatch whose teardown fails after the terminal write cannot finish on replay (D7). |

## Iteration 2 findings

### D7 — the finalization fence makes partially completed cleanup non-retryable

- **Severity:** Critical
- **Dimensions:** Crash consistency, retry safety, cleanup ownership, retained
  02-05 guarantees
- **Affected contracts:** C-GTI-FINALIZE-TWICE, S-GTI-12b, 02-05
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1432-1453`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1591-1646`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1287-1321`
  - `crates/overdrive-control-plane/src/action_shim/reclamation.rs:195-218`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:624-686`

The new fence returns as soon as the current row already carries the exact
terminal claim. On the first dispatch, however, the action writes that row at
line 1591 and only afterwards calls driver terminal hooks, removes the driver
index, stops mTLS, tears down the network namespace, releases the slot, and
emits the lifecycle event. `teardown_and_release_netns` is explicitly fallible
and deliberately retains the slot on failure. Thus this concrete sequence is
reachable:

1. the terminal observation write succeeds;
2. network teardown returns a non-benign error, or ownership is lost after the
   write and before cleanup completes;
3. dispatch returns an error or is retried after restart;
4. the exact-terminal fence returns `Ok(())` before the unfinished cleanup.

Boot reclamation cannot recover this residue because its write-time guard
refuses every already-terminal row. The result is not idempotent convergence;
it is a durable terminal claim with a permanently held slot/netns or missing
later effect. The new test runs two uninterrupted successful dispatches and
uses a provisioner whose teardown always succeeds, so it proves only adjacent
happy replay and cannot see the partial-completion window.

**Required remediation:** make the finalization operation resumable across
every fallible/crash boundary. The durable idempotency state must distinguish
"terminal claim recorded" from "all required cleanup completed," or the
ordering must otherwise guarantee that retry completes every outstanding
effect without duplicate externally visible work. Add a faulted first
teardown/process-loss partition followed by replay and assert eventual complete
cleanup, released ownership, one terminal transition, exact terminal data, and
no double-clean regression. Do not use the terminal row as a fence for work
that occurs after that row and may still be incomplete.

### D8 — `abort_for_test` retains the old live userspace dataplane

- **Severity:** Critical
- **Dimensions:** External validity, process-loss fidelity, concurrency
  ownership
- **Affected contracts:** S-GTI-06a, S-GTI-06b
- **Evidence:**
  - `crates/overdrive-control-plane/src/lib.rs:1228-1264`
  - `crates/overdrive-control-plane/src/lib.rs:1336-1395`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:702-741`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:822-924`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:811-820`

Aborting the axum/convergence/router/observer tasks is a real improvement over
graceful shutdown, but `AbruptServerResidue` deliberately retains a strong
worker `Arc`. Each allocation's blocking accept loops hold a `Weak` worker and
exit on owner loss; because the residue keeps the strong owner alive, those
loops continue accepting and enforcing on the old listeners. The fixture then
replaces the old worker's store-backed resolver with a frozen resolver, which
confirms that it is preserving an operational userspace dataplane, not inert
kernel/process residue.

A real `serve` process loss kills those threads, sockets, resolver, and
enforcement tasks even though independently owned workload processes and
kernel artifacts can survive. The code comment that `abort_for_test` revokes
"every in-process task" is therefore false. Both restart scenarios run boot
two while an impossible old control-plane worker remains alive, so their
reclamation boundary is not the required abrupt task-owner loss.

**Required remediation:** end every old control-plane worker/accept/enforce/
resolver task at the crash cut while preserving only the inert resources that
really survive process death. Any test-only residue owner must not be capable
of accepting or enforcing traffic. Re-run both 06 scenarios through that
boundary and retain assertion-safe cleanup without invoking graceful workload
stop.

### D9 — S-GTI-06a manually installs the peer intercept it claims production restored

- **Severity:** Critical
- **Dimensions:** Production-path accuracy, fixture truthfulness, test seam
  containment
- **Affected contract:** S-GTI-06a
- **Evidence:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:3-8`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4126-4162`
  - `crates/overdrive-control-plane/src/lib.rs:1280-1286`
  - `crates/overdrive-control-plane/src/lib.rs:1308-1333`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:267-273`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:1159-1164`

The peer is deployed as a production Service and has a durable intent and
Running allocation row in the same database that boot two reopens. It is
therefore not the separately owned, history-free peer fixture permitted by the
Iteration 1 remediation direction. After boot two starts, the test extracts
that peer's `AllocationSpec` from the old worker and calls
`retain_external_peer_for_test`, which directly invokes
`ensure_intercept_identity` and `MtlsInterceptWorker::start_alloc` on the new
server. If production has already installed an entry for that allocation, the
worker's keyed insert replaces/drops that production-owned entry.

This directly contradicts the test module's boundary declaration that no test
installs an intercept or resolver entry and that those effects come exclusively
from production boot/allocation. The successful D7 flow is consequently
conditional on a harness-authored peer dataplane. The same test also uses a
15-second guest delay as the only pre-crash coordination and never asserts the
target process is still live immediately before abort, adding avoidable
scheduling dependence to the ownership cut.

**Required remediation:** remove the peer re-homing API and all direct test
calls to identity/intercept installation. Either let boot two's real lifecycle
restore a durable peer or use a genuinely independent peer fixture outside the
restarted server's durable workload history. Use an explicit causal gate for
the target's pre-crash liveness rather than a fixed guest delay. The target's
first post-restart flow must succeed without any test-authored product effect.

### D10 — the illegal-event property preserves an already-reopened row

- **Severity:** Major
- **Dimensions:** Property strength, state-machine fidelity, mutation-killing
  power
- **Affected contract:** P-GTI-ILLEGAL-07
- **Evidence:**
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1633-1680`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1712-1730`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:665-688`

For the READY and EXEC partitions the property first changes the terminal row
itself to `Pending` or `Running` with `reason = Started`. It preserves the
terminal marker, invokes `reconcile`, asserts an empty action vector, and then
asserts the already-reopened `actual` row is unchanged. No READY/EXEC event or
writer boundary is applied and rejected. In the Running partition the normal
running-allocation branch was already action-free, so that partition does not
need the new fence to pass.

An operator-visible `Running + terminal Failed/Completed` hybrid is not proof
that reopening was prevented. The new reconciler guard can suppress a further
action while leaving precisely that invalid projection in place, and it does
not protect an already in-flight action or a late writer that clears the
terminal field.

**Required remediation:** drive the real transition/writer boundary, or a pure
canonical transition function used by it, from a byte-identical terminal row
through each late READY, EXEC, and duplicate-finalization input. Assert that the
event is rejected and the durable/current state remains the exact terminal
state. Platform Reclaimed must be the only generated reopening class. Do not
pre-apply the forbidden mutation and call its preservation success.

### D11 — C-GTI-RECLAMATION-ONCE is not a pure-function contract

- **Severity:** Major
- **Dimensions:** Contract Shape honesty, mechanical traceability, test
  architecture
- **Affected contract:** C-GTI-RECLAMATION-ONCE
- **Evidence:**
  - `crates/overdrive-control-plane/src/vm_reclamation_boot.rs:280-312`
  - `crates/overdrive-control-plane/src/vm_reclamation_boot.rs:314-355`
  - `crates/overdrive-control-plane/src/vm_reclamation_boot.rs:374-423`
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/roadmap.json:535-540`

The exact declaration was added, but the function creates a temp directory,
mutates `SimVmHostState`, writes intent and observation stores, executes the
async boot reclamation executor, reads the store again, and hydrates lifecycle
state through runtime adapters. That is a bounded stateful component example,
not a source-local return-only pure function. Adding the token does not satisfy
the mapped Contract Shape.

The newly added action assertions are useful: they observe one same-id
`RestartAllocation` and no second such action under the returned private View.
They should be retained in an honestly classified component test.

**Required remediation:** put the mapped pure property over the deterministic
planner/reconciler inputs and returned action/View trace with no store, host, or
executor effects. Keep boot/store/executor coverage as a separately named
bounded-change component example. Give each its truthful declaration and
oracles.

### D12 — the default production feature set fails the affected-package lint gate

- **Severity:** Major
- **Dimensions:** Build quality, feature-matrix coverage, production/test seam
  isolation
- **Evidence:**
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:267-273`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:411-420`
  - `crates/overdrive-control-plane/src/lib.rs:148-152`

The recorded all-target/all-feature clippy run enables the integration-only
read sites and therefore hides default-build defects. Independent default-
feature linting fails with `-D warnings`: `AllocIntercept.spec` is compiled into
the shipped worker but read only by the integration-test accessor. A normal
default compilation also reports the new unconditional `AllocationId`,
`WorkloadId`, and `AllocationSpec` imports as unused.

The same fixture changes the production hot path from an immutable resolver
`Arc` to an `RwLock<Arc<_>>` so a test can replace it, and stores a full
`AllocationSpec` on every live intercept solely for the abrupt-peer seam. These
are production behavior/data-layout changes made to support the invalid D9
fixture, not compiler-required fallout.

**Required remediation:** remove the peer seam and its production hot-path
changes, or compile every genuinely test-only field/import/read completely out
of the default build without weakening the real boundary. The default-feature
affected libraries and the all-feature matrix must both pass clippy with
`-D warnings`.

## Strong evidence retained after Iteration 2

- The target VM in both 06 tests is deployed once and boot two reopens the same
  data/config paths. The target rootfs/executable and intent are no longer
  rewritten, and the old direct `LocalObservationStore` mutation is gone.
- Both 06 tests observe the exact allocation id, one Platform Reclamation
  history entry, and `restart_count == 1`; 06b reaches the real INPUT-hook
  failure with no EXEC/frame and exact packet-path restoration.
- S-GTI-06a's post-cut target oracle still provides strong C3 identity,
  capture-before-release, exact D7 accounting, plaintext-at-guest,
  non-plaintext-at-peer, bidirectional TLS-record, and kTLS evidence. D9 limits
  what its end-to-end success proves; it does not make those target-side
  observers weak.
- S-GTI-12a and S-GTI-12b retain their direct stop outcomes and exact sibling
  complements.
- Immediate successful duplicate finalization now leaves the exact row
  timestamp/counter unchanged and observes one event, one selected-driver hook,
  zero fallback hooks, one mTLS-stop invocation, one network teardown, and an
  empty slot allocation. D7 is specifically the interrupted/failing-first-call
  hole.
- The production second resolver probe after frontend-address boot rebuild is
  an idempotent re-List over the existing single-owner watch and is justified by
  the restart ordering; no defect was found in that production fix itself.

## Iteration 2 scope and boundary audit

The remediation range changes 11 files with 1,258 insertions and 157 deletions;
the cumulative step range changes the same 11 files with 1,878 insertions and
68 deletions. The extra files beyond the original test-only scope are tightly
related to the attempted restart fixture and lifecycle/idempotency fixes, but
D7-D12 show that several expansions are not correct as landed.

| Boundary | Result |
|---|---|
| Built-product boundary | PASS — no Rust test spawns the built Overdrive production binary |
| Example/expectation separation | PASS — no example or expectation file changed; no expectation evidence is emitted |
| E08/E09 | PASS — neither path was introduced |
| Legacy/no-token | PASS — no legacy path was added |
| Service-plus-VM category | PASS — the target VM remains Job-kind; the peer is Exec-backed, though its fixture ownership is invalid under D9 |
| Mutation discipline | PASS — no mutation run or exclusion change occurred |
| OpenAPI/Cargo/schema diff | PASS — empty for both remediation and cumulative step ranges |
| 02-05 cleanup authority | **FAIL** — D7 can strand post-terminal-write cleanup permanently |

## Iteration 2 DES and commit chronology

The appended remediation events are chronologically ordered: RED at
`11:17:21Z`, GREEN at `11:20:57Z`, and COMMIT at `11:21:27Z`. The RED event
names all seven mapped contracts and honestly says which corrected tests failed
and which existing behaviors stayed green. D6 is therefore closed.

The COMMIT event names `2c110ce07b3721701ea2d8a97e016f2c3b9e3374`.
That object exists in reflog with the reviewed parent, subject, trailer, and
code/review tree. The only difference from final commit `6000b2e9` is the
seven-line appended COMMIT log event; the final amend occurred at `11:21:37Z`.
This is the expected recoverable log-bearing amend and is not a finding.

## Iteration 2 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 6691bb67..6000b2e9` | PASS |
| `execution-log.json` parse | PASS |
| Focused Lima P-GTI-ILLEGAL-07, C-GTI-RECLAMATION-ONCE, C-GTI-FINALIZE-TWICE, and retained-cleanup selection | PASS — 4/4; 832 skipped |
| Qualified native S-GTI-06a/06b/12a/12b selection | PASS — 4/4; 263 skipped; 86.893s; `/srv/vm/overdrive-testing/{kernel,rootfs.ext4}` |
| Canonical Lima DNS composition-root gate | PASS — 1/1; the execution log's host kTLS refusal did not reproduce on the canonical Linux lane |
| Default-feature affected-library clippy with `-D warnings` | **FAIL in scope** — new `AllocIntercept.spec` is dead code; default compilation also reports new test-only imports as unused |
| Canonical Lima `cargo openapi-check` | KNOWN PRE-EXISTING FAIL — `/v1/workloads/{id}/stop` live `workload_addr` versus checked-in `workload_id`; this step has no OpenAPI diff |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

The first Iteration 2 metal invocation intentionally demonstrated that the
native preflight rejects missing kernel/rootfs selections. The reported 4/4
result is the subsequent qualified run under one retained metal lease.

## Iteration 2 verdict

**NEEDS_REVISION.** Do not complete step 02-06 or advance the DELIVER wave.
Return D7-D12 to the original 02-06 crafter. The next iteration must make
finalization resumable after a post-write cleanup fault, end the old userspace
dataplane at the abrupt boundary, remove the manual durable-peer intercept
installation, reject rather than preserve late READY/EXEC reopening, restore
truthful Contract Shapes, and pass the default as well as all-feature lint
matrix. Continue remediation and re-review without an iteration cap until the
reviewer returns **APPROVED**.

---

## Iteration 3 metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Step | `02-06` |
| Reviewed commit | `66111cd2a4aa17c80372b0d89b93720bb1052d5c` |
| Parent | `6000b2e9e31faea2f57ba4ea63251d0cfc7e2e93` |
| Subject | `fix(mtls): make restart ownership crash-safe` |
| Trailer | `Step-Id: 02-06` |
| Review iteration | 3 |
| Verdict | **NEEDS_REVISION** |

## Iteration 3 summary

This remediation closes several Iteration 2 defects. The manual peer-rehoming
API and frozen resolver are gone; boot two now reconstructs the standing
peer's identity and intercept from the production boot path. Both qualified
restart examples prove the allocation processes are live immediately before
the abrupt cut. The terminal-event property now starts from an exact terminal
pre-state and calls the same pure transition function used by the action shim.
The reclamation-once evidence is truthfully split into a pure planner/lifecycle
property and a bounded-change boot/store example. Default-feature and
all-feature affected-package clippy both pass. Independent qualified native
execution remains green 4/4.

The crash-safety and ownership claims are still not true. Finalization records
driver-hook completion only in process memory, writes the terminal row before
the lifecycle event, and tests retries with the same in-process owners. A
process loss can therefore duplicate a hook or permanently skip the event.
`StopAllocation` retains the older, worse ordering: it writes the terminal row
before every terminal hook, mTLS stop, fallible network teardown, and event, so
a teardown error or crash strands cleanup behind the already-terminal fence.

The worker's task ownership is also incomplete. `stop_alloc` removes the
allocation from the only map, detaches its task set, and can leave in-flight
enforce/pass-through children beyond a later worker shutdown. Concurrent
`start_alloc` can spawn and record a new intercept after `shutdown_owner` has
drained its one map snapshot. Resolver supervision bypasses the new generic
primitive by adding shutdown to the public resolution domain port, while the
generic primitive itself lets a concurrent second `abort_and_join` caller
return before the first caller's tasks have joined.

Finally, production boot recovery silently skips a live Running allocation
when its adopted slot, intent row, or reconstructable allocation spec is
missing. Boot has already swept the old intercept rules at that point. Such a
surviving process can therefore remain live without a replacement intercept;
the absence must fail closed, not be treated as a stale-row no-op. The latest
DES RED is also a wrong-reason compile failure rather than behavioral RED
evidence for these remediation changes.

## Iteration 3 remediation disposition

| Prior finding | Disposition | Evidence |
|---|---|---|
| D1 — graceful shutdown mislabeled unclean | **CLOSED for the two mapped scenarios** | `abort_for_test` aborts/joins the server tasks and explicitly invokes worker-owner shutdown; both native examples assert live cgroup PIDs immediately before the cut. D15 remains a separate general owner-race defect. |
| D2 — target/workload substitution | **CLOSED** | The target and peer intents, executables, rows, and allocation ids remain unchanged; the direct peer re-home seam is removed. |
| D3 — duplicate finalization effects | **OPEN / still blocking** | Happy and same-process fault replay improved, but process loss still duplicates/skips effects and Stop remains fence-first (D13, D14). |
| D4 — vacuous illegal/reclamation contracts | **CLOSED** | D10 and D11 now drive truthful pure pre-state/transition and pure planner/lifecycle deltas, with separate bounded-change executor coverage. |
| D5 — names and Contract Shapes | **CLOSED** | All seven mapped function names and declarations match the approved executable map. |
| D6 — incomplete/wrong RED | **REOPENED** | The latest RED records an accidental vocabulary/import compilation failure, not a fail-for-right-reason assertion for D7-D12 (D18). |
| D7 — partial cleanup fenced as complete | **OPEN / still blocking** | Commit-last fixes the first teardown-error case but not process-loss exactness, event delivery, or the untouched Stop boundary (D13, D14). |
| D8 — old userspace dataplane survives | **PARTIAL / still blocking** | Active mapped intercepts are shut down in the tested cut, but detached stopped-allocation children and late intercept registration escape the one-shot drain (D15, D16). |
| D9 — test-authored peer intercept | **PARTIAL / still blocking** | The test seam is removed and the happy peer is production-recovered. Missing components of the live row/intent/slot/PID join fail open (D17). |
| D10 — illegal property preserves reopened row | **CLOSED** | The property begins from a byte-identical terminal row and proves zero delta for READY, EXEC, and Finalize through the canonical production preflight. |
| D11 — false pure-function declaration | **CLOSED** | The mapped function is now synchronous and effect-free; the host/store/executor scenario has a separate bounded-change name and declaration. |
| D12 — default-feature lint failure | **CLOSED** | Independent default and all-feature affected-package clippy runs pass with `-D warnings`. |

## Iteration 3 criterion disposition

| Criterion | Result | Evidence |
|---|---|---|
| S-GTI-06a | **PASS for the mapped native trace; production fail-closed gap remains** | Qualified metal proves same data/id, live pre-cut target and peer processes, Platform Reclamation, production peer recovery, exact D7 first-flow accounting, natural exit, TLS/kTLS, and no manual intercept seam. D17 covers an untested recovery-join error partition. |
| S-GTI-06b | PASS for the mapped native trace | Qualified metal reaches same-id reclamation, real INPUT-hook rejection, no EXEC/frame, terminal Failed, and exact restoration after ending active old worker ownership. |
| S-GTI-12a | **PARTIAL** | The happy target-handle deletion and ordered sibling complement pass. The production Stop arm can publish terminal before a failing teardown, so removal is not crash/error-total (D14). |
| S-GTI-12b | **FAIL** | The mapped Stopped/AlreadyStopped and absent-guard complement pass, but the terminal-first Stop boundary can permanently suppress the cleanup that the first stop failed to complete (D14). |
| P-GTI-ILLEGAL-07 | PASS | Exact terminal pre-state plus canonical READY/EXEC/Finalize `NoChange`, and Platform Reclaimed remains `Apply`. |
| C-GTI-RECLAMATION-ONCE | PASS | Pure planner/lifecycle trace shows one claim and one same-id redrive; bounded-change boot coverage persists the claim and observes no repeat. |
| C-GTI-FINALIZE-TWICE | **FAIL** | Same-process happy/fault retry is strong, but a process cut duplicates the process-local hook and a post-row cut skips the lifecycle event (D13). |

## Iteration 3 findings

### D13 — finalization is not crash-exact and can either duplicate a hook or skip the terminal event

- **Severity:** Critical
- **Dimensions:** Crash consistency, exactly-once effects, lifecycle event
  completeness, test external validity
- **Affected contracts:** C-GTI-FINALIZE-TWICE, S-GTI-12b, retained 02-05
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:765-772`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1751-1773`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:652-763`

`terminal_hooks_completed` is an in-memory `BTreeSet`. It is claimed before
the hook call and is lost with the process. If the process dies after
`on_alloc_terminal` but before the terminal observation write, boot sees the
non-terminal row and a fresh index; a level-triggered retry calls the hook
again. The worker map and slot map are also process-local, so their absence is
not a durable record proving exactly which effects completed.

The opposite cut is equally observable. The terminal row is written at line
1772 and the broadcast event is emitted at line 1773. A process loss between
those statements leaves the terminal row as the replay fence, so no later
dispatch emits the missing event. The comment that there is no effect after
the write is therefore false.

The new test injects teardown and observation-write faults, then invokes the
action four times against the same `AllocDriverIndex`, worker, slot allocator,
and receiver. It does not replace the process owner or cut between the row and
event. Its one-hook/one-event result cannot establish crash exactness.

**Required remediation:** make the complete terminal operation recoverable at
the owner boundary, including terminal event delivery. Use a durable/atomic
completion mechanism or an existing durable idempotency identity from which
hooks and events can be replayed without duplicates. Add process-replacement
cuts after each effect and after the row write, and assert one durable
transition, one hook, one mTLS stop, one network cleanup, and one lifecycle
event in every partition while preserving frozen schemas.

### D14 — `StopAllocation` still publishes terminal before fallible cleanup

- **Severity:** Critical
- **Dimensions:** Cleanup ownership, error recovery, stop idempotency, retained
  02-05 guarantees
- **Affected contracts:** S-GTI-12a, S-GTI-12b, retained 02-05
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2800-2860`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:796-801`

The Stop arm writes `Terminated` first. Only afterwards does it call terminal
hooks, release supervision, remove the driver index, stop mTLS, and call the
fallible network teardown. If teardown fails, dispatch returns an error with
the row already terminal. Reconciliation and the second operator stop now see
the completed stop and do not retain an outstanding cleanup token. A crash
after the row write can skip even more of the sequence, including the event.

This is the same stranded-cleanup shape D7 identified, still live on the path
that S-GTI-12a/b directly own. Their fixtures use successful/already-absent
cleanup, so the defect stays invisible.

**Required remediation:** apply one crash-safe terminal protocol consistently
to Stop and Finalize. A failed post-stop teardown must retain a retryable owner,
and replay must converge every missing effect exactly once before the durable
stop is considered complete. Add first-teardown-fails and process-cut
partitions to the real Stop action boundary.

### D15 — allocation stop detaches children that later owner shutdown cannot join

- **Severity:** Critical
- **Dimensions:** Task ownership, old-owner liveness, connection cleanup
- **Affected contracts:** D8 owner-loss remediation, S-GTI-06a/06b ownership
  premise
- **Evidence:**
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:746-803`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:816-851`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:989-1103`

`stop_alloc` removes the intercept from `self.intercepts`, calls
`OwnedTaskSet::detach`, and drops every handle. The accept loops have a
cooperative stop flag, but in-flight enforce and cleartext pass-through tasks
do not. A later `shutdown_owner` can drain only entries still present in
`self.intercepts`; it has no handle with which to abort or join children of an
already-stopped allocation. The teardown task spawned at lines 782-795 is
detached as well.

The owner-shutdown test records two idle accept loops. It does not put an
enforce or pass-through child in flight, call `stop_alloc`, and then prove a
later owner shutdown joins that detached child.

**Required remediation:** retain worker-level supervision for every child even
when allocation-level ownership transitions to cooperative detach. Prove idle
accept, blocked enforce, live pass-through, late child, connection teardown,
and resolver children all end before owner shutdown returns.

### D16 — worker shutdown and `OwnedTaskSet` do not provide an atomic reusable completion fence

- **Severity:** Major
- **Dimensions:** Concurrency ownership, reusable API correctness, dependency
  direction, public-surface discipline
- **Evidence:**
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:701-743`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:823-851`
  - `crates/overdrive-core/src/task_ownership.rs:13-24`
  - `crates/overdrive-core/src/task_ownership.rs:51-113`
  - `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:619-623`
  - `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:861-866`
  - `crates/overdrive-core/src/traits/mtls_resolve.rs:243-249`

`shutdown_owner` takes one snapshot of `self.intercepts`. `start_alloc` creates
two tasks before `record_intercept_full` inserts that allocation. A concurrent
start can therefore record an intercept after the shutdown snapshot and leave
its listeners/tasks owned by the old worker.

At the generic layer, two concurrent `abort_and_join` calls are not joined to
one completion. The first caller takes the handles and awaits them; the second
caller sees an empty vector and returns immediately even though those tasks
are still running. That contradicts a reusable completion-fence API.

The core placement itself is domain-neutral and adds no Cargo dependency, and
`dst-lint` passes. However the resolver watch does not use `OwnedTaskSet`; it
retains a separate `Mutex<Option<JoinHandle>>`, and task-owner shutdown is
added to the public `MtlsResolve` classification port. That places operational
ownership on a domain resolution API instead of keeping the domain wiring in
the worker/control-plane composition outside core.

**Required remediation:** seal worker allocation registration atomically with
owner shutdown, make every concurrent shutdown caller await the same completed
state, and compose resolver ownership through the generic primitive without
expanding the resolution port with an unrelated lifecycle method. Add races
for start-during-shutdown and concurrent shutdown callers.

### D17 — production live-intercept recovery fails open on an incomplete durable join

- **Severity:** Critical
- **Dimensions:** Security fail-closed behavior, production boot lifecycle,
  recovery completeness
- **Affected contract:** S-GTI-06a production recovery boundary
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:117-200`
  - `crates/overdrive-control-plane/src/lib.rs:2823-2855`
  - `crates/overdrive-control-plane/src/lib.rs:2902-2915`

The happy production join is correct: durable Running row, live cgroup PID,
adopted slot, immutable intent, exact recovered address, audited identity, and
normal worker installation. But after the old nft rules are swept, the helper
silently `continue`s when a live Running allocation has no adopted slot, no
intent bytes, or an intent that does not produce an allocation spec. It then
returns `Ok`, allowing boot to continue without an intercept for that live
process. A Running row with standing intent is not automatically re-driven by
ordinary lifecycle convergence, so the missing-slot case can persist.

The native test covers only the complete happy join. There is no production
boot test for each missing component of the row/intent/slot/live-PID matrix.

**Required remediation:** once a Running row has a live cgroup PID, absence or
mismatch of every other required recovery component must refuse boot or
terminally contain the process before serving. Add all incomplete-join
partitions and prove no listener opens and no cleartext-capable survivor
remains.

### D18 — the remediation RED failed for the wrong reason

- **Severity:** Major
- **Dimensions:** TDD phase honesty, regression evidence
- **Evidence:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/execution-log.json:789-800`

The latest RED explicitly records that the remediation did not compile because
the test used the wrong Platform Reclamation vocabulary and had unused imports.
That is not a semantically correct failing assertion for finalization crash
exactness, task-owner completion, production peer recovery, or Contract Shape
splitting. GREEN then claims all D7-D12 behaviors. The earlier seven-contract
RED remains valid historical evidence for the prior remediation, but it cannot
serve as RED for newly added ownership and process-cut behavior.

**Required remediation:** preserve the existing log and append a fresh
fail-for-right-reason RED for every corrected blocking behavior, followed by
GREEN and COMMIT. Compilation failure qualifies only when the missing
production symbol/signature is the intended test requirement, not when the
test itself uses wrong vocabulary.

## Strong evidence retained after Iteration 3

- Both 06 tests use one immutable target image/intent/data directory and assert
  live cgroup PIDs immediately before abrupt owner loss.
- The manual peer spec/identity/intercept seam and frozen store-free resolver
  are removed. The successful native trace recovers the peer through the boot
  composition path and preserves its allocation id and restart count.
- The active worker shutdown path drains mapped intercepts, aborts/joins their
  accept/enforce/pass-through task sets, closes listeners, drops rule guards,
  drains enforced handles, and joins the resolver watch. D15-D16 identify the
  detached and concurrent-registration complements, not a failure of this
  active happy path.
- The P-GTI-ILLEGAL-07 test is now a genuine pure transition property over an
  exact terminal pre-state. The action shim calls the same function before
  Start, Restart, and Finalize mutation.
- C-GTI-RECLAMATION-ONCE now has an honest pure-function declaration, and the
  effectful boot/store/executor example is separately bounded-change.
- Both lint feature matrices pass. No production `AllocationSpec` storage or
  mutable resolver hot-path seam remains.
- No REST/OpenAPI, Beacon, persisted/rkyv/observation schema, Cargo manifest,
  example, expectation, E08/E09, legacy/no-token, built-process, or mutation
  scope is present in either the remediation or cumulative step diff.

## Iteration 3 scope and boundary audit

The Iteration 3 remediation changes 15 files with 1,529 insertions and 407
deletions; the cumulative step changes the same 15 files with 3,091 insertions
and 159 deletions. `overdrive-core::task_ownership` is the only created source
file. No crate or Cargo dependency was added.

| Boundary | Result |
|---|---|
| Built-product boundary | PASS — the Rust integration tests do not spawn the built Overdrive binary |
| Example/expectation separation | PASS — no example or expectation file changed or was invoked as the system under test |
| E08/E09 | PASS — neither path was introduced |
| Legacy/no-token | PASS — no legacy path was added |
| Service-plus-VM category | PASS — the target remains a VM Job and the independent peer is an Exec Service |
| Frozen wire/persistence shapes | PASS — no Beacon, REST/OpenAPI, describe, rkyv, or observation-schema diff |
| Rust ownership API | **FAIL architecture** — D16 adds lifecycle shutdown to the public resolution port and does not compose resolver ownership through the generic primitive |
| 02-05 cleanup authority | **FAIL** — D13/D14 still permit skipped, duplicate, or permanently stranded terminal cleanup |
| Mutation discipline | PASS — no mutation run or exclusion change |

## Native-substrate and `LOCAL_BACKEND_MAP` investigation

The canonical native runner's fail-closed preflight is effective. My first
invocation omitted `OVERDRIVE_METAL_KERNEL`/`OVERDRIVE_METAL_ROOTFS` and was
refused before test execution with `selected guest kernel is required`. The
qualified rerun selected `/srv/vm/overdrive-testing/{kernel,rootfs.ext4}`,
acquired the global lease, passed the native non-virtualized x86_64/KVM
preflight, and ran the four scenarios. No Lima or nested-KVM result is used as
runtime evidence.

Stale “nested KVM” wording still exists in E06 and several crate comments, but
each checked path is byte-identical to baseline `6691bb67` and outside both the
Iteration 3 and cumulative 02-06 diff. The authoritative feature/DISTILL and
runner surfaces reject nested or virtualized evidence. This is baseline
documentation debt, not a 02-06 regression.

Neither the approved 02-06 executable map nor its D7/stop/reclamation scenarios
claim `LOCAL_BACKEND_MAP`; they exercise the VM nft-TPROXY/frontend path. The
cumulative step diff contains no `LOCAL_BACKEND_MAP` change. The allegedly
missing map claim is therefore unrelated to this step rather than omitted
acceptance evidence.

## Iteration 3 DES and commit chronology

The remediation events are ordered: RED `12:15:49Z`, GREEN `12:55:55Z`, and
COMMIT `12:58:59Z`. The COMMIT event does not record a hash. Reflog nevertheless
provides the recoverable chronology: initial commit
`68114f6e9a231c78c5caa8df3501e8ddb7a93977` at `12:56:14Z`, then a seven-line
execution-log-only amend to reviewed commit `66111cd2` at `12:59:15Z`. Parent,
tree except for the appended event, subject, and trailer match. The missing
hash is less precise but the final commit exists and the amend is honest; it is
not a separate finding. D18 concerns the RED reason, not commit existence.

## Iteration 3 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 6691bb67..66111cd2` | PASS |
| `execution-log.json` parse | PASS |
| `cargo xtask dst-lint` | PASS |
| Focused Lima D7/D8/D10/D11 selection | PASS — 8/8; 1,883 skipped |
| Default-feature affected-package clippy with `-D warnings` | PASS for core, worker, control-plane, reconcilers, and CLI |
| All-feature affected-package clippy with `-D warnings` | PASS for the same five packages |
| Qualified native S-GTI-06a/06b/12a/12b selection | PASS — 4/4; 162 skipped; 100.174s; native non-virtualized x86_64 KVM; selected kernel/rootfs under `/srv/vm/overdrive-testing` |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

The focused suites validate their implemented happy/fault paths. They do not
invalidate D13-D17, whose missing process cuts, detached-child partitions,
registration races, and incomplete boot joins are absent from the selection.

## Iteration 3 verdict

**NEEDS_REVISION.** Do not complete step 02-06 or advance the DELIVER wave.
Return D13-D18 to the original 02-06 crafter. The next iteration must make Stop
and Finalize one crash-exact terminal protocol including event delivery, retain
worker-level ownership of cooperatively detached children, seal late intercept
registration, make concurrent task-owner shutdown a real completion fence,
keep resolver ownership out of the resolution domain port, and fail closed on
every incomplete live-intercept recovery join. Append a genuine behavioral
RED→GREEN→COMMIT cycle and continue the uncapped review loop until the reviewer
returns **APPROVED**.

---

## Iteration 4 metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Step | `02-06` |
| Reviewed commit | `3c563e28b7b3c05ddf3da00c481c8c9ab69f0373` |
| Parent | `66111cd2a4aa17c80372b0d89b93720bb1052d5c` |
| Subject | `fix(mtls): make terminal ownership crash exact` |
| Trailer | `Step-Id: 02-06` |
| Review iteration | 4 |
| Verdict | **NEEDS_REVISION** |

## Iteration 4 summary

The ownership remediation is materially better. `OwnedTaskSet::spawn` now
makes spawn-and-register one locked operation; ordinary concurrent
`abort_and_join` callers share a watch-backed completion notification; the
worker holds one process-level task set after allocation bookkeeping is
removed; late installs are excluded by a worker lifecycle gate; stopped
allocation enforce, pass-through, and teardown children remain supervised; and
the concrete resolver watch uses the core owner without adding runtime shutdown
to `MtlsResolve`. The focused canonical Lima selection passes 13/13, both
default and all-feature affected-package clippy matrices pass, and the four
qualified native scenarios pass again on the canonical non-virtualized metal
host.

The terminal protocol still does not meet the crash-exact contract. Its
completion records for driver hooks and lifecycle events are process-local
sets. A replacement process therefore cannot distinguish a cut before an
effect from a cut after it. The implementation resolves that ambiguity by
never replaying a driver hook without a fresh in-memory driver index and by
re-emitting a terminal event once per replacement process. The former skips a
hook when the cut was before it; the latter duplicates an event when the cut
was after it. The event also remains an effect after the purported commit-last
row. No process replacement test cuts after the event.

The mTLS effect is not terminal-last either. `stop_alloc` removes the intercept
and spawns async connection teardown, converting teardown errors to warnings;
both Finalize and Stop can write the durable terminal row while that teardown
is unfinished. `StopAllocation` additionally continues to absorb ordinary
`Driver::stop` errors, then removes mTLS/network protection and publishes
`Terminated`. That can leave a still-live process behind a terminal row with no
traffic guard.

Boot recovery now validates missing slot, intent, spec, and address for each
live Running row before the nft sweep, which closes the literal D17 partitions.
It nevertheless sweeps every surviving fail-closed rule before frontend
rebuild, resolver refresh, identity issuance, or replacement worker install.
Any failure in those later operations refuses the server but leaves the
surviving Running/EXEC process alive after its old rule was removed. The new
test calls the plan/apply helper directly and cannot observe that production
sweep-to-install window.

Finally, the generic task fence is not cancellation-safe and the worker does
not share one fence for its full shutdown. Cancelling the elected
`OwnedTaskSet` leader leaves the state permanently `ShuttingDown`; later callers
can never take the already-aborted handles or publish completion. Two worker
shutdown callers can also diverge: the first owns the drained enforcement
handles, while the second can return as soon as the task set completes, before
the first caller's awaited enforcement teardown has finished.

## Iteration 4 remediation disposition

| Prior finding | Disposition | Evidence |
|---|---|---|
| D1 — graceful shutdown mislabeled unclean | **CLOSED** | Both mapped restart scenarios retain the abrupt server/worker-owner cut and live pre-cut PID assertions; qualified native remains green. |
| D2 — target/workload substitution | **CLOSED** | The immutable same-data target/peer route remains unchanged. |
| D3 — duplicate finalization effects | **OPEN / still blocking** | The process-local hook/event witnesses cannot make cuts crash-exact, and mTLS teardown remains asynchronous (D19, D20). |
| D4 — vacuous illegal/reclamation contracts | **CLOSED** | The truthful pure transition/planner contracts remain intact and pass in the focused selection. |
| D5 — names and Contract Shapes | **CLOSED** | All seven executable-map names and declarations remain exact. |
| D6 — incomplete/wrong RED | **CLOSED** | The new RED is a genuine compiled behavior RED for five D13-D18 partitions. It does not prove the still-missing cuts in D19-D22, but it closes the specific wrong-reason compilation defect D18 identified. |
| D7 — partial cleanup fenced as complete | **OPEN / still blocking** | Network teardown is commit-first corrected, but async/log-only mTLS teardown, swallowed stop errors, and event-after-row remain outside the fence (D19, D20). |
| D8 — old userspace dataplane survives | **PARTIAL / still blocking** | Stopped-allocation children are now retained and late starts rejected, but cancellation and concurrent full-worker completion are not fenced (D22). |
| D9 — test-authored peer intercept | **PARTIAL / still blocking** | The test-authored seam remains gone and the four literal join faults refuse, but post-sweep recovery failures expose survivors (D21). |
| D10 — illegal property preserves reopened row | **CLOSED** | Exact terminal pre-state and READY/EXEC/Finalize `NoChange` remain unchanged. |
| D11 — false pure-function declaration | **CLOSED** | The mapped reclamation function remains synchronous/effect-free and carries the exact declaration. |
| D12 — default production lint failure | **CLOSED** | Both independent affected-package feature matrices pass with `-D warnings`. |
| D13 — Finalize crash exactness/event delivery | **OPEN / still blocking** | D19 shows process-local hook/event claims still skip or duplicate effects across cuts. |
| D14 — Stop terminal-first cleanup | **PARTIAL / still blocking** | The row moved after network teardown, but it still precedes completion of async mTLS teardown, and ordinary driver stop errors are swallowed (D20). |
| D15 — stopped allocation children detach | **CLOSED for tracked child classes** | One worker-level owner retains accept/enforce/pass-through/teardown children after map removal; focused enforce/pass-through tests pass. |
| D16 — atomic/reusable owner fence and resolver port | **PARTIAL / still blocking** | Normal concurrent calls, late registration, core placement, and resolver dependency direction are fixed. Cancellation and the worker's post-task teardown phase still bypass one shared completion fence (D22). |
| D17 — incomplete live recovery join | **PARTIAL / still blocking** | Missing slot/intent/spec/address preflight now refuses before sweep; later production boot/apply failures occur after the sweep and leave live survivors unguarded (D21). |
| D18 — wrong-reason RED | **CLOSED** | The `13:29:56Z` RED names compiled, driven failures for task concurrency, detached enforce ownership, Stop cleanup ordering, hook process replacement, and missing-slot recovery. |

## Iteration 4 criterion disposition

| Criterion | Result | Evidence |
|---|---|---|
| S-GTI-06a | **PASS for the mapped native happy trace; production recovery still has a fail-closed hole** | Qualified metal again proves the immutable same-id reclamation and protected first flow. D21 covers failure after the global sweep, which this happy trace does not inject. |
| S-GTI-06b | PASS for the mapped native trace | Same-id reinstall refusal, no EXEC/frame, final failure, and exact restoration remain green. |
| S-GTI-12a | **PARTIAL** | Exact target-handle and sibling complement remain green; the action can still terminalize before async mTLS teardown completes or after a swallowed driver stop error (D20). |
| S-GTI-12b | **FAIL** | `Stopped` then `AlreadyStopped` and the absent-guard complement pass, but the complete no-duplicate/no-stranded cleanup claim is contradicted by D19-D20. |
| P-GTI-ILLEGAL-07 | PASS | The pure canonical transition remains exact and green. |
| C-GTI-RECLAMATION-ONCE | PASS | The pure planner/lifecycle action trace remains one claim and one same-id redrive. |
| C-GTI-FINALIZE-TWICE | **FAIL** | Adjacent same-owner replay and one selected replacement cut pass; pre/post event, pre-hook replacement, and async mTLS teardown cuts remain non-exact (D19-D20). |

## Iteration 4 findings

### D19 — the terminal row cannot distinguish pre/post hook and event cuts

- **Severity:** Critical
- **Dimensions:** Crash consistency, exactly-once effects, durable replay,
  lifecycle event completeness
- **Affected contracts:** C-GTI-FINALIZE-TWICE, S-GTI-12b, retained 02-05
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:776-829`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1611-1623`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1824-1856`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2781-2789`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:714-779`

`terminal_hooks_completed` and `lifecycle_events_emitted` are both fields of
the process-local `AllocDriverIndex`. The terminal row records neither effect.
Consequently these two histories have the same durable state at replacement:

1. cleanup completes, the process dies immediately before
   `on_alloc_terminal`;
2. cleanup and `on_alloc_terminal` complete, the process dies immediately
   after the hook but before the row write.

The replacement index is empty in both cases. `active_driver_for_alloc` returns
`None`, so case 1 permanently skips the hook in order not to duplicate case 2.
The test covers only case 2: it resets `AllocDriverIndex` after the assertion
that the old hook count is already one. It never cuts before the hook.

Events have the inverse failure. The terminal row is written at line 1848 and
the event is emitted afterwards at lines 1850-1856. A replacement replay of
that row inserts its timestamp into a fresh process-local set and emits. This
repairs the row-before-event cut only by duplicating the event for an
event-before-crash cut. Repeating another process replacement can emit it
again. The Stop replay also rebuilds the event with `prior_state` equal to the
already-`Terminated` row, not the original pre-stop state, so it is not even a
byte-equivalent replay of the first event.

The test performs its completed replay with the same reset index that emitted
the first event; it does not replace the owner after event emission. A
timestamp in an in-memory set is not a durable outbox or acknowledgment.

**Required remediation:** introduce one crash-recoverable effect protocol that
can distinguish every pre/post hook and event cut under the frozen public
schemas. Prove replacement-process cuts immediately before and after each
effect, including repeated replacement after a delivered event. Every
partition must converge one terminal transition, one exact lifecycle event,
and the required terminal hook exactly once; it cannot choose skipped hooks to
avoid duplicates or at-least-once event delivery to avoid skips.

### D20 — Stop and Finalize can publish terminal while authoritative cleanup is unfinished or failed

- **Severity:** Critical
- **Dimensions:** Cleanup authority, terminal-last ordering, live-process
  containment, error propagation
- **Affected contracts:** S-GTI-12a, S-GTI-12b, C-GTI-FINALIZE-TWICE, retained
  02-05
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1824-1856`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2794-2830`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2896-2947`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:782-833`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:853-930`

`MtlsInterceptWorker::stop_alloc` is synchronous only at the map/rule removal
surface. It drains active enforced handles into a newly spawned task and
returns. That task awaits `enforcement.teardown`; every error is reduced to a
warning. The action shim then proceeds to the durable row write without
awaiting that task or observing its result. Both terminal arms can therefore
publish completion while a live enforced connection is still tearing down,
and an ordinary non-crash teardown failure is permanently hidden behind the
terminal row.

The Stop driver boundary is similarly non-authoritative. Lines 2809-2829
special-case duplicate ownership and retained start-cleanup dispositions, but
silently absorb every other `DriverError`, including `Io`. The code then
removes the intercept/network and writes `Terminated`. `NotFound` may be a
legitimate idempotent result; an arbitrary I/O failure is not proof that the
process stopped. The reachable outcome is a live Exec/VM survivor whose
durable row says Terminated after its mTLS/netns protection was removed.

The new Stop test injects only a network-provisioner failure. Its scripted
driver always returns `Ok`, no mTLS worker is composed, it never replays the
successful second attempt, and it has no connection-teardown error surface.

**Required remediation:** make every authoritative stop and mTLS teardown
result part of the awaited terminal protocol. Classify only explicit benign
absence as idempotent. Any other driver or enforcement cleanup failure must
retain a durable/reconstructable retry owner or contain the still-live process
before protection is removed; it must not author the terminal row. Add Stop
and Finalize tests for in-flight teardown, teardown error, arbitrary driver I/O
error, process replacement at those boundaries, and eventual exact cleanup.

### D21 — recovery removes the last fail-closed rule before all fallible replacement work

- **Severity:** Critical
- **Dimensions:** Security fail-closed behavior, boot ordering, recovery
  atomicity, production-path accuracy
- **Affected contract:** S-GTI-06a production recovery boundary
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:120-219`
  - `crates/overdrive-control-plane/src/lib.rs:2837-2886`
  - `crates/overdrive-control-plane/src/lib.rs:2888-2949`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:3738-3892`

The new planner correctly refuses missing slot, intent, reconstructable spec,
or exact address for a live Running row. Production then globally sweeps the
surviving rules. Only after that destructive sweep does boot execute the
fallible frontend rebuild, resolver refresh, SVID issuance, and per-allocation
`worker.start_alloc` calls. Each has an error return after the old rule is gone.

Refusing to open the API listener is not enough: independently owned workload
processes deliberately survive the control-plane process, and no error arm in
this range stops, pauses, or re-isolates them. If frontend/relist/CA/rule install
fails, the process remains Running/EXEC-capable without either the old
fail-closed redirect or a replacement intercept. With multiple plans, a later
install failure also leaves earlier siblings partially installed and the
failing/later siblings exposed.

The new test invokes `recover_live_mtls_intercepts`, which plans then applies
without the production nft sweep. It asserts only that no valid sibling was
installed before an incomplete *data join* returned. It cannot falsify any
failure between production sweep and replacement install.

**Required remediation:** keep every surviving process fail-closed through the
entire fallible boot sequence. The production test must drive the actual
adopt/preflight/sweep/rebuild/resolve/identity/install boundary and inject every
post-sweep failure, including a later sibling install. Before returning an
error it must prove no surviving process has an unredirected cleartext path;
successful boot must prove replacement protection is established before the
old fail-closed ownership is released or through an equivalent atomic/paused
handoff.

### D22 — task and worker shutdown completion can be orphaned or observed early

- **Severity:** Major
- **Dimensions:** Cancellation safety, reusable ownership, concurrent
  completion, task leak prevention
- **Evidence:**
  - `crates/overdrive-core/src/task_ownership.rs:96-129`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:846-880`
  - `crates/overdrive-core/src/task_ownership.rs:207-268`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:2303-2421`

`OwnedTaskSet::abort_and_join` moves every handle into the elected caller and
sets `ShuttingDown` before its first await. If that future is cancelled while
joining, the local handles are dropped and the lifecycle never becomes
`Shutdown`; the watch value is never sent. All future callers see
`ShuttingDown` and wait forever. This is especially observable for an already
running `spawn_blocking` child, because abort does not stop it and no future
owner can join it.

At the worker layer, the shared task fence does not cover the whole method.
The first `shutdown_owner` caller takes all active `EnforcedConnection`
handles, awaits the task set, and only then awaits each enforcement teardown.
A concurrent second caller takes an empty intercept map, awaits the same task
set, and can return while the first is still blocked in teardown. Thus the
method's advertised owner-complete postcondition depends on which concurrent
caller returns.

The new tests cover one uninterrupted leader plus one waiter, late registration,
and stopped-allocation enforce/pass-through children. They do not cancel the
leader or gate an active enforcement teardown across two worker shutdown
callers.

**Required remediation:** make the generic shutdown transition
cancellation-safe so some durable in-memory leader/guard owns the handles until
completion and every later caller can finish or observe that completion. Put
the worker's intercept drain, task join, and enforcement teardown behind one
shared completion state. Add leader-cancellation with a running blocking child
and two concurrent worker shutdown callers with a gated teardown; neither
caller may return early and no task/handle may be left unjoinable.

## Strong evidence retained after Iteration 4

- The core primitive remains in dependency-neutral `overdrive-core`; no Cargo
  dependency or public persistence/wire shape was added.
- `spawn` rejects late work before invoking its closure, so the ordinary
  spawn/register versus shutdown race no longer produces an unowned child.
- One worker-level task set retains stopped-allocation accept, enforce,
  pass-through, and async teardown tasks after their `AllocIntercept` entry is
  removed. The focused enforce and pass-through complements pass.
- The worker lifecycle write gate excludes a completed/late `start_alloc`, and
  resolver task ownership is exposed only on the concrete adapter; the
  lifecycle method has been removed from `MtlsResolve`.
- Missing slot, missing intent, non-reconstructable spec, and exact recovered
  address mismatch now refuse during the pre-sweep planning pass, and planning
  all rows before mutation prevents a data-join failure from partially applying
  a valid sibling.
- All seven mapped executable names and Contract Shape declarations are exact.
  The pure properties use the required exact rustdoc line
  `/// CONTRACT_SHAPE: pure-function.`.
- The cumulative step still introduces no built-product invocation from Rust
  integration tests, expectation/example leakage, E08/E09, legacy/no-token
  path, Service-plus-VM category, schema/OpenAPI/Cargo diff, or per-step
  mutation run.

## Iteration 4 scope and boundary audit

The Iteration 4 remediation changes ten files with 1,338 insertions and 390
deletions; 368 inserted lines are the reviewer-authored Iteration 3 artifact.
The cumulative step changes fifteen files. The extra production files are
tightly related to the terminal, owner, resolver, and recovery findings, but
D19-D22 show the implementation is not complete.

| Boundary | Result |
|---|---|
| Built-product boundary | PASS — no Rust test spawns the built Overdrive production binary |
| Example/expectation separation | PASS — no example or expectation changed or ran as the system under test |
| E08/E09 | PASS — neither path was introduced |
| Legacy/no-token | PASS — no legacy path was added |
| Service-plus-VM | PASS — target is still a VM Job and peer is an independent Exec Service |
| Frozen wire/persistence/API shapes | PASS — no Beacon, REST/OpenAPI, rkyv, observation-schema, or Cargo diff |
| Core ownership placement | PASS — `OwnedTaskSet` remains dependency-neutral and resolver ownership is outside `MtlsResolve` |
| Terminal cleanup authority | **FAIL** — D19-D20 permit skipped/duplicate events/hooks and terminal-before-mTLS/error completion |
| Boot fail-closed recovery | **FAIL** — D21 exposes live survivors after the global sweep |
| Mutation discipline | PASS — correctly reserved for the final DELIVER gate |

## Iteration 4 DES and commit chronology

The appended events are ordered: behavioral RED at `13:29:56Z`, GREEN at
`14:05:58Z`, and COMMIT at `14:07:52Z`. The RED is genuine: it reports compiled
behavior failures for normal concurrent task shutdown, stopped-allocation
enforce ownership, Stop network ordering, one hook replacement cut, and the
missing-slot join. D18 is closed.

The COMMIT event does not name a hash. Reflog provides the recoverable
chronology: initial remediation commit `6870d546` at `16:06:11+02:00`, followed
by the execution-log-only amend to reviewed commit `3c563e28` at
`16:08:01+02:00`. Subject, parent, trailer, and implementation tree chronology
are consistent. This is not a finding. D19-D22 concern uncovered behavior, not
phase ordering or commit existence.

## Broad-gate investigation

The canonical privileged Lima broad run executed 1,879 tests: 1,876 passed and
three failed.

- OpenAPI fails on the known checked-in `/v1/workloads/{id}/stop` drift
  (`workload_addr` live versus `workload_id` on disk). The cumulative 02-06 diff
  is empty for `api/openapi.yaml`, `api.rs`, and `openapi.rs`; this remains
  baseline debt, not a step regression.
- `submit_service_workload_tcp_round_trip_through_vip_succeeds` reproducibly
  fails because `LOCAL_BACKEND_MAP` is not populated within five seconds. Its
  test, hydrator, and BPF map files are byte-unchanged across
  `6691bb67..3c563e28`, and `LOCAL_BACKEND_MAP` is not owned by the approved
  02-06 executable map. This is a real broad-suite baseline failure but not a
  basis for accepting or rejecting this step.
- `outbound_enforce_substrate_bidirectional_splice_zero_copy` failed in the
  broad run at mesh-peer handshake with zero splice calls. Its standalone
  rerun passed, followed by five consecutive standalone passes. The splice
  test and dataplane splice implementation are unchanged in the cumulative
  diff. The evidence classifies this occurrence as concurrency-sensitive
  transient/baseline flakiness rather than an 02-06 regression.

These unrelated failures do not substitute for the focused and native green
evidence, and that green evidence does not invalidate D19-D22.

## Iteration 4 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 6691bb67..3c563e28` | PASS |
| execution-log and roadmap JSON parse | PASS |
| `cargo xtask dst-lint` | PASS |
| Focused canonical Lima D13-D18/retained selection | PASS — 13/13; 1,885 skipped |
| Default-feature affected-package clippy with `-D warnings` | PASS for core, worker, control-plane, reconcilers, and CLI |
| All-feature affected-package clippy with `-D warnings` | PASS for the same five packages |
| Broad canonical privileged Lima affected run | FAIL — 1,876/1,879 passed; OpenAPI and `LOCAL_BACKEND_MAP` baseline failures plus one standalone-nonreproducing splice failure |
| Standalone splice reruns | PASS — initial exact rerun plus five consecutive repeats |
| Standalone `LOCAL_BACKEND_MAP` walking skeleton | FAIL reproducibly with the same five-second missing-entry assertion |
| Qualified native S-GTI-06a/06b/12a/12b selection | PASS — 4/4; 162 skipped; 87.664s; canonical native non-virtualized x86_64 KVM with selected `/srv/vm/overdrive-testing/{kernel,rootfs.ext4}` |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

## Iteration 4 verdict

**NEEDS_REVISION.** Do not complete step 02-06 or advance the DELIVER wave.
Return D19-D22 to the original 02-06 crafter. The next iteration must replace
process-local hook/event guesses with a crash-recoverable exact effect
protocol, await and propagate authoritative driver/mTLS cleanup before the
terminal fence, preserve fail-closed protection through every post-sweep boot
failure, and make both generic and worker shutdown completion cancellation-safe
and common to every concurrent caller. Add the missing pre/post process cuts
and production boot failure partitions, then continue the uncapped
review/remediation loop until the reviewer returns **APPROVED**.

---

## Iteration 5 metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Step | `02-06` |
| Reviewed commit | `ea7add2546035dae4342e2513f27d4da7de41c59` |
| Parent | `3c563e28b7b3c05ddf3da00c481c8c9ab69f0373` |
| Subject | `fix(mtls): make recovery teardown crash-safe` |
| Trailer | `Step-Id: 02-06` |
| Review iteration | 5 |
| Verdict | **NEEDS_REVISION** |

## Iteration 5 summary

The cancellation and awaited-cleanup mechanics are substantially stronger.
`OwnedTaskSet` now moves the join work into an independently owned supervisor;
late and concurrent callers share a retained completion value; allocation stop
joins its producer tree before draining enforcement handles; Stop propagates an
ordinary driver I/O error; and both Stop and Finalize await the worker's
allocation teardown result before writing their own terminal row. The focused
canonical Lima selection passes 14/14, the exact splice scenario passes, both
affected-package clippy matrices pass, and `des-verify-integrity` reports
complete traces for all nine roadmap steps.

The crash-exact terminal protocol is nevertheless still not exact. The new
filesystem files are effect *claims*, not effect completion acknowledgements.
The marker is durably created before `on_alloc_terminal` and before the
broadcast send. A cut after marker creation but before either call permanently
suppresses an effect that never happened. The tests inject only a failure
before marker creation, then manually rebuild the private driver-routing index;
they do not drive either real claim-to-effect cut or production owner
reconstruction. A crash while writing a newly created event-context file can
also leave an already-existing truncated JSON file that every later replay
rejects.

The fail-closed recovery handoff closes the specific sweep-to-reinstall window:
distinct DROP rules survive the old-rule sweep and are retained on frontend,
resolver, identity, and worker-install failure. They are released too early,
however. DNS responder probe, API bind, nonblocking setup, address lookup, and
trust-triple write are still fallible after the quarantine transaction is
deleted. A DNS probe fault returns before a server owner exists; dropping the
new worker removes the replacement rules while the independently owned
workload survives, leaving neither quarantine nor redirect.

The required native gate also regressed. The S-GTI-06a exact test failed twice
at the same assertion: the public snapshot had already exposed `Terminated`
but still reported `exit_code = None`, not the required natural completion
`Some(0)`. Both invocations then reached the 240-second nextest timeout because
the assertion bypassed fixture shutdown. The other three mapped native tests
pass 3/3. This is consistent with the newly awaited worker teardown widening
the interval between the exit-observer's terminal-state row and the later
finalization claim; whatever the internal timing, the mapped public contract is
reproducibly red on the canonical metal host.

Two additional production/test boundaries are unsafe. A failure deleting a
stale hook marker occurs after a successful process start and Running row but
before mTLS installation, so the newly fallible journal operation can return
with an unprotected live allocation. Worker-owner shutdown also logs a failed
enforcement teardown and opens its completion fence while retry handles remain.
Finally, the splice fixture changed its RAII destructor from synchronous stop
to a detached async spawn followed immediately by topology/table destruction;
that no longer proves cleanup completion and can contaminate following tests.

## Iteration 5 remediation disposition

| Prior finding | Disposition | Evidence |
|---|---|---|
| D19 — terminal hook/event crash exactness | **OPEN / still blocking** | D23 shows both durable markers are written before their effects, so a real post-claim/pre-effect cut skips the hook or event. The replacement test does not drive those cuts and manually recreates private driver routing. |
| D20 — authoritative terminal cleanup | **PARTIAL / still blocking** | Stop now rejects non-`NotFound` driver errors and both arms await `stop_alloc`; focused error/retry tests pass. The public S-GTI-06a trace still exposes terminal state before the final completion projection (D25), and journal failure can strand a Running unprotected process before intercept install (D26). |
| D21 — post-sweep fail-closed recovery | **PARTIAL / still blocking** | Quarantine correctly covers sweep through the complete replacement batch, including later-sibling install failure. It is released before later fallible boot gates and the test drives primitives rather than the production boot boundary (D24). |
| D22 — cancellation-safe shared task/worker completion | **PARTIAL / still blocking** | Successful leader cancellation, concurrent callers, late registration, producer join, and common enforcement wait are fixed and green. A worker-owner teardown error is still reduced to a warning while the fence completes with retry handles retained (D27). |

## Iteration 5 retained finding disposition (D1-D18)

| Finding | Disposition |
|---|---|
| D1 — graceful shutdown mislabeled unclean | **CLOSED** — mapped restart scenarios still use abrupt owner loss and live pre-cut PID evidence. |
| D2 — target/workload substitution | **CLOSED** — immutable target/peer intents, data, executables, and ids remain intact. |
| D3 — duplicate finalization effects | **OPEN through D23** — the durable claim-before-effect window can skip effects; exactness is not established. |
| D4 — vacuous illegal/reclamation contracts | **CLOSED** — pure pre-state/transition and planner/lifecycle contracts remain unchanged and focused-green. |
| D5 — names and Contract Shapes | **CLOSED** — all seven roadmap mappings and exact declarations remain present. |
| D6 — incomplete/wrong RED | **REOPENED as new audit defect D28** — the earlier wrong-reason RED remains superseded, but the current D19-D22 RED is only `FAIL` with no traceable behavior/result. |
| D7 — partial cleanup fenced as complete | **OPEN through D23/D25/D26** — the action arms improved, but exact effects, public terminal-last behavior, and journal-failure fail-closure remain incomplete. |
| D8 — old userspace dataplane survives | **CLOSED for successful owner shutdown** — accept/enforce/pass-through producers are joined. Failed teardown completion remains D27. |
| D9 — test-authored peer intercept | **PARTIAL through D24** — the peer remains production-recovered, but later post-release failures can remove all protection. |
| D10 — illegal property preserves reopened row | **CLOSED** — canonical terminal transition remains exact. |
| D11 — false pure-function declaration | **CLOSED** — exact rustdoc declaration remains on the pure reclamation function. |
| D12 — default production lint failure | **CLOSED** — both independent clippy feature matrices pass with `-D warnings`. |
| D13 — Finalize crash exactness/event delivery | **OPEN through D23** — a durable pre-effect claim is still at-most-once, not exact. |
| D14 — Stop terminal-first cleanup | **PARTIAL through D25/D26** — direct Stop action cleanup is awaited and error-propagating; the complete public terminal/fail-closed boundary is not. |
| D15 — stopped allocation children detach | **CLOSED for tracked producers** — focused enforce and pass-through ownership tests remain green. |
| D16 — reusable owner fence and resolver port | **CLOSED for successful completion** — core placement, cancellation, concurrency, late registration, and resolver dependency direction are correct. D27 is the failed-worker-teardown complement. |
| D17 — incomplete live recovery join | **PARTIAL through D24** — preflight and quarantine cover the replacement batch, not every later fallible boot return. |
| D18 — wrong-reason RED | **CLOSED for the Iteration 3 remediation, but current evidence is insufficient** — see D28. |

## Iteration 5 criterion disposition

| Criterion | Result | Evidence |
|---|---|---|
| S-GTI-06a | **FAIL** | Canonical qualified native execution failed twice at `guest_stack_mtls_egress.rs:4192`: snapshot state was `Terminated`, but `exit_code` was `None` rather than `Some(0)`; each run then timed out at 240 seconds. |
| S-GTI-06b | PASS | Qualified native same-id real reinstall rejection remains green in the 3/3 remainder selection. |
| S-GTI-12a | PASS for the mapped native trace | Exact target-handle removal and ordered sibling complement remain green. |
| S-GTI-12b | **FAIL overall** | The mapped native absent-guard/idempotent stop trace passes, but exact terminal effect delivery is contradicted by D23 and journal failure closure by D26. |
| P-GTI-ILLEGAL-07 | PASS | Exact pure terminal pre-state rejects READY, EXEC, and Finalize while Platform Reclaimed remains the one apply class. |
| C-GTI-RECLAMATION-ONCE | PASS | Pure claim/redrive and bounded executor support remain green. |
| C-GTI-FINALIZE-TWICE | **FAIL** | The selected test passes only the pre-claim and completed-marker partitions; D23 identifies the untested post-claim/pre-hook and post-claim/pre-event histories. |

## Iteration 5 findings

### D23 — the durable terminal journal claims effects before executing them

- **Severity:** Critical
- **Dimensions:** Crash consistency, exactly-once effects, lifecycle event
  delivery, test external validity
- **Affected contracts:** C-GTI-FINALIZE-TWICE, S-GTI-12b, retained D3/D13
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:855-887`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:889-923`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:982-1014`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1052-1065`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2076-2083`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:734-807`

`claim_terminal_hooks` creates and fsyncs `terminal-hook:<alloc>` before
`driver.on_alloc_terminal`. A process cut after `create_once` returns and
before the synchronous hook call leaves the marker present, so every
replacement returns `false` and the hook is skipped forever. Lifecycle events
have the same ordering: `claim_lifecycle_event` creates the marker before
`bus.send`. The protocol therefore gives at-most-once *attempts*, not exactly
once effects.

The test-injected event failure fires before `claim`, so it covers row-before-
claim only. It never cuts after marker fsync and before broadcast. Hook
replacement similarly occurs either before the marker or after the hook, never
inside the actual claim-to-call window. The test also manually inserts
`DriverType::Exec` into every replacement `AllocDriverIndex`; production live
recovery does not rebuild that private map. Thus its one-hook count is partly a
fixture-authored owner reconstruction.

The event context is not crash-atomically published either. `create_new` makes
the final path visible before `write_all`/`sync_all`; a cut in that interval
leaves an existing empty or partial `.event` file. The next `create_once`
returns `AlreadyExists`, and `serde_json::from_slice` then rejects the same file
on every replay.

**Required remediation:** use an internal durable outbox/progress protocol that
can recover both sides of every effect boundary. Marker-before-effect is only
valid when the effect port consumes a stable idempotency key atomically; the
current void hook and ephemeral broadcast do not. Add real process cuts after
durable claim and before/after hook/send, durable consumer acknowledgment or an
honest idempotent delivery identity, and atomic temp-write/fsync/rename for
context records. Drive production owner reconstruction without inserting the
private routing map from the test.

### D24 — quarantine is released before the complete fallible boot boundary

- **Severity:** Critical
- **Dimensions:** Security fail-closed recovery, boot atomicity, production-path
  accuracy
- **Affected contract:** S-GTI-06a production recovery
- **Evidence:**
  - `crates/overdrive-control-plane/src/lib.rs:2982-2991`
  - `crates/overdrive-control-plane/src/lib.rs:3014-3050`
  - `crates/overdrive-control-plane/src/lib.rs:3185-3208`
  - `crates/overdrive-control-plane/tests/integration/adopt_on_restart.rs:545-624`

The quarantine correctly stays ahead of the replacement rules during sweep,
frontend rebuild, resolver probe, identity issuance, and sibling installation.
It is then deleted immediately after `apply_live_mtls_intercepts`. The DNS
responder probe is still fallible after that point; so are API listener bind,
`set_nonblocking`, `local_addr`, and trust-triple persistence.

The deterministic `dns_probe_fault` path returns before a `ServerHandle`
exists. The live workload is independently owned and survives, while dropping
the local worker removes its freshly installed intercept rules. Because the
quarantine batch was already deleted, the return leaves no fail-closed rule.
Later bind/trust failures have additional background-task ownership hazards,
but the DNS path alone falsifies complete post-sweep containment.

The new integration test directly installs, retains, adopts, and releases one
quarantine around direct sweep/reinstall functions. It does not invoke
`run_server_with_obs_and_drivers` or inject DNS/bind/trust failure with live
survivors, so it cannot observe the early release.

**Required remediation:** retain one recoverable quarantine owner until every
fallible server-construction gate has completed and the returned server owner
can preserve replacement rules, or add an error guard that atomically restores/
retains quarantine before any post-release return. Drive the actual production
boot with live sibling survivors and faults at every post-sweep return.

### D25 — the mapped native successful-reclamation contract is reproducibly red

- **Severity:** Critical
- **Dimensions:** Acceptance regression, terminal-last behavior, native evidence
- **Affected contracts:** S-GTI-06a, retained D7/D14
- **Evidence:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4187-4201`
  - canonical metal nextest run `3a2ecd03-c23a-4f1f-9561-4b907ad7d471`
  - canonical metal nextest run `39613190-133b-4e62-a631-f092def63f23`

The four-test canonical selection stopped on S-GTI-06a, and its exact
standalone rerun failed identically. In both runs the snapshot passed the
`state == Terminated` assertion but failed `exit_code == Some(0)` with
`left: None`; nextest then terminated the leaked fixture at 240 seconds. This
is not a Lima/nested substitute: both failures came from the qualified native
x86_64/KVM metal runner with the selected kernel and rootfs.

The newly awaited `stop_alloc` makes the interim terminal-state surface long
enough for the public poll to observe before the Completed projection. Whether
the correct remediation is to prevent a terminal public state before cleanup
or to make the contract wait for a typed terminal claim, the checked-in mapped
scenario is red and the natural-completion outcome is not currently proved.

**Required remediation:** eliminate the terminal-without-completion public
window or make the approved native contract observe the authoritative typed
terminal completion without weakening its natural-exit assertion. Ensure a
failed assertion still performs bounded fixture shutdown so failures do not
turn into 240-second timeouts. Re-run all four mapped scenarios on metal.

### D26 — a terminal-journal I/O error can publish Running before mTLS install

- **Severity:** Critical
- **Dimensions:** Security fail-closed start/restart, error ordering, new
  persistence dependency
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:926-940`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2400-2463`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2905-2954`

`record_running` now performs a fallible filesystem deletion before inserting
the driver route. Both Start and Restart invoke it only after a successful
driver start and durable Running observation, but before `worker.start_alloc`.
Permission, filesystem, or directory-fsync failure therefore returns
`TerminalEffectJournal` immediately: the row says Running, the process is live,
and the fail-closed mTLS install/rollback arm has not executed.

No new test makes the terminal-effects directory fail at this boundary. The
happy temporary-directory fixture cannot falsify the exposure.

**Required remediation:** move/prepare journal generation state before any
Running commit, or treat a journal failure through the same authoritative
stop, supervision release, Failed-row, and mTLS fail-closed rollback protocol
as an intercept-install failure. Add Start and same-id Restart failure tests
using the production ordering.

### D27 — worker-owner shutdown completes after a failed enforcement teardown

- **Severity:** Major
- **Dimensions:** Full-owner completion, error authority, retained-handle leak
- **Affected finding:** D22
- **Evidence:**
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:799-818`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:866-889`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:1231-1252`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:2247-2331`

Allocation stop correctly returns an enforcement teardown error and a later
allocation stop retries its retained handles. Full worker shutdown does not.
It waits each `AllocStop`, logs an `Err`, and then lets the shared completion
fence open. The failed handles remain in `retry_handles`, but no retry is
started and `shutdown_owner` has no error result. Every concurrent/replacement
caller consequently observes successful completion while authoritative
teardown is still incomplete.

The replacement-shutdown test uses only successful teardown. The failure/retry
test calls `stop_alloc` twice and never drives `shutdown_owner`, so the log-only
full-shutdown arm is uncovered.

**Required remediation:** define and implement an authoritative failed-owner-
shutdown disposition: retry to convergence, return a shared typed error while
retaining an addressable owner, or explicitly contain/close the resource before
completion. Every concurrent caller must observe the same result, and a test
must combine leader cancellation with a failing first teardown.

### D28 — the latest DES RED is mechanically valid but behaviorally untraceable

- **Severity:** Major
- **Dimensions:** TDD phase honesty, auditability
- **Evidence:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/execution-log.json:831-850`
  - reflog entries `a3613fb2` and `ea7add25`

The current RED, GREEN, and COMMIT events contain only `FAIL`, `PASS`, and
`PASS`. `des-verify-integrity` accepts the phase shape and reports all nine
steps complete, and reflog proves the initial commit plus execution-log-only
amend. Neither source identifies the tests run, the assertions that failed, or
why the RED was the right failure for D19-D22. The DES audit log at those
timestamps records generic non-DES tool-hook invocations without command or
result detail. The phase is therefore mechanically present but not
behaviorally traceable.

**Required remediation:** preserve the existing append-only log and append a
fresh behavioral RED→GREEN→COMMIT remediation cycle whose RED records the
compiled failing contract names and right-reason assertions, and whose GREEN
records the exact focused/native commands and counts.

### D29 — the splice fixture detaches cleanup from its RAII boundary

- **Severity:** Major
- **Dimensions:** Test isolation, cleanup honesty, kernel residue
- **Evidence:**
  - `crates/overdrive-worker/tests/integration/outbound_enforce_substrate_splice.rs:286-303`

`TopologyGuard::drop` now spawns `worker.stop_alloc` and immediately destroys
the topology and shared nft infrastructure. It neither awaits nor owns the
spawned task. Runtime shutdown may cancel it, and concurrent table deletion can
race the worker's guard removal. The test's primary splice assertion still
passes standalone, but its claimed stop-first cleanup ordering and residue
isolation are no longer true.

**Required remediation:** make teardown an explicitly awaited async fixture
phase before Drop, with Drop only a bounded emergency fallback, and assert the
worker task/rule/connection complement before removing shared infrastructure.

## Strong evidence retained after Iteration 5

- Direct Stop now propagates ordinary driver I/O errors; only explicit
  `NotFound` is treated as benign absence.
- `stop_alloc` is async, joins the allocation producer tree, drains enforcement
  handles only after producer completion, surfaces teardown failure, and lets a
  later allocation retry converge.
- `CompletionFence` retains fast completion and owns work independently of the
  initiating caller. Core cancellation, concurrent waiter, and late-registration
  tests pass.
- Quarantine userdata is excluded from the ordinary per-workload sweep;
  release deletes the whole batch transactionally, and failed release retains
  the kernel rules.
- The four literal incomplete live-join partitions remain refused before
  sweep; the new issue is after the replacement batch and quarantine release.
- All seven mapped function names and Contract Shape declarations are exact,
  including `/// CONTRACT_SHAPE: pure-function.` on both pure properties.
- The cumulative step still adds no OpenAPI/REST, Beacon, rkyv/observation
  schema, Cargo dependency, example, expectation, E08/E09, legacy/no-token,
  built-product-from-Rust-test, Service-plus-VM, or mutation scope.

## Iteration 5 scope and boundary audit

The remediation commit changes 18 files with 1,777 insertions and 389
deletions; 386 inserted lines are the committed Iteration 4 review artifact.
The cumulative 02-06 diff changes 26 files with 5,771 insertions and 503
deletions. The new persistence is an internal filesystem directory beside the
intent database, so public frozen REST/wire/rkyv/observation schemas remain
unchanged. D23 and D26 show that internal placement alone does not make its
crash protocol correct.

| Boundary | Result |
|---|---|
| Built-product boundary | PASS — Rust integration tests do not spawn the built Overdrive product binary |
| Example/expectation separation | PASS — no example or expectation changed or ran as the system under test |
| E08/E09 | PASS — neither path was introduced |
| Legacy/no-token | PASS — no legacy path was added |
| Service-plus-VM | PASS — target remains a VM Job; peer remains an independent Exec Service |
| Frozen public schemas | PASS — no REST/OpenAPI, Beacon, rkyv, observation-schema, or Cargo change |
| Terminal exactness | **FAIL** — D23/D26 |
| Complete boot fail-closure | **FAIL** — D24 |
| Worker completion | **FAIL on teardown error** — D27 |
| Native mapped gate | **FAIL** — D25 |
| Test isolation | **FAIL** — D29 |
| Mutation discipline | PASS — no per-step mutation run or exclusion edit |

## Iteration 5 DES and commit chronology

`PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity
docs/feature/guest-stack-transparent-mtls-intercept/deliver/` exits 0 with
`All 9 steps have complete DES traces`. The roadmap is approved and enumerates
exactly `01-01` through `02-06`. JSON parsing succeeds.

The latest timestamps are ordered RED `15:18:59Z`, GREEN `15:29:36Z`, COMMIT
`15:30:06Z`. Reflog records initial implementation commit `a3613fb2` at
`17:29:52+02:00`, then an execution-log-only seven-line amend to reviewed
commit `ea7add25` at `17:30:06+02:00`; subject, parent, and trailer are
consistent. D28 is not a phase-shape or commit-existence failure. It is the
absence of behavioral evidence in this latest terse cycle.

## Iteration 5 broad-gate and native investigation

The affected all-feature broad Lima run started 2,150 tests. It stopped at the
known checked-in OpenAPI drift after 846 passes, so 1,303 tests were not run in
that invocation. The cumulative diff remains empty for `api/openapi.yaml`,
`api.rs`, and `openapi.rs`; the failure is still baseline debt. The focused
14-test remediation selection and exact splice scenario independently pass.
The prior known `LOCAL_BACKEND_MAP` baseline and prior transient splice
classification were not reclassified by this fail-fast run.

The native four-test command selected the canonical kernel/rootfs and acquired
the metal lease. It ran S-GTI-06a first and failed, so nextest did not execute
the remaining three. A separate qualified selection of S-GTI-06b/12a/12b then
passed 3/3 in 54.215 seconds. An exact qualified S-GTI-06a rerun reproduced the
same assertion and timeout, so D25 is a current mapped failure rather than a
single broad-run transient.

## Iteration 5 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 6691bb67..ea7add25` | PASS |
| execution-log and roadmap JSON parse | PASS |
| `cargo xtask dst-lint` | PASS |
| Full DES integrity verifier | PASS — all 9 steps have complete phase shapes |
| Focused canonical Lima remediation/retained selection | PASS — 14/14; 1,888 skipped |
| Exact canonical Lima splice scenario | PASS — 1/1; 61 skipped |
| Default-feature affected-package clippy with `-D warnings` | PASS for core, worker, control-plane, reconcilers, and CLI |
| All-feature affected-package clippy with `-D warnings` | PASS for the same five packages |
| Broad affected all-feature Lima run | FAIL-FAST — 846/847 executed passed except known OpenAPI drift; 1,303 not run |
| Qualified native S-GTI-06a/06b/12a/12b selection | FAIL — S-GTI-06a `exit_code None != Some(0)` and 240-second timeout; remaining tests cancelled |
| Qualified native S-GTI-06a exact rerun | FAIL reproducibly — identical assertion and timeout |
| Qualified native S-GTI-06b/12a/12b selection | PASS — 3/3; 163 skipped; 54.215s |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

## Iteration 5 verdict

**NEEDS_REVISION.** Do not complete step 02-06 or advance the DELIVER wave.
Return D23-D29 to the original 02-06 crafter. The next iteration must replace
claim-before-effect files with a genuinely recoverable/idempotent effect
protocol, keep survivor quarantine recoverable through every fallible boot
return, restore S-GTI-06a on qualified metal without weakening natural
completion, fail closed on journal I/O before publishing Running, make failed
worker-owner teardown part of the shared result, restore awaited splice fixture
cleanup, and append a behaviorally traceable DES cycle. Continue the uncapped
review/remediation loop until the reviewer returns **APPROVED**.

---

## Iteration 6 metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Step | `02-06` |
| Reviewed commit | `bfcd39726ffa6f65deb7aad6bae36b25d3bad58c` |
| Parent | `ea7add2546035dae4342e2513f27d4da7de41c59` |
| Subject | `fix(mtls): make protected recovery effects exact` |
| Trailer | `Step-Id: 02-06` |
| Review iteration | 6 |
| Verdict | **NEEDS_REVISION** |

## Iteration 6 summary

Several concrete defects are closed. Start and same-id Restart now prepare and
fsync the terminal-effect route before invoking the driver or publishing
Running. Lifecycle context is atomically linked from a fully synced temporary
file. The recovery quarantine is transferred into the complete boot scope and
is released only after DNS probe, listener bind, nonblocking setup, bound-address
read, and trust-triple persistence; its Drop leaves the kernel DROP rules
adoptable on every intervening error. S-GTI-06a drives the real production
composition through a post-reinstall DNS refusal and observes quarantine without
replacement redirects. Allocation and worker cleanup now expose typed results,
and the splice clean path explicitly awaits the full stop/task/rule complement
before removing shared infrastructure.

The step is still not complete. The purported terminal-effect idempotency is
only process-local at both production consumers. A real replacement process
recreates `IdempotentLifecycleEventPort` with an empty set, while the Driver
trait's default idempotent method ignores the stable key and delegates to the
ordinary hook. The acceptance fixture recreates only `AllocDriverIndex`; it
keeps the same driver registry, shared hook-deduplication set, lifecycle port,
sender, and receiver. It therefore cannot prove the pre/post-effect cuts across
the real owner boundary and masks the duplicate effect that a fresh production
composition permits.

The required native stability gate is red. Four consecutive exact S-GTI-06a
runs on the qualified metal host produced three passes and one 240-second
timeout. The failing run observed a guest-originated ARP reply and IPv4 TCP RST
on the tap before the exact reinstall readiness barrier, then leaked the
fixture because that earlier assertion is still outside the bounded cleanup
path. A subsequent full four-scenario run passed 4/4, but it cannot erase the
failed third repeat or establish deterministic first-flow protection.

Finally, the worker's internal retry generation is not carried through the
server ownership boundary. Both graceful shutdown and abrupt test-owner loss
discard the shared typed error and consume the only server-held worker `Arc`,
so failed teardown handles cease to be addressable for the retry the unit test
demonstrates. The latest DES cycle also records prohibited non-doctest
`cargo test` commands in RED and GREEN. The repository explicitly blocks that
runner; syntactically complete phase events cannot treat an illegal command as
executed evidence.

## Iteration 6 remediation disposition

| Iteration 5 finding | Disposition | Evidence |
|---|---|---|
| D23 — terminal journal claims before effects | **PARTIAL / still blocking** | Atomic event-context publication and stable effect keys are improvements, and the claim marker no longer suppresses delivery. The actual consumers do not preserve key consumption across a real process recreation, while the test preserves both consumer witnesses (D30). |
| D24 — quarantine released before complete boot | **CLOSED** | `RecoveryQuarantineBatch` spans the replacement batch through DNS, bind, nonblocking, local address, and trust persistence; every early return drops to retained kernel rules, and only the post-trust success site releases. S-GTI-06a drives the production DNS-fault boundary and observes quarantine present with replacement redirect guards gone. |
| D25 — S-GTI-06a native failure | **OPEN / changed failure** | Typed natural-completion polling no longer mistakes `Terminated + exit_code None` for completion, and later assertions clean up before failing. Four independent exact final-source repeats were only 3/4: one run observed pre-readiness guest frames and timed out (D31). |
| D26 — journal error after Running | **CLOSED** | Both Start and Restart call `prepare_running` before `driver.start`; failure tears down the provisioned C3 owner and the focused tests prove no driver start and no new Running row. |
| D27 — worker-owner teardown failure is log-only | **PARTIAL / still blocking** | Concurrent callers now share a typed error and a later direct `shutdown_owner` call retries retained handles. `ServerHandle::{shutdown,abort_for_test}` discard that error and consume the addressable owner (D32). |
| D28 — behaviorally untraceable DES | **OPEN / still blocking** | The new text names behaviors, commands, counts, and run ids, but its RED and D27 GREEN command use forbidden non-doctest `cargo test`; the evidence is not legal repository execution (D33). |
| D29 — detached splice cleanup | **CLOSED** | `TopologyGuard::finish().await` waits for `stop_alloc`, asserts the joined task/connection result and exact rule absence, then removes topology/shared infra. Drop is synchronous emergency scrub only and no longer spawns detached work. |

## Iteration 6 retained finding disposition (D1-D22)

| Finding | Disposition |
|---|---|
| D1 — graceful shutdown mislabeled unclean | **CLOSED** — mapped restart still revokes the server/worker owner abruptly and proves live processes before the cut. |
| D2 — target/workload substitution | **CLOSED** — target and peer intent, executable, data, and allocation identity remain unchanged; no second deploy/restart manufactures the target result. |
| D3 — duplicate finalization effects | **OPEN through D30** — stable keys are not consumed durably across the actual process composition boundary. |
| D4 — vacuous illegal/reclamation contracts | **CLOSED** — exact pure transition and one-claim/one-redrive properties remain green. |
| D5 — names and Contract Shapes | **CLOSED** — all seven roadmap function mappings remain exact and the two mapped pure properties carry the exact required rustdoc declaration. |
| D6 — incomplete/wrong RED | **OPEN through D33** — the older right-reason RED remains history, but the current remediation cycle cites a prohibited runner. |
| D7 — partial cleanup fenced as complete | **OPEN through D30/D31** — destructive cleanup is awaited before the row, but exact hook/event delivery and the native pre-reinstall first-flow boundary remain red. |
| D8 — old userspace dataplane survives | **CLOSED on successful owner shutdown** — tracked producers are joined. The failed full-owner disposition remains D32. |
| D9 — test-authored peer intercept | **CLOSED** — the peer is reconstructed by production boot; no direct identity/intercept installation seam returned. |
| D10 — illegal property preserves reopened row | **CLOSED** — READY, EXEC, and duplicate Finalize apply zero delta to the exact terminal pre-state. |
| D11 — false pure-function declaration | **CLOSED** — reclamation planning/lifecycle evidence is effect-free and the bounded executor evidence remains separate. |
| D12 — default production lint failure | **CLOSED** — both affected-package feature matrices pass independently with `-D warnings`. |
| D13 — Finalize crash exactness/event delivery | **OPEN through D30** — the outbox key reaches consumers, but both production consumer acknowledgements reset with process recreation. |
| D14 — Stop terminal-first cleanup | **PARTIAL through D30/D32** — driver/network/mTLS cleanup is ordered and error-propagating; exact terminal effects and failed full-owner retry do not survive the outer owner boundary. |
| D15 — stopped allocation children detach | **CLOSED** — accept, enforce, pass-through, and teardown producers remain under owned task sets and the focused complements stay green. |
| D16 — reusable owner fence and resolver port | **CLOSED for successful completion** — core placement, cancellation safety, shared waiters, late registration, and resolver dependency direction remain correct. Failed composition-owner completion remains D32. |
| D17 — incomplete live recovery join | **CLOSED** — complete planning precedes sweep and quarantine protects all later fallible production gates. |
| D18 — wrong-reason RED | **CLOSED for its historical cycle; current evidence fails D33**. |
| D19 — terminal row cannot distinguish hook/event cuts | **OPEN through D30** — stable identities exist, but the production consumers do not retain their dedupe state across a real replacement process. |
| D20 — terminal before authoritative cleanup | **CLOSED for allocation cleanup** — Stop and Finalize await worker, driver, and network results before the durable terminal write. |
| D21 — recovery removes last fail-closed rule | **CLOSED** — quarantine precedes sweep, survives every error, and releases after complete readiness. |
| D22 — shutdown completion orphaned/early | **PARTIAL through D32** — the generic and worker-local fences are cancellation-safe and shared, but the outer server owner discards failure and retry ownership. |

## Iteration 6 criterion disposition

| Criterion | Result | Evidence |
|---|---|---|
| S-GTI-06a | **FAIL** | The production DNS-refusal/quarantine partition and typed natural completion pass often, and the full mapped selection passed. The required four exact consecutive repeats were 3/4; one qualified native run captured guest tap traffic before reinstall readiness and then timed out (D31). |
| S-GTI-06b | PASS for the mapped native trace | The full qualified four-scenario selection passes same-id INPUT-hook rejection, no EXEC/frame, Failed terminal, and restoration. |
| S-GTI-12a | PASS for the mapped native trace | The full selection preserves exact target-handle deletion and the ordered sibling complement. |
| S-GTI-12b | **FAIL overall** | The native Stopped/AlreadyStopped and absent-guard trace passes, but its repeated-finalization/no-duplicate claim is contradicted by D30; failed outer-owner teardown also loses retry authority (D32). |
| P-GTI-ILLEGAL-07 | PASS | The exact terminal-state transition property is independently green. |
| C-GTI-RECLAMATION-ONCE | PASS | The pure same-epoch claim/redrive property is independently green. |
| C-GTI-FINALIZE-TWICE | **FAIL** | The focused fixture passes only because it preserves the driver and lifecycle consumer dedupe sets while recreating the journal index (D30). |

## Iteration 6 findings

### D30 — stable effect keys are consumed only by witnesses that reset or survive unrealistically

- **Severity:** Critical
- **Dimensions:** Crash consistency, exactly-once effects, production-path
  accuracy, test external validity
- **Affected contracts:** C-GTI-FINALIZE-TWICE, S-GTI-12b, retained D3/D13/D19
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:785-842`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1286-1301`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2073-2095`
  - `crates/overdrive-core/src/traits/driver.rs:1036-1043`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:91-156`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:725-747`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:856-863`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:928-936`

The filesystem context and stable keys remove D23's literal
claim-before-effect suppression, but they do not make either effect consumer
crash-idempotent. `IdempotentLifecycleEventPort` owns a process-local
`BTreeSet`. `AppState` constructs it fresh on every boot. After an event was
sent and the process died, the exact-terminal replay arm calls the fresh port,
whose empty set sends the same logical event again. The durable marker's
`claimed` result is ignored by `emit_lifecycle_event_once`, so it is not an
acknowledgement and cannot distinguish pre-send from post-send process loss.

The driver side has the same gap. `Driver::on_alloc_terminal_idempotent`
receives a stable key, but its default implementation discards that key and
calls `on_alloc_terminal`. Neither production Driver overrides the method.
Thus a cut after the hook and before the terminal row rebuilds the durable
route and invokes the ordinary hook through a fresh driver composition. A
stable string is not idempotency unless the effect port atomically consumes it.

The focused test does not recreate those owners. It constructs one registry
whose scripted driver's `terminal_effects` set survives every attempt and one
`IdempotentLifecycleEventPort` whose consumed set, sender, and receiver also
survive. Each purported process replacement reassigns only
`AllocDriverIndex`. The assertion therefore proves that long-lived test
consumers deduplicate keys; it does not prove that production process
recreation does.

**Required remediation:** make the real driven consumers crash-idempotent under
the stable identity, or use an equivalent recoverable delivery/acknowledgement
protocol that distinguishes both sides of the hook and event boundaries.
Recreate the complete production owner in the cut test: journal index, Driver
registry/driver instance, lifecycle port, and sender/subscriber ownership. Cut
immediately before and after each effect and after repeated process
replacements. Every partition must observe one logical hook and one exact event,
not one per reconstructed process.

### D31 — S-GTI-06a fails one of four exact native repeats before reinstall readiness

- **Severity:** Critical
- **Dimensions:** Native acceptance stability, first-flow fail-closure,
  assertion-safe cleanup
- **Affected contract:** S-GTI-06a
- **Evidence:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4048-4106`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4240-4286`
  - metal run `49384387-7bba-4cfd-8889-4ade34424618`

I ran the exact scenario four consecutive times from reviewed source on the
qualified non-virtualized x86_64/KVM host. Runs
`d758de8b-25ce-4f53-b62a-bb3b8efa3a15`,
`b9fa58ce-55b2-4e15-8a0f-8fcfe257a711`, and
`17a30417-0523-4c70-afea-eee6ffab8be5` passed. The third run,
`49384387-7bba-4cfd-8889-4ade34424618`, failed at line 4103. The exact tap
capture contained an ARP reply and an IPv4 TCP RST sourced from the guest-side
address before `readiness.kernel_barrier_at`. This is the contract's direct
“no guest frame before exact reinstall” oracle, not a Lima/nested substitute or
an unrelated peer mutation.

The failure then reached nextest's 240-second timeout. The new cleanup-first
assertion discipline covers the later natural-completion result, but
`observe_restarted_mesh_flow` still asserts its pre-readiness capture before it
returns ownership to the caller, so that earlier failure bypasses peer stop and
server shutdown. A later full mapped run
`189d4774-dcda-40f9-91ea-9201f9f0fe26` passed 4/4; it does not supersede a
failed member of the required repeat set.

**Required remediation:** close the actual VMM-release-to-intercept-readiness
race so no guest-originated EtherType can cross the tap/host-veth boundary
before replacement protection is authoritative. Preserve the strict capture
oracle; do not filter ARP/RST or add delay. Move all assertions behind an
owned result so every failure synchronously drains the server, peer, VM, and
kernel fixture. Re-run four consecutive exact S-GTI-06a cases and then the
full mapped selection on metal.

### D32 — server shutdown discards the typed worker failure and its retry owner

- **Severity:** Major
- **Dimensions:** Teardown authority, retry ownership, composition-root error
  propagation
- **Affected finding:** D27/D22
- **Evidence:**
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:359-380`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:912-961`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:2348-2380`
  - `crates/overdrive-control-plane/src/lib.rs:1334-1339`
  - `crates/overdrive-control-plane/src/lib.rs:1441-1450`
  - `crates/overdrive-cli/src/commands/serve.rs:88-99`

The worker-local design now does what D27 requested: one fenced attempt stores
one typed result, all concurrent callers observe it, failed handles stay under
`stopping`, and a later direct call starts a retry generation. The test passes
because it retains its own `Arc<MtlsInterceptWorker>` and explicitly calls
`shutdown_owner` a second time.

Neither production ownership path retains that ability. Graceful
`ServerHandle::shutdown` and abrupt `abort_for_test` both execute
`let _ = worker.shutdown_owner().await`, discard `Err`, and then consume/drop
the only worker owner carried by the server. The CLI wrapper documents shutdown
as currently infallible and unconditionally returns `Ok(())`. On the exact
failed-first-teardown partition, authoritative cleanup is therefore reported as
successful and the retained retry handles become unreachable when the consumed
handle returns.

**Required remediation:** carry the shared typed result through the outer
server/CLI ownership boundary. Either retry internally to convergence before
consuming the owner or return an error together with an addressable retry
owner/disposition; do not silently drop the only owner of retained handles. Add
graceful and abrupt composition-root tests with a failing first enforcement
teardown and prove the caller sees one shared result and can converge the exact
retained handle.

### D33 — the latest DES cycle cites a runner the repository forbids

- **Severity:** Major
- **Dimensions:** TDD evidence legality, auditability, repository compliance
- **Evidence:**
  - `.claude/rules/testing.md:181-204`
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/execution-log.json:852-864`

The appended RED is behaviorally descriptive, but its exact command is
`cargo xtask lima run -- cargo test -p overdrive-worker ...`. The GREEN event
repeats the same non-doctest `cargo test` command as its D27 proof. The testing
rule is categorical: `cargo test` is blocked for every non-doctest shape and
must be rewritten to Lima-routed `cargo nextest run`. Wrapping the prohibited
runner inside `cargo xtask lima run --` does not make it allowed. The GREEN
also renders its other “exact” nextest commands without their required Lima
wrapper, despite describing them as Lima runs.

`des-verify-integrity` correctly accepts the append-only RED/GREEN/COMMIT
shape; it does not validate command legality. Reflog chronology cannot turn a
forbidden command into executable evidence either.

**Required remediation:** leave history append-only and add a corrective
behavioral RED→GREEN→COMMIT cycle using repository-legal commands: Lima-wrapped
nextest for Linux tests and the canonical metal wrapper for native tests. The
RED can directly record the currently failing D30-D32/D31 contracts. GREEN
must include the full outer wrapper, exact selection, count, and result. Do not
claim an illegal `cargo test` invocation as an executed phase.

## Strong evidence retained after Iteration 6

- `prepare_running` atomically persists the route before either Start or
  Restart can invoke `Driver::start`; the broken-root tests prove zero starts
  and zero new Running publication.
- Lifecycle event context is temp-write, file-fsync, atomic hard-link, and
  directory-fsync. A crash can leave an ignored temp file but cannot expose a
  truncated final context.
- `RecoveryQuarantineBatch::Drop` disarms ownership while leaving DROP rules in
  the kernel; successful release deletes the whole deduplicated handle batch in
  one nft transaction. The production DNS-fault trace observes quarantine and
  no replacement redirect after failed composition cleanup.
- `CompletionFence`, `OwnedTaskSet`, allocation stop, and worker-local owner
  stop retain cancellation-safe independently owned work and shared completion.
  D32 is specifically the outer composition's discarded failure.
- `TopologyGuard::finish` is awaited on the clean splice path and verifies the
  worker result plus exact target-rule absence before shared teardown. The exact
  Tier-3 splice scenario passes independently.
- Typed natural-completion polling requires the unchanged allocation id,
  `Terminated`, and `exit_code == Some(0)` and rejects Failed. It does not turn
  the earlier `Terminated + None` projection into success.
- All seven mapped names and Contract Shape declarations remain exact. No
  frozen public schema changed.

## Iteration 6 scope and boundary audit

The Iteration 6 remediation changes 13 files with 1,281 insertions and 168
deletions; 425 inserted lines are the committed Iteration 5 review artifact.
The cumulative 02-06 range changes 27 files with 6,909 insertions and 528
deletions, including this native Markdown review and the append-only DES log.
The production expansions are related to crash effects, boot recovery,
shutdown ownership, and test cleanup. D30-D33 identify correctness/evidence
defects within that related scope, not unrelated file creep.

| Boundary | Result |
|---|---|
| Built-product boundary | PASS — no Rust integration test spawns the built Overdrive production binary |
| Example/expectation separation | PASS — no example or expectation changed or acted as the system under test |
| E08/E09 | PASS — neither path appears in the remediation or cumulative code diff |
| Legacy/no-token | PASS — no legacy or token-bypass path was introduced |
| Service-plus-VM | PASS — the target remains a VM Job and the peer remains an independent Exec Service |
| Frozen wire/persistence/API shapes | PASS — no REST/OpenAPI, Beacon, rkyv, observation-schema, or Cargo-manifest change |
| Generic ownership placement | PASS — `OwnedTaskSet` stays dependency-neutral in core and resolver ownership stays outside `MtlsResolve` |
| Crash-exact terminal effects | **FAIL** — D30 |
| Complete boot fail-closure | PASS — D24 closed |
| Outer worker teardown authority | **FAIL** — D32 |
| Native mapped stability | **FAIL** — D31 |
| DES command legality | **FAIL** — D33 |
| Mutation discipline | PASS — no per-step mutation run or exclusion edit |

## Iteration 6 broad-gate investigation

I reran the affected all-feature broad command with `--no-fail-fast`. Nextest
run `2d681539-01eb-4515-b2b6-eb9180202eb5` executed all 2,152 selected tests:
2,081 passed, 71 failed, and 19 were skipped. The 71 failures classify exactly
as follows, rather than as one undifferentiated environment count:

| Category | Count | Classification |
|---|---:|---|
| `NestedAppleHost` fixture refusal | 66 | Every failure is an overdrive-cli KVM/native scenario selected by `--all-features` inside virtualized Lima. They fail before scenario execution at the qualified-native preflight. The canonical metal lane is the required replacement; the full mapped selection passed there, while D31 records a real metal failure separately. |
| Host-global nft fault-fixture preconditions | 3 | `prior_fixture_duplicate_exemptions_are_safely_repaired_once` found no production table; two setup/trap matrix cases did not reach their expected injected rename interruption against the absent/unchanged host object graph. The entire fault-fixture file is byte-unchanged across `6691bb67..bfcd3972`; these are baseline fixture/precondition failures, not remediation regressions. |
| Checked-in OpenAPI drift | 1 | The known `/v1/workloads/{id}/stop` `workload_addr` versus `workload_id` divergence. The cumulative step has no API/OpenAPI diff. |
| `LOCAL_BACKEND_MAP` walking skeleton | 1 | The unchanged backend-discovery bridge failed to populate the map within five seconds. Its test and owned production files are unchanged in this step; this is the previously reproduced baseline failure. |

This classification validates the execution log's four high-level categories,
but it does not make the broad run green and it does not excuse D31: D31 was
observed on the qualified native substrate after preflight and inside the
mapped oracle.

## Iteration 6 DES and commit chronology

The newest timestamps are ordered RED `16:03:16Z`, GREEN `16:56:29Z`, and
COMMIT `16:57:06Z`. `des-verify-integrity` exits 0 and reports complete traces
for all nine roadmap steps. The roadmap remains approved and JSON parsing
succeeds.

The COMMIT event names `407677b79abec13b5009b772318d26f1b1e95e35`.
Reflog records that initial implementation commit at `18:56:53+02:00`, followed
by a seven-line execution-log-only amend to reviewed commit `bfcd3972` at
`18:57:12+02:00`. Parent, subject, trailer, and implementation tree match. The
amend is recoverable and honest. D33 is solely the prohibited RED/GREEN command,
not phase order or commit existence.

## Iteration 6 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 6691bb67..bfcd3972` | PASS |
| execution-log and roadmap JSON parse | PASS |
| `cargo xtask dst-lint` | PASS |
| Full DES integrity verifier | PASS — all 9 steps have complete phase shapes |
| Focused canonical Lima action-shim crash-observability file | PASS — 14/14; nextest run `eb969733-b904-4a71-b1a2-eba2b9a2aa54` |
| Focused pure P-GTI-ILLEGAL-07 and C-GTI-RECLAMATION-ONCE | PASS — 2/2; nextest run `3ae831d8-12ae-4589-9626-c86b00d82603` |
| Legal Lima D27 worker-local retry selection | PASS — 1/1; nextest run `99b5e11c-f190-4d97-bc67-5afeb78858de` |
| Exact canonical Lima splice scenario | PASS — 1/1; nextest run `c3b152e3-2869-4635-a0a7-6ea37971e07e` |
| Default-feature affected-package clippy with `-D warnings` | PASS for core, worker, control-plane, reconcilers, and CLI |
| All-feature affected-package clippy with `-D warnings` | PASS for the same five packages |
| Broad canonical privileged Lima affected run | FAIL — 2,081/2,152 passed; exact 71-failure classification above |
| Four consecutive exact qualified-native S-GTI-06a repeats | **FAIL — 3/4**; third run captured pre-readiness guest frames and timed out at 240s |
| Qualified-native S-GTI-06a/06b/12a/12b selection | PASS — 4/4; nextest run `189d4774-dcda-40f9-91ea-9201f9f0fe26`; 88.434s |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

## Iteration 6 verdict

**NEEDS_REVISION.** Do not complete step 02-06 or advance the DELIVER wave.
Return D30-D33 to the original 02-06 crafter. The next iteration must make the
terminal hook and lifecycle event consumers genuinely idempotent across a full
production process reconstruction, close the native pre-reinstall guest-frame
race with assertion-safe cleanup, preserve/propagate the typed worker teardown
failure through the server owner so retry handles remain addressable, and append
a corrective DES cycle using only legal Lima/metal runners. Continue the
uncapped remediation/re-review loop until the reviewer returns **APPROVED**.

---

## Iteration 7 metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Step | `02-06` |
| Reviewed commit | `1b5cef000540330abbc3b7ed924e4193fc544420` |
| Parent | `bfcd39726ffa6f65deb7aad6bae36b25d3bad58c` |
| Subject | `fix(mtls): close reconstructed teardown boundaries` |
| Trailer | `Step-Id: 02-06` |
| Review iteration | 7 |
| Verdict | **NEEDS_REVISION** |

## Iteration 7 summary

D32 is closed at the composition boundary. `ServerShutdownError` now retains
both the typed worker teardown failure and the exact `Arc<MtlsInterceptWorker>`
that owns the shared fence and retry handles. Graceful and abrupt server paths
return it, the CLI preserves it as a typed source, and `retry(self)` either
converges the same owner or returns a replacement error carrying that owner.
All ordinary repository call sites were migrated to handle the new result; the
remaining ignored abrupt results are deliberate process-loss fixtures rather
than production shutdown. The focused server and worker retry contracts pass.

The runtime half of D31 also materially improves. Start and Restart install the
intercept before calling the driver, so VM spawn is behind the production
intercept call. The revised native fixture arms the exact tap/host-veth capture,
waits until the outbound rule is observed, and only then releases the held VMM
spawn. Four consecutive exact S-GTI-06a runs pass on the qualified metal host,
followed by a passing four-scenario S-GTI-06a/06b/12a/12b run. No pre-readiness
frame was observed in any of those five executions.

The step is nevertheless not complete. D30's new filesystem receipts use the
classic acknowledgement-before-effect ordering. Both the lifecycle consumer
and the production probe-hook consumer create their final receipt before they
perform the externally observed action. A crash in that interval leaves a
receipt which makes a replacement suppress an effect that never happened. The
lifecycle helper also creates the final path before writing and syncing its
contents, so an empty or partial final receipt is itself accepted as completion.
The acceptance fixture mirrors the same flawed algorithm and injects cuts only
before the consumer, not between receipt and effect. It also retains the real
`MtlsInterceptWorker` and does not drive the real production `ProbeRunner`
consumer. The green test therefore cannot establish no-skip/no-duplicate over
every cut of a fresh full production composition.

D31's cleanup half remains incomplete. Only the final pre-readiness-frame
assertion was moved after teardown. `observe_restarted_mesh_flow` still contains
many assertions and `expect` calls before it returns control to the owner of the
peer and server; the caller also asserts the returned allocation id before
natural completion and cleanup. Any one of those regressions can still unwind
past awaited peer stop and `ServerHandle::shutdown`, reproducing the 240-second
failure mode the remediation was required to eliminate.

The pre-spawn reorder introduces two adjacent lifecycle defects. A same-owner
restart asks `MtlsInterceptWorker::start_alloc` to tear down the prior intercept,
but that synchronous method merely spawns `begin_stop_alloc` and immediately
installs the replacement. It never waits for the prior rule guards, listeners,
tasks, and enforced handles to converge, while the action shim unconditionally
labels the baseline stable and releases the driver/EXEC. The existing re-fire
test observes one eventual rule on a two-thread runtime; it does not establish a
happens-before edge. Separately, an unclassified `Driver::start` failure returns
from both Start and Restart before the new intercept cleanup block. Production
`ExecDriver::start` can return exactly such a `DriverError::NetnsEntry`, so the
newly preinstalled guard/listeners remain owned after the start was refused.

Finally, D33 remains open. The latest append-only DES cycle is syntactically
ordered but records only `FAIL`, `PASS`, and `PASS`. It identifies no command,
selection, assertion, result count, run id, evidence artifact, or commit hash.
The committed review artifact predates this remediation cycle, and reflog can
prove commit chronology but not which RED/GREEN commands the crafter actually
ran. My independent legal nextest/metal runs verify the current tree; they do
not retroactively make the crafter's TDD phase log auditable.

## Iteration 7 remediation disposition

| Iteration 6 finding | Disposition | Evidence |
|---|---|---|
| D30 — process-resetting terminal-effect witnesses | **OPEN / superseded by D34** | Production now supplies persistent roots and fresh test consumers, but both real receipts commit before their effect. The fixture cannot cut inside either consumer and does not reconstruct the actual worker/probe composition. This changes duplicate risk into skip risk rather than proving exactly once. |
| D31 — unstable pre-reinstall native boundary and leaky failure path | **PARTIAL / still blocking through D35-D36** | Four exact native repeats and the mapped four-scenario run pass with no pre-readiness frame. The helper still asserts before awaited cleanup, and same-owner production re-fire does not await prior intercept teardown before release. |
| D32 — outer server loses typed retry owner | **CLOSED** | `ServerShutdownError` retains the typed failure and exact worker owner through graceful and abrupt server shutdown, CLI error mapping, repeat failure, and retry success. Focused composition-root 2/2 and worker-local 1/1 contracts pass. |
| D33 — illegal/untraceable DES evidence | **OPEN / still blocking** | The corrective cycle is append-only and uses no visible illegal command, but its three descriptions contain no command or behavioral evidence at all. Legality and right-reason RED/GREEN cannot be audited. |

## Iteration 7 retained finding disposition (D1-D29)

| Finding | Disposition |
|---|---|
| D1 — graceful shutdown mislabeled unclean | **CLOSED** — the mapped restart continues to revoke the server owner abruptly while retaining the workload process and durable intent. |
| D2 — target/workload substitution | **CLOSED** — target VM Job, allocation id, executable, intent, and data are unchanged; the peer remains a separately submitted Exec Service. |
| D3 — duplicate finalization effects | **OPEN through D34** — persistent keys exist, but receipt-before-effect permits a skipped hook/event and the fixture does not enumerate the consumer-internal cuts. |
| D4 — vacuous illegal/reclamation contracts | **CLOSED** — exact pure transition and one-claim/one-redrive properties remain independently green. |
| D5 — names and Contract Shapes | **CLOSED** — the seven mapped functions and required exact pure-function rustdoc declarations remain present. |
| D6 — incomplete/wrong RED | **OPEN through D33** — prior historical REDs remain, but the latest remediation cycle has no auditable behavioral RED. |
| D7 — partial cleanup fenced as complete | **OPEN through D34-D37** — terminal effects may be skipped, native assertions can bypass owner cleanup, same-owner intercept replacement is not awaited, and unclassified start failure strands the new intercept. |
| D8 — old userspace dataplane survives | **CLOSED** — successful shutdown joins tracked producers, and D32 now preserves typed failure plus retry authority on unsuccessful shutdown. |
| D9 — test-authored peer intercept | **CLOSED** — the peer is still reconstructed through production boot, without a test-only direct install seam. |
| D10 — illegal property preserves reopened row | **CLOSED** — READY, EXEC, and duplicate Finalize retain exact zero delta from the terminal pre-state. |
| D11 — false pure-function declaration | **CLOSED** — reclamation planning/lifecycle evidence stays synchronous and effect-free, distinct from bounded execution evidence. |
| D12 — default production lint failure | **CLOSED** — default and all-feature affected-package clippy pass with `-D warnings`. |
| D13 — Finalize crash exactness/event delivery | **OPEN through D34** — the durable outbox context survives, but a committed consumer receipt can suppress an event which was never broadcast. |
| D14 — Stop terminal-first cleanup | **PARTIAL through D34** — driver/network/mTLS teardown remains terminal-last and error-propagating; exact terminal hook/event delivery is still not crash-safe. D32 is closed. |
| D15 — stopped allocation children detach | **CLOSED** — allocation accept/enforce/pass-through/teardown children remain under owned task sets and the exact splice complement passes. |
| D16 — reusable owner fence and resolver port | **CLOSED** — dependency-neutral core placement, cancellation safety, shared waiters, resolver direction, and outer retry-owner propagation are now intact. |
| D17 — incomplete live recovery join | **CLOSED** — full survivor planning still precedes sweep and quarantine spans every later fallible boot gate. |
| D18 — wrong-reason RED | **CLOSED for the historical finding; the current cycle is untraceable under D33**. |
| D19 — terminal row cannot distinguish hook/event cuts | **OPEN through D34** — stable identities are present, but the receipt protocol cannot distinguish receipt-before-effect from effect completion. |
| D20 — terminal before authoritative cleanup | **CLOSED for terminal allocation cleanup** — Stop and Finalize await worker, driver, and network results before the durable terminal row. The pre-start failure leak is separately D37. |
| D21 — recovery removes last fail-closed rule | **CLOSED** — quarantine still precedes sweep, survives complete boot failure, and releases only after readiness. |
| D22 — shutdown completion orphaned/early | **CLOSED** — worker-local and server-level error paths now retain a reachable owner and shared retry fence. |
| D23 — claim-before-hook/event | **OPEN through D34** — outbox context and stable keys are durable, but the new driven consumers acknowledge before effect. |
| D24 — quarantine released before complete boot | **CLOSED** — production DNS-fault evidence and source ordering remain unchanged. |
| D25 — mapped S-GTI-06a native failure | **CLOSED for runtime ordering; cleanup remains D35** — four consecutive exact passes plus mapped 4/4 replace the prior 3/4 runtime result, but assertion-safe teardown is not complete. |
| D26 — journal error after Running | **CLOSED** — Start and Restart still prepare the journal route before driver start/Running and refuse cleanly on I/O failure. |
| D27 — worker-owner teardown failure is log-only | **CLOSED** — typed worker failure is shared, retained, propagated, and retryable through the server boundary. |
| D28 — behaviorally untraceable DES | **OPEN through D33** — the newest descriptions regress to bare status words. |
| D29 — detached splice cleanup | **CLOSED** — exact Tier-3 splice cleanup remains explicitly awaited and independently green. |

## Iteration 7 criterion disposition

| Criterion | Result | Evidence |
|---|---|---|
| S-GTI-06a | **PARTIAL / blocking** | Runtime oracle is now green in four consecutive exact metal runs and the mapped selection, with no pre-readiness frames. Assertion-safe teardown remains incomplete (D35), and the production same-owner replacement lacks an awaited exact-baseline edge (D36). |
| S-GTI-06b | PASS for the mapped native trace | The mapped run preserves same-id INPUT-hook rejection, no EXEC/frame, terminal Failed, and restoration; focused pre-start refusal cases pass 4/4. |
| S-GTI-12a | PASS for the mapped native trace | Exact target cleanup and sibling complement pass on qualified metal. |
| S-GTI-12b | **FAIL overall** | Native stop/idempotent cleanup trace passes, and typed outer shutdown is now retained, but D34 permits skipped terminal effects after a consumer-internal crash. |
| P-GTI-ILLEGAL-07 | PASS | Exact pure terminal-state rejection remains green. |
| C-GTI-RECLAMATION-ONCE | PASS | Same-epoch claim/redrive property remains green. |
| C-GTI-FINALIZE-TWICE | **FAIL** | The fixture passes its selected cuts, but does not cut after final receipt creation and before either real effect and does not compose the real production hook consumer (D34). |

## Iteration 7 findings

### D34 — durable receipts acknowledge terminal effects before performing them

- **Severity:** Critical
- **Dimensions:** Crash consistency, exactly-once effects, durable-port
  correctness, test external validity
- **Affected contracts:** C-GTI-FINALIZE-TWICE, S-GTI-12b, retained
  D3/D13/D19/D23/D30
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:832-890`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:981-1005`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1215-1223`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1336-1351`
  - `crates/overdrive-control-plane/src/lib.rs:684-707`
  - `crates/overdrive-control-plane/src/lib.rs:1749-1759`
  - `crates/overdrive-worker/src/probe_runner/mod.rs:99-108`
  - `crates/overdrive-worker/src/probe_runner/mod.rs:136-187`
  - `crates/overdrive-worker/src/driver.rs:834-849`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:91-175`
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:719-1001`

The production composition does now point the lifecycle event consumer and
`ProbeRunner` terminal hook at stable directories under `data_dir`. Stable
effect keys are also derived from allocation generation and terminal claim.
Those are necessary improvements, but the ordering at each driven boundary is
wrong for the required no-skip/no-duplicate contract.

`IdempotentLifecycleEventPort::emit_terminal_once` calls `consume`, which
creates and fsyncs the receipt, before `emit_broadcast`. A process cut after
line 882 returns `Ok(true)` and before line 883 sends leaves a durable receipt
without an event. A fresh port sees `AlreadyExists` and suppresses the only
replay. `TerminalEffectJournal::create_once` has an even earlier torn-record
window: it uses `create_new` directly on the final path, then writes/syncs. A
cut after final-path creation but before complete content and directory fsync
leaves an empty or partial final receipt that every replacement treats as
consumed.

`ProbeRunner::stop_alloc_idempotent` repeats the same protocol: final receipt
creation and sync occur at lines 160-166, then `stop_alloc` runs at line 175.
A cut between them suppresses the hook. Process death may incidentally destroy
that process's probe tasks, but it does not establish the general terminal-hook
contract or one logical external effect; the port itself claims exactly-once
consumption and accepts a receipt as proof of an effect it has not performed.

The acceptance test's reconstructed `ScriptedDriver` uses the same
receipt-before-counter algorithm. Its injected hook failure is immediately
before entering the consumer, and its lifecycle failure is before
`emit_terminal_once`; neither can cut after receipt creation and before effect.
The test recreates registries, scripted drivers, lifecycle channels/port, and
`AllocDriverIndex`, but it keeps one real `MtlsInterceptWorker`, one observation
owner, slot/network state, and allocator for the whole loop, and it never uses
the production `ExecDriver -> ProbeRunner::stop_alloc_idempotent` port. The
comment that every process-owned participant is reconstructed is therefore
false at the boundary that matters.

Writing the effect before the receipt would merely flip the unhandled cut from
skip to duplicate. The remediation needs a recoverable acknowledgement protocol
at the real effect boundary: for example an idempotent keyed downstream consumer
whose durable acknowledgement is authoritative, or a transaction/outbox scheme
that can reconcile prepared versus externally applied state. The test must
drive the real production consumers under a fresh complete composition and cut
on both sides of receipt preparation, effect application, and acknowledgement,
including torn/empty receipt publication. Every cut must converge to exactly
one logical hook and lifecycle projection.

### D35 — native assertions can still unwind before awaited fixture teardown

- **Severity:** Major
- **Dimensions:** Assertion-safe cleanup, test determinism, failure
  diagnosability
- **Affected contract:** S-GTI-06a
- **Evidence:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4064-4152`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:4272-4313`

The specific pre-readiness oracle now returns `pre_ready` to the caller, and its
assertion runs only after peer stop and server shutdown. That closes the exact
leak observed in Iteration 6. It does not satisfy the requested “move all
assertions behind an owned result” remediation.

Before returning, `observe_restarted_mesh_flow` still asserts/unwraps the VMM
allocation identity, network plan, slot parsing, rule-observer readiness, cut
release, reclamation row, peer Running state, kTLS installation, capture-loss
checks, guest address, D7 accounting, first SYN, plaintext at the guest
boundary, no plaintext at the peer, and bidirectional TLS records. Any of these
oracles can panic while `boot_two` and the independent peer owner are still held
only by the caller. The caller itself asserts `restarted.alloc_id` at line 4282
before natural-completion polling, peer stop, process release, and server
shutdown at lines 4284-4299. `WireCapture::Drop` now joins capture threads, but
it cannot asynchronously drain those server/workload owners.

Return a non-panicking observation result containing every oracle input and
error, synchronously stop/drain all fixture owners, and assert only afterward.
Then inject at least one early helper failure and prove the test returns within
its own bound instead of nextest's 240-second process timeout.

### D36 — same-owner intercept replacement is spawned, not awaited, before execution release

- **Severity:** Major
- **Dimensions:** Lifecycle ordering, exact kernel-rule baseline, first-flow
  determinism
- **Affected contracts:** S-GTI-06a, retained D7
- **Evidence:**
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:616-626`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:833-878`
  - `crates/overdrive-core/src/task_ownership.rs:66-100`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2776-2812`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:3270-3293`
  - `crates/overdrive-worker/tests/integration/start_alloc_installs_both_tproxy.rs:378-395`

`start_alloc` documents teardown-then-reinstall for a same allocation id, but
line 626 discards the `Arc<AllocStop>` returned by `begin_stop_alloc`. That
method removes the old record and uses `CompletionFence::start_with` to spawn an
independent future; old rule guards are dropped and old accept tasks joined
inside that future. Synchronous `start_alloc` immediately continues through new
listener/rule installation and returns without waiting for the fence.

The Start action then assigns `stable_exact_rule_baseline = true` without an
observation at line 2777. Restart is weaker: it logs successful pre-spawn
installation and directly invokes `release_for_exit_emission` without even the
helper predicate. A same-owner restart can therefore expose overlapping old and
new exact rules/listeners when the driver/guest is released. Both remain
fail-closed, but the first packet may be selected by the retiring rule and
listener rather than the new owner; this is not the promised stable exact-one
baseline.

The existing worker re-fire test calls synchronous `start_alloc` on a
two-thread Tokio runtime and dumps nft immediately. A scheduler opportunity
often lets the spawned stop complete during the blocking replacement install,
but there is no synchronization requiring it. Add a controllable old teardown
fence and prove replacement install/release cannot pass it. The production API
must await prior allocation teardown, or atomically adopt/replace the exact rule
owner without an overlapping retiring listener, before reporting readiness.

### D37 — unclassified driver-start failure bypasses cleanup of the preinstalled intercept

- **Severity:** Major
- **Dimensions:** Fail-closed lifecycle cleanup, error totality, remediation
  regression
- **Affected contracts:** retained D7/D20 adjacency and D-MTLS-18 pre-start
  fail-closure
- **Evidence:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2490-2530`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2582-2624`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2994-3043`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:3092-3127`
  - `crates/overdrive-worker/src/driver.rs:500-552`

Moving intercept installation before `driver.start` requires every non-success
driver outcome to release that new owner. Classified `StartRejected` and typed
start-cleanup failures flow through the `state != Running` cleanup blocks.
`Err(other)` with no `start_cleanup_failure`, however, returns immediately at
lines 2595-2598 and 3105-3108. Those returns precede `worker.stop_alloc`, so the
new rule guards, listeners, and tasks remain registered even though no Running
row or release is produced.

This is not an unreachable trait shape. Production `ExecDriver::start` returns
`DriverError::NetnsEntry` when opening the allocation netns or executing
`setns` fails. The old C3 cleanup behavior on that branch may be pre-existing,
but the remediation has newly added a live mTLS owner before it. Treat every
driver error as a total pre-start transaction: stop the installed intercept and
release the provisioned C3 owner before returning, while preserving the typed
driver error and any typed cleanup failure. Add Start and same-id Restart tests
for a real unclassified error, asserting no process, no intercept record/rule,
no listener/task, no Running/EXEC, and retryable error disposition.

### D33 — the corrective DES cycle is still not behaviorally auditable

- **Severity:** Major
- **Dimensions:** TDD evidence traceability, command legality, commit audit
- **Evidence:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/execution-log.json:873-893`
  - commit reflog entries for `c82d2fa3fca6fc91bcae9e6dbff8ad8b1490f703`
    and `1b5cef000540330abbc3b7ed924e4193fc544420`

The append-only shape is legal and timestamps are ordered RED `18:10:53Z`,
GREEN `18:43:16Z`, COMMIT `18:43:40Z`. The initial commit landed at
`18:43:31Z` and the seven-line log-only amend at `18:43:45Z`, with the required
subject, parent, and trailer. None of that identifies the TDD evidence: the
descriptions are only `FAIL`, `PASS`, and `PASS`.

There is no committed Iteration 7 evidence artifact to which those entries
refer. The previous review cannot support tests for code written afterward, and
the current source cannot prove which command was used in RED, whether RED
failed for the intended assertion, whether GREEN used Lima-wrapped nextest and
the qualified metal wrapper, or what commit scope was accepted. Append another
corrective RED/GREEN/COMMIT cycle naming each complete legal outer command,
exact selection, right-reason failure/pass result, count, run id, and committed
evidence path or commit identity. History must remain append-only.

## Strong evidence retained after Iteration 7

- Production lifecycle and probe consumers now receive deterministic receipt
  roots under `data_dir`; no test-only default is used at the production
  composition root. D34 concerns the transaction protocol, not missing wiring.
- Start and Restart prepare the terminal-effect route before driver start, and
  intercept install now precedes driver/VMM spawn. The mapped VMM cut is held
  until exact capture and outbound-rule readiness are armed.
- Four consecutive qualified-native S-GTI-06a executions pass without a
  pre-readiness frame, and the subsequent canonical S-GTI-06a/06b/12a/12b
  selection passes 4/4.
- `ServerShutdownError` preserves typed nested failure and same-owner retry
  capability through control-plane and CLI layers. All normal in-repository
  shutdown call sites handle the result. The API return-type change is a
  necessary source-level change for D32, not a wire/API schema change.
- `CompletionFence` and `OwnedTaskSet` remain dependency-neutral in
  `overdrive-core`; resolver ownership stays outside `MtlsResolve`; no frozen
  public persistence/wire shape changed.
- Recovery quarantine, terminal-last allocation cleanup, exact splice cleanup,
  pure illegal/reclamation properties, and the retained 02-05 ownership
  interactions remain intact under the focused and broad verification.

## Iteration 7 scope and boundary audit

The remediation commit changes 32 files with 1,197 insertions and 411
deletions; 375 inserted lines are the committed Iteration 6 review. The
cumulative 02-06 range changes 49 files with 8,000 insertions and 833 deletions.
The 32-file expansion is largely mechanical fallout from the fallible public
server shutdown return and the new required receipt-root constructor argument.
Every normal shutdown caller now asserts or propagates the result. The one
blocking Drop implementation cannot return an async error and deliberately
discards it; the mapped abrupt-cut fixtures likewise model irrevocable process
owner loss. No production caller silently converts shutdown failure to success.

| Boundary | Result |
|---|---|
| Built-product boundary | PASS — no Rust integration test spawns the built Overdrive production binary |
| Example/expectation separation | PASS — no root example or `verification/expectations` file changed; Rust tests remain in-process/native fixtures |
| E08/E09 | PASS — neither acceptance path is changed by the remediation or cumulative code diff |
| Legacy/no-token | PASS — no legacy, no-token, or bypass path is introduced |
| Service-plus-VM | PASS — target remains a VM Job; peer remains a separate Exec Service and is not combined into one forbidden fixture |
| Frozen wire/persistence/API shapes | PASS — no REST/OpenAPI, Beacon, rkyv, observation schema, or Cargo manifest changed |
| Public Rust API fallout | PASS with intentional source change — fallible shutdown is criterion-required, typed, and all ordinary repository callers migrated |
| Production receipt-root wiring | PASS structurally — both roots are mandatory at production composition; transactional correctness fails D34 |
| Generic ownership placement | PASS — `OwnedTaskSet` remains dependency-neutral core and resolver ownership remains outside the domain port |
| 02-05 interaction | PASS — target/peer production composition, persistent CA/trust, DNS/quarantine, and VM lifecycle ownership remain distinct |
| D7/06/12 mapped boundaries | **PARTIAL** — native traces pass, but D34-D37 keep crash effects, cleanup safety, and exact re-fire ordering open |
| OpenAPI exclusion | PASS honestly — the known stop-response `workload_addr`/`workload_id` drift is reproduced, but no API/OpenAPI source is in either remediation diff |
| Mutation discipline | PASS — no per-step mutation run and no mutation exclusion edit |

## Iteration 7 broad-gate investigation

The canonical affected all-feature Lima command completed with
`--no-fail-fast`. Nextest run
`ab103a55-863c-4b76-b307-96c1541568c9` selected all 2,154 tests: 2,084
passed, 70 failed, and 19 were skipped. No failure executes the new D30-D32
logic and then reports a remediation assertion failure.

| Category | Count | Classification |
|---|---:|---|
| `NestedAppleHost` native preflight refusal | 66 | KVM/native CLI scenarios are intentionally unqualified inside virtualized Lima. Canonical metal evidence replaces them. |
| Host-global nft fixture preconditions | 3 | Unchanged fault fixtures lack the required host-global production object/rename interruption state; no corresponding source is changed here. |
| Checked-in OpenAPI drift | 1 | Known `/v1/workloads/{id}/stop` `workload_addr` versus `workload_id` mismatch. Neither the remediation nor cumulative step changes API/OpenAPI files. |

The previously intermittent unchanged `LOCAL_BACKEND_MAP` walking skeleton
passed in this run, which explains 70 rather than Iteration 6's 71 failures.
The broad result remains red as an aggregate; its exact classifications do not
replace the focused passing evidence and do not excuse D34-D37.

## Iteration 7 DES and commit chronology

`execution-log.json` and the roadmap parse, and the full integrity verifier
reports complete phase shapes for all nine steps. The latest cycle's timestamps
are monotone. Reflog records initial implementation commit
`c82d2fa3fca6fc91bcae9e6dbff8ad8b1490f703` at `20:43:31+02:00`, followed by
an execution-log-only amend to reviewed commit `1b5cef00` at
`20:43:45+02:00`. `git diff --check` passes for both the remediation and
cumulative ranges, and the reviewed commit carries the required trailer.

The mechanical history is recoverable. D33 remains because phase descriptions
do not identify any behavioral command or committed evidence, so a reviewer
cannot audit command legality or right-reason transition from the log.

## Iteration 7 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 6691bb67..1b5cef00` | PASS |
| execution-log and roadmap JSON parse | PASS |
| `cargo xtask dst-lint` | PASS |
| Full DES integrity verifier with required `PYTHONPATH` | PASS — all 9 steps have complete syntactic phase shapes |
| Focused Finalize/Stop terminal acceptance selection | PASS — 2/2; nextest run `13d3bc2a-c3e1-4d54-88f4-26d9b413024e`; D34 explains the untested receipt/effect cuts |
| Focused graceful/abrupt server failure propagation | PASS — 2/2; nextest run `67335a18-dc2b-4231-88f4-637b1ece4ea9` |
| Focused worker-local retained-handle retry | PASS — 1/1; nextest run `be8e859c-f657-472e-a712-6aef47baadbc` |
| Focused pre-start intercept refusal paths | PASS — 4/4; nextest run `823c5033-315a-4b3d-91d4-7ae7144ca37e` |
| Focused P-GTI-ILLEGAL-07 and C-GTI-RECLAMATION-ONCE | PASS — 2/2; nextest run `94fe8fa3-fd55-4584-bf48-ac7e214b157a` |
| Exact canonical Tier-3 splice scenario | PASS — 1/1; nextest run `9f366cde-8f88-4d3c-b17e-b82ac19a02df` |
| Default-feature affected-package clippy with `-D warnings` | PASS for core, worker, control-plane, reconcilers, and CLI |
| All-feature affected-package clippy with `-D warnings` | PASS for the same five packages |
| Broad canonical privileged Lima affected run | FAIL — 2,084/2,154 passed; exact 70-failure classification above |
| Exact qualified-native S-GTI-06a repeat 1 | PASS — run `d00d0004-8001-4815-926d-5e388cf79d99`; 34.054s |
| Exact qualified-native S-GTI-06a repeat 2 | PASS — run `11209616-f3bd-4a37-82d6-639905b265ed`; 34.078s |
| Exact qualified-native S-GTI-06a repeat 3 | PASS — run `b4246107-8024-4cf8-95d4-095ff5488919`; 34.101s |
| Exact qualified-native S-GTI-06a repeat 4 | PASS — run `655c437b-cd2b-42c7-977a-e92d0330d22e`; 34.050s |
| Qualified-native S-GTI-06a/06b/12a/12b selection | PASS — 4/4; nextest run `86c5fdf7-6d7d-4458-b7a4-4e8593663012`; 84.386s |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

An initial attempted native invocation omitted the required
`OVERDRIVE_METAL_KERNEL` and `OVERDRIVE_METAL_ROOTFS` preflight variables and
failed before test execution; it is excluded from evidence. The four recorded
repeats used the canonical metal wrapper and qualified kernel/rootfs, and the
first synchronized source while later `--no-sync` runs passed source identity
checks.

## Iteration 7 verdict

**NEEDS_REVISION.** Do not complete step 02-06 or advance the DELIVER wave.
Return D33-D37 to the original 02-06 crafter. The next iteration must replace
receipt-before-effect with a genuinely recoverable no-skip/no-duplicate
protocol exercised through fresh real production consumers at every crash cut;
move every S-GTI-06a assertion behind synchronous fixture teardown; await or
atomically replace the prior same-owner intercept before driver/EXEC release;
clean the preinstalled intercept on every driver-start error; and append a
behaviorally traceable DES cycle containing only complete legal Lima/metal
commands and committed evidence. Continue the uncapped remediation/re-review
loop until the reviewer returns **APPROVED**.
