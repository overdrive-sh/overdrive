# Evolution — single-node-dataplane-wiring

**Finalized:** 2026-06-03 · **Wave lifecycle:** DESIGN → DELIVER (4 steps,
all GREEN) · bug-driven, no DISCUSS/DISTILL upstream · **Source:**
production RCA — `overdrive serve` cannot boot in its default single-node
configuration (`EBUSY`). **ADR:** ADR-0061 (Accepted 2026-06-02).

## Summary

Fix the production `overdrive serve` default-config boot abort, where two
distinct XDP programs both tried to attach to `lo` and the second attach
returned `EBUSY`. Replace the bug-shaped `lo`/`lo` default with a
dedicated host-netns **veth pair** (`ovd-veth-cli` / `ovd-veth-bk`),
auto-provisioned idempotently at boot, and add a typed
`DataplaneError::IfaceXdpSlotBusy` diagnostic so the residual
both-ifaces-one-NIC operator error surfaces honestly instead of behind a
masking `DRV_MODE` string.

Before this feature, `DataplaneConfig::loopback()` set **both**
`client_iface` and `backend_iface` to `"lo"`. The dataplane attaches two
distinct programs — `xdp_service_map_lookup` to the client iface ingress
and `xdp_reverse_nat_lookup` to the backend iface ingress — but the
kernel permits exactly one program per netdev XDP hook. The second attach
returned `EBUSY`, which the loader wrapped in
`DataplaneError::LoadFailed(format!("...DRV_MODE: {e}"))`, masking the
real mechanism behind a misleading "native attach failed" string and
prescribing the wrong remediation. `lo` was wrong on a **second** count:
it has no native XDP driver, so the attach is always generic/SKB mode,
which can bypass cloned skbs on the TCP retransmit/segmentation path — a
silent traffic miss even after the collision is resolved.

The chosen fix (research Option E, ADR-0061) provisions a dedicated veth
pair instead of `lo`: two distinct hooks remove the `EBUSY`, native veth
XDP removes the cloned-skb correctness hole, and — critically — the
two-iface split **preserves ADR-0045's cross-iface `bpf_redirect`
datapath** verbatim. Merging the two programs onto one hook (research
Option B) was rejected as structurally dominated: a single-hook program
has no second iface to `bpf_redirect` to, which would reverse ADR-0045's
pivot. Zero kernel-side or BPF-mechanism change; the work is single-node
deployment plumbing.

## Business context

Phase 1 is single-node-in-scope (ADR-0025). The single-node
`overdrive serve` MUST boot and MUST steer traffic — it is the
foundational floor every other Phase-1 capability stands on. The default
path was broken end-to-end: an operator running `overdrive serve` with
the shipped default config hit a boot abort with a diagnostic that
pointed them at the wrong problem. This fix restores the "single-node
serve just boots" ergonomic floor and makes the residual failure mode
(an operator pointing both ifaces at one real NIC) self-explanatory.

The RCA surfaced via the in-flight `udp-service-support` verification
catalogue (`O03` expectation evidence: `serve.log`), but the bug is
orthogonal to UDP — it blocks *any* default-config single-node boot.

## Key decisions

| # | Decision | Choice | Rationale |
|---|---|---|---|
| D1 | Interface provisioning | Dedicated **veth pair**, not `lo` | Removes `EBUSY` (two distinct hooks) AND the `lo` cloned-skb correctness hole; zero BPF change; matches ADR-0043 topology. |
| D2 | Program topology | **Two distinct programs on two distinct ifaces** (Option E alone); reject merge (Option B) | ADR-0045's cross-iface `bpf_redirect` datapath needs a second iface to redirect *to*; a merged single-hook program cannot reproduce it. Option B is dominated, not a fallback. |
| D3 | Diagnostic | Typed `DataplaneError::IfaceXdpSlotBusy { iface }` classified on `raw_os_error() == EBUSY` **before** the masking `DRV_MODE` string | Distinct-failure-mode rule; the prior message prescribed the wrong remediation. |
| D-G4 / DQ-1 | Who provisions the pair | **Auto-provision at `serve` boot**, idempotent detect-and-reuse | Phase-1 "just boots" floor; `CAP_NET_ADMIN` already required for XDP + cgroup. Idempotent reuse makes the pair **OS-image-adoptable** — a Yocto image or the Lima VM boot can own the lifecycle and `serve` adopts what it finds. |
| DQ-4 | Teardown | **Never tear down on shutdown**; persist across restarts | One detect-and-reuse property serves both the serve-restart case and OS-image pre-provisioning (mirrors bpffs-pin persistence, ADR-0052 § 3). |
| D4 / D5 | Steering + config defaults | Host-netns veth pair + `ip route add <vip_range> dev ovd-veth-cli`; gateway derived from the first `[dataplane.vip_allocator].ranges` entry (ADR-0049) | Production single-NIC routing-host model (ADR-0043) collapsed to one host in the host netns; persist inputs (the VIP range), not the derived gateway/route. |

