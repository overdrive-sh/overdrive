# Consolidated DISTILL review — requirements and Definition of Ready

Date: 2026-11-08. Reviewer: `nw-product-owner-reviewer` perspective, GPT-6.1 Sol / xhigh. Iteration 1. **Verdict: APPROVED.** Blocking findings: zero. Required remediation: none.

This verdict approves the requirements-to-acceptance-authoring handoff for `metal-run-isolation`. It does not claim implemented v2 behavior, successful native E2E execution, completed dogfood, or approval of a future roadmap. The original service-feature DELIVER remains stopped.

## Authority, method and execution boundary

Read the current `AGENTS.md`, applicable `CLAUDE.md` and development/testing/design/verification/domain-naming rules; the installed `nw-product-owner-reviewer` role and `nw-por-review-criteria`, `nw-dor-validation`, `nw-po-review-dimensions`, and `nw-review` skills. The task-specific native Markdown artifact requirement supersedes the role's default chat-only YAML output. This review changes only this artifact.

Reviewed the DISCUSS, DESIGN, DEVOPS and DISTILL sections of [the authoritative feature delta](../feature-delta.md), [the single slice](../slices/slice-01-concurrent-worktree-verification.md), [environment cases](../environments.yaml), the persona/job/journey SSOTs, [readable scenarios](test-scenarios.md), [completeness audit](completeness.md), [actual check dispositions](checks.md), and [explicit placement additions](test-placement-additions.json). Examined the corresponding identity, admission, controller, collection, finalization, placement, qualification and transition acceptance bodies, the owner-conformance bindings, and all three external tooling drivers for outcome and scope traceability. This is the requirements perspective; independent test and architecture reviews own their detailed executable and contract scrutiny.

P1–P5 are user-approved: the delta's `User approval — 2026-11-08` section quotes **“Approve P1–P5”**. [DESIGN review](../design/review.md) iterations 2 and 3 are APPROVED. Earlier proposal/pending language is historical and does not revoke the later authoritative approval. No new decision or specification ambiguity is identified by this requirements review.

The user explicitly directed **“STOP RUNNING TESTS THAT REQUIRE THE METAL LEASE”**. Review used local reads only. No Metal command, remote observation, direct-SSH fallback, test execution, model run, mutation run or lease request was launched. The last retained Clippy pass and seeded semantic RED are historical evidence with the limits stated in `checks.md:52`. The permission-fixture adjustment, final Clippy, refreshed third-body metadata, effective profile views and native/v2 E2E work remain **NOT RUN** where recorded. Their omission is the explicit pre-implementation waiver, not a fabricated pass or an automatic rejection of this authoring handoff.

## User value, journey and slice gates

**praise:** the feature keeps one concrete developer decision throughout: Marcus can continue or revise each independent worktree change from its own source-matched result. The elevator pitch names the real `cargo xtask metal run -- …` command and observable run-ID, guest-group and completion output; it does not substitute an internal function or a passing test for user value (`feature-delta.md:45–60`).

| Gate | Evidence and assessment |
|---|---|
| One job and traceable story | `US-METAL-ISOLATION` has `job_id: J-PLAT-006` and must-have priority. The actual job register entry describes concurrent independent verification and the continue/revise decision. No infrastructure-only exemption is used. |
| Elevator pitch | Before / After / Decision enabled are present. The After names the existing developer command and exact observable marker shapes; the decision is independent acceptance or revision of each source change. PASS. |
| Journey coherence | Choose source → prepare → invoke → observe → collect/decide is complete in the delta story map and `docs/product/journeys/verify-independent-worktrees.yaml`. Source/preparation refusal, controller loss, retained runner failure and maintenance contention have explicit branches rather than an assumed all-success flow. PASS. |
| Emotional arc | The persona and journey name frustration at unrelated serialization, focused observation of separate invocations, and confidence in an attributable result. These correspond to the actual stages and failure diagnostics; no additional UI or dashboard is implied. PASS. |
| Shared artifacts | `run_id` comes from durable registration; source identity comes from the worktree snapshot/source marker; command status comes from the supervisor terminal. Their consumers are identified. P1 and P5 pin bindings, metadata and completion ownership instead of introducing another source of truth. PASS. |
| Example consistency | `ouagadougou`/`san-antonio`, equal branches with different dirty contents, sanitisation collisions and actual status 255 exercise distinct risks. The target comes from existing configuration, and no host address or credential becomes a requirement. PASS. |
| One value-bearing release slice | The slice names the same story/job and requires AC-1–AC-7 together. Identity-only and lease-removal-only releases cannot claim the outcome. No technical-only slice is introduced. PASS. |

