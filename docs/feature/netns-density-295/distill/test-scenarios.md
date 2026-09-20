# DISTILL scenarios — `netns-density-295`

This document is specification prose. Per `.claude/rules/testing.md`, it is
not a Cucumber input and no `.feature` file exists. Each scenario below maps to
Rust `#[test]`, `#[tokio::test]`, proptest, seeded `overdrive-sim`, Tier-3
native-metal evidence, or a benchmark receipt. Tests drive the production
owners; fixtures may inject faults only through accepted driven ports.

## Reconciliation and scope

- Reconciliation passed — 0 contradictions.
- The SPIKE decision is `DISCARD from promotion; hand off to DESIGN`. Parts A
  through D are feasibility evidence only. The accepted DESIGN selects the
  shared bridge, TCX classifier plus proof-mark guard, shared DNS, two
  node-owned listeners, fixed admission cap, constant nft rules, exact
  generation ownership, and bounded recovery/fail-stop.
- Missing `discuss/` and `devops/` directories are graceful-degradation
  warnings. GH #295 plus its comments is the DISCUSS-equivalent scope source;
  the accepted single-file `feature-delta.md` is the DESIGN source. The
  project ATDD policy supplies the environment mechanisms.
- Cross-host routing is GH #298, heterogeneous schedulable capacity is GH
  #299, and pump/concurrent-flow scaling is GH #300. No scenario below
  manufactures those outcomes.
- CAP-295-A is attachment-only. `T1-BASE` and `T1-PORT4` are benchmark
  receipts, not EDD expectations and not simultaneous-flow tests.

## Journey and outcome trace

| Product journey / outcome | User-valued promise | Prose scenarios | Registered outcome |
|---|---|---|---|
| J-OPS-003 — run a VM workload | Ana deploys and stops VM work with the familiar verbs, and lifecycle/cleanup remain truthful | S-ND295-01/02/05/06/07/13/35 | OUT-ND295-SHARED-SWITCH |
| J-OPS-004 — submit a Service | A VM Service keeps its existing health/selection truth while using the shared guest network | S-ND295-01/21/34/36 | OUT-ND295-SHARED-SWITCH; related OUT-SVM-SERVICE-TARGET-PROJECTION |
| J-MESH-001 — dial a mesh peer by name | A credential-free guest resolves and reaches a healthy peer by name without a direct-L2 bypass | S-ND295-01/08/09/10/21/24/34/37 | OUT-ND295-BORN-CAPTURED |
| J-SEC-003 — enforce transparent mTLS | A guest speaks plaintext locally while the peer wire is TLS 1.3/kTLS and stale identity fails closed | S-ND295-01/10/14..26/31A/31B/36/37 | OUT-ND295-BORN-CAPTURED; related OUT-MTLS-COMPOSED-PROXY-SKELETON and OUT-MTLS-WIRE-TLS13 |
| Attachment-capacity outcome | Operators receive an honest attachment receipt without a concurrent-flow claim | B-ND295-T1-BASE / B-ND295-T1-PORT4 | OUT-ND295-DENSITY |

## Scenario-to-test matrix

