# Quint evidence — `public-ingress-gateway`

Written by `cargo xtask quint check --subsystem public-ingress-gateway --record`; do not edit. `cargo xtask quint verify-evidence --subsystem public-ingress-gateway` checks it against the specs.

| | |
|---|---|
| Result | 35 of 35 check(s) matched `expect` |
| Run | `1289080-1794151150.031`, 2026-11-08T15:19:10.030953728Z → 2026-11-08T15:22:34.779206912Z |
| Jobs | 16 |
| Git | `c23aa9be55d62c0e53c02b2ac59f4ac909882b8f` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| identity-safety-tlc | gateway_identity.qnt | identity_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 6.6 | 0.0 | 6.6 |  |
| identity-run-issue-hold | gateway_identity.qnt | identity_conf | run `issue_hold` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| identity-run-reissue-near-expiry | gateway_identity.qnt | identity_conf | run `reissue_near_expiry` | evaluator | run | holds | holds | PASS | 1 | 2.1 | 0.0 | 2.1 |  |
| identity-run-disable-races-inflight-issue | gateway_identity.qnt | identity_conf | run `disable_races_inflight_issue` | evaluator | run | holds | holds | PASS | 1 | 2.1 | 0.0 | 2.1 |  |
| identity-run-audit-failure-no-hold | gateway_identity.qnt | identity_conf | run `audit_failure_no_hold` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| identity-run-restart-reissue | gateway_identity.qnt | identity_conf | run `restart_reissue` | evaluator | run | holds | holds | PASS | 1 | 2.1 | 0.0 | 2.1 |  |
| identity-hazard-no-epoch-check | hazard/gateway_identity_hazards.qnt | identity_bugNoEpochCheck | invariant `StaleEpochNeverHolds` | tlc | exhaustive | violation | violation | PASS | 1 | 7.6 | 0.0 | 7.6 | [traces/identity-hazard-no-epoch-check.txt](traces/identity-hazard-no-epoch-check.txt) [traces/identity-hazard-no-epoch-check.tlc.txt](traces/identity-hazard-no-epoch-check.tlc.txt) |
| application-safety-tlc | gateway_application.qnt | app_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| application-run-replace-valid | gateway_application.qnt | app_conf | run `replace_valid` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| application-run-route-withdraw-drain | gateway_application.qnt | app_conf | run `route_withdraw_drain` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| application-run-late-probe-after-supersession | gateway_application.qnt | app_conf | run `late_probe_after_supersession` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| application-run-failed-candidate-not-restaged | gateway_application.qnt | app_conf | run `failed_candidate_not_restaged` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| application-hazard-retire-while-leased | hazard/gateway_application_hazards.qnt | app_bugRetireWhileLeased | invariant `DrainRetainsUntilLeasesZero` | tlc | exhaustive | violation | violation | PASS | 1 | 6.8 | 0.0 | 6.8 | [traces/application-hazard-retire-while-leased.txt](traces/application-hazard-retire-while-leased.txt) [traces/application-hazard-retire-while-leased.tlc.txt](traces/application-hazard-retire-while-leased.tlc.txt) |
| application-hazard-restage-failed | hazard/gateway_application_hazards.qnt | app_bugRestageFailed | invariant `FailedCandidateNotRestaged` | tlc | exhaustive | violation | violation | PASS | 1 | 7.1 | 0.0 | 7.1 | [traces/application-hazard-restage-failed.txt](traces/application-hazard-restage-failed.txt) [traces/application-hazard-restage-failed.tlc.txt](traces/application-hazard-restage-failed.tlc.txt) |
| receipt-safety-tlc | selection_receipt.qnt | receipt_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 204.7 | 0.0 | 204.7 |  |
| receipt-run-commit-then-select | selection_receipt.qnt | receipt_conf | run `commit_then_select` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| receipt-run-abandoned-reservation-rolled-back | selection_receipt.qnt | receipt_ok | run `abandoned_reservation_rolled_back` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| receipt-run-conflicting-alloc-rejected | selection_receipt.qnt | receipt_conf | run `conflicting_alloc_rejected` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| receipt-run-reverse-nat-conflict-rejected | selection_receipt.qnt | receipt_conf | run `reverse_nat_conflict_rejected` | evaluator | run | holds | holds | PASS | 1 | 3.9 | 0.0 | 3.9 |  |
| receipt-run-identity-conflict-precedes-reverse-nat | selection_receipt.qnt | receipt_conf | run `identity_conflict_precedes_reverse_nat` | evaluator | run | holds | holds | PASS | 1 | 3.9 | 0.0 | 3.9 |  |
| receipt-run-receipt-waits-for-publish-guard | selection_receipt.qnt | receipt_ok | run `receipt_waits_for_publish_guard` | evaluator | run | holds | holds | PASS | 1 | 3.9 | 0.0 | 3.9 |  |
| receipt-run-restart-destroys-receipts | selection_receipt.qnt | receipt_conf | run `restart_destroys_receipts` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| receipt-hazard-publish-early | hazard/selection_receipt_hazards.qnt | receipt_bugPublishEarly | invariant `IdentityReadableOnlyAfterCommit` | tlc | exhaustive | violation | violation | PASS | 1 | 11.4 | 0.0 | 11.4 | [traces/receipt-hazard-publish-early.txt](traces/receipt-hazard-publish-early.txt) [traces/receipt-hazard-publish-early.tlc.txt](traces/receipt-hazard-publish-early.tlc.txt) |
| receipt-hazard-rebind | hazard/selection_receipt_hazards.qnt | receipt_bugRebind | invariant `IdentityImmutablePerBackendId` | tlc | exhaustive | violation | violation | PASS | 1 | 13.7 | 0.0 | 13.7 | [traces/receipt-hazard-rebind.txt](traces/receipt-hazard-rebind.txt) [traces/receipt-hazard-rebind.tlc.txt](traces/receipt-hazard-rebind.tlc.txt) |
| receipt-hazard-rnat-share | hazard/selection_receipt_hazards.qnt | receipt_bugRnatShare | invariant `ReverseNatKeyOneVip` | tlc | exhaustive | violation | violation | PASS | 1 | 15.7 | 0.0 | 15.7 | [traces/receipt-hazard-rnat-share.txt](traces/receipt-hazard-rnat-share.txt) [traces/receipt-hazard-rnat-share.tlc.txt](traces/receipt-hazard-rnat-share.tlc.txt) |
| receipt-hazard-take-any | hazard/selection_receipt_hazards.qnt | receipt_bugTakeAny | invariant `ReceiptMatchesIntent` | tlc | exhaustive | violation | violation | PASS | 1 | 12.4 | 0.0 | 12.4 | [traces/receipt-hazard-take-any.txt](traces/receipt-hazard-take-any.txt) [traces/receipt-hazard-take-any.tlc.txt](traces/receipt-hazard-take-any.tlc.txt) |
| relay-safety-tlc | relay_exchange.qnt | relay_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 7.3 | 0.0 | 7.3 |  |
| relay-run-exchange-cl-keepalive | relay_exchange.qnt | relay_conf | run `exchange_cl_keepalive` | evaluator | run | holds | holds | PASS | 1 | 2.1 | 0.0 | 2.1 |  |
| relay-run-early-response-closes | relay_exchange.qnt | relay_conf | run `early_response_closes` | evaluator | run | holds | holds | PASS | 1 | 2.1 | 0.0 | 2.1 |  |
| relay-run-drain-with-inflight-body | relay_exchange.qnt | relay_conf | run `drain_with_inflight_body` | evaluator | run | holds | holds | PASS | 1 | 1.5 | 0.0 | 1.6 |  |
| relay-run-half-close-after-request-read | relay_exchange.qnt | relay_conf | run `half_close_after_request_read` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| relay-run-upstream-close-mid-body-aborts | relay_exchange.qnt | relay_conf | run `upstream_close_mid_body_aborts` | evaluator | run | holds | holds | PASS | 1 | 2.1 | 0.0 | 2.1 |  |
| relay-run-close-delimited-clean-end | relay_exchange.qnt | relay_conf | run `close_delimited_clean_end` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| relay-run-public-close-mid-request-aborts | relay_exchange.qnt | relay_conf | run `public_close_mid_request_aborts` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| relay-hazard-relay-past-boundary | hazard/relay_exchange_hazards.qnt | relay_bugRelayPastBoundary | invariant `NoBytePastMessageBoundary` | tlc | exhaustive | violation | violation | PASS | 1 | 6.8 | 0.0 | 6.8 | [traces/relay-hazard-relay-past-boundary.txt](traces/relay-hazard-relay-past-boundary.txt) [traces/relay-hazard-relay-past-boundary.tlc.txt](traces/relay-hazard-relay-past-boundary.tlc.txt) |

