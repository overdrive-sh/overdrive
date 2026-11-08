# DELIVER Review — 01-02: Run ordinary verification concurrently with owned guest cleanup

## Review metadata

- Feature: `metal-run-isolation`
- Step: `01-02`
- Iteration: 1
- Reviewed commit: `2ba2232208ce9a5e91666f61b6b9845d9797c62a`
- Prior accepted step: `01-01`, commit `ee0bbd65805524afa4a6348b5019ce12a7390e8b`
- Review baseline: `0e8ae265c` (with preserved test/lint commit `4bc17cc74` before the target commit)
- Reviewer: fresh independent `nw-software-crafter-reviewer`
- Review date: 2026-11-08
- Implementation technical verdict: **APPROVED**
- Step acceptance status: **PENDING mandatory native validation**
- Findings accepted for source remediation: **0**
- Findings rejected as unproven hypotheses: **0**

The technical verdict covers the committed implementation against the accepted P2–P5 interface, ownership, lifecycle, error, and placement contracts, plus the integrity of the assigned pre-authored test activations. It does not claim that the complete 01-02 acceptance criteria have passed or authorize advancement to a later roadmap step. The roadmap explicitly requires the implemented-path native owner cases and all three existing external drivers before GREEN and independent step approval; that proof is still pending.

## Authority and reviewed artifacts

The contract is the accepted P2–P5 portion of [feature-delta.md](../feature-delta.md), [ADR-0278](../../../product/architecture/adr-0278-metal-ordinary-runs-share-the-maintenance-barrier.md), [ADR-0279](../../../product/architecture/adr-0279-metal-guest-resources-belong-to-the-durable-invocation.md), [ADR-0280](../../../product/architecture/adr-0280-metal-nextest-execution-uses-explicit-test-placement.md), and [ADR-0281](../../../product/architecture/adr-0281-metal-run-completion-follows-owned-resource-cleanup.md). The reviewed acceptance and activation authority is [roadmap step 01-02](roadmap.json), [test-scenarios.md](../distill/test-scenarios.md), the integrated activation list and native gate in `feature-delta.md`, and the approved DISTILL reviews. Step 01-01 remains accepted by [review-01-01.md](review-01-01.md); P1 is not reopened.

The review inspected target commit `2ba223220`, its exact diff from `4bc17cc74`, the accepted P2–P5 artifacts, existing implementation and test bodies, the committed ordinary-concurrency attempt evidence, and the live DES log. The step commit changes the files named by the roadmap, including the complete static placement policy. It does not change the acceptance bodies or model actions: the acceptance/conformance test diffs remove only the assigned `pending DELIVER` attributes. The separately retained `4bc17cc74` test/lint edits predate this step and remain untouched.

The current worktree contains pre-existing service-feature edits and an `AGENTS.md` memory-context update. They were not included in the target commit and were not modified by this review. The post-commit DES COMMIT event remains a dirty update in `execution-log.json`; it is preserved as authored. The native-evidence commit `59a9296d1` retains the proof attempt after target commit `2ba223220`.

## Iteration 1 — comprehensive contract and test-integrity review

## Summary

The committed implementation follows the approved two-lock order: ordinary durable invocations take the shared physical-maintenance barrier and their bound checkout lock; exclusive maintenance takes the exclusive physical-maintenance barrier and waits for admitted ordinary units to stop. The service retains controller detachment, one admission deadline, per-run owner records, v1 collection compatibility, and the exact P2 runner failure codes.

The nextest path is closed over the reviewed binary/test placement inventory. Host-pure selections retain the original runner chain; native-launcher and guest selections receive the approved local-mode vsock namespace; guest attempts use the actual selected test executable, libtest arguments, Nextest environment/helper inputs and attempt result. The existing explicit suite retains its batching. Missing and malformed placement/context refuse before selected-body execution.

The P5 finalizer uses the existing transient unit, runs after systemd's stop lifecycle, removes only the three named invocation roots, and publishes an ordered completion receipt. The v2 collector waits for the unit stop, validates the command terminal and completion receipt, retains actual command status, and performs no cleanup or record writes. The v1 reader path remains separate.

No public surface beyond the designed `metal run --exclusive` mode is introduced. The private 18-position durable launch carries the three approved added values. No new scheduler, CID allocator, guest pool, recovery protocol, product API, or duplicated test framework was introduced.

## Contract and implementation audit

### P2 — physical-maintenance admission, checkout exclusion, and durable owner

