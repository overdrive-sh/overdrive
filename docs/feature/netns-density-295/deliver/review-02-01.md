# DELIVER Review — netns-density-295 / 02-01

## Metadata

- Reviewer: isolated software-crafter reviewer
- Iteration: 1
- Baseline: cc752bcf0cd83707c06071dcc88b2b0bbfd88b37
- Cumulative commits: c60cdd3b6d80607a218d281c414c2bed83074a24 and 7ec987a815ad5c9d68f3247d33ffb4ccd6adf481
- Scope: complete cumulative diff, all 51 changed files, and affected production caller/owner paths for step 02-01 only
- Authority: feature-delta.md A2, B1, C-295-A/B/G, ERR-295-A, D-295-DISTILL-4/5; architecture brief and ADR-0114/0118/0122; DISTILL scenarios/red classification; roadmap step 02-01

## Summary

The grouped AllocationSpec.network and VmNetworkAttachment shapes, opaque
GuestNetworkPlan accessors, and exact two-method provisioner/five-method
shared-owner traits are present. The private scratch algorithm tests exercise
ordering, source preservation, cleanup continuation, and honest complements.
The pool tests exercise replay, release, reuse, and exhaustion.

CHANGES_REQUIRED: the cumulative implementation does not deliver the accepted
production owner. Real scratch I/O is unconditional no-op, host
provision/teardown, audit, and TAP quiescence are no-op, bridge guard is an
in-process map rather than a kernel adapter, and the TCX adapter remains a RED
panic scaffold. Ordinary production dispatch still selects the old
netns/veth/NetSlot owner while the injected test seam selects the new owner.
Boot calls stale sweep before VM reclamation and sweeps only TAP names.
Required S-ND295 bodies are absent or replaced by weaker evidence.

## Evidence and verification

- Both commits retain Marcus as author and contain exactly one
  Co-Authored-By: Codex <codex@openai.com> and one Step-Id: 02-01 trailer.
- DES has RED/PASS, GREEN/PASS, COMMIT/PASS for c60cdd3b, then only
  COMMIT/PASS for 7ec987a8. The second commit has no separately logged RED or
  GREEN event.
- git diff --check and cargo fmt --all -- --check pass.
- The required local command
  PROPTEST_CASES=1024 cargo test -p overdrive-control-plane --lib guest_network
  cannot build on this macOS host because Linux-only linux-keyutils references
  unavailable macOS libc symbols (SYS_add_key, __errno_location, and keyring
  errno constants). The crafter reports Lima acceptance/check/clippy/format
  gates green.
- Native-metal evidence was unavailable because OVERDRIVE_METAL_TARGET is
  unset; native bpffs was unavailable in Lima due permissions. Those
  environment gaps do not excuse the source-level no-op implementation.
- No mutation testing was run.

## Design and API comparison

### Matches

- GuestNetworkAssignment is exactly the six-field derived transient value in
  overdrive-core, and AllocationSpec has exactly network: Option<...> with no
  serde/rkyv derive (driver.rs:283-348).
- VmNetworkAttachment has exactly tap and mac and CloudHypervisorVmm renders
  direct --net tap=...,mac=... without ip netns exec (vm/config.rs:885-892;
  vmm.rs:246-305).
- GuestNetworkPlan has private fields and exactly alloc, bridge, node_prefix,
  and assignment accessors (guest_network.rs:30-61). The provisioner and
  shared-owner traits have the accepted async method sets
  (guest_network.rs:63-96).
- One host owner is used; HostGuestNetworkProvisioner is an alias, not a
  second object (guest_network.rs:674-681).
- The private scratch tests drive the production owner algorithm through the
  private effect seam; the public sim owner is not used for scratch proof.
- Pool PBT compares detached complete snapshots and checks replay, idempotent
  release, smallest-free model agreement, reserved addresses, and typed
  exhaustion (guest_network.rs:1562-1660).

### Contract Shape Compliance

- New scratch/pool and active acceptance/integration tests carry
  CONTRACT_SHAPE declarations. The pool PBT asserts the complete allocation
  map, and scratch tests assert full call/complement surfaces.
- The transitioned grouped-handoff test still declares deleted fields at
  action_shim/mod.rs:1348-1349 (netns, host_veth, workload_addr, guest_tap,
  guest_mac, guest_gateway, guest_prefix_len, guest_dns), although its live
  delta is only spec.network at :1365-1379. This is not an exact declaration.
- Active netns_density_guest_network and shared_guest_network_startup acceptance
  bodies have no Outcome anchor: DISCUSS Elevator Pitch declaration, failing
  the reviewer mechanical acceptance check. No test name matches the banned
  technical-result regex.

## Test honesty and TDD assessment

- No changed assertion was found weakened, deleted, skipped, or relaxed to make
  production pass. Grouped-field edits preserve the old six guest facts.
  G9 therefore passes for the inspected diff.
- Private scratch tests are legitimate owner tests: production algorithm is the
  SUT and only the accepted private leaf-effect adapter is scripted.
- The production_host_owner_boots_only_after_real_shared_identity_is_exact test
  is not an honest real bridge-guard integration proof: its assertion at
  shared_guest_network_startup.rs:289-293 calls process-local nft bridge
  observe, whose state is BRIDGE_GUARDS at nft.rs:1893-1897, not a kernel dump.
- There are 16 active feature-related tests (9 private pool/scratch, 3
  control-plane acceptance, 4 startup integration). Table-driven loops
  consolidate variations; no standalone test-budget excess was established.
- The second pool-fix commit has no RED/GREEN evidence. No G9 violation was
  found, so this is a process-evidence gap, not a silent test-modification
  finding.

## Findings

### F-01 — Blocking: production shared owner is a no-op/fake effect owner

Reachability is concrete: run_server constructs HostSharedGuestNetworkOwner at
lib.rs:2230-2236, and run_server_with_obs_and_drivers calls probe_startup,
sweep_stale, and converge_shared at lib.rs:2538-2574. RealSharedGuestNetwork-
ScratchIo returns Ok, Ok(true), and zero for every leaf action/query at
guest_network.rs:616-668. Host provision/teardown return Ok at
guest_network.rs:1052-1059; audit and quiesce return Ok at :1152-1157.
The canonical TCX adapter still panics at overdrive-dataplane/src/guest_tcx.rs:
110-154. converge_shared records guard state in an in-process BRIDGE_GUARDS
map (overdrive-netlink/src/nft.rs:1893-1897, :2243-2263) and creates ordinary
files at bpffs paths (guest_network.rs:1125-1149), not maps/pins.

The node can therefore pass startup without a classifier, endpoint map,
bridge guard, or cleanup proof, and allocation provision/teardown has no
effect. This directly fails the step's real HostGuestNetworkProvisioner,
TCX provision effects, and S-ND295-11..13 contracts.

Bounded remediation: implement the accepted real effects through existing
netlink/dataplane adapter ownership and complete the named read-backs. Do not
add a second owner or public fault API. If an already-approved later adapter
dependency makes this impossible without an architectural choice, surface a
DESIGN gap rather than advancing with a fake owner.

Disposition: CHANGES_REQUIRED.

### F-02 — Blocking: deleted netns/veth/NetSlot path and dual owner seam remain

