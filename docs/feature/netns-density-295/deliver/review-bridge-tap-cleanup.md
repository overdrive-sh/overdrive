# Independent review: unused bridge and TAP cleanup

## Scope and verdict

**APPROVED.** This review covers the one-off cleanup requested after the two
current website pages described reusable bridge and TAP library APIs as still
present. The review checked the current source tree, public and test callers,
Cargo dependency edges, the worker audit ledger, the two website edits, and
the applicable repository writing and deletion rules.

Reviewed on 2026-11-01 at `HEAD a6a099933` on
`marcus-sa/flow-quota-240s-cap`.

The worker's source disposition is correct. The former reusable bridge and TAP
production APIs are already absent from the current tree. The requested source
cleanup is therefore a no-op. The two website edits accurately remove the
stale current-state claims.

The pre-existing dirty `AGENTS.md` was preserved. The worker's edits to
`website/content/docs/concepts/guest-networking.mdx` and
`website/content/docs/concepts/architecture.mdx` were preserved. This review
created only this Markdown artifact. No production, test, dependency, DES,
commit, push, deployment, kernel, VM, Lima, Metal, BPF, cgroup, nftables, or
network state was changed by the review.

## Exact scope

| Item | Review result |
|---|---|
| `.context/unused-bridge-tap-cleanup.md` | Used as the worker ledger and independently checked against the current tree. The file remains ignored and was not staged or force-added. |
| `website/content/docs/concepts/guest-networking.mdx` | Removes the stale sentence that reusable bridge and TAP library APIs remain. The surrounding current-state limits remain intact. |
| `website/content/docs/concepts/architecture.mdx` | Removes the stale sentence that reusable TAP and bridge APIs remain in the host library. The VM and process attachment distinction remains intact. |
| Historical ADRs, feature artifacts, evidence, blog journey, and source comments | Preserved. Historical mentions were not treated as current API claims and were not edited. |
| Production and test source | Audited read-only. No unused legacy bridge or TAP API remains to delete. |

The review applies the repository's `AGENTS.md`, `.claude/rules/development.md`
Deletion discipline, `website/CLAUDE.md`, `.claude/rules/finalize.md`, and the
repository-local `.claude/skills/technical-writer/SKILL.md`. The website prose
edits follow the mandatory technical-writer pass. The authored review prose
also follows its no-semicolon rule.

## Production and caller inventory

The inventory distinguishes the removed reusable API family from live network
effects and test fixtures.

| Surface | Current definition and caller evidence | Disposition |
|---|---|---|
| Former reusable bridge and TAP API family | The exact search for `TapLinkState`, `ObservedLinkKind`, `ObservedLinkIdentity`, `PersistentTapIdentity`, `create_persistent_tap`, `set_persistent_tap_owner`, `ensure_bridge`, `set_link_down`, `set_link_mac`, `set_link_master`, `observe_persistent_tap`, `observe_link_identity`, `debug_msg_mask`, `debug_msg_masks`, `BridgeGuard`, `TcxAttachPoint`, `GuestNetworkScratch`, `VmTapPlan`, `derive_vm_tap_plan`, `ObservedVmTap`, and `VmTapStep` returned no matches in `crates`, `tests`, or `examples`. | Already deleted. There is no production definition, re-export, caller, or exclusive test to remove. |
| `overdrive-netlink::Client` veth and route operations | `crates/overdrive-netlink/src/client.rs:107-707` contains link, veth, address, route, FIB rule, local-route, and netns operations. `HostDataplaneComposer::compose` calls `veth_provisioner::provision` at `crates/overdrive-control-plane/src/node_adapters.rs:473-498`. The provisioner awaits the client operations at `crates/overdrive-control-plane/src/veth_provisioner.rs:1507-1513` and executes veth, address, link, offload, and route steps at `:1625-1668`. | Live process and single-node veth path. Retain. |
| Process workload netns and TPROXY | `crates/overdrive-control-plane/src/veth_provisioner.rs:1919-1939` owns workload netns provisioning and teardown. The worker's TPROXY path ensures the shared fwmark rule and local route through `Client` at `crates/overdrive-worker/src/mtls_intercept.rs:976-1023`. The shared nft program is observed and converged through the worker port at `crates/overdrive-worker/src/mtls_intercept_port.rs:973-1075`. | Live process network and mTLS interception path. Retain. |
| Private nft bridge-family guard | `NftFamily::Bridge` is a private enum at `crates/overdrive-netlink/src/nft.rs:97-110`. `shared_ip::collect_once` checks the bridge family only to reject a same-named table in the wrong family at `:3302-3310`. The observer is reached by `observe_shared_ip_intercept_state` at `:2844-2848` and the worker's shared-program audit. | Fail-closed identity safety check. It does not create, manage, or expose a bridge API. Retain. |
| Scratch TAP fixture | `crates/overdrive-host/src/vmm.rs:1405-1417` places `launch_seccomp_kernel` under `#[cfg(all(test, feature = "integration-tests"))]`. Its private `ScratchTap` and raw `/dev/net/tun` setup are at `:1456-1517`. The fixture creates a TAP to exercise the production launch seccomp filter and uses `Client::del_link` only for fixture cleanup. | Live test-only confinement fixture. It proves the TAP-mutating ioctl deny list and is not a production TAP API. Retain. |

