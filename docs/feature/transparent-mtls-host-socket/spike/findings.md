# Spike findings — transparent-mtls-host-socket (GH #26)

**nw-spike Phase 1 (PROBE) — throwaway, real-kernel.** Kernel `7.0.0-15-generic`
(Ubuntu 26.04, aarch64; ≥ the pinned 6.18 floor, ADR-0068). aya 0.13.1 /
aya-ebpf 0.1.1 / bpf-linker 0.10.3 / rustc 1.95.0. Throwaway code lives
(gitignored) in `spike-scratch/{increment-a,increment-b,increment-c}/`. **Not
promoted; no walking skeleton built; no `overdrive-*` API touched.**

Captured: 2026-06-11. Loopback only; software kTLS; AES-256-GCM TLS 1.3 only.

## Overall verdict: **PARTIAL — in-band kTLS is REAL; "race-free" is qualified**

| # | Unknown | Verdict |
|---|---------|---------|
| 1 | rustls→kTLS handback accepted by kernel | **CONFIRMED (works)** |
| 2 | agent EXITS the data path (kernel does crypto) | **CONFIRMED (works)** |
| 3 | `pidfd_getfd` cross-process fd handoff | **CONFIRMED (works)** |
| 4 | race window closes **fail-closed** | **CONFIRMED fail-closed, but LOSSY** |

Per-increment: **A = WORKS · B = WORKS · C = PARTIAL · D = validated-by-parts +
ordering constraint.**

**The Cilium out-of-band fallback is NOT selected** — in-band kTLS is alive on
7.0. The single qualification is unknown 4 (the race window closes fail-closed
but not losslessly); it constrains the v1 contract but does not kill the design.

## Increment A — rustls → kTLS, plain socket, same process (unknowns 1+2): **WORKS**

Built on tokio-rustls + the `ktls` crate v6.0.2 (the canonical Rust in-band
path: handshake → `CorkStream` drain → `dangerous_extract_secrets()` →
`setsockopt(TCP_ULP "tls")` + `TLS_TX`/`TLS_RX`). Two TLS 1.3 halves over `lo`;
agent writes/reads **plaintext**, kernel does crypto.

Kernel-side proof (`ss -K` on the live socket):

```
ESTAB ... tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw txconf: sw
```

`rxconf/txconf: sw` = software kTLS, kernel does the crypto. Wire
(`tcpdump -i lo`) shows 5× `1703 03` Application Data records; the encrypted
42-byte marker appears as `1703 0300 45` (len 0x45 = 42 + inner-type + 16-byte
GCM tag). **Plaintext-leak oracle `strings pcap | grep MARKER` = CLEAN (0
hits).** → Unknowns 1 & 2 confirmed.

## Increment B — `pidfd_getfd` cross-process handoff (unknown 3): **WORKS**

Three processes: **workload** (`connect()` → hands `(pid, fd)` to agent → reads
plaintext off its **own** fd), **agent** (`pidfd_open` + `pidfd_getfd` to
duplicate the workload's socket → rustls client handshake on the dup → kTLS
install via **raw `setsockopt`** → closes its dup), **server**.

```
AGENT: pidfd_getfd OK — acquired dup fd 6 of workload's socket
AGENT: TCP_ULP=tls installed on dup fd 6 ; TLS_TX + TLS_RX installed
AGENT: closed its dup fd (kTLS persists on the workload's fd via shared sk)
WORKLOAD: SUCCESS — read SERVER_MARKER as PLAINTEXT off my own fd (kernel decrypted)
```

kTLS lives on the shared `struct sock` — the agent installs on a duplicate,
**closes it**, and the workload's own fd still carries kTLS. This is the
**workload-owns-fd** shape (so a later restart-survival slice is reachable).
`pidfd_getfd` worked **unprivileged** for same-uid; cross-uid needs
`CAP_SYS_PTRACE` on the target.

**Load-bearing finding from B:** first run failed with `EIO` — the server's
post-handshake TLS 1.3 **`NewSessionTicket`** (a control record) on the kTLS RX
path returns `EIO`, because raw-kTLS RX only decrypts `application_data`. Fixed
with `send_tls13_tickets = 0`. **DESIGN must handle control records** (suppress
tickets/renegotiation for the internal mesh, OR route them out-of-band) — a
strong reason to **reuse the `ktls` crate's `KtlsStream`** (it does the
control-message loop) over the raw hand-roll. KeyUpdate (present on 7.0) is the
same class, out of probe scope.

## Increment C — sockops detect + race-window gate (unknown 4): **PARTIAL (decision-grade)**

Real aya BPF object (`sockops` → `SOCKHASH` + ringbuf; `sk_msg` egress verdict =
DROP-until-`ARMED`) attached to a real cgroup on 7.0. Race probe: workload writes
`PLAINTEXT_PROBE` immediately on connect; agent delays 500 ms then arms.

