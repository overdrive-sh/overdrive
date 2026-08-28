# DESIGN Decisions — guest-stack-transparent-mtls-intercept (GH #222)

Mode: PROPOSE (options + trade-offs presented; recommendations recorded here
as decisions). Scope: APPLICATION/COMPONENTS. Paradigm: object-oriented Rust.
Full narrative: `../feature-delta.md`. ADRs: 0088 (topology + addressing),
0089 (provisioning boundary + CH net attach).

## Key Decisions

| # | Decision | One-line rationale |
|---|---|---|
| D1 | **Routed two-/30 topology** (tap + guest /30 in the netns, `ip_forward`, host return route); guest /30 carved from `10.99.0.0/16` upper half (`base + 0x8000 + slot*4`) | Byte-for-byte the spike-proven shape (WORKS, no toggles); in-/16 carve keeps every mesh-membership test correct for free. L2-bridge and /32-onlink variants rejected as unproven L2/unnumbered-routing risk. |
| D2a | **`workload_addr` = the guest addr** for VM-kind allocs (injected at C3; rides the EXISTING `AllocStatusRowV2` field — no schema change) | The only address a connection can terminate at; every downstream consumer (persist, resolve, future inbound daddr match, future advertise) is correct with zero change. |
| D2b | **Guest addressing = one platform-owned kernel-cmdline parameter applied silently by `overdrive-init` before READY** (NIC-down check, per-interface IPv6 disable, verified `arp_notify=0`, static IPv4 ioctls + guest resolv.conf), fail-closed before the beacon session reaches READY | Makes READY the deterministic platform-initialization barrier and the pre-intercept packet contract closed: a host that received READY knows guest networking completed without emitting a guest L2 frame and the guest is blocked awaiting EXEC. `CONFIG_IP_PNP` unavailability is moot; DHCP/vsock-message add mechanism for a statically known value. Beacon PL unchanged. |
| D3 | **Return route (+ ip_forward + tap converge) owned by the C3-seam provisioner extension** (`veth_provisioner`, Bar-1 converge-on-boot); teardown structural; Bar-2 rides #197/#234 | Same lifecycle as the veth it rides on; fail-closed via existing `ShimError`; no new reconciler (reconcilers.md valid-intermediate). |
| D4 | **C3 branches on the `DriverPayload` VM arm** → pure `VmTapPlan` + tap converge + spec injection; `VmDriver` composes `VmConfig` (netns now CONSUMED + net attach + cmdline); **`Vmm` prepends `ip netns exec <ns>` to the existing wrapper argv + `--net tap=,mac=`**; tap creation subprocess-free (ioctl create + netlink netns-move, EXTEND `overdrive-netlink`) | Provisioner-creates/driver-enters preserved; `overdrive-host` stays `#![forbid(unsafe_code)]`; spike launch shape verbatim. fd-passing REJECTED ON THE MERITS (ADR-0089 §A2 — the Firecracker-jailer `setns` precedent points AT the wrapper), re-open only with evidence against the wrapper — NOT a queued refinement. |
| D5 | **Inbound (peer→guest): topology settled NOW, build deferred to #257** (existing issue) | Zero change needed in `install_inbound_tproxy` (keys on `workload_addr` = guest addr); leg-S delivery = the spike-proven reply path; a #222 inbound slice has NO serve+deploy driver until #257 removes the `[vm]+[service]` parse rejection — building it would repeat the #236 dead-mechanism precedent. |
| D6 | **Lift the `DriverType::Exec` intercept gate to include VM-kind at BOTH install sites** (`action_shim/mod.rs:1584` fresh-start + `:1880` restart); teardown (`stop_alloc` `:1269`/`:2038`) ungated-by-design (no flip, none must be added); D-MTLS-18 fail-closed inherited | The gate's own comment names #222 as its lifter; without the flip the feature is dead on the production path. Flipping ONLY fresh-start leaves restarted VM allocs CLEARTEXT fail-OPEN (restart budget / crash-recovery / `overdrive workload restart` ADR-0073 — all live for VM) while the fresh-deploy AT goes green — so both gates flip, and a Tier-3 restart AT (kvm-tests / metal) pins the restart site. |

## Architecture Summary

The ONLY new production code is the tap wire: tap-in-netns provisioning
(pure `VmTapPlan` + four idempotent converge steps), the CH netns entry +
`--net tap` attach, guest addressing via cmdline + `overdrive-init`, and the
one-site intercept-gate flip. The host-side intercept
(`install_outbound_tproxy` + `ensure_shared_routing_infra`) and the entire
#26 `MtlsEnforcement` proxy (handshake/kTLS/splice, ADR-0069/0070) are reused
verbatim — the spike proved the production rule fires unchanged over a
tap-fed veth. No new crate, port, daemon, dependency, or observation-schema
change.

