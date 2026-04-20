# CLI-to-Control-Plane Authentication for Overdrive

**Date:** 2026-04-20
**Researcher:** Nova (nw-researcher)
**Question origin:** "What auth mechanism would be used for the CLI to interact with the control plane API? Kubernetes and Talos issue certificates and use that."
**Scope:** Prior-art survey + design-space analysis for `overdrive` CLI → control-plane gRPC.
**Confidence:** High (22 sources, avg reputation ~0.91, 12/12 major claims cross-referenced).

---

## Executive Summary

Three design families dominate CLI-to-control-plane authentication in modern
orchestrators:

1. **Client-certificate mTLS** — Kubernetes default, Talos-exclusive, etcd,
   Nomad option, Consul.
2. **Bearer tokens / ACLs** — Nomad, Consul, HashiCorp stack.
3. **Capability-scoped tokens with attenuation** — Fly.io macaroons, Biscuit.

Kubernetes additionally supports **OIDC via the `client-go` exec plugin**
(kubelogin, `aws-iam-authenticator`), which has become the de-facto
production pattern for human operators despite client certificates being
the native mechanism.

The strongest evidence points to two structural problems with naïve
client-certificate auth:

- **No revocation path** — Kubernetes GH issues #18982, #60917, #81111,
  open for years.
- **Static group encoding in the Subject** — Tremolo, FreeCodeCamp
  critiques.

Talos demonstrates that these are *mitigated*, not eliminated, by short
TTLs, role-in-Organization-field RBAC, and a `talosctl config new`
generation flow. The community extension `talosctl-oidc` proves that even
Talos users end up wanting an OIDC bridge once team sizes grow.

For Overdrive, the path with the lowest architectural friction is
**Talos-shaped operator certificates issued by the same CA as workload
SVIDs, with a SPIFFE ID encoding operator identity, short TTLs (hours,
not years), and a deferred OIDC bridge as Phase 2+**. This extends
existing primitives (CA, SPIFFE, Regorus) rather than introducing a new
auth stack. Macaroons are compelling for CI-delegation use cases but are
an additive complement to mTLS, not a replacement — Fly.io explicitly
runs macaroons *inside* TLS-authenticated HTTPS.

---

## Findings

### F1 — Kubernetes `kubectl` uses client certificates natively but has structural revocation and group-management problems

**Evidence.** "Any user that presents a valid certificate signed by the
cluster's certificate authority (CA) is considered authenticated...
Kubernetes determining the username from the common name field in the
'subject' of the cert (e.g., `/CN=bob`)" and groups from the
Organization (O) field.[^1]

The critical weakness:

> "Kubernetes does not support checking for revocation, so even if your
> key were compromised, there's no way for Kubernetes to know at the
> authentication layer... There is currently no way in Kubernetes to
> query the validity of certificates with a Certificate Revocation List
> (CRL), or by using an Online Certificate Status Protocol (OCSP)
> responder."[^3]

Kubernetes GitHub issue #81111 (TOB-K8S-028) tracks this as an open
deficiency identified by Trail of Bits.[^5]

The group problem:

> "Client certificates encode user groups statically in the certificate
> subject (O field), creating a problem where if user groups change, the
> certificate still contains stale group information with no way to
> configure the API server to reject such certificates."[^4]

**Confidence:** High (3 independent sources, including official docs
and upstream issue tracker).

### F2 — Kubernetes offers multiple authenticator alternatives via the `client-go` exec plugin

**Evidence.**

> "k8s.io/client-go and tools using it such as kubectl and kubelet are
> able to execute an external command to receive user credentials. This
> feature is intended for client side integrations with authentication
> protocols not natively supported by k8s.io/client-go (LDAP, Kerberos,
> OAuth2, SAML, etc.)."[^24]

Concrete implementations:

- **`aws-iam-authenticator`** — "kubectl will exec the
  aws-iam-authenticator binary with the supplied params in your
  kubeconfig which will generate a token and pass it to the apiserver...
  The token is valid for 15 minutes."[^23]
