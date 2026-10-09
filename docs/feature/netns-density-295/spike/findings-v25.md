# V-25 — the kernel CID claim and its handoff to the fork (D16-CLAIM, K-C3, A-29)

**Verdict: HOLDS** on every sub-claim — but the handoff (c, e, f) holds only on a throwaway `fd=` prototype, because **the fork at `41a619d19` has no `fd=` mode**.

Probe and raw captures: `spike-scratch/netns-density-295-v25/` (`launch.py`, `common.py`, `fd-mode.patch`, `increment-a/run.py`, `increment-a/host_stage.py`, evidence in `increment-a/evidence/{host,vm}/` and `evidence/vm-attempt1-build-error/`).

## The fork has no `fd=` mode

`VhostKernelVsock::new` opens `/dev/vhost-vsock` itself, calls `VHOST_SET_OWNER`, then `SET_GUEST_CID`. `VsockConfig` has no `fd` field, and Landlock always grants the device path. ADR-0146 and ADR-0170 assume `fd=`. To test the handoff, `fd-mode.patch` (applied to a copy of the fork at run time) wraps the inherited `File` in its own `VhostKernBackend` + `VhostVsock` type — vhost 0.16's `Vsock` has no from-fd constructor — with a once-only guard so a reboot cannot reuse a closed descriptor number.

## Setup

- Native x86_64 metal (AMD EPYC 8024P) via `cargo xtask metal run` under the shared lease. Fail-closed probes passed: `systemd-detect-virt` = none, `/dev/kvm` API 12, `KVM_CREATE_VM` OK.
- `uname -r` = `7.0.0-29-generic` on host and guest (not the 6.18 pin).
- Fork `41a619d191f6472f79a3f22bcb9576c2a516906c` (v53.0 + 2).
- `/dev/vhost-vsock` is `crw-rw---- root kvm`. vhost modules loaded for the probe and unloaded after. CIDs 3250–3259, `v25-` prefix.

## Verdicts

| Sub-claim | Verdict |
|---|---|
| (a) `SET_GUEST_CID` on an unowned instance succeeds | HOLDS |
| (b) Exclusive while any fd, dup, inherited or SCM_RIGHTS reference is open, including after the handoff to CH | HOLDS |
| (c) CH started with `fd=` uses the claimed instance; the guest gets that CID | HOLDS (prototype) |
| (d) CID claimable again at the last close (poweroff, SIGKILL, parent close) | HOLDS, with a measurement caveat |
| (e) No Landlock rule for `/dev/vhost-vsock` needed in `fd=` mode | HOLDS in operation |
| (f) Guest-initiated reboot in `fd=` mode ends the VMM | HOLDS on the prototype, only because of the guard |

## Evidence (from `evidence/host/host-results.json`, `evidence/vm/results.json`)

**(a)**
```
a1: {"n": 1000, "outcomes": {"OK": 1000}, "set_owner_issued": false}
a2 invalid CIDs 0,1,2,U32_MAX,2^32: all EINVAL
a5: ten CIDs held on ten instances, each cross-claim EADDRINUSE: 10/10
```

**(b)** (1,000 iterations each, 500 for SCM_RIGHTS)
```
b1 parent alone         while held: EADDRINUSE x1000   after last close: OK x1000
b2 dup, close original  EADDRINUSE x1000               OK x1000
b3 fork, no exec        parent+child and child-alone: EADDRINUSE x1000 each; after child reaped: OK x1000
b4 exec-inherited       child /proc/fd: {"3":"/dev/vhost-vsock"}; EADDRINUSE x1000 each; OK x1000
b5 SCM_RIGHTS to peer   EADDRINUSE x500; OK x500
b6 O_CLOEXEC not passed to child: OK x200
b7 8 processes race for one CID, 200 rounds: exactly one winner 200/200
```

**(c)**
```
claim_before_launch OK; second claim while serve holds: EADDRINUSE
vhost_vsock_fds_in_vmm: ["5->/dev/vhost-vsock"]   (CH run with --landlock)
guest: LOCAL_CID=3251, AGENT_READY local_cid=Ok(3251)
second claim while the VMM alone holds it: EADDRINUSE
host-to-guest and guest-to-host: stream 4 MiB byte-exact, seqpacket boundaries_exact=1
guest-originated connections: server_saw_peer=vsock:3251:...
```

**(d)**
```
poweroff x20: 20/20 claimed on the first attempt; max 0.06 ms measured from reap
SIGKILL x20:  20/20 claimed; 3.9–7.9 ms measured from the SIGKILL send (first attempt always fails: process still dying)
              a reap-first claim succeeded on attempt 1 (0.034 ms); held host connection reset (ECONNRESET)
sleep-child test, 500 samples: 3.96–10.013 ms from SIGKILL send; max 0.028 ms after reap
serve keeps its copy: VMM SET_OWNER OK, serve's own SET_OWNER = EBUSY; CID stays EADDRINUSE after VMM exit until serve closes
```
Time measured from the kill signal is process teardown, not release delay; one of 500 samples was 10.013 ms from the kill. "Within 10 ms of the last close" holds by about two orders of magnitude.

**(e)** CH ran the whole scenario with `--landlock` (`landlock_enable: true`) and `fd=5`. `strace -f` showed zero `openat` of `/dev/vhost-vsock` by the VMM; its ioctls ran on inherited descriptor 5. The live ruleset was not inspected.

**(f)**
```
ident_before OK; after 'reboot': VMM exit_code 1 within 0.1 s
Error rebooting VM > Cannot create vhost-kernel virtio-vsock device > The claimed vhost-vsock descriptor was already consumed
reclaim after exit: attempt 1, 0.031 ms
```
The unguarded variant was not run. The post-reboot serial log is empty (CH truncates it on VM re-creation).

**Bad descriptors:** `fd=` pointing at `/dev/null` (ENOTTY) or a closed number (EBADF) makes CH exit 1 before READY; the CID stays claimable.

## Design implications

Nothing falsified — the claim port, the move-only handle and `Vmm::create`'s argument keep their A-29 shape. Points to pin:

1. Reserved CIDs (0, 1, 2, `U32_MAX`, > 32 bits) return `EINVAL`, not `EADDRINUSE` — a typed non-`InUse` error.
2. A second `SET_GUEST_CID` on the same instance silently moves the claim and frees the old CID (`a4`) — the handle must not offer re-claim.
3. The launcher clears `CLOEXEC` and passes the same descriptor number as `fd=N`; reopening via `/proc/self/fd/N` creates a new instance (`a7`).
4. `serve` closes its copy after the spawn, or the CID outlives the VMM.
5. A-29's 10 ms release bound is measured from the reaped exit of the VMM, not from the signal.
6. DELIVER work on the fork: implement `fd=` with its own vhost wrapper over the inherited `File` and a once-only consumption guard for the reboot path.
7. Not covered: the production launch identity (uid drop, ADR-0129 hook, ADR-0143 filter) with `fd=`, and an unprivileged opener of `/dev/vhost-vsock` (0660 root:kvm). The spike ran as root.

**Gate recommendation:** PROMOTE — V-25 discharged on the prototype; `fd=` in the fork is DELIVER work.
