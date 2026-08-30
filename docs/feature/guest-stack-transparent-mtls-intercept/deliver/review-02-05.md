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
