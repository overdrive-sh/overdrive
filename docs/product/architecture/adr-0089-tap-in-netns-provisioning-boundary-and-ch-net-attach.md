# ADR-0089: Tap-in-netns provisioning boundary + Cloud Hypervisor net attach

## Status

**Accepted** (2026-08-27), **amended** (2026-08-28) for the Q7/Q9 guest
initialization barrier after the step 02-03 metal counterexample, and
**amended** (2026-08-29) for the mutation-aware exact outbound-rule counter
oracle and to correct D6's same-allocation route and native-metal trust
boundary, and **amended** (2026-08-31) to order the existing fwmark before
TPROXY for dead-listener fail-closure and to make provision/restart teardown
total at the existing C3 boundary, and **amended** (2026-09-01) to name the
action shim's existing mTLS allocation-lifecycle dependency as an injectable
async port so the same-ID replacement protocol has a pure Tier-1 simulation
boundary. Companion
to ADR-0088 (topology + addressing).
Extends the C3 provision seam (ADR-0071 Q2/C3), the veth provisioner
(ADR-0061 converge-on-boot), `overdrive-netlink` (ADR-0085 subprocess-free),
and the `Vmm`/`VmConfig` boundary (ADR-0082/0083). GH #222.

## Context

ADR-0088 fixes WHAT the guest wire looks like. This ADR fixes WHO builds each
piece and WHERE the seams sit. The pieces: tap creation inside the per-alloc
netns; tap addressing + `ip_forward` + the host return route; carrying the
guest-net facts to the driver; getting the CH process and its `--net` attach
into the netns; and the production call sites that make VM allocs reach the
intercept at all. The Q9 metal witness also requires one kernel-observable hit
on the exact alloc rule, so this amendment extends the existing rule owner and
internal nft read projection with complete multipart, ruleset-generation,
full-program-identity, and exact packet/`skb->len` evidence without widening a
public schema. Today
`MtlsInterceptWorker::start_alloc` is deliberately
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

The 2026-08-31 same-ID correction is implemented through an awaited concrete
`Option<&Arc<MtlsInterceptWorker>>` parameter on every action-shim dispatch
form. That concrete shape makes §6's cross-component order impossible to drive
through a pure `overdrive-sim` composition: the worker's successful start path
must obtain two `std::net::TcpListener`s from the lower-level
`MtlsIntercept::bind_transparent` port. `SimMtlsIntercept` therefore performs a
real loopback bind on its successful arm, exactly as ADR-0076 requires. Moving
that test to the integration lane made its tier honest, but left the required
seeded safety/liveness/convergence invariant without a socket-free lifecycle
boundary. The production responsibility already exists — the action shim
orchestrates `start_alloc` and `stop_alloc` as one allocation lifecycle — so
§7 names that existing dependency without changing its behavior or adding a
second lifecycle owner.

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
Flipping only the fresh-start gate leaves the real same-allocation VM Job route
with NO intercept re-install. After an unclean control-plane restart while
standing intent remains, the boot-epoch `VmReclamation` drive authors a
Platform Reclamation ending for the unsupervised non-terminal VM allocation;
`WorkloadLifecycle` then emits `RestartAllocation` with the same
`AllocationId`. If the `:1880` gate remains `Exec`-only, that re-drive boots
the guest, writes `Running`, and skips `start_alloc` → egress runs CLEARTEXT,
fail-OPEN, invisible to the fresh-deploy Slice-1 AT.

Natural VM Job result/crash is not this route: the Job branch finalizes
run-once without consuming restart budget. `overdrive workload restart`
(ADR-0073) advances desired generation, ends the old instance, mints a fresh
`AllocationId`, and reaches the fresh-start gate. Both gates remain mandatory:
fresh deploy/generation replacement use the fresh-start site, while
boot-reclamation same-id re-drive uses the restart site. S-GTI-06a/06b pin
successful and failed reinstall on that latter route.

