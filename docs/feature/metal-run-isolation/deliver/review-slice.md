# Final slice review — metal-run-isolation

## Metadata and current disposition

- Reviewer: Codex, GPT-6.1 Sol, independently dispatched for the user's one final full-slice review.
- Date: 2026-11-09.
- Preparation baseline: `0e8ae265c2d80d98c345b342a396057940837e3a`.
- Implementation: `ee0bbd65805524afa4a6348b5019ce12a7390e8b`, `2ba2232208ce9a5e91666f61b6b9845d9797c62a`, and the bounded corrections through `47328fb88d21d5f3f57d25a935d1a17c505cbb9b`.
- Source-audit HEAD: `8914e5ee47da37689919399382b9304d75d18a21`; later commits to that point retain evidence without changing the reviewed runtime implementation.
- Final bounded runtime correction: `e21d4fcb5c0bd96e42c9adaf90e1f8ec741f50b7` plus `060359bc1cf88c78252e9d789396821413b41d7e`; final bootstrap SHA-256 `47e7152b18ef5d7b57a26d288f473769d70258d8086477bdb563cd9c17e2f625`.
- Final evidence receipt: `d4d011b4fdd8e2954d5cf90f4da261702d88576f`, after the frozen-source four-mode evidence in `f8c0f02d224887f2cd7ef67c3f0547950e7b71d7`.
- Current verdict: **APPROVED — full-slice implementation and consolidated feature-specific native evidence.** FSL-1 and FSL-2 are **CLOSED**. Final physical/process/root complements pass their original unchanged assertions. No open reproduced feature finding or outstanding feature-specific native check remains within this review. The one pre-existing burst-body exclusion and precise source/boundary qualifications below remain explicit; no all-owner GREEN or one-SHA uninterrupted driver result is claimed.
- This artifact is the one full-slice review, completed through the recorded bounded remediation and final receipt assessments. Earlier pending/rejected dispositions remain in its history; no separate full review cycle was substituted.

## Authority, scope, and method

The authoritative contracts are the user-approved P1–P5 in [feature-delta.md](../feature-delta.md), accepted ADRs [0277](../../../product/architecture/adr-0277-metal-checkout-identity-is-owned-by-the-local-git-worktree.md), [0278](../../../product/architecture/adr-0278-metal-ordinary-runs-share-the-maintenance-barrier.md), [0279](../../../product/architecture/adr-0279-metal-guest-resources-belong-to-the-durable-invocation.md), [0280](../../../product/architecture/adr-0280-metal-nextest-execution-uses-explicit-test-placement.md), [0281](../../../product/architecture/adr-0281-metal-run-completion-follows-owned-resource-cleanup.md), and the approved [two-step roadmap](roadmap.json). The feature's AC-1–AC-7 and K1–K3 retain their original zero-mixing, zero-forbidden-kernel-delta, zero-completed-owner-residue, and zero-survivor-change outcomes.

The review read the current applicable `AGENTS.md`, relevant development, testing and verification rules, the installed `nw-software-crafter-reviewer` role, and its `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology` skills. The repository/user requirements control artifact format, model selection, scope, reachability, independent reproduction, and unlimited same-step remediation; the role's legacy Haiku, YAML-only and iteration-cap defaults do not override them.

The reviewer independently read the feature-owned implementation and baseline-to-final diffs, rather than adopting the earlier step verdict as proof. The earlier [01-01 review](review-01-01.md) and [01-02 history](review-01-02.md), including iterations 1–27, supplied historical reproductions and validation attribution. They did not replace this source audit. No production code, test body, design, roadmap or DES phase event was edited by this reviewer. No native campaign, direct physical-host test command, new model or mutation run was started.

Only this review artifact is owned by the reviewer. The seven original dirty paths remain outside the review's write/commit scope: `AGENTS.md`; the service feature's `exit_observer.rs`, `crash_ending_authority.rs`, `terminal_hook_ownership.rs`, and core `driver.rs`; and that feature's roadmap and untracked execution log.

## Full implementation and interface audit

