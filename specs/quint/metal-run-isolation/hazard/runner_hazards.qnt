// One weakened rule for each protocol decision modeled here, P2 and P5.
module runner_bugOrdinaryExclusive {
  import runner_protocol(OFF = Set("ordinaryExclusive"), REC = false).* from "../runner_protocol"
}
module runner_bugCollectAtTerminal {
  import runner_protocol(OFF = Set("collectAtTerminal"), REC = false).* from "../runner_protocol"
}