The Tier-3 route is `kvm-tests` via `cargo xtask metal run --` on the native,
non-virtualized x86_64 bare-metal host's hardware-backed `/dev/kvm`.
`kvm-tests` is only the Cargo feature name; nesting, Lima, and every virtualized
host are non-signal. A failed architecture/KVM API/virtualization preflight
aborts the feature run, and the target comes from `OVERDRIVE_METAL_TARGET` or
the gitignored workspace `.env`, never a hostname embedded in this ADR.

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

Intercept-live requires successful `start_alloc` return plus strict complete
multipart snapshots of the exact tag+handle+normalized-production-program
outbound rule on the correlated host-veth, a stable pre-EXEC counter baseline,
one unchanged full ruleset generation, and a loss-free nft change stream.
Capture and the guard continue across EXEC release; the first operator TCP SYN
must match the expected `guest_addr -> mesh VIP:port` five-tuple, every
rule-eligible packet in the bracketed window must retain that tuple, and
checked packet/byte deltas must equal the complete capture's matching-packet
count and validated IPv4 `tot_len` (the nft `skb->len` domain). Any reset,
replacement/delete/reinsert, generation change/wrap, partial/interrupted dump,
notification loss, or ambiguity fails before the original destination arrives
at leg-F. No cleartext copy appears on the external peer path and TLS records
appear on the inter-agent path. The full order is `capture-ready ≺ VMM-spawn
≺ network-ready ≺ READY ≺ intercept-live ≺ EXEC-release ≺
operator-first-connect`.

`install_outbound_tproxy` remains the sole install/adopt/delete-by-handle
owner, but is now correctly classified EXTEND: its egress expression order is
the existing `iifname` match → existing TCP match → one anonymous non-terminal
`counter` → mark → TPROXY → accept. The inbound prerouting rule also orders
its existing mark before TPROXY. With a live transparent listener, redirect
semantics are unchanged. With no listener, kernel `NFT_BREAK` occurs after the
mark side effect, so the existing fwmark/local route keeps the flow on the host
instead of restoring its original cleartext route. No second rule, quarantine,
listener adoption, or guard API is added. Userdata, redirect, match, mark,
verdict, table/chain/order, normal teardown, same-tag adoption, and boot sweep
ownership are unchanged. The metal decorator remains read-only and
cannot install, replace, reset, or delete. Its internal
`RuleInfo.counter: Option<RuleCounterSnapshot>` projection is paired with a
normalized identity for every ordered expression/operand in the production
encoder, ignoring only live counter values; same tag+handle is not sufficient.
Counter-free siblings stay `None`. `list_rules` is one dedicated-socket,
absolute-deadline multipart operation that checks kernel sender/sequence,
expected nft reply type/family, all netlink and nested attribute
lengths/alignments, and exactly one `NLMSG_DONE` with zero completion status.
It rejects a nonzero `NLMSG_ERROR`, `NLM_F_DUMP_INTR`, overrun, timeout/EOF,
missing/duplicate DONE, and malformed/trailing/partial data before uniqueness.

After `start_alloc` returns and before its initial strict full-`NFTA_GEN_ID`
single-reply `GETGEN`, the observer subscribes to `NFNLGRP_NFTABLES` with loss
reporting enabled. `GETGEN` requires exactly one complete kernel
`NFT_MSG_NEWGEN` reply with the request sequence and expected family and
rejects any extra, error, overrun, malformed, trailing, partial, timeout, or
EOF result. The completed production install precedes the guarded epoch. Every
snapshot is bracketed
`GETGEN(G) -> complete GETRULE -> GETGEN(G)` and all brackets plus the final
drain must retain initial `G`; any nft notification, `ENOBUFS`/overrun,
generation change/decrease/wrap, handle-preserving replacement,
delete/reinsert, or ambiguous mutation fails (including unrelated global
change). Two equal guarded snapshots and a quiet interval define each cut.

