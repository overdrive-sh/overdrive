// -*- mode: Bluespec; -*-
/**
 * Increment B probe — committed cluster-version gate + "Delos-lite" epoch seal.
 *
 * Abstraction level: EPOCHS and the COMMITTED LOG, not messages. Ordering *within* one epoch is
 * assumed correct (that is VR's job; viewstamp's VOPR covers it). What is modelled is the machinery
 * layered on top:
 *
 *   - the version gate: each node's binary supports versions up to `supported(n)` (binaries only
 *     upgrade). A gate entry `ClusterVersion(v)` is proposed, then committed. Entries acknowledged
 *     after it carry format tag v. A node that meets an entry it cannot interpret HALTS.
 *   - the epoch seal: an epoch ends with a committed seal entry naming exactly one successor
 *     (voter set + pinned wire version). The seal is committed by a quorum Q of the epoch's voters
 *     and is an acknowledgement FENCE: a voter in Q refuses further appends in that epoch. The
 *     successor epoch starts from the sealed prefix (the content-addressed boundary).
 *   - appliers: every node (voters and learners alike) applies the authoritative virtual log — the
 *     epochs' segments, each cut at its seal — through a hash-verified fetch.
 *
 * Encoding choices for Apalache: the authoritative log `vlog` and the acknowledgement-order log
 * `acklog` are state variables maintained incrementally (an acknowledgement in a sealed epoch — a
 * "zombie" — lands in `acklog` but not in `vlog`, because the successor already started from the
 * sealed boundary). Facts that must be judged AT THE MOMENT OF COMMIT/APPLY (did every voter support
 * the gate? did the applier receive the committed entry at that position?) are recorded as ghost
 * fields evaluated independently of the action's guard.
 *
 * Each BUG_* constant switches ONE mechanism off; the correct model sets all of them to false.
 */
module gate_seal {
  // ------------------------------------------------------------------ parameters
  const NODES: Set[str]
  const INIT_VOTERS: Set[str]
  const VOTER_COUNT: int    // voter-set size every epoch keeps
  const MAXV: int           // highest version any binary can support
  const MAXE: int           // highest epoch index (bounds the number of seals)
  const MAXOPS: int         // bound on acknowledged client ops

  // bug switches (all false in the correct model)
  const BUG_GATE_MAJORITY: bool           // gate commits when a MAJORITY of voters support v
  const BUG_GATE_PROPOSE_TIME_ONLY: bool  // gate checked only at propose time; no epoch fence at commit
  const BUG_NO_SEAL_FENCE: bool           // accepting the seal does not close the epoch for appends
  const BUG_UNVERIFIED_FETCH: bool        // appliers take a peer's raw acknowledgement stream
  const BUG_ADMIT_UNSUPPORTED_VOTER: bool // a seal may seat a voter lacking the committed version
  const BUG_RESEAL: bool                  // an already-sealed epoch may be sealed again
  const BUG_APPLY_UNSUPPORTED: bool       // an applier applies an entry it cannot interpret

  // ------------------------------------------------------------------ types
  type Entry = {
    kind: str,         // "op" | "gate" | "seal"
    epoch: int,        // epoch in which it was acknowledged by that epoch's quorum
    tag: int,          // entry-format version an applier must support
    target: int,       // gate: announced version; seal: the successor epoch it started; else 0
    okAtCommit: bool,  // gate ghost: every voter of `epoch` supported `target` at commit time
    id: int,           // unique acknowledgement id
  }
  type Pending = { active: bool, target: int, epoch: int }

