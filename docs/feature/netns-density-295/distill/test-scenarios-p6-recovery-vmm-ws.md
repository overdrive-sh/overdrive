# DISTILL scenario spec — P6: quiescence & recovery, shared-table, VMM device & fork, guest READY, walking skeleton + crash matrix

> **DISTILL status (2026-10-10, second increment after DG-1..DG-16).** Every
> round-1 `UI-*` item is closed by a DG decision. Round 2 leaves two open DESIGN
> items that bear on P6 and must be decided before the attachment cut: P6-S13
> depends on UI-R2-2 (the `VsockBeaconIo` constructor), and P6-S14's clean-close
> / `HalfCloseAck` half on UI-R2-5 (d) (how an owner learns a half-closed
> direction has drained — a contract-shaping decision for the user). Current status, per the feature-delta § *vsock Scenario List* and
> § *Attachment-cut DISTILL increment*:
>
> - **P6-S1..S6, S10, S12, S12b, P6-C1, P6-C2** — attachment cut (DG-2 rows 2, 3, 7,
>   8, 9, 10, 12). The bodies and the `quiescence` / `shared_table` drivers call
>   `quiesce_forwarding` / `restore_forwarding`, `HostSharedGuestNetworkOwner::new(ports)`
>   and the reshaped plan, so they cannot compile before the cut. They are
>   written by the cut's DISTILL increment, source-local in
>   `overdrive-control-plane`, starting from the removed 91f9436ff draft
>   `vsock_quiescence_recovery.rs`. They drive the runtime supervisor over the
>   owner and `SimMtlsIntercept` (`fail_next_table_transaction`) and assert O3
>   (`holders`, `admission`, `intake`, `flow_listeners_running`), O6
>   `recovery_observation()` and S `SimMtlsIntercept::shared_table_state()`.
> - **P6-S7** — withdrawn (DG-1).
> - **P6-S8** — withdrawn with the VMM backend probe (DG-1). Its `fd=` handoff
>   obligations live in P1-S12's VMM cell (attachment cut, DG-2 row 6) and in
>   P6-S13.
> - **P6-S9** — DELIVER new-step 1 provisioning; the provisioning checksum is
>   the identity check (DG-1).
> - **P6-S11** — authored: `overdrive-init` `transport::tests` and
>   `guest_flow::tests::session` (source-local, DG-5), RED, new-step 5. The
>   `main` wiring cell is at the attachment cut (DG-2 row 16).
> - **P6-S13, S14, S15** — attachment cut (DG-2 rows 1-6): `run_server` + the
>   public API on native metal. P6-S13 also carries every walking-skeleton
>   cell another scenario hands to it; they are listed by name in P6-S13.
>
> The walking skeleton and crash matrix drive `run_server` in-process and the
> public HTTPS API (the in-process equivalent of `overdrive deploy`), never the
> binary.

Piece P6 of the parallel DISTILL for the vsock attachment-replacement DESIGN
(feature-delta L16552+, ADR-0145..0181). Binding rules: **no `.feature`
files** — every scenario below is a GIVEN/WHEN/THEN spec; the crafter
translates each into a Rust `#[test]` / `#[tokio::test]`
(`.claude/rules/testing.md` §"Testing"). Shared test-support contract:
[`shared-test-infrastructure.md`](shared-test-infrastructure.md); index:
[`test-scenarios.md`](test-scenarios.md). The Quint oracle is
`specs/quint/guest-flow-owner/` (do **not** edit).

## Scope (from the index coverage matrices)

| Facet | P6 owns |
|---|---|
| Gates | G-V2 (command release, L20181), G-V4 (VMM device, L20233), G-V5 (recovery reopen, L20263), G-V7 (guest config before READY, L20337) |
| Decisions | D2, D13, D21, D8a-HOLD, D19, D19-LINKS, HALFCLOSE-SIGNAL (crash matrix), QUINT process; G-V7 guest config |
| Quint modules | `quiescence` (inv 10, L21071), `shared_table` (inv 11, L21076) |
| R5-* | R5-4, R5-5 (quiesce half — the quiesce/restore interleaving), R5-10, R5-30, R5-32, R5-45 (the Tier-3 crash cell inside R5-10); the Tier-3 halves of R5-41 (kill-and-reuse) and R5-28 / R5-31 (the V-24 shape), as P6-S13 cells |
| V-items | V-1(c) (fork `fd=` under production launch; open, DELIVER step 3), V-6 (walking skeleton; open, DELIVER 10-01), V-8 (fail-closed; open, DELIVER 10-02), V-9 (density & costs; open, replaces 08-04/10-05), V-28 (6.18 re-run; open, not a gate) |

R5-5 state-interleaving is split with P4: `ListenState` interleaving during
activation lives in P4 (`intake`); the quiesce/restore interleaving half lives
here (P6-S4). R5-45 half-close: codec + owner-seam are P2's; the Tier-3
crash-matrix cell is P6's (P6-S14). R5-10 Tier-3 is the V-13 matrix through
`serve` + `deploy`. R5-19's CID-claim admission half is P1's; the VMM-side
handoff / release / consume-once half is P1-S12's VMM cell at the attachment
cut (DG-2 row 6) plus P6-S13 (P6-S8 withdrawn, DG-1); the VMM-death half is
P6-S10.

## Authoring sequencing (DG-2)

DG-2 sorts every P6 body into one of two authoring times; this doc follows it
exactly (feature-delta § *DG-2 — Single-cut migration of the replaced
signatures*, § *DISTILL authorship*):

- **Authored now — additive surface only.** A body that names only additive
  surface is a complete, compiled body marked `#[ignore = "pending DELIVER
  vsock new-step N (…)"]`. For P6 that is the `overdrive-init` `transport` and
  `guest_flow` modules (DG-5): `crates/overdrive-init/src/transport/tests.rs`
  and `crates/overdrive-init/src/guest_flow/tests/session.rs` (P6-S11). They
  are source-local because `overdrive-init` is a binary crate with no library
  target; the transport tests pass the kernel command line as a string.
- **Authored at the attachment cut.** Every body or driver that calls a DG-2
  row-1..18 replacement (`GuestAddressPool::new` / `assign`, the
  `GuestNetworkPlan` reshape, `ForwardingActivation`, `quiesce_forwarding` /
  `restore_forwarding`, `HostSharedGuestNetworkOwner::new(ports)`,
  `recovery_progress() -> Vec<…>`, `Vmm::create(&cfg, claim)`, `vm_override`)
  cannot compile before the cut. DG-2 forbids an uncompiled, unregistered or
  always-false-`cfg` file, so none is in the tree. The cut's DISTILL increment
  writes these bodies in the cut's working tree, against the pinned
  contracts, before the crafter lands the cut. For P6 that is P6-S1..S6, S10,
  S12, S12b, the `quiescence` and `shared_table` drivers (P6-C1, P6-C2), and the
  walking skeleton, crash matrix and density work (P6-S13..S15).

The earlier pass's orphaned-file and gated-conformance-binary plan (unregistered
`tests/{integration,acceptance}/` files, a shared `tests/conformance/src/vsock_harness.rs`,
a `quint_driver` base) is withdrawn: DG-2 replaces it with the split above, and
conformance drivers live in their owner's crate as in-crate modules (QPROJ-1).

## Files and attachment-cut items for P6

| File / item | Status | Scenarios |
|---|---|---|
| `crates/overdrive-init/src/transport/tests.rs` (source-local, `#[cfg(test)]`) | authored; RED, `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`; two default-lane GUARD tests | P6-S11 (token parse, setup order, setup-error `Display`) |
| `crates/overdrive-init/src/guest_flow/tests/session.rs` (source-local) | authored; RED, `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`; one default-lane GUARD test | P6-S11 (session open with the guest's own token, fatal classes) |
| Quiescence and recovery bodies, source-local in `overdrive-control-plane` (from the removed 91f9436ff draft `vsock_quiescence_recovery.rs`) | attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12) | P6-S1..S6, S10, S12, S12b (feature disabled) |
| `quiescence` and `shared_table` drivers, in-crate modules in `overdrive-control-plane` | attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12) | P6-C1, P6-C2 |
| `overdrive-init` `main` wiring cell (`complete_pre_ready_init`) | attachment cut (DG-2 row 16) | P6-S11 |
| Walking skeleton, crash matrix, density through `run_server` + public API on native metal | attachment cut (DG-2 rows 1-6) | P6-S13, S14, S15, and every cell listed in P6-S13 |
| P1-S12 VMM handoff cell (`Vmm::create(&cfg, claim)` on the real fork) | attachment cut (DG-2 row 6) | the former P6-S8 `fd=` obligations |
| DELIVER new-step 1 provisioning (checksum verify, `--landlock` check, submodule pointer) | DELIVER provisioning, not a DISTILL body | P6-S9 |

