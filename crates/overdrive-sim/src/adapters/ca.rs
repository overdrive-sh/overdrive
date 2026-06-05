//! `SimCa` — in-memory [`Ca`] adapter for DST (ADR-0063 D1, D7).
//!
//! The sim adapter loads a **pre-generated fixture P-256 key via PEM**
//! (research Finding 11 — key generation is a host-adapter concern and is NOT
//! injectable; DST uses fixture keys) and draws certificate serials through
//! the seeded [`Entropy`] port. The whole point: issuance composes
//! **bit-identically from a seed** (KPI K5) — two `SimCa` over the same seed
//! draw the same serial bytes and therefore produce a byte-identical
//! [`RootCaHandle`].
//!
//! `overdrive-sim` carries no `rcgen` / `ring` dependency and must not gain
//! one, so `SimCa` treats the fixture material as **opaque bytes** — it never
//! parses real crypto. The fixture root key + cert are embedded as inline
//! `const`s (a real `openssl`-generated P-256 self-signed root); the bytes are
//! deterministic, which is all the sim needs.
//!
//! # Dependency discipline
//!
//! `SimCa::new` takes its [`Entropy`] source as a **required constructor
//! parameter** — no builder, no production-binding default (`.claude/rules/
//! development.md` § "Port-trait dependencies"). A caller that forgets to
//! inject entropy fails to compile.

use std::sync::Arc;

use overdrive_core::CertSerial;
use overdrive_core::traits::ca::{
    Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, IntermediateHandle, RootCaHandle, SvidMaterial,
    SvidRequest, TrustBundle,
};
use overdrive_core::traits::entropy::Entropy;

/// Pre-generated fixture root signing key (P-256, PEM). Held opaquely — the
/// sim never parses it; it exists so the [`RootCaHandle`]'s sign-capability
/// material is realistic fixture bytes.
const FIXTURE_ROOT_KEY_PEM: &str = "-----BEGIN EC PRIVATE KEY-----\n\
MHcCAQEEIIlF/A8AVlDmKlM1emtYp6alEyYKyfbMZwhZUGRExBvpoAoGCCqGSM49\n\
AwEHoUQDQgAEJkph+YTCV7Eq9drSyuaF9R5VbqBLN7s80/Q9AwTDJJZhScACTtmZ\n\
gQTAjB0+Z63GhRd8Ijh/E7LjNp1M4fg8Yg==\n\
-----END EC PRIVATE KEY-----\n";

/// Pre-generated fixture self-signed root certificate (PEM), over
/// [`FIXTURE_ROOT_KEY_PEM`]. `CA:TRUE`, `keyCertSign`+`cRLSign` critical, no
/// pathLen — the root profile shape (ADR-0063, [`CertRole::Root`]).
const FIXTURE_ROOT_CERT_PEM: &str = "-----BEGIN CERTIFICATE-----\n\
MIIBpTCCAUugAwIBAgIUPQr03J0po2Qc76LsCvar89RgTmEwCgYIKoZIzj0EAwIw\n\
IDEeMBwGA1UECgwVb3ZlcmRyaXZlLXNpbS1maXh0dXJlMB4XDTI2MDYwNTA3Mzgw\n\
OFoXDTM2MDYwMjA3MzgwOFowIDEeMBwGA1UECgwVb3ZlcmRyaXZlLXNpbS1maXh0\n\
dXJlMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEJkph+YTCV7Eq9drSyuaF9R5V\n\
bqBLN7s80/Q9AwTDJJZhScACTtmZgQTAjB0+Z63GhRd8Ijh/E7LjNp1M4fg8YqNj\n\
MGEwHQYDVR0OBBYEFPDFrOSZNnfMXGWc3wfTthxEZa6iMB8GA1UdIwQYMBaAFPDF\n\
rOSZNnfMXGWc3wfTthxEZa6iMA8GA1UdEwEB/wQFMAMBAf8wDgYDVR0PAQH/BAQD\n\
AgEGMAoGCCqGSM49BAMCA0gAMEUCIQD8XQfI05RhzcAeCsK3Mfk7KfyD3ukAvDvj\n\
cGu6pRQytgIgXYDlOl0Z3ZVCLhdYL0a7nWf+NVZq+5uxELIqGzWNk5E=\n\
-----END CERTIFICATE-----\n";