  // ------------------------------------------------------------------ state
  var supported: str -> int      // max version the node's binary supports (monotone)
  var voters: int -> Set[str]    // epoch -> voter set (empty until the epoch starts)
  var wire: int -> int           // epoch -> pinned wire version
  var parent: int -> int         // epoch -> epoch whose seal started it (-1 for genesis/unstarted)
  var curEpoch: int              // newest started epoch
  var ackCount: int -> int       // epoch -> entries acknowledged in that epoch (zombies included)
  var segLen: int -> int         // epoch -> entries of that epoch in the authoritative log
  var sealCount: int -> int      // epoch -> seal entries acknowledged in that epoch
  var closedFor: str -> int      // highest epoch the node has fenced (refuses appends for <= it)
  var clusterVersion: int        // committed cluster version
  var pending: Pending           // in-flight gate proposal
  var vlog: List[Entry]          // the authoritative virtual log (sealed-bounded segments, in order)
  var acklog: List[Entry]        // every acknowledged entry in acknowledgement order
  var applied: str -> int        // how many entries of its fetch source each node has applied
  var halted: str -> bool        // node stopped applying: next entry needs a version it lacks
  var appliedAbove: str -> bool  // ghost: node applied some entry it could not interpret
  var diverged: str -> bool      // ghost: node applied an entry that is not the committed one there
  var nextId: int
  var opsDone: int

  // ------------------------------------------------------------------ helpers
  pure def quorum(s: Set[str]): int = s.size() / 2 + 1
  pure def imax(a: int, b: int): int = if (a > b) a else b

  pure def mkOp(e: int, tag: int, id: int): Entry =
    { kind: "op", epoch: e, tag: tag, target: 0, okAtCommit: true, id: id }
  pure def mkGate(e: int, v: int, ok: bool, id: int): Entry =
    { kind: "gate", epoch: e, tag: v, target: v, okAtCommit: ok, id: id }
  pure def mkSeal(e: int, tag: int, succ: int, id: int): Entry =
    { kind: "seal", epoch: e, tag: tag, target: succ, okAtCommit: true, id: id }

  pure val NoPending: Pending = { active: false, target: 0, epoch: -1 }
  pure val EPOCHS: Set[int] = 0.to(MAXE)
  pure val SuccCandidates: Set[Set[str]] = NODES.powerset().filter(s => s.size() == VOTER_COUNT)

  def started(e: int): bool = e <= curEpoch
  def sealed(e: int): bool = sealCount.get(e) > 0

  // A voter of epoch e accepts appends for e unless it has fenced e, and only if it speaks e's wire.
  def accepts(n: str, e: int): bool = and {
    voters.get(e).contains(n),
    closedFor.get(n) < e,
    supported.get(n) >= wire.get(e),
  }
  def acceptors(e: int): Set[str] = voters.get(e).filter(n => accepts(n, e))
  def canAppend(e: int): bool = started(e) and acceptors(e).size() >= quorum(voters.get(e))

  // The log an applier fetches from: hash-verified committed log, or a peer's raw stream (bug).
  def src: List[Entry] = if (BUG_UNVERIFIED_FETCH) acklog else vlog

  // Version an applier needs for entry x: its format tag and its epoch's pinned wire version.
  def needs(x: Entry): int = imax(x.tag, wire.get(x.epoch))

  def supporters(v: int): Set[str] = NODES.filter(n => supported.get(n) >= v)
  def allSupport(vs: Set[str], v: int): bool = vs.subseteq(supporters(v))

  // The gate rule: every voter supports v (correct) or a majority does (BUG_GATE_MAJORITY).
  def gateOk(vs: Set[str], v: int): bool =
    if (BUG_GATE_MAJORITY) vs.filter(n => supported.get(n) >= v).size() >= quorum(vs)
    else allSupport(vs, v)

  // Admissible successor (voter set, wire) at a seal.
  def successorOk(sv: Set[str], sw: int): bool = and {
    sw <= clusterVersion,                   // P1 follows P2: never pin a wire past the committed gate
    sv.forall(n => supported.get(n) >= sw), // successor voters speak the pinned wire
    BUG_ADMIT_UNSUPPORTED_VOTER or allSupport(sv, clusterVersion),
  }

