# Quint evidence — `public-ingress-gateway`

Written by `cargo xtask quint check --subsystem public-ingress-gateway --record`; do not edit. `cargo xtask quint verify-evidence --subsystem public-ingress-gateway` checks it against the specs.

| | |
|---|---|
| Result | 21 of 21 check(s) matched `expect` |
| Run | `3491355-1793584822.766`, 2026-11-02T02:00:22.765550336Z → 2026-11-02T02:15:24.123065856Z |
| Jobs | 16 |
| Git | `019d0c1a449de09b82793bc124282a4803c75dd7` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| identity-safety-tlc | gateway_identity.qnt | identity_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 7.8 | 0.0 | 7.8 |  |
| identity-run-issue-hold | gateway_identity.qnt | identity_conf | run `issue_hold` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| identity-run-reissue-near-expiry | gateway_identity.qnt | identity_conf | run `reissue_near_expiry` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| identity-run-disable-races-inflight-issue | gateway_identity.qnt | identity_conf | run `disable_races_inflight_issue` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| identity-run-audit-failure-no-hold | gateway_identity.qnt | identity_conf | run `audit_failure_no_hold` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| identity-run-restart-reissue | gateway_identity.qnt | identity_conf | run `restart_reissue` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| identity-hazard-no-epoch-check | hazard/gateway_identity_hazards.qnt | identity_bugNoEpochCheck | invariant `StaleEpochNeverHolds` | tlc | exhaustive | violation | violation | PASS | 1 | 7.6 | 0.0 | 7.6 | [traces/identity-hazard-no-epoch-check.txt](traces/identity-hazard-no-epoch-check.txt) [traces/identity-hazard-no-epoch-check.tlc.txt](traces/identity-hazard-no-epoch-check.tlc.txt) |
| application-safety-tlc | gateway_application.qnt | app_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 8.4 | 0.0 | 8.4 |  |
| application-run-replace-valid | gateway_application.qnt | app_conf | run `replace_valid` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| application-run-route-withdraw-drain | gateway_application.qnt | app_conf | run `route_withdraw_drain` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| application-run-late-probe-after-supersession | gateway_application.qnt | app_conf | run `late_probe_after_supersession` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| application-hazard-retire-while-leased | hazard/gateway_application_hazards.qnt | app_bugRetireWhileLeased | invariant `DrainRetainsUntilLeasesZero` | tlc | exhaustive | violation | violation | PASS | 1 | 7.1 | 0.0 | 7.1 | [traces/application-hazard-retire-while-leased.txt](traces/application-hazard-retire-while-leased.txt) [traces/application-hazard-retire-while-leased.tlc.txt](traces/application-hazard-retire-while-leased.tlc.txt) |
| receipt-safety-tlc | selection_receipt.qnt | receipt_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 901.3 | 0.0 | 901.3 |  |
| receipt-run-commit-then-select | selection_receipt.qnt | receipt_conf | run `commit_then_select` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| receipt-run-abandoned-reservation-rolled-back | selection_receipt.qnt | receipt_conf | run `abandoned_reservation_rolled_back` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| receipt-run-conflicting-alloc-rejected | selection_receipt.qnt | receipt_conf | run `conflicting_alloc_rejected` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| receipt-run-receipt-waits-for-publish-guard | selection_receipt.qnt | receipt_conf | run `receipt_waits_for_publish_guard` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| receipt-run-restart-destroys-receipts | selection_receipt.qnt | receipt_conf | run `restart_destroys_receipts` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| receipt-hazard-publish-early | hazard/selection_receipt_hazards.qnt | receipt_bugPublishEarly | invariant `IdentityReadableOnlyAfterCommit` | tlc | exhaustive | violation | violation | PASS | 1 | 9.3 | 0.0 | 9.3 | [traces/receipt-hazard-publish-early.txt](traces/receipt-hazard-publish-early.txt) [traces/receipt-hazard-publish-early.tlc.txt](traces/receipt-hazard-publish-early.tlc.txt) |
| receipt-hazard-rebind | hazard/selection_receipt_hazards.qnt | receipt_bugRebind | invariant `IdentityImmutablePerBackendId` | tlc | exhaustive | violation | violation | PASS | 1 | 13.9 | 0.0 | 13.9 | [traces/receipt-hazard-rebind.txt](traces/receipt-hazard-rebind.txt) [traces/receipt-hazard-rebind.tlc.txt](traces/receipt-hazard-rebind.tlc.txt) |
| receipt-hazard-take-any | hazard/selection_receipt_hazards.qnt | receipt_bugTakeAny | invariant `ReceiptMatchesIntent` | tlc | exhaustive | violation | violation | PASS | 1 | 12.4 | 0.0 | 12.4 | [traces/receipt-hazard-take-any.txt](traces/receipt-hazard-take-any.txt) [traces/receipt-hazard-take-any.tlc.txt](traces/receipt-hazard-take-any.tlc.txt) |

## Inputs (sha256)

- `checks.toml` `29e5d702def75291ad4efac70c4e3b332018f91ab84f3c2bf2fae8857579086d`
- `gateway_application.qnt` `dedb1ef0f88e5e8a925a25f7e3cb9f211ee5366c15a94107e423bef54fae55a0`
- `gateway_identity.qnt` `fde6d56456c2f46f63e44d3ed3e0db02df3760a5c7c19c28047f0003ff33a862`
- `hazard/gateway_application_hazards.qnt` `6c5dd96128f17a0d021a0ec3bf12ad419baab23ce0a016026fb4fa49cfaeb52c`
- `hazard/gateway_identity_hazards.qnt` `319bdc88ab2a970a45444c8abf6c30dacba1abdc45d1e956e52db121d402b432`
- `hazard/selection_receipt_hazards.qnt` `cde0fc712500edd536cb0ddee6b7515ceab338663d542ac39ade836eba38af59`
- `selection_receipt.qnt` `0f3b830786f5bc2ba165507135beee53fe3fc3a8272a3be24a7877c635a2b020`
