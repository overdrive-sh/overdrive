//! Class B acceptance scenarios + F1 K3 determinism guard for
//! `SimCgroupFs` per ADR-0054 § Sim adapter (step 01-03).
//!
//! PORT-TO-PORT discipline: every scenario constructs
//! `Arc<dyn CgroupFs>` and exercises the trait surface. The concrete
//! `Arc<SimCgroupFs>` is retained alongside via `Arc::clone` solely
//! for `snapshot()` / `inject_error()` test hooks — the trait IS the
//! contract.

mod create_dir;
mod error_schedule;
mod k3_determinism;
mod kind;
mod probe;
mod remove_dir;
mod write;
