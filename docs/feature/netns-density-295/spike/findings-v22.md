# Spike findings — V-22: a dead VM's host-side vsock connections vs CID reuse (K-A2, K-B1)

Substrate: native x86_64 metal host (AMD EPYC 8024P), `systemd-detect-virt` = `none`, `/dev/kvm` present,
via `cargo xtask metal run` under the canonical lease. Host kernel and guest kernel: `uname -r` =
**7.0.0-29-generic** (guest `IDENT ... uname=7.0.0-29-generic`). Cloud Hypervisor: the fork,
`overdrive/vhost-kernel-vsock` @ **41a619d191f6472f79a3f22bcb9576c2a516906c** (v53.0 + 9b68dbb57 + 41a619d19),
`cloud-hypervisor v53.0.0`, sha256 `0e58b5ba…2f6d196`; launched with `--vsock cid=3220,backend=vhost-kernel`,
seccomp default. Probe/guest/initramfs/kernel pins: `increment-f/evidence/artifact-pins.json`.
Code and raw captures: `spike-scratch/netns-density-295-v22/` (verdict run `increment-f`, 4 rounds x 250
connections per port per set, ports 1234 / 1240 / 1241 / 1243).

Triple (as registered in feature-delta V-22 before the run): hypothesis = release of the vhost device resets
every host-side connection of that CID incl. accept-queue entries before the CID can be reused; prediction =
the accepted socket fails at its first read/write; a guest connect-then-close reads EOF; falsification = any
accepted connection from the dead VM that reads or writes successfully after the new VM holds the CID, or any
new-VM byte on it.

## Verdicts

| Sub-claim | Verdict |
|---|---|
| (1) Established connections (host-accepted) of the dead VM are reset before the CID is reused | **HOLDS** (4,000/4,000; 1,000 per port) |
| (2) Connections still in the host listener's accept queue are reset before reuse, and fail at their first read/write | **FALSIFIED** as written. The kernel does mark them reset before reuse, but a queued connection with unread bytes still delivers those bytes at the first read, after `accept()` returns it with the new VM's CID as peer: 4,000/4,000 (1,000 per port) returned the dead VM's bytes. Queued connections with no unread bytes do read as ended (4,000/4,000). |
| (3) A connection the guest closed before the host accepted it reads as ended | **HOLDS** (4,000/4,000 without data: EOF at first read; 4,000/4,000 with data: the data, then EOF) |

## What was measured

Per round: bystander VM (CID 3221), VM under test (3220) → host accepts N=250 connections per port (set E);
guest connects and closes before accept (set C3, with and without data); guest connects and the host does NOT
accept (set Q with a marker line `QO-i`, set QN with no bytes); `kill -9` of the VMM (reaped 19–37 ms later);
observation **before** reuse; a new VM launched with the same CID 3220 whose guest connects **from the same
guest local ports** (same 4-tuples) with different markers; observation **after** reuse; the host then
accepts the queues. On ports 1234/1240 the queue is accepted before the new VM exists ("early"), on
1241/1243 only after the new VM holds the CID ("late").

### (1) Established — HOLDS

```
E.setup          1234 {"live flags=IN+OUT": 1000}                                   (same for 1240/1241/1243)
E.before_reuse   1234 {"flags=IN+OUT+ERR+RDHUP recv=ECONNRESET send=ECONNRESET": 1000}   (CID not yet reused)
E.after_reuse_before_new_traffic 1234 {"flags=IN+OUT+RDHUP recv=EOF send=EPIPE": 1000}   (new VM holds CID 3220)
E.after_new_vm_traffic_same_tuple 1234 {"flags=IN+OUT+RDHUP recv=EOF send=EPIPE": 500}   (new VM dialled the same 4-tuple)
new_vm.connect.E_tuple_dead_socket_still_open 1234 {"HOLD ... ok=0 fail=125 ... Connection reset by peer": 4}
new_vm.connect.E_tuple_freed                  1234 {"HOLD port=1234 ok=125 fail=0": 4}
bystander (CID 3221, other VM)  1234 {"flags=IN+OUT recv=DATA:BYST send=OK": 12}
```
Same on all four ports. No byte of the new VM was ever read on a dead-VM socket. While the host still held
a reset socket on a 4-tuple, the new VM's connect from that tuple was refused (`ECONNRESET`, 500/500), not
cross-delivered; once the host closed it the same tuple connected normally (500/500). The bystander VM's
connections were untouched (48/48). The reset is visible as pending `POLLERR` + `ECONNRESET` immediately
after the VMM is reaped. `ss --vsock` still shows these sockets `ESTAB` (3,000 for peer 3220 before and after
the kill, `increment-f/evidence/ss-r0-*.txt`): the orphan reset sets `sk_err` but its `sk_state =
SS_UNCONNECTED` is numerically `TCP_ESTABLISHED` (upstream `vhost_vsock_reset_orphans`,
`netns-density-295-ch-vhost-vsock/linux-reference/drivers-vhost-vsock.c:721-751`; the source link is an
inference, the symptom is measured). Observe the reset through `recv`/`send`/poll, not `ss`.

### (2) Accept-queue entries — FALSIFIED as written

