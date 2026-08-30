# Architecture Delta Assessment Since Step 02-04

## Assessment metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Requested scope | Every committed and current tracked worktree change since step 02-04 began; current untracked files inventoried separately |
| Baseline | `9be3680b82ed216fa44cb1a231ad62fb6a79a41b` — parent of initial 02-04 commit `45cab7ea` |
| Assessed committed head | `f7106c1637367bf7a3a348c8a111b8f15d806657` |
| Current step state | 02-04 APPROVED; 02-05 APPROVED; 02-06 **not approved** (`review-02-06.md`, Iteration 8: `NEEDS_REVISION`) |
| Method | Read-only diff, history, design, roadmap, review-artifact, and source audit; no tests or mutation testing run |

## Executive verdict

**The change set has crossed from DELIVER implementation into unreviewed architecture design. Stop 02-06 remediation at the current state. Do not treat the 02-05 implementation review approval, the passing native traces, or the user-directed placement of `OwnedTaskSet` in `overdrive-core` as approval of the broader architecture.**

The approved DESIGN is unusually explicit: the only new production behavior is the guest tap/network handoff, two existing mTLS install-gate extensions, pre-READY diagnostic selection, exact Job terminal classification, and D7 nft accounting. Its architecture summary says this feature adds no new port, external API, persistence subsystem, or observation-schema change (`design/wave-decisions.md:20-39`). That is a statement of the chosen feature architecture, **not** a general "frozen shapes" rule. Existing Rust interfaces and internal persistence schemas may evolve when the accepted design requires it, using the repository's normal compatibility and schema-evolution mechanisms. The defect assessed here is not that a shape changed; it is that DELIVER invented new systems of record, ownership authorities, recovery protocols, and consistency boundaries that DESIGN never selected. Roadmap file lists are guidance rather than restrictive allowlists. Repository orchestration rules require a separate DESIGN remediation only when an acceptance criterion genuinely cannot be met without choosing such a new architecture (`AGENTS.md:148-157`).

The current tree nevertheless contains all of those mechanisms:

- an fsync-backed `terminal-effects/` filesystem subsystem with route records, event-context records, and a four-state lifecycle outbox;
- a new `LifecycleEventPort` and durable lifecycle-event data protocol;
- a cross-layer failed-start cleanup ownership protocol spanning `VmDriver`, `DriverError`, `AllocStatusRow`, `WorkloadLifecycle`, action dispatch, and VM reclamation;
- a generic task-ownership/completion-fence API, a fallible server-shutdown API retaining retry capability, and new worker/resolver ownership semantics;
- a boot survivor-recovery protocol that joins live PIDs, Running rows, adopted slots, and IntentStore data, installs special kernel quarantine rules, sweeps, reconstructs SVID/intercept state, and releases quarantine after readiness;
- a changed security ordering that installs the mTLS intercept before `Driver::start`, contradicting the approved `VMM-spawn ... READY ... intercept-live ... EXEC-release` sequence.

These are not small implementation details. They select new systems of record, transaction boundaries, crash semantics, ownership authorities, and public contracts. Several remain incomplete on their own terms: the current 02-06 review is still `NEEDS_REVISION`, and the dirty outbox rewrite cannot make an ephemeral broadcast exactly-once because it acknowledges after send while no consumer deduplicates `effect_key`.

The last clean architecture checkpoint is the approved 02-04 boundary at `408f5feb`. This is a **candidate recovery anchor**, not an instruction to reset the worktree. The 02-05/02-06 changes are heavily entangled, so recovery should preserve the current branch and surgically retain approved behavior. A separate DESIGN remediation is required only for a specific unresolved architectural choice that remains after the invented mechanisms are removed; it is not a prerequisite for restoring the already-approved design.

## Quantitative scope

### Cumulative baseline-to-current tracked delta

| Scope | Files | Insertions | Deletions |
|---|---:|---:|---:|
| Committed baseline → `HEAD` | 81 | 24,574 | 1,486 |
| Uncommitted `HEAD` → worktree | 14 | 1,763 | 253 |
| Baseline → current tracked state | 88 | 26,099 | 1,501 |
| Production/manifests within current delta | 28 | 6,909 | 1,058 |
| Tests, verification, evidence, and examples | 53 | 13,597 | 442 |
| Feature documentation | 6 | 5,592 | 1 |
| Other configuration | 1 | 1 | 0 |

