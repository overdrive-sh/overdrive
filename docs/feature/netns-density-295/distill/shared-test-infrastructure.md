# DISTILL — shared test infrastructure (netns-density-295, vsock replacement)

SSOT for the test-support surface every piece shares, so the six parallel
piece-authors do not each invent it. Authored by the DISTILL planner.
Binding rule for this project: **no `.feature` files** — scenarios are
specified as GIVEN/WHEN/THEN in the per-piece `test-scenarios-*.md` docs
(spec only, never parsed), and the crafter/author translates each into a Rust
`#[test]` / `#[tokio::test]` body under `crates/{crate}/tests/{acceptance,integration}/*.rs`
or `tests/conformance/tests/*.rs` (`.claude/rules/testing.md` §"Testing").

The design handed to DISTILL is `docs/feature/netns-density-295/feature-delta.md`
§"Wave: DESIGN / [REF] vsock Attachment Replacement DESIGN" (L16552+) and
ADR-0145..0181. The Quint oracle is `specs/quint/guest-flow-owner/`.

---

## 0. Load-bearing sequencing constraint (read first)

The replacement production surface (`SharedGuestNetworkOwner::activate`,
`GuestAddressPool` CID-claim, the `guest_flow` host forwarder, `GuestPrefixSteering`,
the CH-fork `fd=` backend, guest `overdrive-init` programs, the wire codecs)
**does not exist yet**. DELIVER is stopped at the old-model step 08-04, the
**roadmap reconciliation is the first DELIVER activity** (feature-delta
L16554-16558, user decision 2026-10-08), and the old attachment-model code +
its GREEN tests are still in tree.

Consequence for DISTILL authoring:

- A Rust acceptance body only compiles once the production API it calls exists.
  Per CLAUDE.md §"Implement to the design — never invent API surface" and
  `.claude/rules/testing.md` §"RED acceptance scaffolds and activation",
  authors MUST NOT invent production surface to make a body compile. The
  production scaffolds (exact API shapes the feature-delta §"[REF] Interface
  contracts" pins) are created during DELIVER roadmap reconciliation / step 1,
  not by the DISTILL planner.
- Therefore each piece's **scenario-spec doc is authorable now** (the real
  DISTILL product); the **compiling Rust RED scaffolds + harness + quint-connect
  drivers are gated** on that reconciliation. This is recorded in `blocked`.
- All contract-shaping validations (V-10, V-22, V-24, V-25, V-26, V-5(c),
  V-13, V-16, V-17, V-18, V-20, V-21) have **run** (P-37..P-48). **Nothing is
  blocked on V-22/V-24/V-25/V-26** — their remediation decisions (ATTR-TOKEN,
  ATTR-LIVE-CLAIM, CLAIM-PINS, STEER-PINS, HALFCLOSE-SIGNAL, CELL-RECREATE,
  USO-NO-SEGMENT, IFACE-NO-STACK, FRAG-MAP-LIFECYCLE, D26-PINS) ARE the design.
- `V-6, V-8, V-9, V-19, V-1(c), V-12, V-28` are DELIVER-step validations;
  scenarios resting on them are authored as RED bodies marked
  `#[ignore = "pending DELIVER step NN-NN"]` (normal RED-scaffold convention,
  `.claude/rules/testing.md`), not "blocked".

---

## 1. Mandate-12 / domain-vocabulary (Rust adaptation)

The Python "domain_types.py" pilot maps, in this Rust codebase, to the
**`overdrive-core` newtype vocabulary** — the design already pins it in
feature-delta §"[REF] Core vocabulary — overdrive-core (D2, D3, D5, D8, D8a-CAP, D16)":
`GuestCid`, `LaunchToken`, `FlowId`, `GuestAddressLease`, `WorkloadAddr`,
the flow/control/datagram wire types, `GuestNetworkError` taxonomy, the
intake / steering capacity constants. Test bodies invoke production services
(the owner, the pool, the forwarder port) and never inline business logic.
No `domain_types.py`, no `pytest-bdd`, no `.feature` files. Newtype
completeness (Display/FromStr/serde/Arbitrary roundtrip) is a mandatory
proptest call site (`.claude/rules/testing.md` §"Property-based testing").

