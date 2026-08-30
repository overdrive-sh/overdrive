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
| D2b | **Guest addressing = one platform-owned kernel-cmdline parameter applied silently by `overdrive-init` before READY** (NIC-down check, per-interface IPv6 disable/read-back, verified `arp_notify=0` write/read-back, static IPv4 ioctls + guest resolv.conf), fail-closed before the beacon session reaches READY | Makes READY the deterministic platform-initialization barrier and the pre-intercept packet contract closed: a host that received READY knows guest networking completed without emitting a guest L2 frame and the guest is blocked awaiting EXEC. `CONFIG_IP_PNP` unavailability is moot; DHCP/vsock-message add mechanism for a statically known value. Beacon PL unchanged. |
| D3 | **Return route (+ ip_forward + tap converge) owned by the C3-seam provisioner extension** (`veth_provisioner`, Bar-1 converge-on-boot); teardown structural; Bar-2 rides #197/#234 | Same lifecycle as the veth it rides on; fail-closed via existing `ShimError`; no new reconciler (reconcilers.md valid-intermediate). |
| D4 | **C3 branches on the `DriverPayload` VM arm** → pure `VmTapPlan` + tap converge + spec injection; `VmDriver` composes `VmConfig` (netns now CONSUMED + net attach + cmdline); **`Vmm` prepends `ip netns exec <ns>` to the existing wrapper argv + `--net tap=,mac=`**; tap creation subprocess-free (ioctl create + netlink netns-move, EXTEND `overdrive-netlink`) | Provisioner-creates/driver-enters preserved; `overdrive-host` stays `#![forbid(unsafe_code)]`; spike launch shape verbatim. fd-passing REJECTED ON THE MERITS (ADR-0089 §A2 — the Firecracker-jailer `setns` precedent points AT the wrapper), re-open only with evidence against the wrapper — NOT a queued refinement. |
| D5 | **Inbound (peer→guest): topology settled NOW, build deferred to #257** (existing issue) | Zero change needed in `install_inbound_tproxy` (keys on `workload_addr` = guest addr); leg-S delivery = the spike-proven reply path; a #222 inbound slice has NO serve+deploy driver until #257 removes the `[vm]+[service]` parse rejection — building it would repeat the #236 dead-mechanism precedent. |
| D6 | **Lift the `DriverType::Exec` intercept gate to include VM-kind at BOTH install sites** (`action_shim/mod.rs:1584` fresh-start + `:1880` restart); teardown (`stop_alloc` `:1269`/`:2038`) ungated-by-design (no flip, none must be added); D-MTLS-18 fail-closed inherited | Initial deploy and generation replacement use the fresh-start gate. The reachable same-`AllocationId` VM Job route is unclean control-plane restart → boot-epoch Platform Reclamation while intent stands → `RestartAllocation`; without the restart-site flip that re-drive is cleartext fail-open. Natural Job exit/crash finalizes run-once without retry, and `overdrive workload restart` creates a fresh allocation. S-GTI-06a/06b pin the same-id success/failure outcomes. |
| D7 | **EXTEND the existing alloc-scoped outbound nft rule with one anonymous counter and expose a strict, bounded, generation-bracketed internal `GETRULE` projection plus an exact kernel-domain counter oracle** | Q9 needs one unchanged production rule across both cuts. Full-program identity rejects same-handle replacements; `GETGEN` plus a loss-detecting nft notification guard rejects ruleset mutation; complete multipart parsing rejects partial dumps; and exact packet/`skb->len` equality rejects in-place counter reset. The counter remains non-terminal after the existing interface/TCP matches, preserving match, redirect, mark, verdict, userdata, ownership, and teardown. |
| R1-R7 | **Recover 02-05/02-06 onto ObservationStore terminal truth, awaited allocation-local cleanup, private mTLS task ownership, the existing VM reclamation route, and the accepted post-READY intercept gate** | Removes the unreviewed outbox, persistent route/event protocol, hidden Pending cleanup token, generic task API, retry-owner transfer, live-survivor reconstruction/quarantine, and pre-start intercept while retaining the minimum single-process reclamation lease and typed async stop/shutdown surfaces. |

## Architecture Summary

