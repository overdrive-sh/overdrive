# ADR-0088: Guest-stack netns topology — routed two-/30 tap wire + guest addressing

## Status

**Accepted** (2026-08-27), **amended** (2026-08-28) to restore READY as the
post-network-initialization barrier after the step 02-03 metal counterexample.
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
host-veth ingress`, where the production `install_outbound_tproxy` rule fires
byte-for-byte unchanged. What the spike deliberately left open as design
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
bootstraps the minimal guest root, parses the platform token, applies the
static network, and writes resolver configuration **before opening/reaching
READY on the beacon session**. READY means guest platform initialization,
including networking, completed and the guest is blocked awaiting EXEC. Any
init, malformed-token, or net-apply failure powers the guest off before READY
and never execs the operator command. The host consumes that shutdown through
`VmDriver`'s already-existing pre-READY `VmmExited` boot-race arm. The beacon
Published Language is byte-for-byte unchanged; `EXIT` keeps its original
post-operator-wait meaning. Exact parameter grammar and MAC byte layout remain
DISTILL/DELIVER shapes; the lifecycle ordering is pinned here.

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
`VmGuestExitUnreported { vmm_exit_code, vmm_signal }`, with the VMM console tail
preserved as detail. The action shim records the attempt as Failed without a
Running transition. For #222's executable `[vm]+[job]` surface, the existing
Job-kind natural-exit branch then writes `TerminalCondition::Failed {
exit_code: vmm_exit_code }` without emitting `RestartAllocation`; the
finalization classifier preserves the code already carried by
`VmGuestExitUnreported` rather than fabricating a default. Both the private
restart budget and durable restart count remain unchanged. `overdrive workload
describe` already renders the final reason, detail, terminal claim, and count.
No new describe field, `ExitKind`, `TransitionReason`, status sentinel, or
beacon field is required. The future `[vm]+[service]` surface remains deferred
to #257 and keeps the generic Service start-rejection restart policy unless
that issue explicitly changes it.

Pre-READY networking is configuration-only: no DHCP, DNS lookup, reachability
probe, neighbor warm-up, socket connect, or workload send. The security order
is `network-ready ≺ READY ≺ intercept-installed ≺ EXEC-release ≺
operator-first-connect`. The metal gate must observe zero guest-originated
workload packet before intercept installation; if the guest kernel would emit
one autonomously, the implementation must suppress it before claiming READY.

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

## Consequences

- Positive: the spike topology ships verbatim (residual risk is wiring, not
  mechanism); the veth provision, the intercept rule, `MtlsResolve`, the
  `workload_addr` persistence, and the inbound install all carry over with
  zero change; inbound delivery (leg-S dial to the guest addr) is the proven
  reply path; converge/teardown stay structural.
- Negative: two /30s per VM slot (address budget halves per VM alloc relative
  to exec — bounded by the /17 carve, 4096 slots retained); two address
  identities per VM alloc (transit vs guest) that documentation and the
  vocabulary table must keep straight; one extra routed hop per packet inside
  the netns.
- The 6.18 appliance confirmation (ADR-0068) is the Tier-3 matrix at merge —
  the verdict is pinned to 7.0.0-29 (user-waived nft_tproxy confirmation).
