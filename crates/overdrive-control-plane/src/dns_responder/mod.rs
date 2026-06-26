//! `dns_responder` — the in-agent dial-by-name DNS layer (ADR-0072, GH #243).
//!
//! The responder is the **third reader** of the `ObservationStore`
//! `service_backends` surface (outbound resolve · inbound install · name
//! answers, D-TME-11) — it answers `<job>.svc.overdrive.local` queries with a
//! running-AND-healthy IPv4 backend addr, NODATA for AAAA-on-live, and
//! NXDOMAIN when no running-and-healthy backend exists.
//!
//! # Module map (per ADR-0072 § Component decomposition)
//!
//! Step 01-02 lands ONLY [`wire`] — the `hickory-proto` DNS codec behind the
//! DDN-4 / D-DBN-5 anti-corruption boundary (decode the inbound query; encode
//! the A / NODATA-SOA / NXDOMAIN-SOA reply). The remaining components are
//! later slices and are NOT declared here yet (the MANIFEST forbids
//! pre-declaring modules that don't exist):
//!
//! - `answer.rs` — the pure `answer_for(name, qtype, &index) -> NameAnswer`
//!   (the mutation-gate target).
//! - `name_index.rs` — the name-keyed `NameIndex` (List-then-Watch sibling
//!   reader over the `service_backends` rows).
//! - `responder.rs` — the `DnsResponder` host adapter (bind + `IP_PKTINFO`
//!   recv/sendmsg loop).
//!
//! Step 01-02 lands `wire` GREEN — its `encode`/`decode` are fully
//! implemented. Step 01-04 lands `frontend_addr_allocator` GREEN — its
//! `assign`/`release`/`snapshot` bodies are fully implemented, so the module
//! carries no `clippy::todo` scaffold expectation. Step 01-03 lands `answer`
//! (the pure `answer_for`) and `name_index` (the List-then-Watch `NameIndex`
//! that maps each resolvable `<job>` to its stable frontend addr `F`); both
//! are fully implemented GREEN in the same slice. Step 01-05 lands
//! `boot_rebuild` — the empty-on-boot converge-on-boot rebuild that
//! re-populates the [`frontend_addr_allocator::FrontendAddrAllocator`] from
//! the declared-Service intent SSOT (the writer's boot half; the
//! assign-on-declare half lives in the `submit_workload` Service arm).
//! Step 02-01 lands `responder` — the `DnsResponder` host adapter (the
//! wildcard-first / per-gateway-addr-fallback bind + the `recvmsg`/`sendmsg`
//! `IP_PKTINFO` source-pinned serve loop) wired into `run_server` behind the
//! Earned-Trust probe gate (DDN-6).

pub mod answer;
pub mod boot_rebuild;
pub mod frontend_addr_allocator;
pub mod name_index;
pub mod responder;
pub mod wire;
