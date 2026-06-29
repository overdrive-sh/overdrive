# Spike findings — output-hook leg-B re-dial interception

**Feature:** dial-by-name-responder
**Probe:** `spike-scratch/increment-c/` (gitignored; nft/ip CLI + Rust `IP_TRANSPARENT` listener, no eBPF, no `crates/` touched)
**Kernel:** `uname -r` = `7.0.0-22-generic` (dev-Lima; merge gate is the pinned-6.18 Tier-3 matrix, ADR-0068 — re-confirm on 6.18 at DEVOPS)
**Run:** for real, as root, under Lima (no `--no-run`, no compile-only gate)
**Date:** 2026-06-27
**Authored by:** orchestrator, from the spike crafter's report (the crafter's own file-write was blocked by a guard; the decisive program output below is its real stdout, re-emitted verbatim).

---

## Hypothesis / Predicted / Falsification (written before probing)

- **Hypothesis:** an `output`-hook nft companion to the inbound interception + the existing fwmark `ip rule`→`local` route + an `IP_TRANSPARENT` leg-C listener can divert a **host-locally-originated** connect to `(workload_addr, port)` into the transparent listener (recovering orig-dst via `getsockname`), reproducing the leg-B re-dial the agent makes when the resolved frontend `F` ≠ the backend `workload_addr`.
- **Predicted (WORKS):** the un-marked local connect lands on the `IP_TRANSPARENT` listener (not the plaintext decoy), `getsockname` = `(workload_addr, port)`.
- **Falsification:** the connect lands on the plaintext decoy, is dropped, or errors — and/or steering locally-generated (output-path) packets needs an `iif lo`-aware `ip rule` beyond the prerouting path's infra.

---

## Verdict: **WORKS**

An nft `output`-hook companion **does** divert a host-locally-originated leg-B connect into a leg-C-style `IP_TRANSPARENT` listener, and `getsockname` recovers the exact orig-dst.

**The crux unknown is resolved with ONE load-bearing delta:** the output chain must be **`type route hook output`** (NOT `type filter`). `type route` forces a kernel route **re-evaluation** after the `meta mark set`, so the *existing* fwmark `ip rule` → `local` table route fires on the OUTPUT path. **No `iif lo` clause is needed.**

### Decisive real output (program stdout)

| Case | Mark | Expected | Observed |
|---|---|---|---|
| leg-B re-dial | un-marked | intercept | `landed on TRANSPARENT (intercepted)`, `getsockname_orig_dst=10.99.0.2:18951` ✓ |
| leg-S dial | `SO_MARK 0x2` | exempt | `landed on DECOY` (exemption fires, not diverted) ✓ |
| unrelated daddr | un-marked | no capture | `landed on DECOY` (no over-capture) ✓ |
| **counter-test:** same recipe, `type filter` instead of `type route` | un-marked | — | `landed on DECOY` — **proves `type route` is necessary** |

The `type filter` counter-test is the falsification control: with everything else identical, a `filter`-hook output chain does NOT trigger the route re-lookup, so the packet is not diverted and lands on the plaintext decoy — exactly the production symptom today.

---

## The exact working incantation (production-promotable)

```sh
# (1) shared infra — UNCHANGED, already installed by ensure_shared_routing_infra:
ip rule  add fwmark 0x1 lookup 100
ip route add local 0.0.0.0/0 dev lo table 100

# (2) NEW output chain — `type route` (NOT `filter`) is the load-bearing delta:
nft add chain ip overdrive-mtls output \
  '{ type route hook output priority mangle; policy accept; }'

# (3) leg-S exemption at the output chain head (mirrors the prerouting F5 exemption):
nft insert rule ip overdrive-mtls output meta mark 0x2 accept

# (4) per-workload output divert:
nft add rule ip overdrive-mtls output \
  ip daddr <workload_addr> tcp dport <port> \
  meta mark != 0x2 meta mark set 0x1 accept
```

Plus: the leg-C `IP_TRANSPARENT` listener must also set **`IP_FREEBIND`** to bind the non-local `workload_addr` on the output path.

`0x1` = the existing shared fwmark (`ensure_shared_routing_infra`); `0x2` = `MTLS_LEG_S_DIAL_MARK`.

---

## Cross-check against a production precedent

Cilium's from-host / host-firewall path steers locally-originated traffic with mark-based policy routing that relies on a **`mangle OUTPUT` route re-lookup** — `type route hook output` is the nftables equivalent of that route re-evaluation. The WORKS recipe matches that precedent; the DOESN'T-WORK `type filter` shape is precisely the limitation Cilium avoids. The surprising "`type route`, not `type filter`" verdict is therefore corroborated, not a probe artifact.

---

## Edge cases probed

- **leg-S exemption works:** the `meta mark 0x2 accept` head rule keeps the agent's *inbound* leg-S dial (already `SO_MARK 0x2`-stamped, `mtls/mod.rs:624`) from being diverted — it must reach the workload directly. Confirmed (lands on decoy).
- **No over-capture:** an un-related un-marked local connect to a different daddr is not diverted. Confirmed.
- **Recursion:** the leg-B dial is un-marked (`mtls/mod.rs:612`) and SHOULD be intercepted; the leg-S exemption prevents the inbound re-dial from looping. No recursion observed.

---

## Design implications for the production change (`crates/overdrive-worker/src/mtls_intercept.rs`)

- **`ensure_shared_routing_infra` (~L506):** add an idempotent create-if-missing for an `output` chain (`type route hook output priority mangle`, policy accept) + the leg-S exemption rule at its head. The `ip rule` / `ip route` lines are **unchanged** (the existing fwmark→table-100→`local` route already covers the output path once `type route` re-triggers the lookup). Add a `NFT_OUTPUT_CHAIN` const.
- **`install_inbound_tproxy` (~L248):** append a companion `output` divert rule (`ip daddr <virt> tcp dport <port> meta mark != 0x2 meta mark set 0x1 accept`) alongside the existing prerouting `tproxy` rule. The leg-C listener adds `IP_FREEBIND`.
- **`TproxyInterceptGuard::Drop` / `sweep_per_workload_tproxy_rules`:** the new output rule has **no `tproxy` verb** (it is `meta mark set`), so the teardown classifier `per_workload_rule_handles_in_dump`'s `"tproxy to "` predicate must be **widened** to also reap the output divert rule. This is a real teardown-classifier change, not a no-op — miss it and output rules leak across allocs.

---

## ONE architect decision to pin before a crafter writes the production change

Per CLAUDE.md "Implement to the design — never invent API surface": the leg-C **binding shape on the output path** is the open design choice —
- **(proven, recommended)** bind `workload_addr:port` with `IP_FREEBIND` on the leg-C listener (spike-proven; simpler), **vs**
- **(not probed)** `meta mark set` + DNAT/`redirect` to `127.0.0.1:<legC>`.

Only the `IP_FREEBIND` shape is proven here. The architect must pin this (and ratify the `type route hook output` datapath surface into ADR-0072 / the feature-delta) before implementation.

---

## Gate recommendation: **PROMOTE**

The mechanism works, is falsification-tested, matches a production precedent, and the production-change shape is fully specified above. Promote to the walking-skeleton completion: implement the output-hook companion into `mtls_intercept.rs` so the existing 02-02 walking-skeleton Tier-3 tests close the loop.
