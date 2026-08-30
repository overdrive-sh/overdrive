# Adversarial review — step 02-05

- **Feature:** `guest-stack-transparent-mtls-intercept`
- **Step:** `02-05` — Truthful fresh and pre-READY Rust failure closure
- **Reviewer:** `nw-software-crafter-reviewer` (fresh isolated reviewer)
- **Review ID:** `code_rev_20260830_02_05_iteration_1`
- **Iteration:** 1
- **Reviewed commit:** `e882ec57b8775f860e3e5c03459266bed2dcdf65`
- **Parent:** `408f5feb7a320c79dd204596160adbfc244d6cf2`
- **Subject:** `feat(guest-stack-mtls): close pre-ready failure paths`
- **Trailer:** `Step-Id: 02-05`
- **Verdict:** **NEEDS_REVISION**

## Executive summary

Step 02-05 cannot advance. The qualified native selection is reproducible and
green: all six selected S-GTI-05/S-GTI-08/supporting cases passed on the
native x86_64 KVM host. The three focused worker cases also pass. Those green
results do not establish the approved contracts, because several of their
oracles can false-pass and the only new ownership behavior is incomplete.

Two production defects are blocking. Duplicate VM ownership is returned as
the generic `Unclassified` fallback and distinguished only by matching display
text, contrary to both the typed-rejection criterion and the repository's
distinct-error rule. Failed-start cleanup discards every termination, cgroup,
run-directory, clone, and index-removal error and then releases the allocation
claim. A failed cleanup can therefore make residue unowned and admit a later
start onto the same allocation.

The acceptance evidence is also incomplete. The C4a case issues its duplicate
only after the original VM is fully `Live`, never while the first creation is
in flight, and cannot detect an extra leaked VMM or replacement of the
attempt-owned rootfs/listener artifacts. The native cleanup helper treats a
duplicate, malformed, or unreadable target nft rule as absence. The S-GTI-08a
poll can miss a transient `Running` row and observes no Beacon history, so it
does not prove the required absence of READY and guest EXIT. The resolver
rootfs mutator can leak its mount and loop device on any intermediate error or
panic.

Finally, there is no independently auditable RED. The parent contains ignored
scaffolds and no new C4a/cleanup tests, while the only 02-05 RED event records
`PASS` with no command or fail-for-the-right-reason evidence. The final commit
contains tests and production together, so the missing RED cannot be
reconstructed from commit history.

## Review scope and evidence integrity

The review covered the complete parent-to-target diff, all eight executable
mappings owned by 02-05, the approved DESIGN/DISTILL Q7 failure, diagnostic,
cleanup, interruption, replay, and C4a contracts, the DES log, and the relevant
production ownership/cleanup paths. The worktree already contained user-owned
untracked instruction and prior-review files; none was reset, discarded, or
committed.

This reviewer changed only this Markdown review artifact. No implementation,
test, expectation, DES, or evidence file was edited, and no mutation testing
was run.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 2 |
| High | 3 |
| Medium | 1 |
| Low | 0 |

## Mechanical evidence

### Commit, scope, and process

| Check | Result | Evidence |
|---|---|---|
| Exact parent | PASS | `e882ec57^` is `408f5feb`. |
| Commit scope | PASS | 5 files, 1,081 insertions, 18 deletions; changes are confined to the VM driver, worker/CLI tests, fault fixture, and DES log. |
| Trailer | PASS | Exact `Step-Id: 02-05`. |
| Formatting/whitespace | PASS | `cargo fmt --all -- --check` and the exact parent-to-target `git diff --check` both pass. |
| EDD boundary | PASS | No E08/E09 expectation or evidence exists in the step diff. |
| Rust/product boundary | PASS | New Rust tests use the in-process production composition root and real external fixtures; they do not spawn the built Overdrive product or invoke an expectation runner. |
| Mutation discipline | PASS | No mutation run or mutation-exclusion edit occurred. |
| DES phase order | FAIL | RED, GREEN, and COMMIT are chronological, but the RED event is only `PASS`; there is no recorded failing mapped test or intermediate commit from which a real RED can be reconstructed. |

### Executable mapping and Contract Shape

| Mapping | Structural result | Semantic result |
|---|---|---|
| S-GTI-05 | PASS | Native behavior passes, including the real INPUT-hook `EOPNOTSUPP` chain and zero guest frames. |
| S-GTI-08a | PASS | **FAIL:** polling and final LWW state do not prove the negative READY/Running/EXIT history. |
| S-GTI-08b | PASS | Native exit-78 behavior passes and reaches a prior Running row, ordinary exit 78, and zero restart count. |
| C-GTI-DIAGNOSTIC-TOTALITY | **FAIL** | The named function exists in private worker source, not in the canonical mapped control-plane test file; its focused behavior passes. |
| C-GTI-FAILED-START-CLEANUP | PASS | **FAIL:** the target-rule absence oracle converts malformed/duplicate/error states to `None` and does not inspect the full nft/FIB complement. |
| M-GTI-INTERRUPT-BOOT | PASS | The real external termination executes and passes, but shares the incomplete cleanup and negative-event oracles. |
| C-GTI-FAILED-START-CLEANUP-TWICE | PASS | Focused behavior passes for two successful cleanup runs; cleanup-error outcomes are not driven. |
| C4a-ATTEMPT-RESOURCE-DUPLICATE-CREATE | PASS | **FAIL:** the duplicate is sequential against `Live`, the result is generic `Unclassified`, and the resource complement is incomplete. |

