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
