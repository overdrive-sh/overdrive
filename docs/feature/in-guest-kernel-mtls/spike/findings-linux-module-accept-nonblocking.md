# Spike E — the inbound `accept()` server path + non-blocking / epoll readiness for the Linux kernel-module mTLS mechanism (Firecracker)

Overdrive GH #303 (`in-guest-kernel-mtls`). PROBE phase, throwaway. Probe sources,
scripts and raw run logs: `spike-scratch/in-guest-kernel-mtls/increment-j/`
(`runs/NNNN.{meta,stdout,stderr}` + `tap.pcap` for boots 0020/0021/0030/0035/0040,
append-only, metal target/host/login-home redacted). Host kernel never touched; the
`.ko` `insmod`'d only inside the disposable Firecracker guest. Built as the login
user; boots as root.

The probe agent could not write under `docs/`, so the orchestrator wrote this file
from its report and re-checked the headline lines (accept server-identity,
`EPOLLOUT` only after kTLS install, pre-ready `write()`=EAGAIN, `SO_ERROR=13` deny,
40000/40000 bytes `match=1`, custody 0, host-clean, and the run-0021 panic +
reverified fix) against the committed `runs/` captures.

## Verdict: WORKS. The Linux kernel module is now proven for CLIENT **and** SERVER, BLOCKING **and** NON-BLOCKING.

Spike C proved the outbound/client/blocking slice; Spike D added resolve + LB. Spike
E closes the two items Spike C deferred — the inbound `accept()` server path and
non-blocking/epoll readiness — on the same stock kernel, and re-confirms the Spike
C/D blocking outbound path is unchanged. One kernel bug (a stack-overflow from a
stale `struct sock` pointer reuse) was found by a real boot, diagnosed from its
backtrace, fixed, and reverified.

| # | Checklist item | Verdict | Run |
|---|---|---|---|
| 1 | accept() inbound: guest server gets server-side kTLS; client SVID verified; byte-exact; TLS-only on wire; SVID key 0× on vsock | **WORKS** | 0020 |
| 2 | accept() deny: denied inbound caller rejected; guest `accept()`'d socket errors; no app bytes | **WORKS** | 0020 |
| 3 | non-blocking connect: EINPROGRESS → no EPOLLOUT / `write()`=EAGAIN until kTLS → then writable byte-exact; deny → `SO_ERROR` | **WORKS** | 0030 |
| 4 | non-blocking accept + edge-triggered epoll small reads: every byte of a multi-record stream; readiness not reported before handshake, not lost while buffered | **WORKS** | 0035 |
| 5 | the background-handshake-drive mechanism named exactly, proven not to block the hooked syscall / deadlock under `lock_sock` | **WORKS** (per-socket kthread) | 0030, 0035 |
| 6 | regression: Spike C/D blocking outbound (connect, resolve+LB, re-resolve, deny) still passes | **WORKS** | 0040 |
| — | host clean after the run (host kernel untouched, no leaked objects) | **host clean = YES** | 0045 |

## Substrate (runs 0002 / 0045)

- Guest `uname -r` = **`7.0.0-29-generic`** (Ubuntu 7.0.0-29.29); host `uname -r` identical — Firecracker boots the host's own `/boot/vmlinuz-7.0.0-29-generic` bzImage directly. No patch/rebuild.
- Firecracker **v1.17.0**, sha256 `99ad0f5cd0514a88aad0e9ae8cfdb3cc3b4ab9d190e1194602406c786b5de7a5`.
- gcc `(Ubuntu 15.2.0-16ubuntu1) 15.2.0`; rustc/cargo `1.95.0`.
- `.ko` vermagic `7.0.0-29-generic SMP preempt mod_unload modversions`; module source `module/igkmd_mtls.c` = 989 lines.
- Metal host: AMD EPYC 8024P, `systemd-detect-virt=none`, `/dev/kvm` usable. The box is shared; several runs recorded lease-contention timeouts (0001, 0003–0005, 0011) — infra, not probe results.

## Kernel-header ground truth (run 0002) — the module was written against these, not guessed

