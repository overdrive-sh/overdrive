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

---

# Iteration 4 re-review

## Metadata

| Field | Value |
|---|---|
| Step | `02-05` |
| Reviewed commit | `58360435c31af3bc097b19cd80927381389f1c23` |
| Parent | `800ccbf2ae2c6393f6eb208f1de176f147e139fc` |
| Review iteration | 4 |
| Verdict | **NEEDS_REVISION** |

## Iteration 4 summary

The remediation closes D12's competing-teardown race at both layers that need
to participate. Terminal and non-terminal planner rows now consult the same
supervision predicate, and each kill-capable executor takes an atomic claim
against the real `VmDriver` supervision map before host I/O. The joined tests
show that an existing cleanup owner defeats periodic planning, boot
convergence, and stale already-planned actions; the inverse interleaving shows
that a reclaimer which wins first prevents a concurrent start. A fresh process
can adopt residue, preserves the original diagnostic detail, and releases its
claim after reclamation. These are substantive and correct safety repairs.

The new non-terminal disposition has no live-system retry producer, however.
An incomplete cleanup is written as `Pending`, but `WorkloadLifecycle` retries
only `Failed`, `Terminated`, and `Draining` allocations. A `Pending` row falls
through to fresh placement, and the scheduler deliberately excludes it from
capacity. Production therefore mints a different allocation instead of
calling `VmDriver::start` for the retained allocation, which is the only entry
to `retry_pending_start_cleanup`. VM reclamation correctly refuses the old
held allocation forever. Under a persistent fault the loop can accumulate one
new `Pending` allocation, retained claim, and host residue set per attempt
until node/network capacity is exhausted.

The new composition test hides this by manually injecting a same-allocation
`RestartAllocation` that the production reconciler never emits for that row.

D12 is consequently safer but not closed: the double-owner failure has become
a live-process leak/retry and capacity-exhaustion failure. D13 records the
remaining Critical defect.

## Iteration 4 disposition of D8-D12

| Finding | Disposition | Evidence |
|---|---|---|
| D8 — cleanup/API/action/retry composition | **PARTIALLY CLOSED; D13 remains Critical** | The compatible carrier, source chain, serialized per-allocation attempt, write-failure retry slot, and post-commit release remain correct. The production lifecycle never selects that retry slot after the new `Pending` write. |
| D9 — duplicate owner can be marked Failed | **CLOSED** | The exact typed duplicate still returns `Ok(())` before row/event mutation. Both Starting and Live preservation tests remain green, and the new reclaimer-first test extends the same rule across the reclamation lease. |
| D10 — no direct pre-READY Beacon complement | **CLOSED** | No Beacon oracle was weakened. The qualified native selection again observed the pre-READY and post-READY complements and passed 6/6. |
| D11 — watchdog loses the only `Child` | **CLOSED** | No watchdog ownership code regressed; the qualified rootfs watchdog scenario passed with the other five native cases. |
| D12 — incomplete cleanup permits competing reclamation | **PARTIALLY CLOSED; D13 remains Critical** | `Pending` prevents terminal classification, terminal rows now honor supervision, stale executors re-check a real atomic lease, and restart adoption works. During the original live process, no production action retries the retained cleanup, so residue and ownership do not converge. |

## Iteration 4 D12 safety and interleaving audit

| Interleaving | Result |
|---|---|
| Incomplete cleanup → periodic hydration | PASS for exclusion — desired is non-terminal, actual reports the retained claim, and the planner emits nothing |
| Incomplete cleanup → stale precomputed reclaim/discard action | PASS — `try_begin_reclamation` sees `pending_cleanup`/`live` and both executors return a total no-op before host I/O |
| Reclaimer claim → concurrent start | PASS — the same `live` map contains `EndingInFlight`; the real start returns the exact duplicate-owner result without invoking `Vmm::create` or changing the row |
| Recovered cleanup → Failed write | PASS — the claim remains held across the terminal write and is released only after commit; a write failure returns the single disposition slot |
| Terminal row → planner during write/release interval | PASS — terminal disposal now also requires `reclamation_authorised`, so the still-held interval emits nothing |
| Executor error/cancellation | PASS for exclusivity — `ReclamationLease::Drop` releases on every return/unwind/cancellation path, permitting a later retry rather than leaking the execution claim |
| Process crash with Pending row and residue | PASS — the fresh empty driver can claim and reclaim once, keeps the old error detail, writes `PlatformReclaimed`, and releases |
| Live process with Pending row and retained cleanup | **FAIL — D13**; WorkloadLifecycle emits a fresh allocation, not a same-allocation cleanup retry, while reclamation must continue refusing the retained owner |
| Stop/intent withdrawal during retained Pending cleanup | **FAIL — D13**; the stop/GC branches select only Running rows, so neither path supplies the missing cleanup retry or release |

The lock ordering itself is sound. `try_begin_reclamation` holds
`pending_cleanup` while performing the `live` check-and-insert;
`release_supervision` takes the same order and does not create an interval in
which a retained cleanup record is absent while its live claim can be stolen.
The executor lease is also used by both steady-state action dispatch and the
boot drive. The blocker is not another atomicity defect; it is the absence of
an action producer that ever re-enters the retained cleanup protocol.

## D13 — Pending cleanup is never retried by production lifecycle and leaks across replacement attempts

- **Severity:** Critical
- **Dimension:** Cleanup liveness, bounded resources, production-composition honesty
- **Locations:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1724`
  - `crates/overdrive-worker/src/vm_driver.rs:1073`
  - `crates/overdrive-worker/src/vm_driver.rs:1470`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:858`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1078`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1124`
  - `crates/overdrive-reconcilers/src/vm_reclamation.rs:175`
  - `crates/overdrive-control-plane/tests/acceptance/vm_cleanup_reclamation_authority.rs:480`
  - `crates/overdrive-core/tests/acceptance/first_fit_place_branches.rs:248`

On an incomplete cleanup, the action now commits a `Pending` row and
`VmDriver` retains both `PendingStartCleanup` and the `Starting` supervision
entry. That is a valid safety state only if something later retries the same
allocation. The only cleanup retry entry point is
`VmDriver::retry_pending_start_cleanup`, called from `Driver::start` with the
requested allocation id.

The production lifecycle does not request that allocation again. Its
restartable predicate includes `Terminated | Draining | Failed`, not
`Pending`. After all earlier branches miss, a `Pending` row reaches the fresh
placement branch. `scheduler::free_capacity` intentionally counts only
`Running` rows; the existing acceptance scenario
`placement_excludes_non_running_allocs_on_same_node` explicitly proves that a
Pending row permits `StartAllocation`. Fresh placement derives its id from
the number of existing rows, so it produces the next allocation id rather
than the retained one. `VmDriver::start(new_id)` cannot find or retry
`pending_cleanup[old_id]`.

The other convergence path is intentionally unavailable. VM reclamation sees
the old Pending row as non-terminal and the old claim as held, so it emits no
action; a stale executor independently refuses the same claim. Operator stop
and absent-intent GC select only Running rows. There is therefore no live
transition from this state to cleanup recovery, durable Failed, or release.
A process crash happens to heal it because the in-memory owner disappears,
but requiring a crash is not convergence.

