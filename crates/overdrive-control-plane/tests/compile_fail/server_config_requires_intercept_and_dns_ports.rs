//! S-ND295-65 — a server configuration without the protection and DNS ports
//! does not compile (D-295-R16, E16; FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `ServerConfig` constructor and port fields)).
//!
//! `ServerConfig::new` takes the KEK, the `MtlsIntercept` port, and the
//! `GuestDnsFactory` port as required parameters. A configuration built from
//! the KEK alone — the shape that let a server boot without composing
//! protection or DNS — MUST NOT compile. The sibling `.stderr` is the
//! assertion: the one error is that the constructor is not a function of the
//! KEK alone. The fixture coerces the constructor to a KEK-only function
//! pointer rather than calling it, so the diagnostic names the constructor's
//! parameters and not the source file that declares it: file placement is not
//! part of the contract (DISTILL review DR-18).

use std::sync::Arc;

use overdrive_control_plane::ServerConfig;
use overdrive_core::ca::kek::Kek;

fn main() {
    let _from_the_kek_alone: fn(Arc<dyn Kek>) -> ServerConfig = ServerConfig::new;
}
