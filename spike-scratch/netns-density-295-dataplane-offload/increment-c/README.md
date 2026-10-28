# Control-plane netns runtime-closure proof

This bounded diagnostic reuses the existing GuestKit public fixture and runs
only
`adopt_on_restart::serve_restart_readopts_surviving_slot_and_gcs_orphan_netns`.
It reproduces the existing control-plane integration guest, stages only the
qualified `setsid` executable/ELF closure plus an empty ordinary
`/etc/resolv.conf` bind target, and retains the unchanged netns/PID/cgroup/
adoption/orphan-GC case result.

It adds no selectable group, CI lane, scenario, executor, product API, or
physical-host network effect.

## Result

Durable Metal `73e45a4bc5b719df49b62f80` exited zero. The exact unchanged case
reported one pass, zero failed/ignored, no runtime skip, and
`CONTROL_PLANE_NATIVE_BOUNDED_REPLAY_PASS`. Its original assertions prove the
survivor PID entered the provisioned namespace and cgroup, the slot was
adopted, a fresh allocation did not collide, and the orphan namespace was
garbage-collected.

Raw receipt: `evidence/metal-73e45a4bc5b719df49b62f80.log`, SHA-256
`202242ae3ddbb958cc9a612662adbe9b552c892c7850bea2df4c7ebd3a475d53`.
