# Feature Delta — `single-node-dataplane-wiring`

**Wave**: DESIGN (bug-driven; no DISCUSS/DISTILL upstream). **Mode**:
propose (2–3 options + trade-off matrix + recommendation; decision-ready
for the user). **Architect**: Titan (system-design). **Date**: 2026-06-02.

> **Scope sentence.** Decide how a single-node `overdrive serve` wires
> its XDP dataplane interface(s) so it **boots** in the default
> configuration and **steers traffic correctly** — closing the
> production `EBUSY` boot abort and the deeper `lo`/generic-XDP
> cloned-skb correctness gap. This feeds a **fix**, orthogonal to the
> in-flight `udp-service-support` feature.

---

## 1. Problem statement (RCA-grounded)

Production `overdrive serve` cannot boot in its **default single-node
configuration**. The chain (confirmed by RCA; evidence at
`verification/expectations/O03-deploy-udp-service-accepted-udp-intent/evidence/serve.log`):

1. `EbpfDataplane::new_with_pin_dir` attaches **two distinct** XDP
   programs:
   - forward `xdp_service_map_lookup` → `client_iface` ingress
     (`crates/overdrive-dataplane/src/lib.rs:488-510`),
   - reverse `xdp_reverse_nat_lookup` → `backend_iface` ingress
     (`crates/overdrive-dataplane/src/lib.rs:533-555`).
2. `DataplaneConfig::loopback()`
   (`crates/overdrive-control-plane/src/dataplane_config.rs:61-63`),
   wired as the default `ServerConfig.dataplane`
   (`crates/overdrive-control-plane/src/lib.rs:526`), sets **both
   ifaces to `"lo"`**.
3. The kernel permits **exactly one program on a netdev's XDP hook**
   (research Finding 1.1). The second attach returns `EBUSY` → boot
   aborts.
4. The error message hardcodes `DRV_MODE`
   (`crates/overdrive-dataplane/src/lib.rs:551`), **masking** the real
   `EBUSY` mechanism behind a misleading "native attach failed" string.

**Deeper issue (research Finding 6.1).** `lo` has no native XDP driver,
so the attach is *always* generic/SKB mode — which Cilium documents can
**bypass cloned skbs** on the TCP retransmit / segmentation path. A
loopback dataplane may therefore *silently miss traffic* even after the
collision is resolved. `lo` is the wrong target on **two** counts.

**Why this matters now.** Phase 1 is **single-node-in-scope** (project
constraint; ADR-0025). Single-node `overdrive serve` MUST boot and MUST
steer traffic. The default path is currently broken end-to-end.

---

## 2. The decision (spine)

**How does a single-node `overdrive serve` wire its XDP dataplane so it
boots and steers traffic correctly?** Three sub-decisions, settled
below and in `design/wave-decisions.md` + the ADRs:

- **D1 — Interface provisioning.** Provision a dedicated veth pair for
  single-node (research Option E) instead of pointing both ifaces at
  `lo`. **Recommended.** See § 4 (options) and § 6 (G-4 traffic
  steering).
- **D2 — Program topology.** Keep the **two distinct XDP programs on
  two distinct veth ifaces** (E alone). Do **not** merge into one
  staged program (research Option B) — § 4.2 explains why ADR-0045's
  cross-iface `bpf_redirect` datapath makes B *structurally unable* to
  reproduce the production behaviour on a single hook.
- **D3 — Diagnostic fix.** Add a typed
  `DataplaneError::IfaceXdpSlotBusy { iface }` variant + honest `EBUSY`
  remediation, as a **defensive guard** even though E makes the
  collision unreachable on the default path.

The load-bearing open gap this design **closes** is **G-4: how
single-node steers traffic through the veth client side** — § 6.

---

## 3. DDD / component context (inherited, not re-derived)

This is an infrastructure-wiring fix. The domain model is untouched —
`Service`, `Listener`, `ServiceVip`, `Backend`, `ServiceFrontend`
(ADR-0060) all stay as-is. The relevant component context the fix
threads through:

| Component | Role | This feature |
|---|---|---|
| `EbpfDataplane` (`overdrive-dataplane`) | `adapter-host` body of the `Dataplane` port; loads + attaches the two XDP programs | **EXTEND** — add `IfaceXdpSlotBusy` classification; no attach-shape change |
| `DataplaneConfig` (`overdrive-control-plane`) | parses `[dataplane] client_iface/backend_iface` (ADR-0052 § 3) | **EXTEND** — `loopback()` test-helper replaced with a veth-named helper; production parser unchanged |
| **veth provisioner** (NEW) | stands up the single-node veth pair + addresses + routes at boot | **CREATE NEW** — see § 5 Reuse Analysis |
| `BackendDiscoveryBridge` (`overdrive-control-plane`) | resolves `host_ipv4` from `client_iface` at boot (ADR-0052 § 1) | **unchanged** — `host_ipv4` now resolves on the veth client side |
| `ThreeIfaceTopology` (`overdrive-testing`) | Tier-3 test topology (ADR-0043) | **REUSE as reference shape** — the production provisioner mirrors a *subset* of its `ip` sequence; the test topology is NOT regressed |

### Driving / driven ports touched

- **Driven port — none new on the `Dataplane` trait.** The trait
  surface (`update_service`, `service_backends`, `probe`) is unchanged.
  The fix lives *below* the port (interface provisioning) and *inside*
  the host adapter (error classification).
- **Driving — none.** No new operator verb, no new CLI surface. The
  `[dataplane]` section already exists (ADR-0052); this feature changes
  what the **default** single-node value *is*. The optional explicit
  `provision` knob (§ 6.4) is deferred to issue **#194** (DQ-2).

---

## 4. Options (propose mode)

Five options were enumerated by the research
(`docs/research/dataplane/xdp-multiprog-same-iface-aya-research.md`);
this design narrows to the three that are viable for Phase-1
single-node and presents them as decision-ready.

### 4.1 Trade-off matrix

| Axis | **Opt 1 — E: dedicated veth pair (2 programs, 2 ifaces)** | **Opt 2 — E + B: veth pair + merged single program** | **Opt 3 — B-on-`lo`: merge programs, attach both to `lo`** |
|---|---|---|---|
| Fixes `EBUSY` boot abort | **Yes** — two distinct hooks, one program each | **Yes** — one program, one hook | **Yes** — one program owns the single `lo` hook |
| Fixes `lo` cloned-skb correctness (Finding 6.1) | **Yes** — veth native XDP, no `lo` | **Yes** — veth native XDP | **No** — still `lo`/generic; silent traffic miss persists |
| Compatible with ADR-0045 cross-iface `bpf_redirect` datapath | **Yes** — forward on client veth ingress, reverse on backend veth ingress; `bpf_fib_lookup`+`bpf_redirect` crosses between them, exactly as the Tier-3 topology proves | **NO (structural)** — a single merged program on **one** hook has no second iface to `bpf_redirect` *to*; the forward→backend cross-iface hop disappears (§ 4.2) | **NO (structural)** — same single-hook problem as Opt 2, plus `lo` |
| Kernel-side BPF change | **None** | Merge two `#[xdp]` bodies into one + Tier-4 verifier re-baseline | Merge + re-baseline |
| Matches landed ADRs | **0043** (veth topology) + **0045** (datapath) + **0052** (boot wiring) — all preserved | Partially — would amend the two-program attach shape | Conflicts with 0045's cross-iface model |
| aya 0.13.x constraint | Within surface (no dispatcher needed) | Within surface | Within surface |
| New mechanism / risk | **Low** — boot-time veth provisioning (deploy plumbing) | Low-Med — provisioning + a kernel-side merge + verifier gate | Med — merge + accepts a known-broken correctness mode |
| Regresses two-NIC production path | No — distinct ifaces stay distinct | No, *if* merged program attached to each iface | No |
| Regresses veth e2e Tier-3 tests | No — same shape | **Risk** — kernel-side merge touches the programs the e2e tests exercise | **Risk** — same |
| Effort (advisory) | ~8–12 h (provisioner + config + diagnostic + tests) | ~16–24 h (above + kernel merge + Tier-4 re-baseline) | ~12–18 h (merge + re-baseline; ships a known correctness hole) |

### 4.2 Why Option B (merge) is dominated *here* (a sharper finding than the research)

The research treated B as a viable fallback/complement on the
assumption the two programs are independent early-returning stages on
**one** ingress hook. That assumption held when the forward path was
`XDP_TX` (bounce out the same iface). **It no longer holds.** ADR-0045
pivoted the datapath to **cross-iface delivery**:

- `xdp_service_map_lookup` attaches at the **client-facing** veth
  ingress, does `bpf_fib_lookup` + `bpf_redirect(fib.ifindex, 0)` to
  push the rewritten frame **out the backend-facing iface**.
- `xdp_reverse_nat_lookup` attaches at the **backend-facing** veth
  ingress and redirects responses back toward the client.

A merged program on a **single** hook (Opt 2/Opt 3) has **no second
iface to redirect to** — the forward→backend cross-iface hop is the
whole point of the two-iface split, and it cannot be reproduced when
both stages share one netdev. `ThreeIfaceTopology`'s own docstring
states this directly: *"`XDP_TX` bounces a frame out the SAME iface …
so cannot deliver from `lb_veth_a` to `lb_veth_b`. Cross-iface delivery
uses `bpf_redirect(fib.ifindex, 0)`."* Merging onto one hook reverses
the structural premise of ADR-0045.

**Conclusion**: Option B is not merely "the fallback" — it is
**dominated** for Overdrive's *current* datapath. It would re-open
ADR-0045's pivot. Opt 1 (E alone) is the only option that boots, fixes
correctness, AND preserves the landed cross-iface architecture.

### 4.3 Recommendation

**Option 1 — E: dedicated veth pair, two programs on two distinct
ifaces.** It is the single option that (a) removes `EBUSY`, (b) removes
the `lo`/generic-XDP cloned-skb correctness hole, (c) preserves
ADR-0045's cross-iface `bpf_redirect` datapath verbatim, (d) requires
**zero kernel-side or BPF-mechanism change**, (e) reuses the exact
topology shape the Tier-3 tests already prove (ADR-0043). The work is
single-node **deployment plumbing** (provision the pair, steer traffic
— § 6), not dataplane surgery.

Options A/C/D from the research stay rejected (over-engineered /
dominated / reverses ADR-0045 respectively) and are not restated here;
see the research § Recommendation.

---

## 5. Reuse Analysis (mandatory hard gate)

| Capability needed | Existing asset | Verdict | Rationale |
|---|---|---|---|
| Load + attach the two XDP programs | `EbpfDataplane::new_with_pin_dir` (`overdrive-dataplane/src/lib.rs`) | **EXTEND** | Attach shape is correct for two distinct ifaces; only the error-classification arm changes (add `IfaceXdpSlotBusy`). No new attach path. |
| Parse `[dataplane] client_iface/backend_iface` | `parse_dataplane_section` (`dataplane_config.rs`) | **REUSE (unchanged)** | Production parser already requires both fields (ADR-0052 § 3). The fix changes the *default value the operator config carries*, not the parser. |
| Test-fixture interface names | `DataplaneConfig::loopback()` | **EXTEND / REPLACE** | The `loopback()` helper is the *origin of the bug-shaped default*. Replace it with a veth-named helper for fixtures that exercise the real attach path; pure-non-attach fixtures may keep loopback (they never attach). Single-cut per project policy. |
| Stand up a veth pair + addrs + routes at boot | `ThreeIfaceTopology` + `NetNs` (`overdrive-testing`) | **CREATE NEW (production); REUSE as reference** | `overdrive-testing` is **dev-dep only** (`adapter-host`, never `[dependencies]`) — a production binary MUST NOT link it (it shells `ip netns add`). The production provisioner is a *new* `adapter-host` component in `overdrive-worker` (or `overdrive-host`) that mirrors a **subset** of the `ip link add … type veth` sequence WITHOUT the netns machinery (single-node runs in the host netns). Reference shape: `netns.rs::ThreeIfaceTopology::create`. |
| Resolve `host_ipv4` from `client_iface` | `BackendDiscoveryBridge` boot resolution (ADR-0052 § 1) | **REUSE (unchanged)** | `getifaddrs` on `client_iface` now returns the veth client-side address; no code change. |
| Diagnostic error surface | `DataplaneError` enum (`overdrive-core`) | **EXTEND** | Add `IfaceXdpSlotBusy { iface }` variant + constructor; classify `EBUSY` in the attach path. |

**Net new code**: one veth-provisioner `adapter-host` component + one
`DataplaneError` variant + a config-default change. Everything else is
EXTEND/REUSE. No new domain types, no new port traits, no new reconciler.

