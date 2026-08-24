# SPIKE Findings — subprocess-free-veth-provisioner

**Feature:** GH [#233](https://github.com/overdrive-sh/overdrive/issues/233) (expanded scope
— eliminate ALL subprocesses in `veth_provisioner.rs` AND the `ip`/`nft` shell-outs in
`crates/overdrive-worker/src/mtls_intercept.rs`; Cloud Hypervisor stays the only
sanctioned subprocess).
**Kernel (`uname -r`):** `7.0.0-29-generic` — Lima `overdrive` VM, run as root. ethtool 6.19.
**Date:** 2026-08-24.

## Assumption tested

Can we eliminate every subprocess shell-out in
`crates/overdrive-control-plane/src/veth_provisioner.rs` — `ip link/addr/route`,
`ip netns add/del/list`, `ip link set … netns`, `ip netns exec ethtool -K/-k`,
`sysctl -w/-n` — using the `rust-netlink` crate family + direct `/proc/sys` file
I/O, **without** reintroducing the `62fa6be2` tx-offload packet-corruption
regression? Primary risk: the ethtool `FEATURES_SET` path, because the
`rust-netlink` `ethtool` crate is **GET-only** and must be hand-rolled.

## Verdicts

| Increment | Scope | Verdict |
|---|---|---|
| **A** | `rtnetlink` veth / addr / route / `NetworkNamespace::add/del` / `setns_by_fd` move | **WORKS** |
| **B** | hand-rolled `ETHTOOL_MSG_FEATURES_SET` (the trap) | **WORKS** |
| **C** | per-netns sysctl via `/proc/sys` file I/O + isolation | **WORKS** |
| **D** | mtls `ip rule fwmark` + `ip route local … table` → `rtnetlink` | **WORKS** |
| **E** | mtls nft TPROXY chain+rule → netlink (hand-rolled tproxy expr) + real divert | **WORKS** |

**Gate recommendation: PROMOTE.** Every subprocess across BOTH sites
(`veth_provisioner.rs` and `mtls_intercept.rs`) is replaceable with `rtnetlink` +
hand-rolled genl/nfnetlink encoders (ethtool `FEATURES_SET`, nft `tproxy`) + `/proc/sys`
file I/O. Cloud Hypervisor remains the only sanctioned subprocess. Per-increment analysis
for D/E in `findings-d.md` / `findings-e.md`.

## Evidence (raw, committed alongside the probe)

- `spike-scratch/subprocess-free-veth-provisioner/increment-a/evidence.txt` (+ `src/main.rs`, `Cargo.toml`)
- `spike-scratch/subprocess-free-veth-provisioner/increment-b/evidence.txt`
- `spike-scratch/subprocess-free-veth-provisioner/increment-c/evidence.txt`

Build output (~1 GB `target/`) is git-ignored via the probe dir's local
`.gitignore` (`**/target/`); the root `.gitignore` only covers the depth-1
`spike-scratch/*/target/`, which does not reach the feature-scoped nesting.

## Increment A — rtnetlink link/addr/route/netns/setns (WORKS)

`rtnetlink` 0.23.0 covers link add/del, `LinkVeth::new(a,b).build()`, `.up()`,
addr add, route (on-link + `default via`), `NetworkNamespace::add/del` (pure
`unshare`+mount, no exec), and link move via `.setns_by_fd()`. Every op verified
by netlink read-back; zero subprocesses.

**Design implications:**
- Idempotency shifts from brittle iproute2 **stderr-substring matching**
  (`"File exists"`, the multi-phrase `link_absent`) to typed netlink
  **`ErrorMessage.code`** (`-EEXIST` / `-ENODEV`). Structured read-back
  (`LinkFlags`, `AddressAttribute`, `RouteAttribute`) replaces
  `contains(",UP,")` / `inet <addr>/` string parsing.
- The **on-link/connected route is kernel-auto-created** when the addr is added,
  so an explicit on-link route add returns `-EEXIST` (`os error 17`). Treat as
  idempotent success via the typed code, not a "File exists" substring. Captured
  verbatim in `increment-a/evidence.txt:15-16`.

## Increment B — hand-rolled `ETHTOOL_MSG_FEATURES_SET` (WORKS — the primary de-risk)

The `ethtool` crate is **GET-only**: `EthtoolFeatureAttr::Wanted`'s emit is
literally `todo!("Does not support changing ethtool feature yet")` — it
**panics** if reused. The SET path must be hand-rolled at the wire level over
`genetlink` + `netlink-packet-generic` (~120 lines; `increment-b/src/main.rs` is
a working reference).

**Correction to the issue brief:** `ETHTOOL_MSG_FEATURES_SET = 12` (`0x0c`), **not
`0x0a`** — `0x0a` is `WOL_SET`.

