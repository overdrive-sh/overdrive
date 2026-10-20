# ADR-0169 — Forwarding quiescence is a counted, move-only hold; forwarding reopens only when the last hold is restored

## Status

**Accepted — decision QHOLD approved by user 2026-10-11** (forwarding
quiescence is a counted, move-only hold; a dropped hold keeps forwarding
quiesced and the recovery supervisor fail-stops on it); D-UP-3 (a failed
repair is a counted attempt that keeps its hold) and D-UP-4 with PD-1 and
PD-2 (holds are issued and redeemed by the owner's own ledger; a foreign hold
is refused) approved by user 2026-10-14. Not yet implemented. GH #295. Recorded in
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

- **A quiesce returns a hold issued by the owner's own ledger.** Each owner
  holds exactly one ledger, which is not shared and cannot be cloned. Every
  quiesce has that ledger issue one hold and increments the owner's hold
  count. The hold is move-only: it cannot be cloned or copied, cannot be
  built except by a ledger, and the type system flags a hold that is not
  used. Each hold refers to the ledger that issued it.
- **Forwarding is quiesced while the count is above zero.** The first hold
  performs the quiesce (latch, abort flows); later holds join it.
- **Restore consumes a hold of this owner's ledger.** A restore takes the
  hold it was given and has the owner's ledger redeem it; then it decrements
  the count and reopens admission only when the count reaches zero, otherwise
  it reports that forwarding is still quiesced and changes nothing else. A
  restore without a hold cannot be written, so no restore can end a hold it
  was not given.
- **A foreign hold is refused and changes nothing.** A restore given a hold
  that another ledger issued returns a typed foreign-hold error — never a
  panic — and leaves the count, the latch and the owner's leaked-hold count
  unchanged, so forwarding stays as quiesced as it was. One supervisor is
  composed with one owner, so only a composition defect produces this; the
  supervisor that receives the error takes ADR-0124's typed fail-stop of
  `serve` at once with its own cause, and the restarted `serve` converges on
  boot. The foreign hold, dropped unredeemed, counts as leaked at the ledger
  that issued it.
- **Repair inside the hold.** A recovery repairs its component only while it
  holds a hold, and restores only after its repair and a clean audit
  (ADR-0124). No other recovery's restore can reopen forwarding meanwhile, so
  every recovery's repair runs before forwarding reopens — the firewall
  recovery's included.
- **A failed repair is a counted attempt that keeps its hold.** A repair that
  returns an error, and a post-repair audit that fails, are failed attempts
  within the recovery's ADR-0124 budget. The recovery keeps its hold across
  them and ends only by a restore within the budget or by the budget's
  fail-stop while still holding it; no recovery returns early or is dropped
  while it holds a hold.
- **A dropped hold fails closed.** Because no recovery path drops a hold, a
  hold dropped without a restore comes only from an Overdrive defect. A
  panic is not a producer: the release profile aborts the process on panic
  (`Cargo.toml`, `[profile.release] panic = "abort"`), and the restarted
  `serve` converges on boot. A dropped hold reaches the ledger that issued
  it, whose leaked-hold count it increments; the drop performs no I/O, and
  the count of holds is not decremented, so forwarding stays quiesced. The
  recovery supervisor treats a leaked-hold count above zero as an exhausted
  recovery and takes ADR-0124's typed fail-stop at its next audit tick; the
  restarted `serve` converges on boot. `serve`'s own shutdown cancels the
  supervisor with any recovery in progress: that drop is not classified as a
  defect, because the audit that reads the count never runs again; the
  shutdown exit is unchanged and forwarding stays quiesced until the process
  exits.
- **The observation is the count.** The owner exposes the hold count and the
  leaked-hold count (its ledger's); holds carry no holder names.

The exact operations, hold and ledger types, outcomes, the foreign-hold
error, the fail-stop causes and the accessors are pinned in the feature
delta (§ *Owner and provisioner*, § *Typed error taxonomy*).

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
- **A dropped hold decrements the count.** A defect that drops a hold would
  reopen forwarding with its component unrepaired. Rejected.
- **A failed repair returns early and drops its hold (a `?` on the repair).**
  Turns a transient repair failure into a fail-stop of `serve` and
  contradicts ADR-0124's attempt budget, under which a failed repair is a
  failed attempt. Rejected.
- **Restoring a foreign hold is unreachable (`unreachable!`).** A panic is a
  process abort with no typed cause, and the claim rests on a composition
  convention the type system does not enforce. Rejected for a typed error
  that changes nothing and a typed fail-stop.
- **Treat a foreign hold as leaked and report it at the next audit tick.**
  Delays the fail-stop of a composition defect by one audit period and
  merges two distinct defects into one cause. Rejected.
- **The ledger private to the owner, the simulated owner keeping its own
  counter.** The simulated owner could not build a hold, so it could not
  honour the same contract. Rejected: the ledger is public and each owner,
  real or simulated, constructs its own.

## Consequences

- Every recovery's repair completes before forwarding reopens, under
  fairness of that recovery's own steps only: one audit period plus its own
  ADR-0124 bound, else fail-stop. It never depends on fairness against other
  recoveries.
- Activation is refused with its existing quiescence refusal while the hold
  count is above zero.
- A lost hold ends in fail-stop, never in reopened forwarding; the supervisor
  restart converges on boot (ADR-0184 for the shared mTLS table).
- A repair failure costs an attempt, not the process; only a spent budget,
  a leaked hold or a foreign hold fail-stops `serve`.
- No hold of one owner can reopen another owner's forwarding, including a
  simulated owner's hold handed to a production owner.
- The rules are covered by the feature delta's § *Formal protocol model*.
- The foreign-hold fail-stop is verified by the model check and by a unit
  test of the supervisor's mapping of the foreign-hold error to its
  fail-stop cause, not by a production-composition test: one supervisor is
  composed with one owner, so no production path presents a foreign hold,
  and no test seam is added to manufacture one (user ruling 2026-10-15).