## Walking-skeleton egress slice (Slice 1, BLOCKING)

`overdrive serve` (mTLS-composed) + `overdrive deploy` of a `[vm]`+`[job]`
whose guest dials a mesh `[service]` by name: `overdrive-init` first completes
guest platform initialization (including static networking), emits READY, and
blocks awaiting EXEC; guest egress is then captured at the host-veth by the
EXISTING rule → leg-F → `MtlsResolve` `Mesh` → proven mTLS to the peer's
agent → response returns into the guest; a non-mesh dial passes through
(`NonMesh`); an intercept-install failure drives the alloc terminal.
No test-only wiring — every install/bind/address/route is a production call
site.

The AT must additionally assert three obligations this remediation names:
(1) **boot-through-first-connect safety** — a verified tap capture is ready
before VMM spawn; it observes ZERO guest-originated L2 frames until the exact
alloc's host-veth intercept is live, then proves the guest's first mesh dial is
captured with ZERO cleartext SYN escaping (the born-captured ordering invariant
`capture-ready ≺ VMM-spawn ≺ network-ready ≺ READY ≺ install-live ≺
EXEC-release ≺ operator-first-connect`,
Finding 2 / Q9; the closed packet contract is pinned below);
(2) **guest dial-by-name**
resolves over routed hops FIRST — it is topology-reasoned, NOT spike-proven
(the spike used raw-IP connect; Finding 5); (3) a **restart** AT (same
nested-KVM metal surface) — a *restarted* VM alloc re-installs the intercept
(the `:1880` gate) and is driven terminal fail-closed on install failure
(Finding 1). DISTILL authors the scenarios.

**Execution surface (iteration-1 HIGH remediation)**: the Slice-1 Tier-3 AT
boots a REAL microVM → requires nested KVM → gated behind `kvm-tests` (on top
of `integration-tests`) and run via `cargo xtask metal run --` on the x86_64
metal box, NOT Lima (arm64 Lima has no nested KVM; a Lima AT returns no
signal).

## Reuse Analysis (verdict tally)

9 REUSE-AS-IS (`install_outbound_tproxy`, `ensure_shared_routing_infra`,
`start_alloc` + the #26 proxy, `NetSlotAllocator`, `install_inbound_tproxy`
(deferred use), DNS responder, `AllocStatusRowV2.workload_addr`, the
`VmRunDir::console_log`/CH serial-file route, VMM-stderr capture) · 9 EXTEND
(C3 seam including `AllocationSpec` injection, veth provisioner,
`overdrive-netlink`, `VmConfig`+`Vmm`, `VmDriver`, `overdrive-init`, the Exec gate,
`WorkloadLifecycle::classify_natural_exit_terminal`, the Tier-3 observation
decorator) · 1 CREATE-NEW
(the pure `VmTapPlan` value + its converge steps, housed inside EXTENDed
components). Full table with contract shapes: `../feature-delta.md`
§ Reuse Analysis.

## Tech Stack

CH `--net tap=` (v53.0, spike-proven) · `/dev/net/tun` ioctls via `nix` 0.30 ·
netlink netns-move via `overdrive-netlink` · `ip netns exec` wrapper argv
(iproute2, appliance-present) · in-tree `overdrive-init`. No new external
dependency; no proprietary component.

## Constraints

- Vertical slice: Slice 1 MUST run through real `serve`+`deploy` (the #236
  precedent is the counter-example).
- `overdrive-host` stays `#![forbid(unsafe_code)]`; CH remains the only
  sanctioned subprocess (the netns wrapper rides it).
- Spike verdict pinned to kernel 7.0.0-29; 6.18 appliance confirmation =
  Tier-3 matrix at merge (nft_tproxy confirmation user-waived).
- `NET_SLOT_MAX = 4095` keeps both /30 carves inside `10.99.0.0/16`.
  **DELIVER item (Finding 4):** the guest-carve disjointness must be
  COMPILER-PROVEN — DELIVER adds a symmetric guest-carve const guard
  `(0x8000 + NET_SLOT_MAX*4 + 3) < base_span` beside the existing S6 transit
  guard (`veth_provisioner.rs:518`, which asserts only `(NET_SLOT_MAX*4 + 3) <
  base_span`), so a future slot-domain / base raise cannot silently overflow
  the guest carve. A prose "re-check the split" caveat is insufficient (noted
  beside the #239 tunable-base caveat).

## Upstream Changes

- brief.md: new `## Guest-stack transparent-mTLS intercept extension
  (ADR-0088/0089, GH #222)` section + ADR-index rows + changelog row.
- ADR-0069's "STAGED to #222" pointer is now realised by this design (no
  ADR-0069 edit needed — the fold decision is unchanged).