Files removed and not to be recreated: `crates/overdrive-control-plane/tests/acceptance/vsock_quiescence_recovery.rs`,
`crates/overdrive-init/tests/integration/vsock_guest_ready_config.rs` (moved
source-local, DG-5). P6-S7 and P6-S8 have no test file (DG-1).
The `tests/conformance/tests/vsock_{walking_skeleton,crash_matrix,quiescence_conformance,shared_table_conformance}.rs`
files of the earlier plan are not authored; the walking skeleton, crash matrix
and density bodies are written at the cut and the two drivers are in-crate
modules in `overdrive-control-plane`. `guest_network::e18_test_support` is
deleted at the cut (DG-2 row 17).

---

## PBT-layer policy for P6 (Mandate 9 / `.claude/rules/testing.md`)

- **Unit / pure-function layer — PBT (`proptest`):** `LaunchToken`
  `Display`/`FromStr`/serde roundtrip incl. rejection of upper-case, wrong
  length and all-zero (mandatory newtype call site); the D21 errno→`Attach`
  classification map over all errnos; `guest_intake_host_port` injectivity
  (shared with P1, not re-asserted here).
- **Seeded `overdrive-sim` layer (owner private seam) — example-based:** every
  quiescence / recovery / activation / VMM-death scenario is enumerated, never
  PBT-generated; the seed is printed on failure and the triggering sequence
  retained (`.claude/rules/testing.md` §"Tier 1 — seeding").
- **Tier-3 (native metal / Lima) — example-based:** the walking skeleton, the
  crash matrix and the guest READY config are single deliberate captures per
  shape (Mandate 11).
- **quint-connect conformance — randomized `#[quint_run]` traces (seed
  printed):** the two modules below. The specs define no Quint `run`, so there
  is no `#[quint_test]` pin (`UI-R2-1`).

---

## G-V5 — runtime recovery reopen (quiescence + shared-table at the owner seam)

**Owner path:** `SharedGuestNetworkOwner::quiesce_forwarding(QuiescenceHolder)
-> (QuiescenceHold, ForwardingQuiescence)`, `restore_forwarding(QuiescenceHold)
-> ForwardingRestore`, `converge_shared()` as a per-component runtime repair,
`audit_shared()`; the runtime supervisor (RUN-295-B) drives one recovery per
damaged `SharedGuestNetworkComponent`, each holding its own
`QuiescenceHolder::Recovery(component)` (ADR-0169, D8a-HOLD). Driven through the
production owner and the runtime supervisor, with `SimGuestFlowIo` /
`SimGuestNode` at the owner's effect seam and `SimMtlsIntercept` at the shared
table (`fail_next_table_transaction`); no test hand-installs a production
effect. Every G-V5 body is authored at the attachment cut (DG-2 rows 8, 9, 10,
12), source-local in `overdrive-control-plane`.

### P6-S1 — Holder-set reopen (R5-4, R5-23; Quint quiescence inv 10) `@contract-shape:bounded-change`

- **Contract / invariant:** forwarding is open iff the holder set is empty
  (`LatchIffHolders`); the first holder latches admission, aborts every live
  flow and closes every intake listener (each close removing its steering entry
  in the same kernel step, M-7); `restore_forwarding` removes only that holder
  and reopens — latch cleared, listeners re-bound from the last `ListenState` —
  only when no holder remains. A TCP probe on a listening port fails until
  restore; restore re-binds from the last `ListenState`.
- **Fault stimulus:** script the owner seam (`SimGuestFlowIo`) so the audit
  finds one component (`FlowListeners`) damaged; the supervisor quiesces under
  `Recovery(FlowListeners)`, repairs, audits clean, restores.
- **GWT:**
  - Given a booted node whose owner serves intake listener `(alloc-A, port p)`
    with a steering entry, forwarding open (holder set empty).
  - When the supervisor quiesces forwarding under `Recovery(FlowListeners)`.
  - Then admission is latched `Quiesced`; every live flow is aborted; the intake
    listener for `(alloc-A, p)` is closed and its steering entry is gone; a
    guest-command admission poll reads `Closed`.
  - And when the recovery repairs, audits clean and restores its hold.
  - Then no holder remains, admission reopens, and a listener for `(alloc-A, p)`
    is bound and steered again because the last `ListenState(p)` is listening.
- **Oracle (state-delta, port-exposed universe):** `{admission_latch,
  holder_set, intake_listeners[(alloc-A,p)], steering_entries[(workload_addr,p)],
  owner_calls}`; `quiesce` delta = `latch: set_to(Quiesced)`,
  `holder_set: set_to({Recovery(FlowListeners)})`, listener+entry `set_to(absent)`;
  `restore` delta = `latch: set_to(Open)`, `holder_set: set_to({})`, listener+entry
  `set_to(present)`; CID table, control sessions, steering **program** and route
  `unchanged`.
- **Evidence tier:** seeded sim (layer 2) + Tier-3 (the R5-4 cell of the
  P6-S13 walking skeleton).
- **Status:** attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): the runtime supervisor over the owner
  (`HostSharedGuestNetworkOwner::with_flow_io` over `SimGuestFlowIo`) and
  `SimMtlsIntercept`; asserts O3 `holders`, `admission`, `intake`,
  `flow_listeners_running`, O6 `recovery_observation()` and S
  `SimMtlsIntercept::shared_table_state()`.

### P6-S2 — Two overlapping recoveries: `StillQuiesced` + `QuiescenceHolderBusy` (R5-32; Quint quiescence `WitnessNoStillQuiesced`, `HeldIsHolder`) `@contract-shape:bounded-change`

- **Contract / invariant:** a restore that leaves other holders returns
  `StillQuiesced { holders }` and reopens nothing; a second
  `quiesce_forwarding` by a holder already holding returns
  `Err(QuiescenceHolderBusy { holder })` with no state change; no restore ever
  ends another recovery's hold (`HeldIsHolder`); forwarding never reopens while
  another recovery holds without having repaired (`NoReopenBeforeRepair`).
- **Fault stimulus:** script both `Recovery(IpRules)` and
  `Recovery(FlowListeners)` damaged; interleave their quiesce/restore so
  `FlowListeners` restores first.
- **GWT:**
  - Given both `IpRules` and `FlowListeners` recoveries have quiesced (two
    holders held, admission latched, no intake listener up).
  - When the `FlowListeners` recovery restores its hold first.
  - Then the result is `StillQuiesced { holders: {Recovery(IpRules)} }`,
    admission stays latched and no intake listener comes up.
  - And when a holder that already holds attempts a second quiesce.
  - Then it is `Err(QuiescenceHolderBusy)` with no change.
  - And when the `IpRules` recovery repairs, audits clean and restores.
  - Then `Reopened`: admission reopens and listeners are brought back.
- **Oracle:** `{holder_set, admission_latch, restore_result, quiesce_result,
  intake_listeners}`; assert `StillQuiesced.holders == {Recovery(IpRules)}`,
  `QuiescenceHolderBusy`, final `Reopened` with latch `Open`.
- **Evidence tier:** seeded sim (layer 2) + Quint `quiescence`.
- **Status:** attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): the runtime supervisor over the owner
  (`HostSharedGuestNetworkOwner::with_flow_io` over `SimGuestFlowIo`) and
  `SimMtlsIntercept`; asserts O3 `holders`, `admission`, `intake`,
  `flow_listeners_running`, O6 `recovery_observation()` and S
  `SimMtlsIntercept::shared_table_state()`.

### P6-S3 — Absent-table repair runs only inside the `IpRules` hold (R5-30; Quint shared_table inv 11 / quiescence `RepairNeverWritesPrefix`) `@contract-shape:unbounded-preservation`

- **Contract / invariant:** the shared nft table is written only in single
  transactions, so it is absent or complete (U-QP-1). Boot step 8's
  transaction failed on a node with no table, leaving it absent; with the table
  in either state no connection to
  the guest prefix reaches a host service (`NoWildcardReached`, K-D3); the
  firewall recovery repairs leg-C interception and the D26 output rule **only
  while it holds quiescence** (`Recovery(IpRules)`); the `IpRules` repair writes
  neither a steering entry, the pinned steering program nor the prefix route
  (`RepairNeverWritesPrefix`); every listener serving after restore is steered.
- **Fault stimulus:** `SimMtlsIntercept::fail_next_table_transaction` on the
  step-8 table transaction on a node with no table (R5-30); the failed transaction leaves
  the table as it was (absent). There is no runtime partial write (S-6).