Every newly claimed function carries an exact bounded-change declaration and
an Outcome Anchor. The declarations do not compensate for incomplete
preservation/delta oracles.

## Findings

### D1 — no fail-for-the-right-reason RED exists for step 02-05

- **Severity:** Blocker
- **Dimension:** DES/TDD integrity
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/execution-log.json:615-620`
  - parent `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:2789-2807`

The parent has ignored panic scaffolds for S-GTI-05 and S-GTI-08a and has none
of the new C4a, cleanup-twice, exit-78, or interruption tests. Ignored scaffolds
are not an active failing acceptance test. The only new RED event says
`d: "PASS"` and records neither a command nor a mapped assertion failure. There
is one final implementation commit and no intermediate RED commit.

This cannot establish the canonical RED gate: a mapped test must execute,
fail for the intended missing behavior rather than compilation or fixture
failure, and be preserved as real DES evidence before GREEN.

**Required remediation:** the original crafter must execute a genuine mapped
RED through the legal nextest/native boundary, record only the phase it
actually executed with the DES launcher, then implement/remediate and append a
fresh GREEN and COMMIT cycle. Do not hand-edit the log and do not treat an
ignored scaffold or a successful RED-phase command as the failing behavioral
observation.

### D2 — duplicate ownership is stringly typed, and C4a never drives in-flight creation or the complete resource complement

- **Severity:** Critical
- **Dimension:** Ownership correctness, typed errors, C4a test integrity
- **Locations:**
  - `crates/overdrive-worker/src/vm_driver.rs:1047-1058`
  - `crates/overdrive-worker/tests/acceptance/vm_driver_start_failure_contract.rs:476-529`

The atomic `BTreeMap::entry` check is the right synchronization primitive, but
its occupied branch calls `start_rejected_unclassified`. `Unclassified` is the
documented unknown fallback, not a distinct duplicate-ownership cause. The
test then asserts that fallback and searches `failure.detail` for “already has
an active VM”. Callers cannot match this failure mode without parsing text.

The test also starts target and sibling fully through READY and EXEC release
before issuing the duplicate. It proves only rejection against `Live`; it does
not hold the first start in `Starting` and race a second request against the
attempt-owned creation window. Its complement checks only the original two
PIDs remain live, cgroup-map equality, and run-directory existence. `SimVmm`
exposes no total process set or create count here, so a late-rejected duplicate
that spawns a third VMM can pass. Rootfs clone/index identity, beacon listener
ownership, and exact before/after attempt artifacts are not compared, so
replacement or cross-adoption can also pass.

This does not close `C4a-ATTEMPT-RESOURCE-DUPLICATE-CREATE` or raise the
delivery audit from 14/15 to 15/15.

**Required remediation:** add a distinct typed VM ownership-conflict variant
with structured allocation identity. Drive a deterministically barriered
duplicate while the first request is in `Starting`, plus the already-`Live`
case. Compare the complete VMM process/create set, run directory and beacon
listener, rootfs clone/index identity, cgroup, claim, and sibling state before
and after rejection; then prove ordinary cleanup removes exactly the original
owners.

### D3 — failed-start cleanup discards authoritative failures and releases ownership over possible residue

- **Severity:** Critical
- **Dimension:** Production cleanup safety and diagnostic precedence
- **Locations:**
  - `crates/overdrive-worker/src/vm_driver.rs:975-1006`
  - `crates/overdrive-worker/src/vm_driver.rs:383-402`
  - `crates/overdrive-worker/src/vm_driver.rs:1415-1446`
  - `crates/overdrive-worker/src/vm_driver.rs:2237-2275`

`cleanup_after_start_failure` returns `()` and discards `Vmm::terminate`,
`cgroup_kill`, cgroup removal, and run-directory removal results. The rootfs
helper logs non-`NotFound` clone failure and returns without surfacing it; its
index-link removal is likewise best-effort. Cleanup then unconditionally calls
`release_claim`.

A termination, cgroup, clone, or directory failure can therefore leave a live
VMM or allocation artifact while removing the only supervision claim. A later
start can acquire the same allocation and create the cross-ownership condition
the new entry check is meant to prevent. This directly contradicts the step's
“cleanup errors remain authoritative” criterion.

The diagnostic totality test injects six console-reader outcomes but uses only
successful cleanup doubles. It proves that diagnostics do not replace the
original rejection on the happy cleanup path; it does not prove precedence or
ownership when cleanup itself fails.

**Required remediation:** make failed-start cleanup return structured cleanup
results, preserve and surface every non-benign error, and do not release the
claim while allocation residue may still be owned. Add fallible termination,
cgroup, clone/index, and run-directory partitions proving that diagnostic
selection never obscures the cleanup failure and that no second owner can be
admitted.

### D4 — the native cleanup oracle accepts corrupt or duplicate nft residue as absence

- **Severity:** High
- **Dimension:** Cleanup completeness and false-green resistance
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1137-1188`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1886-1927`

`outbound_rule_snapshot` returns `None` for either a failed rule dump or any
`exact_d7_target` error. `exact_d7_target` errors when there are two matching
rules, when the target program is malformed, or when its counter is absent.
`assert_failed_vm_cleanup` then treats that same `None` as proof that the target
rule is absent. Duplicate or corrupt target residue and observer failure are
therefore green outcomes.

The helper also claims route and nft cleanup in its timeout message without a
complete route/FIB observation or normalized before/after nft snapshot. It
checks one projected rule and allocation-named filesystem/link facts, not the
full bounded complement required by S-GTI-08a and M-GTI-INTERRUPT-BOOT.

**Required remediation:** separate “complete observation succeeded” from
“zero raw allocation-owned matches”. Fail closed on dump/decode ambiguity and
on any malformed or duplicate allocation-tagged rule. Capture and compare the
complete normalized nft/FIB state, with the exact target delta removed and
every pre-existing production/sibling object, order, program, handle,
userdata, and counter preserved.

### D5 — S-GTI-08a can miss Running and does not observe the forbidden READY or guest EXIT events

- **Severity:** High
- **Dimension:** Negative lifecycle evidence and acceptance-test honesty
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1931-1976`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:2006-2030`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:3311-3416`