| Contract | Current production entry and owner path | Independent assessment |
|---|---|---|
| P1 — canonical worktree/clone identity | `xtask/src/main.rs:987` dispatches the existing Metal command to `infra/metal/bootstrap.sh`. The bootstrap discovers physical checkout/Git/common directories at `:373–441`, hashes the approved domain/kind/identity bytes, and preserves linked-worktree identity across branch changes. Clone fallback requires the attached full ref. | Matches the approved owner and exact identity encoding; no branch-name-only or digest-only reuse authority was added. |
| P1 — full binding, transfer, pull and cold migration | `bootstrap.sh:491–556` compares the complete ordered binding under checkout exclusion. `:632–676` resolves the remote physical home/path and directory-hash lock; `:937–1050` handles named pull, exclusion-preserving rsync, standalone Git and atomic source marker publication. `:1060` checks the no-sync marker. | Bind only an empty/new directory; preserve legacy directories, target output and sibling checkouts. Pull authorizes the binding without requiring the caller's current dirty digest. Collection remains keyed by target/run ID. |
| P2 — service-owned admission and checkout exclusion | `bootstrap.sh:768–833` captures the helper and launches the same transient unit with the original fifteen plus exactly three approved positional values. `lease-holder.sh:800–1008` writes v2 registration and acquires SH/EX maintenance followed by EX checkout using one deadline. `:1011–1085` publishes token-matched invocation/tree and, only for exclusive mode, singleton owner records. | Matches the approved lock order, authority and metadata ownership. Ordinary runs do not publish an exclusive singleton owner or release/reacquire checkout exclusion around START. |
| P2 — queued EOF, control grammar and detachment | `lease-holder.sh:403–478` queues one real flock waiter and observes the existing control pipe; EOF cancels and reaps that waiter. `:1091–1134` accepts one START or the approved ABORT grammar and detaches inherited controller descriptors before the command-running promise. `:1188–1257` forwards supported supervisor termination and retains the actual command status. | The bounded queued-EOF correction preserves the original control vocabulary, one deadline and command owner. Exact seed and before-/after-START controls have native evidence; no replay, detached cleanup task or public cancellation surface was introduced. |
| P2 — exclusive maintenance after ordinary owner stop | `lease-holder.sh:480–526` reads real admitted v2 ordinary unit liveness; durable EX and the existing shell/provisioning holder call it before authorizing maintenance. Waiting unadmitted ordinary units are excluded by their retained phase. | Current source implements the approved admitted-owner stop gate. The external E4 proof is explicitly a controlled prior-owner fixture with public EX/waiter bootstraps on one fixture-private barrier; it is not relabeled as full default-barrier E4 evidence. |
| P3 — local vsock namespace and finite host resources | `test-wrapper.sh:145–198` creates the anonymous parent, writes only its child mode, creates the child and verifies `local` before any native/guest body. `guest_lane.rs:514–535,635–644` uses the beacon from the same held `HostListeners`. `guest_kit.rs:423–441,615–625` and `vm_fixture.rs:263–274` select the approved invocation roots. | CID constants, bands and signatures remain unchanged. No userspace allocator, guessed free CID, preclaim holder, named namespace or whole-invocation outer guest was added. Existing VMM confinement remains the production adapter. |
| P3 — traversal permissions and cleanup universe | `bootstrap.sh:1080–1099` prepares staging/runtime and command-tmp inputs before START. The two native fixture constructors give their supplied run parent group traversal before `Vmm::create`; existing assertions remain. | The staging/runtime traversal and VMM confinement contracts are preserved. FSL-2 is closed by the bounded common-parent/cache correction and corrected private argument positions; all four existing modes pass at frozen source `060359bc`. The final external complement receipt is independently approved below. |
| P4 — finite ordinary/exclusive interface | `bootstrap.sh:95–235` parses the exact approved Cargo/env grammar; `:237–266` confines manifest/target arguments; `:305–341` rejects invalid mode and execution replacements. `xtask/src/main.rs:430–443` adds only `Run.exclusive`. | Exactly one new operator flag, run-only `--exclusive`; no implicit escalation or arbitrary shell interpretation. Explicit suite runs receive the local namespace wrapper; unsupported forms refuse before START. |
| P4 — closed placement and existing profiles | `.config/nextest.toml:1016–1022` supplies the inherited default wrapper with `target-runner=within-wrapper`. `test-wrapper.sh:58–143` validates the entire static policy, duplicate keys and the exact selected identity; `:203–247` selects host/native/group/lane execution. | Non-Metal/exclusive preserve the original exec chain. Host-pure needs no dispatcher. Unknown/malformed/ambiguous placement refuses before body execution. Existing `ci`/`mutants` profile inheritance remains intact. |
| P4 — actual attempt inputs and nextest result owner | `fwd_guest_suite.rs:3145–3226` captures approved environment, actual executable and `NEXTEST_BIN_EXE_*` helpers. `:3451–3759` reuses the existing group context, exact CWD, argv and peer status. `guest_lane.rs:250–493` stages the actual selected lane ELF/helpers/env/CWD; `fwd_guest_suite.rs:4014–4124` validates the closed lane fixture mapping and invokes that selected cell once with the authored host script. | The prepared dispatcher stays checkout-owned; no on-demand wrapper Cargo build, stale selected ELF substitution, batch-result interpreter, pool or new scheduler was added. Nextest retains retries, attempt metadata and aggregate 100; directly supervised command 255 remains 255. |
| P4 — exact existing preparation and bounded native prerequisites | `xtask/src/guest_kit_prepare.rs:271–309` prepares the static init and host dispatcher through the existing preparation owner. The nested group adds only the reproduced scheduler-module, XFS ordering and ldd/Bash runtime inputs. | The recorded native failures justify these consumer-required inputs. Existing explicit suite batching, module loading inside guests and named nested/native claim boundaries remain distinct. |
| P5 — stop-owned finalization | `bootstrap.sh:811–829` embeds this invocation's helper bytes in the same unit's `ExecStopPost`. `lease-holder.sh:229–303` validates retained v2 metadata, removes only the exact three invocation roots and atomically records complete or the exact failing root. | No collector cleanup, global glob, target deletion, source deletion, recovery controller or pruning protocol. Normal fixture Drop/terminate remains first owner; the finalizer follows the existing systemd stop boundary. |
| P5 — read-only v1/v2 result collection | `lease-holder.sh:640–798` enforces the matching metadata filename/format, retained phase, v2 stopped-unit and completion gates. `:568–638` retains actual command/runner result mapping. | A command terminal cannot imply cleanup completion. Missing terminal remains supervisor loss; malformed/missing completion is artifact failure; cleanup failure retains the command terminal. Repeat collection does not write source, records or owned roots. |

No product public Rust method, type, variant, trait, wire format, product state or product CLI was introduced. The crate-private image-directory helper and test-local callback/argv plumbing are internal structure/test support, which the accepted contract leaves to implementation/DISTILL. The new shell v2 metadata and completion formats, private finalizer entry, eighteen-value launch, and dispatcher entry match their pinned public/cross-component shapes.