Mechanism: enumerate the changeable `tx-checksum-*` bits via `FEATURES_GET`, then
`FEATURES_SET` each off through the `ETHTOOL_A_FEATURES_WANTED` bitset (bit
present, `BIT_VALUE` absent ⇒ value 0). This correctly reproduces the `tx`
keyword's group-expansion the netlink way (named-bit-granular). On the veth only
`tx-checksum-ip-generic` and `tx-checksum-sctp` were changeable; `-ipv4`/`-ipv6`/
`-fcoe-crc` are `[fixed]` — matching `ethtool -K dev tx off` semantics.

**Three independent oracles all PASS** (`increment-b/evidence.txt`):
- (a) netlink `FEATURES_GET` read-back: every changeable `tx-checksum-*` active `true → false`.
- (b) independent `ethtool -k`: `tx-checksumming: off` (evidence oracle only — not the mechanism).
- (c) **wire capture** — the honest byte-correctness signal:
  - offload **ON** → `10.99.0.1.56797 > 10.99.0.2.9: [bad udp cksum 0x1507 -> 0xbe24!]`
  - offload **OFF** → `10.99.0.1.44397 > 10.99.0.2.9: [udp sum ok]`

## Increment C — per-netns sysctl via `/proc/sys` (WORKS)

`/proc/sys/net/**` is **per-netns**: writes from a `setns`'d context do not leak
to the host, and distinct values coexist per-netns (isolation proven in
`increment-c/evidence.txt:56-59`). Knobs matched to the current shims:
`net.ipv4.ip_forward` and `net.ipv4.conf.all.rp_filter` (plus per-veth
`rp_filter`). The production swap must `setns` into the alloc netns before
writing, exactly as `ip netns exec sysctl` does today.

## Cross-cutting design implications for the production swap

- **`provision()` becomes `async fn`** (matches the issue). The sole call site
  (`lib.rs`, inside `run_server_…`) is already async → `.await`, no
  `spawn_blocking`.
- **`VethProvisionError` variants carry `errno`** (typed netlink
  `ErrorMessage.code`), not parsed stderr. This is the real win: no locale /
  version phrasing drift can silently reclassify EPERM as benign on the
  packet-corruption-critical path.
- **In-netns netlink AND sysctl require `setns` on a dedicated `std::thread`** —
  netns is per-thread; the tokio worker pool must never be moved. Production uses
  `nix::sched::setns` (`rtnetlink` already pulls `nix 0.30`).
- **Hand-rolled ethtool `FEATURES_SET` module** (over `genetlink` /
  `netlink-packet-generic`): GET changeable `tx-checksum-*` bits, SET each off.
  The `ethtool` crate cannot SET (Wanted emit = `todo!()`).
- **Dependencies** (workspace, adapter-host crate only): `rtnetlink` 0.23,
  `netlink-packet-route` 0.33, `netlink-packet-core` 0.9, `netlink-sys` 0.9,
  `netlink-proto` 0.13, `netlink-packet-generic` + `genetlink` (ethtool),
  `nix` 0.30. `CAP_NET_ADMIN` requirement unchanged.
- **`NetworkNamespace::add` forks internally** (child does the `unshare` so the
  caller isn't moved) but does **not** exec an external binary — honors "no
  subprocess except Cloud Hypervisor", worth naming explicitly in DESIGN.
- **Relationship to [#197](https://github.com/overdrive-sh/overdrive/issues/197):**
  this is the minimal in-place mechanism swap (structured errno + no PATH dep);
  it is a natural down-payment on #197's Host adapter but ships independently and
  does NOT add DST coverage (still real I/O in an adapter-host crate).

## mtls_intercept.rs (added scope) — design implications

- **Drop `rustables`; hand-roll the nftables netlink directly.** `rustables` 0.8.8 has
  no typed `tproxy` expression and no public raw-expression escape hatch (`nlmsg` is
  `pub(crate)`), and drags a `bindgen`/`libclang` build dep. The tproxy rule was
  hand-rolled over raw `NETLINK_NETFILTER` (working wire bytes in `findings-e.md`);
  table + chain are strictly simpler than the proven rule.
- **Keep the `ip_rule_fwmark_present` dump-then-add guard.** Naked netlink `ip rule add`
  (`NLM_F_EXCL|CREATE`) does NOT dedup fib rules — the kernel stacks a duplicate, exactly
  like iproute2. The presence-guard is load-bearing and must be ported. (`ip route add
  local` IS `-EEXIST`-idempotent.)
- **Structural handle recovery** (`GETRULE` / `NFTA_RULE_HANDLE`) replaces the current
  `# handle N` text scrape in `find_virt_rule_handle`.
- The `mtls_intercept.rs` `ip` calls (`ip rule` / `ip route local`) reuse the same
  `rtnetlink` surface as `veth_provisioner`; only the `nft` verbs are new mechanism.
