# Independent DISTILL review — S-30 startup-stop ordering oracle

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | One S-30 acceptance oracle in `crates/overdrive-control-plane/tests/acceptance/startup_stop_authority.rs` |
| Authority | Approved D7/D8/B2 S-30 retry and stop-row order; existing exact-predecessor cleanup contract of `RestartAllocation` |
| Protected work | Original step 02-03 crafter's dirty production changes, activated tests, DES phases and commit |
| Verdict | **APPROVED** — the oracle separates startup-stop retries from later exact-predecessor restart cleanup without weakening the row, hook or successor order |

## Reproduction and actual owner path

The composed `failed_startup_stop_is_retried_once_per_new_startup_row` body failed at seed `0` after the production work for step 02-03. The retained pre-correction Metal log (`.context/step-02-03-green-acceptance.log:56–61`) printed this driver trace:

```text
Start(first), Running(first), Release(first),
Stop(first), Stop(first), Stop(first), Terminal(first),
Start(successor), Running(successor), Release(successor), Stop(first)
```

The test scripted two `Driver::stop` failures on distinct written startup probe rows. A third newer row caused the successful startup stop, writing `Terminated` / `Stopped { StartupProbe }` and then calling `on_alloc_terminal` (D8, D7 and B2). The test's old `rposition(Stop(first)) < position(Start(successor))` selected the **fourth** stop in the trace and failed. That fourth call is the separate `RestartAllocation` exact-predecessor cleanup after the successor outcome. The production `RestartAllocation` arm captures the predecessor identity and driver, completes the successor path, then awaits `finish_restart` (`action_shim/mod.rs:2698–2741,3200–3227`). `finish_restart` calls `cleanup_restart_predecessor`, which performs `Driver::stop` against the captured predecessor handle (`:1777–1835`). The composed `RestartWorld::restart_successor` invokes the production workload-lifecycle tick and returns the current allocation (`restart_authority_world.rs:620–624`). No test-only stop or fake cleanup was introduced.

## No-weakening audit of the changed assertion

The revised body retains the same setup, `SimDriver::stop` fault script, unchanged-row universe check, one stop on the first row, no stop on a repeated same row, a second stop on the next row, a third successful stop on the next newer row, and the typed stopped-row assertion (`startup_stop_authority.rs:222–247`). It now also requires:

1. Exactly three `Stop(first)` calls **before** `Terminal(first)`: the two failed startup attempts and the successful retry.
2. `Terminal(first)` before `Start(successor)`: under B2 the hook follows the successful terminal-row write, so this keeps the accepted stopped-before-successor order.
3. Exactly one `Stop(first)` after `Start(successor)`: the existing exact-predecessor cleanup during `RestartAllocation`, distinct from the startup-stop retry sequence.

The assertions inspect the production driver trace, not a test-authored action or model projection (`startup_stop_authority.rs:248–279`). The successful stopped row is read before the restart. The late-`Pass` complement and failed-stop-row-write/B2 complement retain their bodies and assertions. The per-test `/// CONTRACT_SHAPE: bounded-change.` declarations remain; the three removed `#[ignore]` markers in the diff are the original crafter's step activation, not part of this DISTILL oracle correction. No production lifecycle behavior, Quint model, public API or new test seam changed.

## Verification and disposition

| Evidence | Result |
|---|---|
| Before | The same three-case S-30 selection ran **2 passed, 1 failed**; the failed test printed the trace above (`.context/step-02-03-green-acceptance.log`). |
| After, independent focused Metal run | `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance --no-fail-fast -E 'test(failed_startup_stop_is_retried_once_per_new_startup_row) | test(late_startup_pass_after_a_failed_stop_ends_the_retry_and_reaches_stable) | test(failed_stop_row_write_keeps_the_supervisor_and_the_next_probe_row_retries)'` completed **3/3 passed**, exit 0 (run ID `942bbc8dd713fec9821abc0f`). |
| Static checks | The author reports Metal `cargo clippy -p overdrive-control-plane --test acceptance --features integration-tests -- -D warnings`, changed-file `rustfmt --check --edition 2024`, and `git diff --check` passed. My read-only `git diff --check` passed. No mutation run was made. |

**Verdict: APPROVED for this bounded DISTILL oracle correction.** It preserves D8's per-new-row retry and dedup evidence, D7's stopped-row/successor order, B2's post-write hook and the exact-predecessor cleanup effect. The original step 02-03 crafter retains ownership of its implementation checks, DES log and commit; this review is not a DELIVER production review or a claim that the whole step is complete.
