# Review: Built-in CA Operator Composition (Step 02-01)

**Reviewer:** codex_nw_review
**Review Date:** 2026-06-10
**Commit:** `a017dd09` (`feat(control-plane): add cause-distinct CaBoot error variant at operator boundary`)
**Step:** `02-01` - Typed CA-boot failure stays cause-distinguishable at the operator boundary
**Verdict:** **APPROVED**

## Findings

No blocking or non-blocking defects found.

## Scope Reviewed

Step `02-01` is correctly limited to the declared implementation scope:

- `crates/overdrive-control-plane/src/error.rs`
- `crates/overdrive-control-plane/tests/acceptance/error_mapping_exhaustive.rs`

The commit adds no underlying `CaError` or `CaBootError` shape change, matching the roadmap constraint that the slice is an additive control-plane error surface.

## Criteria Conformance

### Dedicated typed boundary variant

`ControlPlaneError::CaBoot(#[from] crate::ca_boot::CaBootError)` is added as a dedicated transparent variant, mirroring the existing boot-time pass-through variants (`Tls`, `ViewStoreBoot`, `DataplaneBoot`) instead of flattening through `ControlPlaneError::Internal(String)`.

This satisfies the main contract: `CaBootError` can flow through `?` / `From` and remain matchable as `ControlPlaneError::CaBoot(_)` at the composition root.

### HTTP mapping

`to_response` maps `ControlPlaneError::CaBoot(_)` to HTTP 500 with `error = "internal"` and no `field`, matching the step's exhaustiveness-only requirement. This is consistent with other pre-listener boot failures.

### Cause-distinct test coverage

The acceptance test enumerates the finite boot-cause set required by the roadmap:

- absent KEK: `CaBootError::KekUnavailable`
- wrong KEK: `CaBootError::EnvelopeDecrypt { source: CaError::WrongKek }`
- tampered envelope: `CaBootError::EnvelopeDecrypt { source: CaError::TamperedEnvelope }`

For each case, it verifies conversion into `ControlPlaneError::CaBoot(_)`, HTTP 500 mapping, stable `internal` response kind, and no response field. It also asserts wrong-KEK and tampered-envelope messages remain pairwise distinct and preserve their underlying cause strings.

## Test Quality

The new test is not testing theater:

- It has concrete assertions on the public error boundary, not only construction or type existence.
- The finite table over the three enumerated causes is appropriate for the closed cause set.
- A regression that routes `CaBootError` through `Internal(String)` would fail the matchability assertion.
- A regression that collapses wrong-KEK and tampered-envelope display into the same cause string would fail the pairwise-distinct assertion.

The test intentionally avoids private field inspection and stays at the observable `From` / `to_response` boundary.

## Verification

I attempted both focused local checks:

- `cargo test -p overdrive-control-plane --test acceptance ca_boot_error_causes_map_to_distinct_control_plane_ca_boot_variant -- --nocapture`
- `cargo check -p overdrive-control-plane --tests`

Both fail before reaching this code on macOS because `linux-keyutils` references Linux-only `libc` symbols such as `SYS_add_key`, `SYS_request_key`, `SYS_keyctl`, `EKEYEXPIRED`, and `ENOKEY`. This is an environment limitation, not a defect in the reviewed slice; the roadmap already requires Lima for this lane.

The DELIVER execution log records `02-01` `GREEN` as `PASS`, but I did not independently reproduce the green run in Lima from this workspace.

## Residual Risk

Low. The production change is additive, exhaustive-match enforced, and localized to the control-plane error boundary. The next slice (`02-02`) still needs to prove the production `run_server` boot path actually propagates `CaBootError` through this variant via `?`.

## Approval

**APPROVED** for step `02-01`.
