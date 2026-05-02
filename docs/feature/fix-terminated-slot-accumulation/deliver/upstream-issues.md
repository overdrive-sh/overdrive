# Upstream Issues — fix-terminated-slot-accumulation

Issues surfaced during step 01-02 (GREEN — drop `LiveAllocation::Terminated`)
that originate **upstream** of this feature's scope and should be tracked
as separate work.

## Issue 1 — `stop_escalates_to_sigkill` timing-lower-bound assertion is unsafe under `SimClock`

**File**: `crates/overdrive-worker/tests/integration/exec_driver/stop_escalates_to_sigkill.rs`
(L184-187 — the `elapsed >= Duration::from_millis(250)` assertion)

**Origin**: Commit `dd6437` ("refactor(worker): inject Clock into ExecDriver,
ban tokio::time in production") — that commit migrated `ExecDriver::stop`
from `tokio::time::timeout` to `tokio::select!` over `Clock::sleep`, and
switched the test to wire `Arc::new(SimClock::new())` instead of a wall-clock
implementation. The test was never run end-to-end under Lima after that
commit because the regression test added in step 01-01 (`a3606fb`) carried a
`format!()`-vs-`&str` compile typo that prevented the integration test
binary from compiling on Linux.

**Symptom**: `elapsed = 525.451µs` against an expected `>= 250ms`. The driver
sleeps via `SimClock::sleep(250ms)`, which under `SimClock` advances logical
time and yields once cooperatively rather than blocking on a real timer.
`Instant::now()` in the test reads wall-clock, so the elapsed reflects only
the cooperative yield (~µs), not the configured grace.

**Why it's upstream**: This test pinned a *timing lower bound* that was
correct under the pre-`dd6437` `tokio::time::timeout` shape. The clock-trait
migration (intentionally, per `dd6437`'s commit message) moved grace-window
timing into injectable logical time. The test assertion shape was not
updated alongside the driver change. This is independent of the
`LiveAllocation::Terminated` removal in step 01-02 — my GREEN change
does not alter the `tokio::select!` race or the grace-window mechanism.

**Resolution options** (recommended order):
1. **Wire `SystemClock` into this specific test** — the test's purpose is to
   pin SIGKILL escalation behavior; it needs real wall-clock to validate
   the grace window. `Arc::new(overdrive_host::SystemClock)` (already a
   dev-dep on the worker crate's tests).
2. Remove the `elapsed >= 250ms` lower bound and assert only that
   `send_sigkill_pgrp` reaped the workload (the existing
   `await_sleep_grandchild_reaped` already validates the SIGKILL-fallback
   side-effect). The grace-window timing is now an internal `SimClock`
   contract, not a wall-clock observable.
3. Quarantine the assertion behind `#[ignore = "upstream-issue: see ..."]`.

**Out of scope for step 01-02**: My orchestrator instructions explicitly
list this file in `files_to_modify` for the `Ok(Terminated) → Err(NotFound)`
assertion rewrite only. Touching the timing assertion is a different
concern with its own RCA and trade-offs (which clock to wire where for
timing-sensitive tests). Tracked here so the next agent picks it up.

**Workaround applied for step 01-02 GREEN**: None — surfaced this file
and proceeded. The two regression tests from step 01-01
(`live_map_bounded.rs` worker, `sim_driver_live_map_bounded.rs` sim) are
the GREEN target this step is responsible for; both pass under Lima after
the variant removal.

## Issue 2 — Step 01-01 regression test had a `format!` / `&str` compile bug

**File**: `crates/overdrive-worker/tests/integration/exec_driver/live_map_bounded.rs`
(L59 — `AllocationId::new(format!(...))` — `new` expects `&str`)

**Origin**: Commit `a3606fb` (step 01-01 RED scaffold). Committed via
`--no-verify` per the RED scaffold protocol; the macOS `--no-run` gate
does not exercise the `#[cfg(target_os = "linux")]` test surface, so the
typo slipped through.

**Resolution**: One-character fix (`&format!(...)`). Applied as part of
step 01-02 GREEN — without this fix, the worker integration binary did
not compile on Linux and the regression test could not turn GREEN.
Outside the strict `files_to_modify` of step 01-02, but unavoidable to
make the predecessor RED test compile.