run_convergence_tick passes None,None at reconciler_runtime.rs:1352-1362;
the normal branch invokes dispatch_with_workflow_intent at :1609-1615, whose
production dispatch supplies HostNetworkProvisioner at action_shim/mod.rs:
850-882. provision_and_inject_netns receives guest_provisioner=None and enters
the old NetSlot/WorkloadNetns branch at :1267-1297. The new guest owner is
selected only by the test-gated helper at :1215-1245.

AppState still publicly owns net_slot_allocator (lib.rs:373-386), boot still
calls veth_provisioner::adopt_on_restart_recovery (lib.rs:3300-3322), and
ShimError still exposes WorkloadNetnsProvision and NetSlotExhausted
(action_shim/mod.rs:3220-3237). VMM direct launch now ignores namespaces while
the old provisioner moves the TAP into a per-workload namespace
(veth_provisioner.rs:2175-2184). This is a reachable production dual path,
not a hypothetical cancellation state, and contradicts the accepted deletion
of NetSlot, netns, veth, setns launch, and NetSlot recovery.

Bounded remediation: remove the legacy production branch, allocator, adoption
path, and obsolete shim variants within the accepted cut; route ordinary and
test-gated dispatch through the one shared owner. Do not retain an optional
compatibility branch or invent a second adapter.

Disposition: CHANGES_REQUIRED.

### F-03 — Blocking: boot sweep ordering and stale-residue scope are wrong

run_server_with_obs_and_drivers invokes shared_guest_network.sweep_stale at
lib.rs:2555-2564 before vm_reclamation_boot::converge at :3267-3275. The
accepted G-295-1 order is VM reclamation, then stale TAP/map/pin/guard sweep,
then convergence/admission. sweep_stale itself only lists /sys/class/net and
deletes ovd-tp-* links (guest_network.rs:1078-1095); it does not sweep maps,
TCX links/pins, guard members/rules, or prove a complete zero complement.
The later legacy netns adoption is not the shared-switch sweep.

Bounded remediation: move owner-driven sweep after existing VM reclamation and
implement accepted TAP/map/pin/guard inventory/deletion/read-back. Do not add
a persistence store or surviving-VMM adoption mechanism.

Disposition: CHANGES_REQUIRED.

### F-04 — Blocking: required S-ND295 acceptance bodies are absent, pending, or weaker substitutions

The required active set is S-ND295-00, 02..07, and 10..13.

- S00 private scratch/composed refusal bodies are active, subject to F-01.
- S02..04 grouped/pool tests are active but do not prove production dispatch.
- S05 remains ignored at overdrive-core/tests/acceptance/netns_density_placement_cap.rs:39.
- S06..07 are active through the injected action-owner seam
  (netns_density_guest_network.rs:154-262).
- No active real deliberate-link-loss/bridge-guard body exists for S10;
  DetachedLinkGuard is only a scripted private exercise.
- No active real-kernel provision/read-back or effect-first teardown body exists
  for S11..12.
- S13 is active only as a stale-TAP test; it creates no VMM and proves neither
  reclamation-before-sweep nor a complete map/pin/guard complement.
- S08/09 and later remain pending as required (for example
  guest_tcx_classifier_test_run.rs:200 remains ignored).

The active production-host test also observes the process-local fake bridge
state, so it cannot supply the missing kernel evidence. The authored
scenario bodies must remain the evidence layer; these substitutes cannot.

Bounded remediation: activate the existing authored S05 and S10..13 bodies at
their designated real boundaries without re-authoring scenarios or moving
S08/09 forward. If an authored body is missing from the repository, surface
that source/design gap rather than creating a substitute.

Disposition: CHANGES_REQUIRED.

### F-05 — Blocking: transitioned Contract Shape metadata is stale

action_shim/mod.rs:1348-1349 names the deleted network fields while the live
test changes only grouped spec.network at :1365-1379. Active acceptance bodies
also omit the required Outcome anchor: DISCUSS Elevator Pitch declaration.
Update only metadata to the accepted grouped delta and add the anchors; preserve
authored assertions and scenario language.

Disposition: CHANGES_REQUIRED.

### F-06 — Blocking: GuestAddressPool does not keep release atomic under one mutex

The accepted pool contract says assign, release, and snapshot complete under
one parking_lot::Mutex critical section. The implementation adds a second
AtomicU32 next_free field (`guest_network.rs:367-377`). release removes the
plan while holding held, drops that guard, and only then updates next_free at
`guest_network.rs:460-464`. The operation therefore has two independently
observable state updates rather than the one critical section named by C-295-E.
The pool PBT does not exercise this atomicity boundary; it is sequential and
compares only after each completed operation.

Bounded remediation: keep the accepted three-operation private pool surface,
but make the held allocation map the sole mutex-protected source for choosing
the smallest free non-reserved address and for release. Do not add a public
allocator operation or a second owner.

Disposition: CHANGES_REQUIRED.

### F-07 — Medium: second production fix has incomplete DES/TDD evidence

execution-log.json records a full RED/GREEN/COMMIT for c60cdd3b and only
COMMIT/PASS for 7ec987a8. The second commit changes production pool behavior and
fixture construction without a separately logged RED/GREEN result. No
assertion weakening was found. Record the pool fix as a post-COMMIT correction,
not a complete independent TDD cycle; no mutation-testing or architecture
requirement is inferred.

Disposition: evidence gap; existing blockers remain.

## File-scope assessment

The 51-file expansion is mostly compiler/API fallout from deleting eight
AllocationSpec fields and updating neutral literals/projections. Those edits
are tightly related. However, overdrive-netlink/src/nft.rs contains 290 added/
66 deleted behavior lines implementing a process-local fake bridge adapter, and
overdrive-netlink/src/client.rs adds host operations while the accepted real D9
bridge codec belongs to later step 02-02; these are not compiler-required
fallout. action_shim and reconciler_runtime are necessary for owner seams, but
their legacy production branch means the resulting seam is not the identical
production workflow claimed by the criterion.

## Verification gates

| Gate | Result | Evidence |
| --- | --- | --- |
| G1 one active acceptance body per lane | FAIL | S05 remains ignored; S10..12 lack active real bodies. |
| G2 valid RED failure | PASS (reported) | Main step has RED/PASS; second fix lacks RED. |
| G3 assertion-failure RED | PASS (reported) | Crafter reports valid RED; local Linux build unavailable. |
| G4 no mocks inside owner algorithm | PASS | Private leaf seam is accepted; public sim is not used for scratch proof. |
| G5 business/contract language | FAIL | Stale delta and missing outcome anchors (F-05). |
| G6 all green | Environment-limited | Crafter reports Lima green; local macOS cannot build Linux dependency. |
| G7 100% before commit | Reported PASS | Crafter/DES report. |
| G8 test budget | PASS/no excess found | Table-driven/PBT tests consolidate variations. |
| G9 no test modification | PASS | No weakened/deleted/skipped assertion identified. |

## Verdict

# CHANGES_REQUIRED

F-01 through F-06 are unresolved blocking findings. The original crafter must
remediate step 02-01 within the accepted architecture, re-run designated
production-owner and acceptance evidence, and obtain a fresh review iteration
before any later roadmap step starts. F-07 remains an explicit DES evidence
disposition; it does not authorize scope expansion or mutation testing.

## Iteration 2 — remediation and upstream-disposition audit

### Scope and state

