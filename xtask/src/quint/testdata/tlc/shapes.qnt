module shapes {
  var x: int

  action init = x' = 0
  action step = x' = (x + 1) % 3
  action stepUpTo2 = all { x < 2, x' = x + 1 }

  // initial-state invariant violation
  val initBad = x > 0
  // multi-state invariant violation
  val multiBad = x < 2
  val ok = x >= 0 and x < 3

  // temporal: lasso under fairness
  temporal lassoBad = weakFair(step, x) implies eventually(x == 5)
  // temporal without fairness: stuttering
  temporal stutterBad = eventually(x == 2)
  // temporal violated in the initial state
  temporal initTemporalBad = always(x > 0)
}

module dl {
  var x: int
  action init = x' = 0
  action step = all { x < 2, x' = x + 1 }
  val ok = x >= 0
}
