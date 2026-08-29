# ADR-0088: Guest-stack netns topology — routed two-/30 tap wire + guest addressing

## Status

**Accepted** (2026-08-27), **amended** (2026-08-28) to restore READY as the
post-network-initialization barrier after the step 02-03 metal counterexample,
and **amended** (2026-08-29) to make the Q9 exact-rule hit kernel-observable
and mutation-aware.
Extends ADR-0071 (Path A per-workload netns +
nft-TPROXY both directions) to VM-kind (guest-stack) workloads; realises the
guest-stack intercept adapter ADR-0069 STAGED to GH #222. Companion:
ADR-0089 (the provisioning boundary + CH net attach). Spike evidence:
`docs/feature/guest-stack-transparent-mtls-intercept/spike/findings.md`
(verdict WORKS; kernel 7.0.0-29; CH v53.0; no empirical toggle required —
neither `rp_filter=0` nor `ethtool tx off`).

## Context

A microVM terminates TCP in the GUEST kernel. The host sees only virtio-net
frames at a tap — no host `struct sock` — so `cgroup_connect4`/sockops are
structurally blind. The prior-art recon and the increment-n spike proved the
shipped Path-A interception is origin-agnostic: frames a CH virtio-net backend
writes into a tap inside the workload netns route `tap → netns forward → veth →
host-veth ingress`, where the production `install_outbound_tproxy` match and
TPROXY/mark/accept semantics fire unchanged. The 2026-08-29 amendment adds a
non-terminal anonymous counter between those matches and actions plus strict
read-side generation/program/counter evidence; it does not add another rule
owner or mutate the witness target. What the spike deliberately left open as design
decisions: the netns topology, where `workload_addr` sits, and the
guest-addressing mechanism (`CONFIG_IP_PNP` was unset on the probe kernel, so
kernel `ip=` autoconfig was unavailable and the probe guest self-configured
via ioctls).

Constraints honored: the veth half of the provision (`WorkloadNetnsPlan`,
slot-keyed /30s from `WORKLOAD_SUBNET_BASE = 10.99.0.0/16`, `NET_SLOT_MAX =
4095`) is shipped and untouched; §35a's canonical-`workload_addr` discipline
(persisted as an observed input on `AllocStatusRowV2`; the inbound rule and
the bridge advertise key on it); the `ServiceMapHydrator` GATE partition arm
keys mesh membership on `addr ∈ WORKLOAD_SUBNET_BASE`.

## Decision

### 1. Topology: routed two-/30 (the spike shape, verbatim)

The per-workload netns gains a persistent tap addressed from a second
slot-derived /30; the guest's virtio-net NIC is its only interface;
`net.ipv4.ip_forward=1` inside the netns routes tap↔veth; the host holds a
per-alloc return route to the guest /30. The existing veth /30 becomes the
**transit /30** — a pure routed hop carrying no workload endpoint for VM
allocs.

### 2. Subnet carve: the guest /30 lives in the SAME /16, upper half

Guest network = `WORKLOAD_SUBNET_BASE.network() + 0x8000 + slot*4` — a
/18-sized carve starting at the upper-half boundary (`10.99.128.0`),
slot-keyed, mirroring the transit carve's /18-within-the-lower-half shape
(`veth_provisioner.rs:405`). Tap gateway = first usable; guest addr =
second usable. With `NET_SLOT_MAX = 4095` the transit carve tops out at offset
16383 and the guest carve at 49151 — disjoint by construction, both inside the
/16. Mesh-membership tests that key on the /16 (the §35a GATE arm; any future
consumer) remain correct with zero change when #257 lands.

**DELIVER item — the disjointness is compiler-proven, not prose.** The existing
S6 compile-time guard (`veth_provisioner.rs:518`) asserts only the transit
carve: `(NET_SLOT_MAX*4 + 3) < base_span`. DELIVER MUST add the **symmetric
guest-carve const guard** `(0x8000 + NET_SLOT_MAX*4 + 3) < base_span` (49151 <
65536) beside it, so the guest carve's top /30 broadcast is proven to tile
strictly inside `WORKLOAD_SUBNET_BASE` AND strictly above the transit carve at
compile time. Without it, a future `NET_SLOT_MAX` raise (or a #239 base
narrowing) could silently overflow the guest carve past the /16 or into the
transit carve — the exact class the S6 guard exists to make unrepresentable,
left half-covered. This is a DELIVER obligation, not a "re-check the split"
caveat.

