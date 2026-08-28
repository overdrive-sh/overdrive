# ADR-0089: Tap-in-netns provisioning boundary + Cloud Hypervisor net attach

## Status

**Accepted** (2026-08-27), **amended** (2026-08-28) for the Q7/Q9 guest
initialization barrier after the step 02-03 metal counterexample. Companion to
ADR-0088 (topology + addressing).
Extends the C3 provision seam (ADR-0071 Q2/C3), the veth provisioner
(ADR-0061 converge-on-boot), `overdrive-netlink` (ADR-0085 subprocess-free),
and the `Vmm`/`VmConfig` boundary (ADR-0082/0083). GH #222.

## Context

ADR-0088 fixes WHAT the guest wire looks like. This ADR fixes WHO builds each
piece and WHERE the seams sit. The pieces: tap creation inside the per-alloc
netns; tap addressing + `ip_forward` + the host return route; carrying the
guest-net facts to the driver; getting the CH process and its `--net` attach
into the netns; and the production call sites that make VM allocs reach the
intercept at all — today `MtlsInterceptWorker::start_alloc` is deliberately
gated on `DriverType::Exec` at **TWO** action-shim install sites, the
fresh-start `Running` arm (`action_shim/mod.rs:1584`, comment block
`:1559-1569`) AND the restart `Running` arm (`:1880`, comment `:1877`
"symmetric"), a gate whose own comment names #222 as its lifter.

Boundary facts honored: the provisioner creates, the driver ENTERS (Q2/C3
ratified — driver-creates was rejected for exec and stays rejected here);
`overdrive-host` is `#![forbid(unsafe_code)]`; `Vmm` owns the spawn
(ADR-0082); Cloud Hypervisor is the only sanctioned subprocess (ADR-0085);
tun/tap devices are ioctl-created (netlink cannot create them) but CAN be
netlink-moved between netns.

## Decision

### 1. The C3 seam grows a VM branch; the intercept gate is lifted

`provision_and_inject_netns` keeps its kind-agnostic half (slot → plan →
netns+veth, byte-identical), then matches the spec's `DriverPayload` VM arm:
derive the pure `VmTapPlan` from the SAME slot, run the tap converge, and
inject onto the spec — `workload_addr = guest_addr` plus the guest-net
channel (tap name, MAC, guest-addressing inputs; a pure in-memory field
family with the same no-serde discipline as `netns`/`host_veth`). The
`DriverType::Exec` gate on the intercept install extends to VM-kind at **BOTH**
install sites — the fresh-start `Running` arm (`:1584`) AND the restart
`Running` arm (`:1880`, comment `:1877` "symmetric"). With the tap wire the
host-veth carries the guest's traffic, so the gate's guard condition is
dissolved; the D-MTLS-18 fail-closed posture (install failure ⇒ drive the alloc
terminal) applies to VM allocs unchanged.

**Both install gates flip, or the feature ships a silent cleartext regression.**
Flipping only the fresh-start gate leaves a *restarted* VM alloc — restart
budget / crash-recovery / `overdrive workload restart` (ADR-0073), all live for
VM kind — with NO intercept re-install: it boots the guest, writes `Running`,
and skips `start_alloc` → egress runs CLEARTEXT, fail-OPEN, invisible to the
fresh-deploy Slice-1 AT (which never exercises the restart arm). A Tier-3
**restart** AT (`kvm-tests` via `cargo xtask metal run --`) pins the `:1880`
flip: a restarted VM alloc re-installs the intercept and is driven terminal
fail-closed on install failure (DISTILL authors it).

**Teardown is ungated-by-design — no flip, and none must be added.** The two
intercept-teardown sites — `stop_alloc` at the FinalizeFailed arm (`:1269`,
`!is_stable`) and the StopAllocation arm (`:2038`) — are gated ONLY on
`mtls_worker.is_some()`, NOT on `DriverType`, so they already cover VM allocs
(`stop_alloc` is idempotent — a no-op for an alloc with no intercept). There is
NO leak-on-stop bug. The inverse hazard is the one to guard: adding an
`Exec` gate to either teardown site would leak the VM alloc's nft rule on stop
— the current ungated shape structurally avoids it.