With a persistent partition, each fresh placement can fail in the same way,
write another Pending row under a new id, and remain excluded from scheduler
capacity. The level-triggered runtime can therefore accumulate clone
indexes/directories, run/cgroup residue, observation rows, supervision
claims, and network-slot ownership without backoff until some finite host or
slot capacity is exhausted. This violates the required no-leak and eventual
single-disposition properties even though it no longer permits two cleanup
owners at once.

The new joined test does not expose the production defect. After repairing
the injected filesystem partition it directly calls
`dispatch_action(..., restart_action(&h), ...)`; it never hydrates and
reconciles `WorkloadLifecycle` from the Pending row. That manually supplied
same-allocation action is exactly the missing behavior under review.

**Required remediation:** give the retained cleanup a real production retry
producer while preserving the one-owner and post-durable-release rules. Drive
the actual workload-lifecycle/runtime composition from the incomplete row and
prove that it retries the retained allocation rather than minting a new one;
then prove residue-free cleanup, the original diagnostic's durable Failed
disposition, claim release, and subsequent reclamation behavior. Also prove a
persistent cleanup partition remains bounded and stop/intent withdrawal does
not strand the retained owner. A manually injected `RestartAllocation` is not
a substitute for the production transition.

## Iteration 4 D1-D7 regression audit

| Prior dimension | Result |
|---|---|
| D1 — lifecycle/evidence placement | PASS — no example/expectation boundary changed; the added test is an in-process integration test over production crates |
| D2 — exact packet-path complement | PASS — no packet oracle changed and the native S-GTI selection remains green |
| D3 — diagnostic source/order | PASS — the live retry and fresh-restart tests retain the original primary/cleanup detail; reclamation forwards prior detail instead of erasing it |
| D4 — duplicate driver resources | PASS — the real driver and action-level exact-complement scenarios remain green, and the executor lease extends exclusion to plan/dispatch races |
| D5 — negative lifecycle evidence | PASS — no Running/marker/Beacon negative assertion was weakened |
| D6 — native fixture safety | PASS — no guard ownership path changed; the qualified watchdog and failure selections passed |
| D7 — DES/verification honesty | PASS — the fresh RED fails on the old terminal disposition for the intended reason; GREEN and COMMIT follow chronologically, with the pre-log-amend commit retained in reflog |

## Iteration 4 API, persistence, and test-boundary audit

| Surface | Result |
|---|---|
| Frozen `DriverError` enum | PASS — remains the same five variants and the external exhaustive-match witness compiles |
| `Driver` trait compatibility | PASS — `try_begin_reclamation` is additive with a conservative default, so existing implementations still compile; the real VM implementation opts in |
| Persisted/wire/API schema | PASS — no rkyv/serde/observation/REST/OpenAPI field or enum variant changed; Pending, Failed, detail, and PlatformReclaimed are existing values |
| Cleanup diagnostics | PASS on exercised action/restart/crash paths — primary and per-stage rendering remains in the durable detail; restart reclamation forwards it |
| Test production composition | **FAIL for liveness only — D13**; real adapters and shims are used, but the recovery action is manually manufactured instead of being produced by WorkloadLifecycle |
| E08/E09 | PASS — neither exists and no expectation/evidence file changed |
| Built-product boundary | PASS — Rust tests do not spawn the Overdrive production binary or act as expectation runners |
| Example/expectation/integration separation | PASS |
| Legacy/no-token path | PASS — none introduced |
| Unsupported Service-plus-VM category | PASS — none introduced |
| Contract Shape declarations | PASS — all three new executable tests carry the required bounded-change declaration; live pure-function properties retain the exact declaration |
| Mutation discipline | PASS — no exclusion change and no per-step mutation run |

## DES and commit audit

The new RED event at `2026-08-30T05:05:20Z` describes the joined real-driver
test observing the old terminal Failed state where the new test expected a
retryable non-terminal disposition. The test compiles against the parent and
fails at that intended assertion before the remediation. GREEN follows at
`05:20:47Z`; COMMIT follows at `05:23:24Z`. Reflog retains implementation
commit `941a11a6` at `05:22:03Z` and the final log-bearing amend
`58360435c` at `05:23:24Z`. The final commit has exact parent `800ccbf2`,
conventional subject `fix(guest-stack-mtls): serialize cleanup reclamation`,
and exact `Step-Id: 02-05` trailer. JSON parsing, formatting, and exact diff
checks pass.

## Broad-suite failure isolation

The crafter's Iteration 4 event records focused GREEN but makes no false claim
that the entire workspace is green. The reviewer independently reproduced the
same checked-OpenAPI failure as Iterations 2 and 3: live
`/v1/workloads/{id}/stop` contains `workload_addr` where the checked YAML has
`workload_id`. This commit changes neither OpenAPI source declarations nor
`api/openapi.yaml`, so the deterministic failure remains inherited and
non-target. No kTLS failure occurred in this review's selected runs; the
Iteration 3 10/10 isolation remains the relevant classification for the prior
broad-run transient.

## Iteration 4 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| Exact remediation `git diff --check` | PASS |
| `execution-log.json` parse | PASS |
| New D12 composition + cleanup disposition + Pending lifecycle selection | PASS — 6/6; 1,624 skipped; includes the existing Pending→fresh-placement witness that exposes D13 |
| Reclamation decision-table/supervision properties | PASS — 3/3; 837 skipped |
| Frozen API, structured cleanup, duplicate-owner, and disposition retry selection | PASS — 8/8; 1,832 skipped |
| Qualified native Beacon/rootfs/failure selection | PASS — 6/6; 259 skipped; 71.391s |
| Focused OpenAPI checked-file gate | FAIL — inherited deterministic YAML drift, isolated above |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

The first unqualified metal invocation failed closed on missing selected guest
artifacts. The reported native result is from the canonical qualified run with
`/srv/vm/overdrive-testing/{kernel,rootfs.ext4}` and a single metal lease.

## Iteration 4 verdict

**NEEDS_REVISION.** The terminal/non-terminal supervision gate and
execution-time reclamation lease close the destructive D12 race. D1-D7 and
D9-D11 remain closed; D8 remains blocked only by the cleanup liveness defect.
D13 is nevertheless Critical: the new Pending disposition has no live
production retry transition, so the sole cleanup owner and its artifacts can
remain forever and persistent failures can accumulate fresh allocations until
capacity exhaustion. Do not begin step 02-06. Return D13 to the original 02-05
crafter and continue the uncapped remediation/re-review cycle until the actual
lifecycle drives same-allocation cleanup recovery and the reviewer returns
**APPROVED**.

---

# Iteration 5 re-review

## Metadata

| Field | Value |
|---|---|
| Step | `02-05` |
| Reviewed commit | `f9c5c59e46200d7ea610cb08bac2d2d99cd3fefd` |
| Parent | `58360435c31af3bc097b19cd80927381389f1c23` |
| Review iteration | 5 |
| Verdict | **NEEDS_REVISION** |

## Iteration 5 summary

The remediation closes D13 for the ordinary live-process, single-owner path.
`WorkloadLifecycle` now recognizes the durable failed-start-cleanup `Pending`
row before fresh placement, emits `RestartAllocation` for that exact allocation
id, persists a one-second retry timestamp before dispatch, and deliberately
does not increment the workload restart counter. `VmDriver::stop` re-enters the
serialized retained cleanup; the restart shim skips network provisioning and
`Driver::start` when that cleanup remains incomplete or has just recovered.
Persistent cleanup failure therefore remains one row, one driver claim, one
host residue set, and one original VMM create. Operator stop and intent
withdrawal use the analogous same-allocation `StopAllocation` path and preserve
the original diagnostic while cleanup is incomplete and when it authors the
ending. The new acceptance journeys enter through the real registered
`WorkloadLifecycle`, hydration, persisted View, validator, workflow-aware shim,
and production ports, replacing only the privileged network provisioner.

