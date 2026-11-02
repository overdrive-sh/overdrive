// -*- mode: Bluespec; -*-
/**
 * Gateway relay exchange protocol (public-ingress-gateway DESIGN): ADR-0239 (body relay boundaries and
 * backpressure), ADR-0245 (the gateway parses HTTP/1.1 heads and owns message framing), ADR-0246 (splice relay).
 * This module is the CONFORMANCE TARGET: a pure state machine over the `ExchangeMachine` owner. The kTLS KeyUpdate /
 * transmit-lock protocol (ADR-0244) lives in the model-only `relay_ktls.qnt`.
 *
 * One connection: Public leg P (keep-alive) and a per-exchange Upstream leg U. One exchange at a time (no
 * pipelining). Body relay is modelled in atomic units (one splice-out never crosses a record boundary, A-K6); chunk
 * framing is NOT modelled (pure-parser property-test concern). The response half may start before the request body
 * is finished.
 *
 * Phases: Idle -> Relay (request head read, upstream connected) -> Idle (reuse) or Closed. Closed is terminal and
 * absorbing. There is no separate "both halves complete" phase: `conclude` (the owner's `finish()`) decides at once.
 *
 * Keep-alive rule (`reusableOf`, the predicate `finish()` applies): Relay -> Idle only if the request body was fully
 * relayed AND the response is complete AND the response was Length-delimited AND the connection is not draining AND P
 * has not half-closed. Otherwise both legs close (Closed). An early response (response complete while the request
 * body is not) closes. Upstream close_notify: before the response head, or mid a Length body with bytes left, aborts
 * both legs; on a close-delimited body it is the clean end of the response (P-13d). Public close_notify: idle or
 * mid-request closes; after the request is fully read the in-flight response finishes, then the connection closes
 * (`p_eof`). Drain: an idle connection closes at once, an in-flight exchange runs to completion (or `force_abort`),
 * and no request is read after it.
 *
 * Bounds: MAXU = 2 units, MAXEX = 2 exchanges per connection.
 *
 * P-13d: everything that exists only for close-delimited (UntilClose) responses is marked `P-13d` below (`len_delim`,
 * `ud_units`, MAXU as the UntilClose bound, resp_head with ld=false, run close_delimited_clean_end). If the user
 * declines P-13d they are all removed in one cut.
 *
 * OFF names the rules switched off (hazard in hazard/):
 *   relayPastBoundary  the relay moves octets of the request or response although the current message is complete
 *                      (the remaining-length guard is dropped; ADR-0239)
 *
 * CONFORMANCE PROJECTION. Every named run of this module is #[quint_test]-replayable and every action maps to one
 * `ExchangeMachine` call. The action -> call table and the variable -> accessor table live ONLY in feature-delta
 * "Pinned observable projection for Quint conformance"; they are not repeated here. Model-only, NOT projected:
 * `exch`, `overread` (ghost), `ud_units`, `conf_step`.
 *
 * Interpretation notes (design silent): framing and body length of the response are chosen by the upstream at
 * response-head time. Upstream close before the head and force_abort stay seeded-sim obligations (no named run).
 *
 * ASSUMPTION A-K6 (unit abstraction: a splice-out never crosses a record boundary); details in feature-delta.
 */
module relay_exchange_conf {
  type ConfStep =
    | init
    | read_head({ n: int })
    | req_unit
    | resp_head({ ld: bool, n: int })
    | resp_unit
    | up_close
    | conclude
    | p_close
    | drain_begin
    | force_abort
}

module relay_exchange {
  import relay_exchange_conf.ConfStep from "./relay_exchange"
  const OFF: Set[str]
  const REC: bool

  val MAXU: int = 2 // P-13d: also the UntilClose unit bound
  val MAXEX: int = 2
  val BUG_BOUNDARY: bool = OFF.contains("relayPastBoundary")

  type S = {
    phase: str, exch: int,
    req_left: int, resp_state: str, resp_left: int, len_delim: bool /* P-13d */, ud_units: int /* P-13d */,
    draining: bool, p_eof: bool,
    overread: int,
  }

  var st: S
  var conf_step: ConfStep

  pure def closedS(s: S): S = {
    ...s, phase: "Closed", req_left: 0, resp_state: "RNone", resp_left: 0, len_delim: true, ud_units: 0,
  }
  pure def reusableOf(s: S): bool =
    s.req_left == 0 and s.resp_left == 0 and s.resp_state == "RDone" and s.len_delim and not(s.draining) and not(s.p_eof)