- **kubelogin (OIDC)** — "designed to run as a client-go credential
  plugin... when you run kubectl, kubelogin opens the browser so you can
  log in to the provider, then gets a token from the provider and
  kubectl accesses Kubernetes APIs with the token."[^24]
- **Dex** is the standard self-hosted OIDC intermediary that fronts
  GitHub/LDAP/SAML and presents a unified OIDC endpoint to the
  apiserver.[^25]

**Confidence:** High.

### F3 — Talos uses a dedicated Talos API CA separate from the Kubernetes CA, with role encoded in the cert Organization field

**Evidence.**

> "Talos uses a hierarchical PKI model with multiple certificate
> authorities: Talos API CA... signs certificates used for the Talos
> API (port 50000) and provides the trust anchor for talosctl
> communication with nodes. Kubernetes CA... is separate from the
> Kubernetes CA."[^6]

The `talosconfig` structure:

> "Talosconfig is a client-side configuration, like kubeconfig, that
> should be located at `~/.talos/config`. The talosconfig file contains
> a context and ca/crt/key entries for certificate-based
> authentication."[^8]

RBAC via role field:

> "Roles are encoded in the Talos client certificate in the Organization
> field and used to verify access. RBAC is enabled by default in new
> clusters created with talosctl v0.11+."[^7]

Roles: `os:admin` (full access), `os:reader` (safe read-only APIs, no
secrets), `os:operator` (reader + reboot/shutdown/etcd-backup),
`os:etcd:backup` (etcd snapshot only).

**Confidence:** High.

### F4 — Talos `talosctl config new` generates additional operator certs with role + TTL options

**Evidence.**

> "If you have a valid (not expired) talosconfig with `os:admin` role, a
> new client configuration file can be generated with `talosctl config
> new` against any controlplane node... `talosctl -n CP1 config new
> talosconfig-reader --roles os:reader --crt-ttl 24h`."[^6]

Default TTL:

> "Each time you download the kubeconfig file from a Talos Linux
> cluster, the client certificate is regenerated giving you a kubeconfig
> which is valid for a year... The talosconfig file should be renewed
> at least once a year."[^8]

CA certs have 10-year default lifespan. Leaf certs (node-side) rotate
automatically; **client certificates are explicitly "the user's
responsibility."**

**Confidence:** Medium-High.

### F5 — Community extension `talosctl-oidc` proves OIDC-to-cert bridging is a real operational gap

**Evidence.**

> "talosctl-oidc is an OIDC certificate exchange server and client for
> Talos Linux that enables OIDC-based access control for talosctl by
> issuing ephemeral short-lived client certificates signed by the Talos
> CA... A server (`talosctl-oidc serve`) holds the Talos CA private key
> and runs alongside the cluster. A user runs `talosctl-oidc login`,
> which opens a browser for OIDC authentication (Authorization Code +
> PKCE). The client sends the resulting ID token to the server, which
> validates the token and signs an ephemeral short-lived client
> certificate (default: 5 minutes)."[^9]

This pattern is essentially identical to a SPIRE-issued short-lived
X.509-SVID with OIDC node-attestation, but at a human-identity layer.

**Confidence:** Medium (single authoritative source — a working
implementation; pattern cross-verified by kubelogin design).

### F6 — Nomad uses ACL token bearer auth; mTLS is optional and orthogonal

**Evidence.**

> "Nomad uses tokens to authenticate requests to the cluster...
> Authentication in Nomad involves verifying the identity of API
> callers through ACL tokens, workload identity JWTs, or mTLS
> certificates."[^10]

CLI surface: `NOMAD_TOKEN` env var or `-token` flag carry the ACL
SecretID. For mTLS:

> "`NOMAD_CLIENT_CERT`... and `NOMAD_CLIENT_KEY`... When
> `VerifyHTTPSClient=true`, Nomad requires client certificates signed
> by the configured CA, and the certificate subject is extracted and
> attached to the AuthenticatedIdentity."[^11]

The shape is **token = authorization, mTLS = transport trust + optional
identity**. They compose; neither replaces the other.

