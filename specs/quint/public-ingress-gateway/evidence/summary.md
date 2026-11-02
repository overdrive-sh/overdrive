# Quint evidence — `public-ingress-gateway`

Written by `cargo xtask quint check --subsystem public-ingress-gateway --record`; do not edit. `cargo xtask quint verify-evidence --subsystem public-ingress-gateway` checks it against the specs.

| | |
|---|---|
| Result | 33 of 33 check(s) matched `expect` |
| Run | `3622205-1793633298.676`, 2026-11-02T15:28:18.675664128Z → 2026-11-02T15:44:01.705299712Z |
| Jobs | 16 |
| Git | `019d0c1a449de09b82793bc124282a4803c75dd7` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| identity-safety-tlc | gateway_identity.qnt | identity_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 8.1 | 0.0 | 8.1 |  |
| identity-run-issue-hold | gateway_identity.qnt | identity_conf | run `issue_hold` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| identity-run-reissue-near-expiry | gateway_identity.qnt | identity_conf | run `reissue_near_expiry` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| identity-run-disable-races-inflight-issue | gateway_identity.qnt | identity_conf | run `disable_races_inflight_issue` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| identity-run-audit-failure-no-hold | gateway_identity.qnt | identity_conf | run `audit_failure_no_hold` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| identity-run-restart-reissue | gateway_identity.qnt | identity_conf | run `restart_reissue` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| identity-hazard-no-epoch-check | hazard/gateway_identity_hazards.qnt | identity_bugNoEpochCheck | invariant `StaleEpochNeverHolds` | tlc | exhaustive | violation | violation | PASS | 1 | 6.8 | 0.0 | 6.8 | [traces/identity-hazard-no-epoch-check.txt](traces/identity-hazard-no-epoch-check.txt) [traces/identity-hazard-no-epoch-check.tlc.txt](traces/identity-hazard-no-epoch-check.tlc.txt) |
| application-safety-tlc | gateway_application.qnt | app_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 6.8 | 0.0 | 6.8 |  |
| application-run-replace-valid | gateway_application.qnt | app_conf | run `replace_valid` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| application-run-route-withdraw-drain | gateway_application.qnt | app_conf | run `route_withdraw_drain` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| application-run-late-probe-after-supersession | gateway_application.qnt | app_conf | run `late_probe_after_supersession` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| application-hazard-retire-while-leased | hazard/gateway_application_hazards.qnt | app_bugRetireWhileLeased | invariant `DrainRetainsUntilLeasesZero` | tlc | exhaustive | violation | violation | PASS | 1 | 7.1 | 0.0 | 7.1 | [traces/application-hazard-retire-while-leased.txt](traces/application-hazard-retire-while-leased.txt) [traces/application-hazard-retire-while-leased.tlc.txt](traces/application-hazard-retire-while-leased.tlc.txt) |
| receipt-safety-tlc | selection_receipt.qnt | receipt_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 943.0 | 0.0 | 943.0 |  |
| receipt-run-commit-then-select | selection_receipt.qnt | receipt_conf | run `commit_then_select` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| receipt-run-abandoned-reservation-rolled-back | selection_receipt.qnt | receipt_fine | run `abandoned_reservation_rolled_back` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| receipt-run-conflicting-alloc-rejected | selection_receipt.qnt | receipt_conf | run `conflicting_alloc_rejected` | evaluator | run | holds | holds | PASS | 1 | 3.8 | 0.0 | 3.8 |  |
| receipt-run-receipt-waits-for-publish-guard | selection_receipt.qnt | receipt_fine | run `receipt_waits_for_publish_guard` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| receipt-run-restart-destroys-receipts | selection_receipt.qnt | receipt_conf | run `restart_destroys_receipts` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| receipt-hazard-publish-early | hazard/selection_receipt_hazards.qnt | receipt_bugPublishEarly | invariant `IdentityReadableOnlyAfterCommit` | tlc | exhaustive | violation | violation | PASS | 1 | 10.9 | 0.0 | 10.9 | [traces/receipt-hazard-publish-early.txt](traces/receipt-hazard-publish-early.txt) [traces/receipt-hazard-publish-early.tlc.txt](traces/receipt-hazard-publish-early.tlc.txt) |
| receipt-hazard-rebind | hazard/selection_receipt_hazards.qnt | receipt_bugRebind | invariant `IdentityImmutablePerBackendId` | tlc | exhaustive | violation | violation | PASS | 1 | 15.2 | 0.0 | 15.2 | [traces/receipt-hazard-rebind.txt](traces/receipt-hazard-rebind.txt) [traces/receipt-hazard-rebind.tlc.txt](traces/receipt-hazard-rebind.tlc.txt) |
| receipt-hazard-take-any | hazard/selection_receipt_hazards.qnt | receipt_bugTakeAny | invariant `ReceiptMatchesIntent` | tlc | exhaustive | violation | violation | PASS | 1 | 13.7 | 0.0 | 13.7 | [traces/receipt-hazard-take-any.txt](traces/receipt-hazard-take-any.txt) [traces/receipt-hazard-take-any.tlc.txt](traces/receipt-hazard-take-any.tlc.txt) |
| relay-safety-tlc | relay_exchange.qnt | relay_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 6.8 | 0.0 | 6.8 |  |
| relay-ktls-safety-tlc | relay_ktls.qnt | ktls_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 6.9 | 0.0 | 6.9 |  |
| relay-run-exchange-cl-keepalive | relay_exchange.qnt | relay_conf | run `exchange_cl_keepalive` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| relay-run-early-response-closes | relay_exchange.qnt | relay_conf | run `early_response_closes` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| relay-run-drain-with-inflight-body | relay_exchange.qnt | relay_conf | run `drain_with_inflight_body` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| relay-run-half-close-after-request-read | relay_exchange.qnt | relay_conf | run `half_close_after_request_read` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| relay-run-upstream-close-mid-body-aborts | relay_exchange.qnt | relay_conf | run `upstream_close_mid_body_aborts` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| relay-run-close-delimited-clean-end | relay_exchange.qnt | relay_conf | run `close_delimited_clean_end` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| relay-run-public-close-mid-request-aborts | relay_exchange.qnt | relay_conf | run `public_close_mid_request_aborts` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| relay-run-keyupdate-waits-for-writer | relay_ktls.qnt | ktls_ok | run `keyupdate_waits_for_writer` | evaluator | run | holds | holds | PASS | 1 | 1.3 | 0.0 | 1.3 |  |
| relay-hazard-relay-past-boundary | hazard/relay_exchange_hazards.qnt | relay_bugRelayPastBoundary | invariant `NoBytePastMessageBoundary` | tlc | exhaustive | violation | violation | PASS | 1 | 6.8 | 0.0 | 6.8 | [traces/relay-hazard-relay-past-boundary.txt](traces/relay-hazard-relay-past-boundary.txt) [traces/relay-hazard-relay-past-boundary.tlc.txt](traces/relay-hazard-relay-past-boundary.tlc.txt) |
| relay-ktls-hazard-rekey-during-write | hazard/relay_ktls_hazards.qnt | ktls_bugRekeyDuringWrite | invariant `TxRekeyExcludesWriters` | tlc | exhaustive | violation | violation | PASS | 1 | 6.1 | 0.0 | 6.1 | [traces/relay-ktls-hazard-rekey-during-write.txt](traces/relay-ktls-hazard-rekey-during-write.txt) [traces/relay-ktls-hazard-rekey-during-write.tlc.txt](traces/relay-ktls-hazard-rekey-during-write.tlc.txt) |

