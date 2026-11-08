# Consolidated acceptance completeness review — metal-run-isolation

- Date: 2026-11-08.
- Role: acceptance-designer-reviewer; consolidated upstream and DISTILL structural review.
- Iteration: 1.
- Verdict: **NEEDS_REVISION** — one bounded assertion gap in the existing cleanup-complement scenario.
- Review boundary: local, read-only source and artifact inspection. Only this review artifact was written. No tests, compiler/lint/model checks, Metal commands, SSH, native execution, mutations, commits or agent dispatches were performed.

## Strengths and authority

The author has supplied complete executable bodies rather than names, placeholders or unconditional failures. The principal scenarios preserve the developer's actual decision: verify independent worktree changes together, retain each source-matched result, and leave the physical test kernel and the other invocation intact. The corpus distinguishes retained-record reader fixtures from actual producer failures, and the sole-model conformance driver holds and drives the actual shell/systemd owner.

Authority is the four-wave `feature-delta.md`, the single slice, `environments.yaml`, P1–P5, and DESIGN review iterations 2 and 3. The explicit **“Approve P1–P5”** at `feature-delta.md:547` supersedes historical proposed/pending labels. The codec correction is independently approved and does not reopen the protocol or add another oracle. No unresolved interface or semantic-error representation was discovered. This review does not reopen stopped service-feature work.

Loaded review criteria: `nw-ad-critique-dimensions`, `nw-at-completeness-check`, `nw-test-design-mandates`, `nw-bdd-methodology` and `nw-tdd-review-enforcement`, together with repository AGENTS/CLAUDE and applicable testing/development/design/verification rules. Repository Rust conventions and the explicit execution waiver override Python/Gherkin examples and execution defaults.

## Reviewed corpus and activation honesty

The placement handoff contains **54 new identities plus two existing transitions**:

| Corpus | Count | Examined source |
|---|---:|---|
| Rust acceptance and fixture-control cases | 36 | All cases in `xtask/tests/acceptance/metal_run_isolation/`: admission 6, collection 5, controller 2, finalization 2, identity 8, namespace controls 2, placement 10, qualification 1 |
| Owner conformance | 15 | All thirteen named `quint_test` bodies, one randomized driver, and one illegal-control case in `xtask/src/metal_run_isolation_conformance.rs` |
| Actual selected guest-input bodies | 3 | `infra/metal/tests/fixtures/attempt/tests/attempt.rs` |
| Existing qualification transitions | 2 | `xtask/src/main.rs:1776` and `:1791`, with full delegated bodies in `legacy_contract.rs` |
| External tooling E2E drivers | 3, outside Rust identity count | `worktree_journey.sh`, `placement_journey.sh`, `admission_journey.sh`, with `observe_host.py` |

All 56 identities have a source-local `CONTRACT_SHAPE` declaration. No new source-local pure-function Rust property requires the special pure-function rustdoc line. The 54 new descriptors comprise 45 host-pure, five native-launcher and four guest-group/core identities. Existing transitions retain their reviewed host-pure placement. The runtime-policy assertion compares the reviewed partition and explicit additions rather than a brittle fixed total (`placement.rs:328`). Test-only wiring is visible at `xtask/src/lib.rs:62`, the existing acceptance entrypoint, and the qualification module.

Pending markers contain the future implementation boundary; the selected guest inputs intentionally return 255, time out or reset their own identified guest and are interpreted only by their external driver. Existing test names are preserved for the qualification transitions. Their replacement bodies preserve contention, holder diagnostics, cancellation/reacquisition and explicit artifact reinjection under approved exclusive authority. No empty action, `todo!`, always-false configuration gate or fixture-supplied successful guest result was found.

`checks.md` distinguishes actual earlier compilation, the source-binding assertion RED and generated owner trace RED from infrastructure failure. The latter printed a reproducible Quint seed and stopped at the missing P1 binding after actual transfer; it is not evidence that later protocol states pass. The last permission/capacity fixture failure remains recorded, and its adjusted construction, final Clippy, refreshed three-body metadata, effective profiles and v2/native/E2E work remain **NOT RUN**. The user's **“STOP RUNNING TESTS THAT REQUIRE THE METAL LEASE”** waiver is honored. Those unrun checks do not cause this rejection; F1 can be addressed by local authoring alone.