## Typed semantic error mapping

The approved interface uses finite shell fields; OS/tool diagnostic strings provide context and do not replace those fields. The reviewer checked the producer, retained representation and collector/nextest owner for each designed semantic failure.

| Designed failure | Approved and implemented representation | Current owner path |
|---|---|---|
| Invalid Git identity / detached clone | `checkout_identity_invalid`, 64 | Bootstrap discovery before remote mutation. |
| Invalid remote override | `remote_subdir_invalid`, 64 | Local override grammar and remote path resolution. |
| Unbound nonempty / malformed / foreign full binding | `checkout_identity_conflict`, 78; tagged ABORT and same retained runner error when registered | Binding checker → existing control pipe → durable terminal → collector. |
| No-sync binding or source mismatch | `source_mismatch`, 78; same tagged/retained cause | Binding/source marker checker → durable ABORT → collector. |
| Maintenance/predecessor deadline | `maintenance_timeout`, 75 | One durable deadline; retained terminal and collection; legacy holder immediate marker. |
| Checkout deadline | `checkout_timeout`, 75 | Remaining admission budget at checkout; same retained cause. |
| Filesystem/context/source-transfer preparation | `runner_preparation_failed`, 75; immediate `prerequisite=rsync` when applicable | Bootstrap preparation or durable preparation producer; no ordinary package install; registered ABORT/finalization/collector retains 75. |
| Native prerequisite/source/owner | `preflight_failed`, 78, immediate approved `probe` | Existing native preflight → tagged ABORT → retained cause. No qualification credit from negative fixture inputs. |
| Unsupported ordinary command/env | `unsupported_ordinary_command`, 64 | Finite argv/env parser before START. |
| Config/debugger/tracer/runner replacement | `execution_override_refused`, 64 | Ordinary execution guard. |
| Archive/build reuse or external artifact scope | `artifact_scope_mismatch`, 64 | Ordinary provenance guard. FSL-1 concerns a driver expecting the wrong marker for this correctly implemented branch. |
| Invalid exclusive mode | Public Clap parse error 2; bootstrap `invalid_mode`, 64 | Existing public parser / direct bootstrap guard, before registration. |
| Missing/invalid ordinary context | `OVERDRIVE_METAL_TEST_ERROR error=invalid_run_context field=<run_id|execution_class|source_owner>`, 78 | Wrapper before selected body. |
| Invalid / duplicate / absent placement | Test `placement_invalid` / `placement_ambiguous` / `test_unclassified`, 78 | Closed JSON parser and exact identity lookup. |
| Namespace operation failure | Test `namespace_setup_failed step=<parent|child_mode|local|verify> errno=<positive decimal|none>`, 70 | Actual namespace/sysctl/read operation, before body; no global fallback. |
| Missing dispatcher/guest input | Test `lane_not_prepared`, 78 | Wrapper or dispatcher producer; existing `GuestKitError::LaneNotPrepared` preserved. |
| Existing image/VMM/peer failure | Test `guest_execution_failed`, 70, with original `GuestKitError` diagnostic cause | Existing guest adapter; nextest owns resulting aggregate status. |
| Owned-root removal failure | Runner `cleanup_failed root=<guest_staging|vmm_runtime|command_tmp>`, 74 | Finalizer receipt → stopped-v2 collector; actual terminal retained. |
| Corrupt/missing retained artifact | Existing `run_artifact_io`, 74 | Ordered finite validators; no result reconstruction. |
| Invalid ID / unknown run / missing command terminal | Existing `invalid_run_id` / `run_not_found` / `supervisor_lost`, 64/66/70 | Existing collection boundary, including supervisor loss after successful cleanup. |
| Existing ABORT/EOF/launch/log failures | Existing exact retained causes and prior collection mappings, 75/74 | Durable supervisor and legacy collector rules. |
| Actual selected command or body | Exact command terminal/peer status 0–255 | Existing supervisor/peer; nextest retains its own aggregate mapping and retries. |

No unresolved typed-error representation gap was discovered. No generic carrier or diagnostic text was accepted as a substitute for a designed distinction.

## Test integrity, Contract Shape Compliance, and DES evidence

The baseline-to-runtime diff activates the authored identity, admission, collection, controller, finalization, placement, qualification and real-owner conformance bodies. It preserves their assertions. The additional conformance edits expose the existing checkout input gates before awaiting an admitted owner and print actual owner diagnostics; they do not replace production state with a driver ledger or weaken model poststates. The randomized seed restoration remains recorded separately. Native fixture changes preserve their confinement/data assertions and supplied resource envelopes.

Independent read-only comparison of the static inventory found exact equality: **4,640 approved DESIGN rows + 54 declared DISTILL additions = 4,694 runtime identities**, with no additional/missing/remapped descriptor. The partition is 3,854 host-pure, 43 native-launcher, 786 guest-group, four guest-lane-operator and seven guest-lane-owner identities. Unknown identity never grants host placement.

Contract Shape Compliance:

- Every current test body in the reviewed feature acceptance modules has its declaration: admission 6/6, collection 5/5, controller 2/2, finalization 2/2, identity 8/8, namespace tracing 2/2, placement 10/10 and qualification 1/1. The single remaining placement ignore is the deliberately external-only body, not an unimplemented capability.
- Each activated conformance entry retains its `CONTRACT_SHAPE` declaration. No source-local pure-function property was added by this implementation; the exact `/// CONTRACT_SHAPE: pure-function.` requirement has no new instance.
- Retained-record cases declare records as preconditions and snapshot source/retained-file complements; they do not claim executed guest or native evidence. Real finalizer cases use actual filesystem effects and immutable owned-file failures. Native owner and external driver layers separately observe real unit/process/root complements.
- No new implementation-authored unit test was added; the unit-test budget is therefore not exceeded. Existing real-I/O examples, exact typed refusal cases, native owner conformance and external E2E cases are not redundant unit-test multiplication.
- The pre-existing burst body/oracle is unchanged. Its documented transient exclusion is recorded below; it is not converted into a successful body or a blanket skip of owner placement.

