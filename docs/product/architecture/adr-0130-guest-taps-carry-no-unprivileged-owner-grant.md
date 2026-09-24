# ADR-0130 — Guest TAPs are owned by uid 0 so no unprivileged process without the queue can attach one

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R4. Proposed 2026-09-23 and revised 2026-09-24 through
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. Depends on ADR-0127 and ADR-0128. ADR-0143 denies
the queue holder the TAP-mutating ioctls; this decision owns the owner uid and
the audit read-back set. The title is authoritative: owner uid 0 is not an
unprivileged grant.

## Context

The persistent-TAP creator sets `TUNSETOWNER` to the shared VMM uid (4200). That
grant existed so Cloud Hypervisor, running unprivileged as that uid, could open
the TAP by name. After ADR-0127 and ADR-0128 the root `overdrive serve`
launcher attaches the single queue and Cloud Hypervisor only inherits it, so no
unprivileged process needs to attach.

The kernel decides who may attach a queue to an existing TAP in
`tun_not_capable()` (`drivers/net/tun.c`). It requires `CAP_NET_ADMIN` only
when an owner or a group is set and the caller does not match it. A new device
starts with an invalid owner and an invalid group, so for an ownerless TAP the
check is false for every caller: anyone who can open `/dev/net/tun` may attach
to an unattached TAP it can name. `/dev/net/tun` is mode `0666` under systemd's
default udev rule. Firecracker's own CI depends on this behaviour: it creates
TAPs with no owner and opens them as jailed uid 1234 without `CAP_NET_ADMIN`.
The kernel TUN/TAP document's sentence about "devices which aren't owned by the
user" describes the owned case; the code is authoritative for the ownerless
case. (Research: `docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 2.1–2.3 and Conflict 1.)

Every #295 guest TAP is persistent and lives in the host network namespace
(ADR-0114), so every VMM process can name every TAP. Prior art isolates TAPs in
one of three ways: an owner uid (crosvm), descriptor custody plus a
non-persistent TAP (libvirt), or a per-VM network namespace (Firecracker,
where the by-name lookup is namespace-scoped). Descriptor custody is adopted by
ADR-0128, but the TAP stays persistent and host-scoped, so custody alone leaves
the by-name attach open whenever the TAP holds no queue, for example between a
VMM's exit and the TAP's teardown.

## Decision

Guest TAPs are created with owner uid 0, the uid of the root launcher, and with
no group. The shared guest-network owner's TAP identity read-back expects owner
uid 0. The kernel therefore refuses a queue attach from any process that is
neither uid 0 nor holding `CAP_NET_ADMIN`, which includes every confined VMM
that does not already hold that TAP's queue.

The owner uid does not bind the process that holds the queue. The kernel runs
no capability or owner check for any ioctl on an attached tun queue — the only
gate before the ioctl `switch` is attachment (`drivers/net/tun.c`
`__tun_chr_ioctl`, `3264-3266`). ADR-0143's launch seccomp filter therefore
denies every Cloud Hypervisor thread the TAP-mutating requests, returning
`EPERM`. The complete list of the function's arms and their disposition, with
the owner held at uid 0 and that filter in force, verified from
`drivers/net/tun.c` (`__tun_chr_ioctl`; the arm set is the same in v6.18 and
current mainline, and the line numbers cited in this ADR are mainline's):

| ioctl | Effect on the holder's own TAP | Disposition |
|---|---|---|
| `SIOCSIFHWADDR` | Sets the TAP's host-side MAC (`dev_set_mac_address_user`; `IFF_LIVE_ADDR_CHANGE` lets it apply while up). Setting it to another guest's virtio-net MAC poisons the bridge FDB and can redirect that guest's host-originated frames to this port (reproduced natively, increment-z). | **Prevented** by the launch filter (ADR-0143). A change made anyway, through a gap in the filter or by another process, is closed at delivery by ADR-0142's TAP egress classifier: a TAP egresses unicast only to its registered guest MAC, so neither the redirected frames nor flooded unknown unicast reach it. It is detected by the host-side MAC read-back: per-allocation damage, so only that VM is killed (ADR-0124). Its teardown deletes the TAP, which removes the port and the poisoned `LOCAL` FDB entry on it. The next host unicast to the victim is flooded and admitted only by the victim's own egress classifier, and the bridge re-learns the victim's MAC on the victim's port from the victim's next frame. |
| `TUNSETOWNER` | Hands the TAP to uid 4200. | **Prevented** by the launch filter (ADR-0143). A change made anyway is detected by the owner-uid read-back → per-allocation damage → kill. |
| `TUNSETGROUP` | Sets the TAP group. | **Prevented** by the launch filter (ADR-0143). A group change would grant nothing in any case while the owner stays uid 0: `tun_not_capable` (`tun.c:516-524`) refuses an attach when `(owner-mismatch OR group-mismatch) AND no CAP_NET_ADMIN`, and for owner 0 the owner-mismatch disjunct holds for every uid-4200 caller regardless of the group. So the audit needs no group read-back. (This corrects the research addendum's A1 claim that a group match alone admits a caller; that holds only for an ownerless TAP, not for owner 0.) |
| `TUNSETPERSIST(0)` | Makes the TAP vanish when the holder exits. | **Prevented** by the launch filter (ADR-0143). A change made anyway is detected by the persistence read-back, and teardown converges on absence. |
| `TUNSETCARRIER` | Toggles carrier only (`tun_net_change_carrier` touches carrier alone). | **Prevented** by the launch filter (ADR-0143). It could not open the activation gate in any case, which is administrative state and needs `CAP_NET_ADMIN`. |
| `TUNSETLINK` | Would change `dev->type`, only while the TAP is down. | **Prevented** by the launch filter (ADR-0143). It is harmless on an enslaved TAP in any case: the bridge vetoes `NETDEV_PRE_TYPE_CHANGE` with `NOTIFY_BAD` (`net/bridge/br.c:136-138`), and the TAP is enslaved at provision before the VMM runs. |
| `TUNSETDEBUG` | Sets the TAP's `msg_enable`. It is **not** a no-op: with its bits set, the kernel writes an unratelimited `netdev_info` line to the host kernel log for every ioctl on the queue and for every frame delivered toward the guest. | **Prevented** by the launch filter (ADR-0143). A change made anyway is detected by the debug-mask read-back → per-allocation damage → kill. |
| `TUNSETTXFILTER`, `TUNATTACHFILTER` / `TUNDETACHFILTER`, `TUNSETSTEERINGEBPF` / `TUNSETFILTEREBPF` | Install the TX filter and the classic and eBPF filters and steering applied to frames toward the holder's own guest; they can only reduce or drop what that guest receives. | **Prevented** by the launch filter (ADR-0143). Cloud Hypervisor does not use them on the `fd=` path. |
| `TUNSETQUEUE` | Attaches or detaches a multiqueue queue; returns `-EINVAL` on this single-queue TAP. | **Prevented** by the launch filter (ADR-0143). |
| `TUNSETOFFLOAD`, `TUNSETSNDBUF`, `TUNSETVNETHDRSZ` / `TUNSETVNETLE` / `TUNSETVNETBE`, `TUNSETNOCSUM` | Configure only the holder's own TAP and its queue: its offload features (the bridge recomputes the union of its ports' features, a performance effect only); its queue's send buffer; and its virtio-net header size and byte order. `TUNSETNOCSUM` is an unimplemented no-op. Cloud Hypervisor itself issues `TUNSETOFFLOAD` and `TUNSETVNETHDRSZ` on the `fd=` path. | Allowed. Harmless to every other guest: no attach, activation-gate, or cross-guest effect. Not in the audit read-back. |
| Read-only arms: `TUNGETFEATURES`, `TUNGETIFF`, `SIOCGIFHWADDR`, `TUNGETSNDBUF`, `TUNGETFILTER`, `TUNGETVNETHDRSZ` / `TUNGETVNETLE` / `TUNGETVNETBE` | Read the holder's own TAP state. | Allowed. No effect. |
| Refused arms: `TUNSETIFF`, `TUNSETIFINDEX`, `SIOCGSKNS`, `TUNGETDEVNETNS`, and every other request | On an attached single-queue TAP: `TUNSETIFF` returns `-EEXIST` (Cloud Hypervisor reissues it on the `fd=` path and accepts `EEXIST`), and `TUNSETIFINDEX` `-EPERM`; `SIOCGSKNS` and `TUNGETDEVNETNS` require `CAP_NET_ADMIN`. Every other request, `SIOCSIFFLAGS` and `SIOCSIFMTU` included, returns `-EINVAL` and is never forwarded to the netdev ioctl path. | Allowed by the filter; refused by the kernel. No effect. Administrative state therefore stays out of the holder's reach. |

The audit's TAP identity read-back includes the host-side MAC, the owner uid,
persistence, and the TAP's debug message mask (`msg_enable`, which must read
0). The filter prevents the holder from changing any of them; the read-back
detects a change made anyway, through a gap in the filter or by another
process. A change to any is treated as damage to that VM's network parts: the
VM is killed and its TAP is torn down (ADR-0124). The prevention is ADR-0143,
and the delivery-side structural control against the `SIOCSIFHWADDR` hazard is
ADR-0142; this decision owns only the uid-0 owner and the read-back set.

## Alternatives considered

### No owner

Rejected. The kernel applies no capability check to an ownerless TAP, so any
VMM uid could attach to any unattached guest TAP. That widens attach authority
rather than narrowing it.

### Keep the uid-4200 grant

Rejected. Every VMM runs as uid 4200, so the grant authorizes any VMM to attach
to any other guest's unattached TAP. The grant is unnecessary once the launcher
attaches the queue.

### Per-VM uids

Out of scope. An owner uid per VM (the crosvm model) would isolate TAPs from
each other but would still let each VMM attach its own TAP. It needs a larger
identity and confinement change than #295 requires, and the root owner already
excludes every VMM.

### A per-VM network namespace

Rejected by ADR-0114's topology: every guest TAP is a port on one host-netns
bridge.

## Consequences

Positive: the kernel refuses cross-guest queue attaches from every unprivileged
process that does not hold the TAP's queue. The creator keeps its existing
signature and only changes the uid it passes.

Negative:

- A process with `CAP_NET_ADMIN` or uid 0 can still attach; such a process is
  outside the threat model.
- The protection is not independent of the queue holder's confinement. The
  holder's TAP-mutating ioctls are prevented by ADR-0143's launch filter, and
  the table in the Decision gives each arm's disposition with it in force. The
  arms the filter allows configure only the holder's own TAP and queue, or are
  refused by the kernel. Owner-uid, persistence, host-side-MAC, and debug-mask
  changes made anyway are each detected by the audit read-back within one audit
  period and kill that VM.
- **No re-grant path remains.** `TUNSETOWNER` returns `EPERM` to every Cloud
  Hypervisor thread (ADR-0143), so an exited holder's TAP stays owned by uid 0
  until teardown, and no second uid-4200 process can attach it in that window.
  Cloud Hypervisor v53's own Landlock ruleset grants `/dev/net/tun` `rw`
  whenever any `--net` device is configured (research addendum A5), so this
  rests on the launch filter and the uid-0 owner, not on Landlock.
- The debug-mask read-back adds ethtool generic-netlink reads, because the mask
  is not in the link attributes the owner already observes: one dump per audit
  pass whatever the allocation count, and one single-interface read each at
  provision and at activation. A failed dump is a node-level audit failure
  that ADR-0124's bounded recovery retries, not damage to any one VM.
- The TAP identity expectation changes from uid 4200 to uid 0, which affects
  committed step `02-01` code.
- Native evidence must show that an attach attempt as the VMM uid gets
  `EPERM`, and that Cloud Hypervisor still reaches READY through the inherited
  descriptor on a TAP owned by uid 0.