  val s0: S = {
    phase: "Idle", exch: 0, req_left: 0, resp_state: "RNone", resp_left: 0, len_delim: true, ud_units: 0,
    draining: false, p_eof: false, overread: 0,
  }

  action init = all { st' = s0, conf_step' = variant("init", ()) }

  action go(n: S, c: ConfStep): bool = all {
    st' = n,
    conf_step' = if (REC) c else conf_step,
  }

  // the gateway parses the request head from P (ADR-0245) and connects the per-exchange upstream leg
  action read_head(n: int): bool = all {
    n >= 0, n <= MAXU,
    st.phase == "Idle", not(st.draining), not(st.p_eof), st.exch < MAXEX,
    go({ ...st, phase: "Relay", exch: st.exch + 1, req_left: n, resp_state: "RNone", resp_left: 0,
         len_delim: true, ud_units: 0 },
       variant("read_head", { n: n })),
  }

  // request half: one unit P.rx -> U.tx
  action req_unit: bool = all {
    st.phase == "Relay",
    st.req_left > 0 or BUG_BOUNDARY,
    go({ ...st, req_left: if (st.req_left > 0) st.req_left - 1 else 0,
         overread: st.overread + (if (st.req_left == 0) 1 else 0) },
       variant("req_unit", ())),
  }

  // P-13d (ld=false): response head from U.rx -> P.tx; the upstream chooses framing (Length(n) or UntilClose, n is 0 for UntilClose)
  action resp_head(ld: bool, n: int): bool = all {
    n >= 0, n <= MAXU, ld or n == 0,
    st.phase == "Relay", st.resp_state == "RNone",
    go({ ...st, len_delim: ld, resp_left: n, ud_units: 0,
         resp_state: if (ld and n == 0) "RDone" else "RBody" },
       variant("resp_head", { ld: ld, n: n })),
  }

  // response body unit U.rx -> P.tx
  action resp_unit: bool = all {
    st.phase == "Relay",
    (st.resp_state == "RBody" and (if (st.len_delim) st.resp_left > 0 else st.ud_units < MAXU))
      or (BUG_BOUNDARY and st.resp_state == "RDone"),
    go(
      if (st.resp_state == "RDone") { ...st, overread: st.overread + 1 }
      else if (st.len_delim)
        { ...st, resp_left: st.resp_left - 1, resp_state: if (st.resp_left == 1) "RDone" else "RBody" }
      else { ...st, ud_units: st.ud_units + 1 },
      variant("resp_unit", ())),
  }

  // close_notify from the upstream: before the head or mid a Length body -> abort; close-delimited body -> clean end
  action up_close: bool = all {
    st.phase == "Relay",
    st.resp_state == "RNone" or st.resp_state == "RBody",
    go(if (st.resp_state == "RBody" and not(st.len_delim)) { ...st, resp_state: "RDone" } else closedS(st),
       variant("up_close", ())),
  }

  // response complete: finish() -> Idle when reusable (keep-alive at a clean boundary, never after drain/half-close),
  // else Closed. An early response (request body unfinished) is never reusable. The reused Idle keeps the completed
  // message counters; the next read_head resets them.
  action conclude: bool = all {
    st.phase == "Relay", st.resp_state == "RDone",
    go(if (reusableOf(st)) { ...st, phase: "Idle" } else closedS(st), variant("conclude", ())),
  }

  // ------------------------------------------------------------------ environment
  action p_close: bool = all {
    st.phase != "Closed", not(st.p_eof),
    go(
      if (st.phase == "Idle" or (st.phase == "Relay" and st.req_left > 0)) closedS(st)
      else { ...st, p_eof: true },
      variant("p_close", ())),
  }

  action drain_begin: bool = all {
    st.phase != "Closed", not(st.draining),
    go(if (st.phase == "Idle") { ...closedS(st), draining: true } else { ...st, draining: true },
       variant("drain_begin", ())),
  }

  action force_abort: bool = all {
    st.draining, st.phase == "Relay",
    go(closedS(st), variant("force_abort", ())),
  }

  // stutter for the exhaustive main only; the conformance main (REC = true) never takes it
  action idle: bool = all { not(REC), st' = st, conf_step' = conf_step }