/// Pre-generated fixture self-signed root certificate (DER bytes), the binary
/// X.509 form of [`FIXTURE_ROOT_CERT_PEM`].
const FIXTURE_ROOT_CERT_DER: &[u8] = &[
    0x30, 0x82, 0x01, 0xa5, 0x30, 0x82, 0x01, 0x4b, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x14, 0x3d,
    0x0a, 0xf4, 0xdc, 0x9d, 0x29, 0xa3, 0x64, 0x1c, 0xef, 0xa2, 0xec, 0x0a, 0xf6, 0xab, 0xf3, 0xd4,
    0x60, 0x4e, 0x61, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x30,
    0x20, 0x31, 0x1e, 0x30, 0x1c, 0x06, 0x03, 0x55, 0x04, 0x0a, 0x0c, 0x15, 0x6f, 0x76, 0x65, 0x72,
    0x64, 0x72, 0x69, 0x76, 0x65, 0x2d, 0x73, 0x69, 0x6d, 0x2d, 0x66, 0x69, 0x78, 0x74, 0x75, 0x72,
    0x65, 0x30, 0x1e, 0x17, 0x0d, 0x32, 0x36, 0x30, 0x36, 0x30, 0x35, 0x30, 0x37, 0x33, 0x38, 0x30,
    0x38, 0x5a, 0x17, 0x0d, 0x33, 0x36, 0x30, 0x36, 0x30, 0x32, 0x30, 0x37, 0x33, 0x38, 0x30, 0x38,
    0x5a, 0x30, 0x20, 0x31, 0x1e, 0x30, 0x1c, 0x06, 0x03, 0x55, 0x04, 0x0a, 0x0c, 0x15, 0x6f, 0x76,
    0x65, 0x72, 0x64, 0x72, 0x69, 0x76, 0x65, 0x2d, 0x73, 0x69, 0x6d, 0x2d, 0x66, 0x69, 0x78, 0x74,
    0x75, 0x72, 0x65, 0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
    0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04, 0x26, 0x4a,
    0x61, 0xf9, 0x84, 0xc2, 0x57, 0xb1, 0x2a, 0xf5, 0xda, 0xd2, 0xca, 0xe6, 0x85, 0xf5, 0x1e, 0x55,
    0x6e, 0xa0, 0x4b, 0x37, 0xbb, 0x3c, 0xd3, 0xf4, 0x3d, 0x03, 0x04, 0xc3, 0x24, 0x96, 0x61, 0x49,
    0xc0, 0x02, 0x4e, 0xd9, 0x99, 0x81, 0x04, 0xc0, 0x8c, 0x1d, 0x3e, 0x67, 0xad, 0xc6, 0x85, 0x17,
    0x7c, 0x22, 0x38, 0x7f, 0x13, 0xb2, 0xe3, 0x36, 0x9d, 0x4c, 0xe1, 0xf8, 0x3c, 0x62, 0xa3, 0x63,
    0x30, 0x61, 0x30, 0x1d, 0x06, 0x03, 0x55, 0x1d, 0x0e, 0x04, 0x16, 0x04, 0x14, 0xf0, 0xc5, 0xac,
    0xe4, 0x99, 0x36, 0x77, 0xcc, 0x5c, 0x65, 0x9c, 0xdf, 0x07, 0xd3, 0xb6, 0x1c, 0x44, 0x65, 0xae,
    0xa2, 0x30, 0x1f, 0x06, 0x03, 0x55, 0x1d, 0x23, 0x04, 0x18, 0x30, 0x16, 0x80, 0x14, 0xf0, 0xc5,
    0xac, 0xe4, 0x99, 0x36, 0x77, 0xcc, 0x5c, 0x65, 0x9c, 0xdf, 0x07, 0xd3, 0xb6, 0x1c, 0x44, 0x65,
    0xae, 0xa2, 0x30, 0x0f, 0x06, 0x03, 0x55, 0x1d, 0x13, 0x01, 0x01, 0xff, 0x04, 0x05, 0x30, 0x03,
    0x01, 0x01, 0xff, 0x30, 0x0e, 0x06, 0x03, 0x55, 0x1d, 0x0f, 0x01, 0x01, 0xff, 0x04, 0x04, 0x03,
    0x02, 0x01, 0x06, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x03,
    0x48, 0x00, 0x30, 0x45, 0x02, 0x21, 0x00, 0xfc, 0x5d, 0x07, 0xc8, 0xd3, 0x94, 0x61, 0xcd, 0xc0,
    0x1e, 0x0a, 0xc2, 0xb7, 0x31, 0xf9, 0x3b, 0x29, 0xfc, 0x83, 0xde, 0xe9, 0x00, 0xbc, 0x3b, 0xe3,
    0x70, 0x6b, 0xba, 0xa5, 0x14, 0x32, 0xb6, 0x02, 0x20, 0x5d, 0x80, 0xe5, 0x3a, 0x5d, 0x19, 0xdd,
    0x95, 0x42, 0x2e, 0x17, 0x58, 0x2f, 0x46, 0xbb, 0x9d, 0x67, 0xfe, 0x35, 0x56, 0x6a, 0xfb, 0x9b,
    0xb1, 0x10, 0xb2, 0x2a, 0x1b, 0x35, 0x8d, 0x93, 0x91,
];