```
struct proto .accept (include/net/sock.h:1297):  struct sock *(*accept)(struct sock *sk, struct proto_accept_arg *arg);
struct proto_ops .poll (include/linux/net.h:178): __poll_t (*poll)(struct file*, struct socket*, struct poll_table_struct*);
kallsyms: tcp_poll T · inet_stream_ops D · inet_csk_accept T · tcp_prot T · tcp_set_ulp T · kallsyms_lookup_name T
```

## The accept()-hook mechanism (Part 1)

The module overwrites `tcp_prot.accept` (`module/igkmd_mtls.c:964`, restored at
`:975`) with `hooked_accept` (`:905`), prototype matching the 7.0 signature
confirmed in run 0002. `hooked_accept` calls the saved `inet_csk_accept`; when the
listener's local port (`inet_sk(listener)->inet_num`) is in the `mesh_listen=`
param, it tags the accepted CHILD `ROLE_SERVER`, records the ACCEPT-frame addrs from
`inet_sk(child)`, swaps the LISTENER's `struct socket ->ops` so future children are
born with the readiness-masking `->poll`, and spawns the per-socket driver.

`do_handshake` (`:527`) gained a role: `ROLE_SERVER` sends the **13-byte ACCEPT
frame (0x08)** (`:554`) the host relay already understood (defined in Spike B/C,
never exercised by the Linux module) — selecting the rustls SERVER role holding the
`guest-server` SVID, client-cert-required, zero tickets. The
`TO_PEER / NEED_PEER / SECRETS / DENY` loop and the kTLS install are identical to the
client role.

### Part 1 predicted vs actual (run 0020)

```
[0.765770] IGKME: [accept] inbound on mesh listen :8443 -> child local 192.168.204.2:8443 remote 192.168.204.1:46660; server-side mTLS, handshake driven by kthread
[0.822318] IGKME: [c1] HANDSHAKE DONE role=server peer=.../peer-client; kTLS installed on the workload socket (TO_PEER=1 NEED_PEER=3)
[0.824520] IGKME: [c1] SERVER-IDENTITY verified-client-peer=spiffe://overdrive.test/ns/default/sa/peer-client
IGKME-APP: server conn#1 read(32) PLAINTEXT-IN-GUEST: IGKM-E-REQI inbound peer->guest
INCLIENT[peer-client] client-side handshake done ... server SPIFFE ID=.../guest-server
RELAY[c1] handshake complete: policy ALLOW local .../guest-server <-> peer .../peer-client ... HKDF(secret)==rustls key/iv tx=true rx=true
RELAY[c1] custody: ... client SVID key PKCS#8 DER=0 PEM=0 private scalar=0; guest-server SVID key DER=0 PEM=0 scalar=0
# deny:
[0.841992] IGKME: [c2] DENY from relay: policy deny ... peer .../peer-client-denied -> first I/O returns -EACCES, no app bytes on wire
IGKME-APP: server conn#2 read FAILED errno=13 (Permission denied) -- DENY expected: no application bytes crossed
# wire (:8443): records only — SCAN peer->guest records=4 (0x17 count=2) unframed_bytes=0 ; 'IGKM-E-REQI':0 'PLAINTEXT-IN-GUEST':0
# /proc/net/tls_stat: TlsTxSw 1  TlsRxSw 1
```

Deny note: in TLS 1.3 client-auth the inbound caller's own handshake completes (it
gets the guest-server cert + Finished) and writes an app record, but the relay (the
guest's server endpoint) rejects the `peer-client-denied` client cert and never
emits SECRETS, so the module shuts the accepted socket before kTLS — the guest's
`read()` returns `EACCES` with 0 plaintext delivered, and the caller's ciphertext is
never decrypted (wire markers 0×). Server-side analogue of the client-side deny.

## The non-blocking background-handshake-drive mechanism (Part 2, item 5) — named exactly

The question Spike C left open: *on the kTLS/module path, who drives the handshake
while the workload is in `epoll_wait` and never calls into the socket?* Answer: **a
per-socket kernel thread + per-socket `proto_ops` poll masking** — the Linux-kTLS
analogue of B2's Unikraft guard-thread + `lib-lwip` readiness hook (there a userspace
record layer + lwIP patch; here the kernel owns the record layer via kTLS and the
readiness surface is `tcp_poll`, which can't be patched, so it's interposed
per-socket).

