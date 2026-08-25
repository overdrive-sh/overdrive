# SPIKE Findings — Increment E (mtls_intercept `nft` TPROXY → netlink) — PRIMARY de-risk

**Feature:** GH [#233](https://github.com/overdrive-sh/overdrive/issues/233), expanded to
`crates/overdrive-worker/src/mtls_intercept.rs`.
**Verdict: WORKS.** Kernel `7.0.0-29-generic` (Lima, root, real `cargo run`).
**Evidence:** `spike-scratch/subprocess-free-veth-provisioner/increment-e/evidence.txt`.

## Scope
The nft TPROXY table/chain/rule + handle recovery + by-handle delete in
`ensure_shared_routing_infra` (L642+), `install_inbound_tproxy` (L293+),
`find_virt_rule_handle` (L902+), `TproxyInterceptGuard::drop` (L1111+).

## Crate finding (the crux) — `rustables` cannot do it; hand-rolled
`rustables` 0.8.8 (rustwall fork of Mullvad `nftnl-rs`):
- **No typed `tproxy` expression** (`grep -rli tproxy` over its src = empty), AND
- **No public raw-expression escape hatch** — its `nlmsg` module (the only path to build
  an `ExpressionRaw`) is `pub(crate)`, and `ExpressionRaw`'s field is private.
- Also drags **`bindgen` 0.72 + `libclang`** at build time (generates its `sys` consts
  from kernel `nf_tables.h`) — a real production cost.

So the tproxy verb was **hand-rolled over a raw `NETLINK_NETFILTER` socket** (batch
NEWRULE with payload/cmp/immediate/tproxy/meta/verdict expressions, modelled on
`net/netfilter/nft_tproxy.c`). `rustables` still handled table + chain + the leg-S
exemption + structural handle read-back (`list_rules_for_chain`) + by-handle delete.

## Working `tproxy` expression — exact wire bytes (kernel-accepted)
```
2c 00 01 80                              NFTA_LIST_ELEM|NESTED  len=44
  0b 00 01 00  74 70 72 6f 78 79 00 00   NFTA_EXPR_NAME "tproxy\0"
  1c 00 02 80                            NFTA_EXPR_DATA|NESTED  len=28
    08 00 01 00  00 00 00 02             NFTA_TPROXY_FAMILY   = be32 2 (NFPROTO_IPV4)
    08 00 02 00  00 00 00 01             NFTA_TPROXY_REG_ADDR = be32 1 (NFT_REG_1)
    08 00 03 00  00 00 00 02             NFTA_TPROXY_REG_PORT = be32 2 (NFT_REG_2)
```
Paired typed immediates load `REG_1 = 127.0.0.1` (network-order octets) and
`REG_2 = agent_port` (be16). All nft integer attrs are big-endian on the wire
(`nla_get_be32`); netlink message/attr headers (len,type) are host order. tproxy is
prerouting-`mangle`-only (chain: `type filter hook prerouting priority mangle` = -150).

## Predicted vs actual — the divert is REAL
Predicted install + divert. Kernel accepted the 584-byte NEWRULE; `nft -a list chain`
(evidence-only) rendered it byte-identically to the production rule:
`ip daddr 10.200.0.1 tcp dport 9999 tproxy to 127.0.0.1:47001 meta mark set 0x00000001
accept # handle 3` (matches `install_inbound_tproxy` L303-323). A real netns client
connection to the FOREIGN dst `10.200.0.1:9999` was **diverted** to the
`127.0.0.1:47001` `IP_TRANSPARENT` listener, `getsockname` on the accepted socket
returned `10.200.0.1:9999` (orig-dst preserved), and a byte-distinct REQUEST/RESPONSE
round-tripped. Handle 3 recovered structurally from the NEWRULE dump; deleted by handle
(2 → 1). Full plumbing (fwmark rule + local route via rtnetlink, from increment-D) made
the whole mechanism path subprocess-free.

## Design implications for the production swap
- **Drop `rustables`; hand-roll the nftables netlink directly.** It cannot express the
  load-bearing tproxy verb and adds a `bindgen`/`libclang` build dep. table + chain are
  strictly simpler than the rule already proven.
- **Structural handle recovery** (`GETRULE` / `NFTA_RULE_HANDLE`) replaces the current
  `# handle N` text scrape.
- The captured tproxy encoding above is the reference for the production expression.
