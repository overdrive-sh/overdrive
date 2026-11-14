// One weakened rule for each protocol decision: D-OWNER, D-SERIAL, D-DETACH, D-TAINT, D-LIFE, D-BOUND, D-IDLE.
module guest_bugNoLock { import shared_guest(OFF = Set("noLock"), REC = false, ENV = true).* from "../shared_guest" }
module guest_bugNoSerial { import shared_guest(OFF = Set("noSerial"), REC = false, ENV = true).* from "../shared_guest" }
module guest_bugNoTaint { import shared_guest(OFF = Set("noTaint"), REC = false, ENV = true).* from "../shared_guest" }
module guest_bugOrphan { import shared_guest(OFF = Set("orphan"), REC = false, ENV = true).* from "../shared_guest" }
module guest_bugCancelOnDisconnect { import shared_guest(OFF = Set("cancelOnDisconnect"), REC = false, ENV = true).* from "../shared_guest" }
module guest_bugNoBound { import shared_guest(OFF = Set("noBound"), REC = false, ENV = true).* from "../shared_guest" }
module guest_bugIdleAnyTime { import shared_guest(OFF = Set("idleAnyTime"), REC = false, ENV = true).* from "../shared_guest" }
