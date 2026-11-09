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
- Implementation technical verdict: **APPROVED** — Iterations 26–27 review the proven guest-lane environment-helper and exact-CWD input corrections against the accepted P4 contract. Post-fix native guest-lane execution and full step acceptance remain pending.
- Step acceptance status: **PENDING mandatory native validation**
- Current focused implementation review: **APPROVED** — P2 queued-EOF owner behavior, P4 descriptor-field parsing, and bounded guest-lane environment-helper plus exact-CWD staging have each been independently reviewed. Post-fix native placement remains pending.
- Findings accepted for source remediation: **0**
- Findings rejected as unproven hypotheses: **0**
- Unproven hypotheses awaiting bounded reproduction: **0**

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

During iteration 1, the reviewer ran no tests or project validation checks, performed no mutation testing, and performed no Metal/native/SSH/remote operation. This respected the explicit stop on checks requiring the current exclusive Metal lease and avoided repeating reported passing checks. The review used retained local code and evidence only.

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

The remaining gate is the existing approved final native proof in `feature-delta.md:822-835`: run the complete `worktree_journey.sh` capture with the corrected collection launcher, plus the existing `admission_journey.sh` and `placement_journey.sh` drivers. The partial `/tmp` capture reviewed below contains valid overlap and first-survivor observations but stops before the second collection and final `after.json` complements. Record the full source, owner, placement, body, cleanup, kernel/process/root complements and each command result. No contract has been waived.

| Roadmap acceptance layer | Evidence status |
|---|---|
| Implementation/API/record/ownership source review | **APPROVED** |
| Assigned Lima acceptance and library results | **Reported PASS; not rerun** |
| Existing qualification transitions | **Reported PASS; not rerun** |
| Full native ordinary overlap and K1–K3 complements | **PENDING — attempt 5 reached overlap, survivor, collection and final snapshots, but exact raw physical equality failed on two RA `expires` countdowns; the approved projection correction has not been natively rerun** |
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

**The roadmap step is not complete and may not advance on this artifact alone.** Its mandatory implemented-path native validation remains **PENDING**: attempts 1 and 2 stopped before START; a later capture reached overlap, survivor, repeated collection and final snapshots but failed exact raw physical equality on two RA expiry countdowns. The focused projection correction is approved but has not been natively rerun, and `admission_journey.sh` and `placement_journey.sh` remain unrun. No contract has been waived and this review does not claim full native acceptance.

## Iteration 2 — focused review of the retained-collection output fix

### Review metadata

- Scope: commit `1421c23314b7b33f048f4a4c24ca8609a46d25c4`, limited to the `worktree_journey.sh` collect launcher and its accompanying evidence note.
- Review type: bounded harness-fix review; this is not a new DELIVER source implementation step or a new native proof attempt.
- Result: **APPROVED**.

### Finding and evidence

The change is one line in `infra/metal/tests/worktree_journey.sh`: the `invoke` helper now runs `cargo --quiet xtask metal ...`. The helper is used for the two read-only collection and repeat-collection pairs; the two ordinary run invocations remain unchanged. All exact `cmp` calls on retained collector stdout and stderr, the command-status checks, the completion-marker assertions, and the final physical/root complement assertions remain intact. No output is piped, filtered, normalized, or rewritten.

The offline capture at `/tmp/metal-native-retry-20261108T214703Z` reproduces the issue the fix addresses. A local byte comparison of the retained files found `first.collect.stdout` and `first.repeat.stdout` identical; the stderr diff is only Cargo's first `Finished ... target(s) in 0.22s` line versus `0.23s`. The `Running` command, `xtask` diagnostic, and `OVERDRIVE_METAL_RUN_COMPLETE ... exit_code=0 cause=command_returned` marker are unchanged. The capture's exact stdout/stderr comparison therefore failed only on a Cargo timing banner, not on collector output or an actual product diagnostic. The installed `cargo --help` confirms `--quiet` is a Cargo global option before the subcommand and defines it as suppressing Cargo log messages. The child `xtask` still writes directly to the redirected stdout/stderr files; the existing completion-marker oracle would fail if those outputs were hidden.

The same retained capture independently records that the journey had already passed its earlier live-overlap and first-completion/survivor assertions before the collection comparison stopped it. `overlap.json` contains two active `ordinary` owners, each with a live Cloud Hypervisor holding `/dev/vhost-vsock`; both use CID `16785416` in distinct network namespaces (`net:[4026532936]` and `net:[4026532989]`). `survivor-boundary.json` contains one inactive owner with `command_returned`/0, complete receipt, no processes, and absent owned roots, plus one continuing active owner. The driver writes those named snapshots only after its corresponding assertions pass. This is evidence from the failed original driver capture, not a rerun of commit `1421c233`.

The capture stops at the first repeated-collection stderr comparison. It has no second in-driver collection pair or final `after.json`; it does not establish the remaining repeat-collection, second-owner completion, or final physical-kernel/root complements. The later evidence report about a separate canonical collect does not fill those missing assertions in this capture. The native proof remains **PENDING**.

### Subtractive review and remediation disposition

The fix removes only Cargo's nondeterministic launcher banners at their source. It preserves the exact retained-output oracle and all existing failure visibility, while leaving ordinary execution, systemd ownership, product records, and guest behavior untouched. No production finding, test weakening, API change, model change, or remediation is required for this bounded fix.

### Verification disposition and verdict

This focused review inspected the diff, retained offline output files, the exact stdout/stderr comparison result, and installed Cargo help. It ran no tests or project validation checks and no native, Metal, or SSH/remote operation; the local `cmp`/`diff` operations only read the preserved capture.

**APPROVED — focused collection-output harness fix.** This approval means the `--quiet` change is a faithful, non-weakening correction to the collection comparison. It does not advance the overall DELIVER step: the completed native overlap capture still lacks the in-driver repeat collection and final after-complements, and the mandatory full native-validation gate remains **PENDING**.

### Native-capture evidence reviewed for the focused fix

The retained failed capture at `/tmp/metal-native-retry-20261108T214703Z` had already passed the earlier assertions before reaching the collection comparison:

- `overlap.json` records two active ordinary owners, each in `phase=running` with a live Cloud Hypervisor process holding `/dev/vhost-vsock`. Both VMM command lines use CID `16785416`; their network namespace identities are distinct (`net:[4026532936]` and `net:[4026532989]`).
- `survivor-boundary.json` records the first owner as inactive with a `command_returned` terminal of exit code 0, a complete receipt, no processes, and absent owned roots. The second owner remains active. The driver only writes this file after its pre-authored source, checkout, root, and surviving-VMM assertions pass.
- The first read-only collection and repeat collection have byte-identical stdout. Their stderr differs only in Cargo's elapsed-time banner (`0.22s` versus `0.23s`); `xtask`'s read-only message and `OVERDRIVE_METAL_RUN_COMPLETE ... exit_code=0 cause=command_returned` are identical.

The original driver then stopped at `cmp` on that Cargo banner. It did not run its second in-driver collect/repeat pair or write the final `after.json`; those results remain unproven by this capture. These observations establish a genuine partial native result and the exact harness-only failure. They do not complete the accepted K1–K3 outcome or the remaining two native drivers. The focused fix is approved so the root can run the complete authorized native workflow; this reviewer did not rerun it.

## Iteration 3 — focused review of the deterministic survivor-boundary stimulus

### Review metadata

- Scope: commit `b98f2d105ebb2c137df3c5b112b03c959d9e8c2b`, limited to the existing external `worktree_journey.sh` stimulus/assertions and the author’s completeness note.
- Review type: bounded review of the native-driver determinism adjustment; no DELIVER production code or public interface changed.
- Result: **APPROVED**.

### Approved owner path and assertion audit

The added sequence uses only the approved existing owner actions. DESIGN's action table permits `SupervisorLost` by terminating the real durable main process through its existing systemd owner, followed by the existing blocking `StopOwner` operation (`feature-delta.md:510-512`). The permanent model includes the named `supervisor_loss_is_not_a_result` path, `Start → SupervisorLost → StopOwner → Collect` (`specs/quint/metal-run-isolation/runner_protocol.qnt:194-195`). The driver does this against the first test-owned run unit after it has observed two real active ordinary owners and killed that run's local controller: `systemctl kill --kill-whom=main --signal=KILL` followed by blocking `systemctl stop` (`worktree_journey.sh` in commit `b98f2d105`). This is a real systemd owner operation, not a production test seam, a simulated future state, or a test-only replacement of the owner.

The fault is explicitly distinguished from command completion. For the stopped first owner, the driver requires both collection statuses to be 70, the exact `supervisor_lost` runner marker, identical repeat stdout/stderr, no `terminal.v1`, completed cleanup receipt, no surviving processes, and no owned roots. It requires the second owner to remain live with identical checkout, source marker, roots, and VMM namespace through the survivor observation, then to finish all 86 core bodies with its actual command status 0. The final `after.json` assertions still compare the physical-kernel complement and both owners' process/root cleanup. No expected result is relaxed to an arbitrary status.

The stopped first run's result is a runner failure `supervisor_lost`; this driver does not claim it as an actual command result preserved after controller loss. The pre-authored controller-only owner case `admission::independent_ordinary_owners_overlap_after_controller_loss` and named protocol case `protocol_controller_loss_after_start` remain unchanged and retain the actual-command-result contract; their earlier Lima PASS is reported, not rerun here. The native driver retains the actual controller disconnect before the additional declared supervisor-loss stimulus, but only credits the first invocation's supervisor-loss result. This distinction preserves the accepted error taxonomy and avoids inferring command success after a missing terminal.

The adjustment addresses the retained qualitative failure at `/tmp/metal-native-retry-20261108T221035Z`: raw `overlap.json` showed two active ordinary owners with the same CID in disjoint namespaces, while the retained boundary samples show both finite core commands finishing at `2026-11-08T22:12:33Z` with command status 0 and complete receipts before any one-complete/one-live sample was captured. The fault stimulus creates a deterministic supported owner-failure boundary after the real overlap, so the existing survivor assertions can observe one completed owner and one continuing owner. No production defect is claimed from the prior timing outcome.

### Findings and remediation disposition

- No production, API, model, or test-contract finding was accepted.
- The systemd `SupervisorLost → StopOwner` stimulus is already named by DESIGN and exercised against the real test-owned transient unit. It does not add a pause hook, state accessor, or recovery promise.
- The driver's first-owner oracle intentionally changes from natural `command_returned`/0 to the approved `supervisor_lost`/70 outcome after an explicit supervisor kill. The native journey therefore does not prove first-command completion following controller-only loss; that claim remains in the unchanged `admission::independent_ordinary_owners_overlap_after_controller_loss` and `protocol_controller_loss_after_start` cases. Do not credit this driver as native evidence for that controller-only command result. It does retain exact status/marker/repeat assertions for the separate supervisor-loss path, and exact result/resource assertions for the continuing peer.
- The native capture in `/tmp/metal-native-retry-20261108T221035Z` predates this driver change and does not verify the new survivor trigger. This focused approval is not a native rerun and does not complete the overall native gate.

### Verification disposition and verdict

This review inspected commit `b98f2d105`, the accepted DESIGN action table and model trace, the pre-authored controller-loss tests, and retained local capture files. No test, Metal/native command, SSH/remote command, source edit, or model edit was performed.

**APPROVED — focused deterministic survivor-boundary driver change.** The injected sequence uses an approved real owner fault path and maintains exact runner-result, surviving-owner, and complement assertions. The full native workflow remains **PENDING** until the corrected existing drivers run and produce their actual retained evidence; this approval does not claim the new native capture has passed.

## Iteration 4 — focused review of the RA expiry comparison projection

### Review metadata

- Scope: commit `2b6a55b47c7412c8aaaefa58a9dc43f2eae3fdf1`, limited to the existing physical-complement comparisons in `worktree_journey.sh` and `placement_journey.sh`, plus the evidence note.
- Retained attempt-5 raw evidence: evidence-only commit `8304fea4b75290c5f22922f2f839a4a0b9c79be6` under `deliver/native-evidence/ordinary-concurrency-attempt-2026-11-08-attempt-5/`.
- Review type: bounded comparison-projection review; no production implementation, API, test identity, model, or runtime behavior changed.
- Result: **APPROVED**.

### Projection and contract audit

The accepted invariant remains **zero test-owned physical-host route mutations**. The fix does not ignore routes or replace the physical snapshot with a smaller set. At both existing equality sites it deep-copies only `after['physical']`, then changes a copied `expires` value only when the paired before/after route entries both have `protocol == 'ra'`, both `expires` values are exact Python integers, and both are nonnegative. It then compares the complete original `before['physical']` against that in-memory copy. The raw `before.json` and `after.json` are never edited.

The filter leaves route list length/order, route presence, protocol, device, destination, gateway, metric, table, scope, flags, and every other route attribute under exact equality. Missing, noninteger, negative, or mismatched-protocol expiry values are not normalized. An added/removed/reordered route still differs under the full list comparison. Static and test-owned routes remain exact. All non-route physical fields—modules, nftables, BPF programs, test cgroups, and initial vsock modes—remain in the comparison unchanged. The same narrow rule appears in both existing driver assertions: `worktree_journey.sh:136-146` and `placement_journey.sh:120-134`.

### Attempt-5 raw evidence and attribution

The preserved [attempt-5 capture](native-evidence/ordinary-concurrency-attempt-2026-11-08-attempt-5/README.md) records the pre-fix driver exit 1 at the original raw `before['physical'] == after['physical']` assertion. Its retained `before.json` and `after.json` show two changed fields, both `protocol=ra` route `expires` countdowns:

| Route identity | Before | After |
|---|---:|---:|
| `eno12399np0`, `2001:bc8:1f90:1f::/64`, RA, metric 100 | 2,591,997 | 2,591,998 |
| `eno12399np0`, default via `fe80::2e4f:52ff:fefc:65d3`, RA, metric 100 | 12 | 13 |

The retained intermediate observations show those same countdowns moving up and down while route identities and other attributes stay the same. The capture did not include an RA packet or identify a route-refreshing process, so refresh attribution remains an inference. The evidence note cites the pinned Linux 7.0 IPv6 route dump and RA-route lifetime paths; this is consistent with the observed remaining-lifetime field, not direct packet evidence.

For attempt 5's `worktree_journey` run, the observed suite caller is confined to its invocation-local network namespace, and the captured live VMM arguments use vsock without a `--net` device. No route-write operation appears in that selected command/owner path into the physical initial namespace. The raw capture proves the exact-equality assertion failed; it does **not** prove a test-owned physical route mutation. The `placement_journey` comparator change is code-reviewed here but was not exercised by attempt 5. Conversely, the corrected projection has not yet been exercised by a native run, so K3 remains uncredited until the corrected full driver completes.

The commit's completeness note reports an offline replay accepting only the paired RA-expiry variations and 852 negative attribute comparisons rejecting other changes. This reviewer did not rerun that replay or the driver. The projection code itself retains raw snapshots, preserves the zero test-owned mutation invariant, and does not authorize any route operation or broader physical-kernel delta.

### Findings and remediation disposition

- No production or contract finding was accepted. The raw physical-equality failure is explained by the narrowly scoped, time-varying RA `expires` values; it is not evidence that the test mutated a host route.
- No route, route identity, or other physical snapshot key is filtered. The sole comparison normalization is paired nonnegative integer `expires` on `protocol=ra` entries.
- No source, production code, test identity, model, or DES event was changed by this reviewer. No native/Metal/SSH command or test was run.

### Verification disposition and verdict

**APPROVED — focused RA expiry projection fix.** This approval is for the comparison rule only. The preserved attempt-5 capture remains a failed raw equality run, and the revised projection has not been natively rerun. The full native acceptance gate remains **PENDING** until the corrected worktree driver and remaining native drivers produce the accepted evidence; no zero-host-mutation outcome is inferred from the existing failed capture.

## Iteration 5 — focused review of the prepared dispatcher path correction

### Review metadata

- Scope: commit `854a131a6a4fa4705536bb783b97f4472ac749ad`, limited to the ordinary bootstrap's dispatcher path and the retained admission-capture note.
- Evidence source: native pre-fix capture in `native-evidence/admission-attempt-2026-11-08-230613Z/`.
- Review type: bounded implementation-remediation review; no production API, acceptance body, fixture owner, or runtime behavior added.
- Result: **APPROVED**.

### Producer/consumer contract

The native failure is consistent with an exact existing producer/consumer path mismatch. `cargo xtask guest-kit-prepare` calls `cargo_target_dir(&root)` and builds/verifies `target/debug/overdrive-fwd-guest-suite` (`xtask/src/main.rs:504-506`; `xtask/src/guest_kit_prepare.rs:213-216,283-297`). The P4 context requires the prepared suite dispatcher under the bound checkout's target (`feature-delta.md:354-356`). Before this correction, `bootstrap.sh` injected `<remote-checkout>/target/guest-kit/overdrive-fwd-guest-suite`; commit `854a131a6` changes that one value to `<remote-checkout>/target/debug/overdrive-fwd-guest-suite`, matching the existing producer. The consumer's fail-closed canonical-path/file/executable checks remain unchanged (`test-wrapper.sh:212-224`).

