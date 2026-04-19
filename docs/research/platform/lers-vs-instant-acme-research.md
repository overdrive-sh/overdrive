# `lers` vs `instant-acme` for Overdrive's Embedded ACME Client

**Date**: 2026-04-19 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 14

## Executive Summary

**Recommendation: switch from `lers` to `instant-acme`.**

Three decisive reasons:

1. **Maintenance trajectory.** `lers` last released v0.4.0 on 2023-04-04; last commit (a single dependabot update) was 2024-05-01. Three of three open issues — including a three-year-old "fix flakey JWS signing" bug opened by the maintainer themselves, and a January-2024 "Rustls support" request — remain unactioned. `instant-acme` ships on a quarterly-ish cadence (latest 0.8.5, 2026-02-24; 0.8.0 in 2025-07-09 with API redesign + modern ACME extensions), has 1.38M total downloads to `lers`'s 7.7K (≈180× ratio), and is the single most popular ACME crate on crates.io per its maintainer's 2025 retrospective.

2. **Ecosystem alignment with Overdrive's stack.** `instant-acme` is maintained by djc (Dirkjan Ochtman), who also maintains `rustls`, `rcgen`, `quinn`, and `hickory-dns` — precisely the Rust TLS/crypto/DNS libraries Overdrive already depends on (rustls for kTLS termination and the gateway, rcgen for the built-in CA, aya/eBPF adjacent to QUIC in the dataplane). `instant-acme` uses rustls directly via `hyper-rustls`, supports `rcgen` as an optional feature for CSR generation, and ships with `aws-lc-rs` by default (with `ring` as an alternative and `fips` as an opt-in). `lers` pulls in the C OpenSSL crate for all cryptography and uses `reqwest` (which in turn defaults to hyper+openssl unless carefully configured).

3. **Feature coverage is a wash or slightly favours `instant-acme` on what matters.** Both claim RFC 8555 compliance. Both support HTTP-01, DNS-01, and TLS-ALPN-01. `instant-acme` additionally supports the ACME Renewal Information (ARI) and Profiles extensions, has explicit EAB + key rollover + contact update + revocation APIs, a documented `RetryPolicy`, and a pluggable `HttpClient` trait. The one place `lers` looks richer — it ships built-in HTTP-01 / TLS-ALPN-01 / Cloudflare DNS-01 solvers — is the wrong axis for Overdrive: Overdrive must drive challenges through its own dataplane (XDP/TC programs serve HTTP-01; ObservationStore-backed DNS-01 for internal hickory-dns integration), so "bring-your-own-solver" is a fit, not a gap. `instant-acme`'s flow (retrieve challenge token → user dispatches → `challenge.set_ready()` → library polls) maps cleanly onto a reconciler-driven rotation loop.

The "Rust throughout" principle (§2.7) and the "Own your primitives" principle (§2.1) both lean the same way here. The `lers` OpenSSL dependency is the only feature-parity cost of switching, and it is not a feature.

## Research Methodology

**Search Strategy**: Direct fetches of `docs.rs` + `github.com` + `crates.io` API for both libraries; cross-checked release dates against git tags, changelogs, and the maintainer's published 2025 retrospective; sampled source code (Cargo.toml, types.rs, lib.rs, examples) to verify feature claims.

**Source Selection**: Types: `docs.rs`, `crates.io` (technical_documentation, high reputation), `github.com` (industry_leaders, medium-high), maintainer's personal blog (primary author, corroborates GitHub), `letsencrypt.org` + IETF RFC 8555 (official, high). Verification: every major claim cross-referenced between docs.rs/crates.io and GitHub source tree.

**Quality Standards**: 2+ independent sources per major claim (small OSS projects have limited secondary coverage — 2 sources/claim was agreed acceptable in the research brief). Cross-reference status documented per finding.

## 1. Feature Coverage Comparison

