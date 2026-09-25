// Minimal reproduction attempt of the toolchain hazard found in gate_seal.qnt: a `nondet` drawn
// from an integer range whose LOWER BOUND IS A STATE VARIABLE, `(c + 1).to(MAX).oneOf()`, then used
// in a later step's comparison. `rangeState` is the hazardous form, `rangeConst` the fixed form
// (constant range + guard). `Inv` is violated after two steps under a faithful semantics.
module repro_range {
  const STATE_BOUND: bool
  pure val MAX = 2
  var c: int        // committed version
  var tag: int      // tag of the entry committed at c
  var applied: bool // a v1-only applier applied it without halting
  action init = all { c' = 1, tag' = 0, applied' = false }
  action commitStateBound = {
    nondet v = (c + 1).to(MAX).oneOf()
    all { STATE_BOUND, v > c, c' = v, tag' = v, applied' = applied }
  }
  action commitConstBound = {
    nondet v = 1.to(MAX).oneOf()
    all { not(STATE_BOUND), v > c, c' = v, tag' = v, applied' = applied }
  }
  action commit = any { commitStateBound, commitConstBound }
  // A v1-only applier "applies" the entry iff it needs more than it supports (the B7 bug shape).
  action apply = all { tag > 0, c' = c, tag' = tag, applied' = tag > 1 }
  action step = any { commit, apply }
  val Inv: bool = not(applied)
}
module rangeState { import repro_range(STATE_BOUND = true).* }
module rangeConst { import repro_range(STATE_BOUND = false).* }