## Definition of Ready

All eight role checks pass for the single approved story. The additional numeric KPI check in the delta also passes at the requirements level; measurements remain pending implementation verification.

| DoR item | Result | Evidence |
|---|---|---|
| 1. Clear, grounded problem | PASS | “Marcus waits behind another worktree's ordinary verification” and cannot rely on branch names to separate remote state (`feature-delta.md:51`). The premise table cites existing runner/source behavior and explicitly calls this a requested capability, not a newly reproduced product defect. |
| 2. Specific persona | PASS | Marcus is the Overdrive engineer using multiple Git worktrees and the shared Metal box, with exact-results/native-evidence responsibilities. The dedicated persona identifies role, context and motivations. |
| 3. At least three concrete domain examples | PASS | The three examples at `feature-delta.md:56` cover the existing workspace pair, equal/colliding branch identities, and disconnected-controller collection including 255. |
| 4. Happy, edge and failure UAT | PASS | Seven GIVEN/WHEN/THEN scenarios cover overlap, identity, source exclusion, physical test-kernel isolation, resource ownership, controller loss and evidence/maintenance boundaries (`feature-delta.md:62`). |
| 5. Derived, checkable acceptance criteria | PASS | AC-1–AC-7 map in order to those seven scenarios, with checkbox completion criteria and explicit zero-mixing/zero-forbidden-delta invariants. Observable overlap and exact retained results are testable without a promised speedup. |
| 6. One demonstrable, estimated outcome | PASS | One release slice, seven UAT scenarios and the six-hour/one-day planning estimate. The estimate explicitly cannot truncate invariant-preserving work. DISTILL's twenty detailed cases refine the seven accepted UAT scenarios; they are not twenty new stories or independent release outcomes. |
| 7. Constraints and technical notes | PASS | D1–D5, P1–P5, the slice IN/OUT boundaries, direct-command applicability, exact command preservation, exclusive maintenance and finite host-launch allow-set are explicit. |
| 8. Dependencies resolved or tracked | PASS | P1–P5 approval and independent DESIGN approval resolve the earlier design questions. Prepared dispatcher/artifacts, qualified Metal and capacity for two groups are named prerequisites. Remaining implementation verification is truthfully separated from completed authoring under the user's waiver. No unspecified API or error representation is delegated to a crafter. |

No implement-X story, generic persona/data, absent examples, happy-path-only UAT, tests-after-code promise or independently shipped infrastructure story was found. Technical nouns in this developer-tooling domain identify the user's actual commands and evidence boundaries; treating them as consumer-UI jargon would remove necessary precision. The accepted placement, record and lifetime choices are DESIGN contracts, not newly invented requirements.

## AC-1–AC-7 preservation through DESIGN and authored tests