## Inputs (sha256)

- `checks.toml` `78d9425899c433a1eb8335546076623a9d748c4e53a1ca16c8be4c6a3462ea55`
- `gateway_application.qnt` `dedb1ef0f88e5e8a925a25f7e3cb9f211ee5366c15a94107e423bef54fae55a0`
- `gateway_identity.qnt` `fde6d56456c2f46f63e44d3ed3e0db02df3760a5c7c19c28047f0003ff33a862`
- `hazard/gateway_application_hazards.qnt` `6c5dd96128f17a0d021a0ec3bf12ad419baab23ce0a016026fb4fa49cfaeb52c`
- `hazard/gateway_identity_hazards.qnt` `319bdc88ab2a970a45444c8abf6c30dacba1abdc45d1e956e52db121d402b432`
- `hazard/relay_exchange_hazards.qnt` `b4459b6534cb0f15548b0eb8f3e0fd80f7822e1845e2abb764219e123273f3b6`
- `hazard/relay_ktls_hazards.qnt` `778d63f631c43e742081a17f40cd3cedb2a977877b0c8f532a2126dba0e937c3`
- `hazard/selection_receipt_hazards.qnt` `cde0fc712500edd536cb0ddee6b7515ceab338663d542ac39ade836eba38af59`
- `relay_exchange.qnt` `706686a5e0b663c824625521bbd3068fdc431160e60372cbf23ac00e86f4afd3`
- `relay_ktls.qnt` `8e8904c7f399bb2c98b81a7cf0fa7b4f6ae36060817d37747f24eec2d0ea270f`
- `selection_receipt.qnt` `ece0430f8a0ee0002a2f560b9a8137ec22dbabe6f0d5c5e64d17ec565de83a42`
