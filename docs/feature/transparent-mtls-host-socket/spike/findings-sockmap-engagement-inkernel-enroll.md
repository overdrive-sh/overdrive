# Spike findings — in-kernel sockops enroll-at-establishment closes the sk_skb engagement race (transparent-mtls-host-socket, GH #26)

**nw-spike Tier-3 PROBE — throwaway, real-kernel, increment J.** Kernel
`7.0.0-15-generic` (Ubuntu, aarch64; ≥ the pinned 6.18 floor, ADR-0068).
`CONFIG_TLS=m` (loaded), `CONFIG_BPF_STREAM_PARSER=y`, `CONFIG_NET_HANDSHAKE=y`,
`CONFIG_CGROUP_BPF=y`. aya 0.13.1 / aya-ebpf 0.1.1 / ktls 6.0.2 / rustls 0.23 /
rcgen P-256 / rustc 1.95.0 (agent) + nightly (BPF target, `-Z build-std=core`).
Throwaway code lives (gitignored) in
`spike-scratch/increment-j-sockops-enroll/{bpf,agent}/`. **Not promoted; no
`overdrive-*` API touched; nothing under `crates/` modified or committed.**

Captured: 2026-06-13. Loopback only; software kTLS; AES-256-GCM TLS 1.3 only;
agent-as-TLS-client shape.

This probe settles **Gap 1 + Gap 4** of
`docs/research/dataplane/sockmap-strparser-engagement-race-research.md`
(direction (A): enroll leg F at the empty-by-construction moment) and the
verdict-engagement race named in
`findings-egress-ktls-splice.md` § "Load-bearing mechanics" #3.

---

## THE ONE THING UNDER TEST (the decisive experiment)

> **Direction A (research Finding 4a/6):** does enrolling leg F into the
> `SOCKMAP` **IN-KERNEL at the sockops `BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB`
> moment** — a `sock_ops` program that calls `bpf_sock_map_update(skops,
> &SOCKMAP, &F_IDX, 0)` the instant leg F becomes `ESTABLISHED`, before
> userspace `accept()` returns and before any app byte — make the forward
> F→B kTLS-TX splice **lossless across 20 runs**, INCLUDING the clean-queue
> sentinel shape (the run-17 case) that the userspace-after-`accept()` enroll
> loses ~20% of the time?

Kernel mechanism (research, kernel-source-pinned): `sk_psock_start_verdict`
swaps `sk->sk_data_ready` PASSIVELY (Finding 1/2) and governs only the *next*
event; it does NOT drain the backlog. A userspace enroll *after* `accept()`
races the workload's first app byte through the OLD callback (`invocations=0`).
The sockops in-kernel enroll installs the verdict's `sk_data_ready` AT the
`ESTABLISHED` transition — on a provably-empty receive queue, before the app
byte and before `accept()` returns — so the first byte is serviced by the
verdict. This is the structural enroll Cilium uses (research Finding 4a).

---

## VERDICT: **YES — in-kernel sockops enroll-at-establishment makes the forward splice LOSSLESS on 7.0 (≥6.18 floor).**

Direction A closes the `invocations=0` race **deterministically and
structurally**. Both decisive shapes are byte-exact across the gate, INCLUDING
the clean-queue sentinel (the exact failure direction the userspace-after-accept
enroll could not close). Direction B (kernel-nudge) is **NOT needed** — there is
no residual loss to close.

