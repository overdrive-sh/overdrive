# Adversarial review — step 01-02

## Current review state

| Field | Value |
|---|---|
| Latest iteration | 4 |
| Current reviewed implementation | `2f3cb4befc66de93186b527d820a67e4da80df84` |
| DES-log completion | `9d0b37d7e625e089c68363fc2bfe43234d8e635b` |
| Current verdict | **APPROVED** |
| Open findings | 0 (F1-F8 closed) |

## Iteration 1 metadata

| Field | Value |
|---|---|
| Feature | `vm-lifecycle-latency` |
| Step | `01-02` — Host/guest termination |
| Base | `af158034781784684d9ad15af9fdd94fe031dc64` |
| Implementation commit | `5edd751ec9c1e9e3e3916dbadc8f4e46703935b5` |
| DES-log completion commit | `ffea32c6f76793ccbe0c4a28f8da8d2eba619e06` |
| Reviewer | `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-11 |
| Iteration | 1 |
| Verdict | **CHANGES_REQUESTED** |

## Executive summary

The implementation preserves all public `Driver`, `Vmm`, beacon-wire,
allocation-state, persistence, and terminal-error surfaces. The exact private
`overdrive-init` lifecycle signature, `exec_operator_command` signature, and
two new `InitError` variants match ADR-0103. The stop path atomically moves the
unique `LiveVm`, starts the writer cap and the single VMM grace together,
consumes every writer outcome, awaits VMM termination, and leaves
`EndingInFlight` supervised. Qualified native metal is green for the complete
15-test VM module, the focused S10a/S10b pair, and the nine VMM/host-state
equivalence tests. S-VLL-11 remains correctly ignored for step 01-03.

Approval is nevertheless blocked. The natural watcher can publish its
`ExitEvent` after an invented two-second timeout without having consumed the
VMM exit; the shared cleanup helper contains an unauthorized fifth timed wait
and the watcher makes its required cleanup conditional. The implementation
commit also weakens S10b's post-EXEC exact-error assertions to a generic marker
printed by the same production function, leaving the original typed causes
unverified. The real signal-death case does not assert the direct child's exact
`128 + signal` result. A transitioned S13 test lacks its required Contract Shape
declaration and still encodes the superseded pre-reaper publication order.
Finally, module and ownership documentation contradict the implemented
supervisor/move semantics.

## Contract and review scope

The authoritative contract is the independently approved natural/post-EXEC
amendment in ADR-0103, the matching architecture-brief transition, DDD-8 and
the DISTILL handoff in `feature-delta.md`, roadmap step 01-02, and scenarios
S-VLL-07, S-VLL-08, S-VLL-09, S-VLL-10a, S-VLL-10b, and shared S-VLL-13. The
approved Landlock design/fix/review commits precede the base and were treated as
a preservation boundary, not reopened.

The committed implementation changes six files: three production Rust files,
one crate manifest, and two test files (634 insertions, 220 deletions). The
manifest change adds only the compiler-required pinned nix `poll`, `process`,
and `signal` features. This reviewer did not modify production, tests, design,
roadmap, DES state, or commits. The pre-existing dirty `AGENTS.md` is preserved.
This review artifact is the only reviewer write.

## Inspected implementation evidence

### API and ownership shape

| Contract | Result | Evidence |
|---|---|---|
| Existing public `Driver::stop`, `Vmm::terminate`, `BeaconWriter::request_stop` retained | PASS | The base-to-implementation diff adds no `pub` method, type, variant, field, or parameter. |
| Exact private `exec_operator_command(&mut File, &[String]) -> Result<i32, InitError>` | PASS | `crates/overdrive-init/src/main.rs:1037-1052`. |
| Exact private `complete_guest_lifecycle` generic/callback shape | PASS | `crates/overdrive-init/src/main.rs:171-215`; the post-EXIT shutdown callback and helper are removed. |
| Exact new private `InitError::{Wait, ProcessGroup}` only | PASS | `crates/overdrive-init/src/main.rs:254-367`; the only added variants are at `:353-363`, and existing mappings remain. |
| Exact `ClaimGuard::try_begin_ending(&mut self) -> Option<LiveVm>` | PASS | `crates/overdrive-worker/src/vm_driver.rs:931-966`; accepted-session identity, removal, unit `EndingInFlight` insertion, and handoff verdict occur under one lock. |
| Unique `LiveVm` move on operator stop | PASS | `vm_driver.rs:1686-1744`; the whole non-`Clone` value moves to the winner before any await. |
| Exact `run_exit_watcher` parameter order | PASS | `vm_driver.rs:2165-2178`; `CgroupManager` is between `exit_tx` and `cgroup_accounting`. |
| One shared cleanup helper with exactly the selected contents/order | **FAIL** | F2: `vm_driver.rs:1431-1436` contains an extra timer, and `:2235-2243` makes the watcher call conditional. |
| Natural watcher consumes VMM exit before cleanup/report | **FAIL** | F1: `vm_driver.rs:2179-2187` discards a two-second timeout result and continues. |

### Host stop and stage evidence

`VmDriver::stop` creates and first-polls the existing termination future in the
same `tokio::select!` composition as the writer deadline. A completed writer
continues the same termination future; writer expiry aborts and joins the
writer while continuing it; VMM completion aborts and joins a pending writer;
and absence skips only submission. Cleanup follows termination. The disposition
event is emitted after the writer task is consumed. No hidden Tokio runtime or
detached stop effect was introduced.

The implementation emits the approved create-entry, created, READY, stop-entry,
writer-finished, reaper, and cleanup-calls-finished events with the specified
fields. `vmm.process.reaped` precedes both VMM outcome publications in the host
reaper. The cleanup event on the natural watcher is subject to F1/F2; a passing
healthy native sample does not make the missing unconditional ordering
structural.

### Guest supervision

The guest keeps one `File` stream owner, consumes exactly one pre-EXEC frame
without buffering past its newline, establishes PGID equal to the direct child,
polls control while reaping, records one five-second deadline, signals the
whole group, retries `EINTR`, treats group `ESRCH` as absence, rejects premature
`ECHILD`, preserves normal direct-child exit values, and powers off without a
post-EXIT read. EOF after EXEC becomes `Io(UnexpectedEof)` and malformed or
duplicate frames retain their typed values in production. The two gaps are in
the evidence: F3 removes the post-EXEC typed-cause oracle, and F4 does not
observe the signal-mapped direct-child result that production computes.

## Acceptance-test honesty and quantitative gates

There are six mapped step behaviors (S07, S08, S09, S10a, S10b, shared S13),
giving a review budget of twelve newly authored/activated behavior tests. The
step activates five mapped scenario bodies and adds one focused private-File
boundary test; six is within budget. Existing shared S13 regressions are
preservation evidence, not newly authored behavior tests.

| Gate | Result | Evidence |
|---|---|---|
| G1 — step activation scope | PASS | S07-S10 are active; S11 alone remains reasoned-pending for 01-03. |
| G2 — semantic RED | PASS | Step 01-02 RED is recorded before GREEN and the base bodies carry real oracles. |
| G3 — assertion failure for any added inner test | NOT APPLICABLE | No separate PBT inner loop was required; the added File-boundary test is an acceptance complement. |
| G4 — doubles only at ports | PASS | Worker tests decorate the existing `Vmm` port; native S10 uses the production serve/deploy/Cloud Hypervisor/init path and an existing VMM-port transport decorator. |
| G5 — business/domain language | FAIL | F6 leaves directly contradictory module/ownership prose; the active test names otherwise use the feature vocabulary. |
| G6/G7 — GREEN and commit phase | PASS | Independently rerun focused, native, preservation, compile, clippy, and format commands are green; execution log records GREEN and COMMIT PASS. |
| G8 — test budget | PASS | 6 activated/new bodies <= `2 × 6 = 12`. |
| G9 — no test weakening | **FAIL** | F3 removes the three exact post-EXEC cause assertions and replaces them with one generic marker assertion. |

### Contract Shape Compliance

**Overall: FAIL.** S07, S08, S09, S10a, S10b and the new private File-boundary
test carry valid declarations; S09 uses the exact
`/// CONTRACT_SHAPE: pure-function.` spelling. F5 identifies one S13 test whose
contract changed under DDD-8 but still has no declaration. No step-owned test
name matches the banned implementation-detail pattern. The repository-specific
feature contract governs outcome anchoring and does not prescribe a new literal
anchor string for this feature.

