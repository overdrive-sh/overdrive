// One weakened rule for each protocol decision: D-OWNER, D-SERIAL, D-TAINT, D-LIFE, D-DETACH.
module guest_bugNoLock { import shared_guest(OFF = Set("noLock"), REC = false).* from "../shared_guest" }
module guest_bugNoSerial { import shared_guest(OFF = Set("noSerial"), REC = false).* from "../shared_guest" }
module guest_bugNoTaint { import shared_guest(OFF = Set("noTaint"), REC = false).* from "../shared_guest" }
module guest_bugOrphan { import shared_guest(OFF = Set("orphan"), REC = false).* from "../shared_guest" }
module guest_bugCancelOnDisconnect { import shared_guest(OFF = Set("cancelOnDisconnect"), REC = false).* from "../shared_guest" }
