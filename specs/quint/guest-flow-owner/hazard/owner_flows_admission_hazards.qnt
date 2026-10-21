// -*- mode: Bluespec; -*-
// Hazard instances of owner_flows (../owner_flows.qnt), CFG "core": the typed dial refusals (INB-1) and the activation
// gate (G-V3), the design with ONE rule switched off (OFF). A file of its own keeps the compiled IR of each hazard file
// within the Apalache server heap. Expected verdicts are in ../checks.toml.

// INB-1 off: a typed admission refusal registers (and closes) the dial slot instead of changing nothing
module flows_bugRefusalRegisters { import owner_flows(OFF = Set("refusalRegisters"), CFG = "core", REC = false).* from "../owner_flows" }
// G-V3 off: activate marks the CID Active without a live control session or while quiesced
module flows_bugActivateNoGuard { import owner_flows(OFF = Set("activateNoGuard"), CFG = "core", REC = false).* from "../owner_flows" }
