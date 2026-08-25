# Evolution — subprocess-free-veth-provisioner (GH #233, ADR-0085)

**Finalized:** 2026-08-25. **Feature slug:** `subprocess-free-veth-provisioner`.
**Decision record:** ADR-0085 (accepted; not amended by this record).
**Waves run:** SPIKE → DESIGN → DISTILL → DELIVER → FINALIZE (DISCUSS/DISTILL
user-skipped; feature started at SPIKE by user choice).

---

## 1. What this is — a mechanism swap, nothing more

A **behaviour-preserving mechanism swap**: every `ip` / `nft` / `ethtool` /
`sysctl` subprocess shell-out on the dataplane-provisioning path was replaced by
direct **netlink + `/proc/sys` file I/O**. Cloud Hypervisor remains the ONLY
sanctioned subprocess in a production binary. No user-facing surface changed; the
pure derivation/diff cores stayed **byte-identical**.

Two production files were swapped:

- `crates/overdrive-control-plane/src/veth_provisioner.rs` — the host-netns
  single-node veth pair (driven by `overdrive serve` boot, `lib.rs:2133`) and the
  per-alloc netns + veth provisioning (driven by `overdrive deploy` →
  `start_alloc`). `ip link/addr/route`, `ip netns add/del/list`, `ip link set …
  netns`, `ip netns exec ethtool -K/-k`, `sysctl -w/-n` → rtnetlink +
  setns'd `/proc/sys` + hand-rolled ethtool genl.
- `crates/overdrive-worker/src/mtls_intercept.rs` — the transparent-mTLS
  inbound/outbound nft-TPROXY path plus `ip rule fwmark` / `ip route add local
  … table 100` (driven by `overdrive deploy` → `start_alloc` →
  `install_*_tproxy` → `ensure_shared_routing_infra`, and the `overdrive serve`
  boot sweep). `ip rule`/`ip route local` → rtnetlink; `nft` table/chain/tproxy
  → hand-rolled `NETLINK_NETFILTER`.

### What shipped

| Component | Path | Disposition |
|---|---|---|
| `overdrive-netlink` crate (`crate_class = "adapter-host"`) | `crates/overdrive-netlink/` | **NEW** — rtnetlink client (`client.rs`), hand-rolled ethtool `FEATURES_SET=0x0c` genl encoder (`ethtool.rs`), hand-rolled nft `NETLINK_NETFILTER` tproxy encoder (`nft.rs`), `in_netns` setns helper (`setns.rs`), the `block_on_*` sync→async bridge (`runtime.rs`), and the errno-carrying `NetlinkError` (`error.rs`). |
| `veth_provisioner.rs` | `crates/overdrive-control-plane/src/veth_provisioner.rs` | **EXTEND** — netlink swap; `provision()` became `async`; observers read structured `LinkFlags(IFF_UP)` / presence-or-`ENODEV` / `ETHTOOL_A_FEATURES_*` bitset. |
| `mtls_intercept.rs` | `crates/overdrive-worker/src/mtls_intercept.rs` | **EXTEND** — netlink swap; `InterceptError::TproxyInstall{reason:String}` catch-all decomposed into four typed per-site variants. |
| `ban-infra-subprocess` lint clause | `xtask/src/dst_lint.rs` (+ `lib.rs`/`main.rs` wiring) | **NEW** — structural xtask lint forbidding the seven named infra-CLI literals (`ip`/`nft`/`ethtool`/`sysctl`/`tc`/`bpftool`/`iptables`) as `Command::new("<tool>")` args in `{core,adapter-host}` production `src/`, minus `overdrive-testing`. |

### What was deleted (single-cut, with tests, per CLAUDE.md deletion discipline)

The observation **text-parsers** that scraped `ip`/`ethtool`/`nft` CLI output —
`link_state`, `link_absent`, `tx_checksumming_on`, `ip_rule_dump_has_fwmark` /
`ip_rule_fwmark_present`, the `# handle N` scrape family (`find_virt_rule_handle`,
`output_divert_handle_in_dump`, `find_egress_rule_handle_in_dump`,
`dump_has_egress_rule`), `stderr_reports_absent_chain`, `dump_has_leg_s_exemption`
— became dead code the moment their observer read structured netlink/genl
attributes, and were deleted **with their `#[cfg(test)]` unit tests** in the slice
that landed each structured replacement (ADR-0085 D10 / DDD-13). The like-named
**test-side** helpers in `adopt_on_restart.rs` observe the kernel directly via
`nft`/`ip` and survive (F3) — test-side subprocess is allowed; the lint scopes
production `src/` only.

---

## 2. ADR-0085 decisions (DDD-1..DDD-14)

