# Metal run isolation roadmap review

- Review ID: `metal-run-isolation-roadmap-20261108-1`.
- Reviewer: Codex, fresh independent roadmap reviewer applying the acceptance-designer and solution-architect roadmap criteria.
- Date: 2026-11-08.
- Iteration: 1.
- Artifact: [roadmap.json](roadmap.json).
- Verdict: **NEEDS_REVISION**. One bounded acceptance-activation inconsistency remains; roadmap validation stays **pending**.

## Review basis and scope

Reviewed the current `AGENTS.md`, `CLAUDE.md`, applicable development, design, testing, verification and naming rules; the installed `nw-review`, `nw-roadmap-review-checks`, architecture and acceptance review criteria; both roadmap steps; the approved P1–P5 contracts and delivery notes in [feature-delta.md](../feature-delta.md); [environments.yaml](../environments.yaml); the one release [slice](../slices/slice-01-concurrent-worktree-verification.md); the four consolidated DISTILL review dispositions; authored activation markers, test wiring, placement additions and the three external journeys. Canonical Lima routing and the selected source/collector fixture paths were inspected locally.

The requirements, architecture, platform and acceptance DISTILL reviews are all currently **APPROVED**. Their historical findings remain closed. This review does not reopen those design decisions, allege a new production defect, or require a waived pre-implementation native run. It checks whether the new activation plan can use the existing acceptance artifacts as written.

## Findings

### R1 — the final placement driver excludes a test that the roadmap activates

**issue (blocking): HIGH — acceptance activation and final verification disagree.**

The integrated step requires all assigned bodies and all three external drivers to pass (`roadmap.json:45`). Its activation notes remove every remaining pending feature marker (`feature-delta.md:751`), explicitly including `placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain` (`:760`). That case currently has exactly the pending attribute at `xtask/tests/acceptance/metal_run_isolation/placement.rs:128`; removing it makes the case an ordinary, non-ignored test.

The existing final native entry point `infra/metal/tests/placement_journey.sh` then reaches the four supported `--no-sync`/`--no-sudo` combinations at lines 113–118. Each invocation at line 117 selects that same test while requesting `--run-ignored only`, and the enclosing `run_case` requires status zero. The retained nextest help states that `only` runs ignored tests (`spike/nextest-run-help.txt:189–195`); its default empty-selection behavior fails with status 4 (`:293–300`). After the prescribed activation, this selector therefore executes none of the intended body. Default empty-selection handling fails the driver; an environment permitting an empty selection would still provide no coexistence evidence.

This is a deterministic disagreement between the planned test metadata and the driver selector, established by the exact current source and the retained tool semantics. It is **not** a claim of a reproduced runner, systemd, concurrency or cleanup defect. No runtime execution was performed or requested to establish this artifact inconsistency. A production fix, new API, new fixture, new test identity, new lane, new model check or native rerun is unnecessary.

**Required bounded disposition:** return the existing external-driver selector to DISTILL for correction so that these four commands execute the now-live case after integrated activation. Removing the ignored-only restriction from this one existing command is sufficient; retain the body, assertions, four flag combinations, exact intended selection and status requirement. The deliberate 255/timeout/reset/profile inputs must keep their separate ignores and ignored-only invocations. Do not leave the coexistence case ignored, add a replacement canary, relax the expected result, or authorize the crafter to rewrite acceptance bodies. Re-review the corrected artifact locally; no Metal command is required for this authoring correction.

### Positive assessment

**praise:** the dependency cut preserves the accepted isolation boundary. Source attribution can become correct while execution remains exclusive; shared ordinary admission cannot be released until placement, invocation namespaces, source ownership and completed-owner cleanup are integrated. The explicit distinction between Lima construction evidence and the final updated-runner native proof also preserves the user's lease instruction without claiming that compilation proves concurrency.

## Mandatory roadmap checks