| Accepted outcome | Exact downstream commitment | Examined authored evidence |
|---|---|---|
| AC-1: independent ordinary execution overlaps | P2 uses SH maintenance admission plus each checkout's EX lock. K1 still requires two overlapping actual command intervals and zero unrelated ordinary EX waits. | `admission.rs:39` checks actual overlapping owners/descriptor modes after controller loss; named owner conformance includes `independent_worktrees`; `worktree_journey.sh:32` drives both actual prepared worktrees and checks live ordinary owners, SH locks and guest VMMs. |
| AC-2: exact worktree/source/artifact/evidence attribution | P1 pins full Git identity, checkout-qualified ordinary-clone branch fallback, canonical binding, override refusal and non-destructive cold migration. | `identity.rs:40` supplies equal-branch/different-source inputs and preserves the first checkout's artifact/evidence complement. Branch switch/detachment, clone/sanitisation collisions, foreign overrides and pull-after-local-edit have complete separate bodies. E1/E6 retain the same obligation. |
| AC-3: same-checkout exclusion survives controller loss | The durable service owns the checkout lock before transfer through selected-command execution; one budget covers admission and checkout waiting. Another checkout may proceed. | `admission.rs:39`, `:86`, `:178` and `:220` cover durable lock ownership, distinct timeout causes, sync/pull exclusion and one real deadline. Owner conformance includes `same_checkout_excludes_sync`; the actual external journey checks each canonical checkout lock. |
| AC-4: ordinary test kernel changes stay in fresh guest groups | P3's finite host-launch allow-set remains separate from forbidden physical test-kernel effects. P4 makes direct nextest placement mandatory; pure/Sim bodies require no guest. | `placement.rs:91`/`:129` preserve pure and non-Metal chains; refusal/namespace cases fail before body. `placement_journey.sh` selects actual guest ELF/helper/retry/profile bodies, and `worktree_journey.sh` compares the physical complement. Existing core batching remains the paired journey. |
| AC-5: resource and cleanup ownership stay disjoint | Local-mode namespace references own CID/listener isolation; P5 ends descendants before exactly three run-root removals and completion. No global allocator or recovery service is introduced. | The paired journey checks disjoint live namespace ownership, a completed/surviving owner boundary and completed-root/process absence. `finalization.rs:38` preserves source/target/terminal/foreign-root complements; `:95` produces each declared cleanup-root failure. Owner conformance covers cleanup and maintenance ordering. |
| AC-6: durable logs/statuses and read-only repeat collection | Actual command 0–255 remains separate from runner/test errors. v1 remains compatible; v2 waits for stopped owner plus approved completion, preserving supervisor/artifact precedence. | `collection.rs:13`, `:49`, `:77`, `:146`, `:219`; full-controller source/preparation producers in `controller.rs`; controller-loss and repeat-collection model bindings; paired/placement drivers retain actual status 255 versus nextest aggregate 100. |
| AC-7: native/nested qualification and exclusive maintenance keep their meanings | P4 retains direct physical native launcher proof, named nested correctness cells, exact existing operator/owner contexts and fail-closed missing prerequisites. Shell/provisioning/explicit `--exclusive` retain maintenance authority. | `placement_journey.sh` selects the existing exact native, nested, operator and owner bodies; `qualification.rs:55` checks each declared probe. `admission_journey.sh` observes real owner stop/root cleanup before maintenance command start and an unadmitted ordinary waiter. No result is relabeled with broader credit. |

This mapping checks the declared outcomes and actual body intent; neither the 54 new identities plus two transitions nor the author's 15/15 score establishes runtime correctness. The placement additions preserve explicit classification rather than absence-means-host behavior. Pending markers are activation boundaries over authored bodies, not delivered success claims.

## Typed-error requirement traceability

The approved interface representation is the finite shell marker/record contract at `feature-delta.md:421`; implementation is pending. Authored oracles retain that representation rather than replacing semantic causes with diagnostic prose.

| Designed semantic failure | Approved representation | Authored requirement oracle |
|---|---|---|
| Invalid discovery/override/full binding | `checkout_identity_invalid`/64; `remote_subdir_invalid`/64; `checkout_identity_conflict`/78 | Identity refusal tables and before/after foreign complements; registered tagged ABORT cases. |
| No-sync source/binding mismatch | `source_mismatch`/78, retained if registered | Healthy control followed by actual full-bootstrap dirty-source refusal in `controller.rs:10`, plus conformance at the declared ABORT boundary. |
| Maintenance versus own-checkout deadline | `maintenance_timeout`/75 versus `checkout_timeout`/75 | Actual contending locks, unchanged held source/owner, single-budget case and owner conformance. |
| Source-transfer/context preparation | `runner_preparation_failed`/75; missing transfer tool has immediate `prerequisite=rsync` | Sync/pull no-record/no-install checks and full registered producer in `controller.rs:34`; no-sync healthy control does not require unused rsync. |
| Native/source/owner prerequisite | `preflight_failed`/78 plus the exact approved `probe` enum | Qualification table; tagged ABORT/reader retention and named preflight conformance. These negative fixtures claim no native boot credit. |
| Unsupported ordinary command, replaced execution, foreign artifacts, invalid mode | `unsupported_ordinary_command`, `execution_override_refused`, `artifact_scope_mismatch` /64; Clap mode error/2 or bootstrap `invalid_mode`/64 | External placement refusal cases and both existing mode-entrypoint checks. No automatic exclusive fallback. |
| Bad context, placement or missing guest preparation | `invalid_run_context`/78 with approved `field`; `placement_invalid`, `placement_ambiguous`, `test_unclassified`, `lane_not_prepared` /78 | Placement tables assert the exact code/payload and no body/on-demand build. |
| Namespace setup operation | `namespace_setup_failed`/70 with `step=parent|child_mode|local|verify` and errno | Scoped real operation-failure producers and verification-reader fixture, with execution limits retained in `checks.md`. |
| Image/VMM/peer failure | `guest_execution_failed`/70 plus original `GuestKitError` cause; actual nextest aggregate preserved | External boot-guarded guest-reset selection checks the adapter marker/cause instead of treating a wrapper refusal as the guest failure. Not executed at this handoff. |
| Cleanup, corrupt records, unknown invocation or missing supervisor terminal | `cleanup_failed`/74 with exact root; `run_artifact_io`/74; `invalid_run_id`/64; `run_not_found`/66; `supervisor_lost`/70 | Real root-failure producers plus separate retained-reader tables check exact root, closed fields/status domains, precedence, unchanged terminal and repeat read-only retrieval. |
| Existing ABORT/EOF/launch/log failure versus a command result | Existing approved mappings; command remains actual canonical 0–255 | Before-START controls, reader compatibility tables and actual 255/100 external inputs. Unknown causes do not escape as command success. |

