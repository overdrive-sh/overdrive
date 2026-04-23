# ADR-0016 — `overdrive-host` crate extraction and `adapter-real` → `adapter-host` rename

## Status

Accepted. 2026-04-23.

Amends ADR-0003 (Core-crate labelling via
`package.metadata.overdrive.crate_class`). Does not supersede. The
taxonomy mechanism in ADR-0003 stands unchanged; this ADR records two
coherent changes that happened together in one session:

1. A new `crates/overdrive-host/` was extracted from the
   `overdrive-sim::real::*` module that had been parked inside the sim
   crate behind a `real-adapters` Cargo feature. The feature flag was
   deleted; depending on `overdrive-host` in a `Cargo.toml` is now the
   explicit opt-in to real host I/O.
2. The `crate_class = "adapter-real"` string value was renamed to
   `"adapter-host"`. The class-enum taxonomy (four values, per-crate
   metadata, every-crate-must-declare) is unchanged.

## Context

### Where the real adapters lived before

`overdrive-sim` held both the Sim adapters (`SimClock`, `SimTransport`,
`SimEntropy`, etc.) AND the host-bound real adapters (`SystemClock`,
`OsEntropy`, `TcpTransport`) inside a `src/real/` module. Non-sim
consumers reached them by opting into the `real-adapters` Cargo feature.

Three problems accumulated:

- **Dependency-boundary smell.** `overdrive-sim` is classed
  `adapter-sim` (ADR-0003). Shipping `adapter-host`-shaped code from
  inside an `adapter-sim` crate conflates two classes in one compile
  unit. A future reconciler that wants `SystemClock` cannot add
  `overdrive-sim` as a dep — the sim machinery comes along.
- **Feature-flag as class boundary.** Opting into real adapters was a
  Cargo feature, not a crate boundary. Cargo features unify across the
  workspace; a feature toggled anywhere in a dep graph turns it on
  everywhere. A class boundary wants to be a crate boundary, the only
  place Cargo actually draws a line.
- **Naming mismatch.** Inside `overdrive-sim::real::*`, the files
  named real-world I/O bindings `Real*`. The taxonomy value at the
  Cargo.toml level was `adapter-real`. Both names were DST-internal
  framing — "real vs fake" reads as "real vs what-we-test-against" —
  not an observable property of the code.

### Why the rename (`adapter-real` → `adapter-host`)

The symmetric, descriptive pair for the two non-core, non-binary
classes is `adapter-host | adapter-sim`. It matches:

- the whitepaper's host/guest vocabulary, already established in
  `docs/whitepaper.md` §6 (guest agent for persistent microVMs), §17
  (`vhost-user-fs`, `virtiofsd`, and related host-side surfaces);
- what the newly-extracted crate actually is — host-side bindings for
  the core port traits against the OS / kernel / network;
- what the code does structurally — the axis is host-ness vs
  simulated-host-ness, not realness vs fakeness. Every `Sim*` adapter
  is "real Rust code that really runs"; it just simulates the host.

"Real vs fake" was DST-discourse framing borrowed from the simulation
literature. Inside the codebase, the meaningful distinction is
host-vs-sim, and the class names should say so.

## Decision

### 1. New crate `crates/overdrive-host/`, class `adapter-host`

Holds every production binding of a core port trait to the host
OS / kernel / network:

- `SystemClock` — `Clock` impl over `std::time::Instant` / `SystemTime`.
- `OsEntropy` — `Entropy` impl over the OS RNG.
- `TcpTransport` — `Transport` impl over `tokio::net::*`.
- Future: real `Dataplane`, real `Driver` (Cloud Hypervisor, Wasmtime),
  real `Llm` adapter as Phase 2+ wiring crates land.

Declaration:

```toml
# crates/overdrive-host/Cargo.toml
[package.metadata.overdrive]
crate_class = "adapter-host"
```

The `real-adapters` Cargo feature on `overdrive-sim` is deleted.
Depending on `overdrive-host` in a `Cargo.toml` is the opt-in to real
I/O; there is no second mechanism.

