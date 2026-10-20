# V-EV-PAIR findings: attach at pairing loses no early data (WORKS, with 3 contract corrections and 1 module bug fixed)

## 1. Verdict

**WORKS for the contract as proposed, with 3 corrections to its text and 1 module bug fixed during the spike.** Attaching at pairing loses no early data on the P-22/P-23 shapes. Early bytes and early ends waited in the source socket's own receive queue, and the attach-time run forwarded them.

- **Final-build tally:** 961,995 of 962,000 trials passed. The 5 failures were harness `connect()` resets before any attach.
- **Byte conservation:** every pair's end record matched the exact byte counts in both directions. Checksums were verified at both endpoints.

The 3 corrections change contract text that DISTILL pins. Under `.claude/rules/design.md` they need user approval before they are treated as accepted.

## 2. Hypothesis / prediction / falsification

- **Hypothesis:** with attach at pairing (host and guest TcpAccept before `Paired`; guest TcpConnect and datagram on `Paired`), no early byte or early end is lost, duplicated or stranded.
- **Prediction:** every trial is byte-exact in both directions, every early end is carried out, and a reset before `Paired` aborts the flow instead of ending clean. The source queue is the only place early bytes wait.
- **Falsified by:** any lost, duplicated or stranded byte or end, or an early reset that ends clean.

## 3. Substrate and artifacts

- **Host:** metal, `uname -r` = **7.0.0-38-generic**, x86_64, AMD EPYC 8024P, `systemd-detect-virt` = none, `/dev/kvm` API 12.
- **Guest:** Cloud Hypervisor fork (sha `eadddf1bb…`) with `--vsock backend=vhost-kernel`, guest `uname -r` = **7.0.0-29-generic**, 4 vCPU.
- **Module placement:** the module loaded only inside the guest. `physical_module_names_unchanged = true` and `physical_ovdpair_loaded = false` on every run.
- **Lease use:**
  - The lease was taken per job and released at job end.
  - The work dir was `/var/tmp/ev-pair-work`, with no rsync `--delete`.
  - My first in-cfearly run hung and held the lease 25 minutes. That was my mistake: I had no watchdog. The driver now kills a VM that produces no serial output for a set time, and the probe dumps its state and exits after 45 s with no trial completing.
- **Module:** `ovdpair.ko` sha256 `8c68891644ae…`, identical across F1, F2 and F3.
- **Provenance:** probe `3fb76c630b97` ran F1 and F2. `dcf70dafd013` ran F3. They differ only in the `eg-drst` client and verdict code.
- **Deliberately not done:** the module sets no `rmmod` path, and I did not touch `crates/`, `specs/` or `docs/`.

**Module (`module/ovdpair.c`, derived from `evmod.c`).** It implements the PROPOSED control shape as far as this question needs:
- `/dev/ovd_fwd` with one context per `open()`.
- `release()` aborts every live pair before returning.
- u64 ids, never reused.
- A 32-byte end record via `read()` and `poll()`.
- No `module_exit`.
- The probe confirms `delete_module` returns `EBUSY` (16): `{"abi":1,"permanent_flag":1,"delete_module_rc":-1,"delete_module_errno":16,"record_size":32}`.
- Per-vsock-half end classification (EVMOD-END).
- A TCP half is accepted in `ESTABLISHED` or `CLOSE_WAIT`. A reset or closed half is refused with its errno.
- Attach reports `early_bytes` and `early_flags` per half, so every run proves whether data and ends were really queued at attach time.

**Probe (`probe/pairprobe.c`).**
- **L mode** runs the full chain in one guest kernel: guest pair, host pair, and vsock loopback CID 1 between them.
- **G mode** runs the guest pair over the real vhost-vsock device against a plain userspace host peer.
- **Jitter** is injected at 6 points: client, server, guest-owner ×2, host-owner ×2. Classes are 0 / 20 µs / 200 µs / 2 ms / 10 ms with weights 30/25/25/17/3 %.
- **Payload size mix:** 0 B (5 %), 1–64 B (40 %), 65–8 KB (40 %), 8–256 KB (13.5 %), up to 1.46 MB (1.5 %).
- **Checks:** FNV-1a 64 of the received stream plus a `memcmp` against the regenerated stream, per direction.
- **D20:** owner TCP sockets get `SO_LINGER{1,0}`, cleared on a completed pair.

