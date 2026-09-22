# Adversarial review — step 03-02

- **Feature:** `netns-density-295`
- **Step:** `03-02` — Worker EXEC-release acceptance: gate-coupled beacon-writer schedules
- **Reviewer:** fresh isolated DELIVER `nw-software-crafter-reviewer`
- **Review ID:** `code_rev_20260922_201415_iteration_1`
- **Iteration:** 1
- **Reviewed cumulative range:** `c40d35394dd02a2eb62fb03f3ff661370cd14f75..0a94cbba5de399b65d7297d9c796e7bbee6988cc`
- **Implementation commits:** `7fa9c94185f315e2863fa13e2254fd5436722d23`, `0a94cbba5de399b65d7297d9c796e7bbee6988cc`
- **Accepted DISTILL correction authority:** `7a50786553748608097b3e1fb2e26c5664a689ec`, `e74cf14c5ea799565abb30a4df447fd36f2d41d9`, approved by `c40d35394dd02a2eb62fb03f3ff661370cd14f75`
- **Final verdict:** **CHANGES_REQUIRED**

## Authority and scope

This review used the approved step `03-02` in
`docs/feature/netns-density-295/deliver/roadmap.json`, the accepted contracts
in `feature-delta.md`, `distill/test-scenarios.md`,
`distill/red-classification.md`, the approved D-295-DELIVER-03-01 evidence
allocation, the listed architecture references, and the durable DISTILL
remediation review at `deliver/review-distill-remediation-03-02.md`.

The implementation range was inspected together with the post-DISTILL parent
and the actual production callers. The primary step file is
`crates/overdrive-worker/tests/acceptance/netns_density_exec_release.rs`.
Mapped S-ND295-28 evidence was also inspected in
`crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs`,
`crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs`, and
`crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs`.

No production, test, DISTILL, design, roadmap, or DES log file was changed by
this reviewer. The review artifact below is the sole intended write.

## Strengths

- The cumulative implementation diff is intentionally narrow: commit `7fa9c941`
  removes only the three step-owned reasoned `#[ignore]` markers from the
  primary worker acceptance file; commit `0a94cbba` records the DES COMMIT
  phase. No production file or public API was changed.
- The exact accepted constructor and release path are used by the three
  primary schedules. `driver()` calls the seven-argument `VmDriver::new` at
  `netns_density_exec_release.rs:97-110`; `start()` calls production
  `VmDriver::start` and observes the real Unix beacon at `:126-139`; each
  release schedule calls the existing `Driver::release_for_exit_emission`
  operation at `:171-173`, `:223-225`, or `:288-290`.
- The corrected claim-before-detection oracle is fail-closed. Every
  newline-complete frame is UTF-8 decoded and parsed by the production
  `BeaconMessage` parser before the exact empty-or-one-`Exec` slice assertion at
  `netns_density_exec_release.rs:246-269`. Only a final unterminated suffix is
  excluded, as authorized by S-ND295-28 and the accepted DISTILL remediation.
- The recovery schedule keeps a Recovering release future pending and checks
  that no EXEC is observed before `complete_attempt(None)` at
  `netns_density_exec_release.rs:167-187`. The FailStop schedule checks the
  real waiter wakes without writing EXEC and retains the allocation at
  `:284-301`. The mapped writer, stop-deadline, and cancellation schedules all
  passed independently through the existing production writer/stop owners.
- Every newly transitioned/step-owned S-ND295-28 body inspected has the
  required explicit Outcome anchor and Contract Shape declaration. The
  pre-existing writer schedule retains its named `S-VLL-08` outcome and
  bounded-change declaration, which the accepted DISTILL review explicitly
  permits as the named S-VLL outcome. The primary file has three declarations
  and no remaining ignore marker; the generated core model, the other worker
  stop/cancellation schedules, and the action-shim body retain their explicit
  declarations.
- The test diff contains no weakened, deleted, or replaced assertion. The
  accepted fail-closed oracle correction predates this step's implementation
  range and was independently approved; step `03-02` only activates its body.

## Contract Shape Compliance

**Overall: PASS for the changed activation surface; the step remains blocked by
the two evidence/traceability findings below.**