### 3. `workload_addr` = the guest addr for VM-kind allocs

The C3 seam injects `spec.workload_addr = guest_addr` (not
`plan.workload_addr`). Every downstream consumer — the `AllocStatusRowV2`
persist (existing field, no envelope bump), `MtlsResolve` `Mesh`
classification, the future bridge advertise and `install_inbound_tproxy`
`daddr` match (#257) — is correct with zero change, because the guest addr is
the only address at which a connection can actually terminate. Exec-kind
allocs are byte-identical to today.

### 4. Guest addressing: kernel-cmdline parameter, `overdrive-init`-applied

`VmDriver` appends ONE platform-owned cmdline parameter carrying
`(guest_addr/30, gateway = tap_gw, dns = responder_addr)`. `overdrive-init`
(the platform-owned guest PID 1, universal for VM-kind via the beacon
contract) parses and applies it via the spike-proven ioctl path
(`SIOCSIFADDR`/`SIOCSIFNETMASK`/`SIOCSIFFLAGS`/`SIOCADDRT`) and writes the
guest's `/etc/resolv.conf` to the node-local DNS responder — dial-by-name
(ADR-0072) from guests with zero app config.

**Fail-closed ordering contract (amended 2026-08-28):** `overdrive-init`
bootstraps the minimal guest root, verifies the non-loopback NIC is down,
disables IPv6 for that interface and reads it back, writes and reads back IPv4
`arp_notify=0`, parses the platform token, applies the static IPv4 network,
and writes resolver configuration **before opening/reaching READY on the
beacon session**. Every
precondition is fail-closed. READY means guest platform initialization,
including silent static networking, completed and the guest is blocked
awaiting EXEC. Any init, malformed-token, suppression, or net-apply failure
powers the guest off before READY and never execs the operator command. The
host consumes that shutdown through `VmDriver`'s already-existing pre-READY
`VmmExited` boot-race arm. The Beacon Published Language is byte-for-byte
unchanged; `EXIT` keeps its original post-operator-wait meaning. Exact
parameter grammar and MAC byte layout remain DISTILL/DELIVER shapes; the
lifecycle ordering is pinned here.

**Metal counterexample superseding the prior observability pin:** step 02-03
RED proved that `EXIT`-before-host-EXEC-flush was not a protocol state. The
isolated failure could win that race, but in the combined metal gate the host
installed the intercept and flushed EXEC while the guest was still applying
networking; the same pre-operator `EXIT 78` was classified as `crashed (exit
Some(78), signal None)`. Host write success does not acknowledge guest
consumption. Status reservation and timing delays therefore cannot distinguish
a guest-setup failure from a normal operator non-zero exit. An acknowledgement
or new field/message could, but would version the beacon protocol unnecessarily
once READY is restored as the initialization barrier.

**Deterministic host classification and operator surface:** a pre-READY guest
shutdown is an existing driver start rejection,
`VmGuestExitUnreported { vmm_exit_code, vmm_signal }`. The action shim records
the attempt as Failed without a Running transition. For #222's executable
`[vm]+[job]` surface, `WorkloadLifecycle` keeps its existing Job-first branch
but **EXTENDS** pure `classify_natural_exit_terminal`: every
`VmGuestExitUnreported { vmm_exit_code, .. }` maps exactly to
`TerminalCondition::Failed { exit_code: vmm_exit_code }`. Its source-local
property generates every `Option<i32>` exit code (with arbitrary signal) and
has the exact declaration `/// CONTRACT_SHAPE: pure-function.`. A reconciler/
action-shim example seeds private and durable counts, proves the only action is
`FinalizeFailed` (never `RestartAllocation`), returned View equals input View,
and the final row's `restart_count` equals the prior row's count.

The concrete guest error does **not** come from `VmmExit.stderr_tail`: CH sends
guest serial to `VmRunDir::console_log()`, while `VmmDiagnostics` captures the
hypervisor process's separate stderr. `VmDriver` owns the pre-READY boot race
and cleanup, so after `VmmExited` resolves and before it removes the run
directory it asynchronously reads only the final 8 KiB / five line fragments
of `console.log` (final unterminated fragment retained; lossy UTF-8), reusing
`VMM_CONSOLE_TAIL_MAX_BYTES` and `STDERR_TAIL_LINES`. A nonempty
guest-console snapshot is primary `detail`. The separately bounded VMM stderr
is fallback only for absent, empty, or unreadable console; if both are absent,
a stable bounded diagnostic says so. Snapshot failure never masks cleanup or
the typed rejection. `overdrive workload describe` already renders reason,
selected detail, terminal claim, and unchanged count. No new describe,
`VmmExit`, `ExitKind`, `TransitionReason`, status-sentinel, or Beacon field is
required. The future `[vm]+[service]` surface remains deferred to #257 and
keeps the generic Service start-rejection policy unless that issue changes it.

**Closed pre-intercept packet contract: zero guest-originated L2 frames.** The
static /30 requires no autonomous control exchange. Linux enables IPv6 by
default and can initiate link-local DAD/router solicitation at link-up, so PID
1 disables it before NIC-up; Linux defines IPv4 `arp_notify=0` as no gratuitous
ARP on device/MAC change, so PID 1 pins and verifies it rather than assuming a
rootfs default. See the kernel's [IPv6 module
policy](https://docs.kernel.org/networking/ipv6.html) and [`arp_notify`
definition](https://docs.kernel.org/networking/ip-sysctl.html). There is no
DHCP, DNS lookup, probe, neighbor warm-up, socket connect, or workload send.
A closed control-frame allowlist is rejected: no such frame is required, and
an allowance creates a hiding place for unexpected destinations or payload-
bearing TCP/UDP.

The metal witness starts after C3 has provisioned the allocation netns, tap,
and host-veth, but before `Vmm::create` delegates to real CH. An observation-
only decorator binds an all-EtherType capture to the exact tap ifindex inside
the exact netns and a correlated witness to the root host-veth ifindex, then
acknowledges capture-ready before allowing spawn/NIC-up. Identity is the full
tuple `(alloc id, slot, netns inode, tap name+ifindex, host-veth name+ifindex,
guest MAC, guest address)`; an unexpected source MAC fails rather than escaping
correlation. From capture-ready through intercept-live, **any** guest-to-host
Ethernet frame fails, whether tagged/untagged, known/unknown EtherType,
ARP/IPv4/IPv6, ICMP/TCP/UDP/other, unicast/multicast/broadcast, any destination,
and with or without payload. Truncation/malformation, capture drop/overflow,
unknown direction/timestamp, missing readiness, or ambiguous interface identity
also fails; none is filtered into a pass.

Intercept-live means `start_alloc` returned success and a coherent pre-EXEC
counter baseline is read from the exact tag+handle+normalized-program outbound
rule on the correlated host-veth, with strict complete multipart replies, one
unchanged full ruleset generation, and no nft notification or notification
loss. Captures and that change guard remain active across EXEC release. The
first post-release guest TCP SYN must be the operator's expected
`guest_addr -> mesh service VIP:port` five-tuple; that must remain the only
rule-eligible tuple, and checked packet/byte deltas must equal the complete
capture's matching-packet count and validated IPv4 `tot_len` (the nft
`skb->len` domain). Any reset, replacement/delete/reinsert, generation
change/wrap, partial/interrupted dump, loss, or ambiguity fails before leg-F
recovers the same original destination, while no cleartext copy reaches the
external peer path and the inter-agent path carries TLS records. Thus the
complete order is `capture-ready ≺ VMM-spawn ≺
network-ready ≺ READY ≺ intercept-live ≺ EXEC-release ≺
operator-first-connect`.