Two adversarial state transitions remain unsafe. First, a process can die after
a retained cleanup retry removes the residue but before the action writes the
authoritative `Failed` row. The durable row is still the old cleanup `Pending`,
but the fresh process has neither residue nor an in-memory cleanup record. The
next lifecycle retry treats `VmDriver::stop`'s `NotFound` as best-effort and
starts a new VM through the cleanup-only branch. That overwrites the durable
diagnostic without an ending and starts a workload process without charging the
restart budget. Second, the Run branch always selects the lexicographically
first cleanup `Pending` row. If that row is persistently partitioned, every due
tick retries it and refreshes its timestamp; later retained cleanup owners are
never selected, even when their cleanup is immediately recoverable.

D14 and D15 record these remaining blockers.

## Iteration 5 disposition of prior findings

| Finding | Disposition | Evidence |
|---|---|---|
| D1 — no real RED | **CLOSED** | The remediation RED is chronological and the new production-composition assertion distinguishes the parent behavior: it enters the registered lifecycle rather than manufacturing the missing restart action. |
| D2 — exact packet-path complement | **CLOSED** | No native packet oracle was weakened; the qualified six-case selection remains green. |
| D3 — cleanup diagnostic source/order | **PARTIALLY REOPENED by D14** | The live retry, stop/delete, observation-write retry, and residue-bearing restart paths preserve the original typed primary and cleanup history. A residue-free process crash can bypass the ending and overwrite that durable diagnostic with `Running`. |
| D4 — duplicate driver resources | **CLOSED** | Exact duplicate ownership remains a no-op before row/event mutation; the focused duplicate and reclaimer-first selections remain green. |
| D5 — negative lifecycle evidence | **CLOSED** | The pre-READY native cases still prove no READY/Running/EXEC complement and no assertion was removed. |
| D6 — native fixture safety | **CLOSED** | The qualified rootfs watchdog case passes with the five native failure/complement cases. |
| D7 — locator and verification honesty | **CLOSED** | The exact diagnostic test remains source-local and mapped; DES phase order and commit mechanics are sound. |
| D8 — cleanup/API/action/retry composition | **PARTIALLY CLOSED; D14 remains High** | Frozen public and persisted shapes, serialized live retries, disposition write retry, and post-commit release remain correct. Crash recovery after cleanup has already removed all residue can bypass the durable disposition. |
| D9 — duplicate owner can be marked Failed | **CLOSED** | Both Starting and Live duplicate action paths still return before observation or event mutation. |
| D10 — no direct pre-READY Beacon complement | **CLOSED** | Qualified S-GTI-08a and interruption cases remain green. |
| D11 — watchdog loses the only child | **CLOSED** | The recursive watchdog remains owner-safe on finish, unwind, signal, and parent death. |
| D12 — incomplete cleanup permits competing reclamation | **CLOSED for its stated teardown race** | Non-terminal and terminal planning honor supervision; stale actions acquire the executor lease; live and boot reclamation cannot compete with a retained owner. D14 is a later, residue-free crash transition rather than a second teardown owner. |
| D13 — production lifecycle never retries retained cleanup | **CLOSED for one owner; D15 exposes multi-owner starvation** | The real lifecycle now retries the same retained id with persisted backoff and no fresh id or crash-budget charge. Stop and delete also converge. The single-owner proof does not cover two retained rows. |

## D13 remediation audit

| Required property | Result |
|---|---|
| Real lifecycle producer | PASS — the test calls `run_convergence_tick_with_network_provisioner_for_test`, which delegates to the same registered reconciler, hydration, ViewStore, validation, workflow preflight, dispatch, and re-enqueue path as production |
| Only privileged adapter replaced | PASS — the seam substitutes `WorkloadNetworkProvisioner`; the real `VmDriver`, `RealVmHostState`, driver registry, action shim, reclamation planner/executor, intent store, and observation contract remain composed |
| Same retained allocation id | PASS — the lifecycle recognizes the durable `Pending + DriverInternalError` row before placement and builds `RestartAllocation` from that row's id and spec |
| Persisted bounded backoff | PASS — `last_failure_seen_at[id]` is written through before dispatch; `view_has_backoff_pending` keeps the target enqueued; the driver retains at most one latest failure per finite cleanup stage |
| Restart budget | PASS on live cleanup retries — `restart_counts` is not incremented and no new workload process starts while the retained cleanup record exists |
| Persistent partition | PASS for one owner — one row, claim, residue set, and original create remain bounded across repeated ticks |
| Recovery disposition | PASS without a process crash — cleanup recovery writes `Failed` with the original diagnostic before releasing supervision; a write failure reopens the single disposition slot |
| Stop and delete | PASS — both retry cleanup under the same backoff, never provision/start, preserve the original detail, author `Terminated`, release after commit, and clear View memory on confirmation |
| Reclamation concurrency | PASS — held cleanup excludes planning and stale actions; a reclaimer-first lease blocks a concurrent start; lease Drop covers error, unwind, and cancellation |
| Process restart with residue | PASS — boot reclamation adopts the residue once, preserves the durable detail, writes the ending, and releases |
| Process restart after residue-free recovery | **FAIL — D14** |
| Multiple retained cleanup owners | **FAIL — D15** |

## D14 — residue-free cleanup recovery can crash before disposition and restart without diagnostic or budget

- **Severity:** High
- **Dimension:** Crash consistency, diagnostic authority, restart accounting
- **Locations:**
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:679`
  - `crates/overdrive-control-plane/src/reconciler_runtime.rs:1661`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2083`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2105`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2125`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2162`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2316`
  - `crates/overdrive-worker/src/vm_driver.rs:1073`
  - `crates/overdrive-worker/src/vm_driver.rs:1110`
  - `crates/overdrive-worker/src/vm_driver.rs:1679`
  - `crates/overdrive-worker/src/vm_driver.rs:1739`
  - `crates/overdrive-control-plane/src/vm_reclamation_boot.rs:105`
  - `crates/overdrive-control-plane/tests/acceptance/vm_cleanup_reclamation_authority.rs:755`

The runtime correctly persists the cleanup retry timestamp before dispatch.
During dispatch, however, the restart shim awaits `driver.stop`. A successful
retained cleanup retry removes the final host residue and records only
process-local `recovery_complete + disposition_in_flight`. Before the shim can
write the `Failed` row it performs another awaited observation read, and the
later observation write is also an await. A process death at either boundary
loses the in-memory cleanup carrier while leaving the already-durable
`Pending + DriverInternalError` row and its original detail unchanged.

On the next boot, `VmReclamation` observes no clone, run directory, scope, or
other host residue, so it has nothing to adopt and writes no ending. The fresh
`VmDriver` also has no `pending_cleanup` entry. `WorkloadLifecycle` eventually
emits its cleanup `RestartAllocation` for the same id. The shim calls
`VmDriver::stop`, receives ordinary `NotFound`, absorbs it, provisions the
network, and calls `start`. A successful start then overwrites the old Pending
row with `Running`.