## Findings

### F1 — natural `ExitEvent` can precede reaper-observed VMM exit

- **Severity:** Blocker
- **Dimension:** Exact DESIGN ordering; natural-exit ownership
- **Locations:** `crates/overdrive-worker/src/vm_driver.rs:2105-2130,2179-2187,2229-2258`; ADR-0103 lines 180-191
- **Production entry point and owner path:** `overdrive serve` composes
  `VmDriver`; `Driver::start` spawns `run_exit_watcher`; the watcher drains the
  guest report and `VmExitWatch`, claims the accepted `LiveVm`, cleans, and
  sends the sole `ExitEvent` to the production exit observer.
- **Exact reachable ordering:** When a guest `EXIT` line wins the biased first
  read, `drain_guest_report` returns `vmm_reaped = false`. The watcher waits only
  through `tokio::time::timeout(Duration::from_secs(2), exit.recv())`, discards
  timeout versus closed-watch versus actual-exit, then claims, cleans, and sends
  the event. ADR-0103 instead requires consuming the VMM exit before the OOM /
  gate / claim / cleanup / event chain and states that direct exit has already
  been reaped.
- **Reproducer:**
  `cargo xtask lima run -- cargo nextest run -p overdrive-worker --features integration-tests -E 'test(exit_event_is_gated_until_running_confirmed_release)' --success-output immediate`
  passes in 2.02 s. Its `SimVmm` remains live because the test never calls
  `Vmm::terminate`; it supplies only guest `EXIT 0` and the Running-confirmed
  release, yet receives an `ExitEvent`. The two-second duration is the discarded
  timeout, not a VMM exit. This demonstrates the implemented owner ordering; it
  does not claim that the qualified healthy native population currently takes
  longer than two seconds.
- **Disposition:** OPEN. Remove the invented timeout and require the existing
  watch to resolve before the natural path advances, preserving the captured
  guest status. The S13 test must be transitioned by its test owner to resolve
  the VMM watch and to prove that Running-gate release alone cannot publish
  before reaping. Do not add another deadline, termination call, state, or API.

### F2 — DDD-8's exact cleanup helper and unconditional watcher call are changed

- **Severity:** Blocker
- **Dimension:** Exact private contract; scope control; production shaped by test timing
- **Locations:** `crates/overdrive-worker/src/vm_driver.rs:1388-1436,2229-2243`; ADR-0103 lines 124-142 and 159-184; `feature-delta.md:272-293,642-648`
- **Production entry point and owner path:** Every successful `VmDriver::stop`
  calls `cleanup_driver_artifacts`; every natural watcher winner is required to
  call the same helper before its `ExitEvent`.
- **Exact reachable ordering:** The helper always executes an added
  `tokio::time::sleep(Duration::from_millis(10))` between `cgroup_kill` and
  scope removal. ADR-0103 says the helper contains **only** the four existing
  calls in their current order. The watcher then adds a `yield_now` and calls
  cleanup/event emission only when the moved `LiveVm.pending_exec` is `None`,
  although the selected contract makes `Some(LiveVm)` itself the unique and
  unconditional cleanup capability. `spawn_exit_watcher_task` also receives a
  new `CgroupManager` parameter instead of cloning `self.cgroup_manager` as the
  pinned routing says.
- **Reproducer:** Source/caller audit proves the unconditional extra timer on
  both real stop and natural cleanup paths. The focused S07 run is green but
  takes 0.02 s and asserts only that it stays below one second, so it cannot
  reject the new ten-millisecond floor. Qualified native S10a/S10b passes only
  populations where the conditional is true; it does not make the contract's
  unconditional call or exact helper contents hold.
- **Disposition:** OPEN. Restore the exact four-call helper, clone the existing
  manager at the pinned owner, and invoke cleanup plus its event immediately and
  unconditionally after a successful claim. If removing the timer makes the
  qualified artifact oracle fail, report that as a reproduced DESIGN gap; do
  not invent a wait inside the accepted helper.

### F3 — S10b's exact post-EXEC error assertions were weakened to a self-authored marker

- **Severity:** Blocker
- **Dimension:** G9 test integrity; testing theater; missing typed-error coverage
- **Locations:** base test lines 3485-3511 versus current
  `crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs:3509-3548`;
  `crates/overdrive-init/src/main.rs:1095-1164`; `feature-delta.md:564`
