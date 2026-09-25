// SPIKE (flat-cluster-intent-log, increment A) — protocol-level model of Viewstamped Replication with
// viewstamp's LIVE SingleChange reconfiguration, as implemented at viewstamp-proto@f7fbcd96 (NOT the
// whole-config epochs of VR Revisited §7). Throwaway probe code.
//
// Modelled behaviour (each item is pinned to the viewstamp source it abstracts):
//   normal op     primary mints op, Prepare -> all members, voters PrepareOk; commit when
//                 |acks ∩ voters| >= quorum(cfg) AND (for a voter-set shrink) the successor-quorum
//                 conjunction (normal.rs try_commit / shrink_successor_quorum_met). Backups learn the
//                 commit from Prepare.commit / Commit heartbeats.
//   reconfig      propose_membership (endpoint/reconfigure.rs): primary + Normal; single change in
//                 flight (a Reconfigure op above commit fences BOTH client ops and a 2nd change — the
//                 "Reconfigure op is the last op of its epoch" rule, normal.rs check_new_op_admission);
//                 f-reduction refused without AcceptReducedFaultTolerance; PromoteLearner gated on a
//                 FRESH RequestLearnerProof/LearnerProof whose frontier (the learner's commit_min)
//                 covers the head.
//   epoch swap    COMMIT-FIRST: installing the successor happens when a replica commits the Reconfigure
//                 op whose pinned predecessor is its current config (normal.rs commit_reconfigure). The
//                 the successor epoch starts at view/log_view 0, so predecessor primaryship cannot
//                 remap into a successor view the new group never formed.
//                 Messages emitted in the same step carry the PRE-install epoch (the SwapEpoch root
//                 lands asynchronously after the emit).
//   view change   TigerBeetle-style: SVC quorum = quorum_view_change = n - quorum + 1 (flexible),
//                 DVC to primary(view); canonical log = max log_view, max op, commit* = max commit,
//                 nack truncation at the (n - q + 1)-th smallest head (view_change.rs
//                 select_canonical_log). Learners never vote; they adopt StartView.
//   catch-up      a replica behind in VIEW (same epoch) adopts the primary's StartView (GetView path,
//                 recovery/mod.rs catch_up_to_view -> on_get_view). adopt_canonical_head ASSERTS the
//                 adopted head >= own commit_min (a violation is a fail-stop panic in viewstamp).
//   crossing      a replica behind in EPOCH state-syncs from a donor's durable CHECKPOINT (served once it
//                 covers the donor's install op) and installs the donor's config;
//                 the successor epoch begins durably at VIEW 0 / LOG_VIEW 0 and predecessor-epoch
//                 view-change standing is abandoned (VR Revisited §8.3).
//   crash/restart durable: cfg, view, log_view, log, commit. Volatile: acks, SVC/DVC tallies, proof.
//                 A restarted Normal primary abdicates to view+1; one crashed mid view-change re-drives.
//   add learner   a brand-new node is bootstrapped out of band with the installed successor membership
//                 and an empty log (the interim path of viewstamp issue #84).
module vr_sc {
  import vr_common.* from "./vr_common"

  // ---------------------------------------------------------------------------------------------
  // Parameters
  // ---------------------------------------------------------------------------------------------
  const NODES: Set[int]
  const GENESIS: Cfg
  const MAX_LOG: int
  const MAX_VIEW: int
  const CHECKPOINT_OPS: int
  // Cross-epoch view reset feature switch retained for the historical as-is/fixed comparison modules.
  // The primary topology modules set this true: crossing starts the successor at view/log_view 0 and
  // carries no predecessor-epoch SVC/DVC standing (VR Revisited §8.3 / §7.1.1).
  const CROSS_FIX: bool

  // ---------------------------------------------------------------------------------------------
  // Types
  // ---------------------------------------------------------------------------------------------
  // kind: "client" | "reconfig". `cfg`/`prevEpoch` are meaningful for "reconfig" only.
  type Entry = { kind: str, label: int, cfg: Cfg, prevEpoch: int, ack: bool, reduces: bool }

  // kind: "prepare" | "prepareOk" | "commit" | "svc" | "dvc" | "sv" | "reqProof" | "proof"
  type Msg = {
    kind: str, src: int, dst: int, view: int, epoch: int, op: int, commit: int,
    logView: int, log: List[Entry], frontier: int, srcVoter: bool,
    // the learner-proof challenge nonce (RequestLearnerProof / LearnerProof); 0 on every other kind
    nonce: int,
    // the SENDER's checkpoint_op at emission (Prepare / Commit carry it; 0 on every other kind)
    ckpt: int,
  }

  // status: "normal" | "vc" | "retired"
  type NState = {
    id: int, started: bool, up: bool, status: str,
    cfg: Cfg, view: int, logView: int, log: List[Entry], commit: int,
    // commit_max: the highest commit number learned (applied up to min(cmax, head) — commit_min)
    cmax: int,
    // the commit recorded in the node's last DURABLE root (view / StartView-adoption / SwapEpoch /
    // checkpoint / sync roots record max(commit_max, checkpoint_op)); recovery restarts from it
    dcommit: int,
    // the ops the durable root carries a sparse CANONICAL HEADER for: the committed-band ops its writer HELD
    // (as a body or a header-only entry) when it was written (<= min(commit_max, head)); a HEADERLESS hole is
    // not held, so it has none. A written slot at/below the durable commit WITHOUT one reads back
    // StaleCommitted (classify_committed_slot arm 2: "the writer did not hold this committed op — unproven"),
    // and a header-only op WITH one survives a crash as a repair hole.
    hdrs: Set[int],
    // the durable CHECKPOINT op and the durable root's config_install_op (the committed Reconfigure op that
    // installed the current config). Checkpoints fire on the commit-advance tail at
    // checkpoint_op + checkpoint_ops (maybe_checkpoint), and a swap owes a checkpoint at >= the install op
    // (the SwapEpoch-landing force / maybe_pay_checkpoint_debt): `inst > ckpt` IS the debt.
    ckpt: int, inst: int,
    // the CURRENT config was installed by the out-of-band bootstrap (#84 interim: the membership is preinstalled
    // but NOT its config_id lineage / config_install_op). Such a node cannot DONATE a cross-epoch crossing: the
    // receiver's apply_sync rejects its successor payload (to_membership_verified Err). Cleared by any verified
    // install (a commit-first swap, or a crossing from a non-bootstrapped donor).
    boot: bool,
    // the backup idle (primary_idle) timer is armed: only after primary contact or a status transition
    // (genesis arms NO timer — `Timers::default()`; `note_primary_contact` / `arm_timers` arm it)
    idle: bool,
    // ops held as HEADER ONLY (viewstamp's view-change carriers are header-only; bodies are repaired
    // peer-to-peer from a Normal holder before they can be acked or applied)
    holes: Set[int],
    // REGISTERED repair holes (viewstamp `self.repair`): a header-only / faulty op becomes a registered hole
    // only when advance_commit walks onto it (request_repair_run registers the contiguous hole run up to
    // min(commit_max, head)). Only registered holes are re-solicited by a backup (repair_timeouts), filled by
    // fill_repair (which then re-applies up to commit_max), and gate the forced sync (maybe_force_sync). An
    // UNREGISTERED hole filled by a Prepare goes through ordinary on_prepare (advance only to p.commit).
    // Volatile; cleared by adoption / formation / sync.
    reg: Set[int],
    // WRITTEN slots whose WAL body DIFFERS from the log's (canonical) entry at that op: an adoption / formation
    // that replaces op i with a different entry, which the node holds only header-only, leaves the old body in
    // the WAL. On recovery, classify_committed_slot arm 1 (canonical header present, identity mismatch) reads
    // it StaleCommitted; a written slot whose body MATCHES a canonical header is Verified (present again).
    wmis: Set[int],
    // holes held with NO canonical header in memory: a faulty slot recovery DROPPED (its placeholder is not
    // re-seeded unless the durable root carries its canonical header, i.e. op in hdrs). fill_repair accepts a
    // body for such a hole only when the Prepare vouches it committed (p.commit >= p.op); a hole from an
    // adoption / formation carries its canonical header and is verified by identity instead.
    nohdr: Set[int],
    // the VIEW stamped on this node's WAL header for each op it holds a body for (parallel to `log`; -1 for a
    // header-only op). Recovery (classify_committed_slot) DROPS a held op above the durable commit whose
    // stamped view is below the durable log_view ("a superseded earlier-view proposal"). Stamps: a mint /
    // a Prepare append / an adoption or formation re-append (adopt_append) stamp the CURRENT view; a repair
    // fill (fill_repair) stamps the SERVER's view carried by the served Prepare (Prepare::new(self.view..)).
    stamp: List[int],
    // new-primary repair-or-truncate tally: (op, voter that nacked it in this view)
    nacks: Set[(int, int)],
    svcTarget: int, svcFrom: Set[int], dvcs: Set[Msg], acks: Set[(int, int)],
    proofTarget: int, proofAt: int, proofFrontier: int,
    // the outstanding challenge's nonce (on_learner_proof accepts only a reply carrying it)
    proofNonce: int,
    // an ARMED same-epoch state sync (viewstamp `self.sync`): its target (0 = none) and whether it is FORCED.
    // While armed, an ordinary Prepare is dropped (on_prepare returns). Volatile; cancelled by a view
    // transition (reset_for_view_transition). Completed by `completeSync`.
    sync: int, syncForced: bool,
    // the peer-checkpoint floor this backup has learned (the primary's Commit reports its checkpoint_op);
    // volatile, dropped on a view transition
    pfloor: int,
  }

  pure val NO_ENTRY: Entry =
    { kind: "none", label: 0, cfg: GENESIS, prevEpoch: -1, ack: false, reduces: false }
  pure val NO_MSG: Msg = {
    kind: "none", src: -1, dst: -1, view: 0, epoch: 0, op: 0, commit: 0,
    logView: 0, log: [], frontier: -1, srcVoter: false, nonce: 0, ckpt: 0,
  }

  // ---------------------------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------------------------
  var st: int -> NState
  var msgs: Set[Msg]
  var nextLabel: int
  // ghosts (verification only)
  var ghostLog: List[Entry]            // longest committed prefix ever observed
  var ghostAt: List[int]               // ev = epoch*(MAX_VIEW+1)+view at which ghostLog[i] FIRST committed
  var ghostOk: bool                    // every committed prefix ever observed extends ghostLog
  var committers: Set[(int, int, int)] // (epoch, view, node) that advanced commit AS PRIMARY
  var promoteOk: bool                  // every minted PromoteLearner's target durably held the head
  var adoptOk: bool                    // no StartView adoption below own commit (viewstamp: panic)
  var commitStarOk: bool               // no canonical-log formation with commit* > op_head (panic)
  var faults: int                      // crashes injected so far (exploration budget for stepChurn only)
  var nonceCtr: int                    // ghost: challenge nonces are unique across nodes and incarnations

  // ---------------------------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------------------------
  pure def imax(a: int, b: int): int = if (a > b) a else b
  pure def imin(a: int, b: int): int = if (a < b) a else b

  pure def lenOf(s: NState): int = s.log.length()

  pure def entryAt(s: NState, op: int): Entry = s.log[op - 1]

  pure def committedOf(s: NState): List[Entry] = s.log.slice(0, imin(s.commit, s.log.length()))

  pure def isPrefix(a: List[Entry], b: List[Entry]): bool =
    a.length() <= b.length() and a.indices().forall(i => a[i] == b[i])

  pure def consistent(a: List[Entry], b: List[Entry]): bool = isPrefix(a, b) or isPrefix(b, a)

  pure def voterSelf(s: NState): bool = isVoter(s.cfg, s.id)

  pure def primarySelf(s: NState): bool = isVoter(s.cfg, s.id) and isPrimaryIn(s.cfg, s.view, s.id)

  pure def live(s: NState): bool = s.started and s.up and s.status != "retired"

  // An uncommitted Reconfigure op in the log: the single-change fence (has_pending_reconfigure).
  pure def pendingReconfig(s: NState): bool =
    s.log.indices().exists(i => i + 1 > s.commit and s.log[i].kind == "reconfig")

  // A durable root write (view change entry, new-primary formation, StartView adoption, SwapEpoch,
  // checkpoint): the root records commit = commit_max.
  // (the root records max(commit_max, checkpoint_op) AT WRITE TIME — commit_max is not monotone: a new
  // primary's formation overwrites it with commit*)
  pure def persist(s: NState): NState =
    s.with("dcommit", imax(s.cmax, s.ckpt))
     .with("hdrs", 0.to(MAX_LOG).filter(op => op >= 1 and op <= imin(imax(s.cmax, s.ckpt), s.log.length())
                                             and not(s.nohdr.contains(op))))

