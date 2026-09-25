// SPIKE (flat-cluster-intent-log, increment A): shared configuration algebra for the VR +
// SingleChange reconfiguration specs. Mirrors viewstamp-proto@f7fbcd96 `membership/mod.rs`:
//
//   * a configuration is an ORDERED voter prefix + an ORDERED learner suffix (slot layout);
//   * quorum(n)            = floor(n/2) + 1                  (replication quorum)
//   * quorum_view_change(n) = n - quorum(n) + 1              (SVC / DVC / nack quorum, flexible)
//   * primary(view)        = voters[view % n]                (slot remaps across an epoch swap)
//   * apply_delta: Promote moves a learner to the END of the voter range; Demote moves a voter to the
//     END of the learner range; AddLearner appends a learner; RemoveLearner closes the learner's slot.
//   * f(n) = n - quorum(n); a delta with f(succ) < f(pred) needs AcceptReducedFaultTolerance.
module vr_common {
  type Cfg = { epoch: int, voters: List[int], learners: List[int] }

  // Delta kinds: "promote" | "demote" | "add" | "remove". `who` is the member id.
  type Delta = { kind: str, who: int }

  pure def has(l: List[int], x: int): bool = l.indices().exists(i => l[i] == x)

  pure def setOf(l: List[int]): Set[int] = l.indices().map(i => l[i])

  pure def without(l: List[int], x: int): List[int] = l.select(y => y != x)

  pure def isVoter(c: Cfg, n: int): bool = has(c.voters, n)

  pure def isLearner(c: Cfg, n: int): bool = has(c.learners, n)

  pure def isMember(c: Cfg, n: int): bool = isVoter(c, n) or isLearner(c, n)

  pure def members(c: Cfg): Set[int] = setOf(c.voters).union(setOf(c.learners))

  pure def nVoters(c: Cfg): int = c.voters.length()

  pure def quorum(c: Cfg): int = nVoters(c) / 2 + 1

  pure def quorumVc(c: Cfg): int = nVoters(c) - quorum(c) + 1

  pure def faultTolerance(c: Cfg): int = nVoters(c) - quorum(c)

  pure def primaryOf(c: Cfg, view: int): int = c.voters[view % nVoters(c)]

  pure def isPrimaryIn(c: Cfg, view: int, n: int): bool = primaryOf(c, view) == n

  // Structural validity of a delta against `c` (the `MembershipError` screens of `apply_delta`).
  pure def deltaValid(c: Cfg, d: Delta): bool =
    if (d.kind == "promote") isLearner(c, d.who)
    else if (d.kind == "demote") isVoter(c, d.who) and nVoters(c) > 1
    else if (d.kind == "add") not(isMember(c, d.who))
    else if (d.kind == "remove") isLearner(c, d.who)
    else false

  // The successor configuration (epoch + 1). Only meaningful when `deltaValid(c, d)`.
  pure def applyDelta(c: Cfg, d: Delta): Cfg =
    if (d.kind == "promote")
      { epoch: c.epoch + 1, voters: c.voters.append(d.who), learners: without(c.learners, d.who) }
    else if (d.kind == "demote")
      { epoch: c.epoch + 1, voters: without(c.voters, d.who), learners: c.learners.append(d.who) }
    else if (d.kind == "add")
      { epoch: c.epoch + 1, voters: c.voters, learners: c.learners.append(d.who) }
    else
      { epoch: c.epoch + 1, voters: c.voters, learners: without(c.learners, d.who) }

  // `ReducedFaultToleranceUnacknowledged`: the successor tolerates fewer voter crashes.
  pure def reducesFaultTolerance(c: Cfg, d: Delta): bool =
    faultTolerance(applyDelta(c, d)) < faultTolerance(c)
}
