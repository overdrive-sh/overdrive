// -*- mode: Bluespec; -*-
/**
 * Service Dataplane selection receipt and applied-backend-identity association (public-ingress-gateway DESIGN):
 * ADR-0228 / ADR-0232 (transient Gateway Connect Intent and Selection Receipt, bound to the socket cookie and
 * ServiceKey), ADR-0233 (commit-guarded publication of the BackendId -> workload identity association), ADR-0344 /
 * ADR-0346 / ADR-0347 (workload SPIFFE identity shared by all allocations of a workload; the allocator memo is
 * keyed by endpoint only and the association of a live BackendId follows the workload currently programmed at its
 * endpoint), which supersede ADR-0234 and ADR-0241; ADR-0243 (the adapter
 * prepares and registers the upstream socket; modelled by `open`); context ADR-0247, ADR-0253, ADR-0254 (the exchange
 * modelled in relay_exchange.qnt); the connector ledger order is ADR-0248 (not modelled).
 *
 * ONE owner: `EbpfDataplane` (and its Sim pair) behind `Dataplane::update_service` and `GatewayConnectDataplane`.
 * Two Service keys (1, 2), guest backend addresses (1, 2), workload identities (1, 2), at most two gateway sockets.
 * The dataplane never sees an allocation: a replacement allocation of the same workload at the same endpoint is a
 * byte-identical `Backend` (R-A), so the model has no allocation dimension.
 *
 * Rules (R-A..R-C):
 *   R-A  `Backend` carries the workload identity `Be.wl`, not an allocation.
 *   R-B  the Applied association BackendId -> workload identity is readable only after commit
 *        (IdentityReadableOnlyAfterCommit) and, whenever the guard is free, equals the workload most recently committed
 *        for that BackendId (AssociationFollowsProgrammedWorkload).
 *   R-C  the allocator memo is keyed by endpoint only (`addr`). The same workload at a memoised endpoint is a memo hit:
 *        the SAME BackendId, association unchanged. A DIFFERENT workload at an endpoint whose memo entry is live is also
 *        a memo hit: the endpoint keeps its BackendId and the id's association is RE-ASSOCIATED with the new workload,
 *        reserved under the guard and published with the commit like a fresh id's. A connection whose receipt was taken
 *        while the id meant workload A and whose identity is read after the re-association resolves workload B (it is
 *        verified against the workload now programmed there). A memo entry is released by orphan GC once no committed
 *        generation references its id. `update_service` NEVER returns an identity error.
 *
 * Transaction (feature-delta "Commit-guarded Service-map and applied-identity transaction"), under ONE write guard:
 *   update_begin  take the guard; allocate fresh BackendIds (monotonic, never recycled) for keys not in the memo and
 *                 RESERVE their association (not readable); the outer SERVICE_MAP pointer is still the prior one
 *   commit        the atomic outer-pointer swap (assumption A-K1) is the selection commit point; the guard is still held.
 *                 Orphan GC follows EVERY commit, including a commit of an empty set (R-38): a memo entry whose BackendId
 *                 no outer generation of any key references is released. The counter never recycles a number and `ident`
 *                 keeps historical identities.
 *   publish       infallible: every reservation (fresh id or re-association) becomes Applied (readable) and the guard is released
 *   rollback      a pre-commit failure: reservations and fresh memos removed, prior outer pointer and prior associations of
 *                 live ids kept, guard released
 * The one remaining rejection is R-59 (`update_rejected_rnat`): an endpoint (reverse-NAT key = endpoint -> VIP) already
 * in the committed generation of ANOTHER Service key, under ANY of its BackendIds. It does not depend on identity.
 * `selected_backend_identity` takes the read side of the guard: it waits while an update holds it, so a read never sees
 * a Reserved, early or mixed association; whenever the guard is free the Applied association is the committed one.
 * Post-commit orphan cleanup failure and its retry (cleanup debt) are NOT modelled: no invariant depends on them, and
 * the design pins them to the private, in-memory debt set of the instance (P-33).
 *
 * Per-socket path: open (prepare, register intent) -> kernel_connect (the hook validates the registered intent, selects
 * from the committed outer generation: no entry / empty => NoBackend, else Selected(BackendId) ; writes
 * exactly one receipt keyed by the cookie and bound to the ServiceKey) -> take (consumes the receipt bound to this
 * cookie and ServiceKey) -> read_identity (Selected only) -> cleanup (removes intent and receipt on every return).
 * `restart` drops and re-creates the dataplane instance: every intent, receipt, map, association and the allocator
 * memo start empty (the maps are not pinned).
 *
 * OFF names the rules switched off (hazards in hazard/):
 *   publishEarly    the association is published Applied at update_begin, before the outer swap (ADR-0233)
 *   staleAssoc      a different workload at a memoised endpoint reuses the BackendId but the association is not updated (ADR-0347, R-C)
 *   takeAny         take_receipt returns any live receipt, not the one bound to its cookie and ServiceKey (ADR-0232)
 *   rnatShare       the R-59 reverse-NAT rejection is off
 *
 * Conformance owner (D-PASS7-2): the real EbpfDataplane; receipt_conf uses update_apply (begin+commit+publish fused),
 * kernel_connect_det and empty/singleton sets {0, 1, 4, 5}. take/read_identity results are checked from port returns, not projected.
 * Model-only (the driver does not project them): `outer` (the kernel map generation, assumption A-K1), `resv`,
 * `infl`, `guard`, `g`, `ckey`, `conf_step`, and (R-27, R-33) `memo`, `next_bid`, `taken`, and the ghost `g.prog`. `memo` and
 * `next_bid` have no owner accessor; their effect is compared through `ident` and the BackendId each live receipt
 * carries. `taken` is a function of projected state, asserted by the take handler; `read_identity` calls
 * selected_backend_identity and has no state effect beyond `conf_step` (the compared values are `ident` and the per-cookie `sel`). A re-association is observed through `ident[BackendId]`
 * changing from workload A to workload B after the `update_apply` that programs B at the endpoint.
 * Conformance projection: identity[BackendId] <- GatewayConnectDataplane::selected_backend_identity(BackendId) (the
 * WORKLOAD SpiffeId); intents <- GatewayConnectCleanup::live_intent_count(); receipts, PER COOKIE,
 * <- GatewayConnectCleanup::live_receipts() (R-33).
 * Runs: commit_then_select, replacement_reuses_backend_id, serve_restart_replacement, different_workload_takeover,
 * reverse_nat_conflict_rejected, restart_destroys_receipts (main receipt_conf, replayed);
 * abandoned_reservation_rolled_back, receipt_waits_for_publish_guard (main receipt_ok: they need the
 * update_begin/commit/publish/rollback phases and the nondeterministic hook, which the real owner cannot be paused
 * at without a seam; evaluator-only, NOT conformance-replayed, R-36, P-32).
 *
 * ASSUMPTIONS: A-K1 (outer swap atomic for the hook: V-GW-1 and the existing atomic-swap Tier-3); A-K2 (SO_COOKIE equals
 * the hook's cookie: V-GW-5); A-3 (every backend address is in the guest prefix: ADR-0190, GH #317). SR-1 the hook
 * writes exactly one receipt per registered cookie and none for an unregistered one (S-PIG-50, Tier 3).
 * SR-2 `restart` empties the Service maps with the instance (see the findings: not stated by the design).
 * A-WL (R-A): the control plane supplies the workload identity on every `Backend`; two allocations of one workload
 * present equal identities (discharged by the ADR-0344/0347 identity derivation tests, not by this model).
 */