  // Acknowledge entry x in epoch e: it joins the authoritative log iff e is the active, unsealed
  // epoch; otherwise (a zombie in a sealed epoch) the successor already started without it.
  action ack(e: int, x: Entry): bool = all {
    acklog' = acklog.append(x),
    ackCount' = ackCount.set(e, ackCount.get(e) + 1),
    if (e == curEpoch and not(sealed(e)))
      all { vlog' = vlog.append(x), segLen' = segLen.set(e, segLen.get(e) + 1) }
    else
      all { vlog' = vlog, segLen' = segLen },
  }

  // ------------------------------------------------------------------ frame helpers
  action keepApply = all {
    applied' = applied, halted' = halted, appliedAbove' = appliedAbove, diverged' = diverged,
  }
  action keepEpochs = all {
    voters' = voters, wire' = wire, parent' = parent, curEpoch' = curEpoch,
    sealCount' = sealCount, closedFor' = closedFor,
  }
  action keepLog = all {
    vlog' = vlog, acklog' = acklog, ackCount' = ackCount, segLen' = segLen, nextId' = nextId,
  }

  // ------------------------------------------------------------------ actions
  action init = all {
    supported' = NODES.mapBy(n => 1),
    voters' = EPOCHS.mapBy(e => if (e == 0) INIT_VOTERS else Set()),
    wire' = EPOCHS.mapBy(e => 1),
    parent' = EPOCHS.mapBy(e => -1),
    curEpoch' = 0,
    ackCount' = EPOCHS.mapBy(e => 0),
    segLen' = EPOCHS.mapBy(e => 0),
    sealCount' = EPOCHS.mapBy(e => 0),
    closedFor' = NODES.mapBy(n => -1),
    clusterVersion' = 1,
    pending' = NoPending,
    vlog' = List(),
    acklog' = List(),
    applied' = NODES.mapBy(n => 0),
    halted' = NODES.mapBy(n => false),
    appliedAbove' = NODES.mapBy(n => false),
    diverged' = NODES.mapBy(n => false),
    nextId' = 0,
    opsDone' = 0,
  }

  // Operator replaces a node's binary with one supporting MAXV; a halted node restarts and resumes.
  action Upgrade = {
    nondet n = NODES.filter(m => supported.get(m) < MAXV).oneOf()
    all {
      supported' = supported.set(n, MAXV),
      halted' = halted.set(n, false),
      applied' = applied, appliedAbove' = appliedAbove, diverged' = diverged,
      clusterVersion' = clusterVersion, pending' = pending, opsDone' = opsDone,
      keepEpochs, keepLog,
    }
  }

  // A primary of ANY started epoch (a stale or zombie primary included) tries to append a client op.
  // It is acknowledged iff a quorum of that epoch's voters accepts. The fence is what stops a zombie.
  action AppendOp = {
    nondet e = EPOCHS.oneOf()
    all {
      opsDone < MAXOPS,
      canAppend(e),
      ack(e, mkOp(e, clusterVersion, nextId)),
      nextId' = nextId + 1,
      opsDone' = opsDone + 1,
      supported' = supported, clusterVersion' = clusterVersion, pending' = pending,
      keepEpochs, keepApply,
    }
  }

  // A proposer checks the voter set it can see and puts ClusterVersion(v) in flight.
  action ProposeGate = {
    // NB: a CONSTANT range plus a guard. `(clusterVersion + 1).to(MAXV)` (a state-dependent lower
    // bound) silently lost behaviours under Apalache 0.56.1 — see evidence/apalache-range-hazard.txt.
    nondet v = 1.to(MAXV).oneOf()
    all {
      v > clusterVersion,
      not(pending.active),
      gateOk(voters.get(curEpoch), v),
      pending' = { active: true, target: v, epoch: curEpoch },
      supported' = supported, clusterVersion' = clusterVersion, opsDone' = opsDone,
      keepEpochs, keepLog, keepApply,
    }
  }

