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
