# DELIVER Review — Step 05-00

## Review metadata

- Feature: `netns-density-295`
- Step: `05-00` — Managed-link identity and the fresh-host bridge MAC pin
- Iteration: 1
- Reviewer: `nw-software-crafter-reviewer`
- Reviewed commits: `f4d2613aba4906a88439e9d9c42970d95fae226e` and `0f7d8fb9be31e2a4510743405c259259f1155249`
- Verdict: **APPROVED**
- Authoritative contract: approved `deliver/roadmap.json` step 05-00; feature-delta section “Managed-link identity independent of host link configuration (fresh-host RCA)” and C-295-0; ADR-0126; DISTILL S-ND295-72 legs (a), (b), and (d), and S-ND295-00 bridge-identity leg (c), including their red-classification entries.

## Iteration 1

### Contract and implementation review

The implementation matches the pinned interface. `Client::ensure_bridge(&self, name, mac)` retains its existing signature and now sends the requested address and down state in the bridge creation message (`crates/overdrive-netlink/src/client.rs:449-467`). Its rustdoc states creation with `IFLA_ADDRESS`, down state, and adopt-without-write behavior. Both production call sites pass `GUEST_BRIDGE_MAC`: the startup probe’s `ConvergeBridge` arm and `HostSharedGuestNetworkOwner::converge_shared` (`crates/overdrive-control-plane/src/guest_network.rs:1183-1192, 3989-3999`). No public type, method, variant, or parameter was added.

`converge_shared` lowers the bridge, converges the MAC and gateway prefix, reads the bridge identity and gateway, reports a mismatch with the observed `GuestNetworkFact::Bridge` address and up state, and only then brings the bridge up (`guest_network.rs:3993-4080`). `audit_shared` also reports the observed bridge identity through the existing `GuestNetworkFact::Bridge` variant (`guest_network.rs:4210-4279`). The expected/observed facts preserve the existing error taxonomy. The `attach_tap_to_bridge` comment now describes the creation-pinned bridge address without claiming that Linux adopts the first attached port’s address.

These operations follow C-295-0 and ADR-0126: bridge identity and gateway are read back before bringing the bridge up, and no endpoint or TAP attachment precedes that check. The code makes no TAP changes, adds no startup-probe address read, and changes no host configuration or infrastructure file.

### Activated acceptance bodies and test integrity

The only accepted bodies newly activated are S-ND295-72 (a), S-ND295-72 (d), and S-ND295-00’s bridge leg (c). S-ND295-72 (b) remains active and unchanged. The superseded scratch-TAP probe bodies remain deleted; the TAP invariant bodies remain pending for their owning steps. The commit diff removes only the three corresponding `#[ignore]` markers and does not alter their assertions or outcomes.

The acceptance evidence is meaningful at the specified boundaries:

- The real-netlink integration body observes the newly created bridge before any later address write and asserts bridge kind, `NET_ADDR_SET`, requested address, and down state (`managed_link_address.rs:135-186`). Its active adoption body checks that a present link is unchanged (`managed_link_address.rs:188-198`).
- The source-local kernel body drives the production owner’s `audit_shared` port after an out-of-band MAC or up-state change and checks the typed bridge mismatch, including the observed address, ifindex, and up state (`guest_network.rs:11036-11110`).
- The startup body calls the in-process `run_server` composition root on fresh host state, checks the created bridge identity and up state, verifies the bridge guard and map pins, and drains the server handle (`shared_guest_network_startup.rs:342-429`). It does not spawn the production binary.

Each activated test carries `CONTRACT_SHAPE: bounded-change` and an outcome anchor. Their names do not match the banned test-name pattern. No source-local pure-function property was added or transitioned, so the special `CONTRACT_SHAPE: pure-function.` declaration rule does not apply. The activated bodies preserve their original assertions; no test was weakened, replaced, deleted, or skipped to accommodate the implementation.

### TDD and phase evidence

`execution-log.json` records RED, GREEN, and COMMIT as `EXECUTED` / `PASS`, in order, at lines 636-654. The accepted RED evidence is independently recorded in `distill/red-classification.md:586-587, 591, 593`: bridge creation had `addr_assign_type` 1 instead of 3; the owner mismatch reported no observed fact; and fresh-host boot reproduced `BridgeObserve` refusals. The adoption body was already green and stays active. The commit preserves the original test bodies and changes production code for GREEN.

### Files and commit scope

The two implementation commits retain Marcus as author and each has exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer and `Step-Id: 05-00`. The modified files are the two production files, the two activation files, and `execution-log.json`. The additional `shared_guest_network_startup.rs` activation is the approved S-ND295-00 bridge-leg fallout named in the review handoff. The workspace has a pre-existing dirty `AGENTS.md`; it was not changed.

### Verification

All four roadmap verification commands passed in Lima:

1. `cargo xtask lima run -- cargo nextest run -p overdrive-netlink --test integration --features integration-tests -E 'test(managed_link_address)' --no-fail-fast` — 2 passed, 6 skipped.
2. `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --lib -E 'test(shared_owner_link_address_kernel)' --no-fail-fast` — 1 passed, 298 skipped.
3. `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` — passed.
4. `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` — passed.

The separately targeted production-composition acceptance body also passed:

- `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(production_host_owner_boots_only_after_real_shared_identity_is_exact)' --no-fail-fast` — 1 passed, 240 skipped.

### Findings and remediation disposition

No proven defect or contract divergence was found in this step. There are no findings to remediate and no unresolved hypothesis that requires a design change. No mutation test was run; that remains the single final DELIVER-wave gate.

## Final verdict

**APPROVED.** Step 05-00 implements the exact approved bridge creation contract, performs the required read-back-before-up sequence, reports the observed bridge mismatch through the existing fact type, activates only its named acceptance bodies, and passes the required verification.
