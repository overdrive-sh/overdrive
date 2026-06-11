# C4 diagrams — transparent mTLS universal agent-light L4 proxy (ADR-0069, GH #26)

Three levels (Mermaid). L1 System Context + L2 Container are mandatory; L3
Component is rendered for the proxy dataplane (a complex subsystem — the
detect→intercept→handshake→kTLS-arm→forward-splice→return-splice path). Every
arrow is labelled with a verb. Abstraction levels are not mixed.

---

## L1 — System Context

The actors and the systems the transparent-mTLS proxy touches. The workload is
identity-unaware; the operator declares policy; the peer is another Overdrive
workload.

```mermaid
C4Context
  title System Context — Transparent mTLS (universal agent-light L4 proxy)
  Person(operator, "Platform/Security operator (Sam)", "Declares workloads + policy; verifies the wire with tcpdump / ss -tie")
  System(workload, "Host or guest workload", "Process/WASM/microVM/unikernel. Identity-unaware; holds NO key. Opens ordinary plaintext sockets.")
  System_Boundary(node, "Overdrive node") {
    System(proxy, "Transparent mTLS proxy", "Intercepts the workload's TCP, handshakes with the peer presenting the workload's SVID, arms kTLS, splices steady-state in-kernel")
    System(identity, "IdentityMgr / IdentityRead", "Holds the per-allocation SVID + leaf key + trust bundle in memory")
  }
  System_Ext(peer, "Peer Overdrive workload", "Another workload's transparent-mTLS endpoint; presents its own SVID")

  Rel(operator, workload, "Deploys + sets policy for")
  Rel(workload, proxy, "Connects via (transparently intercepted)")
  Rel(proxy, identity, "Reads held SVID + trust bundle from")
  Rel(proxy, peer, "Originates mutual TLS 1.3 to (presenting the workload's SVID)")
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
  System(workload, "Workload", "Plaintext socket; holds nothing")
  System_Ext(peer, "Peer workload", "mTLS endpoint")

  Container_Boundary(node, "Overdrive node (single binary)") {
    Container(agent, "mTLS proxy agent", "overdrive-worker (adapter-host)", "Owns per-connection lifecycle: drive handshake, manage legs F+B, supervise return splice pump")
    Container(coreports, "Ports (traits)", "overdrive-core (core, no I/O)", "MtlsEnforcement (NEW) + IdentityRead (consumed)")
    Container(hostadapter, "HostMtlsEnforcement", "adapter-host", "Intercept · capture · rustls handshake · kTLS arm (ktls crate) · sockmap egress-redirect · splice(2) pump")
    Container(simadapter, "SimMtlsEnforcement", "overdrive-sim (adapter-sim)", "In-memory contract model for DST equivalence")
    Container(identity, "IdentityMgr", "overdrive-control-plane (adapter-host)", "Held SVID map + hydrated trust bundle; implements IdentityRead")
    ContainerDb(bpf, "BPF programs + maps", "overdrive-bpf (kernel)", "sockops (ESTABLISHED detect) · sk_skb/stream_verdict (forward egress redirect) · cgroup_connect4 mtls-variant (intercept) · SOCKHASH/SOCKMAP/ringbuf")
  }

  Rel(operator, agent, "Deploys workloads driving (no direct verb)")
  Rel(workload, bpf, "Connects (sockops fires; connect4 rewrites to agent)")
  Rel(agent, coreports, "Drives enforcement + reads identity through")
  Rel(coreports, hostadapter, "Bound in production to")
  Rel(coreports, simadapter, "Bound in DST to")
  Rel(identity, coreports, "Implements IdentityRead")
  Rel(agent, identity, "Reads held SVID + bundle from (via IdentityRead)")
  Rel(hostadapter, bpf, "Loads + attaches + drives")
  Rel(hostadapter, identity, "Reads leaf key + bundle from (via IdentityRead)")
  Rel(hostadapter, peer, "Originates mutual TLS 1.3 to")
```

---

## L3 — Component (the proxy dataplane path)

