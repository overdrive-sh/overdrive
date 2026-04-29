# Overdrive — Style & Conventions

## Rust conventions
- `thiserror` for typed errors in libs; `eyre` only at CLI/binary boundaries.
- Every lib crate exports `pub type Result<T, E = Error> = std::result::Result<T, E>`.
- Newtypes STRICT: `JobId`, `AllocationId`, `NodeId`, `SpiffeId`, `Region`, `ContentHash`, etc.
- Every newtype: `FromStr` (case-insensitive for human-typed IDs), `Display`, `Serialize`/`Deserialize`, validating constructors.
- `BTreeMap` default; `HashMap` only with `// dst-lint: hashmap-ok <reason>`.
- `parking_lot::RwLock`/`Mutex` for sync; `tokio::sync` only when lock must cross `.await`.
- No `Instant::now()`/`SystemTime::now()` in core — use `Clock` trait / `tick.now`.
- No `rand::random()`/`thread_rng()` in core — use `Entropy` trait.
- No `TcpStream`/`TcpListener` in core — use `Transport` trait.
- `async_trait` for async trait methods.
- `rkyv` for durable hot-path reads; `bumpalo` for reconciler scratch.

## Comments
- No block comments. One-line `///` on public items only when WHY is non-obvious.
- No aspirational docs. `SCAFFOLD:` marker for intentional stubs.
- `// mutants: skip` with one-line justification for untestable blocks.

## Error handling
- Library: `thiserror` enum, pass-through `#[from]`, consistent constructors.
- Binary: `eyre::Result<T>`, `color-eyre`, `.expect()` in `main()`.
- Never `anyhow` — use `eyre` at binary boundaries.

## Integration tests
- EVERY workspace member declares `integration-tests = []` in `[features]`.
- Integration tests go in `tests/integration/<scenario>.rs` gated by `#![cfg(feature = "integration-tests")]`.
- Tests mutating process globals use `#[serial(env)]` + RAII guard.

## Reconciler rules
- `reconcile()` is sync, pure, NO `.await`, NO I/O, NO direct libSQL writes.
- All libSQL reads in `hydrate()` (async).
- Wall-clock from `tick.now` (TickContext), NEVER `Instant::now()`.
- External calls emit `Action::HttpCall`; responses arrive as observation on next tick.

## Features
- `integration-tests = []` in EVERY workspace member (no-op if no integration tests).
