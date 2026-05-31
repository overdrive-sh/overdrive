# Slice 08 — EarlyExit detection within startup deadline (RCA-A regression guard)

**Stories:** US-08
**Priority:** P0
**KPI:** K1 (Service-submit honesty rate — direct RCA-A close)
**Dependencies:** Slice 01

## Outcome the operator can verify

Submitting the coinflip-shaped Service spec produces an honest Failed render with exit code and stderr tail:

```
$ overdrive job submit examples/coinflip-as-service.toml
Accepted: service 'coinflip-as-service' (intent_key=service/cf, commit=44)
Service 'coinflip-as-service' failed: workload exited within startup deadline
  exit_code:    1
  elapsed:      0.05s (startup_deadline=60s)
  stderr_tail:  "ERROR"

The workload exited before any startup probe could pass. Inspect the
spec's command, environment, or listener configuration.

$ echo $?
1
```

## Adds onto Slice 01

Slice 01 introduces the `ServiceFailureReason::EarlyExit { exit_code }` variant; Slice 08:

| Component | Change |
|---|---|
| `ServiceLifecycleReconciler.reconcile()` | Add the EarlyExit-detection branch (AllocStatusRow.state == Failed AND no Pass ProbeResultRow AND elapsed < startup_deadline) |
| stderr_tail flow | Plumb the ExitObserver's captured stderr_tail through `ServiceSubmitEvent::Failed.stderr_tail` to the CLI render |
| CLI render for EarlyExit | Multi-line shape with exit_code, elapsed, stderr_tail, guidance |
| Regression fixture | `examples/coinflip-as-service.toml` lands in repo; integration test runs 100 deterministic seeds asserting ≥99 emit Failed |
| Edge case: exit 0 within deadline | Treat as EarlyExit { exit_code: 0 }; render explains Service-kind expects long-lived |

## Acceptance test additions

- Service exec exits 1 within deadline → Failed { EarlyExit { exit_code: 1 } }
- stderr_tail substring survives end-to-end (Worker → ExitObserver → wire event → CLI render)
- Service exec exits 0 within deadline → STILL EarlyExit { exit_code: 0 } (long-lived expectation)
- Service exec exits AFTER reaching Stable → NOT EarlyExit (existing restart / BackoffExhausted paths apply)
- 100-seed regression on coinflip fixture → ≥99 emit Failed; zero emit Stable; zero emit `(took live)` substring

## Demoable check

`cargo xtask lima run -- cargo nextest run -p overdrive-cli --features integration-tests -E 'test(coinflip_service_early_exit_regression)'` passes.

## Importance

This slice IS the direct regression guard for RCA root cause A — without it the K1 honesty rate test relies on Slice 01's foundation but does not exercise the specific failure shape the RCA documented.
