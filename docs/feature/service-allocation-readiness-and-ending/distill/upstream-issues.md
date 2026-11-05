# Upstream issues — DISTILL to DESIGN

Feature `service-allocation-readiness-and-ending`. These are shapes or model facts the accepted design did not pin. DISTILL did not improvise any of them (CLAUDE.md "Implement to the design"; `.claude/rules/design.md`).

**All of DG-1 to DG-12 and the minor items were ruled on 2026-11-05** (user: "proceed with recommendations"). The approval is recorded in the feature delta's § Approval record (rows DG-1 to DG-12 and DG-M). **No gap remains open; DISTILL is complete, pending the independent review.** Each "Action for DISTILL" below is done: the specs, drivers, scaffolds and tests follow the rulings. Delta references are to `../feature-delta.md`; spec files are under `specs/quint/service-allocation-readiness-and-ending/`. Summary table: delta § "Design gaps routed to DESIGN".

## Gaps that blocked activation of a conformance run or a composed test

### DG-1 — restart ceiling in the model is not the production ceiling — RESOLVED
`ra_conf` set `CEIL = 2`; production `RESTART_BACKOFF_CEILING` is 5 (`workload_lifecycle.rs:34`).
- **Ruling:** option (a). `ra_conf` is instantiated at `CEIL = 5`; the give-up runs are rewritten to five restarts; the driver asserts that the spec's `CEIL` equals the const (test-only); `ra_ok` is unchanged. No production change and no ceiling seam.
- **Done:** `restart_authority_*` runs at `CEIL = 5` (21 named runs plus `#[quint_run]`); five pass at HEAD, 16 and the random run are ignored with right-reason RED; no G-R1 block remains.

### DG-2 — the model's pending row has no production step — RESOLVED
The shim writes `Running` atomically with the awaited start.
- **Ruling:** option (a). `restart_authority` drops its `pending` row and its `StartOk` and `StartRejected` actions. The start outcome (`ok` or `rejected`) is a nondeterministic parameter of `Deploy` and `WLRestart`, matching the shim (`Running` on success, `Failed` on `DriverError::StartRejected` within the awaited tick, `action_shim/mod.rs:~2433-2459`). The rejected-start named run is kept. The driver scripts the rejection through `SimDriver::script_next_start_rejection` (sim test support, DISTILL's). `non_crash_failed_row_gives_up_backoff_exhausted` now passes at HEAD.

### DG-8 — three G-10 boundary scenarios were not authored — RESOLVED
- **Ruling:** (9) init's own binds and (10) loopback and v4-mapped binds are Tier-3 real-boot DELIVER obligations of D13 and D14, observed through production-written state: the fixture declares a port that init itself binds (resp. a workload that binds only a loopback or v4-mapped loopback address), and the startup probe row is `Fail` at the deadline naming that port as unreported. (8) the composed ring drop is a seeded Tier-1 composed run (`VmDriver` over `SimVmm` and a stepped `SimBeaconIo`, the production exit observer and the real reader): the fault is the omitted `LISTEN` line on the existing `SimBeaconIo`, because a ring drop is in-guest and its only host-visible effect is that the line never arrives. The kernel's drop-and-count behaviour stays V37's evidence and the `ring_drop_leaves_set_partial` model run. No writer seam in `overdrive-init`.
- **Action for DISTILL:** author (8) with the S-32 composition. **Done:** `crates/overdrive-control-plane/src/reconciler_runtime/guest_listener_gate_composed.rs::a_service_whose_listener_report_never_arrives_is_stopped_at_the_deadline_restarted_and_given_up` (seeded, four seeds, seed printed; the real `VmDriver::start` over `SimVmm` and `SimBeaconIo`, the shared `GuestListenerBoard`, the real `ProbeRunner`, `ServiceLifecycle`, `WorkloadLifecycle`, the shim and the exit observer). The fault is the omitted `guest_write` of one declared listener's `LISTEN` line; the oracle is no row before the deadline, the first `Fail` naming only the unreported listener, `Stopped { StartupProbe }`, restart, and the give-up `StartupProbeFailed`; never `Pass`, never `Stable`. Fixture extension: `LivenessWorld::compose_restartable_guest_listeners`, `guest_writes`, `startup_rows`; the fixture now shares one board between driver and runner (DG-6). (9) and (10) remain Tier-3 DELIVER obligations.

### DG-9 — waivers — RESOLVED
- **Ruling:** the S-32 restart and give-up leg is required and is composed with the `quint_cid_lease` composition (`VmDriver` over `SimVmm` plus the stepped beacon). S-48 (b) is waived and carried by the mutation gate: its evidence is the gate killing the D17 signal-position mutants (moved before the insert, dropped, stored permit). If the gate cannot generate or kill one of them, (b) has no evidence. S-48 (e) is waived: it needs a reconciler whose tick blocks in flight, which would be a test seam.

## Gaps in model fidelity or projection

### DG-3 — `committed` in `liveness_gate` — RESOLVED
- **Ruling:** the projection table states that `committed` is compared only while the row is `Running`; after `Crash` it is model-only. The model is unchanged.

### DG-4 — `stream_snapshot` projection has no owner object — RESOLVED
`conformance-lint` reported `projection-without-owner-accessor` at `stream_snapshot.rs:384`.
- **Ruling:** option (b). The compared state is the closing `AllocStatusRow` read through the existing `ObservationStore::alloc_status_row`. The line `build_service_stream` yields is an input and an oracle, not a projected variable. No lint exemption. `conformance-lint` reports 0 violations.

### DG-5 — model guards in `stream_snapshot` and `progress_deadline` — RESOLVED
- **Ruling:** the channel is an ordered list; `Deliver` takes the oldest event and is disabled while `lagged`. The `Lag` abstraction (the model drops every unseen event; tokio reports `Lagged` and resumes at the oldest retained event) is stated and harmless: the relist reads the store and commit precedes publish. `StableHappens` is disabled after `OperatorStop`.
- **Done:** the `stream_snapshot` driver (7 runs plus random) and the `progress_deadline` driver (5 runs RED on the D10 scaffold) follow the guards.

### DG-12 — `terminal_announced` membership after D6 — RESOLVED (changes D6)
The unpinned case was a started allocation that then `Failed`. Verified at HEAD: `observed` is never pruned (`service_lifecycle.rs:514`), the only `terminal_announced` insertion for a crash was the early-exit one that D6 deletes, and the pre-`Running` insertion (`:592`) does not cover a started row. So the allocation would stay in `has_alloc_mid_startup_window` (`:357-361`), which gates `next_evaluation_at` (`:701`).
- **Ruling:** every allocation `ServiceLifecycle` observes in `Failed` or `Terminated` joins `terminal_announced`, whatever its `started_at`. D6 item 1, the D6 interface-changes row, Gate G-4 (unaffected list) and ADR-0258 carry it. The `restart_authority` model's `ann` reflects it (the modeler's edit).
- **New obligation:** S-56 (pure proptest, mutation target). Oracle: the returned `View.terminal_announced` contains the allocation, `has_alloc_mid_startup_window()` is false, `next_evaluation_at` is `None`, no action.
- **Done:** S-56 is `service_lifecycle_reconcile_branches.rs::terminal_allocation_of_any_start_history_closes_its_mid_startup_window`, ignored D6.