**Born-captured is an ORDERING INVARIANT, not boot-then-install alone.** The
install fires at the `Running` arm (after `driver.start()` receives READY), so
READY is a security boundary: under the 2026-08-28 amendment,
`overdrive-init` completes minimal-root bootstrap, verifies the NIC is down,
disables per-interface IPv6 and reads it back, writes/reads back IPv4
`arp_notify=0`, parses the platform token, applies static IPv4, and writes the
resolver before READY. A failure
powers the guest off before READY and resolves through the existing pre-READY
`VmmExited` driver start-rejection arm. A successful READY means the guest is
network-ready but blocked awaiting the existing EXEC reply.

The platform then gates EXEC-release on intercept-install success. The closed
packet contract is **zero guest-originated L2 frames** from capture-ready
before VMM spawn through intercept-live; there is no autonomous-control
allowlist. Disabling IPv6 before NIC-up suppresses link-local DAD/router-
solicitation, and `arp_notify=0` suppresses gratuitous ARP. The static path has
no DHCP, DNS lookup, probe, neighbor warm-up, socket connect, or workload send.
On install `Err`, EXEC is never sent (D-MTLS-18).

The Tier-3 witness is an observation-only decorator over the real `Vmm` port.
After C3 provisions the alloc netns/tap/host-veth and before delegating to real
CH, it binds all-EtherType capture to the exact tap ifindex inside that netns
and a correlated witness to the exact host-veth ifindex, then acknowledges
ready. Correlation covers alloc id, slot, netns inode, both names+ifindices,
guest MAC, and guest address. Until intercept-live, every guest-to-host frame
is failure: tagged/untagged, any EtherType/L3/L4 protocol, source MAC,
destination, and payload presence. Capture drop/overflow, truncated/malformed
records, unknown direction/timestamp, absent readiness, or ambiguous identity
also fail. Thus no payload-bearing TCP/UDP or unexpected destination can hide
under "control traffic."

Intercept-live requires both successful `start_alloc` return and observation
of the exact outbound rule on the correlated host-veth. Capture continues
across EXEC release; the first operator TCP SYN must match the expected
`guest_addr -> mesh VIP:port` five-tuple, increment that rule and arrive at
leg-F, with no cleartext copy on the external peer path and TLS records on the
inter-agent path. The full order is `capture-ready ≺ VMM-spawn ≺ network-ready
≺ READY ≺ intercept-live ≺ EXEC-release ≺ operator-first-connect`.

**Superseded Q7 shape.** The former post-READY/pre-EXEC `EXIT` classification
is not a deterministic protocol phase: step 02-03 metal RED showed the host can
install the intercept and flush EXEC while the guest is still applying
networking, after which the same pre-operator `EXIT 78` looks like an operator
crash. A successful host flush is not a guest-consumption acknowledgement.
Status sentinels and delays do not repair that race; a new acknowledgement or
field/message is unnecessary and remains rejected. After this amendment,
`EXIT` retains its post-operator-wait meaning and every platform-init failure
precedes READY.

No new public lifecycle surface is needed. A pre-READY poweroff is classified
by the existing `VmGuestExitUnreported { vmm_exit_code, vmm_signal }` start
rejection with selected diagnostic detail and recorded as a Failed attempt with
no Running transition. "Captured console" means CH's existing guest-serial file,
not hypervisor stderr: `VmDriver` asynchronously snapshots the final 8 KiB /
five line fragments from `VmRunDir::console_log()` after VMM exit and before
run-dir cleanup, using existing `VMM_CONSOLE_TAIL_MAX_BYTES` and
`STDERR_TAIL_LINES`. Nonempty guest console is primary detail; separately
bounded `VmmExit.stderr_tail` is fallback for absent/empty/unreadable console; a stable
bounded message covers neither. Snapshot failure never masks cleanup.

