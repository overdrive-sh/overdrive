# V-EV-CTL findings: the control context (EVMOD-CTL, EVMOD-LIFE) on Linux 7.0

**Verdict: HOLDS for items (a) to (f).** No device operation, record layout, release rule or the module's lifetime was falsified. Eight contract points need pinning or amending (section 4), and one of them (4.3) changes a watch's abort action. Everything below was executed on metal and the output is pasted.

## 0. Substrate and scope

- Physical host: `uname -r` is `7.0.0-38-generic`, x86_64, AMD EPYC 8024P, `systemd-detect-virt` is `none`, `/dev/kvm` API 12. The driver fails closed on all of these.
- The module was loaded only inside Cloud Hypervisor guests. Guest kernel image is `7.0.0-29-generic` (sha `b51367c7dab2`). The CH fork binary is the one from the evmod spike (sha `eadddf1bbcdc`).
- After every job `lsmod | grep -c ovd_fwd` on the physical host was 0, and `modules added since start: []`.
- The module build is `ovd_fwd.ko`, sha256 `9c470fb863f74263`, identical in all runs. It was built with `W=1` against the `7.0.0-29-generic` tree.
- Each suite ran in its own fresh VM, with 4 vCPU and 4 to 8 GiB.
- `ovd_fwd` is the proposed EVMOD-CTL surface grafted onto the evmod forwarding core:
  - `/dev/ovd_fwd` is a misc device, mode 0600.
  - Each `open()` is a context.
  - Ids are u64 and per context. Records are 32-byte and preallocated.
  - `read` and `poll`.
  - There is no `module_exit`.
- Spike-only additions:
  - `/proc/ovd_fwd_audit` holds global counters used as evidence.
  - Datagram modes 1 and 2 return `EOPNOTSUPP`. They belong to V-EV-DGRAM.
  - The end classification is the minimum the control tests need. V-EV-END owns the full matrix.
