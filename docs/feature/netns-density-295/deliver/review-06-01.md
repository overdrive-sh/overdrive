# DELIVER Review — Step 06-01

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `06-01` — Egress guest-MAC classifier and debug-mask netlink reads |
| Iteration | 1 |
| Reviewer | Fresh isolated `nw-software-crafter-reviewer` |
| Commits | `644e1d499caab18baa92f569fb2de67ee1ce5a56`, `0203afc35f78fc609ebb15fef97eca0716865df9` |
| Verdict | **APPROVED** |

## Scope and contract reviewed

Reviewed the commits against the approved Step 06-01 roadmap and the accepted D-295-R21 adapter contract, D-295-R22 debug-mask read contract, and DISTILL scenarios S-ND295-47, S-ND295-48, S-ND295-49, and S-ND295-08's nine-slot body. The changed production boundary is the existing Rust BPF guest TCX object, `overdrive-dataplane::guest_tcx`, and `overdrive-netlink::ethtool`; the reviewed tests are the Tier-2 `BPF_PROG_TEST_RUN` classifier body and the Lima kernel integration bodies for egress lifecycle, inventory, and TAP debug-mask reads.

The contract requires the total egress verdict keyed by the egressing device's `ifindex`, the ninth counter at private index 8, loaded-object and pin inventory for both classifiers, an egress pin beside ingress, and single/dump debug-mask reads that preserve sourced failures. It keeps the existing dataplane and netlink error taxonomies and exact accepted signatures. The step must activate its eight authored bodies without changing their assertions or setup. The roadmap is approved, and the implementation uses the existing `programs/guest_tcx.rs`, `maps/guest_tcx.rs`, and `ethtool.rs` module homes; the roadmap's anticipated aliases are not missing contract files.

## Findings

No proven defect, contract divergence, test-integrity issue, or scope expansion was found. No remediation is required.

## Contract and implementation evidence

### Egress verdict and counter

`COUNTERS` now has nine entries in `crates/overdrive-bpf/src/maps/guest_tcx.rs:20`. `gh295c_egress` reads the Ethernet destination, delivers every group frame, then checks `ENDPOINTS` by `__sk_buff.ifindex`; it delivers only the registered `source_mac` and counts exactly one `EgressDestinationDrop` for an unreadable header, map miss, or foreign unicast (`crates/overdrive-bpf/src/programs/guest_tcx.rs:132-154`). This follows R21's total verdict table, including fail-closed map-miss behavior.

The S-ND295-47 body drives the embedded production classifier with `BPF_PROG_TEST_RUN`, varies endpoint presence and destination class, and asserts both verdict and the complete counter vector (`crates/overdrive-bpf/tests/integration/guest_tcx_classifier_test_run.rs:582-749`). Its live loopback `ifindex` is an accepted Tier-2 device selector; the distinct Lima integration test attaches the loaded program to a real scratch TAP. The test deliberately does not submit a runt skb because the accepted scenario records the kernel's pre-classifier `EINVAL` boundary and relies on verifier acceptance for the program's header bounds check.

`GuestTcxCounter::EgressDestinationDrop` maps to slot 8, and the counter schema accepts exactly nine entries; eight and ten project as unsupported (`crates/overdrive-dataplane/src/guest_tcx.rs:235-270, 992-1015, 1941-1987`). The activated ingress nine-slot body also checks that all ingress rows leave slot 8 unchanged (`crates/overdrive-bpf/tests/integration/guest_tcx_classifier_test_run.rs:232-250`).

### Dataplane lifecycle, inventory, and API shape

`GuestTcxProgram::load` loads and receipts both classifier identities from the same embedded object. The exact designed `attach_first_egress(&mut self, interface: &str) -> Result<GuestTcxLink, GuestTcxError>` uses `TcAttachType::Egress` and first ordering, returning the egress program id and attach type (`crates/overdrive-dataplane/src/guest_tcx.rs:1101-1187, 1332-1364`). The nine-slot schema and inventory checks reuse the existing public types and operations. Inventory validates each receipted program's id, tag, name, type, and map ids, reports up to two loaded programs, and keeps an unreceipted post-baseline program ambiguous (`guest_tcx.rs:1680-1725`).

The egress lifecycle integration test proves the loaded program is distinct, attaches at egress, leaves ingress untouched, pins at `links/<tap>-egress`, sits beside ingress, is queryable at egress, exposes counter slot 8, and detaches with an absence read-back (`crates/overdrive-dataplane/tests/integration/guest_tcx_egress_lifecycle.rs:83-180`). The inventory tests cover two receipted programs, cleanup to zero, and the retained unpinned-object lifecycle (`guest_tcx_inventory.rs:329-469`). The `unpin_*_map` implementation now removes the pin while retaining an already-adopted map handle until adopted-state drop. That lifetime is explicitly required by the accepted D12/S-ND295-48 contract and is exercised by the retained-map rows; it adds no public method, type, ownership mechanism, or persisted state.

The only public additions match the accepted feature-delta signatures and variants: `GuestTcxObject::EgressClassifier`, `GuestTcxCounter::EgressDestinationDrop`, `GuestTcxProgram::attach_first_egress`, and the two named `ethtool` read functions. `GuestTcxLink::pin`/`detach`, `query_attachment`, `detach_pinned_link`, and `read_counter` are reused. There is no added low-level owner port, public error variant, lifecycle owner, or cross-crate method beyond the approved shape (`feature-delta.md:2630-2703`).

