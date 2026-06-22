# Spike findings — canonical-workload-address inbound TPROXY (GH #241)

**Probe:** `spike-scratch/increment-a/` (gitignored, throwaway, standalone
workspace; deps = `libc` + `nix` only — production helpers copied verbatim,
no `overdrive-*` dependency edge).

**Kernel (pinned to the verdict):**

```
uname -r: 7.0.0-22-generic
nftables v1.1.6 (Commodore Bullmoose #7)
```

> Dev Lima kernel is **7.0.0-22-generic**, NOT the pinned-6.18 appliance
> kernel (ADR-0068). The mechanisms exercised (netns, veth, nft TPROXY,
> `ip rule`/`ip route`, `IP_TRANSPARENT`, `getsockname`) are stable
> kernel features present since well before 6.18, so the verdict is
> expected to hold on 6.18; the authoritative re-confirmation is the
> Tier-3 matrix when the slice lands. No kernel-version-sensitive
> primitive was used.

---

## Binary verdict: **WORKS**

Both legs of the one assumption succeed on the real kernel, through two
real per-workload `/30` veths carved from `10.99.0.0/16` with production
`/30` math:

| Sub-probe | What it proves | Result |
|---|---|---|
| 1 — baseline routing (NO nft rule) | host forwards netns-B → netns-C between the two `/30`s | **PASS** |
| 2 — production-shape inbound TPROXY capture | client in netns-B `connect(workload_addr_C:port)` is captured by the nft-TPROXY rule, handed to a host-side leg-C `IP_TRANSPARENT` listener, and `getsockname()` recovers the original destination `workload_addr_C:port` | **PASS** |

**Recovered orig-dst (sub-probe 2): `10.99.0.6:18241` — exactly
`workload_addr_C:SVC_PORT`, the address the client dialed.**

---

## Topology (production `/30` math, ADR-0071)

`workload_addr = base + slot*4 + 2`; host-side gateway = `base + slot*4 + 1`;
`base = 10.99.0.0`. Spike-distinct names; production-identical addresses.

```
netns spk-ns-b : /30 10.99.0.0  workload_addr 10.99.0.2  host-gw 10.99.0.1   (client, slot 0)
netns spk-ns-c : /30 10.99.0.4  workload_addr 10.99.0.6  host-gw 10.99.0.5   (server, slot 1)
```

Host routes that result (auto-installed by `ip addr add <gw>/30 dev <host_veth>`):

```
10.99.0.0/30 dev spk-hv-b proto kernel scope link src 10.99.0.1
10.99.0.4/30 dev spk-hv-c proto kernel scope link src 10.99.0.5
```

Each workload-side veth (inside its netns) carries `workload_addr/30` and a
`default via <host-gw>` route. The two `/30`s are connected ONLY through the
host (each is on a separate host-side veth) — there is no shared subnet, so
host forwarding (or PREROUTING diversion) is the only path between them.

---

## Predicted vs actual

### Sub-probe 1 — baseline `/30` routing (no nft rule)

- **Hypothesis:** with `ip_forward=1`, the host forwards a TCP connection
  from netns-B's `workload_addr_B` to netns-C's `workload_addr_C` across
  the two `/30`s.
- **Predicted:** client connects, bytes echo back.
- **Actual (PASS):**

```
[1] baseline: listener in spk-ns-c @ 10.99.0.6:18241, client in spk-ns-b connect()s
    listener accepted from 10.99.0.2:47320
    listener read 15 bytes: [83, 80, 73, 75, 69, 45, 50, 52, 49, 45, 72, 69, 76, 76, 79]   (= "SPIKE-241-HELLO")
    client got echo: [83, 80, 73, 75, 69, 45, 50, 52, 49, 45, 65, 67, 75]                  (= "SPIKE-241-ACK")
  SUB-PROBE 1: PASS
```

The listener saw the client's REAL source `10.99.0.2:47320` (workload_addr_B,
unmasqueraded) — confirming a genuine routed path B→C, not a NAT hop.

### Sub-probe 2 — production-shape inbound TPROXY capture

- **Hypothesis:** the production nft-TPROXY rule on `workload_addr_C:port`
  (plus `ip rule fwmark`, `ip route local … table`, leg-C `IP_TRANSPARENT`)
  captures the netns-B client's `connect(workload_addr_C:port)` and
  `getsockname()` on the accepted leg-C socket recovers
  `workload_addr_C:port`.