The exact-host-veth read-only `AF_PACKET/SOCK_DGRAM` capture is armed before
VMM spawn, retains `sockaddr_ll` direction/ifindex/protocol, detects truncation
with `recvmsg(MSG_TRUNC)` and a 65,535-byte L3 buffer, and requires zero closing
`PACKET_STATISTICS` drops. It counts every kernel-valid unfragmented IPv4/TCP
ingress skb matching the preceding rule predicates and sums its validated IPv4
`tot_len`, exactly the counter's
priority -150 `skb->len`; fragment, malformed/truncated data, capture/offload
ambiguity, or loss fails. Checked addition and subtraction without wrap require
`C > 0`, `L > 0`,
`after.packets.checked_sub(before.packets) == Some(C)`,
`after.bytes.checked_sub(before.bytes) == Some(L)`,
`before.packets.checked_add(C) == Some(after.packets)`, and
`before.bytes.checked_add(L) == Some(after.bytes)` for the complete captured
totals. Thus an in-window
`NFT_MSG_GETRULE_RESET` after any increment loses a prefix and fails; before
the first increment it changes no observed state. Regression, reset, wrap,
overflow, competing eligible traffic, or capture loss cannot false-pass. A
same-tag adopt gets a new baseline; restart boot sweep/reinstall is never
compared across handles; normal teardown still deletes only its exact handle;
sibling rules/counters remain excluded and untouched by exact
tag+handle+program. This additive workspace-internal projection changes no
API, Beacon, persistence, observation, or describe schema.

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

### 6. Provision failure and same-id replacement reuse the existing teardown

Two narrow ordering corrections apply at C3; neither adds a network recovery
model.

First, `provision_and_inject_netns` assigns a `NetSlot` before it invokes
the network provisioner
(`crates/overdrive-control-plane/src/action_shim/mod.rs:1113-1148`). If that
provision call fails after assignment, the caller must invoke the existing
`teardown_and_release_netns_raw` and capture its result before recording the
existing Failed disposition. That helper derives the
held slot, calls the allocation-keyed teardown, and releases only after
teardown succeeds (`action_shim/mod.rs:1317-1333`). The existing provisioner
already removes the netns, any stranded platform-owned host TAP, host veth, and
resolver directory idempotently
(`crates/overdrive-control-plane/src/veth_provisioner.rs:2171-2212`). A
cleanup error therefore retains the slot. The `Failed` observation continues
to carry `WorkloadNetnsProvisionFailed` as the primary cause, while cleanup
uses the existing typed error. If the Failed write errors, that store error
keeps its existing precedence; otherwise a captured cleanup error returns
through the existing `ShimError`. No cleanup aggregate, marker, scanner,
namespace-path state, persistence, or boot-GC extension is added.

Second, same-id `RestartAllocation` already awaits every resolved prior
driver stop (`action_shim/mod.rs:2136-2158`) but currently begins replacement
provisioning immediately afterward (`:2168-2188`). The corrected order is:

```text
await prior Driver::stop
  -> await prior MtlsInterceptLifecycle::stop_alloc
  -> teardown prior netns/veth/TAP/resolver state and release its slot
  -> provision/inject replacement network
  -> ensure replacement identity
  -> start replacement driver
```

Failure of either prior teardown stage returns its existing typed error before
replacement work begins. Once replacement assignment succeeds, any later
provision failure uses the same first-paragraph unwind. This removes the need
for `RestartNetworkDisposition`; no parallel cleanup or replay protocol is
sanctioned.

### 7. The action shim accepts the existing allocation lifecycle through one async port

The action shim owns a new public driven port beside
`WorkloadNetworkProvisioner`, because it is the application component that
orders the lifecycle calls:

```rust
#[async_trait::async_trait]
pub trait MtlsInterceptLifecycle: Send + Sync {
    async fn start_alloc(
        &self,
        spec: &AllocationSpec,
    ) -> Result<(), MtlsInterceptInstallError>;

    async fn stop_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<(), MtlsInterceptStopError>;
}
```

