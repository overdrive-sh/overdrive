//! Regression test for `fix-job-submit-body-decode-variant` step 01-01.
//!
//! Pins the [`CliError`] variant returned when a successful 2xx control-plane
//! response carries a `job_id` field that the validating
//! [`JobId::new`](overdrive_core::id::JobId::new) constructor rejects.
//!
//! Bug under regression. The handler at
//! `crates/overdrive-cli/src/commands/job.rs:110` was mapping post-HTTP
//! `JobId::new` failure to [`CliError::InvalidSpec`] — but per the rustdoc
//! on the variants in `crates/overdrive-cli/src/http_client.rs:39-88`:
//!
//!  * [`CliError::InvalidSpec`] is *"Client-side spec validation failed
//!    BEFORE any HTTP call"* — client-side, pre-HTTP.
//!  * [`CliError::BodyDecode`] is *"a successful 2xx response whose body
//!    failed to deserialise into the expected typed shape — server-side
//!    contract violation"* — post-HTTP.
//!
//! The `JobId::new(&resp.job_id)` call site is post-HTTP. The variant was
//! wrong; this test pins the correct one. Step 01-02 swaps the variant in
//! the helper at `parse_response_job_id` from `InvalidSpec` to `BodyDecode`,
//! and this test goes RED → GREEN at that step.
//!
//! ## Why this is unit-shaped, not server-shaped
//!
//! Per `crates/overdrive-cli/CLAUDE.md` *Integration tests — no subprocess*,
//! integration tests in this crate call CLI handler functions directly. A
//! full reproduction of the bug end-to-end would require a real in-process
//! TLS-trusted control-plane server returning a 200 OK with a malformed
//! `job_id` JSON body — ≥100 lines of harness for one assertion. The bug
//! itself is a one-line variant choice on a private helper that is the
//! call site (RPP L1 *Extract Method* — extracted in step 01-01). Calling
//! the helper directly is port-to-port at unit scope (the function
//! signature IS the public interface) and kills the same mutations the
//! end-to-end shape would. The mutation-testing gate at the per-file level
//! still sees this as the canonical test for the variant choice.

use overdrive_cli::commands::job::parse_response_job_id;
use overdrive_cli::http_client::CliError;

/// A `job_id` value that is JSON-decodable (it is just a `String`) but
/// fails [`JobId::new`](overdrive_core::id::JobId::new) — empty string is
/// rejected by `validate_label` with `IdParseError::Empty { kind: "JobId" }`.
const MALFORMED_JOB_ID: &str = "";

#[test]
fn post_http_invalid_job_id_maps_to_body_decode_not_invalid_spec() {
    let err = parse_response_job_id(MALFORMED_JOB_ID)
        .expect_err("empty job_id must fail JobId::new validation");

    // Negative-assert FIRST: variant must NOT be the (wrong) InvalidSpec
    // shape this test was written to drive out. A future refactor that
    // re-introduces InvalidSpec at this call site would re-introduce the
    // bug; this assertion is the variant-choice pin.
    assert!(
        !matches!(err, CliError::InvalidSpec { .. }),
        "post-HTTP `JobId::new` failure must NOT map to CliError::InvalidSpec \
         (that variant is reserved for client-side, pre-HTTP validation per \
         rustdoc at http_client.rs:78-87) — got: {err:?}"
    );

    // Positive-assert: variant IS BodyDecode and its `cause` field names
    // the post-HTTP nature of the failure so operators can distinguish it
    // from a request-payload validation error.
    assert!(
        matches!(&err, CliError::BodyDecode { cause } if cause.contains("server returned invalid job_id")),
        "post-HTTP `JobId::new` failure must map to CliError::BodyDecode \
         carrying a cause that names the post-HTTP origin — got: {err:?}"
    );
}
