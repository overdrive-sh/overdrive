# Root-Cause Analysis — dial-by-name agent-leg dial to the stable frontend RSTs

**Investigator:** Rex (Toyota 5-Whys RCA)
**Date:** 2026-06-27
**Kernel under probe:** dev-Lima `7.0.0-22-generic` (merge gate is the pinned-6.18 Tier-3 matrix, ADR-0068)
**Scope:** the `S-DBN-WS` walking-skeleton Tier-3 test
`crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs::deployed_workload_resolves_peer_stable_frontend_and_hop_is_mtls`.

> HEADLINE (corrects the dispatch premise): `resolve(F)` is **NOT** the bug.
> `resolve(F)` correctly returns `Mesh(10.99.0.2)` — the `by_frontend`
> translation **works**. Confirmed at runtime: the worker reaches the
> `Enforce` arm, not `FailClosed`/`resolve_failed`. **All of the dispatch's
> hypotheses (a)–(e) are REFUTED by the captured worker verdict.** The dial
> RSTs at a *later* stage — the agent's **outbound mTLS enforce** (leg-B
> handshake to the resolved backend) fails with
> `received corrupt message of type InvalidContentType`, because the agent's
> host-originated leg-B dial **bypasses the destination workload's
> prerouting-hooked inbound TPROXY rule** and lands on the raw plaintext
> listener. This is a production *datapath architecture* gap (the
> outbound→inbound double-agent hop), not a resolve/`by_frontend` defect.

---

## Problem statement (scoped)

A deployed client resolves `server.svc.overdrive.local` → the stable frontend
`F = 10.98.0.1` (`getent` answers F). The agent-leg mTLS dial to
`(F, SERVICE_PORT=18951)` RSTs and the round-trip never completes. The 120 s
nextest timeout is a *downstream consequence* (the post-assert
`skeleton.shutdown()` never runs on the panic path → spawned boot-fixture tasks
keep the runtime alive), NOT the bug.

---

## Decisive runtime evidence (this investigation's probes)

Three `[DIAG]` probes were added to the test and run under Lima root on
kernel `7.0.0-22-generic`. All three are clearly marked `[DIAG]` for removal.

### Probe 1 — the live `service_backends` LWW winner at dial time

```
[02-02][DIAG] LIVE service_backends winner for server: service_id=ServiceId(6623880010248082302)
              backend.addr=10.99.0.2:18951 healthy=true (updated_at=LogicalTimestamp { counter: 3, ... })
```

The bridge's row stably advertises `10.99.0.2:18951 healthy=true` at dial time
— NOT the `10.96.0.1` host_ipv4 fallback. **Hypothesis (e-fallback) REFUTED**:
the layer-2 `workload_addr` forward-carry fix held; the bridge does not revert.

### Probe 2 — the worker's mTLS verdict event during the F dial

```
[02-02][DIAG] worker mTLS verdict events during/after the F dial:
[02-02][DIAG]   health.mtls.enforce_failed: { alloc: "alloc-client-0",
    error: "peer verification failed against the trust bundle:
            received corrupt message of type InvalidContentType",
    message: "mTLS enforce refused the connection (fail-closed; no cleartext)" }
```

NOT `health.mtls.outbound_fail_closed` (would be `MeshUnreachable`), NOT
`health.mtls.resolve_failed` (would be an `Err` from resolve). It is
`health.mtls.enforce_failed` — the worker reached the **`Enforce { peer }`
arm**, i.e. `resolve(F)` returned `Ok(Mesh(10.99.0.2))`. **Hypotheses
(a),(b),(c),(d),(e) all REFUTED** — `by_frontend` translated `F` to the live
backend correctly. The failure is in `enforce`, downstream of a *correct*
resolve. The `InvalidContentType` rustls error means the agent's leg-B CLIENT
handshake read **plaintext** where it expected a TLS ServerHello.

### Probe 3 — population diff (canonical vs F) + nft chain + route

