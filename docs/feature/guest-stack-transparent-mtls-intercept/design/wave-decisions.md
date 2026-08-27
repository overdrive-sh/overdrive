# DESIGN Decisions — guest-stack-transparent-mtls-intercept (GH #222)

Mode: PROPOSE (options + trade-offs presented; recommendations recorded here
as decisions). Scope: APPLICATION/COMPONENTS. Paradigm: object-oriented Rust.
Full narrative: `../feature-delta.md`. ADRs: 0088 (topology + addressing),
0089 (provisioning boundary + CH net attach).

## Key Decisions

| # | Decision | One-line rationale |
|---|---|---|
| D1 | **Routed two-/30 topology** (tap + guest /30 in the netns, `ip_forward`, host return route); guest /30 carved from `10.99.0.0/16` upper half (`base + 0x8000 + slot*4`) | Byte-for-byte the spike-proven shape (WORKS, no toggles); in-/16 carve keeps every mesh-membership test correct for free. L2-bridge and /32-onlink variants rejected as unproven L2/unnumbered-routing risk. |
| D2a | **`workload_addr` = the guest addr** for VM-kind allocs (injected at C3; rides the EXISTING `AllocStatusRowV2` field — no schema change) | The only address a connection can terminate at; every downstream consumer (persist, resolve, future inbound daddr match, future advertise) is correct with zero change. |
| D2b | **Guest addressing = one platform-owned kernel-cmdline parameter applied by `overdrive-init`** (ioctls + guest resolv.conf → responder), fail-closed (apply-or-EXIT before exec) | Reuses the platform-owned guest PID 1 + the existing cmdline surface; `CONFIG_IP_PNP` unavailability is moot; DHCP/vsock-message add mechanism for a statically known value. Beacon PL unchanged. |
| D3 | **Return route (+ ip_forward + tap converge) owned by the C3-seam provisioner extension** (`veth_provisioner`, Bar-1 converge-on-boot); teardown structural; Bar-2 rides #197/#234 | Same lifecycle as the veth it rides on; fail-closed via existing `ShimError`; no new reconciler (reconcilers.md valid-intermediate). |
| D4 | **C3 branches on the `DriverPayload` VM arm** → pure `VmTapPlan` + tap converge + spec injection; `VmDriver` composes `VmConfig` (netns now CONSUMED + net attach + cmdline); **`Vmm` prepends `ip netns exec <ns>` to the existing wrapper argv + `--net tap=,mac=`**; tap creation subprocess-free (ioctl create + netlink netns-move, EXTEND `overdrive-netlink`) | Provisioner-creates/driver-enters preserved; `overdrive-host` stays `#![forbid(unsafe_code)]`; spike launch shape verbatim. fd-passing REJECTED ON THE MERITS (ADR-0089 §A2 — the Firecracker-jailer `setns` precedent points AT the wrapper), re-open only with evidence against the wrapper — NOT a queued refinement. |
| D5 | **Inbound (peer→guest): topology settled NOW, build deferred to #257** (existing issue) | Zero change needed in `install_inbound_tproxy` (keys on `workload_addr` = guest addr); leg-S delivery = the spike-proven reply path; a #222 inbound slice has NO serve+deploy driver until #257 removes the `[vm]+[service]` parse rejection — building it would repeat the #236 dead-mechanism precedent. |
| D6 | **Lift the `DriverType::Exec` intercept gate to include VM-kind at BOTH install sites** (`action_shim/mod.rs:1584` fresh-start + `:1880` restart); teardown (`stop_alloc` `:1269`/`:2038`) ungated-by-design (no flip, none must be added); D-MTLS-18 fail-closed inherited | The gate's own comment names #222 as its lifter; without the flip the feature is dead on the production path. Flipping ONLY fresh-start leaves restarted VM allocs CLEARTEXT fail-OPEN (restart budget / crash-recovery / `overdrive workload restart` ADR-0073 — all live for VM) while the fresh-deploy AT goes green — so both gates flip, and a Tier-3 restart AT (kvm-tests / metal) pins the restart site. |

## Architecture Summary

The ONLY new production code is the tap wire: tap-in-netns provisioning
(pure `VmTapPlan` + four idempotent converge steps), the CH netns entry +
`--net tap` attach, guest addressing via cmdline + `overdrive-init`, and the
one-site intercept-gate flip. The host-side intercept
(`install_outbound_tproxy` + `ensure_shared_routing_infra`) and the entire
#26 `MtlsEnforcement` proxy (handshake/kTLS/splice, ADR-0069/0070) are reused
verbatim — the spike proved the production rule fires unchanged over a
tap-fed veth. No new crate, port, daemon, dependency, or observation-schema
change.

## Walking-skeleton egress slice (Slice 1, BLOCKING)

`overdrive serve` (mTLS-composed) + `overdrive deploy` of a `[vm]`+`[job]`
whose guest dials a mesh `[service]` by name: guest egress captured at the
host-veth by the EXISTING rule → leg-F → `MtlsResolve` `Mesh` → proven mTLS
to the peer's agent → response returns into the guest; a non-mesh dial passes
through (`NonMesh`); an intercept-install failure drives the alloc terminal.
No test-only wiring — every install/bind/address/route is a production call
site.

