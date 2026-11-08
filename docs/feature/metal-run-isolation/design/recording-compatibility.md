# Quint recording codec compatibility

This is a test/model recording correction for the accepted P1–P5 design. It changes no production command, API, owner, lifecycle state, error, invariant or selected verification outcome. DISTILL still owns its private conformance driver and fixtures. The sole permanent protocol model remains the oracle; this correction introduces no second model or runtime mechanism.

## Verified interface limitation

The workspace pins `quint-connect = "=0.1.2"`. Its installed `src/driver/step.rs` resolves `Config::nondet` as a static record path, then requires `tag: String` and a `value` containing a record or unit (`Step::new`, `extract_from_sum_type`, lines 23–49 and 106–129). The original `ConfStep = { op: str, actor: int, value: int }` does not have that shape.

`src/runner/mod.rs:51–84` constructs the private `Step::new` before calling `Driver::step`; the public driver exposes only `step` and static-path `config`, and `Step.state` is private. `State::from_spec` also runs after decoding. Therefore neither a handler-local mapping nor a state deserializer can repair this envelope before decoding. An empty `Config::nondet` instead requires `mbt::actionTaken` and `mbt::nondetPicks`, which the actual named-test traces do not supply. This is a recording interface mismatch, not a production protocol failure.

A separate trace-rewriting command or imported recording state machine would add another codec/recording layer around the same oracle. The smaller correction is to emit the library's required envelope from the sole model's already-existing write-only ghost. No dependency fork or version change is needed.

## Recording representation and mapping

The corrected recording envelope is:

```text
type ConfStep = { tag: str, value: { actor: int, value: int } }
Config { state: &[], nondet: &["runner_conf::runner_protocol::conf_step"] }
```

The string tag is the existing semantic action name; `actor` and the inner `value` retain their existing meanings. The library accepts this ordinary record shape; a separate variant enum or definitions module is unnecessary. Rust handler signatures and private decoding helpers remain DISTILL decisions.

| Tag | `actor` pick | Inner `value` pick |
|---|---|---|
| `init` | 0 | The selected second checkout, 1 or 2 |
| `Register`, `Barrier`, `Checkout`, `Prepare`, `Start`, `LoseController`, `AdmissionTimeout`, `SupervisorLost`, `Collect`, `NoSyncRefusal` | Invocation slot 1 or 2 | 0 |
| `AbortPreparation` | Invocation slot | Existing runner code 75 or 78 |
| `MainExit` | Invocation slot | Actual modeled representative command code 0 or 255 |
| `StopOwner` | Invocation slot | Existing stop outcome 0, 1, 2 or 3 |
| `Sync` | Checkout 1 or 2 | 1 |
| `Maintain` | 0 | Existing maintenance operation 0, 1, 2 or 3 |
| `Idle` | 0 | 0 |

`Idle` must record its own tag when `REC=true`. Previously it retained the preceding tag, so a randomized trace's actual stutter could be decoded as a repeated START or other previous operation. This correction affects only the recording ghost: the existing Idle action still preserves every modeled production state field. A real conformance handler must stutter and compare the owner observations; it must not repeat the prior action.

`REC=false` continues to keep the initial recording value constant for safety, progress and hazard checks. No guard or property uses its payload; the existing fairness tuple still names the same ghost variable. `conf_step` remains excluded from production projections. The full action set, thirteen named scenarios, invariant/progress definitions, two hazards and eighteen manifest checks remain unchanged.

## Validation boundary

The delegated project `quint-modeler` owns the recording edit and formal evidence refresh. Its bounded codec check uses real generated traces and the pinned library's public runner/Driver boundary. That check establishes decoding and exact stimulus delivery only; it is explicitly not production conformance or a substitute owner. DISTILL must still implement the actual shell/systemd/kernel driver with the accepted projections.

For P2/P5 replay, a private fixture may play the existing controller at the declared START/ABORT boundary while the real durable service remains the protocol owner. `NoSyncRefusal` uses an actually mismatching source fixture, sends the approved `source_mismatch` ABORT and verifies the real retained 78/no-start outcome. That exercises the control protocol; it does not prove that the production controller selected that branch. Separate composed full-bootstrap tests must exercise the actual no-sync and missing-rsync producer decisions. `Prepare` uses actual production transfer/preflight adapters under the service-owned preparation authority and then reads actual source/records/fdinfo; copied validation or a fixture-owned prepared-state ledger is forbidden. No new CLI phase-selection or public test port follows from this fixture choice.

## Completed verification

The delegated modeler reproduced the original flat-record failure through quint-connect 0.1.2's actual public runner before any `Driver::step` call. The repaired runner decoded all **13 named traces / 119 states** and **four seeded randomized traces / 132 states**, including **45 Idle transitions**. Exact action tags and both integer picks matched real generated ITF; random tags also matched actual `mbt::actionTaken`. [Full findings, real traces and codec diagnostic](codec-compatibility/findings.md).

All **18 existing formal checks** were re-recorded: 16 holds, two expected hazard violations, zero unexpected results; `verify-evidence` passed. All 53 repository Quint files typechecked. Safety and both progress checks retained the same **695,458 distinct states**. The original/repaired named traces have identical non-recording states, and all non-recording model text, manifest entries, hazards and named runs are unchanged. The final record's slowest check took 117.7 seconds; a preliminary run exceeded the CI-duration threshold, which is disclosed in the retained findings rather than hidden by changing the manifest.

The diagnostic's `State = ()` explicitly disables state comparison and proves only codec/stimulus delivery. It is not a test-only substitute owner, a production conformance pass or permission to project driver bookkeeping. DISTILL received the checked mapping and retains its actual owner-conformance work. No production or DISTILL files, user-approved P1–P5 semantics, CLI/error contracts, additional model checks or commits were created by this compatibility remediation.