---

## 2. Production composition harness (shared) — `tests/conformance`

The walking skeleton, the cross-owner crash matrix, and every quint-connect
conformance driver drive the **production composition** through its public
entry points (`overdrive serve` boot via `run_server_with_obs_and_drivers`,
and `overdrive deploy`), with only external / non-deterministic ports faked
(Pillar 3). The root `overdrive-system-conformance` crate (`tests/conformance/`)
already exists and depends on control-plane + core + netlink + sim + worker.

Shared builder to add (shared layer, NOT owned by a single piece —
`tests/conformance/src/lib.rs` + `tests/conformance/src/vsock_harness.rs`):

- `fn boot_vsock_node(cfg) -> ConformanceNode` — boots the server composition
  with the vsock transport wired, injecting `Sim*` at the external/
  non-deterministic driven ports (clock, entropy, VMM, LLM, CA) and real
  adapters for internal driven ports per the Project Infrastructure Policy
  (`docs/architecture/atdd-infrastructure-policy.md`).
- `ConformanceNode` exposes only the observable surface the oracles read
  (public API, `alloc_status`, exported handler, kernel-query helpers such as
  `ss`, `bpftool net/link/map`, `ip route`) — never private owner fields
  (Quint-conformance rule: state projected from the observable surface,
  `.claude/rules/testing.md` §"Quint conformance").

Existing `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs`
is an OLD-model fail-stop test — retired by the DELIVER roadmap reconciliation
alongside the old production code (see §7); its replacement is P6
`vsock_crash_matrix.rs`.

---

## 3. Sim adapters (shared) — `overdrive-sim/src/adapters`

The seeded-sim lane drives the production owner path through the private seam
with `Sim*` adapters at the driven ports (`.claude/rules/testing.md` §"Tier 1").
The vsock surface needs these Sim adapters. Unless owned by a piece below they
belong to the **shared layer** and the first piece to need one creates it,
announcing it here:

| Sim adapter | Models | Owner |
|---|---|---|
| `SimGuestCidClaim` | host-kernel CID holder table; holders outside the pool's leases (R5-19) | **P1** (CID-centric) |
| `SimGuestFlowForwarder` | `overdrive-dataplane::guest_flow` forwarder port (install/route/abort/drain) | shared (first of P2/P3/P6) |
| `SimGuestPrefixSteering` | `GuestPrefixSteering` load/attach/swap/steer/close with a capacity bound | shared (first of P4) |
| `SimVmmVsock` | CH-fork `backend=vhost-kernel` probe + `fd=` launch + pre-READY exit | shared (extend `adapters/vmm.rs`; first of P1/P6) |

Every Sim adapter MUST enforce the same input preconditions the real adapter
rejects (`.claude/rules/tdd-methodology`, "test doubles must validate inputs";
`.claude/rules/development.md` §"Trait definitions specify behavior"). The
existing OLD-model `adapters/guest_network.rs` and
`invariants/netns_density_boot_order.rs` are retired by the DELIVER roadmap
reconciliation (§7).

---

## 4. quint-connect conformance drivers (shared base + per-module files)

`quint-connect = "=0.1.2"` is **already** a workspace dependency (root
`Cargo.toml:227`) — no Cargo edit needed. No conformance driver exists yet.

Per `.claude/rules/testing.md` §"Quint conformance": the driver drives the
production owner through its driving ports (Sim at driven ports), projects
`State` from the owner's **observable** surface, references the spec **in
place** (`specs/quint/guest-flow-owner/<module>.qnt` — never copied), and runs
both trace modes (`#[quint_test]` for the named CI runs in `checks.toml`,
`#[quint_run]` randomized with the seed printed on failure). Conformance tests
spawn the `quint` CLI → gated behind `integration-tests`, run via
`cargo xtask lima run --`, placed with the owning crate / in `tests/conformance`.

