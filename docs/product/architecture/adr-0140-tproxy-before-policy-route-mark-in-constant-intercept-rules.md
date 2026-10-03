# ADR-0140 — Order TPROXY before the policy-route mark in the constant intercept rules

## Status

**Withdrawn (2026-10-03) under this ADR's own accepted native condition.** The
conditional acceptance recorded below carried the self-withdrawal clause *"If
native evidence shows that outbound guest TCP to an absent transparent listener
is already dropped under the current order, this decision is withdrawn."* That
condition is met: physical-native E14(c) (external leg-F destruction) and
E14(d) (killed `serve` owner, guest alive) reproduce the fail-closed outcome
under the existing mark-before-TPROXY order — valid positive controls, live
guest SYNs, unchanged program/members and policy route, complete captures, and
zero correlated SYN-ACKs or wildcard accepts. The reorder is therefore
withdrawn and is never adopted; the constant program's two TPROXY rules keep
their existing tail (the policy-route mark `0x1`, then `tproxy`, then `accept`).

The listener-loss classification that rested on this ADR is resolved on native
evidence rather than on the reorder. A pure leg-F or leg-C listener failure
does not quiesce managed TAPs, because the ordinary absent-listener outbound
path already fails closed (E14(c)/(d)) and the `TIME_WAIT` side door fails
closed on the 2026-10-03 named-Service E14(e) result (a true `TIME_WAIT` on the
original Service-VIP destination tuple, `serve` owner killed, guest TAP up and
the entry surviving: crafted newer-sequence guest reconnects answered with no
SYN-ACK, `reopened=false`, host wildcard listener `accepts=0`). The accepted
single-loss fail-closed security outcome — no SYN-ACK, no wildcard accept, no
reopen to the user — is preserved; nothing here relaxes it. This is recorded on
that bounded native evidence and is not extrapolated to untested kernels or
host configurations. Exact rule, helper, and evidence detail live only in the
#295 feature delta (§ *Intercept-mark fail-closure* and its *Native
falsification register*). ADR-0088, ADR-0089, ADR-0124, ADR-0125, and the
architecture brief record the same withdrawal through their pointers. The
independent intercept-mark guard (ADR-0139, D-295-R18) is unaffected: its
IP-program-loss exposure independently reproduces and its decision stands.

Roadmap revalidation of the #295 08-01 step and DELIVER resumption remain a
separate downstream gate.

Historical conditional acceptance, retained as provenance rather than current
execution authority:

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

This decision is withdrawn. The constant program's two TPROXY rules keep their
existing tail: the policy-route mark `0x1`, then `tproxy`, then `accept`. The
reorder is not adopted, so the canonical program identity does not change and no
rule, set, port, helper, or ownership is altered.

Native evidence settles what the reorder was meant to address. E14(c) and
E14(d) show that outbound guest TCP to an absent transparent listener already
fails closed under the existing order, so ADR-0124 leaves leg-F and leg-C loss
without TAP quiescence on that native evidence, not on a reorder. The exact
retained rule tail and its native receipts live in the #295 feature delta.

## Alternatives considered

### Keep the mark before TPROXY

Selected. Native E14(c)/(d) show that with the existing mark-before-TPROXY order
outbound guest TCP to an absent transparent listener already fails closed — zero
correlated SYN-ACKs and zero wildcard accepts under valid positive controls. The
source-reasoned exposure in the Context does not reproduce in production on the
tested native composition, so the reorder is unnecessary and is withdrawn.

### Quiesce managed TAPs on every listener loss

Reopened and not selected. Native evidence shows listener loss already fails
closed: the ordinary outbound path (E14(c)/(d)) and the `TIME_WAIT` side door
(E14(e), 2026-10-03, door-closed in killed mode with the guest TAP up). Taking
every guest off the network for a listener repair is therefore not needed, and
it would do nothing for a crashed process where no supervisor runs to quiesce
anything. The kernel path preserves the single-loss security outcome without it.

### Rely on the independent intercept-mark guard table (ADR-0139)

Rejected as a substitute for listener loss. That guard drops TCP still marked
`0x295a`, and a listener-absent packet under the retained order is handled by
the unhandled-intercept drop. The guard's own decision (ADR-0139, D-295-R18) is
unaffected and stands for IP-program loss.

## Consequences

Positive: the constant program is unchanged — no canonical-identity change, no
schema conflict, and no one-time stale-table cleanup is introduced by this
decision. Outbound guest TCP to an absent transparent listener fails closed
under the retained order, confirmed on native evidence (E14(c)/(d)), including
after a crash that leaves microVMs running (E14(d), killed `serve` owner).

Neutral:

- The `TIME_WAIT` side door — a SYN matching a transparent `TIME_WAIT` socket
  left by an earlier leg-F connection while leg F is absent — was reasoned from
  kernel source as a possible exposure. The 2026-10-03 named-Service E14(e)
  native result shows it fails closed: a true `TIME_WAIT` on the original
  Service-VIP destination tuple, `serve` owner killed, guest TAP up and the
  entry surviving, crafted newer-sequence guest reconnects answered with no
  SYN-ACK (`reopened=false`) and no wildcard accept (`accepts=0`). The
  crash-window residual that would have gone to the user does not arise, because
  the kernel path fails the reopen closed with no live owner. This is recorded
  on that bounded native evidence and is not extrapolated to untested kernels or
  host configurations.
- Correctness rests on native evidence, because no simulation can observe
  kernel TPROXY and routing.