/// Number of random bytes drawn for a certificate serial — 128 bits, well
/// above the CA/B Forum 64-bit floor (research Finding 10).
const SERIAL_BYTES: usize = 16;

/// In-memory [`Ca`] adapter for DST.
///
/// Holds the fixture root material as opaque bytes and a seeded [`Entropy`]
/// source. `Send + Sync` (the `Arc<dyn Entropy>` is `Send + Sync`), matching
/// the sibling sim adapters, so it can be shared across async tasks.
pub struct SimCa {
    entropy: Arc<dyn Entropy>,
}

impl SimCa {
    /// Construct a `SimCa` over a required [`Entropy`] source.
    ///
    /// No builder, no default — the entropy dependency is mandatory at
    /// construction so a test that forgets to inject it fails to compile
    /// (`.claude/rules/development.md` § "Port-trait dependencies"). The
    /// fixture root key/cert are embedded `const`s, not constructor inputs.
    #[must_use]
    pub fn new(entropy: Arc<dyn Entropy>) -> Self {
        Self { entropy }
    }

    /// Draw a fresh certificate serial from the seeded entropy source.
    ///
    /// Lowercase hex of [`SERIAL_BYTES`] random bytes — satisfies
    /// [`CertSerial`]'s validator (even-length lowercase hex, ≤20 bytes). The
    /// draw is the load-bearing seed dependency: two `SimCa` over the same
    /// seed draw identical bytes, so the whole handle is byte-identical.
    fn draw_serial(&self) -> CertSerial {
        let mut bytes = [0u8; SERIAL_BYTES];
        self.entropy.fill(&mut bytes);
        let hex = bytes.iter().fold(String::with_capacity(SERIAL_BYTES * 2), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        });
        CertSerial::new(&hex).unwrap_or_else(|_| {
            unreachable!("{SERIAL_BYTES}-byte lowercase hex is a valid CertSerial")
        })
    }
}

impl Ca for SimCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        Ok(RootCaHandle::new(
            CaCertPem::new(FIXTURE_ROOT_CERT_PEM.to_owned()),
            CaCertDer::new(FIXTURE_ROOT_CERT_DER.to_vec()),
            self.draw_serial(),
            CaKeyPem::new(FIXTURE_ROOT_KEY_PEM.to_owned()),
        ))
    }

    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice 02-03")]
    fn issue_intermediate(
        &self,
        _node: &overdrive_core::NodeId,
    ) -> Result<IntermediateHandle, CaError> {
        todo!("RED scaffold: SimCa::issue_intermediate (pathLen=0, signed by fixture root)")
    }

    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice 04")]
    fn issue_svid(&self, _req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        todo!("RED scaffold: SimCa::issue_svid (single URI SAN, CA:FALSE, CSPRNG serial)")
    }

    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice 03")]
    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        todo!("RED scaffold: SimCa::trust_bundle (root anchor; intermediate chain material)")
    }
}