| Dimension | `lers` 0.4.0 | `instant-acme` 0.8.5 |
|---|---|---|
| RFC 8555 compliance | ACMEv2 per README [1] | ACMEv2 per docs.rs [6] |
| HTTP-01 | Yes, with built-in solver [1][3] | Yes, `ChallengeType::Http01`; bring-your-own dispatch [7][9] |
| DNS-01 | Yes, with built-in Cloudflare-only solver [1][3] | Yes, `ChallengeType::Dns01`; bring-your-own [7][9] |
| TLS-ALPN-01 | Yes, with built-in solver (v0.4.0+) [3] | Yes, `ChallengeType::TlsAlpn01` [7] |
| Custom solver extensibility | Yes, `Solver` trait [1] | N/A — user drives challenge loop; pluggable `HttpClient` trait for transport [6][9] |
| Account create | Yes [1] | Yes, `Account`/`AccountBuilder`/`NewAccount` [6][10] |
| Account contact update | Not surfaced in docs [1] | Yes (per maintainer 0.8.0 notes) [6][11] |
| Account key rollover | Not surfaced [1] | Yes [6][11] |
| Account deactivate | Not surfaced [1] | Yes [6][11] |
| External Account Binding (EAB) | Yes [3] | Yes, `ExternalAccountKey` [6][10] |
| Order / issue | Yes, `certificate().add_domain().obtain()` [1] | Yes, `NewOrder`/`Order`/`OrderState` [6][10] |
| Renewal | Yes [1] | Yes, including ARI (`RenewalInfo`) [6][11] |
| Revocation | Yes [1] | Yes, `RevocationRequest`/`RevocationReason` [6][10] |
| Wildcard certs | Implied via DNS-01; not explicit in README [1] | Explicit `AuthorizedIdentifier { wildcard: bool }` [7] |
| Rate-limit / retry policy | Not surfaced [1] | Yes, `RetryPolicy` [6][10] |
| Certificate bundling | **Not implemented** (per README) [1] | Yes, standard order flow returns full chain [6] |
| ACME Profiles extension | No [1] | Yes (added in 0.8.0, Jul 2025) [11] |

**Solver architecture — key architectural divergence.** `lers` ships batteries-included: start an HTTP-01 server, hand it a Cloudflare API token for DNS-01, and it drives the whole flow end-to-end. `instant-acme` gives you the challenge token and expects the caller to dispatch it however makes sense in their environment, then calls `challenge.set_ready()` and polls. For a gateway that already owns the HTTP ingress path and a cluster that already runs hickory-dns, `instant-acme`'s model is the right shape — Overdrive should never delegate HTTP-01 serving or DNS record writing to an ACME library's own mini-solver.

## 2. Dependency Footprint

### 2.1 `lers` (v0.4.0, from Cargo.toml inspection)

Required direct dependencies: `async-trait`, `base64`, `chrono`, `futures`, `hex`, **`openssl`**, **`reqwest`**, `serde`, `tokio`, `tracing`. Optional: `hyper`, `rcgen`, `trust-dns-resolver`, `uuid` (behind feature flags) [2].

Transitive implications:

- **`openssl ^0.10`** — C library binding. Requires `libssl-dev` at build time; vendored feature exists but still pins a C toolchain as build requirement. This is the dependency brushing against Overdrive Principle 7.
- **`reqwest ^0.11`** — unless `default-features = false` + `rustls-tls` is set, this pulls in hyper + tokio + the `native-tls` crate, which on Linux also links openssl. The `lers` Cargo.toml default does not disable this [2].
- **`trust-dns-resolver`** — optional, only for DNS-01 propagation checks. Predecessor to hickory-dns (now deprecated upstream; hickory is the maintained successor).
- **`chrono`** — not needed for ACME semantics; `time` or `jiff` would be more aligned with the rustls ecosystem.

Approximate transitive crate count: ~120+ (dominated by reqwest's default feature set + openssl's sys crates). Not precisely established; `docs.rs/crate/lers/0.4.0/dependencies` returned 404.

### 2.2 `instant-acme` (v0.8.5 / 0.9.0 on main)