The retained pre-fix ordinary invocation (`11eae164e76918cb07451a86`) reached START and Nextest collection, then reported `OVERDRIVE_METAL_TEST_ERROR error=lane_not_prepared`; its Nextest run reported zero passed, one failed, two skipped, and the outer command retained exit 100. No `METAL-ISOLATION-TIMEOUT body_started=true` marker appears, and the body did not reach its guest-boot-ID assertion. This is a preparation refusal, not evidence of a maintenance-lock, systemd-stop, or guest-kernel defect. The capture lacks `guest-kit-prepare` output and an artifact listing, so it cannot establish whether preparation was omitted in that attempt. Independently of that ambiguity, the source paths prove that successful preparation at the producer's path would not satisfy the old injected consumer path.

The correction changes only the internal path value. `vm_fixture::workspace_root()` uses `OVERDRIVE_METAL_SOURCE_ROOT` only when `OVERDRIVE_METAL_EXECUTION_CLASS` is exactly `ordinary`; the ordinary wrapper has already validated that canonical source owner before dispatch (`test-wrapper.sh:33-55`). Non-Metal and explicit `exclusive` contexts continue through the previous `cargo locate-project --workspace` path and compile-time fallback. An ordinary invocation missing required context remains refused by the wrapper before the dispatcher reaches this helper.

The remediation does not move or duplicate the preparation owner, add an on-demand dispatcher build, add an alternate host execution fallback, change the selected test identity or assertion, or alter the typed `lane_not_prepared` refusal for a genuinely absent/invalid executable. The pre-authored absent-dispatcher refusal body remains unchanged; no public API or environment name is introduced.

### Verification disposition and remediation

The evidence note reports the existing artifact-path unit test passing in Lima, the existing fail-closed missing-dispatcher acceptance body passing in Lima, and `bash -n` passing. This reviewer did not rerun those checks. The corrected dispatcher path has **not** been validated by a native admission or placement rerun; the captured command predates the fix and remains the reproduced failure evidence. The user-facing driver must still run with the approved preparation command completed for the same bound checkout before native admission results can be credited.

No code finding or further remediation is required for this one-line path alignment. The overall 01-02 implementation verdict remains technically **APPROVED**, and full native acceptance remains **PENDING** until the corrected workflow succeeds. This review claims no native pass for the dispatcher correction, admission driver, or full feature.

## Iteration 6 — focused review of owned-console timeout observation

### Review metadata

- Scope: commits `8037c3789891115ead21205970447bf250d83927` and `401d902ed8af5ee2f883b2e39c6e3d8ba2523f02`, limited to the existing external admission/placement timeout drivers and read-only host observer.
- Prior native capture: `admission-rerun-2026-11-09-000016Z`, run `10d61cd38baa759626482c31`.
- Review type: bounded test-observation review; no product API, protocol, production code, test identity, or model changed.
- Result: **APPROVED**.

### Owner and VMM-console provenance

The pre-fix capture shows the ordinary run reached START, Nextest timed the selected timeout test out at 10.002 seconds, and the outer command retained aggregate status 100. Its `ordinary.stdout` lacks the body-start marker, but the capture contains no owner snapshot. It therefore cannot establish whether the test body failed to start or started and wrote only to the guest console; the author correctly leaves that distinction unresolved.

The new marker source is the existing selected fixture body, not driver bookkeeping. `selected_attempt_waits_for_its_timeout` first checks that the guest boot ID differs from the captured physical-host boot ID, then prints and flushes the exact `METAL-ISOLATION-TIMEOUT body_started=true` line before its 180-second wait (`attempt.rs:40-49`). Nextest launches the selected ELF in the guest through the existing dispatcher/peer path. `GuestPeer::exec` consumes the typed `PeerReply::Exit` only (`guest_peer.rs:1289-1300`), and the guest peer runs the child with `Command::status()` and inherited stdio (`bin/guest_peer.rs:1774-1788`). On the host, Cloud Hypervisor's `--serial file=...` points to the run-owned console path (`overdrive-host/src/vmm.rs:295-297`). Thus the outer controller stream is not the source of truth for this body marker; the VMM serial file is.

`observe_host.py:52-80,83-149` associates the console with the requested run ID's actual owner process observations. It accepts only a Cloud Hypervisor argv with one file-backed `--serial` path lexically below `/run/ovd-gk/<run-id>`. It walks from `/` using directory descriptors and `O_NOFOLLOW`, rejects `..`, opens the final file with `O_NOFOLLOW|O_NONBLOCK`, and verifies it is a regular file before reading raw bytes into base64. Read/path errors are retained in `guest_console_error`; the driver retains each owner snapshot and query stderr. A missing or unreadable console cannot satisfy the exact-line predicate. The driver additionally requires the same owner to have an active unit and `phase=running`; guest boot ID is guarded by the selected body itself. No marker is synthesized by the observer or copied from local driver output.

### Assertion preservation and fault-path limits

The admission driver now waits for the exact body-start line from that live owner console before continuing to its existing exclusive-maintenance and waiter checks. Its previous `ordinary.stdout` marker check was not replaced with a boot/VMM-liveness proxy. The placement driver uses the same body and console observation while the timeout run is live, then joins the existing `run_case` status-100 result, checks the Nextest TIMEOUT output, collects the run, and verifies finalizer completion and root/process cleanup. Its existing retry, guest-reset, command-status, exclusive/non-Metal, and physical-kernel assertions remain. The background timeout subshell clears only its inherited `EXIT` trap so the parent retains the existing foreign-fixture cleanup owner.

The capture at `000016Z` remains a pre-fix failure record: the original outer-stream body marker is absent and no owner snapshot exists, so it is not evidence that the selected body failed to start. The new observer has not yet been exercised by a native admission or placement run. This focused review therefore approves the observation path, not timeout-path native proof, exclusive/waiter behavior, placement completion, or the full K1–K3 result.

### Findings and remediation disposition

- No production finding or contract change was accepted. The `GuestPeer` status-only response and `GuestRun::finish`-after-return lifecycle explain the old observability gap; the approved host observer now reads the existing run-owned VMM console without changing production output or protocol.
- No new API, test identity, model action, timeout, or production seam was added. No assertion was removed or replaced with a weaker body-start proxy.
- The body-start query is bounded by the existing 1,200-iteration driver loop; failing path/console observations remain in raw per-attempt files and cannot produce an `ordinary-running.json` or `timeout.body-start.json` success snapshot.
- The author reports offline marker/path checks, shell syntax, and focused Lima results; this reviewer did not rerun tests or checks.

### Verification disposition and verdict

**APPROVED — focused owned-console timeout-observation change.** It observes the actual selected body through the same run-owned VMM serial console that carries the guest's inherited stdout, with the owner still active. The correction does not convert the prior timeout into a pass or claim that the body-start marker was observed natively. Full native admission and placement validation, and the overall 01-02 gate, remain **PENDING** until the prescribed drivers run after this fix and produce their actual evidence.

## Iteration 7 — focused review of static init preparation before Nextest

### Review metadata

- Scope: commit `500126cddbd759b46f1f47201bf18e6b4d8bad5b`, limited to the additional `overdrive-init` build and required-path check in `xtask/src/guest_kit_prepare.rs`, plus its retained admission evidence note.
- Triggering evidence: admission run `97fde96f2704e50754be8acd`, including the later pre-fix timeout capture in `native-evidence/admission-rerun-2026-11-09-003244Z/` and the earlier real Cargo workspace failure in `native-evidence/admission-rerun-2026-11-08-233719Z/`.
- Review basis: exact accepted P4 in `feature-delta.md:354-356`, the accepted ADR-0280, production callers, the commit diff, retained local captures, and the same target-directory resolver used by both owners.
- Result: **APPROVED — focused preparation remediation**. The full native acceptance gate remains **PENDING**.

### Reachable failure and owner path

The retained `233719Z` run establishes the earlier failure after dispatch: the selected Nextest case started, the existing `GuestKit::provision` path called `vm_fixture::build_overdrive_init_static`, and Cargo failed to find `overdrive-init` because the selected manifest belonged to the standalone attempt-fixture workspace. That run did not reach the body and cannot prove native admission behavior.

After the ordinary-context source-root correction, the `003244Z` owner snapshots show the same real path reaching the correct workspace but still compiling under the selected case's 10-second timeout. Snapshot `ordinary-start-28.json` records the dispatcher and `cargo build --package overdrive-init --target x86_64-unknown-linux-musl --release`; subsequent snapshots contain rustc children before any Cloud Hypervisor process or guest console exists. A bounded read-only decode of `ordinary-start-34.json` shows rustc's `overdrive_core` child writing to the bound checkout's `target/x86_64-unknown-linux-musl/release/deps`. The capture ends before a rustc invocation for the `overdrive-init` crate or a VMM appears. This is evidence of the nested build's source-workspace target and its pre-body timing; it is not evidence that the selected body started or that the build completed.

The production caller remains `fwd_guest_suite` → `GuestKit::provision` → `vm_fixture::build_overdrive_init_static` (`guest_kit.rs:452-469`; `vm_fixture.rs:1051-1075`). In ordinary context, `workspace_root()` selects the already validated `OVERDRIVE_METAL_SOURCE_ROOT`; outside that context it preserves its existing Cargo workspace discovery (`vm_fixture.rs:1078-1108`). The fixture's standalone `--target-dir target/metal-run-isolation-fixture` belongs to the outer Nextest test executable; it is absent from the captured inner Cargo argv.

### Contract and implementation mapping

| Accepted contract | Implementation in `500126c` | Review disposition |
|---|---|---|
| P4 uses declared `guest-kit-prepare` setup to have required guest-context artifacts ready before the selected attempt; it forbids replacing the prepared dispatcher with an on-demand wrapper build. | The existing setup owner builds the static init with `cargo build --package overdrive-init --target <existing musl triple> --release` and requires `<target_dir>/<triple>/release/overdrive-init` before it reports the lane prepared (`guest_kit_prepare.rs:274-284,309`). No wrapper build or selected-test timeout change is added. | Matches the reproduced pre-body compile delay and uses the approved setup entrypoint. |
| The existing fixture provisioning call and its failure meaning remain in place. | `GuestKit::provision` still invokes `build_overdrive_init_static`; that function retains the same Cargo command, path derivation, and typed `OverdriveInitBuildFailed` mapping (`guest_kit.rs:466-469`; `vm_fixture.rs:1051-1075`). With the artifact current, Cargo's existing incremental build verifies it; if the artifact is absent or stale, the fixture's existing build/error path remains available. | P4's existing fixture-internal provisioning remains owned by `GuestKit::provision`: it still obtains the init path through the same runtime function before image/VMM setup. The new setup action pre-materializes that exact build artifact; it does not move image construction, staging, or runtime failure handling into xtask. |
| The artifact must be scoped to the bound checkout and the same Cargo target directory as its consumer. | `xtask::main` passes `cargo_target_dir(&root)` to `guest_kit_prepare::run` (`main.rs:504-506,1391-1394`). The runtime consumer uses the same `CARGO_TARGET_DIR`-or-`<workspace>/target` rule (`vm_fixture.rs:1133-1134`), the same source root in ordinary context (`vm_fixture.rs:1086-1108`), the same host musl triple, package, target, and release profile. The retained capture confirms the nested Cargo target was under the bound checkout, not the fixture's test target directory. | Exact producer/consumer artifact path and build options agree. No arbitrary target default, alternate feature set, artifact copy, or fallback path was introduced. |
| Public API, CLI, wire format, test identity/body, and error taxonomy remain exact. | The commit adds no public type, function, parameter, environment name, CLI option, fixture-body edit, or new runtime error mapping. The setup command uses its existing `run_step` failure path; the runtime fixture still maps its own build failure to `OverdriveInitBuildFailed`. | No interface or typed-error contract divergence found. |

`praise:` The change addresses the observed in-timeout Cargo work at the declared preparation boundary and keeps the runtime consumer's package, target, profile, path, and typed failure behavior aligned with that prebuild.

### Verification limits

The committed evidence note reports that canonical Lima preparation completed with target `aarch64-unknown-linux-musl`, including the new required path, and reports the existing artifact-path unit test, xtask all-target check, Clippy, and scoped rustfmt as passed. Those are author-reported results; this reviewer did not rerun tests or checks. No native preparation, admission, or placement driver ran after `500126c`. The prior native capture remains a pre-fix failure: it proves the timeout can be consumed by setup before VMM/body start, and provides no admission, waiter, placement, or zero-host-mutation credit.

### Findings and remediation disposition

- No production finding or design-contract contradiction was accepted. The reproduced timing failure is reachable through the existing Nextest dispatcher and GuestKit provisioning owner; the remedy adds the exact existing static artifact build to the already declared setup step while preserving the fixture's runtime call and typed failure path.
- No additional mechanism, API, test identity, body assertion, timeout, or failure representation is required for this bounded correction.
- No source, test, model, or DES event was changed by this reviewer. No test, native, Metal, SSH, or remote command was run.

### Verdict

**APPROVED — focused remediation in commit `500126cddbd759b46f1f47201bf18e6b4d8bad5b`.** The implementation uses the exact approved artifact owner path, command, target, and profile and prepares it before Nextest's selected-attempt timeout. This approval covers the source-level remediation and reported Lima checks only. Native admission and placement, and the full 01-02 acceptance gate, remain **PENDING** until the prescribed post-fix native evidence is retained; no native result or zero-host-mutation outcome is inferred here.

## Iteration 8 — focused review of selected guest working-directory materialization

### Review metadata

- Scope: commit `349e7cc2fd9fc2af4bbde08f2397ab96b801cf13`, limited to staging `/usr/bin/mkdir` and creating the selected Nextest working directory inside a guest-group attempt.
- Triggering evidence: current pre-fix native admission capture `admission-rerun-2026-11-09-012325Z-root-final/`, run `e6f13d369350e848d51c0192`, source commit `500126cddbd759b46f1f47201bf18e6b4d8bad5b`.
- Review basis: accepted P4 in `feature-delta.md:350-356`, the raw local capture and owner snapshots, the commit diff, the existing GuestFile/tool-copy path, and the existing GuestRun/GuestKit cleanup and failure mapping.
- Result: **APPROVED — focused working-directory remediation**. The full native acceptance gate remains **PENDING**.

### Reproduced failure and exact source provenance

The retained native controller output shows the selected timeout case reached Nextest `START`, booted its ordinary `core` guest, and failed in guest `/usr/bin/env --chdir` with `No such file or directory` for the exact standalone fixture CWD under the bound checkout. Nextest recorded `FAIL [1.830s]`, zero passed; the durable ordinary command recorded aggregate exit 100 with `cause=command_returned`. The selected body did not execute its boot-ID assertion or emit `METAL-ISOLATION-TIMEOUT body_started=true`, so this capture is neither a timeout-body result nor an admission pass.

The raw run record is consistent with its README attribution: the `ordinary-start-17.json` metadata identifies run `e6f13d369350e848d51c0192`, source commit `500126cddbd759b46f1f47201bf18e6b4d8bad5b`, and the recorded source digest. `ordinary-start-32.json` records the same live owner with a Cloud Hypervisor process and the guest's owned console path; later samples show the command terminal and no owner processes or the three owned roots. The exclusive-maintenance and waiter portions were not reached. The earlier `003244Z` static-init-preparation failure is not used as evidence for this CWD issue.

The production path is bounded and specific: `nextest_attempt_files` reads the wrapper's actual current directory and returns it with the selected ELF/helper files (`fwd_guest_suite.rs:3090-3126`); `run_nextest_group_attempt` carries that exact `PathBuf` through guest planning and boot (`:3595-3636`); and `exec_nextest_attempt` already passes the same path as the operand to `/usr/bin/env --chdir` (`:1419-1447`). The captured missing-directory error reproduces the failed path through that path. The new `mkdir` materialization occurs in the same disposable guest before that existing invocation.

### Contract and implementation mapping

