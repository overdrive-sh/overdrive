# SPIKE Decisions — subprocess-free-veth-provisioner

## Assumption Tested
- Can every subprocess in `veth_provisioner.rs` (`ip link/addr/route`, `ip netns
  add/del/list`, `ip link set … netns`, `ip netns exec ethtool -K/-k`,
  `sysctl -w/-n`) be replaced by the `rust-netlink` crate family + direct
  `/proc/sys` file I/O, WITHOUT reintroducing the `62fa6be2` tx-offload
  packet-corruption regression? Primary risk: the ethtool `FEATURES_SET` path,
  because the `ethtool` crate is GET-only and must be hand-rolled.

## Probe Verdict
- **WORKS** on all three increments — kernel `7.0.0-29-generic`, root, Lima.
  A (`rtnetlink` link/addr/route/netns/setns), B (hand-rolled
  `ETHTOOL_MSG_FEATURES_SET`), C (per-netns sysctl via `/proc/sys`). Increment B
  proven by three independent oracles, incl. a wire-checksum contrast
  (`[bad udp cksum …]` ON → `[udp sum ok]` OFF). Full analysis in `findings.md`.

## Promotion Decision
- **PROMOTE** (user, 2026-08-24). Build the thinnest LIVE slice: swap the
  low-risk `rtnetlink` link/addr/route/up shims into production
  `veth_provisioner.rs`, drive through a real `overdrive serve` + `overdrive
  deploy`, one Tier-3 acceptance test (`reverse_nat_e2e`-shape) green. Defer the
  ethtool `FEATURES_SET`, netns-move, and per-netns sysctl swaps to DELIVER
  slices. DESIGN designs the rest around the already-working skeleton.

## Design Implications
- `provision()` → `async fn` (sole call site already async → `.await`).
- `VethProvisionError` variants carry typed netlink `errno`, not parsed stderr.
- In-netns netlink AND sysctl require `setns` on a dedicated `std::thread`
  (`nix::sched::setns`); never move a tokio worker.
- Hand-rolled ethtool `FEATURES_SET` module over `genetlink` /
  `netlink-packet-generic` (the `ethtool` crate's `Wanted` emit is `todo!()`).
- Deps (adapter-host only): `rtnetlink` 0.23, `netlink-packet-route` 0.33,
  `netlink-packet-core` 0.9, `netlink-sys` 0.9, `netlink-proto` 0.13,
  `netlink-packet-generic` + `genetlink`, `nix` 0.30. `CAP_NET_ADMIN` unchanged.

## Constraints Discovered
- `ETHTOOL_MSG_FEATURES_SET = 12` (`0x0c`), NOT `0x0a` — the issue body is wrong
  (`0x0a` is `WOL_SET`).
- The on-link connected route is kernel-auto-created by `addr add`, so an
  explicit on-link route add returns `-EEXIST` → treat as idempotent success via
  the typed code.
- `NetworkNamespace::add` forks internally (no exec) — honors the
  "no subprocess except Cloud Hypervisor" rule; name it explicitly in DESIGN.

## Out-of-feature (surfaced by the subprocess sweep — needs its own track)
- `crates/overdrive-worker/src/mtls_intercept.rs` also shells out to `ip` (×3)
  and `nft` (×2) on the transparent-mTLS inbound-TPROXY path. Same violation
  class as the veth shims. The `ip` calls generalize to the same `rtnetlink`
  swap; **`nft` needs its own netlink nftables mechanism** (`rustables` /
  hand-rolled nfnetlink) with its own rule-encoding trap — a separate spike,
  exactly as the ethtool half warranted one here. A workspace-wide "ban infra
  subprocesses" xtask lint therefore cannot go green until BOTH this feature and
  the `mtls_intercept` `ip`/`nft` swap land. Tracked separately (pending user
  approval to file an issue).