`poll_until_failed_without_running` samples a last-write-wins describe row
every 50 ms. A transient `Running` observation can be written and superseded
between samples. The final assertion checks restart accounting and the VMM
exit class but never requires `started_at == None`, even though S-GTI-05
explicitly demonstrates that this field retains a prior Running publication.

The capture proves zero guest-originated L2 frames, but no observer records the
Beacon messages. A guest that sends READY or an illegal pre-READY EXIT and
still ends in the same final VMM class is not directly rejected by the test.
The “forbidden exec” binary attempts one TCP connection and exits; it does not
produce the separately required operator marker or a lifecycle trace proving
no EXEC release.

**Required remediation:** retain an exact lifecycle/Beacon event history for
the bounded test and assert no READY, Running, EXEC, or guest EXIT event, plus
`started_at == None` in the final public/durable projection. Use an independent
operator-action marker whose absence is observable even when networking is
unavailable.

### D6 — the resolver failure fixture is not error- or signal-safe

- **Severity:** High
- **Dimension:** Native fixture operational safety
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:271-312`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:432-436`

`replace_resolver_with_directory` attaches a loop device, mounts it, mutates
the image, then unmounts and detaches only at the end. Every `expect`, assert,
I/O error, panic, test cancellation, or process signal between attachment and
the final two commands leaks host-global mount/loop state. There is no guard,
watchdog, or recovery journal for this new fixture.

The nft fault fixture has a watchdog, but its Rust `Drop` path discards the
`stop_and_wait` result during unwind. Only the normal `finish` path surfaces a
failed restoration and compares the final baseline. Thus the adopted S-GTI-05
path also lacks authoritative Rust-side restoration evidence after an earlier
assertion fails.

**Required remediation:** place loop attachment/mount ownership in an
assertion-safe guard or watchdog before the first mutation, unmount and detach
on every error/panic/signal path, and surface cleanup failure. Make fault
fixture unwind restoration authoritative rather than discarded, while
retaining the existing exact final nft/FIB equality check.

### D7 — the canonical diagnostic mapping does not resolve at its declared file

