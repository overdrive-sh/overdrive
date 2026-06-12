# Spike findings — INBOUND transparent intercept + server-side mTLS + kTLS-RX decrypt → splice-to-S (transparent-mtls-host-socket, GH #26)

**nw-spike Phase 1 (PROBE) — throwaway, real-kernel.** Kernel
`7.0.0-15-generic` (Ubuntu 26.04, aarch64; ≥ the pinned 6.18 floor, ADR-0068).
`tls.ko` loaded (`tcp-ulp-tls` confirmed via `ss -tie`); `nft_tproxy`,
`xt_TPROXY`, `nf_tproxy_ipv4/6` all present and loaded; in-kernel TLS 1.3
TX+RX. rustc 1.95.0 / rustls 0.23.40 / rcgen 0.13.2 / ring 0.17.14 /
rustls-pemfile 2.2.0 / libc 0.2.186. Throwaway code lives (gitignored) in
`spike-scratch/increment-i-inbound-intercept/{agent}/`. **Not promoted; no
`overdrive-*` API touched; Phase-2 gate NOT run.**

Captured: 2026-06-12. Loopback only; software kTLS (`rxconf: sw txconf: sw`);
AES-256-GCM TLS 1.3 only; agent-as-TLS-**server** shape (the mirror of the
outbound agent-as-client probes).

This is the **inbound / server half** — the mirror of the proven outbound
proxy. Prior probes settled the outbound half:
- `findings-egress-ktls-splice.md` (increment-f): the **forward**
  F→B(kTLS-TX) direction WORKS agent-idle (egress sockmap redirect into a
  kTLS-armed target; agent does ZERO per-byte I/O).
- `findings-ktls-rx-splice.md` (increment-g): the kTLS-**RX** decrypt →
  sockmap-redirect **return** direction is FUNCTIONAL but **pull-driven** (not
  agent-idle) — the decrypted-verdict path needs a userspace `recvmsg` on the
  kTLS socket; `tls_sw_read_sock` returns `-EINVAL` when a psock is attached.
- The outbound intercept mechanism (`cgroup/connect4`) + orig-dst recovery is
  proven in the outbound increments.

**What was open and this probe settles:** the *inbound* counterpart — an
**inbound transparent intercept** (the mirror of `connect4`), **server-side
mutual-TLS terminate** (present S's SVID, **verify C's client SVID** chains to
the bundle), **kTLS-RX decrypt**, and **splice the decrypted plaintext to an
identity-unaware SERVER workload S**, agent-light. The dataplane decrypt+splice
is reused from increment-g/h; the **novel** work is (1) the inbound TPROXY
intercept + orig-dst recovery and (2) the server-side client-auth boundary.

---

## THE ONE THING UNDER TEST

> On 7.0, can an inbound connection aimed at a server workload S's logical
> address be **TPROXY-intercepted** to an agent, the agent perform a
> **server-side mutual-TLS** handshake (present S's SVID, **verify** C's client
> SVID against the bundle), **arm kTLS-RX**, and **splice the decrypted
> plaintext to an identity-unaware S** — so S reads the byte-exact plaintext,
> the wire on the client leg carries TLS ciphertext (`0x17`), the agent does
> ZERO per-byte payload I/O, and a missing / wrong-CA client cert **fails
> closed** (S receives nothing)?

## Overall verdict: **(a) YES — the inbound half works agent-light on 7.0. F3's inbound mechanism is proven.**

Every sub-claim was demonstrated with real-kernel evidence, all runs
timeout-bounded and clean (no hang, all `rc=0`):

