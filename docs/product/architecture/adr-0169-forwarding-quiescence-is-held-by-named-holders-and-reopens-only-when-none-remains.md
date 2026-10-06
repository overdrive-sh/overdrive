# ADR-0169 — Forwarding quiescence is held by named holders; forwarding reopens only when no holder remains

## Status

**Proposed — decision D8a-HOLD, approved by user 2026-10-06 (direction:
correct design over simple); pending independent DESIGN review.** GH #295.
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN — PROPOSED 2026-10-05*. Completes ADR-0160 (restore reopens admission
only) for the question of who may restore. Runtime recovery itself is
ADR-0124 (cadence, bound, fail-stop).

## Context

Node forwarding is quiesced by the runtime supervisor when an audit finds a
shared component damaged: flow admission is refused, live flows are aborted
and intake listeners are closed (ADR-0160, ADR-0163). Recovery then repairs
the component, audits, and restores.

Recovery is per component, and more than one component can be under
recovery at once — a firewall table deleted by other software damages the
rule and set components together; a listener task can exit while the
firewall is being repaired. With one shared latch, any recovery's restore
reopens forwarding, including while another recovery is still between its
quiesce and its repair. The model check of revision 7 showed the
consequence (`spike/quint-owner-findings-r3.md`, item 3): after other
software deletes the firewall table, quiesce and restore from different
sources can alternate so that the firewall repair never runs, and the
exposure the deletion opens has no upper bound. The previous text assumed
that the repair would eventually run; nothing in the contract made it so.

## Decision

- **Holders are named.** Quiescing forwarding names its holder — the
  recovery of one shared component. Each holder holds at most one hold at a
  time; a second quiesce by a holder that already holds is refused with a
  typed error.
- **Forwarding is quiesced while any holder holds.** The first hold performs
  the quiesce (latch, abort flows, take intake listeners down); later holds
  join it.
- **Only a holder ends its own hold.** A restore presents the hold it was
  given. It removes that holder and reopens forwarding only if no other
  holder remains; otherwise it reports which holders still hold and changes
  nothing else. No restore can end another holder's hold.
- **Repair inside the hold.** A recovery repairs its component only while it
  holds quiescence, and restores only after its repair and a clean audit
  (ADR-0163 for the firewall). Because no other party can reopen forwarding
  meanwhile, the firewall recovery's repair always runs before forwarding
  reopens.
- A hold that is dropped without a restore keeps forwarding quiesced; the
  supervisor's bounded recovery ends in fail-stop (ADR-0124).

The exact operations, hold handle and error are pinned in the feature delta
(§ *Owner and provisioner*).

## Alternatives considered

- **One shared latch, any recovery may restore.** The r3 lasso: another
  recovery's restore reopens forwarding between the firewall recovery's
  quiesce and its repair; the exposure after a foreign table deletion has no
  upper bound. Rejected.
- **A counter of holds without identity.** Bounds reopening, but a recovery
  that restores twice (or a restore with no matching quiesce) can end another
  recovery's hold, and diagnostics cannot say who holds. Rejected.
- **Serialize all recoveries behind one global recovery.** Correct, but a
  listener-task exit would wait behind an unrelated firewall repair, and the
  per-component recovery that ADR-0124 bounds becomes a sum of bounds.
  Rejected.

## Consequences

- The firewall-deletion exposure (ADR-0152, D8a-FLUSH) has a real upper
  bound while `serve` is up: one audit period plus the firewall recovery's
  own ADR-0124 bound, else fail-stop. It no longer depends on fairness against
  other recoveries.
- Activation reports `QuiescenceLatched` while any holder holds, as before.
- The supervisor's recovery progress can report the current holders.
- The rule is checked by the formal model (ADR-0168,
  `specs/quint/guest-flow-owner/`, module `steering`) with a hazard variant
  in which any party may restore.