  // The gate entry is appended in the active epoch. Correct: re-checked at commit against the voters
  // of the epoch it commits in, and only in the epoch it was proposed in (the epoch fence).
  action CommitGate = all {
    pending.active,
    canAppend(curEpoch),
    BUG_GATE_PROPOSE_TIME_ONLY or (pending.epoch == curEpoch and gateOk(voters.get(curEpoch), pending.target)),
    ack(curEpoch, mkGate(curEpoch, pending.target, allSupport(voters.get(curEpoch), pending.target), nextId)),
    nextId' = nextId + 1,
    clusterVersion' = pending.target,
    pending' = NoPending,
    supported' = supported, opsDone' = opsDone,
    keepEpochs, keepApply,
  }

  // Correct model: a proposal from an older epoch is abandoned (it must be re-proposed and re-checked).
  action DropStaleGate = all {
    not(BUG_GATE_PROPOSE_TIME_ONLY),
    pending.active,
    pending.epoch != curEpoch,
    pending' = NoPending,
    supported' = supported, clusterVersion' = clusterVersion, opsDone' = opsDone,
    keepEpochs, keepLog, keepApply,
  }

  // Seal epoch e: the seal entry naming exactly one successor is committed by a quorum Q of e's
  // voters; every voter in Q fences e. The successor epoch starts from the sealed prefix.
  action Seal = {
    nondet e = EPOCHS.oneOf()
    nondet sv = SuccCandidates.oneOf()
    nondet sw = 1.to(MAXV).oneOf()
    nondet Q = voters.get(e).powerset().oneOf()
    all {
      curEpoch < MAXE,
      if (BUG_RESEAL) started(e) else e == curEpoch,
      Q.subseteq(acceptors(e)),
      Q.size() >= quorum(voters.get(e)),
      successorOk(sv, sw),
      ack(e, mkSeal(e, clusterVersion, curEpoch + 1, nextId)),
      nextId' = nextId + 1,
      sealCount' = sealCount.set(e, sealCount.get(e) + 1),
      closedFor' =
        if (BUG_NO_SEAL_FENCE) closedFor
        else NODES.mapBy(n => if (Q.contains(n)) imax(closedFor.get(n), e) else closedFor.get(n)),
      voters' = voters.set(curEpoch + 1, sv),
      wire' = wire.set(curEpoch + 1, sw),
      parent' = parent.set(curEpoch + 1, e),
      curEpoch' = curEpoch + 1,
      supported' = supported, clusterVersion' = clusterVersion, pending' = pending,
      opsDone' = opsDone,
      keepApply,
    }
  }

  // A node hears of the newest epoch (VR: any message with a later epoch) and fences older epochs.
  action LearnEpoch = {
    nondet n = NODES.filter(m => closedFor.get(m) < curEpoch - 1).oneOf()
    all {
      not(BUG_NO_SEAL_FENCE),
      closedFor' = closedFor.set(n, curEpoch - 1),
      voters' = voters, wire' = wire, parent' = parent, curEpoch' = curEpoch, sealCount' = sealCount,
      supported' = supported, clusterVersion' = clusterVersion, pending' = pending, opsDone' = opsDone,
      keepLog, keepApply,
    }
  }

  // A node applies the next entry of the log it fetches — or halts if it cannot interpret it.
  action Apply = {
    nondet n = NODES.filter(m => not(halted.get(m)) and applied.get(m) < src.length()).oneOf()
    val i = applied.get(n)
    val x = src[i]
    val committedHere = i < vlog.length() and vlog[i].id == x.id
    all {
      if (needs(x) > supported.get(n) and not(BUG_APPLY_UNSUPPORTED))
        all {
          halted' = halted.set(n, true),
          applied' = applied, appliedAbove' = appliedAbove, diverged' = diverged,
        }
      else
        all {
          applied' = applied.set(n, i + 1),
          appliedAbove' = appliedAbove.set(n, appliedAbove.get(n) or needs(x) > supported.get(n)),
          diverged' = diverged.set(n, diverged.get(n) or not(committedHere)),
          halted' = halted,
        },
      supported' = supported, clusterVersion' = clusterVersion, pending' = pending, opsDone' = opsDone,
      keepEpochs, keepLog,
    }
  }