## Accepted outcome and composed-failure coverage

| Outcome | Driving path, fault and authored oracle | Assessment |
|---|---|---|
| AC-1 ordinary overlap | Actual durable entry and real SH/EX descriptors in `admission.rs:39`; `independent_worktrees` trace; real paired worktree driver observes both selected command/VMM intervals | Authored at the required owner and external boundaries; not yet executed against v2 |
| AC-2 source attribution | Real Git/rsync/bootstrap binding assertions, equal-branch contrasting bytes, clone/sanitisation/override cases in `identity.rs`; full-bootstrap no-sync producer in `controller.rs:10` | Distinct valid and invalid partitions, original source/artifact/evidence complements, and actual producer refusal are covered |
| AC-3 checkout exclusion | Durable checkout descriptor survives controller loss; competing run/sync/pull; actual two-lock single budget in `admission.rs:220`; model same-checkout trace | Owner, competing source adapter and continued command are covered together without a driver-local lock claim replacing the kernel observation |
| AC-4 actual placement | Wrapper argv/env/status, refusal-before-body, no on-demand build; actual selected ELF/helper/retry/profile bodies and boot-ID guards; initial-namespace complement observer | Selected bodies and actual nextest result ownership are preserved; missing wrapper/dispatcher is not misreported as guest evidence |
| AC-5 scoped cleanup and surviving invocation | Real finalizer removes its three roots; real immutable-file failures; actual systemd stop/drain and VMM/process observations; live-peer boundary in worktree driver | **F1:** surviving peer's filesystem roots/files are observed or available but their preservation is not asserted across the other finalizer |
| AC-6 retained results | Legacy/current reader tables, actual controller no-sync/missing-rsync producers, controller EOF/loss, supervisor loss, actual command 255 versus nextest 100, repeat collection | Error kind/status, owner-stop gate, precedence and read-only retrieval are distinct authored obligations |
| AC-7 qualification/maintenance | Exact native prerequisite probes, real maintenance waiting/stop ordering, unadmitted waiter, selected native/nested/operator/owner bodies | Applicable execution boundary retained; no nested result is relabeled native, no new lane or expanded qualification campaign |

This satisfies the required cross-owner structure: faults enter actual control, transfer, qualification, kernel/FS or selected-guest boundaries; production owners author records and derived outcomes. `controller.rs` first establishes a healthy control, changes the actual source or transfer prerequisite, and follows the real bootstrap → transient owner → ABORT → finalizer → collector path. Namespace refusal fixtures remove capability, make only a private sysctl view read-only, exhaust private namespace capacity, or exhaust a real owned verification reader's descriptor budget. The ptrace fixture restores the real limit and checks actual EMFILE; it does not substitute a syscall return. Cleanup producers use real immutable files in their exact owned roots with scoped restoration. Guest peer loss is caused only after the actual guest boot identity differs from the observed host.

Retained record corruption is appropriately a reader precondition. It is not presented as proof that a production fault produced that record. These distinctions meet `.claude/rules/testing.md`'s cause-versus-consequence requirement. No new Sim seam, product API or failure model is requested.

## Sole-model owner conformance

The permanent manifest still has **18 checks**. The thirteen declared critical runs and thirteen `quint_test(test = ...)` names match exactly. The randomized driver uses the same in-place `runner_conf` spec, four samples and 24 steps (`metal_run_isolation_conformance.rs:1285`). Earlier captured failures print seeds; this review did not generate another trace or add a model family.