| ID | Contract shape | Kind / evidence | Rust or receipt home | Status at DISTILL |
|---|---|---|---|---|
| S-ND295-00 | bounded-change | D12 private projection/capture tables, D5 continuation, and Lima inventory | dataplane `guest_tcx::tests::{every_locked_aya_map_kind_projects_to_exact_or_opaque_semantics,wrong_valid_map_properties_remain_opaque_and_schema_mismatch_is_source_less,capture_failure_keeps_an_observation_identity_and_only_the_first_genuine_source,a_unique_unreceipted_candidate_is_ambiguous_and_never_an_owned_count,every_receipted_family_returns_one_and_clean_families_return_exact_zero}`; control-plane `guest_network::scratch_probe_acceptance::*`; integration `guest_tcx_inventory::{clean_and_receipted_inventory_observes_all_eight_exact_families,retained_unpinned_maps_programs_and_links_survive_handle_release_and_remain_observable,wrong_exact_path_owner_or_valid_map_schema_is_typed_and_never_fabricates_zero}` | five non-waived D12 bodies are reasoned-pending, compile, and include distinguishable program-before-link first-source precedence; three Linux real-kernel placeholders retain Lima authority and are user-waived from this remediation gate |
| S-ND295-01 | bounded-change | native-metal production traffic composition | `guest_stack_mtls_egress::{microvm_dials_a_mesh_peer_by_name_and_receives_the_reply,the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear}` paired with the post-cut topology/cleanup body | continuing Rust proof maps the exact callee Running 1/1, reply-dependent caller success, TLS 1.3/kTLS/splice/no-cleartext wire, direct-host-TAP topology, and total cleanup; historical E07 is not current proof |
| S-ND295-02..05 | pure-function / bounded-change | grouped values plus address-pool properties and cap table | control-plane `guest_network::pool_acceptance`, `netns_density_guest_network::scratch_complement_never_fabricates_zero`, and the direct-TAP VMM projection body | complete reasoned-pending bodies |
| S-ND295-06..09 | bounded-change | action-owner refusal/lease plus classifier partitions | `netns_density_guest_network::{provision_refusal_stops_before_driver_start_and_preserves_the_typed_owner_cause,teardown_failure_holds_the_lease_until_retry_completes_then_allows_exact_address_reuse}` and `guest_tcx_classifier_test_run::classifier_partitions_return_one_verdict_and_advance_one_exact_counter` | pre-existing accepted bodies remain mapped to revised `02-01` |
| S-ND295-10 | bounded-change | Lima deliberate D6 detach + D9 guard/counter/capture/audit | `shared_guest_network_startup::deliberate_link_loss_reaches_default_drop_and_the_exact_production_audit_cause` | reasoned-pending compiled RED scaffold; fixture owns only external typed detach and observations |
| S-ND295-11 | bounded-change | D12A source-local netlink projection and real-owner order/identity tables plus Lima production read-back | `overdrive_netlink::client::tests::persistent_tap_and_bridge_projection_preserves_every_observable_identity_field`; `guest_network::allocation_owner_acceptance::{provision_reads_every_attachment_fact_before_reporting_success,every_incompatible_tap_or_bridge_identity_refuses_owner_publication}`; and `shared_guest_network_startup::ordinary_provision_reads_back_the_complete_attachment_before_injected_vmm_start` | reasoned-pending; three non-waived source-local bodies fail at the exact parser/projection or owner behavior gap, while the user-waived Lima placeholder remains a separate production-composition layer |
| S-ND295-12 | bounded-change | D12A source-local failure/continuation plus Lima two-attachment complement | `guest_network::allocation_owner_acceptance::every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement` and `shared_guest_network_startup::two_attachment_teardown_releases_last_and_preserves_the_unrelated_attachment_byte_equal` | reasoned-pending; source-local RED is current no-op teardown and its finite table compiles every exact cleanup-leaf operation/source/continuation row; the Lima body remains a waived production-composition scaffold |
| S-ND295-13 | bounded-change | seeded production-helper ordering, separate GREEN telemetry, native VMM/kernel complement | `overdrive_sim::invariants::netns_density_boot_order::tests::reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm`; `shared_guest_network_startup::{production_boot_trace_completes_vm_reclamation_before_stale_sweep_starts,native_prior_vmm_reclamation_precedes_full_attachment_sweep_and_first_lease_acceptance}` | the non-waived seeded invariant fails reproducibly on the actual sweep snapshot with shrunk seed `0`; the telemetry and native reasoned-pending placeholders are user-waived from this remediation gate, and native execution still requires `OVERDRIVE_METAL_TARGET` |
| S-ND295-14..19 | bounded-change | shared nft replacement/rollback | `mtls_intercept_port::shared_program_rollback_acceptance::{replacement_and_every_rollback_disposition_preserve_exact_identity_and_source,runtime_present_wrong_target_is_reported_without_mutation}` plus Tier-3 normalized read-back | exact private seam; absent/exact prior, rejection, successful rollback, rollback write/read source, source-less wrong identity, and runtime no-rewrite are executable |
| S-ND295-20..26 | bounded-change | node-shared listener/capability lifecycle | `netns_density_shared_owner::*`, `capability_registry_acceptance::*`, plus existing real-enforcement retirement/zero-copy suites | complete reasoned-pending owner start, leg-F/leg-C/rule partial cleanup, exact-port recovery/occupied refusal, D7 generation/conflict/Pending/claim/publication/scoped-drain/reuse, isolated handle and shutdown bodies |
| S-ND295-27 | bounded-change | Rust acceptance | `crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs` | one active RED + five reasoned-pending recovery/fail-stop/model/table bodies authored |
| S-ND295-28 | bounded-change | generated gate PBT plus deterministic real-`VmDriver` schedules | `generated_operation_sequences_match_the_gate_model`, `writer_bound_overlaps_the_single_vmm_grace_and_every_writer_is_consumed`, `backpressured_exec_release_cannot_delay_stop_deadline`, `cancelling_backpressured_release_cannot_leave_an_exec_sender_running`, and `start_allocation_awaits_release_and_cancellation_owns_the_future` | no seeded-sim writer or injected supervisor consequence |
| S-ND295-29..32 | bounded-change | owner-port schedule/recovery and finite exit table | paired gate bodies, `netns_density_shared_owner::{lost_leg_f_rebinds_the_recorded_nonzero_address_before_audit_succeeds,occupied_original_leg_f_address_refuses_recovery_without_selecting_another_port}`, B1 host/sim owner bodies, DNS lifecycle, cgroup/VMM cleanup regressions, and `shared_network_task_owner_acceptance::*` | D8 drives actual Tokio return/error/panic/cancel/channel-close across all 12 snapshot components and DNS live/exited replacement/shutdown matrices; 31A/31B are distinct actual socket-state bodies |
| S-ND295-33 | bounded-change | direct-handler request ownership and recurring system conformance | `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs` over `tests/conformance/src/lib.rs` | exported server handler + HTTPS API; no product CLI, subprocess SUT, PID oracle, assert_cmd, or trycmd |
| S-ND295-34 | bounded-change | Tier 3 adapter integration | production DNS bind/wire evidence plus private `DnsServeTaskOwner` lifecycle through the retained supervisor | actual task return/panic/cancel, replacement, intentional shutdown, and invalid-state bodies compile; real shared-gateway wire evidence stays Tier 3 |
| S-ND295-35 | bounded-change | Tier 3 native metal + existing owner contracts | `vm_walking_skeleton.rs::two_vm_allocations_share_the_node_bridge_without_per_workload_namespaces` paired with `one_server_boot_shares_exactly_one_trusted_probe_runner_with_vm_driver`, `vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner`, and probe-target acceptance bodies | complete reasoned-pending topology body plus mapped existing probe-owner bodies |
| S-ND295-36 | bounded-change | non-regression composition | `guest_stack_mtls_egress::{microvm_dials_a_mesh_peer_by_name_and_receives_the_reply,the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear}`, `service_kind_vm_workloads`, VM cgroup/accounting equivalence, `mtls_resolve_rekey`, and Service dataplane selection suites | mapped existing bodies remain mandatory; no #295 duplicate body |
| S-ND295-37 | bounded-change | Tier 3 timed fault example | `vm_walking_skeleton::simultaneous_external_tcx_and_guard_loss_quiesces_the_managed_tap_within_one_second` | complete reasoned-pending native-metal body uses D6 typed attachment/endpoint/counter operations, D9 semantic observation/`delete_owned_guard`, identifiable guest frames, passive guard counter, peer-TAP capture, one-second TAP quiescence, post-quiescence no-forwarding, and total cleanup |
| B-ND295-T1-BASE | bounded-change | benchmark receipt | `target/benchmarks/netns-density-295/T1-BASE/` via a future DELIVER benchmark target | benchmark design below; not EDD |
| B-ND295-T1-PORT4 | bounded-change | benchmark receipt | `target/benchmarks/netns-density-295/T1-PORT4/` via the same target | benchmark design below; not EDD |

## Outcome traceability and marker discipline

Every S-ND295 scenario and benchmark receipt rolls up to a registered
`OUT-ND295-*` outcome (registry `docs/product/outcomes/registry.yaml`), so the
outcome → scenario reverse-trace is total:

- **OUT-ND295-SHARED-SWITCH** — S-ND295-00, 02, 03, 04, 05, 06, 07, 11, 12, 13,
  27, 28, 29, 30, 31A, 31B, 32, 33, 34, 35 (grouped network assignment,
  lease/admission, provision/teardown, boot reclamation, gate admission,
  bounded recovery/fail-stop, shared-gateway DNS, direct-TAP VMM).
