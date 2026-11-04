# DESIGN wave decisions — `service-allocation-readiness-and-ending` (GH #318, GH #334)

> Authored by Morgan (nw-solution-architect), 2026-11-02, revised 2026-11-03. PROPOSE mode.
> Scope: all priorities. There is no DISCUSS wave. The exact contract is the feature delta
> ([`../feature-delta.md`](../feature-delta.md)); this file is the index. Nothing authorises
> DISTILL until the independent DESIGN re-review of this revision approves it.

## Status

- **Accepted by the user:** D1 to D12, BC-1, B1 to B3, D17. B4 was decided as D12; B5 rejected.
  Rulings of 2026-11-03: the ADR-0096 per-allocation backend veto and the veto-before-stop ordering
  are removed (R2-1); the `stable_announced` view field is removed because no reader remains after
  B1 (R2-2); D17 added.
- **Rulings of 2026-11-03 (recorded 2026-11-03):** (1) the corrected D13 is ACCEPTED in contract
  shape (sockops `TCP_LISTEN_CB` arm merged into the existing program; `post_bind4` and
  `post_bind6` `CgroupSock` programs, both return 1, filter protocol 17; loopback-only exclusion;
  one ordered beacon writer; drain `LISTEN` before `EXIT`; host reader ends only on `EXIT`/EOF).
  (2) Dual-stack: Overdrive has no IPv6 yet; the `AF_INET6` wildcard rule and the `V6ONLY`
  exposure are recorded against GH #308 (comment 6614793072); P-11 is dropped. (3) P-10 (level
  map) is REJECTED: the ring is sized generously by the crafter, the drop counter stays visible,
  a lost report is a stated bounded exposure (stopped at the startup deadline, not hung); only
  TCP listen transitions and explicit UDP binds produce events. (4) D17 shape choices S1 to S6
  ACCEPTED as proposed; D17 / ADR-0266 Accepted.
- **User-approved, pending V41 and the independent re-review (not yet Accepted):** D13, D14, D15
  and D16 (option R). Cold start is the platform goal, so the pre-`READY` cost of the listen
  programs is a contract-shaping budget item, **V41** (variant B versus variant A; findings at
  `../spike/findings-boot-cost.md`, running); a material B-minus-A delta returns D13 to DESIGN
  (`.claude/rules/design.md` § "Run contract-shaping validation before the design is accepted").
  Spikes `listen-hooks` (V34, V35, V37, V39) and `guest-signal` (V36, V38) are discharged
  (`../spike/findings-guest-signal.md`, guest kernel 7.0.0-38-generic). The findings added three
  constraints to D13: one ordered guest beacon writer, drain pending `LISTEN` before `EXIT`, and a
  host reader that ends the guest-report wait only on `EXIT` or EOF. ADR-0058 stays operative
  until ADR-0263 is accepted.

## Decisions