| Required driver property | Static evidence |
|---|---|
| Owning-crate placement and executable gate | Source under `xtask/src/`; `lib.rs:62` gates only on test plus integration-tests; reasoned ignore markers, no false cfg |
| Holds the real owner/composition | `RunnerDriver.owner: Option<World>` at `:858`; `World` contains real transient units, source checkout handles and actual collector returns at `:348` |
| Every model action mapped to its owner boundary | Exhaustive `switch!` at `:881` maps init, Register, Barrier, Checkout, Prepare, Start, LoseController, AdmissionTimeout, AbortPreparation, MainExit, SupervisorLost, StopOwner, Collect, Sync, NoSyncRefusal, Maintain and Idle. Handlers reach real shell/control/source/preflight/flock/systemd/collector boundaries. Idle rereads actual owner observations rather than repeating the preceding action |
| State projection uses the approved facts | `from_driver` at `:906`: metadata/phase plus actual manager liveness/MainPID and discovered fdinfo for phase; retained validated terminal/completion; binding/path equality; source marker and actual controlled source bytes; real maintainer descriptor/command/termination |
| No simulator or ghost ledger projection | No Sim adapter is used. Driver handles identify actual owners, not projected state. `ctl`, `checked`, `ever`, `starts`, `desc`, `roots` and historical `observed` are not projected. The current Collect action compares one actual `Output.status` with that action's model result, then clears the return before the next step (`:877`, `:1108`) |
| Single oracle and both trace modes | In-place permanent state machine referenced in all thirteen named declarations at `:1129` onward and the randomized declaration; no copied spec or CLI/REPL definition oracle |
| Actual resource complements independent of ghosts | Selected PID/cgroup validation at `:978`; systemd stop, actual remaining processes and exact-root absence at `:593`; broader VMM/root observer remains external. F1 concerns the *other live owner's* filesystem complement, not a request to project resource ghosts |

The external flock and finalizer-process inputs control real operation timing rather than return fabricated owner states. The short-wait stimulus is explicitly not evidence of the outer admission budget; the separately authored two-real-lock deadline case establishes that accepted distinction. Controller-at-ABORT replay and full-bootstrap producer tests observe different contracts and are both justified by the approved action table.

## Typed error mapping

The approved representation is the finite shell marker/record vocabulary at `feature-delta.md:421`; implementation is pending. No generic Rust envelope or diagnostic prose replaces that representation.

| Designed failure | Approved representation | Authored trigger/assertion |
|---|---|---|
| Invalid discovery, override or full binding | `checkout_identity_invalid`/64; `remote_subdir_invalid`/64; `checkout_identity_conflict`/78 | Identity/refusal tables; complete unchanged remote complement; tagged ABORT |
| Actual no-sync mismatch | `source_mismatch`/78, retained registered runner error | Healthy full-bootstrap control, actual dirty edit, no body, terminal/complete receipt and exact collect; model protocol counterpart |
| Maintenance/checkout budget | `maintenance_timeout` or `checkout_timeout`/75 | Real competing holders, exact error plus owner/source preservation; two-lock single-deadline body |
| Missing transfer prerequisite/preparation | `runner_preparation_failed`/75 with immediate `prerequisite=rsync` | Sync/pull no-registration/no-install; actual registered producer and collected retention; no-sync healthy control without rsync |
| Native/source/owner prerequisite | `preflight_failed`/78 plus exactly the eleven approved probe values | `qualification.rs:55` fault table, tagged owner protocol and collector mapping |
| Ordinary grammar/config/artifact/mode refusal | `unsupported_ordinary_command`, `execution_override_refused`, `artifact_scope_mismatch`/64; public mode parse 2 or bootstrap `invalid_mode`/64 | External exact argv tables; public/bootstrap mode cases |
| Bad context/placement/unprepared lane | `invalid_run_context`/78 and approved field; `placement_invalid`, `placement_ambiguous`, `test_unclassified`, `lane_not_prepared`/78 | Actual wrapper inputs; no body, no on-demand Cargo, unknown placement fails closed |
| Namespace operation | `namespace_setup_failed`/70, exact parent/child_mode/local/verify and errno | Real scoped permission/readonly/capacity/descriptor causes; post-adjustment execution honestly unrun |
| Guest image/VMM/peer failure | `guest_execution_failed`/70 plus original typed GuestKit cause; actual nextest aggregate | Boot-guarded real guest reset and expected original peer/VMM diagnostics in external placement driver |
| Cleanup failure | `cleanup_failed`/74 and exact guest_staging/vmm_runtime/command_tmp root | Actual immutable-file producer at each root, retained original command terminal and exact collected error |
| Artifact/ID/unknown/supervisor | `run_artifact_io`/74, `invalid_run_id`/64, `run_not_found`/66, `supervisor_lost`/70 | Closed schema/status tables, absent retrieval, conflicting records and missing-terminal precedence |
| Existing ABORT/EOF/launch/log failures and actual command result | Existing exact mappings; actual canonical command 0–255 | Unknown ABORT/terminal codes refused; legacy mappings retained; status-boundary tables and real 255-versus-100 inputs |

