// -*- mode: Bluespec; -*-
// Teeth for steering concern 4 (../shared_table.qnt): each instance is the design with ONE rule
// switched off (OFF). Expected verdicts are in ../checks.toml.

// D8a-LOOKUP: the steering decision lives in the shared nft table
module table_bugSteeringInNft { import shared_table(OFF = Set("steeringInNft"), CFG = "ok").* from "../shared_table" }
// D8a-LOOKUP / ADR-0159: the steering is owned by the serve process (never pinned)
module table_bugUnpinned { import shared_table(OFF = Set("unpinned"), CFG = "ok").* from "../shared_table" }
// D8a-LOOKUP: the program also assigns non-loopback ingress
module table_bugNonLoopback { import shared_table(OFF = Set("nonLoopback"), CFG = "ok").* from "../shared_table" }
// D8a-HOLD: any recovery's restore reopens forwarding and ends every hold
module table_live_bugAnyRestore { import shared_table(OFF = Set("anyRestore"), CFG = "live").* from "../shared_table" }