```
[02-02][DIAG] canonical dial to 10.99.0.2:18951 from client netns ovd-ns-0001: rst=false, byte_exact=true
[02-02][DIAG] --- mTLS events AFTER canonical dial (...) ---   (NO enforce_failed: canonical enforce SUCCEEDED)
...
[02-02][DIAG] nft prerouting chain at F-dial time:
table ip overdrive-mtls {
  chain prerouting {
    type filter hook prerouting priority mangle; policy accept;
    meta mark 0x00000002 accept
    iifname "ovd-hv-0000" meta l4proto tcp tproxy to 127.0.0.1:37047 meta mark set 0x1 accept   # SERVER egress
    ip daddr 10.99.0.2 tcp dport 18951 tproxy to 127.0.0.1:41879 meta mark set 0x1 accept        # SERVER inbound
    iifname "ovd-hv-0001" meta l4proto tcp tproxy to 127.0.0.1:44553 meta mark set 0x1 accept   # CLIENT egress
    ip daddr 10.99.0.6 tcp dport 18952 tproxy to 127.0.0.1:35599 meta mark set 0x1 accept        # CLIENT inbound
  }
}
[02-02][DIAG] ip route get 10.99.0.2:
10.99.0.2 dev ovd-hv-0000 src 10.99.0.1 uid 0
```

This is the smoking gun. The whole `overdrive-mtls` chain is
`type filter hook **prerouting**`. The two interception shapes are:

- **egress** rule: `iifname "<host-veth>" … tproxy to 127.0.0.1:<legF>`
  (`install_outbound_tproxy`, matches workload traffic *as it ingresses* the
  host-side veth).
- **inbound** rule: `ip daddr <workload_addr> tcp dport <port> … tproxy to
  127.0.0.1:<legC>` (`install_inbound_tproxy`).

---

## The mechanism (fully grounded in the probe)

### Why the CANONICAL dial works (single agent hop)

Client (netns) → `connect(10.99.0.2:18951)`. The SYN ingresses the **client**
host-veth `ovd-hv-0001` and traverses `prerouting`. In chain order the
**server inbound rule** `ip daddr 10.99.0.2 tcp dport 18951` (appended when the
server deployed, *before* the client's egress rule) matches FIRST → diverts to
the **server's leg-C** `:41879`. The client's rustls dial therefore handshakes
**directly against the server's leg-C mTLS terminator** (which presents the
server SVID). One hop, captured by a `daddr`-matched prerouting rule. Works
(`rst=false, byte_exact=true`, and NO `enforce_failed` in the canonical window).

### Why the F dial fails (the double-agent hop the datapath can't make)

Client (netns) → `connect(10.98.0.1:18951)`. `10.98.0.1` matches **no** `daddr`
inbound rule (the only inbound rules are `10.99.0.2`/`10.99.0.6`), so it matches
ONLY the **client egress** rule `iifname "ovd-hv-0001"` → diverts to the
**client's leg-F** `:44553`. The client agent recovers `orig_dst=10.98.0.1`,
calls `resolve(10.98.0.1)` → `Ok(Mesh(10.99.0.2))` (`by_frontend` HIT — correct),
and runs **outbound enforce**: the agent's leg-B dials `10.99.0.2:18951`
(`crates/overdrive-dataplane/src/mtls/outbound.rs:82` `dial_leg(peer, …)`;
`mtls/mod.rs:612` `dial_leg` — **no `SO_MARK`**, an ordinary host socket).

`ip route get 10.99.0.2` shows that dial is **locally generated from the host
netns** (`src 10.99.0.1 dev ovd-hv-0000`). Locally-originated traffic traverses
the **`output`** hook, **never `prerouting`**. The `overdrive-mtls` chain is
`hook prerouting` only — so the agent's leg-B SYN to `10.99.0.2:18951` is **NOT
matched by the server inbound TPROXY rule**. It is delivered, unintercepted,
straight to the server workload's **raw plaintext Python listener** at
`10.99.0.2:18951`.

