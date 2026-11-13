# Step 03-02 final Clippy remediation

## Scope and approved contract

This closes the final strict Clippy failure in the private action-shim test
adapter added for D17. `EvaluationSubmission::Standalone` now exists only when
`cfg(test)` is active, matching its construction sites in the source-local
reclamation tests and the `#[cfg(all(test, feature = "integration-tests"))]`
replacement acceptance module. The `EvaluationBroker` import and match arm use
the same gate. The production `Runtime` route, `submit_evaluation` ordering,
drain-only `BrokerDrain`, stored `Notify` permit, and cadence behavior are
unchanged. No public or cross-crate API, ownership, or notification semantics
changed.

## Reproduced failure

The primary workspace's canonical Metal run could not sync because the remote
checkout contains root-owned `.context/step-05-07-native` evidence. Following
the repository procedure, the focused checks below ran through the normal
Metal runner from a fresh sparse worktree at `d73862622`, with the current
control-plane tracked fixture diffs copied into it. No pre-existing workspace
changes were altered.

Before the code change, the focused strict Clippy command failed for the
expected reason:

```text
cargo xtask metal run -- cargo clippy -p overdrive-control-plane --all-targets --features integration-tests,kvm-tests -- -D warnings
```

It reported `variant Standalone is never constructed` at
`action_shim/mod.rs:79` while compiling the non-test library with the
`integration-tests` feature. The only constructors are under `cfg(test)`:
`action_shim/reclamation.rs`'s test module and
`action_shim/replacement_acceptance.rs`, which is included under both `test`
and `integration-tests`.

## Remediation

The three `Standalone` cfg gates and its `EvaluationBroker` import changed from
`cfg(any(test, feature = "integration-tests"))` to `cfg(test)`. This keeps the
existing standalone broker test support available in unit-test builds,
including builds with `integration-tests`, and removes the unused variant from
the feature-enabled non-test library. No `allow(dead_code)` or other lint
suppression was added.

## Verification

| Check | Result |
|---|---|
| Focused strict Clippy before change | **RED** — one expected `Standalone` dead-code error. Metal run `663a252835b77fd90d55c0cc`. |
| Focused strict Clippy after change: `cargo xtask metal run -- cargo clippy -p overdrive-control-plane --all-targets --features integration-tests,kvm-tests -- -D warnings` | **PASS**. Metal run `b3116b9963afeb5071cfbc4a`. |
| D17 convergence-loop wake tests: `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(/convergence_loop_wake::/)'` | **PASS**, 5 passed; 581 unrelated tests filtered. This also compiles the source-local standalone adapter constructors. Metal run `42302d54ea1a3dca6b4d5f79`. |
| Full workspace strict Clippy: `cargo xtask metal run -- cargo clippy --workspace --all-targets --features integration-tests,kvm-tests -- -D warnings` | Reached an unrelated existing `clippy::doc_markdown` failure in `xtask/src/dst_lint.rs:5731,5752` (`CONTRACT_SHAPE` needs backticks). No xtask or test-design files were changed. |
| Mutation testing | Not run, per the user's explicit final mutation-testing waiver. |

The workspace-wide run used a prepared BPF object from
`cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build`
(Metal run `3653eb110635896cf72e7471`). The focused Clippy and D17 test runs
used the normal sync, lease, and native preflight. No native boot or separate
proof run was needed for this private cfg correction.

## Contract and scope audit

The D17 public API and behavior remain exactly as approved in the feature
delta. The diff is limited to the private `EvaluationSubmission` test
representation and this remediation record, plus the DES phase events for this
replacement crafter. The existing shared dirty files remain untouched.