| Check | Result | Evidence |
|---|---|---|
| `CONTRACT_SHAPE` declaration on every live mapped body | PASS mechanically | Primary worker bodies at `netns_density_exec_release.rs:155-156,196-197,272-273`; core model at `netns_density_exec_gate.rs:140-143`; mapped worker and action-shim bodies retain their declarations. |
| Required Outcome anchor | PASS under the accepted DISTILL mapping | Newly transitioned bodies have `/// Outcome anchor: DISCUSS Elevator Pitch`; the pre-existing writer body retains its named `S-VLL-08` outcome, the explicit exception recorded by the accepted DISTILL review (`test-scenarios.md:785-788`). |
| Banned technical test names | PASS | No mapped live function matches the repository's banned `test_.*(returns_[0-9]+|exit_code|calls_.*_once|status_code|http_[0-9]+)` pattern. |
| Accepted bounded-change universe | PASS for the accepted bodies | The corrected typed-frame universe and complement are exactly the accepted DISTILL contract; the existing writer/stop/cancellation bodies retain their previously accepted owner-level universes. |
| Production composition boundary | **FAIL for one mapped body** | The action-shim body enters `action_shim::dispatch`, but its direct fixture selects the historical `HostNetworkProvisioner` composition and never reaches the injected release owner; see D2. |
| Private `active_claims` oracle | PASS | No body inspects private gate storage. S-ND295-27 owns the public gate model; S-ND295-28 owns real `VmDriver` claim lifetime, writer acknowledgement, and cancellation as approved by D-295-DELIVER-03-01. |

## Blocking findings

### D1 — The literal roadmap generated-model selector has no worker test

- **Severity:** Blocker
- **Dimension:** Roadmap traceability / verification contract
- **Locations:** `docs/feature/netns-density-295/deliver/roadmap.json:390,397,405,415`; authoritative body `crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs:139-276`; worker file `crates/overdrive-worker/tests/acceptance/netns_density_exec_release.rs:1-301`

The exact roadmap verification command is scoped to `overdrive-worker` and
selects `generated_operation_sequences_match_the_gate_model`. The independent
writable-Lima run of that literal selector started zero tests and exited 4:

```text
Starting 0 tests across 1 binary (102 tests skipped)
error: no tests to run
exit status: 4
```

The authoritative generated body is instead in
`overdrive-core/tests/acceptance/netns_density_exec_gate.rs:143`, and the exact
core selector passed one test with 528 unrelated tests skipped under
`PROPTEST_CASES=1024`. The worker acceptance file contains no function with
that name. The accepted DISTILL mapping describes S-ND295-28 as the core gate
PBT plus deterministic real-`VmDriver` schedules, while S-ND295-27 owns the
complete core gate evidence. Thus this is not a production failure and does
not justify adding a second worker PBT, a seeded writer, or any API/test seam.

It is nevertheless a blocker against the exact approved roadmap as written:
the required literal verification command fails, and the step description's
criterion 1 also says the generated model drives the real `VmDriver`, which is
not what the authoritative core model does. The DES GREEN event records this
zero-test failure but still records the step as `PASS`.

**Required disposition:** reconcile the roadmap verification command and
criterion wording with the accepted DISTILL ownership, or obtain an explicit
authoritative correction before approval. The correction must preserve the
existing split: the core PBT remains the gate model and the real worker tests
remain the writer/claim-lifetime evidence. No production change is required.

### D2 — The action-shim owner schedule times out before release ownership is reached

- **Severity:** Blocker
- **Dimension:** External validity / production-composition evidence
- **Locations:** `crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs:483-524,1095-1188`; production dispatch composition `crates/overdrive-control-plane/src/action_shim/mod.rs:924-1022,1224-1282,1391-1435,2089-2537`

The exact mapped selector was rerun through writable Lima as root:

```text
cargo xtask lima run -- sh -c 'CARGO_TARGET_DIR=/tmp/codex-netns-density-target TMPDIR=/tmp cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E "test(=integration::mtls_install_fail_closed::start_allocation_awaits_release_and_cancellation_owns_the_future)" --no-fail-fast'
```

Result: nextest run `cac36d80-7e74-49e5-a4d0-ffd899c2ef11`, 0 passed, 1
failed, 208 skipped, after 10.012 seconds. The failure is the bounded
assertion at `mtls_install_fail_closed.rs:1170`:

```text
dispatch reaches the post-install async release: Elapsed(())
```

This is not an environment-only failure. The writable Lima build completed,
the test ran as root, and the timeout is the test's own `release_entered`
boundary. The complete current path is:

