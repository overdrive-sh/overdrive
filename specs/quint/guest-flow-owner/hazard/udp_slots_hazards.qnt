// -*- mode: Bluespec; -*-
// Hazard / teeth instances of udp_slots (../udp_slots.qnt): the design with ONE rule switched off
// (OFF). Expected verdicts are in ../checks.toml.

// slot claim is two steps (check, then take)
module slots_bugClaimSplit { import udp_slots(OFF = Set("claimSplit"), CFG = "ok", REC = false).* from "../udp_slots" }
// P-35: release returns the slot with datagrams still in its socket's queue
module slots_bugKeepFrames { import udp_slots(OFF = Set("keepFrames"), CFG = "ok", REC = false).* from "../udp_slots" }
// a late Paired is matched by slot, not by flow id
module slots_bugLatePairedBySlot { import udp_slots(OFF = Set("latePairedBySlot"), CFG = "ok", REC = false).* from "../udp_slots" }
// SLOT-ABORT off: a late Paired is dropped silently
module slots_bugIgnoreLatePaired { import udp_slots(OFF = Set("ignoreLatePaired"), CFG = "ok", REC = false).* from "../udp_slots" }
// U-2 / EVMOD-DGRAM off: Refused / Abort / the pairing deadline leaves the datagrams in the slot socket's queue (it is
// not discarded at an end of association)
module slots_bugKeepFramesOnEnd { import udp_slots(OFF = Set("keepFramesOnEnd"), CFG = "ok", REC = false).* from "../udp_slots" }
// liveness teeth: no fairness on the audit — a lost sock_release report strands the slot
module slots_liveTeeth {
  import udp_slots(OFF = Set(), CFG = "live", REC = false).* from "../udp_slots"
  temporal NoAuditFairness =
    weakFair(releaseAny, slots).implies(always(anyOrphan.implies(eventually(not(anyOrphan)))))
}
