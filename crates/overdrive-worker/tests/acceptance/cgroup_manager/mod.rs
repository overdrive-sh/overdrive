//! `CgroupManager` acceptance scenarios per ADR-0054 § D5 + DISTILL
//! § E1 CONVERT rows.
//!
//! PORT-TO-PORT discipline: every scenario constructs
//! `Arc<dyn CgroupFs>` from `Arc<SimCgroupFs>` and exercises
//! `CgroupManager` over the port surface. Snapshot inspection through
//! `SimCgroupFs::snapshot()` is the assertion mechanism — the trait
//! IS the contract.

mod create_workload_scope;
mod place_pid_in_scope;
mod workloads_slice_bootstrap;
mod write_resource_limits;
mod write_resource_limits_warn_on_error;
