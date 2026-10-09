# Research: Placement of quint-connect conformance drivers (in-crate vs central harness)

**Date**: 2026-10-09 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium | **Sources**: 11 cited (tool-fetched), several claims marked [unverified]

## Executive Summary

The Rust ecosystem's primary, documented answer for "test code that needs non-public seams" is in-crate `#[cfg(test)]` modules. Anything exposed to a separate crate must be public API, and the only ways to hide that API from production are Cargo features (which unify additively across a workspace and so cannot be guaranteed off in a production build) or `#[doc(hidden)]` (which is a documentation convention, not a barrier). The Quint Connect project itself documents no crate layout: its examples keep the driver in ordinary Rust test code beside the implementation, and `State::from_driver` is expected to read data out of the implementation into spec-shaped types.

For this repo, the recommended placement is a **hybrid that leads with in-crate drivers**: each driver lives with its owner (a `#[cfg(all(test, feature = "integration-tests"))]` module in the owner crate, or the crate's `tests/` dir where the seam is already public), shared Quint/trace plumbing lives in a dev-dependency helper crate, and `tests/conformance` stays reserved for its existing remit (multi-owner, exported-server-handler contracts). The binary-only `overdrive-init` needs no lib split under this option, because unit tests inside `src/` of a bin crate run normally. The one real caveat: the repo's own design rule requires `from_driver` to read accessors *pinned by the design*; those must be real production observation accessors, not `#[cfg(test)]` ones, or the design contract and the driver diverge.

## Research Methodology

**Search Strategy**: web search + page fetch of official docs (Rust Book, Cargo reference, API Guidelines, Quint/quint-connect), maintainer essays (matklad), and rust-analyzer architecture docs; repo read for context (`tests/conformance/CLAUDE.md`, `crates/overdrive-init/Cargo.toml`). A grep of `crates/**/Cargo.toml` found no existing `test-support` feature or `doc(hidden)` precedent in this repo.
**Source Selection**: official language/tool docs (High), maintainer blogs (Medium-High). **Limits**: attempted but could not verify specific claims about tokio/tikv/materialize/madsim/cargo-test-support; see Knowledge Gaps.

## Findings

### Q1. How Rust projects expose test seams across crates

**F1. `#[cfg(test)]` in-crate tests are the language-sanctioned way to reach private items.**
Evidence: "Rust's privacy rules do allow you to test private functions"; unit tests go in `src/` under `#[cfg(test)]`; integration tests "can only call functions that are part of your library's public API."
Source: [The Rust Book, ch. 11.3](https://doc.rust-lang.org/book/ch11-03-test-organization.html), accessed 2026-10-09. Corroboration: [matklad, "Delete Cargo Integration Tests"](https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html) recommends that internal libraries keep tests in `src/` to access non-public APIs. Confidence: High (official + maintainer).

**F2. `cfg(test)` is only set for the crate under test, so it cannot provide a seam to another crate.**
Evidence: integration tests link against the library built in normal configuration; `#[cfg(test)]` library code is not available to them. Source: Rust Book (above) plus the Arti project's coverage page discussing this ([search result](https://arti.torproject.org/coverage/all/crates/arti/build.rs.html), secondary). Consequence: a separate `tests/conformance` crate cannot see `#[cfg(test)] pub(crate)` accessors; it can only use real `pub` items. Confidence: High on the mechanism.

**F3. Test-support Cargo features are additive and unify, so they cannot be guaranteed off in production builds.**
Evidence: "Cargo uses the union of all features enabled on that dependency"; features "should be additive"; with resolver 2 dev-dependencies activate features only when building targets that need them. Source: [Cargo Book, Features](https://doc.rust-lang.org/cargo/reference/features.html). Further: [RFC 3692](https://github.com/rust-lang/rfcs/blob/master/text/3692-feature-unification.md) notes feature selection depends on "the set of packages selected to build", so `cargo build --workspace` or a workspace-wide `--features` run can enable a feature a production binary then inherits; users "can't just run `cargo check --workspace` to verify that the correct set of features are enabled." Confidence: High. Interpretation (mine): a `testing` feature on an owner crate is safe only if nothing in a production dependency path enables it AND CI builds the shipped binary with `-p <bin>` and asserts the feature is absent (e.g. `cargo tree -e features`). Note this repo already runs `--features integration-tests` workspace-wide in places, so a `test-support` feature riding on that flag would be ON in those builds.

**F4. `#[doc(hidden)]` hides from docs only.**
Evidence: API Guidelines list it for "private methods that shouldn't appear in documentation" and stress keeping public surface "small and stable". Source: [Rust API Guidelines, Future proofing](https://rust-lang.github.io/api-guidelines/future-proofing.html). It does not prevent calls; this repo's CLAUDE.md and `tests/conformance/CLAUDE.md` ("add a test-only production API" is forbidden) already reject it as a seam. Confidence: High on semantics.

**F5. Real precedents for feature-gated test surfaces exist but are mostly about runtime facilities, not owner internals.**
- Tokio exposes `test-util`: "Enables testing based infrastructure for the Tokio runtime" ([docs.rs/tokio feature flags](https://docs.rs/tokio/latest/tokio/#feature-flags)), but the page says nothing on production risk. Medium confidence on role only.
- tikv `fail-rs` generates failpoints only "with the `failpoints` Cargo feature" ([github.com/tikv/fail-rs](https://github.com/tikv/fail-rs)); no unification warning on the page.
- rust-analyzer tests at the `ide` API boundary and keeps a `test_utils` crate for shared helpers ([architecture.md mirror](https://git.joshthomas.dev/language-servers/rust-analyzer/raw/commit/a044ff0138d6bff9406b94de89fde43e7672ee1b/docs/dev/architecture.md)); its test boundary is a deliberately *public* API, not a private seam.
Confidence: Medium (single mirror/secondary pages; I could not verify tikv/materialize/databend/cargo-test-support layouts, see gaps).

### Q2. Model-based testing driver placement and state observation

**F6. quint-connect prescribes no crate layout; the driver is test-side code holding the implementation, and state comes from `from_driver`.**
Evidence: "Tests live alongside your implementation in standard Rust test modules" (repo summary); the `Driver` trait needs a `step`, and the `State` trait's `from_driver(driver)` converts runtime state into spec-shaped types. [github.com/quint-co/quint-connect](https://github.com/quint-co/quint-connect); [Quint blog, Quint Connect](https://quint.sh/posts/quint_connect) ("We extract data from the concrete implementation into types that match the spec"; the page gives no guidance on exposing internals or multi-crate layout). Also `Config { state: &[...] }` path-narrowing selects which part of the spec state is compared, without exposing the whole implementation. Confidence: Medium (docs read via summarizer; no explicit placement guidance exists to find).

**F7. Modelator / TLA+ trace validation / stateright placement.** Searches surfaced only Quint Connect material (docs.rs, Elixir/TS ports); no Informal Systems or stateright guidance on driver placement was found. Treated as a gap.

### Q3. Binary-only crates

**F8. lib+thin-bin is the documented standard for integration-testing a binary's logic from outside.**
Evidence: a binary-only crate cannot be imported by `tests/`; the Book recommends a thin `main.rs` over `lib.rs`: "integration tests can test the library crate". Source: [Rust Book 11.3](https://doc.rust-lang.org/book/ch11-03-test-organization.html). Confidence: High.
Downsides (mine, partly repo-derived): the split creates a library target that other crates *can* depend on, which contradicts `overdrive-init`'s documented stance, "Not a library any other crate imports" (Cargo.toml header, class `binary`); every `pub` item needed by the harness becomes semver/API surface and a candidate for misuse; and it is only needed if the test lives outside the crate. Unit tests inside `src/` of a bin crate need none of it.

### Q4. Fidelity, API surface, maintenance

- Fidelity ("production code, not a copy"): identical in both options provided the driver calls the real owner type. The risk differs: a central crate with a *public test constructor* tends to grow a second constructor path (test-only wiring) that production never uses, which is the failure mode CLAUDE.md "Production code is not shaped by simulation" warns about; an in-crate driver uses the same private constructor production's own `new` delegates to.
- Surface: in-crate adds zero public API; central adds one per owner plus a lib target for `overdrive-init`.
- matklad's counterpoint ([How to Test](https://matklad.github.io/2021/05/31/how-to-test.html)): test at stable boundaries so tests survive refactors. That argues for the driver reading only *pinned observable accessors* (which the repo's design rule already requires), whichever crate holds it.

## Comparison

| Criterion | 1. In-crate drivers | 2a. Central, public ctor | 2b. Central, feature-gated | 2c. Central, `doc(hidden)` | 3. Hybrid (in-crate drivers + shared helper dev-crate) |
|---|---|---|---|---|---|
| New public API in production build | None | Yes, permanent | Compiled out only if feature off; unification can enable it (F3) | Yes, permanent | None |
| Binary-only `overdrive-init` | Works as-is | Needs lib target | Needs lib target | Needs lib target | Works as-is |
| Uses production constructor | Yes | Yes (public) | Yes | Yes | Yes |
| Discoverability | Scattered, near owner | One place | One place | One place | Owner-local drivers; shared helper in one place |
| Shared Quint/trace plumbing | Duplicated unless extracted | Native | Native | Native | Extracted to dev-dep crate |
| Fits `tests/conformance` charter (multi-owner, public server API) | n/a | Violates "no test-only production API" | Same | Same | Yes, charter untouched |
| CI lane | Existing owner-crate `integration-tests` lane under Lima | Root crate lane | Same | Same | Owner-crate lane |
| `conformance-lint` (scans `tests/conformance/tests/*conformance*.rs`) | Must be extended to new paths | Works unchanged | Works | Works | Extend globs |
| Key risk | Dev-dep cycle / duplicate-crate-instance if a sim crate depends on the owner crate [unverified] | API creep | Feature leak to prod | Convention only | Helper crate must stay dev-dependency-only |

## Recommendation

Adopt **Option 3 (hybrid, in-crate drivers)**:

1. **Driver location**: in the owner's crate, gated `#[cfg(all(test, feature = "integration-tests"))]` (matching the existing testing.md rule that drivers compile under `integration-tests` and use `#[ignore = "pending DELIVER ..."]`, never an always-false cfg). Where the seam is already public, the crate's `tests/` dir is equally fine.
2. **Binary-only crate**: keep `overdrive-init` bin-only; its driver is a `src/` test module. No lib target, no new public items, consistent with its "not a library" declaration.
3. **No production API change**: no `test-support` feature and no `doc(hidden)` seam. This satisfies "no test-only public API in production builds" structurally rather than by feature discipline (F3, F4).
4. **Shared plumbing**: put trace-runner glue, seed printing, and common `Sim*` wiring into a dev-dependency-only helper (existing `overdrive-testing` is the precedent for real-OS fixtures; a pure-sim helper could live in `overdrive-sim`). Never in `[dependencies]` (development.md port-trait rule).
5. **Observation accessors**: the repo's design rule says `from_driver` reads accessors the design pins. Make those real production observation/audit accessors (public where they are part of the owner's contract). Do **not** satisfy a pinned accessor with a `#[cfg(test)] pub(crate)` read-only accessor, or the driver reads something the design never pinned and production cannot be shown to expose. `#[cfg(test)] pub(crate)` is acceptable only for variables the design explicitly marks as model-only/ghost or for the marked Sim-arbiter exception.
6. **Keep `tests/conformance`** for cross-owner, exported-handler contracts as its CLAUDE.md states; do not turn it into the home of per-owner Quint drivers.
7. **Process**: extend `cargo xtask conformance-lint` globs to the in-crate driver files so the checks for gating, empty handlers, and Sim reads still apply; record the driver path per owner in the DISTILL scenario spec to offset scattering.

When option 2 would be justified: only if one driver must orchestrate several owners across crates (then it is a `tests/conformance` scenario using exported public constructors, not a seam).

## Source Analysis

| Source | Domain | Reputation | Type | Accessed | Cross-verified |
|---|---|---|---|---|---|
| Rust Book 11.3 | doc.rust-lang.org | High | official | 2026-10-09 | Y (matklad) |
| Cargo Book, Features | doc.rust-lang.org | High | official | 2026-10-09 | Y (RFC 3692) |
| RFC 3692 | github.com/rust-lang/rfcs | High | official | 2026-10-09 | Y |
| API Guidelines | rust-lang.github.io | High | official | 2026-10-09 | N |
| quint-connect repo | github.com/quint-co | Medium-High | official project | 2026-10-09 | Y (blog) |
| Quint blog | quint.sh | Medium-High | project | 2026-10-09 | Y |
| matklad x2 | matklad.github.io | Medium-High | maintainer | 2026-10-09 | Y (Book) |
| tokio docs | docs.rs | High | official | 2026-10-09 | N |
| fail-rs | github.com/tikv | Medium-High | project | 2026-10-09 | N |
| rust-analyzer arch (mirror) | git.joshthomas.dev | Medium | mirror | 2026-10-09 | N |
| Arti coverage page | arti.torproject.org | Medium | project | 2026-10-09 | N |

Content was fetched through a summarizing model, so quotes are as returned; no prompt-injection patterns were observed.

## Knowledge Gaps

- **Gap 1: Large-project precedents.** Could not verify how materialize, databend, sled, madsim/turmoil users, or cargo's `cargo-test-support` structure their seams. Attempted: targeted searches (no direct hits). Recommendation: inspect those repos' `Cargo.toml` features directly.
- **Gap 2: Quint/Informal Systems/Modelator/stateright placement guidance.** None found beyond the quint-connect README and blog; its Emerald example not inspected. Recommendation: read the quint-connect `examples/` and the Emerald MBT tests' crate layout.
- **Gap 3: Dev-dependency cycle behaviour.** The claim that a dev-dependency on a crate that itself depends on the crate under test yields two distinct crate instances (breaking trait/type identity) is from general knowledge, not a fetched source. Check each owner crate against `overdrive-sim`'s dependency graph (`cargo tree -p overdrive-sim`) before choosing in-crate placement.
- **Gap 4: Mutation-testing interaction.** Whether cargo-mutants runs `integration-tests`-gated in-crate driver modules per mutation was not researched.

## Recommendations for Further Research

1. Dependency-graph check per candidate owner crate vs `overdrive-sim` (Gap 3).
2. Read quint-connect `examples/` for any in-tree layout convention (Gap 2).

## Full Citations

[1] Rust Project. "Test Organization". The Rust Programming Language. https://doc.rust-lang.org/book/ch11-03-test-organization.html. Accessed 2026-10-09.
[2] Rust Project. "Features". The Cargo Book. https://doc.rust-lang.org/cargo/reference/features.html. Accessed 2026-10-09.
[3] Rust Project. "RFC 3692: Feature Unification". https://github.com/rust-lang/rfcs/blob/master/text/3692-feature-unification.md. Accessed 2026-10-09.
[4] Rust API Guidelines. "Future proofing". https://rust-lang.github.io/api-guidelines/future-proofing.html. Accessed 2026-10-09.
[5] quint-co. "quint-connect". https://github.com/quint-co/quint-connect. Accessed 2026-10-09.
[6] Quint. "Quint Connect". https://quint.sh/posts/quint_connect. Accessed 2026-10-09.
[7] matklad. "Delete Cargo Integration Tests". https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html. Accessed 2026-10-09.
[8] matklad. "How to Test". https://matklad.github.io/2021/05/31/how-to-test.html. Accessed 2026-10-09.
[9] Tokio. "Feature flags". https://docs.rs/tokio/latest/tokio/#feature-flags. Accessed 2026-10-09.
[10] tikv. "fail-rs". https://github.com/tikv/fail-rs. Accessed 2026-10-09.
[11] rust-analyzer. "architecture.md" (mirror). https://git.joshthomas.dev/language-servers/rust-analyzer/raw/commit/a044ff0138d6bff9406b94de89fde43e7672ee1b/docs/dev/architecture.md. Accessed 2026-10-09.

## Research Metadata
Examined ~25 | Cited 11 | Confidence: High 40%, Medium 55%, Low 5% | Output: docs/research/testing/conformance-driver-placement-research.md
