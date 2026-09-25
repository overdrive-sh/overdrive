//! S-ND295-65 — a server configuration without the protection and DNS ports
//! does not compile (D-295-R16, E16; FD 4372-4386).
//!
//! `ServerConfig::new` takes the KEK, the `MtlsIntercept` port, and the
//! `GuestDnsFactory` port as required parameters. A configuration built from
//! the KEK alone — the shape that let a server boot without composing
//! protection or DNS — MUST NOT compile. The sibling `.stderr` is the
//! assertion: the one error is the missing two arguments.

use std::sync::Arc;

use overdrive_control_plane::ServerConfig;
use overdrive_core::ca::kek::Kek;

fn server_config_from_the_kek_alone(kek: Arc<dyn Kek>) -> ServerConfig {
    ServerConfig::new(kek)
}

fn main() {
    let _ = server_config_from_the_kek_alone;
}