- **Production entry point and owner path:** The native S10b decorator delivers
  EOF, malformed data, or a duplicate EXEC after production init starts the
  child. `supervise_command` records `Io(UnexpectedEof)`, `BeaconParse`, or
  `UnexpectedBeaconMessage`, completes group teardown, returns that typed value,
  and `main` owns rendering plus fatal poweroff.
- **Failure and reachability:** The base RED body parameterized exact expected
  causes (`beacon connection I/O failed`, `could not parse`, `received Exec`)
  and asserted each after teardown. The implementation commit deletes the
  expected-cause column and all three assertions. It instead asserts only
  `console.contains("overdrive-init: fatal:")`; the same new production
  `supervise_command` prints that literal immediately before returning any
  `control_error`. The test therefore stays green if every post-EXEC failure is
  collapsed to the wrong typed variant or if `main` never renders the returned
  cause. The new private-File test covers pre-EXEC only and cannot replace this
  post-EXEC obligation.
- **Reproducer:** Base/current diff is the mechanical G9 reproducer. The
  qualified native 2/2 and 15/15 runs pass the generic marker assertion, which
  confirms that the green gate no longer distinguishes the three required
  causes.
- **Disposition:** OPEN. Restore an exact post-EXEC typed-cause oracle through
  the approved private File/process boundary (acceptance-designer-owned if test
  construction is needed), remove the self-fulfilling bare marker, and retain
  the native group/reap/poweroff and no-EXIT observations separately. No new
  public adapter or error is authorized.

### F4 — the real direct-child signal result is not observed

- **Severity:** High, blocking acceptance completeness
- **Dimension:** S-VLL-10a completeness; test honesty
- **Locations:** `crates/overdrive-init/src/main.rs:1191-1204,1259-1276`;
  `crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs:3314-3388`;
  ADR-0103 lines 278-290; `feature-delta.md:564`
- **Production entry point and owner path:** The TERM-resistant native case
  drives the real guest supervisor to SIGKILL the process group. `reap_children`
  then maps the direct child to `128 + signal`, and successful supervision sends
  that value through the existing EXIT sender before guest poweroff.
- **Failure and reachability:** The native case is real and qualified, but its
  only EXIT assertion is `matches("EXIT ").count() <= 1`. It never asserts
  `EXIT 137`, so a descendant status, zero, another signal, or the retained
  `-1` fallback would all pass. The preserved `exit_status_to_wire` function is
  now dead/allowed and is not called by `reap_children`; no independent property
  links its documented mapping to the production wait path.
- **Reproducer:** The source audit is a bounded coverage reproducer, not an
  allegation that the current native result is wrong. The qualified 15-test
  run includes the signal case and still cannot distinguish any signed status
  value because the oracle checks only cardinality.
- **Disposition:** OPEN. Strengthen the existing S10a matrix to assert the
  direct child's exact signal-mapped EXIT value and route the production wait
  result through the retained mapping SSOT (or otherwise make the approved
  mapping the actual tested path without inventing a new public surface).

### F5 — transitioned S13 watcher test lacks Contract Shape and preserves the old gate

- **Severity:** Blocker
- **Dimension:** Mechanical Contract Shape compliance; preservation-test transition
- **Location:** `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:1688-1753`
- **Reachability:** This is the mapped S13 Running-confirmed-gate regression.
  DDD-8 changes its production observation from “gate release permits event” to
  “gate release permits the already-reaped watcher to proceed to claim and
  cleanup.” The test therefore transitioned in step 01-02 even though its body
  was left untouched.
- **Reproducer:** The live `#[tokio::test]` has no preceding
  `CONTRACT_SHAPE` rustdoc, and the focused 2.02-second run in F1 shows it still
  accepts an event from a live VMM after only the gate plus timeout. A green
  test cannot supply the missing declaration or the missing reaper complement.
- **Disposition:** OPEN. Have the acceptance-test owner transition this S13
  test to the DDD-8 order, add `/// CONTRACT_SHAPE: bounded-change.`, and assert
  both gate preservation and no event before reaper/cleanup completion. Do not
  weaken the Running-confirmed gate.

### F6 — source documentation contradicts the landed behavior and ownership

- **Severity:** Medium, blocking zero-defect approval
- **Dimension:** RPP L1 readability; no-aspirational-docs discipline
- **Locations:** `crates/overdrive-init/src/main.rs:34-63`;
  `crates/overdrive-worker/src/vm_driver.rs:909-918,1388-1403,1716-1727`;
  `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:901-938`
- **Reachability:** Documentation is read at every maintenance/review call site
  and currently states mutually exclusive lifecycle contracts.
- **Reproducer:** The init module still says concurrent SHUTDOWN is not
  attempted and that control is read only after child completion, while the new
  supervisor does the opposite. `ClaimGuard` still describes a Boolean `true`
  handoff; `spawn_exit_watcher_task` says its parameters are unchanged; and the
  stop comment says moving the `LiveVm` drops its gate sender even though the
  sender is retained until the explicit `take` after unlocking. The preserved
  `stop_sequence_b_unresponsive_guest_escalates_after_deadline` prose also
  describes the superseded sequential writer-then-VMM ordering.
- **Disposition:** OPEN. Update or remove the stale historical prose so it
  states the accepted ADR-0103 supervisor, `Option<LiveVm>` capability,
  explicit gate-sender drop, and concurrent writer/VMM wait exactly. Do not add
  new behavior while correcting documentation.

## Non-findings and rejected hypotheses

- No public API, error, state, wire, persistence, compatibility method, or
  runtime dependency was added. The new init functions are private.
- The stop/watcher map mutation itself is atomic. `LiveVm` is non-`Clone`, the
  winner installs unit `EndingInFlight`, the loser cannot own the cleanup
  capability, and no map guard crosses an await.
- The operator-stop writer/VMM composition is structured and awaited. Every
  writer task is completed or aborted and joined; no hidden-runtime or detached
  completion finding was established.
- Pre-EXEC EOF/malformed/SHUTDOWN are distinguished at the exact private File
  boundary, start no operator closure, and emit no EXIT. Their lack of the four-
  artifact native oracle is the approved population split, not a defect.
- READY remains before EXEC; Running remains owned by the host/action shim;
  Service Stable is not gated or reinterpreted.
- S-VLL-11 remains the sole ignored 01-03 measurement body. No result in this
  review is represented as a native latency distribution.