- **OUT-ND295-BORN-CAPTURED** — S-ND295-01, 08, 09, 10, 14–26, 37 (TCX
  classification/guard, shared nft replacement/rollback, node-shared
  listener/capability lifecycle, double-loss quiescence); related
  OUT-MTLS-COMPOSED-PROXY-SKELETON and OUT-MTLS-WIRE-TLS13.
- **OUT-ND295-DENSITY** — B-ND295-T1-BASE, B-ND295-T1-PORT4 (attachment-only
  capacity receipts).
- **Non-regression** — S-ND295-36 maps to the existing
  OUT-SVM-SERVICE-TARGET-PROJECTION / OUT-MTLS-WIRE-TLS13 outcomes; it
  introduces no new #295 outcome.

Marker discipline: every body added or transitioned by the phase-02 remediation
names exact step `02-01` in its reasoned `#[ignore]` marker. Revised `02-01`
owns S-ND295-00, 02..04, and 06..13; S-ND295-05 remains exclusively `03-01`;
S-ND295-14..19 remain `02-02`; S-ND295-20..26 remain `02-03`.

## Acceptance specification

### Primary stakeholder scenarios

The four blocks here are the stakeholder-readable acceptance language. The
numbered S-ND295 sections that follow are technical verification contracts:
they preserve exact ports, errors, counters, state universes, and evidence
lanes without pretending those implementation terms are user goals.

### An operator deploys two VM workloads that can reach each other by name

`@walking_skeleton @driving_port @real-io @contract-shape:bounded-change`

```gherkin
GIVEN a node ready to accept VM work
WHEN Ana deploys the checked-in VM Service and its VM client Job
THEN the callee reports Running with replicas 1/1
AND the reply-dependent caller reports Succeeded with the exact guest reply
AND stopping both workloads leaves no resource owned by that journey
```

### A security reviewer sees encrypted peer traffic without guest credentials

`@security @real-io @contract-shape:bounded-change`

```gherkin
GIVEN two credential-free guests communicating through the platform
WHEN one guest exchanges a byte-distinct request and reply with the other
THEN the peer-facing wire carries TLS 1.3 application data in both directions
AND no application cleartext or direct guest-to-guest bypass is observed
AND stopping one workload cannot transfer its identity or connection to a successor
```

### A node refuses new work when shared-network trust cannot be proved

`@operator @error @recovery @contract-shape:bounded-change`

```gherkin
GIVEN a running node loses a shared-network owner or finds damaged network state
WHEN the platform cannot restore and verify the owner within five seconds
THEN new guest commands remain closed and the node exits visibly instead of running partially trusted
AND the failed handler shuts down with an operator-visible reason
AND a deployment supervisor may construct a fresh process as a separate operational action
AND the replacement handler accepts work only after reclamation and startup checks complete
```

### Capacity evidence says exactly what was measured

`@benchmark @capacity @contract-shape:bounded-change`

```gherkin
GIVEN the declared attachment-only T1 workload profiles
WHEN the production attachment owner is measured on the qualified native host
THEN the receipt reports exact attachment, rule, element, counter, memory, update, and cleanup inventories
AND it makes no claim about VM population, simultaneous flows, throughput, file descriptors, pump threads, or stacks
```

## Technical verification contracts

### S-ND295-00 — The node refuses work when its shared-network proof is incomplete

`@driving_port @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN the operator starts a node whose isolated shared-network proof encounters one accepted fault
WHEN the node validates its network substrate before admitting workloads
THEN startup refuses with an actionable shared-network cause and no workload is admitted
AND every scratch resource is removed before the refusal is reported
AND no production shared-network task or owner is published
```

Technical mapping: one table covers TCX load/verifier refusal, valid-frame
classification/original-destination mismatch, deliberate TCX-link-loss guard
failure, and scratch-cleanup complement failure. The composed `run_server`
body must prove scratch probe precedes production convergence/admission,
`GuestNetworkExecSupervisor::is_boot_closed()` remains true,
`health.startup.refused` is emitted, and no production listener/DNS/supervisor
task is published. Only the private host-owner algorithm table proves the
complete scratch bridge/TAP/link/pin/map/guard/rule complement; the public
simulation owner scripts the final result solely to prove caller reaction.
The D12 remediation adds a separate dataplane-private evidence layer:
`project_map_kind`/`project_map_schema` cover the locked aya map-kind table and
opaque property equality; `capture_with_source` deterministically fails maps,
programs, and links independently plus simultaneous program/link failure using
outer `Program` errors with distinguishable nested aya I/O sources while returning an observation-capable
identity; D5 maps all eight TCX resource families after explicit cleanup and
both handle-release boundaries. The earlier program-enumeration source moves
once into `TcxLoad`, while the failed link family is exact source-less
`CaptureUnavailable`; independent domains and exact pin paths continue;
no-receipt candidates are always `InventoryAmbiguous`.
Lima `guest_tcx_inventory` owns real enumeration, by-ID and exact-path proof,
including deliberately retained unpinned map/program/link objects.

### S-ND295-01 — Two VM workloads communicate by name through the production shared guest network

`@walking_skeleton @driving_port @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN the built default-feature node is started on native metal with no live allocation owned by the example
WHEN the operator deploys the checked-in VM Service and VM client Job through `overdrive deploy`
THEN the callee reports Running with replicas 1/1 and the reply-dependent caller reports Succeeded after receiving the byte-distinct guest reply by service name
AND the peer-facing wire contains TLS 1.3 application data in both directions with no application cleartext
AND the production path owns one shared guest switch, two shared interception listeners, one TAP per live guest, and no per-workload namespace, veth pair, `/30`, `NetSlot`, or `host_veth`
AND public stop plus owner cleanup leaves the example's VMM, TAP, classifier, nft element, cgroup, run-directory, and preparation complements empty
```

The Rust test enters through the existing production in-process composition and
does not spawn the `overdrive` binary. E07's retained capture is historical
pre-cut evidence and is **not** #295 proof. A fresh post-#295 E07 capture may
be reviewed as a point-in-time built-binary receipt; the stabilized Rust
walking skeleton remains the continuing regression alarm.

### S-ND295-02 — A guest network assignment is complete or absent

`@property @contract-shape:pure-function`

```gherkin
GIVEN any valid allocation specification and any valid guest address, TAP, MAC, gateway, prefix, and DNS values
WHEN the network assignment is projected into the VM attachment and guest token
THEN all six guest-network facts travel together or none of them do
AND the VM attachment contains exactly TAP and MAC
AND every non-network allocation fact is unchanged
```