  // Debug harness: the state right after the gate commits in epoch 0 (voters upgraded, n4 not).
  action debugPostGateInit = all {
    supported' = NODES.mapBy(n => if (INIT_VOTERS.contains(n)) MAXV else 1),
    voters' = EPOCHS.mapBy(e => if (e == 0) INIT_VOTERS else Set()),
    wire' = EPOCHS.mapBy(e => 1),
    parent' = EPOCHS.mapBy(e => -1),
    curEpoch' = 0,
    ackCount' = EPOCHS.mapBy(e => if (e == 0) 1 else 0),
    segLen' = EPOCHS.mapBy(e => if (e == 0) 1 else 0),
    sealCount' = EPOCHS.mapBy(e => 0),
    closedFor' = NODES.mapBy(n => -1),
    clusterVersion' = MAXV,
    pending' = NoPending,
    vlog' = [mkGate(0, MAXV, true, 0)],
    acklog' = [mkGate(0, MAXV, true, 0)],
    applied' = NODES.mapBy(n => 0),
    halted' = NODES.mapBy(n => false),
    appliedAbove' = NODES.mapBy(n => false),
    diverged' = NODES.mapBy(n => false),
    nextId' = 1,
    opsDone' = 0,
  }

  // Debug harness: one step earlier — voters upgraded, gate proposed but not committed.
  action debugPreCommitInit = all {
    supported' = NODES.mapBy(n => if (INIT_VOTERS.contains(n)) MAXV else 1),
    voters' = EPOCHS.mapBy(e => if (e == 0) INIT_VOTERS else Set()),
    wire' = EPOCHS.mapBy(e => 1),
    parent' = EPOCHS.mapBy(e => -1),
    curEpoch' = 0,
    ackCount' = EPOCHS.mapBy(e => 0),
    segLen' = EPOCHS.mapBy(e => 0),
    sealCount' = EPOCHS.mapBy(e => 0),
    closedFor' = NODES.mapBy(n => -1),
    clusterVersion' = 1,
    pending' = { active: true, target: MAXV, epoch: 0 },
    vlog' = List(),
    acklog' = List(),
    applied' = NODES.mapBy(n => 0),
    halted' = NODES.mapBy(n => false),
    appliedAbove' = NODES.mapBy(n => false),
    diverged' = NODES.mapBy(n => false),
    nextId' = 0,
    opsDone' = 0,
  }
  action debugCommitThenApply = any { CommitGate, Apply }
  // Debug harness: voters upgraded, nothing proposed yet.
  action debugPreProposeInit = all {
    supported' = NODES.mapBy(n => if (INIT_VOTERS.contains(n)) MAXV else 1),
    voters' = EPOCHS.mapBy(e => if (e == 0) INIT_VOTERS else Set()),
    wire' = EPOCHS.mapBy(e => 1),
    parent' = EPOCHS.mapBy(e => -1),
    curEpoch' = 0,
    ackCount' = EPOCHS.mapBy(e => 0),
    segLen' = EPOCHS.mapBy(e => 0),
    sealCount' = EPOCHS.mapBy(e => 0),
    closedFor' = NODES.mapBy(n => -1),
    clusterVersion' = 1,
    pending' = NoPending,
    vlog' = List(),
    acklog' = List(),
    applied' = NODES.mapBy(n => 0),
    halted' = NODES.mapBy(n => false),
    appliedAbove' = NODES.mapBy(n => false),
    diverged' = NODES.mapBy(n => false),
    nextId' = 0,
    opsDone' = 0,
  }

  action Stutter = all {
    supported' = supported, clusterVersion' = clusterVersion, pending' = pending, opsDone' = opsDone,
    keepEpochs, keepLog, keepApply,
  }