1. **Inbound TPROXY intercept + orig-dst recovery WORKS** — a loopback
   connection aimed at the virtual dst `127.0.0.2:18443` is TPROXY-redirected
   to the agent's `IP_TRANSPARENT` listener, and `getsockname()` on the
   accepted socket **recovers the original destination `127.0.0.2:18443`** (the
   address C aimed at — the inbound mirror of `connect4`'s orig-dst recovery).
2. **Server-side mutual-TLS WORKS** — the rustls `ServerConfig` +
   `WebPkiClientVerifier` presents S's server SVID and **verifies C's client
   cert chains to the shared CA**. Valid client cert → handshake succeeds;
   kTLS-RX armed.
3. **Agent-light decrypt + splice-to-S WORKS** — the agent arms kTLS-RX on the
   client-facing leg and **`splice()`s** the decrypted 90-byte plaintext to S.
   S reads the **byte-exact** `CLIENT_REQUEST` as plaintext (S holds no
   cert/key, is identity-unaware). `tcpdump` shows TLS `0x17` app_data records
   on the client leg (ciphertext on the wire) and **zero cleartext of the
   request on the client leg**; the cleartext appears **only** on the
   agent→S leg (post-decrypt). `strace` shows the agent moves the payload via
   `splice`/`ppoll` only — **no per-byte `read`/`write`/`recv`/`send` of the
   payload**.
4. **Fail-closed WORKS** — `nocert` (no client cert) is rejected with
   `peer sent no certificates`; `wrongca` (client cert from an untrusted CA) is
   rejected with `invalid peer certificate: BadSignature`. In both, the agent
   delivers **nothing** to S (no splice, S receives 0 bytes). The two
   rejections carry **distinct reasons**, proving the verifier genuinely
   evaluates the chain rather than vacuously accepting/rejecting.

**Cost tier:** **agent-light, not agent-idle** — same discipline as the
outbound spikes. The agent issues the `splice`/`ppoll` syscalls for the payload
but never copies a payload byte into/out of userspace. This is *expected* for
the inbound direction: the decrypt-then-deliver path is pull-driven (the agent
reads the kTLS-RX plaintext via splice and pushes it to S). It is NOT agent-idle
(which would require an in-kernel decrypted-verdict redirect with no agent
syscall on the payload — foreclosed for the RX direction by increment-g's
finding that `tls_sw_read_sock` returns `-EINVAL` under a psock). For the
inbound server half, the agent-light splice is the right primitive.

---

## Architecture under test

```
CLIENT C  --TLS1.3 (presents client SVID)-->  127.0.0.2:18443 (S's logical addr)
                                                   |
                          [ nft TPROXY prerouting: ip daddr 127.0.0.2
                            tcp dport 18443 tproxy to 127.0.0.1:<agent>
                            meta mark 0x1 ; ip rule fwmark 0x1 lookup 100 ;
                            ip route local 0.0.0.0/0 dev lo table 100 ]
                                                   v
                                    AGENT  (IP_TRANSPARENT listener)
                                      | getsockname() -> ORIG_DST 127.0.0.2:18443
                                      | rustls ServerConfig: present S's SVID,
                                      |   WebPkiClientVerifier REQUIRE+VERIFY C's cert
                                      | dangerous_extract_secrets -> arm kTLS-RX (+TX)
                                      v
                          splice(client_leg_fd -> pipe -> to_S_fd)   <-- agent-light
                                      v
                          SERVER S  (plain TCP, holds NOTHING)
                                      reads byte-exact PLAINTEXT
```

Roles (one binary, `argv[1]`): `server` = S (plain TCP, reads plaintext, holds
no cert/key); `client` = C (rustls TLS-1.3 client presenting a client SVID, or
no/wrong cert); `probe-intercept` = isolated TPROXY-only probe (orig-dst
recovery, no TLS); default (`agent`) = the orchestrator (TPROXY setup, spawns S
+ C, accepts the intercepted leg, server-mTLS, kTLS-RX arm, splice-to-S).

---

## Evidence

### 0. Harness discipline — bounded, never hangs (the #1 fix this run)

The prior run left the harness **hung forever** because the agent ran unbounded
in the foreground (`run-full.sh` did a bare `"$AGENT_BIN"`), and three blocking
`accept()` calls had no timeout. In particular, the fail-closed modes deadlock
structurally: when client-auth is rejected the agent never connects to S, so
S's `accept()` blocks forever, so the agent's `server.wait()` blocks forever.

Fixes applied (all in `spike-scratch/increment-i-inbound-intercept/`):
- **`run-full.sh`**: the agent is wrapped in `timeout --kill-after=5 35` — a
  hard wall-clock backstop. Leftover agent procs are reaped **by PID** (never
  `pkill -f`, which self-matches the limactl session).
- **`agent/src/main.rs`**: every blocking `accept()` (server S, agent intercept
  leg, probe-intercept) now goes through `accept_with_timeout()` (poll the
  listener fd with a bounded budget, then non-blocking accept). The agent's
  child-reaping uses `wait_child_bounded()` — `try_wait` with a grace window,
  then **kill by handle**. The agent now always self-terminates and self-cleans.

Result: every run below completed within budget with `rc=0`; the VM is left
clean (no stray procs, no nft table, no ip rule/route, no stray listeners,
loopback healthy).

### 1. Inbound TPROXY intercept + orig-dst recovery (Crux Unknown 1 — the novel mechanism)

Isolated probe (`run-intercept-probe.sh`, NO TLS), real output:

```
===== 2. nft TPROXY rule: 127.0.0.2:18443 -> 127.0.0.1:18555 =====
table ip overdrive_spike {
	chain prerouting {
		type filter hook prerouting priority mangle; policy accept;
		ip daddr 127.0.0.2 tcp dport 18443 tproxy to 127.0.0.1:18555 meta mark set 0x00000001 accept
	}
}
PROBE_RESULT: ORIG_DST=127.0.0.2:18443 peer=127.0.0.1:54790
PROBE: RECOVERED original destination (getsockname) = 127.0.0.2:18443
PROBE: read 31 bytes off intercepted conn: "RAW_INTERCEPT_PROBE_MARKER_0001"
CLIENT(raw): connected to 127.0.0.2:18443; kernel peer = ('127.0.0.2', 18443), local = ('127.0.0.1', 54790)
```

- The connection aimed at the **virtual** dst `127.0.0.2:18443` is delivered to
  the agent's `IP_TRANSPARENT` listener on `127.0.0.1:18555` (a different
  address) — TPROXY intercept works on 7.0 loopback.