- **Predicted:** leg-C accepts, `getsockname` = `10.99.0.6:18241`, bytes echo.
- **Actual (PASS):**

Installed ruleset (note the production-identical rule shape; only the table
name / fwmark / rt-table are spike-distinct):

```
table ip spike-mtls {
	chain prerouting {
		type filter hook prerouting priority mangle; policy accept;
		meta mark 0x00000002 accept
		ip daddr 10.99.0.6 tcp dport 18241 tproxy to 127.0.0.1:43955 meta mark set 0x00000099 accept
	}
}
```

```
[2] leg-C IP_TRANSPARENT listener @ 127.0.0.1:43955
[2] installing nft-TPROXY: ip daddr 10.99.0.6 tcp dport 18241 tproxy to 127.0.0.1:43955
    leg-C accepted from 10.99.0.2:47326
    leg-C getsockname orig-dst = 10.99.0.6:18241          <-- ORIG-DST RECOVERED CORRECTLY
    leg-C read 15 bytes: [83, 80, 73, 75, 69, 45, 50, 52, 49, 45, 72, 69, 76, 76, 79]
    client got echo: [83, 80, 73, 75, 69, 45, 50, 52, 49, 45, 65, 67, 75]
  SUB-PROBE 2: getsockname recovered 10.99.0.6:18241 (expected 10.99.0.6:18241) -> PASS
```

The client (in netns-B) dialed `10.99.0.6:18241`; the packet transited
netns-B → host PREROUTING, was TPROXY'd to the host-local leg-C socket at
`127.0.0.1:43955`, leg-C accepted it, and `getsockname` on the accepted
(IP_TRANSPARENT) socket returned the **original** dialed destination
`10.99.0.6:18241` — not the leg-C local `127.0.0.1:43955`. This is the exact
production orig-dst-recovery semantic (NOT `SO_ORIGINAL_DST` — `getsockname`
on an `IP_TRANSPARENT` socket).

---

## The routing recipe that worked — and which knobs are LOAD-BEARING

The probe set four classes of knob, then ISOLATED each by toggling it. The
findings below correct an initial over-assumption (I expected `rp_filter`
relaxation to be required for the TPROXY asymmetric path; **it is not**, for
this topology).

### REQUIRED for inbound TPROXY capture (sub-probe 2)

These are exactly the components production's `ensure_shared_routing_infra` +
`make_transparent_listener` already install — nothing more:

1. **nft-TPROXY prerouting rule** (per-virt):

   ```
   nft add table ip <table>
   nft add chain ip <table> prerouting { type filter hook prerouting priority mangle; policy accept; }
   nft insert rule ip <table> prerouting meta mark 0x00000002 accept   # F5 leg-S exemption, once at head
   nft add rule ip <table> prerouting ip daddr <workload_addr_C> tcp dport <port> tproxy to 127.0.0.1:<leg_c_port> meta mark set <fwmark> accept
   ```

2. **fwmark policy route** — diverts the marked packet to the local table so
   the kernel delivers it to a local socket instead of forwarding:

   ```
   ip rule add fwmark <fwmark> lookup <table>
   ip route add local 0.0.0.0/0 dev lo table <table>
   ```