| ID | Decision |
|---|---|
| DDD-1 | Eliminate all `ip`/`nft`/`ethtool`/`sysctl` subprocess; CH stays the only sanctioned subprocess. Remove `$PATH` dependency; type the idempotency the packet-corruption path depends on. |
| DDD-2 | New `overdrive-netlink` (`adapter-host`) crate is the shared home — one auditable home for hand-rolled kernel wire bytes; avoids the deliberately-avoided production `overdrive-worker → overdrive-host` edge. |
| DDD-3 | Shared errno `NetlinkError` embedded via `#[source]`. `VethProvisionError` per-site variants swap `stderr:String,status` → `errno:Option<i32>`; `Spawn(#[from] io::Error)` → discrete `NetlinkConnect`. `InterceptError::TproxyInstall{reason:String}` catch-all DECOMPOSED into `IpRuleAddFailed` / `IpRouteLocalAddFailed` / `NftRuleInstallFailed` / `NftHandleRecoveryFailed`. |
| DDD-4 | `setns` helper on a dedicated throwaway `std::thread` (never a pooled tokio worker — `setns` permanently mutates the calling thread's netns). |
| DDD-5 | `provision()` → `async fn` (sole call site already async → `.await`, no `spawn_blocking`). |
| DDD-6 | Port the `ip_rule_fwmark_present` dump-then-add guard; keep `ip route local` `-EEXIST`-idempotent. Naked netlink `rule add` stacks duplicates (no FIB dedup). |
| DDD-7 | Drop `rustables`; hand-roll nft over `NETLINK_NETFILTER` including `tproxy` (`rustables` cannot express `tproxy` and drags `bindgen`/`libclang`). |
| DDD-8 | ethtool `FEATURES_SET` = `0x0c` hand-rolled over genl (the `ethtool` crate is GET-only; `0x0a` is `WOL_SET`, the issue body was wrong). |
| DDD-9 | Locked dep set (rtnetlink 0.23 + netlink-packet-* + genetlink + nix 0.30), `adapter-host`-only; `CAP_NET_ADMIN` unchanged. |
| DDD-10 | Final slice = xtask `ban-infra-subprocess` lint; catalogue sanctioned variable-binary spawns (CH `vmm.rs`, workload drivers `driver.rs`/`exec_prober.rs`, guest PID-1 `overdrive-init`); `// subprocess-ok:` marker. Bounded guarantee: named literals only, not variable-binary spawns. |
| DDD-11 | This is the in-place swap, NOT #197 — no port trait / Sim / DST promotion. `overdrive-netlink` is a future #197 home, not #197 itself. |
| DDD-12 | Surface the `nix` 0.29→0.30 workspace bump as a slice-1 gating task (rtnetlink 0.23 pulls 0.30 transitively; do not silently mix across the setns FD boundary). |
| DDD-13 | Delete each observation text-parser + its tests in the slice that lands the structured replacement. |
| DDD-14 | Before DELIVER, cite the exact production call site for each slice's entry point. Slice 4's `install_inbound_tproxy` is production-wired today (`on_alloc_running` → `worker.start_alloc` → `HostMtlsIntercept::install_inbound`); the Tier-3 guard drives that path, never hand-installs the rule. |

---

## 3. Delivery — five vertical slices, lint last

Each slice was driven end-to-end through a production entry point (`serve` /
`deploy`), guarded by a named LIVE Tier-3 e2e, and kept the pure cores
byte-identical. Sequenced across three DELIVER phases (roadmap 5 steps):

| Step | Slice | Commit |
|---|---|---|
| 01-01 | Crate scaffold + host-netns veth swap + ethtool `FEATURES_SET` encoder + `async provision()` + nix 0.29→0.30 bump | `31c2fa81` |
| 01-02 | Per-alloc netns + veth swap; `in_netns` setns thread; last shared-parser deletion | `5b87449d` |
| 02-01 | mtls `ip rule`/`route local` swap; ported fwmark dump-then-add guard; ip-side `InterceptError` variants | `2622a7aa` |
| 02-02 | mtls `nft` swap (hand-rolled tproxy encoder); structural `NFTA_RULE_HANDLE` recovery; boot sweep; nft-side `InterceptError` decomposition complete | `0a478ddc` |
| 03-01 | xtask `ban-infra-subprocess` lint; flip S-LINT-01..05 self-test scaffolds GREEN | (xtask commit) |

Post-DELIVER hardening: `45f1b45f` (async netns-file open via `tokio::fs` —
`BlockingIoInAsync` dst-lint fix), `ba1d41ed` (L1-L6 refactor: centralize the
netlink sync→async bridge into `runtime.rs` + pre-size `nft.rs` message buffers),
`47c3fa13` (close the mutation gate on the `overdrive-netlink` pure corpus).

---

## 4. Decisions worth recording (the four that shaped the final shape)

1. **`rp_filter` behaviour-lock relaxed `== 0` → `!= 1`.** The per-host-veth
   `rp_filter` assertion was relaxed to match production's OWN
   `sysctl_rp_filter_relaxed` converge contract. The exact-`0` write is reverted
   by the Lima systemd-sysctl netdev-add re-apply, and the old subprocess path
   only passed by winning a ~300 ms race. The lock now asserts the load-bearing
   invariant ("not strict / `!= 1`") rather than a value the environment fights.
   **Re-tighten to `== 0` once the immutable appliance OS exists** (ADR-0068) —
   which it does NOT yet.
2. **Sim `IpRuleAddFailed` fault models a `NetlinkError::Connect` source.** To
   avoid a sim/host-split violation (a `Sim*` fault must not import a host-only
   errno shape), the sim fault synthesises the failure through
   `NetlinkError::Connect` rather than fabricating a raw errno.
3. **`BlockingIoInAsync` dst-lint fix.** The netns-file open inside the async
   provision path used blocking `std::fs`; the dst-lint gate flagged it and it was
   moved to `tokio::fs` (`45f1b45f`). The async fn body never blocks a tokio
   worker.
4. **`NFTA_RULE_HANDLE` structural recovery uses `NFTA_RULE_USERDATA` identity
   tagging.** The by-handle delete / boot sweep identifies Overdrive's own rules by
   an `NFTA_RULE_USERDATA` tag rather than by the old `# handle N` text scrape.
   This is a **single-cut migration** — pre-swap untagged rules are NOT swept
   (consistent with project single-cut/greenfield policy; "delete the on-disk
   state" is the upgrade path).

---

## 5. Quality-gate outcomes (how we know it works)

- **Post-merge integration gate: 567 tests GREEN** on kernel `7.0.0-29-generic`
  (Lima, root, `--features integration-tests`). Every named behaviour-lock —
  veth create/idempotent/half-heal/recreate, ethtool `tx-checksumming: off`
  (the packet-corruption-critical oracle, RUN not skipped), per-alloc netns
  lifecycle, fwmark `ip_rule_fwmark_count == 1` after two installs, real TPROXY
  divert with `getsockname == virt`, §5 boot sweep exactly-one-after-reinstall —
  stayed GREEN across the swap.
- **Adversarial review: APPROVED, 0 blockers.**
- **Mutation: 100% kill rate** on the `overdrive-netlink` pure corpus (the errno
  benign/fatal classifier, the ethtool WANTED-bitset derivation, the tproxy byte
  layout vs the pin, the `NFTA_RULE_HANDLE` recovery predicate). Impure netlink /
  genl / nfnetlink I/O shims excluded per the existing shim-exclusion pattern.
- **Integrity check: exit 0.**
- **`ban-infra-subprocess` lint: S-LINT-01..05 GREEN** — the door-lock proves
  zero named infra-CLI literals remain in the migrated in-scope production tree
  (only the excluded `overdrive-testing/netns.rs` retains any).

---

## 6. Evidence pointers

- **Spike verdict (all five increments WORKS on real kernel `7.0.0-29-generic`):**
  `docs/feature/subprocess-free-veth-provisioner/spike/findings.md`,
  `findings-d.md` (mtls `ip` → rtnetlink), `findings-e.md` (mtls `nft` tproxy →
  hand-rolled netlink), `wave-decisions.md` (straight-to-DESIGN gate +
  lint-as-final-DELIVER-phase).
- **The golden-bytes wire pin** for the hand-rolled tproxy expression (the
  correctness contract for the encoder, per bpf.md Rule 3 — verifier-accept ≠
  correct; only a real-packet divert proves it):
  `spike/findings-e.md` (the kernel-accepted `NFTA_TPROXY_FAMILY/REG_ADDR/REG_PORT`
  + paired `REG_1=127.0.0.1` / `REG_2=agent_port` immediate byte layout, prerouting
  mangle -150).
- **Spike probe sources + raw captures:**
  `spike-scratch/subprocess-free-veth-provisioner/increment-{a..e}/` (committed
  evidence; disposition surfaced to the user separately at finalize).
- **The behaviour-lock Tier-3 tests** that drive `serve` + `deploy` end-to-end:
  `crates/overdrive-control-plane/tests/integration/{veth_provision_idempotent,
  serve_boot_provisions_veth, workload_netns_provision, alloc_netns_lifecycle,
  adopt_on_restart}.rs`;
  `crates/overdrive-worker/tests/integration/{mtls_intercept_install,
  egress_tproxy_capture, bidirectional_walking_skeleton, inbound_tproxy_harness,
  canonical_address_inbound_walking_skeleton, start_alloc_installs_both_tproxy}.rs`;
  the ethtool tx-off oracle guards the BPF incremental-checksum path via
  `crates/overdrive-dataplane/tests/integration/{reverse_nat_e2e,
  reverse_nat_udp_e2e, sanity_mixed_batch}.rs`.
- **Decision record:** `docs/product/architecture/adr-0085-subprocess-free-netlink-mechanism-swap.md`.
- **Feature workspace (preserved):** `docs/feature/subprocess-free-veth-provisioner/`
  (spike/, feature-delta.md, deliver/).

---

## 7. Follow-ups (tracked, not deferred here)

- **#197** — promote `overdrive-netlink` consumers to a first-class network
  port-trait / Sim-adapter / DST reconciler. This feature was the in-place swap,
  NOT #197 (DDD-11); `overdrive-netlink` is the candidate home.
- **`rp_filter` re-tighten to `== 0`** — once the immutable appliance OS (ADR-0068)
  removes the Lima systemd-sysctl re-apply race. No issue filed at finalize; the
  re-tighten is a one-line assertion change gated on the appliance kernel, recorded
  here as the environment premise, not a promise.
