# SPIKE Decisions — subprocess-free-veth-provisioner

## Assumption Tested
- Can every subprocess in `veth_provisioner.rs` (`ip link/addr/route`, `ip netns
  add/del/list`, `ip link set … netns`, `ip netns exec ethtool -K/-k`,
  `sysctl -w/-n`) be replaced by the `rust-netlink` crate family + direct
  `/proc/sys` file I/O, WITHOUT reintroducing the `62fa6be2` tx-offload
  packet-corruption regression? Primary risk: the ethtool `FEATURES_SET` path,
  because the `ethtool` crate is GET-only and must be hand-rolled.

## Probe Verdict
- **WORKS** on all FIVE increments — kernel `7.0.0-29-generic`, root, Lima.
  A (`rtnetlink` link/addr/route/netns/setns), B (hand-rolled
  `ETHTOOL_MSG_FEATURES_SET`), C (per-netns sysctl via `/proc/sys`),
  D (mtls `ip rule`/`ip route local` → `rtnetlink`), E (mtls nft TPROXY chain+rule
  → hand-rolled netlink, proven by a real connection divert). B proven by three
  independent oracles incl. a wire-checksum contrast (`[bad udp cksum …]` ON →
  `[udp sum ok]` OFF); E proven by a real TPROXY divert with orig-dst preserved.
  Full analysis in `findings.md` + `findings-{a..e}` (a–c raw evidence.txt; d/e
  in `findings-d.md` / `findings-e.md`).

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

## Scope expansion — mtls_intercept.rs folded IN (user ruling, 2026-08-24)
`crates/overdrive-worker/src/mtls_intercept.rs`'s `ip` (×3) and `nft` (×2)
shell-outs on the transparent-mTLS inbound-TPROXY path are **in scope for this
feature**, not a separate follow-up. Spiked as increments D and E:

- **D (`ip` → rtnetlink) — WORKS.** `ip rule add fwmark 0x1 lookup 100` +
  `ip route add local 0.0.0.0/0 dev lo table 100` via `rtnetlink` (same surface
  as the veth swap). Constraint: **keep the `ip_rule_fwmark_present` dump-guard**
  — naked netlink `ip rule add` stacks duplicates (no fib-rule dedup). `findings-d.md`.
- **E (`nft` TPROXY → netlink) — WORKS (primary de-risk).** A real netns
  connection to a foreign VIP was TPROXY-**diverted** to a transparent listener
  with orig-dst preserved. Decision: **drop `rustables`** (no typed `tproxy`
  expression, no public raw-expr hatch, drags `bindgen`/`libclang`) and
  **hand-roll the nftables netlink directly**; the working tproxy wire encoding is
  captured in `findings-e.md`. Structural `GETRULE`/`NFTA_RULE_HANDLE` recovery
  replaces the `# handle N` text scrape.

## Follow-up — the xtask "ban infra subprocesses" lint (still separate, lands last)
A structural xtask lint banning shell-outs to named infra CLIs (`ip`, `nft`,
`ethtool`, `sysctl`, `tc`, `bpftool`, `iptables`) from production `src/` of
runtime crates — with a `// subprocess-ok: <reason>` marker for the Cloud
Hypervisor exception — enforces the "no subprocess except CH" principle
structurally (mirrors dst-lint). It can only flip to hard-deny AFTER this
feature lands (both sites swapped), so it is the "lock the door" step. Pending
user approval to file a tracking issue.
