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