---

## 6. G-4 closure — single-node traffic steering (the load-bearing gap)

The research left G-4 open: *Option E provisions a veth pair, but how is
single-node service traffic steered through the client side?* This is
the heart of the design. The answer reuses the **production routing-host
model** ADR-0043 proves (the LB host's routing table reaches the backend
on the iface the program redirects to) — but collapsed to a **single
machine in the host network namespace** (no netns; Phase 1 is one
process on one host).

### 6.1 The single-node topology (production, host-netns)

```
        ovd-veth-cli (client side)        ovd-veth-bk (backend side)
   ┌──────────────────────────────┐   ┌──────────────────────────────┐
   │ XDP xdp_service_map_lookup   │   │ XDP xdp_reverse_nat_lookup   │
   │ addr: VIP-range gateway IP   │<=>│ addr: backend-range gateway  │
   │ (e.g. 10.96.0.1/24)          │   │ (e.g. 10.97.0.1/24)          │
   └──────────────────────────────┘   └──────────────────────────────┘
                 ▲                                   │
                 │ route: VIP-range → dev ovd-veth-cli
                 │ (operator/deploy client sends to VIP)
                 ▼                                   ▼
        host routing table                  workload cgroup egress
        (overdrive serve process)           (Backend host_ipv4 lives here)
```

The two veth ifaces are a **pair** (`ip link add ovd-veth-cli type veth
peer name ovd-veth-bk`), both in the **host netns** (single node — no
namespace boundary). Naming and addressing mirror
`ThreeIfaceTopology`'s `lb_veth_a`/`lb_veth_b` but without the
`client-ns`/`backend-ns` namespaces — the single host **is** the client,
the LB, and the backend host all at once.

### 6.2 How traffic reaches the VIP (the steering mechanism)

The VIP is owned by the allocator (ADR-0049); the operator never names
it. A host-local route makes the VIP range reachable via the client-side
veth:

```
ip route add <vip_range> dev ovd-veth-cli        # e.g. 10.96.0.0/24
```

- The VIP-range gateway address sits on `ovd-veth-cli` so ARP/neigh
  resolves on-link (same trick as `ThreeIfaceTopology` where the VIP IS
  the address on `lb_veth_a`).
- An operator's `overdrive deploy` client, or a workload, connecting to
  `<vip>:<port>` routes the SYN **out `ovd-veth-cli`**, where the
  `xdp_service_map_lookup` program is attached at ingress on the **peer**
  (`ovd-veth-bk`-facing) side — exactly the `bpf_redirect`-into-the-pair
  delivery the Tier-3 topology proves (research Finding 4.2: XDP into a
  veth peer requires a program on the receiving peer; the production
  programs satisfy this for each other; a stub is only needed where one
  side has no real program).
- Backend traffic egresses through the workload cgroup; the bridge's
  `host_ipv4` (ADR-0052 § 1) resolves to the client-side veth address so
  `Backend.ipv4` points at a reachable on-host address.

This is the production single-NIC routing-host model (ADR-0043 § Context)
expressed on **one host in the host netns**, instead of across three
netns. The `bpf_fib_lookup` + `bpf_redirect` datapath (ADR-0045) is
**unchanged** — it resolves the egress iface and next-hop MAC from the
host routing table, which is precisely what the host-local `ip route add`
above populates.

### 6.3 Who creates the veth pair — **D-G4 decision (ratified)**

Three candidate owners were surfaced; the user ratified (a):