| Accepted contract | Implementation in `349e7cc` | Review disposition |
|---|---|---|
| P4 keeps Nextest's selected body, argv, environment, and placement owner; guest contexts execute in their selected context while the dispatcher uses the already prepared guest path. | The code creates the directory represented by the existing `current_dir`, then passes the unchanged `&current_dir` to `execute_nextest_guest_attempt`; `exec_nextest_attempt` still supplies it unchanged to `/usr/bin/env --chdir` (`fwd_guest_suite.rs:3637-3652,1419-1447`). | It materializes the prerequisite directory for the already accepted working-directory contract; it does not substitute a fallback CWD or alter the selected body, arguments, environment, timeout, or result. |
| Host checkout files and physical-host test-kernel state remain outside the disposable guest execution boundary. | `tool("mkdir")` is added to the existing guest attempt file closure (`:3468-3477`); the guest image builder copies that utility and its declared shared-library closure into the invocation-owned staging root (`guest_kit.rs:545-550`). The selected guest runs `/usr/bin/mkdir -p -- <exact CWD>` through `GuestRun::command` (`fwd_guest_suite.rs:1627-1635,3640-3643`). No host checkout tree or host test working-directory tree is copied. | The directory is created in the per-attempt guest filesystem; its backing rootfs/work directories are removed with the existing attempt owner. The current directory path is preserved, while host files at that path are not imported. |
| Missing guest inputs and guest setup failures fail closed and retain the existing typed dispatcher result. | The `mkdir` command uses the same `GuestRun::command` path as other guest preparation commands. That path returns an error for nonzero status; the new call maps it to `NextestDispatchFailure::GuestExecution`, and the existing reporter emits `guest_execution_failed` with exit 70 (`fwd_guest_suite.rs:1627-1635,3640-3643,3670-3682`). | The command failure is not ignored and the selected ELF is not started after a failed mkdir. No new error variant or generic-string shortcut is introduced. |
| Guest resources remain owned and cleaned by the existing per-attempt lifecycle. | The failure propagates before `execute_nextest_guest_attempt`; dropping `GuestRun` drops its `ScriptedGuest`/`GuestKitVm` and `WorkDir`. `GuestKitVm::drop` invokes its existing VMM terminate path (`guest_kit.rs:756-766,894-901`), and `WorkDir::drop` removes its per-attempt root (`guest_kit.rs:1203-1238`). | The new branch retains the existing owner and cleanup behavior used by prior pre-execution setup failures. It introduces no persistent host or guest directory outside that disposable attempt. |
| Non-Metal and explicit-exclusive execution preserve their original exec chain. | The commit changes only `fwd_guest_suite.rs`'s guest attempt file closure and guest-group execution path; the ordinary/exclusive adapter and its passthrough branches are untouched. The evidence note reports the existing passthrough acceptance case passing in Lima. | No passthrough or CLI behavior change found. |

`praise:` The fix stages the small existing guest utility through the established GuestFile dependency-closure mechanism and applies it only after the selected disposable guest is booted, keeping the accepted `--chdir` value exact.

### Verification limits

The committed evidence note reports canonical Lima all-target check and Clippy passing, 24 testing-library tests passing, the existing missing-dispatcher fail-closed acceptance body passing, the existing non-Metal/explicit-exclusive passthrough acceptance body passing, and scoped rustfmt passing. These are author-reported results; this reviewer did not rerun tests or checks. There is no native preparation, admission, or placement run after `349e7cc`. The retained native failure predates the change and proves only the CWD precondition failure. It supplies no post-fix selected-body, timeout, exclusive/waiter, placement, or zero-host-mutation credit.

### Findings and remediation disposition

- No production or accepted-contract finding was accepted. The concrete guest `env --chdir` failure is reachable through the current ordinary dispatcher; the remediation creates precisely its absent guest-side directory before the existing invocation.
- The creation command uses a single argv element for the exact path after `--`; failure remains a guest-execution failure and stops dispatch before the selected ELF. The existing guest utility copy includes its shared-library closure, and existing GuestRun owners terminate the VMM and remove the attempt work directory on this pre-execution error path.
- No source, test, model, API, acceptance body, or DES event was changed by this reviewer. No test, native, Metal, SSH, or remote command was run.

### Verdict

**APPROVED — focused remediation in commit `349e7cc2fd9fc2af4bbde08f2397ab96b801cf13`.** It preserves and materializes the exact Nextest working-directory path within the disposable guest using the existing tool and resource owners. This source review does not establish a post-fix native result. Native admission and placement, and the full 01-02 acceptance gate, remain **PENDING** until the prescribed post-fix native evidence is retained; no host-mutation or full journey outcome is inferred here.

## Iteration 9 — focused review of exclusive-admission and waiter ordering

### Review metadata

- Scope: commit `446db729139c16257cffd8207f93dadb34b9fbe6`, limited to the ordering and owner assertions in `infra/metal/tests/admission_journey.sh` and its DISTILL completeness note.
- Prior raw capture: `native-evidence/admission-rerun-2026-11-09-020736Z/`, run IDs `b162f48b28d9859edf1403a8` (ordinary timeout owner), `5496e8a54617b3e3c113da61` (exclusive maintenance), and `af4b745513beabed81144db5` (ordinary waiter).
- Review basis: accepted P2 at `feature-delta.md:284-296`, approved E4 at `feature-delta.md:564`, the exact production waiter loop/call path in `lease-holder.sh`, the commit diff, and the preserved pre-fix native capture.
- Result: **NEEDS_REVISION** for one bounded acceptance-coverage gap. This is not a production implementation defect finding. Native admission/placement remains **PENDING**.

### Contract and live owner ordering

P2 requires exclusive maintenance to wait for already-admitted ordinary units through their actual systemd stop/cleanup, while a registered ordinary unit that has not acquired the shared barrier cannot block that exclusive owner (`feature-delta.md:296`). The approved E4 consequence explicitly says to contrast “waiting unadmitted runs and admitted units still stopping” and requires EX maintenance to start only after admitted units finish stop/cleanup (`feature-delta.md:564`). This review does not infer a FIFO or priority rule for an ordinary request that reaches the flock before the EX owner acquires it.

The production ordering makes the relevant distinction observable. The exclusive service acquires the canonical EX flock at `lease-holder.sh:869-889`, then calls `wait_for_admitted_ordinary_units` at `:891-907`. That loop scans live ordinary records in `preparing|running`, and explicitly skips `waiting_for_lease` (`:414-455`, especially `:438-445`). The owner does not write phase `preparing` until after this loop (`:997`). The meaningful contrast is therefore an ordinary waiter in `waiting_for_lease` with no shared descriptor while the EX owner has its real FLOCK WRITE descriptor and the earlier admitted ordinary owner is still live in the stop/cleanup path.

### Blocking finding — the waiter starts after the required liveness decision

`issue (blocking):` The new sequence waits for `maintenance-admitted.json` before it launches the ordinary waiter. That snapshot requires the predecessor's `completion.v1` to be complete, no predecessor processes, and all three predecessor roots absent (`admission_journey.sh:76-94`). Since the production `wait_for_admitted_ordinary_units` call has already returned before the EX owner reaches the `preparing`/`running` phase accepted by this gate, the subsequent waiter at `:98` is created too late to exercise the P2 rule that an unadmitted waiting ordinary run does not block the EX owner's stop/cleanup wait. The new simultaneous snapshot proves the narrower fact that a waiter remains unadmitted while an already-admitted EX owner holds the barrier after predecessor cleanup. It does not prove the accepted E4 contrast between that waiter and an admitted unit still stopping.

This is a coverage finding, not a claim that the current production loop is wrong: the current loop visibly skips `waiting_for_lease` records. A regression that included live waiting records in that loop could still pass this revised journey because the journey does not register its waiter until after that loop has completed.

**Bounded remediation:** move registration of this same waiter to after the observer has established the EX owner's real FLOCK WRITE descriptor but before the admitted predecessor is observed complete/stopped. Retain a simultaneous owner snapshot proving the earlier admitted ordinary owner is still live/stopping, the EX owner holds the canonical EX descriptor, and the new ordinary owner is `waiting_for_lease` with no shared flock descriptor. Then retain the existing final evidence that the predecessor's stop/cleanup completes before the maintenance body starts, and preserve the current exact 100/0/0 command results, maintenance marker, waiter completion, and three-root/process cleanup assertions. This uses the accepted real owner path and does not require a new priority policy, forced state, sleep, timeout, production seam, or test identity.

### Prior evidence and verification limits

The preserved `020736Z` capture confirms the selected guest body-start and EX `waiting_for_lease` while the ordinary timeout owner was live. Its first valid waiter observation already has maintenance inactive and the waiter only preparing; buffered `maintenance.stdout` was collected after the maintenance command completed. That is the reproduced harness observation-order failure, not a production admission failure and not post-`446db` evidence.

This reviewer made no test, native, Metal, SSH, remote, source, or model change and ran no tests/checks. The revised driver has not been natively run. The focused result is **NEEDS_REVISION** until the same waiter is observed during the real EX owner's admitted-unit stop/cleanup wait; after remediation, an independent re-review is required before the root runs the prescribed native driver.

### Verdict

**NEEDS_REVISION — commit `446db729139c16257cffd8207f93dadb34b9fbe6` does not yet exercise the accepted P2/E4 unadmitted-waiter-versus-admitted-stop sequence.** It repairs the stale buffered-output observation and proves an unadmitted waiter alongside an already-admitted EX owner, but registers that waiter only after the predecessor has completed the exact stop/cleanup wait the contract requires it to contrast. No production defect is alleged. Native admission and placement and the full 01-02 acceptance gate remain **PENDING**.

## Iteration 10 — re-review of EX/waiter sequencing remediation

### Review metadata

- Scope: commit `c5df6e910629acfdb55ba7b42289ff711e117100`, limited to remediation of Iteration 9's admission-driver ordering finding and its DISTILL completeness note.
- Finding addressed: register the existing ordinary waiter while the EX owner holds its actual barrier descriptor and the earlier admitted ordinary owner is still in systemd stop/cleanup.
- Review basis: accepted P2 at `feature-delta.md:284-296`, approved E4 at `feature-delta.md:564`, current `lease-holder.sh` owner ordering, the exact commit diff, and prior `NEEDS_REVISION` Iteration 9.
- Result: **APPROVED — Iteration 9 finding closed**. The post-fix native admission and placement evidence remains **PENDING**.

### Required three-owner witness

The revised driver now requires the actual predecessor-stop state before starting the waiter. Its `stopping` snapshot requires the admitted ordinary predecessor to remain `phase=running`, unit `ActiveState=deactivating`, and carry its actual command terminal 100; at the same time, the exclusive owner must be active in `waiting_for_lease` and its real MainPID must hold the canonical maintenance `FLOCK WRITE` descriptor (`admission_journey.sh:76-95`). This is the production interval after EX lock acquisition and while `wait_for_admitted_ordinary_units` is still deciding whether the admitted owner has stopped.

The same existing waiter command starts only after that witness. The driver then requires one `observe` result containing all three owner IDs: the prior admitted owner still deactivating with terminal 100, the EX owner still waiting with the FLOCK WRITE descriptor, and the newly registered ordinary owner active in `waiting_for_lease` with no acquired shared FLOCK descriptor (`:96-121`). This is the exact P2/E4 contrast: an admitted predecessor still requires stop/cleanup, while the unadmitted waiter has not acquired admission and must not join that wait set.

The follow-up `progress` witness requires the prior owner to be inactive with complete receipt, no processes and all three roots absent, while the same EX owner has advanced to `preparing` or `running` and still owns the EX descriptor; the same waiter remains active in `waiting_for_lease` with no shared descriptor (`:122-143`). This proves the EX owner passed the admitted-unit stop/cleanup wait while the unadmitted waiter remained queued. The maintenance body still independently checks predecessor stop and root absence before its existing marker and three-second hold. The driver retains its original selected body, waiter command, exact 100/0/0 statuses, output assertions, and final process/root cleanup checks (`:48-58,144-159`).

The ordering matches the current production owner path without imposing a lock-priority rule: the EX service acquires the barrier, then runs `wait_for_admitted_ordinary_units`, which includes live ordinary `preparing|running` records and skips `waiting_for_lease` (`lease-holder.sh:414-455,869-907`). The EX phase remains `waiting_for_lease` until after that loop; the revised snapshots rely on actual owner metadata and `/proc` descriptor observations, not driver-local timing records or a synthesized admitted state. No production protocol, model state, owner, command, timeout, delay, or fixture control is added or changed.

`praise:` The remediation closes the semantic gap by placing the same waiter inside the already-existing EX stop/cleanup wait and requiring the actual three-owner descriptor/phase conjunction before accepting the journey.

### Findings and verification limits

- Iteration 9's blocking finding is **closed**. The earlier `maintenance-admitted.json` gate no longer precedes waiter registration; the test now proves both the simultaneous stopping/unadmitted state and subsequent EX progress while the waiter remains unadmitted.
- No production defect is claimed. The review confirms test coverage of the approved existing behavior, without adding FIFO/priority semantics or a new protocol state.
- The commit's completeness note reports offline rejection of the old late-waiter trace and shell syntax/whitespace checks. This reviewer did not rerun checks. No native run exists after `c5df6e9`; the preserved `020736Z` capture remains pre-fix evidence only.
- No test, native, Metal, SSH, remote, source, production, or model change was made by this reviewer.

### Verdict

**APPROVED — focused remediation in commit `c5df6e910629acfdb55ba7b42289ff711e117100`.** It now exercises the approved P2/E4 stop-versus-unadmitted-waiter sequence through the actual owner phases and flock descriptors, then confirms EX progress while the waiter remains unadmitted. This approves the harness ordering only; post-fix native admission and placement, and the full 01-02 acceptance gate, remain **PENDING** until the prescribed native evidence is retained.

## Iteration 11 — focused review of the authorized NativeUnit E4 fixture

### Review metadata

- Scope: commit `be7d50659adc8b4c6808b782c2207d87647cc002`, limited to the external E4 prior-owner fixture, shared `NativeUnit` test input primitive, read-only multi-owner bracket, and truthful coverage notes.
- Triggering evidence: preserved native attempt `admission-rerun-2026-11-09-024233Z/`, source commit `c5df6e910629acfdb55ba7b42289ff711e117100`, which missed the brief deactivation interval between an ordinary unit active sample and its fully stopped sample.
- Review basis: accepted P2 at `feature-delta.md:284-296`, E4 at `:564`, the test-support boundary authorized for this remediation, `lease-holder.sh` owner ordering, the current `NativeUnit` implementation, external driver/fixture code, and the updated evidence-boundary table.
- Result: **APPROVED — focused test-support remediation**. Native execution and full 01-02 acceptance remain **PENDING**.

### Boundary and evidence attribution

The new prior is explicitly an owner-level controlled `NativeUnit` fixture. It is not credited as the unchanged public default-barrier guest-timeout command: the driver runs that public guest body first, requires its real guest-console marker and aggregate 100, waits for its completed owner, and records it separately (`admission_journey.sh:23-57`). The E4 fixture then invokes the captured production `lease-holder.sh --durable` under a real `systemd-run` owner; public `cargo xtask metal run --exclusive` and ordinary waiter bootstraps still drive the actual bootstrap/controller paths. The fixture-private barrier, owner, and tree-lock path substitutions are explicitly test-support inputs permitted by P2 (`feature-delta.md:288`), and the coverage tables now limit the fixture's credit to owner-level E4 evidence (`feature-delta.md:663-684,831-832`). They do not weaken the P2/E4 outcome, change the public default barrier, or claim that this fixture alone proves canonical default-path E4. The separate default-path overlap/stop and blocked-maintenance captures retain their own stated limits.

### Owner, constructor, and descriptor mapping

The controlled prior uses a fresh private actor directory and derives its run ID from that directory (`native-unit-controller.py:22-40`). It reads the reference public run's real v2 metadata to obtain the bound remote source/workspace and copies `lease-holder.sh` from that source; the helper's actual systemd unit creates the run records and lock descriptors. The 18 durable positional values line up with the approved order and live parser: fixture maintenance lock, maintenance owner, existing 120-second budget, valid 24-hex token, scenario, workspace, commit, source digest, remote source, root target user, privileged mode, remote-home value, PATH, selected command, runs root, fixture tree lock, tree owner, and `ordinary` execution class (`native-unit-controller.py:56-61`; `lease-holder.sh:740-762`). No public durable argument or production interface changes.

The prior and the existing Rust `NativeUnit` share one private `native-unit` primitive for the selected command's `command.exit` input and its `ExecStopPost` callback's `stop` input, including the existing 20-second callback bound and driver-liveness fallback (`fixtures/native-unit:7-19`; `metal_run_isolation_conformance.rs:191-215`). The fixture uses actual Type=exec/KillMode=mixed systemd ownership with the existing 60-second runtime and 45-second stop bounds; the callback execs the captured production `--finalize` with the actual run ID and runs root. The controlled command's exit 0 is test input for the predecessor owner lifecycle, not a substitute for or relabeling of the separate public guest timeout result 100.

All three E4 participants use the same actor-owned `maintenance.lock`, owner path, tree lock and tree-owner path: the controlled durable invocation receives them positionally; the public EX and ordinary waiter bootstrap invocations receive the same values through the existing fixture-only `OVERDRIVE_METAL_*_PATH` substitutions (`admission_journey.sh:37-40,116,158`). The default public timeout owner has already completed before this private E4 barrier is created, so there is no outer default-barrier holder that self-blocks this fixture. The waiter's own recorded run ID remains independent.

