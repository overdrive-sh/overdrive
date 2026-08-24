# SPIKE Findings — Increment D (mtls_intercept `ip` ops → rtnetlink)

**Feature:** GH [#233](https://github.com/overdrive-sh/overdrive/issues/233), expanded to
`crates/overdrive-worker/src/mtls_intercept.rs`.
**Verdict: WORKS.** Kernel `7.0.0-29-generic` (Lima, root, real `cargo run`).
**Evidence:** `spike-scratch/subprocess-free-veth-provisioner/increment-d/evidence.txt`.

## Scope
The two node-global `ip` shell-outs in `ensure_shared_routing_infra`:
- `ip rule add fwmark 0x1 lookup 100` (`run_ip` L649, guarded by `ip_rule_fwmark_present` L648).
- `ip route add local 0.0.0.0/0 dev lo table 100` (`ensure_ip_route_local` L741-766, EEXIST-tolerant).

Real constants: `TPROXY_FWMARK = 0x1` (L88), `TPROXY_RT_TABLE = 100` (L93).

## Result
Both replicated via `rtnetlink` 0.23, each verified by netlink read-back:
- rule → `handle.rule().add().v4().action(ToTable).fw_mark(0x1).table_id(100)`.
- route → `RouteMessageBuilder::<Ipv4Addr>::new().kind(Local).scope(Host).table_id(100)
  .destination_prefix(0.0.0.0, 0).output_interface(lo)`.

## Predicted vs actual (the load-bearing finding)
Predicted both installable + idempotent. The route was (`-EEXIST` on re-add, mapped to
`Ok` as production does at L755). But a **naked `rule add` re-issue stacked a duplicate** —
netlink `NLM_F_EXCL|CREATE` does NOT dedup fib rules (identical to iproute2 `ip rule add`).

**Design implication:** the production `ip_rule_fwmark_present` dump-then-add guard is
**load-bearing and must be ported** to the netlink path; do not rely on `EEXIST` for the
rule. The `ip route add local` path IS `-EEXIST`-idempotent and matches production's
tolerance. Probe cleaned up (0 rules / 0 routes left).
