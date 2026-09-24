# ADR-0142 — A guest TAP egress classifier delivers host-originated frames only to that TAP's registered guest MAC

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R21. Proposed 2026-09-24 from review finding R5-H1, revised the
same day by the round-5 verification (`arch_rev_20260924_netns295_r5_verify`,
defects D2, D4, D5, D6), and accepted by the user on 2026-09-24 with the
replacement DESIGN. Depends on ADR-0114 (one host-netns bridge), ADR-0115 (the
TCX endpoint classifier and endpoint map), ADR-0126 (the fixed bridge MAC), and
ADR-0130 (uid-0 TAP owner). The control is structural. It complements ADR-0143,
whose launch seccomp filter prevents the VMM's `SIOCSIFHWADDR` at its source.
Exact program, map, and rule details live only in the #295 feature delta.

## Context

Every guest TAP is a port of one node-local Linux bridge (ADR-0114), and each
Cloud Hypervisor process holds its own TAP's queue descriptor as uid 4200
without `CAP_NET_ADMIN` (ADR-0130). The kernel applies no capability or owner
check to `SIOCSIFHWADDR` on an attached tun queue (`drivers/net/tun.c`
`__tun_chr_ioctl`, arm at `3385-3394`; `net/core/dev.c` `netif_set_mac_address`;
the TAP carries `IFF_LIVE_ADDR_CHANGE` so the change applies while it is up).
Setting the TAP's host-side MAC to another guest's virtio-net MAC drives
`br_fdb_changeaddr` → `fdb_add_local`, which deletes the victim's learned entry
(and any user `static`/`sticky` pin) and installs a `LOCAL|STATIC` entry for
that MAC on the attacker's port (`net/bridge/br_fdb.c:430-458`, `460-504`). The
bridge transmit path then forwards host-originated unicast for that MAC out of
the attacker's port without testing `BR_FDB_LOCAL`
(`net/bridge/br_device.c:109-112`), and the entry resists re-learning
(`br_fdb.c:985-988`). Every such frame travels host→guest (TAP egress), which
the ingress-only endpoint classifier of ADR-0115 never sees
(`__netif_receive_skb_core` runs TCX ingress on frames *from* the guest;
`__dev_queue_xmit` runs TCX egress on frames *to* it).

The hazard is reproduced natively. On kernel `7.0.0-29` an attacker running as
uid 4200 with no effective capabilities, holding only its own TAP's queue
descriptor, moved the victim's MAC to its own port as `LOCAL|STATIC` and read
the victim's host-to-guest frames from that descriptor; host unknown-unicast to
a never-learned MAC also reached it. A destination-MAC gate at the nft bridge
`output` hook (`NF_BR_LOCAL_OUT`), delivering only each TAP's registered guest
MAC, blocked both while broadcast was still delivered, and dropped every flooded
copy at each non-target port. Disabling unicast flood on a port closed the flood
variant independently. The gate did not revert the stuck FDB entry. The explicit
bridge MAC (ADR-0126) held. (Spike increment-z:
`docs/feature/netns-density-295/spike/findings-mac-fdb-isolation.md`.)

Two adjacent facts bound the problem:

- The attacker must target the victim's *guest* (virtio-net) MAC, not the
  victim TAP's host-side MAC. Guest MACs are a deterministic function of the
  guest IPv4 (`0x02:0x00` followed by the four address octets,
  `guest_network.rs`), so the target MAC is derivable from the address and does
  not need to be learned first.
- The same delivery path also floods host-originated unknown-unicast to every
  flood-enabled port (`br_forward.c:216-218`), leaking host→guest frames to
  other guests during the windows when the victim's entry is absent (after
  ageing, before its first frame, or just after its entry is removed).

Cloud Hypervisor v53's own seccomp does not block the ioctl: the VMM thread's
filter allows `SIOCSIFHWADDR` keyed on the request number only, and the main
thread is unfiltered. ADR-0143's launch filter, installed before Cloud
Hypervisor runs, does block it on every thread; this decision does not depend
on that filter. Prior art that is not steered by a port's learned MAC
delivers from a registered control-plane record: Cilium writes the destination
MAC and interface from the endpoint record (`bpf/lib/local_delivery.h`,
`bpf/lib/l3.h`), and Neutron's OVS firewall selects the output port by
`dl_dst=<registered MAC>`. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Addendum 2 B1, B3, B4.)

## Decision

Each managed guest TAP carries a TCX egress classifier that delivers a unicast
frame to the guest only when the frame's destination MAC is that TAP's
registered guest MAC; every other unicast frame is dropped. A unicast frame on
a TAP whose ifindex has no endpoint entry is dropped too: the classifier fails
closed on an endpoint-map miss. Broadcast and multicast are always delivered,
with or without an endpoint entry, so ARP and neighbour discovery are
unaffected. The registered guest MAC is the source-MAC value the ADR-0115
endpoint map already holds, keyed by the TAP's ifindex; no new map or record is
introduced. Delivery is therefore steered by the registered record, not by the
bridge's learned forwarding database, so a host-side TAP MAC change cannot
redirect another guest's host-to-guest traffic to the attacker. The same check
drops flooded unknown-unicast at every TAP except its registered target, so
bridge unicast flooding stays enabled on managed ports.

## Alternatives considered

