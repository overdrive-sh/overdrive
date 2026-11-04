# DESIGN wave decisions — `service-allocation-readiness-and-ending` (GH #318, GH #334)

> Authored by Morgan (nw-solution-architect), 2026-11-02, revised 2026-11-03. PROPOSE mode.
> Scope: all priorities. There is no DISCUSS wave. The exact contract is the feature delta
> ([`../feature-delta.md`](../feature-delta.md)); this file is the index. Nothing authorises
> DISTILL until the independent DESIGN re-review of this revision approves it.

## Status

- **Accepted by the user:** D1 to D12, BC-1, B1 to B3, D17. B4 is D12; B5 is rejected. The
  rulings are in the feature delta's § Approval record.
- **User-approved, not yet Accepted:** D13 to D16. V34 to V38, V41 and V42 are held (the cost figures are in the feature
  delta's § Spike table, V41 and V42 rows), V39 is partial. User rulings of 2026-11-04
  ("proceed with recommendations") on the four items inside them: **PR-1** accepted (the
  read-only `GuestListenerReports` port, `GuestListenerBoard`, the extra mandatory parameters
  on `ProbeRunner::new` and `VmDriver::new`, `SimGuestListenerReports`; `Driver`, `Action` and
  the reconcilers unchanged; D13 item 10). **PR-2** accepted in contract shape (direction approved
  2026-11-04 pending V42; V42 held 2026-11-04, kernel `7.0.0-38-generic`,
  [findings](../spike/findings-workload-cgroup.md)): init creates a workload
  cgroup and attaches the three listen programs there; the TCP arm is a separate small
  sockops program in the same object (D13 items 1 and 7). V42 pins four constraints (D13
  item 1, P-a to P-d): the programs live in the object init already loads, never a second
  `Ebpf::load`; the root programs stay `BPF_F_ALLOW_MULTI` link attachments
  (`kernel/bpf/cgroup.c:1506-1507`, `:459`); the workload is created directly in the cgroup
  with no placement window; reports cover only sockets of tasks in the cgroup (a workload
  that leaves it fails closed). **PR-3** accepted: `Pass` on the
  event; first `Fail` at the derived deadline, then one per
  interval until the stop lands (**P1, accepted 2026-11-04, option (a):** rows carry honest
  `same_status_runs` 1, 2, 3, …, and the stop gate's attempts threshold comes from the
  Service's startup descriptor: 1 for `GuestListeners`, `max_attempts` for `Tcp`/`Http`;
  `ServiceAllocFact.max_attempts` becomes `startup_attempts_required`; explicit probes
  unchanged); no pre-deadline `Fail`; `describe` loses pre-deadline
  progress (D14 item 3). **PR-4** accepted: `declared_listeners` replaces `service_ports`,
  with `tcp_service_ports()` for the TPROXY consumers (D14 item 2). `guest_report` model:
  host-half conformance plus Tier-3 boot coverage for the guest half; no writer seam in
  `overdrive-init`; the gap is recorded in the feature delta (§ Formal protocol model).
  **A1** accepted 2026-11-04: the board entry is removed at `close`, so `reported` on an id
  with no entry is empty; "frozen at the end of the wait" is the host reader's rule; ids are
  never reused (ADR-0105). **A3** accepted 2026-11-04: a host-input projection module of
  `guest_report` (`guest_report_host`) is the conformance target. **R1** accepted 2026-11-04:
  the projection gains a `VmmExit` action (`SimVmm::exit_process` plus the driver's exit-gate
  release, as the `quint_flows_beacon` driver's `vm_stop` does) and `exitrep` becomes
  three-valued (`none`, `guest-reported`, `no-report`), projected from the exit observer's
  `AllocStatusRow` through `ObservationStore::alloc_status_row` (**R1-a**; the observer, not the
  harness, takes the exit receiver). **R2** accepted 2026-11-04: three full-module `guest_report`
  runs are dropped (their host behaviour lives in the `guest_report_host` runs), `gl` is pinned
  `false` in the exhaustive `ra_ok` instance and its hazard instances, and
  `guest_listener_fail_fail_reemits_stop_per_row` is folded into
  `first_fail_at_deadline_with_no_earlier_row_stops`; `ListenBeforeExit` is kept. ADR-0058 stays
  operative until ADR-0270 is accepted. No ADR text changes: R1 and R2 change the conformance
  projection and the model's runs, not an ADR decision.

## Decisions

| ID | Decision | Status | Record |
|---|---|---|---|
| D1 | The exit observer fires `Driver::on_alloc_terminal` once per consumed `ExitEvent`, before and independent of the row write | Accepted | ADR-0255 |
| D2 | `on_alloc_terminal` is row-independent, plural-author, infallible, idempotent, a no-op for unknown ids | Accepted | ADR-0255 |
| D3 | The shim calls `on_alloc_running` before `release_for_exit_emission` at both Running sites | Accepted | ADR-0256 |
| D4 | `StopAllocation` and `FinalizeFailed` keep their hooks; `cleanup_restart_predecessor` gets none | Accepted | ADR-0255 |
| BC-1 | A crashed Service allocation's startup streak no longer grows by probing a dead guest | Accepted (behaviour change) | ADR-0255 |
| D5 | `ServiceLifecycle` announces a startup failure only for a non-terminal allocation | Accepted | ADR-0257 |
| D6 | `WorkloadLifecycle` alone authors a crash's ending and names a pre-startup crash loop `EarlyExit` at give-up; one hydrated field `current_startup_probe` | Accepted | ADR-0258 |
| D7 | A startup-probe failure on a live Service allocation is a stop (`StoppedBy::StartupProbe`); `WorkloadLifecycle` names `StartupProbeFailed` at give-up | Accepted | ADR-0259 |
| D8 | A startup stop that does not land is retried once per new startup probe row (`startup_last_acted_run`) | Accepted | ADR-0260 |
| D9 | Liveness probing starts at `Stable` (`ProbeRunner::start_liveness`) | Accepted | ADR-0261 |
| D10 | The Service deploy stream is bounded by `max(streaming_cap, startup_deadline + 30 s)`; the timeout carries `last_attempt` | Accepted | ADR-0262, ADR-0264 |
| D11 | The Service deploy stream first reports the verdict the current attempt already carries | Accepted | ADR-0263 |
| B1 | `Stable` is announced from the committed row (`ServiceAllocFact.stable_committed`); the `stable_announced` field is removed (R2-2) | Accepted | ADR-0265 |
| B2 | The shim fires `on_alloc_terminal` after the stop's terminal row write succeeds | Accepted | ADR-0266 |
| B3 | The pre-startup-crash predicate accepts `WorkloadCrashedImmediately` and `VmOutOfMemory` | Accepted | ADR-0258 |
| D12 | A startup-probed allocation is a backend only after its own `Stable` is committed; the per-allocation veto and the veto-before-stop ordering are gone (R2-1); amends ADR-0096 | Accepted | ADR-0267 |
| D13 | Kernel hooks observe each workload listener; a `LISTEN` beacon line reports it; contract shape accepted 2026-11-03 | User-approved | ADR-0272, ADR-0268 |
| D14 | `ProbeMechanic::GuestListeners` and `AllocationSpec.declared_listeners` feed the startup probe row | User-approved | ADR-0269 |
| D15 | A probe-less Service is `Stable` when `Running` and every declared listener is reported; opt-out kept; no minimum-healthy window; supersedes ADR-0058 | User-approved | ADR-0270 |
| D16 | `inferred` is removed from the descriptor, probe row, witness, wire and CLI (option R) | User-approved | ADR-0271 |
| D17 | A broker submit wakes the convergence loop at once; the cadence tick stays as the backstop (no early wake exists today). Shape choices S1 to S6 accepted as proposed 2026-11-03 | Accepted 2026-11-03 | ADR-0273 |
| V41 | Boot cost of the listen programs on the pre-`READY` path: +0.36 ms at p50 (paired p90 +1.29, max +2.10) against a same-toolchain baseline | Held | feature delta § Spike table |
| V42 | Hook scope on the real init (PR-2): init's binds invisible, workload's reports intact, cost in the V41 range | **Held (2026-11-04, `7.0.0-38-generic`, PROMOTE)**: 1000/1000 exact, 0 from init; cost in the V41 range; `spike/findings-workload-cgroup.md` | feature delta § Spike table |
| R-1 to R-5 | Clarifications of accepted decisions and current code (feature delta § Pinned readings) | Clarification | n/a |

Not in scope by user ruling 2026-11-02 (no deferral, no issue): P-3, P-4, P-5. Findings: F-1 is
GH #335, F-2 is GH #336, F-6 is GH #340; F-4 is included (a DISTILL fixture, S-25); F-5 is fixed by
D11. P-1, P-2 and P-9 remain proposals.

## Architecture summary

- Style: unchanged ports-and-adapters with dependency inversion. No new crate. Public surface added: `StoppedBy::StartupProbe` (D7), `ProbeRunner::start_liveness` (D9), the D10
  items, `ReconcilerRuntime::submit_evaluation` and `submitted` (D17; `broker_mutex()` removed,
  `broker()` narrowed). D13 to D16: `BeaconMessage::Listen`, `ProbeMechanic::GuestListeners`,
  `AllocationSpec.declared_listeners`, and (PR-1) the `GuestListenerReports` port with the
  `GuestListenerBoard` adapter and `SimGuestListenerReports`. In-memory State fields: `current_startup_probe`
  (D6, D7), `stable_committed` (B1). CBOR `View`: `startup_last_acted_run` added (D8),
  `stable_announced` removed (R2-2).
- C4: a component diagram is in the feature delta; `c4-diagrams.md` is unchanged.
- Technology: none added; no external integrations, so no contract-test annotation for the
  platform-architect handoff.
- Enforcement: existing `xtask dst-lint` (the `release-supervision-placement` clause is unaffected;
  the reconciler purity scan covers new `reconcile` code), the crate-class gate, `openapi-check`
  (D7 and D10 regenerate the spec), the schema-evolution golden-fixture test (S-25), the CBOR
  default-decode test (S-38) and clippy. No new tooling is proposed.

## Reuse analysis

Everything is REUSE, EXTEND or DELETE except the CREATE NEW items justified in the feature delta §
Reuse Analysis: `StoppedBy::StartupProbe`, `startup_last_acted_run`, `ProbeRunner::start_liveness`,
the D10 deadline items, `ServiceAllocFact.stable_committed`, and D17's `submit_evaluation` and
`submitted` (no existing path wakes the loop).

## Technology and constraints

Overdrive appliance threat model: only Overdrive's own endings are in scope; no foreign-mutation or
root-compromise defences were added. D1 to D12 and D17 make no kernel claim. D13 to D16 cite the
Linux 7.0 source as `path:line @ v7.0`; the runtime facts are settled by V34 to V39 and V41, and V42 settles PR-2.
Quality attributes: reliability and maintainability first; latency for D13 to D17.

## Upstream changes

None to DISCUSS (none exists). The RCA's proposed fix (a hook call in `cleanup_restart_predecessor`)
is superseded by D1 and D3. K1 is restated by D6, D7, D10 and D11 (feature delta § Changed and
unchanged assumptions). ADR-0055's early-exit clause, the slice-08 `elapsed:` line and the
`EarlyExit` variant doc change (D6); ADR-0054 § 2's "spawns one task per descriptor at
`on_alloc_running`" is restated for the liveness role (D9); ADR-0096 D1 step 1 and D2 are
superseded (ADR-0267).

## Handoff to DISTILL (after the independent re-review of this revision)

- Formal protocol model: `specs/quint/service-allocation-readiness-and-ending/` (D1, D3, D5 to D11, D13, D14).
  The named runs, hazards and invariants are that directory's `README.md`. D12, D17, R2-1
  and R2-2 add no model state (D17 is justified without a model: § Formal protocol model).
- Scenarios S-1 to S-48 are in the feature delta with oracles and tiers. D6 and D7 rewrite the
  early-exit and startup-failure tests listed in their sections. D13 to D16 obligations are
  G-10's boundary scenarios.
- No open contract-shaping validation items for D1 to D12 and D17. V34 to V38 and V41 are held and
  V39 is partial (not contract-shaping); V42, the last contract-shaping item (PR-2), is held.
  The one gate before DISTILL is the independent DESIGN review (approval).

## Surfaced to the user

1. **D17's shape (ACCEPTED as proposed, S1 to S6, 2026-11-03).** The wake is a
   `tokio::sync::Notify` signalled with `notify_one` after the broker lock is released; the loop
   gains a fourth `select!` arm; `ReconcilerRuntime` gains `submit_evaluation` and `submitted`;
   `broker_mutex()` is removed and `broker()` loses `submit`, so a submit that forgets to wake is not
   expressible. No Quint model (the hazard costs latency, is excluded by the primitive's documented
   semantic, and cannot be conformance-tested); the outline is in the feature delta should you want it.
2. **Removals.** `stable_announced` has no reader after B1 and is removed; the ADR-0096 veto and the
   veto-before-stop ordering are removed. Three veto and ordering tests collapse into one (S-46).
3. **Exposures stated, not mitigated:** a crash racing the stop overwrites the crash row (E-2); the
   progress deadline can fire before the give-up (E-3) and before a boot at its deadline (E-4).
4. **PR-1 to PR-4 (ruled 2026-11-04; PR-2 direction pending V42, V42 held):** see Status above and the feature
   delta's D13 items 1, 7 and 10 and D14 items 2 and 3. PR-3 (accepted) trades the pre-deadline `describe` progress
   ("missing tcp:8080") for about 29 fewer observation writes per never-ready attempt. P1
   (accepted 2026-11-04): the first `Fail`'s counter is honest (1) because the gate's threshold
   for this mechanic comes from the descriptor. A1 (accepted 2026-11-04): the board's entry
   lives from `open` to `close` (no frozen read after close). A3, R1 and R2 (accepted
   2026-11-04): the conformance target is the host-input projection of `guest_report`, with a
   `VmmExit` action and a three-valued `exitrep`, and the model trims in the Status section.
5. **A rules-doc touch at landing:** `.claude/rules/development.md` § "Reconciler I/O" describes the
   steady-state tick as the drain period; D17 adds the wake. Not edited by the architect.