No `SPECIFICATION_AMBIGUITY` is found at the requirements handoff. This table is not a claim that the future implementation already supplies these mappings.

## Equal-weight subtractive assessment

| Candidate addition or duplication | Accepted obligation and disposition |
|---|---|
| More user stories, infrastructure release slices or a redesigned operator interface | None required. The single story already closes choose → prepare → invoke → observe → decide. Adding stories solely to mirror P1–P5 would fragment one outcome. No addition requested. |
| A scheduler, CID registry, outer guest per invocation, persistent pool, recovery/scavenger or new telemetry | None authorized. P2/P3/P5 reuse existing durable owner, kernel namespace and unit finalizer; model ghosts are excluded from telemetry. No addition or expansion requested. |
| Twenty detailed scenarios / 54 new test identities | They refine the seven UAT scenarios across explicitly accepted identity, admission, placement, typed-failure and completion contracts. Finite reader/argv/identity tables avoid multiplying guest boots for each spelling/status. The count alone provides neither justification nor rejection; no unanchored user outcome was found in the reviewed bindings. |
| Component reader checks plus producer/controller/owner/E2E checks | These observe different promises: accepted record interpretation, actual cause selection, real manager/descriptor ownership, and the actual developer journey. A manufactured retained record cannot prove its producer; a raw fixture cannot prove actual worktree concurrency. Those layers are necessary under the approved scope and are not interchangeable duplicate evidence. |
| Thirteen named conformance runs plus bounded randomized/illegal-control coverage | The thirteen are the accepted named critical scenarios against the sole permanent model. Randomized traces and illegal-control cases exercise that same contract without adding protocol actions or runtime interfaces. No second model, new Sim seam or extra model-check family is demanded. |
| Additional native campaigns, profile matrices or repeated KPI measurements | Not required. E1–E9 are selected dimensions, not a Cartesian matrix; the paired capture supplies K1–K3 together. Default/ci compatibility and existing named native/nested/operator/owner selections are bounded accepted behavior. No benchmark, fleet, OS matrix, new lane, recurring job or mutation work is requested. The current user waiver forbids further pre-implementation Metal execution. |

The original service-feature files and unrelated dirty work were not edited or absorbed into this review. No new production mechanism, public API, CLI flag, error variant, lifecycle state, weakened zero-state invariant or additional acceptance outcome is requested.

## Findings, disposition and verdict

- **Blocking findings:** none.
- **Non-blocking remediation requests:** none.
- **praise:** the check record preserves both the real semantic RED and the unrelated permission-fixture failure, and leaves post-adjustment execution explicitly NOT RUN. That distinction supports an honest authoring handoff without claiming that the native outcome has been delivered.

**APPROVED** for consolidated DISTILL requirements/DoR review. One value-bearing story and all AC-1–AC-7 remain intact through accepted P1–P5 and the authored acceptance corpus. No further native execution is requested by this review; eventual implementation acceptance retains the approved source, isolation, resource, exact-result and evidence-applicability invariants.
