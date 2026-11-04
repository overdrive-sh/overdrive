# Upstream issues — DISTILL to DESIGN

Feature `service-allocation-readiness-and-ending`. These are shapes or model facts the accepted design does not pin. DISTILL did not improvise any of them (CLAUDE.md "Implement to the design"; `.claude/rules/design.md`). **DISTILL is not complete until DG-1 to DG-12 are ruled on.** Delta references are to `../feature-delta.md`; spec files are under `specs/quint/service-allocation-readiness-and-ending/`. Summary table: delta § "Design gaps routed to DESIGN".

## Gaps that block activation of a conformance run or a composed test

### DG-1 — restart ceiling in the model is not the production ceiling
`restart_authority.qnt` instance `ra_conf` sets `CEIL = 2`; production `RESTART_BACKOFF_CEILING` is a const of 5 with no pinned seam. Seven give-up runs and the randomized run cannot activate: `WorkloadLifecycle` restarts again at the model's ceiling (`cur: 3`).
- Concerns: delta § Formal protocol model (`restart_authority`, "Actions map to"); `restart_authority.qnt`, `checks.toml`; driver `crates/overdrive-control-plane/src/reconciler_runtime/quint_restart_authority.rs`.
- Options: (a) instantiate `ra_conf` at `CEIL = 5` (spec only; larger state space, re-run `cargo xtask quint check`); (b) pin a ceiling seam (new API, needs an ADR-level decision).

### DG-2 — the model's pending row has no production step
The shim writes `Running` atomically with the awaited start. The model's `pending` row and `StartRejected` therefore have no production counterpart, and `restart_authority_non_crash_failed_row_gives_up_backoff_exhausted` cannot activate.
- Concerns: `restart_authority.qnt` (`Deploy`, `StartOk`, `StartRejected`); same driver.
- Options: (a) remove the pending split from the model; (b) pin a start-rejection step on a driving port.

### DG-8 — three G-10 boundary scenarios are not authored
Delta § Decisions D13 to D16, Gate G-10: (8) a ring drop leaves the reported set partial (composed run), (9) the guest init's own binds produce zero LISTEN lines, (10) loopback and v4-mapped binds. (8) needs the D7 stop, restart and give-up chain together with the host reader; (9) and (10) need a real-boot fixture workload binding specified addresses and a `GuestListenerBoard` read in the real composition. None is pinned, and no writer seam exists in `overdrive-init` (user ruling).
- Concerns: `guest_report.qnt` (full module, model check only); V34, V35, V36, V42 spike findings.
- Options: (a) pin the Tier-3 fixture and the board read in the real composition; (b) record (8) to (10) as spike-evidence obligations with no executable test.

### DG-9 — waivers
- S-32 restart and give-up leg (`LivenessProbeFailed` after restarts under the shared budget) is not composed: a restart needs `VmDriver::start` over `SimVmm` plus the beacon (the `quint_cid_lease` composition). The pure two-reconciler trajectory test covers the leg; D9 changes only the first row's timing, which `liveness_gate_composed.rs` pins.
- S-48 (b): there is no await point on a single-threaded sim between the insert and the wake; the design rests it on tokio's permit and mutation targets.
- S-48 (e): needs a reconciler whose tick blocks in flight; no production reconciler does, so adding one is a test seam.
- Options: accept the waivers, or pin the composition (S-32) and a seam (S-48 e).

## Gaps in model fidelity or projection

### DG-3 — `committed` in `liveness_gate`
The model keeps `committed` monotone across `Crash`; the pinned accessor reads the current `AllocStatusRow`, which the exit observer's `Failed` row replaces. The driver compares `committed` only while the row is `Running` (the spec's `rw`).
- Concerns: delta § Formal protocol model "Pinned conformance projection"; `liveness_gate.qnt`; driver `quint_liveness_gate.rs`.
- Options: state the restriction in the projection table, or change the model to forget `committed` on `Crash`.

### DG-4 — `stream_snapshot` projection has no owner object
The pinned projection is the yield of the free function `build_service_stream`. `cargo xtask conformance-lint` reports `projection-without-owner-accessor` at `crates/overdrive-control-plane/src/quint_stream_conformance/stream_snapshot.rs:384` (the one open lint violation).
- Concerns: delta § Formal protocol model, `stream_snapshot` rows; `.claude/rules/testing.md` "Per-driver checklist" items 4 and 5; `.claude/rules/design.md` "Pin the observable projection".
- Options: (a) rule the lint exemption shape for stream-yield projections; (b) pin an owner accessor (for example an `alloc_status_row` cross-check of the closing row).

