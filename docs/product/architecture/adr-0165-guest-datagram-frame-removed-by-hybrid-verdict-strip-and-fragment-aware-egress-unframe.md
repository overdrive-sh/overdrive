# ADR-0165 — The host removes the guest datagram frame in its verdict for every non-empty datagram, and a fragment-aware, tuple-gated egress program on every host interface removes the frames the verdict keeps

## Status

**Accepted — approved by user (D5a = hybrid 2026-10-06; its attachment set,
current wording D5a-IFACE, 2026-10-08; the boot refusal on any attach
failure, D5a-IFACE-BOOT, and no attach after boot because the appliance
configures no interface registered after boot, D5a-LATE, 2026-10-08; that
image requirement recorded in ADR-0068 § 5, IMG-IFACE-BOOT, and `serve`'s own
boot-created interfaces ordered before the attach, D5a-SERVE-IFACE,
2026-10-08); independent review r12 answered; further review waived by user
2026-10-08; handed to DISTILL.** GH #295. Split out of ADR-0149 (frame format, D5) so that each ADR
records one decision. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*, which holds the
forwarder contract.

## Context

ADR-0149 frames every guest datagram inside the transport. Guest→host
datagrams leave the host toward host-local and off-host destinations, so the
frame must be removed somewhere in the host kernel.

The off-host spike (`spike/v11-vip-v14-findings.md`; qualified metal;
destinations behind veth at MTU 1,500 and through the physical NIC) compared
removal points:

- **Tuple-gated unframe on egress TC only.** Works up to the egress MTU. Above
  it the datagram is lost whole: IP fragmentation runs before TC egress, the
  first fragment fails the program's length check and is dropped, and
  `ip_do_fragment()` then stops sending the remaining fragments. A NIC capture
  of a 4,096-byte datagram showed no packet at all.
- **Fragment-aware egress TC only.** Works for every size (first fragment
  shortened by 8 bytes; later fragments' offsets moved back by one 8-byte
  unit; wire capture confirmed). If an egress interface lacks the program,
  every datagram through it reaches the peer 8 bytes too long.
- **Removal in the host verdict only.** Works for every non-empty size with no
  TC. An empty datagram cannot be redirected: a zero-length redirect makes the
  psock disable TX, after which every later datagram of the association is
  lost. Empties must therefore be dropped and counted.
- **Hybrid.** The verdict removes the frame from every non-empty datagram and
  keeps it only for an empty datagram or a payload that itself parses as a
  frame; a fragment-aware TC on each egress interface removes a valid frame on
  a registered tuple and passes everything else. All of 0, 1, 1,431, 4,096,
  4,097 and 59,000 bytes passed, connected and unconnected; frame-shaped
  payloads arrived byte-exact; a non-owner frame-shaped datagram was
  untouched; unregistered traffic cost about 270 ns per packet. On an
  interface lacking the program only empty and frame-shaped datagrams arrived
  in their 8-byte-longer form.
- **Tuple identity.** A tuple registered from the requested destination
  missed every datagram whose destination `connect4` rewrote (a service VIP,
  ADR-0164): the frame reached the backend. A tuple registered from the
  connected host socket's kernel peer was correct.

## Decision

- **Verdict strip.** The host verdict removes the frame from every non-empty
  guest→host datagram. It keeps the frame only when the datagram is empty, or
  when the payload itself parses as a valid frame (the escape rule, so a
  payload is never mistaken for a frame).
- **Egress unframe.** A fragment-aware, tuple-gated TC egress program on host
  `lo` and on every interface of the host's root network namespace removes a
  valid frame from a datagram on a registered tuple — including from the first
  fragment of a fragmented datagram, with the later fragments' offsets moved
  back to match — and passes every other packet unchanged. It never drops for
  bad magic.
- **Tuples come from the kernel.** A tuple is registered from the connected
  host socket's kernel-reported local address and peer (after any `connect4`
  rewrite), never from the requested destination.
- **Attachment set.** An interface carries guest datagrams only once it has
  addresses and routes. Root-namespace interfaces are configured by two
  producers, both before the attach: the appliance image at boot, before
  `serve` starts (ADR-0068 § 5), and `serve`'s own boot steps before the
  attach step (ADR-0061's load-balancer veth pair). So:
  - **at boot, the program is attached to every interface present** — `lo`,
    the image's NICs and every interface `serve`'s earlier boot steps
    created. Any failed attachment refuses startup with the interface named,
    before forwarding opens;
  - **after boot, nothing is attached.** An interface the kernel registers
    later (a NIC whose driver probes late, a hot-added NIC, or one any
    component creates) is never given addresses or routes, so it carries no
    guest traffic and needs no attach. For the image this is ADR-0068 § 5;
    for `serve`, no step after the attach creates or configures a
    root-namespace interface, and a future runtime owner of such an
    interface must attach the program before giving it addresses or routes.
    Neither is a runtime check. The dev and metal hosts run a catch-all
    systemd-networkd configuration and are not the appliance image.

  No attach outcome quiesces forwarding or counts as audit damage. The links
  are process-owned and not audited at runtime (ADR-0159). A TCX link of an
  interface the kernel removes goes with the interface.

## Alternatives considered

- **Tuple-gated unframe on egress TC only, not fragment-aware.** Loses every
  datagram above the egress MTU. Rejected.
- **Fragment-aware egress TC only.** A missing or late attachment silently
  corrupts every datagram through that interface. Rejected in favour of hybrid,
  which confines that failure to empty and frame-shaped datagrams.
- **Removal in the host verdict only, empties dropped.** Drops every empty
  datagram (a D15 regression). Rejected.
- **Boot proceeds with a NIC that failed to attach, counted and retried.**
  Leaves empty and frame-shaped datagrams through that NIC reaching their
  peers framed for as long as the attach keeps failing, an exposure a boot
  precondition removes. Rejected.
- **Attach an interface registered after boot on its link notification,
  configured independently, retrying failures.** Leaves a window from
  registration to attach — unbounded while the attach keeps failing, and
  retried even when the failure is a defect — in which empty and
  frame-shaped datagrams leave through it framed. Rejected.
- **Attach an interface registered after boot as a precondition of
  configuring it, through a late-interface configuration path.** Closes the
  window, but adds an attach operation, a failure classification, a retry
  and a lifecycle gate for a configuration the appliance never performs.
  Rejected in favour of never configuring such an interface.

## Consequences

- Non-empty datagrams leave the host independent of routing, interface set and
  MTU; only empty and frame-shaped datagrams depend on the egress program.
- Every interface that can carry a guest datagram carries the program from
  boot; no runtime exposure remains, and the forwarder has no attach
  operation after boot.
- An interface registered after boot is unusable for guest traffic on the
  appliance. A design that needs one configured must revisit this decision.
- On a host whose network configuration configures interfaces registered
  after boot (the dev and metal hosts), empty and frame-shaped datagrams
  leaving through such an interface reach their peers framed. That host is
  not the appliance.
- The host carries one TC egress program per root-namespace interface plus a
  fragment-tracking map; it is a no-op for unregistered tuples.
- Untested, and therefore validation items in the feature delta: NICs with UDP
  segmentation offload, several egress interfaces with policy routing, and a
  frame-shaped payload above the MTU.
