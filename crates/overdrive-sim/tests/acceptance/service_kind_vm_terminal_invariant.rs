//! Seeded DISTILL scaffold for the accepted terminal-authority outcome.
//!
//! DELIVER activates this through the existing `overdrive-sim` lifecycle and
//! backend-observation boundaries. The fixed seed must reproduce a schedule in
//! which a VM Service becomes terminal while health work is in flight, then
//! prove only the externally meaningful invariant: terminal wins and the dead
//! backend never returns to eligibility. This scenario does not authorize a
//! ProbeRunner drain/join contract, tombstone, write suppression, or new seam.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

/// S-SVM-17 — fixed seed `25717` interleaves terminal state with outstanding
/// health work; the terminal allocation remains terminal and its backend never
/// becomes eligible again.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[should_panic(expected = "RED scaffold")]
fn terminal_state_wins_and_dead_vm_backend_never_returns_to_eligibility() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SVM-17 / seeded terminal authority, seed 25717)"
    );
}