This is not merely transient loss of an intermediate state. The original
start rejection and cleanup history never reach a durable ending or
`last_terminated`. The lifecycle deliberately did not increment
`restart_counts` because this branch promised that no workload process would
start; after the crash, a new workload process does start through that branch.
The crash therefore bypasses both diagnostic authority and restart-budget
accounting.

The existing process-restart test covers the complementary state: its fresh
process still observes clone residue, so boot reclamation can adopt it. It
does not exercise cleanup success followed by process death before the
disposition write.

**Required remediation:** make a durable cleanup `Pending` row whose fresh
driver reports no retained owner and whose host observation has no residue
converge to an authoritative ending before any new start. Preserve the prior
diagnostic; only a later ordinary restart may start a workload and consume the
normal restart budget. Add a production-composition test that blocks/cancels
the retry after cleanup succeeds but before the `Failed` write, reconstructs a
fresh `AppState`, runs boot and the real lifecycle, and proves: no VMM create
before the ending, original detail retained, no lost row, and subsequent
restart accounting follows the normal failure path.

## D15 — the Run branch can starve every cleanup owner after the first

- **Severity:** High
- **Dimension:** Cleanup liveness, fairness, bounded ownership
- **Locations:**
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:669`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:679`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:682`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:687`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1437`
  - `crates/overdrive-control-plane/tests/acceptance/vm_cleanup_reclamation_authority.rs:454`

`WorkloadLifecycleState.allocations` is a `BTreeMap`. In the Run branch,
`active_allocs_vec.iter().find(...)` always returns the first cleanup Pending
row in allocation-id order. If that row is inside its backoff window, the
reconciler returns no action without scanning any later row. When it becomes
due, the reconciler retries the same first row and refreshes only that row's
timestamp. If its substrate failure is persistent, the cycle repeats forever.
A second cleanup Pending row is never selected even if it has no timestamp or
its cleanup would succeed immediately.

This state is meaningful at a convergence boundary: observation hydration can
contain more than one allocation row for a workload, and the cleanup protocol
claims each retained allocation independently. The stop/delete helper already
acknowledges this by scanning every Running or cleanup-Pending row and emitting
every due action. The Run helper instead turns the first retained owner into a
permanent head-of-line blocker. Later owners keep their claim and host residue
forever, and no current test supplies two cleanup Pending rows.

**Required remediation:** select every due retained cleanup owner, or use a
persisted fair selection that cannot let one persistent partition suppress
another allocation indefinitely, while still prohibiting fresh placement
until retained cleanup work is resolved. Add a two-owner test in which the
lexicographically first cleanup remains partitioned and the second is
recoverable; prove the second reaches exactly one authoritative ending and
releases its claim while the first remains bounded and retryable.

## Iteration 5 API, persistence, and boundary audit

| Surface | Result |
|---|---|
| Frozen `DriverError` and `Driver` API | PASS — this remediation changes no core trait, public error enum, or external exhaustive-match surface |
| Persisted/wire/REST/OpenAPI shape | PASS — no core type, observation schema, wire type, REST declaration, or checked OpenAPI file changed |
| Test seam exposure | PASS — the two added functions are hidden and compiled only for tests or the existing `integration-tests` feature; default production calls the unchanged wrapper |
| E08/E09 | PASS — neither exists and no expectation/evidence path changed |
| Built-product boundary | PASS — the new Rust tests do not spawn the built Overdrive binary, emit expectation evidence, or act as expectation runners |
| Example/expectation/integration separation | PASS |
| Legacy/no-token path | PASS — none introduced or broadened |
| Unsupported Service-plus-VM category | PASS — no new unsupported category is claimed; the D13 production journey is the roadmap's VM Job path |
| Contract Shape declarations | PASS — the two added executable tests and the rewritten production-composition test carry exact per-test `CONTRACT_SHAPE: bounded-change.` declarations; existing pure-function declarations remain exact |
| Mutation discipline | PASS — no mutation exclusion changed and no per-step mutation run occurred |
| Repository hygiene | PASS — the seven-file remediation is tightly related; pre-existing untracked AGENTS/review files remain untouched |

## DES and commit audit

The appended D13 cycle is chronological: RED `FAIL` at
`2026-08-30T06:02:22Z`, GREEN `PASS` at `06:17:11Z`, and COMMIT `PASS` at
`06:17:43Z`. The RED detail is terse, but the executable test change is honest:
the retained-cleanup journey now begins with the real workload tick, and the
parent implementation necessarily takes the D13 fresh-placement path rather
than the same-allocation retry asserted by the test. Reflog retains the initial
implementation commit `1926e367` at local `08:17:36 +0200` and the log-bearing
amend `f9c5c59e` at `08:17:48 +0200`. The final commit has exact parent
`58360435c`, conventional subject
`fix(guest-stack-mtls): retry retained cleanup in lifecycle`, and exact
`Step-Id: 02-05` trailer. JSON parse, format, exact remediation diff check, and
cumulative step diff check pass.

## Suite and failure classification

The deterministic checked-OpenAPI failure is unchanged: live
`/v1/workloads/{id}/stop` contains `workload_addr` where the checked YAML has
`workload_id`. The D13 remediation touches neither OpenAPI source declarations
nor `api/openapi.yaml`; this remains inherited and non-target. An attempted
workspace-wide `--all-features` focused selection also reached the no-std BPF
binary and failed during compilation because unwinding panics are unsupported;
the same eight requested tests pass under their correct core/worker/control
package and integration-feature selection. Neither result is evidence against
the target code. No broad green claim was added to the final D13 events.

## Iteration 5 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| Exact remediation and cumulative `git diff --check` | PASS |
| `execution-log.json` parse | PASS |
| Real lifecycle retained cleanup, stop/delete, reclamation, and residue-bearing restart selection | PASS — 5/5; 787 skipped |
| Frozen API, structured cleanup, duplicate-owner, disposition-write retry, and cleanup-twice selection | PASS — 8/8; 1,834 skipped |
| Reclamation decision-table and supervision properties | PASS — 3/3; 837 skipped |
| Qualified native Beacon/rootfs/failure selection | PASS — 6/6; 259 skipped; 58.922s |
| Focused OpenAPI checked-file gate | FAIL — inherited deterministic YAML drift, isolated above |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

## Iteration 5 verdict

**NEEDS_REVISION.** The normal live-process remediation is correct and closes
D13's original single-owner leak: the real lifecycle retries the same retained
allocation with durable backoff, no fresh allocation, no cleanup-time restart
charge, no duplicate teardown, and correct stop/delete behavior. D1-D2,
D4-D7, and D9-D13 otherwise remain closed. D14 still permits a residue-free
crash window to erase the authoritative cleanup diagnostic and start a new
workload without normal restart accounting; D15 permits one persistent cleanup
owner to starve every later owner. Do not begin step 02-06. Return D14 and D15
to the original 02-05 crafter and continue the uncapped remediation/re-review
cycle until both crash consistency and multi-owner liveness are proved and the
reviewer returns **APPROVED**.

---

# Iteration 6 — D14/D15 remediation re-review

## Metadata

| Field | Value |
|---|---|
| Step | `02-05` |
| Review iteration | 6 |
| Target commit | `35509f94fec54ffe3135356d6fc46742492cc89d` |
| Parent | `f9c5c59e46200d7ea610cb08bac2d2d99cd3fefd` |
| Subject | `fix(guest-stack-mtls): close cleanup crash and starvation` |
| Trailer | `Step-Id: 02-05` |
| Verdict | **APPROVED** |

## Iteration 6 scope and summary

This iteration reviewed the complete cumulative 02-05 implementation and the
five-file D14/D15 remediation. The remediation changes the production action
shim and workload lifecycle, expands the real production-composition cleanup
suite, appends the DES cycle, and commits Iteration 5's review artifact. Its
exact scope is 955 insertions and 32 deletions. It does not change the public
driver API, persisted or wire schema, REST/OpenAPI declarations, examples,
expectations, BPF programs, legacy/no-token behavior, or Cargo features.

Both remaining findings are closed. A retained cleanup can no longer pass
through the post-cleanup/pre-observation crash window into fresh placement:
after successful boot reclamation, a fresh process treats an authoritative
cleanup-Pending row plus a non-vacuous all-`NotFound` stop result as proof that
the old owner is gone, writes `Failed` with the original diagnostic, and lets
the normal Job lifecycle produce the single terminal ending. Multi-owner
cleanup selection now deterministically considers every retained owner on
each lifecycle evaluation, dispatches every due owner, durably backs off each
attempt, and continues to gate fresh placement while any retained cleanup
exists.

## Iteration 6 finding dispositions

| Finding | Disposition | Evidence |
|---|---|---|
| D1 — public `DriverError` shape | CLOSED | The cumulative implementation retains the frozen core enum and uses the private structured cleanup carrier. The D14/D15 remediation does not touch the core trait or error type. |
| D2 — exact cleanup-stage ledger | CLOSED | The six-stage decision table, cleanup-twice properties, and qualified native failure paths remain green. No stage semantics changed here. |
| D3 — structured worker cleanup disposition | CLOSED | The worker still returns exact incomplete/recovered disposition; the crash barrier delegates to the real `VmDriver` and opens only after its recovered disposition. |
| D4 — duplicate-owner cleanup | CLOSED | Exact duplicate-owner and cleanup-twice selections remain green. |
| D5 — composition propagation | CLOSED | The production composition still preserves the private cleanup carrier through the shim and lifecycle. |
| D6 — incomplete cleanup stays retryable | CLOSED | Pending remains nonterminal, persisted, periodically/boot retryable, and fresh-placement gating. |
| D7 — complete cleanup has one authoritative ending | CLOSED | Recovered live-process, boot, stop, delete, and crash-restart paths converge without a duplicate ending. |
| D8 — persisted crash-recoverable authority | CLOSED | D14's post-cleanup/pre-write crash now converges from the persisted Pending marker to diagnostic-preserving `Failed`, then normal Job finalization, without a new start. |
| D9 — restart accounting | CLOSED | Cleanup retries and D14 closure do not increment restart counters; no fresh placement bypasses the configured restart policy. |
| D10 — bounded retry and durable backoff | CLOSED | Per-owner timestamps are written to the persisted view before dispatch and immediate re-evaluation does not reissue an action. |
| D11 — duplicate teardown | CLOSED | Real cleanup is performed once per attempt, recovered owners are not rewritten, and later ticks/boot do not re-create or re-tear down the completed owner. |
| D12 — stop/delete authority | CLOSED | Both intent-withdrawal modes scan and retry every retained cleanup owner and converge independently. |
| D13 — lifecycle-owned retained cleanup | CLOSED | `Run` continues to prioritize retained cleanup over placement through the production lifecycle/runtime path. |
| D14 — residue-free crash after cleanup | CLOSED | Non-vacuous all-`NotFound` probing plus successful boot authority closes the crash window while preserving the exact cleanup reason/detail and normal terminal accounting. |
| D15 — lexical multi-owner starvation | CLOSED | The lifecycle collects every retained owner, emits every due action in deterministic allocation order, and cannot let a failing first owner suppress a later recoverable owner. |

No new finding was opened in Iteration 6.

## D14 crash-boundary audit

The closure is conservative at every decision boundary:

| Boundary | Result |
|---|---|
| Durable precondition | Closure applies only when the prior durable observation is the exact start-cleanup `Pending` marker. An ordinary start failure cannot enter it. |
| Non-vacuous ownership probe | `probed_prior_owner` must be true. A fresh process has an empty allocation-driver index, so resolution falls back to all composed drivers; the production registry always contains the Exec driver even when VM capability is unavailable. |
| Stop-result semantics | Closure requires every probed stop to return `NotFound`. Any `Ok(())`, cleanup carrier, or other error makes `every_stop_was_not_found` false or follows the ordinary retry/error path. A live or uncertain owner is therefore not declared absent. |
| Residue authority | Production boot runs VM reclamation before serving lifecycle work and refuses startup if reclamation fails. Residue-bearing crashes are reclaimed by boot; only the successful-boot, residue-free case reaches the all-`NotFound` closure. This prevents a resource false-negative from being inferred from `NotFound` alone. |
| Diagnostic preservation | The new `Failed` row copies the retained cleanup `reason` and `detail` byte-for-byte. The production test checks the same detail before cancellation, after restart/boot, after failure, and after terminal finalization. |
| Placement ordering | The failed observation is written and returned before network provisioning, driver start, or any fresh allocation path. Both fresh post-crash VMMs observe zero creates. |
| Observation-write failure | If the `Failed` write fails, the persisted Pending marker and per-owner backoff remain authoritative; a later lifecycle evaluation retries rather than placing fresh work. |
| Ending and accounting | The closure writes one nonterminal `Failed` row and the ordinary Job lifecycle subsequently writes the terminal `FinalizeFailed` ending. It emits no restart action and leaves restart counts empty across two process rebuilds. |
| Cleanup proof honesty | The cancellation barrier wraps the real VM driver, awaits its real stop, verifies its returned cleanup disposition is `recovery_complete`, and blocks before the shim can receive that result or write the ending. Thus the injected crash is exactly after resource cleanup and before authoritative observation, not a synthetic state shortcut. |

`crash_after_cleanup_before_failed_write_converges_to_an_ending_before_any_fresh_start`
exercises this sequence through the registered workload lifecycle and runtime:
the first real tick creates Pending; a real inner `VmDriver` completes cleanup;
the runtime task is cancelled at the disposition barrier; fresh application
state, view, driver registry, and VMM are constructed; real boot convergence
runs; and later real ticks produce `Failed` then the Job terminal ending. The
test proves no new VMM create, no live driver allocation, no restart charge,
no diagnostic rewrite, and no orphaned clone/index. The barrier's
`recovery_complete` predicate is backed by the already-reviewed exact cleanup
stage ledger, so it is meaningful evidence for all VM cleanup resources, not
merely the two filesystem assertions repeated at the crash boundary.

## D15 deterministic-fairness audit

`WorkloadLifecycle::Run` now materializes all start-cleanup Pending rows from
the already deterministic active-allocation map. It performs one finite scan,
checks each owner's persisted timestamp independently, appends an action for
every due owner, records every due timestamp in the next persisted view, and
returns the full action vector. The runtime dispatcher drains the whole vector
and records only the first error after attempting later actions, so a failing
first cleanup cannot prevent a later cleanup action from executing.

| Fairness/liveness condition | Result |
|---|---|
| Deterministic order | B-tree hydration and vector iteration give stable allocation order. Correctness does not depend on changing that order. |
| Every due owner | Every Pending row whose independent backoff has elapsed emits exactly one action in the evaluation. There is no early return inside the owner scan. |
| Backed-off owner | A not-yet-due row emits no action but remains in the retained set and continues to gate placement. |
| Durable attempts | The next view contains timestamps for all due owners before action dispatch. A crash or an early action error cannot erase the backoff decisions for later emitted owners. |
| Error isolation | Runtime dispatch continues after one action error, allowing later recoverable owners to reach their ending in the same evaluation. |
| Persistent first owner | The adversarial two-owner test leaves the lexicographically first owner Pending while the second reaches diagnostic-preserving `Failed` and releases its live claim/residue. |
| No rewrite or duplicate work | Immediate and one-second follow-up ticks keep the completed second row unchanged and keep VMM creates and observation rows bounded. |
| Fresh-placement gate | The mere presence of any retained cleanup owner selects the cleanup branch, even when every owner is currently backed off; no normal placement action is emitted. |
| Crash/restart | After the first owner becomes recoverable, rebuilt application state and real boot converge it to the authoritative ending without restarting either owner. |
| Stop and delete | Separate production-composition journeys prove both operator stop and intent deletion retry all due owners, allow a later owner to finish despite the first partition, then finish the first after repair. |
| Scan/spin behavior | There is one bounded scan per externally scheduled lifecycle evaluation and no internal retry loop, busy wait, or recursive reconciliation. |

`run_retries_every_due_cleanup_owner_without_lexical_starvation_or_fresh_placement`
stages the adversarial historical second owner through the real registered
`StartAllocation` action shim, then exercises the behavior under review through
the real lifecycle/runtime entry path. Manual staging is appropriate here: the
state under test is a valid hydrated multi-owner history that ordinary new
placement is intentionally forbidden to create while cleanup is retained. The
test keeps the first owner partitioned and repairs only the second, proving
both actions are attempted, only the first claim/residue remains, both durable
timestamps are set, neither restart counter changes, and the create count
stays at the two original allocations. The paired stop/delete tests cover the
same failure/success ordering and eventual recovery for intent withdrawal.

## Production-composition and fault-injection audit

The new tests do not replace production decisions with a local model. They use
the production `AppState`, workload lifecycle, view store, action dispatcher,
observation store, driver registry, real VM driver, real reclamation boot
entry, and VMM boundary. Test wrappers only expose precise fault points:
partition repair and the post-real-cleanup disposition barrier. The multi-owner
fixture uses a production action-shim transition rather than inserting an
invented observation. Assertions cover the externally meaningful and private
integration guarantees needed here: durable row phase/detail, driver
claim/residue, allocation identity, VMM create count, restart count, persisted
backoff, boot adoption, and terminal convergence.

All nine live tests in the cleanup authority file carry the exact required
`/// CONTRACT_SHAPE: bounded-change.` declaration. No test spawns the built
Overdrive binary, invokes `cargo test`/nextest from inside a Rust test, emits
expectation evidence, or imports an expectation harness. No E08/E09 path,
legacy path, or unsupported Service-plus-VM success category is introduced.