For #222's executable `[vm]+[job]` surface, the Job-first branch remains but
pure `WorkloadLifecycle::classify_natural_exit_terminal` is **EXTENDED** so
every `VmGuestExitUnreported { vmm_exit_code, .. }` yields
`TerminalCondition::Failed { exit_code: vmm_exit_code }`. Its property covers
every `Option<i32>` and has the exact rustdoc declaration
`/// CONTRACT_SHAPE: pure-function.`. A reconciler/action-shim example proves
`FinalizeFailed` only, no `RestartAllocation`, returned private View unchanged,
and final durable `restart_count` unchanged. `overdrive workload describe`
already renders the selected detail and lifecycle facts, so no Beacon,
`VmmExit`, describe, enum, or observation field is added. The future
`[vm]+[service]` surface remains #257's concern and keeps generic Service
restart policy unless that issue changes it. The D6 install site, deferred
EXEC reply, and single `start_alloc` remain unchanged.

### 2. The tap converge lives in the veth provisioner, Bar-1

Four idempotent observe → diff → converge steps beside the veth steps: (a)
tap exists + persistent in the netns; (b) tap addressed as the guest gateway;
(c) `net.ipv4.ip_forward=1` in the netns (`/proc/sys` write, the ADR-0085
file-I/O shape); (d) host return route `<guest /30> via plan.workload_addr
dev plan.host_veth` (add-if-missing). Fail-closed through the existing
`ShimError::WorkloadNetnsProvision`. **Teardown is structural**: deleting the
netns destroys the tap; deleting the veth drops the return route —
`teardown_workload_netns` is unchanged. **Return-route ownership is therefore
the provisioner's** (D3): it is per-alloc host-routing state with the same
lifecycle as the veth it rides on. Bar-2 promotion (continuous drift repair)
rides the existing #197/#234 network-reconciler track — no new reconciler.

### 3. Tap creation is subprocess-free

Open `/dev/net/tun`, `TUNSETIFF` + `TUNSETPERSIST` (ioctl, via `nix`), then
netlink `RTM_SETLINK`/`IFLA_NET_NS_FD` moves the device into the netns;
in-netns address/up reuse the ops `overdrive-netlink` already performs for
the veth end. `overdrive-netlink` EXTENDs with the tuntap create/move
primitives.

### 4. CH enters the netns via the existing wrapper-argv mechanism

`Vmm` composes `ip netns exec <ns>` ahead of the existing wrapper chain when
the config carries a netns, and appends `--net tap=<name>,mac=<mac>`. This is
byte-for-byte the spike's launch shape (CH v53.0 attaches the pre-created
persistent tap by name; the "Tap already exists" warning is the benign
expected path). `overdrive-host` stays `#![forbid(unsafe_code)]` — the netns
entry is an exec-time wrapper on the already-sanctioned CH subprocess, not a
provisioning shell-out. Running the VMM *inside* the workload netns is the
industry-standard hardened-microVM shape: the Firecracker jailer `setns`-es
its VMM into the target netns before exec (its `--netns`), and `ip netns exec`
is the CLI spelling of that same `setns`-before-`exec` (see A2). CH's
unix-socket surfaces (api-socket, vsock backend, console file) are
filesystem-based and unaffected by the netns. `VmConfig`'s
`netns` goes from carried-but-unconsumed to consumed; the net attach is
carried such that "netns without NIC" is unrepresentable for mesh VM allocs
(exact struct shape → DISTILL; the sum-types-over-sentinels fold is the
recommended shape).

### 5. Inbound (peer → guest) is topology-settled here, built with #257

`install_inbound_tproxy` needs zero change (its `daddr` match keys on
`workload_addr`, which ADR-0088 makes the guest addr); leg-S delivery is a
plain dial to the guest addr over the spike-proven host→guest reply path; the
leg-S mark exemptions already head both shared chains. The BUILD is deferred
to **#257** (existing issue): until it removes the `[vm]`+`[service]` parse
rejection no production path can declare a guest listener, so a #222 inbound
slice would have no serve+deploy driver — the #236 dead-mechanism precedent
this codebase refuses to repeat. #257 should open with a thin Tier-3 AT for
the one residual empirical gap (a host-originated SYN into the guest, vs. the
proven reply leg).

## Alternatives Considered