### S-ND295-03 — Every admitted guest receives one collision-free lease

`@property @boundary @contract-shape:pure-function`

```gherkin
GIVEN a `/16` node guest prefix whose network, broadcast, and gateway are reserved
WHEN any sequence of zero, one, or many distinct allocation identities is assigned up to the admitted population
THEN every held address is unique and is the smallest currently free non-reserved address
AND each TAP is `ovd-tp-<4hex>` from the full 16-bit host offset
AND each guest MAC is `02:00:<IPv4>` and can never equal the fixed `02:01:00:00:00:01` bridge MAC
```

### S-ND295-04 — Lease replay and release change only the named allocation

`@property @error @contract-shape:bounded-change`

```gherkin
GIVEN any held lease map and one allocation identity
WHEN assignment is repeated, release is repeated, or release is requested for an absent allocation
THEN repeated assignment returns the byte-equal existing plan
AND absent or repeated release is an idempotent no-op
AND every other allocation binding is unchanged
AND the next assignment may reuse a released address only after release-last cleanup completed
```

### S-ND295-05 — Fixed admission stops before lease assignment

`@property @boundary @error @contract-shape:pure-function`

```gherkin
GIVEN a node with 16,383, 16,384, or 16,385 active allocations
WHEN placement evaluates one additional workload
THEN 16,383 may continue through the existing resource checks
AND 16,384 or more returns the existing `NoCapacity` outcome before address assignment
AND no public, wire, persisted, or advertised `network_ports` value is created
```

### S-ND295-06 — Start publishes no partial attachment

`@property @tier1 @in-memory @error @contract-shape:bounded-change`

```gherkin
GIVEN the production action owner has assigned one guest lease
WHEN any one of TAP creation, bridge attach, guard membership, endpoint insert, TCX attach, pin, or read-back fails through its accepted driven port
THEN the typed operation and original source reach `ShimError::GuestNetwork`
AND the VMM is not started and guest EXEC is not released
AND the named allocation's partial effects are removed while unrelated attachments are unchanged
AND the lease remains held until effect-first cleanup completes
```

### S-ND295-07 — Stop removes the predecessor before address reuse

`@property @tier1 @in-memory @contract-shape:bounded-change`

```gherkin
GIVEN an active allocation with a guest lease, exact registration capability, shared IP elements, endpoint entry, pinned TCX link, guarded TAP, and VMM owner
WHEN the production action owner stops or replaces that allocation
THEN it quiesces the driver, removes registration visibility, waits in-flight claims, drains handles, removes IP elements, endpoint, TCX pin/link, TAP, and guard membership, and releases the address last
AND no successor can receive that address before the predecessor complement is empty
AND unrelated allocations and both shared listeners remain unchanged
```

### S-ND295-08 — Valid guest traffic enters the protected path exactly once

`@property @tier2 @tier3 @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN a managed TAP whose endpoint map contains the exact source IP, source MAC, and fixed bridge MAC
WHEN valid peer-MAC TCP, gateway-MAC TCP, gateway traffic, and valid ARP request/reply frames enter TCX ingress
THEN both TCP shapes preserve original IP and port, receive intercept mark `0x295a`, bridge-MAC rewrite, and local-host delivery
AND accepted gateway/non-TCP and ARP shapes receive accepted mark `0x295b`
AND exactly one matching counter advances for each frame
```

### S-ND295-09 — Malformed or impersonated guest traffic cannot escape

`@property @tier2 @tier3 @error @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN the complete finite partition of map miss, short Ethernet, every ARP truncation through byte 41, wrong ARP type/length/opcode, MAC spoof, sender-IP spoof, non-IP/non-ARP, and peer-directed non-TCP input
WHEN each frame enters the production classifier
THEN it returns `TC_ACT_SHOT`
AND exactly the declared one of map-miss, malformed, MAC-spoof, IP/ARP-spoof, or direct-bypass counters advances
AND peer-TAP and host captures observe no escaped packet
```

### S-ND295-10 — A second safety barrier still blocks traffic when primary protection is removed

`@tier3 @error @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN a managed TAP remains in the bridge guard and has a healthy endpoint entry
WHEN an external actor detaches its TCX ingress link and a valid guest frame is sent
THEN the unmarked frame reaches the guard's passive counter and drops
AND it reaches neither another guest TAP nor the host IP stack
AND the owner audit reports the exact missing link identity
```

Executable body:
`shared_guest_network_startup::deliberate_link_loss_reaches_default_drop_and_the_exact_production_audit_cause`.
It enters through ordinary production composition, uses only D6 query/detach
for the external mutation, and leaves creation of the TAP, endpoint, link, pin,
and D9 guard entirely to production.

### S-ND295-11 — A workload is admitted only after its complete network attachment is verified

`@tier3 @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN the production shared-switch owner and one unused lease
WHEN it provisions the attachment
THEN guard membership precedes endpoint insertion and TCX attach/pin/query while the TAP is down
AND success returns only after bridge master, link state, endpoint value, program, attach type, ifindex, pin, and fixed bridge MAC read back exactly
AND no production effect is installed by the test fixture
```

Paired executable bodies:
`overdrive_netlink::client::tests::persistent_tap_and_bridge_projection_preserves_every_observable_identity_field`,
`guest_network::allocation_owner_acceptance::{provision_reads_every_attachment_fact_before_reporting_success,every_incompatible_tap_or_bridge_identity_refuses_owner_publication}`
and
`shared_guest_network_startup::ordinary_provision_reads_back_the_complete_attachment_before_injected_vmm_start`.
The private netlink projection table covers absent, persistent TAP with exact
or missing owner UID, non-persistent TAP, TUN, dummy, veth, other, and correct
or same-name wrong-kind bridge messages with exact ifindex, up/master, MAC,
persistence, and bridge identity. The D12A scripted
leaf returns those actual TAP/bridge observations only; it never returns a
completed workflow or boolean verdict. The owner must refresh bridge identity
before both master comparisons, reject every first/final checkpoint partition
without publication, and publish the exact allocation record only after the
final checkpoint succeeds.

