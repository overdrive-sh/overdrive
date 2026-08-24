# ADR-0085 — Subprocess-free netlink mechanism swap for veth provisioning + transparent-mTLS interception

## Status

Accepted. 2026-08-24. Decision-makers: Morgan (solution-architect,
proposing, Propose mode). Tags: phase-2, dataplane, transparent-mtls,
adapter-host, netlink, mechanism-swap, GH-233.

**Scope:** a **mechanism swap only** — replace every `ip` / `nft` /
`ethtool` / `sysctl` subprocess shell-out in
`crates/overdrive-control-plane/src/veth_provisioner.rs` and
`crates/overdrive-worker/src/mtls_intercept.rs` with direct netlink +
`/proc/sys` file I/O. **The precise, enforced invariant** (D8) is: **no
named infra-CLI subprocess** (`ip`/`nft`/`ethtool`/`sysctl`/`tc`/`bpftool`
/`iptables`) remains in scoped production `src/`. Cloud Hypervisor is the
sanctioned *workload* launcher — but note it is **not literally the only
`Command::new` in the production graph**: `overdrive-host/src/vmm.rs`
spawns CH *through* a `prlimit`/`setpriv` confinement wrapper
(`vmm.rs:725`) and a diagnostic `stat` (`vmm.rs:673`), and the drivers
spawn the workload binary. None of those is a named infra-CLI, so the D8
lint neither targets nor is contradicted by them; the invariant this
feature delivers is the narrower, enforceable "no infra-CLI shell-out,"
not the literal "only CH ever spawns." No new user-facing surface; the
pure **derivation/diff** cores stay byte-identical (the observation
*text-parsers* are deleted — D10); the swap is confined to the impure
executor/observer shims. This is the minimal in-place swap and an explicit
**down-payment on — NOT the delivery of — issue
[#197](https://github.com/overdrive-sh/overdrive/issues/197)** (the
continuous network-reconciler port-trait + Sim-adapter + DST promotion).
See § "In-place swap vs #197" (D9).

**Companion ADRs:** ADR-0061 (single-node veth converge-on-boot — the
provisioner this swaps the mechanism of), ADR-0003 (crate-class
taxonomy — governs the new crate's class), ADR-0071 (transparent-mTLS
Path A — the per-alloc netns + nft-TPROXY path in scope), ADR-0068
(pinned appliance kernel). `.claude/rules/bpf.md` Rule 2 (the `tx off`
invariant the ethtool swap must not regress).

## Context

`overdrive serve` and `overdrive deploy` provision host networking by
**shelling out** to iproute2 / nftables / ethtool / procps binaries
(~30 call sites across the two files). Three costs:

1. **A `$PATH` runtime dependency on iproute2/nftables/ethtool/procps.**
   The Yocto immutable-appliance target (ADR-0061, ADR-0068) must not
   depend on the presence, version, or `$PATH` resolution of userland
   network CLIs to boot its dataplane. A missing or renamed `ip` binary
   is a boot failure that a self-contained netlink client cannot suffer.
2. **Stderr-substring idempotency on a packet-corruption-critical path.**
   Idempotency today is brittle locale/version-fragile string matching
   (`stderr.contains("File exists")`, the multi-phrase `link_absent`
   classification, the `# handle N` text scrape). On the `ethtool -K tx
   off` path this is acute: a wrong classification that silently treats a
   genuine failure as benign leaves TX-checksum-offload ON, which
   **silently corrupts every NAT'd packet** (commit `62fa6be2`,
   `.claude/rules/bpf.md` Rule 2). Typed netlink `errno` cannot drift the
   way a phrase can.
3. **The `nft`/`ip` handle recovery is a text scrape** (`# handle N`)
   rather than a structural read of `NFTA_RULE_HANDLE`.