- **Background drive = `igkm_drive()` (`:654`), launched by `spawn_driver()` (`:730`, `kthread_run(igkm_drive, sk, "igkme-hs")`).** Kicked at `connect()` for a non-blocking client and at `accept()` for every server child. Client: waits for `TCP_ESTABLISHED` (bounded `msleep`); server: child already established. Then runs the vsock relay (`do_handshake`) and installs kTLS. Runs in its own task context, so it never blocks the hooked syscall and never holds `lock_sock` across the vsock round trips (`install_ktls` takes the sock lock only briefly around `tcp_set_ulp`). Proof (runs 0030/0035): the hooked `connect()` returned `EINPROGRESS` immediately while the handshake finished later in the kthread; `accept4()` returned the child fd immediately with the handshake completing later in `igkme-hs`.
- **Readiness suppression = `igkm_poll()` (`:633`) on a per-socket `proto_ops` copy (`igkm_stream_ops`), installed by `igkm_swap_ops()` (`:620`).** Calls the real `tcp_poll` to register the poll_wait, then MASKS `EPOLLIN|EPOLLOUT|…` while `PENDING`/`HANDSHAKING`, ADDS `EPOLLERR|EPOLLHUP` on `FAILED`, passes through once terminal-success. The `struct socket ->ops` pointer is writable (only the pointee is `const`) — no rodata patching.
- **On completion the kthread wakes poll** (`sk_state_change`/`sk_write_space`/`sk_data_ready`, `:717`); **on DENY it sets `sk->sk_err` + `sk->sk_error_report` (`:723-725`)**, surfaced via `SO_ERROR` (and `EPOLLERR`).
- **Non-blocking I/O before kTLS returns `EAGAIN`; a blocking caller on a kthread-driven socket waits on the entry waitqueue** (`io_gate()` `:789` / `wait_for_driver()` `:761`).

### Part 2 item 3 predicted vs actual (run 0030) — the EPOLLOUT-after-install timing

Serial order (kernel `pr_info` and app stdout both on ttyS0 unbuffered, so order is the proof):

```
IGKME-APP: nbclient connect() rc=-1 errno=115 (Operation now in progress)
IGKME-APP: nbclient pre-ready write() rc=-1 errno=11 (EAGAIN expected: not writable before kTLS)
[0.813970] IGKME: [c1] HANDSHAKE DONE role=client peer=.../peer-allowed; kTLS installed on the workload socket
[0.818166] IGKME: [kthread] role=client background handshake DONE; kTLS installed, waking poll (readiness now reported)
IGKME-APP: nbclient EPOLLOUT (writable) FIRST seen at wake#1 -- this is AFTER the module's kTLS install line above
IGKME-APP: nbclient post-ready write() rc=38 ... read(38) PLAINTEXT: IGKM-E-REQN nbclient->peer first line
# handshake-deny:
[0.853482] IGKME: [kthread] role=client background handshake FAILED err=-13 -> sk_err=13 (app sees EPOLLERR / SO_ERROR), no app bytes
IGKME-APP: nbclient EPOLLERR -> SO_ERROR=13 (Permission denied); app bytes written=0
# resolve-deny: nbclient connect() rc=-1 errno=13 -> no TCP, no app bytes
# wire: tls markers 0, unframed_bytes=0; custody SVID key 0× (ALLOW secrets [1,1], DENY [])
```

`EPOLLOUT` is reported only after the kTLS-install line — the mask held. Deny
surfaced via `SO_ERROR=13` with 0 bytes written; the hsdeny stream carried only
handshake records (`0x17 count=0`).

### Part 2 item 4 predicted vs actual (run 0035) — edge-triggered epoll, every byte

```
IGKME-APP: nbserver accept4(NONBLOCK) child fd=5 from 192.168.204.1:40890; adding to epoll EPOLLIN|EPOLLET
[0.882868] IGKME: [kthread] role=server background handshake DONE; kTLS installed, waking poll
IGKME-APP: nbserver ET edge#1 drained 40000 bytes in 625 small reads
IGKME-APP: nbserver RECEIVED total=40000 bytes over 1 ET edges, rolling-hash=0x5ffb9001e638b5f6 (expected 40000) match=1
# wire (:8444): peer->guest 40874 bytes records=6 (0x17 count=4) unframed_bytes=0 ; 'IGKM-E-FLOOD':0 cleartext ; custody SVID key 0×
```