| ID | Decision | Status | Record |
|---|---|---|---|
| D1 | The exit observer fires `Driver::on_alloc_terminal` once per consumed `ExitEvent`, before and independent of the row write | Accepted | ADR-0248 |
| D2 | `on_alloc_terminal` is row-independent, plural-author, infallible, idempotent, a no-op for unknown ids | Accepted | ADR-0248 |
| D3 | The shim calls `on_alloc_running` before `release_for_exit_emission` at both Running sites | Accepted | ADR-0249 |
| D4 | `StopAllocation` and `FinalizeFailed` keep their hooks; `cleanup_restart_predecessor` gets none | Accepted | ADR-0248 |
| BC-1 | A crashed Service allocation's startup streak no longer grows by probing a dead guest | Accepted (behaviour change) | ADR-0248 |
| D5 | `ServiceLifecycle` announces a startup failure only for a non-terminal allocation | Accepted | ADR-0250 |
| D6 | `WorkloadLifecycle` alone authors a crash's ending and names a pre-startup crash loop `EarlyExit` at give-up; one hydrated field `current_startup_probe` | Accepted | ADR-0251 |
| D7 | A startup-probe failure on a live Service allocation is a stop (`StoppedBy::StartupProbe`); `WorkloadLifecycle` names `StartupProbeFailed` at give-up | Accepted | ADR-0252 |
| D8 | A startup stop that does not land is retried once per new startup probe row (`startup_last_acted_run`) | Accepted | ADR-0253 |
| D9 | Liveness probing starts at `Stable` (`ProbeRunner::start_liveness`) | Accepted | ADR-0254 |
| D10 | The Service deploy stream is bounded by `max(streaming_cap, startup_deadline + 30 s)`; the timeout carries `last_attempt` | Accepted | ADR-0255, ADR-0257 |
| D11 | The Service deploy stream first reports the verdict the current attempt already carries | Accepted | ADR-0256 |
| B1 | `Stable` is announced from the committed row (`ServiceAllocFact.stable_committed`); the `stable_announced` field is removed (R2-2) | Accepted | ADR-0258 |
| B2 | The shim fires `on_alloc_terminal` after the stop's terminal row write succeeds | Accepted | ADR-0259 |
| B3 | The pre-startup-crash predicate accepts `WorkloadCrashedImmediately` and `VmOutOfMemory` | Accepted | ADR-0251 |
| D12 | A startup-probed allocation is a backend only after its own `Stable` is committed; the per-allocation veto and the veto-before-stop ordering are gone (R2-1); amends ADR-0096 | Accepted | ADR-0260 |
| D13 | Kernel hooks observe each workload listener; a `LISTEN` beacon line reports it. Corrected contract shape accepted 2026-11-03 | User-approved, pending V41 and re-review | ADR-0265, ADR-0261 |
| D14 | `ProbeMechanic::GuestListeners` and `AllocationSpec.declared_listeners` feed the startup probe row | User-approved, pending V41 and re-review | ADR-0262 |
| D15 | A probe-less Service is `Stable` when `Running` and every declared listener is reported; opt-out kept; no minimum-healthy window; supersedes ADR-0058 | User-approved, pending V41 and re-review | ADR-0263 |
| D16 | `inferred` is removed from the descriptor, probe row, witness, wire and CLI (option R) | User-approved, pending V41 and re-review | ADR-0264 |
| D17 | A broker submit wakes the convergence loop at once; the cadence tick stays as the backstop (no early wake exists today). Shape choices S1 to S6 accepted as proposed 2026-11-03 | Accepted 2026-11-03 | ADR-0266 |
| V41 | Contract-shaping boot-cost budget item for D13 (pre-`READY` cost of the listen programs, B versus A); added 2026-11-03; a material delta returns D13 to DESIGN | Open (running) | feature delta § Spike table |
| R-1 to R-5 | Clarifications of accepted decisions and current code (feature delta § Pinned readings) | Clarification | n/a |

Not in scope by user ruling 2026-11-02 (no deferral, no issue): P-3, P-4, P-5. Findings: F-1 is
GH #335, F-2 is GH #336, F-6 is GH #340; F-4 is included (a DISTILL fixture, S-25); F-5 is fixed by
D11. P-1, P-2 and P-9 remain proposals.

## Architecture summary

- Style: unchanged ports-and-adapters with dependency inversion. No new component, crate or port.
  Public surface added: `StoppedBy::StartupProbe` (D7), `ProbeRunner::start_liveness` (D9), the D10
  items, `ReconcilerRuntime::submit_evaluation` and `submitted` (D17; `broker_mutex()` removed,
  `broker()` narrowed). Pending: `BeaconMessage::Listen`, `ProbeMechanic::GuestListeners`,
  `AllocationSpec.declared_listeners` (D13, D14). In-memory State fields: `current_startup_probe`
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
Linux 7.0 source as `path:line @ v7.0` and are pending the spikes that settle the runtime facts.
Quality attributes: reliability and maintainability first; latency for D13 to D17.

## Upstream changes

None to DISCUSS (none exists). The RCA's proposed fix (a hook call in `cleanup_restart_predecessor`)
is superseded by D1 and D3. K1 is restated by D6, D7, D10 and D11 (feature delta § Changed and
unchanged assumptions). ADR-0055's early-exit clause, the slice-08 `elapsed:` line and the
`EarlyExit` variant doc change (D6); ADR-0054 § 2's "spawns one task per descriptor at
`on_alloc_running`" is restated for the liveness role (D9); ADR-0096 D1 step 1 and D2 are
superseded (ADR-0260).

## Handoff to DISTILL (after the independent re-review of this revision)

- Formal protocol model: `specs/quint/service-allocation-readiness-and-ending/` (D1, D3, D5 to D11). The named
  runs, hazards and invariants are that directory's `README.md`; the work the rulings require is
  the feature delta's § Modeler change list. D12, D13 to D16, D17, R2-1 and R2-2 add no model
  state (D17 is justified without a model: § Formal protocol model).
- Scenarios S-1 to S-48 less the unused numbers are in the feature delta with oracles and tiers.
  D6 and D7 rewrite the early-exit and startup-failure tests listed in their sections. D13 to D16
  obligations are written only if they are accepted.
- No open contract-shaping validation items for D1 to D12 and D17. V34 to V38 are discharged
  (held) and V39 is partial (not contract-shaping); the one open contract-shaping item for D13 to
  D16 is **V41**, the boot-cost budget item (running). V40 is a post-DELIVER measurement and does
  not gate.

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
4. **A rules-doc touch at landing:** `.claude/rules/development.md` § "Reconciler I/O" describes the
   steady-state tick as the drain period; D17 adds the wake. Not edited by the architect.
