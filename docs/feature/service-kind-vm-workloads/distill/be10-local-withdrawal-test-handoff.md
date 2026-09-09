# BE10 local direct-VIP withdrawal — bounded DISTILL handoff

Date: 2026-09-08. Owner: Codex acceptance designer.
Authority: ADR-0101 revision 4 D7 and
[independently approved DESIGN](../design/review-amendment-be10-local-backend-withdrawal.md).
This handoff is not implementation approval, a DES event or a commit.

## Scope and existing coverage

The approved amendment selects existing registration for healthy and existing
deregistration for a still-present unhealthy local candidate. The current
fingerprint gate, remote plan/retry state, mesh exclusion, classifier, keyed
listener port/protocol, exact correlation and public ports stay intact.
BE02's independently approved singleton correction and the four completed
startup fixture alignments are unchanged.

BE10 `existing_consumers_follow_withdrawal_and_recovery_asynchronously` remains
byte-for-byte unchanged: seed 257221 drives a healthy baseline, real
ProbeRunner readiness Fail, authoritative unhealthy row, mesh/DNS withdrawal,
queued ServiceMapHydrator and SimDataplane, then expects local removal and
recovery. It remains the composed reachability/convergence evidence. The pure
properties below do not author production observation rows or claim that their
constructed input is a reachable production schedule.

| Existing evidence | Coverage / disposition |
|---|---|
| BE10 seed257221 | Reuse unchanged; implements the required successful-effect health transition and recovery oracle. Recovery remains unexecuted until GREEN. |
| `mesh_backend_lb_gate::three_way_split_routes_each_address_class_to_exactly_one_disposition` | Retain unchanged; generated healthy local/remote/mesh classification and remote fingerprints. It does not cover unhealthy local deregistration. |
| `service_map_hydrator::tests::push_register_local_backend_*` (three controls) | Retain healthy exact action fields, unequal VIP/backend ports, and IPv6/loopback rejection. Private-helper call rename is compiler fallout; exact test-only patch supplied separately. |
| `deregister_local_backend_dispatch_removes_entry_from_dataplane` | Reuse unchanged; existing typed removal shim actually removes a previously registered Sim forward entry. |
| `deregister_retry_safety::deregister_purges_reverse_entry_when_forward_already_removed` | Retain existing real-kernel caller-keyed reverse-removal evidence; not run here. This is port idempotence, not a new hydrator retry promise. |
| `reverse_local_map_handle::…::remove_deletes_the_reverse_entry` and `local_backend_proto_connect::tcp_and_udp_connect_to_same_vip_port_reach_proto_correct_backend` | Retain existing real-map removal and TCP/UDP key-isolation controls; not run here. No new kernel suite or map layout. |

The existing tests leave one precise pure-reconcile gap: unhealthy action
selection with exact purpose/tuple and unchanged remote/View output. Added
one paired property in the existing `mesh_backend_lb_gate.rs` test binary:

`local_health_selects_existing_action_preserving_remote_and_view`

- Exact `/// CONTRACT_SHAPE: pure-function.` declaration.
- Fixed proptest seed 257228, 128 configured generated port-pair cases.
- Each case traverses local/remote/mesh × TCP/UDP × healthy/unhealthy.
- Whole ordered action-vector equality checks the remote action first,
  exact local register/deregister variant, ServiceId, allocator-domain VIP,
  independent listener/backend ports, protocol, backend tuple and exact
  full-content-derived correlation purpose.
- Whole View equality checks both maps, including remote retry inputs and
  the full unhealthy local fingerprint. A same-input/time repeat checks
  empty actions and complete unchanged View; this is the existing emission
  gate, not acknowledgement or failed-effect repair.
- One backend per Service pure input; no multi-replica arbitration, new
  state machine, deletion/GC family or recovery protocol.

## Fresh pre-implementation RED and controls

The production correction had not started for these runs. The current helper
was still `push_register_local_backend_actions` and unconditionally registered.
The fresh BE10 RED preceded the added property and test-only patch preparation.

```text
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(existing_consumers_follow_withdrawal_and_recovery_asynchronously)' --no-capture
```