No production or test code changed after iteration 1. No remediation commit or
new DES event exists. The repository still ends at 7ec987a8, and the only
reviewer-authorized write remains this review artifact.

### F-01 — confirmed accepted DESIGN/DISTILL dependency blocker

The blocker is confirmed against the approved roadmap and the feature delta.
Step 02-01 explicitly requires the real provisioner's TCX attach/pin/query
effects and S-ND295-11..13 real-kernel evidence, while step 02-02 exclusively
owns the aya TCX adapter, classifier, typed query/detach/endpoint/counter
operations, and D-295-DISTILL-9 bridge-family codec. The current
overdrive-dataplane operations remain the exact RED panics at lines 110-154;
they are not an implementation gap that 02-01 may close by inventing another
adapter or importing 02-02 behavior early.

Disposition: F-01 remains BLOCKING, but it is returned as an upstream DESIGN
roadmap/dependency reconciliation gap (with the corresponding DISTILL
evidence dependency), not as a remediation instruction to the 02-01 crafter.
An independent DESIGN review must reconcile the accepted step boundary before
DELIVER resumes. No implementation review iteration may evolve this into a
new persistence, adapter, or ownership mechanism.

### F-02 — remains an implementation finding, paused behind F-01

The ordinary production caller path still selects the legacy NetSlot/netns
branch and the exact old public ShimError variants remain. This is a concrete
structural/API divergence, not a timing hypothesis, and remains within the
accepted single-cut architecture. It is therefore still CHANGES_REQUIRED for
the original crafter after the upstream dependency is resolved. No code was
requested or written in this iteration.

### F-03 — not promoted to a remediation finding

The source ordering observed in iteration 1 still differs from the accepted
prose order, but the claimed control-plane defect depends on an ordering
schedule across reclamation and sweep. Repository inventory found no seeded
overdrive-sim safety/liveness/convergence invariant for that schedule. Under
the repository rule, a static suspicion cannot become a remediation/design
requirement without that failing invariant and its printed seed.

Disposition: retain F-03 as an unproven hypothesis/evidence question only;
no reorder, new Sim seam, or architecture change is authorized. If the
upstream DESIGN/DISTILL remediation chooses to make this ordering executable,
it must first add the approved invariant boundary and reproduce the failure.

### F-04 — confirmed missing DISTILL/acceptance evidence

The test inventory confirms that no authored active Rust body exists for
S-ND295-10, S-ND295-11, or S-ND295-12. The only source matches are the prose
scenarios in distill/test-scenarios.md and the weaker private/composed tests
listed in iteration 1. S-ND295-05 has an authored body, but it remains
reasoned-ignored in netns_density_placement_cap.rs for the later 03-01
placement-cap step. S-ND295-13 has an active stale-TAP body, but it does not
contain the authored VMM/reclamation/full-complement contract.

Disposition: F-04 remains BLOCKING as an upstream DISTILL/acceptance-designer
gap (and a roadmap activation reconciliation for S-ND295-05). The 02-01
crafter is not authorized to author replacement acceptance bodies, weaken the
prose scenarios, or turn private scripts into host evidence. The upstream wave
must restore the authored bodies and independently approve their boundary
before DELIVER can activate them.

### F-05 and F-06 — remain bounded implementation findings

The stale Contract Shape metadata and missing acceptance outcome-anchor
metadata remain mechanically present; no metadata remediation was made. The
pool still uses AtomicU32 next_free outside the held-map mutex, so the exact
one-critical-section contract remains unfulfilled. Both findings remain
CHANGES_REQUIRED for the original crafter once the upstream DESIGN/DISTILL
blockers are cleared. They do not justify new public API or architecture.

### F-07 — unchanged process evidence disposition

The second commit still has only COMMIT/PASS in execution-log.json. No new
RED/GREEN evidence was created. This remains a documented post-COMMIT
correction/evidence gap and is not converted into a new process requirement.

## Iteration 2 verdict

# CHANGES_REQUIRED

The accepted DESIGN/DISTILL dependency gap (F-01) and missing acceptance
evidence gap (F-04) prevent honest 02-01 remediation or advancement. F-02,
F-05, and F-06 remain bounded implementation findings; F-03 is explicitly
unproven and receives no remediation prescription. Required upstream waves are
DESIGN for the 02-01/02-02 ownership/dependency reconciliation and DISTILL /
acceptance design for the missing S-ND295-10..12 bodies and step-activation
mapping. No later DELIVER step may start, and the final verdict remains
CHANGES_REQUIRED.

## Iteration 3 — replacement implementation review

### Metadata and scope

- Reviewer: fresh isolated replacement software-crafter reviewer
- Feature / step: `netns-density-295` / `02-01`
- Baseline: `cc752bcf0cd83707c06071dcc88b2b0bbfd88b37`
- Cumulative implementation under review: `c60cdd3b`, `7ec987a8`, approved
  upstream checkpoint `c13bcc87`, and replacement commit `6fdae90d`
- Replacement commit scope: 11 files, 1,686 insertions, 241 deletions
- Review scope: the complete cumulative step diff, real production caller and
  owner paths, the accepted feature-delta/roadmap/DISTILL contracts, and the
  replacement DES cycle
- Exclusions: step `02-02`, step `02-03`, mutation testing, and all eight
  explicitly user-waived real-I/O test-side panic placeholders

The replacement commit retains Marcus as author and has exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer and one `Step-Id: 02-01`
trailer. The fresh DES events are RED/PASS at `23:48:36Z`, GREEN/PASS at
`00:14:18Z`, and COMMIT/PASS at `00:25:42Z`; historical events remain
unchanged.

### Executive result

`CHANGES_REQUIRED`. The grouped handoff, pool mutex correction, boot reorder,
Sim sweep observation, source-local owner algorithm, and semantic projections
are present. The committed implementation still does not deliver the accepted
production slice: the live startup probe uses an all-success/zero-count fake
adapter, the D12A host leaf is still an unconditional RED panic, the D9 bridge
guard is process-local, the legacy NetSlot/netns/veth owner remains reachable,
and the D12 lifecycle receipts are not populated or observed source-honestly.
The required non-waived owner/S13/BPF green evidence is unavailable, while the
DES GREEN/PASS event claims completion. Formatting also fails locally.

### Design and API comparison

#### Conforming portions

- `AllocationSpec` now has exactly `network: Option<GuestNetworkAssignment>`;
  the grouped assignment has the six accepted fields and no schema derives,
  generation, or listener-port field (`crates/overdrive-core/src/traits/driver.rs:283-347`).
- `VmNetworkAttachment` is exactly `{ tap, mac }`, and Cloud Hypervisor renders
  the direct `tap=...,mac=...` argument (`crates/overdrive-core/src/vm/config.rs:885-900`;
  `crates/overdrive-host/src/vmm.rs:246-305`).
- `GuestAddressPool` uses one mutex-protected held map for assign/release/
  snapshot, chooses the smallest free address, and has no former
  `AtomicU32 next_free` (`crates/overdrive-control-plane/src/guest_network.rs:376-461`).
- The accepted `GuestNetworkProvisioner`, `SharedGuestNetworkOwner`,
  `GuestNetworkPlan`, and D12 semantic declarations have the approved
  visibility and method signatures. No new public product command, persistence
  mechanism, recovery store, generic command port, or second owner was found.
