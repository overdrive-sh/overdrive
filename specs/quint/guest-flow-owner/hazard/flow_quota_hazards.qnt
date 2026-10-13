// -*- mode: Bluespec; -*-
// Teeth for the flow quota (../flow_quota.qnt): each instance is the design with ONE rule switched off
// (OFF). Expected verdicts are in ../checks.toml.

// R2-DRAIN off: the quota counts active flows only — a draining flow keeps its slot but not its count, so one
// withholding guest holds more than its own quota
module quota_bugCountActiveOnly { import flow_quota(OFF = Set("countActiveOnly")).* from "../flow_quota" }
// the quota is per allocation: with one quota for the node, a withholding guest exhausts the node
module quota_bugNodeWideQuota { import flow_quota(OFF = Set("nodeWideQuota")).* from "../flow_quota" }
// R2-DRAIN off: the flow's slot is released at the half-close while its drain cell is pending
module quota_bugFreeAtHalfClose { import flow_quota(OFF = Set("freeAtHalfClose")).* from "../flow_quota" }
