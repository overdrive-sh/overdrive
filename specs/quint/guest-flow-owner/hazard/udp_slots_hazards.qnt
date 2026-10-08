// -*- mode: Bluespec; -*-
// Hazard / teeth instances of module B (../udp_slots.qnt): the design with ONE rule switched off
// (OFF). Expected verdicts are in ../checks.toml.

// slot claim is two steps (check, then take)
module slots_bugClaimSplit { import udp_slots(OFF = Set("claimSplit"), CFG = "ok").* from "../udp_slots" }
// P-35: release returns the slot with frames still parked
module slots_bugKeepFrames { import udp_slots(OFF = Set("keepFrames"), CFG = "ok").* from "../udp_slots" }
// a late Paired is matched by slot, not by flow id
module slots_bugLatePairedBySlot { import udp_slots(OFF = Set("latePairedBySlot"), CFG = "ok").* from "../udp_slots" }
// SLOT-ABORT off: a late Paired is dropped silently
module slots_bugIgnoreLatePaired { import udp_slots(OFF = Set("ignoreLatePaired"), CFG = "ok").* from "../udp_slots" }
// U-2 off: Refused / Abort / the pairing deadline leaves the parked frames in the slot's cell
module slots_bugKeepFramesOnEnd { import udp_slots(OFF = Set("keepFramesOnEnd"), CFG = "ok").* from "../udp_slots" }
// liveness teeth: no fairness on the audit — a lost sock_release report strands the slot
module slots_liveTeeth {
  import udp_slots(OFF = Set(), CFG = "live").* from "../udp_slots"
  temporal NoAuditFairness =
    weakFair(releaseAny, slots).implies(always(anyOrphan.implies(eventually(not(anyOrphan)))))
}
