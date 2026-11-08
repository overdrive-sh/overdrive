// -*- mode: Bluespec; -*-
/**
 * Gateway Application owner, drain and supersession only (public-ingress-gateway DESIGN, remediation r1): ADR-0222
 * (the owner stages a new generation, promotes it after the connector probe, moves the prior generation to Draining,
 * retires a Draining generation by itself when its admitted connections reach zero (eager), and never restores a superseded
 * generation).
 *
 * Reduced scope: route/frontend/key whole-object coherence, bind gating (ADR-0230), last-usable key preservation
 * (ADR-0231), identity loss, cold-boot order and shutdown ordering are sequential rules of one task and are DISTILL
 * seeded-sim or unit obligations, not modelled here.
 *
 * Protocol: a Route declare creates a new Route generation `route_gen`. `stage` creates a candidate {gid, rg} from
 * the live Route (supersedes a pending candidate; the superseded token's probe result may still arrive). A
 * `probe_result` for the CURRENT staged token whose Route generation is still live promotes it: Current := candidate,
 * prior Current -> Draining. A result for a superseded or discarded token changes nothing. `route_withdraw` clears
 * the staged candidate and moves Current to Draining. Connections lease the Current generation; `retire` removes a
 * Draining generation only at zero leases.
 *
 * OFF names the rules switched off (hazards in hazard/):
 *   retireWhileLeased  a Draining generation is retired while its lease count is non-zero (ADR-0222)
 *
 * Model-only (the driver does not project them): `staged`, `inflight`, `leases`, `next_gid`, `route_on`, `route_gen`,
 * `conf_step`. Conformance projection:
 * current_generation, draining_generations <- GatewayStatusReads::application_status().
 *
 * ASSUMPTION GA-1: the connector probe result is delivered at most once per staged token (discharged by the probe
 * owner's unit-level test). A lease is an RAII guard (released exactly once by construction).
 */
module gateway_application_conf {
  type ConfStep =
    | init
    | route_declare
    | route_withdraw
    | stage
    | probe_result({ tok: int, ok: bool })
    | retire({ gid: int })
    | open_connection
    | close_connection({ gid: int })
}

module gateway_application {
  import gateway_application_conf.ConfStep from "./gateway_application"
  const OFF: Set[str]
  const REC: bool

  val MAXR: int = 3     // Route generations
  val MAXG: int = 4     // application generation ids
  val MAXC: int = 2     // simultaneous admitted connections
  val BUG_RETIRE: bool = OFF.contains("retireWhileLeased")

  type Obj = { gid: int, rg: int }
  type S = {
    route_on: bool, route_gen: int,
    staged: Set[Obj], current: Set[Obj], draining: Set[Obj],
    inflight: Set[int],
    next_gid: int,
    leases: int -> int,
  }

  var st: S
  var conf_step: ConfStep

  pure val zeroLeases: int -> int = 1.to(MAXG).mapBy(i => 0)
  pure def totalLeases(m: int -> int): int = m.keys().fold(0, (a, k) => a + m.get(k))

  val s0: S = {
    route_on: false, route_gen: 0, staged: Set(), current: Set(), draining: Set(), inflight: Set(),
    next_gid: 1, leases: zeroLeases,
  }

  // ADR-0222 (eager retirement): the owner retires a Draining generation by itself the moment its leases reach zero,
  // so no other step runs while a zero-lease Draining generation is pending; only `retire` may.
  val noPendingRetire: bool = st.draining.forall(o => st.leases.get(o.gid) > 0)
  // a demoted Current joins Draining only while it is leased; a zero-lease one is retired in the same action
  pure def demote(cur: Set[Obj], lv: int -> int): Set[Obj] = cur.filter(o => lv.get(o.gid) > 0)

  action init = all { st' = s0, conf_step' = variant("init", ()) }

  action go(n: S, c: ConfStep): bool = all {
    st' = n,
    conf_step' = if (REC) c else conf_step,
  }

  // PublicRouteSetHandle::apply (declare, or replace of the same Route): a new Route generation
  action route_declare: bool = all {
    noPendingRetire, st.route_gen < MAXR,
    go({ ...st, route_on: true, route_gen: st.route_gen + 1 }, variant("route_declare", ())),
  }

  // withdrawal: the pending candidate is discarded (its token stays in flight), Current moves to Draining
  action route_withdraw: bool = all {
    noPendingRetire, st.route_on,
    go({ ...st, route_on: false, staged: Set(), current: Set(),
         draining: st.draining.union(demote(st.current, st.leases)) },
       variant("route_withdraw", ())),
  }

  // the owner stages a candidate from the live Route (supersedes a pending one; its probe result may still arrive)
  action stage: bool = all {
    noPendingRetire, st.route_on, st.next_gid <= MAXG,
    st.staged.union(st.current).forall(o => o.rg != st.route_gen),
    go({ ...st,
         staged: Set({ gid: st.next_gid, rg: st.route_gen }),
         inflight: st.inflight.union(Set(st.next_gid)),
         next_gid: st.next_gid + 1 },
       variant("stage", ())),
  }

