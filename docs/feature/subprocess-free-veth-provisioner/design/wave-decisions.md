# DESIGN Wave Decisions — subprocess-free-veth-provisioner (GH #233)

**Wave:** DESIGN (Application/components, Propose mode). **Author:**
Morgan. **Date:** 2026-08-24. **Record:** ADR-0085; feature-delta.md.

## Scope

Mechanism swap only — replace every `ip`/`nft`/`ethtool`/`sysctl`
subprocess in `veth_provisioner.rs` + `mtls_intercept.rs` with netlink +
`/proc/sys`. Cloud Hypervisor stays the only sanctioned subprocess. Pure
cores byte-identical; swap confined to impure shims. All #233 priorities
(P1 + P2 ethtool trap) IN. NOT #197.

## Resolved open points

1. **Module placement** — NEW `overdrive-netlink` crate
   (`crate_class="adapter-host"`, impl-only, no port trait). Rejected:
   duplicated-submodule (encoder drift), `overdrive-host` (forces the
   deliberately-avoided production worker→host edge; drags vmm/cgroup).
2. **Error model** — shared errno `NetlinkError` embedded `#[source]` into
   the existing per-site `VethProvisionError`/`InterceptError` variants
   (swap `stderr`→`errno`); idempotent `-EEXIST`/`-ENODEV` matched on
   typed errno. No `Internal(String)`, no catch-all.
3. **`setns` helper** — `overdrive-netlink::in_netns(&NetnsName, closure)`
   on a dedicated throwaway `std::thread` (never a pooled tokio thread).
4. **DELIVER slices** — 5 vertical slices, lint last (see feature-delta
   § 6): (1) crate + host-netns veth; (2) per-alloc netns; (3) mtls `ip`;
   (4) mtls `nft`; (5) xtask ban-infra-subprocess lint.

## Locked (from spike — honored)

rtnetlink 0.23 (link/addr/route/rule/netns/setns; `NetworkNamespace::add`
forks-no-exec); hand-rolled ethtool `FEATURES_SET`=0x0c over genl;
hand-rolled nft over `NETLINK_NETFILTER` incl. `tproxy` expr (drop
`rustables`); ported `ip rule` dump-then-add guard; `ip route local`
EEXIST-idempotent; setns on dedicated `std::thread`; `provision()`→async;
deps `adapter-host`-only; `CAP_NET_ADMIN` unchanged.

## Blockers / contradictions surfaced

- **GH #233 not fetched** — Bash/`gh` unavailable this session; scope
  reconstructed from spike findings + wave-decisions (which capture #233
  fully). Verify against the live issue before DELIVER.
- **`nix` 0.29 vs 0.30** — workspace pins 0.29 (`overdrive-init` uses
  `reboot`/`kmod`); spike locked 0.30; rtnetlink pulls 0.30 transitively.
  Recommend a workspace bump to 0.30 as a slice-1 gating task (re-verify
  `overdrive-init` compiles). Surfaced, not silently resolved.

## Peer review (solution-architect-reviewer, iteration 1)

`conditionally_approved` — 0 critical, 3 high, 4 medium, 3 low. New-crate
choice confirmed NOT resume-driven (worker→host dev-dep asymmetry verified
real). All 3 HIGH + the load-bearing MEDIUM/LOW resolved in ADR-0085 +
feature-delta:

- **HIGH-1** — "only CH sanctioned subprocess" overclaim (`vmm.rs` also
  spawns `prlimit`/`setpriv`/`stat`). Fixed: ADR Scope/D1/D8/DDD-10 now
  state the precise enforced invariant — no *named infra-CLI* literal, not
  "no subprocess except CH."
- **HIGH-2** — `InterceptError::TproxyInstall{reason:String}` is a
  multi-site catch-all; "swap stderr→errno" was untrue for it. Fixed: D3
  now names the decomposition (`NftRuleInstallFailed`/`IpRuleAddFailed`/
  `IpRouteLocalAddFailed`/`NftHandleRecoveryFailed`) so the crafter
  implements-to-design; `VethProvisionError::Spawn` → `NetlinkConnect`.
- **HIGH-3** — "keep every dump parser byte-identical" contradicts the
  replace-text-with-structured-reads decision. Fixed: new ADR **D10** —
  only derivation/diff cores stay byte-identical; the observation
  text-parsers are DELETED with their tests (parser→structured-read map +
  slice assignment). feature-delta §1/§3/§6 + DDD-13 reconciled.
- **MEDIUM** — module-placement **C** (host in `overdrive-worker`) added to
  ADR alternatives with rejection; slice-2/4 production call-site gate
  added (DDD-14, feature-delta §6); lint literal-only guarantee stated
  precisely (D8).
- **LOW** — pinned-6.18 Tier-3 guard note added (ADR Consequences).

## Deferrals

None created. #197 (continuous network-reconciler) is a pre-existing
tracked forward pointer, explicitly out of scope; `overdrive-netlink` is
its future home but is not #197.