- The BPF map shape is the accepted 65,536-entry endpoint hash and eight-slot
  counter array (`crates/overdrive-bpf/src/maps/guest_tcx.rs:9-25`), and the
  classifier has the eight named counter slots and TCX first-order attachment
  call path in the approved surface.
- D13's same-host Sim sweep observation is wired through the actual sweep port
  (`crates/overdrive-sim/src/adapters/guest_network.rs:208-221`), and the
  production source order now invokes VM reclamation before `sweep_stale`
  (`crates/overdrive-control-plane/src/lib.rs:3256-3310`).

#### Contract divergences

- The exact D12 method signatures exist, but the implementation does not carry
  the private receipts that the contract requires. `GuestTcxProgram::load`
  merely looks up the classifier and defaults its program ID to zero
  (`crates/overdrive-dataplane/src/guest_tcx.rs:664-682`); no classifier load or
  classifier/map identity receipt occurs. `pin_*`, endpoint insertion, attach,
  and link pin cannot populate the `GuestTcxInventoryReceipts` fields, which
  are only populated directly by source-local tests (`guest_tcx.rs:625-635,
  1669-1677`).
- The private D12 source violates non-mutating observation: when the exact link
  pin is observed, `AyaGuestTcxInventorySource::observe_pin` calls
  `PinnedLink::unpin()` (`crates/overdrive-dataplane/src/guest_tcx.rs:469-505`).
  Link observations also hard-code `program_id = 0`, and the endpoint-entry
  fallback probes `map.id` as an ifindex (`guest_tcx.rs:423-445, 879-887`)
  instead of using the required exact private map/entry identity boundary.
- `GuestTcxAdoptedState`, `map_by_id`, and the eight inventory observers have
  no real production caller because the real scratch/allocation adapters remain
  scaffolds. This is not an API-shape failure; it is an incomplete production
  binding of the accepted API.

### Prior-finding dispositions

| Prior finding | Replacement disposition |
|---|---|
| F-01 no-op/fake production owner | **Remains blocking**, now evidenced by the live `RealSharedGuestNetworkScratchIo`, D12A RED leaf, and fake D9 bridge adapter below. The upstream design/dependency blocker is closed; this is an implementation failure. |
| F-02 legacy path / dual owner | **Remains blocking.** `HostNetworkProvisioner`, `NetSlotAllocator`, netns adoption, and the optional old branch remain reachable. |
| F-03 sweep order/scope | **Order portion resolved in source** and now has the approved Sim invariant boundary; complete residue sweep, audit/quiescence, and post-reorder GREEN evidence remain blocking. |
| F-04 missing real-I/O authored bodies | **User-waived and not scored.** The eight unconditional test-side panic placeholders are not approval conditions. This waiver does not waive production no-ops or unrun non-waived bodies. |
| F-05 Contract Shape metadata | **Remains blocking.** The transitioned test declaration still names deleted fields and the active acceptance body set has no required outcome-anchor declaration. |
| F-06 pool atomicity | **Resolved.** The current pool has one held-map mutex and no second cursor state. |
| F-07 prior post-COMMIT evidence | **Partially superseded.** The replacement has an independently logged RED/GREEN/COMMIT cycle, but the GREEN/PASS claim is not substantiated because required tests were not completed. |

### New findings

#### F-08 — Blocking: production startup and allocation effects are still no-op/RED/fake

The ordinary production constructor installs `RealSharedGuestNetworkScratchIo`
(`crates/overdrive-control-plane/src/guest_network.rs:920-930`). Its netlink and
TCX actions return `Ok(())`, exercise always returns `true`, and every resource
count returns zero (`guest_network.rs:608-660`). `run_server_with_obs_and_drivers`
calls this probe before admission (`crates/overdrive-control-plane/src/lib.rs:2560`),
so the production server can pass startup without creating or exercising a
bridge, TAP, guard, endpoint map, classifier, or pin.

The ordinary allocation owner is not real either. Every
`HostGuestNetworkAllocationIo` method expands to an unconditional RED panic
(`guest_network.rs:766-891`), while `HostSharedGuestNetworkOwner::new` installs
that leaf as the production adapter (`guest_network.rs:920-930`). The only
passing owner tests inject the private scripted leaf. This is not one of the
waived test-side placeholders: it is the production constructor and its
production call path.

The bridge guard is also a process-local map. `overdrive-netlink::nft::bridge`
stores only `BRIDGE_GUARDS` in a `OnceLock<Mutex<...>>` and synthesizes rules,
handles, and inventory (`crates/overdrive-netlink/src/nft.rs:1893-1954`;
`nft.rs:2243-2349`). The family codec still contains an explicit RED panic at
`nft.rs:95-106`. Consequently, `converge_shared` records a fake guard and only
the bridge link/MAC read-back (`guest_network.rs:1970-2066`); it cannot establish
the accepted real kernel guard or TCX complement.

Required bounded remediation: complete the already-approved D12/D12A/D9
adapter bindings through the existing netlink/dataplane owners and make the
startup probe, allocation provision/teardown, audit, and quiescence use those
effects. Preserve the exact accepted public surface and one concrete owner; do
not add a second adapter, public fault hook, persistence system, or compatibility
mechanism.

#### F-09 — Blocking: the deleted NetSlot/netns/veth path and a compatibility branch remain reachable

`AppState` still owns the public `net_slot_allocator`
(`crates/overdrive-control-plane/src/lib.rs:378-399`), boot still calls
`veth_provisioner::adopt_on_restart_recovery`
(`lib.rs:3342-3348`), and the action shim still defines and passes
`HostNetworkProvisioner` (`crates/overdrive-control-plane/src/action_shim/mod.rs:736-780`).
The ordinary owner dispatcher explicitly retains an old fallback when
`state.shared_guest_network` is `None` (`action_shim/mod.rs:1143-1172`) and,
even when the new owner is present, passes `&HostNetworkProvisioner` alongside
`Some(shared_guest_network)` (`action_shim/mod.rs:1174-1193`). The old
`ShimError` variants and legacy netns/veth implementation remain in the same
production module.

This is a structural reachability defect, not a hypothetical schedule: normal
`run_server` composition sets the optional field, and the same ordinary
dispatch function retains both owners and old teardown branches. It contradicts
the accepted single-cut requirement that the production HostNetworkProvisioner,
NetSlot/netns/veth branch, adoption path, and optional compatibility path be
absent.

Required bounded remediation: route ordinary handler/reconciler start/restart/
stop through the one shared owner and delete the legacy production allocator,
adoption, branch, and obsolete error surface required by the accepted cut.
Keep only explicitly allowed neutral compiler fallout; do not preserve a
compatibility path.

#### F-10 — Blocking: D12 lifecycle/inventory is not source-honest or production-wired

The accepted D12 contract requires real receipts, classifier loading, exact
map/program/link identity, non-mutating pin observation, and all eight
post-cleanup family observations. The implementation fails those requirements:

- `GuestTcxProgram::load` does not call the classifier's load operation and
  initializes `program_id` from an unpopulated receipt, defaulting to zero
  (`crates/overdrive-dataplane/src/guest_tcx.rs:664-682`).
- `loaded_links` discards the program identity and hard-codes it to zero
  (`guest_tcx.rs:423-445`); no live lifecycle method records the map/program/
  link/pin receipts consumed by the observers.
