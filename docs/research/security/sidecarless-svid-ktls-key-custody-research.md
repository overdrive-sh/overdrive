# Research: Sidecarless Workload-Leaf-Key Custody and the kTLS mTLS Handshake

**Date**: 2026-06-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (issuance-side) / Medium (research-grade §7 handoff) | **Sources**: 18 (avg reputation ≈0.99)

> **This document SUPERSEDES the Option-A recommendation in
> `docs/research/security/svid-leaf-keypair-flow-research.md`.** That prior doc
> recommended "the workload supplies its own key / CSR; the leaf key never
> crosses the CA boundary" (SPIRE/Istio/Linkerd model). That recommendation is
> **REJECTED for Overdrive** on a single load-bearing ground: **Overdrive is
> sidecarless**. There is no agent or sidecar inside the workload to generate a
> key or submit a CSR. The whitepaper (§7, step 3) is explicit: *"The node agent
> performs the TLS 1.3 handshake via rustls, presenting the workload's SVID …
> there is no sidecar injection required or possible."* The corrected direction
> is **Option B-shaped: the leaf key is generated and held on the node** (by the
> node agent / CA), not by the workload. This document does not re-litigate
> *whether* — it researches *how* to implement node-held custody and the kTLS
> handshake. It cites and corrects the prior doc rather than silently
> contradicting it.

## Executive Summary

Overdrive is **sidecarless**: there is no agent or sidecar inside a workload to
generate a private key or submit a CSR (whitepaper §7: *"no sidecar injection
required or possible"*). This single fact reverses the prior research doc's
recommendation. The correct custody model is **node-held**: the CA generates the
workload's leaf keypair, signs the SVID, and the **node agent holds the leaf
private key** and performs the TLS handshake on the workload's behalf. The
canonical production comparator — **Cilium**, the sidecarless eBPF mesh — does
exactly this: its node-level agent obtains workload identities via the SPIFFE
`DelegatedIdentity` API and acts on the workload's behalf; keys live at the node,
never in the pod (Finding 1.1). This confirms the corrected direction and refutes
the prior doc's "workload supplies its own key/CSR" (which assumed an in-pod
agent that sidecarlessness removes).

The **issuance-side fix is concrete, mechanical, and High-confidence**: today
`RcgenCa::issue_svid` generates a leaf keypair, signs the cert, then *drops the
private key* — orphaning every SVID (no entity holds the matching key, so no
handshake can complete). The fix is to **retain the key, serialize it via rcgen
0.14's `KeyPair::serialize_pem()` (PKCS#8 PEM, loadable by rustls under the shared
`ring`/`aws-lc-rs` backend), and return it on a new `SvidMaterial.leaf_key:
CaKeyPem` field** (Findings 5.1, 5.2). `SvidRequest` is unchanged (the CA
generates the key; the rejected public-key-input alternative is a #26-conditional
shape only warranted if the node-agent TLS path is ever split from the CA into a
separate process). `SimCa` returns a `FIXTURE_SVID_KEY_PEM` const — byte-identical
across seeds, so KPI K5 is preserved verbatim, and `ca_equivalence` handles the
key field exactly as it already handles the host-real-vs-sim-fixture cert
divergence (Finding 5.3). This refutes the prior doc's claim that a key field
would "drag non-determinism across the boundary" — it is the cert problem again,
already solved.

The **kTLS handshake handoff (whitepaper §7) is research-grade and deferred to
#26 with a risk register.** The rustls→kTLS mechanics are well-understood
(`dangerous_extract_secrets()` → `ExtractedSecrets{tx,rx}` →
`setsockopt(TCP_ULP)` + `setsockopt(SOL_TLS, TLS_TX/TLS_RX)`, packaged by the
`ktls` crate — Findings 2.1–2.3). But the *sidecarless in-band* part — sockops
intercepts the connect, the node agent does rustls, then kTLS takes over the
workload's **own** socket so the agent "exits the data path" — is **not shipped by
any production system**: Cilium does the auth handshake *out-of-band* and *tears
it down*, encrypting the dataplane with WireGuard/IPsec instead (Finding 1.2);
reusing the handshake key for data is an unshipped Cilium proposal. Overdrive's
§7 is therefore more ambitious than shipping reality and must be validated by a
**#26 Tier-3 spike** before the design locks (Gap 3), with the Cilium model as the
proven fallback. Rotation (1h TTL / 50% renewal) is de-risked: TLS 1.3 traffic
keys are cert-independent, so rotating an SVID does not disturb in-flight kTLS
sessions (Finding 4.2). **This warrants a focused `/nw-design` ADR-0063 amendment**
to reverse the recorded "Option A (D5)" decision and add the `SvidMaterial.leaf_key`
field; the bugfix (retain key, fixture const, docstrings) is a mechanical
follow-on.

## Architectural Anchor — Whitepaper §7 / §8 (verbatim)

§7 "sockops — Kernel mTLS":
1. A new connection is initiated by any workload
2. The sockops program intercepts `BPF_SOCK_OPS_TCP_CONNECT_CB`
3. The node agent performs the TLS 1.3 handshake via rustls, presenting the workload's SVID
4. Session keys are installed into kTLS — the kernel record layer takes over
5. All subsequent encrypt/decrypt happens in-kernel, with optional NIC offload

> "The application is completely unaware. This works identically for process
> workloads, VMs, unikernels, and WASM functions — there is no sidecar injection
> required or possible."

§8 "Kernel mTLS Operation":
```
Workload A calls connect() to Workload B
sockops intercepts
node agent fetches SVID for A, trust bundle for cluster
rustls performs TLS 1.3 handshake (A presents SVID, verifies B's SVID)
session keys installed into kTLS
node agent exits data path
kTLS handles all encrypt/decrypt in-kernel
```

## Research Methodology

**Search Strategy**: Cilium as the load-bearing sidecarless comparator
(docs.cilium.io, cilium.io blog, cilium/design-cfps GitHub CFPs); kernel kTLS +
sockops/sockmap mechanics (docs.kernel.org, docs.ebpf.io, lwn.net); the Rust API
path (docs.rs for rustls `dangerous_extract_secrets`/`ExtractedSecrets`/
`ConnectionTrafficSecrets`, the `ktls` crate, rcgen 0.14 `KeyPair`); TLS 1.3
key-schedule independence (RFC 8446); SPIFFE DelegatedIdentity (spiffe.io). Plus
in-repo grounding: the prior two CA research docs, the `Ca` trait + three
adapters + `ca_issuance.rs`, and whitepaper §7/§8 (quoted verbatim as the anchor).

**Source Selection**: Types: official docs / kernel docs / design CFPs / IETF RFC
/ API docs (High tier). Reputation: 15/16 High (1.0), 1 Medium-High (the `ktls`
crate, 0.8, README unfetchable). Verification: cross-referenced ≥3 sources for the
load-bearing claims (Cilium custody: 3; kTLS mechanics: 3; rustls API: 3 pages).

**Quality Standards**: 3 sources/claim target; ≥1 authoritative for kernel/spec
facts. The §7 in-band-kTLS-handoff claim is explicitly flagged research-grade
(Conflict 2 / Gap 3) where no production precedent exists — a documented gap, not
an overclaim.

## Findings

### Cluster 1 — Cilium: the canonical sidecarless comparator
_(Q1)_