The canonical path is
`overdrive_control_plane::action_shim::MtlsInterceptLifecycle`. It is not put
in `overdrive-core`: its two existing error types are owned by
`overdrive-worker`, and moving or duplicating those errors would widen this
bounded change. It is not put beside the lower-level `MtlsIntercept` port in
`overdrive-worker`: `MtlsIntercept` names the three privileged install
primitives the worker orders, whereas `MtlsInterceptLifecycle` names the two
complete effects the action shim orders. The two ports have different owners,
and neither replaces the other.

Both methods are async because returning is the effect-completion boundary.
`start_alloc(Ok)` means this adapter's complete allocation interception owner
is live and may be followed by EXEC release. A repeated start for the same
allocation must first converge the prior lifecycle; failure is the existing
`MtlsInterceptInstallError::PriorTeardown`, and no replacement becomes live.
Any other install failure remains the existing `MtlsInterceptInstallError`.
`stop_alloc(Ok)` means admission is closed and every listener/rule/task/
connection teardown owned by this lifecycle has completed; only then may the
caller tear down structural networking or begin replacement work. An unknown
allocation is an idempotent `Ok(())`. `stop_alloc(Err)` is the existing
`MtlsInterceptStopError`, retains only the failed teardown work that a later
call may retry, and is not completion. No method may detach either effect.

Those common postconditions are substrate-neutral. The production
implementation's stronger listener/nft/task mechanics remain on
`MtlsInterceptWorker`; the simulation implementation owns logical lifecycle
state only. Every trait method receives four-section behavior rustdoc
(preconditions, postconditions, edge cases, observable invariants). There is
deliberately no `probe`, `is_live`, event accessor, owner-shutdown method,
generation, cancellation token, or retry method on the production trait.
`shutdown_owner` remains a concrete process-owner operation held by
`ServerHandle`, not an allocation action.

#### 7.1 Production binding and exact dispatcher shape

The production binding is the existing owner, without a wrapper or second
state machine:

```rust
#[async_trait::async_trait]
impl MtlsInterceptLifecycle for Arc<MtlsInterceptWorker> {
    async fn start_alloc(
        &self,
        spec: &AllocationSpec,
    ) -> Result<(), MtlsInterceptInstallError> {
        MtlsInterceptWorker::start_alloc(self, spec).await
    }

    async fn stop_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<(), MtlsInterceptStopError> {
        MtlsInterceptWorker::stop_alloc(self, alloc_id).await
    }
}
```

The implementation is intentionally for `Arc<MtlsInterceptWorker>` because
the existing inherent methods take `self: &Arc<Self>` to create the weak and
owned task references used by the worker. Changing those receivers or adding
a forwarding newtype would be unrelated lifecycle refactoring.

`dispatch`, `dispatch_with_network_provisioner`, and private
`dispatch_single` replace only this parameter:

```rust
mtls_lifecycle: Option<&dyn MtlsInterceptLifecycle>
```

`fail_closed_on_mtls_install` accepts
`&dyn MtlsInterceptLifecycle`; `cleanup_restart_abort` accepts
`Option<&dyn MtlsInterceptLifecycle>`. The private C3 helper does not need the
port and receives only `mtls_composed: bool`, supplied as
`mtls_lifecycle.is_some()`. Every existing gate and `DriverType::Exec |
DriverType::Vm` check continues to use that same presence bit. The existing
`ShimError::MtlsStop`, install-error classification, cleanup precedence, and
fail-closed tails are unchanged.

`AppState::mtls_worker`, both `AppState` constructors,
`MtlsInterceptWorker::new`, and `ServerHandle::mtls_worker_owner` remain
concrete and unchanged. `run_server` still constructs exactly one
`Arc<MtlsInterceptWorker>` and the server handle still clones that same owner
for `shutdown_owner`. `dispatch_with_workflow_intent` and
`dispatch_with_workflow_intent_and_network_provisioner_for_test` project the
borrowed Arc at their call sites:

```rust
let mtls_lifecycle = state
    .mtls_worker
    .as_ref()
    .map(|worker| worker as &dyn MtlsInterceptLifecycle);
```

Direct callers passing `Some(&worker)` make the same explicit coercion;
`None` callers do not change. This keeps the process-owner composition intact
while making the action dispatcher independently injectable.

#### 7.2 Pure simulation binding and observation surface

`overdrive-sim/src/adapters/mtls_intercept_lifecycle.rs` adds
`SimMtlsInterceptLifecycle`. It implements the lifecycle port directly and
never constructs `MtlsInterceptWorker`, `SimMtlsIntercept`, a listener, a
socket address, an enforcement task, or a kernel-rule guard. No Cargo edge is
added: `overdrive-sim` already depends directly on both
`overdrive-control-plane` and `overdrive-worker`.

`overdrive-sim/src/adapters/mod.rs` declares
`pub mod mtls_intercept_lifecycle` and re-exports the adapter, state, event,
and snapshot types. Their canonical caller path is therefore
`overdrive_sim::adapters::{SimMtlsInterceptLifecycle, ...}`; this bounded
change does not add another crate-root re-export.

The exact Sim-only surface is:

```rust
pub struct SimMtlsInterceptLifecycle {
    // private interior state
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimMtlsInterceptLifecycleState {
    Live,
    TeardownPending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimMtlsInterceptLifecycleEvent {
    StartCompleted { alloc_id: AllocationId },
    StartPriorTeardownFailed {
        alloc_id: AllocationId,
        failures: Vec<String>,
    },
    StopCompleted {
        alloc_id: AllocationId,
        prior: Option<SimMtlsInterceptLifecycleState>,
    },
    StopFailed {
        alloc_id: AllocationId,
        failures: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimMtlsInterceptLifecycleSnapshot {
    pub allocations: BTreeMap<AllocationId, SimMtlsInterceptLifecycleState>,
    pub events: Vec<SimMtlsInterceptLifecycleEvent>,
}

impl SimMtlsInterceptLifecycle {
    pub fn new() -> Self;

    pub fn inject_stop_failure_once(
        &self,
        alloc_id: AllocationId,
        detail: impl Into<String>,
    );

    pub fn snapshot(&self) -> SimMtlsInterceptLifecycleSnapshot;
}
```

`Default` is identical to `new`: no allocation state, fault, or event. Each
`inject_stop_failure_once` appends one transient failure for that allocation.
Multiple injections are consumed FIFO, one per owning stop call. The next stop
that owns `Live` or `TeardownPending` state consumes one queued detail and
returns
`MtlsInterceptStopError { alloc_id, failures: vec![detail] }`; absence remains
idempotent and does not consume the queued fault. A stop of `Live` first closes
logical admission by changing it to `TeardownPending`. Failure leaves that
state and records `StopFailed` with the exact returned error's allocation and
failure vector; success removes it and records
`StopCompleted`. A retry therefore drives the same retained owner rather than
creating another one. A start from `Live` or `TeardownPending` runs the same
logical stop transition before inserting `Live`; a failed prior transition
returns `MtlsInterceptInstallError::PriorTeardown` and records
`StartPriorTeardownFailed` with the nested stop error's exact allocation and
failure vector. A successful start inserts `Live` and records
`StartCompleted`. The adapter scripts no other install failures; lower-level
install-fault coverage remains the responsibility of `SimMtlsIntercept` and
ADR-0076. Exactly one outcome event is appended per public trait call. The
prior-stop state transition performed inside a repeated `start_alloc` is
therefore represented by that call's `StartCompleted` or
`StartPriorTeardownFailed` event, not by an additional synthetic `Stop*`
event. On `StopCompleted`, `prior` is the allocation state observed at method
entry (`None` for the idempotent absent case).

