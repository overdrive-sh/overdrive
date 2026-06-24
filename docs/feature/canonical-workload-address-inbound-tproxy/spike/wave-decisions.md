# SPIKE Decisions — canonical-workload-address-inbound-tproxy (GH #241)

## Assumption Tested

Host routing between two per-workload `/30` veths in `10.99.0.0/16` (production
`/30` math, ADR-0071: `workload_addr = base + slot*4 + 2`) lets a client inside
workload-B's netns `connect(workload_addr_C:port)`:

- (a) reach workload-C's netns when **no** rule is present (baseline `/30`
  forwarding), AND
- (b) be **captured** by a production-shape inbound nft-TPROXY rule on
  `workload_addr_C:port`, handed to a host-side leg-C `IP_TRANSPARENT`
  listener, with `getsockname()` recovering the original destination
  `workload_addr_C:port`.

## Probe Verdict

**WORKS** — kernel `7.0.0-22-generic`, nftables v1.1.6. Both sub-probes PASS;
orig-dst recovered exactly (`10.99.0.6:18241` = `workload_addr_C:port`). The
routing recipe required for capture is **exactly** what production's
`mtls_intercept::ensure_shared_routing_infra` + `install_inbound_tproxy` +
`make_transparent_listener` already build — **no new routing primitive is
needed**. Full evidence (pasted real output) in `findings.md`.

Two sharpening findings:

- **Capture is forwarding-independent** — the inbound intercept does NOT
  depend on `ip_forward`; the TPROXY fwmark + `local` route divert the packet
  to the host-local leg-C socket before the forwarding decision.
  `ip_forward=1` is only required for the *no-rule* workload↔workload
  reachability path (a separate node-global concern).
- **`rp_filter` relaxation is NOT load-bearing** for this host-local-
  termination topology (both sub-probes pass with strict `rp_filter=1`).

## Promotion Decision

**DISCARD** (user, 2026-06-22). The mechanism is proven and the remaining work
is design + production wiring that carries a **design-sensitive contract** —
the `BackendDiscoveryBridge` `host_ipv4`→`workload_addr` change is an
**input-model change** (the bridge is `host_ipv4`-keyed at construction today;
advertising `workload_addr` requires a per-alloc slot→addr input), not a value
swap. Per CLAUDE.md "Implement to the design — never invent API surface," that
contract is pinned in DESIGN **before** any production wiring. The findings are
sufficient evidence for DESIGN to proceed; no walking skeleton is built.

Hand straight to DESIGN.

## Walking Skeleton

N/A (DISCARD). No skeleton committed; **no `crates/` production code touched.**
Probe code (`spike-scratch/increment-a/`, gitignored throwaway) deleted after
the gate; `findings.md` is the preserved, reproducible evidence.

## Design Implications (for the DESIGN wave to pin)

1. **No new routing primitive.** The inbound-capture path the slice must
   install is the existing production triple: per-virt nft-TPROXY rule on
   `ip daddr <workload_addr> tcp dport <port>` → `tproxy to 127.0.0.1:<leg_c>`,
   the shared `ip rule fwmark → table 100` + `ip route local 0.0.0.0/0 dev lo
   table 100`, and the leg-C `IP_TRANSPARENT` listener. The #241 gap is the
   **production wiring** (`start_alloc` records `tproxy_guard = None`), not the
   mechanism.

2. **Per-virt inbound rule keys on `workload_addr` (`net + slot*4 + 2`), not a
   VIP.** Confirmed end-to-end with `ip daddr 10.99.0.6`. `start_alloc` already
   holds the slot's `workload_addr`, so the production inbound install does not
   require a new data source.

3. **`BackendDiscoveryBridge` advertise change is design-sensitive — pin the
   contract.** Today `BackendDiscoveryBridge::new(host_ipv4, …)` advertises
   `(host_ipv4, listener.port)` (`backend_discovery_bridge.rs:343-353`). #241's
   one-source/two-readers invariant (D-TME-10) needs it to advertise
   `workload_addr:port`. This is an **input-model change** (per-alloc slot→addr
   from the NetSlot allocator / a C3 `on_alloc_running` hook), and the bridge
   lives in `overdrive-core`. DESIGN must pin: the exact bridge input shape,
   where the slot→addr mapping is sourced, and how the reconciler reads it —
   without inventing API surface.

4. **Routing-converge ownership.** Per `.claude/rules/reconcilers.md`:
   - shared infra (`ip rule`, `ip route local`, nft table/chain, F5 exemption)
     = node-global **converge-on-boot (Bar 1)**, add-if-missing — matches
     production `ensure_shared_routing_infra`; Bar-2-on-drift is the
     [#234](https://github.com/overdrive-sh/overdrive/issues/234) deferral.
   - per-virt TPROXY rule = **per-alloc** (append on `on_alloc_running`,
     RAII-delete by handle on teardown).
   - workload↔workload reachability (`ip_forward=1` + the `/30` routes) is a
     **separate** node-global converge concern, decoupled from the intercept
     rule. DESIGN must decide where this lives (the `/30` veths are provisioned
     by `veth_provisioner`; `ip_forward` ownership needs an explicit home).

5. **Thinnest live loop for DELIVER's walking skeleton** (per the issue): a
   workload can **dial the concrete `workload_addr` directly — no DNS needed to
   prove the loop**. The bridge advertise change + service resolution can be a
   later, independently-drivable concern; the production inbound install + `/30`
   routing is the loop that makes inbound *usable* through `serve` + `deploy`.

## Constraints Discovered

- **Do NOT add `rp_filter` munging** to the #241 inbound path on a generic
  TPROXY-asymmetric-path worry — unneeded for the host-local-termination shape
  production uses. (Gate any future need on a real-kernel signal.)
- **Process-global sysctls** (`net.ipv4.ip_forward`, `rp_filter`) must be owned
  idempotently by the convergence code — the probe mutated and did not restore
  them (a throwaway-probe concern, but a flag for the production owner).
- **Kernel pin caveat:** verdict is on dev Lima `7.0.0-22`, not the pinned
  6.18 appliance kernel (ADR-0068). All primitives used predate 6.18; the
  authoritative re-confirmation is the Tier-3 matrix when the slice lands.
- `getsockname` (NOT `SO_ORIGINAL_DST`) is the correct orig-dst recovery on the
  `IP_TRANSPARENT` accepted socket — matches production `getsockname_orig`.
- Source IP is preserved through the captured path (no SNAT hop) — leg-C sees
  the client's real `workload_addr_B:<ephemeral>`, relevant to downstream mTLS
  peer-identity reasoning.

## Next Wave

**DESIGN** (`nw-solution-architect` / `nw-design`) — reads `findings.md` + this
file, pins the four design-sensitive contracts above (bridge input-model,
routing-converge ownership, `start_alloc` install contract, `ip_forward`
ownership) before any production wiring.