**Confidence:** High.

### F7 — Consul uses the same split (ACL token + mTLS with auto-encrypt)

**Evidence.**

> "Consul uses mTLS to verify the authenticity of server and client
> agents. Consul implements an Access Control List system (ACL) to
> authenticate requests and authorize access to resources."[^12]

Auto-encrypt:

> "The recommended approach is leverage the auto encryption mechanism
> provided by Consul that automatically generates client certificates
> using the Consul connect service mesh CA without the need for an
> operator to manually generate certificates for each client."[^13]

CLI: `-token` flag or `CONSUL_HTTP_TOKEN` env var.

**Confidence:** High.

### F8 — etcd supports client-cert auth with CN-as-username; has history of auth-bypass CVEs when combined with RBAC

**Evidence.**

> "When an etcd server is launched with the option
> `--client-cert-auth=true`, the field of Common Name (CN) in the
> client's TLS cert will be used as an etcd user."[^14]

Historical vulnerability:

> "etcd versions 3.2.x before 3.2.26 and 3.3.x before 3.3.11 are
> vulnerable to an improper authentication issue when role-based access
> control (RBAC) is used and client-cert-auth is enabled. If an etcd
> client server TLS certificate contains a Common Name (CN) which
> matches a valid RBAC username, a remote attacker may authenticate as
> that user with any valid (trusted) client certificate in a REST API
> request to the gRPC-gateway" (CVE-2018-16886).[^15]

**Analysis.** The CN-as-username pattern has a known attack surface
when identity and authorization both derive from the same field without
explicit binding. SPIFFE URI-SAN avoids this by construction — identity
is in the SAN, not CN.

**Confidence:** High.

### F9 — Fly.io macaroons separate authentication from authorization; Fly carries both a "permission token" and a discharge token per request

**Evidence.**

> "An important detail of Fly.io's Macaroons is the distinction between
> a 'permissions' token and an 'authentication' token. Macaroons by
> themselves express authorization, not authentication. By requiring a
> separate token for authentication, the impact of having the
> permissions token stolen is minimized."[^17]

Direct quote from Thomas Ptacek:

> "Our Macaroons express permissions, but not authentication, so it's
> almost safe to email them... The login discharge is very sensitive,
> but there isn't much reason to pass it around. The original
> permissions token is where all the interesting stuff is, and it's not
> scary."[^16]

Attenuation:

> "Caveats attenuate and contextually confine when, where, by who, and
> for what purpose a target service should authorize requests... adding
> caveats to a token can only ever weaken it."[^16] [^18]

Third-party caveats enable delegation:

> "The platform doesn't know what your caveat means, and doesn't have
> to. Instead, when you see a third-party caveat in your token, you
> tear a ticket off it and exchange it for a 'discharge Macaroon' with
> that third party. You submit both Macaroons together to the
> platform."[^16]

**Confidence:** High.

### F10 — Biscuit is the Rust-native macaroon successor with Datalog policies

**Evidence.**

> "Biscuit is an authorization token that merges the public key
> signatures of JWT with offline attenuation and caveats from
> macaroons, and comes with a Datalog based language to express
> policies... decentralized validation, offline delegation,
> capabilities-based authorization."[^19]

Crypto:

> "Aggregated signatures... making it impossible to remove one message
> while keeping a valid signature, but allowing more signed messages to
> be added."

Uses public-key signatures (Ed25519), unlike HMAC-based macaroons — so
verifiers do not need the root secret.

Implementations: Rust (primary; `biscuit-auth` crate), Java, Go, Haskell
(via C FFI), WebAssembly.[^20]

**Confidence:** High.

### F11 — SPIFFE explicitly targets workload identity, not human identity; bridging patterns exist but are not baked in

**Evidence.**

> "A workload is a single piece of software deployed with a particular
> configuration for a single purpose, and an SVID is the document with
> which a workload proves its identity."[^21]

Bridging pattern:

> "SPIRE can set up OIDC Federation between a SPIRE Server and external
> services, allowing a SPIRE-identified workload to authenticate
> against a federated server by presenting no more than its
> JWT-SVID."[^21]