1. `dispatch_one` calls the public integration `action_shim::dispatch` at
   `mtls_install_fail_closed.rs:502-523`. Its `Some(worker)` argument is the
   mTLS lifecycle port, not a `SharedGuestNetworkOwner` or
   `GuestNetworkProvisioner`.
2. `action_shim::dispatch` calls `dispatch_with_network_provisioner` with the
   historical `HostNetworkProvisioner` and no guest owner
   (`action_shim/mod.rs:924-964,981-1022`).
3. Under the integration-test configuration,
   `StartAllocation` enters `provision_and_inject_netns` before driver start;
   with `guest_provisioner == None` it takes the legacy slot/netns/veth/TAP
   branch at `action_shim/mod.rs:1391-1435`, then reaches the mTLS and release
   sequence only if that legacy path completes.
4. The test's `HoldingReleaseDriver::release_for_exit_emission` would set
   `release_entered` at `mtls_install_fail_closed.rs:463-471`, but the observed
   state never reaches that call. Therefore the subsequent cancellation,
   `release_cancelled`, and `on_alloc_running` assertions at `:1173-1188` are
   not executed evidence.

The real production owner path is different and reachable. Ordinary
`dispatch_with_workflow_intent` calls `dispatch_with_network_owner` at
`action_shim/mod.rs:1205-1216`; when the boot-composed
`state.shared_guest_network` exists, that path passes the same owner through
`dispatch_with_network_provisioner_and_guest` at `:1261-1282`. The normal
`run_server` composition stores the one boot owner in `AppState` at
`crates/overdrive-control-plane/src/lib.rs:3725-3728`. The source explicitly
describes `HostNetworkProvisioner` as a historical fixture adapter
(`action_shim/mod.rs:786-795`) and uses `ProductionNetworkGuard` in the
production owner path.

Consequently, this reproduction does **not** prove a reachable production
defect in the normal shared-owner composition, and it does not authorize a
production change. It proves that the mapped acceptance body is a stale
fixture/composition boundary: it does not drive the accepted shared-owner
composition and therefore does not prove criterion 5's action-shim
await/cancellation contract. The accepted DISTILL statement that this body
drives the existing production dispatch owner is not true of the committed
body.

**Required disposition:** return this as a DISTILL/acceptance-composition gap.
The authoritative acceptance body/mapping must be corrected to drive the
existing production owner composition (or explicitly reclassify the body and
its evidence boundary) before S-ND295-28 can be approved. Do not add a new
public API, worker reverse edge, detached task, or production fallback in
this step. Until the corrected boundary is independently approved and the
selector reaches the release-entered state, criterion 5 remains unproven.

## Roadmap criteria assessment

| Criterion | Result | Evidence |
|---|---|---|
| 1. Generated operation model | **BLOCKED by D1** | The authoritative core model passed; the literal worker selector has no tests and the roadmap wording conflicts with the accepted core-model/real-owner split. |
| 2. Writer overlaps VMM grace and every writer is consumed | PASS | `writer_bound_overlaps_the_single_vmm_grace_and_every_writer_is_consumed` passed through the existing real `BeaconWriter`/VMM owner. |
| 3. Backpressure does not delay stop deadline | PASS | `backpressured_exec_release_cannot_delay_stop_deadline` passed; stop reached its existing deadline and the release task was joined. |
| 4. Cancellation leaves no EXEC sender running | PASS | `cancelling_backpressured_release_cannot_leave_an_exec_sender_running` passed through the production writer and VMM termination boundaries. |
| 5. Action-shim start awaits release and cancellation owns the future | **FAIL / BLOCKED by D2** | Exact selector timed out before `release_entered`; the current fixture uses legacy `dispatch` composition and never observes the release owner. |
| 6. Every step-owned reasoned body is active without weakening | PASS for the primary step file | All three primary reasoned ignores were removed; no ignore marker remains in the primary file, and the cumulative test diff only removes those markers. The mapped writer/stop/core/action bodies were already live. |

## Quantitative validation

The accepted S-ND295-28 mapping groups the eight bodies into five observable
behavior groups: gate model/transition closure; recovery and FailStop release;
claim-before-detection cancellation; stop/writer lifetime and backpressure;
and action-shim ownership. The test budget is therefore `5 × 2 = 10` bodies.
The mapping contains eight bodies, so the numerical budget passes. The failed
action-shim composition and stale literal selector are evidence failures, not
permission to add parallel test bodies.