- **`getsockname()` on the accepted socket recovers the ORIGINAL destination
  `127.0.0.2:18443`** — the address C aimed at, which selects S's identity.
  This is the inbound mirror of `connect4`'s orig-dst recovery; the kernel keeps
  the intercepted socket's local addr as the original dst under TPROXY.
- The client still believes it reached `127.0.0.2:18443` (its `getpeername()`
  returns the virtual addr) — the intercept is transparent to C.

### 2. Full inbound mTLS compose — `ok` mode (valid client cert)

Real agent output:

```
AGENT_RESULT: ORIG_DST_RECOVERED=127.0.0.2:18443
AGENT_RESULT: MTLS_OK client_auth=VERIFIED client_cn=overdrive-spike-CA ktls_rx=ARMED
CLIENT C: handshake complete Some(TLSv1_3)
AGENT: server-side mTLS OK; client-auth VERIFIED ...; kTLS RX armed (rec_seq=0)
AGENT: connected to S (127.0.0.1:19777); splicing decrypted plaintext (agent-light)
SERVER_RESULT: PLAINTEXT_EXACT — S received the exact 90-byte CLIENT_REQUEST as plaintext, in order
=== SPLICE STATS === splice_in calls=1 bytes=90 ; splice_out calls=1 bytes=90 ; poll=12
SERVER S: first 96 lossy = "CLIENT_REQUEST_inbound_mtls_must_arrive_as_plaintext_at_server_S_agent_light_in_order_0001"
```

- Orig-dst recovered (`127.0.0.2:18443`); server-side mTLS handshake completes
  TLS 1.3; client-auth VERIFIED; kTLS-RX armed.
- The 90-byte request is spliced to S in a single splice-in / splice-out; **S
  receives the byte-exact `CLIENT_REQUEST` as plaintext** — S holds no cert/key,
  is identity-unaware.