- Observing a pinned link mutates the kernel by unpinning it
  (`guest_tcx.rs:482-495`).
- The exact `map_by_id` source method has no production caller, and
  `GuestTcxAdoptedState::{for_inventory,adopt_*}` has no production caller
  (`rg` over `crates/**/*.rs` found only declarations and source-local tests).

The production call path reaches `capture`, `load`, and map pinning from
`HostSharedGuestNetworkOwner::converge_shared`
(`crates/overdrive-control-plane/src/guest_network.rs:2052-2065`), so this is
not an unreachable helper concern. It will either fail to attach, report fake
identity, or mutate a pin during observation once the waived allocation leaf
is replaced.

Required bounded remediation: implement the exact approved receipt and
observation semantics inside dataplane, wire the existing owner actions to
those methods, and retain source-honest error families. Do not add a lifecycle
trait, generic command method, raw aya type, numeric ABI, or new public API.

#### F-11 — Blocking: stale sweep, audit, and quiescence do not cover the accepted complement

The source order was changed, but `sweep_stale` still enumerates only
`/sys/class/net` and deletes names beginning `ovd-tp-`
(`crates/overdrive-control-plane/src/guest_network.rs:1951-1968`). It does not
inventory or remove stale endpoint entries, loaded programs/maps, TCX links,
bpffs pins, guard members, rules, or sets. `audit_shared` and
`quiesce_managed_taps` are unconditional `Ok(())`
(`guest_network.rs:2068-2073`). The accepted boot and security contracts
require complete zero complements and a real non-repairing audit/quiescence
boundary.

Required bounded remediation: extend the existing owner algorithm to sweep and
read back the accepted TAP/map/link/pin/guard families, and implement the
already-approved audit/quiescence operations. Preserve VM reclamation-before-
sweep ordering and the existing one owner.

#### F-12 — Blocking: guest-owner failure paths call legacy netns cleanup instead of guest teardown

The new provision branch assigns an action-pool guest plan and invokes the
guest owner (`crates/overdrive-control-plane/src/action_shim/mod.rs:1304-1314`).
When that provision fails, the Start path unconditionally calls
`teardown_and_release_netns_raw` (`action_shim/mod.rs:2027-2033`); when the
driver rejects after guest provisioning, it does the same at
`action_shim/mod.rs:2137-2147`. Restart successor failure paths repeat the old
cleanup at `action_shim/mod.rs:2468-2474`, `2516-2522`, and `2584-2594`.
`teardown_for_dispatch` selects the guest owner only for the normal terminal
path (`action_shim/mod.rs:1497-1512`), so these abort/rejection paths bypass
the guest plan and never release the action-pool lease.

The production caller is concrete: `dispatch_with_network_owner` supplies both
`HostNetworkProvisioner` and `Some(shared_guest_network)` to the same
`dispatch_single` path (`action_shim/mod.rs:1174-1193`). A guest provision
failure therefore reaches the old cleanup helper, whose empty NetSlot lookup
can return success without touching the guest plan. This violates effect-first
cleanup and release-last ownership even before real kernel effects are wired.

Required bounded remediation: make every Start/Restart rejection and successor
abort use the existing guest-owner teardown path when the guest owner was the
selected provisioner, preserving the accepted first-source and release-last
semantics. Remove the old helper from the ordinary guest path; do not add a
second cleanup owner or public method.

#### F-13 — Blocking: required non-waived GREEN evidence is missing and DES GREEN/PASS was premature

The replacement DES GREEN/PASS event is recorded at
`docs/feature/netns-density-295/deliver/execution-log.json:76-94`, but the
crafter handoff explicitly reports that the non-waived owner/S13 tests and
real BPF/Lima verification were not completed. The eight waived test-side
panic bodies are excluded from this finding. The required population is ten
non-waived source-local/Sim bodies plus the active authored BPF evidence; the
crafter-reported subset covers five D12 tests and the netlink projection, but
the owner/S13 tests and BPF/Lima execution remain incomplete.

I attempted the required Linux routes. `cargo xtask lima run -- true` stopped
while the host-side launcher was building with `No space left on device`;
direct `limactl shell overdrive ...` attempts returned SSH
`Connection reset by peer` even though `limactl list` reports the VM as
running; Docker was unavailable because its daemon socket was absent. No
valid post-reorder GREEN result exists for the seeded S13 invariant, the
control-plane owner tests, or the active BPF test. This is a verification gap,
not evidence of GREEN.

Required bounded remediation: the original step crafter must obtain a valid
Linux runner and run the ten exact non-waived source-local/Sim commands plus
the relevant existing acceptance/BPF tests, with the seeded S13 invariant
GREEN after the reorder. Reconcile the DES GREEN evidence with the actual
commands; do not claim completion from compilation alone. The eight waived
placeholders remain ignored and unscored.

#### F-14 — Blocking: transitioned Contract Shape and acceptance metadata remain stale

The transitioned injection test still declares the deleted pre-grouped fields
(`netns`, `host_veth`, `workload_addr`, and the separate guest fields) at
`crates/overdrive-control-plane/src/action_shim/mod.rs:1383-1387`, although its
actual delta is only `spec.network` at `:1402-1416`. The active
`netns_density_guest_network` acceptance bodies contain `CONTRACT_SHAPE`
markers but no required `Outcome anchor: DISCUSS Elevator Pitch` declaration
(`crates/overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs:154-188`).
The current feature-delta header also still says the non-waived bodies await an
independent iteration-3 re-review (`docs/feature/netns-density-295/feature-delta.md:51-60`)
despite the upstream bounded review's approved iteration 4.

Required bounded remediation: update only the stale declaration/traceability
metadata to the exact grouped `network` delta and accepted outcome anchor/status;
preserve the authored assertions and scenario language. Do not add behavior or
change the public API.

#### F-15 — Blocking: repository formatting gate fails

`cargo fmt --all -- --check` exits non-zero on the committed step tree. It
reports extra blank lines at `crates/overdrive-control-plane/src/guest_network.rs:6`
and `crates/overdrive-dataplane/src/guest_tcx.rs:3`. `git diff --check` passes,
but that does not replace the required rustfmt gate.

Required bounded remediation: apply rustfmt to the step-owned files and rerun
the stated formatting check; no behavior or unrelated file changes are needed.

### Test honesty and quality gates

| Gate | Result | Evidence |
|---|---|---|
| Exact grouped API / visibility | PASS for declarations; FAIL for production binding | Core/VMM/pool/trait/D12 signatures match, but F-08/F-10 leave required effects unwired. |
| One owner / legacy deletion | FAIL | F-09 and F-12 show the old owner and cleanup branches remain reachable. |
| Real effects / source honesty | FAIL | F-08, F-10, and F-11. |
| D12A source-local owner algorithm | Reported PASS, not independently rerun | Existing scripted tests exercise the private owner algorithm; no valid runner was available for this review. |
| Seeded S13 invariant | BLOCKED | No post-reorder GREEN execution evidence; F-13. |
| Existing BPF evidence | BLOCKED | The active BPF test was not run in a valid Linux kernel runner; F-13. |
| Contract Shape declarations | FAIL | F-14; stale bounded-change declaration and missing acceptance outcome anchor. |
| Test modification / G9 | PASS | Replacement test changes are activation/removal of `#[ignore]` for the BPF and seeded Sim bodies; no weakened assertion or deleted test was found. |
| DES phase order | PASS mechanically | Fresh RED → GREEN → COMMIT events exist, but GREEN/PASS is not honest without the missing test results. |
| Formatting | FAIL | `cargo fmt --all -- --check`; F-15. |
| Mutation testing | NOT RUN | Correctly deferred to the final DELIVER gate. |