The agent then drives a rustls **CLIENT** handshake on leg-B
(`outbound.rs:154` `client_handshake` → `mtls/mod.rs` peer verification),
sending a ClientHello and reading the Python server's plaintext bytes as a TLS
record → rustls returns `received corrupt message of type InvalidContentType`
→ `MtlsEnforcementError` → `health.mtls.enforce_failed`
(`mtls_intercept_worker.rs:959-965`) → leg-F dropped → the workload's
connection RSTs.

**The canonical dial never exercised the double hop** — its `daddr` matched the
inbound rule directly. The F dial is the FIRST path that requires the
outbound→inbound *agent-to-agent* hop, and that hop's leg-B dial is
host-originated and structurally invisible to the prerouting-only inbound rule.

---

## 5-Whys chain

```
PROBLEM: the agent-leg dial to the stable frontend F RSTs; the round-trip never completes.

WHY 1 (symptom): the dialer observes a transport RST on the connection to (F, 18951).
  [Evidence: DialResult.observed_rst on the F dial; "dial handshake failed (agent leg)".]

WHY 2 (context): the agent's OUTBOUND mTLS enforce failed — leg-B peer handshake errored.
  [Evidence: captured event health.mtls.enforce_failed{alloc=alloc-client-0,
   error="...received corrupt message of type InvalidContentType"}. NOT fail_closed,
   NOT resolve_failed → resolve(F) returned Ok(Mesh) and the worker reached Enforce.]

WHY 3 (system): the agent's leg-B CLIENT handshake to the resolved backend
  10.99.0.2:18951 read PLAINTEXT, not a TLS ServerHello — the destination was the
  workload's raw plaintext listener, never an mTLS terminator.
  [Evidence: rustls InvalidContentType is the canonical "TLS client got cleartext"
   error; the server workload (server_service_spec) binds a raw Python TCP socket on
   0.0.0.0:18951 with no TLS.]

WHY 4 (design): the agent's leg-B dial (mtls/mod.rs:612 dial_leg → outbound.rs:82) is
  LOCALLY GENERATED from the host netns (ip route get 10.99.0.2 → src 10.99.0.1), so it
  traverses the `output` hook, while the destination workload's inbound interception
  (install_inbound_tproxy, mtls_intercept.rs:248: `ip daddr 10.99.0.2 dport 18951 tproxy
  to 127.0.0.1:<legC>`) lives ONLY in a `type filter hook prerouting` chain.
  [Evidence: the nft dump shows the whole overdrive-mtls chain is `hook prerouting`;
   locally-originated packets never hit prerouting. The leg-B dial is therefore
   un-intercepted and reaches the plaintext listener directly.]

WHY 5 (ROOT CAUSE): the OUTBOUND→INBOUND "double-agent" hop is unbuilt on the production
  datapath. The egress path (client → leg-F → resolve → leg-B) and the inbound path
  (peer → prerouting daddr rule → leg-C → leg-S) were each built and proven INDEPENDENTLY,
  but only ever exercised on a topology where the client's `orig_dst` IS the destination
  workload_addr (so a single `daddr`-matched prerouting rule captures the client's own
  packet). dial-by-name is the FIRST path whose orig_dst (the frontend F, 10.98.0.0/16) is
  NOT a workload_addr, so resolution forces the agent to RE-DIAL the backend — and that
  agent-originated re-dial is invisible to the prerouting-only inbound rule. No production
  mechanism steers the agent's host-originated leg-B dial into the destination's inbound
  TPROXY (no `output`-hook rule, no fwmark/route for agent-originated mesh dials, no
  same-agent local handoff).
  -> ROOT CAUSE: the production inbound interception is `hook prerouting` ONLY, and the
     agent's leg-B mesh re-dial is host-(locally-)originated — so the outbound-resolved
     mTLS hop is never terminated by the destination workload's inbound mTLS, on any path
     where the dialed orig_dst differs from the backend addr (i.e. every dial-by-name dial).
```