module selection_receipt_conf {
  type ConfStep =
    | init
    | update_apply({ key: int, sidx: int })
    | update_rejected_rnat({ key: int, sidx: int })
    | open({ key: int })
    | kernel_connect_det({ c: int })
    | take({ c: int, r: int })
    | read_identity({ c: int })
    | cleanup({ c: int })
    | restart
}

module selection_receipt {
  import selection_receipt_conf.ConfStep from "./selection_receipt"
  const OFF: Set[str]
  const REC: bool

  const MAXUPD: int       // bound on accepted update_service calls (2 on the exhaustive main, 3 on the conformance main)
  val MAXC: int = 2       // gateway sockets (cookies)
  val BUG_EARLY: bool = OFF.contains("publishEarly")
  val BUG_STALE: bool = OFF.contains("staleAssoc")
  val BUG_TAKEANY: bool = OFF.contains("takeAny")
  val BUG_RNAT: bool = OFF.contains("rnatShare")

  type Be = { addr: int, wl: int, h: int }
  // assoc = the associations the update establishes (fresh ids and re-associated live ids); wls = the workload of every
  // backend in the set, by BackendId (the committed association once the update commits)
  type Infl = { on: bool, key: int, ents: int -> int, assoc: int -> int, wls: int -> int, fresh_addrs: Set[int],
                prior: int -> int, identBefore: int -> int, committed: bool }
  type Rcpt = { key: int, sel: int }                  // sel 0 = NoBackend
  type Taken = { rc: int, key: int, sel: int }

  // the input catalogue of update_service backend sets (index -> set)
  pure val CATALOG: int -> Set[Be] = Map(
    0 -> Set(),
    1 -> Set({ addr: 1, wl: 1, h: 1 }),
    2 -> Set({ addr: 1, wl: 1, h: 0 }),
    3 -> Set({ addr: 1, wl: 2, h: 1 }, { addr: 2, wl: 1, h: 1 }),
    4 -> Set({ addr: 1, wl: 2, h: 1 }),
    5 -> Set({ addr: 2, wl: 2, h: 1 }),
  )

