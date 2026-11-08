# Metal run isolation — consolidated platform and verification review

## Metadata and authority

- Date: 2026-11-08; iteration 1.
- Role: `nw-platform-architect-reviewer`; platform/DEVOPS/verification perspective of consolidated DISTILL review.
- Verdict: **NEEDS_REVISION** — one bounded external-driver oracle correction.
- Authority: P1–P5 in `feature-delta.md`, with the explicit approval at line 549; independent DESIGN review iterations 2 and 3. Historical proposal labels do not override the recorded user approval.
- Reviewed: all four waves of `feature-delta.md`, `environments.yaml`, DISTILL scenario/check/completeness/placement artifacts, the three external journeys and observer, their standalone attempt/transport fixtures, Rust acceptance support and owner-conformance composition, and the relevant existing Metal entry/collection paths.
- Instructions: current `AGENTS.md`, `CLAUDE.md`, applicable design/development/testing/verification/naming rules; installed reviewer definition and `nw-par-critique-dimensions`, `nw-par-review-criteria`, `nw-review-output-format` skills. Native Markdown follows the repository requirement over the skill's YAML default. Generic service-platform checklists are applied only where relevant to this developer tool.
- Changes: this review artifact only. No production, test, model or configuration edit; no commit, agent dispatch or service-feature DELIVER action.

## Verification boundary

This is a local, read-only artifact review. **No test, build, lease acquisition, SSH command, native check or Metal execution was run.** The user's **“STOP RUNNING TESTS THAT REQUIRE THE METAL LEASE”** instruction and explicit pre-implementation waiver remain effective. The finding below requires a local authoring correction; it does not require a deferred native run to close this review.

`checks.md:60` retains the scoped Linux Clippy pass for its captured source. `checks.md:61` separately retains the intended missing-binding RED at seed `0x6d789d91`, the passing verification-read fixture and the unexpected child-mode/capacity fixture permission failure. The subsequent permission adjustment, final Clippy, refreshed three-body fixture listing, effective profile views and full v2/native journeys are correctly **NOT RUN** (`checks.md:62–66`). Neither the 15/15 authoring self-audit nor this review converts those results into implementation success.

## Finding P-F1 — selected-command diagnostics are asserted on the controller's stderr

**Severity:** high; blocks acceptance-authoring approval. **Owner:** DISTILL external-driver author. **Location:** `infra/metal/tests/placement_journey.sh:60`, `:72`, `:73`. **Affected contract:** preserved combined command log and exact nextest/test-adapter failure evidence, P2/P4/P5 and AC-6/AC-7.

`run_case` correctly captures the developer command's stdout and stderr separately (`placement_journey.sh:21–26`), but the timeout assertion searches `timeout.stderr` for nextest's `TIMEOUT`. The guest-execution assertions likewise search `guest-execution-failure.stderr` for the test-adapter marker and original peer/VMM diagnostic. These are diagnostics of the **selected command**, whose stdout and stderr the durable supervisor deliberately combines. They are not controller/collector diagnostics.

The concrete current production path is:

1. `cargo xtask metal run` → `MetalAction::Run` → `sh_status` → `bootstrap.sh`, preserving the bootstrap's two streams (`xtask/src/main.rs:987–1005`, `:1033–1036`).
2. The durable supervisor launches the selected command with **both** streams redirected to its output FIFO (`infra/metal/lease-holder.sh:636–657`). Its logger writes that stream to the retained log (`:627–628`); successful log finalization preserves it (`:691–723`).
3. The bootstrap finishes by calling its actual collector (`infra/metal/bootstrap.sh:672–677`). `terminal_result` emits the combined retained log through `emit_available`/`dd` on **stdout**, while the completion marker is explicitly on **stderr** (`infra/metal/lease-holder.sh:265–276`, `:279–301`). The same stream promise is documented at `infra/metal/README.md:61–65` and retained by the approved DESIGN contract.