| Check | Result | Evidence and assessment |
|---|---|---|
| External validity | **FAIL, bounded by R1** | The real invocation path and integrated wiring exist: existing `metal run/sync/pull/collect`, mandatory nextest adapter, actual systemd owners and all three native external journeys. However, the mandatory placement journey cannot execute its named coexistence case after the plan's activation. Correct R1 without another integration step. |
| Acceptance-criterion implementation coupling | **PASS** | All nine criteria describe source attribution, exclusion, execution placement, typed failure/result behavior and resource complements. References to P1–P5, native resources, ELF/argv and three roots preserve already-approved interface/ownership contracts. They do not prescribe a new private decomposition or invent an interface. |
| Step decomposition ratio | **PASS** | Two steps over 18 unique production/tooling/configuration paths after excluding test-only bodies, documentation and the conformance driver: `2 / 18 = 0.111`. No repeated substitution steps. The integrated step's 43 pending activations are an informational sizing signal, not justification for unsafe horizontal commits. |
| Implementation code in roadmap | **PASS** | No method body, algorithm or pseudocode. The authoritative delivery notes provide commands and contract references, not crafter implementation logic. |
| Concision and precision | **PASS** | At review entry, all JSON string values total 482 whitespace-separated words, below the 500-word ceiling. The phase description has 40 words; notes have 71/85; criterion counts are 4/5 and individual lengths are 19–25 words. Metadata added by this review is administrative. |
| Unit and acceptance boundaries | **PASS, execution selection subject to R1** | Fixtures drive the copied real shell entry points; native owners use actual systemd; external journeys drive actual developer commands and observe kernel/process/root facts. No Rust test becomes an Overdrive binary expectation runner. Rust unit properties retain the approved proptest/state-delta rule; missing authored coverage returns to DISTILL. No new domain-entity or private-method test is requested. |

## Exact activation and dependency accounting

Local source inspection found **50** current `pending DELIVER: metal-run-isolation …` attributes. Every associated function name is explicitly present in the ROADMAP activation notes, and each has exactly one planned owner:

| Current marker location | Source step `01-01` | Integrated step `01-02` |
|---|---:|---:|
| `identity.rs` | 7 | 1: explicit maintenance mode |
| `admission.rs` | 0 | 6 |
| `collection.rs` | 0 | 5 |
| `controller.rs` | 0 | 2 |
| `finalization.rs` | 0 | 2 |
| `placement.rs` | 0 | 9 |
| `qualification.rs` | 0 | 1 |
| `xtask/src/metal_run_isolation_conformance.rs` | 0 | 15 |
| `xtask/src/main.rs` qualification transitions | 0 | 2 |
| **Total** | **7** | **43** |

The graph has one dependency, `01-02 → 01-01`, with no cycle. The two roadmap `test_file` and `scenario_name` locators resolve to complete pre-authored cases in the Linux-gated `xtask::acceptance` entry point. All 50 pending names are accounted for; no broad ignore removal is permitted.

[Placement additions](../distill/test-placement-additions.json) contain **54 new identities plus two existing transitions**, matching the **56 owned identities**. The remaining six beyond the 50 pending cases are two already-live namespace fixture controls and four deliberately ignored external inputs. The live controls remain live. The profile canary and three standalone attempt bodies remain deliberately ignored and are selected only by their external drivers. R1 concerns a different, pending case that is expressly scheduled to become live.

The source step retains existing exclusive command authority and does not claim an independently complete concurrent feature release. The integrated step combines P2–P5 within the one release slice. Necessary compiler, configuration, hook, renderer or consumer fallout is permitted by the stated guidance rule; omission from a file list is not a reason to add a compatibility API or stop required work. Existing service-feature changes and all other dirty work remain outside this review's edits.

## RED, GREEN and native evidence boundaries

The canonical Lima commands in `feature-delta.md:796–810` are real existing entry points. `LimaAction::Run` accepts the trailing argv (`xtask/src/main.rs:396–403`), and `lima()` executes it inside the existing VM (`:795–879`). The acceptance entry point is gated by `integration-tests` and Linux, rather than a not-yet-created runner API.

