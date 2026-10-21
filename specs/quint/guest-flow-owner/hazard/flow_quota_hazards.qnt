// -*- mode: Bluespec; -*-
// Teeth for the flow quota (../flow_quota.qnt): each instance is the design with ONE rule switched off
// (OFF). Expected verdicts are in ../checks.toml.

// ADR-0147 off: the quota counts active flows only — a half-closed or ended flow keeps its place but not its count, so
// one withholding guest holds more than its own quota
module quota_bugCountActiveOnly { import flow_quota(OFF = Set("countActiveOnly"), REC = false).* from "../flow_quota" }
// the quota is per allocation: with one quota for the node, a withholding guest exhausts the node
module quota_bugNodeWideQuota { import flow_quota(OFF = Set("nodeWideQuota"), REC = false).* from "../flow_quota" }
