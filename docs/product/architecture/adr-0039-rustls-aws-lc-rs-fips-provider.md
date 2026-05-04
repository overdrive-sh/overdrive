# ADR-0039 — rustls cryptographic provider: adopt `aws-lc-rs` with opt-in `fips` feature

## Status

Accepted. 2026-05-04. Decision-makers: Morgan (proposing), user
ratification 2026-05-04. Tags: cross-cutting, tls, identity, mtls,
compliance, dependency.

## Context

`rustls` is the platform's TLS terminator at every userspace handshake
site Overdrive owns:

- **Whitepaper §7** — sockops + kTLS. The node agent performs the
  TLS 1.3 handshake via `rustls`, presenting the workload's SVID, then
  installs the negotiated session keys into kTLS so the kernel record
  layer takes over for the connection's lifetime.
- **Whitepaper §8** — workload SVIDs and operator mTLS to the CLI.
  Every workload-to-workload connection and every `overdrive` operator
  call is mutually authenticated via X.509 certificates anchored in
  the built-in CA; the userspace handshake runs through `rustls`.
- **Whitepaper §11** — the gateway. North-south termination uses
  `hyper` + `rustls`; ACMEv2 issuance uses `instant-acme`, which is
  itself rustls-native and shares the same `IdentityMgr` cert path
  (§11 *Public-Trust Certificates*).
- **Whitepaper §4 / §17** — Raft transport (HA `IntentStore`) and
  Corrosion's QUIC-based gossip both run under the platform CA's
  trust bundle. Corrosion's QUIC is `quinn`, which terminates TLS via
  `rustls`.

Design principle 7 (*Rust throughout, no FFI to Go or C++ in the
critical path*) makes the choice of TLS stack non-negotiable.
`rustls`'s 0.23+ release line introduces a pluggable cryptographic
provider model: the same `rustls` crate, configured with a different
`CryptoProvider`, dispatches every primitive (AEAD, KEM, signatures,
key schedule) through the chosen backend. Two providers ship upstream:

- **`ring`** — historically the implicit default. Pure-Rust + asm; not
  on the CMVP cert list and the upstream `ring` crate has no FIPS
  validation roadmap.
- **`aws-lc-rs`** — Rust bindings to AWS-LC, Amazon's open-source fork
  of BoringSSL. Exposes a `fips` cargo feature that links the
  FIPS-validated build of AWS-LC. The corresponding cryptographic
  module holds **NIST FIPS 140-3 Cert #4816** [1][2][3].

Two binding facts settle the provider decision:

1. **Compliance is structural, not retrofit.** The §8 promise of
   mTLS-by-default — "every packet carries cryptographic workload
   identity" — is only defensible to a FedRAMP / DoD / regulated
   customer if the underlying crypto module is on the CMVP active
   cert list. `ring` cannot satisfy this; `aws-lc-rs` with the `fips`
   feature can. The transparent-encryption research [4 § Finding 13]
   identified this as the single change required to light up an
   operator-facing FIPS mode without altering primitives.