The production delta spans seven crates: `overdrive-cli`, `overdrive-control-plane`, `overdrive-core`, `overdrive-init`, `overdrive-netlink`, `overdrive-reconcilers`, and `overdrive-worker`, plus `Cargo.lock` and one crate manifest.

### Phase growth

The figures below are sequential patch sizes, not additive ownership counts, because later phases repeatedly rewrote earlier files.

| Phase | Commits / state | All-file patch | Production patch |
|---|---|---:|---:|
| 02-04 | `45cab7ea`, `4dae2b98`, `408f5feb` | 25 files, +8,688/-319 | 8 files, +1,128/-127 |
| 02-05 | `e882ec57` through `6691bb67` | 21 files, +6,650/-344 | 10 files, +1,476/-294 |
| 02-06 committed | `9e5c629d` through `f7106c16` | 48 files, +9,279/-866 | 16 files, +3,994/-700 |
| Current dirty remediation | uncommitted | 14 files, +1,763/-253 | 4 files, +441/-67 |

There are 18 commits after the baseline. 02-06 alone accumulated one initial commit and seven implementation-remediation commits without reaching approval. The current dirty state continues that remediation after Iteration 8.

### Current dirty tracked files

The uncommitted production changes are concentrated in:

- `crates/overdrive-control-plane/src/action_shim/mod.rs` (+401/-67): replaces a durable marker with a versioned pending/applied/complete/notified outbox and adds retained pre-start rollback ownership;
- `crates/overdrive-control-plane/src/lib.rs` (+16): replays the outbox at the production boot root;
- `crates/overdrive-control-plane/src/streaming.rs` (+22): makes Job and Service stream creation consume/replay the outbox;
- `crates/overdrive-control-plane/src/worker/exit_observer.rs` (+2): initializes the new event idempotency key.

Ten dirty test/config/documentation files are coupled to those production changes. The dirty review artifact still records 02-06 as `NEEDS_REVISION`.

### Untracked inventory

Seventeen pre-assessment untracked files exist. Git cannot establish whether they were created before or after the baseline, so they are **not attributed** to the 02-04+ implementation delta. They are `AGENTS.md`, six earlier DELIVER review artifacts, one roadmap-remediation review, one DESIGN review, and eight DISTILL review artifacts. This assessment file is an additional untracked deliverable created by the requested audit.

## Classification model

| Class | Meaning | Action |
|---|---|---|
| A | Directly authorized by approved DESIGN/roadmap | Retain, subject to normal review |
| B | Tightly bounded implementation fallout or enabling glue for an approved decision | Usually retain; re-review at the resulting boundary |
| C | Plausibly necessary correctness work that chooses a new architecture | Exclude from reconstruction; if an AC truly cannot be met without it, surface that specific gap to separate DESIGN remediation/review |
| D | Contradicts an explicit approved constraint or ordering | Remove or redesign unless DESIGN is formally amended |

Classification is by mechanism, not whole file. Several files contain A/B code interleaved with later C/D code.

## Delta classification