### An nft bridge-family output/forward rule keyed on `oifname` + `ether daddr`

Rejected as the mechanism, on ownership grounds. It observes the same frame at
`NF_BR_LOCAL_OUT` and enforces the same registered-MAC delivery; it is the form
the native reproduction proved. It needs a per-TAP rule or a TAP→MAC mapping
expressed in nft, a second owner and codec beside the endpoint map the
classifier already keys on, whereas the TCX egress program reuses the endpoint
map and the aya TCX mechanism the design already owns per TAP.

### Also disable bridge unicast flooding on every managed port (`flood off`)

Rejected on evidence. It closes the unknown-unicast leak at the bridge, as the
native run showed, but the egress classifier already drops every flooded copy
at each non-target TAP, so it adds no protection. It does nothing against the
directed steal, which uses a known FDB entry. It costs delivery: with flooding
off, host unicast to a guest whose MAC has no FDB entry (after ageing, before
its first frame, or after teardown removes a poisoned entry) reaches no port,
not even its legitimate target, until that guest transmits. With flooding on,
the target's own classifier admits the flooded copy and delivery never waits on
learning.

### Detection only — audit the TAP's host-side MAC and kill on a change

Not chosen as the closure. Setting the TAP down after detection leaves the
stolen `LOCAL|STATIC` entry in place (`br_fdb.c:885-890` skips static entries),
and there is an unavoidable leak window between the change and the next audit.
Detection is retained as a *complement* to this structural control (ADR-0130's
audit reads back the host-side MAC and treats a change as per-allocation damage,
so the affected VM is killed and teardown removes its port, which clears the
poisoned entry), but detection is not the control that closes the theft.

### Prevent the poisoning at its source instead of controlling delivery

Rejected as a replacement for this control, and adopted beside it. ADR-0143's
launch seccomp filter denies `SIOCSIFHWADDR` to every Cloud Hypervisor thread,
so the VMM cannot poison the forwarding database at all. It does not make this
control unnecessary:

- The unknown-unicast flood leak needs no ioctl. Host unicast to a guest whose
  MAC has no FDB entry is flooded to every port, and only a per-TAP delivery
  check keeps it from the other guests.
- The filter is pinned to a Cloud Hypervisor version, a launch shape, and a
  syscall ABI (ADR-0143 Consequences). A MAC change made through a gap in the
  filter, or by another process, would otherwise steal delivery again.

A BPF-LSM `file_ioctl` hook refusing `SIOCSIFHWADDR` on the tun device is
rejected in ADR-0143: it is a node-global mandatory-access-control policy over
every process's ioctls and adds nothing the launch filter lacks.

### Pin the victim MAC with a `static`/`sticky` or `permanent` FDB entry

Rejected. `static`/`sticky` entries are `NUD_NOARP`, which clears
`BR_FDB_LOCAL`, so `fdb_add_local` deletes them (`br_fdb.c:443-447`,
`1211-1214`). A `permanent` (local) pin survives in source but carries
guest-to-guest termination on the host, a rate-limited per-frame kernel warning,
and incompatibility with `locked` (`br_fdb.c:985-988`; libvirt deliberately
keeps the TAP MAC different from the guest MAC for these reasons), and it relies
on transmit behaviour the `bridge(8)` man page contradicts.

## Consequences

Positive: a compromised VMM changing its TAP's host-side MAC can no longer
receive another guest's host-originated plaintext. Delivery is decided by the
registered endpoint record, not the learned FDB, which is the Cilium/Neutron
shape. The same check closes the unknown-unicast flood leak without disabling
flooding, so delivery to a guest whose MAC is not yet learned is never delayed.
The endpoint-map-miss verdict fails closed, so a TAP whose entry is absent (for
example during teardown) never delivers unicast unchecked. The control reuses
the ADR-0115 endpoint map and adds no new record.

Negative:

- Each managed TAP gains a second TCX program (egress) beside its ingress
  classifier, with its own verifier budget and one drop-counter class in the
  shared counter map.
- **The check stops a theft but does not revert a poisoned entry.** ADR-0143
  prevents the VMM's `SIOCSIFHWADDR`, so a poisoned entry can arise only through
  a gap in that filter, or from a MAC change made by another process, which
  needs `CAP_NET_ADMIN` (only Cloud Hypervisor holds a queue) and is outside the
  threat model. If one arises, host
  unicast to the victim is dropped at the changed TAP until that TAP is torn
  down, so the victim receives none. The bound: the change is detected within
  one audit period (ADR-0130 read-back); only the changed TAP's VM is killed
  (ADR-0124); its ordinary lifecycle cleanup, within about one restart-backoff
  window plus the cleanup itself (retried at a one-second cadence on failure),
  deletes the TAP, which removes the port and the poisoned entry; the next host
  unicast to the victim is then flooded and admitted only by the victim's own
  classifier, and the bridge re-learns the victim's MAC from its next frame.
- **Evidence.** The mechanism, and the gate in its nft form, are proven
  natively (increment-z). The chosen TCX form carries the feature delta's
  native case (E12 (h)), which also proves the kill, teardown, and re-learn
  restoration. That case changes the MAC from a test process holding a copy of
  the queue, which is not launched through the VMM adapter and so runs outside
  ADR-0143's filter; it models a change the filter does not see. The natively
  proven nft form is the alternative rejected above on ownership grounds.