| Accepted promise | Implementation evidence | Result |
|---|---|---|
| Ordinary invocations share the existing physical-maintenance barrier; explicit exclusive work keeps exclusive admission | `bootstrap.sh:444-448,777-829` chooses the existing barrier and sends execution class to the durable owner. `lease-holder.sh:869-889` takes shared mode for `ordinary` and exclusive mode for `exclusive`. | Pass |
| Checkout exclusion is acquired after maintenance admission and held across transfer, selected command, and supervisor result publication | `lease-holder.sh:909-923,925-968` takes the existing per-checkout lock after the maintenance descriptor and publishes the invocation/tree owner records only after both locks are held. The bootstrap no longer has the old release/reacquire window around START. | Pass |
| Same-checkout sync/pull and runs exclude one another; different ordinary checkouts can overlap | Sync and pull use the canonical checkout lock at `bootstrap.sh:742-750`; durable runs use the same lock at `lease-holder.sh:909-923`. The maintenance descriptor is shared for ordinary owners at `:879-883`. | Pass by implementation; the required native overlap observation is still pending. |
| A single monotonic admission deadline covers the maintenance and checkout waits | Deadline is established once at `lease-holder.sh:785-795`; each later wait uses `remaining_lock_timeout` before flock at `:874-920`. The exclusive predecessor-stop wait uses that same deadline at `:891-906`. | Pass |
| Ordinary owner records are invocation-specific; only exclusive runs publish the singleton maintenance owner | `lease-holder.sh:925-995` writes the per-run owner and checkout owner for each durable run and the singleton maintenance owner only for `execution_class=exclusive`. Token-matched cleanup is at `:823-836`. | Pass |
| An exclusive maintenance holder waits for already-admitted ordinary units to complete systemd stop/cleanup; a waiter that has not acquired shared admission does not block it | `lease-holder.sh:414-456,891-906` scans v2 ordinary records in `preparing`/`running` and waits while the actual unit is live, within the current deadline. `waiting_for_lease` is not treated as admitted. | Pass by implementation; native `admission_journey.sh` remains pending. |
| START is one-shot; controller loss before and after START retains the designed distinction | `bootstrap.sh:696-729,1117-1124`; `lease-holder.sh:1004-1030,1033-1038`. START is sent once; the service records EOF-before-START or retains the detached command after START. | Pass |
| v1 and older exclusive callers preserve their old record/authority meanings | `lease-holder.sh:102-141,572-738` accepts exactly one matching metadata v1/v2 file and uses the existing v1 collection path without a completion requirement. `bootstrap.sh:303-341` enforces the new flag's valid mode. | Pass |

### P3 — namespace and invocation-owned resource lifecycle

| Accepted promise | Implementation evidence | Result |
|---|---|---|
| The wrapper creates an anonymous parent namespace, sets only that namespace's child mode to `local`, creates/verifies its local child, then starts the selected process | `test-wrapper.sh:147-198` performs the two `unshare(CLONE_NEWNET)` operations, writes only `/proc/sys/net/vsock/child_ns_mode`, checks read-only `ns_mode`, and `execvpe`s the selected program. | Pass by implementation; native KVM/vsock owner proof remains pending. |
| Existing CID bands and actual listener ports remain in use; no CID window/allocator was added | `guest_lane.rs` now uses the beacon returned by `HostListeners::bind` at its existing listener owner; CID-band definitions remain unchanged. The dispatcher reuses the existing `GuestRun` group CIDs in `fwd_guest_suite.rs:3367-3466`. | Pass |
| Guest staging, VMM runtime, and command scratch paths are owned by one run ID | `vm_fixture.rs` selects `/srv/vm/metal/<run-id>` for valid Metal context; `guest_kit.rs:423-438,452-455,576-588` selects `/run/ovd-gk/<run-id>`. `bootstrap.sh:793-796,1080-1087` provides the run-specific command temp root and context. | Pass |
| The finalizer removes only `/srv/vm/metal/<run-id>`, `/run/ovd-gk/<run-id>`, and `<remote-dir>/target/metal-run-tmp/<run-id>` | `lease-holder.sh:203-235,237-285` validates exact canonical non-symlink paths and removes no source checkout, Cargo target, module, or host policy. | Pass by implementation; actual root complements remain pending native proof. |
| No raw-host/global-CID fallback or on-demand dispatcher build | `test-wrapper.sh:212-224` refuses missing dispatchers and `:158-197` refuses namespace failure before exec. `guest-kit-prepare.rs:277-291` builds the dispatcher as the preparation command; the wrapper only checks for it. | Pass |