C6 is specified upstream. There is no `SPECIFICATION_AMBIGUITY`; the required remediation belongs to acceptance authoring.

## Taxonomy, mandates and boundary separation

| Checklist item | Result and evidence |
|---|---|
| C1a empty/minimum | PASS — unset/empty override control, empty placement, absent run |
| C1b boundaries | PASS — 23/24/25-character IDs, case, canonical and out-of-range command status tables |
| C2a documented state machine | PASS — acceptance module docstring and model phase 0–6 definitions |
| C2b illegal input per state | PASS — real-owner illegal-control case at `:1299`, absent/registered/shared/preparing/running/terminal/stopped boundaries |
| C3 zero/one/many | PASS — no/single/duplicate records/identities and one/two actual owner scenarios |
| C4a repeat mutation | PASS — sync twice, duplicate START, repeat finalization and collection; F1 separately limits the cleanup-complement oracle |
| C4b inverse without prerequisite | PASS — collect without run and repeat finalization over missing roots |
| C5a material modes | PASS — ordinary four no-sync/no-sudo combinations, exclusive/non-Metal passthrough, invalid mode, default/ci canaries |
| C5b orthogonality | PASS — mode authority, linked branch identity and host-pure lack of dispatcher dependency |
| C6a malformed values | PASS — identity/binding/context/placement/record/status input tables |
| C6b declared errors | PASS — producer versus reader mappings in the table above |
| C6c closed error set | PASS — unknown ABORT/terminal/placement never escapes as a valid result/context |
| C7a degraded resources | PASS — actual missing tools/inputs and bounded namespace/FS failures |
| C7b interruption | PASS — real control EOF/loss, owner kill/stop, actual timeout/reset inputs |
| C7c concurrent actors | PASS — two actual durable owners and external worktrees; F1 is the missing filesystem complement assertion within that existing scenario |

**Mechanical category-presence score: 15/15, COMPLETE.** This rubric establishes that every scenario class exists; it does not override an explicit accepted invariant whose asserted observable universe is incomplete. F1 therefore still blocks the acceptance handoff.

CM-A driving-port discipline, CM-B developer-domain vocabulary, and CM-C one value-bearing journey pass. Rust/Markdown GWT replaces `.feature` files; no consumer or framework is introduced. Finite real-I/O cases remain example tables. The approved sole-model conformance requirement supplies bounded randomized protocol traces; it is not a generated guest-boot matrix. Real integration/E2E assertions legitimately use direct snapshots/status/root/process comparisons; no private-field oracle or new generic state-delta framework is necessary. The missing survivor filesystem complement is the one exception requiring correction to the claimed bounded-change universe.

Rust cases exercise the shell tooling under test; they do not spawn the Overdrive product binary or emit product expectation evidence. External E2E scripts drive the actual developer commands and use external observations. Their Python helper observes/decodes actual facts and cleans only a driver-owned fixture; it does not implement the runner protocol. No point-in-time product expectation, benchmark, new CI lane or mutation suite is created.

Scoring: happy/error balance 9; GWT 9; domain vocabulary 9; coverage 6 pending F1; walking-skeleton value 9; priority 9; observable assertions 6 pending F1; traceability 9; walking-skeleton boundary 9. The readable twenty scenarios include eleven failure/negative scenarios (55%). E1–E9 and the existing non-Metal coexistence environments have explicit bindings; no environment Cartesian matrix is required. Roadmap-step sizing is not assessed because no executable roadmap is part of this handoff.

## Finding F1 — preserve the surviving invocation's actual filesystem complement

