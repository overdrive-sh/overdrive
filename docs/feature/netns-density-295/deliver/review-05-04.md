# DELIVER Review — Step 05-04

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `05-04` — Creation-time close-on-exec at the eight sites and the cloexec lint gate |
| Iteration | 1 |
| Reviewer | Fresh isolated `nw-software-crafter-reviewer` |
| Commits | `6acbe6a241776c84a6007e51890068b6d7a3d8d7`, `cf8f3b720e00d3b605d0970f2110daae7f1aecf7` |
| Verdict | **APPROVED** |

## Scope and contract reviewed

Reviewed the two Step 05-04 commits against the approved roadmap, OBL-295-CLOEXEC's exact eight-site table and source-gate contract in the feature-delta, and DISTILL's S-ND295-46 source-local and workspace cases. The review covered the eight descriptor-creation changes, `xtask::cloexec_lint`, its `cargo xtask cloexec-lint` entry point, activation of S-ND295-46, and the recorded DELIVER phases.

The contract requires each listed descriptor to receive its close-on-exec flag at creation; the source gate must inspect the `overdrive serve` first-party workspace closure, reject the pinned libc/nix/rustix call families, fail closed on unreadable or unparseable source, and exempt an item only when one of its own `cfg` predicates requires `test`. S-ND295-46 must be activated without changing its authored assertions or replacing its bodies. The roadmap also pins gate-then-stop: the lint and existing cargo alias are sufficient; automatic CI or lefthook wiring is outside this step.

## Findings

No proven defect, contract divergence, or test-integrity issue was found. No remediation is required.

## Contract and implementation evidence

### Creation-time flags at all eight sites

The eight OBL-295-CLOEXEC calls in the six pinned files now carry their flags on the creating call:

| Site | Evidence |
|---|---|
| Shared leg-F and leg-C listener socket | `crates/overdrive-worker/src/mtls_intercept.rs:361` adds `SOCK_CLOEXEC` to `SOCK_STREAM`. |
| Leg-S dial socket | `crates/overdrive-dataplane/src/mtls/mod.rs:662` adds `SOCK_CLOEXEC` to `SOCK_STREAM`. |
| Netfilter netlink socket | `crates/overdrive-netlink/src/nft.rs:1645` adds `SOCK_CLOEXEC` to `SOCK_RAW`. |
| Generic netlink socket | `crates/overdrive-netlink/src/ethtool.rs:342` adds `SOCK_CLOEXEC` to `SOCK_RAW`. |
| Guest DNS socket | `crates/overdrive-control-plane/src/dns_responder/responder.rs:462` uses `SockFlag::SOCK_CLOEXEC`. |
| Decrypt-pump pipe | `crates/overdrive-dataplane/src/mtls/splice.rs:533` adds `O_CLOEXEC` while preserving `O_NONBLOCK`. |
| Encrypt-pump pipe | `crates/overdrive-dataplane/src/mtls/splice.rs:632` sets `O_CLOEXEC` alone, preserving the required blocking pipe. |
| Guest DNS `recvmsg` | `crates/overdrive-control-plane/src/dns_responder/responder.rs:390` passes `MsgFlags::MSG_CMSG_CLOEXEC`. |

There is no later `fcntl` workaround. The `recvmsg` flag is the pinned change and adds no exemption marker.

### Source gate and exact public surface

The public surface matches the feature-delta signatures and types: `CloexecRule`, `CloexecViolation`, `scan_source(source, file)`, `scan_workspace(manifest_path)`, `render_violation(v)`, and `run(manifest_path)` are present in `xtask/src/cloexec_lint.rs:20-179`. The module is exported from `xtask/src/lib.rs:9`. The existing `Task::CloexecLint { manifest_path }` and dispatch are in `xtask/src/main.rs:33-40, 340-343`; `.cargo/config.toml:10` already supplies the generic `cargo xtask` alias. No API beyond the accepted contract was added, and no alias/dispatch rewrite was needed.

The implementation follows the pinned boundary:

- `scan_source` parses already-read Rust with `syn`, collects imports, scans calls, applies the per-call marker, and returns locations in source order (`xtask/src/cloexec_lint.rs:64-75`). Its scanner imports no `overdrive-*` crate.
- `scan_workspace` starts from the workspace `overdrive-cli` package, follows normal dependencies among workspace members, scans `src/**/*.rs` except top-level `src/bin/**`, and propagates source read and parse failures (`xtask/src/cloexec_lint.rs:86-160, 578-598`). It errors when the workspace has no `overdrive-cli` package.
- The scanner resolves the pinned crate roots and `use` renames, and implements the accepted libc/nix/rustix call families and their pinned flag positions/rules (`xtask/src/cloexec_lint.rs:184-317, 421-529`). Marker placement is limited to the call line or the line immediately above and requires a reason (`xtask/src/cloexec_lint.rs:567-575`).
- The test-only predicate follows the accepted rule: `test` itself or recursively nested `all(...)` requiring `test` exempts the item; `any`, `not`, and `cfg_attr` do not (`xtask/src/cloexec_lint.rs:319-387`). The `launch_seccomp_kernel` module has `#[cfg(all(test, feature = "integration-tests"))]` at `crates/overdrive-host/src/vmm.rs:1460`; the real-workspace S-ND295-46 body needs and uses no marker to bypass it.
- `run` renders violations and returns an error if any are found (`xtask/src/cloexec_lint.rs:168-180`). The existing command variant dispatches to this function.