- **Severity:** Medium
- **Dimension:** Executable traceability
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/roadmap.json:479-487`
  - `crates/overdrive-control-plane/tests/integration/exit_observer_stderr_tail.rs`
  - `crates/overdrive-worker/src/vm_driver.rs:2237-2275`

The roadmap maps `diagnostic_selection_is_total_and_never_masks_rejection_or_cleanup`
to the control-plane integration file. That file has no such function. The
function instead lives as a private source-local worker test. The private seam
is technically appropriate for injecting the console reader, but the
canonical executable locator is false and cannot be mechanically followed by
a reviewer or runner.

**Required remediation:** align the canonical mapping with the actual private
worker boundary, or provide the declared executable at the mapped file without
duplicating implementation logic. Revalidate the locator after the correction.

## Passing boundaries and behavior

The review found the following sound behavior, which remediation should
preserve:

- the VM supervision claim uses one locked `entry` check before run-directory
  creation;
- the real S-GTI-05 product path reaches the typed install-stage projection
  with `append-egress` → `append-rule` → `EOPNOTSUPP`, final Failed, zero
  restart count, zero captured guest frames, and normal fixture restoration;
- the console reader is asynchronous, bounds reads to the final 8 KiB, retains
  five line fragments including an unterminated tail, uses lossy UTF-8, and
  gives nonempty guest console precedence over VMM stderr;
- real resolver failure and external pre-READY termination reach
  `VmGuestExitUnreported`, preserve exact observed VMM process facts, do not
  consume restart budget, and preserve the observed sibling row/rule;
- post-READY exit 78 reaches the ordinary Job failure result with exact exit 78
  and no restart consumption;
- no E08/E09 expectation or evidence was added, and examples, Rust tests, and
  black-box expectations remain at their distinct boundaries.

## Independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| Exact parent-to-target `git diff --check` | PASS |
| Focused worker C4a, cleanup-twice, and diagnostic selection via Lima nextest | PASS — 3/3; 206 skipped |
| Qualified native S-GTI-05/S-GTI-08a/S-GTI-08b/cleanup/interruption/wrong-hook selection | PASS — 6/6; 258 skipped; 58.279s |
| Broader Lima workspace suite with `integration-tests` | 2,865 passed, 1 failed, 27 skipped; the sole OpenAPI stop-parameter drift is inherited from the exact parent and untouched by this step |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

The broader run's only failure remains the existing generated OpenAPI
`workload_addr` versus checked-in `workload_id` stop-path divergence. The exact
02-05 diff changes neither the OpenAPI sources nor `api/openapi.yaml`, so it is
not counted as a step-attributable defect. The supplied impacted-suite count is
not independently recoverable from the DES event because no command or output
was recorded; the reviewer therefore ran the broader workspace selection plus
the exact focused/native mappings.

## Iteration 1 verdict

**NEEDS_REVISION.** D1-D7 remain open. Do not begin step 02-06. Return the
findings to the original 02-05 crafter, preserve all existing dirty work, and
cycle remediation and re-review until every production, oracle, fixture, DES,
and mapping defect is closed and the reviewer returns **APPROVED**.

# Iteration 2 re-review

- **Review ID:** `code_rev_20260830_02_05_iteration_2`
- **Iteration:** 2
- **Reviewed commit:** `bbd23ea031c3c28b9ae7c50cfc2ec34a59f0a404`
- **Parent:** `e882ec57b8775f860e3e5c03459266bed2dcdf65`
- **Subject:** `fix(guest-stack-mtls): close step 02-05 review findings`
- **Trailer:** `Step-Id: 02-05`
- **Verdict:** **NEEDS_REVISION**

## Iteration 2 executive summary

The remediation materially improves the step. The C4a test now holds the
first creation behind a deterministic barrier, rejects duplicates in both
`Starting` and `Live` with a typed allocation identity, compares the exact
attempt-owned trees/cgroups/claims/create count, and proves ordinary cleanup.
The native failure cases now compare the full normalized nft/FIB baseline,
fail closed on corrupt or duplicate tagged rules, retain an independent
operator-action marker, and use the durable `started_at` and transition edge
to rule out a transient public `Running`. The diagnostic locator is correct,
the fixture mutations are guarded by watchdogs, and a fresh chronological DES
cycle records RED as `FAIL` before GREEN and COMMIT.

The step still cannot advance. Two errors that are correct at the private
driver seam are composed incorrectly at the production action boundary. A
cleanup failure is introduced as a new variant on the exhaustive public
`DriverError` enum, breaking the frozen Rust API, and the action shim does not
project or retry it. It leaves the in-memory ownership claim held; a later
start is then converted to the duplicate `StartRejected`, which can overwrite
the authoritative cleanup failure with a generic durable
`DriverInternalError`. Independently, any duplicate start delivered through
the real action shim writes `Failed` for the allocation already owned by the
first start/VM, so a rejection intended to protect a healthy owner can corrupt
that owner's lifecycle row.

Two evidence defects also remain. S-GTI-08a has no Beacon observer, so a guest
that sends a forbidden pre-READY `EXIT` frame can still satisfy every current
assertion. The resolver watchdog owns the mount and loop device, but both
`restore` and `signal_and_wait` remove the only `Child` handle before fallible
stop/signal/wait operations; an error therefore defeats the guard and can
leave a watchdog and host-global state behind.

## Iteration 1 finding dispositions

| Finding | Iteration 2 disposition | Evidence |
|---|---|---|
| D1 — no real RED | **CLOSED** | The appended 02-05 cycle is chronological (`RED FAIL` at `02:43:56Z`, GREEN at `03:28:29Z`, COMMIT at `03:29:08Z`). The remediated C4a assertion is mapped and rejects the old generic duplicate result for the intended ownership reason. The historical bad RED remains intact as audit history. |
| D2 — stringly/sequential C4a | **PARTIALLY CLOSED; D9 remains** | `VmStartFailure::AllocationAlreadyOwned { alloc }` is typed and API-compatible because `VmStartFailure` was already `#[non_exhaustive]`. The barriered `Starting` and subsequent `Live` attempts compare exact claims, cgroups, run/clone/index trees, process liveness, and VMM create count. The real action-shim composition still authors `Failed` for the allocation whose existing owner was protected. |
| D3 — cleanup failures swallowed | **PARTIALLY CLOSED; D8 remains** | Every supplied cleanup stage is attempted, non-benign stage failures are collected, the primary error is retained, and ownership is not released on residue. The new outer error breaks the frozen public enum and is not composed through the action/retry/reclamation path, where its authority is subsequently lost. |
| D4 — cleanup oracle false-pass | **CLOSED** | Observation failure, malformed ownership, and duplicate ownership are distinct errors; only zero raw allocation-tagged matches is absence. Resolver and interruption cases compare the complete normalized nft/FIB baseline, including pre-existing sibling state. |
| D5 — negative lifecycle evidence | **PARTIALLY CLOSED; D10 remains** | `last_transition.from == None` and durable `started_at == None` exclude a transient public Running row; the independent console/rootfs marker excludes execution of the operator binary. No observer records guest Beacon messages, so forbidden guest `EXIT` remains unproved. |
| D6 — unsafe native fixture | **PARTIALLY CLOSED; D11 remains** | The shell watchdog owns mount/loop cleanup before mutation and the executable tests cover normal finish, unwind, signal, and parent death. Rust-side error paths discard the only child handle before fallible operations complete. |
| D7 — false diagnostic locator | **CLOSED** | The roadmap now resolves to `crates/overdrive-worker/src/vm_driver.rs::diagnostic_selection_is_total_and_never_masks_rejection_or_cleanup`, the approved private injection seam. |

## Iteration 2 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 2 |
| High | 2 |
| Medium | 0 |
| Low | 0 |