Shared base to add (`tests/conformance/src/quint_driver.rs`): the
`quint-connect` wiring scaffold (trace ingest, per-step dispatch trait,
state-equality harness) that each module driver specialises. The eight module
drivers are owned per piece:

| Quint module (`specs/quint/guest-flow-owner/`) | Driver file | Piece |
|---|---|---|
| `cid_lease.qnt` | `tests/conformance/tests/vsock_cid_lease_conformance.rs` | P1 |
| `owner_flows.qnt` | `tests/conformance/tests/vsock_owner_flows_conformance.rs` | P2 |
| `udp_slots.qnt` | `tests/conformance/tests/vsock_udp_slots_conformance.rs` | P3 |
| `prefix_boot.qnt` | `tests/conformance/tests/vsock_prefix_boot_conformance.rs` | P4 |
| `intake.qnt` | `tests/conformance/tests/vsock_intake_conformance.rs` | P4 |
| `prefix_landing.qnt` | `tests/conformance/tests/vsock_prefix_landing_conformance.rs` | P4 |
| `quiescence.qnt` | `tests/conformance/tests/vsock_quiescence_conformance.rs` | P6 |
| `shared_table.qnt` | `tests/conformance/tests/vsock_shared_table_conformance.rs` | P6 |

Every spec invariant must have ≥1 conformance path; the per-piece scenario
doc records which named `checks.toml` run + invariant each driver replays.
**Do NOT edit `specs/quint/**`** (checks.toml is hashed into the evidence).

---

## 5. Real-kernel fixture policy (Tier-3)

The vsock model has **no per-VM netns/veth/`NetSlot`** (D1). The per-allocation
net-slot band registry (`crates/overdrive-control-plane/tests/common/net_slots.rs`)
is therefore **not** the isolation mechanism for most new Tier-3 tests.
Instead:

- **Node-global kernel state** — the host root-namespace `sk_lookup` steering
  link + pinned map, the `overdrive-mtls` / guest-prefix nft tables, the TCX
  egress unframe links on `lo` and every interface, the shared `local` route,
  the `/dev/vhost-vsock` CID holder table, process-global `AF_VSOCK` listeners
  (1234/1240/1241/1242/1243) — is shared by every vsock Tier-3 test. **Every
  new Tier-3 test binary/module that touches it joins the `host-kernel-shared`
  nextest group** (`.config/nextest.toml`, `max-threads = 1`, matched by
  MODULE — rename-proof), per `.claude/rules/testing.md` §"Node-global kernel
  state requires a catch-all nextest group". Each piece's scenario doc names
  the exact module→group entries its author adds.
- **Hand-built netns** in an egress/non-mesh test (e.g. a veth to reach an
  off-host destination, P5) MUST draw a route-disjoint CIDR via
  `overdrive_testing::cidr_lease::TestCidrLease::acquire(owner)` — never a CIDR
  literal (`.claude/rules/testing.md` §"Hand-built real-netns fixtures").
- The `serve`-created `ovd-veth-cli` / `ovd-veth-bk` pair (R5-39) is
  **production-owned** (its `NetSlotAllocator` is internal to `run_server`);
  tests observe it, they do not provision it.
- Tier-3 runs through `cargo xtask lima run --` (CI matrix) and, for the real
  microVM boot + CH-fork + `/dev/kvm` paths (P1 V-25, P6 V-1(c)/V-6), through
  `cargo xtask metal run --` on the native x86_64 KVM host
  (`.claude/rules/testing.md` §"native x86_64 bare-metal KVM host"). Never
  pipe Lima/metal runs through `tail`/`head`.

### 5.1 `host-kernel-shared` entries for the 6 Tier-3 modules (gap closed at DISTILL)

