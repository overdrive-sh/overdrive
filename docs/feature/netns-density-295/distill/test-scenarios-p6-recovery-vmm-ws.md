# DISTILL scenario spec — P6: quiescence & recovery, shared-table, VMM device & fork, guest READY, walking skeleton + crash matrix

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
| R5-* | R5-4, R5-5 (quiesce half — the quiesce/restore interleaving), R5-10, R5-30, R5-32, R5-45 (the Tier-3 crash cell inside R5-10) |
| V-items | V-1(c) (fork `fd=` under production launch; open, DELIVER step 3), V-6 (walking skeleton; open, DELIVER 10-01), V-8 (fail-closed; open, DELIVER 10-02), V-9 (density & costs; open, replaces 08-04/10-05), V-28 (6.18 re-run; open, not a gate) |

R5-5 state-interleaving is split with P4: `ListenState` interleaving during
activation lives in P4 (`intake`); the quiesce/restore interleaving half lives
here (P6-S4). R5-45 half-close: codec + owner-seam are P2's; the Tier-3
crash-matrix cell is P6's (P6-S14). R5-10 Tier-3 is the V-13 matrix through
`serve` + `deploy`. R5-19's CID-claim admission half is P1's; the VMM-side
handoff / release / consume-once half is P6's (P6-S8/S10).

## Load-bearing sequencing (why some files are authored and some are gated)

Per [`shared-test-infrastructure.md`](shared-test-infrastructure.md) §0 the
replacement production surface (`SharedGuestNetworkOwner::activate`,
`quiesce_forwarding` / `restore_forwarding`, the CH-fork `fd=` backend,
`overdrive-init` guest config, the `boot_vsock_node` conformance harness, the
`quint_driver` base, the `Sim*` vsock adapters) **does not exist yet**: DELIVER
is stopped at old-model step 08-04 and the roadmap reconciliation is the first
DELIVER activity. A Rust acceptance body compiles only once the surface it
calls exists; CLAUDE.md §"Implement to the design" forbids inventing that
surface to make a body compile.

Consequence for this pass:

- **Authored now, orphaned (compile-check green):** the three crate-subdir body
  files below. Per §6 the author registers the `mod <file>;` line in the
  crate's entrypoint only when the compiling body co-arrives with the DELIVER
  production scaffold; until then the file sits unregistered under
  `tests/{integration,acceptance}/` (not a cargo target, so `cargo check
  --all-targets` never compiles it and the build stays green). The complete RED
  body is on disk against the pinned `[REF]` interface contracts, marked
  `#[ignore = "pending DELIVER step …"]`, ready for DELIVER to wire.
