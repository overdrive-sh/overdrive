//! `overdrive-cli` — library surface for the `overdrive` binary.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`, the CLI exposes a library
//! surface alongside the thin binary entry point (`src/main.rs`). Tests
//! import this library and call handlers (or exercise argv parsing) as
//! Rust functions rather than spawning `overdrive` as a subprocess.
//!
//! The `cli` submodule re-exports the top-level `clap::Parser`-derived
//! `Cli` struct so integration tests can invoke
//! `Cli::try_parse_from([...])` without a process fork — the "argv
//! parsing for the binary wrapper itself" exception in
//! `overdrive-cli/CLAUDE.md`.

#![forbid(unsafe_code)]

pub mod cli;