```
queue.early_before_reuse.Q_withdata  1234 {"flags=IN+OUT+ERR+RDHUP first_read=DATA:QO": 1000}   (accepted before the new VM exists)
queue.late_after_reuse.Q_withdata    1241 {"flags=IN+OUT+ERR+RDHUP first_read=DATA:QO": 1000}   (accepted with the new VM holding the CID)
queue.late_after_reuse.QN_nodata     1241 {"flags=IN+OUT+ERR+RDHUP first_read=ECONNRESET send=ECONNRESET": 1000}
queue.early_before_reuse.QN_nodata   1234 {"flags=IN+OUT+ERR+RDHUP first_read=ECONNRESET send=ECONNRESET": 1000}
```
A raw record from the late phase (peer CID is the **new** VM's, the bytes are the **dead** VM's):
```
{"claim":"queue","phase":"late_after_reuse","round":0,"port":1241,"set":"Q_withdata","peer":[3220,9100],"flags":"IN+OUT+ERR+RDHUP","first_read":"DATA:QO","data":"QO-0\n"}
```
Reading: the queued entries were reset before the CID was reused (`ERR` pending already when accepted
before reuse, same flags when accepted after). But a reset socket still returns its already-buffered bytes
before the error, and `accept()` hands it out. A guest that connects and writes its hello, then dies before
the host accepts, leaves a socket whose peer CID reads as the new VM's CID and whose first read is the dead
VM's hello. That meets the row's falsification clause ("any accepted connection from the dead VM that reads
... successfully after the new VM holds the CID"). Not reproduced: any byte of the new VM on a stale socket
(fresh connections were separate sockets, flags `IN+OUT`, 100% correct markers: `fresh_NQf` 4,000/4,000,
`fresh_NEf` 2,000/2,000, `fresh_NQc_same_tuple` 1,000/1,000 on the early ports). If the stale entry is still
queued, a new-VM connect from the same 4-tuple is refused with `ECONNRESET` (500/500 on ports 1241/1243;
the same tuples connect fine, 500/500, once the host closed the stale sockets on 1234/1240).

### (3) Guest closed before accept — HOLDS

```
C3.accepted_count 1234 {"accepted=500/500": 4}          (no entry silently dropped; same for all ports)
C3.nodata         1234 {"flags=IN+OUT+RDHUP first_read=EOF": 1000}
C3.data           1234 {"flags=IN+OUT+RDHUP first=DATA:CD second=EOF": 1000}
```
An orderly end: `RDHUP`, no `ERR`, no reset. The VM was alive, so this does not exercise a dead VM.

## Edge cases and caveats

- Tested path: CH opens `/dev/vhost-vsock` itself (`cid=…,backend=vhost-kernel`); the fork at this commit has
  no `fd=` handoff mode (that is V-25). Release happens at VMM exit (last close). If `serve` keeps a
  reference to the claimed device after the VMM dies, release is deferred; not tested here.
- The host reset marks do not distinguish "dead VM" from "guest aborted with RST": both appear as pending
  `ERR` with buffered data. Distinguishing signals measured at accept: stale `IN+OUT+ERR+RDHUP`; guest-closed
  `IN+OUT+RDHUP`; live new connection `IN+OUT`. Whether a poll-at-accept rule is sufficient (and race-free) was
  not validated here.
- CID reuse speed: 4.8 ms after reap in smoke increment-b/c (`increment-b/evidence/results.json`); in the
  verdict run the new VM launched ~4 s after reap because the early-port drain ran first.
- Forced same-tuple reuse is a worst case: real guests auto-bind random local ports, so a tuple collision is
  improbable but possible. Without a collision the stale entry is simply accepted next to the new ones.
- A host `close()` toward a peer that dies immediately appears to keep its 4-tuple busy for a while: the
  first full run (increment-d) failed in round 1 when it reused the previous round's local ports. Not
  isolated; the verdict run uses disjoint ports per round.
- Smoke/early increments a–e are kept as evidence of harness defects, not results (a: `/init` not
  executable; b: first observation; c: smoke after harness fix; d: cross-round tuple reuse; e: guest local
  port 4000 equals the agent control port).
- Modules `vhost_vsock`, `vsock` etc. were already loaded by earlier spikes on the shared host; none were
  loaded by this run, none unloaded (`results.json`: `modules_unloaded_by_me: []`).

## What the falsification changes (per the V-22 row)

Sub-claim (2) is falsified, so the row's stated consequence applies to the queued-connection case: the
control session and beacon cannot attribute a freshly accepted connection to the VM that holds the CID by peer
CID alone, because a queued connection from the previous holder of that CID can be accepted with its first
bytes intact. The row names the remedy: a per-launch attribution token at open (a wire-contract change for
DISTILL to pin in golden bytes, surfaced to the user first). The evidence also shows a host-side signal
(pending `POLLERR`/`ECONNRESET` on the accepted socket) that separates stale entries from live ones; whether
that is an acceptable alternative to the token is a design decision, not something this spike settles.
Sub-claims (1) and (3) hold and need no design change. R5-16 / R5-18 and the G-V3 / G-V6 rows should assert
the (2) behaviour as measured (bytes readable, `ERR` pending), not "fails at its first read".
