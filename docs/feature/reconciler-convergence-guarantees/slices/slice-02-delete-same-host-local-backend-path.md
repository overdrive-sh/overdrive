# Slice 02 — delete-same-host-local-backend-path

Decisions: D-9 (ADR-0283), D-10 and D-11 (ADR-0284), D-13 (persisted route tag). Persona: Ana. Job: J-OPS-003. Order: first in DELIVER (it depends on nothing).

## Goal

The ADR-0053 same-host local-backend path is deleted. It has no production producer (ADR-0283), so
its latch (`last_applied_local_fingerprint`) goes with it. The remote service path and the public
ingress gateway are unchanged.

## IN (deletion inventory; the crafter confirms each item by grep before deleting)

- BPF programs `cgroup_connect4_service`, `cgroup_sendmsg4_service`, `cgroup_recvmsg4_service`;
  `shared/build_local_service_key.rs`; their entries in `programs/mod.rs`.
- BPF maps `LOCAL_BACKEND_MAP`, `REVERSE_LOCAL_MAP`, `reverse_local_miss_counter`, and their
  `pub mod` lines and comments in `overdrive-bpf/src/maps/mod.rs`.
- Userspace handles `local_backend_map_handle.rs`, `reverse_local_map_handle.rs`.
- `EbpfDataplane`: the three program loads, attaches and link fields; the map fields and probes
  (`LocalBackendProbe`, `ReverseLocalProbe`); the read-back and test seams for the two maps.
- `overdrive-core`: `Dataplane::register_local_backend` / `deregister_local_backend` and the error
  variants only they use; `Action::RegisterLocalBackend` / `DeregisterLocalBackend`; their shim
  executors (`action_shim/register_local_backend.rs`, `action_shim/deregister_local_backend.rs`) and
  action validation (`action_shim/validate.rs`: the cgroup-route key tracking and `WriteRoute::Cgroup`).
- `DataplaneError` variants (`overdrive-core/src/traits/dataplane.rs`) that go: `LocalBackendInsert`,
  `LocalBackendDelete`, `LocalBackendProbe`, `ReverseLocalProbe` and `CgroupSendRecvAttach`, and in
  `overdrive-dataplane/src/lib.rs` the `cgroup_attach_error` helper that builds the last one (its only
  callers attach `cgroup_sendmsg4_service` / `cgroup_recvmsg4_service`) with its unit test. These STAY:
  `cgroup_load_error` and `LoadFailed` (the gateway load, `cgroup_connect4_gateway`, uses them; its
  unit test names the deleted `cgroup_sendmsg4_service` as an example hook and is re-pointed to the
  gateway hook). `MapAllocFailed` and the other variants stay unless the crafter finds, by grep, that
  only the two deleted maps use them.
- `ConflictRoute::Cgroup` (persisted in the rkyv `ReconcileConflictRowV1`,
  `overdrive-core/src/traits/observation_store.rs`) and `WriteRoute::Cgroup`, removed in place
  (D-13, APPROVED 2026-11-15, option A): `ConflictRoute` stays a one-variant enum, both route fields
  stay in the row, no new envelope version. Also the `Cgroup` arm of `write_route_to_conflict_route`
  (`reconciler_runtime.rs`). The schema-evolution fixture is not edited.
- Both `Dataplane` adapters (`EbpfDataplane`, `SimDataplane`) and the local-backend state in the sim.
- `ServiceMapHydrator`: the local partition, `last_applied_local_fingerprint` (View field and its
  GC), and the `host_ipv4` field and its accessor if the crafter finds no other use.
- Sim invariant `reply_source_rewrite_lockstep` and the local parts of the hydrator invariants (see
  the mixed list below for the invariant files that also cover kept paths).
- Every test that defends only the above (acceptance, integration, Tier 2/3, proptest), in the same
  commit, including those that name `ConflictRoute::Cgroup` / `WriteRoute::Cgroup`
  (`tests/acceptance/reconcile_conflict_observation.rs`, `tests/integration/reconcile_output_validator.rs`),
  and the `mod` lines that wire them: `overdrive-dataplane/tests/integration/{unconnected_udp_roundtrip,
  unconnected_udp_reply_hardening,deregister_retry_safety,local_backend_proto_connect}.rs` and
  `overdrive-control-plane/tests/integration/deregister_local_backend_dispatch.rs`, with their entries in
  `tests/integration.rs`.