The remaining source hits for `tap` and `bridge` are bounded to these live
paths, sync-to-async netlink bridge terminology, domain-level bridge names,
or historical comments. A broad term search does not establish unused
network APIs.

## Dependency inventory

`overdrive-netlink` remains an active host adapter. Its `Cargo.toml` declares
`rtnetlink`, `ethtool`, `libc`, `nix`, `tokio`, `futures`, and `thiserror`
for the current veth, route, ethtool, netns, nft, and runtime paths. It has no
TAP or Linux bridge library dependency that can be removed for this cleanup.

`overdrive-control-plane/Cargo.toml:84-91` uses `overdrive-netlink` for the
host-netns veth provisioner. `overdrive-worker/Cargo.toml:48-54` uses it for
the TPROXY policy-route and nft path. `overdrive-host/Cargo.toml:134-137`
declares it as a development dependency for the launch-seccomp scratch TAP
fixture. Those edges have live callers and remain necessary.

The attachment-cut commit `7105b5807` is an ancestor of the reviewed HEAD.
Its source diff records 4,048 deletions across the guest-network, veth,
netlink, ethtool, and nft implementation files. It also removes the old
netlink integration fixtures. The current-tree audit independently confirms
that this deletion is complete for the named reusable API family.

## Website accuracy

The current diff is limited to the two requested pages. It removes four prose
lines and adds two lines. No link target, heading, navigation entry, code
example, or historical blog file changed.

The resulting pages state the current boundary in several places:

- `guest-networking.mdx:9` says VM attachment creates no per-VM host network
  namespace, veth, TAP, or bridge.
- `architecture.mdx:210-211` states the same VM boundary.
- `transparent-mtls.mdx:26` and `microvms.mdx:66` preserve the same current
  behavior in related pages.
- The process workload pages continue to describe network namespaces and
  TPROXY capture.

The exact stale-claim search over `website/content` returns no matches for
`Reusable bridge`, `Reusable TAP`, `bridge APIs remain`, `TAP library APIs
remain`, or `Reusable TAP and bridge APIs`. Other `bridge` references describe
the `BackendDiscoveryBridge`, `WorkflowLifecycle`, or an ordinary conceptual
composition. The engineering journey in
`website/content/blog/networking-without-a-host-interface-per-vm.mdx` remains
untouched, so its historical bridge and TAP evidence is preserved.

The deletion in `guest-networking.mdx` leaves the hardware-offload paragraph
grammatically complete. The deletion in `architecture.mdx` leaves the
process-versus-VM attachment paragraph complete. Neither edit weakens a
current behavior or typed contract.

## Validation evidence

| Check | Result |
|---|---|
| Exact legacy API search across `crates`, `tests`, and `examples` | PASS. No definitions, re-exports, callers, or exclusive tests matched. |
| Current production caller and dependency audit | PASS. Remaining veth, netns, TPROXY, nft, and scratch-TAP edges are live and correctly classified. |
| `git diff --check -- website/content/docs/concepts/guest-networking.mdx website/content/docs/concepts/architecture.mdx` | PASS, exit 0. |
| `bun run typecheck` from `website` | PASS, exit 0. Fumadocs MDX generation and TypeScript checking completed. |
| `bun run lint` from `website` | PASS, exit 0. |
| Exact stale current-website claim search | PASS. The search exits 1 because no stale claim remains. |
| Local MDX links | PASS by unchanged-link inspection. The diff deletes prose only and changes no Markdown link target or heading anchor. The website typecheck also compiles both pages. |
| Worktree preservation after checks | PASS. Only pre-existing `AGENTS.md` and the two worker-edited pages were dirty before this review artifact. No generated website file was added or changed. |

Rust, native-kernel, model, mutation, KASAN, Lima, Metal, cgroup, module,
BPF, nftables, network, VM, and production binary checks were not needed for
this documentation-only result. The source audit used read-only commands.

## Findings and dispositions

No finding requires remediation.

The deletion discipline requires production code and its defending tests to be
removed together when production code becomes unused. That condition is not
present in the reviewed tree because the attachment cut already removed the
legacy production surface and its exclusive integration fixtures. The live
scratch TAP fixture is a separate test of the production seccomp boundary and
has a current caller and purpose.

No dependency removal, test rewrite, source cleanup, historical-document edit,
or architecture change is warranted. Expanding this task to remove the active
netlink client, veth provisioner, TPROXY path, private foreign-family safety
check, or scratch TAP confinement fixture would weaken existing behavior.

## Final verdict

**APPROVED — unused bridge and TAP cleanup.** The current source contains no
unused legacy reusable bridge or TAP production API from the stale website
claims. The two current website claims were removed accurately, the historical
journey remains intact, and the targeted website checks pass. The root agent
may commit the approved worker docs and this review artifact within the user's
existing scope.
