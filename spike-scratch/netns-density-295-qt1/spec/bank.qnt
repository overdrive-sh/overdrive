// Spike QT1 (D-UP-1). `conf_step` records the action that produced the state.
// ConfStep lives in its own module: constructors named like the actions would clash.
module bank_conf {
  type ConfStep =
    | Init
    | Deposit({ acct: str, amt: int })
    | Withdraw({ acct: str, amt: int })
    | Transfer({ amt: int })
}

module bank {
  import bank_conf.ConfStep from "./bank"

  // REC: does this instance record the action taken? (false = a model-checking instance)
  const REC: bool

  var a: int
  var b: int
  var conf_step: ConfStep // model-only, write-only

  def rec(v: ConfStep): ConfStep = if (REC) v else variant("Init", ())

  action Init = all { a' = 0, b' = 0, conf_step' = variant("Init", ()) }

  action Deposit(acct: str, amt: int): bool = all {
    amt > 0,
    a' = if (acct == "a") a + amt else a,
    b' = if (acct == "b") b + amt else b,
    conf_step' = rec(variant("Deposit", { acct: acct, amt: amt })),
  }

  action Withdraw(acct: str, amt: int): bool = all {
    amt > 0,
    if (acct == "a") a >= amt else b >= amt,
    a' = if (acct == "a") a - amt else a,
    b' = if (acct == "b") b - amt else b,
    conf_step' = rec(variant("Withdraw", { acct: acct, amt: amt })),
  }

  action Transfer(amt: int): bool = all {
    amt > 0,
    a >= amt,
    a' = a - amt,
    b' = b + amt,
    conf_step' = rec(variant("Transfer", { amt: amt })),
  }

  action init = Init
  action step = {
    nondet acct = Set("a", "b").oneOf()
    nondet amt = 1.to(3).oneOf()
    any { Deposit(acct, amt), Withdraw(acct, amt), Transfer(amt) }
  }

  val NonNeg = a >= 0 and b >= 0

  // named runs calling parameterised actions with literal args
  run DepositThenWithdraw =
    Init.then(Deposit("a", 3)).then(Withdraw("a", 2)).expect(NonNeg and a == 1 and b == 0)
  run DepositTransferDeposit =
    Init.then(Deposit("a", 2)).then(Transfer(2)).then(Deposit("b", 1)).then(Withdraw("b", 1))
      .expect(NonNeg and a == 0 and b == 2)
}

// the conformance instance: records conf_step
module bank_ok { import bank(REC = true).* from "./bank" }
// a model-checking instance: conf_step stays at its initial value
module bank_mc { import bank(REC = false).* from "./bank" }
// a qualified (`as`) instance, to see the ITF variable path
module bank_as {
  import bank(REC = true) as inst from "./bank"
  action init = inst::init
  action step = inst::step
  run AsDepositThenWithdraw = inst::DepositThenWithdraw
}
