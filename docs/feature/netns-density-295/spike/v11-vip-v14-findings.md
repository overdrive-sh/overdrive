# Spike findings: off-host guest UDP (V-11, D5a), guest traffic to a service VIP, guest listen-state mirroring (V-14, D23)

> Status: PRE-REGISTRATION for the evidence increment (`increment-e`), written after the
> development increments a–d and before e ran. Results are appended below. Nothing in this
> section is edited after e executes.

## Questions

1. **V-11.** Can an unmodified guest program send UDP (connected and unconnected) to a
   destination that is **not on the host**, with sizes 0, 1, 1,431, 4,096, 4,097 and 59,000
   bytes, get replies with the correct source address (D15), and have its first datagram
   delivered under repetition, with no userspace payload copy? **D5a**: which removal point
   for the internal 8-byte datagram frame works?
2. **VIP.** How does guest-originated UDP and TCP reach a service VIP when the host-side
   socket lives in the guest-flow owner, outside any workload cgroup? Prove one kernel-only
   mechanism end to end, including the reply source address for UDP.
3. **V-14.** Can the guest kernel tell the host, without a payload path, when an
   application starts and stops listening, so the host intake listener exists only while
   the guest application listens (D23)?

## Substrate

- Qualified native x86_64 metal under the canonical exclusive lease (AMD EPYC 8024P,
  `systemd-detect-virt` = `none`). No Lima runtime (Lima was used only to compile-check).
- Host kernel and guest kernel: stock Ubuntu `7.0.0-29-generic` (the guest boots the same
  bzImage, `/srv/vm/overdrive-testing/kernel`). The pinned 6.18 appliance kernel is not
  installed on the host and was not run.
- VMM: patched Cloud Hypervisor (`vendors/cloud-hypervisor`, `overdrive/vhost-kernel-vsock`),
  `--vsock cid=N,backend=vhost-kernel`, no net device. The guest has `lo` and `dummy0`
  (workload address `10.99.7.2`) only.

### What "off-host" means here

A second machine was not available. Two stand-ins were used, and the findings say which
one each result comes from:

- **netns peer behind a real egress interface.** Network namespace `v11p` behind veth
  `v11h` (`10.250.95.1/24` ↔ `10.250.95.2/24`, MTU 1500 on both ends). The host owner's
  datagram leaves the host's root namespace through `v11h`, exactly as it would leave
  through a NIC, and is IP-fragmented to the 1500-byte MTU. The echo server is in the
  namespace. This carries the size matrix and the stresses.
- **the physical NIC** `eno12399np0` (MTU 1500, the default route). Real Internet DNS
  (`dig @1.1.1.1`) and wire captures (`tcpdump` on the NIC) of datagrams to the TEST-NET
  address `192.0.2.77:9`. These have no echo, so they prove what leaves the wire, not a
  round trip.

## Mechanism reused and what was added

Everything from the guest-capture spike (`spike/guest-vsock-capture-findings.md`) is
reused unchanged: guest cgroup capture hooks, `sock_ops` install at establishment, parking
cells, SEQPACKET guest→host datagram leg with the 8-byte `ZUD1`+length frame, strparser
reassembly toward the guest, out-of-band `Paired` on the control session. Added:

- **Host verdict modes** (Aya SK_SKB, `bpf_skb_adjust_room(-8)` in the verdict):
  - `verdict` (D5a option b): verify the frame, remove it, send the bare datagram; an
    empty datagram is dropped and counted.
  - `verdict-naive`: the same, but the zero-length result is redirected anyway (control).
  - `hybrid`: remove the frame in the verdict, except keep it when the datagram is empty
    or when the application payload itself parses as a frame (escape rule). The egress TC
    then strips any valid frame on that tuple and passes everything else.
- **Egress TC programs** (Aya TCX, tuple-gated, no per-VM attachment):
  - `tc` (D5a option a, as ADR-0149 describes it): the existing `lo_decode` attached to the
    egress of `lo`, `v11h` and the NIC.
  - `tc-frag` (option a2): fragment-aware. It removes 8 bytes from the first fragment of a
    registered tuple, records `(src, dst, IP id)`, and moves every later fragment of that
    datagram back by one 8-byte offset unit.
- **Tuple registration from the kernel peer.** The tuple is registered from
  `getpeername()` of the connected host socket, not from the requested destination, so it
  stays correct when `connect4` rewrites the destination (VIP). A `--tuple-from requested`
  control reproduces the old behaviour.
- **VIP**: `vip_connect4`, a line-for-line mirror of production `cgroup_connect4_service`
  (ADR-0053; same `(vip, port, proto)` host-order key and value layout), attached to the
  cgroup that holds the host owner. The owner always connects its host UDP and TCP sockets,
  so `connect4` sees every guest flow.
- **V-14**:
  - Guest: `fexit` on `inet_csk_listen_start` (return 0) and on `inet_csk_listen_stop`.
    Each writes a wake event (kind, cookie, port, ktime) to a ring buffer. `fexit` runs
    after the socket is hashed into, or removed from, the listening table.
  - Guest owner: on each wake, re-derive every declared port's state from the kernel
    (`NETLINK_SOCK_DIAG`, `TCPF_LISTEN` only; IPv4 `0.0.0.0`/W, IPv6 `::`/`::ffff:W`).
    It sends `ListenState` (8 bytes: type, listening, port, seq) on the control vsock only
    when a port's state changes. It sends every port's state when the session opens, and
    runs a 1 s level-triggered audit as a backstop.
  - Host owner: binds the intake listener for `(alloc, port)` when the guest reports
    listening, and closes it when the guest reports not listening or the control session
    is lost.
  - The guest re-opens a lost control session every 250 ms.