  action step = any {
    Upgrade, AppendOp, ProposeGate, CommitGate, DropStaleGate, Seal, LearnEpoch, Apply, Stutter,
  }

  // ------------------------------------------------------------------ invariants

  // (1) No node applies an entry above its supported version (it halts instead) ...
  val NoApplyAboveSupported: bool = NODES.forall(n => not(appliedAbove.get(n)))
  // ... and a halt is only ever caused by such an entry.
  val HaltOnlyWhenBlocked: bool =
    NODES.forall(n => halted.get(n) implies and {
      applied.get(n) < src.length(),
      needs(src[applied.get(n)]) > supported.get(n),
    })

  // (2) A gate entry is acknowledged only if EVERY voter of the epoch it commits in supported it.
  val GateOnlyWithAllVoters: bool =
    acklog.indices().forall(i => acklog[i].kind == "gate" implies acklog[i].okAtCommit)
  // (2b) Stronger state invariant: the active voter set always supports the committed version.
  val VotersSupportClusterVersion: bool =
    voters.get(curEpoch).forall(n => supported.get(n) >= clusterVersion)

  // (3) No append is acknowledged in an epoch after its seal.
  val NoAckAfterSeal: bool =
    EPOCHS.forall(e => (started(e) and sealed(e)) implies ackCount.get(e) == segLen.get(e))

  // (4) Each sealed epoch has exactly one successor, the chain has no forks, and the active epoch
  //     is unsealed.
  val UniqueSuccessor: bool = and {
    EPOCHS.forall(k => (k >= 1 and started(k)) implies parent.get(k) == k - 1),
    EPOCHS.forall(e => started(e) implies sealCount.get(e) <= 1),
    EPOCHS.forall(e => e < curEpoch implies sealCount.get(e) == 1),
    sealCount.get(curEpoch) == 0,
  }

  // (5) The committed prefix is preserved across epochs: every acknowledged entry is in the
  //     authoritative log, which the successor started from.
  val CommittedPrefixPreserved: bool =
    acklog.length() == vlog.length() and acklog.indices().forall(i => acklog[i].id == vlog[i].id)

  // (6) Two epochs never both have a primary able to commit a (client) append.
  val NoTwoCommittingPrimaries: bool =
    EPOCHS.forall(e1 => EPOCHS.forall(e2 =>
      (e1 < e2 and started(e2)) implies not(canAppend(e1) and canAppend(e2))))

  // (7) A learner never diverges: every entry it applied is the committed entry at that position;
  //     it is a prefix of the committed log, or halted at one.
  val LearnerNoDivergence: bool = NODES.forall(n => not(diverged.get(n)))

  val AllSafety: bool = and {
    NoApplyAboveSupported, HaltOnlyWhenBlocked, GateOnlyWithAllVoters, VotersSupportClusterVersion,
    NoAckAfterSeal, UniqueSuccessor, CommittedPrefixPreserved, NoTwoCommittingPrimaries,
    LearnerNoDivergence,
  }

  // ------------------------------------------------------------------ liveness
  def allUpgraded: bool = NODES.forall(n => supported.get(n) == MAXV)
  // Once every node runs the new binary, the gate eventually commits — under weak fairness of the
  // gate's own propose / commit / abandon-stale steps (everything else may interleave arbitrarily).
  temporal GateEventuallyCommits =
    and {
      weakFair(ProposeGate, (pending, clusterVersion)),
      weakFair(CommitGate, (pending, clusterVersion)),
      weakFair(DropStaleGate, pending),
    }.implies(always(allUpgraded.implies(eventually(clusterVersion == MAXV))))