Thus, for the intended real timeout or guest-peer-loss case with nextest status 100, the assertions inspect a stream that does not carry the required evidence. The timeout assertion can stop the driver before the authored guest failure, profile, native/nested/cell and final complement checks are reached. The stopped owner's cleanup remains the existing systemd/finalizer responsibility; no new cancellation or recovery mechanism follows from this finding.

This is a contradiction between an authored oracle and the existing, explicitly preserved output contract, not a newly alleged production defect or a hypothetical future schedule. Retained native timeout/recollection evidence also contains the actual nextest timeout diagnostics (`spike/native-increment-o.log:517`, `spike/native-increment-o-recollect.log:492`); it is construction evidence rather than an execution of this future driver. **The future v2 driver has not been run, and no such reproduction is claimed.**

**Minimal disposition:** change these three selected-command assertions to read their corresponding `.stdout` files, or the already-collected combined command log. Keep run-ID, immediate runner-error and completion-marker checks on `.stderr`. Preserve their exact existing marker/cause/status oracles. This requires no new test identity, framework, production output change, contract amendment or native campaign. Return the corrected artifact for local re-review; update the self-audit's disposition without claiming a new runtime pass.

**Disposition:** OPEN. No remediation was performed by this reviewer.

## Environment and outcome coverage

The environmental cases name existing substrate states, not a new matrix or deployment platform. Subject to P-F1, the authored coverage has the required platform boundaries:

| Outcome | Environment and actual observable | Assessment and evidence limit |
|---|---|---|
| AC-1 / K1 | E2/E4/E7: two ordinary invocation IDs, actual SH barrier and checkout WRITE descriptors, live selected commands/VMMs in different namespaces | `admission.rs:39`, owner-conformance composition and `worktree_journey.sh:30–80` use actual owner/process observations. Paired v2 execution remains pending; no speedup or unlimited-capacity claim. |
| AC-2 / K2 | E1/E6: full binding, source bytes/digest, cold remote paths, target and evidence complements | `identity.rs:40–263` covers equal-branch dirty worktrees, linked branch changes/detachment, clone fallback, colliding spellings, overrides and dirty-local pull. Fake SSH is limited to transport; real Git/rsync/files establish the local adapter behavior. |
| AC-3 | E2/E4: service-held checkout exclusion through preparation, command and controller loss; competing sync/pull | `admission.rs:39`, `:86`, `:178`, `:220` distinguish real lock holders and one admission budget. Full-bootstrap controller tests separately drive the producer's actual refusal decision. |
| AC-4 / K3 | E3/E8/E9: explicit placement, guest boot identity, current selected ELF/helper/attempt, physical initial-namespace complement | `placement.rs:91–430` distinguishes adapter/context refusals from execution. Standalone attempt bodies and `placement_journey.sh` require guest boot IDs different from the observed physical host before timeout/reset effects. Native success is never inferred from qualification substitutions. P-F1 affects the external failure-evidence path. |
| AC-5 / K3 | E5/E7/E8: actual owner stop, exact three-root lstat absence, VMM/process complement and surviving invocation | `finalization.rs:38`, `:95` address owned roots, repeat cleanup and actual deletion refusal. `observe_host.py:51` observes systemd/cgroup and exact-root launchers; `worktree_journey.sh` checks the surviving source and held VMM namespace. Supported timeout/peer-loss execution remains unrun. |
| AC-6 / K2 | E3/E5/E6: actual producer ABORT, exact command/runner terminal, read-only repeat collection and completion precedence | `controller.rs`, `collection.rs:13–239` and the external drivers separate producer tests from seeded reader inputs. Legacy v1 requires no receipt; v2 requires owner stop/completion. Command 255, nextest aggregate 100 and runner failures retain distinct meanings. P-F1 must be corrected before external evidence can be evaluated. |
| AC-7 | E3/E4/E9: qualified physical native launcher, named nested/operator/owner contexts and explicit maintenance exclusion | `qualification.rs:55` covers the declared probe partition without native credit. `placement_journey.sh:78–81` selects the existing exact native/nested/operator/owner bodies; `admission_journey.sh` observes real stopped predecessors and an unadmitted waiter. No nested-to-native substitution or automatic exclusive fallback is requested. |

