# Independent DISTILL review — D7 startup give-up model fidelity

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | Iteration 1: bounded `restart_authority.qnt` `WLGiveUp` row-class correction and Rust conformance-driver comment. Iteration 2: typed startup-stop oracle in the existing S-54 acceptance helper. |
| Author record | `startup-give-up-model-remediation.md` and retained `.context/step-02-02-distill-*` Metal logs |
| Protected work | Original step 02-02 crafter's production changes, activated tests, DES phases and commit |
| Current verdict | **APPROVED after iteration 2** — both the model correction and the bounded D7 acceptance-oracle correction preserve their approved outcomes |

## Reproduced mismatch and exact owner path

At `QUINT_SEED=0x50d50042`, two named conformance runs failed on the same `WLGiveUp` transition: `startup_never_passes_loop_gives_up_startup_probe_failed` and `crash_then_startup_stop_names_last_cause`. The retained pre-change log (`.context/step-02-02-distill-seed-before.log:1243–1268`) shows the only projected difference: model `row: "t_startup"`, production projection `row: "failed_other"`. The model and driver already agreed on `claim: "startup_probe_failed"`, `author: "workload_lifecycle"`, terminal membership, acted row, restart count and `done`. This is a reproducible abstraction mismatch, not a missing or renamed operator verdict.

The approved D7 owner path is precise. `ServiceLifecycle` emits `StopAllocation { terminal: Stopped { StartupProbe } }` for the live failed startup gate; the action shim stops the allocation and writes a `Terminated` row carrying that typed disposition. That row is restartable and consumes the shared budget. At the ceiling, `WorkloadLifecycle` reads the **final attempt's** `Terminated`/`Stopped { StartupProbe }` row and emits `FinalizeFailed { terminal: ServiceFailed { StartupProbeFailed { .. } } }` (feature delta D7 item 3). The production `FinalizeFailed` arm treats `ServiceFailed` as a genuine failure, builds the successor status row with `AllocState::Failed`, forwards the prior typed reason and detail, and writes the terminal unchanged (`action_shim/mod.rs:1962–2069,2080–2115`). The approved `WorkloadLifecycle` give-up author and typed `StartupProbeFailed` terminal are separate facts from that row's new state.

The conformance driver still drives the production workload tick through hydration, `reconcile`, view persistence and action-shim dispatch (`quint_restart_authority.rs:400–482,678–681`). Its unchanged `row_class` reads the actual `AllocStatusRow`: a `Failed` row with a non-crash reason projects to `failed_other`, while `claim_of` and the returned-action trace independently project the typed terminal and author (`:760–799,860–890`). The driver did not manufacture or rewrite an observation row. Its only code edit in this remediation is a comment explaining the production projection.

## Model change and no-weakening audit

`WLGiveUp` now computes `claim: classify(at.get(cur))` from the **prior** final-attempt row, then records the action shim's persisted row class on that same attempt (`restart_authority.qnt:348–365`). A prior `failed_crash` stays `failed_crash` because the shim forwards its typed crash reason. Every other restartable prior class, including `t_startup`, becomes `failed_other` after the failure terminal write. The model still requires `claim == startup_probe_failed` and `author == workload_lifecycle` for the D7 path; the new row class cannot turn a wrong terminal into a passing trace. Earlier attempts, `classify` priority, `giveUpEnabled`, budget, `done`, D6/DG-12 whole-workload terminal membership, stop faults and the safety/liveness predicates are unchanged. The change adds no state field, actor, transition, policy or production/test seam.

The correction preserves the test's ability to detect the important wrong outcomes: a startup stop that was never written remains a row mismatch before give-up; a wrong give-up cause or author remains a `claim`/`author` mismatch; a finalizer that leaves the row `Terminated` remains a row-class mismatch. The two targeted tests were not removed, ignored, or given a string-only terminal assertion. The original D6 whole-workload acknowledgement correction remains in the model (`acknowledgeTerminalFacts` on ServiceLifecycle ticks) and in the unchanged driver projection.

## Verification and disposition

