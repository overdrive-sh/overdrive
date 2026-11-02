// -*- mode: Bluespec; -*-
// Teeth for ../selection_receipt.qnt: each instance is the approved rule set with ONE decision's rule switched off.
// Expected verdicts are in ../checks.toml.

// ADR-0233: the association is published Applied before the outer swap
module receipt_bugPublishEarly { import selection_receipt(OFF = Set("publishEarly"), REC = false, MAXUPD = 2).* from "../selection_receipt" }
// ADR-0241: a live BackendId is rebound to another allocation
module receipt_bugRebind { import selection_receipt(OFF = Set("rebind"), REC = false, MAXUPD = 2).* from "../selection_receipt" }
// ADR-0232: take_receipt returns a receipt not bound to its cookie and ServiceKey
module receipt_bugTakeAny { import selection_receipt(OFF = Set("takeAny"), REC = false, MAXUPD = 2).* from "../selection_receipt" }
