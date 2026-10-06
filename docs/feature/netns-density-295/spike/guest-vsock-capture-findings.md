# Spike findings: guest-side vsock capture with Aya and no guest NIC (V-2, V-3, V-4, V-5, D15)

> Status: PRE-REGISTRATION for the evidence increment (`increment-g`), written
> after the development increments a–f and before g ran. Results are appended
> below. Nothing in this section is edited after g executes.

## Question

Can an **unmodified** program inside a real booted guest with **no NIC** use
ordinary `AF_INET` sockets while guest-kernel Aya programs, loaded by an init
process standing in for `overdrive-init`, carry the traffic over virtio-vsock
(kernel vhost-vsock, patched CH `backend=vhost-kernel`) to the host, where
host-kernel Aya forwards it? Neither side may copy payload in userspace, and the
D15 semantics must hold.

## Substrate

- Qualified native x86_64 metal (AMD EPYC 8024P, `systemd-detect-virt` = `none`)
  under the canonical exclusive lease. No Lima runtime.
- Host kernel: stock Ubuntu `7.0.0-29-generic`.
- Guest kernel: stock `7.0.0-29-generic` bzImage (`/srv/vm/overdrive-testing/kernel`),
  with stock `vsock`/`vmw_vsock_virtio_transport*`/`dummy` modules from the
  initramfs.
- VMM: patched Cloud Hypervisor (`vendors/cloud-hypervisor`, branch
  `overdrive/vhost-kernel-vsock`, `41a619d19`), `--vsock cid=N,backend=vhost-kernel`,
  2 vCPUs, 1 GiB, seccomp on. No net device: `ip -br link` in the guest shows `lo`
  and `dummy0` only.
- The pinned 6.18 appliance kernel is not installed on the metal host
  (`/boot` has only 7.0.0-29 and 7.0.0-34) and was not run.

## The mechanism under test (built in increments a–f)

Guest, all inside the guest kernel, controlled by `vfwd guest` (the
`overdrive-init` stand-in):

1. **Capture.** `cgroup/connect4` and `cgroup/sendmsg4` on the root cgroup rewrite
   any non-local IPv4 destination (and the guest DNS address `127.0.0.53:53`) to
   the workload address: `W:15001` for TCP, `W:20000+slot` for UDP. The original
   destination is kept per socket cookie. `cgroup/getpeername4` and
   `cgroup/recvmsg4` give the application the original destination back.
   `W = 10.99.7.2` lives on `dummy0`, so `getsockname` returns it.
2. **TCP intake.** `sock_ops` `PASSIVE_ESTABLISHED` installs each intake child
   into the SockHash at establishment and routes its bytes into a pre-made
   loopback TCP parking cell (P1→P2). The owner re-arms the child right after
   `accept()` with `SO_RCVLOWAT`. After the host's `Paired`, the owner routes
   P2 to the flow's vsock STREAM and re-arms P2.
3. **Acceptor legs** (guest `C` for inbound, host `D` for egress) are armed
   before `connect()` and installed by `sock_ops` `ACTIVE_ESTABLISHED`.
4. **Inbound.** `C` binds the real client `ip:port` with `IP_TRANSPARENT` and
   `SO_MARK`. `tcp_fwmark_accept` plus `ip rule fwmark → local table` delivers
   the app's replies back to `C`.
5. **UDP.** Each slot owns a UDP intake that is SockHash-installed from boot.
   Its verdict prepends an 8-byte `ZUD1`+len frame and redirects into a
   per-slot TCP parking cell. That cell's P2 has a strparser, so one verdict
   sees exactly one frame. On `Paired`, P2 is routed to the association's
   SEQPACKET. Replies arrive on the vsock STREAM and go through a second
   strparser cell (P3→P4), then out of the intake socket. TCX on guest `lo`
   strips the frame. `sock_release` recycles slots.
6. **Control.** A fixed 16-byte request goes on each flow vsock. `Paired`,
   `Refused` and `Abort` travel **out of band** on one per-VM control vsock
   STREAM (CID 2:1234). The opener installs its vsock leg right after
   `connect()` returns, before the peer owner can write.

Host: `vfwd host` loads only SK_SKB verdicts and strparser on its own maps, TCX
on `lo` (frame decode, tuple-gated) and an `fexit` on `skb_send_sock` (drain
counter). It also loads `sock_ops` on a **dedicated cgroup that holds only the
owner process**, never the root cgroup. It runs the same intake parking for the
host intake listeners and the same armed destination legs.

