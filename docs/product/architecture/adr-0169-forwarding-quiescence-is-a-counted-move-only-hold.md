# ADR-0169 — Forwarding quiescence is a counted, move-only hold; forwarding reopens only when the last hold is restored

## Status

**Accepted — decision QHOLD approved by user 2026-10-11** (forwarding
quiescence is a counted, move-only hold; a dropped hold keeps forwarding
quiesced and the recovery supervisor fail-stops on it). GH #295. Recorded in
the #295 feature delta, § *[REF] Owner and provisioner*. Completes ADR-0160
(restore reopens admission only) for the question of who may restore.
Runtime recovery itself is ADR-0124 (cadence, bound, fail-stop); a failed
boot write of the shared mTLS table is ADR-0184.

## Context

Node forwarding is quiesced by the runtime supervisor when an audit finds a
shared component damaged: flow admission is refused and live flows are
aborted (ADR-0160). Recovery then repairs the component, audits, and
restores.

Recovery is per component (ADR-0124), and the components fail
independently, each through Overdrive's own failure: the flow and control
listeners, leg-F and leg-C are tasks that end when their accept loop returns
an error (a panic aborts the whole process under the release profile); the DNS responder is a task; the firewall rules and sets
are compared on every audit with what the mTLS worker wrote, and a mismatch
is a recovery of that component; the forwarder's pair bookkeeping is
compared with the owner's pair table. On the appliance only Overdrive writes
the firewall table (ADR-0068), and every write of it is one nft transaction
that leaves the prior table intact on failure (ADR-0184), so a firewall
recovery repairs a mismatch the audit observed, not a part-written table.

Two such recoveries can run at once: the flow-listener task can exit while
the firewall component is being repaired (feature delta, R5-32). With one
shared boolean latch, any recovery's restore reopens forwarding, including
while another recovery is still between its quiesce and its repair: the
listener recovery's restore would reopen forwarding while the firewall
repair has not yet run, so leg-C interception and the host-internal output
rule (ADR-0167) could be missing while flows are admitted, and quiesce and
restore from repeated listener recoveries could keep reopening it with no
upper bound.

## Decision

- **A quiesce returns a hold.** Every quiesce increments the owner's hold
  count and returns one hold. The hold is move-only: it cannot be cloned or
  copied, and the type system flags a hold that is not used.
- **Forwarding is quiesced while the count is above zero.** The first hold
  performs the quiesce (latch, abort flows); later holds join it.
- **Restore consumes a hold.** A restore takes the hold it was given,
  decrements the count, and reopens admission only when the count reaches
  zero; otherwise it reports that forwarding is still quiesced and changes
  nothing else. A restore without a hold cannot be written, so no restore can
  end a hold it was not given.
- **Repair inside the hold.** A recovery repairs its component only while it
  holds a hold, and restores only after its repair and a clean audit
  (ADR-0124). No other recovery's restore can reopen forwarding meanwhile, so
  every recovery's repair runs before forwarding reopens — the firewall
  recovery's included.
- **A dropped hold fails closed.** A hold is dropped without a restore when
  a recovery returns early — a `?` on a failed repair step — or when the
  recovery's future is dropped before it completes. A panic is not a
  producer: the release profile aborts the process on panic (`Cargo.toml`,
  `[profile.release] panic = "abort"`), and the restarted `serve` converges
  on boot. A hold dropped without a restore cannot reopen forwarding: its drop increments the owner's leaked-hold count and
  performs no I/O, and the count of holds is not decremented, so forwarding
  stays quiesced. The recovery supervisor treats a leaked-hold count above
  zero as an exhausted recovery and takes ADR-0124's typed fail-stop; the
  restarted `serve` converges on boot.
- **The observation is the count.** The owner exposes the hold count and the
  leaked-hold count; holds carry no holder names.

The exact operations, hold type, outcomes and accessors are pinned in the
feature delta (§ *Owner and provisioner*).

## Alternatives considered

- **One shared boolean latch, any recovery may restore.** The first
  recovery's restore reopens forwarding while a second recovery is still
  repairing; damaged firewall state has no upper bound. Rejected.
- **Named holders, each restoring only its own hold.** Correct, but the names
  add a holder vocabulary, a "holder already holds" refusal and a holder set
  to observe, none of which the counted move-only hold needs: move-only
  ownership already makes a double restore or a restore without a quiesce
  unwritable. Rejected.
- **Serialize all recoveries behind one global recovery.** Correct, but a
  listener-task exit would wait behind an unrelated firewall repair, and the
  per-component recovery that ADR-0124 bounds becomes a sum of bounds.
  Rejected.
- **A dropped hold decrements the count.** A recovery that returns early on
  a failed repair or whose future is dropped would reopen forwarding with its
  component unrepaired.
  Rejected.

## Consequences

- Every recovery's repair completes before forwarding reopens, under
  fairness of that recovery's own steps only: one audit period plus its own
  ADR-0124 bound, else fail-stop. It never depends on fairness against other
  recoveries.
- Activation is refused with its existing quiescence refusal while the hold
  count is above zero.
- A lost hold ends in fail-stop, never in reopened forwarding; the supervisor
  restart converges on boot (ADR-0184 for the shared mTLS table).
- The rules are covered by the feature delta's § *Formal protocol model*.