- The implementation diff does not touch the separately approved Landlock
  configuration. The qualified 15-test native module and host equivalence suite
  remain green, so no Landlock regression was established.
- No cleanup-error retry, persistence owner, stronger `Driver::stop Ok`, second
  terminal author, or adjacent hardening is requested by these findings.

## Verification results

| Command / evidence | Result |
|---|---|
| `git diff --check af158034..5edd751e` | PASS |
| Implementation parent and attribution audit | PASS — parent is the exact base; both commits retain the user author and exactly `Co-Authored-By: Codex <codex@openai.com>`. |
| DES-log audit | PASS mechanically — step 01-02 records ordered RED, GREEN, COMMIT events, all `EXECUTED/PASS`. G9 is independently failed by F3. |
| Lima worker/init/clone selection from roadmap | PASS — 37/37; nextest `097f0c3f-14f1-4623-8c18-d4f1d7e67e29`. |
| Qualified metal full VM-module filter excluding S11 | PASS — 15/15; nextest `7c2f933a-0fbd-40de-b165-17a94df9c59a`. |
| Qualified metal focused S10a/S10b | PASS — 2/2; nextest `a8f46bee-0df6-45a6-b7d5-3448068372e4`. |
| Qualified metal host VMM/host-state equivalence | PASS — 9/9; nextest `dda1f28b-56aa-47ef-81a9-1d760da5ab35`. |
| Focused S13 control-plane/Sim preservation selection | PASS — 13/13; nextest `e0e90f42-0ef2-4552-9680-bc00c9cc7bff`. |
| Focused pre-reaper watcher reproducer | PASS in 2.02 s while proving F1's wrong oracle; nextest `89618ea7-94ea-486e-8ca1-eb4bc95e36a5`. |
| Focused S07 | PASS in 0.02 s; nextest `1f44598d-71da-4b71-9f22-5ce95ccdac1e`. |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| `cargo fmt --all --check` | PASS |
| Crafter-reported full affected Lima run | 2,217/2,217 reported; not independently repeated because the focused and qualified gates above already reproduce the blocking contract/test defects. |
| Mutation testing | Correctly not run; repository policy reserves it for the final DELIVER-wave gate. |

RPP scanning stops at L1 because F2's speculative timed/yield machinery and
F6's contradictory documentation are already lower-level findings. Higher
levels were inspected for correctness and API conformance as required, but no
additional refactoring request is introduced.

## Iteration 1 disposition

| Finding | Status | Required owner/action |
|---|---|---|
| F1 | OPEN | Original 01-02 crafter restores reaper-before-report ordering; acceptance-test owner transitions the S13 oracle if required. |
| F2 | OPEN | Original crafter restores the exact four-call, unconditional cleanup path and exact owner routing. |
| F3 | OPEN | Acceptance-test owner restores exact post-EXEC typed-cause evidence; original crafter removes test-shaped marker behavior. |
| F4 | OPEN | Acceptance-test owner strengthens the existing real signal/status oracle; original crafter reuses the retained mapping path. |
| F5 | OPEN | Acceptance-test owner adds the required declaration and DDD-8 complement. |
| F6 | OPEN | Original crafter corrects only the stale source/test documentation. |

## Final verdict

**CHANGES_REQUESTED.** Green focused/native suites and clean compilation do not
override F1/F2's exact ADR-0103 ordering divergences or F3-F5's test-integrity,
coverage, and Contract Shape failures. Step 01-02 must remain closed to 01-03.
Return the implementation findings to the original 01-02 crafter, route any
acceptance-body construction to the acceptance-test owner, and re-run this same
reviewer after remediation. No design expansion is authorized or required by
this verdict.

---

## Iteration 2 — remediation re-review

### Iteration 2 metadata

