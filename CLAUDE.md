Read the entire @docs/whitepaper.md file

## Development Paradigm

This project follows the **object-oriented** paradigm. Use @nw-software-crafter for implementation.

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