## 4. Shapes

| shape | P-22/P-23 case | what is early at attach |
|---|---|---|
| `eg-wf` | egress write + FIN before `Paired` | client bytes and its FIN on the intake child (the guest TCP socket the application connected to) |
| `eg-sf` | egress server-first | destination banner in the vsock half (guest) and in D (host) |
| `eg-sffin` | server-first plus early FIN | banner plus a SHUTDOWN already received on the vsock half |
| `eg-nofin` | egress, no FIN | request bytes only |
| `eg-rst` | pending intake child resets before `Paired` | RST |
| `eg-drst` | destination resets before `Paired` | RST/SHUTDOWN 0x3 on the vsock half |
| `in-sf` / `in-sffin` | inbound server-first (± FIN) | application banner on C |
| `in-cf` | inbound client-first, H-1 order (write after `Paired`) | nothing, by construction |
| `in-cfearly` | inbound client-first, violating H-1 (write + FIN right after the request) | bytes plus a SHUTDOWN 0x2 on V_g |

## 5. Final results

**How to read the table.**
- "Early at attach" is the share of attaches that found bytes or an end already queued, as reported by the module.
- "Any" is bytes or end on either half.
- Detail columns:
  - `bytes A/B`: attaches with early bytes on half A / half B.
  - `END A/B`: attaches with an early end on half A / half B.
- Half A/B for the guest pair: A is the intake child I (egress) or V_g (inbound); B is V_g (egress) or C (inbound).
- L = loopback chain, G = real vhost.

| mode | shape | trials | passed | early at attach (any) | bytes A/B | END A/B |
|---|---|---:|---:|---:|---|---|
| L | eg-wf | 10,000 serial | 10,000 | 90 % | 8525 / 0 | 8151 / 0 |
| L | eg-wf | 8 lanes × 1,250 | 10,000 | 89 % | 8425 / 0 | 8071 / 0 |
| L | eg-wf | 8 × 10,000 | 80,000 | 89 % | 67599 / 0 | 64775 / 0 |
| L | eg-sf | 10,000 / ×8 1,250 / ×8 10,000 | 10,000 / 10,000 / 80,000 | 78 % | 429 / 7343 (serial) | 406 / 0 |
| L | eg-sffin | same three sizes | all pass | 78 % | 0 / 7341 (serial) | 0 / 7413 (serial) |
| L | eg-nofin | 10,000 / ×8 1,250 | all pass | 90 % | 8618 / 357 | 422 / 0 |
| L | eg-rst | 10,000 / ×8 1,250 | all pass | attach refused in ≥ 99 % | – | – |
| L | eg-drst | 10,000 / ×8 1,250 / ×8 10,000 | 9,999 / 10,000 / 79,996 | 98 % | 5784 / 5723 | 0 / 5678 |
| L | in-sf | 10,000 / ×8 1,250 | all pass | 35 % | 0 / 3541 | 0 / 0 |
| L | in-sffin | 10,000 / ×8 1,250 / ×8 10,000 | 10,000 / 10,000 / 80,000 | 37 % | 0 / 3517 | 0 / 3293 |
| L | in-cf | 10,000 / ×8 1,250 / ×8 10,000 | 10,000 / 10,000 / 80,000 | 0 % (H-1 respected) | – | – |
| L | in-cfearly | 10,000 / ×8 1,250 / ×8 10,000 | 10,000 / 10,000 / 80,000 | 61 % | 5773 / 0 | 5745 / 0 |
| G | eg-wf, eg-sf, eg-sffin, eg-nofin, eg-drst | 10,000 each | all pass | 59–97 % | e.g. wf 8526 / 0 | e.g. wf 8163 / 0 |
| G | in-sf, in-sffin, in-cf, in-cfearly | 10,000 each | all pass | 0–66 % | – | – |
| G | eg-rst | 2,000 | 2,000 | – | – | – |
| G | eg-wf, eg-drst, in-cfearly | 8 × 1,250 | 10,000 each | – | – | – |
| G | in-cf | 8 × 10,000 | 80,000 | 0 % | – | – |