- `VmConfig.netns` docstring's "Job-kind VMs need no tap" becomes stale when
  DELIVER lands the consuming code — the crafter fixes it in the step that
  changes the behavior (behavior-change-marks-stale-docs discipline), not
  here.

## Q7/Q9 lifecycle amendment (2026-08-28; metal counterexample)

Step 02-03 RED produced the counterexample that supersedes the prior Q7
classification: the isolated net-apply-failure scenario could receive `EXIT
78` before the host flushed EXEC and pass, while the combined metal gate let
the host install the intercept and flush EXEC before the guest finished
networking. The same pre-operator failure was then classified as `crashed
(exit Some(78), signal None)`. A successful host write proves only that EXEC
reached the transport; it does not prove that the guest consumed EXEC before
emitting EXIT. The post-READY/pre-EXEC distinction was therefore scheduling-
dependent and is explicitly superseded.

The ratified deterministic lifecycle is:

1. `overdrive-init` bootstraps its minimal root, parses the platform network
   token, applies the static guest network, and writes resolver configuration
   **before opening/reaching READY on the beacon session**.
2. READY means guest platform initialization, including networking, completed;
   the guest is now blocked awaiting the existing EXEC reply.
3. Any init, malformed-token, or net-apply failure happens before READY,
   powers the guest off fail-closed, and is consumed by the driver's existing
   pre-READY `VmmExited` boot-race arm. It never execs the operator command.
4. After READY, an `EXIT` is exclusively the existing post-EXEC operator
   result. The host still withholds EXEC until intercept installation succeeds.

The terminal state is explicit for #222's executable `[vm]+[job]` surface: the
start-rejection write records a Failed attempt with
`VmGuestExitUnreported { vmm_exit_code, vmm_signal }`. The existing Job-kind
natural-exit branch stays in place, but its pure
`classify_natural_exit_terminal` function is **EXTENDED** so every
`VmGuestExitUnreported { vmm_exit_code, .. }` maps exactly to
`TerminalCondition::Failed { exit_code: vmm_exit_code }`. That branch emits no
`RestartAllocation`, so
`WorkloadLifecycleView.restart_counts` and the durable `restart_count` remain
unchanged. Its source-local property generates every `Option<i32>` exit-code
shape (and arbitrary signal), asserts that exact mapping, and carries the exact
rustdoc line `/// CONTRACT_SHAPE: pure-function.`. A reconciler/action-shim
example additionally seeds nonzero private and durable counts, proves the only
action is `FinalizeFailed` (never `RestartAllocation`), proves the returned
View equals the input View, and proves the final row forwards the prior
`restart_count` unchanged.

The diagnostic is not VMM stderr. Cloud Hypervisor routes guest serial to
`VmRunDir::console_log()` while `VmmDiagnostics`/`VmmExit.stderr_tail` contain
the hypervisor process's separate stderr. `VmDriver` therefore **EXTENDS** its
pre-READY `VmmExited` arm: after the VMM exit resolves and before destructive
run-dir cleanup, it asynchronously snapshots the end of `console.log`, bounded
to the last five line fragments and 8 KiB (retaining an unterminated final
fragment, lossy UTF-8), sourcing those bounds from existing
`STDERR_TAIL_LINES` and `VMM_CONSOLE_TAIL_MAX_BYTES`. A non-empty guest-console
tail is the primary `detail`;
the separately bounded hypervisor stderr is fallback only when the console is
absent, empty, or unreadable; if both are absent, a stable bounded diagnostic
names that fact. Snapshot failure never masks cleanup or the typed start
rejection. This reuses the existing serial-file path and public describe
projection; it adds no Beacon field, `VmmExit` field, observation field, or
public enum. `workload describe` renders the final reason, selected detail,
terminal claim, and unchanged restart count. Future `[vm]+[service]` behavior
remains #257's concern and retains the generic Service start-rejection restart
policy unless that issue designs a different policy.

The Beacon Published Language and `BeaconMessage` enum remain byte-for-byte
unchanged; pre-READY setup failure no longer overloads `EXIT`.