  // check_new_op_admission's CommitGap fence: no mint while a committed op is unapplied (commit_max >
  // commit_min) or a repair hole is outstanding.
  pure def admits(s: NState): bool = s.cmax <= s.commit and s.holes.size() == 0

  // Installing a successor configuration: every epoch has an independent view-number space, so the
  // durable/live identity resets to view/log_view 0 and predecessor SVC/DVC standing is abandoned.
  // A node absent from the successor retires; the learner proof is cleared.
  pure def install(s: NState, c: Cfg): NState =
    val reset = persist(s.with("view", 0).with("logView", 0).with("svcTarget", 0)
                         .with("svcFrom", Set()).with("dvcs", Set()))
    if (not(isMember(c, s.id)))
      reset.with("cfg", c).with("status", "retired")
    else
      reset.with("cfg", c).with("status", "normal").with("boot", false).with("idle", true)
        .with("proofTarget", -1).with("proofAt", -1).with("proofFrontier", -1)

  // Commit exactly `op` (== s.commit + 1): a committed Reconfigure op pinned to the current config
  // installs the successor (commit-first swap).
  pure def commitOne(s: NState, op: int): NState =
    val e = entryAt(s, op)
    // commit_max >= commit_min always (commit_reconfigure lifts it before staging the SwapEpoch root)
    val s0 = s.with("commit", op).with("cmax", imax(s.cmax, op))
    if (e.kind == "reconfig" and e.prevEpoch == s.cfg.epoch) install(s0.with("inst", op), e.cfg) else s0

  // The commit-advance-tail checkpoint (maybe_checkpoint / maybe_pay_checkpoint_debt): only in a settled
  // Normal view (log_view == view), when the cadence boundary is reached or a swap checkpoint is owed.
  pure def ckptTail(s: NState): NState =
    if (s.status == "normal" and s.logView == s.view
        and (s.commit >= s.ckpt + CHECKPOINT_OPS or (s.inst > s.ckpt and s.commit >= s.inst)))
      persist(s).with("ckpt", s.commit)
    else s

  // complete_recovery's no-traffic kick: pay ONLY an owed swap checkpoint (not the ordinary cadence)
  pure def ckptDebt(s: NState): NState =
    if (s.status == "normal" and s.logView == s.view and s.inst > s.ckpt and s.commit >= s.inst)
      persist(s).with("ckpt", s.commit)
    else s

  // Advance the commit frontier towards `target`, one op at a time (normal.rs advance_commit: commit_max is
  // lifted to max(commit_max, target), but the apply loop runs only while commit_min < TARGET — a known
  // commit_max above the carried target is not applied by this call).
  pure def advanceTo(s0: NState, target0: int): NState =
    val s = s0.with("cmax", imax(s0.cmax, target0))
    val target = target0
    val a = range(0, MAX_LOG).foldl(s, (acc, i) =>
      if (i + 1 == acc.commit + 1 and i + 1 <= target and i + 1 <= acc.log.length()
          and acc.status != "retired" and not(acc.holes.contains(i + 1)))
        commitOne(acc, i + 1)
      else acc
    )
    // request_repair_run: the apply loop stopped at a hole at commit+1 within target and the head -> register
    // the contiguous hole run [lo .. min(commit_max, head)]
    val lo = a.commit + 1
    if (a.status != "retired" and lo <= target and lo <= a.log.length() and a.holes.contains(lo))
      a.with("reg", a.reg.union(0.to(MAX_LOG).filter(op => op >= lo and op <= imin(a.cmax, a.log.length())
                                  and 0.to(MAX_LOG).filter(k => k >= lo and k <= op).forall(k => a.holes.contains(k)))))
    else a

  pure def ackCount(s: NState, op: int): int =
    setOf(s.cfg.voters).filter(n => s.acks.contains((op, n))).size()

  // shrink_successor_quorum_met: a voter-set-shrinking Reconfigure op additionally needs a successor
  // quorum among the RETAINED voters' acks.
  pure def shrinkOk(s: NState, op: int): bool =
    val e = entryAt(s, op)
    if (e.kind == "reconfig" and nVoters(e.cfg) < nVoters(s.cfg))
      setOf(s.cfg.voters).intersect(setOf(e.cfg.voters))
        .filter(n => s.acks.contains((op, n))).size() >= quorum(e.cfg)
    else true

  // try_commit on the primary: commit the longest contiguous quorum-acked prefix. Stops as soon as the
  // node is no longer a Normal primary (e.g. a committed demotion of itself).
  pure def tryCommit(s: NState): NState = ckptTail(tryCommit0(s))

  // try_commit without the checkpoint tail (a new primary's formation: its StartViewAsPrimary durable-view
  // write is in flight, which excludes a checkpoint)
  pure def tryCommit0(s: NState): NState =
    range(0, MAX_LOG).foldl(s, (acc, i) =>
      if (i + 1 == acc.commit + 1 and i + 1 <= acc.log.length() and acc.status == "normal"
          and primarySelf(acc) and ackCount(acc, i + 1) >= quorum(acc.cfg) and shrinkOk(acc, i + 1)
          and not(acc.holes.contains(i + 1)))
        commitOne(acc, i + 1)
      else acc
    )

  pure def bcast(s: NState, targets: Set[int], m: Msg): Set[Msg] =
    targets.exclude(Set(s.id)).map(d => m.with("dst", d))

  // PrepareOk for every op in (commit, head] — a voter's acks after adopting a canonical head.
  pure def acksFor(s: NState, dest: int): Set[Msg] =
    if (not(voterSelf(s))) Set()
    else
      0.to(MAX_LOG - 1).filter(i => i + 1 > imax(s.commit, s.cmax) and i + 1 <= s.log.length()
                                    and not(s.holes.contains(i + 1)))
        .map(i => { ...NO_MSG, kind: "prepareOk", src: s.id, dst: dest, view: s.view,
                    epoch: s.cfg.epoch, op: i + 1, srcVoter: true })

  // The DVC this node casts for its current view.
  pure def dvcOf(s: NState): Msg =
    { ...NO_MSG, kind: "dvc", src: s.id, dst: primaryOf(s.cfg, s.view), view: s.view,
      epoch: s.cfg.epoch, op: s.log.length(), commit: imin(s.cmax, s.log.length()),
      logView: s.logView, log: s.log, srcVoter: voterSelf(s) }

  // Enter ViewChange for `v` (enter_view_change): reset volatile tallies, cast the DVC.
  pure def enterVc(s: NState, v: int): (NState, Set[Msg]) =
    val s1 = resetSync(persist(s)).with("view", v).with("status", "vc").with("svcTarget", v).with("svcFrom", Set())
      .with("dvcs", Set()).with("acks", Set())
    // send_do_view_change: `if !self.is_voter() { return; }` — a non-voter casts no DVC.
    (s1, if (voterSelf(s1)) Set(dvcOf(s1)) else Set())

  // select_canonical_log over a DVC set, for a configuration `c`. Returns (log, opHead, commitStar).
  pure def canonical(dvcs: Set[Msg], c: Cfg): (List[Entry], int, int) =
    val lvStar = dvcs.fold(0, (a, d) => imax(a, d.logView))
    val canon = dvcs.filter(d => d.logView == lvStar)
    val opHead0 = canon.fold(0, (a, d) => imax(a, d.op))
    val commitStar = dvcs.fold(0, (a, d) => imax(a, d.commit))
    // nack truncation: the threshold-th smallest donor head (threshold = quorum_nack_prepare).
    val threshold = quorumVc(c)
    val heads = dvcs.map(d => d.op)
    val kth = heads.filter(h => dvcs.filter(d => d.op <= h).size() >= threshold)
                   .fold(MAX_LOG + 1, (a, h) => imin(a, h))
    val crossOp = imax(kth + 1, commitStar + 1)
    val opHead = if (dvcs.size() >= threshold and crossOp <= opHead0) crossOp - 1 else opHead0
    // every canonical donor agrees on the ops it holds; take the longest canonical log.
    val best = canon.fold(NO_MSG, (a, d) => if (d.op > a.op or a.kind == "none") d else a)
    (best.log.slice(0, imin(opHead, best.log.length())), opHead, commitStar)

  // reset_for_view_transition (and a crash): cancel an in-flight sync, drop the peer-checkpoint reports.
  pure def resetSync(s: NState): NState = s.with("sync", 0).with("syncForced", false).with("pfloor", 0)

  // maybe_request_sync (on_commit / on_prepare, Normal backup): a checkpoint above the head arms an ORDINARY
  // sync to it, or raises an outstanding sync's target.
  pure def maybeReqSync(s: NState, incoming: int): NState =
    if (s.status == "normal" and not(primarySelf(s)) and incoming > s.log.length())
      (if (s.sync == 0) s.with("sync", incoming).with("syncForced", false)
       else if (incoming > s.sync) s.with("sync", incoming)
       else s)
    else s

  // maybe_force_sync (Normal backup): a REGISTERED hole at/below the peer-checkpoint floor is snapshot-only on
  // its reporter -> clear those holes and arm (or raise to) a FORCED sync to the floor (a sync already at or
  // above the floor is left to run).
  pure def maybeForce(s: NState): NState =
    val floor = imax(s.ckpt, s.pfloor)
    if (s.status == "normal" and not(primarySelf(s)) and floor > 0 and s.reg.exists(op => op <= floor))
      (if (s.sync == 0 or floor > s.sync)
         s.with("reg", s.reg.filter(op => op > floor)).with("sync", floor).with("syncForced", true)
       else s.with("reg", s.reg.filter(op => op > floor)))
    else s

  // Ghost bookkeeping after a transition: the longest committed prefix must extend what we saw.
  pure def ghostAfter(nst: int -> NState, g: List[Entry]): (List[Entry], bool) =
    val prefs = NODES.map(n => committedOf(nst.get(n)))
    val longest = prefs.fold(g, (a, p) => if (p.length() > a.length()) p else a)
    (longest, isPrefix(g, longest) and prefs.forall(p => isPrefix(p, longest)))

  // ---------------------------------------------------------------------------------------------
  // The action record (quint-connect reads the action name + its arguments from `lastAction`, so the
  // SAME driver replays random `quint run` traces and scripted `quint test` runs).
  // ---------------------------------------------------------------------------------------------
  type Act =
    | Init
    | ClientRequest({ p: int })
    | DeliverPrepare({ m: Msg })
    | DeliverPrepareOk({ m: Msg })
    | DeliverCommit({ m: Msg })
    | Heartbeat({ p: int })
    | Timeout({ n: int })
    | DeliverSvc({ m: Msg })
    | DeliverDvc({ m: Msg })
    | DeliverSv({ m: Msg })
    | CatchUpView({ n: int, p: int })
    | Crash({ n: int })
    | Restart({ n: int })
    | RequestProof({ p: int, l: int })
    | DeliverReqProof({ m: Msg })
    | DeliverProof({ m: Msg })
    | Propose({ p: int, kind: str, who: int, ack: bool })
    | Bootstrap({ x: int, d: int })
    | SyncFrom({ n: int, d: int, cp: int })
    | RepairExchange({ n: int, h: int })
    | CompleteSync({ n: int, d: int })
    | RecoverHead({ n: int, p: int })
    | Idle
    | ProposeRefused({ p: int, kind: str, who: int })

  var lastAction: Act

  // ---------------------------------------------------------------------------------------------
  // Transition plumbing: every action goes through `commitStep`, which updates the ghosts.
  // ---------------------------------------------------------------------------------------------
  action commitStep(nst: int -> NState, out: Set[Msg], primaryCommit: Set[(int, int, int)],
                    promote: bool, adopt: bool, cstar: bool, labelInc: int, act: Act): bool =
    val g = ghostAfter(nst, ghostLog)
    // the (epoch, view) of the primary whose commit first exposed the new ghost indices
    val cev = primaryCommit.fold(-1, (a, c) => imax(a, c._1 * (MAX_VIEW + 1) + c._2))
    // (constant-bounded range so Apalache can unroll it)
    val grown = range(0, MAX_LOG).foldl(ghostAt, (acc, i) =>
      if (i >= ghostLog.length() and i < g._1.length()) acc.append(cev) else acc)
    all {
      st' = nst,
      msgs' = msgs.union(out),
      nextLabel' = nextLabel + labelInc,
      ghostLog' = g._1,
      ghostAt' = grown,
      ghostOk' = (ghostOk and g._2),
      committers' = committers.union(primaryCommit),
      promoteOk' = (promoteOk and promote),
      adoptOk' = (adoptOk and adopt),
      commitStarOk' = (commitStarOk and cstar),
      faults' = faults + (match act { | Crash(_) => 1 | _ => 0 }),
      nonceCtr' = nonceCtr + (match act { | RequestProof(_) => 1 | _ => 0 }),
      lastAction' = act,
    }