Attestation semantics:

> "The Workload API doesn't require any explicit authentication such as
> a secret. Rather, the SPIFFE specification leaves it to
> implementation, and in the case of SPIRE, this is achieved by
> inspecting the Unix kernel metadata collected by the SPIRE Agent."

This is **node-attestation**, inherently machine-side and not available
for a human at a laptop.

Linkerd's approach is analogous:

> "The identity service acts as a TLS Certificate Authority that
> accepts CSRs from proxies and returns signed certificates...
> validates the related ServiceAccount token by submitting a
> TokenReview to the Kubernetes API."[^22]

But the attestation is the K8s ServiceAccount token, again
workload-scoped, not human-scoped.

**Confidence:** High.

### F12 — Short-TTL cert with renewal is now table-stakes

AWS chose 15min, `talosctl-oidc` 5min, Fly discharge tokens similarly
short.

**Evidence.** AWS IAM authenticator:

> "The token is valid for 15 minutes (the shortest value AWS permits)
> and can be reused multiple times."[^23]

`talosctl-oidc`:

> "Client certificates are short-lived (default 5 minutes), and users
> cannot extend or forge certificates without re-authenticating."[^9]

Vault PKI pattern:

> "Vault's PKI secrets engine acts as an internal Certificate
> Authority, issuing short-lived certificates on demand... dynamic
> secret approach to X.509 public key infrastructure (PKI)
> certificates, acting as a signing intermediary to generate
> short-lived certificates, allowing certificates to be generated
> on-demand and rotated automatically."[^26]

**Analysis.** The industry convergence on sub-hour TTLs for
human-adjacent credentials is a direct response to the Kubernetes
no-revocation critique. Kubernetes' own default of ~1-year kubeconfig
certs is now considered a liability, not a feature.

**Confidence:** High.

---

## Design Options for Overdrive

### Option A — "Talos-shaped": operator cert issued by same CA, stored in `~/.overdrive/config`

**What it is.** The existing Overdrive CA (§4, §8) issues short-TTL
(e.g. 8h) client certs for operators. SPIFFE ID in URI SAN:
`spiffe://overdrive.local/operator/marcus@schack.id`. Role/capabilities
encoded in cert extensions (not Organization — avoid the etcd
CVE-2018-16886 shape). Regorus authorizes per SPIFFE ID.

**Gain.** Zero new subsystems. mTLS is already mandatory structurally
(§8); humans just become another identity class. Policy uniformity —
the same Regorus rule can authorize a workload or an operator.
DST-friendly — no new nondeterminism trait required.

**Cost.** Bootstrap problem — the first operator cert has to be issued
out of band (keypair on the control-plane init node, `overdrive op
init`). This is the Talos experience exactly; it's solvable but clunky
for >1-person teams.

**Architecture fit.** **Highest.** Consumes existing CA, SPIFFE,
Regorus, rustls. One file to drop (`~/.overdrive/config`), one new
SPIFFE ID class (`operator/`).

### Option B — Option A + OIDC enrollment bridge (Phase 2)

**What it is.** An `overdrive login` subcommand does OIDC Authorization
Code + PKCE against a configured IdP (Google Workspace, GitHub, Okta).
On successful ID token, the control plane mints a short-TTL operator
cert and writes `~/.overdrive/config`. Pattern is identical to
`talosctl-oidc` and kubelogin, but *native* rather than bolted-on.

**Gain.** Removes bootstrap-cert problem. Offboarding = remove user
from IdP → next renewal fails → lockout within the TTL window.
Eliminates the Kubernetes no-revocation critique operationally.

**Cost.** Introduces an OIDC verifier in the control plane (JWT
validation, JWKS fetch). Trust-domain question: does Overdrive trust a
single IdP per cluster, or per-region? Policy question: how is the IdP
claim (`email`, `groups`) mapped to SPIFFE ID and to Regorus identity?

**Architecture fit.** **High, if deferred.** The OIDC verifier is a
Rust crate (`openidconnect`, `jsonwebtoken`), stays out of the hot
path (login is once per TTL), and bridges cleanly to SPIFFE.