2. **Upstream rustls-ecosystem alignment is already on `aws-lc-rs`.**
   `instant-acme` (whitepaper §11, the gateway's ACMEv2 client) and
   `quinn` (Corrosion's QUIC stack, §4 / §17) both default to
   `aws-lc-rs`. Selecting `aws-lc-rs` workspace-wide aligns with the
   ecosystem's chosen direction rather than fighting it; selecting
   `ring` would force a non-default configuration in two transitive
   deps and split the workspace's crypto provider against itself.

The transparent-encryption research at
`docs/research/transparent-encryption-comprehensive-research.md`
landed Finding 13 as the canonical citation for the cert and
Recommendation 3 as the operator-facing consequence. This ADR is the
binding decision that follows.

## Decision

### 1. `aws-lc-rs` is the workspace-standard rustls cryptographic provider

Workspace `Cargo.toml`:

```toml
[workspace.dependencies]
rustls = { version = "0.23", default-features = false, features = ["aws-lc-rs", "tls12", "logging"] }
```

`default-features = false` is load-bearing: it disables the implicit
`ring` provider feature so the workspace cannot accidentally compile
both providers and pick `ring` as the installed default. Every
workspace member that needs `rustls` inherits via `rustls.workspace =
true` per the project's dependency convention
(`.claude/rules/development.md` § Dependencies); no leaf crate pins
a separate version or features. `aws-lc-rs` itself is pulled in
transitively by the rustls feature; no direct dependency declaration
is required.

### 2. The `fips` feature is exposed as a workspace-level opt-in

A workspace cargo feature `fips` propagates to every binary and
adapter crate that wires TLS:

```toml
[features]
fips = ["aws-lc-rs/fips"]
```

`fips` defaults to **off**. Operators who require FIPS-validated
deployments build with `--features fips` and the production binary
links the FIPS-validated build of AWS-LC; non-regulated deployments
get the standard `aws-lc-rs` build with no behavioural difference at
the rustls API level. The boundary is build-time, not run-time —
mixing a FIPS-validated and non-validated module in one process is
incoherent and the cargo feature flag forecloses it.

### 3. Provider installation at every TLS-aware binary entry point

The composition root wires the provider exactly once before any
`rustls::ClientConfig` / `ServerConfig` is constructed. Three binaries
own a TLS handshake surface today:

- `crates/overdrive-cli/src/main.rs` — operator mTLS to the control
  plane (§8 *Operator Identity and CLI Authentication*).
- The `overdrive serve` entry point in `overdrive-cli` — composes the
  control plane (§4 IntentStore replication, §11 gateway when
  enabled, §8 SVID issuance / mTLS termination, future §7 sockops
  handshake handler).
- `xtask` Tier-3 integration test harness — only when its in-process
  TLS bring-up needs an explicit provider; otherwise inherits.

The wiring is one call:

```text
// pseudocode — exact API per rustls 0.23 docs
rustls::crypto::aws_lc_rs::default_provider()
    .install_default()
    .expect("rustls provider must install before any TLS config is built");
```

Under `--features fips` the same call site uses
`default_fips_provider()` instead and asserts
`ServerConfig::fips() == true` against the constructed config to
confirm the FIPS-validated path is the one running.

### 4. kTLS kernel-side FIPS posture is out of scope for this ADR

`rustls` performs the TLS handshake; once session keys are installed
into kTLS via the sockops handoff (§7), the kernel's record-layer
implementation handles encrypt/decrypt for the connection's
remaining lifetime. The kernel's crypto API and its FIPS posture are
governed by the host kernel build, not by `rustls`'s provider
choice. A "Overdrive runs in FIPS-validated mode end-to-end" claim
requires *both* a FIPS-built `aws-lc-rs` (this ADR) *and* a
FIPS-validated kernel crypto module on the host (operator concern,
e.g. RHEL FIPS-mode kernel, Ubuntu Pro FIPS kernel). The Image
Factory `meta-overdrive` layer (§23) does not currently produce
FIPS-validated kernel images; doing so is a separate decision and a
separate ADR if and when it is taken.

This ADR draws the boundary explicitly: the userspace handshake is
covered; the kernel record layer is the operator's substrate
choice, not Overdrive's.

## Alternatives considered

### Alternative A — Stay on `ring`

Keep the implicit default; treat FIPS as a future concern.

**Rejected.** Two binding reasons. (a) `ring` is not on the CMVP
active cert list and `ring` upstream has no FIPS validation roadmap;
the §8 mTLS-by-default claim is undefensible to regulated customers
without a cert-list-backed module. (b) `instant-acme` and `quinn`
already default to `aws-lc-rs`; staying on `ring` would split the
workspace's crypto provider against itself and require non-default
configuration on two transitive deps. The cost of "do nothing" is
already higher than the cost of switching.

### Alternative B — Use a `rustcrypto`-family provider

Community-maintained pure-Rust providers exist (e.g.
`rustls-rustcrypto`). Pure-Rust through and through; matches design
principle 7 maximally.

**Rejected.** Two reasons. (a) No FIPS 140-3 validation; the
compliance angle that motivates this ADR is unsatisfiable. (b)
Ecosystem maturity gap — none of the production-grade rustls
consumers in Overdrive's transitive graph (`instant-acme`, `quinn`,
`hyper-rustls`) treat `rustcrypto` providers as a first-class
default; selecting it imposes ongoing integration tax for no
operator-visible benefit. Design principle 7 is satisfied by
`aws-lc-rs` at the rustls API surface — the FFI to AWS-LC sits
beneath rustls, not in Overdrive's critical path.

### Alternative C — Use OpenSSL via a non-rustls TLS stack

Substitute `openssl` or `native-tls` for `rustls` workspace-wide.
OpenSSL has long-standing FIPS-validated builds.

**Rejected.** Three reasons. (a) Contradicts design principle 7 — a
C TLS stack in the §7 sockops handshake critical path is the exact
shape the principle exists to prevent. (b) Whitepaper §11 names
`rustls` explicitly as the gateway's TLS terminator; §8's east-west
mTLS posture is built on the same library; switching stacks is a
cross-cutting rework with no compensating benefit
`aws-lc-rs/fips` does not already provide. (c) `instant-acme`,
`quinn`, and `hyper-rustls` are rustls-coupled by construction;
abandoning rustls forecloses the existing ACMEv2 path (§11), the
existing QUIC stack (§4 / §17), and the hyper integration in one
move.

## Consequences

### Positive

- **Operator-facing FIPS mode lights up via a single cargo feature.**
  No architectural rework, no library swap, no parallel TLS stack.
  Build with `--features fips` and the FIPS-validated AWS-LC build
  of `aws-lc-rs` (Cert #4816) is what handles every userspace
  handshake.
- **Workspace alignment with the rustls ecosystem.** `instant-acme`
  and `quinn` already default to `aws-lc-rs`; this ADR makes
  Overdrive consistent with that default rather than fighting two
  transitive deps with non-default features.
- **Compliance posture is structural.** The §8 mTLS-by-default
  promise is now defensible against a FedRAMP / DoD / regulated
  audit because the cert-list-backed module is one cargo feature
  away in the same binary, not a separate product or future
  refactor.
- **No DST surface change.** The `Transport` trait (whitepaper §21)
  and the `Sim*` adapters are unaffected — the provider swap is a
  wiring change at the binary boundary; sim adapters under
  `overdrive-sim` do not perform real handshakes.
- **No architectural ADR drift.** ADR-0010 (Phase 1 TLS bootstrap)
  remains valid; the cert-issuance state machine, the trust bundle,
  and the rotation reconciler operate above the provider boundary
  and are unchanged.

### Negative

- **`aws-lc-rs` carries a C dependency.** AWS-LC is the BoringSSL
  fork; `aws-lc-rs` builds it from source via `cmake` + a C
  toolchain on first compile. The build-time dependency on a C
  toolchain is new for Overdrive; it is well-supported on every
  target the project ships against (Lima Ubuntu 24.04, GitHub
  Actions runners, the Image Factory `meta-overdrive` layer's Yocto
  build environment). Design principle 7's intent — *no FFI to Go
  or C++ in the critical path* — is preserved at the call-site
  level: the rustls API surface is the boundary; AWS-LC sits
  beneath it; no Overdrive code calls AWS-LC directly.
- **Build-time cost.** First compile of `aws-lc-rs` takes longer
  than `ring` (compiling AWS-LC is more work than `ring`'s
  vendored asm). Amortised across cargo's build cache; not
  observable on incremental rebuilds. CI cache hit rate covers the
  steady-state cost.
- **`fips` builds require a FIPS-validated `aws-lc-rs` artifact at
  the linked version.** Operators selecting `--features fips` must
  pin a `aws-lc-rs` version whose `fips` feature corresponds to a
  currently-active CMVP cert (the validated module is versioned;
  not every `aws-lc-rs` release ships a FIPS build). This
  constraint is upstream-managed; the workspace dependency
  declaration tracks the active validated version.

### Quality-attribute impact

- **Security — confidentiality / integrity / authenticity**:
  positive. The cert-list-backed crypto module is one cargo feature
  away on the same call sites; the §8 mTLS-by-default property
  gains a compliance-defensible foundation without architectural
  change.
- **Security — non-repudiation / accountability**: neutral. The
  cert-issuance and SVID-rotation paths are above the provider
  boundary; this ADR does not affect them.
- **Compatibility — interoperability**: positive. Aligning with
  `instant-acme` and `quinn` defaults removes a minor source of
  configuration friction at the workspace boundary.
- **Maintainability — modifiability**: neutral. Provider is
  installed once at composition root; future provider changes are
  a one-line edit at three known call sites.
- **Performance efficiency — time behaviour**: neutral. AWS-LC's
  AEAD performance is competitive with `ring`; the per-handshake
  cost difference is far below kTLS's per-connection setup cost
  (which is the dominant term once the handshake is amortised
  across the connection's lifetime per §7).
- **Portability — installability**: neutral. Build-time C toolchain
  is already present on every shipping target.
- **Reliability — fault tolerance / recoverability**: neutral. No
  runtime-semantics change.

### Migration

Single-cut per the project's greenfield single-cut convention
(`.claude/rules/development.md` § "Single-cut migrations in
greenfield"). One PR:

1. Workspace `Cargo.toml` declares `rustls = { version = "0.23",
   default-features = false, features = ["aws-lc-rs", "tls12",
   "logging"] }` and exposes the `fips` workspace feature.
2. Every `rustls`-using crate switches to `rustls.workspace = true`
   if not already (audit pass).
3. The composition root in `overdrive-cli` calls
   `rustls::crypto::aws_lc_rs::default_provider().install_default()`
   exactly once before any TLS config is constructed; `xtask`'s
   Tier-3 in-process harness inherits the same call if it
   constructs its own configs.
4. Any latent `ring`-feature reference in a leaf crate is removed
   in the same PR. No deprecation period; no parallel provider
   path.

The crafter handles the implementation; this ADR records the
architectural decision. No existing ADR is superseded by this one;
ADR-0010 (Phase 1 TLS bootstrap) is unchanged.

## Compliance

- **Whitepaper Design principle 7** (*Rust throughout*): preserved
  at the rustls API surface. AWS-LC sits beneath the `rustls`
  boundary; no Overdrive code calls into AWS-LC directly. The FFI
  to C lives in `aws-lc-rs`'s build, not in any Overdrive critical
  path.
- **Whitepaper §7** (eBPF Dataplane — sockops + kTLS handshake):
  the userspace handshake the section describes runs through the
  provider this ADR mandates.
- **Whitepaper §8** (Identity and mTLS): the mTLS-by-default
  property gains a compliance-defensible foundation via the `fips`
  feature; no change to SVID issuance, trust-bundle distribution,
  or rotation.
- **Whitepaper §11** (Gateway — Public-Trust Certificates): aligns
  with `instant-acme`'s rustls-native default; the gateway
  continues to use `IdentityMgr` for both internal-trust SVIDs and
  ACMEv2-issued certs.
- **Whitepaper §4 / §17** (IntentStore Raft transport,
  Corrosion's QUIC): both run on `rustls`-terminated TLS via
  `quinn` (Corrosion) and the project's own Raft transport; both
  inherit the workspace-standard provider.
- **ADR-0010** (Phase 1 TLS bootstrap): unchanged. The cert
  bootstrap state machine sits above the provider boundary.
- **`development.md` § Dependencies**: workspace dependencies
  always; leaf crates never pin a separate version. This ADR's
  declaration follows the convention.
- **`development.md` § Single-cut migrations in greenfield**: no
  deprecation period, no parallel provider path; one PR replaces
  the implicit `ring` configuration with explicit `aws-lc-rs`.

## References

- [1] AWS Security Blog — "AWS-LC FIPS 3.0: First cryptographic
  library to include ML-KEM in FIPS 140-3 validation."
  https://aws.amazon.com/blogs/security/aws-lc-fips-3-0-first-cryptographic-library-to-include-ml-kem-in-fips-140-3-validation/
  — Cert #4816 reference; accessed 2026-05-04.
- [2] AWS Security Blog — "AWS-LC is now FIPS 140-3 certified."
  https://aws.amazon.com/blogs/security/aws-lc-is-now-fips-certified/
  — accessed 2026-05-04.
- [3] rustls authors — "FIPS." rustls manual chapter.
  https://docs.rs/rustls/latest/rustls/manual/_06_fips/index.html
  — covers the `fips` feature, `default_fips_provider()`, and
  runtime validation via `ClientConfig::fips()` /
  `ServerConfig::fips()`. Accessed 2026-05-04.
- [4] `docs/research/transparent-encryption-comprehensive-research.md`
  — Finding 13 (FIPS 140-3 Cert #4816 via rustls + aws-lc-rs);
  Recommendation 3 (operator-facing FIPS mode lights up by
  switching the rustls provider to `aws-lc-rs` with the `fips`
  feature).
- AWS-LC GitHub repository — https://github.com/aws/aws-lc — the
  upstream C library `aws-lc-rs` binds.
- `aws-lc-rs` crate page — https://crates.io/crates/aws-lc-rs —
  the Rust binding crate; documents the `fips` cargo feature.
- Whitepaper §7 (eBPF Dataplane), §8 (Identity and mTLS), §11
  (Gateway), §4 / §17 (IntentStore + ObservationStore transports)
  — the call sites this ADR's provider choice covers.
- Whitepaper Design principle 7 — *Rust throughout, no FFI to Go
  or C++ in the critical path.*
- ADR-0010 — Phase 1 TLS bootstrap (unchanged by this ADR).
- `.claude/rules/development.md` § Dependencies; § Single-cut
  migrations in greenfield.

## Changelog

- 2026-05-04 — Initial accepted version.