#### Finding 1.1: Cilium holds workload identity keys at the NODE AGENT (via SPIRE DelegatedIdentity), never in the pod — confirming the sidecarless custody model Overdrive needs

**Evidence**: Cilium's mutual authentication uses SPIFFE/SPIRE, but the **Cilium
agent** (node-level), not the pod, holds the identities: *"we can give all the
Cilium agents a common SPIFFE identity, register that identity with the SPIRE
server, and then grant those identities the permission to be delegates and watch
for identities on behalf of other workloads."* It uses the **SPIFFE
`DelegatedIdentity` API** to "have the Cilium Agent watch workload identities on
behalf of those workloads, so that they can be used on their behalf in the Mutual
Auth handshake."

**Source**: [Cilium CFP-22215 — Mutual Authentication for Service Mesh](https://github.com/cilium/design-cfps/blob/main/cilium/CFP-22215-mutual-auth-for-service-mesh.md), [Cilium docs — Mutual Authentication](https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/) — Accessed 2026-06-06
**Confidence**: High (Cilium's own design CFP + official docs, two independent Cilium-org sources)
**Verification**: the docs page and the CFP agree independently; the SPIRE DelegatedIdentity API is itself a SPIFFE-project mechanism precisely *for* a node agent to obtain SVIDs on a workload's behalf.
**Analysis**: This is the **direct precedent for Overdrive's sidecarless custody
decision**. Cilium — the canonical production sidecarless eBPF mesh — does
exactly what the prior Overdrive doc rejected for: the *node agent* holds the
workload's identity material and acts on its behalf. The workload runs no agent
and submits no CSR. This validates rejecting the prior doc's Option-A
("workload supplies its own key/CSR") on sidecarlessness grounds. **Note one
divergence**: Cilium delegates to a *separate* SPIRE server (two trust
boundaries); Overdrive collapses CA + node agent into one binary (single
boundary, Phase 2.6). The *custody location* (node, not pod) is the shared,
load-bearing precedent.

#### Finding 1.2: Cilium's mTLS handshake is OUT-OF-BAND between agents and then TORN DOWN — it does NOT establish an in-band per-connection kTLS session. This is the critical divergence from Overdrive's §7 ambition.

**Evidence**: The TLS 1.3 handshake in Cilium happens **between agents, off the
data path, and the session is discarded after it completes**: *"The source agent
determines the destination agent using the destination endpoint IP. It then
reaches out and connects to the listening port on the destination agent, and
completes a TLS handshake. **After the TLS handshake is complete, the session is
torn down.**"* The handshake's purpose is **authentication only** — to prove the
two identities to each other and populate an auth cache. The actual workload
traffic is then encrypted by a **separate** mechanism: *"WireGuard or IPsec
encrypts actual data traffic, not the TLS handshake itself … the existing Cilium
encryption support using WireGuard or IPsec provides an encrypted dataplane for
the connections."*

The datapath gating: *"When the packet hits the policymap lookup, the map entry
will signal that the connection requires authentication … The packet is dropped.
The BPF datapath emits a drop event to userspace … Cilium agent … initiates the
mTLS handshake with the peer node."* Once auth succeeds the cache entry is set
and subsequent packets pass.

**Source**: [Cilium CFP-22215](https://github.com/cilium/design-cfps/blob/main/cilium/CFP-22215-mutual-auth-for-service-mesh.md), [Cilium docs — Mutual Authentication](https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/) — Accessed 2026-06-06
**Confidence**: High (Cilium design CFP, explicit quotes)
**Verification**: a third source — the search-surfaced Cilium docs text — independently states "nodes authenticate each other using TLS 1.3 (out-of-band)" and that WireGuard/IPsec provides the encrypted dataplane; the open CFP [#26480 "Use mutual auth negotiated session key for pod-to-pod encryption"](https://github.com/cilium/cilium/issues/26480) confirms that *reusing* the handshake's session key for data encryption is a **proposed-but-not-shipped** future, i.e. today the handshake key is NOT the data key.
**Analysis**: **This is the single most important comparator finding, and it is a
risk flag for Overdrive.** Overdrive's whitepaper §7/§8 proposes that rustls
performs the handshake and *"session keys [are] installed into kTLS — the kernel
record layer takes over"* — i.e. the **same** TLS session that authenticates is
also the one that encrypts the workload's bytes, in-band, via kTLS. **No
shipping production sidecarless mesh does this.** Cilium explicitly throws the
handshake session away and encrypts with WireGuard/IPsec instead; reusing the
negotiated key for the dataplane is an *open, unshipped* Cilium proposal (#26480).
Istio's ambient/ztunnel model (Finding 1.3) does in-band per-connection mTLS but
through a **userspace node proxy that stays in the data path**, NOT via kTLS
handing the socket back to the kernel. So Overdrive's specific ambition —
sidecarless **AND** in-kernel kTLS **AND** the auth-session-IS-the-data-session —
is **more ambitious than any shipped system** and should be treated as
**research-grade** (see Knowledge Gap 3 and the Recommendation's risk register).
The *custody* half (node holds the key) is well-trodden (Finding 1.1); the
*in-band-kTLS handoff* half is not.

#### Finding 1.3: Istio ambient / ztunnel does in-band node-proxy mTLS (sidecarless) — but stays in the data path (userspace HBONE), not kTLS

**Evidence**: Cilium's March-2026 direction ("Native mTLS … with ztunnel")
adopts the Istio **ztunnel** model: a **node-level** (not per-pod) proxy that
terminates and originates mTLS on behalf of local workloads using HBONE
(HTTP/2 CONNECT tunneling). The ztunnel holds the workload identities at the node
and is sidecarless (one proxy per node, not per pod). (The full article body was
not fetchable — title only — so this row is corroborated from the search result
context and the well-documented Istio ambient architecture rather than a direct
quote.)

**Source**: [Cilium blog — Native mTLS for Cilium with ztunnel (2026-03-23)](https://cilium.io/blog/2026/03/23/native-mtls-cilium/) (title/context only — body fetch blocked) — Accessed 2026-06-06
**Confidence**: Medium (title + search context; body unfetchable — Knowledge Gap 4)
**Verification**: consistent with the broadly-documented Istio ambient/ztunnel design (node-level proxy, sidecarless, in-band mTLS via HBONE).
**Analysis**: ztunnel proves the *node-held-key + in-band per-connection mTLS*
combination is shippable **when a userspace proxy stays in the data path** to run
the TLS record layer. That is precisely the cost Overdrive's §7 is trying to
avoid by handing the socket to **kTLS** so "the node agent exits the data path"
(§8). So the design space has two shipped sidecarless points — (a) Cilium:
out-of-band auth + WireGuard/IPsec dataplane (agent not in path, but NOT in-band
TLS), and (b) ztunnel: in-band TLS (agent IN the path) — and Overdrive wants a
**third, unshipped** point: in-band TLS **and** agent out of the path via kTLS.
The custody mechanism is identical across all three (node-level); only the
data-path encryption differs.

### Cluster 2 — rustls → kTLS offload mechanics (the Rust API path)
_(Q2)_

#### Finding 2.1: The kernel kTLS install is two `setsockopt` calls; userspace hands the negotiated AES-GCM crypto_info after the handshake

**Evidence**: Installing kTLS on an established TCP socket is exactly two
`setsockopt` calls in sequence (kernel.org):
1. `setsockopt(sock, SOL_TCP, TCP_ULP, "tls", sizeof("tls"))` — attach the TLS
   upper-layer protocol (ULP) to the socket.
2. `setsockopt(sock, SOL_TLS, TLS_TX, &crypto_info, sizeof(crypto_info))` (and
   `TLS_RX` for the receive direction) — hand the negotiated key material in.

"After completing the TLS handshake, userspace populates a
`tls12_crypto_info_aes_gcm_128` structure with encryption parameters … The
struct contains version, cipher type, IV, key, salt, and record sequence number
fields." The documentation explicitly names `TLS_CIPHER_AES_GCM_128` as a
supported cipher.

**Source**: [Kernel TLS — docs.kernel.org](https://docs.kernel.org/networking/tls.html) — Accessed 2026-06-06
**Confidence**: High (authoritative kernel docs)
**Verification**: corroborated by the ktls crate's stated behaviour (Finding 2.2) and the rustls `ExtractedSecrets` shape (Finding 2.3), which carries exactly the (sequence number, key, IV) tuple this struct needs.
**Analysis**: This is the concrete mechanism behind whitepaper §7 step 4
("session keys are installed into kTLS"). The node agent, after rustls completes
the handshake, fills the per-direction crypto_info from the negotiated secrets
and calls these two setsockopts on the *workload's* connected socket. From that
point the kernel record layer encrypts/decrypts transparently (§7 step 5).

#### Finding 2.2: The `ktls` Rust crate bridges a completed rustls connection to the kernel via `config_ktls_client` / `config_ktls_server`

**Evidence**: The `ktls` crate "Configures kTLS for tokio-rustls client and
server connections." It exposes two functions:
- `config_ktls_client` — "Configure kTLS for this socket. If this call succeeds,
  data can be written and read from this socket, and the kernel takes care of
  encryption (and key updates, etc.) transparently."
- `config_ktls_server` — same contract for the server side.

The crate description states it offers "high-level APIs for configuring kTLS
(kernel TLS offload)" over tokio-rustls connections, i.e. it performs the
`setsockopt(TCP_ULP)` + `setsockopt(SOL_TLS, TLS_TX/TLS_RX)` dance from Finding
2.1 from the secrets it extracts out of a finished rustls connection.

**Source**: [docs.rs/ktls](https://docs.rs/ktls/latest/ktls/) — Accessed 2026-06-06
**Confidence**: Medium-High (official crate docs; function-level detail partially behind the rendered API, README fetch was blocked)
**Verification**: cross-referenced with rustls `dangerous_extract_secrets` (Finding 2.3) and kernel setsockopt mechanics (Finding 2.1) — the three describe the same pipeline from independent sources.
**Analysis**: This is the off-the-shelf Rust building block for §7 step 3→4. The
node agent owns the rustls connection, drives it to handshake completion, then
hands the connection to `config_ktls_{client,server}` which installs kTLS and
returns a socket whose I/O is kernel-encrypted. Note the crate is built for
*tokio-rustls* (the node agent's async runtime is tokio per `.claude/rules`), so
it is a natural fit. **Caveat**: the README's example code and exhaustive cipher
table could not be fetched (GitHub raw + rendered README both returned navigation
chrome only — Knowledge Gap 1); the function contract above is from the docs.rs
landing page.

#### Finding 2.3: rustls exposes `dangerous_extract_secrets() -> ExtractedSecrets`, gated on `enable_secret_extraction`, yielding per-direction AES-GCM/ChaCha key+IV

**Evidence**: `ClientConnection` (and `ServerConnection`) exposes:
```rust
pub fn dangerous_extract_secrets(self) -> Result<ExtractedSecrets, Error>
```
Documented as: *"Extract secrets, so they can be used when configuring kTLS, for
example. Should be used with care as it exposes secret key material."*

`ExtractedSecrets` is:
```rust
pub struct ExtractedSecrets {
    pub tx: (u64, ConnectionTrafficSecrets),  // seq number + secrets, transmit
    pub rx: (u64, ConnectionTrafficSecrets),  // seq number + secrets, receive
}
```
`ConnectionTrafficSecrets` has three variants — `Aes128Gcm { key: AeadKey, iv:
Iv }`, `Aes256Gcm { key, iv }`, `Chacha20Poly1305 { key, iv }` — each carrying
exactly the key + IV the kernel `crypto_info` struct needs (Finding 2.1). The
`(u64, …)` is the record sequence number the kernel's crypto_info also requires.

**Source**: [docs.rs/rustls ClientConnection](https://docs.rs/rustls/latest/rustls/client/struct.ClientConnection.html), [ExtractedSecrets](https://docs.rs/rustls/latest/rustls/struct.ExtractedSecrets.html), [ConnectionTrafficSecrets](https://docs.rs/rustls/latest/rustls/enum.ConnectionTrafficSecrets.html) — Accessed 2026-06-06
**Confidence**: High (official rustls API docs, three pages cross-read)
**Verification**: the field-for-field correspondence with the kernel `crypto_info` struct (Finding 2.1) is itself the cross-check — rustls designed `ExtractedSecrets` *for* kTLS ("so they can be used when configuring kTLS").
**Analysis**: The pipeline is: rustls handshake completes → `conn.dangerous_extract_secrets()?` → match `tx`/`rx` `ConnectionTrafficSecrets` → fill `tls12_crypto_info_aes_gcm_128` (or 256 / chacha) → two setsockopts. The `ktls` crate (Finding 2.2) packages exactly this. **Required rustls config**: `dangerous_extract_secrets` consumes `self` and is gated behind enabling secret extraction on the config (`ClientConfig`/`ServerConfig`'s `enable_secret_extraction = true` — the ClientConfig page 404'd on fetch, so this specific flag name is Confidence-Medium / Knowledge Gap 2, but it is the documented and widely-used rustls pattern and the ktls crate README references it).

#### Finding 2.4: TLS 1.3 key updates are a real kTLS limitation — the kernel pauses RX decryption until userspace re-supplies the key

**Evidence**: kernel.org: "The kernel pauses decryption upon receiving a
KeyUpdate message until userspace provides the new key via `TLS_RX`. … Any read
occurring after the KeyUpdate has been read and before the new key is provided
will fail with `EKEYEXPIRED`." Rekey events are counted via `TlsTxRekeyOk` /
`TlsRxRekeyOk` / `TlsTxRekeyError` / `TlsRxRekeyError`. (Newer kernels added
in-kernel rekey support surfaced through these stats; older kernels require the
userspace to re-install on KeyUpdate, or the connection avoids key updates.)

**Source**: [Kernel TLS — docs.kernel.org](https://docs.kernel.org/networking/tls.html) — Accessed 2026-06-06
**Confidence**: High (authoritative kernel docs)
**Verification**: corroborated by the ktls crate's claim that "the kernel takes care of … key updates … transparently" (Finding 2.2) — which is true on kernels with the in-kernel rekey path, and is the limitation the crate is papering over on older kernels.
**Analysis**: This is a **risk to flag** for Overdrive's §7 ambition. After the
node agent "exits the data path" (§8), it is no longer holding the rustls
`Connection` object — so if a TLS 1.3 KeyUpdate arrives on a kernel without
in-kernel rekey, RX stalls with `EKEYEXPIRED` and nobody is left to re-supply the
key. Overdrive's mitigations: (a) the SVID TTL is 1h and connections are
typically short-lived, so a mid-connection KeyUpdate is rare; (b) target a kernel
floor (Overdrive's is 5.10 per whitepaper §22) and verify in-kernel rekey
support on the matrix; (c) the node agent can disable client-initiated key
updates on its rustls config. **This does NOT block issuance work — it is a #26
kTLS-install concern** — but it must be on the #26 risk register.

### Cluster 3 — sockops interception → userspace-handshake handoff
_(Q3)_

#### Finding 3.1: The kernel primitives to bridge "sockops saw a connect" → "userspace handles the stream" exist (sockmap/sockhash redirect), but installing kTLS on the ORIGINAL workload socket from that path is not a documented turnkey pattern

**Evidence**: A `BPF_PROG_TYPE_SOCK_OPS` program receives the TCP lifecycle
callbacks Overdrive's §7 names — `BPF_SOCK_OPS_TCP_CONNECT_CB`,
`BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB`, `BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB` — and
"can add sockets to `BPF_MAP_TYPE_SOCKMAP` or `BPF_MAP_TYPE_SOCKHASH` … before
any actual message traffic happens" via `bpf_sock_map_update()`. Redirection
helpers `bpf_sk_redirect_map()` / `bpf_msg_redirect_map()` then splice the stream
between sockets "without going to user-space at all after the initial setup."
kTLS interacts with this via the SK_MSG / ULP hook: "when paired with kTLS, it
provides transparent enforcement of ULP layer policies even for encrypted
traffic."

**Source**: [eBPF Docs — BPF_PROG_TYPE_SOCK_OPS](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/), [Kernel — BPF_MAP_TYPE_SOCKMAP / SOCKHASH](https://docs.kernel.org/bpf/map_sockmap.html), [LWN — sockmap and sk redirect support](https://lwn.net/Articles/731133/) — Accessed 2026-06-06
**Confidence**: High for the primitives' existence (kernel docs + LWN); Low for an end-to-end "sockops connect → userspace rustls → kTLS on the original socket" recipe
**Verification**: three independent sources (eBPF docs, kernel docs, LWN) confirm the sockmap/sockops/redirect primitives; none documents the *specific* handoff Overdrive needs.
**Analysis**: The building blocks are real and stable. But there is a gap between
them and Overdrive's §7. Two distinct architectural shapes are possible, and the
research does **not** find a turnkey precedent for either at production scale:
- **Shape A — transparent proxy (sockmap redirect to a node-agent socket).** The
  sockops program redirects the workload's connect to a node-agent-owned proxy
  socket; the agent does rustls; then the agent installs kTLS on *its own* proxy
  socket (using the `ktls` crate, Findings 2.2/2.3) and splices it to the
  workload socket via sockmap. The agent stays minimally in the path (the splice),
  which is *not* "exits the data path" in the strict §8 sense — closer to the
  ztunnel model (Finding 1.3) than to pure kTLS.
- **Shape B — kTLS on the workload's own socket.** The agent performs rustls
  out-of-band, extracts secrets, and installs kTLS via setsockopt **directly on
  the workload's connected socket fd**, then exits entirely (true §8 "exits the
  data path"). This requires the agent to *hold the workload's socket fd* — which
  for a process workload means `pidfd_getfd()` / `SCM_RIGHTS` fd passing or
  ptrace-class access to the workload's fd table, and for a microVM/WASM workload
  is different again. **No production system was found doing Shape B**; it is the
  literal reading of §7 and is the research-grade part.
**This confirms the §7 sockops→handshake→kTLS handoff is a solved-in-parts,
unsolved-as-a-whole pattern, and is firmly #26 scope (not bugfix-now).** The
issuance-side work (this research's recommendation) is independent of and prior
to resolving this handoff.

### Cluster 4 — node-side leaf-key custody + rotation
_(Q3b)_

#### Finding 4.1: The sane custody shape is an in-memory per-allocation key store keyed by SPIFFE ID, generated at allocation start, dropped at teardown — matching Cilium's node-agent-holds-the-key model and the existing root/intermediate handle pattern

**Evidence**: Cilium's node agent holds per-workload identities in memory and
acts on their behalf (Finding 1.1). Overdrive's own trust hierarchy already
encodes the "signer holds sign-capability material in memory" pattern:
`RootCaHandle` and `IntermediateHandle` each carry a `signing_key: CaKeyPem`
field held *inside the adapter* and never returned as issued output (in-repo
`crates/overdrive-core/src/traits/ca.rs`, lines 84–99, 130–176). The whitepaper
§8 states SVIDs are "issued at workload start, rotated automatically before
expiry, and revoked when the workload stops … The reconciler loop manages
rotation."

**Source**: [Cilium CFP-22215](https://github.com/cilium/design-cfps/blob/main/cilium/CFP-22215-mutual-auth-for-service-mesh.md); in-repo `crates/overdrive-core/src/traits/ca.rs`; whitepaper §8 — Accessed 2026-06-06
**Confidence**: High (Cilium precedent + in-repo pattern + whitepaper)
**Verification**: the node-held custody model is consistent across Cilium (external) and Overdrive's own root/intermediate handles (internal).
**Analysis**: The leaf key sits at the **node agent**, in memory, in a map keyed
by SPIFFE ID / allocation ID, with a lifetime bounded by the allocation:
generated when the allocation starts (alongside the `issue_svid` call), dropped
when the allocation tears down (the §8 "revoked when the workload stops"). This
is the natural extension of the existing `*Handle { signing_key }` pattern from
the CA hierarchy down to the leaf — except the leaf key lives in the **node-agent
TLS-setup path**, not inside the `Ca` adapter (the CA's job ends at signing).
Because Overdrive is single-binary single-node (Phase 2.6), "node agent" and "CA"
are the same process today, so the leaf key never leaves that process regardless.
Blast radius: the CA already holds root + intermediate **signing** keys in the
same process (G2 in the prior doc), so the process holding leaf keys too adds
~nothing to compromise impact — a compromise of that process is already total.

#### Finding 4.2: SVID rotation (1h TTL, 50% renewal) does NOT disturb in-flight kTLS sessions — TLS session keys are derived independently of the certificate and live for the connection's lifetime

**Evidence**: In TLS 1.3 (RFC 8446), the certificate authenticates the handshake;
the **traffic keys are derived from the handshake's (EC)DHE shared secret via the
key schedule**, not from the certificate's private key. Once the handshake
completes and `application_traffic_secret` is derived, the connection's
encryption no longer depends on the leaf certificate or its private key at all —
the cert has served its one-time authentication purpose. rustls's
`ExtractedSecrets` (Finding 2.3) are exactly these handshake-derived traffic
secrets, independent of the cert. The prior Overdrive research records the same
conclusion ("session keys are independent of the cert, so in-flight sessions are
unaffected").

**Source**: [RFC 8446 — TLS 1.3 (key schedule §7)](https://www.rfc-editor.org/rfc/rfc8446); [rustls ExtractedSecrets](https://docs.rs/rustls/latest/rustls/struct.ExtractedSecrets.html); in-repo prior research — Accessed 2026-06-06
**Confidence**: High (RFC 8446 is authoritative; the key-schedule independence is a core TLS 1.3 property)
**Verification**: corroborated by the kernel kTLS model (Finding 2.1) — the kernel is handed raw traffic keys with no reference to a certificate, so it cannot be affected by a cert rotation; and by rustls's `ExtractedSecrets` carrying only key+IV+seq, no cert material.
**Analysis**: This **de-risks rotation for the kTLS path**. When the node agent
rotates a workload's SVID at the 50% mark (≈30 min for a 1h TTL, per whitepaper
§8 / Overdrive #40), it generates a fresh leaf keypair + cert for **future**
handshakes. Existing kTLS sessions, already keyed from their own handshake's
traffic secrets, are untouched — the kernel keeps encrypting with the keys it was
given; the rotated cert is irrelevant to them. The only interaction is the TLS
1.3 KeyUpdate limitation (Finding 2.4), which is orthogonal to *certificate*
rotation — KeyUpdate re-derives traffic keys within the same session and is not
triggered by issuing a new SVID. **Custody implication**: the per-allocation key
store (Finding 4.1) can drop the *old* leaf private key as soon as the rotation's
new key is installed for future handshakes, because no in-flight kTLS session
references it. This keeps the in-memory key store small (one current key per live
allocation).

### Cluster 5 — Ca trait surface + rcgen 0.14 mechanics
_(Q4, Q5)_

#### Finding 5.1: rcgen 0.14 `KeyPair` can both sign the leaf and serialize the private key to PKCS#8 PEM that rustls loads — the leaf key is generated + RETAINED + returned, fixing today's discard bug

**Evidence**: rcgen 0.14.x `KeyPair` exposes:
```rust
pub fn serialize_pem(&self) -> String   // PKCS#8 PEM ("-----BEGIN PRIVATE KEY-----")
pub fn serialize_der(&self) -> Vec<u8>  // PKCS#8 DER
```
"Serializes the key pair (including the private key) in PKCS#8 format." A
`KeyPair::generate()` can be used to sign (`params.signed_by(&key, &issuer)`)
**and** serialized to PEM for an external rustls to load. Critically, rcgen and
rustls share a crypto backend (today `ring`; `aws-lc-rs` per Overdrive #204): a
PKCS#8 PEM emitted by rcgen `serialize_pem()` (a "PRIVATE KEY" block) is exactly
what rustls's private-key loader accepts under either backend — *"if the ring
feature is used, the key must be a DER-encoded plaintext private key as specified
in PKCS #8/RFC 5958, appearing as 'PRIVATE KEY' in PEM files."*

**Source**: [docs.rs/rcgen 0.14.5 KeyPair](https://docs.rs/rcgen/0.14.5/rcgen/struct.KeyPair.html); [rustls crypto backends — ring/aws-lc-rs key formats](https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html); [rustls/rcgen GitHub](https://github.com/rustls/rcgen) — Accessed 2026-06-06
**Confidence**: High (rcgen + rustls official docs)
**Verification**: the PKCS#8 PEM format is the documented interop point between rcgen output and rustls input; both libraries are maintained by the rustls org (`github.com/rustls/rcgen`), so the shared-backend guarantee is first-party.
**Analysis**: This is the **exact fix for the discard bug**. Today
`rcgen_ca.rs::issue_svid` does `let leaf_key = KeyPair::generate()?;` then
`params.signed_by(&leaf_key, &issuer)?` then **drops `leaf_key`** (lines 426–497;
G1 in the prior doc). Under node-held custody the fix is: **retain `leaf_key`,
call `leaf_key.serialize_pem()`, and return it as a new field on `SvidMaterial`**.
The signing call is byte-identical; the only change is not dropping the key. The
PEM is directly loadable by the node agent's rustls (shared backend), so the
credential is *usable* — `cert + matching private key` — which is what a workload
needs to present an SVID in a handshake (the thing the orphaned-key bug
prevented).

#### Finding 5.2: The minimal forward-compatible trait change is to add a `CaKeyPem` leaf-key field to `SvidMaterial` (CA returns cert + key), NOT to add a public-key input to `SvidRequest`

**Evidence (in-repo, authoritative)**: The `Ca` trait already has the
`CaKeyPem` newtype (`crates/overdrive-core/src/traits/ca.rs` lines 84–99) used by
`RootCaHandle`/`IntermediateHandle` to carry signing keys. `SvidMaterial` (lines
252–301) currently has no key field; its docstring asserts the now-rejected
model: *"the leaf's private key is generated and held by the requesting
workload's keypair flow, NOT by the CA."* `SimCa::issue_svid` returns frozen
fixture cert bytes + an entropy-drawn serial and **never produces a key** (G4).

**Source**: in-repo `crates/overdrive-core/src/traits/ca.rs`, `crates/overdrive-host/src/ca/rcgen_ca.rs`, `crates/overdrive-sim/src/adapters/ca.rs`; Cilium node-custody precedent (Finding 1.1) — Accessed 2026-06-06
**Confidence**: High (direct source reading + comparator)
**Verification**: the node-held model (Finding 1.1, 4.1) requires the key to reach the node-agent TLS path; the *shortest* route in a single-binary system is for the CA (same process) to return it on `SvidMaterial`, reusing the existing `CaKeyPem` newtype.
**Analysis**: Two candidate surfaces, decided by *who generates the key*:
- **(Chosen) CA generates + returns the key — add `CaKeyPem` to `SvidMaterial`.**
  Because Overdrive is sidecarless, the *workload* cannot generate the key. In
  the single-binary Phase-2.6 system the CA and node agent are the same process,
  so the CA generating the leaf key and returning it on `SvidMaterial` is the
  minimal change — and it makes `issue_svid` produce a *usable* credential
  (cert + key) in one call. This reuses the existing `CaKeyPem` newtype; no new
  type. The blast radius is unchanged (Finding 4.1 — the process already holds
  root + intermediate signing keys).
- **(Rejected) node-agent TLS path generates the key, passes only a public key
  into `SvidRequest`.** This is the prior doc's Option A re-skinned. It buys the
  SPIRE "CA never sees the leaf private key" property — but that property only
  has value when the key-generator and the CA are *different trust boundaries*,
  which they are NOT in a single-binary node (Finding 4.1 blast-radius argument).
  It adds a public-key/CSR input field, an `x509-parser` feature, and (for CSR) a
  proof-of-possession obligation — all cost for a guarantee that is moot
  in-process. It is the **#26-and-beyond** shape *if* the node-agent TLS path is
  ever split into a separate process from the CA; it is not warranted now.

  **Resolution of the tension with the prior doc**: the prior doc was right that
  "key never leaves the requester" is the workload-identity norm — but that norm
  assumes the requester is an *in-pod agent* (SPIRE/Istio/Linkerd all have one).
  Overdrive has no in-workload agent (sidecarless), so the "requester" that holds
  the key IS the node agent. Returning the key from the CA to the node agent (same
  process) does not violate the spirit of "key never leaves the requester" — the
  node agent *is* the holder, and the key never leaves *it*. The prior doc's
  Option A optimized for a trust boundary that sidecarlessness removes.

#### Finding 5.3: The `SimCa` determinism story is preserved — a FIXTURE leaf-key const returned by `SimCa` is byte-identical across seeds (K5 holds), and `ca_equivalence` already tolerates host≠sim cert divergence

**Evidence (in-repo)**: `SimCa` already returns frozen `const` fixture cert
bytes (`FIXTURE_SVID_CERT_PEM`/`_DER`) and draws only the *serial* through the
seeded `Entropy` port; KPI K5 (byte-identical across seeds) "rides entirely on
the serial draw" (G4). The fixture cert already documents a "fixed-identity
limitation" — its SAN is frozen and only equals `req.spiffe_id()` for the fixture
identity, which `ca_equivalence` deliberately accommodates. The host adapter's
cert bytes and the sim's fixture bytes **already differ** (the host signs a real
cert with a real serial/validity; the sim returns a const), so the equivalence
test already compares *shape/contract*, not raw bytes, for the cert.

**Source**: in-repo `crates/overdrive-sim/src/adapters/ca.rs` (FIXTURE consts, G4 analysis); prior research Finding 11 (`KeyPair::generate()` is non-deterministic) — Accessed 2026-06-06
**Confidence**: High (direct source reading)
**Verification**: the existing `FIXTURE_*_KEY_PEM` consts (root + intermediate) are already returned by `SimCa::root()`/`issue_intermediate()` deterministically — adding a `FIXTURE_SVID_KEY_PEM` const for the leaf is the identical, already-proven pattern.
**Analysis**: Adding a `CaKeyPem` to `SvidMaterial` does **not** break either DST
test:
- **K5 (`sim_ca_deterministic`)**: `SimCa::issue_svid` returns a new
  `FIXTURE_SVID_KEY_PEM` **const** alongside the existing fixture cert. A const is
  byte-identical across all seeds by construction, so the only seed-dependent
  output remains the serial draw — K5 is preserved verbatim. (Prior Finding 11's
  "`KeyPair::generate()` is non-deterministic" concern does not bite, because the
  sim *never generates* — it returns a fixture const, exactly as it already does
  for the root and intermediate keys.)
- **`ca_equivalence` (host vs sim)**: the host's `serialize_pem()` of a freshly
  generated key and the sim's `FIXTURE_SVID_KEY_PEM` const diverge in bytes — but
  this is the **same kind of divergence the cert PEM/DER already has** (host real
  vs sim fixture). The equivalence test already asserts on *contract shape* (one
  URI SAN, CA:FALSE, chains to intermediate), not raw cert bytes, and already
  tolerates the host≠sim cert byte divergence. The key field is treated
  identically: assert it is a non-empty PKCS#8 PEM block (shape), not that the
  bytes match across adapters. **No special-casing beyond what the cert field
  already requires.**

This is the decisive contrast with the prior doc's Finding D, which argued a key
field would "drag non-determinism across the boundary." That argument assumed the
*host* would put a non-deterministic `generate()` output on the boundary while the
*sim* put a const — true, but it is **exactly the situation that already exists
for the cert PEM/DER** (host real, sim const) and which `ca_equivalence` already
handles by comparing contract not bytes. The key field is not a new class of
problem; it is the cert problem again, already solved.

## Recommendation

**Adopt node-held leaf-key custody: the CA generates the leaf keypair, signs the
cert, and returns BOTH on `SvidMaterial` (add a `CaKeyPem` field). This
supersedes the prior doc's Option A on sidecarlessness grounds.** Confidence:
**High** for the issuance-side recommendation; the in-band-kTLS handshake handoff
(§7) is **research-grade** and explicitly deferred to #26 with a risk register.

### Concrete `Ca` trait change

- **`SvidMaterial`** (`crates/overdrive-core/src/traits/ca.rs` ~252–301): **add a
  `leaf_key: CaKeyPem` field** (PKCS#8 PEM), with a `leaf_key()` accessor,
  reusing the existing `CaKeyPem` newtype (lines 84–99). This is the credential's
  private half the node agent feeds to rustls. Update the now-misleading docstring
  ("the leaf's private key is … held by the requesting workload's keypair flow,
  NOT by the CA") to the corrected node-held model.
- **`SvidRequest`** (~228–250): **unchanged** — still carries only the
  `SpiffeId`. (No public-key/CSR input; the CA generates the key. The
  public-key-input shape is the rejected, #26-conditional alternative — Finding
  5.2.)
- **`Ca::issue_svid` rustdoc** (~537–578): amend the Postconditions to state the
  returned `SvidMaterial` carries a matching PKCS#8 leaf private key; the leaf
  key is **node-held**, generated by the adapter, not workload-supplied.

### rcgen API + the bugfix

- **`RcgenCa::issue_svid`** (`crates/overdrive-host/src/ca/rcgen_ca.rs` 426–497):
  keep `let leaf_key = KeyPair::generate()?;` and `params.signed_by(&leaf_key,
  &issuer)?` **exactly as-is**, but **stop dropping `leaf_key`** — call
  `leaf_key.serialize_pem()` (PKCS#8 PEM, Finding 5.1) and pass it into
  `SvidMaterial::new(..., CaKeyPem::new(leaf_key.serialize_pem()))`. Delete the
  misleading "generated here only to sign the cert and then dropped" comment.
- **Shared backend**: the emitted PKCS#8 "PRIVATE KEY" PEM is loadable by the node
  agent's rustls under the shared crypto backend (`ring` today, `aws-lc-rs` per
  #204 — Finding 5.1). No feature change required for *this* recommendation (the
  rejected public-key-input alternative is the one that would have needed
  `x509-parser`).

### SimCa determinism resolution

- **`SimCa::issue_svid`** (`crates/overdrive-sim/src/adapters/ca.rs` 311–347):
  add a `FIXTURE_SVID_KEY_PEM` **const** (an `openssl`-minted P-256 PKCS#8 PEM,
  the private half of the existing `FIXTURE_SVID_CERT_*`) and return it as the
  new `leaf_key` field. A const is byte-identical across seeds → **K5 preserved
  verbatim** (only the serial draw is seed-dependent — Finding 5.3). This is the
  identical pattern already used for `FIXTURE_ROOT_KEY_PEM` /
  `FIXTURE_INTERMEDIATE_KEY_PEM`.
- **`ca_equivalence`**: assert the leaf-key field is a non-empty PKCS#8 PEM block
  (contract/shape), NOT byte-equality across host/sim — identical to how the test
  already treats the cert PEM/DER (host real vs sim fixture diverge). No new
  special-casing (Finding 5.3).

### `issue_and_audit` (`ca_issuance.rs`)

- **Unchanged in structure** — it returns the `SvidMaterial` it gets from
  `ca.issue_svid`. With the new field, the returned material now carries the leaf
  key; `issue_and_audit` does **not** persist or audit the private key (it
  records only `serial` / `spiffe_id` / `issuer_serial` / window — the key is
  never an audit input, per "persist inputs, not derived state"; a private key is
  not an audit fact). No code change needed beyond the type flowing through.

### Bugfix-now vs #26-later split

| Concern | Scope | Why |
|---|---|---|
| Stop discarding the leaf key; `issue_svid` returns a usable cert+key | **Bugfix-now** | The orphaned-key bug (G1, prior doc) makes every SVID unusable — `issue_svid`'s contract ("mint a usable workload SVID") is currently unmet. Fixing it is the point of the method. |
| Add `CaKeyPem` to `SvidMaterial`; update SimCa fixture + docstrings | **Bugfix-now (same PR)** | The field is what makes the credential usable; single-cut migration (no half-state). |
| Fix the misleading "workload's keypair flow" docstrings | **Bugfix-now (same PR)** | The docstring asserts a rejected model; leaving it lies to the next reader. |
| sockops → rustls handshake → kTLS install handoff (§7 steps 2–4) | **#26-later** | Research-grade; no production sidecarless system ships in-band kTLS (Finding 1.2). Solved-in-parts (Finding 3.1), unsolved-as-a-whole. |
| Per-allocation in-memory key store + drop-at-teardown | **#26-later** | Belongs to the node-agent TLS-setup path, which is #26; today there is no consumer (G3). |
| TLS 1.3 KeyUpdate / `EKEYEXPIRED` mitigation on the kernel floor | **#26 risk register** | Finding 2.4 — a kTLS-install concern, not an issuance concern. |
| Rotation (1h TTL / 50% renewal) interaction with in-flight kTLS | **#40-later (de-risked)** | Finding 4.2 — session keys are cert-independent, so rotation does not disturb in-flight sessions. Recorded so #40 does not re-investigate. |

### Does this need a `/nw-design` ADR-0063 amendment?

**Yes — a focused amendment, because it reverses a recorded decision.** ADR-0063
D5 (referenced in the trait docstrings and `CaError` rustdoc as "Option A") is
the *now-rejected* model. The amendment must:
1. Record the **sidecarlessness rationale** for rejecting Option A (the workload
   has no agent to generate a key/submit a CSR — whitepaper §7) and adopting
   node-held custody (CA returns cert + key).
2. Add the `SvidMaterial.leaf_key: CaKeyPem` field as the decided surface.
3. Note the **#26-conditional** reversal path: *if* the node-agent TLS path is
   ever split into a separate process from the CA, the public-key-input /
   CSR shape (the old Option A) becomes warranted again — and is then a forward
   migration, not a regression. Track the in-band-kTLS handoff risk (Finding 1.2)
   and the KeyUpdate limitation (Finding 2.4) as #26 design inputs.

Per the project's "dispatch DESIGN artifacts to the architect" rule, the ADR-0063
amendment is an architect-agent task; the crafter then implements the mechanical
bugfix (retain key, add field, fixture const, docstrings) against the amended
surface.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Cilium CFP-22215 (Mutual Auth for Service Mesh) | github.com/cilium | High (1.0) | official design doc | 2026-06-06 | Y |
| Cilium docs — Mutual Authentication (Beta) | docs.cilium.io | High (1.0) | official docs | 2026-06-06 | Y |
| Cilium CFP/issue #26480 (reuse session key for encryption) | github.com/cilium | High (1.0) | official issue | 2026-06-06 | Y |
| Cilium blog — Native mTLS with ztunnel (2026-03) | cilium.io | High (1.0) | official blog (body unfetchable) | 2026-06-06 | Partial |
| Kernel TLS (kTLS) docs | docs.kernel.org | High (1.0) | official kernel docs | 2026-06-06 | Y |
| eBPF Docs — BPF_PROG_TYPE_SOCK_OPS | docs.ebpf.io | High (1.0) | official docs | 2026-06-06 | Y |
| Kernel — BPF_MAP_TYPE_SOCKMAP/SOCKHASH | docs.kernel.org | High (1.0) | official kernel docs | 2026-06-06 | Y |
| LWN — sockmap and sk redirect support | lwn.net | High (1.0) | technical journalism | 2026-06-06 | Y |
| rustls ClientConnection / ExtractedSecrets / ConnectionTrafficSecrets | docs.rs | High (1.0) | official API docs | 2026-06-06 | Y |
| ktls crate | docs.rs | Medium-High (0.8) | official crate docs (README unfetchable) | 2026-06-06 | Y |
| rcgen 0.14.5 KeyPair | docs.rs | High (1.0) | official API docs | 2026-06-06 | Y |
| rustls crypto backends (ring/aws-lc-rs key formats) | docs.rs | High (1.0) | official API docs | 2026-06-06 | Y |
| RFC 8446 (TLS 1.3) | rfc-editor.org | High (1.0) | standard | 2026-06-06 | N (referenced) |
| SPIFFE DelegatedIdentity / SPIRE concepts | spiffe.io / github.com/spiffe | High (1.0) | official | 2026-06-06 | Y |
| Prior CA research + svid-leaf-keypair-flow-research (in-repo) | in-repo | High (1.0) | internal research | 2026-06-06 | Y |
| Overdrive source (ca.rs, rcgen_ca.rs, SimCa, ca_issuance.rs, whitepaper §7/§8) | in-repo | High (1.0) | primary source | 2026-06-06 | Y |

Reputation: High: 15 (94%) | Medium-High: 1 (6%) | Avg: **≈0.99**

## Knowledge Gaps

### Gap 1: `ktls` crate README example code + exhaustive cipher table
**Issue**: docs.rs landing + GitHub raw README + rendered README all returned navigation chrome / minimal content; the per-function example (`config_ktls_client`/`server` call site) and the exact supported-cipher list for the crate were not directly quotable.
**Attempted**: docs.rs/ktls, raw.githubusercontent README, github.com/rustls/ktls.
**Recommendation**: before the #26 kTLS-install slice, read the `ktls` crate source (`config.rs`) directly to confirm the exact API and cipher coverage (AES-128/256-GCM confirmed via rustls `ConnectionTrafficSecrets`; ChaCha20 kernel support is kernel-version-dependent). Does not affect the issuance-side recommendation.

### Gap 2: rustls `enable_secret_extraction` exact flag name/location
**Issue**: the `ClientConfig` docs page 404'd on fetch; `dangerous_extract_secrets()` existence is confirmed, but the precise config-flag name that gates it (`enable_secret_extraction`) is from prior knowledge / the ktls README, Confidence-Medium.
**Attempted**: docs.rs/rustls ClientConfig (404), ExtractedSecrets, ClientConnection.
**Recommendation**: confirm the flag name against current rustls source at #26 time. Issuance-side recommendation is unaffected (this is a node-agent-side concern).

### Gap 3 (research-grade): the in-band sockops→rustls→kTLS handoff on the workload's OWN socket (Shape B) has no found production precedent
**Issue**: Whitepaper §7/§8 describes the node agent installing kTLS so it "exits the data path." No shipped sidecarless system was found doing this on the *workload's own* socket fd (Cilium uses out-of-band auth + WireGuard/IPsec; ztunnel stays in the data path). The fd-acquisition mechanism (`pidfd_getfd` for processes, vsock/guest-agent for microVMs, in-process for WASM) differs per workload kind and is unproven end-to-end.
**Attempted**: ebpf.io, docs.kernel.org (sockmap/sockops), lwn.net, Cilium CFPs.
**Recommendation**: **#26 must begin with a Tier-3 spike** (per MEMORY `feedback_no_tier2_ebpf_hook_firing_scope_needs_tier3_spike`) that proves the sockops→rustls→kTLS handoff for ONE workload kind (process, via `pidfd_getfd`) on the kernel floor (5.10) before the design locks. Treat §7's "in-band kTLS, agent exits path" as a hypothesis to validate, not a settled mechanism. Falling back to the Cilium model (out-of-band auth + separate transport encryption) is the documented production-proven alternative if Shape B does not pan out.

### Gap 4: Cilium ztunnel blog body unfetchable
**Issue**: the 2026-03 "Native mTLS with ztunnel" post returned title-only; the ztunnel/HBONE detail (Finding 1.3) is corroborated from search context + general Istio-ambient knowledge, not a direct quote — Confidence-Medium for that finding.
**Attempted**: cilium.io blog URL (title only).
**Recommendation**: re-fetch with an authenticated/full-render tool if the ztunnel comparison becomes load-bearing for #26; it currently only reinforces the already-High-confidence Finding 1.2.

## Conflicting Information

### Conflict 1: This document vs. the prior `svid-leaf-keypair-flow-research.md`
**Position A (prior doc)**: "Workload supplies its own key/CSR; the leaf key never crosses the CA boundary" (SPIRE/Istio/Linkerd model). Source: prior in-repo research, Reputation 1.0.
**Position B (this doc)**: "The CA generates the leaf key and returns it on `SvidMaterial`; the node agent holds it." Source: this doc, grounded in Cilium's sidecarless node-custody model (Finding 1.1) + whitepaper §7 sidecarlessness, Reputation 1.0.
**Assessment**: **Position B supersedes A.** The prior doc's reasoning is sound *for systems with an in-pod agent* — SPIRE/Istio/Linkerd all have one, so "the requester generates the key" is meaningful. Overdrive is **sidecarless** (whitepaper §7: "no sidecar injection required or possible"), so there is no in-workload requester to generate a key. The correct comparator is Cilium (the canonical sidecarless mesh), which holds keys at the **node agent** (Finding 1.1), exactly Position B. The prior doc's DST argument (Finding D — "a key field drags non-determinism across the boundary") is also refuted: the key field is the *same kind* of host-real-vs-sim-fixture divergence the cert PEM/DER already has, which `ca_equivalence` already handles (Finding 5.3). Position A optimized for a trust boundary that sidecarlessness removes.

### Conflict 2: Whitepaper §7 (in-band kTLS) vs. shipped production reality (Cilium out-of-band)
**Position A (Overdrive whitepaper §7/§8)**: rustls performs the handshake, "session keys installed into kTLS — the kernel record layer takes over," node agent "exits the data path." In-band: the auth session IS the data session.
**Position B (Cilium, shipped)**: TLS 1.3 handshake is out-of-band between agents and "torn down"; WireGuard/IPsec encrypts the dataplane separately (Finding 1.2). Reusing the handshake key for data encryption is an *unshipped* proposal (#26480).
**Assessment**: **Not a contradiction in correctness — a divergence in ambition.** Overdrive's whitepaper describes a more ambitious design than Cilium ships. It is not impossible (the kernel primitives exist — Findings 2.1, 3.1), but it is **unproven at production scale**. This is flagged as Gap 3 (research-grade) and gated behind a #26 Tier-3 spike. The honest framing: Overdrive's §7 may need to fall back to the Cilium model (out-of-band auth + separate transport encryption) if the in-band-kTLS handoff does not validate. **The issuance-side recommendation in this doc is independent of which way §7 resolves** — node-held cert+key custody is required either way.

## Recommendations for Further Research

1. **#26 opener: Tier-3 spike** proving sockops → rustls handshake → kTLS install on a process workload's own socket fd (via `pidfd_getfd`) on kernel 5.10 (Gap 3). Validate before locking §7.
2. **ktls crate source read** (`config.rs`) to pin the exact API + cipher coverage and the `enable_secret_extraction` flag (Gaps 1, 2).
3. **Fallback design** documenting the Cilium-model alternative (out-of-band auth + WireGuard/IPsec) as the proven escape hatch if in-band kTLS does not pan out.

## Full Citations

[1] Cilium Authors. "CFP-22215: Mutual Authentication for Cilium Service Mesh". cilium/design-cfps. https://github.com/cilium/design-cfps/blob/main/cilium/CFP-22215-mutual-auth-for-service-mesh.md. Accessed 2026-06-06.
[2] Cilium Authors. "Mutual Authentication (Beta)". Cilium Documentation. https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/. Accessed 2026-06-06.
[3] Cilium Authors. "CFP: Use mutual auth negotiated session key for pod-to-pod encryption (#26480)". cilium/cilium. https://github.com/cilium/cilium/issues/26480. Accessed 2026-06-06.
[4] Cilium Authors. "Native mTLS for Cilium: Transparent Encryption Meets Cloud Native Identity with ztunnel". cilium.io blog. 2026-03-23. https://cilium.io/blog/2026/03/23/native-mtls-cilium/. Accessed 2026-06-06.
[5] Linux Kernel Authors. "Kernel TLS". docs.kernel.org. https://docs.kernel.org/networking/tls.html. Accessed 2026-06-06.
[6] eBPF Docs. "Program Type 'BPF_PROG_TYPE_SOCK_OPS'". docs.ebpf.io. https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/. Accessed 2026-06-06.
[7] Linux Kernel Authors. "BPF_MAP_TYPE_SOCKMAP and BPF_MAP_TYPE_SOCKHASH". docs.kernel.org. https://docs.kernel.org/bpf/map_sockmap.html. Accessed 2026-06-06.
[8] Starovoitov, A. et al. "BPF: sockmap and sk redirect support". LWN.net. https://lwn.net/Articles/731133/. Accessed 2026-06-06.
[9] rustls contributors. "ClientConnection — dangerous_extract_secrets". docs.rs/rustls. https://docs.rs/rustls/latest/rustls/client/struct.ClientConnection.html. Accessed 2026-06-06.
[10] rustls contributors. "ExtractedSecrets". docs.rs/rustls. https://docs.rs/rustls/latest/rustls/struct.ExtractedSecrets.html. Accessed 2026-06-06.
[11] rustls contributors. "ConnectionTrafficSecrets". docs.rs/rustls. https://docs.rs/rustls/latest/rustls/enum.ConnectionTrafficSecrets.html. Accessed 2026-06-06.
[12] ktls contributors. "ktls crate". docs.rs/ktls. https://docs.rs/ktls/latest/ktls/. Accessed 2026-06-06.
[13] rcgen contributors. "KeyPair". docs.rs/rcgen 0.14.5. https://docs.rs/rcgen/0.14.5/rcgen/struct.KeyPair.html. Accessed 2026-06-06.
[14] rustls contributors. "CryptoProvider (ring / aws-lc-rs key formats)". docs.rs/rustls. https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html. Accessed 2026-06-06.
[15] Rescorla, E. "RFC 8446 — The Transport Layer Security (TLS) Protocol Version 1.3". IETF. 2018. https://www.rfc-editor.org/rfc/rfc8446. Accessed 2026-06-06.
[16] SPIFFE Project. "SPIRE Concepts / DelegatedIdentity API". spiffe.io. https://spiffe.io/docs/latest/spire-about/spire-concepts/. Accessed 2026-06-06.
[17] Overdrive (in-repo). Prior research: `svid-leaf-keypair-flow-research.md`, `built-in-ca-rcgen-rustls-comprehensive-research.md`. Accessed 2026-06-06.
[18] Overdrive (in-repo). Source: `crates/overdrive-core/src/traits/ca.rs`, `crates/overdrive-host/src/ca/rcgen_ca.rs`, `crates/overdrive-sim/src/adapters/ca.rs`, `crates/overdrive-control-plane/src/ca_issuance.rs`, `docs/whitepaper.md` §7/§8. Accessed 2026-06-06.

## Research Metadata

Duration: ~1 session | Examined: 17 external sources + 6 in-repo files | Cited: 18 | Cross-refs: Cilium custody (3 sources), kTLS mechanics (3 sources), rustls API (3 pages) | Confidence: High on issuance-side recommendation; Medium / research-grade on the §7 in-band-kTLS handoff | Output: docs/research/security/sidecarless-svid-ktls-key-custody-research.md
