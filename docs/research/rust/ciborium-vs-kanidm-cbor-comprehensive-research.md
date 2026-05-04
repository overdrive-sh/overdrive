# Research: ciborium vs kanidm/cbor (serde_cbor_2) for Rust serde-based CBOR encoding

**Date**: 2026-05-03 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: TBD

## Executive Summary

(populated at end)

## Research Methodology

**Search Strategy**: Direct GitHub repo inspection for both candidates; docs.rs for API surfaces; crates.io / lib.rs for release metadata; web search for lineage and issue history; IETF for RFC 8949 anchor.

**Source Selection**: Official repo READMEs, crate documentation, RFC text. All sources from `github.com`, `docs.rs`, `crates.io`, `ietf.org` — high-reputation per the trusted source config.

**Quality Standards**: Target 2+ sources per major claim. Lineage claims cross-referenced against original-author archive notice + downstream consumer migration discussions.

## Findings

### Finding 1: kanidm/cbor IS the maintained continuation of pyfisch/serde_cbor

**Evidence**: The original serde_cbor was archived on 2021-08-15 by its maintainer (pyfisch), who wrote that the crate "needs to be retired" after nearly six years and that "no one else stepped up to maintain this crate." The README of the archived original recommends `ciborium` and `minicbor` as successors, and does NOT mention `serde_cbor_2`.

The kanidm fork publishes as `serde_cbor_2` (version 0.11.2 visible at time of access), with its README still describing itself as "an implementation of the Concise Binary Object Representation from RFC 7049" — copied from the upstream README — and explicitly positions itself as "a drop in replacement of `serde_cbor`."

