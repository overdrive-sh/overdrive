# Final DISTILL oracle alignment

## D9 terminal-hook and probe-count fixtures

Approved D9 / ADR-0261 starts liveness only after the allocation's `Stable` row commits and the shim calls `Driver::on_alloc_stable`. The retained final-check failures at Metal run `0e88b6e07d6ba9b1f44ab817` came from three `terminal_hook_ownership` scenarios that asserted live liveness immediately after `StartAllocation`. Their existing `HookedDriver` now mirrors the production Stable hook (`stop_role(Startup)`, then `start_liveness`). Each affected fixture dispatches a real `FinalizeFailed { Stable }` through the production shim before checking liveness. The natural-exit oracle still requires a `Failed` row without a terminal claim; it no longer carries the prior Stable success claim into the exit row. The D1/D4 hook-call, predecessor, sibling, supervisor, and full observed-state assertions remain.

The retained probe-count conformance failure at Metal run `15bc6abc4f499e88a29f8ddb` had the same D9 cause: the test composed `ServiceLifecycle` with a plain `SimDriver`, so the real `Stable` row committed but no hook reached the real `ProbeRunner`. A private fixture driver delegates the existing `Driver` operations to that `SimDriver` and forwards its existing `on_alloc_stable` port to the runner after the shim's commit. The conformance model, seed, counter and row projections, and all ten test identities are unchanged. No production seam or public API was added.

## D10 timeout oracle

Approved D10 derives a Service progress deadline as `max(streaming_cap, startup_deadline + 30 s)`; the no-startup-probe Service's deadline is 90 seconds even when the fixture sets the floor to one second. The retained Metal failure at run `4b2b6db7c720bf96ec2c3c6b1` advanced `SimClock` only two seconds. The existing handler test now derives the deadline from its submitted Service spec, advances just past that value, and requires the typed `Timeout.after_seconds` to equal the same deadline. Its `Failed`, absent verdict allocation, and typed Timeout assertions remain.

## Verification

| Evidence | Result |
|---|---|
| Focused Metal selection: `terminal_hook_ownership::`, `quint_probe_count_`, `quint_stream_snapshot_`, and `s_shcp_wire_13_cap_timer_synthesises_service_failed_timeout`, with `QUINT_SEED=0x43d4164d` | Run `b482a5e4f8d02acc81b5a8d6`: 22 selected; 20 passed. The only failures were the natural-exit oracle's prior Stable claim, corrected afterward, and the separate D11 randomized conformance mismatch below. All ten probe-count runs, the other terminal-hook scenarios, the D10 handler test, and the seven named stream-snapshot runs passed. |
| Corrected natural-exit oracle, exact selected Metal test | Run `ff3b6854be9a8c320977dac1`: 1/1 passed. |
| Changed Rust files, `rustfmt --edition 2024 --check`; `git diff --check` | Passed. |

The user waived mutation testing. No DES phase event, production code, public API, new test identity, or new test universe changed.

## D11 randomized conformance mapping remains open

The retained failing seed `0x43d4164d` first exposed a one-poll-to-one-`Deliver` assumption: production can drain a restartable stop and a later projecting event in one poll. An exploratory private-driver adjustment advanced past that trace but exposed a distinct window trace. The model's `Subscribe` action queues an operator-stop event for attempt 0 before `Read`; it then mints attempt 1 and expects later `Deliver(0)` to close on the old operator stop. The conformance driver maps `Subscribe` to the first `Accepted` line. Production `build_service_stream` yields that line **before** calling `bus.subscribe()` on the next poll; its read then sees attempt 1 as current and, after a later event, yields attempt 1's give-up. The exploratory adjustment was reverted; this commit changes no D11 driver or model.

D11's approved operator contract is `Accepted`, then subscribe, then read the **current attempt** and report its recorded verdict or wait. It does not promise that receipt of `Accepted` means the bus receiver already exists, nor one channel event per external stream poll. The model note and current driver mapping are therefore insufficient evidence of a production defect. The injected `operator_stop(0) -> Restart -> giveup(1)` trace also does not establish reachability through the production restart owner. This is an unresolved conformance-driving and model-abstraction issue for independent test-specialist review; no public contract has been weakened and no `SPECIFICATION_AMBIGUITY` is claimed. The original seed remains a required failing witness until an honest mapping is approved and verified.