### P4 — command authority, placement, and actual attempt result

| Accepted promise | Implementation evidence | Result |
|---|---|---|
| Ordinary commands are finite; arbitrary host commands and execution/config/artifact overrides refuse before START | `bootstrap.sh:95-235` recognizes the approved command forms, `:237-266` bounds manifest/target artifacts, and `:324-341` checks nextest wrapper and target-runner configuration before durable registration. `unsupported_ordinary_command`, `execution_override_refused`, and `artifact_scope_mismatch` return the approved 64. | Pass |
| `--exclusive` exists only on the run mode and preserves arbitrary administrative command authority | `main.rs` adds the explicit `Run { exclusive, ... }` argument; `bootstrap.sh:303-310` rejects invalid mode combinations. The README documents the existing command boundary. | Pass |
| Nextest resolves every selected identity through the closed reviewed inventory; absence, malformed descriptors, and duplicate identities fail closed | `test-wrapper.sh:58-144` validates the complete JSON schema and exact `(binary_id, test_name)` entry. It maps duplicate-key parser status to `placement_ambiguous`, absence to `test_unclassified`, and malformed policy to `placement_invalid`; every case returns 78 before selected-body execution. The assigned runtime-placement acceptance body is unchanged and was reported passing in Lima. | Pass by implementation and reported adapter evidence; profile/native external proof remains pending. |
| Host-pure, native-launcher, guest-group, and guest-lane placements retain distinct execution boundaries | `test-wrapper.sh:205-235` keeps host-pure on the original chain, applies the namespace to native-launcher, and dispatches the exact guest descriptor to the existing dispatcher. The two guest-lane identities retain their existing host-driver path. | Pass |
| The actual Nextest-selected ELF, arguments, helper executables, environment, retry identity, and status are preserved | `fwd_guest_suite.rs:3090-3131` stages the actual runner executable and `NEXTEST_BIN_EXE_*` helpers; `:3045-3087` retains the approved environment; `:1419-1447,3344-3364,3532-3645` invokes the selected binary with unchanged libtest arguments and returns the actual peer status. The wrapper does not substitute a stale test ELF. | Pass by implementation; the native `placement_journey.sh` attempt/helper/retry proof remains pending. |
| The explicit guest suite retains existing batching; each direct guest nextest attempt gets a fresh instance of the existing group | `fwd_guest_suite.rs:1385-1417,3594-3645` creates one new `GuestRun` for the exact selected attempt, while the ordinary explicit suite path below `:3693` retains the pre-existing grouped suite implementation. | Pass |
| Non-Metal and explicit exclusive execution preserve the original runner chain | `test-wrapper.sh:25-35` passes through without Metal context and explicitly in exclusive context. The corresponding pre-authored acceptance case is unchanged except activation. | Pass by implementation and reported Lima evidence. |

Nextest wrapper rules are configured at `profile.default` in `.config/nextest.toml:1016-1022`; repository `ci` and `mutants` profiles inherit from default. The `run-wrapper` profile rule therefore remains in their inheritance chain. This matches Nextest's documented profile inheritance and wrapper-script configuration ([profile inheritance](https://nexte.st/docs/configuration/), [wrapper scripts](https://nexte.st/docs/configuration/wrapper-scripts/)).

### P5 — systemd-owned cleanup and read-only completion collection

| Accepted promise | Implementation evidence | Result |
|---|---|---|
| Completion is recorded by the existing transient unit's `ExecStopPost`, not by a new service or the collector | `bootstrap.sh:811-829` embeds this invocation's captured lease-helper bytes in `ExecStopPost`. `lease-holder.sh:1167-1175` dispatches the private `--finalize` entry. | Pass by implementation; native stop/finalizer ordering proof remains pending. |
| The finalizer runs after the existing mixed-mode stop, cleans only the three owned roots, and retains either exact complete receipt or the typed cleanup-failure root | `lease-holder.sh:148-201,203-285` validates the exact allowed roots and writes the ordered `completion.v1` record atomically. | Pass by implementation; real immutable-root failures and process complements remain pending native proof. |
| v2 collection waits for stopped owner, then validates terminal and receipt; collection is read-only and repeatable | `lease-holder.sh:572-738` waits for the actual unit, validates both retained records, emits the actual terminal/log result and does not write or clean anything. v1 collection remains independent. | Pass by implementation and reported Lima fixture evidence; native owner stop is pending. |
| Missing terminal, missing/malformed receipt, cleanup failure, and actual command status preserve the accepted distinctions | `terminal_valid` and `completion_valid` are at `lease-holder.sh:148-178,309-347`; the collector gates v2 results at `:641-669,671-721`; `terminal_result` preserves the declared status mappings at `:497-569`. | Pass for the implemented outer record/status mappings. The real v2 owner-stop and cleanup-failure driver observations remain pending. |