### Debug-mask read surface

`overdrive_netlink::ethtool::{debug_msg_mask, debug_msg_masks}` implement the accepted single-link and host-netdev dump reads (`crates/overdrive-netlink/src/ethtool.rs:228-329`). The implementation decodes the compact debug bitset and dump `ifindex`, returns the kernel's `ENODEV` under `debug-get`, and preserves transport, decode, dump, and worker-thread causes instead of substituting zero (`ethtool.rs:331-445`). The async functions await the worker result before returning; no read failure is converted into a successful observation.

The activated S-ND295-49 integration bodies create an isolated scratch TAP, set its debug level through a test-local raw `TUNSETIFF` queue and `TUNSETDEBUG`, verify the single and dump results, and assert the absent-device `ENODEV` and operation label (`crates/overdrive-netlink/tests/integration/tap_debug_msg_mask.rs:58-145`). This is independent of the 05-03 queue-handoff API, as the scenario requires.

### Acceptance coverage and test integrity

The eight bodies activated by this step are the S-ND295-08 nine-slot body; S-ND295-47's egress verdict body; S-ND295-48's counter-schema unit body, egress lifecycle body, and two inventory bodies; and S-ND295-49's two integration bodies. The commit diff removes only their pending-step `#[ignore]` attributes. No assertion, expected value, setup, fixture, test name, or body was weakened, replaced, or re-authored.

The source-local tests use the repository's per-test Contract Shape declarations: `bounded-change` for the kernel/integration bodies and the exact `/// CONTRACT_SHAPE: pure-function.` line for the counter-schema property. The already-authored debug-constant test also retains its pure-function declaration (`ethtool.rs:923-955`). The four distinct product behaviors are the egress verdict/counter partition, the egress attach/pin/query/detach and inventory lifecycle, the nine-slot schema with ingress counter preservation, and the debug-mask single/dump/error read. The unit-test budget is 8; the two source-local unit bodies are within it. Real-kernel and Tier-2 integration bodies are assessed at their separate boundaries, not counted as unit tests. No testing-theater, domain-mock, or external-validity defect was found.

## DELIVER discipline and mechanical evidence

The execution log records ordered RED, GREEN, and COMMIT events, each `EXECUTED/PASS`, at `2026-10-02T20:34:20Z`, `20:53:07Z`, and `20:53:47Z`. The supplied RED report records semantic failures at the authored behavior: foreign unicast delivery, an eight-entry counter/schema, single-program inventory, and the debug-read `ENODEV` scaffold. Both commits retain Marcus as author, exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer, and `Step-Id: 06-01`. The roadmap validation is approved. The scoped commit diff passes `git diff --check`. The pre-existing dirty `AGENTS.md` was preserved.

The following roadmap verification outcomes were supplied by the DELIVER orchestrator; this reviewer did not rerun the Lima commands:

| Verification | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-bpf --test integration --features integration-tests -E 'test(guest_tcx_classifier_test_run)' --no-fail-fast` | Passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --lib -E 'test(guest_tcx)' --no-fail-fast` | Passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --test integration --features integration-tests -E 'test(guest_tcx_egress_lifecycle) or test(guest_tcx_inventory)' --no-fail-fast` | Passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-netlink --test integration --features integration-tests -E 'test(tap_debug_msg_mask)' --no-fail-fast` | Passed |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | Passed |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | Passed |
| `cargo verifier-regress` | Passed |

## Contract Shape Compliance

Every transitioned body retains its authored per-test Contract Shape declaration. The source-local pure-function property uses the exact rustdoc declaration. Only pending-step ignores were removed. Public and doc-hidden cross-crate additions match D-295-R21/R22; no extra public method, parameter, type, enum variant, error class, or low-level port was added.

## Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — required acceptance coverage | PASS | Only the eight bodies required by S-ND295-47/48/49 and S-ND295-08 were activated. |
| G2 — valid RED failure | PASS | DES RED is `EXECUTED/PASS`; the reported failures were semantic and reached the missing behavior. |
| G3 — assertion failure for authored unit behavior | PASS | The counter schema/counter-slot behavior had an assertion-driven RED; no scaffold/import failure was reported. |
| G4 — no mocks inside the hexagon | PASS | The egress and debug-read adapter bodies use BPF/kernel/netlink effects; source-local coverage is pure. |
| G5 — domain language | PASS | Tests name registered guest MAC delivery, counter slots, inventory, TAP debug masks, and typed source errors. |
| G6 — required verification green | PASS | Every roadmap command listed above passed as reported. |
| G7 — green before commit | PASS | GREEN and COMMIT are ordered and pass in the DES log; check, clippy, and verifier regression passed. |
| G8 — test budget | PASS | Four behaviors give a budget of eight unit tests; two source-local unit bodies are in scope. |
| G9 — no test weakening | PASS | The test diff removes only pending-step ignore attributes. |

**Test integrity:** no modification-to-pass, testing theater, skipped-body, or escalation issue detected. **RPP:** no scope-relevant L1/L2 finding requiring remediation. Review iteration 1 is approved; no remediation disposition is pending.

## Verdict and disposition

**APPROVED.** Step 06-01 satisfies the exact egress verdict and ninth-counter contract, dataplane lifecycle and inventory contract, debug-mask read boundary, authored test dispositions, and required verification gates. Review artifact complete.