| Owner | Shape | Trade-off |
|---|---|---|
| **(a) `overdrive serve` at boot (auto-provision)** — *ratified* | `serve` checks for `ovd-veth-cli`/`ovd-veth-bk`; if absent, creates the pair + addresses + route before `EbpfDataplane::new`. **Idempotent detect-and-reuse**; never tears down on shutdown (DQ-4). | Zero operator steps; "it just boots." Needs `CAP_NET_ADMIN` (already required for XDP attach + cgroup writes). Mirrors the Tier-3 fixture's create-then-attach order. **Reuse makes it OS-image-adoptable** — a Yocto image or Lima VM boot can pre-provision the pair and `serve` reuses it untouched. |
| (b) a `overdrive dataplane setup` subcommand (explicit) | Operator runs a one-shot provisioning verb before `serve`. | Explicit + inspectable, but adds an operator step and a "did you run setup?" failure mode — the same friction ADR-0025 § Alternative E rejected for required config. |
| (c) external (operator's responsibility, documented) | `serve` validates the ifaces exist and refuses with remediation if not. | Lowest code, highest operator burden; the default "just boots" property is lost. |

**Ratified: (a) auto-provision at boot, idempotent and
OS-image-adoptable.** Rationale: Phase 1's ergonomic floor is "single-node
serve just boots" (ADR-0025 § Alternative E precedent — hard-refusal is
reserved for safety properties that genuinely cannot be defaulted, e.g.
cgroup delegation; interface provisioning is defaultable). `CAP_NET_ADMIN`
is already a hard precondition for XDP attach and cgroup management, so
auto-provision adds no new privilege. The provisioner **detects and
reuses** a pre-existing pair (matching the Tier-3 fixture's
best-effort-cleanup-then-create discipline) and its addresses/route/names
come from config with honest defaults (the VIP range is already the
`[dataplane.vip_allocator] ranges` value from ADR-0049 — the provisioner
derives the gateway IP and route from it, *persisting inputs not derived
state* per the development rule).

**Load-bearing constraint (added at ratification): serve-boot
provisioning MUST be idempotent and compatible with OS-image
pre-provisioning.** The future **Yocto OS image** is expected to set up
networking at OS-init time — exactly how the current **Lima dev VM**
provisions the veth/networking at VM boot. Serve-boot provisioning is the
Phase-1 mechanism, but it must *adopt* a pre-existing veth pair (created
by the OS image / VM boot) rather than fail or recreate it. The two
mechanisms are interchangeable by construction because reuse is
idempotent. This is the **same idempotent-reuse property** as DQ-4 — one
property serves both the serve-restart case and the OS-pre-provisioned
case (a restarting `serve` and an external provisioner are just two
callers of the same detect-and-adopt path).

### 6.4 Config surface (additive on ADR-0052's `[dataplane]`)

```toml
[dataplane]
# Default single-node values (auto-provisioned veth pair).
client_iface  = "ovd-veth-cli"   # was the bug-shaped "lo"
backend_iface = "ovd-veth-bk"

[dataplane.vip_allocator]
ranges = ["10.96.0.0/24"]        # existing (ADR-0049); the provisioner
                                  # derives the on-link gateway + route
                                  # from the first range.
```