### Verification record

- `git diff --check`: PASS.
- `cargo fmt --all -- --check`: FAIL with the two extra-blank-line reports
  above.
- The crafter-reported Linux dataplane/netlink/control-plane compile/clippy
  checks and five D12/netlink projection tests are recorded as reported
  evidence, not independently reproduced in this replacement review.
- No valid Linux runner was available in this review to independently rerun the
  required population, and no valid green result exists for the owner/S13
  bodies or active BPF acceptance. No native-metal run was possible
  (`OVERDRIVE_METAL_TARGET` is unset); this is not counted against the eight
  waived placeholders but leaves the required non-waived Sim/BPF evidence gap.
- Worktree preservation check: only pre-existing dirty `.serena/project.yml`,
  `AGENTS.md`, and `execution-log.json` remain modified; this review changed
  no production/test/DES/roadmap file.

### Verdict

# CHANGES_REQUIRED

F-08 through F-15 are unresolved blocking findings. The replacement crafter
must remediate only the accepted 02-01 production owner/adapter/wiring scope,
obtain valid non-waived Linux evidence, correct metadata/formatting, and return
the step to this same-step review cycle. The eight user-waived test-side panic
placeholders remain ignored and unscored. No later roadmap step may start.

## Iteration 4 — remediation review

### Scope and mechanical evidence

- Reviewer: fresh isolated replacement software-crafter reviewer
- Step: `netns-density-295` / `02-01`
- Remediation commit: `da5574d82726e970e5e076872f079dbb2baddf2f`, on top of
  `6fdae90d`
- Commit scope: 14 files, 2,175 insertions, 414 deletions
- The commit retains Marcus as author and has exactly one Codex co-author and
  one `Step-Id: 02-01` trailer.
- Fresh DES events are RED/PASS at `01:29:25Z`, GREEN/PASS at `03:51:40Z`,
  and COMMIT/PASS at `03:52:49Z`.

The crafter reports owner 12/12, D12 9-pass plus one pre-existing ignored test,
three acceptance tests, seeded S13 with `PROPTEST_CASES=1024`, netlink/nft,
canonical release BPF/verifier 5/5, control-plane check/clippy, and
fmt/diff green. Lima SSH remained unavailable; the reported privileged Linux
runner supplied release ELF/BPF/verifier evidence. The eight user-waived
real-I/O placeholders remain ignored and unscored; mutation testing was not
run.

### Prior-finding dispositions

| Prior finding | Iteration-4 disposition |
|---|---|
| F-08 no-op/fake production owner | **Substantially remediated, residual F-16.** Real scratch/allocation leaves and real nft/dataplane calls now exist, but scratch inventory/exercise and complete bridge read-back still fabricate or omit accepted observations. |
| F-09 legacy production owner/branch | **Production portion closed.** The old allocator/provisioner and old error variants are cfg-gated to test/integration fixtures; production requires the shared owner and uses `ProductionNetworkGuard`. DNS's separate NetSlot fallback is outside this step's owned listener/DNS re-home. |
| F-10 D12 lifecycle/inventory | **Partially resolved, residual F-17.** Classifier loading, link identity, adoption, and non-mutating pin observation were added, but map-pin receipts are never recorded. |
| F-11 stale sweep/audit/quiescence | **Partially resolved, residual F-16/F-21.** Sweep and owner methods now perform effects, but complete D5 inventory and production callers remain incomplete. |
| F-12 guest failure cleanup | **Closed.** Start/restart rejection, successor abort, mTLS failure, and predecessor cleanup now route through `teardown_for_dispatch` with the selected guest owner. |
| F-13 missing GREEN evidence | **Closed by reported evidence.** The privileged Linux report supplies the non-waived owner/S13/BPF/verifier results; local rerun was attempted but stopped when the Darwin filesystem reached `No space left on device`, and Lima SSH remained unavailable. |
| F-14 stale Contract Shape/status metadata | **Partially resolved, residual F-20.** The grouped declaration and two acceptance anchors were corrected; the pure acceptance property has no outcome anchor and the feature header remains contradictory. |
| F-15 formatting | **Closed.** `cargo fmt --all -- --check` and `git diff --check` pass. |

### Design/API comparison

The remediation preserves the approved public API shape. D12 methods remain the
exact named methods with private fields; D12A remains module-private; the
production network guard is private; no raw aya/netlink value or generic
command surface crosses a crate boundary. The production action path no longer
falls back to the legacy NetSlot/netns owner.

The following accepted behavior remains divergent:

- The real scratch probe's `count_netlink` returns `0` for every guard table,
  chain, set, rule, and member resource (`crates/overdrive-control-plane/src/guest_network.rs:1035-1050`),
  regardless of kernel state.
- The real probe's `exercise` ignores the probe stage and only reads the local
  endpoint entry (`guest_network.rs:1019-1033`); it does not inject classifier
  traffic, original-destination traffic, or detached-link guard traffic.
- `converge_shared` verifies only bridge kind/MAC
  (`guest_network.rs:2638-2696`), not administrative-up state or the exact
  gateway/prefix read-back required by the accepted bridge identity contract.
- `GuestTcxProgram::pin_endpoint_map` and `pin_counter_map` record map IDs but
  never set `endpoint_map_pin_id` or `counter_map_pin_id`
  (`crates/overdrive-dataplane/src/guest_tcx.rs:750-792`), while pin observers
  require those receipts (`guest_tcx.rs:1215-1233`). A successful owned pin
  therefore reports `InventoryAmbiguous` during cleanup.

### New findings

#### F-16 — Blocking: the real startup probe still fabricates D5 guard counts and does not exercise the classifier

`RealSharedGuestNetworkScratchIo::count_netlink` performs real bridge/TAP
lookups but returns literal zero for all five guard resource families
(`crates/overdrive-control-plane/src/guest_network.rs:1035-1050`). The D5
cleanup complement converts those values to `Observed(0)`, so startup can pass
with residual bridge guard table/chain/set/rule/member objects.

The probe's stage-specific `exercise` is also not the accepted packet probe: it
ignores `GuestNetworkProbeStage` and merely calls `GuestTcxProgram::read_endpoint`
(`guest_network.rs:1019-1033`). A missing classifier verdict, wrong mark/MAC
rewrite, original-destination failure, or detached-link guard failure can pass
this probe.

The owner bridge read-back has the same gap. After `converge_addr` and
`set_link_up`, `converge_shared` checks only kind and fixed MAC, not `up` or
`observe_addr` for the exact gateway/prefix (`guest_network.rs:2641-2696`),
although the accepted contract requires all of those facts before admission.

Bounded remediation: use the existing D5 resource-specific observations and
the existing production classifier/guard packet boundaries for each probe stage;
call the existing `Client::observe_addr` and verify `up`/gateway/prefix. Keep
the one owner and exact private seams; do not add a public test hook.

#### F-17 — Blocking: owned map pins are never recorded as receipts

