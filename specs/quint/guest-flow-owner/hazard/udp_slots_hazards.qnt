// -*- mode: Bluespec; -*-
// Hazard / teeth instances of module B (../udp_slots.qnt). Expected verdicts are in ../checks.toml.

// slot claim is two steps (check, then take)
module slots_bugClaimSplit {
  import udp_slots(SOCKS = Set(1, 2), DESTS = Set(1, 2), SLOTS = Set(0, 1), MAXFID = 3,
    BUG_CLAIM_SPLIT = true, BUG_KEEP_FRAMES = false, BUG_LATE_PAIRED_BY_SLOT = false,
    BUG_IGNORE_LATE_PAIRED = false, BUG_RELEASE_ONE_SLOT = false, ENV_REPORT_LOSS = true, BUG_KEEP_FRAMES_ON_END = false).* from "../udp_slots"
}
// P-35: release returns the slot with frames still parked
module slots_bugKeepFrames {
  import udp_slots(SOCKS = Set(1, 2), DESTS = Set(1, 2), SLOTS = Set(0, 1), MAXFID = 3,
    BUG_CLAIM_SPLIT = false, BUG_KEEP_FRAMES = true, BUG_LATE_PAIRED_BY_SLOT = false,
    BUG_IGNORE_LATE_PAIRED = false, BUG_RELEASE_ONE_SLOT = false, ENV_REPORT_LOSS = true, BUG_KEEP_FRAMES_ON_END = false).* from "../udp_slots"
}
// a late Paired is matched by slot, not by flow id
module slots_bugLatePairedBySlot {
  import udp_slots(SOCKS = Set(1, 2), DESTS = Set(1, 2), SLOTS = Set(0, 1), MAXFID = 3,
    BUG_CLAIM_SPLIT = false, BUG_KEEP_FRAMES = false, BUG_LATE_PAIRED_BY_SLOT = true,
    BUG_IGNORE_LATE_PAIRED = false, BUG_RELEASE_ONE_SLOT = false, ENV_REPORT_LOSS = true, BUG_KEEP_FRAMES_ON_END = false).* from "../udp_slots"
}
// SLOT-ABORT off: a late Paired is dropped silently
module slots_bugIgnoreLatePaired {
  import udp_slots(SOCKS = Set(1, 2), DESTS = Set(1, 2), SLOTS = Set(0, 1), MAXFID = 3,
    BUG_CLAIM_SPLIT = false, BUG_KEEP_FRAMES = false, BUG_LATE_PAIRED_BY_SLOT = false,
    BUG_IGNORE_LATE_PAIRED = true, BUG_RELEASE_ONE_SLOT = false, ENV_REPORT_LOSS = true, BUG_KEEP_FRAMES_ON_END = false).* from "../udp_slots"
}
// U-2 off: Refused / Abort / the pairing deadline leaves the parked frames in the slot's cell
module slots_bugKeepFramesOnEnd {
  import udp_slots(SOCKS = Set(1, 2), DESTS = Set(1, 2), SLOTS = Set(0, 1), MAXFID = 3,
    BUG_CLAIM_SPLIT = false, BUG_KEEP_FRAMES = false, BUG_LATE_PAIRED_BY_SLOT = false,
    BUG_IGNORE_LATE_PAIRED = false, BUG_RELEASE_ONE_SLOT = false, ENV_REPORT_LOSS = true, BUG_KEEP_FRAMES_ON_END = true).* from "../udp_slots"
}
// liveness teeth: no fairness on the audit — a lost sock_release report strands the slot
module slots_liveTeeth {
  import udp_slots(SOCKS = Set(1, 2), DESTS = Set(1), SLOTS = Set(0, 1), MAXFID = 2,
    BUG_CLAIM_SPLIT = false, BUG_KEEP_FRAMES = false, BUG_LATE_PAIRED_BY_SLOT = false,
    BUG_IGNORE_LATE_PAIRED = false, BUG_RELEASE_ONE_SLOT = false, ENV_REPORT_LOSS = true, BUG_KEEP_FRAMES_ON_END = false).* from "../udp_slots"
  temporal NoAuditFairness =
    weakFair(releaseAny, slots).implies(always(anyOrphan.implies(eventually(not(anyOrphan)))))
}