## Iteration 2 findings

### D8 — cleanup authority is lost across the public action/retry composition, and the remediation breaks the frozen Rust API

- **Severity:** Critical
- **Dimension:** Public API compatibility, cleanup authority, convergence
- **Locations:**
  - `crates/overdrive-core/src/traits/driver.rs:211-229`
  - `crates/overdrive-core/src/traits/driver.rs:282-297`
  - `crates/overdrive-worker/src/vm_driver.rs:1014-1070`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1701-1708`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2077-2084`
  - `crates/overdrive-control-plane/src/vm_reclamation_boot.rs:109-118`
  - `crates/overdrive-reconcilers/src/vm_reclamation.rs:65-77`

`DriverError` is a public exhaustive enum. Adding
`DriverError::StartCleanupFailed` is therefore a source-breaking change for
every external exhaustive match, contrary to this step's explicit “preserve
frozen public and persistence shapes” instruction. The duplicate cause did
not make this mistake: it extends the already-`#[non_exhaustive]`
`VmStartFailure` enum. The cleanup representation also flattens each original
I/O/VMM source into `detail: String`, so its stage is typed but its source
chain is not preserved.

More importantly, both fresh-start and restart action-shim matches accept only
`StartRejected`; `StartCleanupFailed` takes `Err(other)` and returns
`ShimError::Driver` before writing a lifecycle row. The driver intentionally
retains a `Starting`/`Live` claim when residue may remain. That claim is
reported by `live_allocations`, so VM reclamation correctly refuses to touch
the allocation. On the next convergence attempt, `VmDriver::start` sees the
retained claim and returns `AllocationAlreadyOwned`; that ordinary
`StartRejected` is then persisted as generic `DriverInternalError`. The
authoritative primary-plus-cleanup composition has disappeared, the residue
has no in-process retry owner, and the allocation is stuck until process
restart.

The private helper test proves one all-fail vector and one index-only vector,
but never drives this actual action → retry → reclamation composition. Its
green result therefore cannot establish cleanup authority across the live
system.

**Required remediation:** preserve the existing public `DriverError` shape
(an extension under an already non-exhaustive typed VM start cause is one
compatible option), preserve structured sources, and give the production
action/convergence path an explicit cleanup-failure disposition. Add a real
composition test proving that the primary and every cleanup stage remain
authoritative after a subsequent tick, cleanup is retried or otherwise
recoverable, no second owner is admitted, and the error cannot be replaced by
the duplicate-conflict fallback.

### D9 — a typed duplicate rejection can mark its existing healthy owner Failed

- **Severity:** Critical
- **Dimension:** Duplicate ownership, lifecycle integrity, composition testing
- **Locations:**
  - `crates/overdrive-worker/src/vm_driver.rs:1107-1122`
  - `crates/overdrive-core/src/transition_reason.rs:331-334`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1690-1708`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2065-2084`
  - `crates/overdrive-worker/tests/acceptance/vm_driver_start_failure_contract.rs:557-674`

The driver-level rejection is now correctly typed and leaves every owner
unchanged. The production consumer, however, classifies it as an ordinary VM
start rejection, maps it to `TransitionReason::DriverInternalError`, and
writes `AllocState::Failed` for that same allocation. During the barriered
`Starting` race, the duplicate action can therefore publish `Failed` while
the original call continues toward Running. Against `Live`, it can publish
Failed while the original VMM remains healthy and supervised. Which writer
wins becomes a lifecycle race; either outcome contradicts the exact ownership
result the driver test claims.

C4a calls `VmDriver` directly, so none of its exact resource assertions can
detect this public-state corruption. Preserving the wire enum by using the
generic internal-error variant preserves bytes, but not behavior.

**Required remediation:** compose duplicate ownership as an operational
conflict that rejects the second author without writing a failure on the
existing allocation owner. Exercise both barriered `Starting` and already
`Live` duplicate actions through the real action shim and observation store,
proving exact row/event preservation together with the existing exact
resource complement.

### D10 — S-GTI-08a still has no oracle for the forbidden guest `EXIT` event

- **Severity:** High
- **Dimension:** Negative lifecycle/Beacon evidence
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:180-263`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:2223-2269`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:2298-2333`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:3626-3738`

The durable timestamp and first-edge assertions are a sound non-sampling
oracle for no public Running, and the new marker is an independent oracle for
no operator action. `FailureObservedVmm` observes only process ending, console
text, and the rootfs marker. Nothing in this integration test records the
guest's Beacon frames.

`accept_ready` rejects any first frame other than READY; a buggy guest that
sends `EXIT` before powering off can therefore still end in the same
`VmGuestExitUnreported` VMM class, with `started_at == None`, marker absent,
zero workload frames, and complete cleanup. Every current assertion would
pass despite violating S-GTI-08a's explicit “no guest EXIT” complement. The
post-READY exit-78 case does prove READY → Running → operator action → ordinary
exit 78, but it does not close the negative pre-READY event universe.

**Required remediation:** retain an observation-only Beacon trace at the real
VM boundary and assert the exact pre-READY event history contains neither
READY nor guest EXIT (and no EXEC release). Keep the durable lifecycle and
operator-marker assertions as independent evidence layers.

### D11 — the rootfs watchdog guard loses ownership before fallible restoration completes