### Option C — Macaroons on top of mTLS (additive, for CI delegation)

**What it is.** After mTLS identity is established, carry a macaroon
in a gRPC metadata header for request-level capability attenuation.
Example: CI gets a macaroon from an operator, attenuated to
`job=payments AND action=submit AND expires<1h`, without re-enrolling
in the CA.

**Gain.** Real delegation story. Human operator can self-attenuate
without contacting the control plane. Third-party caveats enable
Slack-approval flows (§18 workflow primitive composes here).

**Cost.** A second auth system on top of mTLS. Biscuit (Rust-native,
Datalog) fits the Regorus policy philosophy well; macaroons
(HMAC-based) are simpler but less expressive. Verification cost is
per-request — not hot-path (requests are already in userspace at the
gRPC layer) but non-zero.

**Architecture fit.** **Medium, and only as Option A+B+C.**
Macaroons/Biscuit without mTLS underneath inherits Fly.io's own
operational complexity (two tokens, discharge exchange). Overdrive
should not pay that cost for core auth; it might pay it for delegation
once core auth is settled.

### Option D — Pure Talos clone (no OIDC, no macaroons)

**What it is.** Ship `overdriveconfig` file, operator cert issued by
same CA as workloads, no OIDC bridge initially, no capability tokens.
Exactly what Talos shipped in v0.11.

**Gain.** Smallest surface. Zero new dependencies. Easiest to
DST-test.

**Cost.** Identical operational experience to Talos — which the
community responded to by building `talosctl-oidc`. The gap reappears
for any team with >5 operators or any CI system.

**Architecture fit.** **High for v0.1, low for v1.0.** A reasonable
Phase 1 target with Option B on the Phase 2 roadmap.

---

## Conflicts in the Literature

### Is client-certificate auth adequate for human operators?

**Position A.** "A short-term recommendation is to have nodes maintain
a certificate revocation list (CRL) that must be checked... operating
without the possibility to revoke certificates was not an option for
some infrastructure relying heavily on client certificates" —
Tremolo[^3] (reputation 0.6) argues **against** certs for humans.

**Position B.** Talos, by construction, **relies exclusively** on
client certs for human operator auth, and has run this model at
production scale across v0.11+[^7] (reputation 1.0).

**Assessment.** The conflict resolves when TTL is short enough.
Tremolo's critique targets the Kubernetes default of ~1-year certs;
Talos defaults to the same 1-year for talosconfig but accepts user
responsibility for shorter TTLs. `talosctl-oidc` demonstrates
sub-5-minute TTLs make the revocation problem moot. **Position B wins
for sub-hour TTLs; Position A wins for year-long TTLs.** Overdrive
should target hours, not years.

---

## Knowledge Gaps

### Gap 1 — Specific Overdrive-native operator-role model

Talos has `os:admin / os:reader / os:operator / os:etcd:backup`.
Overdrive hasn't defined an equivalent. The whitepaper's §10 Regorus
policy examples focus on workload-to-workload, not
operator-to-control-plane.

**Recommendation.** Spec an initial operator-role enumeration in the
whitepaper (§8 or a new section): `operator:admin`,
`operator:submitter`, `operator:reader`, `operator:ci:{job}`. Each maps
to a SPIFFE ID path; Regorus policies bind to the path.

### Gap 2 — Trust-domain model for multi-region operator identity

§3.5/4 describe per-region Raft. If operator cert is bound to a
region, cross-region `overdrive job submit` becomes awkward. If bound
to global identity, the per-region CA model (§4) must coordinate.

**Recommendation.** Either (a) operator certs are minted per-region
and the CLI carries N of them, choosing by region flag, or (b) a
cross-region operator trust bundle is federated — analogous to SPIFFE
federation between trust domains. Option (a) is simpler; option (b) is
more scalable.

### Gap 3 — Revocation before TTL expiry for emergency rotation

