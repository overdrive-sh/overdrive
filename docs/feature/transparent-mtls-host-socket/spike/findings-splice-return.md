# Spike findings — `splice(2)` agent-light return path for kTLS-RX decrypted plaintext (increment H)

> **nw-spike Phase-1 PROBE — THROWAWAY, real-kernel, NOT promoted.** Confirms-or-refutes the
> research finding in `docs/research/dataplane/ktls-rx-agent-light-relay-research.md`
> (verdict (c) PLAUSIBLE-BUT-UNVALIDATED).

**GH**: [#26](https://github.com/overdrive-sh/overdrive/issues/26) (transparent mTLS host socket) /
[#222](https://github.com/overdrive-sh/overdrive/issues/222) (return path).
**Date**: 2026-06-12. **Kernel**: 7.0.0-15-generic, Ubuntu 26.04 LTS, Lima VM `overdrive`, aarch64.
**Scope**: loopback only; software kTLS (`rxconf: sw txconf: sw`); AES-256-GCM TLS 1.3 only;
agent-as-TLS-client; single-record (86 B) **and** multi-record (100 000 B) payloads.
**Probe code** (gitignored, throwaway): `spike-scratch/increment-h-splice-return/`.

---

## VERDICT

**(a) YES — `splice(2)` delivers kTLS-RX-decrypted *plaintext* to leg F agent-light.**

The research's verdict (c) PLAUSIBLE-BUT-UNVALIDATED is **promoted to (a) CONFIRMED**. A
`splice(legB → pipe → legF)` pump, with leg B a **plain kTLS-RX socket (NO sockmap, NO psock)**,
moves the peer's TLS-encrypted application_data to the workload-facing socket as **byte-exact
decrypted plaintext** while the agent issues **only `splice()` + `ppoll()`** — **ZERO
`read`/`recvmsg`/`recvfrom`/`write`/`send` of payload bytes**. No `-EINVAL`. The result is even
*cleaner* than the research predicted: `splice` hands the **decrypted record payload only** (no
TLS header, no inner-type byte, no GCM tag) — settling the increment-g 114-vs-92 framing caveat in
splice's favour.

**#222 is now agent-light in BOTH directions:**
- **Forward (workload-plaintext F → peer-kTLS-TX B)**: **agent-idle** — egress sockmap redirect
  drives `tcp_sendmsg_locked`, agent out of the path (`findings-egress-ktls-splice.md`, 15/15).
- **Return (peer-kTLS-RX-decrypt B → workload-plaintext F)**: **agent-light, zero-copy splice** —
  the agent drives a bounded `splice` pump (~1 splice per TLS record, kernel-paced), **NO per-byte
  userspace copy**. Confirmed here.

So the precise #222 framing — **"agent-idle forward, agent-light zero-copy-splice return"** — holds
empirically. No userspace per-byte copy in either direction.

**Q1 (#26 in-band lossy DROP-RESET gate) does NOT move.** This probe is about the #222 two-socket
proxy's return leg only.

---

## What was tested (and how it differs from increment-g)

The prior spike (increment-g, `findings-ktls-rx-splice.md`) foreclosed the **sockmap stream-verdict**
return path: the decrypted BPF verdict runs only inside `tls_sw_recvmsg`, and `tls_sw_read_sock`
returns `-EINVAL` when a psock is attached, so nothing decrypts-then-redirects while the agent is
idle. **But increment-g never tested `splice(2)`.** Per kernel source, `tls_sw_splice_read`
(`net/tls/tls_sw.c`) is a *different, unrestricted* path: it decrypts each record
(`tls_rx_one_record`) and splices the plaintext into a pipe (`skb_splice_bits`) with no per-byte
userspace copy and **no `sk_psock`/`-EINVAL` check** (that check lives in `tls_sw_read_sock`, which a
splice-only design never calls).

This probe tests exactly that gap:

| | increment-g (sockmap verdict) | increment-h (THIS probe — splice) |
|---|---|---|
| Leg B sockmap / psock | **YES** (SOCKMAP + `sk_skb` verdict attached) | **NO** — plain kTLS-RX socket only |
| Relay mechanism | in-kernel verdict EGRESS-redirect | userspace `splice(B→pipe→F)` pump |
| Agent posture | agent-light *only when read-driven* (drives `recvmsg`) | agent-light (only `splice`/`ppoll`) |
| Bytes seen on leg F | **114** (RAW pre-decrypt record: 5B hdr + 92B + inner-type + GCM tag) | **86 / 100000** (CLEAN decrypted plaintext, no framing) |
| `-EINVAL` | n/a (verdict path) | **none** (`einval_on_B=0`) |

The probe carries **no `aya`, no BPF object, no sockmap** at all — the dependency surface is
`rustls` + `ktls` + `libc` only. The kTLS-RX arm path (`install_ktls`, the rustls handshake `pump`,
peer/reader roles, SCM_RIGHTS fd-passing) is reused verbatim from increment-g's proven harness.

### Topology

```
  PEER P (rustls + ktls TLS 1.3 server, child proc)
     │  arms kTLS TX, sends SERVER_BANNER as TLS 1.3 application_data (0x17 ciphertext on wire)
     ▼
  LEG B (agent's client socket to P) ── kTLS RX armed (AES-256-GCM TLS1.3), NO sockmap/psock
     │
     │  splice(legB_fd → pipe[1], SPLICE_F_MOVE|SPLICE_F_NONBLOCK)   ← kernel decrypts here
     ▼
  PIPE (pipe2)
     │  splice(pipe[0] → legF_target_fd, SPLICE_F_MOVE|SPLICE_F_NONBLOCK)
     ▼
  LEG F target (agent-owned, splice DESTINATION) ── TX ──▶ F_peer
                                                              │ (handed via SCM_RIGHTS)
                                                              ▼
                                                       READER proc (reads plaintext off F_peer)
```

The agent's per-connection loop after setup is **only** `ppoll(legB, POLLIN)` for readiness +
`splice()` calls. The reader (separate PID) does the `read` off F_peer — outside the agent's strace.

---

## ASSERTION 1 — Agent-light proof (the load-bearing evidence)

`strace -f -tt` on the agent during a single-record transfer. The agent's main PID was 92924; the
two children (PIDs 92926, 92957) are the peer and reader. **fd-number disambiguation was done by
timeline**: fd 3 is reused (process startup names it `/proc/filesystems`, `libc.so.6`, the ELF
loader, then the rustls handshake socket); the leg-B kTLS *socket* is fd 3 **only in the transfer
window**, which begins at the first `splice()` call. Everything the agent did **at/after the
transfer-window start** (`03:12:06.254572`), excluding stderr logging:

```
92924 03:12:06.254572 splice(3, NULL, 9, NULL, 65536, SPLICE_F_MOVE|SPLICE_F_NONBLOCK <unfinished ...>
92924 03:12:06.255444 <... splice resumed>) = 86
92924 03:12:06.258523 splice(7, NULL, 6, NULL, 86, SPLICE_F_MOVE|SPLICE_F_NONBLOCK) = 86
92924 03:12:06.262685 ppoll([{fd=3, events=POLLIN}], 1, {tv_sec=0, tv_nsec=20000000}, NULL, 0) = 0 (Timeout)
92924 03:12:06.290736 ppoll([{fd=3, events=POLLIN}], 1, ...) = 0 (Timeout)
   … (8 more ppoll readiness timeouts) …
92924 03:12:06.574464 --- SIGCHLD {si_pid=92957, si_status=0} ---  (reader exited)
92924 03:12:09.767222 --- SIGCHLD {si_pid=92926, si_status=0} ---  (peer exited)
92924 03:12:09.771325 +++ exited with 0 +++
```

**In the transfer window the agent issued ONLY `splice()` (the relay) + `ppoll()` (readiness).
ZERO `read`/`recvmsg`/`recvfrom` of decrypted payload; ZERO `write`/`send` to leg F.**

Per-syscall totals for the agent process (whole run, including setup):

```
splice       2     ← the relay: splice(B=3 → pipe_w=9) and splice(pipe_r=7 → F=6)
ppoll       51     ← readiness polling on leg B
read        18     ← ALL pre-window: ELF/libc loader + /proc/{filesystems,self/maps} at 03:12:04.09–.12
recvfrom     3     ← ALL pre-window: rustls TLS HANDSHAKE control reads at 03:12:04.63–.64 (before kTLS arm)
recvmsg      0
write       76     ← all write(2,…) eprintln stderr logging (NONE to a socket)
send/sendto  0
sendmsg      1     ← SCM_RIGHTS fd-pass of F_peer to the reader child (coordination, not payload)
```

The pre-window reads are demonstrably non-payload — sampled content: `"\177ELF\2\1\1…"` (ELF
headers), `"nodev\tsysfs\nnodev\ttmpfs…"` (`/proc/filesystems`), `"…/libc.so.6\n…"` (`/proc/self/maps`).
The 3 `recvfrom(3)` are the rustls handshake at `04.63`, **1.6 s before** the transfer window opens
at `06.25`. The single `sendmsg(8, …SCM_RIGHTS…cmsg_data=[5])` hands the F_peer fd to the reader.

**The 86 decrypted banner bytes moved exclusively through `splice`. Agent-light CONFIRMED.**

### Control baseline — the `recvmsg`+`write` copy loop, for contrast

The same harness in `MODE=copyloop` (classic userspace relay) makes the contrast concrete. There the
payload bytes **transit userspace and appear inside the syscall trace**:

```
93515 …  sendto(7, "SERVER_BANNER_ktls_rx_splice_mus"..., 86, MSG_NOSIGNAL, NULL, 0) = 1   ← payload IN a syscall
   splice 0   recvfrom 41   sendto 1   →  copyloop recvmsg_bytes=86 write_bytes=86
```

In the splice run the banner string **never** appears in any `read`/`write`/`send` syscall; in the
copyloop run it appears verbatim inside `sendto`. That is the smoking-gun difference between
agent-light (splice) and the userspace-copy baseline.

---

## ASSERTION 2 — Byte-exact plaintext on F_peer

The reader (separate process) read the relayed bytes off F_peer. Single-record run, reproduced ×2:

```
splice_B_to_pipe calls=1 bytes=86  einval_on_B=0
per_splice_in_call_bytes=[86]
READER: got 86 bytes off F_peer (wanted 86)
READER: hex(first 16) = 53 45 52 56 45 52 5f 42 41 4e 4e 45 52 5f 6b 74
READER: first 96 lossy = "SERVER_BANNER_ktls_rx_splice_must_deliver_decrypted_plaintext_to_legF_agent_light_0001"
READER_RESULT: RELAY_EXACT_CLEAN — exact 86-byte plaintext, byte-identical, no framing
```

Byte comparison against the expected banner (`xxd` of the first 16 bytes):

```
F_peer received:  53 45 52 56 45 52 5f 42 41 4e 4e 45 52 5f 6b 74    "SERVER_BANNER_kt"
expected banner:  5345 5256 4552 5f42 414e 4e45 525f 6b74           "SERVER_BANNER_kt"
```

Identical. **F_peer received the exact 86-byte `SERVER_BANNER` plaintext, in order, no loss/dup.**

---

## ASSERTION 3 — No `-EINVAL`

`splice(legB → pipe)` returned the byte count, never `-EINVAL`, in **every** run (single-record ×2,
copyloop, and the 100 000-byte multi-record run): `einval_on_B=0` throughout. This empirically
confirms the splice path skips the psock `read_sock` `-EINVAL` check — exactly as the research
predicted from source (the `-EINVAL` is in `tls_sw_read_sock`, a function the splice path never
calls). The probe's `run_splice_pump` is instrumented to detect and bail on a persistent `-EINVAL`;
it never fired.

---

## ASSERTION 4 — Decrypt happened + record-framing characterization

### Wire oracle (decrypt, not passthrough)

`tcpdump -i lo` during the transfer:

```
leg B<->P (peer port): TLS application_data records on the wire (ciphertext)
      2  1703 0300       ← 0x17 = application_data, TLS 1.3 records
      1  1703 0302
SERVER_BANNER cleartext on the leg B<->P wire?  → 0   (encrypted; decrypt is required)
SERVER_BANNER cleartext on the leg-F loopback pair?  → 1   (the spliced plaintext landed on F)
```

kTLS confirmed live on leg B via `ss -tie`:

```
tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw txconf: sw
```

Ciphertext (`0x17` records) on the peer wire + plaintext on leg F + software kTLS RX armed ⇒ the
`splice` triggered the kTLS decrypt (`tls_sw_splice_read` → `tls_rx_one_record`); it is **not** a
passthrough.

### Record-framing layout — the increment-g 114-vs-92 caveat, settled

`splice` delivers **clean decrypted record payload only** — no framing leaks into leg F:

| | bytes on leg F | layout |
|---|---|---|
| increment-g (sockmap verdict, raw RX) | **114** | 5B TLS header + 92B banner + 1B inner-type + 16B GCM tag region — RAW pre-decrypt record |
| increment-h (THIS probe, splice) | **86** | **just the 86-byte banner plaintext** — no header, no inner-type byte, no tag |

The splice path hands `rxm->full_len` of the *decrypted data record* (the plaintext), confirming the
research's Gap-2 hypothesis: splice gives the clean payload, **not** the raw recv-stripped record the
verdict path saw. A return-path design over `splice` needs **no userspace framing strip** — the
kernel delivers application bytes.

### Splice-calls-per-record (the cost-tier granularity)

The 100 000-byte multi-record run characterizes the pump granularity precisely:

```
splice_B_to_pipe calls=7 bytes=100000  einval_on_B=0
per_splice_in_call_bytes=[16384, 16384, 16384, 16384, 16384, 16384, 1696]
READER: got 100000 bytes off F_peer (wanted 100000)
READER_RESULT: RELAY_EXACT_CLEAN — exact 100000-byte plaintext, byte-identical, no framing
```

Each `splice(B→pipe)` returns **at most 16384 bytes = 2^14 = the TLS 1.3 maximum record plaintext**.
6 full records (16 KiB each) + 1 tail record (1696 B) = 100 000 bytes, reassembled byte-exact on
leg F. **One `splice` call per TLS record** — kernel-paced and bounded, NOT one syscall per byte.

So the cost is **N records → ~N `splice` submissions** (plus readiness `ppoll`s), independent of
byte count within a record. For a single small response (the common SVID-handshake-then-banner
shape) that is **1 splice** per direction-event.

---

## Cost-tier — precise, honest

This is **AGENT-LIGHT**, not agent-idle. The distinction matters and is stated plainly:

| Direction | Mechanism | Decrypts? | Per-byte userspace copy? | Agent syscalls per connection | Tier |
|---|---|---|---|---|---|
| Forward F→B | egress sockmap redirect → kTLS-TX (`findings-egress-ktls-splice.md`) | n/a (encrypt) | No | **0** (kernel drives `tcp_sendmsg_locked`) | **AGENT-IDLE** |
| **Return B→F** | **`splice(B→pipe→F)`** (THIS probe) | **Yes** | **No** | **~1 `splice` + readiness `ppoll`s per TLS record** | **AGENT-LIGHT (zero-copy syscalls)** |

The agent must *drive* the `splice` pump (one bounded submission per record / readiness event) —
nothing pushes the decrypt autonomously (per research SQ3, `tls_sw_read_sock` refuses a psock and is
not a userspace-reachable syscall). But the agent **never copies a payload byte**: the kernel does
both the decrypt (`tls_rx_one_record`) and the byte movement (`skb_splice_bits` → pipe → leg-F socket
buffer). Strictly better than the sockmap-verdict `recvmsg` path increment-g foreclosed (which copies
on the read), and strictly better than the userspace copy-loop baseline (which copies twice).

An `IORING_OP_SPLICE` variant (research Finding 2.1) would batch the submissions and avoid blocking,
routing through the same `do_splice → tls_sw_splice_read` path — same zero-copy guarantee, fewer
syscall round-trips. Not tested here; the bare `splice(2)` primitive is sufficient to validate the
mechanism, and io_uring is an optimization of submission, not a different decrypt path.

---

## Reproducibility & honesty notes

- **3 independent splice runs** (single-record ×2 + the framing-dump run) all produced
  `splice_B_to_pipe calls=1 bytes=86 einval_on_B=0` and `RELAY_EXACT_CLEAN`. The multi-record run
  reassembled 100 000 bytes byte-exact. No flakiness observed.
- **strace timing**: `strace -f` was used for the agent-light capture with a generous settle
  (`PEER_SEND_DELAY_MS=1600`) so the single record lands deterministically inside the window. Unlike
  the increment-g verdict-engagement race (which `strace` perturbed), the splice pump has **no
  engagement race** — the agent `ppoll`s leg B and splices whatever decrypted record is ready, so
  strace does not change the outcome (the non-strace smoke run and the strace run agree).
- **fd-reuse confound handled**: the raw per-fd grep initially conflated startup-file-fd-3 with
  socket-fd-3; resolved by attributing syscalls by **timeline** (transfer window = at/after the first
  `splice`). The decisive claim rests on the timeline-attributed window trace, not a raw fd count.
- **Reader label**: the reader's success message reads `RELAY_EXACT_CLEAN` in all byte-exact cases
  including `MODE=copyloop` (the reader does not know the relay mode) — substance (86/100000 bytes
  byte-identical) is what is asserted, not the mode label.
- **No harness artifact masquerading as the kernel result**: the agent's payload-syscall count is
  read straight from `strace`, the byte-exactness from a *separate-process* reader over a real
  socket, the ciphertext from `tcpdump` on the wire, the kTLS arm from `ss -tie`. The splice byte
  counts are the kernel's own `splice()` return values.

---

## Environment & cleanup

- Built `--no-sudo` as the Lima user; agent run as root via `cargo xtask lima run --` (for
  `tcpdump`/`strace`/`ss` capture; the kTLS arm itself needs no special cap).
- `CARGO_TARGET_DIR=./target-scratch` (local, gitignored).
- No `overdrive-*` crate touched. Minimal rcgen P-256 self-signed cert. No BPF object, no sockmap.
- **All kernel state cleaned at the end**: no stray procs, no XDP attachments (the probe used no
  BPF), no `sk_skb`/sockmap progs, no bpffs pins, loopback verified healthy (`ECONNREFUSED`, no
  hang). Evidence tmp files removed.
- **Phase-2 gate NOT run. Nothing promoted.**

---

## Bottom line for #222 / #26

- **`splice(2)` on a plain kTLS-RX socket (no psock) delivers byte-exact decrypted plaintext to leg
  F agent-light** — research verdict (c) → **(a) CONFIRMED**.
- **#222 is agent-light in both directions**: agent-idle forward (sockmap→kTLS-TX), agent-light
  zero-copy-splice return (~1 splice per record, no per-byte userspace copy).
- The splice path delivers **clean application plaintext** (no TLS framing), so the return-leg design
  needs no userspace framing strip — simpler than the increment-g verdict path suggested.
- **Q1 (#26 in-band lossy DROP-RESET gate) is unchanged** by this finding.