The SPIKE (`docs/feature/subprocess-free-veth-provisioner/spike/`, five
increments, all **WORKS** on kernel `7.0.0-29-generic`) de-risked the
entire swap on a real kernel, including the two hardest paths: the
hand-rolled ethtool `FEATURES_SET` (increment B, proven by a wire
checksum contrast — `[bad udp cksum]` ON → `[udp sum ok]` OFF) and the
hand-rolled nft `tproxy` expression (increment E, proven by a real
connection divert with orig-dst preserved). The user ratified
**DISCARD walking-skeleton → straight to DESIGN**; the spike findings are
the design input. All priorities in #233 are IN scope — the ethtool
"trap" half is hand-rolled here, not deferred.

## Decision

### D1. Eliminate every named infra-CLI subprocess across both files

Replace all `ip` (link/addr/route/rule/netns/setns), `nft`
(table/chain/rule/tproxy/handle), `ethtool` (`-K`/`-k`), and `sysctl`
(`-w`/`-n`) shell-outs with:

- **`rtnetlink` 0.23** for link / addr / route / rule / netns / setns
  over `NETLINK_ROUTE`. `NetworkNamespace::add` forks internally (child
  does the `unshare`+mount so the caller is not moved) but **execs no
  external binary** — it introduces no new subprocess and is named here so
  the fork-no-exec behavior is on the record.
- **Hand-rolled `ETHTOOL_MSG_FEATURES_SET` = 12 (`0x0c`)** over
  `genetlink` / `netlink-packet-generic` on `NETLINK_GENERIC`: enumerate
  the changeable `tx-checksum-*` bits via `FEATURES_GET`, then
  `FEATURES_SET` each off through `ETHTOOL_A_FEATURES_WANTED`. The
  `ethtool` crate is GET-only (`Wanted` emit is `todo!()`), so SET must
  be hand-rolled. (Issue-brief correction: `0x0a` is `WOL_SET`, not
  `FEATURES_SET`.)
- **Hand-rolled nftables** over raw `NETLINK_NETFILTER`, including the
  `tproxy` expression (exact kernel-accepted wire bytes in
  `spike/findings-e.md`). `rustables` is dropped (D-alt below).
- **Direct `/proc/sys/net/**` file I/O** for the per-netns sysctl knobs
  (`ip_forward`, `rp_filter`), written from a `setns`'d context.

### D2. Module placement — a new focused `overdrive-netlink` (`crate_class = "adapter-host"`) crate

Both consumers need a shared netlink client, a shared errno error type, a
shared `setns` helper, and the two hand-rolled wire encoders. A **new
`adapter-host`-class crate `overdrive-netlink`** is the home. It exposes
**plain impl modules** (a client wrapper, two encoders, a setns helper,
`NetlinkError`) — **no port trait, no Sim adapter** (that is #197, D9).

Contents:

- `NetlinkError` — typed, errno-carrying (D3).
- rtnetlink client wrapper: link add/del/show, addr add, route add
  (incl. `Local` kind), rule add + rule-dump presence check, netns
  add/del, link-move-into-netns.
- `setns`-on-a-dedicated-`std::thread` helper (D4).
- ethtool `FEATURES_SET` genl encoder (`disable_tx_offload`).
- nftables nfnetlink encoder: table/chain/rule add, the `tproxy`
  expression, `GETRULE` / `NFTA_RULE_HANDLE` structural recovery,
  by-handle delete.

**Why a new crate and not `overdrive-host` (Alternative B):**
`overdrive-control-plane` already carries a production `[dependencies]`
edge on `overdrive-host`, but **`overdrive-worker` depends on
`overdrive-host` only as a `[dev-dependency]`** — a *deliberate*
port-trait-purity posture (its Cargo.toml documents that production host
bindings are composed at the CLI binary boundary, not named in the worker
crate). Housing the shared module in `overdrive-host` would force a **new
production `overdrive-worker → overdrive-host` edge** and drag
`overdrive-host`'s unrelated heavy surface (the `vmm` Cloud-Hypervisor
launcher, `cgroup_fs`, `vm_host_state`) into the worker's production
compile graph. A focused `overdrive-netlink` crate gives both consumers
**exactly** the netlink surface, concentrates **all** hand-rolled kernel
wire bytes (the highest-risk code in this change) into **one auditable,
single-responsibility home**, and is idiomatic under ADR-0003 (mirrors
the existing small `adapter-host` crates `overdrive-store-local` /
`overdrive-testing`). It is also the natural, isolated future home for
#197's netlink adapter without entangling `vmm`/`cgroup`.

