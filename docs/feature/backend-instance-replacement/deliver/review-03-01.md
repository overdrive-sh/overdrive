# Adversarial Review - backend-instance-replacement step 03-01

**Reviewer**: Codex (`/nw-review`, adversarial)  
**Date**: 2026-07-01  
**Step**: `03-01` - A1 pump half-close-forward plus T1/T2 churn test-model fix  
**Reviewed HEAD**: `adbbb600`  
**Verdict**: `APPROVED_WITH_NITS`

## Scope Reviewed

- `docs/feature/backend-instance-replacement/deliver/roadmap.json` step `03-01`
- `docs/feature/backend-instance-replacement/deliver/execution-log.json`
- `docs/feature/backend-instance-replacement/deliver/mutants-03-01.md`
- `crates/overdrive-dataplane/src/mtls/splice.rs`
- `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs`

Review posture: adversarial code/test/evidence review against step `03-01`, focused on the A1 clean-close half-close forward, teardown discrimination, production pump call-site coverage, T1/T2 test-model repairs, and the mandatory mutation evidence. Per instruction, I did not run AC gates.

## Findings

No blocking findings.

### LOW-1 - One T1 doc phrase overstates the empty-read behavior

**Dimension**: readability / documentation precision  
**Severity**: low  
**Location**: `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs:747-780`

The T1 comment says the Python backend "blocks on an empty read without closing." The implementation is correct for a long-lived full-duplex backend: it blocks on `recv()` while the client keeps the connection open, responds to each non-empty request, and closes only when `recv()` returns empty data. The wording is slightly off because an empty read is already peer EOF, not an idle read.

This is not a behavioral defect and does not weaken S-DBN-CHURN. Suggested cleanup: rephrase that sentence to "blocks in `recv()` while idle and closes only after peer EOF."

## Positive Evidence

- The production A1 forward is present at both terminal pump paths before `mark_exited`: `run_decrypt_pump` calls `forward_half_close_if_source_eof(dst_fd, exit, state)` before `mark_exited` at `splice.rs:448-449`, and `run_encrypt_pump` does the same at `splice.rs:538-539`.
- The forward predicate matches the amended contract: it forwards only on `PumpExit::Graceful` when `state.stop` is false, and it suppresses deliberate teardown and transport-death paths (`splice.rs:252-269`).
- The original helper-only testing gap has been closed. `decrypt_pump_forwards_half_close_on_source_eof` and `encrypt_pump_forwards_half_close_on_source_eof` enter through the real pump loops and assert the dst peer observes EOF after a genuine source half-close (`splice.rs:791-889`).
- The non-source/dst-EOF ambiguity is now explicitly documented and regressed as a harmless already-closed-dst forward (`splice.rs:236-251`, `splice.rs:892-961`). I do not see a remaining correctness blocker there because the roadmap/ADR amendment now pins `!state.stop` as the sole discriminator.
- T1 is implemented: `server_service_spec` now serves each accepted connection on a daemon thread and loops over requests instead of closing after one response (`dns_responder_walking_skeleton.rs:760-783`).
- T2 is implemented: `churn_in_flight_read` now requires the first round-trip to equal `RESPONSE` before it holds the connection open for churn (`dns_responder_walking_skeleton.rs:2110-2135`).
- The mandatory mutation evidence exists and is reviewable. `mutants-03-01.md` records the requested diff-scoped command, a 5/5 caught result with `kill_rate=100.0%`, the three named kill targets, and manual call-site deletion proofs (`mutants-03-01.md:102-163`, `:167-238`).

## Validation

I did not run AC gates, per instruction. Static review only: roadmap, execution log, current code, test fixtures, and the existing mutation artifact.

## Decision

Step `03-01` is approved with one documentation nit. The earlier blocker class is addressed in the current workspace state: the pump call sites are covered through real pump-level tests, and the mutation artifact accounts for the step's mandatory A1 kill targets.
