# Step 05-07 final schema documentation remediation

Date: 2026-11-13
Crafter: Codex replacement
Feature: service-allocation-readiness-and-ending
Step: 05-07 — Default startup gate is guest-reported listeners

## Contract and scope

The existing canonical OpenAPI check failed at Metal run 92e992ed88053e8f155b815b. The generated schema still described the superseded ADR-0058 inferred TCP probe, the default-TCP mechanic summary, and the old synthesized ProbeIdx description.

ADR-0270 / feature-delta D15 is precise: only the Service TOML parser synthesizes one fieldless GuestListeners descriptor when the startup section is omitted. It gates Stable on the guest reporting every declared TCP and UDP listener, with the existing 60-second startup window. An explicit empty TOML startup array remains the opt-out. Raw-API omission or an empty startup vector remains the opt-out.

This remediation changes schema-source documentation and regenerates the existing api/openapi.yaml. It does not change API fields, types, enum variants, serialization, parsing, or runtime behavior.

## Documentation updates

The source comments now distinguish TOML parsing from raw API admission and describe the D15 mechanic accurately. The additional comments in api.rs, api/submit.rs, probe_result_row.rs, and transition_reason.rs are schema sources used by utoipa; correcting only the mechanic enum would leave the generated ProbeIdx, ServiceSpecInput, probe projection, and Stable descriptions stale. The TCP mechanic documentation itself remains a TCP-connect description without default-inference wording.

The generated schema no longer describes an inferred TCP default. Its ProbeMechanic summary includes the platform-managed GuestListeners gate, its TCP variant describes only TCP-connect behavior, the GuestListeners variant names all TCP and UDP listeners and the 60-second deadline, and its startup/Stable/ProbeIdx descriptions preserve the TOML-only default and raw-API opt-out.

## Verification

| Command or evidence | Result |
|---|---|
| Pre-remediation canonical OpenAPI check, Metal run 92e992ed88053e8f155b815b | Failed on stale schema descriptions |
| BPF build prerequisite in isolated current-HEAD worktree, Metal run 1d2896899f3438192f3f1d8e | Passed; this only supplied the object needed to compile the workspace OpenAPI binary |
| OpenAPI generation, Metal run 228b7fe8b06154b1a24909fa | Passed: cargo run -p overdrive-control-plane --bin openapi -- gen |
| OpenAPI check, Metal run 3cf9d255047d03481953fd10 | Passed: cargo run -p overdrive-control-plane --bin openapi -- check |
| Scoped formatting, Metal run 2a36f17cfcc7b89f20450531 | Passed: cargo fmt --check -p overdrive-core -p overdrive-control-plane |
| Generated schema scan | No ADR-0058, default-TCP, or inferred-TCP startup descriptions remain |
| Diff hygiene | git diff --check passed for the remediation paths |
| Tests, native proof, and mutation tests | Not rerun; the changes are documentation-only |

No DES events were added or rewritten for this schema-only remediation.
