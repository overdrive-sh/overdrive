// -*- mode: Bluespec; -*-
// Teeth for SessionBecomesLive (../owner_flows.qnt) on the sessions-only instance (CFG "sess"), the instance the
// property is checked on. Expected verdicts are in ../checks.toml.

// K-A5S dropped: the guest owner never observes the end of the control connection it holds (D-UP-8), so it never
// re-opens a session, and a launch whose session ended before it went live, or was lost, never holds one again
module flows_sess_envGuestMissesSessionLoss { import owner_flows(OFF = Set("K-A5S"), CFG = "sess", REC = false).* from "../owner_flows" }