The ONLY new production code is the tap wire, its bounded lifecycle
handoff: tap-in-netns provisioning (pure `VmTapPlan` + four idempotent
converge steps), the CH netns entry + `--net tap` attach, silent pre-READY
guest addressing via cmdline + `overdrive-init`, the **two-site** intercept
gate flip (fresh start + restart), bounded pre-cleanup selection of the
existing guest console with VMM-stderr fallback, and the exact
`VmGuestExitUnreported` Job terminal-classifier extension with no restart,
plus the D7 alloc-rule counter encoder, strict single-reply generation reader,
complete multipart rule dump, and full-program decoder.
`install_outbound_tproxy` is EXTENDED only to place that
non-terminal counter after its unchanged interface/TCP matches and before its
unchanged TPROXY/mark/accept tail; the read-only metal observer adds the
loss-detecting nft change guard and exact capture packet/`skb->len` equality.
`ensure_shared_routing_infra` and the entire #26 `MtlsEnforcement` proxy
(handshake/kTLS/splice, ADR-0069/0070) are reused verbatim. No new crate, port,
daemon, dependency, external API, Published Language, persistence, or
observation-schema change.

## Walking-skeleton egress slice (Slice 1, BLOCKING)

`overdrive serve` (mTLS-composed) + `overdrive deploy` of a `[vm]`+`[job]`
whose guest dials a mesh `[service]` by name: `overdrive-init` first completes
guest platform initialization (including static networking), emits READY, and
blocks awaiting EXEC; guest egress is then captured at the host-veth by the
existing rule semantics, whose exact tag+handle+normalized-program counter is
generation-guarded and advances by exactly the capture-derived nft packet and
`skb->len` totals, → leg-F → `MtlsResolve` `Mesh` → proven mTLS to the peer's
agent → response returns into the guest; a non-mesh dial passes through
(`NonMesh`); an intercept-install failure drives the alloc terminal.
No test-only wiring — every install/bind/address/route is a production call
site.

The AT must additionally assert three obligations this remediation names:
(1) **boot-through-first-connect safety** — a verified tap capture is ready
before VMM spawn; it observes ZERO guest-originated L2 frames until the exact
alloc's host-veth rule has a coherent counter baseline, then proves the guest's
first mesh dial is the only eligible tuple and produces checked, exact packet
and kernel-`skb->len` deltas on one generation-guarded full program, with ZERO
cleartext SYN escaping (the born-
captured ordering invariant
`capture-ready ≺ VMM-spawn ≺ network-ready ≺ READY ≺ intercept-live ≺
EXEC-release ≺ operator-first-connect`,
Finding 2 / Q9; the closed packet contract is pinned below);
(2) **guest dial-by-name**
resolves over routed hops FIRST — it is topology-reasoned, NOT spike-proven
(the spike used raw-IP connect; Finding 5); (3) a **same-allocation re-drive**
AT on the same native metal surface — unclean control-plane restart with
standing intent causes boot-epoch Platform Reclamation and reuses the
`AllocationId`; that VM Job re-installs through the `:1880` gate, and install
failure remains terminal/fail-closed (Finding 1; S-GTI-06a/06b). DISTILL
authors the scenarios.

**Execution surface (iteration-1 HIGH remediation)**: the Slice-1 Tier-3 AT
boots a REAL microVM through hardware-backed `/dev/kvm` on the native,
non-virtualized x86_64 bare-metal host. `kvm-tests` is only the Cargo feature
name (on top of `integration-tests`); it does not authorize nesting. Run it
through `cargo xtask metal run --` after a fail-closed architecture/KVM API/
virtualization preflight. Lima and every virtualized or nested host are
compile-only/non-signal. The target remains user-supplied through
`OVERDRIVE_METAL_TARGET` or the gitignored workspace `.env`, never a hostname
embedded in DESIGN.

## Reuse Analysis (verdict tally)

8 REUSE-AS-IS (`ensure_shared_routing_infra`, `start_alloc` + the #26 proxy,
`NetSlotAllocator`, `install_inbound_tproxy`
(deferred use), DNS responder, `AllocStatusRowV2.workload_addr`, the
`VmRunDir::console_log`/CH serial-file route, VMM-stderr capture) · 10 EXTEND
(`install_outbound_tproxy`, C3 seam including `AllocationSpec` injection, veth provisioner,
`overdrive-netlink`, `VmConfig`+`Vmm`, `VmDriver`, `overdrive-init`, the Exec gate,
`WorkloadLifecycle::classify_natural_exit_terminal`, the Tier-3 observation
decorator) · 1 CREATE-NEW
(the pure `VmTapPlan` value + its converge steps, housed inside EXTENDed
components). Full table with contract shapes: `../feature-delta.md`
§ Reuse Analysis.

