# SPIKE Decisions — transparent-mtls-host-socket (GH #26)

## Assumption Tested

In-band **sidecarless kTLS** is achievable **race-free** on the pinned-floor
kernel for ONE process→process host-socket flow:
`sockops ACTIVE_ESTABLISHED → pidfd_getfd → rustls TLS 1.3 handshake (secret
extraction) → kTLS install (setsockopt TCP_ULP "tls" + TLS_TX/TLS_RX) → agent
EXITS the data path → tcpdump shows TLS 1.3 records → no cleartext before
install`. This is unshipped anywhere (Cilium = out-of-band auth + separate
WireGuard/IPsec; Istio = userspace proxy stays in the data path).

Ran on kernel **7.0.0-15-generic** (Ubuntu 26.04; ≥ the pinned 6.18 floor,
ADR-0068). Full evidence: [`findings.md`](./findings.md).

## Probe Verdict

**PARTIAL — in-band kTLS is REAL; "race-free" is qualified.** Per increment:
**A** (rustls→kTLS, agent exits) = WORKS · **B** (`pidfd_getfd`, workload-owns-fd)
= WORKS · **C** (sockops + sk_msg race gate) = fail-closed but **lossy** · **D**
(compose / ordering) = a hard kernel ordering invariant confirmed.

**The Cilium out-of-band fallback is NOT selected** — the riskiest assumption
held. The one qualification: the race window closes fail-closed (no cleartext
leak) but **lossily** (`sk_msg` has no lossless HOLD — a pre-arm write is
`SK_DROP`'d → `EACCES` + dead connection). This constrains the v1 contract; it
does not kill the design.

## Promotion Decision

**DISCARD → hand to DESIGN** (findings are the durable record; **NOT** promoted
to a walking skeleton). `spike-scratch/` is retained gitignored as a DELIVER
reference (user decision, 2026-06-11), not committed.

**Rationale.** The probe is three independent micro-probes (A/B/C) plus
raw-syscall checks (D), **not a single composed end-to-end flow** — so "promote"
would mean *building* the composed walking skeleton from scratch, not wrapping an
existing one. Worse, building it now would force three mechanism choices the
probe **deliberately leaves to DESIGN**:

1. the **v1 race-window contract** (documented lossy-DROP-then-RESET under a
   named server-speaks-first assumption, vs a lossless arm-before-write stall at
   `cgroup/connect4` or a `sockops` stall);
2. **control-record handling** (reuse `ktls::KtlsStream` vs raw `setsockopt` +
   ticket suppression);
3. the **gate hook** + the gate-before-kTLS ordering invariant.

The findings are decision-grade; DESIGN locks those choices, then DELIVER builds
the real e2e slices (00–05) on the chosen mechanism. PIVOT was not in play
(mechanism confirmed viable).

## Walking Skeleton

N/A — not promoted. The DELIVER-wave slices (`../slices/slice-00…05`) build the
real end-to-end flow after DESIGN locks the mechanism.

## Design Implications

1. **In-band kTLS is viable — do NOT fall back to Cilium out-of-band.**
2. **Gate-before-kTLS is a hard kernel invariant**: a `SOCKMAP` insert must
   precede `TCP_ULP "tls"`; the reverse is `EINVAL` (both replace `sk->sk_prot`).
   Pin with a Tier-3 test (`tls-ULP-after-sockmap == EINVAL`).
3. **Pick the v1 race-window contract deliberately.** `sk_msg` cannot HOLD;
   fail-closed is lossy. v1 (documented loss-then-RESET) is acceptable **only if
   the server-speaks-first assumption is named explicitly**; a lossless
   pre-first-byte stall is a follow-up Tier-3 question.
4. **Control records must be handled** (`NewSessionTicket` → `EIO` on raw kTLS
   RX; KeyUpdate is the same class) — suppress for the internal mesh or route
   out-of-band; favours reusing `ktls::KtlsStream` over raw `setsockopt`.
5. **`pidfd_getfd` is the right handoff primitive** (workload-owns-fd shape;
   cross-uid needs `CAP_SYS_PTRACE`). CP-restart survival of *operative* crypto
   remains the #26-coupled Tier-3 question (CLAUDE.md) — not answered here.
6. **Cert is a non-risk** — a minimal rcgen P-256 drove every handshake. Real
   `IdentityRead`/SPIFFE-SAN SVID (#35) is a DELIVER wiring concern.

## Constraints Discovered

- `SOCKMAP` insert must precede `TCP_ULP "tls"` (reverse = `EINVAL`).
- `sk_msg` PASS/DROP/REDIRECT only — no lossless HOLD; `bpf_msg_cork_bytes` does
  not buffer. Pre-arm write → `EACCES` + dead connection.
- TLS 1.3 control records (`NewSessionTicket`, KeyUpdate, renegotiation) → `EIO`
  on raw kTLS RX unless handled.
- `pidfd_getfd` cross-uid requires `CAP_SYS_PTRACE` on the target.
- Confirmed constants on 7.0: `SOL_TLS=282 TLS_TX=1 TLS_RX=2 TCP_ULP=31
  TLS_CIPHER_AES_GCM_256=52 sizeof(tls12_crypto_info_aes_gcm_256)=56`.

## Environment note (infra)

The dev VM was rebuilt to Ubuntu 26.04 / kernel 7.0 for this spike. Two
`infra/lima/overdrive-dev.yaml` fixes were required and land with this commit:
(1) `base: template:_images/ubuntu-26.04` → a direct `images:` cloud-image URL
(limactl 2.1.1 ships no 26.04 image template); (2) removal of the defunct
`qemu-kvm` package (no installation candidate on 26.04; under `set -e` it aborted
the entire apt block, dropping every build dependency).

---

# SPIKE Decisions — increment M (blocking splice into kTLS-TX, 2026-08-26)

## Assumption Tested

- A **blocking** `splice(legF → pipe → legB kTLS-TX)` pump delivers every
  application byte losslessly and byte-exact — including under leg-B
  send-buffer exhaustion (`SO_SNDBUF=2048` + slow peer reader), the condition
  where the retired non-blocking (`MSG_DONTWAIT`) paths lost ~10–15% of
  records.

## Probe Verdict

- **WORKS**: 20/20 runs `PEER_RESULT: EXACT` (1× 86 B, 1× 100 000 B, **15/15**
  1 000 000 B adversarial, 3× copy-control); `eagain=0`, no stalls, no
  `EINVAL`; strace proves the pump issues only `splice` on the payload path.
  Full evidence: `findings-ktls-tx-blocking-splice.md`.

## Promotion Decision

- **PROMOTE** (user, 2026-08-26): swap the two production COPY pumps
  (forward `legF→legB`, response `legS→legC` in
  `crates/overdrive-dataplane/src/mtls/`) for the blocking-splice pump. The
  verdict is pinned to the dev-VM kernel 7.0.0-29-generic, not the pinned 6.18
  appliance kernel (ADR-0068) — accepted explicitly by the user; the Tier-3
  suite run at promotion carries the signal, and the 6.18 matrix re-proves it
  at merge time.

## Design Implications

- The `MSG_DONTWAIT` record-loss class is a property of *non-blocking*
  delivery into `tls_sw_sendmsg`, NOT of `splice(2)` — three stale claims to
  correct on promote: `mtls/mod.rs:21-23`, `mtls/outbound.rs:11-12`,
  `website/content/docs/concepts/transparent-mtls.mdx` (the encrypt-side
  asymmetry passage).
- Pre-arm prelude bytes still ride `write_all` (an in-memory `Vec` cannot be
  spliced); only the steady-state pump changes.

## Constraints Discovered

- `shutdown(SHUT_WR)` on a kTLS leg emits a bare FIN with no TLS
  `close_notify` — identical to the copy pump's close shape (neither sends
  close_notify); preserve, don't "fix".
