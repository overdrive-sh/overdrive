# ADR-0144 — A managed link's identity is independent of the host's link configuration

## Status

**Accepted. Not yet implemented.** GH #295. Accepted 2026-09-26 on the
evidence of root cause A of the fresh-host RCA
(`docs/analysis/root-cause-analysis-netns295-fresh-bridge-boot-refusal.md`).
The user's rulings of 2026-09-28 fix the decision recorded here (feature delta,
§ *User rulings of 2026-09-28*). The decision rests on:

- **The user's rulings of 2026-09-28.** Every managed link must be correct
  whether or not systemd-udevd runs, and whatever its policy. #295 adds no
  dependence on udev or on the host's configuration. Removing the node
  runtime's dependence on systemd in general is
  [GH #304](https://github.com/overdrive-sh/overdrive/issues/304).
- **The user's ruling that technical decisions for #295 are settled on
  evidence.** The evidence is:
  - the RCA's probes on the Lima VM (kernel `7.0.0-31`, systemd 259), with
    the upstream kernel v7.0 and systemd v259 sources the RCA cites;
  - the bridge forwarding-database read in research addendum 2
    (`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
    B1), which used a local kernel 7.2 tree;
  - the native MAC-steal reproduction, spike increment-z
    (`docs/feature/netns-density-295/spike/findings-mac-fdb-isolation.md`).

It depends on:

- ADR-0126, the fixed bridge MAC;
- ADR-0130, the audit read-back set;
- ADR-0142, TAP egress delivery only to the registered guest MAC;
- ADR-0143, which denies Cloud Hypervisor `SIOCSIFHWADDR`.

The exact contract lives only in the #295 feature delta
(`docs/feature/netns-density-295/feature-delta.md`). § *Managed-link identity
independent of host link configuration* holds the bridge-creation operation.
§ *Driven port — TAP egress guest-MAC delivery (D-295-R21)* holds the reserved
set, the fact a violation reports, and the residual.

## Context

The #295 design creates four kinds of managed host link: the node bridge
`ovd-gbr0`, the startup probe's scratch bridge `ovd-gbr-probe`, every guest
TAP `ovd-tp-<4hex>` (named by ADR-0118), and the probe's scratch TAP
`ovd-tp-probe`. It reads back the address of the node bridge and of every
guest TAP:

- The bridge carries the fixed `02:01:00:00:00:01` (ADR-0126). A mismatch at
  boot refuses startup. At runtime the audit repairs it under quiescence.
- Provision, activation, and the audit read back each TAP's host-side MAC
  (ADR-0130). A TAP that holds another guest's MAC poisons the bridge
  forwarding database, and can take that guest's host-originated traffic
  (ADR-0142, reproduced natively in increment-z). The read-back is therefore a
  tamper detector. A TAP that fails it is per-allocation damage, and only its
  VM is killed (ADR-0124).

The host runs other writers of link addresses. systemd-udevd applies the first
matching `.link` file to every new link, and the default `99-default.link`
matches every name, with `MACAddressPolicy=persistent`. When udev processes a
link's add event, it reads `addr_assign_type`:

- if the kernel assigned the address at random, udev replaces it with a MAC
  derived from the machine and the link name;
- if userspace set the address, udev's address policy leaves it alone (systemd
  v259 `src/udev/net/link-config.c:605-606`).

The fresh-host RCA traced a boot refusal to this writer:

- The owner created `ovd-gbr0` without an address and then set it. The
  kernel registers a bridge with a random address and emits its add event
  inside `register_netdevice`, before the creating `RTM_NEWLINK` is
  acknowledged, so udev read "random" before the owner's set.
- On the Lima VM, udev's write landed 0.19 ms after the owner's set, and the
  boot read-back refused. This happened in 6 of the 9 fresh-host boots that
  reached the check. A boot that adopts an existing bridge gets no add event
  and passes.
- A bridge created with its address in the same `RTM_NEWLINK` is marked as set
  by userspace before registration (`net/core/rtnetlink.c:3698-3701`). Its add
  event never shows a random address. The RCA read it so at the first
  observation in 10 of 10 trials, and every one kept its address. Adding and
  removing ports does not change such a bridge's address
  (`net/bridge/br_stp_if.c:263-265` at v7.0, as the RCA cites; `:269-270` in
  the 7.2 tree).

TAPs cannot be created that way. The tun driver assigns a random address at
creation and refuses creation over rtnetlink (`drivers/net/tun.c:1332`,
`:2283`), and udev rewrote the probe's scratch TAP 3.6 ms after creation. A
read-back that compares a TAP's address with the value seen at provision
therefore reads a udev rewrite landing after that reading as tampering. It
would kill a healthy VM.

A host-side fix works where it is installed. A `.link` file exempting managed
links kept a new TAP at its kernel address (RCA P9). But it makes correctness
depend on the host's configuration. Every host, the appliance image, and each
test substrate would have to carry and prove the file, and a host without it
would refuse boot or kill healthy VMs. The user ruled that #295 must not
depend on udev or on host configuration.

What a TAP's host-side MAC can do is narrow. It matters only through the
bridge forwarding database. The bridge installs a local entry for a port's own
address at enslavement and at every address change, and that entry does two
things (research addendum 2 B1):

- it sends host-originated unicast for that address out of the port;
- it passes unicast for that address, bridged in from other ports, up to the
  host.

An address that is no guest's MAC and not the bridge's steers no frame. The
danger is therefore a small, known set of addresses, not change as such.

## Decision

A managed link's correctness does not depend on the host's link
configuration. It holds whether or not systemd-udevd runs, and whatever its
policy. Each link kind realises that one decision as follows.

- **A managed bridge is created with its address.** The shared owner creates
  the node bridge, and the startup probe creates its scratch bridge, with the
  fixed address in the `RTM_NEWLINK` that creates the link, administratively
  down. udev's address policy therefore leaves it alone, whatever that policy
  is. A bridge that already exists is adopted without a write and then
  converged, as ADR-0126 specifies. Any other writer is caught by the boot
  read-back, which names the observed address and up state, and repaired by
  the runtime audit.
- **A TAP's host-side MAC is judged by an invariant, not by a recorded
  value.** The address a TAP reads back must not be a reserved address. The
  reserved addresses are the bridge MAC and the guest MAC of every allocation
  the owner holds outside `Condemned`, the TAP's own included. Provision,
  activation, and every audit pass check it, and nothing records a TAP's
  address. A TAP that breaks the invariant is refused at provision, fails
  activation, or is per-allocation damage that kills its VM alone. Any other
  address is correct, whether the kernel, udev, or another link manager
  chose it.
- **No host requirement and no host probe.** Nothing installs, reads, or
  requires a host link-configuration file or udev's device database. The
  startup probe does not read its scratch TAP's address. A narrow `.link` file
  exempting managed links may ship as optional image hygiene. That is the
  Image Factory's choice
  ([GH #75](https://github.com/overdrive-sh/overdrive/issues/75)), and no
  contract depends on it.

## Alternatives considered

### Require a host `.link` file exempting managed links

Every host that runs systemd-udevd would carry an Overdrive `.link` file
(`OriginalName=ovd-gbr* ovd-tp-*`, `MACAddressPolicy=none`), proved at
installation, so that a TAP keeps its creation-time address and a recorded
value stays valid. Rejected. It makes boot and workload correctness depend on
the host's configuration, which the user ruled out:

- every host, the appliance image, and each test substrate would have to
  carry and prove the file;
- a host without it would refuse boot, or kill healthy VMs at random;
- another link manager matching the managed names would still break it.

The file is kept only as optional hygiene.

### Compare a TAP's host-side MAC with the value recorded at provision

Rejected. A TAP cannot be created with its address, and udev rewrites it a few
to tens of milliseconds after creation (3.6 ms at info level and 28–31 ms at
debug level in the RCA). Without a host exemption, a rewrite that lands after
the recording reads as tampering and kills a healthy VM. The recorded value is
also not the property that matters. An unreserved rewrite is harmless, and a
reserved address is dangerous whether or not it changed.

### Owner-assigned TAP addresses

The owner would set each TAP's address and record the value it set. Rejected.
The owner's write lands after the add event, inside the window udev races, so
it needs the same host exemption. Bounding the race needs udev's completion
signal, which the runtime does not read.

### A repair or reconcile loop, or a settle delay

The owner would re-read and re-set until the address is stable, wait a settle
delay, or leave the rewrite to the runtime audit. Rejected:

- the writer is asynchronous and gives no completion signal, so a loop cannot
  know it has won;
- udev's own latency can exceed any settle delay;
- the deterministic signal, udev's device database, needs a udev dependency or
  a `udevadm` shell-out of the kind ADR-0085 removed;
- leaving a creation-time bridge write to the audit would turn every fresh
  boot on a default host into a recovery.

### A startup-probe refusal when the scratch TAP's address changes

Rejected. Under the invariant a rewrite is harmless, so the refusal would
refuse a correct host.

### Reserve only other guests' MACs

Rejected. A TAP that holds its own guest's MAC steals nothing. It does break
that guest's bridged reception: unicast addressed to the guest from another
port, such as a peer's ARP reply, is passed up to the host instead. Keeping it
out of the set would also need a per-TAP exception in the check.

### Reserve `Condemned` allocations' guest MACs too

Rejected. A `Condemned` allocation's VMM has been killed, so a TAP holding its
guest MAC harms no live guest. Reserving it would kill a second VM for no
protection. Its address is reassigned only after its teardown, because the
lease is released last. The next audit then reserves the MAC for its new
holder.

### A global `MACAddressPolicy=none`, or a masked `99-default.link`

Rejected. It changes the naming and address policy of every host interface,
the physical NICs included, and drops their predictable names.

## Consequences

Positive:

- A fresh-host boot no longer refuses because udev rewrote the bridge. On any
  host, udev's address policy leaves the bridge and the scratch bridge at the
  fixed address from registration.
- A host link manager's rewrite of a TAP is harmless. No host, image, or test
  substrate carries a requirement for #295, and #295 adds no systemd
  dependence to the node.
- The TAP check tests the property the MAC steal needs. A hijack that moves
  another guest's MAC onto a TAP breaks the invariant, whatever made the
  change. Meanwhile ADR-0142's egress check keeps the stolen frames from the
  attacker's guest.
- No dependency, crate, subprocess, or daemon is added. The runtime never
  reads udev.

Negative:

- **A random TAP address can collide with a reserved one.** For one uniform
  46-bit locally administered address, the chance is about 2 × 10^-10 at the
  placeholder cap, and under 10^-9 against the whole /16, which bounds a TAP's
  lifetime as allocations come and go. On a host running udev, a TAP carries
  the kernel's address and then udev's, so its chance is at most twice that.
  The consequence is fail-safe: the provision is refused, the activation
  fails, or that one VM is killed. The guest whose MAC collided, if it is
  live, receives no host unicast until the colliding TAP is torn down, within
  one audit period plus that TAP's cleanup.
- **udev's draw does not repeat.** udev's persistent address is a fixed
  function of the machine and the link name, and a TAP's name derives from its
  allocation's address. On a host running that policy, a colliding pair of
  addresses is therefore permanent, and the kill recurs whenever both are
  held. The chance that a given host has any such pair is at most about
  6 × 10^-5. The optional `.link` file restores a new draw at each creation.
- **The check does not detect a change to an unreserved address.** Such a
  change steers no frame, so nothing depends on detecting it. An owner-uid,
  persistence, or debug-mask change is still detected (ADR-0130).
- **Detection can wait for the victim.** A reserved address set on a TAP
  before its victim allocation is held is caught by the first audit after the
  victim is provisioned, not at the victim's own provision. The victim
  receives no host unicast until the offending TAP is torn down after that
  audit, and no frame reaches the attacker's guest (ADR-0142).
- **The kernel evidence is not yet on the pinned kernel.** The bridge-creation
  behaviour is cited at upstream v7.0 and was observed on the Lima kernel
  `7.0.0-31`. The forwarding-database behaviour is read from the 7.2 tree of
  research addendum 2. The pinned 6.18 appliance kernel (ADR-0068) has not
  exercised either.
- ADR-0068 is not amended. The appliance image carries no requirement for this
  decision.
