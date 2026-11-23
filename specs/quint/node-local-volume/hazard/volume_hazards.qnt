// One weakened rule per protocol decision of the volume spec: VD-8 (noWait: the launch treats the VMM process exit as the
// claim free), VD-9 (recreate: ensure creates unconditionally instead of create-if-absent), VD-2 (stopDeletes: stop deletes
// the volume), SD-1 to SD-5 (noInterrupt: a signalled launch is not abandoned: raising the signal only records it
// and the signal-guards are gone). SD-6 belongs to specs/quint/restart-generation/.
// VD-3 (the backing choice) is not a protocol rule and has no hazard.
module vol_bugNoWait {
  import node_volume(OFF = Set("noWait"), REC = false, OPS = Set(), NA = 3, FB = 1, LIMIT = 3, DVMAX = 1).*
    from "../node_volume"
}
module vol_bugRecreate {
  import node_volume(OFF = Set("recreate"), REC = false, OPS = Set(), NA = 3, FB = 1, LIMIT = 3, DVMAX = 1).*
    from "../node_volume"
}
module vol_bugStopDeletes {
  import node_volume(OFF = Set("stopDeletes"), REC = false, OPS = Set("stop"), NA = 2, FB = 0, LIMIT = 3, DVMAX = 1).*
    from "../node_volume"
}
module vol_bugNoInterrupt {
  import node_volume(OFF = Set("noInterrupt"), REC = false, OPS = Set("stop"), NA = 2, FB = 0, LIMIT = 3,
    DVMAX = 1).* from "../node_volume"
}