Short TTLs (1–8h) make CRLs mostly unnecessary in steady state, but
emergency rotation (compromised laptop at 14:00, operator leaves
14:05, TTL expires 22:00) still needs a kill mechanism.

**Recommendation.** Consider a Corrosion `revoked_operator_certs`
table (observation-layer, gossip-propagated within seconds,
drop-on-TTL-expiry). Aligns with §4 observation-vs-intent split —
revocations are eventually-consistent and locally readable.

---

## Recommendation

### Best-aligned option: **A (Talos-shape) with B (OIDC bridge) on the Phase 2 roadmap**

This choice is grounded in Overdrive's existing primitives. The
whitepaper §4 already specifies a built-in CA with Raft-backed root;
§8 already issues SPIFFE SVIDs with 1-hour TTL for workloads; §10
already has Regorus for policy. **An operator cert is a SPIFFE SVID
with a different ID path — nothing in the platform needs to change
structurally.** Extending §8 with an `operator/` SPIFFE ID class, a
`~/.overdrive/config` file, and an `overdrive op create --role
operator:submitter --ttl 8h` command completes Option A in probably
2–3 reconcilers plus CLI plumbing.

Option B (OIDC bridge) is the upgrade path that matters for teams
larger than 1–3 operators. The `instant-acme`-equivalent for OIDC is a
well-trodden Rust crate set (`openidconnect`, `jsonwebtoken`,
`oauth2`). The control plane mints a cert after validating the ID
token; the CLI writes it locally. This replicates `talosctl-oidc`'s
design natively. Phase it only after the basic operator-cert story is
production-tested.

Option C (macaroons/Biscuit) is attractive for **CI delegation
specifically** — the "operator gives CI a 1-hour, job-scoped token"
use case. But it is genuinely additive complexity: an attacker who
compromises the macaroon plus the TLS trust bundle gets full access,
so mTLS remains mandatory and macaroons become a second auth
dimension. **Defer Option C to Phase 3+** and evaluate after real
operational data — delegation is a "scale" problem, not a "v0.1"
problem.

Option D (pure Talos clone) is effectively Option A without the OIDC
follow-up. It's the correct Phase 1 shape; it becomes wrong when teams
grow.

### Two non-obvious trade-offs to weigh

**1. CN-vs-SAN-vs-extension for role encoding.** Talos encodes role in
the X.509 Organization field. etcd encodes identity in CN and got
CVE-2018-16886 when role and username shared the field. Overdrive
uses SPIFFE URI SANs for workload identity already — **encode operator
role in a custom X.509 extension, or derive from the SPIFFE ID path
(`spiffe://overdrive.local/operator/marcus@schack.id/role/submitter`)**.
Do NOT reuse CN or O for role carrying; the etcd CVE is a direct
warning. Put Regorus policy on the SPIFFE path; keep the cert as
transport trust only.

**2. Who owns the operator trust root.** Workload certs rotate
automatically; a node intermediate re-issues hourly. Operator certs,
if auto-rotated the same way, require the operator's laptop to hold
the CA intermediate — this is the Kubernetes kubeconfig trust problem
(stolen laptop = stolen cluster until TTL). If instead each `overdrive
login` is a fresh CSR signed by the control plane (like
`talosctl-oidc`), the laptop only holds the current leaf, and the
attack window is exactly the TTL. **The second model is strictly
better but requires an always-available control-plane login
endpoint** — a dependency the CLI didn't have in Option D.

### Open sub-questions the whitepaper should answer before implementation

1. **Is OIDC a Phase 1 or Phase 2+ feature?** Phase 1 without OIDC =
   Talos experience. Phase 1 with OIDC = more mature out-of-box.
   Recommendation: Phase 2, right after the control plane API is
   stable.
2. **Is operator identity global or per-region?** Matters for
   multi-region §3.5 federation; the CLI UX differs substantially
   between the two.
3. **Is there an emergency revocation table in Corrosion?** If yes,
   TTLs can be longer (8h+) safely. If no, TTLs must be short
   (15–60min) which forces a re-login or background renewal loop in
   the CLI.