- **G host peer:** `peer-summary.json` shows 0 failures for every G run.
- **eg-drst, L:**
  - In the 2 runs that still included the serial one, 5 trials failed in the harness's own `connect()` to the destination, with ECONNRESET, before any attach. The cause is loopback ephemeral-port reuse after the server's RST-closes (inferred).
  - Of the 100,000 trials, 99,995 passed.
  - The attach-time abort branch ran 5,678 times in the serial run.
- **Pair accounting:** attached = completed + aborted = records read, with `ab_unclassified_eof = 0` on every non-hazard run. F1 total was 786,476 pairs: 640,000 completed and 146,476 aborted. No live pair remained.
- **Memory:** F1 SUnreclaim 19.9 → 26.0 MB after 786k pairs. F2 was 19.2 → 26.0 MB after 320k pairs. Both plateaued.
- **Bound on early bytes:** the largest early queue seen was 262,144 B on a vsock half (its 256 KiB receive buffer). On an intake-child TCP half it was about 128 KB.
- **Race fix check:** G `in-cf` 8 × 10,000 produced 0 aborts. The earlier module raced about once per 10,000 in that shape. Zero in 80,000 trials makes a rate of 1 in 10,000 unlikely (about e⁻⁸). The code ordering is fixed, and the hazard below shows the race is reachable.

## 6. Hazard variants (each removes one design rule; controls 500/500 before and after)

Run 1701–1763 in F3.