**Why not duplicate a private submodule per crate (Alternative A):** the
shared `NetlinkError` errno mapping, the rtnetlink connect/run helper,
and the `setns` helper would drift across two copies; concentrating all
hand-rolled netlink wire encoding in one place is a load-bearing
auditability property.

### D3. Error model — a shared errno-carrying `NetlinkError`, embedded (not substituted) into the per-site enums

- **`overdrive-netlink::NetlinkError`** (thiserror) is the shared
  low-level error, with variants keyed by netlink op family, each
  carrying a typed `errno: Option<i32>` sourced from
  `rtnetlink`'s `ErrorMessage.code` (and, for the hand-rolled encoders,
  the kernel `NLMSG_ERROR` code) — **never a parsed stderr string**.
- **`VethProvisionError` keeps its per-call-site variants** (`LinkAddFailed`,
  `AddrAddFailed`, `TxOffloadDisableFailed`, `NetnsAddFailed`,
  `SysctlSetFailed`, …) so the operator still gets a **cause-specific**
  message naming the failing provisioning step (ADR-0061;
  `.claude/rules/development.md` § Errors). Each of these variants **swaps
  its `stderr: String, status: Option<i32>` fields → `errno: Option<i32>`**
  and embeds the shared error via `#[source] NetlinkError` (pass-through,
  full chain preserved). The obsolete `Spawn(#[from] std::io::Error)`
  variant (whose Display is "spawning `ip(8)` failed") is **replaced** by a
  discrete `NetlinkConnect { source }` — and the blanket `#[from]
  std::io::Error` is **dropped** so netlink-socket-open and `/proc/sys`
  write `io::Error`s map to *distinct* variants (`SysctlSetFailed` for the
  latter) rather than being silently absorbed.
- **`InterceptError` MUST be decomposed, not left as-is.** Its current
  `TproxyInstall { reason: String }` is a **multi-site `String`
  catch-all** — `ensure_ip_route_local`, the `ip rule` add, and every `nft`
  op all flatten into it — which is exactly the `.claude/rules/development.md`
  § Errors anti-pattern. It is **split into per-site variants**, each
  embedding `#[source] NetlinkError` (errno-carrying):
  `NftRuleInstallFailed { op: &'static str, source: NetlinkError }`,
  `IpRuleAddFailed { source: NetlinkError }`,
  `IpRouteLocalAddFailed { source: NetlinkError }`, and
  `NftHandleRecoveryFailed { context: String }` (the structural
  `NFTA_RULE_HANDLE` dump had no matching rule — a logic/parse failure, not
  an errno). The `TransparentListener` / `Accept` / `OrigDst` / `ChainAbsent`
  variants are **unchanged** — they are socket / `getsockname` / benign-absence
  signals the swap does not touch. **This naming is the contract** — the
  crafter implements these variant names, does not invent them
  (CLAUDE.md "implement to the design; never invent API surface").
- **Idempotent codes are matched on the typed errno and swallowed at the
  executor**, not surfaced: `-EEXIST` (address/route already present, the
  kernel-auto-created on-link route), `-ENODEV` (absent link on observe).
  This replaces every `stderr.contains("File exists")` / `link_absent`
  substring check.
- **No `Internal(String)` flattening and no single `Netlink(NetlinkError)`
  catch-all on the top-level enums** — that would collapse the per-step
  operator context ADR-0061 requires. The per-site variants stay; the
  shared error is embedded, not substituted.

### D4. `setns`-on-a-dedicated-`std::thread` helper