Run `4d38274f-380a-45f1-a295-c61afc223a3b`: **0 passed, 1 failed, 35 skipped**;
inner nextest 100 / outer xtask 1. Seed **257221** printed. At
`service_backend_projection.rs:1019`, observed `Some(192.0.2.10:18081)` versus
expected `None`. Healthy registration, actual readiness failure, unhealthy row
and mesh/DNS withdrawal passed before that assertion. Recovery after it did
not execute. This reproduces the earlier retained RED, not a setup failure.

```text
cargo xtask lima run -- cargo nextest run -p overdrive-core --features integration-tests --test mesh_backend_lb_gate -E 'test(local_health_selects_existing_action_preserving_remote_and_view) | test(three_way_split_routes_each_address_class_to_exactly_one_disposition)' --no-capture --no-fail-fast
```

Run `90bab83e-71ab-4690-958b-cafb892e8c40`: **1 passed, 1 failed, 8 skipped**;
inner 100 / outer 1. Existing healthy classification property passed. New
property reached healthy local/TCP whole-output/View/repeat checks, then failed
at unhealthy local/TCP action equality: actual RegisterLocalBackend versus
expected DeregisterLocalBackend, with the corresponding wrong purpose.
Shrunk ports: listener=1, backend=1; zero successful complete generated cases.
Do not claim that its subsequent unhealthy View, UDP, remote/mesh or all 128
configured cases executed. Proptest saved regression
`db817f77def559bab6b209dcd17d286417bff50cc890aeca3562161efebb6453` in the paired
`.proptest-regressions` file; that generated evidence is retained.

```text
cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers -p overdrive-control-plane --features integration-tests --lib --test integration -E 'test(push_register_local_backend) | test(deregister_local_backend_dispatch_removes_entry_from_dataplane)' --no-capture --no-fail-fast
```

Run `1d42dfc0-a765-492f-9349-d237a53e740e`: **4 passed, 468 skipped**, exit 0.
These are the three existing helper controls and the existing Sim removal
shim control. Compilation exposed one unused `NodeId` import left by the
earlier bridge-test retirement in `reconciler_runtime_view_store.rs`; only
that import was removed as necessary test-transfer cleanup. This run compiled
before that cleanup; a warning-free recompilation is not claimed here.

## Exact test-only patch and owned files

`.context/be10-hydrator-test-transfer.patch` is an exact apply_patch payload for
the crafter, **not applied by the acceptance designer**. It changes only three
existing cfg(test) calls to the approved private `push_local_backend_actions`
name and adds each control's required pure-function Contract Shape. Apply
alongside the crafter's approved production helper rename. No public seam or
production line is in the patch; helper implementation and its production
call remain crafter-owned. The `.context` patch is an ignored handoff artifact,
not a required commit file.

Files authored/changed in this bounded work:

- `crates/overdrive-core/tests/mesh_backend_lb_gate.rs`
- `crates/overdrive-core/tests/mesh_backend_lb_gate.proptest-regressions` (generated RED seed)
- `crates/overdrive-control-plane/tests/integration/reconciler_runtime_view_store.rs` (unused import only)
- `docs/feature/service-kind-vm-workloads/distill/be10-local-withdrawal-test-handoff.md`
- `docs/feature/service-kind-vm-workloads/distill/adr-0101-acceptance.md` (BE10 handoff reference only)
- `docs/feature/service-kind-vm-workloads/deliver/adr-0101-test-transfer.md` (later handoff reference only)
- `.context/be10-hydrator-test-transfer.patch`

## Downstream obligations and limits

After authorized production implementation, rerun the same focused commands.
BE10 must reach recovery; the paired property must execute every configured
case and finite combination, and the retained helper/port controls must pass.
Keep original seed257209 and native E09 evidence independent. No test claim
conflates a map miss with denied connection or established-flow revocation.

Native unhealthy direct-VIP routing is still unexecuted. Any later authorized
native claim needs the existing production dispatch/attached-cgroup boundary,
healthy control and a new connection after withdrawal. The retained port tests
are not that composed native proof. No test here adds map-drift repair,
failed-effect retry, historical membership cleanup or replica scheduling.

No production, ADR/ruling/roadmap/review, DES event or commit was changed.
No native, mutation or whole-suite run occurred. Formatting and
`git diff --check` passed. No testability/API blocker remains for this bounded
handoff; revised test material still requires the independent implementation
review. Roadmap approval and production continuation are orchestrator-owned.
