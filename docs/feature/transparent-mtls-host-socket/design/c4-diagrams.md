# C4 diagrams — transparent mTLS universal agent-light L4 proxy (ADR-0069, GH #26)

Four diagrams (Mermaid). L1 System Context + L2 Container are mandatory; L3
Component is rendered TWICE for the proxy dataplane (a complex subsystem) — once
for the OUTBOUND/client path (detect→intercept→handshake→kTLS-arm→forward-encrypt-copy
→return-splice) and once for the INBOUND/server path
(TPROXY-intercept→orig-dst→server-mTLS→kTLS-RX→splice-to-server, F3). Every arrow
is labelled with a verb. Abstraction levels are not mixed. The per-direction
primitives AND the composed INBOUND flow are spike-proven (increment-i; outbound
primitives: increments-f/g; inbound: `findings-inbound-intercept.md`); **Slice 00
composes the remaining gaps** — outbound composed in one flow, the bidirectional
response legs, and real netns/veth + cgroup isolation (these three are NOT yet
proven).

---

## L1 — System Context

The actors and the systems the transparent-mTLS proxy touches. The workload is
identity-unaware and holds NOTHING; the operator declares policy. The "peer" is
another Overdrive workload **paired with its own node's agent** — the peer
workload holds nothing either; the peer's *agent* presents the peer workload's
SVID (this resolves the self-contradiction the prior diagram carried: the peer
workload does NOT present its own SVID — its agent does, on its behalf).

```mermaid
C4Context
  title System Context — Transparent mTLS (universal agent-light L4 proxy, bidirectional)
  Person(operator, "Platform/Security operator (Sam)", "Declares workloads + policy; verifies the wire with tcpdump / ss -tie")
  System(workload, "Host or guest workload", "Process/WASM/microVM/unikernel. Identity-unaware; holds NO key. Opens ordinary plaintext sockets (client) AND/OR is reached on its logical address (server).")
  System_Boundary(node, "Overdrive node") {
    System(proxy, "Transparent mTLS proxy (this node's agent)", "OUTBOUND: intercepts the workload's connect() (cgroup_connect4), client-handshakes presenting the workload SVID. INBOUND: TPROXY-intercepts connections to the workload's logical addr, server-handshakes presenting the workload SVID + verifies the client SVID, splices decrypted plaintext to the workload. Arms kTLS, splices steady-state.")
    System(identity, "IdentityMgr / IdentityRead", "Holds the per-allocation SVID + leaf key + trust bundle in memory")
  }
  System_Ext(peeragent, "Peer workload + its node's agent", "The peer workload holds NOTHING; the peer's AGENT presents the peer workload's SVID and verifies this side. The other half of the mTLS — never a TLS-aware workload.")

  Rel(operator, workload, "Deploys + sets policy for")
  Rel(workload, proxy, "Connects via / is reached via (transparently intercepted, both directions)")
  Rel(proxy, identity, "Reads held SVID + trust bundle from")
  Rel(proxy, peeragent, "Mutual TLS 1.3 with (this agent presents THIS workload's SVID; verifies the peer chains to the bundle)")
  Rel(operator, proxy, "Verifies the wire via tcpdump / ss -tie (TEST-tier observable)")
```

---

## L2 — Container

The deployment units inside the Overdrive node binary and the BPF/kernel surface.
The hexagon: the agent (control logic) depends on the `MtlsEnforcement` and
`IdentityRead` ports; production wires the host adapter, DST wires the sim adapter.

