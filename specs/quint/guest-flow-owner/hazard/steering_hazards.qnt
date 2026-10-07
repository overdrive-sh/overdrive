// -*- mode: Bluespec; -*-
// Hazard / teeth / environment-fault instances of module D (../steering.qnt). Each instance
// switches ONE design rule off (BUG_*) or enables an environment fault (ENV_*). Expected verdicts
// are in ../checks.toml. Generated from one constant table so every import names every constant.

// D8a-LOOKUP off: the revision-8 shape — steering is a firewall element separate from the
// listener; a failed element removal keeps the non-serving listener bound (the old D8a-PROBE)
module steer_bugElementSet {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 1,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = true, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// ... and a foreign nft flush deletes the elements and the marked-reset rule (the old D8a-FLUSH)
module steer_bugElementSetFlush {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 1,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = true, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// ... with the listener closed before its element is removed: the element steers to a wildcard service
module steer_bugElementCloseFirst {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = true, BUG_ELEMENT_CLOSE_FIRST = true, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-LOOKUP off: steering held in the flushable nft table — a foreign flush removes it
module steer_bugSteeringInNft {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = true,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-LOOKUP off: steering owned by the serve process (ADR-0159) — gone while serve is down; exposed only
// when the nft rules are also gone (foreign flush)
module steer_bugUnpinned {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = true, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-LOOKUP off: the program also assigns non-loopback ingress; with the divert flushed, remote
// traffic reaches an intake directly
module steer_bugNonloopback {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = true, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// K-L3 off: a steering entry outlives its listener's close
module steer_bugEntryOutlives {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = true,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-ROUTE off: the route step runs before the steering converge / verify; a crash between them
module steer_bugRouteFirst {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = true, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-ROUTE off: a boot converge / verify failure is ignored and the route is added anyway
module steer_bugNoVerify {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = true, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// U-6 off: the activation rollback closes only the failing port (two declared ports)
module steer_bugRollback {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1, 2), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 1, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = true,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// U-6 off: activation returns Ok without every wanted port steered
module steer_bugActOkPartial {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = true, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// U-3 off: teardown returns Ok with listeners still open; release follows
module steer_bugTeardownKeeps {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = true,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-HOLD / M-7 off: quiescence leaves intake listeners open
module steer_bugQuiesceKeeps {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = true, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-HOLD off: one latch — any recovery's restore reopens forwarding (the revision-7 rule)
module steer_bugAnyRestore {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = true, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-HOLD off: a recovery repairs without holding quiescence
module steer_bugRepairNoHold {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = true,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-FENCE off: boot refusal and every fence REMOVE the tagged route instead of fencing — prefix
// traffic then leaves by the default route
module steer_bugRemoveNotFence {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = true, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-FENCE off: repair_guest_prefix lifts after converge without verify + probe
module steer_bugNoFreshVerify {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = true, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// D8a-FENCE off: the fence is lifted by restore_forwarding (any reopening restore), not only by
// boot or repair_guest_prefix
module steer_bugRestoreLiftsFence {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = true,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Item 7 off (F-4b-1): no quiesce call verifies / fences before closing listeners
module steer_bugQuiesceNoFence {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = true, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Item 7 off: only the first quiesce call verifies; a joining call (another holder already holds)
// does not (root detach + foreign flush, one port, no crash)
module steer_bugJoinNoVerify {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = true, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Item 7 off (F-4c-1): a non-quiesce intake listener close (ListenState off, session loss,
// teardown, activation rollback, failed bring-up) does not verify first
module steer_bugCloseNoVerify {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = true,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// One owner off (round 4c's total converge_shared, F-4c-2/3/4): another recovery's repair
// converges the steering and lifts the route; its failure fences (root detach + foreign flush, one port, no crash)
module steer_bugOtherRepairsPrefix {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = true, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Serialized guest-prefix order off (F-4c-5, UP-9): a fence may fall between a lift's verify and
// its route write (root detach + foreign flush, one port, no crash)
module steer_bugUnserializedLift {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = true, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Trust boundary: root detaches the pinned link (plus crash at any step, foreign nft flush — the IpRules
// recovery overlaps —, one boot converge failure, one repair failure, attempt budget 2; one declared
// port; MAXGEN 1).
module steer_envDetach {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Trust boundary: root detaches the pinned link (unbounded) while every recovery can overlap (foreign
// flush → IpRules, listener-task exit → FlowListeners), one repair failure, attempt budget 2; one declared
// port; MAXGEN 1; no crash (crash, bind / steer / boot failures: steer_envDetach)
module steer_envDetachAll {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Trust boundary: root detaches the link between a close's passing verify and the close (one port,
// no crash, no flush, no listener exit: the guest-prefix recovery alone)
module steer_envCloseGap {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = true,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// A fence (route replace) fails once: a fence failure in a quiesce or a close does not stop it
// (root detach + foreign flush, one port, no crash)
module steer_envFenceFail {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 1,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Trust boundary: root removes / replaces / fences the tagged route, or sets `local` (plus crash, foreign
// flush, one boot converge failure; one declared port; no link detach)
module steer_envRootRoute {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = true, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Liveness of the trust-boundary exposure and of the guest-prefix recovery: root detach (unbounded),
// overlapping FlowListeners recoveries, crashes, one repair failure, attempt budget 2
module steer_detachLive {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// ... with no other recovery (no listener-task exit)
module steer_detachLiveSolo {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// ... without crash (serve stays up): instance for the all-recoveries liveness check (cost)
module steer_detachLiveNoCrash {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// UP-6 off: a re-attempt after a failed post-repair audit does not begin with the fence (no other
// recovery)
module steer_detachLiveSoloRedoNoFence {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = true, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// UP-7 off: a failed post-repair audit is not a failed attempt
module steer_detachLiveRedoNoCount {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 1, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = true).* from "../steering"
}

// The guest-prefix repair fails forever after a root detach: the recovery ends in fail-stop with the fence
module steer_repairForever {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = true, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// ... without crash: instance for the all-recoveries liveness check (cost)
module steer_repairForeverNoCrash {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = true, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// ... with D8a-HOLD off (any restore reopens): the guest-prefix recovery can be reset forever (no crash)
module steer_detachLiveAnyRestore {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = true, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// ... with no fairness on any recovery: nothing forces the end of the exposure
module steer_detachLiveNoFairness {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
  temporal DetachExposureEndsNoRecoveryFairness =
    and { weakFair(BootStep, bootVars), weakFair(Restart, bootVars) }
      .implies(always(exposed.implies(eventually(not(exposed)))))
}

// Liveness of the IpRules recovery while root detaches the link (foreign flush + root detach, no
// crash): the IpRules recovery's progress never depends on the guest-prefix recovery's (UP-11)
module steer_flushDetachLive {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = true,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Liveness after root removes or fences the tagged route at runtime (UP-5: the audit observes the
// route; the guest-prefix recovery repairs it)
module steer_rootRouteLive {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = true, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// UP-5 off: the audit does not observe the tagged route's form
module steer_rootRouteLiveAuditIgnores {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = true, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = true,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// Liveness after a foreign nft flush: leg-C interception returns (IpRules recovery)
module steer_flushLive {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}

// steer fails forever: a serving port stays refused (never a wildcard service)
module steer_steerForever {
  import steering(
    PORTS = Set(1), DECLARED = Set(1), MAXGEN = 1, ALLOW_CRASH = true, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "../steering"
}