## API, persistence, and frozen-shape audit

| Surface | Result |
|---|---|
| Frozen `DriverError` and `Driver` API | PASS — remediation is confined to private composition/reconciliation behavior |
| Persisted allocation row shape | PASS — only existing phase/reason/detail/terminal fields are written |
| View persistence | PASS — existing per-allocation `last_failure_seen_at` map carries independent durable backoff; no schema change |
| Wire/REST/OpenAPI | PASS — zero target diff |
| Core/BPF/Cargo | PASS — zero target diff |
| Examples and expectations | PASS — zero target diff; the Rust suite remains in-process integration evidence |
| Unsupported Service-plus-VM | PASS — tests and claimed terminal behavior are for the roadmap's supported Job journey |
| Contract Shape | PASS — all new/changed live tests have exact declarations |
| Mutation discipline | PASS — no exclusion changed and mutation testing was not run per step |
| Repository hygiene | PASS — pre-existing untracked AGENTS/review files remain untouched |

## DES and commit audit

The appended D14/D15 cycle is chronological and mechanically valid: RED
`FAIL` at `2026-08-30T06:43:03Z`, GREEN `PASS` at `06:57:57Z`, and COMMIT
`PASS` at `06:58:22Z`. RED records the two observable parent defects: the
crash journey proceeds toward a VMM/start instead of the authoritative failed
ending, and the multi-owner journey selects only the first cleanup. Those
failures correspond directly to the production branches changed by the
remediation. Reflog retains the initial implementation commit `87928b5f` at
local `08:58:14 +0200` and the log/review-bearing amend `35509f94` at
`08:58:28 +0200`. The final commit has exact parent `f9c5c59e`, conventional
subject, and exact `Step-Id: 02-05` trailer. JSON parsing, formatting, exact
remediation diff checking, and cumulative step diff checking pass.