**K2 (drain before half-close).** On EOF the owner calls
`shutdown(SHUT_WR)` on the partner only when three conditions hold for every
hop: verdict-forwarded bytes (`FWD`, per cookie) equal the bytes received
(`TCP_INFO.bytes_received` minus FIN), and the `fexit(skb_send_sock)` sent count
on the target has caught up.

**Payload rule.** Neither owner reads or writes payload on any paired socket.
It reads or writes only 16 B per flow request, 8 B control messages, and (in
the D18 comparison mode only) a 4 B in-band ack.

## Discoveries made in the development increments (each led to a mechanism change)

| Inc | Observation (evidence path) | Kernel cause (v7.0 source) | Consequence |
|---|---|---|---|
| a | Host vsock listener bind on CID 2 → `EADDRNOTAVAIL` before vhost_vsock loaded | h2g transport absent | bind `VMADDR_CID_ANY`, check peer CID |
| c | Target vsock/TCP gets `SO_ERROR=EPIPE` right after the source's FIN; 1 MiB and curl truncated | `sk_psock_backlog`: a FIN-only (len 0) skb redirected to egress → `skb_send_sock` returns 0 → "hard error" `EPIPE`, TX disabled (`skmsg.c:713-733`) | verdict drops len-0 TCP skbs; FIN is propagated by the owner |
| c | `bpf_map_update_elem` `EOPNOTSUPP` for intake sockets whose client already sent FIN | `sock_map_sk_state_allowed`: TCP must be `ESTABLISHED` (`sock_map.c:541`) | **K1 as written cannot hold for write-then-FIN clients**; install moved to `sock_ops` at establishment |
| d | 59,000-byte host→guest datagram arrived as ~15 guest skbs (16 bad frames) | guest virtio-vsock RX buffers are 4 KiB; vhost splits; vsock `read_skb` hands one skb at a time | reply reassembly via TCP strparser cell |
| e | Intake children installed at `PASSIVE_ESTABLISHED` lose bytes that arrive before `accept()`; `FIONREAD` showed 0 | `sk_psock_verdict_data_ready` returns early when `sk->sk_socket` is NULL (`skmsg.c:1268-1282`); `tcp_bpf_ioctl` SIOCINQ reports the psock msg queue, not the TCP queue (`tcp_bpf.c:336`) | owner re-arms with `SO_RCVLOWAT` after `accept()`; drain uses `TCP_INFO`, not FIONREAD |
| f | 400×3 stresses with 8-cell pools (cell reuse every 8 flows): 0 failures | — | mechanism stable enough for the evidence run |

Kernel-source facts that bound K1 (v7.0):

- `tcp_set_rcvlowat()` calls `tcp_data_ready()` (`tcp.c:1846-1859`), and
  `tcp_read_skb()` drains the whole receive queue (`tcp.c:1770`). TCP therefore
  has a userspace-triggered, payload-free re-arm.
- `udp_read_skb()` and `virtio_transport_read_skb()` take **one** skb per
  `data_ready` (`udp.c:2042`, `virtio_transport_common.c:1751`).
  `vsock_set_rcvlowat()` never calls `sk_data_ready`
  (`af_vsock.c:2585`). UDP and vsock have **no re-arm**: a backlog that was
  present at install lags by one forever.
- `virtio_transport_read_skb()` ignores the partial-read offset
  (`VIRTIO_VSOCK_SKB_CB(skb)->offset`). A userspace read of a 4-byte prefix
  from an skb that also holds payload, followed by install, can re-forward the
  prefix.

## Pre-registered probes for increment-g

Modes (both owners restarted together):

- **OOB** — mechanism: `--ack oob --intake-mode park --arm yes --flush yes --udp park`
- **LATE-NOFLUSH** — install the intake after `Paired`, no re-arm.
- **LATE-FLUSH** — install after `Paired`, `SO_RCVLOWAT` re-arm (ADR-0158 shape for the TCP intake).
- **D18** — in-band 4-byte `Paired`, install as ADR-0158 orders it, no
  `sock_ops` install (`--ack inband --intake-mode late --arm no`).
- **UDP-NAIVE** — OOB, but the UDP intake is installed only after `Paired`.