Two scope-cuts were ratified with existing GitHub issues and cited at
every reference site — no unowned deferrals:

- **DQ-2** — explicit `[dataplane] provision = "veth"|"none"` opt-out
  knob — deferred to **#194**. The fix ships implicit-by-default veth
  names + idempotent reuse.
- **DQ-3** — IPv6 / AF_INET6 single-node veth steering — deferred to
  **#195** (depends on IPv6 dataplane forwarding, #155). This fix is
  IPv4-only, matching the IPv4-only datapath.

## Steps completed

All four steps executed Outside-In TDD under the legacy 5-phase DES
contract (PREPARE → RED_ACCEPTANCE → RED_UNIT → GREEN → COMMIT). Order is
walking-skeleton-first: make the failure honest, then build the one new
component, then remove the bug-shaped default and wire it, then prove it
end-to-end at Tier 3.

| Step | Name | Commit | Outcome |
|---|---|---|---|
| 01-01 | Honest EBUSY diagnostic — `IfaceXdpSlotBusy` variant + EBUSY classifier | `ca3b05c3` | PASS |
| 01-02 | Veth provisioner (CREATE NEW, adapter-host) — pure plan derivation + idempotent `ip` shell-out | `4660b1c5` | PASS |
| 01-03 | Veth-named default + serve-boot provisioning before `EbpfDataplane::new` | `6877990f` | PASS (after one honest GREEN FAIL → retry; see Lessons) |
| 01-04 | Tier-3 regression — two XDP programs on two veth ifaces; `IfaceXdpSlotBusy` on single-iface collision | `9f369151` | PASS |

A follow-on refactor `7eea73cc` dropped redundant `#[cfg(target_os =
"linux")]` gates on the provisioner (see Lessons).

### What landed, by surface

- **`overdrive-core`** — one new `DataplaneError::IfaceXdpSlotBusy {
  iface }` variant. `#[error(...)]` Display names the iface, names the
  real cause (`EBUSY`), and carries the `client_iface != backend_iface` +
  stale-XDP-detach remediation. Bubbles up unchanged through the existing
  `DataplaneBootError::Construct` (ADR-0052 § 3) — no new boot-error
  variant.
- **`overdrive-dataplane`** — a pure classifier `classify_iface_xdp_slot_
  busy` (sibling to `should_fallback_to_generic` / `classify_attach_
  result`) maps a `SyscallError` whose `io_error` is `libc::EBUSY` →
  `Some(IfaceXdpSlotBusy)`, `None` otherwise. Both `AttachOutcome::
  Propagate` arms classify EBUSY before the `LoadFailed`/`DRV_MODE`
  fallthrough. No attach-shape change; the happy path and the
  `EOPNOTSUPP → SKB_MODE` fallback are untouched.
