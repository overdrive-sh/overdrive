# fix-terminal-reason-channel-closed — Feature Evolution

**Feature ID**: fix-terminal-reason-channel-closed
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-05-02
**Commits**:
- `b71579e` — `test(control-plane,cli): RED — TerminalReason::StreamInterrupted regression`
- `95b34a3` — `fix(streaming): TerminalReason::StreamInterrupted distinguishes channel closure from timeout`

**Status**: Delivered.

---

## Symptom

A code-review comment on `crates/overdrive-control-plane/src/streaming.rs:281-297` flagged the `Err(broadcast::error::RecvError::Closed)` arm of the streaming-submit `tokio::select!`: when the lifecycle-events broadcast channel was dropped mid-stream (typical: server shutdown while a streaming submit is in-flight), the handler synthesised a terminal `SubmitEvent::ConvergedFailed` carrying `terminal_reason: TerminalReason::Timeout { after_seconds: 0 }`. Downstream consumers branch on the structured discriminant — the CLI's `derive_reason_from_terminal` rendered "workload did not converge within 0s" and `derive_hint` offered the `--detach` timeout hint, which is the wrong message and the wrong remediation for what is actually a server-side stream interruption. The human-readable `error: Some("lifecycle channel closed")` field clarified the cause for an operator who read it, but the structured discriminant was authoritative for any programmatic path (exit-code mapping, hint selection, future TUI).

## Root cause

**The `TerminalReason` wire enum had a domain gap, and the implementer reached for a sentinel to fill it.** Five-whys traced three reinforcing branches: (A) **type-system gap** — `TerminalReason` (api.rs:403-418) modelled reconciler convergence outcomes only (`DriverError`, `BackoffExhausted`, `Timeout`), with no variant for stream-transport failure. The implementer reached for `Timeout { after_seconds: 0 }` because it was the only payload-free variant; `0` is a sentinel — exactly the shape `.claude/rules/development.md` § "Sum types over sentinels" forbids. (B) **CLI consumer is correctly discriminant-driven** — `render.rs:390-403, 428-435` branches on the discriminant for both `Reason:` text and `Hint:` selection, so a wrongly-discriminated event produces wrong operator-facing text. The CLI is right to trust the discriminant; the bug is upstream. (C) **untested path** — zero hits across `crates/overdrive-control-plane/tests/` and `crates/overdrive-cli/tests/` for `after_seconds: 0`, `channel closed`, or `RecvError::Closed`. The `Closed` arm had never been exercised by an acceptance test. ADR-0032 §8 nominally routed channel-closed → `DriverError`, but `DriverError` requires `cause: TransitionReason`, which the `Closed` arm cannot construct (the bus that would carry the cause is gone). The ADR routing was unimplementable; the fix corrects both.

A secondary engineering surprise surfaced during 01-01: the streaming handler retains an internal `Arc<Sender>` clone after `let mut sub = bus.subscribe();`, which keeps the broadcast channel open even when every external clone drops. Without an explicit `drop(bus);` after subscribe, the `RecvError::Closed` arm cannot fire under any production scenario — the test would have hit the cap-timer fallback instead. This was caught in step 01-01's RED phase and addressed atomically in 01-02's GREEN.

## Fix

