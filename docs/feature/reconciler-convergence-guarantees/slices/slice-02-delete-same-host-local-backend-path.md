# Slice 02 — delete-same-host-local-backend-path

Decision: D-9 (ADR-0283). Persona: Ana. Job: J-OPS-003. Order: first in DELIVER (it depends on nothing).

## Goal

The ADR-0053 same-host local-backend path is deleted. It has no production producer (ADR-0283), so
its latch (`last_applied_local_fingerprint`) goes with it. The remote service path and the public
ingress gateway are unchanged.

## IN (deletion inventory; the crafter confirms each item by grep before deleting)

- BPF programs `cgroup_connect4_service`, `cgroup_sendmsg4_service`, `cgroup_recvmsg4_service`;
  `shared/build_local_service_key.rs`; their entries in `programs/mod.rs`.
- BPF maps `LOCAL_BACKEND_MAP`, `REVERSE_LOCAL_MAP`, `reverse_local_miss_counter`.
- Userspace handles `local_backend_map_handle.rs`, `reverse_local_map_handle.rs`.
- `EbpfDataplane`: the three program loads, attaches and link fields; the map fields and probes
  (`LocalBackendProbe`, `ReverseLocalProbe`); the read-back and test seams for the two maps.
- `overdrive-core`: `Dataplane::register_local_backend` / `deregister_local_backend` and the error
  variants only they use; `Action::RegisterLocalBackend` / `DeregisterLocalBackend`; their shim
  executors and action validation.
- Both `Dataplane` adapters (`EbpfDataplane`, `SimDataplane`) and the local-backend state in the sim.
- `ServiceMapHydrator`: the local partition, `last_applied_local_fingerprint` (View field and its
  GC), and the `host_ipv4` field and its accessor if the crafter finds no other use.
- Sim invariant `reply_source_rewrite_lockstep` and the local parts of the hydrator invariants.
- Every test that defends the above (acceptance, integration, Tier 2/3, proptest), in the same commit.
- Verifier baselines of the three deleted programs under `perf-baseline/main/verifier-budget/`.

### ADR-0164 datagram clause and startup check (D-10, ADR-0283)

- The owner's cgroup-placement startup check, in `overdrive-control-plane`:
  `GuestNetworkError::OwnerCgroupPlacement` and its message (`guest_network.rs`); the
  `attach_point` / `starts_with` branch in `probe_guest_flow_startup` (`guest_network.rs`), and the
  `OwnerCgroupPlacement` mention in its doc comment and in `probe_startup`'s; `GuestHostFacts.owner_cgroup`
  (`flow_io.rs`), its host reader in `flow_io/host.rs` (and the cgroup-path helper if nothing else calls
  it) and its sim default in `flow_io/sim.rs`. The scripted sim refusal and its test in
  `overdrive-sim/src/adapters/guest_network.rs`; the `OwnerCgroupPlacement` arms in
  `owner_acceptance/errors.rs` (label and the `/sys/fs/cgroup` refusal case); the test
  `an_owner_outside_the_connect4_attach_path_refuses_before_any_boot_effect`
  (`prefix_boot_acceptance.rs`). The `shared_guest_network_startup.rs` header paragraph on
  `OwnerCgroupPlacement` goes.
- The owner's datagram-to-VIP handling in `owner_runtime.rs` `datagram_associate`: the
  `runtime_vip_contains` branch that skips the route-type check, the VIP term of `owner_egress_mark`
  (it reduces to the `gateway:53` exemption alone), and `runtime_vip_contains` itself; the kernel-peer
  check after `connect_udp`, which only caught a `connect4` rewrite into the guest prefix, so it is
  redundant with the static destination policy once nothing rewrites; the `GuestFlowIo::peer_addr`
  port method and its host and sim impls if no other caller exists; `OwnerRuntime.vip_range`
  (`flow_owner.rs`) and the constructor parameter at its call sites if `runtime_vip_contains` is its
  only reader. The ports' `vip_range` stays: the boot overlap check and the TCP VIP branch (D24a) read it.
- The sim `connect4` rewrite table and `set_connect4_rewrite` (`flow_io/sim.rs`), and the test
  `a_vip_association_is_rewritten_by_connect4_and_checked_on_its_kernel_peer`
  (`owner_acceptance/datagrams.rs`).
- Stale comments to correct, not delete: the UDP-skips-VIP-branch wording in
  `mtls_resolve_adapter.rs` (module doc, `classify` doc, arm 2). UDP still skips arm 2; the reason is
  no longer a `connect4` rewrite.