- Guest `net.ipv4.fwmark_reflect=1` (increment-c discovery, below).

## Discoveries in the development increments

| Inc | Observation | Cause | Consequence |
|---|---|---|---|
| b | `tc`: every datagram ≥ 4,096 B to the netns peer and to the NIC is lost; `tcpdump` on the NIC shows **no fragment at all** | `ip_fragment()` runs before TC egress. The first fragment fails the program's length check and is `TC_ACT_SHOT`, and `ip_do_fragment()` stops sending the remaining fragments when one transmit fails | ADR-0149 option (a) as written works only up to the egress MTU. Added `tc-frag` |
| b | `verdict-naive`: after one empty datagram, every later datagram on the same association is lost | a zero-length redirect makes `skb_send_sock` return 0, so the psock reports `EPIPE` and disables TX (same mechanism as the guest-capture spike's FIN finding) | option (b) must drop empties |
| b | first `hybrid`: empties OK, every non-empty datagram dropped | the tuple-gated TC still tried to unframe datagrams the verdict had already stripped, and dropped them on bad magic | `hybrid` needs a lenient tuple mode plus the escape rule |
| c | a host connect to an always-bound intake for a guest port with **no** listener took ~3 s to reset; the owner's blocking acceptor `connect()` stalled the guest loop, delaying listen reports by ~1 s; the probe flood then exhausted the host owner's 1,024 fds (`EMFILE`) | the guest's RST toward the transparent client address carries no mark, so the fwmark route never delivers it and the acceptor waits for its 3 s timeout | guest `net.ipv4.fwmark_reflect=1` (the refusal then takes ~1 ms); owner `RLIMIT_NOFILE` 65,536 |
| c | VIP host-local TCP took 2 s | harness: an unmapped VIP port made the host owner block 5 s in a `connect()` inside its event loop, delaying the next flow | completed the map. **Design note**: owner connects must not block the event loop |
| d | listen mirror p50 ≈ 8 ms in both directions | 5–11 ms of that was reading `/proc/net/tcp`, which is large because of the owner's own pooled sockets | switched to `NETLINK_SOCK_DIAG` with `TCPF_LISTEN`, which walks only the listening hash |

## Pre-registered probes for increment-e

The D5a options are compared on one booted VM. Both owners are restarted per option.
Destinations: `P` = netns peer echo `10.250.95.2:7210`, NIC = TEST-NET via `eno12399np0`.

| ID | Hypothesis | Predicted | Falsification |
|---|---|---|---|
| E-a-matrix | `tc` unframes only datagrams that fit the egress MTU | P: 0, 1, 1431 OK (connected and unconnected, `from` = P); 4096, 4097, 59000 fail; NIC capture: 0 → `UDP, length 0`, 1431 → `length 1431`, 4096 → no packet | any size ≥ 4096 OK, or any frame on the wire |
| E-a2-matrix | `tc-frag` unframes every size, fragmented or not | all 12 OK with `from` = P; NIC 4096 capture shows `UDP, length 4096` first fragment and the second fragment at offset 1472; `frag_first`/`frag_shift` counters > 0 | any failure, or a non-shifted offset |
| E-b-matrix | `verdict` delivers all non-empty sizes with no TC, and drops empty with a counter | all non-empty OK; 0 fails; `vstrip_empty_drop` > 0; NIC 0 → no packet; 1431/4096 on the wire | empty delivered, or a non-empty failure |
| E-bn-assoc | a zero-length redirect kills the association | same-association sequence 0,1,1431,…: 0 fails and **every** later size fails | later sizes succeed |
| E-h-matrix | `hybrid` delivers every size including empty | all 12 OK; NIC 0 → `length 0`; `hybrid_empty_framed` > 0, `tc_lenient_pass` > 0 | any failure |
| E-framelike | application payloads that are themselves valid frames (8-byte empty frame, 13-byte frame around `hello`) arrive byte-exact for every option | all OK for `tc`, `tc-frag`, `verdict`, `hybrid` (hybrid: `hybrid_escaped` > 0) | any corruption |
| E-unrelated | a host process (not the owner) sending a frame-shaped datagram through a TC-equipped interface is untouched | 13 B sent, 13 B echoed, `intact` | stripped or dropped |
| E-missing-tc | an egress interface **without** the program (TC on the NIC only, traffic to P via `v11h`) | `tc-frag`: every non-empty reply is 8 B too long (frame leaked to the peer); `hybrid`: non-empty OK, only empty arrives as 8 B; `verdict`: non-empty OK | other outcome |
| E-first | first datagram under repetition, fresh socket per iteration | `tc-frag` and `hybrid`: 1,000 unconnected + 500 connected (64 B), 300 empty, 200 × 4,097 B, 50 × 59,000 B — all OK; `verdict`: 64 B all OK, empty 0/30 | any failure where OK is predicted |
| E-dns | real Internet DNS through the NIC | `dig @1.1.1.1 example.com` answers under every option | failure |
| E-cost | TC cost on traffic that is not registered | `ns_per_run` a few hundred ns or less (`bpf_stats_enabled`, which adds its own overhead) | > 1 µs |
| E-kp | no payload in either owner (strace, `tc-frag` and `hybrid` windows) | only 8/16-byte socket I/O in both owners during the matrix and stresses | any payload-sized socket I/O |
| E-vip-tcp | `connect4` on the owner's cgroup resolves guest TCP to a VIP | `tcp-client` to `VIP:7200` (100 kB), `VIP:7201` (server speaks first) and `VIP2:7000` (host-local backend) OK; guest `getpeername` = VIP; server sees the host's address; 300-iteration stress OK | failure, or the guest sees the backend address |
| E-vip-udp | the same for UDP, reply source = VIP | all 12 matrix cells to `VIP:7210` OK with `from` = `VIP:7210`; host log `peer` = backend; 300-iteration first-datagram stress OK | `from` ≠ VIP, or a failure |
| E-vip-neg1 | without the map the VIP is unreachable | TCP fails, UDP no reply | success |
| E-vip-neg2 | registering the egress tuple from the requested destination (VIP) misses the rewritten packet | `tc-frag --tuple-from requested`: replies are 8 B too long (frame reached the backend) | correct replies |
| E-14-refuse | before listen, a D23 port refuses; an always-bound intake accepts then resets | `refused`; always-bound: connect OK then `ECONNRESET` within a few ms | D23 port accepts |
| E-14-lag | mirror lag for listen/close (host-observed, from the app's syscall return to the first matching host connect result) | 200 cycles: no missing transitions; p50 < 2 ms, p99 < 5 ms in both directions | missing transitions, or p99 > 20 ms |
| E-14-probe | a TCP readiness probe follows the application | 50 ms probe timeline: refused while down, ok while up, transitions within one sample of `L`/`C` | ok while down, or refused while up for more than one sample |
| E-14-exit | app killed (`-9`) or terminated | listener appears after start, e2e request OK, refused after the kill | listener survives the app |
| E-14-churn | rapid churn converges | 2,000 listen/close then final open → listener present and serving; final closed → refused; guest audit changes = 0 | wrong final state |
| E-14-reuseport | a steady `SO_REUSEPORT` listener plus a churning second one: the host listener never flaps | 0 host closes and 0 probe refusals during the window; refused after the steady one exits | any flap |
| E-14-restart | guest-owner restart re-syncs; host-owner restart re-syncs | owner killed → refused; restarted with the app still listening → listener back and e2e OK; app stopped while the owner was down → stays refused; host owner restarted → guest re-opens the session and the listener is back | stale state either way |
| E-14-kp | the listen path reads no payload | guest-owner strace during 20 cycles: control writes 8 B; netlink reads only; no payload-sized reads on paired sockets | otherwise |

---

# Increment-e outcome (appended after execution)

- **V-11 and VIP: every pre-registered prediction held except E-first** (details in the
  final results below).
- **E-first falsified for the traced runs.** `first_64_unconn_1000` was 996/1000 for both
  `tc-frag` and `hybrid`, failing at the same indices, 60–63, in both runs. Both owners were
  under `strace` and the run was about 100× slower. The untraced `tc` run was 1000/1000.
  The FIFO slot order maps indices 60–63 onto the four slots last used by the
  fire-and-forget wire-probe senders (`udp-send`, which exits about 43 ms after its first
  datagram). Under strace the host paired those associations after 98–132 ms
  (`host_dg_installed … waited_ms`). Hypothesis: a UDP slot whose application releases it
  before `Paired` returns to the pool with frames still parked in its cell. This is the
  guest-capture spike's mechanism, not something added here. Tested in increment-f.
- **V-14 in increment-e is invalid (environment).**
  - The guest owner made 282 netlink calls and wrote no control message, so no listener
    ever appeared (200/200 "up" missing).
  - The guest initramfs carries no `inet_diag`/`tcp_diag` modules and has no module
    loader, so every `SOCK_DIAG_BY_FAMILY` dump returned `NLMSG_ERROR`, which the owner
    treated as an empty result.
  - Increment-d, which read `/proc/net/tcp`, worked.
  - Fix: add `tcp_diag` + `inet_diag` to the image, and surface `NLMSG_ERROR` as an error.
    Design implication: the observer depends on `CONFIG_INET_DIAG`/`CONFIG_INET_TCP_DIAG`
    in the guest kernel (modules on stock Ubuntu).

## Pre-registered probes for increment-f

| ID | Hypothesis | Predicted | Falsification |
|---|---|---|---|
| F-slot-control | without a pairing delay, fire-and-forget senders do not poison slots | `hybrid`, untraced: 8 × `udp-send` (777 B ×3, exits after ~60 ms) to TEST-NET, then 200 first-datagram exchanges to P: 200/200 | any failure |
| F-slot-fault | if the host pairs later than the sender lives, the slot is recycled with parked frames and a later association on it fails | same, with `--pair-delay-ms 300` for TEST-NET only: ≥ 1 failure among the 200; guest `ss` shows non-zero Recv-Q on loopback cell sockets after the senders exit; guest owner log shows an association created for a slot after its application released it | 200/200, no residual Recv-Q |
| F-slot-leak | (exploration) parked frames from a released slot reach the next application's destination | netns echo log shows a 777-byte datagram, or it does not; reported either way | — |
| F-first-untraced | the E-first failures were the slot hazard, not forwarding | untraced `tc-frag` and `hybrid`: 1000 unconnected + 500 connected 64 B, 300 empty: all OK | any failure |
| F-14 | the E-14 rows of increment-e's pre-registration, re-run unchanged with diag modules present | as pre-registered in E-14-* | as pre-registered |

# Increment-f outcome (appended after execution)

- **F-slot confirmed.**
  - Fault run: 12/200 failed. After the senders exited, four guest loopback cells held
    `Recv-Q 2355` (3 × 785-byte frames each). The guest owner re-associated each released
    slot with the departed application's port and destination
    (`udp_associate … "queued_on_trigger":2355`).
  - Control: 200/200, no residue.
  - F-slot-leak: no 777-byte datagram reached the netns peer. The parked frames were sent
    to the old destination by the stale re-association, not to the next application's
    destination.
- **F-first-untraced:** `tc-frag` and `hybrid` passed every stress (1000 + 500 + 300 + 200
  + 50), so the increment-e failures were the slot hazard.
- **V-14 (F-14):**
  - Every functional row matched its E-14 prediction: refuse before listen, probe timeline,
    kill/TERM, churn, `SO_REUSEPORT` (0 host closes, 255/255 probes OK), and owner restarts
    on both sides.
  - **E-14-lag falsified.** Untraced p50 ≈ 99 ms in both directions, which equals the
    100 ms half-cycle. During the first 20 cycles under strace, p50 ≈ 9 ms.
  - The host log shows bind/close alternating at a steady 100 ms. The host was
    consistently **one transition behind**: it bound when the app closed and closed when
    it re-listened.
  - Hypothesis: the `sock_diag` dump taken immediately after the `fexit` wake returns the
    pre-change state. Under strace the dump runs late enough to see the change.

## Pre-registered probes for increment-g

| ID | Hypothesis | Predicted | Falsification |
|---|---|---|---|
| G-race | in level mode, a dump at wake often disagrees with a dump 2 ms later | 100 cycles: `level_dump_stale` > 0 (most transitions), host-observed lag ≈ one half-cycle | `level_dump_stale` = 0 |
| G-event-lag | the event-sourced mode (state from the event itself; one dump only at session open, or after a reported loss) mirrors within a small bound | 200 untraced cycles: 0 missing; p99 < 5 ms both directions; guest `wake_lag_us` p99 < 1 ms | missing transitions or p99 > 20 ms |
| G-event-functional | the E-14 functional rows hold in event mode, including restart re-sync seeded from the dump's cookies | as E-14-* | as E-14-* |
| G-layout | the `sock_common` offsets the event reads (rcv_saddr @4, num @14, family @16) match this kernel | BTF dump shows those offsets | any mismatch |

# Increment-g outcome (appended after execution)

- **G-layout confirmed.** This kernel's BTF has `skc_rcv_saddr` at 4, `skc_num` at 14 and
  `skc_family` at 16.
- **G-race falsified.** Level mode, instrumented with a dump at wake and a second dump
  2 ms later: 197/197 consistent, and the decision was correct. Lag p50 2.5 ms, which is
  the instrumentation's 2 ms plus overhead. Increment-f's one-transition-late behaviour is
  therefore **not explained**. It ran level mode without the 2 ms pause, and its first 20
  cycles ran under strace.
- **Event mode: lag met.** 180 untraced cycles: up p50 0.23 / p99 1.40 ms; down p50 0.19 /
  p99 1.39 ms. 0 missing. Restart re-sync, `SO_REUSEPORT` (0 closes, 248/248 probes OK),
  kill/TERM and probe timeline all held.
- **Event mode: churn failed.** After 2,000 rapid listen/close cycles ending **closed**,
  the host listener was still bound 0.5 s later (`churn_closed_probe_after: ok`). Each
  cycle emits two records (~2 × 40 B with the ring header), so 4,000 records overflowed
  the 64 KiB ring and stop events were lost. The 1 s audit had not yet re-seeded.

## Pre-registered probes for increment-h

New mode **map**: the `fexit` programs maintain the kernel hash map `LISTENERS`
(`cookie → port`, reachable listeners only), and the ring buffer is only a wake hint. The
owner reads the map on wake and on the 1 s audit. At start it seeds the map in two phases
(insert the dump's cookies with `BPF_NOEXIST`, then remove seeded cookies absent from a
second dump).

| ID | Hypothesis | Predicted | Falsification |
|---|---|---|---|
| H-level-light | level mode decides from a stale dump when nothing delays it (f's anomaly) | 100 untraced cycles: lag ≈ half-cycle; `level_decision_disagrees` > 0 | lag p99 < 5 ms and 0 disagreements |
| H-level-after-strace | if H-level-light is clean, f's anomaly follows strace detaching | 20 traced cycles, then 100 untraced: untraced lag ≈ half-cycle | untraced lag p99 < 5 ms |
| H-event-churn | (negative control) event mode loses state under churn | final-closed churn: probe at 0.5 s `ok` (stale), ring-loss counter > 0; probe at 1.5 s (after the audit) `refused` | `refused` at 0.5 s |
| H-map | map mode keeps the state exact under churn and mirrors within a small bound | the E-14 rows as pre-registered, plus: 200 untraced cycles 0 missing, p99 < 5 ms; churn final open → `ok` and final closed → `refused` at 0.5 s even when ring-loss counter > 0; seeding: owner restarted while the app listens → listener back | stale state after churn, or any E-14 row fails |

# Increment-h outcome (appended after execution)

- **H-level-light falsified.** 100 untraced cycles in level mode with no instrumentation
  delay: up p50 1.39 / p99 1.46 ms, down p50 1.38 / p99 1.43 ms; 196 decisions agree with
  the event, 0 disagree.
- **H-level-after-strace falsified.** 20 cycles under strace, then 100 untraced: untraced
  p99 1.43 ms.
- Increment-f's one-transition-late behaviour did not reproduce in 300 later level-mode
  cycles and **remains unexplained**. It is recorded as an anomaly, not as a mechanism
  property.
- **H-event-churn falsified on this run.** The probe was `refused` at 0.5 s, although the
  ring dropped 610 records. Increment-g showed the stale outcome on the same probe. Event
  mode is therefore intermittently wrong under churn.
- **H-map: every prediction held.**
  - 180 untraced cycles: up p50 0.23 / p99 1.39 ms; down p50 0.19 / p99 1.27 ms; 0 missing.
    The ~1.4 ms tail is the host harness's 1 ms poll granularity.
  - Churn 4,000 + 4,000 with **8,828 ring records lost**: final open → `ok`; final closed →
    `refused` at 0.5 s.
  - Every E-14 row held (below).

---

# Results

## Verdicts

| Question | Verdict |
|---|---|
| **V-11** off-host guest UDP | **WORKS** with D5a option **a2** (`tc-frag`) or **hybrid**. Option **a as ADR-0149 writes it does not work** above the egress MTU. Option **b** works for every size except empty. |
| **D5a** | **Recommend hybrid** (verdict strip + escape rule + fragment-aware tuple-gated egress TC for kept frames). **User decision** (below). |
| **VIP** (guest UDP and TCP) | **WORKS**: the existing ADR-0053 `connect4` rewrite, applied to the guest-flow owner's own host sockets. The guest sees the VIP in `getpeername` and `recvfrom`. |
| **V-14 / D23** | **WORKS** with the kernel-map design: `fexit` on `inet_csk_listen_start`/`inet_csk_listen_stop` maintains a `cookie → port` BPF map; ring-buffer wake hint; 8-byte `ListenState`. |
| Found on the way | **UDP slot recycling hazard** in the inherited guest mechanism (a slot released before `Paired` keeps parked frames). Guest needs `net.ipv4.fwmark_reflect=1`. |

## Substrate (as executed)

```
host : Linux em-determined-roentgen 7.0.0-29-generic #29-Ubuntu SMP PREEMPT_DYNAMIC Fri Jul 17 20:52:35 UTC 2026 x86_64   systemd-detect-virt: none
guest: Linux (none) 7.0.0-29-generic #29-Ubuntu SMP PREEMPT_DYNAMIC Fri Jul 17 20:52:35 UTC 2026 x86_64
guest links: lo, dummy0 (10.99.7.2/32) only            guest modules: tcp_diag inet_diag dummy vmw_vsock_virtio_transport …
OVERDRIVE_METAL_LEASE_ACQUIRED (every run)             NIC: eno12399np0, MTU 1500, tx-udp-segmentation off [fixed]
```

Off-host stand-ins: netns `v11p` behind veth `v11h` (MTU 1500), and the physical NIC
(`dig @1.1.1.1` and TEST-NET wire captures). Every run restored the host (`cleanup` in each
`results.json`):

```
{"bpf_prog_count":[17,17],"bpf_stats_enabled":["0","0"],"links_unchanged":true,"lo_tcx_unchanged":true,"modules_restored":true,
 "netns_unchanged":true,"root_cgroup_bpf_unchanged":true,"routes_unchanged":true,"spike_cgroups_left":[],"tcx_unchanged":true}
```

## V-11 / D5a: per-probe (increment-e unless noted)

| ID | Predicted | Actual | Verdict |
|---|---|---|---|
| E-a-matrix (`tc`) | 0/1/1431 OK, ≥4096 fail, NIC 4096 nothing | exactly that, connected and unconnected; NIC 4096: `0 packets captured`; `tc_err` 13 | as predicted (**a fails above MTU**) |
| E-a2-matrix (`tc-frag`) | all 12 OK; first fragment `length 4096`, next offset 1472 | all 12 OK, `from`=P; NIC: `offset 0 … length 1492 / UDP, length 4096`, `offset 1472`, `offset 2952`; `frag_first` 262, `frag_shift` 2485 | **WORKS** |
| E-b-matrix (`verdict`) | non-empty OK, 0 dropped+counted | non-empty 10/10 OK; 0 fails both ways; `vstrip_empty_drop` 35; NIC 0: `0 packets captured` | as predicted |
| E-bn-assoc (`verdict-naive`) | one empty kills the association | same association 0,1,1431,4096,4097,59000: **all 6 fail** | as predicted |
| E-h-matrix (`hybrid`) | all 12 OK, NIC 0 `length 0` | all 12 OK; NIC `UDP, length 0` (IP length 28); `hybrid_empty_framed` 305, `tc_lenient_pass` 1767 | **WORKS** |
| E-framelike | frame-shaped payloads byte-exact | 8 B and 13 B exact for **every** option; hybrid `hybrid_escaped` 4 | as predicted |
| E-unrelated | non-owner frame-shaped datagram untouched | `{"sent":13,"got":13,"intact":true}` for every option | as predicted |
| E-missing-tc | `tc-frag` +8 B on every reply; hybrid only empty wrong; verdict non-empty OK | `tc-frag`: 0→8, 1→9, 1431→1439 B; hybrid: 1/1431 OK, 0→8 B; verdict: 1/1431 OK, 0 dropped | as predicted |
| E-first | all OK | `tc` (untraced): 1000/1000, 500/500, 300/300. `tc-frag` and `hybrid` under strace: **996/1000** (indices 60–63), the rest OK. **f, untraced**: `tc-frag` and `hybrid` 1000/1000, 500/500, 300/300 empty, 200/200 × 4097 B, 50/50 × 59000 B | forwarding **WORKS**; the 4 failures are the slot hazard (below) |
| E-dns | `dig @1.1.1.1` answers | `104.20.23.154 / 172.66.147.243 rc=0` under every option | as predicted |
| E-cost | a few hundred ns/run | 20,000 unregistered datagrams through `v11h`: `tc` 306 ns, `tc-frag` 271 ns, `hybrid` 272 ns per run (with `bpf_stats_enabled`'s own overhead included) | as predicted |
| E-kp | only control-sized I/O | `tc-frag` and `hybrid` windows (matrix + all stresses), both owners: 6,216 socket calls each, sizes **{8, 16}** only; guest netlink is metadata (72 B requests) | **WORKS** |

Wire evidence (`pcap-tc-frag-nic-4096.txt`; the three fragments sum to 8 + 4,096 bytes,
and each later offset has moved back by 8):
```
IP (… id 4382, offset 0, flags [+], proto UDP (17), length 1492)  <metal-address>.47093 > 192.0.2.77.9: UDP, length 4096
IP (… id 4382, offset 1472, flags [+], proto UDP (17), length 1500)
IP (… id 4382, offset 2952, flags [none], proto UDP (17), length 1172)
```
`pcap-tc-nic-4096.txt`: `0 packets captured`. The first fragment is `TC_ACT_SHOT`, and
`ip_do_fragment()` stops sending the rest when one transmit fails, so nothing leaves the
host.

### D5a recommendation and evidence

| | empty | ≤ MTU | > MTU (fragmented) | frame-shaped app payload | egress interface lacking the program | needs egress TC |
|---|---|---|---|---|---|---|
| a `tc` (ADR-0149 text) | ✓ | ✓ | **✗ whole datagram lost** | ✓ | frame leaks (+8 B) | every egress interface |
| a2 `tc-frag` | ✓ | ✓ | ✓ | ✓ | **every** datagram +8 B (silent corruption) | every egress interface + fragment map |
| b `verdict` | **✗ dropped, counted** | ✓ | ✓ | ✓ | unaffected | none |
| hybrid | ✓ | ✓ | ✓ | ✓ (escape) | only empty (and frame-shaped) arrive as 8 B | every egress interface, touching only kept frames |

**Recommendation: hybrid**, because it is the only option that preserves empty datagrams
(D15) and still degrades safely. The verdict removes the frame from every non-empty
datagram, so non-empty traffic is independent of routing, interface set and MTU. The egress
TC strips only frames the verdict deliberately kept: empty datagrams, and payloads that
themselves parse as a frame. On a hybrid tuple it passes everything else.
- **Rule:** the verdict keeps the frame iff the datagram is empty or the payload is itself
  a valid frame; on that tuple the TC strips any valid frame and passes the rest. Every
  frame on the wire is then one the verdict kept.
- **If the user prefers no egress TC at all:** choose **b**, and accept that empty
  datagrams to off-host and host-local destinations are dropped and counted.
- **Not a2:** its failure mode (a missing or late attachment) silently corrupts every
  off-host datagram.
- **Not a as written.**

## VIP: per-probe (increment-e, owner `--unframe hybrid`)

| ID | Actual | Verdict |
|---|---|---|
| E-vip-tcp | 100 kB to `VIP:7200` OK, `peer` = `10.251.0.10:7200`, `server_saw_peer` = `10.250.95.1:53790` (host egress address); `VIP:7201` banner `BANNER peer=10.250.95.1:50574 local=10.250.95.2:7201`; host-local backend `VIP2:7000` OK in 1.3 ms; `sockname` `getpeername` = `10.251.0.10:7200`; 300/300 stress | **WORKS** |
| E-vip-udp | all 12 cells to `VIP:7210` OK, every `from` = `10.251.0.10:7210`; host-local `VIP2:7010` 0/1431/59000 OK, `from` = VIP2; 300/300 first-datagram stress; host log `"dst":"10.251.0.10:7210" … "peer":"10.250.95.2:7210"`; `vip_hits` 547 | **WORKS** |
| E-vip-neg1 (no map) | TCP: `Connection reset by peer` (the host connect to the VIP timed out → refused); UDP: no reply | as predicted |
| E-vip-neg2 (`--tuple-from requested`, `tc-frag`) | replies 8 / 9 / 1,439 B for 0 / 1 / 1,431: the frame reached the backend | as predicted |

**VIP mechanism.**
1. The guest captures the VIP like any other destination and records it as the original
   destination.
2. The host owner always `connect()`s its host socket for the flow, TCP and UDP alike. The
   ADR-0053 `connect4` program on the owner's cgroup therefore rewrites VIP → backend in
   the kernel: one map lookup, no new program type, no guest-side map.
3. Replies come back to the connected host socket from the backend. The guest's existing
   `recvmsg4`/`getpeername4` restore the recorded original destination, so the application
   sees the VIP.
4. Production attaches `cgroup_connect4_service` to `overdrive.slice`, which is an ancestor
   of the control plane. The owner's sockets are therefore rewritten whenever the owner
   runs inside that subtree.
5. Not tested: the spike attached the mirror to the owner's own cgroup, not to an ancestor.

## V-14 / D23: per-probe (increment-h, map mode; f and g in brackets)

| ID | Actual | Verdict |
|---|---|---|
| E-14-refuse | D23 port before listen: `refused`; always-bound intake: connect OK, then `ConnectionResetError(104)` after 1.1 ms | as predicted |
| E-14-lag | 180 untraced cycles: up p50 0.23 / p99 1.39 / max 1.40 ms; down p50 0.19 / p99 1.27 / max 1.28 ms; 0 missing. Guest `wake_lag_us` (fexit ktime → owner wake), 360 untraced events: p50 48 / p99 117 / max 202 µs. 20 cycles under strace: p50 ≈ 9–10 ms | **WORKS** (bound ≈ 1.5 ms including the 1 ms poll) |
| E-14-probe | 50 ms timeline: `refused` from 0 s, `ok` from 1.006 s (app `L` at 1.002 s), `refused` from 2.013 s (`C` at 2.003 s), `ok` from 3.018 s (`L` 3.003 s), `refused` from 4.025 s (`C` 4.004 s); each transition at the first sample after the syscall | as predicted |
| E-14-exit | `kill -9` / `kill -TERM`: listener up in 1.4 / 0.2 ms; e2e 1,000-byte request OK; refused within 0.15 ms | as predicted |
| E-14-churn | 2,000 then final open: `HELLO`; final closed: `refused`. 4,000 + 4,000 with 8,828 ring records lost: same | as predicted (map); event mode stale in g |
| E-14-reuseport | 249/249 probes OK during 2,000 churns of a second `SO_REUSEPORT` listener; 0 host closes in the window; refused 0.1 ms after the steady listener exited | as predicted |
| E-14-restart | guest owner killed → refused in 9 ms; restarted with the app listening → listener back (`map_seeded dump1:3 seeded:3`), e2e `HELLO`. App stopped while the owner was down → stays refused (all ports sent `listening:false` at session open). Host owner restarted → guest re-opened the session after two 250 ms retries, listener back, e2e `HELLO` | as predicted |
| E-14-kp | guest owner strace over 20 cycles: socket I/O sizes {8, 16, 24}. The 24 B comes from the probes' inbound flows (16 B request + 8 B control), not from the listen path | as predicted |

**V-14 mechanism.**
1. In the guest kernel, `fexit(inet_csk_listen_start)` (return 0) and
   `fexit(inet_csk_listen_stop)` read `skc_rcv_saddr`, `skc_num` and `skc_family` (BTF
   offsets 4/14/16 on this kernel) and the socket cookie.
2. They insert or delete `cookie → port` in a BPF hash map for listeners reachable at the
   workload address (`0.0.0.0`, W, or IPv6), and push a ring-buffer wake hint.
3. The guest owner reads the map on wake, sends an 8-byte `ListenState` only on change,
   sends the full state when the control session opens, and runs a 1 s audit.
4. The map is seeded once per owner process from two `NETLINK_SOCK_DIAG` `TCPF_LISTEN`
   dumps (insert with `BPF_NOEXIST`, then delete seeded cookies missing from the second
   dump), so listeners created before the owner started are included.
5. The host owner binds the intake listener on `listening`, closes it on `not listening`,
   and closes all of them on control-session loss.
6. A lost ring record cannot lose state.

Rejected variants (evidence):
- **Event-sourced state** (state carried in the ring records): stale after churn (g),
  because the ring overflows.
- **`/proc/net/tcp` re-derivation**: 5–11 ms per wake because the owner's own pooled
  sockets bloat the table.
- **`sock_diag` re-derivation per wake** ("level"): correct in h, but it needs a dump per
  wake, and increment-f showed an unexplained one-transition lag with it.

## Slot recycling hazard (found here; affects the inherited guest UDP mechanism)

Increment-f, `hybrid`, owner fault injection `--pair-delay-ms 300` for TEST-NET only, 8
fire-and-forget senders (3 × 777 B each; they exit ~60 ms after the first datagram):
```
ESTAB 2355   0   127.0.0.1:40559    127.0.0.1:32920        # 4 parking cells still hold 3 frames each
{"app":"10.99.7.2:37108","dst":"192.0.2.77:9","ev":"udp_associate","flow":1,…,"slot":0}
{"ev":"udp_teardown","flow":1,"slot":0,"why":"app_socket_released"}
{"app":"10.99.7.2:37108","dst":"192.0.2.77:9","ev":"udp_associate","flow":3,"queued_on_trigger":2355,"slot":0}   # re-associated after release
stress 200 → ok 188, failed 12 (indices 0, 56–64, 128, 192)       control (no delay): 200/200, no residue
```
- **What happens:** a slot whose application releases it before `Paired` arrives goes
  back to the free pool with frames parked in its cell. The owner then re-associates it to
  the departed application's destination, and later first datagrams that land on such
  slots fail.
- **No cross-destination leak was observed:** 0 × 777-byte datagrams reached the netns
  peer.
- **Normal timing:** not hit, because pairing takes under 1 ms (thousands of untraced
  first datagrams passed). An owner slower than an application's lifetime triggers it.
- **Fix needed in the design:** discard or recreate the cell on release, and never
  re-associate a released slot.

## Design implications

1. **ADR-0149 / D5a** — **user decision**. Option (a) as written cannot carry datagrams
   above the egress MTU, because fragmentation happens before TC egress.
   - Recommended contract: hybrid, i.e.:
     - verdict strip with the escape rule;
     - a lenient tuple mode;
     - fragment-aware egress TC on every host egress interface (for kept frames), plus `lo`.
   - Fallback: (b), with empty off-host datagrams dropped and counted.
   - Either way, tuples are registered from the **kernel peer** (`getpeername`) of the
     connected host socket, never from the requested destination: E-vip-neg2 shows the
     frame leaking to the VIP's backend otherwise.
2. **VIP** — no new mechanism is needed. ADR-0053's `connect4` covers guest flows,
   **provided the guest-flow owner's sockets are inside the attach subtree**. The feature
   delta's row "ADR-0053 … Owner sockets live outside the workload subtree and are not
   rewritten (0162)" is misleading:
   - production attaches at `overdrive.slice`, which contains the control plane;
   - guest VIP access **depends** on the owner's sockets being rewritten.
   **User decision:** confirm that the owner runs under the `connect4` attach point. This
   also covers ADR-0053's `sendmsg4`/`recvmsg4`: the owner never needs them, because it
   always connects.
3. **ADR-0163 / D23**:
   - Use the kernel-map design.
   - Pin the lag bound at **≤ 2 ms** (measured ≤ 1.4 ms including harness granularity).
   - Requirements on the guest image:
     - `fexit` + BTF in the guest kernel;
     - `CONFIG_INET_DIAG` + `CONFIG_INET_TCP_DIAG` (modules on Ubuntu; increment-e failed
       without them);
     - the three `sock_common` offsets (aya has no CO-RE here, so pin them per guest
       kernel or read them through BTF-typed access).
   - The listener mirror must not share a loop with blocking work (increment-c: a blocking
     acceptor `connect()` delayed reports by ~1 s).
4. **Guest inbound refusals** need `net.ipv4.fwmark_reflect=1`. Without it, a connect to
   a non-listening guest port waits for the acceptor's 3 s timeout instead of an immediate
   RST. This changes the D23-fallback ("always bind") behaviour from established-then-reset
   in 1 ms to established-then-hang for 3 s.
5. **Owner I/O**: every owner `connect()` must be non-blocking (a 5 s blocking connect to
   an unmapped VIP delayed unrelated flows), and the owner's fd budget must cover probe
   floods (increment-c `EMFILE` at 1,024).
6. **Guest UDP slots** (ADR-0147/0149 guest side): add slot-release hygiene (above).

## Not proven

- The pinned 6.18 appliance kernel (only 7.0.0-29 was run).
- A real second machine (netns behind veth plus the NIC wire capture were used).
- NICs with UDP segmentation offload.
- Hosts with several egress interfaces, policy routing or interfaces that appear at
  runtime (hybrid's TC attachment set).
- A frame-shaped application payload larger than the MTU under hybrid (the escaped frame
  is fragmented; the fragment-aware TC is designed for it but this was not tested).
- `connect4` inherited from an ancestor cgroup (`overdrive.slice`).
- IPv6 listeners with `IPV6_V6ONLY=1` (treated as reachable).
- Scale beyond one VM.
- Deferred by ruling: IPv6 #308, > 59,000 B #309, inbound UDP #310, ICMP #311.
- Increment-f's level-mode lag anomaly is unexplained (not reproduced in 300 later
  cycles).

## Files

- `spike-scratch/netns-density-295-v11-vip-v14/` (README lists every file and increment).
- Evidence for the verdicts:
  - `increment-e/evidence` (V-11, missing-TC, VIP; pcaps `pcap-*.txt`; straces
    `strace-{host,guest}-owner-{tc-frag,hybrid}.txt`);
  - `increment-f/evidence` (slot hazard, untraced stresses);
  - `increment-g/evidence` (BTF `sock_common`, event mode);
  - `increment-h/evidence` (map mode, level controls).
- Development: `increment-{a,b,c,d}/evidence`.