### 5. Exact outbound-rule hit: anonymous nft counter, read-only witness

The alloc-scoped egress rule is EXTENDED from
`iifname + TCP match → TPROXY/mark/accept` to
`iifname + TCP match → counter → TPROXY/mark/accept`. The counter is anonymous,
local to that rule, and non-terminal; rule table/chain/order, match set,
redirect, mark, verdict, and `userdata_egress(host_veth, leg_f_port)` bytes are
unchanged. Shared exemptions and inbound/output-divert rules are not modified.
`install_outbound_tproxy` remains the only install/adopt/delete owner.

The existing internal `GETRULE` surface gains
`RuleInfo.counter: Option<RuleCounterSnapshot>` with packet+byte `u64` values
and an internal normalized full-expression-program identity. Normalization
preserves every ordered expression, register, operand, address, port, mark, and
verdict, replacing only the live counter values with a typed placeholder. The
target must equal the normalized production
`egress_tproxy_rule_exprs(host_veth, AGENT_LOOPBACK, leg_f_port,
TPROXY_FWMARK)` output; tag+handle alone is not identity. Counter-free siblings
remain valid `None`; missing/duplicate/partial/wrong-width/malformed target
counters or any unknown/extra/reordered target expression fail. These are not
public, Beacon, persisted, observation, or describe schemas.

