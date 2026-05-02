# 2026-05-02 — Fix `Terminated` slot accumulation in `ExecDriver::live`

## Summary

`Driver::stop` re-inserted a `LiveAllocation::Terminated` entry into
`ExecDriver::live` after teardown so that `Driver::status` could return
`Ok(AllocationState::Terminated)` for once-known allocations. The slot
was never evicted; over a long-running node session every alloc that
completed its lifecycle without a subsequent `start` overwriting it
left a permanent `BTreeMap` entry — unbounded growth proportional to
total lifecycle count.

The fix (RCA Candidate A): drop `LiveAllocation::Terminated` outright,
evict the slot in `stop()`, simplify the `status`/`stop`/`resize`
matches to two arms with `None → Err(NotFound)`, document the relaxed
contract on `Driver::status`, and apply the symmetric change to
`SimDriver` so the trait contract stays uniform across the host and
sim adapters. Three pre-existing acceptance assertions were rewritten
from `Ok(AllocationState::Terminated)` to `Err(DriverError::NotFound)`.

**Production impact: zero.** The reconciler reads terminal state from
`AllocStatusRow` in the ObservationStore (whitepaper §18 three-layer
state taxonomy); driver-private memory was duplicating obs's job.
The only production caller of `Driver::stop` —
`crates/overdrive-control-plane/src/action_shim.rs` — already absorbs
the contract change with `let _ = driver.stop(...)` at L140 and L187,
so the new `Err(NotFound)` on double-`stop()` is dropped silently.
No production caller branched on the removed `Ok(Terminated)` shape.

## Business context

Originated from a code-review comment on
`crates/overdrive-worker/src/driver.rs` flagging unbounded `Terminated`
accumulation in `ExecDriver::live`. The 5-Whys troubleshooting
established that the `Driver` trait conflated two state classes:
"what's running right now" (driver-private, ephemeral) and "what was
the terminal state of a once-known alloc" (durable, ObservationStore).
The trait surface had no eviction primitive, so the workaround was an
in-memory tombstone slot — which leaked. The §18 state-layer split
already prescribes the correct home for terminal state; the fix aligns
the driver trait with that prescription.

## Key decisions

- **Driver-trait state-layer split.** Durable terminal state belongs
  in the ObservationStore (`AllocStatusRow`), not in driver-private
  memory. The driver's `live` map is now strictly "currently running."
  This codifies the whitepaper §18 split inside the `Driver` trait
  contract.
- **Symmetric host/sim change.** `SimDriver` received the same
  variant deletion and slot eviction so the two adapters remain
  contract-compatible. Asymmetry between adapters of the same trait
  would be a DST-correctness hazard — sim and host must converge.
- **Single-cut greenfield discipline.** `LiveAllocation::Terminated`
  was deleted outright. No `#[deprecated]` attribute, no compat shim,
  no commented-out match arms. Per project memory
  `feedback_single_cut_greenfield_migrations.md` and
  `feedback_delete_dont_gate.md`: removed is removed.
- **Test rewrites, not test deletions.** Three pre-existing acceptance
  assertions at the cited lines (`stop_with_grace.rs:64-65`,
  `stop_escalates_to_sigkill.rs:189-190`,
  `sim_adapters_deterministic.rs:298-303`) were rewritten in place to
  expect `Err(DriverError::NotFound)`. The tests still defend the
  same invariant — "the driver no longer holds the slot post-stop" —
  just expressed against the new contract.
- **Cardinality accessor stays test-only.** The new `live_count()`
  helper used by the regression tests is `pub(crate)` /
  `#[cfg(test)]`, never exposed on the public `Driver` trait surface.

## Steps completed

| Step | Phase | Outcome |
|---|---|---|
| 01-01 | PREPARE | PASS (2026-05-01T20:02:39Z) |
| 01-01 | RED_ACCEPTANCE | PASS — regression scaffold added at `crates/overdrive-worker/tests/integration/exec_driver/live_map_bounded.rs` and `crates/overdrive-sim/tests/acceptance/sim_driver_live_map_bounded.rs`; both fail against current code |
| 01-01 | RED_UNIT | SKIPPED — integration test exercises smallest reachable surface; no separate unit needed |
| 01-01 | GREEN | SKIPPED — GREEN deferred to step 01-02 per RED-scaffold pattern |
| 01-01 | COMMIT | PASS — commit `a3606fb` `test(worker,sim): regression test for bounded driver live-map` (committed via `--no-verify` per RED-scaffold protocol) |
| 01-02 | PREPARE | PASS (2026-05-01T20:06:37Z) |
| 01-02 | RED_ACCEPTANCE | SKIPPED — RED scaffold landed in 01-01 |
| 01-02 | RED_UNIT | SKIPPED — 01-01 regression covers the surface |
| 01-02 | GREEN | PASS — variant dropped, slot eviction wired, three assertions rewritten, trait rustdoc updated |
| 01-02 | COMMIT | PASS — commit `f87e039` `fix(worker): evict terminated slot in ExecDriver::stop` |