The piece-authors listed the `host-kernel-shared` membership their Tier-3
modules need (P6 §"Real-kernel fixture policy", P1 §file-ownership) but could
**not add it themselves** — `.config/nextest.toml` is a single shared file, and
six piece-authors each editing it in parallel would collide. The P6 doc records
it as a gap ("`.config/nextest.toml` is not a P6 file"). This shared-file piece
closes that gap **now** (DISTILL scaffolds), using the same "membership declared
before the body exists" discipline the `guest_stack_mtls_egress` /
`vm_walking_skeleton` entries already follow: a `binary()` / `test()` filter
matches nothing until DELIVER lands the compiling body, so declaring it early is
safe and keeps the group in place the moment the body co-arrives. The entries
match **by BINARY** (rename-proof) per `.claude/rules/testing.md` §"Node-global
kernel state requires a catch-all nextest group".

Concrete `.config/nextest.toml` `profile.default` overrides → `host-kernel-shared`:

| Module (P6 gap list) | nextest filter | Status |
|---|---|---|
| `overdrive-system-conformance::vsock_walking_skeleton` | `package(overdrive-system-conformance) & binary(vsock_walking_skeleton)` | **added** (one OR-override with the other three conformance binaries) |
| `overdrive-system-conformance::vsock_crash_matrix` | `… & binary(vsock_crash_matrix)` | **added** (same OR-override) |
| `overdrive-system-conformance::vsock_quiescence_conformance` | `… & binary(vsock_quiescence_conformance)` | **added** (same OR-override) |
| `overdrive-system-conformance::vsock_shared_table_conformance` | `… & binary(vsock_shared_table_conformance)` | **added** (same OR-override) |
| `overdrive-host::integration::vsock_vmm_backend_probe` | `package(overdrive-host) & binary(integration)` | **already covered** by the existing whole-binary M4 entry — no new filter; the M4 comment is extended to name `vsock_vmm_backend_probe` so the binary-level membership survives the DELIVER retirement of `vmm_tap_queue_errors.rs` |
| `overdrive-init::integration::vsock_guest_ready_config` | `package(overdrive-init) & binary(integration)` | **added** as the WHOLE integration binary (catch-all form, matching the B5 / M4 whole-binary precedent), not a per-module `test()` filter |

Notes on the two deviations from the P6 gap list's module-granularity wording:

- **`vsock_vmm_backend_probe` needs no new entry.** It lives in
  `overdrive-host`'s `integration` binary, which the existing M4 override already
  places whole-binary in `host-kernel-shared`. Adding a redundant module filter
  would be the discouraged "module-only" form the catch-all rule warns against;
  the gap is closed by documentation (the M4 comment now names it) rather than a
  second filter.
- **`vsock_guest_ready_config` joins as the whole `overdrive-init` integration
  binary**, not a `test(vsock_guest_ready_config)` module filter. The catch-all
  rule mandates the whole package/binary pair; `overdrive-init` has no other
  `host-kernel-shared` entry, so the whole-binary form is the sole assignment
  (never module-only). `overdrive-init`'s separate `acceptance` binary
  (`vsock_guest_udp_slots.rs`, a P3 file) is intentionally left for the P3 author
  — outside this 6-module gap.

The `vsock_crash_matrix` binary replaces the retired
`shared_guest_network_fail_stop_recovery` conformance binary (§7); its existing
single-binary override is left in place until the DELIVER roadmap reconciliation
removes the old-model binary it names (removing the override now would strand an
entry for a binary still in tree). No `specs/quint/**` or Cargo files were
touched.

---

## 6. Test entrypoint + module registration contract

Each piece owns a disjoint set of **module files** (listed in its scenario
doc). When an author creates a module file, they register it in the crate's
existing inline entrypoint block (`crates/{crate}/tests/acceptance.rs` →
`mod acceptance { … }`, `crates/{crate}/tests/integration.rs` →
`#![cfg(feature = "integration-tests")] mod integration { … }`,
`tests/conformance/tests/*.rs` are standalone binaries) — adding only their own
`mod <file>;` line. Because registration + the compiling body co-arrive with
the DELIVER production scaffold the body calls, the planner does **not**
pre-create empty stubs in the committed entrypoints (that would be premature
churn before the roadmap reconciliation reshapes the step→crate→file map and
removes the old-model modules the entrypoints currently register). The
disjoint file-ownership table in each scenario doc + the plan is the
collision-avoidance SSOT the parallel authors rely on.