### Backwards-chain validation

If the root cause holds (leg-B host-originated, inbound rule prerouting-only),
then: a dial whose `orig_dst` == backend `workload_addr` (canonical) is captured
by the `daddr` prerouting rule on the *client's own* ingress and terminates in a
single hop — **it works** (matches the observed canonical success). A dial whose
`orig_dst` is the frontend `F` (≠ workload_addr) forces the agent to re-dial the
backend; that host-originated re-dial bypasses prerouting and hits the plaintext
listener — **it fails with InvalidContentType** (matches the observed F failure).
Both observed outcomes are produced by the single root cause. ✓ No contradiction.

---

## Hypothesis disposition (the dispatch's a–e, all REFUTED)

| Hyp | Claim | Verdict | Evidence |
|---|---|---|---|
| (a) | `by_frontend` drain not wired into `run_server` | REFUTED | worker reached `Enforce` (Probe 2) ⇒ resolve returned `Mesh`; lib.rs:2011-2031 construct+probe |
| (b) | divergent `<job>→F` source / key | REFUTED | one allocator (lib.rs:1931); byte-identical key (handlers.rs:347 = job_of = boot_rebuild; SUFFIX=svc.overdrive.local id.rs:846); resolve HIT |
| (c) | port/proto mismatch in the FrontendKey | REFUTED | resolve HIT to `Mesh(10.99.0.2:18951)`; bridge sets `addr.port()=listener.port=18951` (bdb.rs:364), proto `Tcp` both sides |
| (d) | pre-bind race (eager allocator read) withheld the key | REFUTED | resolve HIT (Probe 2) ⇒ `by_frontend` key present at dial time |
| (e) | by_frontend HIT but wrong/unhealthy/fallback backend | REFUTED | live row `10.99.0.2 healthy=true` (Probe 1); resolve → `Mesh(10.99.0.2)`; the *resolved* addr is correct and reachable (canonical dial to it succeeds byte-exact) |

The dispatch correctly localized the gap to "everything downstream of a correct
`Mesh` verdict is proven working" — but the decisive prior evidence (the
canonical `by_addr` dial) succeeded **because it never used the outbound→inbound
hop** (its `daddr` matched the inbound rule directly), which masked the real gap.
The new probe shows resolve IS correct and the gap is one layer further down,
in the datapath that connects a resolved outbound dial to the destination's
inbound mTLS terminator.

---

## The fix