  // Commit messages a primary emits after its commit advanced (carrying the PRE-install epoch/view).
  pure def commitMsgs(before: NState, after: NState): Set[Msg] =
    if (after.commit > before.commit)
      bcast(before, members(before.cfg),
            { ...NO_MSG, kind: "commit", src: before.id, view: before.view,
              epoch: before.cfg.epoch, commit: after.commit, srcVoter: true, ckpt: before.ckpt })
    else Set()

  pure def committerMark(before: NState, after: NState): Set[(int, int, int)] =
    if (after.commit > before.commit) Set((before.cfg.epoch, before.view, before.id)) else Set()

  // ---------------------------------------------------------------------------------------------
  // Init
  // ---------------------------------------------------------------------------------------------
  pure def genesisState(n: int): NState = {
    id: n, started: isMember(GENESIS, n), up: isMember(GENESIS, n), status: "normal",
    cfg: GENESIS, view: 0, logView: 0, log: [], commit: 0, cmax: 0, dcommit: 0, hdrs: Set(), ckpt: 0, inst: 0, boot: false,
    idle: false,
    holes: Set(), reg: Set(), wmis: Set(), nohdr: Set(), nacks: Set(), stamp: [],
    svcTarget: 0, svcFrom: Set(), dvcs: Set(), acks: Set(),
    proofTarget: -1, proofAt: -1, proofFrontier: -1, proofNonce: 0,
    sync: 0, syncForced: false, pfloor: 0,
  }

  action init = all {
    st' = NODES.mapBy(n => genesisState(n)),
    msgs' = Set(),
    nextLabel' = 1,
    ghostLog' = [],
    ghostAt' = [],
    ghostOk' = true,
    committers' = Set(),
    promoteOk' = true,
    adoptOk' = true,
    commitStarOk' = true,
    faults' = 0,
    nonceCtr' = 0,
    lastAction' = Init,
  }

  // ---------------------------------------------------------------------------------------------
  // Normal operation
  // ---------------------------------------------------------------------------------------------
  action clientRequestM(p: int): bool =
    val s = st.get(p)
    all {
      live(s), s.status == "normal", primarySelf(s), not(pendingReconfig(s)), admits(s),
      s.log.length() < MAX_LOG,
      val e = { ...NO_ENTRY, kind: "client", label: nextLabel }
      val op = s.log.length() + 1
      val s1 = s.with("log", s.log.append(e)).with("stamp", s.stamp.append(s.view))
                .with("acks", s.acks.union(Set((op, p))))
      val s2 = tryCommit(s1)
      val prep = bcast(s, members(s.cfg),
                       { ...NO_MSG, kind: "prepare", src: p, view: s.view, epoch: s.cfg.epoch,
                         op: op, commit: s.commit, log: [e], srcVoter: true, ckpt: s.ckpt })
      commitStep(st.set(p, s2), prep.union(commitMsgs(s1, s2)), committerMark(s1, s2),
                 true, true, true, 1, ClientRequest({ p: p })),
    }

  // A Prepare DROPPED by an armed state sync (on_prepare: `if self.sync.is_some() ... return`).
  action deliverPrepareDropM(m: Msg): bool =
    val s = st.get(m.dst)
    val e = m.log[0]
    val regHole = s.holes.contains(m.op) and s.reg.contains(m.op) and m.op <= s.log.length()
                  and entryAt(s, m.op) == e
    // fill_repair accepts it (identity vs a kept header, or a committed-vouched body for a headerless hole)
    val regFill = regHole and (not(s.nohdr.contains(m.op)) or m.commit >= m.op)
    // on_prepare after fill_repair: note_primary_contact + maybe_request_sync; an ARMED sync then DROPS it
    val sArm = maybeReqSync(s.with("idle", true), m.ckpt)
    all {
      m.kind == "prepare", msgs.contains(m),
      live(s), s.status == "normal", s.cfg.epoch == m.epoch, s.view == m.view,
      not(primarySelf(s)),
      or {
        m.op == s.log.length() + 1,                                  // in-order append
        m.op <= s.log.length() and entryAt(s, m.op) == e,            // duplicate: re-ack
      },
      not(regHole) and sArm.sync > 0 and sArm != s,
      commitStep(st.set(m.dst, sArm), Set(), Set(), true, true, true, 0, DeliverPrepare({ m: m })),
    }

  // The ordinary / fill path (fill_repair of a registered hole runs BEFORE the sync check; otherwise only
  // when no sync is armed).
  action deliverPrepareM(m: Msg): bool =
    val s = st.get(m.dst)
    val e = m.log[0]
    val regHole = s.holes.contains(m.op) and s.reg.contains(m.op) and m.op <= s.log.length()
                  and entryAt(s, m.op) == e
    // fill_repair accepts it (identity vs a kept header, or a committed-vouched body for a headerless hole)
    val regFill = regHole and (not(s.nohdr.contains(m.op)) or m.commit >= m.op)
    val sArm = maybeReqSync(s.with("idle", true), m.ckpt)
    all {
      m.kind == "prepare", msgs.contains(m),
      live(s), s.status == "normal", s.cfg.epoch == m.epoch, s.view == m.view,
      not(primarySelf(s)),
      or {
        m.op == s.log.length() + 1,                                  // in-order append
        m.op <= s.log.length() and entryAt(s, m.op) == e,            // duplicate: re-ack
      },
      regFill or (not(regHole) and sArm.sync == 0),
      val s1 = (if (m.op == s.log.length() + 1)
                  s.with("log", s.log.append(e)).with("stamp", s.stamp.append(m.view))
                else if (s.holes.contains(m.op)) s.with("stamp", s.stamp.replaceAt(m.op - 1, m.view))
                else s)
                 .with("idle", true).with("holes", s.holes.exclude(Set(m.op))).with("reg", s.reg.exclude(Set(m.op)))
                 .with("wmis", s.wmis.exclude(Set(m.op))).with("nohdr", s.nohdr.exclude(Set(m.op)))
      // on_prepare: advance_commit(p.commit); an in-order APPEND — or a Prepare that FILLS a REGISTERED hole
      // (fill_repair: "a filled op at/below commit_max is already known committed -> advance_commit") —
      // then re-applies up to commit_max; an unregistered hole filled here advances only to p.commit
      val s2 = ckptTail(maybeForce(advanceTo(s1, if (m.op == s.log.length() + 1 or regFill)
                                                   imax(m.commit, s.cmax) else m.commit)))
      val ack = if (voterSelf(s1)) Set({ ...NO_MSG, kind: "prepareOk", src: s.id, dst: m.src,
                                         view: s.view, epoch: s.cfg.epoch, op: m.op,
                                         srcVoter: true })
                else Set()
      commitStep(st.set(m.dst, s2), ack, Set(), true, true, true, 0, DeliverPrepare({ m: m })),
    }

  action deliverPrepareOkM(m: Msg): bool =
    val s = st.get(m.dst)
    all {
      m.kind == "prepareOk", msgs.contains(m),
      live(s), s.status == "normal", primarySelf(s), s.view == m.view, s.cfg.epoch == m.epoch,
      isVoter(s.cfg, m.src), m.op <= s.log.length(), not(s.acks.contains((m.op, m.src))),
      val s1 = s.with("acks", s.acks.union(Set((m.op, m.src))))
      val s2 = tryCommit(s1)
      commitStep(st.set(m.dst, s2), commitMsgs(s1, s2), committerMark(s1, s2), true, true, true, 0,
                 DeliverPrepareOk({ m: m })),
    }

  action deliverCommitM(m: Msg): bool =
    val s = st.get(m.dst)
    all {
      m.kind == "commit", msgs.contains(m),
      live(s), s.status == "normal", s.cfg.epoch == m.epoch, s.view == m.view,
      not(primarySelf(s)),
      // on_commit: record the primary's checkpoint (peer floor), maybe_request_sync, maybe_force_sync, then
      // advance_commit (which may register holes -> request_repair_run -> maybe_force_sync again)
      val s0 = s.with("idle", true).with("pfloor", imax(s.pfloor, m.ckpt))
      val s1 = maybeForce(maybeReqSync(s0, m.ckpt))
      val s2 = ckptTail(maybeForce(advanceTo(s1, m.commit)))
      all {
        s2 != s,
        commitStep(st.set(m.dst, s2), Set(), Set(), true, true, true, 0, DeliverCommit({ m: m })),
      },
    }

  // The primary's periodic Commit heartbeat (re-advertises its commit at its CURRENT epoch).
  action heartbeatM(p: int): bool =
    val s = st.get(p)
    val hb = bcast(s, members(s.cfg),
                   { ...NO_MSG, kind: "commit", src: p, view: s.view, epoch: s.cfg.epoch,
                     commit: s.commit, srcVoter: true, ckpt: s.ckpt })
    all {
      live(s), s.status == "normal", primarySelf(s), not(hb.subseteq(msgs)),
      commitStep(st.set(p, ckptTail(s)), hb, Set(), true, true, true, 0, Heartbeat({ p: p })),
    }

  // ---------------------------------------------------------------------------------------------
  // View change
  // ---------------------------------------------------------------------------------------------
  // propose_next_view (forfeit.rs): target = view + 1; raise the SVC target (resetting the tally) only if
  // above it; then join (own bit + broadcast of the CURRENT target).
  action timeoutM(n: int): bool =
    val s = st.get(n)
    val target = s.view + 1
    val s1 = if (target > s.svcTarget) s.with("svcTarget", target).with("svcFrom", Set(n))
             else s.with("svcFrom", s.svcFrom.union(Set(n)))
    all {
      live(s), voterSelf(s), s.status == "vc" or (s.status == "normal" and not(primarySelf(s)) and s.idle),
      s1.svcTarget <= MAX_VIEW,
      val svc = bcast(s, setOf(s.cfg.voters),
                      { ...NO_MSG, kind: "svc", src: n, view: s1.svcTarget, epoch: s.cfg.epoch,
                        srcVoter: true })
      if (s1.svcFrom.size() >= quorumVc(s.cfg) and s1.svcTarget > s.view) {
        val r = enterVc(s1, s1.svcTarget)
        commitStep(st.set(n, r._1), svc.union(r._2), Set(), true, true, true, 0, Timeout({ n: n }))
      } else commitStep(st.set(n, s1), svc, Set(), true, true, true, 0, Timeout({ n: n })),
    }

  action deliverSvcM(m: Msg): bool =
    val s = st.get(m.dst)
    all {
      m.kind == "svc", msgs.contains(m),
      live(s), voterSelf(s), s.status != "rhead", s.cfg.epoch == m.epoch, isVoter(s.cfg, m.src),
      m.view > s.view, m.view >= s.svcTarget,
      val raise = m.view > s.svcTarget
      val s1 = if (raise) s.with("svcTarget", m.view).with("svcFrom", Set(s.id, m.src))
               else s.with("svcFrom", s.svcFrom.union(Set(m.src)))
      val own = if (raise) bcast(s, setOf(s.cfg.voters),
                                 { ...NO_MSG, kind: "svc", src: s.id, view: m.view,
                                   epoch: s.cfg.epoch, srcVoter: true })
                else Set()
      if (s1.svcFrom.size() >= quorumVc(s.cfg)) {
        val r = enterVc(s1, s1.svcTarget)
        commitStep(st.set(m.dst, r._1), own.union(r._2), Set(), true, true, true, 0,
                   DeliverSvc({ m: m }))
      } else commitStep(st.set(m.dst, s1), own, Set(), true, true, true, 0, DeliverSvc({ m: m })),
    }

