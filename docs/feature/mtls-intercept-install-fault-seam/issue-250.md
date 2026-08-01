## Summary

`fail_closed_on_mtls_install` (`crates/overdrive-control-plane/src/action_shim/mod.rs`) is the security-critical fail-closed handler from the **transparent-mtls-host-socket** feature (step 06-03; introduced by `5d7fbae0`). When `MtlsInterceptWorker::start_alloc` returns `Err(MtlsInterceptInstallError)` on a freshly-spawned Running alloc, it stops the driver, supersedes the Running row with a Failed row carrying `TransitionReason::MtlsInterceptInstallFailed`, and does NOT release the exit-emission gate (so a now-Failed alloc never releases its watcher).

It has **no killer test** — no integration/unit test forces an install failure on a Running alloc and asserts the fail-closed Failed row. cargo-mutants' whole-body-replacement mutant (`-> Ok(())`) is therefore **MISSED**: a fail-closed security handler silently turned into a no-op would pass the suite.

## Why it isn't trivially testable today

`mtls_worker` is `Option<&Arc<MtlsInterceptWorker>>` — a **concrete type**, not a port trait. To inject an install failure in a default-lane `dispatch_single` test, the worker needs a fault-injection seam: either a port trait for the intercept-install surface (matching the existing port-trait discipline — cf. the `MtlsEnforcement` trait + `SimMtlsEnforcement` in `overdrive-sim`) or a sanctioned test-only constructor whose `start_alloc` returns `Err`. That mtls-intercept-install fault-injection infrastructure does not exist.

## Scope

- Build the fault-inject seam for the mtls-intercept-install surface (port trait + sim/host adapters, or a sanctioned test-only failure injection — pin in DESIGN; do not invent surface ad hoc, CLAUDE.md "Implement to the design").
- Default-lane killer test: dispatch `StartAllocation` (and the symmetric `RestartAllocation` arm — both call this helper) with a failing worker; assert the Running row is superseded by a Failed row carrying `TransitionReason::MtlsInterceptInstallFailed`, the driver is stopped, and the exit-emission gate is NOT released.
- Remove the `.cargo/mutants.toml` `exclude_re` exclusion for `fail_closed_on_mtls_install` once the killer test lands.

## Provenance / surfaced by

- Helper: `crates/overdrive-control-plane/src/action_shim/mod.rs::fail_closed_on_mtls_install` (transparent-mtls-host-socket step 06-03; commit `5d7fbae0`).
- `MtlsInterceptWorker::start_alloc` — `crates/overdrive-worker/src/mtls_intercept_worker.rs:495`; `MtlsInterceptInstallError` — `:121`.
- Surfaced by the dial-by-name-responder **02-02** mutation review (`docs/feature/dial-by-name-responder/deliver/review-02-02.md`): the helper's ONLY 02-02 diff-touch was the required-parameter `None,` wiring on its `build_alloc_status_row` call (a Failed row carries no addr — correct); cargo-mutants' `--in-diff` pulled the whole-body mutant from that changed-region window. The missed mutant is **pre-existing and out of 02-02 scope**. Skipped via `.cargo/mutants.toml` `exclude_re` pending this issue.
- Origin feature issue (closed): #236 (transparent-mTLS interception model).
