// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — the owner's handling of the empty-datagram notice (UDP-EMPTY,
 * ADR-0165).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * The rules: the host SEQPACKET verdict strips the 8-byte frame of every guest datagram. A frame whose
 * declared length is 0 is dropped (never passed on or redirected) and one notice {flow id, cell epoch}
 * is written to the ringbuf; the guest-flow owner then sends one real zero-length datagram on the
 * association's connected host UDP socket. A notice whose epoch is not the flow's current epoch is
 * discarded (the A2 shape: the cell was reused). A full ring drops the notice and raises the flow's
 * lost counter; the owner reads the counter and, on a rise, sends the counted number of empty
 * datagrams on that flow (the positions are not preserved, D15-R4). Compensation applies only to the live
 * flow of the current epoch: a removed association is not compensated (the counter is not read at removal —
 * compensating a removed association is meaningless). The cell's lost counter is reset when the cell is
 * initialised for a new flow (a new epoch). Notices still in the ring when the association ends are
 * discarded when read.
 *
 * H-2 (r1): the counter modelled here is the cell's `empty_lost`, which counts lost EmptyDatagram notices only; a lost
 * flow-end / drain / redirect-refused record increments the other counter (`ring_lost`, module flow_end), which aborts
 * flows. A lost record increments exactly one of them, so a lost empty notice compensates and never aborts.
 *
 * Abstraction: ONE flow cell reused by successive associations (incarnations) of epochs 1..MAXEP. The
 * kernel verdict (an empty frame) is an environment action; whether the ring accepts its notice is
 * nondeterministic. `owed` counts the empty frames the verdict dropped for the current incarnation;
 * `sent` counts the real zero-length datagrams the owner sent for it. Nothing about the order of
 * datagrams is modelled. The kernel-program verdict invariants (never pass or redirect a zero-length
 * skb, pull the whole skb before a redirect) are Tier-2/3 facts, not owner protocol.
 *
 * OFF names the rules switched off (teeth; instances in hazard/):
 *   noEpoch         a notice is acted on whatever its epoch (a stale notice sends an empty on the new flow)
 *   noCompensation  the owner ignores a raised lost counter
 *   noResetAtReuse  the cell's lost counter is not reset when the cell is initialised for a new flow
 *
 * ASSUMPTIONS (each is an input the model takes as given; the discharging item follows the statement):
 *   UE-1  The verdict never passes or redirects a zero-length skb; a dropped empty frame yields exactly one notice or one lost-counter increment
 *         Discharged by: V-29 items 1b, 2, 3 (findings-v29-udp-empty.md)
 *   UE-2  `send(fd, NULL, 0)` on the association's connected host UDP socket emits one empty datagram
 *         Discharged by: V-29 (the udp_bpf sendmsg citation is the ADR-0165 editor's)
 */
module udp_empty {
  const OFF: Set[str]

  val MAXE: int = 2        // empty frames per incarnation
  val MAXEP: int = 2       // incarnations of the cell
  val BUG_NO_EPOCH: bool = OFF.contains("noEpoch")
  val BUG_NO_COMPENSATION: bool = OFF.contains("noCompensation")
  val BUG_NO_RESET: bool = OFF.contains("noResetAtReuse")

  type Ghost = {
    spurious: bool,       // an empty datagram was sent that no dropped frame of the current incarnation owes
    discarded: bool,      // a stale notice was discarded (witness)
    compensated: bool,    // a lost count was compensated (witness)
  }

  var live: bool            // the association exists
  var ep: int               // the cell's current epoch
  var owed: int
  var sent: int
  var ring: List[int]       // notices (their epochs)
  var lost: int             // the flow's lost counter
  var seen: int             // the owner's last read of it
  var g: Ghost

  val g0: Ghost = { spurious: false, discarded: false, compensated: false }

  action init = all {
    live' = false, ep' = 0, owed' = 0, sent' = 0, ring' = List(), lost' = 0, seen' = 0, g' = g0,
  }

  // a new association takes the cell: a new epoch; the cell's lost counter is reset at initialisation
  // (notices of earlier epochs may remain in the ring)
  action Start = all {
    not(live), ep < MAXEP,
    live' = true, ep' = ep + 1, owed' = 0, sent' = 0,
    lost' = if (BUG_NO_RESET) lost else 0, seen' = if (BUG_NO_RESET) seen else 0,
    ring' = ring, g' = g,
  }
  // the verdict drops an empty frame: a notice with the cell epoch, or — the ring being full — the lost counter
  action Empty(full: bool): bool = all {
    live, owed < MAXE,
    owed' = owed + 1,
    ring' = if (full) ring else ring.append(ep),
    lost' = if (full) lost + 1 else lost,
    live' = live, ep' = ep, sent' = sent, seen' = seen, g' = g,
  }
  // the owner reads one notice
  action OwnNotice = all {
    ring.length() > 0,
    val r = ring.head()
    val send = live and (r == ep or BUG_NO_EPOCH)
    all {
      ring' = ring.tail(),
      sent' = if (send) sent + 1 else sent,
      g' = { ...g, spurious: g.spurious or (send and sent + 1 > owed),
             discarded: g.discarded or not(send) },
    },
    live' = live, ep' = ep, owed' = owed, lost' = lost, seen' = seen,
  }
  // the owner reads the lost counter; on a rise it sends the counted number of empty datagrams
  action OwnReadLost = all {
    not(BUG_NO_COMPENSATION), live, lost > seen,
    sent' = sent + (lost - seen), seen' = lost,
    g' = { ...g, spurious: g.spurious or sent + (lost - seen) > owed, compensated: true },
    live' = live, ep' = ep, owed' = owed, ring' = ring, lost' = lost,
  }
  // the association ends: the flow is removed; its lost counter is not read (no compensation for a removed association)
  action Remove = all {
    live,
    live' = false, ep' = ep, owed' = owed, sent' = sent, ring' = ring, lost' = lost, seen' = seen, g' = g,
  }

  // the last incarnation ended and nothing is pending: the system rests (keeps the bounded search from deadlocking)
  action Idle = all {
    live' = live, ep' = ep, owed' = owed, sent' = sent, ring' = ring, lost' = lost, seen' = seen, g' = g,
  }
  action step = {
    nondet full = Set(true, false).oneOf()
    any { Start, Empty(full), OwnNotice, OwnReadLost, Remove, Idle }
  }

  // ------------------------------------------------------------------ invariants
  // NoSpuriousEmpty (UDP-EMPTY, A2 shape): the owner never sends more empty datagrams than the verdict dropped
  // for the current association — a notice of an earlier epoch sends nothing, and a lost count of an earlier
  // epoch (the cell reused without a reset) is never compensated on the new flow
  val NoSpuriousEmpty: bool = not(g.spurious)
  // LostCountResetAtCellInitialisation (UDP-EMPTY): a live flow's cell carries no lost count of an earlier epoch
  // (the counter and the owner's read of it are zero until a frame of this epoch is dropped)
  val LostCountResetAtCellInitialisation: bool = live implies (lost == seen or owed > 0)
  val Safety: bool = and { NoSpuriousEmpty, LostCountResetAtCellInitialisation }

  // ------------------------------------------------------------------ liveness (TLC)
  val allVars = (live, ep, owed, sent, ring, lost, seen)
  // EveryEmptyEventuallySent (UDP-EMPTY): while the association lives, every empty frame the verdict dropped is
  // eventually sent as a real empty datagram — by its notice, or by the compensation of the lost counter
  temporal EveryEmptyEventuallySent =
    and { weakFair(OwnNotice, allVars), weakFair(OwnReadLost, allVars) }.implies(
      always((live and sent < owed).implies(eventually(sent >= owed or not(live)))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // the notice of an empty frame makes the owner send one empty datagram
  run EmptyFrameNoticeSendsOneEmptyDatagram = init
    .then(Start).then(Empty(false)).expect(Safety and owed == 1 and ring == List(1) and sent == 0)
    .then(OwnNotice).expect(Safety and sent == 1 and owed == 1 and ring == List())
  // a notice of an earlier association, still in the ring when the cell is reused, is discarded
  run StaleEpochNoticeIsDiscardedAfterCellReuse = init
    .then(Start).then(Empty(false)).then(Remove)
    .then(Start).expect(Safety and ep == 2 and ring == List(1) and owed == 0)
    .then(OwnNotice).expect(Safety and sent == 0 and ring == List() and g.discarded)
  // a full ring drops the notices; the lost counter rises; the owner sends the counted number of empties
  run RingDropIsCompensatedByTheCountedNumber = init
    .then(Start).then(Empty(true)).then(Empty(true))
    .expect(Safety and lost == 2 and owed == 2 and sent == 0)
    .then(OwnReadLost).expect(Safety and sent == 2 and seen == 2 and g.compensated)
  // one notice, one drop: the two paths together send exactly the dropped frames
  run NoticeAndCompensationTogetherSendEachDroppedFrameOnce = init
    .then(Start).then(Empty(false)).then(Empty(true))
    .then(OwnReadLost).then(OwnNotice)
    .expect(Safety and sent == 2 and owed == 2 and ring == List() and seen == 1)
  // the lost counter is reset at cell initialisation for a new flow; a removed association is not compensated
  run LostCountIsResetWhenTheCellIsReusedForANewFlow = init
    .then(Start).then(Empty(true)).then(Remove)
    .expect(Safety and lost == 1 and sent == 0 and not(live))
    .then(Start).expect(Safety and ep == 2 and lost == 0 and seen == 0 and owed == 0 and not(g.spurious))
    .then(Empty(false)).then(OwnNotice).expect(Safety and sent == 1 and owed == 1)

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoDiscard: bool = not(g.discarded)
  val WitnessNoCompensation: bool = not(g.compensated)
}

// the design: one cell, two incarnations, two empty frames each
module empty_ok { import udp_empty(OFF = Set()).* from "./udp_empty" }
