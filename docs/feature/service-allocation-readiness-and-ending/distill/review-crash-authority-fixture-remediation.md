# Independent DISTILL review — D6 crash-authority fixture remediation

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | Test/support fallout from the approved D6 deletion of `ServiceAllocFact.exit_code` and `ServiceLifecycle`'s EarlyExit branch |
| Author record | `crash-authority-fixture-remediation.md`; retained `.context/step-02-01-distill-*` Metal logs |
| Protected work | Original step 02-01 crafter's dirty production edits, activated acceptance and conformance tests, RED/GREENFAIL DES record and eventual commit |
| Current verdict | **APPROVED after iteration 2** for the bounded DISTILL fixture, acceptance-oracle and Quint model-fidelity revisions; step 02-01 implementation and commit remain the original crafter's |

## Necessity and exact D6 contract

The approved feature delta D6 item 1 and its interface table delete `ServiceAllocFact.exit_code` and its ServiceLifecycle hydration. A `Failed` or `Terminated` Service allocation fact now produces no `ServiceLifecycle` action and joins `ServiceLifecycleView.terminal_announced` regardless of `started_at` (DG-12). `WorkloadLifecycle` classifies the final attempt at give-up from the durable `AllocStatusRow.reason` and `current_startup_probe`; its `ServiceFailureReason::EarlyExit { exit_code }` wire shape remains. A failed probe-result read retains the existing `HydrateError::ObservationRead` distinction. No new public or typed-error contract was needed for this test work.

The old fixtures could not compile after the approved field deletion. The retained pre-remediation Metal log (`.context/step-02-01-distill-compile-before.log:34–140`) has **13 E0560/E0609 errors**: 11 obsolete `ServiceAllocFact { exit_code: ... }` members and two reads of `fact.exit_code`. This is direct compiler fallout of the approved interface change, not a new behavior or API proposal.

## Test honesty, subtraction and retained outcomes

| Changed evidence | Review disposition |
|---|---|
| `service_lifecycle_hydrate.rs` GAP-1-AT-07 | **Strengthened to the new owner contract.** For a `Failed` fact with `started_at: None`, it now requires an empty action vector and `terminal_announced` membership. The old check only rejected two specific finalizations; the new assertion rejects every ServiceLifecycle action. The `max_attempts: 0` fixture still makes the startup-gate conflict meaningful. |
| Former GAP-11 hydration projection test | **Correctly relocated the typed cause.** Three cases remain: crash with `Some(1)`, signal-only crash with `None`, and non-crash `Stopped { Process }`. The renamed body checks hydrated `Failed`/`started_at` and the exact typed `AllocStatusRow.reason` via `ObservationStore::alloc_status_row`; it no longer asserts a deleted fact copy. This body alone does not claim to prove `WorkloadLifecycle`'s ceiling choice. The separate S-15 selection and S-16 composed crash-loop obligations remain active and retain that oracle; one S-16 stderr-tail failure remains visibly RED. |
| `service_lifecycle_stable.rs` terminal-after-Stable case | **Strengthened to D6/DG-12.** Its Failed fact no longer takes an obsolete exit-code argument. It still requires no ServiceLifecycle action and now requires `terminal_announced` membership. The existing Stable, dedup and startup-stop complements remain. |
| `service_lifecycle_reconcile_branches.rs` | **Obsolete branch tests removed without losing the new terminal rule.** The two deleted `early_exit_*` tests targeted the `no_pass`/deadline predicates of the EarlyExit branch D6 explicitly deletes. S-56's live 24-cell Failed/Terminated × start-history × latest-probe × declared-probe grid requires no action, terminal membership, a closed mid-startup window, no next evaluation and idempotence; its `Running` control preserves the unaffected case. The S-21 state/streak/deadline property remains pre-authored for the later D5/D7 step. The private `fact` helper and calls lose only the obsolete exit-code argument. The corresponding two Metal placement entries were deleted. |
| Remaining ServiceAllocFact literals and hydration golden | **Neutral compiler/fixture fallout.** The affected control-plane, core, reconciler and sim test literals lose only the deleted member. `fixtures/hydration_golden/service_lifecycle.txt` loses exactly `exit_code: None`; its other fields and the six other reconciler goldens remain unchanged. Typed `TransitionReason` and `TerminalCondition` exit-code payloads are retained. |