The actual accepted witness is preserved from Iteration 10 and now uses a deterministic owner input rather than sampling for a sub-1.5-second external deactivation window. It requires the controlled admitted ordinary owner to be `running` with command terminal 0 and systemd `ActiveState=deactivating`/`MainPID=0`, while the EX owner is still `waiting_for_lease` and its real MainPID holds `FLOCK WRITE` on that same fixture barrier (`admission_journey.sh:133-155`). The already-existing waiter is then registered; one `observe` result must show all three owners in that state, with the waiter `waiting_for_lease` and no FLOCK descriptor, and the second unit/phase read must bracket that same state (`:156-184`; `observe_host.py:185-194`). Only after retaining that simultaneous witness does the driver release the existing `stop` input. It then requires the prior completion receipt, no processes/roots, EX progress to `preparing|running`, and the same waiter still unadmitted (`:185-210`). The public maintenance body independently verifies predecessor stop and root absence before its marker; final status, collector result, terminal outcomes, completion receipts and all three roots are checked (`:211-238`).

The observer reads actual systemd fields, durable records, `/proc` processes and fdinfo. It brackets the multi-owner read with fresh `unit_state` and phase queries and remote monotonic timestamps; query errors propagate to the captured query stderr. The fixture's `fixture_input` JSON contains only constructor paths/run ID and is validated before use (`admission_journey.sh:61-89`); it is never projected as an observed owner state. The actual EX WRITE, controlled prior READ, and waiter no-FLOCK assertions all compare the same descriptor target. No owner record, phase, terminal, completion receipt, lock result or cleanup outcome is synthesized.

### Contract mapping, scope, and verification limits

P2 remains the authority: the real exclusive service holds the barrier while `wait_for_admitted_ordinary_units` waits for live ordinary `preparing|running` units and skips `waiting_for_lease` (`lease-holder.sh:414-455,869-907`). The fixture tests that actual owner loop and kernel descriptor behavior using the authorized test-support barrier input. It does not assert FIFO/priority before EX acquires its lock; the pinned-kernel finding that an earlier SH may win over queued EX is accurately documented as the reason not to demand that unsupported premise. No DES/architecture protocol, priority rule, production API/hook, model action, test identity, timeout, maintenance sleep, or outcome has been added or weakened. The `feature-delta.md` edits update only DISTILL coverage/evidence attribution; the normative P2/E4 contract is unchanged.

The commit's authoring note reports local formatting/parsing and offline primitive/input/observation checks. This reviewer did not rerun them. The orchestrator reports that `cargo xtask lima run -- cargo check -p xtask --all-targets --features integration-tests` passed (exit 0, 0.64s); this is recorded as orchestrator-run compilation evidence, not as a reviewer test run. There is no post-`be7d506` Linux systemd fixture execution, native E4 run, or final admission-driver result. The prior `024233Z` capture remains evidence only of the observation gap; it is not a production defect or a post-fix success.

### Findings and remediation disposition

- Iteration 9's blocking sequencing gap is **closed** by the controlled prior callback fixture. It places the same public EX/waiter requests around a real production durable owner in the actual deactivation interval, then releases only the existing test input and observes production finalization/EX progress.
- No production contract gap or owner-path defect was found. This review credits the controlled private-barrier run only for owner-level E4 protocol behavior and preserves the separate evidence boundaries for the public default-barrier timeout and other native captures.
- No source, test body, model, production file, or DES phase event was changed by this reviewer. No test, native, Metal, SSH, or remote command was run.

### Verdict

**APPROVED — focused fixture refinement in commit `be7d50659adc8b4c6808b782c2207d87647cc002`.** The shared NativeUnit inputs, exact durable-owner arguments, actual systemd/finalizer lifecycle, one fixture-private barrier across the three E4 participants, bracketed owner observations, and evidence labels preserve the accepted P2/E4 semantics without changing production interfaces or outcomes. This approves the test-support authoring only. Native execution and the full 01-02 acceptance gate remain **PENDING** until root runs the post-fix native drivers and retains their actual results.

## Iteration 12 — focused re-review of the E4 fixture boundary and owner wiring

### Review metadata

- Scope: commit `be7d50659adc8b4c6808b782c2207d87647cc002` only; test-support fixture wiring, `NativeUnit` reuse, read-only owner observation, evidence attribution, and the changed DISTILL coverage tables.
- Triggering evidence: preserved native admission attempt `admission-rerun-2026-11-09-024233Z/`, which missed the sub-1.5-second deactivation interval during read-only polling and did not observe a waiter.
- Review basis: accepted P2/E4 at `feature-delta.md:284-296,564`, the authorized DISTILL-owned private fixture refinement, current `lease-holder.sh` and `NativeUnit` paths, `admission_journey.sh`, the shared fixture/controller, `observe_host.py`, and the commit's evidence-limit documentation.
- Result: **APPROVED — focused test-support refinement**. No post-fix native result is claimed.

### Test-support boundary and public-path attribution

The fixture preserves two different evidence boundaries. The driver first runs the unchanged public default-barrier Nextest guest timeout and requires the exact owned-console body marker, Nextest timeout result, and aggregate 100 (`admission_journey.sh:23-57`). E4 then uses a controlled `NativeUnit` prior with exit 0; it is explicitly labeled `controlled-owner-fixture` in the final evidence marker and documentation, not credited as that public guest command (`admission_journey.sh:48-57,235`; `distill/completeness.md` E4 refinement). The E4 maintenance and waiter requests still use the public `cargo xtask metal run --exclusive` and ordinary `cargo xtask metal run` bootstrap/controller paths. Their shared fixture-private path substitutions are the existing test-support mechanism allowed by P2 (`feature-delta.md:288`); they do not change the default physical barrier or claim that this fixture proves default-path E4. The source-of-truth DISTILL tables preserve the same distinction, and the P2/E4 normative text is unchanged.

### Prior-owner construction and shared barrier

`native-unit-controller.py` uses the completed public ordinary owner's actual `metadata.v2` identity, remote source path, workspace, commit, and source digest, validates the reference run ID/unit, and copies the production `lease-holder.sh` and shared test-support `native-unit` primitive from that bound remote source (`native-unit-controller.py:22-36`). It creates a fresh actor ID from its private directory, concrete inputs under the exact three production cleanup roots, a real Type=exec/KillMode=mixed `systemd-run` unit, and an `ExecStopPost` wrapper which delegates to the captured production `--finalize` (`:31-61`). Its durable helper call has exactly the existing 18 positional values and matches `lease-holder.sh:740-762`; no new production argument or protocol is introduced.

The single `native-unit` primitive is used by both the external prior-owner fixture and the existing Rust `NativeUnit` constructor. The selected command waits on its existing `command.exit` file; the existing finalizer callback waits on `stop` for its existing 20-second bound and then execs production finalization (`fixtures/native-unit:7-19`; `metal_run_isolation_conformance.rs:191-215`). The controlled prior returns command status 0 as fixture input; the separate public guest timeout retains its own 100 result. The existing 60-second RuntimeMaxSec and 45-second TimeoutStopSec remain.

The E4 triad shares one actor-owned maintenance lock/owner and tree lock/owner. The controlled prior receives those paths in its existing durable arguments. The public EX and waiter commands receive the same values through the existing `OVERDRIVE_METAL_*_PATH` fixture inputs (`admission_journey.sh:37-40,116,158`). The initial public timeout owner has completed before the private-barrier triad starts; it cannot self-block that triad on the default barrier. The actual default public maintenance and overlap evidence remains separately attributed to its retained captures.

### Owner ordering, observations, and cleanup

The retained E4 witness now brackets the exact owner sequence required by P2: ordinary SH/running → public EX waiting → command exit input → prior ordinary `terminal.v1` command/0 and systemd `deactivating` with MainPID 0 → EX `waiting_for_lease` holding actual `FLOCK WRITE` → waiter registered in `waiting_for_lease` without a shared FLOCK (`admission_journey.sh:90-184`). The waiter sample is taken with all three owner IDs in one `observe` call. The observer verifies `unit`/`phase` again after the multi-owner read and attaches remote monotonic timestamps; the assertions require the predecessor to remain deactivating and EX/waiter to remain in their respective phases across this bracket (`observe_host.py:185-194`). Fixture paths are validated constructor inputs, not owner-state assertions. Query failures remain captured by each `*-query-*.stderr` file and cannot satisfy a snapshot predicate.

Only after the simultaneous witness does the driver release the existing `stop` input. The following snapshot requires the prior completion receipt, process absence and all three roots absent while EX advances to `preparing|running` and the same waiter remains unadmitted (`admission_journey.sh:185-210`). Public maintenance independently checks the predecessor unit and roots before its marker. The driver waits for all four participants, preserves exact 100/0/0/0 results, collects the controlled owner through public `metal collect`, checks final receipt/terminal/root complements, and removes the private control directory only after all units are stopped (`:211-238`). There is no synthetic owner, phase, terminal, receipt, lock result, or cleanup success.

### Scope and verification limits

The fixture is within DISTILL's private test-support ownership. The updated `feature-delta.md` rows clarify coverage and attribution; they do not revise P2/E4, add a design decision, or weaken any outcome. The default physical barrier, source API, runtime interfaces, public commands, selected acceptance identities, model and production lifecycle are unchanged. The controlled private barrier is evidence only for the real owner/lock protocol; the test does not credit it as a direct default-barrier guest journey or as a completed native run.

The orchestrator reports that the canonical Lima `cargo check -p xtask --all-targets --features integration-tests` completed with exit 0 in 0.64s. This closes the prior compilation limitation; it is not a test execution. The author reports scoped formatting, syntax, primitive/input and offline observation checks. This reviewer ran no tests, checks, native, Metal, SSH, or remote commands. The new external fixture has not been run under native Linux systemd, so E4 and the full admission driver remain **PENDING** until the root retains that actual post-fix result.

### Findings and remediation disposition

- No accepted-contract or test-support boundary finding remains. Iteration 11's owner-level E4 proof remains accurately distinguished from the public default timeout and separate default-path overlap/blocked-maintenance evidence.
- No new state, priority policy, production seam/API, owner, test identity, timeout increase, maintenance delay, or outcome relaxation was found.
- No source, acceptance body, model, production code, or DES event was changed by this reviewer.

### Verdict

**APPROVED — focused review of commit `be7d50659adc8b4c6808b782c2207d87647cc002`.** The controlled NativeUnit prior, actual systemd finalizer, shared existing fixture barrier, public EX/waiter bootstraps, bracketed real owner observations, and explicit evidence limits satisfy the accepted P2/E4 owner-level test-support contract. This approves fixture authoring only; native E4, post-fix admission/placement and the full 01-02 acceptance gate remain **PENDING**.

## Iteration 13 — focused review of native VMM fixture run-parent preparation

### Review metadata

- Scope: commit `6fbb68fafc48a88057d2edf123bc5579bf236a80`, limited to the supplied run-parent preparation and failure-only diagnostics in the two existing `overdrive-host` integration fixtures, plus their test-support notes.
- Triggering native failure: direct-native run `57bd9829da0b29fefbb3ea19` in `placement-rerun-2026-11-09-040806Z/`, source commit `be7d50659adc8b4c6808b782c2207d87647cc002`, selected peer test failed with `NeverConnected` after 60.806 seconds.
- Review basis: accepted P3 native-parent requirement at `feature-delta.md:325`, native applicability at `:412-420`, the preserved 040806 raw capture and earlier increments L/N, and the exact diff in `vsock_vmm_device.rs` and `vmm_equivalence.rs`.
- Result: **APPROVED — focused test-fixture remediation**. Post-fix native placement remains **PENDING**.

### Reproduced failure and accepted parent contract

The current 040806 raw capture proves the selected direct-native body reached the peer-connect assertion and failed `NeverConnected` after 60.806 seconds. That capture does **not** include a current-run parent `lstat`, VMM argv, `VmmExit.stderr_tail`, serial console, or exact child errno. It therefore does not prove that this run saw EACCES or the earlier mode. The retained preceding evidence is separate: increment L records VMM `CreateApiServerSocket` permission denied with the supplied intermediate `run` parent `root:root` mode 0750 while the allocation child is GID 4200; increment N prepares that parent as `root:4200` mode 0710 and passes the same 128 MiB direct-native identity, confinement, device and data-round-trip assertions. The accepted P3 contract explicitly assigns GID-4200/mode-0710 traversal to the caller-supplied native run parent before `Vmm::create` (`feature-delta.md:325`). This is required fixture fallout, not a new VMM API or a speculative product failure.

### Scope, ordering, and ownership

Both changed constructors prepare only their supplied `run_root`: `create_dir_all`, `chown(run_root, None, Some(OVERDRIVE_VMM_UID))`, then mode `0710`, before deriving/creating the allocation child or invoking VMM creation (`vsock_vmm_device.rs:113-130`; `vmm_equivalence.rs:80-97`). `None` preserves the caller-supplied parent UID; the existing VMM group gets traversal but no group write access. The mode is explicit and does not depend on the inherited umask. The test does not chown the supplied parent to the VMM UID, change the allocation child ownership contract, or broaden access to an ancestor.

The `vmm_equivalence` helper is shared by its existing Sim and Cloud Hypervisor comparison sequences, so both receive the same prepared `VmConfig` run parent; no test identity or oracle changed. The Sim identity now has the required `/// CONTRACT_SHAPE: bounded-change.` declaration (`vmm_equivalence.rs:177-179`). Existing 128 MiB sizing, selected native launcher, UID/GID/device-group checks, seccomp/Landlock assertions, VMM backend, data round trip, termination and cleanup assertions are retained. This is test-support construction, not a product namespace, network, VMM, or lifecycle change.

On the direct-native peer failure branch only, the existing 60-second `accept_peer` result is matched without changing its timeout or failure. Before `KillOnDrop` and `WorkDir` cleanup, the branch records the same error, a zero-duration exit-watch observation, `symlink_metadata` for the supplied parent and allocation child, and the existing bounded VMM diagnostic tail (`vsock_vmm_device.rs:432-452`). This is diagnostic-only: it adds no wait, replacement errno, success condition, test identity, memory setting, confinement option, or error relabeling. The 040806 capture remains an honest pre-fix failure; it is not retrospectively filled with these diagnostics.

### Verification limits and remediation disposition

The author reports scoped formatting and the canonical Lima `cargo check -p overdrive-host --test integration --features integration-tests,kvm-tests --locked` passed. The strict Clippy attempt reports only an untouched pre-existing `clippy::doc_markdown` at `vm_host_state_equivalence.rs:172`; no lint override or edit to that unowned file was included. These are reported results, not reviewer runs. This reviewer ran no tests or checks. There is no native execution after `6fbb68f`; the retained L/N pass is supporting diagnosis, not post-fix proof.

- No production contract or fixture-boundary finding was accepted. The change satisfies the explicit caller-owned run-parent requirement with the smallest permission adjustment and preserves both direct-native oracles.
- No source, acceptance body, model, production API, DES phase event, native command, Metal command, SSH command, or remote operation was changed or run by this reviewer.

### Verdict

**APPROVED — focused remediation in commit `6fbb68fafc48a88057d2edf123bc5579bf236a80`.** Both existing native fixture constructors now make the supplied run parent traversable by the approved VMM identity while preserving UID, child setup, 128 MiB memory, confinement, backend and data assertions. Failure-only diagnostics stay bounded and run before existing cleanup. The original 040806 failure remains the reproduced pre-fix record; post-fix native placement and the full 01-02 acceptance gate remain **PENDING**.

## Iteration 14 — focused review of placement collector-output determinism

### Review metadata

- Scope: commit `841354ae5541b588073a2ec171c34a7364c9a367`, limited to the Cargo invocation used by `placement_journey.sh` collector cases and its DISTILL completeness note.
- Triggering evidence: preserved placement capture `placement-rerun-2026-11-09-050128Z/`, selected run `8c1b67ba311626eec8afbadc`, source commit `6fbb68fafc48a88057d2edf123bc5579bf236a80`.
- Review basis: exact diff, raw prior capture and its command statuses, unchanged byte-comparison and result assertions, and the adjacent approved collector-launcher approach in the worktree driver.
- Result: **APPROVED — focused driver-output fix**. Native placement remains **PENDING**.

### Reproduced comparator failure and owner attribution

The retained 045906Z capture shows the actual selected `attempts` command returned status 100. Both read-only collection invocations also returned 100; their stdout streams are byte-identical, and their stderr differs only in Cargo's first `Finished dev profile` line (`0.07s` versus `0.06s`). The remaining stderr bytes match, including the same `OVERDRIVE_METAL_RUN_COMPLETE` result. The driver failed at its exact stderr `cmp`, before the direct-native case; the preserved README and raw files do not claim that the 6fbb VMM run-parent remediation was exercised.

### Contract and implementation mapping

| Accepted evidence contract | Implementation in `841354a` | Review disposition |
|---|---|---|
| Repeat collection is a read-only operation and repeated collected output/status/diagnostics remain exact. | `run_case` now selects `cargo --quiet xtask metal ...` only when the post-shift subcommand is exactly `collect` (`placement_journey.sh:21-33`). Existing command statuses, exact stdout/stderr `cmp`s, result markers, errors, and later assertions are unchanged (`:59-63,108-113`). | Suppresses only Cargo launcher banners at their source; it does not normalize, filter, or rewrite collector output. |
| Non-collection commands retain their existing launcher and behavior. | Every other `run_case` uses the unchanged `cargo xtask metal ...` invocation. The foreign-fixture cleanup path remains direct `cargo xtask metal run --exclusive`; the test's selected commands, inputs, and expected statuses are untouched. | Narrow scope; no product source/API, CLI, environment, status, or test identity change. |
| Admission-driver diagnostics retain their existing assertions. | `admission_journey.sh` has no repeated stderr comparison and is not modified by this commit. | No unrelated driver edit. |