**Approved fix shape**: additive new payload-free `TerminalReason::StreamInterrupted` variant on the `#[non_exhaustive]` enum. Step 01-01 landed the variant declaration, extended the `arb_terminal_reason` proptest generator (round-trip coverage including the new variant), authored a new acceptance test `streaming_channel_closed.rs::closed_lifecycle_channel_emits_stream_interrupted_terminal` that drops the broadcast `Sender` mid-stream and asserts the terminal carries `StreamInterrupted`, and authored a new CLI render scenario `renders_stream_interrupted_terminal_with_correct_reason_and_hint`. Both new acceptance scenarios were intentionally RED — production behaviour was unchanged in 01-01. Commit `b71579e` used `git commit --no-verify` per `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing commits".

Step 01-02 (commit `95b34a3`) flipped both tests RED → GREEN with three production changes: (1) the emit site at `streaming.rs:281-297` now constructs `TerminalReason::StreamInterrupted` instead of the `Timeout { after_seconds: 0 }` sentinel; (2) `derive_reason_from_terminal` and `derive_hint` in `render.rs` gained explicit match arms (placed before the `_ =>` fallback) producing "server-side stream interrupted before convergence" for the `Reason:` line and "server-side stream was interrupted; re-run `overdrive job submit` or consult `overdrive alloc status --job <id>` for the current state" for the `Hint:` line; (3) the streaming handler drops its internal `bus` Arc clone after subscribing so the `RecvError::Closed` arm is genuinely reachable when external senders drop. `api/openapi.yaml` regenerated atomically via `cargo xtask openapi-gen` — the schema picks up the new variant through `utoipa::ToSchema`, gaining `kind: stream_interrupted` as a fourth `oneOf` member.

The fix is structurally additive: no existing variant changed, no existing test failed or needed rewriting, no CLI exit code semantics changed (`ConvergedFailed → 1` regardless of inner reason per ADR-0032 §9), and no wire contract broke (Phase 1 has no external NDJSON consumers; intra-workspace versions are lock-stepped). The renderer's pre-existing `_ =>` fallback arms also meant the variant addition was forward-compatible — adding the variant without updating the renderer would have compiled cleanly and degraded gracefully.

## Files changed

`git diff --stat b71579e^..95b34a3`:

| Path | Lines | Role |
|---|---|---|
| `crates/overdrive-control-plane/src/api.rs` | +14 | `TerminalReason::StreamInterrupted` variant declaration + rustdoc (step 01-01) |
| `crates/overdrive-control-plane/src/streaming.rs` | +22/-5 | Emit-site swap to `StreamInterrupted` + `drop(bus);` after `subscribe()` (step 01-02) |
| `crates/overdrive-cli/src/render.rs` | +8 | Explicit match arms in `derive_reason_from_terminal` and `derive_hint` (step 01-02) |
| `api/openapi.yaml` | +20 | Regenerated `TerminalReason.oneOf` fourth member with `kind: stream_interrupted` (step 01-02) |
| `crates/overdrive-control-plane/tests/acceptance/submit_event_serialization.rs` | +1 | Extended `arb_terminal_reason` with `Just(StreamInterrupted).boxed()` (step 01-01) |
| `crates/overdrive-control-plane/tests/acceptance/streaming_channel_closed.rs` | +NEW | RED-then-GREEN regression test for the closed-channel arm (step 01-01) |
| `crates/overdrive-control-plane/tests/acceptance.rs` | +1 | Module declaration for new test file (step 01-01) |
| `crates/overdrive-cli/tests/acceptance/streaming_submit_cli_render.rs` | +N | New render scenario for `StreamInterrupted` (step 01-01) |

## Tests added

- **NEW**: `crates/overdrive-control-plane/tests/acceptance/streaming_channel_closed.rs::closed_lifecycle_channel_emits_stream_interrupted_terminal` — constructs `AppState` with sim adapters (LocalStore intent, SimObservationStore observation, SimClock), kicks off a streaming submit, drops the lifecycle broadcast `Sender`, collects the NDJSON terminal line, and asserts `terminal_reason == TerminalReason::StreamInterrupted`. RED in 01-01 (emit site still produced `Timeout { after_seconds: 0 }`); GREEN in 01-02. Closes the gap-C "Closed arm is currently un-tested" finding from the RCA.

- **NEW**: `crates/overdrive-cli/tests/acceptance/streaming_submit_cli_render.rs::renders_stream_interrupted_terminal_with_correct_reason_and_hint` — constructs `SubmitEvent::ConvergedFailed { terminal_reason: TerminalReason::StreamInterrupted, ... }`, runs the renderer, asserts the printed `Reason:` line contains "server-side stream interrupted before convergence" AND the `Hint:` line does NOT contain `--detach`. RED in 01-01 (renderer fell through to `_ =>` arms); GREEN in 01-02.

- **EXTENDED**: `crates/overdrive-control-plane/tests/acceptance/submit_event_serialization.rs::arb_terminal_reason` — proptest generator gained `Just(TerminalReason::StreamInterrupted).boxed()`. The pre-existing round-trip property now covers the new variant; serde tag-and-content shape (`#[serde(tag = "kind", content = "data", rename_all = "snake_case")]`) emits `{"kind":"stream_interrupted"}` correctly.