K1–K3 reuse one paired capture and its read-only snapshots. The additional focused nonzero/255/retry/error cases are deterministic tooling checks; they do not establish throughput, latency distributions or success rates. Evidence collection remains pending. `environments.yaml:201–220` correctly limits native applicability and finite capacity, and excludes a new fleet, OS matrix or staging/production service.

## Typed failure and compatibility audit

No new semantic error representation is needed for this handoff. The exact approved sets remain the authority; the reviewed coverage does not license generic diagnostic strings in their place.

| Designed failure | Approved representation | Authored boundary |
|---|---|---|
| Invalid Git identity / override / foreign binding | `checkout_identity_invalid`/64, `remote_subdir_invalid`/64, `checkout_identity_conflict`/78 | Identity adapter cases, with full foreign-source/evidence complement |
| No-sync mismatch / missing source-transfer rsync | `source_mismatch`/78; `runner_preparation_failed prerequisite=rsync`/75 immediately and retained preparation cause when registered | Full-bootstrap `controller.rs` producer paths plus focused sync/pull cases; no ordinary package installation |
| Maintenance versus own-checkout deadline | `maintenance_timeout` versus `checkout_timeout`, both 75 | Real flock cases, explicit single-budget scenario, real unit conformance/maintenance journey |
| Native prerequisite failure | `preflight_failed probe=<declared probe>`/78 | All eleven declared probes in `qualification.rs`; fake healthy inputs carry no native evidence |
| Placement/context/absent dispatcher | Declared `invalid_run_context`, `placement_invalid`, `placement_ambiguous`, `test_unclassified`, `lane_not_prepared`/78 | Wrapper driving port and finite schema/context fixtures; no on-demand build or host fallback |
| Namespace operation / actual guest execution failure | `namespace_setup_failed step=<parent|child_mode|local|verify> errno=<approved form>`/70; `guest_execution_failed`/70 with original cause | Scoped real namespace/resource faults and boot-guarded actual guest reset; the latter's external diagnostic oracle needs P-F1 |
| Root cleanup / malformed completion / lost supervisor | `cleanup_failed root=<guest_staging|vmm_runtime|command_tmp>`/74; `run_artifact_io`/74; `supervisor_lost`/70 | Actual finalizer deletion refusal plus independent retained-record reader tests; missing terminal precedence and legacy collection retained |
| Actual command versus nextest attempt result | Actual command 0–255; body 255 remains failed attempt/aggregate 100 | Reader-domain tables and standalone selected-body/helper/retry fixtures; no numeric-status-only runner classification |

The test-placement additions explicitly account for the 54 new identities and two existing qualification transitions. The original inventory is compared as a partition, without a brittle total-count assertion. The selected profile canary reuses `host-kernel-shared`; its effective default/ci settings remain unverified under the waiver. Ordinary/exclusive authority and privileged/login identity remain separate. Old exclusive callers and retained v1 records are explicit compatibility cases rather than an incomplete-v2 fallback.

## Deployment, hooks and evidence classification

The DEVOPS rollout and rollback at `feature-delta.md:599–601` fit source-consumed developer tooling: prepare both updated worktrees, preserve retained records/bindings/caches, finish invocation owners before changing captured finalizer code, and retain a v2 reader for v2 records. No service publication, daemon or schema-migration framework is required. The DEVOPS coexistence matrix and `environments.yaml:176` keep existing Git/SSH/rsync, nextest profiles, guest groups, Lefthook/Linux routing, Lima and CI ownership. Removal of ordinary hook-driven host module loading is an implementation obligation; this DISTILL handoff does not claim it is implemented.