## Inputs (sha256)

- `checks.toml` `d5bfb8773a297148d26e3f2cbca0f62fb1e104ea704c893084db92df07e12e81`
- `gateway_application.qnt` `db744ec85d077b7c046103387f2211f0b68035a6714fd54a52b745c86ebd0b5c`
- `gateway_identity.qnt` `72d5628c65cfe3ed0cd9a15f40e63e99fbf31b2fa842962aa10a80986e1138d7`
- `hazard/gateway_application_hazards.qnt` `80e63d8a0507732c34c0b7a0746be8939f3ff27cc80f870e1ebd64f469058e3e`
- `hazard/gateway_identity_hazards.qnt` `8f20fea9bd951d560500a55396dbebbaa261511c7a1381866d3876510200a253`
- `hazard/relay_exchange_hazards.qnt` `b4459b6534cb0f15548b0eb8f3e0fd80f7822e1845e2abb764219e123273f3b6`
- `hazard/selection_receipt_hazards.qnt` `68d340b854a5791510add2208d7e494e5b3db4c8eab9f8f58ce67c722def5d66`
- `relay_exchange.qnt` `fbf429a5dceb87b2e9f32617f755f78fb4f22e176e9bb0b14fae321d109c6261`
- `selection_receipt.qnt` `8fd64de37294b58506aecf4db58f1ea558fa1c43ca35a0dcb051724eaa6d57a6`