`praise:` The change removes an unstable Cargo-only timing banner without weakening the byte-for-byte comparison that detects changes in the collector's own diagnostics.

### Verification limits

The commit's evidence note reports an offline replay of the raw stderr pair, shell syntax/whitespace checks, and no native rerun; this reviewer did not rerun those checks. The 050128Z capture predates `841354a` and records the real original failure. The corrected launcher has not been exercised by a placement rerun. That capture stopped before the direct-native VMM case, so it neither validates nor refutes the `6fbb68f` run-parent fixture correction.

### Findings and remediation disposition

- No implementation or acceptance-contract finding was accepted. The previous comparator failure was caused by differing Cargo elapsed-time banners, while the retained collector bytes/status were identical.
- No stream filtering, assertion weakening, extra product output, or replacement of nextest/collector result ownership was introduced.
- No tests, native, Metal, SSH, remote commands, source files, models, or DES phase events were changed or run by this reviewer.

### Verdict

**APPROVED — focused fix in commit `841354ae5541b588073a2ec171c34a7364c9a367`.** It adds Cargo quiet mode only to collector launches and preserves exact comparison of the collector's stdout, stderr, and status. The corrected placement driver and direct-native fixture remain **PENDING** until root retains the actual post-fix native results.

## Iteration 15 — focused review of nested module-load ordering

### Review metadata

- Scope: commit `f73ea2c8c7f1b3155a30a93bd66ac9bab0193474` only, limited to the nested per-test setup's module ordering and XFS staging/list membership, plus its truthful failure note.
- Triggering evidence: pre-fix native placement capture `placement-rerun-2026-11-09-051107Z/`, run `9c28953bbc4e4e93803cb8f8`, source commit `841354ae5541b588073a2ec171c34a7364c9a367`.
- Review basis: approved P4's retained explicit-suite batching, the existing batch and per-test nested owner paths, the commit diff, and the raw pre-fix `chmod`-before-module-load failure.
- Result: **NEEDS_REVISION** for one reachable regression in the existing batched nested-test path. No production/API or architecture finding is claimed.

### Reproduced failure and intended per-test ordering

The preserved 051107Z capture records that the direct-native case passed status 0, then the selected `guest-group/nested-boundary` case returned existing `guest_execution_failed`/aggregate 100 in 3.104 seconds before its test body. Its guest console shows `chmod` failing because `/dev/kvm` and `/dev/vhost-vsock` were absent. This capture predates `f73ea2c`; it does not validate the new module ordering or claim anything about a post-fix nested run.

For the intended nextest per-test path, the fix is appropriate in isolation. `nested_vm_test_support` stages XFS separately from `guest.module_paths`, because `prepare_nested_attempt` already loads XFS with `command_with_modules` before `losetup` and the XFS mount (`fwd_guest_suite.rs:1987-2006,3271-3278`). The fix then loads the remaining staged module paths before changing `/dev` permissions (`:3296-3309`). The existing batch `run_nested_boundary` path similarly loads XFS for loop setup and then loads its device-module list before `chmod` (`:2950-2995`). The selected per-test setup preserves typed guest-command failure propagation and changes no body, placement, memory, VMM interface, or expected result.

### Blocking finding — shared support change removes XFS from the existing batched path

`issue (blocking):` `nested_vm_test_support` is also used by the existing batched nested-test owner, `run_nested_vm_test_binary_with_resources` (`fwd_guest_suite.rs:2299-2324`), which is called for the STEP 14-01 CLI and control-plane cells from the explicit suite (`:2862-2885`). This commit removes XFS from the helper's returned `module_paths` for **all** consumers, while only the nextest `prepare_nested_attempt` path explicitly loads XFS before `losetup` (`:3273-3278`). The existing batched path bulk-loads `guest.module_paths` (`:2323-2324`) and then runs `losetup`/`mount -t xfs` (`:2352-2353`), with no separate XFS module load. The file remains staged in `extra`, but GuestKit image construction copies staged files; it does not load kernel modules (`guest_kit.rs:545-550`). `GuestPeer::run_with_modules` performs explicit `finit_module` only for the supplied path list (`guest_peer.rs:1105-1114`). The earlier `run_nested_boundary`'s separate XFS load for loop setup confirms this dependency is intentional.

This is a reachable regression in the existing explicit batched suite, not a hypothetical owner state: that suite calls the affected helper and consumes its module list before mounting its XFS reflink image. The retained 051107Z failure does not exercise this later batch path, so no post-fix failure result is claimed.

**Bounded remediation:** preserve XFS in the shared `nested_vm_test_support` module list for existing batched consumers, and exclude it only from the later per-test bulk load after `prepare_nested_attempt` has explicitly loaded it for `losetup`; alternatively keep a single explicit XFS load in each affected batch owner. Preserve exactly one XFS load per guest and retain the new device-module-before-`chmod` ordering. No new API, fixture identity, module, timeout, body or architecture choice is needed.

### Verification limits and remediation disposition

- Iteration 15 rejects approval until the shared helper no longer drops the required XFS load from `run_nested_vm_test_binary_with_resources`.
- No production defect is alleged. The issue is a test-support ownership regression affecting the existing batched suite, contrary to P4's explicit batching contract.
- The commit reports Lima check/Clippy, eight existing placement acceptance tests and scoped formatting; this reviewer did not rerun checks. The retained 051107Z run is pre-fix evidence, and no native run occurred after `f73ea2c`.
- No tests, native, Metal, SSH, source, model, production code, or acceptance body were changed or run by this reviewer.

### Verdict

**NEEDS_REVISION — commit `f73ea2c8c7f1b3155a30a93bd66ac9bab0193474` fixes the selected nextest nested setup order but also removes XFS from the existing batched nested-test module list.** Keep XFS's separate early load in `prepare_nested_attempt` while preserving its explicit load for the batched `run_nested_vm_test_binary_with_resources` path. Post-fix native placement remains **PENDING**; this reviewer ran no tests or native commands.

## Iteration 16 — re-review of shared XFS module-path remediation

### Review metadata

- Scope: commit `eb210bb204e6d93f860faa8de20b1cc30ca35462`, limited to restoring the shared nested XFS module path and filtering it from the per-test bulk load after its explicit loop-setup load.
- Finding addressed: Iteration 15's missing XFS load in the existing batched `run_nested_vm_test_binary_with_resources` path.
- Review basis: exact diff, all `nested_vm_test_support` call sites, the existing `prepare_nested_attempt` and batched nested owners, and the accepted P4 batching contract.
- Result: **APPROVED — Iteration 15 finding closed**. Native placement remains **PENDING**.

### Caller-by-caller XFS ordering

`nested_vm_test_support` once again stages XFS into both `extra` and the shared `module_paths` list (`fwd_guest_suite.rs:1987-2008`). This preserves the existing batched `run_nested_vm_test_binary_with_resources` caller: it passes the complete list to `command_with_modules` before `chmod`, `losetup`, and the XFS mount (`:2299-2353`). That path therefore loads XFS once before using the reflink volume. Its callers in the existing explicit `cgroup-default` suite remain unchanged (`:2862-2885`). The independent batched `run_nested_boundary` path still stages and explicitly loads XFS before `losetup`, with its device modules loaded before `chmod` (`:2947-2995`).

For the nextest per-test path, `prepare_nested_attempt` continues to load the staged XFS path explicitly with `losetup` before mounting the XFS volume (`:3268-3275`). It now removes exactly `XFS_MODULE_GUEST_PATH` from the later bulk list and loads every remaining module before the device-node permission change (`:3293-3309`). The filter is scoped to this caller after XFS has loaded; other shared-helper consumers keep XFS in their bulk list. This prevents a second `finit_module` for XFS in the per-test path without removing the XFS load from the batched path. The staged `GuestFile` is still required and installed in the image (`guest_kit.rs:545-550`); it is not itself treated as a module-load operation. `GuestPeer::run_with_modules` loads only the supplied paths and propagates a module-load error (`guest_peer.rs:1105-1114`).

The ordering follows the existing setup contracts: XFS is available for loop setup, nested KVM/vhost and remaining staged modules load before `/dev/kvm` and `/dev/vhost-vsock` are changed, and any module-load failure remains an ordinary guest-setup failure before the selected body. No module is skipped on error, duplicate XFS load is avoided in the per-test path, and the previous batch setup continues to load XFS exactly once.

### Findings and verification limits

- Iteration 15's blocking finding is **closed**. The shared helper once again supplies XFS to the existing batched nested-test owner; only `prepare_nested_attempt`, which already loads XFS earlier for `losetup`, excludes it from its later bulk load.
- The preservation retains the existing explicit-suite batching and the exact XFS image/mount, module and device permissions; no placement, selected test identity/body, memory, VMM interface, oracle, or error classification changes.
- The author/orchestrator reports canonical Lima binary check and Clippy, eight existing placement acceptance cases, and scoped formatting/whitespace passing. This reviewer did not rerun them. The 051107Z capture is pre-`f73ea2c` evidence: its direct-native case passed after the prior run-parent correction, then its nested per-test path failed before body execution. There is no native placement result after `eb210bb`.
- No tests, native, Metal, SSH, remote commands, source, model, production API, or acceptance bodies were changed or run by this reviewer.

### Verdict

**APPROVED — focused remediation in commit `eb210bb204e6d93f860faa8de20b1cc30ca35462`.** It restores XFS to the shared module list for existing batched nested-test callers and excludes it only after explicit loading in the per-test path, retaining module-load error propagation and the fixed device-module-before-`chmod` order. Post-fix native placement and the full 01-02 acceptance gate remain **PENDING** until the prescribed native results are retained.

## Iteration 17 — focused review of nested GuestKit scheduler module sources

### Review metadata

- Scope: commit `261bf29cda77b5c04d2124367229d738d8dfb446` only, limited to adding `net/sched/sch_prio` and `net/sched/sch_netem` to the existing nested GuestKit module-source list.
- Triggering evidence: pre-fix placement capture `placement-rerun-2026-11-09-054741Z/`, source commit `eb210bb204e6d93f860faa8de20b1cc30ca35462`; the direct-native selected case passed, then nested GuestKit staging failed before the selected test body.
- Review basis: accepted P3/P4 fixture and placement boundaries, the exact consumer in `GuestKit::image`, `nested_guest_kit_module_source` staging logic, the current diff, and the retained raw placement failure.
- Result: **APPROVED — focused fixture-artifact correction**. Post-fix native placement remains **PENDING**.

### Reproduced consumer failure and producer path

The retained 054741Z capture attributes the source to `eb210bb`, records direct-native status 0, then reports named-nested status 100. The selected outer Rust test body started and attempted to construct its inner scripted image; `GuestKit::image` failed while asking its host module reader for `net/sched/sch_prio`, absent from the outer guest's module tree. The inner guest workload/reset did not start. The capture records the actual staging error and does not claim a completed nested reset, later placement selections, or a final physical-kernel complement.

The source path confirms the exact missing inputs. `GuestKit::image` for `GuestInit::ScriptedPeer` stages both `net/sched/sch_prio` and `net/sched/sch_netem` by calling `host_module_file` (`guest_kit.rs:525-532`). That reader accepts either `<kernel>/.../<relative>.ko.zst` or `.ko` and fails closed if neither exists (`guest_kit.rs:1023-1062`). In the nested path, `nested_vm_test_support` calls `nested_guest_kit_module_source` for every `NESTED_GUEST_KIT_MODULE_SOURCES` entry; that helper stages the host kernel's compressed module at the exact `/lib/modules/<release>/kernel/<relative>.ko.zst` path, or compresses an existing `.ko` to that guest path (`fwd_guest_suite.rs:112-122,1865-1907,1950-1953`). The two additions therefore satisfy both scripted-guest dependencies with the existing source, staging and error owners.

The new entries are added only to `NESTED_GUEST_KIT_MODULE_SOURCES`, which contributes `GuestFile`s to the outer guest image; they are not added to `guest.module_paths` and are not loaded into the outer guest kernel. `GuestKit::image` copies `spec.extra` files into the image (`guest_kit.rs:545-550`). This preserves the nested boundary: the outer guest carries module source files for its nested GuestKit image builder, which later stages them into the inner scripted guest. No runtime `insmod`, parent guest module load, fallback path, or duplicate outer module operation is introduced.

### Contract, scope, and verification limits

The change repairs a missing prerequisite for the already declared `guest-group/nested-boundary` selected body; it does not change test identity, placement, body, assertion, kernel, VMM, module load order, device permissions, error taxonomy, or nested/native evidence credit. The direct-native status-0 result in the pre-fix capture remains credited only to its own existing identity; the following nested staging failure remains pre-body evidence.

The commit's focused Lima binary check, Clippy, eight existing placement acceptance cases, and scoped formatting are author/orchestrator-reported; this reviewer did not rerun them. No native execution exists after `261bf29`; the 054741Z capture is unchanged pre-fix evidence and records an inner-image staging failure inside the selected outer Rust test body, before the inner guest workload starts. The complete placement journey remains **PENDING**. The root separately reports that native placement will run only after this review.

### Findings and remediation disposition

- No further contract or fixture-scope finding was accepted. Both module sources named by the existing `GuestInit::ScriptedPeer` consumer are staged at the exact kernel-release paths its existing reader checks.
- No source, acceptance body, model, production interface, runtime module load, or DES phase event was changed by this reviewer. No test, native, Metal, SSH, or remote command was run.

### Verdict

**APPROVED — focused remediation in commit `261bf29cda77b5c04d2124367229d738d8dfb446`.** It adds only the two existing scheduler module sources required by the inner scripted-guest image builder, using the existing nested module staging path and fail-closed consumer. Post-fix native placement and the full 01-02 acceptance gate remain **PENDING** until their actual results are retained.

## Iteration 18 — focused review of nested `ldd` runtime inputs

### Review metadata

- Scope: commit `dd85152c439151a7703475f5b53b7f70b8acfc78`, limited to staging the existing `ldd` script and its shebang interpreter in the nested guest support file list.
- Triggering evidence: pre-fix placement capture `placement-rerun-2026-11-09-061627Z/`, source commit `261bf29cda77b5c04d2124367229d738d8dfb446`, where direct-native passed and the nested selected body failed during inner image construction.
- Review basis: exact commit diff, the captured nested `GuestKit::image` failure, `nested_guest_kit_module_source`, `GuestKit::image`/`shared_libraries`, and the retained read-only canonical-target helper probe.
- Result: **APPROVED — focused nested fixture-input correction**. Post-fix native placement remains **PENDING**.

### Reproduced consumer failure and exact helper path

The 061627Z capture records direct-native status 0 followed by the existing nested `guest_boundary` Rust test body returning status 100 during inner scripted-image construction. The inner guest workload did not start. The raw error is `GuestKitError::SharedLibraries { binary: "/usr/sbin/tc", detail: "spawning ldd: No such file or directory" }`. The existing consumer is nested scripted-image construction: `GuestKit::image` installs `/usr/sbin/tc` and calls `shared_libraries(tc)` (`guest_kit.rs:525-536`), which executes `Command::new("ldd")` and preserves spawn/nonzero failures as `GuestKitError::SharedLibraries` (`:1092-1105`). `nested_vm_test_support` stages the nested module source files and tool inputs into the outer guest, but before this fix did not provide an `ldd` executable or its shebang interpreter.

The retained read-only canonical-target probe says `/usr/bin/ldd` is an executable shell script with shebang `#!/bin/bash`; `/bin/bash` is an ELF executable, and the existing host-side `shared_libraries` expansion reports its interpreter and shared-library dependencies. The probe prints only the first 12 script lines, so it does not establish an exhaustive inventory of any other script helper commands. This source review does not infer or add such helpers without evidence; the corrected native nested run remains the evidence needed to establish successful execution of this target's ldd path.

### Producer/consumer mapping and scope

The new `nested_vm_test_support` entries use the existing `tool` resolver and `GuestFile` staging path: `ldd` is copied to `/usr/bin/ldd` with shared-library discovery disabled because it is a script; Bash is copied to the exact `/bin/bash` shebang path with the existing shared-library expansion enabled (`fwd_guest_suite.rs:1947-1956`). `GuestKit::image` installs both files, sets executable permissions from the host files, and installs declared shared libraries for the Bash ELF (`guest_kit.rs:487-550`). The nested consumer's `Command::new("ldd")` uses the existing guest PATH, and the kernel can resolve the recorded shebang and Bash runtime files. No new command, public API, environment name, module load, fallback, or error mapping is added.

The only changed source is the private nested guest fixture's `GuestFile` list. The selected nested identity, placement, nextest argv/environment/retry/status ownership, guest body, image/data oracle, device/module order and VMM configuration are unchanged. The driver still distinguishes the public direct-native result from the separately selected nested result.