- **Source step:** the exact identity body calls the copied current bootstrap with real Git and rsync. `Fixture::environment` assigns temporary maintenance/tree paths (`support.rs:270–282`); the SSH input executes local Bash, not the physical Metal route. The body reads actual canonical bindings and sibling source/artifact/evidence complements. The previously captured missing-binding RED is disclosed in DISTILL; the new crafter must record its own RED, then execute all seven assigned cases in Lima. No native evidence is credited to this fixture.
- **Integrated step:** the exact collector body uses complete retained-v1/v2 records as reader preconditions and invokes the current `lease-holder.sh --collect` (`support.rs:311–319`). It does not require a missing wrapper or guest to reach its semantic RED. Supported flock/controller/source/qualification/wrapper and owner-conformance cases can then run through canonical Lima. Actual controller fixtures keep real systemd while substituting only the declared external substrate/transport inputs. The notes explicitly leave unavailable namespace, immutable-filesystem and native prerequisites pending rather than accepting skips or substitute results.
- **Final proof:** the integrated GREEN still requires actual remaining native/owner cases and the existing `worktree_journey.sh`, `admission_journey.sh` and `placement_journey.sh` through the updated ordinary protocol (`feature-delta.md:822–836`). Both callers must be updated. Existing exclusive ownership is respected. One paired capture supplies K1–K3; successful compilation, listing, old evidence or finite fixtures cannot replace observed overlap, source/result attribution, forbidden-kernel zero deltas, completed-run zero residue and survivor preservation. R1 must be corrected before this last driver can supply its intended evidence.

No pre-implementation Metal check, old exclusive-runner campaign, direct SSH test bypass, mutation run or additional native matrix is introduced. Read-only SSH observations inside the already-authored final external drivers are distinct from bypassing the canonical test execution boundary. The user's waived checks remain **NOT RUN**. Missing final actual native proof prevents a completion claim; it does not make a waived native campaign a prerequisite for starting the authorized implementation.

## Interface and typed-error contract assessment

The roadmap references the approved exact P1–P5 shape; it adds no public/cross-crate API, flag, environment field, protocol value, record field or error representation. Its error gate correctly routes a genuinely missing semantic set or representation to DESIGN as `SPECIFICATION_AMBIGUITY`, and keeps private implementation details with the crafter.

| Semantic contract carried into implementation | Approved representation and ownership |
|---|---|
| Discovery, override and full binding; no-sync source mismatch | `checkout_identity_invalid`/64, `remote_subdir_invalid`/64, `checkout_identity_conflict`/78, `source_mismatch`/78; exact registered tagged ABORT retention where specified |
| Maintenance/checkout budget and preparation/native prerequisite | `maintenance_timeout`/75, `checkout_timeout`/75, `runner_preparation_failed`/75 with immediate rsync context, `preflight_failed`/78 with its finite probe set |
| Ordinary command/config/artifact/mode refusal | `unsupported_ordinary_command`, `execution_override_refused`, `artifact_scope_mismatch`/64; public Clap mode error/2 versus bootstrap `invalid_mode`/64 |
| Context, placement, namespace and guest execution | Approved test markers, finite field/step/errno payloads, exact 78/70 statuses and original typed GuestKit cause; no raw-host or on-demand-build fallback |
| Cleanup, corrupt/absent retained data and result interpretation | `cleanup_failed`/74 with one exact owned root; `run_artifact_io`/74; existing ID/not-found/supervisor mappings; original command terminal and canonical 0–255 result retained, with nextest owning its aggregate |

The detailed payload, retention and precedence table remains authoritative at `feature-delta.md:421–449`. No error-contract ambiguity was found in this roadmap. Implementation reviewers must verify each implemented mapping; this review does not claim those behaviors already exist or pass.

## Equal-weight subtractive assessment

- Keep the two steps. Splitting admission, namespace, placement and completion into independent exposed cuts would permit ordinary execution before its accepted isolation guarantees. Adding a transition flag or temporary host fallback would create unauthorized surface.
- Keep the existing retained-reader, producer/controller, real-owner and external journey layers. They observe different approved promises; a fabricated terminal cannot prove its producer and a fixture transport cannot prove native concurrency.
- Reuse the closed placement inventory, its explicit additions, sole permanent model, thirteen named runs, four bounded randomized traces and existing illegal-control case. No scheduler, CID registry, persistent pool, recovery mechanism, new model family or telemetry is required.
- Reuse one paired K1–K3 capture and the existing three drivers. No repeated whole-suite run per identity/error row, benchmark, density campaign or profile matrix is warranted.
- Close R1 by changing the existing selection restriction. A new canary, new test, preserved pending ignore or changed expected outcome would add or weaken scope unnecessarily. This is an acceptance-author correction, not additional production work.

No other removal or mechanism expansion is requested. The estimate is expressly provisional and cannot cap invariant-preserving work. Scope guidance does not permit silently committing stopped-feature or unrelated changes.

## Verification, dispositions and verdict

