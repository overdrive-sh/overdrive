# Feature delta — `service-allocation-readiness-and-ending` (GH #318, GH #334)

> DESIGN wave, independently APPROVED at full review iteration 4 on 2026-11-09;
> authored by Morgan (nw-solution-architect), 2026-11-02.
> Source of truth for this feature's exact interface contract. D1 to D17 are
> **Accepted** after full DESIGN review iteration 4 (§ Approval record). B1 (ADR-0265), B2 (ADR-0266) and B3 (ADR-0258) are
> accepted, B5 is rejected, B4 is D12 (ADR-0267; it amends the operative ADR-0096).
> **D13 to D16 (guest-reported listener readiness) are accepted; V41 (boot cost)
> and V42 (workload cgroup) are held.** Four items inside
> D13 and D14 are ruled (2026-11-04): **PR-1** the driver-to-probe-task seam (D13 item 10),
> **PR-3** the `Fail` cadence (D14 item 3) and **PR-4** one declared-listener field (D14
> item 2) are accepted; **PR-2** the scope of the listen hooks (D13 items 1 and 7) is
> accepted in contract shape (direction approved 2026-11-04 pending V42; V42 held
> 2026-11-04, `spike/findings-workload-cgroup.md`). Three round-4 items are accepted
> (2026-11-04, "proceed with recommendations"): **P1** the startup stop gate's attempts
> threshold comes from the startup descriptor, **A1** the board entry's lifetime, **A3** the
> host-input conformance projection of the report model; **R1** (`VmmExit` and the
> three-valued `exitrep`), **R1-a** (`exitrep` is read from the exit observer's row) and
> **R2** (model trims) are also accepted.
> The independent full DESIGN review approved this exact revision in
> [`design/review.md`](design/review.md), iteration 4. The independent full
> DISTILL review approved the acceptance suite in [`distill/review.md`](distill/review.md),
> iteration 4. Roadmap validation remains a separate pending gate before DELIVER.
> **User-approved 2026-11-09:** D11-E1 (typed Service stream error), D14-A1 (owned
> TCP view signature), and D14-E2 (typed GuestListeners input rejection) amend the
> exact contract below. The bounded technical review approved these shapes in
> [`design/review-contract-remediation.md`](design/review-contract-remediation.md),
> iteration 2; the full D1–D17 DESIGN review approved them at iteration 4.
> **User-approved 2026-11-09:** exact D13-S1 and D17-S1 signatures are pinned
> below (user: “rest is fine”). For D16, the user explicitly waived only
> historical-byte/backward-read compatibility for this greenfield feature:
> “"preserve historical bytes" -> what do um ean? we dont need to preserve
> anything yet. this is greenfield. not used in production. rest is fine”.
> **D16-G1 accepted under that exact greenfield waiver:** the five affected
> current envelope payload layouts are rebaselined in place under existing tags.
> The exact current-format contract is below; the full review approved it at
> iteration 4. Current lifecycle, readiness and typed-error outcomes remain unchanged.
> The research that supports D7 to D10 is
> `docs/research/orchestration/probe-failure-restart-authority-and-give-up-reporting-research.md`
> (Kubernetes @7b4bd5d3, Nomad @64cdf9f9, source-cited); it is cited in each
> decision record.
> No DISCUSS wave exists. Inputs: `docs/analysis/root-cause-analysis-crash-restart-predecessor-probe-leak.md`
> (the RCA), the task text for #318 and #334 (neither issue has comments), and the
> source. **Every `path:line` citation in this document is pinned at commit
> `04052ad75`** (no file under `crates/` differs at the current branch head).

## Wave: DESIGN / [REF] Premise, grounded on the production path

**Problem.** `Driver::on_alloc_terminal` is the only thing that cancels an
allocation's probe supervisor (`VmDriver::on_alloc_terminal` →
`ProbeRunner::stop_alloc`, `crates/overdrive-worker/src/vm_driver.rs:2061-2063`,
`crates/overdrive-worker/src/probe_runner/mod.rs:374-379`). Its only production
callers are the action shim's `FinalizeFailed` genuine-terminal arm
(`crates/overdrive-control-plane/src/action_shim/mod.rs:2176`) and
`StopAllocation` arm (`mod.rs:3272`). A natural guest exit is ended by neither.

**Production path that produces the state (ground-the-premise check).**
`overdrive serve` composes one exit observer per registry driver
(`crates/overdrive-control-plane/src/lib.rs:4512-4526`). A guest exit travels
`run_exit_watcher` → `ExitEvent` on the driver's channel
(`vm_driver.rs:2315-2329`) → exit observer → `Failed` / `Terminated` row
(`exit_observer.rs:208-306`). Nothing on that path touches the probe runner. Then:

- **#334.** The supervisor of a crashed allocation keeps probing a dead guest for
  the whole restart backoff, or for the rest of the process if no restart comes.
- **#318.** `WorkloadLifecycle` restarts under a fresh allocation id
  (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:1220-1222`, ADR-0105),
  so `cleanup_restart_predecessor` (`action_shim/mod.rs:1776-1805`) cleans the
  predecessor's lease, driver, mTLS and netns but never its supervisor. The
  successor's `on_alloc_running` registers a second supervisor.
- **Wider than "probe-bearing" allocations.** `ProbeRunner::start_alloc` calls
  `register_alloc`, which inserts a supervisor entry for every allocation, with
  or without descriptors (`probe_runner/mod.rs:256-259`, `:307-308`). So
  `active_alloc_count()` grows by one per VM crash-restart of any VM workload.

Both states occur under real `serve` + `deploy` with real adapters. No test
override is involved.

## Wave: DESIGN / [REF] Approval record

User rulings of 2026-11-02, recorded verbatim in outcome:

| ID | Ruling | Recorded in |
|---|---|---|
| D1 | Approved as written | ADR-0255 |
| D2 | Approved as written | ADR-0255 |
| D3 | Approved as written | ADR-0256 (split out of ADR-0255, one decision per ADR) |
| D4 | Approved as written | ADR-0255 |
| BC-1 | Behaviour change approved: a crashed service allocation no longer has its startup-probe failure streak grown by probing a dead guest; the crash row is the recorded cause and `WorkloadLifecycle` (the single restart authority) decides restart or give-up. User: "yes, new behavior is correct." | ADR-0255 Consequences; § Answers Q3; S-10 |
| D5 | Approved as proposed: `startup_probe_failed_action` skips any terminal row (`AllocState::is_terminal`: `Failed` and `Terminated`), so `ServiceLifecycle` cannot finalise a crashed allocation as a startup-probe failure. | ADR-0257 |
| D6 | Approved as proposed (user: "proceed with recommendations"), 2026-11-02. The early-exit verdict moves to `WorkloadLifecycle` at give-up. **K1 restatement, option (a), approved:** the `EarlyExit` honesty bound moves to "when the restart budget is spent" (§ Changed and unchanged assumptions). | ADR-0258 (Accepted) |
| P-3, P-4, P-5 | **Not in scope, by user ruling 2026-11-02.** The deploy stream staying silent between the first crash and the give-up is accepted. No per-attempt wire event (P-3), no attempt count on the early-exit reason (P-4), no classification of a pre-startup clean exit (P-5). | § Proposals not included |
| F-1, F-2 | Filed by the user's ruling as GH #335 (F-1) and GH #336 (F-2). | § Findings |
| D7 | **Approved as written, 2026-11-02** ("proceed with recommendations"), including the restated K1 for a Service that boots but never passes its startup probe (the `StartupProbeFailed` terminal arrives at give-up), the new public variant `StoppedBy::StartupProbe`, the changed value meaning of `StartupProbeFailed.attempts` (restart budget consumed, as for `LivenessProbeFailed`), and D5's changed user-visible sentence. | ADR-0259 (Accepted) |
| D8 | **Approved, 2026-11-02** (mirror of the liveness retry): a startup stop that does not land is retried once per new written startup probe row while the allocation is live. One additive CBOR `View` field with `#[serde(default)]`. | ADR-0260 (Accepted) |
| D8 item 3 | **Confirmed, 2026-11-02 ("yes to all", ruling 1):** a startup `Pass` after a stop that did not land ends the retry, removes the allocation from `terminal_announced` when `Stable` is announced (so liveness, which skips members, acts on it; the backend veto no longer exists, ruling R2-1). | ADR-0260 |
| D9 | **Approved, 2026-11-02**: liveness supervision begins at `Stable`, so liveness acts only once the startup probe has passed or no startup probe is declared. Owner: the probe runner, started from the existing `on_alloc_stable` hook. | ADR-0261 (Accepted) |
| D9 extension | **Confirmed, 2026-11-02 (ruling 2):** D7 item 5's stream projection returns `None` for `Stopped { by: StartupProbe }` and for `Stopped { by: LivenessProbe }`. | ADR-0259, ADR-0261 |
| D10 | **Approved in full, 2026-11-02 (ruling 3), every shape choice in § D10** (server-side stream handler owns it; `max(streaming_cap, startup_deadline + 30 s)`, derived, no spec key or flag; clock at stream open, not extended; optional `last_attempt: LastAttemptWire { alloc_id, reason, terminal, detail }` on `ServiceSubmitEvent::Failed`; `ServiceFailureReason` untouched; CLI request timeout = deadline + 30 s; the timeout wording; K1 as restated). | ADR-0262 (deadline) and ADR-0264 (`last_attempt` detail, split out of ADR-0262 on review; content unchanged) |
| F-4 | Included 2026-11-02: add the missing ADR-0087 golden fixture for `StoppedBy::LivenessProbe` beside D7's `StartupProbe` fixture. Test support in an existing format; a DISTILL obligation (S-25), not a design decision. | § Obligations |
| F-5 | **Verified real, 2026-11-02 (ruling 4: "check now and, if real, fold the fix into this design").** Code evidence in § Findings. The fix is D11. | § D11, ADR-0263 (Accepted) |
| D11 | **Approved in full, 2026-11-03 (ruling "proceed with recommendations")**, all seven shape choices in § D11 exactly as proposed: the stream subscribes then reads the recorded verdict; reuse of `current_alloc` and `service_event_from_terminal` (the original no-new-surface clause is amended by D11-E1 below); only the current attempt's own terminal counts (restartable probe stops are not a verdict); relist on `Lagged`; a failed read ends the stream with an error; any recorded verdict is reported, including `Stopped { Operator }` (exit 0); no new timer or config. | ADR-0263 (Accepted, amended D11-E1) |
| F-6 | **Filed as GH #340 (type Bug), 2026-11-03**, on the user's ruling. Not changed by this feature. | § Findings |
| B1 | **Accepted as proposed, 2026-11-03 ("proceed with recommendations").** `Stable` is announced from the committed row (`ServiceAllocFact.stable_committed`); the `stable_announced` marker no longer decides emission. | ADR-0265 (Accepted) |
| B2 | **Accepted as proposed, 2026-11-03.** The `StopAllocation` arm fires `on_alloc_terminal` after the terminal row write succeeds; amends D4's and D7 item 4's "the shim is unchanged" for this one statement. | ADR-0266 (Accepted) |
| B3 | **Accepted as proposed, 2026-11-03.** D6's pre-startup-crash predicate accepts `WorkloadCrashedImmediately` and `VmOutOfMemory` (`exit_code` is `None` for the latter); fail-closed supersession reasons stay out. | ADR-0258 (text amended) |
| B4 | **Decided by the user, 2026-11-03: the orchestrator's recommendation, not parity.** Kubernetes semantics: any allocation whose Service declares a startup probe is not backend-eligible until its own startup has passed, first allocation or restart successor. Recorded as **D12**; amends the operative ADR-0096. | ADR-0267 (Accepted), ADR-0096 (amendment note) |
| B5 | **Rejected, 2026-11-03.** `PROGRESS_DEADLINE_MARGIN` stays the literal 30 s as written in D10; DL-6, E-4 and V30 stand. No derived margin. | D10 |
| R2-1 | **Approved, 2026-11-03.** The redundant ADR-0096 per-allocation backend veto and the veto-before-stop ordering are removed. | ADR-0267, ADR-0096 (supersession note), § Interface contract changes |
| R2-2 | **Approved, 2026-11-03 ("remove it if no reader remains").** No reader remains after B1, so the `stable_announced` view field is removed. | ADR-0265, § Interface contract changes |
| D13, D14, D15 | **User-approved in direction, 2026-11-03 ("add it. and proceed with recommendations").** Gate on every declared listener, TCP and UDP; the opt-out `startup = []` is kept; no minimum-healthy window; ADR-0058 is superseded on acceptance of ADR-0270. | ADR-0268, 0269, 0270, 0272 |
| D16 | **Option R approved, 2026-11-03** (remove `inferred`; its historical-read procedure was superseded only for this feature by the 2026-11-09 greenfield compatibility waiver and exact D16-G1 current-format policy below). | ADR-0271 |
| D17 | **Accepted, 2026-11-03** ("add it"; shape choices S1 to S6 accepted as proposed): S1 `Notify::notify_one`; S2 signal after the lock is released; S3 one submit path including the loop's own resync; S4 `BrokerDrain` without `submit`, `broker_mutex` removed; S5 `submit_evaluation` and `BrokerDrain` `pub`, `submitted` `pub(crate)`; S6 no Quint model. | ADR-0273 |
| D13 contract shape | **Accepted, 2026-11-03.** Sockops `TCP_LISTEN_CB` arm as a separate small program in the same object (PR-2); `post_bind4` and `post_bind6` `CgroupSock` programs, both returning 1, both filtering protocol 17; loopback exclusion; one ordered beacon writer; drain `LISTEN` before `EXIT`; the host reader ends the guest-report wait only on `EXIT` or EOF. | ADR-0268, 0272 |
| V41 | **Held** (boot-cost spike, `spike/findings-boot-cost.md`): +0.36 ms at p50 on the pre-`READY` path (D13 item 6). | § Spike table |
| Dual-stack | **2026-11-03.** Overdrive does not support IPv6 yet. The rule "an `AF_INET6` wildcard report satisfies an IPv4-declared listener" and the `V6ONLY` exposure are recorded against GH #308 (comment: https://github.com/overdrive-sh/overdrive/issues/308#issuecomment-6614793072). | D13 item 8 |
| Dropped report | **2026-11-03.** No level map. The ring is sized generously by the crafter; the drop counter stays visible; a lost report is a stated, bounded exposure (the service is stopped at the startup deadline, not hung). Only TCP listen transitions and explicit UDP binds produce events. | D13 item 9 |
| PR-1 | **Accepted, 2026-11-04 ("proceed with recommendations").** The `GuestListenerReports` core read-only port (`reported` / `changed`), the `GuestListenerBoard` worker adapter (`open` / `record` / `close`), the extra mandatory parameters on `ProbeRunner::new` and `VmDriver::new`, and `SimGuestListenerReports`. The `Driver` trait, `Action` and the reconcilers are unchanged. | D13 item 10 |
| PR-2 | **Direction approved 2026-11-04 pending V42; V42 held 2026-11-04 (kernel `7.0.0-38-generic`, `spike/findings-workload-cgroup.md`, PROMOTE), so PR-2 is Accepted in contract shape.** Init creates a workload cgroup and attaches the three listen programs there, so init's own transport sockets are invisible by construction. The TCP listen arm is a separate small sockops program in the same object. Pinned by V42: the programs ride in the object init already loads (never a second `Ebpf::load`), the root programs stay link attachments, the workload is created directly in the cgroup (D13 item 1). | D13 items 1 and 7 |
| PR-3 | **Accepted, 2026-11-04.** `Pass` is written on the event. The first `Fail` is written at the derived startup deadline; then one `Fail` per interval until the stop lands. No `Fail` is written before the deadline. `describe` loses the pre-deadline progress (accepted). The row counter is P1's. | D14 item 3 |
| P1 | **Accepted, 2026-11-04 (option (a); "proceed with recommendations").** The guest-listener task writes honest rows through the shared run state (`Pass` and the first `Fail` have `same_status_runs` 1; later `Fail`s 2, 3, …). `ServiceLifecycle`'s startup stop gate takes its attempts threshold from the Service's startup descriptor: `max_attempts` for `Tcp`/`Http` (unchanged), 1 for `GuestListeners`. `ServiceAllocFact.max_attempts` becomes `startup_attempts_required`. Explicit probes are unchanged. | D14 item 3, ADR-0269 |
| A1 | **Accepted, 2026-11-04, as the architect chose.** The board entry is removed at `close`: an allocation id has an entry from `open` to `close` and none before or after, and `reported` on an id with no entry is the empty set. "Frozen at the end of the guest-report wait" is the host reader's rule (it records nothing once `Exit` or EOF ended the wait), not the board's. Ids are never reused (ADR-0105). | D13 item 10, ADR-0268 |
| A3 | **Accepted, 2026-11-04.** The `guest_report` conformance target is a host-input projection module (same file, its own `step` and named runs); the host reader is driven through the existing `BeaconIo` / `SimBeaconIo` seam. The guest-side actions stay in the non-conformance module. The action-to-driving-port mapping, the State projection and the named runs are in § Formal protocol model — D13 to D16. | § Formal protocol model, ADR-0268 |
| PR-4 | **Accepted, 2026-11-04.** `AllocationSpec.service_ports` is replaced by `declared_listeners`; `AllocationSpec::tcp_service_ports()` is the accessor the TPROXY consumers read. | D14 item 2 |
| `guest_report` model | **Accepted, 2026-11-04.** The `guest_report` Quint module gets host-half conformance (`quint-connect` driver on the host reader, board and probe task) and Tier-3 real-boot coverage for the guest half. There is no writer seam in `overdrive-init`. The gap: the guest-side writer ordering (one ordered writer, `LISTEN` drained before `EXIT`) is not replayed against the model; it is covered by the Tier-3 boot scenarios (V36, V38 shapes) and by the host reader's treatment of any line order. | § Formal protocol model |
| R1 | **Accepted, 2026-11-04 ("proceed with recommendations").** The conformance projection `guest_report_host` gains a `VmmExit` action, mapped to `SimVmm::exit_process` plus the driver's exit-gate release (precedent: the `quint_flows_beacon` driver's `vm_stop`, `crates/overdrive-worker/src/vm_driver/quint_flows_beacon.rs:446-465`). `exitrep` becomes three-valued: `none` (no terminal exit row yet), `guest-reported` (the guest's `EXIT <status>` was read), `no-report` (no `EXIT`, e.g. EOF first); its source is R1-a. | § Formal protocol model — D13 to D16, ADR-0268 |
| R1-a | **Ruled by the user, 2026-11-04, option (a).** The `guest_report_host` conformance projection reads `exitrep` from the exit observer's resulting `AllocStatusRow` through `ObservationStore::alloc_status_row` (re-readable owner observation), not from the `ExitEvent` the consuming `take_exit_receiver` hands out. The row distinguishes the three values (`Terminated` with `Stopped { by: Process }`, or `Failed` with `WorkloadCrashedImmediately { exit_code: Some(_) }`, is guest-reported; `Failed` with `exit_code: None` or `VmOutOfMemory` is no-report; no terminal row is `none`); the `VmmExit` handler composes the production exit observer on the driver's `VmDriver` and `SimObservationStore` and awaits the row. | § Formal protocol model — D13 to D16 |
| R2 | **Accepted, 2026-11-04.** Model trims: the full-module runs `garbage_line_is_dropped`, `all_declared_reported` and `beacon_closes_with_partial_set` are dropped (the host-half behaviour lives in the `guest_report_host` runs; `ring_drop_leaves_set_partial`, `listen_then_exit_drains_first` and `ListenBeforeExit` are kept); `gl = false` is pinned in the exhaustive `ra_ok` instance and its hazard instances (free only in `ra_conf`); `guest_listener_fail_fail_reemits_stop_per_row` is folded into `first_fail_at_deadline_with_no_earlier_row_stops`. | § Formal protocol model — D13 to D16 |
| DG-1 | **Approved, 2026-11-05 ("proceed with recommendations").** `ra_conf` is instantiated at `CEIL = 5`, the value of production `RESTART_BACKOFF_CEILING` (`workload_lifecycle.rs:34`). The give-up runs are rewritten to five restarts. The driver asserts that the spec's `CEIL` equals the const (test-only). `ra_ok` is unchanged. No production change. | § Formal protocol model, "Conformance abstractions" |
| DG-2 | **Approved, 2026-11-05.** `restart_authority` drops its separate `pending` row and its `StartOk` and `StartRejected` actions. The start outcome (`ok` or `rejected`) is a nondeterministic parameter of the start-producing actions (`Deploy`, `WLRestart`), as in the shim, which writes `Running` on success and `Failed` on `DriverError::StartRejected` within the awaited tick (`action_shim/mod.rs:~2433-2459`). A rejected-start named run is kept. The driver scripts the rejection through a runtime-settable `SimDriver` start-failure flag (test support, DISTILL's). | § Formal protocol model, "Conformance abstractions" |
| DG-3 | **Approved, 2026-11-05.** In the `liveness_gate` projection, `committed` is compared only while the row is `Running`. After `Crash` it is model-only. | § Pinned conformance projection |
| DG-4 | **Approved, 2026-11-05.** In `stream_snapshot`, the compared state is the closing `AllocStatusRow` read through the existing `ObservationStore::alloc_status_row`. The line `build_service_stream` yields is an input and an oracle, not a projected variable. | § Pinned conformance projection |
| DG-5 | **Approved, 2026-11-05.** In `stream_snapshot`, the channel is an ordered list; `Deliver` takes the oldest event and is disabled while `lagged`. The `Lag` abstraction is stated. In `progress_deadline`, `StableHappens` is disabled after `OperatorStop`. | § Formal protocol model, "Conformance abstractions" |
| DG-6 | **Approved, 2026-11-05 (pin).** `ReportedListeners` construction is `Default`, `FromIterator<Listener>` and `is_empty`; `GuestListenerBoard::new() -> Self`. D13 names the DELIVER obligation that production composition shares one board. | D13 item 10, § Interface changes (D13 to D16) |
| DG-7 | **Approved, 2026-11-05 (pin).** `progress_deadline: Duration` is the last parameter of `build_service_stream`; the server deadline is the second parameter of `ApiClient::submit_workload_streaming`; `LastAttemptWire` lives in `overdrive_control_plane::streaming`. | § D10 interface changes |
| DG-8 | **Approved, 2026-11-05.** G-10 boundary (9b) and (10) are Tier-3 real-boot DELIVER obligations of D13 and D14. (8) is a seeded composed run whose fault is the omitted `LISTEN` line on `SimBeaconIo`. | Gate G-10, D13 item 9 |
| DG-9 | **Approved, 2026-11-05.** S-32's composed restart and give-up leg is required. S-48 (b) is waived and carried by the mutation gate. S-48 (e) is waived. | § Executable obligations, S-32 and S-48 |
| DG-10 | **Approved, 2026-11-05.** `tokio` with `test-util` as a dev-dependency of `overdrive-cli`, for S-36. The S-36 test sits behind `integration-tests` and carries a litmus proof that it fails when the `deadline + 30 s` envelope is removed. | S-36 |
| DG-11 | **Approved, 2026-11-05 (pin).** The declared listeners reach both mechanic-summary formatters as a parameter. No wire field and no field on `ProbeMechanic::GuestListeners`. | D14 item 4 |
| DG-13 | **Approved, 2026-11-05.** Clause (9a) of G-10 boundary (9), init's own binds produce no `LISTEN` line, carries no test obligation. The failure it would defend, false readiness from Overdrive's own in-guest binds, is unreachable, and a report of init's binds has no production effect. Every TCP and UDP socket init binds or listens on comes from fixed constants inside `guest_reserved_ports()` (61000..=62024); init's other sockets are connect-only flow sockets on ephemeral ports and an AF_VSOCK listener. The Service validator rejects any declared port in that range through the same function (`ListenerPortReserved`), so no declared listener can match init's binds. The board ignores a report for an undeclared listener (D13 item 10; `guest_listener_board.rs::a_pending_changed_completes_on_an_insertion_and_on_no_no_op`, the undeclared-pair case). PR-2 additionally keeps init outside the hooked workload cgroup, so init's binds produce no events; a regression there would add only ignored beacon lines. Clause (9b) is covered by `guest_listener_real_boot.rs::every_declared_listener_a_real_workload_binds_is_reported_before_it_exits`. | Gate G-10 (9) |
| DG-12 | **Approved, 2026-11-05 (changes D6, behaviour).** Every allocation `ServiceLifecycle` observes in `Failed` or `Terminated` joins `ServiceLifecycleView.terminal_announced`, whatever its `started_at`. | D6 item 1, S-56 |
| DG-M | **Approved, 2026-11-05 (pin).** `BeaconParseError::MalformedListen { raw: String, detail: String }` for a rejected `LISTEN` line. `guest_report_host`'s `Idle` maps to a read-only `GuestListenerReports::reported(alloc)` call compared with the previous projected value. The original D16 historical-fixture/version-bump clause was superseded only by the explicit 2026-11-09 greenfield compatibility waiver below; the parse and model pins remain approved. | § Interface changes (D13 to D16), § Formal protocol model — D13 to D16 |
| D11-E1 | **Approved as the exact contract, 2026-11-09** (user: “Approve all three exact contracts”). `build_service_stream` returns a stream whose item is `Result<Bytes, ServiceStreamError>`, with typed `ObservationRead { source: ObservationStoreError }` and `LineEmission { source: std::io::Error }`. The prior D11 “no new public surface” clause is amended only for this in-process Rust error type; HTTP/NDJSON and CLI wire remain unchanged. | D11 item 5, § D11 interface changes; bounded technical review iteration 2 |
| D14-A1 | **Approved as the exact contract, 2026-11-09** (same user ruling). `AllocationSpec::tcp_service_ports(&self) -> Vec<NonZeroU16>` returns an owned TCP filter of `declared_listeners` in declaration order. | D14 item 2, § Interface changes (D13 to D16); bounded technical review iteration 2 |
| D14-E2 | **Approved as the exact contract, 2026-11-09** (same user ruling). `AggregateError::GuestListenersInvalidRole { vector_role, descriptor_role, index }`, `AggregateError::GuestListenersNotSoleStartup { count }`, and `ParseError::GuestListenersNotOperatorConfigurable { role, probe_idx }` with the ingress and HTTP/CLI mappings below. | § D14-E2 typed input rejection, § Interface changes (D13 to D16); bounded technical review iteration 2 |
| D13-S1 | **Approved as the exact contract, 2026-11-09** (user: “rest is fine”, referring to D13-S1 and D17-S1 in the surfaced proposal). `VmDriver::new` takes the board as its ninth/last mandatory argument; `ReportedListeners::{contains,len,is_empty}` have the exact signatures below. | D13 item 10, § Interface changes (D13 to D16), ADR-0268 |
| D17-S1 | **Approved as the exact contract, 2026-11-09** (same ruling). `BrokerDrain<'a>` has a private guard and only the exact three drain/read methods below; `ReconcilerRuntime::broker(&self) -> BrokerDrain<'_>`. | § D17-S1 interface changes, ADR-0273 |
| D16 greenfield waiver | **Explicitly approved, 2026-11-09**, user: “"preserve historical bytes" -> what do um ean? we dont need to preserve anything yet. this is greenfield. not used in production. rest is fine”. This waives historical persisted-byte/backward-read compatibility and the historical nested types, conversion chains and fixture additions needed solely for it **for this feature only**; current behavior and unrelated schema systems or fixture literals remain unchanged. | § D16, `design/full-review-remediation.md` |
| D16-G1 | **Accepted as the exact current-format consequence of the user's greenfield waiver, independently approved 2026-11-09 (full review iteration 4).** Rebaseline only the five affected current envelope payloads under existing version tags; no backward-read promise or new historical machinery. | § D16, `design/full-review-remediation.md`, `design/review.md` |

## Wave: DESIGN / [REF] Decisions

D1 to D17 are accepted following the independent full DESIGN review at
iteration 4, with D16-G1's exact greenfield waiver recorded above. PR-1 to PR-4
and B1 to B3 retain their accepted user rulings (V42 held; § Decisions D13 to D16). The approval
record and as ADR-0265, ADR-0266 and ADR-0258.

### D1 — The exit observer fires `on_alloc_terminal` for every `ExitEvent` it consumes

The exit observer calls `Driver::on_alloc_terminal(&event.alloc)` on the driver it
was spawned for, once per consumed `ExitEvent`, **before** the awaited row write
and **regardless of the write outcome** (`Wrote`, `NoWrite`, `Failed`). The
`Weak<dyn Driver>` upgrade is transient and is not held across an `.await`. A
failed upgrade is a no-op. There is one observer per registry driver and it holds the
exact owning driver (`lib.rs:4512-4526`).

1. The trigger is the **`ExitEvent`**, not the successful row write: on `NoWrite`
   (`exit_observer.rs:469-476`) and `Failed` (`:244-287`) no row is written, yet the
   guest is dead.
2. The hook fires **first**, because it is infallible and in-memory, and the
   observer's retry loop can sleep up to 350 ms (`RETRY_BACKOFFS`, `:81-82`)
   while a dead guest keeps probing.

### D2 — The `on_alloc_terminal` contract is row-independent and idempotent

The trait docstring (`crates/overdrive-core/src/traits/driver.rs:739-751`) is
rewritten as specified in § Interface contract: the hook means "this
allocation's workload has ended or its terminal state is being claimed";
authors are plural; it is infallible, synchronous and non-blocking; repeated and
unknown ids are no-ops; it is not ordered relative to the terminal row write.
`VmDriver` is the only production override (`driver.rs:751` default,
`vm_driver.rs:2061` override).

### D3 — The shim registers probe supervision before it releases the exit-event gate

At both `Running` sites the action shim calls `driver.on_alloc_running(&spec)`
**before** `driver.release_for_exit_emission(handle)`: `StartAllocation`
(`mod.rs:2679` then `:2686`) and `RestartAllocation`'s successor (`mod.rs:3201`
then `:3202`).

`release_for_exit_emission` is what lets the watcher emit `ExitEvent`
(`vm_driver.rs:2295-2329`; the gate send is the method's last effect, `:2019-2024`).
In the reverse order D1's hook can run as a no-op on an unregistered id before
`on_alloc_running` registers the supervisor, orphaning it for the life of the
process. Record: [ADR-0256](../../product/architecture/adr-0256-the-shim-registers-probe-supervision-before-it-releases-the-exit-event-gate.md).

### D4 — The two existing shim hook calls stay; `cleanup_restart_predecessor` gains none

- `StopAllocation` keeps its hook (`mod.rs:3272`). `VmDriver::stop` moves the
  entry to `EndingInFlight`, so `try_begin_ending` returns `None` and **no
  `ExitEvent` is emitted** (`vm_driver.rs:1793`, `:2304-2308`). (**B2, accepted
  2026-11-03, ADR-0266**, amends D4 for this one statement: the hook is fired after
  the stop's terminal row write succeeds instead of before it.)
- `FinalizeFailed` keeps its hook (`mod.rs:2176`); it runs against a VM that may
  still be alive, so no `ExitEvent` exists to fire it.
- `cleanup_restart_predecessor` gets **no** call. `RestartAllocation` requires the
  predecessor row to be `Failed | Terminated` (`mod.rs:2708`), and every way a
  supervised predecessor reaches that row now fires the hook (§ Q1).

### D5 — `ServiceLifecycle` announces a startup-probe failure only for a non-terminal allocation

Record: [ADR-0257](../../product/architecture/adr-0257-service-lifecycle-announces-a-startup-failure-only-for-a-live-allocation.md).

**The edge.** `startup_probe_failed_action`
(`crates/overdrive-reconcilers/src/service_lifecycle.rs:1168-1217`) returns
`None` for `AllocState::Terminated` only (`:1192-1194`); otherwise it emits
`FinalizeFailed { ServiceFailed { StartupProbeFailed } }` when
`attempts >= max_attempts && elapsed >= deadline && no_pass` (`:1199-1213`),
`Failed` included. D1 freezes the streak at the crash, so a crashed allocation
whose streak was at the maximum is finalised as a startup-probe failure while
`WorkloadLifecycle` still treats the row as restartable (`is_restartable` ignores
`terminal`, `workload_lifecycle.rs:1656-1660`). D6 deletes the early-exit branch, so
the guard below is then the only thing that stops the verdict for a `Failed` row.

**Contract.** The `Terminated` exclusion (`:1192`) becomes the existing predicate
`AllocState::is_terminal` (`crates/overdrive-core/src/traits/observation_store.rs:230`,
`Terminated | Failed`), so the function returns `None` for a terminal row before
reading any gate. No signature, type, variant, error, wire or persisted shape
changes. The comment at `:1174-1191` is rewritten accordingly. `reconcile` is
unchanged: a `Failed` row yields `None` and no action. Its membership of
`terminal_announced` is D6 item 1's rule (DG-12), not this function's.

**Other branches.** No other `ServiceLifecycle` branch acts on a terminal row:
Stable (`:516`, `:538`), liveness terminate (`:1011`) and the backend row
(`:1055`, `:1069`) are gated on `Running`; the early-exit branch (`:569-601`) is
deleted by D6.

**Single restart authority.** `.claude/rules/reconcilers.md` § "Single restart
authority" assigns restart, and give-up at exhaustion, to `WorkloadLifecycle`
(`workload_lifecycle.rs:1106-1108`, `:1132`, `:1177-1191`).

**Dedup and membership do not depend on the branch firing on `Failed`.**
`terminal_announced` (written at `:619`) is read by the dedup (`:488`), the
successor hold (`compute_backend_healthy`, `:1137`) and the deadline hook (`:698`);
membership considers only `Running` allocations (`:1055`, `:1069`). One behaviour
differs: a restart successor is no longer held out of backend eligibility by the
predecessor's announcement (`:1137-1144`). **D12 (ADR-0267, accepted 2026-11-03)**
deletes that hold and replaces it with a per-allocation rule (a startup-probed
allocation is a backend only after its own startup has passed).

**User-visible consequence.** A service allocation that crashes after its
startup-probe attempts were exhausted and the deadline passed is restarted under
`WorkloadLifecycle`'s budget; at exhaustion the operator sees the crash-loop
terminal (`EarlyExit` or `BackoffExhausted`, per D6) instead of `StartupProbeFailed`.
A guest that stays `Running` with exhausted probes past the deadline is stopped (D7).

DISTILL rewrites `crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs:263-274`,
which expects `StartupProbeFailed` from a `Failed` fact (S-21).

### D6 — `WorkloadLifecycle` alone authors a crashed Service allocation's ending and names a pre-startup crash loop at give-up

Record: [ADR-0258](../../product/architecture/adr-0258-the-restart-authority-names-a-pre-startup-crash-loop-at-give-up.md),
including the K1 restatement (§ Changed and unchanged assumptions, option (a)).

**The defect at HEAD.** `ServiceLifecycle::reconcile`'s early-exit branch
(`service_lifecycle.rs:560-601`) fires on a `Failed` fact with `started_at` set, a
non-`Pass` startup probe and elapsed time below the deadline, and emits
`FinalizeFailed { ServiceFailed { EarlyExit { exit_code } } }` (`:589-594`). The
shim treats it as a genuine terminal (`action_shim/mod.rs:2156-2194`) while
`WorkloadLifecycle` still restarts the row until the budget (`:1106-1252`): two
reconcilers author one allocation's ending, against ADR-0087 D1/D2. The K1 deploy
stream closes on the first terminal event (`streaming.rs:1025-1036`).

**Contract.**

1. **`ServiceLifecycle` loses the early-exit branch.** For a row in state `Failed`
   or `Terminated` it emits no `Action`. Deleted: the branch (`:560-601`) and
   `ServiceAllocFact.exit_code` (`:111`, hydrated `:891-894`, `:899`), whose only
   reader is the branch (`:592`). **Membership (DG-12, 2026-11-05).** Every
   allocation `ServiceLifecycle` observes in state `Failed` or `Terminated` joins
   `ServiceLifecycleView.terminal_announced`, whatever its `started_at`, at the
   head of the per-allocation step, so no later branch decides it. The insertion
   is no longer confined to a `Failed` row that never reached `Running`
   (`service_lifecycle.rs:581-594`), and the early-exit insertion (`:610`) is
   deleted with the branch. The startup-stop insertion (`:631`, a live allocation)
   remains. Reason: `observed` is never pruned (`:514`) and nothing else inserts a
   started, crashed allocation, so without this rule it stays in
   `has_alloc_mid_startup_window` (`:357-361`) for good. That predicate gates
   `next_evaluation_at` (`:701`), and the runtime would keep re-enqueueing a dead
   allocation (the busy loop the comment at `:583-591` warns of). Membership
   triggers no action. `terminal_announced` for a `Failed` or `Terminated` fact
   records "this reconciler has finished watching the allocation", the same
   meaning as for the startup stop (D7).
2. **`ServiceFailureReason::EarlyExit { exit_code: Option<i32> }` is kept
   unchanged in shape** (no rkyv, serde, `ToSchema` or API-spec change). Its
   author is now `WorkloadLifecycle`; the doc comment at
   `transition_reason.rs:694-697` is rewritten to say: the Service's final
   restart attempt crashed before any declared startup probe passed and the
   restart budget was then spent; no timing claim against `startup_deadline`.
3. **`WorkloadLifecycle` selects the give-up terminal from the observed row.** In
   the ceiling branch (`workload_lifecycle.rs:1132-1192`) the terminal selection
   at `:1177-1186` gains a case, evaluated after the liveness case and after D7's
   startup-kill case (D7 § item 3 fixes the order):
   `TerminalCondition::ServiceFailed { reason: ServiceFailureReason::EarlyExit {
   exit_code } }` when the failed row is a **pre-startup crash**, else the
   existing `BackoffExhausted { attempts }`. A pre-startup crash is a row for
   which all of the following hold:
   - the workload kind is `Service`;
   - `desired.probe_descriptors` contains at least one descriptor of role
     `Startup` (an explicit opt-out declares none, ADR-0058);
   - the row's state is `Failed` and its typed reason is
     `WorkloadCrashedImmediately`. **B3 (accepted 2026-11-03)** widens this to the
     two guest-crash reasons the exit observer writes, `WorkloadCrashedImmediately`
     and `VmOutOfMemory` (`exit_observer.rs:597-619 @ 04052ad75`); the fail-closed
     supersessions (`MtlsInterceptInstallFailed`, `WorkloadNetnsProvisionFailed`,
     `DriverInternalError`) stay out (ADR-0083 D6's hazard). `exit_code` is the
     crash's exit code, and `None` for `VmOutOfMemory` (an existing shape that
     S-19 renders);
   - the current attempt's latest startup probe result is not `Pass`
     (`actual.current_startup_probe`, below).
   The idempotency guard at `:1143-1151` matches `ServiceFailed { EarlyExit }` as
   well, so a row already carrying the give-up is not re-finalised. The
   classification is evaluated only in this branch: it never changes whether a
   restart is emitted, the backoff, the ceiling, the budget count, or the
   platform-reclamation and generation-replacement exemptions. Last cause wins at
   the ceiling (ADR-0087 D4): only the final attempt's row is classified. A
   platform-reclaimed row never reaches the ceiling branch (§ Pinned readings, R-1).
4. **One new hydrated input on `WorkloadLifecycleState`, shared with D7:**
   `current_startup_probe: Option<ProbeStatus>` (the type `ServiceAllocFact`'s
   `latest_startup_probe` uses, `service_lifecycle.rs:114`). Pinned meaning: the
   status of the latest result row for role `Startup`, probe index 0, of the
   workload's **current attempt only** (`current_alloc`, `workload_lifecycle.rs:1106-1108`,
   `:1422-1428`), read only when that row's state is `Failed` or `Terminated`;
   `None` when there is no such row or no result row (ADR-0054 § 5). D6 reads
   whether it is `Some(Pass)`; D7 reads the failure reason of `Some(Fail { .. })`.
   Populated only by `hydrate_actual`, through `HydrationContext`'s observation
   store (`list_probe_results_for_alloc`); `hydrate_desired` leaves it `None`. A
   read failure surfaces as `HydrateError::ObservationRead` (logged and skipped for
   the tick, ADR-0086); there is no default and no "failed read means no pass"
   rule. No port, trait method, adapter or `Sim*` surface changes; nothing is
   persisted.
5. **Restart budget on the API wire.** `restart_budget_from_rows`
   (`handlers.rs:1399-1407`) reports the budget as exhausted for a row whose
   terminal is `ServiceFailed { EarlyExit }` as well as `BackoffExhausted`, with
   `used` equal to `RESTART_BACKOFF_CEILING`. The same gap exists today for
   `LivenessProbeFailed` (F-2, GH #336); this decision does not change it.
6. **Stream and CLI.** `service_event_from_terminal` already maps
   `ServiceFailed { reason }` to `ServiceSubmitEvent::Failed { reason }`
   (`streaming.rs:840-844`); no change. The CLI's `elapsed:` line for `EarlyExit`
   is removed: `format_service_failed_block` loses its `early_exit_timing`
   parameter (`render.rs:1461-1476`, `:1540-1545`) and the stream-side elapsed
   measurement is deleted (`deploy.rs:689-770`). The exit-code line, the Service
   guidance sentence and the stderr tail stay.

**Behaviour change (approved).** After the first crash the `overdrive deploy`
Service stream no longer ends with `Failed { EarlyExit }`; it stays open through
the restarts (a successor that passes startup ends it with `Stable`) and ends at
give-up with `Failed { EarlyExit { exit_code } }` for a pre-startup crash loop, or
`BackoffExhausted` for any other cause. `workload describe` shows one row per
attempt (ADR-0105) and, at give-up, `terminal: ServiceFailed { EarlyExit }` with the
wire `restart_budget` exhausted. `Restarts` reads 0 on every row (F-1, GH #335).

**ADR-0083 D6 hazard.** The `Failed`-gated early-exit branch fabricating
`ServiceFailed { EarlyExit }` on a non-crash row is removed with the branch; the
new classification requires the typed crash reason and runs only at the ceiling,
after the platform-reclamation exemption (`workload_lifecycle.rs:1130-1132`).
ADR-0083 D6 itself is unchanged.

**Pre-existing gap surfaced, not changed (D6.b, optional).** The slice-08 scenario
says a Service that exits 0 inside the deadline is `EarlyExit { exit_code: 0 }`; in
production a clean exit lands as `Terminated` with `Stopped { by: Process }`
(`classify_natural_exit_terminal`, `workload_lifecycle.rs:1692-1695`), so it was never
produced. Under D6 a pre-startup clean exit gives up as `BackoffExhausted`.
Extending the predicate to that `Terminated` row with `exit_code: Some(0)` would
deliver the scenario; it is the user's to add.

Alternatives: ADR-0258. Giving up at the first pre-startup crash is a
restart-policy change and is not taken without the user.

**Tests that pin the old behaviour.** DISTILL rewrites them: the `early_exit_*`
cases in `crates/overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs`
(S-21, S-15); `service_lifecycle_stable.rs` `:229`, `:260`, `:282` (S-16; `:282`
deleted or the D6.b scenario); the `exit_code` assertions in
`service_lifecycle_hydrate.rs`; `service_early_exit_render.rs` and
`render_pure_fns.rs` (no `elapsed:`, new signature); and
`verification/expectations/E01-coinflip-service-honest-early-exit/` (the "within the
startup deadline" wording is restated with K1). `EarlyExit` wire and rkyv roundtrip
tests are unchanged.

**Not touched.** `RESTART_BACKOFF_CEILING`, the backoff, restart decision functions,
`is_restartable` and the reclamation/replacement exemptions; the exit observer,
shim, probe runner, ADR-0078/0105 and every persisted or wire format.

### D7 — a startup-probe failure stops the allocation; `WorkloadLifecycle` alone restarts it and names the failure at give-up

Decision record: [ADR-0259](../../product/architecture/adr-0259-a-startup-probe-failure-stops-the-allocation-and-the-restart-authority-decides.md)
(context, evidence from Kubernetes `kuberuntime_manager.go:1440-1470` and Nomad
`watcher.go:105-117`, restated K1 in § Changed and unchanged assumptions). Item 5 is
extended by D9; D8 replaces the stamp-at-emission dedup of item 2; D10 replaces the
90 s stream cap.

**The defect at HEAD.** `ServiceLifecycle` emits `FinalizeFailed { ServiceFailed {
StartupProbeFailed } }` on a live allocation (`startup_probe_failed_action`,
`crates/overdrive-reconcilers/src/service_lifecycle.rs:1168-1217`, called at
`:612-620`); the shim writes the row `Failed` without `driver.stop`
(`action_shim/mod.rs:2163-2194`), `WorkloadLifecycle` restarts it to the ceiling
(`workload_lifecycle.rs:1543-1558`), and the stream closes on the first terminal
(`streaming.rs:1029-1056`): two reconcilers author one ending.

**The shape D7 mirrors.** The liveness stop emits `StopAllocation { terminal:
Some(Stopped { by: LivenessProbe }) }` (`service_lifecycle.rs:1021-1024`); the shim
stops the driver, fires `on_alloc_terminal` and writes the row `Terminated`
(`action_shim/mod.rs:3234-3388`); the row is restartable by construction
(`transition_reason.rs:391-411`) and `WorkloadLifecycle` reads the cause at the
ceiling through `is_liveness_killed` (`:1575-1584`), never through a
cross-reconciler read (ADR-0087 D3). The hazard: the shim echoes the stop's terminal
onto the lifecycle event (`action_shim/mod.rs:508`), the Service stream returns on
every `Stopped` (`streaming.rs:845-847`, `:1047-1051`), and the CLI exits 0
(`deploy.rs:795-817`). A startup stop happens while the stream is open, so item 5
pins the handling.

**Proposed contract.**

1. **New public variant `StoppedBy::StartupProbe`** (`overdrive-core`,
   `transition_reason.rs`, `#[non_exhaustive]` enum). Fieldless, appended after
   `LivenessProbe` (rkyv discriminant 6; serde and OpenAPI name `startup_probe`).
   Doc comment: a startup-probe failure (detected by `ServiceLifecycle`) stopped
   this instance; the workload's intent still stands and `WorkloadLifecycle`
   restarts it under its single budget; restartable, not an intentional stop, not
   a platform reclaim. Reusing `LivenessProbe` would make the two causes
   indistinguishable at the ceiling (ADR-0259). No helper method is added. The two
   exhaustive `StoppedBy` match sites without a wildcard gain an arm:
   `TransitionReason::human_readable` (`transition_reason.rs:900-907`, text
   `stopped (startup probe)`) and `by_reclaims_platform` (`:1027-1038`, `false`).
2. **`ServiceLifecycle` stops instead of finalising.**
   `startup_probe_failed_action` keeps its guard (ADR-0257: `None` for a terminal
   row) and its three gates unchanged. When it fires it returns
   `Action::StopAllocation { alloc_id, terminal: Some(TerminalCondition::Stopped {
   by: StoppedBy::StartupProbe }) }` instead of the `FinalizeFailed`. No new
   `Action` variant. The bookkeeping is unchanged: the allocation is inserted into
   `terminal_announced` at emission (the dedup, the liveness skip and the runtime
   re-enqueue falseness read it). Nothing precedes the stop in the returned
   `Vec<Action>`: the per-allocation backend veto, `startup_failed_this_tick` and the
   backend-row splice ahead of the startup action (`:634-647`) are deleted, because
   an allocation without a committed `Stable` is already not `healthy` (D12). **D8
   changes one thing here: the dedup.** Membership of `terminal_announced` no longer
   suppresses a startup stop for an allocation that is still `Running`; the stop is
   re-emitted once per new written startup probe row until the row leaves `Running`
   (D8). The doc comments of `terminal_announced` and the branch are rewritten: the
   set records "this reconciler has acted on the allocation's startup window". The
   field name is a serde key of the CBOR `View` and is not renamed. D7 itself adds
   and removes no `View` field; D8 adds one. The `no_pass` gate reads the latest
   startup row, in D5, D7 and D8 alike (§ Pinned readings, R-2).
3. **`WorkloadLifecycle` restarts the stopped row and names the failure at the
   ceiling.**
   - `is_restartable` and `is_intentionally_stopped` are unchanged: a `Terminated`
     row carrying `Stopped { by: StartupProbe }` is restartable and **must not** be
     exempted by the intentional-stop test (the exemption stays exactly operator
     and system gc). It consumes restart budget as a crash does.
   - A private predicate on the observed row, true iff the row is `Terminated` and
     its `terminal` is `Stopped { by: StartupProbe }` (the crafter names it). It
     reads `terminal` only: the shim hardcodes the row's `reason` to `Reconciler`
     for every `StopAllocation`.
   - **Give-up priority order in the ceiling branch** (`:1177-1186`), first match
     wins: (a) liveness kill, `ServiceFailed { LivenessProbeFailed }` (unchanged);
     (b) startup kill, `ServiceFailed { StartupProbeFailed { probe_idx: 0,
     last_fail, attempts } }`; (c) pre-startup crash, `ServiceFailed { EarlyExit }`
     (D6); (d) otherwise `BackoffExhausted` (unchanged). The three specific cases
     are mutually exclusive by construction (a, b read a `Terminated` row's own stop
     disposition; c requires a `Failed` row with `WorkloadCrashedImmediately`), so
     the order never changes an outcome; a property test asserts exclusivity. Only
     the final attempt's row is classified; the last cause wins (ADR-0087 D4).
   - `attempts` in the give-up terminal is `restart_counts` for the final row (the
     restart budget consumed, equal to the ceiling), as for `LivenessProbeFailed`
     and `BackoffExhausted` (ADR-0087 D4), **not** the probe streak the detector
     read. The doc comment on `ServiceFailureReason::StartupProbeFailed`
     (`transition_reason.rs:689-693`) is rewritten to say so; this changes the
     meaning of an existing field's value, not its type or archived layout.
   - `last_fail` is the final attempt's last startup-probe failure reason, read from
     its startup probe row.
   - The idempotency guard (`:1143-1151`) also matches
     `ServiceFailed { StartupProbeFailed }`. The ceiling-branch `FinalizeFailed` is
     emitted only for a row that does not already carry one of the give-up
     terminals.
   - **Hydrated input: the same `current_startup_probe` field D6 item 4 pins**, no
     second field. `last_fail` is the failure reason of
     `current_startup_probe == Some(ProbeStatus::Fail { last_fail_reason, .. })` and
     the empty string for any other value, the same total mapping
     `startup_probe_failed_action` applies (`service_lifecycle.rs:1200-1203`). The
     mapping is total because a `Fail` row can be followed by a late `Pass` (the
     hook allows one bounded late probe row, D2, Q3; probe rows are never deleted,
     V9). A read failure is `HydrateError::ObservationRead` (D6 item 4).
4. **The shim is unchanged** (apart from B2, ADR-0266: the hook now follows the row
   write). The `StopAllocation` arm already stops the guest, cleans up, fires the
   terminal hook (D4) and writes the `Terminated` row, so the guest is stopped
   before its successor is placed.
5. **The Service stream does not end on a restartable probe stop.**
   `service_event_from_terminal` (`streaming.rs:825`, a `pub fn`) returns `None`
   for `TerminalCondition::Stopped { by: StoppedBy::StartupProbe }` **and for
   `TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }`** (see D9 item 6).
   The existing `None` path continues the loop (`streaming.rs:1039-1046`). Every
   other `Stopped { by }` projects as today. The stream closes on a give-up
   terminal, on `Stable` for a successor that passes, on an operator stop, or on
   its progress deadline (D10).
6. **`restart_budget_from_rows`** (`handlers.rs:1399-1407`) treats
   `ServiceFailed { StartupProbeFailed }` as exhausted, with `used` equal to
   `RESTART_BACKOFF_CEILING`, consistent with D6 for `EarlyExit`. The same gap for
   `LivenessProbeFailed` stays GH #336.
7. **`workload describe`.** `render::workload_describe`'s Service arm
   (`overdrive-cli/src/render.rs:1135-1158`) prints one presence-guarded line,
   `    terminal: stopped (startup probe failed)`, for a row whose terminal is
   `Stopped { by: StartupProbe }`, beside the existing `terminal: Stable` line.

**Behaviour change.** For a Service with a declared startup probe whose instance
stays alive and never passes, the deploy stream no longer ends with
`Failed { StartupProbeFailed }` at the startup deadline `D`; the instance is stopped
and restarted, a successor that passes ends the stream with `Stable`, and the
give-up (`StartupProbeFailed` with `attempts` equal to the ceiling) arrives after
about `6 x D + 5 s`. With the default `D = 60 s` (`DEFAULT_STARTUP_DEADLINE`,
`service_lifecycle.rs:359`) the progress deadline (D10) is reached first, so the
deploy ends with a `Timeout` carrying the first attempt's startup stop as detail.
A crashing instance is unaffected (D6).

**Interaction with D5.** D5's guard stays and is still required; D7 changes only what
the live branch emits. After the stop lands the row is `Terminated`, so a
re-evaluation yields `None`. D5's sentence "a guest that stays `Running` with
exhausted probes is finalised as `StartupProbeFailed`" is replaced by the behaviour
above.

**Exposures (E-1 is closed by D8).**

- **E-2: a crash racing the stop.** If the guest crashes after `ServiceLifecycle`
  evaluated the `Running` row and before the shim writes the stop row, the shim's
  re-read-then-write loop (`action_shim/mod.rs:3286-3355`, no refusal to overwrite a
  terminal winner at `:3300`) overwrites the `Failed` crash row with `Terminated` /
  `Stopped { StartupProbe }`, so that attempt's crash cause is lost; restart, budget
  and termination are unchanged. It is a documented bounded exposure outside
  `PreStartupCrashStaysObservable`, not modelled (the liveness stop carries it,
  ADR-0087); S-28 pins the outcome. Closing it would change the shim's
  `StopAllocation` contract: P-9.
- **E-3: the progress deadline fires before the give-up** (above). Bound: the
  timeout carries the last attempt's cause as labelled detail (D10); the outcome is
  an honest failure, never a success.
- **E-4: the progress deadline fires before a slow-booting healthy instance reaches
  `Stable`** (D10, assumption DL-6). Bound: the timeout is honest and retracts nothing;
  only a boot near `VM_BOOT_DEADLINE` combined with a last-attempt startup pass
  reaches it.

**Persisted and wire format impact.** `StoppedBy` is rkyv-archived inside
`AllocStatusRow` and `AllocLifecycleOccurrenceRow`, serde-serialised on the API and
stream, and `ToSchema` in `api/openapi.yaml`. The new variant is **fieldless and
tail-appended**, like `LivenessProbe` (ADR-0087 § Compliance), so existing rows
decode unchanged: **no new envelope version**, `AllocStatusRowEnvelope` stays V1,
**no existing `FIXTURE_Vn` is touched**. Obligations: **add one golden-bytes
fixture** pinning `StartupProbe` to
`crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs`, plus the missing
`LivenessProbe` fixture (ADR-0087 § Compliance; F-4; S-25); regenerate
`api/openapi.yaml` (`cargo openapi-gen`; `openapi-check` gates). No compatibility
mode. **No** other archived, wire or `ToSchema` change: `ServiceFailureReason` keeps
its shape, `WorkloadLifecycleView` changes nothing, `ServiceLifecycleView` gains one
field in D8, and `current_startup_probe` (D6 item 4) is an in-memory `State` field.

**Alternatives:** ADR-0259.

**Tests that pin the old behaviour.** DISTILL rewrites the `StartupProbeFailed`
finalisation, backend-veto and ordering tests in `service_lifecycle_reconcile_branches.rs`
(Category C), `service_lifecycle_stable.rs`, `service_kind_vm_workloads.rs`,
`service_backend_projection.rs` and `service_kind_vm_terminal_invariant.rs` to the
`StopAllocation { Stopped { StartupProbe } }` contract (S-21, S-22, S-46).
`single_restart_authority_liveness_trajectory.rs` is the model for S-22.

**Not touched.** `RESTART_BACKOFF_CEILING`, the backoff, `restart_decision`, the
restartable and intentional-stop predicates, the shim, the exit observer, D5's guard,
D6's contract and `ServiceFailureReason`'s shape.

### D8 — a startup stop that does not land is retried once per new written startup probe row

Decision record: [ADR-0260](../../product/architecture/adr-0260-a-failed-startup-stop-is-retried-once-per-new-startup-probe-row.md).
**Own ADR**, because it is independently reversible from D7: D7 is complete
without it (the E-1 exposure then stands), and D8 is reverted by restoring the
stamp-at-emission dedup and deleting one `View` field.

**The defect.** At emission D7's stop stamps the allocation into
`terminal_announced`, and the dedup (`service_lifecycle.rs:488-490`) skips it on
every later tick; if the shim's `StopAllocation` errors, the row stays `Running` and
no reconciler acts again. The liveness path re-acts on a newer crossed row keyed by
`run_seq` (`service_lifecycle.rs:996-1025`, view field `:338`), and the startup probe
task writes a row each interval until `Stable` or the terminal hook
(`probe_runner/mod.rs:763-799`).

**Contract.**

1. **New field `ServiceLifecycleView.startup_last_acted_run:
   BTreeMap<(AllocationId, ProbeIdx), ProbeRunSeq>`**, `#[serde(default)]`,
   documented as `liveness_last_acted_run` is: the `run_seq` of the startup probe
   row on which this reconciler last emitted a startup stop; an input, not a derived
   value; written only when a stop is emitted; absent means never acted. The key is
   `(alloc, ProbeIdx::new(0))`. CBOR evolution is additive with `#[serde(default)]`
   (ADR-0035 / ADR-0036); no versioned envelope, no migration.
2. **Emission rule.** The startup stop is emitted for an allocation iff the three
   gates and the ADR-0257 guard hold (D7 item 2) **and** the latest startup row's
   `run_seq` is greater than the recorded value for the key (or none is
   recorded). Emission records the row's `run_seq` and, as in D7, inserts the
   allocation into `terminal_announced`.
3. **The dedup no longer hides a live allocation.** `terminal_announced`
   membership of an allocation whose fact is `Running` means "a startup stop was
   emitted and has not landed"; for such an allocation the loop still evaluates
   the startup stop rule of item 2. For any non-`Running` fact the existing dedup
   applies unchanged. The liveness skip and the re-enqueue falseness keep reading
   `terminal_announced` (the lineage-wide successor hold is deleted by D12; the
   per-allocation backend veto no longer exists), with one exception: **a startup
   `Pass` recorded after a stop that did not land contradicts the decision the stop
   acted on.** The gate's `no_pass` is then false, so no further stop is emitted; a
   `Running` fact whose latest startup row is `Pass` is evaluated by the Stable
   branch despite its `terminal_announced` membership, and announcing `Stable` for
   it **removes it from `terminal_announced`** (the two sets stay disjoint, and
   liveness, which skips members, acts on it once `Stable`). Without this a late
   `Pass` would strand a serving instance, never `Stable`, never stopped, never
   liveness-supervised. A partial landing cannot produce a `Pass`: a stop whose
   `driver.stop` succeeded leaves a dead guest (Q3).
4. **Retry pace.** One re-emission per new written startup row, so at most one per
   `interval_seconds`, until the row leaves `Running` or the supervisor ends (the
   terminal hook, D1/D4). The shim's `StopAllocation` arm is idempotent for an
   allocation already stopped (`DriverError::NotFound` tolerated,
   `action_shim/mod.rs:3286-3355`), the property the liveness retry relies on
   (ADR-0191).
5. **Bound.** A stop that fails persistently is retried at probe-interval pace
   without limit while the allocation is `Running`; each failure is an
   Overdrive-own driver or cleanup error, and the retry ends when the cause clears,
   as for liveness.

**Owner.** `ServiceLifecycle` (detector); the shim executes. No new `Action`, port,
wire or archived shape; the only persisted change is the additive CBOR `View` field.

**Alternatives:** ADR-0260.

**Bound of the retry (fix: B2, ADR-0266).** The retry trigger is a new startup probe
row, which exists only while the probe supervisor is alive. It covers a stop that
fails *before* the shim fires `on_alloc_terminal` (`action_shim/mod.rs:3250-3271`). It
does **not** cover a failed terminal row write (`:3356`, `:3361-3366`), which comes
after the hook (`:3272`): the guest is dead, the supervisor is gone, `VmDriver::stop`
emits no `ExitEvent` (V1), the row stays `Running`, and nothing acts on it; the
liveness stop carries the same gap since ADR-0087. B2 moves the hook after the
successful row write, so the supervisor stays registered and the next probe row
re-emits the stop.

### D9 — liveness supervision begins at `Stable`; the probe runner owns the gate

Decision record: [ADR-0261](../../product/architecture/adr-0261-liveness-probing-begins-when-the-service-is-stable.md).
**Own ADR**: independently reversible (restoring the `on_alloc_running` spawn
reverts it without touching D7 or D8).

**The defect.** `ProbeRunner::start_alloc` spawns every role's tasks at
`on_alloc_running` (`probe_runner/mod.rs:328-361`); only `Stable` retires the startup
role (`stop_role`, `:393-398`, via `VmDriver::on_alloc_stable`, `vm_driver.rs:2065-2067`).
`liveness_terminate_action` gates on `Running`, a crossed streak and a newer row only
(`service_lifecycle.rs:1002-1013`), so a liveness kill can fire on an instance still
starting and, at the ceiling, name `LivenessProbeFailed` for a workload that never
became ready, although `ProbeRole` documents liveness as "continuous post-Stable"
(`observation/probe_result_row.rs:283-286`).

**Where the gate lives.** The streak is producer-recorded (`same_status_runs`,
`probe_runner/mod.rs:637-657`), so a `ServiceLifecycle` gate would leave a
startup-time streak in place and kill a healthy instance on the first post-`Stable`
tick. The gate sits where the streak starts: the runner starts no liveness task
before `Stable`, following `Driver::on_alloc_stable`, which the shim fires after the
`Stable` row commits (`action_shim/mod.rs:2144-2153`). `Stable` stays
`ServiceLifecycle`'s decision (`service_lifecycle.rs:516-558`).

**Contract.**

1. **`on_alloc_running` no longer starts liveness tasks.**
   `ProbeRunner::start_alloc` spawns the allocation's startup and readiness tasks
   as today and **retains** its liveness descriptors on its supervisor without
   spawning them. An allocation with no liveness descriptor is unchanged.
2. **New `ProbeRunner::start_liveness(&self, alloc_id: &AllocationId)`**
   (`overdrive-worker`, `pub`, beside `stop_role` and `is_role_live`): spawns the
   retained liveness tasks of a supervised, not-yet-started-liveness allocation.
   Postconditions: idempotent (a second call spawns nothing); a no-op for an
   unknown, ended or already-stopped allocation; the spawn and `stop_alloc` are
   serialised by the supervisor lock, so either the liveness tasks start and a
   later `stop_alloc` cancels them, or the allocation is already gone and
   nothing starts. Tick-then-sleep is unchanged: the first liveness row lands one
   interval after `Stable`, and its streak starts at 1 there.
3. **`VmDriver::on_alloc_stable` calls `start_liveness` after `stop_role(Startup)`.**
   `Driver::on_alloc_stable`'s docstring (`traits/driver.rs:753-770`) gains one
   sentence: the production driver also starts the allocation's liveness
   supervision. The trait signature does not change. An opt-out Service (no
   startup probe) reaches `Stable` at its first `Running` (ADR-0058), so its
   liveness starts then.
4. **Readiness is not gated.** Out of this decision.
5. **The `stable_this_tick` skip in `collect_liveness_actions`**
   (`service_lifecycle.rs:980-983`) becomes unreachable (no liveness row can
   exist at the `Stable` tick); the crafter removes it and the reviewer's
   subtractive pass checks it.
6. **A liveness stop and the open deploy stream.** After the gate a liveness stop
   cannot reach the stream of the deploy that is waiting for `Stable`. It **can
   still reach an open stream** through the interval between a stop's row write and
   the publication of its lifecycle event (`action_shim/mod.rs:2187-2193`): a stream
   opened in that interval, for example by `overdrive deploy` of an unchanged spec
   (`handlers.rs:518-546`), would receive the `Stopped` event and end with exit 0
   (`streaming.rs:845-847`, `deploy.rs:795-817`); a lagged subscriber
   (`streaming.rs:1058-1064`) is the second path. So D7 item 5 returns `None` for
   `Stopped { by: LivenessProbe }` as well. The liveness give-up
   `ServiceFailed { LivenessProbeFailed }` still ends the stream. (An `Unchanged`
   redeploy of an already-`Stable` Service no longer waits: D11 ends it at once with
   the recorded `Stable`.)

**Owner.** The probe runner owns when probing of each role starts; `ServiceLifecycle`
remains the liveness detector and `WorkloadLifecycle` the restart authority
(ADR-0087). The runner does not decide `Stable`; it follows the existing hook.

**Alternatives:** ADR-0261.

**Behaviour change.** A Service is no longer killed for failing liveness while still
starting: a slow-starting Service with an aggressive liveness probe that previously
restart-looped to `LivenessProbeFailed` now starts. After `Stable`, liveness behaves
as before. No persisted or wire change; one new `pub` method on `ProbeRunner`.

**Dependency on the committed `Stable` row (fix: B1, ADR-0265).** D9 starts liveness
from the `Stable` hook, which fires after the `Stable` row commits. At HEAD
`ServiceLifecycle` records `stable_announced` in the same pure `reconcile` that emits
the action (`service_lifecycle.rs:531`, `:555`), the runtime persists the `View`
before dispatch and does not roll it back (`reconciler_runtime.rs:1648-1652`), and
the shim's `Stable` arm returns on a write error before the hook
(`action_shim/mod.rs:2147-2150`). A failed write leaves the `View` "announced" and
the row without `Stable`: the startup role is never retired and under D9 **liveness
never starts**. B1 keys the announcement on the committed row, as ADR-0079 does for
the backend row.

**Control-plane restart.** Probe supervisors and the liveness gate are process-local.
Boot reclamation makes any unsupervised VM terminal (`action_shim/mod.rs:2161-2162`;
`action_shim/reclamation.rs:233-244`) and `WorkloadLifecycle` places a successor whose
own `Stable` starts its own liveness; `start_liveness` for an unregistered id is a
no-op (item 2). S-31 carries the replay case.

### D10 — `overdrive deploy` waits on a progress deadline distinct from the restart budget

Decision records: [ADR-0262](../../product/architecture/adr-0262-the-service-deploy-stream-is-bounded-by-a-progress-deadline-derived-from-the-startup-deadline.md)
(the derived deadline, its owner and the client envelope) and
[ADR-0264](../../product/architecture/adr-0264-the-progress-deadline-timeout-carries-the-latest-attempt-as-labelled-detail.md)
(the `last_attempt` detail, items 3 to 4). Each is independently reversible
(restoring the fixed cap reverts ADR-0262 without touching D6 to D9; deleting the
field reverts ADR-0264).

**The defect.** The Service deploy stream is bounded by one fixed timer, `AppState::streaming_cap` (`DEFAULT_STREAMING_CAP` = 90 s, `overdrive-control-plane/src/lib.rs:288`, `:786`), fired as `Failed { Timeout { after_seconds } }` (`streaming.rs:995`, `:1075-1083`; the Job stream shares it); the CLI envelope is a fixed 120 s (`overdrive-cli/src/http_client.rs:276-292`). A Service with startup deadline `D` (`interval_seconds x max_attempts`, default 60 s, `service_lifecycle.rs:758-770`) that becomes `Stable` between 90 s and `D` is reported `Failed { Timeout }` while healthy; and with D7 the give-up (about `6 x D + 5 s` = 365 s) is unreachable inside 90 s, so the timeout is the operator's only terminal and carries no cause.

**Where the deadline lives.** In the server-side Service stream handler, the existing owner of the timer; it already holds the workload's `LifecycleEvent`s (`alloc_id`, `to`, `reason`, `terminal`, `detail`, `action_shim/mod.rs:275-311`), so the last attempt's cause needs no new read. A CLI disconnect ends the stream; `overdrive deploy` again returns `Unchanged` and opens a new stream with a new clock. The durable record is the rows.

**Contract.**

1. **Derived, not configurable.** The Service stream's deadline is
   `progress_deadline(startup_probes) = max(streaming_cap, startup_deadline +
   PROGRESS_DEADLINE_MARGIN)` where `startup_deadline` is the existing derivation
   (`interval x max_attempts` of startup probe index 0; `DEFAULT_STARTUP_DEADLINE`
   when the list is empty or the product overflows) and
   `PROGRESS_DEADLINE_MARGIN` = 30 s. It exceeds the startup deadline by at least 30 s;
   no TOML key, flag or server setting is added. At the default spec the value is 90 s.
   `streaming_cap` stays the floor and the Job stream's bound, unchanged.
2. **Clock.** Armed at stream open (after `Accepted`) with the injected `Clock`. It is
   **not extended**: a `Stable` ends the Service stream, so progress cannot be observed
   mid-stream.
3. **Terminal.** At the deadline the stream emits the existing
   `ServiceSubmitEvent::Failed { alloc_id: None, reason: ServiceFailureReason::Timeout {
   after_seconds }, stderr_tail: None }` with `after_seconds` = the deadline, plus the
   new optional field of item 4. `ServiceFailureReason` is unchanged (it is rkyv
   archived; a field on `Timeout` would change its archived layout).
4. **New wire surface (named): `ServiceSubmitEvent::Failed` gains
   `last_attempt: Option<LastAttemptWire>`** (`#[serde(default,
   skip_serializing_if = "Option::is_none")]`, additive), and a new serde +
   `ToSchema` struct `LastAttemptWire { alloc_id: String, reason: TransitionReason,
   terminal: Option<TerminalCondition>, detail: Option<String> }`, composed of
   existing types. The stream keeps in memory the latest `LifecycleEvent` of this
   workload whose `to` is `Failed` or `Terminated` and projects it into the
   timeout; a stream that saw none emits `None`. It is populated **only** on the
   synthesised timeout; `service_event_from_terminal` and every other `Failed` set
   `None`. `api/openapi.yaml` is regenerated. The detail is labelled "last attempt"
   in the render and is never the verdict: the restart authority's give-up (D6, D7)
   stays the only authoritative `Failed { cause }`.
5. **Pure derivation fn (named, new public surface).**
   `overdrive_reconcilers::service_lifecycle::startup_deadline(&[ProbeDescriptor])
   -> Duration` (extracting the existing derivation behind `spec_facts_for_service`,
   which then calls it) and `overdrive_control_plane::streaming::progress_deadline(
   startup_probes: &[ProbeDescriptor], floor: Duration) -> Duration`, with
   `PROGRESS_DEADLINE_MARGIN`. One derivation, called by the submit handler
   (`handlers.rs:455-546`, both `Inserted` and `Unchanged`) and by the CLI on the
   `Service` that `Service::from_submit` validated client-side (`deploy.rs:654`).
   **Both sides use the same floor:** in production `streaming_cap` is always
   `DEFAULT_STREAMING_CAP` (`lib.rs:786`; the stale doc naming a `[server]
   streaming_submit_cap_seconds` key, `lib.rs:283-287`, is corrected with this feature),
   and `overdrive-cli` already depends on `overdrive-control-plane` (`Cargo.toml:77`), so
   the CLI imports the constant and its timeout `deadline + 30 s` cannot fall below the
   server deadline. If `streaming_cap` ever becomes operator-configurable, the server
   must report its deadline to the client; no such surface is added now.
6. **`build_service_stream` takes the deadline** as a new parameter
   (`streaming.rs:988`); the handler passes `progress_deadline(&service.startup_probes,
   state.streaming_cap)` (`handlers.rs:630`). No intent-store read in the stream.
7. **CLI.** `ApiClient::submit_workload_streaming` takes the server's deadline
   (`http_client.rs:270`) and sets its total request timeout to `deadline + 30 s`
   instead of the fixed 120 s (otherwise a deadline above 90 s is cut by the client and
   hides the typed terminal, `http_client.rs:275-287`); the Service lane passes
   `progress_deadline(&spec.startup_probes, DEFAULT_STREAMING_CAP)`, the Job lane
   `DEFAULT_STREAMING_CAP` (byte-identical). `format_service_failed_block` takes
   `last_attempt: Option<&LastAttemptWire>` in the place of D6's removed
   `early_exit_timing`. The `Timeout` text becomes "workload did not become Stable
   within {after_seconds}s; the platform may still be restarting it (see `overdrive
   workload describe`)", followed, when present, by a `last attempt (alloc <id>):`
   block carrying `reason.human_readable()` (or the `Stopped { by }` text) and the
   `detail` tail. Exit code 1.
8. **Job workloads are unchanged**: the Job stream keeps `streaming_cap`.
9. **D7 item 5 extends to `Stopped { by: LivenessProbe }`** (a one-arm extension: a
   stream can still be open when a liveness stop's event arrives, see D9 item 6 and D11).
10. **The last attempt** is the latest attempt-ending lifecycle event this stream
    observed; no probe-row read, no attempt number, nothing before the stream opened
    (a recorded verdict is reported at once by D11).

**Assumption DL-6 and exposure E-4 (the margin is exact, not slack).** The stream
clock starts at stream open; the startup deadline counts from `started_at`, set when
the `Running` row is written, after placement and VM boot. A healthy instance passing
on its last allowed attempt reaches `Stable` at about `open + boot + D + one
evaluation`, so the stream does not time out iff `boot + placement + one evaluation +
(the first probe interval, if the pass comes on the last attempt) <
PROGRESS_DEADLINE_MARGIN`. `VM_BOOT_DEADLINE` is 30 s (`vm_driver.rs:63`, used at
`:1598`, `:1675`), equal to the margin, so a guest booting at its deadline then passing
on its last attempt is `Stable` after the progress deadline. The invariant is stated
with its assumption: **DL-6, boot plus placement plus one evaluation is at most the
margin.** Where DL-6 fails, the stream ends `Failed { Timeout }` with `last_attempt:
None` for an instance that then reaches `Stable`: an honest timeout, never a false
success and never a retracted failure. That is exposure E-4, stated and bounded, pinned
by S-35 (f); the model's `HealthySlowStartNeverTimesOut` is scoped to DL-6 (modeler
list). The margin stays the literal 30 s (B5, 2026-11-03).

K1 as restated is in § Changed and unchanged assumptions.

Alternatives: ADR-0262, ADR-0264.

**Operator view.** A healthy start is `Stable` as soon as the startup probe passes, even after 90 s; a Service never ready ends at the deadline with `Failed` and the `last attempt` block (exit 1); a crash loop (D6) and a give-up inside the deadline (D7) end with their own terminals.

**Persisted and wire format impact.** No archived, CBOR or intent change. One
additive stream-wire field and one new wire struct (OpenAPI regenerated).

### D11 — the Service deploy stream first reports the verdict the workload's current attempt already carries, then waits

Decision record: [ADR-0263](../../product/architecture/adr-0263-a-service-deploy-stream-first-reports-the-verdict-the-workload-already-has.md).
Independently reversible (removing the snapshot step restores the wait-only stream
without touching D10's deadline or D6 to D9).

**The defect (F-5).** `overdrive deploy` of a byte-identical spec returns `Unchanged` and still opens the Service stream (`handlers.rs:518-546`, `:609-635`). After `Accepted` that stream only subscribes to the `LifecycleEvent` broadcast and waits (`streaming.rs:1010-1056`); it reads no observation row (`:1058-1064`), unlike the Job stream, which recovers the recorded terminal first (`workload_terminal_from_snapshot`, `streaming.rs:217-234`, `:580-625`, and on `Lagged`, `:283-300`). Nothing publishes a new terminal for an already-`Stable` allocation (events are published once per committed row write, `action_shim/mod.rs:2147-2152`, `:2187-2193`; `ServiceLifecycle` skips an allocation in `stable_announced`, `service_lifecycle.rs:473-479`), so the stream ends `Failed { Timeout }` for a healthy `Stable` Service. Two more paths share the root cause: the first-deploy race (`Stable` published before the stream subscribes, `streaming.rs:998-1010`) and a lagged subscriber (`:1058-1064`).

**Production path.** `overdrive serve`, then `overdrive deploy web.toml` until the
service is `Stable` (exit 0), then `overdrive deploy web.toml` again (same bytes).

**Contract.**

1. **Order, in `build_service_stream`:** emit `Accepted`; **subscribe** to the
   lifecycle bus; **read the recorded verdict** (below); arm the progress deadline
   (D10) only if no verdict was found; then wait as today. Subscribe-before-read is
   load-bearing (List-then-Watch): the shim commits the row before it publishes the
   event (`action_shim/mod.rs:2148 -> 2152`, `:2187 -> 2193`), so an earlier event's row
   is visible to the read and a later event is in the channel; a terminal visible to both
   ends the stream on the first.
2. **The recorded verdict** is read from the observation store: the workload's
   rows, the **current attempt** selected by the reconciler's own rule
   (`overdrive_reconcilers::current_alloc`, the numeric-maximum attempt suffix;
   existing `pub` function), and that row's `terminal` projected through
   `service_event_from_terminal`. If the projection is `Some`, the stream emits that
   line and ends. If the current attempt has no terminal, or its terminal projects to
   `None` (D7 item 5: `Stopped { StartupProbe }` and `Stopped { LivenessProbe }` are
   restartable and are not a verdict), the stream waits. A superseded attempt's
   terminal is never reported; the live path needs no such filter because it never
   receives a projecting event for a superseded attempt (§ Pinned readings, R-4).
3. **Same wire as the live path.** For a row `R` and the lifecycle event published
   for the write of `R`, the snapshot line is byte-identical to the event line (S-41).
   The projection is `service_event_from_terminal(alloc_id = the row's alloc id,
   terminal = the row's terminal, stderr_tail = the row's `detail`, last_exit_code =
   None)`; the live path passes the event's `detail` and `None` (`streaming.rs:1032-1038`),
   which is the write's occurrence `detail` (`lifecycle_event_from_occurrence`,
   `action_shim/mod.rs:505`). That it equals the committed row's `detail` is read from
   the write path, not proven here; S-41 discharges it, and a mismatch is a DESIGN
   deviation to surface. The row's `stderr_tail` field is not used.
4. **Relist on `Lagged`.** The stream repeats the read of item 2 (as the Job stream
   does; the level-triggered backstop of `.claude/rules/reconcilers.md` § "The wakeup
   model"), then continues to wait.
5. **A failed read is not absorbed.** If the observation read fails, at open or on
   a relist, the stream ends with a stream error (`Err`, the existing
   `workload_event_from_lifecycle_with_state` precedent, `streaming.rs:259-262`);
   it does not treat the failure as "no verdict" (`.claude/rules/rust.md` § "Distinct
   failure modes get distinct error variants"). The Job stream's `.ok()?` (`:585`) is
   not copied. **D11-E1 (user-approved 2026-11-09)** pins the exact typed error
   representation and mappings under § D11 interface changes below.
6. **Unchanged:** `ServiceLifecycle`'s once-per-allocation `Stable` announcement and
   its dedup (**B1** changes what the dedup keys on, not that there is one), the Job
   stream, the wire (`ServiceSubmitEvent`, `ServiceFailureReason`, ADR-0264's
   `last_attempt`), every row, `View` and intent, the CLI and the handler (it opens the
   same stream for `Inserted` and `Unchanged`).
7. **The verdict is a pure function of rows; the store read is at the edge.**
   `build_service_stream` reads the workload's allocation rows once per open and once
   per `Lagged` (`ObservationStore::alloc_status_rows`) and hands them to a
   **module-private** pure function (no clock, store handle or stream state; the
   crafter names it) that selects the current attempt, projects its terminal through
   `service_event_from_terminal` and returns the stream line or none. S-39 (cases a to
   f) and S-41 are unit tests of it over constructed rows; S-40 and S-42 drive the
   stream. A failed read is the edge's error (item 5) and never reaches the function.
8. **Reporting any stored verdict.** The stream reports `Stopped { Operator }` (exit 0)
   for a stopped workload and a stored give-up for a given-up one. It states what is
   recorded; it does not retry, restart or clear anything. No new timer, deadline,
   config or wire change; D11-E1 adds only the in-process Rust stream error type;
   D10's deadline arms after the read.

**Owner.** The Service submit stream handler, which already owns "what does this
deploy report" (ADR-0056, ADR-0059 Q4), reading state through the existing
observation port.

**Operator view.** A redeploy of the same spec reports the recorded verdict at once (`Stable`, a stored give-up, or `Stopped` naming the operator; the `/stop` sentinel persists until `overdrive workload restart`, `handlers.rs:941-993`); a starting or restarting Service waits under D10's deadline.

**Persisted and wire format impact.** None.

Alternatives: ADR-0263. Alternatives to D1: ADR-0255.

**Formal protocol model.** D11 is an ordering-sensitive protocol (subscribe, read,
publish, commit); it is module `stream_snapshot` in
`specs/quint/service-allocation-readiness-and-ending/` (§ Formal protocol model). Its hazard is
`read_before_subscribe` (violates `NoMissedTerminal`). Conformance projection: the
first terminal line yielded by `build_service_stream` under `SimClock` with
`SimObservationStore` rows and bus events scripted around the subscribe point; no new
accessor.

Gate G-8: § Lifecycle Gate Ownership. Obligations S-39 to S-42: § Executable obligations.

### D12 — a startup-probed allocation is a backend only after its startup has passed

Decision record: [ADR-0267](../../product/architecture/adr-0267-a-startup-probed-allocation-is-a-backend-only-after-its-startup-has-passed.md).
It amends the operative
[ADR-0096](../../product/architecture/adr-0096-startup-failure-withdraws-backend-eligibility.md)
(dated amendment note recorded there, per `.claude/rules/design.md` § "Do not
manufacture amendment history before implementation": ADR-0096 is implemented, so the
change is stated, not rewritten).

**Grounding (production path).** Under `overdrive serve` + `overdrive deploy` of a Service with a startup probe and no readiness probe, the first allocation is `healthy` as soon as its row is `Running`: `compute_backend_healthy` returns true before any startup row exists (`service_lifecycle.rs:1145-1148 @ 04052ad75`); the only startup-related hold is `!fact.startup_probes_empty && !next_view.terminal_announced.is_empty() && latest startup != Pass` (`:1136-1144`), false for a first allocation. A crash successor was held only incidentally, by the early-exit announcement ADR-0258 deletes.

**1. The rule.** In the backend-eligibility predicate (`Backend.healthy`, authored by
`ServiceLifecycle`, ADR-0101), in this order:

1. **New.** If the Service declares a startup probe (`!startup_probes_empty`, the set
   `ServiceAllocFact` already reports) and the allocation's status row does **not**
   carry a committed `Stable` claim: `healthy = false`.
2. Otherwise the existing readiness rule, unchanged: no readiness probe means
   `healthy = true`; a declared readiness probe uses its latest row and
   consecutive-success threshold.

**Deleted (ruling R2-1, 2026-11-03): the ADR-0096 per-allocation terminal veto**
(`startup_failed_this_tick`, the allocation's own `terminal_announced` membership read as
a veto) **and the veto-before-stop ordering** (the backend-row splice ahead of the startup
action, ADR-0096 D2). They are unreachable: the startup stop is gated on the latest
startup row not being `Pass` and `Stable` on its being `Pass`, so a stop-eligible
allocation has no committed `Stable` and step 1 already withholds it, at the deciding
tick and after; an allocation with a committed `Stable` is skipped by the reconcile loop
before the stop gates (B1). The deleted conjunct of the old successor hold is the
`terminal_announced.is_empty()` test (`:1136-1144`). Step 1 subsumes every case the hold
and the veto covered. ADR-0096 carries a dated supersession note (D1 step 1, D2).

**2. The signal: the committed `Stable` claim, not the latest startup row.** The fact
is `ServiceAllocFact.stable_committed`, added by B1 / ADR-0265 and hydrated from the
allocation's own row (`terminal` is `Stable`). D12 adds **no field**. Why this signal:

- *Authoritative.* `Stable` is announced iff the allocation is `Running` and the latest
  startup row is `Pass` (R-2, `service_lifecycle.rs:538-558`), so a committed `Stable`
  implies the startup probe passed; D9 already treats it as "startup done".
- *No flap, no stranding.* A `Fail` row can land between the `Pass` and the startup
  role's retirement at `Stable` (D9 `on_alloc_stable`, R-2); keying on the latest startup
  row would turn a `Stable` allocation unhealthy with nothing to turn it back. The
  committed claim is monotone (no later writer drops it, V31) and re-read from the row
  every evaluation, so a restart does not change it. A `Pass, Fail, Fail` history before
  `Stable` is committed is ineligible and stop-eligible, consistently (R-2).
- *Not a view marker.* `stable_announced` recorded what was emitted, not what landed (R2-2 deletes it); eligibility keyed on it would route to an allocation whose `Stable` write was lost.
- *Cost.* Eligibility lags the startup `Pass` by the `Stable` write plus one evaluation; a lost write holds traffic until B1's re-announcement lands (the bound liveness has too). Intended.

**3. What changes, by case.**

- First allocation, startup probe declared (readiness probe or not): `healthy` only when its `Stable` is committed, then the readiness rule.
- Restart successor after a startup stop (D7, D8) or a crash (D6): held until its own `Stable` is committed, identical to a first allocation; no lineage state.
- Startup-probed allocation after `Stable`: unchanged, eligible per the readiness rule; later startup rows cannot change it.
- Opt-out Service (explicit empty startup list; `startup_probes_empty`): **unchanged**, `healthy` at `Running` (its first `Running` is its `Stable`, ADR-0058); step 1 does not apply.
- Allocation whose startup failure was decided (D7): `healthy: false` because its `Stable` is not committed; the veto is gone.
- Late startup `Pass` after a stop that did not land (D8 item 3): eligible once that `Stable` is committed.

**4. Interaction with the readiness gate.** Readiness is evaluated only after step 1
passes. The readiness probe task is not deferred (D9 gates liveness only), so a streak
already at threshold makes the allocation eligible at the `Stable` commit; readiness
withdraw-and-recover after `Stable` is unchanged.

**5. Deleted code and the tests that pin it.** Deleted: the `terminal_announced`
non-empty hold in `compute_backend_healthy` (`service_lifecycle.rs:1136-1144`), the
`latest_startup_probe` read in it, the per-allocation veto branch and its
`startup_failed_this_tick` set (R2-1); the predicate becomes the step-1 test on
`stable_committed`. Whether the private helper signature changes is the crafter's.
Single cut, no flag, no fallback. DISTILL rewrites the tests pinning the old behaviour
(confirming the set by running the suite):
`backend_policy_preserves_veto_threshold_and_unrelated_view_inputs`
(`service_lifecycle.rs:1265-1320`, becoming the two-step oracle over startup declared x
`stable_committed` x readiness, S-46) and the startup-probed `healthy` assertions in
`overdrive-sim/tests/integration/service_backend_projection.rs`,
`overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs`,
`overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs` and
`overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs`. E09 and E13
(`verification/expectations/`) are point-in-time records, not rerun or edited.

**6. Operator-visible behaviour change (user-approved).** A Service that declares a
startup probe receives no traffic until its startup probe has passed (the existing fail-closed
outcome for a frontend with no healthy backend); a Service needing traffic from
`Running` declares the opt-out.

**7. Alternatives** are recorded in ADR-0267.

**8. Cost.** Pure code in `reconcile`'s backend projection; no extra read, field, action, port, wire or persisted shape.

## Wave: DESIGN / [REF] Pinned readings of the accepted decisions

Clarifications raised by the Quint model (2026-11-03). Each pins what an accepted decision and the
current code already imply; none changes a contract, a public surface or a persisted shape. R-3 is
recorded in place (§ D7, exposure E-2).

**R-1 — a platform-reclaimed final row is restarted, consumes no budget and is never the row a
give-up is decided on (D6, D7).** `restart_decision` is `attempts < CEILING || platform_reclaimed ||
generation_replacement`, true for a reclaimed row (`Terminated`,
`Stopped { PlatformReclaimed }`, no `terminal`) at any attempt count; the successor inherits the
predecessor's count and `last_failure_seen_at`; the ceiling branch is never
entered for it, so the four give-up terminals are selected only from a `Failed` or non-reclaimed
`Terminated` final row. `ServiceLifecycle` does not act on it (D5) and the stream does not project
it. S-15's grid row `Terminated` + `PlatformReclaimed` at the ceiling expects `RestartAllocation`
and no terminal.

**R-2 — the startup gate reads the latest startup row (D5, D7, D8).** `no_pass` is
`!matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass))` over
the last-writer-wins projection of the startup row; the attempt count is that row's
producer-counted streak when it is `Fail`, 0 otherwise. `Stable` is announced iff the
allocation is `Running` and the latest row is `Pass`; the stop iff it is `Fail` with the gates
holding; the two partition a `Running` startup-probed allocation (an ever-passed reading would
strand `Pass, Fail, Fail`). A startup row can follow a `Pass` only before `Stable`: `on_alloc_stable`
retires the role after the `Stable` row commits, and a committed `Stable`
(B1) skips the allocation before any gate. `Pass, Fail, Fail` with the deadline passed stops the
instance (a restartable event).

**R-3 — a crash racing the stop (E-2).** See § D7, "Exposures".

**R-4 — the Service stream's live path needs no current-attempt filter (D11).** The only events
whose `terminal` projects to a stream line are published by the shim at the `Stable` commit, the genuine-terminal commit and the `StopAllocation` commit; every other publisher carries `terminal: None` or writes the non-terminal row of the
attempt being started. Those sites run inside one evaluation's dispatch of `workload/<id>`; the loop
admits one evaluation per target at a time and awaits the dispatch inside it; the successor row is written only by
a later `RestartAllocation` dispatch, and the predecessor cleanup writes no row and publishes no
event. So a projecting event is always published while its attempt
is current, and no live-path filter is added. Residual: an event published while attempt N was
current can be consumed after attempt N+1 exists (one broadcast hop); the stream reports the verdict
that was true at publication, as for an operator stop. Model: assumption SS-6 and the `Restart`
guard (S-40).

**R-5 — HO-5 and RA-2 are discharged by validation items V27 and V28.**

## Wave: DESIGN / [REF] Component decomposition and C4

No new component, crate, container or port. Container topology is unchanged, so
`c4-diagrams.md` is not touched. Component relationships touched by this feature:

```mermaid
C4Component
  title Terminal-hook ownership - control-plane worker components
  Container_Boundary(cp, "overdrive-control-plane / overdrive-worker") {
    Component(shim, "Action shim", "Rust", "Executes lifecycle Actions")
    Component(obs, "Exit observer", "Rust", "Consumes ExitEvents, authors Failed or Terminated rows")
    Component(drv, "VmDriver", "Rust, Driver port", "Owns VM supervision and the exit watcher")
    Component(runner, "ProbeRunner", "Rust", "Owns per-allocation probe supervisors")
    Component(reconc, "WorkloadLifecycle and ServiceLifecycle", "Rust", "Restart authority and probe consumer")
  }
  ContainerDb(obsstore, "ObservationStore", "Observation rows")
  Rel(drv, obs, "Emits ExitEvent on natural guest exit to")
  Rel(obs, drv, "Fires on_alloc_terminal on, per ExitEvent (D1)")
  Rel(obs, obsstore, "Writes Failed or Terminated row to")
  Rel(shim, drv, "Calls on_alloc_running (D3, before gate release), then release_for_exit_emission")
  Rel(shim, drv, "Calls on_alloc_terminal on, after StopAllocation and genuine FinalizeFailed (D4)")
  Rel(drv, runner, "Starts and stops supervisor in")
  Rel(runner, obsstore, "Writes ProbeResultRow to")
  Rel(reconc, obsstore, "Reads rows from")
  Rel(reconc, shim, "Emits RestartAllocation, StopAllocation, FinalizeFailed to")
```

**Architecture style.** Unchanged: ports-and-adapters (`Driver` is a port in `overdrive-core`; the
observer holds `Weak<dyn Driver>`). The `xtask dst-lint` `release-supervision-placement` clause over
`worker/exit_observer.rs` is unaffected (the hook call sits outside `match outcome`; DISTILL
confirms).

## Wave: DESIGN / [REF] Driving and driven ports

| Port | Kind | Role in this feature |
|---|---|---|
| `Receiver<ExitEvent>` from `Driver::take_exit_receiver` | driving port of the exit observer | Injects the guest-exit fact. Existing. |
| `Action::{StartAllocation, RestartAllocation, StopAllocation, FinalizeFailed}` through `dispatch` | driving port of the shim | Existing. |
| `Driver::on_alloc_terminal` / `on_alloc_running` / `release_for_exit_emission` | driven port (the `Driver` port) | `on_alloc_terminal` contract rewritten (D2); call order of the other two pinned (D3). |
| `ObservationStore::write_alloc_lifecycle` | driven port | Unchanged. |

`ProbeRunner::active_alloc_count()` and `is_role_live()` are read-only inspection surfaces
(`probe_runner/mod.rs:414`, `:407`); nothing adds a write method to them.

## Wave: DESIGN / [REF] Interface contract changes

D1-D5 change one interface text and no signature, type, variant, error, wire or persisted shape.
D6 changes the public surface below and adds no archived, wire or persisted shape. D7 adds one archived, wire and API-visible
enum variant. D8 adds one additive CBOR `View` field. D9 adds one `pub` method and one docstring
sentence. D10 adds two pure functions, one constant, one wire field with one wire struct, and two
changed public signatures. D11-E1 adds one in-process public Rust error enum and changes
`build_service_stream`'s stream item error type; HTTP/NDJSON is unchanged.

### `Driver::on_alloc_terminal` — replacement docstring (D2)

> Lifecycle hook: this allocation's workload has ended, or its terminal state
> is being claimed. Release every per-allocation supervision resource the driver
> holds for `alloc_id`.
>
> Production drivers that hold the worker's `ProbeRunner` override this to call
> `probe_runner.stop_alloc(alloc_id)`, cooperatively cancelling every per-probe
> task and dropping the allocation's supervisor (ADR-0054 § 2).
>
> # Who calls it
>
> Whoever ends an allocation or observes its end, for the driver that owns it:
> the exit observer once per `ExitEvent` it consumes, before and independently of
> any row write; the action shim after `StopAllocation` and after a genuine-terminal
> `FinalizeFailed`. A caller never needs a terminal `AllocStatusRow` to exist
> first, and there is no ordering relative to the row write. A restart's
> predecessor needs no separate call: its end was already announced by one of the
> callers above.
>
> # Preconditions
>
> - `alloc_id` names one physical execution attempt that will not be started
>   again under the same id (allocation ids are not reused after a terminal,
>   ADR-0105). A fresh replacement has a different id.
> - Called from a synchronous context. The caller holds no lock the
>   implementation may need.
>
> # Postconditions
>
> - After return, the driver holds no probe supervisor for `alloc_id`: no
>   further probe task is started for it, and each task parked on its interval
>   exits at its next scheduling point. A probe attempt already past its
>   interval wait at the time of the call may complete and write one
>   `ProbeResultRow` for `alloc_id`. This is bounded by one attempt per task and
>   by the descriptor timeout, and no reader acts on it (see Q3).
> - Supervision of every other allocation, including the successor of a
>   restarted allocation, is unchanged.
>
> # Edge cases
>
> - **Unknown, never-registered, or already-stopped `alloc_id`: no-op.** This is
>   the normal case for a second author and for non-VM drivers.
> - **Repeated calls, from the same or different callers, in any order:
>   idempotent.** Concurrent calls are safe.
> - **A driver with no probe runner: default no-op.**
> - Infallible and non-blocking: it does no I/O and no `.await`, and writes no
>   observation row.
>
> # Observable invariants
>
> - For a driver with a `ProbeRunner`: the supervised-allocation count falls by
>   exactly one when `alloc_id` was supervised, and is unchanged otherwise.
> - A call never changes the `AllocStatusRow` of any allocation.
>
> Default no-op, symmetric with `on_alloc_running`.

### `Driver::on_alloc_running` — one added ordering sentence (D3)

Append to the existing docstring (`driver.rs:720-737`):

> The action shim calls this **before** it calls `release_for_exit_emission` for
> the same allocation, so any `ExitEvent` for the allocation is emitted only
> after its probe supervision is registered.

### Exit observer call-order contract (D1; internal behaviour, not a signature)

For each consumed `ExitEvent`: (1) transiently upgrade the `Weak<dyn Driver>`, call
`on_alloc_terminal(&event.alloc)`, drop the strong reference; (2) run the existing retry-bounded
row write; (3) the existing `release_supervision` at the abandonment boundary, unchanged. The strong
reference must not be live across (2) (`exit_observer.rs:177-197`). Module docs at
`exit_observer.rs:1-44` state that the observer also owns the hook for natural exits.

### D6 interface changes

| Surface | Change |
|---|---|
| `ServiceAllocFact.exit_code` (`service_lifecycle.rs:111`) | **Deleted**, with its hydration (`:891-894`, `:899`) |
| `ServiceLifecycleReconciler::reconcile` | Emits no `Action` for a row in state `Failed` or `Terminated`; branch `:560-601` deleted. **DG-12:** inserts every `Failed` or `Terminated` fact into the returned `View`'s `terminal_announced` (pub field), whatever its `started_at`. No type, field or signature change |
| `WorkloadLifecycleState.current_startup_probe: Option<ProbeStatus>` (`overdrive-reconcilers`) | **New field** (D6 item 4), populated by `hydrate_actual` only |
| `WorkloadLifecycle::reconcile`, ceiling branch | Terminal selection gains `ServiceFailed { EarlyExit { exit_code } }` between the liveness case and `BackoffExhausted` (D6 item 4); the idempotency guard also matches it |
| `ServiceFailureReason::EarlyExit { exit_code }` | Shape unchanged; doc rewritten |
| `format_service_failed_block` (`overdrive-cli`) | The `early_exit_timing: Option<(u64, u64)>` parameter and the `elapsed:` line are removed |
| `restart_budget_from_rows` (`handlers.rs:1399`) | Treats `ServiceFailed { EarlyExit }` as exhausted, `used = RESTART_BACKOFF_CEILING` |
| Archived, serde, `ToSchema`, OpenAPI, CBOR `View`, wire | **None** (views, `AllocStatusRow`, `TransitionReason`, `TerminalCondition`, `ServiceFailureReason` unchanged) |

### D7 interface changes

| Surface | Change |
|---|---|
| `StoppedBy` (`transition_reason.rs:349`) | **New variant `StartupProbe`**, fieldless, tail-appended after `LivenessProbe` (rkyv discriminant 6, serde and OpenAPI `startup_probe`). Arms added at `human_readable` (`:900-907`) and `by_reclaims_platform` (`:1027-1038`) |
| `startup_probe_failed_action` (`service_lifecycle.rs:1168`; private) | Returns `Action::StopAllocation { terminal: Some(Stopped { by: StartupProbe }) }` instead of `FinalizeFailed { StartupProbeFailed }`. Splice `:634-647` and `startup_failed_this_tick` **deleted** (R2-1). No new `Action` variant |
| `WorkloadLifecycleState.current_startup_probe` | **No second field.** `last_fail` is the total mapping in D7 item 3 over this field |
| `WorkloadLifecycle::reconcile`, ceiling branch | Terminal selection gains `ServiceFailed { StartupProbeFailed { 0, last_fail, restart_counts } }` between the liveness case and D6's `EarlyExit` case; the guard also matches it. `is_intentionally_stopped` is **not** extended |
| `service_event_from_terminal` (`streaming.rs:825`, `pub`) | Returns `None` for `Stopped { by: StartupProbe }` and, by D9, for `Stopped { by: LivenessProbe }`. Every other `Stopped { by }` projects unchanged |
| `restart_budget_from_rows` (`handlers.rs:1399`) | Also treats `ServiceFailed { StartupProbeFailed }` as exhausted, `used = RESTART_BACKOFF_CEILING` |
| `render::workload_describe` Service arm (`render.rs:1135-1158`) | One presence-guarded line `terminal: stopped (startup probe failed)` for a `Stopped { StartupProbe }` row |
| `ServiceFailureReason::StartupProbeFailed` (`transition_reason.rs:689`) | Doc rewritten; `attempts` is the restart budget consumed at give-up |
| `api/openapi.yaml` | `StoppedBy` enum gains `startup_probe` (regenerated) |
| rkyv | No envelope version. Two new golden fixtures in `crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs` (`StartupProbe`, `LivenessProbe` per F-4, S-25) |
| CBOR `View`, `ServiceFailureReason` shape, `Action`, shim | **None** in D7 (D8 adds one `View` field) |

### D8 interface changes

| Surface | Change |
|---|---|
| `ServiceLifecycleView.startup_last_acted_run: BTreeMap<(AllocationId, ProbeIdx), ProbeRunSeq>` (`service_lifecycle.rs`, beside `:338`) | **New field**, `#[serde(default)]` (D8 item 1); no envelope |
| `ServiceLifecycleReconciler::reconcile`, startup stop emission and top-of-loop dedup (`:488-490`) | A startup stop is emitted once per new written startup row while the fact is `Running`; `terminal_announced` no longer skips a `Running` fact; `Pass` after a stop ends the retry; `Stable` removes the allocation from `terminal_announced` (D8 item 3). No new `Action` |

### D9 interface changes

| Surface | Change |
|---|---|
| `ProbeRunner::start_alloc` (`probe_runner/mod.rs:307`) | Spawns startup and readiness tasks; retains liveness descriptors on the supervisor unspawned |
| `ProbeRunner::start_liveness(&self, alloc_id: &AllocationId)` | **New `pub` method** (D9 item 2): idempotent, no-op for an unknown or ended allocation, serialised with `stop_alloc` by the supervisor lock |
| `VmDriver::on_alloc_stable` (`vm_driver.rs:2065`) | Calls `stop_role(Startup)` then `start_liveness` |
| `Driver::on_alloc_stable` docstring (`traits/driver.rs:753-770`) | One sentence added; no signature change |
| `collect_liveness_actions` `stable_this_tick` skip (`service_lifecycle.rs:980-983`) | Unreachable after D9; removed |
| Archived, wire, `View`, OpenAPI | **None** |

### D10 interface changes

| Surface | Change |
|---|---|
| `overdrive_reconcilers::service_lifecycle::startup_deadline(&[ProbeDescriptor]) -> Duration` | **New `pub` pure fn**: the existing derivation (`interval x max_attempts` of startup probe index 0; `DEFAULT_STARTUP_DEADLINE` for an empty list or on overflow), extracted from `spec_facts_for_service` |
| `overdrive_control_plane::streaming::PROGRESS_DEADLINE_MARGIN: Duration` (30 s) and `progress_deadline(startup_probes: &[ProbeDescriptor], floor: Duration) -> Duration` (`max(floor, startup_deadline + margin)`) | **New `pub` const and pure fn** |
| `build_service_stream` (`streaming.rs:988`) | **Signature gains `progress_deadline: Duration` as its last parameter (DG-7)**; the cap timer arms from it |
| `ServiceSubmitEvent::Failed` (`streaming.rs:776`) | **New optional field `last_attempt: Option<LastAttemptWire>`**, `#[serde(default, skip_serializing_if = "Option::is_none")]`; set only by the synthesised timeout |
| `LastAttemptWire { alloc_id: String, reason: TransitionReason, terminal: Option<TerminalCondition>, detail: Option<String> }` | **New wire struct**, serde + `ToSchema`, composed of existing types; defined in `overdrive_control_plane::streaming` (DG-7) |
| `overdrive_control_plane::streaming::service_stream_synth_cap_timeout(after_seconds: u32, last_attempt: Option<LastAttemptWire>) -> ServiceSubmitEvent` (**D10-H1 exact helper-shape clarification**) | Public pure helper consumes the stream's latest attempt-ending observation by value and returns `ServiceSubmitEvent::Failed { alloc_id: None, reason: Timeout { after_seconds }, stderr_tail: None, last_attempt }`. `None` remains the no-observation case; the helper, rather than its caller, owns field population. No new wire type, error, retry or lifecycle state |
| `ApiClient::submit_workload_streaming` (`http_client.rs:270`) | **Signature gains the server deadline as its second parameter (DG-7)**; total request timeout = deadline + 30 s; the Job lane passes `DEFAULT_STREAMING_CAP` |
| `format_service_failed_block` (`render.rs`) | `early_exit_timing` replaced by `last_attempt: Option<&LastAttemptWire>`; renders the `Timeout` text and `last attempt` block |
| `ServiceFailureReason` (archived), `TerminalCondition`, `AllocStatusRow`, CBOR `View`, TOML spec, `AppState::streaming_cap` | **None** |
| `api/openapi.yaml` | `ServiceSubmitEvent::Failed` gains `last_attempt`; `LastAttemptWire` added (regenerated) |

**D10-H1 ownership and provenance.** D10 item 4 and ADR-0264 already require
the stream's optional last attempt to travel as the owned
`ServiceSubmitEvent::Failed.last_attempt: Option<LastAttemptWire>` field; the
accepted D10 interface table already assigned this helper the population of
that field but left its parameter untyped. `service_stream_synth_cap_timeout`
is `pub` in the public `streaming` module and is called from a separate
integration-test crate, so its signature is an interface contract. Passing the
existing `Option<LastAttemptWire>` by value moves the stream's in-memory value
into the approved owned wire field without cloning. At the
deadline `build_service_stream` calls the helper with its current value and
does not mutate the returned event. Existing one-argument callers that test
the no-observation timeout supply `None`. This pins the accepted D10 behavior
without adding a new decision about what the stream reports. The independent
bounded DESIGN review in
`design/review-progress-deadline-helper-contract-remediation.md` approved this
exact clarification before DELIVER step 04-01 resumed.

### D11 interface changes

| Surface | Change |
|---|---|
| `build_service_stream` (`streaming.rs:988`) | D10's four arguments remain, with `progress_deadline: Duration` last. **D11-E1 changes the stream item** to `Result<Bytes, ServiceStreamError>`; it still returns `impl Stream + Send + 'static`. Docstring gains: the current attempt's recorded verdict is reported first; `Lagged` relists; a failed read ends the stream with an error; subscribe precedes the read |
| `overdrive_control_plane::streaming::ServiceStreamError` (D11-E1) | New public in-process enum with exactly `ObservationRead { source: ObservationStoreError }` and `LineEmission { source: std::io::Error }`; implements `std::error::Error + Send + Sync + 'static`, and `Error::source` exposes the nested typed source |
| `overdrive_reconcilers::current_alloc`, `service_event_from_terminal` | **Reused unchanged** |
| Wire, archived, CBOR `View`, OpenAPI, CLI wire parser, handler route | **None**; `Body::from_stream` consumes the new Rust error type through its existing `Error + Send + Sync` bound |

The exact public Rust type and signature are:

```rust
pub enum ServiceStreamError {
    ObservationRead { source: ObservationStoreError },
    LineEmission { source: std::io::Error },
}

pub fn build_service_stream(
    state: AppState,
    workload_id: WorkloadId,
    accepted: ServiceSubmitEvent,
    progress_deadline: Duration,
) -> impl Stream<Item = Result<Bytes, ServiceStreamError>> + Send + 'static;
```

For `ObservationStore::alloc_status_rows` at stream open and on every `Lagged` relist,
each `ObservationStoreError::Unreachable { peer }`, `Io(error)` and
`Envelope { source }` maps unchanged into
`ServiceStreamError::ObservationRead { source: original_error }`. The original
variant, payload, `io::ErrorKind` and envelope cause remain inspectable. The
stream yields that `Err` once and ends, never arms or waits for a timeout after
an open-read failure and never resumes after a failed relist. The existing
`emit_service_line` failure at Accepted, live or recorded terminal, or a
synthesized terminal maps to `ServiceStreamError::LineEmission { source:
original_io_error }`, yields once and ends. These are the exhaustive stream-error
variants: no `Other`, string-only classification, error-to-absence conversion,
retry, or synthetic `ServiceSubmitEvent::Failed` for an observation read. Open
and relist are two sites of the same semantic read failure. The local adapter's
ordinary corrupt-row scan still logs and skips under ADR-0048; its declared
port error set nevertheless includes `Envelope`. The Job stream is unchanged.
Conformance accessors for D7 to D11 remain those in § Formal protocol model.

### Interface changes of B1, B2, B3, D12 and D17

| Ruling | Surface | Change |
|---|---|---|
| B1 | `ServiceAllocFact.stable_committed: bool` (in-memory State) | **New field**, hydrated from `row.terminal` being `Stable`; `reconcile`'s `Stable` branches and top-of-loop skip key on it |
| R2-2 | `ServiceLifecycleView.stable_announced: BTreeSet<AllocationId>` (`service_lifecycle.rs:284`, CBOR `View`) | **Deleted.** The emission dedup (`:474-479`) is replaced by `fact.stable_committed`; `has_alloc_mid_startup_window` (`:345-349`) and `next_evaluation_at` (`:697`) read it instead. Old blobs carrying the key decode; no envelope; S-38 gains the removed-field case |
| R2-1 | `compute_backend_healthy`, `service_backend_row_actions`, `reconcile` (private) | **Deleted:** the backend veto parameter and branch of `compute_backend_healthy` (`terminal_startup_veto`), `startup_failed_this_tick`, and the backend-row splice ahead of the startup action. |
| B2 | Shim `StopAllocation` arm | **Order only**: `on_alloc_terminal` after the terminal row write succeeds; no signature, type or contract text changes |
| B3 | `WorkloadLifecycle` ceiling classification | The pre-startup-crash predicate accepts `WorkloadCrashedImmediately` and `VmOutOfMemory`; `exit_code` is `None` for the latter. No new variant |
| D12 (was B4) | `compute_backend_healthy` (private) | **Behaviour only**: reads `ServiceAllocFact.stable_committed`; the `terminal_announced` successor hold is deleted. No field, signature, wire or persisted change |
| B5 | None | **Rejected.** `PROGRESS_DEADLINE_MARGIN` stays the literal 30 s |
| D17 (Accepted 2026-11-03, exact guard shape D17-S1 approved 2026-11-09) | `ReconcilerRuntime` (`reconciler_runtime.rs`) | **New `pub`: `submit_evaluation(&self, eval: Evaluation, now: Instant, eligibility: EvaluationEligibility)`** (submit, release the lock, signal; the only production submit path), **`BrokerDrain<'_>`** (guard newtype: `drain_pending`, `next_eligible_at`, `counters`, no `submit`) and **`pub(crate) async submitted(&self)`** (consumes the stored or delivered permit; called only by the loop). **Changed:** `broker() -> BrokerDrain<'_>`; **`broker_mutex()` is removed**. Exact receivers, parameters and result types are pinned below. Callers that took the mutex to submit take the runtime; `InterestRouterBroker::from_runtime` submits through `submit_evaluation`; `from_shared_broker` is unchanged and does not wake. `EvaluationBroker` unchanged. Detail: § D17, ADR-0273 |

#### D17-S1 — exact drain-only guard interface

```rust
pub struct BrokerDrain<'a> { /* private guard; no public fields */ }

impl<'a> BrokerDrain<'a> {
    pub fn drain_pending(
        &mut self,
        limit: usize,
        blocked_targets: &BTreeSet<TargetResource>,
        now: Instant,
        now_unix: UnixInstant,
    ) -> Vec<(Evaluation, Duration)>;
    pub fn next_eligible_at(
        &self,
        blocked_targets: &BTreeSet<TargetResource>,
    ) -> Option<UnixInstant>;
    pub fn counters(&self) -> BrokerCounters;
}

impl ReconcilerRuntime {
    pub fn broker(&self) -> BrokerDrain<'_>;
}
```

`BrokerDrain` wraps the per-runtime `parking_lot::MutexGuard<EvaluationBroker>`
and exposes only these three read/drain methods. The parameter and result
types keep `EvaluationBroker`'s existing FIFO, blocked-target, `NotBefore` and
counter meanings. No `submit`, `Deref`/`DerefMut`, `AsRef`/`AsMut`, public guard
field or other escape to `EvaluationBroker` is exposed: every production submit
must call `submit_evaluation` and wake the loop. The loop drops this guard
before any `.await`; the existing `EvaluationBroker` remains unchanged.

Comments naming the early-exit branch's owner, and those D8 to D10 make false (the `streaming_cap` doc, the `build_service_stream` contract, the `Timeout` doc, the `submit_workload_streaming` comment, the `stable_this_tick` comments), are updated in the same change.

### Reuse Analysis (EXTEND vs CREATE NEW)

| Component | Decision | Contract shape | Universe and assertion mechanism |
|---|---|---|---|
| `Driver::on_alloc_terminal` (trait docstring only) | EXTEND (text) | bounded-change | Mutation set: the driver's supervisor entry for `alloc_id`. Universe: supervisor set of the runner. Assert `active_alloc_count()` delta and `is_role_live` per role, before/after. |
| `worker/exit_observer.rs` run loop | EXTEND | bounded-change | Mutation set: supervisor entry for `event.alloc` (new), plus the existing row, broadcast and `release_supervision`. Assert: no supervisor-set delta beyond that id; the row stream is identical to today's. |
| `action_shim` `StartAllocation` and `RestartAllocation` Running sites | EXTEND (statement order) | bounded-change | Only call order. Assert order via a recording driver trace; no `ExitEvent` observable before registration. |
| `cleanup_restart_predecessor`, `FinalizeFailed` arm, `StopAllocation` arm (D7), `VmDriver`, `ProbeRunner`, `ObservationStore` | REUSE unchanged | unchanged | Existing tests plus § DISTILL obligations (S-22) |
| `startup_probe_failed_action` guard (D5) | EXTEND (one predicate swap) | pure-function (return-only) | Universe: the `Option<Action>` it returns. Assert: `None` iff the state is terminal or a gate fails; proptest over `AllocState` x gates (S-21) |
| `ServiceLifecycleReconciler::reconcile`, early-exit branch (D6) | DELETE | pure-function (return-only) | Universe: returned `Vec<Action>` and next `View`. Assert: no action for any `Failed` or `Terminated` fact; `View` unchanged except the pre-`Running` `Failed` bookkeeping (S-21, S-16) |
| `WorkloadLifecycle::reconcile` ceiling-terminal selection (D6) | EXTEND | pure-function (return-only) | Universe: returned `Vec<Action>` and next `View`. Assert: the `FinalizeFailed` terminal per (kind, probe declaration, state, reason, pass) combination; `next_view` equals the input view; no restart past the ceiling (S-15) |
| `WorkloadLifecycle::hydrate_actual` startup-pass read (D6) | EXTEND (reuse `ObservationStore::list_probe_results_for_alloc`) | unbounded-preservation | Universe: the observation store. Assert: store unchanged; a read error yields `HydrateError`, not an empty set (S-17) |
| `restart_budget_from_rows` (D6) | EXTEND | pure-function (return-only) | Assert: exhausted for `BackoffExhausted` and `ServiceFailed { EarlyExit }` rows, not otherwise (S-27) |
| `format_service_failed_block` (D6) | EXTEND (remove a parameter) | pure-function (return-only) | Assert on the returned string (S-19) |
| `ServiceAllocFact.exit_code` (D6) | DELETE | n/a | Deleted with its only reader |
| `startup_probe_failed_action` (D7) and the backend-row splice, `startup_failed_this_tick` and veto branch (R2-1) | EXTEND (change the returned action) / DELETE (splice, set, veto) | pure-function (return-only) | Universe: returned `Vec<Action>` and next `View`. Assert: for the existing gate inputs, the action is `StopAllocation { Stopped { StartupProbe } }` and never a `FinalizeFailed`; no backend-row group is emitted ahead of it for this reason; `terminal_announced` handling is unchanged (S-21) |
| `WorkloadLifecycle::reconcile` ceiling-terminal selection (D7) | EXTEND (one more case, same function as D6) | pure-function (return-only) | Universe: returned `Vec<Action>` and next `View`. Assert: the terminal for each final-row class (liveness kill, startup kill, pre-startup crash, other) is the one in the pinned order and the four are pairwise exclusive; `next_view` equals the input view (S-23) |
| `WorkloadLifecycle::is_restartable` / `is_intentionally_stopped` (D7) | REUSE unchanged | pure-function | Assert a `Terminated` + `Stopped { StartupProbe }` row is restartable and not intentionally stopped; operator and system-gc exemption unchanged (S-23) |
| `WorkloadLifecycle::hydrate_actual` startup-fail read (D7) | The same read as D6: `current_startup_probe` | unbounded-preservation | As the D6 row (S-17) |
| `StoppedBy` (D7) | EXTEND (one variant) | pure-function | Assert match arms, serde and rkyv roundtrip incl. the golden fixture (S-25) |
| `service_event_from_terminal` (D7) | EXTEND (one arm) | pure-function (return-only) | Assert `None` for `Stopped { StartupProbe }`; every other `Stopped { by }` projects unchanged (S-26) |
| `restart_budget_from_rows`, `render::workload_describe` Service arm (D7) | EXTEND | pure-function (return-only) | Assert on the returned struct and string (S-27) |
| `ServiceLifecycleReconciler::reconcile` startup stop dedup (D8) | EXTEND | pure-function (return-only) | Universe: returned `Vec<Action>` and next `View`. Assert: a stop is emitted iff gates and guard hold and the latest startup `run_seq` exceeds the recorded value; the same row never emits twice; a `Running` fact in `terminal_announced` re-emits on a newer row; a `Pass` after an emitted stop emits nothing and the fact reaches `Stable`; `next_view` differs only in `startup_last_acted_run`, `terminal_announced`, `observed` (S-29) |
| `ServiceLifecycleView.startup_last_acted_run` (D8) | CREATE NEW (field, mirror of `liveness_last_acted_run`) | n/a | CBOR decode of an old blob yields an empty map (S-38) |
| `ProbeRunner::start_alloc` / `VmDriver::on_alloc_stable` (D9) | EXTEND | bounded-change | Mutation set: the allocation's liveness tasks (spawned) and its supervisor entry. Universe: the runner's supervisor set. Assert `is_role_live(alloc, Liveness)` is false after `on_alloc_running`, true after `on_alloc_stable`, false after `on_alloc_terminal`; `active_alloc_count()` unchanged by `start_liveness` (S-31) |
| `ProbeRunner::start_liveness` (D9) | CREATE NEW (one `pub` method) | bounded-change | Nothing existing starts retained descriptors |
| `service_lifecycle::startup_deadline` (D10) | EXTEND (extract the private derivation, make it `pub`) | pure-function (return-only) | Assert equality with the value hydration used before, proptest over descriptor lists incl. overflow (S-34) |
| `streaming::progress_deadline`, `PROGRESS_DEADLINE_MARGIN` (D10) | CREATE NEW | pure-function (return-only) | Justified: nothing combines the startup deadline with the stream floor. Assert `progress_deadline > startup_deadline` for every input and `>= floor` (S-34) |
| `build_service_stream` timer (D10) | EXTEND | bounded-change | Mutation set: the stream's yielded lines only (it writes no row). Assert the terminal at the deadline, with `last_attempt`, under `SimClock` (S-35) |
| `ServiceSubmitEvent::Failed.last_attempt`, `LastAttemptWire` (D10) | CREATE NEW | pure-function | Justified: archived `Timeout` cannot gain a field without an envelope version. Serde roundtrip, old-shape decode, OpenAPI (S-35, S-36) |
| `ApiClient::submit_workload_streaming`, `format_service_failed_block` (D10) | EXTEND | pure-function (render) / bounded-change (client timeout) | Assert the request timeout equals deadline + 30 s and the Job lane is unchanged; assert the rendered string (S-36) |
| `build_service_stream` recorded-verdict read (D11) | EXTEND (behaviour; reuse `current_alloc`, `service_event_from_terminal`, `ObservationStore::alloc_status_rows`) | unbounded-preservation + bounded-change (the stream's yielded lines only) | Universe: the observation store (unchanged by the read) and the lines the stream yields. Assert: store unchanged; first terminal line equals the live-path line for the same row (S-41); a `None` projection yields no line; a read fault yields a stream error, never `Timeout` (S-42) |
| `ServiceAllocFact.stable_committed` (B1) | CREATE NEW (one in-memory field) | n/a | Hydration reads the row already read for the fact (S-43) |
| `ServiceLifecycleReconciler::reconcile` `Stable` branches and `ServiceLifecycleView.stable_announced` (B1, R2-2) | EXTEND (key on the committed row) / DELETE (the view field and `has_alloc_mid_startup_window`) | pure-function (return-only) | Universe: returned `Vec<Action>` and next `View`. Assert: `Stable` emitted iff the startup condition holds and `!stable_committed`; a committed fact emits nothing; `next_view` differs from the input only in `observed`, `terminal_announced` and `startup_last_acted_run` (S-43) |
| Shim `StopAllocation` arm hook position (B2) | EXTEND (statement order) | bounded-change | Mutation set: the supervisor entry for the stopped id. Assert via a failing `Observation` write: supervisor still registered; after the retry lands: gone (S-30) |
| `ReconcilerRuntime::submit_evaluation`, `submitted`, `BrokerDrain` (D17, Accepted) | CREATE NEW (one `pub` method, one `pub(crate)` method, one `pub` guard type; `broker_mutex` removed, `broker()` narrowed) | bounded-change | Mutation set: the broker's pending map and one permit. Assert: a submit while the loop is parked is admitted without advancing `SimClock`; a burst yields one drain; with no submits the tick still drains (S-48) |
| `WorkloadLifecycle` pre-startup-crash predicate (B3) | EXTEND (one more accepted reason) | pure-function (return-only) | Assert the S-15 grid gains `Failed` + `VmOutOfMemory` (S-45) |
| `compute_backend_healthy` startup hold (D12) | EXTEND (replace the `terminal_announced` hold and the veto with the committed-`Stable` test) | pure-function (return-only) | Universe: the returned `bool`. Assert: the two-step predicate over (startup declared x `stable_committed` x readiness); an unrelated allocation in `terminal_announced` changes nothing (S-46) |
| `GuestListenerReports` port and `SimGuestListenerReports` (D13 item 10, PR-1) | CREATE NEW (one read-only core port, one `Sim*`; the production adapter is the next row). No existing alternative: the `Driver` trait is not consumed for this by the shim, and the observation-store alternative is rejected in ADR-0269; the pattern is `GuestStreamDial` (mandatory `ProbeRunner::new` parameter, `Sim*` for DST) | read-only port (no write method on it) | Universe: one allocation's reported set. Assert through `GuestListenerReports::reported` and `changed` only (S-50 to S-52) |
| `GuestListenerBoard` (D13 item 10, PR-1) | CREATE NEW (the port's only production implementation; entry map owned by `overdrive-worker`; structure is the crafter's) | bounded-change | Mutation set: the entries of the board, insert-only while an entry exists. Universe: one allocation's reported set, a subset of its declared listeners, no entry before `open` or after `close`. Assert through `reported`: the set only grows and stays within the declared set (proptest over insertion sequences, S-50); a duplicate, undeclared, no-entry or after-close record changes nothing and completes no `changed` (S-51); the wake is seeded (S-52) |
| `ProbeRunner` guest-listener task (D14 item 3) | EXTEND (one task kind beside the dial task; writes only that allocation's startup `ProbeResultRow`s through the dial task's run state) | bounded-change | Mutation set: startup rows of the allocation. Assert (S-53): `Pass` (`same_status_runs` 1) on the insertion that completes the declared set, no row before the deadline, first `Fail` at the deadline (`same_status_runs` 1), one `Fail` per interval after (2, 3, …), no row write started after the task observed cancellation (seeded sim with `SimGuestListenerReports` and `SimClock`) |
| `ServiceAllocFact.max_attempts` -> `startup_attempts_required` and the stop gate at `service_lifecycle.rs:1199` (D14 item 3, P1) | EXTEND (rename one in-memory State field; its only reader is the gate; hydration sets it from the descriptor's mechanic) | pure-function (return-only) | Universe: the `Option<Action>` the gate returns. Assert (S-54): for `Tcp` and `Http` the gate is today's `attempts >= max_attempts`; for `GuestListeners` it holds at `attempts >= 1` with the elapsed and no-pass conditions; proptest over mechanic x streak x elapsed x pass; mutation target |
| Beacon line reading in `VmDriver` (D13 item 3) | EXTEND (behaviour: classify by kind, never block the stop or command path) | bounded-change | Mutation set: the board and the exit report of one allocation. Assert: `Listen` never occupies the exit report; garbage and post-`Exit` lines change nothing (scripted lines through the reader's existing beacon seam) |
| New components, ports, variants, errors | D7: one variant (`StoppedBy::StartupProbe`). D8: one `View` field. D9: one `pub` method. D10: one const, two pure functions, one wire field and one wire struct. D11-E1: one public in-process `ServiceStreamError` with two variants. D17: one `pub` runtime method, one `pub(crate)` method, one `pub` guard type (one accessor removed). D13 to D16: see § Interface changes (D13 to D16) and the three rows above. D1 to D12 and D17 add no new component, crate, port or `Action` | n/a | D6 adds one in-memory State field and no port; D7 adds one in-memory State field and one enum variant; D14-E2 adds three typed input-error variants. |

Effect isolation: the hook is a pure-intent call through `Weak<dyn Driver>`; no new adapter or external dependency, so no `probe()` is specified.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

### State-ownership matrix

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| `AllocStatusRow` `Failed` / `Terminated` from a natural exit | Exit observer | The guest process ended and how it was classified | `ExitEvent` | Probe supervision release (the hook does not wait for the row) |
| `AllocStatusRow` `Terminated` from stop; `FinalizeFailed` terminal rows | Action shim | The shim's cleanup protocol completed | The stop/finalize cleanup effects | Restart |
| Probe supervisor for an allocation id | `ProbeRunner`, via the `Driver` hooks | Probe tasks run iff the allocation is registered and not ended | `on_alloc_running` (startup, readiness), `on_alloc_terminal`, `on_alloc_stable` (retires startup, starts liveness: D9) | `Running`, `Stable`, readiness, backend membership, restart budget |
| Start of liveness probing (D9) | `ProbeRunner`, from `on_alloc_stable` | No liveness row exists before the allocation's `Stable` row is committed | The committed `Stable` row (the shim's hook call after the row write) | `Running`, `Stable`, readiness, startup supervision, the restart decision |
| `ExitEvent` emission | `VmDriver` exit watcher | The guest ended and the Running-confirmed gate released | The Running-confirmed gate; originating-session claim (ADR-0100) | Row content |
| Restart decision and budget | `WorkloadLifecycle` (single authority, ADR-0087) | One budget across crash and liveness | Observed rows | Probe supervision |
| Readiness, liveness detection | `ServiceLifecycle` | Backend `healthy`; liveness stop trigger | Probe rows of `Running` allocations only (`service_lifecycle.rs:1011`, `:1055`); liveness rows exist only after `Stable` (D9) | Restart |
| Backend eligibility (`Backend.healthy`) of a startup-probed Service allocation (D12; supersedes the ADR-0096 per-allocation veto) | `ServiceLifecycle` | The allocation's own startup has passed (its `Stable` is committed) and the readiness rule holds | The allocation's committed `Stable` claim (ADR-0265), the readiness rule | `Running`, `Stable`, the startup stop, the restart decision, liveness, the readiness probe schedule, an opt-out Service |
| Wake of the convergence loop (D17) | `ReconcilerRuntime` (wake), convergence loop (drain) | A submitted evaluation is considered at once; a cadence tick at least every period. Gates no state | Submits (stored permit), cadence timer, an evaluation's completion | Any lifecycle state, eligibility (`NotBefore` still decides admission), per-target serialization, drain order |
| Ending of a `Failed` allocation: restart or give-up, and the give-up terminal claim (D5, D6) | `WorkloadLifecycle` (`workload_lifecycle.rs:1106-1191`) | The crash was observed and either a replacement is placed or the budget is spent | The observed `Failed` / `Terminated` row, the restart budget | Probe-derived verdicts from `ServiceLifecycle` |
| Cause class of the give-up terminal (`LivenessProbeFailed` / `StartupProbeFailed` / `EarlyExit` / `BackoffExhausted`) (D6, D7) | `WorkloadLifecycle` | Which kind of attempt the budget was spent on, read from the final attempt's row | The final row's state and typed reason, the declared startup probes, the allocation's startup probe rows | Whether, when or how often a restart is placed; the budget count |
| Startup-probe failure stop (D5 guard, D7, D8) | `ServiceLifecycle` (detector only) | A live allocation's startup probes exhausted attempts and deadline with no pass; a stop with `Stopped { StartupProbe }` was ordered, re-ordered once per new startup row until it lands or a `Pass` contradicts it. No failure claim | Producer streak, deadline, no pass, state not terminal, a startup row newer than the last acted one | `Failed` and `Terminated` rows; the restart decision; the stream's termination |
| `ServiceFailed { StartupProbeFailed }` terminal (D7) | `WorkloadLifecycle` | The budget is spent and the final attempt was stopped for a startup-probe failure | The final row's `Stopped { StartupProbe }` disposition, the last startup failure reason | `ServiceLifecycle`; any live allocation; the restart decision |
| `ServiceFailed { EarlyExit }` terminal (D6) | `WorkloadLifecycle` | The final restart attempt crashed before any declared startup probe passed, and the budget is spent | As the cause-class row | `ServiceLifecycle`; any live or `Stable` allocation; the restart decision |
| Service deploy stream termination (D7, D9, D10) | Service submit stream handler (`streaming.rs`) | The deploy reached an end no later attempt can contradict (`Stable`, a give-up terminal, an operator stop); or the stream stops waiting at its progress deadline without claiming failure | The projected lifecycle terminal; the progress deadline (startup deadline plus margin, at least the floor) | `Stopped { StartupProbe }` and `Stopped { LivenessProbe }` (restartable stops); restarts in progress; the restart authority's verdict |
| Progress-deadline timeout (D10) | Service submit stream handler | "Not `Stable` within the deadline; the restart budget may not be spent". Not a workload verdict; written nowhere | Stream open time, the derived deadline | Any persisted state, the restart decision, `Stable`, the give-up terminal |
| Recorded verdict reported at stream open (D11) | Service submit stream handler (a reader) | The verdict the workload's current attempt already carries. Authors nothing | The current attempt's row terminal, projected by the live projection | The restart decision; `Stable` announcement and dedup; any superseded attempt's terminal; the deadline's meaning |

### Gate G-1 — an exit event ends the allocation's probe supervision

- **Existing evidence:** ADR-0054 § 2, ADR-0100; `vm_driver.rs:2237-2330` → `exit_observer.rs:199-306`.
- **Owner:** the exit observer, for the driver it was spawned for.
- **Promise:** after the observer consumed the `ExitEvent`, the driver holds no probe supervisor for that allocation id.
- **Affected state:** the probe supervisor entry for `event.alloc` only.
- **Failure projection:** weak upgrade `None` (driver dropped) -> no-op; duplicate `ExitEvent` -> idempotent; row-write failure or retry exhaustion -> hook still ran (it runs first); shutdown cancellation -> loop breaks, probe tasks are process-local.
- **Explicitly unaffected:** row content and LWW counter; `Running`, `Stable`, readiness and membership of other allocations; the restart budget; the Running-confirmed gate; `release_supervision`.
- **Ordering:** after `ExitEvent` consumption, before the awaited row write and its retry budget (consumes none); `release_supervision` stays after. No new deadline.
- **Counterexample:** gating the hook on a successful row write leaves `NoWrite` or retry-exhausted allocations probing forever.
- **Evidence lane:** pure/unit (observer with a recording driver); seeded `overdrive-sim` invariant across exit, write failure and restart.

### Gate G-2 — probe supervision is registered before exit events can be emitted

- **Existing evidence:** ADR-0083; `mod.rs:2605-2687`, `:3201-3202` → `vm_driver.rs:1965-2025`.
- **Owner:** the action shim (owns both calls and their order).
- **Promise:** when the Running-confirmed gate releases, the allocation's supervisor is registered.
- **Affected state:** only the relative order of two existing calls; `ExitEvent` emission now follows `on_alloc_running`.
- **Failure projection:** shim error before `on_alloc_running` (mTLS install, guest activation) -> neither call runs, nothing registered, nothing to stop.
- **Explicitly unaffected:** the `Running` row write, mTLS interception, Exec release, `Stable`, readiness.
- **Ordering:** after `obs.write(Running)` and mTLS/guest-activation success; before the gate. Probe tasks tick-then-sleep (`probe_runner/mod.rs:274-282`), so none dials early. No timeout consumed.
- **Counterexample:** the current order lets an exit consumed in the window run the hook against an unregistered id, then permits registration: a permanent orphan.
- **Evidence lane:** Quint model plus a seeded sim invariant.

### Gate G-3 — a startup-probe failure action needs a non-terminal allocation (D5; the action it gates is changed by D7)

- **Existing evidence:** ADR-0055, ADR-0087; `service_lifecycle.rs:603-620`, `:854-914`, `:1168`.
- **Owner:** `ServiceLifecycle`, for a live allocation.
- **Promise:** a live allocation's startup probes exhausted attempts and deadline without a pass.
- **Affected state:** the action `ServiceLifecycle` emits for that allocation only: a stop with `Stopped { StartupProbe }` under D7 (G-5).
- **Failure projection:** terminal row -> no action (typed absence); duplicate evaluation -> deduplicated; late probe rows for a dead allocation -> not read (D2, Q3).
- **Explicitly unaffected:** `Stable`; liveness termination; backend membership (G-9); the restart budget; the exit observer's row.
- **Ordering:** none new; a pure predicate on hydrated state, no timeout.
- **Counterexample:** gating on "a `Failed` row exists" suppresses the verdict for a `Running` allocation that is alive and failing its probes.
- **Evidence lane:** pure/unit on the reconciler; seeded `overdrive-sim` invariant for the `WorkloadLifecycle` interaction.

### Gate G-4 — a pre-startup crash loop is named by the restart authority at give-up (D6)

- **Existing evidence:** ADR-0087, ADR-0055, ADR-0105; `workload_lifecycle.rs:1106-1252`, `streaming.rs:840-844`.
- **Owner:** `WorkloadLifecycle`.
- **Promise:** when the budget is spent on a crash loop, the terminal names an early exit exactly when the final attempt reached `Running`, crashed, and never passed a declared startup probe.
- **Affected state:** which of `EarlyExit`, `LivenessProbeFailed`, `BackoffExhausted` the give-up terminal carries.
- **Failure projection:** unreadable startup rows -> `HydrateError::ObservationRead`, tick skipped, no terminal on a guess; other reason, `Terminated`, Job, opt-out, recorded `Pass` -> existing `BackoffExhausted` (or `LivenessProbeFailed`); duplicate -> idempotency guard (`:1143-1151`); cancelled/disconnected -> re-derived next tick.
- **Explicitly unaffected:** whether a restart is emitted, backoff, ceiling, budget count, reclamation and generation-replacement exemptions; `Stable`; readiness and membership; liveness termination; the live `StartupProbeFailed` verdict; the exit observer's row; `ServiceLifecycle`'s `terminal_announced` bookkeeping for a `Failed` or `Terminated` fact (DG-12), which closes the allocation's mid-startup window and decides no action or terminal.
- **Ordering:** after the ceiling check, before `FinalizeFailed`; no timeout; one terminal write per allocation.
- **Counterexample:** classifying at the first crash makes the classification decide the restart (two-authors defect).
- **Evidence lane:** pure/unit and property tests on `reconcile`; seeded `overdrive-sim` crash loop through terminal and stream event.

### Gate G-5 — a startup-probe failure stops the allocation; the restart authority names it at give-up (D7, D8)

- **Existing evidence:** ADR-0087 D2 to D4, ADR-0257, ADR-0037 §4; `service_lifecycle.rs:612` → `action_shim/mod.rs:3234` → `workload_lifecycle.rs:1106`, `:1187`.
- **Owner:** `ServiceLifecycle` for the stop; `WorkloadLifecycle` for the give-up. The stop makes no failure claim; the claim is made once, at give-up.
- **Promise:** the stop: "this instance failed its startup probes and was stopped". The give-up: "the budget is spent and the final attempt failed its startup probes".
- **Affected state:** the live allocation's row (to `Terminated` with the disposition) and, at the ceiling, which give-up terminal the final row carries.
- **Failure projection:** stop dispatch error -> `ShimError`, row stays `Running`, D8 re-emits on the next newer startup row unless a `Pass` ends the retry; terminal row write error -> same case (ADR-0266 fires the hook after the write); unreadable startup rows at ceiling -> `HydrateError::ObservationRead`, no terminal; other disposition, `Failed`, Job -> that class's terminal; duplicate -> `startup_last_acted_run` / extended idempotency guard; crash racing the stop -> E-2.
- **Explicitly unaffected:** whether, when or how often a restart is placed; backoff, ceiling, budget count; reclamation, generation-replacement and intentional-stop exemptions; `Stable` and the opt-out path; readiness and membership; liveness detection, stop and give-up; the exit observer's rows; D5, D6.
- **Ordering:** after the three gates and the D5 guard; the stop precedes the restart; the give-up follows the ceiling check. The stop consumes the startup deadline once per attempt, so give-up arrives after about `(RESTART_BACKOFF_CEILING + 1)` deadlines (§ D7; deliberate).
- **Counterexample:** letting the stop's lifecycle event reach the stream as a terminal ends `overdrive deploy` "stopped", exit 0, at the first startup failure.
- **Evidence lane:** pure/unit and property tests on both `reconcile` bodies and the stream projection; seeded `overdrive-sim` never-ready guest through stop, restarts, give-up and stream event.

### Gate G-6 — liveness supervision begins at `Stable` (D9)

- **Existing evidence:** ADR-0054 § 2, ADR-0080 § D4, ADR-0058; `action_shim/mod.rs:2147-2150` → `vm_driver.rs:2065`.
- **Owner:** `ProbeRunner`, driven by `Driver::on_alloc_stable`.
- **Promise:** no liveness probe row exists for an allocation before its `Stable` row was committed; the first liveness row lands one liveness interval after, with its streak at 1.
- **Affected state:** the allocation's liveness tasks only.
- **Failure projection:** unknown, ended or already-started allocation -> no-op; failed `Stable` write -> no hook, ADR-0265 re-announces; control-plane restart -> supervisors lost, boot reclamation makes the VM terminal, successor starts liveness at its own `Stable`; ended between write and hook -> nothing starts.
- **Explicitly unaffected:** `Running`, `Stable`, the startup role's retirement, readiness probing, the restart decision and budget, the liveness stop rule and give-up, D1/D3/D4.
- **Ordering:** `Stable` row write, `stop_role(Startup)`, `start_liveness`; the latter two serialised by the supervisor lock. Probes tick-then-sleep. No timeout consumed.
- **Counterexample:** gating in `ServiceLifecycle` on a Stable marker leaves the streak accumulated during startup, so the first tick after `Stable` kills a healthy instance.
- **Evidence lane:** pure/unit on the runner with `SimClock`; seeded `overdrive-sim` slow-starting Service with an aggressive liveness probe reaches `Stable` and is not stopped.

### Gate G-7 — the Service deploy stream stops waiting at a derived progress deadline (D10)

- **Existing evidence:** ADR-0059 Q4, ADR-0259; `handlers.rs:609-635`, `streaming.rs:906`, `:988`, `:1013`.
- **Owner:** the Service submit stream handler.
- **Promise:** if no stream terminal arrives within `progress_deadline`, the stream ends `Failed { Timeout }`, exit 1, with the latest attempt-ending observation as labelled detail; it claims no workload failure and writes nothing.
- **Affected state:** the lines the stream yields. No row, `View` or intent.
- **Failure projection:** no attempt-ending event -> `last_attempt: None`; broadcast closed -> `StreamInterrupted`; lagged -> waits for next event or deadline; client disconnect -> dropped, nothing persists.
- **Explicitly unaffected:** `Stable`, the restart budget and give-up terminals, Job streams (keep `streaming_cap`), the stores, the rows `workload describe` reads.
- **Ordering:** armed after `Accepted` from the injected `Clock`, not extended. The deadline is a function of the startup probes and the floor and exceeds the startup window under DL-6; E-4 states where it does not.
- **Counterexample:** a fixed 90 s cap with a startup deadline above 60 s reports a healthy slow-starting Service `Timeout` before its startup window elapses.
- **Evidence lane:** pure/unit on `progress_deadline`; Tier 1 under `SimClock` (never-ready Service, short and default deadline, `Unchanged` redeploy); CLI render tests.

### Gate G-9 — a startup-probed allocation is a backend only after its `Stable` is committed (D12)

- **Existing evidence:** ADR-0096 (terminal veto superseded; amended by ADR-0267), ADR-0101, ADR-0265; `service_lifecycle.rs:1040-1155 @ 04052ad75` → `mtls_resolve_adapter.rs:491-539`.
- **Owner:** `ServiceLifecycle`.
- **Promise:** for a Service declaring a startup probe, a backend row never marks an allocation `healthy` unless its own status row carries a committed `Stable`; the readiness rule only narrows that.
- **Affected state:** the `healthy` bit of the allocation's entry in the complete `ServiceBackendRow`.
- **Failure projection:** `Stable` uncommitted or write failed -> `healthy: false` (B1 re-announces); row unreadable -> `HydrateError`, tick skipped; malformed/duplicate/cancelled/disconnected -> re-derived next evaluation, nothing stamped.
- **Explicitly unaffected:** `Running` and `Stable`, the startup stop and `Stable` decisions, the restart decision and budget, the liveness stop, the readiness probe schedule, an opt-out Service, the deploy stream. (No per-allocation terminal veto exists, R2-1: a startup-failed allocation never has a committed `Stable`.)
- **Ordering:** `Stable` row write -> hydration -> eligibility; `Stable` gates eligibility, never the reverse. No timeout consumed.
- **Counterexample:** keying on the latest startup row makes a `Stable` allocation unhealthy when a `Fail` lands between the `Pass` and the startup role's retirement; keying on a view marker routes to an allocation whose `Stable` write was lost (R2-2).
- **Evidence lane:** pure/unit and proptest on the predicate (S-46); seeded `overdrive-sim` through `ServiceLifecycle`, the shim and the resolver's `BackendIndex` (S-47).

### Gate G-8 — the Service deploy stream reports a recorded verdict before it waits (D11)

- **Existing evidence:** ADR-0056, ADR-0059 Q4, ADR-0037 §4; `handlers.rs:518-546`, `streaming.rs:988`, `:1010-1087`, `action_shim/mod.rs:2147-2152`.
- **Owner:** the Service submit stream handler.
- **Promise:** if, after subscribing, the current attempt carries a terminal the live path projects to a stream terminal, the stream ends with exactly that terminal; otherwise it waits, bounded by G-7.
- **Affected state:** the lines the stream yields. No row, `View`, intent or event.
- **Failure projection:** read error at open or relist -> typed stream error, never `Timeout`; no rows or no current attempt -> wait; terminal absent or projecting `None` (incl. restartable stops) -> wait; same terminal by read and channel -> first ends the stream; disconnect -> dropped; `Closed` bus -> `StreamInterrupted`; timeout -> G-7.
- **Explicitly unaffected:** `Stable` announcement and dedup; restart decision and budget; give-up terminals' authorship; the Job stream; readiness and liveness roles; D10's deadline value; the CLI; every persisted shape.
- **Ordering:** subscribe, read, (if no verdict) arm the deadline from the injected `Clock`, wait. The read consumes none of the deadline. A row committed before the subscription is visible to the read; one after has its event in the channel.
- **Counterexample:** reading before subscribing loses a terminal committed in between; reporting any row's terminal rather than the current attempt's reports a superseded give-up.
- **Evidence lane:** pure/unit on projection equality; Tier 1 under `SimClock` through the submit handler with real shim and reconcilers; seeded schedule variation on subscribe/read/commit/publish order; Quint model.

### Required boundary scenarios

1. **Gate available:** a guest exits; the supervisor is gone and the row is written (G-1). Normal start: supervisor registered before the gate (G-2).
2. **Unavailable / timed out:** the observation write fails and retries exhaust: supervisor still gone, degraded `LifecycleEvent` still sent. Weak upgrade fails: no-op.
3. **Unrelated state:** a sibling's supervisor, a successor's supervisor, and a `Running` allocation's readiness are unchanged.
4. **Late success:** (a) an in-flight probe completing after the hook writes at most one row for the dead id, which no reader acts on. (b) A late `on_alloc_running` after an exit is prevented by G-2.
5. **Disconnect / reconnect / replay:** the exit event is consumed once; shutdown drops unconsumed events; a control-plane restart loses probe tasks. Nothing is replayed.
6. **Feature disabled:** a driver with no `ProbeRunner` or a job with no descriptors takes the default no-op.

Items 7 to 12 apply the same six cases to each later gate:

7. **G-3 (D5):** S-10 (crash after streak exhausted: typed absence; late `Pass` changes nothing), S-21 (state x gates proptest), S-8 (`Stable`, liveness, membership as before).
8. **G-4 (D6):** S-15 (classification grid; `Pass` before ceiling selects `BackoffExhausted`; written terminal never flips; Job, opt-out, other reasons keep theirs), S-16 (composed crash loop; passing successor ends `Stable`; restart timing unchanged; restart before ceiling re-derives the same terminal), S-17 (unreadable rows: no terminal).
9. **G-5 (D7, D8):** S-21, S-22 (never-ready loop: stream open across every stop, successor `Stable` never after `Failed`, Job and opt-out never get the disposition), S-23 (ceiling grid; operator and system-gc exemptions), S-17, S-26 (stream projection), S-29 and S-30 (retry once per new startup row; late `Pass` ends it; restart re-reads `startup_last_acted_run`; B2 failed stop-row write retried), S-28 (E-2).
10. **G-6 (D9):** S-31 (runner contract incl. restart between `Stable` and first liveness row), S-32 (slow-starting Service with failing liveness not stopped after `Stable`; opt-out starts liveness at its `Stable`), S-43 (B1 lost `Stable` write).
11. **G-7 (D10):** S-34 (derivation), S-35 (composed stream: slow start inside 120 s ends `Stable`; never-ready ends `Timeout` with `last_attempt`; none gives `None`; in-deadline give-up unchanged; `biased` select takes event arriving with the deadline; Job byte-identical), S-36 (CLI envelope).
12. **G-8 (D11):** S-39 (redeploy cases a to f), S-40 (ordering, `Lagged`, superseded attempt), S-41 (snapshot equals live line), S-42 (failed read).
13. **G-9 (D12):** (1) committed `Stable` makes it eligible (S-46, S-47 a). (2) `Stable` uncommitted or write failed: `healthy: false` until B1 re-announces (S-43). (3) opt-out Service stays `healthy` at `Running`; `Stable`, startup stop, liveness, restart unchanged; unrelated `terminal_announced` changes nothing (S-46, S-47 e). (4) startup `Fail` after `Stable` does not unhealthy it (S-47 e); `Pass` after a stop that did not land makes it eligible only at its `Stable` commit (S-30). (5) restart re-derives eligibility from the row, no view marker (S-47 f). (6) opt-out path byte-identical (S-46).
14. **G-10 (D13 to D15):** see § Decisions D13 to D16, Gate G-10.
15. **D17:** no gate (the matrix says "gates no state"); the six cases do not apply. S-48 carries its ordering and fairness invariants.

## Wave: DESIGN / [REF] Formal protocol model

**Required: yes.** The probe supervisor set is shared state with three owners and an ordering
edge (register before gate); two reconcilers can author one allocation's ending; row writes are
crash-sensitive; a stream timer races the lifecycle terminals; the stream's subscribe, read, commit
and publish steps are order-sensitive; the guest's beacon writer is a single ordered writer whose
`LISTEN`-before-`EXIT` order the host reader depends on. The specification is in
`specs/quint/service-allocation-readiness-and-ending/`, written and model-checked by the
`quint-modeler` agent for D1, D3, D5 to D11, D13 and D14 (D2 and D4 are contract and non-change
decisions covered by named runs and assumption HO-3; D15 and D16 reuse the D14 model and add
none). Its `README.md`, `checks.toml` and `evidence/summary.md` are the authoritative record of
modules, invariants, named runs, hazards, assumptions, counts and results; this section does not
restate them.

**D17 needs no Quint model (decision S6, § D17).** The one ordering hazard is a lost wake; the
primitive excludes it by documented semantics, a lost wake costs only the unchanged tick latency,
trace replay has nothing to project, and a hazard would model a rejected alternative. S-48 and
mutation carry it. If overruled: one module (pending count, permit bit, loop phase, cadence-due
bit), runs `submit_during_drain_is_not_lost`, `burst_coalesces_to_one_drain`,
`cadence_fires_without_submits`, hazard `wait_without_permit` (the user's call; not added,
`.claude/rules/design.md` § "No unrequested scope").

**R2-1 needs no model change.** **R2-2 changes the model by removal only** (the stamp variable and
its action go; the B1 hazard is rebuilt on the existing write-outcome variable).

**D12 needs no model change.** Backend eligibility is a pure predicate over one hydrated fact
(`stable_committed`) and the readiness rule, with no state, interleaving or second owner; S-46
enumerates its truth table. The one protocol-shaped premise, that a committed `Stable` claim is
never dropped while the allocation is `Running`, is B1's `committed` flag in `liveness_gate.qnt`,
discharged by V31 and S-43.

**Layout.** One module per protocol: `hook_ownership` (D1, D3; D2 and D4 by runs),
`restart_authority` (D5 to D8, and D14's producer rule through the workload flag `gl`),
`liveness_gate` (D9), `progress_deadline` (D10), `stream_snapshot` (D11), `guest_report` (D13
items 1, 3 and 10: the model check of the beacon report channel) and `guest_report_host`
(the host-input projection of the same file, the conformance target for the report protocol;
detail in § Formal protocol model — D13 to D16). Each declares `step` and named runs; each
instance is `*_ok` (checked), `*_conf` (conformance, named runs only) or a hazard under
`hazard/`. One hazard per decision. Counts of modules, checks and runs are the README's and
`checks.toml`'s.

**Scenario to named run.** `specs/quint/service-allocation-readiness-and-ending/README.md` § "Named runs" maps each
critical scenario to the run DISTILL's `#[quint_test]` pins; that table is the authority. The
scenarios, by decision: D1 (crash without restart, crash then fresh-id restart, write exhaustion),
D2 (double fire), D3 (a start registers probe supervision before it releases the gate, replayed by
every crash run), D4 (a stop emits no `ExitEvent`; finalize on a live VM), D5 (crash after the
streak is exhausted; a live exhausted allocation is stopped), D6 (pre-startup crash loop, successor
passes, post-pass crash loop, opt-out, non-crash failed row, liveness-kill precedence), D7
(never-passes loop, successor passes, last cause wins in both orders, the latest startup row gates
the stop), D8 (failed stop retried, late pass reaches Stable), D9 (liveness failing during startup,
starts at Stable, Stable after the end, opt-out), D10 (slow start, never ready, give-up inside the
deadline, redeploy opens a new clock), D11 (redeploy of a Stable service, commit between subscribe
and read, restartable stop, superseded attempt, lag then relist), D13 (a listen produced before
exit is on the wire before `exit`; a dropped declared event leaves the set partial; and, on the
host-input projection, all declared reported, a garbage line dropped, EOF with a partial set, a
`listen` after `exit` ignored, with the exit report read from the row after `VmmExit`), D14 (the first
`Fail` at the deadline stops the instance and a second re-emits the stop; an explicit probe's
`Fail` after the deadline keeps its counted streak).

- **Pinned conformance projection** (for DISTILL; no new API):

  | Model variable | Owner accessor (existing) |
  |---|---|
  | supervised-allocation count | `ProbeRunner::active_alloc_count()` (`probe_runner/mod.rs:414`) |
  | per-role liveness of an allocation | `ProbeRunner::is_role_live(alloc, role)` (`:407`) |
  | terminal row state | `ObservationStore::alloc_status_row(alloc)` |
  | `ServiceLifecycle` startup stop (D5, D7) | `Action::StopAllocation { terminal: Some(Stopped { by: StartupProbe }) }` in the `Vec<Action>` returned by `ServiceLifecycleReconciler::reconcile` (pure driving port) |
  | restart or give-up decision (D5, D6, D7) | `Action::RestartAllocation` or `Action::FinalizeFailed { terminal: Some(BackoffExhausted \| ServiceFailed { EarlyExit } \| ServiceFailed { StartupProbeFailed } \| ServiceFailed { LivenessProbeFailed }) }` in the `Vec<Action>` returned by `WorkloadLifecycle::reconcile` |
  | deploy stream open or closed (D7) | `service_event_from_terminal` return for each row write's terminal (`Some` closes, `None` keeps open); the Service stream handler's first yielded terminal in the composed run |
  | no terminal action over a terminal row by `ServiceLifecycle` (D6) | no `Action::FinalizeFailed` in the `Vec<Action>` returned by `ServiceLifecycleReconciler::reconcile` for a `Failed` or `Terminated` fact |
  | predecessor attempt stays observable (D6) | `ObservationStore::alloc_status_row(predecessor)` after the successor row is `Running` |
  | row state, streak, deadline, pass (D5 inputs) | hydrated `ServiceAllocFact` / `AllocStatusRow` the driver supplies; inputs, not projected |
  | declared startup probe, startup probe row, final reason class (D6 inputs) | `WorkloadLifecycleState.probe_descriptors`, `.current_startup_probe` and the `AllocStatusRow` reason and terminal supplied through `WorkloadLifecycle::hydrate_actual` on `SimObservationStore`; inputs, not projected |
  | VM slot of a stopped id is not live (D7) | `Driver::stop` outcome through the shim `StopAllocation` driving port, observed as the recorded order of the `stop` call before the successor's `start` |
  | recorded acted run, stop emitted for a row (D8) | `ServiceLifecycleView.startup_last_acted_run` (pub) on the `View` returned by `ServiceLifecycleReconciler::reconcile`, and the startup `StopAllocation` in the returned `Vec<Action>` |
  | latest startup row, stop outcome (D8 inputs) | the `ProbeResultRow` and shim `StopAllocation` result supplied through `SimObservationStore` and a failing `SimDriver::stop`; inputs, not projected |
  | liveness role live (D9) | `ProbeRunner::is_role_live(alloc, ProbeRole::Liveness)` through `Driver::on_alloc_running`, `on_alloc_stable`, `on_alloc_terminal` |
  | liveness stop enabled (D9) | `Action::StopAllocation { terminal: Some(Stopped { by: LivenessProbe }) }` in the `Vec<Action>` returned by `ServiceLifecycleReconciler::reconcile` |
  | stream closed and kind, deadline (D10) | first terminal line yielded by `build_service_stream` under `SimClock` (`Failed { Timeout }`, `Stable`, a give-up `Failed`), and the value returned by `progress_deadline` |
  | last attempt observation (D10) | the `last_attempt` field of that `Failed` line |
  | recorded verdict read, subscribe/read/commit/publish order (D11) | first terminal line yielded by `build_service_stream` under `SimClock`, with `AllocStatusRow`s written through `SimObservationStore` and `LifecycleEvent`s sent on the bus at scripted points relative to the stream's first poll (the `Accepted` line marks the subscribe point); the current attempt is the one `current_alloc` selects. Commit-before-publish is the shim's own order, proven by S-41 |
  | per-id supervisor presence, gate, queued event (`hook_ownership`) | model-only ghosts. Supervisor presence is observed through `active_alloc_count()` and `is_role_live`; gate and queued event only as the recorded call order of `on_alloc_running` and `release_for_exit_emission` (S-4) |
  | `stable` and `ann` (`restart_authority`, D8) | `stable`: `Action::FinalizeFailed { terminal: Some(Stable { .. }) }` in the `Vec<Action>` returned by `ServiceLifecycleReconciler::reconcile`, and the committed claim on the row (`ServiceAllocFact.stable_committed`, an input written through `SimObservationStore`). `ann`: the pub `terminal_announced` set of the returned `ServiceLifecycleView` (`service_lifecycle.rs:330`) |
  | `committed` (`liveness_gate`, B1) | the committed `Stable` claim read through `ObservationStore::alloc_status_row(alloc)` (`terminal` is `Stable`); `swr` (the `Stable` write outcome) is an input scripted on `SimObservationStore`. **DG-3:** compared only while the row is `Running` (the spec's `rw`). After `Crash` the exit observer's `Failed` row replaces the row and `committed` is model-only |
| closing state (`stream_snapshot`, D11) | **DG-4:** the closing `AllocStatusRow` read through the existing `ObservationStore::alloc_status_row(alloc)` (its `state` and `terminal`). The line `build_service_stream` yields is an input and an oracle for the step (S-39 to S-42), not a projected variable |
| `terminal_announced` membership of a `Failed` or `Terminated` fact (D6, DG-12) | the pub `terminal_announced` set of the `ServiceLifecycleView` returned by `ServiceLifecycleReconciler::reconcile`, and `ServiceLifecycleView::has_alloc_mid_startup_window()` on that view (false once the allocation is a member) |
  | `used` (restart budget consumed) | pub `WorkloadLifecycleView.restart_counts` entry for the current attempt in the `View` returned by `WorkloadLifecycle::reconcile` (`workload_lifecycle.rs:2066`) |
  | `cur` (current attempt) | `overdrive_reconcilers::current_alloc` over the observation rows (`workload_lifecycle.rs:1422`, pub) |
  | `done` (give-up decided) | `Action::FinalizeFailed` carrying a give-up terminal in the `Vec<Action>` returned by `WorkloadLifecycle::reconcile` |
  | `claim` and `author` | `claim`: the `terminal` of the `FinalizeFailed` in a returned `Vec<Action>`. `author`: which reconciler's `reconcile` returned it (the driver calls the two separately), not a driver record |
  | `deployed`, ghost record `g`, `conf_step` (`restart_authority`) | model-only: `deployed` is "the intent exists" (an input written through the intent store), `g` states invariants only, `conf_step` is the conformance recorder; none is projected |
  | liveness role live, streak (`liveness_gate` `lv`) | role live: `ProbeRunner::is_role_live`. The retained-descriptors flag is model-only; the streak is an input (`same_status_runs` of the probe rows the recording prober writes), its effect read as the liveness `StopAllocation` |
  | `rset`, `exitrep` (`guest_report_host`, D13) | `rset`: `GuestListenerReports::reported(alloc)`; `exitrep`: three-valued, `state` and `reason` of `ObservationStore::alloc_status_row(alloc)` (the exit observer's row, R1-a). Exact pins, the action mapping including `VmmExit`, and the runs: § Formal protocol model — D13 to D16 |

  Actions map to: `Driver::on_alloc_running`, `Driver::on_alloc_terminal`,
  `Driver::release_for_exit_emission` through the shim `dispatch` driving port;
  `ExitEvent` injection through the receiver returned by `take_exit_receiver`
  into `exit_observer::spawn_with_runtime`. D6 actions map to
  `WorkloadLifecycle::hydrate_actual` followed by `WorkloadLifecycle::reconcile`
  and `ServiceLifecycleReconciler::reconcile` (the pure driving ports), with the
  shim `dispatch` of the returned `FinalizeFailed` for the terminal write. D7
  actions map to `ServiceLifecycleReconciler::reconcile` followed by the shim
  `dispatch` of the returned `StopAllocation`, then `WorkloadLifecycle::hydrate_actual`
  and `reconcile` for the restart and the give-up, and the stream projection
  through `service_event_from_terminal`. D8 actions map to
  `ServiceLifecycleReconciler::reconcile` fed successive hydrated facts, then the
  shim `dispatch` of the returned `StopAllocation` against a `SimDriver` whose
  `stop` is scripted to fail or succeed. D9 actions map to `Driver::on_alloc_running`,
  `on_alloc_stable` and `on_alloc_terminal` on a `VmDriver`-shaped driver holding a
  real `ProbeRunner` with a `SimClock` and recording probers. D10 actions map to
  `build_service_stream` driven by `LifecycleEvent`s on the bus and `SimClock`
  advance. A `Sim*` adapter is not an arbiter of any projected fact here.
- **Conformance abstractions (rulings DG-1, DG-2, DG-5, 2026-11-05).** Test-side facts; no production change.
  - **Restart ceiling (DG-1).** The conformance instance `ra_conf` sets `CEIL = 5`, the value
    of `RESTART_BACKOFF_CEILING` (`workload_lifecycle.rs:34`), so the model gives up where
    `WorkloadLifecycle` gives up. The give-up named runs are written for five restarts. The
    driver asserts that the spec's `CEIL` equals the const; that assertion is test code, not a
    seam. The exhaustive instance `ra_ok` is unchanged.
  - **Start outcome (DG-2).** `restart_authority` has no separate `pending` row and no `StartOk`
    or `StartRejected` action. A start-producing action (`Deploy`, `WLRestart`) takes the start
    outcome, `ok` or `rejected`, as a nondeterministic parameter. This is the shim's behaviour:
    it awaits `Driver::start` and writes `Running` on success and `Failed` on
    `DriverError::StartRejected` in the same tick (`action_shim/mod.rs:~2433-2459`), so no
    production step exists between request and outcome. The named run for a rejected start
    (`non_crash_failed_row_gives_up_backoff_exhausted`) is kept. The driver makes `Driver::start`
    reject through a runtime-settable start-failure flag on `SimDriver`; that flag is test
    support and DISTILL's to shape.
  - **`ann` (DG-12).** The model's `ann` is the `terminal_announced` set and includes every
    allocation observed `Failed` or `Terminated`, as D6 item 1 states; the conformance read is
    the `terminal_announced` row of the table above.
  - **Channel and lag (DG-5).** In `stream_snapshot` the channel is an ordered list. `Deliver`
    takes the oldest event, and is disabled while `lagged` is set (the stream relists before it
    polls the bus again). The model's `Lag` drops every unseen event and sets `lagged`; tokio's
    broadcast channel instead reports `Lagged` and resumes at the oldest retained event. The
    difference is harmless: the relist reads the store, and commit precedes publish (S-41), so
    the relist sees every row whose event was lost, and an event retained after the relist
    describes a row the relist has already read (one terminal line, S-40). In
    `progress_deadline`, `StableHappens` is disabled after `OperatorStop`.
- **Commands:** `cargo xtask metal run -- cargo xtask quint typecheck` and
  `cargo xtask metal run -- cargo xtask quint check --subsystem specs/quint/service-allocation-readiness-and-ending`
  (the modeler runs them; never Lima).

## Wave: DESIGN / [REF] Validation items

No item for D1 to D12 or D17 is contract-shaping and open; every row is discharged by source
reading (D17: the tokio semantics it rests on are cited in § D17). Numbers not listed are
unused. D13 to D16 carry their own contract-shaping items, V34 to V39, V41 and V42, in
§ Spike table (V42 held, 2026-11-04).

| # | Claim | Evidence | Status |
|---|---|---|---|
| V1 | `VmDriver::stop` leads to no `ExitEvent` (the shim stop hook is needed; no observer/shim double-fire for stops) | `vm_driver.rs:1793` (EndingInFlight), `:2304-2308` (`try_begin_ending` → `None` → return); `:2325` sets `intentional_stop: false` on the events it does emit | Settled |
| V2 | `stop_alloc` is idempotent and a no-op for unknown ids | `probe_runner/mod.rs:374-379` | Settled |
| V5 | `release_for_exit_emission` ends with the gate send, so the shim's next statement runs with no intervening await | `vm_driver.rs:2019-2024` | Settled |
| V6 | `on_alloc_running` has no dependency on the Exec release or the gate | `probe_runner/mod.rs:307-365` (spawns tick-then-sleep tasks; no dial in `start_alloc`) | Settled |
| V7 | (HO-4, for B2's order) Reclamation writes `Terminated` without hooks, which is correct; the one state B2 creates, a registered supervisor with no held VM slot, is inert for reclamation | After a failed row write the slot is vacant (`VmDriver::stop` moves the entry to `EndingInFlight`; the arm calls `release_supervision` on both paths, `action_shim/mod.rs:3363-3365`, `:3370-3372`), the supervisor is registered and the row is `Running`. `plan_reclamation` derives candidates only from host residue (`vm_reclamation.rs:155-196`), which `VmDriver::stop` removed, so no `ReclaimAllocation` or `DiscardStrandedArtifacts` is planned and nothing else writes the row | Settled by source; the failed-write state is retried by the next probe row (D8, S-30) |
| V9 | (D6) A crashed attempt's row and probe rows are not deleted or overwritten when its successor starts | `ObservationStore` has no delete or prune for `alloc_status` or `probe_result` rows (`observation_store.rs:1790-2314`); the successor is a different key (`mint_alloc_id`, `workload_lifecycle.rs:1220`), its row built with no prior (`action_shim/mod.rs:2982-2996`); `GET /v1/allocs` returns every row of the workload (`handlers.rs:1187-1193`) | Settled |
| V10 | (D6) The latest `Startup`/index-0 probe row says whether a startup probe passed for that allocation, readable by the ceiling decision after the crash | `Stable` is decided on the same row (`service_lifecycle.rs:538-558`); keyed `(alloc, role, idx)`, it survives allocation end (V9); `on_alloc_stable` retires the startup role after `Stable` (`action_shim/mod.rs:2150`). Bounded exposure: a `Pass`, a crash before `Stable`, then one in-flight `Fail` (Q3) leaves the row `Fail` and the give-up named `EarlyExit` instead of `BackoffExhausted`; this affects the terminal's name only, never the restart decision. S-15 pins the boundary | Settled, exposure stated |
| V12 | (D7) The shim echoes a stop's terminal onto the lifecycle event and the Service stream closes on any projected `Stopped`, so the new disposition must be dropped by the projection | `action_shim/mod.rs:508`; `streaming.rs:845-847`, `:1034-1051`; `overdrive-cli/src/commands/deploy.rs:795-817` (exit code 0) | Settled |
| V13 | (D7) A stopped startup-failed row is restartable and no exemption covers it | `is_intentionally_stopped` matches `Stopped { Operator \| SystemGc }` only (`workload_lifecycle.rs:1543-1558`); `is_restartable` (`:1656-1660`); `by_reclaims_platform` is exhaustive and gets `false` for the new arm (`transition_reason.rs:1027-1038`) | Settled |
| V14 | (D7) The give-up's `last_fail` is a total function of the current attempt's latest startup row | A late row (D2, Q3), possibly `Pass`, can follow the `Fail` that triggered the stop, and rows are never deleted (V9). The mapping is `startup_probe_failed_action`'s (`service_lifecycle.rs:1200-1203`): `Fail` gives its reason, any other value the empty string. No `unreachable!` | Settled by source |
| V15 | (D7, D10) Time to give-up versus the Service stream's progress deadline | `DEFAULT_STARTUP_DEADLINE` 60 s (`service_lifecycle.rs:359`); progress deadline `max(DEFAULT_STREAMING_CAP 90 s (lib.rs:558), startup deadline + 30 s)` = 90 s at the default; the first restart is immediate, later ones wait `backoff_for_attempt` (`workload_lifecycle.rs:1204-1210`); give-up about `(RESTART_BACKOFF_CEILING + 1) x deadline + backoffs`. Arithmetic on source constants | Settled |
| V16 | (D7) A fieldless tail variant on `StoppedBy` adds no archived layout size, so no envelope version is needed | ADR-0087 § Compliance reasoning (same shape as `LivenessProbe`); discharged by the new golden fixture (D7, format impact), whose RED would route back to DESIGN. Finding F-4: the `LivenessProbe` fixture ADR-0087 required is absent at HEAD (`crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs` carries `FIXTURE_V1` only) | Settled by rkyv layout rules; discharged in DISTILL by the fixture |
| V17 | (D8) A new startup probe row arrives every interval after the deadline while the guest is alive, so a retry keyed by `run_seq` is paced and not starved | `supervised_probe_loop` is bounded only by cancellation (`probe_runner/mod.rs:763-799`); the startup role is cancelled only by `Stable` or the terminal hook; `ServiceLifecycle` is woken by accepted `ProbeResult` rows (`service_lifecycle.rs:451-453`); the liveness retry rests on the same facts | Settled by source |
| V18 | (D8) A `Pass` cannot follow a stop whose `driver.stop` succeeded | A stopped guest cannot answer a dial (a failed dial is `Fail` or no-sample, ADR-0187); Q3. A cleanup error after a successful `driver.stop` leaves a dead guest and no `Pass` | Settled |
| V19 | (D9) `Stable` is committed before `on_alloc_stable` fires, and the hook fires on every `Stable` path including opt-out | `action_shim/mod.rs:2144-2153` (row write, hook, event); the opt-out `Stable` uses the same `FinalizeFailed(Stable)` action (`service_lifecycle.rs:527-531`). Does not establish that a `Stable` whose write failed is announced again; B1 (ADR-0265) supplies that | Settled by source for hook ordering; re-announcement is B1 (S-43) |
| V20 | (D9) `start_liveness` is atomic with `stop_alloc` | Both take the `supervisors` lock (`probe_runner/mod.rs:257`, `:375`); the spawn happens while the entry is held, or the entry is absent. The crafter keeps the spawn under the same lock hold (`rust.md` check-and-act) | Settled by source |
| V21 | (D10) The Service stream receives every lifecycle event of its workload, so `last_attempt` needs no extra read | `streaming.rs:1017-1031` (receives each `LifecycleEvent`, drops non-terminal ones after the `terminal` test); `LifecycleEvent` carries `to`, `reason`, `terminal`, `detail` (`action_shim/mod.rs:275-311`); a lagged subscriber misses events (G-7) | Settled by source |
| V22 | (D10) The client total request timeout is the only client-side bound on the streaming lane; a value below the server deadline hides the typed terminal | `http_client.rs:275-292` (120 s, written for this reason) | Settled by source |
| V23 | (D11) The shim commits a row before it publishes the event for it, on every terminal path, so subscribe-then-read loses nothing | `action_shim/mod.rs:2147-2152` (`Stable`), `:2187-2193` (genuine terminal), `:3384-3385` (`StopAllocation`: `write_alloc_lifecycle` then `emit_lifecycle_occurrence`); the event is built from the write's occurrence (`:480-519`) | Settled by source; discharged by S-41 |
| V24 | (D11) `Stable` is recorded on the `Running` row and not carried onto the row a crash writes, so the recorded verdict cannot report `Stable` for a crashed current attempt | The exit observer writes `terminal: None` on every row it authors (`exit_observer.rs:432-438`, `:519-524`; ADR-0037 §4); the shim's stop and finalize arms write their own `terminal` | Settled by source |
| V25 | (D11) `current_alloc` selects the workload's current attempt: the numeric-maximum attempt suffix, and every restart mints a higher one | `workload_lifecycle.rs:1402-1428`; `next_allocation_attempt` mints above every accepted row and reserved key (`:1388-1400`); the reconciler uses the same function (`:1029`, `:1106`) | Settled by source |
| V27 | (D1, assumption HO-5) The exit observer's `Weak<dyn Driver>` upgrade succeeds while the process runs, so D1's hook call is never skipped while supervisors exist | The observer holds a `Weak` (`exit_observer.rs:177-198`); the strong `Arc` is held by `AppState.drivers` (`lib.rs:252`, cloned at `lib.rs:4512-4526`). The `None` arm (`exit_observer.rs:262-266`, `:304`) is reached only after `AppState` releases the registry, at shutdown, where in-process supervisors end with the runtime | Settled by source; falsified by S-1, S-3 failing |
| V28 | (D7, D8, assumption RA-2) The shim's `StopAllocation` dispatch is awaited inside the `ServiceLifecycle` evaluation, so a stop's landed or failed outcome is a fact of the row (`Terminated` or still `Running`) before the next evaluation of the workload | `spawn_convergence_loop` awaits `run_convergence_tick` per admitted evaluation (`lib.rs:4981-4993`), which awaits `action_shim::dispatch*` (`reconciler_runtime.rs:1679-1742`); the arm awaits `driver.stop`, cleanup and `write_alloc_lifecycle` (`action_shim/mod.rs:3248-3356`); the target is blocked from admission until then (`lib.rs:4961`, `:4968`, `:5031`; `eval_broker.rs:144-149`). A dispatch error returns after the self-re-enqueue (`reconciler_runtime.rs:1761-1765`); the reconciler reads the row, not the error. **RA-2 also includes idempotence of the stop arm's cleanup effects on retry** (B2's and D8's retries rerun the arm): `Driver::stop` on an absent entry is `Err(NotFound)` (`vm_driver.rs:1806-1808`), tolerated (`action_shim/mod.rs:3249-3253`); `retire_guest_network_lease` acts only when `guest_pool.retire` returns true (`:1732-1745`); `MtlsInterceptLifecycle::stop_alloc` treats an unknown allocation as `Ok(())` (`mtls_intercept_worker.rs:2793-2815`), removes a completed prior stop (`:2830-2831`, `:2887`) and joins an in-flight or incomplete one (`:2829`, `:2833`); `teardown_guest_network` returns `Ok(())` when the pool has no plan left (`action_shim/mod.rs:1605-1607`) and re-runs `GuestNetworkProvisioner::teardown` (converges on absence, `guest_network.rs:134-142`) when the first pass failed inside it. The failed-write error arm leaves the `alloc_drivers` entry (`:3363-3366` returns before `:3383`), so the retry resolves the driver. No non-idempotent effect found | Settled by source; S-30 scripts a failing `SimDriver::stop` and row write |
| V29 | (D11) A projecting terminal event is published only for the then-current attempt of its workload | The three projecting publish sites run inside one evaluation's dispatch of the workload's target (`action_shim/mod.rs:2152`, `:2193`, `:3385`); evaluations of a target are serialised and the successor row is written only by a later `RestartAllocation` dispatch; observer events carry `terminal: None` (`exit_observer.rs:432-438`, `:519-524`). Trace in § Pinned readings, R-4 | Settled by source; discharged by S-40's added clause |
| V30 | (D10, assumption DL-6) The stream clock starts at open and the startup deadline at the `Running` row, so boot and placement sit between them; the margin equals the boot deadline | `VM_BOOT_DEADLINE` 30 s (`vm_driver.rs:63`; `:1598`, `:1675`); `started_at` set when the `Running` row is written (`action_shim/mod.rs:2459`, `:2976`); the stream arms its clock at open (`streaming.rs:1013`); `PROGRESS_DEADLINE_MARGIN` 30 s (D10 item 1) | Settled by source; scoped to DL-6, E-4 states the exposure |
| V31 | (B1; D12 reads it) The committed `Stable` claim on a `Running` row is never dropped by a later writer | `FinalizeFailed(Stable)` forwards the prior row's fields and keeps `Running` (`action_shim/mod.rs:2063-2064`, `:2083-2118`); the exit observer authors only `Failed` and `Terminated` rows with `terminal: None` (`exit_observer.rs:432-438`, `:519-525`); start and restart arms write the row before `Stable` exists. No later writer of a `Running` row found | Settled by source; S-43 asserts it |
| V32 | (B2) At HEAD, after a failed terminal row write in `StopAllocation` nothing converges the allocation | Arm order and error arm (`action_shim/mod.rs:3250-3272`, `:3356-3366`); `VmDriver::stop` emits no `ExitEvent` (V1); both stop retries key on a new probe row (D8, ADR-0191); `is_restartable` excludes a `Running` row (`workload_lifecycle.rs:1656-1660`); `plan_reclamation` iterates only allocations with host residue (`vm_reclamation.rs:155-196`), removed by `VmDriver::stop` (V7) | Settled by source; B2 closes it |
| V33 | (B3) The `Failed` reasons that can carry `started_at` and mean a guest crash are exactly `WorkloadCrashedImmediately` and `VmOutOfMemory` | `exit_observer.rs:597-619` (both written for `ExitKind::Crashed` with `prior.started_at`, `:535`); other `Failed` reasons carrying `started_at` are shim fail-closed rows over an already-`Running` row (`action_shim/mod.rs:660-751`, `:753-833`) and a `FinalizeFailed` row forwarding the prior reason; `Vm*` start failures and the netns provisioning failure carry none (`:628`, `:2435`, `:2943`) | Settled by source |

## Wave: DESIGN / [REF] Answers to the design questions

**Q1 — every terminal writer and its hook coverage.**

| Terminal row author | Site | Supervisor possible? | Hook after this design |
|---|---|---|---|
| Exit observer, natural exit (`Failed`, `Terminated`) | `exit_observer.rs:507-553` | Yes | **D1** (new) |
| Exit observer after intentional `Driver::stop` | n/a: `VmDriver` emits no event after `stop` (V1). A driver that does (`SimDriver` with `intentional_stop`, `overdrive-sim/src/adapters/driver.rs:306-332`) is covered by D1 | Yes | Shim `StopAllocation` hook; D1 also fires if an event arrives; double-fire is harmless |
| Shim `StopAllocation` (`Terminated`) | `mod.rs:3331-3355` | Yes | Existing hook `:3272` |
| Shim `FinalizeFailed` genuine terminal (`Failed`, `Terminated` for `Completed`) | `mod.rs:2083-2194` | Yes | Existing hook `:2176` |
| Shim `FinalizeFailed(Stable)` | `mod.rs:2144-2153` | Yes, but non-terminal | `on_alloc_stable`, unchanged |
| Shim start-failure `Failed` rows (`StartAllocation` reject, mTLS install fail-closed, guest activation fail-closed, restart-successor equivalents) | `mod.rs:628`, `:730`, `:815`, `:2435`, `:2943`, `:3069` | No: all precede `on_alloc_running` (`:2622-2686`) | None needed (G-2 keeps this true) |
| `VmReclamation` `Terminated` | `reclamation.rs:229-244` | No: needs a vacant slot (V7) | None needed |

Double fire occurs for a natural exit followed by `FinalizeFailed` on the same id
(budget exhaustion) and for a `SimDriver`-style event after a shim stop. Both are
harmless by contract: D2 makes idempotence and the unknown-id no-op postconditions
(V2).

**Q2 — ordering and failure.** The hook fires before the row write and its retries
(D1), so `RetryOutcome::Failed` and `NoWrite` still fire it. A weak-upgrade failure at
shutdown is a no-op: the driver and its runner are gone (`exit_observer.rs:262-265`).

**Q3 — late success and lost signal.** `supervised_probe_loop` awaits `probe_tick`
outside the cancellation race (`probe_runner/mod.rs:777-796`), so an attempt already
running when the hook fires may write one `ProbeResultRow`. Liveness detection requires
`state == Running` (`service_lifecycle.rs:1011`); backend membership and readiness
consider only `Running` allocations (`:1055`, `:1069`); a dead guest cannot produce a
`Pass` (ADR-0187). The only reader of probe rows for a `Failed` allocation is the
startup-failure branch (`:1192-1199`). Stopping probes at crash removes no signal a
reconciler needs. It does remove one behaviour: a crashed Service allocation with no
`Pass`, past its startup deadline, no longer has its failure streak grown by dead-guest
probing until `ServiceFailed { StartupProbeFailed }` is finalised over the crash. The
crash row is the cause and `WorkloadLifecycle` restarts under its single budget
(ADR-0087). This behaviour change is approved (BC-1, 2026-11-02) and is obligation S-10.
A streak that reached its limit before the crash is the edge closed by D5.

**Q4 — shim call sites.** `cleanup_restart_predecessor` is unchanged. The
`FinalizeFailed` and `StopAllocation` hooks both remain necessary (D4, V1); nothing
becomes redundant, because the shim never fired the hook for a natural exit.

**Q5 — lifecycle gates.** G-1 (observer hook) and G-2 (registration before gate
release); see § Lifecycle Gate Ownership. No state meaning changes.

**Q6 — Quint.** Required and written; see § Formal protocol model.

**Q7 — contract-shaping validation items.** None open (§ Validation items).

## Wave: DESIGN / [REF] Executable obligations for DISTILL

Evidence tiers follow `.claude/rules/testing.md`. All run in the default or
`integration-tests` lane; none spawns the `overdrive` binary; none needs a test seam
beyond a test-local recording driver. Scenario numbers not listed are unused.

| ID | Scenario | Fault stimulus | Oracle | Tier |
|---|---|---|---|---|
| S-1 | **#334** crash without restart: `Running` VM allocation with a registered supervisor; `ExitEvent` `Crashed` arrives; restart budget not due | `ExitEvent` through the observer's receiver | `active_alloc_count()` falls by one and `is_role_live` is false for every role within the observer's handling, before the restart backoff elapses; row is `Failed` | Tier 1 composed (observer + real `ProbeRunner` + `SimObservationStore`), seeded |
| S-2 | **#318** shim restart path: `Failed` predecessor, fresh successor id, `RestartAllocation` dispatched through the shim with the real runner | crash `ExitEvent` for the predecessor, then `dispatch(RestartAllocation)` | `active_alloc_count() == 1` (successor only) and `is_role_live(predecessor, _)` false. A recording driver trace shows `on_alloc_terminal(predecessor)` came from the observer path and no call was made for the successor | Tier 1 composed |
| S-3 | **D1 outcome independence:** observation write fails with retries exhausted (`RetryOutcome::Failed`), and `NoWrite` (no prior row) | `SimObservationStore` write fault; missing prior row | Supervisor gone in both; degraded `LifecycleEvent` still sent in the first | Tier 1 |
| S-4 | **D3 ordering:** recording driver trace for `StartAllocation` and `RestartAllocation` successor: `on_alloc_running` precedes `release_for_exit_emission`; a seeded schedule where the exit is available the instant the gate releases still leaves no supervisor | schedule variation on gate release vs observer | `NoRegisterAfterEnd`: after the observer consumed the event, `active_alloc_count()` has no entry for the id, across seeds | Tier 1 seeded sim invariant |
| S-6 | **D4 retained hooks:** `StopAllocation` on a `Running` VM allocation stops probes exactly once (no `ExitEvent` follows `VmDriver::stop`); `FinalizeFailed(ServiceFailed)` on a live VM stops probes | stop; finalize | Supervisor gone; `AllocStatusRow` as today | Tier 1 composed |
| S-7 | **Idempotence (D2):** double fire (observer then `FinalizeFailed`), unknown id, repeated calls | repeated hook | Count unchanged by the second call; other allocations and the successor's supervisor untouched | Tier 1 |
| S-8 | **Unaffected states / disabled:** sibling allocation keeps probing and `healthy`; a job with no descriptors and a driver with no runner take the default path | crash of one of two allocations | Sibling's `is_role_live` and backend `healthy` unchanged | Tier 1 |
| S-9 | **Conformance:** `quint-connect` replay of `specs/quint/service-allocation-readiness-and-ending` via `#[quint_test]` for each named run plus `#[quint_run]` | per spec | Per the pinned projection above | Conformance, `integration-tests` lane, `cargo xtask metal run --` |
| S-10 | **D5 edge:** a `Service` allocation whose startup streak reached `max_attempts` before the crash, deadline not yet passed at the crash, the first `ServiceLifecycle` evaluation of the `Failed` row at or after the deadline; the variant where the crash lands after the deadline before the `Running` row was evaluated; and the **Q3 consequence (BC-1)**: a crashed Service allocation past its startup deadline with no `Pass` and a streak below `max_attempts` (post-crash probing no longer grows the streak; the crash-authored rows are unchanged; no `StartupProbeFailed` finalisation arises) | crash `ExitEvent` through the observer (the production author of the `Failed` row, not a seeded row), then advance `SimClock` past the deadline | `ServiceLifecycle::reconcile` returns no `StartupProbeFailed` `FinalizeFailed`; `WorkloadLifecycle::reconcile` returns `RestartAllocation` once the backoff elapses (at the ceiling, `ServiceFailed { EarlyExit }` when every attempt repeats the pre-startup crash, D6); the row's recorded cause is the crash; the probe supervisor is gone (D1) | Tier 1 composed (both reconcilers + observer + real `ProbeRunner` + `SimObservationStore`), seeded, seed printed |
| S-15 | **D6, the classification at the ceiling:** `WorkloadLifecycle::reconcile` at `attempts >= RESTART_BACKOFF_CEILING` over kind (Service, Job) x startup probe declared (yes, explicit opt-out) x final row (`Failed` + `WorkloadCrashedImmediately`, `Failed` + start rejection, `Failed` + mTLS fail-closed, `Terminated` + `Stopped { Process }`, `Terminated` + `Stopped { LivenessProbe }`, `Terminated` + `PlatformReclaimed`) x `current_startup_probe` (`None`, `Some(Pass)`, `Some(Fail)`) | pure input | Terminal is `ServiceFailed { EarlyExit { exit_code } }` iff Service, declared, `Failed` + crash reason, not passed; `LivenessProbeFailed` for the liveness row; otherwise `BackoffExhausted`, except `Terminated` + `PlatformReclaimed`, which at the ceiling emits `RestartAllocation` with the successor's count equal to the predecessor's and no terminal (R-1); the returned `next_view` equals the input view; an already-finalised row (`BackoffExhausted`, `LivenessProbeFailed` or `EarlyExit`) emits nothing; `attempts < ceiling` emits `RestartAllocation` identically with and without the classification inputs (property: the restart decision is a function of the same inputs as at HEAD) | Tier 1 pure proptest; mutation target |
| S-16 | **D6, composed crash loop and stream:** a Service with a declared startup probe whose guest exits 1 on every start, through the production observer, `WorkloadLifecycle`, the shim and the Service stream handler | guest exit through `SimDriver` / `ExitEvent`; `SimClock` advanced across the backoffs | The stream's first terminal is `Failed { reason: EarlyExit { exit_code: Some(1) } }` with the stderr tail, after `RESTART_BACKOFF_CEILING` restarts and no earlier than the last attempt's crash; no `Stable`; the row carries `terminal: ServiceFailed { EarlyExit }`; every predecessor row is still `Failed` with its crash reason; the same service with a successor that passes the startup probe ends the stream with `Stable` and writes no `EarlyExit`; the same loop with the guest crashing after a recorded `Pass` ends `BackoffExhausted`; the only terminal write on a crashed allocation's row is `WorkloadLifecycle`'s and `ServiceLifecycle` drives no teardown or `on_alloc_terminal` effect; a control-plane restart between the last crash and the ceiling decision re-derives the same terminal from the durable rows. Seed printed | Tier 1 composed, seeded |
| S-17 | **D6 and D7, hydration:** `WorkloadLifecycle::hydrate_actual` fills `current_startup_probe` from `list_probe_results_for_alloc` of the current attempt only, when its row is `Failed` or `Terminated`: a `Pass`, a `Fail { last_fail_reason }`, an absent row, a non-terminal current row (not read), a superseded attempt's rows (never read); a read error | `SimObservationStore` fixture with probe rows on two attempts; injected read fault | Field equals the current attempt's latest startup status; superseded rows do not affect it; the store is unchanged (a recording store counts one probe-row read per hydration); a read fault returns `HydrateError::ObservationRead`, never `None` | Tier 1 |
| S-19 | **D6, render:** `format_service_failed_block` for `EarlyExit { Some(1) }` and `{ None }` | pure input | Exit-code line, Service guidance and stderr tail present; no `elapsed:` line; never `(took live)`; existing non-`EarlyExit` blocks byte-identical | Tier 1 pure (the live path is the stream render in `crates/overdrive-cli`, per its CLAUDE.md rule 2) |

The existing tests the early rows named
(`service_lifecycle_reconcile_branches.rs:598-642`, `service_lifecycle_stable.rs:313-354`,
`service_kind_vm_terminal_invariant.rs:263-274`) keep their inputs and change only the
expected action: the stop for a live allocation, not the finalisation (D7).

| ID | Scenario | Fault stimulus | Oracle | Tier |
|---|---|---|---|---|
| S-21 | **D7, the stop:** `ServiceLifecycle::reconcile` over state (`Pending`, `Running`, `Failed`, `Terminated`, `Draining`) x streak (`< max`, `== max`) x elapsed (`< deadline`, `>= deadline`) x startup pass (yes, no; the latest-row history `Pass, Fail, Fail` yields the stop, R-2) x `started_at` (set, unset) x listener present (yes, no) | pure input | A `Failed` or `Terminated` fact yields no action (the D5 guard and D6's deleted branch); otherwise an action iff all three gates hold, and then exactly `StopAllocation { terminal: Some(Stopped { StartupProbe }) }`, never a `FinalizeFailed`; no backend-row write accompanies the stop (R2-1; with a listener the backend row is written only by the ordinary eligibility projection, which already marks the allocation not `healthy`); `terminal_announced` contains the allocation after; a second evaluation with the same startup row emits nothing; re-emission on a newer row is S-29 | Tier 1 pure proptest; mutation target |
| S-22 | **D7, composed never-ready loop:** a Service with a declared startup probe whose guest stays alive and never passes, through the production observer, both reconcilers, the shim and the Service stream handler (model on `single_restart_authority_liveness_trajectory.rs`) | `SimDriver` guest that never binds; `SimClock` advanced across deadlines and backoffs; once with a short startup deadline (give-up inside the progress deadline) and once with the default deadline (progress deadline first) | Each stopped attempt's row is `Terminated` with `terminal: Stopped { StartupProbe }`, its guest stopped before the successor's start (recording driver order), its probe supervisor gone; `restart_counts` increments as for a crash; the stream is open across every stop; short deadline: first terminal `Failed { StartupProbeFailed { 0, last_fail, 5 } }` with row terminal `ServiceFailed { StartupProbeFailed }`; default deadline: first terminal `Failed { Timeout { 90 } }` with `last_attempt` naming the first stopped attempt (`Stopped { StartupProbe }`), and the give-up arrives on the row later; a successor that binds ends the stream with `Stable` and no `StartupProbeFailed` is ever written; a Job and an opt-out Service never receive the disposition. Seed printed | Tier 1 composed, seeded |
| S-23 | **D7, the ceiling classification:** `WorkloadLifecycle::reconcile` at `attempts >= RESTART_BACKOFF_CEILING` over final row (`Terminated` + `Stopped { LivenessProbe }`, `{ StartupProbe }`, `{ Operator }`, `{ SystemGc }`, `Failed` + `WorkloadCrashedImmediately` with and without a recorded startup pass, `Failed` + start rejection, `Terminated` + `PlatformReclaimed`) x kind (Service, Job) x declared startup probe (yes, opt-out) | pure input | Terminal is `LivenessProbeFailed` for the liveness row, `StartupProbeFailed { 0, last_fail, restart_counts }` iff the row is the startup stop (`last_fail` from the total mapping over `current_startup_probe`, D7 item 3, including a `Some(Pass)` row after the stop, which yields the empty string), `EarlyExit` per D6 for the crash, otherwise `BackoffExhausted`; the four are pairwise exclusive (property); `Stopped { Operator \| SystemGc }` is not restarted and not finalised as before; below the ceiling a startup-stopped row yields `RestartAllocation` identically with and without the new input; an already-finalised row emits nothing; `next_view` equals the input view | Tier 1 pure proptest; mutation target |
| S-25 | **D7, format:** `StoppedBy` serde and rkyv roundtrip incl. the new variant; the new golden-bytes fixture in `schema_evolution/alloc_status_row.rs`; existing `FIXTURE_V1` untouched; `api/openapi.yaml` regenerated and `openapi-check` clean; exhaustive-match arms (`human_readable`, `by_reclaims_platform` is `false`) | none | Roundtrips equal; the fixture decodes through `AllocStatusRowEnvelope` and `into_latest()`; the spec lists `startup_probe`. **Also (F-4):** a second golden-bytes fixture pins an `AllocStatusRow` whose `terminal` is `Stopped { by: LivenessProbe }`, the fixture ADR-0087 § Compliance required and HEAD lacks; `FIXTURE_V1` untouched | Tier 1 default lane (schema-evolution, proptest, openapi gate) |
| S-26 | **D7 and D9, stream projection:** `service_event_from_terminal` for `Stopped { StartupProbe }`, `Stopped { LivenessProbe }` and every other `Stopped { by }` | pure input | `None` for `StartupProbe` and `LivenessProbe`; `Some(Stopped { .. })` for `Operator`, `Reconciler`, `Process`, `SystemGc`, unchanged | Tier 1 pure; mutation target (the CLI must never reach "stopped by an unrecognised initiator" with exit 0 for a startup failure) |
| S-27 | **D6 and D7, wire and render:** `restart_budget_from_rows` for `BackoffExhausted`, `ServiceFailed { EarlyExit }`, `{ LivenessProbeFailed }`, `{ StartupProbeFailed }` and no terminal (`EarlyExit` and `StartupProbeFailed` are exhausted with `used == RESTART_BACKOFF_CEILING`; `LivenessProbeFailed` unchanged, tracked as F-2, GH #336); `render::workload_describe` for a Service with a startup-stopped predecessor and a `Running` successor; `format_service_failed_block` for `StartupProbeFailed` | wire fixture | Budget exhausted with `used == RESTART_BACKOFF_CEILING`; the predecessor renders `terminal: stopped (startup probe failed)`, the `Terminated` state and its `Fail` probe line, the successor `Running`; the failed block is byte-identical to today | Tier 1 pure (`render::workload_describe`, per `overdrive-cli/CLAUDE.md`) |
| S-28 | **D7, race exposure pinned (E-2):** the guest crashes after the evaluation and before the stop row | `ExitEvent` injected between evaluation and dispatch through the production observer | The written row is `Terminated` / `Stopped { StartupProbe }` and the give-up names the startup failure (the failed-stop exposure E-1 is closed by D8, S-30) | Tier 1 composed, seeded |

D8, D9, D10 and F-4 obligations; each row records contract, owner path, fault
stimulus, oracle and tier (`.claude/rules/testing.md` § "DISTILL — prove composed
system behavior under deliberate failure"). The D11 rows (S-39 to S-42) follow.

| ID | Contract / invariant | Production owner path | Fault stimulus | Oracle | Tier |
|---|---|---|---|---|---|
| S-29 | **D8 emission rule.** `StartupStopAtMostOncePerRow`, `NoStableOverPendingStop`, `LatePassUnstrandsTheInstance` | `ServiceLifecycleReconciler::reconcile` over successive hydrated facts (state x gates x latest startup `run_seq` x recorded `startup_last_acted_run` x latest startup status) | pure input | Emit iff guard and gates hold and `run_seq` exceeds the recorded value; the same `run_seq` never emits twice; a `Running` fact already in `terminal_announced` re-emits on a newer row; a `Pass` row ends the re-emission and the fact reaches the Stable branch; a non-`Running` fact in `terminal_announced` is skipped; `next_view` differs from the input only in `startup_last_acted_run`, `terminal_announced` and `observed`; the backend row marks the allocation not `healthy` while the stop is pending (no committed `Stable`, D12) | Tier 1 pure proptest; mutation target |
| S-30 | **D8 composed retry; B2 stop row write.** `FailedStartupStopIsRetried`, `NoStrandedStop` | production observer, both reconcilers, shim `StopAllocation`, real `ProbeRunner`, Service stream | `SimDriver::stop` scripted to fail the first N calls, then succeed; `SimClock` advanced across probe intervals; a variant where the startup probe passes after the first failed stop; **a variant (B2) where `SimObservationStore::write_alloc_lifecycle` fails once for the stop's `Terminated` row (startup stop, then a liveness stop)** | Stop re-emitted exactly once per new startup row (recording shim trace); row becomes `Terminated` / `Stopped { StartupProbe }` after the first success; the successor starts only after the stop landed; in the `Pass` variant no further stop is emitted and the allocation reaches `Stable` and is eligible only at that `Stable` commit; in the row-write variant the supervisor is still registered after the failed write (`active_alloc_count()` unchanged), the next probe row re-emits the stop, the retry finds the guest stopped (not-found tolerated), writes the row, and only then does `active_alloc_count()` fall. Seed printed | Tier 1 composed, seeded |
| S-31 | **D9 runner contract.** `LivenessOnlyAfterStable`, `NoLivenessForEndedId` | `ProbeRunner::start_alloc`, `start_liveness`, `stop_alloc` through `VmDriver::on_alloc_running` / `on_alloc_stable` / `on_alloc_terminal` | `SimClock` and recording probers; `on_alloc_terminal` before `on_alloc_stable`; `on_alloc_stable` twice; concurrent `stop_alloc` and `start_liveness` over seeds | `is_role_live(alloc, Liveness)` false after `on_alloc_running`, true after `on_alloc_stable`, false after terminal; startup and readiness roles unchanged; a second `on_alloc_stable` spawns nothing; after terminal, `on_alloc_stable` leaves `active_alloc_count()` unchanged and spawns nothing; no liveness row exists before the `Stable` call under any seed; after a control-plane restart between `Stable` and the first liveness row, boot reclamation makes the unsupervised VM terminal (`PlatformReclaimed`), the successor's supervisor registers fresh, and `on_alloc_stable` for the old id is a no-op | Tier 1 composed (real runner), seeded |
| S-32 | **D9 composed.** `NoLivenessStopBeforeStable`, `LivenessStreakStartsAtStable` | production observer, both reconcilers, shim, real runner with a prober whose liveness fails for the whole startup. **DG-9 (2026-11-05):** the restart and give-up leg is composed, not waived: the production `VmDriver` over `SimVmm` and a stepped `SimBeaconIo` (the `quint_cid_lease` composition, `guest_network/quint_cid_lease.rs`), so a restart runs `VmDriver::start` | a Service whose liveness probe fails from t=10 s and whose startup probe passes at t=30 s; then a liveness probe that keeps failing after `Stable` | No `Stopped { LivenessProbe }` before `Stable` or on the first tick after; the first liveness row is one interval after `Stable` with streak 1; after `failure_threshold` post-`Stable` failures the instance is stopped and restarted under the shared budget, and the give-up is `LivenessProbeFailed`; an opt-out Service starts liveness at its first `Running`'s `Stable`. Seed printed | Tier 1 composed, seeded |
| S-34 | **D10 derivation.** The deadline exceeds the startup deadline by construction (tested on the real function) | `startup_deadline`, `progress_deadline` (pure) | proptest over descriptor lists (empty, one, many, overflowing `interval x max_attempts`) and floors | `progress_deadline > startup_deadline` and `>= floor` for every input; the default spec yields 90 s; `startup_deadline` equals the value `spec_facts_for_service` produced before the extraction (golden cases) | Tier 1 pure proptest; mutation target |
| S-35 | **D10 composed stream.** `HealthySlowStartNeverTimesOut` (under DL-6), the timeout carries no verdict (the stream writes no row), `StreamEndsOnceHonestly` | `build_service_stream` through the submit handler with real shim events, `SimClock`, both `Inserted` and `Unchanged` paths | (a) startup deadline 120 s, `Stable` at 100 s; (b) a never-ready Service with the default deadline; (c) a short startup deadline whose give-up lands first; (d) an `Unchanged` redeploy of a stuck Service with no recorded verdict (a settled one is S-39); (e) a Job; (f) exposure E-4: a guest that boots at `VM_BOOT_DEADLINE` (30 s) and passes its startup probe on the last attempt | (a) `Stable`, no `Timeout`; (b) `Failed { Timeout { 90 } }` with `last_attempt` = the latest `Failed` or `Terminated` event seen (the startup stop), `alloc_id: None`, no row written by the stream; (c) the give-up terminal, unchanged; (d) a new clock, `last_attempt` from events after open only; (e) the Job stream keeps `streaming_cap` and its output byte-identical; (f) `Failed { Timeout }` with `last_attempt: None` while the allocation then reaches `Stable`: an honest timeout, the stated exposure, never a success claim | Tier 1 composed, seeded |
| S-36 | **D10 CLI.** wire and render | `ApiClient::submit_workload_streaming`, `format_service_failed_block`, `consume_stream` | wire fixtures; a request timeout probe. **DG-10 (2026-11-05):** the probe pauses the clock over a real TLS socket, so `overdrive-cli` takes `tokio` with the `test-util` feature as a dev-dependency (approved). The test sits behind `integration-tests` and carries a litmus proof that it fails when the `deadline + 30 s` envelope is removed (the request timeout falls back to the fixed 120 s) | Request timeout is `deadline + 30 s` for the Service lane and `DEFAULT_STREAMING_CAP + 30 s` (120 s) for the Job lane; the `Timeout` block shows the new wording and the labelled `last attempt (alloc ...)` block when present, nothing extra when absent; exit code 1; `ServiceSubmitEvent::Failed` without `last_attempt` still decodes (additive); OpenAPI regenerated and `openapi-check` clean | Tier 1 pure (render and wire per `overdrive-cli/CLAUDE.md`) |
| S-38 | **D8 CBOR evolution.** ADR-0035 additive field | `ServiceLifecycleView` decode through the `ViewStore` codec | a CBOR blob written without `startup_last_acted_run`; a blob that still carries the removed `stable_announced` key | Decodes with an empty map, other fields unchanged; the removed key is ignored; a blob with the field round-trips | Tier 1 default lane |

D11 obligations (accepted 2026-11-03). The recorded verdict is read through the
production handler, shim and reconcilers; no test installs a row the production owner
path does not write.

| ID | Contract / invariant | Production owner path | Fault stimulus | Oracle | Tier |
|---|---|---|---|---|---|
| S-39 | **F-5, redeploy ends at once.** `NoMissedTerminal`, `RecordedVerdictNeverStale` | submit handler (`Inserted` then `Unchanged`, NDJSON lane) -> `build_service_stream`, with the real shim, `ServiceLifecycle`, `WorkloadLifecycle` and `SimObservationStore` | the same spec submitted twice: (a) the first reached `Stable`; (b) the first was given up (each of the four give-up terminals); (c) the first is starting; (d) the first's current attempt was stopped for a probe (startup, liveness) and its successor is starting; (e) the first was operator-stopped; (f) `overdrive workload restart` after a give-up, redeploy while the successor starts | (a) second stream yields `Accepted { outcome: unchanged }` then `Stable` with the recorded `settled_in_ms` and witness, no clock advance; (b) the recorded `Failed`; (c), (d), (f) no terminal until the live event or D10's deadline, and (f) never reports the superseded give-up; (e) `Stopped` naming the operator. The redeploy writes no row, event or `View`. `Inserted`-path behaviour unchanged | Tier 1 composed, seeded |
| S-40 | **Ordering.** `NoMissedTerminal` | `build_service_stream` with rows written and events published at scripted points around the stream's subscription and read | seeded schedule: commit+publish before the stream's first poll; between `Accepted` and the read; after the read; `Lagged` with a missed commit then relist; a superseding fresh-id attempt | In every schedule exactly one terminal line, the same whichever path delivers it; none lost or duplicated; a `Lagged` stream recovers the terminal it missed; a line from a live event names an attempt that was current when the event was published (R-4); seed printed | Tier 1 seeded sim invariant |
| S-41 | **Snapshot equals live line.** ADR-0037 §4 | the shim `FinalizeFailed` (`Stable`, give-ups) and `StopAllocation` rows, written through the real shim, and the event published for each write | none | For each terminal class the line built from the committed row equals the line built from the published event (byte-equal); an event received implies its row is readable (commit precedes publish). A mismatch is a design deviation to surface | Tier 1 composed |
| S-42 | **Failed read (D11-E1).** `FailedReadNeverWaits` | `build_service_stream` with a `SimObservationStore` whose `alloc_status_rows` fails: at open, and on a relist after `Lagged` | injected `ObservationStoreError::Unreachable`, `Io` or `Envelope` where the port permits; an ordinary local corrupt-row scan is logged/skipped under ADR-0048, not claimed as an `Envelope` stream error | The stream yields exactly `ServiceStreamError::ObservationRead { source: original_error }` and ends; the source variant/payload survives, no `Timeout` or wait follows, and the store is unchanged. No other stream-error variant is emitted for an observation-read failure | Tier 1 |

Obligations of the 2026-11-03 rulings (B1, B2, B3, D12, D17). B5 was rejected and has no
obligation; B2's obligation is the row-write variant of S-30.

| ID | Contract / invariant | Production owner path | Fault stimulus | Oracle | Tier |
|---|---|---|---|---|---|
| S-43 | **B1 and R2-2, a lost `Stable` is re-announced; no view marker.** `StableEventuallyCommitted`, `LivenessOnlyAfterStable` | `ServiceLifecycleReconciler::reconcile` over successive hydrated facts, then the shim `FinalizeFailed(Stable)` arm and `VmDriver::on_alloc_stable`, real `ProbeRunner` | `SimObservationStore::write_alloc_lifecycle` fails once for the `Stable` row (**including on the allocation's very first evaluation**); a variant with a control-plane restart (the `View` reloaded) between the `View` persist and the dispatch | The next evaluation emits `Stable` again (the fact has no committed `Stable`); after the write lands the row carries `Stable`, `on_alloc_stable` fires once, liveness is live and the allocation is eligible only from then; a fact with a committed `Stable` emits nothing and `next_evaluation_at` returns no boundary for it; no `Stable` is emitted twice for a committed row (V31); a `View` blob that still carries the removed `stable_announced` key decodes (S-38) | Tier 1 composed, seeded |
| S-45 | **B3, an out-of-memory crash loop gives up as `EarlyExit`.** | The S-16 composed loop with the guest killed by the cgroup OOM killer on every start | `SimDriver` exit with OOM facts | The give-up is `ServiceFailed { EarlyExit { exit_code: None } }`; the S-15 grid gains `Failed` + `VmOutOfMemory` (pre-startup: `EarlyExit`; startup passed: `BackoffExhausted`) | Tier 1 composed and pure |
| S-46 | **D12, the eligibility predicate.** `compute_backend_healthy` (through `ServiceLifecycleReconciler::reconcile`'s backend row) over startup probe declared (yes, opt-out) x `stable_committed` (yes, no) x own `terminal_announced` membership (yes, no) x readiness (none; declared with Pass at, below and above threshold; declared with Fail; no row) x an unrelated allocation in `terminal_announced` (yes, no) x latest startup row (`None`, `Pass`, `Fail`) | pure input | `healthy` iff (opt-out or `stable_committed`) and the readiness rule holds. No per-allocation backend veto exists: the latest startup row, the allocation's own and any unrelated `terminal_announced` membership never change the result, and a stop-eligible allocation has no committed `Stable`, so it is not `healthy`. Opt-out rows are byte-identical to the pre-feature predicate; `next_view` is unchanged by the call | Tier 1 pure proptest; mutation target |
| S-47 | **D12, composed.** `EligibleOnlyAfterOwnStable`, no flap after `Stable` (G-9) | production `WorkloadLifecycle`, `ServiceLifecycle`, shim, `SimObservationStore` and the resolver's `BackendIndex` read of the written `ServiceBackendRow`; real `ProbeRunner` with a recording prober; `SimClock` | (a) a Service with a startup probe and no readiness probe: first allocation `Running`, startup probe unbound then passing; (b) the same, first attempt startup-stopped (D7/D8), successor `Running`; (c), (d) are covered by S-30 (a late startup `Pass` after a stop that did not land) and S-43 (the `Stable` row write fails once); (e) after `Stable`, an extra startup `Fail` row lands (role not yet retired), no further startup rows; (f) a control-plane restart (`View` reloaded) between `Stable` and the next evaluation; (g) a crash successor (D6); (h) an opt-out Service; (i) a readiness-probed Service whose readiness streak met the threshold before `Stable` | (a) the backend row is `healthy: false` from `Running` until the `Stable` row is committed, then `true`; the resolver returns the existing fail-closed result before and the backend after; (b), (g) the successor is `false` until its own `Stable` is committed, then `true`; (c), (d) as in S-30 and S-43 (eligible exactly at the `Stable` commit, never `true` in between); (e) stays `true`; (f) unchanged; (h) `true` at `Running`, as today; (i) `true` at the `Stable` commit. A dropped row write is repaired by the next evaluation. Seed printed | Tier 1 composed, seeded |
| S-48 | **D17, the wake (ADR-0273, once confirmed).** `NoLostWake` (a submit is admitted without waiting for the cadence tick), `CadenceNeverStarved`, `DrainOrderDeterministicAcrossSeeds`. No Quint model: this scenario is the evidence (§ D17, S6) | `ReconcilerRuntime::submit_evaluation` (a producer, including the interest router's `from_runtime` handle and the submit handler) and the production `spawn_convergence_loop` with `SimClock` and a recording action shim; `EvaluationBroker::counters()` as the observation | seeded schedules: (a) a submit while the loop is parked, `SimClock` not advanced; (b) a submit made from a task the sim schedules while the loop is admitting or awaiting an in-flight evaluation (the drain-to-park window has no await point on a single-threaded sim, so it rests on tokio's documented permit, § D17; the mutation targets carry the signal position); (c) a burst of N submits for N distinct targets and for one target; (d) no submits, `SimClock` advanced one cadence period; (e) a submit for a target whose evaluation is in flight; (f) a deferred (`NotBefore`) evaluation | (a) admitted with no clock advance, once; (b) admitted without waiting for the tick; (c) one drain admits up to the capacity in FIFO order, a same-key burst collapses to one dispatch (`counters().dispatched` and `.cancelled`); (d) the cadence arm still drains and the due resync fires at most once per period; (e) admitted when that evaluation completes, never concurrently; (f) admitted at its eligibility, not earlier because of an unrelated wake; the admission order is identical across two runs of the same seed. Seed printed. **DG-9 (2026-11-05):** (b) is waived: a single-threaded sim has no await point between the insert and the wake, and the scenario is carried by the mutation gate, whose targets for the signal position (moved before the insert, dropped, or a stored permit; D17 mutation targets) the gate must kill. That dependency is the whole of (b)'s evidence. (e) is waived: it needs a reconciler whose tick blocks in flight, which no production reconciler is, so authoring one is a test seam | Tier 1 seeded sim invariant |

| ID | Contract / invariant | Production owner path | Fault stimulus | Oracle | Tier |
|---|---|---|---|---|---|
| S-56 | **D6, DG-12: a crashed started allocation closes its mid-startup window.** `CrashedAllocationDoesNotKeepTheLoopAlive` | `ServiceLifecycleReconciler::reconcile` over a hydrated fact for each (state: `Failed`, `Terminated`) x (`started_at`: set, unset) x (latest startup probe: none, `Fail`, `Pass`) x (startup probes: declared, opt-out), starting from a view whose `observed` already holds the allocation (as after its `Running` evaluation) | pure input | In every case the returned `View.terminal_announced` contains the allocation, `has_alloc_mid_startup_window()` on the returned view is false, and `next_evaluation_at` returns `None`. The returned `Vec<Action>` is empty. The `started_at` set cases are the ones the pre-DG-12 code leaves open. A second evaluation of the same fact changes neither the view nor the actions. A `Running` fact with no gate met is the control: it is not in `terminal_announced` and its mid-startup window stays open | Tier 1 pure enumerated grid (not a proptest); mutation target (the insertion on a started `Failed` row, and on a `Terminated` row) |


Obligations of D13 to D16 for PR-1, PR-4 and P1 (the guest-listener gate boundary scenarios
are G-10 above; the beacon-channel sequences are the `guest_report` runs). Nothing here is
accepted until the user accepts D13 to D16.

| ID | Contract / invariant | Production owner path | Fault stimulus | Oracle | Tier |
|---|---|---|---|---|---|
| S-50 | **PR-1, the board's set.** Entry lifetime; the set only grows and stays within the declared set | `GuestListenerBoard` driven only through the beacon reader (scripted `SimBeaconIo` lines) and `VmDriver::start` / `on_alloc_terminal`, observed through `GuestListenerReports::reported` | proptest over sequences of scripted lines (declared, undeclared, duplicate, another allocation's), then `on_alloc_terminal`, then lines after close | `reported` is empty before `start` and after `on_alloc_terminal`; during the entry's life the set only grows and `⊆ declared`; a duplicate changes nothing (insertion is observed as a change of `reported`) | Tier 1 proptest; mutation target |
| S-51 | **PR-1, no completion on a no-op; host reader.** `changed` completes on none of: duplicate, undeclared pair, another allocation's report, no-entry id, record after close; a waiter pending at close is not woken. Reader: a garbage line after `READY` is dropped with a warn event naming the allocation; a `Listen` line after `Exit` changes neither the set nor the exit report; a `Listen` never occupies the exit report | `GuestListenerBoard` and the real beacon reader driven by `SimBeaconIo` (`guest_write`, `guest_close`, `release_read`) | scripted lines in each order; close with a pending `changed` | each `changed` future stays pending (polled under a seeded executor) for the no-op cases; the pending one at close is not completed; reader outcomes as stated. `SimGuestListenerReports` is driven through the same clauses and a `guest_listener_reports_equivalence` test runs the board and the sim through one call sequence and compares `reported` and completions (design rule: a trait's contract is enforced by an equivalence test) | Tier 1 |
| S-52 | **PR-1, `changed`.** `GuestListenerChangedNeverMissesAnInsertion`: if the set differs from `seen` when `changed` is called it completes at once; an insertion between a `reported` and a `changed` is not lost; cancel-safe (a dropped `changed` future then an insertion: the next `changed(seen)` completes at once) | board + `SimClock` under seeded schedules of `reported` / `changed` / record / cancel | schedule variation on the order of the four | the invariant holds in every schedule; completion is on the insertion, with no clock advance; seed printed | Tier 1 seeded sim invariant |
| S-53 | **D14 item 3, the guest-listener task rows.** `NoFailBeforeDeadline`, `NoPassAfterEnding`; rows use the shared run state (PROBE-RUN-COUNT unchanged) | real `ProbeRunner::start_alloc(&AllocationSpec)` with `declared_listeners`, `SimGuestListenerReports`, `SimObservationStore`, `SimClock` | the declared set completes before / after the deadline; a failing observation write on the `Pass` (retried until it lands) and on a `Fail`; `stop_alloc` mid-wait and between the insertion and the write | `Pass` is the first row with `same_status_runs` 1 and `run_seq` 1; no row before the deadline; the first `Fail` at the deadline has `same_status_runs` 1, then 2, 3, … one per interval; a failed write advances neither number; no row write is started after cancellation; the existing run-numbering property (`run_numbering_tests.rs`) is run with the new task as a producer | Tier 1 composed, seeded; mutation target |
| S-54 | **P1, the stop gate.** `startup_attempts_required` | `ServiceLifecycleReconciler::reconcile` over hydrated facts; `spec_facts_for_service` | proptest: mechanic (`Tcp`, `Http`, `GuestListeners`) x latest startup row (`Pass`, `Fail` with streak 1, 2, `max_attempts`) x elapsed (`< deadline`, `>= deadline`) | for `Tcp` / `Http` the stop requires streak `>= max_attempts` exactly as before (golden cases from today's tests); for `GuestListeners` the stop holds at streak 1 with the deadline elapsed and no `Pass`, never before the deadline; a later `Fail` row re-emits per D8 (S-29) | Tier 1 pure proptest; mutation target |
| S-55 | **PR-4, one declared set.** The TCP view is a filter by construction | `AllocationSpec::tcp_service_ports()`, `WorkloadLifecycle::hydrate_desired`, the inbound TPROXY install (`mtls_intercept_worker.rs`, `action_shim/mod.rs:1522-1525`) | proptest over declared listener lists (TCP, UDP, mixed, empty-TCP); a Service declaring only a UDP listener | `tcp_service_ports()` equals the TCP-protocol filter of `declared_listeners` in declaration order, for every list; a UDP-declared Service gets no TPROXY rule; `WorkloadLifecycleState` no longer has `service_ports` and all three construction sites carry `declared_listeners` | Tier 1 pure proptest; mutation target |

Mutation targets by decision:

- **D13 to D16 (PR-1, PR-4, P1).** The insertion-reporting check-and-act, the declared filter and the wake-only-on-insertion rule in the board (S-50, S-51); the `seen` comparison in `changed` (S-52); the deadline arm and the `Fail` cadence of the guest-listener task, and the token re-check before a write (S-53); the `GuestListeners` arm of `startup_attempts_required` and its `1` (S-54); the TCP filter of `tcp_service_ports()` (S-55).
- **Hook ownership (D1 to D5).** The observer's new call (S-1); the order swap in both shim sites (S-4); `cleanup_restart_predecessor`'s non-change (S-2); D5's guard, replacing the terminal-state test with `== Terminated` or deleting it (S-10, S-21).
- **D6.** The terminal selection in the ceiling branch, each condition of the pre-startup-crash predicate removed or negated in turn, and always choosing `BackoffExhausted` (S-15, S-16); the idempotency guard extension, removing `EarlyExit` from it (S-15); the `current_startup_probe` population, always-`None` or always-`Some(Pass)` (S-15, S-17); the `restart_budget_from_rows` arm (S-27); the `terminal_announced` insertion for a `Failed` or `Terminated` fact, removed or restricted to `started_at == None` or to `Failed` (S-56, DG-12). The `workload_lifecycle.rs` and `service_lifecycle.rs` `reconcile` bodies are mandatory mutation targets (`.claude/rules/testing.md`).
- **D7.** The action swap in `startup_probe_failed_action`, restoring a `FinalizeFailed` (S-21, S-22); the stream projection arm (S-22, S-26); the exclusion of the new disposition from the intentional-stop test (S-23); each case and the order of the ceiling selection (S-23); the `current_startup_probe` population for the startup stop (S-23, S-17); the `restart_budget_from_rows` arm (S-27).
- **D8 to D10.** The `run_seq > recorded` comparison and the `Pass` exemption (S-29, S-30); the top-of-loop dedup change for `Running` facts (S-29); the removal of the liveness spawn from `start_alloc` and of the `start_liveness` call from `on_alloc_stable` (S-31, S-32); the lock hold in `start_liveness` (S-31); the `startup_deadline + margin` term and the `max` with the floor (S-34); the timer armed from the passed deadline and the `last_attempt` projection (S-35); the request-timeout derivation (S-36).
- **D11.** Moving the read before the subscription (S-40); selecting any row instead of `current_alloc` (S-39 f); dropping the `None` projection of the two restartable stops from the read (S-39 d); absorbing a read error (S-42); the relist on `Lagged` (S-40).
- **D12, R2-1, B1 to B3.** The `stable_committed` test in step 1: removing it, negating it, or reading the latest startup row instead (S-46, S-47 a, e); the opt-out exemption (S-46, S-47 h); the readiness rule after step 1 (S-47 i); reintroducing a backend-row write on account of the stop (S-21, S-46); the `!stable_committed` condition of the `Stable` branches and of the loop-top skip (S-43); the position of `on_alloc_terminal` relative to the row write (S-30); the `VmOutOfMemory` arm of the predicate (S-45).
- **D17.** The signal after the lock release, moving it before the insert or dropping it (S-48 a, b); a stored permit rather than a waiter-only wake (S-48 b); the `submitted` arm of the loop's `select!` (S-48 a); the cadence arm (S-48 d).

Test support: the existing `RecordingDriver` fixtures (`action_shim/replacement_acceptance.rs:178-232`, `mod.rs:4062-4110`) gain a recording `on_alloc_terminal`; the S-SVM-16 driver-level test (`service_kind_vm_workloads.rs:489-492`) hand-calls the hook with the same id for both registrations, so it is not evidence for this feature.

## Wave: DESIGN / [REF] Changed and unchanged assumptions

- **Changed:** the contract text "fired by the action shim … immediately after the
  action shim writes the terminal `AllocStatusRow`" (`driver.rs:739-742`) is replaced
  by D2. The shim is no longer the only caller; the hook no longer presupposes a row.
- **Changed (D5):** the comment at `service_lifecycle.rs:1174-1191` (startup-failure
  branch barred only for `Terminated`) becomes: barred for every terminal row. State
  meanings do not change.
- **Changed (D6, option (a)): the K1 honesty target, for `EarlyExit`.** The historical
  document under `docs/evolution/` is not edited. Original text, from
  `docs/evolution/2026-05-30-service-health-check-probes.md`:

  > K1 | North Star | Service-submit honesty rate: `Failed { StartupProbeFailed \| EarlyExit }` (never `Stable`, never bare `Running`) within `startup_deadline + 1 tick`, ≥99/100

  and the north-star contract:

  > a Service whose workload exits `1` within the startup deadline (the reshaped coinflip case) must emit `ServiceSubmitEvent::Failed { reason: EarlyExit { exit_code: 1 } }` on **≥99 of 100** deterministic trials, with **zero** `Stable` terminals and **zero** occurrences of the literal `(took live)` string in any rendered event.

  Under D6 the time bound no longer holds for `EarlyExit`: the terminal is written at
  the restart authority's give-up (about six seconds for the coinflip fixture),
  independent of `startup_deadline` (60 s default, ADR-0058). Restatement (the
  `StartupProbeFailed` clause is restated by D7):

  > K1 | North Star | Service-submit honesty rate: `Failed { StartupProbeFailed \| EarlyExit }` (never `Stable` for a workload that keeps exiting, never bare `Running`): `StartupProbeFailed` within `startup_deadline + 1 tick` for an instance that stays alive, `EarlyExit` when the restart budget is spent for an instance that keeps exiting before a startup probe passes (about `RESTART_BACKOFF_CEILING` x `RESTART_BACKOFF_DURATION` plus per-attempt start time and one tick), ≥99/100

  North-star contract restated: a Service whose workload keeps exiting `1` before any
  startup probe passes emits `Failed { reason: EarlyExit { exit_code: 1 } }` once the
  restart budget is spent, on ≥99 of 100 trials. The zero-`Stable` and
  zero-`(took live)` claims are unchanged.
- **Changed (D7): the K1 bound for a Service that boots but never passes its startup
  probe.** "Within `startup_deadline + 1 tick`" held at HEAD only because
  `ServiceLifecycle` finalised the first attempt while `WorkloadLifecycle` kept
  restarting. Under D7 the terminal arrives at give-up (about 365 s at the default).
  Restatement of the `StartupProbeFailed` clause:

  > `StartupProbeFailed` when the restart budget is spent for an instance that stays alive and never passes a startup probe (about `(RESTART_BACKOFF_CEILING + 1) x startup_deadline` plus `RESTART_BACKOFF_DURATION` per restart), on the allocation's row and in `workload describe`; on `overdrive deploy` when that is within the stream's progress deadline (D10), otherwise the stream ends `Failed { Timeout }` carrying the last attempt's cause as labelled detail

  The stream never reports success for a startup failure (D7 item 5, D9); a
  non-terminal event per failed attempt (P-3) stays out.
- **Changed (D10): K1 for the whole stream.** The Service deploy stream ends with
  exactly one terminal: `Stable` as soon as a startup probe passes; the restart
  authority's give-up (`EarlyExit`, `StartupProbeFailed`, `LivenessProbeFailed`,
  `BackoffExhausted`) when written inside the progress deadline (the larger of 90 s and
  startup deadline plus 30 s); else `Failed { Timeout }` with the latest
  attempt-ending observation as labelled `last_attempt` (detail, never the verdict).
  It never reports success for a non-`Stable` Service and never retracts a failure.
  Replaces the "within `startup_deadline + 1 tick`" wording for `StartupProbeFailed`
  and the "cap timer (90 s) is not reached" sentence.
- **Changed (D11): K1 mirror for a settled workload.** The stream never reports a
  failure for a Service already `Stable`: a redeploy of an unchanged spec ends with the
  verdict the current attempt already carries, not `Timeout`.
- **Changed (D7): `StartupProbeFailed.attempts`.** The doc at
  `transition_reason.rs:689-693` and ADR-0055 describe probe attempts; at give-up it is
  the restart budget consumed (ADR-0087 D4). The rendered sentence is unchanged.
- **Changed (D7): D5's user-visible sentence** "a guest that stays `Running` with
  exhausted probes past the deadline is finalised as `StartupProbeFailed`" becomes the
  D7 operator table. D5's guard is unchanged.
- **Changed (D9): when liveness probing starts.** Liveness tasks start at `Stable`, not
  `Running` (role contract `probe_result_row.rs:283-286`). ADR-0054 § 2's "spawns one
  task per descriptor at `on_alloc_running`" is restated for liveness.
- **Changed (D8): the startup stop's dedup.** D7's "inserted into `terminal_announced`
  at emission, dedups" becomes: inserted at emission, no longer suppresses a `Running`
  fact; one re-emission per new startup row; a `Pass` after a stop that did not land
  ends the re-emission and lets `Stable` run.
- **Changed (D6): ADR-0055's early-exit clause.** `decide_per_alloc` step 1
  (`adr-0055-service-lifecycle-reconciler.md:386-390`): "**Terminal check first.** If
  `alloc_row.state == Failed` AND `view.stable_announced.contains(alloc_id) == false`
  AND `tick.now_unix - alloc_row.started_at < startup_deadline`: emit
  `Action::SetTerminalCondition(Failed { reason: EarlyExit { ... } })` — closes US-08."
  becomes: `ServiceLifecycle` has no such step; `WorkloadLifecycle` selects `EarlyExit`
  at the restart ceiling. A dated note on ADR-0055 lands with the D6 DELIVER commit.
- **Changed (D6): `ServiceFailureReason::EarlyExit`.** The variant doc at
  `transition_reason.rs:694-697` ("Workload exited before any startup probe could pass
  AND within `startup_deadline` window") loses the deadline clause and gains the
  restart-budget clause.
- **Changed (D6): the CLI block.** The `elapsed: Xs (startup_deadline=Ys)` line
  (`render.rs:1540-1545`) is removed; it would include the restarts.
- **Changed (D12): ADR-0096's "a non-terminal no-readiness allocation stays healthy".**
  ADR-0096 D1: "Thus the no-readiness compatibility default remains true for a
  non-terminal Running allocation, including an ordinary startup-success/Stable
  allocation." It now holds only for a Service declaring no startup probe (the opt-out)
  and for an allocation whose own `Stable` is committed. For a startup-probed Service,
  an allocation without committed `Stable` is not `healthy`. Dated note on ADR-0096
  points at ADR-0267.
- **Changed (R2-1): ADR-0096 D1 step 1 and D2 are superseded.** D1: "an allocation for
  which the current `ServiceLifecycle` tick has reached the existing non-Stable startup
  terminal is `healthy = false`"; D2: "The existing `WriteServiceBackendRow` carrying
  `healthy: false` for the deciding allocation must precede the existing
  `FinalizeFailed` action in that reconcile batch." D12 already withholds that
  allocation and D7 replaced `FinalizeFailed` with a stop. ADR-0096 D3 (owners) and D4
  (scope, test obligations 1 and 2 replaced) stand.
- **Changed (R2-2):** the `stable_announced` field of `ServiceLifecycleView` is removed
  (no reader remains after B1); ADR-0055's `decide_per_alloc` text naming it is moot.
- **Changed (B1, B2):** D9's "the `Stable` hook fires after the `Stable` row commits"
  now holds across a write error and a restart (ADR-0265); D4's and D7 item 4's "the
  shim is unchanged" no longer holds for the `StopAllocation` hook position (ADR-0266).
- **Changed (B3):** D6's pre-startup-crash predicate reads `WorkloadCrashedImmediately`
  and `VmOutOfMemory` (ADR-0258).
- **Unchanged by D6:** ADR-0087; ADR-0078 and ADR-0105 (findings F-1, GH #335; F-2, GH
  #336); ADR-0083 D6 (reclaimed row stays `Terminated`); the ceiling exemption for
  platform reclamation and generation replacement.
- **Unchanged by D7:** the liveness path (detection, stop, give-up); ADR-0087's budget
  rule; the intentional-stop exemption; the shim; the exit observer; D5's guard; D6's
  classification; the shape of `ServiceFailureReason` and of every persisted row (apart
  from one fieldless `StoppedBy` variant).
- **Unchanged by D8 to D10:** ADR-0087's budget rule; the liveness stop and give-up
  once `Stable`; the intentional-stop exemption; the shim; the exit observer;
  `ServiceFailureReason` and every archived row; the Job stream; the readiness role; the
  TOML spec surface.
- **Unchanged by D17:** state meanings, per-target serialization, drain order, deferred
  eligibility, the ADR-0084 wakeup model and no-storm constraint; the cadence tick
  remains the level-triggered backstop.
- **Unchanged:** state meanings of `Running`, `Stable`, `Failed`, `Terminated`; the
  Running-confirmed gate; ADR-0087 single restart authority; ADR-0105 fresh ids;
  ADR-0100 watcher claim; `release_supervision` placement and its dst-lint clause; the
  `ProbeRunner` API apart from `start_liveness` (D9).

## Wave: DESIGN / [REF] Quality attributes

Reliability (no leaked supervisors or stale probes; supervisor count tracks live
allocations); maintainability (one author per kind of ending); testability (existing
inspection surfaces); performance (budgets below). OSS: no new dependency.

## Wave: DESIGN / [REF] Cost budgets

Growth beyond either budget needs the user's approval.

**Hot-path budget.**

| Decision | Added cost | Bound |
|---|---|---|
| D1 | One transient `Weak` upgrade and one `stop_alloc` (supervisors-map lock hold, no I/O, no `.await`) per consumed `ExitEvent` | Zero on the steady path; one call per guest exit, bounded by the restart budget |
| D3 | None | Zero |
| D5, D6, D7 | Pure code in `reconcile`; no extra read except the D6 hydration | See hydration |
| D6, D7 hydration | **One** `list_probe_results_for_alloc` read, current attempt only, only when that row is `Failed` or `Terminated` | Zero reads for a `Running` workload; never one per historic row |
| D8 | One stop re-emission per new written startup probe row while a stop has not landed; one `View` entry per `(allocation, probe)` stopped | At most one per `interval_seconds`; unbounded in time only while the stop keeps failing |
| D9 | One `start_liveness` per allocation at `Stable`, under the supervisors lock | O(liveness descriptors), once |
| D10 | Two pure arithmetic functions per stream open; the CLI computes the same | Zero I/O |
| D17 | One atomic per submit; one extra iteration (broker lock plus one `drain_pending`, O(P log P)) per wake | At most one iteration per submit, cadence tick and completion; a burst coalesces into one permit |
| D11 | **One `alloc_status_rows` scan per Service stream open and one per `Lagged`**; none per event | Per request; a `Lagged` storm is bounded by the broadcast capacity |
| D12 | None (fact already hydrated) | Zero |

**Validation budget.**

- Model checks: the recorded run (`specs/quint/service-allocation-readiness-and-ending/evidence/summary.md`)
  is the authority for counts and times. D12, D17, R2-1, R2-2 and B3 add none. The beacon
  report protocol (D13 items 1, 3 and 10) adds two small modules, `guest_report` and the
  conformance projection `guest_report_host` (§ Formal protocol model — D13 to D16); counts
  and times are the recorded run's, not restated here.
- Conformance: one `#[quint_test]` per kept named run and one `#[quint_run]` per
  module, in the owning crate's `integration-tests` lane on metal. No new lane, tool or
  xtask subcommand.
- DISTILL: the S-numbers in § Executable obligations are the scenario set; D13 to D16 add the
  obligations in G-10's boundary scenarios once accepted.

## Wave: DESIGN / [REF] D17 — a broker submit wakes the convergence loop at once (Accepted 2026-11-03; shape choices S1 to S6 accepted as proposed)

Decision record: [ADR-0273](../../product/architecture/adr-0273-a-broker-submit-wakes-the-convergence-loop-at-once.md).
The user ruled "add it" (2026-11-03) before seeing a shape, then **accepted the shape choices
S1 to S6 as proposed on 2026-11-03** (approval record, ruling 4). Verified against source, the
grounding and primitive hold; this section adds the cancellation argument, names the drain-side
guard type, settles the loop's own submit, and decides the model question. D17 is independent
of D1 to D12; D15's cold-start claim leans on it.

**Is there already an early wake? No, for a submit.** `spawn_convergence_loop`
(`overdrive-control-plane/src/lib.rs:4915-5052 @ 04052ad75`), the only drainer, parks in a
`biased` `select!` over shutdown, `active.next()` (an evaluation completed) and
`clock.sleep(sleep_for)` (`:5022-5049`); `sleep_for` is the cadence (`tick_period_ms`, 100 ms)
or the earliest `NotBefore` eligibility (`:4996-5004`). No arm reacts to a submit and no
`Notify` exists in the loop or `ReconcilerRuntime`. A submit inside an in-flight evaluation
(`enqueue_evaluation.rs:59`; `reconciler_runtime.rs:1767-1773`) is picked up at its completion.
Any other submitter waits for the timer: the interest router (`lib.rs:5105-5113`), the submit
handler (`handlers.rs:75`), the reclamation executor (`reclamation.rs:253-276`); each such hop
costs up to one tick. D17 does not collapse to a clarification.

**Primitive: `tokio::sync::Notify`, `notify_one` (S1).** A call with no waiter stores one permit
that the next `notified().await` consumes at once; repeated calls store one
(tokio 1.52.1, the version `Cargo.lock` pins: `sync/notify.rs:32-49`, `:627-632`).
A submit between the loop's drain and its park is therefore not lost, and a burst coalesces.
`notify_waiters` stores no permit (`:714`) and is rejected. `select!` cancellation: if the
`notified()` future has been woken by `notify_one` and the `select!` completes on another arm,
the dropped future passes the notification to the next waiter or leaves the permit
(`drop_notified`, `:1365-1375`), so a biased `select!` that picks the completion arm does not
swallow a wake. Alternatives: ADR-0273.

**Contract (behaviour; the named surface is below and in the interface table).**

1. A submit takes the broker lock, inserts, releases it, then signals (S2).
2. The `select!` gains a wake arm, after the completion arm and before the timer arm, still
   `biased`. The wake and the timer are alternatives; the loop re-enters its top either way,
   where due resync evaluations are submitted and the broker is drained. The timer is re-created
   every iteration today (completions already reset it); a wake does the same, which cannot
   starve resync or deferred eligibility, since both are evaluated at the top of every iteration.
3. The cadence tick is unchanged and is the level-triggered backstop: a lost or absent wake
   costs exactly today's latency. The timer arm is `clock.sleep` on the injected `Clock`
   (`SimClock` advances it); the wake arm needs no clock and has no simulator-only counterpart.
4. One drainer, one admission order: FIFO by first-pending age over the broker's `BTreeMap`, one
   evaluation per target, the capacity cap, `NotBefore` and the per-key LWW collapse unchanged.
   A wake that finds nothing admissible parks again; the blocked target's completion wakes it.
5. Iterations are bounded by submits + cadence ticks + completions. The loop never submits in
   response to a wake, so no self-perpetuating cycle exists. ADR-0084's no-storm constraint holds:
   resync still routes through the broker and re-arms at most once per period.
6. `EvaluationBroker` (`overdrive-core`) is unchanged and free of `tokio` and clocks.

**Named surface:** the interface table row (all in `overdrive-control-plane`;
`BrokerDrain<'_>` is a new guard newtype over `parking_lot::MutexGuard<EvaluationBroker>`). The
submit handler, the self-re-enqueue, `execute_reclaim_allocation` and the `EnqueueEvaluation`
dispatch all submit through `submit_evaluation`, so a submit that forgets to wake is not
expressible. The loop's own resync submit (`:4953-4960`) goes through it too, before the loop takes
the drain guard (the lock is not re-entrant); the cost is one extra cheap iteration per due
resync (S3).

**Shape choices (all accepted as proposed by the user, 2026-11-03).** S1 `Notify::notify_one` (rejected: shorter cadence, producer-side
drain, capacity-1 `mpsc`, `watch` on queue size, `notify_waiters`). S2 signal after the lock, never
before the insert. S3 one submit path including the loop's own resync (rejected: a loop-private
submit that does not wake, which keeps two paths). S4 enforce the single path by type
(`BrokerDrain` without `submit`; rejected: convention, `broker().submit` stays reachable). S5
`submitted` is `pub(crate)`; `submit_evaluation` and `BrokerDrain` are `pub` because tests submit
through the runtime. S6 no Quint model (below).

**Lifecycle Gate Ownership: Not applicable.** D17 adds, removes and moves no gate; the wake only
starts an iteration earlier, and the iteration (resync, drain, admit) is the same function of the
broker as before. `NotBefore` still decides admission.

**Formal protocol model (decision S6): not required.** The handoff is the classic lost-wakeup
shape (submit between drain-empty and park). It is closed by the primitive's documented
semantics (stored permit, cancel-forwarding above), a model could only assume them, a lost wake
costs only the unchanged tick latency, and the park/drain interleaving is internal to one private
task with no owner accessor, so a conformance driver has nothing to project. A hazard
`wait_without_permit` would model the rejected `notify_waiters` alternative, which
`.claude/rules/design.md` forbids as a check. The evidence is S-48 (seeded production loop under
`SimClock`) and mutation of the signal's position and primitive. Nothing is added to the modeler
change list.

**Contract shapes.** `submit_evaluation`: bounded-change (the pending map and one permit; assert
via `counters()` and admission without a clock advance). `submitted`: consumes the permit only.
`BrokerDrain`: drain side unchanged. **Operator-visible effect:** latency only; no CLI, wire,
persisted or config change.

**DISTILL obligation.** S-48 and the D17 mutation targets (§ Executable obligations); DISTILL
reviews existing tests that assume nothing drains until `SimClock` advances
(`runtime_convergence_loop.rs`, `interest_router.rs`, `cadence_resync.rs`, and every test calling
`broker().submit`).

## Wave: DESIGN / [REF] Decisions D13 to D16 — guest-reported listener readiness

> Authored by Morgan (nw-solution-architect), 2026-11-03; accepted after
> independent full DESIGN review iteration 4 on 2026-11-09. Scope: all
> priorities. Origin: the user's ruling of 2026-11-03, "Drop inference and take readiness
> from the guest itself". **Status: accepted; V41 and V42 held. The rulings
> PR-1 to PR-4 are recorded in § Approval record, the one home for their text, with P1 (the
> row counter and the stop gate's attempts threshold, D14 item 3), A1 (the board's entry
> lifetime, D13 item 10), A3 (the host-input conformance projection), R1 and R2 (§ Formal
> protocol model).** ADR-0058's inferred-TCP behavior remains operative until
> the accepted ADR-0270 replacement is implemented. Independent DISTILL and
> roadmap approval remain necessary before DELIVER. Decision records:
> [ADR-0272](../../product/architecture/adr-0272-the-guest-observes-workload-listeners-with-kernel-hooks-not-a-poll.md) (D13, observation),
> [ADR-0268](../../product/architecture/adr-0268-the-guest-reports-each-workload-listener-it-opens-over-the-beacon-channel.md) (D13, report),
> [ADR-0269](../../product/architecture/adr-0269-a-service-with-no-declared-startup-probe-waits-on-a-guest-listener-startup-probe.md) (D14),
> [ADR-0270](../../product/architecture/adr-0270-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md) (D15),
> [ADR-0271](../../product/architecture/adr-0271-the-probe-descriptor-no-longer-records-whether-it-was-inferred.md) (D16).
> Repo citations are as read on 2026-11-04 at the branch head. Kernel citations are
> `path:line @ v7.0`.

### Step 05-06 legacy guest-flow verification disposition (2026-11-11)

The independent bounded [DESIGN adjudication](design/guest-flow-abort-contract-adjudication.md) and its [iteration-2 review](design/review-guest-flow-abort-contract-adjudication.md) establish two **pre-05-06** legacy failures unrelated to D13's listener hook → ordered `LISTEN` beacon → host board → startup-row path. The user said, **“these are being fixed in another session”**, then directed, **“no, u dont need to wait for anything.”** For this feature's step 05-06 only, the two known baseline failure modes, represented by three test identities, are nonblocking external dependencies: `overdrive-init::bin/overdrive-init guest_flow::quint_fwd_pair::quint_fwd_pair_pair_conf_guest` for the known accepted-but-unattached watch-mode `GuestAbort(g)` mismatch across seeds (reproduced at `QUINT_SEED=0x9ab8de37` and `0xaddef4a3`), and the D-22 one-second `every burst flow ended` assertion under either `overdrive-init::integration integration::in_guest_lane::a_burst_of_host_aborts_resets_every_connection_in_a_real_guest` (outer native wrapper) or `overdrive-init::bin/overdrive-init guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection` (the same source-local cell: `infra/metal/test-placement.json:10560-10563` routes it through the outer fixture via `infra/metal/test-wrapper.sh:212-242`; `in_guest_lane.rs:425-429`, `real_kernel.rs:1900-1948`). The direct identity failed in the serial 211-test 05-06 run (210 passed) recorded in `.context/step-05-06-init-suite-final.log:33-42`; the retained native baseline provides the precise one-second/78-of-1,000 assertion evidence. The step review records each failed identity and its retained baseline evidence; it does not call a failing test green. A different assertion, failure mode or test is outside this ruling; another seed of this same T-5 failure is covered. Every other existing guest-flow and beacon test, both D13 native real-boot outcomes, guest-kit preparation, BPF build and verifier-budget gate remains required. The older `netns-density-295` T-5 and R5-31 production requirements and its two pending validation scopes remain with the other session; this current-feature ruling changes no production/model contract and does not wait for the external fixes. If those fixes land, their independent evidence can revalidate the named identities without retroactively claiming this step's failed runs passed.

### Step 05-06 guest BPF verifier evidence mapping (2026-11-11)

The [bounded evidence adjudication](design/guest-verifier-evidence-adjudication.md) pins three distinct claims under D13 and the existing Tier-4 rules. The **2/2 native real-boot tests** prove that production guest setup loads and attaches `listen_tcp`, `post_bind4` and `post_bind6` from `overdrive_guest_bpf.o` and exercises their listener outcomes; a successful `bpf-build` proves only compilation. The current `verifier_regress` binary reads `overdrive_bpf.o` and the dataplane baseline directory, so its unchanged pass proves only the existing dataplane budget. Step 05-06 also requires a **quantitative guest budget** for the three new programs: the existing verifier gate's measured corpus must include each guest-object program's `verified_insns` on the pinned Linux 7.0 kernel, each ≤500,000 under `.claude/rules/development.md` Tier 4, with initial reviewed baselines and the gate's existing later-growth policy. V37's 750 count was for the rejected merged TCP arm and is not a baseline for the accepted separate `listen_tcp`. The same canonical exclusive verifier invocation remains pending while another checkout holds the live Metal shared lock; a timeout before START and a dataplane-only pass cannot discharge the guest-count requirement. No new public API, CLI command, test lane or D13 behavior is introduced by this evidence clarification.

### Premise: what the guest is in production (ground-the-premise)

`overdrive serve` + `overdrive deploy` of a Service VM (the only Service driver:
`WorkloadDriverV1` has the single variant `Vm`, `aggregate/mod.rs:156-159`).

- **Guest kernel.** Linux; kernel and rootfs are the operator's (`[vm] kernel` / `rootfs`,
  `vm/config.rs:136-198`; ADR-0206 calls them an interim until #259). ADR-0068 pins the
  appliance guest kernel at 7.0 as a design target, but the repo has no guest `.config` or
  build recipe. The Tier-3 fixture boots the metal host's distro kernel
  (`overdrive-testing/src/vm_fixture.rs:85-115`). Kernel claims are pinned to 7.0 source and
  to the `uname -r` the spikes record.
- **Guest init.** `overdrive-init` is PID 1 (`src/main.rs`): mount procfs; load vsock
  modules; dial the host beacon; parse `overdrive.transport=`; configure the guest
  transport, load the guest eBPF and start the flow-owner thread, all **before** `READY`;
  send `READY`; receive one `EXEC`; exec the workload (`main.rs:738-740`, a process group,
  no cgroup of its own); on exit send `EXIT <status>`, honour one `SHUTDOWN`, power off.
  The guest eBPF (ADR-0150) is seven programs (`transport/mod.rs:419-436`): a sockops
  program (`tcp_correlation.rs:269-359`, no `TCP_LISTEN_CB` arm), an `fentry` program,
  `cgroup/sock_addr` connect4/sendmsg4/getpeername4/recvmsg4 and a `sock_release`
  `CgroupSock` program, attached `Single` at the cgroup2 root (`transport/kernel.rs:41-42,
  162-173, 300-346`). **Nothing in the guest observes `listen()` or `bind()` today.**
  Init itself opens inet sockets before `READY`: `configure_guest_transport` binds the TCP
  intake port (61000) and 1,024 UDP slot ports at `workload_addr`, a non-loopback address
  (`transport/mod.rs:227-231`, `transport/kernel.rs:271-290`). They are in the same cgroup
  (the root) as the workload, and the declared-listener validation rejects the reserved
  range (`aggregate/mod.rs:500-511`), so they can never satisfy a declared listener, but
  hooks that see them would emit about 1,025 reports before `EXEC` (D13 item 1, PR-2).
- **Channels (virtio-vsock, NIC-less guest).** Beacon, guest dials host, default 1234, text
  lines `READY`/`EXEC`/`EXIT`/`SHUTDOWN` (`vm/beacon.rs:185-224`;
  `overdrive-host/src/beacon_io.rs:104-125`). Flow stream 1240 and control 1243, guest
  dials host. Inbound 1242, host dials the guest (probes, leg-S relay). Ports are node
  config (ADR-0218); the guest learns them from the cmdline token (ADR-0217; seven fields,
  an eighth is rejected, `transport/mod.rs:96-140`).
- **How the host learns from the guest today.** `READY` (`vm_driver.rs:1540-1740`); `EXIT`
  (drained once after the VMM exits, `vm_driver.rs:2171`, `:2208-2222`); the console tail
  for diagnostics; and probes, which dial the guest over vsock 1242 (`probe_tick`,
  `probe_runner/mod.rs:513-640`). A dial to an undeclared or UDP port is `NotServing`
  because `listen_ports` is TCP-only (`aggregate/mod.rs:926-940`): **a UDP listener cannot
  be probed at all**. The beacon reader forwards every guest line into a bounded
  `mpsc::channel(1)` drained only after exit (`vm_driver.rs:755-835`, `:580`): a mid-life
  guest line would block the reader and be misread as the exit report. D13 pins the fix.
- **Call sites of the state under change.** Inference: `parse_startup_probes`
  (`workload_spec.rs:1157-1211`; `listeners[0]`, TCP, `0.0.0.0`, 5/2/30, `inferred: true`);
  API ingress `Service::from_submit` does not infer, so a raw submit with an empty list is
  the opt-out. Reconciler: opt-out branch (a') and `Stable` branch (a)
  (`service_lifecycle.rs:504-558`), deadline `interval x max_attempts` (`:758-770`,
  `DEFAULT_STARTUP_DEADLINE = 60 s`, `:359`). Shim order: `Running` row
  `action_shim/mod.rs:2535`, `release_for_exit_emission` `:2679`, `on_alloc_running` `:2686`
  (D3 swaps the last two).
- **#303 (in-guest kernel mTLS).** Open research; separate hooks, same init; no dependency
  either way.

### D13 — the guest observes each workload listener with kernel hooks and reports it over the beacon channel

Two independently reversible decisions: ADR-0272 (the observation) and ADR-0268 (the report).

**The hook (in-guest kernel mechanism, evaluated first per design.md).**

TCP: sockops `BPF_SOCK_OPS_TCP_LISTEN_CB`, once per entry to `TCP_LISTEN`. UDP: `CgroupSock`
post-bind4/6 on an explicit `bind()` (autobind skipped, correct for a server). The kernel
citations are in ADR-0272 § Context; the program shape is item 7.

Rejected alternatives (`fentry`, `sock_diag` polling, workload-signalled readiness): see ADR-0272.

**Contract.**

1. **Guest side (behaviour, not structure).** Before `READY`, init has attached the listen
   hooks (scope: **PR-2**, below). For every distinct `(protocol, port)` for which
   a socket **of the workload** enters `TCP_LISTEN` or completes an explicit UDP bind, init
   sends exactly one beacon line. Reports are not filtered by the declared set (the guest
   does not know it) and carry no process identity. A socket bound only to a loopback
   address is not a report (the workload is dialled at `workload_addr:port`, so a
   loopback-only bind does not serve the declared listener); both hooks read the local
   address (V34, V35). Items 8 and 9 pin the address and drop rules.

   **PR-2: the listen hooks attach to the workload's cgroup, not the cgroup2 root, so init's
   own sockets are structurally invisible.** (Ruling: § Approval record. Held by V42, § Spike
   table, which is the one home for the outcome numbers.) Evidence: init's pre-`READY`
   transport binds 1 TCP intake and 1,024 UDP slots at `workload_addr` (premise above); with
   the hooks at the root, each is an event, so init would send about 1,025 `LISTEN` lines
   before `EXEC` and press the ring and the pre-`READY` reader for nothing the host uses.
   The workload and init share the root cgroup today (`main.rs:738-740`, a process group
   only). A socket belongs to the cgroup of the task that creates it (`cgroup_sk_alloc`,
   `kernel/cgroup/cgroup.c:7408-7429 @ v7.0`), the sockops and `CgroupSock` programs run in
   the socket's cgroup (`kernel/bpf/cgroup.c:1639-1646`, `:1721-1730 @ v7.0`), and a
   program attached to a child cgroup runs for its sockets only. Behaviour: init creates one
   workload cgroup before `READY`, attaches the three listen programs to it, and starts the
   workload (and so every descendant) inside it (P-c). The existing seven programs stay at the
   root (P-b); the effective program array of the child includes the root's
   (`compute_effective_progs`, `kernel/bpf/cgroup.c:432-490 @ v7.0`; a parent's programs are
   included when flagged `BPF_F_ALLOW_MULTI` or when the child has none of that attach type,
   `:443-447`, `:458-484`), so correlation and the connect/sendmsg hooks keep seeing the
   workload's sockets.
   Consequence for item 7: the TCP arm is not merged into the existing root-attached
   sockops program (it would fire for init's intake listen); it is a separate small sockops
   program, in the same object, attached to the workload cgroup. Alternatives: (a) filter the reserved port range
   in the programs (keeps the merged arm, two constants compared; init's sockets stay
   visible and correctness rests on the range constants, and a future init socket outside
   the range would report); (b) attach the hooks after `configure_guest_transport` returns
   and before `READY` (zero cost; invisible only by ordering, and any socket init binds
   later would report; hooks at the root attached after the binds were also silent, 50 of
   50, so the cgroup's value is order-independence); (c) filter by PID in-kernel (a
   heuristic, rejected).

   **Contract pinned by V42 (binding on DELIVER; evidence and cost: § Spike table V42 row and
   item 6):**
   - **P-a. One object.** The three listen programs live in the guest eBPF object
     `overdrive-init` already loads. A second `Ebpf::load` of a separate object is not
     permitted: it costs about 20 ms on the pre-`READY` path (V42 row).
   - **P-b. Root programs remain link attachments.** The existing root programs stay
     `BPF_F_ALLOW_MULTI` link attachments (aya `bpf_link_create`, `kernel/bpf/cgroup.c:1506-1507
     @ v7.0`), so they stay effective in the workload cgroup. A legacy `BPF_PROG_ATTACH`
     `Single` without `BPF_F_ALLOW_MULTI` would drop the root sockops program for a child that
     has its own sockops program (`kernel/bpf/cgroup.c:459 @ v7.0`). Only sock_ops has this
     interaction (post-bind are different attach types).
   - **P-c. No placement window.** The workload is created directly in the workload cgroup
     (atomic placement, `CLONE_INTO_CGROUP`; the kernel claim and its citations are the V42
     row's). No workload instruction runs outside the cgroup, and every descendant is inside it.
   - **P-d. Scope and fail-closed.** Reports cover only sockets created by tasks in the
     workload cgroup (mechanism above; accepted
     children inherit, `net/core/sock.c:2538 @ v7.0`). A workload that moves itself out of the cgroup
     is never reported, so its declared listeners are never satisfied and the service is
     never `Stable` (it is stopped at the startup deadline, D14).
   - **DELIVER note (crafter-level, not an interface contract).** `clone3` replaces
     `Command::spawn` for the workload. An exec failure then shows only as exit status 127
     where `InitError::Spawn` is returned today; a missing workload binary was not exercised
     by V42, so the crafter pins the exit-127 versus `InitError::Spawn` mapping.

   Two guest-side ordering rules are pinned by the `guest-signal` spike (V36, V38;
   `spike/findings-guest-signal.md`):
   - **One ordered beacon writer.** Every beacon write (`READY`, `LISTEN`, `EXIT`) goes
     through one owner: `overdrive-init::send`'s exclusive `&mut File` stays the only
     write path, and no forwarder thread is a second writer. Two unsynchronised writers
     corrupt the line stream (10 malformed lines and 6 lost `EXIT`s per 200 000 at
     saturation); one ordered writer, or one lock held across each whole-line write, had
     none.
   - **Drain before `EXIT`.** After `waitpid`, init drains the listen event ring and sends
     every pending `LISTEN` before it sends `EXIT`. A `LISTEN` therefore never follows
     `EXIT` (0 of 1000 boots; 185 of 300 without the drain).
2. **Wire (new public surface, named).** `overdrive_core::vm::beacon::BeaconMessage` gains
   a guest-to-host variant `Listen { protocol: Proto, port: NonZeroU16 }`; text form
   `LISTEN <tcp|udp> <port>\n`; `FromStr` requires exactly three fields, a non-zero port and
   a known protocol, and returns the existing typed parse error otherwise (no panic).
   `Proto` is the existing `dataplane::backend_key::Proto`. No change to `READY`, `EXEC`,
   `EXIT`, `SHUTDOWN` or the cmdline token.
3. **Host classification (behaviour pinned; structure is the crafter's).** The host reads
   the allocation's beacon lines as they arrive and routes each by kind: `Listen` to the
   allocation's reported set (item 10), `Exit` to the exit report. Only `Exit` or EOF ends
   the guest-report wait; the wait keeps reading past every other line. A mid-life line
   never blocks the reader, the stop path or the command path (today the capacity-1 line
   channel is drained only after exit, so a mid-life line would stall them and be taken as
   the exit report). A `Listen` line never occupies the exit report and cannot be read as
   it. An unparseable line, and a `Listen` line that arrives after the wait ended, is
   dropped with a structured warn event naming the allocation; neither fails the driver nor
   changes the exit report or the set. Reports for undeclared `(protocol, port)` are not
   stored (memory is O(declared)).
4. **Ordering.** The guest cannot report before the workload runs; the workload runs only
   after `EXEC`; `EXEC` is written by `release_for_exit_emission`, which D3 places after
   the `Running` row write and after `on_alloc_running` (supervisor registration). So a
   report never precedes the `Running` row or the supervisor. The reported set is created
   when the VM starts, before the beacon reader runs, so no report precedes it (item 10).
5. **Idempotence and lifetime.** Duplicate lines are no-ops. The set belongs to one VM
   incarnation. A guest reboot or crash ends the VMM (CLAUDE.md, "a guest reset ends the
   VMM"); init runs the workload once; a restart is a fresh allocation id and a fresh set
   (ADR-0105). There is no close report and no un-ready transition.
6. **Cold-start cost (V41, held).** On a real Cloud Hypervisor guest (`7.0.0-38-generic`,
   1 vCPU, 256 MiB, 40 interleaved boots per variant), attaching the listen programs before
   `READY` adds **+0.36 ms at p50** to init-start-to-`READY` against a same-toolchain
   baseline (paired p90 +1.29 ms, max +2.10 ms; the +20 ms falsification bar was missed by
   about 50 times). The platform already spends about 117 ms (p50 116.8) of BPF work before
   `READY`. The cost is spent on every first boot; the repository has no snapshot-resume
   path (GH #96, #97, #100, #43), so no resume latency is affected. It buys the ordering that
   no listener is missed: attaching after the workload ran reported 0 of 50, so the attach
   cannot move after `EXEC`. Findings: `spike/findings-boot-cost.md`, verdict PARTIAL on one
   point only (the guest PID 1 is an instrumented copy of the real init, equivalent to it by
   a host-side control, A minus R -1.1 ms). **PR-2 shape cost:** the design (programs in the
   existing object, attached to the workload cgroup) adds **+0.88 ms at p50** over the same
   object without them (paired p90 +1.66 ms, max +2.49 ms); the breakdown, the hazard behind P-a
   and what was not measured are in the V42 row of § Spike table. Budget for what D13 adds to
   the pre-`READY` path: about +0.9 ms p50 (+1.7 ms p90), against about 117 ms of existing BPF
   work.
7. **Program shape.** aya attaches through `bpf_link_create`, which always sets
   `BPF_F_ALLOW_MULTI` (`kernel/bpf/cgroup.c:1506-1507 @ v7.0`), so a second sockops attach
   does not replace the first: both coexist (V37, M1). **Shape (PR-2, item 1):** the TCP arm
   is a separate small sockops program in the same object, attached to the workload cgroup;
   the three listen programs ride in the object init already loads. UDP needs **two** `CgroupSock`
   programs, post-bind4 and post-bind6: `src_ip4` and `src_ip6` are readable per attach type
   only (`net/core/filter.c:8976-8989 @ v7.0`). Behaviour pinned: both return 1 (a 0 fails
   the workload's `bind()`, `af_inet.c:549-557 @ v7.0`); both ignore non-UDP binds (TCP
   `bind()` reaches them).
8. **What a report means: the address rule and IPv6.** `IPV6_V6ONLY` is not observable on
   either hook (`bpf_getsockopt(SOL_IPV6)` returns -EINVAL in sockops,
   `net/core/filter.c:5562-5573 @ v7.0`; V34, V35: `V6ONLY=1` and dual-stack wildcards
   produce identical reports). The guest does not hold `workload_addr`, so the only address
   exclusion is loopback: `127.0.0.0/8`, `::1` and the IPv4-mapped loopback
   `::ffff:127.0.0.0/104`; a bind to any other address is a report. The v4-mapped range is
   decidable from the same four words of the IPv6 local address both hooks already read
   (`src_ip6` for post-bind6, `local_ip6` for sockops): words 0 and 1 zero, word 2
   `htonl(0x0000ffff)` (the constant as the network-order word the hooks read, bytes `00 00 ff ff`;
   the host-order value `0x0000ffff` compared against a network-order word would never match on
   a little-endian guest), and the first byte of word 3 in memory order (the first address
   octet) equal to 127. Word 3 is a network-order word read on a little-endian guest, so that
   byte is the low byte of the loaded `u32`: the test is the explicit mask `word3 & 0xff ==
   127`, not a host-order shift or compare. Kernel grounding (`vendors/linux @ v7.0`): the UAPI
   declares both fields `__u32 [4]`, not `__be32` (`struct bpf_sock_ops`, `local_ip6[4]`
   "Stored in network byte order", `include/uapi/linux/bpf.h:6909`; `struct bpf_sock`,
   `src_ip6[4]`, `bpf.h:6450`, no byte-order comment), so the network order is a documented
   convention, not a type the compiler checks; the context conversion loads the words from
   `skc_v6_rcv_saddr.s6_addr32[]` (`net/core/filter.c:10761-10775` for sockops,
   `:10223-10226` for `bpf_sock`), and `s6_addr32` is `__be32 u6_addr32[4]`
   (`include/uapi/linux/in6.h:38,44`). An `AF_INET6` socket bound to
   `::ffff:127.0.0.1` is reachable only on loopback, so reporting it would satisfy a declared
   listener the workload cannot serve. The spikes exercised `127.0.0.1` and `::1` only; the
   v4-mapped case is a Tier-3 scenario in G-10.
   **A report `LISTEN <proto> <port>` therefore means "a socket of that protocol is bound
   to that port on a non-loopback address"; it makes no claim about IPv4 reachability.**
   **Dual-stack, recorded against GH #308 (user ruling 2026-11-03: Overdrive does not support
   IPv6 yet; orchestrator comment
   https://github.com/overdrive-sh/overdrive/issues/308#issuecomment-6614793072).** Rule, for
   the declared listener (which has no address family): a report from an `AF_INET6` wildcard
   **does** satisfy it, so a listener declared for IPv4 is satisfied by a workload that binds
   `[::]` (a Go or Java `":8080"` listener, dual-stack by default). Refusing it would fail the
   common case, and a `V6ONLY=1` socket cannot be told apart. Exposure, stated: a workload that
   sets `IPV6_V6ONLY=1` (or listens only on a specific IPv6 address) reaches `Stable` while
   IPv4 peers are refused; it is bounded to that explicit choice, the retired probe would have
   caught it, and a declared startup or readiness probe still decides for a Service that
   declares one. The rule and the exposure are #308's to revisit when IPv6 is supported.
9. **A dropped report.** The event ring drops and counts when full; the workload is not
   slowed (`seen == delivered + drops` in all 12 flood runs, V37). A dropped `LISTEN` for a
   declared listener leaves the service never `Stable`, so it is stopped at the startup
   deadline (60 s) and restarted, ending in `StartupProbeFailed` naming a listener the
   workload did open. Decision (user ruling 2026-11-03, no level map): **accepted as a
   stated, bounded exposure, not a fallback.** Only TCP listen transitions and explicit UDP
   binds produce events (`accept()`, connect and autobind produce none: V34 T11, V35 U5,
   U6), and a workload opens a handful of listeners. Reaching the exposure takes more such
   events than the ring holds between two drains, with the declared listener's among the
   dropped. The crafter sizes the ring generously; the drop counter stays visible (a
   guest-readable map; no wire or host surface is added). A lost report stops the service at
   the startup deadline; it does not hang it. Under PR-2 init's own binds produce no events,
   so the ring carries only workload events.
10. **The reported set and its read seam (PR-1).** The reported set is owned by a
    host-side board in `overdrive-worker`; the probe task reads it through a read-only port.
    The `Driver` trait is **unchanged**: the probe runner is owned by `VmDriver` (the shim
    holds `dyn Driver` and never reads the set), and both the beacon reader (the writer) and
    the probe runner (the reader) live in `overdrive-worker`.

    Port (new, `overdrive-core::traits`, read-only; contract in core, impl in the adapter
    crate, per `reconcilers.md` § "The hydration boundary"):

    ```
    #[async_trait]   // held as Arc<dyn GuestListenerReports>, as GuestStreamDial is
    trait GuestListenerReports: Send + Sync {
        fn reported(&self, alloc: &AllocationId) -> ReportedListeners;
        async fn changed(&self, alloc: &AllocationId, seen: &ReportedListeners)
            -> ReportedListeners;
    }
    ```

    `ReportedListeners` is a value type: an ordered set of `(Proto, NonZeroU16)` with
    `contains`, `len`, `is_empty` and equality, and constructed through `Default` and
    `FromIterator<Listener>` (DG-6; every composition and oracle builds values this way).
    `GuestListenerBoard::new() -> Self` is the board's constructor. Behaviour (the trait docstring carries it; DISTILL
    scenarios S-50 to S-52 pin each clause):

    **D13-S1, exact public signatures (user-approved 2026-11-09):**

    ```rust
    impl VmDriver {
        pub fn new(
            vmm: Arc<dyn Vmm>,
            clock: Arc<dyn Clock>,
            fs: Arc<dyn CgroupFs>,
            cgroup_accounting: Arc<dyn CgroupAccounting>,
            probe_runner: Arc<ProbeRunner>,
            guest_network_exec: Arc<GuestNetworkExecGate>,
            layout: VmHostLayout,
            beacon: BeaconIntake,
            guest_listener_board: Arc<GuestListenerBoard>,
        ) -> Self;
    }

    impl ReportedListeners {
        pub fn contains(&self, protocol: Proto, port: NonZeroU16) -> bool;
        pub fn len(&self) -> usize;
        pub fn is_empty(&self) -> bool;
    }
    ```

    The board is the ninth and last mandatory constructor argument; the first
    eight retain their existing order and types. `contains` takes the pair's
    constituents by value, `len` counts distinct pairs, and `is_empty` is
    equivalent to `len() == 0`. `Default`, `FromIterator<Listener>`, equality,
    the read-only `GuestListenerReports` trait signatures, and the sixth
    `ProbeRunner::new` parameter remain as already approved. The board's
    storage and private write-side methods remain implementation choices.
    - **Entry lifetime (one contract; A1, accepted 2026-11-04).** An allocation id has an entry from `open` to `close`
      and none before or after. `reported` on an id with no entry returns the empty set; there
      is no frozen-after-close read, because no production reader exists after close (the
      supervisor is cancelled first, below) and a retained entry per past allocation would grow
      without bound across restarts. While the entry exists, the set only grows and holds only
      pairs the Service declares. *Frozen at the end of the guest-report wait* is the host
      reader's rule (item 3: it records nothing once `Exit` or EOF ended the wait), not the
      board's; the entry stays readable until `close`.
    - `reported` is a level read: it returns the current set, never blocks.
    - `changed` completes with the current set as soon as it is not equal to `seen`. If it
      already differs when called it completes at once, so no insertion between a `reported`
      and a `changed` is lost. Otherwise it waits for an insertion; completion is on the
      insertion itself, not on a timer, so a `Pass` is written one beacon delivery plus one
      task wake after the guest's `listen()`. A duplicate, an undeclared pair or a report for
      another allocation does not complete it. It is cancel-safe. For an id with no entry it
      does not complete (the caller's supervisor token cancels it); a waiter pending when the
      entry is closed is not woken.
    - **Ordering with the probe task.** `VmDriver::on_alloc_terminal` calls
      `ProbeRunner::stop_alloc` and only then `close`. `stop_alloc` is synchronous and does
      **not** await the probe task's exit: it removes the supervisor and cancels its tokens
      (`probe_runner/mod.rs:374-379`). So a task never reads a closed entry (its token is
      cancelled before `close`), but a `Pass` write the task had already issued may still land
      after the exit event. The guarantee is therefore *no `Pass` write is started after the
      task observed cancellation*, and a `Pass` that lands after the exit event is inert:
      `Stable` needs the row `Running` (B1, D5) and the exit observer writes the terminal row
      after the hook (D1). The task re-checks its token immediately before issuing each row
      write.

    Writer (concrete, `overdrive-worker`, not on the port; **internal structure, the
    crafter's**): the board implements the port and is fed by the beacon reader. Pinned
    behaviour only: the entry is created by `VmDriver::start` before the beacon reader runs,
    from `AllocationSpec.declared_listeners` (D14 item 2); an id is opened once (ids are never
    reused, ADR-0105); recording is the check-and-act (`rust.md` § TOCTOU): one locked operation
    reports whether it inserted, and `changed` waiters wake only on an insertion; a duplicate,
    an undeclared pair, an id with no entry, and a record after close are no-ops and complete
    nothing; the entry is removed by `VmDriver::on_alloc_terminal` after `ProbeRunner::stop_alloc`,
    and on any failure of `start`, so closing can never produce a `Pass`. The open-before-reader
    and close-after-`stop_alloc` ordering is lifecycle behaviour, not a signature.
    **`reported` is the only read the conformance driver uses** (§ Formal protocol model).

    Composition: `ProbeRunner::new` gains a sixth mandatory parameter `Arc<dyn
    GuestListenerReports>`; `VmDriver::new` gains the `Arc<GuestListenerBoard>`; the worker
    composition root builds one board and passes it to both (a missing wiring does not
    compile, `development.md` § "Port-trait dependencies"). **DELIVER obligation (DG-6):**
    production composition shares ONE board between the probe runner and `VmDriver`. The
    DISTILL scaffolds create one board per consumer, which compiles and is inert, so the type
    system does not catch it, and the DELIVER reviewer checks the composition root for it. The simulation adapter
    `SimGuestListenerReports` (`overdrive-sim`) implements the port with scripted insertions
    (its scripting API is DISTILL's); `SimDriver` is unchanged, because DST drives
    `ProbeRunner` directly as it does with `SimGuestStreamDial`.

    **The declared set reaches the guest-listener task through the existing
    `ProbeRunner::start_alloc(&self, spec: &AllocationSpec)` parameter** (`probe_runner/mod.rs:307`;
    `VmDriver::on_alloc_running` passes the same spec, `vm_driver.rs:2057-2058`): the task
    clones `spec.declared_listeners` (PR-4) at spawn, as the dial tasks clone their
    descriptors, and tests `declared ⊆ reported` against it. No `start_alloc` parameter is
    added and the port has no `declared()` read; the board's declared filter and the task's
    completeness test read the one `AllocationSpec` field, so they cannot drift.

    Alternatives: (a) a `Driver` trait method returning the set: the shim does not consume
    it and the probe runner is inside `VmDriver`, so it would widen a trait for no caller;
    (b) an argument on `ProbeRunner::start_alloc`: works, but optional by construction, so a
    wiring that forgets it compiles and the sim seam differs from `GuestStreamDial`;
    (c) an observation row per reported listener (ADR-0269): a new family, envelope and wake
    for the same fact. Consequence: one new core trait, one `ProbeRunner::new` parameter,
    one `VmDriver::new` parameter, no `Driver`, `Action` or reconciler change. The model
    accessor for the reported set is `GuestListenerReports::reported` (§ Formal protocol model
    — D13 to D16).

**DELIVER notes (not contract).** `local_ip6` is copied element by element (the verifier
rejects an array copy through a modified ctx pointer). The ring size is the crafter's; the
spikes saw no drops at 4 MiB under a 20,000-socket flood and drops, counted, at 4,096 bytes;
V41 used 1 MiB.


### D14 — the reports feed the startup probe row; the startup gates carry over unchanged

Decision record: ADR-0269.

1. **New probe mechanic (new public surface, named).** `ProbeMechanic` gains a fieldless
   tail variant `GuestListeners` (the D7 `StoppedBy::StartupProbe` precedent). It is valid
   only as the sole startup descriptor of a Service (`from_submit` and the parser reject it
   elsewhere), cannot be written in TOML (only D15's default rule produces it), and carries
   no listener data (that would persist derived state). D14-E2 below pins the
   exact typed rejection at each applicable ingress.
2. **Declared listeners reach the probe runner (PR-4: one field, not two).**
   `AllocationSpec` gains `declared_listeners: Vec<Listener>` (the existing
   `workload_spec::Listener { port: NonZeroU16, protocol: Proto }`), the full declared set in
   declaration order, projected in `hydrate_desired` from the same live intent. In-memory
   only, no serde or rkyv. Today `AllocationSpec.service_ports: Vec<NonZeroU16>` (also
   `WorkloadLifecycleState.service_ports`) is a TCP-only projection of the same declaration
   (`project_service_listen_ports` -> `Service::listen_ports()`), consumed by the inbound
   TPROXY rule (`mtls_intercept_worker.rs:2723, 2747`) and the inbound-rule helper
   (`action_shim/mod.rs:1522-1525`). **Decision: `declared_listeners` replaces
   `service_ports`**, in both places it lives: `AllocationSpec.service_ports` and
   `WorkloadLifecycleState.service_ports` (`workload_lifecycle.rs:1924`, cloned into every
   emitted spec at `:1328` and `:1612`) are both replaced by `declared_listeners`, projected
   once in `hydrate_desired` (`project_service_listen_ports`, `:590` and `:2053`, becomes the
   declared-set projection; no TCP-only projection remains in `WorkloadLifecycle`).
   `AllocationSpec` gets one read accessor for the TCP view
   (`pub fn tcp_service_ports(&self) -> Vec<NonZeroU16>`, D14-A1, user-approved
   2026-11-09), which those two consumers call, so the TCP set is a filter of the
   full set by construction and the two cannot drift. Alternative: keep both fields, each
   projected separately in three `WorkloadLifecycle` construction sites; rejected because
   nothing then ties them together, and a Service edit that changes one projection and not
   the other breaks TPROXY or the gate silently. Cost: a mechanical change at `service_ports`'s
   213 matching lines across 72 files under `crates/` (grep at 2026-11-04; almost all are tests
   that set `service_ports: Vec::new()`, and they include the hydration golden fixture
   `hydration_golden/workload_lifecycle.txt` and the sim trajectory fixture
   `workload_lifecycle_trajectory.txt`, which are regenerated, not edited by hand), two production
   consumers, one accessor; an unfiltered future consumer would install a TPROXY rule for a
   UDP port, which the accessor makes the only TPROXY-facing read.
3. **Producer (PR-3: no `Fail` row before the deadline; P1, accepted 2026-11-04: the row's
   counter is honest and the stop gate keys on the deadline for this mechanic).** For a
   `GuestListeners` descriptor the allocation's probe-runner task (registered by
   `start_alloc`, cancelled by `stop_alloc`, ADR-0054/0256) does not dial. It waits on the
   reported-set port (item 10 of D13) and writes **ordinary startup `ProbeResultRow`s through
   the same per-(allocation, probe) run state as the dial task** (`ProbeRunState`,
   `probe_runner/mod.rs:666-694`: `next_run`, `next_streak`, `record_written`), so
   PROBE-RUN-COUNT holds unchanged: `run_seq` counts every written row and
   `same_status_runs` is the count of consecutive written rows of the row's status (a row
   whose write failed advances neither). The rows:
   - **one `Pass` row at once** when the reported set contains every declared
     `(protocol, port)` (event-driven; `same_status_runs` is 1, the first written row of its
     status); a failed `Pass` write is retried until it lands;
   - **the first `Fail` row at the derived startup deadline** (the parser fills the
     explicit-probe defaults, `interval_seconds` 2 and `max_attempts` 30,
     `workload_spec.rs:1139-1141`, so `interval x max_attempts`, `service_lifecycle.rs:758-770`,
     and D10's `startup_deadline` are unchanged code and give 60 s), with `same_status_runs`
     1 and the reason `guest listeners not open after <n>s: <missing, e.g.
     tcp:8080 udp:5353>`; then **one further `Fail` row per `interval_seconds`** (the
     `same_status_runs` rising 2, 3, …) while the allocation is supervised and no `Pass` has
     been written. Every `Fail` is a new row, which is what D8 needs to retry a stop that did
     not land. No `Fail` is written before the deadline.

   **P1 — the stop gate for this mechanic (Accepted 2026-11-04, option (a)).** `ServiceLifecycle` reads the counter
   as `attempts` = the latest startup `Fail` row's `same_status_runs`
   (`service_lifecycle.rs:607-611`) and stops when `attempts >= max_attempts && elapsed >=
   deadline && no Pass` (`:1199`). A dial probe earns `max_attempts` by writing one counted
   `Fail` per interval from its first tick, so the three conditions agree at the deadline.
   The guest-listener task writes no row before the deadline, so its first `Fail` carries a
   streak of 1 and the count test would not hold until `max_attempts - 1` further intervals.
   The count is a dial-probe notion; for this mechanic the deadline is the only clock. The
   gate therefore takes its attempts threshold from the Service's startup descriptor, which the
   reconciler already reads: `spec_facts_for_service` holds `svc.startup_probes[0]` and its
   `mechanic` (`:758-770`), and puts the threshold on the fact as `startup_attempts_required:
   u32` (replacing `ServiceAllocFact.max_attempts`, whose only reader is that comparison;
   the deadline derivation keeps reading `probe.max_attempts`): `probe.max_attempts` for
   `Tcp` and `Http`, and **1** for `GuestListeners` (a `Fail` row exists, because the task that
   declares failure writes it, and carries the reason and the new `run_seq` D8 retries on).
   Nothing else in the gate, in D7 (the stop action, the give-up), in D8 (the retry rule
   `run_seq > startup_last_acted_run`) or in D9, D10 and D12 changes: the first `Fail` at the
   deadline satisfies `1 >= 1 && elapsed >= deadline && no Pass`; each further `Fail` is a
   newer row with a larger honest streak, so D8 re-emits the stop per row. The supervisor is
   registered after the `Running` row's `started_at` (D3), so the task's deadline measured
   from its start is at least 60 s after `started_at` and the reconciler's `elapsed` test
   holds when the first `Fail` is read. An explicit `[[health_check.startup]]` probe keeps
   `startup_attempts_required = max_attempts`, so its stop rule is byte-identical to today's.
   The terminal's `attempts` is the restart budget consumed (D7), not this streak.
   Evaluated alternatives: (b) a `Fail` row per interval from the start, which gives honest
   counted rows and no gate change but reverses the PR-3 trade-off (about 29 extra writes,
   wakes and evaluations per never-ready attempt); (c) a counter written on the row
   (`attempt`), which needs a new `ProbeResultRow` version and forges the field PROBE-RUN-COUNT
   defines as an observed streak; and (d) the first `Fail` forging `same_status_runs =
   max_attempts`, which contradicts the producer's arithmetic and the pinned tests. (a) is
   chosen: it keeps the accepted trade-off, adds no row field and no wire surface, and makes
   the one place that mattered, the reconciler's own reading of the descriptor, say what the
   mechanic means.

   **Consequence, stated:** for the first 60 s of an attempt that has not yet opened every
   declared listener there is no startup probe row, so `describe` shows no startup result
   until `Pass` or the deadline, and the operator does not see progress such as "missing
   tcp:8080" before 60 s. The alternative of writing a `Fail` every
   interval from the start (30 rows) gives that progress but costs about 29 extra
   observation writes, router wakes and `ServiceLifecycle` evaluations per never-ready
   attempt, and it is the only reader of those rows; surfacing progress some other way
   would need the reported set on the wire (a new surface nobody has asked for), so it is
   not part of the design. **Persisted-default trade-off:** like ADR-0058, the parser materialises
   the default into the stored spec, so a later change of the default does not reach stored
   Services. It is kept because the opt-out must stay distinguishable from "omitted" (F-7).
4. **Rest of the system.** The rows are ordinary startup `ProbeResultRow`s, so `Stable`
   (branch (a), B1), D7, D8, D9, D12, D6 (its precondition "a Startup descriptor exists"
   holds), D10 and D11 are **unchanged code** except the one gate input named in item 3
   (P1, `startup_attempts_required`). `mechanic_summary` is `guest listeners
   tcp:8080 udp:5353` (declared order).

   **Where the declared listeners reach the summary (DG-11, ruled 2026-11-05).** Two
   formatters render a `ProbeMechanic`, and each takes the declared listeners as a second
   parameter, `&[Listener]` (`overdrive_core::aggregate::Listener`); `ProbeMechanic::GuestListeners`
   stays fieldless and no wire field is added. For `Tcp` and `Http` the parameter is unused.
   (a) The reconciler's `format_mechanic_summary` (`service_lifecycle.rs:760`), which fills
   `ProbeWitness.mechanic_summary`, is called from `spec_facts_for_service(svc: &Service)`,
   which already holds `svc.listeners` (`aggregate/mod.rs:363`, declaration order). This is
   the string the deploy stream prints: `format_service_stable_summary` writes
   `witness.mechanic_summary` verbatim (`render.rs:1283`), so the deploy path needs neither the
   parsed spec nor a new field. (b) The CLI's `format_probe_mechanic_summary` (`render.rs:1351`),
   used by the describe `Probes:` section, receives the snapshot's listeners
   (`out.snapshot.listeners`, `render.rs:274`; `AllocStatusResponse.listeners`, `api.rs:312`),
   which the snapshot already carries. Each formatter renders the listeners as `tcp:<port>`
   or `udp:<port>` joined by single spaces in declaration order.
5. Alternatives: see ADR-0269.

#### D14-A1 — exact TCP-view interface

```rust
impl AllocationSpec {
    pub fn tcp_service_ports(&self) -> Vec<NonZeroU16>;
}
```

`NonZeroU16` is `std::num::NonZeroU16`. Each call returns a newly owned vector
containing every `declared_listeners` entry whose protocol is `Proto::Tcp`, in
declaration order; it preserves duplicate TCP declarations if any are present,
does not sort or include UDP, and does not mutate the spec. The action shim's
`declared_tcp_listen_ports` converts this vector to `Vec<ListenPort>`; the
worker's inbound installer derives its capability set and installs rules from
the vector. No `Driver` trait method, stored TCP field, row or wire change is
added by this signature pin.

#### D14-E2 — exact typed input rejection

The **user-approved 2026-11-09** semantic error set adds only these variants to
the existing public enums:

```rust
// overdrive_core::aggregate::AggregateError
GuestListenersInvalidRole {
    vector_role: ProbeRole,
    descriptor_role: ProbeRole,
    index: usize,
},
GuestListenersNotSoleStartup { count: usize },

// overdrive_core::aggregate::workload_spec::ParseError
GuestListenersNotOperatorConfigurable {
    role: ProbeRole,
    probe_idx: usize,
},
```

`ProbeRole` is the existing `overdrive_core::observation::ProbeRole`. Both
indices are zero-based positions in their containing role vector, not the
caller-supplied `ProbeDescriptor.idx`. `vector_role` names the input vector and
`descriptor_role` is the descriptor's own role; the two may disagree on raw
JSON. The variants and their payloads are the semantic causes. Display text
names role and index only for diagnosis.

| Ingress condition | Exact typed result | Caller mapping |
|---|---|---|
| `Service::from_submit` sees `GuestListeners` in `readiness_probes` or `liveness_probes`, or in `startup_probes` with `ProbeDescriptor.role != Startup` | `AggregateError::GuestListenersInvalidRole { vector_role, descriptor_role, index }`; first invalid in Startup, Readiness, Liveness vector order, then index order. This check precedes the sole-startup and general probe-count checks; existing scalar, driver, listener and mechanic-value validation keeps its current precedence. | `submit_workload` retains `ControlPlaneError::Aggregate` with this exact nested variant. `to_response` returns HTTP 400, `ErrorBody.error = "validation"`, and `field = "<vector_role>_probes[<index>].mechanic"` for a non-startup vector or `"startup_probes[<index>].role"` for a startup-vector role mismatch. `message` is diagnostic only. |
| The Service startup vector has a correctly role-tagged `GuestListeners` and more than one descriptor | `AggregateError::GuestListenersNotSoleStartup { count: startup_probes.len() }`, after the invalid-role check and before `ProbeCountExceeded`. The count includes all startup descriptors. | The same typed `ControlPlaneError::Aggregate` chain; HTTP 400 `ErrorBody { error: "validation", field: Some("startup_probes"), message: diagnostic }`. |
| The Service startup vector has exactly one correctly role-tagged `GuestListeners` | Neither new error; other existing validation remains. | Ordinary Service admission. D14 defines no raw-JSON provenance restriction, so none is added. |
| Service TOML writes canonical `type = "guest_listeners"`, case-insensitive, in startup, readiness or liveness | `WorkloadSpecInput::from_toml_str` returns `ParseError::GuestListenersNotOperatorConfigurable { role, probe_idx }`, first in parser role order Startup, Readiness, Liveness and index order. The shared mechanic parser recognizes this one platform-only token before its `UnknownProbeType` arm. | No HTTP request or intent write. Both CLI deploy modes pass this exact parse error as `CliError::ParseError(source)`, with the existing exit code 1. |
| Job or Schedule TOML has any probe block | Existing `ParseError::ProbesNotAllowedOnKind { kind: "job" or "schedule", guidance: existing per-kind guidance }` before mechanic parsing. | Existing typed parser error; no D14-E2 variant. |

`guest_listeners` is the canonical snake-case token; `GUEST_LISTENERS` denotes
the same forbidden mechanic. `GuestListeners` without an underscore remains
the existing `UnknownProbeType { probe_idx, found: "guestlisteners".to_owned()
}` under the current lowercase parser; no new alias is introduced. A raw Job
or Schedule `SubmitSpecInput` cannot contain a probe descriptor: its per-kind
wire payload has no probe vectors and denies unknown fields. Raw JSON adding a
probe field fails the `Json<SubmitWorkloadRequest>` extractor before the
handler body and before `Service::from_submit`. That structural rejection
requires no new typed Service error or test seam. The existing
`AggregateError::Validation { field, message }` flattening is not used for the
two new aggregate variants; `ControlPlaneError::Aggregate` preserves them.
`to_response` updates its exhaustive aggregate match to set the fields above,
while the accepted ADR-0015 status 400 and wire error code `"validation"`
remain unchanged. `ProbeMechanic::validate()` continues to accept the
fieldless value; role and operator-origin checks live at the constructor and
parser ingresses. The CLI's existing private `aggregate_to_cli_error`
exhaustively matches `AggregateError` and needs bounded arms for the two new
variants; this compiler fallout adds no public `CliError` variant and may not
use an `unreachable!` panic for a production-reachable input. No additional
probe-field policy is introduced.

### D15 — the default startup gate is the guest's report of every declared listener

Decision record: ADR-0270 (supersedes ADR-0058 on acceptance).

1. **Rule.** Absent `[[health_check.startup]]` (Service): the parser materialises the
   single `GuestListeners` descriptor. The Service is `Stable` when `Running` and the
   descriptor's latest row is `Pass`. A declared `[[health_check.startup]]` is unchanged.
   **`[[health_check.startup]] = []` remains the opt-out** (`Stable` at the first
   `Running`, branch (a')), for a workload that opens a declared listener lazily or has
   nothing to report at start. Raw-API submits with an empty list keep meaning opt-out (F-7).
2. **All declared listeners**, not `listeners[0]` (reverses ADR-0058 Alternative C). A
   declared listener is a promise the frontend routes to; D12 makes the allocation a
   backend for all of them.
3. **TCP and UDP**, as D13. A Service has at least one listener, so the gate always has a
   target.
4. **Deadline.** 60 s, as before; `progress_deadline` = `max(90 s, 60 s + 30 s)` = 90 s,
   unchanged (D10, ADR-0262). ADR-0095's 60 s deadline is unchanged.

**RCA-A closure argument.** RCA-A: kernel-accepted exec is not liveness; a Service that
exits 1 right after start must not be reported successful.

| Workload behaviour | New outcome |
|---|---|
| Exits 1 without opening a declared listener (the coinflip) | No `Pass` row; `Stable` never announced; exit consumed (D1), row `Failed` with `WorkloadCrashedImmediately`; `WorkloadLifecycle` restarts to the ceiling and gives up `EarlyExit` (D6). **Closed.** |
| Exits 0 before opening a declared listener | No `Pass` row and no `Stable`; the clean exit is `Terminated` with `Stopped { by: Process }`, and the existing D6 ceiling classification gives up `BackoffExhausted`. This is the accepted P-5 exclusion, not an `EarlyExit` classification. |
| Runs but never opens a declared listener (or only an undeclared one), or opens only some | `Fail` rows from 60 s; D7 stops it, D8 retries the stop, give-up `StartupProbeFailed` with the missing listeners as `last_fail`. **Closed.** |
| Opens every declared listener, then exits non-zero **before the terminal row is written** | Can be reported `Stable` if the `Pass` row and `Stable` commit land first. **Residual window**, below. |
| Opens every declared listener, runs long, crashes later | Ordinary post-`Stable` crash: restart authority, `restart_count`, `LastTerminated` (ADR-0078). |

The earlier combined “1 (or 0) → EarlyExit” row contradicted the accepted D6
predicate, S-15 and the user's 2026-11-02 ruling excluding P-5. This table
restates those already accepted outcomes; it adds no clean-exit classifier.

**Residual window.** The platform-controlled part is the time from the guest's
`listen()`/`bind()` to the exit observer writing the terminal row: vsock delivery, the
probe task's `Pass` write, one reconcile wake (immediate with D17; up to `tick_period_ms`,
default 100 ms, without it) and the `Stable` commit. No `Pass` write is started after
the exit event is consumed (D1 cancels the task first; D13 item 10: a write already issued may
land and is inert), and no `Stable` is announced for a row no longer `Running`. The
rest is outside the platform's control: **`Stable` means "every declared listener has been
opened", with no minimum healthy time.** That is weaker than ADR-0058 for a workload that
opens its listener and crashes within about 2 s, the same promise as Kubernetes
(`startupProbe`, `minReadySeconds` 0) and weaker than Nomad (`min_healthy_time` 10 s). The
user ruled on 2026-11-03 that no minimum-healthy window is added. Sustained health is the
readiness probe's job, which D12 gates traffic on.

### D16 — remove `ProbeDescriptor.inferred` and its carriers, single cut

Decision record: ADR-0271 (option R). With D14 the platform-supplied descriptor is
identified by its mechanic, so the flag is derived state (`inferred ==
(mechanic == GuestListeners)`). Option K (keep the field) is the rejected
alternative. The accepted outcome removes `inferred` from the descriptor, the
probe-result row, the Stable witness, the API/describe wire and CLI render in
one cut; no replacement field or stub is added.

**Five actual carrier boundaries:** `WorkloadIntentEnvelope` (Service intent),
`ServiceSpecEnvelope` (parser-side Service spec), `ProbeResultRowEnvelope`,
`AllocStatusRowEnvelope` (including `LastTerminated.terminal`), and
`AllocLifecycleOccurrenceRowEnvelope` (the separately persisted lifecycle
occurrence). The occurrence carrier was omitted from the earlier table; the
local store archives it in `write_alloc_lifecycle_transaction`. All five must
carry one coherent current layout after the field removal.

**Explicit greenfield compatibility waiver, user 2026-11-09:**

> "preserve historical bytes" -> what do um ean? we dont need to preserve
> anything yet. this is greenfield. not used in production. rest is fine

This waives historical persisted-byte/backward-read compatibility for this
feature's pre-D16 archive shapes and the historical nested-type copies,
old-to-new conversion chains and historical fixture additions that served
only that compatibility. It does not waive current-format correctness, any
other accepted D16 deletion, the five-carrier inventory, or unrelated schema
and fixture contracts. The prior D16 table's V2/V3 historical migration
instructions are superseded by this waiver; D16-S1 was never accepted.

**D16-G1 — exact current-format contract, accepted under the waiver and
independently approved at DESIGN review iteration 4.** Keep the five existing
envelope families and their version tags, latest aliases and constructor
selection. Rebaseline only their current payload layouts in place; no V2/V3
variant, historical nested type or new migration reader is added for D16.

| Carrier | Exact current shape after D16-G1 |
|---|---|
| Service intent | `WorkloadIntentEnvelope::V1(WorkloadIntentV1)` stays tag 0; `WorkloadIntent`/`WorkloadIntentLatest` stay aliases of `WorkloadIntentV1`, and `Service` stays an alias of `ServiceV1`. `ServiceV1` keeps its field order and three `Vec<ProbeDescriptor>` vectors. The current `ProbeDescriptor` loses only `inferred: bool`; `ProbeMechanic::{Tcp,Http,GuestListeners}` retain tags 0, 1, 2. A current V1 Service intent may contain `GuestListeners`. |
| Parser-side Service spec | `ServiceSpecEnvelope::V1(ServiceSpecV1)` stays tag 0; `ServiceSpec`/`ServiceSpecLatest` stay aliases of `ServiceSpecV1`. Its fields and three `Vec<ProbeDescriptor>` vectors stay in order and use the current descriptor without `inferred`; operator TOML still rejects `guest_listeners` under D14-E2. |
| Probe result | `ProbeResultRowEnvelope::{V1(ProbeResultRowV1), V2(ProbeResultRowV2)}` keep tags 0 and 1; `ProbeResultRow`/`ProbeResultRowLatest` stay aliases of V2. Remove `inferred: bool` from both payloads; V2's surviving fields remain `alloc_id`, `probe_idx`, `role`, `status`, `last_observed_at_unix_ms`, `run_seq`, `same_status_runs` in that order. V1 keeps its five surviving fields in order. Its existing V1→V2 run-count conversion remains (`run_seq = FIRST`, `same_status_runs = ONE`, other fields copied). |
| Allocation status | `AllocStatusRowEnvelope::V1(AllocStatusRowV1)` stays tag 0; `AllocStatusRow`/`AllocStatusRowLatest` stay aliases of V1. Every row field and its order stays. `terminal` and `last_terminated.terminal` use current `TerminalCondition::Stable` with current `ProbeWitness { probe_idx: u32, role: String, mechanic_summary: String }` in that order; only `inferred` is removed. `LastTerminated` and the other `TerminalCondition` variants keep their names and fields. |
| Allocation lifecycle occurrence | `AllocLifecycleOccurrenceRowEnvelope::V1(AllocLifecycleOccurrenceRowV1)` stays tag 0; `AllocLifecycleOccurrenceRow` and `VersionedEnvelope::Latest` remain V1. Fields and order stay; its `terminal` uses the same current `ProbeWitness` without `inferred`. `LocalObservationStore::write_alloc_lifecycle_transaction` still persists this envelope. |

`latest()` and `into_latest()` keep selecting/projecting the existing latest
variants. `known_discriminants()` stays `[0]` for the four V1-only envelopes and
`[0, 1]` for `ProbeResultRowEnvelope`; `WorkloadIntentEnvelope`'s discriminant
offset remains `None`. The other affected numeric codec offsets are derived from
new canonical archive bytes and checked with current-layout golden evidence.
Existing `FIXTURE_V*` byte literals are not rewritten. Their historical-byte
decode assertions cease to gate only these waived pre-D16 layouts; current-layout
golden/roundtrip evidence covers all five carriers. Unrelated envelope fixtures
and tests remain. Earlier unfinished-checkout redb data is discarded before
using the new build under the repository's greenfield single-cut policy. The
new build may reject or reinterpret those waived pre-D16 bytes; no dual-read
period is promised. The archived inner `WorkloadIntentV1::spec_digest` baseline
may change across builds, including Job/Schedule enum bytes because rkyv sizes
from the largest variant; within the new build, byte-identical intent remains
byte-identical and `put_if_absent` idempotency and kind dispatch are unchanged.

Roadmap 05-04 changes the descriptor and rebaselines the Service intent/spec
layouts in one commit while retaining the default TCP startup gate. Roadmap
05-05 changes the probe row and witness and rebaselines the three observation
carrier layouts in one commit. Each includes directly affected constructor,
offset and current-layout evidence fallout. No intermediate commit writes an
old nested shape under a changed current payload. This scoped policy adds no
storage table, historical migration system, new wire field, or test lane.

**Deletion list (single cut, no stub), by behaviour.** The default-startup-probe
synthesis in the TOML parser (the `STARTUP_*_DEFAULT` constants stay for explicit probes and
for the materialised `GuestListeners` descriptor); `inferred` as a field of the Service alloc
fact, its hydration, the `Stable` witness and the opt-out witness; `inferred` on the probe
descriptor, the probe-result row and the witness, and on the wire and OpenAPI; the CLI
`(inferred)` suffix and witness text. Tests and goldens that pin `inferred`
(the parse tests of the synthesised probe, the probe-count validation test, the
`service_lifecycle` tests, render and describe tests, the `inferred` proptests, the
hydration golden) are updated or retired with the code; existing archive
`FIXTURE_V*` byte literals are not rewritten, and current-layout golden
evidence is required under D16-G1. DISTILL confirms the
affected set by running the suite. **Probe cap:** the `GuestListeners` descriptor holds no dial, so it does
not count toward `GUEST_PROBE_SLOTS` (`guest_transport/mod.rs:554-560`: the cap bounds the
node flow slots a workload's probe tasks hold outside its quota, one dial per task); the
count is over the dial-bearing descriptors and the `ProbeCountExceeded` message no longer
mentions an inferred probe. `verification/expectations/E13-vm-service-inferred-tcp-startup/`
is a point-in-time record, not edited. Operator docs
`website/content/docs/how-to/deploy-a-workload.mdx:108-111` are rewritten at the finalize
gate.

### Interface changes (D13 to D16)

| Surface | Change | Crate |
|---|---|---|
| `BeaconMessage` | New guest-to-host variant `Listen { protocol: Proto, port: NonZeroU16 }`, text form `LISTEN <tcp\|udp> <port>`, strict three-field parse | `overdrive-core` |
| `BeaconParseError` (**DG-M**) | New variant `MalformedListen { raw: String, detail: String }`, `#[error("LISTEN message is malformed ({detail}) in line {raw:?}")]`, in the shape of `MalformedArgv` (`beacon.rs:155`): `raw` is the full line, `detail` is the `Display` text of the underlying failure (no `#[source]`, so the enum keeps its `Clone` and `PartialEq` derives). Every rejected `LISTEN` line returns this one variant: wrong field count, unknown protocol, a port that is not a number, out of range or zero. `FieldCount` and `InvalidInt` are not used for `LISTEN` | `overdrive-core` |
| `ProbeMechanic` | New fieldless tail variant `GuestListeners` (Service, Startup, sole descriptor only); typed ingress errors D14-E2 below | `overdrive-core` |
| `AggregateError`, `ParseError` (D14-E2) | New exact variants/payloads and mappings in § D14-E2: `GuestListenersInvalidRole { vector_role: ProbeRole, descriptor_role: ProbeRole, index: usize }`, `GuestListenersNotSoleStartup { count: usize }`, `GuestListenersNotOperatorConfigurable { role: ProbeRole, probe_idx: usize }`. Existing `ControlPlaneError::Aggregate`, HTTP 400 `validation`, and `CliError::ParseError` carriers are retained | `overdrive-core`, bounded `overdrive-control-plane` and `overdrive-cli` mapping fallout |
| `format_mechanic_summary` (reconciler, private) and `format_probe_mechanic_summary` (`overdrive-cli`, `pub`) (**DG-11**) | Each gains a second parameter `&[Listener]`, the declared listeners; used only by the `GuestListeners` arm (D14 item 4). No wire, archived or `ProbeMechanic` change | `overdrive-reconcilers`, `overdrive-cli` |
| `ReportedListeners`, `GuestListenerBoard` (**DG-6, D13-S1**) | `ReportedListeners`: `Default`, `FromIterator<Listener>`, equality, and exact `contains(&self, Proto, NonZeroU16) -> bool`, `len(&self) -> usize`, `is_empty(&self) -> bool` signatures (D13 item 10). `GuestListenerBoard::new() -> Self` | `overdrive-core`, `overdrive-worker` |
| `AllocationSpec`, `WorkloadLifecycleState` | New `declared_listeners: Vec<Listener>` (in-memory); **PR-4:** replaces `service_ports` in both types. **D14-A1:** `AllocationSpec::tcp_service_ports(&self) -> Vec<NonZeroU16>` is the owned TCP view the TPROXY consumers read (D14 item 2) | `overdrive-core`, `overdrive-reconcilers` |
| `GuestListenerReports` (**PR-1**) | New read-only driven port, `#[async_trait]`, held as `Arc<dyn GuestListenerReports>`: `reported(&AllocationId) -> ReportedListeners`, `async changed(&AllocationId, &ReportedListeners) -> ReportedListeners`; behaviour contract in D13 item 10. New value type `ReportedListeners` | `overdrive-core` |
| `GuestListenerBoard` (**PR-1**) | New concrete implementation of the port in `overdrive-worker`; its methods and any outcome type are internal (the crafter's). Pinned behaviour only, in D13 item 10: entry created before the beacon reader, removed after `stop_alloc` and on a failed `start`, an insertion-reporting check-and-act, three no-op cases | `overdrive-worker` |
| `ServiceAllocFact` (**P1**, accepted 2026-11-04) | `max_attempts: u32` is replaced by `startup_attempts_required: u32` (in-memory State; `Tcp`/`Http` descriptor: its `max_attempts`; `GuestListeners`: 1). No row, wire, archived or CBOR change | `overdrive-reconcilers` |
| `ProbeRunner::new`, `VmDriver::new` (**PR-1, D13-S1**) | Gain a mandatory `Arc<dyn GuestListenerReports>` as ProbeRunner's sixth parameter and `Arc<GuestListenerBoard>` as VmDriver's ninth/last parameter respectively. The full `VmDriver::new` signature is in D13 item 10 | `overdrive-worker` |
| `SimGuestListenerReports` (**PR-1**) | New `Sim*` implementation of the port (scripting API is DISTILL's) | `overdrive-sim` |
| `ProbeDescriptor.inferred`, `ProbeResultRow.inferred`, `ProbeWitness.inferred` | Removed (D16 option R) from current public, wire and all five affected carrier layouts; exact scoped in-place envelope version policy is **accepted D16-G1** (§ D16) | `overdrive-core` |
| Wire / OpenAPI / CLI | `inferred` removed; describe/render print the mechanic summary; new mechanic value in the probe schema | `overdrive-core`, `overdrive-control-plane`, `overdrive-cli` |
| Probe runner | A task kind that waits on the reported-set port and writes `Pass`/`Fail` rows through the dial task's run state; takes the declared set from `start_alloc`'s existing `&AllocationSpec` (D13 item 10, D14 item 3) | `overdrive-worker` |
| Guest | Listen programs: a separate small sockops `TCP_LISTEN_CB` program in the existing object and two `CgroupSock` programs (post-bind4, post-bind6), all three attached to a workload cgroup that init creates and starts the workload in (**PR-2**, V42 held; objects and attachments pinned in D13 item 1, P-a to P-d); forwarding of events as `Listen` lines (structure is the crafter's) | `overdrive-bpf`, `overdrive-init` |
| `Driver` trait, `Action`, reconciler traits, other read-ports, cmdline token | Unchanged | |

Contract shapes (design.md rule 12): the parse of `Listen` is **pure-function**;
`GuestListenerReports` is a **read-only** port (no write method on it); the board is
**bounded-change** (insert-only while an entry exists; universe = the declared listeners of
one allocation; assertion: through `reported`, the set only grows and stays a subset of the
declared set);
the probe task is **bounded-change** (writes only startup `ProbeResultRow`s for its own
allocation); no component here is unbounded-preservation. The board is an in-process
adapter over memory, with no external dependency, so it carries no `probe()`; the guest
hooks' real-environment proof is V34 to V39 and V42.

### Lifecycle Gate Ownership — Gate G-10, the guest-listener startup gate (D13 to D15)

- **Existing evidence:** ADR-0054, ADR-0055, ADR-0259, ADR-0265, ADR-0267; path: guest
  `listen()`/`bind()` → [new] in-guest BPF → [new] `LISTEN` beacon line → [new] host board
  (`GuestListenerBoard`, read through `GuestListenerReports`) →
  probe task → startup `ProbeResultRow` → `ServiceLifecycle::reconcile` branch (a)
  (`service_lifecycle.rs:535-558`) → `FinalizeFailed { Stable }`
  (`action_shim/mod.rs:2028-2040`) → `streaming.rs:835`.
- **Owner:** the probe runner for the fact (the only startup-row writer); `ServiceLifecycle`
  for `Stable`, unchanged.
- **Promise:** every declared listener has been bound or listened on, on a non-loopback
  address, at least once by this incarnation's workload; no claim about staying up or about
  IPv4 reachability (D13 item 8).
- **Affected state:** the allocation's startup probe row, and through existing gates `Stable`.
- **Failure projection:** no report by the deadline → a `Fail` row at the deadline listing the
  missing listeners, then one per interval → D7 stop → give-up `StartupProbeFailed`;
  unparseable line → dropped and logged; undeclared report → not stored; duplicate → no-op;
  beacon disconnected → the reader records nothing further and the VMM exit is handled by the
  exit watcher; late report after the terminal row → no row started; cancelled consumer → level-triggered from
  rows; a dropped report (full ring) → the same as no report.
- **Explicitly unaffected:** `Running`; the driver `start`/`READY` outcome (init-ready, not
  workload-ready); restart budget, backoff and ceiling; liveness (D9); readiness probes;
  the opt-out path; Job and Schedule.
- **Ordering:** after the `Running` row and supervisor registration (D3), before `Stable`;
  consumes the one 60 s startup deadline.
- **Counterexample:** gating `Running` or driver `start` on the report deadlocks: the
  workload does not run until `EXEC`, which is released after `Running`.
- **Evidence lane:** pure/proptest (wire parse; set-versus-declared predicate); seeded
  `overdrive-sim` with `SimGuestListenerReports` (happy path, deadline `Fail` cadence,
  retry, exit-before-pass, ordering); quint-connect conformance of the beacon report module;
  real kernel (V34 to V39, V42) on native metal.

**Required boundary scenarios (DISTILL obligations once accepted).** The beacon-channel
sequences below are specified once, by the named `guest_report` runs (§ Formal protocol
model); a scenario cites its run and does not restate it. (1) all declared listeners reported
→ `Pass` → `Stable` (run `host_all_declared_reported`); (2) one never reported → no row before the
deadline, the first `Fail` at the deadline (run `first_fail_at_deadline_with_no_earlier_row_stops`),
stop, one `Fail` per interval while the stop has not landed, retry, give-up; (3) `Running` row
and an opt-out Service unchanged; an undeclared port changes nothing; (4) late `Pass`: no
`Pass` write is started after the task observed cancellation (`stop_alloc`), and a `Pass` that
lands after the exit event is inert (D13 item 10); (5) beacon closed with the set partial →
no `Pass`, deadline stop (run `host_beacon_closes_with_partial_set`); guest reboot is a new
allocation with an empty set; (6) opt-out and explicit-probe Services are behaviourally
unchanged, including an explicit probe's `Fail` written after the deadline keeping its counted
streak (run `explicit_probe_fail_after_deadline_keeps_counted_streak`, P1); (7) **host reader**
(Tier 1, scripted beacon lines through the reader's `BeaconIo` seam): a garbage line after
`READY` is dropped with a warn event and the following `Listen` and `Exit` still take effect
(run `host_garbage_line_is_dropped`); a `Listen` line after `Exit` changes neither the set nor
the exit report (run `host_listen_after_exit_is_ignored`; the full module's drain rule keeps
such a line off the wire, so only the host-input projection reaches it); (8) **ring drop** (seeded sim): the declared listener's report
never reaches the board while the workload is running → no `Pass`, `Fail` at the startup
deadline, stop, restart, give-up `StartupProbeFailed` naming the missing listener (the stated
bounded exposure, D13 item 9; run `ring_drop_leaves_set_partial`);
(9) **the guest's own binds**: init's transport binds (TCP intake,
1,024 UDP slots) produce zero `LISTEN` lines before `EXEC` under PR-2. Clause (9a) carries no
test obligation (ruling DG-13): every socket init binds comes from `guest_reserved_ports()`
(61000..=62024), the Service validator rejects any declared port in that range
(`ListenerPortReserved`), and the board ignores a report for an undeclared listener (S-51,
undeclared pair), so a report of init's binds has no production effect. Clause (9b): the
workload's own listener, started after `EXEC`, produces exactly one (Tier 3, real guest); (10) **loopback** (Tier 3): binds to `127.0.0.1`, `::1`
and `::ffff:127.0.0.1` produce no line, and `::ffff:<non-loopback>` produces one.
**Ruling DG-8 (2026-11-05): how (8) to (10) are driven.** (9b) and (10) are Tier-3 real-boot
DELIVER obligations of D13 and D14 (native metal, `cargo xtask metal run --`), observed
through production-written state, with no writer seam in `overdrive-init`. For (9), the
first clause (9a) carries no test obligation (ruling DG-13); the second clause (9b) is the real-boot test
that holds a declared TCP and UDP listener and observes `Pass` while `Running`. For
(10), the fixture declares a listener and the workload binds it only to `127.0.0.1`, `::1`
or `::ffff:127.0.0.1`; the startup probe row is `Fail` naming the port as unreported. A
workload bind to `::ffff:<non-loopback>` on a declared port gives `Pass`. (8) is a seeded
Tier-1 composed run: the production `VmDriver` over `SimVmm` and a stepped `SimBeaconIo`,
the production exit observer and the real reader (the `quint_cid_lease` composition), where
the fault is the omitted `LISTEN` line, i.e. the script does not write the declared
listener's line while the guest stays running. The driven port that injects it is the
existing `SimBeaconIo` (`guest_write` is simply not called for that line). This is the
right injection point because the ring drop is in-guest, and its only host-visible effect is
that the line never arrives; nothing on the host distinguishes a dropped line from a
`listen()` that never happened. The run then asserts `SimClock` advance to the deadline,
the first `Fail` row, the D7 stop, the restart and the give-up `StartupProbeFailed` naming
the missing listener. The kernel's drop-and-count behaviour itself stays V37's evidence and
the `ring_drop_leaves_set_partial` model run. Seeded
invariants: `GuestListenerPassImpliesAllDeclared`, `NoPassAfterEnding` (no `Pass` write
is started after the task observed cancellation), `StableNeverForNonRunning`,
`NoFailBeforeDeadline`, `GuestListenerChangedNeverMissesAnInsertion` (S-52).

### Interaction with the accepted decisions

D12 unchanged (keyed on the committed `Stable`). D10: `progress_deadline` is 90 s. D7/D8: a
never-listening guest is stopped at the first `Fail` (60 s) and re-stopped per new `Fail`.
D6: the pre-startup crash precondition holds for the default; the opt-out has none and
gives up `BackoffExhausted`. D11 and ADR-0095 unchanged. ADR-0080: one more task kind.

### Formal protocol model — D13 to D16

**D14 and D15 add no state to the existing modules.** Their producer is the existing startup
row; the workload-level flag `gl` in `restart_authority` carries the one rule that differs
(no `Fail` before the deadline; stop-gate threshold 1, P1) and is free only in the conformance
instance `ra_conf`, pinned to `false` in the exhaustive `ra_ok` instance and its hazard
instances. **The beacon report channel (D13 items 1, 3 and 10) is an ordered, single-writer,
crash-sensitive protocol and carries two modules** in `specs/quint/service-allocation-readiness-and-ending/guest_report.qnt`:
`guest_report` (the model check, not a conformance target) and `guest_report_host` (the
conformance target). The guest-signal spike measured the hazard: 185 of 300 boots delivered a
`LISTEN` after `EXIT` without the drain.

- **Variables.** `wl` (workload: not started, running, exited); `ring` (the kernel's listen
  events not yet taken by the writer, a sequence over a small alphabet such as `tcp80`,
  `udp53`, `other`); `wire` (what init has written to the beacon, in order: `ready`,
  `listen(p)`, `exit`); `rd` (how much of `wire` the host reader has consumed); `set` (the
  board's reported set); `wait` (the host's guest-report wait: waiting, ended); model-only ghosts `produced` (events the hook enqueued,
  with a `dropped` marker for a full-ring event) and `eof` (the beacon closed).
- **Actions.** `WorkloadListen(p)` (only while `wl` is running; enqueues to `ring` or marks
  the event dropped); `WorkloadExit`; `WriterForward` (moves the head of `ring` to `wire`);
  `WriterDrain` (forwards everything left in `ring`, enabled only after `WorkloadExit`);
  `WriterExit` (appends `exit`; guard: `wl` exited and `ring` empty); `BeaconClose` (EOF:
  guest or VMM gone); `HostRead` (consumes one `wire` entry: `listen(p)` inserts into `set`
  only while `wait` is waiting and `p` is declared; `exit` ends the wait; EOF ends the wait);
  `HostGarbage` (an unparseable line: no effect on `set` or `wait`). The full module has no
  exit-report variable: the exit report is a conformance concern of `guest_report_host`
  alone. There is one writer: every `wire` append is a writer action, never two.
- **Invariants** (only those a hazard or a measured race gives teeth). `ListenBeforeExit`
  (in `wire`, every `listen` precedes `exit`); `DeliveredBeforeExit` (when `exit` is on
  `wire`, every produced, non-dropped event is on `wire` before it). Progress:
  `ExitEventuallyReported` (a workload that exits with the beacon open ends the wait).
  **Not invariants, on purpose:** one writer (true of the action set by construction, so a
  check on a ghost counter proves nothing); `set` within the produced pairs (true by the
  `HostRead` action's own definition, no hazard removes it); `set` not changing after the wait ended (the reader's rule of
  item 3 is a guard in `HostRead`; the late-`Listen` clause is exercised by the host-input
  projection's run `host_listen_after_exit_is_ignored`, S-51).
- **Named runs of the full module** (the guest-side scenarios only; each listed in `checks.toml`
  with `backend = "evaluator"`): `listen_then_exit_drains_first` (happy path and the
  185-of-300 race: a `LISTEN` produced just before exit is on `wire` before `exit`) and
  `ring_drop_leaves_set_partial` (a dropped declared event: `set` stays partial, the wait still
  ends on `exit`). The host-visible scenarios (all declared reported, a garbage line, EOF with a
  partial set, a `listen` after `exit`) are runs of `guest_report_host`, below.
- **One hazard, one check:** `hz_no_drain_before_exit` — `WriterExit` is enabled without the
  ring being empty (the drain removed). The one check expects a violation of
  `DeliveredBeforeExit`. No second check on `ListenBeforeExit` (the same decision), and no
  other hazard: the writer-ordering rule is the one decision, and "two writers" is covered by
  the single-writer structure of the action set (`.claude/rules/design.md`: one hazard per
  decision).
- **Assumptions, each with a discharging item.** The kernel enqueues a listen event for every
  `listen()` of the workload before the workload can observe the socket serving (V34, V35,
  V36); the ring drops and counts on full (V37); vsock preserves the order of one stream (V38).
- **Pinned accessors for the conformance driver (module `guest_report_host`).** `set`
  (`rset` in the spec) ↔ `GuestListenerReports::reported(alloc)` (D13 item 10; **the only
  conformance read of the board**, read while the entry is open: `close` is lifecycle and not an
  action of the projection). `exitrep` is **three-valued** and is read from the exit
  observer's resulting row, through `ObservationStore::alloc_status_row(alloc)` (re-readable
  owner observation; ruling R1-a, 2026-11-04), never from the consumed `ExitEvent`. The
  chain that makes the row carry the distinction: `run_exit_watcher` joins the guest report
  with `classify_vm_exit` (`vm_driver.rs:2150-2156`: `Some(0)` is `CleanExit`, `Some(n)` is
  `Crashed { exit_code: Some(n), signal: None }`, `None` is `Crashed { exit_code: None, .. }`)
  and sends the `ExitEvent` with `intentional_stop: false` (`:2315-2329`, always false for a
  VM, so the operator-stop arm of `classify` never applies); `exit_observer::classify`
  (`exit_observer.rs:579-621`) maps it to the row `state` and `reason`
  (`AllocStatusRow.state`, `AllocStatusRow.reason`, `observation_store.rs:1041,1043`), and
  `handle_exit_event` writes that row (`exit_observer.rs:463-555`). The projection:

  | `exitrep` | `alloc_status_row(alloc)` |
  |---|---|
  | `none` | no row, or `state` not terminal (`AllocState::is_terminal`) |
  | `guest-reported` | `state == Terminated` with `reason == Stopped { by: Process }` (guest status 0), or `state == Failed` with `reason == WorkloadCrashedImmediately { exit_code: Some(_), .. }` (guest status n) |
  | `no-report` | `state == Failed` with `reason == WorkloadCrashedImmediately { exit_code: None, .. }`; or `reason == VmOutOfMemory { .. }` (classification only: the cgroup OOM diagnosis runs only when `guest_report.is_none()`, `vm_driver.rs:2263-2275`, so the full mapping classes it `no-report`, but no projection action reaches it: `VmmExit` calls `SimVmm::exit_process(pid, Some(0), None)`, which leaves no OOM facts in `SimCgroupAccounting`) |

  The two reported and unreported classes are disjoint: a no-report exit never produces
  `Terminated` and never carries an `exit_code`, and a guest-reported exit always does. Any
  terminal row outside these three rows (for example `Stopped { by: Operator }`) is not
  reachable from the projection's actions and fails the step. The exit event is emitted only
  after the VMM has exited and the exit gate is released (`run_exit_watcher`: `drain_guest_report`,
  then the gate wait at `vm_driver.rs:2295`, then the send at `:2329`), which is why `exitrep`
  stays `none` until the projection's `VmmExit`. `wait`, `lines`, `eof` and the status the
  reader holds for the pending exit are **model-only**: no accessor, never projected; their
  observable effects are covered by `set` and `exitrep`. In the full module `ring`, `wire`,
  `wl`, `produced`, `eof` and `junk` are model-only.
- **Action to driving-port mapping (host-half conformance).** The driving seam is the
  existing scripted beacon source of the real host reader, not a new one: the reader consumes
  lines through the `BeaconIo` port (`overdrive-core/src/vm/beacon.rs:431-450`), and
  `SimBeaconIo` (`overdrive-sim/src/adapters/beacon_io.rs`) is stepped and scriptable
  (`guest_write(key, line)` `:248`, `guest_close(key)` `:258`, `release_read(key)` `:292`),
  as `quint_flows_beacon.rs` already drives it. So the real reader can be driven from a
  scripted source with no new surface.

  | Action | Conformance mapping |
  |---|---|
  | `HostRead` | `SimBeaconIo::release_read`: the real reader consumes the next scripted line and routes it (item 3) |
  | `HostGarbage` | `SimBeaconIo::guest_write` of an unparseable line |
  | the line-appending writer actions (`WriterForward`, `WriterDrain`, `WriterExit`) | `SimBeaconIo::guest_write` of the `LISTEN`/`EXIT` line they append: the guest's effect on the host is only the line (**not** a writer seam: nothing in `overdrive-init` is observed) |
  | `BeaconClose` | `SimBeaconIo::guest_close`: EOF to the real reader |
  | `VmmExit` (**R1**, **R1-a**) | `SimVmm::exit_process` (the VMM process exits) and `Driver::release_for_exit_emission` (the exit-gate release), so `run_exit_watcher` classifies the guest report (or its absence) and sends the `ExitEvent` (`vm_driver.rs:2237-2330`) to the **production exit observer**, which writes the row; the handler then awaits that row (composition and preconditions below). Precedent for the VMM-side step: the `quint_flows_beacon` driver's `vm_stop` (`crates/overdrive-worker/src/vm_driver/quint_flows_beacon.rs:446-465`): `self.vmm.exit_process(pid, Some(0), None)`, then `observe_vm_exits`; that driver does not compose the exit observer, so the composition below is new to this driver |
  | `WorkloadListen`, `WorkloadExit`, and the `ring`, `wl`, `produced` state | **model-only**: guest and kernel internals with no host-visible effect |
  | `Idle` (DG-M) | a read-only call to `GuestListenerReports::reported(alloc)`; the handler compares the result with the previously projected `rset` and fails the step if it changed. It writes nothing and releases no read |

  **Driver home (decided here).** The `guest_report_host` driver lives in
  `overdrive-control-plane`, as `crates/overdrive-control-plane/src/worker/quint_guest_report_host.rs`,
  declared in `src/worker/mod.rs` as `#[cfg(all(test, feature = "integration-tests"))] mod
  quint_guest_report_host;` (testing.md § "Quint conformance": a driver sits in the owning
  crate's `src/` behind that gate; placement mirrors `src/guest_network.rs:56-57`, which
  declares `quint_cid_lease`). It is not in `overdrive-worker` and not in `tests/conformance`.
  Reasons, from the manifests and source:
  - **Dependency direction.** `overdrive-control-plane` depends on `overdrive-worker`
    (`crates/overdrive-control-plane/Cargo.toml:69`, `[dependencies]`); `overdrive-worker` has
    no `overdrive-control-plane` entry in `[dependencies]` or `[dev-dependencies]`
    (`crates/overdrive-worker/Cargo.toml:37-127`). The exit observer
    (`exit_observer::spawn_with_runtime`, `exit_observer.rs:161`) and the action shim live in
    the control plane, so a driver in `overdrive-worker` could not compose them without a
    dev-dependency on the crate that depends on it (a second compiled copy of the worker's
    types, the hazard `lib.rs:163-164` already records). A driver in the control plane needs
    nothing the worker keeps private.
  - **The multi-owner rule does not apply.** The projection reads two owners (the worker's
    board and the control plane's observer row), but it is observed through the owners' own
    ports, not through the exported server handler, so `tests/conformance` is not the home.
  - **Precedent for the composition.** `quint_cid_lease.rs` (same crate, same gate) composes
    the production `VmDriver` over `SimVmm` and a stepped `SimBeaconIo`, the production exit
    observer, an `AppState`, and the action shim over a substituted guest-network
    provisioner (`quint_cid_lease.rs:20-37`, `compose` `:516-616`, `boot_serve` `:620-648`).

  **`VmmExit` composition and preconditions (R1-a).** Every piece is an existing production
  entry or port reachable from `overdrive-control-plane`'s `src/` (the cited seams exist
  today); the board and the extra constructor parameters are PR-1, already accepted.

  - **Seams, each reachable from the driver's crate.** `VmDriver::new`, `VmHostLayout`,
    `BeaconIntake::bind` and `ProbeRunner::new` are `pub` in `overdrive-worker` and imported
    by `quint_cid_lease.rs:184,536-562` (`BeaconIntake` is `pub`, `overdrive-worker/src/lib.rs:22`,
    `beacon_intake.rs:43`; the worker's own driver uses the same type through a crate path, not
    a private one). `SimVmm`, `SimBeaconIo::stepped`, `SimClock`, `SimObservationStore` come
    from `overdrive-sim`, a `[dev-dependencies]` entry of the control plane
    (`Cargo.toml:212`). `exit_observer::spawn` (`exit_observer.rs:116-123`, the four-argument form
    of `spawn_with_runtime`) is `pub`, as `quint_cid_lease.rs:627` calls it.
    `AppState::new`, `test_default_allocator` (`lib.rs:483`) and `test_empty_listener_facts`
    (`lib.rs:505`) are `pub`. `shared_network_test_ports::{mtls_worker, shared_guest_network_owner}`
    (`lib.rs:165-166` `#[cfg(test)]`; `shared_network_test_ports.rs:674,699`) are crate-private
    and reachable from `src/`. The `GuestListenerBoard` is PR-1's: the driver builds one,
    passes it to `VmDriver::new`, and reads `GuestListenerReports::reported` on it.
  - **The `Running` write and the mTLS precondition.** The driver dispatches one start action
    through `action_shim::dispatch_with_guest_network_provisioner_for_test`
    (`action_shim/mod.rs:1422-1456`, `#[cfg(any(test, feature = "integration-tests"))]`, the
    entry `guest_activation_acceptance.rs:295-302` uses), with the provisioner
    `TestSharedOwner` (`shared_network_test_ports.rs:297`: `provision` and `activate` succeed
    unscripted, `teardown` too). That arm calls `Driver::start`, then
    `MtlsInterceptWorker::start_alloc`, then writes the `Running` row, then calls
    `release_for_exit_emission` only after `start_alloc` succeeds (`vm_driver.rs:1957-1964`;
    shim call sites `action_shim/mod.rs:2679,3201`). The precondition is met with existing Sim
    adapters: `mtls_worker()` is an `MtlsInterceptWorker` over `SimMtlsIntercept` (unfaulted,
    so `start_alloc` succeeds), started with `start_shared_owner()` as `quint_cid_lease.rs:588-591`
    does, and passed to `AppState::new`. The guest's READY (carrying the lease's launch token)
    is delivered through `SimBeaconIo` as in `quint_cid_lease.rs` `launch` (`:1171`) and
    `launch_vmm` (`:857`). The exact `Action` fields and the READY-delivery order are DISTILL's
    to write from those two precedents; no new surface.
  - **Composition.** The driver holds the production `VmDriver` and spawns the production
    exit observer on it, `exit_observer::spawn_with_runtime(obs, driver, events, clock,
    None, token)` (`exit_observer.rs:161-168`), with `obs` the `SimObservationStore` shared
    with the driver (the driven port; `SimClock` as `clock`). The observer, not the driver,
    takes the receiver (`take_exit_receiver`, `exit_observer.rs:170`) and writes the row, so
    the harness never calls `take_exit_receiver` and reads no `ExitEvent`. The observer needs
    a prior `Running` row for the alloc (`find_prior_row`, `exit_observer.rs:468-471`; no row
    is a `NoWrite`); that row is written by the production action-shim start path (its
    `Pending` to `Running` write, which then fires `release_for_exit_emission`), not
    hand-installed by the driver.
  - **Preconditions of `VmmExit`.** (1) A started VM whose guest-initiated READY/EXEC beacon
    session was accepted, so `guest_network_exec.claim_release()` yields a claim and
    `live_vm.pending_exec` and `gate_sender` are present (`vm_driver.rs:1641`, `:1965-1991`).
    Without that session `release_for_exit_emission` returns at `:1966-1968` and the exit
    watcher parks on the gate until it is dropped. If the shim's start path already fired the
    release, the handler's own call is the idempotent no-op (`Option::take` on both pending
    values, `:1957-1964`); either way exactly one release has fired and the `Running` row was
    committed before it. (2) No `Driver::stop` of the alloc in the run, so the row is the
    exit observer's and not the stop path's.
  - **Await.** After `exit_process` and the release, the handler awaits the alloc's row
    reaching a terminal `state`, polling `alloc_status_row(alloc)` with yields under a bound
    DISTILL states (a missed bound fails the step, never maps to `none`). It then projects
    `exitrep` per the table above.
  - **Abstraction stated.** The model's `VmmExit` guard `wait == ended` is the driver's
    ordering: every `HostRead` (and `BeaconClose`) the run needs is released through
    `SimBeaconIo` and observed before `VmmExit` is issued. In production the watcher's
    `drain_guest_report` races the VMM exit against the `EXIT` line with a bounded yield
    drain (`vm_driver.rs:2165-2197`); the model does not cover that race, only the join
    result after the wait has ended.

  **Constraint on the spec (A3, accepted by the user 2026-11-04):** testing.md forbids
  an empty or unmapped handler in a conformance driver, so the model-only actions cannot sit
  in the `step` relation of the module a driver targets. The conformance target is therefore
  the host-input projection `guest_report_host`, with its own `step` and no hazard, invariant
  or witness of its own (the decision's hazard is `gr-hazard-no-drain-before-exit` in the full
  module; the reader's rule is a guard of `HostRead`):
  - **State.** `rset` (the reported set of declared pairs); `exitrep` (`none`,
    `guest-reported`, `no-report`, projected from the exit observer's row as pinned above); model-only `wait` (waiting or ended),
    `lines` (the scripted input not yet consumed: `listen(p)`, `exit`, `junk`), `eof`, and the
    guest status the reader holds once it has read `exit`. The declared set is a module
    constant; the pair of a valid line ranges over the declared pairs and one undeclared pair.
  - **Actions:** `HostRead`, `AppendListen(p)`, `AppendExit`, `HostGarbage`, `BeaconClose`,
    `VmmExit`, and the stutter `Idle` (mapped as in the table above, DG-M). Appends are unordered with respect to each other (the guest's drain rule is not
    a host concern), so the projection reaches `listen` after `exit`, which is the host
    reader's own rule. `VmmExit` is enabled once the guest-report wait has ended (`exit` read,
    or EOF reached); it sets `exitrep` to `guest-reported` if the reader holds a guest status,
    else to `no-report`, and is the only action that changes `exitrep` from `none`.
  - **Named runs** (each in `checks.toml`, `backend = "evaluator"`, each a `#[quint_test]` in
    the driver, which also carries `#[quint_run]`):
    `host_all_declared_reported` (valid declared lines then `HostRead`s: `rset` reaches the
    declared set; `exitrep` `none`); `host_garbage_line_is_dropped` (`HostGarbage` mid-stream;
    the following `listen` and `exit` still take effect, and after `VmmExit` `exitrep` is
    `guest-reported`); `host_beacon_closes_with_partial_set` (EOF with `rset` partial: the wait
    ends, `rset` stays as it was, `exitrep` stays `none` until `VmmExit`, then `no-report`);
    `host_listen_after_exit_is_ignored` (an `exit` line then a `listen` line: `rset` unchanged
    by the later line; `exitrep` `none` until `VmmExit`, then `guest-reported`).

  **Design gap, surfaced:** `overdrive-init`'s writer (`send(conn: &mut File, ..)`) has no seam that
  records the lines it writes, so the guest half (`ListenBeforeExit`, `DeliveredBeforeExit`)
  has no owner accessor in Tier 1. **User ruling 2026-11-04: accepted.** Conformance covers
  the host half only (`HostRead`, `HostGarbage`, `set`, `exitrep`); the guest half
  (`ListenBeforeExit`, `DeliveredBeforeExit`, the drain rule) is covered by Tier-3 real-boot
  coverage: the V36 real-guest boots (1,000 boots, 0 `LISTEN` after `EXIT`) and DISTILL's
  Tier-3 scenario G-10 (9). There is no writer seam in `overdrive-init`. The recorded gap:
  the guest writer's ordering is not replayed against the model step by step.

### Spike table — contract-shaping validation items V34 to V39, V41 and V42

Per `.claude/rules/design.md` § "Run contract-shaping validation before the design is
accepted", a falsified item is a design defect, remediated with user approval. The spikes ran
as `listen-hooks` (V34, V35, V37, V39), `guest-signal` (V36, V38) and `boot-cost` (V41). Spike
code is throwaway under `spike-scratch/`, aya-rs Rust only, run in a real guest VM on the
native metal host with `cargo xtask metal run --`, recording the **guest** `uname -r`
(`.claude/rules/spike.md`). The findings files carry earlier numbers for some items; the
mapping is in `spike/wave-decisions.md`.

| ID | Shapes | Claim | Status |
|---|---|---|---|
| V34 | D13 (ADR-0272) | A sockops `TCP_LISTEN_CB` arm at the cgroup2 root fires once per listen transition for an unmodified workload, port and address readable | **Discharged (held)**, 13 of 13 cases, `findings-listen-hooks.md`, `7.0.0-38-generic`. Precision limit: `V6ONLY` unreadable (D13 item 8). A backlog-only re-`listen()` emits none; `shutdown(SHUT_RD)` then `listen()` emits a second event on the same port (duplicates occur, D13 item 5) |
| V35 | D13 (ADR-0272) | `CgroupSock` post-bind4/6 reports an explicit UDP bind and not autobind; aya can attach the types; loopback is distinguishable | **Discharged (held)**, 13 of 13, same findings and kernel. Two programs are required, both return 1, both filter protocol 17 (D13 item 7); `IP_BIND_ADDRESS_NO_PORT` and `sendto` autobind are never reported |
| V36 | D13 (ADR-0268), D3 | A hook attached before `READY` misses no listener | **Discharged (held)**: 1000 of 1000 real guest boots reported the TCP listen and the UDP bind, none before `EXEC`; negative control (hooks attached after the workload) 0 of 50; `findings-guest-signal.md`, guest kernel `7.0.0-38-generic`. Pins the drain-before-`EXIT` rule |
| V37 | D13 (ADR-0272, ADR-0268) | The listen op can be an arm in the existing sockops program within the verifier budget, correlation unchanged; ring overflow is fail-closed | **Discharged (held)**: merged arm 750 of 1,000,000 verified instructions (+112), correlation scenario identical against both objects, `seen == delivered + drops` in 12 of 12 flood runs, overflow costs the workload nothing. A second sockops attach does not replace the first (aya links imply `BPF_F_ALLOW_MULTI`). Ring size not established (D13 item 9) |
| V38 | D13 (ADR-0268) | `LISTEN` delivery over the beacon vsock is fast and lines stay intact beside a concurrent `EXIT` | **Discharged (held)**: round-trip p99 138.8 us over 100 000 iterations, 0 malformed with one ordered writer, corruption with two unsynchronised writers (single-writer rule), reader fix (D13 item 3); `findings-guest-signal.md` |
| V39 | D13 premise | The guest kernel has the config the hooks need | **Partial**: held on the metal fixture kernel `7.0.0-38-generic` (`/boot/config-…`; the fixture guest kernel is byte-identical to it; all program types load and attach). **Residual:** the operator-example guest kernel config was not located and no `IKCONFIG` is embedded; the requirement list (`CONFIG_BPF_SYSCALL`, `CONFIG_CGROUP_BPF`, `CONFIG_INET`, `CONFIG_IPV6`, `CONFIG_DEBUG_INFO_BTF`) goes to the appliance image config assertion (ADR-0068 §7). Not contract-shaping |

| ID | Shapes | Claim | Status |
|---|---|---|---|
| V41 | D13 (ADR-0268, ADR-0272); the cold-start budget | The listen programs add an acceptable cost to the pre-`READY` path of a first boot | **Discharged (held)**, `findings-boot-cost.md`, guest `7.0.0-38-generic`, 40 interleaved boots per variant: **+0.36 ms at p50** (paired p90 +1.29 ms, max +2.10 ms) against a same-toolchain baseline; the +20 ms bar was missed by about 50 times; today's pre-`READY` BPF work is 116.8 ms p50. PROMOTE. PARTIAL on one point: the guest PID 1 is an instrumented copy of the real init (equivalent by control, A minus R -1.1 ms; the copy was deleted). Unmeasured: the production merge and the PR-2 shape, the forwarder, drain and write path, a 4 MiB ring (the spike used 1 MiB) |
| V42 | D13 PR-2 (ADR-0272); **contract-shaping; held, gates PR-2 (now Accepted in contract shape)** | With the listen programs attached to a workload cgroup that init creates and starts the workload in, on the real init: init's transport binds produce no report; the workload's listeners (and its forked children's) are all reported; there is no window in which the workload runs outside the cgroup; the existing correlation and connect programs still see the workload's sockets; the boot cost stays in the V41 range | **Discharged (held), 2026-11-04, PROMOTE PR-2**, `spike/findings-workload-cgroup.md`, guest kernel `7.0.0-38-generic`, Cloud Hypervisor v53.0.0 on native metal (`systemd-detect-virt: none`). Observed: 1000 of 1000 exact workload reports (400 single, 300 child-only, 300 parent-and-child), 0 from init, counters match events, 0 drops; negative controls hold (workload outside the cgroup 0 of 150; hooks at the root: 50 of 50 report init's `61000` and `61001-62024`); root programs effective in the child (flow stream header byte-identical, 80 of 80; absent 20 of 20 without the root sockops program); cost **+0.88 ms p50** (p90 +1.66, max +2.49) with the programs in the existing object, **+20.4 ms** for a second `Ebpf::load`. Pins P-a to P-d in D13 item 1. **The `CLONE_INTO_CGROUP` claim (single home; P-c and the model README GR-4 point here):** the workload is created directly in the cgroup, the atomic form of writing the child pid to `cgroup.procs` (`include/uapi/linux/sched.h:38`, `kernel/fork.c:2840-2842`, `kernel/cgroup/cgroup.c:6769-6838 @ v7.0`; without the flag a forked child takes its parent's css_set, `:6791-6794`). **Reproducing the spike** depends on `gen_init.py`, which regenerates the real init at build time from `crates/overdrive-init/src` with eight anchored patches, so a later `overdrive-init` change can break an anchor; the committed evidence (`raw-*.log`, `summary-*.txt`) is the record. Not shown: the production reader and beacon `LISTEN` path (Tier-3 boot scenarios), the missing-workload-binary exit-127 mapping (DELIVER note). **Cost breakdown (the one home for the V42 figures; D13 item 6 cites this row):** 40 interleaved boots per variant, two runs within 0.2 ms; carrying the programs and their two maps without attaching -0.01 ms; the workload cgroup `mkdir` plus placement +0.01 ms (0.07 ms absolute); attaching the three programs 0.56 ms. A second `Ebpf::load` of a separate object costs +20.4 ms p50 because aya 0.13.1 `EbpfLoader::new()` parses kernel BTF eagerly (`aya-0.13.1/src/bpf.rs:169-171`; `.btf(None)` does not help), which meets the +20 ms falsification bar; this is the reason for P-a. The same-object arms use a spike-built copy of the guest object (the xtask-built object loads about 2.8 ms slower in this guest; not investigated). Not measured: the ring was 1 MiB; the forwarder thread, the `LISTEN` write path and the drain were not timed; one guest kernel, a warm host |

### Findings surfaced, not changed

- **F-7.** The default startup probe is materialised only by the TOML parser; the API
  ingress treats an omitted list as the opt-out. Pre-existing; unchanged by D15.
- **F-8.** ADR-0058's inferred probe targets `listeners[0]` over TCP whatever the protocol,
  and a UDP listener cannot be probed (TCP-only `listen_ports`); a UDP-first Service never
  passes today. D15 removes the case.

## Wave: DESIGN / [REF] Proposals not included in the design (the user decides)

Per `.claude/rules/design.md` § "No unrequested scope", these are **not** part of
D1-D12. P-3, P-4 and P-5 are **not in scope by the user's ruling of 2026-11-02**. P-1 and
P-2 remain proposals for the user. Also not included: gating the readiness role on `Stable`
(D9 gates liveness only), an operator-configurable progress deadline, a persisted
deployment condition, an attempt number on `last_attempt`, and making `overdrive deploy` of
an unchanged spec resume an operator-stopped workload (D11 reports the stopped state;
`overdrive workload restart` resumes).

- **P-1:** a `dst-lint` clause pinning the hook call's placement in `exit_observer.rs`.
  Cost: one xtask scan plus tests; S-1 and S-3 already fail on that refactor. Not recommended.
- **P-2:** correct the S-SVM-16 docstring (RCA § 6). Documentation only.
- **P-3 (not in scope, user ruling 2026-11-02):** a non-terminal Service stream event per
  failed attempt; silence between the first crash and the give-up is accepted.
- **P-4 (not in scope, user ruling 2026-11-02):** the attempt count on the early-exit reason.
- **P-5 (not in scope, user ruling 2026-11-02):** classifying a pre-startup clean exit as
  `EarlyExit { exit_code: Some(0) }`; it gives up as `BackoffExhausted`, as today.
- **P-9:** make the shim's `StopAllocation` write refuse to overwrite a terminal `Failed`
  crash row the exit observer authored (closes E-2). Prevents losing one attempt's crash
  cause when a crash lands between a probe stop's evaluation and its dispatch. Cost: it
  changes the stop arm's accepted rule that its row wins the re-read
  (`action_shim/mod.rs:3287-3355`), which also governs operator and liveness stops, so it
  needs a per-disposition rule and seeded scenarios. Cheaper alternative: the stated
  exposure (one attempt's label, never a restart decision). Not recommended.
- **P-12:** an exec-to-`Stable` latency benchmark (guest-listener gate versus the inferred
  poll, with and without the D17 wake; per-hop queue time; the listen ring's drop counter on
  a healthy start). It is a benchmark, not a gate or an expectation, and
  `.claude/rules/testing.md` requires an approved methodology and baseline policy first.
  Cost: 30 runs per arm, three arms, so about 90 real microVM boots on the metal host plus
  the harness and a methodology note; no CI lane. No decision depends on it. Cheaper
  alternative: none needed for correctness; the claim "no 2 s floor" follows from the
  mechanism (the `Pass` is written on the insertion, D13 item 10).

## Wave: DESIGN / [REF] Open questions

No contract-shaping item is open. The last one, **V42** (the PR-2 hook scope), is held
(2026-11-04). No user decision is awaiting; P1, A1, A3, R1 and R2 are accepted
(§ Approval record).
Process items:

1. **ADR map.** D3 is ADR-0256, D5 is ADR-0257, D6 is ADR-0258, D7 is ADR-0259, D8 is
   ADR-0260, D9 is ADR-0261, D10 is ADR-0262 (deadline) and ADR-0264 (`last_attempt`
   detail), D11 is ADR-0263; ADR-0255 keeps D1, D2 and D4 as one decision. B1 is ADR-0265,
   B2 is ADR-0266, D12 is ADR-0267 (amends the operative ADR-0096 with a dated note), D17 is
   ADR-0273; B3 is text in ADR-0258. D13 to D16 are ADR-0268 to ADR-0272.
2. **Gate:** the independent DESIGN review approved this revision at iteration 4
   (`design/review.md`); no modeler work is outstanding. On
   approval each of ADR-0268 to ADR-0272 moves to Accepted (ADR-0058 stays operative until
   ADR-0270 is accepted).
3. At landing, `.claude/rules/development.md` § "Reconciler I/O" ("Steady-state tick") and
   the `spawn_convergence_loop` doc comment gain the D17 wake; the user's rules docs are not
   edited by the architect.

### Findings outside this feature's scope (surfaced, not changed)

F-1 is GH #335 and F-2 is GH #336. F-3 was brought in as D7. F-4 was included by ruling
(S-37). F-5 is real and fixed by D11. F-6 is GH #340 (type Bug), not changed here.

- **F-1 (GH #335): `last_terminated` and `restart_count` (ADR-0078) are never populated on
  the production restart path.** ADR-0105 gives each restart a fresh allocation id and the
  shim builds the successor row with no prior (`action_shim/mod.rs:2982-2996`), so the
  `Restarts` column and `last terminated:` block of `workload describe` (`render.rs:1142`,
  `:1157`) read 0 and absent. The predecessor rows preserve each crash (V9).
- **F-2 (GH #336): the wire `restart_budget` does not report a liveness give-up as
  exhausted** (`handlers.rs:1399-1407` matches `BackoffExhausted` only). D6 and D7 avoid
  adding instances for `EarlyExit` and `StartupProbeFailed`.
- **F-4 (included by ruling, 2026-11-02): the golden fixture ADR-0087 § Compliance required
  for `StoppedBy::LivenessProbe` was absent.** `schema_evolution/alloc_status_row.rs` had
  only `FIXTURE_V1`; it is added with D7's `StartupProbe` fixture as DISTILL obligation S-37.
- **F-5 (verified by code reading, 2026-11-02; fixed by D11): an `Unchanged` redeploy of an
  already-`Stable` Service waits for a terminal event that never comes and ends
  `Failed { Timeout }`.** The Service stream (`streaming.rs:1010-1087`) subscribes to the
  lifecycle bus and reads no observation row; `Stable` was published once and is not
  re-announced (`service_lifecycle.rs:473-479`); the stream hits its cap timer
  (`:1075-1083`). The Job stream recovers a recorded terminal (`:217-234`). A `Stable`
  committed between enqueue and subscription, and a lagged subscriber, share the root
  cause; D11 closes all three.
- **F-6 (GH #340, type Bug; not changed here): the Job stream's snapshot recovery absorbs an
  observation read failure** (`workload_terminal_from_snapshot`, `streaming.rs:585`,
  `.ok()?`) and orders rows of different allocations by the per-key `updated_at.counter`
  (`:586-589`). D11 copies neither and does not change the Job stream.

## Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDR | Impact |
|---|---|---|---|
| DESIGN#Executable obligations | S-1 to S-56 and the D14-E2 input restriction are the scenario authority (numbers not in the table below are unused; S-37, the F-4 fixture, is folded into S-25); each has a production owner path, a fault stimulus and an oracle | D1 to D17 | Every row is covered by an executable Rust body or listed as a waiver or gap below. No `.feature` file exists (`.claude/rules/testing.md`). |
| DESIGN#Formal protocol model | The stateful `*_conf` modules with named runs are the conformance targets; `guest_report` (full) is a model check only | D1, D3, D5 to D11, D13 | Six `quint-connect` drivers exist, one `#[quint_test]` per named run plus `#[quint_run]`. The guest half of `guest_report` is Tier-3 real-boot evidence (user ruling); no writer seam was added to `overdrive-init`. |
| DESIGN#Lifecycle Gate Ownership | G-1 to G-10 and their "Required boundary scenarios" (items 1 to 15) | D1 to D15 | Each gate has a gate-available, gate-unavailable and unaffected-state test. Three G-10 scenarios are not authored (DG-8). |
| DESIGN#Interface contract changes | Exact pinned shapes only; no invented API | D1 to D17 | Scaffolds carry exactly the pinned shape and panic with `RED scaffold`. Where the design does not pin a shape the test stops and the gap is routed to DESIGN (DG-1 to DG-12). |
| DESIGN#Open questions | V39, P-12, F-1/F-2/F-6 (GH #335, #336, #340) and IPv6/V6ONLY exposure (GH #308) stay open | n/a | No test is written for them (see Carried open items). |
| CLAUDE.md, `testing.md` | Pre-authored acceptance bodies; reasoned `#[ignore = "pending DELIVER: <decision> <behaviour>"]`; names describe behaviour, not roadmap steps | n/a | DELIVER removes the marker, records a semantic RED, and does not edit the body. |

## Wave: DISTILL / [REF] Reconciliation

Prior-wave reconciliation recorded 0 contract contradictions at the time it was written. There is no `discuss/` and no `devops/` directory (warned; acceptance criteria derived from DESIGN, default environment matrix, no story-to-scenario traceability). A prior version claimed DESIGN approval at review round 7 without a recorded artifact or verdict; that claim remains withdrawn (F-06). The actual independent full DESIGN review approved the current D1–D17 revision at iteration 4 on 2026-11-09 (`design/review.md`), including D16-G1 under the user's quoted greenfield waiver. This does not replace an independent DISTILL review. The `spike/` findings were read; V39 stays open and is not tested. Policy: `docs/architecture/atdd-infrastructure-policy.md` was inherited; its Python-pilot machinery does not apply (Rust workspace, universe-bound assertions are written as exact before/after observations over port-exposed names).

## Wave: DISTILL / [REF] Scenario coverage

Path prefixes: `CP` = `crates/overdrive-control-plane`, `CORE` = `crates/overdrive-core`, `REC` = `crates/overdrive-reconcilers`, `WRK` = `crates/overdrive-worker`, `CLI` = `crates/overdrive-cli`, `SIM` = `crates/overdrive-sim`. `T/a` = `tests/acceptance`. State at HEAD: **ign(D)** = complete body, `#[ignore = "pending DELIVER: D..."]`, fails for the named missing behaviour; **pass** = runs and passes at HEAD (unaffected-state control or behaviour that already holds). Seeded tests print their seed. The composed tests drive the production observer, reconcilers, shim, `ProbeRunner` and stream handler; the doubles sit at the driven ports only.

| S-ID | Decision / gate | Test file::name(s) | Tier / lane | State at HEAD |
|---|---|---|---|---|
| S-1 | D1, G-1 (#334) | `CP/T/a/terminal_hook_ownership.rs::a_crashed_allocation_stops_probing_as_soon_as_the_observer_consumes_the_exit` | T1 composed, 12 seeds, default | ign(D1) |
| S-2 | D1, D4 (#318) | `terminal_hook_ownership.rs::a_restart_leaves_only_the_successor_supervised_without_a_second_hook_call` | T1 composed, default | ign(D1) |
| S-3 | D1 outcome independence | `terminal_hook_ownership.rs::a_crash_ends_probe_supervision_even_when_every_row_write_attempt_fails`, `::a_crash_with_no_prior_row_still_ends_probe_supervision` | T1 composed, default | ign(D1) x2 |
| S-4 | D3, G-2 | `terminal_hook_ownership.rs::a_start_registers_probe_supervision_before_it_releases_the_exit_gate`, `::an_exit_available_the_instant_the_gate_releases_leaves_no_supervisor` | T1 composed, seeded, default | ign(D3) x2 |
| S-6 | D4 retained hooks | `terminal_hook_ownership.rs::an_operator_stop_ends_probe_supervision_exactly_once`, `::finalizing_a_live_allocation_ends_probe_supervision_exactly_once` | T1 composed, default | pass |
| S-7 | D2 idempotence | `terminal_hook_ownership.rs::the_terminal_hook_is_idempotent_and_a_no_op_for_unknown_ids` (pass); `::a_crash_then_a_finalize_fire_the_hook_twice_with_one_effect` | T1, default | pass; ign(D1) |
| S-8 | D1 unaffected / disabled | `terminal_hook_ownership.rs::a_crash_leaves_the_siblings_supervision_and_row_untouched`, `::a_driver_with_no_probe_runner_takes_the_default_path` (pass); `::a_crash_ends_only_the_crashed_allocations_supervision` | T1, default | pass x2; ign(D1) |
| S-9 | all | see § Conformance drivers | conformance, `integration-tests` | ign per run; 7 pass unignored (5 `restart_authority`, 2 `stream_snapshot`) |
| S-10 | D5, D1, G-3 | `CP/T/a/crash_ending_authority.rs::crashed_allocation_with_exhausted_startup_probes_is_restarted_not_finalised`, `::crash_after_the_deadline_before_the_first_look_is_restarted_not_finalised` (D5); `::crash_with_a_partial_streak_is_not_probed_after_the_crash` (BC-1, D1) | T1 composed, default | ign(D5) x2; ign(D1) |
| S-15, S-23, S-45 (grid) | D6, D7, B3, G-4, G-5 | `CORE/T/a/workload_lifecycle_ceiling_classification.rs::ceiling_terminal_names_the_final_attempts_cause`, `::an_already_finalised_row_is_not_finalised_again` (ign); `::ceiling_terminal_of_unclassified_rows_is_unchanged`, `::job_crash_at_the_ceiling_never_names_a_service_failure`, `::restart_below_the_ceiling_ignores_the_classification_inputs` (pass) | T1 pure proptest, default | ign(D6 D7 B3) x2; pass x3 |
| S-16 | D6, G-4 | `crash_ending_authority.rs::pre_startup_crash_loop_ends_the_stream_with_early_exit_at_give_up`, `::pre_startup_crash_then_a_passing_successor_ends_the_stream_stable`, `::control_plane_restart_before_the_ceiling_decision_rederives_early_exit` (ign); `::crash_loop_after_a_recorded_startup_pass_gives_up_backoff_exhausted` (pass) | T1 composed, default | ign(D6) x3; pass |
| S-45 | B3 | `crash_ending_authority.rs::out_of_memory_crash_loop_gives_up_as_early_exit_without_an_exit_code` | T1 composed, default | ign(B3) |
| S-17 | D6, D7 hydration | `CP/T/a/workload_lifecycle_startup_probe_hydration.rs::current_attempts_startup_status_is_hydrated_for_a_terminal_row`, `::a_superseded_attempts_startup_rows_are_never_read`, `::a_failed_startup_row_read_is_an_observation_read_error` (ign); `::no_startup_status_is_hydrated_without_a_result_row_or_for_a_live_row` (pass) | T1, default | ign(D6 D7) x3; pass |
| S-19 | D6 render | `CLI/T/a/service_early_exit_render.rs::early_exit_renders_multiline_block_with_exit_code_and_stderr_tail_and_no_elapsed_line`, `::early_exit_without_exit_code_renders_guidance_and_no_elapsed_line`, `::early_exit_zero_render_includes_service_kind_guidance`, `::signal_killed_render_says_killed_by_signal`, `::service_kind_render_never_contains_took_live` (proptest) | T1 pure, default | pass |
| S-21 | D5, D7, G-3, G-5 | `CORE/T/a/service_lifecycle_startup_stop.rs::startup_probe_failure_stops_only_a_live_allocation` | T1 pure proptest, default | ign(D5 D7) |
| S-21 (rewrites of old-behaviour tests) | D6, D7 | `CORE/T/a/service_lifecycle_reconcile_branches.rs::startup_probe_failure_stops_the_allocation_when_all_three_gates_are_met`, `::reconcile_startup_stop_clears_mid_window_and_dedups`, `::startup_probe_failure_suppresses_liveness_stop_same_tick`; `CP/T/a/service_lifecycle_stable.rs::given_startup_probe_exhausts_attempts_when_reconcile_then_emits_startup_stop`; `SIM/T/a/service_kind_vm_terminal_invariant.rs::terminal_state_wins_and_dead_vm_backend_never_returns_to_eligibility` (D5 D6 D12) | T1, default | ign(D7) x4; ign(D5 D6 D12) |
| S-22 | D7, G-5 | `CP/T/a/startup_stop_authority.rs::never_ready_guest_is_stopped_and_restarted_until_give_up_names_the_startup_failure`, `::never_ready_guest_with_the_default_startup_window_times_out_naming_the_first_stop` (D7/D10), `::successor_that_binds_after_a_startup_stop_ends_the_stream_stable` (ign); `::opt_out_service_never_receives_the_startup_stop` (pass) | T1 composed, default | ign(D7) x3; pass |
| S-28 | D7 (E-2 race) | `startup_stop_authority.rs::crash_racing_the_startup_stop_leaves_the_stop_row_and_the_give_up_names_startup` | T1 composed, default | ign(D7) |
| S-29 | D8, G-5 | `CORE/T/a/service_lifecycle_startup_stop.rs::startup_stop_is_emitted_once_per_new_startup_row` | T1 pure proptest, default | ign(D8) |
| S-30 | D8, B2, G-5 | `startup_stop_authority.rs::failed_startup_stop_is_retried_once_per_new_startup_row`, `::late_startup_pass_after_a_failed_stop_ends_the_retry_and_reaches_stable` (D8), `::failed_stop_row_write_keeps_the_supervisor_and_the_next_probe_row_retries` (B2, startup stop); `CP/src/reconciler_runtime/liveness_gate_composed.rs::failed_liveness_stop_row_write_keeps_the_supervisor_until_the_retry_lands` (B2, liveness stop) | T1 composed, default / `--lib` | ign(D8) x2; ign(B2) x2 |
| S-25, S-37 | D7 format, F-4 | `CORE/T/a/stopped_by_startup_probe_format.rs` (serde name, rkyv discriminant 6, text, not a reclaim); `CORE/tests/schema_evolution/alloc_status_row.rs::alloc_status_row_stopped_by_startup_probe_decodes_through_current_envelope`, `::alloc_status_row_stopped_by_liveness_probe_decodes_through_current_envelope` (two new fixtures; `FIXTURE_V1` untouched); `api/openapi.yaml` regenerated | T1 pure, default | pass |
| S-26 | D7, D9 stream projection | `CP/T/a/service_stream_restartable_stop_projection.rs::a_restartable_probe_stop_projects_to_no_stream_line` (ign); `::every_other_stop_projects_to_a_stopped_line_naming_its_initiator`, `::the_verdict_terminals_still_project_to_a_stream_line` (pass) | T1 pure, default | ign(D7 D9); pass x2 |
| S-27 | D6, D7 wire and render | `CP/T/a/alloc_status_restart_budget_terminals.rs::an_early_exit_give_up_is_an_exhausted_budget_at_the_ceiling`, `::a_startup_probe_failed_give_up_is_an_exhausted_budget_at_the_ceiling` (ign); `::a_backoff_exhausted_terminal_is_an_exhausted_budget_with_its_attempts`, `::a_liveness_give_up_no_terminal_and_a_probe_stop_leave_the_budget_untouched` (pass); `CLI/T/a/workload_describe_startup_stop.rs::describe_names_a_startup_stopped_predecessor_and_shows_the_running_successor` (ign), `::describe_keeps_the_stable_terminal_line_and_adds_no_stopped_line_for_other_rows`, `::describe_renders_no_startup_line_for_an_operator_or_liveness_stop` (pass) | T1 pure, default | ign(D6), ign(D7) x2; pass x4 |
| S-31 | D9, G-6 | `CP/T/a/liveness_begins_at_stable.rs::liveness_supervision_begins_when_the_allocation_becomes_stable`, `::first_liveness_row_lands_one_interval_after_stable_with_streak_one`, `::a_second_stable_hook_spawns_no_second_liveness_task`, `::a_restarted_process_starts_liveness_only_for_the_successor_at_its_own_stable` (ign D9); `::concurrent_stop_and_start_liveness_never_leave_liveness_for_an_ended_id` (64 seeds, ign D9); `::stable_hook_after_the_end_starts_no_liveness`, `::startup_and_readiness_supervision_are_unchanged_by_the_liveness_gate`, `::an_allocation_without_a_liveness_descriptor_never_has_a_liveness_role` (pass) | T1 composed, default | ign(D9) x5; pass x3 |
| S-32 | D9, G-6 | `CP/src/reconciler_runtime/liveness_gate_composed.rs::slow_starting_service_with_failing_liveness_is_stopped_only_after_stable` (8 seeds), `::opt_out_service_starts_liveness_at_its_first_stable`, `::a_service_whose_liveness_keeps_failing_is_restarted_under_the_shared_budget_and_given_up_at_the_ceiling` (restart and give-up leg, DG-9; `quint_cid_lease` composition) | T1 composed, `--lib` (`integration-tests`) | ign(D9) x3. The restart and give-up leg is RED on HEAD ("a liveness row before Stable") and green with a temporary D9 patch (not kept). |
| S-43 | B1, R2-2 | `liveness_gate_composed.rs::a_lost_stable_write_is_announced_again_until_it_commits`, `::a_lost_stable_write_is_announced_again_after_a_control_plane_restart` | T1 composed, `--lib` | ign(B1 R2-2) x2 |
| S-34 | D10 derivation, G-7 | `CP/T/a/progress_deadline_derivation.rs::the_progress_deadline_exceeds_the_startup_deadline_and_keeps_the_floor` (proptest), `::the_default_spec_yields_ninety_seconds`, `::a_long_startup_deadline_lifts_the_progress_deadline_and_a_short_one_keeps_the_floor`; `REC/T/a/startup_deadline.rs::the_startup_deadline_matches_the_values_the_derivation_produced_before_extraction`, `::the_startup_deadline_equals_the_original_derivation_for_every_descriptor_list` | T1 pure, default | ign(D10) x5 |
| S-35 | D10, G-7 | `CP/T/a/service_stream_progress_deadline.rs::a_service_that_becomes_stable_after_ninety_seconds_ends_stable_not_timeout`, `::a_never_ready_service_times_out_at_the_progress_deadline_naming_the_last_attempt`, `::a_give_up_inside_the_progress_deadline_ends_the_stream_with_the_give_up`, `::an_unchanged_redeploy_of_a_stuck_service_opens_a_new_clock_and_reports_only_new_attempts` (ign); `::a_job_stream_keeps_the_streaming_cap_and_its_timeout_line`, `::a_guest_that_boots_at_its_deadline_and_passes_on_its_last_attempt_ends_in_an_honest_timeout` (pass) | T1 composed, `integration-tests` | ign(D10 / D7) x4; pass x2 |
| S-36 | D10 CLI | `CLI/T/a/service_early_exit_render.rs::timeout_block_says_the_workload_did_not_become_stable_and_may_still_be_restarting`, `::timeout_block_with_a_last_attempt_renders_the_labelled_last_attempt_block` (ign D10); `::timeout_block_without_a_last_attempt_carries_no_last_attempt_section`, `::a_give_up_failure_never_renders_a_last_attempt_block`, `::startup_probe_failed_block_is_byte_identical_to_today` (pass); `CP/T/a/service_stream_last_attempt_wire.rs` (decode, omit-when-absent roundtrip, derived schema names `LastAttemptWire`; pass); `CLI/tests/integration/deploy_stream_request_timeout.rs::the_service_lane_waits_the_server_deadline_plus_thirty_seconds` (ign D10), `::the_job_lane_keeps_the_one_hundred_twenty_second_envelope` (pass) | T1 pure + CLI integration (real TLS socket, paused clock; behind `integration-tests`, DG-10) | ign(D10) x3; the rest pass. Litmus (DG-10): the Service lane fails `waited 120.0016s` against the required 180 s because the deadline + 30 s envelope is absent at HEAD; the Job lane passes. |
| S-38 | D8 CBOR evolution, R2-2 | `REC/T/a/startup_last_acted_run_evolution.rs::blob_without_startup_last_acted_run_decodes_with_an_empty_map`, `::startup_last_acted_run_survives_persistence` (pass); `::blob_carrying_the_removed_stable_announced_key_is_ignored` | T1, default | pass x2; ign(R2-2) |
| S-39 | D11, F-5, G-8 | `CP/T/a/service_stream_recorded_verdict.rs::a_redeploy_of_a_stable_service_ends_stable_at_once`, `::a_redeploy_of_a_given_up_service_reports_the_recorded_give_up_at_once`, `::a_redeploy_of_an_operator_stopped_service_reports_the_operator_stop_at_once` (ign); `::a_redeploy_of_a_starting_service_waits_for_the_live_event`, `::a_redeploy_while_a_probe_stopped_attempts_successor_starts_waits_for_the_successor`, `::a_redeploy_while_a_restarted_successor_starts_never_reports_the_superseded_give_up` (pass) | T1 composed, `integration-tests` | ign(D11) x3; pass x3 |
| S-40 | D11, G-8 | `service_stream_recorded_verdict.rs::a_terminal_committed_and_published_before_the_stream_reads_is_reported_once`, `::a_lagged_stream_recovers_the_terminal_it_missed_by_relisting` (ign); `::a_terminal_committed_and_published_after_the_read_is_delivered_live`, `::a_superseded_attempts_give_up_is_never_the_verdict`, `::a_recorded_probe_stop_is_not_a_verdict_and_the_stream_waits`, `::an_event_arriving_with_the_deadline_wins_over_the_timeout` (pass; G-7 `biased` boundary) | T1 composed | ign(D11) x2; pass x4 |
| S-41 | ADR-0037 section 4 | `service_stream_recorded_verdict.rs::the_line_built_from_a_committed_row_equals_the_line_built_from_its_event` | T1 composed | pass (holds at HEAD) |
| S-42 | D11-E1 failed read | `service_stream_recorded_verdict.rs::a_failed_read_at_open_ends_the_stream_with_an_error_and_never_waits`, `::a_failed_relist_after_lagged_ends_the_stream_with_an_error`; each parameterizes `Unreachable`, `Io`, `Envelope`, asserts `ServiceStreamError::ObservationRead` with the original nested variant and payload, one error then end, and no later `Timeout` | T1 composed | ign(D11) x2 |
| S-46 | D12, G-9 | `CORE/T/a/service_lifecycle_startup_stop.rs::startup_probed_allocation_is_a_backend_only_after_its_stable_is_committed` | T1 pure proptest, default | ign(D12) |
| S-47 | D12, G-9 | `CP/T/a/startup_gated_backend_eligibility.rs::startup_probed_allocation_is_a_backend_only_after_its_stable_is_committed` (a), `::readiness_met_before_stable_makes_the_backend_eligible_at_the_stable_commit` (i) (ign); `::successor_of_a_startup_stopped_attempt_is_a_backend_only_after_its_own_stable` (b), `::crash_successor_is_a_backend_only_after_its_own_stable` (g), `::startup_failure_after_stable_does_not_make_the_backend_unhealthy` (e), `::control_plane_restart_after_stable_keeps_the_backend_healthy` (f), `::opt_out_service_is_a_backend_at_running` (h) (pass) | T1 composed, default | ign(D12) x2; pass x5 |
| S-48 | D17 | `CP/src/convergence_loop_wake.rs::a_submit_while_the_loop_is_parked_is_admitted_without_waiting_for_the_tick` (a), `::a_burst_of_submits_coalesces_per_target_and_drains_in_full` (c), `::a_deferred_evaluation_is_not_admitted_early_by_an_unrelated_wake` (f), `::the_admission_order_of_a_burst_is_identical_across_runs_of_one_seed` (DrainOrder, 4 seeds) (ign); `::the_cadence_tick_with_no_submits_drains_nothing_and_keeps_the_loop_alive` (d) (pass) | T1 seeded sim, `--lib` | ign(D17) x4; pass. (b) and (e) waived (DG-9). |
| S-50 | PR-1 / D13 item 10 | `WRK/T/a/guest_listener_board.rs::the_reported_set_only_grows_within_the_declared_set_and_ends_with_the_entry` (proptest) | T1, default | ign(D13) |
| S-51 | PR-1, D13 item 3 | `guest_listener_board.rs::a_pending_changed_completes_on_an_insertion_and_on_no_no_op`, `::a_garbage_line_is_dropped_with_a_warning_and_later_lines_take_effect`, `::a_listen_after_exit_changes_neither_the_set_nor_the_exit_report`, `::guest_listener_reports_equivalence` (board against `SimGuestListenerReports`) | T1, default | ign(D13) x4 |
| S-52 | PR-1 `changed` | `guest_listener_board.rs::changed_never_misses_an_insertion_under_seeded_schedules` (6 seeds) | T1 seeded, default | ign(D13) |
| S-53 | D14 item 3 | `WRK/T/a/guest_listener_probe_rows.rs`: `completing_the_declared_set_writes_one_pass_row_with_no_earlier_row`, `an_incomplete_set_writes_no_row_before_the_deadline_then_one_fail_per_interval`, `the_deadline_fail_reason_names_only_the_missing_listeners`, `a_failed_pass_write_is_retried_until_it_lands_without_advancing_the_counters`, `a_failed_fail_write_advances_neither_number`, `no_row_is_written_after_stop_alloc`, `an_undeclared_or_duplicate_report_never_completes_the_set`, `guest_listener_rows_keep_the_run_numbering_over_random_schedules` | T1 composed, `SimClock`, default | ign(D14) x8 |
| S-54 | P1 | `CP/T/a/service_lifecycle_startup_stop_gate.rs::the_startup_stop_gate_takes_its_attempts_threshold_from_the_mechanic` (proptest), `::guest_listeners_stop_at_streak_one_once_the_deadline_elapsed` (ign); `::dial_mechanics_stop_only_at_max_attempts` (pass) | T1 pure / hydrated, default | ign(D14 P1) x2; pass |
| D14 item 4 (DG-11) | mechanic summary names the declared listeners | `CLI/T/a/probes_section_render.rs::guest_listeners_summary_names_the_declared_listeners_in_declaration_order` (`probes_section` and the CLI formatter take `&[Listener]`); `REC/src/service_lifecycle.rs::guest_listeners_summary_names_the_declared_listeners_in_declaration_order` (reconciler formatter takes `&[Listener]`) | T1 pure, default / `--lib` | ign(D14) x2 |
| S-55 | PR-4 | `CORE/T/a/declared_listeners_tcp_view.rs::the_tcp_view_is_the_tcp_filter_of_the_declared_listeners_in_declaration_order`, `::a_udp_only_declaration_has_no_tcp_ports_and_a_mixed_one_keeps_tcp_order`; `WRK/tests/integration/inbound_rules_only_for_tcp_declared_listeners.rs::a_udp_declared_listener_gets_no_inbound_capture_and_a_tcp_one_does` | T1 proptest; integration (nft, metal) | ign(PR-4) x3 |
| S-56 | D6, DG-12 | `CORE/T/a/service_lifecycle_reconcile_branches.rs::terminal_allocation_of_any_start_history_closes_its_mid_startup_window` (oracle: the returned `View.terminal_announced` contains the allocation, `has_alloc_mid_startup_window()` is false, `next_evaluation_at` is `None`, no action) | T1 pure, enumerated grid (state x `started_at`), default | ign(D6) |
| S-57 | D14-E2 input restriction | `CORE/T/a/guest_listeners_input_rejection.rs::{raw_service_guest_listeners_wrong_role_is_typed, raw_service_guest_listeners_not_sole_startup_precedes_count_check, operator_toml_guest_listeners_is_typed_for_every_role_and_case}`; `CP/T/a/guest_listeners_http_rejection.rs::invalid_guest_listeners_raw_service_is_rejected_at_http_ingress`; `CLI/T/a/probes_kind_rejection_cli.rs::canonical_guest_listeners_token_is_typed_in_each_role_on_both_cli_modes`. The core's `raw_service_guest_listeners_as_sole_startup_is_admitted` is a passing valid-input control. The Job/Schedule TOML kind gate remains covered by existing tests; raw Job/Schedule probe fields cannot enter the typed payload (`deny_unknown_fields`). | T1 pure proptest and in-process ingress (`integration-tests` for HTTP) | ign(D14-E2) x5; valid control pass |
| G-10 (10) | D13 item 8, D14 item 3, Tier 3 | `CP/tests/integration/guest_listener_real_boot.rs::a_loopback_only_bind_never_satisfies_a_declared_listener` (real guest, `run_server`, `POST /v1/workloads`; observes the startup row in `GET /v1/allocs`) | T3 real boot, `integration-tests,kvm-tests`, native metal | ign(D13 guest side); RED on the probe-runner scaffold, no deadline `Fail` row |
| G-10 (9) second clause, `guest_report` guest half | D13 items 1 and 10, Tier 3 | `guest_listener_real_boot.rs::every_declared_listener_a_real_workload_binds_is_reported_before_it_exits` (TCP and UDP listener held, then exit; `Pass` seen while `Running`, before the terminal row) | T3 real boot | ign(D13 guest side); RED on the same scaffold, no `Pass` row |
| DG-6, F-5 | D13 host shared board | `CP/src/guest_listener_board_wiring_acceptance.rs::a_listen_line_on_the_drivers_beacon_completes_the_probe_tasks_declared_set` (`run_server` over `SimVmm`, `SimBeaconIo`, the existing `SimGuestFlowIo` control-session seam, and `SimMtlsIntercept`; undeclared and partial reports change nothing, the completing report writes one `Pass`) | T1 source-local production composition, `integration-tests` in `cgroup-default` (metal) | activated in 05-03; the production share is observed through the startup row |
| G-10 (9a) | none | no test obligation (DG-13: init's binds come from the reserved range, which no Service can declare, and the board ignores an undeclared report) | n/a | resolved, DG-13 |
| D13 item 2 | LISTEN wire form (G-10 "wire parse") | `CORE/T/a/beacon_listen_wire_form.rs::listen_round_trips_for_every_protocol_and_port`, `::listen_with_an_out_of_range_port_is_a_typed_error`, `::malformed_listen_lines_are_rejected_with_a_typed_error` (asserts `BeaconParseError::MalformedListen { raw, detail }` carrying the full line, DG-M) | T1 pure proptest, default | ign(D13) x3 |

Error and boundary tests are above 40 % in every area (ended ids, lost and failed writes, restarts, races, deferrals, same-key bursts, failed reads, undeclared reports).

### Lifecycle Gate "Required boundary scenarios" to tests

| Item | Gate | Covered by |
|---|---|---|
| 1 gate available | G-1, G-2 | S-1, S-4 |
| 2 unavailable or timed out | G-1 | S-3 (write retries exhausted, `NoWrite`); the weak-upgrade-`None` no-op is source-settled (V27), not driven |
| 3 unrelated state | G-1 | S-2, S-7, S-8 |
| 4 late success | G-1, G-2 | (b) S-4 `an_exit_available_the_instant_the_gate_releases_leaves_no_supervisor`; (a) the in-flight probe row after the hook is not asserted (no reader acts on it) |
| 5 replay | G-1 | S-1 (hook fires once per consumed exit); a restart loses probe tasks, not driven |
| 6 feature disabled | G-1 | S-8 `a_driver_with_no_probe_runner_takes_the_default_path`, S-22 opt-out |
| 7 G-3 | D5 | S-10, S-21, S-8 |
| 8 G-4 | D6 | S-15, S-16, S-17, S-45 |
| 9 G-5 | D7, D8 | S-21, S-22, S-23, S-17, S-26, S-29, S-30, S-28 |
| 10 G-6 | D9 | S-31, S-32, S-43; the B2 liveness variant of S-30 |
| 11 G-7 | D10 | S-34, S-35, S-36 |
| 12 G-8 | D11 | S-39, S-40, S-41, S-42 |
| 13 G-9 | D12 | S-46, S-47, S-43, S-30 |
| 14 G-10 | D13 to D15 | S-50 to S-53 (host half), the `guest_report_host` driver, the LISTEN wire form; item (8) ring-drop composed run is authored (`crates/overdrive-control-plane/src/reconciler_runtime/guest_listener_gate_composed.rs::a_service_whose_listener_report_never_arrives_is_stopped_at_the_deadline_restarted_and_given_up`, ignored `pending DELIVER: D13/D14`); item (10) loopback and v4-mapped binds and the second clause of (9) are authored as Tier-3 real-boot tests (`guest_listener_real_boot.rs`); clause (9a) carries no test obligation (DG-13, ruled 2026-11-05); DG-8 (ruled 2026-11-05) pins (8) as a seeded composed run with the omitted `LISTEN` line on `SimBeaconIo` |
| 15 D17 | none | S-48 |

## Wave: DISTILL / [REF] Conformance drivers

All drivers live in the owning crate's `src/` under `#[cfg(all(test, feature = "integration-tests"))]` (no always-false `cfg`), reference the spec in place (`../../specs/quint/service-allocation-readiness-and-ending/<module>.qnt`), hold the production owner or composition as a driver field, map every `step` action (`switch!`, no `_` arm), and project `State` only through the accessors pinned in § Formal protocol model "Pinned conformance projection". They spawn no `quint` process. Full `guest_report` has no driver: it is model-check only, and its guest half (`ListenBeforeExit`, `DeliveredBeforeExit`, the drain rule) is Tier-3 real-boot evidence by user ruling (V36, G-10 item 9); no writer seam was added to `overdrive-init`.

| Module (main) | File and composition | Named runs to `#[quint_test]` fn, plus `#[quint_run]` | Accessors read | Checklist 1 to 7 | Outcome at HEAD |
|---|---|---|---|---|---|
| `hook_ownership` (`hooks_conf`) | `CP/src/worker/quint_hook_ownership.rs`; production exit observer (`spawn_with_runtime`), shim `dispatch_with_network_provisioner`, real `ProbeRunner`; dispatches parked at the model's intra-dispatch boundaries | `crash_without_restart`, `crash_then_fresh_id_restart`, `write_exhaustion_still_stops_probes`, `operator_or_liveness_stop_emits_no_exit_event`, `finalize_on_live_vm_stops_probes`, `double_fire_is_idempotent` as `quint_hook_ownership_<run>`; `quint_hook_ownership_randomized_traces` (100 traces) | `ProbeRunner::is_role_live` (cross-checked with `active_alloc_count`), `ObservationStore::alloc_status_row` | 1 to 7 met. `Die` (guest death) and `Idle` are environment and stutter handlers with no owner call, as the precedent's `VmmCrash`. | 7 ign (D1, D3, B2); fail at `Register` on the pre-D3 order (right reason). With D1, D3, B2 patched in a throwaway tree, 6 runs and 100 traces passed (patch not kept). |
| `restart_authority` (`ra_conf`) | `CP/src/reconciler_runtime/quint_restart_authority.rs`; `AppState` with both reconcilers, shim, exit observer; probe rows written through `SimObservationStore` as inputs; start rejection scripted with `SimDriver::script_next_start_rejection` (sim test support, DG-2); the driver asserts spec `CEIL` equals `RESTART_BACKOFF_CEILING` (DG-1) | 21 named runs as `restart_authority_<run>` at `CEIL = 5`; `restart_authority_conformance_ra_conf` (`#[quint_run]`) | `alloc_status_rows` via `attempts()`, `loaded_service_lifecycle_views_for_test` (`startup_last_acted_run`, `terminal_announced`), `current_alloc`, `loaded_workload_lifecycle_views_for_test` (`restart_counts`), row `terminal`; `author` = reconciler whose tick returned the claim | 1 to 7 met. Deviations stated in the file: returned actions are read from the row terminal the shim wrote. | 5 pass unignored (`crash_after_attempts_exhausted_then_deadline_passes`, `crash_after_startup_pass_gives_up_backoff_exhausted`, `opt_out_service_crash_loop_gives_up_backoff_exhausted`, `non_crash_failed_row_gives_up_backoff_exhausted`, `explicit_probe_fail_after_deadline_keeps_counted_streak`); 16 named runs and the random run are ignored and right-reason RED: D7 x11, D6/D7 x2, D6 x1, DG-12 x1, D14 item 3 x1. No run is blocked by a design gap. |
| `liveness_gate` (`lg_conf`) | `CP/src/reconciler_runtime/quint_liveness_gate.rs` (+ `liveness_gate_fixture.rs`); real `ProbeRunner`, `VmDriver`, `AppState` | `failing_during_startup_is_not_stopped`, `starts_at_stable_and_stops_on_failure`, `stable_after_end_starts_no_liveness`, `lost_stable_write_is_reannounced`, `opt_out_service_starts_liveness_at_first_running_stable` as `liveness_gate_<run>`; `liveness_gate_conformance_lg_conf` | `ProbeRunner::is_role_live`, `active_alloc_count`, `alloc_status_row` (`committed` compared only while `Running`, DG-3) | 1 to 7 met; item 4 carries DG-3 | 6 ign (D9; B1 for the lost-write run); diverge at the first step (`role_live` true before `Stable`). With a temporary D9 patch 4 of 6 passed and the B1 runs failed for B1 (patch not kept). |
| `progress_deadline` (`pd_conf`) | `CP/src/quint_stream_conformance/progress_deadline.rs`; `AppState`, `submit_workload` NDJSON lane | `slow_start_reaches_stable_not_timeout`, `never_ready_times_out_with_last_attempt_cause`, `giveup_inside_deadline_wins_over_timeout`, `redeploy_unchanged_opens_new_clock` as `quint_progress_deadline_<run>`; `quint_progress_deadline_randomized` | stream lines yielded, `progress_deadline(&probes, DEFAULT_STREAMING_CAP)` | 1 to 7 met | 5 runs ign RED on the D10 scaffold (panic `RED scaffold: D10 progress_deadline`). With a temporary body 2 runs passed. |
| `stream_snapshot` (`ss_conf`) | `CP/src/quint_stream_conformance/stream_snapshot.rs`; `AppState`, `build_service_stream` | 7 named runs (`redeploy_unchanged_of_stable_ends_stable`, `terminal_between_subscribe_and_read_is_not_lost`, `restartable_stop_recorded_is_not_a_verdict`, `superseded_attempt_terminal_is_not_reported`, `lagged_relist_recovers_terminal`, `publish_after_relist_reaches_channel`, `failed_read_ends_with_error`) as `quint_stream_snapshot_<run>`; `quint_stream_snapshot_randomized` | the closing `AllocStatusRow` through `ObservationStore::alloc_status_row` (DG-4); the yielded line is an input and oracle | 1 to 7 met; `conformance-lint` clean | `publish_after_relist_reaches_channel` and `superseded_attempt_terminal_is_not_reported` pass unignored; the other 5 named runs and the random run are ign RED (D11, D7) |
| `guest_report_host` (`gr_host_conf`) | `CP/src/worker/quint_guest_report_host.rs`; production `VmDriver`, shared `GuestListenerBoard`, `SimBeaconIo`, `SimVmm`, production exit observer | `host_all_declared_reported`, `host_garbage_line_is_dropped`, `host_beacon_closes_with_partial_set`, `host_listen_after_exit_is_ignored` as `quint_guest_report_host_<name>`; `quint_guest_report_host_randomized_traces` | `rset` from `GuestListenerReports::reported`; `exitrep` from `ObservationStore::alloc_status_row` (three-valued; never the `ExitEvent` receiver) | 1 to 7 met. `Idle` is a read-only `GuestListenerReports::reported(alloc)` call compared with the previous projected value (DG-M). | 5 ign; fail at `RED scaffold: PR-1 GuestListenerBoard::reported` after launch, READY, Running row and observer. The modelled behaviour is not exercised until DELIVER. |

Regression-witness limits (unignored runs, green at HEAD because no owner changed behaviour): `explicit_probe_fail_after_deadline_keeps_counted_streak` has no owner that observes the startup gate (streak and gate state are model-only), so it proves only that the driver replays the run; the D14 item 3 run `first_fail_at_deadline_with_no_earlier_row_stops` and S-53 prove the streak. `crash_after_attempts_exhausted_then_deadline_passes` never ticks `ServiceLifecycle` after the crash, so D5's guard is not exercised there; `running_alloc_with_exhausted_probes_past_deadline_is_stopped` and `startup_never_passes_loop_gives_up_startup_probe_failed` do. `publish_after_relist_reaches_channel` and `superseded_attempt_terminal_is_not_reported` compare only `open` states; `lagged_relist_recovers_terminal` and the stream acceptance tests prove the relist and the supersession. The `control_plane_restart_keeps_acted_run` driver restarts the control plane by rebuilding `ReconcilerRuntime` over the same redb ViewStore (`register` runs the production `bulk_load`) at every `Idle`, and projects from the reloaded views.

The ignored driver runs were re-run with `--run-ignored only` and fail on missing behaviour (53 ignored tests in that run, all right-reason RED). The five unignored `restart_authority` runs, `publish_after_relist_reaches_channel` and `superseded_attempt_terminal_is_not_reported` pass at HEAD.

## Wave: DISTILL / [REF] Production scaffolds

Every scaffold has exactly the pinned shape, a `// SCAFFOLD: true` marker, and (where it has behaviour) a body that panics with `RED scaffold: <decision>`.

| Item | File | Pinned in | Body |
|---|---|---|---|
| `StoppedBy::StartupProbe` (rkyv discriminant 6; `human_readable`, `by_reclaims_platform` = false) | `CORE/src/transition_reason.rs` | D7 item 1 | real arms (S-25 passes) |
| `WorkloadLifecycleState.current_startup_probe: Option<ProbeStatus>` | `REC/src/workload_lifecycle.rs` | D6 item 4, D6 interface changes | always `None` |
| `ServiceLifecycleView.startup_last_acted_run` (`#[serde(default)]`) | `REC/src/service_lifecycle.rs` | D8 item 1 | field only |
| `ServiceAllocFact.stable_committed: bool` | `REC/src/service_lifecycle.rs` | B1, D12 | field only; `false` in fixtures |
| `startup_deadline(&[ProbeDescriptor]) -> Duration` | `REC/src/service_lifecycle.rs` | D10 item 5 | panics |
| `ProbeRunner::start_liveness(&self, &AllocationId)` | `WRK/src/probe_runner/mod.rs` | D9 item 2 | panics |
| `ReconcilerRuntime::submit_evaluation(Evaluation, Instant, EvaluationEligibility)` | `CP/src/reconciler_runtime.rs` | D17 interface row | panics |
| `LastAttemptWire`; `ServiceSubmitEvent::Failed.last_attempt`; `PROGRESS_DEADLINE_MARGIN`; `progress_deadline(&[ProbeDescriptor], Duration)`; `build_service_stream(.., progress_deadline)` | `CP/src/streaming.rs`, `CP/src/handlers.rs` | D10 items 1, 4, 5, 6 | wire types real; `progress_deadline` panics; the new parameter is accepted and ignored |
| `format_service_failed_block(.., last_attempt)` (`early_exit_timing` and `elapsed:` removed); `submit_workload_streaming(req, _server_deadline)` | `CLI/src/render.rs`, `CLI/src/commands/deploy.rs`, `CLI/src/http_client.rs` | D6 table, D10 item 7 | Timeout wording and last-attempt block not implemented; deadline ignored |
| `GuestListenerReports`, `ReportedListeners` | `CORE/src/traits/guest_listener_reports.rs` | D13 item 10 | trait real; `Default`, `FromIterator<Listener>` and `is_empty` pinned by DG-6 |
| `BeaconMessage::Listen { protocol, port }`; `BeaconParseError::MalformedListen { raw, detail }` | `CORE/src/vm/beacon.rs` | D13 item 2, DG-M | Display and FromStr arms panic |
| `ProbeMechanic::GuestListeners` (fieldless tail variant) | `CORE/src/aggregate/probe_descriptor.rs` | D14 item 1 | `validate` arm `Ok`; compile-unblock arms in the worker (panics in `probe_tick`), CLI render and reconciler summary |
| `AllocationSpec.declared_listeners`; `AllocationSpec::tcp_service_ports()` | `CORE/src/traits/driver.rs` | D14 item 2, PR-4 | field beside `service_ports` (DELIVER removes `service_ports`); accessor panics |
| `GuestListenerBoard` (`new()`, port impl); `ProbeRunner::new` sixth argument; `VmDriver::new` last argument | `WRK/src/guest_listener_board.rs`, `WRK/src/probe_runner/mod.rs`, `WRK/src/vm_driver.rs` | D13 item 10 | `new() -> Self` real (pinned, DG-6); port impl panics; call sites pass a fresh board (production wiring must share one) |
| `SimDriver::script_next_start_rejection(reason)` | `SIM/src/adapters/driver.rs` | DG-2 (sim test support) | real implementation, not a scaffold |
| `format_mechanic_summary(.., &[Listener])` (reconciler) and `format_probe_mechanic_summary(.., &[Listener])`, `probes_section(.., &[Listener])` (CLI) | `REC/src/service_lifecycle.rs`, `CLI/src/render.rs` | D14 item 4, DG-11 | `GuestListeners` arm not implemented |
| `SimGuestListenerReports` (`new`/`open`/`insert`/`close`) | `SIM/src/adapters/guest_listener_reports.rs` | D13 item 10 (scripting API is DISTILL's) | real implementation, not a scaffold |

Not scaffolded at DISTILL time: `ServiceAllocFact.startup_attempts_required` (S-54 drives hydration instead), `WorkloadLifecycleState.declared_listeners`, the removal of `inferred` (D16), `BrokerDrain`/`submitted`/`broker()` (D17), and `service_stream_synth_cap_timeout`'s new parameter (then unpinned; exact D10-H1 signature is now in § D10 interface changes). `api/openapi.yaml` is regenerated; the existing `FIXTURE_V*` literals are untouched.

## Wave: DISTILL / [REF] Test placement and lanes

| Lane | Location | Content |
|---|---|---|
| Default (`cargo nextest run`) | `CP/tests/acceptance/*.rs` wired through `tests/acceptance.rs`; `CORE`, `REC`, `WRK`, `CLI`, `SIM` `tests/acceptance/*.rs` | pure, hydrated and composed Tier-1 scenarios over `Sim*` ports; shared composition `hooked_driver.rs` (`HookedDriver`, `NoopNetworkProvisioner`), `restart_authority_world.rs`, `service_stream_support.rs`; shared fault store `tests/common/issue_fault_store.rs` (also injects probe-read faults) |
| `integration-tests` | `CP/tests/acceptance/service_stream_*.rs` (use the convergence-tick entry), `CP/src/**` composed modules (`liveness_gate_composed.rs`, `convergence_loop_wake.rs`) | composed streams, liveness, wake |
| Conformance (`integration-tests`) | `CP/src/worker/quint_*.rs`, `CP/src/reconciler_runtime/quint_*.rs`, `CP/src/quint_stream_conformance/` | six `quint-connect` drivers; they need crate-private constructors and ITF helpers, so they sit beside their owners (precedent `guest_network/quint_cid_lease.rs`) |
| Integration, real infrastructure | `WRK/tests/integration/inbound_rules_only_for_tcp_declared_listeners.rs` (nft); `CLI/tests/integration/deploy_stream_request_timeout.rs` (real TLS socket, paused clock) | host effects Sim cannot observe |
| Tier 3 real boot | `CP/tests/integration/guest_listener_real_boot.rs` (`integration-tests,kvm-tests`, native metal, after `cargo xtask guest-kit-prepare`); the guest application's new `bind` subcommand is test support (`overdrive-testing`) | G-10 item 10, the second clause of item 9 and the guest half of `guest_report`; clause (9a) of item 9 carries no test obligation (DG-13); V34 to V36 and V42 stay the evidence for the tight drain race |

Every cargo, nextest, quint and lint command ran through `cargo xtask metal run --`. No test spawns the `overdrive` binary.

## Wave: DISTILL / [REF] Adapter coverage

| Driven port | Adapter in the scenarios | Real-I/O evidence |
|---|---|---|
| `Clock` | `SimClock` (seeded, advanced explicitly) | none needed |
| `ObservationStore` | `SimObservationStore` with `inject_write_failure` and a delegating read-fault store (`issue_fault_store.rs`) | redb not exercised by this feature |
| `Driver` | `SimDriver` behind `HookedDriver`; real `VmDriver` over `SimVmm` and `SimBeaconIo` (S-31, S-32, conformance) | microVM boot is Tier 3, not authored |
| TCP prober, guest stream dial | `SimTcpProber`, `SimGuestStreamDial` | none |
| `GuestListenerReports` | real `GuestListenerBoard` (via the reader) and `SimGuestListenerReports`, held equal by `guest_listener_reports_equivalence` | real guest reports: Tier 3, `guest_listener_real_boot.rs` (DG-8); the shared board: `guest_listener_board_wiring_acceptance.rs` (DG-6) |
| Network provisioner | `NoopNetworkProvisioner` | inbound capture install: nft integration test (PR-4) |
| Intent store, view store | `LocalStore`, the runtime's view store (restart tests reload it) | none |
| Operator HTTP stream | production handler and `ApiClient` | CLI request timeout over a real TLS socket |

## Wave: DISTILL / [REF] Gate results

| Gate | Result |
|---|---|
| `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | clean |
| Default lane, `cargo nextest run --workspace` | 3606 passed, 17 failed. 15 fail identically at HEAD (pre-existing): `service_projection_equals_declared_listener_set`, `a_vmm_exit_ends_the_launch_and_releases_its_claim_for_the_next_launch`, `quint_flows_beacon_vm_exit_ends_launch_and_releases_claim`, and 12 `quint_cid_lease_*` runs. The other two are flaky and not DISTILL regressions: seed-dependent `flows_conf_randomized_traces`, and `action_shim::replacement_acceptance::successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver` (pre-existing race in its first `tokio::select!`; 6 of 40 runs fail at `076be27d9`; this feature does not address it). |
| Control-plane quint lane, `--features integration-tests -E test(quint_)` | 146 passed, 12 failed (the `quint_cid_lease` set, pre-existing). |
| Ignored tests, `--run-ignored only` | 53 ignored tests, all fail on missing behaviour (right-reason RED); none fails on setup, import or fixture. |
| `cargo xtask dst-lint` | pass |
| `cargo xtask conformance-lint` | 0 violations |
| `cargo clippy` / `cargo fmt` | clippy `-D warnings` clean; fmt clean |
| CLI `cli_submit_streaming_service_routes_to_service_submit_event_consumer` (real server) | times out at HEAD `e5c233ba7` as well: pre-existing, not a DISTILL regression |
| 2026-11-09 D11-E1/D14-E2 focused metal runs | The valid raw-Service `GuestListeners` control passed. Three new core D14-E2 bodies, the HTTP body, the CLI body and the two amended S-42 bodies failed for their named absent behaviour; all compiled and reached their production driving boundary. Details and first failure points: `distill/red-classification.md` § 2026-11-09. The `infra/metal/test-placement.json` additions classify the six new tests in the existing `host-pure` lane; the first focused control attempt returned `test_unclassified` before its body because its placement was absent, then passed after the inventory update. |
| Focused unaffected controls on Metal | Five passed: raw sole-startup `GuestListeners` admission, the D11 live-after-read Service stream, the pre-existing Job probe-kind CLI rejection, the `quint_stream_snapshot_publish_after_relist_reaches_channel` conformance run, and the Job stream's unchanged-resubmit acceptance. The latter two were run through the control-plane `--lib --test acceptance --features integration-tests` lane. |
| 2026-11-09 compile/lint/format of bounded amendment | `cargo xtask metal run -- cargo clippy -p overdrive-core -p overdrive-control-plane -p overdrive-cli -p overdrive-reconcilers --all-targets --features integration-tests -- -D warnings` passed. `rustfmt --check` passed on every Rust file changed by this DISTILL remediation; `git diff --check` and both edited JSON parses passed. The Metal workspace `cargo fmt --all -- --check` fails solely on pre-existing clean `xtask/tests/acceptance.rs` module ordering; no DISTILL-remediation file appears in its second run's diff. |
| Independent DISTILL review iteration 3 F-01 remediation | The reviewer's exact scan initially found 122/156 scheduled pending DELIVER tests without a per-test Contract Shape declaration. All 122 now carry one immediately before their pending marker; the same scan returns 0/156 missing. This is test metadata only; bodies, assertions, ignores and roadmap activations are unchanged. Changed-file `rustfmt --check`, `git diff --check`, and Metal Clippy across the five affected crates with `--all-targets --features integration-tests -- -D warnings` passed. The independent reviewer approved the full suite after re-review at iteration 4 in `distill/review.md`. Its F-02 mapped-nonloopback and F-03 S-53 schedule proposals were withdrawn and were not implemented. |
| D11-E1 / D14-E2 typed-error contract audit | Open and `Lagged` relist read faults each map `Unreachable { peer }`, `Io(error)` and `Envelope { source }` to `ServiceStreamError::ObservationRead` retaining the original source; line emission maps to `LineEmission { source: io::Error }` with no artificial emitter seam. Raw Service role and cardinality violations map to the exact two `AggregateError` variants, then `ControlPlaneError::Aggregate` and HTTP 400 `validation` with the specified field; canonical case-insensitive operator TOML maps to `ParseError::GuestListenersNotOperatorConfigurable`, then `CliError::ParseError` and exit 1 in both deploy modes. The authored tests reject every other error variant. No material typed-error gap remains. |
| Review | Sentinel round 1 `needs_revision` (3 blockers, 6 high, 5 low): the restart driver now calls hydrate, reconcile and shim dispatch and reads `claim`, `author` and `done` from the returned actions; the liveness handlers were exercised with a temporary D9 patch; the hook drivers compile and are clippy-clean; the B2 liveness-stop test was added. Subtractive review applied: four duplicate "Failed row yields no action" rewrites and a checked-in-openapi test deleted, one shared `HookedDriver` replaced three per-file drivers, the probe-read fault store merged into `issue_fault_store.rs`, an rkyv roundtrip duplicate and dead helpers deleted, `terminal_announced` assertions removed from the sim invariant test (DG-12). Remaining findings are the design gaps below. |
| Current DISTILL review status | **APPROVED at independent full-suite review iteration 4** (`distill/review.md`). The historical round-1 verdict and the iteration-2/3 findings and dispositions are preserved there. `deliver/roadmap.json` validation remains `pending` for its separate independent roadmap review. The bounded self-audit is `distill/contract-remediation-audit.md`. |

## Wave: DISTILL / [REF] Design gaps routed to DESIGN

**All thirteen gaps (DG-1 to DG-13) and the minor items were ruled on 2026-11-05**; the approval is recorded in § Approval record. Each gap was a shape or model fact the design did not pin; DISTILL did not improvise (CLAUDE.md "Implement to the design"). DISTILL applied the rulings: S-56 and the composed restart and give-up leg of S-32 are authored, and the specs, drivers and scaffolds follow the pins below. **DG-1 to DG-13 are closed. DISTILL is complete and independently approved at iteration 4.** Detail and file references: `distill/upstream-issues.md` and `distill/review.md`.

| ID | Ruling (2026-11-05) | Recorded in |
|---|---|---|
| DG-1 | `ra_conf` at `CEIL = 5`; give-up runs for five restarts; the driver asserts spec `CEIL` equals `RESTART_BACKOFF_CEILING`; no seam | § Formal protocol model, "Conformance abstractions" |
| DG-2 | The start outcome is a nondeterministic parameter of `Deploy` and `WLRestart`; no `pending` row, no `StartOk` or `StartRejected`; rejected-start run kept; `SimDriver` start-failure flag is test support | same |
| DG-3 | `committed` compared only while `Running`; model-only after `Crash` | § Pinned conformance projection |
| DG-4 | The compared state is the closing `AllocStatusRow` via `alloc_status_row`; the yielded line is an input and oracle | same |
| DG-5 | Ordered channel; `Deliver` oldest-first and disabled while `lagged`; `Lag` abstraction stated; `StableHappens` disabled after `OperatorStop` | § Formal protocol model, "Conformance abstractions" |
| DG-6 | `ReportedListeners`: `Default`, `FromIterator<Listener>`, `is_empty`; `GuestListenerBoard::new() -> Self`; DELIVER shares one board | D13 item 10, § Interface changes (D13 to D16) |
| DG-7 | `progress_deadline` last in `build_service_stream`; server deadline second in `submit_workload_streaming`; `LastAttemptWire` in `overdrive_control_plane::streaming` | § D10 interface changes |
| DG-8 | (9b) and (10) Tier-3 DELIVER obligations observed through the startup probe row; (8) seeded composed run with the omitted `LISTEN` line on `SimBeaconIo` | Gate G-10 |
| DG-9 | S-32 composed leg required (`quint_cid_lease` composition); S-48 (b) waived, carried by the mutation gate; S-48 (e) waived | S-32, S-48 |
| DG-10 | `tokio` `test-util` approved; S-36 behind `integration-tests` with a litmus proof | S-36 |
| DG-11 | Both formatters take `&[Listener]`; no wire field | D14 item 4 |
| DG-12 | Every `Failed` or `Terminated` fact joins `terminal_announced`; new obligation S-56 | D6 item 1, S-56 |
| DG-13 | G-10 (9a) carries no test obligation: the failure is unreachable (init's binds come from `guest_reserved_ports()`, the Service validator rejects those ports with `ListenerPortReserved`, the board ignores an undeclared report, PR-2 keeps init outside the hooked cgroup). Clause (9b) stays covered by `guest_listener_real_boot.rs` | `distill/upstream-issues.md` |
| Minor | `BeaconParseError::MalformedListen { raw, detail }`; `Idle` is a read-only `reported(alloc)` call; the original D16 historical-fixture/version-bump obligation is superseded by the 2026-11-09 greenfield waiver and accepted D16-G1 current-format policy; the per-consumer board is fixed by DG-6 | § Interface changes (D13 to D16) |

The gaps as DISTILL routed them:

| ID | Gap | Options |
|---|---|---|
| DG-1 | `ra_conf` uses `CEIL = 2`; production `RESTART_BACKOFF_CEILING` is a const (5) with no seam, so 7 give-up runs and the randomized run cannot activate | instantiate `ra_conf` at `CEIL = 5` (spec only), or pin a ceiling seam |
| DG-2 | The model's `pending` row and `StartRejected` have no production step (the shim writes `Running` with the awaited start); `non_crash_failed_row_gives_up_backoff_exhausted` cannot activate | remove the pending split from the model, or pin a start-rejection step |
| DG-3 | `liveness_gate` `committed` is monotone across `Crash` in the model but the pinned accessor reads the current row; the driver compares it only while `Running` | pin the restriction in the projection table, or change the model |
| DG-4 | `stream_snapshot` projects the yield of a free function; `conformance-lint` requires an owner accessor | rule an exemption shape, or pin an accessor (for example an `alloc_status_row` cross-check) |
| DG-5 | Spec fidelity: `stream_snapshot` `Deliver` is not FIFO and is enabled while `lagged`; `progress_deadline` `StableHappens` is enabled after `OperatorStop` | modeler guard fix (`Deliver` oldest-first and `not(lagged)`; guard `StableHappens`) |
| DG-6 | `ReportedListeners` construction (`Default`, `FromIterator`, `is_empty`) and `GuestListenerBoard::new` are unpinned but every composition needs them | pin them, or accept them as the crafter's internal structure |
| DG-7 | Position of the `progress_deadline` parameter in `build_service_stream`, of the server deadline in `submit_workload_streaming`, and the module path of `LastAttemptWire` (placed in `streaming.rs`) | pin the three |
| DG-8 | G-10 items (9) init's own binds produce zero LISTEN lines, (10) loopback and v4-mapped binds are not authored (item (8), the ring-drop composed run, is authored: `guest_listener_gate_composed.rs`): they need a Tier-3 real-boot fixture and a board read in the real composition, neither pinned | pin the fixture and read, or record them as spike-evidence obligations only |
| DG-9 | Waivers: S-32 restart and give-up leg (needs `VmDriver::start` over `SimVmm` plus beacon; the pure trajectory test covers it); S-48 (b) (no await point on a single-thread sim) and (e) (needs a reconciler that blocks in flight, a test seam) | accept the waivers, or pin the missing composition and seam |
| DG-10 | Proposal: `tokio` `test-util` dev-dependency in `overdrive-cli` for the S-36 paused-clock request-timeout probe (the `Cargo.toml` edit is in the tree) | user approves or rejects |
| DG-11 | D14 item 4 pins `guest listeners tcp:8080 udp:5353`, but `format_mechanic_summary` sees only the fieldless mechanic; where the listener set reaches the formatter is unpinned | pin the carrier |
| DG-12 | Whether a started `Failed` allocation stays in `terminal_announced` after D6 is unpinned; the sim invariant assertions on it were removed | pin the membership rule |

Historical minor items from DISTILL: the `BeaconParseError` variant for a rejected LISTEN line, the `Idle` action mapping of `guest_report_host`, and the D16 current-format policy are now pinned above. The formerly unpinned parameter type of the **public** `service_stream_synth_cap_timeout` is pinned by D10-H1 in § D10 interface changes. The retry cadence of a failed `Pass` write and whether `<n>` stays at the deadline value in later Fail reasons, and test-support fixtures remain crafter/DISTILL choices within the accepted observable contract. The original D16 historical fixture/version-bump obligation was superseded by the explicit 2026-11-09 greenfield waiver. The per-consumer `GuestListenerBoard` in production wiring is inert until DELIVER shares one board; pre-existing tests that assume liveness starts at `Running` or that `broker().submit` is drained only by the clock are DELIVER fallout; `RED` bodies still using `inferred` on `ProbeDescriptor` and `ProbeWitness` are removed in the D16 step.

## Wave: DISTILL / [REF] Carried open items

Not tested by DISTILL, by DESIGN ruling: **V39**; **P-12** (benchmark, proposal only); **F-1 (GH #335), F-2 (GH #336), F-6 (GH #340)**; the **IPv6/V6ONLY** exposure (GH #308).

## Wave: DELIVER / [REF] Finalization preparation (2026-11-13)

**Status: verification pending; not finalized.** The approved
[`deliver/roadmap.json`](deliver/roadmap.json) has 17 implementation steps. Each
step has a completed DES trace and a final `APPROVED` verdict in a
[`deliver/review-*.md`](deliver/) artifact. Later bounded corrections
and their independent reviews are recorded in
[`deliver/finalization.md`](deliver/finalization.md); they do not turn the
diagnostic full-workspace suite into a passing gate.

The reviewed default-feature native `overdrive serve` and `overdrive deploy`
capture reached `Stable` in 1,363 ms when the guest reported `tcp:18081`.
The never-listening case recorded six allocation rows, missing `tcp:18998`,
and a terminal `StartupProbeFailed` on identical redeploy. Both captures
recorded zero deltas on their **respective measured** cleanup surfaces; the
negative capture did not measure a probe-ownership delta. These are the
point-in-time receipts in [`deliver/step-05-07-verification.md`](deliver/step-05-07-verification.md),
not a claim about every later source commit. The historical SHA-pinned E13
inferred-TCP receipt remains unchanged and does not prove the new default.

The user's exact instruction, **“no mutation testing is required,”** is
recorded in the roadmap final wave gate as `required: false` and
`status: not_required_by_user`. No final-gate mutant run or kill rate is claimed. The
D17 production wake invariant and all other approved outcomes remain in force.

Finalization waits for the D11 randomized `stream_snapshot` poll/subscribe
mapping alignment at seed `0x43d4164d`, its named and randomized passing
results and independent review, the latest-source scoped integration rechecks,
and an independent final evidence audit. The broad native workspace diagnostic
reported 4,512 passed, 129 failed, two timed out and 27 skipped; it is not a
workspace-green result. The website docs and blog are checked in and
independently approved; the pre-existing search-ranking assertion still fails
on untouched HEAD, and no website deployment or publication occurred.