  // ------------------------------------------------------------------ reachability witnesses
  // Each is EXPECTED to be violated in the correct model: a violation proves the state is reachable,
  // so the safety results above are not vacuous.
  val WitnessGateNeverCommits: bool = clusterVersion == 1
  // A seal committed by a PARTIAL quorum leaves a voter of the sealed epoch unfenced (a would-be
  // zombie primary's only ally) — exercises Seal's state-dependent `powerset` nondet under Apalache.
  val WitnessNoPartialSealQuorum: bool =
    not(EPOCHS.exists(e => e < curEpoch and voters.get(e).exists(n => closedFor.get(n) < e)))
  val WitnessNoApplyAfterGate: bool = not(clusterVersion == MAXV and NODES.exists(n => applied.get(n) > 0))
  val WitnessNoSealAfterGate: bool = not(clusterVersion == MAXV and curEpoch > 0)
  val DebugPostGateUnreached: bool =
    not(clusterVersion == MAXV and supported.get("n4") == 1 and applied.get("n4") == 0
        and vlog.length() == 1 and not(halted.get("n4")))
  val DebugN4NeverAppliedGate: bool =
    not(applied.get("n4") > 0 and vlog.length() > 0 and vlog[0].kind == "gate")
  val WitnessNoLearnerApplyAfterGate: bool =
    not(NODES.exists(n => not(INIT_VOTERS.contains(n)) and applied.get(n) > 0) and clusterVersion == MAXV)
  val WitnessNoLearnerHaltAtGate: bool =
    not(NODES.exists(n => halted.get(n) and not(voters.get(curEpoch).contains(n))
                          and src[applied.get(n)].kind == "gate"))
  val WitnessNoWireBumpedEpoch: bool = not(EPOCHS.exists(e => started(e) and wire.get(e) == MAXV))
  val WitnessNoVoterChange: bool = EPOCHS.forall(e => started(e) implies voters.get(e) == INIT_VOTERS)
  val WitnessNoUpgradedLearnerResumed: bool =
    not(NODES.exists(n => not(INIT_VOTERS.contains(n)) and applied.get(n) == vlog.length()
                          and clusterVersion == MAXV and vlog.length() > 0))
}

// ======================================================================= model instances
// ok4: 4 nodes = 3 voters + 1 learner; versions 1..2; epochs 0..2.
module ok4 {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// ok5: 5 nodes = 3 voters + 2 learners.
module ok5 {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4", "n5"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// ok3: 3 nodes, all voters (no learner) — the smallest configuration.
module ok3 {
  import gate_seal(
    NODES = Set("n1", "n2", "n3"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B1 — the gate commits when a MAJORITY of voters support it, not all of them.
module bugMajority {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = true, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B2 — the gate is checked only when proposed. This is the shape viewstamp's current API forces on
// an application-level gate: `StateMachine::apply(op, body)` sees no membership, and a client
// `Request` carries no epoch, so nothing re-checks the voter set at the commit position.
module bugProposeTime {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = true, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B3 — accepting the seal does not fence the epoch (a stale primary keeps its quorum).
module bugNoFence {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = true,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B4 — no fence AND appliers take a peer's raw stream without verifying it against the seal record.
module bugUnverifiedFetch {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = true,
    BUG_UNVERIFIED_FETCH = true, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B4' — raw stream, fence intact. Expected: NO violation — with the fence, the raw stream and the
// committed log are identical; verification is defence in depth.
module bugUnverifiedFetchFenced {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = true, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B5 — a seal may seat a successor voter whose binary lacks the committed version (membership
// admission not coupled to the gate).
module bugAdmitUnsupported {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = true, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B6 — re-sealing an already-sealed epoch allowed, fence off: two successors.
module bugResealNoFence {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = true,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = true,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B6' — re-sealing allowed, fence intact. Expected: NO violation — the fence alone makes the
// successor unique.
module bugResealFenced {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = true,
    BUG_APPLY_UNSUPPORTED = false
  ).*
}

// B7 — an applier applies an entry it cannot interpret instead of halting.
module bugApplyUnsupported {
  import gate_seal(
    NODES = Set("n1", "n2", "n3", "n4"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = true
  ).*
}