`overdrive-init` has no `tests/acceptance.rs` entrypoint today; the first of
P3/P6 to need guest-side Rust tests creates it (test-support surface is
DISTILL's to shape, CLAUDE.md §"Implement to the design"). Guest-only
behaviour that cannot run on the host (real guest kernel) is a Tier-3 cell
inside the P6 walking skeleton, not a standalone host test.

---

## 7. Retired by DELIVER roadmap reconciliation (NOT deleted by the planner)

The following OLD-attachment-model (TAP / shared-bridge / intercept-program)
GREEN tests defend old production code **still in tree**. Per
`.claude/rules/development.md` §"Deletion discipline" (delete production code
AND its tests in the SAME commit) and the user decision that roadmap
reconciliation is the first DELIVER activity, they are retired **in DELIVER**,
coupled with removal of the production code they defend — not by this DISTILL
planner (deleting them now would leave an uncommittable, inconsistent tree).
Cataloged here so the reconciliation does not miss them:

- `crates/overdrive-core/tests/acceptance/`: `netns_density_exec_gate`,
  `netns_density_placement_cap`, `netns_density_cleanup_pending`,
  `vm_beacon_roundtrip`
- `crates/overdrive-control-plane/tests/acceptance/`: `netns_density_guest_network`
- `crates/overdrive-worker/tests/`: `netns_density_exec_release` (acceptance),
  `shared_intercept_members`, `netns_density_shared_owner` (integration),
  `mtls_intercept_install`, `leg_listener`, `outbound_enforce_substrate_splice` (revalidate vs replace — reconciliation decides per the Contract impact matrix)
- `crates/overdrive-sim/tests/`: `netns_density_activation_order`,
  `netns_density_retiring_cleanup` (acceptance), `netns_density_node_admission`,
  `netns_density_reclaim` (integration); `src/invariants/netns_density_boot_order.rs`;
  OLD parts of `src/adapters/guest_network.rs`
- `crates/overdrive-bpf/tests/integration/guest_tcx_classifier_test_run`,
  `crates/overdrive-dataplane/tests/integration/{guest_tcx_egress_lifecycle,guest_tcx_inventory}`
- `crates/overdrive-host/tests/integration/vmm_tap_queue_errors`
- `crates/overdrive-netlink/tests/integration/{tap_queue_attach,bridge_guard_lifecycle,managed_link_address,tap_debug_msg_mask}`
- `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs`

The Contract-impact matrix (feature-delta §"Contract impact matrix") classes
each prior ADR Replaced / Retained / Revalidate — the reconciliation uses it
to decide revalidate-vs-delete per file. Prior DISTILL **docs** for the old
design (`distill/test-scenarios.md`, `distill/red-classification.md`) ARE
deleted by the planner (they are DISTILL-wave artifacts, superseded, and
deleting them touches no production code).

---

## 8. Completeness-audit + review gate (per-feature, applies to all pieces)

- Target ≥40% error/edge scenarios per piece (there is a very rich sad-path
  set: the 6-row boundary table rows 2/4/5, and R5-* enumerate failure modes).
- Layer-dependent PBT mode (Mandate 9 / `.claude/rules/testing.md`): PBT
  (`proptest`) at unit / pure-function layer (codec roundtrips, pool
  invariants, overlap predicates, fragment-decision fn); **example-based**
  at Tier-3 / integration / seeded-sim (sad paths enumerated, never
  PBT-generated).
- The `nw-at-completeness-check` 15-item gate + the four-reviewer Final Wave
  Review Gate (Sentinel always dispatches) run over the assembled scenario set
  once all six pieces land.
