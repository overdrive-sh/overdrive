# Final DISTILL remediation — backend projection and CLI test lint

## Accepted contract and reproduced RED

D7 / ADR-0259 changes an exhausted startup attempt to `StopAllocation { Stopped { by: StartupProbe } }`; `WorkloadLifecycle` owns restart and any eventual give-up verdict. D9 / ADR-0261 starts liveness only after a committed `Stable` row invokes `Driver::on_alloc_stable`. D12 / ADR-0267 makes each startup-probed allocation backend-ineligible until its **own** Stable row commits, first attempt or successor; the old predecessor veto and veto-before-stop action ordering were removed. The feature delta explicitly lists `service_backend_projection.rs` for DISTILL rewrite (§ D7 “Tests that pin the old behaviour” and § D12 item 5).

At clean source `d7386262215d88ada061846b8c1d8740b46ab9fc`, the focused Metal run `d787256257caf5df86d334f4` selected 12 `service_backend_projection` tests: 6 passed and 6 failed. Four expected healthy backends before Stable; one expected a backend write ahead of the now-first startup stop; one expected liveness to run immediately on a fresh successor. The raw output is `/private/tmp/service-readiness-final-backend-projection-focused.log`. Strict Metal Clippy run `a4d01646a15ccf21e9b248ca` separately found `doc_markdown` on the exact required `/// CONTRACT_SHAPE: pure-function.` line in `render_pure_fns.rs`.

## Bounded correction

The test-local `ProbedDriver` forwards `on_alloc_stable` to `ProbeRunner::start_liveness` after retiring startup, mirroring the production VM driver. Scenarios with declared startup now require unhealthy backend rows before committed Stable. The startup-failure trajectory requires a `Terminated` row with typed `Stopped { StartupProbe }`, then a fresh successor that remains unhealthy until its own Stable. The liveness budget trajectory drives `ServiceLifecycle` for every successor before sampling and still checks each actual replacement's distinct physical allocation id, restart budget, terminal cause, and handoffs.

The two former startup-failure withdrawal-order scenarios retain their valid row-publication and failure-isolation guarantees by driving startup Pass and committed Stable, then post-Stable readiness and liveness failures. They continue to check exact listener row stamps, allocator VIP and content-derived correlations, ordered write/hydrator pairs before the liveness stop, the typed `Terminated` stop row, the exact injected backend-write error, persisted View reload, unchanged historical predecessor, fresh successor identity, and fail-closed successor health. Obsolete startup-veto and `FinalizeFailed` assertions were replaced by those accepted typed outcomes. The rejected-first-publication scenario still proves repair from observed state and now requires the repaired pre-Stable backend to remain unhealthy. The six originally passing controls and all 12 test identities remain in the module; no seeds were skipped.

The CLI test adds one function-local `clippy::doc_markdown` allowance with a reason naming the repository's exact machine-read Contract Shape marker. Its required rustdoc line and every variant-specific assertion remain unchanged.

## Verification

| Evidence | Result |
|---|---|
| `cargo xtask metal run -- cargo nextest run -p overdrive-sim --features integration-tests --test integration -E 'test(service_backend_projection::)' --no-fail-fast` | Run `ae8883059f3ba280d1ffdac7`: **12/12 passed**, 27 skipped outside the selection. |
| `cargo xtask metal run -- cargo clippy -p overdrive-cli -p overdrive-sim --all-targets --features integration-tests -- -D warnings` | Run `1709989d88d8874e3c93afab`: passed with warnings denied. |
| Changed-file `rustfmt --edition 2024 --check`; `git diff --check` | Passed. |

The GREEN Metal runs used an isolated sparse worktree at source base `67b7a8551a5e75d7a6d9d4338a1356b71f623d96` with only the two owned test-file edits, keeping unrelated dirty work out of the source transfer; the normal Metal runner and its native preflight were used. No production code, public API, new test identity, model, DES phase event, or mutation configuration changed. Mutation testing was waived by the user.