| Field | Value |
|---|---|
| Review role | `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-11 |
| Iteration | 2 |
| Original implementation | `5edd751ec9c1e9e3e3916dbadc8f4e46703935b5` |
| Remediation commit | `493b793ffd5018dd3be75f083e13c949b7f517f3` |
| Remediation DES commit | `96bf8f1c634a09bfa2e2bbf91dc87f861c53b770` |
| Current verdict | **CHANGES_REQUESTED** |

### Remediation scope and integrity

The remediation commit changes only the two affected production owners,
their two acceptance surfaces, and `overdrive-init/Cargo.toml` documentation.
It adds no dependency, public item, state, error, wire frame, persistence
shape, compatibility method, cleanup owner, retry protocol, or measurement
surface. The cumulative base-to-remediation public-API scan is empty. S-VLL-11
remains the sole reasoned ignore for step 01-03. Both remediation commits
retain the user author and exactly the required Codex co-author trailer; the
DES log appends one GREEN and one COMMIT event for 01-02, both
`EXECUTED/PASS` and ordered after iteration 1.

The new private `shutdown_requested` fact keeps EOF following an already
accepted SHUTDOWN from replacing the successful group-termination result. It
does not suppress standalone post-EXEC EOF: the new private File/process test
still observes `InitError::Io(UnexpectedEof)`. This is bounded fallout needed
for the exact `EXIT 137` native stop result, not a new public state or general
reconnect policy.

### Prior-finding dispositions

#### F1 disposition — CLOSED

`drain_guest_report` now consumes the existing `VmExitWatch` without a new
deadline when the guest line wins the initial biased race
(`vm_driver.rs:2100-2114`). When the VMM watch wins, the bounded existing
guest-line drain retains that already-consumed result. Only after this join does
`run_exit_watcher` read the OOM fact, await the Running-confirmed gate, claim the
originating `LiveVm`, await cleanup, emit cleanup completion, and send its
`ExitEvent` (`:2175-2243`). The invented two-second timeout is gone.

The transitioned S13 test now proves all three distinct gates through the real
`VmDriver` composition: Running-confirmed release does not enter cleanup while
the `SimVmm` remains live; resolved VMM exit may enter but cannot pass a held
scope removal; only cleanup release permits one event, after VMM, run-directory,
cgroup, clone, and index absence (`vm_driver_stop_totality.rs:1746-1875`). Its
focused run is green. F1's exact reaper-before-report ordering is restored.

#### F2 disposition — PARTIALLY REMEDIATED, OPEN

The helper itself is corrected: `cleanup_driver_artifacts` now contains exactly
the four approved awaits, in order, with no timer
(`vm_driver.rs:1429-1436`). The natural watcher invokes it unconditionally after
`Some(LiveVm)`, with no intervening yield, and emits the cleanup event before
the existing event send (`:2217-2243`). Stop still invokes the same helper only
after its joined writer and awaited VMM composition (`:1750-1821`). These parts
of F2 are closed.

One exact routing clause remains unresolved. ADR-0103 lines 159-161 and the
feature delta lines 283-287 require `spawn_exit_watcher_task` to clone the
existing `self.cgroup_manager` and pass that one additional argument to
`run_exit_watcher`. Instead, `VmDriver::start` adds
`self.cgroup_manager.clone()` as a new parameter to
`spawn_exit_watcher_task` (`vm_driver.rs:1591-1600`), whose signature still
contains `cgroup_manager: CgroupManager` and merely forwards it
(`:1394-1425`). The remediation changed the helper's rustdoc to claim “it
clones” (`:1389-1392`), but no clone occurs in that function. This is the same
private contract divergence recorded in iteration 1, not a new architecture
request.

**Disposition:** OPEN. Remove the added `CgroupManager` parameter from
`spawn_exit_watcher_task`; clone `self.cgroup_manager` inside that existing
owner and pass it to the exact already-approved watcher position. Correct the
rustdoc to describe the code. No public or design change is required.

#### F3 disposition — CLOSED

The self-authored bare fatal marker is removed from `supervise_command`.
`post_exec_control_stream_faults_preserve_exact_typed_errors_at_file_process_boundary`
drives the approved private `exec_operator_command(&mut File, ...)` boundary
with a real child and distinguishes all three post-EXEC values:
`Io(UnexpectedEof)`, the exact `BeaconParse::UnknownKind`, and
`UnexpectedBeaconMessage::Exec` including its argv
(`overdrive-init/src/main.rs:1573-1637`). Native S10b now retains only the
qualified group/reap/poweroff/no-EXIT observations and explicitly delegates
typed-cause proof to that source-local boundary. The focused private-boundary
test and the full native module are green. This is stronger than the deleted
serial-console substring oracle and restores G9 integrity without changing an
error type.

#### F4 disposition — CLOSED

`reap_children` now stores a real `std::process::ExitStatus` for both exited and
signalled direct children, and `supervise_command` routes it through the
retained `exit_status_to_wire` SSOT (`overdrive-init/src/main.rs:1142-1154,
1180-1205,1253-1269`). The private real-process test checks the same production
entrypoint maps SIGKILL to 137 (`:1639-1663`). Qualified native S10a now requires
the complete EXIT-line population to equal exactly `['EXIT 137']`, not merely
at most one line (`vm_stop_restart_and_vmm_death.rs:3367-3381`). The real
15-test native module is green, so direct-child signal identity, exact value,
and cardinality are all observed.

#### F5 disposition — CLOSED

The transitioned S13 watcher test carries the exact
`/// CONTRACT_SHAPE: bounded-change.` declaration at
`vm_driver_stop_totality.rs:1763`. Its reaper, Running-gate, cleanup-pending,
artifact-complement, one-event, and idempotent-release assertions are
non-vacuous and run through the production `VmDriver` with doubles only at the
existing `Vmm` and `CgroupFs` ports. The prior two-second self-fulfilling oracle
is gone.

#### F6 disposition — PARTIALLY REMEDIATED, OPEN

The init module ownership note now describes the one PID-1 supervisor,
concurrent control/reaping, no post-EXIT read, and current qualified Tier-3
owner. The ClaimGuard Boolean wording and stop gate-drop wording are corrected.
The S-VM-76 sequence-(b) prose now says writer and VMM waits begin together,
and the already-dead sequence-(c) prose no longer claims a writer-first wait.

Two contradictions remain:

- `spawn_exit_watcher_task` says it clones the manager, while the clone is
  still performed by its caller and passed through an invented helper
  parameter (`vm_driver.rs:1389-1425`; the open F2 remainder).
- `resize_rejects_with_resize_unsupported_naming_gh_92` still says a later
  stop would await the two-second `SimClock` writer deadline and hang without a
  tick (`vm_driver_stop_totality.rs:1731-1737`). With the current biased
  concurrent stop composition, `SimVmm::terminate` resolves first, the writer
  is aborted/joined, and stop does not require that tick. The live test's name
  `stop_sequence_b_unresponsive_guest_escalates_after_deadline` likewise still
  states the superseded writer-first timing even though its amended rustdoc
  correctly says the VMM grace starts immediately.

**Disposition:** OPEN. Reconcile these remaining comments/name with the actual
concurrent stop and exact watcher-routing behavior. Documentation-only changes
must not alter the accepted lifecycle.

### New finding F7 — two remediation-transitioned S13 tests lack Contract Shape declarations

- **Severity:** Blocker
- **Dimension:** Mechanical Contract Shape compliance
- **Locations:**
  `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:962-999`
  and `:1421-1456`
- **Reachability:** Both are live S13 preservation tests on the exact stop
  composition changed by ADR-0103. The remediation edits their test contract
  prose from the old sequential writer/VMM order to the concurrent order, so
  both are transitioned tests under the repository rule.
- **Reproducer:** Direct source audit shows `#[tokio::test]` immediately follows
  each rustdoc with no `CONTRACT_SHAPE` declaration. Both execute in the green
  37-test selection; execution cannot supply missing mandatory metadata.
- **Disposition:** OPEN. Add
  `/// CONTRACT_SHAPE: bounded-change.` to each transitioned test without
  changing its behavior. Rename the sequence-(b) test consistently with its
  concurrent-wait contract while preserving the same oracle.

### Cumulative API, ordering, and test-honesty check