- The crafter confirms each item by grep. What happens to a guest UDP datagram to a VIP-range
  address once the VIP branch is gone is an open question for the user (feature delta, Open
  questions); the crafter does not choose.

## OUT

- `cgroup_connect4_gateway` and every map it reads (`GATEWAY_CONNECT_INTENT_MAP`,
  `GATEWAY_SELECTION_RECEIPT_MAP`, `SERVICE_MAP`, `BACKEND_MAP`).
- The remote path (`DataplaneUpdateService`, `service_hydration_results`).
- The attach-cgroup operator setting (ADR-0053 § 7): the gateway still attaches there.
- ADR-0164's TCP-as-mesh decision (D24a): the resolver's VIP branch for TCP, the VIP range input to
  `ServiceBackendsResolve`, and the boot check that the VIP range does not overlap the guest prefix.
- Guest UDP to a VIP stays unsupported until GH #310 (UDP service delivery to VM workloads; requires
  kernel-resident forwarding, no userspace relay). Not worked in this slice.

## Gateway attach dependency (flag for the crafter)

The gateway program's load and attach code does not call the service programs' code, but in
`EbpfDataplane::new` it reuses the cgroup directory `File` opened for `cgroup_connect4_service`
(`overdrive-dataplane/src/lib.rs`, opened before the service attach, reused at the gateway attach)
and the `cgroup_attach_path` field. The deletion must keep the open of the attach cgroup and the
`cgroup_attach_path` field, and keep the gateway attach and its failure mode. Its Earned-Trust
coverage must not be lost with the deleted probes: a failed gateway attach still refuses boot.

The owner's placement check (ADR-0164) is not part of this dependency. It only reads the owner's
cgroup path and returns an error; it opens, attaches and probes nothing, and the gateway's
attach does not consult it. `run_server` places the process into `overdrive.slice/control-plane.slice`
(`create_and_enrol_control_plane_slice`) independently, before the check runs. Deleting the check
leaves the gateway's attach, its failure mode and its process placement unchanged.

## Acceptance criteria

- AC1 Production-drivable: through `overdrive serve` + `overdrive deploy`, a VM Service deploys and
  serves through its existing paths after the deletion, and the public ingress gateway example still
  works.
- AC2 The boot no longer loads or attaches the three programs and no longer creates the two maps.
  The observable (`bpftool prog show`, the cgroup link list) is chosen by DISTILL.
- AC3 The verifier-regress baseline no longer lists the three programs, and the BPF object shrinks.
- AC4 The hydrator's remaining behaviour (remote partition, retry memory) is unchanged. The mutation
  gate applies to the touched hydrator logic only; pure deletions have nothing to mutate.
- AC5 No reference to the deleted names remains in `crates/`, except history in Git.
- AC6 Guest TCP to a VIP still resolves as mesh (ADR-0164 D24a, unchanged), and the boot no longer
  has an owner cgroup-placement refusal. The guest-flow owner has no datagram-to-VIP case that
  depends on a `connect4` rewrite.

## Evidence (not coverage)

Metal reproduction, Linux 7.0.0-38, commit `fb71b400f`, three runs
(`spike-scratch/guest-udp-vip-probe/evidence/run{7,8,9}/`): for a Running UDP VM Service,
`LOCAL_BACKEND_MAP` has 0 entries while `SERVICE_MAP` holds the VIP's TCP and UDP keys; guest UDP to
the VIP gets no reply in 8 of 8 attempts; guest TCP to the same VIP and port works; `getpeername` on
the owner's connected host UDP socket after `connect` is still the VIP. It supports the claim that
ADR-0164's datagram clause never worked for VM backends. It is evidence for that claim and covers no
property: the deleted tests are not replaced by it.

## Dependencies

None. It is scheduled first because it removes code the other slices would otherwise touch.

## Effort

About 1 day of deletion plus review of the shared constructor.

## Production-drivable check

The deletion removes a path no `serve` + `deploy` reaches. The slice is verified by the unchanged
behaviour of the paths that are reached (AC1), not by exercising the removed path.

## DELIVER follow-ups outside `crates/` (listed here; not edited by DESIGN)

- Rules docs, to update in the deletion commit: `.claude/rules/debugging.md` § 11 uses
  `LOCAL_BACKEND_MAP` as the single-node local example (replace the example); `.claude/rules/development.md`
  § "`bpf_sock_addr.user_port`" cites the ADR-0053 walking skeleton (the rule stays valid for the gateway's
  connect4; change only the citation and example).
- Website: `website/content/blog/what-is-ebpf.mdx` mentions the path (2 hits); review at FINALIZE.
