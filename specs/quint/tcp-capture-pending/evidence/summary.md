# Quint evidence — `tcp-capture-pending`

Written by `cargo xtask quint check --subsystem tcp-capture-pending --record`; do not edit. `cargo xtask quint verify-evidence --subsystem tcp-capture-pending` checks it against the specs.

| | |
|---|---|
| Result | 8 of 8 check(s) matched `expect` |
| Run | `166979-1792763128.074`, 2026-10-23T13:45:28.074093312Z → 2026-10-23T13:45:33.710201088Z |
| Jobs | 3 |
| Git | `94f3cb6c7246b8ab12d7ff1edbe473c4c82d09bc` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| tcp-capture-pending-safety-tlc | tcp_capture_pending.qnt | pending_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 5.6 | 0.0 | 5.6 |  |
| tcp-capture-pending-hazard-capacity-refusal | hazard/tcp_capture_pending_hazards.qnt | pending_bugCapacityRefusal | invariant `Safety` | tlc | exhaustive | violation | violation | PASS | 1 | 4.8 | 0.0 | 4.8 | [traces/tcp-capture-pending-hazard-capacity-refusal.txt](traces/tcp-capture-pending-hazard-capacity-refusal.txt) [traces/tcp-capture-pending-hazard-capacity-refusal.tlc.txt](traces/tcp-capture-pending-hazard-capacity-refusal.tlc.txt) |
| tcp-capture-pending-run-pending-to-ready-transfers-the-same-charge | tcp_capture_pending.qnt | pending_ok | run `PendingToReadyTransfersTheSameCharge` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| tcp-capture-pending-run-ready-at-accept-emits-once-then-abort-reuses-the-place | tcp_capture_pending.qnt | pending_ok | run `ReadyAtAcceptEmitsOnceThenAbortReusesThePlace` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| tcp-capture-pending-run-missing-refused-and-child-close-release-without-outputs | tcp_capture_pending.qnt | pending_ok | run `MissingRefusedAndChildCloseReleaseWithoutOutputs` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| tcp-capture-pending-run-overflow-resets-only-at-the-bound-then-a-release-makes-room | tcp_capture_pending.qnt | pending_ok | run `OverflowResetsOnlyAtTheBoundThenAReleaseMakesRoom` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| tcp-capture-pending-run-attach-abort-and-close-each-release-their-place | tcp_capture_pending.qnt | pending_ok | run `AttachAbortAndCloseEachReleaseTheirPlace` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| tcp-capture-pending-run-hazard-ready-below-bound-is-not-capacity-refused | hazard/tcp_capture_pending_hazards.qnt | pending_bugCapacityRefusal | run `ReadyAtAcceptEmitsOnceThenAbortReusesThePlace` | evaluator | run | violation | violation | PASS | 1 | 1.0 | 0.0 | 1.0 | [traces/tcp-capture-pending-run-hazard-ready-below-bound-is-not-capacity-refused.txt](traces/tcp-capture-pending-run-hazard-ready-below-bound-is-not-capacity-refused.txt) [traces/tcp-capture-pending-run-hazard-ready-below-bound-is-not-capacity-refused.test.txt](traces/tcp-capture-pending-run-hazard-ready-below-bound-is-not-capacity-refused.test.txt) |

## Inputs (sha256)

- `checks.toml` `64615ae079ad2f7b63b39c1ce05eef1db6c51477f1c513fb9e65cd1624efc34b`
- `hazard/tcp_capture_pending_hazards.qnt` `8f30c50424f9847fb7efd51de90fc36bee7685f9a9c836882f29f7a217877246`
- `tcp_capture_pending.qnt` `a32369b38b8911e4acf12e2436d32fe4a784f914f48f01b366b2bf520ecce421`