In-netns netlink AND in-netns `/proc/sys` writes require entering the
target netns via `nix::sched::setns(fd, CLONE_NEWNET)`, and netns
membership is **per-thread**. The helper (`overdrive-netlink`) spawns a
**dedicated, throwaway `std::thread`** per invocation, opens the netns fd
(`/var/run/netns/<name>` — the persistent-netns convention
`NetworkNamespace::add` creates), `setns` into it, runs the closure
(which opens an in-netns rtnetlink handle and/or writes `/proc/sys`), and
lets the thread die on completion. It **must NOT** use a `tokio`
worker/`spawn_blocking` pooled thread: `setns` permanently mutates the
calling thread's netns, so a pooled thread returned to the runtime would
be poisoned for reuse. The `async` `provision()` awaits the join
(`spawn_blocking` may bridge the join only). Shape:
`in_netns(&NetnsName, impl FnOnce() -> Result<T, NetlinkError> + Send) ->
Result<T, NetlinkError>`.

### D5. `provision()` becomes `async fn`

The sole call site — `run_server_with_obs_and_driver`
(`overdrive-control-plane/src/lib.rs:2133`, an `async fn`) — becomes
`veth_provisioner::provision(&plan).await`. No `spawn_blocking` at the
call site. The `health.startup.refused` /
`DataplaneBootError::Provision { source }` refuse-to-boot path is
unchanged; only `source` changes from stderr-carrying to errno-carrying.

### D6. Preserve the `ip rule` dump-then-add guard; `ip route local` stays EEXIST-idempotent

Spike increment D proved that a **naked netlink `rule add`
(`NLM_F_EXCL|CREATE`) stacks a duplicate** — netlink does NOT dedup fib
rules, identical to iproute2. The `ip_rule_fwmark_present` dump-then-add
guard is **load-bearing and ported verbatim** to the netlink path (a
`GETRULE` dump + presence check before `NEWRULE`). The
`ip route add local … table 100` path IS `-EEXIST`-idempotent and keeps
its tolerate-EEXIST shape (now via typed errno, not `File exists`).

### D7. Locked dependency set (workspace, `adapter-host` crates only)

`rtnetlink` 0.23, `netlink-packet-route` 0.33, `netlink-packet-core` 0.9,
`netlink-packet-generic` (ethtool genl), `genetlink`, `netlink-sys` 0.9,
`netlink-proto` 0.13, `nix` 0.30. Added to `[workspace.dependencies]`;
consumed only by `overdrive-netlink` (and re-exported types where a
consumer names an errno). `CAP_NET_ADMIN` requirement is **unchanged**
(already a serve-boot precondition for XDP attach + cgroup delegation) —
no new privilege. See § "Open constraint — nix version" for the 0.29→0.30
workspace bump.

### D8. Final DELIVER slice — an xtask "ban infra-CLI subprocess" lint

A structural xtask lint mirroring `xtask/src/dst_lint.rs` (syn AST
visitor + marker-comment suppression) that **bans `Command::new("<tool>")`
for the named infra CLIs** `ip`, `nft`, `ethtool`, `sysctl`, `tc`,
`bpftool`, `iptables` in production `src/**` of runtime crates.

- **Scope:** crates whose `crate_class ∈ {core, adapter-host}`, MINUS an
  explicit exclusion of `overdrive-testing` (a dev-dep-only Tier-3
  fixture crate that legitimately shells `ip netns add` and is never
  linked into a production binary — the same "own only what ships"
  discipline dst-lint uses to scan only `core`). `binary`-class
  (`overdrive-cli`, `xtask`) and `adapter-sim` (`overdrive-sim`) crates
  are out of scope by class. `#[cfg(test)]` items and `bin/` tooling are
  exempt (mirrors dst-lint).
- **What it bans, and the precise guarantee:** the seven **named
  string-literal** args to `Command::new`, NOT `Command::new` generically
  (banning all spawns would forbid running workloads and Cloud
  Hypervisor). The guarantee is therefore bounded: it catches
  `Command::new("ip")`-shaped literals but **not** a variable-binary spawn
  (`Command::new(var)`) or a `run_ip()`/`run_nft()` helper indirection.
  That residual is acceptable and mirrors `dst-lint`'s own literal-scope:
  the structural backstop against indirection is (a) the swap leaves **no**
  infra-CLI helper in either file, and (b) code review. DDD-10 / D8 claim
  "structurally enforce **no named infra-CLI literal**," not "no infra
  subprocess by any indirection."