Every byte received (40000/40000, `match=1`) via 64-byte ET reads to `EAGAIN`,
across a stream that crossed the wire as 4 encrypted application records, zero
cleartext. Readiness was not reported before the handshake. **Honest nuance:** all 4
records drained in one ET edge (625 reads), not one-edge-per-record, because the 40
KB flood had already arrived and been decrypted into the socket buffer by the time
the post-kTLS wake fired — correct ET semantics (drain to `EAGAIN`); the "every byte,
none lost while buffered" property holds. A trickle-with-gaps test would exercise the
one-edge-per-record count; completeness is proven.

## Readiness semantics chosen for non-blocking accept

A non-blocking `accept4(SOCK_NONBLOCK)` returns the child fd immediately (the hold is
NOT on `accept()` — matches Linux, where `accept4` on a ready queue returns at once).
The child's readiness is WITHHELD by the masking `->poll` until kTLS is installed: the
app may add the fd to epoll and block in `epoll_wait`; the first readiness it observes
is after the kthread completes the server handshake. A denied inbound handshake
surfaces as `EPOLLERR` + `SO_ERROR` on the child. Mirrors the Unikraft B2 choice and
is Linux-faithful.

## The kernel bug found, diagnosed and fixed (run 0021 → 0030)

The first non-blocking boot (0021) proved item 3's OK case, then **panicked** on the handshake-deny sub-case:

```
[0.767466] BUG: TASK stack guard page was hit ...  Oops: stack guard page
RIP: __raw_spin_lock_irqsave <- tbl_get <- io_gate <- hooked_sendmsg <- hooked_sendmsg <- hooked_sendmsg (x60+)
```

Root cause (from the backtrace, not guessed): the prior OK connection reached
`ST_READY` and its per-`sk` table entry persisted after the socket closed (entries
are kept so `igkm_poll` can report readiness after kTLS; a kTLS socket's close
bypasses the `tcp_prot` hook because `sk_prot` is then `tls`). The kernel reused the
freed `struct sock *` for the next (hsdeny) socket; `io_gate` matched the stale
`ST_READY` entry and forwarded via `sk->sk_prot->sendmsg`, which for the new
(non-kTLS) socket is still the hooked `tcp_prot.sendmsg` → self-recursion → stack
overflow. This is the check-and-act / stale-cache hazard noted in the module's own
design comments.

Fix (reverified GREEN in 0030): evict any stale entry for a reused `sk` pointer in
`tbl_add` and at the top of `hooked_connect` (`:863`) / `hooked_accept` (`:912`), and
treat an `ST_READY` entry reached inside the hook as definitively stale — evict and
pass through, never forward (`io_gate` `:796`) — since a genuinely-kTLS socket never
enters the hook.

## Module source delta vs Spike D (`igkmd_mtls.c`, 614 → 989 lines)

- New `mesh_listen=` param + `is_mesh_listen()`; `hooked_accept` on `tcp_prot.accept`; `do_handshake` role (server ACCEPT frame 0x08 vs client OPEN).
- Per-socket `igkm_stream_ops` copy + masking `igkm_poll` + `igkm_swap_ops` (readiness suppression).
- Per-socket kthread driver `igkm_drive` + `spawn_driver`; the table entry gained role, kthread-driven flag, `ST_READY`/`ST_FAILED` + a waitqueue, and the server ACCEPT addrs.
- `io_gate` unifies first-I/O: inline (blocking client = Spike C/D regression, byte-for-byte), `EAGAIN` (non-blocking), wait-on-waitqueue (blocking caller on a kthread-driven socket), DENY→`sk_err`→`SO_ERROR`, plus stale-entry eviction.
- Host side needed NO protocol change: `relay.rs` already decoded the ACCEPT frame → rustls `ServerConnection` (`guest-server` SVID, client cert required), and `inclient.rs`/`gencerts` already had the inbound caller + certs. Only host edits: a `peer-denied` backend on :6445 (`peer.rs`) and an `inclient` `flood` mode — test fixtures, not mechanism.

## Edge cases / notes