### A1. Driver creates the tap (VmDriver or Vmm provisions at start)

**Rejected**: violates the ratified provisioner-creates/driver-enters split
(Q2/C3); duplicates provisioning in a second component class; the driver
cannot converge-on-boot what it did not derive; and a `Vmm`-side create would
put privileged netdev mutation inside the spawn adapter.

### A2. Tap fd-passing (`--net fd=`) with CH staying in the host netns

Avoids the `ip netns exec` wrapper by opening the tap fd in the workload netns
from a `setns`'d helper and passing it (`--net fd=<N>`) to a CH that stays in
the HOST netns. **Rejected — and the evidence confirms the wrapper on the
merits, not on ease.** The guest-NIC-attachment research (2026-08-27,
References) settled the question against fd-passing, in one respect *correcting
a premise* that appeared to favour it:

1. **The hardened-microVM precedent points AT the wrapper, not at fd-passing.**
   The premise that fd-passing is "Kata- / Firecracker-jailer-shaped" is wrong.
   The Firecracker *jailer* — the most security-scrutinised production microVM
   manager (AWS Lambda / Fargate) — `setns`-es the VMM *into* the target netns
   before dropping privileges and exec'ing (its `--netns`). That is the wrapper
   (§4), not fd-passing; `ip netns exec <ns> cloud-hypervisor …` is the CLI
   spelling of exactly that `setns`-before-`exec`. The genuine fd-passing
   precedent is **CNI-handoff-shaped** — Kata inherits a CNI-created `veth` it
   does not own and must bridge into a VM tap — and does NOT transfer, because
   Overdrive **creates its own tap+veth** by construction (see A6 for why Kata's
   endpoint zoo does not apply here).
2. **Isolation direction favours the wrapper** (defense-in-depth, Medium
   confidence — the *direction* is standard `namespaces(7)` containment
   doctrine, not a single-source guarantee). VMM-in-workload-netns confines a
   compromised VMM's network reach to the tenant netns; fd-passing leaves the
   VMM in the HOST netns with host-network reach. fd-passing's ONLY isolation
   counter-advantage — the VMM retaining host reachability — is a **non-need**:
   CH's control/vsock/console surfaces are UNIX-domain / filesystem paths
   (mount-ns scoped, netns-transparent — §4, spike P2), so entering the netns
   hides none of them.
3. **Statelessness / operability.** Tap-by-name is a stateless reference to a
   persistent tap the provisioner already converged; fd-passing adds
   reboot-fragile fd state (CH documents reboot breakage unless the fd is
   duplicated), a cross-netns privileged `setns` thread (ADR-0085 discipline),
   and the SCM_RIGHTS plumbing CH adopted after its raw-fd-over-API footgun.
   Fewer moving parts at reboot/crash boundaries is a correctness property, not
   a convenience.

`fd=` vs `tap=` is **not** a datapath or performance axis — identical tap,
identical virtio-net, identical bytes, identical interceptability — so the
choice is correctly settled on process placement, isolation, and operability,
where the wrapper wins. The "reopen with evidence, not preference" spirit
stands, but the bar is now met the OTHER way: the evidence confirms the
wrapper. Preserved here as a rejected-with-evidence alternative, not a queued
refinement.

### A3. Worker-side `pre_exec` setns before handing to `Vmm`

**Rejected**: crosses the ADR-0082 boundary (`Vmm` owns the spawn); `pre_exec`
is `unsafe` and would smuggle spawn mechanics into the driver.

### A4. A dedicated tap/network reconciler (Bar-2 now)

**Rejected**: `reconcilers.md` names converge-on-boot the valid intermediate;
runtime drift repair for the whole netns/veth/tap family is the existing
#197/#234 promotion, where these steps ride along. Building it now forks the
provisioning into two mechanisms.

### A5. `start_alloc` owns the return route

**Rejected**: the worker's `start_alloc` owns nft rules + listener legs (RAII
per-alloc guards); the return route is host routing state with the
provisioner's lifecycle (structural teardown with the veth). Splitting one
alloc's routing across two owners re-creates the split-authority shape
ADR-0087 dissolved.