- **GWT:**
  - Given the owner is serving and the shared nft table is absent (boot step 8's
    transaction failed on a node that had no table).
  - When the audit reports the damage and the firewall recovery quiesces under
    `Recovery(IpRules)`, repairs with `converge_shared` (that component only),
    audits and restores.
  - Then at no point in the sequence does a connection to the guest prefix reach
    a host service; the repair wrote no steering entry, no steering program and
    no prefix route; and every listener serving after restore is steered.
  - And the same holds when the table is absent while the node is already
    quiesced by another recovery.
- **Oracle:** `{prefix_exposed_to_host_service, repair_wrote_steering_entry,
  repair_wrote_steering_program, repair_wrote_route, holder_set,
  intake_listeners_steered}`; assert `prefix_exposed… == false` at every step,
  the three `repair_wrote_*` flags `false`, repair only while
  `Recovery(IpRules)` in the holder set.
- **Evidence tier:** seeded sim (scripted failed table transaction) + Quint
  `shared_table`.
- **Status:** attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): the runtime supervisor over the owner
  (`HostSharedGuestNetworkOwner::with_flow_io` over `SimGuestFlowIo`) and
  `SimMtlsIntercept`; asserts O3 `holders`, `admission`, `intake`,
  `flow_listeners_running`, O6 `recovery_observation()` and S
  `SimMtlsIntercept::shared_table_state()`. The table state is read only through
  `shared_table_state()` (`SimSharedTable::Absent` | complete).

### P6-S4 — Quiesce/restore interleaved with activation, `ListenState`, teardown (R5-5 quiesce half) `@contract-shape:bounded-change`

- **Contract / invariant:** M-7 per-allocation serialization — `ListenState`
  arriving during quiescence/restore leaves the final listener state a function
  of `serving(port) ⇔ Active ∧ no holder ∧ session Live ∧ last ListenState =
  listening ∧ port ∈ listen_ports()`; a late restore after a fail-stop is
  ignored.
- **Fault stimulus:** interleave a `ListenState(p)=listening`, a quiesce, a
  `ListenState(p)=stopped`, a restore, in several orders through the owner's
  per-CID queue.
- **GWT:**
  - Given an Active allocation with the owner quiesced by a recovery and a
    pending `ListenState(p)` transition.
  - When the recovery restores and the final `ListenState(p)` says listening.
  - Then exactly one steered listener for `(alloc, p)` exists; when the final
    `ListenState(p)` says stopped, no listener exists.
- **Oracle:** `{intake_listeners, steering_entries}` final state matches the
  `serving(port)` predicate for every interleaving enumerated.
- **Evidence tier:** seeded sim (layer 2).
- **Status:** attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): the runtime supervisor over the owner
  (`HostSharedGuestNetworkOwner::with_flow_io` over `SimGuestFlowIo`) and
  `SimMtlsIntercept`; asserts O3 `holders`, `admission`, `intake`,
  `flow_listeners_running`, O6 `recovery_observation()` and S
  `SimMtlsIntercept::shared_table_state()`.

### P6-S5 — Recovery fail-stop; node-global objects stay; links not audited (D19-LINKS, UP-12; Quint quiescence `FailStopLeavesPrefixAsBooted`) `@contract-shape:unbounded-preservation`

- **Contract / invariant:** a recovery that spends its attempt budget
  fail-stops with `ServeShutdownRequest::SharedGuestNetwork`; another
  component's fail-stop leaves the route `local` behind the attached pinned
  steering (`FailStopLeavesPrefixAsBooted`); the audit never audits the
  forwarder's links (`SockOpsLink`, `DrainCounterLink`, `UnframeLink` are not
  supervisor components, D19-LINKS); no fail-closed reopen.
- **Fault stimulus:** script `Recovery(IpRules)` repair to fail for the whole
  bounded attempt budget (`RECOVERY_ATTEMPTS`).
- **GWT:**
  - Given a damaged `IpRules` component whose repair fails every attempt within
    the ADR-0124 bound.
  - When the attempt budget is spent.
  - Then the owner emits `ServeShutdownRequest::SharedGuestNetwork`; admission
    never reopens; the shared route and pinned steering remain as boot
    converged them; the audit set contains no link component.
- **Oracle:** `{shutdown_request, admission_latch, prefix_route, steering_link,
  audited_components}`; assert the fail-stop request is observed, latch never
  `Open` again, `prefix_route == local`, `steering_link == pinned`,
  `audited_components ∩ {SockOpsLink,DrainCounterLink,UnframeLink} == {}`.
- **Evidence tier:** seeded sim (layer 2) + native fault (V-8, DELIVER 10-02).
- **Status:** attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): the runtime supervisor over the owner
  (`HostSharedGuestNetworkOwner::with_flow_io` over `SimGuestFlowIo`) and
  `SimMtlsIntercept`; asserts O3 `holders`, `admission`, `intake`,
  `flow_listeners_running`, O6 `recovery_observation()` and S
  `SimMtlsIntercept::shared_table_state()`.

### P6-S6 — G-V5 boundary table rows 2/4/5 `@contract-shape:unbounded-preservation`

- Row 2 (typed failure in owner domain): fail-stop — P6-S5.
- Row 4 (late success cannot resurrect): a restore delivered after the recovery
  already fail-stopped is ignored (no reopen). GWT: Given a fail-stopped owner,
  When a late `restore_forwarding(hold)` for a now-dead recovery arrives, Then
  it is a no-op and admission stays closed.
- Row 5 (disconnect/reconnect): quiesce aborts flows; restore does **not**
  resurrect them — aborted flows are not restored (ADR-0160). GWT: Given live
  flows, When a recovery quiesces and later restores, Then the aborted flows are
  gone and only admission is reopened (no flow is resurrected).
- **Oracle:** `{flows, admission_latch}`; assert aborted flows absent after
  restore, admission reopened.
- **Evidence tier:** seeded sim (layer 2).
- **Status:** attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): the runtime supervisor over the owner
  (`HostSharedGuestNetworkOwner::with_flow_io` over `SimGuestFlowIo`) and
  `SimMtlsIntercept`; asserts O3 `holders`, `admission`, `intake`,
  `flow_listeners_running`, O6 `recovery_observation()` and S
  `SimMtlsIntercept::shared_table_state()`.

---

## G-V4 — VMM vsock device (CloudHypervisorVmm + fork)

**Owner path:** `overdrive-host::CloudHypervisorVmm` implementing
`overdrive_core::traits::vmm::Vmm`: `probe()` is unchanged (DG-1);
`create(&VmConfig, ClaimedGuestCid) -> Result<VmProcess>` launches
`--vsock cid=…,backend=vhost-kernel,fd=K` with the claimed `/dev/vhost-vsock`
descriptor handed over (D2, D16-CLAIM, ADR-0146/0170). That argument is the
launch contract of the vendored fork; no probe identifies the installed Cloud
Hypervisor build (DG-1). The fork `fd=` mode (ADR-0161) is DELIVER new-step 1;
the launch through `Vmm::create(&cfg, claim)` is DG-2 row 6 at the attachment
cut. CID-claim admission half is P1.

### P6-S7 — withdrawn (DG-1)

Overdrive runs only with the vendored fork; no probe identifies the installed
Cloud Hypervisor build and no typed error exists for it. `/dev/vhost-vsock`
reachability — a host fact — is the claim probe's (G-V0/P1, P1-S12).

### P6-S8 — withdrawn with the VMM backend probe (DG-1); `fd=` launch obligations relocated (V-1(c), D2, D16-CLAIM; R5-19 (d)(e); G-V4 rows 1/4/5)

P6-S8 is withdrawn as a separate test (DG-1). The launch contract and GWT
below are kept as the obligations it carried; they are discharged by **P1-S12's
VMM cell** (attachment cut, DG-2 row 6: `Vmm::create(&cfg, claim)` on the real
fork; asserts the CID is held by the VMM until it exits) and by **P6-S13** (the
walking skeleton boots every VM through the production launch). The pre-READY
exit projection is P6-S10's.

- **Contract / invariant:** `create(config, claim)` installs the claimed
  `/dev/vhost-vsock` open file description at descriptor `K`, passes exactly
  `--vsock cid=<claim.cid()>,backend=vhost-kernel,fd=K`, no `socket=`, no second
  vsock device; the adapter closes its copy as soon as spawn returns (so the
  claim lives exactly as long as the VMM); the fork never opens
  `/dev/vhost-vsock` (needs no Landlock path rule) and issues
  `VHOST_SET_OWNER`/ring setup on `K`; the descriptor is consumed once — a
  second device creation (guest reboot) finds it consumed and the VMM exits
  non-zero, never running the guest without its device; `ENOTTY`/`EBADF` on `K`
  → pre-READY exit, CID stays claimable; after `create` returns `Ok`,
  `serve` holds no copy and `take_claim` returns `None`; a failed `create` and
  a `release` before launch each free the CID (within 10 ms of reaped exit).