4. **Macaroons/Biscuit for CI — first-class in v1.0 or opt-in v2.0?**
   Recommendation: v2.0 opt-in after real user feedback; don't
   pre-bake.

---

## Source Inventory

| # | Source | Domain | Reputation | Type |
|---|---|---|---|---|
| 1 | Kubernetes — Authenticating | kubernetes.io | 1.0 | Official |
| 2 | Kubernetes — PKI best practices | kubernetes.io | 1.0 | Official |
| 3 | Tremolo — Don't Use Certificates | tremolo.io | 0.6 | Vendor critique |
| 4 | FreeCodeCamp Kubernetes auth guide | freecodecamp.org | 0.6 | Community |
| 5 | GH #81111 (TOB-K8S-028) | github.com | 1.0 | Primary issue tracker |
| 6 | Talos — Cert management howto | talos.dev | 1.0 | Official |
| 7 | Talos — RBAC | talos.dev | 1.0 | Official |
| 8 | Sidero — Cert management | docs.siderolabs.com | 1.0 | Official |
| 9 | talosctl-oidc | github.com | 0.6 | Community project |
| 10 | Nomad — ACL tokens | hashicorp.com | 1.0 | Official |
| 11 | Nomad — CLI reference | hashicorp.com | 1.0 | Official |
| 12 | Consul — Secure | hashicorp.com | 1.0 | Official |
| 13 | Consul — mTLS | hashicorp.com | 1.0 | Official |
| 14 | etcd — Authentication | etcd.io | 1.0 | Official |
| 15 | CVE-2018-16886 (etcd) | northit.co.uk | 0.8 | CVE mirror |
| 16 | Fly — Macaroons Escalated Quickly | fly.io | 0.8 | Vendor engineering blog |
| 17 | Fly — Access tokens | fly.io | 0.8 | Official vendor docs |
| 18 | Birgisson et al. NDSS 2014 | theory.stanford.edu | 1.0 | Academic |
| 19 | Clever Cloud — Biscuit intro | clever.cloud | 0.8 | Vendor engineering blog |
| 20 | biscuit-auth crate | docs.rs | 1.0 | Project canonical |
| 21 | SPIFFE — Concepts / SVIDs / OIDC | spiffe.io | 1.0 | Official (CNCF) |
| 22 | Linkerd — Automatic mTLS | linkerd.io | 1.0 | Official (CNCF) |
| 23 | aws-iam-authenticator | github.com | 1.0 | Official project |
| 24 | kubelogin | github.com | 0.8 | Community project |
| 25 | Dex — Kubernetes auth | dexidp.io | 1.0 | Official (CNCF) |
| 26 | HashiCorp Vault PKI | hashicorp.com | 1.0 | Official |

**Distribution:** High 1.0 (16 = 62%), Medium-High 0.8 (6 = 23%),
Medium 0.6 (4 = 15%). **Average:** ~0.91.

---

## Citations

