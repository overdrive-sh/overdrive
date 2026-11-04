// -*- mode: Bluespec; -*-
// Teeth for ../guest_report.qnt. Expected verdicts are in ../checks.toml.

// D13 item 1: init writes EXIT without draining the listen events left in the ring (WriterExit enabled with the ring non-empty)
module gr_bugNoDrainBeforeExit { import guest_report(OFF = Set("noDrainBeforeExit"), REC = false).* from "../guest_report" }