### 2. Taxonomy rename: `adapter-real` → `adapter-host`

The four valid `crate_class` values become:
`core | adapter-host | adapter-sim | binary`.

The rename propagates to:

- `xtask/src/dst_lint.rs` (bail! message, allow-list comparison).
- `xtask/tests/acceptance/dst_lint_banned_apis.rs` (test name and
  fixture assertion strings).
- `crates/overdrive-host/Cargo.toml` (new crate) and
  `crates/overdrive-store-local/Cargo.toml` (existing crate whose label
  moved from `adapter-real` to `adapter-host`).
- `crates/overdrive-host/src/lib.rs` and `crates/overdrive-sim/src/lib.rs`
  module-doc references.
- `CLAUDE.md` repository-structure table and surrounding prose.
- `docs/product/architecture/brief.md` — every occurrence of
  `adapter-real` in the crate-topology section, class table, and
  C4 L2 container diagram.
- `docs/product/architecture/adr-0003-core-crate-labelling.md` — class
  enum + amendment block + examples + "mistyped-value" consequence.
- `docs/product/architecture/adr-0008-rest-openapi-transport.md` — the
  `overdrive-control-plane` crate's declared class.
- `docs/product/architecture/adr-0012-observation-store-server-impl.md`
  — class-label note + enforcement bullet.
- `docs/feature/phase-1-control-plane-core/design/wave-decisions.md`
  — in-flight design doc's crate-declaration line.

### 3. ADR-0004 relationship

ADR-0004 (`overdrive-sim` single crate, not split) covers the sim
crate being one unit. It says nothing about where real adapters live.
The real-adapter extraction does not split `overdrive-sim`; it
extracts code that was previously *inside* `overdrive-sim` into its
own crate. ADR-0004's "one sim crate" decision is unchanged.

### 4. Handling of archived / delivered artifacts

- `docs/evolution/phase-1-foundation-evolution.md` — archived feature
  evolution doc. Two mentions of `adapter-real` were left in the prose
  (describing `crate_class` declarations and the extraction note) and
  a dated nomenclature note was added at the top pointing readers to
  this ADR. Rationale: evolution docs are a historical record of what
  shipped under what name; rewriting the prose rewrites history.
- `docs/feature/phase-1-foundation/deliver/roadmap.json` — delivered
  roadmap with two `adapter-real` occurrences inside `description` and
  `notes` string fields. Left unchanged for the same reason. JSON
  cannot carry an inline footnote; the evolution doc's nomenclature
  note is the canonical marker.

Current/in-flight design artifacts (ADR-0003, ADR-0008, ADR-0012,
`brief.md`, `wave-decisions.md`) are edited in place because they are
live decision records, not historical ones.

## Alternatives considered

### Option A — Keep real adapters in `overdrive-sim` behind a Cargo feature (status quo)

**Rejected.** The feature-flag boundary is weaker than a crate boundary
(Cargo features unify workspace-wide), the dependency-class mismatch
is real (an `adapter-sim` crate shipping `adapter-host`-shaped code),
and it prevents any future reconciler from taking `SystemClock`
without pulling turmoil + StdRng into its compile graph.

### Option B — New crate `overdrive-adapters-real`, keep taxonomy name

**Rejected.** The extraction is clearly correct; the only argument is
the crate's name. "overdrive-adapters-real" hyphenates poorly (three
nouns), does not match the whitepaper's existing host/guest
vocabulary, and perpetuates the "real vs fake" framing that the
taxonomy rename is meant to retire. The whitepaper already says
"host" everywhere the platform's host-side surface is named (§6, §17).

### Option C — Keep `adapter-real` as the taxonomy value but name the crate `overdrive-host`

**Rejected as inconsistent.** Class names should match what the
class *is*; if the class means "host-side adapters" then the value
should say so. Having the crate called `overdrive-host` and the class
called `adapter-real` re-opens the naming mismatch one level up.

