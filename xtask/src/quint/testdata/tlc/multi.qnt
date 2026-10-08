module multi {
  var x: int
  var big: Set[{ name: str, offset: int, state: str }]

  action init = all {
    x' = 0,
    big' = 0.to(6).map(i => { name: "allocation-number", offset: i, state: "waiting-for-device" }),
  }
  action step = all { x' = (x + 1) % 3, big' = big }

  val initBad = x > 0
  val multiBad = x < 2
  temporal lassoBad = weakFair(step, x) implies eventually(x == 5)
}