- **Severity:** High
- **Dimension:** Native fixture error-path safety
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:401-428`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:451-463`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:501-560`

Both `signal_and_wait` and `restore` call `self.watchdog.take()` before their
first fallible operation. If `kill`, stop-file creation, or `wait` fails, the
`Child` is dropped and `self.watchdog` remains `None`. `Drop` then calls
`restore` again and receives `Ok(())`, even though the watchdog can still be
running and the mount/loop device can still be owned. In a caught unwind the
parent remains alive, so the watchdog's parent-death fallback does not rescue
the leak.

The new native test covers successful restoration reached by normal finish,
panic, a successfully delivered signal, and process death. It injects none of
the Rust-side stop/signal/wait failures that expose the ownership loss.

**Required remediation:** retain the child handle in the guard until stop,
wait, and detached-state verification have all completed (or transition to a
separate explicit recovery owner). Add injected stop/signal/wait error
partitions and prove exact pre-existing mount/loop state is restored without
an orphan watchdog.

## Iteration 2 mapping, boundary, and scope audit

| Gate | Result |
|---|---|
| Eight 02-05 executable mappings | PASS structurally; D8-D10 block semantic completion |
| Contract Shape declarations | PASS — new bounded-change tests are declared; live source-local pure properties retain the exact `/// CONTRACT_SHAPE: pure-function.` line |
| Diagnostic mapping | PASS — corrected to the actual private worker seam |
| E08/E09 boundary | PASS — no expectation or evidence was added |
| Rust/product boundary | PASS — Rust tests do not spawn the built Overdrive product or act as expectation runners; the rootfs safety case recursively invokes only its integration-test fixture process |
| Example/expectation/integration separation | PASS |
| Legacy/no-token bypass | PASS — none introduced |
| Unsupported service-plus-VM category | PASS — none introduced |
| Commit scope/trailer/diff check | PASS — exact parent, `Step-Id: 02-05`, and `git diff --check` |
| Mutation discipline | PASS — no mutation run or mutation-exclusion edit |

## Iteration 2 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| Exact remediation `git diff --check` | PASS |
| Lima focused worker diagnostic and structured-cleanup selection | PASS — 2/2 |
| Lima focused worker barriered C4a and cleanup-twice selection | PASS — 2/2 |
| Lima strict outbound-rule pure oracle | PASS — 1/1 |
| Qualified native S-GTI-05/S-GTI-08a/S-GTI-08b/cleanup/interruption/rootfs-watchdog selection | PASS — 6/6; 259 skipped; 58.429s |
| Broader Lima workspace suite with `integration-tests` | 2,865 passed, 2 failed, 27 skipped. The OpenAPI stop-parameter drift is inherited and untouched; the second failure is environmental `ENOSPC` in a trybuild archive, after the ordinary workspace tests compiled and ran. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

The first unqualified metal invocation correctly failed closed because the
required kernel/rootfs selections were absent. The reported native result is
from the subsequent canonical run with the qualified
`/srv/vm/overdrive-testing/{kernel,rootfs.ext4}` inputs and one retained metal
lease.

## Iteration 2 verdict

**NEEDS_REVISION.** D8-D11 are open. Do not begin step 02-06. Return these
findings to the original 02-05 crafter and continue the uncapped
remediation/re-review cycle until the cleanup and duplicate paths are correct
through the production action boundary, the negative Beacon complement is
directly observable, the native guard retains recovery ownership on every
error path, and the reviewer returns **APPROVED**.

---

# Iteration 3 re-review

## Metadata

| Field | Value |
|---|---|
| Step | `02-05` |
| Reviewed commit | `800ccbf2ae2c6393f6eb208f1de176f147e139fc` |
| Parent | `bbd23ea031c3c28b9ae7c50cfc2ec34a59f0a404` |
| Review iteration | 3 |
| Verdict | **NEEDS_REVISION** |

## Iteration 3 summary

The remediation closes the public-enum compatibility defect, the duplicate
owner's row/event corruption, the missing guest Beacon complement, and the
watchdog `Child` ownership loss. The source-preserving cleanup carrier and
single-disposition retry protocol are materially stronger, and the focused
and native selections are green.

The cleanup composition is still unsafe at the production reclamation
boundary. An *incomplete* cleanup is immediately persisted as
`AllocState::Failed` while `VmDriver` deliberately retains the cleanup record
and supervision claim. `Failed` is terminal. The reclamation plan shared by
boot and periodic convergence therefore takes the terminal-row exemption and
ignores supervision. During the live process, the periodic sweep can execute
`DiscardStrandedArtifacts`, killing the scope and deleting the run
directory/rootfs artifacts underneath the active cleanup owner. This
contradicts the exact theorem on which the exemption is based and reopens
competing teardown authorship. D8 is consequently not substantively closed;
D12 records the remaining Critical defect.

## Iteration 3 disposition of D8-D11