> **Evidence-label caveat (cosmetic, not a verification gap):**
> `client_cn=overdrive-spike-CA` is wrong — it should read `client.overdrive`.
> The label comes from a crude DER byte-scan (`describe_cert_cn`) that finds the
> first `CN` OID, which is the issuer (CA) CN, not the subject. This is a
> diagnostic-string artifact only; the **verification** is performed by rustls's
> `WebPkiClientVerifier` (that is what `MTLS_OK` / `MTLS_REJECTED` reflect), not
> by the DER scan. The fail-closed results in §4 prove the verifier is real.

### 3. Per-leg wire oracle (`tcpdump` on `lo`, `ok` mode) — ciphertext on the client leg, plaintext only on the agent→S leg

Client-facing leg = `127.0.0.1:50744 <-> 127.0.0.2:18443`;
agent→S leg = `127.0.0.1:48780 <-> 127.0.0.1:19777`.

```
=== CLIENT-FACING leg (127.0.0.2:18443) ===
  cleartext-marker-hits on CLIENT leg: 0      <-- request NEVER in cleartext on C's wire
  0x17(app_data) records on CLIENT leg: 2     <-- TLS 1.3 ciphertext on the wire

=== AGENT->S leg (127.0.0.1:19777) ===
  cleartext-marker-hits on AGENT->S leg: 1     <-- S receives decrypted plaintext (by design)
  0x17 records on AGENT->S leg: 0              <-- plaintext, not TLS
```

- **The application request is never visible as cleartext on the client leg**
  (the leg C uses); that leg carries TLS `0x17` app_data records — ciphertext.
- The decrypted plaintext appears **only** on the agent→S leg, exactly as
  intended (S is identity-unaware; the agent terminated TLS).

kTLS confirmation (`ss -tie`, client-facing leg):

```
tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw txconf: sw
```

The TLS ULP is installed on the client-facing socket (TLS 1.3 / AES-256-GCM);
`rxconf: sw` confirms software kTLS-RX is armed (loopback/virtio — no NIC
offload, expected).

### 4. Fail-closed — `nocert` and `wrongca` (the inbound authn boundary)

`nocert` (C presents no client cert):

```
AGENT_RESULT: MTLS_REJECTED reason=client-auth/handshake rejected: peer sent no certificates
AGENT: fail-closed — NO plaintext delivered to S (S should report NOTHING)
AGENT: server overran grace — killing by handle    <-- S never received a connection (bounded)
```

`wrongca` (C presents a cert from a different, untrusted CA):

```
AGENT_RESULT: MTLS_REJECTED reason=client-auth/handshake rejected: invalid peer certificate: BadSignature
AGENT: fail-closed — NO plaintext delivered to S (S should report NOTHING)
```

- Both reject **before any splice** — the agent never connects to S, so S
  receives nothing (S's bounded accept times out / is killed by the agent's
  grace; either way: 0 bytes delivered).
- The **distinct** rejection reasons (`peer sent no certificates` vs
  `invalid peer certificate: BadSignature`) prove the `WebPkiClientVerifier` is
  genuinely evaluating presence AND chain-to-bundle — not a vacuous gate.

### 5. Agent-light proof (`strace -f`, `ok` mode)

The agent's own (parent) pid, data-path syscalls only:

```
181150 splice(4, NULL, 7, NULL, 65536, SPLICE_F_MOVE|SPLICE_F_NONBLOCK)   <-- client_leg -> pipe
181150 splice(6, NULL, 5, NULL, 90,    SPLICE_F_MOVE|SPLICE_F_NONBLOCK)   <-- pipe -> S, 90 bytes
  splice  : 2
  ppoll   : 17
  recvfrom: 2        (handshake/control path, pre-kTLS — NOT payload)
  sendto  : 0   sendmsg : 0   recvmsg : 0

any read/write/recv/send returning ~90 bytes (the payload size)?
  -> ONLY fd 1/2 (stdout/stderr log lines). NONE on a socket fd.
```

