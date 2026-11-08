# DELIVER Review — 01-01: Bind each checkout's source and retained artifacts

## Review metadata

- Feature: `metal-run-isolation`
- Step: `01-01`
- Iteration: 1
- Reviewed commit: `ee0bbd65805524afa4a6348b5019ce12a7390e8b`
- Baseline: `0e8ae265`
- Reviewer: independent `nw-software-crafter-reviewer`
- Date: 2026-11-08
- Verdict: **APPROVED**
- Scope: the reviewed step commit, its P1 obligations, the seven assigned acceptance activations, DES phase evidence, and the accepted P1 contract. The already accepted P2–P5 decisions and stopped service feature are not reopened.

## Authority and reviewed artifacts

The exact interface contract is P1 in [feature-delta.md](../feature-delta.md), accepted by [ADR-0277](../../../product/architecture/adr-0277-metal-checkout-identity-is-owned-by-the-local-git-worktree.md). Step boundaries and the seven activation names come from [roadmap.json](roadmap.json) and the source-ownership activation notes in `feature-delta.md`. The reviewed implementation is commit `ee0bbd658`; the baseline already contains the reviewed preparation and complete acceptance bodies.

The target commit changes only `infra/metal/bootstrap.sh`, `infra/metal/README.md`, seven pending markers in `xtask/tests/acceptance/metal_run_isolation/identity.rs`, and the step execution log. All seven test bodies and their pre-authored assertions remain unchanged. Existing dirty service-feature work and the post-commit DES log event are outside the target commit and were left untouched.

## Summary

The implementation derives checkout identity before remote contact, selects the P1 default directory from the complete identity, canonicalizes overrides before deriving their checkout lock, and verifies the complete binding before transfer. Binding and source markers are kept out of rsync deletion; the source marker is atomically replaced after source transfer. Pull uses the same checkout lock and binding while allowing local edits made after the run. The seven P1 cases exercise the real `bootstrap.sh` entrypoint with real Git/filesystem/rsync behavior and the established SSH transport fixture.

The existing `--run` execution still acquires the physical global lease and its checkout lock. This step does not admit shared ordinary execution or change the run owner protocol; those integrated P2–P5 obligations remain assigned to step `01-02`. The P1 missing-rsync acceptance covers source-only sync and pull. The registered-run preparation/ABORT mapping remains at the later accepted P2 boundary, and the legacy `--run` installer path is unchanged in this step.

No public API, CLI flag, protocol, error variant, ownership rule, or lifecycle meaning was invented. No correctness finding remains open.

## Contract and implementation audit

### Canonical checkout identity

| P1 condition | Implementation evidence | Acceptance evidence | Result |
|---|---|---|---|
| Git discovery must succeed; a bare repository or detached ordinary clone refuses instead of guessing | `infra/metal/bootstrap.sh:134-161` validates the non-bare repository, physical top level, absolute Git directories, full symbolic ref, and ref format before the first SSH call | `sad_invalid_identity_and_override_never_contact_a_mutating_remote` | Pass |
| A linked worktree is identified by its canonical worktree-specific Git directory, stable across branch switch and detached HEAD | `infra/metal/bootstrap.sh:142-174` canonicalizes the Git and common directories; different directories select `kind=worktree` and hash the Git-directory bytes | `linked_identity_survives_branch_switch_and_detachment`; same-branch worktree contrast in `same_branch_worktrees_keep_source_artifacts_and_evidence_disjoint` | Pass |
| An ordinary clone fallback is physical checkout root, NUL, and the full attached branch ref; detached ordinary clones do not fall back to names | `infra/metal/bootstrap.sh:153-168` constructs the branch identity and domain-separated key | `attached_clone_fallback_and_sanitised_spellings_never_adopt_old_state` and the detached-clone refusal | Pass |
| The key includes the specified domain separator, kind, and exact identity bytes; reuse compares the full binding, not only the digest | `infra/metal/bootstrap.sh:163-174,189-192,246-275` computes the key, serializes the four ordered LF fields, and byte-compares the complete binding | `assert_binding` independently checks the specified encoding/hash; the foreign-owner case changes `identity_b64` while retaining the recorded key | Pass |
| Branch spelling, sanitization aliases, and old cache names do not select or adopt the new identity | `infra/metal/bootstrap.sh:177-192` selects only the type prefix and full key; the old branch-name path is never selected by the default | `attached_clone_fallback_and_sanitised_spellings_never_adopt_old_state` preserves the legacy directory and binds distinct identities | Pass |

The implementation uses the absolute, physical checkout root and canonical Git-directory paths (`pwd -P`) before building identity bytes. Linked-worktree identity therefore does not depend on the branch ref; ordinary-clone identity intentionally includes the full ref.