| Finding | Disposition | Evidence |
|---|---|---|
| D8 — cleanup/API/action/retry composition | **PARTIALLY CLOSED; D12 remains Critical** | `DriverError` again has its frozen five variants, and an external-crate exhaustive match compiles. `DriverStartCleanupError` rides the pre-existing `Io` variant, preserves the primary source plus typed per-stage source objects, changes no persisted schema, serializes attempts with one in-flight recovered disposition, retries failed disposition writes, and releases only after the recovered row commits. The initial incomplete-cleanup row is nevertheless terminal and authorizes reclamation against the retained owner. |
| D9 — duplicate owner can be marked Failed | **CLOSED** | Both Start and Restart return `Ok(())` for the exact typed same-allocation conflict before a row write or event. The barriered action test proves byte-exact Pending/Starting and Running/Live row/event preservation. The real `VmDriver` C4a test independently preserves the complete cgroup/run/clone/index/process/claim complement for both phases; the existing C3 converge oracle remains a no-op over a complete VM network plan. |
| D10 — no direct pre-READY Beacon complement | **CLOSED** | `overdrive-init` now records successful guest-boundary READY and EXIT sends and EXEC receipt. The real-VMM decorator captures the console before clone deletion. Native S-GTI-08a observes exactly `{ready:0, exec:0, exit:0}`; the independent post-READY case observes exactly `{1,1,1}`, so the negative oracle is not vacuous. Lifecycle durability and the rootfs/operator marker remain separate evidence layers. |
| D11 — watchdog loses the only `Child` | **CLOSED** | `signal_and_wait` and `restore` retain `self.watchdog: Some(Child)` through signal/stop, wait, exit validation, and detached-state verification, clearing it only after all stages succeed. Injected stop, signal, wait, and verification partitions on ordinary and signal paths retry successfully; panic/unwind, readiness-deadline unwind, successful signal, and parent death retain the shell fallback. The qualified native watchdog scenario passed. |

## Iteration 3 API, persistence, and authority audit

| Surface | Result |
|---|---|
| Exhaustive `DriverError` source compatibility | PASS — no new enum variant; the external integration-test crate exhaustively matches `StartRejected`, `NotFound`, `Io`, `NetnsEntry`, and `ResizeUnsupported` |
| Cleanup source preservation | PASS — every `DriverCleanupFailure` owns an `Arc<dyn Error + Send + Sync>` and exposes it without flattening; the primary `DriverError` remains the standard source |
| Persisted/wire compatibility | PASS — no observation, transition-reason, rkyv, serde, REST, or OpenAPI cleanup field was added; the action writes the existing `DriverInternalError` plus existing diagnostic detail |
| Retry serialization | PASS locally — the per-allocation async mutex serializes cleanup attempts, `disposition_in_flight` admits one recovered outcome, and a failed observation write returns that slot through the additive defaulted trait hook |
| Action authority | PASS only after recovery — a recovered disposition holds ownership until its Failed row commits; an incomplete disposition is written before recovery and creates D12 |
| Reclamation authority | **FAIL** — terminal-row disposal bypasses supervision, so the retained incomplete-cleanup owner is not authoritative across the live system |

## D12 — an incomplete cleanup's Failed row authorizes reclamation against its retained owner

- **Severity:** Critical
- **Dimension:** Cleanup/reclamation authority and competing teardown
- **Locations:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1724-1737`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1794-1808`
  - `crates/overdrive-worker/src/vm_driver.rs:1074-1117`
  - `crates/overdrive-worker/src/vm_driver.rs:1136-1166`
  - `crates/overdrive-worker/src/vm_driver.rs:1892-1905`
  - `crates/overdrive-core/src/traits/observation_store.rs:221-223`
  - `crates/overdrive-reconcilers/src/vm_reclamation.rs:153-184`
  - `crates/overdrive-reconcilers/src/vm_reclamation.rs:346-360`
  - `crates/overdrive-control-plane/src/action_shim/reclamation.rs:229-253`
  - `crates/overdrive-control-plane/src/vm_reclamation_boot.rs:105-137`
  - `crates/overdrive-core/tests/acceptance/vm_reclamation_plan_purity.rs:141-148`

When rollback leaves residue, `cleanup_after_start_failure` keeps both the
`PendingStartCleanup` entry and the VM supervision claim. The action recognizes
the typed carrier but unconditionally constructs and commits a `Failed` row,
even when `cleanup.recovery_complete()` is false. `release_supervision` then
correctly refuses to drop the incomplete record. Locally, the driver still
looks authoritative.

Across the production composition it is not. `AllocState::is_terminal`
classifies `Failed` as terminal, and `hydrate_vm_reclamation_desired` copies
that value directly into `VmAllocFacts.terminal`. The terminal arm of
`plan_reclamation` explicitly does **not** consult
`reclamation_authorised`; it emits `DiscardStrandedArtifacts` under the stated
theorem that a terminal-row instance can never still be claimed. The existing
property fixture even pins this behavior by inserting a held terminal
allocation and expecting disposal.

The new remediation makes that theorem false. At the 30-second periodic
sweep, the executor can call `kill_scope` and `discard_artifacts` while
`VmDriver` still owns and retries VMM termination, rootfs clone/index removal,
cgroup cleanup, and run-directory removal. The result is two teardown
authorities racing over the same artifacts. The boot path confirms the same
unguarded terminal-disposal plan after a process restart, when the in-memory
cleanup owner no longer exists; the live competing-owner defect is the
periodic path. The `release_supervision` guard cannot help because terminal
reclamation bypasses the supervision predicate before the release hook is
involved.

The new lifecycle test does not expose this defect. Its title says the next
tick observes retained ownership, but it dispatches another Start/Restart
action against a fake driver; it never hydrates `VmReclamation`, runs
`plan_reclamation`, or executes the discard action. The worker test likewise
stays within `VmDriver` and manually repairs its fault partitions before
retrying.