  action deliverDvcM(m: Msg): bool =
    val p = m.dst
    val s = st.get(p)
    all {
      m.kind == "dvc", msgs.contains(m),
      live(s), s.status == "vc", s.view == m.view, s.cfg.epoch == m.epoch, primarySelf(s),
      isVoter(s.cfg, m.src),
      val withOwn = if (s.dvcs.exists(d => d.src == p)) s.dvcs else s.dvcs.union(Set(dvcOf(s)))
      val prior = withOwn.filter(d => d.src == m.src)
      val replace = prior.forall(d => m.logView > d.logView or (m.logView == d.logView and m.op > d.op))
      val dvcs2 = if (replace) withOwn.exclude(prior).union(Set(m)) else withOwn
      val s1 = s.with("dvcs", dvcs2)
      if (dvcs2.size() >= quorumVc(s.cfg)) {
        val c = canonical(dvcs2, s.cfg)
        val log2 = c._1
        val commitStar = c._3
        if (commitStar > c._2 or commitStar > log2.length()) {
          // viewstamp: `assert!(commit_star <= op_head, "VSR safety invariant violated ...")` — the new
          // primary FAIL-STOPS (flagged).
          commitStep(st.set(p, died(s1)), Set(), Set(), true, true, false, 0, DeliverDvc({ m: m }))
        } else {
          // start_view_as_new_primary does NOT run reset_for_view_transition: SVC bits collected for a
          // target above this view during the view change (svc_from) survive into Normal
          val hs2 = holesAfter(s.log, s.holes, log2)
          // start_view_as_new_primary OVERWRITES commit_max with commit* ("must OVERWRITE, not max")
          val formed = resetSync(s1).with("log", log2).with("logView", s.view).with("status", "normal")
            .with("dvcs", Set()).with("cmax", commitStar)
            .with("commit", imin(s.commit, log2.length()))
            .with("holes", hs2).with("reg", Set()).with("nacks", Set()).with("stamp", restamp(log2, hs2, s.view, s.stamp))
            .with("wmis", wmisAfter(s.log, s.stamp, s.wmis, log2, hs2)).with("nohdr", Set())
          val s2 = persist(advanceTo(formed, commitStar))
          val own = 0.to(MAX_LOG - 1).filter(i => i + 1 > s2.commit and i + 1 <= log2.length()
                                               and not(s2.holes.contains(i + 1)))
                      .map(i => (i + 1, p))
          val s3 = s2.with("acks", own)
          val s4 = tryCommit0(s3)
          val sv = bcast(s, members(s.cfg),
                         { ...NO_MSG, kind: "sv", src: p, view: s.view, epoch: s.cfg.epoch,
                           op: log2.length(), commit: s2.cmax, log: log2, srcVoter: true, ckpt: s.ckpt })
          commitStep(st.set(p, s4), sv.union(commitMsgs(s3, s4)), committerMark(s3, s4),
                     true, s.commit <= log2.length() and isPrefix(committedOf(s), log2), true, 0,
                     DeliverDvc({ m: m }))
        }
      } else commitStep(st.set(p, s1), Set(), Set(), true, true, true, 0, DeliverDvc({ m: m })),
    }

  // A process death (crash, or an implementation assertion = fail-stop): volatile state is lost.
  // The highest op whose BODY this node wrote. A header adopted/formed in a view change without its body
  // is MEMORY-ONLY (adopt_canonical_head: "the head/log adoption above is pure data (memory-only)"), and
  // recovery derives the head by scanning the WAL for WRITTEN slots (settle_recover_verified_head: the
  // head is the highest written slot), so a crash drops a header-only tail; interior holes stay holes.
  // (A WAL slot stays WRITTEN once a body landed in it — a later adoption that turns the op into a
  // header-only hole does not unwrite it, and recovery's classify_committed_slot reads it back as
  // StaleCommitted, still counted in the scanned head. `stamp[i] >= 0` IS "slot i+1 holds a written body".)
  pure def writtenHead(s: NState): int =
    range(0, MAX_LOG).foldl(0, (acc, i) =>
      if (i < s.log.length() and i < s.stamp.length() and s.stamp[i] >= 0) i + 1 else acc)

  // (recover_with_reconfig: `op` = the scanned written extent FLOORED at the durable commit — a committed
  // op the node held only as a header survives the crash as a repair hole.)
  // A held op above the durable commit whose WAL stamp is below the durable log_view reads back as
  // StaleCommitted (classify_committed_slot arm 3) -> faulty: it becomes a hole (still counted in the head).
  // classify_committed_slot over a WRITTEN slot above the durable checkpoint: arm 3 (above the durable commit,
  // stamped below the durable log_view: a superseded earlier-view proposal) or arm 2 (at/below the durable
  // commit with no canonical header in the root: unproven). Either reads back StaleCommitted -> faulty.
  pure def faultyAt(s: NState, op: int): bool =
    op >= 1 and op <= s.log.length() and op <= s.stamp.length() and s.stamp[op - 1] >= 0 and op > s.ckpt
    and ((op > s.dcommit and s.stamp[op - 1] < s.logView)
         or (op <= s.dcommit and (not(s.hdrs.contains(op)) or s.wmis.contains(op))))

  pure def staleOps(s: NState): Set[int] =
    0.to(MAX_LOG - 1).filter(i => faultyAt(s, i + 1)).map(i => i + 1)

  // (recovery's head = the highest WRITTEN slot, floored at the highest op the durable root carries a canonical
  // header for — a committed op held only as a header survives as a repair hole — and at the checkpoint)
  pure def died(s: NState): NState =
    val maxHdr = s.hdrs.filter(op => op <= s.log.length()).fold(0, (a, op) => imax(a, op))
    val h = imax(imax(writtenHead(s), maxHdr), imin(s.ckpt, s.log.length()))
    resetSync(s).with("up", false).with("svcFrom", Set()).with("svcTarget", s.view).with("dvcs", Set())
      .with("log", s.log.slice(0, h)).with("stamp", s.stamp.slice(0, h))
      // recovery re-derives the holes from the WAL: a never-written (header-only) op stays a hole, a written
      // slot is a hole iff it reads back faulty — a written slot that now verifies is PRESENT again
      .with("holes", 0.to(MAX_LOG).filter(op => op >= 1 and op <= h
                       and ((s.holes.contains(op) and not(op <= s.stamp.length() and s.stamp[op - 1] >= 0))
                            or faultyAt(s, op))))
      .with("wmis", s.wmis.filter(o => o <= h)).with("reg", Set()).with("nacks", Set())
      .with("nohdr", 0.to(MAX_LOG).filter(op => op >= 1 and op <= h and not(s.hdrs.contains(op))
                       and ((s.holes.contains(op) and not(op <= s.stamp.length() and s.stamp[op - 1] >= 0))
                            or faultyAt(s, op))))
      .with("acks", Set()).with("proofTarget", -1).with("proofAt", -1).with("proofFrontier", -1)

  // Ops of an adopted/formed log `log` held only as headers by a node whose previous log was `old` with
  // holes `oldHoles`: a body survives only where the node held the IDENTICAL entry with its body.
  // The written-mismatch set after an adoption / formation of `log` (holes `holes`) over the previous log
  // `old` (WAL stamps `oldStamp`, mismatches `oldWmis`): a hole whose slot was written keeps its old body.
  pure def wmisAfter(old: List[Entry], oldStamp: List[int], oldWmis: Set[int], log: List[Entry],
                     holes: Set[int]): Set[int] =
    0.to(MAX_LOG).filter(op => op >= 1 and op <= log.length() and holes.contains(op)
      and op <= oldStamp.length() and oldStamp[op - 1] >= 0
      and (oldWmis.contains(op) or op > old.length() or old[op - 1] != log[op - 1]))

  pure def holesAfter(old: List[Entry], oldHoles: Set[int], log: List[Entry]): Set[int] =
    0.to(MAX_LOG - 1).filter(i => i < log.length() and
      not(i < old.length() and old[i] == log[i] and not(oldHoles.contains(i + 1))))
      .map(i => i + 1)

  // Every held (non-hole) op re-stamped with `view` (adopt_append in start_view_acks / formation AdoptVote;
  // committed-band ops are vouched by the durable root's canonical headers, so their stamp is immaterial).
  // A hole KEEPS the slot's previous WAL stamp (`old`): adoption is memory-only above the WAL
  // (truncate_wal_above_adopted_head drops only slots ABOVE the adopted head), so a slot that held a
  // (now superseded) body stays written; a never-written slot stays -1.
  pure def restamp(log: List[Entry], holes: Set[int], view: int, old: List[int]): List[int] =
    range(0, MAX_LOG).foldl([], (acc, i) =>
      if (i < log.length())
        acc.append(if (holes.contains(i + 1)) (if (i < old.length()) old[i] else -1) else view)
      else acc)

  // A list of `n` immaterial stamps (ops covered by a checkpoint, not held in the WAL).
  pure def zeros(n: int): List[int] =
    range(0, MAX_LOG).foldl([], (acc, i) => if (i < n) acc.append(0) else acc)

  // Adopt a canonical head (StartView), shared by deliverSv and the GetView catch-up.
  // `floor` = the sender's checkpoint the StartView / GetView answer / RecoveryResponse carries: its canonical
  // log slice covers only ops ABOVE it (the primary pruned the rest), so an adopted hole at/below the floor is
  // HEADERLESS. Registered holes the carried log now supplies (as headers) are retired; ones it omits stay
  // registered; advance_commit then registers what it walks onto (maybe_force_sync is a no-op here: not yet
  // Normal). After reset_for_view_transition the carried floor is recorded as the primary's checkpoint report.
  pure def adoptHead(s: NState, view: int, log: List[Entry], commit: int, floor: int): NState =
    // reset_for_view_transition clears the SVC tally but KEEPS a raised svc_target
    val hs = holesAfter(s.log, s.holes, log)
    val nh = hs.filter(op => op <= floor)
    val s1 = resetSync(s).with("log", log).with("view", view).with("logView", view).with("status", "normal")
      .with("svcTarget", imax(s.svcTarget, view)).with("svcFrom", Set()).with("dvcs", Set()).with("acks", Set())
      .with("commit", imin(s.commit, log.length())).with("idle", true)
      .with("holes", hs).with("reg", s.reg.intersect(nh)).with("nacks", Set()).with("stamp", restamp(log, hs, view, s.stamp))
      .with("wmis", wmisAfter(s.log, s.stamp, s.wmis, log, hs)).with("nohdr", nh)
    persist(advanceTo(s1, commit).with("pfloor", floor))

  action deliverSvM(m: Msg): bool =
    val s = st.get(m.dst)
    all {
      m.kind == "sv", msgs.contains(m),
      live(s), s.cfg.epoch == m.epoch, isMember(s.cfg, s.id),
      m.view > s.view or (m.view == s.view and s.status == "vc"),
      primaryOf(s.cfg, m.view) == m.src,
      if (m.log.length() < s.commit) {
        // adopt_canonical_head: `assert!(op >= self.commit_min, "must not rewind below our
        // committed op")` — viewstamp panics: the process FAIL-STOPS (flagged).
        commitStep(st.set(m.dst, died(s)), Set(), Set(), true, false, true, 0, DeliverSv({ m: m }))
      } else {
        val s1 = adoptHead(s, m.view, m.log, m.commit, m.ckpt)
        commitStep(st.set(m.dst, s1), acksFor(s1, m.src), Set(), true,
                   isPrefix(committedOf(s), m.log), true, 0, DeliverSv({ m: m }))
      },
    }

  // Higher-view catch-up (same epoch): the laggard GetViews the primary of the higher view, which
  // answers with its StartView (collapsed into one atomic step).
  action catchUpViewM(n: int, p: int): bool =
    val s = st.get(n)
    val q = st.get(p)
    all {
      n != p, live(s), live(q), q.status == "normal", primarySelf(q), s.cfg.epoch == q.cfg.epoch,
      s.status != "rhead",
      q.view > s.view or (q.view == s.view and s.status == "vc" and not(voterSelf(s))),
      isMember(s.cfg, n),
      if (q.log.length() < s.commit) {
        // same assertion on the GetView path: the laggard FAIL-STOPS (flagged).
        commitStep(st.set(n, died(s)), Set(), Set(), true, false, true, 0, CatchUpView({ n: n, p: p }))
      } else {
        val s1 = adoptHead(s, q.view, q.log, q.cmax, q.ckpt)
        commitStep(st.set(n, s1), acksFor(s1, p), Set(), true, isPrefix(committedOf(s), q.log),
                   true, 0, CatchUpView({ n: n, p: p }))
      },
    }

  // ---------------------------------------------------------------------------------------------
  // Crash / restart
  // ---------------------------------------------------------------------------------------------
  action crashM(n: int): bool =
    val s = st.get(n)
    all {
      live(s),
      commitStep(st.set(n, died(s)), Set(), Set(), true, true, true, 0, Crash({ n: n })),
    }