### Override, binding, source transfer, and pull

| P1 condition | Implementation evidence | Acceptance evidence | Result |
|---|---|---|---|
| Override syntax is relative, uses the approved character set, and rejects empty, `.`/`..`, and symlink components | Local syntax checks are at `infra/metal/bootstrap.sh:177-184`; the remote resolver checks physical-home containment and override symlink components at `:374-404` before lock acquisition or transfer | Invalid syntax and symlink cases in `sad_invalid_identity_and_override_never_contact_a_mutating_remote` and `sad_override_conflicts_preserve_the_full_remote_complement` | Pass |
| Canonical spellings of one remote directory derive the same full checkout lock | `infra/metal/bootstrap.sh:368-405,421-424` canonicalizes the remote directory and hashes its absolute bytes for the default lock and owner paths | Override cases exercise the selected directory and lock-bound source operation | Pass |
| Only a new or empty directory may be bound; a non-empty unbound, malformed, symlinked, or differently owned directory refuses before source rsync | `infra/metal/bootstrap.sh:242-310` probes the directory and exact binding, only writes the binding after an empty/new result, and maps conflict to the approved refusal; the lock is acquired at `:449-463` before binding checks | `sad_override_conflicts_preserve_the_full_remote_complement` snapshots the full remote complement for all four conflict states | Pass |
| Binding survives rsync deletion and source identity publishes only after transfer | `infra/metal/bootstrap.sh:726-737` excludes the root binding from `rsync --delete`; `:784-789` writes the source marker to an adjacent temporary file and renames it after source and Git transfer | Repeated sync preserves binding and `target/` in `pull_uses_binding_after_local_source_changes_and_repeated_sync_preserves_target` | Pass |
| Pull is serialized by the same checkout lock and binding, permits local source edits, retrieves named evidence, and does not mutate the remote checkout | `infra/metal/bootstrap.sh:453-457,668-696` takes the checkout lock, validates binding, and transfers only requested paths without deletion; it does not compare the local dirty digest to the pre-run marker | `pull_uses_binding_after_local_source_changes_and_repeated_sync_preserves_target` | Pass |
| Missing remote rsync for the P1 source-only sync and pull cases is a typed preparation refusal, with no package installation | `infra/metal/bootstrap.sh:661-671` returns `runner_preparation_failed prerequisite=rsync`/75 before sync or pull mutation. The legacy `--run` transfer/install handling remains unchanged pending its registered-run P2 protocol in step `01-02` | `sad_missing_rsync_is_typed_without_installing_packages` verifies sync-only and pull both refuse, no install sentinel appears, and remote/run snapshots remain unchanged | Pass |

### Existing execution authority

`infra/metal/bootstrap.sh:449-463` still routes `--run`, shell, and provisioning through the existing global physical lease; only sync-only and pull take the checkout lock alone. The run path continues to take the checkout lock after the global lease (`:602-635`), and the existing durable command retains checkout exclusion. This commit does not admit concurrent ordinary runs before step `01-02`.

### Typed failure contract

| Designed P1 failure | Approved representation | Implemented mapping | Review result |
|---|---|---|---|
| Invalid Git discovery, bare repository, or detached ordinary clone | `checkout_identity_invalid`, status 64, before remote mutation | `checkout_identity_invalid()` at `infra/metal/bootstrap.sh:33-36`; discovery failures route to it at `:135-161` | Exact |
| Invalid override syntax or symlinked/out-of-home override | `remote_subdir_invalid`, status 64 | `remote_subdir_invalid()` at `:38-41`; local syntax at `:177-184`, remote read-only validation at `:374-404` | Exact |
| Non-empty unbound, malformed, or differently bound checkout in bind mode | `checkout_identity_conflict`, status 78 | `checkout_identity_conflict()` at `:43-46`; `ensure_checkout_binding()` at `:246-310` | Exact |
| `--no-sync` binding or source-marker mismatch | `source_mismatch`, status 78 | `ensure_checkout_binding()` maps check-mode conflicts at `:283-308`; source marker mismatch maps at `:791-796` | Exact |
| Missing rsync for P1 source-only sync/pull | `runner_preparation_failed`, status 75, immediate `prerequisite=rsync` context | `rsync_prerequisite_failed()` at `:53-56`; checks at `:661-671` | Exact |

These failures are expressed as the approved finite shell marker and exit-status pairs. No semantic distinction is hidden in a diagnostic string, and no new public Rust error type or CLI surface is needed for this step. The P2 registered-run preparation and tagged-ABORT lifecycle remains explicitly outside this step, as reflected in the later roadmap acceptance body.