### DG-5 — model guards in `stream_snapshot` and `progress_deadline`
`stream_snapshot.qnt`: `Deliver(a)` may take any channel member while the bus is FIFO (the driver fails with a typed `ModelAbstraction` error), and `Deliver` is enabled while `lagged` (the implementation relists first on the next poll). `progress_deadline.qnt`: `StableHappens` is enabled after `OperatorStop` (harmless in the driver). Both can fail `#[quint_run]` once D11 lands.
- Concerns: the two specs and `checks.toml`; drivers `stream_snapshot.rs`, `progress_deadline.rs`.
- Options: modeler guard fix (`Deliver` oldest-first and `not(lagged)`; guard `StableHappens` on no `OperatorStop`); re-run the checks and re-record evidence.

### DG-12 — `terminal_announced` membership after D6
Whether an allocation that started and then `Failed` stays in `ServiceLifecycleView.terminal_announced` after D6 is not pinned (only a pre-`Running` `Failed` is). The assertions on it in `crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs` were removed.
- Concerns: delta § D6, § D12 (eligibility keys on the committed `Stable`, not on membership).
- Options: pin the membership rule in the D6 interface changes, or state that membership is unobservable by contract.

## Gaps in pinned shape

### DG-6 — `ReportedListeners` construction and `GuestListenerBoard::new`
D13 item 10 pins `contains`, `len` and equality of `ReportedListeners` and the `GuestListenerReports` trait; it leaves the board's methods and outcome types internal. The Sim adapter and the oracles need to build values (`Default`, `FromIterator`, `is_empty` were added to the scaffold), and every composition needs a board constructor (`new() -> Self`). Production wiring currently gives each consumer a fresh board (inert); DELIVER shares one.
- Concerns: `crates/overdrive-core/src/traits/guest_listener_reports.rs`, `crates/overdrive-worker/src/guest_listener_board.rs`, `crates/overdrive-sim/src/adapters/guest_listener_reports.rs`.
- Options: pin the construction surface and constructor, or rule them the crafter's internal structure and keep the scaffold as written.

### DG-7 — positions and module path in the stream and CLI surface
The delta pins that `build_service_stream` gains a `progress_deadline` parameter and `submit_workload_streaming` a server deadline, but not their positions; `LastAttemptWire` has no pinned module. Scaffolded as: `progress_deadline` last in `build_service_stream`, `server_deadline` second in `submit_workload_streaming`, `LastAttemptWire` in `crates/overdrive-control-plane/src/streaming.rs`.
- Options: pin the three, or confirm the scaffold.

### DG-11 — where the listener set reaches the mechanic summary
D14 item 4 pins the text `guest listeners tcp:8080 udp:5353`, but `format_mechanic_summary` sees only the fieldless `ProbeMechanic::GuestListeners`. The scaffold arm prints the placeholder `guest listeners`.
- Concerns: `crates/overdrive-reconcilers/src/service_lifecycle.rs`, `crates/overdrive-cli/src/render.rs`.
- Options: pin the carrier (a parameter of the formatter, or the descriptor-plus-spec pair).

## Proposal for the user

### DG-10 — `tokio` `test-util` in `overdrive-cli`
The S-36 request-timeout probe pauses the clock over a real TLS socket and needs `tokio`'s `test-util` feature as a dev-dependency of `overdrive-cli` (the `Cargo.toml` edit is already in the tree). Failure it prevents: the Service lane's `deadline + 30 s` envelope regresses to the fixed 120 s with no test noticing. Cost: one dev-dependency feature, no runtime cost. Cheaper alternative: assert the computed timeout value only (does not exercise the client).
- Decision: approve or reject; on rejection the test is deleted.

## Minor, unpinned

- `BeaconParseError` variant for a rejected LISTEN line: `beacon_listen_wire_form.rs` asserts `Err` only.
- `Idle` in `guest_report_host`: in the `step` relation but model-only in the delta; the driver maps it to "the reader holds no outstanding grant".
- `service_stream_synth_cap_timeout`: the type of its "latest attempt-ending observation" parameter is unpinned; not scaffolded, no test calls it.
- S-53: the retry cadence of a failed `Pass` write and whether `<n>` in a `Fail` reason stays at the deadline value are unpinned; the tests tick up to eight intervals.
- D16 historical fixtures: V1 fixtures holding a probe descriptor with `inferred` set must be committed with the version bump (`.claude/rules/development.md` "Version-bump procedure"); DELIVER.
- Fixtures that construct `ProbeDescriptor` and `ProbeWitness` with `inferred` (stream, CLI and reconciler acceptance files, both stream drivers) lose the field when D16 lands.
- Pre-existing tests that assume liveness is live right after `on_alloc_running`, or that `broker().submit` is drained only by the clock (`runtime_convergence_loop.rs`, `interest_router.rs`, `cadence_resync.rs`, `probe_count_fixture`), are DELIVER's to revisit with D9 and D17.
- The CLI integration test `cli_submit_streaming_service_routes_to_service_submit_event_consumer` times out at HEAD `e5c233ba7` with and without these changes; verify before DELIVER whether the default startup gate (D13 to D15) is the cause.
