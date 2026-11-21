// -*- mode: Bluespec; -*-
// Teeth for ../selection_receipt.qnt: each instance is the rule set with ONE decision's rule switched off.
// Expected verdicts are in ../checks.toml.

// ADR-0233: the association is published Applied before the outer swap
module receipt_bugPublishEarly { import selection_receipt(OFF = Set("publishEarly"), REC = false, MAXUPD = 2).* from "../selection_receipt" }
// ADR-0347 (R-C): a different workload at a memoised endpoint reuses the BackendId but its association is not updated
module receipt_bugStaleAssoc { import selection_receipt(OFF = Set("staleAssoc"), REC = false, MAXUPD = 2).* from "../selection_receipt" }
// ADR-0232: take_receipt returns a receipt not bound to its cookie and ServiceKey
module receipt_bugTakeAny { import selection_receipt(OFF = Set("takeAny"), REC = false, MAXUPD = 2).* from "../selection_receipt" }
// R-59: update_service accepts an endpoint whose reverse-NAT key already maps to another Service
module receipt_bugRnatShare { import selection_receipt(OFF = Set("rnatShare"), REC = false, MAXUPD = 2).* from "../selection_receipt" }