### Option D — Extract + rename together (chosen)

See Decision above.

## Consequences

### Positive

- **Compile graph honesty.** Code that depends on
  `overdrive-host` is taking on real host I/O, and the dep graph
  shows it. No hidden Cargo feature making `SystemClock` appear.
- **Dep-class coherence.** `overdrive-sim` returns to being a pure
  `adapter-sim` crate: turmoil, StdRng, SWIM simulator — nothing
  host-bound. `overdrive-host` is a pure `adapter-host` crate: OS
  bindings only, no turmoil.
- **Vocabulary alignment.** The class name, the crate name, and the
  whitepaper's existing host/guest terminology all agree. New
  contributors do not have to learn "real means host here but host
  means something else in §6."
- **Simpler banned-API lint narrative.** `dst-lint` scans `core` crates
  and ignores `adapter-host` / `adapter-sim` / `binary`. The
  allow-list message now reads "permits adapter-host crate using
  Instant::now" — it names the reason, not a vague "real adapter."

### Negative

- **Touch radius.** Six docs + one in-flight design doc + one
  (leave-in-place) archive note + roadmap.json (left alone). Each is
  a simple string rewrite; total is small but nonzero.
- **Legacy archives retain the old name.** The evolution doc and the
  delivered roadmap.json describe `adapter-real`. Mitigated by the
  nomenclature note at the top of the evolution doc and by this ADR
  being linked from ADR-0003's amendment. Anyone encountering
  `adapter-real` in repo history is one hop from the current name.
- **Amendment-plus-new-ADR is two reading locations.** The taxonomy
  mechanism lives in ADR-0003; the rationale for the rename +
  extraction lives here. Mitigated by cross-links at both ends.

### Neutral

- The `crate_class` enum is still closed at four values. Adding a
  future class (`wasm-plugin`, `test-only`, etc.) remains the same
  mechanical change it was in ADR-0003.
- The `adapter-sim` value and the `overdrive-sim` crate name already
  shared a word; the rename makes `adapter-host` + `overdrive-host`
  symmetric on the other side. No new asymmetry introduced.

### Quality-attribute impact

- **Maintainability — modularity**: positive. A crate boundary is a
  stronger modularity mechanism than a Cargo feature. Future
  reconciler/control-plane crates can depend on `overdrive-host`
  without pulling simulation infrastructure into their compile graph.
- **Maintainability — analyzability**: positive. The dep graph
  literally shows which crates take real host I/O. `cargo tree -p
  overdrive-control-plane` either mentions `overdrive-host` or it
  does not; there is no hidden-feature third option.
- **Portability — replaceability**: neutral. The port-trait surface
  is unchanged; swapping host for sim at a wiring site is still a
  one-line change.
- **Performance efficiency**: neutral. Crate boundaries are compile-
  time; no runtime impact.

### Enforcement

- `cargo xtask dst-lint` reads the new class name and produces a
  structured error for any `crate_class` value outside the four
  allowed strings (per ADR-0003's `FromStr` parser mitigation).
- `cargo check --workspace` — green post-rename.
- `cargo xtask dst-lint` — green post-rename; allow-list permits
  `overdrive-host` using `Instant::now`, as the renamed acceptance
  test verifies (`dst_lint_permits_adapter_host_crate_using_instant_now`).
- `cargo nextest run -p xtask --features xtask/integration-tests
  -E 'binary(acceptance)'` — 22/22 green.

## References

- ADR-0003 — `package.metadata.overdrive.crate_class` mechanism and
  its 2026-04-23 Amendment recording the value rename.
- ADR-0004 — single `overdrive-sim` crate; unaffected by the real
  adapters moving out.
- `docs/whitepaper.md` §6 (guest agent), §17 (`vhost-user-fs`) — host
  vocabulary already in use.
- `CLAUDE.md` — repo-structure table reflects the new crate and class.
- Commit `6fa25d2` (phase-1-control-plane-core) and the follow-up
  session that extracted `crates/overdrive-host/`.