**Required remediation:** establish one end-to-end authority rule. Do not make
an incomplete-cleanup disposition appear terminal to VM reclamation, or amend
the reclamation model so a retained cleanup claim gates this exact state and
cannot reach `DiscardStrandedArtifacts`. Preserve the compatible public error
carrier and source chain. Add a production-composition test that leaves real
cleanup residue, commits or attempts the action disposition, hydrates both VM
reclamation surfaces during the incomplete interval, and proves no kill or
discard occurs while the cleanup owner is retained. Then prove retry to
residue-free state, durable disposition, claim release, and only thereafter
the appropriate reclamation behavior. A driver-only retry and an action-only
row assertion are not substitutes for this joined test.

## Iteration 3 D1-D7 regression audit

| Prior dimension | Result |
|---|---|
| D1 — lifecycle/evidence placement | PASS — no expectation or example boundary moved; native scenarios still drive command libraries and the real VM substrate |
| D2 — exact packet-path complement | PASS — no oracle weakening in the remediation diff; qualified failure scenarios retain exact independent-allocation and nft/FIB baselines |
| D3 — diagnostic source/order | PASS — typed primary and cleanup sources are retained and rendered only at the existing diagnostic edge |
| D4 — duplicate driver resources | PASS — real `VmDriver` exact Starting/Live complement remains green; action row/event preservation is now additive |
| D5 — negative lifecycle evidence | PASS — durable no-Running, operator-marker absence, and exact Beacon `{0,0,0}` are independent |
| D6 — native fixture safety | PASS — shell ownership begins before mutation and Rust retains the `Child` through every fallible completion stage |
| D7 — DES/verification honesty | PASS — fresh RED→GREEN→COMMIT events append after history; JSON is valid and the amend chronology is recoverable from retained commit objects/reflog |

## DES and commit audit

The fresh RED event at `2026-08-30T03:56:25Z` records the external exhaustive
match rejecting the prior `StartCleanupFailed` variant. GREEN follows at
`04:36:35Z`, then COMMIT evidence at `04:39:55Z`, then the final-amend event at
`04:40:14Z`. Commit `3440916c` is retained and contains the pre-log-amend
implementation. Reflog retains the log-only amendments `354529ef` and final
`800ccbf2`; the final committer time matches the last DES event. Each amendment
changes only `execution-log.json`. The final commit has the exact parent,
conventional subject, `Step-Id: 02-05`, nine-file scope, and no tracked dirty
state. `git diff --check` and JSON parsing pass.

## Broad-suite failure isolation

| Failure | Classification | Evidence |
|---|---|---|
| OpenAPI checked-file drift | **Inherited, deterministic, non-target** | Independent focused reproduction fails at `/v1/workloads/{id}/stop` (`workload_addr` live versus `workload_id` on disk). The same failure was present in Iteration 2's parent suite, and this remediation changes neither `api/openapi.yaml` nor the OpenAPI/API declaration files. |
| Two kTLS RX `ENOTCONN` sentinel failures | **Transient environment/concurrent-suite failure, not a target regression** | The remediation has zero diff in the two worker scenarios, dataplane mTLS implementation, and dataplane integration tests. The two affected worker tests were run serially five times each on the same Lima kernel: 10/10 passed. Their production probe and kTLS RX sentinel are unrelated to the changed VM cleanup/action/init paths. This is not a persistent inherited failure, but the broad-run occurrence is conclusively outside the 02-05 diff. |

The broad suite is therefore not globally green, but neither failure supplies
evidence against the target behavior. D12 is the target-owned blocker.

## Iteration 3 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| Exact remediation `git diff --check` | PASS |
| `execution-log.json` parse | PASS |
| Focused API/action/worker cleanup and duplicate selection | PASS — 7/7; 1,830 skipped |
| Qualified native Beacon/rootfs/failure selection | PASS — 6/6; 259 skipped; 71.398s |
| Serial kTLS isolation, initial run | PASS — 2/2 |
| Serial kTLS isolation, four repetitions | PASS — 8/8; total 10/10 |
| Focused OpenAPI checked-file gate | FAIL — inherited checked-YAML drift, isolated above |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

## Iteration 3 boundary and scope audit

| Gate | Result |
|---|---|
| E08/E09 | PASS — neither exists and no expectation/evidence file changed |
| Built-product boundary | PASS — Rust tests do not spawn the built Overdrive production binary; the watchdog recursively invokes only its integration-test fixture process |
| Example/expectation/integration separation | PASS |
| Legacy/no-token path | PASS — none introduced |
| Unsupported Service-plus-VM category | PASS — none introduced |
| Contract Shape declarations | PASS — new executable scenarios carry the required declaration; no live pure-function property lost its exact declaration |
| Mutation exclusions/discipline | PASS — no exclusion edit and no per-step mutation run |
| Unrelated work | PASS — pre-existing untracked review/AGENTS files remain untouched; tracked tree was clean before this review artifact append |

## Iteration 3 verdict

**NEEDS_REVISION.** D9-D11 are closed, and most of D8's compatibility,
source, retry, and action work is correct. D12 leaves the live
cleanup/reclamation composition Critical: a terminal Failed row authorizes a
second teardown owner while the first owner is explicitly retained. Do not
begin step 02-06. Return D12 to the original 02-05 crafter and continue the
uncapped remediation/re-review cycle until the joined action→reclamation
boundary proves one teardown authority and the reviewer returns
**APPROVED**.