| Shape | Runs | EXACT | FAIL | Notes |
|---|---|---|---|---|
| **(a) clean-queue sentinel** (run-17 case) | **20** | **20** | **0** | single sentinel byte written AFTER arm on a never-pre-queued socket → `SK_REDIRECT`ed losslessly |
| **(a) clean-queue, hardened** | **40** | **40** | **0** | 0% loss is not a 20-sample fluke |
| **(b) speaks-first / phase-split** (55/110 case) | **20** | **20** | **0** | pre-arm phase captured via `SK_PASS`→drain→flush; steady phase `SK_REDIRECT`ed; both byte-exact, in order |
| **CONTROL: J_NO_ENROLL** (FLPORT unset → sockops can't enroll) | **5** | **0** | **5** | peer gets **0 bytes**, `enroll_ok=0`, `redir=0` — the negative population |

Every positive run: `enroll_ok=1`, `passive_est=2`, `redir=1`, `redir_err=0`,
`passive_port == FLPORT`. The peer's kTLS-RX reconstructed the exact plaintext,
in order, no loss.

**The population diff (debugging.md §5) is the proof.** The ONLY difference
between EXACT (20/20, 40/40) and NOTHING (0 bytes, control) is whether the
sock_ops in-kernel enroll fired (`enroll_ok=1` vs `0`). No residual userspace
path enrolls leg F — the in-kernel sockops enroll IS the mechanism.

---

## Probe log (Hypothesis / Prediction / Falsification per debugging.md §4/10)

### Probe 1 — clean-queue sentinel (the decisive run-17 case)
- **Hypothesis:** the sockops enroll at `PASSIVE_ESTABLISHED_CB` installs the
  verdict's `sk_data_ready` before any byte, so a single byte written on a
  never-pre-queued socket after ARM is `SK_REDIRECT`ed losslessly.
- **Predicted:** 20/20 `PEER_RESULT: EXACT got=87 want=87`; `enroll_ok=1`,
  `redir=1`, `redir_err=0` every run.
- **Result:** 20/20 EXACT (and 40/40 on the hardened burst). Predicted
  counters every run. **Hypothesis confirmed.**

### Probe 2 — speaks-first / phase-split (the 55/110 case)
- **Hypothesis:** with leg F enrolled at establishment + ARMED-gate, the pre-arm
  phase `SK_PASS`es to leg F's own recv queue (captured by userspace drain,
  flushed through leg B post-arm) and the steady phase `SK_REDIRECT`s — both
  byte-exact.
- **Predicted:** 20/20 `EXACT got=144 want=144`; `pass_prearm=1`, `redir=1`.
- **Result:** 20/20 EXACT. `pass_prearm=1`, `redir=1` every run. **Confirmed.**

### Probe 3 (CONTROL) — J_NO_ENROLL: is the sockops enroll actually load-bearing?
- **Hypothesis:** if FLPORT is never written, the sockops can't identify leg F
  and never enrolls it; leg F is not a sockmap member; the verdict never fires
  on its bytes; the peer receives nothing.
- **Predicted:** 0 bytes at the peer; `enroll_ok=0`; `redir=0`;
  `last_port` ≠ leg F's port (the verdict only `SK_PASS`es leg B's RX).
- **Falsification path:** if the peer still got EXACT, some *other* path is
  redirecting and the verdict above is invalid.
- **Result:** 5/5 `got_len=0`; `enroll_ok=0`; `redir=0`; `last_port` = leg B's
  ephemeral port (3 leg-B handshake `SK_PASS`es). **Falsification did NOT fire —
  the in-kernel enroll is the sole enrollment mechanism.**

---

## Topology built (mirrors increment-f; enroll moved in-kernel)

- **PEER P** — a rustls TLS 1.3 server that arms **kTLS RX** (ktls crate),
  DECRYPTS what it receives, and reports byte-exactness against the expected
  payload (the loss oracle).
- **Leg B (kTLS, peer-facing)** — the agent's client socket to P. Inserted into
  `SOCKMAP[1]` **from userspace** BEFORE `TCP_ULP "tls"` (the arming invariant);
  rustls handshake; arm kTLS TX+RX. Leg B is the agent's OWN connect socket — it
  is not subject to the leg-F enroll race, so userspace enrolls it directly.
- **Leg F (plaintext source)** — a self-connected loopback TCP pair the agent
  owns. The **agent owns the leg-F LISTENER inside the cgroup**; the ACCEPT end
  (leg F target) is enrolled into `SOCKMAP[0]` **IN-KERNEL** by the sock_ops
  program at `PASSIVE_ESTABLISHED_CB`; the CONNECT end is the writer.
- **The sock_ops program (`j_enroll`)** — attached to a per-run cgroup the agent
  joins. On `BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB` for a socket whose host-order
  `local_port == FLPORT[0]` (the agent's leg-F listener port, written by
  userspace BEFORE accept), it calls `SockMap::update(F_IDX, ctx.ops, 0)` →
  `bpf_sock_map_update` and records the socket's `local_port` into `FPORT[0]`
  for the verdict. **This is the enroll** — userspace does NOT call
  `sockmap.set(F_IDX, ...)`.
