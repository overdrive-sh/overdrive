Read the entire @docs/whitepaper.md file

## Development Paradigm

This project follows the **object-oriented** paradigm. Use @nw-software-crafter for implementation.

## Repository structure

Workspace crates live under `crates/` (plus `xtask/` for build tooling).
Each crate declares `package.metadata.overdrive.crate_class` per
ADR-0003; the dst-lint gate (`xtask/src/dst_lint.rs`) scans only
`crate_class = "core"` crates for banned real-infra calls.

| Crate | Class | What's in it |
|---|---|---|
| `overdrive-core` | `core` | Newtypes, error types, **port traits** (`Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `IntentStore`, `ObservationStore`, `Llm`). No I/O. No `tokio`, `rand`, or `std::net` in the dependency graph — if it would fail dst-lint, it can't live here. |
| `overdrive-host` | `adapter-host` | Production bindings from the core port traits to the host OS / kernel / network (`SystemClock`, `OsEntropy`, `TcpTransport`, etc.). Reconciler and policy crates MUST NOT depend on this — depending on `overdrive-host` is the explicit opt-in to real I/O. |
| `overdrive-sim` | `adapter-sim` | `Sim*` bindings for the same traits, the turmoil DST harness, and the invariant catalogue. Owns `turmoil` and `StdRng` — nothing else should. See ADR-0004. |
| `overdrive-store-local` | `adapter-host` | `LocalStore` (single-node `IntentStore` over `redb`). |
| `overdrive-cli` | `binary` | Operator CLI entry point; `eyre`-based error reporting. |
| `xtask` | `binary` | Build/lint/DST runner (`cargo xtask …`). Allowed to touch the filesystem and wall-clock; not scanned by dst-lint. |

**The sim/host split is load-bearing, not cosmetic.** `overdrive-core`
depends only on the trait surface; wiring crates (future
`overdrive-node`, `overdrive-control-plane`, gateway) pick host impls
for production and sim impls under tests. Anything that would put
`tokio::net::*`, `Instant::now`, or `rand::thread_rng` on a
`core`-class compile path fails dst-lint at PR time.

The four valid crate classes are `core | adapter-host | adapter-sim |
binary` — nothing else. Adding a new crate means picking one of those
up front and declaring it in the crate's `Cargo.toml`.

Non-code layout:

- `docs/whitepaper.md` — SSOT for platform design (§ references in
  ADRs and rules point here).
- `docs/product/architecture/adr-*.md` — accepted architectural
  decisions. Editing an ADR or supersession goes through the
  architect agent, not inline.
- `docs/product/architecture/brief.md`, `docs/product/commercial.md`
  — SSOT for scope and commercial shape (tenancy, licensing, tiers).
- `docs/feature/{slug}/{wave}/…` — in-flight nWave artifacts (discuss
  → distill → design → deliver). Temporary; archived into
  `docs/evolution/` when the feature is finalised.
- `docs/research/` — research notes and evidence.
- `.claude/rules/{development,testing}.md` — project-wide Rust and
  testing discipline. These override defaults for agents working in
  this repo.

## Rust library conventions

Every library crate that defines its own error type also exposes a matching
`Result` alias alongside it, so call sites never have to name the error type:

```rust
/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
```

Usage:

- Internal code writes `fn foo(...) -> Result<Foo>` (no error generic), and
  `?` propagates anything that converts via `thiserror`'s `#[from]`.
- Cross-crate callers either write `overdrive_core::Result<T>` or import
  the alias. They never re-declare `std::result::Result<T, SomeError>`.
- Override the default when a function returns a different error type
  explicitly: `fn bar() -> Result<Bar, OtherError>`.
- Binary boundaries (`overdrive-cli`, `xtask`) drop this pattern and return
  `eyre::Result<T>` instead — see `crates/overdrive-cli/src/main.rs`.

This keeps the typed-error discipline from `.claude/rules/development.md`
intact while removing the noise of repeating the error type at every call
site.

## Mutation Testing Strategy

This project uses **per-feature** mutation testing. Per-PR runs are diff-scoped via `cargo mutants --in-diff origin/main` with a kill-rate gate of ≥80%. A nightly job runs the full workspace against the baseline in `mutants-baseline/main/` to catch drift. Mutations to `unsafe` blocks, `aya-rs` eBPF programs, generated code, and async scheduling logic are excluded per `.claude/rules/testing.md`.

## Roadmap validator warnings

`des.cli.roadmap validate` flags length-limit warnings (`STEP_NAME_TOO_LONG`, `CRITERIA_TOO_LONG`, `DESCRIPTION_TOO_LONG`) that are cosmetic and non-blocking — the validator exits 0 anyway. Overdrive roadmap ACs deliberately carry scenario-level specificity (test names, invariant names, proptest targets, kill-rate thresholds), and tightening them to the defaults would lose traceability. Ignore these warnings; do not ask the crafter to trim them.