| Contract | Iteration 2 result | Evidence |
|---|---|---|
| No public API/state/error/wire/persistence change | PASS | Cumulative base diff adds no `pub` item; remediation changes only private code/tests/comments. |
| Exact init private signatures/errors | PASS | No signature or variant drift from iteration 1; new tests are feature-gated private complements. |
| Writer cap overlaps one VMM grace; every writer consumed | PASS | Production stop composition unchanged and 37-test/S07/S08 selections green. |
| Guest READY/EXEC/group/reap/direct status/poweroff | PASS | Exact private tests plus qualified native 15/15, including one `EXIT 137`. |
| VMM reaper before natural cleanup/report | PASS | F1 closed; S13 holds reaper and cleanup independently. |
| Exact four cleanup calls and unconditional watcher cleanup | PASS | F2 implementation body corrected; owner-routing sub-clause remains open. |
| Atomic unique `LiveVm` ownership and exactly-once cleanup | PASS | Stop/watcher still serialize on one `LiveMap`; only `Some(LiveVm)` can call the single helper; loser sees non-`Live`; S13/native artifact complements are green. |
| Typed pre/post-EXEC errors separated from native effects | PASS | F3 closed with the private real-process matrix and native S10 boundary. |
| Contract Shape compliance | **FAIL** | F5 closed; new F7 identifies two other transitioned S13 tests. |
| S11 remains pending for 01-03 | PASS | Sole matching `#[ignore]` is `native_lifecycle_profiles_meet_stage_targets_without_dropping_trials`. |
| Scope and no adjacent hardening | PASS | The private shutdown-after-accepted-request distinction is bounded; no new subsystem or public mechanism. |

The remediation adds two focused private behavior tests, bringing the
new/activated count to eight against the unchanged `2 × 6 = 12` budget. The
post-EXEC matrix is parameterized inside one body; no duplicate test inflation
or inside-hexagon mock was introduced. G9 is now PASS. Contract Shape remains
FAIL only for F7.

### Iteration 2 verification

| Command / evidence | Result |
|---|---|
| Remediation commit parent/scope/attribution | PASS — parent is `ffea32c6`; five bounded files; exact trailer and user author. |
| DES remediation commit | PASS — parent is `493b793f`; exact trailer; one GREEN then one COMMIT event appended. |
| `git diff --check ffea32c6..493b793f` | PASS |
| Focused F1/F3/F4/F5 plus S07/S08 Lima selection | PASS — 5/5; nextest `597994a6-9fe4-4e1c-aa90-f8b9779e15f2`. |
| Roadmap worker/init/clone Lima selection | PASS — 37/37; nextest `8b87040a-d646-4281-8035-4611ab69dcd3`. |
| Shared S13 control-plane/Sim selection | PASS — 13/13; nextest `c1507529-cdfd-4166-8c6b-92803f22ca2b`. |
| Qualified metal VM-module filter excluding S11 | PASS — 15/15 after bounded retry; nextest `8aec5c11-718c-47d7-be81-1675abd181f1`. |
| Qualified metal host VMM/host-state equivalence | PASS — 9/9; nextest `d14739b1-7f85-4874-982a-4f1cf1789820`. |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| `cargo fmt --all --check` | PASS |
| Crafter-reported affected Lima suite | 2,217/2,217; not repeated independently after the focused, preservation, native, compile, clippy, and format gates above. |
| Mutation testing | Correctly not run; final-wave gate only. |

The first independent 15-test native attempt timed out in
`restarted_vm_boots_from_a_clean_unmodified_rootfs_copy` and left its exact,
empty `alloc-job-dispatch-1-0.scope` residue. No Cloud Hypervisor process
remained. After removing only that identified test residue, the isolated test
passed 1/1 in 5.56 s (nextest `fee8e63d-0a21-4ae4-9835-61c0a291f189`) and the
complete identical 15-test filter passed. The initial timeout was not
reproduced and is not promoted to a production finding; neither a mechanism nor
a code change is inferred from it.

### Iteration 2 cumulative disposition

| Finding | Status | Evidence / next action |
|---|---|---|
| F1 | CLOSED | Existing VMM watch is consumed before OOM/gate/claim/cleanup/event; S13 proves the order. |
| F2 | OPEN, partially remediated | Four-call/unconditional cleanup fixed; move the manager clone into `spawn_exit_watcher_task` and remove its added parameter. |
| F3 | CLOSED | Exact private post-EXEC error matrix replaces the self-authored marker oracle. |
| F4 | CLOSED | Production uses mapping SSOT; private and native tests prove exactly one 137. |
| F5 | CLOSED | Exact declaration and complete reaper/gate/cleanup complement present. |
| F6 | OPEN, partially remediated | Correct remaining false clone, stop-wait comment, and stale test name. |
| F7 | OPEN | Add exact bounded-change declarations to the two transitioned S13 tests. |

### Iteration 2 final verdict

**CHANGES_REQUESTED.** F1, F3, F4, and F5 are closed, and the substantive
natural/post-EXEC reaper/cleanup order is now green through focused and
qualified native evidence. F2's exact private owner routing, F6's remaining
contradictory documentation, and F7's mandatory Contract Shape declarations
remain unresolved. Return only those bounded items to the original 01-02
crafter, persist its real GREEN/COMMIT work, and re-run this reviewer. Step
01-03 must not start yet. No DESIGN remediation or scope expansion is required.

---

## Iteration 3 — routing and Contract Shape remediation re-review

### Iteration 3 metadata

| Field | Value |
|---|---|
| Review role | `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-11 |
| Iteration | 3 |
| Remediation commit | `869d150a5b07e848f497bc87b796cb43a6684b22` |
| Remediation DES commit | `c46aece0e5f3d2baaac2e79991f8093e0cb275d0` |
| Current verdict | **CHANGES_REQUESTED** |

### Remediation integrity

The implementation commit is based exactly on the iteration-2 DES commit and
changes only `overdrive-worker/src/vm_driver.rs` plus
`overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs`. Its production
delta is two removed/relocated argument lines and one manager clone at the
accepted owner. The remaining changes are documentation, two Contract Shape
declarations, and one behavior-preserving test rename. No public item, state,
error, wire, persistence, cleanup behavior, test fixture, assertion, or runtime
dependency changes. Both commits retain the user author and exactly the
required Codex co-author trailer. The DES log appends one ordered GREEN and one
COMMIT event, both `EXECUTED/PASS`.

### Finding dispositions

#### F2 disposition — CLOSED

`spawn_exit_watcher_task` no longer accepts a `CgroupManager` parameter. It
clones `self.cgroup_manager` inside the exact existing owner and forwards that
owned clone between `exit_tx` and `cgroup_accounting` in the already-approved
`run_exit_watcher` signature (`vm_driver.rs:1389-1425`). The beacon-win call
site is restored to its pre-amendment parameter set (`:1591-1599`). The helper
rustdoc now matches the actual clone owner. The exact four-call helper,
unconditional watcher invocation, reaper ordering, and stop invocation from
iteration 2 are unchanged. F2 is fully closed.

#### F6 disposition — CLOSED

All remaining stale descriptions are reconciled without behavior changes:

- the watcher-spawn rustdoc now truthfully matches its internal manager clone;
- sequence B is renamed
  `stop_sequence_b_writer_and_vmm_waits_begin_together`, matching its existing
  concurrent writer/VMM contract (`vm_driver_stop_totality.rs:962-1004`);
- the resize-test teardown note now states that the biased `SimVmm`
  termination may resolve first and no writer-clock tick is required
  (`:1741-1747`);
- sequence C retains its correct already-gone VMM description and gains only
  required test metadata.

No false sequential 2+10-second composition remains in the remediated source
or test documentation. Historical wording in prior review iterations is
intentionally preserved as review history, not current implementation prose.
F6 is closed.

#### F7 disposition — CLOSED

Both transitioned S13 tests now carry the exact line
`/// CONTRACT_SHAPE: bounded-change.` immediately before their attributes:

- concurrent sequence B at `vm_driver_stop_totality.rs:967`;
- already-dead sequence C at `:1430`.

The added `#[allow(clippy::doc_markdown, ...)]` annotations are the established
repository shape. Sequence B's body and observable oracle are byte-unchanged
apart from the function name; sequence C's body is unchanged. The focused 4/4
and roadmap 37/37 selections execute the renamed and declared tests. F7 is
closed.

#### F1/F3/F4/F5 closure preservation

The iteration-3 production change is upstream argument routing only; it does
not touch `drain_guest_report`, `run_exit_watcher`, the four-call cleanup
helper, guest supervision/error handling, direct-status mapping, native S10,
or the three-gate S13 assertions. A cumulative diff read confirms:

- F1's VMM-exit join still precedes OOM, Running gate, atomic claim, cleanup,
  cleanup event, and natural `ExitEvent`;
- F3's three exact post-EXEC typed causes remain distinct at the private
  File/process boundary, with native effects separate;
- F4 still routes real direct-child `ExitStatus` through the retained mapper
  and the qualified native assertion remains exactly one `EXIT 137`;
- F5's declaration and reaper/cleanup/gate/artifact complement are unchanged.

All four prior closures remain valid.

### New finding F8 — the S13 roadmap locator was not updated with the required rename

- **Severity:** Blocker
- **Dimension:** Acceptance traceability; roadmap executable locator integrity
- **Location:** `docs/feature/vm-lifecycle-latency/deliver/roadmap.json:253`
- **Reachability:** `scenario_locators.S-VLL-13` is the approved mechanical map
  from the shared preservation contract to its executable test. The feature
  delta states that each locator expands to an existing exact function. The
  iteration-3 remediation renamed sequence B as required by F6/F7, so the
  locator participates in this bounded fallout.
- **Reproducer:**
  `rg -n 'stop_sequence_b_unresponsive_guest_escalates_after_deadline|stop_sequence_b_writer_and_vmm_waits_begin_together' . --glob '!target/**' --glob '!.git/**'`
  finds the new function only in the test source, while the active roadmap
  still names the removed old function. Prior review-history references are
  historical and correctly remain unchanged. A direct source lookup for the
  roadmap's current target fails.
- **Consequence:** The test suite is green because nextest discovers the new
  function independently, but roadmap-driven S13 selection/traceability points
  to a nonexistent test. A green suite cannot repair an orphaned authoritative
  locator.
- **Disposition:** OPEN. Mechanically replace only the S13 locator's old
  function name with
  `stop_sequence_b_writer_and_vmm_waits_begin_together`, validate the roadmap,
  and persist the bounded remediation. Do not rename the test back to the
  misleading sequential name or alter any criterion, scenario, design, test
  body, or production behavior.

### Cumulative contract and test-honesty check

| Contract | Iteration 3 result | Evidence |
|---|---|---|
| Exact ADR-0103 private/public API | PASS | Internal clone now exactly matches the pinned owner; no cumulative public diff. |
| Natural/post-EXEC reaper/claim/cleanup/event order | PASS | F1/F2 implementation closures unchanged; focused and roadmap selections green. |
| Atomic unique `LiveVm` and exactly-once cleanup | PASS | One locked move, unit `EndingInFlight`, one four-call helper, loser has no value/call. |
| Writer/VMM overlap and every writer consumed | PASS | Stop body unchanged; S07/S08 and 37-test selection green. |
| Guest group/reap/direct status/typed errors/poweroff | PASS | F3/F4 closures untouched; iteration-2 qualified native evidence remains current. |
| Contract Shape declarations | PASS | F5 and F7 declarations are present; no newly transitioned live test lacks metadata. |
| Test integrity and budget | PASS | No assertion or fixture changed; 8 tests remain within the 12-test budget. |
| S11 pending for 01-03 | PASS | The sole relevant ignore remains reasoned and untouched. |
| Roadmap scenario-locator integrity | **FAIL** | F8: S13 names the removed sequence-B function. |
| Scope / no adjacent hardening | PASS | Two-file remediation only; no new mechanism or behavioral expansion. |

### Iteration 3 verification