```mermaid
C4Container
  title Container Diagram — Transparent mTLS enforcement
  Person(operator, "Operator (Sam)")
  System(workload, "Workload (client and/or server)", "Plaintext socket; holds nothing")
  System_Ext(peeragent, "Peer workload + agent", "The peer's agent is the mTLS endpoint; peer workload holds nothing")

  Container_Boundary(node, "Overdrive node (single binary)") {
    Container(agent, "mTLS proxy agent", "overdrive-worker (adapter-host)", "Owns per-connection lifecycle BOTH directions: drive outbound client handshake (legs F+B), drive inbound server handshake (legs C+S), supervise the return/deliver splice pumps")
    Container(coreports, "Ports (traits)", "overdrive-core (core, no I/O)", "MtlsEnforcement (NEW) + IdentityRead (consumed)")
    Container(hostadapter, "HostMtlsEnforcement", "adapter-host", "OUTBOUND: connect4-intercept · capture · rustls CLIENT handshake · kTLS arm · forward read->write_all COPY pump (legF->legB kTLS-TX) · return zero-copy splice pump. INBOUND: TPROXY-intercept · getsockname orig-dst · rustls SERVER handshake + WebPkiClientVerifier · kTLS-RX arm · deliver zero-copy splice-to-server pump · response read->write_all COPY pump")
    Container(simadapter, "SimMtlsEnforcement", "overdrive-sim (adapter-sim)", "In-memory contract model for DST equivalence")
    Container(identity, "IdentityMgr", "overdrive-control-plane (adapter-host)", "Held SVID map + hydrated trust bundle; implements IdentityRead")
    ContainerDb(bpf, "BPF programs + maps", "overdrive-bpf (kernel)", "OUTBOUND: cgroup_connect4 mtls-variant (intercept) — the forward path is an agent-light splice pump, no sockops/sk_skb forward-redirect program (D-MTLS-13). INBOUND: nft TPROXY + IP_TRANSPARENT listener (server intercept).")
  }

  Rel(operator, agent, "Deploys workloads driving (no direct verb)")
  Rel(workload, bpf, "Connects (connect4 rewrites to agent) / is reached (TPROXY redirects to agent)")
  Rel(agent, coreports, "Drives enforcement + reads identity through")
  Rel(coreports, hostadapter, "Bound in production to")
  Rel(coreports, simadapter, "Bound in DST to")
  Rel(identity, coreports, "Implements IdentityRead")
  Rel(agent, identity, "Reads held SVID + bundle from (via IdentityRead)")
  Rel(hostadapter, bpf, "Loads + attaches + drives")
  Rel(hostadapter, identity, "Reads leaf key + bundle from (via IdentityRead)")
  Rel(hostadapter, peeragent, "Mutual TLS 1.3 with (outbound: originates as client; inbound: terminates as server)")
```

---

## L3 — Component (the proxy dataplane path — OUTBOUND / client side)

The per-connection enforcement path inside `HostMtlsEnforcement`:
detect → intercept → capture → handshake → kTLS-arm → forward-encrypt-copy
(agent-light `read → write_all` into kTLS-TX) → return-splice (agent-light zero-copy
out of kTLS-RX). leg F = the agent-owned plaintext leg facing the workload; leg B =
the agent-owned kTLS leg facing the peer.

