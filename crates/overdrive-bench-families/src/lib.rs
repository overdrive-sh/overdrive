//! The benchmark families (ADR-0296). One module per family under `families/`.
//!
//! RED scaffold created by DISTILL: the connect-hook family declares its
//! benchmark with the attribute pinned by DDD-2 and the declared fixture is
//! absent. Other families land with their slices.
// SCAFFOLD: true

pub mod families {
    pub mod cgroup_connect_hook;
}