The CLI-authored execution log records 01-01 RED → GREEN → COMMIT and the replacement 01-02 crafter's RED → GREEN → COMMIT. The historical seven event records are preserved; this reviewer wrote none. The recorded local/step evidence includes seven P1 identity cases; 32 supported acceptance cases; 24 fixture-library cases; and two transitioned qualification cases. These prior results are attributed reports, not freshly executed tests by this reviewer. No broad test campaign, compile-only `--no-run` proof or mutation result is claimed.

Independent local checks in this review: Bash syntax parsing returned 0 for all eight changed runner/driver/callback scripts; static placement equality returned true; the production-bootstrap negative reproduction below returned the actual 64 and typed marker. These checks do not earn native credit.

## FSL-1 — archive-reuse driver expects the wrong approved typed refusal

**Disposition: CLOSED — correction `4bea1ebe3dc8da3cae9451c293ddbab4f39f4671` approved. The final placement-driver native receipt remains pending. Production mapping conforms to DESIGN.**

### Reachability and independent reproduction

The real external entry is `infra/metal/tests/placement_journey.sh:120`, `run_case archive-reuse 64 run -- cargo nextest run … --archive-file "$foreign_root/absent.tar"`. `run_case` invokes the existing `cargo xtask metal run` port (`:25–35`), which dispatches `bootstrap.sh --run`. Before registration, source transfer, locks, systemd launch or START, `validate_ordinary_command` sees `--archive-file` and calls `artifact_scope_mismatch` (`bootstrap.sh:225–232`). This is the exact P4 provenance refusal in the accepted typed table. No selected body or durable owner exists, so there is no shutdown/retry/cleanup obligation on this path.

The reviewer independently executed the real bootstrap on the same argv with an unreachable placeholder target. It returned before SSH:

```sh
bash infra/metal/bootstrap.sh metal-isolation-review.invalid --run -- \
  cargo nextest run -p xtask --test acceptance \
  -E 'test(=acceptance::metal_run_isolation::placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain)' \
  --archive-file /tmp/metal-placement-foreign.REVIEW/absent.tar
```

Actual process status: **64**. Actual stderr:

```text
OVERDRIVE_METAL_RUNNER_ERROR error=artifact_scope_mismatch
```

The driver originally grouped `archive-reuse` with config/debugger replacement at `placement_journey.sh:132–134` and required `error=execution_override_refused`. Consequently it rejected the correct typed refusal even when every preceding case passed. This reproduction depends only on the current production entry and original oracle, independently of a proposed remedy; it changes no source or physical-host state.

### Required bounded disposition

The orchestrator forwarded this proven blocker to the acceptance author. Commit `4bea1ebe3dc8da3cae9451c293ddbab4f39f4671` changes only the two existing marker-group lines: remove `archive-reuse` from config/debugger replacement, and include it beside foreign target/manifest under the existing artifact-scope assertion. The archive status remains 64, every other assertion and selected body is unchanged, and no production or design file is touched.

The actual capture at `/tmp/metal-feature-closeout-20261109T104319Z/placement` independently confirms `archive-reuse.status=64` and the full public `cargo xtask metal run` stderr marker `artifact_scope_mismatch`. It retains the original driver's failure and successful external-fixture cleanup, rather than rewriting that run as a pass. The reviewer extracted the corrected source's unchanged negative-assertion block and executed it directly against those immutable outputs; all exact marker assertions returned 0. This closes FSL-1's bounded source/oracle defect. It does **not** convert that stopped capture into a complete native pass: invalid-mode, the four pure-mode cases and the final physical snapshot had not run. Their final receipt is still required.

## FSL-2 — ordinary login mode cannot create its command-tmp child after privileged runs

**Initial severity: BLOCKER. Current disposition: CLOSED — bounded correction and frozen-source four-mode native results approved below.** This was a reproduced feature-owned production defect, distinct from the excluded pre-existing burst body. The separately assessed final placement complement also passes.

### Contract and exact production reachability

P3 requires the selected command's `<remote-dir>/target/metal-run-tmp/<run-id>` access to match its existing privileged/login mode. P4 preserves `--no-sudo` and the host-pure exec path; the existing final placement driver explicitly exercises ordinary, no-sync, no-sudo and combined modes. A host-pure body requires no guest-dispatcher execution privilege.

The caller/owner path is `placement_journey.sh:140–145` → its existing `run_case` → public `cargo xtask metal run --no-sudo -- cargo nextest run …` → `xtask/src/main.rs:987–1011` → `bootstrap.sh --run --no-sudo`. Bootstrap selects `EXECUTION_MODE=login` at `:797–799`, registers the real ordinary durable systemd owner with its approved eighteen arguments at `:820–833`, waits for maintenance SH and checkout EX acquisition, transfers the bound source, and captures the checkout binding. At `:1080–1087`, before native preflight or START, it runs the invocation-owned filesystem preparation.