The per-connection enforcement path inside `HostMtlsEnforcement`:
detect → intercept → capture → handshake → kTLS-arm → forward-splice (agent-idle)
→ return-splice (agent-light). leg F = the agent-owned plaintext leg facing the
workload; leg B = the agent-owned kTLS leg facing the peer.

```mermaid
flowchart TB
    W["Workload (plaintext socket, holds nothing)"]

    subgraph kernel["Kernel (BPF + TCP + kTLS)"]
        SOCKOPS["sockops: ESTABLISHED detect → SOCKHASH + ringbuf event"]
        CONNECT4["cgroup_connect4 (mtls variant): rewrite connect() dest → agent listener"]
        VERDICT["sk_skb/stream_verdict on SOCKMAP: leg F RX → bpf_sk_redirect_map(B, flags=0 EGRESS)"]
        LEGBKTLS["leg B kTLS: tcp_sendmsg_locked → TLS 1.3 encrypt (TX) · tls_sw_splice_read decrypt (RX)"]
    end

    subgraph adapter["HostMtlsEnforcement (adapter-host)"]
        ACCEPT["accept leg F (agent-owned plaintext leg)"]
        CAPTURE["drain pre-arm plaintext losslessly (recv → buffer)"]
        HS["rustls TLS 1.3 handshake on leg B (present held SVID via IdentityRead; verify peer vs bundle)"]
        ARM["arm kTLS on leg B (ktls::KtlsStream; SOCKMAP-insert BEFORE TCP_ULP tls)"]
        FLUSH["flush captured plaintext as first application_data (rec_seq=0)"]
        SPLICE["return pump: splice(legB → pipe → legF) ~1/record"]
    end

    IR["IdentityRead (svid_for + current_bundle)"]
    PEER["Peer workload (mTLS endpoint)"]

    W -->|"connect()"| CONNECT4
    CONNECT4 -->|"redirected to"| ACCEPT
    W -.->|"ESTABLISHED"| SOCKOPS
    SOCKOPS -->|"event"| ACCEPT
    ACCEPT --> CAPTURE
    CAPTURE --> HS
    HS -->|"reads SVID + bundle"| IR
    HS -->|"TLS 1.3 records"| PEER
    HS --> ARM
    ARM --> FLUSH
    FLUSH -->|"encrypted"| PEER

    %% steady-state forward (agent-idle)
    W ==>|"plaintext bytes"| ACCEPT
    ACCEPT ==>|"leg F RX"| VERDICT
    VERDICT ==>|"egress redirect"| LEGBKTLS
    LEGBKTLS ==>|"TLS 1.3 encrypted"| PEER

    %% steady-state return (agent-light)
    PEER -->|"TLS 1.3 encrypted"| LEGBKTLS
    LEGBKTLS -->|"decrypted record"| SPLICE
    SPLICE -->|"plaintext"| W
```

**Reading the diagram**:

- **Setup (thin arrows)**: connect4 rewrites the workload's `connect()` to the
  agent's leg F; sockops fires the ESTABLISHED event; the agent drains the
  pre-arm plaintext losslessly, handshakes on leg B (reading the held SVID via
  `IdentityRead`), arms kTLS, and flushes the captured bytes.
- **Steady-state forward (thick `==>` arrows) — AGENT-IDLE**: leg F's RX is
  egress-redirected (`bpf_sk_redirect_map`, `flags=0`) into leg B's kTLS TX; the
  kernel's `tcp_sendmsg_locked` encrypts; the agent issues zero per-byte syscalls
  (`findings-egress-ktls-splice.md`, 15/15).
- **Steady-state return (thin arrows from PEER) — AGENT-LIGHT**: leg B is a plain
  kTLS-RX socket (NO psock); the agent drives a `splice(legB → pipe → legF)` pump;
  `tls_sw_splice_read` decrypts each record into clean plaintext, zero-copy, ~1
  splice/record (`findings-splice-return.md`).

**Invariant (Tier-3 test target)**: leg B carries NO sockmap verdict/psock on its
RX — that both fights kTLS RX (`ConnectionAborted`) and forecloses the return path
(`tls_sw_read_sock` `-EINVAL`). The return is `splice`, not a verdict redirect.