`GuestTcxInventoryReceipts` has dedicated `endpoint_map_pin_id` and
`counter_map_pin_id` fields (`crates/overdrive-dataplane/src/guest_tcx.rs:679-684`),
and `observe_pin` deliberately returns `InventoryAmbiguous` when a map pin has
no receipt (`guest_tcx.rs:1215-1233`). The remediation's pin methods set only
`endpoint_map_id`/`counter_map_id` (`guest_tcx.rs:750-792`); no production code
assigns either pin-receipt field. The source-local positive-count test writes
those fields manually, so it does not prove the real lifecycle.

Consequently, the real scratch cleanup cannot report the accepted positive
owned pin counts after pinning, adoption, and handle release; it reports an
unavailable/ambiguous family instead of an exact owned count. Record the exact
map identity in the existing private receipt state immediately after the
successful pin read-back. No API change is authorized.

#### F-18 — Blocking: D9 bridge observation/convergence is not idempotent and does not preserve complete foreign inventory

The bridge adapter now uses real `NETLINK_NETFILTER`, but its mutation methods
blindly send `NEW*` operations (`crates/overdrive-netlink/src/nft.rs:2583-2667`).
`send_batched_family` does not swallow `EEXIST` (`nft.rs:1729-1760`), so a second
identical `converge_shared` can fail on an existing table/chain/set/rule rather
than adopt it idempotently. It also does not observe/classify before mutation
to refuse foreign/conflicting state.

The read path fabricates the table and chain inventory in `expected_inventory`
and then lists only the target chain's rules and target set members
(`nft.rs:2495-2534,2669-2748`). It has no table/chain/other-child enumeration;
`other_children` is always empty. A foreign chain or child in the same bridge
table can therefore be invisible, classified as exact, and later removed by
`delete_owned_guard`, violating the exact-exclusive/foreign-preservation
contract.

The D9 source-local and integration acceptance bodies remain `#[ignore]`
(`nft.rs:2898-3193`; `crates/overdrive-netlink/tests/integration/bridge_guard_lifecycle.rs:40-43`).
They are not among the eight user-waived placeholders, so reported generic
netlink/nft checks do not establish this bridge-family contract.

Bounded remediation: complete the existing private family-aware codec's
generation-bracketed table/chain/set/child inventory, classify before mutation,
swallow only exact idempotent outcomes, preserve foreign/conflicting objects,
and activate/run the already-authored D9 tests. Do not add a second adapter or
public ABI.

#### F-19 — Blocking: allocation guard membership derives TAP names from an arbitrary ifindex

`HostGuestNetworkOwner::expected_guard_members` reconstructs every unrelated TAP
as `ovd-tp-{:04x}` from `ifindex - 293`
(`crates/overdrive-control-plane/src/guest_network.rs:1595-1610`). Linux ifindices
are kernel-assigned and have no relationship to the guest address offset. A
real attachment with ifindex 17, or a restarted node with different indices,
therefore produces the wrong expected guard complement; teardown/audit can
reject an empty named complement or accept the wrong unrelated membership.

The source-local fixtures mask this by choosing ifindices 295/296. The owner
must retain or obtain the exact accepted allocation/TAP identity through the
existing private owner state/read-back boundary; it must not derive a TAP name
from an ifindex. Keep the private state/API shape within the approved D12A
contract.

#### F-20 — Blocking: Contract Shape/status metadata remains incomplete

The remediation corrects the grouped bounded-change declaration and adds
outcome anchors to the two stateful acceptance tests, but the acceptance-file
pure property `scratch_complement_never_fabricates_zero` still has no
`Outcome anchor: DISCUSS Elevator Pitch` line (`crates/overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs:62-66`).
The feature delta also remains internally contradictory: lines 53-55 say the
independent iteration-3 re-review completed, while lines 81-87 still say the
bodies await that same review. Update only this traceability/status metadata;
preserve behavior and scenario assertions.

#### F-21 — Blocking: new owner audit/quiesce methods still have no production caller

Repository-wide production search finds `HostSharedGuestNetworkOwner::audit_shared`
and `quiesce_managed_taps` only at their definitions; the remaining callers are
Sim adapter tests. The implementation exists (`guest_network.rs:2736-2866`),
but no ordinary `run_server`, convergence loop, retained supervisor, or owner
recovery path invokes either method. The accepted owner contract requires a
real non-repairing audit and TAP quiescence boundary, and the review rule
requires every new production function to have a real production callsite.

Wire these existing private owner methods from the already-approved owner
supervision/recovery boundary when that boundary is in this step's accepted
composition; do not add a new public hook, supervisor, or recovery mechanism.

### Test honesty and verification

The BPF test loop change from byte ranges `0..42`/`14..34` to executable
post-Ethernet ranges `14..42`/`34..38` is not a G9 weakening: the accepted
DISTILL contract was concurrently corrected to exclude kernel-rejected
0..13-byte SKB inputs and explicitly require 14..41 and 34..37 partitions.
No assertion was otherwise weakened or deleted in the remediation diff.

The reported owner/D12/acceptance/S13/BPF/verifier/check/clippy evidence and
fresh DES cycle are accepted as mechanical evidence. Local Darwin execution was
not completed because the filesystem reached `No space left on device`; Lima
SSH remained unavailable. The eight waived placeholders are not scored. Fmt
and diff checks pass. Mutation testing remains correctly deferred.

| Gate | Result |
|---|---|
| Grouped/D12/D12A declaration shape | PASS at declaration level |
| Real startup/all-family complement | FAIL — F-16/F-17 |
| D9 real idempotent/conflict-preserving adapter | FAIL — F-18 |
| Allocation complement and unrelated identity | FAIL — F-19 |
| Legacy production path | PASS for production cfg split; fixture-only legacy code remains out of production |
| Guest failure cleanup | PASS — prior F-12 closed |
| Contract/status metadata | FAIL — F-20 |
| New production callsites | FAIL — F-21 |
| Test integrity | PASS — accepted BPF partition correction; no silent weakening |
| DES RED/GREEN/COMMIT | PASS mechanically and reported GREEN evidence available |
| `cargo fmt --all -- --check` / `git diff --check` | PASS |
| Mutation testing | NOT RUN, correctly deferred |

### Verdict

# CHANGES_REQUIRED

F-16 through F-21 are unresolved blocking findings. The remediation materially
closes the prior fake-owner, legacy production, cleanup, formatting, and test
evidence findings, but the accepted D5/D9/D12/D12A production contracts are not
yet source-honest or complete. Return the same step to its original crafter for
bounded remediation and re-review. Do not advance to step `02-02`; do not add
later-step DNS/listener behavior; leave the eight user-waived placeholders
ignored and unscored.

## Iteration 5 — remediation review

### Scope and mechanical evidence

- Reviewer: fresh isolated replacement software-crafter reviewer
- Step: `netns-density-295` / `02-01`
- Remediation commit: `90aea574f1e69ac43433e9bacfabe99c918b046b`, on top of
  `da5574d8`
- Commit scope: 7 files
- The commit retains Marcus as author and has exactly one Codex co-author and
  one `Step-Id: 02-01` trailer.
- Fresh DES events are RED/PASS at `04:18:35Z`, GREEN/PASS at `05:41:44Z`,
  and COMMIT/PASS at `05:42:43Z`.