  // Recovery (recovery/mod.rs complete_recovery): the node recovers its durable root (commit restarts
  // from the durable commit), then
  //   * log_view < view        -> a voter RE-DRIVES the view change at `view` (re-casts its DVC); a learner
  //                               enters the catch-up posture (ViewChange, GetView)
  //   * primary of `view` with more than one voter, unless a freshly formatted genesis store
  //                               (view 0, op 0, commit_max 0) -> ABDICATES to view + 1
  //   * otherwise Normal (a solo voter resumes its pipeline and re-commits its own tail).
  action restartM(n: int): bool =
    val s = st.get(n)
    // recover_with_reconfig: commit_min = the durable CHECKPOINT op (the restored SM reflects exactly
    // [1..=checkpoint_op]); commit_max = the durable root's commit. The committed band (ckpt .. cmax]
    // re-applies only via a later advance_commit (Commit / Prepare / adoption / debt-pay / solo resume).
    val s0 = s.with("up", true).with("commit", imin(s.ckpt, s.log.length())).with("cmax", s.dcommit)
    // recover_progress: a HEAD that read back faulty (here: a stale-stamped held op, dropped to a hole by
    // `died`) -> RecoveringHead: the node does not participate; it solicits the canonical head (Recovery ->
    // the primary's RecoveryResponse) or adopts a StartView. Decided BEFORE complete_recovery.
    val headFaulty = s.log.length() >= 1 and (faultyAt(s, s.log.length())
                     or (s.log.length() > s.dcommit and s.holes.contains(s.log.length())))
    all {
      s.started, not(s.up), s.status != "retired",
      if (headFaulty) {
        commitStep(st.set(n, s0.with("status", "rhead")), Set(), Set(), true, true, true, 0, Restart({ n: n }))
      } else if (s.logView < s.view) {
        if (voterSelf(s0)) {
          val r = enterVc(s0, s.view)
          commitStep(st.set(n, r._1), r._2, Set(), true, true, true, 0, Restart({ n: n }))
        } else
          commitStep(st.set(n, s0.with("status", "vc")), Set(), Set(), true, true, true, 0,
                     Restart({ n: n }))
      } else if (nVoters(s0.cfg) > 1 and primarySelf(s0)
                 and not(s.view == 0 and s.log.length() == 0 and s.dcommit == 0)) {
        all {
          s.view + 1 <= MAX_VIEW,
          val r = enterVc(s0, s.view + 1)
          commitStep(st.set(n, r._1), r._2, Set(), true, true, true, 0, Restart({ n: n })),
        }
      } else if (nVoters(s0.cfg) == 1 and voterSelf(s0)) {
        // resume_solo_voter_pipeline: the lone voter re-seeds its own votes for the uncommitted tail
        val own = 0.to(MAX_LOG - 1).filter(i => i + 1 > s0.commit and i + 1 <= s0.log.length())
                    .map(i => (i + 1, n))
        val s1 = tryCommit(s0.with("status", "normal").with("idle", true).with("acks", own))
        commitStep(st.set(n, s1), commitMsgs(s0, s1), committerMark(s0, s1), true, true, true, 0,
                   Restart({ n: n }))
      } else
        commitStep(st.set(n, ckptDebt(s0.with("status", "normal").with("idle", true))), Set(), Set(), true, true,
                   true, 0, Restart({ n: n })),
    }

  // RecoveringHead exit (on_recovery_response): the node's Recovery reaches the Normal primary `p` of a
  // view >= its own, whose RecoveryResponse (view, op, commit_max, log) it adopts exactly like a StartView.
  action recoverHeadM(n: int, p: int): bool =
    val s = st.get(n)
    val q = st.get(p)
    all {
      n != p, live(s), live(q), s.status == "rhead", q.status == "normal", primarySelf(q),
      s.cfg.epoch == q.cfg.epoch, q.view >= s.view, isMember(q.cfg, n),
      if (q.log.length() < s.commit) {
        commitStep(st.set(n, died(s)), Set(), Set(), true, false, true, 0, RecoverHead({ n: n, p: p }))
      } else {
        val s1 = adoptHead(s, q.view, q.log, q.cmax, q.ckpt)
        commitStep(st.set(n, s1), acksFor(s1, p), Set(), true, isPrefix(committedOf(s), q.log),
                   true, 0, RecoverHead({ n: n, p: p }))
      },
    }

  // ---------------------------------------------------------------------------------------------
  // Body repair and repair-or-truncate (repair.rs): `n` asks `h` for the bodies it holds only as
  // headers. `h` serves only while Normal: a body it holds (identical entry, not itself a hole) is
  // served and filled; an op `h` lacks entirely (above its head and its commit_max) is NACKED — counted
  // by a new primary only for a candidate (a header-only op above its commit_max) from a voter in its
  // own view. A nack quorum (n - q + 1) on the LOWEST candidate truncates the uncommitted tail there.
  // ---------------------------------------------------------------------------------------------
  pure def serves(h: NState, n: NState, op: int): bool =
    op <= h.log.length() and op <= n.log.length() and h.log[op - 1] == n.log[op - 1]
    and not(h.holes.contains(op))
    // a checkpointed op is pruned from the holder's WAL/log (run_gc): it can no longer be served
    and op > h.ckpt

  pure def nacksFor(h: NState, n: NState, op: int): bool =
    op > h.log.length() and op > h.cmax and primarySelf(n) and n.status == "normal"
    and h.view == n.view and isVoter(n.cfg, h.id) and op > n.cmax

  // Which of its header-only ops `n` actually SOLICITS (repair.rs): a REGISTERED hole (repair_timeouts
  // re-solicits `self.repair`; registration happens when advance_commit reaches the hole, capped at
  // commit_max), or, on a Normal primary, a header-only op above commit_max (the repair-or-truncate
  // candidates). A backup does NOT solicit an unregistered header-only op; it is registered (and solicited)
  // once an advance_commit reaches it.
  pure def solicits(s: NState, op: int): bool =
    s.reg.contains(op) or (primarySelf(s) and s.status == "normal")

  action repairExchangeM(n: int, h: int): bool =
    val s = st.get(n)
    val q = st.get(h)
    val filled = s.holes.filter(op => solicits(s, op) and serves(q, s, op)
                                      and (not(s.nohdr.contains(op)) or q.commit >= op))
    val nacked = s.holes.filter(op => not(serves(q, s, op)) and nacksFor(q, s, op))
    all {
      // repair is solicited and serviced only while the REQUESTER is Normal too (handle_timeout runs
      // repair_timeouts only in Normal; arm_timers arms repair_retry only when Normal): a hole held into a
      // view change / catch-up waits until Normal resumes
      n != h, live(s), live(q), s.status == "normal", q.status == "normal",
      // a peer serves repair only within its own configuration (a request from a configuration it has not
      // reached is not admitted)
      s.cfg.epoch == q.cfg.epoch,
      filled.union(nacked).size() > 0,
      // fill_repair stamps the SERVED Prepare's view — the server's CURRENT view (repair.rs Prepare::new)
      val s1 = s.with("holes", s.holes.exclude(filled)).with("reg", s.reg.exclude(filled)).with("wmis", s.wmis.exclude(filled))
                .with("nohdr", s.nohdr.exclude(filled))
                .with("stamp", range(0, MAX_LOG).foldl(s.stamp, (acc, i) =>
                                 if (filled.contains(i + 1) and i < acc.length()) acc.replaceAt(i, q.view) else acc))
                .with("nacks", s.nacks.union(nacked.map(op => (op, h))))
      // repair-or-truncate on the lowest remaining candidate
      val cands = s1.holes.filter(op => op > s1.cmax)
      val gap = cands.fold(MAX_LOG + 1, (a, op) => imin(a, op))
      val nackers = s1.nacks.filter(x => x._1 == gap).map(x => x._2)
      val trunc = primarySelf(s1) and s1.status == "normal" and gap <= MAX_LOG
                  and nackers.size() >= quorumVc(s1.cfg)
      val s2 = if (trunc)
                 s1.with("log", s1.log.slice(0, gap - 1)).with("stamp", s1.stamp.slice(0, gap - 1))
                   .with("holes", s1.holes.filter(op => op < gap)).with("reg", s1.reg.filter(op => op < gap))
                   .with("wmis", s1.wmis.filter(op => op < gap)).with("nohdr", s1.nohdr.filter(op => op < gap))
                   .with("acks", s1.acks.filter(a => a._1 < gap))
                   .with("nacks", s1.nacks.filter(x => x._1 < gap))
               else s1
      // a filled body is durably appended, then voted: the primary's own vote, a backup's PrepareOk
      val isP = primarySelf(s2) and s2.status == "normal"
      val newVotes = filled.filter(op => op > s2.commit and op <= s2.log.length() and voterSelf(s2))
      // (fill_repair: a filled op ABOVE commit_max on the primary is an adopted candidate -> own vote +
      // try_commit; a filled op at/below commit_max is already known committed -> advance_commit)
      val s3 = if (isP) s2.with("acks", s2.acks.union(newVotes.filter(op => op > s2.cmax).map(op => (op, n))))
               else s2
      val s3a = if (isP) advanceTo(s3, s3.cmax) else s3
      val s4 = if (isP) tryCommit(s3a) else ckptTail(advanceTo(s3, s3.cmax))
      val oks = if (not(isP) and s2.status == "normal")
                  newVotes.filter(op => op > s2.cmax)
                    .map(op => { ...NO_MSG, kind: "prepareOk", src: n, dst: primaryOf(s2.cfg, s2.view),
                                 view: s2.view, epoch: s2.cfg.epoch, op: op, srcVoter: true })
                else Set()
      commitStep(st.set(n, s4), oks.union(if (isP) commitMsgs(s3a, s4) else Set()),
                 if (isP) committerMark(s3a, s4) else Set(), true, true, true, 0,
                 RepairExchange({ n: n, h: h })),
    }

  // ---------------------------------------------------------------------------------------------
  // Reconfiguration
  // ---------------------------------------------------------------------------------------------
  pure def proofFresh(s: NState, l: int): bool =
    s.proofTarget == l and s.proofAt >= s.log.length() and s.proofFrontier >= s.log.length()

  // The shared propose pre-gates (propose_membership, in order): primary, Normal, no change in flight.
  pure def canPropose(s: NState): bool =
    live(s) and s.status == "normal" and primarySelf(s) and not(pendingReconfig(s)) and admits(s)

  // propose_membership(PromoteLearner(l)) with NO fresh proof: Phase 2 — solicit a proof for the head.
  action requestProofM(p: int, l: int): bool =
    val s = st.get(p)
    all {
      canPropose(s), s.log.length() < MAX_LOG,
      deltaValid(s.cfg, { kind: "promote", who: l }), not(proofFresh(s, l)),
      // an IN-FLIGHT challenge (same target + head, no reply yet) is REUSED (same nonce, retransmitted);
      // anything else draws a FRESH nonce and supersedes the old challenge (reconfigure.rs Phase 2)
      val reuse = s.proofTarget == l and s.proofAt == s.log.length() and s.proofFrontier == -1
      val nonce = if (reuse) s.proofNonce else nonceCtr + 1
      val s1 = s.with("proofTarget", l).with("proofAt", s.log.length()).with("proofFrontier", -1)
                .with("proofNonce", nonce)
      commitStep(st.set(p, s1),
                 Set({ ...NO_MSG, kind: "reqProof", src: p, dst: l, epoch: s.cfg.epoch,
                       op: s.log.length(), srcVoter: true, nonce: nonce }),
                 Set(), true, true, true, 0, RequestProof({ p: p, l: l })),
    }

  // on_request_learner_proof: answer with commit_min for the live config only.
  action deliverReqProofM(m: Msg): bool =
    val s = st.get(m.dst)
    val reply = { ...NO_MSG, kind: "proof", src: s.id, dst: m.src, epoch: s.cfg.epoch,
                  op: m.op, frontier: s.commit, srcVoter: voterSelf(s), nonce: m.nonce }
    all {
      m.kind == "reqProof", msgs.contains(m),
      live(s), s.status != "rhead", s.cfg.epoch == m.epoch,
      not(msgs.contains(reply)),
      commitStep(st, Set(reply), Set(), true, true, true, 0, DeliverReqProof({ m: m })),
    }

  // on_learner_proof: validate against the outstanding challenge (nonce ~ the challenged head).
  action deliverProofM(m: Msg): bool =
    val s = st.get(m.dst)
    all {
      m.kind == "proof", msgs.contains(m),
      live(s), s.proofTarget == m.src, s.proofAt == m.op, s.proofNonce == m.nonce, s.cfg.epoch == m.epoch,
      s.proofFrontier != m.frontier,
      commitStep(st.set(m.dst, s.with("proofFrontier", m.frontier)), Set(), Set(),
                 true, true, true, 0, DeliverProof({ m: m })),
    }