### S-ND295-12 — A stopped workload leaves none of its network attachment behind

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN one active attachment beside an unrelated active attachment
WHEN the named attachment is torn down and one deletion is retried
THEN endpoint deletion, TCX unpin/detach, guarded TAP deletion, and guard removal complete before lease release
AND the named complement becomes empty without changing the unrelated attachment
AND a normal-path deletion failure remains typed and is never hidden by best-effort Drop
```

Paired executable bodies:
`guest_network::allocation_owner_acceptance::every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement`
and
`shared_guest_network_startup::two_attachment_teardown_releases_last_and_preserves_the_unrelated_attachment_byte_equal`.
The source-local body combines a primary endpoint-deletion failure with a later
TAP-deletion failure, requires later cleanup calls, returns the exact first
source, retains the same owner/allocation and lease, then disarms and retries to
the named empty complement. It then repeats the named teardown and calls a
never-published teardown twice with no leaf calls; the unrelated attachment's
exposed facts remain byte-for-byte equal. A separate finite table in the same
body fails every endpoint/link/pin/TAP/guard cleanup mutation and observation
leaf—including both TAP-observation occurrences—and asserts its exact
operation/source plus continuation through final guard observation. Lima owns
actual two-attachment isolation and release-last read-back.

### S-ND295-13 — Boot reclaims old owners before accepting new leases

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN a previous process left a VMM, managed TAP, endpoint entry, TCX pin/link, guard membership, and shared dynamic elements
WHEN a fresh `overdrive serve` starts
THEN VMM reclamation completes before shared-switch sweep
AND the zero-managed-TAP and zero-dynamic-element complements are read back before the empty address pool accepts assignment
AND no old VMM, allocation capability, listener, or address lease is adopted
```

Three independent executable layers are named. The first is the seeded
production-helper invariant
`reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm`:
the same `SimVmHostState` is injected into production reclamation and
`SimSharedGuestNetworkOwner::with_sweep_host_state`, so the actual sweep port
snapshot proves ordering and prints the replay seed. The second is
`production_boot_trace_completes_vm_reclamation_before_stale_sweep_starts`, a
separate GREEN obligation over the exact structured phase events. The third is
`native_prior_vmm_reclamation_precedes_full_attachment_sweep_and_first_lease_acceptance`,
which retains independent real-VMM/kernel authority and is gated by the native
metal target.

### S-ND295-14 — A restarted node preserves protection while refreshing listener destinations

`@tier3 @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN EXEC is BootClosed, managed TAPs are zero, and an exact owned prior eight-rule/three-set program names old F and C targets
WHEN fresh F and C listeners bind ephemeral non-zero ports and shared convergence runs
THEN one atomic transaction replaces only the owned target registers
AND normalized rule order, expressions, userdata, set schemas, foreign objects, and the zero-element complement remain equal
AND admission opens only after full listener, rule, set, target, and complement read-back
```

### S-ND295-15 — A restarted node refuses ambiguous retained security state without changing it

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN any foreign object, unknown userdata, duplicate owned rule, conflicting set schema, incomplete prior program, nonzero managed-TAP complement, or non-BootClosed gate
WHEN fresh-process target recovery is requested
THEN startup refuses before target mutation
AND EXEC remains BootClosed
AND every prior owned and foreign object remains byte-equal
```

### S-ND295-16 — A failed restart change restores the prior protection exactly

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN an exact owned prior program and a committed replacement whose mandatory read-back mismatches
WHEN one atomic rollback commits and its full read-back equals the captured prior identity
THEN startup still refuses with `NftSharedReplacementMismatchRolledBack`
AND the outcome carries prior, requested, and replacement observations with no fabricated source
AND fresh listeners/tasks close and EXEC remains BootClosed
```

### S-ND295-17 — Restart rollback failures keep their operator-actionable causes

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN replacement read-back mismatched
WHEN rollback write fails or rollback read fails at the netlink boundary
THEN `NftSharedRollbackFailed` names `RestorePrior` or `ReadBackPrior` respectively
AND it carries the real `NetlinkError` plus prior, requested, and replacement observations
AND no semantic mismatch is disguised as an I/O source
```

### S-ND295-18 — A node refuses when prior protection cannot be restored exactly

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN rollback write and read both completed successfully
WHEN the rollback observation does not equal the captured prior program
THEN startup refuses with `NftSharedRollbackPostconditionMismatch`
AND both replacement and rollback observations are retained
AND no `NetlinkError` is fabricated
```

### S-ND295-19 — Live repair never redirects protection to a different listener

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN the node owner is running and one owned constant rule names a different listener target
WHEN runtime convergence audits the shared program
THEN it reports a structured conflict without mutation
AND it never binds port zero or substitutes a port
AND bounded recovery eventually fail-stops if the conflict persists
```

### S-ND295-20 — All local workloads share one protected connection entry pair

`@in-memory @contract-shape:bounded-change`

```gherkin
GIVEN the shared intercept adapter and enforcement, resolution, and clock ports are healthy
WHEN the worker starts the shared owner
THEN it publishes exactly one leg-F socket/task, one leg-C socket/task, one node guard, and one task-observation receiver only after full audit
AND starting again is idempotent or returns the accepted lifecycle result without adding cardinality
AND partial start failure returns to the Absent complement
```

### S-ND295-21 — A workload becomes protected and reachable all at once

`@property @in-memory @contract-shape:bounded-change`

```gherkin
GIVEN any allocation capability and any finite declaration-order listener list containing TCP duplicates and UDP listeners
WHEN allocation registration succeeds
THEN it mints one checked non-zero generation, acquires managed/source elements and one element per distinct TCP port, and publishes source and IP-keyed destination indexes atomically
AND UDP ports create no TCP membership while TCP order/dedup projection is deterministic
AND a conflicting live key or failed element acquisition publishes no Active capability
```

### S-ND295-22 — An exhausted node session refuses new protected membership without side effects

`@in-memory @boundary @error @contract-shape:bounded-change`

```gherkin
GIVEN the next node-session generation is `u64::MAX`
WHEN an allocation registration is attempted
THEN it returns `GenerationExhausted { next: u64::MAX }`
AND `u64::MAX` is never minted
AND indexes, capability state, shared elements, handles, and listener cardinality remain unchanged
```

### S-ND295-23 — Stopping during connection setup cannot leak predecessor identity

`@tier1 @tier3 @error @contract-shape:bounded-change`

```gherkin
GIVEN an accepted connection has claimed one exact active capability and production enforcement is paused
WHEN allocation stop removes index visibility, marks that generation Retiring, and waits
AND enforcement returns a real handle after retirement began
THEN publication is refused and the late handle is awaited to teardown outside the registry lock
AND stop does not complete until the in-flight count reaches zero
AND no successor receives the handle or identity
```

### S-ND295-24 — Address reuse never reattributes an accepted predecessor connection

`@property @tier1 @error @contract-shape:bounded-change`

```gherkin
GIVEN a predecessor connection captured generation A before its address was released and later assigned to successor generation B
WHEN the predecessor finishes enforcement and a new connection is accepted
THEN the predecessor is rejected at publication and torn down under A
AND the new connection selects only B and B's SPIFFE identity
AND unknown, stale, and post-removal connections fail closed
```

### S-ND295-25 — Stopping one allocation leaves unrelated handles and listeners live

`@tier3 @real-io @contract-shape:bounded-change`

```gherkin
GIVEN two active allocations each own an enforced connection through the node-shared listeners
WHEN the first allocation stops
THEN only its exact capability and handle set drain
AND the second handle, both listener sockets, both accept tasks, and the node guard remain live
AND the second allocation completes a byte-distinct exchange afterward
```

### S-ND295-26 — Node shutdown drains every protected connection owner before completion

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN multiple active capabilities and in-flight claims
WHEN the node owner shuts down
THEN it closes both admissions, retires every capability, waits every claim, drains handles, removes allocation elements, joins both tasks, and closes both sockets
AND it relinquishes rather than deletes the constant empty rules/sets
AND it returns only after completion or the accepted aggregate teardown error
```