| Evidence | Result |
|---|---|
| Both original named runs, seeded before | **0/2 passed** at the precise `WLGiveUp` row-class difference; the log prints `0x50d50042`. |
| Both seeded after | **2/2 passed** (`.context/step-02-02-distill-seed-after.log`). |
| Seven affected D7 named conformance runs | **7/7 passed** (`.context/step-02-02-distill-named-after.log`). |
| Full formal check and recorded evidence | Metal `cargo xtask quint check --subsystem service-allocation-readiness-and-ending --record` passed **84/84**; evidence was pulled into this checkout (`.context/step-02-02-distill-quint-record.log`, `-quint-pull.log`). I compared current `summary.json` to the prior committed summary: all 84 check names and their property, expected and actual hold/violation outcomes are unchanged. The current summary records 84 passes and all 13 input SHA-256 values match current spec files. |
| Evidence verification | Metal `cargo xtask quint verify-evidence --subsystem service-allocation-readiness-and-ending` passed (`.context/step-02-02-distill-quint-verify.log`). |
| Static scope | The author reports changed-file `rustfmt --check`, JSON parse and `git diff --check` passing. My read-only `git diff --check`, JSON parse and hash comparison passed. `.cargo/config.toml` is clean after the temporary seed configuration was restored. |

**Iteration-1 verdict: APPROVED for the bounded DISTILL model-fidelity correction.** The accepted D7 finalizer changes the persisted row state to `Failed`, and the model now records that state while keeping `StartupProbeFailed` and `WorkloadLifecycle` as independent exact oracles. This did not approve the original step 02-02 production implementation or replace its GREEN and commit work.

## Iteration 2 — D7 startup-stop gate control oracle

The original step 02-02 crafter's wider relevant run exposed a failing unchanged TCP/HTTP control. The pre-change `stop_emitted` helper in `service_lifecycle_startup_stop_gate.rs:259–270` looked for the debug text `StartupProbeFailed` in the actions returned by `ServiceLifecycleReconciler::reconcile`. That name belonged to the **old live-allocation `FinalizeFailed`** path. Under approved D7 item 2, the live startup gate returns `Action::StopAllocation { terminal: Some(Stopped { by: StartupProbe }) }`; only `WorkloadLifecycle` names `StartupProbeFailed` when the restart budget is spent. The Metal control failed specifically at TCP `Fail(3)` and elapsed 60 seconds before any oracle edit (`.context/step-02-02-distill-gate-control-before.log`). The failure is reachable through the existing hydration and production reconciler call in the helper, and the changed oracle is necessary to observe the accepted action rather than a debug string from a deleted action.

The revised helper still writes a Running Service row and latest Startup probe row through the observation port, hydrates the ServiceLifecycle states for the workload target, and calls the production `ServiceLifecycleReconciler::reconcile` (`service_lifecycle_startup_stop_gate.rs:194–267`). It now matches the target allocation's exact `StopAllocation { Stopped { StartupProbe } }`, rejects any stop with a wrong terminal, rejects the obsolete live `FinalizeFailed { ServiceFailed { StartupProbeFailed } }`, and requires at most one startup stop. It also requires the returned `ServiceLifecycleView.terminal_announced` membership to equal whether that stop was emitted (`:268–299`). This is a stricter typed action-and-view oracle. It permits the legitimate `FinalizeFailed { Stable }` on a startup `Pass`; the author's first draft rejected it and was corrected after a focused pending-property run exposed the mistake (`.context/step-02-02-distill-gate-pending-red.log`).

The `Mechanic × Latest × elapsed` property and `expected_stop` function are unchanged. The unignored TCP/HTTP golden still checks streaks 1 and 2 at the deadline, streak 3 before the deadline, and streak 3 at the deadline. The two D14/P1 GuestListeners bodies keep their `#[ignore]` markers and the same `Fail(1)` at 60 seconds expectation. The final helper does not make those future obligations pass: the focused ignored run remains right-reason RED at that exact GuestListeners cell, with a minimal proptest counterexample (`.context/step-02-02-distill-gate-pending-red-final.log`). No production code, Quint model, formal evidence or DESIGN outcome was changed by this oracle correction.

| Focused evidence | Result |
|---|---|
| TCP/HTTP control before / after | Failed on TCP at `max_attempts` and deadline, then **1/1 passed** after the exact typed oracle (`.context/step-02-02-distill-gate-control-before.log`, `-gate-control-final.log`). |
| Pending D14/P1 GuestListeners bodies | **0/2 passed as expected RED** on `GuestListeners`, `Fail(1)`, 60 seconds; the pending markers and input grid remain intact. |
| Static gate | Metal `cargo clippy -p overdrive-control-plane --test acceptance --features integration-tests -- -D warnings` passed (`.context/step-02-02-distill-gate-clippy.log`); author's changed-file rustfmt and `git diff --check` passed. My read-only `git diff --check` passed. |

**Iteration-2 verdict: APPROVED.** The helper now observes the approved D7 startup stop through the real reconciler, with exact typed terminal and view membership; TCP/HTTP threshold complements remain green and future GuestListeners threshold obligations remain RED. The original step 02-02 crafter may rerun its own GREEN gate and commit. This bounded DISTILL approval is not a DELIVER implementation review, DES event, mutation run or commit.