| Mechanism | Primary files | Class | Assessment |
|---|---|---|---|
| D7 anonymous counter, strict `GETGEN`/multipart `GETRULE`, normalized full-program identity, loss-detecting observer | `overdrive-netlink/src/nft.rs`, `overdrive-worker/src/mtls_intercept.rs` | A | Explicit D7 implementation. Retain the counter/observer/decoder portions. |
| D7 capture and sole E07 built-product journey | CLI native integration, checked-in example, expectation runner | A | Explicit 02-04 scope; 02-04 review is APPROVED. |
| Same-tag rule adoption and exact-handle shared guard | `overdrive-worker/src/mtls_intercept.rs` | A/B | Implements the approved D7 adoption/teardown identity requirement. Separate it from later quarantine behavior in the same file. |
| IPv6 suppression before link-up, existing exit-code field projection, production-composition VMM decorator, identity issuance helper | `veth_provisioner.rs`, `handlers.rs`, `serve.rs`, `action_shim/mod.rs` | B | Local glue/remediation for the approved all-EtherType and E07 contract; no new system of record. |
| Pre-READY diagnostic selection, guest Beacon boundary evidence, `VmGuestExitUnreported` mapping | `vm_driver.rs`, `overdrive-init/src/main.rs`, `transition_reason.rs`, exit observer/tests | A/B | Direct 02-05 behavior or bounded evidence support. |
| Typed duplicate-start cause | `overdrive-core/src/traits/driver.rs`, `vm_driver.rs` | B | A bounded cause addition to an already non-exhaustive enum; useful independently of the later ownership protocol. |
| Retained failed-start cleanup ownership and retry protocol | `traits/driver.rs`, `vm_driver.rs`, `action_shim/mod.rs`, `workload_lifecycle.rs`, `vm_reclamation.rs`, `reclamation.rs` | C | New cross-layer ownership, recovery, and consistency protocol. It requires DESIGN even though 02-05 implementation review eventually approved it. |
| Overloading `Pending + DriverInternalError` as the durable cleanup token | action shim and workload lifecycle | C | Creates a hidden persisted state machine using pre-existing observation fields. No approved document defines its ownership, compatibility, or recovery semantics. |
| Execution-time reclamation lease (`Driver::try_begin_reclamation`) | core Driver trait, VM driver, reclamation executor | C | New cross-component arbitration/ownership model; not described in D6. |
| Pure same-attempt terminal fence and same-id Platform-Reclamation re-drive | `workload_lifecycle.rs`, `vm_reclamation_boot.rs` | A | Directly maps the approved D6 route and illegal-event/reclamation-once contracts. Retain the pure behavior while separating it from C mechanisms. |
| Allocation/worker task trees and cancellation-safe completion fence | new `overdrive-core/src/task_ownership.rs`, mTLS worker, resolver | C | The primitive may be sound and the user explicitly directed its **placement** in `overdrive-core`; placement does not approve the new lifecycle model, shutdown protocol, consumers, or public API. |
| Fallible server shutdown with retained retry owner | control-plane `ServerHandle`, CLI `ServeHandle`/`CliError` | C/D | New ownership-transfer and retry contract not required by the approved feature. Revert it unless that ownership model is separately designed. The violation is the invented lifecycle contract, not the mere fact that a Rust API changed. |
| Live mTLS survivor planning/reinstallation | action shim, control-plane boot, workload projection | C | New recovery join and authority model across host observation, ObservationStore, IntentStore, slot adoption, identity, and worker state. |
| Kernel recovery quarantine batch | netlink nft, worker intercept, control-plane boot | C | New kernel rule kind, ownership protocol, boot transaction, failure retention, and release gate. Plausible fail-closed remediation, but absent from DESIGN. |
| Persistent route/event context journal under `terminal-effects/` | action shim, `AppState` | D | Adds a parallel persistence subsystem that DESIGN did not select and duplicates the established observation boundary. It has no approved schema, migration, garbage collection, corruption, or quota policy. Multi-process ownership is irrelevant: the current product is single-node and one control-plane process owns the data directory. |
| Durable lifecycle outbox and new `LifecycleEventPort` | action shim, `AppState`, streaming | D | Explicitly adds both a port and persistence, each forbidden by the approved architecture. Current dirty work expands it into a versioned storage protocol. |
| `LifecycleEvent.effect_key` and idempotent terminal driver hook | action shim, core Driver trait, Exec driver/probe runner | C/D | New event contract and effect-consumer protocol; no downstream stream consumer currently deduplicates the key. |
| Move mTLS install ahead of `Driver::start`/VMM spawn | action shim Start/Restart arms | D | Contradicts the approved ordering `capture-ready ≺ VMM-spawn ≺ network-ready ≺ READY ≺ intercept-live ≺ EXEC-release` (`wave-decisions.md:63`). |
| Tests and evidence for C/D mechanisms | many control-plane/worker/CLI tests | B only relative to their mechanism | They are valuable evidence but cannot approve or legitimize an unreviewed architecture. They must be reshaped after the architecture decision. |

## Architectural impact

### 1. New persistence and system-of-record boundary — Critical

`AppState::new` derives `<intent-redb-parent>/terminal-effects` and constructs two independent durable components (`control-plane/src/lib.rs:683-710`):

```text
terminal-effects/
├── <alloc-hash>.route
├── <terminal-event-hash>.event
└── lifecycle-consumer/
    ├── <event-hash>.pending
    ├── <event-hash>.applied
    ├── <event-hash>.complete
    └── <event-hash>.notified
```

This is a persistence subsystem in everything but name. It stores a versioned lifecycle record, driver routing, transition source/prior state, effect completion, and notification acknowledgement. The approved architecture instead says no persistence change, and the roadmap's reuse tally expressly says not to add persistence/rkyv surface.

The subsystem has no approved answers for:

- which store is authoritative when an ObservationStore terminal row and a filesystem record disagree;
- atomicity between ObservationStore writes and filesystem fsync/rename/hard-link operations;
- schema/version migration and downgrade behavior;
- the single-process owner's behavior when the directory is corrupt, partially restored, full, or otherwise unavailable;
- cleanup/compaction (the production code never deletes final `.route`, `.event`, `.pending`, `.applied`, `.complete`, or `.notified` files);
- corruption, partial directory restoration, quota/disk-full behavior, and operator repair;
- whether notification delivery is at-most-once, at-least-once, or exactly-once.

The current dirty outbox is still not exactly-once. It broadcasts and only then creates `.notified`; a process cut between those operations re-broadcasts. `LifecycleEvent.effect_key` is not consumed by either workload or Service streaming projection, so the duplicate cannot be collapsed there. Conversely, an existing `.notified` suppresses delivery even if the intended subscriber did not consume the earlier broadcast. This is the same class of defect tracked through review findings D23/D30/D34/D38, not a closed implementation detail.

### 2. Hidden failed-start cleanup state machine — High

The cleanup flow now crosses six owners:

```text
VmDriver.pending_cleanup
  → DriverStartCleanupError hidden inside DriverError::Io
  → action shim writes Pending + DriverInternalError
  → WorkloadLifecycle treats that row as retained cleanup
  → RestartAllocation/StopAllocation re-drives the old allocation
  → VM reclamation consults a Driver-level execution lease
```

Evidence includes new public cleanup types (`overdrive-core/src/traits/driver.rs:216-292`), `VmDriver.pending_cleanup` (`vm_driver.rs:954`), the observation discriminator (`workload_lifecycle.rs:1452`), and `Driver::try_begin_reclamation` (`traits/driver.rs:1090-1117`).

This is not merely “total cleanup.” It defines:

- the authoritative owner of residual VMM/cgroup/rootfs/run-directory state;
- how that owner is represented durably without a schema field;
- retry scheduling/backoff and fairness;
- post-crash inference from an all-`NotFound` probe;
- arbitration between action dispatch and kill-capable reclamation;
- when a cleanup disposition may be committed and supervision released.

Those choices emerged across 02-05 review iterations D3, D8, D12, D13, D14, and D15. The final implementation review accepted the behavior, but repository rules explicitly forbid using the remediation loop to invent this kind of ownership/recovery protocol. It is therefore an implementation divergence and is excluded from reconstruction by default. Only a demonstrated acceptance-criterion gap that remains after returning to the approved mechanisms belongs in DESIGN.

### 3. New task/server ownership architecture — High

`CompletionFence` and `OwnedTaskSet` are new public core primitives (`overdrive-core/src/task_ownership.rs:22,126`). They now govern mTLS allocation children, full-worker shutdown, resolver watch ownership, abrupt server-owner loss, and retryable server teardown. `ServerHandle::shutdown` changed to return `Result<(), ServerShutdownError>` (`control-plane/src/lib.rs:1454`), and the error retains the worker as a retry capability.

The user's explicit direction that the generic primitive live in `overdrive-core` resolves only dependency direction and placement. It does not approve:

- the one-shot/retry-generation semantics of `CompletionFence`;
- which tasks belong to allocation, worker, resolver, or server owners;
- whether abort, cooperative cancellation, or drain is authoritative;
- transfer of a live worker owner through a public error;
- the CLI/API compatibility change;
- the interaction between owner shutdown and persistent workload processes.

This mechanism is internally coherent enough to merit DESIGN consideration, but it is not part of approved D6/D7.

### 4. New boot survivor-recovery transaction — High

The approved D6 route is: unclean control-plane restart → boot VM reclamation → Platform Reclamation row → same-id `RestartAllocation` → existing restart install gate. Current production boot now adds an earlier transaction:

```text
live host PID
  + durable Running row
  + adopted C3 slot
  + immutable workload intent
  → reconstructed AllocationSpec
  → temporary nft DROP quarantine
  → old-rule sweep
  → DNS/frontend + resolver refresh
  → SVID issuance + userspace intercept reinstall
  → post-readiness atomic quarantine release
```

The join is implemented in `action_shim::plan_live_mtls_intercepts`/`apply_live_mtls_intercepts` (`action_shim/mod.rs:127-226`), quarantine in `mtls_intercept.rs:620-775`, and boot orchestration in `control-plane/src/lib.rs:2954-3322`.