**The gate closes the window fail-closed:** wire oracle
`grep PLAINTEXT_PROBE pcap = 0`, sink received 0 bytes — the cleartext probe
**never egressed**. Keys matched exactly (agent armed-key == sk_msg
computed-key).

**But the close is lossy + connection-breaking:**

```
WORKLOAD: PLAINTEXT_PROBE write result=Err(Os { code: 13, PermissionDenied })   ← SK_DROP → EACCES
WORKLOAD: ARMED_DATA      write result=Err(Os { code: 13, PermissionDenied })   ← connection now dead
DEBUG sk_msg: invocations=1 PASS=0 DROP=1                                        ← never re-invoked after the drop
```

The PASS path itself is sound (isolation run, arm-before-write):
`invocations=2 PASS=2 DROP=0`, sink got all 105 bytes. So armed → data flows;
the only failure is the **lossy DROP** when a byte is written before arming.
**`sk_msg` has PASS/DROP/REDIRECT and no HOLD; `bpf_msg_cork_bytes` does not
buffer.** A pre-arm write yields `EACCES` + a dead connection — exactly the
prompt's hypothesis, now confirmed on 7.0.

## Increment D — compose + the hard ordering constraint

Rather than re-wire A+B+C, the probe answered the one new compose question via
raw-syscall micro-probes — **can the `sk_msg` gate (`SOCKMAP`) and kTLS
(`TCP_ULP "tls"`) coexist, and in what order?**

```
sockmap insert THEN tls ULP:  insert rc=0 ;  ULP rc=0           ← WORKS
tls ULP THEN sockmap insert:  ULP rc=0   ;  insert rc=-1 EINVAL ← REJECTED
```

**Gate-before-kTLS is a hard kernel invariant** — both replace `sk->sk_prot`, so
inserting into the sockmap after `tls` ULP is `EINVAL`. The natural flow
(detect → gate → install) satisfies it, but DESIGN must encode it and a Tier-3
test must pin `tls-ULP-after-sockmap == EINVAL`. Confirmed constants:
`SOL_TLS=282 TLS_TX=1 TLS_RX=2 TCP_ULP=31 TLS_CIPHER_AES_GCM_256=52
sizeof(tls12_crypto_info_aes_gcm_256)=56` (the B hand-roll's 56-byte struct
matched → install worked).

## Design implications (for DESIGN)

1. **In-band kTLS is viable — do NOT fall back to Cilium out-of-band.**
2. **Gate-before-kTLS ordering is a hard invariant** (`SOCKMAP` insert must
   precede `TCP_ULP "tls"`; reverse is `EINVAL`). Pin with a Tier-3 test.
3. **The race window is fail-closed but lossy — pick the v1 contract
   deliberately.** `sk_msg` cannot HOLD. Options:
   - **(v1, simplest)** documented data-loss-then-RESET on a rare pre-arm write
     (security invariant holds — no cleartext leak). Acceptable **IF
     server-speaks-first** — *name that assumption explicitly*; a
     client-speaks-first protocol eats a reset on the first connection until
     armed.
   - **(stronger)** arm before the workload can write (gate at
     `cgroup/connect4`, or stall at a `sockops` callback until the SVID is
     ready). A true lossless pre-first-byte stall is a **follow-up Tier-3
     question**, not answered here.
   - `REDIRECT`-to-agent-buffer re-introduces an agent in the data path (the
     thing we're avoiding) — not probed.
4. **Control records (`NewSessionTicket`, KeyUpdate, renegotiation) must be
   handled, not ignored** — suppress for the internal mesh or route out-of-band;
   favours reusing `ktls::KtlsStream` over raw setsockopt.
5. **`pidfd_getfd` is the right handoff primitive** (cross-uid needs
   `CAP_SYS_PTRACE`). Whether *operative* crypto survives a control-plane
   restart remains the #26-coupled Tier-3 question already flagged in CLAUDE.md
   — **not** answered here.
6. **Cert is a non-risk** — a minimal rcgen P-256 drove every handshake. Real
   `IdentityRead`/SPIFFE-SAN SVID (#35) is a Phase-3 promotion concern.

## Scope / honesty notes

Loopback only; software kTLS; AES-256-GCM TLS 1.3 only. Throwaway, no
`overdrive-*` touched. KeyUpdate, renegotiation, NIC offload, restart-survival of
operative crypto, and a true lossless pre-first-byte stall are out of scope and
are the named follow-ups. Kernel state cleaned up (cgroup removed, no stray BPF
progs).

**Stopped after findings per the brief — did NOT run the Phase-2 promotion gate
or promote.** `spike-scratch/` left in place (gitignored) for review.