## Test integrity and TDD evidence

The diff in the acceptance file consists of exactly seven removals of `#[ignore = "pending DELIVER: …"]`. It adds, removes, or rewrites no assertion, fixture, expected value, test identity, or `CONTRACT_SHAPE` declaration. All seven activated cases already carry `/// CONTRACT_SHAPE: bounded-change`; the active test names do not match the banned status/call-count name pattern. These are pre-authored cases, so this review does not create a retroactive test-authoring requirement.

The step has four roadmap acceptance criteria and seven assigned real-I/O acceptance cases, below the role's `2 × behaviors` ceiling of eight when counted conservatively at criterion level. No unit/property tests were introduced; the source/Git/rsync/bootstrap integration cases remain at their authored boundary. The bodies compare exact binding bytes and whole-tree complements where required, and drive the copied production bootstrap rather than an internal helper.

DES phase evidence is ordered and complete. The step report records a semantic RED where the expected binding count was one and the observed count was zero, followed by all seven Lima cases passing in GREEN. The working execution log now records RED/FAIL, GREEN/PASS, and COMMIT/PASS in order. The reviewed commit itself contains the CLI-initialized RED and GREEN events; the subsequent COMMIT event is the expected post-commit worktree update.

The step report also records successful `bash -n`, targeted acceptance binary Clippy, and `git diff --check`. The initial all-target xtask Clippy attempt found two lints in untouched baseline acceptance fixture code (`placement.rs:41` and `support.rs:137`); the original acceptance author closed those separately in commit `4bc17cc` with behavior-equivalent `map_or_else` expressions, and reports that all-target xtask Clippy now passes. Those baseline lints are not attributed to `ee0bbd658`.

No reviewer tests or checks were run. The user explicitly stopped tests/checks requiring a Metal lease and prohibited Metal/native/SSH-bypass/remote-host activity; no mutation testing was run. The recorded Lima evidence is sufficient for this step, and no changed or uncertain behavior justified rerunning it.

## Contract Shape Compliance

- **Required declaration:** PASS. All seven transitioned tests retain their pre-authored `CONTRACT_SHAPE: bounded-change` declaration.
- **Outcome and assertion shape:** PASS. The names and assertions describe checkout binding, branch/detached stability, legacy/cache isolation, typed refusal, pull fidelity, and missing-rsync behavior. The complement snapshots preserve the declared unchanged remote state where that is part of the outcome.
- **Test identity and selection:** PASS. No test was renamed, duplicated, broadened, or rewritten; only the seven assigned pending markers were removed. The eighth identity case remains ignored for step `01-02` as the activation notes require.
- **Implementation coupling / theater:** PASS. Each case launches the actual bootstrap script with real Git and rsync behavior; the fixture supplies only the existing external SSH transport boundary. Assertions check persisted binding bytes and source/artifact complements, not private helper calls.

## Equal-weight subtractive review

The changed mechanisms each serve the approved P1 contract: a domain-separated full identity, a full binding record, a canonical remote-directory lock, exact error helpers, and atomic source-marker publication. Removing any one would weaken the accepted identity, ownership, refusal, locking, or source-fidelity outcome. Existing legacy directories are retained; no migration, recovery subsystem, concurrency admission, scheduler, or extra API was added.

The new `ensure_checkout_binding()` helper is longer than the generic RPP L2 line-count heuristic, but it implements one cohesive boundary operation: probe the selected directory under its lock, compare the complete approved binding, and bind only an empty/new directory. Its probe and writer are private remote scripts needed to preserve exact bytes across the SSH shell boundary. Splitting them would be style-only for this bounded step, so no remediation is requested. No other L1–L2 readability, duplication, or speculative-generalization issue blocks the change.

## Verification disposition

- Seven assigned P1 acceptance cases active and reported green in Lima: **PASS**.
- Acceptance bodies and assertions preserved: **PASS**.
- P1 interface and semantic error mappings: **PASS**.
- Exclusive run authority preserved pending integrated step `01-02`: **PASS**.
- Commit scope and attribution: **PASS**. The existing Git author is unchanged and the commit has exactly `Co-Authored-By: Codex <codex@openai.com>`.
- Reviewer rerun: **NOT RUN**, per the explicit Metal-lease stop and because the reported GREEN evidence is unchanged.
- Findings: **0 blocking; 0 non-blocking**.
- Remediation disposition: **None required**.

## Verdict

**APPROVED.** Commit `ee0bbd65805524afa4a6348b5019ce12a7390e8b` satisfies the approved P1 contract and step `01-01` acceptance criteria. The next step may be considered only after this artifact is present and the repository's strict step sequence is followed.