This adds new recovery authority, a new kernel rule class, a multi-stage consistency protocol, and a fail-closed retention policy. It may be a reasonable solution to the review-discovered post-sweep gap, but it was never chosen by DESIGN and has significant interaction with the approved boot-reclamation route. It must not continue evolving through implementation review.

### 5. Approved security ordering was changed — High

DESIGN pins `VMM-spawn` before `intercept-live`, with guest networking/READY providing the safe pre-EXEC boundary. Current Start and Restart arms call `worker.start_alloc(&spec).await` before calling `driver.start(&spec)` (`action_shim/mod.rs:2842-2884` and `3377-3424`). This was introduced during 02-06 remediation to make unknown start failures rollback-safe.

The change may strengthen one packet-safety dimension, but it also creates pre-start ownership of listeners, nft rules, SVID state, and network slots. The current dirty `PrestartRollbackPending` map and retry protocol are direct consequences. Because the approved ordering is explicit, this is a DESIGN contradiction, not an implementation freedom.

### 6. Public and cross-crate contract growth — High

Representative new public/cross-crate surfaces include:

- `overdrive_core::task_ownership::{CompletionFence, OwnedTaskSet}`;
- `DriverCleanupFailure`, `DriverCleanupStage`, `DriverStartCleanupError`;
- new Driver methods for terminal idempotency, reclamation claims, supervision release/retry;
- `LifecycleEventPort`, `IdempotentLifecycleEventPort`, `TerminalEffectJournalError`;
- `LifecycleEvent.effect_key`;
- `ServerShutdownError` and fallible shutdown/retry;
- `RecoveryQuarantine`, `RecoveryQuarantineBatch`, quarantine encoder APIs;
- live-intent allocation reconstruction and same-attempt transition APIs.

Some internal D7 projection types are explicitly authorized. The ownership, persistence, shutdown, and recovery mechanisms are not. Roadmap file-scope flexibility permits necessary API, schema, compiler, and test fallout; it does not authorize a new architectural mechanism merely because that mechanism happens to require additional files or public types.

## Review-loop provenance

The architecture did not arrive in one deliberate change. It accreted in response to implementation-review findings:

| Review progression | Mechanism added |
|---|---|
| 02-05 D3/D8 | typed cleanup failures and retained cleanup carrier |
| 02-05 D12/D13 | nonterminal Pending cleanup token, reclamation exclusion, lifecycle retry |
| 02-05 D14/D15 | crash inference and cross-component retry arbitration |
| 02-06 D13-D20 | terminal cleanup ordering, idempotent terminal hook/event identity |
| 02-06 D15/D16/D22/D27/D32 | task trees, completion fences, fallible shutdown, retry-owner propagation |
| 02-06 D17/D21/D24 | live survivor join and nft quarantine transaction |
| 02-06 D23/D30/D34/D38 | persistent effect receipts, then durable lifecycle outbox |
| 02-06 D37/D40 | pre-start intercept installation and retained pre-start rollback |

This history is exactly the “iteratively prescribe a new persistence/ownership/recovery protocol inside DELIVER” pattern that `AGENTS.md:148-157` forbids. It also explains why no simple per-commit revert cleanly separates behavior from architecture.

## Rollback and redesign dependencies

### Safe preservation boundary

- Preserve the current branch/worktree exactly; do not reset or discard it.
- Tag or branch the assessed state before any recovery work.
- Treat `408f5feb` (02-04 APPROVED) as the last clean architecture checkpoint.
- Reapply only explicitly retained 02-05/02-06 behaviors after DESIGN decides the mechanisms.

### Surgical unwind order if the new architecture is rejected

1. Remove the current dirty lifecycle outbox rewrite and restore the pre-outbox live broadcast path.
2. Remove `terminal-effects/`, persistent `AllocDriverIndex` route/event records, `LifecycleEventPort`, `effect_key`, streaming replay, and boot replay as one unit.
3. Restore the existing terminal hook and terminal-row/event flow; remove the idempotent terminal driver-hook protocol.
4. Restore the approved post-spawn/pre-EXEC install ordering. Remove `PrestartRollbackPending` and its retry path unless a newly approved design retains pre-start ownership.
5. Remove the live-survivor reinstall/quarantine transaction or replace it with an approved boot-reclamation ordering. Quarantine code is mixed into D7 files and must be peeled away without removing the D7 observer/counter.
6. Remove the generic task/server ownership and fallible-shutdown expansion unless approved independently. Preserve only test-fixture mechanics that do not alter production ownership.
7. Collapse the failed-start cleanup protocol back to the approved observable behavior. If exact retry across process failure is still required, stop and select its durable authority/schema in DESIGN rather than continuing to overload `Pending + DriverInternalError`.
8. Retain the pure D6 same-id transition/fence behavior, the direct Q7 diagnostics, and all 02-04 D7/E07 work. Rework tests to prove only the retained architecture.