| hazard | shape | passed / trials | counter evidence | what it proves |
|---|---|---:|---|---|
| `h_strict` (attach needs `ESTABLISHED` only; the evmod spike's check) | eg-wf | 86 / 500 | 414 attaches refused, `attach_rej_state = 414` | a TCP half in `CLOSE_WAIT` must be accepted |
| `h_strict` | in-sffin | 336 / 500 | 164 refused | same, inbound |
| `h_noseed` (no attach-time classification) | eg-sffin | 114 / 500 | `ab_unclassified_eof` +386 | an end received before the hook fired no callback; seeding at attach is required |
| `h_noseed` | in-cfearly | 211 / 500 | `ab_unclassified_eof` +289 | same |
| `h_noseed` | eg-wf (control) | 500 / 500 | – | no vsock early end by construction |
| `h_nokick` (no attach-time run) | eg-wf | 5 / 20 | stranded, client and server time out | TCP early bytes and ends rely on the attach-time run |
| `h_nokick` | in-sf | 13 / 20 | stranded | same |
| `h_nokick` | in-cfearly | 20 / 20 | classification kicks the direction anyway | the vsock early-end case is kicked by its own classification |
| `h_fault` (one byte dropped) | eg-wf | 1 / 200 | `fault_hits` +395 | the probe sees a lost byte |
| `h_fault` | in-cf | 1 / 200 | `fault_hits` +199 | same |
| `h_lateshut` (publish `own_wr_shut` after the SHUTDOWN leaves, window widened to about 1 ms) | in-cf | 30 / 300 | `ab_peer` +270 | CLEAN-FULL-END ordering race |
| `h_lateshut` | in-sf | 0 / 300 | `ab_peer` +300 | same |
| `h_lateshut` | eg-wf | 300 / 300 | – | forwarder peers close only after completion |

The 1 passing `h_fault` trial each is a flow too small to lose a byte (the module skips payloads under 2 bytes).

## 7. Defects found

**Module, fixed.** CLEAN-FULL-END ordering race.
- **Trigger:** one real-vhost G `in-cf` serial trial (1 of 10,000) aborted with `PEER_ABORT` after the payload was delivered byte-exact.
- **Cause:** the forwarder sent SHUTDOWN 0x2, then set `own_wr_shut`. The host dialer read EOF and closed (0x3) in that window. The callback classified "RCV newly set while this side still sends" and aborted.
- **Fix:** record "send direction shut" before the SHUTDOWN leaves. The `h_lateshut` hazard reproduces the old order.

**Probe defects (not module faults), found and fixed.**
- **Paired socketpair fill:** `in-cfearly` never reads `Paired`. About 278 unread 8-byte messages filled the AF_UNIX socketpair buffer, and the guest owner's send blocked. The hang was at trial 278 in every lane, serial and ×8. The kernel counters showed all pairs attached and completed.
- **eg-drst host-attach verdict:** the host attach is legitimately refused when the destination's RST precedes it. The verdict had required an aborted host pair record.
- **eg-drst client verdict:** a client `send` that fails with the reset consumes `sk_err`. The following `recv` then returns 0, which the probe called "reset became EOF". Kernel basis: `net/ipv4/tcp.c:2794-2806 @ v7.0` (`SOCK_DONE` first, then `sk_err` via `sock_error` which clears it, then `RCV_SHUTDOWN`) and `tcp_done` setting `sk_shutdown`.
- **Runlists:** one lost its `P=` line, so the hazard switches never took effect. One was run without `peer=1`, so the G section failed at once. Neither run is cited above.
- **Fail-fast:** a systematically failing hazard run stalled 3 s per trial until the probe sent `Paired` after a refused attach.

## 8. Kernel citations (`vendors/linux` @ v7.0)

- `net/ipv4/tcp_input.c:4947-4960`: `tcp_fin` sets `SOCK_DONE` and moves `ESTABLISHED` to `CLOSE_WAIT`. A write+FIN-before-`Paired` intake child is therefore in `CLOSE_WAIT` at attach.
- `net/ipv4/tcp_input.c:4905-4930`: `tcp_reset` sets `EPIPE` in `CLOSE_WAIT`, otherwise `ECONNRESET`. These are the attach errnos the module returns.
- `net/ipv4/tcp.c:2794-2806`: recv order is `SOCK_DONE` → `sk_err` (via `sock_error`, which clears it) → `RCV_SHUTDOWN`. A reset after a FIN reads as clean EOF; a consumed `sk_err` reads as EOF.
- `net/ipv4/tcp.c:5087-5141`: `tcp_abort`. The module uses it and calls `EXPORT_SYMBOL_GPL`.
- `net/vmw_vsock/virtio_transport_common.c:1443-1461`: SHUTDOWN updates `peer_shutdown`; `sk_state_change` is called only when the flags are non-zero; a full `SHUTDOWN_MASK` with queued data keeps the socket `ESTABLISHED` (attach succeeds and classifies abort).
- `net/vmw_vsock/virtio_transport_common.c:1244-1253`: RST handling (`do_close`).
- `net/vmw_vsock/virtio_transport_common.c:1661-1705`: the receive path holds `lock_sock`. Taking `lock_sock` at attach, installing the hooks and snapshotting `peer_shutdown` in one critical section means no transition is missed or seen twice.
- `net/vmw_vsock/af_vsock.c:1030-1037`: `vsock_stream_has_data` (exported GPL), used for `early_bytes`.
- `kernel/module/main.c:818-826`: `delete_module` returns `EBUSY` when there is no `exit`.
- `net/vmw_vsock/vsock_loopback.c:24-26`: the loopback transport reports `VMADDR_CID_LOCAL`.

## 9. Design implications for the EVMOD contract

**What holds**
- **EVMOD-PAIR:** attach at pairing, early bytes and ends waiting in the source's own queue, and the attach-time run forwarding them. This is validated on all three prescribed shapes plus server-first-with-FIN, no-FIN, early reset, and destination reset.
- **Byte conservation:** the end record's per-direction counters matched the payload exactly.
- **Bounded early data:** early data is bounded by the source's own buffer (vsock 256 KiB, TCP receive buffer).
- **Context semantics:** u64 ids, one 32-byte end record per id, no module exit.

**What must change**
1. **Attach must accept a TCP half in `CLOSE_WAIT`.** The EVMOD-CTL attach contract currently names no accepted states. State that a TCP half is accepted in `ESTABLISHED` and `CLOSE_WAIT`. The evmod spike's check rejects 82–90 % of write-then-FIN flows (hazard `h_strict`).
2. **Seed the end classification at attach.** EVMOD-END says the forwarder "keeps the peer-shutdown bits it has already acted on". That is incomplete. Attach must, under `lock_sock`, install the hooks and classify `peer_shutdown` as if the callback had fired. Without it, `unclassified_eof` aborts every flow whose vsock end arrived early (hazard `h_noseed`: 33–77 % fail). A pre-attach 0x3 (or RST) aborts the pair. That path ran 5,678 times in `eg-drst`.
3. **Pin the attach refusal errnos.** A reset or closed half is refused with its own errno. Observed values:
   - `ECONNRESET` (the half was reset)
   - `EPIPE` (reset while in `CLOSE_WAIT`)
   - `ENOTCONN` (closed with no error)

   In EVMOD-CTL's distinct-errno list these must be distinct from `ABI`/`NOT_PERMANENT` refusals. The owner then aborts the flow by closing both halves under D20. The far side's pair classifies that and aborts.
4. **Order CLEAN-FULL-END.** The forwarder must publish "this side shut its send direction" before the SHUTDOWN leaves. State this in the end-record rules. It is an internal ordering, but END-IDEMPOTENT depends on it.
5. **Attach-time run is load-bearing for TCP halves.** Keep it in EVMOD-PAIR. The vsock early-end case also gets a kick from its own classification, but the TCP-only early data needs the run.
6. **Cautions on EVMOD-END.**
   - "A TCP half needs no classifier" holds only because every error aborts and the forwarder is the only reader of that socket. A consumed `sk_err` reads as EOF.
   - An owner that sends on, or reads from, an attached TCP socket would turn a reset into a clean end.
   - A reset after a FIN reads as clean EOF.

## 10. Residual and not covered

- **Host-role pair on the real vhost transport.** The module cannot load on the physical host, so the host pair is exercised only on vsock loopback (same `af_vsock` / `virtio_transport_common` code, different transport). In egress the guest sends only after `Paired`, so the host vsock half has no early traffic by construction. The inbound host side is a Watch, not a pair. Residual: the host pair's attach on a vhost-accepted socket.
- **eg-drst on G** has the peer play the forwarder, so it cannot see a vsock reset as distinct from a close (P-49).
- **Not tested:** datagram mode (V-EV-DGRAM), kTLS halves, `GUEST_TCP_PENDING_MAX` exhaustion, and a context release with early data pending.

## 11. Gate recommendation

**PROMOTE EVMOD-PAIR to DISTILL with corrections 1–4 pinned in the feature-delta.** Corrections 1–3 change the EVMOD-CTL/EVMOD-PAIR interface text, so they need user approval first. No contract-shaping item remains open for this id. G-mode on the real vhost device was clean on all shapes.

## 12. Evidence

All under `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-ev-pair/`:
- `module/ovdpair.c`, `module/ovd_fwd_uapi.h`, `probe/pairprobe.c`.
- `driver/vmrun.py`, `driver/build.sh`, `launch.py`, `retry.sh`, `bg.sh`, `sync.sh`.
- `runlists/F1-L-egress.sh`, `runlists/F2-L-inbound.sh`, `runlists/F3-hazards-G.sh`.
- `analyze.py` produces the per-run table from the evidence directories.
- `out/REPORT-progress.md` is the progress file.
- Final evidence: `evidence/vm-F1-20261017T020433Z`, `evidence/vm-F2-20261017T021621Z`, `evidence/vm-F3-20261017T022218Z`. Each holds `run-results.jsonl`, `result.json`, `artifact-pins.json`, the executed sources, `run.log`, and `vm/serial.log`.
- Superseded and invalid runs are kept as evidence but not cited: `vm-smoke-*`, `vm-job1-*`, `vm-job2-*`, `vm-job2b-*`, `vm-job3-*`, `vm-job5-*`, `vm-job6-*`, `vm-job6b-*`.