- **Marker:** `// subprocess-ok: <reason>` on the use-site line or the
  line immediately above (mirrors `// dst-lint: hashmap-ok`). Reserved for
  a future sanctioned infra-CLI use; after this feature there are none in
  production.
- **Sanctioned exceptions the lint MUST NOT flag** (each verified
  in-tree): safe **by construction** because they spawn a *variable*
  binary or a non-listed one, not one of the seven literals — Cloud
  Hypervisor (`overdrive-host/src/vmm.rs`, `Command::new(&wrapper[0])`);
  the workload drivers (`overdrive-worker/src/driver.rs`,
  `Command::new(spec.driver.command())`) and
  `overdrive-worker/src/probe_runner/exec_prober.rs` — spawning the
  WORKLOAD is the product, not an infra shell-out; guest PID-1
  (`overdrive-init/src/main.rs` — spawns init/reboot/kmod via `nix`
  syscalls, invokes none of the seven CLIs). Excluded **by scope**:
  tooling `overdrive-sim/src/bin/dst.rs`, `overdrive-cli`, `xtask`
  (`binary`/`adapter-sim` classes), and the dev-dep fixture
  `overdrive-testing`.
- It flips green immediately once slices 1–4 land (a static grep confirms
  the only production `src/` sites invoking the seven literals are the two
  files in scope plus the excluded `overdrive-testing/netns.rs`). It is
  the **final DELIVER slice of THIS feature** (the "lock the door" step),
  not a follow-up issue.

### D9. In-place swap vs #197 — the scope boundary

This ADR delivers the **minimal in-place mechanism swap**: structured
`errno` + no `$PATH` dependency + structural handle recovery, confined to
the impure shims of two files, with the pure cores unchanged. It ships
independently and adds **no DST coverage** — the netlink client is real
I/O in an `adapter-host` crate, exercised by the existing Tier-3 e2e, not
under `Sim`/turmoil. It is explicitly **NOT** #197's continuous
network-reconciler: no `NetworkProvisioner` port trait, no `Sim` adapter,
no observed-state hydration, no continuous tick. `overdrive-netlink` is a
natural future home for #197's Host adapter, but designing that port is
out of scope here and MUST NOT be pre-built.

### D10. "Byte-identical" applies to the derivation/diff cores ONLY — the observation text-parsers are DELETED with their tests

The pure surfaces split into two classes, and only ONE stays:

- **Derivation / diff cores — stay byte-identical:** `derive_veth_plan`,
  `converge_steps`, `derive_workload_netns_plan`,
  `workload_converge_steps`, `smallest_free_slot`, `NetSlot`,
  `resolv_conf_contents`, and the `NetSlotAllocator`. These key on
  structured facts / plan inputs, not on CLI text; the swap does not touch
  them.
- **Observation *text*-parsers — DELETED with their tests** (CLAUDE.md
  § Deletion discipline: unused production code is deleted WITH the tests
  defending it, in the same slice — not gated, not salvaged). Once the
  observer reads **structured** netlink/genl attributes, the text parsers
  become dead code. Each is replaced by a structured read:

  | Deleted text-parser (file) | Replaced by structured read |
  |---|---|
  | `link_state` `contains(",UP,")`, `link_absent(stderr)` (`veth_provisioner.rs`) | `LinkFlags` (IFF_UP) + presence from the `RTM_GETLINK` reply / `-ENODEV` |
  | `tx_checksumming_on` (`ethtool -k` text) | `ETHTOOL_A_FEATURES_*` bitset from `FEATURES_GET` |
  | `# handle N` scrape — `find_virt_rule_handle`, `output_divert_handle_in_dump`, `find_egress_rule_handle_in_dump`, `dump_has_egress_rule` (`mtls_intercept.rs`) | `NFTA_RULE_HANDLE` from the `GETRULE` reply (structural) |
  | `ip_rule_dump_has_fwmark` / `ip_rule_fwmark_present` text (`mtls_intercept.rs`) | FIB-rule attributes from the `RTM_GETRULE` dump (the ported dump-then-add guard, D6) |
  | `stderr_reports_absent_chain`, `dump_has_leg_s_exemption` (`mtls_intercept.rs`) | `-ENOENT` on `GETCHAIN` / structural exemption-rule presence from the `GETRULE` reply |

  The feature-delta scope table (§ 6) assigns each deletion to the slice
  that lands its structured replacement, so no slice leaves a dead parser
  behind a green suite.