  var outer: int -> (int -> int)       // ServiceKey -> (BackendId -> healthy): the committed selectable generation
  var ident: int -> int                // BackendId -> workload identity: Applied, readable
  var resv: int -> int                 // BackendId -> workload identity: Reserved, not readable
  var memo: int -> int                 // endpoint (addr) -> BackendId (allocator memo, R-C)
  var next_bid: int
  var guard: bool
  var infl: Infl
  var upd: int
  var intents: int -> int              // cookie -> ServiceKey
  var receipts: int -> Rcpt            // cookie -> receipt
  var conn: int -> str                 // cookie -> opened | connected | taken | identified | cleaned | lost
  var ckey: int -> int                 // cookie -> the ServiceKey it was opened for
  var taken: int -> Taken              // cookie -> the receipt its connector consumed
  var next_cookie: int
  // ghosts: committed = BackendIds of any committed generation; prog = BackendId -> the workload most recently committed
  // for it (the workload of the backend programmed at its endpoint in the latest committed generation naming it)
  var g: { committed: Set[int], prog: int -> int, rbBad: bool, badSel: bool }
  var conf_step: ConfStep

  // R-38 orphan GC: release the memo entries whose BackendId no outer generation (any key) references
  pure def gcMemo(m: int -> int, o: int -> (int -> int)): int -> int = {
    val live = o.keys().fold(Set(), (acc, k) => acc.union(o.get(k).keys()))
    m.keys().filter(a => live.contains(m.get(a))).mapBy(a => m.get(a))
  }
  // R-59: a backend endpoint already in the committed generation of ANOTHER Service key (the endpoint has exactly one
  // memoised BackendId). A Service's own generation does not count (replacing its own set is not sharing).
  pure def rnatHeld(o: int -> (int -> int), m: int -> int, key: int, bs: Set[Be]): bool =
    bs.exists(b => m.keys().contains(b.addr) and
                   o.keys().exists(k => k != key and o.get(k).keys().contains(m.get(b.addr))))
  pure def rm(m: int -> a, k: int): int -> a = m.keys().exclude(Set(k)).mapBy(x => m.get(x))
  pure def entsOf(o: int -> (int -> int), s: int): int -> int = if (o.keys().contains(s)) o.get(s) else Map()

  val noInfl: Infl = { on: false, key: 0, ents: Map(), assoc: Map(), wls: Map(), fresh_addrs: Set(), prior: Map(),
                       identBefore: Map(), committed: false }

  action init = all {
    outer' = Map(), ident' = Map(), resv' = Map(), memo' = Map(), next_bid' = 1, guard' = false, infl' = noInfl,
    upd' = 0, intents' = Map(), receipts' = Map(), conn' = Map(), ckey' = Map(), taken' = Map(),
    next_cookie' = 1,
    g' = { committed: Set(), prog: Map(), rbBad: false, badSel: false },
    conf_step' = variant("init", ()),
  }