- The 90-byte payload moves via a single `splice` pair (`client_leg_fd → pipe →
  to_S_fd`), kernel-to-kernel, gated by `ppoll` for readiness.
- The agent does **zero** `read`/`write`/`recv`/`send` of the payload bytes on
  any socket. The only ~90-byte `write`s are log lines to stdout/stderr.
- The 2 `recvfrom` calls are the TLS handshake control records (before kTLS-RX
  is armed), not the application payload.

This is **agent-light**: splice + ppoll for the payload, no per-byte userspace
copy of application data.

---

## Mechanics that mattered

1. **TPROXY orig-dst recovery is `getsockname()`, not `SO_ORIGINAL_DST`.** Under
   TPROXY (unlike REDIRECT/DNAT), the kernel keeps the intercepted socket's
   *local* address as the original destination, so `getsockname()` on the
   accepted fd returns `127.0.0.2:18443` directly. `SO_ORIGINAL_DST`
   (`getsockopt SOL_IP`) is the REDIRECT-fallback path and is not needed for
   TPROXY. The `IP_TRANSPARENT` (=19) sockopt on the listener is what lets it
   accept connections to non-local addresses; it needs `CAP_NET_ADMIN`/root.
2. **The ip-rule/route + nft-TPROXY triple is the whole intercept.** `ip rule
   add fwmark 0x1 lookup 100` + `ip route add local 0.0.0.0/0 dev lo table 100`
   route marked packets to local delivery; the nft `prerouting` (priority
   mangle) rule `ip daddr <virt> tcp dport <port> tproxy to 127.0.0.1:<agent>
   meta mark set 0x1` does the redirect+mark. On loopback this is a `filter`
   hook (not `nat`) — TPROXY is a filter-family target.
3. **Suppress NewSessionTicket on the server config.** `cfg.send_tls13_tickets
   = 0` — carried over from `findings.md` (raw kTLS-RX hits `-EIO` on a
   post-handshake ticket record). The server-side handshake must not emit a
   ticket after the kTLS-RX seq is fixed.
4. **`enable_secret_extraction` on BOTH sides.** The agent needs
   `dangerous_extract_secrets()` to read the negotiated RX (and TX) keys to arm
   kTLS; the client sets it too (harness symmetry; only the agent arms kTLS
   here).