| Command / evidence | Result |
|---|---|
| Commit parent/scope/attribution | PASS — exact parent `96bf8f1c`; two files; user author; exact Codex trailer. |
| DES commit parent/scope/attribution | PASS — exact parent `869d150a`; execution log only; user author; exact Codex trailer. |
| DES event audit | PASS — one GREEN then one COMMIT appended for 01-02, both `EXECUTED/PASS`. |
| Focused F2/F6/F7 Lima selection | PASS — 4/4; nextest `6893a0d8-ebb4-4034-b418-86ad29677b04`. |
| Roadmap worker/init/clone Lima selection | PASS — 37/37; nextest `387409bd-8504-4b8c-afd1-5af20ce36b16`. |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| `cargo fmt --all --check` | PASS |
| `git diff --check 96bf8f1c..869d150a` | PASS |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-roadmap validate docs/feature/vm-lifecycle-latency/deliver/roadmap.json` | PASS structurally — `VALID: 1 phases, 3 steps`; this validator does not resolve locator function names, so it does not close F8. |
| Qualified native CLI 15/15 and host 9/9 | Not repeated: iteration-3 moves one clone across an immediate private call and changes no native behavior/test. Iteration-2 independent 15/15 (`8aec5c11`) and 9/9 (`d14739b1`) remain current; crafter independently reports both green at iteration 3. |
| Crafter-reported full affected Lima suite | 2,217/2,217; focused plus compile/clippy gates independently agree. |
| Mutation testing | Correctly not run; final-wave gate only. |

### Iteration 3 cumulative disposition

| Finding | Status | Evidence / next action |
|---|---|---|
| F1 | CLOSED | VMM exit remains consumed before the complete natural continuation. |
| F2 | CLOSED | Manager clone is inside the exact spawn owner; exact watcher position retained. |
| F3 | CLOSED | Exact post-EXEC typed-error matrix remains live. |
| F4 | CLOSED | Direct status still uses the mapper and native `EXIT 137` oracle. |
| F5 | CLOSED | Three-gate S13 declaration/oracle unchanged. |
| F6 | CLOSED | Current code, docs, comments, and sequence-B name agree. |
| F7 | CLOSED | Both exact bounded-change declarations present. |
| F8 | OPEN | Update the one orphaned S13 roadmap locator and validate it. |

### Iteration 3 final verdict

**CHANGES_REQUESTED.** The production implementation and executable tests now
satisfy the reviewed ADR-0103 contract, and F1-F7 are closed. Approval remains
blocked solely because the authoritative S13 roadmap locator names the removed
pre-remediation sequence-B function. Apply that one mechanical traceability
correction, persist the real GREEN/COMMIT work, and return to this reviewer.
Step 01-03 must not start until the on-disk locator and executable test agree.
No production, test-body, DESIGN, or architecture change is required.

---

## Iteration 4 — F8 locator remediation re-review

### Iteration 4 metadata

| Field | Value |
|---|---|
| Review role | `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-11 |
| Iteration | 4 |
| Locator remediation commit | `2f3cb4befc66de93186b527d820a67e4da80df84` |
| Remediation DES commit | `9d0b37d7e625e089c68363fc2bfe43234d8e635b` |
| Current verdict | **APPROVED** |

### Scope and integrity

The locator remediation commit is based exactly on the iteration-3 DES commit
and changes one line in
`docs/feature/vm-lifecycle-latency/deliver/roadmap.json`: one string deletion
and one string insertion. The phase/step structure, validation status,
criteria, scenario identities, commands, implementation scope, and every other
locator are byte-unchanged. No production, test, design, dependency, API,
state, error, wire, persistence, or behavior file changed.

Both commits retain the user author and exactly
`Co-Authored-By: Codex <codex@openai.com>`. The DES commit changes only
`execution-log.json` and appends one GREEN then one COMMIT event for 01-02,
both `EXECUTED/PASS` and ordered after iteration 3.

### F8 disposition — CLOSED

`scenario_locators.S-VLL-13` now names exactly:

```text
crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs::stop_sequence_b_writer_and_vmm_waits_begin_together
```

The function exists at
`crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:973`, and
the old `stop_sequence_b_unresponsive_guest_escalates_after_deadline` locator
is absent from the active roadmap. The named test executes through Lima and
passes 1/1. `des-roadmap validate` reports `VALID: 1 phases, 3 steps`; unlike
the structural validator alone, the direct source-resolution check proves the
locator's function component exists.

**Disposition:** CLOSED. The approved S13 traceability map and executable test
now agree, with no criterion or behavior change.

### Prior closure preservation and cumulative verdict

F1-F7 remain closed because the iteration-4 implementation commit changes no
production or test source. The cumulative evidence from iterations 1-3 remains
current:

- the exact ADR-0103 private/public API and init error shapes are preserved;
- the VMM exit is consumed before OOM, Running gate, atomic claim, exact
  four-call cleanup, cleanup event, and natural `ExitEvent`;
- stop and watcher move one unique `LiveVm`, only the winner cleans, and unit
  `EndingInFlight` preserves supervision/reclamation exclusion;
- writer cap and one VMM grace overlap, every writer is consumed, and no
  cleanup work is detached;
- guest READY/EXEC, process-group termination/reaping, direct status,
  post-EXEC typed errors, exactly one native `EXIT 137`, and immediate poweroff
  remain independently proven;
- all transitioned tests carry their exact Contract Shape declarations, G9 is
  clean, and the eight new/activated tests remain within the twelve-test
  budget;
- S-VLL-11 remains reasoned-pending for 01-03 and no latency distribution is
  claimed by this step;
- the separately approved Landlock path and all public states/errors/
  persistence/LWW authorship remain unchanged.

No new finding is established.

### Iteration 4 verification

| Command / evidence | Result |
|---|---|
| Locator commit parent/scope/attribution | PASS — exact parent `c46aece0`; roadmap only; one replacement; user author and exact Codex trailer. |
| DES commit parent/scope/attribution | PASS — exact parent `2f3cb4be`; execution log only; user author and exact Codex trailer. |
| `git diff --check c46aece0..2f3cb4be` | PASS |
| Direct old/new locator scan across roadmap and test source | PASS — active roadmap and existing source function match exactly; old active locator absent. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-roadmap validate docs/feature/vm-lifecycle-latency/deliver/roadmap.json` | PASS — `VALID: 1 phases, 3 steps`. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-worker --features integration-tests -E 'test(stop_sequence_b_writer_and_vmm_waits_begin_together)'` | PASS — 1/1; nextest `2993f506-a131-45f4-9937-c4e6817b6eb7`. |
| Production/test/native/compile gates | Not repeated: locator-only remediation cannot change their result. Iteration 3 independently passed focused 4/4, roadmap 37/37, workspace check/clippy/format; iteration 2 independently passed qualified native CLI 15/15 and host 9/9. Crafter reports the unchanged full affected suite 2,217/2,217. |
| Mutation testing | Correctly not run; final DELIVER-wave gate only. |

### Iteration 4 cumulative disposition

| Finding | Status |
|---|---|
| F1 | CLOSED |
| F2 | CLOSED |
| F3 | CLOSED |
| F4 | CLOSED |
| F5 | CLOSED |
| F6 | CLOSED |
| F7 | CLOSED |
| F8 | CLOSED |

### Iteration 4 final verdict

**APPROVED.** No required remediation remains for step 01-02. The exact
ADR-0103 host/guest termination contract, DDD-8 cleanup ownership/order,
S-VLL-07 through S-VLL-10, shared S-VLL-13, Contract Shape metadata, native
evidence, commit scope/attribution, DES history, and roadmap traceability are
consistent and green. Step 01-02 may advance to step 01-03.