At the failed `4bea1ebe3` source, the exact construction is in `bootstrap.sh:1081–1084`: the shared command-tmp parent is installed as root with mode 0755, but the login-mode child is created by `sudo -n -u <remote-user> install -d -m 0700 -o <remote-user> <run-root>`. Once an earlier privileged invocation has prepared that parent, the login user has no write permission on it. The parent's existence check skips creation, and the unprivileged child create fails. This state is produced by normal earlier public privileged invocations; it is not a forced private state or an imagined schedule.

### Actual independent native reproduction

The acceptance author ran the original unmodified pure-mode tail after the FSL-1 marker correction, at source `4bea1ebe3dc8da3cae9451c293ddbab4f39f4671`. The reviewer read the complete invocation receipt, exact failed stderr/status, read-only parent stat and actual post-stop owner observation from `/tmp/metal-feature-placement-continuation-20261109T110632Z`. Source HEAD and dirty-state listings agree before/after this continuation. The reproduction uses the current production command and parent created by existing public calls, independently of a proposed fix.

```sh
cargo xtask metal run --no-sudo -- cargo nextest run \
  -p xtask --test acceptance --features integration-tests \
  -E 'test(placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain)'
```

Actual run ID: **`cbc4adb69839725766cd35be`**. Required driver result: **0**. Actual public result: **75**. The captured failure is:

```text
install: cannot create directory '/home/ubuntu/overdrive-worktrees/e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410/target/metal-run-tmp/cbc4adb69839725766cd35be': Permission denied
OVERDRIVE_METAL_RUNNER_ERROR error=runner_preparation_failed
```

The actual read-only `stat` of the exact parent reports uid **0**, gid **0**, mode **755**. `failed-login-parent-query.json` preserves the exact command, successful result and timestamps; `failed-login-parent.stdout` preserves the unmodified stat output. The prior ordinary and no-sync calls in this same continuation returned **0**; `invalid-mode` returned its expected **2**. The combined no-sync/no-sudo call and the final placement physical snapshot were not reached. The continuation enclosing result is 1 because the existing driver correctly rejects 75 where this supported path requires 0.

### Actual error, control, shutdown and cleanup disposition

The failed command directory is never created and no selected body starts. Bootstrap observes the preparation command's non-255 failure at `:1087–1094`, invokes the existing `run_preparation_failed` producer at `:67–74`, and sends the already-approved `ABORT error=runner_preparation_failed` through `abort_registered_run` at `:696–717`. The durable helper retains that cause at `lease-holder.sh:1110–1114`, exits before START, and the existing systemd stop/finalizer/collector path completes.

The actual `failed-login-owner.json` snapshot records:

| Surface | Observed value |
|---|---|
| Metadata | v2; `execution_class=ordinary`; `execution_mode=login`; source commit `4bea1ebe3dc8da3cae9451c293ddbab4f39f4671` |
| Phase | `preparing` |
| Terminal | `kind=runner`, `error=runner_preparation_failed`, finished `2026-11-09T11:08:13Z` |
| Completion | `kind=complete`, finished `2026-11-09T11:08:14Z` |
| Unit | `LoadState=not-found`, `ActiveState=inactive`, `MainPID=0`, empty ControlGroup |
| Actual process complement | empty |
| `guest_staging`, `vmm_runtime`, `command_tmp` | all absent |

Thus the typed preparation error and P5 failure cleanup are correct; they do not satisfy the supported login command's required outcome. Neither an SSH transport failure nor an unsupported native body caused this failure.

### Bounded remediation and continuation

Return this proven preparation defect to the original 01-02 crafter. Preserve the existing login-command ownership/permissions, the root/group-4200/mode-0710 launcher roots, exact error taxonomy, source/checkout exclusion, three-root cleanup and every surviving invocation. Correct the private filesystem-preparation authority needed to create this invocation's login-owned command-tmp child. Do not change public API, weaken permissions, relabel the expected result as 75, skip no-sudo, or alter the body/oracle. No new architecture or DESIGN contract is needed: the accepted access-mode contract already pins the required outcome, and this evidence disproves its implementation rather than a kernel/design premise.

The reviewer made no production/test changes and ran no additional native test. After original-crafter remediation, review its bounded diff and actual regression/tail receipts in **this same full-slice artifact**. Keep final placement and slice acceptance pending until the supported login modes and outstanding final physical complement are actually proved.

### Same-finding remediation: observed Cargo-cache boundary and private argument correction

The original crafter's parent correction let both actual login commands reach START, then exposed the next failure on the same approved command path: Cargo returned **101** because the current bound checkout's root-owned `target/debug/.cargo-lock` could not be opened as the login user. Runs `cce6d6ed6f5fe4faaf8555f2` and `04241420430e5cd67d0298cb` retain actual command terminals 101. These are command results after START, not the earlier preparation/ABORT failure. The reviewer read both raw outputs and invocation records in [the retained first follow-up](native-evidence/login-cache-followup-2026-11-09-111929Z/README.md). This proves the necessity of bounded Cargo-cache access for the unchanged login selector; no hypothetical release/target/worktree failure was used to expand scope.

Commit `e21d4fcb5c0bd96e42c9adaf90e1f8ec741f50b7` changes only the private bootstrap preparation plus those retained failed receipts. Its root-owned common command-tmp parent has the login user's existing primary group and mode **01730**: group write/search allows creation, and the sticky bit protects differently owned existing run-root entries. No new group or membership is created. Each invocation child retains the existing owner and **0700** mode. The launcher staging/runtime bases and run children remain root/group-4200/**0710**. Before a login command, while the durable service still owns checkout exclusion, the code hands only root-owned entries under that canonical checkout's nonsymlink `target/debug` back to the existing login user with `chown -R --from=0 "$user" "$target/debug"`. It changes ownership metadata without deleting or rewriting cache contents, does not change group ownership or issue a recursive chmod, and does not operate on the source tree, retained evidence/records, captured helpers, other target subtrees or any invocation's private temporary root. The production ordinary/exclusive grammar, login/privileged command UID, command exec shape, internal context, one deadline, typed failures and finalizer are unchanged.

