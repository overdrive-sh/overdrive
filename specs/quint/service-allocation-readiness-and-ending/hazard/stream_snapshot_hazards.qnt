// -*- mode: Bluespec; -*-
// Teeth for ../stream_snapshot.qnt. Expected verdicts are in ../checks.toml.

// D11: the stream reads the recorded verdict and then subscribes; a terminal committed and published between the two is lost
module ss_bugReadBeforeSubscribe { import stream_snapshot(OFF = Set("readBeforeSubscribe"), REC = false).* from "../stream_snapshot" }