### S-ND295-27 — Guest commands share one node-trust admission boundary

`@property @tier1 @contract-shape:bounded-change`

```gherkin
GIVEN any generated legal or illegal operation sequence over BootClosed, Open, Recovering, and FailStop
WHEN boot-open, claim/drop, begin-recovery, complete-attempt, and fail-stop operations execute through their separate opaque capabilities
THEN only the accepted state transitions occur
AND recovery blocks new claims without revoking a claim linearized before detection
AND FailStop wakes waiters with refusal and is terminal
AND later fail-stop calls produce no second request
```

The model property generates `open_after_boot`, `begin_recovery`,
`complete_attempt`, and `fail_stop` operations and compares every return plus
recovery projection to an independent state model. A separate finite table
drives all six fail-stop causes. The illegal-event table explicitly attempts
an invalid operation from each of BootClosed, Open, Recovering, and FailStop.

### S-ND295-28 — Recovery never leaks or wrongly releases a guest command

`@property @tier1 @in-memory @error @contract-shape:bounded-change`

```gherkin
GIVEN a guest command is waiting for release while the node may lose shared-network trust
WHEN recovery, writer acknowledgement, cancellation, and command ownership occur in any accepted order
THEN a command claimed before detection may finish and a later command remains paused until recovery completes
AND a fail-stop neither writes nor discards the guest's pending command
AND cancellation leaves no detached writer or retained claim
```

Technical mapping: the generated core operation-sequence property owns every
legal and illegal gate transition. Deterministic real-`VmDriver` schedules use
the existing beacon socket fixture for recovery-before-release,
claim-before-detection with backpressured acknowledgement, cancellation after
writer transfer, and claim drop. The existing held-`Driver` action-shim body
proves the production owner awaits and owns cancellation. This evidence does
not introduce a seeded simulation writer seam and never feeds a supervisor
transition back as a cause.

### S-ND295-29 — Every shared owner follows the same bounded recovery contract

`@property @tier1 @error @contract-shape:bounded-change`

```gherkin
GIVEN each finite component in the accepted audit order is independently missing, corrupt, or terminated through its existing driven port
WHEN the retained supervisor detects the fault
THEN it closes EXEC under the paired lock before emitting unhealthy evidence
AND attempts occur at 250 ms through 5 s, each counted only after convergence plus full audit returns
AND partial repair records the first remaining component and never emits recovered or reopens EXEC
AND complete repair performs one Recovering-to-Open transition
```

Operational-event oracle: append-only diagnostics contain exactly one
`guest_network.shared_owner_unhealthy` for the episode; exactly one
`guest_network.shared_owner_retry` for each completed attempt with the same
component/attempt/monotonic-elapsed snapshot the gate reports; and exactly one
`guest_network.shared_owner_recovered` before admission reopens. The fail-stop
branch instead records `cleanup=abandoned_to_shutdown` plus owned TAP/link/pin/
map/managed-set/intercept-set/handle counts, then records exactly one
`cleanup=drained_before_exit` or `cleanup=abandoned_at_exit`. The next boot
appends `guest_network.shared_owner_boot_recovered` with recovered counts and
`complement_empty=true`. Retries and later cleanup never overwrite the initial
failure or an earlier attempt record.

### S-ND295-30 — Unconfirmed traffic isolation stops affected guests fail-closed

`@tier1 @tier3 @error @contract-shape:bounded-change`

```gherkin
GIVEN a bridge, TCX, map, pin, or nft kernel-path mismatch was detected
WHEN the owner cannot confirm every managed TAP is administratively down
THEN it invokes the existing per-VM cgroup kill for the affected node inventory
AND it enters fail-stop rather than leaving a potentially forwarding guest alive
AND unrelated host resources remain unchanged
```

### S-ND295-31A — The shared listener returns on its original address

`@tier3 @real-io @contract-shape:bounded-change`

```gherkin
GIVEN a running workload loses one shared connection listener while its original address remains available
WHEN the platform repairs the shared network owner
THEN new workload commands remain paused until the listener returns on the same address and every shared-network check passes
AND already-running workload commands and established protected connections keep their existing ownership
```

Technical mapping: the worker rebinds the recorded non-zero
`SocketAddrV4`, acquires/read-backs the replacement node guard before
relinquishing the prior guard, performs the full audit, and only then completes
the Recovering-to-Open transition. Port zero and target rewrite are forbidden.