This review used local source reads and JSON/text accounting only: 50 pending markers mapped once, 7/43 assignment, 54 additions plus two transitions, real test locators, acyclic two-step graph, word limits and production-file ratio. No tests, Cargo compilation/linting, Lima execution, model evaluation, Metal lease acquisition, remote command, SSH command, mutation testing, DES event or commit was performed. The author's prior runtime results and all later **NOT RUN** limits remain unchanged.

| Finding or gate | Disposition |
|---|---|
| R1: ignored-only selector for the scheduled-live coexistence case | **OPEN — blocking bounded DISTILL authoring correction** |
| P1–P5 approval and four consolidated review decisions | Preserved; no DESIGN reopening |
| Exact activation ownership and semantic error specification | Complete; no additional interface or test identity requested |
| Canonical Lima starting path | Structurally valid; crafters must run their own actual RED/GREEN |
| Final implemented concurrent-runner/native proof | Required and pending implementation; not run in this review |
| Roadmap validation | **pending**; actual reviewer, timestamp and artifact pointer recorded in validation metadata |

**NEEDS_REVISION.** Correct the existing final-driver selector through the acceptance author, preserve the deliberate external ignores and all assertions, and return for a local re-review. No additional Metal check is a condition of this review.

## Iteration 2 — R1 selection correction

Date: 2026-11-08. Reviewer: Codex, the same independent roadmap reviewer. **Current verdict: APPROVED.** This bounded re-review covers R1 only and supersedes iteration 1's open disposition and NEEDS_REVISION verdict. The complete original review remains above as history; no other scope or design decision was reopened.

### Evidence and remediation disposition

| Required correction | Examined current artifact | Disposition |
|---|---|---|
| Execute the coexistence case after integrated activation | `infra/metal/tests/placement_journey.sh:117` now uses ordinary nextest selection, with no `--run-ignored only`. The existing case filter is unchanged and resolves to the case that `feature-delta.md:760` schedules to become live. | **R1 CLOSED** |
| Preserve the four modes and original oracle | The loop at `:113` still contains exactly the empty, `--no-sync`, `--no-sudo` and combined flag inputs. The line at `:117` still requires status zero through the existing `run_case`; no empty-selection allowance, new case, replacement canary or weaker assertion was added. | **Preserved** |
| Retain deliberately ignored external selections | The status-255/helper/retry input at `:46`, timeout input at `:58`, guest-reset input at `:70` and default/ci profile canary at `:75` still use their original ignored-only selection. Their existing expected 100 results and body/resource assertions remain present. | **Preserved** |
| Record the bounded authoring correction honestly | `distill/completeness.md:37` records the selector correction and retained modes/oracles; it explicitly claims no runtime execution. | **Complete** |

After the prescribed marker removal, normal nextest selection includes the intended coexistence body rather than excluding it as a non-ignored test. This closes the exact activation/verification mismatch. The equal-weight subtractive requirement is satisfied: one existing selection restriction was removed; no test identity, fixture, API, protocol, model check, execution lane, native campaign or implementation mechanism was added.

### Verification limits and final verdict

This re-review used local source inspection and administrative roadmap validation only. No test, compiler/lint/model execution, Lima command, Metal lease acquisition, remote/SSH command, mutation test, DES event or commit was performed. The author's reported shell-syntax/static checks are not represented as runtime tests run by this reviewer. All previously waived post-adjustment/native checks remain **NOT RUN**.

Local administrative checks passed: `des-roadmap validate` returned 0 (`VALID: 1 phases, 2 steps`); `des-verify-integrity … --roadmap-only` returned 0 with no errors and correctly skipped execution-log checks. Both used the required `PYTHONPATH=/Users/marcus/.claude/lib/python`. JSON/reference/selector checks and scoped whitespace validation also passed.

The original review's passing activation accounting, dependency cut, interface/error contracts, canonical Lima starting paths and subtractive assessment remain valid. The external-validity failure was solely R1 and is now **PASS**. There are **zero remaining blocking or non-blocking remediation requests** from this roadmap review.

**APPROVED for implementation sequencing.** `roadmap.json` now records `validation.status = approved`, the actual reviewer, approval/review timestamp and this artifact pointer. This approval does not claim implemented behavior, successful native acceptance or completed K1–K3 capture. The final actual updated-runner native/owner cases, all three external drivers and every accepted zero-delta invariant remain required before completion.