5. **Arm RX with the extracted `rx.0` rec_seq.** The kTLS `TLS_RX` info struct
   takes the record sequence from rustls's extracted secrets (`rec_seq =
   secrets.rx.0.to_be_bytes()`); arming both `TLS_TX` and `TLS_RX` after
   `TCP_ULP=tls`. AES-256-GCM only (cipher id 52, version `0x0304`).
6. **Inspect the client cert BEFORE `dangerous_extract_secrets` consumes the
   connection.** `conn.peer_certificates()` is read for the fail-closed guard
   (and the CN label) prior to extracting secrets, since extraction moves the
   `ServerConnection`.
7. **Don't drop the client-leg `TcpStream` after the handshake** —
   `std::mem::forget(tcp)` in `server_handshake_and_arm`, because the caller
   owns the raw fd it captured and is about to splice from it. Dropping would
   `close()` the fd out from under the splice.
8. **The fail-closed deadlock was the structural hang.** With client-auth
   rejected, the agent never connects to S → S's `accept()` blocks forever →
   the agent's `server.wait()` blocks forever. Bounding **both** the accept and
   the child-wait is what makes the fail-closed path terminate.

---

## Design implications (for DESIGN, not promoted here)

1. **The inbound half is mechanically proven on 7.0 — F3's inbound mechanism
   holds.** Inbound TPROXY intercept + orig-dst recovery + server-side
   mutual-TLS terminate + kTLS-RX decrypt + agent-light splice-to-S all work on
   the pinned-LTS-class kernel, loopback, software kTLS. Combined with the
   proven outbound half (increment-f forward / increment-g return), both
   directions of the agent-light proxy have real-kernel evidence.
2. **Cost tier: agent-light (splice + ppoll), not agent-idle.** Same as the
   outbound spikes. The agent issues syscalls to move the payload but copies no
   payload bytes into userspace. An agent-idle inbound path (in-kernel
   decrypted redirect with no agent payload syscall) is foreclosed for the RX
   direction by increment-g's `tls_sw_read_sock -EINVAL` finding; for the
   inbound server, agent-light splice is the correct primitive.
3. **Orig-dst → identity selection.** The recovered original destination
   (`127.0.0.2:18443`) is what selects S's server identity (which SVID to
   present, which client-auth bundle to require). In production this is the
   address→workload-identity lookup the control plane owns; the spike hardcodes
   it.
4. **The DER-scan CN label must be replaced with a real X.509 parser** before
   any production use (the spike's label reports the issuer CN, not the subject
   CN). This is an evidence-label bug only — the actual client-auth verification
   is rustls's `WebPkiClientVerifier` and is correct.
5. **`IP_TRANSPARENT` + nft-TPROXY need `CAP_NET_ADMIN`.** The host-side agent
   runs privileged for the intercept setup; the workload S holds nothing and is
   unprivileged. Consistent with the workload-identity model (CLAUDE.md:
   "workloads hold NOTHING; the kernel/agent does mTLS").

---

## What was NOT tested (scope boundaries)

- **Real (non-loopback) NICs / hardware kTLS offload** — loopback + software
  kTLS only (`rxconf: sw`). Tier-3/Tier-4 concern.
- **The cgroup/network-namespace shape** a real workload S would run in — S here
  is a sibling process on the same loopback, not a netns-isolated workload. The
  intercept (nft prerouting on `lo`) and the splice-to-S would need re-proving
  in the real netns/veth topology.
- **Bidirectional steady-state** — only C→S (request) was driven. The S→C
  response leg (re-encrypt S's plaintext reply back onto the client leg's
  kTLS-TX) was not exercised; that is the kTLS-TX direction proven separately in
  increment-f, but composing it into this server shape is unproven.
- **Identity → SVID lookup / rotation / revocation** — all out of scope; the CA
  + leaves are minted in-process per run.
- **Phase-2 gate / promotion** — explicitly NOT run, per the spike's
  right-sizing.

---

## Reproduction

All commands timeout-bounded; run as root via `cargo xtask lima run --`
(build with `--no-sudo`). Throwaway dir:
`spike-scratch/increment-i-inbound-intercept/`.

```bash
# build (real-execution target; uses build.sh's split-verb to pass the cargo hook)
cargo xtask lima run --no-sudo -- timeout 240 bash -lc \
  'cd .../increment-i-inbound-intercept && bash build.sh'

# 1. isolated intercept + orig-dst probe (no TLS)
cargo xtask lima run -- timeout 60 bash -lc \
  'cd .../increment-i-inbound-intercept && timeout --kill-after=5 45 bash run-intercept-probe.sh'

# 2. full mTLS compose (ok mode) with wire capture
cargo xtask lima run -- timeout 90 bash -lc \
  'cd .../increment-i-inbound-intercept && MODE=ok CAPTURE=1 timeout --kill-after=5 70 bash run-full.sh'

# 3. fail-closed
cargo xtask lima run -- timeout 90 bash -lc \
  'cd .../increment-i-inbound-intercept && MODE=nocert  timeout --kill-after=5 70 bash run-full.sh'
cargo xtask lima run -- timeout 90 bash -lc \
  'cd .../increment-i-inbound-intercept && MODE=wrongca timeout --kill-after=5 70 bash run-full.sh'

# cleanup (idempotent; agent self-cleans, this is belt-and-suspenders)
cargo xtask lima run -- timeout 30 bash -lc \
  'cd .../increment-i-inbound-intercept && bash cleanup.sh'
```

The `agent` self-orchestrates (spawns S + C, accepts the intercepted leg,
handshakes, splices, reaps children, runs cleanup) and every blocking
accept/child-wait inside it is timeout-bounded, so it returns within a bounded
wall-clock regardless of mode. The `timeout` wrappers are the backstop.