### Verification limits and remediation disposition

- No implementation or accepted-contract finding was accepted. The staged files directly address the captured missing-`ldd` spawn error, and the existing closure mechanism provides the shebang interpreter and Bash ELF libraries without recursively running ldd on its own script.
- The author reports focused canonical Lima binary check, Clippy and scoped rustfmt passing. This reviewer did not rerun checks. There is no native placement run after `dd85152`; the 061627Z capture is pre-fix evidence. It records direct-native status 0 and a failure inside the selected outer test's inner-image staging, before the inner guest workload starts; it cannot establish full placement success or post-fix VMM behavior.
- No tests, native, Metal, SSH, remote commands, source, model, production API, acceptance body, or DES phase event were changed or run by this reviewer.

### Verdict

**APPROVED — focused remediation in commit `dd85152c439151a7703475f5b53b7f70b8acfc78`.** It supplies the existing nested scripted-image dependency resolver and its exact Bash interpreter/library inputs through the established private GuestFile staging path, preserving failure mapping and all test identities/oracles. Post-fix native placement and the full 01-02 acceptance gate remain **PENDING** until the root retains their actual results.

## Iteration 19 — focused review of the prepared guest-lane ELF path

### Review metadata

- Scope: commit `2bf81e141c0965aa7264509142d9e968985f6025`, limited to the existing guest-lane operator/owner prerequisite path in `infra/metal/test-wrapper.sh`.
- Triggering evidence: placement capture `placement-rerun-2026-11-09-065706Z/`, source commit `dd85152c439151a7703475f5b53b7f70b8acfc78`, where the direct-native and named-nested cases passed before a guest-lane operator selection failed `lane_not_prepared` before its selected Rust body.
- Review basis: exact diff, accepted P4 lane/dispatcher contract (`feature-delta.md:354-370`), existing `guest-kit-prepare` producer, `GuestKit::prepared` consumer, canonical source-root validation, and the retained current preparation log.
- Result: **APPROVED — focused path correction**. Post-fix native placement remains **PENDING**.

### Failure and producer/consumer path mapping

The 065706Z raw capture shows the selected `overdrive-init::bin/overdrive-init` Nextest identity routed to the existing `guest-lane-operator` fixture, then refused with `lane_not_prepared`/78 in 0.063 seconds. The selected lane body did not run. The preserved owner log identifies the old wrapper path as a sibling of the dispatcher under `target/debug`; the current `guest-kit-prepare` log separately records the existing producer output at `<remote-source-root>/target/guest-kit/in-guest/overdrive-init--overdrive-init`.

The exact producer and consumer paths agree with the one-line correction. `guest-kit-prepare` stages each `IN_GUEST_LANES` ELF with `in_guest_binary_path(target_dir, package, bin)`, which is `<target_dir>/guest-kit/in-guest/<package>--<bin>` (`guest_kit_prepare.rs:169-172,286-293`). The existing `GuestKit::prepared` consumer resolves the same `cargo_target_dir(&root)/guest-kit/in-guest/...` path (`guest_lane.rs:144-162`). The retained ordinary Metal preparation log resolves that target directory to the bound checkout's `target` and prints the exact artifact path. In this Metal run, the bootstrap injects the validated canonical `OVERDRIVE_METAL_SOURCE_ROOT`; the guest-lane branch now constructs the existing artifact path under that root (`test-wrapper.sh:38-45,213-225`).

The dispatcher remains at its separate existing `<source-root>/target/debug/overdrive-fwd-guest-suite` path (`bootstrap.sh:671-675`); its realpath, source-target containment, file, and executable checks are unchanged. The in-guest ELF remains a fail-closed `-f`/`-x` prerequisite that returns existing `lane_not_prepared`/78. No fallback path, on-demand build, selected identity or fixture mapping change was added. Both `guest-lane-operator` and `guest-lane-owner` use this same existing prepared `overdrive-init--overdrive-init` artifact, consistent with the approved lane fixture contract.

### Contract and scope

P4 keeps the selected Nextest body, its owner and placement, calls the prepared dispatcher for guest groups, preserves the guest-lane fixture's existing owner, and refuses missing prepared context rather than running raw-host or building on demand (`feature-delta.md:354-370`). This patch only corrects the wrapper's in-guest artifact path to match the already-existing producer/consumer. It does not change CLI/environment/wire APIs, typed errors, `GuestKit` ownership, nextest argv/environment/retry/status, test bodies, placement inventory, or product behavior.

`praise:` The path correction distinguishes the prepared guest-lane ELF from the separately prepared host dispatcher and uses the canonical bound source root already validated by the wrapper.

### Verification limits and remediation disposition

- No implementation or accepted-contract finding was accepted. The pre-fix `lane_not_prepared` result is explained by the exact source path mismatch and supplies no claim that the selected lane body ran.
- The orchestrator/author reports the current preparation owner at the corrected artifact path. These observations are consistent with `guest-kit-prepare` and `GuestKit::prepared`; the old 065706Z placement run remains pre-fix.
- No native placement run exists after `2bf81e1`. The capture's earlier direct-native and named-nested passes remain credited only to those individual pre-fix invocations; they do not validate the guest-lane operator/owner path or full placement.
- No tests, native, Metal, SSH, remote, source, production, test body, model, or DES phase event was changed or run by this reviewer.

### Verdict

**APPROVED — focused path fix in commit `2bf81e141c0965aa7264509142d9e968985f6025`.** The wrapper now checks the exact existing guest-lane ELF path under the validated source root, matching `guest-kit-prepare` and the `GuestKit::prepared` consumer while retaining its fail-closed status. Post-fix native placement and the full 01-02 acceptance gate remain **PENDING** until the root retains the prescribed post-fix native results.

## Iteration 20 — focused review of pre-start controller-EOF fixture progress

### Review metadata

- Scope: commit `e26f0448aaa09f692b7bb2be2d601a87ea79c43d`, limited to the existing test-only `NativeUnit` controller-loss driver, its failure-only diagnostics, and the retained RED/GREEN provenance note.
- Reproduced RED/GREEN evidence: `native-evidence/owner-controller-loss-remediation-2026-11-09-064737Z/`, same logged seed `0x8c542458`.
- Review basis: accepted P2 control/error contract, the permanent runner protocol model and existing `NativeUnit`/fixture gate paths, exact commit diff, and retained raw captures.
- Result: **APPROVED — focused conformance-fixture correction**. Native feature acceptance remains **PENDING**.

### Reachable RED path and bounded correction

The retained RED run records action `LoseController(2)` while actor 2 is registered and waiting for checkout: its actual systemd unit is active with the MainPID alive, the maintenance READ flock is held, the fixture's checkout gate file is absent, and there is no terminal or selected-body PID. The old driver closed the real control pipe and then waited for a terminal while its own fixture gate prevented the actual durable owner from reaching the control read. This reproduced the driver's self-held fixture wait; it did not show the production helper skipping an EOF transition.

The revised `World::lose_controller` still closes the actual control pipe first. For pre-start phases only, it then releases the existing barrier and checkout gate files so the actual owner can complete its real admission path and consume EOF (`metal_run_isolation_conformance.rs:553-575`). The fixture `model-flock` adapter waits on those files and then executes the real `/usr/bin/flock`; it fabricates no lock outcome (`infra/metal/tests/fixtures/model-flock:5-28`). After release, the driver waits for the actual terminal and systemd main-process end, then requires exact `terminal.v1 kind=runner error=controller_eof_before_start`, no selected-body PID, and unchanged source snapshot. It does not write any owner record, phase, terminal, receipt, START, or ABORT.

The after-START branch remains the existing detached-controller behavior: it closes the pipe but does not release operation gates or cancel the selected command. The retained targeted GREEN selected the same existing randomized identity/seed plus the existing before-START and after-START controller-loss identities; all three passed, with the after-START owner retaining its actual 255 result. The temporary seed attribute is absent from the final driver, which retains its original randomized four-sample/24-step configuration and model.

### Typed outcomes, projection, and error context

P2 explicitly retains `controller_eof_before_start` for EOF before START (`feature-delta.md:290`); the permanent model maps that runner error to its existing 75 representative and its `controller_eof_before_start` run remains unchanged (`runner_protocol.qnt:76-80,186-188`; `metal_run_isolation_conformance.rs:1073-1086,1210-1218`). The fix does not skip or replace the model action, broaden the approved error set, alter model projections, change a public API, or add an environment key.

`NativeUnit::terminal` attaches actual owner facts only when its existing poll fails: unit fields, phase/terminal bytes, owner argv, discovered fd targets/fdinfo and labeled fixture input-file presence (`metal_run_isolation_conformance.rs:317-355`). `RunnerDriver::step` adds the actual step/error as context without changing the returned success/error state (`:926-960`). These diagnostics run before the existing `World`/`TempDir` cleanup and do not replace state reads with fixture bookkeeping. On the successful pre-start trace, the production terminal and owner-stop/collection path still supplies the observed values.

### Verification limits and remediation disposition

- Iteration 20 found no production implementation or accepted-error-contract defect. The correction lets the real durable owner progress after the driver closes its controller pipe and keeps the exact pre-start typed failure, no-body, source-preservation, finalization and collection assertions.
- The author reports the same-seed RED/GREEN replay plus the existing pre-start and after-START cases passing, final canonical Lima strict Clippy passing, and removal of the temporary seed override. This reviewer did not rerun them. The final unseeded randomized run remains pending; the retained targeted seed does not substitute for it.
- No model check, native/Metal/SSH command, source change, production change, test identity change, or DES phase event was made by this reviewer. The fixture is test-only (`xtask/src/lib.rs` gates the conformance module on `cfg(test, feature="integration-tests")`).

### Verdict

**APPROVED — focused remediation in commit `e26f0448aaa09f692b7bb2be2d601a87ea79c43d`.** The pre-start owner can now reach its existing EOF terminal through the real systemd/flock path; exact error, no-body and source invariants remain asserted, while after-START detachment is unchanged. Native feature acceptance and the final unseeded randomized/native evidence remain **PENDING**.

## Iteration 21 — focused review of P4 guest-lane nextest dispatch

### Review metadata

- Scope: commit `f92580b968af3756bc8df25ffafce9b37b206633` only, covering `infra/metal/test-wrapper.sh`, `crates/overdrive-testing/bin/fwd_guest_suite.rs`, and `crates/overdrive-testing/src/guest_lane.rs`.
- Triggering evidence: pre-fix placement capture `placement-rerun-2026-11-09-072512Z/`, source HEAD `e26f0448aaa09f692b7bb2be2d601a87ea79c43d`; it reached the selected guest-lane operator descriptor but the dispatcher had no lane route and refused `guest group "" has no prepared attempt context`.
- Review basis: exact `f92580b9` diff; P4 selected-ELF/argv/helper/environment/status and closed-placement contracts in `feature-delta.md:350-372,421-441,563,626`; the exact descriptor in `infra/metal/test-placement.json`; existing operator/owner helpers; and the `GuestKit::image`/guest-peer execution path.
- Result: **PENDING — one P4 guest-input hypothesis awaits bounded reproduction through the current production entrypoint and owner path**. No post-fix native evidence exists; feature acceptance remains **PENDING**.

### Dispatch mapping and preserved owner path

The new wrapper branch exports the exact parsed descriptor kind and fixture, the actual selected ELF, and a hex-encoded representation of the wrapper's original libtest arguments before invoking the existing `overdrive-fwd-guest-suite --nextest-wrapper` entry. The dispatcher accepts only the fixed `overdrive-init::bin/overdrive-init` binary and an exact `(kind, fixture, test)` tuple in `INIT_GUEST_LANE_PLACEMENTS`. It checks that the encoded argv and ELF still match the actual invocation. Unknown or incomplete lane context returns the existing `lane_not_prepared`/78 path; no host execution fallback was introduced.

For an operator descriptor, the dispatcher creates a fresh `WorkDir` and invokes the existing `run_in_guest_cell` owner at the declared `INIT_IN_GUEST_LANE` slot. For an owner descriptor, it uses `run_in_guest_owner_cell` and the fixture-matched host-side owner script. The seven owner scripts in `run_guest_lane_owner_script` reproduce the corresponding existing `OwnerLaneHost` interactions and are selected only after the exact descriptor mapping succeeds. The guest-side command uses the actual Nextest-selected executable and libtest arguments; `NEXTEST_*`, `PROPTEST_*`, the approved `OVERDRIVE_*` selectors and `RUST_*` values are passed with the selected working directory. Helpers named by `NEXTEST_BIN_EXE_*` are resolved and staged at their reported paths. The selected cell's peer exit status is returned to Nextest; zero also requires the existing single-pass libtest summary, while Nextest retains its aggregate result and retry owner.

The descriptor is private wrapper-to-dispatcher execution context, not a new product CLI, persistent config, wire field or public Rust signature. The implementation retains one selected cell per fresh lane guest and does not execute or credit the fixture test body as an extra body. The owner-lane host script remains paired with the selected owner cell. The inspected host scripts match the existing in-guest lane fixture's sequence and observable checks; no placement identity or Contract Shape declaration was changed.

### Hypothesis H21-1 — lane guest may invoke an unstaged `/usr/bin/env`

`guest_lane_exec_argv` constructs every nextest lane attempt with `/usr/bin/env --chdir <working-directory> ... <selected-ELF> <original-args>` (`guest_lane.rs:416-425`). The new lane `NextestLaneAttempt.files` contains only the selected ELF and the reported `NEXTEST_BIN_EXE_*` helpers (`:355-388`); `guest_lane_image_files` forwards only that list for a nextest attempt (`:428-440`). Neither the wrapper nor the dispatcher appends an `env` executable to this list. These are source observations; runtime absence at the selected guest's execution point has not been observed.

The source path to be exercised is the actual descriptor branch in `test-wrapper.sh`, then `guest_lane_dispatch`, then `run_in_guest_cell` or `run_in_guest_owner_cell`, then `GuestKit::image`, then the corresponding guest command owner. `GuestKit::image` constructs the disposable rootfs from its explicit stage directory and copies the base init/peer/modules plus `GuestImageSpec.extra` (`guest_kit.rs:493-557`; `mkfs` uses `-d <stage>` at `:1130-1149`). The default lane `GuestImageSpec.extra` has no `/usr/bin/env`; the nextest lane list also has no such `GuestFile`. The operator lane's `overdrive-init::exec_operator_command` spawns the first argv element directly (`overdrive-init/src/main.rs:712-734`); the owner lane's `PeerCommand::Exec` likewise passes its first argv element to `Command::new(program)` (`guest_peer.rs:1774-1788`). This source trace motivates H21-1, but it is not evidence of a current-path runtime failure or its exact outcome. The existing regular guest-group path explicitly adds `tool("env")` and `tool("mkdir")` before selected attempt files (`fwd_guest_suite.rs:3575-3584`), while this lane path does not call that helper.

If the source trace proves to be the complete current runtime path, the absence would conflict with P4's exact selected-body and helper/environment promises (`feature-delta.md:354,358,370`). The approved mapping distinguishes an absent required guest input (`lane_not_prepared`/78) from an existing image/VMM/peer execution failure (`guest_execution_failed`/70; `feature-delta.md:440-441`). No such failure or mapping mismatch is accepted yet: it must first be reproduced with a bounded invocation of the current operator or owner cell, and its actual owner/status/error evidence retained.

**Remediation disposition:** none yet. Do not change source, tests, model, design, or outcome oracles based on H21-1 until the required current-entrypoint reproduction establishes whether the executable is actually absent and what error/status is observed. If reproduced, route the bounded evidence to the original 01-02 crafter for same-step review remediation.

### Verification limits

- The 072512Z capture is pre-fix evidence for the missing dispatch route; it does not execute the new lane argv and does not reproduce H21-1.
- The author reports focused canonical Lima check/Clippy, eight existing placement acceptance cases, shell syntax and scoped formatting passing. This reviewer did not rerun those commands. No post-`f92580b` native guest-lane execution is retained.
- The source-level file-list/argv mismatch is a hypothesis, not an accepted failure. No current-entrypoint reproduction was run by this reviewer under the assigned no-test/no-native boundary; the root will perform the specified operator/owner captures before the full placement journey. Lima checks cannot establish native guest contents or behavior.
- Review of source paths found no separate mismatch in descriptor identity, host-script ownership, selected ELF/argv/helper staging logic, environment whitelist, working directory, Nextest status handoff, or public CLI/wire/config shape. No source, acceptance test, model, DES event, native/Metal/SSH command, or remote command was changed or run by this reviewer.

### Verdict

**PENDING — focused review of `f92580b968af3756bc8df25ffafce9b37b206633`.** The new route preserves the approved closed descriptor and existing lane ownership. H21-1 identifies a source-level `/usr/bin/env` input concern, but no defect, required remediation, or contract mismatch is accepted without the bounded current-entrypoint reproduction. Technical approval awaits that evidence. Native placement and the complete 01-02 acceptance gate remain **PENDING**.

