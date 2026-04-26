//! CLI command handlers.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`, handlers are plain `async fn`
//! that return typed `Result`s — integration tests call them directly
//! as Rust functions rather than spawning `overdrive` as a subprocess.
//! The binary (`main.rs`) is a thin wrapper that parses `argv`, constructs
//! production adapters, and dispatches into the matching handler here.

pub mod alloc;
pub mod cluster;
pub mod job;
pub mod node;
pub mod serve;