- **`overdrive-control-plane`** (the single CREATE NEW + the wiring) —
  - `veth_provisioner` module: `derive_veth_plan(client_iface,
    backend_iface, vip_range: Ipv4Net) -> VethProvisionPlan` is a **pure**
    default-lane fn — `client_gateway` = first usable host of the VIP
    range (`10.96.0.0/24 → 10.96.0.1`), `backend_gateway` = second usable
    host (the smallest honest Phase-1 rule, rustdoc'd), `route_cidr` =
    the range. The plan carries config iface names verbatim and is never
    persisted (derived at provision time).
  - `provision(plan) -> Result<(), VethProvisionError>` — **production**
    code (not `integration-tests`-gated, because serve boot calls it).
    Idempotent detect-and-reuse: `ip link show` first, adopt a
    pre-existing pair untouched; create + address + up + on-link route
    only when absent; route-add tolerates `File exists`. Never tears
    down. `VethProvisionError` is a thiserror enum with a distinct
    variant per `ip(8)` step (no catch-all String).
  - Config default: `DataplaneConfig::loopback()` (the `lo`/`lo` origin
    of the bug) replaced by a veth-named default returning two distinct
    SSOT iface names. The production parser `parse_dataplane_section` is
    unchanged.
  - Serve-boot wiring: the production (non-override) boot branch resolves
    the VIP range, calls `derive_veth_plan` + `provision().await`
    **before** `EbpfDataplane::new_with_pin_dir`, extending the ADR-0052
    parse → construct → probe → use order additively. A
    `VethProvisionError` maps into `DataplaneBootError` via a dedicated
    `#[from]` variant (no `internal(String)` flattening); failure emits a
    `health.startup.refused` event. The `SimDataplane`-override path
    (test fixtures) skips provisioning entirely.
- **Tier-3 regression** (`crates/overdrive-control-plane/tests/
  integration/serve_boot_provisions_veth.rs`): (1) HAPPY — default veth
  pair provisioned, the two distinct programs both attach to two distinct
  ifindexes, construction `Ok`; asserted on observable kernel state (each
  ifindex carries a distinct attached XDP program id). (2) DIAGNOSTIC —
  both ifaces pointed at one real iface yields a real kernel `EBUSY`
  surfaced as `DataplaneBootError::Construct { source:
  IfaceXdpSlotBusy }`, matched by variant, not Display-grep. RAII
  `VethGuard` reaps both veth ends; `reverse_nat_e2e.rs` is unchanged and
  still passes (ADR-0045 untouched).

## Lessons learned

- **A bug-shaped default has a blast radius far beyond the bug.**
  Changing the default `dataplane` from `lo`/`lo` to `ovd-veth-cli`/
  `ovd-veth-bk` regressed 11 `SimDataplane`-override fixtures that inherit
  the default via `..Default::default()`. They inject a Sim dataplane (no
  XDP attach) but still resolve `host_ipv4` from `client_iface` at boot —
  and `ovd-veth-cli` does not exist in the test VM. The fix was a single
  shared SSOT helper (`tests/common/dataplane_lo.rs`, `#[path]`-included
  into both the acceptance and integration crate roots) naming `lo`
  through one place so the shape cannot drift. This is why step 01-03
  logged an honest GREEN **FAIL** before the retry PASS — the regression
  was real, surfaced, and fixed, not papered over.
- **A synthetic-error unit test and a real-kernel test are not
  substitutes.** Lima virtio-net never produces a real `EBUSY` on a fresh
  veth, so 01-01's classifier unit tests drive **synthetic**
  `SyscallError` values, and the only place the real EBUSY path is
  exercised is 01-04's Tier-3 single-iface-collision case. Both are
  load-bearing — the unit test pins the branch logic and feeds the
  mutation gate; the Tier-3 test proves the classifier fires against an
  actual kernel rejection.
- **Provisioning belongs in production, gating belongs in tests.**
  `provision()` is production code (`serve` calls it at boot), so it is
  **not** `integration-tests`-gated — only its Tier-3 *tests* are. Mixing
  the two up (gating the production fn behind the test feature) would have
  made the serve-boot call site reference a cfg-absent fn.
- **`#[cfg(target_os = "linux")]` on production code is noise here.** The
  provisioner shells `ip(8)` via `std::process::Command`, which compiles
  everywhere; the gates on `provision`/`addr_add`/`link_up` were redundant
  *and* latently inconsistent with the ungated serve-boot call site
  (a non-Linux compile would have referenced a cfg-absent fn). Overdrive
  targets Linux and the canonical compile is Lima — the refactor
  `7eea73cc` removed them.
- **Reuse the reference shape, not the crate.** The provisioner mirrors a
  subset of `overdrive-testing`'s `ThreeIfaceTopology::create` `ip`
  sequence (minus the `ip netns add` machinery — single-node runs in the
  host netns) but does **not** depend on `overdrive-testing`, which is
  dev-dep-only and would smuggle `ip netns add` into a production binary.

## Issues encountered

- **Step 01-03 GREEN FAIL → PASS** — the default-value change regressed
  pre-existing Sim-override fixtures (see Lessons). Recorded honestly in
  the execution log as a FAIL event, then resolved via the shared
  `dataplane_lo` SSOT helper. No scope was deferred.

No new GitHub issues were created during delivery — DQ-2 (#194) and
DQ-3 (#195) were already created and cited in ADR-0061 before DELIVER
began.

## Must-not-regress (held)

- **Two-NIC / multi-NIC production path** — the provisioner fires only
  for the default veth names; an operator naming real NICs skips it and
  the existing boot resolution is unchanged.
- **ADR-0045 cross-iface `bpf_redirect` datapath** — preserved verbatim;
  Option E *depends* on it. `reverse_nat_e2e.rs` untouched and passing.
- **aya 0.13.x** — no dispatcher API, no libxdp/C-FFI dep, no
  kernel-floor bump.
- **ADR-0052 boot composition** — extended additively (provision slots
  before `EbpfDataplane::new`), not reshaped.

## Links to permanent artifacts

- **ADR** — `docs/product/architecture/adr-0061-single-node-veth-dataplane-wiring.md` (the authoritative design record; Accepted 2026-06-02).
- **Architecture / design** — `docs/architecture/single-node-dataplane-wiring/` (migrated `feature-delta.md` design spine + `c4-diagrams.md` C4 context/container/sequence/request-flow views).
- **SSOT** — `docs/product/architecture/brief.md` § System Architecture (single-node dataplane interface-wiring decision recorded).
- **Deferral tracking** — GitHub **#194** (provision opt-out knob), **#195** (IPv6 single-node steering).
- **Feature workspace (history, preserved)** — `docs/feature/single-node-dataplane-wiring/`.