- **Gated (spec-only here), co-arrives at DELIVER reconciliation:** the four
  `tests/conformance/tests/vsock_*.rs` standalone binaries. Each is always a
  cargo target (no orphan state), so a compiling body would break
  `cargo check`, and it needs the shared `tests/conformance/src/vsock_harness.rs`
  + `quint_driver.rs` (shared files, not P6's to create — §3/§4) and a
  `quint-connect` dependency added to `tests/conformance/Cargo.toml` (a Cargo
  edit outside P6's file set). The no-stub discipline
  (`.claude/rules/testing.md` §"RED acceptance scaffolds") forbids a placeholder
  body. They are therefore specified in full below and surfaced as a gap.

## Files P6 owns (disjoint from the other pieces)

| File | Status this pass | Scenarios |
|---|---|---|
| `crates/overdrive-control-plane/tests/acceptance/vsock_quiescence_recovery.rs` | authored (orphaned) | P6-S1..S6, S10, S12 (owner seeded-sim) |
| `crates/overdrive-host/tests/integration/vsock_vmm_backend_probe.rs` | authored (orphaned) | P6-S7, S8, S9 (host-adapter half) |
| `crates/overdrive-init/tests/integration/vsock_guest_ready_config.rs` | authored (orphaned) | P6-S11 (unit token parse + Tier-3 guest config) |
| `tests/conformance/tests/vsock_walking_skeleton.rs` | **gated — spec only** | P6-S13 (V-6), S15 (V-8/V-9 pointers) |
| `tests/conformance/tests/vsock_crash_matrix.rs` | **gated — spec only** | P6-S14 (R5-10 / V-13 / R5-45) |
| `tests/conformance/tests/vsock_quiescence_conformance.rs` | **gated — spec only** | P6-C1 (`quiescence.qnt`) |
| `tests/conformance/tests/vsock_shared_table_conformance.rs` | **gated — spec only** | P6-C2 (`shared_table.qnt`) |

Shared files P6 depends on but must NOT edit (first-needer / DELIVER creates;
surfaced as gaps): `tests/conformance/src/vsock_harness.rs`
(`boot_vsock_node` + `ConformanceNode`), `tests/conformance/src/quint_driver.rs`
(the quint-connect base), `overdrive-sim/src/adapters/vmm.rs` (`SimVmmVsock`
extension), `tests/conformance/Cargo.toml` (`quint-connect` dep),
`overdrive-init/tests/integration.rs` (the entrypoint P6/P3 create when a
compiling body co-arrives), the `.config/nextest.toml` `host-kernel-shared`
group entries.

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
- **quint-connect conformance — example-only replay (`#[quint_test]`) +
  randomized (`#[quint_run]` seed printed):** the two modules below.

---

## G-V5 — runtime recovery reopen (quiescence + shared-table at the owner seam)

**Owner path:** `SharedGuestNetworkOwner::quiesce_forwarding(QuiescenceHolder)
-> (QuiescenceHold, ForwardingQuiescence)`, `restore_forwarding(QuiescenceHold)
-> ForwardingRestore`, `converge_shared()` as a per-component runtime repair,
`audit_shared()`; the runtime supervisor (RUN-295-B) drives one recovery per
damaged `SharedGuestNetworkComponent`, each holding its own
`QuiescenceHolder::Recovery(component)` (ADR-0169, D8a-HOLD). Driven through the
production owner with `SimSharedGuestNetworkOwner` scripting at the private
seam — no test hand-installs a production effect.

### P6-S1 — Holder-set reopen (R5-4, R5-23; Quint quiescence inv 10)

- **Contract / invariant:** forwarding is open iff the holder set is empty
  (`LatchIffHolders`); the first holder latches admission, aborts every live
  flow and closes every intake listener (each close removing its steering entry
  in the same kernel step, M-7); `restore_forwarding` removes only that holder
  and reopens — latch cleared, listeners re-bound from the last `ListenState` —
  only when no holder remains. A TCP probe on a listening port fails until
  restore; restore re-binds from the last `ListenState`.
- **Fault stimulus:** script `SimSharedGuestNetworkOwner` so the audit finds
  one component (`FlowListeners`) damaged; the supervisor quiesces under
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
- **Evidence tier:** seeded sim (layer 2) + Tier-3 (R5-4 cell inside the WS
  run). Pending marker: `pending DELIVER owner-recovery step (G-V5)`.

### P6-S2 — Two overlapping recoveries: `StillQuiesced` + `QuiescenceHolderBusy` (R5-32; Quint quiescence `WitnessNoStillQuiesced`, `HeldIsHolder`)

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
- **Evidence tier:** seeded sim (layer 2) + Quint `quiescence`. Pending marker:
  `pending DELIVER owner-recovery step (G-V5, R5-32)`.

### P6-S3 — Overdrive partial-table repair runs only inside the `IpRules` hold (R5-30; Quint shared_table inv 11 / quiescence `RepairNeverWritesPrefix`)

- **Contract / invariant:** the mTLS worker fails part-way converging the shared
  nft table, serving and quiesced; with the table in any state no connection to
  the guest prefix reaches a host service (`NoWildcardReached`, K-D3); the
  firewall recovery repairs leg-C interception and the D26 output rule **only
  while it holds quiescence** (`Recovery(IpRules)`); the `IpRules` repair writes
  neither a steering entry, the pinned steering program nor the prefix route
  (`RepairNeverWritesPrefix`); every listener serving after restore is steered.
- **Fault stimulus:** `SimMtlsIntercept` scripted partial convergence of the
  constant rules (R5-30), once while serving and once while quiesced.
- **GWT:**
  - Given the owner is serving and the shared nft table is scripted partial (an
    mTLS worker failure left some constant rules missing).
  - When the audit reports the damage and the firewall recovery quiesces under
    `Recovery(IpRules)`, repairs with `converge_shared` (that component only),
    audits and restores.
  - Then at no point in the sequence does a connection to the guest prefix reach
    a host service; the repair wrote no steering entry, no steering program and
    no prefix route; and every listener serving after restore is steered.
  - And the same holds when the table is left partial while the node is already
    quiesced by another recovery.
- **Oracle:** `{prefix_exposed_to_host_service, repair_wrote_steering_entry,
  repair_wrote_steering_program, repair_wrote_route, holder_set,
  intake_listeners_steered}`; assert `prefix_exposed… == false` at every step,
  the three `repair_wrote_*` flags `false`, repair only while
  `Recovery(IpRules)` in the holder set.
- **Evidence tier:** seeded sim (scripted partial convergence) + Quint
  `shared_table`. Pending marker: `pending DELIVER owner-recovery step (G-V5,
  R5-30)`.

### P6-S4 — Quiesce/restore interleaved with activation, `ListenState`, teardown (R5-5 quiesce half)

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
- **Evidence tier:** seeded sim (layer 2). Pending marker: `pending DELIVER
  owner-recovery step (G-V5, R5-5)`.

### P6-S5 — Recovery fail-stop; node-global objects stay; links not audited (D19-LINKS, UP-12; Quint quiescence `FailStopLeavesPrefixAsBooted`)

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
  Pending marker: `pending DELIVER owner-recovery step (G-V5, D19-LINKS)`.

### P6-S6 — G-V5 boundary table rows 2/4/5

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
- **Evidence tier:** seeded sim (layer 2). Pending marker: `pending DELIVER
  owner-recovery step (G-V5)`.

---

## G-V4 — VMM vsock device (CloudHypervisorVmm + fork)

**Owner path:** `overdrive-host::CloudHypervisorVmm` implementing
`overdrive_core::traits::vmm::Vmm`: `probe()` adds the
`backend=vhost-kernel`/`fd=` `--help` stage (`VsockBackendUnsupported`);
`create(&VmConfig, ClaimedGuestCid) -> Result<VmProcess>` launches
`--vsock cid=…,backend=vhost-kernel,fd=K` with the claimed `/dev/vhost-vsock`
descriptor handed over (D2, D16-CLAIM, ADR-0146/0170). The fork `fd=` mode
(ADR-0161) is DELIVER step 1; the production launch identity is V-1(c), DELIVER
step 3. CID-claim admission half is P1.

### P6-S7 — Startup probe refuses upstream CH (D2; G-V4 row 2)

- **Contract / invariant:** `Vmm::probe` refuses `VmmProbeError::VsockBackendUnsupported`
  when the installed binary's `--vsock` `--help` does not advertise
  `backend=vhost-kernel` with `fd=`; nothing falls back; `health.startup.refused`.
  `/dev/vhost-vsock` reachability is NOT a VMM probe (the VMM never opens it;
  that is the claim probe's, G-V0/P1).
- **Fault stimulus:** a stand-in binary whose `--help` lacks the capability
  (upstream CH); a stand-in that advertises it (the fork).
- **GWT (host integration):**
  - Given a `CloudHypervisorVmm` over a binary whose `--vsock --help` lacks
    `backend=vhost-kernel`/`fd=`.
  - When `probe()` runs.
  - Then it is `Err(VmmProbeError::VsockBackendUnsupported { binary, version })`
    naming the binary and version.
  - And given a binary that advertises the capability, `probe()` is `Ok`.
- **Oracle:** the typed `Err` variant for upstream; `Ok` for the fork; assert on
  the public `probe()` return, never internal state.
- **Evidence tier:** host integration (Tier-3, `--help` of a real binary) +
  seeded sim (`SimVmm` scripts the new `VmmProbeError` variant, feeds boot
  refusal). Pending marker: `pending DELIVER step 3 (G-V4 probe)`.

### P6-S8 — Fork `fd=` launch: handoff, consume-once, release (V-1(c), D2, D16-CLAIM; R5-19 (d)(e); G-V4 rows 1/4/5)

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
- **Evidence tier:** native metal (V-1(c), DELIVER step 3; the handoff kernel
  facts are P-39) + seeded sim (`SimVmmVsock`: pre-READY exit, a launch that
  takes the claim, R5-19 (d)(e)). Pending marker: `pending DELIVER step 3
  (V-1(c))`.

### P6-S9 — D13/D21 fork provisioning: checksum verify + capability gate (D21)

- **Contract / invariant:** provisioning downloads the fork release asset,
  verifies SHA-256 before install and refuses on mismatch, then refuses unless
  `--help` lists `--landlock` and `backend=vhost-kernel`; the submodule pointer
  equals `CLOUD_HYPERVISOR_FORK_REV` (consistency test, xtask-side, no
  `overdrive-*` dependency); `infra/metal/native-preflight.sh` checks the
  vhost-kernel capability.
- **GWT (host-adapter capability half, in P6's host file):**
  - Given a `CloudHypervisorVmm` over the provisioned fork.
  - When `probe()` runs its capability stage.
  - Then `--landlock` and `backend=vhost-kernel`/`fd=` are present → `Ok`;
    absent → `VsockBackendUnsupported`.
- **Oracle:** `probe()` return over a real provisioned binary.
- **Evidence tier:** host integration (shares P6-S7's probe body). The SHA-256
  verify + submodule-pointer consistency test are **xtask-side** (no
  `overdrive-*` dep) — specified here, owned by the DELIVER provisioning step;
  surfaced as a gap (not a P6 file). Pending marker: `pending DELIVER provisioning
  step (D21)`.

### P6-S10 — VMM death aborts flows; pre-READY exit; late registration (G-V4 rows 4/5)

- **Contract / invariant:** a VMM death (release of its vhost device) resets
  every host-side vsock connection whose peer is that CID (K-A1/K-A2′, V-10/V-22
  evidence) and aborts the CID's flows; a pre-READY VMM exit is the existing
  non-terminal start failure (lease retired and released, next placement claims
  afresh); a late registration after the alloc is torn down does not resurrect
  it; a CID clash cannot reach this gate (D16-CLAIM).
- **Fault stimulus:** `SimVmmVsock` scripts a pre-READY exit and a VMM death
  mid-flow; the owner's private seam delivers the orphaned connections.
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
  Pending marker: `pending DELIVER owner step 5 (G-V4 rows 4/5)`.

---

## G-V2 — guest command release

**Owner path:** `VmDriver::release_for_exit_emission` under
`exec.claim_release()` after `activate_guest_network` (ADR-0131); the owner's
`activate` is the only Provisioned → Active edge.

### P6-S12 — EXEC only after Active; retries; late activation after teardown (G-V2 rows 1/2/4)

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
  native WS (V-6). Pending marker: `pending DELIVER activation step (G-V2)`.

---

## G-V7 — guest network configuration before READY

**Owner path:** `overdrive-init` `configure_guest_network` before READY
(ADR-0082 §D4): `lo` up; dummy device up with `workload_addr/32`; the fwmark
rule + `net.ipv4.fwmark_reflect=1`; `ip_local_reserved_ports =
guest_reserved_ports()` and every intake bound (D25); guest programs loaded and
attached; the listener map seeded from two `sock_diag` `TCPF_LISTEN` dumps
(D23); the inbound vsock listener bound; the kernel-command-line transport
token parsed (`overdrive.transport=vsock2,addr=…,dns=…,launch=<LaunchToken>`).

### P6-S11 — Guest config completes before READY; token parse; typed setup failures (G-V7 rows 1/2/4)

- **Contract / invariant:** READY is reached only after guest network
  configuration is complete (meaning unchanged); attaching programs after READY
  would let EXEC start an application whose first `connect()` has no route
  (`ENETUNREACH`) — so configuration is strictly before READY; every typed
  `GuestTransportSetupError` variant makes `overdrive-init` exit before READY
  (existing "guest exited before READY" start failure); a late READY after a
  stop is ignored.
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
  native Tier-3 (P-19 bounded; V-6). Pending marker: `pending DELIVER guest-init
  step (G-V7)`.

---

## Walking skeleton, crash matrix, fail-closed, density (conformance — GATED, spec only)

These four `tests/conformance/tests/vsock_*.rs` files are **not created this
pass** (gated — see §"Load-bearing sequencing"). They drive the production
composition through `boot_vsock_node(cfg) -> ConformanceNode` (shared harness,
to be added at `tests/conformance/src/vsock_harness.rs`) and
`overdrive deploy`, observing only the observable surface (public API,
`alloc_status`, `ss`, `bpftool net/link/map`, `ip route`, wire capture) — never
private owner fields.

### P6-S13 — End-to-end walking skeleton (V-6; `tests/conformance/tests/vsock_walking_skeleton.rs`)

- **Contract / invariant (`@walking_skeleton`, user value):** a VM service,
  started through `overdrive serve` + `overdrive deploy`, dials a mesh peer by
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
  Pending marker: `pending DELIVER 10-01 (V-6)`.

### P6-S14 — Owner-death crash matrix: reset-never-EOF + clean-close half-close (R5-10, V-13, HALFCLOSE-SIGNAL, R5-45 Tier-3 cell; `tests/conformance/tests/vsock_crash_matrix.rs`)

This file replaces the retired OLD-model
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
  replayed here as the crash-matrix cell). Pending marker: `pending DELIVER
  step 7 (R5-10 / V-13)`.

### P6-S15 — Fail-closed composition (V-8) + density & costs (V-9)

- **V-8 (fail-closed):** owner SIGKILL, leg-F loss, control-session loss in the
  production composition; seeded sim for quiesce/restore (no link-detach
  injection — on the appliance only Overdrive could detach a held link,
  D19-LINKS). Oracle: no connection to the prefix reaches a host service at any
  point; admission stays closed while a holder holds. Pending marker: `pending
  DELIVER 10-02 (V-8)`.
- **V-9 (density & costs):** through production at 1 / 64 / 1,024 / 4,096 /
  16,384 attachments — audit and quiesce bounds (each recovery ≤ ADR-0124's
  5 s), per-flow fds/threads/CPU, boot time and kernel memory of the 65,536-cell
  host pool, guest READY latency, listen-state lag p50/p99 ≤ 2 ms, close-storm
  half-close latency (four control messages per clean close). A result outside a
  pinned bound is **surfaced to the user, never relaxed silently**. This is a
  DELIVER **measurement** obligation (classify: benchmark/stress, not an EDD
  expectation; `.claude/rules/testing.md` §"Classify external execution"), not a
  per-PR AT gate. Pending marker: `pending DELIVER density step (V-9, replaces
  08-04/10-05)`.

---

## Quint conformance drivers (GATED, spec only)

Per `.claude/rules/testing.md` §"Quint conformance": each driver drives the
production owner's state machine at its driving ports (`Sim*` at driven ports),
projects `State` from the owner's **observable** surface, references the spec
**in place** (`specs/quint/guest-flow-owner/<module>.qnt` — never copied), and
runs both trace modes: `#[quint_test]` for the named CI runs, `#[quint_run]`
randomized with the seed printed on failure. Both spawn the `quint` CLI →
gated behind `integration-tests`, run via `cargo xtask lima run --`.

### P6-C1 — `quiescence.qnt` conformance (`tests/conformance/tests/vsock_quiescence_conformance.rs`)

- **Module:** `specs/quint/guest-flow-owner/quiescence.qnt` (inv 10, D8a-HOLD).
- **Action → owner driving port mapping:** `RecQuiesce(c)` →
  `quiesce_forwarding(Recovery(c))`; `RecRepair(c)` → `converge_shared()` for
  component `c` under the hold; `RecRedo(c)` → a failed post-repair
  `audit_shared()`; `RecRestore(c)` → `restore_forwarding(hold)`; `FailStop(c)`
  → the supervisor's bounded-attempt exhaustion emitting
  `ServeShutdownRequest::SharedGuestNetwork`; `Damage`/`Crash`/`Restart`/`BringUp`
  → scripted fault / boot via the harness.
- **State projection (observable):** `{holders (holder set), latch (admission
  via gate.claim_release), rec (per-component recovery phase from supervisor
  recovery_progress), lis (intake listener presence via owner_calls / ss),
  prefix (route `local` + pinned steering present)}` — projected from the owner's
  public observation surface, never private fields.
- **Named `checks.toml` runs replayed (`#[quint_test]`):**
  `quiescence-safety-tlc`, `quiescence-live-safety-tlc`,
  `quiescence-safety-apalache` (invariant `Safety` = `LatchIffHolders`,
  `HeldIsHolder`, `NoReopenBeforeRepair`, `NoRepairWithoutHold`,
  `QuiescedClosesListeners`, `RepairNeverWritesPrefix`,
  `FailStopLeavesPrefixAsBooted`); progress runs
  `quiescence-ip-rules-episode-ends`, `quiescence-flow-listeners-episode-ends`,
  `quiescence-ip-rules-repaired`, `quiescence-ok-ip-rules-repaired`. Randomized
  `#[quint_run]` over `quiesce_ok` / `quiesce_live`.
- **Invariants each conformance path exercises (every spec invariant has ≥1
  path):** inv 10 via `LatchIffHolders` + `HeldIsHolder` +
  `NoReopenBeforeRepair` + `NoRepairWithoutHold` + `QuiescedClosesListeners`;
  the repair-never-writes-prefix clause via `RepairNeverWritesPrefix`; the
  fail-stop-keeps-prefix clause via `FailStopLeavesPrefixAsBooted`.
- **Evidence tier:** quint-connect conformance (Lima; `cargo xtask lima run --`).
  Pending marker: `pending DELIVER owner-recovery step (Quint quiescence
  conformance)`.

### P6-C2 — `shared_table.qnt` conformance (`tests/conformance/tests/vsock_shared_table_conformance.rs`)

- **Module:** `specs/quint/guest-flow-owner/shared_table.qnt` (inv 11, K-D3).
- **Action → owner driving port mapping:** `BootStep` / `RulesPartial` →
  boot through `boot_vsock_node` with a scripted partial `start_shared_owner`
  convergence; `MtlsPartial` → `SimMtlsIntercept` scripted partial convergence
  at runtime; `BringUp(p)` → the owner binds/listens/steers a wanted port;
  `RecQuiesce`/`RecRepair`/`RecRestore`/`FailStop` as in P6-C1 for the `IpRules`
  (table) and `FlowListeners` recoveries; `Crash`/`Restart` → process exit /
  re-boot (boot step 8 re-converges the table, possibly partial).
- **State projection (observable):** `{divert, reject (nft table completeness via
  overdrive-netlink read-back / the owner's shared audit facts), link (pinned
  steering link via bpftool), route (ip route), lis/entries (intake listeners +
  steering entries), holders}`.
- **Named `checks.toml` runs replayed (`#[quint_test]`):**
  `shared-table-safety-tlc`, `shared-table-live-safety-tlc`,
  `shared-table-safety-apalache` (invariant `Safety` = `NoWildcardReached`,
  `RemoteNeverDirect`, `NothingDeliveredWhileDown`, `NoReopenBeforeRepair`,
  `QuiescedClosesListeners`); progress `shared-table-repaired`,
  `shared-table-ok-repaired`. Randomized `#[quint_run]` over `table_ok` /
  `table_live`.
- **Invariants each conformance path exercises:** inv 11 (K-D3) via
  `NoWildcardReached` with the table complete, partial and absent;
  `RemoteNeverDirect` (K-L5); `NothingDeliveredWhileDown` (K-L4);
  `NoReopenBeforeRepair`; `QuiescedClosesListeners`.
- **Evidence tier:** quint-connect conformance (Lima). Pending marker: `pending
  DELIVER owner-recovery step (Quint shared_table conformance)`.

---

## 6-row boundary-scenario placement (feature-delta §"Required boundary scenarios") — P6 gates

| Row | G-V2 | G-V4 | G-V5 | G-V7 |
|---|---|---|---|---|
| 1 available | EXEC after Active (P6-S12) | VM boots with its CID (P6-S8) | reopen (P6-S1) | READY (P6-S11) |
| 2 typed failure | activation-failure projection (P6-S12) | pre-READY exit start failure (P6-S10) | fail-stop (P6-S5) | guest exits before READY (P6-S11) |
| 3 unrelated unchanged | READY unchanged (P6-S12) | other VMs unaffected (P6-S10) | leases unchanged (P6-S5) | other VMs unaffected (P6-S11) |
| 4 late success can't resurrect | no EXEC after teardown (P6-S12) | late registration after stop torn down (P6-S10) | late restore after fail-stop ignored (P6-S6) | late READY after stop ignored (P6-S11) |
| 5 disconnect/reconnect | beacon reconnect refused (cross-ref P2 ATTR-TOKEN) | VMM death → flows aborted (P6-S10) | quiesce aborts, restore doesn't resurrect (P6-S6) | guest reboot ends the VMM in `fd=` mode (P6-S8) |
| 6 feature disabled | `transport: None` sim fixture behaves as today's `network: None` (P6-S12 variant) | ← | ← | ← |

Rows 2/4/5 for G-V2/G-V4/G-V5 are seeded `overdrive-sim` through the production
owner path (the authored owner + host files). Kernel effects (G-V4/G-V7) are
native Tier-3 through `serve` + `deploy` (the gated WS/crash-matrix files).

## Real-kernel fixture policy (Tier-3) — P6

vsock has **no per-VM netns/veth/`NetSlot`** (D1), so the net-slot band registry
is not P6's isolation mechanism. P6's Tier-3 tests touch **node-global kernel
state** (the `/dev/vhost-vsock` CID holder table, process-global `AF_VSOCK`
listeners 1234/1240/1241/1242/1243, the owner's links, the pinned steering, the
`overdrive-mtls`/guest-prefix nft tables, the shared `local` route). Every P6
Tier-3 binary/module that touches it joins the `host-kernel-shared` nextest
group (`.config/nextest.toml`, `max-threads = 1`, matched by MODULE). The
`host-kernel-shared` entries P6 adds when the compiling bodies co-arrive (gap —
`.config/nextest.toml` is not a P6 file):

- `overdrive-system-conformance::vsock_walking_skeleton`
- `overdrive-system-conformance::vsock_crash_matrix`
- `overdrive-system-conformance::vsock_quiescence_conformance`
- `overdrive-system-conformance::vsock_shared_table_conformance`
- `overdrive-host::integration::vsock_vmm_backend_probe` (kvm / native metal)
- `overdrive-init::integration::vsock_guest_ready_config` (guest Tier-3)

The crash matrix's non-mesh remote destination reached off-host uses a
hand-built egress netns drawing a **route-disjoint CIDR** via
`overdrive_testing::cidr_lease::TestCidrLease::acquire("vsock-crash-matrix")` —
never a CIDR literal (`.claude/rules/testing.md` §"Hand-built real-netns
fixtures"). The `serve`-created `ovd-veth-cli`/`ovd-veth-bk` pair is
production-owned (R5-39, P4); P6 observes it, never provisions it. The VMM boot +
CH-fork + `/dev/kvm` paths (P6-S8 V-1(c), P6-S13 V-6) run through
`cargo xtask metal run --` on the native x86_64 KVM host; never pipe Lima/metal
runs through `tail`/`head`.

## Completeness audit (`nw-at-completeness-check`, 15-item)

| Item | Pass | Evidence |
|---|---|---|
| C1a empty/zero/min | ✓ | zero holders (P6-S1), zero-byte / empty datagram (P6-S13), pre-READY exit zero bytes (P6-S10) |
| C1b boundary | ✓ | recovery attempt budget N/N+1 (P6-S5); descriptor consume-once first-vs-second creation (P6-S8); crash 0 ms vs 40 ms (P6-S14) |
| C2a state machine documented | ✓ | allocation / flow / control-session / quiescence-holder machines documented in module docstrings + §G-V5/§Lifecycle |
| C2b illegal-event-from-each-state | ✓ | EXEC before Active → `NotActive` (P6-S12); `activate` after teardown → `LeaseRetiring` (P6-S12); second quiesce by same holder → `QuiescenceHolderBusy` (P6-S2); restore with others → `StillQuiesced` (P6-S2) |
| C3 count 0/1/N | ✓ | holder set 0/1/2 (P6-S1/S2); density 1/64/1024/4096/16384 (P6-S15) |
| C4a idempotency apply-twice | ✓ | `converge_shared` idempotent re-run after crash (P6-S3 restart path); second quiesce by same holder a correct no-op `Busy` (P6-S2) |
| C4b inverse w/o prerequisite | ✓ | late restore after fail-stop ignored (P6-S6); `QuiescenceHold` not constructible outside the owner (type-level prevents restore-without-hold) |
| C5a mode-flag combinations | ✓ | both-owner-death orders × 0/40 ms (P6-S14); feature-disabled `transport: None` (P6-S12) |
| C5b flag orthogonality | ✓ | quiescence latches node, not per-CID Active (P6-S1/S5 `Explicitly unaffected`) |
| C6a malformed input | ✓ | malformed/all-zero launch token (P6-S11); non-vhost descriptor `ENOTTY`/`EBADF` (P6-S8); upstream CH `--help` (P6-S7) |
| C6b each declared error triggered | ✓ | `VsockBackendUnsupported` (S7), `QuiescenceHolderBusy`/`StillQuiesced` (S2), `GuestTransportSetupError` variants (S11), `UnannouncedVsockEnd` (S14), fail-stop request (S5) |
| C6c closed error set | ✓ | owner quiescence error set + init setup error set asserted exhaustive (S5/S11) |
| C7a degraded resource | ✓ | `KernelMemory` steer retry at restore (P6-S1/S4 cross-ref D8a-CAP); recovery under ADR-0124 bound (S5) |
| C7b interruption mid-operation | ✓ | owner SIGKILL mid-transfer (S14); crash between nft batches (S3/C2); VMM kill mid-flow (S10) |
| C7c concurrent actors | ✓ | both owners killed concurrently (S14); two overlapping recoveries (S2); concurrent application half-closes + close storm (S14/S15) |

**Verdict: COMPLETE (15/15).** No `SPECIFICATION_AMBIGUITY` blockers: the DESIGN
pins every state machine (§Lifecycle), mode flag (D2/D19/transport:None), error
contract (§Typed error taxonomy) and env/interruption matrix (§Required boundary
scenarios + V-8/V-9). Gaps below are `AT_GAP_IN_DELIVERY_SCOPE` (shared-harness
and Cargo wiring co-arriving in DELIVER), not spec ambiguity.

## Retired by DELIVER roadmap reconciliation (not deleted here)

`tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs` is the OLD
fail-stop conformance test; its P6 replacement is `vsock_crash_matrix.rs`. It is
retired in DELIVER coupled with the old production-code removal (shared-infra
§7), not deleted by the DISTILL planner.
