# ADR-0181 — `overdrive serve` refuses to boot when a root-namespace interface is stacked on another

## Status

**Accepted — decision IFACE-NO-STACK approved under the user's 2026-10-09
remediation directive** ("if design needs changing, then you remediate, and
then proceed to DISTILL"), answering the stacked-interface falsification of
validation item V-17 (feature delta P-44, `spike/findings-v17.md`). GH #295.
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN* (§ *Spike remediation decisions, round 2 — 2026-10-09*). Depends on
ADR-0165 (the egress unframe program attached at boot to every
root-namespace interface present). The image requirement it relies on is
ADR-0068 § 5.

## Context

ADR-0165 attaches one fragment-aware, tuple-gated egress program to `lo` and
to every root-namespace interface present at boot. The program removes the
frame from a guest datagram on a registered tuple and moves later fragments
back by one 8-byte unit.

V-17 attached it, as ADR-0165 directs, to a stacked pair — an upper device
(macvlan, VLAN, bridge) and the lower device it transmits through — on Lima
7.0.0-34 and metal 7.0.0-29. One skb then passed two attached egresses and
was unframed twice: a payload that itself parses as a frame lost 8 more bytes
(silent corruption), and a datagram of three or more fragments was shifted
twice and lost. With only the upper or only the lower attached, every case
was byte-exact. Independent interfaces under policy routing were byte-exact.

## Decision

- **No stacked root-namespace interface.** A root-namespace interface is
  *stacked* when it has a master (`IFLA_MASTER`: it is a port of a bridge,
  bond, team or VRF) or its link kind is one that transmits through a lower
  device in the same namespace (`vlan`, `macvlan`, `macvtap`, `ipvlan`,
  `ipvtap`). The appliance image configures none (ADR-0068 § 5).
- **Boot refuses one.** `serve`'s startup probe enumerates the root
  namespace's interfaces and, before any boot effect, refuses startup naming
  the interface and how it is stacked. So every interface the unframe
  program is attached to lies on a transmit path no other attached
  interface shares.
- **Not stacked:** a veth pair (its link is its peer, not a lower device),
  and tunnel or encrypting devices (`vxlan`, `geneve`, `gre`, `ipip`, `xfrm`,
  `macsec`, `wireguard`): whatever they transmit through their underlay
  leaves with an outer header that is never on a registered tuple, so the
  program passes it unchanged.

## Alternatives considered

- **Attach both upper and lower (as ADR-0165 first read).** Falsified by
  V-17. Rejected.
- **Attach only the leaf device of each transmit path.** Removes the double
  pass, but the program must then parse in-packet VLAN tags whenever the
  lower lacks tag offload, and macvlan bridge-mode traffic between two uppers
  of one lower never reaches the lower's egress. Neither was validated.
  Rejected for this design.
- **Make the program idempotent per skb.** A mark in `skb->cb` survived VLAN
  and macvlan but not a bridge, which clears it; `skb->tc_index` did not
  survive; `skb->mark` is owned by policy routing and nft. No uniform
  carrier. Rejected.
- **Key the fragment records per interface.** Removes the double shift of
  fragments but not the double strip of a frame-shaped payload. Rejected.

## Consequences

- An appliance whose root namespace carries a bonded, bridged, VLAN or
  macvlan interface refuses to start. Bonded NICs are common on bare metal;
  supporting them needs a different attachment rule, validated first.
- A dev or metal host with such an interface (for example a bridge with
  enslaved ports) is refused the same way, so Tier-3 hosts must carry none.
- One more boot refusal with a typed cause, checked before any boot effect.