- Kind: **AT_GAP_IN_DELIVERY_SCOPE**.
- Severity: **HIGH; blocks AC-5 acceptance completeness**.
- Accepted obligation: `feature-delta.md:90–95,118` requires cleanup to release only this invocation's resources and preserve the other's images, temporary files, source/artifacts/evidence. P5 names exactly three owned roots; DEVOPS E8/K3 requires observing the same resource set on the surviving invocation.
- Evidence: `worktree_journey.sh:99–106` asserts the finished owner's root absence, the survivor's binding/source marker, and unchanged VMM PID/network-namespace identities. It never asserts preservation of `survivor['roots']`, although the observer already provides those facts. Later, `:132–138` requires both invocations' roots to be absent, so that final assertion cannot establish when the survivor's roots disappeared. `finalization.rs:52–80` protects a foreign command-tmp directory but supplies no foreign guest-staging or VMM-runtime root/file complement.
- Consequence for the authored oracle: removing another live invocation's guest-staging/runtime filesystem entries is outside the accepted delta, yet root disappearance alone does not violate the current live-peer assertions. A still-live VMM PID/netns is a different observation from preservation of its filesystem resources. The current checks therefore cannot support the claimed zero cleanup of another invocation's filesystem resources.
- Required bounded authoring correction: strengthen the **existing** finalization case with fresh fixture-owned foreign staging/runtime roots and file sentinels, and compare their unchanged contents across finalization; retain scoped cleanup for those inputs. Strengthen the **existing** external live-peer boundary using its already-captured survivor root observations, so the other finalizer cannot remove the live owner's roots unnoticed. Preserve the source/terminal/build/command-tmp/VMM checks already present. Update the completeness disposition to describe the actual asserted universe.
- Scope/cost: existing cases and observer data only; no new test identity, API, port, error, owner, model action/check, simulation seam, guest selection, lane or native campaign. No execution is required while the user's waiver applies.

This is a statically established missing assertion against an already-approved criterion, **not** a claim that the current production finalizer has a reproduced deletion defect. Production implementation of P5 is pending. No hypothetical failure has been promoted into an architectural or production remediation requirement.

## Equal-weight subtractive review

The correction above should strengthen two existing bodies; adding another guest journey or new cleanup subsystem would be unnecessary. No new test family is requested.

- Reader corruption tables and actual producer tests are not duplicates: one checks interpretation, the other proves actual owner cause/retention. Keep that separation.
- Thirteen thin named conformance declarations are required critical-run bindings, not thirteen independent fixture implementations. Their shared driver plus four bounded random traces reuses the approved model; no additional speculative formal checks are warranted.
- The two real namespace fixture-control cases verify the honesty of the otherwise subtle readonly/capacity/descriptor stimulus. They are test support and grant no native boot credit. A generic fault-injection service or production hook would add surface without an accepted need.
- `Fixture`, `Durable`, `NativeController`, the owner replay fixture and existing GuestKit-backed paths share preparation at their respective boundaries. Private durable tests, full-bootstrap controller tests and external developer journeys do not need to be collapsed into a single substitute owner.
- Runtime placement comparison enforces the approved exact partition and explicit additions. It does not mirror an incidental module layout or inflate a fixed expected test count.
- The standalone selected-body/helper fixture is necessary to observe actual ELF/helper/attempt forwarding and fresh retry identity. Default/ci canaries cover mandatory adapter inheritance without creating another lane or another broad profile matrix.
- Existing 18 formal checks, retained native construction inputs and the three bounded external scripts remain sufficient scope. No extra model family, Sim seam, benchmark, schedule, mutation work or repeated whole-suite campaign is justified by this review.

No demonstrably redundant case requires deletion. No generic fixture abstraction or wholesale test rewrite is requested. The one mandatory change is directly anchored in AC-5 and uses observations/fixtures already required by that accepted outcome.

## Dispositions and verdict

The author's previous conformance, actual fault-cause and full-controller gap closures are structurally supported by the examined bodies. Existing qualification transitions are fully authored and explicitly pending. Their post-adjustment execution remains unclaimed under the user's waiver.

| Finding | Disposition |
|---|---|
| F1 — live peer filesystem cleanup complement | OPEN; return to the acceptance author for the bounded existing-body assertion correction above |
| Interface/error representation gap | None; no DESIGN return requested |
| Additional production defect or architecture change | None alleged or authorized |
| Unrun post-adjustment/native checks | Explicitly disclosed; not a rejection reason or request to acquire the Metal lease |

