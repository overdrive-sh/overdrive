// -*- mode: Bluespec; -*-
// Hazard instances of owner_flows (../owner_flows.qnt), CFG "beacon": the design with ONE rule switched off
// (OFF). Expected verdicts are in ../checks.toml.

// ATTR-LIVE-CLAIM off for the beacon: READY compared with the lease's token, without the live check
module flows_bugBeaconNoLiveRecheck { import owner_flows(OFF = Set("beaconNoLiveRecheck"), CFG = "beacon").* from "../owner_flows" }
// ATTR-TOKEN off for the beacon: READY attributed by peer CID alone
module flows_bugBeaconPeerCidOnly { import owner_flows(OFF = Set("beaconPeerCidOnly"), CFG = "beacon").* from "../owner_flows" }
// ADR-0157 off: driver.stop closes the beacon session but does not release the CID's beacon claim — the next launch's READY is held out forever
module flows_bugDriverStopKeepsClaim { import owner_flows(OFF = Set("driverStopKeepsClaim"), CFG = "beacon").* from "../owner_flows" }
