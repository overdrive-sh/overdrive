//! Kernel-side BPF map declarations for Phase 2.2 (XDP service map
//! + Maglev + `REVERSE_NAT` + drop counter).
//!
//! Each map module declares a `#[map]` static of the appropriate
//! `BPF_MAP_TYPE_*` per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 10.
//!
//! **RED scaffolds** — the `#[map]` static declarations are NOT
//! yet emitted; DELIVER lands them per the carpaccio slice plan
//! (Slice 02 / 03 / 04 / 05 / 06). The shared sanity prologue's
//! `DROP_COUNTER` lands in Slice 06.

pub mod backend_map;
pub mod drop_counter;
pub mod hash_of_maps;
pub mod local_backend_map;
pub mod maglev_map;
pub mod reverse_nat_map;
// unconnected-udp-sendmsg4 (GH #200, ADR-0053 rev 2026-06-05) — the
// reply store for the unconnected same-host cgroup path + its miss
// counter. The `cgroup_recvmsg4_service` program (Slice 03) reads the
// map and bumps the counter on a reverse miss.
pub mod reverse_local_map;
pub mod reverse_local_miss_counter;
pub mod service_map;
// transparent-mtls-host-socket (ADR-0069, GH #26). The OUTBOUND intercept's
// destination table (`cgroup_connect4_mtls` reads it) and the forward
// EGRESS-redirect maps (`sk_skb_stream_verdict_mtls` reads them). Userspace
// `HostMtlsEnforcement` writes all four.
pub mod mtls_forward;
pub mod mtls_redirect_dest;
