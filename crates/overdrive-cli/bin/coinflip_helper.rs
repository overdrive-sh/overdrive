//! Test-fixture binary — exits 1 within ~30ms.
//!
//! Consumed by `examples/coinflip-as-service.toml` (Fixture A) for
//! the K1 100-seed honesty loop in step 01-03f-2.
//!
//! Per RCA-A (`docs/feature/service-health-check-probes/discuss/...`):
//! this binary models a workload whose true behaviour is
//! deterministic-failure-immediately. The reconciler's `EarlyExit`
//! branch (US-08) MUST emit `Failed { reason: EarlyExit {
//! exit_code: 1 } }` for every alloc that runs this binary —
//! never `Stable`, never `(took live)`.
//!
//! Per `.claude/rules/development.md` § "src/ is production code —
//! tooling binaries live in bin/": this is a test-fixture binary
//! with no library consumer, so it lives at the crate's `bin/`
//! directory, NOT under `src/bin/`.
//!
//! Production exec command path: the alloc's TOML may either
//! reference the cargo-built artifact at
//! `target/{debug,release}/coinflip_helper`, OR a copy installed at
//! `/tmp/coinflip-helper` (which `coinflip-as-service.toml` uses by
//! default; 01-03f-2's test harness copies the artifact into place
//! before driving the K1 loop).

#![allow(clippy::expect_used)]

fn main() {
    // No deliberate sleep — the process exits within the natural
    // process-startup + main-entry overhead, which is well under
    // 30ms on every supported platform. RCA-A's "Running → Failed
    // within startup_deadline" gate requires the exit to land
    // before the reconciler's first probe tick at t=interval_seconds
    // (default 2s) — 30ms vs 2s is a comfortable margin.
    //
    // Exit code 1 is the deterministic-failure signal: the
    // reconciler's `EarlyExit` branch carries this verbatim into
    // `ServiceFailureReason::EarlyExit { exit_code: 1 }`.
    std::process::exit(1);
}