The changed tests retain per-test Contract Shape declarations. The D6 acceptance markers activated by the original crafter were not altered by DISTILL. The test revision keeps the no-action/membership rule, current-attempt typed-cause classification and unaffected `Running` behavior as independent evidence; it does not change a production branch or replace an observable outcome with a fixture-only assertion.

## Verification and remaining step work

| Retained Metal evidence | Result |
|---|---|
| Initial focused control-plane compilation | Failed on the 13 obsolete-field diagnostics, before any test body ran. |
| Focused ServiceLifecycle hydration and Stable acceptance | **18/18 passed** after the fixture revision (`.context/step-02-01-distill-cp-focused.log`). |
| Core/reconciler/sim fixture selection | **18/18 passed** (`.context/step-02-01-distill-cross-crate.log`). |
| Other affected control-plane acceptance modules | **24/24 passed** (`.context/step-02-01-distill-unaffected.log`). |
| Core reconcile-branches suite after subtraction | **33/33 passed** (`.context/step-02-01-distill-reconcile-branches-after.log`). |
| Hydration characterization golden | Initially failed only on the removed `exit_code: None` line; **1/1 passed** after the one-line update (`.context/step-02-01-distill-golden-before.log`, `-golden-after.log`). |
| Activated D6/B3/DG-12 selection | **15/17 passed, two remain failing**: S-16's stderr-tail assertion and `restart_authority_pre_startup_crash_then_successor_passes` at seed `0x3fbbc5ad` (`.context/step-02-01-distill-activated.log`). Neither was suppressed or reclassified as a fixture failure. |
| Static checks | Author's changed-test `rustfmt --check`, JSON parse and `git diff --check` passed. Strict Metal Clippy remains **failed** on the original crafter's production `workload_lifecycle.rs:1165` `clippy::unnested_or_patterns`; a rerun allowing only that lint passed across the affected crates and all targets (`.context/step-02-01-distill-clippy.log`, `-clippy-scoped-final.log`). The scoped pass is not a clean step gate. |

I reviewed the approved D6 contract, the affected test/support diffs and retained Metal logs. I did not rerun cargo or mutation, edit production/tests/DES, or commit. At this **iteration-1** handoff the original step 02-01 crafter still had two semantic failures and a production Clippy error; iteration 2 below records their subsequent disposition.

**Iteration-1 verdict: APPROVED for bounded DISTILL fixture remediation only.** The obsolete field and branch expectations were removed because the accepted D6 contract deletes them; the replacement assertions and retained S-56/S-15/S-16/S-21 evidence preserve the required behavior. This did not approve step 02-01 implementation or turn its then-failing acceptance/conformance runs green.

## Iteration 2 — terminal-hook oracle and whole-workload model fidelity

The author reproduced the two remaining failures before editing test support. The production stderr-tail projection and its Clippy lint were subsequently fixed by the original step crafter; this review evaluates only the DISTILL assertion, Quint abstraction and recorded evidence changes. The original crafter's production files, activated markers, DES events and commit ownership remain outside this review.

### F-1 — the finalizer's retained terminal hook is a second call, one effect

**Before:** `.context/step-02-01-distill-hooks-before.log` shows the composed `pre_startup_crash_loop_ends_the_stream_with_early_exit_at_give_up` failing with seven `on_alloc_terminal` calls where its old oracle expected six. The real path is the six `ExitEvent`s consumed by `exit_observer::spawn_with_runtime`, each calling `Driver::on_alloc_terminal` before its observation write (`exit_observer.rs:204–226`), followed by the final `WorkloadLifecycle` `FinalizeFailed` dispatched by the action shim, which retains its own hook (`action_shim/mod.rs:2162–2181`). Approved D4 expressly keeps that finalizer call; D2 makes the repeated call for the final allocation idempotent. D6 removes a competing **ServiceLifecycle** finalization, not the `WorkloadLifecycle` finalizer's hook.

