# Step 05-02 final lint remediation

## Scope and approved contract

This remediation closes the stale private `xtask::dst_lint` recognizer found by
the final workspace lint after step 05-02 was approved. It does not change
production code, a public or cross-crate interface, the probe gate, or the
step's accepted behavior.

The approved roadmap criterion requires `run_server` to construct exactly one
`GuestListenerBoard` and pass the same board to `ProbeRunner` and `VmDriver`.
The feature delta pins that composition in D13 host-side and DG-6. In the
current production path, `lib.rs:3299-3309` constructs one board, coerces a
clone to `GuestListenerReports`, and calls
`compose_and_probe_runner_gate_with_reports`. That helper constructs
`ProbeRunner` with the supplied reports port and awaits `runner.probe()` at
`probe_runner_boot.rs:75-92`. The same board is passed to `compose_vm_driver`
later in `lib.rs`.

The existing `compose_and_probe_runner_gate` helper remains a valid gate entry
point: it delegates to the same reports helper. The required invariant is that
the production composition reaches a helper that runs `ProbeRunner::probe`,
while using the shared board required by this step.

## Reproduced failure

The recorded canonical lint run at source commit `f13990f43` (run
`ae3dac63ba6e4596d52154e9`, raw log
`/private/tmp/service-readiness-final-dst-lint.log`) returned six violations.
Five were blocking filesystem calls in the fixture owned by the parallel
DISTILL remediation. The sixth was `ProbeGateHelperUnused`: the scanner at
`xtask/src/dst_lint.rs` recognized only a call named
`compose_and_probe_runner_gate`, while production `lib.rs:3302` called
`compose_and_probe_runner_gate_with_reports`.

Before changing the scanner, the existing regression test was rerun:

```text
cargo test -p xtask --lib probe_gate_helper_real_lib_rs_is_clean -- --nocapture
0 passed; 1 failed
ProbeGateHelperUnused: missing call: compose_and_probe_runner_gate
```

This failure is reachable through the approved production composition described
above. It is a scanner-recognition defect; no behavior or design premise was
disproved.

## Remediation

`xtask/src/dst_lint.rs` now recognizes only the exact approved helper names
`compose_and_probe_runner_gate` and
`compose_and_probe_runner_gate_with_reports`, for both qualified and
unqualified calls. Calls inside `#[cfg(test)]` still do not satisfy the gate.
The existing helper shape remains accepted. A lookalike name such as
`compose_and_probe_runner_gate_with_reports_disabled` still fails, so the
recognizer does not use prefix or broad pattern matching.

The real-`lib.rs` regression test now describes the production
`with_reports` call. Two focused tests exercise the exact new helper shape and
reject a lookalike. Both new tests carry `CONTRACT_SHAPE: pure-function.`.

## Verification

| Check | Result |
|---|---|
| Baseline real-source scanner test | **RED**, 0/1; failed with the expected `ProbeGateHelperUnused` business violation |
| Focused scanner suite: `cargo test -p xtask --lib probe_gate_ -- --nocapture` | **PASS**, 10/10 |
| `cargo clippy -p xtask --lib -- -D warnings` | **PASS** |
| `rustfmt --edition 2024 --check xtask/src/dst_lint.rs` | **PASS** |
| `git diff --check -- xtask/src/dst_lint.rs` | **PASS** |
| Canonical final lint: `cargo xtask metal run -- cargo xtask dst-lint` | **PASS**, run `dbf2700f5458905e4584797e`, terminal record `exit_code=0` |

The final lint ran from a fresh isolated worktree detached at fixture-remediation
commit `493e450d9`, with the scanner file copied from this remediation (SHA-256
`fd4b48db358783c6455314bd980e49d81ca60b7ecbffbf714219e0e3c892b9ef`). This
ensured the complete scanner and the five completed fixture fixes were tested
together. The canonical runner used its normal sync, lease, and native
preflight. The first attempt from the original worktree stopped before lint
execution at rsync (exit 23) because its existing remote checkout held
root-owned step-05-07 evidence absent locally; that retained remote state was
left untouched.

`cargo fmt --package xtask -- --check` reports an existing formatting diff in
the unrelated, unchanged `xtask/src/metal_run_isolation_conformance.rs:519`.
The changed scanner file passes its direct rustfmt check; the unrelated file was
left untouched. Mutation testing was not run, consistent with the explicit
step-level waiver.