## Tech Stack

CH `--net tap=` (v53.0, spike-proven) · `/dev/net/tun` ioctls via `nix` 0.30 ·
netlink netns-move plus nft anonymous-counter encode, strict single-reply
`GETGEN` / multipart `GETRULE` decode, and nft change-notification guard via
`overdrive-netlink` · `ip netns exec` wrapper argv (iproute2,
appliance-present) · in-tree `overdrive-init`. No new external dependency; no
proprietary component.

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
the NIC is down, disables IPv6 for that interface and reads it back, writes
and reads back IPv4 `arp_notify=0`, and only then applies the static IPv4
address/route and raises the NIC. Those preparations fail closed before READY.
There is no DHCP, DNS, probe, neighbor warm-up, socket connect, or workload
send. Linux otherwise
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
4. intercept-live means `start_alloc` returned success **and** the observer
   bound the unique full userdata+handle outbound rule to that host-veth,
   proved its normalized complete expression program is exactly the program
   emitted by the production encoder, and established a coherent pre-EXEC
   counter baseline inside one unchanged ruleset generation. A log line,
   function entry, handle-only dump, or partial multipart reply is
   insufficient. Capture continues across EXEC release. The first guest TCP
   SYN after release must be the operator's expected `guest_addr -> mesh
   service VIP:port` flow; it must be the only rule-eligible tuple and the
   counter deltas must equal, not merely fit beneath, the complete capture's
   matching-packet count and those IPv4 packets' exact nft `skb->len` total
   before the original destination arrives at leg-F. No cleartext copy reaches
   the external peer path and the inter-agent path carries TLS records.
   Unknown ordering, identity, capture equivalence, or concurrent mutation
   invalidates the test.

## D6 route and Tier-3 substrate correction (2026-08-29; DISTILL re-review)

This amendment supersedes active and historical #222 wording that called
restart budget, natural crash recovery, or `overdrive workload restart` a
same-allocation VM Job route. The actual route is narrower:

1. after an unclean control-plane restart, the boot-epoch `VmReclamation`
   drive sees the standing non-terminal VM allocation intent with no live
   supervision claim and authors a Platform Reclamation ending;
2. `WorkloadLifecycle` excludes that ending from Job natural-exit finalization
   and emits `RestartAllocation` with the same `AllocationId`;
3. that action reaches the restart `Running` install site, which must include
   `DriverType::Vm` and preserve D-MTLS-18 fail-closed behavior.

A normal VM Job result or crash is run-once: it takes the Job natural-exit
branch, finalizes, and consumes no restart budget. `overdrive workload restart`
advances the desired generation, ends the old instance, mints a fresh
`AllocationId`, and reaches the fresh-start install site. Both production
install gates therefore remain required, but S-GTI-06a/06b specifically prove
the boot-reclamation same-id route rather than either invalid substitute.

The same re-review corrects “nested KVM” terminology. Authoritative runtime
evidence comes only from the native, non-virtualized x86_64 metal host using
its hardware-backed `/dev/kvm`; `kvm-tests` is only the feature name. The
canonical command remains `cargo xtask metal run --`, with a user-provided
`OVERDRIVE_METAL_TARGET`/gitignored `.env` target and a fail-closed preflight
that rejects missing or unusable KVM, non-x86_64, failed virtualization
detection, and every virtualized/nested host. Lima is compile-only/non-signal.

## Q9 exact-rule-hit amendment (2026-08-29; DISTILL platform P3)

The shipped outbound rule had no counter, and `RuleInfo`/`list_rules` exposed
only handle+userdata. Therefore the prior exact-increment wording and
REUSE-AS-IS verdict contradicted one another. D7 closes the defect without a
test-owned production mutation:

- the sole production owner, `install_outbound_tproxy`, remains responsible
  for install/adopt/delete-by-handle and adds exactly one anonymous,
  non-terminal `counter` to the egress rule. Expression order is unchanged
  `iifname` match → unchanged TCP match → **counter** → byte-identical
  TPROXY/mark/accept tail. The table, chain, rule order, match set, redirect,
  mark, verdict, and `userdata_egress(host_veth, leg_f_port)` identity remain
  fixed; shared, inbound, and output-divert rules remain counter-free.
- internal `RuleInfo` grows `counter: Option<RuleCounterSnapshot>` with exact
  `RuleCounterSnapshot { packets: u64, bytes: u64 }` and an internal normalized
  full-expression-program identity. The identity preserves every expression's
  order, kind, register, operand, address, port, mark, and verdict while
  replacing only the counter's live packet/byte values with a typed counter
  placeholder. The selected rule must equal the normalization of
  `egress_tproxy_rule_exprs(host_veth, AGENT_LOOPBACK, leg_f_port,
  TPROXY_FWMARK)`; userdata+handle alone is never identity. Counter-free
  siblings remain valid and project `None`; a selected target without exactly
  one complete big-endian-u64 counter fails.
- `list_rules` becomes one strict, bounded multipart `GETRULE` operation on a
  dedicated socket and absolute deadline. Every datagram must come from the
  kernel, every message must carry the request sequence, and every data reply
  must have the expected `NFT_MSG_NEWRULE` type and nft family; every
  `nlmsg_len`, `NLA` length, nested boundary, and alignment must consume a
  complete record.
  The dump accepts exactly one `NLMSG_DONE` with zero completion status and
  rejects nonzero `NLMSG_ERROR`, `NLM_F_DUMP_INTR`, overrun, malformed/trailing
  data, timeout, EOF, or a partial reply, and evaluates target uniqueness only
  after that complete success. A bounded `GETGEN` read separately requires
  exactly one complete kernel `NFT_MSG_NEWGEN` reply with the request sequence,
  expected family, and full nonzero `NFTA_GEN_ID`; any extra, error, overrun,
  malformed, trailing, partial, timeout, or EOF result fails.
- after `start_alloc` returns and before the first `GETGEN`, the read-only
  observer subscribes to the network namespace's `NFNLGRP_NFTABLES` change
  stream with loss reporting enabled. The completed production install thus
  precedes the guarded witness epoch. An
  `ENOBUFS`/overrun or any nft ruleset notification is failure. Every stable
  before/after rule read is bracketed `GETGEN(G) -> complete GETRULE dump ->
  GETGEN(G)` using the full `NFTA_GEN_ID`, and every bracket plus the final
  guard drain must equal the initial nonzero `G`; a changed, decreased, or
  wrapped generation, same-handle replacement, delete/reinsert, unrelated
  concurrent transaction, notification loss, or ambiguous mutation fails.
  The conservative global guard may reject an unrelated nft change but may
  never turn it into a pass.
- while the guest is blocked, two equal generation-bracketed target snapshots
  separated by a capture-confirmed quiet interval define `before`; the second
  completes intercept-live. Two equal guarded snapshots after the round trip
  define `after`. The exact-host-veth read-only `AF_PACKET/SOCK_DGRAM` witness
  is armed before VMM spawn, retains `sockaddr_ll` direction/ifindex/protocol,
  and uses `recvmsg(MSG_TRUNC)` with a 65,535-byte L3 buffer so an oversize or
  truncated record is detected rather than counted; `PACKET_STATISTICS` must
  report zero drops when the window closes. It records one full L3 copy per
  ingress skb without the Ethernet header. For this IPv4 table, a
  counter-eligible record is precisely a kernel-valid,
  unfragmented IPv4 packet on that ifindex whose protocol is TCP; any
  fragmentation, malformed/truncated header, capture truncation, or inability
  to reproduce the kernel eligibility decision fails. Its byte contribution
  is the validated IPv4 `tot_len`, exactly the `skb->len` seen by the priority
  -150 prerouting rule after IPv4 validation/trim; generic L2 frame length,
  snap length, and TCP payload length are forbidden substitutes.
- use checked addition and subtraction with no modular arithmetic. If `C` is
  the complete eligible-packet count and `L` is the checked sum of those exact
  IPv4 `tot_len` values between the quiet cuts, require `C > 0`, `L > 0`,
  `after.packets.checked_sub(before.packets) == Some(C)`,
  `after.bytes.checked_sub(before.bytes) == Some(L)`,
  `before.packets.checked_add(C) == Some(after.packets)`, and
  `before.bytes.checked_add(L) == Some(after.bytes)`. The first eligible packet
  is the expected initial SYN and every eligible packet has that directional
  five-tuple. Exact equality makes an in-window
  `NFT_MSG_GETRULE_RESET` fail: after any counted packet it loses a prefix;
  before the first increment it changes no observed state. Regression, reset,
  wrap/overflow, capture loss, offload/fragment ambiguity, or competing traffic
  therefore cannot false-pass.
- same-tag adoption keeps accumulated counts and establishes a new baseline;
  normal stop deletes exactly that handle; boot recovery sweeps old per-workload
  rules before reinstall, so no comparison crosses restart/replacement. Sibling
  allocation rules/counters are excluded by exact tag+handle+program and remain
  untouched; counter-free siblings still return `None`, and a quiescent sibling
  snapshot stays equal across target teardown. The observer remains read-only:
  it never installs, replaces, resets, or deletes.

The official nftables contract classifies counter as a stateful packet+byte
statement and non-terminal statements as passive for rule evaluation; its
position after both matches is therefore load-bearing. See the
[nftables man page](https://netfilter.org/projects/nftables/manpage.html#COUNTER-STATEMENT).
The reset, generation, dump-consistency, and byte-domain pins follow the kernel
[`NFT_MSG_GETRULE_RESET` UAPI](https://github.com/torvalds/linux/blob/master/include/uapi/linux/netfilter/nf_tables.h),
[`nf_tables_commit`/rule-dump implementation](https://github.com/torvalds/linux/blob/master/net/netfilter/nf_tables_api.c),
[`nft_counter_eval`](https://github.com/torvalds/linux/blob/master/net/netfilter/nft_counter.c),
[`ip_rcv_core` validation/trim](https://github.com/torvalds/linux/blob/master/net/ipv4/ip_input.c),
and [`packet_rcv`](https://github.com/torvalds/linux/blob/master/net/packet/af_packet.c).

## 02-05/02-06 architecture recovery (2026-08-30)

The complete contract is `../feature-delta.md` § "DESIGN recovery amendment".
This summary is authoritative for implementation routing and supersedes
DELIVER review prescriptions that accreted a persistence, ownership, survivor,
or quarantine architecture.

### Fixed decisions

| ID | Decision |
|---|---|
| F1 | `ObservationStore` terminal rows are the sole durable lifecycle truth; no `terminal-effects/`, outbox, receipt, route record, `LifecycleEventPort`, replay, or `effect_key`. |
| F2 | One node and one control-plane process own the data directory; no shared-dir multi-process protocol. |
| F3 | `capture-ready -> VMM-spawn -> network-ready -> READY -> intercept-live -> awaited EXEC-release`; never pre-start interception. |
| F4 | Await effects through the existing operation. An existing sync operation becomes async when required; do not add a sibling method, runtime lookup, or detached future. |
| F5 | Roadmap lists are guidance, its acceptance criteria remain binding, and this recovery does not edit the roadmap. |

### Designer decisions

| ID | Decision |
|---|---|
| R1 | `VmDriver::start` owns and attempts all failed-start cleanup before return. Preserve the primary typed start cause when cleanup succeeds; on cleanup failure use existing `StartRejected/Unclassified(Vm)` plus bounded ordered detail. Existing VM reclamation owns any residue; no `pending_cleanup` or special Pending row. |
| R2 | Retain `allocation_attempt_transition` and the ObservationStore terminal claim as the same-attempt lifecycle fence. An identical duplicate finalization is a zero-effect no-op; Platform Reclamation carries `terminal: None` and is the only same-id reopen. |
| R3 | Remove public `CompletionFence`/`OwnedTaskSet`. Keep a private per-allocation mTLS task owner and private cancellation-safe stop completion only. `stop_alloc` stays async/fallible and is awaited; failed handles may be retried only by a later stop for that allocation. |
| R4 | Retain `Driver::try_begin_reclamation(&AllocationId) -> bool` only as a process-local atomic claim over the existing VM supervision map for both kill-capable reclamation executors. It is not persisted and is unrelated to cleanup retry. |
| R5 | Server/worker owner shutdown remains async/fallible but one-shot. It joins every child before returning. `ServerShutdownError` carries diagnostics only; remove the retained worker and `retry()`. |
| R6 | Replacement boot kills/reclaims the old VMM, commits Platform Reclamation, runs ordinary netns/rule cleanup, then lets normal lifecycle re-drive the same id. Remove live-survivor joins and every recovery-quarantine surface. |
| R7 | Restore the fresh/restart post-READY gate: `Driver::start -> Running row -> start_alloc -> D7 baseline -> awaited release_for_exit_emission`. On install failure, EXEC stays closed while driver/mTLS/network cleanup is attempted and Failed is recorded. |

### Public/cross-crate contract

- Remove `LifecycleEventPort`, `IdempotentLifecycleEventPort`,
  `TerminalEffectJournalError`, `DriverCleanupFailure`, `DriverCleanupStage`,
  `DriverStartCleanupError`, `Driver::retry_start_cleanup_disposition`,
  `Driver::on_alloc_terminal_idempotent`, the public core task module, and all
  `RecoveryQuarantine*` APIs.
- Restore `AllocDriverIndex` to the process-local
  `Mutex<BTreeMap<AllocationId, DriverType>>` alias and action-shim event
  parameters to `&broadcast::Sender<LifecycleEvent>`.
- Retain `VmStartFailure::AllocationAlreadyOwned`, `Driver::live_allocations`,
  `Driver::release_supervision`, and the narrowed reclamation claim.
- Retain these exact awaited signatures:

```rust
impl MtlsInterceptWorker {
    pub async fn stop_alloc(
        self: &Arc<Self>,
        alloc_id: &AllocationId,
    ) -> Result<(), MtlsInterceptStopError>;