### Recovery constraints and conditional DESIGN gates

These are recovery defaults, not seven mandatory new design exercises:

1. **Terminal event semantics:** The existing `ObservationStore` terminal row remains the durable source of observed lifecycle truth. Remove the filesystem outbox, `LifecycleEventPort`, and replay protocol. A different durable-delivery model would require an explicit future DESIGN decision.
2. **Cleanup authority:** Reconstruct the approved observable failure and cleanup behavior without the hidden `Pending + DriverInternalError` state machine. If a concrete acceptance criterion remains impossible without a durable residual-owner model, surface only that exact unresolved choice to DESIGN.
3. **Boot safety:** Restore the approved D6 boot-reclamation route and ordering. Do not carry forward live-survivor reconstruction or quarantine transactions unless a separate design explicitly selects them.
4. **Task ownership:** Keep only ownership directly necessary for resources introduced by the approved feature. Generic allocation/worker/resolver/server retry ownership is not part of this recovery unless independently designed.
5. **API evolution:** Existing interfaces may evolve when required—for example, a synchronous method that performs awaited I/O must become async. Remove API surface whose sole purpose is a rejected mechanism; do not reject an interface merely because its shape changed.
6. **Persistence:** No `terminal-effects/` filesystem protocol remains in the reconstruction. The supported deployment is one control-plane process on one node; simultaneous processes sharing a data directory are not a requirement.
7. **Security ordering:** Restore the approved post-spawn, post-READY, pre-EXEC intercept gate. Pre-spawn intercept ownership and its rollback protocol are excluded unless DESIGN is deliberately amended.

Reconstruct the implementation from the preserved state against those existing decisions, then re-enter RED → GREEN → COMMIT → independent review. Invoke separate DESIGN remediation only if the reconstruction exposes a genuine unresolved architectural choice rather than another implementation-review preference.

## Production file inventory in the cumulative delta

The 28 production/manifest files are:

- `Cargo.lock`
- `crates/overdrive-cli/Cargo.toml`
- `crates/overdrive-cli/src/commands/serve.rs`
- `crates/overdrive-cli/src/http_client.rs`
- `crates/overdrive-control-plane/src/action_shim/mod.rs`
- `crates/overdrive-control-plane/src/action_shim/reclamation.rs`
- `crates/overdrive-control-plane/src/error.rs`
- `crates/overdrive-control-plane/src/handlers.rs`
- `crates/overdrive-control-plane/src/lib.rs`
- `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs`
- `crates/overdrive-control-plane/src/reconciler_runtime.rs`
- `crates/overdrive-control-plane/src/streaming.rs`
- `crates/overdrive-control-plane/src/veth_provisioner.rs`
- `crates/overdrive-control-plane/src/vm_reclamation_boot.rs`
- `crates/overdrive-control-plane/src/worker/exit_observer.rs`
- `crates/overdrive-core/src/lib.rs`
- `crates/overdrive-core/src/task_ownership.rs`
- `crates/overdrive-core/src/traits/driver.rs`
- `crates/overdrive-core/src/transition_reason.rs`
- `crates/overdrive-init/src/main.rs`
- `crates/overdrive-netlink/src/nft.rs`
- `crates/overdrive-reconcilers/src/vm_reclamation.rs`
- `crates/overdrive-reconcilers/src/workload_lifecycle.rs`
- `crates/overdrive-worker/src/driver.rs`
- `crates/overdrive-worker/src/mtls_intercept.rs`
- `crates/overdrive-worker/src/mtls_intercept_worker.rs`
- `crates/overdrive-worker/src/probe_runner/mod.rs`
- `crates/overdrive-worker/src/vm_driver.rs`

## Final gate

**Status: BLOCKING IMPLEMENTATION DIVERGENCE.**

02-06 must not be reported complete or advanced. The current implementation should remain preserved for evidence and selective salvage, but further review-driven mutation of persistence, ownership, recovery, shutdown, or ordering is not authorized. Reconstruct against the approved design; use separate DESIGN remediation only for a specific gap that the approved architecture cannot satisfy.