## Test integrity, architecture, and TDD gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected activation set | PASS for the primary file | The three reasoned bodies in `netns_density_exec_release.rs` are live and no ignore marker remains. |
| G2 — valid RED | PASS for the actual changed oracle | DES RED records a semantic assertion failure from the authored claim-before-detection body; the remediation history records the prior raw-prefix contradiction and accepted fail-closed correction. |
| G3 — assertion failure, not collection failure | PASS for executed bodies | Worker, core, and writer/stop/cancellation selectors collected and ran; the action body fails at its bounded assertion timeout, not at collection. |
| G4 — no mocks inside the hexagon | PASS | Core uses real opaque capabilities; worker schedules use real `VmDriver`/`BeaconWriter` ports with SimVmm and filesystem/cgroup adapters; no seeded writer or private gate accessor was added. |
| G5 — business/Contract Shape language | PASS mechanically under the accepted mapping | Explicit Outcome anchors/declarations are present on transitioned bodies; the pre-existing writer body retains its accepted named `S-VLL-08` outcome and declaration. |
| G6 — all required tests green | **FAIL** | The required action-shim selector fails, and the literal roadmap worker generated selector has no tests and exits 4. |
| G7 — green before commit | **FAIL for the complete step** | DES GREEN was recorded despite the known action-shim failure and zero-test literal selector; the independent review does not credit that as complete step evidence. |
| G8 — test budget | PASS numerically | Eight mapped bodies are within the ten-body budget. |
| G9 — no test modification to accommodate implementation | PASS | `git diff` shows only removal of three `#[ignore]` attributes in the primary file; no assertion was weakened, deleted, or skipped. |

External validity is **FAIL** for the action-shim criterion. The other
production-owner schedules enter through their declared driving ports and
observe the real beacon/writer/VMM boundaries. No testing-theater or fixture
theater finding is made against the three primary worker schedules: this is an
explicit activation-only step and the accepted DISTILL classification says the
production behavior is already present. D2 is specifically a stale composition
boundary, not a claim that the `HoldingReleaseDriver` itself is an invalid
test double.

No RPP smell finding is raised. The step changed no production code, and the
required remediation is evidence-boundary/traceability work rather than
adjacent refactoring or architectural hardening.

## DES phase evidence

The current `execution-log.json` entries for `03-02` are ordered:

1. `RED` at `2026-09-22T16:06:33Z`, recording the authored semantic failure;
2. a superseding `RED` at `2026-09-22T16:20:49Z`, correcting the description
   from a complete EXEC to a raw partial prefix and preserving the oracle
   contradiction as an acceptance issue;
3. `GREEN` at `2026-09-22T17:51:23Z`; and
4. `COMMIT` at `2026-09-22T17:53:39Z`.

The phase order and commit event are mechanically present. The GREEN
description is also unusually candid: it records the core selector's
authoritative location, the literal worker selector's zero-test result, the
workspace clippy baseline, and the action-shim timeout. However, a GREEN/PASS
event cannot close a roadmap criterion whose exact selector failed. D1 and D2
therefore remain blockers despite the otherwise valid DES ordering.

`des-verify-integrity --roadmap-only` passes the roadmap validator. Full
integrity verification reports only that future steps `03-03` and `04-01`..
`04-04` have no entries; step `03-02` has all three phase entries and is not
the source of that future-step report.

## Independent verification

All commands below used writable Lima target space where compilation or Linux
execution was required.

