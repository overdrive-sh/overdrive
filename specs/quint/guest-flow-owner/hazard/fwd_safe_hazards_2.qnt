// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_safe (../fwd_safe.qnt), continued: rule 5 (MODULE-REFCOUNT) and its write-space closure
// (round 16). A separate file because Apalache's parse of one file with every instance exhausts a check's 2 GiB heap.
// Expected verdicts are in ../checks.toml.

// MODULE-REFCOUNT, rule 5 off, one clause each (every one violates NoTextRunAfterUnload):
// the half's module_put at its removal, before the grace period
module safe_bugRefDroppedAtUnhook { import fwd_safe(OFF = Set("refDroppedAtUnhook"), CFG = "pair").* from "../fwd_safe" }
// the removal does not take the half's socket lock
module safe_bugRemoveWithoutSockLock { import fwd_safe(OFF = Set("removeWithoutSockLock"), CFG = "pair").* from "../fwd_safe" }
// the reference of a half with a ULP at its removal is dropped anyway
module safe_bugUlpRefReleased { import fwd_safe(OFF = Set("ulpRefReleased"), CFG = "pair").* from "../fwd_safe" }
// the free work on a system workqueue: module_exit's drain does not wait for its return
module safe_bugPutOnSystemQueue { import fwd_safe(OFF = Set("putOnSystemQueue"), CFG = "pair").* from "../fwd_safe" }
// rule 5's write-space closure off (round 16): the write-space trigger an sk_write_space override again, entered by a
// "wfree" waker outside wqlk(t) (violates NoTextRunAfterUnload)
module safe_bugWriteSpaceOverride { import fwd_safe(OFF = Set("writeSpaceOverride"), CFG = "pair").* from "../fwd_safe" }
// rule 5's write-space closure order off: the entry removed by the free work after its cancel_work_sync, i.e. after the
// free work was queued (violates NoTouchAfterFree)
module safe_bugRemoveWaitAfterFreeQueued { import fwd_safe(OFF = Set("removeWaitAfterFreeQueued"), CFG = "pair").* from "../fwd_safe" }