## Iteration 22 — focused P2 review of queued controller EOF

### Review metadata

- Scope: commit `8b1dd117d9ea020c6fb0429896f7a32186574331` only, modifying `infra/metal/lease-holder.sh` to observe controller EOF while an actual maintenance or checkout `flock` waiter is queued.
- Reviewer: independent 01-02 DELIVER reviewer
- Review date: 2026-11-09
- Reproduced pre-fix evidence: `native-evidence/owner-queued-eof-gap-2026-11-09-073342Z/`, especially its `README.md`, `reproduction/foreground-invocation.json`, and `reproduction/foreground.log`.
- Contract basis: accepted P2 in `feature-delta.md:280-296,421-442,508-528`; permanent `specs/quint/metal-run-isolation/runner_protocol.qnt:76-88,186-188`; the documented owner-state mapping and retained current owner observations.
- Result: **APPROVED — focused P2 owner correction**. The exact post-fix native seed has not yet been rerun; overall 01-02 native acceptance remains **PENDING**. H21-1 from Iteration 21 remains unproven and is not resolved by this commit.

### Reproduced production-owner gap and current path

The retained pre-fix native replay decoded `LoseController(actor 1, value 0)` at logged seed `0x8e8e19a4`. The current capture is distinct from the earlier closed-checkout-gate failure. At the observation, the ordinary unit was active with phase `waiting_for_lease`, its real descriptor 8 referenced the fixture maintenance lock without `FLOCK`, its controller stdin pipe had closed, and it had no terminal, checkout descriptor, or selected-body PID. The independent maintainer unit was active in phase `preparing` with an actual exclusive flock on that same barrier. No later maintenance stimulus could run while the handler awaited the ordinary terminal. The modeled poststate was maintenance 1 with actor 1 at phase 5 and terminal 75; the real owner remained queued with no terminal. The native observation and raw command/status attribution are retained in the named evidence directory; this is a reachable owner-path failure, not an inferred lock from the test driver's ledger.

The production path before the correction was `lease-holder.sh --durable` → open maintenance descriptor 8 → blocking `flock -s` or `-x` → only after admission, open/lock checkout descriptor 10 → publish owner records and `preparing` → read controller control. Closing controller stdin while the actual EX owner held the kernel lock could not reach that read. P2 and the permanent model require a pre-START controller EOF to terminate the durable main with `controller_eof_before_start`/75 while preserving the independent maintenance owner.

### Exact correction and ownership lifecycle

`durable_main` still constructs one absolute deadline before the same lock sequence (`lease-holder.sh:861-863`). `wait_for_prestart_lock` asks `remaining_lock_timeout` for only that remaining budget, starts one `flock` child against the already-open parent descriptor, and polls the controller control pipe while that child remains queued (`:402-478`). Ordinary still takes maintenance SH at descriptor 8; explicit exclusive still takes maintenance EX at descriptor 8. The exclusive predecessor-stop wait remains after successful EX acquisition, and the checkout lock remains exclusive at descriptor 10 after that same point. The added polling does not acquire locks in a different order or create another deadline.

When a complete control line arrives during a lock wait, the helper retains its bytes in the existing `control` variable and sets `control_received`; it does not execute the control line, publish an owner, transfer source, or send START during the wait. Once the existing admission and checkout gates and owner-publication ordering have completed, the original control case consumes that saved line; the post-START descriptor detachment and command path are unchanged (`:1084-1127`). This preserves one START, the existing ABORT grammar and precedence, and the rule that the controller can authorize source preparation only after the service owns admission and checkout exclusion.

When the timed read reports EOF rather than timeout, the helper sends TERM to the one `flock` waiter and synchronously waits for it before returning its EOF result (`:433-478`). Each caller then writes the existing runner terminal `controller_eof_before_start` and returns 75 before the next lock or source/owner/command work (`:945-1009`). A lock wait that instead expires retains the distinct existing `maintenance_timeout` or `checkout_timeout` code and status 75. The helper's successful waiter exits after applying flock to the inherited descriptor; the parent descriptor remains the lock owner through the existing durable lifetime. On cancellation the waiter is reaped before the durable function returns. EXIT cleanup calls the same cancellation helper idempotently, and pre-start signal/pipe handlers cancel the waiter too (`:891-933`).

The added terminal is already in the closed runner error validator (`:332-343`) and in the accepted typed mapping as `controller_eof_before_start`/75 (`feature-delta.md:290,421-442`). The terminal file format, metadata v2 order, private 18-argument durable launch, owner fields, run phases and collector are unchanged. After the durable main exits, the existing transient unit's P5 `ExecStopPost` finalizer still owns stop completion and receipt publication; the new code neither writes a completion receipt nor cleans roots itself.

### Contract and regression audit

| Concern | Review disposition |
|---|---|
| Necessity and reachability | The retained native trace proves an ordinary production durable owner waiting on the real shared flock behind an active EX owner, controller EOF, no terminal, and no selected command. The committed change moves EOF observation into precisely those existing flock waits. |
| One admission deadline and distinct timeout causes | The one absolute deadline remains initialized once. Each maintenance/checkout flock gets only its current remaining duration. Maintenance timeout and own-checkout timeout still map to their distinct existing runner codes; EOF uses the approved controller code. |
| Lock order and exclusive owner | Maintenance SH/EX descriptor 8 is still acquired before checkout EX descriptor 10. An EX owner still waits for admitted ordinary units to stop after obtaining EX. The fix cancels only its own unadmitted lock-wait child on controller EOF and does not interrupt or drain the independent maintenance owner. |
| No pre-admission source or START | EOF return is before owner records, `preparing`, START handling, source transfer, and selected command launch. A buffered START is applied only after the original admission, checkout, and owner-publication sequence. |
| Waiter and systemd/P5 lifetime | The flock child is terminated and waited before the durable main returns. Existing EXIT cleanup repeats cancellation safely. The unit and finalizer retain their prior stop/receipt ownership; no detached lock waiter remains to hold the barrier beyond the main. |
| Control grammar and post-START detach | Existing START/ABORT case statements and typed mappings are unchanged. Stdin polling is added only while `running=0`; after START, pipes are detached and the selected command retains existing controller-loss semantics. |
| API/schema/model | The patch adds only private shell functions/state. It changes no public CLI/config/wire fields, 18-position private launch shape, metadata/terminal/completion schema, model action/projection, or test body/oracle. |

No separate contract finding was accepted. The pre-fix reproduction was already independently retained and the current commit directly covers its exact queued-lock state. No broader EOF cancellation during unrelated setup or admitted-unit draining is required to close this reproduced defect.

### Verification limits and remediation disposition

- The author reports canonical Lima shell syntax plus the existing deadline-cause, pre-start-control and v1/v2 collection cases passing after the fix. This reviewer did not rerun them.
- The exact post-fix native randomized seed `0x8e8e19a4` has not yet been rerun. The pre-fix native trace proves necessity and actual reachability; it does not prove the corrected native terminal, reaping, receipt, or collector outcome. The parent plans the post-fix native check before full placement acceptance.
- No tests, model checks, native/Metal/SSH command, remote command, source/model/test change, or DES phase event was made by this reviewer. The only reviewer-authored work is this review artifact.
- H21-1 from Iteration 21, concerning lane `/usr/bin/env` staging, remains unproven. Iteration 22 neither validates nor rejects that hypothesis.

### Verdict

**APPROVED — focused P2 remediation in commit `8b1dd117d9ea020c6fb0429896f7a32186574331`.** The production owner now observes and cancels its own real queued maintenance/checkout flock waiter on pre-START controller EOF, reaps that waiter, and retains the exact runner terminal/status while preserving the one deadline, lock order, source/START boundary, exclusive maintenance ownership, post-START detachment and P5 finalizer. The post-fix native replay and full 01-02 acceptance remain **PENDING**; Iteration 21's independent H21-1 remains pending reproduction.

## Iteration 23 — focused P4 review of closed guest-lane descriptor parsing

### Review metadata

- Scope: commit `1b623e6e167c4d3113efcf0d393c74748fbe9834` only, changing the private descriptor-output delimiter and corresponding shell `IFS` in `infra/metal/test-wrapper.sh`.
- Reproduced triggering evidence: `/private/tmp/metal-native-remaining-20261109T085627Z-operator-owner-cells/foreground-invocation.json` and `foreground.log`; the invocation records source HEAD `8b1dd117d9ea020c6fb0429896f7a32186574331`, exact source dirty state, both selected identities, and outer exit 100.
- Contract basis: accepted P4 closed placement schema and exact lane fixture semantics in `feature-delta.md:360-372,421-442,626`; the same closed placement parser and dispatcher context check; and the actual retained pre-fix output.
- Reviewer: independent 01-02 DELIVER reviewer
- Review date: 2026-11-09
- Result: **APPROVED — focused descriptor-field preservation fix**. Native operator/owner execution after this parser fix remains **PENDING**. H21-1 from Iteration 21 remains unproven.

### Reproduced parser failure and exact fix

The retained post-preparation capture selected one existing `guest-lane-owner` and one existing `guest-lane-operator` identity. Both compiled and reached the mandatory wrapper, but both were refused before guest launch with `OVERDRIVE_METAL_TEST_ERROR error=lane_not_prepared`. The raw messages were `no closed guest-lane placement maps "guest-lane-owner"/"" to "guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection"` and `no closed guest-lane placement maps "guest-lane-operator"/"" to "guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection"`. The foreground command exited 100 because both nextest selections failed. Neither selected test body ran, and the capture claims no guest ELF execution, VMM launch, or guest-kernel result.

The accepted descriptor for each selected identity has the exact guest-lane `kind` and a nonempty `fixture`; the guest-lane schema has no `group` property. The wrapper's Python parser emitted `kind`, `group`, and `fixture` columns, supplying an empty string for the absent group; the shell read them using tab in `IFS`. Bash treats tab as IFS whitespace, collapsing adjacent tab separators and discarding the empty middle column. The capture's dispatcher message is the production-path witness of that exact parse: it received the correct descriptor kind and selected test, but an empty fixture.

Commit `1b623e6e` changes only that field separator to ASCII unit separator (`\x1f`) in both the Python serializer and the shell `IFS` for the `read`. This is a non-whitespace IFS delimiter, so the empty `group` column stays an empty second column while the fixture remains the third column. The closed schema prevents delimiter ambiguity: `kind` is checked against the finite descriptor kinds, group against the eight literal values, and fixture against `integration::in_guest_lane::[A-Za-z0-9_]+` (`test-wrapper.sh:83-101`). Therefore the serialized descriptor's exact fields survive extraction without changing descriptor contents or placement identity.

### Closed inventory, typed refusal, and scope

The parser still reads and validates the complete policy before lookup, rejects duplicate JSON keys, requires the same format and exact top-level fields, validates every descriptor, and retains the existing status mapping for ambiguous, missing, and invalid policy (`test-wrapper.sh:66-145`). The delimiter change does not alter lookup keys, inventory partition, or descriptor kinds. In particular, it preserves the approved lane `fixture` as the existing context/host-script identity and does not turn an empty group into a guest group or a host fallback.

No public API, CLI/config field, service-injected context key, wire/persisted format, error code, placement, selected identity, test body, helper path, status mapping, retry behavior, or guest execution branch changed. The correction is necessary to pass the closed descriptor's already-validated fixture to the existing dispatcher and nothing else.

### Verification limits and remediation disposition

- The author reports eight existing Lima placement acceptance cases and shell syntax validation passing after the separator change. This reviewer did not rerun those checks.
- The retained 085627Z native capture is pre-`1b623e6e` and proves only the empty-fixture refusal. The post-fix operator/owner cells have not yet been run; the root plans those exact cells before the full placement journey. This review does not credit either guest body.
- H21-1 from Iteration 21 remains unproven: the 085627Z capture fails in descriptor lookup before reaching the guest argv or image, so it does not establish whether `/usr/bin/env` is available in either lane guest.
- No tests, native/Metal/SSH command, model check, production or test source change, acceptance-body change, or DES phase event was made by this reviewer. The only reviewer-authored change is this review artifact.

### Verdict

**APPROVED — focused P4 parser fix in commit `1b623e6e167c4d3113efcf0d393c74748fbe9834`.** The non-whitespace delimiter preserves the exact empty-group/nonempty-fixture descriptor shape required by both existing guest-lane kinds. The correction removes the reproduced pre-dispatch refusal without changing the closed inventory or execution contract. Post-fix native operator/owner execution, H21-1 reproduction, and complete 01-02 acceptance remain **PENDING**.

## Iteration 24 — bounded reproduction of H21-1 guest-lane input failure

### Review metadata

- Scope: evidence update against implementation source commit `1b623e6e167c4d3113efcf0d393c74748fbe9834`; no code change is included in this review iteration.
- Native evidence: `/private/tmp/metal-native-remaining-20261109T090326Z-operator-owner-cells/foreground-invocation.json` and `foreground.log`, retained from the ordinary native entrypoint after the descriptor parser fix and canonical guest-kit preparation.
- Invocation: run `941dffc5130c826ce3eeeb89`, source HEAD `1b623e6e167c4d3113efcf0d393c74748fbe9834`; selected owner cell plus operator cell; nextest and outer command both returned aggregate status 100.
- Contract basis: P4 exact selected ELF/argv/helper/environment and guest-input contracts (`feature-delta.md:350-372,421-441,563,626`), existing owner/peer execution path, and the current `GuestKit::image` inputs.
- Reviewer: independent 01-02 DELIVER reviewer
- Review date: 2026-11-09
- Result: **NEEDS_REVISION — H21-1 is now a reproduced owner-lane guest-input defect**. The operator attempt also failed, but this capture contains no operator-specific `ENOENT`; no independent operator cause is claimed. Post-remediation review and full native acceptance remain pending.

### Reproduced owner-lane failure through the current owner path

The owner selection was `overdrive-init::bin/overdrive-init` / `guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection`, mapped to the existing `guest-lane-owner` fixture. The corrected descriptor parser passed the fixture identity to `guest_lane_dispatch`, and the dispatcher selected the existing `run_in_guest_owner_cell` path. The raw owner-cell console contains `run-with-modules: exec /usr/bin/env: No such file or directory (os error 2)`. The owner cell opened no script channel within its existing 60-second bound, so the retained host script did not start. The wrapper reported the existing `guest_execution_failed` marker with the original `GuestKitError::Staging` detail; Nextest returned 100. No selected owner test body or host script completed.

The current source path explains the observed `ENOENT`. `nextest_lane_attempt` creates `GuestFile`s for the selected ELF and reported `NEXTEST_BIN_EXE_*` helpers (`guest_lane.rs:355-409`), while `guest_lane_exec_argv` invokes `/usr/bin/env --chdir ...` (`:437-445`). The owner path passes `guest_lane_image_files` into `GuestKit::image` (`guest_lane.rs:1112-1122`). That image starts from an explicit stage tree and installs the files in `GuestImageSpec.extra` (`guest_kit.rs:493-557`); the pre-fix lane attempt file set contains no `/usr/bin/env`. The existing `run-with-modules` owner executes the supplied program via `CommandExt::exec` and logs its actual spawn failure (`guest_peer.rs:1105-1127`). These paths match the retained guest console and prove H21-1 for the current owner-lane path.

The same native invocation selected `guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection` as the operator cell. Nextest reports that selection failed after 2.046 seconds, but its raw output does not include an explicit `ENOENT` or enough owner diagnostics to attribute that failure specifically to `/usr/bin/env`. This review therefore credits the owner-cell capture as the bounded reproduction; it does not claim an independently diagnosed operator-cell failure.

### Finding F21-1 and bounded remediation disposition

F21-1 is now accepted: the guest-lane attempt invokes a required `/usr/bin/env` executable but the constructed lane image omits it. This prevents the selected owner body from starting and blocks the approved exact selected-ELF/argv/environment execution path. The pre-fix 085627Z descriptor-parser capture did not establish this defect because dispatch stopped earlier; the 090326Z capture does.

Return F21-1 to the original 01-02 crafter for the bounded existing guest-input correction: stage the existing `/usr/bin/env` executable in the lane attempt's `GuestFile` set with its required shared-library closure, preserving fail-closed behavior if the host input is unavailable. Keep selected ELF, argv, environment, CWD, retries, guest ownership, owner host script, body/status oracle and approved error taxonomy unchanged. No public API, new error variant, new test seam, or broader error architecture is required or authorized by this finding.

### Verification limits

- The current native capture directly proves the missing executable in the owner-lane guest and its failure before the selected owner cell/host script. It does not prove full owner-cell success or post-fix behavior.
- The operator cell was selected and failed, but no operator-specific missing-executable diagnostic was retained. That result remains unattributed beyond its nonzero Nextest attempt.
- The author/orchestrator reports this was run after the canonical guest-kit preparation path. This reviewer did not run tests, Metal/native/SSH commands, or new checks.
- The current worktree shows original-crafter remediation in progress, but this iteration reviews only the reproduced failure against commit `1b623e6e`. That working change remains unreviewed until dispatched as a new review input.
- The P2 native seed and full placement journey have separate pending reviews/acceptance evidence; this update changes neither their status nor the P4 parser-fix verdict.