- The verifier baselines of the three deleted programs and every test, tooling and placement
  entry that names a deleted item (`perf-baseline/`, `.config/nextest.toml`, `.cargo/mutants.toml`,
  `crates/overdrive-testing/bin/fwd_guest_suite.rs`, `infra/metal/test-placement.json`, comments in
  `overdrive-dataplane/bin/verifier_budget.rs`): found by the AC5 grep. `test-placement.json` is
  hand-maintained and no gate flags a stale entry, so the grep is the only check.

### Mixed: edit, do not delete (cover both a deleted and a kept path)

Each of these is re-pointed to the kept path; deleting the file would drop coverage of the kept path.
Other edits to kept files (trait mocks, match arms, comments) are forced by the compiler or found by
the AC5 grep.

- `overdrive-dataplane/tests/integration/public_ingress_gateway_hook.rs`: `unregistered_cgroup_socket_keeps_non_service_behaviour`
  (the gateway hook test) registers a local backend through `Dataplane::register_local_backend`
  and asserts `LOCAL_BACKEND_MAP`. Keep its subject (an unregistered socket keeps non-service behaviour
  under the gateway hook); drop the local-backend registration and map assertions and keep the
  closed-port refusal and XDP-unchanged assertions. The other tests in the file stay.
- `overdrive-core/tests/mesh_backend_lb_gate.rs`: keep the mesh and remote-arm tests; delete the
  host-address (local-arm) tests, the local churn test and the local arm of the three-way split test,
  which becomes a two-way split (mesh versus remote).
- `overdrive-core/tests/acceptance/service_map_hydrator_reconcile.rs`: keep the remote-path fingerprint
  and backoff tests; delete `hydrator_skips_register_local_backend_for_loopback` and
  `hydrator_threads_per_listener_proto_into_register_local_backend`.
- `overdrive-sim/src/invariants/service_map_hydrator.rs` and `invariants/mod.rs`: remove the local-arm
  handling and the `reply_source_rewrite_lockstep` registration; the remote-path hydrator invariants stay.

### ADR-0164 datagram clause and startup check (D-10, ADR-0284)

- The owner's cgroup-placement startup check, in `overdrive-control-plane`: the
  `OwnerCgroupPlacement` error and the startup probe branch that raises it (`guest_network.rs`), the
  owner-cgroup host fact with its host reader and sim default (`guest_network/flow_io*`), the sim
  adapter's scripted refusal (`overdrive-sim/src/adapters/guest_network.rs`), the owner-acceptance
  error arms, and the tests and header text that defend it.
- The owner's datagram-to-VIP rewrite handling in `guest_network/owner_runtime.rs` `datagram_associate`:
  the VIP branch that skips the route-type check and sends through an unmarked socket, the VIP term of
  the egress-mark decision (it reduces to the `gateway:53` exemption alone), and the kernel-peer check
  after the connect, which only caught a `connect4` rewrite into the guest prefix, so it is redundant
  with the static destination policy once nothing rewrites (ADR-0284 records why no remaining program
  can rewrite an owner socket), with the `GuestFlowIo::peer_addr` port method and its impls if no other
  caller exists. `OwnerRuntime.vip_range` and its constructor parameter STAY: the refusal below reads
  them. The ports' `vip_range` stays too: the boot overlap check and the TCP VIP branch (D24a) read it.
- The sim `connect4` rewrite table and its test seam (`flow_io/sim.rs`), and the owner-acceptance test
  that defends the rewrite.
- Stale comments to correct, not delete: the UDP-skips-VIP-branch wording in
  `mtls_resolve_adapter.rs`. UDP still skips that arm; the reason is no longer a `connect4` rewrite.

### The one added behaviour (D-11, ADR-0284)

A guest UDP datagram whose destination is in the configured VIP ranges is refused with the existing
`FlowRefusal::Policy` before any host socket is created or connected, until GH #310 delivers guest UDP
to a VIP. The exact contract is the feature delta's D-11. This is the only behaviour the slice adds;
everything else is deletion.

## OUT

- `cgroup_connect4_gateway` and every map it reads (`GATEWAY_CONNECT_INTENT_MAP`,
  `GATEWAY_SELECTION_RECEIPT_MAP`, `SERVICE_MAP`, `BACKEND_MAP`).
- The remote path (`DataplaneUpdateService`, `service_hydration_results`).
- The attach-cgroup operator setting (ADR-0053 § 7): the gateway still attaches there.
- ADR-0164's TCP-as-mesh decision (D24a): the resolver's VIP branch for TCP, the VIP range input to
  `ServiceBackendsResolve`, and the boot check that the VIP range does not overlap the guest prefix.
- Delivering guest UDP to a VIP: GH #310.