- **The verdict (`j_verdict`, `sk_skb/stream_verdict`)** — the increment-f
  ARMED-gated program. leg-F skb && `ARMED==0` ⇒ `SK_PASS` (pre-arm capture to
  leg F's own recv queue, no redirect → no plaintext leak); leg-F skb &&
  `ARMED==1` ⇒ `SK_REDIRECT` (EGRESS, `flags=0`) into leg B's kTLS TX; leg B's
  own RX / any non-FPORT skb ⇒ `SK_PASS`.

`passive_est=2` per run: the sockops `PASSIVE_ESTABLISHED_CB` fires for both the
leg-F accepted child (matched by FLPORT, enrolled) AND the peer-P-side accept of
leg B (a different local_port, not matched, not enrolled). Only the FLPORT match
enrolls — `enroll_ok=1` exactly once.

---

## Design input for productionisation (the open question, answered on the real kernel)

### Where the sock_ops program attaches
**A cgroup v2 directory containing the agent's leg-F listener.** `aya::programs::
SockOps::attach(cgroup_fd, CgroupAttachMode::Single)` — sockops attach to a
cgroup, not an interface. The leg-F *listener* must `accept()` inside that
cgroup for `PASSIVE_ESTABLISHED_CB` to fire on the leg-F child. In production
the agent is the proxy process that owns the leg-F listener; attach the sockops
to the agent's own cgroup (or a dedicated child cgroup the agent's proxy
listener runs in). This is the same attach surface as the existing
`cgroup_connect4` intercept (`aya::programs::CgroupSockAddr`), so the
composition already has a cgroup handle.

### How leg-F sockets are identified (the load-bearing design choice)
**By the agent's leg-F LISTENER local port, written to a map BEFORE the listener
accepts.** The sockops `PASSIVE_ESTABLISHED_CB` carries the accepted child's
`local_port` (host order in the sock_ops ctx UAPI), which equals the listener's
port. The agent writes that port to `FLPORT[0]` before `accept()`; the sockops
matches `local_port == FLPORT[0]` and enrolls. This is:
- **race-free:** FLPORT is set before any leg-F connection can establish, so the
  match is always available when the CB fires.
- **precise on loopback and beyond:** the PASSIVE side is the accept end (leg F
  target); the agent's own ACTIVE connect (the writer / the workload's connect)
  has a different ephemeral port and is NOT matched. Verified: the control
  (FLPORT unset) enrolls nothing; the positive runs enroll exactly the leg-F
  child (`last_port == FLPORT`).
- **generalises off-loopback:** the spike plumbs the matched child's own
  `local_port` into `FPORT` (separate from `FLPORT`) for the verdict, so a
  non-loopback bind where the child's local port differs from the listener's is
  already accommodated by the two-map split.

**Production refinement to consider** (NOT required for correctness, surfaced as
a design input, not a deferral): on a busy host the agent may run MANY leg-F
listeners. A single `FLPORT[0]` scalar identifies one listener; productionising
to N concurrent flows wants either (i) a `BPF_MAP_TYPE_HASH<u32 listener_port,
u8>` set of agent leg-F listener ports the sockops checks membership against, or
(ii) cgroup-scoping so EVERY `PASSIVE_ESTABLISHED_CB` in the agent's proxy
cgroup is a leg-F socket (no port match needed) — the cleanest shape if the
agent's proxy listeners live in a dedicated cgroup with nothing else accepting
there. Both are single-map / single-attach changes on top of the proven
sequence; neither alters the enroll mechanism this spike proved.

### The proven syscall / attach sequence (for the crafter)
```
1.  cgroup: create/select a cgroup v2 dir; the agent's leg-F listener accepts in it.
2.  load BPF ELF (sock_ops j_enroll + sk_skb/stream_verdict j_verdict + 4 maps).
3.  attach verdict: SkSkb::load(); SkSkb::attach(&sockmap_fd).
4.  attach sockops: SockOps::load(); SockOps::attach(cgroup_fd, CgroupAttachMode::Single).
5.  userspace enrolls LEG B (its own connect socket) into SOCKMAP[1] BEFORE TCP_ULP.
6.  leg B: rustls handshake → arm kTLS TX+RX (ARMED still 0).
7.  leg-F listener: bind; write FLPORT[0] = listener local_port; THEN accept().
        → on accept, the sock_ops PASSIVE_ESTABLISHED_CB has ALREADY enrolled the
          leg-F child into SOCKMAP[0] (bpf_sock_map_update) and set FPORT[0].
          Userspace does NOT call sockmap.set(F_IDX, ...).
8.  pre-arm capture (if the workload speaks first): drain leg F's own recv queue
        (the SK_PASS path), flush captured plaintext through leg B post-arm.
9.  flip ARMED[0] = 1 → steady-state leg-F bytes SK_REDIRECT into leg B kTLS TX.
10. flip-moment guard: one more non-blocking drain of leg F's recv queue + flush
        (catches a byte that SK_PASSed between the last capture and the flip).
```

Kernel-side primitives used (all present in aya-ebpf 0.1.1, no hand-rolling):
`#[sock_ops]`, `SockOpsContext::{op, local_port}`, `SockMap::update` →
`bpf_sock_map_update`, `SockMap::redirect_skb` → `bpf_sk_redirect_map`,
`BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB = 5`. Userspace:
`aya::programs::{SockOps, SkSkb}`, `CgroupAttachMode::Single`.

### Relationship to the current `crates/.../mtls/outbound.rs`
The current production `establish` enrolls leg F **from userspace, after
`accept()`** (`forward.enroll_leg_f(leg_f_fd, ...)` calls `sockmap.set(F_IDX,
...)`). That is the LATE enroll the research/this-spike shows loses the TOCTOU
byte. Productionising direction A means **moving the leg-F enroll into a
sock_ops program** (new kernel-side program + cgroup attach) and **deleting the
userspace `sockmap.set(F_IDX, ...)`** from `enroll_leg_f` (it keeps only the
FLPORT write, moved BEFORE the accept). The ARMED-gate + pre-arm-capture +
flip-moment-guard logic stays as-is. The kernel-side `MTLS_SOCKMAP` /
`MTLS_FPORT` / `MTLS_ARMED` maps stay; one new map (`MTLS_FLPORT`, the listener
port) + the sock_ops program are added.

---

## Scope / honesty notes

Loopback only; software kTLS (`rxconf: sw txconf: sw`); AES-256-GCM TLS 1.3
only; agent-as-TLS-client shape; single-record sentinel (87 B) for clean-queue,
two-phase 144 B for phase-split. **PROVEN (deterministic):** in-kernel sock_ops
enroll at `PASSIVE_ESTABLISHED_CB` enrolls leg F before any app byte; the
forward F→B kTLS-TX splice is lossless across 20/20 (clean-queue), 40/40
(clean-queue hardened), and 20/20 (phase-split); the negative control (no
enroll) loses 100% (0 bytes), isolating the enroll as the sole mechanism.
**NOT exercised:** the reverse (decrypt) direction splice (out of scope —
increment-g/h cover it; the leg-B-RX-psock vs kTLS-RX conflict from
`findings-egress-ktls-splice.md` #2 / increment-g is unchanged by this probe);
multi-record / large transfers; NIC/hardware kTLS offload; N concurrent flows
(single FLPORT scalar today — see "Production refinement"); restart-survival;
the full transparent-intercept compose (deliberately isolated, as increment-f
did, to settle the enroll-timing primitive without a `cgroup_connect4` confound).
This probe used the agent's own leg-F listener inside a cgroup as the
PASSIVE-side accept; in production the workload's connect is intercepted by
`cgroup_connect4` and the agent's proxy listener is the accept end — the
sockops fires on the proxy listener's accepted child exactly as here.

**Direction B (kernel-nudge drain) was NOT needed and NOT built** — direction A
alone closed the loss to 0 across 60 positive runs. If a future non-loopback or
high-load environment surfaces a residual, the research's `tcp_data_ready` nudge
remains the documented fallback (research Finding 6 (B)); this spike found no
residual to close.

**Kernel state cleaned up after the runs:** no stray `incj-*` cgroups, no
increment-j processes, no sock_ops/sk_skb progs left attached; loopback healthy
(refused `:1` fast, did not hang) — verified. Each run loads fresh BPF and the
maps/programs are reclaimed at process exit; the per-run cgroup is removed by
the agent's cleanup.

**Stopped after findings per the brief.** `spike-scratch/increment-j-sockops-
enroll/` left in place (gitignored) for review. Nothing under `crates/` touched;
nothing committed.
