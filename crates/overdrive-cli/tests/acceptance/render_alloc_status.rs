//! Acceptance tests for `overdrive_cli::render::alloc_status` — step 05-05.
//!
//! Rendering is a pure string-builder — no I/O, no server dependency —
//! so it belongs in the default acceptance lane rather than the
//! `integration-tests`-gated slow lane. This is also the load-bearing
//! place the `phase-1-first-workload` reference must appear on an empty
//! allocation-status read per DWD-05 §6.2 / §6.7.
//!
//! Acceptance coverage:
//!   (d) empty-state rendering contains the `phase-1-first-workload`
//!       reference (walking-skeleton gate for the onboarding signpost).
//!   (e) non-empty rendering shows the `spec_digest` + `commit_index`.

use overdrive_cli::commands::alloc::AllocStatusOutput;

fn fixture_empty_state() -> AllocStatusOutput {
    AllocStatusOutput {
        job_id: "payments".to_string(),
        spec_digest: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_string(),
        commit_index: 1,
        allocations_total: 0,
        empty_state_message: "0 allocations for job payments — the scheduler + driver land in \
             phase-1-first-workload"
            .to_string(),
    }
}

fn fixture_with_allocations() -> AllocStatusOutput {
    AllocStatusOutput {
        job_id: "payments".to_string(),
        spec_digest: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        commit_index: 7,
        allocations_total: 3,
        empty_state_message: String::new(),
    }
}

// -------------------------------------------------------------------
// (d) empty-state rendering contains phase-1-first-workload
// -------------------------------------------------------------------

#[test]
fn render_alloc_status_empty_state_contains_phase_1_first_workload() {
    let out = fixture_empty_state();
    let rendered = overdrive_cli::render::alloc_status(&out);

    assert!(
        rendered.contains("phase-1-first-workload"),
        "rendered alloc-status empty-state must reference phase-1-first-workload; \
         got:\n{rendered}",
    );
    assert!(
        rendered.contains("payments"),
        "rendered alloc-status must name the job id; got:\n{rendered}",
    );
    assert!(
        rendered.contains(&out.spec_digest),
        "rendered alloc-status must carry the spec_digest; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (e) non-empty rendering shows allocations_total + spec_digest
// -------------------------------------------------------------------

#[test]
fn render_alloc_status_with_allocations_shows_total_and_digest() {
    let out = fixture_with_allocations();
    let rendered = overdrive_cli::render::alloc_status(&out);

    assert!(
        rendered.contains("payments"),
        "rendered alloc-status must name the job id; got:\n{rendered}",
    );
    assert!(
        rendered.contains('3'),
        "rendered alloc-status must carry allocations_total value; got:\n{rendered}",
    );
    assert!(
        rendered.contains(&out.spec_digest),
        "rendered alloc-status must carry the spec_digest; got:\n{rendered}",
    );
    // On non-empty results we SHOULD NOT print the empty-state hint
    // (would confuse the operator).
    assert!(
        !rendered.contains("phase-1-first-workload"),
        "rendered alloc-status with allocations must NOT print the empty-state hint; got:\n{rendered}",
    );
}