The snapshot is an adapter-sim observation surface, not a production trait
method. It captures state and events under one lock and never exposes the
queued fault script. A same-ID invariant may place an invariant-local tracing
decorator around this adapter and the existing Driver/network ports to create
one cross-port order log; the decorator must delegate every operation and may
not author lifecycle state. This is the same driven-port decorator pattern as
the terminal-contention invariant's `ObservationStore` scheduler.

#### 7.3 Required Tier-1 invariant

The seeded `overdrive-sim` invariant drives the real
`dispatch_with_network_provisioner` boundary. It first dispatches a successful
VM `StartAllocation` and requires the lifecycle snapshot to contain exactly
`alloc -> Live`; preloading the map is not equivalent evidence. It then
dispatches `RestartAllocation` with the same `AllocationId` and checks:

```text
prior Driver::stop completed
  < prior MtlsInterceptLifecycle::stop_alloc completed
  < prior structural-network teardown and slot release
  < replacement provision
  < replacement identity present at Driver::start
  < replacement Driver::start completed
  < replacement MtlsInterceptLifecycle::start_alloc completed
```

The seed selects a clean run, one transient mTLS-stop failure, or one transient
structural-network teardown failure. At either failure cut, dispatch returns
the existing typed `ShimError`, no later replacement event occurs, and the old
slot is retained. For the mTLS cut the snapshot is `TeardownPending`; for the
network cut it is absent because mTLS teardown has already completed.
Re-dispatching the same action must converge within one additional bounded
attempt: the retained teardown completes, the old slot is released before
being selected for the replacement, the final lifecycle state is the same
allocation ID mapped to `Live`, and exactly one replacement driver start plus
one replacement `StartCompleted` event exist. The invariant reports the
reproducing seed and states safety, liveness, and convergence separately.

Its negative control removes or reorders the observed mTLS-stop completion (or
treats `TeardownPending` as absent); the pure checker must fail. The existing
integration-lane BTR-03 test remains independent evidence for the real
worker/listener/rule implementation. The Tier-1 invariant complements it; it
does not move, weaken, or replace it.

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
  zero new crates/daemons and one internal two-method lifecycle port (§7); the
  intercept path from `InterceptedConnection` down is reached with zero
  behavioral change; the gate flip is a **two-site** production call-site
  change (fresh start + restart) whose absence was the #236 failure mode and
  whose partial application would leave boot-reclamation same-id VM re-drives
  cleartext fail-open. Initial deploy and generation replacement continue to
  use the fresh-start site.
- Positive (Tier-1 lifecycle evidence): the real action dispatcher can now
  compose the existing allocation lifecycle with a socket-free Sim adapter,
  making §6's same-ID teardown order and transient-failure convergence a
  seeded safety/liveness/convergence invariant. The existing integration test
  continues to own real worker/listener/rule evidence.
- Positive (isolation, defense-in-depth; Medium confidence): running CH inside
  the workload netns confines a compromised VMM's *network* reach to the tenant
  namespace — the Firecracker-jailer isolation direction — at no control-surface
  cost, since CH's api/vsock/console surfaces are netns-transparent (§4). This
  is a sound direction, not a proven ordering (research Gap G2).
- Positive (exact kernel evidence): one anonymous counter on the production
  alloc egress rule plus strict complete generation-bracketed `GETRULE`
  snapshots, full encoder-derived program identity, a loss-detecting nft
  change guard, and exact captured packet/`skb->len` equality prove a hit on
  one unchanged rule; reset, replacement, wrap, notification loss, and partial
  dumps cannot false-pass. The mark-before-TPROXY tail additionally keeps both
  prerouting directions fail-closed after listener loss without a second rule;
  live-listener handling, lifecycle ownership, and every downstream
  `InterceptedConnection` consumer remain unchanged.
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
- [nftables statements and counter statement](https://netfilter.org/projects/nftables/manpage.html#COUNTER-STATEMENT)
  (official netfilter documentation): the counter records packets+bytes and is
  non-terminal/passive for rule evaluation when placed between matches and the
  terminal verdict.