- **`sockptr.h:49` fortify `WARN_ONCE`** fires once per boot on the first `TLS_TX` install, exactly as Spike C documented — expected, non-fatal; not fixed (out of scope).
- **Teardown debt (unchanged class from Spike C).** The module overwrites global `tcp_prot` function pointers and swaps per-socket `->ops`; a kTLS socket's close bypasses the `tcp_prot` hook, so entries are evicted lazily on `sk`-pointer reuse (the fix above) rather than on close. A production module needs real per-socket lifecycle (a close/ULP hook or `tls`-proto-aware teardown) and a safe `rmmod` that cannot race in-flight holds or leave a socket pointing at the module's `->ops`/`->poll`. The spike reboots the guest, so `rmmod` safety was not exercised. A sharper instance of the same debt: the background handshake kthread (`spawn_driver` → `igkm_drive`, `igkmd_mtls.c:730`) is handed a bare `struct sock *` with no `sock_hold`/refcount and no close-time join, so a workload that closes a non-blocking mesh socket *mid-handshake* can leave the kthread dereferencing freed memory — a guest-kernel use-after-free. Production needs a socket reference held across the handshake plus a close-time join. (Surfaced in PR #306 review, finding 3; same class as the run-0021 stale-`sk` panic.)
- **Fail closed on table exhaustion (not yet done).** The per-socket table is a fixed 64 slots; when it is full `tbl_add` returns NULL and `hooked_connect` (`igkmd_mtls.c:882`) still returns `orig_connect`'s result with **no entry installed** — so that socket's send/recv fall through to the original TCP handlers and a mesh connection could carry application **plaintext with no mTLS**. The probe never fills 64 slots, so no verdict here is affected, but a production module MUST fail the connection closed when no slot can be reserved (never fall through to plaintext) and carry a real-sized table. (Surfaced in PR #306 review, finding 1.)
- **The runner's exit code does not gate the verdict.** `run-fc.sh` / `capture.sh` can record `exit_code=0` for a run that timed out or whose wire scan failed (the plaintext/custody checks only print). The verdicts in this doc rest on the **pasted evidence** cross-checked against the committed `runs/` captures, not on the runner's exit status; a production test harness would make the required checks determine the exit code. (Surfaced in PR #306 review, finding 4.)
- **One-edge-per-record** was not exercised (burst arrival drained all records in one ET edge); completeness (every byte) was.
- Still deferred (unchanged from Spike C): IPv6 (`tcpv6_prot`), and the Cloud-Hypervisor VMM variant (#305).

## Design implications for #303

- **The Linux kernel module is now feasibility-proven for the full matrix:** client AND server, blocking AND non-blocking/epoll, with transparent guest kTLS, over the host-agent vsock relay — the SVID private key never crossing vsock in any case (custody 0× across all phases). This closes the two production-readiness gaps Spike C named. Together with Spike C/D (outbound + resolve/LB) the module can serve every mesh workload direction, the precondition for D1 (superseding the ADR-0069 host proxy for Linux VMs).
- **The accept()/server path cost almost nothing new** because the host relay's ACCEPT frame and the `guest-server` SVID were already designed and built (Spike B/C); only the guest-side hook + a role on the opening frame were missing.
- **Non-blocking readiness on Linux needs a per-socket kthread + a per-socket `proto_ops` poll interposition** — there is no HOLD primitive and `tcp_poll` is not patchable, so the mechanism is heavier than the Unikraft port only in adding a kthread + an ops swap per mesh socket. Production should bound this: one driver pool / event loop rather than a thread per connection, plus a real teardown.
- **`SO_ERROR` is the correct deny channel for a non-blocking connection**; a Shape-2 resolve-deny is even earlier (`connect()`=`EACCES`). Both proven.
- **KeyUpdate on the Linux kTLS path (#229) remains unexamined** — out of scope; the kernel owns the record layer, and the mesh's zero-tickets/no-KeyUpdate invariant (Spike C) governs.

## Gate recommendation

**PROMOTE.** The Linux kernel-module mechanism is proven end-to-end for client and
server, blocking and non-blocking/epoll, on the stock pinned-ABI kernel, with a clean
host. Remaining items are hardening (per-socket teardown / safe `rmmod` including the
close-mid-handshake kthread use-after-free, fail-closed on table exhaustion / never
fall through to plaintext, IPv6 `tcpv6_prot`, the CH VMM variant #305, KeyUpdate on
kTLS #229, and bounding the per-connection kthread), not feasibility — suitable for the
D1/D2 design decision and a production walking skeleton, not another feasibility spike.