## Alternatives Considered

### Keep the subprocess shell-outs (status quo) — Rejected

Retains the `$PATH` dependency on iproute2/nftables/ethtool/procps
(unacceptable for the Yocto appliance) and the locale/version-fragile
stderr-substring idempotency on the packet-corruption-critical `tx off`
path. The spike proved the swap is fully de-risked on a real kernel, so
"the CLIs are simpler" no longer buys anything the netlink path cannot.

### `rustables` for the nftables path — Rejected

`rustables` 0.8.8 (rustwall fork of Mullvad `nftnl-rs`) has **no typed
`tproxy` expression** (`grep -rli tproxy` over its src is empty) **and no
public raw-expression escape hatch** (`nlmsg` is `pub(crate)`,
`ExpressionRaw`'s field is private), so it structurally cannot express
the load-bearing verb. It also drags a `bindgen` 0.72 + `libclang` build
dependency (generates its `sys` consts from kernel `nf_tables.h`) — a real
production build cost. Spike E proved the hand-rolled `tproxy` netlink
works; the table + chain are strictly simpler than the proven rule.

### C `libnftnl` / `libmnl` FFI — Rejected

Reintroduces a C toolchain + native-lib link dependency (the exact
appliance-portability liability the swap removes), for no capability over
the pure-Rust hand-rolled encoder the spike validated. The whole codebase
is Rust; a C FFI dep for nftables is a step backward.

### Module placement A — private netlink submodule duplicated per crate — Rejected

Two copies of the `NetlinkError` errno mapping, the rtnetlink connect/run
helper, and the `setns` helper drift the moment one side is patched.
Concentrating all hand-rolled kernel wire encoding in one auditable home
is load-bearing for a correctness-critical change.

### Module placement B — shared module in `overdrive-host` — Rejected (close runner-up)

Viable and zero-new-crate for `overdrive-control-plane` (already has the
production edge), but forces a **new production `overdrive-worker →
overdrive-host` `[dependencies]` edge** that the worker crate
deliberately avoids (its `overdrive-host` dep is `[dev-dependencies]`-only
for port-trait purity), and drags `overdrive-host`'s `vmm`
(Cloud-Hypervisor launcher), `cgroup_fs`, and `vm_host_state` surface into
the worker's production compile graph. The focused `overdrive-netlink`
crate (D2) gives both consumers exactly the netlink surface with no
unrelated weight and a cleaner future-#197 boundary. B remains the
fallback if the team vetoes a new crate, at that documented cost.

### Module placement C — shared code in `overdrive-worker` (which `overdrive-control-plane` already depends on) — Rejected

Costs neither a new crate NOR a new worker→host edge —
`overdrive-control-plane` already names `overdrive-worker` in
`[dependencies]` (`Cargo.toml:59`, per ADR-0029), so the control-plane
side reuses an existing edge. **Rejected** because `overdrive-worker` is a
**role-specific subsystem** (drivers, probe-runner, mtls interception),
not an infrastructure-adapter home; loading it with a general host netlink
client that the control-plane's veth provisioner also pulls muddies its
single responsibility and inverts the dependency intuition (a network
provisioner reaching *into* the worker crate for kernel plumbing). It also
makes `overdrive-netlink` unavailable as the clean, subsystem-neutral home
#197's future adapter wants. A focused `adapter-host` crate keeps the
netlink client where any host-adapter consumer can reach it without taking
on the worker's role surface.

## Consequences

### Positive

- **No `$PATH` dependency** on iproute2/nftables/ethtool/procps; the
  appliance dataplane is self-contained (Yocto-ready).
- **Typed `errno` idempotency** on the packet-corruption-critical path —
  no locale/version phrasing can silently reclassify a genuine `tx off`
  failure as benign.
- **Structural `NFTA_RULE_HANDLE` recovery** replaces the `# handle N`
  text scrape.
- **All hand-rolled kernel wire encoding lives in one auditable crate**
  (`overdrive-netlink`), the correct home for the highest-risk code.
- **Down-payment on #197** without committing its design.

### Negative

- **A new workspace crate** (`overdrive-netlink`) + eight netlink
  dependencies enter the workspace graph (`adapter-host`-only).
- **Two hand-rolled wire encoders** (ethtool `FEATURES_SET`, nft
  `tproxy`) are now first-party maintenance surface. Mitigated: both are
  spike-proven with pinned wire bytes, small (~120 lines each), and
  guarded by the existing Tier-3 e2e (a wrong ethtool bitset is caught by
  `reverse_nat_e2e`'s real-packet echo, per `bpf.md` Rule 2/3).
- **A `nix` 0.29 → 0.30 workspace bump** (see Open constraint).
- **The two hand-rolled encoders' Tier-3 guard must run on the pinned-6.18
  matrix, not only the dev kernel.** The spike ran on 7.0.0-29-generic; the
  authoritative merge signal is the pinned 6.18 appliance kernel
  (ADR-0068). The pinned wire bytes (`NFTA_TPROXY_*`,
  `ETHTOOL_MSG_FEATURES_SET = 0x0c`) are UAPI constants stable across
  6.18↔7.0, so risk is low — but the `reverse_nat_e2e` + mtls-divert ATs
  that guard the encoders must gate on 6.18.

### Quality-attribute impact

- **Reliability / correctness:** positive — typed errno removes a silent
  packet-corruption reclassification mode; structural handle recovery
  removes a text-scrape fragility.
- **Portability:** positive — no userland-CLI `$PATH` dependency; still
  Linux-only (netlink), unchanged.
- **Maintainability:** mixed — one focused, auditable netlink home
  (positive) at the cost of two hand-rolled encoders as first-party code
  (negative, mitigated by the spike + Tier-3 guard).
- **Security:** neutral — `CAP_NET_ADMIN` unchanged; no new privilege.

## Open constraint — nix version (surfaced, not silently resolved)

The workspace pins `nix = "0.29"` (features `["net","sched","socket",
"uio"]`; `overdrive-init` additionally uses `["reboot","kmod"]`); the
spike locked `nix 0.30`. `sched::setns` exists in both and `sched` is
already enabled, so setns itself does not force the bump — but `rtnetlink`
0.23 transitively pulls its own `nix`, and mixing our direct 0.29 with
rtnetlink's 0.30 across the FD-passing setns boundary is the risk. The
recommended resolution is a **workspace bump to `nix 0.30`** (re-verifying
`overdrive-init`'s `["reboot","kmod"]` compiles against 0.30) so one `nix`
major spans the direct setns use and rtnetlink's transitive use. This is
a DELIVER-slice-1 gating task, flagged here rather than assumed.

## References

- `docs/feature/subprocess-free-veth-provisioner/spike/findings.md`,
  `findings-d.md`, `findings-e.md`, `wave-decisions.md` — the five-
  increment WORKS verdict + pinned wire bytes.
- `docs/feature/subprocess-free-veth-provisioner/feature-delta.md` — full
  component decomposition, Reuse Analysis, C4, DELIVER slices.
- ADR-0061 (single-node veth converge-on-boot — the mechanism swapped),
  ADR-0071 (transparent-mTLS Path A), ADR-0003 (crate-class taxonomy),
  ADR-0068 (pinned appliance kernel).
- `.claude/rules/bpf.md` Rule 2/3 (the `tx off` invariant + checksum
  byte-order domain), `.claude/rules/development.md` § Errors,
  `.claude/rules/reconcilers.md` (converge-on-boot).
- `xtask/src/dst_lint.rs` — the lint template mirrored by D8.
- GH [#233](https://github.com/overdrive-sh/overdrive/issues/233) (this
  feature), [#197](https://github.com/overdrive-sh/overdrive/issues/197)
  (deferred continuous network-reconciler — NOT this ADR).