  // R-C memo key: the endpoint only
  pure def mk(b: Be): int = b.addr
  // Dataplane::update_service, first half: take the guard, reserve (or, under the hazard, publish) the associations
  // of fresh ids and of live ids whose endpoint now hosts a different workload (re-association).
  // No identity check: update_service never returns an identity error (R-C).
  action update_begin(key: int, sidx: int): bool = {
    val bs = CATALOG.get(sidx)
    def assocOf(id) = if (ident.keys().contains(id)) ident.get(id) else resv.get(id)
    val rnat = rnatHeld(outer, memo, key, bs)
    val freshB = bs.filter(b => not(memo.keys().contains(mk(b))))
    def idOf(b) = if (memo.keys().contains(mk(b))) memo.get(mk(b))
                          else next_bid + freshB.filter(x => mk(x) < mk(b)).size()
    val ents = bs.fold(Map(), (m, b) => m.put(idOf(b), b.h))
    // associations this update establishes: the fresh ids plus the re-associated live ids (hazard staleAssoc: the
    // fresh ids only, so a live id keeps the previous workload)
    val assocB = if (BUG_STALE) freshB else bs.filter(b => not(memo.keys().contains(mk(b))) or
                                                           assocOf(memo.get(mk(b))) != b.wl)
    val assoc = assocB.fold(Map(), (m, b) => m.put(idOf(b), b.wl))
    val wls = bs.fold(Map(), (m, b) => m.put(idOf(b), b.wl))
    val newMemo = freshB.fold(memo, (m, b) => m.put(mk(b), idOf(b)))
    all {
      not(guard), not(infl.on), upd < MAXUPD,
      // R-59: an endpoint whose reverse-NAT key maps to another Service is rejected before any mutation (see
      // update_rejected_rnat); the hazard lets it through
      or { not(rnat), BUG_RNAT },
      guard' = true,
      infl' = { on: true, key: key, ents: ents, assoc: assoc, wls: wls, fresh_addrs: freshB.map(b => mk(b)),
                prior: entsOf(outer, key), identBefore: ident, committed: false },
      memo' = newMemo,
      next_bid' = next_bid + freshB.size(),
      resv' = if (BUG_EARLY) resv else assoc.keys().fold(resv, (m, id) => m.put(id, assoc.get(id))),
      ident' = if (BUG_EARLY) assoc.keys().fold(ident, (m, id) => m.put(id, assoc.get(id))) else ident,
      upd' = upd + 1,
      g' = g,
      outer' = outer, intents' = intents, receipts' = receipts, conn' = conn, ckey' = ckey, taken' = taken,
      next_cookie' = next_cookie,
      conf_step' = conf_step,
    }
  }

  // R-59: Dataplane::update_service on a backend set naming an endpoint whose reverse-NAT key already maps to a
  // different VIP (ServiceKey): the call returns Err(ReverseNatConflict) before any mutation; both Services'
  // state, the memo and the counter are unchanged. Outcome = the conf_step variant; no state variable changes.
  action update_rejected_rnat(key: int, sidx: int): bool = {
    val bs = CATALOG.get(sidx)
    all {
      not(guard), not(infl.on), rnatHeld(outer, memo, key, bs),
      outer' = outer, ident' = ident, resv' = resv, memo' = memo, next_bid' = next_bid, guard' = guard, infl' = infl,
      upd' = upd, intents' = intents, receipts' = receipts, conn' = conn, ckey' = ckey, taken' = taken,
      next_cookie' = next_cookie, g' = g,
      conf_step' = if (REC) variant("update_rejected_rnat", { key: key, sidx: sidx }) else conf_step,
    }
  }

  // Conformance-observable form of Dataplane::update_service (D-PASS7-2): update_begin; commit; publish fused,
  // because the real EbpfDataplane cannot be paused between the phases without a test seam. 
  action update_apply(key: int, sidx: int): bool = {
    val bs = CATALOG.get(sidx)
    val rnat = rnatHeld(outer, memo, key, bs)
    val freshB = bs.filter(b => not(memo.keys().contains(mk(b))))
    def idOf(b) = if (memo.keys().contains(mk(b))) memo.get(mk(b))
                          else next_bid + freshB.filter(x => mk(x) < mk(b)).size()
    val ents = bs.fold(Map(), (m, b) => m.put(idOf(b), b.h))
    val assocB = bs.filter(b => not(memo.keys().contains(mk(b))) or ident.get(memo.get(mk(b))) != b.wl)
    val assoc = assocB.fold(Map(), (m, b) => m.put(idOf(b), b.wl))
    val wls = bs.fold(Map(), (m, b) => m.put(idOf(b), b.wl))
    val newMemo = freshB.fold(memo, (m, b) => m.put(mk(b), idOf(b)))
    val newOuter = if (ents.keys().size() == 0) rm(outer, key) else outer.put(key, ents)
    all {
      not(guard), not(infl.on), upd < MAXUPD, not(rnat),
      outer' = newOuter,
      ident' = assoc.keys().fold(ident, (m, id) => m.put(id, assoc.get(id))),
      memo' = gcMemo(newMemo, newOuter),
      next_bid' = next_bid + freshB.size(),
      upd' = upd + 1,
      g' = { ...g, prog: wls.keys().fold(g.prog, (m, id) => m.put(id, wls.get(id))),
             committed: g.committed.union(ents.keys()) },
      resv' = resv, guard' = guard, infl' = infl, intents' = intents, receipts' = receipts, conn' = conn,
      ckey' = ckey, taken' = taken, next_cookie' = next_cookie,
      conf_step' = if (REC) variant("update_apply", { key: key, sidx: sidx }) else conf_step,
    }
  }

  // the atomic outer SERVICE_MAP pointer swap: the selection commit point. An empty set deletes the key.
  action commit: bool = all {
    infl.on, not(infl.committed),
    outer' = if (infl.ents.keys().size() == 0) rm(outer, infl.key) else outer.put(infl.key, infl.ents),
    infl' = { ...infl, committed: true },
    g' = { ...g, prog: infl.wls.keys().fold(g.prog, (m, id) => m.put(id, infl.wls.get(id))),
           committed: g.committed.union(infl.ents.keys()) },
    ident' = ident, resv' = resv, next_bid' = next_bid, guard' = guard, upd' = upd,
    memo' = gcMemo(memo, if (infl.ents.keys().size() == 0) rm(outer, infl.key) else outer.put(infl.key, infl.ents)),
    intents' = intents, receipts' = receipts, conn' = conn, ckey' = ckey, taken' = taken, next_cookie' = next_cookie,
    conf_step' = conf_step,
  }

  // infallible: reservations (fresh ids and re-associations) become Applied, the guard is released
  action publish: bool = all {
    infl.on, infl.committed,
    ident' = infl.assoc.keys().fold(ident, (m, id) => m.put(id, infl.assoc.get(id))),
    resv' = infl.assoc.keys().fold(resv, (m, id) => rm(m, id)),
    guard' = false, infl' = noInfl,
    outer' = outer, memo' = memo, next_bid' = next_bid, upd' = upd, intents' = intents,
    receipts' = receipts, conn' = conn, ckey' = ckey, taken' = taken, next_cookie' = next_cookie,
    g' = g,
    conf_step' = conf_step,
  }

  // an injected pre-commit failure: restore the captured prior state; the Applied association of every live id is
  // untouched (a re-association is only Reserved before the commit); ids are not recycled
  action rollback: bool = all {
    infl.on, not(infl.committed),
    resv' = infl.assoc.keys().fold(resv, (m, id) => rm(m, id)),
    memo' = memo.keys().filter(a => not(infl.fresh_addrs.contains(a))).mapBy(a => memo.get(a)),
    guard' = false, infl' = noInfl,
    g' = { ...g, rbBad: g.rbBad or entsOf(outer, infl.key) != infl.prior or ident != infl.identBefore },
    outer' = outer, ident' = ident, next_bid' = next_bid, upd' = upd, intents' = intents,
    receipts' = receipts, conn' = conn, ckey' = ckey, taken' = taken, next_cookie' = next_cookie,
    conf_step' = conf_step,
  }

  // GatewayConnectDataplane::open_upstream: prepare the socket, read its cookie, register (cookie, ServiceKey)
  action open(key: int): bool = all {
    next_cookie <= MAXC,
    intents' = intents.put(next_cookie, key),
    conn' = conn.put(next_cookie, "opened"),
    ckey' = ckey.put(next_cookie, key),
    next_cookie' = next_cookie + 1,
    outer' = outer, ident' = ident, resv' = resv, memo' = memo, next_bid' = next_bid, guard' = guard, infl' = infl,
    upd' = upd, receipts' = receipts, taken' = taken, g' = g,
    conf_step' = if (REC) variant("open", { key: key }) else conf_step,
  }

  // the cgroup/connect4 hook for a registered cookie: selects from the committed outer generation, writes one receipt.
  // pick = the Maglev choice among the generation's entries (0 when there is none)
  action kernel_connect(c: int, pick: int): bool = {
    val key = intents.get(c)
    val e = entsOf(outer, key)
    val sel = if (e.keys().contains(pick)) pick else 0
    all {
      conn.keys().contains(c), conn.get(c) == "opened", intents.keys().contains(c),
      or { e.keys().size() == 0 and pick == 0, e.keys().contains(pick) },
      receipts' = receipts.put(c, { key: key, sel: sel }),
      conn' = conn.put(c, "connected"),
      g' = { ...g, badSel: g.badSel or (sel != 0 and not(e.keys().contains(sel))) },
      outer' = outer, ident' = ident, resv' = resv, memo' = memo, next_bid' = next_bid, guard' = guard, infl' = infl,
      upd' = upd, intents' = intents, ckey' = ckey, taken' = taken, next_cookie' = next_cookie,
      conf_step' = conf_step,
    }
  }

  // conformance form of the hook (D-PASS7-2): the Maglev choice is not controllable, so the conformance catalogue
  // holds only empty and singleton sets and the selection is the only entry (or NoBackend)
  action kernel_connect_det(c: int): bool = {
    val key = intents.get(c)
    val e = entsOf(outer, key)
    val sel = e.keys().fold(0, (a, k) => k)
    all {
      conn.keys().contains(c), conn.get(c) == "opened", intents.keys().contains(c),
      receipts' = receipts.put(c, { key: key, sel: sel }),
      conn' = conn.put(c, "connected"),
      g' = { ...g, badSel: g.badSel or (sel != 0 and not(e.keys().contains(sel))) },
      outer' = outer, ident' = ident, resv' = resv, memo' = memo, next_bid' = next_bid, guard' = guard, infl' = infl,
      upd' = upd, intents' = intents, ckey' = ckey, taken' = taken, next_cookie' = next_cookie,
      conf_step' = if (REC) variant("kernel_connect_det", { c: c }) else conf_step,
    }
  }

  // take_receipt(&intent): the receipt bound to this cookie and ServiceKey, consumed. r = the receipt looked up
  action take(c: int, r: int): bool = all {
    conn.keys().contains(c), conn.get(c) == "connected",
    receipts.keys().contains(r),
    if (BUG_TAKEANY) true else (r == c and receipts.get(r).key == intents.get(c)),
    taken' = taken.put(c, { rc: r, key: receipts.get(r).key, sel: receipts.get(r).sel }),
    receipts' = rm(receipts, r),
    conn' = conn.put(c, "taken"),
    outer' = outer, ident' = ident, resv' = resv, memo' = memo, next_bid' = next_bid, guard' = guard, infl' = infl,
    upd' = upd, intents' = intents, ckey' = ckey, next_cookie' = next_cookie, g' = g,
    conf_step' = if (REC) variant("take", { c: c, r: r }) else conf_step,
  }

  // selected_backend_identity(BackendId): the read side of the commit guard; Reserved is not readable
  action read_identity(c: int): bool = {
    val sel = taken.get(c).sel
    all {
      conn.keys().contains(c), conn.get(c) == "taken", sel != 0, not(guard),
      conn' = conn.put(c, "identified"),
      outer' = outer, ident' = ident, resv' = resv, memo' = memo, next_bid' = next_bid, guard' = guard, infl' = infl,
      upd' = upd, intents' = intents, receipts' = receipts, ckey' = ckey, taken' = taken,
      next_cookie' = next_cookie, g' = g,
      conf_step' = if (REC) variant("read_identity", { c: c }) else conf_step,
    }
  }

  // the connector removes the intent (and any residual receipt) before it returns, on every path
  action cleanup(c: int): bool = all {
    conn.keys().contains(c), conn.get(c) != "cleaned", conn.get(c) != "lost",
    intents' = rm(intents, c), receipts' = rm(receipts, c), conn' = conn.put(c, "cleaned"),
    outer' = outer, ident' = ident, resv' = resv, memo' = memo, next_bid' = next_bid, guard' = guard, infl' = infl,
    upd' = upd, ckey' = ckey, taken' = taken, next_cookie' = next_cookie, g' = g,
    conf_step' = if (REC) variant("cleanup", { c: c }) else conf_step,
  }

  // environment fault: the dataplane instance is dropped and re-created
  action restart = all {
    outer' = Map(), ident' = Map(), resv' = Map(), memo' = Map(), next_bid' = 1, guard' = false, infl' = noInfl,
    intents' = Map(), receipts' = Map(),
    conn' = conn.keys().mapBy(c => if (conn.get(c) == "cleaned") "cleaned" else "lost"),
    g' = { ...g, committed: Set(), prog: Map() },
    upd' = upd, ckey' = ckey, taken' = taken, next_cookie' = next_cookie,
    conf_step' = if (REC) variant("restart", ()) else conf_step,
  }

  // stutter (QC-3, ruling 7): taken only by the non-conformance step relation; `receipt_conf` never takes it
  action idle = all {
    not(REC),
    outer' = outer, ident' = ident, resv' = resv, memo' = memo, next_bid' = next_bid, guard' = guard, infl' = infl,
    upd' = upd, intents' = intents, receipts' = receipts, conn' = conn, ckey' = ckey, taken' = taken,
    next_cookie' = next_cookie, g' = g,
    conf_step' = conf_step,
  }

  action step = {
    if (REC) {
      // conformance step relation (receipt_conf): the actions the real EbpfDataplane can be driven through;
      // catalogue: empty and singleton healthy sets only
      nondet k = 1.to(2).oneOf()
      nondet cs = Set(0, 1, 4, 5).oneOf()
      nondet c = 1.to(MAXC).oneOf()
      any {
        update_apply(k, cs), update_rejected_rnat(k, cs), open(k), kernel_connect_det(c), take(c, c), read_identity(c),
        cleanup(c), restart,
      }
    } else {
      nondet k = 1.to(2).oneOf()
      nondet s = 0.to(5).oneOf()
      nondet c = 1.to(MAXC).oneOf()
      nondet r = 1.to(MAXC).oneOf()
      nondet pick = 0.to(4).oneOf()
      any {
        update_begin(k, s), update_rejected_rnat(k, s), commit, publish, rollback, open(k), kernel_connect(c, pick), take(c, r),
        read_identity(c), cleanup(c), restart, idle,
      }
    }
  }

  // ------------------------------------------------------------------ invariants
  // ADR-0233: an association is readable only for ids of a committed generation, and a reservation never outlives
  // the guard (a Reserved value is never readable)
  val IdentityReadableOnlyAfterCommit: bool = and {
    ident.keys().forall(id => g.committed.contains(id)),
    (not(guard)) implies resv.keys().size() == 0,
  }
  // R-B / R-C (ADR-0347): whenever the guard is free, the Applied association of every BackendId ever committed is the
  // workload most recently committed for it: a takeover leaves no stale association, and no id resolves a workload that
  // was never programmed at its endpoint. `read_identity` waits for the guard, so a read returns this value.
  val AssociationFollowsProgrammedWorkload: bool = (not(guard)) implies and {
    ident.keys() == g.prog.keys(),
    ident.keys().forall(id => ident.get(id) == g.prog.get(id)),
  }
  // ADR-0232: the consumed receipt is the one of this cookie and its registered ServiceKey
  val ReceiptMatchesIntent: bool = taken.keys().forall(c => and { taken.get(c).rc == c, taken.get(c).key == ckey.get(c) })
  // ADR-0232: a finished or lost socket leaves no intent and no receipt
  val NoResidueAfterCleanup: bool = conn.keys().forall(c =>
    (conn.get(c) == "cleaned" or conn.get(c) == "lost") implies and {
      not(intents.keys().contains(c)), not(receipts.keys().contains(c)) })
  // ADR-0233 / ADR-0347: a failed pre-commit step leaves the prior generation and the Applied association of every
  // live id as they were (a reserved re-association is dropped); before the swap the outer pointer is still the prior one
  val RollbackPreservesPriorGeneration: bool = and {
    not(g.rbBad),
    (infl.on and not(infl.committed)) implies entsOf(outer, infl.key) == infl.prior,
  }
  // ADR-0242: no outer entry or an empty generation is NoBackend, never Selected
  val EmptyIsNoBackend: bool = not(g.badSel)
  // R-59: no reverse-NAT key (endpoint) maps to two VIPs (ServiceKeys) in the committed generations. An endpoint has
  // exactly one BackendId (R-C), so the check is at the BackendId level.
  val ReverseNatKeyOneVip: bool = outer.keys().forall(k1 => outer.keys().forall(k2 =>
    (k1 != k2) implies outer.get(k1).keys().intersect(outer.get(k2).keys()).size() == 0))
  val Safety: bool = and {
    IdentityReadableOnlyAfterCommit, AssociationFollowsProgrammedWorkload, ReverseNatKeyOneVip, ReceiptMatchesIntent, NoResidueAfterCleanup,
    RollbackPreservesPriorGeneration, EmptyIsNoBackend,
  }

  // ------------------------------------------------------------------ named runs
  // Conformance-bound runs (main receipt_conf: fused update_apply, deterministic hook, empty/singleton sets).
  // commit a generation, select through a registered socket, read the committed identity, clean up
  run commit_then_select = init
    .then(update_apply(1, 1))
    .expect(not(guard) and outer.get(1) == Map(1 -> 1) and ident == Map(1 -> 1) and resv.keys().size() == 0 and Safety)
    .then(open(1)).expect(intents == Map(1 -> 1))
    .then(kernel_connect_det(1)).expect(receipts == Map(1 -> { key: 1, sel: 1 }))
    .then(take(1, 1)).expect(receipts.keys().size() == 0 and taken.get(1).sel == 1 and Safety)
    .then(read_identity(1)).expect(conn.get(1) == "identified" and ident.get(taken.get(1).sel) == 1)
    .then(cleanup(1)).expect(intents.keys().size() == 0 and receipts.keys().size() == 0 and Safety)

  // R-A/R-C: a replacement allocation of the same workload at the same endpoint is a byte-identical Backend: memo hit,
  // same BackendId, no new id, association unchanged.
  // The tail pins orphan GC on an empty-set commit (R-38): the memo is released, `ident` keeps its history, the
  // counter does not recycle.
  run replacement_reuses_backend_id = init
    .then(update_apply(1, 1))
    .then(update_apply(1, 1))
    .expect(not(guard) and memo == Map(1 -> 1) and ident == Map(1 -> 1) and next_bid == 2 and outer == Map(1 -> Map(1 -> 1))
            and g.prog == Map(1 -> 1) and upd == 2 and Safety)
    .then(update_apply(1, 0))
    .expect(outer.keys().size() == 0 and memo.keys().size() == 0 and ident == Map(1 -> 1) and next_bid == 2 and Safety)

  // S-PIG-71: serve restarts, boot resync programs the stale row, then the replacement allocation (same workload, same
  // endpoint) is accepted: the workload identity is read.
  run serve_restart_replacement = init
    .then(update_apply(1, 1))
    .then(restart)
    .expect(outer.keys().size() == 0 and memo.keys().size() == 0 and next_bid == 1 and Safety)
    .then(update_apply(1, 1))
    .then(update_apply(1, 1))
    .expect(not(guard) and memo == Map(1 -> 1) and ident == Map(1 -> 1) and next_bid == 2 and outer == Map(1 -> Map(1 -> 1))
            and Safety)
    .then(open(1)).then(kernel_connect_det(1)).then(take(1, 1)).then(read_identity(1))
    .expect(conn.get(1) == "identified" and taken.get(1).sel == 1 and ident.get(taken.get(1).sel) == 1 and Safety)

  // R-C (ADR-0347): a DIFFERENT workload at a memoised endpoint keeps the endpoint's BackendId and re-associates it:
  // the memo keeps one entry, the counter does not advance, `ident[1]` moves from workload 1 to workload 2. The receipt
  // taken BEFORE the takeover (BackendId 1) and read AFTER it resolves workload 2: the connection is verified against
  // the workload now programmed at the endpoint. A new socket after the takeover also selects BackendId 1 -> workload 2.
  run different_workload_takeover = init
    .then(update_apply(1, 1))
    .expect(ident == Map(1 -> 1) and g.prog == Map(1 -> 1))
    .then(open(1)).then(kernel_connect_det(1)).then(take(1, 1))
    .expect(taken.get(1).sel == 1 and ident.get(taken.get(1).sel) == 1 and Safety)
    .then(update_apply(1, 4))
    .expect(not(guard) and outer == Map(1 -> Map(1 -> 1)) and ident == Map(1 -> 2) and memo == Map(1 -> 1)
            and next_bid == 2 and g.prog == Map(1 -> 2) and upd == 2 and Safety)
    .then(read_identity(1))
    .expect(conn.get(1) == "identified" and taken.get(1).sel == 1 and ident.get(taken.get(1).sel) == 2 and Safety)
    .then(open(1)).then(kernel_connect_det(2)).expect(receipts == Map(2 -> { key: 1, sel: 1 }))
    .then(take(2, 2)).then(read_identity(2))
    .expect(conn.get(2) == "identified" and taken.get(2).sel == 1 and ident.get(taken.get(2).sel) == 2
            and memo == Map(1 -> 1) and next_bid == 2 and Safety)

  // R-59: an endpoint already held by another Service is rejected before any mutation (ReverseNatConflict); both
  // Services' state, the memo and the counter are unchanged and each still selects its own backend
  run reverse_nat_conflict_rejected = init
    .then(update_apply(1, 1))
    .then(update_apply(2, 5))
    .expect(outer == Map(1 -> Map(1 -> 1), 2 -> Map(2 -> 1)) and memo == Map(1 -> 1, 2 -> 2) and next_bid == 3
            and upd == 2 and Safety)
    .then(update_rejected_rnat(2, 1))
    .expect(not(guard) and outer == Map(1 -> Map(1 -> 1), 2 -> Map(2 -> 1)) and ident == Map(1 -> 1, 2 -> 2)
            and memo == Map(1 -> 1, 2 -> 2) and next_bid == 3 and upd == 2 and resv.keys().size() == 0 and Safety)
    .then(open(2)).then(kernel_connect_det(1)).expect(receipts == Map(1 -> { key: 2, sel: 2 }))
    .then(take(1, 1)).then(read_identity(1))
    .expect(conn.get(1) == "identified" and ident.get(taken.get(1).sel) == 2 and Safety)

  // restart: the intent and receipt maps are not pinned; the new instance holds nothing
  run restart_destroys_receipts = init
    .then(update_apply(1, 1))
    .then(open(1)).then(kernel_connect_det(1))
    .expect(intents.keys().size() == 1 and receipts.keys().size() == 1)
    .then(restart)
    .expect(intents.keys().size() == 0 and receipts.keys().size() == 0 and ident.keys().size() == 0
            and outer.keys().size() == 0 and conn.get(1) == "lost" and next_bid == 1 and Safety)
    .then(update_apply(1, 1))
    .then(open(1)).then(kernel_connect_det(2)).then(take(2, 2)).then(read_identity(2))
    .expect(conn.get(2) == "identified" and ident.get(taken.get(2).sel) == 1 and Safety)

  // Evaluator-only runs (main receipt_ok, MAXUPD = 2): the phases of update_service are not separately drivable on
  // the real owner, so these are model-level scenarios, NOT conformance-replayable (R-36, P-32).
  // a failed update: the reservations (fresh id 2 and the re-association of live id 1 to workload 2) are removed, the
  // Applied association of id 1 stays workload 1, the prior generation is kept, the id is not recycled (next_bid stays 3)
  run abandoned_reservation_rolled_back = init
    .then(update_begin(1, 1)).then(commit).then(publish)
    .then(update_begin(1, 3))
    .expect(resv == Map(1 -> 2, 2 -> 1) and ident == Map(1 -> 1) and memo == Map(1 -> 1, 2 -> 2) and guard)
    .then(rollback)
    .expect(not(guard) and ident == Map(1 -> 1) and resv.keys().size() == 0 and outer.get(1) == Map(1 -> 1)
            and memo == Map(1 -> 1) and next_bid == 3 and upd == 2 and not(g.rbBad) and g.prog == Map(1 -> 1) and Safety)

  // a receipt can arrive right after the outer swap; its identity read waits for the publish and the guard release
  run receipt_waits_for_publish_guard = init
    .then(update_begin(1, 1)).then(commit)
    .expect(guard and ident.keys().size() == 0 and outer.get(1) == Map(1 -> 1))
    .then(open(1)).then(kernel_connect(1, 1)).then(take(1, 1))
    .expect(guard and taken.get(1).sel == 1 and ident.keys().size() == 0 and Safety)
    .then(publish)
    .then(read_identity(1))
    .expect(conn.get(1) == "identified" and ident.get(taken.get(1).sel) == 1 and not(guard) and Safety)
}

// Main instance: the rules. TLC checks Safety exhaustively; also hosts the two evaluator-only phase runs.
module receipt_ok { import selection_receipt(OFF = Set(), REC = false, MAXUPD = 2).* from "./selection_receipt" }
// Conformance main (D-PASS7-2): real-owner-drivable actions only; records `conf_step`; only the named runs are checked.
module receipt_conf { import selection_receipt(OFF = Set(), REC = true, MAXUPD = 3).* from "./selection_receipt" }