`list_rules` is a strict bounded multipart operation on a dedicated socket and
absolute deadline. It validates kernel sender, request sequence, expected nft
message type/family, every netlink/attribute length, nesting, and aligned
extent, and exactly one
`NLMSG_DONE` with zero completion status; it rejects nonzero `NLMSG_ERROR`,
`NLM_F_DUMP_INTR`,
overrun, timeout/EOF, missing/duplicate DONE, malformed/trailing/partial data,
or wrong sender/sequence before evaluating target uniqueness. Strict bounded
`GETGEN` separately requires exactly one complete kernel `NFT_MSG_NEWGEN`
reply with the request sequence and expected family, and decodes the full
nonzero `NFTA_GEN_ID`; any extra, error, overrun, malformed, trailing, partial,
timeout, or EOF result fails. After `start_alloc` returns and
before the first generation read, the read-only observer joins
`NFNLGRP_NFTABLES` with loss reporting enabled; the completed production
install precedes the guarded witness epoch.
Every ruleset notification, malformed notification, `ENOBUFS`, or overrun is
failure. Each stable snapshot is bracketed
`GETGEN(G) -> complete GETRULE -> GETGEN(G)`, every bracket and final guard
drain must retain the initial `G`, and any change, decrease/wrap, replacement,
delete/reinsert, notification loss, or ambiguous concurrent mutation fails.
The global guard may conservatively reject an unrelated nft transaction.

Two equal generation-bracketed snapshots across a capture-confirmed quiet
interval define `before`; two equal guarded snapshots after the flow define
`after`. A read-only exact-host-veth `AF_PACKET/SOCK_DGRAM` capture, armed
before VMM spawn, retains `sockaddr_ll` direction/ifindex/protocol, uses
`recvmsg(MSG_TRUNC)` with a 65,535-byte L3 buffer, and requires closing
`PACKET_STATISTICS` to report zero drops. It provides one full L3 record per
ingress skb. For this IPv4 rule, eligible means a kernel-valid unfragmented
IPv4 packet on that ifindex with protocol TCP; a
fragment, malformed/truncated record, offload ambiguity, or inability to
reproduce kernel eligibility fails. Each contributes one packet and its
validated IPv4 `tot_len`, exactly the `skb->len` at the priority -150
prerouting counter after IPv4 validation/trim; L2, snap, and payload lengths
are not substitutes. Let `C` and `L` be the complete checked packet and byte
totals between the quiet cuts. Checked addition and subtraction with no wrap
must establish `C > 0`, `L > 0`,
`after.packets.checked_sub(before.packets) == Some(C)`,
`after.bytes.checked_sub(before.bytes) == Some(L)`,
`before.packets.checked_add(C) == Some(after.packets)`, and
`before.bytes.checked_add(L) == Some(after.bytes)`; every eligible packet has
the expected directional five-tuple and the first is its initial SYN. Exact equality makes
`NFT_MSG_GETRULE_RESET` after any increment lose a prefix and fail; a reset
before the first increment changes no observed state. Regression, reset,
counter wrap/overflow, competing traffic, or capture loss cannot false-pass.