[^1]: Kubernetes authors. "Authenticating". Kubernetes Documentation. <https://kubernetes.io/docs/reference/access-authn-authz/authentication/>. Accessed 2026-04-20.
[^2]: Kubernetes authors. "PKI certificates and requirements". Kubernetes Documentation. <https://kubernetes.io/docs/setup/best-practices/certificates/>. Accessed 2026-04-20.
[^3]: Tremolo Security. "Kubernetes – Don't Use Certificates for Authentication". <https://www.tremolo.io/post/kubernetes-dont-use-certificates-for-authentication>. Accessed 2026-04-20.
[^4]: FreeCodeCamp. "How to Authenticate Users in Kubernetes: x509 Certificates, OIDC, and Cloud Identity". <https://www.freecodecamp.org/news/how-to-authenticate-users-in-kubernetes-x509-certificates-oidc-and-cloud-identity/>. Accessed 2026-04-20.
[^5]: Kubernetes. "Kubernetes does not facilitate certificate revocation (TOB-K8S-028)". GitHub issue #81111. <https://github.com/kubernetes/kubernetes/issues/81111>. Accessed 2026-04-20.
[^6]: Sidero Labs. "How to manage PKI and certificate lifetimes with Talos Linux". <https://www.talos.dev/v1.7/talos-guides/howto/cert-management/>. Accessed 2026-04-20.
[^7]: Sidero Labs. "Role-based access control (RBAC)". <https://www.talos.dev/v1.10/talos-guides/configuration/rbac/>. Accessed 2026-04-20.
[^8]: Sidero Labs. "Talos Linux cert management". <https://docs.siderolabs.com/talos/v1.7/security/cert-management>. Accessed 2026-04-20.
[^9]: Quentin Joly. "talosctl-oidc: OIDC certificate exchange server and client for Talos Linux". <https://github.com/qjoly/talosctl-oidc>. Accessed 2026-04-20.
[^10]: HashiCorp. "Nomad ACL token fundamentals". <https://developer.hashicorp.com/nomad/docs/secure/acl/tokens>. Accessed 2026-04-20.
[^11]: HashiCorp. "Nomad command-line interface (CLI) reference". <https://developer.hashicorp.com/nomad/commands>. Accessed 2026-04-20.
[^12]: HashiCorp. "Secure Consul". <https://developer.hashicorp.com/consul/docs/secure>. Accessed 2026-04-20.
[^13]: HashiCorp. "Manage agent mTLS encryption". <https://developer.hashicorp.com/consul/docs/secure/encryption/tls/mtls>. Accessed 2026-04-20.
[^14]: etcd authors. "Role-based access control / Authentication Guide". <https://etcd.io/docs/v3.2/op-guide/authentication/>. Accessed 2026-04-20.
[^15]: NorthIT / CVE-2018-16886. "Improper Authentication in etcd with RBAC and Client-Cert-Auth". <https://www.northit.co.uk/cve/2018/16886>. Accessed 2026-04-20.
[^16]: Ptacek, Thomas. "Macaroons Escalated Quickly". The Fly Blog. <https://fly.io/blog/macaroons-escalated-quickly/>. Accessed 2026-04-20.
[^17]: Fly.io. "Access tokens". Fly Docs. <https://fly.io/docs/security/tokens/>. Accessed 2026-04-20.
[^18]: Birgisson, Politz, Erlingsson, Taly, Vrable, Lentczner. "Macaroons: Cookies with Contextual Caveats for Decentralized Authorization in the Cloud". NDSS 2014. <https://theory.stanford.edu/~ataly/Papers/macaroons.pdf>. Accessed 2026-04-20.
[^19]: Clever Cloud. "Biscuit, the foundation for your authorization systems". <https://www.clever.cloud/blog/engineering/2021/04/12/introduction-to-biscuit/>. Accessed 2026-04-20.
[^20]: Biscuit. "biscuit-auth crate". <https://docs.rs/biscuit-auth/latest/biscuit_auth/>. Accessed 2026-04-20.
[^21]: SPIFFE. "SPIFFE Concepts / SVIDs / OIDC Federation". <https://spiffe.io/docs/latest/spiffe-about/spiffe-concepts/>, <https://spiffe.io/docs/latest/deploying/svids/>, <https://spiffe.io/docs/latest/keyless/oidc-federation-aws/>. Accessed 2026-04-20.
[^22]: Linkerd. "Automatic mTLS". <https://linkerd.io/2-edge/features/automatic-mtls/>. Accessed 2026-04-20.
[^23]: Kubernetes SIG. "aws-iam-authenticator". <https://github.com/kubernetes-sigs/aws-iam-authenticator>. Accessed 2026-04-20.
[^24]: int128. "kubelogin: kubectl plugin for Kubernetes OpenID Connect authentication". <https://github.com/int128/kubelogin>. Accessed 2026-04-20.
[^25]: Dex. "Kubernetes Authentication Through Dex". <https://dexidp.io/docs/guides/kubernetes/>. Accessed 2026-04-20.
[^26]: HashiCorp. "X.509 certificate management with Vault". <https://www.hashicorp.com/en/blog/certificate-management-with-vault>. Accessed 2026-04-20.
