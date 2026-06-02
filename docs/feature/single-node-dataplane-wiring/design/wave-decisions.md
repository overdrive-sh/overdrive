# DESIGN Wave Decisions — `single-node-dataplane-wiring`

**Wave**: DESIGN (bug-driven; propose mode). **Architect**: Titan.
**Date**: 2026-06-02. **Status**: **Accepted** — ratified by the user
2026-06-02. ADR-0061 Accepted.

Bug: production `overdrive serve` aborts at boot with `EBUSY` in its
default single-node configuration (`DataplaneConfig::loopback()` points
both XDP ifaces at `lo`; the kernel allows one program per XDP hook).
`lo` is additionally wrong because generic XDP on loopback can bypass
cloned skbs (silent traffic miss).

---

## Decisions

| # | Decision | Choice | Rationale | Source |
|---|---|---|---|---|
| D1 | Interface provisioning | **Dedicated veth pair** (research Option E), not `lo` | Removes `EBUSY` (two distinct hooks) AND the `lo` cloned-skb correctness hole; zero BPF change; matches ADR-0043 topology | research § Recommendation; Finding 6.1, 6.3 |
| D2 | Program topology | **Two distinct programs on two distinct ifaces** (E alone); reject merge (Option B) | ADR-0045's cross-iface `bpf_redirect` datapath needs a second iface to redirect to; a merged single-hook program cannot reproduce it (§ 4.2 feature-delta) | ADR-0045; `ThreeIfaceTopology` docstring |
| D3 | Diagnostic | Typed `DataplaneError::IfaceXdpSlotBusy { iface }` + honest EBUSY remediation; classify before the `DRV_MODE`-hardcoded string | Distinct-failure-mode rule; the current message masks the EBUSY mechanism (`lib.rs:551`) | development.md § Errors |
| D-G4 | Who provisions the veth pair | **(a) auto-provision at `serve` boot** — idempotent + **OS-image-adoptable** | Phase-1 "single-node just boots" floor; `CAP_NET_ADMIN` already required for XDP + cgroup; idempotent **detect-and-reuse** lets a Yocto OS image / Lima VM-boot provisioner own the iface lifecycle and `serve` reuses it (same property as DQ-4) | ADR-0025 § Alt E; feature-delta § 6.3; ADR-0061 § 3 |
| D4 | Steering mechanism (G-4) | Host-netns veth pair + `ip route add <vip_range> dev ovd-veth-cli`; VIP-range gateway on the client-side veth; `bpf_fib_lookup`+`bpf_redirect` crosses the pair | Production single-NIC routing-host model (ADR-0043) collapsed to one host in the host netns; datapath unchanged | feature-delta § 6 |
| D5 | Config defaults | `client_iface = "ovd-veth-cli"`, `backend_iface = "ovd-veth-bk"`; provisioner derives gateway+route from `[dataplane.vip_allocator].ranges` | Persist inputs not derived state; reuse existing VIP-range config | ADR-0049; development.md |

## Recommendation (decision-ready)

**Adopt Option 1 (E alone): dedicated veth pair, two programs, two
ifaces, auto-provisioned at boot, with a typed `IfaceXdpSlotBusy`
diagnostic guard.** It is the only option that boots, fixes the `lo`
correctness hole, AND preserves ADR-0045's cross-iface datapath — with
zero kernel-side change and full reuse of the Tier-3 topology shape.

Reject Option 2 (E+B) and Option 3 (B-on-`lo`): merging two programs
onto one hook structurally reverses ADR-0045's cross-iface
`bpf_redirect` model (no second iface to redirect to) and (for Opt 3)
keeps the `lo` correctness hole.

## Reuse verdicts (hard gate)

- `EbpfDataplane` — **EXTEND** (error classification only).
- `DataplaneConfig` parser — **REUSE unchanged**; `loopback()` helper
  **REPLACE** with veth-named helper.
- veth provisioner — **CREATE NEW** (`adapter-host`; NOT
  `overdrive-testing`, which is dev-dep-only).
- `ThreeIfaceTopology` — **REUSE as reference shape**, not linked.
- `BackendDiscoveryBridge` `host_ipv4` resolution — **REUSE unchanged**.
- `DataplaneError` — **EXTEND** (one variant).

## Must-not-regress

Two-NIC production path; veth e2e Tier-3 tests; ADR-0045; aya 0.13.x
(no dispatcher/FFI/kernel bump); ADR-0052 boot composition.

## Open questions — resolved at ratification (2026-06-02)

All decided. Two scope-cuts tracked by existing issues (#194, #195); no
unowned deferrals remain.

- **DQ-1** (provisioning owner) — **RESOLVED: (a) serve-boot
  auto-provision**, idempotent and OS-image-adoptable. Serve-boot
  provisioning detect-and-reuses a pre-existing pair, so a Yocto OS image
  or the Lima VM boot can own the iface lifecycle interchangeably. Same
  property as DQ-4.
- **DQ-4** (teardown semantics) — **RESOLVED: idempotent reuse**, never
  tear down on shutdown (mirrors bpffs-pin persistence per ADR-0052 § 3).
  This single property makes both the serve-restart case and DQ-1's
  OS-image adoption work.
- **DQ-2** — explicit `[dataplane] provision = "veth"|"none"` opt-out
  knob — **DEFERRED to issue #194** (operator-tunable; fix ships
  implicit-by-default names + idempotent reuse).
- **DQ-3** — IPv6 / AF_INET6 single-node veth steering — **DEFERRED to
  issue #195** (depends on IPv6 dataplane forwarding, #155; this fix is
  IPv4-only).

## ADRs written

- ADR-0061 — Single-node veth dataplane wiring (this decision).

## SSOT updated

- `docs/product/architecture/brief.md` § System Architecture — single-node
  dataplane interface-wiring decision recorded.
</content>