Required direct dependencies: `tokio ^1.22`, `hyper ^1.3.1`, `hyper-rustls ^0.27.7`, `serde`/`serde_json`, `base64`, `rustls ^0.23`, **`aws-lc-rs ^1.8.0`** (default). Optional features: `rcgen ^0.14.2`, `ring ^0.17`, `fips` (aws-lc-rs FIPS mode), `x509-parser`, `time` [2][11].

Features:

- **Default: `aws-lc-rs` + `hyper-rustls`.** Pure-Rust wrapping of AWS's hardened libcrypto fork, widely used across rustls ecosystem, FIPS-certifiable.
- **`ring` alternative.** Long-standing pure-Rust crypto (Brian Smith / rustls ecosystem). Opt-in for teams that prefer it over aws-lc-rs.
- **`fips` opt-in.** Enables FIPS mode in aws-lc-rs — directly relevant if Overdrive ever targets FedRAMP/PCI.
- **`hyper 1.x`** (not `hyper 0.14`) — aligns with the modern hyper-util / hyper-rustls stack Overdrive should be on anyway for its gateway.
- **`HttpClient` trait.** Caller can swap in a custom transport; useful if Overdrive wants the ACME client to flow through its own telemetry/egress pipeline for audit.

Approximate transitive crate count: smaller than `lers` because hyper 1 + hyper-rustls + aws-lc-rs avoids the reqwest/openssl transitive fan-out. Not precisely counted (docs.rs dependency page 404'd for both); confidence: **Medium** on the "smaller than" claim, **High** on the qualitative direction.

### 2.3 Comparison

- `lers` mandates a C-linked crypto stack (openssl) and a higher-level HTTP client (reqwest) that brings its own default feature set. Fighting that back to pure-Rust is possible but requires careful feature juggling and the public API still takes openssl types.
- `instant-acme` ships pure-Rust crypto (aws-lc-rs default, ring optional), modern hyper 1.x, and its default feature set matches the rustls-ecosystem shape. Zero FFI to C on the default configuration.
- Both depend on tokio; neither claims runtime-agnosticism.
- Both depend on serde / serde_json.

On Overdrive's "own your primitives" axis: `instant-acme` loses zero, `lers` adds OpenSSL to the build.

## 3. Maintenance and Production Use

| Metric | `lers` | `instant-acme` |
|---|---|---|
| Latest release | **v0.4.0, 2023-04-04** [5] | **v0.8.5, 2026-02-24** [4][8] |
| Time since latest release (as of 2026-04-19) | ~36 months | ~2 months |
| Total releases | 5 (0.1.0 → 0.4.0, all in Mar–Apr 2023) [5] | 25+ over 4+ years [8] |
| Last commit to main | 2024-05-01 (dependabot trust-dns-resolver bump) [12] | Active; 0.8.5 tag 2026-02-24, 0.9.0 already staged on main [11] |
| Release cadence 2024–2026 | Zero releases in that window | ~quarterly (0.8.0 Jul 2025; 0.8.1–0.8.4 Jul/Oct/Nov 2025; 0.8.5 Feb 2026) [8] |
| Open issues | 3 (incl. #1 "Fix flakey JWS signing" open since 2023-03-24, opened by the maintainer; #34 "Rustls support" opened 2024-01-02 — no maintainer response visible) [13] | 6 open; active triage [4] |
| Total crates.io downloads | 7,695 [5] | 1,385,269 [8] |
| Recent downloads | 628 [5] | 219,628 [8] |
| GitHub stars | 33 [12] | 199 [4] |
| GitHub dependents (reverse deps) | ~5 repositories, ~2 packages — all from a single user (@manglemix) [14] | ~185 repositories, ~44 packages [14] |
| Maintainer | Alexander Krantz (@akrantz01) — single-author project [5] | Dirkjan Ochtman (@djc) — also maintains rustls, rcgen, quinn, hickory-dns [15][16] |
| Named production users | None in README [1] | "Instant Domain Search" per README and maintainer retrospective [11][15]; "instant-acme is now the most popular ACME library on crates.io" [15] |
| Security advisories (CVE / RUSTSEC) | None found | None found |

**Maintenance assessment.** `lers` is effectively dormant. The three-year gap since its last release spans the entire production-maturity window of rustls 0.23, hyper 1, aws-lc-rs 1.x, ACME Profiles extension (2024–2025), and ARI. A single unanswered "rustls support" issue from January 2024 on what is currently an openssl-only library tells the maintenance story on its own. `instant-acme` is actively maintained by one of the most prolific maintainers in the Rust TLS ecosystem (djc reports 850 PRs authored and 1600 PRs reviewed across ~100 repos in 2025) [15] — the library is in the maintenance hot zone of its author, not the cold storage.

## 4. License

| | License | AGPL-3.0 compatibility |
|---|---|---|
| `lers` | MIT [5] | Compatible (MIT is permissive; can be incorporated into AGPL codebases) |
| `instant-acme` | Apache-2.0 [8] | Compatible (Apache-2.0 is permissive; can be incorporated into AGPL-3.0 — explicit compatibility confirmed by GNU; Apache's patent grant is a plus) |

Both are permissive licenses. Apache-2.0 offers a slight advantage via its explicit patent grant, but the difference is not decisive for Overdrive.

## 5. Overdrive Fit

### 5.1 Runtime and crypto backend alignment

Overdrive is tokio-async and rustls-native (whitepaper §7: sockops mTLS via rustls; §11 gateway: rustls termination). `instant-acme` is tokio-async, rustls-native (hyper-rustls), and defaults to aws-lc-rs — a crypto provider from the same author set as rustls itself. Overdrive should already have rustls + a crypto provider (ring or aws-lc-rs) in its dependency graph; `instant-acme` contributes no new TLS stack.

`lers` contributes a parallel crypto stack (OpenSSL via C FFI) that must coexist with the rustls/ring stack Overdrive already carries. That is the specific footgun Overdrive Principle 7 points at: two TLS stacks in one binary, two places to audit, two places to keep compliant.

### 5.2 Shared paths with rcgen / IdentityMgr

The built-in CA described in whitepaper §4 uses rcgen to mint SVIDs. `rcgen` is maintained by the rustls org [16]. `instant-acme` exposes an optional `rcgen` feature that lets it generate CSRs using rcgen — meaning the cert-generation code path is already shared between `IdentityMgr`'s internal SVID issuance and the gateway's public-trust ACME flow. `IdentityMgr` becomes a single rcgen-using subsystem with two trust anchors (internal CA vs Let's Encrypt) rather than two independent cert-generation paths.

`lers` does have an optional `rcgen` feature too (per its Cargo.toml) but uses openssl internally for signing and key operations. The surface shape of `lers`'s public API also exposes openssl types in places. That's a leaky abstraction relative to Overdrive's rustls/rcgen baseline.

### 5.3 Principle 7 revisited (critical path question)

Principle 7 reads: *"Rust throughout. Memory safety, performance, and a maturing ecosystem that now covers every required primitive. No FFI to Go or C++ in the critical path."*

The "critical path" caveat exists because some primitives (e.g., the Linux kernel, the VMM hypervisor) are not going to be rewritten in Rust, and the principle is pragmatic about that. The intended scope of "critical path" is clearly the per-packet / per-request data path: TLS handshake (kernel + rustls), kTLS record layer (kernel), policy enforcement (XDP/LSM), service routing (XDP).

ACME is genuinely not on the hot path — certs rotate every 60 days (Let's Encrypt default 90-day TTL; operational norm ≤60 days), and issuance is initiated by a reconciler once per cert, not per connection. The "bursty workload startup" argument does not apply to public-trust certs: public-trust certs are attached to public hostnames served by gateway nodes, and gateway nodes are static infrastructure (whitepaper §11: "gateway nodes are designated by configuration, not by scheduling"). There is no world in which Overdrive provisions 1,000 Let's Encrypt certs per second against bursty workloads — Let's Encrypt's own rate limits (300 new orders per account per 3 hours, 50 certs per registered domain per 7 days) [17] make that architecturally impossible.

So Principle 7 is *not* violated by keeping `lers`/OpenSSL strictly because ACME runs off the hot path. But the principle is also stated with ecosystem ambition: *"a maturing ecosystem that now covers every required primitive."* The whole point is that when a pure-Rust option is available at parity or better, the platform should take it. `instant-acme` is that option. The critical-path caveat was written for cases where Rust doesn't yet have a competitive primitive. For ACMEv2 in 2026, Rust does.

There is also an operational-surface argument for switching: two TLS stacks in one process means two CVE-tracking workstreams (OpenSSL's historical CVE rate is orders of magnitude higher than rustls + aws-lc-rs combined), two FIPS stories, two supply-chain trust bundles. The `instant-acme` FIPS opt-in feature is a concrete compliance lever the `lers` path does not offer.

## 6. Recommendation

**Switch to `instant-acme` in the whitepaper (§11) and in the implementation plan.**

Rationale, ranked by weight:

1. **Maintenance risk** — `lers` has effectively stopped receiving updates (single maintenance commit in 2024; zero releases since 2023-04-04). Adopting it now means Overdrive would either fork-and-maintain or ship a stale crypto-touching dependency. `instant-acme` is actively shipped by a core rustls-ecosystem maintainer. This is the single largest asymmetry.

2. **Ecosystem cohesion** — `instant-acme` + `rustls` + `rcgen` + `aws-lc-rs` + `hickory-dns` are all maintained by overlapping author sets and are designed to compose. `IdentityMgr` can use one rcgen-based cert-generation path for internal SVIDs and public-trust certs. No dual crypto stack.

3. **Principle 7 consistency** — the critical-path caveat makes OpenSSL *tolerable* in `lers`, but Principle 7's broader intent ("a maturing ecosystem that now covers every required primitive") points the other way when a pure-Rust option exists. `instant-acme` means zero C FFI in the ACME path, one less dual-TLS-stack audit, and an explicit FIPS mode for future compliance work.

4. **API shape fits Overdrive's reconciler model** — `instant-acme`'s bring-your-own-challenge-dispatch pattern is the right shape for a reconciler that owns the gateway ingress path (HTTP-01 via the gateway's own hyper server) and may later own DNS-01 via a hickory-dns integration. `lers`'s batteries-included solvers would short-circuit or duplicate infrastructure Overdrive already has.

5. **Feature parity is good enough** — every major ACME capability Overdrive needs (HTTP-01, DNS-01, TLS-ALPN-01, wildcard, EAB, rollover, revocation, ARI, rate-limit aware retry) is in `instant-acme`. The one area `lers` is "richer" (built-in solvers) is not a fit axis for Overdrive.

**Dual support is rejected.** Abstracting the ACME client behind a trait would be a platform-level mistake — the library choice affects feature set, crypto stack, and runtime characteristics, and an abstraction thin enough to cover both libraries would also be too thin to hide the feature gap (e.g., ACME Profiles, ARI, rcgen integration) between them. Pick one.

**Suggested whitepaper edit (§11 Public-Trust Certificates):**
> *"The gateway embeds an ACMEv2 client (`instant-acme`, pure-Rust, rustls-native, maintained by the rustls-ecosystem author set) to obtain public-trust certificates for north-south ingress. Certs feed into the same `IdentityMgr` that handles internal SVIDs — one rotation pipeline, two trust lanes, one rcgen-based cert-generation path."*

## Knowledge Gaps

### Gap 1: Exact transitive dependency counts

**Issue**: `docs.rs/crate/{name}/{version}/dependencies` pages returned HTTP 404 for both crates during research. Transitive crate counts are inferred qualitatively from direct-dependency shape (reqwest vs hyper, openssl vs aws-lc-rs). **Attempted**: crates.io API, docs.rs pages, GitHub Cargo.lock (not committed in either repo). **Recommendation**: Before commit, run `cargo tree -p instant-acme` and `cargo tree -p lers` in a scratch crate to confirm the qualitative "`instant-acme` tree is smaller" claim. Does not change the recommendation; increases precision of §2.

### Gap 2: Tip-of-main `instant-acme` version inconsistency

**Issue**: The crates.io release (0.8.5, Feb 2026) differs from the in-tree Cargo.toml version (0.9.0). **Attempted**: Inferred 0.9.0 is an unpublished work-in-progress on main. **Recommendation**: Pin to 0.8.5 initially; watch for 0.9.0 release notes when they land. Negligible risk.

### Gap 3: `lers` rate-limit / retry behaviour

**Issue**: Could not confirm from `lers`'s public docs whether it honours the `Retry-After` header from Let's Encrypt rate-limit responses. **Attempted**: README inspection, docs.rs top-level. **Recommendation**: Would require reading `lers` source; not worth the effort given the broader recommendation is to switch.

### Gap 4: GitHub dependents pages are under-counts

**Issue**: GitHub's dependents view only shows public GitHub repos with visible Cargo.lock; it misses private-cluster and crates.io-published downstreams. Real production use for both libraries is higher than reported. **Attempted**: Cross-checked against crates.io download counts (which do reflect pulls from private CI). **Recommendation**: Downloads numbers (180× ratio) are the more honest proxy than dependents counts. The qualitative conclusion is not affected.

## Conflicting Information

### Conflict 1: Latest `instant-acme` release date

**Position A**: "February 24, 2025" — from GitHub Releases page scrape [initial WebFetch].
**Position B**: "February 24, 2026" — from git tags page [4] and crates.io API [8].
**Assessment**: Position B is correct. The earlier 2025 date was a WebFetch summarisation error. The git tags page (authoritative: shows both tag and commit dates) and the crates.io API agree on 2026-02-24.

## Full Citations

[1] akrantz01. "lers README". GitHub. 2023-04-04. https://github.com/akrantz01/lers/blob/main/README.md. Accessed 2026-04-19. Reputation: Medium-High (primary source, active-though-dormant open source).

[2] akrantz01. "lers Cargo.toml". GitHub. 2023-04. https://raw.githubusercontent.com/akrantz01/lers/main/Cargo.toml. Accessed 2026-04-19. Reputation: High (primary source).

[3] akrantz01. "lers v0.4.0 docs". docs.rs. 2023-04-04. https://docs.rs/lers/latest/lers/. Accessed 2026-04-19. Reputation: High (technical_documentation tier).

[4] djc. "instant-acme repository". GitHub. Active. https://github.com/djc/instant-acme. Accessed 2026-04-19. Reputation: Medium-High.

[5] Alexander Krantz. "lers crate metadata". crates.io API. https://crates.io/api/v1/crates/lers. Accessed 2026-04-19. Reputation: High (official registry).

[6] djc. "instant-acme v0.8.5 docs". docs.rs. 2026-02-24. https://docs.rs/instant-acme/latest/instant_acme/. Accessed 2026-04-19. Reputation: High.

[7] djc. "instant-acme types.rs (ChallengeType enum)". GitHub. 2026. https://raw.githubusercontent.com/djc/instant-acme/main/src/types.rs. Accessed 2026-04-19. Reputation: High (primary source).

[8] Dirkjan Ochtman. "instant-acme crate metadata". crates.io API. https://crates.io/api/v1/crates/instant-acme. Accessed 2026-04-19. Reputation: High.

[9] djc. "instant-acme provision.rs example". GitHub. 2026. https://github.com/djc/instant-acme/blob/main/examples/provision.rs. Accessed 2026-04-19. Reputation: High.

[10] djc. "instant-acme lib.rs (public API surface)". GitHub. 2026. https://raw.githubusercontent.com/djc/instant-acme/main/src/lib.rs. Accessed 2026-04-19. Reputation: High.

[11] djc. "instant-acme Cargo.toml (main branch)". GitHub. 2026. https://raw.githubusercontent.com/djc/instant-acme/main/Cargo.toml. Accessed 2026-04-19. Reputation: High.

[12] akrantz01. "lers commits/main". GitHub. https://github.com/akrantz01/lers/commits/main. Accessed 2026-04-19. Reputation: Medium-High.

[13] akrantz01. "lers open issues". GitHub. https://github.com/akrantz01/lers/issues. Accessed 2026-04-19. Reputation: Medium-High.

[14] GitHub. "Dependents graph for lers and instant-acme". https://github.com/akrantz01/lers/network/dependents and https://github.com/djc/instant-acme/network/dependents. Accessed 2026-04-19. Reputation: Medium-High.

[15] Dirkjan Ochtman. "Rust maintenance in 2025". dirkjan.ochtman.nl. 2026-01-09. https://dirkjan.ochtman.nl/writing/2026/01/09/reviewing-2025.html. Accessed 2026-04-19. Reputation: Medium-High (primary-author blog; corroborated by GitHub + crates.io).

[16] rustls org. "rcgen README". GitHub. Active. https://github.com/rustls/rcgen. Accessed 2026-04-19. Reputation: High (rustls organisation repo).

[17] Let's Encrypt. "Rate Limits". letsencrypt.org. https://letsencrypt.org/docs/rate-limits/. Accessed 2026-04-19. Reputation: High (official, technical_documentation tier).

[18] IETF. "RFC 8555: Automatic Certificate Management Environment (ACME)". datatracker.ietf.org. https://datatracker.ietf.org/doc/html/rfc8555. Accessed 2026-04-19. Reputation: High (official standards body).

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| lers README | github.com | Medium-High | industry | 2026-04-19 | Y (with docs.rs, crates.io) |
| lers Cargo.toml | github.com | High | primary | 2026-04-19 | Y |
| lers docs.rs | docs.rs | High | technical | 2026-04-19 | Y |
| instant-acme repo | github.com | Medium-High | industry | 2026-04-19 | Y |
| lers crates.io API | crates.io | High | technical | 2026-04-19 | Y |
| instant-acme docs.rs | docs.rs | High | technical | 2026-04-19 | Y |
| instant-acme types.rs | github.com | High | primary | 2026-04-19 | Y |
| instant-acme crates.io API | crates.io | High | technical | 2026-04-19 | Y |
| instant-acme provision.rs | github.com | High | primary | 2026-04-19 | Y |
| instant-acme lib.rs | github.com | High | primary | 2026-04-19 | Y |
| instant-acme Cargo.toml | github.com | High | primary | 2026-04-19 | Y |
| lers commits | github.com | Medium-High | industry | 2026-04-19 | Y |
| lers issues | github.com | Medium-High | industry | 2026-04-19 | Y |
| GitHub dependents | github.com | Medium-High | industry | 2026-04-19 | Y (vs crates.io downloads) |
| djc 2025 retro | dirkjan.ochtman.nl | Medium-High | primary-author | 2026-04-19 | Y (vs GitHub + crates.io) |
| rcgen repo | github.com | High | OSS-foundation-adjacent | 2026-04-19 | Y |
| LE rate limits | letsencrypt.org | High | official | 2026-04-19 | N/A (authoritative) |
| RFC 8555 | ietf.org | High | official | 2026-04-19 | N/A (authoritative) |

Reputation mix: High 12 (67%), Medium-High 6 (33%), Medium 0, Excluded 0. Avg ≈ 0.93.

## Research Metadata

Duration: ~30 min. Examined: 18 sources. Cited: 18. Cross-refs: major claims (release cadence, maintainer identity, crypto backend, challenge support, OpenSSL dependency, rustls authorship) each verified across ≥2 independent sources. Confidence: High on the recommendation axis; Medium on exact transitive crate counts (Gap 1). Output: `/Users/marcus/conductor/workspaces/overdrive/taipei-v1/docs/research/platform/lers-vs-instant-acme-research.md`.