  // propose_membership that MINTS (every non-promote delta, or a promote with a fresh proof).
  action proposeM(p: int, kind: str, who: int, ack: bool): bool =
    val s = st.get(p)
    val d = { kind: kind, who: who }
    val hd = s.log.length()
    all {
      canPropose(s), hd < MAX_LOG,
      deltaValid(s.cfg, d),
      ack or not(reducesFaultTolerance(s.cfg, d)),
      kind != "promote" or proofFresh(s, who),
      val succ = applyDelta(s.cfg, d)
      // the acknowledgement token is a PROPOSE-TIME gate, not part of the Reconfigure op body ("a superfluous
      // token there is ignored"): the entry records it only where it is load-bearing (an f-reducing delta)
      val e = { ...NO_ENTRY, kind: "reconfig", cfg: succ, prevEpoch: s.cfg.epoch,
                ack: ack and reducesFaultTolerance(s.cfg, d), reduces: reducesFaultTolerance(s.cfg, d) }
      val op = hd + 1
      val s1 = s.with("log", s.log.append(e)).with("stamp", s.stamp.append(s.view))
                .with("acks", s.acks.union(Set((op, p))))
        .with("proofTarget", if (kind == "promote") -1 else s.proofTarget)
        .with("proofAt", if (kind == "promote") -1 else s.proofAt)
        .with("proofFrontier", if (kind == "promote") -1 else s.proofFrontier)
      val s2 = tryCommit(s1)
      val prep = bcast(s, members(s.cfg),
                       { ...NO_MSG, kind: "prepare", src: p, view: s.view, epoch: s.cfg.epoch,
                         op: op, commit: s.commit, log: [e], srcVoter: true, ckpt: s.ckpt })
      // ghost: the promoted learner ACTUALLY holds the committed-and-prepared prefix [1..hd]
      val target = st.get(who)
      val caughtUp = kind != "promote" or (target.commit >= hd and isPrefix(s.log, target.log))
      commitStep(st.set(p, s2), prep.union(commitMsgs(s1, s2)), committerMark(s1, s2),
                 caughtUp, true, true, 0, Propose({ p: p, kind: kind, who: who, ack: ack })),
    }

  // propose_membership of an f-REDUCING delta WITHOUT the AcceptReducedFaultTolerance token: the implementation
  // must REFUSE it (ReducedFaultToleranceUnacknowledged) and mint nothing (the negative half of shrinkAcked).
  action proposeRefusedM(p: int, kind: str, who: int): bool =
    val s = st.get(p)
    val d = { kind: kind, who: who }
    all {
      canPropose(s), s.log.length() < MAX_LOG, deltaValid(s.cfg, d), reducesFaultTolerance(s.cfg, d),
      commitStep(st, Set(), Set(), true, true, true, 0, ProposeRefused({ p: p, kind: kind, who: who })),
    }

  // A brand-new learner is started out of band with the installed successor membership (#84 interim).
  action bootstrapM(x: int, d: int): bool =
    val s = st.get(x)
    val q = st.get(d)
    all {
      not(s.started), live(q), isLearner(q.cfg, x),
      commitStep(st.set(x, s.with("started", true).with("up", true).with("status", "normal")
                               .with("cfg", q.cfg).with("view", 0).with("logView", 0)
                               .with("log", []).with("stamp", []).with("commit", 0).with("cmax", 0).with("dcommit", 0)
                               .with("hdrs", Set()).with("holes", Set()).with("reg", Set()).with("wmis", Set()).with("nohdr", Set())
                               .with("ckpt", 0).with("inst", 0).with("boot", true)
                               .with("idle", false)),
                 Set(), Set(), true, true, true, 0, Bootstrap({ x: x, d: d })),
    }

  // Cross-epoch catch-up: state-sync from the donor's durable CHECKPOINT `cp` (== donor ckpt, >= the donor's
  // install op), install the donor's config; VIEW and LOG_VIEW are preserved.
  // Same-epoch FORCED state sync (state_sync.rs maybe_force_sync): a Normal backup whose solicited
  // committed hole lies at/below a peer's checkpoint can never have it served (pruned), so it syncs the
  // donor's checkpoint: commit_min = cp, commit_max = max(commit_max, cp) with a held tail above cp (else
  // op = commit_max = cp), holes <= cp dropped; the held tail then re-applies. View/log_view unchanged.
  // Completing an ARMED same-epoch state sync (state_sync.rs on_sync_checkpoint -> block fetch -> apply_sync ->
  // install_sync): a Normal donor `d` whose durable checkpoint is ABOVE n's serves it (on_request_sync's
  // `checkpoint_op > m.checkpoint_op()` gate); n takes it only at/above the armed target, above its own
  // checkpoint, and — for an ORDINARY sync — above its head (else "a racing tail-apply already covered it").
  // The re-persist root is STAGED first (it records commit_max as then known, headers for what n then held);
  // install_sync then advances the frontier to cp — with a held tail above cp (FORCED only) op/commit_max
  // stay, else op = commit_max = cp — and the held tail re-applies.
  action completeSyncM(n: int, d: int): bool =
    val s = st.get(n)
    val q = st.get(d)
    val cp = q.ckpt
    val held = s.log.length() > cp
    all {
      n != d, live(s), live(q), s.status == "normal", not(primarySelf(s)), q.status == "normal",
      s.cfg.epoch == q.cfg.epoch, isMember(s.cfg, d), isMember(q.cfg, n),
      s.sync > 0, cp >= s.sync, cp > s.ckpt, cp >= s.commit, cp <= q.commit,
      s.syncForced or cp > s.log.length(),
      val staged = persist(s.with("ckpt", cp))
      val s1 = staged.with("log", if (held) q.log.slice(0, cp).concat(s.log.slice(cp, s.log.length()))
                                  else q.log.slice(0, cp))
                .with("stamp", if (held) zeros(cp).concat(s.stamp.slice(cp, s.stamp.length())) else zeros(cp))
                .with("holes", s.holes.filter(op => op > cp)).with("reg", s.reg.filter(op => op > cp))
                .with("wmis", s.wmis.filter(op => op > cp)).with("nohdr", s.nohdr.filter(op => op > cp)).with("commit", cp)
                .with("cmax", if (held) imax(s.cmax, cp) else cp)
                .with("sync", 0).with("syncForced", false)
      commitStep(st.set(n, ckptTail(advanceTo(s1, s1.cmax))), Set(), Set(), true,
                 isPrefix(committedOf(s), q.log), true, 0, CompleteSync({ n: n, d: d })),
    }

  // The crossing needs a TRIGGER (normal.rs maybe_request_cross_epoch_catchup): a higher-epoch signal from a
  // node `t` that `n` authenticates as a member of ITS config, and that holds `n` as a member of t's config:
  //   (i)  t is a Normal PRIMARY of the higher epoch — its Commit heartbeat / Prepare reaches n; or
  //   (ii) n's own lower-epoch traffic reaches a NORMAL t, which answers EpochAhead (maybe_answer_lower_epoch
  //        answers only Prepare/Commit/SVC/DVC/LearnerStatus, and only while Normal): a Normal learner's
  //        LearnerStatus goes to primary(view) of its config; a voter's SVC (view change, or a Normal backup's
  //        armed idle timer) goes to the voters of its config; a Normal primary's Prepare/Commit goes to all.
  pure def crossTriggered(nst: int -> NState, n: int): bool =
    val s = nst.get(n)
    NODES.exists(t => t != n and live(nst.get(t)) and nst.get(t).status == "normal"
      and nst.get(t).cfg.epoch > s.cfg.epoch and isMember(s.cfg, t) and isMember(nst.get(t).cfg, n)
      and (primarySelf(nst.get(t))
           or (isLearner(s.cfg, n) and s.status == "normal" and primaryOf(s.cfg, s.view) == t)
           or (voterSelf(s) and isVoter(s.cfg, t) and (s.status == "vc" or (s.status == "normal" and s.idle)))
           or (primarySelf(s) and s.status == "normal")))

  // Trigger (i) alone: a Normal primary of a higher epoch whose heartbeat reaches `n` (it authenticates as a
  // member of n's config and holds n as a member of its own).
  pure def primaryHeartbeatReaches(nst: int -> NState, n: int): bool =
    val s = nst.get(n)
    NODES.exists(t => t != n and live(nst.get(t)) and nst.get(t).status == "normal" and primarySelf(nst.get(t))
      and nst.get(t).cfg.epoch > s.cfg.epoch and isMember(s.cfg, t) and isMember(nst.get(t).cfg, n))

  action syncFromM(n: int, d: int, cp: int): bool =
    val s = st.get(n)
    val q = st.get(d)
    // The install op the crossing CARRIES is the donor's own config_install_op (`inst`), not the true op that
    // produced its config: a node bootstrapped out of band with a successor membership (#84 interim path) has
    // config_install_op = 0 (the genesis default) and a same-config sync never corrects it.
    val installOp = q.inst
    // When no higher-epoch primary's heartbeat reaches a Normal voter BACKUP, its only lower-epoch traffic is
    // its primary_idle timeout: propose_next_view raises svc_target to view+1 and JOINS (own bit + SVC
    // broadcast) before the EpochAhead answer arrives. The fixed crossing discards that predecessor-epoch
    // proposal with the rest of the old view identity. (A join that would complete an SVC quorum — entering a
    // predecessor-epoch view change mid-crossing — is not modelled: disabled.)
    val idleSvc = not(primaryHeartbeatReaches(st, n)) and voterSelf(s) and s.status == "normal" and not(primarySelf(s))
    val raise = s.view + 1 > s.svcTarget
    val sj = if (idleSvc)
               s.with("svcTarget", if (raise) s.view + 1 else s.svcTarget)
                .with("svcFrom", (if (raise) Set() else s.svcFrom).union(Set(n)))
             else s
    all {
      not(idleSvc) or sj.svcFrom.size() < quorumVc(s.cfg),
      // the donor answers (EpochAhead / RequestSync) only a sender that resolves to a member of ITS
      // membership (maybe_answer_lower_epoch's `admissible`; an unresolved peer is dropped at the sender
      // binding): a node REMOVED by an op it never committed cannot cross and so never learns it retired
      // ... and the laggard's broadcast RequestSync (`send_request_sync` -> Backups of ITS OWN membership)
      // reaches only donors that are members of the laggard's current configuration
      n != d, live(s), live(q), q.status == "normal", q.cfg.epoch > s.cfg.epoch, isMember(q.cfg, n),
      s.status != "rhead", crossTriggered(st, n),
      isMember(s.cfg, d),
      // the donor serves its DURABLE checkpoint (SyncCheckpoint carries `checkpoint_op`; "a donor at checkpoint 0
      // serves nothing"), attaching its membership only once that checkpoint covers its config_install_op (the
      // serve gate); the laggard's apply_sync rejects a carried install op BELOW its own effective
      // config_install_op (state_sync.rs `install_op < effective.config_install_op()`)
      cp == q.ckpt, cp > 0, cp >= installOp, installOp >= s.inst, cp <= q.commit, cp >= s.commit,
      not(q.boot),
      val synced =
        sj.with("cfg", q.cfg).with("log", q.log.slice(0, cp)).with("stamp", zeros(cp)).with("commit", cp).with("cmax", cp)
               .with("dcommit", cp)
               .with("hdrs", 0.to(MAX_LOG).filter(op => op >= 1 and op <= cp))
               .with("ckpt", cp).with("inst", installOp).with("boot", false)
               .with("holes", Set()).with("reg", Set()).with("wmis", Set()).with("nohdr", Set()).with("nacks", Set())
               .with("sync", 0).with("syncForced", false).with("pfloor", 0)
               .with("idle", true)
               .with("svcFrom", sj.svcFrom.intersect(setOf(q.cfg.voters))).with("dvcs", Set()).with("acks", Set())
               .with("proofTarget", -1).with("proofAt", -1).with("proofFrontier", -1)
      // VR Revisited §8.3: each epoch has an independent view-number space and begins at view 0. The
      // crossing root persists the same `(view, logView) = (0, 0)` pair, and the live install abandons
      // every predecessor-epoch SVC/DVC collection before the endpoint can participate in the successor.
      val fixed = persist(synced.with("view", 0).with("logView", 0).with("status", "normal")
                                  .with("svcTarget", 0).with("svcFrom", Set()).with("dvcs", Set()))
      val out = if (CROSS_FIX) (fixed, Set()) else (synced, Set())
      commitStep(st.set(n, out._1), out._2, Set(), true, isPrefix(committedOf(s), q.log), true, 0,
                 SyncFrom({ n: n, d: d, cp: cp })),
    }