The AT must additionally assert three obligations this remediation names:
(1) **first-connect safety** — the guest's first mesh dial is captured with
ZERO cleartext SYN escaping (the born-captured ordering invariant
`install-success ≺ EXEC-release`, Finding 2 / Q9); (2) **guest dial-by-name**
resolves over routed hops FIRST — it is topology-reasoned, NOT spike-proven
(the spike used raw-IP connect; Finding 5); (3) a **restart** AT (same
nested-KVM metal surface) — a *restarted* VM alloc re-installs the intercept
(the `:1880` gate) and is driven terminal fail-closed on install failure
(Finding 1). DISTILL authors the scenarios.

**Execution surface (iteration-1 HIGH remediation)**: the Slice-1 Tier-3 AT
boots a REAL microVM → requires nested KVM → gated behind `kvm-tests` (on top
of `integration-tests`) and run via `cargo xtask metal run --` on the x86_64
metal box, NOT Lima (arm64 Lima has no nested KVM; a Lima AT returns no
signal).

## Reuse Analysis (verdict tally)

7 REUSE-AS-IS (`install_outbound_tproxy`, `ensure_shared_routing_infra`,
`start_alloc` + the #26 proxy, `NetSlotAllocator`, `install_inbound_tproxy`
(deferred use), DNS responder, `AllocStatusRowV2.workload_addr`) · 7 EXTEND
(C3 seam, veth provisioner, `overdrive-netlink`, `AllocationSpec` channel,
`VmConfig`+`Vmm`, `VmDriver`, `overdrive-init`, the Exec gate) · 1 CREATE-NEW
(the pure `VmTapPlan` value + its converge steps, housed inside EXTENDed
components). Full table with contract shapes: `../feature-delta.md`
§ Reuse Analysis.

## Tech Stack

CH `--net tap=` (v53.0, spike-proven) · `/dev/net/tun` ioctls via `nix` 0.30 ·
netlink netns-move via `overdrive-netlink` · `ip netns exec` wrapper argv
(iproute2, appliance-present) · in-tree `overdrive-init`. No new external
dependency; no proprietary component.

## Constraints

- Vertical slice: Slice 1 MUST run through real `serve`+`deploy` (the #236
  precedent is the counter-example).
- `overdrive-host` stays `#![forbid(unsafe_code)]`; CH remains the only
  sanctioned subprocess (the netns wrapper rides it).
- Spike verdict pinned to kernel 7.0.0-29; 6.18 appliance confirmation =
  Tier-3 matrix at merge (nft_tproxy confirmation user-waived).
- `NET_SLOT_MAX = 4095` keeps both /30 carves inside `10.99.0.0/16`.
  **DELIVER item (Finding 4):** the guest-carve disjointness must be
  COMPILER-PROVEN — DELIVER adds a symmetric guest-carve const guard
  `(0x8000 + NET_SLOT_MAX*4 + 3) < base_span` beside the existing S6 transit
  guard (`veth_provisioner.rs:518`, which asserts only `(NET_SLOT_MAX*4 + 3) <
  base_span`), so a future slot-domain / base raise cannot silently overflow
  the guest carve. A prose "re-check the split" caveat is insufficient (noted
  beside the #239 tunable-base caveat).

## Upstream Changes

- brief.md: new `## Guest-stack transparent-mTLS intercept extension
  (ADR-0088/0089, GH #222)` section + ADR-index rows + changelog row.
- ADR-0069's "STAGED to #222" pointer is now realised by this design (no
  ADR-0069 edit needed — the fold decision is unchanged).
- `VmConfig.netns` docstring's "Job-kind VMs need no tap" becomes stale when
  DELIVER lands the consuming code — the crafter fixes it in the step that
  changes the behavior (behavior-change-marks-stale-docs discipline), not
  here.

## Open questions → DISTILL

Q1 guest-net channel field shape · Q2 `VmConfig` net-attach struct shape
(fold recommended) · Q3 cmdline parameter grammar · Q4 MAC byte layout ·
Q6 guest-subnet const name/home · Q7 host-side disambiguation of a fail-closed
pre-exec net-apply `EXIT` vs a normal non-zero operator exit (review medium) ·
Q8 the sanctioned `KernelCmdline` compose/append surface (review medium —
named in the design so the crafter is not inventing it) · Q9 the exact
EXEC-release wiring realising the born-captured invariant `install-success ≺
EXEC-release` (where the release gate sits relative to the `Running` boundary +
the READY-vs-EXEC "what is Running for a VM" reconciliation; iteration-2
Finding 2 — model pinned, wiring open). (Q5 tap name PINNED:
`ovd-tp-<4hex-slot>`.)

## Deferrals (existing issues only — none created)

Inbound build + `[vm]+[service]` enablement → **#257** · Bar-2 network
reconciler → **#197/#234** · intended-peer pinning → **#242** (concept
inherited from ADR-0069; ADR-0069's own #178 citation is stale, flagged for
separate cleanup).

## Gaps for the orchestrator

- Outcome Collision Check NOT run (CLI unavailable in this dispatch).