| Command / evidence | Result |
|---|---|
| `cargo xtask lima run -- sh -c 'CARGO_TARGET_DIR=/tmp/codex-netns-density-target TMPDIR=/tmp cargo nextest run -p overdrive-worker --test acceptance -E "test(netns_density_exec_release)" --no-fail-fast'` | **PASS** — run `8f616b4a-8308-423d-b4f2-bfa6adac0fe2`; 3 passed, 99 skipped. |
| Exact primary selectors for recovery, claim-before-detection, and FailStop | **PASS** — run `9b968b5c-26e2-44d8-97b7-710fa9eddf42`; 3 passed, 99 skipped. |
| `PROPTEST_CASES=1024` exact core `generated_operation_sequences_match_the_gate_model` selector | **PASS** — run `05ac875e-8014-4450-bc63-1f2356473da6`; 1 passed, 528 skipped. |
| Literal roadmap worker generated selector `-p overdrive-worker ... test(=acceptance::generated_operation_sequences_match_the_gate_model)` | **FAIL** — run `bec1b35b-bc3d-4073-ad4c-ff5eccc5d8a7`; 0 tests, nextest exit 4 (`no tests to run`). |
| Mapped writer/stop/cancellation selectors in `vm_driver_stop_totality` | **PASS** — run `ad6a4e70-1e26-494e-863a-8f7b6739cef0`; 3 passed, 99 skipped. |
| Mapped action-shim selector | **FAIL** — run `cac36d80-7e74-49e5-a4d0-ffd899c2ef11`; 0 passed, 1 failed, 208 skipped; timeout at `mtls_install_fail_closed.rs:1170`. |
| `cargo xtask lima run -- sh -c 'CARGO_TARGET_DIR=/tmp/codex-netns-density-target TMPDIR=/tmp cargo check --workspace --all-targets --features integration-tests'` | **PASS**. |
| Workspace clippy with `-D warnings` | **FAIL, baseline/out of scope** — `overdrive-netlink/src/nft.rs:5121` (`clippy::print_stderr`); the step diff has no production change. |
| Focused `cargo clippy -p overdrive-worker --lib -- -D warnings` | **PASS**. |
| Focused acceptance-target clippy with integration features | **FAIL, baseline/out of scope** — 29 existing `overdrive-control-plane` guest-network/reconciler diagnostics; no changed step file is implicated. |
| `cargo fmt --all -- --check` | **PASS**. |
| `git diff --check c40d35394dd02a2eb62fb03f3ff661370cd14f75..HEAD` | **PASS**. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python /Users/marcus/.claude/bin/des-verify-integrity --roadmap-only docs/feature/netns-density-295/deliver` | **PASS** — roadmap validator reports format OK. |
| Direct Contract Shape/Outcome/ignore scan across all eight mapped bodies | **PASS under accepted mapping** — declarations and explicit/named anchors are present; no remaining ignore marker in the primary or mapped files. |
| Host-native Rust execution | **Not used for authoritative results** — this macOS environment is not the required Linux execution lane; authoritative selectors ran in Lima. |
| Mutation testing | **NOT RUN**, as required; it is reserved for the final DELIVER gate. |

## Commit, scope, and worktree audit

- `7fa9c94185f315e2863fa13e2254fd5436722d23` retains Marcus Schack Abildskov
  as author and changes only the primary activation file plus the accumulated
  DES execution log.
- `0a94cbba5de399b65d7297d9c796e7bbee6988cc` retains Marcus as author and
  changes only the DES execution log.
- Each implementation commit's message contains exactly one
  `Co-Authored-By: Codex <codex@openai.com>` line and no Claude, Anthropic, or
  generated-by attribution. Each also contains `Step-Id: 03-02`.
- The cumulative range contains only
  `crates/overdrive-worker/tests/acceptance/netns_density_exec_release.rs` and
  `docs/feature/netns-density-295/deliver/execution-log.json`.
- Pre-existing dirty `AGENTS.md` and `.serena/project.yml` were preserved,
  untouched, and excluded from every implementation commit and this review's
  intended commit.
- No mutation testing ran.

## Remediation dispositions

| Finding / item | Disposition |
|---|---|
| D1 — literal worker generated selector and real-`VmDriver` wording | **OPEN blocker.** Reconcile the roadmap verification/criterion with the accepted DISTILL split, or obtain an authoritative roadmap correction. Do not add a parallel PBT or writer seam. |
| D2 — action-shim release/cancellation owner evidence | **OPEN blocker.** Surface as a DISTILL/acceptance-composition gap. The current direct legacy `dispatch` fixture does not reach the release boundary; no production change or new API is authorized. Re-review only after the approved evidence boundary is corrected and the exact selector exercises it. |
| Fail-closed typed-frame oracle | **ACCEPTED and closed.** The body matches the approved e74cf14c/c40d3539 correction and passed the writable-Lima selector. |
| Public/API shape | **PASS.** No new method, type, enum variant, trait, parameter, persisted field, writer seam, or private-state accessor was added. |
| Primary activation scope and test integrity | **PASS.** Only step-owned ignores were removed; no assertion was weakened or re-authored. |
| Mutation testing | **DEFERRED correctly** to the final DELIVER gate. |

# CHANGES_REQUIRED