The observer never installs, replaces, resets, or deletes. Same-tag adoption
keeps accumulated counts but establishes a new baseline; normal stop deletes
the exact handle; boot recovery sweeps then reinstalls, so comparisons never
cross a replacement/restart. Sibling rules are excluded by exact
tag+handle+program, counter-free siblings still return `None`, and a quiescent
sibling stays unchanged.

## Alternatives Considered

### A1. L2 bridge tap↔veth inside the netns

Enslave tap + veth end to an in-netns bridge; the guest sits directly on the
transit /30. One /30, no forwarding, `workload_addr` identity unchanged.
**Rejected**: unproven (the spike proved routed); pulls in the bridge module,
MAC learning, and the br_netfilter sysctl family (bridged frames traversing
iptables hooks is a known surprise surface); moves the address off the veth
device, breaking the shipped `ObservedWorkloadVeth` converge observation; ARP
spans the pair. Trades a proven mechanism for untested L2 subtlety to save one
/30.

### A2. Guest directly on the transit /30 (routed /32 + onlink, unnumbered tap)

Guest takes `plan.workload_addr` as /32 with an onlink default; the tap runs
unnumbered with proxy-ARP-shaped delivery. **Rejected**: the fiddliest routing
of the three (onlink routes, in-netns /32 host routes, proxy-arp on the tap),
entirely unproven, hardest to observe/converge.

### A3. Guest /30 from a second /16 (e.g. `10.100.0.0/16`)

**Rejected**: guest addrs would silently escape every `∈ 10.99.0.0/16`
mesh-membership test when #257 lands, and a second base constant widens the
#239 tunable-base surface. The in-/16 carve gets disjointness for free from
the slot domain.

### A4. Kernel `ip=` autoconfig (CONFIG_IP_PNP)

**Rejected**: unset on the probe kernel; even with a platform guest-kernel
flip it is boot-static, offers no failure surface (a misapplied config is
silent), and couples addressing to kernel build config.

### A5. DHCP on the tap

**Rejected**: a new host-side daemon plus a guest client, for a value the
platform already knows at plan time.

### A6. A new beacon vsock message carrying net config

**Rejected**: versions the beacon Published Language for a static value;
vsock needs no IP, so nothing forces the config onto that channel; the
cmdline is available at PID-1 start with no ordering dance.

### A7. Allow autonomous ARP/IPv6 "control traffic" before interception

**Rejected:** the static IPv4 /30 does not need DHCP, Router Solicitation,
Duplicate Address Detection, gratuitous ARP, or neighbor warm-up. A frame
allowlist would have to parse VLAN nesting, extension headers, destinations,
and payload bounds and would still create a category under which an unexpected
payload-bearing packet could be mislabeled. Disabling IPv6 and pinning
`arp_notify=0` before NIC-up is smaller, deterministic, and gives metal the
closed zero-frame oracle.

## Consequences

- Positive: the spike topology ships verbatim (residual risk is wiring, not
  mechanism); the veth provision, `MtlsResolve`, `workload_addr` persistence,
  and inbound install carry over with zero change. The outbound rule preserves
  its spike-proven packet semantics and gains only the exact-hit counter;
  inbound delivery (leg-S dial to the guest addr) is the proven reply path;
  converge/teardown stay structural.
- Negative: two /30s per VM slot (address budget halves per VM alloc relative
  to exec — bounded by the /17 carve, 4096 slots retained); two address
  identities per VM alloc (transit vs guest) that documentation and the
  vocabulary table must keep straight; one extra routed hop per packet inside
  the netns; VM guests intentionally have IPv6 disabled on this platform
  interface in #222, and a future IPv6 feature must redesign the closed
  pre-intercept contract rather than silently removing the suppression.
- The 6.18 appliance confirmation (ADR-0068) is the Tier-3 matrix at merge —
  the verdict is pinned to 7.0.0-29 (user-waived nft_tproxy confirmation).

## References

- [nftables statements and counter statement](https://netfilter.org/projects/nftables/manpage.html#COUNTER-STATEMENT)
  — counter records packets+bytes; a non-terminal statement is passive for
  rule evaluation, and its placement after the matches scopes what it counts.