  action step = {
    nondet n = 0.to(MAXU).oneOf()
    nondet ld = Set(true, false).oneOf()
    any {
      read_head(n), req_unit, resp_head(ld, n), resp_unit, up_close, conclude,
      p_close, drain_begin, force_abort, idle,
    }
  }

  // ------------------------------------------------------------------ invariants
  // ADR-0239: the relay moves exactly the octets of the current message
  val NoBytePastMessageBoundary: bool = st.overread == 0
  // guard-derived (no hazard): a mid-body close or an early response never reaches a reused Idle
  val MidBodyCloseNeverReused: bool =
    (st.phase == "Idle" and st.exch > 0) implies
      (st.req_left == 0 and st.resp_left == 0 and st.resp_state == "RDone" and st.len_delim)
  val Safety: bool = and { NoBytePastMessageBoundary, MidBodyCloseNeverReused }

  // ------------------------------------------------------------------ named runs (all #[quint_test]-replayable)
  // two Length-delimited exchanges reuse P; in the first the response starts before the request body is finished
  run exchange_cl_keepalive = init
    .then(read_head(2))
    .then(req_unit)
    .then(resp_head(true, 1))
    .expect(st.req_left == 1 and st.resp_state == "RBody")
    .then(req_unit)
    .then(resp_unit)
    .expect(st.phase == "Relay" and st.resp_state == "RDone" and reusableOf(st))
    .then(conclude)
    .expect(st.phase == "Idle" and st.exch == 1 and st.resp_state == "RDone" and Safety)
    .then(read_head(0))
    .then(resp_head(true, 0))
    .then(conclude)
    .expect(st.phase == "Idle" and st.exch == 2 and st.overread == 0 and Safety)

  // the response completes before the request body: close, no reuse
  run early_response_closes = init
    .then(read_head(2))
    .then(req_unit)
    .then(resp_head(true, 0))
    .expect(st.req_left == 1 and st.resp_state == "RDone" and not(reusableOf(st)))
    .then(conclude)
    .expect(st.phase == "Closed" and st.exch == 1 and Safety)

  // drain_begin during the response body: the exchange completes, is not reused, then closes
  run drain_with_inflight_body = init
    .then(read_head(0))
    .then(resp_head(true, 2))
    .then(resp_unit)
    .then(drain_begin)
    .expect(st.phase == "Relay" and st.draining)
    .then(resp_unit)
    .expect(not(reusableOf(st)))
    .then(conclude)
    .expect(st.phase == "Closed" and Safety)

  // close_notify on P after the request was fully read: the response is still delivered, then the connection closes
  run half_close_after_request_read = init
    .then(read_head(1))
    .then(req_unit)
    .then(p_close)
    .expect(st.phase == "Relay" and st.p_eof and st.req_left == 0)
    .then(resp_head(true, 1))
    .then(resp_unit)
    .expect(st.resp_state == "RDone" and not(reusableOf(st)))
    .then(conclude)
    .expect(st.phase == "Closed" and Safety)

  // close_notify from the upstream mid Length-delimited response body: both legs abort
  run upstream_close_mid_body_aborts = init
    .then(read_head(0))
    .then(resp_head(true, 2))
    .then(resp_unit)
    .expect(st.resp_left == 1 and st.resp_state == "RBody")
    .then(up_close)
    .expect(st.phase == "Closed" and Safety)

  // P-13d: close-delimited response: units relayed, upstream clean close ends it, not reusable
  run close_delimited_clean_end = init
    .then(read_head(0))
    .then(resp_head(false, 0))
    .then(resp_unit)
    .then(resp_unit)
    .expect(st.resp_state == "RBody" and st.ud_units == 2 and st.resp_left == 0)
    .then(up_close)
    .expect(st.resp_state == "RDone" and st.phase == "Relay")
    .expect(not(reusableOf(st)))
    .then(conclude)
    .expect(st.phase == "Closed" and Safety)

  // close_notify on P while the request body is unfinished: abort
  run public_close_mid_request_aborts = init
    .then(read_head(2))
    .then(req_unit)
    .then(p_close)
    .expect(st.phase == "Closed" and Safety)
}

// Main instance: the approved rules. TLC checks Safety exhaustively.
module relay_ok { import relay_exchange(OFF = Set(), REC = false).* from "./relay_exchange" }
// Conformance main: records `conf_step`; only the named runs are checked on it.
module relay_conf { import relay_exchange(OFF = Set(), REC = true).* from "./relay_exchange" }
