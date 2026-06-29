//! `answer` — the pure `answer_for(name, qtype, &index) -> NameAnswer` decision
//! (dial-by-name-responder, ADR-0072 REV-2 "stable-frontend"; GH #243; roadmap
//! 01-03 / DDN-4). This is THE primary mutation-gate target of the slice.
//!
//! # The REV-2 answer contract
//!
//! Under ADR-0072 REV-2 the responder answers `<job>.svc.overdrive.local` with
//! the workload's **stable frontend address `F`** (a member of
//! [`WORKLOAD_FRONTEND_BASE`](super::frontend_addr_allocator::WORKLOAD_FRONTEND_BASE),
//! `10.98.0.0/16`) — NOT a per-instance backend address in `10.99.0.0/16`. `F`
//! is the [`FrontendAddrAllocator`](super::frontend_addr_allocator::FrontendAddrAllocator)
//! binding the [`NameIndex`] exposes for the name's `<job>`; the dataplane
//! (02-00 re-keyed `MtlsResolve`) later translates `F → a live backend`.
//!
//! `answer_for` is a pure function of `(name, qtype, &index)` — a deterministic
//! read of the allocator's CURRENT `<job> → F` binding through `&index` (it
//! performs NO I/O and NO clock read, and writes nothing), so it is trivially
//! deterministic and DST-replayable. "Pure" here is read-only-over-current-state,
//! not state-independent: the answer reflects whatever binding the allocator
//! holds at call time (the 01-05 assigner is the only writer).
//! It is the single mutation-gate target (DDN-4): the
//! [`Records`](NameAnswer::Records) / [`NxDomain`](NameAnswer::NxDomain) /
//! [`NoData`](NameAnswer::NoData) arms each carry a falsifiable single-stable-F
//! equality the proptests pin.
//!
//! - **`A` query, name RESOLVABLE** (the index holds `name → F`) →
//!   `NameAnswer::Records(vec![SocketAddrV4::new(F, 0)])` — exactly the single
//!   stable frontend addr, NEVER a per-instance backend addr. Single-element
//!   equality is the fail-closed guard (an empty / extra / wrong addr fails).
//! - **`A` query, name WITHHELD/ABSENT** (not in the index — declared-but-not-
//!   running, all-unhealthy, OR unknown all collapse) → `NameAnswer::NxDomain`.
//!   NEVER a stale / cached / guessed / frontend addr.
//! - **`AAAA` query, name RESOLVABLE** → `NameAnswer::NoData` (the substrate is
//!   IPv4; the live name has no v6 record — NODATA, never a fabricated v6 addr).
//! - **`AAAA` query, name WITHHELD/ABSENT** → `NameAnswer::NxDomain`.
//! - Any other qtype on a resolvable name → `NameAnswer::NoData` (the name
//!   exists but carries no record of that type in v1).
//!
//! # Port-to-port (Mandate M2 / M3)
//!
//! `answer_for`'s signature IS the driving port (a pure domain function — per
//! the methodology, calling it directly IS port-to-port at domain scope). It
//! reads the [`NameIndex`] ONLY through its public `frontend_for` query — never
//! the index's internal `by_name` map. The `<job>`-grouping / healthy-gate /
//! List-then-Watch machinery lives in [`name_index`](super::name_index); this
//! module is the pure answer projection over the index's exposed `<job> → F`
//! resolvability.

use std::net::SocketAddrV4;

use hickory_proto::rr::RecordType;
use overdrive_core::id::{MeshServiceName, NameAnswer};

use super::name_index::NameIndex;

/// Compute the pure DNS answer for a dial-by-name query.
///
/// See the module rustdoc for the full REV-2 answer contract. `name` is the
/// dialed [`MeshServiceName`] (`<job>.svc.overdrive.local`); `qtype` is the
/// queried [`RecordType`] (`A` / `AAAA` in v1); `index` is the
/// List-then-Watch [`NameIndex`] mapping each resolvable `<job>` to its stable
/// frontend address `F`.
///
/// Returns:
///
/// - [`NameAnswer::Records`] holding a single `SocketAddrV4::new(F, 0)` —
///   `qtype == A` AND `name` is resolvable (the index holds `name → F`).
///   Exactly ONE addr, the stable frontend `F`.
/// - [`NameAnswer::NoData`] — `name` is resolvable but `qtype != A` (e.g.
///   `AAAA`): the name exists, no record of that type in v1.
/// - [`NameAnswer::NxDomain`] — `name` is NOT resolvable (absent from the
///   index): the answer is WITHHELD regardless of `qtype`.
#[must_use]
pub fn answer_for(name: &MeshServiceName, qtype: RecordType, index: &NameIndex) -> NameAnswer {
    // The name is RESOLVABLE iff the index exposes a stable frontend `F` for it
    // (the healthy-gate WITHHOLD seam is already applied inside `frontend_for`).
    // A withheld/absent name has no `F` ⇒ NxDomain regardless of qtype.
    let Some(frontend) = index.frontend_for(name) else {
        return NameAnswer::NxDomain;
    };
    match qtype {
        // An `A` query on a resolvable name answers exactly the single stable
        // frontend `F` — never a per-instance backend addr.
        RecordType::A => NameAnswer::Records(vec![frontend_socket_addr(frontend)]),
        // Any other type (`AAAA` in v1) on a resolvable name has no record of
        // that type — the name exists, so NODATA, never a fabricated addr.
        _ => NameAnswer::NoData,
    }
}

/// Wrap a stable frontend address `F` into the [`SocketAddrV4`] shape
/// [`NameAnswer::Records`] carries. The port is not load-bearing — the wire
/// encoder ([`super::wire::encode`]) renders only `addr.ip()` into the A
/// record — so it is fixed to `0`. Centralising the wrap here keeps the
/// `F → SocketAddrV4` projection single-sourced.
fn frontend_socket_addr(frontend: std::net::Ipv4Addr) -> SocketAddrV4 {
    SocketAddrV4::new(frontend, 0)
}