## Quality gates

- **DES integrity** — both steps have complete 5-phase traces in `docs/feature/fix-terminal-reason-channel-closed/deliver/execution-log.json`. Step 01-01: PREPARE / RED_ACCEPTANCE / RED_UNIT (SKIPPED, NOT_APPLICABLE — integration-shaped acceptance test asserts the bug through the streaming-handler and renderer entrypoints; the proptest generator extension is structural, passing post-add) / GREEN / COMMIT. Step 01-02: PREPARE / RED_ACCEPTANCE (SKIPPED, NOT_APPLICABLE — failing scenarios authored in 01-01) / RED_UNIT (SKIPPED, NOT_APPLICABLE — bug surfaces port-to-port; proptest already covered) / GREEN / COMMIT. `verify_deliver_integrity` passed: "All 2 steps have complete DES traces".
- **Workspace nextest** on macOS: default lane (`cargo nextest run -p overdrive-control-plane`, `cargo nextest run -p overdrive-cli`) PASS post-GREEN; pre-GREEN, the two new acceptance scenarios were the only failures, with documented panic messages. Compile-only on the integration-tests-gated surface (`cargo nextest run --workspace --features integration-tests --no-run`) PASS.
- **OpenAPI gate** — `cargo xtask openapi-check` PASS (regenerated yaml matches `utoipa::ToSchema` output).
- **Mutation gate** — **SKIPPED per user instruction during finalize.** Mutation surface was small (one variant declaration, one emit-site swap, two render match arms) and would normally be gated under per-feature mutation testing per `.claude/rules/testing.md` mutation-testing rules; without mutation evidence the suite's defensiveness on those branches is unverified.
- **Refactor + adversarial review** — **SKIPPED per user instruction during finalize.**

## Out of scope (flagged for follow-up)

- **ADR-0032 §8 update** — the architect agent owns the ADR edit per project convention; it is **not** done inline here. Replace the row mapping channel-closed-unexpectedly → `DriverError` with channel-closed-unexpectedly → `StreamInterrupted`. The original mapping was unimplementable as written (`DriverError` requires a `cause: TransitionReason` payload the `Closed` arm cannot produce because the bus that would carry it is gone). To be dispatched separately via `nw-solution-architect` (or the relevant ADR-owning agent).
- **Mutation testing** — explicitly skipped per user instruction. The RCA's risk-assessment names the structurally novel branches that mutation would have probed (variant emit-site selection, render-arm reachability, the `drop(bus)` placement). Without mutation evidence the suite's defensiveness on those branches is unverified.
- **`Refactor` and `Adversarial Review` phases** — both skipped during this finalize.
- **Wire-evolution documentation** — the Phase 1 lock-step model meant this enum addition shipped without an external-consumer migration story. If/when external NDJSON consumers come online, the additive-on-`#[non_exhaustive]` discipline plus the OpenAPI regeneration are the load-bearing primitives; document the convention then, not now.

## References

- **RCA (durable spec for this fix)**: `docs/feature/fix-terminal-reason-channel-closed/deliver/rca.md` — the workspace itself remains under `docs/feature/` (per `nw-finalize` skill: directory preserved as the wave-matrix history; this evolution doc is the summary). The RCA's body is the spec, preserved verbatim in commit `b71579e`'s tree.
- **Specifying ADR**: `docs/product/architecture/adr-0032-ndjson-streaming-submit.md` § 8 (HTTP error semantics) — the ADR row that needs updating per the architect-agent dispatch (out of scope above).
- **Test discipline**: `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing commits", § "Tests that mutate process-global state" (not applicable here, but referenced for completeness).
- **Type-system rule violated by the original code**: `.claude/rules/development.md` § "Sum types over sentinels" — the `Timeout { after_seconds: 0 }` placeholder was exactly the rule-violation shape the convention exists to forbid. The fix replaces the sentinel with a dedicated variant, which is what the rule mandates.
- **Commits**: `b71579e` (RED), `95b34a3` (GREEN).