Rejected remediation shapes:

- reserving status 78 (or any sentinel) cannot distinguish a normal operator
  that returns the same status and still depends on a racy host phase;
- sleeps, grace periods, or delayed EXEC merely change the interleaving;
- an EXEC acknowledgement or new net-ready field/message would distinguish
  consumption, but versions a protocol that needs no extension once READY is
  restored as the initialization barrier.

Security ordering is now
`capture-ready ≺ VMM-spawn ≺ network-ready ≺ READY ≺ intercept-live ≺
EXEC-release ≺ operator-first-connect`. The closed pre-intercept packet
contract is **zero guest-originated L2 frames**, not an open-ended "control
traffic" allowlist.
This is feasible for the static setup path only if `overdrive-init` verifies
the NIC is down, disables IPv6 for that interface, sets and reads back IPv4
`arp_notify=0`, and only then applies the static IPv4 address/route and raises
the NIC. Those preparations fail closed before READY. There is no DHCP, DNS,
probe, neighbor warm-up, socket connect, or workload send. Linux otherwise
enables IPv6 by default (link-local DAD/router-solicitation risk), while
`arp_notify=0` is the kernel's defined no-gratuitous-ARP mode; the design pins
both instead of assuming appliance defaults.

The metal witness is exact and fail-conservative:

1. after C3 provisions the allocation's netns/tap/host-veth, but before
   `Vmm::create` can spawn Cloud Hypervisor, the metal-only observation barrier
   binds an all-EtherType packet capture to the exact `VmTapPlan.tap` ifindex
   inside that netns and a correlated witness to the exact
   `WorkloadNetnsPlan.host_veth` ifindex; the VMM start is released only after
   both report capture-ready. This observer changes no production networking
   or address/install call site;
2. identity is cross-checked by allocation id, slot, netns inode, tap name and
   ifindex, host-veth name and ifindex, guest MAC, and guest address. A frame
   with an unexpected source MAC is a failure, not traffic silently assigned
   to someone else;
3. from capture-ready through intercept-live, **every guest-to-host Ethernet
   frame is forbidden**, including VLAN-tagged or unknown EtherTypes, ARP,
   IPv4, IPv6, ICMP, TCP, UDP, multicast, broadcast, zero-payload, and
   payload-bearing frames, to every destination. There is no control-frame
   exception. A truncated/malformed record, unknown direction or timestamp,
   capture drop/overflow, missing readiness edge, or uncertain interface
   correlation fails the proof rather than being ignored;
4. intercept-live means `start_alloc` returned success **and** the expected
   outbound rule is observed against that same host-veth; a log line or host
   function entry is insufficient. Capture continues across EXEC release. The
   first guest TCP SYN after release must be the operator's expected
   `guest_addr -> mesh service VIP:port` flow; the same five-tuple/original
   destination must increment that host-veth rule and arrive at leg-F, while
   no cleartext copy reaches the external peer path and the inter-agent path
   carries TLS records. Unknown ordering invalidates the test.

## Open questions → DISTILL

Q1 guest-net channel field shape · Q2 `VmConfig` net-attach struct shape
(fold recommended) · Q3 cmdline parameter grammar · Q4 MAC byte layout ·
Q6 guest-subnet const name/home · Q8 the sanctioned `KernelCmdline`
compose/append surface (review medium —
named in the design so the crafter is not inventing it) · Q9 the exact
EXEC-release wiring realising the born-captured invariant `install-success ≺
EXEC-release` (where the release gate sits relative to the `Running` boundary +
the READY-vs-EXEC "what is Running for a VM" reconciliation; iteration-2
Finding 2 — model pinned, wiring open). (Q5 tap name PINNED:
`ovd-tp-<4hex-slot>`.)

Q7 is no longer open: the pre-READY driver-start outcome above replaces the
racy EXIT-before-EXEC arm. Q9's deferred EXEC mechanism remains, but its full
ordering now begins with network-ready-before-READY. The existing DISTILL and
roadmap Q7 state machine is downstream-stale and requires a fresh DISTILL /
roadmap remediation before DELIVER resumes; this DESIGN agent does not edit
those artifacts.

## Deferrals (existing issues only — none created)

Inbound build + `[vm]+[service]` enablement → **#257** · Bar-2 network
reconciler → **#197/#234** · intended-peer pinning → **#242** (concept
inherited from ADR-0069; ADR-0069's own #178 citation is stale, flagged for
separate cleanup).

## Gaps for the orchestrator

- Outcome Collision Check NOT run (CLI unavailable in this dispatch).