    pub async fn shutdown_owner(
        self: &Arc<Self>,
    ) -> Result<(), MtlsInterceptOwnerShutdownError>;
}

impl ServerHandle {
    pub async fn shutdown(
        self,
        drain_deadline: Duration,
    ) -> Result<(), ServerShutdownError>;
}

impl ServeHandle {
    pub async fn shutdown(self) -> Result<(), CliError>;
}
```

Retain `MtlsInterceptInstallError::OwnerShutdown` and `PriorTeardown` only for
the sealed worker and failed prior same-allocation stop states. The shutdown
errors cannot transport or recreate a worker owner. The existing test-gated
`ServerHandle::abort_for_test`, `ServeHandle::abort_for_test`, and
`AbruptServerResidue` remain only as the unclean-restart seam: they await
revocation of control-plane infrastructure tasks, author no lifecycle row, and
invoke no workload terminal/stop path. No other public method, type, enum
variant, field, parameter, or persistence record is sanctioned.

### Mechanical fallout and implementation details

Mechanical compiler fallout may touch any tightly related production, test,
configuration, or constructor file: remove obsolete exports/imports and
journal/quarantine/pre-start fixtures; update trait impls, direct broadcast
arguments, shutdown callers, and struct literals. Preserve D7, Q7 diagnostics,
duplicate-start, pure terminal-fence, same-id re-drive, exact stop/sibling, and
awaited EXEC evidence. Delete tests whose subject is a removed protocol and
reshape task tests around the private allocation owner. No mutation testing or
mutation-exclusion edit belongs in this recovery.

Private helper names/layout, log wording, and Tokio primitive selection remain
implementation details. They cannot weaken the atomic register/stop boundary,
cancellation-safe stop completion, join-before-final-handle-drain order, public
signatures, lifecycle ordering, or single-source-of-truth decisions above.

DELIVER resumes at **02-05 RED with a fresh isolated crafter and fresh review**.
After 02-05 approval, a fresh 02-06 crafter/reviewer pair reconstructs D6 and
exact stop. The previous 02-05 approval and 02-06 review iterations are
historical evidence, not approval of the recovered architecture. `408f5feb` is
a comparison boundary only; recovery is surgical on the current branch.

## Closed handoff to DELIVER

Q1-Q9 were closed by the existing DISTILL/roadmap. The recovery amendment adds
no open API or architecture choice: R1-R7 pin the cleanup, task, lease,
shutdown, boot, and ordering shapes required to execute 02-05/02-06. The
existing approved roadmap remains the downstream acceptance target and is not
regenerated by this design pass.

## Deferrals (existing issues only — none created)

Inbound build + `[vm]+[service]` enablement → **#257** · Bar-2 network
reconciler → **#197/#234** · intended-peer pinning → **#242** (concept
inherited from ADR-0069; ADR-0069's own #178 citation is stale, flagged for
separate cleanup).

## Gaps for the orchestrator

- Outcome Collision Check reports 0 collisions. The registry does not yet
  contain the delta's `OUT-GTI-VMTAPPLAN` or `OUT-GTI-BORNCAPTURED` references;
  this recovery does not create or edit outcome records.