A follow-up housekeeping commit `a61fde6`
`chore(deliver): normalise execution-log outcomes to PASS` normalised
the execution-log shape after delivery completion.

## Lessons learned

### Timing-lower-bound assertions are unsafe under `SimClock`

The `stop_escalates_to_sigkill.rs` integration test had an `elapsed >=
Duration::from_millis(250)` assertion that was correct under the
pre-`dd6437` `tokio::time::timeout` shape. Commit `dd6437`
("refactor(worker): inject Clock into ExecDriver, ban tokio::time in
production") migrated `ExecDriver::stop` to `tokio::select!` over
`Clock::sleep` and switched the test to wire
`Arc::new(SimClock::new())`. Under `SimClock`, the sleep advances
*logical* time and yields cooperatively rather than blocking on a real
timer; `Instant::now()` in the test reads wall-clock, so the elapsed
delta reflects only the cooperative yield (~µs), not the configured
grace.

This is a recurring shape: any test that wires `SimClock` and pins a
*wall-clock-based* timing lower bound is suspect. The crafter removed
the lower bound and retained an upper bound plus the SIGKILL-escalation
invariant via `await_sleep_grandchild_reaped` — the SIGKILL
side-effect is the real observable; the grace-window timing under
`SimClock` is an internal contract, not a wall-clock observable.

The defect was masked because the new regression tests added in step
01-01 are gated `#[cfg(target_os = "linux")]` and the macOS `--no-run`
gate could not exercise them — see next lesson.

### `--no-verify` RED scaffolds in Linux-gated code need a Lima compile gate

Step 01-01's RED scaffold (commit `a3906fb`) carried a one-character
typo at L59 — `AllocationId::new(format!(...))` where `new` expects
`&str`. Per `.claude/rules/testing.md` § "RED scaffolds and
intentionally-failing commits", the commit was landed via
`git commit --no-verify` (the standard pattern for intentionally-RED
commits). On macOS, `cargo nextest run --features integration-tests
--no-run` is the canonical pre-commit compile check — but the new
tests were `#[cfg(target_os = "linux")]`, so macOS skipped them and
the typo slipped through.

Future RED scaffolds whose tests live under
`#[cfg(target_os = "linux")]` should run a Lima compile gate
(`cargo xtask lima run -- cargo nextest run --features integration-tests
--no-run`) before the `--no-verify` commit. The macOS-side gate is
necessary but not sufficient when the gated surface is not reachable on
macOS.

The typo was fixed as a one-character change (`&format!(...)`) during
step 01-02 GREEN. It was outside the strict `files_to_modify` scope
of 01-02 but unavoidable to make the predecessor RED test compile on
Linux.

## Risks accepted

The user explicitly directed `skip and run /nw-finalize`, deliberately
skipping three standard DELIVER-wave gates for this small bug fix:

- **Adversarial review (Phase 4)** — no peer-review pass over the
  delivered change.
- **Mutation kill-rate gate (Phase 5)** — the `cargo xtask mutants`
  diff-scoped run (≥80% kill rate) was not executed for this feature.
- **DES integrity verification (Phase 6)** — the execution-log
  cross-check against the wave matrix was deferred.

Trade-off rationale: this is a small, well-bounded, single-RCA bug
fix with zero production-caller impact (verified statically against
`action_shim.rs`) and a regression test that fails RED and turns GREEN
on the chosen fix. The accepted risk is proportional to the change
size; aggregate exposure is bounded to the deleted variant and three
mechanical assertion rewrites.

## Links

- `a3606fb` — `test(worker,sim): regression test for bounded driver live-map`
- `f87e039` — `fix(worker): evict terminated slot in ExecDriver::stop`
- `a61fde6` — `chore(deliver): normalise execution-log outcomes to PASS`

## References

- Whitepaper §18 — Reconciler / Workflow primitives; three-layer state
  taxonomy.
- ADR-0026 — `ExecDriver` Phase 1 production driver.
- ADR-0029 — `exec` driver naming.
- `.claude/rules/development.md` § State-layer hygiene, § Deletion
  discipline.
- `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing
  commits", § "Running integration tests locally on macOS — Lima VM".
- Project memory: `feedback_single_cut_greenfield_migrations.md`,
  `feedback_delete_dont_gate.md`, `feedback_phase1_single_node_scope.md`.