No CI or lefthook wiring was added. That matches the roadmap's gate-then-stop instruction.

### S-ND295-46 coverage and test integrity

All eight source-local bodies and all four workspace bodies named by DISTILL are active. The source-local table covers call-family/rule pairs, renamed imports and nix/rustix wrappers, unresolved flags, reasoned marker placement, simple and compound test-only predicates, parse failure, and rendering (`xtask/src/cloexec_lint.rs:1113-1206`). The workspace bodies cover the actual serve closure, fail-closed handling of an unparseable source, exclusion of auxiliary binaries and packages outside the closure, and missing `overdrive-cli` (`xtask/tests/integration/cloexec_lint_workspace.rs:127-355`). The compound-`cfg` planted cases include `all(test, feature = ...)`, reverse argument order, nested `all`, a second `cfg` attribute, and the non-exempt `any`, `not`, and `cfg_attr` controls.

Every activated body retains the exact `/// CONTRACT_SHAPE: pure-function.` declaration. Comparing the Step 05-04 commit to its parent shows only removal of the pending-step `#[ignore]` attributes in these pre-authored bodies; assertions, expected violations, fixtures, test names, and body content are unchanged. This is the required S-ND295-46 activation, not a test rewrite. The real-workspace assertion checks that the complete scanned closure returns zero violations; planted fixture assertions check concrete locations, call identities, and rules. No testing-theater pattern or production/test boundary violation was found.

The eight source-local test bodies cover at least eight distinct observable outcomes, giving a unit-test budget of at least 16 under the review rule of two tests per behavior; eight source-local tests ran. The four workspace bodies are integration cases and are not counted as unit tests. The behavior matrix is within budget.

## DELIVER discipline and mechanical evidence

The execution log records Step 05-04 in order as RED `EXECUTED/PASS`, GREEN `EXECUTED/PASS`, and COMMIT `EXECUTED/PASS` (`docs/feature/netns-density-295/deliver/execution-log.json`, timestamps 20:04:01Z, 20:19:41Z, and 20:22:05Z). The supplied RED report identifies semantic failures at the authored scaffolds rather than compile/import failures. Both commits retain Marcus as author, contain exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer, and include `Step-Id: 05-04`. `git diff --check` passes. The roadmap validation is approved.

The following verification results were supplied by the DELIVER orchestrator; this reviewer did not rerun the Lima commands:

| Verification | Reported result |
|---|---|
| `cargo xtask lima run -- cargo xtask cloexec-lint` | Passed; zero violations |
| `cargo xtask lima run -- cargo nextest run -p overdrive-host --lib -E 'test(launch_seccomp)' --no-fail-fast` | 1 passed |
| `cargo xtask lima run -- cargo nextest run -p xtask -E 'test(cloexec_lint)' --no-fail-fast` | 8 passed |
| `cargo xtask lima run -- cargo nextest run -p xtask --test integration --features integration-tests -E 'test(cloexec_lint_workspace)' --no-fail-fast` | 4 passed |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | Passed |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | Passed |

## Contract Shape Compliance

All twelve activated S-ND295-46 bodies retain their per-test Contract Shape declaration. The source-local pure-function tests use the repository's exact rustdoc declaration. No body or assertion was altered to obtain GREEN. The `xtask` module and command surface match the accepted design; no unapproved public API, error variant, or command option was introduced.

## Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — one acceptance scenario active | PASS | S-ND295-46 is the sole scenario activated by Step 05-04; its eight source-local and four workspace bodies are the authored cells. |
| G2 — valid RED failure | PASS | RED is logged PASS; crafter report states the scaffold failures were semantic, not compilation failures. |
| G3 — assertion failure for authored unit tests | PASS | The eight DISTILL source-local tests were already authored and were activated unchanged. |
| G4 — no domain mocks | PASS | The pure scanner tests call the scanner; workspace cases use temporary Cargo fixtures and `cargo_metadata`. No hexagon or domain mocks are involved. |
| G5 — domain language | PASS | Tests and diagnostics name descriptor creation, close-on-exec flags, source locations, call families, and the serve closure. |
| G6 — all required tests green | PASS | Required host, xtask unit, and xtask integration commands passed as reported. |
| G7 — green before commit | PASS | GREEN and COMMIT are ordered and PASS in the execution log; required check/clippy/lint gates passed as reported. |
| G8 — test budget | PASS | Eight source-local unit tests against a budget of at least 16; four integration cases are separate. |
| G9 — no test weakening | PASS | The diff removes only pending `#[ignore]` attributes from the pre-authored scenario bodies. |

No RPP L1-L2 issue affects the accepted contract or handoff. No escalation marker or product-owner test change was needed.

## Verdict and disposition

**APPROVED.** Step 05-04 satisfies the exact eight-site creation-time flags, source-gate API and closure boundary, compound-`cfg` test exemption, S-ND295-46 activation, and gate-then-stop scope. There are no findings to remediate; Step 05-04 review is complete.