The reviewer also checked GNU chown's physical recursive default: without `-H`/`-L`, recursive mode uses the symlink itself rather than traversing/dereferencing it; neither traversal option is added by this change. The root `target` and `target/debug` paths are separately rejected if symlinks. This supports the bounded cache operation's complement, without adding a new filesystem mechanism. [GNU coreutils implementation, recursive-mode option resolution](https://raw.githubusercontent.com/coreutils/coreutils/master/src/chown.c).

The first e21 source review found a misplaced private `%s` argument introduced by the cache branch. Evaluating the exact production construction emitted `sudo -n -u ubuntu install … -o root "$run"` for the privileged branch. This remained an **unproven hypothesis** until the original crafter reran the existing public ordinary selector: run **`da9684effbdb0e2eb6838e42`** actually returned **75** before START with `Operation not permitted` when attempting that chown. The raw [privileged reproduction](native-evidence/login-mode-cache-argument-correction-2026-11-09-113245Z/privileged-pure.output.log) is retained. No production finding or fix mandate was accepted from static suspicion alone.

Commit **`060359bc1cf88c78252e9d789396821413b41d7e`** swaps only those two existing format arguments. The reviewer read its complete bounded diff and evaluated the actual construction again: cache handoff uses root preparation, the privileged child is installed as root with owner root, and the login child is installed as the login user with that user's ownership. The format string, modes, paths, error mapping, public surface, command owner and remaining helper arguments are unchanged. This is a private argument correction, not new architecture or a broadened ownership protocol.

The administratively re-attributed `2ceef898`/`e21d4fcb` pair remains explicitly invalidated for no-sync qualification: even with identical trees, changing HEAD changes the accepted Git/source binding. Its actual no-sync `source_mismatch`/78 is preserved, not relabeled as runtime failure or acceptance. The stable e21 login pair passed but did not validate the privileged branch. Neither pair is substituted for the final-source four-mode evidence.

### Frozen-source native closure of FSL-2

The reviewer independently read every invocation record and complete stdout/stderr in [the final four-mode receipt](native-evidence/login-mode-cache-argument-correction-2026-11-09-113245Z/final-source-four-mode-pass/). The source-before/source-after records are **byte-identical**, including HEAD **`060359bc1cf88c78252e9d789396821413b41d7e`**, the actual dirty-state listing and bootstrap SHA-256 **`47e7152b18ef5d7b57a26d288f473769d70258d8086477bdb563cd9c17e2f625`**; that hash also matches the independently read current script. All four use the existing unchanged acceptance selector and driver order, with no new body, skip, model, mutation or unrelated native campaign.

| Existing mode | Actual run ID | Outer result / actual command result | Selected-body evidence |
|---|---|---|---|
| Ordinary privileged | `eac8724e1b762671708565f7` | 0 / 0, `command_returned` | 1 selected test, 1 passed |
| Ordinary privileged, no-sync | `8e7b311b75348b3612cf43ad` | 0 / 0, `command_returned`; source check passed | 1 selected test, 1 passed |
| Ordinary login, no-sudo | `06ea1168e446d77960b36bb4` | 0 / 0, `command_returned` | 1 selected test, 1 passed |
| Ordinary login, no-sync and no-sudo | `772d30eb605de6b8f5f0e13a` | 0 / 0, `command_returned`; source check passed | 1 selected test, 1 passed |

Every log retains START and the collector's exact complete marker; no source mismatch or preparation error was hidden. This closes the original FSL-2 outcome, its reproduced cache dependency and its introduced privileged-argument regression. The author-reported focused Lima syntax and three existing preparation/passthrough/finalization cases support the bounded change; they are not substituted for these actual native results. This subsection credits the four mode results only; the final external physical/process/three-root observation is assessed separately below.

## Native evidence and precise limits

The reviewer independently inspected the retained raw historical paired snapshots and repeat streams. At `native-evidence/worktree-concurrency-2026-11-08-230613Z`, both recorded owners are ordinary/active/running concurrently; raw before/after physical dictionaries are equal; both repeat stdout/stderr pairs are byte-identical; final owners have complete receipts, no processes and all three roots absent. This is genuine historical evidence at its recorded sources, not a fresh proof of the later runtime changes.

The retained `admission-rerun-2026-11-09-040806Z` raw completed snapshot has the public timeout terminal 100 and the controlled prior owner/EX/waiter terminal 0, all with complete receipts, no processes and no owned roots. Its E4 scope remains the authored controlled-owner fixture on one fixture-private barrier. Public default-barrier guest timeout and historical default-path overlap remain separately attributable.

The post-correction owner receipt `owner-queued-eof-verification-2026-11-09-090213Z` preserves exact failing seed `0x8e8e19a4`, both controller-loss controls, and a fresh randomized execution at `0x83bfe04`, all passing through the corrected actual helper. It is four executions across three identities, not a fresh all-fifteen campaign. Iteration 25 independently approved that evidence. Earlier direct-native and specifically named nested captures retain their actual claim boundaries; none substitutes for the pending complete final placement receipt.

### Scoped pre-existing owner-burst exception

The user explicitly instructed: **“stop focusing on failures that are not related to this feature !!!!!”** The orchestrator supplied and the reviewer read the retained preimplementation comparison: canonical preparation at clean `0e8ae265c` succeeded, and the exact original owner fixture then failed the same unchanged one-second/1,000-flow body assertion with 78 ends consumed. The original fixture/body blob identities match the current source. The current original-fixture comparison fails independently of the new dispatcher. Those receipts are [preimplementation baseline](native-evidence/owner-burst-preimplementation-baseline-2026-11-09/README.md) and [current original-fixture comparison](native-evidence/owner-burst-current-branch-comparison-2026-11-09/README.md).

This is a scoped baseline exception for that one known body. It is not a new metal-run-isolation defect, and this review neither re-investigated nor prescribed a fix for it. The original body, fixture, 1s/1,000-flow oracle and checked-in owner-cell selection remain intact. The acceptance author's documented transient selection may exclude that known failing body for the remaining relevant native proof. **A full current-source owner-cell GREEN remains unproven.** This exception does not waive P1–P5, other owner-lane evidence, exact status/selection semantics, physical kernel complements, stopped-owner process complements or three-root cleanup.

### Current-source native receipts and remaining gate

The acceptance author's sequential `/tmp/metal-feature-closeout-20261109T104319Z` capture has now been forwarded and independently read. Its invocation records pin source HEAD `8914e5ee47da37689919399382b9304d75d18a21`, actual dirty state, commands, timestamps and runtime hashes matching the reviewed implementation; the paired peer update/preparation is separately retained.

| Boundary | Actual current-source evidence | Disposition |
|---|---|---|
| Paired worktree K1–K3 | `worktree/foreground-invocation.json` reports 0 and the driver prints its complete success marker. Raw before/after physical dictionaries are equal. Final run IDs `531f46c36767a7f42293490d` and `a5e15be51e13740a6687f1a2` both have complete receipts, no processes and all three roots absent. | **PASS at recorded source**; the raw polling failures remain retained before the decisive successful observations. |
| Admission | `admission/foreground-invocation.json` reports 0 and prints the complete public-timeout/controlled-E4 marker. All four actual completed owners have complete receipts, no processes and no owned roots. | **PASS at its declared controlled-E4 boundary**, preserving the separate default-barrier timeout attribution. |
| Placement prefix | Actual retry/helper/env/255, timeout, reset, default/ci, direct-native, named-nested, operator and typed negative-case statuses are retained. The known burst body alone is excluded through the documented transient copy. Original driver exits 1 only at FSL-1's wrong marker; external fixture cleanup is 0. | Actual cases retained; **not a complete driver pass**. FSL-1 is closed by the exact corrected assertion replay. |
| Placement continuation | At corrected driver source `4bea1ebe3`, invalid-mode is 2, ordinary and no-sync are 0, then no-sudo is 75 with the independently observed root-owned-parent failure. Source status is stable across this continuation. | **FAIL — FSL-2**. Combined mode and final placement `after.json` remain unrun. |
| Final existing mode selectors | At frozen source `060359bc`, all four actual ordinary/privileged/login/no-sync combinations returned 0, each selected body passed, and exact before/after source records match. | **PASS — FSL-2 closed**. |
| Final placement complement | Evidence `d4d011b4` retains the unchanged final observer/assertion tail returning 0 at evidence-only HEAD `f8c0f02d`, with the corrected runtime bootstrap hash and matching before/after source facts. | **PASS**, independently checked against actual raw physical and owner snapshots below. |

The earlier `110421Z` continuation is expressly invalidated in the author's next receipt: copying new evidence into the source during its ordinary/no-sync sequence correctly triggered source-mismatch refusal. It is retained as an invalidated attempt, not a product defect or a reason to weaken source matching. The stable `110632Z` continuation above is the FSL-2 reproduction.

The source-matched worktree/admission passes, original placement outcomes, corrected negative-marker oracle, final four-mode correction proof and final observer discharge their respective feature outcomes at their explicitly recorded sources. They do not manufacture one identical-SHA uninterrupted full journey. No unrelated native campaign or new full review cycle was run.

### Final complement and consolidated receipt — independently approved

The reviewer read the [final receipt](native-evidence/placement-final-complement-2026-11-09-114229Z/README.md), its raw invocation/source records, final observer script, original before/final after snapshots, complete observer output and consolidated case-to-receipt mapping. The raw files are independently byte-equal to their retained `/tmp/metal-placement-final-observer-20261109T114229Z` origins. The copied physical baseline has its recorded origin/hash; the final tail is **byte-identical to the existing checked-in placement driver's tail** after the observer call. It changes no assertion or normalization rule and runs no test body.

The actual foreground result is **0**, with the exact placement success marker. Before/after source facts are equal at evidence-only HEAD `f8c0f02d224887f2cd7ef67c3f0547950e7b71d7`, with bootstrap hash `47e7152b18ef5d7b57a26d288f473769d70258d8086477bdb563cd9c17e2f625` and unchanged lease-holder/observer hashes. The reviewer independently compared the raw dictionaries: modules, nftables, BPF program identities, test cgroups, initial vsock modes and all route identities/other attributes are exact. The only raw differences are the original permitted finite RA expiry countdowns **2591998→2591997** and **13→12**. Applying precisely the unchanged pre-authored projection yields full equality. No raw observation was rewritten, no test-owned physical delta was excused, and no zero-state contract was weakened.

| Actual final-tail owner | Unit | Retained terminal | Completion / process / root observations |
|---|---|---|---|
| `1b54417e839c436092b85ad3` | inactive/not-found, MainPID 0, no ControlGroup | command **100**, `command_returned` | complete receipt; **0 processes**; `guest_staging`, `vmm_runtime`, `command_tmp` all absent |
| `599aaa9da34be0fb5cd26f9f` | inactive/not-found, MainPID 0, no ControlGroup | command **255**, `command_returned` | complete receipt; **0 processes**; all three exact roots absent |

The reviewer additionally checked all **31** consolidated prefix status entries against their actual retained status files; every actual value matches its original expected value. This includes the intended nextest failures at 100 and direct command result 255; they are not relabeled as test success. Invalid mode retains its actual 2. The four corrected-source mode results are separately pinned above. The archive marker correction retains the same accepted error/status and its initial failed driver result. Every original failure/invalidated source-changing attempt remains attributed to its actual source.

The current paired-worktree raw capture independently shows SH on the canonical physical barrier and EX on each distinct canonical checkout, same numeric CID **16785416** held by actual VMM PIDs **2726976** and **2727106** in distinct namespaces, a stopped first owner with complete cleanup, and the surviving owner's unchanged binding/source/roots and live VMM. Both repeat stdout/stderr pairs are byte-identical. The raw controlled-E4 snapshots bracket the prior ordinary owner as deactivating throughout the read while the public maintainer holds actual EX and the ordinary waiter has no SH; only after prior cleanup does the maintainer reach preparing, while the waiter remains unadmitted. These remain their precise native boundaries, not driver-ledger projections.

The runtime changes after the successful prefix/pair are confined to the proven command-tmp/Cargo-cache preparation correction and its argument mapping. They do not alter the lease owner, control/record/error formats, namespace/guest dispatcher, physical observer or finalizer. The four final-source mode executions validate those changed preparation branches in both UID modes with and without source transfer. Accordingly the consolidated evidence is coherent for the final implementation without pretending that the earlier body executions reran at `060359bc`, or requiring a new campaign over already-passing unchanged mechanisms.

The single excluded preimplementation owner-burst body remains **unproven as GREEN**. That qualified baseline disposition does not weaken or waive P1–P5, physical kernel isolation, exact command meaning/results, process cleanup, owned-root absence or survivor preservation. No additional feature-specific native acceptance item remains outstanding in this review.

## Subtractive assessment and scope

Each runtime mechanism is required by the accepted outcome: canonical full checkout binding, service-owned two-lock admission, per-invocation records, anonymous local vsock context, exact nextest placement, short owned staging/runtime/tmp roots, and existing-unit finalization. The closed policy is approved inventory data, not a framework. The dispatcher reuses the existing groups and host scripts; no general scheduler, new persistence subsystem, new ownership authority, recovery protocol, host-wide cleanup or product control-plane seam is introduced.

No adjacent hardening or out-of-scope architectural change is mandated. Static suspicion without a bounded real-entry reproduction is not promoted into a production finding. FSL-1 is closed at its external oracle boundary. FSL-2 requires only the proven private preparation correction needed for the existing supported login-mode outcome; it does not authorize new architecture or weaker invariants.

## Review history and verdict

| Revision | Evidence / finding disposition | Verdict |
|---|---|---|
| 1 — full source audit | Independently audited all feature-owned runtime/test/configuration changes against P1–P5; reproduced FSL-1 through the real bootstrap; checked static inventory/declarations and retained historical native complements; final native receipts pending. | **PENDING** final receipts and FSL-1 closure. No independently reproduced production defect. |
| 2 — bounded FSL-1 remediation review | Inspected the exact two-line diff in `4bea1ebe3`, confirmed the real public native archive refusal, and independently replayed the corrected marker block against immutable raw outputs. FSL-1 closed; source and typed contract unchanged. | **PENDING** complete final native receipt only. |
| 3 — current-source native receipt and FSL-2 | Independently read the current worktree/admission passes, partial placement prefix and stable failed login continuation, parent stat and stopped-owner observation. FSL-2 is a reproduced implementation blocker; correct error/cleanup is credited separately. No production or test edit by reviewer. | **NEEDS_REVISION** for FSL-2; final placement and slice acceptance **PENDING**. |
| 4 — same-finding bounded remediation and final four-mode proof | Read the first actual Cargo 101 dependency, reviewed the current-checkout-only ownership change, reproduced private command construction, required the real public privileged regression before accepting that finding, then reviewed the two-position correction and every frozen-source four-mode receipt. FSL-2 closed. | **PENDING** final external placement complement only; no open reproduced implementation finding. |
| 5 — final original complement and consolidated evidence | Independently checked the unchanged final observer/tail, raw source/baseline provenance, exact two RA-expiry-only differences, actual stopped-owner/process/three-root observations, all 31 original status files and four corrected-source mode records. Native claim boundaries and the single baseline exclusion remain explicit. | **APPROVED** — no open feature finding or remaining feature-specific native check. |

**APPROVED — full-slice implementation and consolidated feature-specific native acceptance review.** Both reproduced feature findings are closed, and all remaining feature-specific original outcomes/complements are supported by the explicitly attributed actual receipts. The one full-slice review is complete; no additional full audit or native campaign is required. This verdict does not claim an uninterrupted one-SHA full-driver run, a fresh all-fifteen owner campaign or a passing pre-existing burst body. Their stated limits are preserved; none is used as a blanket waiver of P1–P5 or any zero-state invariant. Documentation/finalization bookkeeping remains the orchestrator's responsibility and must report this same qualified evidence honestly.
