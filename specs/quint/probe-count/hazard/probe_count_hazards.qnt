// -*- mode: Bluespec; -*-
// Teeth for probe-run counting (../probe_count.qnt): each instance is the approved rule with ONE rule switched off (OFF).
// Expected verdicts are in ../checks.toml.

// the previous rule: the consumer reconstructs a streak from N - M
module streak_bugReconstruct { import probe_count(OFF = Set("reconstruct")).* from "../probe_count" }
// terminate whenever crossed and Running, no act-once A check
module streak_bugNoActOnce { import probe_count(OFF = Set("noActOnce")).* from "../probe_count" }
// a NoSample run written as a Fail row
module streak_bugNoSampleFail { import probe_count(OFF = Set("noSampleFail")).* from "../probe_count" }
// the store merges by observed-at instead of run_seq, with a backward clock step
module streak_bugLwwObservedAt { import probe_count(OFF = Set("lwwObservedAt")).* from "../probe_count" }
// the previous act condition: once per Fail streak (N - s >= A); never retries a failed stop on a later row
module streak_bugOncePerStreak { import probe_count(OFF = Set("oncePerStreak")).* from "../probe_count" }
// act at most once per key ever (emit only if A == 0)
module streak_bugOnceEver { import probe_count(OFF = Set("onceEver")).* from "../probe_count" }