Only the isolated crafter's D14/D15 RED, GREEN, and COMMIT events were added.
No event claims mutation testing or a broader suite than was executed.

## Suite and failure classification

The focused checked-OpenAPI gate still fails deterministically because live
`/v1/workloads/{id}/stop` contains `workload_addr` while the checked YAML has
`workload_id`. The remediation changes neither OpenAPI source declarations nor
`api/openapi.yaml`; the failure is inherited, independently reproduced, and
outside 02-05's target.

The earlier transient kTLS RX `ENOTCONN` observations also remain non-target.
Iteration 3 isolated the two affected sentinel tests by running each serially
five times, for 10/10 passes, and established zero target diff in their kTLS
implementation surface. Iterations 4 and 5 did not reproduce the transient,
this remediation again has zero dataplane-mTLS/kTLS diff, and the qualified
native six-test selection is green once more. The accumulated evidence
supports retaining the transient infrastructure classification; no target
waiver is being inferred from a single rerun.

## Iteration 6 independent verification

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| Exact remediation `git diff --check` | PASS |
| Cumulative 02-05 `git diff --check` | PASS |
| `execution-log.json` parse | PASS |
| Full cleanup-authority production-composition selection | PASS — 9/9; 787 skipped |
| Frozen API, structured cleanup, duplicate-owner, action-shim selection | PASS — 8/8; 1,838 skipped |
| Reclamation planner/supervision properties | PASS — 3/3; 837 skipped |
| Qualified native Beacon/rootfs/failure selection | PASS — 6/6; 259 skipped; 59.381s |
| Focused OpenAPI checked-file gate | FAIL — inherited deterministic YAML drift, isolated above |
| Target E08/E09/built-process/legacy/Service-plus-VM scan | PASS — none introduced |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate |

## Iteration 6 verdict

**APPROVED.** D14 and D15 are closed without reopening D1-D13. The
post-cleanup crash boundary now preserves the authoritative diagnostic,
cannot place fresh work, cannot hide surviving VM residue behind a vacuous
probe, and converges through normal Job terminal accounting exactly once.
Every due retained cleanup owner is attempted with deterministic, persisted,
independent backoff and dispatch error isolation; a permanently failing first
owner cannot starve later success, and any retained owner continues to gate
fresh placement. Stop, delete, success, failure, crash, and rebuilt-process
paths are covered through honest production composition. Step 02-05 may
advance to 02-06.

---

# Recovery execution — Iteration 1

## Metadata