  // ---------------------------------------------------------------------------------------------
  // The nondeterministic step (random exploration + quint-connect `quint run` traces)
  // ---------------------------------------------------------------------------------------------
  action clientRequest = { nondet p = NODES.oneOf() clientRequestM(p) }
  action deliverPrepare = {
    nondet m = msgs.filter(m => m.kind == "prepare").oneOf()
    any { deliverPrepareM(m), deliverPrepareDropM(m) }
  }
  action deliverPrepareOk = { nondet m = msgs.filter(m => m.kind == "prepareOk").oneOf() deliverPrepareOkM(m) }
  action deliverCommit = { nondet m = msgs.filter(m => m.kind == "commit").oneOf() deliverCommitM(m) }
  action heartbeat = { nondet p = NODES.oneOf() heartbeatM(p) }
  action timeout = { nondet n = NODES.oneOf() timeoutM(n) }
  action deliverSvc = { nondet m = msgs.filter(m => m.kind == "svc").oneOf() deliverSvcM(m) }
  action deliverDvc = { nondet m = msgs.filter(m => m.kind == "dvc").oneOf() deliverDvcM(m) }
  action deliverSv = { nondet m = msgs.filter(m => m.kind == "sv").oneOf() deliverSvM(m) }
  action catchUpView = { nondet n = NODES.oneOf() nondet p = NODES.oneOf() catchUpViewM(n, p) }
  action crash = { nondet n = NODES.oneOf() crashM(n) }
  action restart = { nondet n = NODES.oneOf() restartM(n) }
  // (churn-directed nondeterminism: only a proposing-capable primary, only a learner of its config)
  action requestProof = {
    nondet p = NODES.filter(x => canPropose(st.get(x))).oneOf()
    nondet l = NODES.filter(x => isLearner(st.get(p).cfg, x)).oneOf()
    requestProofM(p, l)
  }
  action deliverReqProof = { nondet m = msgs.filter(m => m.kind == "reqProof").oneOf() deliverReqProofM(m) }
  action deliverProof = { nondet m = msgs.filter(m => m.kind == "proof").oneOf() deliverProofM(m) }
  // (churn-directed nondeterminism: a delta is picked only among the ones proposeM would MINT, so the
  // random walk reaches promote/demote/add/remove instead of mostly-invalid picks)
  action propose = {
    nondet p = NODES.filter(x => canPropose(st.get(x))).oneOf()
    val s = st.get(p)
    nondet d = Set("promote", "demote", "add", "remove").map(k => NODES.map(w => { kind: k, who: w })).flatten()
                 .filter(d => deltaValid(s.cfg, d) and (d.kind != "promote" or proofFresh(s, d.who))).oneOf()
    nondet ack = Set(true, false).filter(a => a or not(reducesFaultTolerance(s.cfg, d))).oneOf()
    proposeM(p, d.kind, d.who, ack)
  }
  action bootstrap = { nondet x = NODES.oneOf() nondet d = NODES.oneOf() bootstrapM(x, d) }
  action repairExchange = { nondet n = NODES.oneOf() nondet h = NODES.oneOf() repairExchangeM(n, h) }
  action completeSync = { nondet n = NODES.oneOf() nondet d = NODES.oneOf() completeSyncM(n, d) }
  action recoverHead = { nondet n = NODES.oneOf() nondet p = NODES.oneOf() recoverHeadM(n, p) }
  action proposeRefused = {
    nondet p = NODES.filter(x => canPropose(st.get(x))).oneOf()
    nondet who = NODES.filter(w => deltaValid(st.get(p).cfg, { kind: "demote", who: w })
                                   and reducesFaultTolerance(st.get(p).cfg, { kind: "demote", who: w })).oneOf()
    proposeRefusedM(p, "demote", who)
  }
  action syncFrom = {
    nondet n = NODES.oneOf()
    nondet d = NODES.oneOf()
    nondet cp = 0.to(MAX_LOG).oneOf()
    syncFromM(n, d, cp)
  }

  // ---------------------------------------------------------------------------------------------
  // Scripted-run helpers: the in-flight message with the given coordinates (read in the CURRENT state).
  // ---------------------------------------------------------------------------------------------
  def msgAt(kind: str, src: int, dst: int, view: int, epoch: int, op: int): Msg =
    msgs.filter(m => m.kind == kind and m.src == src and m.dst == dst and m.view == view
                     and m.epoch == epoch and m.op == op)
        .fold(NO_MSG, (a, m) => m)

  def commitMsgAt(src: int, dst: int, view: int, epoch: int, commit: int): Msg =
    msgs.filter(m => m.kind == "commit" and m.src == src and m.dst == dst and m.view == view
                     and m.epoch == epoch and m.commit == commit)
        .fold(NO_MSG, (a, m) => m)

  action step = any {
    clientRequest, deliverPrepare, deliverPrepareOk, deliverCommit, heartbeat,
    timeout, deliverSvc, deliverDvc, deliverSv, catchUpView,
    crash, restart,
    requestProof, deliverReqProof, deliverProof, propose, bootstrap, syncFrom, repairExchange,
    completeSync, recoverHead, proposeRefused,
  }

  // Churn-focused exploration: the same actions, but at most FAULT_BUDGET crashes per trace, so a random walk
  // gets deep enough for add -> bootstrap -> sync -> proof -> promote -> demote -> remove under view changes.
  pure val FAULT_BUDGET: int = 2
  // (client ops are budgeted too: each one moves the head past a learner's proof, so an unbounded stream of
  // them starves promotion; `nextLabel` is the next client label, starting at 1)
  pure val CLIENT_BUDGET: int = 3
  // (The simulator picks uniformly among the ENABLED branches of `any`, so the reconfiguration chain —
  // bootstrap, same-epoch sync, proof solicit/answer/delivery, propose, crossing — is listed more than once
  // to weight it; every branch is still the unmodified action, so each step is an ordinary protocol step.)
  action stepChurn = any {
    all { nextLabel <= CLIENT_BUDGET, clientRequest }, deliverPrepare, deliverPrepareOk, deliverCommit, heartbeat,
    timeout, deliverSvc, deliverDvc, deliverSv, catchUpView,
    all { faults < FAULT_BUDGET, crash }, restart,
    requestProof, deliverReqProof, deliverProof, propose, bootstrap, syncFrom, repairExchange,
    completeSync, recoverHead,
    requestProof, deliverReqProof, deliverProof, propose, bootstrap, syncFrom, completeSync,
    requestProof, deliverReqProof, deliverProof, propose, completeSync,
    deliverReqProof, deliverProof, propose, proposeRefused,
    idle,
  }

  // A no-op step (nothing is delivered, no clock advances): keeps a settled, budget-exhausted stepChurn
  // walk from deadlocking a `reps` suffix. The driver maps it to no input.
  action idle = commitStep(st, Set(), Set(), true, true, true, 0, Idle)

  // ---------------------------------------------------------------------------------------------
  // Invariants
  // ---------------------------------------------------------------------------------------------
  // (1) Committed-op durability / agreement: committed prefixes never conflict, and nothing ever
  //     committed is lost from the longest committed history (ghost).
  val agreement: bool = ghostOk and NODES.forall(n => isPrefix(committedOf(st.get(n)), ghostLog))

  // (1b) Completeness (VR's view-change invariant, lifted over epochs): a Normal primary that has
  //      formed its log (logView == view) at (epoch E, view v) holds every op that was first committed
  //      at an (epoch, view) strictly before (E, v) (lexicographic). A primary lacking such an op is one
  //      view change away from truncating a committed op.
  val formedPrimaries: Set[int] =
    NODES.filter(n => live(st.get(n)) and st.get(n).status == "normal" and primarySelf(st.get(n))
                       and st.get(n).logView == st.get(n).view)
  pure def ev(s: NState): int = s.cfg.epoch * (MAX_VIEW + 1) + s.view
  val completeness: bool =
    formedPrimaries.forall(n =>
      val s = st.get(n)
      ghostLog.indices().forall(i =>
        ghostAt[i] >= ev(s) or (i < s.log.length() and s.log[i] == ghostLog[i])))

  // (2) At most one node commits AS PRIMARY per (epoch, view).
  val singlePrimary: bool =
    committers.forall(a => committers.forall(b => (a._1 == b._1 and a._2 == b._2) implies a._3 == b._3))

  // (3) A learner is promoted only once it durably holds the head it will vote on.
  val promotionCaughtUp: bool = promoteOk

  // (4) A shrink that lowers fault tolerance is only ever minted with the explicit acknowledgement.
  val shrinkAcked: bool =
    NODES.forall(n => st.get(n).log.indices().forall(i => st.get(n).log[i].reduces implies st.get(n).log[i].ack))

  // (5) Learners never cast a counted vote.
  val learnerNoVote: bool =
    msgs.forall(m => (m.kind == "prepareOk" or m.kind == "svc" or m.kind == "dvc") implies m.srcVoter)

  // (6) No fail-stop assertion of the implementation is reachable (adopt below commit / commit* > head).
  val noFailStop: bool = adoptOk and commitStarOk

  // ---- non-vacuity witnesses (quint run --witnesses=...) ----
  val wEpoch1: bool = NODES.exists(n => st.get(n).cfg.epoch >= 1)
  val wEpoch2: bool = NODES.exists(n => st.get(n).cfg.epoch >= 2)
  val wGrew: bool = NODES.exists(n => st.get(n).started and nVoters(st.get(n).cfg) > nVoters(GENESIS))
  val wShrank: bool = NODES.exists(n => st.get(n).started and nVoters(st.get(n).cfg) < nVoters(GENESIS))
  val wViewChanged: bool = NODES.exists(n => st.get(n).status == "normal" and st.get(n).logView >= 1)
  val wRetired: bool = NODES.exists(n => st.get(n).status == "retired")
  val wBootstrapped: bool = NODES.exists(n => st.get(n).started and not(isMember(GENESIS, n)))
  val wForkedViews: bool = NODES.exists(a => NODES.exists(b =>
    live(st.get(a)) and live(st.get(b)) and st.get(a).status == "normal" and st.get(b).status == "normal"
    and st.get(a).cfg.epoch == st.get(b).cfg.epoch and st.get(a).view != st.get(b).view))
  val wCommitted3: bool = ghostLog.length() >= 3

  val safety: bool = agreement and completeness and singlePrimary and promotionCaughtUp
                     and shrinkAcked and learnerNoVote and noFailStop
}

// ---- Instances ---------------------------------------------------------------------------------
// 4 genesis voters (even quorum: q=3, qvc=2) — exercises the demote/shrink path.
module vr_sc_4v {
  import vr_common.* from "./vr_common"
  import vr_sc(
    NODES = Set(0, 1, 2, 3),
    GENESIS = { epoch: 0, voters: [0, 1, 2, 3], learners: [] },
    MAX_LOG = 4,
    MAX_VIEW = 3,
    CHECKPOINT_OPS = 4,
    CROSS_FIX = true
  ).* from "./vr_sc"