**After:** the same existing S-16 body now asserts **six** observer calls and zero active probe supervisors before the give-up. After give-up it inspects the actual `HookedDriver` trace: each predecessor has one terminal-hook call, the final allocation has two, seven total, while `ProbeRunner::active_alloc_count()` remains zero (`crash_ending_authority.rs:157–199`). The typed EarlyExit stream verdict, stderr tail, one final terminal row and predecessor crash-row checks remain. This raises the oracle's precision and preserves the D1/D2/D4 effects; it neither suppresses the extra call nor changes production behavior. The focused Metal replay passes (`.context/step-02-01-distill-hooks-after.log`).

### F-2 — one ServiceLifecycle tick acknowledges every terminal allocation fact

**Before:** the retained `QUINT_SEED=0x3fbbc5ad` trace diverged at `SLStable(1)`: the model left predecessor attempt 0 `ann=false`, while the driver projected `ann=true` from the real `ServiceLifecycleView.terminal_announced` (`.context/step-02-01-distill-quint-before.log`). The driver action `sl_tick(i, None)` targets the **workload**, not just attempt `i`: it hydrates desired and actual state for that target, calls the registered production `ServiceLifecycleReconciler::reconcile`, persists its returned view and dispatches returned actions (`quint_restart_authority.rs:400–482,662–670`). The reconciler iterates `actual.allocs` and adds every `Failed` or `Terminated` fact to `terminal_announced` before evaluating a live successor's Stable branch (`service_lifecycle.rs:560–574`). That is the exact D6/DG-12 contract; the projected view was honest.

**After:** `restart_authority.qnt:225–309` adds `acknowledgeTerminalFacts` to the four ServiceLifecycle tick actions (`SLStable`, `SLStartupStop`, `SLLivenessStop`, `SLAnnounceTerminal`). It marks `ann=true` only for rows other than `none` or `running`, the model's terminal row classes. The targeted live attempt retains each action's original guard and action-specific result; no new terminal, author, claim, stop or restart policy is introduced. `SLAnnounceTerminal` now records the entire workload tick's effect instead of one chosen terminal fact. The Rust conformance driver changed only its action-map comment; its production call, row/view projection and injected faults did not change. This models the accepted whole-workload owner transition and makes the projection stricter for terminal predecessors. The original seed passes after the correction (`.context/step-02-01-distill-quint-after2.log`).

### Evidence integrity and final disposition

| Check | Observed result and limit |
|---|---|
| Activated step 02-01 selection | The same 17 acceptance/named conformance identities now pass **17/17** (`.context/step-02-01-distill-activated-after2.log`); neither previously failing test was ignored, removed or given a weaker outcome. |
| Formal model | Full check and recorded rerun report **84/84 expected results** (`.context/step-02-01-distill-quint-full.log`, `-quint-record.log`). The current `evidence/summary.json` has the same 84 check names and the same expected/actual holds-or-violation outcomes as its predecessor; only run metadata, measured values, input hashes and generated traces were refreshed. All **13** recorded spec-input SHA-256 values match the current inputs. |
| Evidence verification | `cargo xtask quint verify-evidence --subsystem service-allocation-readiness-and-ending` first rejected the stale prior `restart_authority.qnt` hash, then passed after the full rerun with `--record` and evidence pull (`.context/step-02-01-distill-quint-evidence-before.log`, `-quint-evidence-after.log`). |
| Lint and formatting | Strict Metal `cargo clippy -p overdrive-control-plane --all-targets --features integration-tests -- -D warnings` passed after the crafter's production lint fix (`.context/step-02-01-distill-clippy-iteration2.log`). Author's changed-file formatting and `git diff --check` passed; the workspace fmt failure remains in two unrelated untouched files and is not claimed clean. My read-only `git diff --check`, evidence JSON parse and hash comparison passed. |
| Scope | The temporary `QUINT_SEED`/verbosity configuration was restored; `.cargo/config.toml` is clean. No new public API, Sim seam, production mechanism, policy, DES event or commit was introduced by DISTILL or this review. |

**F-1 and F-2: resolved. Final bounded DISTILL verdict: APPROVED.** The corrected test oracle counts the accepted D4 finalizer call while requiring the D2 idempotent probe-supervision effect. The model correction reflects the already approved D6/DG-12 whole-workload terminal-membership rule and keeps production-driven conformance projection and hazards intact. The original step 02-01 crafter may resume its own checks and commit; this review does not approve its production diff or substitute for the required fresh DELIVER step review.