```mermaid
flowchart TB
    W["Workload (plaintext socket, holds nothing)"]

    subgraph kernel["Kernel (BPF + TCP + kTLS)"]
        CONNECT4["cgroup_connect4 (mtls variant): rewrite connect() dest → agent listener"]
        LEGBKTLS["leg B kTLS: tls_sw_sendmsg → TLS 1.3 encrypt on splice-in (TX) · tls_sw_splice_read decrypt (RX)"]
    end

    subgraph adapter["HostMtlsEnforcement (adapter-host)"]
        ACCEPT["accept leg F (agent-owned plaintext leg)"]
        CAPTURE["drain pre-arm plaintext losslessly (recv → buffer)"]
        HS["rustls TLS 1.3 handshake on leg B (present held SVID via IdentityRead; verify peer vs bundle)"]
        ARM["arm kTLS on leg B (ktls::KtlsStream; setsockopt TCP_ULP tls + TLS_TX/TLS_RX)"]
        FLUSH["flush captured plaintext as first application_data (rec_seq=0)"]
        FSPLICE["forward pump: read(legF) -> write_all(legB kTLS-TX) COPY (per-record read+write, NOT a splice)"]
        SPLICE["return pump: splice(legB -> pipe -> legF) zero-copy ~1/record"]
    end

    IR["IdentityRead (svid_for + current_bundle)"]
    PEERAGENT["Peer-side agent / inbound proxy (the mTLS endpoint — presents the PEER workload's SVID)"]
    PEERW["Peer workload (plaintext socket, holds nothing — identity-unaware, behind its agent)"]

    W -->|"connect()"| CONNECT4
    CONNECT4 -->|"redirected to"| ACCEPT
    ACCEPT --> CAPTURE
    CAPTURE --> HS
    HS -->|"reads SVID + bundle"| IR
    HS -->|"TLS 1.3 records"| PEERAGENT
    HS --> ARM
    ARM --> FLUSH
    FLUSH -->|"encrypted"| PEERAGENT
    PEERAGENT -.->|"decrypted plaintext (peer agent splices to its workload)"| PEERW

    %% steady-state forward (agent-light COPY, NOT zero-copy)
    W ==>|"plaintext bytes"| ACCEPT
    ACCEPT ==>|"read(legF)"| FSPLICE
    FSPLICE ==>|"write_all into kTLS-TX (userspace copy)"| LEGBKTLS
    LEGBKTLS ==>|"TLS 1.3 encrypted (kernel tls_sw_sendmsg)"| PEERAGENT

    %% steady-state return (agent-light)
    PEERAGENT -->|"TLS 1.3 encrypted"| LEGBKTLS
    LEGBKTLS -->|"decrypted record"| SPLICE
    SPLICE -->|"plaintext"| W
```

**Reading the diagram**:

- **Setup (thin arrows)**: connect4 rewrites the workload's `connect()` to the
  agent's leg F; the agent accepts leg F, drains the pre-arm plaintext losslessly,
  handshakes on leg B (reading the held SVID via `IdentityRead`), arms kTLS, and
  flushes the captured bytes.
- **Steady-state forward (thick `==>` arrows) — AGENT-LIGHT but a COPY**: the agent
  drives a bounded `read(legF) → write_all(legB)` COPY into leg B's kTLS-TX; the
  kernel's `tls_sw_sendmsg` encrypts each blocking `write`. Per-record `read`+`write`,
  plaintext copied through a userspace buffer — **NOT zero-copy, NOT a splice** (a
  `splice` into kTLS-TX loses records). The agent runs no cipher; the kernel does the
  crypto. This is NOT symmetric to the return's zero-copy splice (D-MTLS-13 — this
  replaced an earlier sockmap EGRESS-redirect that `MSG_DONTWAIT`-stalled;
  `crates/overdrive-dataplane/src/mtls/splice.rs`,
  `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`).
- **Steady-state return (thin arrows from the peer-side agent) — AGENT-LIGHT**: leg
  B is a plain kTLS-RX socket (NO psock); the agent drives a
  `splice(legB → pipe → legF)` pump; `tls_sw_splice_read` decrypts each record into
  clean plaintext, zero-copy, ~1 splice/record (`findings-splice-return.md`). The
  TLS endpoint on the far side is the **peer-side agent** presenting the peer
  workload's SVID — never the peer workload itself, which holds nothing and sits
  behind its agent as identity-unaware plaintext.

**Invariant (Tier-3 test target)**: every reader leg (leg B's RX here, leg C's RX
inbound) carries NO sockmap verdict/psock — a psock both fights kTLS RX
(`ConnectionAborted`) and returns `tls_sw_read_sock` `-EINVAL` on a splice. The
RETURN pump is a zero-copy `splice` out of kTLS-RX; the FORWARD pump is a
`read → write_all` COPY into kTLS-TX (NOT a splice — a splice into kTLS-TX loses
records). Neither is a verdict redirect (D-MTLS-13 — the sockmap is gone on every
path); leg B carries no psock on either its RX or its TX.

---

## L3 — Component (the proxy dataplane path — INBOUND / server side, F3)