  // ---- churn from a GROWN cluster (see vr_sc_solo): demote 3, then promote it back (E2 = V[0,1,2,3]).
  run grow4v =
    init
      .then(proposeM(0, "demote", 3, false))                  // R1: V[0,1,2,3] -> V[0,1,2] L[3]
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 0, 1)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 1, 0, 0, 0, 1)))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 0, 1)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 2, 0, 0, 0, 1))) // n0 commits R1 (q(4) = 3 + successor q)
      .then(deliverPrepareM(msgAt("prepare", 0, 3, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 1, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 2, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 3, 0, 0, 1)))        // all at E1, n3 a learner holding R1
      .then(requestProofM(0, 3))
      .then(deliverReqProofM(msgAt("reqProof", 0, 3, 0, 1, 1)))
      .then(deliverProofM(msgAt("proof", 3, 0, 0, 1, 1)))
      .then(proposeM(0, "promote", 3, false))                 // R2: V[0,1,2] L[3] -> V[0,1,2,3]
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 1, 2)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 1, 0, 0, 1, 2))) // n0 commits R2 (q(3) = 2)
      .expect(st.get(0).cfg.epoch == 2 and nVoters(st.get(0).cfg) == 4)
  run grow4vChurn = grow4v.then(25.reps(_ => stepChurn))

  // The simulator-found counterexample (seed 0x8c6a96ba0ad1e4d5, 4 voters), minimised to the steps that
  // matter, up to the phantom primary: node 1 becomes a FORMED Normal primary of (epoch 1, view 1) whose
  // log lacks c1, which the real epoch-1 primary committed at (epoch 1, view 0).
  run phantomPrefix =
    init
      .then(proposeM(0, "demote", 3, true))                    // R: 4 voters -> V[0,1,2] L[3]
      .then(deliverPrepareM(msgAt("prepare", 0, 3, 0, 0, 1)))  // n3 appends R, acks
      .then(crashM(1))
      .then(restartM(1))
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 0, 1)))  // n1 appends R (acks, never counted)
      .then(deliverPrepareOkM(msgAt("prepareOk", 3, 0, 0, 0, 1)))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 0, 1)))  // n2 appends R (primary contact arms idle)
      .then(timeoutM(2))                                        // n2: spurious idle timeout -> SVC(v1)
      .then(deliverPrepareOkM(msgAt("prepareOk", 2, 0, 0, 0, 1))) // n0 COMMITS R (acks 0,3,2); installs E1
      .then(deliverSvcM(msgAt("svc", 2, 3, 1, 0, 0)))          // n3 joins -> VC(E0,v1), DVC -> n1
      .then(deliverCommitM(commitMsgAt(0, 2, 0, 0, 1)))        // n2 commits R; installs E1
      .then(clientRequestM(0))                                 // c1 @ (E1,v0), op 2
      .then(syncFromM(3, 0, 1))                                // n3 (demoted) crosses into E1
      .then(crashM(3))
      .then(deliverSvcM(msgAt("svc", 2, 1, 1, 0, 0)))          // n1 joins SVC(v1) -> VC(E0,v1) as its primary
      .then(clientRequestM(0))                                 // c2 @ (E1,v0), op 3
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 1, 2)))  // n2 appends c1
      .then(deliverPrepareOkM(msgAt("prepareOk", 2, 0, 0, 1, 2))) // n0 COMMITS c1 @ (E1,v0) (q(3)=2)
      .then(deliverDvcM(msgAt("dvc", 3, 1, 1, 0, 1)))          // n1 forms (E0,v1): log [R], logView 1
      .then(crashM(0))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 1, 3)))  // n2 appends c2
      .then(syncFromM(1, 2, 1))                                // n1 crosses into E1 at checkpoint 1 and
                                                               // starts E1 at view 0; no phantom primary
      .expect(st.get(1).cfg.epoch == 1 and st.get(1).view == 0
              and st.get(1).logView == 0 and not(primarySelf(st.get(1))) and agreement and noFailStop)

  // (A) Fail-stop: n2 learned c1 committed (commit 2) and then catches up to the phantom's higher view.
  run phantomFailStop =
    init
      .then(proposeM(0, "demote", 3, true))
      .then(deliverPrepareM(msgAt("prepare", 0, 3, 0, 0, 1)))
      .then(crashM(1))
      .then(restartM(1))
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 0, 1)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 3, 0, 0, 0, 1)))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 0, 1)))
      .then(timeoutM(2))
      .then(deliverPrepareOkM(msgAt("prepareOk", 2, 0, 0, 0, 1)))
      .then(deliverSvcM(msgAt("svc", 2, 3, 1, 0, 0)))
      .then(deliverCommitM(commitMsgAt(0, 2, 0, 0, 1)))
      .then(clientRequestM(0))
      .then(syncFromM(3, 0, 1))
      .then(crashM(3))
      .then(deliverSvcM(msgAt("svc", 2, 1, 1, 0, 0)))
      .then(clientRequestM(0))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 1, 2)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 2, 0, 0, 1, 2)))
      .then(deliverDvcM(msgAt("dvc", 3, 1, 1, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 2, 0, 1, 2)))        // n2 learns c1 committed
      .then(crashM(0))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 1, 3)))
      .then(syncFromM(1, 2, 1))                                // n1 enters E1/view 0 as a backup
      .expect(st.get(1).view == 0 and not(primarySelf(st.get(1))) and noFailStop and agreement)

  // (B) Silent loss: n2 never learned c1 committed; it adopts the phantom's short head, and a later
  //     view change forms from {n1, n2} (both log_view 1) and a new op overwrites c1.
  run phantomLoss =
    init
      .then(proposeM(0, "demote", 3, true))
      .then(deliverPrepareM(msgAt("prepare", 0, 3, 0, 0, 1)))
      .then(crashM(1))
      .then(restartM(1))
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 0, 1)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 3, 0, 0, 0, 1)))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 0, 1)))
      .then(timeoutM(2))
      .then(deliverPrepareOkM(msgAt("prepareOk", 2, 0, 0, 0, 1)))
      .then(deliverSvcM(msgAt("svc", 2, 3, 1, 0, 0)))
      .then(deliverCommitM(commitMsgAt(0, 2, 0, 0, 1)))
      .then(clientRequestM(0))
      .then(syncFromM(3, 0, 1))
      .then(crashM(3))
      .then(deliverSvcM(msgAt("svc", 2, 1, 1, 0, 0)))
      .then(clientRequestM(0))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 1, 2)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 2, 0, 0, 1, 2))) // c1 COMMITTED by n0 (client acked)
      .then(deliverDvcM(msgAt("dvc", 3, 1, 1, 0, 1)))
      .then(crashM(0))                                         // the only node that knows c1 committed
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 1, 3)))
      .then(syncFromM(1, 2, 1))                                // n1 enters E1/view 0 as a backup; the
                                                               // phantom-loss suffix is unreachable
      .expect(st.get(1).view == 0 and not(primarySelf(st.get(1))) and agreement and noFailStop)
}

// 3 voters + 1 genesis learner + 1 spare node that can be AddLearner'd.
module vr_sc_3v1l {
  import vr_common.* from "./vr_common"
  import vr_sc(
    NODES = Set(0, 1, 2, 3, 4),
    GENESIS = { epoch: 0, voters: [0, 1, 2], learners: [3] },
    MAX_LOG = 4,
    MAX_VIEW = 3,
    CHECKPOINT_OPS = 4,
    CROSS_FIX = true
  ).* from "./vr_sc"

  // ---- churn from a GROWN cluster (see vr_sc_solo): promote the genesis learner 3 (E1 = V[0,1,2,3]).
  run grow3v1l =
    init
      .then(requestProofM(0, 3))
      .then(deliverReqProofM(msgAt("reqProof", 0, 3, 0, 0, 0)))
      .then(deliverProofM(msgAt("proof", 3, 0, 0, 0, 0)))
      .then(proposeM(0, "promote", 3, false))                 // R: V[0,1,2] L[3] -> V[0,1,2,3]
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 0, 1)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 1, 0, 0, 0, 1))) // n0 commits R (q(3) = 2)
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 0, 1)))
      .then(deliverPrepareM(msgAt("prepare", 0, 3, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 1, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 2, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 3, 0, 0, 1)))        // all installed E1
      .expect(NODES.forall(n => not(st.get(n).started) or st.get(n).cfg.epoch == 1) and wGrew)
  run grow3v1lChurn = grow3v1l.then(25.reps(_ => stepChurn))
}

// The target e2e shape: a solo genesis voter that grows by add -> promote.
module vr_sc_solo {
  import vr_common.* from "./vr_common"
  import vr_sc(
    NODES = Set(0, 1, 2),
    GENESIS = { epoch: 0, voters: [0], learners: [] },
    MAX_LOG = 5,
    MAX_VIEW = 3,
    CHECKPOINT_OPS = 4,
    CROSS_FIX = true
  ).* from "./vr_sc"

  // ---- churn from a GROWN cluster: a scripted grow prefix, then a random stepChurn suffix (quint test
  // --max-samples N resolves the suffix N different ways; quint-connect replays every sample).
  // solo: add 1 -> bootstrap 1 -> same-epoch sync to the swap checkpoint -> proof -> promote 1 (E2 = V[0,1]).
  run growSolo =
    init
      .then(proposeM(0, "add", 1, false))                     // E1 = V[0] L[1] (solo commits at once)
      .then(bootstrapM(1, 0))
      .then(heartbeatM(0))                                     // n0's Commit carries checkpoint 1
      .then(deliverCommitM(commitMsgAt(0, 1, 0, 1, 1)))        // arms n1's ordinary sync to it
      .then(completeSyncM(1, 0))                               // n1 installs the swap checkpoint (op 1)
      .then(requestProofM(0, 1))
      .then(deliverReqProofM(msgAt("reqProof", 0, 1, 0, 1, 1)))
      .then(deliverProofM(msgAt("proof", 1, 0, 0, 1, 1)))
      .then(proposeM(0, "promote", 1, false))                 // E2 = V[0, 1] (predecessor quorum = n0)
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 1, 2)))
      .then(deliverCommitM(commitMsgAt(0, 1, 0, 1, 2)))        // n1 installs E2 as a voter
      .expect(NODES.forall(n => not(st.get(n).started) or st.get(n).cfg.epoch == 2) and wGrew)
  run growSoloChurn = growSolo.then(25.reps(_ => stepChurn))
}

// ---- The CANDIDATE-FIX twins (CROSS_FIX = true): same topologies, crossing does not carry view standing.
module vr_sc_4v_fix {
  import vr_common.* from "./vr_common"
  import vr_sc(
    NODES = Set(0, 1, 2, 3),
    GENESIS = { epoch: 0, voters: [0, 1, 2, 3], learners: [] },
    MAX_LOG = 4,
    MAX_VIEW = 3,
    CHECKPOINT_OPS = 4,
    CROSS_FIX = true
  ).* from "./vr_sc"

  // ---- churn from a GROWN cluster (see vr_sc_solo): demote 3, then promote it back (E2 = V[0,1,2,3]).
  run grow4v =
    init
      .then(proposeM(0, "demote", 3, false))                  // R1: V[0,1,2,3] -> V[0,1,2] L[3]
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 0, 1)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 1, 0, 0, 0, 1)))
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 0, 1)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 2, 0, 0, 0, 1))) // n0 commits R1 (q(4) = 3 + successor q)
      .then(deliverPrepareM(msgAt("prepare", 0, 3, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 1, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 2, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 3, 0, 0, 1)))        // all at E1, n3 a learner holding R1
      .then(requestProofM(0, 3))
      .then(deliverReqProofM(msgAt("reqProof", 0, 3, 0, 1, 1)))
      .then(deliverProofM(msgAt("proof", 3, 0, 0, 1, 1)))
      .then(proposeM(0, "promote", 3, false))                 // R2: V[0,1,2] L[3] -> V[0,1,2,3]
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 1, 2)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 1, 0, 0, 1, 2))) // n0 commits R2 (q(3) = 2)
      .expect(st.get(0).cfg.epoch == 2 and nVoters(st.get(0).cfg) == 4)
  run grow4vChurn = grow4v.then(25.reps(_ => stepChurn))
}

module vr_sc_3v1l_fix {
  import vr_common.* from "./vr_common"
  import vr_sc(
    NODES = Set(0, 1, 2, 3, 4),
    GENESIS = { epoch: 0, voters: [0, 1, 2], learners: [3] },
    MAX_LOG = 4,
    MAX_VIEW = 3,
    CHECKPOINT_OPS = 4,
    CROSS_FIX = true
  ).* from "./vr_sc"

  // ---- churn from a GROWN cluster (see vr_sc_solo): promote the genesis learner 3 (E1 = V[0,1,2,3]).
  run grow3v1l =
    init
      .then(requestProofM(0, 3))
      .then(deliverReqProofM(msgAt("reqProof", 0, 3, 0, 0, 0)))
      .then(deliverProofM(msgAt("proof", 3, 0, 0, 0, 0)))
      .then(proposeM(0, "promote", 3, false))                 // R: V[0,1,2] L[3] -> V[0,1,2,3]
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 0, 1)))
      .then(deliverPrepareOkM(msgAt("prepareOk", 1, 0, 0, 0, 1))) // n0 commits R (q(3) = 2)
      .then(deliverPrepareM(msgAt("prepare", 0, 2, 0, 0, 1)))
      .then(deliverPrepareM(msgAt("prepare", 0, 3, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 1, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 2, 0, 0, 1)))
      .then(deliverCommitM(commitMsgAt(0, 3, 0, 0, 1)))        // all installed E1
      .expect(NODES.forall(n => not(st.get(n).started) or st.get(n).cfg.epoch == 1) and wGrew)
  run grow3v1lChurn = grow3v1l.then(25.reps(_ => stepChurn))
}

module vr_sc_solo_fix {
  import vr_common.* from "./vr_common"
  import vr_sc(
    NODES = Set(0, 1, 2),
    GENESIS = { epoch: 0, voters: [0], learners: [] },
    MAX_LOG = 5,
    MAX_VIEW = 3,
    CHECKPOINT_OPS = 4,
    CROSS_FIX = true
  ).* from "./vr_sc"

  // ---- churn from a GROWN cluster: a scripted grow prefix, then a random stepChurn suffix (quint test
  // --max-samples N resolves the suffix N different ways; quint-connect replays every sample).
  // solo: add 1 -> bootstrap 1 -> same-epoch sync to the swap checkpoint -> proof -> promote 1 (E2 = V[0,1]).
  run growSolo =
    init
      .then(proposeM(0, "add", 1, false))                     // E1 = V[0] L[1] (solo commits at once)
      .then(bootstrapM(1, 0))
      .then(heartbeatM(0))                                     // n0's Commit carries checkpoint 1
      .then(deliverCommitM(commitMsgAt(0, 1, 0, 1, 1)))        // arms n1's ordinary sync to it
      .then(completeSyncM(1, 0))                               // n1 installs the swap checkpoint (op 1)
      .then(requestProofM(0, 1))
      .then(deliverReqProofM(msgAt("reqProof", 0, 1, 0, 1, 1)))
      .then(deliverProofM(msgAt("proof", 1, 0, 0, 1, 1)))
      .then(proposeM(0, "promote", 1, false))                 // E2 = V[0, 1] (predecessor quorum = n0)
      .then(deliverPrepareM(msgAt("prepare", 0, 1, 0, 1, 2)))
      .then(deliverCommitM(commitMsgAt(0, 1, 0, 1, 2)))        // n1 installs E2 as a voter
      .expect(NODES.forall(n => not(st.get(n).started) or st.get(n).cfg.epoch == 2) and wGrew)
  run growSoloChurn = growSolo.then(25.reps(_ => stepChurn))
}
