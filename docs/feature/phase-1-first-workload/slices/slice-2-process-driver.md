# Slice 2 — ProcessDriver (cgroup-aware, gated `integration-tests`)

**Story**: US-02
**Walking skeleton row**: 2 (Start a process)
**Effort**: ~1 day (Linux developer; ~2 days if the developer is on macOS and needs a Linux VM in the loop)
**Depends on**: phase-1-foundation `Driver` trait, `AllocationSpec`, `AllocationHandle`, `AllocationState`, `Resources`. Independent of Slices 1 and 3 mechanically.

## Outcome

`ProcessDriver` in `crates/overdrive-worker/src/driver/process.rs` (per ADR-0029 — relocated from `overdrive-host` by the 2026-04-27 worker-crate extraction amendment) implements the `Driver` trait against `tokio::process::Command` and cgroups v2. `Driver::start` spawns a child process, creates a workload cgroup scope at `overdrive.slice/workloads.slice/<alloc_id>.scope`, places the child PID into `cgroup.procs`, and returns an `AllocationHandle` carrying the PID and the scope path. `Driver::status` polls `/proc/<pid>` (or the cgroup) and returns the live `AllocationState`. `Driver::stop` sends SIGTERM, waits a configurable grace, escalates to SIGKILL if needed, then removes the cgroup scope.

Default unit tests use `SimDriver` and exercise the trait surface with no real processes. A Linux-only integration test (gated behind `integration-tests` feature, in `crates/overdrive-worker/tests/integration/process_driver.rs`) actually starts `/bin/sleep 60` inside a real cgroup scope, asserts `/proc/<pid>/cgroup` matches, then stops it cleanly.

## Value hypothesis

*If* we can't get a clean `ProcessDriver` impl in `overdrive-worker` that confines its children in a workload cgroup scope without polluting `overdrive-core`'s compile path, *then* the `adapter-host` boundary doesn't actually pay for itself — banned-API discipline gets undermined the first time a real driver lands. *Conversely*, if we can — and the integration test proves the cgroup placement on a real Linux kernel — every future driver type (microvm, wasm) has a known-good template.

## Disproves (what's the named pre-commitment we're falsifying)

- **"We need a separate cgroup-management abstraction layer above the Driver."** No — the driver IS the cgroup-aware spawn site. Each driver knows how to confine its own workloads; the §14 right-sizing reconciler reads cgroup signals later, but it doesn't manage placement.
- **"Process spawn can be hidden behind `Driver` without touching the host crate's compile path."** Trivial yes; the test is whether the spawn actually works under the integration-tests feature without leaking into the default lane.

## Scope (in)

- `ProcessDriver` struct + `Driver` impl in `crates/overdrive-worker/src/driver/process.rs` (per ADR-0029).
- `CgroupPath` newtype (FromStr, Display, validation; in `overdrive-worker` per ADR-0029 since the only caller is workload-cgroup management). STRICT-newtype obligation — see System Constraints in `user-stories.md`.
- cgroup scope creation/teardown helpers (direct cgroupfs writes per ADR-0026; no `cgroups-rs` dep).
- `tokio::process::Command` spawn + PID capture into `AllocationHandle`.
- `Driver::stop` SIGTERM → grace → SIGKILL escalation.
- Integration test gated `integration-tests` feature; in `crates/overdrive-worker/tests/integration/process_driver.rs`.

## Scope (out)

- Action shim that calls `Driver::start` (Slice 3).
- AppState extension to hold `Arc<dyn Driver>` (Slice 3).
- Control-plane slice creation (Slice 4).
- cgroup pre-flight delegation check (Slice 4).
- Resource enforcement on the cgroup scope (`cpu.weight`, `memory.max` — DESIGN picks whether to wire these in this slice or defer to a §14 right-sizing follow-on; the codebase research's mapping says this slice should wire them since `Resources` is already on `AllocationSpec`).

## Target KPI

- Linux integration test passes: `Driver::start` returns a handle whose PID is alive and whose cgroup scope exists at the expected path; `Driver::status` reports Running; `Driver::stop` removes the scope.
- Default-lane unit tests against `SimDriver` continue to pass (no regression).
- `dst-lint` does NOT flag `ProcessDriver` — `overdrive-worker` is `adapter-host` class per ADR-0029, not scanned.

## Acceptance flavour

See US-02 scenarios. Focus: real process spawn under integration-tests gate, cgroup scope placement verifiable via `/proc/<pid>/cgroup`, clean SIGTERM-then-SIGKILL escalation.

## Failure modes to defend

- Binary path doesn't exist: returns structured `DriverError::BinaryNotFound { path }`.
- cgroup scope creation fails (permission denied, cgroup v2 not delegated): returns structured error; this is the path Slice 4's pre-flight check makes detectable BEFORE process spawn.
- Process exits between spawn and PID capture: `Driver::status` returns `AllocationState::Terminated`.

## Slice taste-test

| Test | Status |
|---|---|
| ≤4 new components | PASS — ProcessDriver + CgroupPath newtype + cgroup helper + spawn helper (4, at the upper end) |
| No hypothetical abstractions landing later | PASS — uses existing Driver trait; ADR-0026 picks direct cgroupfs writes (no `cgroups-rs` dep) |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — Linux integration test against a real /bin/sleep |
| Demonstrable in single session | PASS — `cargo nextest run -p overdrive-worker --features integration-tests` + observe the cgroup with `systemd-cgls` |
| Same-day dogfood moment | PASS — Linux developer can run the integration test on their workstation |