### A6. Higher-throughput NIC models (macvtap or VFIO/SR-IOV passthrough)

**Rejected for #222 — structurally incompatible with transparent
interception, not merely a perf trade.** Both remove the host-namespace
ingress the proven nft-TPROXY-on-veth intercept depends on, so neither
reopens the wrapper decision (A2):

- **macvtap** attaches the guest NIC directly onto a lower/host link and
  short-circuits the host IP stack — guest egress never reaches a host
  `prerouting` hook, so the `iifname` TPROXY rule can never fire (High
  confidence; three independent primaries — libvirt, Red Hat RHEL, and Cloud
  Hypervisor's own macvtap doc, which notes the host cannot even *reach* the
  guest). The disqualification holds across all macvtap modes.
- **VFIO/SR-IOV** device passthrough hands the PCIe function/VF straight to
  the guest via the IOMMU; packets move by DMA between device and guest memory
  with the host kernel out of the datapath entirely — no host veth, no
  prerouting hook, nowhere to run TPROXY (High confidence).

Kata offers macvtap (and its tcfilter / bridge / ipvlan endpoints) to bridge a
**CNI-inherited** interface — an axis Overdrive does not have (A2 item 1). The
only Kata reasons that transfer are "the VM needs a tap" and "run the VMM
jailed in a netns", both already satisfied by §4. macvtap and VFIO could serve
a hypothetical FUTURE non-mesh, max-throughput VM tier that by definition
forgoes mesh mTLS; they are out of scope for #222. The perf axis is bounded
here so a future reader does not mistake a throughput number for a reason to
reopen A2.

## Consequences

- Positive: one provisioning mechanism, one converge family, one slot key;
  zero new crates/ports/daemons; the intercept path from `InterceptedConnection`
  down is reached with zero change; the gate flip is a **two-site** production
  call-site change (fresh start + restart) whose absence was the #236 failure
  mode and whose partial application would leave restarted VMs cleartext
  fail-open.
- Positive (isolation, defense-in-depth; Medium confidence): running CH inside
  the workload netns confines a compromised VMM's *network* reach to the tenant
  namespace — the Firecracker-jailer isolation direction — at no control-surface
  cost, since CH's api/vsock/console surfaces are netns-transparent (§4). This
  is a sound direction, not a proven ordering (research Gap G2).
- Negative: `ip netns exec` adds iproute2 to the launch path (present on the
  appliance; the wrapper is exec-time only); the C3 seam gains kind-awareness
  (a `DriverPayload` match — the tagged enum makes the branch total);
  `overdrive-init` gains a responsibility (platform initialization including
  silent static net apply) whose failure mode must stay fail-closed (power off
  before READY, never exec); IPv6 is intentionally disabled on this platform
  NIC in #222 so a future IPv6 feature must redesign the zero-frame contract.
- Positive (diagnostics): guest PID 1 errors reuse CH's existing serial file;
  one bounded pre-cleanup read in `VmDriver` corrects observability without
  widening `VmmExit`, Beacon, observations, or describe.
- The walking-skeleton egress slice (feature-delta § "Walking-skeleton") is
  the BLOCKING first deliverable: `[vm]`+`[job]` egress through a real
  `overdrive serve` + `overdrive deploy`. Its VMM decorator is observation-
  only and delegates to real CH; no functional network path is test-only.

## References

- Research: `docs/research/dataplane/guest-nic-attachment-netns-vs-fd-passing-comprehensive-research.md`
  (2026-08-27, 15 sources) — settles the wrapper-vs-fd-passing question on
  evidence and disqualifies macvtap/VFIO for interception (backs §A2 + §A6).
  Primary sources include the Firecracker jailer `--netns`, Kata networking
  design, Cloud Hypervisor `--net` / macvtap docs, libvirt + RHEL macvtap, the
  kernel VFIO documentation, and `setns(2)`/`namespaces(7)` semantics.
- Spike evidence: `docs/feature/guest-stack-transparent-mtls-intercept/spike/findings.md`
  (verdict WORKS; kernel 7.0.0-29; CH v53.0).
