# Step 02-03 DISTILL remediation: startup stop ordering oracle

## Reproduction and contract

The active S-30 composed acceptance test, `failed_startup_stop_is_retried_once_per_new_startup_row`, failed on Metal with the deterministic `RestartWorld` seed `0`. The failing assertion used the **last** `Stop(first)` in the complete driver trace. The trace was:

```text
Start(first), Running(first), Release(first),
Stop(first), Stop(first), Stop(first), Terminal(first),
Start(successor), Running(successor), Release(successor), Stop(first)
```

The first two stops are scripted driver failures. The third stop succeeds, writes the `Terminated` / `Stopped { StartupProbe }` row, and fires the terminal hook. This is the D7/D8/B2 startup-stop path: D8 permits one retry per newer startup probe row, D7 requires the guest stopped before its successor is placed, and B2 places the terminal hook after a successful stop-row write. The final `Stop(first)` is a different effect: `RestartAllocation` calls `finish_restart` after the successor outcome, which calls `cleanup_restart_predecessor` for the exact predecessor identity. ADR-0105/ADR-0106 and the existing action-shim contract permit that cleanup after successor creation. The old `rposition(Stop(first)) < position(Start(successor))` conflated these two effects.

Production path: `RestartWorld::restart_successor` drives the production convergence tick; its `RestartAllocation` dispatch enters `action_shim/mod.rs` at the predecessor row fence, starts the successor, then calls `finish_restart` (`mod.rs:2698-2741`, `:3200-3227`). `finish_restart` calls `cleanup_restart_predecessor`, whose driver stop is exact-old-ID (`mod.rs:1777-1793`, `:1814-1835`). The composed driver records both stop attempts in one trace.

## Bounded correction

Only the S-30 acceptance oracle changed. It now checks that exactly three startup-stop calls precede `Terminal(first)`, that `Terminal(first)` precedes `Start(successor)`, and that exactly one exact-predecessor cleanup stop occurs after `Start(successor)`. The existing assertions for failed-stop errors, one action per new probe row, no action on an unchanged row, the successful stopped row, and untouched observable state remain in place. The approved lifecycle behavior, production code, model, pending markers, and DES log were not changed by this remediation.

## Verification

| Check | Result |
| --- | --- |
| Before: `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(failed_startup_stop_is_retried_once_per_new_startup_row)'` | Failed at the old ordering oracle, seed `0`, with the trace above; exit 100. |
| After: the three activated S-30 composed cases (command below) | 3/3 passed, exit 0. |
| Contract Shape declaration | Existing `/// CONTRACT_SHAPE: bounded-change.` retained on the transitioned S-30 body. |
| Metal `cargo clippy -p overdrive-control-plane --test acceptance --features integration-tests -- -D warnings` | Passed. |
| Changed-file `rustfmt --check --edition 2024` and `git diff --check` | Passed. |

```sh
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance --no-fail-fast -E 'test(failed_startup_stop_is_retried_once_per_new_startup_row) | test(late_startup_pass_after_a_failed_stop_ends_the_retry_and_reaches_stable) | test(failed_stop_row_write_keeps_the_supervisor_and_the_next_probe_row_retries)'
```

Changed file: `crates/overdrive-control-plane/tests/acceptance/startup_stop_authority.rs` (oracle only). Independent DISTILL review remains the handoff gate before the original step 02-03 crafter resumes GREEN and COMMIT.