> Per the dispatch constraints this is **specified, not implemented**. It is a
> *datapath/topology* change, NOT a one-line resolve fix — the dispatch's framing
> ("the single smallest production fix that makes `resolve(F)` return
> `Mesh(10.99.0.2)`") is moot: `resolve(F)` ALREADY returns `Mesh(10.99.0.2)`.
> The real fix must make the agent's host-originated leg-B mesh dial be
> intercepted-and-terminated by the destination workload's inbound mTLS.

### What must change (smallest correct cut)

The destination workload's inbound TPROXY rule must capture the agent's
**locally-originated** leg-B dial, which today only a `prerouting` rule cannot.
The smallest correct production change is to make the inbound interception fire
on locally-originated agent dials as well — concretely, add an **`output`-hook
companion** to the inbound TPROXY rule (or an equivalent local-redial steer) so
that an agent-originated SYN to `(workload_addr, port)` carrying NO leg-S
exemption mark is diverted to the destination's leg-C, exactly as the
prerouting rule diverts a peer's SYN.

- **Exact site:** `crates/overdrive-worker/src/mtls_intercept.rs`
  — `install_inbound_tproxy` (line 248) currently appends ONE rule to the
  `prerouting` chain (`NFT_CHAIN`). The fix adds a companion rule on an
  `output`-hook chain in the same `NFT_TABLE` matching the SAME
  `ip daddr <workload_addr> tcp dport <port>` and excluding the leg-S exemption
  mark (`meta mark != MTLS_LEG_S_DIAL_MARK`), `tproxy`/redirect to the same leg-C
  port. (`tproxy` is a prerouting-only verb; on `output` the equivalent is an
  `ip daddr … dport … meta mark set <fwmark>` + the existing fwmark `ip rule` →
  `local` route → `IP_TRANSPARENT` leg-C divert that `ensure_shared_routing_infra`
  already stands up. The exact `output`-hook steering shape must be pinned by the
  architect — see "Design gap to pin" below.)
- **Why the leg-S exemption already protects against recursion:** the agent's
  *inbound* leg-S dial to the workload is already `SO_MARK`-stamped
  `MTLS_LEG_S_DIAL_MARK` (`mtls/mod.rs:624` `dial_leg_s`) and the chain's first
  rule `meta mark 0x2 accept` exempts it. The agent's *outbound* leg-B dial is
  NOT stamped (`mtls/mod.rs:612` `dial_leg`), which is correct — it SHOULD be
  intercepted by the destination's inbound rule. So the `output`-hook companion
  must match the un-marked leg-B dial and skip the marked leg-S dial.

### Design gap to pin BEFORE implementing (do NOT improvise — CLAUDE.md "Implement to the design")

The inbound interception was designed as `hook prerouting` for *peer-originated*
traffic. Extending it to *agent-locally-originated* traffic (the leg-B re-dial)
is **new datapath surface** that the dial-by-name feature-delta / ADR-0072 did
not specify. The `tproxy` statement is prerouting-only; the `output`-hook path
needs the fwmark+`ip rule`+`local`-route+`IP_TRANSPARENT` shape, and whether
`ensure_shared_routing_infra`'s existing `ip rule`/route covers locally-generated
packets (they need `iif lo`-aware policy routing) is unverified. This is a
Tier-3-spike-class datapath question (mirrors the `.claude/rules/spike.md` "no
Tier-2 backstop for routing/nft mechanisms" hazard). **STOP and surface this to
the architect/orchestrator to pin the exact `output`-hook steering shape before a
crafter writes it** — reaching for the nearest nft verb that compiles is the
divergence this rule exists to prevent.

### Alternative (smaller-slice) framings to offer the user

1. **Same-agent local handoff (no new nft):** when `resolve` returns
   `Mesh(backend)` AND the backend is a *local* workload this agent also serves
   inbound, the outbound enforce could hand the connection to the *local* leg-C
   path in-process (skip the leg-B network dial entirely). Smaller blast radius;
   single-node-only; needs the worker to know "I own this backend's inbound."
2. **Re-size the walking skeleton** (per CLAUDE.md "Build vertical slices"): if
   the `output`-hook datapath is a separate slice, the thinnest dial-by-name loop
   that is live through `serve`+`deploy` may be name→F→resolve proven at the
   resolve boundary, with the full outbound→inbound mTLS hop deferred to the slice
   that builds the local-redial steer. (Requires user approval + a GH issue per
   the deferral rule — do NOT create unilaterally.)

The choice between (fix), (1), and (2) is a user/architect decision, not the
investigation's to make.

---

## Probes added (all `[DIAG]`, for orchestrator removal)

In `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs`:

- `use` block + `DiagCollector`/`DiagVisitor` tracing `Layer` (top of file, marked `[DIAG]`).
- `set_global_default(DiagCollector)` install after the crypto-provider install in the test.
- Probe 1: live `service_backends` LWW-winner dump before the F dial.
- Probe 2: `health.mtls.*` verdict dump after the F dial.
- Probe 3: canonical-window event dump + `nft list chain ip overdrive-mtls prerouting`
  + `ip route get <backend>` dump.

None alters production code or the test's assertions; each is a read-only
observability probe. They prove the verdict; remove them once the production fix
lands.