### Verdict

**NEEDS_REVISION — H21-1 is proven against `1b623e6e167c4d3113efcf0d393c74748fbe9834`.** The actual owner-lane run reached the existing guest peer, recorded `ENOENT` for `/usr/bin/env`, and failed before the script channel or selected owner body. Return the bounded existing-input staging correction to the same step's original crafter. The P2 and descriptor-parser fixes remain individually approved; the 01-02 technical verdict, post-remediation review, and native acceptance remain **PENDING**.

## Iteration 25 — focused review of post-correction P2 native evidence

### Review metadata

- Scope: evidence commit `5cac3dfdf3c95eee223335374bd0a401b186454f`; the implementation under test is production helper `infra/metal/lease-holder.sh` at source HEAD `1b623e6e167c4d3113efcf0d393c74748fbe9834`.
- Evidence: `native-evidence/owner-queued-eof-verification-2026-11-09-090213Z/README.md`, seeded and fresh invocation receipts/logs, source-driver diffs, and `final-driver.diff`.
- Contract basis: P2 controller EOF, one-deadline, lock-order, and maintenance-ownership promises in `feature-delta.md:280-296,421-442,508-528`; permanent model mapping in `specs/quint/metal-run-isolation/runner_protocol.qnt:76-88,186-188`; the reproduced pre-fix owner gap; and Iteration 22's code review.
- Reviewer: independent 01-02 DELIVER reviewer
- Review date: 2026-11-09
- Result: **APPROVED — P2 queued-EOF native evidence and bounded diagnostic diff**. This does not complete native placement or the full 01-02 acceptance gate; H21-1 remains a separate unresolved source finding.

### Current-source owner evidence

The retained seeded ordinary Metal command ran the randomized owner trace with the previously failing exact seed `0x8e8e19a4`, plus the existing before-START EOF and after-START controller-loss identities. Nextest reports 3/3 passed and the outer command returned 0. A second default-seeded native command selected the randomized identity alone, generated fresh seed `0x83bfe04`, passed its four existing traces, and returned outer status 0. Both invocation receipts pin source HEAD `1b623e6e` before and after execution and record their dirty source status.

The seeded raw log observes the decisive poststate while the test is still in the actor's pre-start controller-loss action: ordinary run `b92b366452bf37e5f5e9b27d` retained `terminal.v1 kind=runner error=controller_eof_before_start`, with its unit deactivating and MainPID 0. The independent maintainer `be0a9867709b059b090d9b89` was still active with MainPID 2517636, with an actual exclusive fdinfo lock on the same fixture barrier and no terminal. Its recorded phase was still `waiting_for_lease`; the raw diagnostic correctly grounds admission on the kernel fdinfo observation, not on the phase or model ledger. There was no subsequent maintenance START, completion, or release before the ordinary EOF observation. This matches the permanent model poststate exactly: ordinary phase 5 / terminal 75 while maintenance remains 1.

The default-seeded capture also retains an EOF terminal while an independent maintainer is observed active with an actual EX lock. The named before-START and after-START controls pass in the seeded command; their existing typed terminal/result checks remain unchanged. These runs prove the queued EOF correction on the current durable helper and do not credit a selected command result to the waiting ordinary owner.

### Seed restoration and diagnostic diff

The seeded raw driver diff temporarily adds the existing `quint_run` seed attribute for `0x8e8e19a4`; it changes no model action or runtime protocol. That attribute is absent from `final-driver.diff`. The fresh invocation receipt has no explicit seed override, its raw log prints `0x83bfe04`, and its source receipt remains pinned to `1b623e6e`. The evidence therefore records both the exact prior failure replay and a subsequent run after restoring default randomized selection.

The committed final driver diff changes only the stale comment describing the former EOF read ordering and adds actual success-path observations after the existing pre-start terminal/body/source assertions. It reads the ordinary unit facts, and for an active maintainer it prints the unit facts, actual phase, fdinfo-derived lock mode, and retained terminal read result. It does not branch on these values, write owner state, change the modeled poststate, change assertions, create test stimuli, or assert a new outcome. The extra ordinary `unit_facts` query is an actual observation for the log; terminal existence and MainPID-ended state were already established by the existing `unit.terminal()` and `unit.ended()` steps. If that observation fails, the evidence test reports an observer error rather than treating unobserved state as a pass. The maintainer read results remain printed `Result`s; they are not replaced by driver bookkeeping.

The raw captures retain the existing idempotent Drop messages `Failed to stop ... Unit ... not loaded.` without stripping or normalization. The source receipts retain unrelated pre-existing dirty work. No production helper, model, projection, protocol error, timeout, test identity, selected body, assertion, or DES phase event changed in this evidence commit.

### Test-coverage attribution and limits

This is not a fresh all-fifteen execution. The prior `owner-conformance-2026-11-09-060408Z` artifact records 14/15 successful identities on its own older source receipts, with the randomized identity failing. Commit `5cac3df` closes that randomized identity on source `1b623e6e` and reruns both controller-loss controls on that source. Across the separately attributed artifacts, all fifteen identities have successful executions, but the prior fourteen are not re-credited as having run on `1b623e6e`, and no single current-source all-fifteen pass is claimed.

The author reports scoped rustfmt, `git diff --check`, and strict canonical Lima test-target Clippy passing after the default seed was restored. This reviewer did not rerun checks, native commands, tests, or a model-check campaign. The captured actual owner terminal/MainPID and active EX observations validate the P2 queued-EOF correction, but they do not claim P5 stopped-unit receipt completion or full P4 native placement. H21-1 remains independently proven and unresolved; this evidence commit neither reviews nor credits its guest `/usr/bin/env` remediation.

### Verdict

**APPROVED — focused P2 evidence/diagnostic commit `5cac3dfdf3c95eee223335374bd0a401b186454f`.** The exact failing randomized seed and a fresh unseeded randomized run pass through the current production durable helper, and the actual EOF terminal is observed while an independent EX owner remains unchanged. Seed restoration is retained, the diagnostic diff introduces no model/oracle/protocol change, and no output is normalized. This is not a fresh all-fifteen run or full placement acceptance. The overall 01-02 implementation verdict remains **NEEDS_REVISION** until H21-1 is remediated and independently reviewed.

## Iteration 26 — focused review of guest-lane environment-helper staging

### Review metadata

- Scope: commit `22320c881bc8b2ec004dbbb89cce14634b9cd025`, modifying only private implementation in `crates/overdrive-testing/src/guest_lane.rs` plus preserved failure evidence.
- Reproduced necessity: `native-evidence/guest-lane-dispatch-2026-11-09-090326Z/README.md`, with its source receipt, pre-fix ordinary native foreground log, canonical preparation receipt, and retained earlier descriptor-attribution captures.
- Contract basis: accepted P4 exact selected ELF/argv/helper/environment/CWD/status contract in `feature-delta.md:350-372,421-441,626`; the `GuestKit::image` `GuestFile` staging owner; and the current production/test-support caller path.
- Reviewer: independent 01-02 DELIVER reviewer
- Review date: 2026-11-09
- Result: **APPROVED — bounded P4 guest input correction**. Post-fix native operator/owner execution remains **PENDING**; this review credits no post-fix guest outcome.

### Reproduced necessity and exact staged dependency

Iteration 24 retains the ordinary native failure at source `1b623e6e`: after canonical `guest-kit-prepare` succeeded at the same source, selected owner cell `guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection` entered the existing `guest-lane-owner` route. The guest console recorded `run-with-modules: exec /usr/bin/env: No such file or directory (os error 2)`, and the owner host script received no script-channel connection within its existing 60-second bound. The attempt failed through the existing `guest_execution_failed` marker with `GuestKitError::Staging` context; no selected owner body or host script completed. This directly reproduces H21-1 against the P4 route. The same capture's operator cell failed in 2.046 seconds without an explicit errno, so this review does not independently attribute that operator result.

The current fix stages the existing environment command at the exact path already present in `guest_lane_exec_argv`: private constant `NEXTTEST_ENVIRONMENT_COMMAND` is `/usr/bin/env`, and `nextest_environment_command_file` canonicalizes the existing host executable, adds a `GuestFile { guest: /usr/bin/env, with_shared_libraries: true }`, and uses the existing `seen` map to avoid duplicate paths or fail closed on conflicting sources (`guest_lane.rs:129-130,334-352`). This is included only when Nextest lane context decoded successfully (`:354-384`). `GuestKit::image` installs each extra file at its declared guest path and, when requested, stages the ELF interpreter and shared libraries returned by the existing `shared_libraries` resolver (`guest_kit.rs:545-551,1092-1117`). That resolver parses the absolute interpreter/library paths from `ldd`; no ldd program is needed in the guest for this closure.

The argv producer now uses the same constant, preventing path drift (`guest_lane.rs:440-449`). The selected ELF remains staged at Nextest's exact reported guest path with its shared-library closure; `NEXTEST_BIN_EXE_*` helper handling, original arguments, filtered environment, PWD/CWD, retry context, and the fresh `WorkDir` per dispatcher invocation are unchanged. The operator and owner callers still use their existing `run_in_guest_cell` / `run_in_guest_owner_cell` paths, and the owner host-script closure/lifetime remains unchanged (`fwd_guest_suite.rs:4061-4080`; `guest_lane.rs:1112-1145`). For callers without a Nextest attempt context, `nextest_lane_attempt` still returns `None`, preserving the existing non-Nextest lane image and command.

### Typed error, API, and oracle audit

If the host-side `/usr/bin/env` cannot be resolved, the new resolver returns the existing `GuestKitError::LaneNotPrepared`; conflicting file ownership uses the same existing closed refusal. The dispatcher already maps `LaneNotPrepared` to the approved `lane_not_prepared`/78 marker and status, while genuine guest image/peer failures retain their existing `guest_execution_failed` mapping and original `GuestKitError` cause (`fwd_guest_suite.rs:4082-4088,4126-4136`; `feature-delta.md:440-441`). No new variant, error string substitute, status, CLI/config/environment key, protocol field, public Rust API, test identity, body, or outcome assertion was added or weakened.

The source diff is the smallest closure of the reproduced missing input: it stages the required executable and its existing ELF closure at the exact consumed path. It does not stage a shell, build tool, workspace tree, fallback program, or a different interpreter. It preserves the selected test's status path and does not route a fixture body as an additional credited test.

### Verification limits and remediation disposition

- The pre-fix native capture and source path prove the missing staged executable was reachable and prevented the selected owner test from launching. The README records successful canonical preparation and the exact guest-console `ENOENT`.
- The author reports focused canonical Lima Clippy, scoped formatting, and eight existing placement acceptance cases passing for the fix. This reviewer did not rerun them.
- There is no native run after `22320c8` in the retained evidence. The root is preparing the current dispatcher and the same two selected cells. Their guest image contents, owner-script result, operator cell, and final status remain uncredited until that capture exists.
- The earlier 090326Z operator attempt failed but did not retain its own explicit `ENOENT`. This finding and remediation are grounded in the owner-cell's direct console error; they do not claim that the operator body ran or passed.
- No tests, native/Metal/SSH command, model check, production source change, acceptance-body edit, or DES phase event was made by this reviewer. Iteration 26 updates the review artifact only.

### Verdict

**APPROVED — bounded P4 remediation in commit `22320c881bc8b2ec004dbbb89cce14634b9cd025`.** The existing guest-lane argv now has its consumed `/usr/bin/env` executable and actual ELF runtime closure staged through the existing `GuestFile`/`GuestKit::image` path. The fix preserves the selected invocation, helper/environment/CWD behavior, host-script lifecycle, closed typed failures, public interface, and test oracles. Post-fix native operator/owner execution and the full 01-02 acceptance gate remain **PENDING**.

## Iteration 27 — focused review of exact guest-lane working-directory staging

### Review metadata

- Scope: commit `47328fb88d21d5f3f57d25a935d1a17c505cbb9b`, modifying `GuestKit::image` internals and the existing guest-lane image builder to materialize one exact working-directory path.
- Reproduced trigger: `/private/tmp/metal-native-remaining-20261109T092409Z-operator-owner-cells/foreground-invocation.json` and `foreground.log`; source HEAD `22320c881bc8b2ec004dbbb89cce14634b9cd025`, with outer native command exit 100.
- Contract basis: P4 exact working-directory, selected ELF/argv/environment/helper/status contract in `feature-delta.md:350-372,421-441,626`; the retained current owner-cell failure; and the existing per-attempt `GuestFile`/`GuestKit::image` and owner-host-script paths.
- Reviewer: independent 01-02 DELIVER reviewer
- Review date: 2026-11-09
- Result: **APPROVED — bounded exact-CWD image-input correction**. Post-fix native operator/owner execution remains **PENDING**; no successful body result is credited here.

### Reproduced missing-CWD failure after the environment helper was staged

The 092409Z native invocation selected the existing owner cell `guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection` and operator cell `guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection`, with source HEAD `22320c8`. The owner guest console now gets past `/usr/bin/env` but records `env: cannot change directory to '/home/ubuntu/overdrive-worktrees/e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410/crates/overdrive-init': No such file or directory (os error 2)`. The owner cell consequently opened no script channel within its existing 60-second bound; the selected cell and existing host script did not complete. This is distinct from Iteration 24's `/usr/bin/env` spawn `ENOENT`: this invocation proves the staged environment helper ran and its requested CWD was missing.

The operator selection also failed after 2.176 seconds, but the retained output provides no operator-specific CWD error. The review uses the owner cell's explicit `env --chdir` failure as the bounded reproduction and does not infer the operator failure's cause. The outer Nextest command returned 100; neither capture claims a passing selected body.

### Exact fix and owner/path audit

`nextest_lane_attempt` already captures the exact current working directory and `guest_lane_exec_argv` passes that path unchanged to `/usr/bin/env --chdir`. The fix routes Nextest attempts through `build_guest_lane_image`, which calls the new crate-private `GuestKit::image_with_directories` with only `attempt.working_directory`; legacy non-Nextest lane calls still use the existing `GuestKit::image` path with no extra directory (`guest_lane.rs:458-478,508-521,1123-1138`).

`image_with_directories` creates only directory entries in the fresh guest image stage: each supplied path must be absolute and contain no `.` or `..` components, then its relative path is joined under the private stage root and created with `create_dir_all` (`guest_kit.rs:487-539`). It copies no host CWD files or checkout tree. The existing `GuestKit::image` signature is unchanged and delegates with an empty directory slice, so unrelated image callers preserve their former contents. The caller supplies only the exact `std::env::current_dir()` captured for this Nextest attempt; there is no broader directory inventory or caller-provided path list.

The selected ELF, `/usr/bin/env` plus its resolved shared-library closure from Iteration 26, reported `NEXTEST_BIN_EXE_*` helpers, encoded libtest args, forwarded tool/property environment, `PWD`, and the `--chdir` argv remain unchanged. Dispatcher still creates a fresh `WorkDir` for every attempt; the new directories live in that attempt's fresh image stage. The operator and owner variants still invoke their existing `run_in_guest_cell` / `run_in_guest_owner_cell`; the owner callback and 60-second script-channel lifecycle are unchanged. No host tree copy, stale CWD fallback, or additional execution step is introduced.

### Typed errors, API, and oracle audit

Invalid or unmaterializable CWD directory input uses the existing `GuestKitError::Staging` cause; the dispatcher retains its existing `guest_execution_failed` mapping for image/staging failures. No new error variant, marker, status, CLI/config/environment/wire field, public method, or product API is added. `image_with_directories` is `pub(crate)` and the public `GuestKit::image` signature is unchanged.

The change does not edit selected identities, test bodies, Contract Shape declarations, host-script assertions, Nextest argument/environment/retry/status handling, result summaries, or error oracles. The new directory can only make the already-selected `--chdir` destination exist inside that guest image. It adds no test case or fake success path and leaves the owner cell's existing host-script completion requirement in place.

### Verification limits and remediation disposition

- The 092409Z raw capture reproduces the owner CWD failure against the prior `22320c8` implementation. It contains no post-`47328fb` native result, so it does not establish that the new directory materialization lets either selected body pass.
- The author reports canonical Lima Clippy, eight existing placement acceptance cases, scoped formatting and shell syntax checks passing. This reviewer did not rerun them.
- The operator cell failed in the reproduced pair but lacks a direct CWD diagnostic; the owner capture alone establishes the required directory input.
- No tests, native/Metal/SSH command, model check, source or test-body edit, or DES event was performed by this reviewer. The root is preparing the same two native selections at the corrected source.

### Verdict

**APPROVED — bounded CWD remediation in commit `47328fb88d21d5f3f57d25a935d1a17c505cbb9b`.** It materializes only the exact Nextest working directory inside the existing fresh guest image, preserving the selected ELF/argv/environment/helper and owner-host-script lifecycles without copying host files or changing public interfaces or outcome oracles. Post-fix native operator/owner execution and the full 01-02 acceptance gate remain **PENDING**.
