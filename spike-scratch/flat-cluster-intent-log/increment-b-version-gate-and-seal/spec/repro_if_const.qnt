// Minimal reproduction of the Quint -> Apalache discrepancy found while checking gate_seal.qnt:
// an action-level `if (cond and not(CONST)) A else B` where CONST = true. The simulator takes
// branch B (so `Inv` is violated at step 1); the question is whether Apalache agrees.
module repro_if_const {
  const BUG: bool
  var a: int
  var b: bool
  action init = all { a' = 0, b' = false }
  action step = {
    val x = 2
    all {
      if (x > 1 and not(BUG)) all { a' = a, b' = true }
      else all { a' = a + 1, b' = b },
    }
  }
  val Inv: bool = a == 0
}

module reproBugTrue { import repro_if_const(BUG = true).* }
module reproBugFalse { import repro_if_const(BUG = false).* }

// Same shape, but the condition folded into a val first.
module repro_if_val {
  const BUG: bool
  var a: int
  var b: bool
  action init = all { a' = 0, b' = false }
  action step = {
    val x = 2
    val halt = x > 1 and not(BUG)
    all {
      if (halt) all { a' = a, b' = true }
      else all { a' = a + 1, b' = b },
    }
  }
  val Inv: bool = a == 0
}
module reproValBugTrue { import repro_if_val(BUG = true).* }

// Same shape inside a nondet binding over a set, like `Apply`.
module repro_if_nondet {
  const BUG: bool
  var a: str -> int
  var b: str -> bool
  action init = all { a' = Set("p", "q").mapBy(n => 0), b' = Set("p", "q").mapBy(n => false) }
  action step = {
    nondet n = Set("p", "q").filter(m => a.get(m) < 1).oneOf()
    val x = 2
    all {
      if (x > 1 and not(BUG)) all { a' = a, b' = b.set(n, true) }
      else all { a' = a.set(n, a.get(n) + 1), b' = b },
    }
  }
  val Inv: bool = Set("p", "q").forall(n => a.get(n) == 0)
}
module reproNondetBugTrue { import repro_if_nondet(BUG = true).* }