| Field | Value |
|---|---|
| Step | `02-05` recovery execution |
| Review iteration | Recovery Iteration 1 |
| Approved DESIGN baseline / parent | `e0f11fe174a851b63c9b31b5774dadca4fbde8fd` |
| Target commit | `1f3970500ee472ddc7a44e9b59870ab493ebef45` |
| Subject | `fix(mtls): restore truthful VM failure closure` |
| Trailer | `Step-Id: 02-05` |
| Verdict | **NEEDS_REVISION** |

## Recovery provenance and scope

Historical Iterations 1–6 above are retained verbatim as review provenance.
Their final approval applies to the former retained-cleanup architecture only.
The later approved recovery DESIGN in `feature-delta.md` and
`design/wave-decisions.md` explicitly supersedes the filesystem outbox,
Pending cleanup token, retry-owner transfer, pre-start intercept/rollback, and
generic task architecture that those iterations reviewed. The historical
approval is therefore not approval of this recovery implementation.

The target is a direct child of the approved recovery DESIGN baseline and
changes 13 files: 629 insertions and 2,802 deletions. The review covered the
complete target diff, the authoritative recovery amendment, the current
production paths reached by the changed code, the changed and deleted tests,
the DES log, and independent focused verification. The deletion of
`vm_cleanup_reclamation_authority.rs` is correct in principle because that file
tested the rejected Pending-cleanup state machine; the problem is that the
accepted current+occurrence and resource-specific reclamation contracts were
not implemented and fully replaced.

## Summary

The target correctly narrows VM failed-start cleanup back to a private ordered
stage ledger, preserves the original typed rejection when cleanup succeeds,
uses the existing unclassified VM start failure when cleanup fails, retains the
duplicate-owner refusal, preserves diagnostic precedence and bounds, and
restores post-Running install-failure cleanup without releasing EXEC. The
focused behavior selection, formatting, linting, and DES integrity checks are
green.

The recovery is nevertheless incomplete. The exact R1 ObservationStore
contract and rejected-surface removals are absent; the action shim still has a
pre-start rollback carrier that can strand a real VM claim; two post-claim
`VmDriver::start` branches do not perform the mandated total cleanup; no honest
production-composition test proves cleanup-failure handoff into ordinary
Failed plus Artifact Disposal; and eleven transitioned acceptance tests fail
the mechanical Outcome-anchor gate.

## Finding disposition

| Finding | Severity | Disposition |
|---|---|---|
| REC-01 — exact ObservationStore lifecycle boundary and rejected outbox surfaces were not recovered | Critical | OPEN |
| REC-02 — surviving pre-start rollback carrier can strand the allocation before Failed closure | Critical | OPEN |
| REC-03 — two post-claim VM start failures bypass the total cleanup sequence | High | OPEN |
| REC-04 — cleanup-failure-to-Artifact-Disposal production composition is unproved | Critical | OPEN |
| REC-05 — transitioned acceptance tests lack the mandatory Outcome anchor | Blocker | OPEN |

## Detailed findings

### REC-01 — exact ObservationStore lifecycle boundary and rejected outbox surfaces were not recovered

**Severity: Critical — accepted public and persistence contract divergence.**

The approved recovery DESIGN fixes one lawful allocation-current authoring
path: `ObservationStore::write_alloc_lifecycle(current, source)`, atomically
accepting the LWW current row and one bounded occurrence. Its generic writer
must accept the exact seven-variant non-allocation `ObservationWrite` enum. It
also explicitly removes `LifecycleEventPort`,
`IdempotentLifecycleEventPort`, `TerminalEffectJournalError`, the filesystem
`terminal-effects/` projection, `effect_key`, and
`Driver::on_alloc_terminal_idempotent`.

The target leaves the opposite shape in production:

- `overdrive-core/src/traits/observation_store.rs:1873-1915` still exposes
  `write(ObservationRow)` and has no `write_alloc_lifecycle`, occurrence
  reader, or `ObservationWrite` input.
- There is no `AllocLifecycleOccurrenceRowV1`, exact
  `AllocLifecyclePredecessor`, 64-entry occurrence table, or compound writer in
  the workspace.
- `action_shim/mod.rs:667`, `:771`, `:2346`, `:2393`, `:2691`, `:3093`, and
  `:3301` still author current allocation rows through the generic
  `ObservationRow::AllocStatus` route. This includes the newly changed mTLS
  install-failure and VM start/restart failure paths.
- `action_shim/mod.rs:778-960` still defines the public lifecycle-event port,
  durable idempotent projection, filesystem journal, and public journal error;
  the terminal action paths still use that journal.
- `overdrive-core/src/traits/driver.rs:876-883` still exposes the rejected
  public idempotent terminal hook and effect key, and the action shim still
  calls it.

This is not compiler fallout deferred outside the step: the target directly
modifies both the action shim and the public driver trait while retaining the
surfaces that the recovery contract names for removal. As implemented, the
current row and durable lifecycle effect still have separate persistence
boundaries, while the required occurrence fact does not exist.

**Required remediation:** implement the exact R1 schema and trait signatures,
migrate every allocation-current production/test author through the compound
writer with its real existing `TransitionSource`, and remove the named outbox,
port, effect-key, and idempotent-hook surfaces. Do not invent a compatibility
overload, default source, raw allocation writer, second store, or alternative
public API.

### REC-02 — surviving pre-start rollback carrier can strand the allocation before Failed closure

**Severity: Critical — externally reachable failure path violates truthful
closure and the fixed ordering.**

`rollback_prestarted_allocation` at
`action_shim/mod.rs:2008-2040` still attempts mTLS teardown before any
intercept can legally have been installed, then returns the public
`ShimError::DriverStartRollback` at `:3485-3498` if structural network teardown
fails. Both fresh start (`:2555-2566`) and restart (`:2955-2966`) invoke it
before constructing or writing the ordinary Failed disposition.

For a real `VmDriver::start` rejection, the driver intentionally retains its
supervision claim until the action shim resolves the Failed write. If network
teardown fails here, the helper returns before that write and before claim
release. An identical retry reaches the typed C4a duplicate-owner guard at
`:2548-2553` / `:2948-2953` and returns `Ok(())` before retrying the network
cleanup. The allocation can therefore remain indefinitely claimed with its
slot/network residue and no truthful Failed closure. The preceding DES history
already records this exact production-composition failure: the identical retry
re-entered `Driver::start` instead of completing cleanup.

The retained acceptance examples are also stale and misleading:
`action_shim_crash_observability.rs:408-420` asserts that a “preinstalled
intercept” was removed, but the accepted production order installs the
intercept only after `Driver::start`, Running, and READY. Its fake driver fails
before that gate, so the mTLS assertion is vacuous. Lines `423-447` then bless
the public rollback carrier and retained retry owner that the recovery DESIGN
rejects.

**Required remediation:** remove the rejected pre-start intercept/rollback
surface and tests, preserve the required structural-network cleanup without a
new public cleanup protocol, and prove that fresh and restart start rejection
cannot exit before the ordinary Failed current+occurrence and post-write claim
release/abandonment boundary. Include the teardown-failure/replay partition so
the duplicate-owner guard cannot suppress cleanup. If the approved R2/R6
resource-specific rules are not considered to pin the live-process disposition
when structural teardown itself fails, record that as a blocking DESIGN gap
and obtain the exact shape before implementation; do not improvise another
cleanup carrier or state machine in review remediation.