- **Fault stimulus:** a real claimed device on qualified metal (V-25 handoff
  prototype shape, P-39); a scripted non-vhost descriptor (`ENOTTY`); a second
  device-creation attempt.
- **GWT (host integration, `kvm-tests` / native metal):**
  - Given a `ClaimedGuestCid` claimed on the host kernel for CID `c`.
  - When `CloudHypervisorVmm::create(config, claim)` launches the fork.
  - Then the fork boots and passes data over the device; the VMM never opened
    `/dev/vhost-vsock` (no path in its Landlock ruleset); the launch argument is
    exactly `--vsock cid=c,backend=vhost-kernel,fd=<K>`; after `Ok` the
    assignment's `take_claim()` returns `None`; and when the VMM is reaped the
    CID is claimable again within 10 ms.
  - And given a descriptor that is not a vhost-vsock device, the fork exits
    before READY (existing pre-READY start failure) and CID `c` stays claimable.
- **Oracle:** `VmProcess` observable (boots + reaped); CID re-claimable after
  reap through `GuestCidClaim::claim`; `take_claim() == None` after `Ok`;
  launch-argument shape observed from the spawned command line (host adapter
  surface). Sim half asserts the pre-READY-exit projection.
- **Evidence tier:** native metal (V-1(c); the handoff kernel facts are P-39).
- **Status:** withdrawn (DG-1). Obligations at the attachment cut: P1-S12 VMM
  cell (DG-2 row 6) and P6-S13 (DG-2 rows 1-6).

### P6-S9 — D13/D21 fork provisioning: checksum verify (D21) `@contract-shape:unbounded-preservation`

- **Contract / invariant:** provisioning downloads the fork release asset,
  verifies SHA-256 before install and refuses on mismatch, then refuses unless
  `--help` lists `--landlock` (today's check); the submodule pointer equals
  `CLOUD_HYPERVISOR_FORK_REV` (consistency test, xtask-side, no `overdrive-*`
  dependency). The checksum is the identity check (DG-1).
- **Evidence tier:** the SHA-256 verify + submodule-pointer consistency test
  are **xtask-side** (no `overdrive-*` dep).
- **Status:** DELIVER new-step 1 provisioning (no DISTILL body). The
  provisioning checksum is the identity check; no provisioning check or typed
  error identifies the installed Cloud Hypervisor build (DG-1).

### P6-S10 — VMM death aborts flows; pre-READY exit; late registration (G-V4 rows 4/5) `@contract-shape:bounded-change`

- **Contract / invariant:** a VMM death (release of its vhost device) resets
  every host-side vsock connection whose peer is that CID (K-A1/K-A2′, V-10/V-22
  evidence) and aborts the CID's flows; a pre-READY VMM exit is the existing
  non-terminal start failure (lease retired and released, next placement claims
  afresh); a late registration after the alloc is torn down does not resurrect
  it; a CID clash cannot reach this gate (D16-CLAIM).
- **Fault stimulus:** `SimVmm` scripts a pre-READY exit and a VMM death
  mid-flow; the owner's effect seam (`SimGuestFlowIo`) delivers the orphaned
  connections.
- **GWT (owner seeded-sim):**
  - Given an Active allocation with live flows.
  - When its VMM dies (device released).
  - Then every flow of the CID is aborted and the owner observes no new flow
    registered on a check the death made false.
  - And when a flow request of the dead launch is delivered after teardown, it
    is closed unattributed (`StaleLaunch`), takes no claim, registers nothing
    (ATTR-TOKEN; cross-ref R5-41 in P2).
- **Oracle:** `{flows, owner_calls, registered_flows}`; assert flows aborted,
  no late registration, stale-launch connection closed unattributed.
- **Evidence tier:** seeded sim (layer 2) + Tier-3 (inside R5-10 crash matrix).
- **Status:** attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): source-local in
  `overdrive-control-plane`, the runtime supervisor over the owner (seam
  `SimGuestFlowIo`) and `SimMtlsIntercept`; asserts O3 `holders`, `admission`,
  `intake`, `flow_listeners_running`, O6 `recovery_observation()` and S
  `shared_table_state()`. The Tier-3 half is a P6-S14 crash-matrix cell (DG-2
  rows 1-6).

---

## G-V2 — guest command release

**Owner path:** `VmDriver::release_for_exit_emission` under
`exec.claim_release()` after `activate_guest_network` (ADR-0131); the owner's
`activate` is the only Provisioned → Active edge.

### P6-S12 — EXEC only after Active; retries; late activation after teardown (G-V2 rows 1/2/4) `@contract-shape:bounded-change`

- **Contract / invariant:** the workload command starts only after forwarding is
  Active for the allocation (row 1); EXEC before Active → the application's
  first connect is refused `NotActive` and its first listen is never mirrored
  (counterexample); `activate` returning `QuiescenceLatched` /
  `ControlSessionPending` retries under the EXEC claim's existing ADR-0131
  budget, then `WorkloadNetnsProvisionFailed` stage `guest_network_activate`
  (row 2); `activate` after `teardown` → `LeaseRetiring`, no EXEC (row 4); READY,
  Running, intercept-live, other allocations' EXEC and Service `Stable` are
  unaffected (row 3).
- **Fault stimulus:** script the owner to return `QuiescenceLatched` then
  `ControlSessionPending` then `Activated`; a separate run returns
  `LeaseRetiring` after teardown.
- **GWT (owner seeded-sim):**
  - Given an allocation that reached intercept-live.
  - When `activate` returns `QuiescenceLatched`, then `ControlSessionPending`,
    then `Activated` within the retry budget.
  - Then EXEC is released exactly once, after `Activated`, and never before.
  - And when `activate` is called after `teardown` began.
  - Then it returns a retiring projection and EXEC is never released.
- **Oracle:** `{active_flag, exec_released, activate_result}`; assert EXEC
  released iff `Activated` reached, never on `QuiescenceLatched` /
  `ControlSessionPending` / retiring.
- **Evidence tier:** seeded sim (activation racing quiescence and teardown) +
  native WS (V-6).
- **Status:** attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): source-local in
  `overdrive-control-plane`, the runtime supervisor over the owner (seam
  `SimGuestFlowIo`) and `SimMtlsIntercept`; asserts O3 `holders`, `admission`,
  `intake`, `flow_listeners_running` and O6 `recovery_observation()`. The
  `AllocationSpec.transport` `None` vs `Some` cell (C5a / C5b) is P6-S12b
  below (feature-delta § *Prerequisites and Completeness*).

### P6-S12b — An allocation without a guest transport runs exactly as today (G-V gates row 6, feature disabled; C5a / C5b) `@driving_port @contract-shape:unbounded-preservation`

- **Contract / invariant:** the feature-disabled boundary row of every G-V
  gate: an allocation whose `AllocationSpec.transport` is `None` takes none
  of the new mechanism — no lease, no CID claim, no `activate` — and runs on
  today's path, so its `alloc_status` equals the pre-feature `network: None`
  baseline.
- **Fault stimulus:** none; the contrast is `transport: None` against
  `transport: Some(..)` on the same driver.
- **GWT (owner seeded-sim):**
  - Given a `SimDriver` allocation whose `AllocationSpec.transport` is `None`.
  - When it is started through the action shim.
  - Then no lease is taken and no CID is claimed (O2 `lease_observation()`
    and `SimGuestCidClaim::held_cids()` unchanged); the owner receives no
    `activate` call; EXEC is released on today's path; and the
    `alloc_status` row equals the pre-feature `network: None` baseline.
  - And the same allocation with `transport: Some(..)` takes a lease, claims
    a CID and is activated before EXEC (the C5b orthogonality contrast).
- **Oracle:** O2 `lease_observation()`, `held_cids()`, the owner's recorded
  calls (no `activate`), EXEC release, `alloc_status` compared with the
  baseline row.
- **Evidence tier:** seeded sim (layer 2), production action shim.
- **Status:** planned at the attachment cut (DG-2 rows 1, 2, 3, 4, 7, 9, with
  P6-S12): the body names `AllocationSpec.transport` and the reshaped pool and
  owner, so it is written in the cut's working tree. It closes C5a and C5b.

---

## G-V7 — guest network configuration before READY