The crafter reports netlink 56 plus real bridge integration, owner 12/12, D12
69 plus one pre-existing ignored test, seeded S13 at 1024 cases, Sim 97 plus
one waiver, BPF/verifier 5/5, and fmt/diff green. Fresh clippy was capacity-
blocked; prior Linux clippy evidence remains the available lint evidence. The
eight user-waived placeholders remain unscored and mutation testing remains
deferred.

### Prior-finding dispositions

| Prior finding | Iteration-5 disposition |
|---|---|
| F-16 D5 counts/exercise/bridge facts | **Partially resolved, remains blocking.** Guard-family counts, stage dispatch, and bridge up/gateway checks were added, but the stage exercise still does not inject/observe classifier frames and the owner convergence path still lacks a complete guard read-back. See F-16/F-22. |
| F-17 map-pin receipts | **Closed.** Both map pin methods now populate the matching private pin receipts; positive pin observations can be attributed to the production lifecycle. |
| F-18 D9 inventory/idempotence/foreign preservation | **Partially resolved, remains blocking through F-22.** The generation-bracketed full inventory, raw child dumps, idempotent mutation helpers, and active bridge integration test are present. The production owner does not perform the full guard observation after convergence before publishing shared state. |
| F-19 ifindex-derived TAP complement | **Closed.** Allocation state retains the exact TAP name and all guard/quiesce complements use it. |
| F-20 Contract/status metadata | **Closed.** The pure acceptance property now has its outcome anchor and the contradictory feature-delta status sentence was corrected. |
| F-21 audit/quiesce caller | **Out of step and not an approval condition.** Per orchestrator disposition, roadmap `03-03` owns runtime audit/quiescence/supervisor callsites; this review checks only that the `02-01` method bodies are real. |

### Changed trajectory artifact

`crates/overdrive-sim/tests/acceptance/fixtures/hydration_trajectory/workload_lifecycle_trajectory.txt`
changes only the expected `AllocationSpec` shape from the deleted individual
network fields to `network: None`. This is necessary compiler/golden-output
fallout from the accepted grouped handoff and does not alter the observed
action sequence, state transitions, or lifecycle assertions.

### Remaining findings

#### F-16 — Blocking: the startup probe still does not exercise the accepted packet-level stages

The remediation now dispatches on `GuestNetworkProbeStage`, verifies TCX
attachment, and reads a counter, but `RealSharedGuestNetworkScratchIo::exercise`
still performs no classifier packet injection, no mark/MAC/original-destination
assertion, and no bridge-guard frame observation
(`crates/overdrive-control-plane/src/guest_network.rs:1020-1068`). The
`DetachedLinkGuard` branch merely queries that the TCX attachment list is empty;
it never sends an unmarked frame through the still-managed TAP and observes a
guard drop. The accepted startup contract explicitly requires the real
classifier, original-destination, and detached-link guard stages before startup
success (`feature-delta.md:2154-2161,4024-4029`).

Required bounded remediation: drive the existing production classifier/guard
packet boundary for each private stage and assert the stage-specific observable
verdict/counter/capture outcome. Keep the private scratch seam and do not add a
public packet-test API or a second owner.

#### F-22 — Blocking: shared convergence publishes without a complete guard read-back

`HostSharedGuestNetworkOwner::converge_shared` invokes
`converge_table`, `converge_chain`, `converge_set`, and `converge_rules`, then
stores the loaded TCX state and returns (`crates/overdrive-control-plane/src/guest_network.rs:2697-2734`).
It never calls the now-complete generation-bracketed
`bridge::observe`/`BridgeGuardObservation` path after mutation. The D9 methods
can individually adopt existing objects, but an additional foreign chain,
flowtable/object, unexpected member, or rule in another chain is only visible
to the full inventory observer; it is not part of the owner’s pre-admission
postcondition. The accepted contract requires complete foreign/conflict
preservation and real complements before admission.

Required bounded remediation: use the existing private D9 observation to prove
the exact table/chain/set/rule/member/other-child inventory after convergence
before publishing shared TCX state. Preserve the exact D9 API and one owner.

#### F-23 — Blocking: D12 program receipt omits the required classifier tag identity

`GuestTcxProgram::load` now records program ID, name/type, and map IDs are
checked later, but `GuestTcxInventoryReceipts` has no program tag receipt and
`observe_tcx_programs` never compares `RawGuestTcxProgramObservation::tag`
(`crates/overdrive-dataplane/src/guest_tcx.rs:675-684,1148-1176`). The accepted
D12 identity requires the exact program tag/name/type/map relationship, not
only an ID plus name/type. A stale or replaced same-name classifier can
therefore pass the production observer without the accepted tag identity.

Required bounded remediation: retain the tag in the existing private receipt
identity and compare it in the existing observer. Do not expose raw aya data or
change the public API.

#### F-24 — Blocking: the down-TAP checkpoint silently accepts an incompatible observation

After `observe_tap` immediately following bridge attachment, the owner creates
`second_tap_for_final` for any non-absent observation but only calls
`ensure_master` when the observed identity is valid
(`crates/overdrive-control-plane/src/guest_network.rs:2142-2181`). A wrong
kind, owner, persistence, name, or ifindex at this required checkpoint is
silently ignored if a later final observation happens to be healthy. The
accepted D12A contract requires every TAP identity/master checkpoint to refuse
publication with the exact owner-built `Tap`/`LinkMaster` facts.

Required bounded remediation: return the exact typed postcondition mismatch for
an invalid second checkpoint; retain the existing private leaf and owner
ordering. Do not defer the mismatch to a later observation.

### Verification and test honesty

The BPF range correction remains honest: the updated DISTILL contract limits
the SKB runner to post-Ethernet lengths 14..41 and IPv4/L4 lengths 34..37, so
the remediation did not weaken a required assertion. The bridge integration
test is now active, and the crafter reports the privileged Linux verifier,
owner, D12, Sim, BPF, and netlink/nft suites green. The local Darwin runner
remained capacity-blocked; this does not invalidate the supplied Linux evidence
but prevents independent rerun in this review.

| Gate | Result |
|---|---|
| Exact public/private API shape | PASS at declaration level |
| D5 receipts and all-family observation | PASS for receipts; F-16 remains for packet-stage exercise |
| D9 source inventory/idempotence tests | PASS for the adapter test evidence; F-22 remains for owner pre-admission read-back |
| Allocation→TAP/guard complement | PASS for exact TAP retention; F-24 remains for the second checkpoint |
| Contract/status metadata | PASS |
| Test integrity | PASS; accepted BPF partition correction is specified by DISTILL |
| DES RED/GREEN/COMMIT | PASS mechanically with reported Linux evidence |
| `cargo fmt --all -- --check` / `git diff --check` | PASS |
| Mutation testing | NOT RUN, correctly deferred |

### Verdict

# CHANGES_REQUIRED

F-16 and F-22 through F-24 remain blocking. The remediation closes map-pin
receipts, exact TAP complement state, metadata, D9 inventory plumbing, and the
prior production-path findings, but the startup packet probe, owner-level guard
postcondition, classifier tag receipt, and second TAP checkpoint are not yet
source-honest. Return step `02-01` to the original crafter for bounded
remediation and re-review; do not advance to `02-02`. The eight waived
placeholders remain ignored and unscored.