- Not run: the module in the private QEMU host-role VM (vhost side). The host-role code is the same module, and only the vsock transport under it differs. The real vhost-vsock transport was exercised from the physical host as a plain peer (section 3.9).
- Harness incidents. All were harness bugs, none were module defects, and the evidence is kept:
  - `run-20261016T232226Z`: the guest `lo` was down, so every socket call failed.
  - `run-20261017T010222Z` and `run-20261017T011827Z`: the first watch run was too slow for its budget, and the first storm run blocked 20 min in a blocking `read` on the context fd (bug in the test's drain loop).

## 1. Cases: hypothesis, prediction, falsification, result

| Case | Hypothesis / prediction | Falsified if | Result |
|---|---|---|---|
| a1 | Releasing a context by `close()`, `kill -9` or `exit` with live pairs and watches, idle or stalled with 8 MiB queued, resets every TCP far end and ends every vsock far end as an abort. No record is queued. | A far end not reset, a record queued, an id still live. | HELD. 12 runs (3 hows, 2 holder-of-near-fd variants, 2 traffic states), 48 pairs and 16 watches each. |
| a2 | Release completes inside the exit path, before the parent's `waitpid` returns. | Far ends still open after `waitpid`. | HELD: reset count "at release return" equalled the final count in all 12 runs. |
| a3 | The context lives as long as any descriptor does. | Release on a partial close. | HELD for `dup` and `fork`. Without `O_CLOEXEC`, a descriptor inherited across `exec` keeps it alive. See 4.8. |
| a4 | A `SIGKILL` landing inside in-flight attach, abort and abort_all ioctls leaves no unreset pair and no leak. | A recorded attach with its far end not reset, a never-attached pair reset, a leaked object. | HELD. 120 trials, kill after 0 to 20 ms, 4 threads. |
| a5 | The same over the real virtio-vsock device, with real vhost-vsock host peers. | Guest far ends not reset, or a host peer left open. | HELD. |
| b1 | A close storm of 16,384 pairs yields exactly one record per id: none lost, none duplicated, none dropped by a bounded queue. | Missing, duplicate or foreign id. | HELD in 6 storm shapes (3.3). |
| b2 | Ids are strictly increasing and never reused across 10^6 attaches. | A duplicate or decreasing id. | HELD. |
| c | A watch's record is queued before the holder's `read` observes the end, under load and with 4,096 concurrent watches. | `ABORT(id)` still succeeds after the holder saw the end, or a record is missing. | HELD. |
| d | `delete_module` returns `EBUSY` with and without live pairs. | The module unloads. | HELD for flags 0, `O_NONBLOCK`, `O_TRUNC` and both. |
| e | Attach refusals carry distinct errnos. | An ambiguous or wrong errno. | HELD, plus gaps to pin (4.1, 4.2, 4.4). |
| f | A TLS ULP installed on a socket after attach never panics with the permanent module. | An oops. | HELD. |

## 2. Substrate and kernel citations (vendors/linux @ v7.0)

- **Permanence.**
  - `kernel/module/main.c:818-826`: `if (mod->init && !mod->exit) { forced = try_force_unload(flags); if (!forced) { ret = -EBUSY; ... } }`.
  - `try_force_unload` returns `flags & O_TRUNC` only under `CONFIG_MODULE_FORCE_UNLOAD` (`:715-727`).
  - The `7.0.0-29-generic` config has `# CONFIG_MODULE_FORCE_UNLOAD is not set` (`evidence/run-20261017T015322Z/guest-kernel-config-grep.txt`).
  - `kernel/module/procfs.c:33-36` prints `[permanent],` for exactly the same predicate `mod->init && !mod->exit`.
- **Release timing.**
  - `kernel/exit.c:971` `exit_files`, `:976` `exit_task_work`, `:988` `exit_notify`.
  - `fs/file_table.c:528-531` defers the final `fput` of a user task through `task_work_add`.
  - `__fput` calls `->release` at `:468-469`.
  - So release runs before `exit_notify`, and `waitpid` cannot return earlier.
- **Abort action.**
  - `net/ipv4/tcp.c:5087-5141` `tcp_abort`: `lock_sock`, `tcp_send_active_reset` if `tcp_need_reset`, `tcp_done_with_error`.
  - It returns `-ENOENT` if the socket is already `TCP_CLOSE` (`:5114-5119`).
- **ULP check.** `include/net/inet_connection_sock.h:347-350` `inet_csk_has_ulp`.
- **vsock end classification.**
  - `net/vmw_vsock/virtio_transport_common.c:1443-1464` SHUTDOWN and RST.
  - `:1244-1253` `do_close` sets `SOCK_DONE`, `peer_shutdown = SHUTDOWN_MASK`, and state `TCP_CLOSING`.
  - `:1661` the rx path holds `lock_sock`.
- **Why a reader cannot see an end before the callback finishes.**
  - `net/vmw_vsock/af_vsock.c:1280` `vsock_poll` takes `lock_sock` before returning. This is after the lock-free `EPOLLHUP` and `EPOLLRDHUP` computation at `:1250-1258`.
  - `recvmsg` takes `lock_sock` at `:2518`.
- **Why the holder's read may return `ENOTCONN`.**
  - `af_vsock.c:1220`: `vsock_shutdown` does `sock_reset_flag(sk, SOCK_DONE)`.
  - `:2522-2531`: with state not `ESTABLISHED`, `recvmsg` returns 0 only if `SOCK_DONE` is set, else `-ENOTCONN`.
- **Send side.** `af_vsock.c:2176-2178`: `-EPIPE` when `peer_shutdown & RCV_SHUTDOWN`.
- **kTLS.**
  - `net/tls/tls_main.c:1047-1083`: `tls_init` requires `ESTABLISHED` and does not look at `sk_user_data`.
  - `:767-768` saves `sk_write_space`.
  - `net/tls/tls_sw.c:2693-2694` saves `sk_data_ready`.
  - `:2600-2605` restores the saved `sk_data_ready` at close.
  - `tls_main.c:385-386` restores `sk_write_space` only if it still equals `tls_write_space`.

## 3. Results

### 3.1 (d) `delete_module`: 27/27

Run: `evidence/run-20261017T010222Z/suite-unload`.

```
GUEST_MODULES:
ovd_fwd 36864 0 [permanent], Live 0xffffffffc0435000 (OE)
CHECK PASS control: unknown module -> ENOENT :: ENOENT
CHECK PASS control: a module with module_exit (vsock_diag) unloads :: delete_module -> OK
CHECK PASS delete_module refused :: live pair + live watch, O_TRUNC|O_NONBLOCK (rmmod -f) -> EBUSY
CHECK PASS live pair still forwards byte-exact after the unload attempts :: 4096 bytes
DATA final /proc/modules: ovd_fwd 36864 1 [permanent], Live ...
```

| State when `delete_module` was called | flags 0 | `O_NONBLOCK` | `O_TRUNC` | `O_TRUNC\|O_NONBLOCK` |
|---|---|---|---|---|
| no context open | EBUSY | EBUSY | EBUSY | EBUSY |
| context open, no pairs | EBUSY | EBUSY | EBUSY | EBUSY |
| live pair | EBUSY | EBUSY | EBUSY | EBUSY |
| live pair and live watch | EBUSY | EBUSY | EBUSY | EBUSY |
| all contexts released | EBUSY | EBUSY | EBUSY | EBUSY |

`O_TRUNC` is refused only because the image has `CONFIG_MODULE_FORCE_UNLOAD` off. With it on, `rmmod -f` would run `try_stop_module` and free the module text under live callbacks. That case was not run.

### 3.2 (a) Release matrix: 68/68

Run: `suite-release`. Each run had 48 pairs and 16 watches. The vsock far ends were watched from a second context B.

- Every row had 48/48 far TCP ends open before the release.
- Every row had 48/48 reset **at release return**, with 0 ms settle.
- Every row had 64/64 far vsock ends classified `Aborted(PeerAbort)`.
- Every row had the released context queue no record.

Example row:

```
CHECK PASS far TCP ends all ECONNRESET :: release/kill-9/parent_holds_near=1/traffic=1: reset=48 (at release return 48) eof=0 open=0, last settled 0.0 ms ...
CHECK PASS release queued no record :: release/kill-9/...: rec_queued delta=64 (== ctxB watch records 64), rec_discard delta=64, won=64
DATA audit[end] ... pair_new=1536 pair_free=1536 rec_new=1536 rec_queued=768 rec_read=768 rec_discard=768 rel_won=768 live=0
```

- The 64 delta equals context B's own watch records exactly. The 64 records the released context would have queued were discarded (`rec_discard`).
- In `parent_holds_near=1` the parent keeps the near sockets open. The far-end reset therefore comes from the module, not from the process closing its sockets.
- Descriptor sharing:
  - A sole holder killed releases.
  - Closing one of two `dup`ed descriptors does not release.
  - Killing a forked inheritor does not release.
  - The last close releases.

### 3.3 (b) Close storm of 16,384 pairs: 37/37

Run: `evidence/run-20261017T014514Z/suite-storm`, 8 GiB. Build time was about 5 s and attach 3.5 to 6.4 µs per pair.

| Storm shape | Settled | Records | Notes |
|---|---|---|---|
| far TCP reset, with 16,384 vsock far watches in context B | 92 ms | 16,384 / 0 dup / 0 missing, all `Aborted(SocketError)` | B received 16,384 `Aborted(PeerAbort)` |
| far clean `SHUT_WR` both ways | 2,399 ms | 16,384 `Completed` | |
| owner `ABORT_ALL` | 182 ms | 16,384 `Detached` | returned `n=16384`; all far TCP ends reset |
| context release | 132 ms | 0 queued | 16,384 far TCP ends reset, `rec_discard=16384` |
| far reset with a concurrent reader thread | 105 ms | 16,384 total, 0 dup, 0 missing | reader consumed all during the storm |
| far reset racing `ABORT_ALL` | 85 ms | 16,384 total, 0 dup, 0 missing | `peer=14 sockerr=10050 detached=6320` |
| `kill -9` of the owner | `waitpid` returned 211 ms after the kill | 0 queued | 16,384 of 16,384 far TCP ends reset at `waitpid` return; `live=0` |

- Pre-read `queued_ends == 16384` in four shapes, so the unread queue is bounded by the attach count (one 48 B record each).
- The second `read` after the drain is `EAGAIN`, and the inventory is then zero.

### 3.4 (b) Ids: 13/13

Run: `evidence/run-20261017T011827Z/suite-ids`.

```
DATA ids: 20001 pair attaches + 980000 watches succeeded in 16.0 s; max id 1000001
CHECK PASS ids strictly increasing, never reused, within one context :: attaches+watches=1000001 dup=0 out-of-order=0 other-failures=0
CHECK PASS exactly one record per issued id across 10^6 ids :: records=1000001 issued=1000001 dup=0 unissued=0 missing=0
CHECK PASS second context starts at 1 :: id=1
CHECK PASS an id of another context is ENOENT :: abort of ctx1 id 1000002 from ctx2 -> ENOENT
```

- The count mixes real pair attaches and watch cycles on one re-watched vsock socket. Watch cycles are cheap, so a pure 10^6-pair run was not done.
- `ABORT(0)` and `ABORT(2^64-1)` are `ENOENT`.
- Refused attaches consume an id. After the errno suite a context had reached id 18 with few live pairs. Ids are never reused, but they are not consecutive.

### 3.5 (c) Watch record ordering: 10/10

Run: `evidence/run-20261017T014514Z/suite-watch`.

Method: the holder blocks in `read(vnear)` or `write` until it sees the end, then immediately does a non-blocking `read` of the context and calls `ABORT(id)`. The `ABORT` must return `ENOENT`.

```
peer SHUT_WR (0x2)   under load: 400/400 correct, order-violations=0 live-after-end=0 extra-records=0   (holder read() = 0 for 400)
peer SHUT_RDWR (0x3) under load: 400/400 ...                                                            (read()=0 for 390, ENOTCONN for 10)
peer close()         under load: 400/400 ...                                                            (read()=0 for 396, ENOTCONN for 4)
peer SHUT_RD         under load: 400/400 ...                                                            (holder learns by write -> EPIPE)
0x2 then 0x3         under load: 400/400, no second record
parallel: 4096 watches ended at once, 8 epoll holders: holders saw 4096/4096 ends; live-after-end = 0
          holder read() = 0 for 2180 watches, an error for 1916; records 4096 (half-close=2048, peer-abort=2048)
INFORMATIONAL poll-first window: 0 of 2048 holders that acted on EPOLLRDHUP/HUP alone found the watch still undecided
```

- Load: 100 pairs with 2 threads moving continuously (about 350 GB written by the end of the suite).
- The 400 trials per class is lower than the 2,500 of the aborted first run, which was cut by its time budget. That first run passed 2,500/2,500 in each of the first three classes.
- The poll-first window is closed because `vsock_poll` takes `lock_sock` before returning (`af_vsock.c:1280`).
- The `ENOTCONN` returns come from the module's own `SHUT_RDWR` of an aborted watch (see 4.3).

### 3.6 (e) Attach errnos: 42/42

Run: `suite-errno` (and `+strict`). "A" is the vsock half and "B" the TCP half.

| Case | errno | `bad_side` |
|---|---|---|
| fd -1 as A, 9999 as B, closed fd as B | EBADF | 0, 1, 1 |
| pipe as A, `/dev/null` as B | ENOTSOCK | 0, 1 |
| tcp+tcp, vsock+vsock, UDP+tcp (A=UDP), vsock+unix, vsock-seqpacket+tcp | ESOCKTNOSUPPORT | 1, 1, 0, 1, 0 |
| TCP with ULP only, with kTLS TX+RX, kTLS on the A side | EPROTONOSUPPORT | 1, 1, 0 |
| TCP never connected, TCP listener | ENOTCONN | 1, 1 |
| vsock never connected, vsock listener | ENOTCONN | 0, 0 |
| TCP after peer RST, TCP in FIN_WAIT2, vsock after peer `close()` (state CLOSING) | ENOTCONN | 1, 1, 0 |
| already attached (A), already attached (B), same pair twice, watch of attached, attach of watched | EBUSY | 0, 1, 0, -, 0 |
| watch twice | EBUSY | - |
| same fd twice, `dup` of the same socket (vsock and TCP) | EINVAL | 1 |
| mode 3, flag bit | EINVAL | 0, - |
| mode 1 (spike) | EOPNOTSUPP | 0 |
| IPv6 TCP + vsock | OK (accepted) | - |

- Refusal leaves nothing hooked: after a ULP refusal and after an EBUSY refusal the other half could still be watched.
- Precedence: `(ULP-bearing TCP, never-connected vsock)` gives EPROTONOSUPPORT, so ULP is checked before state. `(vsock, dup(vsock))` gives EINVAL, so same-socket is checked before kind.
- The seven classes EBADF, ENOTSOCK, ESOCKTNOSUPPORT, EPROTONOSUPPORT, ENOTCONN, EBUSY and EINVAL are pairwise distinct.
- TCP `CLOSE_WAIT` (the peer's FIN arrived before attach):

```
DATA close-wait-state tcpi_state=8 (1=ESTABLISHED 8=CLOSE_WAIT)
CHECK PASS tcp-CLOSE_WAIT(B) :: got OK                      (module accepts ESTABLISHED or CLOSE_WAIT)
DATA errno-case tcp-CLOSE_WAIT(B) STRICT module -> ENOTCONN (module with the evmod spike rule, ESTABLISHED only)
```

### 3.7 (f) Installing kTLS after attach: 18/18

Run: `suite-tls`. kTLS (TLS 1.2 AES-GCM-128) was installed on the attached TCP half and on its far end after the attach. Four endings were tried: clean completion, owner `ABORT`, context release, peer RST.

```
CHECK PASS tls-after: vsock->kTLS direction byte-exact :: case 'pair completes cleanly': 8192/8192
CHECK PASS tls-after: kTLS->vsock direction byte-exact :: case 'pair completes cleanly': 8192/8192
CHECK PASS tls-after: one end record :: case 'pair completes cleanly': kind=1 cause=0 moved_ab=8192 moved_ba=8192
CHECK PASS tls-after: one end record :: case 'owner ABORT': kind=2 cause=4 ...
CHECK PASS tls-after: one end record :: case 'peer RST': kind=2 cause=2 ...
CHECK PASS P-68 panic sequence: delete_module with kTLS-after stalled pair is refused :: EBUSY for rmmod and rmmod -f
CHECK PASS no kernel oops/warn after case :: case 0..3 and final
```

- The P-68 shape (kTLS after attach, stalled pair, `rmmod`) cannot start: `delete_module` is refused (`EBUSY`), and the stalled pair kept running.
- Attach also refuses a socket that already has a ULP (3.6).
- This is a single-kernel observation of "no panic", not a proof that every ULP-after-attach interleaving is safe. The residual (4.7) is the saved-callback clobber.

### 3.8 Misc control semantics: 41/41 (+36/36 strict)

Run: `suite-misc`.

- The device is a root-owned 0600 character device.
- The end record is 32 bytes with the pinned offsets (`kind` 8, `cause` 9, `side` 10, `err` 12, `moved_ab` 16, `moved_ba` 24).
- An unknown ioctl returns ENOTTY, and a bad pointer returns EFAULT.
- A `read` shorter than one record returns EINVAL. A non-blocking `read` with no record returns EAGAIN.
- A blocking `read` waited 301 ms for a record.
- `read(100)` returned 96 bytes (three whole records), then the rest, oldest first.
- `poll` is `EPOLLIN` while a record is queued and 0 once drained.
- `ABORT` of a live id returns 0, and the `Detached` record (kind 2, cause 4) is already queued when it returns.
- `ABORT` of an id whose record was read returns ENOENT. A context cannot abort another's id.
- `ABORT_ALL` with nothing live returns 0. With 5 live pairs it returned `n=5` with 5 records queued on return.
- `pair_state` of a live pair returns the state, and `ENOENT` for an ended id.
- Releasing one context leaves another's pair running.
- Early data (3,000 B plus FIN on the TCP half, 1,000 B plus `0x2` on the vsock half, both before attach) is forwarded byte-exact with both ends carried through.
  - The pair completes in its first runs with `moved=1000/3000`.
  - The same attach is refused by the strict module (`ENOTCONN`).
- A TCP reset gives `Aborted(SocketError, side B, ECONNRESET)`. A vsock `0x3` gives `Aborted(PeerAbort, side A)`.
- A watch on a vsock whose peer already half-closed queues the record at once.

### 3.9 (a) over the real virtio-vsock device: 14/14

Run: `suite-virtio` and `host-virtio.json`. A real host process held 32 vhost-vsock connections to the guest pairs.

```
close  : guest report RES reset=32 eof=0 open=0 ; host peers: eof 32, p50 9.5 ms (traffic 0), 25.2 ms (8 MiB queued)
kill-9 : reset=32 ; host peers eof 32, p50 8.5 ms (traffic 0), 20.6 ms (queued)
exit   : reset=32 ; host peers eof 32, p50 8.6 ms (traffic 0), 22.1 ms (queued)
```

- The guest-side TCP far ends were all reset.
- A plain host peer reads EOF, not a reset (`do_close` sets `SOCK_DONE`, `virtio_transport_common.c:1244-1253`). Only a module-classified peer can tell an abort from a clean end, which is the P-49 behaviour again.

### 3.10 Chaos: 7/7

Run: `suite-chaos`.

```
CHECK PASS every pair whose attach returned is reset by the owner's death :: 120 trials, kill after 0..20 ms; attached-and-recorded pairs 27740 (of which owner-aborted 9534); violations 0
CHECK PASS no never-attached pair was reset :: spurious=0
DATA pairs in flight (attach started, not recorded) at the kill: reset=11 eof=0 open=0
DATA audit[end] ... pair_new=27751 pair_free=27751 ... live=0
CHECK PASS every context object was freed (no leak) :: opened=121 released=121 freed=121
```

### 3.11 Owner discipline (`exec`): 5/5

Run: `evidence/run-20261017T015322Z/suite-exec`.

```
exec[O_CLOEXEC]:    600 ms after the owner was killed, 0 of 8 far TCP ends are still open, module live ids = 0
exec[no O_CLOEXEC]: 600 ms after the owner was killed, 8 of 8 far TCP ends are still open, module live ids = 8
CHECK PASS and the ghost pair still forwards bytes :: n=512
CHECK PASS killing the exec'd holder finally releases the context
```

## 4. Design implications for EVMOD-CTL, EVMOD-LIFE and EVMOD-PORT

**What holds, as pinned:**

- Release aborts every pair and watch before it returns and queues nothing.
- Ids are per context, never reused, and one record per id is never lost.
- Whole-record `read` and `poll` work as pinned.
- The module is permanent and `delete_module` is refused.
- Attach refusals are distinct.

**What must change or be pinned** (4.1 and 4.4 to 4.7 correct text the design relies on; 4.3 changes a pinned abort action):

1. **Attach state rule.**
   - ENOTCONN says "a TCP or vsock half not `ESTABLISHED`". This must read "TCP `ESTABLISHED` or `CLOSE_WAIT`; vsock `ESTABLISHED`".
   - With the strict rule, EVMOD-PAIR's early-FIN shape (client write and FIN before `Paired`) is refused. That would silently reintroduce parking.
2. **ENOTCONN is a flow abort for the owner, not an owner error.**
   - A TCP half after the peer's RST, and a vsock half after the peer's `0x3` with no data (state `TCP_CLOSING`), return ENOTCONN. This is the pinned "pending intake child that resets before `Paired`" case.
   - ADR-0199 should say so, and the owners should map it to a flow abort.
3. **Watch abort action (needs a user decision).**
   - The proposed table applies the abort action (`SHUT_RDWR` on the vsock half) to watches.
   - Measured: a watch aborted by `PeerAbort` makes the holder's `read` return ENOTCONN (10/400 for `0x3`, 4/400 for `close()`, 1,916/4,096 in the parallel run), not 0.
   - Recommended pin: a watch's abort never touches the holder's socket. The holder always takes the record after any end of `read` (EOF or error). Only a context release shuts watched sockets, as pinned.
   - The spike's explicit `ABORT` of a watch also does not touch the socket, and the contract is silent.
4. **A holder dropping a `GuestStream` must `ABORT` the watch.**
   - The module holds a file reference. A holder `close()` alone leaves the watch live and the vsock open.
   - Measured: `live_watches=1` after the close, and the far end saw nothing until the `ABORT`, when it saw EOF.
   - Reading the record is not enough on its own.
5. **Error precedence and `bad_side`.**
   - Pin the order EBADF/ENOTSOCK, then EINVAL (same socket), then ESOCKTNOSUPPORT, then EPROTONOSUPPORT, then ENOTCONN, then EBUSY.
   - Pin that `bad_side` is the first offending side, and the side that triggered a same-socket EINVAL is B.
   - Pin that refused attaches consume ids ("never reused, not consecutive").
6. **EBUSY wording.**
   - EBUSY is "`sk_user_data` already set". The final architecture sets it nowhere else (sockmap removed, BPF cgroup hooks do not). A TCP socket in a sockmap would also give EBUSY. That is reasoning, not a measurement.
   - IPv6 TCP is accepted (the hook is family-agnostic), though the evmod spike accepted only `AF_INET`. Pin the intended family set.
7. **kTLS.**
   - EVMOD-KTLS holds.
   - A leftover: kTLS installed after attach saves the module's callbacks, and the unhook then restores the originals over TLS's. No owner does this, and the module never unloads, so the saved pointers stay valid.
8. **Owner discipline: `O_CLOEXEC`.**
   - Without it, an exec'd child holds the context after the owner dies, and the pairs keep forwarding as ghosts (8/8, byte-exact).
   - `KernelFlowForwarder::open` must set `O_CLOEXEC`, and the probe could check `FD_CLOEXEC`.
9. **EVMOD-LIFE probe and image.**
   - The `PERMANENT` flag in `VERSION` is self-asserted. The adapter can independently verify `[permanent],` in `/proc/modules` (`procfs.c:33-36`), which uses the same predicate `delete_module` uses.
   - The image config must keep `CONFIG_MODULE_FORCE_UNLOAD` off. This should be added to ADR-0068/0198. Only the "off" case was measured.
10. **Inventory counters** are per context in the spike. The port's `ForwarderInventory` counters should be pinned as per context.

## 5. What was not covered, and caveats

- **Host-role instance:** the module in the QEMU vhost VM. Release over the real virtio device was measured with plain host peers only, so a host-side forwarder or watch classifying the chain was not exercised (V-EV-END's chain).
- **Datagram modes:** `EOPNOTSUPP` in the spike (V-EV-DGRAM).
- **Classification matrix:** only the minimum needed here. The full V-EV-END matrix is not covered.
- **Fault injection:** no memory-failure injection (`ENOMEM` on attach or open). The "preallocated record cannot fail" property holds by construction (the record is allocated before the hooks) and is not stress-tested.
- **Not an unbound proof for ULP interleavings:** only the four endings in 3.7.
- **Kernel versions:** 7.0.0-29 in the guest, 7.0.0-38 on the physical host. Both are 7.0 builds, neither is the pinned appliance image build.
- Latencies in the logs were measured under a 4-vCPU guest and are indicative only.

## 6. Gate recommendation

**PROMOTE EVMOD-CTL and EVMOD-LIFE, with the pins above.** Items (a) to (f) hold with no falsification and zero kernel faults in any suite. The contract-shaping surfaces (device operations, record layout, release rule, module lifetime) are confirmed. Pins 4.1 to 4.10 should be recorded in ADR-0199 and ADR-0198 before DISTILL. Pin 4.3 changes a pinned abort action and needs a user decision. The unrun host-role chain stays with V-EV-END, and datagram forwarding stays with V-EV-DGRAM.

## 7. Files

- `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-ev-ctl/`
- Module: `module/ovd_fwd.c`, `module/ovd_fwd.h`
- Test program: `test/ctltest.c`
- Driver: `driver/run.py`, `launch.py`, `runlease.sh`, `sync.sh`, `pull.sh`
- Progress: `out/REPORT-progress.md`
- Lease-run logs: `out/runlease-*.log`
- Evidence, one dir per job:
  - `evidence/run-20261016T232226Z` is the failed first attempt (`lo` down).
  - `evidence/run-20261017T005819Z` has errno, misc, unload and tls.
  - `evidence/run-20261017T010222Z` has errno, errno+strict, misc, misc+strict, unload, tls, release, virtio, chaos, and the aborted watch run.
  - `evidence/run-20261017T011827Z` has the ids run (13/13) and the hung storm run.
  - `evidence/run-20261017T014514Z` has watch (10/10) and storm (37/37).
  - `evidence/run-20261017T015322Z` has exec (5/5).
- Each run dir holds `serial.log`, `result.json`, `artifact-pins.json`, `substrate-probes.json`, `guest-kernel-config-grep.txt`, and the sources as executed.