## Gaps in pinned shape

### DG-6 — `ReportedListeners` construction and `GuestListenerBoard::new` — RESOLVED
- **Ruling:** pinned. `ReportedListeners`: `Default`, `FromIterator<Listener>`, `is_empty`, beside `contains`, `len` and equality. `GuestListenerBoard::new() -> Self`; the scaffolds follow (`FromIterator<Listener>` is implemented). D13 names the DELIVER obligation that production composition shares ONE board between the probe runner and `VmDriver` (the scaffolds create one per consumer).

### DG-7 — positions and module path in the stream and CLI surface — RESOLVED
- **Ruling:** the scaffold is confirmed and pinned. `progress_deadline: Duration` is the last parameter of `build_service_stream`; the server deadline is the second parameter of `ApiClient::submit_workload_streaming`; `LastAttemptWire` lives in `overdrive_control_plane::streaming`.

### DG-11 — where the listener set reaches the mechanic summary — RESOLVED
- **Verified at HEAD:** no wire field is needed. The deploy stream prints `ProbeWitness.mechanic_summary` verbatim (`render.rs:1283`); the reconciler fills it in `spec_facts_for_service(svc: &Service)`, which holds `svc.listeners`. The describe path renders `format_probe_mechanic_summary` and already carries `out.snapshot.listeners` (`render.rs:274`).
- **Ruling:** both formatters (the reconciler's `format_mechanic_summary`, the CLI's `format_probe_mechanic_summary`) gain a second parameter `&[Listener]`, used by the `GuestListeners` arm. `ProbeMechanic::GuestListeners` stays fieldless. Text: `guest listeners tcp:8080 udp:5353`, declaration order. Ignored D14 text tests exist at both formatters.

## Proposal for the user

### DG-10 — `tokio` `test-util` in `overdrive-cli` — APPROVED
- **Ruling:** approved. The S-36 test sits behind `integration-tests` and carries a litmus proof that it fails when the `deadline + 30 s` envelope is removed. Litmus: the Service lane fails `waited 120.0016s` against the required 180 s (envelope absent at HEAD); the Job lane passes.

## Minor, unpinned — RESOLVED

- `BeaconParseError` for a rejected `LISTEN` line: `MalformedListen { raw: String, detail: String }`, in the shape of `MalformedArgv`. Every rejected `LISTEN` line returns it (DG-M). Scaffolded; the LISTEN wire-form tests assert it.
- `Idle` in `guest_report_host`: a read-only `GuestListenerReports::reported(alloc)` call compared with the previous projected value (DG-M). Implemented in the driver.
- D16 historical fixtures: V1 fixtures holding a probe descriptor with `inferred` set land with the version bump in DELIVER (DG-M). The fixtures that construct `ProbeDescriptor` and `ProbeWitness` with `inferred` lose the field when D16 lands.

## Not part of the DG rulings (unchanged)

- `service_stream_synth_cap_timeout`: the type of its "latest attempt-ending observation" parameter is unpinned; not scaffolded, no test calls it.
- S-53: the retry cadence of a failed `Pass` write and whether `<n>` in a `Fail` reason stays at the deadline value are unpinned; the tests tick up to eight intervals.
- Pre-existing tests that assume liveness is live right after `on_alloc_running`, or that `broker().submit` is drained only by the clock (`runtime_convergence_loop.rs`, `interest_router.rs`, `cadence_resync.rs`, `probe_count_fixture`), are DELIVER's to revisit with D9 and D17.
- The CLI integration test `cli_submit_streaming_service_routes_to_service_submit_event_consumer` times out at HEAD `e5c233ba7` with and without these changes; verify before DELIVER whether the default startup gate (D13 to D15) is the cause.