## Gateway attach dependency (flag for the crafter)

The gateway program's load and attach code does not call the service programs' code, but in
`EbpfDataplane::new` it reuses the cgroup directory `File` opened for `cgroup_connect4_service`
(`overdrive-dataplane/src/lib.rs`, opened before the service attach, reused at the gateway attach)
and the `cgroup_attach_path` field. The deletion must keep the open of the attach cgroup and the
`cgroup_attach_path` field, and keep the gateway attach and its failure mode. Its Earned-Trust
coverage must not be lost with the deleted probes: a failed gateway attach still refuses boot.

The owner's placement check is not part of this dependency. It only reads the owner's cgroup path
and returns an error; it opens, attaches and probes nothing, and the gateway's attach does not consult
it (ADR-0284).

## Acceptance criteria

- AC1 Production-drivable: through `overdrive serve` + `overdrive deploy`, a VM Service deploys and
  serves through its existing paths after the deletion, and the public ingress gateway example still
  works.
- AC2 The boot no longer loads or attaches the three programs and no longer creates the two maps.
  The observable (`bpftool prog show`, the cgroup link list) is chosen by DISTILL.
- AC3 The verifier-regress baseline no longer lists the three programs, and the BPF object shrinks.
- AC4 The hydrator's remaining behaviour (remote partition, retry memory) is unchanged. The mutation
  gate applies to the touched hydrator logic only; pure deletions have nothing to mutate.
- AC5 No reference to the deleted names remains in `crates/`, `.config/nextest.toml`,
  `.cargo/mutants.toml`, `infra/metal/test-placement.json` or `perf-baseline/`, except history in Git.
  The existing `reconcile_conflict_row` V1 golden fixture
  (`crates/overdrive-core/tests/schema_evolution/reconcile_conflict_row.rs`, which uses `Xdp`) still
  decodes unchanged, unedited; that proves removing `Cgroup` (D-13) did not shift `Xdp`'s archived
  tag. If it fails, the crafter stops and surfaces it to the user; it does not mint an envelope
  version silently.
- AC6 Guest TCP to a VIP still resolves as mesh (ADR-0164 D24a, unchanged), and the boot no longer
  has an owner cgroup-placement refusal. The guest-flow owner has no datagram-to-VIP case that
  depends on a `connect4` rewrite.
- AC7 (D-11) A guest UDP datagram to a destination in the configured VIP ranges is refused
  `FlowRefusal::Policy` before any host socket is created or connected. Tier 1: driven through the
  owner's driving port with the sim flow I/O; the oracle is the `Policy` refusal and that the sim
  records no socket creation, no `connect` and no send. A destination outside the VIP ranges still
  takes the generic path, and TCP to a VIP is unchanged.

## Evidence (not coverage)

Metal observation of the never-working datagram clause: ADR-0284 § Context. It covers no property;
the deleted tests are not replaced by it.

## Dependencies

None. It is scheduled first because it removes code the other slices would otherwise touch.

## Effort

About 1 day of deletion plus review of the shared constructor.

## Production-drivable check

The deletion removes a path no `serve` + `deploy` reaches. The slice is verified by the unchanged
behaviour of the paths that are reached (AC1), not by exercising the removed path.

## DELIVER follow-ups outside `crates/` (listed here; not edited by DESIGN)

- Rules docs, to update in the deletion commit (DESIGN edits neither rules nor website): `.claude/rules/debugging.md` § 11 uses
  `LOCAL_BACKEND_MAP` as the single-node local example (replace the example); `.claude/rules/development.md`
  § "`bpf_sock_addr.user_port`" cites the ADR-0053 walking skeleton (the rule stays valid for the gateway's
  connect4; change only the citation and example).
- `.claude/rules/reconcilers.md` carries a "still live in tree" bullet about
  `ServiceLifecycleView::last_emitted_backend_fingerprint`. ADR-0101 deleted that field, so the bullet
  is stale; correct it together with the new View rule (slice 04).
- Living documents that describe the path as live, to update in the deletion commit:
  `docs/product/architecture/brief.md` and `c4-diagrams.md` (the supersession banners and the
  `Service connect4` container), and the journeys
  `docs/product/journeys/reach-an-unconnected-udp-service.yaml` and `submit-a-udp-service.yaml`
  (they describe the unconnected-UDP path through `sendmsg4` / `recvmsg4`). Accepted ADRs are
  immutable and are not edited; ADR-0283 lists the ones that mention the path and supersedes it.
- Website: `website/content/blog/what-is-ebpf.mdx` mentions the path (2 hits); review at FINALIZE.
