//! Integration-test entrypoint for `overdrive-bench` (testing.md § "Integration vs
//! unit gating"). Subprocess scenarios of the `bench-report` tool and the
//! macro-generated smoke variant; every one is host-pure (no kernel state).

#![cfg(feature = "integration-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "scenario drivers assert their own preconditions and name protocol terms"
)]

use overdrive_testing::overdrive_test_default;

overdrive_test_default!(host_pure);

mod integration {
    mod methodology_documents;
    mod report_rendering;
    mod report_validation;
    mod smoke_variant;
    mod support;
}