### REC-03 — two post-claim VM start failures bypass the total cleanup sequence

**Severity: High — the R2 cleanup invariant is not total.**

R2 requires every post-claim non-success branch to attempt VMM termination,
rootfs/index removal, cgroup kill/removal, and run-directory removal in stable
order, with absence treated idempotently and later stages attempted after a
failure. Most branches now use `cleanup_after_start_failure`, but two do not
honestly supply that sequence:

- After inserting `VmSupervision::Starting`, a run-directory creation error at
  `vm_driver.rs:1158-1161` returns immediately without calling the cleanup
  helper. `create_dir_all` may have created part of the target path before
  reporting an error.
- A workload-scope creation error at `vm_driver.rs:1187-1193` calls cleanup
  with `scope: None`, so the cgroup kill and remove stages are skipped for the
  exact scope whose create operation failed. This contradicts the design's
  total, idempotent-on-absence stage set and cannot clear a partially existing
  or pre-existing allocation scope.

The existing “every rejection” test does not inject either boundary, so its
name overstates the covered rejection set.

**Required remediation:** route both post-claim failures through the same total
cleanup attempt with the known run directory and cgroup scope, preserving the
original typed/unclassified cause only when every cleanup stage succeeds, and
add focused fault partitions for both branches.

### REC-04 — cleanup-failure-to-Artifact-Disposal production composition is unproved

**Severity: Critical — missing acceptance coverage for the principal recovery
handoff.**

The target deletes the former 1,385-line Pending-cleanup production-composition
suite, as the recovery DESIGN requires, but it does not replace the central R2
journey with accepted-architecture evidence. The new
`vm_driver.rs:2460` test calls the private cleanup helper directly, checks its
detail string and residue, then manually calls `release_supervision`. It cannot
prove the production action shim writes ordinary Failed, releases the claim
only at the observation boundary, and lets the existing VM reclamation planner
select terminal-row `DiscardStrandedArtifacts` exactly once. The new install
cleanup test at `action_shim/mod.rs:4723` uses a fake Exec driver and exercises
post-Running mTLS-install cleanup, not failed `VmDriver::start` artifact
cleanup.

The focused green selection therefore proves successful failed-start cleanup
and a private composite, but not the recovery's resource-specific failure
handoff. This is the highest-risk behavior created by removing the Pending
state machine.

**Required remediation:** add an honest production-composition test using the
real action-shim entry, real `VmDriver`, ObservationStore boundary, and existing
VM reclamation path. Inject at least one cleanup-stage failure and prove the
ordered unclassified composite, ordinary Failed current+occurrence, no EXEC,
post-write supervision release, resource-specific Artifact Disposal of only
VM cgroup/run/rootfs residue, exactly-once cleanup/replay behavior, and sibling
preservation. The test must not recreate the deleted Pending protocol.

### REC-05 — transitioned acceptance tests lack the mandatory Outcome anchor

**Severity: Blocker — mechanical Contract Shape gate.**

The transitioned acceptance tests have exact `CONTRACT_SHAPE` declarations,
but eleven lack the required exact rustdoc line
`/// Outcome anchor: DISCUSS Elevator Pitch`:

- Four install-failure tests in
  `tests/integration/mtls_install_fail_closed.rs:714-851`.
- `initial_and_restart_start_expose_identical_cause_and_detail_pairs` and
  `every_vm_start_rejection_leaves_no_vm_resources` in
  `tests/acceptance/vm_driver_start_failure_contract.rs:527-779`.
- The create-failure, pre-READY exit, accepted-close, boot-deadline, and
  pre-beacon-stop examples in
  `tests/acceptance/vm_driver_stop_totality.rs:304-811`.

`cargo xtask dst-lint` is green but does not discharge this reviewer-mandated
diff-gated mechanical check.

**Required remediation:** add the exact Outcome-anchor line to every listed
transitioned acceptance test and rerun the Contract Shape declaration checker.
Do not perform a legacy repository-wide sweep in this step.

## DES, TDD, test budget, and integrity

| Check | Result | Assessment |
|---|---|---|
| DES phase order | PASS | Recovery entries are chronological RED `FAIL` at `2026-08-30T23:11:38Z`, GREEN `PASS` at `23:39:00Z`, and COMMIT `PASS` at `23:56:49Z`. `des-verify-integrity` reports all nine step traces complete. |
| Commit mechanics | PASS | Target parent is the approved DESIGN baseline; subject is conventional and trailer is exact. |
| Distinct behaviors | 8 | S-GTI-05; S-GTI-08a; diagnostic totality; S-GTI-08b; pre-READY interruption cleanup; C4a duplicate refusal; failed-cleanup/reclamation handoff; exact product/fixture/sibling restoration. |
| Source-local unit-test budget | 16 maximum | `2 × 8` behaviors. |
| Actual focused source-local tests | 6 | Guest diagnostic precedence, byte/fragment/lossy bounds, async diagnostic totality, full diagnostic/cleanup totality, cleanup composite, and install-cleanup composite. Within budget. |
| Changed-test integrity | PASS with recovery qualification | Claim-retention assertions replace the superseded release/cleanup-token behavior consistently with the approved recovery. Deleting Pending-state-machine tests is authorized. No skip or assertion weakening was found in the retained accepted behavior. |
| External validity | FAIL | REC-02 retains a vacuous pre-start-intercept assertion; REC-04 lacks the real cleanup-failure/reclamation composition. |
| Contract Shape declarations | PASS | Exact declarations are present on the transitioned tests. |
| Outcome anchors | FAIL | REC-05 lists eleven diff-gated failures. |
| Banned test names | PASS | No transitioned test name matches the prohibited output-encoding regex. |
| Mutation discipline | PASS | No mutation run and no mutation exclusion edit; mutation remains the final DELIVER-wave gate. |

## Independent verification

| Verification | Result |
|---|---|
| Focused Lima nextest selection across worker/control-plane recovery behavior | PASS — 18/18; 997 skipped; run `7644abd9-a2e5-43fa-9114-8a8732931e25` |
| Lima clippy for `overdrive-worker` and `overdrive-control-plane`, integration tests, all targets, `-D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask dst-lint` | PASS |
| `git diff --check e0f11fe1..1f397050` | PASS |
| `jq empty execution-log.json` | PASS |
| `des-verify-integrity deliver/` | PASS — all 9 traces complete |
| Crafter-reported affected Lima suite | PASS — 1,874/1,874; not independently rerun in full by this reviewer |
| Crafter-reported qualified native selection | PASS — 5/5; not independently rerun by this reviewer |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate |

The green focused suite is valid evidence for the behavior it actually drives;
it does not override the public/persistence design mismatch, the stranded
rollback path, or the missing cleanup-failure production composition.

## Recovery Iteration 1 verdict

**NEEDS_REVISION.** The recovery cannot advance to step 02-06. REC-01 and
REC-02 are direct violations of the approved recovery architecture and fixed
public/persistence boundary. REC-03 leaves the promised total failed-start
cleanup incomplete. REC-04 leaves the most important resource-specific
reclamation handoff unproved. REC-05 fails a mandatory mechanical acceptance-
test gate. Remediation must stay inside the exact approved R0–R8 shapes; it
must not reintroduce Pending cleanup, a public cleanup carrier, a second
persistence boundary, an effect key, or a replacement lifecycle/reclamation
subsystem.