**Source**: [pyfisch/cbor (archived original)](https://github.com/pyfisch/cbor) - Accessed 2026-05-03
**Verification**: [qdrant issue #147 discussing the archive](https://github.com/qdrant/qdrant/issues/147), [kanidm/cbor README](https://github.com/kanidm/cbor)
**Confidence**: High

**Analysis**: The naming `serde_cbor_2` and the API drop-in claim are deliberate — the fork exists to keep existing call sites compiling against an unmaintained dep. The original maintainer's recommended successors were ciborium and minicbor; serde_cbor_2 is a third path that prioritises API continuity over a fresh design.

### Finding 2: ciborium is a clean-room serde-CBOR implementation maintained by enarx

**Evidence**: ciborium lives at `enarx/ciborium`, Apache-2.0, and is maintained by the enarx organization (an Intel-originated confidential-compute project). Its README states it "always serializes numeric values to the smallest size" rather than fixed widths and follows the Robustness Principle. It exposes `from_reader()` and `into_writer()` as the primary API, plus a `Value` type for dynamic CBOR. Maps are `Vec<(Value, Value)>` to preserve wire order.

The latest version on crates.io is **0.2.2**, released 2024-06-18. Fedora packaging shows continued downstream rebuilds through July 2025; no new upstream version has shipped between mid-2024 and the time of this research. The repository shows 192 commits on main with active CI.

**Source**: [enarx/ciborium GitHub](https://github.com/enarx/ciborium) - Accessed 2026-05-03
**Verification**: [ciborium on docs.rs](https://docs.rs/ciborium/latest/ciborium/), [Fedora rust-ciborium 0.2.2-4.fc43 package](https://rpmfind.net/linux/RPM/fedora/43/x86_64/r/rust-ciborium+default-devel-0.2.2-4.fc43.noarch.html)
**Confidence**: High

**Analysis**: ciborium's API is `Read`/`Write`-shaped rather than slice-shaped — `into_writer(&value, &mut Vec<u8>)` and `from_reader(&[u8][..])` are the canonical entry points. There is no `to_vec(&value) -> Vec<u8>` convenience function in `ciborium::ser`; you pass a `&mut Vec<u8>` as the writer. This is a real ergonomic difference from `serde_cbor_2`'s `to_vec` / `from_slice` shape.

### Finding 3: serde_cbor_2 still targets RFC 7049, ciborium does not commit publicly to either

**Evidence**: The `serde_cbor_2` README copies the upstream wording: "implementation of the Concise Binary Object Representation from RFC 7049." RFC 7049 was obsoleted by **RFC 8949** in December 2020.

ciborium's README references "the CBOR specification" without naming an RFC number. Its design choice to "always serialize numeric values to the smallest size" aligns with RFC 8949 §4.2 (Preferred Serialization), but the project does not advertise full deterministic-encoding compliance.

**Source**: [RFC 8949 (CBOR), IETF](https://www.rfc-editor.org/rfc/rfc8949.html) - Accessed 2026-05-03
**Verification**: [kanidm/cbor README](https://github.com/kanidm/cbor), [ciborium docs.rs](https://docs.rs/ciborium/latest/ciborium/)
**Confidence**: High

**Analysis**: For Overdrive's reconciler View persistence, RFC compliance is largely cosmetic — both libraries emit and parse interoperable CBOR. The point matters only if View blobs ever cross a wire boundary to a non-Rust consumer that strictly enforces RFC 8949, which is not the ADR-0035 use case (single-node redb).

### Finding 4: ciborium is built directly on serde and supports the standard derive macros

**Evidence**: ciborium's `ciborium::ser::into_writer` takes `T: Serialize`, and `ciborium::de::from_reader` returns `T: DeserializeOwned`. Standard `serde` attributes — `#[serde(default)]`, `#[serde(skip_serializing_if = "...")]`, `#[serde(flatten)]`, enum tagging — all flow through unchanged because ciborium implements the serde data model rather than reinventing it.

serde_cbor_2 inherits the same property from upstream serde_cbor; the original was the canonical example for serde-on-CBOR for many years and the test suite covered the standard attributes.

**Source**: [ciborium::ser docs.rs](https://docs.rs/ciborium/latest/ciborium/ser/index.html) - Accessed 2026-05-03
**Verification**: [serde_cbor_2 docs.rs](https://docs.rs/serde_cbor_2/latest/serde_cbor_2/)
**Confidence**: High

**Analysis**: The schema-evolution path Overdrive needs (`#[serde(default)]` on additive View fields per ADR-0035) works in both. There is one well-known gotcha for both: `#[serde(default)]` only fires when the field is *absent*, not when it is present-but-CBOR-null, and CBOR encodes `Option::None` as `null` rather than as field-absent. For `Option<T>` fields with serde_default semantics under additive evolution, both libraries require the field author to think about whether the encoder emits the field for `None` (default behaviour: yes, as `null`) or skips it (`#[serde(skip_serializing_if = "Option::is_none")]`). This is a serde-CBOR-data-model issue, not a library-specific bug.

### Finding 5: ciborium is pure Rust, no_std-capable; so is serde_cbor_2

**Evidence**: ciborium splits across three crates: `ciborium-io` (zero-dep `Read`/`Write` trait shim), `ciborium-ll` (low-level encoder/decoder), and `ciborium` (high-level serde integration). All are pure Rust with no FFI. The `ciborium-io` design exists specifically to support no_std targets without pulling `std::io`.

serde_cbor_2 documents no_std support explicitly: "Serde CBOR supports building in a `no_std` context. The `alloc` feature enables `from_slice` and `to_vec` functionality in no_std environments with allocator access."

**Source**: [ciborium-io crates.io](https://crates.io/crates/ciborium-io) - Accessed 2026-05-03
**Verification**: [serde_cbor_2 docs.rs no_std section](https://docs.rs/serde_cbor_2/latest/serde_cbor_2/), [ciborium-ll crates.io](https://crates.io/crates/ciborium-ll)
**Confidence**: High

**Analysis**: Both satisfy Overdrive's design principle 7 (Rust throughout, no FFI). Neither pulls in C/C++ deps. The Overdrive node binary will never run in no_std (the redb store is hosted in a tokio runtime), so the no_std story is a nice-to-have rather than a constraint.

### Finding 6: Maintenance activity diverges sharply

**Evidence**: ciborium's most recent crates.io release is 0.2.2 from 2024-06-18 — roughly 11 months stale at time of writing (May 2026). The repository shows ongoing CI activity but no release cut. The kanidm/cbor repository shows 333 commits on main; the kanidm organization is a Rust identity-management project that depends on serde_cbor_2 for its own webauthn/credential storage and therefore has organisational incentive to keep it compiling against new Rust toolchains.

**Source**: [ciborium crates.io versions](https://crates.io/crates/ciborium) - Accessed 2026-05-03
**Verification**: [kanidm/cbor commit count](https://github.com/kanidm/cbor)
**Confidence**: Medium — exact serde_cbor_2 release cadence not directly observable from the searches that succeeded.

**Analysis**: ciborium is "feature complete and stable" rather than "abandoned" — the tag count and commit count both indicate it is in maintenance mode rather than dormant. A library that hashes correctly and round-trips serde does not need monthly releases. serde_cbor_2 is more actively committed-to but that activity reflects keeping a compatibility shim alive, not new feature work.

### Finding 7: Neither offers explicit RFC 8949 deterministic-encoding mode

**Evidence**: RFC 8949 §4.2.1 defines deterministic encoding: shortest-form integers, definite-length strings/arrays/maps, sorted map keys (length-then-bytewise). ciborium documents that it "always serializes numeric values to the smallest size" — partial deterministic compliance, but does NOT advertise sorted map keys or full deterministic mode. serde_cbor_2 documentation does not mention deterministic encoding at all.

A separate crate, `serde_cbor_core` (gordonbrander), exists specifically to provide RFC 8949 Core Deterministic Encoding — its existence is itself evidence that neither ciborium nor serde_cbor_2 fully covers this.

**Source**: [RFC 8949 §4.2.1 Core Deterministic Encoding Requirements](https://www.rfc-editor.org/rfc/rfc8949.html#name-core-deterministic-encoding) - Accessed 2026-05-03
**Verification**: [serde_cbor_core README](https://github.com/gordonbrander/serde_cbor_core), [ciborium README on numeric encoding](https://github.com/enarx/ciborium)
**Confidence**: High

**Analysis**: For Overdrive's reconciler View use case this does not matter — blobs are never hashed for content-addressing per ADR-0035 (the redb key is `(reconciler_name, target)`, not a content hash). If a future use case needed CBOR for content-addressed storage, neither candidate would suffice and `serde_cbor_core` or a manual canonicalisation pass would be required. This is consistent with the project's "Hashing requires deterministic serialization" rule in `.claude/rules/development.md`, which already directs internal hashed data to rkyv rather than CBOR.

### Finding 8: ciborium has notably wider downstream adoption

**Evidence**: Both crates are recommended by the original serde_cbor README as successors, but only `ciborium` is. The Android Open Source Project mirrors `ciborium`, `ciborium-io`, and `ciborium-ll` as platform crates. Google's `coset` (COSE — CBOR Object Signing and Encryption — RFC 8152 implementation) is built on ciborium. Fedora and Debian both package ciborium. By contrast, `serde_cbor_2` shows up primarily as a kanidm-ecosystem dependency with limited adoption beyond.

**Source**: [pyfisch/cbor archived README recommending ciborium and minicbor](https://github.com/pyfisch/cbor) - Accessed 2026-05-03
**Verification**: [google/coset using ciborium](https://github.com/google/coset), [Android platform mirror of ciborium](https://android.googlesource.com/platform/external/rust/crates/ciborium/)
**Confidence**: High

**Analysis**: Bus factor and external scrutiny matter for a foundation primitive. ciborium's adoption breadth means serde-roundtrip bugs surface and get fixed under wider workloads than the kanidm-internal cases drive. For a long-lived persistence format, this is the more conservative dependency choice.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| enarx/ciborium GitHub | github.com | High | industry | 2026-05-03 | Y |
| kanidm/cbor GitHub | github.com | High | industry | 2026-05-03 | Y |
| pyfisch/cbor (archived) | github.com | High | industry | 2026-05-03 | Y |
| qdrant/qdrant issue #147 | github.com | Medium-High | industry | 2026-05-03 | Y |
| ciborium docs.rs | docs.rs | High | technical | 2026-05-03 | Y |
| serde_cbor_2 docs.rs | docs.rs | High | technical | 2026-05-03 | Y |
| RFC 8949 (CBOR) | rfc-editor.org / ietf.org | High | official | 2026-05-03 | Y |
| google/coset | github.com | High | industry | 2026-05-03 | N (single confirmer for adoption claim) |
| Android ciborium mirror | android.googlesource.com | High | industry | 2026-05-03 | N (single confirmer) |
| Fedora rust-ciborium package | rpmfind.net | Medium | package | 2026-05-03 | N |
| serde_cbor_core | github.com | Medium-High | industry | 2026-05-03 | N |

Reputation: High: 8 (~73%) | Medium-High: 2 (~18%) | Medium: 1 (~9%) | Avg: ~0.93

## Knowledge Gaps

### Gap 1: Direct head-to-head benchmarks
**Issue**: No published apples-to-apples encode/decode latency benchmark between ciborium 0.2.2 and serde_cbor_2 0.11.2 was found. The qdrant migration thread discusses correctness considerations, not throughput. **Attempted**: Searched GitHub, docs.rs, and crates.io for benchmark crates and microbenchmark gists. **Recommendation**: For Overdrive's small-blob reconciler workload, the encode/decode cost is dominated by the redb fsync, not by CBOR codec choice — benchmarking is unlikely to change the recommendation.

### Gap 2: Exact serde_cbor_2 release cadence
**Issue**: The crates.io page failed to render under WebFetch on multiple attempts; the 333-commit count is observable but per-version dates are not. **Attempted**: Direct crates.io fetch, lib.rs (403). **Recommendation**: Operator can `cargo search serde_cbor_2` locally if exact dates are needed for an ADR; the directional finding (kanidm-active vs ciborium-mature) does not change.

### Gap 3: serde_cbor_2 RFC 8949 statement of intent
**Issue**: The README still names RFC 7049. It is unclear whether this is a stale doc string or a deliberate scope choice. **Attempted**: README fetch. **Recommendation**: For Overdrive's use case, immaterial — both wire formats are forward-compatible; an encoder targeting RFC 7049 produces bytes a RFC 8949 decoder accepts unchanged for the data shapes Overdrive needs.

## Conflicting Information

### Conflict 1: Which crate is the "official" successor to serde_cbor
**Position A**: ciborium is — the archived upstream README explicitly recommends it (alongside minicbor). Source: [pyfisch/cbor README](https://github.com/pyfisch/cbor), high reputation, evidence: archived-banner recommendation text.

**Position B**: serde_cbor_2 is — kanidm forked it and continues the API. Source: [kanidm/cbor README](https://github.com/kanidm/cbor), high reputation, evidence: drop-in-replacement claim.

**Assessment**: Both claims are true at different layers. ciborium is the *upstream-blessed* successor with a fresh design; serde_cbor_2 is a *compatibility-preserving* fork that lets existing call sites compile without rewrites. For new code, the upstream recommendation carries more weight; for code already calling `serde_cbor::to_vec`, the cheap path is serde_cbor_2.

## Recommendation for Overdrive

**Pick `ciborium`** for reconciler View persistence (ADR-0035).

Reasoning grounded in the constraints:

1. **Upstream-blessed successor.** The original maintainer's archive notice points at ciborium; no fork involved; no compatibility-shim baggage.
2. **Wider adoption surface.** Google's COSE implementation, Android platform mirroring, Fedora/Debian packaging — bus factor is higher and serde-roundtrip bugs have more eyes.
3. **API shape is fine for the use case.** Overdrive's runtime owns the round-trip — it writes one helper that calls `ciborium::ser::into_writer(&view, &mut Vec::new())` and `ciborium::de::from_reader(&bytes[..])`. Reconciler authors never see ciborium directly. The "no `to_vec` convenience" point is a one-line wrapper, not a real cost.
4. **Pure Rust, no_std-capable, Apache-2.0** — clean fit with design principle 7 (`overdrive` whitepaper) and the workspace's Apache/MIT licence posture.
5. **Schema-evolution semantics are equivalent** — both libraries flow standard serde attributes through unchanged; the `Option<T>`-as-`null` behaviour is identical between them and is a serde-CBOR data model fact, not a library bug.
6. **No deterministic-encoding requirement.** ADR-0035 keys the redb table on `(reconciler_name, target)`, not on a content hash; `.claude/rules/development.md` already routes hashable data to rkyv. CBOR-determinism is not on the dependency list for reconciler memory.

**When serde_cbor_2 would be the right answer instead**: only if a different Overdrive subsystem already used `serde_cbor` and needed source compatibility during migration. That subsystem does not exist in the current codebase, so the upstream-blessed path is also the lower-effort path.

## Recommendations for Further Research

1. If Overdrive ever moves any persistence format to a content-addressed shape (e.g. WASM-module manifest digests beyond the existing rkyv path), separately evaluate `serde_cbor_core` or compute the canonicalisation in the producer rather than relying on the codec.
2. Track ciborium's 0.3 series if/when it ships — the README's hints about `Value`-shape stability suggest a future major may change the dynamic-value API. Reconciler authors are insulated by the runtime wrapper, so this is a runtime-author concern only.

## Full Citations

[1] enarx. "ciborium — CBOR serialization implementations for serde." GitHub. https://github.com/enarx/ciborium. Accessed 2026-05-03.
[2] kanidm. "cbor — Serde CBOR (serde_cbor_2)." GitHub. https://github.com/kanidm/cbor. Accessed 2026-05-03.
[3] pyfisch. "cbor — CBOR support for serde (archived 2021-08-15)." GitHub. https://github.com/pyfisch/cbor. Accessed 2026-05-03.
[4] qdrant. "Issue #147: serde_cbor is archived." GitHub. 2021-12-14. https://github.com/qdrant/qdrant/issues/147. Accessed 2026-05-03.
[5] ciborium documentation. docs.rs. https://docs.rs/ciborium/latest/ciborium/. Accessed 2026-05-03.
[6] serde_cbor_2 documentation. docs.rs. https://docs.rs/serde_cbor_2/latest/serde_cbor_2/. Accessed 2026-05-03.
[7] Bormann, C. and Hoffman, P. "RFC 8949 — Concise Binary Object Representation (CBOR)." IETF. December 2020. https://www.rfc-editor.org/rfc/rfc8949.html. Accessed 2026-05-03.
[8] google. "coset — Rust types for COSE built on ciborium." GitHub. https://github.com/google/coset. Accessed 2026-05-03.
[9] Android Open Source Project. "Platform mirror: external/rust/crates/ciborium." Google Git. https://android.googlesource.com/platform/external/rust/crates/ciborium/. Accessed 2026-05-03.
[10] Fedora Project. "rust-ciborium 0.2.2-4.fc43 RPM." rpmfind.net. https://rpmfind.net/linux/RPM/fedora/43/x86_64/r/rust-ciborium+default-devel-0.2.2-4.fc43.noarch.html. Accessed 2026-05-03.
[11] gordonbrander. "serde_cbor_core — RFC 8949 Core Deterministic Encoding for Serde." GitHub. https://github.com/gordonbrander/serde_cbor_core. Accessed 2026-05-03.

## Research Metadata

Duration: ~30 min | Examined: 11 sources | Cited: 11 | Cross-refs: 7 of 8 findings | Confidence: High 7, Medium 1, Low 0 | Output: docs/research/rust/ciborium-vs-kanidm-cbor-comprehensive-research.md
