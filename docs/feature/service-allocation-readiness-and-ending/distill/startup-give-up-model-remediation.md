# D7 startup give-up model fidelity remediation

Status: **bounded DISTILL model/projection correction, pending independent acceptance review and the original step 02-02 crafter's continuation**. The current production implementation, activated acceptance markers, typed terminal oracles, and DES events were not changed by this remediation.

## Reproduced mismatch and approved owner path

The exact `QUINT_SEED=0x50d50042` replay failed for both `restart_authority_startup_never_passes_loop_gives_up_startup_probe_failed` and `restart_authority_crash_then_startup_stop_names_last_cause` through the real control-plane composition (`.context/step-02-02-distill-seed-before.log`). At `WLGiveUp`, all projected fields except the final attempt's abstract row class agreed. The model kept `t_startup`; the driver read the production `AllocStatusRow` and projected `failed_other`. In both runs, `claim == "startup_probe_failed"`, `author == "workload_lifecycle"`, `ann`, `acted`, restart count and `done` already agreed. This was a one-field model-fidelity difference, not an incorrectly named terminal.

Approved D7 item 3 makes `WorkloadLifecycle` the only give-up author. It sees a `Terminated` final attempt whose own terminal is `Stopped { by: StartupProbe }`, then returns `FinalizeFailed { terminal: ServiceFailed { StartupProbeFailed { ... } } }`. The production `FinalizeFailed` action-shim arm sets **every failure give-up row's state to `Failed`**, forwards the prior typed `reason` and detail, and writes the typed give-up terminal. Its prior `Stopped { StartupProbe }` disposition is the input that selects the new typed terminal; the final row is no longer `Terminated`. In the conformance driver, `row_class` correctly maps that `Failed` row with a non-crash reason to `failed_other`. The separate `claim` and `author` projections retain and assert the exact `StartupProbeFailed` terminal and its owner. Filtering the final row or projecting it as `t_startup` would conceal the actual persisted state.

## Model correction

Only `specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt` and the existing conformance driver's explanatory row-class comment changed. `WLGiveUp` now projects the action shim's already approved state transition: a prior `failed_crash` retains that class because the crash reason is forwarded; any other restartable prior class (`failed_other`, `t_startup`, `t_liveness` or `t_clean`) becomes `failed_other` when the failure terminal is written. The `classify` result, typed `claim`, `author`, guards, restart policy, budget, prior attempts, D6/DG-12 whole-workload terminal membership, and every safety/liveness invariant are unchanged. No production code, new API, Sim seam, policy or state field was added.

| Verification | Result |
|---|---|
| Metal replay of the two named runs at `QUINT_SEED=0x50d50042` before correction | 0/2 passed; both mismatched only final `row: t_startup` versus `failed_other` at `WLGiveUp`. |
| Same two seeded runs after correction | **2/2 passed** (`.context/step-02-02-distill-seed-after.log`). |
| All seven D7 named `restart_authority` conformance runs, ordinary Metal lane | **7/7 passed** (`.context/step-02-02-distill-named-after.log`). |
| Full feature Quint check with `--record` on Metal | **84/84 expected results passed**; generated evidence pulled into this checkout (`.context/step-02-02-distill-quint-record.log`, `...-pull.log`). |
| Metal `cargo xtask quint verify-evidence --subsystem service-allocation-readiness-and-ending` | Passed: all **13 current input hashes** match and every recorded check met its expected result (`.context/step-02-02-distill-quint-verify.log`). |
| `rustfmt --check` on the changed Rust driver, evidence JSON parse and `git diff --check` | Passed. |

The temporary Cargo seed/verbosity configuration was restored byte-for-byte; `.cargo/config.toml` is clean. The original step 02-02 crafter retains ownership of its GREEN rerun and COMMIT after independent acceptance review. No mutation testing, DES phase event or commit was created by DISTILL.

## Iteration 2 — D7 startup-stop gate control oracle

The active step 02-02 crafter's broader relevant control run found one failing pre-existing control: `service_lifecycle_startup_stop_gate.rs::dial_mechanics_stop_only_at_max_attempts`. DISTILL reproduced it through the fixture's production `ServiceLifecycleReconciler::reconcile` driving call on Metal (`.context/step-02-02-distill-gate-control-before.log`): TCP at its declared `max_attempts` and startup deadline emitted the approved D7 stop, but `stop_emitted` returned false. The helper searched `format!("{action:?}")` for `StartupProbeFailed`, the old live-allocation `FinalizeFailed` terminal. Approved D7 item 2 instead emits `Action::StopAllocation { terminal: Some(Stopped { by: StartupProbe }) }` for a live failed startup; `WorkloadLifecycle` alone produces `StartupProbeFailed` at give-up. The failure was an obsolete oracle, not a regression in the TCP/HTTP threshold.

Only that helper's oracle and its explanatory module comment changed. It now recognizes the target allocation's exact typed `StopAllocation { Stopped { StartupProbe } }`, rejects a wrong stop terminal or the deleted live `FinalizeFailed { ServiceFailed { StartupProbeFailed } }`, requires at most one startup stop per evaluation, and checks that the returned `terminal_announced` membership agrees with that emission. The existing `Mechanic × Latest × elapsed` property, TCP/HTTP threshold golden cases, GuestListeners pending markers, input setup and `expected_stop` grid remain unchanged. A first draft of the typed rejection was too broad and rejected a legitimate `FinalizeFailed { Stable }` on a `Pass`; the focused pending-property run exposed it. The final oracle rejects only the obsolete **StartupProbeFailed** finalizer, preserving the Stable complement.

| Evidence | Result |
|---|---|
| Focused TCP/HTTP control before the oracle change | Failed at `Tcp`, streak 3, elapsed 60 s because the debug-string search missed the typed stop. |
| Focused TCP/HTTP control after the final typed oracle | **1/1 passed** (`.context/step-02-02-distill-gate-control-final.log`). |
| Two still-ignored D14/P1 GuestListeners scenarios after the change | Both remain right-reason RED at `GuestListeners`, Fail streak 1, elapsed 60 s; the property shrinks to that exact cell (`.context/step-02-02-distill-gate-pending-red-final.log`). No marker was removed. |
| Metal `cargo clippy -p overdrive-control-plane --test acceptance --features integration-tests -- -D warnings` | Passed (`.context/step-02-02-distill-gate-clippy.log`). |
| Changed-file `rustfmt --check` and `git diff --check` | Passed. |

No production code, Quint model, formal evidence, DES event or commit changed in this iteration. The independent acceptance reviewer should check the final typed oracle before the original crafter reruns its GREEN gate and commits step 02-02.