## Semantic error mapping

The mapping below follows the accepted finite error table in [feature-delta.md, Typed failures](../feature-delta.md#L421). P1 mappings were also independently accepted in step 01-01; this review records them to show that integrated P2–P5 changes preserve the complete caller-visible taxonomy.

| Designed semantic failure | Approved representation | Implemented path and retention | Status |
|---|---|---|---|
| Invalid Git discovery or detached ordinary clone | `checkout_identity_invalid` | Existing bootstrap identity refusal; unchanged from accepted 01-01 mapping. | Pass |
| Invalid override | `remote_subdir_invalid` | Existing bootstrap override validation; unchanged from accepted 01-01 mapping. | Pass |
| Unbound, malformed, or differently bound checkout | `checkout_identity_conflict`; registered run tagged ABORT retains the same runner error | `bootstrap.sh:43-48,485-553,696-714`; `lease-holder.sh:1016-1019`; collector maps it to 78 at `:545-548`. | Pass |
| No-sync binding/source marker mismatch | `source_mismatch`; registered run tagged ABORT retains that error | `bootstrap.sh:51-56,959-963,696-714`; `lease-holder.sh:1016-1019`; collector returns 78 at `:545-548`. | Pass |
| Physical-maintenance/admitted-predecessor deadline | `maintenance_timeout` | Durable shared/exclusive wait uses the one deadline at `lease-holder.sh:874-906`; terminal is retained and collector returns 75 at `:529-540`. | Pass |
| Own checkout deadline | `checkout_timeout` | The same deadline applies to the checkout lock at `lease-holder.sh:914-922`; terminal is retained and collector returns 75 at `:541-543`. | Pass |
| Registered source/context/rsync preparation failure | `runner_preparation_failed`, optional immediate `prerequisite=rsync`; tagged ABORT and P5 finalization; valid collection returns 75 | `bootstrap.sh:59-72,920-925,1064-1115`; accepted ABORT at `lease-holder.sh:1021-1024`; collector returns 75 at `:541-543`. | Pass |
| Existing native/source/owner prerequisite failure | `preflight_failed`, with immediate approved `probe` context; registered terminal retains the outer code | `native-preflight.sh:5-9,63-69`; bootstrap observes marker and sends tagged ABORT at `bootstrap.sh:1097-1115`; collector returns 78 at `lease-holder.sh:545-548`. | Pass |
| Unsupported ordinary command | `unsupported_ordinary_command` | `bootstrap.sh:80-83,95-212,324-325`; rejected before registration/START, status 64. | Pass |
| Bypassing nextest/config/target-runner execution policy | `execution_override_refused` | `bootstrap.sh:85-88,326-341`; rejected before registration/START, status 64. | Pass |
| Foreign manifest/target artifact path | `artifact_scope_mismatch` | `bootstrap.sh:90-93,237-266`; remote canonical path check permits only the bound checkout or invocation scratch, status 64. | Pass |
| Invalid `--exclusive` mode | Public CLI's existing Clap parse failure; direct bootstrap `invalid_mode` | `main.rs` declares the flag only on `MetalAction::Run`; `bootstrap.sh:75-78,303-310`; direct bootstrap status 64, public parse status 2. | Pass |
| Invalid run context | `invalid_run_context`, `field=<run_id|execution_class|source_owner>` | `test-wrapper.sh:5-16,33-55`; refuses before body with status 78. | Pass |
| Invalid / duplicate / missing placement | `placement_invalid` / `placement_ambiguous` / `test_unclassified` | `test-wrapper.sh:58-64,103-143`; exact statuses and fail-closed body boundary, status 78. | Pass |
| Namespace setup operation failure | `namespace_setup_failed`, exact `step` and `errno` | `test-wrapper.sh:147-197`; status 70 before selected body. | Pass |
| Required dispatcher/guest input not prepared | `lane_not_prepared` | `test-wrapper.sh:212-224`; dispatcher is prepared only by the existing setup command; status 78. | Pass |
| Image/VMM/peer guest execution failure | `guest_execution_failed` plus the existing `GuestKitError` cause; wrapper status 70 and Nextest retains actual aggregate behavior | `fwd_guest_suite.rs:3606-3635,3662-3673` routes guest-kit failures to the typed outer marker and emits the original error's debug diagnostic; the external native fault path was not executed in this review. | Code path reviewed; native failure cause/status evidence remains pending. |
| Finalizer cannot remove an owned root | `cleanup_failed`, exact `guest_staging|vmm_runtime|command_tmp` root; collector 74 | `lease-holder.sh:190-201,237-285,655-665`; actual terminal is retained and failure names the first failing exact root. | Pass by implementation; native producer/collection evidence pending. |
| Corrupt or missing retained artifact | Existing `run_artifact_io` | `lease-holder.sh:463-466,608-637,641-669`; collector returns 74 and does not infer completion. | Pass |
| Invalid ID / unknown run / supervisor loss | Existing `invalid_run_id` / `run_not_found` / `supervisor_lost` | `lease-holder.sh:589-605,717-734`; existing 64/66/70 values retained. | Pass |
| Existing ABORT/EOF/launch/log errors | Existing exact runner values and precedence | `lease-holder.sh:1011-1030,1055-1158,549-563`; v1 result reader remains compatible. | Pass |
| Actual selected command/body status | Exact 0–255 terminal / peer code; Nextest owns its aggregate mapping | `lease-holder.sh:362-376,1128-1158`; `fwd_guest_suite.rs:1419-1447,3586-3592,3677-3691`. | Pass by implementation; the native 255/aggregate-100 driver assertion is pending. |

No new public error type, variant, payload, product API, or wire record was added. The P2 v2 metadata and P5 completion record have exactly the accepted ordered fields. The native-only error cases above are not credited as exercised merely because their adapter code exists.

## Findings

No source or test-integrity finding was accepted in iteration 1. The unexecuted native owner/error paths are recorded below as pending validation, not as code defects. No contract was waived and no additional mechanism is requested.

## Test integrity, TDD evidence, and Contract Shape

The target diff from `4bc17cc74` removes only the assigned reasoned ignores from the existing acceptance, owner-conformance, and two qualification-transition bodies. It adds or removes no assertion, fixture, test identity, expected value, `#[cfg]`, or `CONTRACT_SHAPE` declaration. The assigned bodies and their authored state/complement oracles remain DISTILL-owned. The existing native-only `selected_guest_body_uses_its_own_kernel_in_existing_profiles` and three standalone expected timeout/reset/status bodies retain their deliberate ignored-only selection.

Every transitioned live body already carries its required Contract Shape declaration; no source-local pure-function property was added, so the Rustdoc `CONTRACT_SHAPE: pure-function.` requirement has no new instance. No new unit/property test was authored in this step. The pre-authored Rust acceptance, real-owner conformance, shell E2E, and external native driver boundaries remain distinct; no test spawns the built Overdrive product binary, and no external expectation was used as a Rust test oracle.

The live execution log records step 01-02 RED failures at `16:40:05Z` and `17:26:21Z`, GREEN PASS at `20:48:46Z`, then COMMIT PASS at `20:50:59Z`. The COMMIT event is the preserved post-commit dirty update, not evidence inserted into target commit `2ba223220`. This order is consistent with the replacement crafter independently executing its own RED and then GREEN/COMMIT; this reviewer wrote no DES events.

The implementation report retained 32 assigned acceptance cases passing in Lima, 16 deliberate external-only ignores, 24 `overdrive-testing` library tests passing, both transitioned qualification tests passing, scoped Clippy/check/format results, and shell syntax results. These are existing reported results, not commands rerun by this reviewer. The reported full-workspace format failure concerns untouched baseline files `guest_peer.rs` and `xtask/tests/acceptance.rs`; scoped formatting of owned files passed.

## Contract Shape Compliance

- **Declaration preservation:** PASS. The target changes only remove the roadmap-assigned pending attributes; each transitioned body retains its pre-authored Contract Shape line.
- **Test identity/body preservation:** PASS. No acceptance or conformance body was authored, rewritten, weakened, or deleted by the target commit.
- **Outcome and complement assertions:** PASS by retained DISTILL bodies and reported adapter run. The body edits are absent, so no oracle could have been weakened by this commit.
- **Placement identity coverage:** PASS by the unchanged authored placement comparison plus the reported Lima result. The complete closed identity map is checked in; no fixed-count or absence-means-host rule was introduced.
- **External-only cases:** PASS. The explicit native body, timeout/reset inputs, and 255 guest input retain their ignored-only execution policy.

## Equal-weight subtractive review

Each added mechanism maps to an accepted P2–P5 obligation: the shared/exclusive barrier and checkout lock, invocation records and receipt, the local namespace, closed test placement, and per-run roots. The 15,060-line placement policy is the approved exact identity inventory and DISTILL extension, not speculative generality. The dispatcher reuses existing guest groups and nextest's wrapper/retry owner; no new guest pool, scheduler, allocation broker, recovery protocol, or new test framework was added. The finalizer is the approved existing-unit hook and does not prune source/build artifacts or another invocation's roots. No adjacent service-feature work was changed.

No L1–L2 issue was accepted as a required correction. A possible concern about a failure path is not upgraded to a finding without the repository-required production-path reproduction; the unexecuted native error and owner cases remain validation work, not permission to add speculative machinery.

## Verification disposition and native gate

The reviewer ran **no tests or checks**, performed no mutation testing, and performed no Metal/native/SSH/remote operation. This respects the explicit stop on checks requiring the current exclusive Metal lease and avoids repeating reported passing checks. The review used retained local code and evidence only.

The preserved [ordinary concurrency proof attempt 1](native-evidence/ordinary-concurrency-attempt-2026-11-08/README.md) is explicitly pending, not a K1–K3 pass. Its two registered ordinary runs, IDs `554fc291d793ef06d84a7928` and `d12aee8a2ce2bf57507197d7`, timed out before START behind an existing exclusive unit. Neither selected body started. The original owner from that attempt was subsequently reported complete by the orchestrator's read-only collection; this did not mutate or interrupt it.

The separately retained [ordinary concurrency proof attempt 2](native-evidence/ordinary-concurrency-attempt-2026-11-08-attempt-2/README.md), added by evidence-only commit `66669d66e`, also does not pass K1–K3. Its raw logs and read-only snapshots are distinguished below. `admission_journey.sh` and `placement_journey.sh` remain unrun. The native owner-conformance selector also remains pending because the ARM Lima preflight correctly returned `preflight_failed probe=arch`.

### Supplemental evidence review — ordinary concurrency attempt 2

This reviewer inspected the retained README, both controller streams, driver stderr, owner observations, and selected raw JSON snapshots. No Metal, SSH, remote command, test, or check was run.

| Evidence source | Direct observation | What it establishes |
|---|---|---|
| Raw `first.stderr` and `second.stderr` | Run IDs `e83202b4ffbbf9fb44c4a194` and `be3b95671f4ab77148735ae2` were registered. Each stream prints `FATAL: timed out after 5s acquiring /run/lock/overdrive-metal-shared.lock` and `OVERDRIVE_METAL_RUNNER_ERROR error=maintenance_timeout`. Neither contains `OVERDRIVE_METAL_RUN_STARTED`. | Both ordinary invocations reached registration/admission wait and stopped before START. The raw streams do not print a numeric process exit status. The README's status-75 statement is the evidence author's reported result, consistent with the implemented mapping; it is not a numeric status in these captured streams. |
| Raw owner record quoted by each controller | Both callers identify PID `1758326`, unit `overdrive-metal-run-9e34103f0e184c634e5cd7af.service`, workspace `bangalore-v1`, commit `036032935f101fc29e6d79d69d2796e9345a2b23`, started `2026-11-08T21:02:45Z`, with source digest `19e8a3fb8636bb72b2e7a3508d11c764a320767a5f9f382f98755d5143b5073c`, on the canonical shared lock. | This is read-only owner attribution reported by the callers. The captured owner fields show `action=run`, not `execution_class` or selected command. The orchestrator identifies this as a different exclusive Bangalore run from attempt 1; that class attribution is not in this owner-record excerpt. No command is inferred. |
| New-run metadata and controller log text | Raw `overlap-71.json` shows `execution_class=ordinary` and `phase=waiting_for_lease` for both new run IDs. Controller stdout also contains the generic bootstrap log text `acquire exclusive physical-maintenance barrier`. | The metadata is the relevant class record. The generic log line does not establish that either ordinary service acquired an exclusive descriptor. |
| `overlap-71.json` | Run `e832…` already has `terminal.v1 kind=runner error=maintenance_timeout` and a complete receipt. Run `be3…` is `phase=waiting_for_lease`, with two owner processes and no terminal or receipt yet. | The snapshot shows one earlier timed-out attempt and one later waiter, not two simultaneously executing ordinary owners. |
| `overlap-87.json` | Both retained run records have `phase=waiting_for_lease`, `terminal.v1 kind=runner error=maintenance_timeout`, complete receipts, and no owner processes. | Both attempts ended without START. These are terminal/finalizer observations, not guest, selected-command, or overlap results. |
| Raw `driver.stderr` and `observations.log` | The driver's expected-running-owner assertions fail; the poll log records the admission observation interval. | The journey did not observe the required simultaneous live ordinary owners. The assertion failures are consequences of the exclusive barrier wait and do not establish a production concurrency defect. |
| `before.json` and captured physical fields | The capture contains an initial physical snapshot and later snapshots; the pair never started the selected guest suite. | These are not K1–K3 evidence. No claim of zero forbidden physical-kernel deltas, complete cleanup, or unchanged survivor state is credited from this attempt. |

The raw timestamps also show the two registered waits were sequential: `e832…` registered at `21:24:03Z`, timed out at `21:24:08Z`, and `be3…` registered at `21:24:22Z`, timing out at `21:24:27Z`. Local compilation of the second worktree appears in its captured stderr before its `xtask metal run` invocation; it delayed that registration until after the first timeout. This attempt therefore did not observe two simultaneously waiting admissions. The evidence author's README marks the attempt pending; its phrase “two ordinary owners remained in `waiting_for_lease`” describes each run's own wait state, not a period when both were waiting together.

The prior attempt-1 owner completion, confirmed separately through the orchestrator's read-only collection, does not change attempt 2's evidence: the raw attempt-2 controller streams identify a different active exclusive owner. No production finding is raised from this blocked pair, and the implementation technical verdict remains unchanged.

The exact remaining gate is the existing approved final native proof in `feature-delta.md:822-835`: after the physical-maintenance barrier is available, prepare both current worktrees and run the one bounded `worktree_journey.sh` capture plus the existing `admission_journey.sh` and `placement_journey.sh` drivers. Record the actual source, owner, placement, body, cleanup, kernel/process/root complements and each command result. Do not interpret the current admission timeouts as successful native evidence, do not use `--exclusive` for the paired ordinary run, and do not bypass or interrupt the existing owner. No contract has been waived.

| Roadmap acceptance layer | Evidence status |
|---|---|
| Implementation/API/record/ownership source review | **APPROVED** |
| Assigned Lima acceptance and library results | **Reported PASS; not rerun** |
| Existing qualification transitions | **Reported PASS; not rerun** |
| Full native ordinary overlap and K1–K3 complements | **PENDING — both retained attempts stopped before START; attempt 2's owner waits were sequential** |
| Native exclusive admission/stop/finalizer proof | **PENDING — driver not run** |
| Native selected ELF/helper/retry/profile/status/timeout/reset proof | **PENDING — driver not run** |
| Mutation testing | **NOT RUN — explicitly waived for this feature** |

## Remediation dispositions

- No production or test remediation was requested. No defect was proven through an unmet production owner path.
- The native acceptance evidence is not a code-remediation finding. It remains a mandatory step-validation gate and has not been waived; no test was rerun because the user stopped current exclusive-lease checks.
- No architectural redesign, new API, persistence mechanism, lifecycle state, recovery path, test seam, or broadened scope was proposed.
- Pre-existing dirty service-feature files, the dirty post-commit DES event, and the retained evidence were left untouched by the reviewer.

## Verdict

**APPROVED — implementation technical review.** The committed step matches the exact P2–P5 contracts at the reviewed code, record, CLI, ownership, lifecycle, and placement boundaries. No proven implementation or test-integrity finding requires remediation.

**The roadmap step is not complete and may not advance on this artifact alone.** Its mandatory implemented-path native validation remains **PENDING**: both retained ordinary-pair attempts stopped before START, the second attempt's two admission waits were not simultaneous, and the other two native drivers were not run. This is not a waiver or a claim that the overall native outcome passed. Preserve that gate; do not schedule another exclusive-host attempt while the explicit stop remains in effect. Record the native result only if and when it is authorized and actually produced.