The inbound/passive half (proven in `findings-inbound-intercept.md`, increment-i):
TPROXY-intercept → orig-dst recovery → server-mTLS terminate → kTLS-RX arm →
splice-to-server (agent-light). leg C = the agent-owned client-facing kTLS leg
(the inbound analogue of leg B); leg S = the agent-owned plaintext leg facing the
server workload (the inbound analogue of leg F). The server workload holds
NOTHING and reads byte-exact plaintext.

```mermaid
flowchart TB
    PEERAGENT["Peer (client) workload's agent — presents the CLIENT SVID over TLS 1.3"]

    subgraph kernel["Kernel (nft TPROXY + TCP + kTLS)"]
        TPROXY["nft prerouting: ip daddr <server logical addr> tproxy to <agent> + ip rule fwmark + ip route local"]
        LEGCKTLS["leg C kTLS-RX: tls_sw_splice_read decrypts each TLS record → clean plaintext"]
    end

    subgraph adapter["HostMtlsEnforcement (adapter-host) — inbound"]
        ACCEPTC["accept leg C on IP_TRANSPARENT listener (agent-owned client-facing leg)"]
        ORIGDST["getsockname(legC) → ORIG_DST → AllocationId of the SERVER workload"]
        HSS["rustls SERVER handshake on leg C: present SERVER SVID (IdentityRead::svid_for); WebPkiClientVerifier REQUIRE+VERIFY the client SVID chains to the bundle"]
        ARMRX["arm kTLS-RX on leg C (suppress NewSessionTicket; read peer_certificates BEFORE extract_secrets)"]
        DELIVER["deliver pump: splice(legC → pipe → legS) ~1/record (agent-light)"]
    end

    IR["IdentityRead (svid_for + current_bundle)"]
    S["Server workload (plaintext socket, holds nothing) — reads byte-exact plaintext"]

    PEERAGENT -->|"TLS 1.3 to server logical addr"| TPROXY
    TPROXY -->|"redirected to"| ACCEPTC
    ACCEPTC --> ORIGDST
    ORIGDST -->|"selects server SVID"| IR
    ORIGDST --> HSS
    HSS -->|"reads server SVID + bundle"| IR
    HSS -->|"fail-closed on nocert/wrongca → NO splice to S"| S
    HSS --> ARMRX

    %% steady-state deliver (agent-light)
    PEERAGENT ==>|"TLS 1.3 encrypted request"| LEGCKTLS
    LEGCKTLS ==>|"decrypted record"| DELIVER
    DELIVER ==>|"plaintext"| S
```

**Reading the diagram**:

- **Intercept (thin arrows)**: `nft` TPROXY redirects the connection aimed at the
  server workload's logical address to the agent's `IP_TRANSPARENT` listener;
  `getsockname()` on the accepted leg-C socket recovers the original destination,
  which selects the server workload's `AllocationId` → its held SVID
  (`findings-inbound-intercept.md` §1).
- **Server-side mutual-TLS (thin arrows)**: the agent presents the server SVID and
  `WebPkiClientVerifier` requires-and-verifies the client SVID chains to the
  bundle. `nocert`/`wrongca` is fail-closed — nothing is spliced to the server
  workload (§2/§4).
- **Steady-state deliver (thick `==>` arrows) — AGENT-LIGHT**: leg C is a plain
  (no-psock) kTLS-RX leg; the agent drives `splice(legC → pipe → legS)`;
  `tls_sw_splice_read` decrypts each record into clean plaintext, zero-copy,
  ~1 splice/record (§3/§5). The server reads the byte-exact plaintext.

**Invariant (Tier-3 test target)**: same as outbound — leg C carries NO sockmap
verdict/psock on its RX; the deliver is a zero-copy `splice` out of kTLS-RX, not a
verdict redirect. The server's **response** leg (re-encrypt the server workload's
reply onto leg C's kTLS-TX) reuses the outbound forward primitive — the agent-light
`read(legS) → write_all(legC kTLS-TX)` COPY pump (D-MTLS-13; NOT a splice into
kTLS-TX, which loses records) — and is part of the composed walking-skeleton gate
(NOT exercised in the inbound spike).
