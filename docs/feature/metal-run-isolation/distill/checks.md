# DISTILL checks — metal-run-isolation

2026-11-08. No production implementation, guest campaign or mutation run.

The canonical Linux runner executed:

```sh
cargo xtask metal run -- cargo nextest run -p xtask --test metal_run_isolation --features integration-tests --run-ignored only -E 'test(identity::same_branch_worktrees_keep_source_artifacts_and_evidence_disjoint)' --no-capture
```

Actual durable invocation: `2a09b487e43a420ff22f5118`. All 25 then-authored identities compiled/collected; exactly one selected test executed and returned nextest aggregate **100**. Bootstrap successfully transferred the selected fixture source with real local Git/rsync before this semantic assertion failed:

```text
assertion `left == right` failed: successful sync must bind exactly its canonical checkout
  left: 0
 right: 1
```

This is missing P1 behavior after successful source transfer, not an unclassified-test refusal, missing executable, parse failure or infrastructure failure. The original same-branch fixture remains authored beyond that first failed assertion. Later additions are separately compiled below. The outer runner is the current legacy exclusive implementation; this result grants no v2 ordinary-concurrency or guest evidence. Raw transport output remains in the existing durable invocation; target addresses are not copied into checked-in evidence.

All executable Rust scenarios remain reasoned pending markers until their corresponding bounded DELIVER work activates them. The standalone attempt fixture deliberately returns 255 / exceeds the fixture default timeout; only its external tooling driver interprets those expected outcomes. It is outside the workspace default suite.

After moving the scenarios into the existing acceptance entrypoint (repository layout rule), the canonical runner compiled the complete 27-case Metal module together with the existing acceptance binary and reran only the same first scenario:

```sh
cargo xtask metal run -- cargo nextest run -p xtask --test acceptance --features integration-tests --run-ignored only -E 'test(acceptance::metal_run_isolation::identity::same_branch_worktrees_keep_source_artifacts_and_evidence_disjoint)' --no-capture
```

Invocation `48b5afb9819c7e75ec65c3fa`: one test executed, 41 other existing/new identities filtered; the same missing-binding assertion failed after successful transfer, nextest **100**. This preserves the semantic RED after conventional placement; it is not a second behavioral scenario or native guest proof.

The standalone Cargo input fixture compiled and listed exactly its two ignored bodies plus the actual helper binary through `cargo xtask metal run -- cargo nextest list --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --message-format json`, exit **0**. [Actual metadata excerpt](fixture-list.json) confirms exact binary/test/helper identity; no body was run and no guest result is claimed.

Shell syntax checks for all three external drivers and transport fixtures, Rust formatting, observer syntax and scoped whitespace checks passed. No `.feature` files, Gherkin framework, production changes, model rerun, mutation test or native E2E campaign was added. Scoped canonical Linux Clippy over `xtask --test acceptance --features integration-tests -- -D warnings` passed in invocation `7d65236860c520d8e66beca1`; the final fixture-input refinement is checked separately below.

Final scoped canonical Linux Clippy, invocation `bbf8b334992a9592f1c6321a`, exited **0** after the private durable-owner fixture gained disjoint bound source inputs, unique valid invocation IDs and captured finalizer cleanup diagnostics. Command: `cargo xtask metal run -- cargo clippy -p xtask --test acceptance --features integration-tests -- -D warnings`. Shared Metal admission was queued without bypassing or stopping another owner. The final Rust corpus remains 27 cases, all compiled behind the existing Linux acceptance gate. No further runtime execution was added.

## Completed gap authoring and bounded validation

The final corpus adds the owning-crate quint-connect driver, all thirteen named runs, bounded randomized traces and explicit illegal-control coverage; actual controller producer failures; all namespace-operation causes; actual guest peer loss and three root-removal failure producers; a real single-deadline case; default/ci guest placement canary; and two pre-authored existing qualification transitions. The recording incompatibility was handled by the DESIGN owner and independently approved; its codec/model evidence is linked from the self-audit rather than repeated as an owner-conformance pass.

Canonical invocation `6b85244cdbe7eded544d2dc7` compiled the owning library and acceptance targets, then selected exactly one named owner-conformance case and one real descriptor-exhaustion fixture control. The fixture passed in 0.255 s: its owned child performed both actual namespace creations, its actual verification open returned EMFILE, and the caller's namespace modes remained unchanged. The named replay generated its real sole-model trace, printed seed `0x472ffaf4`, reached the production source-transfer adapter and failed on missing P1 binding:

```text
actual source transfer did not publish the canonical binding
Reproduce this error with QUINT_SEED=0x472ffaf4
```

Aggregate nextest status was **100** (one intended semantic RED, one passing fixture control). This establishes executable replay and its correct current missing-feature failure; no post-P1 protocol state, native guest success or completed outcome is claimed.