  // GatewayUpstreamConnector::probe result
  action probe_result(tok: int, ok: bool): bool = {
    val matches = st.staged.exists(o => o.gid == tok)
    val cand = st.staged.filter(o => o.gid == tok)
    val promote = matches and ok and st.route_on and cand.forall(o => o.rg == st.route_gen)
    all {
      noPendingRetire, st.inflight.contains(tok),
      go(
        if (promote) {
          ...st,
          inflight: st.inflight.exclude(Set(tok)),
          staged: Set(),
          current: cand,
          draining: st.draining.union(demote(st.current, st.leases)),
        } else {
          ...st,
          inflight: st.inflight.exclude(Set(tok)),
          // a failed result removes only the staged candidate; a superseded token changes nothing
          staged: if (matches) Set() else st.staged,
        },
        variant("probe_result", { tok: tok, ok: ok })),
    }
  }

  // a Draining generation is retired only after its retained connections have closed
  action retire(gid: int): bool = all {
    st.draining.exists(o => o.gid == gid), BUG_RETIRE or st.leases.get(gid) == 0,
    go({ ...st, draining: st.draining.filter(o => o.gid != gid) }, variant("retire", { gid: gid })),
  }

  // GatewayAdmission::acquire_connection: the connection retains the whole Current object
  action open_connection: bool = all {
    noPendingRetire, st.current.size() == 1, totalLeases(st.leases) < MAXC,
    go({ ...st, leases: st.current.fold(st.leases, (m, o) => m.set(o.gid, m.get(o.gid) + 1)) },
       variant("open_connection", ())),
  }

  action close_connection(gid: int): bool = all {
    noPendingRetire, st.leases.get(gid) > 0,
    go({ ...st, leases: st.leases.set(gid, st.leases.get(gid) - 1) }, variant("close_connection", { gid: gid })),
  }

  // stutter (QC-2, ruling 7): exhaustive main only; the conformance main (REC = true) never takes it. `stage` and
  // `retire` stay explicit steps: they are owner-autonomous (the owner performs them itself; the driver yields until the
  // owner is idle and asserts the step's postcondition through `application_status()`).
  action idle: bool = all { not(REC), noPendingRetire, st' = st, conf_step' = conf_step }

  action step = {
    nondet t = 1.to(MAXG).oneOf()
    nondet ok = Set(true, false).oneOf()
    any {
      route_declare, route_withdraw, stage, probe_result(t, ok), retire(t), open_connection,
      close_connection(t), idle,
    }
  }

  // ------------------------------------------------------------------ invariants
  val allObjs: Set[Obj] = st.current.union(st.draining)
  // ADR-0222: a connection's retained object stays in Current or Draining until its count is zero
  val DrainRetainsUntilLeasesZero: bool =
    st.leases.keys().forall(gid => st.leases.get(gid) > 0 implies allObjs.exists(o => o.gid == gid))
  // ADR-0222: a result for a superseded or discarded token never produced a Current object newer than its input:
  // at most one Current exists, Current is never also Draining, and a pending candidate is newer than Current
  val SupersededResultNeverRestores: bool = and {
    st.current.size() <= 1,
    st.current.forall(c => st.draining.forall(d => d.gid != c.gid)),
    st.current.forall(c => st.staged.forall(g => g.gid > c.gid)),
  }
  val Safety: bool = and { DrainRetainsUntilLeasesZero, SupersededResultNeverRestores }

  // ------------------------------------------------------------------ named runs
  // supersede, drain, retire: the displaced generation serves its connection until it closes, then retires
  run replace_valid = init
    .then(route_declare).then(stage).then(probe_result(1, true))
    .then(open_connection)
    .then(route_declare).then(stage)
    .expect(st.current.forall(o => o.rg == 1) and st.staged.forall(o => o.rg == 2))
    .then(probe_result(2, true))
    .expect(st.current.forall(o => o.gid == 2 and o.rg == 2) and st.draining.exists(o => o.gid == 1) and Safety)
    .then(close_connection(1)).then(retire(1))
    .expect(st.draining.size() == 0 and st.current.size() == 1 and Safety)

  // withdraw: Current drains and is retired only after the connection closes; a late probe result for the
  // discarded staged token does not restore anything
  run route_withdraw_drain = init
    .then(route_declare).then(stage).then(probe_result(1, true))
    .then(open_connection)
    .then(route_declare).then(stage).expect(st.staged.size() == 1 and st.inflight == Set(2))
    .then(route_withdraw)
    .expect(st.current.size() == 0 and st.staged.size() == 0 and st.draining.exists(o => o.gid == 1))
    .then(probe_result(2, true))
    .expect(st.current.size() == 0 and st.staged.size() == 0 and Safety)
    .then(close_connection(1)).then(retire(1))
    .expect(st.draining.size() == 0 and st.current.size() == 0 and Safety)

  // a probe result for a superseded staged token is discarded: the newer candidate stays staged, nothing is promoted
  run late_probe_after_supersession = init
    .then(route_declare).then(stage)
    .then(route_declare).then(stage)
    .expect(st.staged.forall(o => o.gid == 2) and st.inflight == Set(1, 2))
    .then(probe_result(1, true))
    .expect(st.current.size() == 0 and st.staged.forall(o => o.gid == 2) and st.inflight == Set(2) and Safety)
    .then(probe_result(2, true))
    .expect(st.current.forall(o => o.gid == 2 and o.rg == 2) and st.staged.size() == 0 and Safety)
}

// Main instance: the approved rules. TLC checks Safety exhaustively.
module app_ok { import gateway_application(OFF = Set(), REC = false).* from "./gateway_application" }
// Conformance main: records `conf_step`; only the named runs are checked on it.
module app_conf { import gateway_application(OFF = Set(), REC = true).* from "./gateway_application" }
