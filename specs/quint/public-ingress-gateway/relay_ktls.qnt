// -*- mode: Bluespec; -*-
/**
 * Gateway kTLS transmit-lock / rekey exclusion on the PUBLIC leg (public-ingress-gateway DESIGN): ADR-0244.
 * MODEL-ONLY module: no `conf_step`, no quint-connect driver. Pausing a kernel writer needs a test seam the design
 * does not pin, so this protocol is covered by exhaustive model checking plus the Tier-3 relay suite.
 *
 * Why only the public leg: the upstream leg terminates at leg-C, which never originates a KeyUpdate (main's
 * transparent-mTLS arm installs kTLS from extracted secrets and has no rekey path), so no reply or TX re-arm ever
 * contends with a writer on the upstream leg. The gateway never originates KeyUpdate on either leg.
 *
 * One leg. The tx lock has holders {free, writer, rekeyer}, modelled as `wr` (a writer is mid-write) and `rk` (the
 * rekeyer holds the lock); `owe` (a reply is owed after a received KeyUpdate that requested one). The reply is a TX
 * control record under the OLD key taken under the lock, then TX is re-armed (`rekey_tx`).
 *
 * ASSUMPTION A-K5 (the kernel does not serialise a multi-call logical write against TX re-arm, so the gateway tx
 * lock is load-bearing); details in feature-delta. The user may instead rule that the lock is covered by A-K5 plus
 * the Tier-3 check and drop this module.
 *
 * OFF names the rules switched off (hazard in hazard/):
 *   rekeyDuringWrite  the reply/rekey sequence does not require the tx lock free (ADR-0244)
 */
module relay_ktls {
  const OFF: Set[str]

  val BUG_REKEY: bool = OFF.contains("rekeyDuringWrite")

  var wr: bool
  var rk: bool
  var owe: bool

  action init = all { wr' = false, rk' = false, owe' = false }

  action write_begin: bool = all { not(wr), not(rk), wr' = true, rk' = rk, owe' = owe }

  action write_end: bool = all { wr, wr' = false, rk' = rk, owe' = owe }

  // the peer injects a KeyUpdate that requests a reply; not while a reply is already owed
  action peer_key_update: bool = all { not(owe), owe' = true, wr' = wr, rk' = rk }

  // the reply takes the tx lock as rekeyer (must be free of writers: A-K5)
  action send_reply: bool = all { owe, not(rk), BUG_REKEY or not(wr), rk' = true, wr' = wr, owe' = owe }

  // TX re-arm: lock released, no reply owed
  action rekey_tx: bool = all { rk, rk' = false, owe' = false, wr' = wr }

  action step = any { write_begin, write_end, peer_key_update, send_reply, rekey_tx }

  // ADR-0244: the rekey sequence and relay writers exclude each other on the tx lock
  val TxRekeyExcludesWriters: bool = rk implies not(wr)
  val Safety: bool = TxRekeyExcludesWriters

  // KeyUpdate(request) arrives while a writer holds the lock: the reply starts only after write_end; writes continue
  run keyupdate_waits_for_writer = init
    .then(write_begin)
    .then(peer_key_update)
    .expect(owe and wr and not(rk))
    .then(write_end)
    .expect(owe and not(wr) and not(rk))
    .then(send_reply)
    .expect(rk and not(wr))
    .then(rekey_tx)
    .expect(not(rk) and not(owe) and Safety)
    .then(write_begin)
    .then(write_end)
    .expect(not(wr) and not(rk) and Safety)
}

// Main instance: the approved rules. TLC checks Safety exhaustively.
module ktls_ok { import relay_ktls(OFF = Set()).* from "./relay_ktls" }
