# Native STEP 11-06 guest transparent-route falsification

## Metadata and disposition

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Date | 2026-10-24 |
| Role | Bounded DESIGN premise validation and accepted replacement record |
| Substrate | Physical x86_64 Metal; disposable Cloud Hypervisor guest; guest Linux `7.0.0-38-generic` |
| Affected step | `11-06` guest `TcpAccept` routing branch only |
| Current validation | **Affected DESIGN APPROVED; roadmap APPROVED** |
| Replacement | **Accepted by independent DESIGN review iteration 2; no further user decision required** |

The accepted guest-network sentence, `ip rule fwmark <platform mark> lookup
local`, does not provide the return path required by the guest owner's
transparent connection to the application. The real production owner reached
`KernelGuestOwnerIo::new_app_connect`, issued the non-blocking connect, and
never observed completion. Enabling `net.ipv4.tcp_fwmark_accept=1` while
retaining that route still failed. Replacing only the affected configuration
with the exact working guest-spike shape made the same production owner cell
pass:

- `net.ipv4.tcp_fwmark_accept=1`;
- `net.ipv4.fwmark_reflect=1`;
- owner application-side socket `SO_MARK=0x295`;
- `ip rule priority 32765 fwmark 0x295/0xffffffff lookup 295`; and
- `ip route add local 0.0.0.0/0 dev lo table 295 proto boot scope host`.

This also exposed a **SPECIFICATION_AMBIGUITY** under the typed-error gate. The
accepted interface names `FwmarkRule` and `FwmarkReflect`, but it names no
semantic failure representation for the now-required local-default route or
accepted-child mark inheritance sysctl. Crafters must not fold either into a
generic `io::Error` stage or one of the existing diagnostic strings. The
accepted `FwmarkRoute` and `TcpFwmarkAccept` stages below resolve that gap.

No test body, fixture, lane, production API, model, physical-host route, or
shared Lima state was changed. The candidate production edits used only for
the disposable-guest contrast were removed byte-for-byte after the run.

## Production path and reproduced evidence

The existing Tier-3 cell launches the disposable guest through `GuestKit`,
calls production `configure_guest_transport`, constructs the production
`GuestFlowOwner` over `KernelGuestOwnerIo`, and runs a positive control after
the aborted-flow branch (`guest_flow/real_kernel.rs:1083-1107`,
`:1174-1179`, `:1315-1320`; `tests/integration/in_guest_lane.rs:295-308`).
The owner decodes the real 32-byte `TcpAccept`, checks the live launch token,
checks that V_g has not ended, and calls `start_connect_app` for
`gateway:0 -> workload_addr:listen_port` (`guest_flow/owner.rs:450-539`).
`KernelGuestOwnerIo::new_app_connect` creates the real non-blocking socket,
sets linger, `IP_TRANSPARENT`, `SO_REUSEADDR`, and `SO_MARK`, binds the
gateway address, then calls `connect` (`guest_flow/kernel_io.rs:616-647`).

| Configuration | Native result | Evidence |
|---|---|---|
| Current rule (`SO_MARK=0x1`, priority 32765 lookup table 255/local), plus the otherwise-missing `tcp_fwmark_accept=1` | **FAIL** after 60.248 s. The one-lease receipt records the synchronized current mark/rule plus added sysctl, rebuilds/stages the guest binary, then shows the owner decode flow `2147483650`, issue `100.95.0.1:0 -> 100.95.32.54:8081`, and receive no connect-completion event; the positive control did not reach `Paired`. | `.context/netns-density-295/11-06/design-route-current-plus-tcp-fwmark-accept-native.log:19-86,103-121,549-556,571-572` |
| Exact accepted configuration above, with no other production/test change | **PASS** in 30.280 s through the same cell. The one-lease receipt records the synchronized source constants, rule, route, both sysctls and socket mark, rebuilds/stages the guest binary for Linux 7.0, then records the native result. | `.context/netns-density-295/11-06/design-route-exact-spike-config-native.log:19-98,115-125` |

The original unmodified failure is independently preserved at
`.context/netns-density-295/11-06/owner-abort-reuseaddr-native.log:28-38`:
V_g was live, the request decoded, and the application connect started but did
not complete. The successful `guest-vsock-capture` and `v5c` probes use the
same dedicated mark/table/local-default route; their checked-in guest setup is
`spike-scratch/netns-density-295-guest-vsock-capture/guest/init.sh:22-26` and
`spike-scratch/netns-density-295-v5c/guest/init.sh:22-29`.

## Linux 7.0 facts

- A socket route lookup carries `sk_mark` in `flowi4_mark`
  (`vendors/linux/include/net/route.h:151-156 @ v7.0`), and a FIB rule matches
  that mark under its mask (`vendors/linux/net/core/fib_rules.c:278-310 @
  v7.0`).