The three scripts are explicitly developer-tool E2E/conformance tests, not `verification/expectations` runners. Their command subprocesses are the actual developer-tool SUT; no Rust test spawns the Overdrive product binary and no expectation imports production crates or invokes the Rust harness. The one delivery receipt records a bounded outcome and its source/substrate; it is not a recurring benchmark, confidence-rate campaign or new EDD regression gate. Model/native construction evidence is accurately kept distinct from implemented-path outcome evidence.

## Equal-weight subtractive audit

- The scope uses the existing systemd owner, kernel namespace lifetime, guest groups, nextest capacity group, retained records and collector. No allocator, registry, broker, persistent guest pool, recovery daemon, telemetry service or new platform is justified or requested.
- Thirteen named conformance cases and one bounded randomized case reuse the sole approved model; the separate illegal-control case addresses its state-boundary obligation. No model fork, redundant hazard/witness family or formal campaign is added by this review.
- Retained-reader fixtures and full-bootstrap producer tests answer different questions. Keeping their roles separate avoids treating fabricated terminal data as proof that the real controller selected the correct error.
- Namespace trace/resource fixtures and exact immutable-file cleanup inputs remain private test support. Their limits and the failed permission control are disclosed. No production pause seam or syscall-result replacement is introduced. No new fixture or validation expansion is requested here.
- Shared support and the small standalone Cargo-reported helper fixture avoid a general execution framework. Finite input tables avoid multiplying native boots for identity/status/record spellings. Native/nested/operator/owner checks reuse existing bodies.
- P-F1 is corrected by using the existing evidence stream. Adding an output multiplexing protocol, another capture helper, a production stderr tee, additional checks or another Metal campaign would be unnecessary.
- Mutation testing remains disabled by the explicit user instruction. Generic DORA dashboards, SLO paging, SBOM/IAM/canary machinery and performance-percentile gates from the role template do not apply to this bounded change and are not findings.

## Verdict and required disposition

**NEEDS_REVISION.** Correct P-F1 in the existing external driver and return it for local re-review. There is no upstream architecture/error-contract blocker identified by this platform review, and no authorization to expand P1–P5 or resume the stopped service feature. Deferred Metal/native/final permission-fixture checks remain **NOT RUN** under the user's waiver; they are not additional conditions for this authoring review.

## Iteration 2 — P-F1 remediation review

Date: 2026-11-08. **Current verdict: APPROVED.** This bounded re-review supersedes the iteration-1 open finding and verdict above, which remain preserved as history. Scope is P-F1 only; no new findings or scope expansion.

| Finding | Disposition | Evidence |
|---|---|---|
| P-F1 — selected-command diagnostic stream | **CLOSED** | `placement_journey.sh:60` now reads `timeout.stdout`; `:72–73` read `guest-execution-failure.stdout`. The exact `TIMEOUT`, `guest_execution_failed` and original peer/VMM diagnostic patterns are retained, as are expected status 100, body-entry checks, collection checks and resource-complement oracles. Run IDs still come from controller stderr at `:61` and the existing corresponding paths. |

The corrected assertions now match the unchanged production stream boundary: selected-command `2>&1` enters the durable FIFO/log (`lease-holder.sh:636–657`), and the collector emits that combined log on stdout while keeping its completion marker on stderr (`:265–301`). `completeness.md:35` records the bounded authoring correction and explicitly preserves the absence of runtime validation. No production output change, new test, helper, gate or model mechanism was introduced to close the finding; the minimal existing-path correction is sufficient.

Verification for this re-review was local source inspection only. The author's reported local syntax/whitespace checks are not represented as tests run by this reviewer. No tests, Metal lease, SSH, remote/native execution or production changes were performed. The permission-fixture adjustment and all other deferred final/native checks remain **NOT RUN** under the user's waiver.

**Final platform/DEVOPS/verification verdict: APPROVED for the DISTILL authoring handoff.** Zero open findings remain from this review. This does not claim implemented v2 behavior, successful native E2E, completed K1–K3 capture, or authorization to resume the stopped service feature.