The later three-case request `3243239ba60335fada1b5642` executed **zero tests**: its existing 120-second admission wait expired behind another workspace's exclusive run. That is retained as admission evidence, not a code failure. No owner was bypassed or interrupted. Final verification is queued using the existing configurable admission budget after the source/artifacts are frozen. Intermediate scoped compiler findings were repaired; no failure was converted into a passing feature claim.

## User-directed handoff without further Metal execution

The user explicitly instructed **“STOP RUNNING TESTS THAT REQUIRE THE METAL LEASE”**, waived further pre-implementation Metal executions for this DISTILL handoff, and required remaining work to be local/static. No further Metal command, direct-SSH test, alternative hidden execution, retry or lease bypass is authorised for this handoff. A local process inspection found **zero own active Metal verification controllers**, so no pending controller needed cancellation; another workspace's owner was left untouched.

Retained results, with their limits:

| Evidence | Actual result |
|---|---|
| Scoped canonical Linux Clippy, `fa383cbc445103f6dcaa5b9e` | **PASS**, including owning test modules and the qualification transition as they stood at capture. Later fixture-input/lifetime refinements have not been rerun |
| Three-case canonical run, `c27c07eecf71f226612c0776` | Named owner replay reached intended missing-binding RED, seed `0x6d789d91`; actual verification-read fixture **PASS** in 0.249 s; child-mode/capacity fixture **FAIL** because its helper was inaccessible after entering the resource context |
| Permission fixture adjustment | Helper is staged in its owned input directory; namespace inputs use a traversable owned context. **NOT RUN after adjustment**, per the user's stop instruction |
| Final scoped Clippy after all fixture refinements | **NOT RUN**, user waiver |
| Final three-body standalone fixture metadata refresh | **NOT RUN**, user waiver. `fixture-list.json` remains the truthful earlier two-body capture; the third body is statically declared in the placement handoff |
| Effective default/ci nextest capacity/profile views | **NOT RUN**, user waiver. Declared TOML and exact canary placement pass local parsing; effective runtime settings are not claimed |
| Full v2 owner protocol, producer faults and external two-worktree/native E2E | Pre-authored and pending implementation/activation. **NOT RUN** for this handoff; no success or completed dogfood is claimed |

All 15 completeness items concern authored coverage. Runtime verification remains separate from that audit, and independent consolidated review remains required. No native execution is needed to close this authorised authoring handoff. Future implementation validation is outside this waived pre-implementation run.

## Controlled E4 NativeUnit authoring refinement — 2026-11-09

After the preserved 024233Z transient-observation miss, the authorized fixture refinement shares the already-authored NativeUnit command/finalizer file-input primitive between Rust conformance and the external E4 prior-owner constructor. Local checks executed the private command process to its actual exit-input result and verified that releasing the bounded callback delegates the exact `--finalize <id> <runs>` arguments. The callback target in that local check was an argv fixture, so it earns **no production cleanup or native-owner credit**. Static evaluation of the actual constructor confirmed exactly eighteen production durable arguments and the existing Type/ KillMode/60s/45s properties.

Offline consumer inputs verified the required three-owner contrast and subsequent EX progress, rejecting an ended prior, advanced phase, replaced main PID or waiter READ descriptor. The whole multi-owner observation is bracketed by fresh actual unit/phase queries and remote monotonic timestamps; fixture handles are never projected as owner state. Rust formatting, Bash syntax, Python/embedded-Python parsing, scoped whitespace and unchanged historical-capture hashes passed. No test identity, model check or production file was added or changed. **Linux compilation, real systemd execution of the new external fixture, native E4 and the complete driver rerun are NOT RUN before independent focused review.**

## Native run-parent fixture inputs — 2026-11-09

The existing constructors in `vsock_vmm_device.rs` and `vmm_equivalence.rs` now prepare only their supplied run parent with the established group/mode idiom. Local formatting and scoped whitespace checks passed. Static comparison retained every original assertion body, test identity, 128MiB constant and confinement/backend/data oracle; the affected existing identities retain their reviewed native-launcher placement. The direct-native failure branch adds only owned-parent/child stats and existing bounded diagnostics, with zero additional exit-watch wait.

Canonical Lima **compile PASS**: `cargo xtask lima run -- env CARGO_HOME=/tmp/metal-run-isolation-cargo CARGO_TARGET_DIR=/tmp/metal-run-isolation-target cargo check -p overdrive-host --test integration --features integration-tests,kvm-tests --locked` returned 0. The corresponding strict Clippy command (`cargo clippy` with the same target/features/locked flags and `-- -D warnings`) was retried after preserving this file's machine-readable Contract Shape declarations using the existing justified doc-markdown convention. It remains **BLOCKED by one untouched pre-existing `clippy::doc_markdown` at `vm_host_state_equivalence.rs:172`**; no lint override or edit to that unowned file was made. The changed fixture code has no remaining reported lint. **No native test or Metal lease command was run; the existing native regression rerun remains pending focused review.** The 040806Z raw capture remains unchanged, and its missing parent stat/child errno is not reconstructed from older L/N evidence.