**Owner path:** `overdrive-init` `parse_transport_token` and
`configure_guest_transport` (over the `GuestTransportOps` port; production
`KernelGuestTransportOps`, whose constructor is crafter structure, `UI-R2-3`
closed), run by
`complete_pre_ready_init` before READY (DG-5)
(ADR-0082 §D4): `lo` up; dummy device up with `workload_addr/32`; the fwmark
rule + `net.ipv4.fwmark_reflect=1`; `ip_local_reserved_ports =
guest_reserved_ports()` and every intake bound (D25); guest programs loaded and
attached; the listener map seeded from two `sock_diag` `TCPF_LISTEN` dumps
(D23); the inbound vsock listener bound; the kernel-command-line transport
token parsed (`overdrive.transport=vsock2,addr=…,dns=…,launch=<LaunchToken>`).

### P6-S11 — Guest config completes before READY; token parse; typed setup failures (G-V7 rows 1/2/4) `@contract-shape:bounded-change`

- **Contract / invariant:** READY is reached only after guest network
  configuration is complete (meaning unchanged); attaching programs after READY
  would let EXEC start an application whose first `connect()` has no route
  (`ENETUNREACH`) — so configuration is strictly before READY; every typed
  `GuestTransportSetupError` variant makes `overdrive-init` exit before READY
  (existing "guest exited before READY" start failure); a late READY after a
  stop is ignored (asserted by P2-S14's attachment-cut cell "a read after
  `stop` takes nothing").
- **Fault stimulus:** malformed / missing / duplicate transport token; a scripted
  `sock_diag` dump failure (`ListenerSeed`); a reserved-port intake bind failure.
- **GWT:**
  - (Unit, PBT) Given a kernel command line, When the transport token is parsed,
    Then exactly one `overdrive.transport=vsock2,addr=A,dns=G,launch=T` with
    fields in that order parses to `(A, G, T)`; a missing token →
    `TokenMissing`; a duplicate → `TokenDuplicate`; an extra/reordered field or
    a non-`LaunchToken::from_str` launch → `TokenMalformed { field }`; an
    all-zero launch token is rejected.
  - (Tier-3, inside the WS run) Given a guest booting with a valid token, When
    `overdrive-init` configures the guest network, Then before READY the dummy
    device carries `workload_addr/32`, the fwmark rule and `fwmark_reflect` are
    set, `ip_local_reserved_ports == guest_reserved_ports()`, the intakes are
    bound (TCP 61000; UDP 61001–62024), the guest programs are attached, the
    listener map is seeded, and the inbound vsock listener is bound; READY is
    sent only after.
  - (Seeded sim) Given a scripted `GuestTransportSetupError`, Then the guest
    exits before READY and the host sees the existing start failure.
- **Oracle:** unit — the typed parse result / error variant; Tier-3 — kernel
  observation (`ip addr`/`ip rule`/`sysctl`/`ss`) that each configuration item
  is present before the READY beacon line; sim — the pre-READY-exit projection.
- **Evidence tier:** unit (PBT token roundtrip) + seeded sim (pre-READY exit) +
  native Tier-3 (P-19 bounded; V-6).
- **Status — authored half:** `overdrive-init` `transport::tests`
  (`crates/overdrive-init/src/transport/tests.rs`, source-local; the kernel
  command line is passed as a string; kernel steps go through a test-local
  recording `GuestTransportOps`): `well_formed_token_parses_and_other_parameters_are_ignored`,
  `missing_and_duplicate_tokens_are_refused`, `first_invalid_position_names_the_field`,
  `missing_trailing_field_names_the_first_missing_one`, `token_rendering_roundtrips`,
  `configure_runs_every_step_in_the_pinned_order`,
  `configure_stops_at_the_first_failure_with_that_steps_variant` — ignored
  `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`;
  `every_setup_error_renders_its_pinned_display_text` and
  `label_enums_render_lower_snake_case` run in the default lane (GUARD). And
  `guest_flow::tests::session` (`crates/overdrive-init/src/guest_flow/tests/session.rs`):
  `session_opens_with_a_control_open_carrying_the_guests_own_token`,
  `listen_state_reports_the_full_map_then_each_change`,
  `host_owner_restart_resets_flows_and_reopens_the_session_without_fatal`,
  `every_session_loss_shape_is_recovered_not_fatal`,
  `only_event_wait_and_listener_failures_are_fatal` — ignored
  `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`;
  `guest_owner_fatal_renders_its_pinned_display_texts` in the default lane
  (GUARD).
- **Status — attachment-cut half:** the `main` wiring cell (DG-2 row 16):
  `complete_pre_ready_init` writes one console line on a setup error and starts
  the owner thread before READY. The in-guest Tier-3 configuration
  observations (`ip` / `sysctl` / `ss` before the READY line) are a P6-S13 cell
  (DG-2 rows 1-6).

---

## Walking skeleton, crash matrix, fail-closed, density (attachment cut, DG-2 rows 1-6)

These bodies are authored at the attachment cut (DG-2 rows 1-6; feature-delta
§ *Attachment-cut DISTILL increment*, item "Walking skeleton P6-S13..S15, P3-S3,
P5-S8 `gateway:53` / guest-broadcast cells"). They start `run_server`
in-process on native metal and deploy through the public HTTPS API (the
in-process `overdrive deploy`), observing only the observable surface (public
API, `alloc_status`, `ss`, `bpftool net/link/map`, `ip route`, wire capture) —
never private owner fields. They use no shared conformance harness (no
`boot_vsock_node`, no `ConformanceNode`); until the cut, the authored tests
that drive the production composition end to end are P4-S19
(`vsock_prefix_landing_conformance`) and P5-S8 (`vsock_host_internal_enforce`).

### P6-S13 — End-to-end walking skeleton (V-6) `@walking_skeleton @real-io @driving_port @contract-shape:bounded-change`

- **Contract / invariant (`@walking_skeleton`, user value):** a VM service,
  started through an in-process `run_server` + the public HTTPS API (the in-process `overdrive deploy`), dials a mesh peer by
  name, reaches a non-mesh address over TCP and over UDP (including an empty
  datagram) through the real NIC, reaches a mesh service's VIP over TCP (mTLS
  through leg-F, D24a) and a service VIP over UDP (`connect4`, V-15 folded in),
  is reached inbound, and TCP probes follow the application's listen state.
- **GWT:**
  - Given a booted vsock node and a deployed VM service that listens on a
    declared port and reaches several destinations.
  - When a mesh peer dials the service by name, the service reaches a non-mesh
    address (TCP + an empty UDP datagram) and a mesh VIP (TCP mTLS + UDP), and an
    external client connects inbound.
  - Then every leg delivers its bytes byte-exact; the inbound connection reaches
    the guest application; the service is demonstrably usable end to end.
- **Oracle:** application-visible round trips (byte-distinct request/response
  litmus), the public `alloc_status` reaching Running, `ss -ltn` showing the
  steered intake listener appear/disappear with the guest's listen state.
- **Evidence tier:** native metal (`cargo xtask metal run --`), single capture.
- **Status:** attachment cut (DG-2 rows 1-6): written by the cut's DISTILL
  increment and driven through `run_server` + the public API on native metal
  (`cargo xtask metal run --`); asserts the end-to-end paths (V-6).
- **Cells carried into the same run.** Every cell another scenario hands to
  the walking skeleton, by name, with what it drives and what it asserts. Each
  is driven through `run_server` + the public API on native metal at the
  attachment cut (DG-2 rows 1-6); none hand-installs a production effect.
  - **Fork boot: guest-reported CID** (from P1-S12, P1-S6). Drives: deploy a
    VM launched by `Vmm::create(&cfg, claim)` on the vendored fork. Asserts:
    the peer CID the guest reports equals the claimed CID.
  - **Fork boot: byte round trip** (from P1-S12). Drives: a byte-distinct
    request / response over the guest's vsock device. Asserts: both directions
    arrive byte-exact.
  - **Fork boot: pre-READY exit on a bad descriptor** (from P1-S12, former
    P6-S8). Drives: a launch whose descriptor `K` is not a vhost-vsock device
    (`ENOTTY`) or is closed (`EBADF`). Asserts: the fork exits before READY,
    the start failure is the existing pre-READY projection, and the CID is
    still claimable (a fresh `GuestCidClaim::claim` succeeds), never a CID
    clash.
  - **Former P6-S8 `fd=` launch obligations** (DG-1). Drives: every VM of the
    run launched through `Vmm::create(&cfg, claim)`. Asserts: the spawned
    command line carries exactly `--vsock cid=<c>,backend=vhost-kernel,fd=<K>`
    with no `socket=` and no second vsock device; after `Ok`, `take_claim()`
    returns `None`; the CID is claimable again within 10 ms of the VMM's reap.
    (The CID-held-until-exit assertion is P1-S12's VMM cell, DG-2 row 6.)
  - **Guest reboot in `fd=` mode ends the VMM** (G-V7 / G-V4 row 5; C1b
    descriptor consumed once). Drives: a reboot of a running guest. Asserts:
    the second device creation finds the descriptor consumed and the VMM exits
    non-zero; the guest never runs without its device; the allocation follows
    the existing VMM-exit path.
  - **P6-S11 in-guest configuration before READY.** Drives: a guest booting
    with a valid transport token. Asserts, before the READY beacon line: the
    dummy device carries `workload_addr/32`, the fwmark rule and
    `fwmark_reflect` are set, `ip_local_reserved_ports ==
    guest_reserved_ports()`, the intakes are bound, the guest programs are
    attached, the listener map is seeded and the inbound vsock listener is
    bound (`ip` / `sysctl` / `ss` inside the guest).
  - **R5-4 holder-set reopen, Tier-3** (from P6-S1). Drives: a recovery
    quiescing and restoring forwarding on the running node. Asserts: a TCP
    probe on a listening port fails while the holder holds and succeeds after
    restore, re-bound from the last `ListenState`.
  - **R5-41 kill-and-reuse** (from P2-S9, P2-S14; G-V3, G-V6). Drives: kill a
    VM mid-traffic, then deploy a new VM that reuses its CID. Asserts: the
    dead launch's queued control, beacon and flow connections are closed
    unattributed; the new launch's session, flows and READY are accepted; no
    message crosses launches.
  - **R5-28 / R5-31 V-24 shape** (from P2-S10; the Tier-3 half). Drives: a
    host-side close of a `TcpAccept` flow before the guest pairs it.
    Asserts: the guest's half ends (request read, then EOF, `POLLRDHUP`, first
    write `EPIPE`) and no byte of the aborted flow reaches the guest
    application.
  - **R5-13 / V-20 intake-child identity in the guest kernel** (from P4-S10).
    Drives: an intake listener and a leg-C transparent child on the same local
    address and port. Asserts: the guest kernel's `sock_ops` tags only the
    intake's children in the clone-flagged intake-tag map.
  - **R5-7 / F-4 forwarder-redirect transmit error** (from P5-S6). Drives: a
    guest datagram association whose destination becomes host-local after the
    owner's check, so the forwarder's redirect transmit on H is rejected by
    the marked-socket output rule. Asserts: the host service receives 0
    bytes, the association ends `DestinationUnreachable`, and the next
    datagram is refused `HostInternal`.
  - **P4-S5 reply-path rules half.** Drives: a host-local client whose source
    lies in the prefix connecting to a steered intake, then a fresh unmarked
    host-local connect with no entry. Asserts: the first completes both ways;
    the second is rejected at once.
  - **P3-S3 reserved intake port bind.** Drives: application binds on the
    reserved ports in a real guest. Asserts: `EADDRINUSE` on a port holding an
    intake of that protocol, `Ok` on the free pairs; the TCP intake absent
    from two `TCPF_LISTEN` dumps.
  - **P3 Tier-3 datagram cells** (P3-S5, S6, S7, S9b, S9c, S10). Drives: real
    guest datagram traffic. Asserts: parked frames discarded on association
    end (S5); the refused burst re-associates only on the next datagram (S6);
    concurrent associations lose no unattributed datagram (S7); host→guest on
    STREAM, guest→host on SEQPACKET, no host write on SEQPACKET (S9b); guest
    DNS to `gateway:53` resolves through the node `DnsResponder` (S9c); slot
    exhaustion `EPERM` counted `guest_udp_slot_exhausted` (S10).
  - **P5-S8 `gateway:53` accept and guest-broadcast reject** (UI-R2-14,
    DG-16). Drives: marked UDP from the owner to `gateway:53` and to the guest
    prefix's broadcast address, with the prefix route `converge_shared`
    wrote. Asserts: `gateway:53` is reachable; the broadcast reaches no host
    service.
  - **P5-S9 non-mesh egress end to end** (DG-16). Drives: guest TCP and UDP
    to a non-mesh address through the real NIC. Asserts: the destination
    receives the bytes; no leg-F hop appears on the wire.
  - **P5-S7 / P5-S10 mesh and VIP egress** (V-6). Drives: guest TCP to a mesh
    peer by name and to a mesh VIP, UDP to a service VIP. Asserts: the
    inter-agent leg carries TLS 1.3 records with zero cleartext; the VIP
    datagram reaches the backend while the guest's reported peer is the VIP.
  - **P5-S3 USO path through `run_server`.** Drives: guest datagrams leaving
    through a UDP-segmentation-offload interface of the composed node.
    Asserts: byte-exact arrival at the peer.
  - **P2-S14 production beacon adapter.** Drives: every VM's READY through
    the production `VsockBeaconIo` (constructor unpinned, UI-R2-2). Asserts:
    the allocation reaches Running only after its own launch's READY.

### P6-S14 — Owner-death crash matrix: reset-never-EOF + clean-close half-close (R5-10, V-13, HALFCLOSE-SIGNAL, R5-45 Tier-3 cell) `@contract-shape:unbounded-preservation`

This body replaces the retired OLD-model
`tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs` (§7 of the
shared-infra doc; retired in DELIVER with the old production code).

- **Contract / invariant:** owner death mid-transfer, each owner in turn, one
  observer per far end moving data both ways before the kill; the observer's
  first terminal event is a **reset** (`ECONNRESET` on read, or the connect/accept
  peer's error) and **never EOF**. Host owner (`serve`) SIGKILL: far ends it
  holds (non-mesh remote D, inbound client A, mesh peer through leg-F) reset by
  `SO_LINGER{1,0}`; far ends `overdrive-init` holds (guest application,
  outbound, inbound I/C) reset by `overdrive-init`'s abort on the unannounced
  flow-vsock end (`UnannouncedVsockEnd`, HALFCLOSE-SIGNAL). Guest owner death
  (`overdrive-init` killed, or guest kernel ending): far ends it holds reset by
  linger; far ends the host owner holds reset by the host owner's abort on the
  unannounced vsock end, before control-session loss is observed. Both owners
  killed 0 ms and 40 ms apart, in both orders. **Clean-close regression in the
  same run:** 2 MB each way with a half-close from each application, every byte
  delivered and a clean EOF at each end, with the `HalfClose`/`HalfCloseAck`
  exchange observed (H-5, G-CRASH, HALFCLOSE-SIGNAL, P-42).
- **GWT:**
  - Given live TCP flows through the booted node, one observer per far end
    moving data both ways.
  - When the host owner (then the guest owner; then both, 0 ms and 40 ms apart,
    both orders) is `kill -9`ed mid-transfer.
  - Then each observer's first terminal event is a reset, never a clean EOF.
  - And in the clean-close regression, a 2 MB bidirectional transfer with an
    application half-close delivers every byte and ends with a clean EOF at each
    end, the `HalfClose`/`HalfCloseAck` exchange observed.
- **Oracle:** each far end's first terminal event (reset vs EOF) — the V-13
  matrix oracle; clean-close delivers all bytes + clean EOF; `HalfClose`/
  `HalfCloseAck` observed on the control session.
- **Evidence tier:** native metal (the V-13 matrix through `serve` + `deploy`) +
  seeded sim (owner seam: vsock end before/after session loss — authored in the
  owner file P6-S10/S5 shape) + Quint `owner_flows` half-close (P2's module;
  replayed here as the crash-matrix cell).
- **Status:** attachment cut (DG-2 rows 1-6), clean-close / `HalfCloseAck` half pinned to the UI-R2-5 (d) decision (cannot be finalised until DESIGN pins the drain trigger), through `run_server` + the public
  API on native metal. The off-host remote uses a `TestCidrLease` route-disjoint
  CIDR (see § "Real-kernel fixture policy").

### P6-S15 — Fail-closed composition (V-8) + density & costs (V-9) `@contract-shape:unbounded-preservation`

- **V-8 (fail-closed):** owner SIGKILL, leg-F loss, control-session loss in the
  production composition; seeded sim for quiesce/restore (no link-detach
  injection — on the appliance only Overdrive could detach a held link,
  D19-LINKS). Oracle: no connection to the prefix reaches a host service at any
  point; admission stays closed while a holder holds. Status: attachment cut
  (DG-2 rows 1-6) for the production composition; the quiesce/restore half is
  the P6-S1..S6 bodies (attachment cut, DG-2 rows 2, 3, 7, 8, 9, 10, 12).
- **V-9 (density & costs):** through production at 1 / 64 / 1,024 / 4,096 /
  16,384 attachments — audit and quiesce bounds (each recovery ≤ ADR-0124's
  5 s), per-flow fds/threads/CPU, boot time and kernel memory of the 65,536-cell
  host pool, guest READY latency, listen-state lag p50/p99 ≤ 2 ms, close-storm
  half-close latency (four control messages per clean close). A result outside a
  pinned bound is **surfaced to the user, never relaxed silently**. This is a
  DELIVER **measurement** obligation (classify: benchmark/stress, not an EDD
  expectation; `.claude/rules/testing.md` §"Classify external execution"), not a
  per-PR AT gate. Status: attachment cut (DG-2 rows 1-6) — it measures the
  production composition the cut lands; V-9 replaces E18 (08-04 **Replace**),
  and `bin/netns_density_benchmark.rs` with `guest_network::e18_test_support` is
  deleted at the cut (DG-2 row 17).

---

## Quint conformance drivers (attachment cut)

> **QPROJ status (2026-10-10).** Placement, `State` projection and action map
> are pinned by feature-delta QPROJ-1 to QPROJ-8 and recorded in
> [`test-scenarios.md` § Quint conformance drivers](test-scenarios.md#quint-conformance-drivers-qproj-1-to-qproj-8-approved-2026-10-09).
> Both drivers (`quiescence`, `shared_table`) are in-crate `quint-connect`
> modules in `overdrive-control-plane`, authored at the attachment cut (DG-2):
> they drive `quiesce_forwarding` / `restore_forwarding` and
> `HostSharedGuestNetworkOwner::with_flow_io` over the reshaped owner, which do
> not compile before the cut. The seams are `SimGuestFlowIo` / `SimGuestNode`
> (`socket_observation`, `steering_observation`, `steering_effects`) and
> `SimMtlsIntercept` (`shared_table_state`, `fail_next_table_transaction`). No
> `tests/conformance` file, shared `quint_driver` base, `e18_test_support`
> (deleted at the cut, DG-2 row 17) or `ConformanceNode` is involved.

Per `.claude/rules/testing.md` §"Quint conformance": each driver drives the
production owner's state machine at its driving ports (`Sim*` at driven ports),
projects `State` from the owner's **observable** surface, references the spec
**in place** (`specs/quint/guest-flow-owner/<module>.qnt` — never copied), and
runs `#[quint_run]` randomized traces with the seed printed on failure. The
specs define no Quint `run`, so there is no `#[quint_test]` pin (`UI-R2-1`).
The driver spawns the `quint` CLI → gated behind `integration-tests`
(`#[cfg(all(test, feature = "integration-tests"))]`), run via
`cargo xtask lima run --`.

### P6-C1 — `quiescence.qnt` conformance (in-crate driver, `overdrive-control-plane`) `@contract-shape:bounded-change`

- **Module:** `specs/quint/guest-flow-owner/quiescence.qnt` (inv 10, D8a-HOLD).
- **Action → owner driving port mapping:** `RecQuiesce(c)` →
  `quiesce_forwarding(Recovery(c))`; `RecRepair(c)` → `converge_shared()` for
  component `c` under the hold; `RecRedo(c)` → a failed post-repair
  `audit_shared()`; `RecRestore(c)` → `restore_forwarding(hold)`; `FailStop(c)`
  → the supervisor's bounded-attempt exhaustion emitting
  `ServeShutdownRequest::SharedGuestNetwork`; `Damage`/`Crash`/`Restart`/`BringUp`
  → scripted seam faults (`SimGuestFlowIo`, `SimMtlsIntercept`) and owner
  boot / restart through `with_flow_io`.
- **State projection (QPROJ, superseding):** `holders` and `latch` ←
  `flow_observation()` (`holders`, `admission`); `rec`, `att` ←
  `recovery_observation()`; `lis` ← `flow_observation()` `intake`;
  `dmg(IpRules)` ⇔ `shared_table_state()` is `Absent`; `dmg(FlowListeners)` ⇔
  ¬`flow_listeners_running`; `prefix` ← `steering_effects()`.
- **`checks.toml` entries (invariant / witness checks run by `cargo xtask
  quint check`, not Quint runs; the driver does not replay them):**
  `quiescence-safety-tlc`, `quiescence-live-safety-tlc`,
  `quiescence-safety-apalache` (invariant `Safety` = `LatchIffHolders`,
  `HeldIsHolder`, `NoReopenBeforeRepair`, `NoRepairWithoutHold`,
  `QuiescedClosesListeners`, `RepairNeverWritesPrefix`,
  `FailStopLeavesPrefixAsBooted`); progress runs
  `quiescence-ip-rules-episode-ends`, `quiescence-flow-listeners-episode-ends`,
  `quiescence-ip-rules-repaired`, `quiescence-ok-ip-rules-repaired`. The
  driver replays randomized `#[quint_run]` traces over `quiesce_ok` /
  `quiesce_live`.
- **Invariants each conformance path exercises (every spec invariant has ≥1
  path):** inv 10 via `LatchIffHolders` + `HeldIsHolder` +
  `NoReopenBeforeRepair` + `NoRepairWithoutHold` + `QuiescedClosesListeners`;
  the repair-never-writes-prefix clause via `RepairNeverWritesPrefix`; the
  fail-stop-keeps-prefix clause via `FailStopLeavesPrefixAsBooted`.
- **Evidence tier:** quint-connect conformance (Lima; `cargo xtask lima run --`).
- **Status:** attachment cut (DG-2 rows 1-3, 7-10, 12 per the driver table;
  AC item "Quiescence and recovery … `quiescence` / `shared_table` drivers":
  rows 2, 3, 7, 8, 9, 10, 12): an in-crate `quint-connect` module in
  `overdrive-control-plane`, written by the cut's DISTILL increment. Actions map
  over `SimGuestFlowIo` and `SimMtlsIntercept`; `State` is read from O3
  `flow_observation()`, O6 `recovery_observation()`, S `steering_observation()`,
  `steering_effects()` and `SimMtlsIntercept::shared_table_state()`.

### P6-C2 — `shared_table.qnt` conformance (in-crate driver, `overdrive-control-plane`) `@contract-shape:bounded-change`

- **Module:** `specs/quint/guest-flow-owner/shared_table.qnt` (inv 11, K-D3).
- **Action → owner driving port mapping (QPROJ, superseding):** `BootStep` →
  the next composition step (`start_shared_owner` on `SimMtlsIntercept`, then
  `converge_shared`); `RulesFail` / `MtlsReplaceFail` → `SimMtlsIntercept`
  scripted failure of the table transaction, which leaves the table as it was; `BringUp(p)` → the owner binds/listens/steers a wanted port;
  `RecQuiesce`/`RecRepair`/`RecRestore`/`FailStop` as in P6-C1 for the `IpRules`
  (table) and `FlowListeners` recoveries; `Crash`/`Restart` → process exit /
  re-boot (boot step 8 writes the table in one transaction: absent or complete).
- **State projection (QPROJ-7, superseding):** `table` (`absent` | `complete`)
  ← `SimMtlsIntercept::shared_table_state()`; `link`, `route`, `entries` ← the
  owner steering seam sim's `steering_observation()`; `lis`, `holders`,
  `flDmg` ← `flow_observation()`; `rec`, `att` ← `recovery_observation()`.
- **`checks.toml` entries (invariant / witness checks, not Quint runs; the
  driver does not replay them):**
  `shared-table-safety-tlc`, `shared-table-live-safety-tlc`,
  `shared-table-safety-apalache` (invariant `Safety` = `NoWildcardReached`,
  `RemoteNeverDirect`, `NothingDeliveredWhileDown`, `NoReopenBeforeRepair`,
  `QuiescedClosesListeners`); progress `shared-table-repaired`,
  `shared-table-ok-repaired`. The driver replays randomized `#[quint_run]`
  traces over `table_ok` / `table_live`.
- **Invariants each conformance path exercises:** inv 11 (K-D3) via
  `NoWildcardReached` with the table complete and absent;
  `RemoteNeverDirect` (K-L5); `NothingDeliveredWhileDown` (K-L4);
  `NoReopenBeforeRepair`; `QuiescedClosesListeners`.
- **Evidence tier:** quint-connect conformance (Lima).
- **Status:** attachment cut (DG-2 rows 1-3, 7-10, 12 per the driver table;
  AC item "Quiescence and recovery … `quiescence` / `shared_table` drivers":
  rows 2, 3, 7, 8, 9, 10, 12): an in-crate `quint-connect` module in
  `overdrive-control-plane`, written by the cut's DISTILL increment. Actions map
  over `SimGuestFlowIo` and `SimMtlsIntercept`; `State` is read from O3
  `flow_observation()`, O6 `recovery_observation()`, S `steering_observation()`,
  `steering_effects()` and `SimMtlsIntercept::shared_table_state()`.

---

## 6-row boundary-scenario placement (feature-delta §"Required boundary scenarios") — P6 gates

| Row | G-V2 | G-V4 | G-V5 | G-V7 |
|---|---|---|---|---|
| 1 available | EXEC after Active (P6-S12) | VM boots with its CID (P1-S12 VMM cell, P6-S13; P6-S8 withdrawn, DG-1) | reopen (P6-S1) | READY (P6-S11) |
| 2 typed failure | activation-failure projection (P6-S12) | pre-READY exit start failure (P6-S10) | fail-stop (P6-S5) | guest exits before READY (P6-S11) |
| 3 unrelated unchanged | READY unchanged (P6-S12) | other VMs unaffected (P6-S10) | leases unchanged (P6-S5) | other VMs unaffected (P6-S11) |
| 4 late success can't resurrect | no EXEC after teardown (P6-S12) | late registration after stop torn down (P6-S10) | late restore after fail-stop ignored (P6-S6) | late READY after stop ignored: P2-S14's attachment-cut cell "a read after `stop` takes nothing" |
| 5 disconnect/reconnect | beacon reconnect refused (cross-ref P2 ATTR-TOKEN) | VMM death → flows aborted (P6-S10) | quiesce aborts, restore doesn't resurrect (P6-S6) | guest reboot ends the VMM in `fd=` mode (P6-S13 cell "Guest reboot in `fd=` mode ends the VMM"; P6-S8 withdrawn, DG-1) |
| 6 feature disabled | `transport: None` behaves as today's `network: None` (P6-S12b, attachment cut) | ← | ← | ← |

Rows 2/4/5 for G-V2/G-V4/G-V5 are seeded `overdrive-sim` through the production
owner path (the owner bodies written at the attachment cut, DG-2 rows 8, 9, 10,
12). G-V7 rows 1/2/4 at the token / setup-order level are the authored
`overdrive-init` `transport::tests` (new-step 5). Kernel effects (G-V4/G-V7)
are native Tier-3 through `run_server` + the public API (the walking skeleton
and crash matrix at the attachment cut, DG-2 rows 1-6).

## Real-kernel fixture policy (Tier-3) — P6

vsock has **no per-VM netns/veth/`NetSlot`** (D1), so the net-slot band registry
is not P6's isolation mechanism. P6's Tier-3 tests touch **node-global kernel
state** (the `/dev/vhost-vsock` CID holder table, process-global `AF_VSOCK`
listeners 1234/1240/1241/1242/1243, the owner's links, the pinned steering, the
`overdrive-mtls`/guest-prefix nft tables, the shared `local` route). Every P6
Tier-3 binary/module that touches it joins the `host-kernel-shared` nextest
group (`.config/nextest.toml`, `max-threads = 1`, matched by MODULE). The
`host-kernel-shared` assignment for the walking skeleton, crash matrix and
density bodies is made by the attachment cut's DISTILL increment, with the
bodies, for the binary they land in (whole package/binary pair, per
`.claude/rules/testing.md` § "Node-global kernel state"). The authored P6
tests need none: `overdrive-init` `transport::tests` and
`guest_flow::tests::session` are pure in-process (recording `GuestTransportOps`,
`SimGuestOwnerIo`); the `quiescence` / `shared_table` drivers run over
`SimGuestFlowIo` and `SimMtlsIntercept`, not kernel state. No VMM-binary test
exists (DG-1), and P6-S11 is source-local in `overdrive-init` (DG-5).

The crash matrix's non-mesh remote destination reached off-host uses a
hand-built egress netns drawing a **route-disjoint CIDR** via
`overdrive_testing::cidr_lease::TestCidrLease::acquire("vsock-crash-matrix")` —
never a CIDR literal (`.claude/rules/testing.md` §"Hand-built real-netns
fixtures"). The `serve`-created `ovd-veth-cli`/`ovd-veth-bk` pair is
production-owned (R5-39, P4); P6 observes it, never provisions it. The VMM boot +
CH-fork + `/dev/kvm` paths (the P1-S12 VMM cell, V-1(c); P6-S13, V-6) run through
`cargo xtask metal run --` on the native x86_64 KVM host; never pipe Lima/metal
runs through `tail`/`head`.

## Completeness audit (`nw-at-completeness-check`, 15-item)

| Item | Pass | Evidence |
|---|---|---|
| C1a empty/zero/min | ✓ | zero holders (P6-S1), zero-byte / empty datagram (P6-S13), pre-READY exit zero bytes (P6-S10) |
| C1b boundary | ✓ | recovery attempt budget N/N+1 (P6-S5); descriptor consume-once first-vs-second creation (P6-S13 cell; P6-S8 withdrawn); crash 0 ms vs 40 ms (P6-S14) |
| C2a state machine documented | ✓ | allocation / flow / control-session / quiescence-holder machines documented in module docstrings + §G-V5/§Lifecycle |
| C2b illegal-event-from-each-state | ✓ | EXEC before Active → `NotActive` (P6-S12); `activate` after teardown → `LeaseRetiring` (P6-S12); second quiesce by same holder → `QuiescenceHolderBusy` (P6-S2); restore with others → `StillQuiesced` (P6-S2) |
| C3 count 0/1/N | ✓ | holder set 0/1/2 (P6-S1/S2); density 1/64/1024/4096/16384 (P6-S15) |
| C4a idempotency apply-twice | ✓ | `converge_shared` idempotent re-run after crash (P6-S3 restart path); second quiesce by same holder a correct no-op `Busy` (P6-S2) |
| C4b inverse w/o prerequisite | ✓ | late restore after fail-stop ignored (P6-S6); `QuiescenceHold` not constructible outside the owner (type-level prevents restore-without-hold) |
| C5a mode-flag combinations | specified; owed to the cut | both-owner-death orders × 0/40 ms (P6-S14); feature-disabled `transport: None` (P6-S12b) — every body at the attachment cut |
| C5b flag orthogonality | specified; owed to the cut | quiescence latches node, not per-CID Active (P6-S1/S5 `Explicitly unaffected`); `transport: None` vs `Some` (P6-S12b) — bodies at the attachment cut |
| C6a malformed input | ✓ | malformed/all-zero launch token (P6-S11); non-vhost descriptor `ENOTTY`/`EBADF` (P6-S8 withdrawn, DG-1; its launch obligations are carried by the P1-S12 VMM cell and P6-S13) |
| C6b each declared error triggered | ✓ | `QuiescenceHolderBusy`/`StillQuiesced` (S2), `GuestTransportSetupError` variants (S11), `UnannouncedVsockEnd` (S14), fail-stop request (S5) |
| C6c closed error set | init half authored; owner half owed to the cut | init setup error set asserted exhaustive (S11, authored); the owner's closed error set (S5) is an attachment-cut body |
| C7a degraded resource | ✓ | `KernelMemory` steer retry at restore (P6-S1/S4 cross-ref D8a-CAP); recovery under ADR-0124 bound (S5) |
| C7b interruption mid-operation | ✓ | owner SIGKILL mid-transfer (S14); crash between nft batches (S3/C2); VMM kill mid-flow (S10) |
| C7c concurrent actors | ✓ | both owners killed concurrently (S14); two overlapping recoveries (S2); concurrent application half-closes + close storm (S14/S15) |

**Verdict: specification 15/15; authored 12/15, with C5a, C5b and C6c owed to
the attachment cut.** No `SPECIFICATION_AMBIGUITY` blockers: the DESIGN
pins every state machine (§Lifecycle), mode flag (D2/D19/transport:None), error
contract (§Typed error taxonomy) and env/interruption matrix (§Required boundary
scenarios + V-8/V-9). This verdict is over the specification. Over what is
authored now (P6-S11 only), the items that rest on P6-S1..S6, S10, S12, C1, C2
and S13..S15 are `AT_GAP_IN_DELIVERY_SCOPE`: their bodies are written at the
attachment cut (DG-2), and the feature-delta § *Prerequisites and
Completeness* records C5a / C5b (`AllocationSpec.transport` `None` vs `Some`)
and C6c (the owner's closed error set) as owed to those bodies. No item is
blocked on DESIGN.

## Retired by DELIVER roadmap reconciliation (not deleted here)

`tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs` is the OLD
fail-stop conformance test; its P6 replacement is the P6-S14 crash-matrix body
written at the attachment cut. It is deleted in the attachment cut with the old
mechanism it defends (DG-2 deletion discipline), not by the DISTILL planner.