Two-NIC / multi-NIC production deployments override `client_iface` /
`backend_iface` with real NIC names and **skip auto-provision** (the
provisioner only fires for the default veth names; the explicit
`[dataplane] provision = "veth" | "none"` opt-out knob is **deferred to
issue #194** — DQ-2). The existing two-NIC path is **not regressed**:
when the operator names real ifaces, the boot path resolves them as
today.

---

## 7. Diagnostic fix (D3)

Even though E makes the single-hook collision unreachable on the default
path, a typed diagnostic is in-scope defensive depth (an operator who
points both ifaces at the same real NIC, or whose veth provisioning
half-failed, still hits `EBUSY`):

- New `DataplaneError::IfaceXdpSlotBusy { iface: String }` variant in
  `overdrive-core`, with an `as_str`/`Display` that names the iface and
  the actual cause: *"interface `<iface>` already has an XDP program
  attached (EBUSY); another program owns its single XDP hook. Single-node
  default expects a dedicated veth pair — verify `client_iface` ≠
  `backend_iface` and that no stale Overdrive XDP program is attached
  (`ip link show <iface>`; detach per debugging.md § Leftover XDP)."*
- The attach path classifies `raw_os_error() == EBUSY` into this variant
  **before** the current `DRV_MODE`-hardcoded string (which masks the
  mechanism — `lib.rs:551`). This follows the development rule "distinct
  failure modes get distinct error variants; never collapse into a
  catch-all whose Display prescribes the wrong remediation."
- Bubbles up through `DataplaneBootError::Construct` (ADR-0052 § 3) — no
  new boot-error variant; the construct path already carries the iface
  names.

---

## 8. What must NOT regress (hard constraints)

- **Two-NIC / multi-NIC production path** — distinct real ifaces stay
  distinct; the boot path resolves operator-named ifaces unchanged.
- **veth e2e Tier-3 tests** (`reverse_nat_e2e.rs`, the `ThreeIfaceTopology`
  consumers) — Option E touches **no** kernel-side program, so these are
  untouched. (Opt 2/3 would have risked them — another reason to reject
  merge.)
- **ADR-0045** (cross-iface `bpf_redirect` datapath) — preserved
  verbatim; Opt 1 *depends* on it.
- **aya 0.13.x** — no dispatcher API needed; no libxdp/C-FFI dep; no
  kernel-floor bump.
- **ADR-0052** boot composition (`[dataplane]` parse → `EbpfDataplane::new`
  → probe → use) — extended additively (veth provision happens **before**
  `EbpfDataplane::new`), not reshaped.

---

## 9. Technology choices

| Choice | Decision | Rationale |
|---|---|---|
| Interface kind | veth pair (not `dummy`, not `lo`) | veth supports native XDP (Finding 6.1); `dummy` ifaces force generic mode (same cloned-skb hole as `lo`); a pair gives the two distinct hooks ADR-0045 needs. |
| Provisioning tool | `ip link`/`ip addr`/`ip route` via the existing host shell-out pattern | Same primitive `overdrive-testing` uses; no new dependency. Production component lives in an `adapter-host` crate, never `overdrive-testing`. |
| Provisioning trigger | auto at `serve` boot (D-G4 (a)), idempotent detect-and-reuse — adoptable by OS-image/VM-boot provisioning (Yocto, Lima) | Phase-1 ergonomic floor; `CAP_NET_ADMIN` already required. Explicit `[dataplane] provision` opt-out knob deferred to **#194** (DQ-2); IPv6 single-node steering deferred to **#195** (DQ-3). |
| Diagnostic | typed `IfaceXdpSlotBusy` variant | Distinct-failure-mode rule. |

---

## 10. Open questions — resolved at ratification (2026-06-02)

All four open questions are decided. Two are scope-cuts tracked by
existing GitHub issues (#194, #195). No unowned deferrals remain.

- **DQ-1 (D-G4 ownership) — RESOLVED: (a) auto-provision at `serve`
  boot**, with the added load-bearing constraint that serve-boot
  provisioning is **idempotent and OS-image-adoptable**. It
  detect-and-reuses a pre-existing veth pair, so an OS image (**Yocto**)
  or a VM-boot provisioner (**Lima**, which already provisions
  veth/networking at VM boot today) can own the interface lifecycle and
  `serve` reuses what it finds. Serve-boot and OS-image provisioning are
  interchangeable by construction because reuse is idempotent — the same
  property as DQ-4. See § 6.3 and ADR-0061 § 3 / "Changed assumptions".
- **DQ-2 (provision gate knob) — DEFERRED to issue #194.** The fix ships
  implicit-by-default veth names + idempotent reuse; the explicit
  operator-tunable `[dataplane] provision = "veth" | "none"` opt-out knob
  is tracked in **#194**. Cited at every reference site (§ 6.4, § 9).
- **DQ-3 (IPv6 single-node) — DEFERRED to issue #195.** The veth
  provisioner + route are IPv4-only (matching the IPv4-only datapath
  today). IPv6 / AF_INET6 single-node veth steering is tracked in
  **#195**; it depends on IPv6 dataplane forwarding (issue #155).
- **DQ-4 (teardown semantics) — RESOLVED: idempotent reuse.** `serve`
  never tears the veth pair down on shutdown; it persists across
  restarts (mirrors bpffs-pin persistence per ADR-0052 § 3 Drop), and
  the next boot reuses it. Manual cleanup is documented in debugging.md.
  This is the **single property that also makes DQ-1's OS-image adoption
  work** — one detect-and-reuse mechanism, two callers (a restarting
  `serve`; an external OS/VM provisioner).

---

## 11. Deliverables index

- This file — `docs/feature/single-node-dataplane-wiring/feature-delta.md`.
- `docs/feature/single-node-dataplane-wiring/design/wave-decisions.md` —
  decisions summary.
- `docs/feature/single-node-dataplane-wiring/design/c4-diagrams.md` —
  C4 System Context + Container (Mermaid).
- ADR — `docs/product/architecture/adr-0061-single-node-veth-dataplane-wiring.md`.
- SSOT — `docs/product/architecture/brief.md` § System Architecture
  extended with the single-node dataplane wiring decision.
</content>
</invoke>
