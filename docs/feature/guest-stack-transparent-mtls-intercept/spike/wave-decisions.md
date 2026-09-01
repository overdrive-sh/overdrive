# SPIKE Decisions — guest-stack-transparent-mtls-intercept

## Assumption Tested
- Can a guest's TCP flow — visible on the host only as virtio-net frames at a
  tap inside the per-workload netns, with **NO host `struct sock`**
  (`cgroup_connect4` / sockmap structurally blind) — be captured and turned into
  the same `InterceptedConnection` the proven universal #26 proxy consumes?
  (Egress direction — the hard case.)

## Probe Verdict
- **WORKS.** A real Cloud-Hypervisor microVM (guest terminates TCP in its own
  kernel; no host `struct sock`) dialing an off-wire peer (`10.99.0.1:9000`) was
  transparently intercepted by the production-shape nft-TPROXY rule on the
  host-side veth ingress; `getsockname()` recovered the original destination; a
  byte-distinct REQUEST/RESPONSE round-tripped. **No empirical toggle**
  (rp_filter / tx-offload) required. Kernel `7.0.0-29`, CH `v53.0`, nft `v1.1.6`.
  Evidence: `findings.md`.
- Confirms the prior-art recon (Cilium + #236: interception is origin-agnostic,
  skb/flow-level). Expected positive, not surprising.

## Promotion Decision
- **DISCARD → DESIGN** (probe verdict WORKS; Phase-3 walking skeleton skipped).
- Rationale: the mechanism is fully de-risked, but the production wiring carries
  genuine design decisions the probe deliberately left as one-option-among-
  several — the netns topology (routed two-`/30` vs L2-bridge vs
  guest-on-workload-`/30`), where `workload_addr` sits, the guest-addressing
  mechanism, return-route ownership, and the (unprobed) **inbound** direction.
  Committing a walking skeleton now would bake those decisions in-flight before
  DESIGN deliberates them (the "invent surface to go green" divergence risk,
  CLAUDE.md). The feature is large enough to deserve its own DESIGN wave
  (nw-spike DISCARD, Example 2). Probe code + findings retained committed per
  spike policy (not deleted).

## Walking Skeleton
- Not built (DISCARD). DESIGN designs the feature from `findings.md` + the
  prior-art recon; DELIVER builds the egress slice through `overdrive serve` +
  `overdrive deploy` once the topology is settled.

## Design Implications
- The intercept host-side is **already production code**: the probe's nft rule is
  byte-for-byte `install_outbound_tproxy(host_veth, agent_leg_f_port)` +
  `ensure_shared_routing_infra()`
  (`crates/overdrive-worker/src/mtls_intercept.rs`). The **only new** production
  code is **tap-in-netns provisioning + CH `--net` tap attach + guest addressing**
  at `start_alloc`.
- Structural requirements the probe pinned: `modprobe nft_tproxy` (assume
  supported — appliance-kernel confirmation waived); a **host return route to the
  guest `/30`**; `net.ipv4.ip_forward=1` in the netns (routed model); the existing
  fwmark rule + `local` table 100.
- **Topology is an OPEN design decision:** routed two-`/30` (proven) vs L2-bridge
  vs guest-directly-on-workload-`/30`.
- Scope: **#222** = tap-in-netns provisioning + guest-stack intercept (tap folded
  in from #257 gap 2 on 2026-08-27). **Egress proven; INBOUND** (peer→guest
  service) is the other half of #222 and is **unprobed** (recon rates it
  lower-risk routed-IP-over-tap). **#257** (service-kind + health probes + remove
  rejection) depends on #222 and is out of scope here.

## Constraints Discovered
- A faithful probe **required a real guest kernel behind a real virtio-net tap** —
  a netns cannot model "no host `struct sock`" (its TCP terminates in the host
  kernel). Any future validation uses a real VM, not a netns stand-in.
- `CONFIG_IP_PNP` was unset on the dev kernel → the guest self-configures its NIC
  (no kernel `ip=` autoconfig). The production guest-addressing mechanism (kernel
  cmdline `ip=` vs in-guest vs DHCP) is a design choice.