- A local route produces an `RTN_LOCAL` output through loopback
  (`vendors/linux/net/ipv4/route.c:2814-2867 @ v7.0`); `RTN_LOCAL` has host
  scope (`vendors/linux/net/ipv4/fib_semantics.c:88-100 @ v7.0`). The
  dedicated catch-all route therefore gives both endpoints of this
  transparent local connection a route without making unmarked traffic local.
- With `tcp_fwmark_accept=1`, an unmarked listener copies the incoming skb mark
  into the request (`vendors/linux/include/net/inet_sock.h:105-112 @ v7.0`;
  `vendors/linux/net/ipv4/tcp_input.c:7515-7536 @ v7.0`), and the accepted
  child inherits it (`vendors/linux/net/ipv4/inet_connection_sock.c:1211-1233
  @ v7.0`). This lets the application's SYN-ACK and later replies select table
  295. `fwmark_reflect=1` makes `IP4_REPLY_MARK` preserve the triggering
  packet's mark for kernel-generated replies (`vendors/linux/include/net/ip.h:403-404
  @ v7.0`).

These source facts explain the native contrast; they do not substitute for it.

## Exact affected contract and boundaries

The accepted correction keeps the `TcpAccept` outcome, peer identity,
non-blocking connect, timeout, owner, READY meaning, and guest-only routing
namespace. It changes no wire or persisted format and adds no public type,
constant, parameter, CLI/config field, owner, retry, repair loop, or runtime
reconciliation mechanism. Component relationships and C4 diagrams are
unchanged: this is the already-owned guest-kernel setup inside
`overdrive-init`.

**Formal protocol model:** not applicable. The correction changes no owner
state, message, ordering, retry, recovery or crash protocol represented by the
existing Quint models. The claimed effect is a Linux routing result and is
therefore discharged by the existing Tier-3 production-owner cell.

**Contract shape:** bounded-change. The additional mutation universe is one
full-mask IPv4 rule, one local-default IPv4 route, and the two named IPv4
sysctls in the disposable guest network namespace. The existing P4
source-local transport cell is the required read-back boundary for those exact
postconditions, and the existing owner positive control proves `TcpAccept`
completion; DISTILL owns that bounded alignment and adds no cell or lane.

`overdrive-init` owns the four configuration effects before READY. A failure
of the rule, route, or either sysctl exits before READY with its exact typed
setup cause. There is no live teardown operation: the effects exist only in
that disposable VM's network namespace. A setup refusal or later PID-1 death
ends the VM (ADR-0150/ADR-0182; `vendors/linux/kernel/exit.c:929-930,971 @
v7.0`), and namespace destruction removes the rule and route. No host rule or
table and no other guest is mutated.

The exact crate-private driven-port and typed-error correction is normative in
the accepted feature-delta contract. It separates `FwmarkRule`, `FwmarkRoute`,
`TcpFwmarkAccept`, and `FwmarkReflect`; a generic carrier or diagnostic string
does not replace those semantic stages.

**Quality and cost.** Functional correctness is the reproduced `Paired`
positive control. Reliability is fail-before-READY with a stage-specific
cause. Security stays confined to full-mask `0x295` traffic inside one guest
network namespace; unmarked traffic is unaffected. Runtime cost is one fixed
FIB rule and one route plus the ordinary marked-flow lookup, with no task,
poll, repair, persistence or userspace payload path. Validation reuses one P4
cell/lane and adds no model check or CI lane.

## Alternatives

1. **Keep `lookup local`; add only `tcp_fwmark_accept`. Rejected.** This is the
   isolated native failure above.
2. **Add the catch-all to reserved table 255. Rejected.** It would change the
   meaning of the kernel's general local table rather than affect only marked
   owner sockets.
3. **Put the catch-all in `main`. Rejected.** It would make unmarked guest
   traffic local and erase the mark as the routing boundary.
4. **Reuse the host transparent-mTLS mark/table (`0x1`/100). Not selected.** A
   v28 probe demonstrated that shape, but it couples guest-internal routing to
   host leg-C/leg-F constants. The guest-capture lineage already proved the
   dedicated `0x295`/295 pair and the current roadmap points STEP 11-06 to that
   working configuration.

## Independent review and acceptance

Independent DESIGN review iteration 2 approved the bounded correction with no
open technical or authority finding and found that no further user decision is
required: the replacement restores already-approved D15, INB-1 and D8-PEER
outcomes without changing them.

> **GUEST-TRANSPARENT-ROUTE:** replace the invalid guest `lookup local`
> configuration with the exact dedicated `0x295`/table-295 local-default
> contract above, including `tcp_fwmark_accept=1`, the four distinct typed
> setup failures, and VM-network-namespace cleanup. Preserve every existing
> `TcpAccept`, READY, owner, timeout, no-payload and fail-closed outcome.

The canonical feature delta and roadmap are revalidated on that independent
approval. STEP 11-06 remains gated only until DISTILL aligns and independently
reviews the existing P4 source-local transport and owner cells; no new cell,
lane, model, test-support API or runtime mechanism is authorized.
