# Probe findings — shared-guest owner lifetime (V-4, V-5, V-10)

**Verdicts: V-4 HOLDS, V-5 HOLDS, V-10 HOLDS.**

Probe sources and raw evidence:
`spike-scratch/metal-test-placement-owner/increment-a/` (`evidence/`), commit
`cb15cde7d`. All runs were ordinary `cargo xtask metal run --` on host kernel
`7.0.0-38-generic`; leak checks after each run found nothing left
(`evidence/leaks-after-*.txt`). This record is condensed from the probe
report; the pasted per-item output is in the evidence files named below.

## Kernel basis (read before running; `vendors/linux` at `028ef9c96e96`, "Linux 7.0")

- `kernel/pid_namespace.c:175-182` — `copy_pid_ns` applies the new namespace
  to children only.
- `kernel/fork.c:2239, 2425-2428` — the first child becomes the namespace's
  init and is marked `SIGNAL_UNKILLABLE`.
- `kernel/exit.c:988, 743, 706, 597-624` — `exit_notify` →
  `forget_original_parent` → `find_child_reaper` → `zap_pid_ns_processes`,
  reached when the init's last thread exits.
- `kernel/pid_namespace.c:192-280` — `zap_pid_ns_processes` SIGKILLs every
  other task in the namespace (`:225-234`), reaps them, and waits until all are
  freed (`:269-274`): the init becomes a zombie only after its VMM is dead.
- `kernel/signal.c:1187-1191, 94-96` — a SIGKILL from an ancestor namespace is
  delivered through `SIGNAL_UNKILLABLE`.
- `kernel/pid.c:688-707, 367-370, 521-530` — `pidfd_open` resolves the pid in
  the caller's pid namespace.
- `fs/pidfs.c:278-299`, `kernel/signal.c:2151` — a pidfd becomes readable when
  its target exits.

## V-4 — the VMM dies with an owner that is init of its own PID namespace

Hypothesis: SIGKILL of an owner that is pid 1 of its own PID namespace removes
the VMM it launched, within a small bound, and the production launcher works
inside that namespace. Falsified if a VMM survives the owner, or the launcher
fails inside the namespace.

- SIGKILL of the owner during a 40 s guest exec left no VMM. Owner and VMM were
  gone in at most 38.6 ms over 13 cycles (6 sequential, 4 killed at once,
  3 single). A normal exit took about 24 ms (`evidence/v4-pidns.txt`,
  `v4-bound.txt`, `owner-logs-v4-pidns.txt`).
- The production launcher (provision, image, boot, guest exec) worked inside the
  namespace; election to first exec took 1.5-2.5 s, the same as without it.
- Control, no PID namespace, guest busy: the VMM was still alive 3 s after the
  owner died.
- An idle-guest control proves nothing: with the guest idle the VMM exits on its
  own even without the namespace (`evidence/v4-attempt1-idle-control.txt`).
- Anchor: a stand-in anchor fired the owner in 1 ms. SIGKILL of the real
  `cargo-nextest` made the owner log `ANCHOR FIRED` 3 ms later; both were gone
  by the first sample, 33 ms after the kill
  (`evidence/v4-real-anchor-sigkill-nextest.watch.txt`; its `+Nms` labels are
  nominal, read the timestamps). This run does not separate the owner's pidfd
  exit from run teardown; only the owner's own log line does.
- An owner started with an already-dead anchor refuses to start (`ESRCH`,
  exit 2).

## V-5 — rendezvous socket, namespace reach, election

Hypothesis: a pathname socket under `/run/ovd-gk/<run>/shared/` is reachable
from other per-test local-mode vsock network namespaces, is gone at run end,
and `flock` elects exactly one owner among simultaneous first arrivals.

- 8 simultaneous first arrivals with `flock`: exactly 1 owner, all 8 attached.
  Without the lock: 8 owners.
- Clients in 7 other per-test network namespaces connected to the socket.
- Socket path length 52-58 bytes (under the 108-byte `sun_path` limit).
- `/run/ovd-gk/<run>` and its `shared/` directory were gone after the run
  (`evidence/v5-rendezvous.txt`).

## V-10 — the wrapper's parent is nextest; pidfd is permitted

- The wrapper's parent is `cargo-nextest` directly (its parent is `sudo`, no
  intermediate `cargo`), stable across all 16 V-5 tests.
- The wrapper pid equals the test pid (the exec chain keeps it).
- As root, `pidfd_open` on the nextest pid succeeds, the pidfd is not readable
  while nextest lives, and `pidfd_send_signal(0)` succeeds
  (`evidence/v10-wrapper-parent.txt`).
- Observed through a probe-local shim that records its parent and then `exec`s
  the repo wrapper unchanged; a placement where the wrapper forks was not
  tested.

## Design implications

1. Open the anchor pidfd **before** `unshare(CLONE_NEWPID)`: from inside the new
   namespace `pidfd_open(<nextest host pid>)` returned `ESRCH` every time
   (`kernel/pid.c` resolves in the caller's namespace). DDD-8's "at start"
   means before the namespace exists.
2. The owner reads its host pid from `/proc/self`, never `/proc/<getpid()>`:
   inside the namespace `getpid()` is 1. A first attempt did this and sent
   `kill(1, SIGKILL)` to the host init; the kernel dropped it
   (`kernel/signal.c:91-92`).
3. The owner must not `waitpid(-1)`, which would steal the launcher's VMM wait
   status.
4. A client attached to a killed owner sees EOF (an empty reply), not an error;
   the client protocol maps EOF to the owner-loss failure.
5. Election and rendezvous need no change.

## Not tested

Pid reuse of the anchor; an owner with its own `/proc` mount; a kernel other
than 7.0.0-38.
