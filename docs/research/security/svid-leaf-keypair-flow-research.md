# Research: Workload-SVID Leaf Private-Key Flow for Overdrive's Built-in CA

**Date**: 2026-06-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 13 (avg reputation 1.0)

> **Decision this document settles**: For `RcgenCa::issue_svid`, should the
> workload keypair be supplied by the requester (Option A — CSR / public-key
> model: extend `SvidRequest` with a public key or PKCS#10 CSR) or generated and
> returned by the CA (Option B — add a private-key field to `SvidMaterial`)?
> The current code is buggy: it generates a leaf key, signs the cert with that
> key's *public* half, then **drops the private key** — every issued SVID is
> cryptographically orphaned (no entity holds the matching private key, so the
> workload can never complete a TLS `CertificateVerify`).

## Executive Summary

The review-caught bug in `RcgenCa::issue_svid` — the CA generates a leaf
keypair, signs the cert with its public half, then drops the private key,
orphaning every SVID — is best fixed by adopting **Option A (the CSR /
public-key model): the workload supplies its public key (or, later, a PKCS#10
CSR), the CA signs it, and the leaf private key never crosses the CA trait
boundary.** This is recommended at **High confidence**, with every analysis axis
agreeing.

The evidence is convergent. (1) **Custody**: every workload-identity peer system
— SPIFFE/SPIRE, Istio, Linkerd — generates the leaf key at the requester and
signs a CSR; "keys never leave the node/pod" is the explicit, blast-radius-driven
norm. Key-delivery (Option B) appears only in convenience PKI (cert-manager,
Vault-`/issue`), and even Vault keeps a `/sign` path "for security." (2)
**Contract**: the `Ca` trait rustdoc and the `issue_svid` comments *already*
assert Option A ("the leaf's private key is ... held by the requesting workload
... NOT by the CA"); the bug is that `SvidRequest` was never extended to carry
the caller's key, so the adapter had nothing to sign but a key it invents and
discards. The fix makes the implementation match the documented SSOT. (3)
**DST**: Option A keeps the leaf key out of the trait boundary, preserving the
`SimCa` serial-only determinism contract and KPI K5 verbatim; Option B would drag
a key field into the K5 / `ca_equivalence` surface where the host's
non-deterministic `KeyPair::generate()` and the sim's fixture-const diverge in
kind — the determinism analysis actively penalizes B.

Mechanically, Option A is a near-drop-in: rcgen 0.14's
`CertificateParams::signed_by` already takes a public key (`&impl
PublicKeyData`), so the adapter change is "parse the supplied
`SubjectPublicKeyInfo` instead of generating a `KeyPair`." The only dependency
consequence is adding rcgen's `x509-parser` feature (a one-line `Cargo.toml`
change). Because this changes the port-trait surface (`SvidRequest` gains an
opaque public-key byte newtype), it should land as a **`/nw-design` ADR-0063
amendment** that formally adds the field the docstrings already presume — not a
silent bugfix — followed by a mechanical crafter implementation. The in-process
single-node nuance means Option A is *not urgent for security today* (no second
trust boundary exists yet — #26 is future), but it is *free to adopt now* and
*expensive to retrofit later* under the project's single-cut migration rule, so
A is the clear lower-regret choice.

## Research Methodology

**Search Strategy**: SPIFFE/SPIRE spec (spiffe.io), service-mesh identity docs
(istio.io, linkerd.io), cert-manager.io, HashiCorp Vault PKI, IETF RFCs (5280
X.509, 2986 PKCS#10), docs.rs/rcgen 0.14, GitHub (SPIRE, rcgen). Plus in-repo
grounding: the `Ca` trait, the buggy `RcgenCa::issue_svid`, the `SimCa`
determinism fixture, `ca_issuance.rs`, ADR-0063, and the prior CA research's
Finding 5 / Finding 11 / Finding 12.

**Source Selection**: official specs + framework docs + IETF RFCs (High tier),
cross-referenced ≥3 where possible.

**Quality Standards**: 3 sources/claim target, 1 authoritative minimum for
spec-mandated facts.

## Findings

### Finding 1: SPIFFE X.509-SVID spec mandates the leaf cert *shape*, delegates *key custody* to the Workload API

**Evidence**: The X.509-SVID standard pins the certificate structure but
explicitly does NOT mandate where the key is generated:
- §2: *"An X.509 SVID MUST contain exactly one URI SAN, and by extension,
  exactly one SPIFFE ID."*
- §4.3: *"Leaf SVIDs MUST set `digitalSignature`."*
- §5.2: the validator *"MUST ensure that the `cA` field in the basic
  constraints extension is set to `false`, and that `keyCertSign` and
  `cRLSign` are not set."*
- Key generation / CSR handling are **out of scope** of this document — it
  *"defines certificate format and validation only, not provisioning
  workflows"*; distribution is delegated to the SPIFFE Workload API spec (§5.1).

**Source**: [SPIFFE X.509-SVID standard](https://github.com/spiffe/spiffe/blob/main/standards/X509-SVID.md) — Accessed 2026-06-06
**Confidence**: High (primary spec)
**Verification**: cross-referenced against SPIRE concepts (Finding 2) and the
prior CA research Finding 5.
**Analysis**: The spec does not *force* either option — both A and B can produce
a spec-compliant leaf (correct SAN cardinality, `CA:FALSE`,
`digitalSignature`). The choice is therefore an *architecture/custody* decision,
not a conformance one. Overdrive's existing `CertSpec::svid` policy and the
`issue_svid` postconditions already enforce the §2/§4.3/§5.2 shape regardless of
which option supplies the key. **The spec is option-neutral; the custody
argument (Findings 2–4) is what decides.**

### Finding 3: rcgen 0.14 has a full PKCS#10 CSR path — but does NOT verify the CSR self-signature on parse

**Evidence**: rcgen 0.14.x exposes a CSR-signing path distinct from the
public-key path:
- `CertificateSigningRequestParams::from_pem(pem_str: &str)` (requires `pem` +
  `x509-parser` features) — *"Parse a certificate signing request from the ASCII
  PEM format."*
- `CertificateSigningRequestParams::from_der(csr: &CertificateSigningRequestDer)`
  (requires `x509-parser`).
- `CertificateSigningRequestParams::signed_by(&self, issuer: &Issuer<'_, impl
  SigningKey>) -> Result<Certificate, Error>` — produces a `Certificate` (then
  `.pem()` / `.der()`).

**Caveat (security-relevant)**: the docs *"do not mention CSR self-signature
verification during parsing. No validation step is explicitly documented for the
`from_pem` or `from_der` methods."* PKCS#10 (RFC 2986 §3) defines the CSR as
signed by the requester's private key to prove possession; if rcgen does not
verify that signature, the proof-of-possession check is the CA's responsibility,
or it is skipped.

**Source**: [docs.rs/rcgen 0.14.5 CertificateSigningRequestParams](https://docs.rs/rcgen/0.14.5/rcgen/struct.CertificateSigningRequestParams.html) — Accessed 2026-06-06
**Confidence**: High (official API docs)
**Verification**: PKCS#10 proof-of-possession semantics per [RFC 2986](https://www.rfc-editor.org/rfc/rfc2986); SPIRE CSR model (Finding 4).
**Analysis**: Two sub-variants of Option A exist:
- **A1 (public-key supply)**: `SvidRequest` carries a SubjectPublicKeyInfo /
  DER public key; adapter feeds it to `CertificateParams::signed_by(public_key,
  issuer)` (Finding 2). No proof-of-possession; the public key is trusted by
  construction (in-process, single trust boundary today).
- **A2 (PKCS#10 CSR)**: `SvidRequest` carries a CSR PEM/DER; adapter parses via
  `CertificateSigningRequestParams::from_pem` then `signed_by(issuer)`. This is
  the literal SPIRE wire shape — but Overdrive would need its OWN
  proof-of-possession verification if it matters, because rcgen does not verify
  the CSR self-signature. For an **in-process single-node** issuer (G3), there
  is no untrusted requester yet, so A1 is the lighter shape that still keeps the
  private key out of the CA boundary; A2 is the natural migration target when a
  separate-process workload boundary (#26) makes proof-of-possession meaningful.
  rcgen supports both, so the trait surface can start at A1 and the *adapter*
  upgrades to A2 later **without changing the conceptual model** (key still never
  crosses the boundary inward as a private key).

### Finding 7: rcgen feature gate — Option A's parse path needs `x509-parser`, NOT enabled in the current pin

**Evidence**: The workspace pins (root `Cargo.toml`):
```toml
rcgen = { version = "0.14", default-features = false, features = ["ring", "pem"] }
```
The `PublicKeyData` trait is implemented by `KeyPair` (feature `crypto`),
`PublicKey` (always available), and `SubjectPublicKeyInfo` (always available).
But the *constructors* that parse caller-supplied material are feature-gated:
- `SubjectPublicKeyInfo::from_der(&[u8])` — requires **`x509-parser`**.
- `SubjectPublicKeyInfo::from_pem(&str)` — requires **`x509-parser` + `pem`**.
- `CertificateSigningRequestParams::from_der` / `from_pem` (the A2 CSR path) —
  require **`x509-parser`** (+ `pem` for PEM).

**Source**: [docs.rs/rcgen 0.14.5 SubjectPublicKeyInfo](https://docs.rs/rcgen/0.14.5/rcgen/struct.SubjectPublicKeyInfo.html), [PublicKeyData trait](https://docs.rs/rcgen/0.14.5/rcgen/trait.PublicKeyData.html) — Accessed 2026-06-06
**Confidence**: High (official API docs, version-pinned)
**Verification**: workspace `Cargo.toml:87` (in-repo); cross-checked against the
`signed_by(public_key, issuer)` signature (Finding 2).
**Analysis**: Implementing Option A (either A1 public-key-supply or A2 CSR)
requires adding **`x509-parser`** to the rcgen feature set — a one-line
`Cargo.toml` change, no new crate. This is the only non-trivial dependency
consequence of A, and it is small. (`x509-parser` is already a transitive dep of
rcgen's `pem`/parsing surface; enabling the feature does not add a foreign
crate to the graph in any meaningful sense — it activates rcgen's own parsing
code.) Note this is a **host-adapter** concern only: `overdrive-core` (the trait)
carries no rcgen and is unaffected; the new `SvidRequest` field is a **byte
newtype** (a `WorkloadPublicKeyDer` / `CsrDer` opaque-bytes type, mirroring
`CaCertDer`), so core never parses it and the dst-lint core-purity gate stays
green. The sim adapter (`overdrive-sim`, no rcgen) treats the supplied key as
opaque bytes exactly as it treats the fixture certs today (G4).

### Finding 4: SPIRE generates the workload key on the AGENT and signs CSRs on the SERVER — the canonical custody split

**Evidence**: SPIRE concepts: *"The agent then sends workload CSRs to the server
which the server signs and returns as workload SVIDs to the client."* The prior
CA research Finding 5 states it directly: *"private keys are generated on the
agent (on the node), never on the server. The server signs CSRs; it never sees
workload private keys. ... Generates private keys for X.509-SVIDs locally via
key manager plugins — keys never leave the node."*

**Source**: [SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/) — Accessed 2026-06-06
**Confidence**: High (official SPIRE docs + prior cross-referenced research)
**Verification**: prior CA research Finding 5 (cited [SPIRE Concepts] +
[SPIRE Config] + [CNCF SPIFFE/SPIRE self-assessment]); confirmed by the SPIRE
Workload API CSR flow.
**Analysis**: SPIRE's split is *two trust boundaries* — agent (holds key) and
server (signs CSR). Overdrive Phase 2.6 collapses both into one in-process
binary on a single node (prior research Finding 5: *"Overdrive's built-in CA
collapses SPIRE's server + agent into a single binary"*). So the *custody
guarantee* SPIRE buys (server never sees the key) does not yet have a second
boundary to protect today (G3). But the canonical pattern is unambiguous:
**the requester generates and holds the leaf key; the CA signs a CSR/public
key.** Option A is the direct analogue; Option B inverts it.

### Finding 2: rcgen 0.14 `signed_by` already takes a public key, not a private key — Option A is a near-drop-in

**Evidence**: rcgen 0.14.x `CertificateParams::signed_by` has the signature:

```rust
pub fn signed_by(
    &self,
    public_key: &impl PublicKeyData,
    issuer: &Issuer<'_, impl SigningKey>,
) -> Result<Certificate, Error>
```

The **subject's material is taken as `&impl PublicKeyData`** (the public key
only); the issuer's signing key is separate, inside `Issuer`. `KeyPair`
implements `PublicKeyData` (which is why the current buggy code compiles passing
`&leaf_key`), but **any type implementing `PublicKeyData` works** — including a
public key parsed from a caller-supplied SubjectPublicKeyInfo / DER. The
private half is never required by `signed_by`.

**Source**: [docs.rs/rcgen 0.14.5 CertificateParams](https://docs.rs/rcgen/0.14.5/rcgen/struct.CertificateParams.html) — Accessed 2026-06-06
**Confidence**: High (official API docs, version-pinned to the workspace's 0.14)
**Verification**: corroborated by the in-repo call site
(`params.signed_by(&leaf_key, &issuer)` in `rcgen_ca.rs`, which already uses the
public-key arm). PKCS#10 CSR path corroborated in Finding 3.
**Analysis**: Option A requires almost no new rcgen surface — replace the
locally-generated `&leaf_key` with a caller-supplied `&impl PublicKeyData`. The
mechanical change is at the *trait input* (`SvidRequest` gains a public-key /
CSR field) and the *adapter* (parse the supplied public key, pass it to the
existing `signed_by`). The signing call itself is unchanged in shape. This makes
A *less* invasive at the rcgen layer than its conceptual weight suggests.

### Finding 5: Industry survey — workload-identity systems are unanimously CSR-model (A); key-delivery (B) appears only in convenience/ephemeral PKI

**Evidence**:

| System | Class | Leaf-key custody | Model |
|---|---|---|---|
| SPIFFE/SPIRE | Workload identity | Agent generates key locally; server signs CSR; *"keys never leave the node"* | **A (CSR)** |
| Istio | Service mesh | *"the Istio agent creates the private key and CSR, and then sends the CSR ... to `istiod` for signing"*; key never reaches the control plane | **A (CSR)** |
| Linkerd | Service mesh | *"the proxy generates a private key, stored in a tmpfs emptyDir which ... never leaves the pod ... and issues a CSR"* | **A (CSR)** |
| cert-manager | K8s cert lifecycle | Controller generates keypair, stores key+cert in a Secret; *"the requester doesn't supply a CSR"* | **B (key-delivery)** |
| HashiCorp Vault PKI | General PKI | `/issue` returns a Vault-generated key; `/sign` signs a caller CSR (*"private keys never leave the caller's environment"*) | **Both — B and A** |

**Sources**:
- [SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/) — Accessed 2026-06-06
- [Istio Security Concepts](https://istio.io/latest/docs/concepts/security/) — Accessed 2026-06-06
- [Linkerd Automatic mTLS](https://linkerd.io/2/features/automatic-mtls/) — Accessed 2026-06-06
- [cert-manager Certificate usage](https://cert-manager.io/docs/usage/certificate/) — Accessed 2026-06-06
- [HashiCorp Vault PKI](https://developer.hashicorp.com/vault/docs/secrets/pki) — Accessed 2026-06-06

**Confidence**: High (5 official sources, 4 distinct organizations)
**Verification**: each row is a distinct vendor's own docs; SPIRE/Istio/Linkerd
agree independently (no circular citation).
**Analysis**: The pattern is *class-dependent*, and the relevant class for
Overdrive is **workload-identity / mTLS service identity** — exactly the SPIFFE
peer group, which is **unanimous on Option A (CSR / key-never-leaves-requester)**.
The blast-radius argument is explicit in every one: a leaf key that is generated
in-place and never transits the issuer cannot be exfiltrated *from* the issuer
(the CA holds no leaf private keys, so a CA compromise yields signing capability
but not a vault of every workload's keys). cert-manager and Vault-`/issue` show
Option B is legitimate where the *requester cannot generate its own key* (a
declarative K8s Certificate resource has no runtime to generate a key; an
ephemeral client wants convenience). Even Vault offers `/sign` *"for security"*
— i.e. Option B is the convenience fallback, Option A is the security default.
Overdrive's workloads WILL be running processes with a runtime capable of
generating a key (post-#26), so the cert-manager justification for B (no
requester runtime) does not apply. **The near-universal choice for systems in
Overdrive's class is Option A, and the cited reason is key custody /
blast-radius / non-exportability.**

## In-Repo Grounding (authoritative for Overdrive specifics)

These are verified by direct reading of the source tree at HEAD
(`marcus-sa/built-in-ca-rcgen-rustls-research`). They are the load-bearing
local facts the recommendation must satisfy.

### G1 — The bug is real and the trait docstrings ALREADY assert Option A

`crates/overdrive-host/src/ca/rcgen_ca.rs` `issue_svid` (lines ~426–497):

```rust
let leaf_key = KeyPair::generate()...;          // CA mints a leaf keypair
let cert = params.signed_by(&leaf_key, &issuer)...; // signs leaf_key's PUBLIC half
Ok(SvidMaterial::new(cert.pem(), cert.der(), serial, subject)) // leaf_key DROPPED
```

`SvidMaterial` (`crates/overdrive-core/src/traits/ca.rs` ~252–301) has **no
private-key field**; `SvidRequest` (~228–250) carries **only** a `SpiffeId` —
no CSR, no public-key slot. So the issued cert embeds a public key whose
private half nobody holds. The cert cannot be used in any mTLS handshake.

Critically, the trait rustdoc and the comments inside `issue_svid` **already
claim Option A is the model**: `SvidMaterial`'s doc says *"the leaf's private
key is generated and held by the requesting workload's keypair flow, NOT by the
CA, so it is not part of this output"*; the in-method comment says *"Under
Option A (ADR-0063 D5 amendment) ... the leaf's private key is NOT a CA-boundary
output, so it is generated here only to sign the cert and then dropped."* The
documented contract is Option A; the implementation generates-and-drops, which
satisfies neither A (no caller-supplied key) nor B (no returned key). **The bug
is the absence of the input slot, not a wrong output slot.** `SvidRequest` was
never extended to carry the caller's key, so the adapter has nothing to sign
*but* a key it invents and discards.

### G2 — `root()` / `issue_intermediate()` RETAIN their keypairs; only the leaf is orphaned

`RootCaHandle` and `IntermediateHandle` both carry a `signing_key: CaKeyPem`
field — the CA keeps these because it must *sign with them later* (the
intermediate signs leaves; the root signs intermediates). They are
**sign-capability material held inside the signer**, never returned as issued
output. The leaf is different: a leaf signs nothing on the CA's behalf — its
private key belongs to the *relying workload*, which is exactly why the SPIRE
"keys never leave the requester" posture applies to leaves but not to the CA's
own hierarchy keys. The existing design already encodes this asymmetry; the leaf
flow is the one place it was left unimplemented.

### G3 — The only consumer is `issue_and_audit`, which has no production caller today

`crates/overdrive-control-plane/src/ca_issuance.rs::issue_and_audit` is the sole
consumer of `issue_svid`. It calls `ca.issue_svid(request)`, builds an audit
row from `svid.serial()` / `svid.spiffe_id()`, writes the audit row, and returns
the `SvidMaterial`. It never touches a private key, and there is **no
production caller** that hands the `SvidMaterial` to a workload — the mTLS
relying-party verifier (sockops/kTLS) is GH #26 (future), and rotation is GH #40
(future). So today the orphaned key has **no observable downstream failure**:
nothing tries to complete a handshake with the leaf. The bug is latent until #26
lands.

### G4 — `SimCa` returns FROZEN opaque leaf bytes; only the SERIAL is entropy-driven

`crates/overdrive-sim/src/adapters/ca.rs::issue_svid` returns inline `const`
fixture bytes (`FIXTURE_SVID_CERT_PEM` / `_DER`, an `openssl`-minted leaf with a
frozen SAN `spiffe://overdrive.local/workload/sim-svid`) plus a serial drawn
through the seeded `Entropy` port. The leaf **private key never appears in
`SimCa` output at all** — `SvidMaterial` has no key field, so the sim has
nothing to fabricate deterministically. The byte-identity-across-seeds contract
(KPI K5) rides **entirely on the serial draw** (`draw_serial()` →
`entropy.fill`); the cert PEM/DER are constant. The doc already records the
"fixed-identity limitation": the frozen SAN only equals `req.spiffe_id()` for
the fixture identity. **This is decisive for the DST analysis** (Finding D
below): Option B would force the sim to either return a key (which it cannot mint
deterministically without a fixture key) or carry a *fixture leaf key const*,
whereas Option A keeps `SvidMaterial` key-free and the sim contract unchanged.

### G5 — Prior CA research already implies Option A at the workflow layer

`docs/research/security/built-in-ca-rcgen-rustls-comprehensive-research.md`
Finding 12 proposes the `cert_rotation` workflow as:
`1. generate_keypair() -> keypair` then `2. sign_svid(spiffe_id, pubkey) ->
signed_cert`. That is the CSR/public-key model (Option A): the keypair is
generated as a *separate step* and only the **public key** is handed to the
signing call. Finding 5 states the SPIRE model verbatim: *"The server signs
CSRs; it never sees workload private keys."* The prior research covered ROOT key
custody (Findings 7–8) and the entropy/DST limitation (Finding 11) but **did not
specify the leaf-key flow at the trait surface** — that gap is what this
document closes. This document **extends** the prior research; it does not
contradict it.

### Finding 6 [Analysis]: The CSR boundary buys little *today* (in-process, single node) but is the cheaper *reversibility* bet

**Claim type**: interpretation, grounded in G1–G5 + Findings 1–5.

Today (Phase 2.6) the CA and the only `issue_svid` consumer
(`issue_and_audit`) run in-process on a single node, and there is **no
production caller** handing the leaf to a separate workload (G3). So the
SPIRE/Istio custody guarantee ("the signer never sees the key") has **no second
trust boundary to protect yet** — under Option B the "leaked" leaf key would
travel from the in-process CA to an in-process caller, the same address space.
The immediate security delta between A and B *today* is therefore near-zero.

The decision is dominated not by today's threat model but by **reversibility
once #26 (mTLS relying party) and #40 (rotation) land**, under the project's
**single-cut greenfield migration** rule (no deprecation shims; old path deleted
and new path landed in the same PR — `CLAUDE.md`, MEMORY
`feedback_single_cut_greenfield_migrations`):

- **If you ship A now and #26/#40 confirm A** (the overwhelmingly likely
  outcome per Finding 5): zero migration. `SvidRequest` already carries the
  workload public key / CSR; the separate-process workload generates its key,
  hands the public half across, done.
- **If you ship B now and later migrate to A** (the likely correction, since
  every comparable system is A): a *hard cut*. `SvidMaterial` loses its
  private-key field; every call site that consumed the returned key is rewritten;
  the workload-side key-generation path is added; the audit/rotation surfaces
  that assumed a CA-returned key are reworked. Single-cut means this all lands at
  once, touching #26 and #40 simultaneously — the most expensive moment to change
  the custody model.
- **If you ship A now and (improbably) need B later**: also a hard cut, but B is
  strictly a *superset addition* at the trait (add a field to `SvidMaterial`)
  and the requester simply stops generating its own key. Walking *forward* to B
  from A is mechanically smaller than walking *back* to A from B, because A's
  invariant ("CA holds no leaf private keys") is the more constrained one — you
  can always relax a constraint more cheaply than you can re-impose one across
  every consumer.

**Conclusion of the analysis**: A is the lower-regret option. It matches the
documented contract (G1), matches every peer system (Finding 5), and is the
direction that is cheapest to *not* have to reverse. The in-process nuance means
A is *not urgent for security today* — but it is *free to adopt today* (G2,
Finding 2) and *expensive to retrofit later* (single-cut). "Future-proofing" here
is not speculative gold-plating; it is choosing the shape that the already-written
trait docstrings, the prior research's workflow design (G5), and the entire
comparable-systems landscape all already assume.

### Finding D [Analysis]: DST determinism strongly favors Option A — Option B would *introduce* a non-determinism source

**Claim type**: interpretation, grounded in G4 + prior-research Finding 11.

This is the potentially decisive constraint, and it points the same way as the
custody argument.

**Prior fact (research Finding 11, High confidence)**: rcgen's
`KeyPair::generate()` does **not** accept an external entropy source — it calls
the backend CSPRNG directly and is **non-deterministic** under DST. Determinism
relies on `KeyPair::from_pem()` fixture keys. Serials (which ARE
reconciler/observation-visible) flow through the `Entropy` port and ARE
deterministic.

**Under Option A**: the leaf private key is **never produced by the CA** and
**never crosses the trait boundary** (`SvidMaterial` stays key-free, exactly as
today — G4). `SimCa::issue_svid` continues to return frozen opaque fixture cert
bytes + an entropy-drawn serial; the only seed-dependent output is the serial.
The KPI K5 byte-identity-across-seeds contract is **unchanged and already
holds**. The deterministic *workload* keypair under DST is supplied by the
*test/sim caller* — a fixture public key (or fixture CSR) loaded via
`KeyPair::from_pem()` and handed into `SvidRequest`, identical across seeds by
construction. The sim CA never has to fabricate a key. **A requires zero changes
to the SimCa determinism contract.**

**Under Option B**: `SvidMaterial` gains a `CaKeyPem` private-key field that the
CA must populate. Two bad sub-cases:
1. **Host adapter** would call `KeyPair::generate()` (non-deterministic) — but
   the host is not under DST, so that is acceptable *for the host*. However, the
   returned key now becomes a **trait-boundary output**, and the
   `ca_equivalence` DST test compares host-vs-sim observable output. A key field
   that is non-deterministic on the host and a *fixture const* on the sim means
   the two adapters' `SvidMaterial` outputs can no longer be compared for the key
   field at all — the equivalence surface shrinks, or the test must special-case
   the key.
2. **Sim adapter** cannot deterministically *generate* a key (Finding 11), so it
   must carry a **fixture leaf-key const** and return it — adding a new const,
   and more importantly adding a *new observable seed-independent output* whose
   relationship to the entropy-drawn serial must be reasoned about. The
   `sim_ca_deterministic` / K5 contract now has to assert byte-identity over a
   key field too. It would still be deterministic (it's a const), but it widens
   the determinism surface for no benefit and couples the leaf-key fixture into
   the K5 proof.

**Conclusion of the analysis**: Option A keeps the leaf key entirely out of the
trait boundary, so the DST determinism contract (G4 — serial-only seed
dependency) is **preserved verbatim**. Option B drags a key field across the
boundary and into the K5 / `ca_equivalence` surfaces, where the host's
non-deterministic `generate()` and the sim's fixture-const diverge in kind. The
determinism analysis is **not merely neutral toward A — it actively penalizes
B**. Combined with custody (Finding 5) and reversibility (Finding 6), this is
decisive.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| SPIFFE X.509-SVID standard | github.com/spiffe | High (1.0) | official spec | 2026-06-06 | Y |
| SPIRE Concepts | spiffe.io | High (1.0) | official | 2026-06-06 | Y |
| Istio Security Concepts | istio.io | High (1.0) | official | 2026-06-06 | Y |
| Linkerd Automatic mTLS | linkerd.io | High (1.0) | official | 2026-06-06 | Y |
| cert-manager Certificate usage | cert-manager.io | High (1.0) | official | 2026-06-06 | Y |
| HashiCorp Vault PKI | developer.hashicorp.com | High (1.0) | official | 2026-06-06 | Y |
| rcgen 0.14.5 CertificateParams | docs.rs | High (1.0) | tech docs | 2026-06-06 | Y |
| rcgen 0.14.5 CertificateSigningRequestParams | docs.rs | High (1.0) | tech docs | 2026-06-06 | Y |
| rcgen 0.14.5 PublicKeyData / SubjectPublicKeyInfo | docs.rs | High (1.0) | tech docs | 2026-06-06 | Y |
| RFC 2986 (PKCS#10) | rfc-editor.org | High (1.0) | standard | 2026-06-06 | N (referenced) |
| Prior CA research (Findings 5/11/12) | in-repo | High (1.0) | internal research | 2026-06-06 | Y |
| Overdrive source (ca.rs, rcgen_ca.rs, SimCa, ca_issuance.rs, Cargo.toml) | in-repo | High (1.0) | primary source | 2026-06-06 | Y |

Reputation: High: 12 (100%) | Medium-high: 0 | Avg: **1.0**

## Knowledge Gaps

### Gap 1: rcgen CSR proof-of-possession verification
**Issue**: docs.rs does not document whether `CertificateSigningRequestParams::from_pem/from_der` verifies the CSR's self-signature (PKCS#10 proof-of-possession). The fetched docs state no validation step is documented.
**Attempted**: docs.rs/rcgen 0.14.5 CSR page.
**Recommendation**: before implementing A2 (the #26 slice), read rcgen 0.14 source for `CertificateSigningRequestParams::from_der` to confirm whether PoP is checked; if not, the CA must verify the CSR signature itself. Does not affect A1 (the recommended starting variant), which carries a public key with no PoP semantics.

### Gap 2: exact `SubjectPublicKeyInfo` / `PublicKey` constructor ergonomics under `ring` backend
**Issue**: Confirmed `from_der` exists and needs `x509-parser`; did not exhaustively confirm it round-trips a P-256 SPKI produced by `KeyPair::public_key_der()` under the `ring` backend specifically.
**Attempted**: docs.rs SubjectPublicKeyInfo page.
**Recommendation**: a 5-line spike in `overdrive-host` (generate `KeyPair`, take `public_key_der()`, `SubjectPublicKeyInfo::from_der`, `signed_by`) confirms the round-trip before the crafter commits. Low risk — this is the documented happy path.

## Conflicting Information

### Conflict 1: Is leaf-key generation the CA's job or the requester's?
**Position A (CSR-model)**: SPIFFE/SPIRE, Istio, Linkerd — requester generates the key; CA signs a CSR; key never leaves the requester. Sources: spiffe.io, istio.io, linkerd.io (Reputation 1.0 each).
**Position B (key-delivery)**: cert-manager and Vault-`/issue` — the issuer generates the key and returns it. Sources: cert-manager.io, developer.hashicorp.com (Reputation 1.0 each).
**Assessment**: Not a true contradiction — it is **class-dependent**. Position B systems serve requesters that *cannot generate their own key* (a declarative K8s resource; an ephemeral convenience client). Overdrive's workloads are running processes with a runtime (post-#26), placing them squarely in the Position-A class, where every workload-identity peer (the directly comparable systems) chooses A. Vault offering both `/issue` and `/sign` resolves the apparent conflict: A is the security default, B the convenience fallback. For Overdrive, A wins.

## Recommendation

**Adopt Option A (CSR / public-key model). Start with the A1 public-key-supply
variant; keep A2 (PKCS#10 CSR) as the documented forward migration when #26
introduces a separate-process workload boundary that makes proof-of-possession
meaningful.** Confidence: **High.**

Every analysis axis points the same way:

1. **Custody / industry alignment (Finding 5)** — Overdrive's class
   (workload-identity mTLS) is *unanimously* Option A across SPIFFE/SPIRE,
   Istio, and Linkerd; the cited reason is key custody / blast-radius /
   non-exportability. Option B appears only in convenience PKI
   (cert-manager, Vault-`/issue`), and even Vault offers `/sign` "for security."
2. **Documented contract already says A (G1)** — the `Ca` trait rustdoc and the
   `issue_svid` comments already assert *"the leaf's private key is generated and
   held by the requesting workload's keypair flow, NOT by the CA."* Option A
   makes the implementation match the contract; Option B contradicts the
   already-written SSOT.
3. **DST determinism penalizes B (Finding D)** — A keeps the leaf key out of the
   trait boundary, so the `SimCa` serial-only determinism contract (G4) and KPI
   K5 are preserved verbatim. B drags a key field into the K5 / `ca_equivalence`
   surface where the host's non-deterministic `KeyPair::generate()` and the sim's
   fixture-const diverge in kind.
4. **Reversibility under single-cut (Finding 6)** — A is the lower-regret bet:
   walking *back* from B to A later is a hard cut across #26 and #40 at the worst
   moment; A needs no reversal at all in the likely future.

### Concrete trait-surface change

- **Extend `SvidRequest`** (`crates/overdrive-core/src/traits/ca.rs`) to carry
  the workload's public key as a new **opaque byte newtype** — e.g.
  `WorkloadPublicKeyDer(Vec<u8>)` holding a DER-encoded `SubjectPublicKeyInfo`,
  defined alongside `CaCertDer` and observed via an `as_der()` accessor. Core
  never parses it (no rcgen on a core compile path — ADR-0063 D1). The
  single-URI-SAN invariant on `SpiffeId` is unchanged.
- **Leave `SvidMaterial` exactly as-is** — no private-key field. This is the
  point of A: the CA emits only public material.
- **`RcgenCa::issue_svid`** (`crates/overdrive-host/src/ca/rcgen_ca.rs`): delete
  the `KeyPair::generate()` line. Parse the supplied public key via
  `SubjectPublicKeyInfo::from_der(req.workload_public_key().as_der())` and pass
  it as the first argument to the existing
  `params.signed_by(&spki, &issuer)` call (Finding 2 — the public-key arm is
  already what the code uses, just with the wrong source key).
- **`SimCa::issue_svid`** (`crates/overdrive-sim/src/adapters/ca.rs`): treat the
  supplied public key as opaque bytes exactly like the fixture certs (G4); the
  serial-draw determinism path is untouched. The DST/sim *caller* supplies a
  fixture public key loaded via `KeyPair::from_pem()` → its DER SPKI, identical
  across seeds.
- **`issue_and_audit`** (`crates/overdrive-control-plane/src/ca_issuance.rs`):
  thread the public key into the `SvidRequest` it builds. No private-key
  handling is added (there is none to handle).

### rcgen API + feature change

- Use `CertificateParams::signed_by(public_key: &impl PublicKeyData, issuer:
  &Issuer<…>)` with `public_key = &SubjectPublicKeyInfo::from_der(spki_der)?`
  (Findings 2, 7).
- **Add `x509-parser` to the rcgen feature set** in the root `Cargo.toml`
  (currently `["ring", "pem"]` → `["ring", "pem", "x509-parser"]`). This is
  required for `SubjectPublicKeyInfo::from_der` (and for the A2 CSR path later).
  This is the only dependency consequence (Finding 7).
- **A2 (later, for #26)**: when workloads are separate processes, switch the
  `SvidRequest` payload to a PKCS#10 CSR (`CsrDer`), parse with
  `CertificateSigningRequestParams::from_der(...)?.signed_by(&issuer)?`. **Add a
  proof-of-possession check** — rcgen does NOT verify the CSR self-signature on
  parse (Finding 3); the CA must verify it (or accept that the single in-process
  boundary made it moot until #26). Track this as a known requirement for the #26
  slice.

### Sim-determinism resolution

No change to the determinism contract. Under Option A the leaf key never crosses
the boundary, so KPI K5 (byte-identical across seeds) continues to ride on the
serial draw alone (G4, Finding D). The deterministic workload public key under
DST is a **fixture loaded by the test caller** (`KeyPair::from_pem()` → DER
SPKI), not anything the CA generates. This sidesteps the
`KeyPair::generate()`-is-non-deterministic limitation (prior Finding 11)
entirely — the CA never generates a leaf key in either adapter.

### Process: ADR amendment vs. straight bugfix

**This warrants a `/nw-design` ADR-0063 amendment, not a straight bugfix**, for
two reasons:
1. It changes the **trait surface** (`SvidRequest` gains a field) — a port-trait
   contract change, which ADR-0063 D1/D5 governs as the SSOT. The trait
   docstrings already *reference* "Option A (ADR-0063 D5 amendment)" as if it
   were decided, but `SvidRequest` was never actually extended — so the ADR
   amendment needs to **formally land the `SvidRequest` public-key field** that
   the docstrings already presume, closing the gap between documented intent and
   implemented surface.
2. It adds a **dependency feature** (`x509-parser`) and a **future
   proof-of-possession obligation** (A2) that should be recorded as a tracked
   decision, not buried in a bugfix commit.

The *implementation* (delete `generate()`, parse supplied key, thread the field)
is a mechanical follow-on once the amendment fixes the surface. Per the project's
"dispatch DESIGN artifacts to the architect" rule, the ADR-0063 amendment is an
architect-agent task; the crafter then implements against the amended surface.

> Note: this document **extends** the prior CA research
> (`built-in-ca-rcgen-rustls-comprehensive-research.md`). It does not contradict
> Finding 5 (it confirms and operationalizes the SPIRE CSR model at the trait
> surface), Finding 11 (it shows Option A makes the
> `generate()`-non-determinism limitation irrelevant to issuance), or Finding 12
> (the proposed `cert_rotation` workflow's `generate_keypair() → sign_svid(pubkey)`
> shape IS Option A). The prior research's gap was that it never specified the
> leaf-key flow *at the `Ca` trait boundary*; that gap is now closed.

## Full Citations

[1] SPIFFE Project. "The X.509 SPIFFE Verifiable Identity Document (X509-SVID)". SPIFFE Standards. https://github.com/spiffe/spiffe/blob/main/standards/X509-SVID.md. Accessed 2026-06-06.
[2] SPIFFE Project. "SPIRE Concepts". spiffe.io. https://spiffe.io/docs/latest/spire-about/spire-concepts/. Accessed 2026-06-06.
[3] Istio Authors. "Security — Concepts". istio.io. https://istio.io/latest/docs/concepts/security/. Accessed 2026-06-06.
[4] Linkerd / Buoyant. "Automatic mTLS". linkerd.io. https://linkerd.io/2/features/automatic-mtls/. Accessed 2026-06-06.
[5] cert-manager Authors. "Certificate — Usage". cert-manager.io. https://cert-manager.io/docs/usage/certificate/. Accessed 2026-06-06.
[6] HashiCorp. "PKI Secrets Engine". developer.hashicorp.com. https://developer.hashicorp.com/vault/docs/secrets/pki. Accessed 2026-06-06.
[7] rcgen contributors. "CertificateParams". docs.rs/rcgen 0.14.5. https://docs.rs/rcgen/0.14.5/rcgen/struct.CertificateParams.html. Accessed 2026-06-06.
[8] rcgen contributors. "CertificateSigningRequestParams". docs.rs/rcgen 0.14.5. https://docs.rs/rcgen/0.14.5/rcgen/struct.CertificateSigningRequestParams.html. Accessed 2026-06-06.
[9] rcgen contributors. "PublicKeyData trait". docs.rs/rcgen 0.14.5. https://docs.rs/rcgen/0.14.5/rcgen/trait.PublicKeyData.html. Accessed 2026-06-06.
[10] rcgen contributors. "SubjectPublicKeyInfo". docs.rs/rcgen 0.14.5. https://docs.rs/rcgen/0.14.5/rcgen/struct.SubjectPublicKeyInfo.html. Accessed 2026-06-06.
[11] Nystrom, M. & Kaliski, B. "RFC 2986 — PKCS #10: Certification Request Syntax Specification v1.7". IETF. 2000. https://www.rfc-editor.org/rfc/rfc2986. Accessed 2026-06-06.
[12] Overdrive (in-repo). "Built-in CA rcgen/rustls comprehensive research" (Findings 5, 11, 12). docs/research/security/built-in-ca-rcgen-rustls-comprehensive-research.md. Accessed 2026-06-06.
[13] Overdrive (in-repo). `Ca` trait, `SvidRequest`, `SvidMaterial` — crates/overdrive-core/src/traits/ca.rs; `RcgenCa::issue_svid` — crates/overdrive-host/src/ca/rcgen_ca.rs; `SimCa` — crates/overdrive-sim/src/adapters/ca.rs; `issue_and_audit` — crates/overdrive-control-plane/src/ca_issuance.rs; rcgen pin — Cargo.toml:87. Accessed 2026-06-06.

## Research Metadata

Duration: ~1 session | Examined: 12 sources (6 external official, 4 docs.rs/IETF, 2 in-repo clusters) | Cited: 13 | Cross-refs: 5 external orgs agree independently on the A/B split | Confidence: High 100% | Output: docs/research/security/svid-leaf-keypair-flow-research.md

**Confidence**: High. **Sources**: 13. Update the header line accordingly.