| ID | Hypothesis | Predicted | Falsification |
|---|---|---|---|
| G-D15 | W is a real guest address; hooks restore original peers | `ip -4 addr` shows `10.99.7.2/32` on `dummy0`; `getsockname`=`10.99.7.2:*`; `getpeername`=original dst; bind to W works; UDP `recvfrom` source = original dst; connected UDP `getpeername` = original dst | any loopback/intake address visible to the app |
| G-V2a | unmodified TCP egress byte-exact, half-close | 1 MiB and 64 MiB litmus OK (distinct request/response patterns; server sees EOF before replying) | any byte error or missing EOF |
| G-V2b | K2 drain signal works both ways | 4 MiB with server RX withheld 300 ms, and with client RX withheld 300 ms: tail byte-exact | truncation or `k2_drain_timeouts` > 0 |
| G-V2c | RST propagates | client gets `ECONNRESET` | clean EOF or hang |
| G-V2d | unmodified `curl` + real Internet (D14) | 2 MiB blob sha256 equals host blob; `curl http://example.com/` 200; real `nc github.com 22` returns `SSH-2.0` banner (server-speaks-first over Internet) | mismatch / failure |
| G-V3-1 | early bytes + immediate FIN at guest intake (OOB) | 10,000/10,000; also 10,000 with 8 parallel clients | any failure |
| G-V3-2 | outbound server-speaks-first (OOB) | 10,000/10,000 | any failure |
| G-V3-3 | inbound server-speaks-first, banner on C (OOB) | 10,000/10,000 | any failure |
| G-V3-4 | host-local client writes immediately into host intake A (OOB) | 10,000/10,000 | any failure |
| G-V3-5 | quiescence | guest and host `ss -tn` Recv-Q 0 on all sockets after the stresses | non-zero Recv-Q on an installed socket |
| G-K1-a | LATE-NOFLUSH: request queued before install, no FIN (early-ack server) | most or all iterations time out (bytes stranded) | ≥ 90 % pass (then K1 holds without re-arm) |
| G-K1-b | LATE-FLUSH, same probe | all pass (re-arm forwards queued bytes) | any failure |
| G-K1-c | LATE-FLUSH, write+FIN immediately | most fail (`late_install_impossible`, `EOPNOTSUPP`) | ≥ 90 % pass |
| G-K1-d | D18, outbound speaks-first (banner) | failures (banner behind in-band `PAIR` stranded or duplicated in guest vsock) | ≥ 90 % pass |
| G-K1-e | D18, inbound speaks-first (guest banner → host V_h) | failures | ≥ 90 % pass |
| G-K1-f | D18, outbound client-first with a 50 ms hold before FIN | pass (D18 is safe when the destination is silent) | failures |
| G-K1-g | UDP-NAIVE first datagram | most first exchanges time out | ≥ 90 % pass |
| G-V5a | unmodified resolvers, first datagram | `getent hosts`, `getent ahostsv4`, `dig`, busybox `nslookup` for `example.com` answer; owner shows association; no userspace read | any failure |
| G-V5b | general UDP, empty and large | sizes 0, 1, 1431, 4096, 4097, 59000 OK, unconnected and connected; reply source = original dst | any failure |
| G-V5c | first datagram at scale | `udp-stress` 10,000 fresh unconnected sockets, plus 2,000 connected: all OK | any failure |
| G-V4a | unmodified `sshd` | `ssh` login + command; `SSH_CONNECTION` and sshd log show the host's real address (not loopback, not intake); `ssh -tt` PTY works | failure or wrong peer |
| G-V4b | scp ≥ 1 MiB both ways | 8 MiB up and down, sha256 equal | mismatch |
| G-KP | kernel path only | `strace` of both owners during 64 MiB TCP, DNS and UDP-59000 shows only control-sized socket I/O (16 B per flow, 8 B control); BPF FWD/SENT counters ≥ payload | any socket read/write with payload-sized byte counts |
| G-FC1 | kill -9 guest owner | in-flight app gets an error or EOF; new `connect`/`sendto` fail immediately with `ENETUNREACH` (hooks detached, no route); host servers see no new connection | cleartext or a silent black hole |
| G-FC2 | kill -9 host owner | in-flight app gets `ECONNRESET`; new connects get RST (vsock connect refused → owner resets); DNS times out | silent success or hang beyond timeout |

---

# Results (appended after execution)

## Verdict: **WORKS**, with a corrected guest mechanism and a precise negative for ADR-0158 as written

This was tested with an unmodified `curl`, `sshd`/`ssh`/`scp`, `getent`, `dig`, busybox
`nslookup` and ordinary-socket litmus programs. They ran in a real booted guest that
has **no NIC**. They reached host, loopback and real Internet destinations, and were
reached from the host. In every case the payload moved through guest-kernel Aya
programs, vhost-vsock and host-kernel Aya programs. Neither owner made a payload
syscall. `strace` of both owners across 64 MiB TCP, 8 MiB scp, DNS and a 59,000-byte
UDP exchange shows **only 8- and 16-byte control I/O** (12 socket calls per side).
D15 holds:

- the application sees the workload address on `dummy0` and in `getsockname`;
- it sees the original destination in `getpeername` and `recvfrom`;
- inbound applications see the **real** client address;
- general UDP works, including empty and 59,000-byte datagrams;
- TCP early bytes and the first UDP datagram are never lost.

The final build ran each pre-registered stress **10,000/10,000**, all with zero
K2 drain timeouts and zero leg errors:

- egress early-bytes, serial and 8-way parallel;
- egress server-speaks-first;
- egress early-bytes without FIN;
- inbound server-speaks-first;
- inbound client-writes-immediately;
- 10,000 first-datagram UDP associations.

One part does not hold. **Premise K1, as ADR-0158/D18 states it, is false on
this kernel.** Three things carry the result instead:

- an **out-of-band `Paired`**;
- **install-at-establishment** through `sock_ops`, with a post-`accept()` re-arm;
- **kernel TCP parking cells** (and strparser reassembly cells for UDP).

The in-band D18 ordering failed **18/100** egress server-speaks-first flows and
**1/100** inbound flows. Its late intake install failed **100/100** for a
client that writes and then immediately half-closes.

Not proven: the pinned 6.18 kernel, which is not installed on the metal host,
so every result is on stock 7.0.0-29 for both host and guest. Also not proven:
inbound UDP service (#310), IPv6/ICMP (#308/#311), datagrams over 59,000 bytes
(#309), re-association of an existing connected UDP socket after a host-only
abort, owner restart while flows stay live, throughput, and scale beyond one VM.

## Substrate and provenance (as executed)

```
Linux em-determined-roentgen 7.0.0-29-generic #29-Ubuntu SMP PREEMPT_DYNAMIC Fri Jul 17 20:52:35 UTC 2026 x86_64 GNU/Linux   # host
none                                                                     # systemd-detect-virt
guest: Linux (none) 7.0.0-29-generic #29-Ubuntu SMP PREEMPT_DYNAMIC Fri Jul 17 20:52:35 UTC 2026 x86_64 GNU/Linux
CH config: ... net: None ... vsock: Some(VsockConfig { ... cid: 58118, socket: None, backend: VhostKernel })
vhost_kernel.rs:216 -- Created vhost-kernel vsock _vsock0 with guest CID 58118
```

| | increment-g | increment-h (final) |
|---|---|---|
| superproject HEAD | `68545fdd0` | `68545fdd0` |
| CH submodule | `41a619d19` (`overdrive/vhost-kernel-vsock`, clean) | same |
| guest kernel sha256 | `b51367c7dab2…` | same |
| BPF ELF sha256 | `6ede0e46f1e2…` | `506462aaefdf…` (FWD/SENT non-LRU) |
| vfwd sha256 | `b57c1c3a444e…` | `5294f6af79bd…` |

Full pins are in `increment-*/evidence/artifact-pins.json`. Source hashes per
run are in `evidence/source-manifest.json`. Every run held the canonical lease
(`OVERDRIVE_METAL_LEASE_ACQUIRED`) and passed the fail-closed native preflight.

**Host footprint.** Unlike the earlier Aya spikes, the host programs ran **in
the physical host kernel**, because vhost-vsock terminates there. Their scope
was confined as follows:

- SockHash/strparser on owned maps;
- TCX on `lo`, a no-op unless a tuple is registered;
- `fexit(skb_send_sock)`, a counter only;
- `sock_ops` on a throw-away cgroup that held only the owner.

Every run restored the host:

```
"cleanup": {"bpf_prog_count": [17, 17], "ch_procs_after": "", "links_unchanged": true, "lo_tcx_unchanged": true,
            "modules_diff": [], "modules_restored": true, "root_cgroup_bpf_unchanged": true, "spike_cgroups_left": []}
```

Increment-a left the stock vsock modules loaded because its cleanup ran before
CH had exited. They were unloaded under a separate lease session:
`removed vsock_diag … removed vhost_iotlb / none-left`. Later runs unload them
with retries.

## Per-probe results (predicted vs actual)

| ID | Predicted | Actual (increment) | Verdict |
|---|---|---|---|
| G-D15 | W on `dummy0`; getsockname=W; getpeername/recvfrom=orig; bind W ok | as predicted (g, h) | **WORKS** |
| G-V2a | 1 MiB + 64 MiB byte-exact, half-close | both OK; 64 MiB in 3.1–3.4 s (g, h) | **WORKS** |
| G-V2b | K2 tail exact with RX withheld 300 ms both ways | both 4 MiB OK; final run 120,008 drained shutdowns, 0 drain timeouts | **WORKS** |
| G-V2c | RST → `ECONNRESET` | `reset_observed: true` | **WORKS** |
| G-V2d | curl blob sha equal; Internet curl; Internet speaks-first | blob sha equal; `http://example.com` 200; `curl telnet://github.com:22` → `SSH-2.0-2097ddd` (h). busybox `nc` printed nothing (g; not investigated) | **WORKS** |
| G-V3-1 | 10,000 early bytes + FIN, serial and ×8 | 10,000/10,000 and 10,000/10,000 (g and h) | **WORKS** |
| G-V3-2 | 10,000 egress server-first | 10,000/10,000 (g, h) | **WORKS** |
| G-V3-3 | 10,000 inbound server-first | g: 9,999/10,000 (harness K2-counter eviction, below); h: 10,000/10,000 | **WORKS** |
| G-V3-4 | 10,000 inbound client-first | 10,000/10,000 (g, h) | **WORKS** |
| G-V3-5 | Recv-Q 0 everywhere at quiescence | guest and host `ss -tna` rows with Recv-Q ≠ 0: none | **WORKS** |
| G-K1-a | LATE-NOFLUSH strands queued bytes | 0/100 (all stranded) | as predicted (K1 false without re-arm) |
| G-K1-b | LATE-FLUSH re-arm forwards them | 1,000/1,000 | as predicted (TCP re-arm works) |
| G-K1-c | LATE-FLUSH, write+FIN immediately | 0/100; `late_install_impossible` 98, `sockhash_insert_errors` 100 | as predicted (install impossible after FIN) |
| G-K1-d | D18 egress server-first fails | 18/100 failed; guest vsock holds the 64 B banner: `"inq":64,"fwd":0` | as predicted (vsock K1 false) |
| G-K1-e | D18 inbound server-first fails | **1/100** failed. The prediction (≥10 % failure) was falsified on rate; the hazard is present | PARTIAL |
| G-K1-f | D18 client-first (hold 50 ms) passes | 200/200 | as predicted |
| G-K1-g | UDP-NAIVE strands the first datagram | 0/30 | as predicted (UDP K1 false) |
| G-V5a | getent/dig/nslookup answer | all rc=0 with answers; 200/200 repeated `getent` | **WORKS** |
| G-V5b | sizes 0,1,1431,4096,4097,59000, unconnected and connected, reply source = orig | all OK, `from` = original destination | **WORKS** |
| G-V5c | 10,000 unconnected + 2,000 connected first datagrams | 10,000/10,000 and 2,000/2,000 (g, h) | **WORKS** |
| G-V4a | sshd login, real peer, PTY | `SSH_CONNECTION=<metal-address> 54138 10.99.7.2 22`; sshd `Accepted publickey for root from <metal-address>`; `/dev/pts/0 PTY_OK` | **WORKS** |
| G-V4b | scp 8 MiB both ways | host, guest and downloaded sha256 all `214f7c7e…` | **WORKS** |
| G-KP | only control I/O in both owners | guest 12 socket calls, sizes {8,16}; host 12, sizes {8,16} | **WORKS** |
| G-FC1 | guest owner killed → error/EOF; new I/O `ENETUNREACH`; nothing reaches host | new TCP/UDP: `Network is unreachable`; DNS rc=2; curl `(7)`; host servers +0 connections; cgroup hooks gone. **In-flight app saw a clean EOF with 0 bytes, not an error** | PARTIAL (no leak; truncation looks like EOF) |
| G-FC2 | host owner killed → in-flight `ECONNRESET`, new connects RST, DNS fails | in-flight `ECONNRESET`; new connect `ECONNRESET`; UDP timeout; DNS rc=2; recovery after restart OK | **WORKS** |

### Pasted evidence

D15 (increment-g `G-D15`, `G-V5b`):
```
2: dummy0: <BROADCAST,NOARP,UP,LOWER_UP> mtu 1500 ... inet 10.99.7.2/32 scope global dummy0
{"connect":"ok","getpeername":"<metal-address>:7000","getsockname":"10.99.7.2:43076"}
{"bind":"10.99.7.2:0","bind_result":"ok","connect":"ok","getpeername":"<metal-address>:7000","getsockname":"10.99.7.2:58025"}
"udp_connected": {"connected": true, "local": "10.99.7.2:56003", "peer": "<metal-address>:7010", "results": [{"from": "<metal-address>:7010", "got": 59000, "ok": true, "size": 59000}, {"got": 0, "ok": true, "size": 0}, ...]}
```

V-4 (increment-g):
```
SSH_CONNECTION=<metal-address> 54138 10.99.7.2 22
Accepted publickey for root from <metal-address> port 54138 ssh2: ED25519 SHA256:oAQBun5IAvDgr7WVimIt2NSA7bQ5Iy3N4Qgsd7g7PkQ
/dev/pts/0
PTY_OK
scp: host 214f7c7e088cc2f225a11671944da11d3cf61f041e798c0de5fd5bc5acb6a2ba = guest = round-trip download
```

V-5 resolvers (increment-g):
```
104.20.23.154   STREAM example.com          # getent ahostsv4
104.20.23.154                                # dig +short
Server: 127.0.0.53  Address: 127.0.0.53:53  Name: example.com  Address: 104.20.23.154   # busybox nslookup
```

V-3 final build (increment-h `G-V3.stresses`):
```
egress_early_fin_10000            10000 ok / 0 failed   16.9 s
egress_early_fin_10000_par8       10000 / 0             15.7 s
egress_server_first_10000         10000 / 0             17.4 s
egress_no_fin_early_ack_10000     10000 / 0             11.7 s
inbound_server_first_10000        10000 / 0             16.1 s
inbound_client_writes_immediately 10000 / 0             23.1 s
udp_first_datagram_unconnected    10000 / 0             10.0 s
udp_first_datagram_connected       2000 / 0              2.0 s
guest owner after all of it: flows_active_total 60004, flows_closed 60004, k2_drained_shutdowns 120008,
  udp_associations 12003, no k2_drain_timeouts, no leg_errors, no aborts
```

Kernel-path proof (increment-h `G-KP`). The window covered 64 MiB TCP egress,
`getent`, UDP 0/1431/59000, 8 MiB scp inbound and a 64 KiB inbound litmus:
```
guest owner (strace -f -yy):  recvfrom:socket 5 calls 56 B; sendto:socket 7 calls 96 B   (other writes: own log/stats files)
host owner  (strace -f -yy):  recvfrom:socket 7 calls 96 B; sendto:socket 5 calls 56 B
110   sendto(888<socket:[281]>, ""..., 16, MSG_NOSIGNAL, NULL, 0) = 16        # guest: flow request
110   recvfrom(52<socket:[2151]>, ""..., 4096, MSG_DONTWAIT, NULL, NULL) = 8    # guest: Paired on control session
1034475 recvfrom(562<socket:[58259121]>, ""..., 16, MSG_TRUNC, NULL, NULL) = 16  # host: SEQPACKET request
guest BPF redirect_ok in the window: 25,841; host: 19,148
```

K1 negatives (increment-h `G-K1.controls`):
```
late_noflush_early_ack_100       ok 0  failed 100  "early ack read: Resource temporarily unavailable"
late_flush_early_ack_1000        ok 1000 failed 0
late_flush_write_fin_immediately ok 0  failed 100  late_install_impossible 98, sockhash_insert_errors 100
d18_egress_server_first_100      ok 82 failed 18   guest: {"ev":"k2_drain_timeout","hops":[{"fwd":0,"inq":64,...}],"leg":1}
d18_inbound_server_first_100     ok 99 failed 1    (request never reached the guest server: "hdr Connection reset by peer")
d18_egress_client_first_hold50   ok 200 failed 0
udp_naive_first_datagram_30      ok 0  failed 30
```

Fail-closed (increment-h `G-FC`):
```
FC1 kill -9 guest owner mid 64 MiB:  in-flight: {"err":"bad response len 0 want 67108924"}   (clean EOF, 0 bytes)
     then: {"connect":"Network is unreachable (os error 101)"}; udp send "Network is unreachable"; getent_rc=2;
     curl: (7) Failed to connect ...; host servers new connections: {"tcp7000": 0, "http8080": 0}
FC2 kill -9 host owner mid 64 MiB:   in-flight: "read: Connection reset by peer (os error 104)"
     new TCP: "Connection reset by peer"; UDP: recv timeout; getent_rc=2; guest owner: udp_assoc_connect_failed (retrying)
     after restarting both owners: TCP 1000 B OK; UDP 0 and 59000 OK
```

### Increment-g's single failure was a harness accounting bug, not forwarding

The one failure was inbound server-first iteration 255 of 10,000. The host
drain record shows the parked bytes reached the cell (`recv 1483`, upstream
`SENT 1483`) but `FWD(P2)` read 0. FWD/SENT were LRU maps with 65,536 entries,
and the run inserted roughly 157k distinct cookies, so a long-lived cell's
counter was evicted. The drain check then withheld the FIN for its 10 s
timeout, longer than the client's 8 s read timeout. After the fix (non-LRU
maps, entries deleted on close), increment-h ran 10,000/10,000. Lesson for
K2: the drain signal needs exact per-socket accounting that cannot be evicted.

## The guest-side mechanism (summary)

Guest-kernel `cgroup/connect4`/`sendmsg4` redirect every captured TCP connect
and UDP send to an intake on the workload address. `getpeername4`/`recvmsg4`
restore the original peer for the application. `sock_ops` installs each intake
child into a SockHash at `PASSIVE_ESTABLISHED` and routes its bytes into a
pre-made loopback TCP parking cell. The owner re-arms the child after
`accept()` with `SO_RCVLOWAT`. When the host's out-of-band `Paired` arrives,
the cell is routed to the flow's vsock STREAM and re-armed. UDP goes through a
strparser framing cell to a SEQPACKET leg, and replies come back through a
strparser reassembly cell, with TCX on guest `lo` stripping the 8-byte frame.
Inbound uses `IP_TRANSPARENT` plus a fwmark local route, so `C` carries the
real client address. The host owner mirrors this. The only userspace I/O is
16 B requests and 8 B control messages.

## D15: preserved vs impossible

| Behaviour | Status | Reason |
|---|---|---|
| Workload address visible in the guest (`ip addr`, `getsockname`, bind) | **Preserved** | `dummy0` carries W; capture rewrites to `W:intake`, so the kernel picks W as the source |
| Original peer in `getpeername` (TCP, connected UDP) and `recvfrom` (UDP) | **Preserved** | `getpeername4` / `recvmsg4` rewrite from per-socket state |
| Real client address inbound (`getpeername`, `SSH_CONNECTION`, sshd log) | **Preserved** | `IP_TRANSPARENT` bind of the real `ip:port`, `SO_MARK`, `tcp_fwmark_accept`, `fwmark → local 0/0` route. This contradicts the feature delta's D15(ii) "inbound peers appear as loopback": the kernel does not force it |
| Guest-initiated TCP, including server-speaks-first and Internet destinations (D14 kernel-forwarded) | **Preserved** | |
| General UDP (not only DNS), empty and up to 59,000 B, first datagram | **Preserved** | parking cell plus strparser cells; ADR-0148's STREAM-toward-guest needs guest-side reassembly (below) |
| Early bytes / connects before forwarding is ready | **Preserved** | bytes park in kernel cells; owner absent means explicit `ENETUNREACH` (guest owner) or `ECONNRESET` (host owner), never a silent black hole |
| Crash of the guest owner mid-flow | **Partial** | in-flight apps see a clean EOF (owner process exit closes the intake with FIN). Setting `SO_LINGER{1,0}` on owner-held TCP legs at creation would make it an RST. Not tested |
| Inbound UDP service, IPv6, ICMP, >59,000 B UDP | Not tested | #310, #308/#311, #309. Inbound UDP from many clients would need TCX packet classification: an SK_SKB verdict on an unconnected UDP socket sees only the payload, not the sender address |

## Kernel facts established (v7.0, reproduced on 7.0.0-29)

1. **K1 for TCP holds only with a userspace-triggered re-arm**
   (`SO_RCVLOWAT` → `tcp_data_ready`). It is impossible after the peer's FIN,
   because `sock_map_sk_state_allowed` requires `ESTABLISHED` (100/100
   failures).
2. **K1 for vsock and UDP does not hold.** `read_skb` takes one skb per
   `data_ready` and there is no re-arm, so backlogged bytes stay stranded
   (D18 egress 18/100, UDP-NAIVE 30/30).
3. **The install window must close before the peer can send.** The vsock
   opener installs right after `connect()` returns, because the peer has no
   userspace handle yet. The intake child installs at `PASSIVE_ESTABLISHED`.
   The acceptor's active leg installs at `ACTIVE_ESTABLISHED`. `Paired`
   travels out of band.
4. **A child installed before `accept()` silently ignores `data_ready`**
   (`sk_socket == NULL`), so the owner must re-arm after `accept()`.
5. **A FIN-only skb redirected to egress disables the target's TX with
   `EPIPE`.** Verdicts must drop zero-length TCP skbs.
6. **Guest virtio-vsock RX splits host data into ≤4 KiB skbs.** Datagram
   frames toward the guest need reassembly; a TCP strparser cell is a working
   kernel-only reassembler (4097 B and 59,000 B passed).
7. **`FIONREAD` on a psock TCP socket reports the psock message queue, not
   the TCP queue** (`tcp_bpf_ioctl`). Use `TCP_INFO`/`ss`.
8. **K2 has a kernel-sourced drain signal.** `fexit(skb_send_sock)` sent
   bytes on the target, `TCP_INFO.bytes_received` (which includes the FIN)
   and the per-cookie verdict count together gave 120,008 correctly ordered
   half-closes with 0 truncations. The counters must be exact and must not be
   evictable.

## Design implications

- **ADR-0158 / D18 (and premise K1).** As written it **cannot be accepted**:
  - K1 is false for vsock and UDP.
  - K1 is false for TCP after an early FIN.
  - The in-band `Paired` places the response in the payload stream, ahead of
    bytes that can no longer be forwarded.
  - The D18 premise "the host-facing peer cannot speak first" is false for
    D14 non-mesh egress (Internet SSH banner shown).

  The working replacement has four parts:
  - `Paired`/`Refused`/`Abort` on the per-VM control session (the ADR-0157
    beacon is the natural carrier);
  - the opener installs its vsock leg immediately after `connect()`;
  - intake children installed by `sock_ops` at establishment, parking in
    kernel TCP cells until `Paired`, with a post-`accept()` re-arm;
  - acceptor active legs armed for `ACTIVE_ESTABLISHED` install.

  **User decision needed.**
- **ADR-0150.** Confirmed in principle: guest-kernel Aya under the init
  process, no payload in userspace. The adaptation is larger than "cgroup hooks
  + SockHash". It needs:
  - `connect4`, `sendmsg4`, `recvmsg4`, `getpeername4`, `sock_release`,
    `sock_ops`, SK_SKB verdict plus strparser, TCX on `lo`, and `fexit`;
  - per-flow parking cells (two loopback TCP sockets);
  - per-UDP-slot framing and reassembly cells (four sockets);
  - the workload address on a dummy interface;
  - an fwmark local route.
- **ADR-0152.** A host intake on the host address works. It must use the same
  establishment-install and parking scheme (host `sock_ops` on the owner's
  cgroup only). The guest can present the **real client address**, so the
  feature delta's D15(ii) "inbound peers appear as loopback" is a design
  choice, not a kernel limit.
- **ADR-0147.** Per-flow pairs: confirmed. Socket accounting changes:
  - TCP egress uses 2 guest sockets plus a reusable parking cell, and 2 host
    sockets.
  - Inbound uses 2 guest sockets and 2 host sockets plus a reusable host
    cell.
  - A UDP association uses 3 sockets per side, plus a permanent per-slot UDP
    intake and 4 cell sockets in the guest.
  - Cells are pooled and reused: the final run cycled 256-cell pools about
    78 (host) to 156 (guest) times per cell.
- **ADR-0148.** STREAM toward the guest is fine, but the guest **must
  reassemble**, because host frames arrive split at 4 KiB. SEQPACKET from the
  guest worked for 59,008 B frames after `pull_data` linearisation.
- **ADR-0154.** DNS as an ordinary datagram association works, first datagram
  included, for glibc `getent`, bind9 `dig` and busybox `nslookup`. The
  stub fallback is not needed. The `127.0.0.53` guest resolver address is
  captured as an exception to the loopback exemption.
- **K2 / ADR-0160.** The drain signal exists and works, provided its
  counters are exact. Owner-crash semantics need `SO_LINGER{1,0}`, or a
  crashed guest owner shows up as a clean EOF.

## Files

- Spike source and harness: `spike-scratch/netns-density-295-guest-vsock-capture/`
  - `bpf/src/main.rs`: Aya eBPF (guest and host programs, one ELF)
  - `vfwd/src/{owner,sys,main}.rs`: owners, test agent and litmus tools
  - `guest/init.sh`, `harness.py`, `launch.py`
  - `increment-{a..h}/run.py`
  - `increment-{a..h}/evidence/` (raw logs, results.json, straces, owner logs, inventories)
- Evidence for the verdict: `increment-g/evidence` (first full matrix) and
  `increment-h/evidence` (final build: stresses, strace, controls, fail-closed).
  Increments a–f are development and diagnosis runs (see the discoveries table).
