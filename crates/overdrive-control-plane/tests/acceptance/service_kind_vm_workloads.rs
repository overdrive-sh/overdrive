//! RED acceptance scaffold for the production composition boundary.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

/// S-SVM-22 — one `overdrive serve` boot performs the existing ProbeRunner
/// Earned-Trust probe exactly once, retains that returned `Arc`, gives clones
/// to both production drivers, and preserves all existing VM-capability
/// success/refusal outcomes.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn one_server_boot_shares_exactly_one_trusted_probe_runner_with_both_drivers() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-22 / production runner composition)");
}
