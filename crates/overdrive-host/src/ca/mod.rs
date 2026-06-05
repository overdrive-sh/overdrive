//! Host certificate-authority adapter — `RcgenCa`.
//!
//! The production [`overdrive_core::traits::ca::Ca`] implementation: it owns
//! all `rcgen` / `ring` crypto and translates the pure `overdrive-core`
//! [`overdrive_core::CertSpec`] policy into real X.509 bytes. Per ADR-0063 D1
//! `rcgen` / `ring` live ONLY in this `adapter-host` module — the
//! `overdrive-core` compile path stays crypto-free (dst-lint).

pub mod aead_codec;
pub mod rcgen_ca;

pub use aead_codec::RootKeyAeadCodec;
pub use rcgen_ca::RcgenCa;