**NEEDS_REVISION.** Complete the F1 authoring correction and return for local/static re-review. Preserve all pending runtime evidence limits, the approved P1–P5 contract, the stopped service feature and unrelated dirty work. This review requires no additional Metal execution.

## Iteration 2 — F1 assertion remediation

Date: 2026-11-08. **Current verdict: APPROVED.** This bounded re-review covers F1 only and supersedes iteration 1's open disposition and NEEDS_REVISION verdict. The historical review above is retained. No other contract, test family or architecture decision was reopened.

### Examined changes and independent observable evidence

| Required correction | Actual authored evidence | Disposition |
|---|---|---|
| Protect the other invocation's three filesystem roots/files across finalization | `finalization.rs:52–69` selects a different fresh valid invocation ID, refuses to adopt existing paths, creates guest-staging, VMM-runtime and command-tmp roots, and writes a sentinel file under each. Snapshots read the actual filesystem before the real `--finalize` call. `:90–101`, inside the existing two-finalization loop, independently requires every foreign root to remain an actual directory and its file contents to equal the pre-call snapshot. These assertions do not infer preservation from a receipt, PID or model ghost. | **CLOSED** |
| Observe the live owner's roots across the peer's actual completion | `worktree_journey.sh:103` compares the survivor's complete three-root observation with its overlap observation at the existing one-completed/one-live boundary. `observe_host.py:121–130` obtains these facts using actual `lstat` on guest-staging, VMM-runtime and recorded-checkout command-tmp paths; it includes existence, UID, GID, mode and symlink status. Existing source/binding and unchanged VMM PID/netns assertions remain alongside the root comparison. | **CLOSED** |
| Keep the correction bounded and truthfully documented | The same finalization test and external journey were strengthened. `OwnedRoots` retains scoped restoration for the separately created fixture roots. Source/build/terminal and own-root cleanup assertions remain intact. `completeness.md:36` describes the new actual asserted universe and explicitly claims no runtime pass. No new identity, production API/error/owner, model action/check, guest selection, execution lane or native campaign was added. | **CLOSED** |

The fixture roots and sentinel bytes are valid preconditions for testing finalizer effects; they are not pre-created successful output. The system under test remains the real declared finalizer entry. The external root observations are independent manager/filesystem observations for the actual live invocation. Together these close the specific oracle gap: removal or modification of the other invocation's fixture files is asserted directly, and disappearance or changed ownership/type of the actual survivor roots is no longer invisible at the completion boundary.

The test keeps its existing Contract Shape declaration and pending activation marker. The correction strengthens the approved bounded-change complement without weakening or expanding AC-5. The equal-weight subtractive assessment is satisfied: existing bodies and existing observation data were reused; no additional scenario or generalized fixture mechanism is necessary.

### Verification limits and final disposition

This iteration used local source/artifact reads only and appended this review. No tests, native commands, compiler/lint/model runs, Metal lease acquisition, remote/SSH execution, mutation tests or commits were performed. Author-reported local formatting/syntax/whitespace results were not rerun. All post-adjustment and v2/native/E2E execution limits recorded in `checks.md` remain **NOT RUN** under the user's explicit waiver. This approval establishes authored acceptance completeness, not implemented behavior or a successful delivery capture.

| Item | Current disposition |
|---|---|
| F1 — live peer filesystem cleanup complement | **CLOSED — corrected by the acceptance author and independently reviewed** |
| Remaining blocking/non-blocking acceptance findings | **None** |
| Specification ambiguity / production or architecture remediation request | **None** |
| Mechanical category coverage | **15/15 — COMPLETE**, with F1's complement assertion gap now closed |
| Previously affected dimension scores | Coverage 9; observable assertions 9. Other iteration-1 scores and mandate conclusions remain unchanged |

**APPROVED** for consolidated acceptance completeness/structural handoff. The original findings and remediation history remain auditable. Runtime acceptance retains the approved P1–P5 invariants and the disclosed unrun evidence boundary; this review requests no further Metal execution.