3. **leg-C listener with `IP_TRANSPARENT`** (sockopt level `IPPROTO_IP`,
   optname `19`, value `1`) bound to `127.0.0.1:<port>`. Without
   `IP_TRANSPARENT` the kernel will not deliver a foreign-dest (TPROXY'd)
   packet to the socket, and `getsockname` would not carry orig-dst.

### NOT required for capture (isolated, surprising)

- **`ip_forward` is NOT required for capture.** With
  `SPIKE_NO_FORWARD=1` (i.e. `net.ipv4.ip_forward=0`), **sub-probe 2 still
  PASSED** with correct orig-dst recovery, while sub-probe 1 (baseline)
  FAILED (`connection timed out`). The TPROXY rule fires at PREROUTING
  (priority `mangle`) and the fwmark + `local` route divert the packet to
  the host-local leg-C socket *before* the forwarding decision, so the
  captured connection terminates locally and `ip_forward` is irrelevant to
  it.

  ```
  net.ipv4.ip_forward=0
  ...
  SUB-PROBE 1: client connect(10.99.0.6:18241) failed: connection timed out  -> FAIL
  SUB-PROBE 2: getsockname recovered 10.99.0.6:18241 ... -> PASS
  ```

- **`rp_filter` relaxation is NOT load-bearing for this topology.** With
  strict `SPIKE_RPF=1` (`rp_filter=1` on all/default/both host veths),
  **BOTH sub-probes still PASSED.** The TPROXY'd reply originates from the
  host-local leg-C socket (via the `local` route in the policy table), not a
  forwarded asymmetric path, so reverse-path filtering on the veths is not
  triggered for the captured flow; and the baseline B→C path is symmetric
  (request and reply traverse the same `/30` routes). The probe's default
  `rp_filter=2` was a precaution that the isolation test showed to be
  unnecessary here.

  ```
  net.ipv4.conf.all.rp_filter=1 (+ default + both veths)
  SUB-PROBE 1: PASS
  SUB-PROBE 2: PASS  (recovered orig-dst: Some(10.99.0.6:18241))
  ```

### REQUIRED only for the no-rule baseline path (sub-probe 1)

- **`ip_forward=1`** — required for one workload to reach another workload's
  `workload_addr` directly when NO intercept rule is present (workload-to-
  workload connectivity between the two `/30`s). This is a property of the
  canonical-address model itself (workloads can be addressed by their
  `workload_addr` across `/30`s), independent of mTLS interception.

### Net recipe summary

| Knob | Capture (sub-probe 2) | Baseline B→C (sub-probe 1) |
|---|---|---|
| nft-TPROXY rule + fwmark `ip rule` + `local` route | **REQUIRED** | n/a |
| leg-C `IP_TRANSPARENT` | **REQUIRED** | n/a |
| `net.ipv4.ip_forward=1` | not required | **REQUIRED** |
| `rp_filter` relaxation | not required | not required |

---

## Edge cases / surprises

1. **Capture is forwarding-independent (decoupled requirements).** The most
   design-relevant surprise: the inbound intercept does NOT depend on
   `ip_forward`. The production routing-convergence code's `ip_forward`
   handling (if any) is for the *workload-to-workload reachability* concern,
   not for making interception work. These are two separate requirements and
   should be reasoned about separately.

2. **`rp_filter` was a non-issue here.** The classic TPROXY "strict rp_filter
   drops the asymmetric reply" trap did NOT fire, because the captured flow's
   reply is host-local (off the `local` route), not a forwarded asymmetric
   path. Production should NOT add `rp_filter` relaxation on the strength of
   this concern alone — it was unneeded in this topology. (Caveat: a more
   complex topology where the *reply* must egress a different interface than
   the request arrived on could still trip rp_filter; this probe's topology
   does not exercise that, and the production inbound path mirrors this
   probe's host-local-termination shape, so the same exemption applies.)

3. **The F5 leg-S exemption (`meta mark 0x00000002 accept`) at the chain head
   is rendered by nft as the zero-padded `0x00000002`** — matching the
   production `dump_has_leg_s_exemption` canonicalisation
   (`{leg_s_mark:#010x}`). The spike installed it once at the head, ahead of
   the per-virt TPROXY rule, exactly as production does. It was not exercised
   for a *match* in this probe (no leg-S dial), but its presence and ordering
   are confirmed in the dumped ruleset.

4. **Source IP is preserved through the captured path.** leg-C saw the
   client's real source `10.99.0.2:<ephemeral>` (workload_addr_B), confirming
   TPROXY is transparent on the source side too — no SNAT/masquerade hop.
   This matters for the downstream mTLS peer-identity reasoning (the server
   side can see who dialed).

5. **`getsockname` (not `SO_ORIGINAL_DST`) is the correct orig-dst recovery
   for TPROXY.** Confirmed: `getsockname` on the `IP_TRANSPARENT` accepted
   socket returns the dialed `workload_addr_C:port`, matching the production
   `getsockname_orig` shape and the transparent-mtls increment-i finding.

6. **The Lima VM's own `table ip nat` (LIMADNS) coexists fine** with the
   spike's `table ip spike-mtls` filter/prerouting chain — different tables,
   different hooks/priorities, no interaction observed. Confirms the
   production "shared table + per-rule append, never raze" model coexists with
   pre-existing host nft state.

7. **Teardown is robust.** Verified after BOTH a clean exit-0 run and the
   exit-1 (`ip_forward=0`) run: no leftover netns, veth, nft `spike-mtls`
   table, `ip rule fwmark`, or rt-table entries. (One non-isolated side
   effect: global `net.ipv4.ip_forward` / `net.ipv4.conf.all.rp_filter` are
   process-global sysctls the probe mutates and does NOT restore — re-set to
   sane values manually after the run. Production convergence code will own
   these idempotently.)

---

## Design implications for #241

1. **The production routing the slice must install for inbound capture is
   exactly what `mtls_intercept::ensure_shared_routing_infra` +
   `install_inbound_tproxy` already build** — the per-virt nft-TPROXY rule on
   `ip daddr <workload_addr> tcp dport <port>`, the shared `ip rule fwmark →
   table 100`, the `ip route local 0.0.0.0/0 dev lo table 100`, and the leg-C
   `IP_TRANSPARENT` listener. **No new routing primitive is needed.** The
   #241 gap was never the routing mechanism; it was the *production wiring of
   the inbound rule into a real `serve` + `deploy` path* (the inbound rule was
   deferred — `start_alloc` recorded `tproxy_guard = None`). This probe
   confirms the mechanism composes correctly over two real `/30`s.

2. **The per-virt inbound rule keys on `workload_addr` (`net + slot*4 + 2`),
   not a VIP.** The probe used `ip daddr 10.99.0.6` (workload_addr_C) and it
   worked end-to-end. The canonical-address model (advertise the per-workload
   `workload_addr`, install the inbound rule for it, route between the `/30`s)
   is the thin, production-drivable slice the CLAUDE.md vertical-slice rule
   calls for — and this probe is its walking-skeleton evidence.

3. **Shared infra vs per-virt rule: converge-on-boot (Bar 1) vs per-alloc.**
   Per `.claude/rules/reconcilers.md`:
   - The **shared infra** (`ip rule fwmark`, `ip route local … table`, nft
     table+chain, F5 exemption) is node-global converge-on-boot state —
     ensured idempotently once, never torn down per-workload. The probe's
     `ensure_shared_routing_infra` (add-if-missing, EEXIST-tolerant) is the
     correct Bar-1 shape. This matches the production code and the
     `#234`-tracked Bar-2-when-drift-matters deferral.
   - The **per-virt TPROXY rule** is **per-alloc** — appended on
     `on_alloc_running`, removed by RAII guard (`TproxyInterceptGuard`) on
     alloc teardown by kernel-assigned handle. The probe confirms a single
     per-virt rule captures correctly; the multi-concurrent-rule coexistence
     is the production-code concern (already handled by the shared-chain +
     per-handle-delete design).
   - **Workload-to-workload reachability** (`ip_forward=1`, the per-`/30`
     routes) is a separate node-global converge-on-boot concern from the
     interception rule — the probe proved they're decoupled. Whatever
     installs the `/30` veths + `ip_forward` is its own slice/reconciler,
     independent of the inbound-intercept rule install.

4. **No `rp_filter` munging needed in the production inbound path.** Do not
   add `rp_filter` relaxation to the #241 routing-convergence code on the
   strength of a generic TPROXY-asymmetric-path concern — this probe shows it
   is unneeded for the host-local-termination shape the production inbound
   path uses. (If a future topology genuinely needs it, gate that on a
   real-kernel signal, not the generic worry.)

5. **Cross-check against Cilium per-endpoint-route model:** Cilium's
   per-endpoint `/32` host routes + TPROXY for L7 proxy redirection use the
   identical `ip rule fwmark` + `local` route + `IP_TRANSPARENT` shape; the
   `/30`-per-workload model here is a coarser variant of the same kernel
   mechanism. The positive verdict is consistent with that production
   precedent — no contradiction found.

---

## Gate recommendation: **PROMOTE**

The one assumption holds on the real kernel with the production rule shape
over two real production-math `/30`s, orig-dst recovered exactly. The routing
recipe required for capture is precisely what production already builds, and
the isolation tests sharpened the recipe (capture is forwarding-independent;
`rp_filter` relaxation is unneeded). Promote to the walking-skeleton slice:
advertise `workload_addr`, install the inbound per-virt TPROXY rule on the
production `serve` + `deploy` path, and wire `ip_forward` + the `/30` routes
as their own node-global converge step.
