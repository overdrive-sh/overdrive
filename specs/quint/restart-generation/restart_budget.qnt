/**
 * The restart budget rule (R-1 = A, ADR-0335 item 4): ONE rule, owned here, that every spec of a restarting workload imports
 * (specs/quint/restart-generation/ and specs/quint/node-local-volume/) and that both conformance replays check against
 * production's `WorkloadLifecycle` (`workload_lifecycle.rs:51-52`, `:1316-1321`, `:1431`).
 *
 * The budget is production's PER-ALLOCATION count (the View's `restart_counts` at issue), not a cumulative tally:
 *   - a fresh placement (no current allocation row) starts at 0;
 *   - a crash restart (the current row is Failed, no generation pending) carries its predecessor's count + 1;
 *   - a generation replacement (`workload restart`, a resume after a stop) carries its predecessor's count UNCHANGED, so a
 *     Failed predecessor is forgiven: the failure it carried stops counting once the replacement has a row.
 * `charged` is the number of failures counted against the budget: the current row's count, plus one when that row is Failed.
 * Production restarts a failed row iff its count is below the ceiling, with a generation replacement admitted regardless, so a
 * chain fails CEILING + 1 times before BackoffExhausted: LIMIT = CEILING + 1 in these terms. Exhaustion is `charged >= LIMIT`
 * with nothing running. `charged` and `exhausted` are DERIVED from the current row's count, never state.
 *
 * Definitions only: no state, no `step`, never a conformance target. The importing spec decides what "the current row" is
 * (production `current_alloc`: the highest allocation id with a row) and supplies it through the arguments.
 */
module restart_budget {
  // The current row is `cur` (0: none), its count is `cnt.get(cur)`, read only when there is one (the argument is lazy: a
  // caller's `curFailed` must not read a row of allocation 0 either).
  // failures counted against the budget: 0 with no current row, else the current row's count (+ 1 when it is Failed)
  def budgetCharged(cur: int, cnt: int -> int, curFailed: bool): int =
    if (cur == 0) 0 else cnt.get(cur) + (if (curFailed) 1 else 0)

  // BackoffExhausted: the budget is spent and nothing runs
  def budgetExhausted(charged: int, limit: int, running: bool): bool = charged >= limit and not(running)

  // the count the next issued allocation carries: 0 for a fresh placement, the predecessor's unchanged for a generation
  // replacement, the predecessor's + 1 for a crash restart
  def budgetStartCnt(cur: int, cnt: int -> int, pendingGen: bool): int =
    if (cur == 0) 0
    else if (pendingGen) cnt.get(cur)
    else cnt.get(cur) + 1

  // a placement is admitted below the ceiling; a generation replacement is admitted whatever the count
  def budgetMayRestart(charged: int, limit: int, pendingGen: bool): bool = charged < limit or pendingGen
}