### S-ND295-31B — Port theft ends in a bounded, visible fail-stop

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN a running workload loses one shared connection listener and another process occupies its original address
WHEN the platform retries repair for five seconds
THEN no replacement address is selected and no workload command is newly released
AND the node exits with an operator-visible shared-network fail-stop after the bounded retry window
```

Technical mapping: every attempt requests the exact recorded address, reports
`EADDRINUSE` through the typed owner error, preserves nft targets, and emits one
completed-attempt retry observation before the five-second request.

### S-ND295-32 — Every abnormal network-owner exit closes new guest commands visibly

`@in-memory @error @contract-shape:bounded-change`

```gherkin
GIVEN the retained supervisor returns, returns a typed error, panics, is cancelled, or loses its request channel
WHEN `ServerHandle` observes that finite exit class
THEN it writes FailStop under the paired lock before returning a typed request
AND no prior recovery reports Supervisor, zero attempts, and zero elapsed
AND in-progress recovery reports the latest remaining component, completed attempts, and injected-clock elapsed
AND late retry completion cannot overwrite FailStop
```

### S-ND295-33 — A failed handler shuts down before a fresh handler admits work

`@driving_port @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN the node is running protected VM workloads through one production server handler
WHEN shared-network ownership cannot be restored within the bounded recovery window
THEN the handler returns the typed fail-stop request rather than continue in a partially trusted state
AND a fresh handler instance accepts new work only after recovery checks complete
AND an ordinary operator interrupt still drains normally
```

Technical mapping: the reusable conformance library starts the exported
production server handler directly, drives submission/status/stop only through
its HTTPS API, scripts only the accepted shared-owner result, observes the
typed `ServeShutdownRequest`, drains the first owner, and starts a fresh
handler over the same data/config roots. `nix` and `overdrive-netlink` provide
typed host observations. No product CLI, subprocess SUT, PID oracle,
assert_cmd, or trycmd participates.

### S-ND295-34 — Shared-gateway DNS stays truthful and supervised

`@tier3 @error @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN the existing NameIndex contains a healthy backend and the shared gateway is configured
WHEN the DNS owner probes, serves A and AAAA queries, encounters malformed input, or loses its serve task
THEN wildcard `:53` is tried first and only `EADDRINUSE` falls back to the exact shared gateway
AND A/NODATA/NXDOMAIN, transaction ID, SOA, source pinning, and live backend truth remain unchanged
AND task loss enters the common bounded recovery contract with no alternate answer or port
```

### S-ND295-35 — The VMM starts without per-workload network indirection

`@tier3 @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN the production owner supplied one grouped network assignment
WHEN the VMM adapter launches the guest on native metal
THEN its network argument contains only the assigned TAP and MAC
AND no `ip netns exec`, workload namespace, veth pair, `/30`, or host-veth identity appears
AND existing cgroup, confinement, rootfs, READY, probe-target, and total-teardown contracts remain true
```

The native body asserts TAP/MAC argv, host network namespace, one bridge,
fixed bridge MAC, no workload veth/netns/new `/30`, per-allocation cgroup and
run-directory ownership, allocation-owned rootfs clone, preserved operator
master, and total owned cleanup. Existing paired probe-owner bodies prove the
same trusted `ProbeRunner` remains mandatory, default/wildcard HTTP/TCP targets
use `workload_addr`, lifecycle hooks are delegated once, and probe outcomes do
not take ownership of Running.

### S-ND295-36 — Existing identity, health, placement, and resource guarantees remain intact

`@tier3 @contract-shape:bounded-change`

```gherkin
GIVEN a VM Service is deployed through the shared guest network
WHEN it becomes healthy, receives selected Service traffic, presents workload identity, and is later stopped
THEN cgroup-BPF remains the backend selector and its selected-BackendId receipt remains the peer-identity input
AND IdentityMgr holds credentials while the guest holds none
AND HTTP/TCP probes target the persisted workload address
AND per-VM cgroup limits, OOM attribution, VMM ownership, and cleanup remain unchanged
```

### S-ND295-37 — Simultaneous external protection loss is bounded and visible

`@tier3 @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN a managed guest is producing identifiable plaintext frames
WHEN an external actor deletes both that TAP's TCX link and the independent bridge guard immediately after one audit
THEN the next one-second audit closes EXEC and quiesces the managed TAP within one second of deletion
AND no claim is made that the double-loss interval itself is fail-closed
AND after quiescence no further guest frame reaches ordinary bridge forwarding
```

## Benchmark receipts — not expectations

`T1-BASE` and `T1-PORT4` run only after functional gates are green. Each run
records source SHA and dirty state, native-metal identity, kernel, CPU/memory,
limits, warm/cold state, N, M, port distribution, raw samples, summary method,
and cleanup complement. The harness drives the production attachment owner but
does not populate VMMs or `EnforcedConnection`s.

| Receipt | Population | Required raw measurements and oracles |
|---|---|---|
| T1-BASE | N=16,384 Job-shaped attachments, M=0 | exact 16,384 TAP/FDB/TCX/link/pin/map/guard/lease facts; 32,768 IP elements + 16,384 bridge elements; kernel memory delta by object family; attach and delete samples/rates; one outbound-hit packet per source plus 256 deterministic misses; exact counters; full 49,152-element sweep and empty complement |
| T1-PORT4 | N=16,384 Service-shaped attachments with TCP 8080/8081/8443/9000, M=65,536 | exact 16,384 attachment facts; 98,304 IP elements + 16,384 bridge elements; memory/update samples; four hits and one port-9001 miss per destination (81,920 packets); exact counters; full 114,688-element sweep and empty complement |

No percentile, throughput, latency, memory, or rate threshold is invented in
DISTILL. DESIGN requires measurement receipts and exact correctness/inventory;
promotion thresholds need measured baseline evidence rather than an authored
number. Connection FDs, pump threads/stacks, simultaneous flows, throughput,
and VMM capacity are excluded.

## Tier and fixture policy

- Layer 1/2 unbounded domains use proptest; finite error/component/cause sets
  use one table-driven Rust test. Slow real-kernel paths are named examples.
- D12's locked aya map-kind space is a finite closed table, not PBT. D12A's
  TAP/bridge identities and teardown leaves are likewise finite owner tables.
  S13 alone is an unbounded seeded property; its failure always prints and
  shrinks the seed.
- Seeded control-plane cases use the accepted test-gated action-shim and
  convergence-runtime seams so production owners author observations and
  cleanup. They do not seed terminal consequences or add a production seam.
- Kernel effects run under Lima except real microVM/KVM and final shared-bridge
  composition, which run only through `cargo xtask metal run --`.
- S-ND295-13 is three separately selected lanes: the seeded sweep-call
  invariant is in-process Sim wrapped by the explicit Lima Linux runner only
  for repository/toolchain execution (not real-kernel evidence); structured
  boot-phase telemetry is the non-KVM Lima integration name; and only
  `native_prior_vmm_reclamation_precedes_full_attachment_sweep_and_first_lease_acceptance`
  is selected by the metal runner. The native Rust body does not spawn or
  require the built product binary.
- Rust acceptance and system-conformance tests do not spawn the built product
  or capture EDD evidence. The **required rerunnable coverage** for the
  serve+deploy contract is the conformance + walking-skeleton tests — S-ND295-33
  (`tests/conformance` direct-handler fail-stop / shutdown / fresh-handler
  readiness) and S-ND295-01 (in-process walking skeleton through the real
  `run_server` composition root). These are the "what, forever" regression alarm
  and satisfy the vertical-slice rule: they drive the production composition
  root, not hand-assembled pieces.
  The `overdrive deploy <spec>` **CLI-verb** path (arg parse, spec-file load,
  exit codes) is deliberately NOT exercised by those tests — `tests/conformance`
  bans product-CLI / subprocess SUT — so it is proved by a **Tier-3 native-metal
  driving-adapter check** that spawns `overdrive deploy` (per the Driving Adapter
  Verification mandate: a handler-level test does not substitute for the CLI
  entry point). E07 is an **optional** point-in-time built-binary receipt, not a
  gate; its historical pre-cut capture is not #295 proof.
- Real kernel fixtures use `overdrive-testing`, the `host-kernel-shared`
  nextest group, named CIDR leases, exact owned cleanup, and append-only
  diagnostics. No broad sweep deletes another owner's resources.

## PBT/parameter density and shared vocabulary

- Unbounded: allocation IDs, `/16` address populations, legal operation
  sequences, detection/release schedules, listener vectors, and malformed
  byte lengths use proptest or seeded state-machine exploration at layers 1–2.
- Finite: 12 components, 6 fail-stop causes, 3 gate modes after boot, bridge
  mark cases, rollback dispositions, packet protocol classes, and active-count
  boundaries use table-driven iteration in one test body per distinct outcome.
- Layer 3+ uses explicit named cases. No proptest drives real VMMs, BPF links,
  nft state, cgroups, sockets, or external processes.
- Rust test helpers reuse domain nouns `GuestNetworkAttachment`,
  `GuestAddressLease`, `ManagedTap`, `RegistrationCapability`, `ProofMark`,
  and `ReleaseLast`; they delegate to the production composition/ports and do
  not duplicate business or kernel policy in fixtures.

The natural step-reuse ceiling is informational: the technical-only population
is used consistently — 39 prose scenarios use 17 distinct Given/When/Then fact
families, for 201 fact invocations / 17 families = **11.82×**. Rust has no decorator layer; this ratio measures repeated
domain fact/helper use in the specification. Readability is not collapsed to
raise it.

## DELIVER carry-forward notes

- **Single-cut scratch cleanup** — the transition-bundle apparatus's scratch
  trees under `.context/netns-density-295/` (`checkpoint-candidate-*`,
  `checkpoint-worktree`, `transition-worktree*`) are registered git worktrees
  (~5 GB) and are now orphaned: the one authored body they held,
  `netns_density_exec_release.rs`, is promoted into
  `crates/overdrive-worker/tests/acceptance/`. Nothing under DISTILL depends on
  them; remove with `git worktree remove` when convenient.
- **`ProofMark` / `ReleaseLast` are not production types** — they are shared
  test-helper / ubiquitous-language nouns (feature-delta § Ubiquitous language),
  used to name domain facts in step helpers only. DELIVER must NOT materialize
  them as public types, variants, or parameters.

## Executable completeness status

The bounded remediation's executable population is ten non-waived
reasoned-pending bodies: five S00 dataplane bodies, one S11 netlink projection
body, two S11 owner bodies, one S12 owner body, and the S13 seeded invariant.
The eight unconditional real-I/O panic placeholders are explicitly
user-waived: they were not rerun, do not contribute points, and are not
approval conditions for this gate.

| Check | Executable disposition for the ten non-waived bodies |
|---|---|
| C1a empty/minimum | PASS — all eight clean inventory families return exact zero; absent TAP/link projections are explicit. |
| C1b boundaries | PASS — endpoint capacity covers max-1/max/max+1 and S11 separates the first and final read-back checkpoints. |
| C2a state machine documented | PASS — the retained owner model is `Unpublished -> Provisioning -> Published -> TeardownPending -> Absent`. |
| C2b illegal event per state | PASS — every first/final observation refusal keeps publication absent; failed teardown retains state; retry and absent teardown are distinct. |
| C3 zero/one/many | PASS — zero and one receipts, unique and multiple unreceipted candidates, and two attachments are explicit. |
| C4a apply twice | PASS — S12 retries the same retained owner/allocation, then repeats teardown in `Absent`. |
| C4b inverse without prerequisite | PASS — teardown of a never-published allocation is an asserted no-op. |
| C5a mode combinations | N/A — D12/D12A/D13 define no independent product-mode dimension for these source-local and Sim contracts. |
| C5b orthogonality | N/A — C5a has no independent mode flag; independent-domain continuation and unrelated byte equality remain bounded-change complement assertions under C2/C7. |
| C6a malformed input | PASS — both accepted schemas are paired with wrong kind/key/value/capacity; raw projection covers absent, TAP with exact/missing owner, TUN, dummy, veth, other, and correct/wrong-kind bridge identities. |
| C6b each declared error | PASS — simultaneous program/link failure retains outer `Program` with the exact earlier nested `IOError(EIO)` rather than the later link-enumeration `IOError(ENOENT)`, while link observation is source-less `CaptureUnavailable`; every cleanup leaf row asserts its exact operation/lower source and continuation. |
| C6c closed error set | PASS — all eight inventory families and the exact Tap/BridgeLinkIdentity/LinkMaster fact variants are finite tables. |
| C7a degraded resource | PASS — independent and simultaneous maps/programs/links capture failures plus every cleanup-leaf failure and the dual-failure precedence row are executable. |
| C7b interruption | PASS — the seeded S13 production-helper schedule reproducibly observes sweep before reclamation and prints/shrinks its seed. |
| C7c concurrent actors | N/A — these accepted bodies specify one owner and no independent concurrent-actor interleaving. |

The executable audit is **COMPLETE — 12 PASS + 3 justified N/A = 15/15
dispositions, roadmap validation pending**. Every non-waived body was invoked
independently with `--ignored`: nine fail on their exact missing D12/D12A
behavior, while S13 reaches the actual sweep call and shrinks to replay seed
`0`. Linux Docker compilation succeeded for the affected dataplane, netlink,
control-plane, and Sim targets. Native metal remains an honest environment
gate because `OVERDRIVE_METAL_TARGET` is unset. Lima execution was unavailable:
the `overdrive` VM failed to restore SSH after restart, so Docker evidence makes
no Lima or native-kernel execution claim.
