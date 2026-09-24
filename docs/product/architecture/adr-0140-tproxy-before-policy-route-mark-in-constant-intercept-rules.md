# ADR-0140 — Order TPROXY before the policy-route mark in the constant intercept rules

## Status

**Accepted (2026-09-24), conditional on reproduction.** GH #295
correctness-recovery replacement DESIGN, decision D-295-R19. Proposed
2026-09-24; reviewed by independent DESIGN review rounds 4 and 5 and the
round-5 verification (`arch_rev_20260924_netns295_r5_verify`); accepted by the
user on 2026-09-24 with the replacement DESIGN. Conditional on reproduction:
the hazard is reasoned from production source and kernel source. DISTILL must
reproduce it on native metal (RED) before DELIVER relies on this decision. If
native evidence shows that outbound guest TCP to an absent transparent listener
is already dropped under the current order, this decision is withdrawn. It
changes the order of expressions inside two of the eight constant rules that
ADR-0125 records. It changes no rule count, set, port, or ownership, and it
does not amend ADR-0125's decision; ADR-0125 carries a pointer to it.
Exact rule and helper details live only in the #295 feature delta.

## Context

ADR-0125's constant program has two TPROXY rules:

- The outbound rule sends TCX-intercepted guest TCP from a registered source to
  the node's leg-F listener.
- The inbound rule sends TCP for a registered destination to leg C.

Both share one tail today: set the policy-route mark `0x1`, then `tproxy`, then
`accept`. The `fwmark 0x1 lookup 100` rule and table 100's
`local 0.0.0.0/0 dev lo` route make every `0x1`-marked packet local. The order
came from the #222 amendment of 2026-08-31 (ADR-0088, ADR-0089), which reasoned
that a surviving mark keeps a flow on the host when its listener is gone.

Kernel source settles what happens when no transparent listener matches. In
`net/netfilter/nft_tproxy.c` (`nft_tproxy_eval_v4`), a failed socket lookup, or
a socket that is not transparent, sets the verdict to `NFT_BREAK`. That ends the
rule without a verdict, and any mark the rule already set survives. The kernel
TPROXY document does not describe this case. The iptables target
(`net/netfilter/xt_TPROXY.c`) differs: with no transparent socket it returns
`NF_DROP`.

The two TPROXY rules are not equally exposed:

- **Outbound.** The next rule drops TCP still carrying the TCX intercept mark
  `0x295a`. With the mark already rewritten to `0x1`, it no longer matches. If
  the packet's destination is outside every managed and registered set (the
  bridge gateway, another host address, or an external address), no later rule
  matches either. The `0x1` policy route then delivers the packet to any host
  listener bound to the wildcard address on its destination port, because
  table 100 makes every address local.
- **Inbound.** Already fail-closed. The rule after it drops every TCP packet
  whose destination is a managed guest, without testing the mark, and every
  registered inbound destination is a managed guest address.

The listener-absent state is reachable in production: leg-F loss (which
ADR-0124 deliberately does not answer with TAP quiescence), a crashed
`overdrive serve` whose microVMs survive, and a fail-stop whose shutdown is
abandoned at its bound. Keeping such a flow on the host is not failing closed.
(Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 9.1 and 9.5; the `NFT_BREAK` behaviour is from kernel source, which the
research lists as a gap in the documentation.)

## Decision

Both TPROXY rules order their tail as `tproxy`, then the policy-route mark,
then `accept`. When no transparent listener matches, `NFT_BREAK` ends the rule
before the mark is set. An outbound packet therefore keeps `0x295a` and falls
through to the unhandled-intercept drop. The healthy path is unchanged: TPROXY
succeeds, the mark is set, and the packet is accepted. The inbound rule shares
the tail helper and changes with it; nothing inbound depends on the change.

This is what allows ADR-0124 to leave leg-F and leg-C loss without TAP
quiescence. If this decision is withdrawn and native evidence does not show the
outbound path already failing closed, listener loss must quiesce TAPs instead.

## Alternatives considered

### Keep the mark before TPROXY

Rejected. With no transparent listener the mark survives, the
unhandled-intercept drop no longer matches, and outbound guest TCP reaches any
host wildcard listener.

### Quiesce managed TAPs on every listener loss

Not chosen while this decision stands. It closes the runtime path, but it takes
every guest off the network for a listener repair that the ordering makes
unnecessary, and it does nothing for a crashed process, where no supervisor
runs to quiesce anything. It is the stated fallback if this decision is
withdrawn, or if the `TIME_WAIT` case below reproduces.

### Rely on the independent intercept-mark guard table (ADR-0139)

Rejected as a substitute. That guard drops TCP still marked `0x295a`, and with
today's order the listener-absent packet carries `0x1`.

## Consequences

Positive: outbound guest TCP fails closed when its transparent listener is
absent, including after a crash that leaves microVMs running. The rule count,
sets, ports, and ownership are unchanged.

Negative:

- The canonical program identity changes. A host whose kernel still holds the
  old order refuses fresh boot on it as a schema-conflicting owned table. No
  released build ever produced that order, so only development or test hosts
  can hold it; they clear the stale table once. No compatibility recognizer is
  added.
- The ordering does not cover one path, reasoned from kernel source and not
  executed. A SYN that matches a transparent `TIME_WAIT` socket left by an
  earlier leg-F connection is assigned to that socket while leg F is absent, so
  TPROXY succeeds and local delivery can reach a wildcard listener on the
  packet's own destination port. The #295 feature delta makes this a separate
  native RED. If it reproduces, listener loss becomes a TAP-quiescing
  component, and the remaining crash-window exposure goes to the user.
- Correctness rests on native evidence, because no simulation can observe
  kernel TPROXY and routing.
